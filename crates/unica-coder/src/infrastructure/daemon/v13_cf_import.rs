#![allow(clippy::result_large_err)]
//! `cf.import` — загрузка файла конфигурации `.cf` или расширения `.cfe` из
//! рабочего пространства в базу силами `v8-runner load` (A-5 зонтика #871).
//! Пара к `cf.export`: тот выносит конфигурацию из базы в файл, этот вносит
//! файл в базу.
//!
//! Аргументы закрыты: `input` — относительный путь к `.cf` или `.cfe` внутри
//! рабочего пространства, `extension` — имя расширения, обязательное для
//! `.cfe` и недопустимое для `.cf`. Режим один — `load`: `merge` тянет
//! внешний файл настроек объединения, а `update` раннер 0.9.0 не поддерживает.
//!
//! Превью зовёт `load --dry-run`: раннер находит платформу, разбирает файл и
//! ничего не применяет. Применение повторяет превью, сверяет забор ревизии,
//! запускает загрузку и проверяет, что входной файл дошёл до конца
//! неизменным. Состояние базы после загрузки Unica не проверяет — оно
//! засвидетельствовано провайдером, и ответ называет это прямо. Путь к
//! платформе, журналу и командная строка наружу не идут.

use super::protocol::InvocationRequest;
use super::v13_infobase_exports::{
    closed_workspace_relative_path, digest_optional_workspace_file, digest_required_workspace_file,
    missing_runner_rejection, resolve_bundled_runner, runner_rejection, valid_1c_identifier,
    CONFIG_NAME, LOCAL_CONFIG_NAME, RUNNER_OUTPUT_LIMIT,
};
use crate::application::invocation_store::ToolIdentity;
use crate::domain::cancellation::CancellationToken;
use crate::domain::invocation::{DomainResult, SafeIdentityHash};
use crate::domain::refusal::RefusalCode;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bundled_tools::BundledTool;
use crate::infrastructure::internal_adapters::{
    ProcessCommand, ProcessOutput, ProcessRunner, SystemProcessRunner,
};
use crate::infrastructure::path_policy::WorkspacePathPolicy;
use crate::infrastructure::redaction::redactor;
use crate::infrastructure::source_roots::normalize_path_identity;
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) const OPERATION: &str = "cf.import";
/// Имя команды в конверте раннера: словарь читается как слой и направление,
/// раннер называет свои команды по-своему.
const RUNNER_COMMAND: &str = "load";
const RUNNER_MODE: &str = "load";
/// Состояния совместимости, которые раннер 0.9.0 обещает в `compatibility_state`.
const COMPATIBILITY_STATES: [&str; 4] = ["supported", "absent", "not_established", "not_probed"];

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

    /// Как раннер называет тип файла и цель загрузки.
    const fn runner_artifact_type(self) -> &'static str {
        match self {
            Self::Cf => "configuration_cf",
            Self::Cfe => "extension_cfe",
        }
    }

    const fn target_kind(self) -> &'static str {
        match self {
            Self::Cf => "configuration",
            Self::Cfe => "extension",
        }
    }
}

#[derive(Debug, Clone)]
struct ImportArguments {
    kind: ArtifactKind,
    /// Имя расширения — только у `.cfe`.
    extension: Option<String>,
    input_relative: PathBuf,
    input: PathBuf,
}

#[derive(Debug, Clone)]
pub(super) struct PreparedCfImport {
    arguments: ImportArguments,
    dry_run: bool,
    if_rev: Option<String>,
    context: WorkspaceContext,
}

pub(super) enum Preparation {
    NotApplicable,
    Rejected(Box<DomainResult>),
    Ready(Arc<PreparedCfImport>),
}

pub(super) fn prepare(request: &InvocationRequest) -> Preparation {
    if request.tool() != ToolIdentity::Run
        || request.arguments().get("op").and_then(Value::as_str) != Some(OPERATION)
    {
        return Preparation::NotApplicable;
    }
    match PreparedCfImport::parse(request) {
        Ok(prepared) => Preparation::Ready(Arc::new(prepared)),
        Err(result) => Preparation::Rejected(Box::new(result)),
    }
}

