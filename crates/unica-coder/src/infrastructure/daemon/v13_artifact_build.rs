#![allow(clippy::result_large_err)]
//! `artifact.build` — сборка файла конфигурации `.cf` или расширения `.cfe`
//! из исходников рабочего пространства силами `v8-runner make` (A-7 зонтика
//! #871). В отличие от `cf.export`, который выносит конфигурацию из базы, тут
//! источник — исходники, а база служит раннеру временной площадкой.
//!
//! Аргументы закрыты: `output` — относительный путь к `.cf` или `.cfe` внутри
//! рабочего пространства; `sourceSet` — имя объявленного набора, если их
//! несколько; `extension` — имя расширения в базе, обязательное для `.cfe` и
//! недопустимое для `.cf`. Внешние обработки и отчёты (`.epf`/`.erf`) за
//! словарём: у них выход — каталог публикации, а не файл, и это другой
//! контракт квитанции.
//!
//! Превью зовёт `make --dry-run`: раннер называет вид артефакта, набор и
//! выход, ничего не публикуя. Раннер принимает выход и вне проекта — Unica
//! ограничивает его рабочим пространством до вызова. Применение повторяет
//! превью, сверяет забор ревизии, собирает и снимает с выхода независимую
//! квитанцию: размер и дайджест файла. Путь к платформе наружу не идёт.

use super::protocol::InvocationRequest;
use super::v13_infobase_exports::{
    closed_workspace_relative_path, digest_optional_workspace_file, digest_required_workspace_file,
    map_runner_code, valid_1c_identifier, CONFIG_NAME, LOCAL_CONFIG_NAME, RUNNER_OUTPUT_LIMIT,
};
use crate::application::invocation_store::ToolIdentity;
use crate::domain::cancellation::CancellationToken;
use crate::domain::invocation::{DomainResult, SafeIdentityHash};
use crate::domain::refusal::RefusalCode;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bundled_tools::{
    bundled_tool_version, resolve_bundled_tool, BundledTool,
};
use crate::infrastructure::internal_adapters::{
    ProcessCommand, ProcessOutput, ProcessRunner, SystemProcessRunner,
};
use crate::infrastructure::path_policy::WorkspacePathPolicy;
use crate::infrastructure::plugin_runtime::find_plugin_root;
use crate::infrastructure::redaction::redactor;
use crate::infrastructure::source_roots::normalize_path_identity;
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) const OPERATION: &str = "artifact.build";
/// Имя команды в конверте раннера: словарь читается как слой и направление,
/// раннер называет свои команды по-своему.
const RUNNER_COMMAND: &str = "make";
const SOURCE_SET_NAME_MAX: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactKind {
    Cf,
    Cfe,
}

impl ArtifactKind {
    const fn suffix(self) -> &'static str {
        match self {
            Self::Cf => "cf",
            Self::Cfe => "cfe",
        }
    }

    /// Как раннер называет вид артефакта в `mode` и `payload.artifact_type`.
    const fn runner_mode(self) -> &'static str {
        match self {
            Self::Cf => "configuration_cf",
            Self::Cfe => "extension_cfe",
        }
    }
}

#[derive(Debug, Clone)]
struct BuildArguments {
    kind: ArtifactKind,
    source_set: Option<String>,
    extension: Option<String>,
    output_relative: PathBuf,
    output: PathBuf,
}

#[derive(Debug, Clone)]
pub(super) struct PreparedArtifactBuild {
    arguments: BuildArguments,
    dry_run: bool,
    if_rev: Option<String>,
    context: WorkspaceContext,
}

pub(super) enum Preparation {
    NotApplicable,
    Rejected(Box<DomainResult>),
    Ready(Arc<PreparedArtifactBuild>),
}

pub(super) fn prepare(request: &InvocationRequest) -> Preparation {
    if request.tool() != ToolIdentity::Run
        || request.arguments().get("op").and_then(Value::as_str) != Some(OPERATION)
    {
        return Preparation::NotApplicable;
    }
    match PreparedArtifactBuild::parse(request) {
        Ok(prepared) => Preparation::Ready(Arc::new(prepared)),
        Err(result) => Preparation::Rejected(Box::new(result)),
    }
}

