#![allow(clippy::result_large_err)]
//! `launch` — единственная терминальная операция словаря `run`: запуск
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
use super::runner_011::Runner011ProcessRunner;
use super::v13_infobase_exports::{
    closed_workspace_relative_path, digest_optional_workspace_file, digest_required_workspace_file,
    missing_runner_rejection, resolve_bundled_runner, runner_rejection, CONFIG_NAME,
    RUNNER_OUTPUT_LIMIT,
};
use crate::application::invocation_store::ToolIdentity;
use crate::domain::cancellation::{CancellationToken, CANCELLED_PREFIX};
use crate::domain::invocation::{DomainResult, SafeIdentityHash};
use crate::domain::refusal::RefusalCode;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bundled_tools::BundledTool;
use crate::infrastructure::internal_adapters::{ProcessCommand, ProcessOutput, ProcessRunner};
use crate::infrastructure::platform::PendingProcessHandoff;
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) const OPERATION: &str = "launch";
/// Имя команды в конверте раннера: у `launch` оно не совпадает с именем
/// операции словаря, в отличие от семейства `infobase`.
const RUNNER_COMMAND: &str = "launch";
const WAIT_TIMEOUT_MAX_MS: u64 = 86_400_000;
const PROCESSOR_EXTENSIONS: [&str; 2] = ["epf", "erf"];
const CLIENT_OWNER_ENV: &str = "V8_RUNNER_CLIENT_OWNER";

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
                    "launch dryRun must be boolean",
                ))
            }
        };
        if arguments.get("ifRev").is_some() {
            return Err(reject(
                RefusalCode::BadValue,
                "launch is terminal and takes no ifRev: it changes neither sources nor the infobase",
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
        execute_with_runner(self, &Runner011ProcessRunner, cancellation)
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
            format!("launch does not accept `{unknown}`; the closed args are clientMode, execute, waitForExit and waitTimeoutMs"),
        ));
    }
    let mode = args
        .get("clientMode")
        .and_then(Value::as_str)
        .and_then(ClientMode::parse)
        .ok_or_else(|| {
            reject(
                RefusalCode::BadValue,
                "launch clientMode must be one of designer, thin, thick, ordinary",
            )
        })?;
    let execute = match args.get("execute") {
        None => None,
        Some(Value::String(value)) => {
            if mode == ClientMode::Designer {
                return Err(reject(
                    RefusalCode::BadValue,
                    "launch execute needs an enterprise client: designer runs no external processor",
                ));
            }
            let relative = closed_workspace_relative_path(value).map_err(|reason| {
                reject(RefusalCode::BadValue, format!("launch execute {reason}"))
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
                    "launch execute must name an .epf or .erf external processor",
                ));
            }
            // Превью раннера существование файла не проверяет: оно составит
            // план и по несуществующему пути. Проверка здесь, до вызова.
            match digest_optional_workspace_file(&context.workspace_root, &relative) {
                Ok(Some((_, size))) if size > 0 => {}
                Ok(Some(_)) => {
                    return Err(reject(
                        RefusalCode::BadValue,
                        "launch execute names an empty file",
                    ))
                }
                Ok(None) => {
                    return Err(reject(
                        RefusalCode::BadValue,
                        "launch execute names a file that does not exist in the workspace",
                    ))
                }
                Err(error) => return Err(reject(RefusalCode::BadValue, error)),
            }
            Some((relative.clone(), context.workspace_root.join(relative)))
        }
        Some(_) => {
            return Err(reject(
                RefusalCode::BadValue,
                "launch execute must be a workspace-relative path",
            ))
        }
    };
    let wait_for_exit = match args.get("waitForExit") {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => {
            return Err(reject(
                RefusalCode::BadValue,
                "launch waitForExit must be boolean",
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
                        format!("launch waitTimeoutMs must be an integer from 1 to {WAIT_TIMEOUT_MAX_MS}"),
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
                "launch waitForExit requires waitTimeoutMs",
            ))
        }
        (false, true, _) => {
            return Err(reject(
                RefusalCode::BadValue,
                "launch waitTimeoutMs requires waitForExit: true",
            ))
        }
        (true, true, false) => {
            return Err(reject(
                RefusalCode::BadValue,
                "launch waitForExit needs execute: only an external processor session has an exit to wait for",
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
        return reject(RefusalCode::Cancelled, "launch cancelled before launch");
    }
    let RunnerInvocation {
        envelope,
        mut handoff,
    } = match invoke_runner(prepared, tool, runner, &cancellation) {
        Ok(invocation) => invocation,
        Err(result) => return result,
    };
    let data = &envelope["data"];
    if prepared.dry_run {
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
        if data["ok"] != true
            || data["provider_dispatched"] != false
            || !data["pid"].is_null()
            || data["plan"]["program"].as_str().is_none_or(str::is_empty)
        {
            return reject(
                RefusalCode::InvalidResult,
                "v8-runner preview did not prove that no client was dispatched",
            );
        }
        let mut result = DomainResult::success(format!(
            "launch planned a {} session without launching a client",
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
    let ValidatedLaunchReceipt {
        pid,
        platform,
        waited,
    } = match ValidatedLaunchReceipt::parse(prepared, data) {
        Ok(receipt) => receipt,
        Err(result) => return result,
    };
    let summary = match &waited {
        Some(waited) if waited["timedOut"] == true => format!(
            "launch ran the processor in a {} session and gave up waiting after {} ms",
            prepared.arguments.mode.as_str(),
            prepared.arguments.wait_timeout_ms.unwrap_or_default()
        ),
        Some(waited) => format!(
            "launch ran the processor in a {} session; it exited with code {}",
            prepared.arguments.mode.as_str(),
            waited["exitCode"]
        ),
        None => format!(
            "launch launched a {} session (pid {pid})",
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
    if let Some(handoff) = &mut handoff {
        if let Err(error) = cancellation.handoff_with_gate(|| handoff.release()) {
            return if error.starts_with(CANCELLED_PREFIX) {
                reject(
                    RefusalCode::Cancelled,
                    "launch cancelled before process handoff",
                )
            } else {
                reject(
                    RefusalCode::ProviderFailed,
                    "cannot transfer ownership of the launched client",
                )
            };
        }
    }
    result
}

struct ValidatedLaunchReceipt {
    pid: u32,
    platform: Value,
    waited: Option<Value>,
}

impl ValidatedLaunchReceipt {
    fn parse(prepared: &PreparedClientRun, data: &Value) -> Result<Self, DomainResult> {
        if data["ok"] != true || data["mode"] != prepared.arguments.mode.as_str() {
            return Err(reject(
                RefusalCode::InvalidResult,
                "v8-runner returned an inconsistent launch result",
            ));
        }
        let platform = platform_of(data).ok_or_else(|| {
            reject(
                RefusalCode::InvalidResult,
                "v8-runner did not name the platform installation it selected",
            )
        })?;
        if data["provider_dispatched"] != true {
            return Err(reject(
                RefusalCode::InvalidResult,
                "v8-runner reported success without dispatching the client",
            ));
        }
        let pid = data["pid"]
            .as_u64()
            .and_then(|pid| u32::try_from(pid).ok())
            .filter(|pid| *pid > 0)
            .ok_or_else(|| {
                reject(
                    RefusalCode::InvalidResult,
                    "v8-runner dispatched the client without reporting its process",
                )
            })?;
        let wait = &data["external_epf_wait"];
        let waited = if prepared.arguments.wait_timeout_ms.is_some() {
            if wait["pid"].as_u64() != Some(u64::from(pid)) {
                return Err(reject(
                    RefusalCode::InvalidResult,
                    "v8-runner wait result names a different process",
                ));
            }
            let timed_out = wait["timed_out"].as_bool().ok_or_else(|| {
                reject(
                    RefusalCode::InvalidResult,
                    "v8-runner waited for the processor without reporting the outcome",
                )
            })?;
            Some(json!({"exitCode": wait["exit_code"], "timedOut": timed_out}))
        } else {
            if !wait.is_null() {
                return Err(reject(
                    RefusalCode::InvalidResult,
                    "v8-runner answered with an unrequested wait result",
                ));
            }
            None
        };
        Ok(Self {
            pid,
            platform,
            waited,
        })
    }
}

struct RunnerInvocation {
    envelope: Value,
    handoff: Option<PendingProcessHandoff>,
}

fn invoke_runner(
    prepared: &PreparedClientRun,
    tool: &BundledTool,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
) -> Result<RunnerInvocation, DomainResult> {
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
    let log_directory = if !prepared.dry_run && prepared.arguments.wait_timeout_ms.is_some() {
        let directory = tempfile::Builder::new()
            .prefix("unica-launch-")
            .tempdir()
            .and_then(|directory| {
                let parent = crate::infrastructure::platform::filesystem::open_directory_nofollow(
                    directory.path(),
                )?;
                let logs =
                    crate::infrastructure::platform::filesystem::create_owner_only_directory_child(
                        &parent,
                        std::ffi::OsStr::new("logs"),
                    )?;
                for name in ["output.log", "stderr.log"] {
                    crate::infrastructure::platform::filesystem::create_owner_only_file_child(
                        &logs,
                        std::ffi::OsStr::new(name),
                    )?;
                }
                Ok(directory)
            })
            .map_err(|_| {
                reject(
                    RefusalCode::InvalidState,
                    "cannot create private launch logs",
                )
            })?;
        Some(directory)
    } else {
        None
    };
    if prepared.dry_run {
        args.push("--dry-run".to_string());
    } else if let (Some(timeout), Some(directory)) =
        (prepared.arguments.wait_timeout_ms, &log_directory)
    {
        args.extend([
            "--wait-for-exit".to_string(),
            "--wait-timeout-ms".to_string(),
            timeout.to_string(),
            "--output".to_string(),
            directory
                .path()
                .join("logs/output.log")
                .display()
                .to_string(),
            "--stderr-output".to_string(),
            directory
                .path()
                .join("logs/stderr.log")
                .display()
                .to_string(),
        ]);
    }
    let owns_client = !prepared.dry_run && prepared.arguments.wait_timeout_ms.is_none();
    let command = ProcessCommand {
        program: tool.program.clone(),
        args,
        cwd: prepared.context.workspace_root.clone(),
        env: if owns_client {
            vec![(CLIENT_OWNER_ENV.into(), "unica".into())]
        } else {
            Vec::new()
        },
        env_remove: vec![CLIENT_OWNER_ENV.into()],
        capture_limits: Some((RUNNER_OUTPUT_LIMIT, RUNNER_OUTPUT_LIMIT)),
        timeout: None,
        cancellation: cancellation.clone(),
    };
    let execution = if owns_client {
        runner
            .run_pending_handoff(&command)
            .map(|(output, handoff)| (output, Some(handoff)))
    } else {
        runner.run(&command).map(|output| (output, None))
    };
    let (output, handoff) = execution.map_err(|error| {
        if error.starts_with(CANCELLED_PREFIX) || cancellation.is_cancelled() {
            reject(
                RefusalCode::Cancelled,
                "launch cancelled while reading the runner result",
            )
        } else if owns_client {
            reject(
                RefusalCode::ProviderFailed,
                "v8-runner could not complete the owned launch",
            )
        } else {
            reject_absent_runner("failed to run bundled v8-runner for launch")
        }
    })?;
    let envelope = parse_runner_output(output, prepared.dry_run)?;
    Ok(RunnerInvocation { envelope, handoff })
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
        // Runner errors may contain its command line and private capture paths.
        // The typed code supplies the continuation; raw provider text is private.
        let message = "v8-runner refused the launch".to_string();
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
        released: Arc<std::sync::atomic::AtomicUsize>,
        discarded: Arc<std::sync::atomic::AtomicUsize>,
        cancel_after_output: bool,
    }

    impl SequenceRunner {
        fn new(outputs: Vec<ProcessOutput>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into_iter().rev().collect()),
                calls: Mutex::new(Vec::new()),
                released: Arc::default(),
                discarded: Arc::default(),
                cancel_after_output: false,
            }
        }
    }

    impl ProcessRunner for SequenceRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.calls.lock().unwrap().push(command.clone());
            Ok(self.outputs.lock().unwrap().pop().expect("runner output"))
        }

        fn run_pending_handoff(
            &self,
            command: &ProcessCommand,
        ) -> Result<(ProcessOutput, PendingProcessHandoff), String> {
            let output = self.run(command)?;
            if self.cancel_after_output {
                command.cancellation.cancel();
            }
            Ok((
                output,
                PendingProcessHandoff::for_test(
                    Arc::clone(&self.released),
                    Arc::clone(&self.discarded),
                ),
            ))
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

    fn fake_launch_executable() -> (tempfile::TempDir, BundledTool) {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("fake_launch.rs");
        let program = crate::infrastructure::platform::testing::fixture_executable_path(
            directory.path(),
            "fake-launch",
        );
        fs::write(
            &source,
            r#"
use std::{env, fs, path::Path, thread};
fn main() {
    let args: Vec<String> = env::args().collect();
    let value = |flag: &str| -> &str {
        let index = args.iter().position(|arg| arg == flag).expect(flag);
        args.get(index + 1).expect(flag)
    };
    assert!(args.iter().any(|arg| arg == "--wait-for-exit"));
    assert_eq!(value("--wait-timeout-ms"), "1500");
    assert!(Path::new(value("--execute")).is_file());
    let output = Path::new(value("--output"));
    let stderr = Path::new(value("--stderr-output"));
    assert!(output.is_absolute() && stderr.is_absolute());
    assert_ne!(output, stderr);
    assert_eq!(output.parent(), stderr.parent());
    assert!(output.parent().unwrap().is_dir());
    let scenario = fs::read_to_string("scenario").unwrap();
    if scenario != "runner-timeout" {
        fs::write(output, "PRIVATE-CLIENT-OUTPUT").unwrap();
        fs::write(stderr, "PRIVATE-CLIENT-STDERR").unwrap();
    }
    assert_eq!(fs::read_to_string(output).unwrap(), "PRIVATE-CLIENT-OUTPUT");
    assert_eq!(fs::read_to_string(stderr).unwrap(), "PRIVATE-CLIENT-STDERR");
    fs::write("observed-paths", format!("{}\n{}", output.display(), stderr.display())).unwrap();
    eprintln!("PRIVATE-RUNNER-STDERR {} {}", output.display(), stderr.display());
    if scenario == "runner-timeout" || scenario == "runner-cancel" {
        loop { thread::park(); }
    }
    let escape = |path: &Path| path.to_str().unwrap().replace('\\', "\\\\").replace('"', "\\\"");
    let reply = fs::read_to_string("reply.json").unwrap()
        .replace("OUTPUT_PATH", &escape(output))
        .replace("STDERR_PATH", &escape(stderr));
    fs::write(output, "PRIVATE-CLIENT-OUTPUT-terminal").unwrap();
    fs::write(stderr, "PRIVATE-CLIENT-STDERR-terminal").unwrap();
    println!("{reply}");
    if scenario == "refusal" { std::process::exit(2); }
    if scenario == "timeout" { std::process::exit(3); }
}
"#,
        )
        .unwrap();
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let compilation = std::process::Command::new(rustc)
            .arg("--edition=2021")
            .arg(&source)
            .arg("-o")
            .arg(&program)
            .output()
            .expect("compile fake launch executable");
        assert!(
            compilation.status.success(),
            "{}",
            String::from_utf8_lossy(&compilation.stderr)
        );
        (
            directory,
            BundledTool {
                program,
                warnings: Vec::new(),
                missing: None,
            },
        )
    }

    #[derive(Default)]
    struct ObservedSystemRunner {
        paths: Mutex<Vec<(PathBuf, PathBuf)>>,
        timeout: bool,
        cancel_after_output: bool,
    }

    impl ProcessRunner for ObservedSystemRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            let path = |flag: &str| {
                let index = command
                    .args
                    .iter()
                    .position(|arg| arg == flag)
                    .unwrap_or_else(|| panic!("waited launch is missing {flag}"));
                PathBuf::from(&command.args[index + 1])
            };
            let output = path("--output");
            let stderr = path("--stderr-output");
            assert_ne!(output, stderr);
            let directory = output.parent().unwrap();
            assert_eq!(Some(directory), stderr.parent());
            assert!(directory.is_dir());
            assert!(!directory.starts_with(&command.cwd));
            for path in [&output, &stderr] {
                let file =
                    fs::File::open(path).expect("private log must exist before runner starts");
                assert_eq!(file.metadata().unwrap().len(), 0);
                crate::infrastructure::platform::filesystem::verify_owner_only_acl(&file).unwrap();
            }
            if let Some(mode) =
                crate::infrastructure::platform::testing::unix_mode_for_test(directory).unwrap()
            {
                assert_eq!(mode & 0o077, 0, "launch output directory must be private");
            }
            self.paths
                .lock()
                .unwrap()
                .push((output.clone(), stderr.clone()));
            let mut command = command.clone();
            if self.timeout {
                // Timeout can precede the fixture's first instruction. Seed
                // private bytes here; that scenario never truncates them.
                assert_eq!(
                    fs::read_to_string(command.cwd.join("scenario")).unwrap(),
                    "runner-timeout"
                );
                fs::write(&output, "PRIVATE-CLIENT-OUTPUT").unwrap();
                fs::write(&stderr, "PRIVATE-CLIENT-STDERR").unwrap();
                command.timeout = Some(std::time::Duration::from_secs(1));
            }
            let result = std::thread::scope(|scope| {
                if self.cancel_after_output {
                    scope.spawn(|| {
                        let ready = command.cwd.join("observed-paths");
                        let deadline =
                            std::time::Instant::now() + std::time::Duration::from_secs(5);
                        while !ready.exists() {
                            if std::time::Instant::now() >= deadline {
                                command.cancellation.cancel();
                                panic!("fixture did not open its logs before cancellation");
                            }
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                        command.cancellation.cancel();
                    });
                }
                crate::infrastructure::internal_adapters::SystemProcessRunner.run(&command)
            });
            if let Ok(result) = &result {
                let suffix = if result.timed_out || result.cancelled {
                    ""
                } else {
                    "-terminal"
                };
                assert_eq!(
                    fs::read_to_string(&output).unwrap(),
                    format!("PRIVATE-CLIENT-OUTPUT{suffix}")
                );
                assert_eq!(
                    fs::read_to_string(&stderr).unwrap(),
                    format!("PRIVATE-CLIENT-STDERR{suffix}")
                );
                for path in [&output, &stderr] {
                    let file = fs::File::open(path).unwrap();
                    crate::infrastructure::platform::filesystem::verify_owner_only_acl(&file)
                        .unwrap();
                }
            }
            result
        }
    }

    fn assert_private_launch_cleanup(result: &DomainResult, paths: &(PathBuf, PathBuf)) {
        let encoded = serde_json::to_string(result).unwrap();
        assert!(result.artifacts.is_empty(), "{result:?}");
        for path in [
            &paths.0,
            &paths.1,
            paths.0.parent().unwrap(),
            paths.0.parent().unwrap().parent().unwrap(),
        ] {
            assert!(
                !path.exists(),
                "private output survived terminal result: {path:?}"
            );
            let json_path = serde_json::to_string(&path.to_string_lossy()).unwrap();
            assert!(
                !encoded.contains(json_path.trim_matches('"')),
                "path leaked: {encoded}"
            );
        }
        for text in [
            "PRIVATE-CLIENT-",
            "PRIVATE-RUNNER-",
            "--output",
            "--stderr-output",
            "output_path",
            "stderr_path",
        ] {
            assert!(!encoded.contains(text), "private output leaked: {encoded}");
        }
    }

    #[test]
    fn waited_launch_real_process_has_private_writable_logs_until_terminal_and_cleans_up() {
        let (_executable_directory, executable) = fake_launch_executable();
        let runner = ObservedSystemRunner::default();
        for scenario in [
            "success",
            "nonzero",
            "timeout",
            "refusal",
            "malformed",
            "invalid-result",
        ] {
            let root = workspace();
            let prepared = prepared(
                root.path(),
                json!({"clientMode": "thin", "execute": "tools/Report.epf", "waitForExit": true, "waitTimeoutMs": 1500}),
                false,
            );
            let mut envelope = launched_envelope(
                "thin",
                77,
                Some(json!({
                    "pid": 77,
                    "exit_code": if scenario == "timeout" { Value::Null } else { json!(if scenario == "nonzero" { 3 } else { 0 }) },
                    "timed_out": scenario == "timeout",
                    "output_path": "OUTPUT_PATH",
                    "stderr_path": "STDERR_PATH",
                    "stdout": "PRIVATE-CLIENT-OUTPUT",
                    "stderr": "PRIVATE-CLIENT-STDERR",
                })),
            );
            if scenario == "refusal" {
                envelope["ok"] = json!(false);
                envelope["error"] = json!({
                    "code": "platform_failure",
                    "message": "OUTPUT_PATH STDERR_PATH PRIVATE-CLIENT-OUTPUT PRIVATE-CLIENT-STDERR",
                });
            }
            // v8-runner 0.11.3 reports an EPF wait timeout as runtime_failure,
            // with a nonzero process status, rather than a successful receipt.
            if scenario == "timeout" {
                envelope["ok"] = json!(false);
                envelope["data"]["ok"] = json!(false);
                envelope["error"] = json!({
                    "code": "runtime_failure",
                    "kind": "runtime",
                    "message": "OUTPUT_PATH STDERR_PATH PRIVATE-CLIENT-OUTPUT wait timed out",
                });
            }
            if scenario == "invalid-result" {
                envelope["data"]["mode"] = json!("designer");
            }
            fs::write(root.path().join("scenario"), scenario).unwrap();
            fs::write(
                root.path().join("reply.json"),
                if scenario == "malformed" {
                    "not-json OUTPUT_PATH STDERR_PATH PRIVATE-CLIENT-OUTPUT".to_string()
                } else {
                    serde_json::to_string(&envelope).unwrap()
                },
            )
            .unwrap();
            let result = execute_with_resolved_runner(
                &prepared,
                &runner,
                CancellationToken::new(),
                &executable,
            );
            let paths = runner.paths.lock().unwrap().last().unwrap().clone();
            assert_eq!(
                fs::read_to_string(root.path().join("observed-paths")).unwrap(),
                format!("{}\n{}", paths.0.display(), paths.1.display())
            );
            assert_private_launch_cleanup(&result, &paths);
            match scenario {
                "timeout" => {
                    assert!(!result.ok, "{result:?}");
                    assert_eq!(result.diagnostics[0]["code"], "provider_failed");
                    assert!(result.changed.is_empty());
                }
                "refusal" => {
                    assert!(!result.ok, "{result:?}");
                    assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
                    assert!(result.changed.is_empty());
                }
                "malformed" | "invalid-result" => {
                    assert!(!result.ok, "{result:?}");
                    assert_eq!(result.diagnostics[0]["code"], "invalid_result");
                }
                _ => {
                    assert!(result.ok, "{result:?}");
                    let data = result.data.as_ref().unwrap();
                    assert_eq!(data["pid"], 77);
                    assert_eq!(data["wait"]["timedOut"], false);
                    assert_eq!(
                        data["wait"]["exitCode"],
                        envelope["data"]["external_epf_wait"]["exit_code"]
                    );
                }
            }
            assert_eq!(
                fs::read(root.path().join(CONFIG_NAME)).unwrap(),
                b"format: DESIGNER\n"
            );
            assert_eq!(
                fs::read(root.path().join("tools/Report.epf")).unwrap(),
                b"epf"
            );
        }
        let paths = runner.paths.lock().unwrap();
        let directories: std::collections::HashSet<_> = paths
            .iter()
            .map(|(output, _)| output.parent().unwrap())
            .collect();
        assert_eq!(
            directories.len(),
            paths.len(),
            "waited applies reused a private directory"
        );
    }

    #[test]
    fn waited_launch_real_process_timeout_cancel_and_spawn_failure_clean_up_private_logs() {
        let (_executable_directory, executable) = fake_launch_executable();
        for scenario in ["runner-timeout", "runner-cancel", "spawn-failure"] {
            let root = workspace();
            fs::write(root.path().join("scenario"), scenario).unwrap();
            let prepared = prepared(
                root.path(),
                json!({"clientMode": "thin", "execute": "tools/Report.epf", "waitForExit": true, "waitTimeoutMs": 1500}),
                false,
            );
            let runner = ObservedSystemRunner {
                timeout: scenario == "runner-timeout",
                cancel_after_output: scenario == "runner-cancel",
                ..Default::default()
            };
            let missing = tool(root.path());
            let result = execute_with_resolved_runner(
                &prepared,
                &runner,
                CancellationToken::new(),
                if scenario == "spawn-failure" {
                    &missing
                } else {
                    &executable
                },
            );
            assert!(!result.ok, "{result:?}");
            assert_eq!(
                result.diagnostics[0]["code"],
                match scenario {
                    "spawn-failure" => "provider_unavailable",
                    "runner-cancel" => "cancelled",
                    _ => "deadline_exceeded",
                }
            );
            assert_private_launch_cleanup(&result, &runner.paths.lock().unwrap()[0]);
            assert!(!serde_json::to_string(&result)
                .unwrap()
                .contains(&root.path().display().to_string()));
        }
    }

    #[test]
    fn waited_preview_and_nonwait_launch_omit_private_output_and_wait_flags() {
        let root = workspace();
        for dry_run in [true, false] {
            let args = if dry_run {
                json!({"clientMode": "thin", "execute": "tools/Report.epf", "waitForExit": true, "waitTimeoutMs": 1500})
            } else {
                json!({"clientMode": "thin", "execute": "tools/Report.epf"})
            };
            let prepared = prepared(root.path(), args.clone(), dry_run);
            let runner = SequenceRunner::new(vec![process(
                if dry_run {
                    preview_envelope("thin", None)
                } else {
                    launched_envelope("thin", 77, None)
                },
                true,
            )]);
            let result = execute_with_resolved_runner(
                &prepared,
                &runner,
                CancellationToken::new(),
                &tool(root.path()),
            );
            assert!(result.ok, "{result:?}");
            let calls = runner.calls.lock().unwrap();
            for flag in [
                "--output",
                "--stderr-output",
                "--wait-for-exit",
                "--wait-timeout-ms",
            ] {
                assert!(
                    !calls[0].args.iter().any(|arg| arg == flag),
                    "unexpected {flag}"
                );
            }
            if dry_run {
                assert_eq!(result.next[0]["args"]["args"], args);
            }
        }
        for field in ["output", "stderrOutput", "stderr-output", "logDirectory"] {
            let mut args = json!({"clientMode": "thin", "execute": "tools/Report.epf", "waitForExit": true, "waitTimeoutMs": 1500});
            args[field] = json!("private.log");
            let result = parse_launch_arguments(args.as_object().unwrap(), &context(root.path()))
                .unwrap_err();
            assert_eq!(result.diagnostics[0]["code"], "bad_value");
        }
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
                json!({"clientMode": "thin", "execute": root.path().join("tools/Report.epf").to_string_lossy()}),
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
        assert_eq!(call.env, vec![(CLIENT_OWNER_ENV.into(), "unica".into())]);
        assert_eq!(
            runner.released.load(std::sync::atomic::Ordering::Acquire),
            1
        );
        assert_eq!(
            runner.discarded.load(std::sync::atomic::Ordering::Acquire),
            0
        );
    }

    #[test]
    fn unverified_launch_receipts_discard_owned_processes() {
        let root = workspace();
        let prepared = prepared(root.path(), json!({"clientMode": "thin"}), false);
        let valid = launched_envelope("thin", 4242, None);
        let mutations = [
            ("/ok", json!(false)),
            ("/data/ok", json!(false)),
            ("/data/mode", json!("designer")),
            ("/data/provider_dispatched", json!(false)),
            ("/data/pid", json!(0)),
            ("/data/pid", json!(-1)),
            ("/data/pid", json!(u64::from(u32::MAX) + 1)),
            ("/data/platform_resolution/source", json!("unknown")),
        ];
        for (pointer, value) in mutations {
            let mut envelope = valid.clone();
            *envelope.pointer_mut(pointer).unwrap() = value;
            let runner = SequenceRunner::new(vec![process(envelope, true)]);
            let token = CancellationToken::new();
            let result =
                execute_with_resolved_runner(&prepared, &runner, token.clone(), &tool(root.path()));
            assert!(!result.ok, "{pointer}: {result:?}");
            assert!(!token.protected_process_started());
            assert_eq!(
                runner.released.load(std::sync::atomic::Ordering::Acquire),
                0,
                "{pointer}"
            );
            assert_eq!(
                runner.discarded.load(std::sync::atomic::Ordering::Acquire),
                1,
                "{pointer}"
            );
        }
        for (case, mut output) in [
            process(valid.clone(), false),
            process(valid.clone(), true),
            process(valid.clone(), true),
            process(valid.clone(), true),
        ]
        .into_iter()
        .enumerate()
        {
            match case {
                0 => {}
                1 => output.stdout_truncated = true,
                2 => output.stdout_had_invalid_utf8 = true,
                _ => output.stdout = "{incomplete".into(),
            }
            let runner = SequenceRunner::new(vec![output]);
            let result = execute_with_resolved_runner(
                &prepared,
                &runner,
                CancellationToken::new(),
                &tool(root.path()),
            );
            assert!(!result.ok);
            assert_eq!(
                runner.released.load(std::sync::atomic::Ordering::Acquire),
                0
            );
            assert_eq!(
                runner.discarded.load(std::sync::atomic::Ordering::Acquire),
                1
            );
        }
        let mut envelope = valid;
        envelope["data"]["external_epf_wait"] =
            json!({"pid": 4242, "timed_out": false, "exit_code": 0});
        let runner = SequenceRunner::new(vec![process(envelope, true)]);
        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
        );
        assert!(!result.ok);
        assert_eq!(
            runner.discarded.load(std::sync::atomic::Ordering::Acquire),
            1
        );
    }

    #[test]
    fn owned_launch_failure_is_not_an_absent_provider_and_has_no_fallback() {
        struct UnsupportedRunner;
        impl ProcessRunner for UnsupportedRunner {
            fn run(&self, _command: &ProcessCommand) -> Result<ProcessOutput, String> {
                panic!("owned launch must never fall back to unmanaged run")
            }
        }
        let root = workspace();
        let prepared = prepared(root.path(), json!({"clientMode": "thin"}), false);
        let result = execute_with_resolved_runner(
            &prepared,
            &UnsupportedRunner,
            CancellationToken::new(),
            &tool(root.path()),
        );
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "provider_failed");
        assert!(result.diagnostics[0].get("detailCode").is_none());
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains(&root.path().display().to_string()));
    }

    #[test]
    fn cancellation_before_verified_handoff_discards_the_client() {
        let root = workspace();
        let prepared = prepared(root.path(), json!({"clientMode": "thin"}), false);
        let mut runner =
            SequenceRunner::new(vec![process(launched_envelope("thin", 4242, None), true)]);
        runner.cancel_after_output = true;
        let token = CancellationToken::new();
        let result =
            execute_with_resolved_runner(&prepared, &runner, token.clone(), &tool(root.path()));
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "cancelled");
        assert!(!token.protected_process_started());
        assert_eq!(
            runner.released.load(std::sync::atomic::Ordering::Acquire),
            0
        );
        assert_eq!(
            runner.discarded.load(std::sync::atomic::Ordering::Acquire),
            1
        );
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
                    "message": "platform 8.3.27.2074 is not installed; V8_RUNNER_CLIENT_OWNER=unica /private/config File=/private/ib"
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
        let public = serde_json::to_string(&result).unwrap();
        for private in [
            "V8_RUNNER_CLIENT_OWNER",
            "/private/config",
            "File=/private/ib",
        ] {
            assert!(!public.contains(private), "{public}");
        }
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
