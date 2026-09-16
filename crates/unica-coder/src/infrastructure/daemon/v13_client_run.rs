#![allow(clippy::result_large_err)]
//! `client.run` — единственная терминальная операция словаря `run`: запуск
//! клиента или конфигуратора 1С. Вариант А развилки A-1 зонтика #871:
//! вызов как опубликовано — `execution: terminal`, `run {op, args}` запускает
//! сразу, `dryRun: true` допускается как необязательный неисполняющий план,
//! `ifRev` не принимается: операция не меняет ни исходники, ни базу, и забору
//! ревизии нечего охранять.
//!
//! Платформу ищет только раннер: превью зовёт `v8-runner launch --dry-run`,
//! которое доходит до составленного `ProcessRequest` и останавливается до
//! spawn (ADR-0025 форка). Наружу уходит выбранная платформа — версия и
//! источник, — но не командная строка, не путь к бинарю и не строка
//! соединения: у выгрузок то же правило (`INV.RUNTIME.V13-INFOBASE-EXPORTS`).

use super::protocol::InvocationRequest;
use super::v13_infobase_exports::{
    closed_workspace_relative_path, digest_optional_workspace_file, digest_required_workspace_file,
    missing_runner_rejection, resolve_bundled_runner, runner_rejection, CONFIG_NAME,
    RUNNER_OUTPUT_LIMIT,
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
use crate::infrastructure::redaction::redactor;
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) const OPERATION: &str = "client.run";
/// Имя команды в конверте раннера: у `launch` оно не совпадает с именем
/// операции словаря, в отличие от семейства `infobase`.
const RUNNER_COMMAND: &str = "launch";
const WAIT_TIMEOUT_MAX_MS: u64 = 86_400_000;
const PROCESSOR_EXTENSIONS: [&str; 2] = ["epf", "erf"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientMode {
    Designer,
    Thin,
    Thick,
    Ordinary,
}

impl ClientMode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "designer" => Some(Self::Designer),
            "thin" => Some(Self::Thin),
            "thick" => Some(Self::Thick),
            "ordinary" => Some(Self::Ordinary),
            _ => None,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Designer => "designer",
            Self::Thin => "thin",
            Self::Thick => "thick",
            Self::Ordinary => "ordinary",
        }
    }
}

#[derive(Debug, Clone)]
struct LaunchArguments {
    mode: ClientMode,
    /// Внешняя обработка для `/Execute`: относительный путь для ответа и
    /// абсолютный для раннера. Проверена как обычный файл внутри рабочего
    /// пространства до вызова: превью раннера существование не проверяет.
    execute: Option<(PathBuf, PathBuf)>,
    /// Ожидание выхода клиента с обработкой; у раннера флаг и срок обязательны
    /// вместе, поэтому здесь один слот.
    wait_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub(super) struct PreparedClientRun {
    arguments: LaunchArguments,
    dry_run: bool,
    context: WorkspaceContext,
}

pub(super) enum Preparation {
    NotApplicable,
    Rejected(Box<DomainResult>),
    Ready(Arc<PreparedClientRun>),
}

pub(super) fn prepare(request: &InvocationRequest) -> Preparation {
    if request.tool() != ToolIdentity::Run
        || request.arguments().get("op").and_then(Value::as_str) != Some(OPERATION)
    {
        return Preparation::NotApplicable;
    }
    match PreparedClientRun::parse(request) {
        Ok(prepared) => Preparation::Ready(Arc::new(prepared)),
        Err(result) => Preparation::Rejected(Box::new(result)),
    }
}

impl PreparedClientRun {
    fn parse(request: &InvocationRequest) -> Result<Self, DomainResult> {
        let arguments = request.arguments();
        let args = arguments
            .get("args")
            .and_then(Value::as_object)
            .ok_or_else(|| reject(RefusalCode::BadValue, "run args must be an object"))?;
        let dry_run = match arguments.get("dryRun") {
            None => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => {
                return Err(reject(
                    RefusalCode::BadValue,
                    "client.run dryRun must be boolean",
                ))
            }
        };
        if arguments.get("ifRev").is_some() {
            return Err(reject(
                RefusalCode::BadValue,
                "client.run is terminal and takes no ifRev: it changes neither sources nor the infobase",
            ));
        }
        let context =
            discover_workspace(Some(PathBuf::from(request.workspace_hint()))).map_err(|error| {
                reject(
                    RefusalCode::ProviderUnavailable,
                    format!("workspace discovery failed: {error}"),
                )
            })?;
        let arguments = parse_launch_arguments(args, &context)?;
        Ok(Self {
            arguments,
            dry_run,
            context,
        })
    }