impl PreparedArtifactBuild {
    fn parse(request: &InvocationRequest) -> Result<Self, DomainResult> {
        let arguments = request.arguments();
        let args = arguments
            .get("args")
            .and_then(Value::as_object)
            .ok_or_else(|| reject(RefusalCode::BadValue, "run args must be an object"))?;
        let dry_run = arguments
            .get("dryRun")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                reject(
                    RefusalCode::BadValue,
                    "artifact.build requires dryRun: true to preview or dryRun: false with ifRev to apply",
                )
            })?;
        let if_rev = match arguments.get("ifRev") {
            None => None,
            Some(Value::String(value)) if !value.trim().is_empty() => Some(value.clone()),
            Some(_) => {
                return Err(reject(
                    RefusalCode::BadValue,
                    "artifact.build ifRev must be non-empty text",
                ))
            }
        };
        if dry_run && if_rev.is_some() {
            return Err(reject(
                RefusalCode::BadValue,
                "artifact.build preview does not accept ifRev; apply the revision returned by this preview",
            ));
        }
        if !dry_run && if_rev.is_none() {
            return Err(reject(
                RefusalCode::BadValue,
                "artifact.build apply requires ifRev from a prior dryRun preview",
            ));
        }
        let context =
            discover_workspace(Some(PathBuf::from(request.workspace_hint()))).map_err(|error| {
                reject(
                    RefusalCode::ProviderUnavailable,
                    format!("workspace discovery failed: {error}"),
                )
            })?;
        let arguments = parse_build_arguments(args, &context)?;
        Ok(Self {
            arguments,
            dry_run,
            if_rev,
            context,
        })
    }

    pub(super) fn workspace_identity_hash(&self) -> SafeIdentityHash {
        let mut hasher = Sha256::new();
        hasher.update(b"unica-v13-artifact-build-workspace-v1\0");
        hasher.update(self.context.workspace_root.as_os_str().as_encoded_bytes());
        SafeIdentityHash::from_sha256(hasher.finalize().into())
    }

    pub(super) fn execute(&self, cancellation: CancellationToken) -> DomainResult {
        execute_with_runner(self, &SystemProcessRunner, cancellation)
    }
}

fn parse_build_arguments(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<BuildArguments, DomainResult> {
    const ACCEPTED: [&str; 3] = ["output", "sourceSet", "extension"];
    if let Some(unknown) = args.keys().find(|key| !ACCEPTED.contains(&key.as_str())) {
        return Err(reject(
            RefusalCode::BadValue,
            format!("artifact.build does not accept `{unknown}`; the closed args are output, sourceSet and extension"),
        ));
    }
    let source_set = match args.get("sourceSet") {
        None => None,
        Some(Value::String(value)) if valid_source_set_name(value) => Some(value.clone()),
        Some(_) => {
            return Err(reject(
                RefusalCode::BadValue,
                format!("artifact.build sourceSet must be the name of a source set declared in v8project.yaml: up to {SOURCE_SET_NAME_MAX} letters, digits, `_`, `-` or `.`"),
            ))
        }
    };
    let extension = match args.get("extension") {
        None => None,
        Some(Value::String(value)) if valid_1c_identifier(value) => Some(value.clone()),
        Some(_) => {
            return Err(reject(
                RefusalCode::BadValue,
                "artifact.build extension must be a non-empty 1C identifier",
            ))
        }
    };
    let output = args
        .get("output")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            reject(
                RefusalCode::BadValue,
                "artifact.build output must be non-empty text",
            )
        })?;
    let output_relative = closed_workspace_relative_path(output).map_err(|message| {
        reject(
            RefusalCode::BadValue,
            format!("artifact.build output {message}"),
        )
    })?;
    let kind = match output_relative
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("cf") => ArtifactKind::Cf,
        Some("cfe") => ArtifactKind::Cfe,
        Some("epf" | "erf") => {
            return Err(reject(
                RefusalCode::UnsupportedOperation,
                "artifact.build builds .cf and .cfe files; external processors and reports publish into a directory and are not published in v0.13",
            ))
        }
        _ => {
            return Err(reject(
                RefusalCode::BadValue,
                "artifact.build output must end in .cf for a configuration or .cfe for an extension",
            ))
        }
    };
    match (kind, extension.is_some()) {
        (ArtifactKind::Cfe, false) => {
            return Err(reject(
                RefusalCode::BadValue,
                "artifact.build output .cfe requires extension: the name the infobase knows it by",
            ))
        }
        (ArtifactKind::Cf, true) => return Err(reject(
            RefusalCode::BadValue,
            "artifact.build extension is only for a .cfe output; a .cf is the main configuration",
        )),
        _ => {}
    }
    let root_context = WorkspaceContext {
        cwd: context.workspace_root.clone(),
        workspace_root: context.workspace_root.clone(),
        cache_root: context.cache_root.clone(),
        workspace_epoch: context.workspace_epoch,
    };
    // Раннер принимает выход и вне проекта; поверхность — нет: та же политика,
    // что у выгрузок, путь внутри пространства и без ссылок.
    let output_path = WorkspacePathPolicy::new(&root_context)
        .resolve_write(&output_relative)
        .map_err(|error| reject(RefusalCode::BadValue, error))?;
    Ok(BuildArguments {
        kind,
        source_set,
        extension,
        output_relative,
        output: output_path,
    })
}

fn valid_source_set_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= SOURCE_SET_NAME_MAX
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        && !value.starts_with('.')
}

/// Проектный файл с локальным дополнением, объявленный состав и состояние
/// выхода: план действителен, пока они прежние. Содержимое исходников забором
/// не держится: сборка читает их целиком, и другой результат — другой файл,
/// а не отказ.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StableInputs {
    config_sha256: String,
    local_config_sha256: Option<String>,
    declared: Vec<String>,
    output_sha256: Option<String>,
}