impl PreparedCfImport {
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
                    "cf.import requires dryRun: true to preview or dryRun: false with ifRev to apply",
                )
            })?;
        let if_rev = match arguments.get("ifRev") {
            None => None,
            Some(Value::String(value)) if !value.trim().is_empty() => Some(value.clone()),
            Some(_) => {
                return Err(reject(
                    RefusalCode::BadValue,
                    "cf.import ifRev must be non-empty text",
                ))
            }
        };
        if dry_run && if_rev.is_some() {
            return Err(reject(
                RefusalCode::BadValue,
                "cf.import preview does not accept ifRev; apply the revision returned by this preview",
            ));
        }
        if !dry_run && if_rev.is_none() {
            return Err(reject(
                RefusalCode::BadValue,
                "cf.import apply requires ifRev from a prior dryRun preview",
            ));
        }
        let context =
            discover_workspace(Some(PathBuf::from(request.workspace_hint()))).map_err(|error| {
                reject(
                    RefusalCode::ProviderUnavailable,
                    format!("workspace discovery failed: {error}"),
                )
            })?;
        let arguments = parse_import_arguments(args, &context)?;
        Ok(Self {
            arguments,
            dry_run,
            if_rev,
            context,
        })
    }

    pub(super) fn workspace_identity_hash(&self) -> SafeIdentityHash {
        let mut hasher = Sha256::new();
        hasher.update(b"unica-v13-cf-import-workspace-v1\0");
        hasher.update(self.context.workspace_root.as_os_str().as_encoded_bytes());
        SafeIdentityHash::from_sha256(hasher.finalize().into())
    }

    pub(super) fn execute(&self, cancellation: CancellationToken) -> DomainResult {
        execute_with_runner(self, &SystemProcessRunner, cancellation)
    }
}

fn parse_import_arguments(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<ImportArguments, DomainResult> {
    const ACCEPTED: [&str; 2] = ["input", "extension"];
    if let Some(unknown) = args.keys().find(|key| !ACCEPTED.contains(&key.as_str())) {
        return Err(reject(
            RefusalCode::BadValue,
            format!("cf.import does not accept `{unknown}`; the closed args are input and extension, and the only mode is load"),
        ));
    }
    let extension = match args.get("extension") {
        None => None,
        Some(Value::String(value)) if valid_1c_identifier(value) => Some(value.clone()),
        Some(_) => {
            return Err(reject(
                RefusalCode::BadValue,
                "cf.import extension must be a non-empty 1C identifier",
            ))
        }
    };
    let input = args
        .get("input")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            reject(
                RefusalCode::BadValue,
                "cf.import input must be non-empty text",
            )
        })?;
    let input_relative = closed_workspace_relative_path(input)
        .map_err(|message| reject(RefusalCode::BadValue, format!("cf.import input {message}")))?;
    let kind = match input_relative
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("cf") => ArtifactKind::Cf,
        Some("cfe") => ArtifactKind::Cfe,
        _ => {
            return Err(reject(
                RefusalCode::BadValue,
                "cf.import input must end in .cf for a configuration or .cfe for an extension",
            ))
        }
    };
    // Раннер сам требует имя у `.cfe`; у `.cf` имя означало бы, что вызывающий
    // ждёт загрузки расширения, а получит замену конфигурации.
    match (kind, extension.is_some()) {
        (ArtifactKind::Cfe, false) => {
            return Err(reject(
                RefusalCode::BadValue,
                "cf.import input .cfe requires extension: the name the infobase will know it by",
            ))
        }
        (ArtifactKind::Cf, true) => return Err(reject(
            RefusalCode::BadValue,
            "cf.import extension is only for a .cfe input; a .cf replaces the main configuration",
        )),
        _ => {}
    }
    let root_context = WorkspaceContext {
        cwd: context.workspace_root.clone(),
        workspace_root: context.workspace_root.clone(),
        cache_root: context.cache_root.clone(),
        workspace_epoch: context.workspace_epoch,
    };
    // Та же политика, что у выгрузок: путь остаётся внутри пространства и не
    // уходит по ссылкам.
    let input_path = WorkspacePathPolicy::new(&root_context)
        .resolve_write(&input_relative)
        .map_err(|error| reject(RefusalCode::BadValue, error))?;
    // Превью раннера отказывает на отсутствующем файле само, но пустой файл
    // оно примет за план: проверка здесь, до вызова.
    match digest_optional_workspace_file(&context.workspace_root, &input_relative) {
        Ok(Some((_, size))) if size > 0 => {}
        Ok(Some(_)) => {
            return Err(reject(
                RefusalCode::BadValue,
                "cf.import input names an empty file",
            ))
        }
        Ok(None) => {
            return Err(reject(
                RefusalCode::BadValue,
                "cf.import input names a file that does not exist in the workspace",
            ))
        }
        Err(error) => return Err(reject(RefusalCode::BadValue, error)),
    }
    Ok(ImportArguments {
        kind,
        extension,
        input_relative,
        input: input_path,
    })
}

