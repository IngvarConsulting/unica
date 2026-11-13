#![allow(clippy::result_large_err)]
// Internal failures are immediately returned as the canonical wire result; keeping
// that value intact avoids a second error model at this adapter boundary.

use super::protocol::InvocationRequest;
use super::v13_source_import::INFOBASE_NAME;
use crate::domain::engine::MissingEngineRefusal;
use crate::infrastructure::bundled_tools::{bundled_tool_version, resolve_bundled_tool};
use crate::infrastructure::plugin_runtime::find_plugin_root;
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use unica_artifact_delivery::engine::BundledTool;
use unica_artifact_delivery::engine::BundledToolError;
use unica_artifact_delivery::engine::MissingEngine;
use unica_execution::invocation_store::ToolIdentity;
use unica_execution_contract::invocation::{
    DomainResult, HumanChoice, HumanChoiceOption, SafeIdentityHash,
};
use unica_process::runner::{ProcessCommand, ProcessOutput, ProcessRunner, ProcessStreamFailure};
use unica_redaction::redactor;
use unica_refusal::{RefusalCode, RefusalDetail};
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::{CancellationToken, CANCELLED_PREFIX};
use unica_workspace::path_policy::WorkspacePathPolicy;
use unica_workspace::paths::normalize_path_identity;
use unica_workspace::read::open_directory_reader;

pub(super) const CONFIG_NAME: &str = "v8project.yaml";
pub(super) const LOCAL_CONFIG_NAME: &str = "v8project.local.yaml";
// resource-limit: run.runner-output-bytes
// class: retention
// unit: bytes
// scope: process
// value: 1048576
// resource: Захват stdout и stderr процесса v8-runner в операциях run.
// status: unproven
// origin: 8a85591e3 (#693)
// problem: неизвестно
// on-reach: stdout сверх предела — отказ invalid_result «v8-runner returned an unreadable or oversized JSON result»; кадр протокола публикации pull сверх предела не читается — отказ invalid_result с пределом и причиной; отказ после публикации сообщает, что исходники опубликованы, а заменённые сохранены в recovery; stderr тихо сохраняет хвост с пометкой об усечении.
pub(super) const RUNNER_OUTPUT_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportOperation {
    Configuration,
    Infobase,
    /// Обратная операция: DT из рабочего пространства загружается в базу.
    Restore,
}

impl ExportOperation {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "download" => Some(Self::Configuration),
            "infobase.dump" => Some(Self::Infobase),
            "infobase.restore" => Some(Self::Restore),
            _ => None,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Configuration => "download",
            Self::Infobase => "infobase.dump",
            Self::Restore => "infobase.restore",
        }
    }

    /// Имя команды в конверте раннера. Словарь Unica читается как «слой и
    /// направление», раннер называет свои команды по-своему; совпадение имён
    /// было случайным и держать его незачем.
    const fn runner_command(self) -> &'static str {
        match self {
            Self::Configuration => "download",
            Self::Infobase => "infobase.dump",
            Self::Restore => "infobase.restore",
        }
    }

    const fn artifact_kind(self, extension: Option<&str>) -> &'static str {
        match (self, extension) {
            (Self::Configuration, Some(_)) => "cfe",
            (Self::Configuration, None) => "cf",
            (Self::Infobase | Self::Restore, _) => "dt",
        }
    }

    /// Пишет ли операция файл рабочего пространства или читает его.
    ///
    /// Различие не косметическое: у выгрузки названный файл — результат, и его
    /// появление и есть улика; у загрузки он вход, и улику приходится искать в базе.
    const fn writes_named_file(self) -> bool {
        match self {
            Self::Configuration | Self::Infobase => true,
            Self::Restore => false,
        }
    }
}

/// Какое необратимое изменение базы разрешил вызывающий.
///
/// Раннер требует ровно один режим и отказывает, если названный не совпал с
/// наблюдаемой целью. Unica это не угадывает и не подставляет умолчание: выбор
/// принадлежит вызывающему, потому что цена ошибки — данные базы.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestoreMode {
    Create,
    Replace,
}

impl RestoreMode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "create" => Some(Self::Create),
            "replace" => Some(Self::Replace),
            _ => None,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Replace => "replace",
        }
    }

    const fn runner_flag(self) -> &'static str {
        match self {
            Self::Create => "--create",
            Self::Replace => "--replace",
        }
    }
}

#[derive(Debug, Clone)]
struct ExportArguments {
    state: Option<String>,
    extension: Option<String>,
    /// Файл рабочего пространства, который операция называет: выход у выгрузки,
    /// вход у загрузки. Его состояние обязано не меняться между превью и
    /// применением в обоих случаях, поэтому слот один.
    named_file_relative: PathBuf,
    named_file: PathBuf,
    /// Присутствует только у `infobase.restore`.
    restore_mode: Option<RestoreMode>,
}

#[derive(Debug, Clone)]
pub(super) struct PreparedInfobaseExport {
    operation: ExportOperation,
    arguments: ExportArguments,
    dry_run: bool,
    context: WorkspaceContext,
    /// The call as made: a question about the busy infobase repeats it.
    call: Map<String, Value>,
}

pub(super) enum Preparation {
    NotApplicable,
    Rejected(Box<DomainResult>),
    Ready(Arc<PreparedInfobaseExport>),
}

pub(super) fn prepare(request: &InvocationRequest) -> Preparation {
    if request.tool() != ToolIdentity::Run {
        return Preparation::NotApplicable;
    }
    let Some(op) = request.arguments().get("op").and_then(Value::as_str) else {
        return Preparation::NotApplicable;
    };
    let Some(operation) = ExportOperation::parse(op) else {
        return Preparation::NotApplicable;
    };
    match PreparedInfobaseExport::parse(request, operation) {
        Ok(prepared) => Preparation::Ready(Arc::new(prepared)),
        Err(result) => Preparation::Rejected(Box::new(result)),
    }
}

impl PreparedInfobaseExport {
    fn parse(
        request: &InvocationRequest,
        operation: ExportOperation,
    ) -> Result<Self, DomainResult> {
        let arguments = request.arguments();
        let args = arguments
            .get("args")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                reject(
                    operation,
                    RefusalCode::BadValue,
                    "run args must be an object",
                )
            })?;
        let dry_run = arguments
            .get("dryRun")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                reject(
                    operation,
                    RefusalCode::BadValue,
                    format!(
                        "{} requires dryRun: true to preview or dryRun: false to execute",
                        operation.name()
                    ),
                )
            })?;

        let context =
            discover_workspace(Some(PathBuf::from(request.workspace_hint()))).map_err(|error| {
                reject(
                    operation,
                    RefusalCode::ProviderUnavailable,
                    format!("workspace discovery failed: {error}"),
                )
            })?;
        let arguments = parse_export_arguments(operation, args, &context)?;
        // Замена данных базы исполняется только с согласия человека. Вопрос
        // задаётся до допуска: вызов, ушедший в фоновую задачу, спросить уже
        // не может. Превью базу не меняет и не спрашивает.
        if operation == ExportOperation::Restore
            && arguments.restore_mode == Some(RestoreMode::Replace)
            && !dry_run
        {
            let file = dt_file_name(&arguments.named_file_relative);
            if !HumanChoice::is_confirmed(
                &restore_replace_question(&file),
                "unica.run",
                request.arguments(),
            ) {
                return Err(restore_replace_unconfirmed(
                    &arguments.named_file_relative,
                    request.arguments(),
                ));
            }
        }
        Ok(Self {
            operation,
            arguments,
            dry_run,
            context,
            call: request.arguments().clone(),
        })
    }

    pub(super) fn workspace_identity_hash(&self) -> SafeIdentityHash {
        let mut hasher = Sha256::new();
        hasher.update(b"unica-v13-infobase-export-workspace-v1\0");
        hasher.update(self.context.workspace_root.as_os_str().as_encoded_bytes());
        SafeIdentityHash::from_sha256(hasher.finalize().into())
    }

    pub(super) fn execute(&self, cancellation: CancellationToken) -> DomainResult {
        let result = super::runner::with_bundled_runner(
            &self.context.workspace_root,
            &self.context.cwd,
            self.operation.name(),
            cancellation,
            |runner, cancellation| execute_with_runner(self, runner, cancellation),
        );
        super::v13_infobase_sessions::bind_to_call(result, &self.call)
    }
}

fn parse_export_arguments(
    operation: ExportOperation,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<ExportArguments, DomainResult> {
    let allowed: &[&str] = match operation {
        ExportOperation::Configuration => &["state", "output", "extension"],
        ExportOperation::Infobase => &["output"],
        // `connection` словарь не принимает: соединение задаёт `v8project.yaml`, и
        // подменить его аргументом вызова раннер не умеет. Принять ключ и
        // проигнорировать значило бы обещать то, чего нет.
        ExportOperation::Restore => &["input", "mode"],
    };
    if let Some(unknown) = args.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(reject(
            operation,
            RefusalCode::BadValue,
            format!("{} does not accept argument `{unknown}`", operation.name()),
        ));
    }
    let state = match operation {
        ExportOperation::Configuration => match args.get("state").and_then(Value::as_str) {
            Some(state @ ("working" | "database")) => Some(state.to_string()),
            _ => {
                return Err(reject(
                    operation,
                    RefusalCode::BadValue,
                    "download state must be `working` or `database`",
                ))
            }
        },
        ExportOperation::Infobase | ExportOperation::Restore => None,
    };
    let restore_mode = match operation {
        ExportOperation::Restore => {
            match args
                .get("mode")
                .and_then(Value::as_str)
                .and_then(RestoreMode::parse)
            {
                Some(mode) => Some(mode),
                None => {
                    return Err(reject(
                        operation,
                        RefusalCode::BadValue,
                        "infobase.restore mode must be `create` for an absent infobase or `replace` to discard the data of an existing one",
                    ))
                }
            }
        }
        _ => None,
    };
    let extension = match args.get("extension") {
        None => None,
        Some(Value::String(name)) if valid_1c_identifier(name) => Some(name.clone()),
        Some(_) => {
            return Err(reject(
                operation,
                RefusalCode::BadValue,
                "download extension must be a non-empty 1C identifier",
            ))
        }
    };
    // Имя аргумента зависит от направления: выгрузка называет выход, загрузка — вход.
    let named_key = if operation.writes_named_file() {
        "output"
    } else {
        "input"
    };
    let named = args
        .get(named_key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            reject(
                operation,
                RefusalCode::BadValue,
                format!("{} {named_key} must be non-empty text", operation.name()),
            )
        })?;
    let named_file_relative = closed_workspace_relative_path(named).map_err(|message| {
        reject(
            operation,
            RefusalCode::BadValue,
            format!("{} {named_key} {message}", operation.name()),
        )
    })?;
    let expected_suffix = operation.artifact_kind(extension.as_deref());
    if named_file_relative
        .extension()
        .and_then(|value| value.to_str())
        != Some(expected_suffix)
    {
        return Err(reject(
            operation,
            RefusalCode::BadValue,
            format!(
                "{} {named_key} must end in .{expected_suffix}{}",
                operation.name(),
                if operation == ExportOperation::Configuration {
                    if extension.is_some() {
                        " when extension is present"
                    } else {
                        " when extension is omitted"
                    }
                } else {
                    ""
                }
            ),
        ));
    }
    let root_context = WorkspaceContext {
        cwd: context.workspace_root.clone(),
        workspace_root: context.workspace_root.clone(),
        cache_root: context.cache_root.clone(),
        workspace_epoch: context.workspace_epoch,
    };
    // Политика одна и та же: путь обязан оставаться внутри пространства и не уходить
    // по ссылкам. Для входа это так же важно, как для выхода.
    let named_file = WorkspacePathPolicy::new(&root_context)
        .resolve_write(&named_file_relative)
        .map_err(|error| reject(operation, RefusalCode::BadValue, error))?;
    Ok(ExportArguments {
        state,
        extension,
        named_file_relative,
        named_file,
        restore_mode,
    })
}

pub(super) fn closed_workspace_relative_path(value: &str) -> Result<PathBuf, &'static str> {
    let path = Path::new(value);
    if path.is_absolute() {
        return Err("must be workspace-relative");
    }
    let mut closed = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => closed.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("must not contain parent traversal or a filesystem root")
            }
        }
    }
    if closed.as_os_str().is_empty() {
        Err("must name a file")
    } else {
        Ok(closed)
    }
}

pub(super) fn valid_1c_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_alphabetic())
        && chars.all(|character| character == '_' || character.is_alphanumeric())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StableInputs {
    config_sha256: String,
    local_config_sha256: Option<String>,
    /// Состояние названного файла: выхода у выгрузки, входа у загрузки.
    named_file_sha256: Option<String>,
    named_file_size: Option<u64>,
}

fn capture_inputs(prepared: &PreparedInfobaseExport) -> Result<StableInputs, DomainResult> {
    let root = &prepared.context.workspace_root;
    let config_sha256 =
        digest_required_workspace_file(root, Path::new(CONFIG_NAME)).map_err(|error| {
            let mut result = reject(prepared.operation, RefusalCode::InvalidState, error);
            result.next.push(json!({
                "tool": "unica.view",
                "args": {},
                "reason": "inspect workspace setup and the required v8project.yaml recipe"
            }));
            result
        })?;
    let local_config_sha256 = digest_optional_workspace_file(root, Path::new(LOCAL_CONFIG_NAME))
        .map_err(|error| reject(prepared.operation, RefusalCode::InvalidState, error))?
        .map(|(digest, _)| digest);
    let named = digest_optional_workspace_file(root, &prepared.arguments.named_file_relative)
        .map_err(|error| reject(prepared.operation, RefusalCode::BadValue, error))?;
    Ok(StableInputs {
        config_sha256,
        local_config_sha256,
        named_file_sha256: named.as_ref().map(|(digest, _)| digest.clone()),
        named_file_size: named.map(|(_, size)| size),
    })
}

/// Набор основной конфигурации, объявленный в `v8project.yaml`, или `None`,
/// если такого набора нет.
///
/// Раннер 0.14 без имени набора у `make` и `download` обходит все наборы и
/// пишет пакеты в каталог; путь к одному файлу без набора он отклоняет. Пакет
/// основной конфигурации поэтому заказывается по имени её набора. Тип набора
/// сравнивается без учёта регистра: Unica принимает и прежнее написание
/// `configuration`.
pub(super) fn configuration_source_set(root: &Path) -> Result<Option<String>, String> {
    let text = std::fs::read_to_string(root.join(CONFIG_NAME))
        .map_err(|error| format!("{CONFIG_NAME}: {error}"))?;
    let document: serde_yaml::Value = serde_yaml::from_str(&text).map_err(|error| {
        format!(
            "{CONFIG_NAME} is not valid YAML: {}",
            redactor(&error.to_string())
        )
    })?;
    let mut names = document
        .get("source-set")
        .and_then(serde_yaml::Value::as_sequence)
        .into_iter()
        .flatten()
        .filter(|set| {
            set.get("type")
                .and_then(serde_yaml::Value::as_str)
                .is_some_and(|kind| kind.eq_ignore_ascii_case("configuration"))
        })
        .map(|set| {
            set.get("name")
                .and_then(serde_yaml::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| {
                    format!("{CONFIG_NAME} declares a configuration source-set without a name")
                })
        });
    let first = names.next().transpose()?;
    if names.next().is_some() {
        return Err(format!(
            "{CONFIG_NAME} declares more than one configuration source-set; the main configuration must have exactly one"
        ));
    }
    Ok(first)
}

/// Объявлен ли в `v8project.yaml` хоть один набор конфигурации или расширения.
fn declares_package_source_sets(root: &Path) -> bool {
    std::fs::read_to_string(root.join(CONFIG_NAME))
        .ok()
        .and_then(|text| serde_yaml::from_str::<serde_yaml::Value>(&text).ok())
        .and_then(|document| document.get("source-set").cloned())
        .and_then(|sets| sets.as_sequence().cloned())
        .is_some_and(|sets| {
            sets.iter().any(|set| {
                set.get("type")
                    .and_then(serde_yaml::Value::as_str)
                    .is_some_and(|kind| {
                        kind.eq_ignore_ascii_case("configuration")
                            || kind.eq_ignore_ascii_case("extension")
                    })
            })
        })
}

pub(super) fn digest_required_workspace_file(
    root: &Path,
    relative: &Path,
) -> Result<String, String> {
    digest_optional_workspace_file(root, relative)?
        .map(|(digest, _)| digest)
        .ok_or_else(|| {
            format!(
                "{} is missing; call unica.view with no arguments for the shortest setup recipe",
                relative
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("required file")
            )
        })
}

