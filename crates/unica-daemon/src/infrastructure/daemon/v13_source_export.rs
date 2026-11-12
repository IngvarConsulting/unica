#![allow(clippy::result_large_err)]
//! `pull` — выгрузка базы в набор исходников рабочего пространства
//! силами `v8-runner pull` (A-6 зонтика #871). Пара к `push`: тот
//! вносит исходники в базу, этот выносит базу в исходники.
//!
//! Аргументы закрыты: `force`, `sourceSet` и `extension`. Без `force` режим
//! выбирает раннер по своей памяти о прошлом обмене, `force:true` требует
//! полной выгрузки. `sourceSet` — имя объявленного набора, без него раннер
//! берёт набор конфигурации; набор расширения требует явного `extension`.
//!
//! Превью зовёт `pull --dry-run`: раннер называет набор, режим и целевой
//! каталог, ничего не записывая. Исполнение вызывает раннер один раз, цель
//! пишет он сам; Unica сверяет ответ с вызовом и не хранит копию прежних
//! исходников. Цель обязана лежать внутри рабочего пространства и наружу
//! уходит относительным путём. Путь к платформе и командная строка наружу
//! не идут.

use super::protocol::InvocationRequest;
use super::runner::PinnedRunner;
use super::v13_infobase_exports::{
    digest_optional_workspace_file, digest_required_workspace_file, missing_runner_rejection,
    note_runner_warnings, resolve_bundled_runner, runner_envelope_refusal, runner_rejection,
    runner_start_rejection, valid_1c_identifier, RunnerAbsence, CONFIG_NAME, LOCAL_CONFIG_NAME,
    RUNNER_OUTPUT_LIMIT,
};
use super::v13_source_set_name::{source_set_name_guidance, valid_source_set_name};
use unica_artifact_delivery::engine::BundledTool;
use unica_execution::invocation_store::ToolIdentity;
use unica_process::runner::{
    ProcessCommand, ProcessOutput, ProcessRunner, ProcessStreamFailure, SystemProcessRunner,
};
use unica_source_contract::workspace::WorkspaceContext;

use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use unica_execution_contract::invocation::{DomainResult, HumanChoice, SafeIdentityHash};
use unica_redaction::redactor;
use unica_refusal::RefusalCode;
use unica_work_control::cancellation::CancellationToken;
use unica_workspace::paths::normalize_path_identity;

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
    const fn public_name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Incremental => "incremental",
        }
    }

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
        // Без `force` режим выбирает раннер по своей памяти о прошлом обмене;
        // `force:true` требует полной выгрузки.
        let force = match args.get("force") {
            None | Some(Value::Bool(false)) => false,
            Some(Value::Bool(true)) => true,
            Some(_) => return Err(reject(RefusalCode::BadValue, "pull force must be boolean")),
        };
        let mut public = args.clone();
        public.remove("force");
        public.insert(
            "mode".into(),
            json!(if force { "full" } else { "incremental" }),
        );
        let arguments = parse_export_arguments(&public)?;
        // Полная выгрузка заменяет весь набор: с несохранёнными правками или
        // без git она исполняется только с согласия человека. Вопрос
        // задаётся до допуска: вызов, ушедший в фоновую задачу, спросить уже
        // не может. Превью и чистый набор не спрашивают. Набор, который по
        // проекту не определить, разбирает исполнение своим отказом.
        if force && !dry_run {
            if let Some((source_set, directory)) =
                selected_source_set(&context.workspace_root, &arguments)
            {
                let local =
                    local_changes(&SystemProcessRunner, &directory, &CancellationToken::new())
                        .map_err(|(code, error)| reject(code, error))?;
                if let Some(question) = pull_unsaved_question(&source_set, local) {
                    if !HumanChoice::is_confirmed(&question, "unica.run", request.arguments()) {
                        return Err(pull_unsaved_unconfirmed(
                            &source_set,
                            local,
                            request.arguments(),
                        ));
                    }
                }
            }
        }
        Ok(Self {
            arguments,
            dry_run,
            context,
        })
    }

    /// The workspace root whose lane of source writes the execution holds.
    pub(super) fn workspace_root(&self) -> &std::path::Path {
        &self.context.workspace_root
    }

    /// A preview neither writes nor loads sources, so it takes no lane.
    pub(super) fn is_preview(&self) -> bool {
        self.dry_run
    }
    pub(super) fn workspace_identity_hash(&self) -> SafeIdentityHash {
        let mut hasher = Sha256::new();
        hasher.update(b"unica-v13-source-export-workspace-v1\0");
        hasher.update(self.context.workspace_root.as_os_str().as_encoded_bytes());
        SafeIdentityHash::from_sha256(hasher.finalize().into())
    }

    pub(super) fn execute(&self, cancellation: CancellationToken) -> DomainResult {
        super::runner::with_bundled_runner(
            &self.context.workspace_root,
            &self.context.cwd,
            OPERATION,
            cancellation,
            |cancellation| execute_with_runner(self, &PinnedRunner, cancellation),
        )
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
    // Набор расширения выгружается только с явным именем расширения, набор
    // конфигурации — без него; отказ приходит до запуска раннера.
    let extension_set = match &prepared.arguments.source_set {
        Some(source_set) => is_extension_set(&root.join(CONFIG_NAME), source_set)?,
        None => false,
    };
    if extension_set != prepared.arguments.extension.is_some() {
        return Err(reject(
            RefusalCode::BadValue,
            "pull requires one Configuration source set or an Extension source set with an explicit extension argument",
        ));
    }
    Ok(StableInputs {
        config_sha256,
        local_config_sha256,
        declared,
    })
}

