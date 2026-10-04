#![allow(clippy::result_large_err)]
//! `pull` — выгрузка базы в набор исходников рабочего пространства
//! силами `v8-runner dump` (A-6 зонтика #871). Пара к `push`: тот
//! вносит исходники в базу, этот выносит базу в исходники.
//!
//! Аргументы закрыты: `force:true` обязателен, выгрузка всегда полная; `sourceSet` — имя
//! объявленного набора, без него раннер берёт набор конфигурации;
//! `extension` — имя расширения, которое раннер требует для набора расширения
//! и сверяет с ним. Частичная выгрузка по объектам за словарём: у неё свой
//! словарь селекторов, и он не описан.
//!
//! Превью зовёт `dump --dry-run`: раннер называет набор, режим и целевой
//! каталог, ничего не записывая. Цель обязана лежать внутри рабочего
//! пространства и наружу уходит относительным путём. Применение повторяет
//! превью, сверяет забор ревизии, выгружает и пересчитывает файлы в цели
//! сама — квитанция не со слов провайдера. Путь к платформе и командная
//! строка наружу не идут.

use super::protocol::InvocationRequest;
use super::runner_011::Runner011ProcessRunner;
use super::v13_infobase_exports::{
    digest_optional_workspace_file, digest_required_workspace_file, missing_runner_rejection,
    resolve_bundled_runner, runner_rejection, valid_1c_identifier, CONFIG_NAME, LOCAL_CONFIG_NAME,
    RUNNER_OUTPUT_LIMIT,
};
use super::v13_source_set_name::{source_set_name_guidance, valid_source_set_name};
use crate::application::invocation_store::ToolIdentity;
use crate::domain::cancellation::CancellationToken;
use crate::domain::invocation::{DomainResult, SafeIdentityHash};
use crate::domain::refusal::RefusalCode;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bundled_tools::BundledTool;
use crate::infrastructure::internal_adapters::{ProcessCommand, ProcessOutput, ProcessRunner};
use crate::infrastructure::redaction::redactor;
use crate::infrastructure::source_roots::normalize_path_identity;
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) const OPERATION: &str = "pull";
/// Имя команды в конверте раннера: словарь читается как слой и направление,
/// раннер называет свои команды по-своему.
const RUNNER_COMMAND: &str = "dump";
/// Предел пересчёта файлов в цели: квитанция бережёт время.
const FILE_COUNT_LIMIT: u64 = 1_000_000;

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

    const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Incremental => "incremental",
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
        execute_with_runner(self, &Runner011ProcessRunner, cancellation)
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
}

fn execute_with_resolved_runner(
    prepared: &PreparedSourceExport,
    runner: &dyn ProcessRunner,
    cancellation: CancellationToken,
    tool: &BundledTool,
    _runner_version: &str,
) -> DomainResult {
    if cancellation.is_cancelled() {
        return reject(RefusalCode::Cancelled, "pull cancelled before preflight");
    }
    let before = match capture_inputs(prepared) {
        Ok(inputs) => inputs,
        Err(result) => return result,
    };
    let root_identity = match normalize_path_identity(&prepared.context.workspace_root) {
        Ok(root) => root,
        Err(error) => {
            return reject(
                RefusalCode::InvalidState,
                format!("failed to resolve the workspace root: {error}"),
            )
        }
    };
    let preview = match invoke_runner(prepared, tool, runner, &cancellation, true) {
        Ok(envelope) => envelope,
        Err(result) => return result,
    };
    let plan = match validate_envelope(prepared, &before, &root_identity, &preview, false) {
        Ok(plan) => plan,
        Err(result) => return result,
    };
    let after = match capture_inputs(prepared) {
        Ok(inputs) => inputs,
        Err(result) => return result,
    };
    if before != after {
        return reject(
            RefusalCode::ConcurrentChange,
            "pull inputs changed during preview; run dryRun: true again",
        );
    }

    if prepared.dry_run {
        let mut result = DomainResult::success(format!(
            "pull planned a {} export of source set `{}` without writing anything",
            prepared.arguments.mode.as_str(),
            plan.source_set
        ));
        result.data = Some(json!({
            "op": OPERATION,
            "dryRun": true,
            "plan": {
                "sourceSet": plan.source_set,
                "extension": prepared.arguments.extension,
                "mode": prepared.arguments.mode.as_str(),
                "target": path_text(&plan.target),
                "targetExists": plan.existed,
                "force": true,
                "replacesLocalSources": true,
                "localWorkProtection": false,
            },
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
            "reason": "execute with the current arguments"
        }));
        return result;
    }
    if cancellation.is_cancelled() {
        return reject(
            RefusalCode::Cancelled,
            "pull cancelled before provider launch",
        );
    }
    let applied = match invoke_runner(prepared, tool, runner, &cancellation, false) {
        Ok(envelope) => envelope,
        Err(result) => return result,
    };
    let performed = match validate_envelope(prepared, &before, &root_identity, &applied, true) {
        Ok(plan) => plan,
        Err(result) => return result,
    };
    if performed.source_set != plan.source_set || performed.target_absolute != plan.target_absolute
    {
        return reject(
            RefusalCode::InvalidResult,
            "v8-runner exported a different source set or target than previewed",
        );
    }
    // Квитанция не со слов провайдера: цель Unica пересчитывает сама.
    let files = match count_regular_files(&plan.target_absolute) {
        Ok(files) if files > 0 => files,
        Ok(_) => {
            return reject(
                RefusalCode::InvalidResult,
                "v8-runner reported the export but the target source set is empty",
            )
        }
        Err(error) => {
            return reject(
                RefusalCode::InvalidResult,
                format!(
                    "v8-runner reported the export but the target could not be verified: {error}"
                ),
            )
        }
    };
    let state = if plan.existed { "replaced" } else { "created" };
    let mut result = DomainResult::success(format!(
        "pull exported source set `{}` ({} mode); the target holds {files} files",
        plan.source_set,
        prepared.arguments.mode.as_str()
    ));
    result.data = Some(json!({
        "op": OPERATION,
        "dryRun": false,
        "sourceSet": plan.source_set,
        "extension": prepared.arguments.extension,
        "mode": prepared.arguments.mode.as_str(),
        "target": path_text(&plan.target),
        "files": files,
        "state": state,
        "providerDispatched": true,
    }));
    result.changed.push(json!({
        "path": path_text(&plan.target),
        "kind": state,
    }));

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
    if data["mode"] != prepared.arguments.mode.runner_mode()
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
    Ok(ExportPlan {
        source_set,
        target,
        existed: target_absolute.is_dir(),
        target_absolute,
    })
}

