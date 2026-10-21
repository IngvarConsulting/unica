//! v8-runner full-dump selection, effective YAML, platform probing and outcome mapping.
use crate::adapter::RuntimeEnvironment;
use serde_json::{Map, Value};
use serde_yaml::{Mapping as YamlMapping, Value as YamlValue};
#[cfg(any(test, feature = "test-support"))]
use std::cell::RefCell;
use std::{
    env,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_process::runner::{
    system_process_runner, ProcessCommand, ProcessOutput, ProcessRunner, ProcessStreamFailure,
};
use unica_redaction::redactor;
use unica_source_contract::{
    read::full_dump::DumpSemanticValidator, workspace::WorkspaceContext, SourceSetKind,
};
use unica_source_designer::format_profile::ACTIVE_FORMAT_PROFILE;
use unica_work_control::cancellation::CancellationToken;
use unica_workspace::full_dump::{
    current_host_platform, observe_executable_permissions, ConfigInputSnapshot,
    DumpCleanupObservation, DumpTargetCapture, ExecutablePermissionObservation, FullDumpFailure,
    FullDumpPhase, HostPlatform, PlatformAttestation, PreparationFailure, PreparedPublication,
    RunnerProjectStore, SerializedDumpProjection, VerifiedFullDumpPublication,
};
use unica_workspace::paths::{
    metadata_is_link_or_reparse_point, normalize_leaf_path, normalize_path_identity,
};
const TARGET_PLATFORM_LINE: &str = ACTIVE_FORMAT_PROFILE.platform_line;
const TARGET_EXPORT_FORMAT: &str = ACTIVE_FORMAT_PROFILE.export_format;
const EFFECTIVE_CONFIG_NAME: &str = "v8project.yaml";
const LOCAL_CONFIG_NAME: &str = "v8project.local.yaml";
const BUILD_DUMP_TIMEOUT: Duration = Duration::from_secs(120);
const PLATFORM_PROBE_TIMEOUT: Duration = Duration::from_secs(15);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullDumpInvocation {
    BuildDump,
    RuntimeExecute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformUtility {
    Designer,
    Ibcmd,
}

impl PlatformUtility {
    pub fn executable_name(self) -> &'static str {
        match (current_host_platform(), self) {
            (HostPlatform::Windows, Self::Designer) => "1cv8.exe",
            (HostPlatform::Windows, Self::Ibcmd) => "ibcmd.exe",
            (_, Self::Designer) => "1cv8",
            (_, Self::Ibcmd) => "ibcmd",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPlatform {
    pub executable: PathBuf,
    pub exact_version: String,
    pub attestation: PlatformAttestation,
}

pub trait PlatformResolver {
    fn resolve(
        &self,
        effective_config: &YamlValue,
        config_dir: &Path,
        utility: PlatformUtility,
        runner: &dyn ProcessRunner,
        cancellation: &CancellationToken,
    ) -> Result<VerifiedPlatform, String>;
}

pub struct SystemPlatformResolver;
static SYSTEM_PLATFORM_RESOLVER: SystemPlatformResolver = SystemPlatformResolver;

impl PlatformResolver for SystemPlatformResolver {
    fn resolve(
        &self,
        effective_config: &YamlValue,
        config_dir: &Path,
        utility: PlatformUtility,
        runner: &dyn ProcessRunner,
        cancellation: &CancellationToken,
    ) -> Result<VerifiedPlatform, String> {
        #[cfg(any(test, feature = "test-support"))]
        if let Some((executable, exact_version)) =
            TEST_PLATFORM_OVERRIDE.with(|slot| slot.borrow().clone())
        {
            return Ok(VerifiedPlatform {
                attestation: PlatformAttestation::capture_test_fixture(&executable)?,
                executable,
                exact_version,
            });
        }
        if let Some(configured_version) =
            nested_yaml_string(effective_config, &["tools", "platform", "version"])?
        {
            validate_configured_platform_line(configured_version)?;
        }

        let configured_hint = nested_yaml_string(effective_config, &["tools", "platform", "path"])?
            .map(|path| absolutize(Path::new(path), config_dir));
        let candidates = if let Some(hint) = configured_hint {
            platform_candidates_from_hint(&hint, utility).map_err(|error| {
                format!(
                    "configured tools.platform.path cannot prove platform {TARGET_PLATFORM_LINE}: {error}"
                )
            })?
        } else {
            default_platform_candidates(utility)
        };

        let mut verified = Vec::new();
        let mut last_rejection = None;
        for candidate in candidates {
            match verify_platform_candidate(&candidate, utility, runner, cancellation) {
                Ok(candidate) => verified.push(candidate),
                Err(error) => last_rejection = Some(error),
            }
        }
        verified.sort_by_key(|candidate| {
            parse_exact_platform_version(&candidate.exact_version)
                .expect("verified platform always carries a four-part version")
        });
        verified.pop().ok_or_else(|| {
            format!(
                "could not resolve an immutable trusted executable for platform {TARGET_PLATFORM_LINE} ({}) with an exact four-part version in its installation path{}",
                utility.executable_name(),
                last_rejection
                    .map(|error| format!("; last rejected candidate: {error}"))
                    .unwrap_or_default()
            )
        })
    }
}

impl SystemPlatformResolver {
    pub fn attest_selected_local(
        executable: &Path,
        runner: &dyn ProcessRunner,
        cancellation: &CancellationToken,
        probe_cancelled: &mut bool,
    ) -> Result<VerifiedPlatform, String> {
        if !executable.is_absolute() {
            return Err("selected local platform executable must be absolute".into());
        }
        #[cfg(any(test, feature = "test-support"))]
        if let Some((fixture, exact_version)) =
            TEST_PLATFORM_OVERRIDE.with(|slot| slot.borrow().clone())
        {
            if normalize_leaf_path(executable)? != normalize_leaf_path(&fixture)? {
                return Err(
                    "selected local executable differs from the test platform receipt".into(),
                );
            }
            let version = parse_exact_platform_version(&exact_version)
                .ok_or("selected local platform version is not exact")?;
            if version[..3] != [8, 3, 27] {
                return Err("selected local platform must be 8.3.27.x".into());
            }
            return Ok(VerifiedPlatform {
                attestation: PlatformAttestation::capture_test_fixture(executable)?,
                executable: executable.to_path_buf(),
                exact_version,
            });
        }
        let utility = [PlatformUtility::Designer, PlatformUtility::Ibcmd]
            .into_iter()
            .find(|utility| executable.file_name() == Some(OsStr::new(utility.executable_name())))
            .ok_or("selected local provider has an unsupported executable name")?;
        verify_platform_candidate_recording_cancellation(
            executable,
            utility,
            runner,
            cancellation,
            Some(probe_cancelled),
        )
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! { static TEST_PLATFORM_OVERRIDE: RefCell<Option<(PathBuf,String)>> = const { RefCell::new(None) }; }
#[cfg(any(test, feature = "test-support"))]
pub fn with_test_platform<T>(executable: &Path, version: &str, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<(PathBuf, String)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_PLATFORM_OVERRIDE.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let old = TEST_PLATFORM_OVERRIDE.with(|slot| {
        slot.borrow_mut()
            .replace((executable.to_path_buf(), version.to_string()))
    });
    let _restore = Restore(old);
    run()
}

pub struct VerifiedFullDumpAdapter<'a> {
    runner: &'a dyn ProcessRunner,
    platform_resolver: &'a dyn PlatformResolver,
    bundled_program_override: Option<PathBuf>,
    environment: &'a dyn RuntimeEnvironment,
    semantic_validator: Arc<dyn DumpSemanticValidator>,
}

impl<'a> VerifiedFullDumpAdapter<'a> {
    pub fn new(
        environment: &'a dyn RuntimeEnvironment,
        semantic_validator: Arc<dyn DumpSemanticValidator>,
    ) -> Self {
        Self {
            runner: system_process_runner(),
            platform_resolver: &SYSTEM_PLATFORM_RESOLVER,
            bundled_program_override: None,
            environment,
            semantic_validator,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_dependencies(
        runner: &'a dyn ProcessRunner,
        platform_resolver: &'a dyn PlatformResolver,
        bundled_program: PathBuf,
        environment: &'a dyn RuntimeEnvironment,
        semantic_validator: Arc<dyn DumpSemanticValidator>,
    ) -> Self {
        Self {
            runner,
            platform_resolver,
            bundled_program_override: Some(bundled_program),
            environment,
            semantic_validator,
        }
    }

    pub fn invoke(
        &self,
        tool_name: &str,
        invocation: FullDumpInvocation,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<AdapterOutcome, String> {
        if cancellation.is_cancelled() {
            return Ok(AdapterOutcome::cancelled(format!(
                "{tool_name} cancelled before verified dump preparation"
            )));
        }

        let mut prepared = match prepare_legacy(
            args,
            context,
            self.platform_resolver,
            self.runner,
            cancellation,
            self.semantic_validator.clone(),
        ) {
            Ok(prepared) => prepared,
            Err(mut failure) => {
                let outcome =
                    dump_failure(tool_name, failure.detail().to_string(), None, None, None);
                return Ok(match failure.cleanup_observation() {
                    Some(observation) => finalize_private_outcome(observation, outcome),
                    None => outcome,
                });
            }
        };
        macro_rules! finish_prepared {
            ($outcome:expr) => {
                return Ok(finalize_private_outcome(
                    prepared.cleanup_observation(),
                    $outcome,
                ))
            };
        }
        let bundled = if let Some(program) = &self.bundled_program_override {
            (program.clone(), Vec::new())
        } else {
            let Some(plugin_root) = self.environment.plugin_root(&context.cwd) else {
                finish_prepared!(dump_failure(
                    tool_name,
                    "could not locate Unica plugin root for internal adapter lookup".to_string(),
                    None,
                    None,
                    None,
                ));
            };
            let bundled = match self.environment.resolve_v8_tool(&plugin_root, true) {
                Ok(bundled) => bundled,
                Err(error) => {
                    finish_prepared!(dump_failure(tool_name, error, None, None, None));
                }
            };
            (bundled.program, bundled.warnings)
        };

        let execution_args = dump_process_args(
            invocation,
            prepared.effective_config(),
            prepared.explicit_source_set(),
            prepared.extension(),
        );
        let report_args = reported_dump_process_args(
            invocation,
            prepared.explicit_source_set(),
            prepared.extension(),
        );
        let mut reported_command = vec![bundled.0.display().to_string()];
        reported_command.extend(report_args);
        if let Err(error) = prepared.recheck_selected_platform() {
            finish_prepared!(dump_failure(
                tool_name,
                format!("platform trust check immediately before v8-runner failed: {error}"),
                None,
                None,
                Some(reported_command),
            ));
        }
        prepared.begin_execution();
        let output_result = self.runner.run_staged(&ProcessCommand {
            program: bundled.0,
            args: execution_args,
            cwd: context.cwd.clone(),
            env: Vec::new(),
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: match invocation {
                FullDumpInvocation::BuildDump => Some(BUILD_DUMP_TIMEOUT),
                FullDumpInvocation::RuntimeExecute => None,
            },
            cancellation: cancellation.clone(),
        });
        if matches!(&output_result, Err(ProcessStreamFailure::Spawn(_)))
            || output_result
                .as_ref()
                .is_ok_and(|output| output.status_success && !output.timed_out && !output.cancelled)
        {
            prepared.confirm_execution_finished();
        }
        let staged_capture = if output_result
            .as_ref()
            .is_ok_and(|output| output.status_success && !output.timed_out && !output.cancelled)
        {
            prepared.capture_finished_staged_tree().map(Some)
        } else {
            Ok(None)
        };
        let output_result = output_result.map_err(|failure| failure.message().to_string());
        let platform_recheck = prepared.recheck_selected_platform();
        let config_cleanup = prepared.remove_effective_config();
        let (output, staged_snapshot) = match (
            output_result,
            staged_capture,
            platform_recheck,
            config_cleanup,
        ) {
            (Ok(output), Ok(snapshot), Ok(()), Ok(())) => (output, snapshot),
            (output_result, staged_capture, platform_recheck, config_cleanup) => {
                let mut errors = Vec::new();
                if let Err(error) = output_result {
                    errors.push(redactor(&error));
                }
                if let Err(error) = staged_capture {
                    errors.push(redactor(&error));
                }
                if let Err(error) = platform_recheck {
                    errors.push(format!(
                        "platform trust check immediately after v8-runner failed: {error}"
                    ));
                }
                if let Err(error) = config_cleanup {
                    errors.push(error);
                }
                finish_prepared!(dump_failure(
                    tool_name,
                    errors.join("; "),
                    None,
                    None,
                    Some(reported_command),
                ));
            }
        };
        if output.cancelled || cancellation.is_cancelled() {
            finish_prepared!(cancelled_dump_outcome(tool_name, &output, reported_command));
        }
        if !output.status_success {
            let error = if output.stderr.trim().is_empty() {
                format!(
                    "internal v8-runner verified dump exited with status {}",
                    output.status
                )
            } else {
                redactor(output.stderr.trim())
            };
            finish_prepared!(dump_failure(
                tool_name,
                error,
                Some(redactor(&output.stdout)),
                Some(redactor(&output.stderr)),
                Some(reported_command),
            ));
        }

        let Some(staged_snapshot) = staged_snapshot else {
            finish_prepared!(dump_failure(
                tool_name,
                "v8-runner did not confirm successful completion of the staged dump".to_string(),
                Some(redactor(&output.stdout)),
                Some(redactor(&output.stderr)),
                Some(reported_command),
            ));
        };
        if let Err(error) = prepared.validate_captured(&staged_snapshot) {
            finish_prepared!(dump_failure(
                tool_name,
                error,
                Some(redactor(&output.stdout)),
                Some(redactor(&output.stderr)),
                Some(reported_command),
            ));
        }
        if cancellation.is_cancelled() {
            finish_prepared!(cancelled_dump_outcome(tool_name, &output, reported_command));
        }

        let target = prepared.target().to_path_buf();
        let publication = prepared.publish_legacy(&staged_snapshot, cancellation);
        let mut private = prepared;
        macro_rules! finish_private {
            ($outcome:expr) => {
                return Ok(finalize_private_outcome(
                    private.cleanup_observation(),
                    $outcome,
                ))
            };
        }
        let cleanup_warnings = match publication {
            Ok(warnings) => warnings,
            Err(error) => {
                finish_private!(dump_failure(
                    tool_name,
                    error,
                    Some(redactor(&output.stdout)),
                    Some(redactor(&output.stderr)),
                    Some(reported_command),
                ));
            }
        };

        let mut warnings = bundled.1;
        warnings.extend(cleanup_warnings);
        Ok(finalize_private_outcome(
            private.cleanup_observation(),
            AdapterOutcome {
            ok: true,
            summary: format!(
                "{tool_name} published a validated platform {TARGET_PLATFORM_LINE} / export format {TARGET_EXPORT_FORMAT} full dump"
            ),
            changes: vec![format!(
                "replaced {} with validated full dump",
                target.display()
            )],
            warnings,
            errors: Vec::new(),
            artifacts: vec![target.display().to_string()],
            stdout: Some(redactor(&output.stdout)),
            stderr: Some(redactor(&output.stderr)),
            command: Some(reported_command),
            },
        ))
    }
}

pub fn prepare_legacy(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    platform_resolver: &dyn PlatformResolver,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
    semantic_validator: Arc<dyn DumpSemanticValidator>,
) -> Result<PreparedPublication, PreparationFailure> {
    validate_applied_full_dump_args(args)?;
    let config_path = resolve_config_path(args, context)?;
    prepare_selected(
        config_path,
        args.get("sourceSet").and_then(Value::as_str),
        args.get("extension").and_then(Value::as_str),
        args.get("workdir").and_then(Value::as_str),
        None,
        None,
        context,
        platform_resolver,
        runner,
        cancellation,
        |mut primary, local, config_dir| {
            if let Some(local) = local {
                validate_local_overlay_keys(&local, &config_dir.join(LOCAL_CONFIG_NAME))?;
                merge_yaml_values(&mut primary, local);
            }
            Ok(primary)
        },
        |effective, _| Ok((effective, None)),
        None,
        semantic_validator,
    )
}

#[allow(clippy::too_many_arguments)]
fn prepare_selected(
    config_path: PathBuf,
    source_set: Option<&str>,
    extension: Option<&str>,
    workdir: Option<&str>,
    expected_target: Option<&Path>,
    config_store: Option<RunnerProjectStore>,
    context: &WorkspaceContext,
    platform_resolver: &dyn PlatformResolver,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
    merge_inputs: impl FnOnce(YamlValue, Option<YamlValue>, &Path) -> Result<YamlValue, String>,
    project: impl FnOnce(YamlValue, &Path) -> Result<(YamlValue, Option<YamlValue>), String>,
    mut preparation_phase: Option<&mut FullDumpPhase>,
    semantic_validator: Arc<dyn DumpSemanticValidator>,
) -> Result<PreparedPublication, PreparationFailure> {
    let config_dir = config_path.parent().unwrap_or(&context.cwd);
    let primary = ConfigInputSnapshot::required(config_path.clone())?;
    let primary_raw = primary
        .raw()
        .expect("required config snapshot always contains bytes");
    let primary_config = parse_yaml_mapping(primary_raw, &config_path)?;

    let local_path = config_dir.join(LOCAL_CONFIG_NAME);
    let local = ConfigInputSnapshot::optional(local_path.clone())?;
    let overlay = local
        .raw()
        .map(|raw| parse_yaml_mapping(raw, &local_path))
        .transpose()?;
    let mut effective = merge_inputs(primary_config, overlay, config_dir)?;
    if cancellation.is_cancelled() {
        if let Some(phase) = preparation_phase.as_deref_mut() {
            *phase = FullDumpPhase::Cancelled;
        }
        return Err("verified dump cancelled while preparing effective config"
            .to_string()
            .into());
    }

    let format = yaml_string(&effective, "format")?.unwrap_or("DESIGNER");
    if format != "DESIGNER" {
        return Err(format!(
                "applied full dump supports only format=DESIGNER source sets; configured format is {format}"
            ).into());
    }
    let builder = nested_yaml_string(&effective, &["providers", "pull"])?
        .or(yaml_string(&effective, "builder")?)
        .unwrap_or("DESIGNER");
    let utility = match builder {
        "DESIGNER" | "designer" => PlatformUtility::Designer,
        "IBCMD" | "ibcmd" => PlatformUtility::Ibcmd,
        _ if config_store.is_some() => PlatformUtility::Designer,
        other => {
            return Err(format!(
                    "applied full dump supports only builder=DESIGNER or IBCMD; configured builder is {other}"
                ).into());
        }
    };

    if yaml_mapping(&effective)?.contains_key(yaml_key("basePath")) {
        return Err(
                "v8-runner rejects the removed top-level `basePath` key; paths are resolved relative to the primary config directory"
                    .to_string().into(),
            );
    }
    let base_path = normalize_path_identity(config_dir)?;
    let selection = select_source_set(&effective, source_set, extension).inspect_err(|_| {
        if let Some(phase) = preparation_phase {
            *phase = FullDumpPhase::Selection;
        }
    })?;
    if config_store.is_none() {
        validate_selected_source_path(&selection.path)?;
    }
    normalize_source_set_paths(&mut effective, &base_path)?;
    normalize_relocated_config_paths(&mut effective, config_dir)?;

    let configured_work_path = match yaml_string(&effective, "workPath")? {
        Some(path) => path,
        None if config_store.is_some() => ".build",
        None => return Err("v8project.yaml field `workPath` must be a string".into()),
    };
    let work_path = workdir
        .map(|path| absolutize(Path::new(path), config_dir))
        .unwrap_or_else(|| absolutize(Path::new(configured_work_path), config_dir));
    let work_path = normalize_path_identity(&work_path)?;
    set_yaml_string(&mut effective, "workPath", &work_path.display().to_string())?;
    normalize_infobase_connection(&mut effective, config_dir)?;

    let canonical = config_store.is_some();
    let capture = DumpTargetCapture::capture(
        context,
        base_path.join(&selection.path),
        &base_path,
        &work_path,
        expected_target,
        selection.kind,
        selection.name,
        selection.explicit_source_set,
        selection.extension,
        vec![primary, local],
        config_store,
        semantic_validator,
    )?;
    let platform = if !canonical {
        let platform =
            platform_resolver.resolve(&effective, config_dir, utility, runner, cancellation)?;
        let version = parse_exact_platform_version(&platform.exact_version)
            .ok_or("resolved platform version is not an exact four-part version")?;
        if version[..3] != [8, 3, 27] {
            return Err(format!(
                "resolved platform {} is not supported; required {TARGET_PLATFORM_LINE}.x",
                platform.exact_version
            )
            .into());
        }
        Some(platform)
    } else {
        None
    };
    let private = capture.create_private(platform.as_ref().map(|p| p.attestation.clone()))?;
    let setup = (|| {
        if let Some(platform) = &platform {
            redirect_selected_source_set(&mut effective, selection.index, private.staged_target())?;
            set_nested_yaml_string(
                &mut effective,
                &["tools", "platform", "path"],
                &platform.executable.display().to_string(),
            )?;
            set_nested_yaml_string(
                &mut effective,
                &["tools", "platform", "version"],
                &platform.exact_version,
            )?;
        }
        let (primary, local) = project(effective, config_dir)?;
        serialize_projection(primary, local, canonical)
    })();
    match setup {
        Ok(projection) => private.configure(projection),
        Err(detail) => Err(private.refuse_setup(detail)),
    }
}
pub struct FullDumpSelection<'a> {
    pub config_path: &'a Path,
    pub source_set: Option<&'a str>,
    pub extension: Option<&'a str>,
    pub expected_target: Option<&'a Path>,
}

#[allow(clippy::too_many_arguments)]
pub fn prepare_publication(
    context: &WorkspaceContext,
    selection: FullDumpSelection<'_>,
    store: RunnerProjectStore,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
    merge_inputs: impl FnOnce(YamlValue, Option<YamlValue>, &Path) -> Result<YamlValue, String>,
    project: impl FnOnce(YamlValue, &Path) -> Result<(YamlValue, Option<YamlValue>), String>,
    semantic_validator: Arc<dyn DumpSemanticValidator>,
) -> Result<VerifiedFullDumpPublication, FullDumpFailure> {
    if cancellation.is_cancelled() {
        return Err(FullDumpFailure::new(
            FullDumpPhase::Cancelled,
            "full export cancelled before private preparation".into(),
        ));
    }
    let mut preparation_phase = FullDumpPhase::Preparation;
    let prepared = prepare_selected(
        selection.config_path.to_path_buf(),
        selection.source_set,
        selection.extension,
        None,
        selection.expected_target,
        Some(store),
        context,
        &SYSTEM_PLATFORM_RESOLVER,
        runner,
        cancellation,
        merge_inputs,
        project,
        Some(&mut preparation_phase),
        semantic_validator,
    )
    .map_err(|error| error.into_canonical_failure(preparation_phase))?;
    Ok(VerifiedFullDumpPublication::from_prepared(prepared))
}

pub fn attest_selected_local(
    publication: &mut VerifiedFullDumpPublication,
    executable: &Path,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
) -> Result<(), FullDumpFailure> {
    if cancellation.is_cancelled() {
        return Err(FullDumpFailure::new(
            FullDumpPhase::Cancelled,
            "full export cancelled before selected local provider attestation".into(),
        ));
    }
    let mut probe_cancelled = false;
    let verified = SystemPlatformResolver::attest_selected_local(
        executable,
        runner,
        cancellation,
        &mut probe_cancelled,
    )
    .map_err(|error| {
        FullDumpFailure::new(
            if probe_cancelled {
                FullDumpPhase::Cancelled
            } else {
                FullDumpPhase::Integrity
            },
            error,
        )
    })?;
    publication.bind_selected_attestation(executable, verified.attestation, cancellation)
}

struct SourceSelection {
    index: usize,
    name: String,
    kind: SourceSetKind,
    path: PathBuf,
    explicit_source_set: Option<String>,
    extension: Option<String>,
}

fn select_source_set(
    effective: &YamlValue,
    source_set: Option<&str>,
    extension: Option<&str>,
) -> Result<SourceSelection, String> {
    let entries = yaml_mapping(effective)?
        .get(yaml_key("source-set"))
        .and_then(YamlValue::as_sequence)
        .ok_or_else(|| "v8project.yaml field `source-set` must be a list".to_string())?;
    let source_set = source_set.filter(|value| !value.trim().is_empty());
    let extension = extension.filter(|value| !value.trim().is_empty());

    let requested_name = match (source_set, extension) {
        (Some(source_set), Some(extension)) if source_set != extension => {
            return Err(format!(
                "sourceSet `{source_set}` does not match extension `{extension}`"
            ));
        }
        (Some(source_set), _) => Some(source_set),
        (None, Some(extension)) => Some(extension),
        (None, None) => None,
    };

    let parsed = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let name = yaml_string(entry, "name")?
                .ok_or_else(|| format!("source-set entry {index} is missing string `name`"))?;
            let source_type = yaml_string(entry, "type")?
                .ok_or_else(|| format!("source-set `{name}` is missing string `type`"))?;
            let kind = match source_type {
                "CONFIGURATION" => SourceSetKind::Configuration,
                "EXTENSION" => SourceSetKind::Extension,
                "EXTERNAL_DATA_PROCESSORS" => SourceSetKind::ExternalProcessor,
                "EXTERNAL_REPORTS" => SourceSetKind::ExternalReport,
                other => {
                    return Err(format!(
                        "source-set `{name}` has unsupported type `{other}`"
                    ));
                }
            };
            let path = yaml_string(entry, "path")?
                .ok_or_else(|| format!("source-set `{name}` is missing string `path`"))?;
            Ok((index, name.to_string(), kind, PathBuf::from(path)))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let selected = if let Some(requested_name) = requested_name {
        parsed
            .iter()
            .find(|(_, name, _, _)| name == requested_name)
            .ok_or_else(|| format!("unknown source-set `{requested_name}`"))?
    } else {
        let configuration = parsed
            .iter()
            .filter(|(_, _, kind, _)| *kind == SourceSetKind::Configuration)
            .collect::<Vec<_>>();
        if configuration.len() != 1 {
            return Err(format!(
                "full dump requires exactly one CONFIGURATION source-set when sourceSet is omitted; found {}",
                configuration.len()
            ));
        }
        configuration[0]
    };

    match (extension, selected.2) {
        (Some(_), SourceSetKind::Extension) => {}
        (Some(extension), _) => {
            return Err(format!(
                "extension `{extension}` does not select an EXTENSION source-set"
            ));
        }
        (None, SourceSetKind::Configuration) => {}
        (None, SourceSetKind::Extension) => {
            return Err(format!(
                "source-set `{}` is an extension and requires `extension`",
                selected.1
            ));
        }
        (None, SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport) => {
            return Err(format!(
                "source-set `{}` is external; applied full dump supports only CONFIGURATION and EXTENSION",
                selected.1
            ));
        }
    }

    Ok(SourceSelection {
        index: selected.0,
        name: selected.1.clone(),
        kind: selected.2,
        path: selected.3.clone(),
        explicit_source_set: source_set.map(str::to_string),
        extension: extension.map(str::to_string),
    })
}

fn validate_applied_full_dump_args(args: &Map<String, Value>) -> Result<(), String> {
    const ALLOWED: &[&str] = &[
        "cwd",
        "dryRun",
        "confirm",
        "operation",
        "config",
        "workdir",
        "mode",
        "sourceSet",
        "extension",
    ];
    if let Some(unsupported) = args.keys().find(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(format!(
            "verified applied full dump does not accept `{unsupported}`"
        ));
    }
    if args.get("mode").and_then(Value::as_str) != Some("full") {
        return Err("verified dump adapter requires explicit mode=full".to_string());
    }
    if args.get("dryRun").and_then(Value::as_bool) != Some(false) {
        return Err("verified dump adapter is only for applied execution".to_string());
    }
    Ok(())
}

fn resolve_config_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let requested = args
        .get("config")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(EFFECTIVE_CONFIG_NAME));
    if requested.file_name().and_then(|name| name.to_str()) == Some(LOCAL_CONFIG_NAME) {
        return Err(format!(
            "{LOCAL_CONFIG_NAME} is a local overlay and cannot be used as the primary config"
        ));
    }
    normalize_leaf_path(&if requested.is_absolute() {
        requested
    } else {
        context.cwd.join(requested)
    })
}

fn validate_selected_source_path(path: &Path) -> Result<(), String> {
    use std::path::Component;

    if path.is_absolute() {
        return Err(format!(
            "selected source-set target must be relative to the primary config directory: {}",
            path.display()
        ));
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(format!(
            "selected source-set target must be a contained relative path without `..`: {}",
            path.display()
        ));
    }
    Ok(())
}

fn redirect_selected_source_set(
    root: &mut YamlValue,
    selected_index: usize,
    stage: &Path,
) -> Result<(), String> {
    let entries = yaml_mapping_mut(root)?
        .get_mut(yaml_key("source-set"))
        .and_then(YamlValue::as_sequence_mut)
        .ok_or_else(|| "v8project.yaml field `source-set` must be a list".to_string())?;
    let entry = entries
        .get_mut(selected_index)
        .ok_or_else(|| "selected source-set disappeared from effective config".to_string())?;
    set_yaml_string(entry, "path", &stage.display().to_string())
}

fn normalize_source_set_paths(root: &mut YamlValue, config_dir: &Path) -> Result<(), String> {
    let entries = yaml_mapping_mut(root)?
        .get_mut(yaml_key("source-set"))
        .and_then(YamlValue::as_sequence_mut)
        .ok_or_else(|| "v8project.yaml field `source-set` must be a list".to_string())?;
    for (index, entry) in entries.iter_mut().enumerate() {
        let path = yaml_string(entry, "path")?
            .ok_or_else(|| format!("source-set entry {index} is missing string `path`"))?;
        let absolute = normalize_path_identity(&absolutize(Path::new(path), config_dir))?;
        set_yaml_string(entry, "path", &absolute.display().to_string())?;
    }
    Ok(())
}

fn normalize_relocated_config_paths(root: &mut YamlValue, config_dir: &Path) -> Result<(), String> {
    for path in [
        &["tools", "va", "epf_path"][..],
        &["tools", "client_mcp", "extension", "source", "path"][..],
        &["tools", "client_mcp", "extension", "artifact", "path"][..],
        &["tests", "va", "params_path"][..],
    ] {
        normalize_optional_nested_path(root, path, config_dir)?;
    }

    let Some(profiles) = yaml_value_at_path_mut(root, &["tests", "va", "profiles"])? else {
        return Ok(());
    };
    let profiles = profiles
        .as_mapping_mut()
        .ok_or_else(|| "YAML field `tests.va.profiles` must be a mapping".to_string())?;
    for (name, profile) in profiles {
        let name = name
            .as_str()
            .ok_or_else(|| "tests.va.profiles contains a non-string profile name".to_string())?;
        let Some(path) = yaml_string(profile, "feature_path")?.map(str::to_string) else {
            continue;
        };
        let absolute = absolutize(Path::new(&path), config_dir);
        set_yaml_string(profile, "feature_path", &absolute.display().to_string())
            .map_err(|error| format!("invalid tests.va.profiles.{name}: {error}"))?;
    }
    Ok(())
}

fn normalize_optional_nested_path(
    root: &mut YamlValue,
    path: &[&str],
    config_dir: &Path,
) -> Result<(), String> {
    let Some(value) = nested_yaml_string(root, path)?.map(str::to_string) else {
        return Ok(());
    };
    let absolute = absolutize(Path::new(&value), config_dir);
    set_nested_yaml_string(root, path, &absolute.display().to_string())
}

fn yaml_value_at_path_mut<'a>(
    value: &'a mut YamlValue,
    path: &[&str],
) -> Result<Option<&'a mut YamlValue>, String> {
    let mut current = value;
    for (index, key) in path.iter().enumerate() {
        let Some(mapping) = current.as_mapping_mut() else {
            return Err(format!(
                "YAML field `{}` must be a mapping",
                path[..index].join(".")
            ));
        };
        let Some(next) = mapping.get_mut(yaml_key(key)) else {
            return Ok(None);
        };
        if matches!(next, YamlValue::Null) {
            return Ok(None);
        }
        current = next;
    }
    Ok(Some(current))
}

fn parse_yaml_mapping(raw: &[u8], path: &Path) -> Result<YamlValue, String> {
    let text = std::str::from_utf8(raw)
        .map_err(|error| format!("failed to read {} as UTF-8: {error}", path.display()))?;
    let value = serde_yaml::from_str::<YamlValue>(text)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    yaml_mapping(&value)?;
    Ok(value)
}

fn yaml_mapping(value: &YamlValue) -> Result<&YamlMapping, String> {
    value
        .as_mapping()
        .ok_or_else(|| "expected a YAML mapping at the document root".to_string())
}

fn yaml_mapping_mut(value: &mut YamlValue) -> Result<&mut YamlMapping, String> {
    value
        .as_mapping_mut()
        .ok_or_else(|| "expected a YAML mapping at the document root".to_string())
}

fn yaml_key(key: &str) -> YamlValue {
    YamlValue::String(key.to_string())
}

fn yaml_string<'a>(value: &'a YamlValue, key: &str) -> Result<Option<&'a str>, String> {
    match yaml_mapping(value)?.get(yaml_key(key)) {
        None | Some(YamlValue::Null) => Ok(None),
        Some(YamlValue::String(value)) => Ok(Some(value)),
        Some(_) => Err(format!("YAML field `{key}` must be a string")),
    }
}

fn nested_yaml_string<'a>(value: &'a YamlValue, path: &[&str]) -> Result<Option<&'a str>, String> {
    let mut current = value;
    for (index, key) in path.iter().enumerate() {
        let Some(next) = current.as_mapping().and_then(|map| map.get(yaml_key(key))) else {
            return Ok(None);
        };
        if index + 1 == path.len() {
            return match next {
                YamlValue::String(value) => Ok(Some(value)),
                YamlValue::Null => Ok(None),
                _ => Err(format!("YAML field `{}` must be a string", path.join("."))),
            };
        }
        current = next;
    }
    Ok(None)
}

