#![allow(clippy::result_large_err)]
//! `pull` — выгрузка базы в набор исходников рабочего пространства
//! силами `v8-runner pull` (A-6 зонтика #871). Пара к `push`: тот
//! вносит исходники в базу, этот выносит базу в исходники.
//!
//! Аргументы закрыты: `force:true` обязателен, выгрузка всегда полная; `sourceSet` — имя
//! объявленного набора, без него раннер берёт набор конфигурации;
//! `extension` — имя расширения, которое раннер требует для набора расширения
//! и сверяет с ним. Частичная выгрузка по объектам за словарём: у неё свой
//! словарь селекторов, и он не описан.
//!
//! Превью зовёт `pull --force --dry-run`: раннер называет набор, режим и целевой
//! каталог, ничего не записывая. Цель обязана лежать внутри рабочего
//! пространства и наружу уходит относительным путём. Применение повторяет
//! превью, сверяет забор ревизии, выгружает и пересчитывает файлы в цели
//! сама — квитанция не со слов провайдера. Путь к платформе и командная
//! строка наружу не идут.

mod publication_protocol;

use super::protocol::InvocationRequest;
use super::runner_015::{lock_project, normalize_full_export_config, project_full_export_config};
use super::v13_infobase_exports::{
    digest_optional_workspace_file, digest_required_workspace_file, missing_runner_rejection,
    note_another_copy, resolve_bundled_runner, runner_rejection, runner_start_rejection,
    valid_1c_identifier, RunnerAbsence, CONFIG_NAME, LOCAL_CONFIG_NAME, RUNNER_OUTPUT_LIMIT,
};
use super::v13_source_set_name::{source_set_name_guidance, valid_source_set_name};
use crate::application::invocation_store::ToolIdentity;
use crate::domain::cancellation::CancellationToken;
use crate::domain::invocation::{DomainResult, SafeIdentityHash};
use crate::domain::refusal::RefusalCode;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bundled_tools::BundledTool;
#[cfg(test)]
use crate::infrastructure::internal_adapters::ProcessOutput;
use crate::infrastructure::internal_adapters::{
    ProcessCommand, ProcessRunner, SystemProcessRunner,
};
use crate::infrastructure::platform::full_dump_publication::{
    FullDumpSelection, VerifiedFullDumpPublication,
};
use crate::infrastructure::redaction::redactor;
use crate::infrastructure::source_roots::normalize_path_identity;
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) const OPERATION: &str = "pull";
/// Имя команды раннера и в командной строке, и в конверте: с 0.12 раннер
/// называет её словом словаря.
const RUNNER_COMMAND: &str = "pull";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportMode {
    Full,
    Incremental,
}

impl ExportMode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "full" => Some(Self::Full),
            "incremental" => Some(Self::Incremental),
            _ => None,
        }
    }

    /// Как раннер пишет режим в конверте.
    const fn runner_mode(self) -> &'static str {
        match self {
            Self::Full => "FULL",
            Self::Incremental => "INCREMENTAL",
        }
    }
}

#[derive(Debug, Clone)]
struct ExportArguments {
    mode: ExportMode,
    source_set: Option<String>,
    extension: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct PreparedSourceExport {
    arguments: ExportArguments,
    dry_run: bool,
    context: WorkspaceContext,
}

pub(super) enum Preparation {
    NotApplicable,
    Rejected(Box<DomainResult>),
    Ready(Arc<PreparedSourceExport>),
}

pub(super) fn prepare(request: &InvocationRequest) -> Preparation {
    if request.tool() != ToolIdentity::Run
        || request.arguments().get("op").and_then(Value::as_str) != Some(OPERATION)
    {
        return Preparation::NotApplicable;
    }
    match PreparedSourceExport::parse(request) {
        Ok(prepared) => Preparation::Ready(Arc::new(prepared)),
        Err(result) => Preparation::Rejected(Box::new(result)),
    }
}

impl PreparedSourceExport {
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
                    "pull requires dryRun: true to preview or dryRun: false to execute",
                )
            })?;
        let context =
            discover_workspace(Some(PathBuf::from(request.workspace_hint()))).map_err(|error| {
                reject(
                    RefusalCode::ProviderUnavailable,
                    format!("workspace discovery failed: {error}"),
                )
            })?;
        if let Some(unknown) = args
            .keys()
            .find(|k| !["sourceSet", "extension", "force"].contains(&k.as_str()))
        {
            return Err(reject(
                RefusalCode::BadValue,
                format!("pull does not accept `{unknown}`; use its published argsSchema"),
            ));
        }
        if args.get("force") != Some(&Value::Bool(true)) {
            return Err(reject(RefusalCode::UnsupportedOperation, "pull requires force:true on the compatibility adapter; synchronization protection is unavailable"));
        }
        let mut public = args.clone();
        public.remove("force");
        public.insert("mode".into(), json!("full"));
        let arguments = parse_export_arguments(&public)?;
        Ok(Self {
            arguments,
            dry_run,
            context,
        })
    }

    pub(super) fn workspace_identity_hash(&self) -> SafeIdentityHash {
        let mut hasher = Sha256::new();
        hasher.update(b"unica-v13-source-export-workspace-v1\0");
        hasher.update(self.context.workspace_root.as_os_str().as_encoded_bytes());
        SafeIdentityHash::from_sha256(hasher.finalize().into())
    }

    pub(super) fn execute(&self, cancellation: CancellationToken) -> DomainResult {
        execute_with_runner(self, &SystemProcessRunner, cancellation)
    }
}

fn parse_export_arguments(args: &Map<String, Value>) -> Result<ExportArguments, DomainResult> {
    const ACCEPTED: [&str; 3] = ["mode", "sourceSet", "extension"];
    if let Some(unknown) = args.keys().find(|key| !ACCEPTED.contains(&key.as_str())) {
        return Err(reject(
            RefusalCode::BadValue,
            format!("pull does not accept `{unknown}`; the closed args are mode, sourceSet and extension"),
        ));
    }
    let mode = args
        .get("mode")
        .and_then(Value::as_str)
        .and_then(ExportMode::parse)
        .ok_or_else(|| {
            reject(
                RefusalCode::BadValue,
                "pull mode must be `full` or `incremental`; object-scoped partial export is not published",
            )
        })?;
    let source_set = match args.get("sourceSet") {
        None => None,
        Some(Value::String(value)) if valid_source_set_name(value) => Some(value.clone()),
        Some(_) => {
            return Err(reject(
                RefusalCode::BadValue,
                source_set_name_guidance(OPERATION),
            ))
        }
    };
    let extension = match args.get("extension") {
        None => None,
        Some(Value::String(value)) if valid_1c_identifier(value) => Some(value.clone()),
        Some(_) => {
            return Err(reject(
                RefusalCode::BadValue,
                "pull extension must be a non-empty 1C identifier",
            ))
        }
    };
    Ok(ExportArguments {
        mode,
        source_set,
        extension,
    })
}

/// Проектный файл с локальным дополнением и объявленный состав наборов: план
/// действителен, пока они прежние. Содержимое базы забором не держится — его
/// снимок и есть предмет выгрузки.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StableInputs {
    config_sha256: String,
    local_config_sha256: Option<String>,
    declared: Vec<String>,
}

fn capture_inputs(prepared: &PreparedSourceExport) -> Result<StableInputs, DomainResult> {
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
                    "pull sourceSet `{source_set}` is not declared in v8project.yaml; declared source sets: {}",
                    declared.join(", ")
                ),
            ));
        }
    }
    Ok(StableInputs {
        config_sha256,
        local_config_sha256,
        declared,
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
            format!("{CONFIG_NAME} declares no source-set; there is nowhere to export"),
        ));
    }
    Ok(names)
}

fn execute_with_runner(
    prepared: &PreparedSourceExport,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExportPlan {
    source_set: String,
    target: PathBuf,
    target_absolute: PathBuf,
    /// Цель уже была на месте при превью: применение назовёт её заменённой.
    existed: bool,
    /// Режим, который случился (у превью — режим плана), строчными буквами:
    /// `full`, `incremental`, `partial` или `unknown`.
    performed_mode: &'static str,
    /// Почему случившийся режим не тот, что просили, словами раннера 0.15.
    mode_reason: Option<&'static str>,
}

/// Случившийся режим выгрузки из конверта: закрытый словарь раннера 0.15.
fn performed_mode(value: &Value) -> Option<&'static str> {
    match value.as_str()? {
        "FULL" => Some("full"),
        "INCREMENTAL" => Some("incremental"),
        "PARTIAL" => Some("partial"),
        "UNKNOWN" => Some("unknown"),
        _ => None,
    }
}

/// Причина расхождения режимов: закрытый словарь раннера 0.15; поле
/// необязательно.
fn mode_reason(value: &Value) -> Result<Option<&'static str>, ()> {
    match value {
        Value::Null => Ok(None),
        Value::String(reason) => match reason.as_str() {
            "version_file" => Ok(Some("version_file")),
            "foreign_format" => Ok(Some("foreign_format")),
            "platform_forecast" => Ok(Some("platform_forecast")),
            "unknown" => Ok(Some("unknown")),
            _ => Err(()),
        },
        _ => Err(()),
    }
}

fn execute_with_resolved_runner(
    prepared: &PreparedSourceExport,
    runner: &dyn ProcessRunner,
    cancellation: CancellationToken,
    tool: &BundledTool,
    _runner_version: &str,
) -> DomainResult {
    publication_protocol::execute(prepared, runner, cancellation, tool)
}

