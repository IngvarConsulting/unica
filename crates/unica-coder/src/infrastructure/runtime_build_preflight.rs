use crate::domain::project_sources::{SourceFormat, SourceSetKind};
use crate::domain::source_target::{ResolvedTarget, TargetKind};
use crate::domain::support_state::{ConfigurationSupportState, SupportStateReader};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform::filesystem::path_lock_identity;
use crate::infrastructure::platform::secure_read::read_root_relative_regular_file;
use crate::infrastructure::project_sources::{
    ProjectSourceMapProvenance, PROJECT_SOURCE_MAP_INPUT_MAX_BYTES,
};
use crate::infrastructure::source_roots::{
    normalize_contained_source_root, normalize_path_identity,
};
use crate::infrastructure::support_state::{
    capture_workspace_configuration_support_evidence, SupportStateReaderFactory,
    WorkspaceConfigurationSupportEvidence, WorkspaceSupportStateReaderFactory,
};
use crate::infrastructure::workspace::discover_workspace;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[cfg(test)]
thread_local! {
    static BEFORE_SUPPORT_EVIDENCE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
struct BeforeSupportEvidenceHookGuard;

#[cfg(test)]
impl Drop for BeforeSupportEvidenceHookGuard {
    fn drop(&mut self) {
        BEFORE_SUPPORT_EVIDENCE_HOOK.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

#[cfg(test)]
#[must_use]
fn set_before_support_evidence_hook_for_test(
    hook: impl FnOnce() + 'static,
) -> BeforeSupportEvidenceHookGuard {
    BEFORE_SUPPORT_EVIDENCE_HOOK.with(|slot| {
        let mut slot = slot.borrow_mut();
        assert!(slot.is_none(), "support evidence hook leaked");
        *slot = Some(Box::new(hook));
    });
    BeforeSupportEvidenceHookGuard
}

fn run_before_support_evidence_hook() {
    #[cfg(test)]
    BEFORE_SUPPORT_EVIDENCE_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

pub(crate) struct RuntimeInvocationPlan {
    pub(crate) args: Map<String, Value>,
    pub(crate) warnings: Vec<String>,
    pub(crate) build_preflight: Option<RuntimeBuildPreflight>,
    incremental_support_evidence: Vec<(String, WorkspaceConfigurationSupportEvidence)>,
    incremental_source_map_provenance: Option<ProjectSourceMapProvenance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunnerProjectFormat {
    Designer,
    Edt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RuntimeBuildPreflight {
    cwd: PathBuf,
    workspace_root: PathBuf,
    workspace_epoch: u64,
    config: String,
    config_sha256: [u8; 32],
    source_set: Option<String>,
}

impl RuntimeBuildPreflight {
    pub(crate) fn capture(
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<Self, String> {
        let current = discover_workspace(Some(context.cwd.clone())).map_err(|error| {
            format!(
                "incremental build authorization could not rediscover the workspace: {error}; \
                 retry with `fullRebuild: true`"
            )
        })?;
        if path_lock_identity(&current.workspace_root)
            != path_lock_identity(&context.workspace_root)
            || current.workspace_epoch != context.workspace_epoch
        {
            return Err(
                "incremental build workspace identity changed during authorization; retry with \
                 `fullRebuild: true`"
                    .to_string(),
            );
        }
        let primary_config = ensure_primary_runtime_config(args, &current)?;
        let config = primary_runtime_config_argument(&primary_config)?;
        let config_sha256 = runtime_config_digest(&current.workspace_root, &primary_config)?;
        let source_set = match args.get("sourceSet") {
            Some(Value::String(value)) => Some(value.clone()),
            Some(_) => {
                return Err("operation `build` argument `sourceSet` must be string".to_string())
            }
            None => None,
        };
        Ok(Self {
            cwd: context.cwd.clone(),
            workspace_root: current.workspace_root,
            workspace_epoch: current.workspace_epoch,
            config,
            config_sha256,
            source_set,
        })
    }

    fn args(&self) -> Map<String, Value> {
        let mut args = Map::from_iter([("operation".to_string(), Value::String("build".into()))]);
        args.insert("config".to_string(), Value::String(self.config.clone()));
        if let Some(source_set) = &self.source_set {
            args.insert("sourceSet".to_string(), Value::String(source_set.clone()));
        }
        args
    }

    pub(crate) fn reauthorize_with_reader(
        &self,
        context: &WorkspaceContext,
        support_reader: &dyn SupportStateReader,
    ) -> Result<(), String> {
        if path_lock_identity(&self.workspace_root) != path_lock_identity(&context.workspace_root) {
            return Err(workspace_identity_changed_error());
        }
        let current = self.rediscover_current_workspace()?;
        self.reauthorize_in_context(&current, support_reader)
    }

    fn reauthorize_in_context(
        &self,
        context: &WorkspaceContext,
        support_reader: &dyn SupportStateReader,
    ) -> Result<(), String> {
        let prelaunch = plan_runtime_invocation(&self.args(), context, support_reader)?;
        ensure_incremental_support_evidence(&prelaunch)?;
        // The support read can overlap a checkout or config rewrite. Rebind
        // every path and exact config byte, then revalidate the opaque marker
        // snapshots without asking the reader for another stale observation.
        self.rediscover_current_workspace()?;
        ensure_incremental_support_evidence(&prelaunch)?;
        Ok(())
    }

    pub(crate) fn reauthorize_current_workspace(&self) -> Result<(), String> {
        let context = self.rediscover_current_workspace()?;
        let support_reader_factory = WorkspaceSupportStateReaderFactory;
        let support_reader = support_reader_factory.create(&context);
        self.reauthorize_in_context(&context, support_reader.as_ref())
    }

    fn rediscover_current_workspace(&self) -> Result<WorkspaceContext, String> {
        let context = discover_workspace(Some(self.cwd.clone())).map_err(|error| {
            format!(
                "incremental build prelaunch authorization could not rediscover the workspace: \
                 {error}; retry with `fullRebuild: true`"
            )
        })?;
        self.ensure_same_workspace(&context)?;
        Ok(context)
    }

    fn ensure_same_workspace(&self, context: &WorkspaceContext) -> Result<(), String> {
        if path_lock_identity(&self.workspace_root) != path_lock_identity(&context.workspace_root)
            || self.workspace_epoch != context.workspace_epoch
        {
            return Err(workspace_identity_changed_error());
        }
        let current_config = ensure_primary_runtime_config(&self.args(), context)?;
        if Path::new(&self.config) != current_config
            || self.config_sha256
                != runtime_config_digest(&context.workspace_root, &current_config)?
        {
            return Err(project_config_changed_error());
        }
        Ok(())
    }
}

fn ensure_incremental_support_evidence(plan: &RuntimeInvocationPlan) -> Result<(), String> {
    if plan.args.get("fullRebuild").and_then(Value::as_bool) == Some(true) {
        return Err(
            "incremental build support evidence changed before v8-runner launch; retry with \
             `fullRebuild: true`"
                .to_string(),
        );
    }
    revalidate_incremental_support_evidence(&plan.incremental_support_evidence)?;
    if let Some(provenance) = &plan.incremental_source_map_provenance {
        provenance.revalidate().map_err(|error| {
            format!(
                "incremental build project source-map evidence changed or became unavailable: \
                 {error}; retry with `fullRebuild: true`"
            )
        })?;
    }
    Ok(())
}

fn revalidate_incremental_support_evidence(
    evidence: &[(String, WorkspaceConfigurationSupportEvidence)],
) -> Result<(), String> {
    for (source_set, evidence) in evidence {
        evidence.revalidate().map_err(|error| {
            format!(
                "incremental build support evidence changed or became unavailable for source-set \
                 `{source_set}`: {error}; retry with `fullRebuild: true`"
            )
        })?;
    }
    Ok(())
}

fn workspace_identity_changed_error() -> String {
    "incremental build workspace identity changed before v8-runner launch; retry with \
     `fullRebuild: true`"
        .to_string()
}

fn project_config_changed_error() -> String {
    "incremental build project config changed before v8-runner launch; retry with \
     `fullRebuild: true`"
        .to_string()
}

/// Designer cannot apply a partial file load to an actively supported
/// configuration. v8-runner owns the incremental baseline, so this boundary
/// deliberately makes only the decision it can prove from source evidence:
/// any selected supported Platform XML configuration takes the runner's full
/// build path before a platform process is started (#404, ADR-0059).
pub(crate) fn plan_runtime_invocation(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    support_reader: &dyn SupportStateReader,
) -> Result<RuntimeInvocationPlan, String> {
    let mut planned_args = args.clone();
    if args.get("operation").and_then(Value::as_str) != Some("build") {
        return Ok(RuntimeInvocationPlan {
            args: planned_args,
            warnings: Vec::new(),
            build_preflight: None,
            incremental_support_evidence: Vec::new(),
            incremental_source_map_provenance: None,
        });
    }
    if args.contains_key("fullRebuild")
        && args.get("fullRebuild").and_then(Value::as_bool).is_none()
    {
        return Err("operation `build` argument `fullRebuild` must be boolean".to_string());
    }
    if args.get("fullRebuild").and_then(Value::as_bool) == Some(true) {
        return Ok(RuntimeInvocationPlan {
            args: planned_args,
            warnings: Vec::new(),
            build_preflight: None,
            incremental_support_evidence: Vec::new(),
            incremental_source_map_provenance: None,
        });
    }

    let primary_config = ensure_primary_runtime_config(args, context)?;
    // The pinned runner also reads this option from V8TR_CONFIG. Always replace
    // the caller's spelling with the already-proved primary path: besides
    // defeating inherited process state, this removes retargetable symlink
    // aliases from the command that crosses the spawn boundary.
    planned_args.insert(
        "config".to_string(),
        Value::String(primary_runtime_config_argument(&primary_config)?),
    );
    let selected_source_set = match args.get("sourceSet") {
        Some(Value::String(value)) => Some(value.trim()),
        Some(_) => return Err("operation `build` argument `sourceSet` must be string".to_string()),
        None => None,
    };
    if let Some(selected) = selected_source_set {
        if selected.is_empty() {
            return Err(
                "incremental build preflight requires a non-empty `sourceSet`; retry with \
                 `fullRebuild: true`"
                    .to_string(),
            );
        }
        planned_args.insert("sourceSet".to_string(), Value::String(selected.to_string()));
    }
    let (source_map, source_map_provenance) =
        crate::infrastructure::project_sources::discover_runtime_project_source_map_with_provenance(
            &context.workspace_root,
        )
        .map_err(|error| {
            format!(
                "incremental build preflight could not read the project source map: {error}; \
                 retry with `fullRebuild: true`"
            )
        })?;
    let selected_source_sets = source_map
        .source_sets
        .iter()
        .filter(|source_set| selected_source_set.is_none_or(|selected| selected == source_set.name))
        .collect::<Vec<_>>();
    if let Some(selected) = selected_source_set {
        if selected_source_sets.len() != 1 {
            return Err(format!(
                "incremental build preflight must resolve source-set `{selected}` exactly one \
                 time, found {}; retry with `fullRebuild: true`",
                selected_source_sets.len()
            ));
        }
    }
    let runner_format = runner_project_format(source_map.configured_format_raw.as_deref())?;

    let mut supported_source_sets = Vec::new();
    let mut incremental_support_evidence = Vec::new();
    for source_set in selected_source_sets {
        if source_set.kind != SourceSetKind::Configuration {
            continue;
        }
        if runner_format == RunnerProjectFormat::Edt {
            continue;
        }
        match source_set.source_format {
            SourceFormat::PlatformXml => {}
            SourceFormat::Edt | SourceFormat::Unknown | SourceFormat::Invalid => {
                return Err(format!(
                    "incremental Designer build preflight could not prove Platform XML source \
                     format for configuration source-set `{}` ({:?}); retry with \
                     `fullRebuild: true`",
                    source_set.name, source_set.source_format
                ));
            }
        }
        let target = ResolvedTarget {
            source_set: source_set.name.clone(),
            metadata_path: None,
            target_kind: TargetKind::SourceRoot,
        };
        let expected_source_root =
            normalize_contained_source_root(&context.workspace_root, &source_set.path).map_err(
                |error| {
                    format!(
                        "incremental build preflight could not bind source-set `{}` to the \
                         project source map: {error}; retry with `fullRebuild: true`",
                        source_set.name
                    )
                },
            )?;
        run_before_support_evidence_hook();
        let evidence = capture_workspace_configuration_support_evidence(
            context,
            &target,
            &expected_source_root,
        );
        let support = support_reader
            .configuration_support(&target)
            .map_err(|error| {
                format!(
                    "incremental build preflight could not determine support state for source-set \
                     `{}`: {error}; retry with `fullRebuild: true`",
                    source_set.name
                )
            })?;
        let evidence = evidence.map_err(|error| {
            format!(
                "incremental build preflight could not capture support evidence for source-set \
                 `{}`: {error}; retry with `fullRebuild: true`",
                source_set.name
            )
        })?;
        source_map_provenance.revalidate().map_err(|error| {
            format!(
                "incremental build project source-map evidence changed while reading support \
                 for source-set `{}`: {error}; retry with `fullRebuild: true`",
                source_set.name
            )
        })?;
        match support.state {
            ConfigurationSupportState::Supported => {
                supported_source_sets.push(source_set.name.clone());
            }
            ConfigurationSupportState::NotSupported | ConfigurationSupportState::Removed => {
                if !evidence.permits_observation(support.state) {
                    return Err(format!(
                        "incremental build preflight received support state for source-set `{}` \
                         that contradicts the provider evidence; retry with `fullRebuild: true`",
                        source_set.name
                    ));
                }
                evidence.revalidate().map_err(|error| {
                    format!(
                        "incremental build support evidence changed or became unavailable for \
                         source-set `{}`: {error}; retry with `fullRebuild: true`",
                        source_set.name
                    )
                })?;
                incremental_support_evidence.push((source_set.name.clone(), evidence));
            }
            ConfigurationSupportState::Extension => {
                return Err(format!(
                    "incremental build preflight received inconsistent `Extension` support state \
                     for configuration source-set `{}`; retry with `fullRebuild: true`",
                    source_set.name
                ));
            }
        }
    }

    if supported_source_sets.is_empty() {
        let build_preflight = RuntimeBuildPreflight::capture(&planned_args, context)?;
        revalidate_incremental_support_evidence(&incremental_support_evidence)?;
        source_map_provenance.revalidate().map_err(|error| {
            format!(
                "incremental build project source-map evidence changed before authorization: \
                 {error}; retry with `fullRebuild: true`"
            )
        })?;
        return Ok(RuntimeInvocationPlan {
            args: planned_args,
            warnings: Vec::new(),
            build_preflight: Some(build_preflight),
            incremental_support_evidence,
            incremental_source_map_provenance: Some(source_map_provenance),
        });
    }

    supported_source_sets.sort();
    let source_sets = supported_source_sets
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ");
    planned_args.insert("fullRebuild".to_string(), Value::Bool(true));
    Ok(RuntimeInvocationPlan {
        args: planned_args,
        warnings: vec![format!(
            "vendor-supported configuration source-set(s) {source_sets} cannot use Designer \
             partial loading; Unica selected a full rebuild before starting v8-runner"
        )],
        build_preflight: None,
        incremental_support_evidence: Vec::new(),
        incremental_source_map_provenance: None,
    })
}

fn runner_project_format(raw: Option<&str>) -> Result<RunnerProjectFormat, String> {
    match raw {
        None | Some("DESIGNER") => Ok(RunnerProjectFormat::Designer),
        Some("EDT") => Ok(RunnerProjectFormat::Edt),
        Some(value) => Err(format!(
            "incremental build preflight cannot use project `format` {value:?}: the pinned \
             v8-runner accepts only exact `DESIGNER` or `EDT`; correct `v8project.yaml` before \
             retrying"
        )),
    }
}

fn ensure_primary_runtime_config(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let configured = match args.get("config") {
        Some(Value::String(path)) => PathBuf::from(path),
        Some(_) => return Err("operation `build` argument `config` must be string".to_string()),
        None => PathBuf::from("v8project.yaml"),
    };
    let effective = if configured.is_absolute() {
        configured
    } else {
        context.cwd.join(configured)
    };
    let primary = context.workspace_root.join("v8project.yaml");
    let effective = normalize_path_identity(&effective).map_err(|error| {
        format!(
            "incremental build preflight could not resolve config `{}`: {error}; retry with \
             `fullRebuild: true`",
            effective.display()
        )
    })?;
    let primary = normalize_path_identity(&primary).map_err(|error| {
        format!(
            "incremental build preflight could not resolve the workspace config `{}`: {error}; \
             retry with `fullRebuild: true`",
            primary.display()
        )
    })?;
    if !same_resolved_runtime_config(&effective, &primary) {
        return Err(format!(
            "incremental build preflight cannot bind non-primary config `{}` to the workspace \
             support-state reader; retry with `fullRebuild: true`",
            effective.display()
        ));
    }
    Ok(primary)
}

fn primary_runtime_config_argument(canonical_primary: &Path) -> Result<String, String> {
    canonical_primary
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| {
            "incremental build preflight could not represent the primary config for v8-runner; \
             retry with `fullRebuild: true`"
                .to_string()
        })
}

fn runtime_config_digest(
    workspace_root: &Path,
    canonical_primary: &Path,
) -> Result<[u8; 32], String> {
    let workspace_root = normalize_path_identity(workspace_root).map_err(|error| {
        format!(
            "incremental build preflight could not resolve the workspace root for config \
             verification: {error}; retry with `fullRebuild: true`"
        )
    })?;
    let bytes = read_root_relative_regular_file(
        &workspace_root,
        canonical_primary,
        PROJECT_SOURCE_MAP_INPUT_MAX_BYTES,
        |_| {},
    )
    .map_err(|error| {
        format!(
            "incremental build preflight could not securely read the workspace config `{}` \
             within its byte limit: {error}; retry with `fullRebuild: true`",
            canonical_primary.display()
        )
    })?
    .bytes;
    Ok(Sha256::digest(&bytes).into())
}

fn same_resolved_runtime_config(effective: &std::path::Path, primary: &std::path::Path) -> bool {
    effective == primary
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File, FileTimes};
    use std::sync::atomic::{AtomicBool, Ordering};
    use tempfile::TempDir;

    struct HeadSwitchingSupportReader {
        head: PathBuf,
    }

    struct ConfigRewritingSupportReader {
        config: PathBuf,
        replacement: Vec<u8>,
        modified: std::time::SystemTime,
    }

    struct SupportAppearingAfterRead {
        context: WorkspaceContext,
        marker: PathBuf,
        changed: AtomicBool,
    }

    #[test]
    fn unconsumed_support_evidence_hook_does_not_leak_past_its_scope() {
        {
            let _guard = set_before_support_evidence_hook_for_test(|| {});
        }

        let _guard = set_before_support_evidence_hook_for_test(|| {});
        run_before_support_evidence_hook();
    }

    struct ConstantConfigurationSupportReader(ConfigurationSupportState);

    struct RestoringWorkspaceSupportReader {
        context: WorkspaceContext,
        config: PathBuf,
        original: Vec<u8>,
        modified: std::time::SystemTime,
    }

    impl SupportStateReader for ConstantConfigurationSupportReader {
        fn configuration_support(
            &self,
            _target: &ResolvedTarget,
        ) -> Result<
            crate::domain::support_state::ConfigurationSupportData,
            crate::domain::support_state::SupportReadError,
        > {
            Ok(crate::domain::support_state::ConfigurationSupportData {
                state: self.0,
                editing_enabled: None,
                objects: None,
            })
        }

        fn object_support(
            &self,
            _target: &ResolvedTarget,
        ) -> Result<
            crate::domain::support_state::ObjectSupportData,
            crate::domain::support_state::SupportReadError,
        > {
            unreachable!("runtime build preflight reads configuration support")
        }

        fn subsystem_support(
            &self,
            _target: &crate::domain::support_state::ResolvedSubsystemTarget,
        ) -> Result<
            crate::domain::support_state::ObjectSupportData,
            crate::domain::support_state::SupportReadError,
        > {
            unreachable!("runtime build preflight reads configuration support")
        }
    }

    impl SupportStateReader for RestoringWorkspaceSupportReader {
        fn configuration_support(
            &self,
            target: &ResolvedTarget,
        ) -> Result<
            crate::domain::support_state::ConfigurationSupportData,
            crate::domain::support_state::SupportReadError,
        > {
            let support = crate::infrastructure::support_state::WorkspaceSupportStateReader::new(
                &self.context,
            )
            .configuration_support(target)?;
            fs::write(&self.config, &self.original).expect("restore original project config");
            File::options()
                .write(true)
                .open(&self.config)
                .expect("open restored project config")
                .set_times(FileTimes::new().set_modified(self.modified))
                .expect("restore project config mtime");
            Ok(support)
        }

        fn object_support(
            &self,
            _target: &ResolvedTarget,
        ) -> Result<
            crate::domain::support_state::ObjectSupportData,
            crate::domain::support_state::SupportReadError,
        > {
            unreachable!("runtime build preflight reads configuration support")
        }

        fn subsystem_support(
            &self,
            _target: &crate::domain::support_state::ResolvedSubsystemTarget,
        ) -> Result<
            crate::domain::support_state::ObjectSupportData,
            crate::domain::support_state::SupportReadError,
        > {
            unreachable!("runtime build preflight reads configuration support")
        }
    }

    impl SupportStateReader for HeadSwitchingSupportReader {
        fn configuration_support(
            &self,
            _target: &ResolvedTarget,
        ) -> Result<
            crate::domain::support_state::ConfigurationSupportData,
            crate::domain::support_state::SupportReadError,
        > {
            fs::write(&self.head, "ref: refs/heads/feature-b\n").expect("switch workspace HEAD");
            Ok(crate::domain::support_state::ConfigurationSupportData {
                state: ConfigurationSupportState::NotSupported,
                editing_enabled: None,
                objects: None,
            })
        }

        fn object_support(
            &self,
            _target: &ResolvedTarget,
        ) -> Result<
            crate::domain::support_state::ObjectSupportData,
            crate::domain::support_state::SupportReadError,
        > {
            unreachable!("runtime build preflight reads configuration support")
        }

        fn subsystem_support(
            &self,
            _target: &crate::domain::support_state::ResolvedSubsystemTarget,
        ) -> Result<
            crate::domain::support_state::ObjectSupportData,
            crate::domain::support_state::SupportReadError,
        > {
            unreachable!("runtime build preflight reads configuration support")
        }
    }

    impl SupportStateReader for ConfigRewritingSupportReader {
        fn configuration_support(
            &self,
            _target: &ResolvedTarget,
        ) -> Result<
            crate::domain::support_state::ConfigurationSupportData,
            crate::domain::support_state::SupportReadError,
        > {
            fs::write(&self.config, &self.replacement).expect("rewrite project config");
            File::options()
                .write(true)
                .open(&self.config)
                .expect("open rewritten project config")
                .set_times(FileTimes::new().set_modified(self.modified))
                .expect("restore project config mtime");
            Ok(crate::domain::support_state::ConfigurationSupportData {
                state: ConfigurationSupportState::NotSupported,
                editing_enabled: None,
                objects: None,
            })
        }

        fn object_support(
            &self,
            _target: &ResolvedTarget,
        ) -> Result<
            crate::domain::support_state::ObjectSupportData,
            crate::domain::support_state::SupportReadError,
        > {
            unreachable!("runtime build preflight reads configuration support")
        }

        fn subsystem_support(
            &self,
            _target: &crate::domain::support_state::ResolvedSubsystemTarget,
        ) -> Result<
            crate::domain::support_state::ObjectSupportData,
            crate::domain::support_state::SupportReadError,
        > {
            unreachable!("runtime build preflight reads configuration support")
        }
    }

    impl SupportStateReader for SupportAppearingAfterRead {
        fn configuration_support(
            &self,
            target: &ResolvedTarget,
        ) -> Result<
            crate::domain::support_state::ConfigurationSupportData,
            crate::domain::support_state::SupportReadError,
        > {
            let reader = crate::infrastructure::support_state::WorkspaceSupportStateReader::new(
                &self.context,
            );
            let support = reader.configuration_support(target)?;
            if !self.changed.swap(true, Ordering::SeqCst) {
                fs::create_dir_all(self.marker.parent().expect("support marker parent"))
                    .expect("create support marker parent");
                fs::write(
                    &self.marker,
                    include_bytes!(
                        "../../../../tests/fixtures/platform_8_3_27/support-edit-bin-only/src/Ext/ParentConfigurations.bin"
                    ),
                )
                .expect("publish support marker after stale read");
            }
            Ok(support)
        }

        fn object_support(
            &self,
            _target: &ResolvedTarget,
        ) -> Result<
            crate::domain::support_state::ObjectSupportData,
            crate::domain::support_state::SupportReadError,
        > {
            unreachable!("runtime build preflight reads configuration support")
        }

        fn subsystem_support(
            &self,
            _target: &crate::domain::support_state::ResolvedSubsystemTarget,
        ) -> Result<
            crate::domain::support_state::ObjectSupportData,
            crate::domain::support_state::SupportReadError,
        > {
            unreachable!("runtime build preflight reads configuration support")
        }
    }

    #[test]
    fn runtime_config_identity_never_folds_case_after_resolution() {
        assert!(!same_resolved_runtime_config(
            std::path::Path::new("/workspace/V8PROJECT.YAML"),
            std::path::Path::new("/workspace/v8project.yaml"),
        ));
    }

    #[test]
    fn authorization_capture_rejects_an_oversized_config_before_digesting() {
        let root = TempDir::new().expect("create workspace");
        let workspace = root.path().canonicalize().expect("canonical workspace");
        fs::File::create(workspace.join("v8project.yaml"))
            .expect("create project config")
            .set_len(
                (crate::infrastructure::project_sources::PROJECT_SOURCE_MAP_INPUT_MAX_BYTES + 1)
                    as u64,
            )
            .expect("size project config");
        let context = discover_workspace(Some(workspace)).expect("discover workspace");
        let args = Map::from_iter([("operation".to_string(), Value::String("build".into()))]);

        let error = RuntimeBuildPreflight::capture(&args, &context)
            .expect_err("oversized config must fail at the bounded digest read");

        assert!(error.contains("byte limit"), "{error}");
    }

    #[test]
    fn durable_reauthorization_preserves_nested_cwd_for_relative_config() {
        let root = TempDir::new().expect("create workspace");
        let workspace = root.path().canonicalize().expect("canonical workspace");
        let nested = workspace.join("nested");
        let source_root = workspace.join("src");
        fs::create_dir_all(&nested).expect("create nested cwd");
        fs::create_dir_all(&source_root).expect("create source root");
        fs::write(
            workspace.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: src\n",
            ),
        )
        .expect("write project config");
        fs::write(
            source_root.join("Configuration.xml"),
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/support-edit-bin-only/src/Configuration.xml"
            ),
        )
        .expect("write configuration root");
        let context = discover_workspace(Some(nested.clone())).expect("discover nested workspace");
        assert_eq!(context.cwd, nested);
        let args = Map::from_iter([
            ("operation".to_string(), Value::String("build".to_string())),
            (
                "config".to_string(),
                Value::String("../v8project.yaml".to_string()),
            ),
        ]);
        let authorization = RuntimeBuildPreflight::capture(&args, &context).unwrap();

        authorization
            .reauthorize_current_workspace()
            .expect("worker must resolve config from the original process cwd");
    }

    #[test]
    fn planned_config_does_not_retain_a_retargetable_primary_alias() {
        let root = TempDir::new().expect("create workspace");
        let workspace = root.path().canonicalize().expect("canonical workspace");
        let primary = workspace.join("v8project.yaml");
        let alternate = workspace.join("alternate.yaml");
        let alias = workspace.join("config-alias.yaml");
        fs::write(&primary, "format: DESIGNER\nsource-set: []\n").expect("write primary config");
        fs::write(&alternate, "format: DESIGNER\nsource-set: []\n")
            .expect("write alternate config");
        let Some(link_result) =
            crate::infrastructure::platform::filesystem::create_file_symlink_for_test(
                &primary, &alias,
            )
        else {
            return;
        };
        link_result.expect("create primary config alias");
        let context = discover_workspace(Some(workspace.clone())).expect("discover workspace");
        let args = Map::from_iter([
            ("operation".to_string(), Value::String("build".into())),
            (
                "config".to_string(),
                Value::String("config-alias.yaml".into()),
            ),
        ]);
        let support_reader_factory = WorkspaceSupportStateReaderFactory;
        let support_reader = support_reader_factory.create(&context);

        let plan = plan_runtime_invocation(&args, &context, support_reader.as_ref())
            .expect("authorize alias while it names the primary config");
        fs::remove_file(&alias).expect("remove primary config alias");
        crate::infrastructure::platform::filesystem::create_file_symlink_for_test(
            &alternate, &alias,
        )
        .expect("file links are supported")
        .expect("retarget config alias");
        let planned_config = plan
            .args
            .get("config")
            .and_then(Value::as_str)
            .expect("planned config");
        let resolved = normalize_path_identity(&context.cwd.join(planned_config))
            .expect("resolve planned config after alias retarget");
        let primary = normalize_path_identity(&primary).expect("resolve primary config");

        assert_eq!(resolved, primary);
    }

    #[test]
    fn planned_config_survives_nested_cwd_replacement() {
        let root = TempDir::new().expect("create root");
        let workspace = root.path().join("workspace");
        let nested = workspace.join("nested");
        let attacker = root.path().join("attacker/nested");
        fs::create_dir_all(&nested).expect("create nested cwd");
        fs::create_dir_all(&attacker).expect("create replacement cwd");
        let primary = workspace.join("v8project.yaml");
        let alternate = root.path().join("attacker/v8project.yaml");
        fs::write(&primary, "format: DESIGNER\nsource-set: []\n").expect("write primary config");
        fs::write(&alternate, "format: EDT\nsource-set: []\n").expect("write alternate config");
        let context = discover_workspace(Some(nested.clone())).expect("discover workspace");
        let args = Map::from_iter([
            ("operation".to_string(), Value::String("build".into())),
            (
                "config".to_string(),
                Value::String("../v8project.yaml".into()),
            ),
        ]);
        let support_reader_factory = WorkspaceSupportStateReaderFactory;
        let support_reader = support_reader_factory.create(&context);

        let plan = plan_runtime_invocation(&args, &context, support_reader.as_ref())
            .expect("plan incremental build");
        fs::remove_dir(&nested).expect("remove original cwd");
        let Some(link_result) =
            crate::infrastructure::platform::filesystem::create_dir_symlink_for_test(
                &attacker, &nested,
            )
        else {
            return;
        };
        link_result.expect("replace cwd with directory link");
        let planned_config = plan
            .args
            .get("config")
            .and_then(Value::as_str)
            .expect("planned config");
        let resolved = normalize_path_identity(&context.cwd.join(planned_config))
            .expect("resolve planned config after cwd replacement");
        let primary = normalize_path_identity(&primary).expect("resolve primary config");

        assert_eq!(resolved, primary);
    }

    #[test]
    fn durable_reauthorization_rejects_a_head_change_in_the_same_workspace_path() {
        let root = TempDir::new().expect("create workspace");
        let workspace = root.path().canonicalize().expect("canonical workspace");
        fs::create_dir_all(workspace.join(".git")).expect("create git metadata");
        fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set: []\n",
        )
        .expect("write workspace config");
        fs::write(workspace.join(".git/HEAD"), "ref: refs/heads/feature-a\n")
            .expect("write initial HEAD");
        let context = discover_workspace(Some(workspace.clone())).expect("discover workspace");
        let args = Map::from_iter([("operation".to_string(), Value::String("build".into()))]);
        let authorization = RuntimeBuildPreflight::capture(&args, &context).unwrap();

        fs::write(workspace.join(".git/HEAD"), "ref: refs/heads/feature-b\n")
            .expect("switch workspace HEAD");
        let error = authorization
            .reauthorize_current_workspace()
            .expect_err("authorization from another workspace epoch must not launch");

        assert!(error.contains("workspace identity changed"), "{error}");
        assert!(error.contains("fullRebuild: true"), "{error}");
    }

    #[test]
    fn initial_authorization_rejects_an_epoch_change_during_support_read() {
        let root = TempDir::new().expect("create workspace");
        let workspace = root.path().canonicalize().expect("canonical workspace");
        let source = workspace.join("src");
        fs::create_dir_all(workspace.join(".git")).expect("create git metadata");
        fs::create_dir_all(&source).expect("create source root");
        fs::write(
            workspace.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: src\n",
            ),
        )
        .expect("write workspace config");
        fs::write(source.join("Configuration.xml"), "<MetaDataObject/>")
            .expect("write configuration root");
        let head = workspace.join(".git/HEAD");
        fs::write(&head, "ref: refs/heads/feature-a\n").expect("write initial HEAD");
        let context = discover_workspace(Some(workspace)).expect("discover workspace");
        let args = Map::from_iter([("operation".to_string(), Value::String("build".into()))]);

        let error =
            match plan_runtime_invocation(&args, &context, &HeadSwitchingSupportReader { head }) {
                Ok(_) => panic!("initial authorization must retain its invocation epoch"),
                Err(error) => error,
            };

        assert!(error.contains("workspace identity changed"), "{error}");
        assert!(error.contains("fullRebuild: true"), "{error}");
    }

    #[test]
    fn synchronous_reauthorization_rechecks_epoch_after_support_read() {
        let root = TempDir::new().expect("create workspace");
        let workspace = root.path().canonicalize().expect("canonical workspace");
        let source = workspace.join("src");
        fs::create_dir_all(workspace.join(".git")).expect("create git metadata");
        fs::create_dir_all(&source).expect("create source root");
        fs::write(
            workspace.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: src\n",
            ),
        )
        .expect("write workspace config");
        fs::write(source.join("Configuration.xml"), "<MetaDataObject/>")
            .expect("write configuration root");
        let head = workspace.join(".git/HEAD");
        fs::write(&head, "ref: refs/heads/feature-a\n").expect("write initial HEAD");
        let context = discover_workspace(Some(workspace)).expect("discover workspace");
        let args = Map::from_iter([("operation".to_string(), Value::String("build".into()))]);
        let authorization = RuntimeBuildPreflight::capture(&args, &context).unwrap();

        let error = authorization
            .reauthorize_with_reader(&context, &HeadSwitchingSupportReader { head })
            .expect_err("workspace epoch changed during support read must not launch");

        assert!(error.contains("workspace identity changed"), "{error}");
        assert!(error.contains("fullRebuild: true"), "{error}");
    }

    #[test]
    fn synchronous_reauthorization_rechecks_exact_config_after_support_read() {
        let root = TempDir::new().expect("create workspace");
        let workspace = root.path().canonicalize().expect("canonical workspace");
        let source = workspace.join("src-a");
        fs::create_dir_all(&source).expect("create source root");
        fs::write(source.join("Configuration.xml"), "<MetaDataObject/>")
            .expect("write configuration root");
        let initial = concat!(
            "format: DESIGNER\n",
            "source-set:\n",
            "  - name: main\n",
            "    type: CONFIGURATION\n",
            "    path: src-a\n",
        );
        let replacement = concat!(
            "format: DESIGNER\n",
            "source-set:\n",
            "  - name: main\n",
            "    type: CONFIGURATION\n",
            "    path: src-b\n",
        );
        assert_eq!(initial.len(), replacement.len());
        let config = workspace.join("v8project.yaml");
        fs::write(&config, initial).expect("write initial project config");
        let modified = fs::metadata(&config)
            .expect("read project config metadata")
            .modified()
            .expect("read project config mtime");
        let context = discover_workspace(Some(workspace)).expect("discover workspace");
        let args = Map::from_iter([("operation".to_string(), Value::String("build".into()))]);
        let authorization = RuntimeBuildPreflight::capture(&args, &context).unwrap();

        let error = authorization
            .reauthorize_with_reader(
                &context,
                &ConfigRewritingSupportReader {
                    config,
                    replacement: replacement.as_bytes().to_vec(),
                    modified,
                },
            )
            .expect_err("config bytes changed during support read must not launch");
        let current = discover_workspace(Some(context.cwd)).expect("rediscover workspace");

        assert_eq!(context.workspace_epoch, current.workspace_epoch);
        assert!(
            error.contains("project source-map evidence changed"),
            "{error}"
        );
        assert!(error.contains("fullRebuild: true"), "{error}");
    }

    #[test]
    fn synchronous_reauthorization_rechecks_support_after_stale_reader_result() {
        let root = TempDir::new().expect("create workspace");
        let workspace = root.path().canonicalize().expect("canonical workspace");
        let source = workspace.join("src");
        fs::create_dir_all(&source).expect("create source root");
        fs::write(
            workspace.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: src\n",
            ),
        )
        .expect("write project config");
        fs::write(
            source.join("Configuration.xml"),
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/support-edit-bin-only/src/Configuration.xml"
            ),
        )
        .expect("write configuration root");
        let context = discover_workspace(Some(workspace)).expect("discover workspace");
        let args = Map::from_iter([("operation".to_string(), Value::String("build".into()))]);
        let authorization = RuntimeBuildPreflight::capture(&args, &context).unwrap();
        let marker = source.join("Ext/ParentConfigurations.bin");

        let error = authorization
            .reauthorize_with_reader(
                &context,
                &SupportAppearingAfterRead {
                    context: context.clone(),
                    marker,
                    changed: AtomicBool::new(false),
                },
            )
            .expect_err("support published after a stale read must prevent incremental launch");

        assert!(error.contains("support evidence changed"), "{error}");
        assert!(error.contains("fullRebuild: true"), "{error}");
    }

    #[test]
    fn incremental_plan_rejects_a_reader_result_that_contradicts_supported_evidence() {
        let root = TempDir::new().expect("create workspace");
        let workspace = root.path().canonicalize().expect("canonical workspace");
        let source = workspace.join("src");
        fs::create_dir_all(source.join("Ext")).expect("create source root");
        fs::write(
            workspace.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: src\n",
            ),
        )
        .expect("write project config");
        fs::write(
            source.join("Configuration.xml"),
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/support-edit-bin-only/src/Configuration.xml"
            ),
        )
        .expect("write configuration root");
        fs::write(
            source.join("Ext/ParentConfigurations.bin"),
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/support-edit-bin-only/src/Ext/ParentConfigurations.bin"
            ),
        )
        .expect("write supported marker");
        let context = discover_workspace(Some(workspace)).expect("discover workspace");
        let args = Map::from_iter([("operation".to_string(), Value::String("build".into()))]);

        let error = match plan_runtime_invocation(
            &args,
            &context,
            &ConstantConfigurationSupportReader(ConfigurationSupportState::NotSupported),
        ) {
            Ok(_) => panic!("supported evidence must reject a less restrictive reader result"),
            Err(error) => error,
        };

        assert!(
            error.contains("contradicts the provider evidence"),
            "{error}"
        );
        assert!(error.contains("fullRebuild: true"), "{error}");
    }

    #[test]
    fn incremental_plan_rejects_config_source_map_aba_during_support_evidence() {
        let root = TempDir::new().expect("create workspace");
        let workspace = root.path().canonicalize().expect("canonical workspace");
        let source_a = workspace.join("src-a");
        let source_b = workspace.join("src-b");
        fs::create_dir_all(source_a.join("Ext")).expect("create supported source root");
        fs::create_dir_all(&source_b).expect("create unsupported source root");
        for source in [&source_a, &source_b] {
            fs::write(
                source.join("Configuration.xml"),
                include_bytes!(
                    "../../../../tests/fixtures/platform_8_3_27/support-edit-bin-only/src/Configuration.xml"
                ),
            )
            .expect("write configuration root");
        }
        fs::write(
            source_a.join("Ext/ParentConfigurations.bin"),
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/support-edit-bin-only/src/Ext/ParentConfigurations.bin"
            ),
        )
        .expect("write supported marker");
        let original = concat!(
            "format: DESIGNER\n",
            "source-set:\n",
            "  - name: main\n",
            "    type: CONFIGURATION\n",
            "    path: src-a\n",
        );
        let replacement = concat!(
            "format: DESIGNER\n",
            "source-set:\n",
            "  - name: main\n",
            "    type: CONFIGURATION\n",
            "    path: src-b\n",
        );
        assert_eq!(original.len(), replacement.len());
        let config = workspace.join("v8project.yaml");
        fs::write(&config, original).expect("write original project config");
        let modified = fs::metadata(&config)
            .expect("project config metadata")
            .modified()
            .expect("project config mtime");
        let context = discover_workspace(Some(workspace)).expect("discover workspace");
        let args = Map::from_iter([("operation".to_string(), Value::String("build".into()))]);
        let config_for_hook = config.clone();
        let _hook_guard = set_before_support_evidence_hook_for_test(move || {
            fs::write(&config_for_hook, replacement).expect("publish replacement project config");
            File::options()
                .write(true)
                .open(&config_for_hook)
                .expect("open replacement project config")
                .set_times(FileTimes::new().set_modified(modified))
                .expect("preserve project config mtime");
        });

        let result = plan_runtime_invocation(
            &args,
            &context,
            &RestoringWorkspaceSupportReader {
                context: context.clone(),
                config,
                original: original.as_bytes().to_vec(),
                modified,
            },
        );

        assert!(
            result.is_err(),
            "support evidence from a transient source map must not authorize the restored project"
        );
    }
}