fn count_regular_files(root: &Path) -> Result<u64, String> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = 0_u64;
    while let Some(directory) = pending.pop() {
        let entries = std::fs::read_dir(&directory)
            .map_err(|error| format!("{}: {error}", directory.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("{}: {error}", directory.display()))?;
            let file_type = entry
                .file_type()
                .map_err(|error| format!("{}: {error}", entry.path().display()))?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                files += 1;
                if files >= FILE_COUNT_LIMIT {
                    return Ok(files);
                }
            }
        }
    }
    Ok(files)
}

fn invoke_runner(
    prepared: &PreparedSourceExport,
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
        "--mode".to_string(),
        prepared.arguments.mode.as_str().to_string(),
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
        // Снимок отказа у `dump` пишет `provider_dispatched: true` и до
        // запуска платформы (v8-runner 0.9.0): отказ идёт по коду раннера.
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
        /// Что раннер пишет в цель на применении: настоящий выгружает дерево.
        materialize_on_apply: Option<PathBuf>,
    }

    impl SequenceRunner {
        fn new(outputs: Vec<ProcessOutput>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into_iter().rev().collect()),
                calls: Mutex::new(Vec::new()),
                materialize_on_apply: None,
            }
        }

        fn exporting(outputs: Vec<ProcessOutput>, target: PathBuf) -> Self {
            Self {
                outputs: Mutex::new(outputs.into_iter().rev().collect()),
                calls: Mutex::new(Vec::new()),
                materialize_on_apply: Some(target),
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
                if let Some(target) = &self.materialize_on_apply {
                    fs::create_dir_all(target.join("Catalogs")).unwrap();
                    fs::write(target.join("Configuration.xml"), "<MetaDataObject />\n").unwrap();
                    fs::write(target.join("Catalogs/Items.xml"), "<MetaDataObject />\n").unwrap();
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

    fn workspace() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join(CONFIG_NAME),
            "format: DESIGNER\ninfobase:\n  connection: 'File=build/ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: main\n  - name: ext-sales\n    type: EXTENSION\n    path: ext-sales\n",
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

    /// Конверт `dump` раннера 0.9.0, снятый с живой пробы: набор, режим
    /// прописными и абсолютный целевой каталог.
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
            "command": "dump",
            "duration_ms": 2,
            "data": data,
            "warnings": [],
            "steps": [],
        })
    }

    fn run(root: &Path, prepared: &PreparedSourceExport, runner: &SequenceRunner) -> DomainResult {
        execute_with_resolved_runner(
            prepared,
            runner,
            CancellationToken::new(),
            &tool(root),
            "0.9.0",
        )
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
            .contains("declared source sets: main, ext-sales"));
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
            .ends_with("--json-message dump --mode full --dry-run"));
    }

    #[test]
    fn preview_of_an_extension_set_passes_the_extension_and_the_set() {
        let root = workspace();
        let runner = SequenceRunner::new(vec![process(
            envelope(
                root.path(),
                "ext-sales",
                "INCREMENTAL",
                Some("Sales"),
                false,
            ),
            true,
        )]);
        let result = run(
            root.path(),
            &prepared(
                root.path(),
                ExportMode::Incremental,
                Some("ext-sales"),
                Some("Sales"),
                true,
            ),
            &runner,
        );
        assert!(result.ok, "{result:?}");
        assert_eq!(result.data.as_ref().unwrap()["plan"]["extension"], "Sales");
        assert!(runner.joined_args(0).ends_with(
            "dump --mode incremental --source-set ext-sales --extension Sales --dry-run"
        ));
    }

    #[test]
    fn preview_accepts_cyrillic_extension_source_set_from_public_run_request() {
        let root = workspace();
        let config = root.path().join(CONFIG_NAME);
        let yaml = fs::read_to_string(&config)
            .unwrap()
            .replace("ext-sales", "Доработки");
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
            .ends_with("--source-set Доработки --extension Доработки --dry-run"));
        assert!(result.changed.is_empty());
        assert!(!root.path().join("Доработки").exists());
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
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("outside the workspace"));

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
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("different source set or target"));

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
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("could not be verified"));

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
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("the target source set is empty"));
    }

    #[test]
    fn runner_refusals_keep_their_outcome() {
        // Снимок отказа `dump` у раннера 0.9.0 пишет `provider_dispatched: true`
        // до запуска платформы; отказ идёт по коду.
        let root = workspace();
        let failure = |code: &str, message: &str| {
            json!({
                "ok": false,
                "command": "dump",
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
                &prepared(root.path(), ExportMode::Full, Some("ext-sales"), None, true),
                &runner,
            );
            assert_eq!(result.diagnostics[0]["code"], expected, "{code}");
            assert_eq!(map_runner_code(code).outcome(), outcome, "{code}");
        }
    }
}