fn capture_inputs(prepared: &PreparedArtifactBuild) -> Result<StableInputs, DomainResult> {
    let root = &prepared.context.workspace_root;
    let config_sha256 =
        digest_required_workspace_file(root, Path::new(CONFIG_NAME)).map_err(|error| {
            let mut result = reject(RefusalCode::InvalidState, error);
            result.next.push(json!({
                "tool": "unica.view",
                "args": {},
                "reason": "inspect workspace setup and the required v8project.yaml recipe"
            }));
            result
        })?;
    let local_config_sha256 = digest_optional_workspace_file(root, Path::new(LOCAL_CONFIG_NAME))
        .map_err(|error| reject(RefusalCode::InvalidState, error))?
        .map(|(digest, _)| digest);
    let declared = declared_source_sets(&root.join(CONFIG_NAME))?;
    if let Some(source_set) = &prepared.arguments.source_set {
        if !declared.contains(source_set) {
            return Err(reject(
                RefusalCode::BadValue,
                format!(
                    "artifact.build sourceSet `{source_set}` is not declared in v8project.yaml; declared source sets: {}",
                    declared.join(", ")
                ),
            ));
        }
    }
    let output_sha256 = digest_optional_workspace_file(root, &prepared.arguments.output_relative)
        .map_err(|error| reject(RefusalCode::BadValue, error))?
        .map(|(digest, _)| digest);
    Ok(StableInputs {
        config_sha256,
        local_config_sha256,
        declared,
        output_sha256,
    })
}

fn declared_source_sets(config: &Path) -> Result<Vec<String>, DomainResult> {
    let text = std::fs::read_to_string(config)
        .map_err(|error| reject(RefusalCode::InvalidState, format!("{CONFIG_NAME}: {error}")))?;
    let document: serde_yaml::Value = serde_yaml::from_str(&text).map_err(|error| {
        reject(
            RefusalCode::InvalidState,
            format!(
                "{CONFIG_NAME} is not valid YAML: {}",
                redactor(&error.to_string())
            ),
        )
    })?;
    let mut names = Vec::new();
    for entry in document
        .get("source-set")
        .and_then(serde_yaml::Value::as_sequence)
        .into_iter()
        .flatten()
    {
        let Some(name) = entry.get("name").and_then(serde_yaml::Value::as_str) else {
            return Err(reject(
                RefusalCode::InvalidState,
                format!("{CONFIG_NAME} declares a source-set without a name"),
            ));
        };
        names.push(name.to_string());
    }
    if names.is_empty() {
        return Err(reject(
            RefusalCode::InvalidState,
            format!("{CONFIG_NAME} declares no source-set; there is nothing to build"),
        ));
    }
    Ok(names)
}

fn execute_with_runner(
    prepared: &PreparedArtifactBuild,
    runner: &dyn ProcessRunner,
    cancellation: CancellationToken,
) -> DomainResult {
    let plugin_root = match find_plugin_root(&prepared.context.cwd) {
        Some(root) => root,
        None => {
            return reject(
                RefusalCode::ProviderUnavailable,
                "Unica plugin root could not be located for the bundled v8-runner",
            )
        }
    };
    let tool = match resolve_bundled_tool(&plugin_root, "v8-runner", true) {
        Ok(tool) => tool,
        Err(error) => return reject(RefusalCode::ProviderUnavailable, redactor(&error)),
    };
    let runner_version = match bundled_tool_version(&plugin_root, "v8-runner") {
        Ok(version) => version,
        Err(error) => return reject(RefusalCode::ProviderUnavailable, redactor(&error)),
    };
    execute_with_resolved_runner(prepared, runner, cancellation, &tool, &runner_version)
}