/// Состояние входов, обязанное совпасть у превью и применения: проектный файл
/// с локальным дополнением и сам загружаемый файл.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StableInputs {
    config_sha256: String,
    local_config_sha256: Option<String>,
    input_sha256: String,
    input_size: u64,
}

fn capture_inputs(prepared: &PreparedCfImport) -> Result<StableInputs, DomainResult> {
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
    let (input_sha256, input_size) =
        digest_optional_workspace_file(root, &prepared.arguments.input_relative)
            .map_err(|error| reject(RefusalCode::BadValue, error))?
            .ok_or_else(|| {
                reject(
                    RefusalCode::InvalidState,
                    "cf.import input disappeared from the workspace",
                )
            })?;
    Ok(StableInputs {
        config_sha256,
        local_config_sha256,
        input_sha256,
        input_size,
    })
}

fn execute_with_runner(
    prepared: &PreparedCfImport,
    runner: &dyn ProcessRunner,
    cancellation: CancellationToken,
) -> DomainResult {
    let runner_tool = match resolve_bundled_runner(&prepared.context.cwd) {
        Ok(resolved) => resolved,
        Err(message) => return reject_absent_runner(message),
    };
    let tool = runner_tool.tool;
    let runner_version = runner_tool.version;
    execute_with_resolved_runner(prepared, runner, cancellation, &tool, &runner_version)
}

