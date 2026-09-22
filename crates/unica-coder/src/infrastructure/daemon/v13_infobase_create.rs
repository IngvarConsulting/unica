#![allow(clippy::result_large_err)]
//! `infobase.create` — создание пустой базы по соединению из `v8project.yaml`
//! силами `v8-runner init` (A-3 зонтика #871). Пара к `infobase.restore`: тот
//! наполняет базу из DT, этот заводит пустую.
//!
//! Аргументов нет: соединение задаёт проектный файл, и словарь его не
//! принимает — как у выгрузок. Превью зовёт `init --dry-run` и читает шаги
//! раннера по их статусам: базу можно создать только если шаг `infobase`
//! запланирован; существующая база — отказ до применения, а не тихий пропуск.
//! Шаг EDT-пространства обязан быть пропущен: Unica работает с выгрузкой
//! Designer и проект другого формата не заводит.
//!
//! Квитанцию после создания даёт сам раннер: повторное превью обязано
//! ответить, что базу создавать больше нечего. Состояние базы Unica иначе не
//! проверяет и называет его засвидетельствованным провайдером. Путь к базе,
//! платформе и командная строка наружу не идут.

use super::protocol::InvocationRequest;
use super::runner_011::Runner011ProcessRunner;
use super::v13_infobase_exports::{
    digest_optional_workspace_file, digest_required_workspace_file, missing_runner_rejection,
    resolve_bundled_runner, runner_rejection, CONFIG_NAME, LOCAL_CONFIG_NAME, RUNNER_OUTPUT_LIMIT,
};
use crate::application::invocation_store::ToolIdentity;
use crate::domain::cancellation::CancellationToken;
use crate::domain::invocation::{DomainResult, SafeIdentityHash};
use crate::domain::refusal::RefusalCode;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bundled_tools::BundledTool;
use crate::infrastructure::internal_adapters::{ProcessCommand, ProcessOutput, ProcessRunner};
use crate::infrastructure::redaction::redactor;
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) const OPERATION: &str = "infobase.create";
/// Имя команды в конверте раннера: словарь читается как слой и направление,
/// раннер называет свои команды по-своему.
const RUNNER_COMMAND: &str = "init";
const INFOBASE_STEP: &str = "infobase";
const EDT_STEP: &str = "edt_workspace";

#[derive(Debug, Clone)]
pub(super) struct PreparedInfobaseCreate {
    dry_run: bool,
    if_rev: Option<String>,
    context: WorkspaceContext,
}

pub(super) enum Preparation {
    NotApplicable,
    Rejected(Box<DomainResult>),
    Ready(Arc<PreparedInfobaseCreate>),
}

pub(super) fn prepare(request: &InvocationRequest) -> Preparation {
    if request.tool() != ToolIdentity::Run
        || request.arguments().get("op").and_then(Value::as_str) != Some(OPERATION)
    {
        return Preparation::NotApplicable;
    }
    match PreparedInfobaseCreate::parse(request) {
        Ok(prepared) => Preparation::Ready(Arc::new(prepared)),
        Err(result) => Preparation::Rejected(Box::new(result)),
    }
}

impl PreparedInfobaseCreate {
    fn parse(request: &InvocationRequest) -> Result<Self, DomainResult> {
        let arguments = request.arguments();
        let args = arguments
            .get("args")
            .and_then(Value::as_object)
            .ok_or_else(|| reject(RefusalCode::BadValue, "run args must be an object"))?;
        // Соединение задаёт проектный файл: аргумент с соединением означал бы
        // вторую правду о том, где база.
        if let Some(unknown) = args.keys().next() {
            return Err(reject(
                RefusalCode::BadValue,
                format!("infobase.create does not accept `{unknown}`: it takes no args, the connection comes from v8project.yaml"),
            ));
        }
        let dry_run = arguments
            .get("dryRun")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                reject(
                    RefusalCode::BadValue,
                    "infobase.create requires dryRun: true to preview or dryRun: false with ifRev to apply",
                )
            })?;
        let if_rev = match arguments.get("ifRev") {
            None => None,
            Some(Value::String(value)) if !value.trim().is_empty() => Some(value.clone()),
            Some(_) => {
                return Err(reject(
                    RefusalCode::BadValue,
                    "infobase.create ifRev must be non-empty text",
                ))
            }
        };
        if dry_run && if_rev.is_some() {
            return Err(reject(
                RefusalCode::BadValue,
                "infobase.create preview does not accept ifRev; apply the revision returned by this preview",
            ));
        }
        if !dry_run && if_rev.is_none() {
            return Err(reject(
                RefusalCode::BadValue,
                "infobase.create apply requires ifRev from a prior dryRun preview",
            ));
        }
        let context =
            discover_workspace(Some(PathBuf::from(request.workspace_hint()))).map_err(|error| {
                reject(
                    RefusalCode::ProviderUnavailable,
                    format!("workspace discovery failed: {error}"),
                )
            })?;
        Ok(Self {
            dry_run,
            if_rev,
            context,
        })
    }

    pub(super) fn workspace_identity_hash(&self) -> SafeIdentityHash {
        let mut hasher = Sha256::new();
        hasher.update(b"unica-v13-infobase-create-workspace-v1\0");
        hasher.update(self.context.workspace_root.as_os_str().as_encoded_bytes());
        SafeIdentityHash::from_sha256(hasher.finalize().into())
    }

    pub(super) fn execute(&self, cancellation: CancellationToken) -> DomainResult {
        execute_with_runner(self, &Runner011ProcessRunner, cancellation)
    }
}