/// Объявлен ли набор `name` как набор расширения (`type: EXTENSION`).
fn is_extension_set(config: &Path, name: &str) -> Result<bool, DomainResult> {
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
    Ok(document
        .get("source-set")
        .and_then(serde_yaml::Value::as_sequence)
        .into_iter()
        .flatten()
        .filter(|entry| entry.get("name").and_then(serde_yaml::Value::as_str) == Some(name))
        .any(|entry| {
            entry
                .get("type")
                .and_then(serde_yaml::Value::as_str)
                .is_some_and(|kind| kind.eq_ignore_ascii_case("EXTENSION"))
        }))
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
    /// Цель была на месте до запуска раннера: исполнение назовёт её обновлённой.
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
    if cancellation.is_cancelled() {
        return reject(RefusalCode::Cancelled, "pull cancelled before preparation");
    }
    let inputs = match capture_inputs(prepared) {
        Ok(value) => value,
        Err(result) => return result,
    };
    let root = match normalize_path_identity(&prepared.context.workspace_root) {
        Ok(value) => value,
        Err(_) => {
            return reject(
                RefusalCode::InvalidState,
                "pull could not resolve the workspace root",
            )
        }
    };
    // Существование цели снимается до запуска: после записи раннером оно
    // уже не говорит, создана цель или обновлена.
    let existed_before = selected_target_exists(prepared, &root);
    // Исполнение вызывает раннер один раз; цель пишет сам раннер, а правки
    // набора через Unica на это время исключены блокировкой рабочей области.
    let envelope = match invoke_runner(prepared, tool, runner, &cancellation) {
        Ok(envelope) => envelope,
        Err(result) => return result,
    };
    let mut plan = match validate_envelope(prepared, &inputs, &root, &envelope, !prepared.dry_run) {
        Ok(plan) => plan,
        Err(mut result) => {
            // Раннер завершился, но его ответ не принят: он мог уже писать.
            if envelope["data"]["provider_dispatched"] == true {
                note_partial_export(prepared, &mut result);
            }
            return result;
        }
    };
    if let Some(existed) = existed_before {
        plan.existed = existed;
    }
    if prepared.dry_run {
        // Полная выгрузка заменяет весь набор: превью называет, сколько
        // несохранённых в git правок она сотрёт, или что набор не под git.
        let local = if prepared.arguments.mode == ExportMode::Full {
            // git — не раннер: его зовёт обычный процесс, без обёртки v8-runner.
            match local_changes(&SystemProcessRunner, &plan.target_absolute, &cancellation) {
                Ok(local) => Some(local),
                Err((code, error)) => return reject(code, error),
            }
        } else {
            None
        };
        preview_result(prepared, plan, &envelope, local)
    } else {
        applied_result(prepared, plan, &envelope)
    }
}

fn invoke_runner(
    prepared: &PreparedSourceExport,
    tool: &BundledTool,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
) -> Result<Value, DomainResult> {
    if cancellation.is_cancelled() {
        return Err(reject(
            RefusalCode::Cancelled,
            "pull cancelled before provider launch",
        ));
    }
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
    if let Some(source_set) = &prepared.arguments.source_set {
        args.push(source_set.clone());
    }
    if prepared.arguments.mode == ExportMode::Full {
        args.push("--force".to_string());
    }
    if let Some(extension) = &prepared.arguments.extension {
        args.push("--extension".to_string());
        args.push(extension.clone());
    }
    if prepared.dry_run {
        args.push("--dry-run".to_string());
    }
    let output = runner
        .run_staged(&ProcessCommand {
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
            runner_start_rejection(Some(OPERATION.to_string()), &error, &tool.program)
        })?;
    parse_runner_output(output).map_err(|(mut result, envelope)| {
        // Отказ раннера до платформы ничего не писал; иначе цель могла
        // остаться частичной.
        if envelope["data"]["provider_dispatched"] != false {
            note_partial_export(prepared, &mut result);
        }
        note_runner_warnings(&mut result, &envelope);
        result
    })
}

fn parse_runner_output(output: ProcessOutput) -> Result<Value, (DomainResult, Value)> {
    if output.cancelled {
        return Err((
            reject(RefusalCode::Cancelled, "v8-runner was cancelled"),
            Value::Null,
        ));
    }
    if output.timed_out {
        return Err((
            reject(
                RefusalCode::DeadlineExceeded,
                "v8-runner exceeded its execution deadline",
            ),
            Value::Null,
        ));
    }
    if output.stdout_truncated || output.stdout_had_invalid_utf8 {
        return Err((
            reject(
                RefusalCode::InvalidResult,
                "v8-runner returned an unreadable or oversized JSON result",
            ),
            Value::Null,
        ));
    }
    let envelope: Value = serde_json::from_str(&output.stdout).map_err(|_| {
        (
            reject(
                RefusalCode::InvalidResult,
                "v8-runner returned an invalid JSON result",
            ),
            Value::Null,
        )
    })?;
    validate_runner_result(envelope.clone(), output.status_success)
        .map_err(|result| (result, envelope))
}

/// Цель выбранного набора по `v8project.yaml` до запуска раннера: `None`, если
/// набор не определить (тогда решает ответ раннера).
fn selected_target_exists(prepared: &PreparedSourceExport, root: &Path) -> Option<bool> {
    let (_, directory) = selected_source_set(root, &prepared.arguments)?;
    Some(directory.is_dir())
}

/// Выбранный набор по `v8project.yaml`: его имя и каталог. Без `sourceSet` —
/// набор конфигурации. `None`, если набор не определить.
fn selected_source_set(root: &Path, arguments: &ExportArguments) -> Option<(String, PathBuf)> {
    let text = std::fs::read_to_string(root.join(CONFIG_NAME)).ok()?;
    let document: serde_yaml::Value = serde_yaml::from_str(&text).ok()?;
    let entry = document
        .get("source-set")?
        .as_sequence()?
        .iter()
        .find(|entry| match &arguments.source_set {
            Some(name) => entry.get("name").and_then(serde_yaml::Value::as_str) == Some(name),
            None => entry
                .get("type")
                .and_then(serde_yaml::Value::as_str)
                .is_some_and(|kind| kind.eq_ignore_ascii_case("CONFIGURATION")),
        })?;
    let name = entry.get("name").and_then(serde_yaml::Value::as_str)?;
    let path = entry.get("path").and_then(serde_yaml::Value::as_str)?;
    Some((name.to_string(), root.join(path)))
}

/// Код вопроса о полной выгрузке поверх несохранённых правок.
const PULL_UNSAVED_CODE: &str = "source.pull_unsaved";

