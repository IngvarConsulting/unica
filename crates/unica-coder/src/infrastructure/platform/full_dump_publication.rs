//! Guarded publication for synchronous full configuration dumps.
//!
//! The pinned v8-runner already stages a full dump before writing its configured
//! source path. This adapter adds an outer, Unica-owned stage. It gives
//! v8-runner an effective private config whose selected source-set points at the
//! outer stage, verifies the resolved 8.3.27 platform and produced 2.20 XML,
//! then publishes the whole tree while holding Unica's exclusive publication
//! gate. No Git-visible source path is passed to the child process.

#![cfg_attr(windows, allow(dead_code))]

use crate::application::AdapterOutcome;
use crate::domain::cancellation::CancellationToken;
use crate::domain::format_profile::ACTIVE_FORMAT_PROFILE;
use crate::domain::project_sources::{SourceFormat, SourceSetKind};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bundled_tools::resolve_bundled_tool;
use crate::infrastructure::internal_adapters::{
    system_process_runner, ProcessCommand, ProcessOutput, ProcessRunner,
};
use crate::infrastructure::native_operations::single_file_publisher::{
    with_publication_locks_mode, PublicationTreeLockMode,
};
#[cfg(unix)]
use crate::infrastructure::platform::filesystem::hard_link_count;
use crate::infrastructure::platform::filesystem::{
    file_identity, metadata_is_link_or_reparse_point, restrict_stage_to_owner, FileIdentity,
};
use crate::infrastructure::platform_xml_owner::root_version_literal;
use crate::infrastructure::plugin_runtime::find_plugin_root;
use crate::infrastructure::project_sources::classify_physical_source_inventory;
use crate::infrastructure::redaction::redactor;
use crate::infrastructure::source_roots::normalize_path_identity;
use roxmltree::Document;
use serde_json::{Map, Value};
use serde_yaml::{Mapping as YamlMapping, Value as YamlValue};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::cell::RefCell;
use std::env;
#[cfg(unix)]
use std::ffi::CString;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
#[cfg(unix)]
use std::io::Read;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};