fn validate_envelope(
    prepared: &PreparedSourceExport,
    inputs: &StableInputs,
    root_identity: &Path,
    envelope: &Value,
    applied: bool,
) -> Result<ExportPlan, DomainResult> {
    let data = &envelope["data"];
    let phase = if applied { "apply result" } else { "preview" };
    if data["provider_dispatched"] != applied {
        return Err(reject(
            RefusalCode::InvalidResult,
            if applied {
                "v8-runner reported success without dispatching the platform"
            } else {
                "v8-runner preview did not prove that nothing was written"
            },
        ));
    }
    // С 0.15 обязательный `requested_mode` — запрошенный режим, а `mode` —
    // случившийся: по плану раннера и прогнозу платформы он бывает полным вместо
    // инкрементного или неизвестным (`UNKNOWN`) (CTR.WIRE.PULL-DATA v6).
    if data["requested_mode"] != prepared.arguments.mode.runner_mode()
        || data["extension"].as_str() != prepared.arguments.extension.as_deref()
    {
        return Err(reject(
            RefusalCode::InvalidResult,
            format!("v8-runner {phase} answered for a different mode or extension"),
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
    let target_text = data["target_path"].as_str().ok_or_else(|| {
        reject(
            RefusalCode::InvalidResult,
            format!("v8-runner {phase} omitted the export target"),
        )
    })?;
    let target_path = Path::new(target_text);
    if !target_path.is_absolute() {
        return Err(reject(
            RefusalCode::InvalidResult,
            format!("v8-runner {phase} returned a relative export target"),
        ));
    }
    let target_absolute = normalize_path_identity(target_path).map_err(|error| {
        reject(
            RefusalCode::InvalidResult,
            format!("v8-runner {phase} returned an invalid export target: {error}"),
        )
    })?;
    let target = target_absolute
        .strip_prefix(root_identity)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .ok_or_else(|| {
            reject(
                RefusalCode::InvalidResult,
                format!("v8-runner {phase} placed the export target outside the workspace"),
            )
        })?
        .to_path_buf();
    let performed_mode = performed_mode(&data["mode"]).ok_or_else(|| {
        reject(
            RefusalCode::InvalidResult,
            format!("v8-runner {phase} reported an unknown export mode"),
        )
    })?;
    let mode_reason = mode_reason(&data["mode_reason"]).map_err(|()| {
        reject(
            RefusalCode::InvalidResult,
            format!("v8-runner {phase} reported an unknown reason for its export mode"),
        )
    })?;
    Ok(ExportPlan {
        source_set,
        target,
        existed: target_absolute.is_dir(),
        target_absolute,
        performed_mode,
        mode_reason,
    })
}

fn validate_runner_result(envelope: Value, status_success: bool) -> Result<Value, DomainResult> {
    if envelope["command"] != RUNNER_COMMAND {
        return Err(reject(
            RefusalCode::InvalidResult,
            "v8-runner returned a result for a different operation",
        ));
    }
    if !status_success || envelope["ok"] != true {
        // `provider_dispatched` отказа не читается: v8-runner 0.9.0 писал `true` и
        // до запуска платформы, 0.12 пишет `false` у отказа до исполнителя. Отказ
        // идёт по коду раннера.
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

fn public_arguments(prepared: &PreparedSourceExport) -> Value {
    let mut args = Map::new();
    args.insert("force".to_string(), Value::Bool(true));
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

/// Поставляемого раннера нет: маршрут один на все операции `run`, уточнение
/// `provider_absent` назначает общий помощник.
fn reject_absent_runner(absence: RunnerAbsence) -> DomainResult {
    missing_runner_rejection(Some(OPERATION.to_string()), absence)
}

#[cfg(test)]
mod tests {
    use super::super::v13_infobase_exports::map_runner_code;
    use super::*;
    use crate::domain::refusal::Outcome;
    use crate::infrastructure::internal_adapters::{
        ProcessLineReply, ProcessStreamFailure, ProcessStreamOutput,
    };
    use crate::infrastructure::platform::full_dump_publication::with_test_platform;
    use std::fs;
    use std::sync::Mutex;

    type ApplyEffect = Box<dyn Fn(&ProcessCommand, &Path)>;

    struct SequenceRunner {
        outputs: Mutex<Vec<ProcessOutput>>,
        calls: Mutex<Vec<ProcessCommand>>,
        /// Ожидаемая логическая цель. Фикстура пишет только в приватный stage
        /// из команды, даже если кадр намеренно сообщает чужую цель.
        materialize_on_apply: Option<PathBuf>,
        after_apply: Option<ApplyEffect>,
        after_publication: Option<ApplyEffect>,
        after_preview: Option<Box<dyn Fn()>>,
        fault: Option<PublicationFault>,
        replies: Mutex<Vec<Value>>,
        stage_writes: Mutex<Vec<PathBuf>>,
        terminals: Mutex<Vec<Value>>,
    }

    enum PublicationFault {
        ReadyField(&'static str, Value),
        SelectedExecutor(Value),
        TerminalField(&'static str, Value),
        MissingTerminal,
        MalformedTerminal,
        AckWriteFailure,
        CancelAfterPublication,
        UnconfirmedRunBeforeReady,
        UnwindBeforeReady,
    }

    impl SequenceRunner {
        fn new(outputs: Vec<ProcessOutput>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into_iter().rev().collect()),
                calls: Mutex::new(Vec::new()),
                materialize_on_apply: None,
                after_apply: None,
                after_publication: None,
                after_preview: None,
                fault: None,
                replies: Mutex::new(Vec::new()),
                stage_writes: Mutex::new(Vec::new()),
                terminals: Mutex::new(Vec::new()),
            }
        }

        fn exporting(outputs: Vec<ProcessOutput>, target: PathBuf) -> Self {
            let mut runner = Self::new(outputs);
            runner.materialize_on_apply = Some(target);
            runner
        }

        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }

        fn joined_args(&self, index: usize) -> String {
            self.calls.lock().unwrap()[index].args.join(" ")
        }

        fn decision(
            &self,
            response: &ProcessLineReply,
            nonce: &str,
            phase: &str,
        ) -> Option<String> {
            let bytes = match response {
                ProcessLineReply::Reply(bytes)
                | ProcessLineReply::ReplyAndProtect { reply: bytes, .. } => bytes,
                ProcessLineReply::Continue | ProcessLineReply::Stop => return None,
            };
            assert!(bytes.len() <= 4096 && bytes.last() == Some(&b'\n'));
            let reply: Value = serde_json::from_slice(bytes).unwrap();
            assert_eq!(reply["version"], 1);
            assert_eq!(reply["nonce"], nonce);
            assert_eq!(reply["phase"], phase);
            let decision = reply["decision"].as_str().unwrap().to_owned();
            self.replies.lock().unwrap().push(reply);
            Some(decision)
        }
    }

    impl ProcessRunner for SequenceRunner {
        fn run(&self, _: &ProcessCommand) -> Result<ProcessOutput, String> {
            panic!("source export must exchange live publication frames")
        }

        fn run_controlled_streaming_staged(
            &self,
            command: &ProcessCommand,
            _: usize,
            on_line: &mut dyn FnMut(usize, &[u8]) -> ProcessLineReply,
        ) -> Result<ProcessStreamOutput, ProcessStreamFailure> {
            self.calls.lock().unwrap().push(command.clone());
            let output = self.outputs.lock().unwrap().pop().expect("runner output");
            let mut envelope: Value = serde_json::from_str(&output.stdout).unwrap();
            let config = Path::new(&command.args[1]);
            let yaml: serde_yaml::Value =
                serde_yaml::from_slice(&fs::read(config).unwrap()).unwrap();
            let local: serde_yaml::Value =
                fs::read(config.parent().unwrap().join(LOCAL_CONFIG_NAME))
                    .ok()
                    .map(|bytes| serde_yaml::from_slice(&bytes).unwrap())
                    .unwrap_or(serde_yaml::Value::Null);
            let pull = command.args.iter().position(|arg| arg == "pull").unwrap();
            let set = command
                .args
                .get(pull + 1)
                .filter(|arg| !arg.starts_with("--"))
                .map(String::as_str)
                .unwrap_or("main");
            let declared = yaml["source-set"]
                .as_sequence()
                .unwrap()
                .iter()
                .find(|entry| entry["name"].as_str() == Some(set));
            let declared = declared.expect("guard selects a declared source set");
            let target = Path::new(declared["path"].as_str().unwrap());
            let original = normalize_path_identity(&command.cwd).unwrap().join(set);
            if envelope["data"]["source_set"].as_str() == Some(set)
                && envelope["data"]["target_path"].as_str().map(Path::new) == Some(&original)
            {
                envelope["data"]["target_path"] = json!(target);
            }
            let stage_index = command
                .args
                .iter()
                .position(|arg| arg == "--publication-stdio")
                .expect("canonical export requests the publication protocol");
            let stage = Path::new(&command.args[stage_index + 1]);
            assert!(stage.is_absolute() && target.is_absolute());
            assert_ne!(stage, target);
            assert!(stage.starts_with(&command.cwd));
            assert!(stage.parent().unwrap().is_dir());
            assert!(stage.ancestors().any(|path| path
                .file_name()
                .is_some_and(|name| { name.to_string_lossy().starts_with(".unica-dump-guard-") })));
            let preview = command.args.iter().any(|argument| argument == "--dry-run");
            let nonce = uuid::Uuid::new_v4().to_string();
            let executable = local["tools"]["platform"]["path"]
                .as_str()
                .or_else(|| yaml["tools"]["platform"]["path"].as_str())
                .map(PathBuf::from)
                .unwrap_or_else(|| command.cwd.join("platform/8.3.27.2074/1cv8"));
            let mut executor = json!({"kind":"local","executable":executable,"endpoint":null});
            if let Some(PublicationFault::SelectedExecutor(value)) = &self.fault {
                executor = value.clone();
            }
            let selected = json!({"version":1,"nonce":nonce,"phase":"selected",
                "provider":{"selected":"designer"},"executor":executor});
            let response = on_line(1, &serde_json::to_vec(&selected).unwrap());
            let authorized =
                self.decision(&response, &nonce, "selected").as_deref() == Some("continue");
            let mut stopped = response == ProcessLineReply::Stop;
            let mut published = false;
            if authorized && !preview {
                if let Some(expected_target) = &self.materialize_on_apply {
                    assert_eq!(normalize_path_identity(expected_target).unwrap(), target);
                    write_valid_dump(stage, declared["type"].as_str() == Some("EXTENSION"));
                    self.stage_writes.lock().unwrap().push(stage.to_path_buf());
                }
                if let Some(effect) = &self.after_apply {
                    effect(command, stage);
                }
                if matches!(&self.fault, Some(PublicationFault::UnwindBeforeReady)) {
                    panic!("injected controlled runner unwind before Ready");
                }
                if matches!(
                    &self.fault,
                    Some(PublicationFault::UnconfirmedRunBeforeReady)
                ) {
                    return Err(ProcessStreamFailure::Run(
                        "process_failed: termination was not confirmed".into(),
                    ));
                }
                let mut ready = json!({"version":1,"nonce":nonce,"phase":"ready",
                    "command":envelope["command"],
                    "requested_mode":envelope["data"]["requested_mode"],
                    "mode":envelope["data"]["mode"],
                    "mode_reason":envelope["data"]["mode_reason"],
                    "extension":envelope["data"]["extension"],
                    "provider_dispatched":envelope["data"]["provider_dispatched"],
                    "source_set":envelope["data"]["source_set"],
                    "target_path":envelope["data"]["target_path"],
                    "stage_path":stage,"snapshot":"prepared","snapshot_warning":null});
                if let Some(PublicationFault::ReadyField(field, value)) = &self.fault {
                    ready[*field] = value.clone();
                }
                let response = on_line(2, &serde_json::to_vec(&ready).unwrap());
                let decision = self.decision(&response, &nonce, "ready");
                published = matches!(response, ProcessLineReply::ReplyAndProtect { .. });
                assert_eq!(published, decision.as_deref() == Some("published"));
                stopped |= response == ProcessLineReply::Stop;
                if published {
                    assert!(
                        !stage.exists(),
                        "publication must consume its private stage"
                    );
                    assert!(target.join("Configuration.xml").is_file());
                    if let Some(effect) = &self.after_publication {
                        effect(command, stage);
                    }
                    match &self.fault {
                        Some(PublicationFault::AckWriteFailure) => {
                            return Err(ProcessStreamFailure::Run(
                                "process_failed: injected commit ACK write failure".into(),
                            ))
                        }
                        Some(PublicationFault::CancelAfterPublication) => {
                            command.cancellation.cancel()
                        }
                        _ => {}
                    }
                }
            }
            if !stopped
                && !(published && matches!(&self.fault, Some(PublicationFault::MissingTerminal)))
            {
                if published && matches!(&self.fault, Some(PublicationFault::MalformedTerminal)) {
                    stopped = on_line(3, b"not a JSON frame") == ProcessLineReply::Stop;
                } else {
                    let mut terminal = json!({"version":1,"nonce":nonce,"phase":"terminal",
                        "publication":if published {"published"} else {"not_published"},
                        "memory":if published {"saved"} else {"unchanged"},"result":envelope});
                    if published {
                        if let Some(PublicationFault::TerminalField(field, value)) = &self.fault {
                            terminal[*field] = value.clone();
                        }
                    }
                    self.terminals.lock().unwrap().push(terminal.clone());
                    stopped = on_line(3, &serde_json::to_vec(&terminal).unwrap())
                        == ProcessLineReply::Stop;
                }
            }
            if preview {
                if let Some(effect) = &self.after_preview {
                    effect();
                }
            }
            Ok(ProcessStreamOutput {
                status_success: output.status_success && !stopped,
                status: output.status,
                stderr: output.stderr,
                timed_out: output.timed_out,
                cancelled: output.cancelled,
                stopped_by_consumer: stopped,
                line_error: None,
            })
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
            "format: DESIGNER\ninfobase:\n  connection: 'File=build/ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: main\n  - name: Sales\n    type: EXTENSION\n    path: Sales\n",
        )
        .unwrap();
        root
    }

    fn write_valid_dump(target: &Path, extension: bool) {
        fs::create_dir_all(target.join("Catalogs")).unwrap();
        let properties = if extension {
            "<Properties><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose></Properties>"
        } else {
            "<Properties/>"
        };
        fs::write(target.join("Configuration.xml"), format!(
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Configuration>{properties}</Configuration></MetaDataObject>"
        )).unwrap();
        fs::write(target.join("Catalogs/Items.xml"),
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Catalog><Properties/></Catalog></MetaDataObject>"
        ).unwrap();
    }

    fn trusted_platform(root: &Path) -> PathBuf {
        let path = root.join("platform/8.3.27.2074/1cv8");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"controlled platform identity").unwrap();
        for name in ["ibcmd", "ibcmd.exe"] {
            fs::write(
                path.parent().unwrap().join(name),
                b"controlled probe identity",
            )
            .unwrap();
        }
        path
    }

    fn prepared(
        root: &Path,
        mode: ExportMode,
        source_set: Option<&str>,
        extension: Option<&str>,
        dry_run: bool,
    ) -> PreparedSourceExport {
        PreparedSourceExport {
            arguments: ExportArguments {
                mode,
                source_set: source_set.map(str::to_string),
                extension: extension.map(str::to_string),
            },
            dry_run,
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

    /// Конверт `pull` раннера 0.12.0 (форма та же, что у `dump` 0.9.0 с живой пробы):
    /// набор, режим прописными и абсолютный целевой каталог.
    fn envelope(
        root: &Path,
        set: &str,
        mode: &str,
        extension: Option<&str>,
        dispatched: bool,
    ) -> Value {
        let root = normalize_path_identity(root).unwrap();
        let mut data = json!({
            "ok": true,
            "provider_dispatched": dispatched,
            "source_set": set,
            "requested_mode": mode,
            "mode": mode,
            "target_path": root.join(set),
            "duration_ms": 2,
            "message": format!(
                "would dump {mode} into '{}' via /opt/1cv8/8.3.27.2074/1cv8; nothing written",
                root.join(set).display()
            ),
        });
        if let Some(extension) = extension {
            data["extension"] = json!(extension);
        }
        json!({
            "ok": true,
            "command": "pull",
            "duration_ms": 2,
            "data": data,
            "warnings": [],
            "steps": [],
        })
    }

    fn run(root: &Path, prepared: &PreparedSourceExport, runner: &SequenceRunner) -> DomainResult {
        with_test_platform(&trusted_platform(root), "8.3.27.2074", || {
            execute_with_resolved_runner(
                prepared,
                runner,
                CancellationToken::new(),
                &tool(root),
                "0.9.0",
            )
        })
    }

    fn captured_014(name: &str) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/v8_runner_014")
            .join(name);
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    /// Живые конверты раннера 0.14.0: `pull` копии в базу другой рабочей копии
    /// идёт с предупреждением раннера. Ответ называет его своим предупреждением
    /// и выходом к своей базе, без прозы раннера. Проверяется путь
    /// предупреждения, а не форма данных: конверт 0.14 без обязательного
    /// в 0.15 `requested_mode` отвергается, поэтому поле добавлено в тесте
    /// (`tests/fixtures/v8_runner_014/README.md`).
    #[test]
    fn captured_pull_into_the_infobase_of_another_copy_names_it() {
        let root = workspace();
        fs::create_dir_all(root.path().join("main")).unwrap();
        write_valid_dump(&root.path().join("main"), false);
        let target = normalize_path_identity(root.path()).unwrap().join("main");
        let legacy = captured_014("other-copy-pull-preview.json");
        let runner = SequenceRunner::new(vec![process(legacy, true)]);
        let result = run(
            root.path(),
            &prepared(root.path(), ExportMode::Full, Some("main"), None, true),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );

        let mut preview = captured_014("other-copy-pull-preview.json");
        let mut applied = captured_014("other-copy-pull-apply.json");
        for envelope in [&mut preview, &mut applied] {
            envelope["data"]["target_path"] = json!(target);
            envelope["data"]["requested_mode"] = envelope["data"]["mode"].clone();
        }
        let runner = SequenceRunner::new(vec![process(preview.clone(), true)]);
        let result = run(
            root.path(),
            &prepared(root.path(), ExportMode::Full, Some("main"), None, true),
            &runner,
        );
        assert!(result.ok, "{result:?}");
        super::super::v13_source_import::tests::assert_another_copy_is_named(&result);

        let runner = SequenceRunner::exporting(
            vec![process(preview, true), process(applied, true)],
            root.path().join("main"),
        );
        let result = run(
            root.path(),
            &prepared(root.path(), ExportMode::Full, Some("main"), None, false),
            &runner,
        );
        assert!(result.ok, "{result:?}");
        super::super::v13_source_import::tests::assert_another_copy_is_named(&result);
    }

    #[test]
    fn compatibility_cycle_accepts_explicit_force_and_rejects_silent_overwrite() {
        let root = workspace();
        let request = |args| {
            InvocationRequest::new(
                ToolIdentity::Run,
                json!({"op":"pull","args":args,"dryRun":true}),
                root.path().display().to_string(),
                7000,
            )
            .unwrap()
        };
        assert!(PreparedSourceExport::parse(&request(json!({"force":true}))).is_ok());
        assert!(PreparedSourceExport::parse(&request(json!({}))).is_err());
        assert!(PreparedSourceExport::parse(&request(json!({"force":false}))).is_err());
    }

    #[test]
    fn arguments_are_closed_and_each_refusal_names_the_fix() {
        for (args, expected) in [
            (json!({}), "mode must be `full` or `incremental`"),
            (
                json!({"mode": "partial"}),
                "mode must be `full` or `incremental`",
            ),
            (
                json!({"mode": "full", "objects": ["Catalog:Items"]}),
                "does not accept `objects`",
            ),
            (
                json!({"mode": "full", "sourceSet": "../main"}),
                "must be the name of a source set",
            ),
            (
                json!({"mode": "full", "extension": "1Sales"}),
                "1C identifier",
            ),
        ] {
            let result = parse_export_arguments(args.as_object().unwrap())
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
        let accepted = parse_export_arguments(
            json!({"mode": "incremental", "sourceSet": "ext-sales", "extension": "Sales"})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(accepted.mode, ExportMode::Incremental);
        assert_eq!(accepted.source_set.as_deref(), Some("ext-sales"));
        assert_eq!(accepted.extension.as_deref(), Some("Sales"));

        let root = workspace();
        let runner = SequenceRunner::new(Vec::new());
        let result = run(
            root.path(),
            &prepared(
                root.path(),
                ExportMode::Full,
                Some("ext-purchases"),
                None,
                true,
            ),
            &runner,
        );
        assert_eq!(result.diagnostics[0]["code"], "bad_value", "{result:?}");
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("declared source sets: main, Sales"));
        assert_eq!(runner.call_count(), 0);
    }

    #[test]
    fn preview_names_the_target_inside_the_workspace_without_writing() {
        let root = workspace();
        let runner = SequenceRunner::new(vec![process(
            envelope(root.path(), "main", "FULL", None, false),
            true,
        )]);
        let result = run(
            root.path(),
            &prepared(root.path(), ExportMode::Full, None, None, true),
            &runner,
        );

        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["providerDispatched"], false);
        assert_eq!(data["plan"]["sourceSet"], "main");
        assert_eq!(data["plan"]["mode"], "full");
        assert_eq!(data["plan"]["target"], "main");
        assert_eq!(data["plan"]["targetExists"], false);
        assert!(result.rev.is_none());
        assert_eq!(result.next[0]["args"]["dryRun"], false);
        assert!(result.next[0]["args"].get("ifRev").is_none());
        assert_eq!(result.next[0]["args"]["args"], json!({"force":true}));
        assert!(result.changed.is_empty());
        let encoded = serde_json::to_string(&result).unwrap();
        let root_text = normalize_path_identity(root.path())
            .unwrap()
            .display()
            .to_string();
        assert!(
            !encoded.contains(&root_text),
            "absolute path leaked: {encoded}"
        );
        assert!(!encoded.contains("1cv8"), "platform path leaked: {encoded}");
        assert!(runner
            .joined_args(0)
            .contains("pull main --force --publication-stdio "));
        assert!(runner.joined_args(0).ends_with("--dry-run"));
    }

    #[test]
    fn preview_of_an_extension_set_passes_the_extension_and_the_set() {
        let root = workspace();
        let runner = SequenceRunner::new(vec![process(
            envelope(root.path(), "Sales", "FULL", Some("Sales"), false),
            true,
        )]);
        let result = run(
            root.path(),
            &prepared(
                root.path(),
                ExportMode::Full,
                Some("Sales"),
                Some("Sales"),
                true,
            ),
            &runner,
        );
        assert!(result.ok, "{result:?}");
        assert_eq!(result.data.as_ref().unwrap()["plan"]["extension"], "Sales");
        assert!(runner
            .joined_args(0)
            .contains("pull Sales --force --publication-stdio "));
        assert!(runner
            .joined_args(0)
            .ends_with("--extension Sales --dry-run"));
    }

    #[test]
    fn preview_accepts_cyrillic_extension_source_set_from_public_run_request() {
        let root = workspace();
        let config = root.path().join(CONFIG_NAME);
        let yaml = fs::read_to_string(&config)
            .unwrap()
            .replace("Sales", "Доработки");
        fs::write(config, yaml).unwrap();
        let request = InvocationRequest::new(
            ToolIdentity::Run,
            json!({
                "op": "pull",
                "args": {"sourceSet": "Доработки", "extension": "Доработки", "force": true},
                "dryRun": true
            }),
            root.path().display().to_string(),
            7000,
        )
        .unwrap();
        let prepared = PreparedSourceExport::parse(&request).expect("Cyrillic source set");
        let runner = SequenceRunner::new(vec![process(
            envelope(root.path(), "Доработки", "FULL", Some("Доработки"), false),
            true,
        )]);
        let result = run(root.path(), &prepared, &runner);
        assert!(result.ok, "{result:?}");
        assert_eq!(
            result.data.as_ref().unwrap()["plan"]["sourceSet"],
            "Доработки"
        );
        assert_eq!(result.data.as_ref().unwrap()["plan"]["target"], "Доработки");
        assert!(runner
            .joined_args(0)
            .contains("pull Доработки --force --publication-stdio "));
        assert!(runner
            .joined_args(0)
            .ends_with("--extension Доработки --dry-run"));
        assert!(result.changed.is_empty());
        assert!(!root.path().join("Доработки").exists());
    }

    #[test]
    fn an_extension_set_requires_its_explicit_extension_before_runner_start() {
        let root = workspace();
        let runner = SequenceRunner::new(Vec::new());
        let result = run(
            root.path(),
            &prepared(root.path(), ExportMode::Full, Some("Sales"), None, true),
            &runner,
        );
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "bad_value");
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("extension"));
        assert!(result.changed.is_empty());
        assert_eq!(runner.call_count(), 0);
        assert!(!root.path().join("Sales").exists());
    }

    #[test]
    fn cancellation_while_waiting_for_project_ownership_is_typed_before_runner_start() {
        let root = workspace();
        let owner = lock_project(root.path(), &CancellationToken::new()).unwrap();
        let cancellation = CancellationToken::new();
        let cancel_waiter = cancellation.clone();
        let runner = SequenceRunner::new(Vec::new());
        let result = crate::infrastructure::platform::full_dump_publication::with_test_project_lock_contention(
            move || cancel_waiter.cancel(),
            || execute_with_resolved_runner(
                &prepared(root.path(), ExportMode::Full, None, None, true),
                &runner,
                cancellation,
                &tool(root.path()),
                "0.15.1",
            ),
        );
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "cancelled");
        assert!(result.changed.is_empty());
        assert_eq!(runner.call_count(), 0);
        assert!(!root.path().join("main").exists());
        drop(owner);
        lock_project(root.path(), &CancellationToken::new()).unwrap();
    }

    #[test]
    fn preview_refuses_a_target_outside_the_workspace_an_undeclared_set_or_a_write() {
        let root = workspace();
        let mut outside = envelope(root.path(), "main", "FULL", None, false);
        outside["data"]["target_path"] = json!(root.path().parent().unwrap().to_string_lossy());
        let runner = SequenceRunner::new(vec![process(outside, true)]);
        let result = run(
            root.path(),
            &prepared(root.path(), ExportMode::Full, None, None, true),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.changed.is_empty());
        assert!(!root.path().join("main").exists());

        let runner = SequenceRunner::new(vec![process(
            envelope(root.path(), "ext-purchases", "FULL", None, false),
            true,
        )]);
        let result = run(
            root.path(),
            &prepared(root.path(), ExportMode::Full, None, None, true),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );

        let runner = SequenceRunner::new(vec![process(
            envelope(root.path(), "main", "FULL", None, true),
            true,
        )]);
        let result = run(
            root.path(),
            &prepared(root.path(), ExportMode::Full, None, None, true),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("nothing was written"));
    }

    #[test]
    fn full_export_refuses_an_untrusted_platform_before_writing_sources() {
        let root = workspace();
        let config = root.path().join(CONFIG_NAME);
        let yaml = fs::read_to_string(&config).unwrap();
        fs::write(
            &config,
            format!("{yaml}tools:\n  platform:\n    path: missing-platform/1cv8\n"),
        )
        .unwrap();
        let runner = SequenceRunner::exporting(
            vec![
                process(envelope(root.path(), "main", "FULL", None, false), true),
                process(envelope(root.path(), "main", "FULL", None, true), true),
            ],
            root.path().join("main"),
        );
        let result = execute_with_resolved_runner(
            &prepared(root.path(), ExportMode::Full, None, None, false),
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
            "0.15.1",
        );
        assert!(!result.ok, "untrusted platform was dispatched: {result:?}");
        assert_eq!(
            runner.call_count(),
            1,
            "selection needs one runner handshake"
        );
        assert!(!runner
            .replies
            .lock()
            .unwrap()
            .iter()
            .any(|reply| reply["decision"] == "continue"));
        assert!(runner.stage_writes.lock().unwrap().is_empty());
        assert!(!root.path().join("main").exists());
    }

    #[test]
    fn apply_without_prior_preview_counts_the_exported_files_itself() {
        let root = workspace();
        let runner = SequenceRunner::exporting(
            vec![
                process(envelope(root.path(), "main", "FULL", None, false), true),
                process(envelope(root.path(), "main", "FULL", None, true), true),
            ],
            root.path().join("main"),
        );

        let result = run(
            root.path(),
            &prepared(root.path(), ExportMode::Full, None, None, false),
            &runner,
        );

        assert!(result.ok, "{result:?}");
        assert_eq!(runner.call_count(), 2);
        assert!(runner.joined_args(0).ends_with("--dry-run"));
        assert!(!runner.joined_args(1).contains("--dry-run"));
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["target"], "main");
        assert_eq!(data["files"], 2);
        assert_eq!(data["state"], "created");
        assert_eq!(result.changed[0]["path"], "main");
        assert_eq!(result.changed[0]["kind"], "created");
        assert!(result.rev.is_none());
        assert!(result.artifacts.is_empty());
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("1cv8"), "platform path leaked: {encoded}");
        assert!(!encoded.contains("--config"));
    }

    #[test]
    fn full_export_publishes_configuration_and_extension_from_a_private_stage() {
        for (set, extension) in [("main", None), ("Sales", Some("Sales"))] {
            let root = workspace();
            let target = root.path().join(set);
            write_valid_dump(&target, extension.is_some());
            fs::write(target.join("local-change.bsl"), "old local source").unwrap();
            fs::write(
                root.path().join(LOCAL_CONFIG_NAME),
                "infobase:\n  password: local-export-secret\n",
            )
            .unwrap();
            let original = fs::read(root.path().join(CONFIG_NAME)).unwrap();
            let original_local = fs::read(root.path().join(LOCAL_CONFIG_NAME)).unwrap();
            let expected_target = target.clone();
            let mut runner = SequenceRunner::exporting(
                vec![
                    process(envelope(root.path(), set, "FULL", extension, false), true),
                    process(envelope(root.path(), set, "FULL", extension, true), true),
                ],
                target.clone(),
            );
            runner.after_apply = Some(Box::new(move |command, staged| {
                assert_ne!(staged, expected_target);
                assert_eq!(
                    fs::read(expected_target.join("local-change.bsl")).unwrap(),
                    b"old local source"
                );
                let config = Path::new(&command.args[1]);
                assert!(!fs::read_to_string(config)
                    .unwrap()
                    .contains("local-export-secret"));
                assert!(
                    fs::read_to_string(config.parent().unwrap().join(LOCAL_CONFIG_NAME))
                        .unwrap()
                        .contains("local-export-secret")
                );
            }));
            let result = run(
                root.path(),
                &prepared(root.path(), ExportMode::Full, Some(set), extension, false),
                &runner,
            );
            assert!(result.ok, "{set}: {result:?}");
            assert_eq!(result.data.as_ref().unwrap()["files"], 2);
            assert_eq!(result.data.as_ref().unwrap()["state"], "replaced");
            assert_eq!(result.data.as_ref().unwrap()["extension"], json!(extension));
            assert!(!target.join("local-change.bsl").exists());
            assert!(target.join("Catalogs/Items.xml").is_file());
            assert_eq!(fs::read(root.path().join(CONFIG_NAME)).unwrap(), original);
            assert_eq!(
                fs::read(root.path().join(LOCAL_CONFIG_NAME)).unwrap(),
                original_local
            );
            assert!(!serde_json::to_string(&result)
                .unwrap()
                .contains("local-export-secret"));
            let calls = runner.calls.lock().unwrap();
            assert_eq!(calls[0].args[1], calls[1].args[1]);
            assert!(
                Path::new(&calls[1].args[1]).is_file(),
                "the stable working-copy owner remains available after finish"
            );
            assert!(calls
                .iter()
                .all(|call| call.args.iter().any(|arg| arg == "--publication-stdio")));
            assert!(fs::read_dir(root.path()).unwrap().all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".unica-dump-guard-")));
        }
    }

    #[test]
    fn full_export_rejects_invalid_results_before_replacing_sources() {
        for defect in [
            "command",
            "requested_mode",
            "mode",
            "mode_reason",
            "extension",
            "source_set",
            "target_path",
            "provider_dispatched",
            "xml",
            "cancel",
            "platform",
            "config",
        ] {
            let root = workspace();
            let target = root.path().join("main");
            write_valid_dump(&target, false);
            fs::write(target.join("keep.bsl"), "unchanged").unwrap();
            let original = crate::test_support::tree_snapshot(&target);
            let mut applied = envelope(root.path(), "main", "FULL", None, true);
            match defect {
                "command" => applied["command"] = json!("push"),
                "requested_mode" => applied["data"]["requested_mode"] = json!("INCREMENTAL"),
                "mode" => applied["data"]["mode"] = json!("UNVERIFIED"),
                "mode_reason" => applied["data"]["mode_reason"] = json!("weather"),
                "extension" => applied["data"]["extension"] = json!("Other"),
                "source_set" => applied["data"]["source_set"] = json!("ext-sales"),
                "target_path" => {
                    applied["data"]["target_path"] = json!(root.path().join("unexpected"))
                }
                "provider_dispatched" => applied["data"]["provider_dispatched"] = json!(false),
                _ => {}
            }
            let mut runner = SequenceRunner::exporting(
                vec![
                    process(envelope(root.path(), "main", "FULL", None, false), true),
                    process(applied, true),
                ],
                target.clone(),
            );
            let workspace_root = root.path().to_path_buf();
            let retained_stage = Arc::new(Mutex::new(None));
            let observed_stage = Arc::clone(&retained_stage);
            runner.after_apply = Some(Box::new(move |command, staged| {
                *observed_stage.lock().unwrap() =
                    Some(staged.parent().unwrap().parent().unwrap().to_path_buf());
                match defect {
                    "xml" => fs::write(staged.join("Configuration.xml"), "<invalid/>").unwrap(),
                    "cancel" => command.cancellation.cancel(),
                    "platform" => {
                        fs::write(
                            workspace_root.join("platform/8.3.27.2074/1cv8"),
                            "replaced platform",
                        )
                        .unwrap();
                    }
                    "config" => {
                        fs::write(workspace_root.join(CONFIG_NAME), "replaced config").unwrap();
                    }
                    _ => {}
                }
            }));
            let result = run(
                root.path(),
                &prepared(root.path(), ExportMode::Full, None, None, false),
                &runner,
            );
            assert!(!result.ok, "{defect}");
            let private_root = retained_stage.lock().unwrap().take().unwrap();
            assert_eq!(result.artifacts.len(), 1, "{defect}");
            assert_eq!(result.artifacts[0]["kind"], "recovery", "{defect}");
            assert_eq!(
                result.artifacts[0].as_object().unwrap().len(),
                2,
                "{defect}"
            );
            assert_retained_artifact_names_directory(&result, root.path(), &private_root);
            let encoded = serde_json::to_string(&result).unwrap();
            assert!(!encoded.contains("1cv8"), "{defect}");
            assert!(!encoded.contains("--config"), "{defect}");
            // Only the checked recovery artifact may name private staging.
            let mut without_recovery = serde_json::to_value(&result).unwrap();
            without_recovery
                .as_object_mut()
                .unwrap()
                .remove("artifacts");
            assert!(
                !serde_json::to_string(&without_recovery)
                    .unwrap()
                    .contains(".unica-dump-guard-"),
                "{defect}"
            );
            assert_eq!(
                crate::test_support::tree_snapshot(&target),
                original,
                "{defect}"
            );
            assert!(result.changed.is_empty(), "{defect}");
        }
    }

    #[test]
    fn selected_executor_must_be_attested_locally_before_continue_or_stage_writes() {
        for executor in [
            json!({"kind":"remote","endpoint":"https://example.invalid","executable":null}),
            json!({"kind":"attached","endpoint":null,"executable":null}),
            json!({"kind":"local","endpoint":null,"executable":null}),
        ] {
            let root = workspace();
            let target = root.path().join("main");
            write_valid_dump(&target, false);
            let original = crate::test_support::tree_snapshot(&target);
            let mut runner = SequenceRunner::exporting(
                vec![process(
                    envelope(root.path(), "main", "FULL", None, false),
                    true,
                )],
                target.clone(),
            );
            runner.fault = Some(PublicationFault::SelectedExecutor(executor));
            let result = run(
                root.path(),
                &prepared(root.path(), ExportMode::Full, None, None, false),
                &runner,
            );
            assert!(!result.ok, "{result:?}");
            assert_eq!(runner.call_count(), 1);
            assert!(runner.stage_writes.lock().unwrap().is_empty());
            assert!(!runner
                .replies
                .lock()
                .unwrap()
                .iter()
                .any(|reply| reply["decision"] == "continue"));
            assert_eq!(crate::test_support::tree_snapshot(&target), original);
            assert!(result.changed.is_empty());
        }
    }

    #[test]
    fn ready_frame_cannot_publish_another_target_stage_or_unprepared_snapshot() {
        for field in [
            "source_set",
            "target_path",
            "stage_path",
            "snapshot",
            "snapshot_warning",
            "nonce",
            "version",
        ] {
            let root = workspace();
            let outside = tempfile::tempdir().unwrap();
            fs::write(
                outside.path().join("foreign.bsl"),
                "preserve foreign target",
            )
            .unwrap();
            let foreign = crate::test_support::tree_snapshot(outside.path());
            let target = root.path().join("main");
            write_valid_dump(&target, false);
            fs::write(target.join("keep.bsl"), "preserve original sources").unwrap();
            let original = crate::test_support::tree_snapshot(&target);
            let mut runner = SequenceRunner::exporting(
                vec![
                    process(envelope(root.path(), "main", "FULL", None, false), true),
                    process(envelope(root.path(), "main", "FULL", None, true), true),
                ],
                target.clone(),
            );
            let value = match field {
                "source_set" => json!("Sales"),
                "target_path" | "stage_path" => json!(outside.path()),
                "snapshot" => json!("unavailable"),
                "snapshot_warning" => json!("memory snapshot failed"),
                "nonce" => json!(uuid::Uuid::new_v4().to_string()),
                "version" => json!(2),
                _ => unreachable!(),
            };
            runner.fault = Some(PublicationFault::ReadyField(field, value));
            let result = run(
                root.path(),
                &prepared(root.path(), ExportMode::Full, None, None, false),
                &runner,
            );
            assert!(!result.ok, "{field}: {result:?}");
            assert!(result.changed.is_empty(), "{field}: {result:?}");
            assert_eq!(
                crate::test_support::tree_snapshot(&target),
                original,
                "{field}"
            );
            assert_eq!(
                crate::test_support::tree_snapshot(outside.path()),
                foreign,
                "{field}"
            );
            assert_eq!(runner.stage_writes.lock().unwrap().len(), 1);
            assert!(!runner
                .replies
                .lock()
                .unwrap()
                .iter()
                .any(|reply| reply["decision"] == "published"));
        }
    }

    #[test]
    fn internal_preview_rechecks_both_original_config_layers_before_apply() {
        for local in [false, true] {
            let root = workspace();
            let target = root.path().join("main");
            write_valid_dump(&target, false);
            let original = crate::test_support::tree_snapshot(&target);
            let mut runner = SequenceRunner::exporting(
                vec![process(
                    envelope(root.path(), "main", "FULL", None, false),
                    true,
                )],
                target.clone(),
            );
            let config = root.path().join(if local {
                LOCAL_CONFIG_NAME
            } else {
                CONFIG_NAME
            });
            runner.after_preview = Some(Box::new(move || {
                if local {
                    fs::write(&config, "infobase:\n  password: changed-after-preview\n").unwrap();
                } else {
                    let mut source = fs::read_to_string(&config).unwrap();
                    source.push_str("\n# changed after the internal preview\n");
                    fs::write(&config, source).unwrap();
                }
            }));
            let result = run(
                root.path(),
                &prepared(root.path(), ExportMode::Full, None, None, false),
                &runner,
            );
            assert!(!result.ok, "local={local}: {result:?}");
            assert_eq!(result.diagnostics[0]["code"], "concurrent_change");
            assert_eq!(
                runner.call_count(),
                1,
                "apply must not start after changed inputs"
            );
            assert!(runner.stage_writes.lock().unwrap().is_empty());
            assert_eq!(crate::test_support::tree_snapshot(&target), original);
            assert!(result.changed.is_empty());
        }
    }

    #[test]
    fn an_unconfirmed_run_retains_the_named_stage_of_a_live_writer() {
        assert_unconfirmed_finish_retains_live_writer(false);
    }

    #[test]
    fn an_unwind_retains_the_named_stage_of_a_live_writer() {
        assert_unconfirmed_finish_retains_live_writer(true);
    }

    fn assert_unconfirmed_finish_retains_live_writer(unwind: bool) {
        use std::io::Write;
        use std::sync::{mpsc, Arc};
        use std::time::Duration;

        struct HeldWriter {
            path: PathBuf,
            release: Option<mpsc::Sender<()>>,
            finished: mpsc::Receiver<Result<(), String>>,
            worker: Option<std::thread::JoinHandle<()>>,
        }

        impl Drop for HeldWriter {
            fn drop(&mut self) {
                if let Some(release) = self.release.take() {
                    let _ = release.send(());
                }
                if let Some(worker) = self.worker.take() {
                    let _ = worker.join();
                }
            }
        }

        let root = workspace();
        let target = root.path().join("main");
        write_valid_dump(&target, false);
        fs::write(target.join("local.bsl"), b"original public source").unwrap();
        let before = crate::test_support::tree_snapshot(&target);
        let writer = Arc::new(Mutex::new(None));
        let writer_for_runner = writer.clone();
        let mut runner = SequenceRunner::exporting(
            vec![
                process(envelope(root.path(), "main", "FULL", None, false), true),
                process(envelope(root.path(), "main", "FULL", None, true), true),
            ],
            target.clone(),
        );
        runner.fault = Some(if unwind {
            PublicationFault::UnwindBeforeReady
        } else {
            PublicationFault::UnconfirmedRunBeforeReady
        });
        runner.after_apply = Some(Box::new(move |_, stage| {
            let path = stage.join("active-writer.bsl");
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .append(true)
                .open(&path)
                .unwrap();
            file.write_all(b"before\n").unwrap();
            file.sync_all().unwrap();
            let (release, wait) = mpsc::channel();
            let (finished, completion) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                if wait.recv().is_ok() {
                    let result = file
                        .write_all(b"after\n")
                        .and_then(|()| file.sync_all())
                        .map_err(|error| error.to_string());
                    let _ = finished.send(result);
                }
            });
            *writer_for_runner.lock().unwrap() = Some(HeldWriter {
                path,
                release: Some(release),
                finished: completion,
                worker: Some(worker),
            });
        }));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(
                root.path(),
                &prepared(root.path(), ExportMode::Full, None, None, false),
                &runner,
            )
        }));
        let result = if unwind {
            assert!(
                result.is_err(),
                "the fixture must exercise guard Drop through unwind"
            );
            None
        } else {
            let result = result.expect("the error-result fixture must not unwind");
            assert!(!result.ok);
            assert!(result.changed.is_empty());
            Some(result)
        };
        let mut writer = writer.lock().unwrap().take().unwrap();
        assert_eq!(crate::test_support::tree_snapshot(&target), before);
        assert!(
            !writer.worker.as_ref().unwrap().is_finished(),
            "the fixture must still own a live stage writer"
        );
        assert_eq!(
            fs::read(&writer.path).unwrap_or_else(|error| {
                panic!("cleanup removed the live writer's named data: {error}")
            }),
            b"before\n"
        );
        writer.release.take().unwrap().send(()).unwrap();
        writer
            .finished
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        assert_eq!(fs::read(&writer.path).unwrap(), b"before\nafter\n");
        assert_eq!(crate::test_support::tree_snapshot(&target), before);
        if let Some(result) = result {
            assert_ne!(
                result.data.as_ref().and_then(|data| data["state"].as_str()),
                Some("unknown"),
                "private retention before Ready does not make the unchanged public target unknown"
            );
            let private_root = writer.path.ancestors().nth(3).unwrap();
            assert_retained_artifact_names_directory(&result, root.path(), private_root);
        }
    }

    fn assert_retained_artifact_names_directory(
        result: &DomainResult,
        workspace: &Path,
        expected: &Path,
    ) {
        let expected = fs::canonicalize(expected).unwrap();
        assert!(
            result.artifacts.iter().any(|artifact| {
                artifact["path"].as_str().is_some_and(|path| {
                    fs::canonicalize(workspace.join(path)).is_ok_and(|path| path == expected)
                })
            }),
            "the result must name its retained recovery directory"
        );
    }

    #[test]
    fn full_export_reports_unconfirmed_private_location_without_changing_publication_facts() {
        use crate::infrastructure::platform::filesystem::{
            file_identity, open_directory_nofollow, FileIdentity,
        };
        use std::collections::BTreeMap;

        fn capture(root: &Path) -> (BTreeMap<PathBuf, Vec<u8>>, BTreeMap<PathBuf, FileIdentity>) {
            fn visit(
                root: &Path,
                current: &Path,
                identities: &mut BTreeMap<PathBuf, FileIdentity>,
            ) {
                let handle = if current.is_dir() {
                    open_directory_nofollow(current).unwrap()
                } else {
                    fs::File::open(current).unwrap()
                };
                identities.insert(
                    current.strip_prefix(root).unwrap().to_path_buf(),
                    file_identity(&handle).unwrap(),
                );
                if current.is_dir() {
                    for entry in fs::read_dir(current).unwrap() {
                        visit(root, &entry.unwrap().path(), identities);
                    }
                }
            }
            let mut identities = BTreeMap::new();
            visit(root, root, &mut identities);
            (crate::test_support::tree_snapshot(root), identities)
        }

        let mut invalid_reports = Vec::new();
        for after_publication in [false, true] {
            let phase = if after_publication {
                "after verified publication"
            } else {
                "before Ready"
            };
            let root = workspace();
            let target = root.path().join("main");
            write_valid_dump(&target, false);
            fs::write(target.join("replaced.bsl"), b"old source").unwrap();
            fs::write(
                root.path().join(LOCAL_CONFIG_NAME),
                "infobase:\n  password: private-location-secret\n",
            )
            .unwrap();
            let public_before = capture(&target);
            let config_before = fs::read(root.path().join(CONFIG_NAME)).unwrap();
            let local_before = fs::read(root.path().join(LOCAL_CONFIG_NAME)).unwrap();
            let observed = std::sync::Arc::new(Mutex::new(None));
            let effect_observed = std::sync::Arc::clone(&observed);
            let effect_target = target.clone();
            let effect: ApplyEffect = Box::new(move |_, stage| {
                let private = stage.parent().unwrap().parent().unwrap().to_path_buf();
                let displaced = private.with_file_name("displaced-owned-private");
                let owned_before = capture(&private);
                let public_at_effect = capture(&effect_target);
                fs::rename(&private, &displaced).unwrap();
                fs::create_dir(&private).unwrap();
                fs::create_dir(private.join("foreign")).unwrap();
                fs::write(
                    private.join("foreign/sentinel.txt"),
                    b"foreign private replacement sentinel",
                )
                .unwrap();
                let foreign_before = capture(&private);
                *effect_observed.lock().unwrap() = Some((
                    private,
                    displaced,
                    owned_before,
                    foreign_before,
                    public_at_effect,
                ));
            });
            let mut runner = SequenceRunner::exporting(
                vec![
                    process(envelope(root.path(), "main", "FULL", None, false), true),
                    process(envelope(root.path(), "main", "FULL", None, true), true),
                ],
                target.clone(),
            );
            if after_publication {
                runner.after_publication = Some(effect);
            } else {
                runner.after_apply = Some(effect);
            }
            let result = run(
                root.path(),
                &prepared(root.path(), ExportMode::Full, None, None, false),
                &runner,
            );
            let (private, displaced, owned_before, foreign_before, public_at_effect) = observed
                .lock()
                .unwrap()
                .take()
                .expect("the real private-directory replacement must execute");
            assert_eq!(
                capture(&displaced),
                owned_before,
                "{phase}: finalization must preserve all owned private identities and bytes"
            );
            assert_eq!(
                capture(&private),
                foreign_before,
                "{phase}: finalization must preserve all foreign identities and bytes"
            );
            assert_ne!(
                owned_before.1[Path::new("")],
                foreign_before.1[Path::new("")]
            );
            assert_eq!(
                fs::read(private.join("foreign/sentinel.txt")).unwrap(),
                b"foreign private replacement sentinel"
            );
            assert_eq!(capture(&target), public_at_effect, "{phase}");
            assert_eq!(
                fs::read(root.path().join(CONFIG_NAME)).unwrap(),
                config_before
            );
            assert_eq!(
                fs::read(root.path().join(LOCAL_CONFIG_NAME)).unwrap(),
                local_before
            );
            if after_publication {
                assert!(result.ok, "{phase}");
                assert_eq!(
                    result.changed,
                    vec![json!({"path":"main","kind":"replaced"})],
                    "{phase}"
                );
                assert_eq!(result.data.as_ref().unwrap()["state"], "replaced");
                assert!(!target.join("replaced.bsl").exists());
                assert!(target.join("Catalogs/Items.xml").is_file());
                assert!(owned_before.0.values().any(|bytes| bytes == b"old source"));
            } else {
                assert!(!result.ok, "{phase}");
                assert_eq!(
                    result.diagnostics[0]["code"], "concurrent_change",
                    "{phase}"
                );
                assert!(result.changed.is_empty(), "{phase}");
                assert_eq!(capture(&target), public_before, "{phase}");
                assert_eq!(
                    fs::read(target.join("replaced.bsl")).unwrap(),
                    b"old source"
                );
                assert_ne!(
                    result.data.as_ref().and_then(|data| data["state"].as_str()),
                    Some("unknown"),
                    "private location uncertainty must not change public publication facts"
                );
            }
            let encoded = serde_json::to_string(&result).unwrap();
            assert!(!encoded.contains("private-location-secret"), "{phase}");
            assert!(!encoded.contains("1cv8"), "{phase}");
            assert!(!encoded.contains("--config"), "{phase}");
            assert!(!encoded.contains(CONFIG_NAME), "{phase}");
            let annotation = format!("original name: {}", private.display());
            let mut without_location_note = serde_json::to_value(&result).unwrap();
            let warnings = without_location_note["warnings"].as_array_mut().unwrap();
            let before_count = warnings.len();
            warnings.retain(|warning| {
                !(warning["code"] == "export_cleanup"
                    && warning["message"].as_str().is_some_and(|message| {
                        message.contains("current location is unconfirmed")
                            && message.contains(&annotation)
                    }))
            });
            let exact_location_note = warnings.len() < before_count;
            let no_other_private_path = !serde_json::to_string(&without_location_note)
                .unwrap()
                .contains(".unica-dump-guard-");
            if !result.artifacts.is_empty() || !exact_location_note || !no_other_private_path {
                invalid_reports.push(phase);
            }
        }
        assert!(
            invalid_reports.is_empty(),
            "false or missing private-location reports: {invalid_reports:?}"
        );
    }

    #[test]
    fn a_verified_publication_with_no_terminal_retains_its_recovery() {
        let root = workspace();
        let target = root.path().join("main");
        write_valid_dump(&target, false);
        fs::write(target.join("replaced.bsl"), b"old source").unwrap();
        let private_root = std::sync::Arc::new(Mutex::new(None));
        let private_root_for_runner = private_root.clone();
        let mut runner = SequenceRunner::exporting(
            vec![
                process(envelope(root.path(), "main", "FULL", None, false), true),
                process(envelope(root.path(), "main", "FULL", None, true), true),
            ],
            target.clone(),
        );
        runner.fault = Some(PublicationFault::MissingTerminal);
        runner.after_apply = Some(Box::new(move |_, stage| {
            *private_root_for_runner.lock().unwrap() =
                Some(stage.parent().unwrap().parent().unwrap().to_path_buf());
        }));
        let result = run(
            root.path(),
            &prepared(root.path(), ExportMode::Full, None, None, false),
            &runner,
        );
        assert!(result.ok);
        assert_eq!(
            result.changed,
            vec![json!({"path":"main","kind":"replaced"})]
        );
        assert_eq!(result.data.as_ref().unwrap()["state"], "replaced");
        assert!(!target.join("replaced.bsl").exists());
        assert!(target.join("Catalogs/Items.xml").is_file());
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning["code"] == "sync_memory_unconfirmed"));
        let private_root = private_root.lock().unwrap().take().unwrap();
        assert!(private_root.is_dir(), "unconfirmed finish removed recovery");
        assert!(
            crate::test_support::tree_snapshot(&private_root)
                .values()
                .any(|bytes| bytes == b"old source"),
            "original source bytes must remain available for recovery"
        );
        assert_retained_artifact_names_directory(&result, root.path(), &private_root);
    }

    #[test]
    fn published_sources_survive_missing_terminal_ack_failure_and_late_cancel() {
        for (name, fault, unconfirmed) in [
            ("missing terminal", PublicationFault::MissingTerminal, true),
            ("ACK write failed", PublicationFault::AckWriteFailure, true),
            (
                "late cancellation",
                PublicationFault::CancelAfterPublication,
                false,
            ),
            (
                "memory not saved",
                PublicationFault::TerminalField("memory", json!("not_saved")),
                true,
            ),
        ] {
            let root = workspace();
            let target = root.path().join("main");
            write_valid_dump(&target, false);
            fs::write(target.join("replaced.bsl"), "old source").unwrap();
            let mut runner = SequenceRunner::exporting(
                vec![
                    process(envelope(root.path(), "main", "FULL", None, false), true),
                    process(envelope(root.path(), "main", "FULL", None, true), true),
                ],
                target.clone(),
            );
            runner.fault = Some(fault);
            let result = run(
                root.path(),
                &prepared(root.path(), ExportMode::Full, None, None, false),
                &runner,
            );
            assert!(result.ok, "{name}: {result:?}");
            assert_eq!(
                result.changed,
                vec![json!({"path":"main","kind":"replaced"})],
                "{name}"
            );
            assert_eq!(result.data.as_ref().unwrap()["files"], 2, "{name}");
            assert!(!target.join("replaced.bsl").exists(), "{name}");
            assert!(target.join("Catalogs/Items.xml").is_file(), "{name}");
            assert_eq!(
                result
                    .warnings
                    .iter()
                    .any(|warning| warning["code"] == "sync_memory_unconfirmed"),
                unconfirmed,
                "{name}: {result:?}"
            );
        }
    }

    #[test]
    fn invalid_terminal_after_publication_keeps_changed_and_reports_invalid_result() {
        for defect in [
            "malformed",
            "nonce",
            "publication",
            "command",
            "requested_mode",
            "mode",
            "mode_reason",
            "extension",
            "provider_dispatched",
        ] {
            let root = workspace();
            let target = root.path().join("main");
            write_valid_dump(&target, false);
            fs::write(target.join("replaced.bsl"), "old source").unwrap();
            let mut runner = SequenceRunner::exporting(
                vec![
                    process(envelope(root.path(), "main", "FULL", None, false), true),
                    process(envelope(root.path(), "main", "FULL", None, true), true),
                ],
                target.clone(),
            );
            let mut forged = envelope(root.path(), "main", "FULL", None, true);
            runner.fault = Some(match defect {
                "malformed" => PublicationFault::MalformedTerminal,
                "nonce" => PublicationFault::TerminalField(
                    "nonce",
                    json!(uuid::Uuid::new_v4().to_string()),
                ),
                "publication" => {
                    PublicationFault::TerminalField("publication", json!("not_published"))
                }
                field => {
                    match field {
                        "command" => forged["command"] = json!("push"),
                        "requested_mode" => forged["data"]["requested_mode"] = json!("INCREMENTAL"),
                        "mode" => forged["data"]["mode"] = json!("UNVERIFIED"),
                        "mode_reason" => forged["data"]["mode_reason"] = json!("weather"),
                        "extension" => forged["data"]["extension"] = json!("Other"),
                        "provider_dispatched" => {
                            forged["data"]["provider_dispatched"] = json!(false)
                        }
                        _ => unreachable!(),
                    }
                    PublicationFault::TerminalField("result", forged)
                }
            });
            let result = run(
                root.path(),
                &prepared(root.path(), ExportMode::Full, None, None, false),
                &runner,
            );
            assert!(!result.ok, "{defect}: {result:?}");
            assert_eq!(
                result.diagnostics[0]["code"], "invalid_result",
                "{defect}: {result:?}"
            );
            assert_eq!(
                result.changed,
                vec![json!({"path":"main","kind":"replaced"})],
                "{defect}"
            );
            assert_eq!(result.data.as_ref().unwrap()["files"], 2);
            assert!(result
                .warnings
                .iter()
                .any(|warning| warning["code"] == "sync_memory_unconfirmed"));
            assert!(!target.join("replaced.bsl").exists());
            assert!(target.join("Catalogs/Items.xml").is_file());
        }
    }

    #[test]
    fn full_export_preserves_concurrent_occupant_and_retains_rollback_recovery() {
        use crate::infrastructure::platform::full_dump_publication::{
            with_publication_hook, PublicationCheckpoint,
        };
        let root = workspace();
        let target = root.path().join("main");
        write_valid_dump(&target, false);
        fs::write(target.join("original.bsl"), "recover me").unwrap();
        let runner = SequenceRunner::exporting(
            vec![
                process(envelope(root.path(), "main", "FULL", None, false), true),
                process(envelope(root.path(), "main", "FULL", None, true), true),
            ],
            target.clone(),
        );
        let occupant = target.clone();
        let result = with_publication_hook(
            PublicationCheckpoint::AfterBackup,
            move || {
                fs::create_dir(&occupant).unwrap();
                fs::write(occupant.join("foreign.bsl"), "keep the concurrent owner").unwrap();
            },
            || {
                run(
                    root.path(),
                    &prepared(root.path(), ExportMode::Full, None, None, false),
                    &runner,
                )
            },
        );
        assert!(!result.ok, "{result:?}");
        assert_eq!(
            fs::read(target.join("foreign.bsl")).unwrap(),
            b"keep the concurrent owner"
        );
        assert!(!target.join("Catalogs/Items.xml").exists());
        let recovery_root = fs::read_dir(root.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".unica-dump-guard-")
            })
            .expect("original tree retained for recovery");
        let original = fs::read_dir(recovery_root.join("recovery"))
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("original.bsl"))
            .find(|path| path.is_file())
            .expect("original source in quarantine");
        assert_eq!(fs::read(original).unwrap(), b"recover me");
        assert!(result.changed.is_empty());
    }

    /// Общий декодер различает запрошенный и случившийся режимы. Публичный
    /// guarded pull принимает только force/full; историческая incremental
    /// ветка проверяется здесь на своей границе декодирования конверта.
    #[test]
    fn apply_names_the_export_mode_that_happened_when_it_differs() {
        let root = workspace();
        let mut performed = envelope(root.path(), "main", "INCREMENTAL", None, true);
        performed["data"]["mode"] = json!("FULL");
        performed["data"]["mode_reason"] = json!("version_file");
        let request = prepared(root.path(), ExportMode::Incremental, None, None, false);
        let inputs = capture_inputs(&request).unwrap();
        let identity = normalize_path_identity(root.path()).unwrap();
        let plan = validate_envelope(&request, &inputs, &identity, &performed, true).unwrap();
        assert_eq!(request.arguments.mode, ExportMode::Incremental);
        assert_eq!(plan.performed_mode, "full");
        assert_eq!(plan.mode_reason, Some("version_file"));

        // Ответ на другой запрошенный режим и неизвестный режим не принимаются.
        for (field, value) in [
            ("requested_mode", "FULL"),
            ("mode", "SOMETIMES"),
            ("mode_reason", "weather"),
        ] {
            let mut preview = envelope(root.path(), "main", "INCREMENTAL", None, false);
            preview["data"][field] = json!(value);
            let result = validate_envelope(&request, &inputs, &identity, &preview, false)
                .expect_err("mismatched request or unknown performed mode is invalid");
            assert_eq!(
                result.diagnostics[0]["code"], "invalid_result",
                "{field}: {result:?}"
            );
        }
    }

    #[test]
    fn apply_refuses_another_target_and_an_empty_target() {
        let root = workspace();
        let runner = SequenceRunner::exporting(
            vec![
                process(envelope(root.path(), "main", "FULL", None, false), true),
                process(envelope(root.path(), "ext-sales", "FULL", None, true), true),
            ],
            root.path().join("main"),
        );
        let result = run(
            root.path(),
            &prepared(root.path(), ExportMode::Full, None, None, false),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.changed.is_empty());
        assert!(!root.path().join("main").exists());
        assert!(!root.path().join("ext-sales").exists());

        let root = workspace();
        let runner = SequenceRunner::new(vec![
            process(envelope(root.path(), "main", "FULL", None, false), true),
            process(envelope(root.path(), "main", "FULL", None, true), true),
        ]);
        let result = run(
            root.path(),
            &prepared(root.path(), ExportMode::Full, None, None, false),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.changed.is_empty());
        assert!(!root.path().join("main").exists());

        // Цель есть, но раннер в неё ничего не положил: квитанция пустая.
        let root = workspace();
        std::fs::create_dir(root.path().join("main")).unwrap();
        let runner = SequenceRunner::new(vec![
            process(envelope(root.path(), "main", "FULL", None, false), true),
            process(envelope(root.path(), "main", "FULL", None, true), true),
        ]);
        let result = run(
            root.path(),
            &prepared(root.path(), ExportMode::Full, None, None, false),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.changed.is_empty());
        assert!(root.path().join("main").is_dir());
        assert_eq!(fs::read_dir(root.path().join("main")).unwrap().count(), 0);
    }

    #[test]
    fn runner_refusals_keep_their_outcome() {
        // Снимок отказа `dump` у раннера 0.9.0 писал `provider_dispatched: true`
        // до запуска платформы (0.12 пишет `false`); отказ идёт по коду, а не по
        // этому признаку.
        let root = workspace();
        let failure = |code: &str, message: &str| {
            json!({
                "ok": false,
                "command": "pull",
                "duration_ms": 0,
                "data": {"ok": false, "provider_dispatched": true, "source_set": "ext-sales", "mode": "FULL", "target_path": "", "message": message},
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
            let runner = SequenceRunner::new(vec![process(
                failure(
                    code,
                    "source-set 'ext-sales' is an extension and requires --extension",
                ),
                false,
            )]);
            let result = run(
                root.path(),
                &prepared(root.path(), ExportMode::Full, Some("main"), None, true),
                &runner,
            );
            assert_eq!(result.diagnostics[0]["code"], expected, "{code}");
            assert_eq!(map_runner_code(code).outcome(), outcome, "{code}");
        }
    }

    fn verify_rejected_ready_retains_the_stage_without_a_process_tree_witness(invalid_xml: bool) {
        use crate::infrastructure::platform::filesystem::{
            file_identity, open_directory_nofollow, open_regular_child_nofollow,
            verify_owner_only_acl,
        };
        use crate::infrastructure::platform::full_dump_publication::{
            with_publication_hook, PublicationCheckpoint,
        };
        use std::ffi::OsStr;
        use std::io::Read;

        let root = workspace();
        let target = root.path().join("main");
        write_valid_dump(&target, false);
        fs::write(target.join("local.bsl"), b"original public source").unwrap();
        fs::write(
            root.path().join(LOCAL_CONFIG_NAME),
            "infobase:\n  password: ready-retention-secret\n",
        )
        .unwrap();
        let original_primary = fs::read(root.path().join(CONFIG_NAME)).unwrap();
        let original_local = fs::read(root.path().join(LOCAL_CONFIG_NAME)).unwrap();
        let target_before = crate::test_support::tree_snapshot(&target);
        let target_identity = file_identity(&open_directory_nofollow(&target).unwrap()).unwrap();
        let mut runner = SequenceRunner::exporting(
            vec![
                process(envelope(root.path(), "main", "FULL", None, false), true),
                process(envelope(root.path(), "main", "FULL", None, true), true),
            ],
            target.clone(),
        );
        if invalid_xml {
            // Actual fake provider write, without touching the cleanup ledger.
            runner.after_apply = Some(Box::new(move |_, stage| {
                fs::write(stage.join("Configuration.xml"), b"<invalid-owned-xml>").unwrap();
            }));
        }
        let config = root.path().join(CONFIG_NAME);
        let mut concurrent_input = fs::read(&config).unwrap();
        concurrent_input.extend_from_slice(b"\n# concurrent input after authenticated Ready\n");
        let hook_input = concurrent_input.clone();
        let invoke = || {
            run(
                root.path(),
                &prepared(root.path(), ExportMode::Full, None, None, false),
                &runner,
            )
        };
        let result = if invalid_xml {
            invoke()
        } else {
            with_publication_hook(
                PublicationCheckpoint::AfterStagedCapture,
                move || fs::write(&config, &hook_input).unwrap(),
                invoke,
            )
        };
        assert!(!result.ok);
        assert!(result.changed.is_empty());
        assert_eq!(
            result.diagnostics[0]["code"],
            if invalid_xml {
                "invalid_result"
            } else {
                "concurrent_change"
            }
        );
        if invalid_xml {
            assert_eq!(result.diagnostics[0]["outcome"], "deadEnd");
            assert_eq!(
                fs::read(root.path().join(CONFIG_NAME)).unwrap(),
                original_primary
            );
        } else {
            assert_eq!(
                fs::read(root.path().join(CONFIG_NAME)).unwrap(),
                concurrent_input
            );
        }
        assert_eq!(crate::test_support::tree_snapshot(&target), target_before);
        assert_eq!(
            file_identity(&open_directory_nofollow(&target).unwrap()).unwrap(),
            target_identity
        );
        let replies = runner.replies.lock().unwrap();
        assert_eq!(
            replies
                .iter()
                .filter(|reply| reply["phase"] == "ready")
                .count(),
            1
        );
        assert!(replies
            .iter()
            .any(|reply| { reply["phase"] == "ready" && reply["decision"] == "rejected" }));
        assert!(!replies.iter().any(|reply| reply["decision"] == "published"));
        drop(replies);
        let terminals = runner.terminals.lock().unwrap();
        let terminal = terminals.last().unwrap();
        assert_eq!(terminal["publication"], "not_published");
        assert_eq!(terminal["memory"], "unchanged");
        drop(terminals);
        let stages = runner.stage_writes.lock().unwrap();
        assert_eq!(stages.len(), 1);
        let stage = &stages[0];
        let private_root = stage.parent().unwrap().parent().unwrap();
        // Generic ProcessStreamOutput has no ALL-processes/both-EOF proof.
        // The stream control retains this named tree even though the fake
        // writer returned synchronously. Its stable runner projection remains
        // private; it has a separate lifetime from this execution stage.
        assert!(stage.is_dir());
        assert!(stage.join("Catalogs/Items.xml").is_file());
        if invalid_xml {
            assert_eq!(
                fs::read(stage.join("Configuration.xml")).unwrap(),
                b"<invalid-owned-xml>"
            );
        }
        assert_retained_artifact_names_directory(&result, root.path(), private_root);
        let calls = runner.calls.lock().unwrap();
        let command = calls.last().unwrap();
        let config_position = command
            .args
            .iter()
            .position(|argument| argument == "--config")
            .expect("the canonical runner receives an explicit config")
            + 1;
        let projected_config = Path::new(&command.args[config_position]);
        let canonical_root = super::normalize_path_identity(root.path()).unwrap();
        let projection_root = super::super::runner_015::projection_directory(&canonical_root);
        assert_eq!(projected_config, projection_root.join(CONFIG_NAME));
        assert!(
            !projection_root.starts_with(private_root),
            "the stable runner projection is separate from retained execution data"
        );
        let projection_directory = open_directory_nofollow(&projection_root).unwrap();
        verify_owner_only_acl(&projection_directory).unwrap();
        let mut projected_primary =
            open_regular_child_nofollow(&projection_directory, OsStr::new(CONFIG_NAME)).unwrap();
        let mut projected_local =
            open_regular_child_nofollow(&projection_directory, OsStr::new(LOCAL_CONFIG_NAME))
                .unwrap();
        verify_owner_only_acl(&projected_primary).unwrap();
        verify_owner_only_acl(&projected_local).unwrap();
        let mut primary_bytes = Vec::new();
        projected_primary.read_to_end(&mut primary_bytes).unwrap();
        let mut local_bytes = Vec::new();
        projected_local.read_to_end(&mut local_bytes).unwrap();
        let local: serde_yaml::Value = serde_yaml::from_slice(&local_bytes).unwrap();
        let origin = &local["infobases"]["origin"];
        assert_eq!(
            origin["password"].as_str(),
            Some("ready-retention-secret"),
            "the stable owner-only local layer preserves the merged origin"
        );
        let connection = origin["connection"].as_str().unwrap();
        let projected_infobase = connection
            .strip_prefix("File=\"")
            .and_then(|path| path.strip_suffix('"'))
            .expect("the fixture keeps its file-infobase connection in the local origin");
        assert_eq!(
            super::normalize_path_identity(Path::new(projected_infobase)).unwrap(),
            super::normalize_path_identity(&root.path().join("build/ib")).unwrap()
        );
        assert!(!String::from_utf8(primary_bytes)
            .unwrap()
            .contains("ready-retention-secret"));
        assert_eq!(
            fs::read(root.path().join(LOCAL_CONFIG_NAME)).unwrap(),
            original_local
        );
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains("ready-retention-secret"));
    }

    #[test]
    fn invalid_owned_xml_ready_rejects_and_retains_the_unknown_writer_stage() {
        verify_rejected_ready_retains_the_stage_without_a_process_tree_witness(true);
    }

    #[test]
    fn authenticated_ready_capture_precedes_concurrent_input_refusal() {
        verify_rejected_ready_retains_the_stage_without_a_process_tree_witness(false);
    }
}
