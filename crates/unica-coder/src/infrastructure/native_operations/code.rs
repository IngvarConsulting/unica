use crate::domain::project_sources::{discover_project_source_map, SourceFormat, SourceSetKind};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::path_policy::WorkspacePathPolicy;
use crate::infrastructure::AdapterOutcome;
use fs2::FileExt;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

/// Resolve the module exactly as [`patch_code`] does, without planning or writing a patch.
///
/// The application layer uses this for support-policy checks. Keeping the resolver here makes
/// those checks and the eventual write agree about source-set format and containment.
pub(crate) fn resolve_module_target(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    resolve_target(args, context).map(|resolved| resolved.target)
}

/// Plan and, unless `dry_run` is set, atomically apply one BSL module patch.
pub(crate) fn patch_code(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    dry_run: bool,
) -> AdapterOutcome {
    match execute_patch(args, context, dry_run) {
        Ok(success) => success.into_outcome(),
        Err(error) => error_outcome(error),
    }
}

/// Attach the optional configured-infobase syntax result to the structured patch details.
///
/// This follow-up is deliberately non-transactional: it never hides or rolls back an already
/// committed source change, and it states that the configured infobase does not yet contain the
/// patched source. Issue #76 owns the later safe build/load workflow.
pub(crate) fn record_platform_syntax_result(
    patch: &mut AdapterOutcome,
    args: &Map<String, Value>,
    syntax: Option<Result<AdapterOutcome, String>>,
    dry_run: bool,
) {
    let requested = args
        .get("platformSyntax")
        .and_then(Value::as_str)
        .unwrap_or("none");
    let mut details = patch
        .stdout
        .as_deref()
        .and_then(|stdout| serde_json::from_str::<Value>(stdout).ok())
        .unwrap_or_else(|| json!({}));

    let syntax_details = if requested == "none" {
        json!({
            "requested": "none",
            "status": "notRequested",
            "scope": Value::Null,
            "validatesPatchedSource": false,
            "validatedPostHash": Value::Null,
            "nonTransactional": true,
            "logPath": Value::Null,
        })
    } else if dry_run {
        json!({
            "requested": requested,
            "status": "skippedDryRun",
            "scope": "configuredInfobase",
            "validatesPatchedSource": false,
            "validatedPostHash": Value::Null,
            "nonTransactional": true,
            "logPath": Value::Null,
        })
    } else if details
        .get("noOp")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        json!({
            "requested": requested,
            "status": "skippedNoOp",
            "scope": "configuredInfobase",
            "validatesPatchedSource": false,
            "validatedPostHash": Value::Null,
            "nonTransactional": true,
            "logPath": Value::Null,
        })
    } else {
        let notice = "platform syntax checks the configured infobase; the patched source is not loaded, so it is not validated until the safe build/load workflow from #76";
        patch.warnings.push(notice.to_string());
        match syntax {
            Some(Ok(runtime)) => {
                let runtime_json = runtime
                    .stdout
                    .as_deref()
                    .and_then(|stdout| serde_json::from_str::<Value>(stdout).ok());
                let diagnostics = runtime_failure_diagnostics(&runtime, runtime_json.as_ref());
                let status = if runtime.ok {
                    "passed"
                } else if contains_any(&diagnostics, &["timed out", "timeout"]) {
                    "timeout"
                } else if contains_any(
                    &diagnostics,
                    &[
                        "failed to spawn process",
                        "executable is unavailable",
                        "runtime adapter is unavailable",
                    ],
                ) {
                    "unavailable"
                } else {
                    "failed"
                };
                if status != "passed" {
                    patch.warnings.push(format!(
                        "configured infobase platform syntax finished with status `{status}`; the source patch remains applied"
                    ));
                }
                let log_path = runtime_json
                    .as_ref()
                    .and_then(find_platform_log_path)
                    .map(Value::String)
                    .unwrap_or(Value::Null);
                json!({
                    "requested": requested,
                    "status": status,
                    "scope": "configuredInfobase",
                    "validatesPatchedSource": false,
                    "validatedPostHash": Value::Null,
                    "nonTransactional": true,
                    "logPath": log_path,
                    "command": runtime.command,
                    "runnerResult": runtime_json,
                    "stderr": runtime.stderr,
                })
            }
            Some(Err(error)) => {
                patch.warnings.push(format!(
                    "configured infobase platform syntax is unavailable: {error}; the source patch remains applied"
                ));
                json!({
                    "requested": requested,
                    "status": "unavailable",
                    "scope": "configuredInfobase",
                    "validatesPatchedSource": false,
                    "validatedPostHash": Value::Null,
                    "nonTransactional": true,
                    "logPath": Value::Null,
                    "error": error,
                })
            }
            None => json!({
                "requested": requested,
                "status": "unavailable",
                "scope": "configuredInfobase",
                "validatesPatchedSource": false,
                "validatedPostHash": Value::Null,
                "nonTransactional": true,
                "logPath": Value::Null,
                "error": "syntax adapter was not invoked",
            }),
        }
    };

    if !details.is_object() {
        details = json!({"patchResult": details});
    }
    details["platformSyntax"] = syntax_details;
    patch.stdout =
        Some(serde_json::to_string_pretty(&details).unwrap_or_else(|_| details.to_string()));
}

fn runtime_failure_diagnostics(runtime: &AdapterOutcome, runtime_json: Option<&Value>) -> String {
    let mut parts = Vec::new();
    parts.extend(runtime.warnings.iter().cloned());
    parts.extend(runtime.errors.iter().cloned());
    if let Some(stderr) = &runtime.stderr {
        parts.push(stderr.clone());
    }
    if let Some(value) = runtime_json {
        collect_runtime_terminal_fields(value, &mut parts);
    }
    parts.join("\n").to_lowercase()
}