pub(super) fn digest_optional_workspace_file(
    root: &Path,
    relative: &Path,
) -> Result<Option<(String, u64)>, String> {
    let root = normalize_path_identity(root).map_err(|error| {
        format!(
            "failed to resolve workspace root {}: {error}",
            root.display()
        )
    })?;
    let display = root.join(relative);
    let mut components = relative.components().peekable();
    let mut directory = open_directory_reader(&root)
        .map_err(|error| format!("failed to open workspace root {}: {error}", root.display()))?;
    let mut file = None;
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(format!(
                "{} is not a closed workspace path",
                display.display()
            ));
        };
        let opened = if components.peek().is_none() {
            directory.open_regular_child(name)
        } else {
            match directory.open_directory_child(name) {
                Ok(child) => {
                    directory = child;
                    continue;
                }
                Err(error) => Err(error),
            }
        };
        match opened {
            Ok(opened) => file = Some(opened),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "{} must be a regular file reached without links: {error}",
                    display.display()
                ))
            }
        }
    }
    let mut file = file.ok_or_else(|| format!("{} must name a file", display.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut size = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to read {}: {error}", display.display()))?;
        if read == 0 {
            break;
        }
        size = size.saturating_add(read as u64);
        hasher.update(&buffer[..read]);
    }
    Ok(Some((format!("{:x}", hasher.finalize()), size)))
}

fn execute_with_runner(
    prepared: &PreparedInfobaseExport,
    runner: &dyn ProcessRunner,
    cancellation: CancellationToken,
) -> DomainResult {
    let runner_tool = match resolve_bundled_runner(&prepared.context.cwd) {
        Ok(resolved) => resolved,
        Err(message) => {
            return missing_runner_rejection(Some(prepared.operation.name().to_string()), message)
        }
    };
    let tool = runner_tool.tool;
    let runner_version = runner_tool.version;
    execute_with_resolved_runner(prepared, runner, cancellation, &tool, &runner_version)
}

fn execute_with_resolved_runner(
    prepared: &PreparedInfobaseExport,
    runner: &dyn ProcessRunner,
    cancellation: CancellationToken,
    tool: &BundledTool,
    _runner_version: &str,
) -> DomainResult {
    if cancellation.is_cancelled() {
        return reject(
            prepared.operation,
            RefusalCode::Cancelled,
            format!("{} cancelled before preflight", prepared.operation.name()),
        );
    }
    let before = match capture_inputs(prepared) {
        Ok(inputs) => inputs,
        Err(result) => return result,
    };
    if prepared.dry_run {
        let preview = match invoke_runner(prepared, tool, runner, &cancellation, true) {
            Ok(envelope) => envelope,
            Err(result) => return result,
        };
        let plan = match validate_preview(prepared, &preview) {
            Ok(plan) => plan,
            Err(result) => return result,
        };
        let after = match capture_inputs(prepared) {
            Ok(inputs) => inputs,
            Err(result) => return result,
        };
        if before != after {
            return reject(
                prepared.operation,
                RefusalCode::ConcurrentChange,
                format!(
                    "{} inputs changed during preview; run dryRun: true again",
                    prepared.operation.name()
                ),
            );
        }

        let mut result = DomainResult::success(format!(
            "{} planned without changing the infobase or workspace files",
            prepared.operation.name()
        ));
        result.data = Some(json!({
            "op": prepared.operation.name(),
            "dryRun": true,
            "plan": public_plan(prepared, &plan),
            "requiresPlatform": true,
        }));

        result.next.push(json!({
            "tool": "unica.run",
            "args": {
                "op": prepared.operation.name(),
                "args": public_arguments(prepared),
                "dryRun": false,
            },
            "reason": "execute with the current arguments plan"
        }));
        note_runner_warnings(&mut result, &preview);
        return result;
    }
    if cancellation.is_cancelled() {
        return reject(
            prepared.operation,
            RefusalCode::Cancelled,
            format!(
                "{} cancelled before provider launch",
                prepared.operation.name()
            ),
        );
    }
    if prepared.operation == ExportOperation::Restore
        && super::v13_infobase_another_copy::needs_preview(
            &prepared.context.workspace_root,
            &prepared.call,
        )
    {
        if let Ok(preview) = invoke_runner(prepared, tool, runner, &cancellation, true) {
            if let Some(refusal) = super::v13_infobase_another_copy::refusal_for(
                prepared.operation.name(),
                &preview,
                &prepared.call,
            ) {
                return refusal;
            }
        }
    }
    let applied = match invoke_runner(prepared, tool, runner, &cancellation, false) {
        Ok(envelope) => envelope,
        Err(result) => return result,
    };
    // Исполнение не повторяет preview раннера: исполнитель и его квитанция
    // берутся из ответа исполнения.
    let provider = match validate_apply(prepared, &applied) {
        Ok(provider) => provider,
        Err(result) => return result,
    };
    let (sha256, size) = match digest_optional_workspace_file(
        &prepared.context.workspace_root,
        &prepared.arguments.named_file_relative,
    ) {
        Ok(Some(receipt)) if receipt.1 > 0 => receipt,
        Ok(Some(_)) => {
            return reject(
                prepared.operation,
                RefusalCode::InvalidResult,
                if prepared.operation.writes_named_file() {
                    "v8-runner reported publication but the exported file is empty"
                } else {
                    "the restored DT is now empty; the provider must not alter its source"
                },
            )
        }
        Ok(None) => {
            return reject(
                prepared.operation,
                RefusalCode::InvalidResult,
                if prepared.operation.writes_named_file() {
                    "v8-runner reported publication but the exported file is missing"
                } else {
                    "the restored DT disappeared; the provider must not consume its source"
                },
            )
        }
        Err(error) => return reject(prepared.operation, RefusalCode::InvalidResult, error),
    };
    // У загрузки названный файл — вход, и он обязан дойти до конца неизменным: иначе
    // провайдер тронул источник, а не только базу.
    if !prepared.operation.writes_named_file()
        && before.named_file_sha256.as_deref() != Some(sha256.as_str())
    {
        return reject(
            prepared.operation,
            RefusalCode::InvalidResult,
            "the restored DT changed during the operation; its source must stay intact",
        );
    }
    if !prepared.operation.writes_named_file() {
        // **Улику о состоянии базы Unica не подделывает.** Файл проверить можно, базу
        // без платформы — нет: она живёт за соединением, и её состояние здесь
        // засвидетельствовано провайдером, а не проверено нами. Поэтому источник
        // признания назван прямо, а не спрятан за словом «проверено».
        let mut result = DomainResult::success(format!(
            "{} loaded the infobase from the named DT; its state is attested by the provider",
            prepared.operation.name()
        ));
        result.data = Some(json!({
            "op": prepared.operation.name(),
            "dryRun": false,
            "provider": provider,
            "source": {
                "kind": prepared.operation.artifact_kind(None),
                "path": path_text(&prepared.arguments.named_file_relative),
                "size": size,
                "sha256": sha256,
            },
            "mode": prepared
                .arguments
                .restore_mode
                .map(RestoreMode::as_str),
            "targetState": applied["data"]["target_state"],
            "targetStateAttestedBy": "provider",
        }));
        // Изменилась база, а не файл рабочего пространства: путь сюда не кладётся,
        // иначе запись читалась бы как «переписали DT».
        result.changed.push(json!({
            "infobase": true,
            "kind": applied["data"]["target_state"],
        }));
        note_runner_warnings(&mut result, &applied);

        return result;
    }
    let mut result = DomainResult::success(format!(
        "{} exported and independently verified",
        prepared.operation.name()
    ));
    result.data = Some(json!({
        "op": prepared.operation.name(),
        "dryRun": false,
        "provider": provider,
        "artifact": {
            "kind": prepared.operation.artifact_kind(prepared.arguments.extension.as_deref()),
            "path": path_text(&prepared.arguments.named_file_relative),
            "size": size,
            "sha256": sha256,
        },
        "targetState": applied["data"]["target_state"],
    }));
    result.changed.push(json!({
        "path": path_text(&prepared.arguments.named_file_relative),
        "kind": applied["data"]["target_state"],
    }));
    result.artifacts.push(json!({
        "kind": prepared.operation.artifact_kind(prepared.arguments.extension.as_deref()),
        "path": path_text(&prepared.arguments.named_file_relative),
        "size": size,
        "sha256": sha256,
    }));

    result
}

fn invoke_runner(
    prepared: &PreparedInfobaseExport,
    tool: &BundledTool,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
    dry_run: bool,
) -> Result<Value, DomainResult> {
    if cancellation.is_cancelled() {
        return Err(reject(
            prepared.operation,
            RefusalCode::Cancelled,
            "cancelled before provider launch",
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
    ];
    match prepared.operation {
        ExportOperation::Configuration => {
            args.push("download".to_string());
            // Основную конфигурацию раннер 0.14 пишет в один файл только по имени
            // её набора: без набора он обходит все наборы в каталог. Без имени
            // он пишет файл, только если в проекте нет ни одного набора
            // конфигурации или расширения.
            if prepared.arguments.extension.is_none() {
                let root = &prepared.context.workspace_root;
                let source_set = configuration_source_set(root).map_err(|error| {
                    reject(prepared.operation, RefusalCode::InvalidState, error)
                })?;
                if source_set.is_none() && declares_package_source_sets(root) {
                    return Err(reject(
                        prepared.operation,
                        RefusalCode::UnsupportedOperation,
                        format!("download of the main configuration is unavailable with the pinned v8-runner in a project that declares extension source sets but no configuration source set: the runner writes the main configuration into one file only by the name of its configuration source set; declare one in {CONFIG_NAME}"),
                    ));
                }
                args.extend(source_set);
            }
            // Рабочее состояние раннер 0.12 берёт без ключа (`--state working` у него
            // скрытый синоним), конфигурацию базы данных — по `--state db`.
            match prepared.arguments.state.as_deref() {
                Some("working") => {}
                Some("database") => args.extend(["--state".to_string(), "db".to_string()]),
                _ => unreachable!("download state is validated as working or database"),
            }
            if let Some(extension) = &prepared.arguments.extension {
                args.extend(["--extension".to_string(), extension.clone()]);
            }
        }
        ExportOperation::Infobase => args.extend(["infobase".to_string(), "dump".to_string()]),
        ExportOperation::Restore => {
            args.extend(["infobase".to_string(), "restore".to_string()]);
        }
    }
    if prepared.operation.writes_named_file() {
        args.extend([
            "--output".to_string(),
            prepared.arguments.named_file.display().to_string(),
        ]);
    } else {
        args.extend([
            "--input".to_string(),
            prepared.arguments.named_file.display().to_string(),
        ]);
        // Режим идёт отдельным флагом, потому что раннер требует ровно один и
        // сверяет его с наблюдаемой целью. Умолчания здесь нет ни у него, ни у нас.
        args.push(
            prepared
                .arguments
                .restore_mode
                .expect("restore mode")
                .runner_flag()
                .to_string(),
        );
    }
    if dry_run {
        args.push("--dry-run".to_string());
    }
    let output = runner.run_staged(&ProcessCommand {
        program: tool.program.clone(),
        args,
        cwd: prepared.context.workspace_root.clone(),
        env: Vec::new(),
        env_remove: Vec::new(),
        capture_limits: Some((RUNNER_OUTPUT_LIMIT, RUNNER_OUTPUT_LIMIT)),
        timeout: None,
        cancellation: if !dry_run && prepared.operation == ExportOperation::Restore {
            cancellation.protect_process_on_spawn()
        } else {
            cancellation.clone()
        },
    });
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            return Err(runner_start_rejection(
                Some(prepared.operation.name().to_string()),
                &error,
                &tool.program,
            ));
        }
    };
    // `mode: create` над базой, которая уже есть: выбрать замену её данных
    // может только человек. Спрашивает только исполнение: превью отдаёт
    // прежний отказ.
    let target_exists = !dry_run
        && prepared.operation == ExportOperation::Restore
        && prepared.arguments.restore_mode == Some(RestoreMode::Create)
        && !output.status_success
        && restore_target_exists(&output.stdout);
    parse_runner_output(prepared.operation, output, dry_run).map_err(|mut rejection| {
        if target_exists {
            rejection.human_choice = Some(Box::new(restore_exists_choice(prepared)));
        }
        rejection
    })
}

/// Код вопроса о замене данных базы: по нему клиент рисует свой экран.
const RESTORE_REPLACE_CODE: &str = "infobase.restore_replace";
/// Код вопроса о загрузке с `mode: create` в базу, которая уже есть.
const RESTORE_EXISTS_CODE: &str = "infobase.restore_exists";

/// Устойчивая часть отказа раннера 0.15 на `infobase restore --create`, когда
/// база уже есть. Кода у этого отказа нет: это ошибка проверки
/// `invalid_argument`, и опознаётся она по этой части текста, как
/// предупреждение о базе другой копии. Тест на конверте раннера упадёт, если
/// раннер сменит прозу.
const RESTORE_TARGET_EXISTS_FRAGMENT: &str =
    "--create was requested but the target infobase already exists";

fn restore_target_exists(stdout: &str) -> bool {
    serde_json::from_str::<Value>(stdout).is_ok_and(|envelope| {
        envelope["error"]["code"] == "invalid_argument"
            && envelope["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains(RESTORE_TARGET_EXISTS_FRAGMENT))
    })
}

/// Имя DT без пути: в тексте вопроса путей проекта нет.
fn dt_file_name(relative: &Path) -> String {
    relative
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path_text(relative))
}

/// Факты вопроса для экрана клиента: база, имя DT и его путь от корня проекта.
fn restore_facts(relative: &Path) -> Value {
    json!({
        "infobase": INFOBASE_NAME,
        "file": dt_file_name(relative),
        "input": path_text(relative),
    })
}

/// Согласованный текст вопроса о замене данных базы.
fn restore_replace_question(file: &str) -> String {
    format!("Заменить данные базы «{INFOBASE_NAME}» данными из «{file}»?")
}

/// Подтверждение замены данных базы этим же вызовом: надпись варианта и то,
/// что пропадёт, согласованы и закреплены тестом.
fn restore_replace_confirmation(relative: &Path, call: &Map<String, Value>) -> HumanChoice {
    HumanChoice::confirmation(
        RESTORE_REPLACE_CODE,
        restore_replace_question(&dt_file_name(relative)),
        "Заменить данные",
        "данные и конфигурация базы пропадут",
        "unica.run",
        call,
    )
    .with_data(restore_facts(relative))
}

/// Отказ `cancelled` без согласия человека: база не тронута. Клиенту, который
/// умеет спрашивать, дверь задаёт вопрос; остальным исполняет вызов как прежде.
fn restore_replace_unconfirmed(relative: &Path, call: &Map<String, Value>) -> DomainResult {
    let mut rejection = reject(
        ExportOperation::Restore,
        RefusalCode::Cancelled,
        "infobase.restore with mode: replace discards the data of the infobase and runs only with the person's confirmation; it was not given, and the infobase is unchanged",
    );
    rejection.human_choice = Some(Box::new(restore_replace_confirmation(relative, call)));
    rejection
}

/// Вопрос исполняемой загрузки с `mode: create` в базу, которая уже есть.
/// Вариант — тот же вызов с `mode: replace`, и он уже несёт согласие на
/// замену данных: второй вопрос тот же вызов не задаёт.
fn restore_exists_choice(prepared: &PreparedInfobaseExport) -> HumanChoice {
    let relative = &prepared.arguments.named_file_relative;
    let mut args = public_arguments(prepared);
    args["mode"] = json!(RestoreMode::Replace.as_str());
    let mut call = Map::new();
    call.insert("op".to_string(), json!(prepared.operation.name()));
    call.insert("args".to_string(), args);
    call.insert("dryRun".to_string(), Value::Bool(false));
    let arguments = restore_replace_confirmation(relative, &call)
        .options
        .remove(0)
        .arguments;
    HumanChoice {
        code: RESTORE_EXISTS_CODE.to_string(),
        question: format!("База «{INFOBASE_NAME}» уже существует."),
        data: Some(restore_facts(relative)),
        options: vec![HumanChoiceOption {
            id: "replace".to_string(),
            title: format!(
                "Заменить данные базы данными из «{}»",
                dt_file_name(relative)
            ),
            description: "данные и конфигурация базы пропадут".to_string(),
            tool: "unica.run".to_string(),
            arguments,
            then: Vec::new(),
        }],
        cancel_description: None,
        proceeds_unasked: false,
    }
}