fn execute_with_resolved_runner(
    prepared: &PreparedCfImport,
    runner: &dyn ProcessRunner,
    cancellation: CancellationToken,
    tool: &BundledTool,
    runner_version: &str,
) -> DomainResult {
    if cancellation.is_cancelled() {
        return reject(
            RefusalCode::Cancelled,
            "cf.import cancelled before preflight",
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
    if let Err(result) = validate_envelope(prepared, &preview, false) {
        return result;
    }
    let after = match capture_inputs(prepared) {
        Ok(inputs) => inputs,
        Err(result) => return result,
    };
    if before != after {
        return reject(
            RefusalCode::ConcurrentChange,
            "cf.import inputs changed during preview; run dryRun: true again",
        );
    }
    let revision = plan_revision(prepared, &before, runner_version);
    if prepared.dry_run {
        let mut result = DomainResult::success(format!(
            "cf.import planned loading the {} without touching the infobase",
            prepared.arguments.kind.target_kind()
        ));
        result.data = Some(json!({
            "op": OPERATION,
            "dryRun": true,
            "plan": public_plan(prepared, &before),
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
            "reason": "apply exactly this previewed load"
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
                "cf.import plan or environment changed after preview: expected rev {revision}, ifRev {}; run dryRun: true again",
                prepared.if_rev.as_deref().unwrap_or("absent")
            ),
        );
    }
    if cancellation.is_cancelled() {
        return reject(
            RefusalCode::Cancelled,
            "cf.import cancelled before provider launch",
        );
    }
    let applied = match invoke_runner(prepared, tool, runner, &cancellation, false) {
        Ok(envelope) => envelope,
        Err(result) => return result,
    };
    if let Err(result) = validate_envelope(prepared, &applied, true) {
        return result;
    }
    // Вход обязан дойти до конца неизменным: иначе провайдер тронул источник,
    // а не только базу.
    let receipt = match digest_optional_workspace_file(
        &prepared.context.workspace_root,
        &prepared.arguments.input_relative,
    ) {
        Ok(Some(receipt)) => receipt,
        Ok(None) => {
            return reject(
                RefusalCode::InvalidResult,
                "the loaded file disappeared; the provider must not consume its source",
            )
        }
        Err(error) => return reject(RefusalCode::InvalidResult, error),
    };
    if receipt.0 != before.input_sha256 {
        return reject(
            RefusalCode::InvalidResult,
            "the loaded file changed during the operation; its source must stay intact",
        );
    }
    let data = &applied["data"];
    // **Улику о состоянии базы Unica не подделывает.** Файл проверить можно,
    // базу без платформы — нет: её состояние здесь засвидетельствовано
    // провайдером, и источник признания назван прямо.
    let mut result = DomainResult::success(format!(
        "cf.import loaded the {} from the named file; the infobase state is attested by the provider",
        prepared.arguments.kind.target_kind()
    ));
    result.data = Some(json!({
        "op": OPERATION,
        "dryRun": false,
        "source": {
            "kind": prepared.arguments.kind.suffix(),
            "path": path_text(&prepared.arguments.input_relative),
            "size": receipt.1,
            "sha256": receipt.0,
        },
        "extension": prepared.arguments.extension,
        "targetKind": prepared.arguments.kind.target_kind(),
        "compatibilityState": data["compatibility_state"],
        "databaseConfigurationUpdated": data["execution"]["payload"]["update_db_cfg_ran"],
        "targetStateAttestedBy": "provider",
    }));
    // Изменилась база, а не файл рабочего пространства: путь сюда не кладётся,
    // иначе запись читалась бы как «переписали файл».
    result.changed.push(json!({
        "infobase": true,
        "kind": prepared.arguments.kind.target_kind(),
        "extension": prepared.arguments.extension,
    }));
    result.rev = Some(revision);
    result
}

fn invoke_runner(
    prepared: &PreparedCfImport,
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
        "--path".to_string(),
        prepared.arguments.input.display().to_string(),
        "--mode".to_string(),
        RUNNER_MODE.to_string(),
    ];
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
            reject_absent_runner(format!(
                "failed to start bundled v8-runner: {}",
                redactor(&error)
            ))
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
        let code = envelope["error"]["code"]
            .as_str()
            .unwrap_or("provider_failed");
        let message = envelope["error"]["message"]
            .as_str()
            .map(redactor)
            .unwrap_or_else(|| "v8-runner failed without a typed message".to_string());
        return Err(runner_rejection(Some(OPERATION.to_string()), code, message));
    }
    Ok(envelope)
}

/// Сверка конверта с планом: превью обязано ничего не применить, применение —
/// применить именно этот файл в именно эту цель.
fn validate_envelope(
    prepared: &PreparedCfImport,
    envelope: &Value,
    applied: bool,
) -> Result<(), DomainResult> {
    let data = &envelope["data"];
    let kind = prepared.arguments.kind;
    let phase = if applied { "apply result" } else { "preview" };
    if data["provider_dispatched"] != applied
        || data["execution"]["payload"]["applied"] != applied
        || data["execution"]["status"] != "succeeded"
    {
        return Err(reject(
            RefusalCode::InvalidResult,
            if applied {
                "v8-runner reported success without applying the load"
            } else {
                "v8-runner preview did not prove that nothing was applied"
            },
        ));
    }
    if data["mode"] != RUNNER_MODE
        || data["artifact_type"] != kind.runner_artifact_type()
        || data["target_kind"] != kind.target_kind()
        || data["extension"].as_str() != prepared.arguments.extension.as_deref()
    {
        return Err(reject(
            RefusalCode::InvalidResult,
            format!("v8-runner {phase} answered for a different artifact or target"),
        ));
    }
    // Поля состояния уходят в ответ как улика провайдера: без них или с
    // чужим типом применение не признаётся, а не отдаётся с `null`.
    if applied
        && (!data["compatibility_state"]
            .as_str()
            .is_some_and(|state| COMPATIBILITY_STATES.contains(&state))
            || !data["execution"]["payload"]["update_db_cfg_ran"].is_boolean())
    {
        return Err(reject(
            RefusalCode::InvalidResult,
            "v8-runner apply result omitted the compatibility state or the database update flag",
        ));
    }
    let reported = data["artifact_path"].as_str().ok_or_else(|| {
        reject(
            RefusalCode::InvalidResult,
            format!("v8-runner {phase} omitted the artifact it acted on"),
        )
    })?;
    let reported = normalize_path_identity(Path::new(reported)).map_err(|error| {
        reject(
            RefusalCode::InvalidResult,
            format!("v8-runner {phase} returned an invalid artifact path: {error}"),
        )
    })?;
    let expected = normalize_path_identity(&prepared.arguments.input).map_err(|error| {
        reject(
            RefusalCode::InvalidResult,
            format!("failed to resolve the named input: {error}"),
        )
    })?;
    if reported != expected {
        return Err(reject(
            RefusalCode::InvalidResult,
            format!("v8-runner {phase} named a different file than the requested input"),
        ));
    }
    Ok(())
}

fn plan_revision(
    prepared: &PreparedCfImport,
    inputs: &StableInputs,
    runner_version: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"unica-v13-cf-import-plan-v1\0");
    hasher.update(
        serde_json::to_vec(&json!({
            "op": OPERATION,
            "args": public_arguments(prepared),
            "inputs": {
                "config": inputs.config_sha256,
                "localConfig": inputs.local_config_sha256,
                "input": inputs.input_sha256,
                "inputSize": inputs.input_size,
            },
            "runnerVersion": runner_version,
            "artifactType": prepared.arguments.kind.runner_artifact_type(),
            "mode": RUNNER_MODE,
        }))
        .expect("plan revision data serializes"),
    );
    format!("unica-cf-import-sha256-v1:{:x}", hasher.finalize())
}