    pub(super) fn workspace_identity_hash(&self) -> SafeIdentityHash {
        let mut hasher = Sha256::new();
        hasher.update(b"unica-v13-client-run-workspace-v1\0");
        hasher.update(self.context.workspace_root.as_os_str().as_encoded_bytes());
        SafeIdentityHash::from_sha256(hasher.finalize().into())
    }

    pub(super) fn execute(&self, cancellation: CancellationToken) -> DomainResult {
        execute_with_runner(self, &SystemProcessRunner, cancellation)
    }
}

fn parse_launch_arguments(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<LaunchArguments, DomainResult> {
    const ACCEPTED: [&str; 4] = ["clientMode", "execute", "waitForExit", "waitTimeoutMs"];
    if let Some(unknown) = args.keys().find(|key| !ACCEPTED.contains(&key.as_str())) {
        return Err(reject(
            RefusalCode::BadValue,
            format!("client.run does not accept `{unknown}`; the closed args are clientMode, execute, waitForExit and waitTimeoutMs"),
        ));
    }
    let mode = args
        .get("clientMode")
        .and_then(Value::as_str)
        .and_then(ClientMode::parse)
        .ok_or_else(|| {
            reject(
                RefusalCode::BadValue,
                "client.run clientMode must be one of designer, thin, thick, ordinary",
            )
        })?;
    let execute = match args.get("execute") {
        None => None,
        Some(Value::String(value)) => {
            if mode == ClientMode::Designer {
                return Err(reject(
                    RefusalCode::BadValue,
                    "client.run execute needs an enterprise client: designer runs no external processor",
                ));
            }
            let relative = closed_workspace_relative_path(value).map_err(|reason| {
                reject(
                    RefusalCode::BadValue,
                    format!("client.run execute {reason}"),
                )
            })?;
            let has_processor_extension = relative
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    PROCESSOR_EXTENSIONS
                        .iter()
                        .any(|accepted| accepted.eq_ignore_ascii_case(extension))
                });
            if !has_processor_extension {
                return Err(reject(
                    RefusalCode::BadValue,
                    "client.run execute must name an .epf or .erf external processor",
                ));
            }
            // Превью раннера существование файла не проверяет: оно составит
            // план и по несуществующему пути. Проверка здесь, до вызова.
            match digest_optional_workspace_file(&context.workspace_root, &relative) {
                Ok(Some((_, size))) if size > 0 => {}
                Ok(Some(_)) => {
                    return Err(reject(
                        RefusalCode::BadValue,
                        "client.run execute names an empty file",
                    ))
                }
                Ok(None) => {
                    return Err(reject(
                        RefusalCode::BadValue,
                        "client.run execute names a file that does not exist in the workspace",
                    ))
                }
                Err(error) => return Err(reject(RefusalCode::BadValue, error)),
            }
            Some((relative.clone(), context.workspace_root.join(relative)))
        }
        Some(_) => {
            return Err(reject(
                RefusalCode::BadValue,
                "client.run execute must be a workspace-relative path",
            ))
        }
    };
    let wait_for_exit = match args.get("waitForExit") {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => {
            return Err(reject(
                RefusalCode::BadValue,
                "client.run waitForExit must be boolean",
            ))
        }
    };
    let wait_timeout_ms = match args.get("waitTimeoutMs") {
        None => None,
        Some(value) => Some(
            value
                .as_u64()
                .filter(|timeout| (1..=WAIT_TIMEOUT_MAX_MS).contains(timeout))
                .ok_or_else(|| {
                    reject(
                        RefusalCode::BadValue,
                        format!("client.run waitTimeoutMs must be an integer from 1 to {WAIT_TIMEOUT_MAX_MS}"),
                    )
                })?,
        ),
    };
    // Раннер требует срок вместе с ожиданием и отказывает на каждом из них
    // порознь; ожидание без обработки — ожидание интерактивного сеанса, и
    // терминального результата у него нет.
    match (wait_for_exit, wait_timeout_ms.is_some(), execute.is_some()) {
        (true, false, _) => {
            return Err(reject(
                RefusalCode::BadValue,
                "client.run waitForExit requires waitTimeoutMs",
            ))
        }
        (false, true, _) => {
            return Err(reject(
                RefusalCode::BadValue,
                "client.run waitTimeoutMs requires waitForExit: true",
            ))
        }
        (true, true, false) => {
            return Err(reject(
                RefusalCode::BadValue,
                "client.run waitForExit needs execute: only an external processor session has an exit to wait for",
            ))
        }
        _ => {}
    }
    Ok(LaunchArguments {
        mode,
        execute,
        wait_timeout_ms: if wait_for_exit { wait_timeout_ms } else { None },
    })
}