fn parse_runner_output(
    operation: ExportOperation,
    output: ProcessOutput,
    dry_run: bool,
) -> Result<Value, DomainResult> {
    if output.cancelled {
        return Err(reject(
            operation,
            RefusalCode::Cancelled,
            "v8-runner was cancelled",
        ));
    }
    if output.timed_out {
        return Err(reject(
            operation,
            RefusalCode::DeadlineExceeded,
            "v8-runner exceeded its execution deadline",
        ));
    }
    if output.stdout_truncated || output.stdout_had_invalid_utf8 {
        return Err(reject(
            operation,
            RefusalCode::InvalidResult,
            "v8-runner returned an unreadable or oversized JSON result",
        ));
    }
    let envelope: Value = serde_json::from_str(&output.stdout).map_err(|_| {
        reject(
            operation,
            RefusalCode::InvalidResult,
            "v8-runner returned an invalid JSON result",
        )
    })?;
    if envelope["command"] != operation.runner_command() {
        return Err(reject(
            operation,
            RefusalCode::InvalidResult,
            "v8-runner returned a result for a different operation",
        ));
    }
    if !output.status_success {
        if dry_run
            && (envelope["data"]["mode"] != "preview"
                || envelope["data"]["provider_dispatched"] != false)
        {
            return Err(reject(
                operation,
                RefusalCode::InvalidResult,
                "failed v8-runner preview did not prove that no provider was dispatched",
            ));
        }
        let code = envelope["error"]["code"]
            .as_str()
            .unwrap_or("provider_failed");
        let message = envelope["error"]["message"]
            .as_str()
            .map(redactor)
            .unwrap_or_else(|| "v8-runner failed without a typed message".to_string());
        return Err(runner_envelope_refusal(
            runner_rejection(
                Some(operation.name().to_string()),
                code,
                envelope["error"]["layer"].as_str(),
                message,
            ),
            &envelope,
        ));
    }
    Ok(envelope)
}

/// Расхождение памяти рабочей копии о базе с самой базой: база ушла вперёд
/// записанного поколения или памяти о ней нет. Выбирать между выгрузкой базы
/// и её перезаписью должен человек. Имена кодов раннера живут только здесь:
/// с конверта 5 (раннер 0.17) отказ без памяти о базе называется
/// `untracked_base`, как ветка без слежения в git.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BaseDivergence {
    /// `non_fast_forward`: база ушла вперёд записанного поколения.
    MovedAhead,
    /// `untracked_base`: памяти рабочей копии о базе нет.
    Untracked,
}

impl BaseDivergence {
    const MOVED_AHEAD: &'static str = "non_fast_forward";
    const UNTRACKED: &'static str = "untracked_base";

    pub(super) fn from_runner_code(code: &str) -> Option<Self> {
        match code {
            Self::MOVED_AHEAD => Some(Self::MovedAhead),
            Self::UNTRACKED => Some(Self::Untracked),
            _ => None,
        }
    }

    pub(super) fn runner_code(self) -> &'static str {
        match self {
            Self::MovedAhead => Self::MOVED_AHEAD,
            Self::Untracked => Self::UNTRACKED,
        }
    }
}

/// Полный словарь кодов, которые раннер кладёт в `error.code`.
///
/// Он закрытый: двадцать пять значений `ErrorCode` конверта раннера 0.17, одни и те
/// же у CLI и у его MCP-поверхности. Набор существует только для стража ниже: в
/// продуктовом пути отображение обязано иметь запасную ветку на случай кода, которого
/// мы ещё не знаем, поэтому сам список ему не нужен.
#[cfg(test)]
const RUNNER_WIRE_CODES: [&str; 25] = [
    "capability_unavailable",
    "subject",
    "target",
    "soon",
    "cancelled",
    "environment_unavailable",
    "invalid_argument",
    "unsupported_value",
    "invalid_output",
    "platform_failure",
    "runtime_failure",
    "timed_out",
    "workspace_busy",
    "infobase_busy",
    "infobase_sessions",
    "infobase_designer_busy",
    "agent_command_abandoned",
    "agent_limit",
    "live_agent_off",
    "live_agent_job",
    "source_set_ignored_by_git",
    "work_path_on_another_volume",
    PLATFORM_RELEASE_MISSING,
    BaseDivergence::MOVED_AHEAD,
    BaseDivergence::UNTRACKED,
];

/// Отображает код раннера в наш отказ, а значит и в исход для агента.
///
/// **Платформа отказала — это «нужен человек», и дальше мы не разбираем.** Отказ
/// авторизации пакетного конфигуратора, отсутствие права и отсутствие лицензии
/// приходят от платформы одним `platform_failure`, и различать их Unica не должна:
/// ни один из трёх не решается ни повтором, ни правкой вызова — нужен человек.
/// Поэтому прав мы заранее не проверяем: факта отказа платформы достаточно.
///
/// `environment_unavailable` раннер отдаёт за любой отказ среды: агент
/// конфигуратора не принял пользователя ИБ, точка входа агента не ответила,
/// платформа или агент не стартовали. Слой раннер 0.17.2 называет полем
/// `error.layer` (`layer`); поля нет у отказа неопределённого слоя, и тогда
/// это среда раннера: «поставщик не готов», а не отказ учётных данных (#1333).
/// Раннер без поля слоя опознаётся по устойчивой части его текста
/// ([`RunnerLayer::of_environment_message`]).
pub(super) fn map_runner_detail(
    code: &str,
    layer: Option<&str>,
    message: &str,
) -> Option<RunnerLayer> {
    match code {
        "environment_unavailable" => Some(match layer {
            Some(layer) => RunnerLayer::of_runner_layer(layer),
            None => RunnerLayer::of_environment_message(message),
        }),
        _ => None,
    }
}

/// Слой, который отверг вызов раннера.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RunnerLayer {
    /// Агент конфигуратора не принял учётные данные пользователя ИБ: у агента
    /// это та же пара, что у пакетного конфигуратора.
    InfobaseUser,
    /// Точка входа агента не ответила или назвалась чужим ключом хоста.
    AgentEndpoint,
    /// Управляемый агент не запустился или его хранитель недоступен.
    AgentStart,
    /// Прочая среда раннера: платформа, порт, слой не определён.
    RunnerEnvironment,
}

/// Устойчивые части текста `AgentError` раннера 0.14 — по одной на слой.
///
/// Запасной путь на один цикл выпуска для раннера без поля `error.layer`
/// (до 0.17.2, IngvarConsulting/v8-runner-rust#485). Удалить вместе с
/// [`RunnerLayer::of_environment_message`] в следующем цикле.
const RUNNER_CREDENTIALS_REJECTED: &str = " rejected the credentials of user ";
const RUNNER_ENDPOINT_FAILURES: [&str; 5] = [
    " is unreachable: ",
    "SSH handshake with the agent at ",
    " as its host key, but ",
    // Another SSH server on the port of the managed agent.
    ": another SSH server holds the port",
    " did not open a shell channel: ",
];

impl RunnerLayer {
    /// Слой по полю `error.layer` раннера. Значение вне закрытого набора —
    /// среда раннера, как и отказ без поля.
    fn of_runner_layer(layer: &str) -> Self {
        match layer {
            "infobase_user" => Self::InfobaseUser,
            "agent_endpoint" => Self::AgentEndpoint,
            "agent_start" => Self::AgentStart,
            _ => Self::RunnerEnvironment,
        }
    }

    /// Слой по тексту раннера без поля `error.layer`. Удалить в следующем
    /// цикле выпуска вместе с маркерами.
    fn of_environment_message(message: &str) -> Self {
        if message.contains(RUNNER_CREDENTIALS_REJECTED) {
            Self::InfobaseUser
        } else if RUNNER_ENDPOINT_FAILURES
            .iter()
            .any(|marker| message.contains(marker))
        {
            Self::AgentEndpoint
        } else {
            Self::RunnerEnvironment
        }
    }

    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::InfobaseUser => "infobase_user",
            Self::AgentEndpoint => "agent_endpoint",
            Self::AgentStart | Self::RunnerEnvironment => "runner_environment",
        }
    }

    pub(super) const fn detail(self) -> RefusalDetail {
        match self {
            Self::InfobaseUser => RefusalDetail::CredentialsRejected,
            Self::AgentEndpoint => RefusalDetail::EndpointUnreachable,
            Self::AgentStart | Self::RunnerEnvironment => RefusalDetail::ProviderAbsent,
        }
    }

    /// Шаг проверки или исправления настройки для этого слоя.
    const fn advice(self) -> &'static str {
        match self {
            Self::InfobaseUser => {
                "the designer agent did not accept the infobase user or password, or found no 1C license: check the infobase user and password in the project's infobase connection settings (an infobase without users takes an empty user) and that a 1C license is available on this machine"
            }
            Self::AgentStart => {
                "the designer agent did not start or its keeper is unavailable: check that the 1C platform can start its designer agent on this machine"
            }
            Self::AgentEndpoint => {
                "the designer agent endpoint did not answer or presented another host key: check that the 1C platform can start its designer agent on this machine and that local ports are not blocked"
            }
            Self::RunnerEnvironment => {
                "the runner environment is not ready: check the installed 1C platform version and the project's platform settings"
            }
        }
    }
}

/// Поставляемый раннер, разрешённый для этого рабочего пространства.
pub(super) struct BundledRunner {
    pub(super) tool: BundledTool,
    pub(super) version: String,
}

/// Почему поставляемого раннера нет.
pub(super) enum RunnerAbsence {
    /// Движка нет на одном из шагов поставки: общий отказ #549.
    Missing(Box<MissingEngine>),
    /// Раннер на месте, но его не прочитать или его версию адаптер не
    /// принимает.
    Unusable(String),
}

impl From<String> for RunnerAbsence {
    fn from(message: String) -> Self {
        Self::Unusable(message)
    }
}

/// Разрешение поставляемого раннера. Причины неудачи — корня плагина нет,
/// движок не разрешается на одном из шагов поставки, версия не читается —
/// значат для вызывающего одно: поставщика нет. Маршрут один на все семь
/// операций `run`, и починка одного из них не выдаётся за починку всех.
pub(super) fn resolve_bundled_runner(cwd: &Path) -> Result<BundledRunner, RunnerAbsence> {
    let Some(plugin_root) = find_plugin_root(cwd) else {
        return Err(RunnerAbsence::Missing(Box::new(
            crate::infrastructure::bundled_tools::plugin_root_missing(RUNNER_TOOL),
        )));
    };
    let tool =
        resolve_bundled_tool(&plugin_root, RUNNER_TOOL, true).map_err(|error| match error {
            BundledToolError::Missing(engine) => RunnerAbsence::Missing(engine),
            BundledToolError::Io(message) => RunnerAbsence::Unusable(redactor(&message)),
        })?;
    let version =
        bundled_tool_version(&plugin_root, RUNNER_TOOL).map_err(|error| redactor(&error))?;
    super::runner::check_version(&version)?;
    Ok(BundledRunner { tool, version })
}

/// Имя поставляемого раннера в поставке.
const RUNNER_TOOL: &str = "v8-runner";

/// Отказ «поставляемого раннера нет». Отсутствие движка отвечает общим отказом
/// «нет поставляемого движка» с полями в `data`; негодная версия — тем же
/// уточнением `provider_absent` с текстом причины.
pub(super) fn missing_runner_rejection(at: Option<String>, absence: RunnerAbsence) -> DomainResult {
    match absence {
        RunnerAbsence::Missing(engine) => engine.refusal(at),
        RunnerAbsence::Unusable(message) => {
            DomainResult::canonical_rejection_detailed(at, RefusalDetail::ProviderAbsent, message)
        }
    }
}

/// Отказ, когда раннер не дал ответа: отмена, раннер не стартовал или сбой
/// после старта.
///
/// Маршрут один на все операции `run`. Отказом «нет движка» становится
/// только то, что тип называет запуском: процесс раннера не возник. Раннер к
/// этому месту уже прошёл полную проверку поставки, поэтому это шаг `start`.
/// Сбой после старта движок не теряет и отвечает своим кодом.
/// Исходная причина в обоих случаях остаётся в тексте отказа: путь к раннеру
/// из неё убран, секреты закрыты общей редакцией.
pub(super) fn runner_start_rejection(
    at: Option<String>,
    failure: &ProcessStreamFailure,
    program: &Path,
) -> DomainResult {
    let error = failure.message();
    if error.starts_with(CANCELLED_PREFIX) {
        return DomainResult::canonical_rejection(
            at,
            RefusalCode::Cancelled,
            "cancelled before provider launch",
        );
    }
    let cause = runner_failure_cause(error, program);
    match failure {
        ProcessStreamFailure::Spawn(_) => crate::infrastructure::bundled_tools::start_failure(
            find_plugin_root(program).as_deref(),
            RUNNER_TOOL,
            program,
        )
        .refusal_with_cause(at, &cause),
        ProcessStreamFailure::Run(_) => DomainResult::canonical_rejection(
            at,
            RefusalCode::ProviderFailed,
            format!("bundled v8-runner failed before returning its result: {cause}"),
        ),
    }
}

/// Причина сбоя раннера для ответа: без пути к самому раннеру и после общей
/// редакции секретов.
fn runner_failure_cause(error: &str, program: &Path) -> String {
    crate::infrastructure::bundled_tools::launch_failure_cause(error, RUNNER_TOOL, program)
}

/// Отказ по коду раннера: с уточнением, когда словарь его знает, иначе по коду.
///
/// Отказ среды называет слой в `data.layer` вместе с кодом раннера и шаг
/// исправления настройки; текст раннера остаётся уликой после него.
///
/// `layer` — поле `error.layer` конверта раннера, если оно есть.
pub(super) fn runner_rejection(
    at: Option<String>,
    code: &str,
    layer: Option<&str>,
    message: impl Into<String>,
) -> DomainResult {
    let message = message.into();
    match map_runner_detail(code, layer, &message) {
        Some(layer) => {
            let mut result = DomainResult::canonical_rejection_detailed(
                at,
                layer.detail(),
                format!("{}; v8-runner said: {message}", layer.advice()),
            );
            result.data = Some(json!({"layer": layer.as_str(), "runnerCode": code}));
            result
        }
        None => match runner_code_detail(code) {
            Some(detail) => DomainResult::canonical_rejection_detailed(at, detail, message),
            None => DomainResult::canonical_rejection(at, map_runner_code(code), message),
        },
    }
}

/// Уточнение отказа по коду раннера, где код сам называет причину: нужной
/// установки платформы или живого агента нет. Исход — за человеком, повтор не
/// поможет.
pub(super) fn runner_code_detail(code: &str) -> Option<RefusalDetail> {
    match code {
        // Строгий выбор платформы (раннер 0.17): выпуска, который пишет формат
        // исходников, среди установок нет.
        PLATFORM_RELEASE_MISSING => Some(RefusalDetail::ProviderAbsent),
        // Удержание живого агента (`agent hold`) Unica не вызывает; его отказы
        // называют настройку или среду, которые правит человек: агент выключен,
        // в этом задании невозможен или их предел исчерпан.
        "agent_limit" | "live_agent_off" | "live_agent_job" => Some(RefusalDetail::ProviderAbsent),
        _ => None,
    }
}

/// Отказ раннера 0.17 в строгом выборе платформы (#593): выпуска, который
/// пишет формат исходников, среди установок нет.
pub(super) const PLATFORM_RELEASE_MISSING: &str = "platform_release_missing";

/// Отказ строгого выбора платформы Unica называет `platformLineNotInstalled`:
/// линия формата и найденные установки других линий. Из описи раннера наружу
/// идут только версии: каталоги установок не публикуются. Прочие отказы
/// проходят как есть.
/// Отказ раннера по его конверту: строка платформы названа, а предупреждения
/// о прежних ключах конфигурации идут с отказом, как и с успехом.
pub(super) fn runner_envelope_refusal(result: DomainResult, envelope: &Value) -> DomainResult {
    let mut result = with_platform_release(result, &envelope["error"]);
    note_config_warnings(&mut result, envelope);
    // Занятую базу освобождает человек: вопрос о ней привязывается к вызову
    // при исполнении (INV.RUNTIME.INFOBASE-SESSIONS-QUESTION).
    if let Some(choice) = super::v13_infobase_sessions::choice_for(envelope) {
        result.human_choice = Some(Box::new(choice));
    }
    result
}