fn set_yaml_string(value: &mut YamlValue, key: &str, new_value: &str) -> Result<(), String> {
    yaml_mapping_mut(value)?.insert(yaml_key(key), YamlValue::String(new_value.to_string()));
    Ok(())
}

fn set_nested_yaml_string(
    value: &mut YamlValue,
    path: &[&str],
    new_value: &str,
) -> Result<(), String> {
    if path.is_empty() {
        return Err("cannot assign an empty YAML path".to_string());
    }
    let mut current = value;
    for key in &path[..path.len() - 1] {
        let mapping = yaml_mapping_mut(current)?;
        current = mapping
            .entry(yaml_key(key))
            .or_insert_with(|| YamlValue::Mapping(YamlMapping::new()));
        if !current.is_mapping() {
            return Err(format!(
                "YAML field `{}` must be a mapping",
                path[..path.len() - 1].join(".")
            ));
        }
    }
    set_yaml_string(current, path[path.len() - 1], new_value)
}

fn validate_local_overlay_keys(value: &YamlValue, path: &Path) -> Result<(), String> {
    for key in yaml_mapping(value)?.keys() {
        let Some(key) = key.as_str() else {
            return Err(format!(
                "local config overlay contains a non-string top-level key: {}",
                path.display()
            ));
        };
        match key {
            "source-set" | "format" | "builder" => {
                return Err(format!(
                    "local config overlay cannot override project identity key `{key}`: {}",
                    path.display()
                ));
            }
            "workPath" | "infobase" | "tools" | "tests" | "mcp" => {}
            other => {
                return Err(format!(
                    "local config overlay does not support top-level key `{other}`: {}",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

pub fn merge_yaml_values(base: &mut YamlValue, overlay: YamlValue) {
    match (base, overlay) {
        (YamlValue::Mapping(base), YamlValue::Mapping(overlay)) => {
            for (key, overlay_value) in overlay {
                match base.get_mut(&key) {
                    Some(base_value) => merge_yaml_values(base_value, overlay_value),
                    None => {
                        base.insert(key, overlay_value);
                    }
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

fn normalize_infobase_connection(root: &mut YamlValue, config_dir: &Path) -> Result<(), String> {
    let Some(connection) =
        nested_yaml_string(root, &["infobase", "connection"])?.map(str::to_string)
    else {
        return Ok(());
    };
    let trimmed = connection.trim();
    let normalized = if trimmed.starts_with('/') || trimmed.starts_with('-') {
        normalize_raw_connection_args(trimmed, config_dir)
    } else {
        normalize_key_value_connection(&connection, config_dir)?
    };
    set_nested_yaml_string(root, &["infobase", "connection"], &normalized)
}

fn normalize_key_value_connection(connection: &str, config_dir: &Path) -> Result<String, String> {
    split_key_value_connection(connection)?
        .into_iter()
        .map(|part| {
            let trimmed = part.trim();
            if !trimmed.to_ascii_lowercase().starts_with("file=") {
                return Ok(trimmed.to_string());
            }
            let normalized = normalize_quoted_connection_file_path(&trimmed[5..], config_dir)?;
            Ok(format!("{}{}", &trimmed[..5], normalized))
        })
        .collect::<Result<Vec<_>, String>>()
        .map(|parts| parts.join(";"))
}

fn split_key_value_connection(connection: &str) -> Result<Vec<&str>, String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut characters = connection.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        if let Some(active_quote) = quote {
            if character == active_quote {
                if characters
                    .peek()
                    .is_some_and(|(_, next)| *next == active_quote)
                {
                    characters.next();
                } else {
                    quote = None;
                }
            }
            continue;
        }
        if character == '"' {
            quote = Some(character);
        } else if character == ';' {
            parts.push(&connection[start..index]);
            start = index + character.len_utf8();
        }
    }
    if let Some(quote) = quote {
        return Err(format!(
            "infobase.connection contains an unterminated {quote} quote"
        ));
    }
    parts.push(&connection[start..]);
    Ok(parts)
}

fn normalize_quoted_connection_file_path(path: &str, config_dir: &Path) -> Result<String, String> {
    let trimmed = path.trim();
    let quote = match (trimmed.chars().next(), trimmed.chars().last()) {
        (Some('"'), Some('"')) => Some('"'),
        (Some('"'), _) => {
            return Err("infobase.connection File value has mismatched quotes".to_string());
        }
        (_, Some('"')) => {
            return Err("infobase.connection File value has mismatched quotes".to_string());
        }
        _ => None,
    };
    let unquoted = quote
        .map(|_| &trimmed[1..trimmed.len() - 1])
        .unwrap_or(trimmed);
    let normalized = normalize_connection_file_path(unquoted, config_dir);
    if let Some(quote) = quote {
        if normalized.contains(quote) {
            return Err(format!(
                "normalized infobase.connection File path contains its {quote} delimiter"
            ));
        }
        return Ok(format!("{quote}{normalized}{quote}"));
    }
    if normalized.contains(';') || normalized.chars().any(char::is_whitespace) {
        return Ok(format!("\"{}\"", normalized.replace('"', "\"\"")));
    }
    Ok(normalized)
}

fn normalize_raw_connection_args(connection: &str, config_dir: &Path) -> String {
    let mut args = split_arg_string(connection);
    let mut index = 0;
    while index + 1 < args.len() {
        if args[index].eq_ignore_ascii_case("/f") || args[index].eq_ignore_ascii_case("-f") {
            args[index + 1] = normalize_connection_file_path(&args[index + 1], config_dir);
            index += 2;
        } else {
            index += 1;
        }
    }
    join_arg_string(&args)
}

fn normalize_connection_file_path(path: &str, config_dir: &Path) -> String {
    let path = strip_matching_quotes(path.trim()).unwrap_or(path.trim());
    absolutize(Path::new(path), config_dir)
        .display()
        .to_string()
}

fn strip_matching_quotes(value: &str) -> Option<&str> {
    if value.len() < 2 {
        return None;
    }
    let quote = value.as_bytes()[0];
    let last = *value.as_bytes().last()?;
    ((quote == b'\'' || quote == b'"') && quote == last).then_some(&value[1..value.len() - 1])
}

fn split_arg_string(raw: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in raw.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ch if ch.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

fn join_arg_string(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.is_empty() || arg.chars().any(char::is_whitespace) {
                format!("\"{}\"", arg.replace('"', "\\\""))
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn absolutize(path: &Path, base: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn validate_configured_platform_line(version: &str) -> Result<(), String> {
    let parts = version
        .split('.')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| format!("invalid tools.platform.version `{version}`"))?;
    if !(2..=4).contains(&parts.len())
        || parts[0] != 8
        || parts[1] != 3
        || parts.get(2).is_some_and(|patch| *patch != 27)
    {
        return Err(format!(
            "configured tools.platform.version `{version}` is incompatible with required {TARGET_PLATFORM_LINE}"
        ));
    }
    Ok(())
}

fn platform_candidates_from_hint(
    hint: &Path,
    utility: PlatformUtility,
) -> Result<Vec<PathBuf>, String> {
    let metadata = fs::symlink_metadata(hint)
        .map_err(|error| format!("failed to inspect {}: {error}", hint.display()))?;
    if metadata_is_link_or_reparse_point(&metadata) {
        return Err(format!(
            "platform hint must not be a symbolic link or reparse point: {}",
            hint.display()
        ));
    }
    if metadata.is_file() {
        let candidate = if hint.file_name() == Some(OsStr::new(utility.executable_name())) {
            hint.to_path_buf()
        } else {
            hint.parent()
                .unwrap_or_else(|| Path::new("."))
                .join(utility.executable_name())
        };
        return Ok(vec![candidate]);
    }
    if !metadata.is_dir() {
        return Err(format!(
            "platform hint is neither a file nor a directory: {}",
            hint.display()
        ));
    }
    let mut candidates = vec![
        hint.join(utility.executable_name()),
        hint.join("bin").join(utility.executable_name()),
    ];
    if let Ok(children) = fs::read_dir(hint) {
        for child in children.flatten() {
            let path = child.path();
            if !path.is_dir() {
                continue;
            }
            candidates.push(path.join(utility.executable_name()));
            candidates.push(path.join("bin").join(utility.executable_name()));
        }
    }
    Ok(candidates)
}

fn default_platform_candidates(utility: PlatformUtility) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for root in default_platform_roots() {
        if let Ok(mut from_root) = platform_candidates_from_hint(&root, utility) {
            candidates.append(&mut from_root);
        }
    }
    if let Some(paths) = env::var_os("PATH") {
        candidates
            .extend(env::split_paths(&paths).map(|path| path.join(utility.executable_name())));
    }
    candidates
}

/// The one enumeration of platform installation roots in the project — reused
/// by `documentation_registry`'s installation resolver in
/// `infrastructure::application_ports` so a second such list never appears.
pub fn default_platform_roots() -> Vec<PathBuf> {
    match current_host_platform() {
        HostPlatform::Windows => vec![
            PathBuf::from(r"C:\Program Files\1cv8"),
            PathBuf::from(r"C:\Program Files (x86)\1cv8"),
        ],
        HostPlatform::Linux => vec![
            PathBuf::from("/opt/1cv8/x86_64"),
            PathBuf::from("/opt/1cv8/i386"),
            PathBuf::from("/usr/local/1cv8"),
        ],
        HostPlatform::Other => vec![PathBuf::from("/opt/1cv8")],
    }
}

fn verify_platform_candidate(
    candidate: &Path,
    utility: PlatformUtility,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
) -> Result<VerifiedPlatform, String> {
    verify_platform_candidate_recording_cancellation(candidate, utility, runner, cancellation, None)
}

fn verify_platform_candidate_recording_cancellation(
    candidate: &Path,
    utility: PlatformUtility,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
    probe_cancelled: Option<&mut bool>,
) -> Result<VerifiedPlatform, String> {
    let metadata = fs::symlink_metadata(candidate)
        .map_err(|error| format!("failed to inspect {}: {error}", candidate.display()))?;
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err(format!(
            "platform utility is not a real regular file: {}",
            candidate.display()
        ));
    }
    if observe_executable_permissions(&metadata) == ExecutablePermissionObservation::NotExecutable {
        return Err(format!(
            "platform utility is not executable: {}",
            candidate.display()
        ));
    }
    if candidate.file_name() != Some(OsStr::new(utility.executable_name())) {
        return Err(format!(
            "platform utility has an unexpected filename: {}",
            candidate.display()
        ));
    }
    let canonical = fs::canonicalize(candidate)
        .map_err(|error| format!("failed to canonicalize {}: {error}", candidate.display()))?;
    let install_dir = canonical.parent().ok_or_else(|| {
        format!(
            "platform utility has no installation directory: {}",
            canonical.display()
        )
    })?;
    let probe_name = PlatformUtility::Ibcmd.executable_name();
    let probe = if utility == PlatformUtility::Ibcmd {
        canonical.clone()
    } else {
        install_dir.join(probe_name)
    };
    verify_regular_executable(&probe, "platform version probe")?;
    if probe.parent() != Some(install_dir) {
        return Err(format!(
            "platform version probe is not a sibling in the exact installation: {}",
            probe.display()
        ));
    }
    let attestation = PlatformAttestation::capture_immutable(&canonical).map_err(|error| {
        format!(
            "platform installation is not immutable and trusted before version probing: {error}"
        )
    })?;
    let output = runner.run(&ProcessCommand {
        program: probe.clone(),
        args: vec!["--version".to_string()],
        cwd: install_dir.to_path_buf(),
        env: Vec::new(),
        env_remove: Vec::new(),
        capture_limits: None,
        timeout: Some(PLATFORM_PROBE_TIMEOUT),
        cancellation: cancellation.clone(),
    })?;
    if output.cancelled || cancellation.is_cancelled() {
        if let Some(probe_cancelled) = probe_cancelled {
            *probe_cancelled = true;
        }
        return Err("platform version probe was cancelled".to_string());
    }
    if !output.status_success {
        return Err(format!(
            "platform version probe failed with status {}: {}",
            output.status,
            redactor(output.stderr.trim())
        ));
    }
    let exact_version = parse_platform_probe_version(&output.stdout, &output.stderr)?;
    let parts = parse_exact_platform_version(&exact_version)
        .expect("platform probe parser returns a four-part version");
    if parts[..3] != [8, 3, 27] {
        return Err(format!(
            "platform version probe for {} reported {exact_version}, required {TARGET_PLATFORM_LINE}.x",
            canonical.display()
        ));
    }
    attestation.recheck().map_err(|error| {
        format!("platform trust changed during version probing and was rejected: {error}")
    })?;
    Ok(VerifiedPlatform {
        executable: canonical,
        exact_version,
        attestation,
    })
}

fn verify_regular_executable(path: &Path, role: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect {role} {}: {error}", path.display()))?;
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err(format!(
            "{role} is not a real regular file: {}",
            path.display()
        ));
    }
    if observe_executable_permissions(&metadata) == ExecutablePermissionObservation::NotExecutable {
        return Err(format!("{role} is not executable: {}", path.display()));
    }
    Ok(())
}

fn parse_platform_probe_version(stdout: &str, stderr: &str) -> Result<String, String> {
    let mut versions = stdout
        .split(|character: char| !(character.is_ascii_digit() || character == '.'))
        .chain(stderr.split(|character: char| !(character.is_ascii_digit() || character == '.')))
        .filter_map(|token| parse_exact_platform_version(token).map(|_| token.to_string()))
        .collect::<Vec<_>>();
    versions.sort();
    versions.dedup();
    match versions.as_slice() {
        [version] => Ok(version.clone()),
        [] => Err(format!(
            "platform version probe did not report an exact four-part version: {}",
            redactor(stdout.trim())
        )),
        _ => Err(format!(
            "platform version probe reported ambiguous versions: {}",
            versions.join(", ")
        )),
    }
}

fn parse_exact_platform_version(value: &str) -> Option<[u32; 4]> {
    let parts = value
        .split('.')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    (parts.len() == 4).then(|| [parts[0], parts[1], parts[2], parts[3]])
}

fn dump_process_args(
    invocation: FullDumpInvocation,
    config: &Path,
    source_set: Option<&str>,
    extension: Option<&str>,
) -> Vec<String> {
    let mut args = Vec::new();
    if invocation == FullDumpInvocation::BuildDump {
        args.push("dump".to_string());
    }
    args.extend(["--config".to_string(), config.display().to_string()]);
    if invocation == FullDumpInvocation::RuntimeExecute {
        args.push("dump".to_string());
    }
    args.extend(["--mode".to_string(), "full".to_string()]);
    if let Some(source_set) = source_set {
        args.extend(["--source-set".to_string(), source_set.to_string()]);
    }
    if let Some(extension) = extension {
        args.extend(["--extension".to_string(), extension.to_string()]);
    }
    args
}

fn reported_dump_process_args(
    invocation: FullDumpInvocation,
    source_set: Option<&str>,
    extension: Option<&str>,
) -> Vec<String> {
    dump_process_args(
        invocation,
        Path::new("<private-effective-config>"),
        source_set,
        extension,
    )
}

pub fn finalize_private_outcome(
    private: DumpCleanupObservation,
    mut outcome: AdapterOutcome,
) -> AdapterOutcome {
    if let Some(error) = private.cleanup_error {
        if outcome.ok {
            outcome.ok = false;
            outcome.summary = format!(
                "{}; validated publication completed but private cleanup failed",
                outcome.summary
            );
        }
        outcome.errors.push(error.clone());
        let status = if private.effective_config_secret_present {
            "Secret-bearing private dump debris may remain"
        } else {
            "Private dump debris may remain"
        };
        match private.cleanup_error_location_proof {
            Ok(_) => {
                outcome
                    .warnings
                    .push(format!("{status} at {}", private.root.display()));
                outcome.artifacts.push(private.root.display().to_string());
            }
            Err(error) => outcome.warnings.push(format!(
                "{status}, but its current location is unconfirmed; original name: {} ({error})",
                private.root.display()
            )),
        }
        outcome.stderr = Some(match outcome.stderr.take() {
            Some(stderr) if !stderr.is_empty() => format!("{stderr}\n{error}\n"),
            _ => format!("{error}\n"),
        });
    }
    if private.publication_uncertain {
        outcome.ok = false;
        outcome.summary =
            "Full dump publication state is unknown; inspect the retained data before retrying"
                .into();
    }
    if private.retained {
        outcome.warnings.retain(|warning| {
            warning
                != "Git-visible sources were not published; the private staged dump was discarded"
        });
        let status = if private.effective_config_secret_present {
            "Private data retained; effective configuration cleanup is incomplete"
        } else {
            "Data retained for recovery"
        };
        let location = if private.entire_stage {
            &private.root
        } else {
            &private.recovery
        };
        let proof = private.retention_location_proof;
        match proof {
            Ok(()) => {
                outcome
                    .warnings
                    .push(format!("{status} at {}", location.display()));
                outcome.artifacts.push(location.display().to_string());
            }
            Err(error) => outcome.warnings.push(format!(
                "{status}, but its current location is unconfirmed; original name: {} ({error})",
                location.display()
            )),
        }
    }
    outcome
}

pub fn dump_failure(
    tool_name: &str,
    error: String,
    stdout: Option<String>,
    stderr: Option<String>,
    command: Option<Vec<String>>,
) -> AdapterOutcome {
    AdapterOutcome {
        ok: false,
        summary: format!("{tool_name} failed before verified dump publication"),
        changes: Vec::new(),
        warnings: vec![
            "Git-visible sources were not published; the private staged dump was discarded"
                .to_string(),
        ],
        errors: vec![error.clone()],
        artifacts: Vec::new(),
        stdout,
        stderr: Some(stderr.unwrap_or_else(|| format!("{error}\n"))),
        command,
    }
}

fn cancelled_dump_outcome(
    tool_name: &str,
    output: &ProcessOutput,
    command: Vec<String>,
) -> AdapterOutcome {
    AdapterOutcome {
        ok: false,
        summary: format!("{tool_name} cancelled before verified dump publication"),
        changes: Vec::new(),
        warnings: vec![
            "Git-visible sources were not published; the private staged dump was discarded"
                .to_string(),
        ],
        errors: vec!["verified full dump cancelled".to_string()],
        artifacts: Vec::new(),
        stdout: Some(redactor(&output.stdout)),
        stderr: Some(redactor(&output.stderr)),
        command: Some(command),
    }
}

fn serialize_projection(
    primary: YamlValue,
    local: Option<YamlValue>,
    canonical: bool,
) -> Result<SerializedDumpProjection, String> {
    if canonical {
        yaml_mapping(&primary)?;
        if let Some(local) = &local {
            yaml_mapping(local)?;
        }
    }
    let local = local
        .as_ref()
        .map(serde_yaml::to_string)
        .transpose()
        .map_err(|e| e.to_string())?
        .map(String::into_bytes);
    let primary = serde_yaml::to_string(&primary)
        .map_err(|e| e.to_string())?
        .into_bytes();
    let guard = serde_yaml::to_string(&YamlValue::Null)
        .map_err(|e| e.to_string())?
        .into_bytes();
    Ok(SerializedDumpProjection::from_serialized(
        primary, local, guard,
    ))
}

pub fn write_projection(
    store: &mut RunnerProjectStore,
    primary: &YamlValue,
    local: Option<&YamlValue>,
) -> Result<(), String> {
    store.write_serialized_projection(&serialize_projection(
        primary.clone(),
        local.cloned(),
        true,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn key_value_connection_preserves_quoted_file_path_and_semicolons() {
        let config_dir = Path::new("/workspace/project");
        let connection = r#"File="build/ib;archive";Usr=O'Brien;Pwd="secret;with:semicolon""#;

        let normalized = normalize_key_value_connection(connection, config_dir).unwrap();

        let expected = match current_host_platform() {
            HostPlatform::Windows => {
                r#"File="/workspace/project\build/ib;archive";Usr=O'Brien;Pwd="secret;with:semicolon""#
            }
            _ => {
                r#"File="/workspace/project/build/ib;archive";Usr=O'Brien;Pwd="secret;with:semicolon""#
            }
        };
        assert_eq!(normalized, expected);
    }
}