fn execute_with_resolved_runner(
    prepared: &PreparedArtifactBuild,
    runner: &dyn ProcessRunner,
    cancellation: CancellationToken,
    tool: &BundledTool,
    runner_version: &str,
) -> DomainResult {
    if cancellation.is_cancelled() {
        return reject(
            RefusalCode::Cancelled,
            "artifact.build cancelled before preflight",
        );
    }
    let before = match capture_inputs(prepared) {
        Ok(inputs) => inputs,
        Err(result) => return result,
    };
    let preview = match invoke_runner(prepared, tool, runner, &cancellation, true) {
        Ok(envelope) => envelope,
        Err(result) => return result,
    };
    let source_set = match validate_envelope(prepared, &before, &preview, false) {
        Ok(source_set) => source_set,
        Err(result) => return result,
    };
    let after = match capture_inputs(prepared) {
        Ok(inputs) => inputs,
        Err(result) => return result,
    };
    if before != after {
        return reject(
            RefusalCode::ConcurrentChange,
            "artifact.build inputs changed during preview; run dryRun: true again",
        );
    }
    let revision = plan_revision(prepared, &before, runner_version, &source_set);
    if prepared.dry_run {
        let mut result = DomainResult::success(format!(
            "artifact.build planned building the {} from source set `{source_set}` without publishing anything",
            prepared.arguments.kind.suffix()
        ));
        result.data = Some(json!({
            "op": OPERATION,
            "dryRun": true,
            "plan": {
                "artifact": {
                    "kind": prepared.arguments.kind.suffix(),
                    "path": path_text(&prepared.arguments.output_relative),
                },
                "sourceSet": source_set,
                "extension": prepared.arguments.extension,
                "outputExists": before.output_sha256.is_some(),
            },
            "providerDispatched": false,
            "requiresPlatform": true,
        }));
        result.rev = Some(revision.clone());
        result.next.push(json!({
            "tool": "unica.run",
            "args": {
                "op": OPERATION,
                "args": public_arguments(prepared),
                "dryRun": false,
                "ifRev": revision,
            },
            "reason": "apply exactly this previewed build"
        }));
        return result;
    }
    if prepared.if_rev.as_deref() != Some(revision.as_str()) {
        // A stale `ifRev` is the caller's conflict with a known recovery, so it
        // answers `stale_revision` and names both revisions
        // (INV.WIRE.V13-REFUSAL-CHANNEL).
        return reject(
            RefusalCode::StaleRevision,
            format!(
                "artifact.build plan or environment changed after preview: expected rev {revision}, ifRev {}; run dryRun: true again",
                prepared.if_rev.as_deref().unwrap_or("absent")
            ),
        );
    }
    if cancellation.is_cancelled() {
        return reject(
            RefusalCode::Cancelled,
            "artifact.build cancelled before provider launch",
        );
    }
    let applied = match invoke_runner(prepared, tool, runner, &cancellation, false) {
        Ok(envelope) => envelope,
        Err(result) => return result,
    };
    let built_from = match validate_envelope(prepared, &before, &applied, true) {
        Ok(source_set) => source_set,
        Err(result) => return result,
    };
    if built_from != source_set {
        return reject(
            RefusalCode::InvalidResult,
            "v8-runner built the artifact from a different source set than previewed",
        );
    }
    // Квитанция не со слов провайдера: файл на выходе Unica читает сама.
    let (sha256, size) = match digest_optional_workspace_file(
        &prepared.context.workspace_root,
        &prepared.arguments.output_relative,
    ) {
        Ok(Some(receipt)) if receipt.1 > 0 => receipt,
        Ok(Some(_)) => {
            return reject(
                RefusalCode::InvalidResult,
                "v8-runner reported the build but the artifact is empty",
            )
        }
        Ok(None) => {
            return reject(
                RefusalCode::InvalidResult,
                "v8-runner reported the build but the artifact is missing",
            )
        }
        Err(error) => return reject(RefusalCode::InvalidResult, error),
    };
    let state = if before.output_sha256.is_some() {
        "replaced"
    } else {
        "created"
    };
    let artifact = json!({
        "kind": prepared.arguments.kind.suffix(),
        "path": path_text(&prepared.arguments.output_relative),
        "size": size,
        "sha256": sha256,
    });
    let mut result = DomainResult::success(format!(
        "artifact.build built the {} from source set `{source_set}` and independently verified it",
        prepared.arguments.kind.suffix()
    ));
    result.data = Some(json!({
        "op": OPERATION,
        "dryRun": false,
        "artifact": artifact,
        "sourceSet": source_set,
        "extension": prepared.arguments.extension,
        "providerDispatched": true,
    }));
    result.changed.push(json!({
        "path": path_text(&prepared.arguments.output_relative),
        "kind": state,
    }));
    result.artifacts.push(artifact);
    result.rev = Some(revision);
    result
}

/// Сверка конверта с планом; возвращает набор, который раннер выбрал.
fn validate_envelope(
    prepared: &PreparedArtifactBuild,
    inputs: &StableInputs,
    envelope: &Value,
    applied: bool,
) -> Result<String, DomainResult> {
    let data = &envelope["data"];
    let phase = if applied { "apply result" } else { "preview" };
    if data["provider_dispatched"] != applied
        || data["execution"]["payload"]["published"] != applied
        || data["execution"]["status"] != "succeeded"
    {
        return Err(reject(
            RefusalCode::InvalidResult,
            if applied {
                "v8-runner reported success without publishing the artifact"
            } else {
                "v8-runner preview did not prove that nothing was published"
            },
        ));
    }
    let mode = prepared.arguments.kind.runner_mode();
    if data["mode"] != mode
        || data["execution"]["payload"]["artifact_type"] != mode
        || data["extension"].as_str() != prepared.arguments.extension.as_deref()
    {
        return Err(reject(
            RefusalCode::InvalidResult,
            format!("v8-runner {phase} answered for a different artifact kind or extension"),
        ));
    }
    let source_set = data["source_set"]
        .as_str()
        .filter(|name| inputs.declared.iter().any(|declared| declared == name))
        .ok_or_else(|| {
            reject(
                RefusalCode::InvalidResult,
                format!(
                    "v8-runner {phase} named a source set that v8project.yaml does not declare"
                ),
            )
        })?
        .to_string();
    if let Some(requested) = &prepared.arguments.source_set {
        if requested != &source_set {
            return Err(reject(
                RefusalCode::InvalidResult,
                format!("v8-runner {phase} answered for a different source set than requested"),
            ));
        }
    }
    let reported = data["output_path"].as_str().ok_or_else(|| {
        reject(
            RefusalCode::InvalidResult,
            format!("v8-runner {phase} omitted the output it acts on"),
        )
    })?;
    let reported = normalize_path_identity(Path::new(reported)).map_err(|error| {
        reject(
            RefusalCode::InvalidResult,
            format!("v8-runner {phase} returned an invalid output path: {error}"),
        )
    })?;
    let expected = normalize_path_identity(&prepared.arguments.output).map_err(|error| {
        reject(
            RefusalCode::InvalidResult,
            format!("failed to resolve the planned output: {error}"),
        )
    })?;
    if reported != expected {
        return Err(reject(
            RefusalCode::InvalidResult,
            format!("v8-runner {phase} named a different output than the requested one"),
        ));
    }
    Ok(source_set)
}