fn collect_runtime_terminal_fields(value: &Value, output: &mut Vec<String>) {
    for pointer in [
        "/status",
        "/message",
        "/data/status",
        "/error/status",
        "/error/kind",
        "/error/message",
    ] {
        if let Some(text) = value.pointer(pointer).and_then(Value::as_str) {
            output.push(text.to_string());
        }
    }
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn find_platform_log_path(value: &Value) -> Option<String> {
    match value {
        Value::Object(object) => {
            for key in [
                "platform_log_path",
                "platformLogPath",
                "log_path",
                "logPath",
            ] {
                if let Some(path) = object.get(key).and_then(Value::as_str) {
                    return Some(path.to_string());
                }
            }
            object.values().find_map(find_platform_log_path)
        }
        Value::Array(values) => values.iter().find_map(find_platform_log_path),
        _ => None,
    }
}

fn execute_patch(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    dry_run: bool,
) -> Result<PatchSuccess, String> {
    let resolved = resolve_target(args, context)?;
    let raw = read_stable_target(&resolved.target)?;
    let envelope = SourceEnvelope::parse(&raw)?;
    let request = PatchRequest::parse(args, envelope.eol)?;
    let plan = plan_patch(envelope.text, &request)?;

    let mut post_raw = Vec::with_capacity(envelope.bom_len + plan.post_text.len());
    if envelope.bom_len != 0 {
        post_raw.extend_from_slice(UTF8_BOM);
    }
    post_raw.extend_from_slice(plan.post_text.as_bytes());

    let pre_hash = sha256(&raw);
    let post_hash = sha256(&post_raw);
    let ranges = changed_ranges(&raw, &post_raw, envelope.bom_len, &plan.edits);
    let diff = unified_diff(&resolved.artifact, envelope.text, &plan.post_text);

    let mut warnings = Vec::new();
    if plan.changed && !dry_run {
        let commit = atomic_replace(&resolved.target, &raw, &post_raw, &context.cache_root)?;
        if let Some(warning) = commit.durability_warning {
            warnings.push(warning);
        }
    }

    let syntax_mode = args
        .get("platformSyntax")
        .and_then(Value::as_str)
        .unwrap_or("none");
    let platform_syntax = json!({
        "mode": syntax_mode,
        "status": if syntax_mode == "none" { "notRequested" } else { "notRun" },
        "validatesPatchedSource": false
    });
    let details = json!({
        "target": resolved.artifact,
        "moduleId": resolved.module_id,
        "sourceRoot": resolved.source_root_artifact,
        "sourceSet": resolved.source_set,
        "selector": request.selector_details(),
        "matchCount": plan.match_count,
        "preHash": pre_hash,
        "postHash": post_hash,
        "changedRanges": ranges,
        "diff": diff,
        "dryRun": dry_run,
        "applied": plan.changed && !dry_run,
        "noOp": !plan.changed,
        "platformSyntax": platform_syntax
    });

    Ok(PatchSuccess {
        target: resolved.artifact,
        dry_run,
        changed: plan.changed,
        details,
        warnings,
    })
}

fn read_stable_target(target: &Path) -> Result<Vec<u8>, String> {
    read_stable_target_with(target, || Ok(()))
}

fn read_stable_target_with<AfterRead>(
    target: &Path,
    after_read: AfterRead,
) -> Result<Vec<u8>, String>
where
    AfterRead: FnOnce() -> Result<(), String>,
{
    ensure_stable_regular_target(target)?;
    let raw = fs::read(target)
        .map_err(|error| format!("failed to read {}: {error}", target.display()))?;
    after_read()?;
    ensure_stable_regular_target(target)?;
    Ok(raw)
}

struct PatchSuccess {
    target: String,
    dry_run: bool,
    changed: bool,
    details: Value,
    warnings: Vec<String>,
}

impl PatchSuccess {
    fn into_outcome(self) -> AdapterOutcome {
        let summary = if !self.changed {
            format!(
                "BSL module {} already contains the requested patch",
                self.target
            )
        } else if self.dry_run {
            format!("dry run: BSL module {} would be patched", self.target)
        } else {
            format!("patched BSL module {}", self.target)
        };
        let mut outcome = AdapterOutcome::ok(summary);
        if self.changed {
            outcome.changes.push(if self.dry_run {
                format!("would patch {}", self.target)
            } else {
                format!("patched {}", self.target)
            });
        }
        outcome.artifacts.push(self.target);
        outcome.stdout = Some(
            serde_json::to_string(&self.details)
                .expect("code.patch details contain only serializable values"),
        );
        outcome.warnings.extend(self.warnings);
        outcome
    }
}

fn error_outcome(error: String) -> AdapterOutcome {
    let mut outcome = AdapterOutcome::ok("code patch rejected without changing files");
    outcome.ok = false;
    outcome.errors.push(error);
    outcome
}

#[derive(Debug)]
struct ResolvedTarget {
    target: PathBuf,
    source_set: Option<String>,
    source_root_artifact: String,
    artifact: String,
    module_id: String,
}

fn resolve_target(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<ResolvedTarget, String> {
    let source_set_arg = nonempty_string(args, "sourceSet");
    let source_dir_arg = nonempty_string(args, "sourceDir");
    match (source_set_arg.is_some(), source_dir_arg.is_some()) {
        (true, true) => {
            return Err("exactly one of `sourceSet` or `sourceDir` must be provided".to_string())
        }
        (false, false) => {
            return Err("exactly one of `sourceSet` or `sourceDir` must be provided".to_string())
        }
        _ => {}
    }

    let workspace_canonical = context
        .workspace_root
        .canonicalize()
        .map_err(|error| format!("failed to inspect workspace root: {error}"))?;
    let source_map = discover_project_source_map(&context.workspace_root)?;
    if !source_map.source_sets_from_config {
        return Err(
            "code.patch requires an explicit non-empty `source-set` in v8project.yaml so the mutation can be reconciled by runtime build/dump"
                .to_string(),
        );
    }

    let (source_root, source_set) = if let Some(name) = source_set_arg {
        let matches = source_map
            .source_sets
            .iter()
            .filter(|candidate| candidate.name == name)
            .collect::<Vec<_>>();
        let selected = match matches.as_slice() {
            [] => return Err(format!("source-set `{name}` was not found")),
            [selected] => *selected,
            _ => return Err(format!("source-set name `{name}` is ambiguous")),
        };
        validate_configuration_source_set(selected.kind, selected.source_format, &selected.name)?;
        let root = context.workspace_root.join(&selected.path);
        reject_source_root_symlink(&root)?;
        let root = WorkspacePathPolicy::new(context).resolve_write(root)?;
        ensure_unique_runtime_source_root(&source_map, context, &root)?;
        (root, Some(selected.name.clone()))
    } else {
        let raw = source_dir_arg.expect("sourceDir was checked above");
        let raw_path = PathBuf::from(raw);
        let candidate = if raw_path.is_absolute() {
            raw_path
        } else {
            context.cwd.join(raw_path)
        };
        reject_source_root_symlink(&candidate)?;
        let root = WorkspacePathPolicy::new(context).resolve_write(candidate)?;
        let source_set = validate_direct_source_root(&root, &source_map, context)?;
        (root, source_set)
    };

    let source_root_metadata = fs::metadata(&source_root).map_err(|error| {
        format!(
            "failed to inspect source root {}: {error}",
            source_root.display()
        )
    })?;
    if !source_root_metadata.is_dir() {
        return Err(format!(
            "source root is not a directory: {}",
            source_root.display()
        ));
    }
    let source_root_canonical = source_root.canonicalize().map_err(|error| {
        format!(
            "failed to resolve source root {}: {error}",
            source_root.display()
        )
    })?;
    if !source_root_canonical.starts_with(&workspace_canonical) {
        return Err(format!(
            "source root escapes workspace through a symlink: {}",
            source_root.display()
        ));
    }

    let module_path_raw = required_nonempty_string(args, "modulePath")?;
    let module_path = safe_relative_module_path(module_path_raw)?;
    let target = normalize_lexically(&source_root.join(&module_path));
    WorkspacePathPolicy::new(context).resolve_write(target.clone())?;
    let symlink_metadata = fs::symlink_metadata(&target)
        .map_err(|error| format!("failed to inspect module {}: {error}", target.display()))?;
    if symlink_metadata.file_type().is_symlink() || !symlink_metadata.file_type().is_file() {
        return Err(format!(
            "module must be an existing regular file, not a symlink: {}",
            target.display()
        ));
    }
    let target_canonical = target
        .canonicalize()
        .map_err(|error| format!("failed to resolve module {}: {error}", target.display()))?;
    if !target_canonical.starts_with(&source_root_canonical)
        || !target_canonical.starts_with(&workspace_canonical)
    {
        return Err(format!(
            "module escapes the selected source root or workspace: {}",
            target.display()
        ));
    }

    let source_root_artifact = workspace_relative(context, &source_root)?;
    let artifact = workspace_relative(context, &target)?;
    let module_identity_root = source_set
        .as_deref()
        .unwrap_or(source_root_artifact.as_str());
    let canonical_module_path = target_canonical
        .strip_prefix(&source_root_canonical)
        .map_err(|_| "resolved module is outside the canonical source root".to_string())?;
    let module_id = format!(
        "module:{module_identity_root}:{}",
        path_for_json(canonical_module_path)
    );

    Ok(ResolvedTarget {
        source_root_artifact,
        artifact,
        target: target_canonical,
        source_set,
        module_id,
    })
}

fn reject_source_root_symlink(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect source root {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() {
        Err(format!(
            "source root must not be a symlink: {}",
            path.display()
        ))
    } else {
        Ok(())
    }
}

fn ensure_unique_runtime_source_root(
    source_map: &crate::domain::project_sources::ProjectSourceMap,
    context: &WorkspaceContext,
    selected_root: &Path,
) -> Result<(), String> {
    let selected_root = selected_root.canonicalize().map_err(|error| {
        format!(
            "failed to resolve selected source root {}: {error}",
            selected_root.display()
        )
    })?;
    let owners = source_map
        .source_sets
        .iter()
        .filter(|candidate| {
            context
                .workspace_root
                .join(&candidate.path)
                .canonicalize()
                .is_ok_and(|root| root == selected_root)
        })
        .count();
    if owners == 1 {
        Ok(())
    } else {
        Err(format!(
            "source root {} is assigned to {owners} source-set entries in v8project.yaml",
            selected_root.display()
        ))
    }
}

fn validate_configuration_source_set(
    kind: SourceSetKind,
    format: SourceFormat,
    name: &str,
) -> Result<(), String> {
    if kind != SourceSetKind::Configuration {
        return Err(format!(
            "source-set `{name}` has kind {kind:?}; code.patch currently supports configuration source-sets only"
        ));
    }
    if format != SourceFormat::PlatformXml {
        return Err(format!(
            "source-set `{name}` has sourceFormat={format:?}; code.patch requires platform_xml"
        ));
    }
    Ok(())
}

fn validate_direct_source_root(
    root: &Path,
    source_map: &crate::domain::project_sources::ProjectSourceMap,
    context: &WorkspaceContext,
) -> Result<Option<String>, String> {
    let root_canonical = root
        .canonicalize()
        .map_err(|error| format!("failed to resolve sourceDir {}: {error}", root.display()))?;
    let mut exact_matches = Vec::new();
    for source_set in &source_map.source_sets {
        let candidate = context.workspace_root.join(&source_set.path);
        if candidate
            .canonicalize()
            .is_ok_and(|candidate| candidate == root_canonical)
        {
            exact_matches.push(source_set);
        }
    }
    match exact_matches.as_slice() {
        [source_set] => {
            if source_map
                .source_sets
                .iter()
                .filter(|candidate| candidate.name == source_set.name)
                .count()
                != 1
            {
                return Err(format!(
                    "source-set name `{}` is ambiguous in v8project.yaml",
                    source_set.name
                ));
            }
            validate_configuration_source_set(
                source_set.kind,
                source_set.source_format,
                &source_set.name,
            )?;
            Ok(Some(source_set.name.clone()))
        }
        [] => Err(format!(
            "sourceDir {} must exactly match one configured platform_xml configuration source-set in v8project.yaml",
            root.display()
        )),
        _ => Err(format!(
            "sourceDir {} maps to multiple source-sets",
            root.display()
        )),
    }
}

fn safe_relative_module_path(raw: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    if path.is_absolute() || path.as_os_str().is_empty() {
        return Err("modulePath must be a non-empty relative .bsl path".to_string());
    }
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => return Err(format!("unsafe modulePath `{raw}`")),
        }
    }
    let is_bsl = safe
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("bsl"));
    if !is_bsl {
        return Err("modulePath must identify a .bsl file".to_string());
    }
    Ok(safe)
}

fn workspace_relative(context: &WorkspaceContext, path: &Path) -> Result<String, String> {
    let workspace = normalize_lexically(&context.workspace_root);
    let path = normalize_lexically(path);
    let relative = if let Ok(relative) = path.strip_prefix(&workspace) {
        relative.to_path_buf()
    } else {
        let canonical_workspace = workspace
            .canonicalize()
            .map_err(|error| format!("failed to resolve workspace root: {error}"))?;
        let canonical_path = path
            .canonicalize()
            .map_err(|error| format!("failed to resolve {}: {error}", path.display()))?;
        canonical_path
            .strip_prefix(&canonical_workspace)
            .map(Path::to_path_buf)
            .map_err(|_| format!("path is outside workspace root: {}", path.display()))?
    };
    let relative = path_for_json(&relative);
    Ok(if relative.is_empty() {
        ".".to_string()
    } else {
        relative
    })
}