/// Согласованный текст вопроса о полной выгрузке; `None` — чистый набор, о
/// нём не спрашивают. Число правок входит в текст: согласие на другое число
/// не годится.
fn pull_unsaved_question(source_set: &str, local: LocalChanges) -> Option<String> {
    match local {
        LocalChanges::Files(0) => None,
        LocalChanges::Files(files) => Some(format!(
            "В исходниках набора «{source_set}» есть несохранённые правки, файлов: {files}."
        )),
        LocalChanges::NotUnderGit => Some(format!(
            "Исходники набора «{source_set}» не под git: несохранённые правки проверить нельзя."
        )),
    }
}

/// Подтверждение полной выгрузки этим же вызовом: надписи согласованы и
/// закреплены тестом.
fn pull_unsaved_confirmation(
    source_set: &str,
    local: LocalChanges,
    call: &Map<String, Value>,
) -> Option<HumanChoice> {
    let question = pull_unsaved_question(source_set, local)?;
    let (description, cancel, facts) = match local {
        LocalChanges::Files(files) => (
            format!("правки пропадут, файлов: {files}"),
            Some("сохраните правки в git и повторите".to_string()),
            json!({"sourceSet": source_set, "underGit": true, "files": files}),
        ),
        LocalChanges::NotUnderGit => (
            "правки в файлах набора пропадут".to_string(),
            None,
            json!({"sourceSet": source_set, "underGit": false}),
        ),
    };
    let mut choice = HumanChoice::confirmation(
        PULL_UNSAVED_CODE,
        question,
        "Перезаписать исходники",
        description,
        "unica.run",
        call,
    )
    .with_data(facts);
    choice.cancel_description = cancel;
    Some(choice)
}

/// Отказ `cancelled` без согласия человека: исходники не тронуты. Клиенту,
/// который умеет спрашивать, дверь задаёт вопрос; остальным исполняет вызов
/// как прежде.
fn pull_unsaved_unconfirmed(
    source_set: &str,
    local: LocalChanges,
    call: &Map<String, Value>,
) -> DomainResult {
    let mut rejection = reject(
        RefusalCode::Cancelled,
        "pull with force:true replaces the whole source set, which has unsaved changes or is not under git; it runs only with the person's confirmation, it was not given, and the sources are unchanged",
    );
    rejection.human_choice = pull_unsaved_confirmation(source_set, local, call).map(Box::new);
    rejection
}

/// Несохранённые правки каталога набора перед полной выгрузкой: она
/// заменяет весь набор, и правки, не сохранённые в git, пропадут.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LocalChanges {
    /// Изменённые, удалённые и неотслеживаемые файлы, не исключённые
    /// `.gitignore`; ноль — правок нет.
    Files(usize),
    /// Каталог набора не под git: правки проверить нельзя.
    NotUnderGit,
}

impl LocalChanges {
    fn public(self) -> Value {
        match self {
            Self::Files(files) => json!({"underGit": true, "files": files}),
            Self::NotUnderGit => json!({"underGit": false}),
        }
    }
}

/// Считает по git несохранённые правки в `target`. Отсутствующий каталог —
/// правок нет. Каталог вне рабочего дерева git или без git на машине —
/// `NotUnderGit`; ответ git, который не прочитать, — отказ, а не ноль.
// resource-limit: pull.git-status-output-bytes
// class: retention
// unit: bytes
// scope: process
// value: 134217728
// resource: Захват stdout `git status` каталога набора в превью полной выгрузки pull.
// status: unproven
// origin: #2843 (PR5): одна запись на файл, в неотслеживаемом наборе — десятки тысяч файлов с длинными путями
// problem: предел вывода раннера (1 МиБ) отказывал превью первого pull большого набора
// on-reach: вывод сверх предела — отказ invalid_state «git status did not report the changes of the source set»; правки не выдаются за отсутствующие.
const GIT_STATUS_OUTPUT_LIMIT: usize = 128 * 1024 * 1024;
// resource-limit: pull.git-timeout
// class: deadline
// unit: ms
// scope: process
// value: 120000
// resource: Каждый вызов git (rev-parse, check-ignore, status) при подсчёте правок набора в превью полной выгрузки pull.
// status: unproven
// origin: #2843 (PR5)
// problem: git, который не отвечает (fsmonitor, сетевой диск), держал бы превью без конца
// on-reach: отказ deadline_exceeded «git <команда> did not answer in time»; превью не выдаёт правки за отсутствующие.
const GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

pub(super) fn local_changes(
    runner: &dyn ProcessRunner,
    target: &Path,
    cancellation: &CancellationToken,
) -> Result<LocalChanges, (RefusalCode, String)> {
    if !target.is_dir() {
        return Ok(LocalChanges::Files(0));
    }
    let git = |args: &[&str]| {
        runner.run_staged(&ProcessCommand {
            program: PathBuf::from("git"),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            cwd: target.to_path_buf(),
            // Messages stay in English: "not a git repository" is read below.
            env: vec![
                ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
                ("LC_ALL".into(), "C".into()),
                ("LANGUAGE".into(), "C".into()),
            ],
            env_remove: [
                "GIT_DIR",
                "GIT_WORK_TREE",
                "GIT_INDEX_FILE",
                "GIT_COMMON_DIR",
            ]
            .into_iter()
            .map(Into::into)
            .collect(),
            capture_limits: Some((GIT_STATUS_OUTPUT_LIMIT, RUNNER_OUTPUT_LIMIT)),
            timeout: Some(GIT_TIMEOUT),
            cancellation: cancellation.clone(),
        })
    };
    let failed = |what: &str, output: &ProcessOutput| -> Option<(RefusalCode, String)> {
        if output.cancelled {
            Some((
                RefusalCode::Cancelled,
                format!("pull cancelled during git {what}"),
            ))
        } else if output.timed_out {
            Some((
                RefusalCode::DeadlineExceeded,
                format!("git {what} did not answer in time"),
            ))
        } else {
            None
        }
    };
    let inside = match git(&["rev-parse", "--is-inside-work-tree"]) {
        Ok(output) => output,
        // Нет git на машине: правки проверить нельзя, как и вне репозитория.
        Err(ProcessStreamFailure::Spawn(_)) => return Ok(LocalChanges::NotUnderGit),
        Err(error) => {
            return Err((
                RefusalCode::InvalidState,
                format!("git rev-parse failed: {}", redactor(error.message())),
            ))
        }
    };
    if let Some(refusal) = failed("rev-parse", &inside) {
        return Err(refusal);
    }
    if !inside.status_success {
        // Вне репозитория правок не проверить; любой другой отказ git
        // (например, чужой владелец репозитория) — не «не под git».
        if inside.stderr.contains("not a git repository") {
            return Ok(LocalChanges::NotUnderGit);
        }
        return Err((
            RefusalCode::InvalidState,
            format!(
                "git could not tell whether the source set is under git: {}",
                redactor(inside.stderr.trim())
            ),
        ));
    }
    if inside.stdout.trim() != "true" {
        return Ok(LocalChanges::NotUnderGit);
    }
    // Каталог набора целиком исключён `.gitignore`: git его не хранит, и
    // «правок нет» соврало бы — набор пропадёт весь.
    let ignored = git(&["check-ignore", "-q", "."]).map_err(|error| {
        (
            RefusalCode::InvalidState,
            format!("git check-ignore failed: {}", redactor(error.message())),
        )
    })?;
    if let Some(refusal) = failed("check-ignore", &ignored) {
        return Err(refusal);
    }
    if ignored.status_success {
        return Ok(LocalChanges::NotUnderGit);
    }
    let status = git(&[
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--no-renames",
        "--",
        ".",
    ])
    .map_err(|error| {
        (
            RefusalCode::InvalidState,
            format!("git status failed: {}", redactor(error.message())),
        )
    })?;
    if let Some(refusal) = failed("status", &status) {
        return Err(refusal);
    }
    if !status.status_success || status.stdout_truncated || status.stdout_had_invalid_utf8 {
        return Err((
            RefusalCode::InvalidState,
            "git status did not report the changes of the source set".to_string(),
        ));
    }
    Ok(LocalChanges::Files(
        status
            .stdout
            .split('\0')
            .filter(|entry| !entry.is_empty())
            .count(),
    ))
}