fn execute_with_runner(
    prepared: &PreparedClientRun,
    runner: &dyn ProcessRunner,
    cancellation: CancellationToken,
) -> DomainResult {
    let tool = match resolve_bundled_runner(&prepared.context.cwd) {
        Ok(resolved) => resolved.tool,
        Err(message) => return reject_absent_runner(message),
    };
    execute_with_resolved_runner(prepared, runner, cancellation, &tool)
}

fn execute_with_resolved_runner(
    prepared: &PreparedClientRun,
    runner: &dyn ProcessRunner,
    cancellation: CancellationToken,
    tool: &BundledTool,
) -> DomainResult {
    if let Err(error) =
        digest_required_workspace_file(&prepared.context.workspace_root, Path::new(CONFIG_NAME))
    {
        let mut result = reject(RefusalCode::InvalidState, error);
        result.next.push(json!({
            "tool": "unica.view",
            "args": {},
            "reason": "inspect workspace setup and the required v8project.yaml recipe"
        }));
        return result;
    }
    if cancellation.is_cancelled() {
        return reject(RefusalCode::Cancelled, "client.run cancelled before launch");
    }
    let envelope = match invoke_runner(prepared, tool, runner, &cancellation) {
        Ok(envelope) => envelope,
        Err(result) => return result,
    };
    let data = &envelope["data"];
    if data["mode"] != prepared.arguments.mode.as_str() {
        return reject(
            RefusalCode::InvalidResult,
            "v8-runner answered for a different client mode",
        );
    }
    let platform = match platform_of(data) {
        Some(platform) => platform,
        None => {
            return reject(
                RefusalCode::InvalidResult,
                "v8-runner did not name the platform installation it selected",
            )
        }
    };
    if prepared.dry_run {
        if data["provider_dispatched"] != false
            || !data["pid"].is_null()
            || data["plan"]["program"].as_str().is_none_or(str::is_empty)
        {
            return reject(
                RefusalCode::InvalidResult,
                "v8-runner preview did not prove that no client was dispatched",
            );
        }
        let mut result = DomainResult::success(format!(
            "client.run planned a {} session without launching a client",
            prepared.arguments.mode.as_str()
        ));
        result.data = Some(json!({
            "op": OPERATION,
            "dryRun": true,
            "mode": prepared.arguments.mode.as_str(),
            "execute": execute_text(prepared),
            "wait": wait_plan(prepared),
            "platform": platform,
            "providerDispatched": false,
            "requiresPlatform": true,
        }));
        result.next.push(json!({
            "tool": "unica.run",
            "args": {
                "op": OPERATION,
                "args": public_arguments(prepared),
                "dryRun": false,
            },
            "reason": "launch exactly this previewed client session"
        }));
        return result;
    }
    if data["provider_dispatched"] != true {
        return reject(
            RefusalCode::InvalidResult,
            "v8-runner reported success without dispatching the client",
        );
    }
    let wait = &data["external_epf_wait"];
    let pid = if prepared.arguments.wait_timeout_ms.is_some() {
        wait["pid"].as_u64()
    } else {
        data["pid"].as_u64()
    };
    let Some(pid) = pid.filter(|pid| *pid > 0) else {
        return reject(
            RefusalCode::InvalidResult,
            "v8-runner dispatched the client without reporting its process",
        );
    };
    let waited = if prepared.arguments.wait_timeout_ms.is_some() {
        let Some(timed_out) = wait["timed_out"].as_bool() else {
            return reject(
                RefusalCode::InvalidResult,
                "v8-runner waited for the processor without reporting the outcome",
            );
        };
        Some(json!({
            "exitCode": wait["exit_code"],
            "timedOut": timed_out,
        }))
    } else {
        None
    };
    let summary = match &waited {
        Some(waited) if waited["timedOut"] == true => format!(
            "client.run ran the processor in a {} session and gave up waiting after {} ms",
            prepared.arguments.mode.as_str(),
            prepared.arguments.wait_timeout_ms.unwrap_or_default()
        ),
        Some(waited) => format!(
            "client.run ran the processor in a {} session; it exited with code {}",
            prepared.arguments.mode.as_str(),
            waited["exitCode"]
        ),
        None => format!(
            "client.run launched a {} session (pid {pid})",
            prepared.arguments.mode.as_str()
        ),
    };
    let mut result = DomainResult::success(summary);
    result.data = Some(json!({
        "op": OPERATION,
        "dryRun": false,
        "mode": prepared.arguments.mode.as_str(),
        "execute": execute_text(prepared),
        "pid": pid,
        "platform": platform,
        "providerDispatched": true,
        "wait": waited,
    }));
    // Сеанс — внешний эффект, не файл рабочего пространства: путь сюда не
    // кладётся, иначе запись читалась бы как правка исходников.
    result.changed.push(json!({
        "clientSession": true,
        "mode": prepared.arguments.mode.as_str(),
        "pid": pid,
    }));
    result
}