fn invoke_runner(
    prepared: &PreparedArtifactBuild,
    tool: &BundledTool,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
    dry_run: bool,
) -> Result<Value, DomainResult> {
    let mut args = vec![
        "--config".to_string(),
        prepared
            .context
            .workspace_root
            .join(CONFIG_NAME)
            .display()
            .to_string(),
        "--json-message".to_string(),
        RUNNER_COMMAND.to_string(),
        "--output".to_string(),
        prepared.arguments.output.display().to_string(),
    ];
    if let Some(source_set) = &prepared.arguments.source_set {
        args.extend(["--source-set".to_string(), source_set.clone()]);
    }
    if let Some(extension) = &prepared.arguments.extension {
        args.extend(["--extension".to_string(), extension.clone()]);
    }
    if dry_run {
        args.push("--dry-run".to_string());
    }
    let output = runner
        .run(&ProcessCommand {
            program: tool.program.clone(),
            args,
            cwd: prepared.context.workspace_root.clone(),
            env: Vec::new(),
            env_remove: Vec::new(),
            capture_limits: Some((RUNNER_OUTPUT_LIMIT, RUNNER_OUTPUT_LIMIT)),
            timeout: None,
            cancellation: cancellation.clone(),
        })
        .map_err(|error| {
            reject(
                RefusalCode::ProviderUnavailable,
                format!("failed to start bundled v8-runner: {}", redactor(&error)),
            )
        })?;
    parse_runner_output(output)
}

fn parse_runner_output(output: ProcessOutput) -> Result<Value, DomainResult> {
    if output.cancelled {
        return Err(reject(RefusalCode::Cancelled, "v8-runner was cancelled"));
    }
    if output.timed_out {
        return Err(reject(
            RefusalCode::DeadlineExceeded,
            "v8-runner exceeded its execution deadline",
        ));
    }
    if output.stdout_truncated || output.stdout_had_invalid_utf8 {
        return Err(reject(
            RefusalCode::InvalidResult,
            "v8-runner returned an unreadable or oversized JSON result",
        ));
    }
    let envelope: Value = serde_json::from_str(&output.stdout).map_err(|_| {
        reject(
            RefusalCode::InvalidResult,
            "v8-runner returned an invalid JSON result",
        )
    })?;
    if envelope["command"] != RUNNER_COMMAND {
        return Err(reject(
            RefusalCode::InvalidResult,
            "v8-runner returned a result for a different operation",
        ));
    }
    if !output.status_success || envelope["ok"] != true {
        // Снимок отказа у `make` пишет `provider_dispatched: true` и до
        // запуска платформы (v8-runner 0.9.0): отказ идёт по коду раннера.
        let code = envelope["error"]["code"]
            .as_str()
            .unwrap_or("provider_failed");
        let message = envelope["error"]["message"]
            .as_str()
            .map(redactor)
            .unwrap_or_else(|| "v8-runner failed without a typed message".to_string());
        return Err(reject(map_runner_code(code), message));
    }
    Ok(envelope)
}

fn plan_revision(
    prepared: &PreparedArtifactBuild,
    inputs: &StableInputs,
    runner_version: &str,
    source_set: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"unica-v13-artifact-build-plan-v1\0");
    hasher.update(
        serde_json::to_vec(&json!({
            "op": OPERATION,
            "args": public_arguments(prepared),
            "inputs": {
                "config": inputs.config_sha256,
                "localConfig": inputs.local_config_sha256,
                "declared": inputs.declared,
                "output": inputs.output_sha256,
            },
            "runnerVersion": runner_version,
            "sourceSet": source_set,
            "artifactType": prepared.arguments.kind.runner_mode(),
        }))
        .expect("plan revision data serializes"),
    );
    format!("unica-artifact-build-sha256-v1:{:x}", hasher.finalize())
}