pub(super) fn with_platform_release(mut result: DomainResult, error: &Value) -> DomainResult {
    if error["code"] != PLATFORM_RELEASE_MISSING {
        return result;
    }
    let release = &error["platform_release"];
    let line = release["source_format"]["release"].as_str();
    let format = release["source_format"]["version"].as_str();
    let installations: Vec<String> = release["installations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|installation| installation["version"].as_str())
        .map(redactor)
        .collect();
    let named_line = match (line, format) {
        (Some(line), Some(format)) => format!("platform line {line} (source format {format})"),
        (None, Some(format)) => format!("the platform line of source format {format}"),
        _ => "the platform line of the sources".to_string(),
    };
    let found = if installations.is_empty() {
        "none".to_string()
    } else {
        installations.join(", ")
    };
    let message = format!(
        "platformLineNotInstalled: {named_line} is not installed; installations found: {found}"
    );
    result.summary = message.clone();
    if let Some(diagnostic) = result.diagnostics.first_mut() {
        diagnostic["message"] = Value::String(message);
    }
    let detail = json!({
        "line": line,
        "sourceFormat": format,
        "installations": installations,
    });
    match result.data.as_mut().and_then(Value::as_object_mut) {
        Some(data) => {
            data.insert("platformLineNotInstalled".into(), detail);
        }
        None => {
            result.data = Some(json!({
                "runnerCode": PLATFORM_RELEASE_MISSING,
                "platformLineNotInstalled": detail,
            }))
        }
    }
    result
}

/// Код предупреждения Unica о записи в файловую базу другой рабочей копии. Тем
/// же кодом раннер 0.17.2 называет своё предупреждение в `warning_details`.
pub(super) const ANOTHER_COPY_WARNING_CODE: &str = "infobase_of_another_copy";

/// Устойчивая часть предупреждения раннера о базе другой рабочей копии
/// (`use_cases::infobase_owner`: «<команда> writes the infobase '<путь>' of
/// another working copy: it is held by …»).
///
/// Запасной путь на один цикл выпуска для раннера без поля `warning_details`
/// (до 0.17.2, IngvarConsulting/v8-runner-rust#460): его держат тесты на
/// живых конвертах 0.14 и 0.15. Удалить в следующем цикле.
const ANOTHER_COPY_RUNNER_FRAGMENT: &str = " of another working copy: it is held by ";

/// Раннер предупредил о записи в базу другой рабочей копии: кодом в
/// `warning_details`, а раннер без этого поля — прозой в `warnings`.
fn runner_warns_of_another_copy(envelope: &Value) -> bool {
    match envelope["warning_details"].as_array() {
        Some(details) => details
            .iter()
            .any(|detail| detail["code"] == ANOTHER_COPY_WARNING_CODE),
        None => envelope["warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .any(|text| text.contains(ANOTHER_COPY_RUNNER_FRAGMENT)),
    }
}

/// Своё предупреждение Unica вместо прозы раннера: без путей чужой копии, без
/// метки владельца и без команд раннера. Наружу идут ещё только предупреждения
/// о прежних ключах конфигурации ([`note_config_warnings`]).
pub(super) fn another_copy_warning(envelope: &Value) -> Option<Value> {
    runner_warns_of_another_copy(envelope).then(|| {
            json!({
                "code": ANOTHER_COPY_WARNING_CODE,
                "message": "this command writes a file infobase that another working copy holds: it changes that copy's infobase, and when it changes the configuration, that copy's next load into it may be refused as moved ahead. Give this working copy an infobase of its own: declare infobases.origin with its own connection in v8project.local.yaml, then create it with infobase.create",
            })
        })
}

/// Код предупреждения раннера о прежнем написании ключа конфигурации.
pub(super) const CONFIG_SYNONYM_WARNING_CODE: &str = "runner_config_synonym";

/// Устойчивая часть предупреждения раннера 0.17 о прежнем ключе, который он
/// принимает ещё один цикл выпуска (`config::loader`: «`build:` in
/// v8project.yaml is a one-cycle synonym for `push:`; rename the key»).
const CONFIG_SYNONYM_RUNNER_FRAGMENT: &str = " is a one-cycle synonym for ";

/// Передаёт предупреждения раннера о прежних ключах конфигурации как есть:
/// они называют файл и ключ, без путей, и своего текста у Unica для них нет.
pub(super) fn note_config_warnings(result: &mut DomainResult, envelope: &Value) {
    for text in envelope["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|text| text.contains(CONFIG_SYNONYM_RUNNER_FRAGMENT))
    {
        let warning = json!({"code": CONFIG_SYNONYM_WARNING_CODE, "message": text});
        if !result.warnings.contains(&warning) {
            result.warnings.push(warning);
        }
    }
}

/// Добавляет к ответу предупреждения раннера о прежних ключах конфигурации и
/// предупреждение о базе другой копии с выходом к своей базе — превью
/// `infobase.create`, — если раннер о ней предупредил.
pub(super) fn note_runner_warnings(result: &mut DomainResult, envelope: &Value) {
    note_config_warnings(result, envelope);
    let Some(warning) = another_copy_warning(envelope) else {
        return;
    };
    if result
        .warnings
        .iter()
        .any(|existing| existing["code"] == ANOTHER_COPY_WARNING_CODE)
    {
        return;
    }
    result.warnings.push(warning);
    result.next.push(json!({
        "tool": "unica.run",
        "args": {"op": "infobase.create", "args": {}, "dryRun": true},
        "reason": "the infobase belongs to another working copy: declare infobases.origin with this working copy's own connection in v8project.local.yaml, then preview creating that infobase"
    }));
}

pub(super) fn map_runner_code(code: &str) -> RefusalCode {
    // Память копии о базе и сама база разошлись: база ушла вперёд записанного
    // поколения или памяти о ней нет. Правкой вызова это не снять, повтором —
    // тоже: выбрать между выгрузкой базы и её перезаписью должен тот, кто
    // знает, чья правка верна.
    if BaseDivergence::from_runner_code(code).is_some() {
        return RefusalCode::InvalidState;
    }
    if let Some(detail) = runner_code_detail(code) {
        return detail.code();
    }
    match code {
        // Платформа сказала нет: авторизация, права, лицензия, занятая база.
        "platform_failure" => RefusalCode::ProviderUnavailable,
        // Бинарник, версия или соединение не готовы.
        "environment_unavailable" => RefusalCode::ProviderUnavailable,
        // У раннера нет адаптера под эту операцию: повторять и править вызов незачем.
        "capability_unavailable" => RefusalCode::UnsupportedOperation,
        // Предмет не тот, цель не та, или раннер пока этого не умеет: правкой
        // значения тот же вызов не пройдёт, нужна другая операция.
        "subject" | "target" | "soon" => RefusalCode::UnsupportedOperation,
        // Значение названо верно по форме, но раннер его не принимает.
        "unsupported_value" => RefusalCode::BadValue,
        // Рабочий каталог или файловую базу держит другая команда, чужие
        // сеансы или чужой Конфигуратор, или ещё идёт брошенная команда агента:
        // занятость проходит, тот же вызов можно повторить, когда та закончит.
        "workspace_busy"
        | "infobase_busy"
        | "infobase_sessions"
        | "infobase_designer_busy"
        | "agent_command_abandoned" => RefusalCode::ConcurrentChange,
        // Рабочее пространство так не годится: набор скрыт правилом git или
        // `workPath` лежит на другом томе, чем набор. Правит человек, текст
        // отказа раннера называет, что именно.
        "source_set_ignored_by_git" | "work_path_on_another_volume" => RefusalCode::InvalidState,
        "invalid_argument" => RefusalCode::BadValue,
        // Раннер отдал негодный результат: аргументы вызывающего тут не виноваты.
        "invalid_output" => RefusalCode::InvalidResult,
        "timed_out" => RefusalCode::DeadlineExceeded,
        "cancelled" => RefusalCode::Cancelled,
        // Неклассифицированный сбой самого раннера.
        "runtime_failure" => RefusalCode::ProviderFailed,
        _ => RefusalCode::ProviderFailed,
    }
}

#[derive(Debug, Clone)]
struct PreviewPlan {
    provider: String,
    receipt: Value,
}

fn validate_preview(
    prepared: &PreparedInfobaseExport,
    envelope: &Value,
) -> Result<PreviewPlan, DomainResult> {
    let data = &envelope["data"];
    let expected_kind = prepared
        .operation
        .artifact_kind(prepared.arguments.extension.as_deref());
    // Доказательство непустого хода у выгрузки — `published`, у загрузки — `restored`:
    // поле называет своё действие, и подменять одно другим нельзя.
    let applied_nothing = if prepared.operation.writes_named_file() {
        data["published"] == false
    } else {
        data["restored"] == false
    };
    if data["mode"] != "preview"
        || data["provider_dispatched"] != false
        || !applied_nothing
        || data["target_state"] != "unchanged"
        || data["execution"]["status"] != "succeeded"
        || data["artifact_kind"] != expected_kind
        || !runner_subject_matches(prepared, data)
    {
        return Err(reject(
            prepared.operation,
            RefusalCode::InvalidResult,
            "v8-runner preview did not satisfy the non-executing export contract",
        ));
    }
    let provider = data["plan"]["provider"]
        .as_str()
        .filter(|provider| valid_provider_id(provider))
        .ok_or_else(|| {
            reject(
                prepared.operation,
                RefusalCode::InvalidResult,
                "v8-runner preview omitted the selected provider",
            )
        })?
        .to_string();
    let receipt = validate_provider_receipt(&data["provider"])
        .map_err(|message| reject(prepared.operation, RefusalCode::InvalidResult, message))?;
    if receipt["selected"] != provider {
        return Err(reject(
            prepared.operation,
            RefusalCode::InvalidResult,
            "v8-runner preview provider differs from its receipt",
        ));
    }
    let plan_path_key = if prepared.operation.writes_named_file() {
        "output"
    } else {
        "input"
    };
    let output = data["plan"][plan_path_key]
        .as_str()
        .ok_or_else(|| {
            reject(
                prepared.operation,
                RefusalCode::InvalidResult,
                format!("v8-runner preview omitted the resolved {plan_path_key}"),
            )
        })?
        .to_string();
    let runner_output = normalize_path_identity(Path::new(&output)).map_err(|error| {
        reject(
            prepared.operation,
            RefusalCode::InvalidResult,
            format!("v8-runner preview returned an invalid {plan_path_key} path: {error}"),
        )
    })?;
    let expected_output =
        normalize_path_identity(&prepared.arguments.named_file).map_err(|error| {
            reject(
                prepared.operation,
                RefusalCode::InvalidResult,
                format!("failed to resolve the planned {plan_path_key}: {error}"),
            )
        })?;
    if runner_output != expected_output {
        return Err(reject(
            prepared.operation,
            RefusalCode::InvalidResult,
            format!("v8-runner preview resolved a different {plan_path_key}"),
        ));
    }
    Ok(PreviewPlan { provider, receipt })
}

/// Validate the pinned runner's structured receipt. Prose and absolute override paths
/// stay private; the public receipt exposes only the validated provider facts.
///
/// Runner 0.12 adds `endpoint` once an agent session was opened, so an apply may
/// carry it and its preview may not. Runner 0.17 adds `live` to the endpoint of a
/// managed agent: whether the command took a live agent and whether it stays. It
/// is validated and left out of the returned receipt with the endpoint: the
/// preview and apply receipts compare by the selected provider.
pub(super) fn validate_provider_receipt(value: &Value) -> Result<Value, &'static str> {
    let object = value
        .as_object()
        .ok_or("v8-runner omitted its provider receipt")?;
    if object
        .keys()
        .any(|key| !["selected", "origin", "skipped", "endpoint"].contains(&key.as_str()))
        || !value["selected"].as_str().is_some_and(valid_provider_id)
    {
        return Err("v8-runner returned an invalid selected provider");
    }
    if let Some(endpoint) = object.get("endpoint") {
        let valid = endpoint.as_object().is_some_and(|endpoint| {
            let mode = endpoint.get("mode").and_then(Value::as_str);
            let live = endpoint.get("live");
            endpoint.len() == 2 + usize::from(live.is_some())
                && matches!(mode, Some("managed" | "attached" | "gate"))
                && endpoint
                    .get("address")
                    .and_then(Value::as_str)
                    .is_some_and(|address| !address.is_empty() && address.len() <= 512)
                && live.is_none_or(|live| mode == Some("managed") && valid_live_receipt(live))
        });
        if !valid {
            return Err("v8-runner returned an invalid provider endpoint");
        }
    }
    let origin = value["origin"]
        .as_object()
        .ok_or("v8-runner omitted provider origin")?;
    let valid_origin = match value["origin"]["kind"].as_str() {
        Some("default") => origin.len() == 1,
        Some("override") => {
            origin.len() == 2
                && value["origin"]["file"]
                    .as_str()
                    .is_some_and(|file| !file.is_empty() && file.len() <= 4096)
        }
        _ => false,
    };
    if !valid_origin {
        return Err("v8-runner returned an invalid provider origin");
    }
    if let Some(skipped) = object.get("skipped") {
        let skipped = skipped
            .as_array()
            .filter(|items| items.len() <= 8)
            .ok_or("v8-runner returned invalid skipped providers")?;
        if !skipped.iter().all(|item| {
            item.as_object().is_some_and(|item| item.len() == 2)
                && item["provider"].as_str().is_some_and(valid_provider_id)
                && item["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.len() <= 4096)
        }) {
            return Err("v8-runner returned invalid skipped providers");
        }
    }
    let mut receipt = value.clone();
    receipt
        .as_object_mut()
        .expect("validated receipt object")
        .remove("endpoint");
    Ok(receipt)
}

/// `endpoint.live` of runner 0.17: `reused` and `kept`, the holders, and the
/// reason an agent did not stay.
fn valid_live_receipt(live: &Value) -> bool {
    live.as_object().is_some_and(|live| {
        live.keys()
            .all(|key| ["reused", "kept", "holders", "reason"].contains(&key.as_str()))
            && live.get("reused").is_some_and(Value::is_boolean)
            && live.get("kept").is_some_and(Value::is_boolean)
            && live
                .get("holders")
                .and_then(Value::as_array)
                .is_some_and(|holders| holders.len() <= 64 && holders.iter().all(Value::is_object))
            && live
                .get("reason")
                .is_none_or(|reason| reason.as_str().is_some_and(|reason| reason.len() <= 64))
    })
}

fn valid_provider_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || (byte == b'-' && index > 0 && index + 1 < value.len())
        })
}

fn validate_apply(
    prepared: &PreparedInfobaseExport,
    envelope: &Value,
) -> Result<String, DomainResult> {
    let data = &envelope["data"];
    let provider = validate_provider_receipt(&data["provider"])
        .ok()
        .and_then(|receipt| receipt["selected"].as_str().map(str::to_owned))
        .filter(|provider| valid_provider_id(provider));
    let applied = if prepared.operation.writes_named_file() {
        data["published"] == true
    } else {
        data["restored"] == true
    };
    if data["mode"] != "apply"
        || !applied
        || data["execution"]["status"] != "succeeded"
        || data["artifact_kind"]
            != prepared
                .operation
                .artifact_kind(prepared.arguments.extension.as_deref())
        || provider.is_none()
        || !matches!(data["target_state"].as_str(), Some("created" | "replaced"))
        || !runner_subject_matches(prepared, data)
    {
        return Err(reject(
            prepared.operation,
            RefusalCode::InvalidResult,
            "v8-runner apply result does not match the export contract",
        ));
    }
    let applied_path_key = if prepared.operation.writes_named_file() {
        "output"
    } else {
        "input"
    };
    let Some(output) = data[applied_path_key].as_str() else {
        return Err(reject(
            prepared.operation,
            RefusalCode::InvalidResult,
            format!("v8-runner apply result omitted the {applied_path_key} it acted on"),
        ));
    };
    let applied_output = normalize_path_identity(Path::new(output)).map_err(|error| {
        reject(
            prepared.operation,
            RefusalCode::InvalidResult,
            format!("v8-runner apply returned an invalid {applied_path_key} path: {error}"),
        )
    })?;
    let expected_output =
        normalize_path_identity(&prepared.arguments.named_file).map_err(|error| {
            reject(
                prepared.operation,
                RefusalCode::InvalidResult,
                format!("failed to resolve the expected output: {error}"),
            )
        })?;
    if applied_output != expected_output {
        return Err(reject(
            prepared.operation,
            RefusalCode::InvalidResult,
            "v8-runner apply published a different output than the requested target",
        ));
    }
    Ok(provider.unwrap_or_default())
}

fn runner_subject_matches(prepared: &PreparedInfobaseExport, data: &Value) -> bool {
    match prepared.operation {
        ExportOperation::Configuration => {
            let state_matches = data["state"].as_str() == prepared.arguments.state.as_deref();
            let subject = &data["subject"];
            let subject_matches = match prepared.arguments.extension.as_deref() {
                Some(extension) => {
                    subject["kind"] == "extension" && subject["name"].as_str() == Some(extension)
                }
                None => subject["kind"] == "main" && subject.get("name").is_none(),
            };
            state_matches && subject_matches
        }
        ExportOperation::Infobase => data["subject"]["kind"] == "infobase",
        ExportOperation::Restore => {
            // Раннер называет режим в ответе; несовпадение значит, что применён не тот
            // план, который одобрили.
            data["subject"]["kind"] == "infobase"
                && data["target_mode"].as_str()
                    == prepared.arguments.restore_mode.map(RestoreMode::as_str)
        }
    }
}