fn path_for_json(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn nonempty_string<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn required_nonempty_string<'a>(
    args: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
    nonempty_string(args, key).ok_or_else(|| format!("`{key}` must be a non-empty string"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Eol {
    Lf,
    CrLf,
}

impl Eol {
    fn text(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }
}

struct SourceEnvelope<'a> {
    bom_len: usize,
    eol: Eol,
    text: &'a str,
}

impl<'a> SourceEnvelope<'a> {
    fn parse(raw: &'a [u8]) -> Result<Self, String> {
        let bom_len = usize::from(raw.starts_with(UTF8_BOM)) * UTF8_BOM.len();
        let content = &raw[bom_len..];
        if content.starts_with(UTF8_BOM) {
            return Err("module contains a repeated UTF-8 BOM".to_string());
        }
        let text = std::str::from_utf8(content)
            .map_err(|error| format!("module is not valid UTF-8: {error}"))?;
        let mut saw_lf = false;
        let mut saw_crlf = false;
        let bytes = text.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'\r' => {
                    if bytes.get(index + 1) != Some(&b'\n') {
                        return Err("module contains a lone CR line ending".to_string());
                    }
                    saw_crlf = true;
                    index += 2;
                }
                b'\n' => {
                    saw_lf = true;
                    index += 1;
                }
                _ => index += 1,
            }
        }
        if saw_lf && saw_crlf {
            return Err("module contains mixed LF and CRLF line endings".to_string());
        }
        Ok(Self {
            bom_len,
            eol: if saw_crlf { Eol::CrLf } else { Eol::Lf },
            text,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    InsertBefore,
    InsertAfter,
    Replace,
}

#[derive(Debug)]
enum Selector {
    Module,
    Method {
        method_name: String,
    },
    Anchor {
        anchor: String,
        method_name: Option<String>,
    },
}

struct PatchRequest {
    selector: Selector,
    operation: Operation,
    content: String,
    expected_count: usize,
}

impl PatchRequest {
    fn parse(args: &Map<String, Value>, eol: Eol) -> Result<Self, String> {
        let kind = required_nonempty_string(args, "selector")?;
        let parsed_selector = match kind {
            "module" => {
                reject_present(args, "methodName", "module")?;
                reject_present(args, "anchor", "module")?;
                Selector::Module
            }
            "method" => {
                reject_present(args, "anchor", "method")?;
                Selector::Method {
                    method_name: required_nonempty_string(args, "methodName")?.to_string(),
                }
            }
            "anchor" => Selector::Anchor {
                anchor: required_nonempty_string(args, "anchor")?.to_string(),
                method_name: nonempty_string(args, "methodName").map(str::to_string),
            },
            other => return Err(format!("unsupported selector kind `{other}`")),
        };
        let operation = match required_nonempty_string(args, "operation")? {
            "insertBefore" => Operation::InsertBefore,
            "insertAfter" => Operation::InsertAfter,
            "replace" => Operation::Replace,
            other => return Err(format!("unsupported operation `{other}`")),
        };
        let expected_count = args
            .get("expectedCount")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or_else(|| "`expectedCount` must be a positive integer".to_string())?;
        let raw_content = args
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| "`content` must be a string".to_string())?;
        if raw_content.is_empty() {
            return Err("`content` must not be empty".to_string());
        }
        let content = normalize_payload_eol(raw_content, eol);
        if content.is_empty() {
            return Err("`content` must not be empty".to_string());
        }
        if content.contains('\u{feff}') {
            return Err("patch content must not contain a UTF-8 BOM character".to_string());
        }
        Ok(Self {
            selector: parsed_selector,
            operation,
            content,
            expected_count,
        })
    }

    fn selector_details(&self) -> Value {
        match &self.selector {
            Selector::Module => json!({"kind": "module"}),
            Selector::Method { method_name } => {
                json!({"kind": "method", "methodName": method_name})
            }
            Selector::Anchor {
                anchor,
                method_name,
            } => json!({
                "kind": "anchor",
                "anchor": anchor,
                "methodName": method_name,
            }),
        }
    }
}

fn reject_present(args: &Map<String, Value>, key: &str, selector: &str) -> Result<(), String> {
    if args.contains_key(key) {
        Err(format!(
            "selector `{selector}` does not accept argument `{key}`"
        ))
    } else {
        Ok(())
    }
}

fn normalize_payload_eol(content: &str, eol: Eol) -> String {
    let logical = content.replace("\r\n", "\n").replace('\r', "\n");
    if eol == Eol::Lf {
        logical
    } else {
        logical.replace('\n', eol.text())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ByteRange {
    start: usize,
    end: usize,
}

#[derive(Clone, Debug)]
struct Edit {
    start: usize,
    end: usize,
    replacement: String,
}

struct PatchPlan {
    post_text: String,
    edits: Vec<Edit>,
    changed: bool,
    match_count: usize,
}

fn plan_patch(text: &str, request: &PatchRequest) -> Result<PatchPlan, String> {
    let plan = plan_patch_once(text, request)?;
    if plan.changed {
        let repeated = plan_patch_once(&plan.post_text, request).map_err(|error| {
            format!("patch cannot be applied idempotently on the next call: {error}")
        })?;
        if repeated.changed || repeated.post_text != plan.post_text {
            return Err(
                "patch cannot be applied idempotently on the next call: repeated planning would change bytes"
                    .to_string(),
            );
        }
    }
    Ok(plan)
}

fn plan_patch_once(text: &str, request: &PatchRequest) -> Result<PatchPlan, String> {
    let mask = code_mask(text);
    let (ranges, anchor_scopes, is_anchor) = match &request.selector {
        Selector::Module => (
            vec![ByteRange {
                start: 0,
                end: text.len(),
            }],
            Vec::new(),
            false,
        ),
        Selector::Method { method_name } => {
            let methods = scan_methods(text, &mask)?;
            (method_body_ranges(&methods, method_name), Vec::new(), false)
        }
        Selector::Anchor {
            anchor,
            method_name,
        } => {
            let scopes = if let Some(method_name) = method_name {
                let methods = scan_methods(text, &mask)?;
                let scopes = method_body_ranges(&methods, method_name);
                if scopes.is_empty() {
                    return Err(format!("method `{method_name}` was not found"));
                }
                scopes
            } else {
                vec![ByteRange {
                    start: 0,
                    end: text.len(),
                }]
            };
            (
                find_code_occurrences(text, anchor, &mask, &scopes),
                scopes,
                true,
            )
        }
    };

    let (edits, match_count) = if is_anchor {
        plan_anchor_edits(text, ranges, &anchor_scopes, request)?
    } else {
        let match_count = ranges.len();
        if ranges.len() != request.expected_count {
            return Err(cardinality_error(request.expected_count, ranges.len()));
        }
        (
            plan_selected_edits(text, ranges, request, false)?,
            match_count,
        )
    };
    validate_non_overlapping_edits(&edits)?;
    let post_text = apply_edits(text, &edits)?;
    Ok(PatchPlan {
        changed: !edits.is_empty(),
        post_text,
        edits,
        match_count,
    })
}

fn plan_anchor_edits(
    text: &str,
    ranges: Vec<ByteRange>,
    scopes: &[ByteRange],
    request: &PatchRequest,
) -> Result<(Vec<Edit>, usize), String> {
    let Selector::Anchor { anchor, .. } = &request.selector else {
        unreachable!("anchor planner called for another selector")
    };
    if request.operation != Operation::Replace {
        if contains_code_occurrence(&request.content, anchor) {
            return Err(format!(
                "{operation} content contains a code occurrence of the anchor and cannot be proven idempotent",
                operation = match request.operation {
                    Operation::InsertBefore => "insertBefore",
                    Operation::InsertAfter => "insertAfter",
                    Operation::Replace => unreachable!(),
                }
            ));
        }
        if ranges.len() != request.expected_count {
            return Err(cardinality_error(request.expected_count, ranges.len()));
        }
        let match_count = ranges.len();
        return Ok((
            plan_selected_edits(text, ranges, request, true)?,
            match_count,
        ));
    }

    if request.content == *anchor {
        if ranges.len() != request.expected_count {
            return Err(cardinality_error(request.expected_count, ranges.len()));
        }
        return Ok((Vec::new(), ranges.len()));
    }
    if contains_code_occurrence(&request.content, anchor) {
        return Err(
            "anchor replacement content contains a code occurrence of the anchor and cannot be proven idempotent".to_string(),
        );
    }
    let applied_ranges = find_raw_occurrences(text, &request.content, scopes);
    match (ranges.len(), applied_ranges.len()) {
        (old, 0) if old == request.expected_count => Ok((
            ranges
                .into_iter()
                .map(|range| Edit {
                    start: range.start,
                    end: range.end,
                    replacement: request.content.clone(),
                })
                .collect(),
            old,
        )),
        (0, applied) if applied == request.expected_count => Ok((Vec::new(), applied)),
        (old, 0) => Err(cardinality_error(request.expected_count, old)),
        (0, applied) => Err(format!(
            "anchor is absent, but replacement content occurs {applied} time(s); expected {}",
            request.expected_count
        )),
        (old, applied) => Err(format!(
            "mixed patch state: anchor occurs {old} time(s) and replacement content occurs {applied} time(s)"
        )),
    }
}

fn plan_selected_edits(
    text: &str,
    ranges: Vec<ByteRange>,
    request: &PatchRequest,
    anchor_selector: bool,
) -> Result<Vec<Edit>, String> {
    let already_applied = ranges
        .iter()
        .map(|range| match request.operation {
            Operation::Replace => text
                .get(range.start..range.end)
                .is_some_and(|selected| selected == request.content),
            Operation::InsertBefore if anchor_selector => range
                .start
                .checked_sub(request.content.len())
                .and_then(|start| text.get(start..range.start))
                .is_some_and(|adjacent| adjacent == request.content),
            Operation::InsertAfter if anchor_selector => text
                .get(range.end..range.end.saturating_add(request.content.len()))
                .is_some_and(|adjacent| adjacent == request.content),
            Operation::InsertBefore => text
                .get(range.start..range.end)
                .is_some_and(|selected| selected.starts_with(&request.content)),
            Operation::InsertAfter => text
                .get(range.start..range.end)
                .is_some_and(|selected| selected.ends_with(&request.content)),
        })
        .collect::<Vec<_>>();
    if already_applied.iter().all(|applied| *applied) {
        return Ok(Vec::new());
    }
    if already_applied.iter().any(|applied| *applied) {
        return Err(
            "mixed patch state: only some selected locations are already patched".to_string(),
        );
    }

    ranges
        .into_iter()
        .map(|range| match request.operation {
            Operation::InsertBefore => Ok(Edit {
                start: range.start,
                end: range.start,
                replacement: request.content.clone(),
            }),
            Operation::InsertAfter => Ok(Edit {
                start: range.end,
                end: range.end,
                replacement: request.content.clone(),
            }),
            Operation::Replace => Ok(Edit {
                start: range.start,
                end: range.end,
                replacement: request.content.clone(),
            }),
        })
        .collect()
}

fn cardinality_error(expected: usize, actual: usize) -> String {
    format!("selector cardinality mismatch: expected {expected} match(es), found {actual}")
}

fn validate_non_overlapping_edits(edits: &[Edit]) -> Result<(), String> {
    let mut sorted = edits.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|edit| (edit.start, edit.end));
    for pair in sorted.windows(2) {
        let left = pair[0];
        let right = pair[1];
        if left.end > right.start || (left.start == right.start && left.end == right.end) {
            return Err("selected patch locations overlap or are ambiguous".to_string());
        }
    }
    Ok(())
}

fn apply_edits(text: &str, edits: &[Edit]) -> Result<String, String> {
    let mut bytes = text.as_bytes().to_vec();
    let mut descending = edits.iter().collect::<Vec<_>>();
    descending.sort_by(|left, right| {
        right
            .start
            .cmp(&left.start)
            .then_with(|| right.end.cmp(&left.end))
    });
    for edit in descending {
        if edit.start > edit.end
            || edit.end > bytes.len()
            || !text.is_char_boundary(edit.start)
            || !text.is_char_boundary(edit.end)
        {
            return Err("internal patch range is not a valid UTF-8 boundary".to_string());
        }
        bytes.splice(edit.start..edit.end, edit.replacement.bytes());
    }
    String::from_utf8(bytes).map_err(|error| format!("patch produced invalid UTF-8: {error}"))
}

#[derive(Debug)]
struct MethodRange {
    name_folded: String,
    body: ByteRange,
}

#[derive(Debug)]
struct Token {
    folded: String,
    start: usize,
    end: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MethodKind {
    Procedure,
    Function,
}

fn scan_methods(text: &str, mask: &[bool]) -> Result<Vec<MethodRange>, String> {
    let structure_mask = without_preprocessor_directive_lines(text, mask);
    let tokens = identifier_tokens(text, &structure_mask);
    let mut methods = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let Some(kind) = declaration_kind(&tokens[index].folded) else {
            index += 1;
            continue;
        };
        let name = tokens
            .get(index + 1)
            .ok_or_else(|| "method declaration has no name".to_string())?;
        let body_start = declaration_body_start(text, mask, name.end)?;
        let mut end_index = index + 2;
        while end_index < tokens.len() {
            if declaration_kind(&tokens[end_index].folded).is_some()
                && tokens[end_index].start >= body_start
            {
                return Err(format!(
                    "method `{}` has no matching end keyword",
                    name.folded
                ));
            }
            if end_kind(&tokens[end_index].folded).is_some() {
                if end_kind(&tokens[end_index].folded) != Some(kind) {
                    return Err(format!(
                        "method `{}` has a mismatched end keyword",
                        name.folded
                    ));
                }
                break;
            }
            end_index += 1;
        }
        let end = tokens
            .get(end_index)
            .ok_or_else(|| format!("method `{}` has no matching end keyword", name.folded))?;
        if body_start > end.start {
            return Err(format!(
                "method `{}` has an invalid declaration",
                name.folded
            ));
        }
        methods.push(MethodRange {
            name_folded: name.folded.clone(),
            body: ByteRange {
                start: body_start,
                end: method_body_end(text, end.start),
            },
        });
        index = end_index + 1;
    }
    Ok(methods)
}

fn method_body_end(text: &str, end_keyword_start: usize) -> usize {
    let line_start = text[..end_keyword_start]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    if text.as_bytes()[line_start..end_keyword_start]
        .iter()
        .all(|byte| matches!(byte, b' ' | b'\t'))
    {
        line_start
    } else {
        end_keyword_start
    }
}

fn without_preprocessor_directive_lines(text: &str, mask: &[bool]) -> Vec<bool> {
    let mut structure_mask = mask.to_vec();
    let bytes = text.as_bytes();
    let mut line_start = 0;
    while line_start < bytes.len() {
        let line_end = bytes[line_start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|offset| line_start + offset + 1)
            .unwrap_or(bytes.len());
        let first = (line_start..line_end)
            .find(|index| !matches!(bytes[*index], b' ' | b'\t' | b'\r' | b'\n'));
        if first.is_some_and(|index| bytes[index] == b'#' && mask[index]) {
            structure_mask[line_start..line_end].fill(false);
        }
        line_start = line_end;
    }
    structure_mask
}

fn declaration_kind(token: &str) -> Option<MethodKind> {
    match token {
        "procedure" | "процедура" => Some(MethodKind::Procedure),
        "function" | "функция" => Some(MethodKind::Function),
        _ => None,
    }
}

fn end_kind(token: &str) -> Option<MethodKind> {
    match token {
        "endprocedure" | "конецпроцедуры" => Some(MethodKind::Procedure),
        "endfunction" | "конецфункции" => Some(MethodKind::Function),
        _ => None,
    }
}

fn declaration_body_start(text: &str, mask: &[bool], after_name: usize) -> Result<usize, String> {
    let line_end = line_end_after(text, after_name);
    let mut open = None;
    for (index, is_code) in mask
        .iter()
        .copied()
        .enumerate()
        .take(line_end.min(text.len()))
        .skip(after_name)
    {
        if text.as_bytes()[index] == b'(' && is_code {
            open = Some(index);
            break;
        }
    }
    let Some(open) = open else {
        return Ok(line_end);
    };
    let mut depth = 0_usize;
    for (index, is_code) in mask.iter().copied().enumerate().skip(open) {
        if !is_code {
            continue;
        }
        match text.as_bytes()[index] {
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Ok(line_end_after(text, index + 1));
                }
            }
            _ => {}
        }
    }
    Err("method declaration contains an unclosed parameter list".to_string())
}

fn line_end_after(text: &str, start: usize) -> usize {
    text.as_bytes()[start.min(text.len())..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|offset| start.min(text.len()) + offset + 1)
        .unwrap_or(text.len())
}

fn identifier_tokens(text: &str, mask: &[bool]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, character)) = chars.next() {
        if !mask[start] || !identifier_character(character) {
            continue;
        }
        let mut end = start + character.len_utf8();
        while let Some((next, character)) = chars.peek().copied() {
            if !mask[next] || !identifier_character(character) {
                break;
            }
            chars.next();
            end = next + character.len_utf8();
        }
        tokens.push(Token {
            folded: text[start..end].to_lowercase(),
            start,
            end,
        });
    }
    tokens
}