/// Проектный файл с локальным дополнением: соединение живёт там, и план
/// действителен, пока они прежние.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StableInputs {
    config_sha256: String,
    local_config_sha256: Option<String>,
}

fn capture_inputs(prepared: &PreparedInfobaseCreate) -> Result<StableInputs, DomainResult> {
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
    Ok(StableInputs {
        config_sha256,
        local_config_sha256,
    })
}

fn execute_with_runner(
    prepared: &PreparedInfobaseCreate,
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

/// Статус шага раннера — закрытый словарь; всё вне его читается как негодный
/// результат, а не как «наверное, получилось».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepStatus {
    Planned,
    Skipped,
    Ok,
    Failed,
}

impl StepStatus {
    fn parse(value: &Value) -> Option<Self> {
        match value.as_str()? {
            "planned" => Some(Self::Planned),
            "skipped" => Some(Self::Skipped),
            "ok" => Some(Self::Ok),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

fn step_status(envelope: &Value, target: &str) -> Result<StepStatus, DomainResult> {
    let steps = envelope["data"]["steps"].as_array().ok_or_else(|| {
        reject(
            RefusalCode::InvalidResult,
            "v8-runner init answered without its step list",
        )
    })?;
    let step = steps
        .iter()
        .find(|step| step["target"] == target)
        .ok_or_else(|| {
            reject(
                RefusalCode::InvalidResult,
                format!("v8-runner init answered without the `{target}` step"),
            )
        })?;
    StepStatus::parse(&step["status"]).ok_or_else(|| {
        reject(
            RefusalCode::InvalidResult,
            format!("v8-runner init reported an unknown status for the `{target}` step"),
        )
    })
}

fn execute_with_resolved_runner(
    prepared: &PreparedInfobaseCreate,
    runner: &dyn ProcessRunner,
    cancellation: CancellationToken,
    tool: &BundledTool,
    runner_version: &str,
) -> DomainResult {
    if cancellation.is_cancelled() {
        return reject(
            RefusalCode::Cancelled,
            "infobase.create cancelled before preflight",
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
    if let Err(result) = validate_preview(&preview) {
        return result;
    }
    let after = match capture_inputs(prepared) {
        Ok(inputs) => inputs,
        Err(result) => return result,
    };
    if before != after {
        return reject(
            RefusalCode::ConcurrentChange,
            "infobase.create inputs changed during preview; run dryRun: true again",
        );
    }
    let revision = plan_revision(&before, runner_version);
    if prepared.dry_run {
        let mut result = DomainResult::success(
            "infobase.create planned creating the infobase without touching anything".to_string(),
        );
        result.data = Some(json!({
            "op": OPERATION,
            "dryRun": true,
            "plan": {
                "target": "infobase",
                "action": "create",
                "connectionFrom": CONFIG_NAME,
                "edtWorkspace": "skipped",
                // Что превью узнать не может, названо, а не умолчано.
                "targetStateKnownBeforeApply": false,
            },
            "providerDispatched": false,
            "requiresPlatform": true,
        }));
        result.rev = Some(revision.clone());
        result.next.push(json!({
            "tool": "unica.run",
            "args": {
                "op": OPERATION,
                "args": {},
                "dryRun": false,
                "ifRev": revision,
            },
            "reason": "create exactly this planned infobase"
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
                "infobase.create plan or environment changed after preview: expected rev {revision}, ifRev {}; run dryRun: true again",
                prepared.if_rev.as_deref().unwrap_or("absent")
            ),
        );
    }
    if cancellation.is_cancelled() {
        return reject(
            RefusalCode::Cancelled,
            "infobase.create cancelled before provider launch",
        );
    }
    let applied = match invoke_runner(prepared, tool, runner, &cancellation, false) {
        Ok(envelope) => envelope,
        Err(result) => return result,
    };
    if applied["data"]["provider_dispatched"] != true {
        return reject(
            RefusalCode::InvalidResult,
            "v8-runner reported success without dispatching the platform",
        );
    }
    match step_status(&applied, INFOBASE_STEP) {
        Ok(StepStatus::Ok) => {}
        // Между превью и применением базу завёл кто-то другой: наш план
        // исполнен не нами, и выдавать чужую базу за созданную нельзя.
        Ok(StepStatus::Skipped) => {
            return reject(
                RefusalCode::ConcurrentChange,
                "the infobase appeared between preview and apply; it was not created by this call",
            )
        }
        Ok(_) => {
            return reject(
                RefusalCode::InvalidResult,
                "v8-runner reported success without creating the infobase",
            )
        }
        Err(result) => return result,
    }
    // Квитанция у раннера, не со слов применения: повторное превью обязано
    // сказать, что создавать больше нечего.
    let receipt = match invoke_runner(prepared, tool, runner, &cancellation, true) {
        Ok(envelope) => envelope,
        Err(result) => return result,
    };
    match step_status(&receipt, INFOBASE_STEP) {
        Ok(StepStatus::Skipped) => {}
        Ok(_) => {
            return reject(
                RefusalCode::InvalidResult,
                "v8-runner reported the infobase created, but its preview still plans to create it",
            )
        }
        Err(result) => return result,
    }
    // **Улику о состоянии базы Unica не подделывает.** Файлы базы за
    // соединением Unica не читает; создание засвидетельствовано провайдером
    // и его же повторным превью, и источник признания назван прямо.
    let mut result = DomainResult::success(
        "infobase.create created the infobase; its state is attested by the provider and confirmed by a repeated preview".to_string(),
    );
    result.data = Some(json!({
        "op": OPERATION,
        "dryRun": false,
        "target": "infobase",
        "state": "created",
        "connectionFrom": CONFIG_NAME,
        "targetStateAttestedBy": "provider",
        "receipt": "repeated preview reports nothing left to create",
    }));
    // Изменилась база, а не файл рабочего пространства: путь сюда не кладётся.
    result.changed.push(json!({
        "infobase": true,
        "kind": "created",
    }));
    result.rev = Some(revision);
    result
}

fn validate_preview(envelope: &Value) -> Result<(), DomainResult> {
    let data = &envelope["data"];
    if data["provider_dispatched"] != false {
        return Err(reject(
            RefusalCode::InvalidResult,
            "v8-runner preview did not prove that nothing was created",
        ));
    }
    match step_status(envelope, INFOBASE_STEP)? {
        StepStatus::Planned => {}
        StepStatus::Skipped => {
            // Существующая база — не «нечего делать», а отказ: план создания
            // применять нельзя, а замена содержимого — дело `infobase.restore`.
            return Err(reject(
                RefusalCode::InvalidState,
                "the infobase at the configured connection already exists; infobase.create only creates an absent one, use infobase.restore with mode replace to overwrite its data",
            ));
        }
        StepStatus::Ok | StepStatus::Failed => {
            return Err(reject(
                RefusalCode::InvalidResult,
                "v8-runner preview reported an executed step instead of a plan",
            ))
        }
    }
    // Unica заводит только выгрузку Designer: проект, которому раннер
    // планирует EDT-пространство, здесь не обслуживается.
    if step_status(envelope, EDT_STEP)? != StepStatus::Skipped {
        return Err(reject(
            RefusalCode::InvalidState,
            "v8project.yaml declares a format that needs an EDT workspace; infobase.create serves Designer projects only",
        ));
    }
    Ok(())
}

fn invoke_runner(
    prepared: &PreparedInfobaseCreate,
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
    ];
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

fn plan_revision(inputs: &StableInputs, runner_version: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"unica-v13-infobase-create-plan-v1\0");
    hasher.update(
        serde_json::to_vec(&json!({
            "op": OPERATION,
            "inputs": {
                "config": inputs.config_sha256,
                "localConfig": inputs.local_config_sha256,
            },
            "runnerVersion": runner_version,
            "plan": {"target": INFOBASE_STEP, "action": "create"},
        }))
        .expect("plan revision data serializes"),
    );
    format!("unica-infobase-create-sha256-v1:{:x}", hasher.finalize())
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
    }

    impl SequenceRunner {
        fn new(outputs: Vec<ProcessOutput>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into_iter().rev().collect()),
                calls: Mutex::new(Vec::new()),
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

    fn workspace() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join(CONFIG_NAME),
            "format: DESIGNER\ninfobase:\n  connection: 'File=build/ib'\n",
        )
        .unwrap();
        root
    }

    fn prepared(root: &Path, dry_run: bool, if_rev: Option<String>) -> PreparedInfobaseCreate {
        PreparedInfobaseCreate {
            dry_run,
            if_rev,
            context: WorkspaceContext {
                cwd: root.to_path_buf(),
                workspace_root: root.to_path_buf(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 1,
            },
        }
    }

    fn tool(root: &Path) -> BundledTool {
        BundledTool {
            program: root.join("v8-runner"),
            warnings: Vec::new(),
            missing: None,
        }
    }

    /// Конверт `init` раннера 0.9.0, снятый с живой пробы: превью планирует
    /// шаг базы, существующая база даёт `skipped`, применение — `ok`.
    fn envelope(root: &Path, infobase: &str, edt: &str, dispatched: bool) -> Value {
        json!({
            "ok": true,
            "command": "init",
            "duration_ms": 0,
            "data": {
                "ok": true,
                "provider_dispatched": dispatched,
                "steps": [
                    {
                        "target": "infobase",
                        "action": "create",
                        "status": infobase,
                        "message": format!(
                            "would create a file infobase at '{}' via /opt/1cv8/8.3.27.2074/1cv8",
                            root.join("build/ib").display()
                        ),
                        "duration_ms": 0
                    },
                    {
                        "target": "edt_workspace",
                        "action": "import",
                        "status": edt,
                        "message": "EDT workspace initialization is not applicable for format=DESIGNER",
                        "duration_ms": 0
                    }
                ],
                "duration_ms": 0
            },
            "warnings": [],
            "steps": []
        })
    }

    fn run(
        root: &Path,
        prepared: &PreparedInfobaseCreate,
        runner: &SequenceRunner,
    ) -> DomainResult {
        execute_with_resolved_runner(
            prepared,
            runner,
            CancellationToken::new(),
            &tool(root),
            "0.9.0",
        )
    }

    #[test]
    fn a_missing_project_file_is_named_before_the_runner_is_called() {
        let root = tempfile::tempdir().unwrap();
        let runner = SequenceRunner::new(Vec::new());
        let result = run(root.path(), &prepared(root.path(), true, None), &runner);
        assert_eq!(result.diagnostics[0]["code"], "invalid_state");
        assert_eq!(result.next[0]["tool"], "unica.view");
        assert_eq!(runner.call_count(), 0);
    }

    #[test]
    fn preview_plans_the_infobase_without_creating_anything_or_naming_its_path() {
        let root = workspace();
        let runner = SequenceRunner::new(vec![process(
            envelope(root.path(), "planned", "skipped", false),
            true,
        )]);
        let result = run(root.path(), &prepared(root.path(), true, None), &runner);

        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["providerDispatched"], false);
        assert_eq!(data["plan"]["target"], "infobase");
        assert_eq!(data["plan"]["action"], "create");
        assert_eq!(data["plan"]["connectionFrom"], "v8project.yaml");
        assert_eq!(data["plan"]["targetStateKnownBeforeApply"], false);
        let revision = result.rev.clone().expect("preview returns a revision");
        assert!(revision.starts_with("unica-infobase-create-sha256-v1:"));
        assert_eq!(result.next[0]["args"]["ifRev"], revision);
        assert_eq!(result.next[0]["args"]["args"], json!({}));
        assert!(result.changed.is_empty());
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(
            !encoded.contains("build/ib"),
            "infobase path leaked: {encoded}"
        );
        assert!(!encoded.contains("1cv8"), "platform path leaked: {encoded}");
        assert!(!encoded.contains("--config"));
        assert!(runner
            .joined_args(0)
            .ends_with("--json-message init --dry-run"));
    }

    #[test]
    fn an_existing_infobase_is_refused_at_preview_and_points_to_import() {
        let root = workspace();
        let runner = SequenceRunner::new(vec![process(
            envelope(root.path(), "skipped", "skipped", false),
            true,
        )]);
        let result = run(root.path(), &prepared(root.path(), true, None), &runner);

        assert_eq!(result.diagnostics[0]["code"], "invalid_state", "{result:?}");
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("infobase.restore with mode replace"));
        assert!(result.rev.is_none());
    }

    #[test]
    fn a_project_that_needs_an_edt_workspace_is_refused() {
        let root = workspace();
        let runner = SequenceRunner::new(vec![process(
            envelope(root.path(), "planned", "planned", false),
            true,
        )]);
        let result = run(root.path(), &prepared(root.path(), true, None), &runner);

        assert_eq!(result.diagnostics[0]["code"], "invalid_state", "{result:?}");
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("Designer projects only"));
    }

    #[test]
    fn apply_creates_and_takes_its_receipt_from_a_repeated_preview() {
        let root = workspace();
        let preview_runner = SequenceRunner::new(vec![process(
            envelope(root.path(), "planned", "skipped", false),
            true,
        )]);
        let revision = run(
            root.path(),
            &prepared(root.path(), true, None),
            &preview_runner,
        )
        .rev
        .unwrap();
        let runner = SequenceRunner::new(vec![
            process(envelope(root.path(), "planned", "skipped", false), true),
            process(envelope(root.path(), "ok", "skipped", true), true),
            process(envelope(root.path(), "skipped", "skipped", false), true),
        ]);

        let result = run(
            root.path(),
            &prepared(root.path(), false, Some(revision.clone())),
            &runner,
        );

        assert!(result.ok, "{result:?}");
        assert_eq!(runner.call_count(), 3);
        assert!(runner.joined_args(0).ends_with("--dry-run"));
        assert!(runner.joined_args(1).ends_with("--json-message init"));
        assert!(runner.joined_args(2).ends_with("--dry-run"));
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["state"], "created");
        assert_eq!(data["targetStateAttestedBy"], "provider");
        assert_eq!(result.changed[0]["infobase"], true);
        assert_eq!(result.changed[0]["kind"], "created");
        assert!(result.changed[0].get("path").is_none());
        assert_eq!(result.rev, Some(revision));
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(
            !encoded.contains("build/ib"),
            "infobase path leaked: {encoded}"
        );
        assert!(!encoded.contains("1cv8"), "platform path leaked: {encoded}");
    }