fn public_arguments(prepared: &PreparedArtifactBuild) -> Value {
    let mut args = Map::new();
    args.insert(
        "output".to_string(),
        Value::String(path_text(&prepared.arguments.output_relative)),
    );
    if let Some(source_set) = &prepared.arguments.source_set {
        args.insert("sourceSet".to_string(), Value::String(source_set.clone()));
    }
    if let Some(extension) = &prepared.arguments.extension {
        args.insert("extension".to_string(), Value::String(extension.clone()));
    }
    Value::Object(args)
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn reject(code: RefusalCode, message: impl Into<String>) -> DomainResult {
    DomainResult::canonical_rejection(Some(OPERATION.to_string()), code, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::refusal::Outcome;
    use std::fs;
    use std::sync::Mutex;

    struct SequenceRunner {
        outputs: Mutex<Vec<ProcessOutput>>,
        calls: Mutex<Vec<ProcessCommand>>,
        publish_on_apply: Option<Vec<u8>>,
    }

    impl SequenceRunner {
        fn new(outputs: Vec<ProcessOutput>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into_iter().rev().collect()),
                calls: Mutex::new(Vec::new()),
                publish_on_apply: None,
            }
        }

        fn publishing(outputs: Vec<ProcessOutput>, bytes: &[u8]) -> Self {
            Self {
                outputs: Mutex::new(outputs.into_iter().rev().collect()),
                calls: Mutex::new(Vec::new()),
                publish_on_apply: Some(bytes.to_vec()),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }

        fn joined_args(&self, index: usize) -> String {
            self.calls.lock().unwrap()[index].args.join(" ")
        }
    }

    impl ProcessRunner for SequenceRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.calls.lock().unwrap().push(command.clone());
            if !command.args.iter().any(|argument| argument == "--dry-run") {
                if let Some(bytes) = &self.publish_on_apply {
                    let index = command
                        .args
                        .iter()
                        .position(|argument| argument == "--output")
                        .expect("output argument");
                    let output = PathBuf::from(&command.args[index + 1]);
                    fs::create_dir_all(output.parent().unwrap()).unwrap();
                    fs::write(output, bytes).unwrap();
                }
            }
            Ok(self.outputs.lock().unwrap().pop().expect("runner output"))
        }
    }

    fn process(envelope: Value, success: bool) -> ProcessOutput {
        ProcessOutput {
            status_success: success,
            status: if success {
                "exit status: 0"
            } else {
                "exit status: 2"
            }
            .to_string(),
            stdout: serde_json::to_string(&envelope).unwrap(),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        }
    }

    fn context(root: &Path) -> WorkspaceContext {
        WorkspaceContext {
            cwd: root.to_path_buf(),
            workspace_root: root.to_path_buf(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    fn workspace() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join(CONFIG_NAME),
            "format: DESIGNER\ninfobase:\n  connection: 'File=build/ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: main\n  - name: ext-sales\n    type: EXTENSION\n    path: ext-sales\n",
        )
        .unwrap();
        root
    }

    fn build_of(
        root: &Path,
        output: &str,
        source_set: Option<&str>,
        extension: Option<&str>,
        dry_run: bool,
        if_rev: Option<String>,
    ) -> PreparedArtifactBuild {
        let mut args = Map::new();
        args.insert("output".to_string(), json!(output));
        if let Some(source_set) = source_set {
            args.insert("sourceSet".to_string(), json!(source_set));
        }
        if let Some(extension) = extension {
            args.insert("extension".to_string(), json!(extension));
        }
        PreparedArtifactBuild {
            arguments: parse_build_arguments(&args, &context(root)).expect("valid arguments"),
            dry_run,
            if_rev,
            context: context(root),
        }
    }

    fn tool(root: &Path) -> BundledTool {
        BundledTool {
            program: root.join("v8-runner"),
            warnings: Vec::new(),
            missing: None,
        }
    }

    /// Конверт `make` раннера 0.9.0, снятый с живой пробы: вид артефакта,
    /// набор, выход как передан и `published` в полезной нагрузке.
    fn envelope(
        output: &Path,
        kind: ArtifactKind,
        set: &str,
        extension: Option<&str>,
        published: bool,
    ) -> Value {
        let mut data = json!({
            "ok": true,
            "provider_dispatched": published,
            "mode": kind.runner_mode(),
            "source_set": set,
            "output_path": output,
            "message": format!(
                "would build into '{}' via /opt/1cv8/8.3.27.2074/1cv8; nothing published",
                output.display()
            ),
            "execution": {
                "status": "succeeded",
                "diagnostics": ["via /opt/1cv8/8.3.27.2074/1cv8"],
                "payload": {
                    "artifact_type": kind.runner_mode(),
                    "output_path": output,
                    "file_names": [output.file_name().unwrap().to_string_lossy()],
                    "published": published
                }
            }
        });
        if let Some(extension) = extension {
            data["extension"] = json!(extension);
        }
        json!({
            "ok": true,
            "command": "make",
            "duration_ms": 3,
            "data": data,
            "warnings": [],
            "steps": [],
        })
    }

    fn run(root: &Path, prepared: &PreparedArtifactBuild, runner: &SequenceRunner) -> DomainResult {
        execute_with_resolved_runner(
            prepared,
            runner,
            CancellationToken::new(),
            &tool(root),
            "0.9.0",
        )
    }

    #[test]
    fn arguments_are_closed_and_each_refusal_names_the_fix() {
        let root = workspace();
        for (args, code, expected) in [
            (
                json!({"output": "dist/main.cf", "publish": true}),
                "bad_value",
                "does not accept `publish`",
            ),
            (json!({}), "bad_value", "output must be non-empty text"),
            (
                json!({"output": "/tmp/main.cf"}),
                "bad_value",
                "workspace-relative",
            ),
            (
                json!({"output": "dist/main.dt"}),
                "bad_value",
                ".cf for a configuration or .cfe",
            ),
            (
                json!({"output": "dist/report.epf"}),
                "unsupported_operation",
                "not published in v0.13",
            ),
            (
                json!({"output": "dist/sales.cfe"}),
                "bad_value",
                "requires extension",
            ),
            (
                json!({"output": "dist/main.cf", "extension": "Sales"}),
                "bad_value",
                "only for a .cfe output",
            ),
            (
                json!({"output": "dist/sales.cfe", "extension": "1Sales"}),
                "bad_value",
                "1C identifier",
            ),
            (
                json!({"output": "dist/main.cf", "sourceSet": "../x"}),
                "bad_value",
                "must be the name of a source set",
            ),
        ] {
            let result = parse_build_arguments(args.as_object().unwrap(), &context(root.path()))
                .err()
                .unwrap_or_else(|| panic!("{args} must be refused"));
            assert_eq!(result.diagnostics[0]["code"], code, "{args}: {result:?}");
            assert!(
                result.diagnostics[0]["message"]
                    .as_str()
                    .unwrap()
                    .contains(expected),
                "{args}: {result:?}"
            );
        }
        let accepted = parse_build_arguments(
            json!({"output": "dist/sales.cfe", "sourceSet": "ext-sales", "extension": "Sales"})
                .as_object()
                .unwrap(),
            &context(root.path()),
        )
        .unwrap();
        assert_eq!(accepted.kind, ArtifactKind::Cfe);
        assert_eq!(accepted.output_relative, PathBuf::from("dist/sales.cfe"));

        let runner = SequenceRunner::new(Vec::new());
        let result = run(
            root.path(),
            &build_of(
                root.path(),
                "dist/main.cf",
                Some("ext-purchases"),
                None,
                true,
                None,
            ),
            &runner,
        );
        assert_eq!(result.diagnostics[0]["code"], "bad_value", "{result:?}");
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("declared source sets: main, ext-sales"));
        assert_eq!(runner.call_count(), 0);
    }

    #[test]
    fn preview_names_the_artifact_and_the_set_without_publishing() {
        let root = workspace();
        let prepared = build_of(root.path(), "dist/main.cf", None, None, true, None);
        let runner = SequenceRunner::new(vec![process(
            envelope(
                &prepared.arguments.output,
                ArtifactKind::Cf,
                "main",
                None,
                false,
            ),
            true,
        )]);
        let result = run(root.path(), &prepared, &runner);

        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["providerDispatched"], false);
        assert_eq!(data["plan"]["artifact"]["kind"], "cf");
        assert_eq!(data["plan"]["artifact"]["path"], "dist/main.cf");
        assert_eq!(data["plan"]["sourceSet"], "main");
        assert_eq!(data["plan"]["outputExists"], false);
        let revision = result.rev.clone().expect("preview returns a revision");
        assert!(revision.starts_with("unica-artifact-build-sha256-v1:"));
        assert_eq!(result.next[0]["args"]["ifRev"], revision);
        assert_eq!(
            result.next[0]["args"]["args"],
            json!({"output": "dist/main.cf"})
        );
        assert!(result.changed.is_empty());
        assert!(!prepared.arguments.output.exists());
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("1cv8"), "platform path leaked: {encoded}");
        let root_text = normalize_path_identity(root.path())
            .unwrap()
            .display()
            .to_string();
        assert!(
            !encoded.contains(&root_text),
            "absolute path leaked: {encoded}"
        );
        let args = runner.joined_args(0);
        assert!(
            args.contains("--json-message make --output ") && args.ends_with("--dry-run"),
            "{args}"
        );
    }

    #[test]
    fn preview_of_an_extension_passes_the_set_and_the_extension() {
        let root = workspace();
        let prepared = build_of(
            root.path(),
            "dist/sales.cfe",
            Some("ext-sales"),
            Some("Sales"),
            true,
            None,
        );
        let runner = SequenceRunner::new(vec![process(
            envelope(
                &prepared.arguments.output,
                ArtifactKind::Cfe,
                "ext-sales",
                Some("Sales"),
                false,
            ),
            true,
        )]);
        let result = run(root.path(), &prepared, &runner);
        assert!(result.ok, "{result:?}");
        assert_eq!(result.data.as_ref().unwrap()["plan"]["extension"], "Sales");
        assert!(runner
            .joined_args(0)
            .ends_with("--source-set ext-sales --extension Sales --dry-run"));
    }

    #[test]
    fn preview_refuses_another_output_kind_or_set_and_a_published_preview() {
        let root = workspace();
        let prepared = build_of(root.path(), "dist/main.cf", None, None, true, None);

        let mut other = envelope(
            &prepared.arguments.output,
            ArtifactKind::Cf,
            "main",
            None,
            false,
        );
        other["data"]["output_path"] = json!(root.path().join("dist/other.cf"));
        let runner = SequenceRunner::new(vec![process(other, true)]);
        let result = run(root.path(), &prepared, &runner);
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("different output"));

        let runner = SequenceRunner::new(vec![process(
            envelope(
                &prepared.arguments.output,
                ArtifactKind::Cfe,
                "main",
                None,
                false,
            ),
            true,
        )]);
        let result = run(root.path(), &prepared, &runner);
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );

        let runner = SequenceRunner::new(vec![process(
            envelope(
                &prepared.arguments.output,
                ArtifactKind::Cf,
                "ext-purchases",
                None,
                false,
            ),
            true,
        )]);
        let result = run(root.path(), &prepared, &runner);
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );

        let runner = SequenceRunner::new(vec![process(
            envelope(
                &prepared.arguments.output,
                ArtifactKind::Cf,
                "main",
                None,
                true,
            ),
            true,
        )]);
        let result = run(root.path(), &prepared, &runner);
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("nothing was published"));
    }

    #[test]
    fn apply_repeats_the_preview_and_returns_an_independent_file_receipt() {
        let root = workspace();
        let preview = build_of(root.path(), "dist/main.cf", None, None, true, None);
        let revision = run(
            root.path(),
            &preview,
            &SequenceRunner::new(vec![process(
                envelope(
                    &preview.arguments.output,
                    ArtifactKind::Cf,
                    "main",
                    None,
                    false,
                ),
                true,
            )]),
        )
        .rev
        .unwrap();
        let prepared = build_of(
            root.path(),
            "dist/main.cf",
            None,
            None,
            false,
            Some(revision.clone()),
        );
        let runner = SequenceRunner::publishing(
            vec![
                process(
                    envelope(
                        &prepared.arguments.output,
                        ArtifactKind::Cf,
                        "main",
                        None,
                        false,
                    ),
                    true,
                ),
                process(
                    envelope(
                        &prepared.arguments.output,
                        ArtifactKind::Cf,
                        "main",
                        None,
                        true,
                    ),
                    true,
                ),
            ],
            b"built cf",
        );

        let result = run(root.path(), &prepared, &runner);

        assert!(result.ok, "{result:?}");
        assert_eq!(runner.call_count(), 2);
        assert!(runner.joined_args(0).ends_with("--dry-run"));
        assert!(!runner.joined_args(1).contains("--dry-run"));
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["artifact"]["path"], "dist/main.cf");
        assert_eq!(data["artifact"]["size"], 8);
        assert_eq!(data["sourceSet"], "main");
        assert_eq!(result.changed[0]["path"], "dist/main.cf");
        assert_eq!(result.changed[0]["kind"], "created");
        assert_eq!(result.artifacts[0]["kind"], "cf");
        assert_eq!(result.artifacts[0]["size"], 8);
        assert_eq!(result.rev, Some(revision));
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("1cv8"), "platform path leaked: {encoded}");
        assert!(!encoded.contains("--config"));
    }

    #[test]
    fn apply_refuses_a_stale_revision_and_a_missing_artifact() {
        let root = workspace();
        let prepared = build_of(
            root.path(),
            "dist/main.cf",
            None,
            None,
            false,
            Some("stale".to_string()),
        );
        let runner = SequenceRunner::new(vec![process(
            envelope(
                &prepared.arguments.output,
                ArtifactKind::Cf,
                "main",
                None,
                false,
            ),
            true,
        )]);
        let result = run(root.path(), &prepared, &runner);
        assert_eq!(result.diagnostics[0]["code"], "stale_revision");
        assert_eq!(runner.call_count(), 1);
        assert!(!prepared.arguments.output.exists());

        let preview = build_of(root.path(), "dist/main.cf", None, None, true, None);
        let revision = run(
            root.path(),
            &preview,
            &SequenceRunner::new(vec![process(
                envelope(
                    &preview.arguments.output,
                    ArtifactKind::Cf,
                    "main",
                    None,
                    false,
                ),
                true,
            )]),
        )
        .rev
        .unwrap();
        let prepared = build_of(
            root.path(),
            "dist/main.cf",
            None,
            None,
            false,
            Some(revision),
        );
        let runner = SequenceRunner::new(vec![
            process(
                envelope(
                    &prepared.arguments.output,
                    ArtifactKind::Cf,
                    "main",
                    None,
                    false,
                ),
                true,
            ),
            process(
                envelope(
                    &prepared.arguments.output,
                    ArtifactKind::Cf,
                    "main",
                    None,
                    true,
                ),
                true,
            ),
        ]);
        let result = run(root.path(), &prepared, &runner);
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("artifact is missing"));
    }

    #[test]
    fn runner_refusals_keep_their_outcome() {
        let root = workspace();
        let prepared = build_of(
            root.path(),
            "dist/sales.cfe",
            Some("ext-sales"),
            Some("Sales"),
            true,
            None,
        );
        let failure = |code: &str| {
            json!({
                "ok": false,
                "command": "make",
                "duration_ms": 0,
                "data": {"ok": false, "provider_dispatched": true, "mode": "extension_cfe", "source_set": "ext-sales", "output_path": prepared.arguments.output, "message": "refused",
                    "execution": {"status": "failed", "diagnostics": ["refused"], "errors": [{"code": "artifacts_failed", "message": "refused"}],
                        "payload": {"artifact_type": "extension_cfe", "output_path": prepared.arguments.output, "file_names": ["sales.cfe"], "published": false}}},
                "warnings": [],
                "steps": [],
                "error": {"code": code, "kind": "validation", "message": "refused"}
            })
        };
        for (code, expected, outcome) in [
            ("invalid_argument", "bad_value", Outcome::FixCall),
            (
                "platform_failure",
                "provider_unavailable",
                Outcome::NeedsHuman,
            ),
        ] {
            let runner = SequenceRunner::new(vec![process(failure(code), false)]);
            let result = run(root.path(), &prepared, &runner);
            assert_eq!(result.diagnostics[0]["code"], expected, "{code}");
            assert_eq!(map_runner_code(code).outcome(), outcome, "{code}");
        }
    }
}