fn identifier_character(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

fn method_body_ranges(methods: &[MethodRange], name: &str) -> Vec<ByteRange> {
    let folded = name.to_lowercase();
    methods
        .iter()
        .filter(|method| method.name_folded == folded)
        .map(|method| method.body.clone())
        .collect()
}

fn code_mask(text: &str) -> Vec<bool> {
    #[derive(Clone, Copy)]
    enum State {
        Code,
        String,
        Comment,
    }
    let bytes = text.as_bytes();
    let mut mask = vec![true; bytes.len()];
    let mut state = State::Code;
    let mut index = 0;
    while index < bytes.len() {
        match state {
            State::Code if bytes[index] == b'"' => {
                mask[index] = false;
                state = State::String;
                index += 1;
            }
            State::Code if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') => {
                mask[index] = false;
                mask[index + 1] = false;
                state = State::Comment;
                index += 2;
            }
            State::Code => index += 1,
            State::Comment if bytes[index] == b'\n' => {
                state = State::Code;
                index += 1;
            }
            State::Comment => {
                mask[index] = false;
                index += 1;
            }
            State::String if bytes[index] == b'"' => {
                mask[index] = false;
                if bytes.get(index + 1) == Some(&b'"') {
                    mask[index + 1] = false;
                    index += 2;
                } else {
                    state = State::Code;
                    index += 1;
                }
            }
            State::String => {
                mask[index] = false;
                index += 1;
            }
        }
    }
    mask
}

fn find_code_occurrences(
    text: &str,
    needle: &str,
    mask: &[bool],
    scopes: &[ByteRange],
) -> Vec<ByteRange> {
    if needle.is_empty() {
        return Vec::new();
    }
    let haystack = text.as_bytes();
    let needle = needle.as_bytes();
    let mut matches = Vec::new();
    for scope in scopes {
        if scope.start > scope.end
            || scope.end > haystack.len()
            || needle.len() > scope.end - scope.start
        {
            continue;
        }
        for start in scope.start..=scope.end - needle.len() {
            if mask.get(start) == Some(&true)
                && haystack.get(start..start + needle.len()) == Some(needle)
            {
                matches.push(ByteRange {
                    start,
                    end: start + needle.len(),
                });
            }
        }
    }
    matches.sort_by_key(|range| (range.start, range.end));
    matches.dedup_by_key(|range| (range.start, range.end));
    matches
}

fn find_raw_occurrences(text: &str, needle: &str, scopes: &[ByteRange]) -> Vec<ByteRange> {
    if needle.is_empty() {
        return Vec::new();
    }
    let haystack = text.as_bytes();
    let needle = needle.as_bytes();
    let mut matches = Vec::new();
    for scope in scopes {
        if scope.start > scope.end
            || scope.end > haystack.len()
            || needle.len() > scope.end - scope.start
        {
            continue;
        }
        let mut start = scope.start;
        while start <= scope.end - needle.len() {
            if haystack.get(start..start + needle.len()) == Some(needle) {
                matches.push(ByteRange {
                    start,
                    end: start + needle.len(),
                });
                start += needle.len();
            } else {
                start += 1;
            }
        }
    }
    matches.sort_by_key(|range| (range.start, range.end));
    matches.dedup_by_key(|range| (range.start, range.end));
    matches
}

fn contains_code_occurrence(text: &str, needle: &str) -> bool {
    let scopes = [ByteRange {
        start: 0,
        end: text.len(),
    }];
    !find_code_occurrences(text, needle, &code_mask(text), &scopes).is_empty()
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn changed_ranges(raw_before: &[u8], raw_after: &[u8], bom_len: usize, edits: &[Edit]) -> Value {
    let mut sorted = edits.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|edit| (edit.start, edit.end));
    let mut delta = 0_isize;
    Value::Array(
        sorted
            .into_iter()
            .map(|edit| {
                let pre_start = bom_len + edit.start;
                let pre_end = bom_len + edit.end;
                let post_start = (pre_start as isize + delta) as usize;
                let post_end = post_start + edit.replacement.len();
                delta += edit.replacement.len() as isize - (edit.end - edit.start) as isize;
                json!({
                    "pre": {
                        "byteStart": pre_start,
                        "byteEnd": pre_end,
                        "lineStart": line_number(raw_before, pre_start),
                        "lineEnd": line_number(raw_before, pre_end)
                    },
                    "post": {
                        "byteStart": post_start,
                        "byteEnd": post_end,
                        "lineStart": line_number(raw_after, post_start),
                        "lineEnd": line_number(raw_after, post_end)
                    }
                })
            })
            .collect(),
    )
}

fn line_number(bytes: &[u8], offset: usize) -> usize {
    1 + bytes[..offset.min(bytes.len())]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiffLine {
    text: String,
    terminated: bool,
}

fn unified_diff(path: &str, before: &str, after: &str) -> String {
    if before == after {
        return String::new();
    }
    let before = split_diff_lines(before);
    let after = split_diff_lines(after);
    let mut prefix = 0;
    while prefix < before.len() && prefix < after.len() && before[prefix] == after[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < before.len() - prefix
        && suffix < after.len() - prefix
        && before[before.len() - 1 - suffix] == after[after.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let removed = &before[prefix..before.len() - suffix];
    let added = &after[prefix..after.len() - suffix];
    let old_start = if removed.is_empty() {
        prefix
    } else {
        prefix + 1
    };
    let new_start = if added.is_empty() { prefix } else { prefix + 1 };
    let mut diff = format!(
        "--- a/{path}\n+++ b/{path}\n@@ -{old_start},{} +{new_start},{} @@\n",
        removed.len(),
        added.len()
    );
    append_diff_lines(&mut diff, '-', removed);
    append_diff_lines(&mut diff, '+', added);
    diff
}

fn split_diff_lines(text: &str) -> Vec<DiffLine> {
    let logical = text.replace("\r\n", "\n");
    if logical.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in logical.bytes().enumerate() {
        if byte == b'\n' {
            lines.push(DiffLine {
                text: logical[start..index].to_string(),
                terminated: true,
            });
            start = index + 1;
        }
    }
    if start < logical.len() {
        lines.push(DiffLine {
            text: logical[start..].to_string(),
            terminated: false,
        });
    }
    lines
}

fn append_diff_lines(output: &mut String, prefix: char, lines: &[DiffLine]) {
    for line in lines {
        output.push(prefix);
        output.push_str(&line.text);
        output.push('\n');
        if !line.terminated {
            output.push_str("\\ No newline at end of file\n");
        }
    }
}

fn atomic_replace(
    target: &Path,
    expected: &[u8],
    replacement: &[u8],
    cache_root: &Path,
) -> Result<AtomicReplaceSuccess, String> {
    atomic_replace_with(
        target,
        expected,
        replacement,
        cache_root,
        AtomicReplaceHooks {
            temp_path: unique_temp_path,
            stage: write_staging_file,
            before_commit: || Ok(()),
            replace: replace_path_atomically,
            sync_parent: sync_parent_directory,
        },
    )
}

#[derive(Debug)]
struct AtomicReplaceSuccess {
    durability_warning: Option<String>,
}

struct AtomicReplaceHooks<TempPath, Stage, BeforeCommit, Replace, SyncParent> {
    temp_path: TempPath,
    stage: Stage,
    before_commit: BeforeCommit,
    replace: Replace,
    sync_parent: SyncParent,
}

fn atomic_replace_with<TempPath, Stage, BeforeCommit, Replace, SyncParent>(
    target: &Path,
    expected: &[u8],
    replacement: &[u8],
    cache_root: &Path,
    hooks: AtomicReplaceHooks<TempPath, Stage, BeforeCommit, Replace, SyncParent>,
) -> Result<AtomicReplaceSuccess, String>
where
    TempPath: FnOnce(&Path, &Path) -> PathBuf,
    Stage: FnOnce(&mut fs::File, &Path, &[u8], &fs::Permissions) -> Result<(), String>,
    BeforeCommit: FnOnce() -> Result<(), String>,
    Replace: FnOnce(&Path, &Path) -> Result<(), String>,
    SyncParent: FnOnce(&Path) -> Result<(), String>,
{
    let AtomicReplaceHooks {
        temp_path,
        stage,
        before_commit,
        replace,
        sync_parent,
    } = hooks;
    let lock_dir = cache_root.join("locks").join("code-patch");
    fs::create_dir_all(&lock_dir)
        .map_err(|error| format!("failed to create patch lock directory: {error}"))?;
    // `resolve_target` passes the already-validated canonical identity. Never adopt a fresh
    // canonical destination here: an attacker could swap the path to an out-of-workspace symlink
    // between planning and locking. `ensure_stable_regular_target` requires the live identity to
    // remain exactly this path.
    let commit_target = normalize_lexically(target);
    let lock_key = sha256(commit_target.to_string_lossy().as_bytes())
        .trim_start_matches("sha256:")
        .to_string();
    let lock_path = lock_dir.join(format!("{lock_key}.lock"));
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| format!("failed to open patch lock {}: {error}", lock_path.display()))?;
    lock.lock_exclusive()
        .map_err(|error| format!("failed to acquire patch lock: {error}"))?;

    ensure_stable_regular_target(&commit_target)?;
    let current = fs::read(&commit_target).map_err(|error| {
        format!(
            "failed to re-read {} under lock: {error}",
            commit_target.display()
        )
    })?;
    if current != expected {
        return Err(format!(
            "module changed concurrently after planning; refusing to overwrite {}",
            commit_target.display()
        ));
    }
    let metadata = fs::symlink_metadata(&commit_target).map_err(|error| {
        format!(
            "failed to inspect {} under lock: {error}",
            commit_target.display()
        )
    })?;
    let parent = commit_target
        .parent()
        .ok_or_else(|| "module path has no parent directory".to_string())?;
    let temp = temp_path(parent, &commit_target);
    let mut owns_temp = false;
    let mut committed = false;
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|error| {
                format!("failed to create staging file {}: {error}", temp.display())
            })?;
        owns_temp = true;
        stage(&mut file, &temp, replacement, &metadata.permissions())?;
        drop(file);
        before_commit()?;

        // The staging write can take long enough for a non-cooperating editor or VCS process to
        // update the module. Re-check both path identity and bytes immediately before rename.
        ensure_stable_regular_target(&commit_target)?;
        let current = fs::read(&commit_target).map_err(|error| {
            format!(
                "failed to perform final pre-commit read of {}: {error}",
                commit_target.display()
            )
        })?;
        if current != expected {
            return Err(format!(
                "module changed concurrently while staging; refusing to overwrite {}",
                commit_target.display()
            ));
        }

        replace(&temp, &commit_target)?;
        committed = true;
        let durability_warning = sync_parent(parent).err().map(|error| {
            format!("module change was committed, but directory durability sync failed: {error}")
        });
        Ok(AtomicReplaceSuccess { durability_warning })
    })();
    if !committed && owns_temp {
        if let Err(cleanup_error) = cleanup_staging_file(&temp) {
            return match result {
                Ok(_) => Err(cleanup_error),
                Err(error) => Err(format!(
                    "{error}; additionally failed to clean staging file: {cleanup_error}"
                )),
            };
        }
    }
    result
}

