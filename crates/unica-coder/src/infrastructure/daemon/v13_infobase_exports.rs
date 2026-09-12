#![allow(clippy::result_large_err)]
// Internal failures are immediately returned as the canonical wire result; keeping
// that value intact avoids a second error model at this adapter boundary.

use super::protocol::InvocationRequest;
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
use crate::infrastructure::platform::filesystem::{
    open_absolute_directory_path_nofollow, open_directory_child_nofollow,
    open_regular_child_nofollow,
};
use crate::infrastructure::plugin_runtime::find_plugin_root;
use crate::infrastructure::redaction::redactor;
use crate::infrastructure::source_roots::normalize_path_identity;
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const CONFIG_NAME: &str = "v8project.yaml";
const LOCAL_CONFIG_NAME: &str = "v8project.local.yaml";
const RUNNER_OUTPUT_LIMIT: usize = 1024 * 1024;

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
            "infobase.configuration.export" => Some(Self::Configuration),
            "infobase.dump" => Some(Self::Infobase),
            "infobase.restore" => Some(Self::Restore),
            _ => None,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Configuration => "infobase.configuration.export",
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
    if_rev: Option<String>,
    context: WorkspaceContext,
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
                        "{} requires dryRun: true to preview or dryRun: false with ifRev to apply",
                        operation.name()
                    ),
                )
            })?;
        let if_rev = match arguments.get("ifRev") {
            None => None,
            Some(Value::String(value)) if !value.trim().is_empty() => Some(value.clone()),
            Some(_) => {
                return Err(reject(
                    operation,
                    RefusalCode::BadValue,
                    format!("{} ifRev must be non-empty text", operation.name()),
                ))
            }
        };
        if dry_run && if_rev.is_some() {
            return Err(reject(
                operation,
                RefusalCode::BadValue,
                format!(
                    "{} preview does not accept ifRev; apply the revision returned by this preview",
                    operation.name()
                ),
            ));
        }
        if !dry_run && if_rev.is_none() {
            return Err(reject(
                operation,
                RefusalCode::BadValue,
                format!(
                    "{} apply requires ifRev from a prior dryRun preview",
                    operation.name()
                ),
            ));
        }

        let context =
            discover_workspace(Some(PathBuf::from(request.workspace_hint()))).map_err(|error| {
                reject(
                    operation,
                    RefusalCode::ProviderUnavailable,
                    format!("workspace discovery failed: {error}"),
                )
            })?;
        let arguments = parse_export_arguments(operation, args, &context)?;
        Ok(Self {
            operation,
            arguments,
            dry_run,
            if_rev,
            context,
        })
    }

    pub(super) fn workspace_identity_hash(&self) -> SafeIdentityHash {
        let mut hasher = Sha256::new();
        hasher.update(b"unica-v13-infobase-export-workspace-v1\0");
        hasher.update(self.context.workspace_root.as_os_str().as_encoded_bytes());
        SafeIdentityHash::from_sha256(hasher.finalize().into())
    }

    pub(super) fn execute(&self, cancellation: CancellationToken) -> DomainResult {
        execute_with_runner(self, &SystemProcessRunner, cancellation)
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
                    "infobase.configuration.export state must be `working` or `database`",
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
                "infobase.configuration.export extension must be a non-empty 1C identifier",
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

fn closed_workspace_relative_path(value: &str) -> Result<PathBuf, &'static str> {
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

fn valid_1c_identifier(value: &str) -> bool {
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

fn digest_required_workspace_file(root: &Path, relative: &Path) -> Result<String, String> {
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

fn digest_optional_workspace_file(
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
    let mut directory = open_absolute_directory_path_nofollow(&root)
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
            open_regular_child_nofollow(&directory, name)
        } else {
            match open_directory_child_nofollow(&directory, name) {
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
    let plugin_root = match find_plugin_root(&prepared.context.cwd) {
        Some(root) => root,
        None => {
            return reject(
                prepared.operation,
                RefusalCode::ProviderUnavailable,
                "Unica plugin root could not be located for the bundled v8-runner",
            )
        }
    };
    let tool = match resolve_bundled_tool(&plugin_root, "v8-runner", true) {
        Ok(tool) => tool,
        Err(error) => {
            return reject(
                prepared.operation,
                RefusalCode::ProviderUnavailable,
                redactor(&error),
            )
        }
    };
    let runner_version = match bundled_tool_version(&plugin_root, "v8-runner") {
        Ok(version) => version,
        Err(error) => {
            return reject(
                prepared.operation,
                RefusalCode::ProviderUnavailable,
                redactor(&error),
            )
        }
    };
    execute_with_resolved_runner(prepared, runner, cancellation, &tool, &runner_version)
}

fn execute_with_resolved_runner(
    prepared: &PreparedInfobaseExport,
    runner: &dyn ProcessRunner,
    cancellation: CancellationToken,
    tool: &BundledTool,
    runner_version: &str,
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
    let revision = plan_revision(prepared, &before, runner_version, &plan);
    if prepared.dry_run {
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
        result.rev = Some(revision.clone());
        result.next.push(json!({
            "tool": "unica.run",
            "args": {
                "op": prepared.operation.name(),
                "args": public_arguments(prepared),
                "dryRun": false,
                "ifRev": revision,
            },
            "reason": "apply exactly this previewed export plan"
        }));
        return result;
    }
    if prepared.if_rev.as_deref() != Some(revision.as_str()) {
        return reject(
            prepared.operation,
            RefusalCode::RevisionMismatch,
            format!(
                "{} plan or environment changed after preview; run dryRun: true again",
                prepared.operation.name()
            ),
        );
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
    let applied = match invoke_runner(prepared, tool, runner, &cancellation, false) {
        Ok(envelope) => envelope,
        Err(result) => return result,
    };
    if let Err(result) = validate_apply(prepared, &plan, &applied) {
        return result;
    }
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
            "provider": plan.provider,
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
        result.rev = Some(revision);
        return result;
    }
    let mut result = DomainResult::success(format!(
        "{} exported and independently verified",
        prepared.operation.name()
    ));
    result.data = Some(json!({
        "op": prepared.operation.name(),
        "dryRun": false,
        "provider": plan.provider,
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
    result.rev = Some(revision);
    result
}

fn invoke_runner(
    prepared: &PreparedInfobaseExport,
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
        "infobase".to_string(),
    ];
    match prepared.operation {
        ExportOperation::Configuration => {
            args.extend(["configuration".to_string(), "export".to_string()]);
            args.extend([
                "--state".to_string(),
                prepared
                    .arguments
                    .state
                    .clone()
                    .expect("configuration state"),
            ]);
            if let Some(extension) = &prepared.arguments.extension {
                args.extend(["--extension".to_string(), extension.clone()]);
            }
        }
        ExportOperation::Infobase => args.push("dump".to_string()),
        ExportOperation::Restore => args.push("restore".to_string()),
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
    let output = runner.run(&ProcessCommand {
        program: tool.program.clone(),
        args,
        cwd: prepared.context.workspace_root.clone(),
        env: Vec::new(),
        env_remove: Vec::new(),
        capture_limits: Some((RUNNER_OUTPUT_LIMIT, RUNNER_OUTPUT_LIMIT)),
        timeout: None,
        cancellation: cancellation.clone(),
    });
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            return Err(reject(
                prepared.operation,
                RefusalCode::ProviderUnavailable,
                format!("failed to start bundled v8-runner: {}", redactor(&error)),
            ))
        }
    };
    parse_runner_output(prepared.operation, output, dry_run)
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
    if envelope["command"] != operation.name() {
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
        return Err(reject(operation, map_runner_code(code), message));
    }
    Ok(envelope)
}

/// Полный словарь кодов, которые раннер кладёт в `error.code`.
///
/// Он закрытый и короткий: девять значений из `cli_error_contract` раннера, одни и те
/// же у CLI и у его MCP-поверхности. Набор существует только для стража ниже: в
/// продуктовом пути отображение обязано иметь запасную ветку на случай кода, которого
/// мы ещё не знаем, поэтому сам список ему не нужен.
#[cfg(test)]
const RUNNER_WIRE_CODES: [&str; 9] = [
    "capability_unavailable",
    "cancelled",
    "environment_unavailable",
    "invalid_argument",
    "invalid_output",
    "platform_failure",
    "runtime_failure",
    "timed_out",
    "workspace_busy",
];

/// Отображает код раннера в наш отказ, а значит и в исход для агента.
///
/// **Платформа отказала — это «нужен человек», и дальше мы не разбираем.** Отказ
/// авторизации, отсутствие права и отсутствие лицензии приходят от платформы одним
/// `platform_failure`, и различать их Unica не должна: ни один из трёх не решается
/// ни повтором, ни правкой вызова — нужен человек. Поэтому прав мы заранее не
/// проверяем: факта отказа платформы достаточно.
fn map_runner_code(code: &str) -> RefusalCode {
    match code {
        // Платформа сказала нет: авторизация, права, лицензия, занятая база.
        "platform_failure" => RefusalCode::ProviderUnavailable,
        // Бинарник, версия или соединение не готовы.
        "environment_unavailable" => RefusalCode::ProviderUnavailable,
        // У раннера нет адаптера под эту операцию: повторять и править вызов незачем.
        "capability_unavailable" => RefusalCode::UnsupportedOperation,
        "workspace_busy" => RefusalCode::ConcurrentChange,
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
    output: String,
    reason: String,
    selection: Value,
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
    let selection_provider = data["selection"]["provider"].as_str();
    let reason = data["selection"]["reason"]
        .as_str()
        .filter(|reason| reason.len() <= 1024)
        .ok_or_else(|| {
            reject(
                prepared.operation,
                RefusalCode::InvalidResult,
                "v8-runner preview omitted a bounded provider-selection reason",
            )
        })?
        .to_string();
    let candidates = data["selection"]["candidates"]
        .as_array()
        .filter(|candidates| !candidates.is_empty() && candidates.len() <= 8)
        .ok_or_else(|| {
            reject(
                prepared.operation,
                RefusalCode::InvalidResult,
                "v8-runner preview returned an invalid provider candidate set",
            )
        })?;
    let candidates_valid = candidates.iter().all(|candidate| {
        candidate["provider"]
            .as_str()
            .is_some_and(valid_provider_id)
            && matches!(
                candidate["implementation"].as_str(),
                Some("implemented" | "experimental" | "unsupported")
            )
            && matches!(
                candidate["readiness"].as_str(),
                Some("ready" | "unavailable" | "not_checked")
            )
            && matches!(
                candidate["evidence"].as_str(),
                Some("documented" | "argv_tested" | "live_verified")
            )
            && candidate["reason"]
                .as_str()
                .is_some_and(|reason| reason.len() <= 1024)
    });
    let selected_ready = candidates.iter().any(|candidate| {
        candidate["provider"].as_str() == Some(provider.as_str())
            && candidate["implementation"] == "implemented"
            && candidate["readiness"] == "ready"
    });
    if selection_provider != Some(provider.as_str()) || !candidates_valid || !selected_ready {
        return Err(reject(
            prepared.operation,
            RefusalCode::InvalidResult,
            "v8-runner preview returned an inconsistent provider selection",
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
    Ok(PreviewPlan {
        provider,
        output,
        reason,
        selection: data["selection"].clone(),
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
    preview: &PreviewPlan,
    envelope: &Value,
) -> Result<(), DomainResult> {
    let data = &envelope["data"];
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
        || data["selection"]["provider"] != preview.provider
        || !matches!(data["target_state"].as_str(), Some("created" | "replaced"))
        || !runner_subject_matches(prepared, data)
    {
        return Err(reject(
            prepared.operation,
            RefusalCode::InvalidResult,
            "v8-runner apply result does not match the previewed export contract",
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
    if applied_output != expected_output || output != preview.output {
        return Err(reject(
            prepared.operation,
            RefusalCode::InvalidResult,
            "v8-runner apply published a different output than the previewed target",
        ));
    }
    Ok(())
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

fn plan_revision(
    prepared: &PreparedInfobaseExport,
    inputs: &StableInputs,
    runner_version: &str,
    plan: &PreviewPlan,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"unica-v13-infobase-export-plan-v1\0");
    hasher.update(
        serde_json::to_vec(&json!({
            "op": prepared.operation.name(),
            "args": public_arguments(prepared),
            "inputs": {
                "config": inputs.config_sha256,
                "localConfig": inputs.local_config_sha256,
                "namedFile": inputs.named_file_sha256,
                "namedFileSize": inputs.named_file_size,
            },
            "runnerVersion": runner_version,
            "provider": plan.provider,
            "providerReason": plan.reason,
            "runnerOutput": plan.output,
        }))
        .expect("plan revision data serializes"),
    );
    format!("unica-infobase-export-sha256-v1:{:x}", hasher.finalize())
}

fn public_plan(prepared: &PreparedInfobaseExport, plan: &PreviewPlan) -> Value {
    let candidates = plan.selection["candidates"]
        .as_array()
        .map(|candidates| {
            candidates
                .iter()
                .map(|candidate| {
                    json!({
                        "provider": candidate["provider"],
                        "implementation": candidate["implementation"],
                        "readiness": candidate["readiness"],
                        "evidence": candidate["evidence"],
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let named = json!({
        "kind": prepared.operation.artifact_kind(prepared.arguments.extension.as_deref()),
        "path": path_text(&prepared.arguments.named_file_relative),
    });
    let mut plan_value = json!({
        "provider": plan.provider,
        "reason": redactor(&plan.reason),
        "candidates": candidates,
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
    use crate::domain::refusal::Outcome;
    use crate::infrastructure::internal_adapters::ProcessOutput;
    use std::fs;
    use std::sync::Mutex;

    /// Страж отображения кодов раннера.
    ///
    /// Корень прежнего дефекта: карта перечисляла имена, которых раннер не пишет
    /// (`platform_error`, `validation_error`, `timeout`), а его настоящие имена
    /// (`platform_failure`, `timed_out`) в неё не попадали и уходили в запасную ветку.
    /// Владелец словаря один — [`RUNNER_WIRE_CODES`]; если раннер добавит код, этот
    /// тест покажет его, а не запасная ветка молча.
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

    #[test]
    fn runner_outcomes_follow_the_codes_rather_than_the_fallback() {
        for (code, expected) in [
            ("environment_unavailable", Outcome::NeedsHuman),
            ("capability_unavailable", Outcome::GoElsewhere),
            ("workspace_busy", Outcome::RetryAsIs),
            ("timed_out", Outcome::RetryAsIs),
            ("invalid_argument", Outcome::FixCall),
            ("invalid_output", Outcome::DeadEnd),
            ("cancelled", Outcome::DeadEnd),
        ] {
            assert_eq!(map_runner_code(code).outcome(), expected, "{code}");
        }
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

    fn prepared(root: &Path, dry_run: bool, if_rev: Option<String>) -> PreparedInfobaseExport {
        let context = WorkspaceContext {
            cwd: root.to_path_buf(),
            workspace_root: root.to_path_buf(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        };
        PreparedInfobaseExport {
            operation: ExportOperation::Configuration,
            arguments: ExportArguments {
                state: Some("working".to_string()),
                extension: None,
                named_file_relative: PathBuf::from("dist/main.cf"),
                named_file: root.join("dist/main.cf"),
                restore_mode: None,
            },
            dry_run,
            if_rev,
            context,
        }
    }

    fn prepared_restore(
        root: &Path,
        dry_run: bool,
        if_rev: Option<String>,
        mode: RestoreMode,
    ) -> PreparedInfobaseExport {
        let context = WorkspaceContext {
            cwd: root.to_path_buf(),
            workspace_root: root.to_path_buf(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        };
        PreparedInfobaseExport {
            operation: ExportOperation::Restore,
            arguments: ExportArguments {
                state: None,
                extension: None,
                named_file_relative: PathBuf::from("transfer/base.dt"),
                named_file: root.join("transfer/base.dt"),
                restore_mode: Some(mode),
            },
            dry_run,
            if_rev,
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
                "selection": {
                    "provider": "designer-batch",
                    "reason": "selected ready provider",
                    "candidates": [{
                        "provider": "designer-batch",
                        "implementation": "implemented",
                        "readiness": "ready",
                        "evidence": "live_verified",
                        "reason": "full platform is ready"
                    }]
                },
                "artifact_kind": "dt",
                "restored": false,
                "target_state": "unchanged",
                "plan": {"provider": "designer-batch", "artifact_kind": "dt", "input": input, "target_mode": mode},
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
                "selection": {"provider": "designer-batch", "reason": "selected ready provider"},
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
            "command": "infobase.configuration.export",
            "data": {
                "mode": "preview",
                "provider_dispatched": false,
                "state": "working",
                "subject": {"kind": "main"},
                "selection": {
                    "provider": "designer-batch",
                    "reason": "selected ready provider",
                    "candidates": [{
                        "provider": "designer-batch",
                        "implementation": "implemented",
                        "readiness": "ready",
                        "evidence": "argv_tested",
                        "reason": "full platform is ready"
                    }]
                },
                "artifact_kind": "cf",
                "published": false,
                "target_state": "unchanged",
                "plan": {"provider": "designer-batch", "artifact_kind": "cf", "output": output},
                "execution": {"status": "succeeded"}
            }
        })
    }

    fn apply_envelope(output: &Path) -> Value {
        json!({
            "command": "infobase.configuration.export",
            "data": {
                "mode": "apply",
                "state": "working",
                "subject": {"kind": "main"},
                "selection": {"provider": "designer-batch", "candidates": []},
                "artifact_kind": "cf",
                "output": output,
                "published": true,
                "target_state": "created",
                "execution": {"status": "succeeded"}
            }
        })
    }

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

    #[test]
    fn restore_preview_names_the_source_and_the_mode_without_touching_the_infobase() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(CONFIG_NAME), "format: DESIGNER\n").unwrap();
        fs::create_dir_all(root.path().join("transfer")).unwrap();
        fs::write(root.path().join("transfer/base.dt"), b"transfer bytes").unwrap();
        let prepared = prepared_restore(root.path(), true, None, RestoreMode::Replace);
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
        assert_eq!(next["ifRev"], json!(result.rev.clone().unwrap()));
        let call = &runner.calls.lock().unwrap()[0];
        assert!(call.args.contains(&"restore".to_string()));
        assert!(call.args.contains(&"--input".to_string()));
        assert!(call.args.contains(&"--replace".to_string()));
        assert!(call.args.contains(&"--dry-run".to_string()));
    }

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
        let preview = prepared_restore(root.path(), true, None, RestoreMode::Replace);
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
        let revision = preview_result.rev.clone().unwrap();

        let apply = prepared_restore(root.path(), false, Some(revision), RestoreMode::Replace);
        let runner = SequenceRunner::new(vec![
            process(restore_preview_envelope(&input, "replace")),
            process(restore_apply_envelope(&input, "replace", "replaced")),
        ]);
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
        // Превью повторяется перед применением, то есть два вызова раннера.
        assert_eq!(runner.calls.lock().unwrap().len(), 2);
        // Источник обязан дойти неизменным.
        assert_eq!(
            fs::read(root.path().join("transfer/base.dt")).unwrap(),
            b"transfer bytes"
        );
    }

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
        let prepared = prepared_restore(root.path(), true, None, RestoreMode::Replace);
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
        let prepared = prepared_restore(root.path(), true, None, RestoreMode::Create);
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

    #[test]
    fn preview_is_non_mutating_and_returns_an_apply_revision_without_raw_command() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(CONFIG_NAME), "format: DESIGNER\n").unwrap();
        let prepared = prepared(root.path(), true, None);
        let output = normalize_path_identity(&prepared.arguments.named_file).unwrap();
        let mut envelope = preview_envelope(&output);
        envelope["data"]["selection"]["candidates"][0]["reason"] =
            json!(format!("platform resolved at {}", root.path().display()));
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
        assert!(result
            .rev
            .as_deref()
            .is_some_and(|rev| rev.starts_with("unica-infobase-export-sha256-v1:")));
        assert_eq!(
            result.data.as_ref().unwrap()["plan"]["provider"],
            "designer-batch"
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

    #[test]
    fn apply_repeats_preflight_and_returns_an_independent_file_receipt() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(CONFIG_NAME), "format: DESIGNER\n").unwrap();
        let preview = prepared(root.path(), true, None);
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
        let apply = prepared(root.path(), false, preview_result.rev.clone());
        let runner = SequenceRunner::publishing(
            vec![
                process(preview_envelope(&output)),
                process(apply_envelope(&output)),
            ],
            b"verified cf",
        );

        let result =
            execute_with_resolved_runner(&apply, &runner, CancellationToken::new(), &tool, "0.7.0");

        assert!(result.ok, "{result:?}");
        assert_eq!(runner.calls.lock().unwrap().len(), 2);
        assert!(runner.calls.lock().unwrap()[0]
            .args
            .contains(&"--dry-run".to_string()));
        assert!(!runner.calls.lock().unwrap()[1]
            .args
            .contains(&"--dry-run".to_string()));
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

    #[test]
    fn stale_apply_stops_after_non_executing_preflight() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(CONFIG_NAME), "format: DESIGNER\n").unwrap();
        let apply = prepared(root.path(), false, Some("stale".to_string()));
        let output = normalize_path_identity(&apply.arguments.named_file).unwrap();
        let tool = BundledTool {
            program: root.path().join("v8-runner"),
            warnings: Vec::new(),
            missing: None,
        };
        let runner = SequenceRunner::new(vec![process(preview_envelope(&output))]);

        let result =
            execute_with_resolved_runner(&apply, &runner, CancellationToken::new(), &tool, "0.7.0");

        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "revision_mismatch");
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
        assert!(!apply.arguments.named_file.exists());
    }

    #[test]
    fn apply_rejects_a_runner_receipt_for_a_different_output() {
        let root = tempfile::tempdir().unwrap();
        let prepared = prepared(root.path(), false, Some("rev".to_string()));
        let output = normalize_path_identity(&prepared.arguments.named_file).unwrap();
        let plan = validate_preview(&prepared, &preview_envelope(&output)).unwrap();
        let mut applied = apply_envelope(&output);
        applied["data"]["output"] = json!(root.path().join("dist/other.cf"));

        let rejection = validate_apply(&prepared, &plan, &applied).unwrap_err();

        assert_eq!(rejection.diagnostics[0]["code"], "invalid_result");
    }

    #[test]
    fn preview_accepts_a_new_bounded_runner_provider_without_mcp_change() {
        let root = tempfile::tempdir().unwrap();
        let prepared = prepared(root.path(), true, None);
        let output = normalize_path_identity(&prepared.arguments.named_file).unwrap();
        let mut envelope = preview_envelope(&output);
        envelope["data"]["plan"]["provider"] = json!("ibcmd-rs");
        envelope["data"]["selection"]["provider"] = json!("ibcmd-rs");
        envelope["data"]["selection"]["candidates"][0]["provider"] = json!("ibcmd-rs");

        let plan = validate_preview(&prepared, &envelope).unwrap();

        assert_eq!(plan.provider, "ibcmd-rs");
    }

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
            operation: ExportOperation::Configuration,
            arguments: parse_export_arguments(ExportOperation::Configuration, &cfe_args, &context)
                .unwrap(),
            dry_run: true,
            if_rev: None,
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
                "infobase",
                "configuration",
                "export",
                "--state",
                "database",
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
            operation: ExportOperation::Infobase,
            arguments: parse_export_arguments(ExportOperation::Infobase, &dt_args, &context)
                .unwrap(),
            dry_run: true,
            if_rev: None,
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
        let prepared = prepared(root.path(), true, None);
        let runner = SequenceRunner::new(vec![failed_process(json!({
            "command": "infobase.configuration.export",
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
}