/// Unica не сохраняет прежние исходники и не откатывает выгрузку: отказ
/// исполнения после запуска раннера говорит, что цель может содержать
/// частичную выгрузку, и предлагает повторить её полностью.
fn note_partial_export(prepared: &PreparedSourceExport, result: &mut DomainResult) {
    if prepared.dry_run {
        return;
    }
    result.warnings.push(json!({
        "code": "partial_export_possible",
        "message": "v8-runner may have written part of the export: the target can hold a partial export; Unica keeps no copy of the previous sources",
    }));
    let mut args = public_arguments(prepared);
    args["force"] = Value::Bool(true);
    result.next.push(json!({
        "tool": "unica.run",
        "args": {"op": OPERATION, "args": args, "dryRun": false},
        "reason": "repeat the export in full to replace a possibly partial target",
    }));
}

fn applied_result(
    prepared: &PreparedSourceExport,
    plan: ExportPlan,
    envelope: &Value,
) -> DomainResult {
    let target = path_text(&plan.target);
    let up_to_date = envelope["data"]["up_to_date"] == true;
    let state = if up_to_date {
        "unchanged"
    } else if plan.existed {
        "updated"
    } else {
        "created"
    };
    let summary = if up_to_date {
        format!(
            "pull found source set `{}` up to date; nothing was exported",
            plan.source_set
        )
    } else {
        format!(
            "pull exported source set `{}` into `{target}` in {} mode",
            plan.source_set, plan.performed_mode
        )
    };
    let mut result = DomainResult::success(summary);
    result.data = Some(json!({
        "op": OPERATION,
        "dryRun": false,
        "sourceSet": plan.source_set,
        "extension": prepared.arguments.extension,
        "mode": prepared.arguments.mode.public_name(),
        "performedMode": plan.performed_mode,
        "modeReason": plan.mode_reason,
        "target": target,
        "state": state,
        "providerDispatched": true,
    }));
    if !up_to_date {
        result.changed.push(json!({"path": target, "kind": state}));
    }
    note_runner_warnings(&mut result, envelope);
    result
}

fn preview_result(
    prepared: &PreparedSourceExport,
    plan: ExportPlan,
    envelope: &Value,
    local: Option<LocalChanges>,
) -> DomainResult {
    let force = prepared.arguments.mode == ExportMode::Full;
    let mut result = DomainResult::success(format!(
        "pull planned a {} export of source set `{}`",
        plan.performed_mode, plan.source_set
    ));
    result.data = Some(json!({
        "op": OPERATION,
        "dryRun": true,
        "plan": {
            "sourceSet": plan.source_set,
            "extension": prepared.arguments.extension,
            "mode": prepared.arguments.mode.public_name(),
            "plannedMode": plan.performed_mode,
            "modeReason": plan.mode_reason,
            "target": path_text(&plan.target),
            "targetExists": plan.existed,
            "force": force,
            "replacesLocalSources": force,
            "localChanges": local.map(LocalChanges::public),
        },
        "providerDispatched": false,
        "requiresPlatform": true,
    }));
    result.next.push(json!({
        "tool": "unica.run",
        "args": {"op": OPERATION, "args": public_arguments(prepared), "dryRun": false},
        "reason": "execute with the current arguments",
    }));
    note_runner_warnings(&mut result, envelope);
    result
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
    let target = unica_workspace::paths::host_path_relative_to(&target_absolute, root_identity)
        .filter(|relative| !relative.as_os_str().is_empty())
        .ok_or_else(|| {
            reject(
                RefusalCode::InvalidResult,
                format!("v8-runner {phase} placed the export target outside the workspace"),
            )
        })?;
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
        return Err(runner_envelope_refusal(
            runner_rejection(Some(OPERATION.to_string()), code, message),
            &envelope,
        ));
    }
    Ok(envelope)
}