fn public_plan(prepared: &PreparedCfImport, inputs: &StableInputs) -> Value {
    json!({
        "source": {
            "kind": prepared.arguments.kind.suffix(),
            "path": path_text(&prepared.arguments.input_relative),
            "size": inputs.input_size,
            "sha256": inputs.input_sha256,
        },
        "extension": prepared.arguments.extension,
        "targetKind": prepared.arguments.kind.target_kind(),
        "mode": RUNNER_MODE,
        // Что превью узнать не может, названо, а не умолчано: совместимость
        // раннер проверяет только на применении.
        "compatibilityKnownBeforeApply": false,
        "targetStateKnownBeforeApply": false,
    })
}

fn public_arguments(prepared: &PreparedCfImport) -> Value {
    let mut args = Map::new();
    args.insert(
        "input".to_string(),
        Value::String(path_text(&prepared.arguments.input_relative)),
    );
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

/// Поставляемого раннера нет: маршрут один на все операции `run`, уточнение
/// `provider_absent` назначает общий помощник.
fn reject_absent_runner(message: impl Into<String>) -> DomainResult {
    missing_runner_rejection(Some(OPERATION.to_string()), message)
}

#[cfg(test)]
mod tests {
    use super::super::v13_infobase_exports::map_runner_code;
    use super::*;
    use crate::domain::refusal::Outcome;
    use std::fs;
    use std::sync::Mutex;

    struct SequenceRunner {
        outputs: Mutex<Vec<ProcessOutput>>,
        calls: Mutex<Vec<ProcessCommand>>,
        /// Что раннер делает с входом на применении: настоящий не трогает.
        tamper_on_apply: Option<Vec<u8>>,
    }

    impl SequenceRunner {
        fn new(outputs: Vec<ProcessOutput>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into_iter().rev().collect()),
                calls: Mutex::new(Vec::new()),
                tamper_on_apply: None,
            }
        }

        fn tampering(outputs: Vec<ProcessOutput>, bytes: &[u8]) -> Self {
            Self {
                outputs: Mutex::new(outputs.into_iter().rev().collect()),
                calls: Mutex::new(Vec::new()),
                tamper_on_apply: Some(bytes.to_vec()),
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
                if let Some(bytes) = &self.tamper_on_apply {
                    let index = command
                        .args
                        .iter()
                        .position(|argument| argument == "--path")
                        .expect("path argument");
                    fs::write(&command.args[index + 1], bytes).unwrap();
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
            "format: DESIGNER\ninfobase:\n  connection: 'File=/tmp/ib'\n",
        )
        .unwrap();
        fs::create_dir_all(root.path().join("dist")).unwrap();
        fs::write(root.path().join("dist/main.cf"), b"configuration bytes").unwrap();
        fs::write(root.path().join("dist/sales.cfe"), b"extension bytes").unwrap();
        fs::write(root.path().join("dist/empty.cf"), b"").unwrap();
        root
    }

    fn import_of(
        root: &Path,
        input: &str,
        extension: Option<&str>,
        dry_run: bool,
        if_rev: Option<String>,
    ) -> PreparedCfImport {
        PreparedCfImport {
            arguments: parse_import_arguments(
                json!({"input": input, "extension": extension})
                    .as_object()
                    .map(|object| {
                        let mut object = object.clone();
                        if extension.is_none() {
                            object.remove("extension");
                        }
                        object
                    })
                    .as_ref()
                    .unwrap(),
                &context(root),
            )
            .expect("valid arguments"),
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

    /// Конверт `load` раннера 0.9.0, снятый с живой пробы: превью и применение
    /// различаются `provider_dispatched`, `applied` и полями состояния.
    fn envelope(input: &Path, kind: ArtifactKind, extension: Option<&str>, applied: bool) -> Value {
        let input = normalize_path_identity(input).unwrap();
        let mut data = json!({
            "ok": true,
            "provider_dispatched": applied,
            "mode": "load",
            "artifact_path": input,
            "artifact_type": kind.runner_artifact_type(),
            "target_kind": kind.target_kind(),
            "compatibility_state": if applied { "supported" } else { "not_probed" },
            "duration_ms": 4,
            "message": format!(
                "load {} {}; would load {} via /opt/1cv8/8.3.27.2074/1cv8",
                input.display(),
                if applied { "applied" } else { "previewed; nothing applied" },
                kind.target_kind()
            ),
            "execution": {
                "status": "succeeded",
                "diagnostics": ["would load via /opt/1cv8/8.3.27.2074/1cv8"],
                "payload": {
                    "applied": applied,
                    "target_kind": kind.target_kind(),
                    "compatibility_state": if applied { "supported" } else { "not_probed" },
                    "update_db_cfg_ran": applied,
                }
            }
        });
        if let Some(extension) = extension {
            data["extension"] = json!(extension);
        }
        if applied {
            data["platform_log_path"] = json!(input.parent().unwrap().join("build/logs/load.log"));
        }
        json!({
            "ok": true,
            "command": "load",
            "duration_ms": 4,
            "data": data,
            "warnings": [],
            "steps": [],
        })
    }

    fn preview(root: &Path, input: &str, extension: Option<&str>) -> DomainResult {
        let prepared = import_of(root, input, extension, true, None);
        let runner = SequenceRunner::new(vec![process(
            envelope(
                &prepared.arguments.input,
                prepared.arguments.kind,
                extension,
                false,
            ),
            true,
        )]);
        execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool(root),
            "0.9.0",
        )
    }

    #[test]
    fn arguments_are_closed_and_each_refusal_names_the_fix() {
        let root = workspace();
        for (args, expected) in [
            (
                json!({"input": "dist/main.cf", "mode": "merge"}),
                "does not accept `mode`",
            ),
            (
                json!({"extension": "Sales"}),
                "input must be non-empty text",
            ),
            (json!({"input": "../main.cf"}), "parent traversal"),
            (
                json!({"input": "dist/main.dt"}),
                ".cf for a configuration or .cfe",
            ),
            (json!({"input": "dist/sales.cfe"}), "requires extension"),
            (
                json!({"input": "dist/main.cf", "extension": "Sales"}),
                "only for a .cfe input",
            ),
            (
                json!({"input": "dist/sales.cfe", "extension": "1Sales"}),
                "1C identifier",
            ),
            (json!({"input": "dist/missing.cf"}), "does not exist"),
            (json!({"input": "dist/empty.cf"}), "empty file"),
        ] {
            let result = parse_import_arguments(args.as_object().unwrap(), &context(root.path()))
                .err()
                .unwrap_or_else(|| panic!("{args} must be refused"));
            assert_eq!(result.diagnostics[0]["code"], "bad_value", "{args}");
            assert_eq!(result.diagnostics[0]["outcome"], "fixCall", "{args}");
            assert!(
                result.diagnostics[0]["message"]
                    .as_str()
                    .unwrap()
                    .contains(expected),
                "{args}: {result:?}"
            );
        }
        let accepted = parse_import_arguments(
            json!({"input": "dist/sales.cfe", "extension": "Sales"})
                .as_object()
                .unwrap(),
            &context(root.path()),
        )
        .unwrap();
        assert_eq!(accepted.kind, ArtifactKind::Cfe);
        assert_eq!(accepted.extension.as_deref(), Some("Sales"));
        assert_eq!(accepted.input_relative, PathBuf::from("dist/sales.cfe"));
    }

    #[test]
    fn a_missing_project_file_is_named_before_the_runner_is_called() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("dist")).unwrap();
        fs::write(root.path().join("dist/main.cf"), b"cf").unwrap();
        let runner = SequenceRunner::new(Vec::new());
        let result = execute_with_resolved_runner(
            &import_of(root.path(), "dist/main.cf", None, true, None),
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
            "0.9.0",
        );
        assert_eq!(result.diagnostics[0]["code"], "invalid_state");
        assert_eq!(result.next[0]["tool"], "unica.view");
        assert_eq!(runner.call_count(), 0);
    }

    #[test]
    fn preview_names_the_source_and_the_target_without_touching_the_infobase() {
        let root = workspace();
        let prepared = import_of(root.path(), "dist/sales.cfe", Some("Sales"), true, None);
        let runner = SequenceRunner::new(vec![process(
            envelope(
                &prepared.arguments.input,
                ArtifactKind::Cfe,
                Some("Sales"),
                false,
            ),
            true,
        )]);
        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
            "0.9.0",
        );

        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["providerDispatched"], false);
        assert_eq!(data["requiresPlatform"], true);
        assert_eq!(data["plan"]["source"]["kind"], "cfe");
        assert_eq!(data["plan"]["source"]["path"], "dist/sales.cfe");
        assert_eq!(data["plan"]["source"]["size"], 15);
        assert_eq!(data["plan"]["extension"], "Sales");
        assert_eq!(data["plan"]["targetKind"], "extension");
        assert_eq!(data["plan"]["mode"], "load");
        assert_eq!(data["plan"]["compatibilityKnownBeforeApply"], false);
        let revision = result.rev.clone().expect("preview returns a revision");
        assert!(revision.starts_with("unica-cf-import-sha256-v1:"));
        assert_eq!(result.next[0]["args"]["ifRev"], revision);
        assert_eq!(
            result.next[0]["args"]["args"],
            json!({"input": "dist/sales.cfe", "extension": "Sales"})
        );
        assert!(result.changed.is_empty());
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("1cv8"), "platform path leaked: {encoded}");
        assert!(!encoded.contains("--config"));
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
            args.contains("--json-message load --path ")
                && args.ends_with("--mode load --extension Sales --dry-run"),
            "{args}"
        );
    }

    #[test]
    fn preview_refuses_a_plan_for_another_artifact_or_one_that_applied() {
        let root = workspace();
        let prepared = import_of(root.path(), "dist/main.cf", None, true, None);

        let mut other = envelope(&prepared.arguments.input, ArtifactKind::Cf, None, false);
        other["data"]["artifact_path"] = json!(root.path().join("dist/other.cf"));
        let runner = SequenceRunner::new(vec![process(other, true)]);
        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
            "0.9.0",
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("different file"));

        let mut applied = envelope(&prepared.arguments.input, ArtifactKind::Cf, None, false);
        applied["data"]["execution"]["payload"]["applied"] = json!(true);
        let runner = SequenceRunner::new(vec![process(applied, true)]);
        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
            "0.9.0",
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("nothing was applied"));
    }

    #[test]
    fn apply_repeats_the_preview_and_attributes_the_infobase_state_to_the_provider() {
        let root = workspace();
        let revision = preview(root.path(), "dist/main.cf", None).rev.unwrap();
        let prepared = import_of(
            root.path(),
            "dist/main.cf",
            None,
            false,
            Some(revision.clone()),
        );
        let runner = SequenceRunner::new(vec![
            process(
                envelope(&prepared.arguments.input, ArtifactKind::Cf, None, false),
                true,
            ),
            process(
                envelope(&prepared.arguments.input, ArtifactKind::Cf, None, true),
                true,
            ),
        ]);

        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
            "0.9.0",
        );

        assert!(result.ok, "{result:?}");
        assert_eq!(runner.call_count(), 2);
        assert!(runner.joined_args(0).ends_with("--dry-run"));
        assert!(!runner.joined_args(1).contains("--dry-run"));
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["source"]["path"], "dist/main.cf");
        assert_eq!(data["source"]["size"], 19);
        assert_eq!(data["targetKind"], "configuration");
        assert_eq!(data["compatibilityState"], "supported");
        assert_eq!(data["databaseConfigurationUpdated"], true);
        assert_eq!(data["targetStateAttestedBy"], "provider");
        assert!(data["extension"].is_null());
        assert_eq!(result.changed[0]["infobase"], true);
        assert_eq!(result.changed[0]["kind"], "configuration");
        assert!(result.changed[0].get("path").is_none());
        assert!(result.artifacts.is_empty());
        assert_eq!(result.rev, Some(revision));
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(
            !encoded.contains("build/logs"),
            "log path leaked: {encoded}"
        );
        assert!(!encoded.contains("1cv8"), "platform path leaked: {encoded}");
    }

    #[test]
    fn stale_apply_stops_after_the_non_executing_preflight() {
        let root = workspace();
        let prepared = import_of(
            root.path(),
            "dist/main.cf",
            None,
            false,
            Some("stale".to_string()),
        );
        let runner = SequenceRunner::new(vec![process(
            envelope(&prepared.arguments.input, ArtifactKind::Cf, None, false),
            true,
        )]);
        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
            "0.9.0",
        );
        assert_eq!(result.diagnostics[0]["code"], "stale_revision");
        assert_eq!(runner.call_count(), 1);
    }

    #[test]
    fn apply_refuses_a_provider_that_touched_the_source_or_reported_nothing_applied() {
        let root = workspace();
        let revision = preview(root.path(), "dist/main.cf", None).rev.unwrap();
        let prepared = import_of(
            root.path(),
            "dist/main.cf",
            None,
            false,
            Some(revision.clone()),
        );
        let runner = SequenceRunner::tampering(
            vec![
                process(
                    envelope(&prepared.arguments.input, ArtifactKind::Cf, None, false),
                    true,
                ),
                process(
                    envelope(&prepared.arguments.input, ArtifactKind::Cf, None, true),
                    true,
                ),
            ],
            b"rewritten by the provider",
        );
        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
            "0.9.0",
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("must stay intact"));

        let root = workspace();
        let revision = preview(root.path(), "dist/main.cf", None).rev.unwrap();
        let prepared = import_of(root.path(), "dist/main.cf", None, false, Some(revision));
        let runner = SequenceRunner::new(vec![
            process(
                envelope(&prepared.arguments.input, ArtifactKind::Cf, None, false),
                true,
            ),
            process(
                envelope(&prepared.arguments.input, ArtifactKind::Cf, None, false),
                true,
            ),
        ]);
        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
            "0.9.0",
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("without applying"));
    }

    #[test]
    fn apply_refuses_an_envelope_without_the_state_it_attests() {
        let root = workspace();
        let revision = preview(root.path(), "dist/main.cf", None).rev.unwrap();
        let prepared = import_of(root.path(), "dist/main.cf", None, false, Some(revision));
        let mut applied = envelope(&prepared.arguments.input, ArtifactKind::Cf, None, true);
        applied["data"]["compatibility_state"] = json!("maybe");
        let runner = SequenceRunner::new(vec![
            process(
                envelope(&prepared.arguments.input, ArtifactKind::Cf, None, false),
                true,
            ),
            process(applied, true),
        ]);
        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
            "0.9.0",
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("compatibility state"));

        let root = workspace();
        let revision = preview(root.path(), "dist/main.cf", None).rev.unwrap();
        let prepared = import_of(root.path(), "dist/main.cf", None, false, Some(revision));
        let mut applied = envelope(&prepared.arguments.input, ArtifactKind::Cf, None, true);
        applied["data"]["execution"]["payload"]
            .as_object_mut()
            .unwrap()
            .remove("update_db_cfg_ran");
        let runner = SequenceRunner::new(vec![
            process(
                envelope(&prepared.arguments.input, ArtifactKind::Cf, None, false),
                true,
            ),
            process(applied, true),
        ]);
        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
            "0.9.0",
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
    }

    #[test]
    fn runner_refusals_keep_their_outcome() {
        let root = workspace();
        let prepared = import_of(root.path(), "dist/main.cf", None, true, None);
        let failure = |code: &str, message: &str| {
            json!({
                "ok": false,
                "command": "load",
                "duration_ms": 0,
                "data": {"message": message},
                "warnings": [],
                "steps": [],
                "error": {"code": code, "kind": "validation", "message": message}
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
            let runner = SequenceRunner::new(vec![process(failure(code, "refused"), false)]);
            let result = execute_with_resolved_runner(
                &prepared,
                &runner,
                CancellationToken::new(),
                &tool(root.path()),
                "0.9.0",
            );
            assert_eq!(result.diagnostics[0]["code"], expected, "{code}");
            assert_eq!(map_runner_code(code).outcome(), outcome, "{code}");
        }
    }
}