fn invoke_runner(
    prepared: &PreparedClientRun,
    tool: &BundledTool,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
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
        prepared.arguments.mode.as_str().to_string(),
    ];
    if let Some((_, absolute)) = &prepared.arguments.execute {
        args.extend(["--execute".to_string(), absolute.display().to_string()]);
    }
    if prepared.dry_run {
        args.push("--dry-run".to_string());
    } else if let Some(timeout) = prepared.arguments.wait_timeout_ms {
        args.extend([
            "--wait-for-exit".to_string(),
            "--wait-timeout-ms".to_string(),
            timeout.to_string(),
        ]);
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
    parse_runner_output(output, prepared.dry_run)
}

fn parse_runner_output(output: ProcessOutput, dry_run: bool) -> Result<Value, DomainResult> {
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
        // Отказ превью, которое всё же запустило клиента, — не отказ раннера,
        // а нарушение его же контракта: и он важнее любого кода ошибки.
        if dry_run && envelope["data"]["provider_dispatched"] == true {
            return Err(reject(
                RefusalCode::InvalidResult,
                "failed v8-runner preview did not prove that no client was dispatched",
            ));
        }
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

/// Выбранная платформа без путей: версия и способ, которым раннер её нашёл.
/// Путь к бинарю и корень установки наружу не идут — как командная строка.
fn platform_of(data: &Value) -> Option<Value> {
    let resolution = &data["platform_resolution"];
    let source = resolution["source"].as_str()?;
    if !matches!(source, "explicit" | "default-root" | "path") {
        return None;
    }
    Some(json!({
        "version": resolution["version"].as_str(),
        "source": source,
    }))
}

fn execute_text(prepared: &PreparedClientRun) -> Value {
    match &prepared.arguments.execute {
        Some((relative, _)) => Value::String(relative.to_string_lossy().replace('\\', "/")),
        None => Value::Null,
    }
}

fn wait_plan(prepared: &PreparedClientRun) -> Value {
    match prepared.arguments.wait_timeout_ms {
        Some(timeout) => json!({"timeoutMs": timeout}),
        None => Value::Null,
    }
}

fn public_arguments(prepared: &PreparedClientRun) -> Value {
    let mut args = Map::new();
    args.insert(
        "clientMode".to_string(),
        Value::String(prepared.arguments.mode.as_str().to_string()),
    );
    if let Value::String(execute) = execute_text(prepared) {
        args.insert("execute".to_string(), Value::String(execute));
    }
    if let Some(timeout) = prepared.arguments.wait_timeout_ms {
        args.insert("waitForExit".to_string(), Value::Bool(true));
        args.insert("waitTimeoutMs".to_string(), json!(timeout));
    }
    Value::Object(args)
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
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    struct SequenceRunner {
        outputs: Mutex<Vec<ProcessOutput>>,
        calls: Mutex<Vec<ProcessCommand>>,
    }

    impl SequenceRunner {
        fn new(outputs: Vec<ProcessOutput>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into_iter().rev().collect()),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl ProcessRunner for SequenceRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.calls.lock().unwrap().push(command.clone());
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
        fs::write(root.path().join(CONFIG_NAME), "format: DESIGNER\n").unwrap();
        fs::create_dir_all(root.path().join("tools")).unwrap();
        fs::write(root.path().join("tools/Report.epf"), b"epf").unwrap();
        root
    }

    fn tool(root: &Path) -> BundledTool {
        BundledTool {
            program: root.join("v8-runner"),
            warnings: Vec::new(),
            missing: None,
        }
    }

    fn prepared(root: &Path, args: Value, dry_run: bool) -> PreparedClientRun {
        let context = context(root);
        let arguments = parse_launch_arguments(args.as_object().unwrap(), &context)
            .unwrap_or_else(|result| panic!("valid launch arguments: {result:?}"));
        PreparedClientRun {
            arguments,
            dry_run,
            context,
        }
    }

    /// Конверт превью, снятый с v8-runner 0.9.0 на 8.3.27.2074 14.09.2026.
    fn preview_envelope(mode: &str, execute: Option<&str>) -> Value {
        let mut args = vec![
            if mode == "designer" {
                "DESIGNER"
            } else {
                "ENTERPRISE"
            }
            .to_string(),
            "/DisableStartupDialogs".to_string(),
            "/IBConnectionString".to_string(),
            "File=/opt/ib/sendbox".to_string(),
        ];
        if let Some(execute) = execute {
            args.extend(["/Execute".to_string(), execute.to_string()]);
        }
        json!({
            "ok": true,
            "command": "launch",
            "duration_ms": 7,
            "data": {
                "ok": true,
                "mode": mode,
                "pid": null,
                "binary": "/opt/1cv8/8.3.27.2074/1cv8",
                "platform_resolution": {
                    "path": "/opt/1cv8/8.3.27.2074/1cv8",
                    "version": "8.3.27.2074",
                    "source": "default-root",
                    "installation_root": "/opt/1cv8/8.3.27.2074"
                },
                "provider_dispatched": false,
                "plan": {"program": "/opt/1cv8/8.3.27.2074/1cv8", "args": args},
                "message": "Previewed; client process not dispatched"
            },
            "warnings": [],
            "steps": []
        })
    }

    fn launched_envelope(mode: &str, pid: u64, wait: Option<Value>) -> Value {
        let mut envelope = preview_envelope(mode, None);
        envelope["data"]["pid"] = json!(pid);
        envelope["data"]["provider_dispatched"] = json!(true);
        envelope["data"]["plan"] = Value::Null;
        envelope["data"].as_object_mut().unwrap().remove("plan");
        if let Some(wait) = wait {
            envelope["data"]["external_epf_wait"] = wait;
        }
        envelope
    }

    #[test]
    fn arguments_are_closed_and_each_refusal_names_the_fix() {
        let root = workspace();
        let context = context(root.path());
        for (args, expected) in [
            (json!({}), "clientMode must be one of"),
            (json!({"clientMode": "web"}), "clientMode must be one of"),
            (
                json!({"clientMode": "thin", "mcpPort": 1}),
                "does not accept `mcpPort`",
            ),
            (
                json!({"clientMode": "designer", "execute": "tools/Report.epf"}),
                "designer runs no external processor",
            ),
            (
                json!({"clientMode": "thin", "execute": "/abs/Report.epf"}),
                "must be workspace-relative",
            ),
            (
                json!({"clientMode": "thin", "execute": "../Report.epf"}),
                "parent traversal",
            ),
            (
                json!({"clientMode": "thin", "execute": "tools/Report.txt"}),
                ".epf or .erf",
            ),
            (
                json!({"clientMode": "thin", "execute": "tools/Missing.epf"}),
                "does not exist",
            ),
            (
                json!({"clientMode": "thin", "waitForExit": true}),
                "requires waitTimeoutMs",
            ),
            (
                json!({"clientMode": "thin", "waitTimeoutMs": 500}),
                "requires waitForExit",
            ),
            (
                json!({"clientMode": "thin", "waitForExit": true, "waitTimeoutMs": 500}),
                "needs execute",
            ),
            (
                json!({"clientMode": "thin", "execute": "tools/Report.epf", "waitForExit": true, "waitTimeoutMs": 0}),
                "from 1 to",
            ),
        ] {
            let result = parse_launch_arguments(args.as_object().unwrap(), &context)
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
        let accepted = parse_launch_arguments(
            json!({"clientMode": "thick", "execute": "tools/Report.epf", "waitForExit": true, "waitTimeoutMs": 1500})
                .as_object()
                .unwrap(),
            &context,
        )
        .unwrap();
        assert_eq!(accepted.mode, ClientMode::Thick);
        assert_eq!(accepted.wait_timeout_ms, Some(1500));
        assert_eq!(
            accepted
                .execute
                .as_ref()
                .map(|(relative, _)| relative.clone()),
            Some(PathBuf::from("tools/Report.epf"))
        );
    }

    #[test]
    fn preview_names_the_platform_without_dispatching_or_exposing_the_command() {
        let root = workspace();
        let prepared = prepared(
            root.path(),
            json!({"clientMode": "thin", "execute": "tools/Report.epf"}),
            true,
        );
        let execute = root.path().join("tools/Report.epf");
        let runner = SequenceRunner::new(vec![process(
            preview_envelope("thin", Some(&execute.display().to_string())),
            true,
        )]);

        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
        );

        assert!(result.ok, "{result:?}");
        assert!(
            result.rev.is_none(),
            "a terminal operation has no revision to fence"
        );
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["dryRun"], true);
        assert_eq!(data["mode"], "thin");
        assert_eq!(data["execute"], "tools/Report.epf");
        assert_eq!(data["providerDispatched"], false);
        assert_eq!(data["platform"]["version"], "8.3.27.2074");
        assert_eq!(data["platform"]["source"], "default-root");
        assert_eq!(result.next[0]["args"]["dryRun"], false);
        assert_eq!(result.next[0]["args"]["args"]["clientMode"], "thin");
        assert!(result.next[0]["args"].get("ifRev").is_none());
        let call = &runner.calls.lock().unwrap()[0];
        assert!(call.args.iter().any(|argument| argument == "--dry-run"));
        assert!(call.args.iter().any(|argument| argument == "launch"));
        assert!(!call
            .args
            .iter()
            .any(|argument| argument == "--wait-for-exit"));
        let encoded = serde_json::to_string(&result).unwrap();
        for leaked in ["1cv8", "/IBConnectionString", "File=", "--config", "/opt/"] {
            assert!(!encoded.contains(leaked), "{leaked} leaked: {encoded}");
        }
        assert!(!encoded.contains(&root.path().display().to_string()));
    }

    #[test]
    fn launch_reports_the_session_the_provider_attests() {
        let root = workspace();
        let prepared = prepared(root.path(), json!({"clientMode": "designer"}), false);
        let runner = SequenceRunner::new(vec![process(
            launched_envelope("designer", 4242, None),
            true,
        )]);

        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
        );

        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["pid"], 4242);
        assert_eq!(data["providerDispatched"], true);
        assert!(data["wait"].is_null());
        assert_eq!(result.changed[0]["clientSession"], true);
        assert_eq!(result.changed[0]["pid"], 4242);
        assert!(result.artifacts.is_empty());
        let call = &runner.calls.lock().unwrap()[0];
        assert!(!call.args.iter().any(|argument| argument == "--dry-run"));
    }

    #[test]
    fn waited_launch_reports_the_exit_code_and_the_timeout() {
        let root = workspace();
        let prepared = prepared(
            root.path(),
            json!({"clientMode": "thin", "execute": "tools/Report.epf", "waitForExit": true, "waitTimeoutMs": 1500}),
            false,
        );
        let runner = SequenceRunner::new(vec![
            process(
                launched_envelope(
                    "thin",
                    77,
                    Some(json!({
                        "pid": 77,
                        "execute_path": "/w/tools/Report.epf",
                        "exit_code": 3,
                        "timed_out": false,
                        "output_path": "/w/build/logs/out.log",
                        "stderr_path": "/w/build/logs/err.log"
                    })),
                ),
                true,
            ),
            process(
                launched_envelope(
                    "thin",
                    78,
                    Some(json!({
                        "pid": 78,
                        "execute_path": "/w/tools/Report.epf",
                        "exit_code": null,
                        "timed_out": true,
                        "output_path": "/w/build/logs/out.log",
                        "stderr_path": "/w/build/logs/err.log"
                    })),
                ),
                true,
            ),
        ]);

        let exited = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
        );
        assert!(exited.ok, "{exited:?}");
        assert_eq!(exited.data.as_ref().unwrap()["wait"]["exitCode"], 3);
        assert_eq!(exited.data.as_ref().unwrap()["wait"]["timedOut"], false);
        assert!(exited.summary.contains("exited with code 3"));
        let encoded = serde_json::to_string(&exited).unwrap();
        assert!(
            !encoded.contains("build/logs"),
            "log paths leaked: {encoded}"
        );
        let joined = runner.calls.lock().unwrap()[0].args.join(" ");
        assert!(
            joined.contains("--wait-for-exit --wait-timeout-ms 1500"),
            "{joined}"
        );

        let timed_out = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
        );
        assert!(timed_out.ok, "{timed_out:?}");
        assert_eq!(timed_out.data.as_ref().unwrap()["wait"]["timedOut"], true);
        assert!(timed_out.summary.contains("gave up waiting after 1500 ms"));
    }

    #[test]
    fn a_preview_that_dispatched_a_client_is_refused_as_a_broken_contract() {
        let root = workspace();
        let prepared = prepared(root.path(), json!({"clientMode": "thin"}), true);
        let mut envelope = preview_envelope("thin", None);
        envelope["data"]["provider_dispatched"] = json!(true);
        envelope["data"]["pid"] = json!(9);
        let runner = SequenceRunner::new(vec![process(envelope, true)]);

        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
        );

        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "invalid_result");
    }

    #[test]
    fn runner_refusals_keep_their_outcome_and_a_missing_platform_asks_for_a_human() {
        let root = workspace();
        let prepared = prepared(root.path(), json!({"clientMode": "thin"}), false);
        let runner = SequenceRunner::new(vec![process(
            json!({
                "ok": false,
                "command": "launch",
                "duration_ms": 0,
                "data": {"message": "platform 8.3.27.2074 is not installed"},
                "warnings": [],
                "steps": [],
                "error": {
                    "code": "environment_unavailable",
                    "kind": "environment",
                    "message": "platform 8.3.27.2074 is not installed"
                }
            }),
            false,
        )]);

        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
        );

        assert!(!result.ok);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
        // The runner said the platform is absent: the refusal names that as
        // its detail, so the agent sees the reason, not just the outcome.
        assert_eq!(result.diagnostics[0]["detailCode"], "provider_absent");
        assert_eq!(result.diagnostics[0]["outcome"], "needsHuman");
    }

    #[test]
    fn a_missing_project_file_is_named_before_the_runner_is_called() {
        let root = tempfile::tempdir().unwrap();
        let prepared = prepared(root.path(), json!({"clientMode": "thin"}), true);
        let runner = SequenceRunner::new(Vec::new());

        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
        );

        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "invalid_state");
        assert_eq!(result.next[0]["tool"], "unica.view");
        assert!(runner.calls.lock().unwrap().is_empty());
    }
}