const TARGET_PLATFORM_LINE: &str = ACTIVE_FORMAT_PROFILE.platform_line;
const TARGET_EXPORT_FORMAT: &str = ACTIVE_FORMAT_PROFILE.export_format;
const EFFECTIVE_CONFIG_NAME: &str = "v8project.yaml";
const LOCAL_CONFIG_NAME: &str = "v8project.local.yaml";
const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
const BUILD_DUMP_TIMEOUT: Duration = Duration::from_secs(120);
const PLATFORM_PROBE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FullDumpInvocation {
    BuildDump,
    RuntimeExecute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlatformUtility {
    Designer,
    Ibcmd,
}

impl PlatformUtility {
    fn executable_name(self) -> &'static str {
        #[cfg(windows)]
        {
            match self {
                Self::Designer => "1cv8.exe",
                Self::Ibcmd => "ibcmd.exe",
            }
        }
        #[cfg(not(windows))]
        {
            match self {
                Self::Designer => "1cv8",
                Self::Ibcmd => "ibcmd",
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VerifiedPlatform {
    executable: PathBuf,
    exact_version: String,
    attestation: PlatformAttestation,
}

trait PlatformResolver {
    fn resolve(
        &self,
        effective_config: &YamlValue,
        config_dir: &Path,
        utility: PlatformUtility,
        runner: &dyn ProcessRunner,
        cancellation: &CancellationToken,
    ) -> Result<VerifiedPlatform, String>;
}

struct SystemPlatformResolver;
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

pub(crate) struct VerifiedFullDumpAdapter<'a> {
    runner: &'a dyn ProcessRunner,
    platform_resolver: &'a dyn PlatformResolver,
    bundled_program_override: Option<PathBuf>,
}

impl VerifiedFullDumpAdapter<'static> {
    pub(crate) fn new() -> Self {
        Self {
            runner: system_process_runner(),
            platform_resolver: &SYSTEM_PLATFORM_RESOLVER,
            bundled_program_override: None,
        }
    }
}

impl<'a> VerifiedFullDumpAdapter<'a> {
    #[cfg(test)]
    fn with_dependencies(
        runner: &'a dyn ProcessRunner,
        platform_resolver: &'a dyn PlatformResolver,
        bundled_program: PathBuf,
    ) -> Self {
        Self {
            runner,
            platform_resolver,
            bundled_program_override: Some(bundled_program),
        }
    }

    pub(crate) fn invoke(
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

        let mut prepared = match PreparedDump::prepare(
            args,
            context,
            self.platform_resolver,
            self.runner,
            cancellation,
        ) {
            Ok(prepared) => prepared,
            Err(error) => return Ok(dump_failure(tool_name, error, None, None, None)),
        };
        macro_rules! finish_prepared {
            ($outcome:expr) => {
                return Ok(finalize_private_outcome(&mut prepared.private, $outcome))
            };
        }
        let bundled = if let Some(program) = &self.bundled_program_override {
            (program.clone(), Vec::new())
        } else {
            let Some(plugin_root) = find_plugin_root(&context.cwd) else {
                finish_prepared!(dump_failure(
                    tool_name,
                    "could not locate Unica plugin root for internal adapter lookup".to_string(),
                    None,
                    None,
                    None,
                ));
            };
            let bundled = match resolve_bundled_tool(&plugin_root, "v8-runner", true) {
                Ok(bundled) => bundled,
                Err(error) => {
                    finish_prepared!(dump_failure(tool_name, error, None, None, None));
                }
            };
            (bundled.program, bundled.warnings)
        };

        let execution_args = dump_process_args(
            invocation,
            &prepared.private.effective_config,
            prepared.source_set_name.as_deref(),
            prepared.extension.as_deref(),
        );
        let report_args = reported_dump_process_args(
            invocation,
            prepared.source_set_name.as_deref(),
            prepared.extension.as_deref(),
        );
        let mut reported_command = vec![bundled.0.display().to_string()];
        reported_command.extend(report_args);
        if let Err(error) = prepared.platform_attestation.recheck() {
            finish_prepared!(dump_failure(
                tool_name,
                format!("platform trust check immediately before v8-runner failed: {error}"),
                None,
                None,
                Some(reported_command),
            ));
        }
        let output_result = self.runner.run(&ProcessCommand {
            program: bundled.0,
            args: execution_args,
            cwd: context.cwd.clone(),
            timeout: match invocation {
                FullDumpInvocation::BuildDump => Some(BUILD_DUMP_TIMEOUT),
                FullDumpInvocation::RuntimeExecute => None,
            },
            cancellation: cancellation.clone(),
        });
        let platform_recheck = prepared.platform_attestation.recheck();
        let config_cleanup = prepared.private.remove_effective_config();
        let output = match (output_result, platform_recheck, config_cleanup) {
            (Ok(output), Ok(()), Ok(())) => output,
            (Err(error), Ok(()), Ok(())) => {
                finish_prepared!(dump_failure(
                    tool_name,
                    redactor(&error),
                    None,
                    None,
                    Some(reported_command),
                ));
            }
            (Ok(_), Err(platform_error), Ok(())) => {
                finish_prepared!(dump_failure(
                    tool_name,
                    format!(
                        "platform trust check immediately after v8-runner failed: {platform_error}"
                    ),
                    None,
                    None,
                    Some(reported_command),
                ));
            }
            (Ok(_), Ok(()), Err(cleanup_error)) => {
                finish_prepared!(dump_failure(
                    tool_name,
                    cleanup_error,
                    None,
                    None,
                    Some(reported_command),
                ));
            }
            (output_result, platform_recheck, config_cleanup) => {
                let mut errors = Vec::new();
                if let Err(error) = output_result {
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

        let staged_snapshot =
            match validate_staged_dump(&prepared.private.staged_tree, prepared.source_kind) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    finish_prepared!(dump_failure(
                        tool_name,
                        error,
                        Some(redactor(&output.stdout)),
                        Some(redactor(&output.stderr)),
                        Some(reported_command),
                    ));
                }
            };
        if cancellation.is_cancelled() {
            finish_prepared!(cancelled_dump_outcome(tool_name, &output, reported_command));
        }

        let target = prepared.target.clone();
        let mut private = prepared.private;
        let publication = with_publication_locks_mode(
            std::slice::from_ref(&target),
            PublicationTreeLockMode::Exclusive,
            |_| {
                for snapshot in &prepared.config_inputs {
                    snapshot.recheck()?;
                }
                prepared.workspace_anchor.verify_descendant_identity(
                    &prepared.target_parent_relative,
                    prepared.target_parent.identity,
                    prepared.target_parent.path.as_path(),
                )?;
                prepared.platform_attestation.recheck()?;
                let current_target = prepared.target_parent.capture_child(
                    target
                        .file_name()
                        .ok_or_else(|| format!("dump target has no name: {}", target.display()))?,
                    &target,
                )?;
                if current_target != prepared.target_snapshot {
                    return Err(format!(
                        "dump target changed after it was inspected; staged output was not published: {}",
                        target.display()
                    ));
                }
                let current_stage =
                    validate_staged_dump(&private.staged_tree, prepared.source_kind)?;
                if current_stage != staged_snapshot {
                    return Err(format!(
                        "private staged dump changed after validation: {}",
                        private.staged_tree.display()
                    ));
                }
                if cancellation.is_cancelled() {
                    return Err("verified dump cancelled before publication".to_string());
                }
                publish_staged_tree(
                    &mut private,
                    &target,
                    &prepared.target_parent,
                    &prepared.target_snapshot,
                    &staged_snapshot,
                )
            },
        );
        macro_rules! finish_private {
            ($outcome:expr) => {
                return Ok(finalize_private_outcome(&mut private, $outcome))
            };
        }
        let cleanup_warnings = match publication {
            Ok(Ok(warnings)) => warnings,
            Ok(Err(error)) => {
                finish_private!(dump_failure(
                    tool_name,
                    error,
                    Some(redactor(&output.stdout)),
                    Some(redactor(&output.stderr)),
                    Some(reported_command),
                ));
            }
            Err(error) => {
                finish_private!(dump_failure(
                    tool_name,
                    format!("failed to acquire verified dump publication locks: {error}"),
                    Some(redactor(&output.stdout)),
                    Some(redactor(&output.stderr)),
                    Some(reported_command),
                ));
            }
        };

        let mut warnings = bundled.1;
        warnings.extend(cleanup_warnings);
        Ok(finalize_private_outcome(
            &mut private,
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

struct PreparedDump {
    private: PrivateDumpStage,
    target: PathBuf,
    workspace_anchor: DirectoryAnchor,
    target_parent_relative: PathBuf,
    target_parent: DirectoryAnchor,
    target_snapshot: TreeSnapshot,
    platform_attestation: PlatformAttestation,
    config_inputs: Vec<ConfigInputSnapshot>,
    source_kind: SourceSetKind,
    source_set_name: Option<String>,
    extension: Option<String>,
}

impl PreparedDump {
    #[cfg(windows)]
    fn prepare(
        _args: &Map<String, Value>,
        _context: &WorkspaceContext,
        _platform_resolver: &dyn PlatformResolver,
        _runner: &dyn ProcessRunner,
        _cancellation: &CancellationToken,
    ) -> Result<Self, String> {
        Err(
            "verified applied full dump is fail-closed on Windows until owner-only ACLs and handle-safe no-clobber directory publication are implemented"
                .to_string(),
        )
    }

    #[cfg(not(windows))]
    fn prepare(
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        platform_resolver: &dyn PlatformResolver,
        runner: &dyn ProcessRunner,
        cancellation: &CancellationToken,
    ) -> Result<Self, String> {
        validate_applied_full_dump_args(args)?;
        let config_path = resolve_config_path(args, context)?;
        let config_dir = config_path.parent().unwrap_or(&context.cwd);
        let primary = ConfigInputSnapshot::required(config_path.clone())?;
        let primary_raw = primary
            .raw()
            .expect("required config snapshot always contains bytes");
        let mut effective = parse_yaml_mapping(primary_raw, &config_path)?;

        let local_path = config_dir.join(LOCAL_CONFIG_NAME);
        let local = ConfigInputSnapshot::optional(local_path.clone())?;
        if let Some(raw) = local.raw() {
            let overlay = parse_yaml_mapping(raw, &local_path)?;
            validate_local_overlay_keys(&overlay, &local_path)?;
            merge_yaml_values(&mut effective, overlay);
        }
        if cancellation.is_cancelled() {
            return Err("verified dump cancelled while preparing effective config".to_string());
        }

        let format = yaml_string(&effective, "format")?.unwrap_or("DESIGNER");
        if format != "DESIGNER" {
            return Err(format!(
                "applied full dump supports only format=DESIGNER source sets; configured format is {format}"
            ));
        }
        let builder = yaml_string(&effective, "builder")?.unwrap_or("DESIGNER");
        let utility = match builder {
            "DESIGNER" => PlatformUtility::Designer,
            "IBCMD" => PlatformUtility::Ibcmd,
            other => {
                return Err(format!(
                    "applied full dump supports only builder=DESIGNER or IBCMD; configured builder is {other}"
                ));
            }
        };

        if yaml_mapping(&effective)?.contains_key(yaml_key("basePath")) {
            return Err(
                "pinned v8-runner 0.5.1 rejects the removed top-level `basePath` key; paths are resolved relative to the primary config directory"
                    .to_string(),
            );
        }
        let base_path = normalize_path_identity(config_dir)?;
        let selection = select_source_set(&effective, args)?;
        validate_selected_source_path(&selection.path)?;
        normalize_source_set_paths(&mut effective, &base_path)?;
        normalize_relocated_config_paths(&mut effective, config_dir)?;

        let configured_work_path = yaml_string(&effective, "workPath")?
            .ok_or_else(|| "v8project.yaml field `workPath` must be a string".to_string())?;
        let work_path = args
            .get("workdir")
            .and_then(Value::as_str)
            .map(|path| absolutize(Path::new(path), config_dir))
            .unwrap_or_else(|| absolutize(Path::new(configured_work_path), config_dir));
        let work_path = normalize_path_identity(&work_path)?;
        set_yaml_string(&mut effective, "workPath", &work_path.display().to_string())?;
        normalize_infobase_connection(&mut effective, config_dir)?;

        let workspace_root = normalize_path_identity(&context.workspace_root)?;
        let workspace_anchor = DirectoryAnchor::capture_exact(&workspace_root)?;
        let configured_target = base_path.join(&selection.path);
        let target = normalize_leaf_path(&configured_target)?;
        validate_dump_target(&target, &workspace_root, &base_path, &work_path)?;
        let target_parent_path = target
            .parent()
            .ok_or_else(|| format!("dump target has no parent directory: {}", target.display()))?;
        let target_parent_relative = target_parent_path
            .strip_prefix(&workspace_root)
            .map_err(|_| {
                format!(
                    "dump target parent is not physically addressable below workspace root {}: {}",
                    workspace_root.display(),
                    target_parent_path.display()
                )
            })?
            .to_path_buf();
        run_target_parent_capture_hook(target_parent_path);
        let target_parent =
            workspace_anchor.capture_descendant(&target_parent_relative, target_parent_path)?;
        let target_snapshot = target_parent.capture_child(
            target
                .file_name()
                .ok_or_else(|| format!("dump target has no name: {}", target.display()))?,
            &target,
        )?;
        workspace_anchor.verify_descendant_identity(
            &target_parent_relative,
            target_parent.identity,
            target_parent_path,
        )?;
        validate_physical_dump_target(&target, &target_snapshot, selection.kind)?;

        let platform =
            platform_resolver.resolve(&effective, config_dir, utility, runner, cancellation)?;
        let platform_version =
            parse_exact_platform_version(&platform.exact_version).ok_or_else(|| {
                format!(
                    "resolved platform version {} is not an exact four-part version",
                    platform.exact_version
                )
            })?;
        if platform_version[..3] != [8, 3, 27] {
            return Err(format!(
                "resolved platform {} is not supported; required {TARGET_PLATFORM_LINE}.x",
                platform.exact_version
            ));
        }
        let mut private = PrivateDumpStage::create(&target_parent)?;
        let private_setup = (|| {
            redirect_selected_source_set(&mut effective, selection.index, &private.staged_tree)?;
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
            private.write_effective_config(&effective)
        })();
        if let Err(error) = private_setup {
            return match private.cleanup_now() {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(format!("{error}; {cleanup_error}")),
            };
        }

        Ok(Self {
            private,
            target,
            workspace_anchor,
            target_parent_relative,
            target_parent,
            target_snapshot,
            platform_attestation: platform.attestation,
            config_inputs: vec![primary, local],
            source_kind: selection.kind,
            source_set_name: selection.explicit_source_set,
            extension: selection.extension,
        })
    }
}

#[derive(Debug)]
struct SourceSelection {
    index: usize,
    kind: SourceSetKind,
    path: PathBuf,
    explicit_source_set: Option<String>,
    extension: Option<String>,
}

fn select_source_set(
    effective: &YamlValue,
    args: &Map<String, Value>,
) -> Result<SourceSelection, String> {
    let entries = yaml_mapping(effective)?
        .get(yaml_key("source-set"))
        .and_then(YamlValue::as_sequence)
        .ok_or_else(|| "v8project.yaml field `source-set` must be a list".to_string())?;
    let source_set = args
        .get("sourceSet")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let extension = args
        .get("extension")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());

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

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfigInputState {
    Exact(SecureFileSnapshot),
    Absent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigInputSnapshot {
    path: PathBuf,
    state: ConfigInputState,
}

impl ConfigInputSnapshot {
    fn required(path: PathBuf) -> Result<Self, String> {
        let snapshot = secure_read_regular_file_snapshot(&path, "project config")?;
        Ok(Self {
            path,
            state: ConfigInputState::Exact(snapshot),
        })
    }

    fn optional(path: PathBuf) -> Result<Self, String> {
        match secure_read_regular_file_snapshot(&path, "local config overlay") {
            Ok(snapshot) => Ok(Self {
                state: ConfigInputState::Exact(snapshot),
                path,
            }),
            Err(read_error) => {
                if secure_path_is_absent(&path)? {
                    Ok(Self {
                        path: normalize_leaf_path(&path)?,
                        state: ConfigInputState::Absent,
                    })
                } else {
                    Err(read_error)
                }
            }
        }
    }

    fn raw(&self) -> Option<&[u8]> {
        match &self.state {
            ConfigInputState::Exact(snapshot) => Some(&snapshot.raw),
            ConfigInputState::Absent => None,
        }
    }

    fn recheck(&self) -> Result<(), String> {
        match &self.state {
            ConfigInputState::Exact(expected) => {
                let actual =
                    secure_read_regular_file_snapshot(&self.path, "project config preimage")?;
                if &actual != expected {
                    return Err(format!(
                        "project config changed during full dump; staged output was not published: {}",
                        self.path.display()
                    ));
                }
            }
            ConfigInputState::Absent => match secure_path_is_absent(&self.path) {
                Ok(true) => {}
                Ok(false) => {
                    return Err(format!(
                        "local config overlay appeared during full dump; staged output was not published: {}",
                        self.path.display()
                    ));
                }
                Err(error) => {
                    return Err(format!(
                        "failed to recheck absent local config overlay {}: {error}",
                        self.path.display()
                    ));
                }
            },
        }
        Ok(())
    }
}

fn secure_read_regular_file(path: &Path, role: &str) -> Result<Vec<u8>, String> {
    secure_read_regular_file_snapshot(path, role).map(|snapshot| snapshot.raw)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SecureFileSnapshot {
    identity: FileIdentity,
    raw: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SecureFileDigest {
    identity: FileIdentity,
    size: u64,
    sha256: [u8; 32],
}

impl SecureFileDigest {
    fn capture(path: &Path, role: &str) -> Result<Self, String> {
        let snapshot = secure_read_regular_file_snapshot(path, role)?;
        let size = snapshot.raw.len() as u64;
        let sha256 = Sha256::digest(&snapshot.raw).into();
        Ok(Self {
            identity: snapshot.identity,
            size,
            sha256,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlatformAttestation {
    executable_path: PathBuf,
    executable: SecureFileDigest,
    probe_path: PathBuf,
    probe: SecureFileDigest,
    install_path: PathBuf,
    install_inventory: TreeSnapshot,
    trust: PlatformTrustAttestation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlatformTrustPolicy {
    Immutable,
    #[cfg(test)]
    TestFixture,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PlatformTrustAttestation {
    Immutable(ImmutablePlatformTrustSnapshot),
    #[cfg(test)]
    TestFixture,
}

impl PlatformTrustAttestation {
    fn policy(&self) -> PlatformTrustPolicy {
        match self {
            Self::Immutable(_) => PlatformTrustPolicy::Immutable,
            #[cfg(test)]
            Self::TestFixture => PlatformTrustPolicy::TestFixture,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImmutablePlatformTrustSnapshot {
    entries: Vec<ImmutablePlatformEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImmutablePlatformEntryKind {
    Directory,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImmutablePlatformEntry {
    path: PathBuf,
    kind: ImmutablePlatformEntryKind,
    identity: FileIdentity,
    owner: u32,
    mode: u32,
}

impl PlatformAttestation {
    fn capture_immutable(executable: &Path) -> Result<Self, String> {
        Self::capture_with_policy(executable, PlatformTrustPolicy::Immutable)
    }

    #[cfg(test)]
    fn capture_test_fixture(executable: &Path) -> Result<Self, String> {
        Self::capture_with_policy(executable, PlatformTrustPolicy::TestFixture)
    }

    fn capture_with_policy(
        executable: &Path,
        trust_policy: PlatformTrustPolicy,
    ) -> Result<Self, String> {
        let executable_path = normalize_leaf_path(executable)?;
        let install_path = executable_path
            .parent()
            .ok_or_else(|| {
                format!(
                    "platform executable has no installation directory: {}",
                    executable_path.display()
                )
            })?
            .to_path_buf();
        let probe_name = if cfg!(windows) { "ibcmd.exe" } else { "ibcmd" };
        let probe_path = if executable_path.file_name() == Some(OsStr::new(probe_name)) {
            executable_path.clone()
        } else {
            install_path.join(probe_name)
        };
        let trust_before = match trust_policy {
            PlatformTrustPolicy::Immutable => {
                PlatformTrustAttestation::Immutable(ImmutablePlatformTrustSnapshot::capture(
                    &install_path,
                    &executable_path,
                    &probe_path,
                )?)
            }
            #[cfg(test)]
            PlatformTrustPolicy::TestFixture => PlatformTrustAttestation::TestFixture,
        };
        let executable_digest =
            SecureFileDigest::capture(&executable_path, "platform executable attestation")?;
        let probe_digest =
            SecureFileDigest::capture(&probe_path, "platform version probe attestation")?;
        let install_inventory = TreeSnapshot::capture_target(&install_path)?;
        if !matches!(install_inventory, TreeSnapshot::Directory { .. }) {
            return Err(format!(
                "platform installation disappeared during attestation: {}",
                install_path.display()
            ));
        }
        let trust = match trust_policy {
            PlatformTrustPolicy::Immutable => {
                let after =
                    PlatformTrustAttestation::Immutable(ImmutablePlatformTrustSnapshot::capture(
                        &install_path,
                        &executable_path,
                        &probe_path,
                    )?);
                if after != trust_before {
                    return Err(format!(
                        "immutable platform ownership or mode changed during attestation: {}",
                        install_path.display()
                    ));
                }
                after
            }
            #[cfg(test)]
            PlatformTrustPolicy::TestFixture => PlatformTrustAttestation::TestFixture,
        };
        Ok(Self {
            executable_path,
            executable: executable_digest,
            probe_path,
            probe: probe_digest,
            install_path,
            install_inventory,
            trust,
        })
    }

    fn recheck(&self) -> Result<(), String> {
        let current = Self::capture_with_policy(&self.executable_path, self.trust.policy())?;
        if &current == self {
            Ok(())
        } else {
            Err(format!(
                "platform attestation changed during full dump; staged output was not published: {}",
                self.install_path.display()
            ))
        }
    }
}

impl ImmutablePlatformTrustSnapshot {
    #[cfg(unix)]
    fn capture(install: &Path, executable: &Path, probe: &Path) -> Result<Self, String> {
        verify_unprivileged_platform_caller()?;
        let mut entries = capture_immutable_platform_ancestry(install)?;
        let install_directory = open_directory_nofollow(install).map_err(|error| {
            format!(
                "failed to securely open immutable platform installation {}: {error}",
                install.display()
            )
        })?;
        capture_immutable_platform_children(&install_directory, install, &mut entries)?;
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        entries.dedup();
        for (path, role) in [
            (executable, "platform executable"),
            (probe, "platform version probe"),
        ] {
            let Some(entry) = entries.iter().find(|entry| entry.path == path) else {
                return Err(format!(
                    "{role} is outside the immutable platform inventory: {}",
                    path.display()
                ));
            };
            if entry.kind != ImmutablePlatformEntryKind::File || entry.mode & 0o111 == 0 {
                return Err(format!(
                    "{role} is not an executable immutable regular file: {}",
                    path.display()
                ));
            }
        }
        Ok(Self { entries })
    }

    #[cfg(not(unix))]
    fn capture(_install: &Path, _executable: &Path, _probe: &Path) -> Result<Self, String> {
        Err("immutable platform trust verification is unavailable on this host".to_string())
    }
}

#[cfg(unix)]
fn verify_unprivileged_platform_caller() -> Result<(), String> {
    // SAFETY: geteuid has no preconditions and does not mutate process state.
    if unsafe { libc::geteuid() } == 0 {
        return Err(
            "immutable platform execution is refused for an effective root caller".to_string(),
        );
    }
    #[cfg(target_os = "linux")]
    {
        let status = fs::read_to_string("/proc/self/status").map_err(|error| {
            format!("failed to verify Linux effective capabilities from /proc/self/status: {error}")
        })?;
        let capabilities = status
            .lines()
            .find_map(|line| line.strip_prefix("CapEff:"))
            .map(str::trim)
            .ok_or_else(|| {
                "Linux effective capabilities are absent from /proc/self/status".to_string()
            })?;
        if capabilities.is_empty()
            || !capabilities
                .bytes()
                .all(|byte| byte == b'0' || byte.is_ascii_hexdigit())
        {
            return Err("Linux effective capabilities have an invalid encoding".to_string());
        }
        if capabilities.bytes().any(|byte| byte != b'0') {
            return Err(
                "immutable platform execution is refused for a caller with effective Linux capabilities"
                    .to_string(),
            );
        }
    }
    Ok(())
}

#[cfg(unix)]
fn capture_immutable_platform_ancestry(
    install: &Path,
) -> Result<Vec<ImmutablePlatformEntry>, String> {
    use std::path::Component;

    if !install.is_absolute() {
        return Err(format!(
            "immutable platform installation path must be absolute: {}",
            install.display()
        ));
    }
    let root_path = PathBuf::from("/");
    let root = open_directory_nofollow(&root_path)
        .map_err(|error| format!("failed to securely open platform ancestry root: {error}"))?;
    let mut entries = vec![capture_immutable_platform_entry(
        &root,
        &root_path,
        ImmutablePlatformEntryKind::Directory,
    )?];
    let mut current = root;
    let mut current_path = root_path;
    for component in install.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => {
                current = open_directory_child_nofollow(&current, name).map_err(|error| {
                    format!(
                        "platform ancestry contains a link, non-directory, or inaccessible component {}: {error}",
                        current_path.join(name).display()
                    )
                })?;
                current_path.push(name);
                entries.push(capture_immutable_platform_entry(
                    &current,
                    &current_path,
                    ImmutablePlatformEntryKind::Directory,
                )?);
            }
            Component::ParentDir | Component::Prefix(_) => {
                return Err(format!(
                    "immutable platform installation path contains a non-normal component: {}",
                    install.display()
                ));
            }
        }
    }
    Ok(entries)
}

#[cfg(unix)]
fn capture_immutable_platform_children(
    directory: &File,
    display_path: &Path,
    entries: &mut Vec<ImmutablePlatformEntry>,
) -> Result<(), String> {
    for name in read_directory_names(directory).map_err(|error| {
        format!(
            "failed to enumerate immutable platform installation {}: {error}",
            display_path.display()
        )
    })? {
        let child_path = display_path.join(&name);
        match open_directory_child_nofollow(directory, &name) {
            Ok(child) => {
                let entry = capture_immutable_platform_entry(
                    &child,
                    &child_path,
                    ImmutablePlatformEntryKind::Directory,
                )?;
                let expected_identity = entry.identity;
                entries.push(entry);
                capture_immutable_platform_children(&child, &child_path, entries)?;
                let rebound = open_directory_child_nofollow(directory, &name).map_err(|error| {
                    format!(
                        "failed to rebind immutable platform directory {}: {error}",
                        child_path.display()
                    )
                })?;
                if file_identity(&rebound).map_err(|error| {
                    format!(
                        "failed to recheck immutable platform directory {}: {error}",
                        child_path.display()
                    )
                })? != expected_identity
                {
                    return Err(format!(
                        "immutable platform directory identity changed while inspecting: {}",
                        child_path.display()
                    ));
                }
            }
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(libc::ENOTDIR) | Some(libc::EINVAL)
                ) =>
            {
                let file = open_regular_child_nofollow(directory, &name).map_err(|file_error| {
                    let detail = if file_error.raw_os_error() == Some(libc::ELOOP) {
                        "symbolic link".to_string()
                    } else {
                        file_error.to_string()
                    };
                    format!(
                        "immutable platform inventory contains an unsupported entry {}: {detail}",
                        child_path.display()
                    )
                })?;
                if hard_link_count(&file).map_err(|link_error| {
                    format!(
                        "failed to inspect hard links for immutable platform file {}: {link_error}",
                        child_path.display()
                    )
                })? != 1
                {
                    return Err(format!(
                        "immutable platform file must have exactly one hard link: {}",
                        child_path.display()
                    ));
                }
                entries.push(capture_immutable_platform_entry(
                    &file,
                    &child_path,
                    ImmutablePlatformEntryKind::File,
                )?);
            }
            Err(error) => {
                let detail = if error.raw_os_error() == Some(libc::ELOOP) {
                    "symbolic link".to_string()
                } else {
                    error.to_string()
                };
                return Err(format!(
                    "immutable platform inventory contains an unsupported entry {}: {detail}",
                    child_path.display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn capture_immutable_platform_entry(
    file: &File,
    path: &Path,
    kind: ImmutablePlatformEntryKind,
) -> Result<ImmutablePlatformEntry, String> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata().map_err(|error| {
        format!(
            "failed to inspect immutable platform entry {}: {error}",
            path.display()
        )
    })?;
    if metadata.uid() != 0 {
        return Err(format!(
            "platform installation must be owned by root; user-owned entry refused: {}",
            path.display()
        ));
    }
    let mode = metadata.mode() & 0o7777;
    if mode & 0o022 != 0 {
        return Err(format!(
            "platform installation entry is group/world writable and not immutable: {}",
            path.display()
        ));
    }
    if kind == ImmutablePlatformEntryKind::File && mode & 0o6000 != 0 {
        return Err(format!(
            "setuid/setgid platform file is not accepted: {}",
            path.display()
        ));
    }
    verify_platform_entry_has_no_acl(file, path)?;
    Ok(ImmutablePlatformEntry {
        path: path.to_path_buf(),
        kind,
        identity: file_identity(file).map_err(|error| {
            format!(
                "failed to capture immutable platform identity {}: {error}",
                path.display()
            )
        })?,
        owner: metadata.uid(),
        mode,
    })
}

#[cfg(target_os = "macos")]
fn verify_platform_entry_has_no_acl(file: &File, path: &Path) -> Result<(), String> {
    type DarwinAcl = *mut libc::c_void;
    const ACL_TYPE_EXTENDED: libc::c_int = 0x0000_0100;
    unsafe extern "C" {
        fn acl_get_fd_np(fd: libc::c_int, acl_type: libc::c_int) -> DarwinAcl;
        fn acl_free(object: *mut libc::c_void) -> libc::c_int;
    }

    // SAFETY: file owns a valid descriptor; acl_get_fd_np returns an owned ACL
    // object that is released below when present.
    let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED) };
    if !acl.is_null() {
        // SAFETY: acl is a live allocation returned by acl_get_fd_np.
        let release_status = unsafe { acl_free(acl) };
        if release_status != 0 {
            return Err(format!(
                "failed to release ACL metadata for immutable platform entry {}: {}",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
        return Err(format!(
            "platform installation entry has an extended ACL and is not immutable: {}",
            path.display()
        ));
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == ErrorKind::NotFound {
        Ok(())
    } else {
        Err(format!(
            "could not prove that platform entry has no extended ACL {}: {error}",
            path.display()
        ))
    }
}

#[cfg(target_os = "linux")]
fn verify_platform_entry_has_no_acl(file: &File, path: &Path) -> Result<(), String> {
    for name in [
        &b"system.posix_acl_access\0"[..],
        &b"system.posix_acl_default\0"[..],
    ] {
        // SAFETY: file owns a valid descriptor, name is NUL-terminated, and a
        // null value with zero size performs a size probe only.
        let status = unsafe {
            libc::fgetxattr(
                file.as_raw_fd(),
                name.as_ptr().cast(),
                std::ptr::null_mut(),
                0,
            )
        };
        if status >= 0 {
            return Err(format!(
                "platform installation entry has a POSIX ACL and is not immutable: {}",
                path.display()
            ));
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ENODATA) {
            return Err(format!(
                "could not prove that platform entry has no POSIX ACL {}: {error}",
                path.display()
            ));
        }
    }
    Ok(())
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn verify_platform_entry_has_no_acl(_file: &File, path: &Path) -> Result<(), String> {
    Err(format!(
        "immutable platform ACL verification is unavailable on this Unix host: {}",
        path.display()
    ))
}

#[cfg(unix)]
fn secure_read_regular_file_snapshot(
    path: &Path,
    role: &str,
) -> Result<SecureFileSnapshot, String> {
    let normalized = normalize_leaf_path(path)?;
    let (parent_path, name) = split_parent_and_name(&normalized)?;
    let parent = open_directory_nofollow(&parent_path).map_err(|error| {
        format!(
            "failed to securely open parent for {role} {}: {error}",
            normalized.display()
        )
    })?;
    run_secure_read_hook(path);
    let mut file = open_regular_child_nofollow(&parent, &name).map_err(|error| {
        let detail = if error.raw_os_error() == Some(libc::ELOOP) {
            "symbolic link or reparse point".to_string()
        } else {
            error.to_string()
        };
        format!(
            "failed to securely open {role} {}: {detail}",
            path.display()
        )
    })?;
    let identity = file_identity(&file).map_err(|error| {
        format!(
            "failed to inspect opened {role} identity {}: {error}",
            path.display()
        )
    })?;
    if hard_link_count(&file).map_err(|error| {
        format!(
            "failed to inspect hard links for {role} {}: {error}",
            path.display()
        )
    })? != 1
    {
        return Err(format!(
            "{role} must have exactly one hard link: {}",
            path.display()
        ));
    }
    let opened = file.metadata().map_err(|error| {
        format!(
            "failed to inspect opened {role} {}: {error}",
            path.display()
        )
    })?;
    let mut raw = Vec::new();
    file.read_to_end(&mut raw)
        .map_err(|error| format!("failed to read {role} {}: {error}", path.display()))?;
    let after = file.metadata().map_err(|error| {
        format!(
            "failed to recheck opened {role} {}: {error}",
            path.display()
        )
    })?;
    if opened.len() != after.len() || opened.modified().ok() != after.modified().ok() {
        return Err(format!("{role} changed while reading: {}", path.display()));
    }
    let rebound = open_regular_child_nofollow(&parent, &name).map_err(|error| {
        format!(
            "failed to rebind {role} name after reading {}: {error}",
            path.display()
        )
    })?;
    let rebound_identity = file_identity(&rebound).map_err(|error| {
        format!(
            "failed to recheck {role} identity {}: {error}",
            path.display()
        )
    })?;
    if rebound_identity != identity {
        return Err(format!(
            "{role} identity changed while reading: {}",
            path.display()
        ));
    }
    run_secure_read_after_hook(path);
    Ok(SecureFileSnapshot { identity, raw })
}

#[cfg(not(unix))]
fn secure_read_regular_file_snapshot(
    path: &Path,
    role: &str,
) -> Result<SecureFileSnapshot, String> {
    Err(format!(
        "{role} secure no-follow reads are unavailable on this host: {}",
        path.display()
    ))
}

fn normalize_leaf_path(path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|error| format!("failed to determine current directory: {error}"))?
            .join(path)
    };
    let parent = absolute
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", absolute.display()))?;
    let name = absolute
        .file_name()
        .ok_or_else(|| format!("path has no final component: {}", absolute.display()))?;
    Ok(normalize_path_identity(parent)?.join(name))
}

fn split_parent_and_name(path: &Path) -> Result<(PathBuf, OsString), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", path.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("path has no final component: {}", path.display()))?;
    Ok((parent.to_path_buf(), name.to_os_string()))
}

#[cfg(unix)]
fn secure_path_is_absent(path: &Path) -> Result<bool, String> {
    let normalized = normalize_leaf_path(path)?;
    let (parent_path, name) = split_parent_and_name(&normalized)?;
    let parent = open_directory_nofollow(&parent_path).map_err(|error| {
        format!(
            "failed to securely open parent while checking {}: {error}",
            normalized.display()
        )
    })?;
    let name = CString::new(name.as_bytes()).map_err(|error| {
        format!(
            "path contains an embedded NUL byte {}: {error}",
            normalized.display()
        )
    })?;
    // SAFETY: parent remains open for the call, name is NUL-terminated, and
    // metadata points to writable storage for the duration of fstatat.
    let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
    let status = unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            &mut metadata,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if status == 0 {
        return Ok(false);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == ErrorKind::NotFound {
        Ok(true)
    } else {
        Err(format!(
            "failed to securely inspect {}: {error}",
            normalized.display()
        ))
    }
}

#[cfg(not(unix))]
fn secure_path_is_absent(path: &Path) -> Result<bool, String> {
    Err(format!(
        "secure no-follow absence checks are unavailable on this host: {}",
        path.display()
    ))
}

#[cfg(unix)]
fn open_directory_nofollow(path: &Path) -> std::io::Result<File> {
    use std::path::Component;

    if !path.is_absolute() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            "secure directory path must be absolute",
        ));
    }
    let root = CString::new("/")?;
    // SAFETY: the static root path is NUL-terminated and the returned descriptor is owned below.
    let root_fd = unsafe {
        libc::open(
            root.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if root_fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: root_fd is a newly owned descriptor from open.
    let mut current = unsafe { File::from_raw_fd(root_fd) };
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                current = open_directory_child_nofollow(&current, name)?;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => {
                return Err(std::io::Error::new(
                    ErrorKind::InvalidInput,
                    "secure directory path contains a non-normal component",
                ));
            }
        }
    }
    Ok(current)
}

#[cfg(unix)]
fn open_directory_relative_nofollow(root: &File, relative: &Path) -> std::io::Result<File> {
    use std::path::Component;

    if relative.is_absolute() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            "secure descendant path must be relative",
        ));
    }
    let mut current = root.try_clone()?;
    for component in relative.components() {
        match component {
            Component::Normal(name) => {
                current = open_directory_child_nofollow(&current, name)?;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(std::io::Error::new(
                    ErrorKind::InvalidInput,
                    "secure descendant path contains a non-normal component",
                ));
            }
        }
    }
    Ok(current)
}

#[cfg(unix)]
fn open_directory_child_nofollow(parent: &File, name: &OsStr) -> std::io::Result<File> {
    let name = CString::new(name.as_bytes())?;
    // SAFETY: parent remains open for the call and name is a live NUL-terminated string.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: fd is a newly owned descriptor from openat.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

#[cfg(unix)]
fn open_regular_child_nofollow(parent: &File, name: &OsStr) -> std::io::Result<File> {
    let name = CString::new(name.as_bytes())?;
    // SAFETY: parent remains open for the call and name is a live NUL-terminated string.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: fd is a newly owned descriptor from openat.
    let file = unsafe { File::from_raw_fd(fd) };
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "entry is not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(unix)]
fn create_regular_child_owner_only(parent: &File, name: &OsStr) -> std::io::Result<File> {
    let name = CString::new(name.as_bytes())?;
    // SAFETY: parent remains open for the call and name is a live
    // NUL-terminated string. The file starts owner-only; there is no
    // world-readable chmod window for credentials in the effective config.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: fd is a newly owned descriptor from openat.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

#[cfg(not(unix))]
fn create_regular_child_owner_only(_parent: &File, _name: &OsStr) -> std::io::Result<File> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "secure openat config creation is unavailable on this host",
    ))
}

#[cfg(unix)]
fn unlink_child_at(parent: &File, name: &OsStr, flags: libc::c_int) -> std::io::Result<()> {
    let name = CString::new(name.as_bytes())?;
    // SAFETY: parent remains open and name is a live NUL-terminated string.
    let status = unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn unlink_directory_child_at(parent: &File, name: &OsStr) -> std::io::Result<()> {
    unlink_child_at(parent, name, libc::AT_REMOVEDIR)
}

#[cfg(not(unix))]
fn unlink_directory_child_at(_parent: &File, _name: &OsStr) -> std::io::Result<()> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "secure unlinkat directory cleanup is unavailable on this host",
    ))
}

#[cfg(unix)]
fn unlink_non_directory_child_at(parent: &File, name: &OsStr) -> std::io::Result<()> {
    unlink_child_at(parent, name, 0)
}

#[cfg(not(unix))]
fn unlink_non_directory_child_at(_parent: &File, _name: &OsStr) -> std::io::Result<()> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "secure unlinkat file cleanup is unavailable on this host",
    ))
}

#[derive(Debug)]
struct DirectoryAnchor {
    path: PathBuf,
    identity: FileIdentity,
    directory: File,
}

impl DirectoryAnchor {
    #[cfg(unix)]
    fn capture_exact(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() {
            return Err(format!(
                "directory anchor path must already be absolute: {}",
                path.display()
            ));
        }
        let path = path.to_path_buf();
        let directory = open_directory_nofollow(&path).map_err(|error| {
            format!(
                "failed to securely open directory anchor {}: {error}",
                path.display()
            )
        })?;
        let identity = file_identity(&directory).map_err(|error| {
            format!(
                "failed to inspect directory anchor {}: {error}",
                path.display()
            )
        })?;
        Ok(Self {
            path,
            identity,
            directory,
        })
    }

    #[cfg(unix)]
    fn capture_descendant(&self, relative: &Path, display_path: &Path) -> Result<Self, String> {
        self.verify_path_binding()?;
        let directory =
            open_directory_relative_nofollow(&self.directory, relative).map_err(|error| {
                format!(
                    "failed to bind dump target parent below retained workspace anchor {}: {error}",
                    display_path.display()
                )
            })?;
        let identity = file_identity(&directory).map_err(|error| {
            format!(
                "failed to inspect workspace-contained directory {}: {error}",
                display_path.display()
            )
        })?;
        self.verify_descendant_identity(relative, identity, display_path)?;
        Ok(Self {
            path: display_path.to_path_buf(),
            identity,
            directory,
        })
    }

    #[cfg(unix)]
    fn verify_descendant_identity(
        &self,
        relative: &Path,
        expected_identity: FileIdentity,
        display_path: &Path,
    ) -> Result<(), String> {
        self.verify_path_binding()?;
        let rebound =
            open_directory_relative_nofollow(&self.directory, relative).map_err(|error| {
                format!(
                    "workspace-contained directory could not be rebound without following links {}: {error}",
                    display_path.display()
                )
            })?;
        let actual_identity = file_identity(&rebound).map_err(|error| {
            format!(
                "failed to recheck workspace-contained directory {}: {error}",
                display_path.display()
            )
        })?;
        if actual_identity != expected_identity {
            return Err(format!(
                "workspace-contained directory identity changed during full dump: {}",
                display_path.display()
            ));
        }
        self.verify_path_binding()
    }

    #[cfg(not(unix))]
    fn capture_descendant(&self, _relative: &Path, display_path: &Path) -> Result<Self, String> {
        Err(format!(
            "secure workspace-relative directory anchors are unavailable on this host: {}",
            display_path.display()
        ))
    }

    #[cfg(not(unix))]
    fn verify_descendant_identity(
        &self,
        _relative: &Path,
        _expected_identity: FileIdentity,
        display_path: &Path,
    ) -> Result<(), String> {
        Err(format!(
            "secure workspace-relative containment checks are unavailable on this host: {}",
            display_path.display()
        ))
    }

    #[cfg(unix)]
    fn try_clone(&self) -> Result<Self, String> {
        let directory = self.directory.try_clone().map_err(|error| {
            format!(
                "failed to duplicate directory anchor {}: {error}",
                self.path.display()
            )
        })?;
        Ok(Self {
            path: self.path.clone(),
            identity: self.identity,
            directory,
        })
    }

    #[cfg(not(unix))]
    fn try_clone(&self) -> Result<Self, String> {
        Err(format!(
            "secure directory anchors are unavailable on this host: {}",
            self.path.display()
        ))
    }

    #[cfg(unix)]
    fn create_child(&self, name: &OsStr, display_path: &Path) -> Result<Self, String> {
        self.verify_path_binding()?;
        let name_c = CString::new(name.as_bytes()).map_err(|error| {
            format!(
                "private directory name contains an embedded NUL byte {}: {error}",
                display_path.display()
            )
        })?;
        // SAFETY: the retained parent descriptor and NUL-terminated child name
        // remain live. Mode 0700 is restrictive even before the umask applies.
        let status = unsafe { libc::mkdirat(self.directory.as_raw_fd(), name_c.as_ptr(), 0o700) };
        if status != 0 {
            return Err(format!(
                "failed to create private directory {}: {}",
                display_path.display(),
                std::io::Error::last_os_error()
            ));
        }
        let directory = match open_directory_child_nofollow(&self.directory, name) {
            Ok(directory) => directory,
            Err(error) => {
                let _ = unlink_directory_child_at(&self.directory, name);
                return Err(format!(
                    "failed to bind newly created private directory {}: {error}",
                    display_path.display()
                ));
            }
        };
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = directory.set_permissions(fs::Permissions::from_mode(0o700)) {
            let _ = unlink_directory_child_at(&self.directory, name);
            return Err(format!(
                "failed to restrict private directory {}: {error}",
                display_path.display()
            ));
        }
        let identity = match file_identity(&directory) {
            Ok(identity) => identity,
            Err(error) => {
                let _ = unlink_directory_child_at(&self.directory, name);
                return Err(format!(
                    "failed to inspect private directory identity {}: {error}",
                    display_path.display()
                ));
            }
        };
        if let Err(error) = self.verify_path_binding() {
            let cleanup = unlink_directory_child_at(&self.directory, name);
            return Err(match cleanup {
                Ok(()) => error,
                Err(cleanup_error) => format!(
                    "{error}; failed to remove the private directory created through the retained parent anchor {}: {cleanup_error}",
                    display_path.display()
                ),
            });
        }
        Ok(Self {
            path: display_path.to_path_buf(),
            identity,
            directory,
        })
    }

    #[cfg(not(unix))]
    fn create_child(&self, _name: &OsStr, display_path: &Path) -> Result<Self, String> {
        Err(format!(
            "secure mkdirat private directories are unavailable on this host: {}",
            display_path.display()
        ))
    }

    #[cfg(not(unix))]
    fn capture_exact(path: &Path) -> Result<Self, String> {
        Err(format!(
            "secure directory anchors are unavailable on this host: {}",
            path.display()
        ))
    }

    #[cfg(unix)]
    fn verify_path_binding(&self) -> Result<(), String> {
        let directory = open_directory_nofollow(&self.path).map_err(|error| {
            format!(
                "failed to securely reopen directory anchor {}: {error}",
                self.path.display()
            )
        })?;
        let identity = file_identity(&directory).map_err(|error| {
            format!(
                "failed to recheck directory anchor {}: {error}",
                self.path.display()
            )
        })?;
        if identity != self.identity {
            return Err(format!(
                "directory anchor identity changed during full dump: {}",
                self.path.display()
            ));
        }
        Ok(())
    }

    #[cfg(not(unix))]
    fn verify_path_binding(&self) -> Result<(), String> {
        Err(format!(
            "secure directory anchors are unavailable on this host: {}",
            self.path.display()
        ))
    }

    fn capture_child(&self, name: &OsStr, display_path: &Path) -> Result<TreeSnapshot, String> {
        self.verify_path_binding()?;
        let snapshot = capture_tree_child_nofollow(&self.directory, name, display_path)?;
        self.verify_path_binding()?;
        Ok(snapshot)
    }

    #[cfg(unix)]
    fn capture_child_root_identity(
        &self,
        name: &OsStr,
        display_path: &Path,
    ) -> Result<Option<FileIdentity>, String> {
        self.verify_path_binding()?;
        let directory = match open_directory_child_nofollow(&self.directory, name) {
            Ok(directory) => directory,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.verify_path_binding()?;
                return Ok(None);
            }
            Err(error) => {
                return Err(format!(
                    "failed to bind dump target root identity {}: {error}",
                    display_path.display()
                ));
            }
        };
        let identity = file_identity(&directory).map_err(|error| {
            format!(
                "failed to inspect dump target root identity {}: {error}",
                display_path.display()
            )
        })?;
        self.verify_path_binding()?;
        Ok(Some(identity))
    }

    #[cfg(not(unix))]
    fn capture_child_root_identity(
        &self,
        _name: &OsStr,
        display_path: &Path,
    ) -> Result<Option<FileIdentity>, String> {
        Err(format!(
            "secure dump target root identity checks are unavailable on this host: {}",
            display_path.display()
        ))
    }
}

struct PrivateDumpStage {
    parent_anchor: DirectoryAnchor,
    root_anchor: DirectoryAnchor,
    root_name: OsString,
    root: PathBuf,
    execution: PathBuf,
    recovery: PathBuf,
    execution_anchor: DirectoryAnchor,
    recovery_anchor: DirectoryAnchor,
    staged_tree: PathBuf,
    effective_config: PathBuf,
    effective_config_handle: Option<File>,
    effective_config_secret_present: bool,
    cleanup_on_drop: bool,
}

impl PrivateDumpStage {
    fn create(target_parent: &DirectoryAnchor) -> Result<Self, String> {
        let parent_anchor = target_parent.try_clone()?;
        let root_name = OsString::from(format!(".unica-dump-guard-{}", Uuid::new_v4()));
        let root = target_parent.path.join(&root_name);
        run_private_create_hook(&root);
        let root_anchor = parent_anchor.create_child(&root_name, &root)?;
        let execution = root.join("execution");
        let recovery = root.join("recovery");
        let execution_anchor = match root_anchor.create_child(OsStr::new("execution"), &execution) {
            Ok(anchor) => anchor,
            Err(error) => {
                return Err(private_creation_error(
                    &parent_anchor,
                    &root_anchor,
                    &root_name,
                    &root,
                    error,
                ));
            }
        };
        let recovery_anchor = match root_anchor.create_child(OsStr::new("recovery"), &recovery) {
            Ok(anchor) => anchor,
            Err(error) => {
                return Err(private_creation_error(
                    &parent_anchor,
                    &root_anchor,
                    &root_name,
                    &root,
                    error,
                ));
            }
        };
        if let Err(error) = parent_anchor.verify_path_binding() {
            return Err(private_creation_error(
                &parent_anchor,
                &root_anchor,
                &root_name,
                &root,
                error,
            ));
        };
        Ok(Self {
            staged_tree: execution.join("staged-source"),
            effective_config: execution.join(EFFECTIVE_CONFIG_NAME),
            parent_anchor,
            root_anchor,
            root_name,
            root,
            execution,
            recovery,
            execution_anchor,
            recovery_anchor,
            effective_config_handle: None,
            effective_config_secret_present: false,
            cleanup_on_drop: true,
        })
    }

    fn write_effective_config(&mut self, value: &YamlValue) -> Result<(), String> {
        let bytes = serde_yaml::to_string(value)
            .map_err(|error| format!("failed to serialize effective dump config: {error}"))?;
        self.execution_anchor.verify_path_binding()?;
        let file = create_regular_child_owner_only(
            &self.execution_anchor.directory,
            OsStr::new(EFFECTIVE_CONFIG_NAME),
        )
        .map_err(|error| {
            format!(
                "failed to securely create private effective config {}: {error}",
                self.effective_config.display()
            )
        })?;
        restrict_stage_to_owner(&file).map_err(|error| {
            format!(
                "failed to restrict private effective config {}: {error}",
                self.effective_config.display()
            )
        })?;
        self.effective_config_handle = Some(file);
        self.effective_config_secret_present = true;
        self.effective_config_handle
            .as_mut()
            .expect("effective config handle was just installed")
            .write_all(bytes.as_bytes())
            .map_err(|error| {
                format!(
                    "failed to write private effective config {}: {error}",
                    self.effective_config.display()
                )
            })?;
        self.effective_config_handle
            .as_ref()
            .expect("effective config handle remains installed")
            .sync_all()
            .map_err(|error| {
                format!(
                    "failed to sync private effective config {}: {error}",
                    self.effective_config.display()
                )
            })?;
        self.execution_anchor
            .verify_path_binding()
            .map_err(|error| {
                format!(
                    "private effective config directory changed after secure creation {}: {error}",
                    self.effective_config.display()
                )
            })?;
        Ok(())
    }

    fn remove_effective_config(&mut self) -> Result<(), String> {
        let Some(file) = self.effective_config_handle.as_ref() else {
            return Ok(());
        };
        let identity = file_identity(file).map_err(|error| {
            format!(
                "private effective config identity check failed for {}: {error}; secret-bearing private data may remain",
                self.effective_config.display()
            )
        })?;
        file.set_len(0).map_err(|error| {
            format!(
                "private effective config scrubbing failed for {}: {error}; secret-bearing private data may remain",
                self.effective_config.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "private effective config scrub sync failed for {}: {error}; secret-bearing private data may remain",
                self.effective_config.display()
            )
        })?;
        self.effective_config_secret_present = false;
        let unlink = unlink_bound_regular_child(
            &self.execution_anchor.directory,
            OsStr::new(EFFECTIVE_CONFIG_NAME),
            identity,
        )
        .map_err(|error| {
            format!(
                "private effective config cleanup failed for {}: {error}",
                self.effective_config.display()
            )
        });
        self.effective_config_handle.take();
        unlink
    }

    fn preserve_for_recovery(&mut self) -> Result<(), String> {
        let mut errors = Vec::new();
        if let Err(error) = self.remove_effective_config() {
            errors.push(error);
        }
        if let Err(error) = remove_bound_directory_child(
            &self.root_anchor.directory,
            OsStr::new("execution"),
            self.execution_anchor.identity,
            &self.execution,
        ) {
            errors.push(format!(
                "private execution cleanup failed before retaining recovery {}: {error}",
                self.execution.display()
            ));
        }
        if let Err(error) = self.root_anchor.verify_path_binding() {
            errors.push(format!(
                "private recovery root is no longer bound to its visible path: {error}"
            ));
        }
        if let Err(error) = self.recovery_anchor.verify_path_binding() {
            errors.push(format!(
                "private recovery directory is no longer bound to its visible path: {error}"
            ));
        }
        if !errors.is_empty() {
            return Err(errors.join("; "));
        }
        self.cleanup_on_drop = false;
        Ok(())
    }

    fn cleanup_now(&mut self) -> Result<(), String> {
        if !self.cleanup_on_drop {
            return Ok(());
        }
        let mut errors = Vec::new();
        if let Err(error) = self.remove_effective_config() {
            errors.push(error);
        }
        if let Err(error) =
            remove_directory_contents_nofollow(&self.root_anchor.directory, &self.root)
        {
            errors.push(format!(
                "private dump contents cleanup failed for {}: {error}",
                self.root.display()
            ));
        }
        if let Err(error) = remove_bound_directory_child(
            &self.parent_anchor.directory,
            &self.root_name,
            self.root_anchor.identity,
            &self.root,
        ) {
            errors.push(format!(
                "private dump root cleanup failed for {}: {error}",
                self.root.display()
            ));
        }
        if errors.is_empty() {
            self.cleanup_on_drop = false;
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

fn private_creation_error(
    parent_anchor: &DirectoryAnchor,
    root_anchor: &DirectoryAnchor,
    root_name: &OsStr,
    root: &Path,
    primary: String,
) -> String {
    let contents_cleanup = remove_directory_contents_nofollow(&root_anchor.directory, root);
    let root_cleanup = remove_bound_directory_child(
        &parent_anchor.directory,
        root_name,
        root_anchor.identity,
        root,
    );
    let mut errors = Vec::new();
    if let Err(error) = contents_cleanup {
        errors.push(format!(
            "private dump contents cleanup failed for {}: {error}",
            root.display()
        ));
    }
    if let Err(error) = root_cleanup {
        errors.push(format!(
            "private dump root cleanup failed for {}: {error}",
            root.display()
        ));
    }
    if errors.is_empty() {
        primary
    } else {
        format!("{primary}; {}", errors.join("; "))
    }
}

impl Drop for PrivateDumpStage {
    fn drop(&mut self) {
        if self.cleanup_on_drop {
            let _ = self.cleanup_now();
        }
    }
}

#[cfg(unix)]
fn unlink_bound_regular_child(
    parent: &File,
    name: &OsStr,
    expected_identity: FileIdentity,
) -> Result<(), String> {
    let rebound = open_regular_child_nofollow(parent, name)
        .map_err(|error| format!("failed to rebind the private file before cleanup: {error}"))?;
    let actual_identity = file_identity(&rebound)
        .map_err(|error| format!("failed to inspect the private file before cleanup: {error}"))?;
    if actual_identity != expected_identity {
        return Err(
            "private file identity changed before cleanup; the replacement was left untouched"
                .to_string(),
        );
    }
    unlink_non_directory_child_at(parent, name)
        .map_err(|error| format!("secure unlinkat failed: {error}"))
}

#[cfg(not(unix))]
fn unlink_bound_regular_child(
    _parent: &File,
    _name: &OsStr,
    _expected_identity: FileIdentity,
) -> Result<(), String> {
    Err("secure descriptor-relative file cleanup is unavailable on this host".to_string())
}

#[cfg(unix)]
fn remove_directory_contents_nofollow(directory: &File, display_path: &Path) -> Result<(), String> {
    for name in read_directory_names(directory).map_err(|error| {
        format!(
            "failed to enumerate the retained private directory anchor {}: {error}",
            display_path.display()
        )
    })? {
        let child_path = display_path.join(&name);
        match open_directory_child_nofollow(directory, &name) {
            Ok(child) => {
                let expected_identity = file_identity(&child).map_err(|error| {
                    format!(
                        "failed to inspect private directory {} before cleanup: {error}",
                        child_path.display()
                    )
                })?;
                remove_directory_contents_nofollow(&child, &child_path)?;
                let rebound = open_directory_child_nofollow(directory, &name).map_err(|error| {
                    format!(
                        "failed to rebind private directory {} before cleanup: {error}",
                        child_path.display()
                    )
                })?;
                let actual_identity = file_identity(&rebound).map_err(|error| {
                    format!(
                        "failed to recheck private directory {} before cleanup: {error}",
                        child_path.display()
                    )
                })?;
                if actual_identity != expected_identity {
                    return Err(format!(
                        "private directory identity changed before cleanup; replacement left untouched: {}",
                        child_path.display()
                    ));
                }
                unlink_directory_child_at(directory, &name).map_err(|error| {
                    format!(
                        "failed to remove private directory {} through its retained parent anchor: {error}",
                        child_path.display()
                    )
                })?;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(libc::ENOTDIR) | Some(libc::EINVAL) | Some(libc::ELOOP)
                ) =>
            {
                unlink_non_directory_child_at(directory, &name).map_err(|unlink_error| {
                    format!(
                        "failed to remove private non-directory entry {} through its retained parent anchor: {unlink_error}",
                        child_path.display()
                    )
                })?;
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect private entry {} before cleanup: {error}",
                    child_path.display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn remove_directory_contents_nofollow(
    _directory: &File,
    display_path: &Path,
) -> Result<(), String> {
    Err(format!(
        "secure descriptor-relative private cleanup is unavailable on this host: {}",
        display_path.display()
    ))
}

#[cfg(unix)]
fn remove_bound_directory_child(
    parent: &File,
    name: &OsStr,
    expected_identity: FileIdentity,
    display_path: &Path,
) -> Result<(), String> {
    let child = open_directory_child_nofollow(parent, name).map_err(|error| {
        format!(
            "failed to rebind expected private directory {} before cleanup: {error}",
            display_path.display()
        )
    })?;
    let actual_identity = file_identity(&child).map_err(|error| {
        format!(
            "failed to inspect expected private directory {} before cleanup: {error}",
            display_path.display()
        )
    })?;
    if actual_identity != expected_identity {
        return Err(format!(
            "private directory identity changed before cleanup; replacement left untouched: {}",
            display_path.display()
        ));
    }
    remove_directory_contents_nofollow(&child, display_path)?;
    let rebound = open_directory_child_nofollow(parent, name).map_err(|error| {
        format!(
            "failed to rebind expected private directory {} after cleanup: {error}",
            display_path.display()
        )
    })?;
    if file_identity(&rebound).map_err(|error| {
        format!(
            "failed to recheck expected private directory {} after cleanup: {error}",
            display_path.display()
        )
    })? != expected_identity
    {
        return Err(format!(
            "private directory identity changed after cleanup; replacement left untouched: {}",
            display_path.display()
        ));
    }
    unlink_directory_child_at(parent, name).map_err(|error| {
        format!(
            "failed to remove expected private directory {} through its retained parent anchor: {error}",
            display_path.display()
        )
    })
}

#[cfg(not(unix))]
fn remove_bound_directory_child(
    _parent: &File,
    _name: &OsStr,
    _expected_identity: FileIdentity,
    display_path: &Path,
) -> Result<(), String> {
    Err(format!(
        "secure descriptor-relative private cleanup is unavailable on this host: {}",
        display_path.display()
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TreeSnapshot {
    Absent,
    Directory {
        identity: FileIdentity,
        entries: Vec<TreeEntrySnapshot>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TreeEntrySnapshot {
    relative_path: PathBuf,
    kind: TreeEntryKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TreeEntryKind {
    Directory {
        identity: FileIdentity,
    },
    File {
        identity: FileIdentity,
        size: u64,
        sha256: [u8; 32],
    },
}

impl TreeSnapshot {
    fn capture_target(path: &Path) -> Result<Self, String> {
        capture_tree_target_nofollow(path)
    }

    fn root_identity(&self) -> Option<FileIdentity> {
        match self {
            Self::Absent => None,
            Self::Directory { identity, .. } => Some(*identity),
        }
    }
}

#[cfg(test)]
fn capture_directory_snapshot(root: &Path) -> Result<Vec<TreeEntrySnapshot>, String> {
    let normalized = normalize_leaf_path(root)?;
    let (parent_path, name) = split_parent_and_name(&normalized)?;
    #[cfg(not(unix))]
    {
        let _ = (parent_path, name);
        Err(format!(
            "secure no-follow directory snapshots are unavailable on this host: {}",
            root.display()
        ))
    }
    #[cfg(unix)]
    {
        let directory = {
            let parent = open_directory_nofollow(&parent_path).map_err(|error| {
                format!(
                    "failed to securely open directory parent {}: {error}",
                    parent_path.display()
                )
            })?;
            open_directory_child_nofollow(&parent, &name).map_err(|error| {
                format!(
                    "failed to securely open directory {}: {error}",
                    root.display()
                )
            })?
        };
        let mut entries = Vec::new();
        capture_directory_snapshot_recursive(&directory, Path::new(""), root, &mut entries)?;
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        Ok(entries)
    }
}

#[cfg(unix)]
fn capture_tree_target_nofollow(path: &Path) -> Result<TreeSnapshot, String> {
    let normalized = normalize_leaf_path(path)?;
    let (parent_path, name) = split_parent_and_name(&normalized)?;
    let parent = open_directory_nofollow(&parent_path).map_err(|error| {
        format!(
            "failed to securely open dump target parent {}: {error}",
            parent_path.display()
        )
    })?;
    run_tree_open_hook(path);
    capture_tree_child_nofollow(&parent, &name, path)
}

#[cfg(unix)]
fn capture_tree_child_nofollow(
    parent: &File,
    name: &OsStr,
    display_path: &Path,
) -> Result<TreeSnapshot, String> {
    let directory = match open_directory_child_nofollow(parent, name) {
        Ok(directory) => directory,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(TreeSnapshot::Absent),
        Err(error) => {
            let detail = if matches!(
                error.raw_os_error(),
                Some(libc::ELOOP) | Some(libc::ENOTDIR)
            ) {
                "symbolic link, reparse point, or non-directory entry".to_string()
            } else {
                error.to_string()
            };
            return Err(format!(
                "dump target must be a real directory or absent {}: {detail}",
                display_path.display()
            ));
        }
    };
    let identity = file_identity(&directory).map_err(|error| {
        format!(
            "failed to inspect dump target directory identity {}: {error}",
            display_path.display()
        )
    })?;
    let mut entries = Vec::new();
    capture_directory_snapshot_recursive(&directory, Path::new(""), display_path, &mut entries)?;
    let rebound = open_directory_child_nofollow(parent, name).map_err(|error| {
        format!(
            "failed to rebind dump target directory {}: {error}",
            display_path.display()
        )
    })?;
    if file_identity(&rebound).map_err(|error| {
        format!(
            "failed to recheck dump target identity {}: {error}",
            display_path.display()
        )
    })? != identity
    {
        return Err(format!(
            "dump target directory identity changed while snapshotting: {}",
            display_path.display()
        ));
    }
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(TreeSnapshot::Directory { identity, entries })
}

#[cfg(not(unix))]
fn capture_tree_target_nofollow(path: &Path) -> Result<TreeSnapshot, String> {
    Err(format!(
        "secure no-follow directory snapshots are unavailable on this host: {}",
        path.display()
    ))
}

#[cfg(not(unix))]
fn capture_tree_child_nofollow(
    _parent: &File,
    _name: &OsStr,
    display_path: &Path,
) -> Result<TreeSnapshot, String> {
    Err(format!(
        "secure no-follow directory snapshots are unavailable on this host: {}",
        display_path.display()
    ))
}

#[cfg(unix)]
fn capture_directory_snapshot_recursive(
    directory: &File,
    relative_root: &Path,
    display_root: &Path,
    entries: &mut Vec<TreeEntrySnapshot>,
) -> Result<(), String> {
    for name in read_directory_names(directory).map_err(|error| {
        format!(
            "failed to securely enumerate {}: {error}",
            display_root.join(relative_root).display()
        )
    })? {
        let relative = relative_root.join(&name);
        let display_path = display_root.join(&relative);
        match open_directory_child_nofollow(directory, &name) {
            Ok(child) => {
                let identity = file_identity(&child).map_err(|error| {
                    format!(
                        "failed to inspect directory identity {}: {error}",
                        display_path.display()
                    )
                })?;
                entries.push(TreeEntrySnapshot {
                    relative_path: relative.clone(),
                    kind: TreeEntryKind::Directory { identity },
                });
                capture_directory_snapshot_recursive(&child, &relative, display_root, entries)?;
                let rebound = open_directory_child_nofollow(directory, &name).map_err(|error| {
                    format!(
                        "failed to rebind directory {}: {error}",
                        display_path.display()
                    )
                })?;
                if file_identity(&rebound).map_err(|error| {
                    format!(
                        "failed to recheck directory identity {}: {error}",
                        display_path.display()
                    )
                })? != identity
                {
                    return Err(format!(
                        "dump tree directory identity changed while snapshotting: {}",
                        display_path.display()
                    ));
                }
            }
            Err(directory_error)
                if matches!(
                    directory_error.raw_os_error(),
                    Some(libc::ENOTDIR) | Some(libc::EINVAL)
                ) =>
            {
                let mut file = open_regular_child_nofollow(directory, &name).map_err(|error| {
                    let detail = if error.raw_os_error() == Some(libc::ELOOP) {
                        "symbolic link or reparse point".to_string()
                    } else {
                        error.to_string()
                    };
                    format!(
                        "dump tree contains an unsupported entry {}: {detail}",
                        display_path.display()
                    )
                })?;
                let identity = file_identity(&file).map_err(|error| {
                    format!(
                        "failed to inspect file identity {}: {error}",
                        display_path.display()
                    )
                })?;
                let links = hard_link_count(&file).map_err(|error| {
                    format!(
                        "failed to inspect hard links for {}: {error}",
                        display_path.display()
                    )
                })?;
                if links != 1 {
                    return Err(format!(
                        "dump tree file must have exactly one hard link: {}",
                        display_path.display()
                    ));
                }
                let before = file.metadata().map_err(|error| {
                    format!(
                        "failed to inspect opened file {}: {error}",
                        display_path.display()
                    )
                })?;
                let mut hasher = Sha256::new();
                let mut size = 0_u64;
                let mut buffer = [0_u8; 64 * 1024];
                loop {
                    let read = file.read(&mut buffer).map_err(|error| {
                        format!("failed to read {}: {error}", display_path.display())
                    })?;
                    if read == 0 {
                        break;
                    }
                    size += read as u64;
                    hasher.update(&buffer[..read]);
                }
                let after = file.metadata().map_err(|error| {
                    format!(
                        "failed to recheck opened file {}: {error}",
                        display_path.display()
                    )
                })?;
                if before.len() != after.len()
                    || before.modified().ok() != after.modified().ok()
                    || after.len() != size
                {
                    return Err(format!(
                        "dump tree file changed while snapshotting: {}",
                        display_path.display()
                    ));
                }
                let rebound = open_regular_child_nofollow(directory, &name).map_err(|error| {
                    format!("failed to rebind file {}: {error}", display_path.display())
                })?;
                if file_identity(&rebound).map_err(|error| {
                    format!(
                        "failed to recheck file identity {}: {error}",
                        display_path.display()
                    )
                })? != identity
                {
                    return Err(format!(
                        "dump tree file identity changed while snapshotting: {}",
                        display_path.display()
                    ));
                }
                entries.push(TreeEntrySnapshot {
                    relative_path: relative,
                    kind: TreeEntryKind::File {
                        identity,
                        size,
                        sha256: hasher.finalize().into(),
                    },
                });
            }
            Err(error) => {
                let detail = if error.raw_os_error() == Some(libc::ELOOP) {
                    "symbolic link or reparse point".to_string()
                } else {
                    error.to_string()
                };
                return Err(format!(
                    "dump tree contains an unsupported entry {}: {detail}",
                    display_path.display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn read_directory_names(directory: &File) -> std::io::Result<Vec<OsString>> {
    // SAFETY: fcntl duplicates the live descriptor; fdopendir assumes ownership of that duplicate.
    let duplicate = unsafe { libc::fcntl(directory.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    if duplicate < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: duplicate is a valid owned directory descriptor.
    let stream = unsafe { libc::fdopendir(duplicate) };
    if stream.is_null() {
        let error = std::io::Error::last_os_error();
        // SAFETY: fdopendir failed and did not take ownership of duplicate.
        unsafe {
            libc::close(duplicate);
        }
        return Err(error);
    }
    let mut names = Vec::new();
    loop {
        // SAFETY: stream remains valid until closed below.
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            break;
        }
        // SAFETY: d_name is a NUL-terminated array owned by the live dirent.
        let bytes = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        names.push(OsString::from_vec(bytes.to_vec()));
    }
    // SAFETY: stream is live and owns duplicate.
    if unsafe { libc::closedir(stream) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    names.sort();
    Ok(names)
}

#[cfg(not(unix))]
fn capture_directory_snapshot_recursive(
    _directory: &File,
    _relative_root: &Path,
    display_root: &Path,
    _entries: &mut Vec<TreeEntrySnapshot>,
) -> Result<(), String> {
    Err(format!(
        "secure no-follow directory snapshots are unavailable on this host: {}",
        display_root.display()
    ))
}

fn validate_staged_dump(root: &Path, kind: SourceSetKind) -> Result<TreeSnapshot, String> {
    let snapshot = TreeSnapshot::capture_target(root)?;
    let TreeSnapshot::Directory { entries, .. } = &snapshot else {
        return Err(format!(
            "v8-runner did not create the private staged dump directory: {}",
            root.display()
        ));
    };
    if entries.is_empty() {
        return Err(format!("private staged dump is empty: {}", root.display()));
    }
    let owner_path = root.join("Configuration.xml");
    let owner_entry = entries
        .iter()
        .find(|entry| entry.relative_path == Path::new("Configuration.xml"))
        .ok_or_else(|| {
            format!(
                "private staged dump has no Configuration.xml owner: {}",
                root.display()
            )
        })?;
    validate_staged_required_owner(&owner_path, kind, &owner_entry.kind)?;

    for entry in entries {
        let TreeEntryKind::File { .. } = entry.kind else {
            continue;
        };
        if !entry
            .relative_path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
        {
            continue;
        }
        validate_staged_xml(&root.join(&entry.relative_path), &entry.kind)?;
    }
    Ok(snapshot)
}

fn validate_required_owner(path: &Path, kind: SourceSetKind) -> Result<(), String> {
    let raw = secure_read_regular_file(path, "staged source-set owner")?;
    validate_required_owner_raw(path, kind, &raw)
}

fn validate_staged_required_owner(
    path: &Path,
    kind: SourceSetKind,
    expected: &TreeEntryKind,
) -> Result<(), String> {
    let raw = read_tree_bound_file(path, "staged source-set owner", expected)?;
    validate_required_owner_raw(path, kind, &raw)
}

fn validate_required_owner_raw(path: &Path, kind: SourceSetKind, raw: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(raw)
        .map_err(|error| format!("staged source-set owner is not UTF-8: {error}"))?;
    let source = text.trim_start_matches('\u{feff}');
    let document = Document::parse(source)
        .map_err(|error| format!("failed to parse staged owner {}: {error}", path.display()))?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        return Err(format!(
            "staged owner must be {{{MD_CLASSES_NS}}}MetaDataObject: {}",
            path.display()
        ));
    }
    let version = root_version_literal(source, root);
    if version.as_deref() != Some(TARGET_EXPORT_FORMAT) {
        return Err(format!(
            "staged owner export format must be the exact raw literal {TARGET_EXPORT_FORMAT}; found {} in {}",
            version.as_deref().unwrap_or("<missing>"),
            path.display()
        ));
    }
    let children = root
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    if children.len() != 1
        || children[0].tag_name().namespace() != Some(MD_CLASSES_NS)
        || children[0].tag_name().name() != "Configuration"
    {
        return Err(format!(
            "staged owner must contain exactly one direct {{{MD_CLASSES_NS}}}Configuration child: {}",
            path.display()
        ));
    }
    let is_extension = is_configuration_extension(children[0]);
    match (kind, is_extension) {
        (SourceSetKind::Configuration, false) | (SourceSetKind::Extension, true) => Ok(()),
        (SourceSetKind::Configuration, true) => Err(format!(
            "CONFIGURATION source-set produced an extension owner: {}",
            path.display()
        )),
        (SourceSetKind::Extension, false) => Err(format!(
            "EXTENSION source-set produced a configuration owner without ConfigurationExtensionPurpose: {}",
            path.display()
        )),
        _ => Err("external source-set kind reached guarded dump validation".to_string()),
    }
}

fn is_configuration_extension(configuration: roxmltree::Node<'_, '_>) -> bool {
    configuration
        .children()
        .find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                && node.tag_name().name() == "Properties"
        })
        .is_some_and(|properties| {
            properties.children().any(|node| {
                node.is_element()
                    && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                    && node.tag_name().name() == "ConfigurationExtensionPurpose"
            })
        })
}

fn validate_staged_xml(path: &Path, expected: &TreeEntryKind) -> Result<(), String> {
    let raw = read_tree_bound_file(path, "staged XML", expected)?;
    let text = std::str::from_utf8(&raw)
        .map_err(|error| format!("staged XML is not UTF-8 {}: {error}", path.display()))?;
    let source = text.trim_start_matches('\u{feff}');
    let document = Document::parse(source)
        .map_err(|error| format!("failed to parse staged XML {}: {error}", path.display()))?;
    let root = document.root_element();
    let namespace = root.tag_name().namespace().unwrap_or("");
    let local_name = root.tag_name().name();
    match staged_root_version_policy(namespace, local_name) {
        Some(StagedRootVersionPolicy::ExactRootVersion) => {
            let version = root_version_literal(source, root);
            if version.as_deref() != Some(TARGET_EXPORT_FORMAT) {
                return Err(format!(
                    "staged XML root {{{namespace}}}{local_name} must use the exact raw version literal {TARGET_EXPORT_FORMAT}; found {} (missing means legacy format 1.0) in {}",
                    version.as_deref().unwrap_or("<missing>"),
                    path.display()
                ));
            }
        }
        Some(StagedRootVersionPolicy::Versionless) => {
            if let Some(version) = root_version_literal(source, root) {
                return Err(format!(
                    "staged XML root {{{namespace}}}{local_name} is a registered versionless family and must not declare version={version} in {}",
                    path.display()
                ));
            }
        }
        None => {
            return Err(format!(
                "staged XML root {{{namespace}}}{local_name} is unsupported by the closed platform {TARGET_PLATFORM_LINE} / export format {TARGET_EXPORT_FORMAT} registry: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn read_tree_bound_file(
    path: &Path,
    role: &str,
    expected: &TreeEntryKind,
) -> Result<Vec<u8>, String> {
    let TreeEntryKind::File {
        identity,
        size,
        sha256,
    } = expected
    else {
        return Err(format!(
            "{role} is not a file in the captured tree snapshot: {}",
            path.display()
        ));
    };
    let actual = secure_read_regular_file_snapshot(path, role)?;
    let actual_size = actual.raw.len() as u64;
    let actual_sha256: [u8; 32] = Sha256::digest(&actual.raw).into();
    if actual.identity != *identity || actual_size != *size || actual_sha256 != *sha256 {
        return Err(format!(
            "{role} bytes or identity changed from the captured tree snapshot: {}",
            path.display()
        ));
    }
    Ok(actual.raw)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StagedRootVersionPolicy {
    ExactRootVersion,
    Versionless,
}

fn staged_root_version_policy(
    namespace: &str,
    local_name: &str,
) -> Option<StagedRootVersionPolicy> {
    match (namespace, local_name) {
        (MD_CLASSES_NS, "MetaDataObject")
        | ("http://v8.1c.ru/8.3/xcf/logform", "Form")
        | ("http://v8.1c.ru/8.3/xcf/extrnprops", "CommandInterface")
        | ("http://v8.1c.ru/8.3/xcf/extrnprops", "Help")
        | ("http://v8.1c.ru/8.3/xcf/extrnprops", "ExchangePlanContent")
        | ("http://v8.1c.ru/8.3/xcf/extrnprops", "HomePageWorkArea")
        | ("http://v8.1c.ru/8.3/xcf/scheme", "GraphicalSchema")
        | ("http://v8.1c.ru/8.2/roles", "Rights") => {
            Some(StagedRootVersionPolicy::ExactRootVersion)
        }
        ("http://v8.1c.ru/8.1/data-composition-system/schema", "DataCompositionSchema")
        | ("http://v8.1c.ru/8.2/data/spreadsheet", "document")
        | ("http://v8.1c.ru/8.2/managed-application/core", "ClientApplicationInterface") => {
            Some(StagedRootVersionPolicy::Versionless)
        }
        _ => None,
    }
}

fn publish_staged_tree(
    private: &mut PrivateDumpStage,
    target: &Path,
    target_parent: &DirectoryAnchor,
    original: &TreeSnapshot,
    staged: &TreeSnapshot,
) -> Result<Vec<String>, String> {
    let target_name = target
        .file_name()
        .ok_or_else(|| format!("dump target has no name: {}", target.display()))?;
    let sealed_name = OsString::from(format!("sealed-stage-{}", Uuid::new_v4()));
    let backup_name = OsString::from(format!("target-backup-{}", Uuid::new_v4()));
    let failed_name = OsString::from(format!("failed-publication-{}", Uuid::new_v4()));
    let sealed = private.recovery.join(&sealed_name);
    let backup = private.recovery.join(&backup_name);
    let failed = private.recovery.join(&failed_name);
    let had_target = matches!(original, TreeSnapshot::Directory { .. });
    let staged_identity = staged.root_identity().ok_or_else(|| {
        format!(
            "validated staged dump has no directory identity: {}",
            private.staged_tree.display()
        )
    })?;

    run_publication_failpoint(PublicationCheckpoint::BeforeStageInstall)?;
    rename_prechecked_directory_child_no_replace(
        &private.execution_anchor,
        OsStr::new("staged-source"),
        staged_identity,
        &private.recovery_anchor,
        &sealed_name,
    )
    .map_err(|error| {
        format!(
            "failed to atomically seal the validated staged dump {} in {}: {error}",
            private.staged_tree.display(),
            sealed.display()
        )
    })?;
    let sealed_snapshot = private
        .recovery_anchor
        .capture_child(&sealed_name, &sealed)?;
    if &sealed_snapshot != staged {
        return Err(format!(
            "private staged dump changed while it was being sealed; Git-visible sources were not touched: {}",
            private.staged_tree.display()
        ));
    }

    if had_target {
        rename_child_no_replace(
            target_parent,
            target_name,
            &private.recovery_anchor,
            &backup_name,
        )
        .map_err(|error| {
            format!(
                "failed to atomically move dump target {} to private recovery {} without clobbering: {error}",
                target.display(),
                backup.display()
            )
        })?;
        if let Err(error) = target_parent.verify_path_binding() {
            return Err(retain_recovery_error(
                private,
                format!("dump target parent changed after the atomic backup move: {error}"),
            ));
        }
        let moved = match private.recovery_anchor.capture_child(&backup_name, &backup) {
            Ok(moved) => moved,
            Err(error) => {
                return Err(retain_recovery_error(
                    private,
                    format!("could not verify the atomic target backup: {error}"),
                ));
            }
        };
        if &moved != original {
            let primary = format!(
                "dump target changed during the atomic backup move; staged output was not published: {}",
                target.display()
            );
            return rollback_before_stage_install(
                private,
                target,
                target_parent,
                target_name,
                &backup_name,
                &moved,
                primary,
            );
        }
        if let Err(error) = run_publication_failpoint(PublicationCheckpoint::AfterBackup) {
            return rollback_before_stage_install(
                private,
                target,
                target_parent,
                target_name,
                &backup_name,
                original,
                error,
            );
        }
    }
    if let Err(error) = run_publication_failpoint(PublicationCheckpoint::BeforeSealedPublishRename)
    {
        if had_target {
            return rollback_before_stage_install(
                private,
                target,
                target_parent,
                target_name,
                &backup_name,
                original,
                error,
            );
        }
        return Err(error);
    }
    // No supported host offers an identity-conditioned source rename. Treat the
    // move plus the immediate snapshot below as one publication boundary: a
    // name-swapped source is atomically removed from the Git-visible target into
    // private quarantine before rollback or lock release.
    if let Err(error) = rename_child_no_replace(
        &private.recovery_anchor,
        &sealed_name,
        target_parent,
        target_name,
    ) {
        let primary = format!(
            "failed to atomically publish validated staged dump {} to {} without clobbering: {error}",
            sealed.display(),
            target.display()
        );
        if had_target {
            return rollback_before_stage_install(
                private,
                target,
                target_parent,
                target_name,
                &backup_name,
                original,
                primary,
            );
        }
        return Err(primary);
    }

    let installed = match target_parent.capture_child(target_name, target) {
        Ok(installed) => installed,
        Err(error) => {
            let primary = format!(
                "could not validate the just-installed staged target before commit: {error}"
            );
            if let Err(quarantine_error) = quarantine_unverified_install(
                private,
                target,
                target_parent,
                target_name,
                &failed_name,
                &failed,
                None,
            ) {
                return Err(retain_recovery_error(
                    private,
                    format!("{primary}; {quarantine_error}"),
                ));
            }
            if had_target {
                return rollback_before_stage_install(
                    private,
                    target,
                    target_parent,
                    target_name,
                    &backup_name,
                    original,
                    primary,
                );
            }
            return Err(primary);
        }
    };
    if &installed != staged {
        let primary = format!(
            "sealed staged source identity changed at the atomic publication boundary; unverified tree was not committed: {}",
            target.display()
        );
        if let Err(error) = quarantine_unverified_install(
            private,
            target,
            target_parent,
            target_name,
            &failed_name,
            &failed,
            Some(&installed),
        ) {
            return Err(retain_recovery_error(
                private,
                format!("{primary}; {error}"),
            ));
        }
        if had_target {
            return rollback_before_stage_install(
                private,
                target,
                target_parent,
                target_name,
                &backup_name,
                original,
                primary,
            );
        }
        return Err(primary);
    }

    let checkpoint_error =
        run_publication_failpoint(PublicationCheckpoint::AfterStageInstall).err();
    let published = match target_parent.capture_child(target_name, target) {
        Ok(published) => published,
        Err(capture_error) => {
            let error = match checkpoint_error.as_deref() {
                Some(checkpoint_error) => format!(
                    "{checkpoint_error}; could not verify the published target: {capture_error}"
                ),
                None => format!("could not verify the published target: {capture_error}"),
            };
            match target_parent.capture_child_root_identity(target_name, target) {
                Ok(Some(identity)) if identity == staged_identity => {
                    return quarantine_unverified_and_rollback(
                        private,
                        target,
                        target_parent,
                        target_name,
                        &failed_name,
                        &failed,
                        None,
                        had_target,
                        &backup_name,
                        original,
                        error,
                    );
                }
                Ok(None) if had_target => {
                    return rollback_before_stage_install(
                        private,
                        target,
                        target_parent,
                        target_name,
                        &backup_name,
                        original,
                        error,
                    );
                }
                Ok(None) => return Err(error),
                Ok(Some(_)) if had_target => {
                    return Err(retain_recovery_error(
                        private,
                        format!(
                            "{error}; a concurrent target with a different root identity now occupies {} and was left untouched",
                            target.display()
                        ),
                    ));
                }
                Ok(Some(_)) => {
                    return Err(format!(
                        "{error}; a concurrent target with a different root identity now occupies {} and was left untouched",
                        target.display()
                    ));
                }
                Err(identity_error) if had_target => {
                    return Err(retain_recovery_error(
                        private,
                        format!(
                            "{error}; published target ownership could not be classified: {identity_error}"
                        ),
                    ));
                }
                Err(identity_error) => {
                    return Err(format!(
                        "{error}; published target ownership could not be classified: {identity_error}"
                    ));
                }
            }
        }
    };
    let validation_error = (&published != staged).then(|| {
        format!(
            "published dump differs from the validated staged snapshot: {}",
            target.display()
        )
    });
    if let Some(error) = checkpoint_error.or(validation_error) {
        if &published == staged {
            if let Err(move_error) = rename_child_no_replace(
                target_parent,
                target_name,
                &private.recovery_anchor,
                &failed_name,
            ) {
                return Err(retain_recovery_error(
                    private,
                    format!(
                        "{error}; rollback could not atomically move failed publication {} to {} without clobbering: {move_error}",
                        target.display(),
                        failed.display()
                    ),
                ));
            }
            let failed_snapshot = match private.recovery_anchor.capture_child(&failed_name, &failed)
            {
                Ok(snapshot) => snapshot,
                Err(snapshot_error) => {
                    return Err(retain_recovery_error(
                        private,
                        format!(
                            "{error}; could not verify failed-publication recovery {}: {snapshot_error}",
                            failed.display()
                        ),
                    ));
                }
            };
            if &failed_snapshot != staged {
                return Err(retain_recovery_error(
                    private,
                    format!(
                        "{error}; failed-publication recovery differs from the validated staged snapshot: {}",
                        failed.display()
                    ),
                ));
            }
            if had_target {
                return rollback_before_stage_install(
                    private,
                    target,
                    target_parent,
                    target_name,
                    &backup_name,
                    original,
                    error,
                );
            }
            return Err(error);
        }

        if published.root_identity() == Some(staged_identity) {
            return quarantine_unverified_and_rollback(
                private,
                target,
                target_parent,
                target_name,
                &failed_name,
                &failed,
                Some(&published),
                had_target,
                &backup_name,
                original,
                error,
            );
        }

        if matches!(published, TreeSnapshot::Absent) && had_target {
            return rollback_before_stage_install(
                private,
                target,
                target_parent,
                target_name,
                &backup_name,
                original,
                error,
            );
        }
        if matches!(published, TreeSnapshot::Absent) {
            return Err(error);
        }

        if had_target {
            return Err(retain_recovery_error(
                private,
                format!(
                    "{error}; a concurrent target now occupies {} and was left untouched",
                    target.display()
                ),
            ));
        }
        return Err(format!(
            "{error}; a concurrent target now occupies {} and was left untouched",
            target.display()
        ));
    }

    if let Err(error) = target_parent.verify_path_binding() {
        if had_target {
            return Err(retain_recovery_error(
                private,
                format!("dump target parent changed after publication verification: {error}"),
            ));
        }
        return Err(error);
    }
    Ok(Vec::new())
}

fn quarantine_unverified_install(
    private: &PrivateDumpStage,
    target: &Path,
    target_parent: &DirectoryAnchor,
    target_name: &OsStr,
    quarantine_name: &OsStr,
    quarantine_path: &Path,
    installed: Option<&TreeSnapshot>,
) -> Result<(), String> {
    rename_child_no_replace(
        target_parent,
        target_name,
        &private.recovery_anchor,
        quarantine_name,
    )
    .map_err(|error| {
        format!(
            "could not atomically quarantine unverified publication {} at {}: {error}",
            target.display(),
            quarantine_path.display()
        )
    })?;
    if let Some(installed) = installed {
        let quarantined = private
            .recovery_anchor
            .capture_child(quarantine_name, quarantine_path)
            .map_err(|error| {
                format!(
                    "could not verify quarantined unverified publication {}: {error}",
                    quarantine_path.display()
                )
            })?;
        if &quarantined != installed {
            return Err(format!(
                "quarantined tree does not match the unverified publication snapshot: {}",
                quarantine_path.display()
            ));
        }
        let current = target_parent.capture_child(target_name, target)?;
        if &current == installed {
            return Err(format!(
                "unverified publication still occupies the Git-visible target after quarantine: {}",
                target.display()
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn quarantine_unverified_and_rollback(
    private: &mut PrivateDumpStage,
    target: &Path,
    target_parent: &DirectoryAnchor,
    target_name: &OsStr,
    quarantine_name: &OsStr,
    quarantine_path: &Path,
    installed: Option<&TreeSnapshot>,
    had_target: bool,
    backup_name: &OsStr,
    expected_backup: &TreeSnapshot,
    primary: String,
) -> Result<Vec<String>, String> {
    if let Err(error) = quarantine_unverified_install(
        private,
        target,
        target_parent,
        target_name,
        quarantine_name,
        quarantine_path,
        installed,
    ) {
        return Err(retain_recovery_error(
            private,
            format!("{primary}; {error}"),
        ));
    }
    if had_target {
        rollback_before_stage_install(
            private,
            target,
            target_parent,
            target_name,
            backup_name,
            expected_backup,
            primary,
        )
    } else {
        Err(primary)
    }
}

fn rollback_before_stage_install(
    private: &mut PrivateDumpStage,
    target: &Path,
    target_parent: &DirectoryAnchor,
    target_name: &OsStr,
    backup_name: &OsStr,
    expected_backup: &TreeSnapshot,
    primary: String,
) -> Result<Vec<String>, String> {
    let backup = private.recovery.join(backup_name);
    if let Err(error) = rename_child_no_replace(
        &private.recovery_anchor,
        backup_name,
        target_parent,
        target_name,
    ) {
        return Err(retain_recovery_error(
            private,
            format!(
                "{primary}; rollback could not atomically restore original target {} from {} without clobbering: {error}",
                target.display(),
                backup.display()
            ),
        ));
    }

    let restored = match target_parent.capture_child(target_name, target) {
        Ok(restored) => restored,
        Err(snapshot_error) => {
            let detail = format!(
                "{primary}; rollback source could not be validated after tentative restore: {snapshot_error}"
            );
            if let Err(quarantine_error) = quarantine_unverified_install(
                private,
                target,
                target_parent,
                target_name,
                backup_name,
                &backup,
                None,
            ) {
                return Err(retain_recovery_error(
                    private,
                    format!("{detail}; {quarantine_error}"),
                ));
            }
            return Err(retain_recovery_error(private, detail));
        }
    };
    if &restored != expected_backup {
        let detail = format!(
            "{primary}; rollback source changed before tentative restore and was not accepted: {}",
            target.display()
        );
        if matches!(restored, TreeSnapshot::Absent) {
            return Err(retain_recovery_error(private, detail));
        }
        if let Err(quarantine_error) = quarantine_unverified_install(
            private,
            target,
            target_parent,
            target_name,
            backup_name,
            &backup,
            Some(&restored),
        ) {
            return Err(retain_recovery_error(
                private,
                format!("{detail}; {quarantine_error}"),
            ));
        }
        return Err(retain_recovery_error(private, detail));
    }

    match target_parent.verify_path_binding() {
        Ok(()) => Err(primary),
        Err(binding_error) => {
            let quarantine = rename_child_no_replace(
                target_parent,
                target_name,
                &private.recovery_anchor,
                backup_name,
            );
            let detail = match quarantine {
                Ok(()) => format!(
                    "{primary}; target parent changed during rollback ({binding_error}); original target was returned to private recovery"
                ),
                Err(quarantine_error) => format!(
                    "{primary}; target parent changed during rollback ({binding_error}); original target could not be returned to private recovery: {quarantine_error}"
                ),
            };
            Err(retain_recovery_error(private, detail))
        }
    }
}

fn retain_recovery_error(private: &mut PrivateDumpStage, primary: String) -> String {
    match private.preserve_for_recovery() {
        Ok(()) => format!(
            "{primary}; secret-free recovery retained at {}",
            private.recovery.display()
        ),
        Err(cleanup_error) => format!(
            "{primary}; recovery could not be retained safely because {cleanup_error}; private root cleanup will be retried: {}",
            private.root.display()
        ),
    }
}

#[cfg(unix)]
fn rename_prechecked_directory_child_no_replace(
    source_parent: &DirectoryAnchor,
    source_name: &OsStr,
    expected_identity: FileIdentity,
    destination_parent: &DirectoryAnchor,
    destination_name: &OsStr,
) -> Result<(), String> {
    source_parent.verify_path_binding()?;
    let source =
        open_directory_child_nofollow(&source_parent.directory, source_name).map_err(|error| {
            format!(
                "failed to bind source directory {}: {error}",
                source_parent.path.join(source_name).display()
            )
        })?;
    let actual_identity = file_identity(&source).map_err(|error| {
        format!(
            "failed to inspect source directory identity {}: {error}",
            source_parent.path.join(source_name).display()
        )
    })?;
    if actual_identity != expected_identity {
        return Err(format!(
            "source directory identity changed before atomic rename: {}",
            source_parent.path.join(source_name).display()
        ));
    }
    rename_child_no_replace(
        source_parent,
        source_name,
        destination_parent,
        destination_name,
    )
}

#[cfg(not(unix))]
fn rename_prechecked_directory_child_no_replace(
    _source_parent: &DirectoryAnchor,
    _source_name: &OsStr,
    _expected_identity: FileIdentity,
    _destination_parent: &DirectoryAnchor,
    _destination_name: &OsStr,
) -> Result<(), String> {
    Err("prechecked atomic no-clobber directory rename is unavailable on this host".to_string())
}

#[cfg(unix)]
fn rename_child_no_replace(
    source_parent: &DirectoryAnchor,
    source_name: &OsStr,
    destination_parent: &DirectoryAnchor,
    destination_name: &OsStr,
) -> Result<(), String> {
    source_parent.verify_path_binding()?;
    destination_parent.verify_path_binding()?;
    let source_name = CString::new(source_name.as_bytes()).map_err(|error| {
        format!(
            "source name contains an embedded NUL byte in {}: {error}",
            source_parent.path.display()
        )
    })?;
    let destination_name = CString::new(destination_name.as_bytes()).map_err(|error| {
        format!(
            "destination name contains an embedded NUL byte in {}: {error}",
            destination_parent.path.display()
        )
    })?;
    #[cfg(target_os = "linux")]
    let no_replace_flag = libc::RENAME_NOREPLACE;
    #[cfg(target_os = "android")]
    let no_replace_flag = libc::RENAME_NOREPLACE as libc::c_uint;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    // SAFETY: both directory descriptors and both NUL-terminated names remain
    // live for the syscall. Calling SYS_renameat2 directly avoids a glibc-only
    // symbol on musl; RENAME_NOREPLACE atomically protects destination absence.
    let status = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            source_parent.directory.as_raw_fd(),
            source_name.as_ptr(),
            destination_parent.directory.as_raw_fd(),
            destination_name.as_ptr(),
            no_replace_flag,
        )
    };
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    // SAFETY: both directory descriptors and both NUL-terminated names remain
    // live for the syscall. RENAME_EXCL atomically protects destination absence.
    let status = unsafe {
        libc::renameatx_np(
            source_parent.directory.as_raw_fd(),
            source_name.as_ptr(),
            destination_parent.directory.as_raw_fd(),
            destination_name.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    )))]
    let status = {
        return Err("atomic no-clobber directory rename is unavailable on this host".to_string());
    };
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

#[cfg(not(unix))]
fn rename_child_no_replace(
    _source_parent: &DirectoryAnchor,
    _source_name: &OsStr,
    _destination_parent: &DirectoryAnchor,
    _destination_name: &OsStr,
) -> Result<(), String> {
    Err("atomic no-clobber directory rename is unavailable on this host".to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationCheckpoint {
    AfterBackup,
    BeforeStageInstall,
    BeforeSealedPublishRename,
    AfterStageInstall,
}

#[cfg(test)]
type PublicationHook = (PublicationCheckpoint, Box<dyn FnOnce()>);
#[cfg(test)]
type PathHook = Box<dyn FnOnce(&Path)>;
#[cfg(test)]
type TargetedReadHooks = (
    PathBuf,
    Option<Box<dyn FnOnce()>>,
    Option<Box<dyn FnOnce()>>,
);

#[cfg(test)]
thread_local! {
    static TEST_PUBLICATION_FAILPOINT: RefCell<Option<PublicationCheckpoint>> = const { RefCell::new(None) };
    static TEST_PUBLICATION_HOOK: RefCell<Option<PublicationHook>> = const { RefCell::new(None) };
    static TEST_SECURE_READ_HOOK: RefCell<Option<PathHook>> = const { RefCell::new(None) };
    static TEST_TARGETED_READ_HOOKS: RefCell<Option<TargetedReadHooks>> = const { RefCell::new(None) };
    static TEST_TREE_OPEN_HOOK: RefCell<Option<PathHook>> = const { RefCell::new(None) };
    static TEST_PRIVATE_CREATE_HOOK: RefCell<Option<PathHook>> = const { RefCell::new(None) };
    static TEST_TARGET_PARENT_CAPTURE_HOOK: RefCell<Option<PathHook>> = const { RefCell::new(None) };
}

#[cfg(test)]
fn with_publication_failpoint<T>(
    checkpoint: PublicationCheckpoint,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<PublicationCheckpoint>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_PUBLICATION_FAILPOINT.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous = TEST_PUBLICATION_FAILPOINT.with(|slot| slot.replace(Some(checkpoint)));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn with_publication_hook<T>(
    checkpoint: PublicationCheckpoint,
    hook: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<(PublicationCheckpoint, Box<dyn FnOnce()>)>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_PUBLICATION_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous =
        TEST_PUBLICATION_HOOK.with(|slot| slot.replace(Some((checkpoint, Box::new(hook)))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn with_secure_read_hook<T>(hook: impl FnOnce(&Path) + 'static, action: impl FnOnce() -> T) -> T {
    struct Reset(Option<PathHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_SECURE_READ_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous = TEST_SECURE_READ_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn with_targeted_read_hooks<T>(
    target: PathBuf,
    before: impl FnOnce() + 'static,
    after: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<TargetedReadHooks>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_TARGETED_READ_HOOKS.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let hooks = (
        target,
        Some(Box::new(before) as Box<dyn FnOnce()>),
        Some(Box::new(after) as Box<dyn FnOnce()>),
    );
    let previous = TEST_TARGETED_READ_HOOKS.with(|slot| slot.replace(Some(hooks)));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn with_tree_open_hook<T>(hook: impl FnOnce(&Path) + 'static, action: impl FnOnce() -> T) -> T {
    struct Reset(Option<PathHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_TREE_OPEN_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous = TEST_TREE_OPEN_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn with_private_create_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<PathHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_PRIVATE_CREATE_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous = TEST_PRIVATE_CREATE_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn with_target_parent_capture_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<PathHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_TARGET_PARENT_CAPTURE_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous = TEST_TARGET_PARENT_CAPTURE_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

fn run_private_create_hook(_path: &Path) {
    #[cfg(test)]
    if let Some(hook) = TEST_PRIVATE_CREATE_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
}

fn run_target_parent_capture_hook(_path: &Path) {
    #[cfg(test)]
    if let Some(hook) = TEST_TARGET_PARENT_CAPTURE_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
}

fn run_secure_read_hook(_path: &Path) {
    #[cfg(test)]
    if let Some(hook) = TEST_SECURE_READ_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
    #[cfg(test)]
    let hook = TEST_TARGETED_READ_HOOKS.with(|slot| {
        let mut slot = slot.borrow_mut();
        let (target, before, _) = slot.as_mut()?;
        if target == _path {
            before.take()
        } else {
            None
        }
    });
    #[cfg(test)]
    if let Some(hook) = hook {
        hook();
    }
}

fn run_secure_read_after_hook(_path: &Path) {
    #[cfg(test)]
    let hook = TEST_TARGETED_READ_HOOKS.with(|slot| {
        let mut slot = slot.borrow_mut();
        let (target, _, after) = slot.as_mut()?;
        if target == _path {
            after.take()
        } else {
            None
        }
    });
    #[cfg(test)]
    if let Some(hook) = hook {
        hook();
    }
}

fn run_tree_open_hook(_path: &Path) {
    #[cfg(test)]
    if let Some(hook) = TEST_TREE_OPEN_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
}

fn run_publication_failpoint(checkpoint: PublicationCheckpoint) -> Result<(), String> {
    #[cfg(test)]
    {
        let hook = TEST_PUBLICATION_HOOK.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot
                .as_ref()
                .is_some_and(|(expected, _)| *expected == checkpoint)
            {
                slot.take().map(|(_, hook)| hook)
            } else {
                None
            }
        });
        if let Some(hook) = hook {
            hook();
        }
        if TEST_PUBLICATION_FAILPOINT.with(|slot| slot.borrow().as_ref() == Some(&checkpoint)) {
            return Err(format!(
                "injected verified dump publication failure at {checkpoint:?}"
            ));
        }
    }
    let _ = checkpoint;
    Ok(())
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

fn finalize_private_outcome(
    private: &mut PrivateDumpStage,
    mut outcome: AdapterOutcome,
) -> AdapterOutcome {
    if !private.cleanup_on_drop {
        outcome.warnings.retain(|warning| {
            warning
                != "Git-visible sources were not published; the private staged dump was discarded"
        });
        outcome.warnings.push(format!(
            "Secret-free recovery was retained at {}",
            private.recovery.display()
        ));
        outcome
            .artifacts
            .push(private.recovery.display().to_string());
        return outcome;
    }
    if let Err(error) = private.cleanup_now() {
        if outcome.ok {
            outcome.ok = false;
            outcome.summary = format!(
                "{}; validated publication completed but private cleanup failed",
                outcome.summary
            );
        }
        outcome.errors.push(error.clone());
        outcome
            .warnings
            .push(if private.effective_config_secret_present {
                format!(
                    "Secret-bearing private dump debris may remain at {}",
                    private.root.display()
                )
            } else {
                format!(
                    "Private dump debris may remain at {}",
                    private.root.display()
                )
            });
        outcome.artifacts.push(private.root.display().to_string());
        outcome.stderr = Some(match outcome.stderr.take() {
            Some(stderr) if !stderr.is_empty() => format!("{stderr}\n{error}\n"),
            _ => format!("{error}\n"),
        });
    }
    outcome
}

fn dump_failure(
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

fn validate_dump_target(
    target: &Path,
    workspace_root: &Path,
    base_path: &Path,
    work_path: &Path,
) -> Result<(), String> {
    if !target.starts_with(workspace_root) {
        return Err(format!(
            "dump target must stay inside workspace root {}: {}",
            workspace_root.display(),
            target.display()
        ));
    }
    if target == workspace_root {
        return Err(format!(
            "dump target must not equal workspace root: {}",
            target.display()
        ));
    }
    if target == base_path {
        return Err("dump target must not equal project basePath".to_string());
    }
    if target == work_path {
        return Err("dump target must not equal workPath".to_string());
    }
    if target.parent().is_none() || target == Path::new("/") {
        return Err(format!(
            "dump target must not be a filesystem root: {}",
            target.display()
        ));
    }
    Ok(())
}

fn validate_physical_dump_target(
    target: &Path,
    snapshot: &TreeSnapshot,
    kind: SourceSetKind,
) -> Result<(), String> {
    let TreeSnapshot::Directory { entries, .. } = snapshot else {
        return Ok(());
    };
    let files = entries
        .iter()
        .filter_map(|entry| {
            matches!(entry.kind, TreeEntryKind::File { .. })
                .then_some(entry.relative_path.as_path())
        })
        .collect::<Vec<_>>();
    if files.is_empty() {
        return Ok(());
    }
    match classify_physical_source_inventory(kind, files.iter().copied()) {
        SourceFormat::PlatformXml => validate_required_owner(
            &target.join("Configuration.xml"),
            kind,
        )
        .map_err(|error| {
            format!(
                "{error}; the existing DESIGNER target is not an exact export format {TARGET_EXPORT_FORMAT} source set and will not be migrated automatically. Explicitly migrate it with platform {TARGET_PLATFORM_LINE} and retry; formats newer than {TARGET_EXPORT_FORMAT} require future platform 8.5 support"
            )
        }),
        SourceFormat::Edt => Err(format!(
            "physical dump target is EDT and cannot be replaced by a DESIGNER YAML claim: {}",
            target.display()
        )),
        SourceFormat::Invalid => Err(format!(
            "physical dump target has mixed/invalid DESIGNER and EDT markers: {}",
            target.display()
        )),
        SourceFormat::Unknown => Err(format!(
            "physical dump target has no authoritative DESIGNER marker and cannot be replaced: {}",
            target.display()
        )),
    }
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

fn merge_yaml_values(base: &mut YamlValue, overlay: YamlValue) {
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

fn default_platform_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        vec![
            PathBuf::from(r"C:\Program Files\1cv8"),
            PathBuf::from(r"C:\Program Files (x86)\1cv8"),
        ]
    }
    #[cfg(target_os = "linux")]
    {
        vec![
            PathBuf::from("/opt/1cv8/x86_64"),
            PathBuf::from("/opt/1cv8/i386"),
            PathBuf::from("/usr/local/1cv8"),
        ]
    }
    #[cfg(all(not(windows), not(target_os = "linux")))]
    {
        vec![PathBuf::from("/opt/1cv8")]
    }
}

fn verify_platform_candidate(
    candidate: &Path,
    utility: PlatformUtility,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
) -> Result<VerifiedPlatform, String> {
    let metadata = fs::symlink_metadata(candidate)
        .map_err(|error| format!("failed to inspect {}: {error}", candidate.display()))?;
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err(format!(
            "platform utility is not a real regular file: {}",
            candidate.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(format!(
                "platform utility is not executable: {}",
                candidate.display()
            ));
        }
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
    let probe_name = if cfg!(windows) { "ibcmd.exe" } else { "ibcmd" };
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
        timeout: Some(PLATFORM_PROBE_TIMEOUT),
        cancellation: cancellation.clone(),
    })?;
    if output.cancelled || cancellation.is_cancelled() {
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(format!("{role} is not executable: {}", path.display()));
        }
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

#[cfg(all(test, not(windows)))]
mod tests {
    use super::{
        normalize_key_value_connection, validate_staged_dump, with_private_create_hook,
        with_publication_failpoint, with_publication_hook, with_secure_read_hook,
        with_target_parent_capture_hook, with_targeted_read_hooks, with_tree_open_hook,
        FullDumpInvocation, PlatformResolver, PlatformUtility, PublicationCheckpoint,
        SystemPlatformResolver, TreeSnapshot, VerifiedFullDumpAdapter, VerifiedPlatform,
        TARGET_EXPORT_FORMAT,
    };
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::project_sources::SourceSetKind;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::internal_adapters::{ProcessCommand, ProcessOutput, ProcessRunner};
    use serde_json::{Map, Value};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn key_value_connection_preserves_quoted_file_path_and_semicolons() {
        let config_dir = Path::new("/workspace/project");
        let connection = r#"File="build/ib;archive";Usr=O'Brien;Pwd="secret;with:semicolon""#;

        let normalized = normalize_key_value_connection(connection, config_dir).unwrap();

        assert_eq!(
            normalized,
            r#"File="/workspace/project/build/ib;archive";Usr=O'Brien;Pwd="secret;with:semicolon""#
        );
    }

    struct FixedPlatform {
        executable: PathBuf,
        version: String,
    }

    impl PlatformResolver for FixedPlatform {
        fn resolve(
            &self,
            _effective_config: &serde_yaml::Value,
            _config_dir: &Path,
            _utility: PlatformUtility,
            _runner: &dyn ProcessRunner,
            _cancellation: &CancellationToken,
        ) -> Result<VerifiedPlatform, String> {
            Ok(VerifiedPlatform {
                executable: self.executable.clone(),
                exact_version: self.version.clone(),
                attestation: super::PlatformAttestation::capture_test_fixture(&self.executable)?,
            })
        }
    }

    struct FailingPlatform;

    impl PlatformResolver for FailingPlatform {
        fn resolve(
            &self,
            _effective_config: &serde_yaml::Value,
            _config_dir: &Path,
            _utility: PlatformUtility,
            _runner: &dyn ProcessRunner,
            _cancellation: &CancellationToken,
        ) -> Result<VerifiedPlatform, String> {
            Err("platform 8.3.26 does not match required 8.3.27".to_string())
        }
    }

    enum RunnerMutation {
        None,
        Config(PathBuf),
        Target(PathBuf),
        PlatformFile(PathBuf),
        PlatformInstall(PathBuf),
        #[cfg(unix)]
        RestrictPrivateRoot,
        #[cfg(unix)]
        Symlink,
        #[cfg(unix)]
        SwapPrivateRoot,
    }

    struct DumpRunner {
        owner_xml: String,
        mutation: RunnerMutation,
        calls: AtomicUsize,
    }

    impl DumpRunner {
        fn valid() -> Self {
            Self {
                owner_xml: valid_configuration_owner(),
                mutation: RunnerMutation::None,
                calls: AtomicUsize::new(0),
            }
        }
    }

    struct SwapRestoreProbeRunner {
        install: PathBuf,
        calls: AtomicUsize,
    }

    impl ProcessRunner for SwapRestoreProbeRunner {
        fn run(&self, _command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let displaced = self.install.with_extension("swap-restore-displaced");
            std::fs::rename(&self.install, &displaced).unwrap();
            std::fs::create_dir(&self.install).unwrap();
            std::fs::remove_dir(&self.install).unwrap();
            std::fs::rename(displaced, &self.install).unwrap();
            Ok(ProcessOutput {
                status_success: true,
                status: "0".to_string(),
                stdout: "8.3.27.2074".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
            })
        }
    }

    impl ProcessRunner for DumpRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let config_index = command
                .args
                .iter()
                .position(|arg| arg == "--config")
                .expect("effective config flag");
            let config_path = PathBuf::from(&command.args[config_index + 1]);
            let config: serde_yaml::Value =
                serde_yaml::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
            assert!(
                config.get("basePath").is_none(),
                "pinned v8-runner rejects the removed basePath key"
            );
            assert_eq!(config["format"].as_str(), Some("DESIGNER"));
            assert!(Path::new(config["workPath"].as_str().unwrap()).is_absolute());
            let connection = config["infobase"]["connection"].as_str().unwrap();
            if connection.starts_with("/F ") {
                assert!(
                    Path::new(connection.trim_start_matches("/F ")).is_absolute(),
                    "{connection}"
                );
            } else {
                assert!(
                    connection.to_ascii_lowercase().starts_with("file="),
                    "{connection}"
                );
            }
            assert_eq!(
                config["tools"]["platform"]["version"].as_str(),
                Some("8.3.27.2074")
            );
            assert!(config["tools"]["platform"]["path"]
                .as_str()
                .unwrap()
                .contains("8.3.27.2074"));
            let source_sets = config["source-set"].as_sequence().unwrap();
            let target = PathBuf::from(source_sets[0]["path"].as_str().unwrap());
            assert!(
                target
                    .file_name()
                    .is_some_and(|name| name == "staged-source"),
                "{}",
                target.display()
            );
            std::fs::create_dir_all(&target).unwrap();
            std::fs::write(target.join("Configuration.xml"), self.owner_xml.as_bytes()).unwrap();
            std::fs::write(
                target.join("Catalogs.xml"),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{TARGET_EXPORT_FORMAT}"><Catalog/></MetaDataObject>"#
                ),
            )
            .unwrap();
            match &self.mutation {
                RunnerMutation::None => {}
                RunnerMutation::Config(path) => {
                    std::fs::write(path, b"changed during child execution").unwrap();
                }
                RunnerMutation::Target(path) => {
                    std::fs::write(path.join("concurrent.txt"), b"concurrent").unwrap();
                }
                RunnerMutation::PlatformFile(path) => {
                    std::fs::write(path, b"replaced platform executable").unwrap();
                }
                RunnerMutation::PlatformInstall(path) => {
                    let displaced = path.with_extension("displaced");
                    std::fs::rename(path, &displaced).unwrap();
                    std::fs::create_dir_all(path).unwrap();
                    make_platform_executable(&path.join("1cv8"));
                    make_platform_executable(&path.join("ibcmd"));
                }
                #[cfg(unix)]
                RunnerMutation::RestrictPrivateRoot => {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(
                        config_path.parent().unwrap(),
                        std::fs::Permissions::from_mode(0o500),
                    )
                    .unwrap();
                }
                #[cfg(unix)]
                RunnerMutation::Symlink => {
                    std::os::unix::fs::symlink(
                        target.join("Configuration.xml"),
                        target.join("unsafe-link.xml"),
                    )
                    .unwrap();
                }
                #[cfg(unix)]
                RunnerMutation::SwapPrivateRoot => {
                    use std::os::unix::fs::PermissionsExt;

                    let private_root = config_path
                        .parent()
                        .and_then(Path::parent)
                        .expect("effective config is under the private root");
                    let displaced_name = format!(
                        "{}-displaced",
                        private_root.file_name().unwrap().to_string_lossy()
                    );
                    let displaced = private_root.with_file_name(displaced_name);
                    std::fs::rename(private_root, &displaced).unwrap();
                    std::fs::create_dir(private_root).unwrap();
                    std::fs::set_permissions(private_root, std::fs::Permissions::from_mode(0o700))
                        .unwrap();
                    for child in ["execution", "recovery"] {
                        let path = private_root.join(child);
                        std::fs::create_dir(&path).unwrap();
                        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                            .unwrap();
                    }
                }
            }
            Ok(ProcessOutput {
                status_success: true,
                status: "0".to_string(),
                stdout: "ok".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
            })
        }
    }

    fn valid_configuration_owner() -> String {
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties/></Configuration></MetaDataObject>"#.to_string()
    }

    fn seed_valid_target(target: &Path) {
        std::fs::create_dir_all(target).unwrap();
        std::fs::write(
            target.join("Configuration.xml"),
            valid_configuration_owner(),
        )
        .unwrap();
        std::fs::write(target.join("before.txt"), b"before").unwrap();
    }

    fn workspace(name: &str) -> (PathBuf, WorkspaceContext, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "unica-verified-full-dump-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let config = workspace.join("v8project.yaml");
        std::fs::write(
            &config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: '/F base'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace,
            cache_root: root.join("cache"),
            workspace_epoch: 0,
        };
        (root, context, config)
    }

    fn fixed_platform(root: &Path) -> FixedPlatform {
        let executable = root.join("8.3.27.2074/1cv8");
        make_platform_executable(&executable);
        make_platform_executable(&executable.parent().unwrap().join("ibcmd"));
        FixedPlatform {
            executable,
            version: "8.3.27.2074".to_string(),
        }
    }

    fn make_platform_executable(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"#!/bin/sh\nprintf '8.3.27.2074\\n'\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).unwrap();
        }
    }

    fn invoke_with_args(
        runner: &DumpRunner,
        platform: &dyn PlatformResolver,
        invocation: FullDumpInvocation,
        context: &WorkspaceContext,
        args: &Map<String, Value>,
    ) -> crate::application::AdapterOutcome {
        VerifiedFullDumpAdapter::with_dependencies(
            runner,
            platform,
            PathBuf::from("fake-v8-runner"),
        )
        .invoke(
            match invocation {
                FullDumpInvocation::BuildDump => "unica.build.dump",
                FullDumpInvocation::RuntimeExecute => "unica.runtime.execute",
            },
            invocation,
            args,
            context,
            &CancellationToken::new(),
        )
        .unwrap()
    }

    fn args() -> Map<String, Value> {
        Map::from_iter([
            ("dryRun".to_string(), Value::Bool(false)),
            ("mode".to_string(), Value::String("full".to_string())),
        ])
    }

    fn invoke(
        runner: &DumpRunner,
        platform: &dyn PlatformResolver,
        invocation: FullDumpInvocation,
        context: &WorkspaceContext,
    ) -> crate::application::AdapterOutcome {
        invoke_with_args(runner, platform, invocation, context, &args())
    }

    #[test]
    fn selected_source_set_target_must_be_relative_and_physically_contained_in_workspace() {
        for case in [
            "absolute",
            "parent-escape",
            "symlink-parent",
            "target-symlink",
        ] {
            let (root, context, config) = workspace(case);
            let outside = root.join("outside");
            std::fs::create_dir_all(&outside).unwrap();
            let configured_path = match case {
                "absolute" => outside.join("absolute-src").display().to_string(),
                "parent-escape" => "../../outside/parent-src".to_string(),
                "symlink-parent" => {
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(&outside, context.cwd.join("linked")).unwrap();
                    #[cfg(not(unix))]
                    std::fs::create_dir_all(context.cwd.join("linked")).unwrap();
                    "linked/symlink-src".to_string()
                }
                "target-symlink" => {
                    let real_target = context.cwd.join("real-target");
                    std::fs::create_dir_all(&real_target).unwrap();
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(&real_target, context.cwd.join("src")).unwrap();
                    #[cfg(not(unix))]
                    std::fs::create_dir_all(context.cwd.join("src")).unwrap();
                    "src".to_string()
                }
                _ => unreachable!(),
            };
            std::fs::write(
                &config,
                format!(
                    "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: '/F base'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: '{configured_path}'\n"
                ),
            )
            .unwrap();
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();

            let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

            assert!(!result.ok, "{case}: {result:?}");
            assert_eq!(runner.calls.load(Ordering::SeqCst), 0, "{case}");
            assert!(
                result.errors.join("\n").contains("workspace")
                    || result.errors.join("\n").contains("relative")
                    || result.errors.join("\n").contains("symbolic link")
                    || result.errors.join("\n").contains("real directory"),
                "{case}: {result:?}"
            );
            assert!(!outside.join("absolute-src/Configuration.xml").exists());
            assert!(!outside.join("parent-src/Configuration.xml").exists());
            assert!(!outside.join("symlink-src/Configuration.xml").exists());
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn target_parent_symlink_swap_after_text_normalization_cannot_escape_workspace_anchor() {
        let (root, context, config) = workspace("target-parent-normalization-swap");
        std::fs::write(
            &config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: '/F base'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: inside/src\n",
        )
        .unwrap();
        let inside = context.cwd.join("inside");
        let displaced = context.cwd.join("inside-displaced");
        let outside = root.join("outside");
        std::fs::create_dir(&inside).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let hook_inside = inside.clone();
        let hook_displaced = displaced.clone();
        let hook_outside = outside.clone();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();

        let result = with_target_parent_capture_hook(
            move |_normalized_parent| {
                std::fs::rename(&hook_inside, &hook_displaced).unwrap();
                std::os::unix::fs::symlink(&hook_outside, &hook_inside).unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert!(!outside.join("src").exists(), "{result:?}");
        assert!(
            std::fs::read_dir(&outside).unwrap().all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".unica-dump-guard-")),
            "{result:?}"
        );
        assert!(
            result.errors.join("\n").contains("workspace")
                || result.errors.join("\n").contains("link"),
            "{result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn physical_edt_or_mixed_target_cannot_be_replaced_by_designer_yaml_claim() {
        for case in ["edt", "mixed"] {
            let (root, context, _) = workspace(case);
            let target = context.cwd.join("src");
            std::fs::create_dir_all(target.join("Configuration")).unwrap();
            std::fs::write(target.join(".project"), b"<projectDescription/>").unwrap();
            std::fs::write(
                target.join("Configuration/Configuration.mdo"),
                b"<mdclass:Configuration/>",
            )
            .unwrap();
            if case == "mixed" {
                std::fs::write(
                    target.join("Configuration.xml"),
                    valid_configuration_owner(),
                )
                .unwrap();
            }
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();

            let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

            assert!(!result.ok, "{case}: {result:?}");
            assert_eq!(runner.calls.load(Ordering::SeqCst), 0, "{case}");
            assert!(target.join(".project").is_file(), "{case}");
            assert!(
                result.errors.join("\n").contains("EDT")
                    || result.errors.join("\n").contains("mixed")
                    || result.errors.join("\n").contains("invalid"),
                "{case}: {result:?}"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn existing_legacy_designer_target_requires_explicit_user_migration() {
        let (root, context, _) = workspace("legacy-designer-target");
        let target = context.cwd.join("src");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(
            target.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration><Properties/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();

        let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

        assert!(!result.ok, "{result:?}");
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert!(
            result
                .errors
                .join("\n")
                .contains("not be migrated automatically")
                && result.errors.join("\n").contains("8.3.27"),
            "{result:?}"
        );
        assert!(std::fs::read_to_string(target.join("Configuration.xml"))
            .unwrap()
            .contains("2.19"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn both_synchronous_routes_publish_only_validated_private_stage() {
        for invocation in [
            FullDumpInvocation::BuildDump,
            FullDumpInvocation::RuntimeExecute,
        ] {
            let (root, context, _) = workspace(match invocation {
                FullDumpInvocation::BuildDump => "build-success",
                FullDumpInvocation::RuntimeExecute => "runtime-success",
            });
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();

            let result = invoke(&runner, &platform, invocation, &context);

            assert!(result.ok, "{invocation:?}: {result:?}");
            assert!(context.cwd.join("src/Configuration.xml").is_file());
            assert_eq!(runner.calls.load(Ordering::SeqCst), 1);
            assert!(
                std::fs::read_dir(&context.cwd).unwrap().all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".unica-dump-guard-")),
                "private stage debris remained"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn private_root_creation_is_bound_to_the_captured_target_parent() {
        let (root, context, _) = workspace("private-create-parent-swap");
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let visible_workspace = context.cwd.clone();
        let displaced_workspace = root.join("workspace-displaced");
        let hook_visible_workspace = visible_workspace.clone();
        let hook_displaced_workspace = displaced_workspace.clone();

        let result = with_private_create_hook(
            move |_private_root| {
                std::fs::rename(&hook_visible_workspace, &hook_displaced_workspace).unwrap();
                std::fs::create_dir(&hook_visible_workspace).unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        for workspace in [&visible_workspace, &displaced_workspace] {
            assert!(
                std::fs::read_dir(workspace).unwrap().all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".unica-dump-guard-")),
                "private root escaped into a replaced parent: {}",
                workspace.display()
            );
        }
        assert!(
            result
                .errors
                .join("\n")
                .contains("directory anchor identity changed"),
            "{result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unselected_external_source_set_does_not_block_configuration_dump() {
        let (root, context, config) = workspace("configuration-with-external");
        std::fs::write(
            &config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: '/F base'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: processors\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external\n",
        )
        .unwrap();
        std::fs::create_dir_all(context.cwd.join("external")).unwrap();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();

        let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

        assert!(result.ok, "{result:?}");
        assert_eq!(runner.calls.load(Ordering::SeqCst), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn applied_full_dump_rejects_ignored_route_arguments_before_runner_execution() {
        let (root, context, _) = workspace("ignored-route-argument");
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let mut unsupported = args();
        unsupported.insert(
            "object".to_string(),
            Value::String("Catalog.Items".to_string()),
        );

        let result = VerifiedFullDumpAdapter::with_dependencies(
            &runner,
            &platform,
            PathBuf::from("fake-v8-runner"),
        )
        .invoke(
            "unica.runtime.execute",
            FullDumpInvocation::RuntimeExecute,
            &unsupported,
            &context,
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(!result.ok, "{result:?}");
        assert!(result
            .errors
            .join("\n")
            .contains("does not accept `object`"));
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selected_external_source_set_is_blocked_before_runner_execution() {
        let (root, context, config) = workspace("selected-external");
        std::fs::write(
            &config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: '/F base'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: processors\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external\n",
        )
        .unwrap();
        std::fs::create_dir_all(context.cwd.join("external")).unwrap();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let mut selected = args();
        selected.insert(
            "sourceSet".to_string(),
            Value::String("processors".to_string()),
        );

        let result = VerifiedFullDumpAdapter::with_dependencies(
            &runner,
            &platform,
            PathBuf::from("fake-v8-runner"),
        )
        .invoke(
            "unica.runtime.execute",
            FullDumpInvocation::RuntimeExecute,
            &selected,
            &context,
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(!result.ok, "{result:?}");
        assert!(result.errors.join("\n").contains("is external"));
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_staged_xml_never_changes_visible_target() {
        for (case, owner_xml, expected) in [
            (
                "older",
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration/></MetaDataObject>"#,
                "2.19",
            ),
            (
                "newer",
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Configuration/></MetaDataObject>"#,
                "2.21",
            ),
            (
                "entity",
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.&#50;0"><Configuration/></MetaDataObject>"#,
                "2.&#50;0",
            ),
            (
                "malformed",
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration>"#,
                "parse",
            ),
        ] {
            let (root, context, _) = workspace(case);
            let target = context.cwd.join("src");
            seed_valid_target(&target);
            let platform = fixed_platform(&root);
            let runner = DumpRunner {
                owner_xml: owner_xml.to_string(),
                mutation: RunnerMutation::None,
                calls: AtomicUsize::new(0),
            };

            let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

            assert!(!result.ok, "{case}: {result:?}");
            assert!(
                result.errors.join("\n").contains(expected),
                "{case}: {result:?}"
            );
            assert_eq!(std::fs::read(target.join("before.txt")).unwrap(), b"before");
            assert_eq!(
                std::fs::read_to_string(target.join("Configuration.xml")).unwrap(),
                valid_configuration_owner()
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn platform_mismatch_blocks_before_runner_execution() {
        let (root, context, _) = workspace("platform-mismatch");
        let runner = DumpRunner::valid();

        let result = invoke(
            &runner,
            &FailingPlatform,
            FullDumpInvocation::BuildDump,
            &context,
        );

        assert!(!result.ok, "{result:?}");
        assert!(result.errors.join("\n").contains("8.3.26"));
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert!(!context.cwd.join("src").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn system_resolver_does_not_trust_an_explicit_platform_path_or_broad_claim() {
        let root = std::env::temp_dir().join(format!(
            "unica-platform-resolution-{}",
            uuid::Uuid::new_v4()
        ));
        let wrong = root
            .join("8.3.26.9999")
            .join(if cfg!(windows) { "1cv8.exe" } else { "1cv8" });
        make_platform_executable(&wrong);
        let wrong_config: serde_yaml::Value = serde_yaml::from_str(&format!(
            "tools:\n  platform:\n    path: '{}'\n    version: 8.3.27\n",
            wrong.display()
        ))
        .unwrap();

        let error = SystemPlatformResolver
            .resolve(
                &wrong_config,
                &root,
                PlatformUtility::Designer,
                crate::infrastructure::internal_adapters::system_process_runner(),
                &CancellationToken::new(),
            )
            .unwrap_err();

        assert!(error.contains("8.3.27"), "{error}");

        let exact = root.join("8.3.27.2074").join("1cv8");
        make_platform_executable(&exact);
        make_platform_executable(&exact.parent().unwrap().join("ibcmd"));
        let exact_config: serde_yaml::Value = serde_yaml::from_str(&format!(
            "tools:\n  platform:\n    path: '{}'\n    version: '8.3'\n",
            exact.display()
        ))
        .unwrap();

        let error = SystemPlatformResolver
            .resolve(
                &exact_config,
                &root,
                PlatformUtility::Designer,
                crate::infrastructure::internal_adapters::system_process_runner(),
                &CancellationToken::new(),
            )
            .expect_err("a user-owned platform installation must be refused");
        assert!(
            error.contains("immutable")
                || error.contains("owned by root")
                || error.contains("effective root caller"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mutable_platform_swap_and_restore_is_blocked_before_any_child_process() {
        let (root, context, config) = workspace("platform-swap-restore-before-probe");
        let exact = root.join("8.3.27.2074").join("1cv8");
        make_platform_executable(&exact);
        make_platform_executable(&exact.parent().unwrap().join("ibcmd"));
        std::fs::write(
            &config,
            format!(
                "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: '/F base'\ntools:\n  platform:\n    path: '{}'\n    version: '8.3.27'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
                exact.display()
            ),
        )
        .unwrap();
        let runner = SwapRestoreProbeRunner {
            install: exact.parent().unwrap().to_path_buf(),
            calls: AtomicUsize::new(0),
        };

        let result = VerifiedFullDumpAdapter::with_dependencies(
            &runner,
            &SystemPlatformResolver,
            PathBuf::from("fake-v8-runner"),
        )
        .invoke(
            "unica.build.dump",
            FullDumpInvocation::BuildDump,
            &args(),
            &context,
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            runner.calls.load(Ordering::SeqCst),
            0,
            "a mutable installation must be rejected before its probe or v8-runner can swap and restore it"
        );
        assert!(
            result.errors.join("\n").contains("immutable")
                || result.errors.join("\n").contains("owned by root")
                || result.errors.join("\n").contains("effective root caller"),
            "{result:?}"
        );
        assert!(!context.cwd.join("src").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn system_resolver_rejects_fake_binary_even_under_exact_version_directory_name() {
        let root = std::env::temp_dir().join(format!(
            "unica-platform-fake-resolution-{}",
            uuid::Uuid::new_v4()
        ));
        let fake = root.join("8.3.27.2074/1cv8");
        std::fs::create_dir_all(fake.parent().unwrap()).unwrap();
        std::fs::write(&fake, b"not a platform executable").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let config: serde_yaml::Value = serde_yaml::from_str(&format!(
            "tools:\n  platform:\n    path: '{}'\n    version: '8.3.27'\n",
            fake.display()
        ))
        .unwrap();

        let error = SystemPlatformResolver
            .resolve(
                &config,
                &root,
                PlatformUtility::Designer,
                crate::infrastructure::internal_adapters::system_process_runner(),
                &CancellationToken::new(),
            )
            .expect_err("a directory name is not platform attestation");

        assert!(
            error.contains("probe") || error.contains("version"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn platform_executable_or_install_swap_after_runner_prevents_publication() {
        for case in ["file", "install"] {
            let (root, context, _) = workspace(&format!("platform-swap-{case}"));
            let target = context.cwd.join("src");
            let platform = fixed_platform(&root);
            let executable = platform.executable.clone();
            let runner = DumpRunner {
                owner_xml: valid_configuration_owner(),
                mutation: match case {
                    "file" => RunnerMutation::PlatformFile(executable),
                    "install" => RunnerMutation::PlatformInstall(
                        platform.executable.parent().unwrap().to_path_buf(),
                    ),
                    _ => unreachable!(),
                },
                calls: AtomicUsize::new(0),
            };

            let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

            assert!(!result.ok, "{case}: {result:?}");
            assert!(result.changes.is_empty(), "{case}: {result:?}");
            assert!(!target.join("Configuration.xml").exists(), "{case}");
            assert!(
                result.errors.join("\n").contains("platform")
                    && (result.errors.join("\n").contains("changed")
                        || result.errors.join("\n").contains("attestation")),
                "{case}: {result:?}"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn config_and_target_races_block_publication_without_overwriting_concurrent_state() {
        for case in ["config", "target"] {
            let (root, context, config) = workspace(case);
            let target = context.cwd.join("src");
            seed_valid_target(&target);
            let platform = fixed_platform(&root);
            let runner = DumpRunner {
                owner_xml: valid_configuration_owner(),
                mutation: match case {
                    "config" => RunnerMutation::Config(config.clone()),
                    "target" => RunnerMutation::Target(target.clone()),
                    _ => unreachable!(),
                },
                calls: AtomicUsize::new(0),
            };

            let result = invoke(
                &runner,
                &platform,
                FullDumpInvocation::RuntimeExecute,
                &context,
            );

            assert!(!result.ok, "{case}: {result:?}");
            assert!(result.changes.is_empty(), "{case}: {result:?}");
            assert_eq!(std::fs::read(target.join("before.txt")).unwrap(), b"before");
            assert_eq!(
                std::fs::read_to_string(target.join("Configuration.xml")).unwrap(),
                valid_configuration_owner()
            );
            if case == "target" {
                assert_eq!(
                    std::fs::read(target.join("concurrent.txt")).unwrap(),
                    b"concurrent"
                );
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn publication_failure_after_backup_restores_original_tree() {
        let (root, context, _) = workspace("rollback");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();

        let result = with_publication_failpoint(PublicationCheckpoint::AfterBackup, || {
            invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context)
        });

        assert!(!result.ok, "{result:?}");
        assert!(result.errors.join("\n").contains("injected"));
        assert_eq!(std::fs::read(target.join("before.txt")).unwrap(), b"before");
        assert_eq!(
            std::fs::read_to_string(target.join("Configuration.xml")).unwrap(),
            valid_configuration_owner()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn backup_name_replacement_is_not_accepted_as_rollback_source() {
        for case in ["snapshot-mismatch", "snapshot-error"] {
            let (root, context, _) = workspace(&format!("rollback-backup-name-replacement-{case}"));
            let target = context.cwd.join("src");
            seed_valid_target(&target);
            let displaced_backup = context.cwd.join("validated-backup-displaced");
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();
            let hook_cwd = context.cwd.clone();
            let hook_displaced = displaced_backup.clone();

            let result = with_publication_hook(
                PublicationCheckpoint::BeforeSealedPublishRename,
                move || {
                    let private_root = std::fs::read_dir(&hook_cwd)
                        .unwrap()
                        .flatten()
                        .map(|entry| entry.path())
                        .find(|path| {
                            path.file_name().is_some_and(|name| {
                                name.to_string_lossy().starts_with(".unica-dump-guard-")
                            })
                        })
                        .expect("private dump root");
                    let recovery = private_root.join("recovery");
                    let backup = std::fs::read_dir(&recovery)
                        .unwrap()
                        .flatten()
                        .map(|entry| entry.path())
                        .find(|path| {
                            path.file_name().is_some_and(|name| {
                                name.to_string_lossy().starts_with("target-backup-")
                            })
                        })
                        .expect("validated target backup");
                    std::fs::rename(&backup, &hook_displaced).unwrap();
                    std::fs::create_dir(&backup).unwrap();
                    std::fs::write(
                        backup.join("Configuration.xml"),
                        valid_configuration_owner(),
                    )
                    .unwrap();
                    match case {
                        "snapshot-mismatch" => {
                            std::fs::write(backup.join("unvalidated.txt"), b"unvalidated").unwrap();
                        }
                        "snapshot-error" => {
                            std::os::unix::fs::symlink(
                                backup.join("Configuration.xml"),
                                backup.join("unvalidated-link.xml"),
                            )
                            .unwrap();
                        }
                        _ => unreachable!(),
                    }
                },
                || {
                    with_publication_failpoint(
                        PublicationCheckpoint::BeforeSealedPublishRename,
                        || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
                    )
                },
            );

            assert!(!result.ok, "{case}: {result:?}");
            assert!(
                !target.join("unvalidated.txt").exists()
                    && std::fs::symlink_metadata(target.join("unvalidated-link.xml")).is_err(),
                "{case}: a replacement under the backup name must not remain Git-visible"
            );
            assert!(
                displaced_backup.join("before.txt").is_file(),
                "{case}: the independently displaced original remains untouched"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn concurrent_target_replacement_after_stage_install_survives_rollback() {
        let (root, context, _) = workspace("rollback-concurrent-after-install");
        let target = context.cwd.join("src");
        let displaced_stage = context.cwd.join("externally-displaced-stage");
        seed_valid_target(&target);
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let hook_target = target.clone();
        let hook_displaced_stage = displaced_stage.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::AfterStageInstall,
            move || {
                std::fs::rename(&hook_target, &hook_displaced_stage).unwrap();
                std::fs::create_dir_all(&hook_target).unwrap();
                std::fs::write(hook_target.join("concurrent.txt"), b"concurrent").unwrap();
            },
            || {
                with_publication_failpoint(PublicationCheckpoint::AfterStageInstall, || {
                    invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context)
                })
            },
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            std::fs::read(target.join("concurrent.txt")).unwrap(),
            b"concurrent",
            "rollback must never move or overwrite a target it no longer owns"
        );
        assert!(displaced_stage.join("Configuration.xml").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn installed_tree_interior_mutation_after_first_probe_is_quarantined_and_rolled_back() {
        let (root, context, _) = workspace("installed-tree-interior-mutation");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let hook_target = target.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::AfterStageInstall,
            move || {
                std::fs::write(hook_target.join("unvalidated.txt"), b"unvalidated").unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            std::fs::read(target.join("before.txt")).unwrap(),
            b"before",
            "the original target must be restored after mutation of our installed directory"
        );
        assert!(
            !target.join("unvalidated.txt").exists(),
            "our installed but mutated directory must be quarantined before lock release"
        );
        assert!(
            result.errors.join("\n").contains("published dump differs")
                || result.errors.join("\n").contains("quarantine"),
            "{result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn installed_tree_capture_error_with_same_root_identity_is_quarantined_and_rolled_back() {
        let (root, context, _) = workspace("installed-tree-interior-symlink");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let hook_target = target.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::AfterStageInstall,
            move || {
                std::os::unix::fs::symlink(
                    hook_target.join("Configuration.xml"),
                    hook_target.join("unvalidated-link.xml"),
                )
                .unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            std::fs::read(target.join("before.txt")).unwrap(),
            b"before",
            "the original target must be restored after capture failure in our installed directory"
        );
        assert!(
            std::fs::symlink_metadata(target.join("unvalidated-link.xml")).is_err(),
            "our installed but invalid directory must be quarantined before lock release"
        );
        assert!(
            result
                .errors
                .join("\n")
                .contains("could not verify the published target")
                || result.errors.join("\n").contains("quarantine"),
            "{result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn stage_name_replacement_before_publish_never_becomes_visible() {
        let (root, context, _) = workspace("stage-name-replacement");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let displaced_stage = context.cwd.join("validated-stage-displaced");
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let hook_cwd = context.cwd.clone();
        let hook_displaced = displaced_stage.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::BeforeStageInstall,
            move || {
                let private_root = std::fs::read_dir(&hook_cwd)
                    .unwrap()
                    .flatten()
                    .map(|entry| entry.path())
                    .find(|path| {
                        path.file_name().is_some_and(|name| {
                            name.to_string_lossy().starts_with(".unica-dump-guard-")
                        })
                    })
                    .expect("private dump root");
                let stage = private_root.join("execution/staged-source");
                std::fs::rename(&stage, &hook_displaced).unwrap();
                std::fs::create_dir_all(&stage).unwrap();
                std::fs::write(stage.join("Configuration.xml"), valid_configuration_owner())
                    .unwrap();
                std::os::unix::fs::symlink(
                    stage.join("Configuration.xml"),
                    stage.join("unvalidated-link.xml"),
                )
                .unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            std::fs::read(target.join("before.txt")).unwrap(),
            b"before",
            "the original target must be restored without exposing the replacement stage"
        );
        assert!(
            !target.join("unvalidated-link.xml").exists(),
            "the replacement stage must never remain Git-visible"
        );
        assert!(displaced_stage.join("Configuration.xml").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn sealed_stage_replacement_at_publish_boundary_is_quarantined_before_lock_release() {
        let (root, context, _) = workspace("sealed-stage-publish-replacement");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let displaced_stage = context.cwd.join("sealed-validated-stage-displaced");
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let hook_cwd = context.cwd.clone();
        let hook_displaced = displaced_stage.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::BeforeSealedPublishRename,
            move || {
                let private_root = std::fs::read_dir(&hook_cwd)
                    .unwrap()
                    .flatten()
                    .map(|entry| entry.path())
                    .find(|path| {
                        path.file_name().is_some_and(|name| {
                            name.to_string_lossy().starts_with(".unica-dump-guard-")
                        })
                    })
                    .expect("private dump root");
                let recovery = private_root.join("recovery");
                let sealed = std::fs::read_dir(&recovery)
                    .unwrap()
                    .flatten()
                    .map(|entry| entry.path())
                    .find(|path| {
                        path.file_name()
                            .is_some_and(|name| name.to_string_lossy().starts_with("sealed-stage-"))
                    })
                    .expect("sealed staged dump");
                std::fs::rename(&sealed, &hook_displaced).unwrap();
                std::fs::create_dir(&sealed).unwrap();
                std::fs::write(
                    sealed.join("Configuration.xml"),
                    valid_configuration_owner(),
                )
                .unwrap();
                std::fs::write(sealed.join("unvalidated.txt"), b"unvalidated").unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            std::fs::read(target.join("before.txt")).unwrap(),
            b"before",
            "the original target must be restored before the publication lock is released"
        );
        assert!(
            !target.join("unvalidated.txt").exists(),
            "the source-name replacement must be removed from the Git-visible target"
        );
        assert!(displaced_stage.join("Configuration.xml").is_file());
        assert!(
            result.errors.join("\n").contains("just-installed")
                || result.errors.join("\n").contains("unverified")
                || result.errors.join("\n").contains("quarantine"),
            "{result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preserved_recovery_never_contains_effective_config_or_credentials() {
        let (root, context, config) = workspace("recovery-secrets");
        std::fs::write(
            &config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: 'File=base;Usr=Admin;Pwd=top-secret'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let displaced_stage = context.cwd.join("secret-test-displaced-stage");
        let platform = fixed_platform(&root);
        let runner = DumpRunner {
            owner_xml: valid_configuration_owner(),
            mutation: RunnerMutation::None,
            calls: AtomicUsize::new(0),
        };
        let hook_target = target.clone();
        let hook_displaced_stage = displaced_stage.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::AfterStageInstall,
            move || {
                std::fs::rename(&hook_target, &hook_displaced_stage).unwrap();
                std::fs::create_dir_all(&hook_target).unwrap();
                std::fs::write(hook_target.join("concurrent.txt"), b"concurrent").unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        let recovery_roots = std::fs::read_dir(&context.cwd)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".unica-dump-guard-"))
            })
            .collect::<Vec<_>>();
        assert!(!recovery_roots.is_empty(), "{result:?}");
        for recovery in recovery_roots {
            let snapshot = super::capture_directory_snapshot(&recovery).unwrap();
            assert!(
                snapshot.iter().all(|entry| {
                    entry
                        .relative_path
                        .file_name()
                        .is_none_or(|name| name != super::EFFECTIVE_CONFIG_NAME)
                }),
                "effective config leaked into recovery: {}",
                recovery.display()
            );
            for entry in snapshot {
                if let super::TreeEntryKind::File { .. } = entry.kind {
                    let raw = std::fs::read(recovery.join(entry.relative_path)).unwrap();
                    assert!(
                        !String::from_utf8_lossy(&raw).contains("top-secret"),
                        "credentials leaked into recovery: {}",
                        recovery.display()
                    );
                }
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn private_root_swap_cannot_escape_secret_scrubbing_or_anchor_cleanup() {
        let (root, context, config) = workspace("private-root-swap-cleanup");
        std::fs::write(
            &config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: 'File=base;Usr=Admin;Pwd=top-secret'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let platform = fixed_platform(&root);
        let runner = DumpRunner {
            owner_xml: valid_configuration_owner(),
            mutation: RunnerMutation::SwapPrivateRoot,
            calls: AtomicUsize::new(0),
        };

        let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

        assert!(!result.ok, "{result:?}");
        assert_eq!(runner.calls.load(Ordering::SeqCst), 1);
        assert!(!context.cwd.join("src").exists(), "{result:?}");
        let private_roots = std::fs::read_dir(&context.cwd)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".unica-dump-guard-"))
            })
            .collect::<Vec<_>>();
        assert!(
            private_roots.iter().any(|path| path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("-displaced")),
            "{private_roots:?}; {result:?}"
        );
        for private_root in &private_roots {
            let snapshot = super::capture_directory_snapshot(private_root).unwrap();
            assert!(
                snapshot.iter().all(|entry| {
                    entry
                        .relative_path
                        .file_name()
                        .is_none_or(|name| name != super::EFFECTIVE_CONFIG_NAME)
                }),
                "effective config escaped cleanup: {}",
                private_root.display()
            );
            for entry in snapshot {
                if let super::TreeEntryKind::File { .. } = entry.kind {
                    let raw = std::fs::read(private_root.join(entry.relative_path)).unwrap();
                    assert!(
                        !String::from_utf8_lossy(&raw).contains("top-secret"),
                        "credentials escaped cleanup: {}",
                        private_root.display()
                    );
                }
            }
        }
        assert!(
            result.errors.join("\n").contains("identity changed")
                || result.warnings.join("\n").contains("debris"),
            "{result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn private_stage_cleanup_failures_are_reported() {
        use std::os::unix::fs::PermissionsExt;

        let (root, context, _) = workspace("stage-remove-error");
        let platform = fixed_platform(&root);
        let runner = DumpRunner {
            owner_xml: valid_configuration_owner(),
            mutation: RunnerMutation::RestrictPrivateRoot,
            calls: AtomicUsize::new(0),
        };

        let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

        assert!(!result.ok, "{result:?}");
        assert!(
            result.errors.join("\n").contains("cleanup failed")
                || result.warnings.join("\n").contains("cleanup failed"),
            "{result:?}"
        );
        for entry in std::fs::read_dir(&context.cwd).unwrap().flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(".unica-dump-guard-")
            {
                std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o700))
                    .unwrap();
                let execution = entry.path().join("execution");
                if execution.exists() {
                    std::fs::set_permissions(execution, std::fs::Permissions::from_mode(0o700))
                        .unwrap();
                }
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_in_private_stage_is_rejected_before_publication() {
        let (root, context, _) = workspace("stage-symlink");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let platform = fixed_platform(&root);
        let runner = DumpRunner {
            owner_xml: valid_configuration_owner(),
            mutation: RunnerMutation::Symlink,
            calls: AtomicUsize::new(0),
        };

        let result = invoke(
            &runner,
            &platform,
            FullDumpInvocation::RuntimeExecute,
            &context,
        );

        assert!(!result.ok, "{result:?}");
        assert!(result.errors.join("\n").contains("symbolic link"));
        assert_eq!(std::fs::read(target.join("before.txt")).unwrap(), b"before");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn owner_kind_is_bound_to_selected_source_set() {
        let root =
            std::env::temp_dir().join(format!("unica-dump-owner-kind-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("Configuration.xml"), valid_configuration_owner()).unwrap();

        let error = validate_staged_dump(&root, SourceSetKind::Extension).unwrap_err();

        assert!(error.contains("without ConfigurationExtensionPurpose"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_tree_uses_closed_root_registry_and_exact_raw_versions() {
        let cases = [
            (
                "known-missing",
                "Forms/Main/Ext/Form.xml",
                r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"/>"#,
                "missing",
            ),
            (
                "known-numeric-equivalent",
                "Forms/Main/Ext/Form.xml",
                r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.200"/>"#,
                "2.200",
            ),
            (
                "unknown-version-bearing",
                "Unknown.xml",
                r#"<Unknown xmlns="urn:unknown" version="2.20"/>"#,
                "unsupported",
            ),
            (
                "unknown-versionless",
                "Unknown.xml",
                r#"<Unknown xmlns="urn:unknown"/>"#,
                "unsupported",
            ),
            (
                "versionless-family-with-version",
                "Templates/Main/Ext/Template.xml",
                r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema" version="2.20"/>"#,
                "versionless",
            ),
        ];
        for (case, relative, xml, expected) in cases {
            let root = std::env::temp_dir().join(format!(
                "unica-staged-root-registry-{case}-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("Configuration.xml"), valid_configuration_owner()).unwrap();
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, xml).unwrap();

            let error = validate_staged_dump(&root, SourceSetKind::Configuration)
                .expect_err("unregistered or inexact staged XML must fail closed");

            assert!(error.contains(expected), "{case}: {error}");
            std::fs::remove_dir_all(root).unwrap();
        }

        let root = std::env::temp_dir().join(format!(
            "unica-staged-root-registry-versionless-control-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join("Templates/Main/Ext")).unwrap();
        std::fs::write(root.join("Configuration.xml"), valid_configuration_owner()).unwrap();
        std::fs::write(
            root.join("Templates/Main/Ext/Template.xml"),
            r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>"#,
        )
        .unwrap();
        validate_staged_dump(&root, SourceSetKind::Configuration)
            .expect("registered versionless subordinate family remains allowed");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_xml_bytes_must_match_the_tree_snapshot_even_if_path_is_restored() {
        let root =
            std::env::temp_dir().join(format!("unica-staged-read-race-{}", uuid::Uuid::new_v4()));
        let form = root.join("Forms/Main/Ext/Form.xml");
        std::fs::create_dir_all(form.parent().unwrap()).unwrap();
        std::fs::write(root.join("Configuration.xml"), valid_configuration_owner()).unwrap();
        let invalid =
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.19"/>"#.to_string();
        let valid = r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#.to_string();
        std::fs::write(&form, &invalid).unwrap();
        let form_before = form.clone();
        let form_after = form.clone();

        let result = with_targeted_read_hooks(
            form.clone(),
            move || std::fs::write(&form_before, valid).unwrap(),
            move || std::fs::write(&form_after, invalid).unwrap(),
            || validate_staged_dump(&root, SourceSetKind::Configuration),
        );

        let error = result.expect_err(
            "XML bytes parsed during validation must be bound to the captured tree snapshot",
        );
        assert!(
            error.contains("snapshot") || error.contains("changed"),
            "{error}"
        );
        assert!(
            std::fs::read_to_string(&form).unwrap().contains("2.19"),
            "the test must restore the invalid preimage before validation returns"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_swap_between_file_inspection_and_open_is_rejected() {
        let root =
            std::env::temp_dir().join(format!("unica-staged-file-swap-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let owner = root.join("Configuration.xml");
        let original = root.join("Configuration.original.xml");
        let alternate = root.join("Configuration.alternate.xml");
        std::fs::write(&owner, valid_configuration_owner()).unwrap();
        std::fs::write(&alternate, valid_configuration_owner()).unwrap();
        let owner_for_hook = owner.clone();
        let original_for_hook = original.clone();
        let alternate_for_hook = alternate.clone();

        let result = with_secure_read_hook(
            move |path| {
                assert_eq!(path, owner_for_hook);
                std::fs::rename(&owner_for_hook, &original_for_hook).unwrap();
                std::os::unix::fs::symlink(&alternate_for_hook, &owner_for_hook).unwrap();
            },
            || super::secure_read_regular_file(&owner, "swap fixture"),
        );

        let error = result.expect_err("O_NOFOLLOW/identity binding must reject the swap");
        assert!(
            error.contains("symbolic link")
                || error.contains("changed")
                || error.contains("identity"),
            "{error}"
        );
        std::fs::remove_file(&owner).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_swap_between_directory_inspection_and_open_is_rejected() {
        let root = std::env::temp_dir().join(format!(
            "unica-staged-directory-swap-{}",
            uuid::Uuid::new_v4()
        ));
        let target = root.join("target");
        let original = root.join("target-original");
        let outside = root.join("outside");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(target.join("inside"), b"inside").unwrap();
        std::fs::write(outside.join("outside"), b"outside").unwrap();
        let target_for_hook = target.clone();
        let original_for_hook = original.clone();
        let outside_for_hook = outside.clone();

        let result = with_tree_open_hook(
            move |path| {
                assert_eq!(path, target_for_hook);
                std::fs::rename(&target_for_hook, &original_for_hook).unwrap();
                std::os::unix::fs::symlink(&outside_for_hook, &target_for_hook).unwrap();
            },
            || TreeSnapshot::capture_target(&target),
        );

        let error = result.expect_err("directory descriptor open must reject the swap");
        assert!(
            error.contains("symbolic link")
                || error.contains("changed")
                || error.contains("identity"),
            "{error}"
        );
        std::fs::remove_file(&target).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