fn public_arguments(prepared: &PreparedSourceExport) -> Value {
    let mut args = Map::new();
    if prepared.arguments.mode == ExportMode::Full {
        args.insert("force".to_string(), Value::Bool(true));
    }
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
mod medium_tests {
    use super::super::v13_infobase_exports::map_runner_code;
    use super::*;
    use std::fs;
    use std::sync::Mutex;
    use unica_refusal::Outcome;

    /// Раннер с заготовленными ответами: `pull` вызывает его один раз,
    /// без протокола публикации.
    struct SequenceRunner {
        outputs: Mutex<Vec<ProcessOutput>>,
        calls: Mutex<Vec<ProcessCommand>>,
        /// Каталог, который раннер создаёт во время вызова, как настоящая выгрузка.
        writes: Option<PathBuf>,
    }

    impl SequenceRunner {
        fn new(outputs: Vec<ProcessOutput>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into_iter().rev().collect()),
                calls: Mutex::new(Vec::new()),
                writes: None,
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
            if let Some(target) = &self.writes {
                fs::create_dir_all(target).unwrap();
            }
            self.outputs
                .lock()
                .unwrap()
                .pop()
                .ok_or_else(|| "unexpected v8-runner call".to_string())
        }
    }

    fn run(root: &Path, prepared: &PreparedSourceExport, runner: &SequenceRunner) -> DomainResult {
        execute_with_resolved_runner(
            prepared,
            runner,
            CancellationToken::new(),
            &tool(root),
            "0.17.0-test-fixture",
        )
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

    fn captured_014(name: &str) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/v8_runner_014")
            .join(name);
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

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
            "message": "fixture",
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

    fn failure(code: &str, message: &str) -> Value {
        json!({
            "ok": false,
            "command": "pull",
            "duration_ms": 0,
            "data": {"ok": false, "provider_dispatched": true, "source_set": "main", "mode": "FULL", "target_path": "", "message": message},
            "warnings": [],
            "steps": [],
            "error": {"code": code, "kind": "validation", "message": message}
        })
    }

    /// Без `force` режим выбирает раннер; `force:true` требует полной
    /// выгрузки, а не логическое значение — неверный вызов.
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
    #[test]
    fn force_is_optional_and_only_force_requests_a_full_export() {
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
        let full = PreparedSourceExport::parse(&request(json!({"force":true}))).unwrap();
        assert_eq!(full.arguments.mode, ExportMode::Full);
        for args in [json!({}), json!({"force": false})] {
            let incremental = PreparedSourceExport::parse(&request(args.clone()))
                .unwrap_or_else(|_| panic!("{args} must be accepted"));
            assert_eq!(
                incremental.arguments.mode,
                ExportMode::Incremental,
                "{args}"
            );
        }
        let refused = PreparedSourceExport::parse(&request(json!({"force":"yes"})))
            .expect_err("non-boolean force is refused");
        assert_eq!(refused.diagnostics[0]["code"], "bad_value");
    }

    /// Превью без `force` не заказывает полную выгрузку и предлагает
    /// исполнить те же аргументы.
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
    #[test]
    fn preview_without_force_leaves_the_mode_to_the_runner() {
        let root = workspace();
        let runner = SequenceRunner::new(vec![process(
            envelope(root.path(), "main", "INCREMENTAL", None, false),
            true,
        )]);
        let result = run(
            root.path(),
            &prepared(
                root.path(),
                ExportMode::Incremental,
                Some("main"),
                None,
                true,
            ),
            &runner,
        );
        assert!(result.ok, "{result:?}");
        let plan = &result.data.as_ref().unwrap()["plan"];
        assert_eq!(plan["mode"], "incremental");
        assert_eq!(plan["plannedMode"], "incremental");
        assert_eq!(plan["force"], false);
        assert_eq!(plan["replacesLocalSources"], false);
        assert_eq!(result.next[0]["args"]["args"], json!({"sourceSet": "main"}));
        assert_eq!(
            runner.joined_args(0),
            format!(
                "--config {} --json-message pull main --dry-run",
                root.path().join(CONFIG_NAME).display()
            )
        );
    }

    /// Исполнение вызывает раннер один раз, без стадии и публикации; квитанция
    /// называет цель и фактический режим.
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
    #[test]
    fn apply_runs_the_runner_once_and_names_the_target_and_the_mode() {
        let root = workspace();
        fs::create_dir(root.path().join("main")).unwrap();
        let runner = SequenceRunner::new(vec![process(
            envelope(root.path(), "main", "FULL", None, true),
            true,
        )]);
        let result = run(
            root.path(),
            &prepared(root.path(), ExportMode::Full, Some("main"), None, false),
            &runner,
        );
        assert!(result.ok, "{result:?}");
        assert_eq!(runner.call_count(), 1);
        let args = runner.joined_args(0);
        assert!(args.ends_with("--json-message pull main --force"), "{args}");
        assert!(!args.contains("--publication-stdio"), "{args}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["target"], "main");
        assert_eq!(data["performedMode"], "full");
        assert_eq!(data["state"], "updated");
        assert!(data.get("files").is_none(), "{data}");
        assert_eq!(
            result.changed,
            vec![json!({"path":"main","kind":"updated"})]
        );
        assert!(result.warnings.is_empty(), "{result:?}");
        assert!(fs::read_dir(root.path()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".unica-dump-guard-")));
        let encoded = serde_json::to_string(&result).unwrap();
        let root_text = normalize_path_identity(root.path())
            .unwrap()
            .display()
            .to_string();
        assert!(
            !encoded.contains(&root_text),
            "absolute path leaked: {encoded}"
        );
    }

    /// Раннер не нашёл изменений: исполнение ничего не выгрузило и цель не
    /// названа изменённой.
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
    #[test]
    fn an_up_to_date_set_is_answered_without_a_change() {
        let root = workspace();
        fs::create_dir(root.path().join("main")).unwrap();
        let mut unchanged = envelope(root.path(), "main", "INCREMENTAL", None, true);
        unchanged["data"]["up_to_date"] = json!(true);
        let runner = SequenceRunner::new(vec![process(unchanged, true)]);
        let result = run(
            root.path(),
            &prepared(
                root.path(),
                ExportMode::Incremental,
                Some("main"),
                None,
                false,
            ),
            &runner,
        );
        assert!(result.ok, "{result:?}");
        assert_eq!(result.data.as_ref().unwrap()["state"], "unchanged");
        assert!(result.changed.is_empty(), "{result:?}");
        assert!(result.summary.contains("up to date"), "{}", result.summary);
    }

    /// Unica не хранит прежние исходники: отказ раннера при исполнении
    /// предупреждает о возможной частичной выгрузке и предлагает повторить её
    /// полностью; превью этого не говорит.
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
    #[test]
    fn a_failed_apply_warns_about_a_partial_export_and_offers_a_full_one() {
        let root = workspace();
        let runner = SequenceRunner::new(vec![process(
            failure("platform_failure", "designer exited with code 1"),
            false,
        )]);
        let result = run(
            root.path(),
            &prepared(
                root.path(),
                ExportMode::Incremental,
                Some("main"),
                None,
                false,
            ),
            &runner,
        );
        assert!(!result.ok);
        assert_eq!(
            result.warnings[0]["code"], "partial_export_possible",
            "{result:?}"
        );
        assert_eq!(
            result.next[0]["args"],
            json!({"op":"pull","args":{"sourceSet":"main","force":true},"dryRun":false})
        );

        let runner = SequenceRunner::new(vec![process(
            failure("platform_failure", "designer exited with code 1"),
            false,
        )]);
        let preview = run(
            root.path(),
            &prepared(
                root.path(),
                ExportMode::Incremental,
                Some("main"),
                None,
                true,
            ),
            &runner,
        );
        assert!(!preview.ok);
        assert!(preview.warnings.is_empty(), "{preview:?}");
    }

    /// Цель, которой до запуска не было, названа созданной, хотя раннер её
    /// уже записал.
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
    #[test]
    fn a_target_absent_before_the_runner_is_named_created() {
        let root = workspace();
        let target = root.path().join("main");
        let mut runner = SequenceRunner::new(vec![process(
            envelope(root.path(), "main", "FULL", None, true),
            true,
        )]);
        runner.writes = Some(target.clone());
        let result = run(
            root.path(),
            &prepared(root.path(), ExportMode::Full, Some("main"), None, false),
            &runner,
        );
        assert!(target.is_dir());
        assert!(result.ok, "{result:?}");
        assert_eq!(result.data.as_ref().unwrap()["state"], "created");
        assert_eq!(
            result.changed,
            vec![json!({"path":"main","kind":"created"})]
        );
    }

    /// Отказ раннера до платформы ничего не писал: предупреждения о частичной
    /// выгрузке нет.
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
    #[test]
    fn a_refusal_before_dispatch_does_not_warn_about_a_partial_export() {
        let root = workspace();
        let mut refused = failure("invalid_argument", "source set is not declared");
        refused["data"]["provider_dispatched"] = json!(false);
        let runner = SequenceRunner::new(vec![process(refused, false)]);
        let result = run(
            root.path(),
            &prepared(
                root.path(),
                ExportMode::Incremental,
                Some("main"),
                None,
                false,
            ),
            &runner,
        );
        assert!(!result.ok);
        assert!(result.warnings.is_empty(), "{result:?}");
        assert!(result.next.is_empty(), "{result:?}");
    }

    /// Ответ раннера о другом наборе не признаётся успехом.
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
    #[test]
    fn apply_refuses_a_result_for_another_source_set() {
        let root = workspace();
        let runner = SequenceRunner::new(vec![process(
            envelope(root.path(), "Sales", "FULL", None, true),
            true,
        )]);
        let result = run(
            root.path(),
            &prepared(root.path(), ExportMode::Full, Some("main"), None, false),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.changed.is_empty());
    }

    /// Живые конверты раннера 0.14.0: `pull` копии в базу другой рабочей копии
    /// идёт с предупреждением раннера. Ответ называет его своим предупреждением
    /// и выходом к своей базе, без прозы раннера. Проверяется путь
    /// предупреждения, а не форма данных: конверт 0.14 без обязательного
    /// в 0.15 `requested_mode` отвергается, поэтому поле добавлено в тесте
    /// (`tests/fixtures/v8_runner_014/README.md`).
    /// rule: INV.RUNTIME.RUNNER-BASE-REFUSALS
    #[test]
    fn captured_pull_into_the_infobase_of_another_copy_names_it() {
        let root = workspace();
        fs::create_dir_all(root.path().join("main")).unwrap();
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

        let runner = SequenceRunner::new(vec![process(applied, true)]);
        let result = run(
            root.path(),
            &prepared(root.path(), ExportMode::Full, Some("main"), None, false),
            &runner,
        );
        assert!(result.ok, "{result:?}");
        super::super::v13_source_import::tests::assert_another_copy_is_named(&result);
    }
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
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
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
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
            .contains("--json-message pull --force"));
        assert!(runner.joined_args(0).ends_with("--dry-run"));
    }
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
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
        assert!(runner.joined_args(0).contains("pull Sales --force"));
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
        assert!(runner.joined_args(0).contains("pull Доработки --force"));
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
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
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
    /// Общий декодер различает запрошенный и случившийся режимы. Публичный
    /// guarded pull принимает только force/full; историческая incremental
    /// ветка проверяется здесь на своей границе декодирования конверта.
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
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
    fn an_export_target_inside_the_workspace_is_recognised_in_any_host_spelling() {
        let root = workspace();
        let request = prepared(root.path(), ExportMode::Incremental, None, None, false);
        let inputs = capture_inputs(&request).unwrap();
        let plain = normalize_path_identity(root.path()).unwrap();
        let extended = unica_workspace::test_support::extended_length_spelling_for_test(&plain);
        for (target, identity) in [
            (extended.join("main"), plain.clone()),
            (plain.join("main"), extended.clone()),
        ] {
            let mut performed = envelope(root.path(), "main", "INCREMENTAL", None, true);
            performed["data"]["target_path"] = json!(target);
            validate_envelope(&request, &inputs, &identity, &performed, true)
                .unwrap_or_else(|result| panic!("{target:?} under {identity:?}: {result:?}"));
        }
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

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_COMMON_DIR")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// Полная выгрузка заменяет набор: подсчёт видит изменённые, удалённые
    /// и неотслеживаемые файлы, но не исключённые `.gitignore`.
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
    #[test]
    fn local_changes_count_changed_deleted_and_untracked_files_under_git() {
        let root = tempfile::tempdir().unwrap();
        git(root.path(), &["init", "-q"]);
        git(root.path(), &["config", "user.email", "t@example.com"]);
        git(root.path(), &["config", "user.name", "test"]);
        let set = root.path().join("main");
        fs::create_dir_all(set.join("CommonModules")).unwrap();
        fs::write(set.join("Configuration.xml"), "configuration").unwrap();
        fs::write(set.join("CommonModules/A.bsl"), "a").unwrap();
        fs::write(set.join("CommonModules/B.bsl"), "b").unwrap();
        fs::write(
            root.path().join(".gitignore"),
            "main/*.log
",
        )
        .unwrap();
        git(root.path(), &["add", "-A"]);
        git(
            root.path(),
            &[
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=",
                "commit",
                "-qm",
                "init",
            ],
        );
        let cancellation = CancellationToken::new();

        assert_eq!(
            local_changes(&SystemProcessRunner, &set, &cancellation).unwrap(),
            LocalChanges::Files(0)
        );
        fs::write(set.join("CommonModules/A.bsl"), "edited").unwrap();
        fs::remove_file(set.join("CommonModules/B.bsl")).unwrap();
        fs::write(set.join("CommonModules/New.bsl"), "new").unwrap();
        fs::write(set.join("ignored.log"), "ignored").unwrap();
        fs::write(root.path().join("outside.txt"), "outside the set").unwrap();
        assert_eq!(
            local_changes(&SystemProcessRunner, &set, &cancellation).unwrap(),
            LocalChanges::Files(3)
        );
    }

    /// Каталог вне git — правки проверить нельзя; отсутствующий каталог —
    /// правок нет.
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
    #[test]
    fn local_changes_name_a_set_outside_git_and_an_absent_one() {
        let root = tempfile::tempdir().unwrap();
        let set = root.path().join("main");
        let cancellation = CancellationToken::new();
        assert_eq!(
            local_changes(&SystemProcessRunner, &set, &cancellation).unwrap(),
            LocalChanges::Files(0)
        );
        fs::create_dir_all(&set).unwrap();
        fs::write(set.join("Configuration.xml"), "configuration").unwrap();
        // Каталог временный, но может лежать внутри чужого репозитория:
        // ответ тогда — число, а не «не под git»; проверяется только вне git.
        let outside = std::process::Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(&set)
            .output()
            .map(|output| !output.status.success())
            .unwrap_or(true);
        if outside {
            assert_eq!(
                local_changes(&SystemProcessRunner, &set, &cancellation).unwrap(),
                LocalChanges::NotUnderGit
            );
        }
    }

    /// Каталог набора, целиком исключённый `.gitignore`, git не хранит: это
    /// «не под git», а не «правок нет».
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
    #[test]
    fn a_set_ignored_by_git_is_not_under_git() {
        let root = tempfile::tempdir().unwrap();
        git(root.path(), &["init", "-q"]);
        fs::write(
            root.path().join(".gitignore"),
            "main/
",
        )
        .unwrap();
        let set = root.path().join("main");
        fs::create_dir_all(&set).unwrap();
        fs::write(set.join("Configuration.xml"), "configuration").unwrap();
        assert_eq!(
            local_changes(&SystemProcessRunner, &set, &CancellationToken::new()).unwrap(),
            LocalChanges::NotUnderGit
        );
    }

    /// Отмена — `cancelled`; отказ git по иной причине, чем «не репозиторий»,
    /// не выдаётся за «не под git».
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
    #[test]
    fn a_cancelled_or_refusing_git_is_not_taken_for_a_set_outside_git() {
        let root = tempfile::tempdir().unwrap();
        let mut cancelled = process(json!(null), false);
        cancelled.cancelled = true;
        let runner = SequenceRunner::new(vec![cancelled]);
        let (code, _) = local_changes(&runner, root.path(), &CancellationToken::new()).unwrap_err();
        assert_eq!(code, RefusalCode::Cancelled);

        let mut dubious = process(json!(null), false);
        dubious.stdout = String::new();
        dubious.stderr = "fatal: detected dubious ownership in repository".to_string();
        let runner = SequenceRunner::new(vec![dubious]);
        let (code, error) =
            local_changes(&runner, root.path(), &CancellationToken::new()).unwrap_err();
        assert_eq!(code, RefusalCode::InvalidState);
        assert!(error.contains("dubious ownership"), "{error}");

        let mut outside = process(json!(null), false);
        outside.stdout = String::new();
        outside.stderr =
            "fatal: not a git repository (or any of the parent directories): .git".to_string();
        let runner = SequenceRunner::new(vec![outside]);
        assert_eq!(
            local_changes(&runner, root.path(), &CancellationToken::new()).unwrap(),
            LocalChanges::NotUnderGit
        );
    }

    /// Ответ git, который не прочитать, — отказ, а не «правок нет».
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
    #[test]
    fn an_unreadable_git_answer_is_refused_rather_than_counted_as_clean() {
        let root = tempfile::tempdir().unwrap();
        let mut inside = process(json!(null), true);
        inside.stdout = "true
"
        .to_string();
        let mut failed = process(json!(null), false);
        failed.stdout = String::new();
        let mut not_ignored = process(json!(null), false);
        not_ignored.stdout = String::new();
        let runner = SequenceRunner::new(vec![inside, not_ignored, failed]);
        let (code, error) =
            local_changes(&runner, root.path(), &CancellationToken::new()).unwrap_err();
        assert_eq!(code, RefusalCode::InvalidState);
        assert!(error.contains("git status"), "{error}");
        let args: Vec<String> = runner.calls.lock().unwrap()[2].args.clone();
        assert!(
            args.contains(&"--untracked-files=all".to_string()),
            "{args:?}"
        );
    }

    /// Превью полной выгрузки называет несохранённые правки; частичная
    /// выгрузка набор не заменяет и их не считает.
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
    #[test]
    fn a_full_pull_preview_names_local_changes_and_a_partial_one_does_not() {
        let root = workspace();
        for (mode, runner_mode, expected) in [
            (ExportMode::Full, "FULL", true),
            (ExportMode::Incremental, "INCREMENTAL", false),
        ] {
            let runner = SequenceRunner::new(vec![process(
                envelope(root.path(), "main", runner_mode, None, false),
                true,
            )]);
            let result = run(
                root.path(),
                &prepared(root.path(), mode, Some("main"), None, true),
                &runner,
            );
            assert!(result.ok, "{result:?}");
            let local = &result.data.as_ref().unwrap()["plan"]["localChanges"];
            assert_eq!(!local.is_null(), expected, "{mode:?}: {local}");
        }
    }

    fn pull_request(root: &Path, arguments: Value) -> InvocationRequest {
        InvocationRequest::new(
            ToolIdentity::Run,
            arguments,
            root.display().to_string(),
            7000,
        )
        .unwrap()
    }

    /// Проект под git с набором `main`, сохранённым в коммите.
    fn committed_workspace() -> tempfile::TempDir {
        let root = workspace();
        let set = root.path().join("main");
        fs::create_dir_all(set.join("CommonModules")).unwrap();
        fs::write(set.join("Configuration.xml"), "configuration").unwrap();
        fs::write(set.join("CommonModules/A.bsl"), "a").unwrap();
        fs::write(set.join("CommonModules/B.bsl"), "b").unwrap();
        git(root.path(), &["init", "-q"]);
        git(root.path(), &["config", "user.email", "t@example.com"]);
        git(root.path(), &["config", "user.name", "test"]);
        git(root.path(), &["add", "-A"]);
        git(
            root.path(),
            &[
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=",
                "commit",
                "-qm",
                "init",
            ],
        );
        root
    }

    /// Исполняемый полный `pull` поверх несохранённых правок идёт только с
    /// согласия человека. Вопрос называет число правок по согласованному
    /// тексту, при отмене советует сохранить их в git. Согласие годится только
    /// для того же числа правок. Чистый набор и превью не спрашивают.
    /// rule: INV.RUNTIME.DESTRUCTIVE-CONFIRMATION
    /// rule: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
    /// rule: INV.MCP.HUMAN-CHOICE
    #[test]
    fn a_full_pull_over_unsaved_changes_runs_only_with_the_persons_confirmation() {
        let root = committed_workspace();
        let call =
            json!({"op": "pull", "args": {"force": true, "sourceSet": "main"}, "dryRun": false});

        // Чистый набор: вопроса нет.
        PreparedSourceExport::parse(&pull_request(root.path(), call.clone()))
            .expect("a clean source set is not asked");

        fs::write(root.path().join("main/CommonModules/A.bsl"), "edited").unwrap();
        let refused = PreparedSourceExport::parse(&pull_request(root.path(), call.clone()))
            .expect_err("unsaved changes need a confirmation");
        assert_eq!(refused.diagnostics[0]["code"], "cancelled");
        let choice = refused.human_choice.as_deref().expect("a confirmation");
        // Надписи согласованы: менять их — только вместе с этим тестом.
        assert_eq!(choice.code, "source.pull_unsaved");
        assert_eq!(
            choice.question,
            "В исходниках набора «main» есть несохранённые правки, файлов: 1."
        );
        assert!(choice.proceeds_unasked);
        assert_eq!(choice.options.len(), 1);
        assert_eq!(choice.options[0].title, "Перезаписать исходники");
        assert_eq!(choice.options[0].description, "правки пропадут, файлов: 1");
        assert_eq!(
            choice.cancel_description.as_deref(),
            Some("сохраните правки в git и повторите")
        );
        assert_eq!(
            choice.data,
            Some(json!({"sourceSet": "main", "underGit": true, "files": 1}))
        );
        assert_eq!(
            Value::Object(unica_execution_contract::invocation::without_confirmation(
                &choice.options[0].arguments
            )),
            call
        );

        let confirmed = choice.options[0].arguments.clone();
        let executed = PreparedSourceExport::parse(&pull_request(
            root.path(),
            Value::Object(confirmed.clone()),
        ))
        .expect("a confirmed full pull is executed");
        assert_eq!(executed.arguments.mode, ExportMode::Full);

        // Правок стало больше: прежнее согласие не годится.
        fs::write(root.path().join("main/CommonModules/New.bsl"), "new").unwrap();
        let again =
            PreparedSourceExport::parse(&pull_request(root.path(), Value::Object(confirmed)))
                .expect_err("another count needs a new confirmation");
        assert_eq!(
            again.human_choice.as_deref().unwrap().question,
            "В исходниках набора «main» есть несохранённые правки, файлов: 2."
        );

        // Без `sourceSet` считается набор конфигурации.
        let unnamed = PreparedSourceExport::parse(&pull_request(
            root.path(),
            json!({"op": "pull", "args": {"force": true}, "dryRun": false}),
        ))
        .expect_err("the configuration set is counted too");
        assert!(unnamed.human_choice.is_some());

        for unasked in [
            json!({"op": "pull", "args": {"force": true, "sourceSet": "main"}, "dryRun": true}),
            json!({"op": "pull", "args": {"sourceSet": "main"}, "dryRun": false}),
        ] {
            PreparedSourceExport::parse(&pull_request(root.path(), unasked.clone()))
                .unwrap_or_else(|refused| panic!("{unasked} is not asked: {refused:?}"));
        }
    }

    /// Набор не под git: правки проверить нельзя, и полный `pull` спрашивает
    /// по согласованному тексту.
    /// rule: INV.RUNTIME.DESTRUCTIVE-CONFIRMATION
    #[test]
    fn a_full_pull_of_a_set_outside_git_asks_the_person() {
        let root = workspace();
        let set = root.path().join("main");
        fs::create_dir_all(&set).unwrap();
        fs::write(set.join("Configuration.xml"), "configuration").unwrap();
        // Каталог временный, но может лежать внутри чужого репозитория.
        let outside = std::process::Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(&set)
            .output()
            .map(|output| !output.status.success())
            .unwrap_or(true);
        if !outside {
            return;
        }
        let refused = PreparedSourceExport::parse(&pull_request(
            root.path(),
            json!({"op": "pull", "args": {"force": true, "sourceSet": "main"}, "dryRun": false}),
        ))
        .expect_err("a set outside git needs a confirmation");
        let choice = refused.human_choice.as_deref().expect("a confirmation");
        assert_eq!(choice.code, "source.pull_unsaved");
        assert_eq!(
            choice.question,
            "Исходники набора «main» не под git: несохранённые правки проверить нельзя."
        );
        assert_eq!(choice.options[0].title, "Перезаписать исходники");
        assert_eq!(
            choice.options[0].description,
            "правки в файлах набора пропадут"
        );
        assert_eq!(choice.cancel_description, None);
        assert_eq!(
            choice.data,
            Some(json!({"sourceSet": "main", "underGit": false}))
        );
    }
}