fn public_plan(prepared: &PreparedInfobaseExport, plan: &PreviewPlan) -> Value {
    let named = json!({
        "kind": prepared.operation.artifact_kind(prepared.arguments.extension.as_deref()),
        "path": path_text(&prepared.arguments.named_file_relative),
    });
    let mut plan_value = json!({
        "provider": plan.provider,
        "origin": plan.receipt["origin"]["kind"],
    });
    let object = plan_value.as_object_mut().expect("plan object");
    if prepared.operation.writes_named_file() {
        object.insert("artifact".to_string(), named);
    } else {
        // У загрузки названный файл — источник, и план называет его источником:
        // слово `artifact` здесь читалось бы как «что мы произведём».
        object.insert("source".to_string(), named);
        object.insert(
            "mode".to_string(),
            json!(prepared.arguments.restore_mode.map(RestoreMode::as_str)),
        );
        // Что превью узнать не может, названо, а не умолчано.
        object.insert(
            "targetStateKnownBeforeApply".to_string(),
            Value::Bool(false),
        );
    }
    plan_value
}

fn public_arguments(prepared: &PreparedInfobaseExport) -> Value {
    let mut args = Map::new();
    if let Some(state) = &prepared.arguments.state {
        args.insert("state".to_string(), Value::String(state.clone()));
    }
    let named_key = if prepared.operation.writes_named_file() {
        "output"
    } else {
        "input"
    };
    args.insert(
        named_key.to_string(),
        Value::String(path_text(&prepared.arguments.named_file_relative)),
    );
    if let Some(mode) = prepared.arguments.restore_mode {
        args.insert("mode".to_string(), Value::String(mode.as_str().to_string()));
    }
    if let Some(extension) = &prepared.arguments.extension {
        args.insert("extension".to_string(), Value::String(extension.clone()));
    }
    Value::Object(args)
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn reject(
    operation: ExportOperation,
    code: RefusalCode,
    message: impl Into<String>,
) -> DomainResult {
    DomainResult::canonical_rejection(Some(operation.name().to_string()), code, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;
    use unica_process::runner::ProcessOutput;
    use unica_refusal::Outcome;

    /// Страж отображения кодов раннера.
    ///
    /// Корень прежнего дефекта: карта перечисляла имена, которых раннер не пишет
    /// (`platform_error`, `validation_error`, `timeout`), а его настоящие имена
    /// (`platform_failure`, `timed_out`) в неё не попадали и уходили в запасную ветку.
    /// Владелец словаря один — [`RUNNER_WIRE_CODES`]; если раннер добавит код, этот
    /// тест покажет его, а не запасная ветка молча.
    /// rule: INV.RUNTIME.RUNNER-BASE-REFUSALS
    #[test]
    fn every_runner_wire_code_maps_away_from_the_fallback() {
        for code in RUNNER_WIRE_CODES {
            let mapped = map_runner_code(code);
            if code == "runtime_failure" {
                // Единственный код, для которого запасной отказ и есть верный ответ.
                assert_eq!(mapped, RefusalCode::ProviderFailed, "{code}");
                continue;
            }
            assert_ne!(
                mapped,
                RefusalCode::ProviderFailed,
                "{code} uses the fallback refusal"
            );
        }
    }

    /// Отказ платформы — авторизация, права, лицензия — обязан звать человека.
    ///
    /// Исход `DeadEnd` здесь самый дорогой из возможных: агент бросает работу там, где
    /// хватило бы одной фразы пользователю — дать пароль, выдать право, поставить
    /// лицензию.
    #[test]
    fn a_platform_refusal_asks_for_a_human_rather_than_reporting_a_dead_end() {
        let mapped = map_runner_code("platform_failure");

        assert_eq!(mapped, RefusalCode::ProviderUnavailable);
        assert_eq!(mapped.outcome(), Outcome::NeedsHuman);
    }

    /// rule: INV.RUNTIME.RUNNER-BASE-REFUSALS
    #[test]
    fn runner_outcomes_follow_the_codes_rather_than_the_fallback() {
        for (code, expected) in [
            ("environment_unavailable", Outcome::NeedsHuman),
            ("capability_unavailable", Outcome::GoElsewhere),
            ("workspace_busy", Outcome::RetryAsIs),
            ("infobase_busy", Outcome::RetryAsIs),
            ("infobase_sessions", Outcome::RetryAsIs),
            ("work_path_on_another_volume", Outcome::NeedsHuman),
            ("source_set_ignored_by_git", Outcome::NeedsHuman),
            ("non_fast_forward", Outcome::NeedsHuman),
            ("untracked_base", Outcome::NeedsHuman),
            ("timed_out", Outcome::RetryAsIs),
            ("invalid_argument", Outcome::FixCall),
            ("invalid_output", Outcome::DeadEnd),
            ("cancelled", Outcome::DeadEnd),
        ] {
            assert_eq!(map_runner_code(code).outcome(), expected, "{code}");
        }
        // Нужной установки платформы или живого агента нет: отказ уточнён.
        for code in [
            "platform_release_missing",
            "agent_limit",
            "live_agent_off",
            "live_agent_job",
        ] {
            let refused = runner_rejection(None, code, None, "m");
            assert_eq!(
                refused.diagnostics[0]["code"], "provider_unavailable",
                "{code}"
            );
            assert_eq!(
                refused.diagnostics[0]["detailCode"], "provider_absent",
                "{code}"
            );
        }
    }

    /// Поставляемого раннера нет — у всех семи операций `run` один ответ.
    ///
    /// Прежде каждая операция отвечала голым `provider_unavailable`, и агент
    /// читал умолчание `needsHuman` без причины. Теперь отказ общий для всех
    /// маршрутов «нет поставляемого движка»: уточнение `provider_absent`
    /// и поля движка в `data`, взятые из типа резолвера, а не из текста.
    /// rule: INV.RUNTIME.RUNNER-ENVIRONMENT-LAYER
    /// rule: INV.WIRE.MISSING-ENGINE-REFUSAL
    #[test]
    fn an_absent_bundled_runner_names_the_missing_engine_in_data() {
        let root = tempfile::tempdir().unwrap();
        let plugin = root.path().join("plugins/unica");
        fs::create_dir_all(plugin.join("skills")).unwrap();
        fs::create_dir_all(plugin.join("third-party")).unwrap();
        fs::write(
            plugin.join("third-party/manifest.json"),
            r#"{"schemaVersion":2,"sourceManifest":true,"tools":[]}"#,
        )
        .unwrap();
        fs::write(
            plugin.join("third-party/tools.lock.json"),
            r#"{"schemaVersion":1,
  "targets":{"darwin-arm64":{"exe":""},"linux-x64":{"exe":""},"win-x64":{"exe":".exe"}},
  "tools":[{"name":"v8-runner","version":"0.17.1","binaryName":"v8-runner"}]}"#,
        )
        .unwrap();

        let absence = match resolve_bundled_runner(root.path()) {
            Err(absence) => absence,
            Ok(_) => panic!("a source checkout without a built runner has no runner"),
        };
        let rejection = missing_runner_rejection(Some("infobase.dump".to_string()), absence);

        let diagnostic = &rejection.diagnostics[0];
        assert_eq!(diagnostic["code"], "provider_unavailable");
        assert_eq!(diagnostic["detailCode"], "provider_absent");
        assert_eq!(diagnostic["outcome"], "needsHuman");
        let data = rejection.data.as_ref().expect("поля движка в data");
        assert_eq!(data["tool"], "v8-runner");
        assert_eq!(data["pinnedVersion"], "0.17.1");
        assert_eq!(data["stage"], "manifest");
        assert_eq!(data["retryable"], false);
        assert_eq!(
            data["target"],
            crate::infrastructure::current_target_id().unwrap()
        );
        assert!(
            data["path"]
                .as_str()
                .is_some_and(|path| path.replace('\\', "/").contains("/bin/")),
            "{data}"
        );

        let rootless = missing_runner_rejection(
            None,
            RunnerAbsence::Missing(Box::new(
                crate::infrastructure::bundled_tools::plugin_root_missing("v8-runner"),
            )),
        );
        assert_eq!(rootless.diagnostics[0]["detailCode"], "provider_absent");
        assert_eq!(rootless.data.as_ref().unwrap()["path"], Value::Null);
        assert_eq!(rootless.data.as_ref().unwrap()["stage"], "manifest");
    }

    /// Отказ среды раннера называет слой. Агент конфигуратора не принял
    /// пользователя ИБ, точка входа агента не ответила, прочая среда раннера и
    /// отсутствующий поставляемый раннер — четыре разных ответа, а не один
    /// `provider_absent`.
    /// rule: INV.RUNTIME.RUNNER-ENVIRONMENT-LAYER
    #[test]
    fn runner_environment_refusals_name_the_layer_that_refused() {
        // Тексты — `AgentError` раннеров 0.14–0.17 дословно, с контекстом, который
        // раннер дописывает перед ним.
        for (message, detail, layer) in [
            (
                "agent at 127.0.0.1:56018 rejected the credentials of user ''",
                "credentials_rejected",
                "infobase_user",
            ),
            (
                "push; agent at 127.0.0.1:56018 rejected the credentials of user 'Админ'",
                "credentials_rejected",
                "infobase_user",
            ),
            (
                "agent at 127.0.0.1:56018 is unreachable: Connection refused (os error 61)",
                "endpoint_unreachable",
                "agent_endpoint",
            ),
            (
                "SSH handshake with the agent at 127.0.0.1:56018 failed: Disconnect",
                "endpoint_unreachable",
                "agent_endpoint",
            ),
            (
                "agent at 127.0.0.1:56018 presented SHA256:a as its host key, but SHA256:b was expected",
                "endpoint_unreachable",
                "agent_endpoint",
            ),
            (
                "127.0.0.1:56018 answered with host key SHA256:a, not with the key the runner handed to the agent it launched (SHA256:b): another SSH server holds the port, and the runner does not talk to it",
                "endpoint_unreachable",
                "agent_endpoint",
            ),
            (
                "agent at 127.0.0.1:56018 did not open a shell channel: Disconnect",
                "endpoint_unreachable",
                "agent_endpoint",
            ),
            (
                "no 1C platform 8.3.27 is installed on this machine",
                "provider_absent",
                "runner_environment",
            ),
        ] {
            let rejection = runner_rejection(
                Some("push".to_string()),
                "environment_unavailable",
                None,
                message,
            );
            let diagnostic = &rejection.diagnostics[0];
            assert_eq!(diagnostic["code"], "provider_unavailable", "{message}");
            assert_eq!(diagnostic["detailCode"], detail, "{message}");
            assert_eq!(diagnostic["outcome"], "needsHuman", "{message}");
            assert_eq!(
                rejection.data,
                Some(json!({"layer": layer, "runnerCode": "environment_unavailable"})),
                "{message}"
            );
            let text = diagnostic["message"].as_str().unwrap();
            assert!(text.ends_with(message), "улика раннера сохранена: {text}");
            assert!(
                text.len() > message.len() + "; v8-runner said: ".len(),
                "шаг исправления назван: {text}"
            );
        }

        // Маршруты передают текст раннера после общей редакции секретов:
        // маркеры слоя должны её пережить, иначе пароль или ключ в тексте
        // увёл бы отказ в чужой слой.
        for (message, layer) in [
            (
                "agent at 127.0.0.1:56018 rejected the credentials of user 'Админ' (password=secret-1333)",
                RunnerLayer::InfobaseUser,
            ),
            (
                "agent at 127.0.0.1:56018 is unreachable: token=secret-1333 Connection refused",
                RunnerLayer::AgentEndpoint,
            ),
            (
                "SSH handshake with the agent at 127.0.0.1:56018 failed: Pwd=secret-1333",
                RunnerLayer::AgentEndpoint,
            ),
            (
                "agent at 127.0.0.1:56018 presented SHA256:a as its host key, but SHA256:b was expected; password=secret-1333",
                RunnerLayer::AgentEndpoint,
            ),
        ] {
            let redacted = redactor(message);
            assert!(!redacted.contains("secret-1333"), "{redacted}");
            assert_eq!(
                map_runner_detail("environment_unavailable", None, &redacted),
                Some(layer),
                "{redacted}"
            );
        }

        // Тот же текст под другим кодом слоя не выбирает: решает код раннера.
        let platform = runner_rejection(
            Some("push".to_string()),
            "platform_failure",
            None,
            "agent at 127.0.0.1:1 rejected the credentials of user ''",
        );
        assert!(platform.diagnostics[0].get("detailCode").is_none());
        assert!(platform.data.is_none());
    }

    /// Раннер 0.17.2 называет слой полем `error.layer`, и поле решает слой
    /// прежде текста: текст другого слоя его не переспорит, а текст без
    /// маркеров не нужен. Запуск агента и утилита платформы — среда раннера,
    /// у запуска агента свой шаг исправления. Значение вне набора — среда
    /// раннера, как и отказ без поля.
    /// rule: INV.RUNTIME.RUNNER-ENVIRONMENT-LAYER
    #[test]
    fn the_layer_field_of_the_runner_decides_the_layer() {
        let endpoint_text = "agent at 127.0.0.1:56018 is unreachable: Connection refused";
        for (field, message, detail, layer, advice) in [
            (
                "infobase_user",
                endpoint_text,
                "credentials_rejected",
                "infobase_user",
                "or found no 1C license",
            ),
            (
                "agent_endpoint",
                "m",
                "endpoint_unreachable",
                "agent_endpoint",
                "endpoint did not answer",
            ),
            (
                "agent_start",
                "m",
                "provider_absent",
                "runner_environment",
                "its keeper is unavailable",
            ),
            (
                "platform",
                endpoint_text,
                "provider_absent",
                "runner_environment",
                "installed 1C platform version",
            ),
            (
                "unknown_layer",
                endpoint_text,
                "provider_absent",
                "runner_environment",
                "installed 1C platform version",
            ),
        ] {
            let rejection = runner_rejection(
                Some("push".to_string()),
                "environment_unavailable",
                Some(field),
                message,
            );
            let diagnostic = &rejection.diagnostics[0];
            assert_eq!(diagnostic["code"], "provider_unavailable", "{field}");
            assert_eq!(diagnostic["detailCode"], detail, "{field}");
            assert_eq!(diagnostic["outcome"], "needsHuman", "{field}");
            assert_eq!(
                rejection.data,
                Some(json!({"layer": layer, "runnerCode": "environment_unavailable"})),
                "{field}"
            );
            let text = diagnostic["message"].as_str().unwrap();
            assert!(text.contains(advice), "{field}: {text}");
            assert!(text.ends_with(message), "{field}: {text}");
        }

        // Поле слоя у другого кода раннера слоя не выбирает.
        let busy = runner_rejection(None, "agent_busy", Some("agent_start"), "m");
        assert!(busy.data.is_none());
    }

    /// rule: INV.RUNTIME.PLATFORM-LINE
    #[test]
    fn a_missing_platform_release_names_the_line_and_hides_installation_paths() {
        // Ответ раннера 0.17 на строгий выбор (#593): формат 2.20 пишет 8.3.27,
        // установлены 8.3.24 и 8.5.1.
        let error = json!({
            "code": "platform_release_missing",
            "kind": "environment",
            "message": "platform release 8.3.27 required by source format 2.20 \
                        (C:/work/src/Configuration.xml) is not installed",
            "platform_release": {
                "source_format": {
                    "version": "2.20",
                    "file": "C:/work/src/Configuration.xml",
                    "release": "8.3.27"
                },
                "installations": [
                    {"version": "8.5.1.100", "root": "C:/Program Files/1cv8/8.5.1.100",
                     "components": ["designer", "ibcmd"]},
                    {"version": "8.3.24.1691", "root": "C:/Program Files/1cv8/8.3.24.1691",
                     "components": ["designer"]}
                ]
            }
        });
        let result = with_platform_release(
            runner_rejection(
                Some("source.export".to_string()),
                error["code"].as_str().unwrap(),
                error["layer"].as_str(),
                error["message"].as_str().unwrap(),
            ),
            &error,
        );
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
        assert_eq!(result.diagnostics[0]["detailCode"], "provider_absent");
        assert!(
            result.summary.starts_with(
                "platformLineNotInstalled: platform line 8.3.27 (source format 2.20) is not installed"
            ),
            "{}",
            result.summary
        );
        assert!(result.summary.contains("8.5.1.100, 8.3.24.1691"));
        assert_eq!(result.diagnostics[0]["message"], result.summary.as_str());
        let data = result.data.expect("refusal data");
        assert_eq!(data["runnerCode"], "platform_release_missing");
        assert_eq!(
            data["platformLineNotInstalled"],
            json!({
                "line": "8.3.27",
                "sourceFormat": "2.20",
                "installations": ["8.5.1.100", "8.3.24.1691"]
            })
        );
        let published = serde_json::to_string(&data).unwrap() + &result.summary;
        assert!(!published.contains("Program Files"), "{published}");
        assert!(!published.contains("Configuration.xml"), "{published}");

        // Формат, неизвестный раннеру: линии нет, формат назван.
        let unknown = json!({
            "code": "platform_release_missing",
            "platform_release": {
                "source_format": {"version": "2.99", "file": "x", "release": null},
                "installations": []
            }
        });
        let result = with_platform_release(
            runner_rejection(None, "platform_release_missing", None, "m"),
            &unknown,
        );
        assert_eq!(
            result.summary,
            "platformLineNotInstalled: the platform line of source format 2.99 is not installed; \
             installations found: none"
        );

        // Прочие отказы проходят как есть.
        let other = json!({"code": "environment_unavailable", "message": "m"});
        let untouched = with_platform_release(
            runner_rejection(None, "environment_unavailable", None, "m"),
            &other,
        );
        let plain = runner_rejection(None, "environment_unavailable", None, "m");
        assert_eq!(untouched.summary, plain.summary);
        assert_eq!(untouched.data, plain.data);
    }

    /// Предупреждение раннера о прежнем ключе конфигурации доходит как есть и
    /// один раз; прочая проза раннера наружу не идёт.
    #[test]
    fn a_former_config_key_warning_of_the_runner_passes_as_is() {
        let synonym =
            "`build:` in v8project.yaml is a one-cycle synonym for `push:`; rename the key";
        let envelope = json!({"warnings": [
            synonym,
            "provider designer wrote C:/work/private/log.txt",
            synonym,
        ]});
        let mut result = DomainResult::success("done");
        note_runner_warnings(&mut result, &envelope);
        assert_eq!(
            result.warnings,
            vec![json!({"code": "runner_config_synonym", "message": synonym})]
        );
        assert!(result.next.is_empty());

        // Отказ раннера несёт то же предупреждение.
        let mut refused = envelope.clone();
        refused["error"] = json!({"code": "invalid_config", "message": "m"});
        let refusal = runner_envelope_refusal(
            runner_rejection(None, "invalid_config", None, "m".to_string()),
            &refused,
        );
        assert_eq!(
            refusal.warnings,
            vec![json!({"code": "runner_config_synonym", "message": synonym})]
        );
    }

    #[test]
    fn an_unknown_runner_code_stays_on_the_fallback() {
        assert_eq!(
            map_runner_code("something-the-runner-never-wrote"),
            RefusalCode::ProviderFailed
        );
    }

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
                    fs::create_dir_all(output.parent().expect("output parent")).unwrap();
                    fs::write(output, bytes).unwrap();
                }
            }
            Ok(self.outputs.lock().unwrap().pop().expect("runner output"))
        }
    }

    fn process(envelope: Value) -> ProcessOutput {
        ProcessOutput {
            status_success: true,
            status: "exit status: 0".to_string(),
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

    fn failed_process(envelope: Value) -> ProcessOutput {
        ProcessOutput {
            status_success: false,
            status: "exit status: 2".to_string(),
            stdout: serde_json::to_string(&envelope).unwrap(),
            stderr: "provider details remain private".to_string(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        }
    }

    fn prepared(root: &Path, dry_run: bool) -> PreparedInfobaseExport {
        let context = WorkspaceContext {
            cwd: root.to_path_buf(),
            workspace_root: root.to_path_buf(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        };
        PreparedInfobaseExport {
            call: Map::new(),
            operation: ExportOperation::Configuration,
            arguments: ExportArguments {
                state: Some("working".to_string()),
                extension: None,
                named_file_relative: PathBuf::from("dist/main.cf"),
                named_file: root.join("dist/main.cf"),
                restore_mode: None,
            },
            dry_run,
            context,
        }
    }

    fn prepared_restore(root: &Path, dry_run: bool, mode: RestoreMode) -> PreparedInfobaseExport {
        let context = WorkspaceContext {
            cwd: root.to_path_buf(),
            workspace_root: root.to_path_buf(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        };
        PreparedInfobaseExport {
            call: Map::new(),
            operation: ExportOperation::Restore,
            arguments: ExportArguments {
                state: None,
                extension: None,
                named_file_relative: PathBuf::from("transfer/base.dt"),
                named_file: root.join("transfer/base.dt"),
                restore_mode: Some(mode),
            },
            dry_run,
            context,
        }
    }

    /// Конверт превью загрузки: экспортная форма, но признак непустого хода свой —
    /// `restored`, и план называет вход, а не выход.
    fn restore_preview_envelope(input: &Path, mode: &str) -> Value {
        json!({
            "command": "infobase.restore",
            "data": {
                "mode": "preview",
                "provider_dispatched": false,
                "subject": {"kind": "infobase"},
                "target_mode": mode,
                "provider": {"selected": "designer", "origin": {"kind": "default"}},
                "artifact_kind": "dt",
                "restored": false,
                "target_state": "unchanged",
                "plan": {"provider": "designer", "artifact_kind": "dt", "input": input, "target_mode": mode},
                "execution": {"status": "succeeded"}
            }
        })
    }

    fn restore_apply_envelope(input: &Path, mode: &str, target_state: &str) -> Value {
        json!({
            "command": "infobase.restore",
            "data": {
                "mode": "apply",
                "subject": {"kind": "infobase"},
                "target_mode": mode,
                "provider": {"selected": "designer", "origin": {"kind": "default"}},
                "artifact_kind": "dt",
                "restored": true,
                "target_state": target_state,
                "input": input,
                "execution": {"status": "succeeded"}
            }
        })
    }

    fn preview_envelope(output: &Path) -> Value {
        json!({
            "command": "download",
            "data": {
                "mode": "preview",
                "provider_dispatched": false,
                "state": "working",
                "subject": {"kind": "main"},
                "provider": {"selected": "designer", "origin": {"kind": "default"}},
                "artifact_kind": "cf",
                "published": false,
                "target_state": "unchanged",
                "plan": {"provider": "designer", "artifact_kind": "cf", "output": output},
                "execution": {"status": "succeeded"}
            }
        })
    }

    fn apply_envelope(output: &Path) -> Value {
        json!({
            "command": "download",
            "data": {
                "mode": "apply",
                "state": "working",
                "subject": {"kind": "main"},
                "provider": {"selected": "designer", "origin": {"kind": "default"}},
                "artifact_kind": "cf",
                "output": output,
                "published": true,
                "target_state": "created",
                "execution": {"status": "succeeded"}
            }
        })
    }

    /// rule: INV.RUNTIME.V13-INFOBASE-EXPORTS
    #[test]
    fn runner_015_provider_receipt_replaces_selection_for_all_three_operations() {
        let root = tempfile::tempdir().unwrap();
        for operation in [
            ExportOperation::Configuration,
            ExportOperation::Infobase,
            ExportOperation::Restore,
        ] {
            let mut prepared = prepared(root.path(), true);
            prepared.operation = operation;
            if operation == ExportOperation::Restore {
                prepared.arguments.restore_mode = Some(RestoreMode::Create);
            }
            let mut preview = if operation == ExportOperation::Restore {
                restore_preview_envelope(&prepared.arguments.named_file, "create")
            } else {
                preview_envelope(&prepared.arguments.named_file)
            };
            let data = preview["data"].as_object_mut().unwrap();
            data.remove("selection");
            data.insert(
                "provider".into(),
                json!({"selected":"designer", "origin":{"kind":"default"}}),
            );
            data["plan"]["provider"] = json!("designer");
            if operation == ExportOperation::Infobase {
                data.insert("subject".into(), json!({"kind":"infobase"}));
                data.insert("artifact_kind".into(), json!("dt"));
            }
            assert!(
                validate_preview(&prepared, &preview).is_ok(),
                "{operation:?}: valid published 0.12 receipt rejected"
            );
        }
    }

    /// rule: INV.RUNTIME.V13-INFOBASE-EXPORTS
    #[test]
    fn parser_rejects_provider_controls_and_output_escape() {
        let root = tempfile::tempdir().unwrap();
        let context = WorkspaceContext {
            cwd: root.path().to_path_buf(),
            workspace_root: root.path().to_path_buf(),
            cache_root: root.path().join(".build/unica"),
            workspace_epoch: 1,
        };
        let provider = Map::from_iter([
            ("state".to_string(), json!("working")),
            ("output".to_string(), json!("main.cf")),
            ("provider".to_string(), json!("designer")),
        ]);
        assert_eq!(
            parse_export_arguments(ExportOperation::Configuration, &provider, &context)
                .unwrap_err()
                .diagnostics[0]["code"],
            "bad_value"
        );
        let escaped = Map::from_iter([
            ("state".to_string(), json!("working")),
            ("output".to_string(), json!("../main.cf")),
        ]);
        assert_eq!(
            parse_export_arguments(ExportOperation::Configuration, &escaped, &context)
                .unwrap_err()
                .diagnostics[0]["code"],
            "bad_value"
        );
    }

    /// rule: INV.RUNTIME.INFOBASE-IMPORT-MODE
    #[test]
    fn restore_preview_names_the_source_and_the_mode_without_touching_the_infobase() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(CONFIG_NAME), "format: DESIGNER\n").unwrap();
        fs::create_dir_all(root.path().join("transfer")).unwrap();
        fs::write(root.path().join("transfer/base.dt"), b"transfer bytes").unwrap();
        let prepared = prepared_restore(root.path(), true, RestoreMode::Replace);
        let input = normalize_path_identity(&prepared.arguments.named_file).unwrap();
        let runner =
            SequenceRunner::new(vec![process(restore_preview_envelope(&input, "replace"))]);
        let tool = BundledTool {
            program: root.path().join("v8-runner"),
            warnings: Vec::new(),
            missing: None,
        };

        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool,
            "0.8.1",
        );

        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().unwrap();
        // План называет источник, а не артефакт: произвести загрузка ничего не должна.
        assert_eq!(data["plan"]["source"]["kind"], "dt");
        assert_eq!(data["plan"]["source"]["path"], "transfer/base.dt");
        assert_eq!(data["plan"]["mode"], "replace");
        // Чего превью узнать не может, названо, а не умолчано.
        assert_eq!(data["plan"]["targetStateKnownBeforeApply"], false);
        assert!(data.get("artifact").is_none());
        // Ход к применению несёт тот же режим: одобряют ровно то, что исполнят.
        let next = &result.next[0]["args"];
        assert_eq!(next["args"]["input"], "transfer/base.dt");
        assert_eq!(next["args"]["mode"], "replace");
        assert!(next.get("ifRev").is_none());
        assert_eq!(next["dryRun"], false);
        assert!(result.rev.is_none());
        let call = &runner.calls.lock().unwrap()[0];
        assert!(call.args.contains(&"restore".to_string()));
        assert!(call.args.contains(&"--input".to_string()));
        assert!(call.args.contains(&"--replace".to_string()));
        assert!(call.args.contains(&"--dry-run".to_string()));
    }

    /// rule: INV.RUNTIME.INFOBASE-IMPORT-MODE
    #[test]
    fn restore_apply_attributes_the_infobase_state_to_the_provider() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(CONFIG_NAME), "format: DESIGNER\n").unwrap();
        fs::create_dir_all(root.path().join("transfer")).unwrap();
        fs::write(root.path().join("transfer/base.dt"), b"transfer bytes").unwrap();
        let tool = BundledTool {
            program: root.path().join("v8-runner"),
            warnings: Vec::new(),
            missing: None,
        };
        let preview = prepared_restore(root.path(), true, RestoreMode::Replace);
        let input = normalize_path_identity(&preview.arguments.named_file).unwrap();
        let preview_runner =
            SequenceRunner::new(vec![process(restore_preview_envelope(&input, "replace"))]);
        let preview_result = execute_with_resolved_runner(
            &preview,
            &preview_runner,
            CancellationToken::new(),
            &tool,
            "0.8.1",
        );
        assert!(preview_result.ok);
        assert!(preview_result.rev.is_none());

        let apply = prepared_restore(root.path(), false, RestoreMode::Replace);
        let runner = SequenceRunner::new(vec![process(restore_apply_envelope(
            &input, "replace", "replaced",
        ))]);
        let result =
            execute_with_resolved_runner(&apply, &runner, CancellationToken::new(), &tool, "0.8.1");

        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["targetState"], "replaced");
        // Состояние базы без платформы не проверяемо, поэтому источник признания назван.
        assert_eq!(data["targetStateAttestedBy"], "provider");
        assert_eq!(data["mode"], "replace");
        assert_eq!(data["source"]["path"], "transfer/base.dt");
        // Изменилась база, а не файл: путь в `changed` читался бы как «переписали DT».
        assert_eq!(result.changed[0]["infobase"], true);
        assert!(result.changed[0].get("path").is_none());
        assert!(result.artifacts.is_empty());
        // Исполнение не повторяет превью: один вызов раннера.
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
        // Источник обязан дойти неизменным.
        assert_eq!(
            fs::read(root.path().join("transfer/base.dt")).unwrap(),
            b"transfer bytes"
        );
    }

    fn captured_014(name: &str) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/v8_runner_014")
            .join(name);
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    /// Раннер 0.17.2 называет базу другой копии кодом в `warning_details`, и
    /// код решает прежде текста: проза без кода предупреждения не даёт, а код
    /// не требует прозы. Раннер без поля опознаётся по тексту.
    /// rule: INV.RUNTIME.RUNNER-BASE-REFUSALS
    #[test]
    fn the_warning_code_of_the_runner_names_the_infobase_of_another_copy() {
        let prose = "push writes the infobase '/x/ib' of another working copy: it is held by /y";
        let coded = json!({
            "warnings": ["reworded by the runner"],
            "warning_details": [{
                "code": "infobase_of_another_copy",
                "message": "reworded by the runner",
                "name": "origin",
                "address": "file:/x/ib",
                "owners": [{"project": "/y", "host": "h", "since": "2026-11-13T00:00:00Z",
                            "this_copy": false, "this_machine": true}]
            }]
        });
        let warning = another_copy_warning(&coded).expect("код назвал базу другой копии");
        assert_eq!(warning["code"], ANOTHER_COPY_WARNING_CODE);
        assert!(!warning["message"].as_str().unwrap().contains("/y"));

        let uncoded = json!({"warnings": [prose], "warning_details": [
            {"code": "some_other_code", "message": prose}
        ]});
        assert!(another_copy_warning(&uncoded).is_none());

        let older = json!({"warnings": [prose]});
        assert!(another_copy_warning(&older).is_some());
        assert!(another_copy_warning(&json!({"warnings": []})).is_none());
    }

    /// Живые конверты раннера 0.14.0: загрузка `.dt` в базу другой рабочей
    /// копии идёт с предупреждением раннера; ответ называет его своим
    /// предупреждением и выходом к своей базе.
    /// rule: INV.RUNTIME.RUNNER-BASE-REFUSALS
    #[test]
    fn captured_restore_into_the_infobase_of_another_copy_names_it() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(CONFIG_NAME), "format: DESIGNER\n").unwrap();
        fs::create_dir_all(root.path().join("transfer")).unwrap();
        let input = root.path().join("transfer/base.dt");
        fs::write(&input, b"transfer bytes").unwrap();
        let tool = BundledTool {
            program: root.path().join("v8-runner"),
            warnings: Vec::new(),
            missing: None,
        };
        let mut preview = captured_014("other-copy-restore-preview.json");
        let mut applied = captured_014("other-copy-restore-apply.json");
        let input_text = json!(input.display().to_string());
        preview["data"]["input"] = input_text.clone();
        preview["data"]["plan"]["input"] = input_text.clone();
        applied["data"]["input"] = input_text;
        let process = |envelope: Value| ProcessOutput {
            status_success: true,
            status: "exit status: 0".to_string(),
            stdout: serde_json::to_string(&envelope).unwrap(),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        };
        let runner = SequenceRunner::new(vec![process(preview.clone())]);
        let result = execute_with_resolved_runner(
            &prepared_restore(root.path(), true, RestoreMode::Replace),
            &runner,
            CancellationToken::new(),
            &tool,
            "0.14.0",
        );
        assert!(result.ok, "{result:?}");
        super::super::v13_source_import::tests::assert_another_copy_is_named(&result);

        let runner = SequenceRunner::new(vec![process(applied)]);
        let result = execute_with_resolved_runner(
            &prepared_restore(root.path(), false, RestoreMode::Replace),
            &runner,
            CancellationToken::new(),
            &tool,
            "0.14.0",
        );
        assert!(result.ok, "{result:?}");
        super::super::v13_source_import::tests::assert_another_copy_is_named(&result);

        // Над файловой базой исполнение без согласия человека сначала
        // сверяется с превью и отменяется с вопросом.
        fs::write(
            root.path().join(CONFIG_NAME),
            "format: DESIGNER
infobase:
  connection: 'File=build/ib'
",
        )
        .unwrap();
        let runner = SequenceRunner::new(vec![process(preview)]);
        let refused = execute_with_resolved_runner(
            &prepared_restore(root.path(), false, RestoreMode::Replace),
            &runner,
            CancellationToken::new(),
            &tool,
            "0.14.0",
        );
        assert_eq!(refused.diagnostics[0]["code"], "cancelled", "{refused:?}");
        assert_eq!(
            refused.human_choice.as_deref().expect("a question").code,
            "infobase.another_copy"
        );
    }

    /// rule: INV.RUNTIME.RUNNER-MUTATION-CANCELLATION
    #[test]
    fn restore_detaches_only_its_mutating_call() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(CONFIG_NAME), "format: DESIGNER\n").unwrap();
        fs::create_dir_all(root.path().join("transfer")).unwrap();
        let input = root.path().join("transfer/base.dt");
        fs::write(&input, b"transfer bytes").unwrap();
        let tool = BundledTool {
            program: root.path().join("v8-runner"),
            warnings: Vec::new(),
            missing: None,
        };
        for (operation, dry_run, protected) in [
            (ExportOperation::Restore, false, true),
            (ExportOperation::Restore, true, false),
            (ExportOperation::Configuration, false, false),
        ] {
            let prepared = if operation == ExportOperation::Restore {
                prepared_restore(root.path(), dry_run, RestoreMode::Replace)
            } else {
                prepared(root.path(), dry_run)
            };
            let envelope = if operation == ExportOperation::Restore {
                restore_apply_envelope(&input, "replace", "restored")
            } else {
                json!({"ok":true,"command":"download","data":{}})
            };
            let runner = SequenceRunner::new(vec![process(envelope)]);
            let cancellation = CancellationToken::new();
            let _ = invoke_runner(&prepared, &tool, &runner, &cancellation, dry_run);
            let (_, child) = runner.calls.lock().unwrap()[0]
                .cancellation
                .spawn_with_gate(|| Ok(()))
                .unwrap();
            cancellation.cancel();
            assert_eq!(cancellation.protected_process_started(), protected);
            assert_eq!(child.is_cancelled(), !protected);
        }
    }

    /// rule: INV.RUNTIME.INFOBASE-IMPORT-MODE
    #[test]
    fn restore_apply_refuses_when_the_provider_reports_another_mode() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(CONFIG_NAME), "format: DESIGNER\n").unwrap();
        fs::create_dir_all(root.path().join("transfer")).unwrap();
        fs::write(root.path().join("transfer/base.dt"), b"transfer bytes").unwrap();
        let tool = BundledTool {
            program: root.path().join("v8-runner"),
            warnings: Vec::new(),
            missing: None,
        };
        let prepared = prepared_restore(root.path(), true, RestoreMode::Replace);
        let input = normalize_path_identity(&prepared.arguments.named_file).unwrap();
        // Раннер отвечает про создание, хотя одобряли замену.
        let runner = SequenceRunner::new(vec![process(restore_preview_envelope(&input, "create"))]);

        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool,
            "0.8.1",
        );

        assert!(!result.ok, "{result:?}");
        assert!(
            result.diagnostics[0]["code"] == "invalid_result",
            "{result:?}"
        );
    }

    /// rule: INV.RUNTIME.INFOBASE-IMPORT-MODE
    #[test]
    fn restore_refuses_a_preview_that_claims_it_already_restored() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(CONFIG_NAME), "format: DESIGNER\n").unwrap();
        fs::create_dir_all(root.path().join("transfer")).unwrap();
        fs::write(root.path().join("transfer/base.dt"), b"transfer bytes").unwrap();
        let tool = BundledTool {
            program: root.path().join("v8-runner"),
            warnings: Vec::new(),
            missing: None,
        };
        let prepared = prepared_restore(root.path(), true, RestoreMode::Create);
        let input = normalize_path_identity(&prepared.arguments.named_file).unwrap();
        let mut envelope = restore_preview_envelope(&input, "create");
        envelope["data"]["restored"] = json!(true);
        let runner = SequenceRunner::new(vec![process(envelope)]);

        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool,
            "0.8.1",
        );

        assert!(!result.ok, "{result:?}");
    }

    /// rule: INV.RUNTIME.INFOBASE-IMPORT-MODE
    #[test]
    fn restore_arguments_require_a_stated_mode_and_a_dt_input() {
        let root = tempfile::tempdir().unwrap();
        let context = WorkspaceContext {
            cwd: root.path().to_path_buf(),
            workspace_root: root.path().to_path_buf(),
            cache_root: root.path().join(".build/unica"),
            workspace_epoch: 1,
        };
        let parse = |args: Value| {
            let map = args.as_object().unwrap().clone();
            parse_export_arguments(ExportOperation::Restore, &map, &context)
        };

        // Режим обязателен: умолчания у необратимого действия быть не может.
        assert!(parse(json!({"input": "transfer/base.dt"})).is_err());
        assert!(parse(json!({"input": "transfer/base.dt", "mode": "maybe"})).is_err());
        // Вход обязан быть DT.
        assert!(parse(json!({"input": "transfer/base.cf", "mode": "replace"})).is_err());
        // `output` у загрузки не принимается: она ничего не производит.
        assert!(parse(json!({"output": "transfer/base.dt", "mode": "replace"})).is_err());
        // `connection` не принимается: соединение задаёт v8project.yaml.
        assert!(parse(
            json!({"input": "transfer/base.dt", "mode": "replace", "connection": "File=x"})
        )
        .is_err());

        let ok = parse(json!({"input": "transfer/base.dt", "mode": "create"})).unwrap();
        assert_eq!(ok.restore_mode, Some(RestoreMode::Create));
        assert_eq!(ok.named_file_relative, PathBuf::from("transfer/base.dt"));
    }

    /// rule: INV.RUNTIME.V13-INFOBASE-EXPORTS
    #[test]
    fn preview_is_non_mutating_and_returns_no_revision_or_raw_command() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(CONFIG_NAME), "format: DESIGNER\n").unwrap();
        let prepared = prepared(root.path(), true);
        let output = normalize_path_identity(&prepared.arguments.named_file).unwrap();
        let mut envelope = preview_envelope(&output);
        envelope["data"]["provider"]["skipped"] = json!([{"provider":"ibcmd", "reason": format!("platform resolved at {}", root.path().display())}]);
        let runner = SequenceRunner::new(vec![process(envelope)]);
        let tool = BundledTool {
            program: root.path().join("v8-runner"),
            warnings: Vec::new(),
            missing: None,
        };

        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool,
            "0.7.0",
        );

        assert!(result.ok, "{result:?}");
        assert!(result.rev.is_none());
        assert!(result.next[0]["args"].get("ifRev").is_none());
        assert_eq!(
            result.data.as_ref().unwrap()["plan"]["provider"],
            "designer"
        );
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
        assert!(runner.calls.lock().unwrap()[0]
            .args
            .contains(&"--dry-run".to_string()));
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("--config"));
        assert!(!encoded.contains(&root.path().display().to_string()));
        assert!(!prepared.arguments.named_file.exists());
    }

    /// rule: INV.RUNTIME.V13-INFOBASE-EXPORTS
    #[test]
    fn apply_runs_the_runner_once_and_returns_an_independent_file_receipt() {
        for published in [Some(&b"verified cf"[..]), None, Some(&b""[..])] {
            let root = tempfile::tempdir().unwrap();
            fs::write(root.path().join(CONFIG_NAME), "format: DESIGNER\n").unwrap();
            let preview = prepared(root.path(), true);
            let output = normalize_path_identity(&preview.arguments.named_file).unwrap();
            let tool = BundledTool {
                program: root.path().join("v8-runner"),
                warnings: Vec::new(),
                missing: None,
            };
            let preview_runner = SequenceRunner::new(vec![process(preview_envelope(&output))]);
            let preview_result = execute_with_resolved_runner(
                &preview,
                &preview_runner,
                CancellationToken::new(),
                &tool,
                "0.7.0",
            );
            assert!(preview_result.ok);
            assert!(preview_result.rev.is_none());
            let apply = prepared(root.path(), false);
            let responses = vec![process(apply_envelope(&output))];
            let runner = match published {
                Some(bytes) => SequenceRunner::publishing(responses, bytes),
                None => SequenceRunner::new(responses),
            };

            let result = execute_with_resolved_runner(
                &apply,
                &runner,
                CancellationToken::new(),
                &tool,
                "0.7.0",
            );

            assert_eq!(runner.calls.lock().unwrap().len(), 1);
            assert!(!runner.calls.lock().unwrap()[0]
                .args
                .contains(&"--dry-run".to_string()));
            if published.is_none_or(|bytes| bytes.is_empty()) {
                assert!(!result.ok, "{result:?}");
                assert_eq!(result.diagnostics[0]["code"], "invalid_result");
                let reason = if published.is_none() {
                    "missing"
                } else {
                    "empty"
                };
                assert!(
                    result.diagnostics[0]["message"]
                        .as_str()
                        .is_some_and(|message| message.contains(reason)),
                    "{result:?}"
                );
                continue;
            }
            assert!(result.ok, "{result:?}");
            assert_eq!(result.data.as_ref().unwrap()["artifact"]["size"], 11);
            assert_eq!(
                result.data.as_ref().unwrap()["artifact"]["sha256"],
                "244e79d203c7fa3c3ac5213f4ef8b6cb3fc0894514864bb337b6a4eb2c5a678b"
            );
            assert_eq!(result.artifacts[0]["path"], "dist/main.cf");
            let encoded = serde_json::to_string(&result).unwrap();
            assert!(!encoded.contains("--config"));
            assert!(!encoded.contains("stdout"));
        }
    }

    /// rule: INV.RUNTIME.V13-INFOBASE-EXPORTS
    #[test]
    fn direct_export_executes_without_a_previous_preview_or_revision() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(CONFIG_NAME), "format: DESIGNER\n").unwrap();
        let apply = prepared(root.path(), false);
        let output = normalize_path_identity(&apply.arguments.named_file).unwrap();
        let tool = BundledTool {
            program: root.path().join("v8-runner"),
            warnings: vec![],
            missing: None,
        };
        let runner =
            SequenceRunner::publishing(vec![process(apply_envelope(&output))], b"verified cf");
        assert!(!apply.arguments.named_file.exists());
        let result =
            execute_with_resolved_runner(&apply, &runner, CancellationToken::new(), &tool, "0.7.0");
        assert!(result.ok, "{result:?}");
        assert!(result.rev.is_none());
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
        assert_eq!(
            fs::read(&apply.arguments.named_file).unwrap(),
            b"verified cf"
        );
        assert_eq!(result.data.as_ref().unwrap()["artifact"]["size"], 11);
    }

    /// Раннер 0.12 называет в квитанции применения точку входа сессии агента:
    /// годная точка входа принимается, неверная — негодный ответ. Исполнение
    /// не повторяет preview, поэтому исполнитель берётся из этой квитанции.
    #[test]
    fn apply_receipt_accepts_a_session_endpoint_and_rejects_a_malformed_one() {
        let root = tempfile::tempdir().unwrap();
        let prepared = prepared(root.path(), false);
        let output = normalize_path_identity(&prepared.arguments.named_file).unwrap();
        for mode in ["managed", "attached", "gate"] {
            let mut applied = apply_envelope(&output);
            applied["data"]["provider"]["endpoint"] =
                json!({"mode": mode, "address": "127.0.0.1:1543"});
            assert_eq!(
                validate_apply(&prepared, &applied).as_deref(),
                Ok("designer"),
                "{mode}"
            );
        }
        // Раннер 0.17 у управляемого агента называет, был ли агент живым.
        for live in [
            json!({"reused": true, "kept": true,
                   "holders": [{"pid": 4242, "started_at": "2026-11-13T08:00:00Z"}]}),
            json!({"reused": false, "kept": false, "holders": [], "reason": "job"}),
        ] {
            let mut applied = apply_envelope(&output);
            applied["data"]["provider"]["endpoint"] =
                json!({"mode": "managed", "address": "127.0.0.1:1543", "live": live});
            assert_eq!(
                validate_apply(&prepared, &applied).as_deref(),
                Ok("designer"),
                "{live}"
            );
        }
        for endpoint in [
            json!({"mode": "remote", "address": "127.0.0.1:1543"}),
            json!({"mode": "managed", "address": ""}),
            json!({"mode": "managed", "address": "h:1", "user": "admin"}),
            json!("127.0.0.1:1543"),
            json!({"mode": "attached", "address": "h:1",
                   "live": {"reused": true, "kept": true, "holders": []}}),
            json!({"mode": "managed", "address": "h:1", "live": {"kept": true}}),
            json!({"mode": "managed", "address": "h:1",
                   "live": {"reused": true, "kept": true, "holders": [], "pid": 1}}),
        ] {
            let mut applied = apply_envelope(&output);
            applied["data"]["provider"]["endpoint"] = endpoint.clone();
            assert!(validate_apply(&prepared, &applied).is_err(), "{endpoint}");
        }
    }

    /// Раннер 0.14 без имени набора обходит все наборы в каталог и путь к одному
    /// файлу отклоняет (снято вживую: `download --output out/main.cf` —
    /// `invalid_argument`). Основная конфигурация заказывается по имени её набора;
    /// пакет расширения — по `--extension`, без набора.
    #[test]
    fn download_of_the_main_configuration_names_its_source_set() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join(CONFIG_NAME),
            "format: DESIGNER\nsource-set:\n  - name: ext\n    type: EXTENSION\n    path: ext\n  - name: Основная\n    type: configuration\n    path: src\n",
        )
        .unwrap();
        let prepared = prepared(root.path(), true);
        let output = normalize_path_identity(&prepared.arguments.named_file).unwrap();
        let tool = BundledTool {
            program: root.path().join("v8-runner"),
            warnings: Vec::new(),
            missing: None,
        };
        let runner = SequenceRunner::new(vec![process(preview_envelope(&output))]);
        invoke_runner(&prepared, &tool, &runner, &CancellationToken::new(), true).unwrap();
        let args = runner.calls.lock().unwrap()[0].args.clone();
        assert_eq!(args[3..5], ["download".to_string(), "Основная".to_string()]);

        fs::write(
            root.path().join(CONFIG_NAME),
            "source-set:\n  - name: a\n    type: CONFIGURATION\n    path: a\n  - name: b\n    type: CONFIGURATION\n    path: b\n",
        )
        .unwrap();
        let runner = SequenceRunner::new(vec![]);
        let refused =
            invoke_runner(&prepared, &tool, &runner, &CancellationToken::new(), true).unwrap_err();
        assert_eq!(refused.diagnostics[0]["code"], "invalid_state");
        assert!(runner.calls.lock().unwrap().is_empty());

        // Только наборы расширений: без имени набора раннер обошёл бы их в
        // каталог и путь к файлу отклонил бы — отказ до его запуска.
        fs::write(
            root.path().join(CONFIG_NAME),
            "source-set:\n  - name: ext\n    type: EXTENSION\n    path: ext\n",
        )
        .unwrap();
        let refused =
            invoke_runner(&prepared, &tool, &runner, &CancellationToken::new(), true).unwrap_err();
        assert_eq!(refused.diagnostics[0]["code"], "unsupported_operation");
        assert!(runner.calls.lock().unwrap().is_empty());

        // Проект без наборов пакетов: файл основной конфигурации без имени.
        fs::write(root.path().join(CONFIG_NAME), "format: DESIGNER\n").unwrap();
        let runner = SequenceRunner::new(vec![process(preview_envelope(&output))]);
        invoke_runner(&prepared, &tool, &runner, &CancellationToken::new(), true).unwrap();
        assert_eq!(runner.calls.lock().unwrap()[0].args[4], "--output");
    }

    /// Рабочее состояние раннер 0.12 выгружает без ключа: `--state working` у него
    /// скрытый синоним прежнего словаря, и адаптер его не произносит.
    #[test]
    fn working_state_download_uses_no_hidden_runner_synonym() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(CONFIG_NAME), "format: DESIGNER\n").unwrap();
        let prepared = prepared(root.path(), true);
        let output = normalize_path_identity(&prepared.arguments.named_file).unwrap();
        let tool = BundledTool {
            program: root.path().join("v8-runner"),
            warnings: Vec::new(),
            missing: None,
        };
        let runner = SequenceRunner::new(vec![process(preview_envelope(&output))]);
        invoke_runner(&prepared, &tool, &runner, &CancellationToken::new(), true).unwrap();
        let args = runner.calls.lock().unwrap()[0].args.clone();
        assert_eq!(
            args[3..],
            [
                "download".to_string(),
                "--output".to_string(),
                prepared.arguments.named_file.display().to_string(),
                "--dry-run".to_string(),
            ]
        );
    }

    /// «нет движка» — только когда процесс раннера не возник.
    /// Настоящий запуск несуществующего файла даёт сбой запуска, и отказ
    /// называет шаг `start` вместе с причиной ОС, но без пути к движку. Сбой
    /// после старта движок не теряет: свой код, без полей движка, причина
    /// сохранена.
    /// rule: INV.WIRE.MISSING-ENGINE-REFUSAL
    #[test]
    fn only_a_failed_launch_of_the_runner_is_a_missing_engine() {
        use unica_process::runner::SystemProcessRunner;
        let root = tempfile::tempdir().unwrap();
        let program = root.path().join("private-dir/v8-runner-that-is-not-there");
        let failure = SystemProcessRunner
            .run_staged(&ProcessCommand {
                program: program.clone(),
                args: Vec::new(),
                cwd: root.path().to_path_buf(),
                env: Vec::new(),
                env_remove: Vec::new(),
                capture_limits: None,
                timeout: None,
                cancellation: CancellationToken::new(),
            })
            .expect_err("a file that is not there cannot start");
        assert!(
            matches!(failure, ProcessStreamFailure::Spawn(_)),
            "{failure:?}"
        );

        let absent = runner_start_rejection(Some("download".into()), &failure, &program);
        let diagnostic = &absent.diagnostics[0];
        assert_eq!(diagnostic["code"], "provider_unavailable");
        assert_eq!(diagnostic["detailCode"], "provider_absent");
        let data = absent.data.as_ref().expect("поля движка в data");
        assert_eq!(data["tool"], "v8-runner");
        assert_eq!(data["stage"], "start");
        assert_eq!(data["retryable"], false);
        assert_eq!(data["path"], json!(program.display().to_string()));
        let message = diagnostic["message"].as_str().unwrap();
        assert!(
            message.starts_with("bundled_tool_missing: v8-runner is not available for "),
            "{message}"
        );
        assert!(
            message.contains("; cause: "),
            "причина ОС сохранена: {message}"
        );
        assert!(!message.contains("private-dir"), "{message}");

        let after_start = runner_start_rejection(
            Some("download".into()),
            &ProcessStreamFailure::Run(format!(
                "process_failed: broken pipe while reading {} output",
                program.display()
            )),
            &program,
        );
        let diagnostic = &after_start.diagnostics[0];
        assert_eq!(diagnostic["code"], "provider_failed");
        assert!(diagnostic.get("detailCode").is_none(), "{after_start:?}");
        assert!(after_start.data.is_none(), "{after_start:?}");
        let message = diagnostic["message"].as_str().unwrap();
        assert!(
            message.contains("broken pipe"),
            "причина сохранена: {message}"
        );
        assert!(!message.contains("private-dir"), "{message}");
    }

    /// rule: INV.RUNTIME.V13-INFOBASE-EXPORTS
    #[test]
    fn apply_rejects_a_runner_receipt_for_a_different_output() {
        let root = tempfile::tempdir().unwrap();
        let prepared = prepared(root.path(), false);
        let output = normalize_path_identity(&prepared.arguments.named_file).unwrap();
        let mut applied = apply_envelope(&output);
        applied["data"]["output"] = json!(root.path().join("dist/other.cf"));

        let rejection = validate_apply(&prepared, &applied).unwrap_err();

        assert_eq!(rejection.diagnostics[0]["code"], "invalid_result");

        // Правильный путь не делает ответ другой команды нашей квитанцией.
        for dry_run in [true, false] {
            let envelope = if dry_run {
                preview_envelope(&output)
            } else {
                apply_envelope(&output)
            };
            parse_runner_output(prepared.operation, process(envelope.clone()), dry_run)
                .expect("the requested runner command is accepted");
            for other_command in ["infobase.dump", "cf.export"] {
                let mut foreign = envelope.clone();
                foreign["command"] = json!(other_command);
                let rejection =
                    parse_runner_output(prepared.operation, process(foreign), dry_run).unwrap_err();
                assert_eq!(
                    rejection.diagnostics[0]["code"], "invalid_result",
                    "foreign command {other_command}, dry_run={dry_run}"
                );
            }
        }
    }

    #[test]
    fn preview_accepts_a_new_bounded_runner_provider_without_mcp_change() {
        let root = tempfile::tempdir().unwrap();
        let prepared = prepared(root.path(), true);
        let output = normalize_path_identity(&prepared.arguments.named_file).unwrap();
        let mut envelope = preview_envelope(&output);
        envelope["data"]["plan"]["provider"] = json!("ibcmd-rs");
        envelope["data"]["provider"]["selected"] = json!("ibcmd-rs");

        let plan = validate_preview(&prepared, &envelope).unwrap();

        assert_eq!(plan.provider, "ibcmd-rs");
    }

    /// rule: INV.RUNTIME.V13-INFOBASE-EXPORTS
    #[test]
    fn cfe_and_dt_invocations_use_only_their_closed_runner_arguments() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(CONFIG_NAME), "format: DESIGNER\n").unwrap();
        let tool = BundledTool {
            program: root.path().join("v8-runner"),
            warnings: Vec::new(),
            missing: None,
        };
        let context = WorkspaceContext {
            cwd: root.path().to_path_buf(),
            workspace_root: root.path().to_path_buf(),
            cache_root: root.path().join(".build/unica"),
            workspace_epoch: 1,
        };
        let cfe_args = Map::from_iter([
            ("state".to_string(), json!("database")),
            ("extension".to_string(), json!("SalesAddon")),
            ("output".to_string(), json!("dist/sales.cfe")),
        ]);
        let cfe = PreparedInfobaseExport {
            call: Map::new(),
            operation: ExportOperation::Configuration,
            arguments: parse_export_arguments(ExportOperation::Configuration, &cfe_args, &context)
                .unwrap(),
            dry_run: true,
            context: context.clone(),
        };
        let cfe_output = normalize_path_identity(&cfe.arguments.named_file).unwrap();
        let cfe_runner = SequenceRunner::new(vec![process(preview_envelope(&cfe_output))]);
        invoke_runner(&cfe, &tool, &cfe_runner, &CancellationToken::new(), true).unwrap();
        assert_eq!(
            cfe_runner.calls.lock().unwrap()[0].args,
            vec![
                "--config",
                cfe.context
                    .workspace_root
                    .join(CONFIG_NAME)
                    .to_str()
                    .unwrap(),
                "--json-message",
                "download",
                "--state",
                "db",
                "--extension",
                "SalesAddon",
                "--output",
                cfe.arguments.named_file.to_str().unwrap(),
                "--dry-run"
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
        );

        let dt_args = Map::from_iter([("output".to_string(), json!("dist/base.dt"))]);
        let dt = PreparedInfobaseExport {
            call: Map::new(),
            operation: ExportOperation::Infobase,
            arguments: parse_export_arguments(ExportOperation::Infobase, &dt_args, &context)
                .unwrap(),
            dry_run: true,
            context,
        };
        let dt_output = normalize_path_identity(&dt.arguments.named_file).unwrap();
        let mut dt_envelope = preview_envelope(&dt_output);
        dt_envelope["command"] = json!("infobase.dump");
        dt_envelope["data"]["artifact_kind"] = json!("dt");
        dt_envelope["data"]["subject"] = json!({"kind": "infobase"});
        let dt_runner = SequenceRunner::new(vec![process(dt_envelope)]);
        invoke_runner(&dt, &tool, &dt_runner, &CancellationToken::new(), true).unwrap();
        assert_eq!(
            dt_runner.calls.lock().unwrap()[0].args,
            vec![
                "--config",
                dt.context
                    .workspace_root
                    .join(CONFIG_NAME)
                    .to_str()
                    .unwrap(),
                "--json-message",
                "infobase",
                "dump",
                "--output",
                dt.arguments.named_file.to_str().unwrap(),
                "--dry-run"
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn unavailable_preview_returns_one_provider_diagnostic_without_mutation() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(CONFIG_NAME), "format: DESIGNER\n").unwrap();
        let prepared = prepared(root.path(), true);
        let runner = SequenceRunner::new(vec![failed_process(json!({
            "command": "download",
            "data": {
                "mode": "preview",
                "provider_dispatched": false,
                "published": false,
                "target_state": "unchanged",
                "execution": {"status": "failed"}
            },
            "error": {
                "code": "environment_unavailable",
                "message": "full platform or ibcmd is not available"
            }
        }))]);
        let tool = BundledTool {
            program: root.path().join("v8-runner"),
            warnings: Vec::new(),
            missing: None,
        };

        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool,
            "0.7.0",
        );

        assert!(!result.ok);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
        assert!(!prepared.arguments.named_file.exists());
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("provider details remain private"));
    }

    fn restore_workspace() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(CONFIG_NAME), "format: DESIGNER\n").unwrap();
        fs::create_dir_all(root.path().join("transfer")).unwrap();
        fs::write(root.path().join("transfer/base.dt"), b"transfer bytes").unwrap();
        fs::write(root.path().join("transfer/other.dt"), b"other bytes").unwrap();
        root
    }

    fn restore_request(root: &Path, arguments: Value) -> InvocationRequest {
        InvocationRequest::new(
            ToolIdentity::Run,
            arguments,
            root.display().to_string(),
            7000,
        )
        .unwrap()
    }

    /// Отказ раннера 0.15 на `infobase restore --create`, когда база уже есть:
    /// код проверки и проза, по которой Unica его опознаёт.
    fn restore_target_exists_envelope(dry_run: bool) -> Value {
        json!({
            "command": "infobase.restore",
            "data": {
                "mode": if dry_run { "preview" } else { "apply" },
                "provider_dispatched": if dry_run { json!(false) } else { Value::Null },
                "subject": "infobase",
                "artifact_kind": "dt",
                "target_mode": "create",
                "restored": false,
                "target_state": "unchanged",
                "execution": {"status": "failed"}
            },
            "error": {
                "code": "invalid_argument",
                "message": "--create was requested but the target infobase already exists; pass --replace to discard its data"
            }
        })
    }

    /// Исполняемый `infobase.restore` с `mode: replace` идёт только с согласия
    /// человека. Без него — отказ `cancelled` с вопросом-подтверждением по
    /// согласованному тексту: база и имя DT без пути. Подтверждение годится
    /// только для того же вызова. Превью и `mode: create` не спрашивают.
    /// rule: INV.RUNTIME.DESTRUCTIVE-CONFIRMATION
    /// rule: INV.MCP.HUMAN-CHOICE
    #[test]
    fn an_executing_restore_with_replace_runs_only_with_the_persons_confirmation() {
        let root = restore_workspace();
        let call = json!({
            "op": "infobase.restore",
            "args": {"input": "transfer/base.dt", "mode": "replace"},
            "dryRun": false
        });

        let refused = PreparedInfobaseExport::parse(
            &restore_request(root.path(), call.clone()),
            ExportOperation::Restore,
        )
        .expect_err("replace without a confirmation is not executed");
        assert_eq!(refused.diagnostics[0]["code"], "cancelled");
        let choice = refused.human_choice.as_deref().expect("a confirmation");
        // Надписи согласованы: менять их — только вместе с этим тестом.
        assert_eq!(choice.code, "infobase.restore_replace");
        assert_eq!(
            choice.question,
            "Заменить данные базы «origin» данными из «base.dt»?"
        );
        assert!(choice.proceeds_unasked);
        assert_eq!(choice.options.len(), 1);
        assert_eq!(choice.options[0].title, "Заменить данные");
        assert_eq!(
            choice.options[0].description,
            "данные и конфигурация базы пропадут"
        );
        assert_eq!(
            choice.data,
            Some(json!({"infobase": "origin", "file": "base.dt", "input": "transfer/base.dt"}))
        );
        assert_eq!(
            Value::Object(unica_execution_contract::invocation::without_confirmation(
                &choice.options[0].arguments
            )),
            call
        );

        let confirmed = choice.options[0].arguments.clone();
        let executed = PreparedInfobaseExport::parse(
            &restore_request(root.path(), Value::Object(confirmed.clone())),
            ExportOperation::Restore,
        )
        .expect("a confirmed replace is executed");
        assert_eq!(executed.arguments.restore_mode, Some(RestoreMode::Replace));
        assert!(!executed.dry_run);

        // Согласие на один файл не загружает в базу другой.
        let mut other = confirmed;
        other["args"]["input"] = json!("transfer/other.dt");
        assert!(PreparedInfobaseExport::parse(
            &restore_request(root.path(), Value::Object(other)),
            ExportOperation::Restore,
        )
        .is_err());

        for unasked in [
            json!({"op": "infobase.restore", "args": {"input": "transfer/base.dt", "mode": "replace"}, "dryRun": true}),
            json!({"op": "infobase.restore", "args": {"input": "transfer/base.dt", "mode": "create"}, "dryRun": false}),
        ] {
            PreparedInfobaseExport::parse(
                &restore_request(root.path(), unasked.clone()),
                ExportOperation::Restore,
            )
            .unwrap_or_else(|refused| panic!("{unasked} is not asked: {refused:?}"));
        }
    }

    /// Исполняемый `mode: create` над базой, которая уже есть, отказывает
    /// вопросом человеку с согласованными надписями. Вариант — тот же вызов с
    /// `mode: replace`, и он уже несёт согласие на замену данных. Превью и
    /// другой отказ раннера вопроса не несут.
    /// rule: INV.RUNTIME.INFOBASE-IMPORT-MODE
    /// rule: INV.MCP.HUMAN-CHOICE
    #[test]
    fn restore_create_over_an_existing_infobase_offers_to_replace_its_data() {
        let root = restore_workspace();
        let tool = BundledTool {
            program: root.path().join("v8-runner"),
            warnings: Vec::new(),
            missing: None,
        };
        {
            let prepared = prepared_restore(root.path(), false, RestoreMode::Create);
            let runner =
                SequenceRunner::new(vec![failed_process(restore_target_exists_envelope(false))]);
            let result = execute_with_resolved_runner(
                &prepared,
                &runner,
                CancellationToken::new(),
                &tool,
                "0.17.1",
            );
            assert!(!result.ok, "{result:?}");
            let choice = result.human_choice.as_deref().expect("a question");
            // Надписи согласованы: менять их — только вместе с этим тестом.
            assert_eq!(choice.code, "infobase.restore_exists");
            assert_eq!(choice.question, "База «origin» уже существует.");
            assert!(!choice.proceeds_unasked);
            assert_eq!(choice.options.len(), 1);
            let option = &choice.options[0];
            assert_eq!(option.title, "Заменить данные базы данными из «base.dt»");
            assert_eq!(option.description, "данные и конфигурация базы пропадут");
            assert_eq!(option.tool, "unica.run");
            assert_eq!(
                choice.data,
                Some(json!({"infobase": "origin", "file": "base.dt", "input": "transfer/base.dt"}))
            );
            let replace = json!({
                "op": "infobase.restore",
                "args": {"input": "transfer/base.dt", "mode": "replace"},
                "dryRun": false
            });
            assert_eq!(
                Value::Object(unica_execution_contract::invocation::without_confirmation(
                    &option.arguments
                )),
                replace
            );
            // Выбор варианта уже согласие на замену: второй вопрос не задаётся.
            assert!(HumanChoice::is_confirmed(
                &restore_replace_question("base.dt"),
                "unica.run",
                &option.arguments
            ));
            PreparedInfobaseExport::parse(
                &restore_request(root.path(), Value::Object(option.arguments.clone())),
                ExportOperation::Restore,
            )
            .expect("the chosen call is admitted without a second question");
        }

        // Превью спрашивать не может: оно отдаёт прежний отказ.
        let preview = execute_with_resolved_runner(
            &prepared_restore(root.path(), true, RestoreMode::Create),
            &SequenceRunner::new(vec![failed_process(restore_target_exists_envelope(true))]),
            CancellationToken::new(),
            &tool,
            "0.17.1",
        );
        assert!(!preview.ok);
        assert!(preview.human_choice.is_none(), "{preview:?}");

        let prepared = prepared_restore(root.path(), false, RestoreMode::Create);
        let mut other = restore_target_exists_envelope(false);
        other["error"]["message"] =
            json!("--input 'transfer/base.dt' is not a non-empty regular file");
        let runner = SequenceRunner::new(vec![failed_process(other)]);
        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool,
            "0.17.1",
        );
        assert!(!result.ok);
        assert!(result.human_choice.is_none(), "{result:?}");
    }
}