    #[test]
    fn apply_refuses_a_stale_revision_and_an_infobase_created_elsewhere() {
        let root = workspace();
        let runner = SequenceRunner::new(vec![process(
            envelope(root.path(), "planned", "skipped", false),
            true,
        )]);
        let result = run(
            root.path(),
            &prepared(root.path(), false, Some("stale".to_string())),
            &runner,
        );
        assert_eq!(result.diagnostics[0]["code"], "stale_revision");
        assert_eq!(runner.call_count(), 1);

        let revision = run(
            root.path(),
            &prepared(root.path(), true, None),
            &SequenceRunner::new(vec![process(
                envelope(root.path(), "planned", "skipped", false),
                true,
            )]),
        )
        .rev
        .unwrap();
        let runner = SequenceRunner::new(vec![
            process(envelope(root.path(), "planned", "skipped", false), true),
            process(envelope(root.path(), "skipped", "skipped", true), true),
        ]);
        let result = run(
            root.path(),
            &prepared(root.path(), false, Some(revision)),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "concurrent_change",
            "{result:?}"
        );
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("not created by this call"));
    }

    #[test]
    fn apply_refuses_a_receipt_that_still_plans_to_create() {
        let root = workspace();
        let revision = run(
            root.path(),
            &prepared(root.path(), true, None),
            &SequenceRunner::new(vec![process(
                envelope(root.path(), "planned", "skipped", false),
                true,
            )]),
        )
        .rev
        .unwrap();
        let runner = SequenceRunner::new(vec![
            process(envelope(root.path(), "planned", "skipped", false), true),
            process(envelope(root.path(), "ok", "skipped", true), true),
            process(envelope(root.path(), "planned", "skipped", false), true),
        ]);
        let result = run(
            root.path(),
            &prepared(root.path(), false, Some(revision)),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("still plans to create"));
    }

    #[test]
    fn runner_refusals_keep_their_outcome() {
        let root = workspace();
        let failure = |code: &str| {
            json!({
                "ok": false,
                "command": "init",
                "duration_ms": 0,
                "data": {"ok": false, "provider_dispatched": true, "steps": [
                    {"target": "infobase", "action": "create", "status": "failed", "message": "refused", "duration_ms": 0}
                ]},
                "warnings": [],
                "steps": [],
                "error": {"code": code, "kind": "platform", "message": "refused"}
            })
        };
        for (code, expected, outcome) in [
            (
                "platform_failure",
                "provider_unavailable",
                Outcome::NeedsHuman,
            ),
            ("workspace_busy", "concurrent_change", Outcome::RetryAsIs),
        ] {
            let runner = SequenceRunner::new(vec![process(failure(code), false)]);
            let result = run(root.path(), &prepared(root.path(), true, None), &runner);
            assert_eq!(result.diagnostics[0]["code"], expected, "{code}");
            assert_eq!(map_runner_code(code).outcome(), outcome, "{code}");
        }
    }
}
