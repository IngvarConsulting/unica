use crate::domain::project_sources::{SourceFormat, SourceSetKind};
use crate::domain::source_target::{ResolvedTarget, TargetKind};
use crate::domain::support_state::{ConfigurationSupportState, SupportStateReader};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform::filesystem::path_lock_identity;
use crate::infrastructure::source_roots::normalize_path_identity;
use crate::infrastructure::support_state::{
    SupportStateReaderFactory, WorkspaceSupportStateReaderFactory,
};
use crate::infrastructure::workspace::discover_workspace;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::PathBuf;

pub(crate) struct RuntimeInvocationPlan {
    pub(crate) args: Map<String, Value>,
    pub(crate) warnings: Vec<String>,
    pub(crate) build_preflight: Option<RuntimeBuildPreflight>,
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
    config: Option<String>,
    source_set: Option<String>,
}

impl RuntimeBuildPreflight {
    pub(crate) fn capture(
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<Self, String> {
        let config = match args.get("config") {
            Some(Value::String(value)) => Some(value.clone()),
            Some(_) => return Err("operation `build` argument `config` must be string".to_string()),
            None => None,
        };
        let source_set = match args.get("sourceSet") {
            Some(Value::String(value)) => Some(value.clone()),
            Some(_) => {
                return Err("operation `build` argument `sourceSet` must be string".to_string())
            }
            None => None,
        };
        Ok(Self {
            cwd: context.cwd.clone(),
            workspace_root: context.workspace_root.clone(),
            config,
            source_set,
        })
    }

    fn args(&self) -> Map<String, Value> {
        let mut args = Map::from_iter([("operation".to_string(), Value::String("build".into()))]);
        if let Some(config) = &self.config {
            args.insert("config".to_string(), Value::String(config.clone()));
        }
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
        self.ensure_same_workspace(context)?;
        let prelaunch = plan_runtime_invocation(&self.args(), context, support_reader)?;
        if prelaunch.args.get("fullRebuild").and_then(Value::as_bool) == Some(true) {
            return Err(
                "incremental build support evidence changed before v8-runner launch; retry with \
                 `fullRebuild: true`"
                    .to_string(),
            );
        }
        Ok(())
    }

    pub(crate) fn reauthorize_current_workspace(&self) -> Result<(), String> {
        let context = discover_workspace(Some(self.cwd.clone())).map_err(|error| {
            format!(
                "incremental build prelaunch authorization could not rediscover the workspace: \
                 {error}; retry with `fullRebuild: true`"
            )
        })?;
        self.ensure_same_workspace(&context)?;
        let support_reader_factory = WorkspaceSupportStateReaderFactory;
        let support_reader = support_reader_factory.create(&context);
        self.reauthorize_with_reader(&context, support_reader.as_ref())
    }

    fn ensure_same_workspace(&self, context: &WorkspaceContext) -> Result<(), String> {
        if path_lock_identity(&self.workspace_root) != path_lock_identity(&context.workspace_root) {
            return Err(
                "incremental build workspace identity changed before v8-runner launch; retry with \
                 `fullRebuild: true`"
                    .to_string(),
            );
        }
        Ok(())
    }
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
        });
    }

    ensure_primary_runtime_config(args, context)?;
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
    let source_map = crate::infrastructure::project_sources::discover_project_source_map(
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
        let support = support_reader
            .configuration_support(&target)
            .map_err(|error| {
                format!(
                    "incremental build preflight could not determine support state for source-set \
                     `{}`: {error}; retry with `fullRebuild: true`",
                    source_set.name
                )
            })?;
        match support.state {
            ConfigurationSupportState::Supported => {
                supported_source_sets.push(source_set.name.clone());
            }
            ConfigurationSupportState::NotSupported | ConfigurationSupportState::Removed => {}
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
        return Ok(RuntimeInvocationPlan {
            args: planned_args,
            warnings: Vec::new(),
            build_preflight: Some(RuntimeBuildPreflight::capture(args, context)?),
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
) -> Result<(), String> {
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
    Ok(())
}

fn same_resolved_runtime_config(effective: &std::path::Path, primary: &std::path::Path) -> bool {
    effective == primary
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn runtime_config_identity_never_folds_case_after_resolution() {
        assert!(!same_resolved_runtime_config(
            std::path::Path::new("/workspace/V8PROJECT.YAML"),
            std::path::Path::new("/workspace/v8project.yaml"),
        ));
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
        let context = WorkspaceContext {
            cwd: nested,
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };
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
}