fn cleanup_staging_file(temp: &Path) -> Result<(), String> {
    #[cfg(windows)]
    if let Ok(metadata) = fs::metadata(temp) {
        let mut permissions = metadata.permissions();
        if permissions.readonly() {
            permissions.set_readonly(false);
            fs::set_permissions(temp, permissions).map_err(|error| {
                format!(
                    "failed to make staging file removable {}: {error}",
                    temp.display()
                )
            })?;
        }
    }
    match fs::remove_file(temp) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to remove staging file {}: {error}",
            temp.display()
        )),
    }
}

fn write_staging_file(
    file: &mut fs::File,
    temp: &Path,
    replacement: &[u8],
    permissions: &fs::Permissions,
) -> Result<(), String> {
    file.write_all(replacement)
        .map_err(|error| format!("failed to write staging file: {error}"))?;
    file.flush()
        .map_err(|error| format!("failed to flush staging file: {error}"))?;
    fs::set_permissions(temp, permissions.clone())
        .map_err(|error| format!("failed to preserve module permissions: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("failed to sync staging file: {error}"))
}

fn ensure_stable_regular_target(target: &Path) -> Result<(), String> {
    let current_identity = target
        .canonicalize()
        .map_err(|error| format!("failed to resolve module during patch: {error}"))?;
    if current_identity != target {
        return Err(
            "module path identity changed after validation; refusing to commit".to_string(),
        );
    }
    let metadata = fs::symlink_metadata(target)
        .map_err(|error| format!("failed to inspect module during patch: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err("module ceased to be a regular file before commit".to_string());
    }
    Ok(())
}

fn unique_temp_path(parent: &Path, target: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Module.bsl");
    parent.join(format!(".{name}.unica-{}-{nanos}.tmp", std::process::id()))
}

#[cfg(not(windows))]
fn replace_path_atomically(source: &Path, target: &Path) -> Result<(), String> {
    fs::rename(source, target)
        .map_err(|error| format!("failed to atomically replace {}: {error}", target.display()))
}

#[cfg(windows)]
fn replace_path_atomically(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both pointers reference NUL-terminated UTF-16 buffers for the duration of the call.
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(format!(
            "failed to atomically replace module: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> Result<(), String> {
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("failed to sync module directory: {error}"))
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn dry_run_skips_requested_platform_syntax() {
        let mut patch = AdapterOutcome::ok("preview");
        patch.stdout = Some("{}".to_string());
        let args = json!({"platformSyntax": "configuredInfobase"})
            .as_object()
            .unwrap()
            .clone();

        record_platform_syntax_result(&mut patch, &args, None, true);

        let details: Value = serde_json::from_str(patch.stdout.as_ref().unwrap()).unwrap();
        assert_eq!(details["platformSyntax"]["status"], "skippedDryRun");
        assert_eq!(details["platformSyntax"]["validatesPatchedSource"], false);
        assert!(patch.warnings.is_empty());
    }

    #[test]
    fn syntax_failure_is_non_transactional_and_preserves_patch_success() {
        let mut patch = AdapterOutcome::ok("patched");
        patch.changes.push("updated Module.bsl".to_string());
        patch.stdout = Some(r#"{"postHash":"sha256:after"}"#.to_string());
        let args = json!({"platformSyntax": "configuredInfobase"})
            .as_object()
            .unwrap()
            .clone();
        let runtime = AdapterOutcome {
            ok: false,
            summary: "syntax failed".to_string(),
            changes: Vec::new(),
            warnings: vec![
                "internal v8-runner runtime adapter exited with status exit status: 1".to_string(),
            ],
            errors: vec!["syntax error".to_string()],
            artifacts: Vec::new(),
            stdout: Some(
                r#"{"data":{"status":"failed","platform_log_path":"/tmp/1cv8.log"}}"#.to_string(),
            ),
            stderr: Some("syntax error".to_string()),
            command: Some(vec!["v8-runner".to_string(), "syntax".to_string()]),
        };

        record_platform_syntax_result(&mut patch, &args, Some(Ok(runtime)), false);

        let details: Value = serde_json::from_str(patch.stdout.as_ref().unwrap()).unwrap();
        assert!(patch.ok);
        assert_eq!(patch.changes, ["updated Module.bsl"]);
        assert_eq!(details["postHash"], "sha256:after");
        assert_eq!(details["platformSyntax"]["status"], "failed");
        assert_eq!(details["platformSyntax"]["logPath"], "/tmp/1cv8.log");
        assert!(details["platformSyntax"]["nonTransactional"]
            .as_bool()
            .unwrap());
        assert!(patch
            .warnings
            .iter()
            .any(|warning| warning.contains("not loaded")));
    }

    #[test]
    fn platform_syntax_reports_passed_timeout_unavailable_and_noop_terminal_states() {
        let args = json!({"platformSyntax": "configuredInfobase"})
            .as_object()
            .unwrap()
            .clone();

        let mut passed = patch_result_for_syntax(false);
        record_platform_syntax_result(
            &mut passed,
            &args,
            Some(Ok(runtime_outcome(
                true,
                r#"{"data":{"status":"clean","platform_log_path":"/tmp/timeout-case/clean.log"}}"#,
            ))),
            false,
        );
        let passed_details = details(&passed);
        assert_eq!(passed_details["platformSyntax"]["status"], "passed");
        assert_eq!(
            passed_details["platformSyntax"]["logPath"],
            "/tmp/timeout-case/clean.log"
        );

        let mut timeout = patch_result_for_syntax(false);
        record_platform_syntax_result(
            &mut timeout,
            &args,
            Some(Ok(runtime_outcome(
                false,
                r#"{"error":{"kind":"ToolFailed","message":"operation timeout"}}"#,
            ))),
            false,
        );
        assert_eq!(details(&timeout)["platformSyntax"]["status"], "timeout");
        assert!(timeout.ok);

        let mut issues_found = patch_result_for_syntax(false);
        record_platform_syntax_result(
            &mut issues_found,
            &args,
            Some(Ok(runtime_outcome(
                false,
                r#"{"data":{"status":"issues_found","issues":[{"message":"Timeout setting was not found"}]}}"#,
            ))),
            false,
        );
        assert_eq!(details(&issues_found)["platformSyntax"]["status"], "failed");

        let mut unavailable = patch_result_for_syntax(false);
        record_platform_syntax_result(
            &mut unavailable,
            &args,
            Some(Err("v8-runner executable is unavailable".to_string())),
            false,
        );
        assert_eq!(
            details(&unavailable)["platformSyntax"]["status"],
            "unavailable"
        );
        assert!(unavailable.ok);

        let mut no_op = patch_result_for_syntax(true);
        record_platform_syntax_result(&mut no_op, &args, None, false);
        assert_eq!(details(&no_op)["platformSyntax"]["status"], "skippedNoOp");
    }

    #[test]
    fn inserts_seven_lines_into_bom_only_module_and_repeats_as_noop() {
        let fixture = Fixture::new("bom-only", UTF8_BOM);
        let content = (1..=7)
            .map(|number| format!("Line{number} = {number};\n"))
            .collect::<String>();
        let args = fixture.args(json!({"kind": "module"}), "insertAfter", &content, 1);

        let first = patch_code(&args, &fixture.context, false);
        assert!(first.ok, "{:?}", first.errors);
        assert_eq!(
            fs::read(&fixture.module).unwrap(),
            [UTF8_BOM, content.as_bytes()].concat()
        );
        let first_details = details(&first);
        assert_eq!(first_details["applied"], true);
        assert_eq!(first_details["preHash"], sha256(UTF8_BOM));

        let second = patch_code(&args, &fixture.context, false);
        assert!(second.ok, "{:?}", second.errors);
        assert!(second.changes.is_empty());
        assert_eq!(details(&second)["noOp"], true);
    }

    #[test]
    fn inserts_exactly_seven_lines_before_one_anchor_and_repeats_as_noop() {
        let fixture = Fixture::new(
            "seven-before-anchor",
            b"Procedure Work()\n    Target();\nEndProcedure\n",
        );
        let content = (1..=7)
            .map(|number| format!("    Step{number}();\n"))
            .collect::<String>();
        let args = fixture.args(
            json!({"kind": "anchor", "anchor": "    Target();", "methodName": "Work"}),
            "insertBefore",
            &content,
            1,
        );

        let first = patch_code(&args, &fixture.context, false);
        assert!(first.ok, "{:?}", first.errors);
        let expected = format!("Procedure Work()\n{content}    Target();\nEndProcedure\n");
        assert_eq!(fs::read_to_string(&fixture.module).unwrap(), expected);

        let bytes_after_first = fs::read(&fixture.module).unwrap();
        let second = patch_code(&args, &fixture.context, false);
        assert!(second.ok, "{:?}", second.errors);
        assert_eq!(details(&second)["noOp"], true);
        assert_eq!(fs::read(&fixture.module).unwrap(), bytes_after_first);
    }

    #[test]
    fn rejects_anchor_insertions_that_create_an_extra_anchor_in_payload_or_boundary() {
        for (name, source, anchor, content) in [
            ("payload-before", "TARGET", "TARGET", "TARGET\n"),
            ("payload-after", "TARGET", "TARGET", "TARGET\n"),
            ("boundary-before", "ABA", "ABA", "AB"),
            ("boundary-after", "ABA", "ABA", "BA"),
            ("comment-before", "TARGET", "TARGET", "// disabled "),
            ("string-before", "TARGET", "TARGET", "\""),
        ] {
            let fixture = Fixture::new(name, source.as_bytes());
            let before = fs::read(&fixture.module).unwrap();
            let operation = if name.ends_with("before") {
                "insertBefore"
            } else {
                "insertAfter"
            };
            let args = fixture.args(
                json!({"kind": "anchor", "anchor": anchor}),
                operation,
                content,
                1,
            );

            let outcome = patch_code(&args, &fixture.context, false);

            assert!(!outcome.ok, "{name} unexpectedly succeeded");
            assert!(outcome.errors[0].contains("idempotent"));
            assert_eq!(fs::read(&fixture.module).unwrap(), before);
        }
    }

    #[test]
    fn anchor_replace_with_comment_string_or_overlapping_content_repeats_as_noop() {
        for (name, source, anchor, content) in [
            ("comment-replacement", "MARK", "MARK", "// Done\n"),
            ("string-replacement", "MARK", "MARK", "\"Done\""),
            ("overlap-replacement", "AX", "X", "AA"),
        ] {
            let fixture = Fixture::new(name, source.as_bytes());
            let args = fixture.args(
                json!({"kind": "anchor", "anchor": anchor}),
                "replace",
                content,
                1,
            );

            let first = patch_code(&args, &fixture.context, false);
            assert!(first.ok, "{name}: {:?}", first.errors);
            let bytes = fs::read(&fixture.module).unwrap();
            let second = patch_code(&args, &fixture.context, false);
            assert!(second.ok, "{name}: {:?}", second.errors);
            assert_eq!(details(&second)["noOp"], true);
            assert_eq!(details(&second)["matchCount"], 1);
            assert_eq!(fs::read(&fixture.module).unwrap(), bytes);
        }
    }

    #[test]
    fn module_selector_can_replace_text_with_an_unclosed_method() {
        let fixture = Fixture::new("repair-malformed-module", b"Procedure Broken()\n");
        let args = fixture.args(json!({"kind": "module"}), "replace", "Fixed = True;\n", 1);

        let outcome = patch_code(&args, &fixture.context, false);

        assert!(outcome.ok, "{:?}", outcome.errors);
        assert_eq!(
            fs::read_to_string(&fixture.module).unwrap(),
            "Fixed = True;\n"
        );
    }

    #[test]
    fn rejects_method_patch_that_would_destabilize_method_boundaries() {
        let source = b"Procedure Work()\n    Old();\nEndProcedure\n";
        let fixture = Fixture::new("unstable-method-boundary", source);
        let args = fixture.args(
            json!({"kind": "method", "methodName": "Work"}),
            "replace",
            "EndProcedure\n",
            1,
        );

        let outcome = patch_code(&args, &fixture.context, false);

        assert!(!outcome.ok);
        assert!(outcome.errors[0].contains("idempotently"));
        assert_eq!(fs::read(&fixture.module).unwrap(), source);
    }

    #[test]
    fn preserves_lf_crlf_bom_and_missing_terminal_newline() {
        for (name, bom, eol) in [
            ("lf", false, "\n"),
            ("crlf", false, "\r\n"),
            ("crlf-bom", true, "\r\n"),
        ] {
            let mut original = Vec::new();
            if bom {
                original.extend_from_slice(UTF8_BOM);
            }
            original.extend_from_slice(format!("A = 1;{eol}TARGET{eol}Tail = 2;").as_bytes());
            let fixture = Fixture::new(name, &original);
            let args = fixture.args(
                json!({"kind": "anchor", "anchor": "TARGET"}),
                "insertBefore",
                "One();\nTwo();\n",
                1,
            );
            let outcome = patch_code(&args, &fixture.context, false);
            assert!(outcome.ok, "{:?}", outcome.errors);
            let patched = fs::read(&fixture.module).unwrap();
            assert_eq!(patched.starts_with(UTF8_BOM), bom);
            let text = std::str::from_utf8(&patched[usize::from(bom) * 3..]).unwrap();
            assert!(text.contains(&format!("One();{eol}Two();{eol}TARGET")));
            assert!(text.ends_with("Tail = 2;"));
            if eol == "\r\n" {
                assert!(!text.replace("\r\n", "").contains('\n'));
            }
        }
    }

    #[test]
    fn anchor_ignores_string_comment_decoys_and_can_be_scoped_to_russian_method() {
        let source = concat!(
            "// MARK\n",
            "Text = \"MARK\";\n",
            "Процедура Цель()\n",
            "    MARK\n",
            "КонецПроцедуры\n",
            "Procedure Other()\n",
            "    MARK\n",
            "EndProcedure\n"
        );
        let fixture = Fixture::new("decoys", source.as_bytes());
        let args = fixture.args(
            json!({"kind": "anchor", "anchor": "MARK", "methodName": "цЕЛЬ"}),
            "replace",
            "Done();",
            1,
        );
        let outcome = patch_code(&args, &fixture.context, false);
        assert!(outcome.ok, "{:?}", outcome.errors);
        let patched = fs::read_to_string(&fixture.module).unwrap();
        assert!(patched.contains("// MARK"));
        assert!(patched.contains("\"MARK\""));
        assert!(patched.contains("    Done();"));
        assert!(patched.contains("Procedure Other()\n    MARK"));

        let repeated = patch_code(&args, &fixture.context, false);
        assert!(repeated.ok, "{:?}", repeated.errors);
        assert_eq!(details(&repeated)["noOp"], true);
    }

    #[test]
    fn cardinality_zero_and_two_never_write() {
        for (name, source, anchor) in [
            ("zero", "A = 1;\n", "MISSING"),
            ("two", "MARK\nMARK\n", "MARK"),
        ] {
            let fixture = Fixture::new(name, source.as_bytes());
            let before = fs::read(&fixture.module).unwrap();
            let args = fixture.args(
                json!({"kind": "anchor", "anchor": anchor}),
                "insertAfter",
                "Changed();",
                1,
            );
            let outcome = patch_code(&args, &fixture.context, false);
            assert!(!outcome.ok);
            assert!(outcome.errors[0].contains("cardinality"));
            assert_eq!(fs::read(&fixture.module).unwrap(), before);
        }
    }

    #[test]
    fn supports_all_operations_and_method_body_only_replacement() {
        let source = "Procedure Work(Value) Export\n    Old();\nEndProcedure\n";
        let fixture = Fixture::new("all-operations", source.as_bytes());
        let before = fixture.args(
            json!({"kind": "anchor", "anchor": "Old();"}),
            "insertBefore",
            "Before();\n    ",
            1,
        );
        assert!(patch_code(&before, &fixture.context, false).ok);
        assert_eq!(
            fs::read_to_string(&fixture.module).unwrap(),
            "Procedure Work(Value) Export\n    Before();\n    Old();\nEndProcedure\n"
        );
        let after = fixture.args(
            json!({"kind": "anchor", "anchor": "Old();"}),
            "insertAfter",
            "\n    After();",
            1,
        );
        assert!(patch_code(&after, &fixture.context, false).ok);
        assert_eq!(
            fs::read_to_string(&fixture.module).unwrap(),
            concat!(
                "Procedure Work(Value) Export\n",
                "    Before();\n",
                "    Old();\n",
                "    After();\n",
                "EndProcedure\n"
            )
        );
        let replace = fixture.args(
            json!({"kind": "method", "methodName": "WORK"}),
            "replace",
            "    NewBody();\n",
            1,
        );
        assert!(patch_code(&replace, &fixture.context, false).ok);
        let patched = fs::read_to_string(&fixture.module).unwrap();
        assert_eq!(
            patched,
            "Procedure Work(Value) Export\n    NewBody();\nEndProcedure\n"
        );
    }

    #[test]
    fn dry_run_has_real_diff_and_hashes_without_touching_bytes() {
        let fixture = Fixture::new("dry-run", b"A = 1;\nTARGET\n");
        let before = fs::read(&fixture.module).unwrap();
        let args = fixture.args(
            json!({"kind": "anchor", "anchor": "TARGET"}),
            "replace",
            "Changed();",
            1,
        );
        let outcome = patch_code(&args, &fixture.context, true);
        assert!(outcome.ok, "{:?}", outcome.errors);
        assert_eq!(fs::read(&fixture.module).unwrap(), before);
        let details = details(&outcome);
        assert_eq!(details["dryRun"], true);
        assert_eq!(details["applied"], false);
        assert_eq!(
            details["moduleId"],
            "module:main:CommonModules/Test/Ext/Module.bsl"
        );
        assert_eq!(details["selector"]["kind"], "anchor");
        assert_eq!(details["matchCount"], 1);
        assert_ne!(details["preHash"], details["postHash"]);
        assert!(details["diff"].as_str().unwrap().contains("+Changed();"));
        assert_eq!(details["changedRanges"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn changed_ranges_report_exact_bom_cyrillic_byte_and_line_offsets() {
        let text = "Процедура Тест()\r\n    Цель();\r\nКонецПроцедуры\r\n";
        let source = [UTF8_BOM, text.as_bytes()].concat();
        let fixture = Fixture::new("cyrillic-offsets", &source);
        let args = fixture.args(
            json!({"kind": "anchor", "anchor": "Цель();", "methodName": "Тест"}),
            "replace",
            "Готово();",
            1,
        );

        let outcome = patch_code(&args, &fixture.context, true);

        assert!(outcome.ok, "{:?}", outcome.errors);
        assert_eq!(fs::read(&fixture.module).unwrap(), source);
        let details = details(&outcome);
        let range = &details["changedRanges"][0];
        assert_eq!(range["pre"]["byteStart"], 38);
        assert_eq!(range["pre"]["byteEnd"], 49);
        assert_eq!(range["pre"]["lineStart"], 2);
        assert_eq!(range["pre"]["lineEnd"], 2);
        assert_eq!(range["post"]["byteStart"], 38);
        assert_eq!(range["post"]["byteEnd"], 53);
        assert_eq!(range["post"]["lineStart"], 2);
        assert_eq!(range["post"]["lineEnd"], 2);
        assert!(details["diff"].as_str().unwrap().contains("+    Готово();"));
    }

    #[test]
    fn accepts_absolute_source_dir_for_platform_xml_configuration() {
        let fixture = Fixture::new("absolute-source-dir", b"TARGET\n");
        let mut args = fixture.args(
            json!({"kind": "anchor", "anchor": "TARGET"}),
            "replace",
            "Changed();",
            1,
        );
        args.remove("sourceSet");
        args.insert(
            "sourceDir".to_string(),
            json!(fixture.root.join("src").display().to_string()),
        );

        let outcome = patch_code(&args, &fixture.context, true);
        assert!(outcome.ok, "{:?}", outcome.errors);
        let details = details(&outcome);
        assert_eq!(details["sourceSet"], "main");
        assert_eq!(details["sourceRoot"], "src");
        assert_eq!(fs::read(&fixture.module).unwrap(), b"TARGET\n");
    }

    #[test]
    fn rejects_unconfigured_source_dir_in_preview_and_apply() {
        let fixture = Fixture::new("unconfigured-source-dir", b"CONFIGURED\n");
        let direct_root = fixture.root.join("direct");
        let direct_module = direct_root.join("CommonModules/Test/Ext/Module.bsl");
        fs::create_dir_all(direct_module.parent().unwrap()).unwrap();
        fs::write(direct_root.join("Configuration.xml"), "<Configuration/>").unwrap();
        fs::write(&direct_module, b"TARGET\n").unwrap();
        let mut args = fixture.args(
            json!({"kind": "anchor", "anchor": "TARGET"}),
            "replace",
            "Changed();",
            1,
        );
        args.remove("sourceSet");
        args.insert(
            "sourceDir".to_string(),
            json!(direct_root.display().to_string()),
        );

        for dry_run in [true, false] {
            let outcome = patch_code(&args, &fixture.context, dry_run);
            assert!(!outcome.ok, "unconfigured sourceDir unexpectedly passed");
            assert!(outcome
                .errors
                .join("\n")
                .contains("configured platform_xml"));
            assert_eq!(fs::read(&direct_module).unwrap(), b"TARGET\n");
        }
    }

    #[test]
    fn rejects_autodetected_source_set_without_authoritative_runtime_config() {
        for (name, project_config) in [
            ("no-project-config", None),
            ("empty-project-source-sets", Some("format: DESIGNER\n")),
        ] {
            let fixture = Fixture::new(name, b"TARGET\n");
            match project_config {
                Some(config) => fs::write(fixture.root.join("v8project.yaml"), config).unwrap(),
                None => fs::remove_file(fixture.root.join("v8project.yaml")).unwrap(),
            }
            let args = fixture.args(
                json!({"kind": "anchor", "anchor": "TARGET"}),
                "replace",
                "Changed();",
                1,
            );

            for dry_run in [true, false] {
                let outcome = patch_code(&args, &fixture.context, dry_run);
                assert!(!outcome.ok, "autodetected source-set unexpectedly passed");
                assert!(outcome
                    .errors
                    .join("\n")
                    .contains("explicit non-empty `source-set`"));
                assert_eq!(fs::read(&fixture.module).unwrap(), b"TARGET\n");
            }
        }
    }

    #[test]
    fn rejects_direct_root_when_source_set_name_is_duplicated_elsewhere() {
        let fixture = Fixture::new("duplicate-source-set-name", b"TARGET\n");
        let other = fixture.root.join("other");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("Configuration.xml"), "<Configuration/>").unwrap();
        fs::write(
            fixture.root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: main\n    type: CONFIGURATION\n    path: other\n",
        )
        .unwrap();
        let mut args = fixture.args(
            json!({"kind": "anchor", "anchor": "TARGET"}),
            "replace",
            "Changed();",
            1,
        );
        args.remove("sourceSet");
        args.insert(
            "sourceDir".to_string(),
            json!(fixture.root.join("src").display().to_string()),
        );

        let outcome = patch_code(&args, &fixture.context, true);
        assert!(!outcome.ok);
        assert!(outcome.errors.join("\n").contains("ambiguous"));
        assert_eq!(fs::read(&fixture.module).unwrap(), b"TARGET\n");
    }

    #[test]
    fn rejects_source_set_when_canonical_root_is_assigned_twice() {
        let fixture = Fixture::new("duplicate-source-set-root", b"TARGET\n");
        fs::write(
            fixture.root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: alias\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let args = fixture.args(
            json!({"kind": "anchor", "anchor": "TARGET"}),
            "replace",
            "Changed();",
            1,
        );

        for dry_run in [true, false] {
            let outcome = patch_code(&args, &fixture.context, dry_run);
            assert!(!outcome.ok);
            assert!(outcome.errors.join("\n").contains("assigned to 2"));
            assert_eq!(fs::read(&fixture.module).unwrap(), b"TARGET\n");
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_internal_source_dir_symlink_in_preview_and_apply() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("source-dir-symlink", b"TARGET\n");
        let alias = fixture.root.join("source-alias");
        symlink(fixture.root.join("src"), &alias).unwrap();
        let mut args = fixture.args(
            json!({"kind": "anchor", "anchor": "TARGET"}),
            "replace",
            "Changed();",
            1,
        );
        args.remove("sourceSet");
        args.insert("sourceDir".to_string(), json!(alias.display().to_string()));

        for dry_run in [true, false] {
            let outcome = patch_code(&args, &fixture.context, dry_run);
            assert!(!outcome.ok);
            assert!(outcome.errors.join("\n").contains("must not be a symlink"));
            assert_eq!(fs::read(&fixture.module).unwrap(), b"TARGET\n");
        }
    }

    #[test]
    fn rejects_edt_source_set_without_writing() {
        let fixture = Fixture::new("edt-source-set", b"TARGET\n");
        fs::remove_file(fixture.root.join("src/Configuration.xml")).unwrap();
        fs::create_dir_all(fixture.root.join("src/Configuration")).unwrap();
        fs::write(
            fixture.root.join("src/Configuration/Configuration.mdo"),
            "edt",
        )
        .unwrap();
        let before = fs::read(&fixture.module).unwrap();
        let args = fixture.args(
            json!({"kind": "anchor", "anchor": "TARGET"}),
            "replace",
            "Changed();",
            1,
        );

        let outcome = patch_code(&args, &fixture.context, false);
        assert!(!outcome.ok);
        assert!(outcome.errors[0].contains("platform_xml"));
        assert_eq!(fs::read(&fixture.module).unwrap(), before);
    }

    #[test]
    fn rejects_invalid_utf8_and_repeated_bom_without_writing() {
        for (name, source) in [
            ("invalid-utf8", vec![0xff, b'X']),
            ("repeated-bom", [UTF8_BOM, UTF8_BOM, b"TARGET"].concat()),
        ] {
            let fixture = Fixture::new(name, &source);
            let args = fixture.args(json!({"kind": "module"}), "replace", "Changed();", 1);
            let outcome = patch_code(&args, &fixture.context, false);
            assert!(!outcome.ok);
            assert_eq!(fs::read(&fixture.module).unwrap(), source);
        }
    }

    #[test]
    fn accepts_nonempty_whitespace_content_and_repeats_safely() {
        let fixture = Fixture::new("whitespace-content", b"TARGET");
        let args = fixture.args(
            json!({"kind": "anchor", "anchor": "TARGET"}),
            "insertBefore",
            "   ",
            1,
        );
        assert!(patch_code(&args, &fixture.context, false).ok);
        assert_eq!(fs::read(&fixture.module).unwrap(), b"   TARGET");
        let repeated = patch_code(&args, &fixture.context, false);
        assert!(repeated.ok, "{:?}", repeated.errors);
        assert_eq!(details(&repeated)["noOp"], true);
    }

    #[test]
    fn method_selector_preserves_annotation_and_multiline_function_signature() {
        let source = concat!(
            "&AtServer\n",
            "Function Compute(\n",
            "    Value,\n",
            "    Other = Call()\n",
            ") Export\n",
            "    Text = \"EndFunction\";\n",
            "    Return Value;\n",
            "EndFunction\n"
        );
        let fixture = Fixture::new("multiline-signature", source.as_bytes());
        let args = fixture.args(
            json!({"kind": "method", "methodName": "compute"}),
            "replace",
            "    Return Other;\n",
            1,
        );
        let outcome = patch_code(&args, &fixture.context, false);
        assert!(outcome.ok, "{:?}", outcome.errors);
        let patched = fs::read_to_string(&fixture.module).unwrap();
        assert_eq!(
            patched,
            concat!(
                "&AtServer\n",
                "Function Compute(\n",
                "    Value,\n",
                "    Other = Call()\n",
                ") Export\n",
                "    Return Other;\n",
                "EndFunction\n"
            )
        );
    }

    #[test]
    fn method_scanner_ignores_preprocessor_directive_names_that_look_like_methods() {
        let source = concat!(
            "#Region Procedure\n",
            "Procedure Work()\n",
            "#Region EndProcedure\n",
            "    Old();\n",
            "#EndRegion\n",
            "EndProcedure\n",
            "#EndRegion\n"
        );
        let fixture = Fixture::new("preprocessor-method-decoys", source.as_bytes());
        let args = fixture.args(
            json!({"kind": "method", "methodName": "Work"}),
            "replace",
            "    New();\n",
            1,
        );

        let outcome = patch_code(&args, &fixture.context, false);

        assert!(outcome.ok, "{:?}", outcome.errors);
        assert_eq!(
            fs::read_to_string(&fixture.module).unwrap(),
            concat!(
                "#Region Procedure\n",
                "Procedure Work()\n",
                "    New();\n",
                "EndProcedure\n",
                "#EndRegion\n"
            )
        );
        let repeated = patch_code(&args, &fixture.context, false);
        assert!(repeated.ok, "{:?}", repeated.errors);
        assert_eq!(details(&repeated)["noOp"], true);
    }

    #[test]
    fn method_selector_preserves_indentation_before_the_end_keyword() {
        let source = concat!(
            "#If Server Then\n",
            "    Procedure Work()\n",
            "        Old();\n",
            "    EndProcedure\n",
            "#EndIf\n"
        );
        for (name, operation, content, expected) in [
            (
                "indented-method-replace",
                "replace",
                "        New();\n",
                concat!(
                    "#If Server Then\n",
                    "    Procedure Work()\n",
                    "        New();\n",
                    "    EndProcedure\n",
                    "#EndIf\n"
                ),
            ),
            (
                "indented-method-insert-after",
                "insertAfter",
                "        Tail();\n",
                concat!(
                    "#If Server Then\n",
                    "    Procedure Work()\n",
                    "        Old();\n",
                    "        Tail();\n",
                    "    EndProcedure\n",
                    "#EndIf\n"
                ),
            ),
        ] {
            let fixture = Fixture::new(name, source.as_bytes());
            let args = fixture.args(
                json!({"kind": "method", "methodName": "Work"}),
                operation,
                content,
                1,
            );

            let first = patch_code(&args, &fixture.context, false);
            assert!(first.ok, "{name}: {:?}", first.errors);
            assert_eq!(fs::read_to_string(&fixture.module).unwrap(), expected);
            let second = patch_code(&args, &fixture.context, false);
            assert!(second.ok, "{name}: {:?}", second.errors);
            assert_eq!(details(&second)["noOp"], true);
        }
    }

    #[test]
    fn rejects_mixed_and_lone_cr_line_endings_without_writing() {
        for (name, source) in [
            ("mixed", b"A\r\nTARGET\n".as_slice()),
            ("lone-cr", b"A\rTARGET".as_slice()),
        ] {
            let fixture = Fixture::new(name, source);
            let before = fs::read(&fixture.module).unwrap();
            let args = fixture.args(
                json!({"kind": "anchor", "anchor": "TARGET"}),
                "replace",
                "Changed();",
                1,
            );
            let outcome = patch_code(&args, &fixture.context, false);
            assert!(!outcome.ok);
            assert_eq!(fs::read(&fixture.module).unwrap(), before);
        }
    }

    #[test]
    fn rejects_traversal_and_non_bsl_targets() {
        let fixture = Fixture::new("traversal", b"TARGET\n");
        let mut traversal = fixture.args(json!({"kind": "module"}), "replace", "Changed();", 1);
        traversal.insert("modulePath".to_string(), json!("../outside.bsl"));
        assert!(!patch_code(&traversal, &fixture.context, false).ok);
        traversal.insert("modulePath".to_string(), json!("Module.txt"));
        assert!(!patch_code(&traversal, &fixture.context, false).ok);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_module_symlink_even_when_it_points_inside_source_root() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new("symlink", b"TARGET\n");
        let real = fixture.module.with_file_name("Real.bsl");
        fs::rename(&fixture.module, &real).unwrap();
        symlink(&real, &fixture.module).unwrap();
        let args = fixture.args(json!({"kind": "module"}), "replace", "Changed();", 1);
        let outcome = patch_code(&args, &fixture.context, false);
        assert!(!outcome.ok);
        assert_eq!(fs::read_to_string(real).unwrap(), "TARGET\n");
    }

    #[cfg(unix)]
    #[test]
    fn canonicalizes_a_symlinked_parent_before_locking_and_committing() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new("symlink-parent", b"TARGET\n");
        let alias = fixture.root.join("src/CommonModules/Alias");
        symlink(fixture.root.join("src/CommonModules/Test"), &alias).unwrap();
        let mut args = fixture.args(
            json!({"kind": "anchor", "anchor": "TARGET"}),
            "replace",
            "Changed();",
            1,
        );
        args.insert(
            "modulePath".to_string(),
            json!("CommonModules/Alias/Ext/Module.bsl"),
        );

        let resolved = resolve_target(&args, &fixture.context).unwrap();
        assert_eq!(resolved.target, fixture.module.canonicalize().unwrap());
        let outcome = patch_code(&args, &fixture.context, false);

        assert!(outcome.ok, "{:?}", outcome.errors);
        assert_eq!(fs::read_to_string(&fixture.module).unwrap(), "Changed();\n");
        let details = details(&outcome);
        assert_eq!(details["target"], "src/CommonModules/Alias/Ext/Module.bsl");
        assert_eq!(
            details["moduleId"],
            "module:main:CommonModules/Test/Ext/Module.bsl"
        );
    }

    #[cfg(unix)]
    #[test]
    fn stable_read_rejects_identity_drift_before_exposing_preview_bytes() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new("preview-identity-swap", b"Before\n");
        let target = fixture.module.canonicalize().unwrap();
        let outside = fixture.root.parent().unwrap().join(format!(
            "unica-code-patch-preview-outside-{}",
            std::process::id()
        ));
        fs::write(&outside, b"Secret\n").unwrap();
        let swapped_target = target.clone();
        let symlink_target = outside.clone();

        let error = read_stable_target_with(&target, || {
            fs::remove_file(&swapped_target)
                .map_err(|error| format!("failed to remove preview target: {error}"))?;
            symlink(&symlink_target, &swapped_target)
                .map_err(|error| format!("failed to inject preview symlink swap: {error}"))
        })
        .unwrap_err();

        assert!(error.contains("identity changed"));
        assert_eq!(fs::read(&outside).unwrap(), b"Secret\n");
        fs::remove_file(outside).unwrap();
    }

    #[test]
    fn atomic_replace_rejects_stale_preimage() {
        let fixture = Fixture::new("cas", b"Before\n");
        let target = fixture.module.canonicalize().unwrap();
        let stale = fs::read(&fixture.module).unwrap();
        fs::write(&fixture.module, b"Concurrent\n").unwrap();
        let error = atomic_replace(
            &target,
            &stale,
            b"Replacement\n",
            &fixture.context.cache_root,
        )
        .unwrap_err();
        assert!(error.contains("changed concurrently"));
        assert_eq!(fs::read(&fixture.module).unwrap(), b"Concurrent\n");
    }

    #[test]
    fn atomic_replace_rechecks_preimage_after_staging_and_cleans_temp() {
        let fixture = Fixture::new("cas-after-staging", b"Before\n");
        let target = fixture.module.canonicalize().unwrap();
        let expected = fs::read(&fixture.module).unwrap();
        let module = target.clone();

        let error = atomic_replace_with(
            &target,
            &expected,
            b"Replacement\n",
            &fixture.context.cache_root,
            AtomicReplaceHooks {
                temp_path: unique_temp_path,
                stage: write_staging_file,
                before_commit: || {
                    fs::write(&module, b"Concurrent\n")
                        .map_err(|error| format!("injected concurrent write failed: {error}"))
                },
                replace: replace_path_atomically,
                sync_parent: sync_parent_directory,
            },
        )
        .unwrap_err();

        assert!(error.contains("while staging"));
        assert_eq!(fs::read(&fixture.module).unwrap(), b"Concurrent\n");
        assert!(patch_staging_files(&fixture.module).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_replace_rejects_identity_swap_to_outside_symlink() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new("identity-swap", b"Before\n");
        let target = fixture.module.canonicalize().unwrap();
        let expected = fs::read(&target).unwrap();
        let outside = fixture
            .root
            .parent()
            .unwrap()
            .join(format!("unica-code-patch-outside-{}", std::process::id()));
        fs::write(&outside, &expected).unwrap();
        let swapped_target = target.clone();
        let symlink_target = outside.clone();

        let error = atomic_replace_with(
            &target,
            &expected,
            b"Replacement\n",
            &fixture.context.cache_root,
            AtomicReplaceHooks {
                temp_path: unique_temp_path,
                stage: write_staging_file,
                before_commit: || {
                    fs::remove_file(&swapped_target)
                        .map_err(|error| format!("failed to remove target for swap: {error}"))?;
                    symlink(&symlink_target, &swapped_target)
                        .map_err(|error| format!("failed to inject symlink swap: {error}"))
                },
                replace: replace_path_atomically,
                sync_parent: sync_parent_directory,
            },
        )
        .unwrap_err();

        assert!(error.contains("identity changed"));
        assert_eq!(fs::read(&outside).unwrap(), expected);
        assert!(patch_staging_files(&fixture.module).is_empty());
        fs::remove_file(outside).unwrap();
    }

    #[test]
    fn atomic_replace_cleans_temp_after_injected_staging_and_replace_failures() {
        for failure in ["staging", "replace"] {
            let fixture = Fixture::new(failure, b"Before\n");
            let target = fixture.module.canonicalize().unwrap();
            let expected = fs::read(&fixture.module).unwrap();
            let result = if failure == "staging" {
                atomic_replace_with(
                    &target,
                    &expected,
                    b"Replacement\n",
                    &fixture.context.cache_root,
                    AtomicReplaceHooks {
                        temp_path: unique_temp_path,
                        stage: |file: &mut fs::File, _: &Path, _: &[u8], _: &fs::Permissions| {
                            file.write_all(b"partial").map_err(|error| {
                                format!("failed to inject staging write: {error}")
                            })?;
                            Err("injected staging failure".to_string())
                        },
                        before_commit: || Ok(()),
                        replace: replace_path_atomically,
                        sync_parent: sync_parent_directory,
                    },
                )
            } else {
                atomic_replace_with(
                    &target,
                    &expected,
                    b"Replacement\n",
                    &fixture.context.cache_root,
                    AtomicReplaceHooks {
                        temp_path: unique_temp_path,
                        stage: write_staging_file,
                        before_commit: || Ok(()),
                        replace: |_: &Path, _: &Path| Err("injected replace failure".to_string()),
                        sync_parent: sync_parent_directory,
                    },
                )
            };

            let error = result.unwrap_err();
            assert!(error.contains(failure));
            assert_eq!(fs::read(&fixture.module).unwrap(), expected);
            assert!(patch_staging_files(&fixture.module).is_empty());
        }
    }

    #[test]
    fn atomic_replace_does_not_delete_a_preexisting_unowned_temp_path() {
        let fixture = Fixture::new("foreign-temp", b"Before\n");
        let target = fixture.module.canonicalize().unwrap();
        let expected = fs::read(&target).unwrap();
        let foreign = target
            .parent()
            .unwrap()
            .join(".Module.bsl.unica-foreign.tmp");
        fs::write(&foreign, b"foreign").unwrap();
        let injected_path = foreign.clone();

        let error = atomic_replace_with(
            &target,
            &expected,
            b"Replacement\n",
            &fixture.context.cache_root,
            AtomicReplaceHooks {
                temp_path: move |_: &Path, _: &Path| injected_path,
                stage: write_staging_file,
                before_commit: || Ok(()),
                replace: replace_path_atomically,
                sync_parent: sync_parent_directory,
            },
        )
        .unwrap_err();

        assert!(error.contains("failed to create staging file"));
        assert_eq!(fs::read(&foreign).unwrap(), b"foreign");
        assert_eq!(fs::read(&target).unwrap(), expected);
        fs::remove_file(foreign).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn cleanup_staging_file_removes_inherited_readonly_file_on_windows() {
        let fixture = Fixture::new("readonly-staging-cleanup", b"Before\n");
        let temp = fixture
            .module
            .with_file_name(".Module.bsl.unica-readonly.tmp");
        fs::write(&temp, b"partial").unwrap();
        let mut permissions = fs::metadata(&temp).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&temp, permissions).unwrap();

        cleanup_staging_file(&temp).unwrap();

        assert!(!temp.exists());
    }

    #[test]
    fn post_rename_sync_failure_is_committed_success_with_warning() {
        let fixture = Fixture::new("post-rename-sync", b"Before\n");
        let target = fixture.module.canonicalize().unwrap();
        let expected = fs::read(&fixture.module).unwrap();

        let success = atomic_replace_with(
            &target,
            &expected,
            b"Replacement\n",
            &fixture.context.cache_root,
            AtomicReplaceHooks {
                temp_path: unique_temp_path,
                stage: write_staging_file,
                before_commit: || Ok(()),
                replace: replace_path_atomically,
                sync_parent: |_: &Path| Err("injected directory fsync failure".to_string()),
            },
        )
        .unwrap();

        assert_eq!(fs::read(&fixture.module).unwrap(), b"Replacement\n");
        assert!(success
            .durability_warning
            .as_deref()
            .is_some_and(|warning| warning.contains("committed")));
        assert!(patch_staging_files(&fixture.module).is_empty());
    }

    #[test]
    fn anchor_replace_rejects_mixed_partial_state() {
        let fixture = Fixture::new("mixed-state", b"MARK\nDone();\n");
        let args = fixture.args(
            json!({"kind": "anchor", "anchor": "MARK"}),
            "replace",
            "Done();",
            2,
        );
        let before = fs::read(&fixture.module).unwrap();
        let outcome = patch_code(&args, &fixture.context, false);
        assert!(!outcome.ok);
        assert!(outcome.errors[0].contains("mixed patch state"));
        assert_eq!(fs::read(&fixture.module).unwrap(), before);
    }

    fn details(outcome: &AdapterOutcome) -> Value {
        serde_json::from_str(outcome.stdout.as_deref().unwrap()).unwrap()
    }

    fn patch_result_for_syntax(no_op: bool) -> AdapterOutcome {
        let mut outcome = AdapterOutcome::ok("patched");
        outcome.stdout = Some(
            json!({
                "applied": !no_op,
                "noOp": no_op,
                "postHash": "sha256:after"
            })
            .to_string(),
        );
        outcome
    }

    fn runtime_outcome(ok: bool, stdout: &str) -> AdapterOutcome {
        AdapterOutcome {
            ok,
            summary: "platform syntax".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: if ok {
                Vec::new()
            } else {
                vec!["platform syntax failed".to_string()]
            },
            artifacts: Vec::new(),
            stdout: Some(stdout.to_string()),
            stderr: None,
            command: Some(vec!["v8-runner".to_string(), "syntax".to_string()]),
        }
    }

    fn patch_staging_files(target: &Path) -> Vec<PathBuf> {
        let prefix = format!(".{}.unica-", target.file_name().unwrap().to_string_lossy());
        fs::read_dir(target.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
            })
            .collect()
    }

    struct Fixture {
        root: PathBuf,
        context: WorkspaceContext,
        module: PathBuf,
    }

    impl Fixture {
        fn new(name: &str, source: &[u8]) -> Self {
            let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "unica-code-patch-{name}-{}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(root.join("src/CommonModules/Test/Ext")).unwrap();
            fs::write(
                root.join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
            fs::write(root.join("src/Configuration.xml"), "<Configuration/>").unwrap();
            let module = root.join("src/CommonModules/Test/Ext/Module.bsl");
            fs::write(&module, source).unwrap();
            let context = WorkspaceContext::discover(root.clone()).unwrap();
            Self {
                root,
                context,
                module,
            }
        }

        fn args(
            &self,
            selector: Value,
            operation: &str,
            content: &str,
            expected_count: usize,
        ) -> Map<String, Value> {
            let selector = selector
                .as_object()
                .expect("test selector must use the compact object helper");
            let mut args = json!({
                "sourceSet": "main",
                "modulePath": "CommonModules/Test/Ext/Module.bsl",
                "selector": selector.get("kind").expect("selector kind"),
                "operation": operation,
                "content": content,
                "expectedCount": expected_count,
                "platformSyntax": "none"
            })
            .as_object()
            .unwrap()
            .clone();
            for key in ["methodName", "anchor"] {
                if let Some(value) = selector.get(key) {
                    args.insert(key.to_string(), value.clone());
                }
            }
            args
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn source_set_selector_preserves_significant_whitespace() {
        let fixture = Fixture::new("source-set-identity", b"Procedure Test()\nEndProcedure\n");
        fs::write(
            fixture.root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: \" main \"\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let args = json!({
            "sourceSet": " main ",
            "modulePath": "CommonModules/Test/Ext/Module.bsl"
        })
        .as_object()
        .unwrap()
        .clone();

        let resolved = resolve_target(&args, &fixture.context).unwrap();

        assert_eq!(resolved.source_set.as_deref(), Some(" main "));
        assert_eq!(
            resolved.module_id,
            "module: main :CommonModules/Test/Ext/Module.bsl"
        );
    }
}
