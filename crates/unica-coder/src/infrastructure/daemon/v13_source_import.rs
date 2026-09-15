#![allow(clippy::result_large_err)]
//! `source.import` — импорт исходников рабочего пространства в базу силами
//! `v8-runner build` (A-4 зонтика #871). Пара к `source.export`: тот выносит
//! базу в исходники, этот вносит исходники в базу.
//!
//! Аргументы закрыты: `sourceSet` — имя одного объявленного набора (без него
//! импортируются все), `fullRebuild` — сбросить кэш изменений раннера и
//! загрузить всё целиком. Превью зовёт `build --dry-run`: раннер выбирает
//! для каждого набора режим (`full` или `partial` по своим правилам частичной
//! загрузки), не запуская конфигуратор. Применение повторяет превью, сверяет
//! забор ревизии и требует тот же состав наборов и те же режимы: иной режим
//! значит, что исходники изменились между превью и применением.
//!
//! Состояние базы после импорта Unica не проверяет — оно засвидетельствовано
//! провайдером, и ответ называет это прямо. Проза шагов раннера наружу не
//! идёт.

use super::protocol::InvocationRequest;
use super::v13_infobase_exports::{
    digest_optional_workspace_file, digest_required_workspace_file, map_runner_code, CONFIG_NAME,
    LOCAL_CONFIG_NAME, RUNNER_OUTPUT_LIMIT,
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
use crate::infrastructure::plugin_runtime::find_plugin_root;
use crate::infrastructure::redaction::redactor;
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) const OPERATION: &str = "source.import";
/// Имя команды в конверте раннера: словарь читается как слой и направление,
/// раннер называет свои команды по-своему.
const RUNNER_COMMAND: &str = "build";
const SOURCE_SET_NAME_MAX: usize = 64;
const STEPS_MAX: usize = 64;

#[derive(Debug, Clone)]
struct ImportArguments {
    /// Один объявленный набор; `None` — все наборы `v8project.yaml`.
    source_set: Option<String>,
    full_rebuild: bool,
}

#[derive(Debug, Clone)]
pub(super) struct PreparedSourceImport {
    arguments: ImportArguments,
    dry_run: bool,
    if_rev: Option<String>,
    context: WorkspaceContext,
}

pub(super) enum Preparation {
    NotApplicable,
    Rejected(Box<DomainResult>),
    Ready(Arc<PreparedSourceImport>),
}

pub(super) fn prepare(request: &InvocationRequest) -> Preparation {
    if request.tool() != ToolIdentity::Run
        || request.arguments().get("op").and_then(Value::as_str) != Some(OPERATION)
    {
        return Preparation::NotApplicable;
    }
    match PreparedSourceImport::parse(request) {
        Ok(prepared) => Preparation::Ready(Arc::new(prepared)),
        Err(result) => Preparation::Rejected(Box::new(result)),
    }
}

impl PreparedSourceImport {
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
                    "source.import requires dryRun: true to preview or dryRun: false with ifRev to apply",
                )
            })?;
        let if_rev = match arguments.get("ifRev") {
            None => None,
            Some(Value::String(value)) if !value.trim().is_empty() => Some(value.clone()),
            Some(_) => {
                return Err(reject(
                    RefusalCode::BadValue,
                    "source.import ifRev must be non-empty text",
                ))
            }
        };
        if dry_run && if_rev.is_some() {
            return Err(reject(
                RefusalCode::BadValue,
                "source.import preview does not accept ifRev; apply the revision returned by this preview",
            ));
        }
        if !dry_run && if_rev.is_none() {
            return Err(reject(
                RefusalCode::BadValue,
                "source.import apply requires ifRev from a prior dryRun preview",
            ));
        }
        let context =
            discover_workspace(Some(PathBuf::from(request.workspace_hint()))).map_err(|error| {
                reject(
                    RefusalCode::ProviderUnavailable,
                    format!("workspace discovery failed: {error}"),
                )
            })?;
        let arguments = parse_import_arguments(args)?;
        Ok(Self {
            arguments,
            dry_run,
            if_rev,
            context,
        })
    }

    pub(super) fn workspace_identity_hash(&self) -> SafeIdentityHash {
        let mut hasher = Sha256::new();
        hasher.update(b"unica-v13-source-import-workspace-v1\0");
        hasher.update(self.context.workspace_root.as_os_str().as_encoded_bytes());
        SafeIdentityHash::from_sha256(hasher.finalize().into())
    }

    pub(super) fn execute(&self, cancellation: CancellationToken) -> DomainResult {
        execute_with_runner(self, &SystemProcessRunner, cancellation)
    }
}

fn parse_import_arguments(args: &Map<String, Value>) -> Result<ImportArguments, DomainResult> {
    const ACCEPTED: [&str; 2] = ["sourceSet", "fullRebuild"];
    if let Some(unknown) = args.keys().find(|key| !ACCEPTED.contains(&key.as_str())) {
        return Err(reject(
            RefusalCode::BadValue,
            format!("source.import does not accept `{unknown}`; the closed args are sourceSet and fullRebuild"),
        ));
    }
    let source_set = match args.get("sourceSet") {
        None => None,
        Some(Value::String(value)) if valid_source_set_name(value) => Some(value.clone()),
        Some(_) => {
            return Err(reject(
                RefusalCode::BadValue,
                format!("source.import sourceSet must be the name of a source set declared in v8project.yaml: up to {SOURCE_SET_NAME_MAX} letters, digits, `_`, `-` or `.`"),
            ))
        }
    };
    let full_rebuild = match args.get("fullRebuild") {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => {
            return Err(reject(
                RefusalCode::BadValue,
                "source.import fullRebuild must be boolean",
            ))
        }
    };
    Ok(ImportArguments {
        source_set,
        full_rebuild,
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

/// Состояние входов, обязанное совпасть у превью и применения: проектный файл
/// с локальным дополнением и объявленный в нём состав наборов. Содержимое
/// деревьев исходников сюда не входит: его перемену раннер показывает
/// режимом шага, и она ловится сверкой режимов, а не байтов.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StableInputs {
    config_sha256: String,
    local_config_sha256: Option<String>,
    declared: Vec<String>,
}

fn capture_inputs(prepared: &PreparedSourceImport) -> Result<StableInputs, DomainResult> {
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
                    "source.import sourceSet `{source_set}` is not declared in v8project.yaml; declared source sets: {}",
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

/// Имена наборов из `v8project.yaml` в объявленном порядке. Файл уже проверен
/// как обычный файл рабочего пространства при взятии дайджеста.
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
            format!("{CONFIG_NAME} declares no source-set; there is nothing to import"),
        ));
    }
    Ok(names)
}

fn execute_with_runner(
    prepared: &PreparedSourceImport,
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

/// Режим шага раннера — закрытый словарь. `partial` приходит объектом с числом
/// файлов, остальные — словом; `edt_export` здесь не обслуживается.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StepMode {
    Full,
    Partial { file_count: u64 },
    Skipped,
}

impl StepMode {
    fn parse(value: &Value) -> Option<Self> {
        match value {
            Value::String(mode) if mode == "full" => Some(Self::Full),
            Value::String(mode) if mode == "skipped" => Some(Self::Skipped),
            Value::Object(mode) => {
                let file_count = mode.get("partial")?.get("file_count")?.as_u64()?;
                Some(Self::Partial { file_count })
            }
            _ => None,
        }
    }

    const fn as_str(&self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Partial { .. } => "partial",
            Self::Skipped => "skipped",
        }
    }

    fn public(&self) -> Value {
        match self {
            Self::Partial { file_count } => json!({"mode": "partial", "files": file_count}),
            other => json!({"mode": other.as_str()}),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlannedStep {
    source_set: String,
    mode: StepMode,
}

fn planned_steps(envelope: &Value) -> Result<Vec<PlannedStep>, DomainResult> {
    let steps = envelope["data"]["steps"]
        .as_array()
        .filter(|steps| !steps.is_empty() && steps.len() <= STEPS_MAX)
        .ok_or_else(|| {
            reject(
                RefusalCode::InvalidResult,
                "v8-runner build answered with an empty or oversized step list",
            )
        })?;
    let mut planned: Vec<PlannedStep> = Vec::with_capacity(steps.len());
    for step in steps {
        let source_set = step["source_set"]
            .as_str()
            .filter(|name| valid_source_set_name(name))
            .ok_or_else(|| {
                reject(
                    RefusalCode::InvalidResult,
                    "v8-runner build reported a step without a valid source set name",
                )
            })?
            .to_string();
        if planned.iter().any(|step| step.source_set == source_set) {
            return Err(reject(
                RefusalCode::InvalidResult,
                format!("v8-runner build reported source set `{source_set}` twice"),
            ));
        }
        if step["ok"] != true {
            return Err(reject(
                RefusalCode::InvalidResult,
                format!("v8-runner build reported success with a failed step for `{source_set}`"),
            ));
        }
        let mode = StepMode::parse(&step["mode"]).ok_or_else(|| {
            if step["mode"] == "edt_export" {
                reject(
                    RefusalCode::InvalidState,
                    format!("source set `{source_set}` is declared in a format that needs EDT; source.import serves Designer sources only"),
                )
            } else {
                reject(
                    RefusalCode::InvalidResult,
                    format!("v8-runner build reported an unknown mode for `{source_set}`"),
                )
            }
        })?;
        planned.push(PlannedStep { source_set, mode });
    }
    Ok(planned)
}

fn execute_with_resolved_runner(
    prepared: &PreparedSourceImport,
    runner: &dyn ProcessRunner,
    cancellation: CancellationToken,
    tool: &BundledTool,
    runner_version: &str,
) -> DomainResult {
    if cancellation.is_cancelled() {
        return reject(
            RefusalCode::Cancelled,
            "source.import cancelled before preflight",
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
    let plan = match validate_preview(prepared, &before, &preview) {
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
            "source.import inputs changed during preview; run dryRun: true again",
        );
    }
    let revision = plan_revision(prepared, &before, runner_version, &plan);
    if prepared.dry_run {
        let mut result = DomainResult::success(format!(
            "source.import planned importing {} without touching the infobase",
            subject_summary(&plan)
        ));
        result.data = Some(json!({
            "op": OPERATION,
            "dryRun": true,
            "plan": public_plan(prepared, &plan),
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
            "reason": "apply exactly this previewed import"
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
                "source.import plan or environment changed after preview: expected rev {revision}, ifRev {}; run dryRun: true again",
                prepared.if_rev.as_deref().unwrap_or("absent")
            ),
        );
    }
    if cancellation.is_cancelled() {
        return reject(
            RefusalCode::Cancelled,
            "source.import cancelled before provider launch",
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
    let performed = match planned_steps(&applied) {
        Ok(steps) => steps,
        Err(result) => return result,
    };
    if performed != plan {
        // Иной состав или режим значит, что раннер импортировал не тот план,
        // который одобрен: исходники или проектный файл сменились после превью.
        return reject(
            RefusalCode::ConcurrentChange,
            "source.import performed a different plan than previewed: the sources changed between preview and apply; run dryRun: true again",
        );
    }
    // **Улику о состоянии базы Unica не подделывает.** База живёт за
    // соединением, и её состояние здесь засвидетельствовано провайдером, а не
    // проверено нами; источник признания назван прямо.
    let mut result = DomainResult::success(format!(
        "source.import imported {}; the infobase state is attested by the provider",
        subject_summary(&plan)
    ));
    result.data = Some(json!({
        "op": OPERATION,
        "dryRun": false,
        "providerDispatched": true,
        "fullRebuild": prepared.arguments.full_rebuild,
        "steps": plan.iter().map(|step| {
            let mut value = step.mode.public();
            value["sourceSet"] = json!(step.source_set);
            value
        }).collect::<Vec<_>>(),
        "targetStateAttestedBy": "provider",
    }));
    // Изменилась база, а не файл рабочего пространства: путь сюда не кладётся.
    for step in &plan {
        result.changed.push(json!({
            "infobase": true,
            "kind": "configuration",
            "sourceSet": step.source_set,
            "mode": step.mode.as_str(),
        }));
    }
    result.rev = Some(revision);
    result
}

fn validate_preview(
    prepared: &PreparedSourceImport,
    inputs: &StableInputs,
    envelope: &Value,
) -> Result<Vec<PlannedStep>, DomainResult> {
    if envelope["data"]["provider_dispatched"] != false {
        return Err(reject(
            RefusalCode::InvalidResult,
            "v8-runner preview did not prove that Designer was not dispatched",
        ));
    }
    let plan = planned_steps(envelope)?;
    let expected: Vec<&str> = match &prepared.arguments.source_set {
        Some(source_set) => vec![source_set.as_str()],
        None => inputs.declared.iter().map(String::as_str).collect(),
    };
    let planned: Vec<&str> = plan.iter().map(|step| step.source_set.as_str()).collect();
    if planned != expected {
        return Err(reject(
            RefusalCode::InvalidResult,
            "v8-runner preview planned different source sets than v8project.yaml declares",
        ));
    }
    if prepared.arguments.full_rebuild && plan.iter().any(|step| step.mode != StepMode::Full) {
        return Err(reject(
            RefusalCode::InvalidResult,
            "v8-runner preview planned a partial import despite fullRebuild",
        ));
    }
    Ok(plan)
}

fn invoke_runner(
    prepared: &PreparedSourceImport,
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
    if let Some(source_set) = &prepared.arguments.source_set {
        args.extend(["--source-set".to_string(), source_set.clone()]);
    }
    if prepared.arguments.full_rebuild {
        args.push("--full-rebuild".to_string());
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

fn subject_summary(plan: &[PlannedStep]) -> String {
    match plan {
        [single] => format!("source set `{}`", single.source_set),
        steps => format!("{} source sets", steps.len()),
    }
}

fn plan_revision(
    prepared: &PreparedSourceImport,
    inputs: &StableInputs,
    runner_version: &str,
    plan: &[PlannedStep],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"unica-v13-source-import-plan-v1\0");
    hasher.update(
        serde_json::to_vec(&json!({
            "op": OPERATION,
            "args": public_arguments(prepared),
            "inputs": {
                "config": inputs.config_sha256,
                "localConfig": inputs.local_config_sha256,
                "declared": inputs.declared,
            },
            "runnerVersion": runner_version,
            "steps": plan.iter().map(|step| json!({
                "sourceSet": step.source_set,
                "mode": step.mode.public(),
            })).collect::<Vec<_>>(),
        }))
        .expect("plan revision data serializes"),
    );
    format!("unica-source-import-sha256-v1:{:x}", hasher.finalize())
}

fn public_plan(prepared: &PreparedSourceImport, plan: &[PlannedStep]) -> Value {
    json!({
        "fullRebuild": prepared.arguments.full_rebuild,
        "steps": plan.iter().map(|step| {
            let mut value = step.mode.public();
            value["sourceSet"] = json!(step.source_set);
            value
        }).collect::<Vec<_>>(),
        // Что превью узнать не может, названо, а не умолчано.
        "targetStateKnownBeforeApply": false,
    })
}

fn public_arguments(prepared: &PreparedSourceImport) -> Value {
    let mut args = Map::new();
    if let Some(source_set) = &prepared.arguments.source_set {
        args.insert("sourceSet".to_string(), Value::String(source_set.clone()));
    }
    if prepared.arguments.full_rebuild {
        args.insert("fullRebuild".to_string(), Value::Bool(true));
    }
    Value::Object(args)
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
            "format: DESIGNER\ninfobase:\n  connection: 'File=build/ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: main\n  - name: ext-sales\n    type: EXTENSION\n    path: ext-sales\n",
        )
        .unwrap();
        root
    }

    fn prepared(
        root: &Path,
        source_set: Option<&str>,
        full_rebuild: bool,
        dry_run: bool,
        if_rev: Option<String>,
    ) -> PreparedSourceImport {
        PreparedSourceImport {
            arguments: ImportArguments {
                source_set: source_set.map(str::to_string),
                full_rebuild,
            },
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

    /// Конверт `build` раннера 0.9.0, снятый с живой пробы: шаг на набор с
    /// режимом, `partial` приходит объектом с числом файлов.
    fn envelope(steps: &[(&str, Value)], dispatched: bool) -> Value {
        json!({
            "ok": true,
            "command": "build",
            "duration_ms": 4,
            "data": {
                "ok": true,
                "provider_dispatched": dispatched,
                "steps": steps.iter().map(|(set, mode)| json!({
                    "source_set": set,
                    "mode": mode,
                    "ok": true,
                    "message": if dispatched {
                        "loaded via /opt/1cv8/8.3.27.2074/1cv8"
                    } else {
                        "full load selected by partial-load rules; planned, Designer not dispatched"
                    },
                    "duration_ms": 0
                })).collect::<Vec<_>>(),
                "duration_ms": 4
            },
            "warnings": [],
            "steps": []
        })
    }

    fn full() -> Value {
        json!("full")
    }

    fn partial(files: u64) -> Value {
        json!({"partial": {"file_count": files}})
    }

    fn run(root: &Path, prepared: &PreparedSourceImport, runner: &SequenceRunner) -> DomainResult {
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
        for (args, expected) in [
            (
                json!({"connection": "File=/x"}),
                "does not accept `connection`",
            ),
            (json!({"sourceSet": 7}), "must be the name of a source set"),
            (
                json!({"sourceSet": "../main"}),
                "must be the name of a source set",
            ),
            (json!({"fullRebuild": "yes"}), "fullRebuild must be boolean"),
        ] {
            let result = parse_import_arguments(args.as_object().unwrap())
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
            json!({"sourceSet": "main", "fullRebuild": true})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(accepted.source_set.as_deref(), Some("main"));
        assert!(accepted.full_rebuild);

        let root = workspace();
        let runner = SequenceRunner::new(Vec::new());
        let result = run(
            root.path(),
            &prepared(root.path(), Some("ext-purchases"), false, true, None),
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
    fn preview_plans_every_declared_source_set_with_its_mode_without_dispatching_designer() {
        let root = workspace();
        let runner = SequenceRunner::new(vec![process(
            envelope(&[("main", full()), ("ext-sales", partial(3))], false),
            true,
        )]);
        let result = run(
            root.path(),
            &prepared(root.path(), None, false, true, None),
            &runner,
        );

        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["providerDispatched"], false);
        assert_eq!(data["plan"]["fullRebuild"], false);
        assert_eq!(data["plan"]["steps"][0]["sourceSet"], "main");
        assert_eq!(data["plan"]["steps"][0]["mode"], "full");
        assert_eq!(data["plan"]["steps"][1]["sourceSet"], "ext-sales");
        assert_eq!(data["plan"]["steps"][1]["mode"], "partial");
        assert_eq!(data["plan"]["steps"][1]["files"], 3);
        assert_eq!(data["plan"]["targetStateKnownBeforeApply"], false);
        let revision = result.rev.clone().expect("preview returns a revision");
        assert!(revision.starts_with("unica-source-import-sha256-v1:"));
        assert_eq!(result.next[0]["args"]["ifRev"], revision);
        assert_eq!(result.next[0]["args"]["args"], json!({}));
        assert!(result.changed.is_empty());
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("1cv8"), "platform path leaked: {encoded}");
        assert!(!encoded.contains("--config"));
        assert!(runner
            .joined_args(0)
            .ends_with("--json-message build --dry-run"));
    }

    #[test]
    fn preview_of_one_source_set_with_full_rebuild_asks_the_runner_for_exactly_that() {
        let root = workspace();
        let runner = SequenceRunner::new(vec![process(
            envelope(&[("ext-sales", full())], false),
            true,
        )]);
        let result = run(
            root.path(),
            &prepared(root.path(), Some("ext-sales"), true, true, None),
            &runner,
        );

        assert!(result.ok, "{result:?}");
        assert_eq!(
            result.next[0]["args"]["args"],
            json!({"sourceSet": "ext-sales", "fullRebuild": true})
        );
        assert!(result.summary.contains("source set `ext-sales`"));
        assert!(runner
            .joined_args(0)
            .ends_with("build --source-set ext-sales --full-rebuild --dry-run"));

        // Частичный план вопреки полной пересборке — не наш план.
        let runner = SequenceRunner::new(vec![process(
            envelope(&[("ext-sales", partial(1))], false),
            true,
        )]);
        let result = run(
            root.path(),
            &prepared(root.path(), Some("ext-sales"), true, true, None),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
    }

    #[test]
    fn preview_refuses_other_sets_a_dispatched_designer_and_edt_sources() {
        let root = workspace();
        let runner = SequenceRunner::new(vec![process(envelope(&[("main", full())], false), true)]);
        let result = run(
            root.path(),
            &prepared(root.path(), None, false, true, None),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );

        let runner = SequenceRunner::new(vec![process(
            envelope(&[("main", full()), ("ext-sales", full())], true),
            true,
        )]);
        let result = run(
            root.path(),
            &prepared(root.path(), None, false, true, None),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("not dispatched"));

        let runner = SequenceRunner::new(vec![process(
            envelope(
                &[("main", json!("edt_export")), ("ext-sales", full())],
                false,
            ),
            true,
        )]);
        let result = run(
            root.path(),
            &prepared(root.path(), None, false, true, None),
            &runner,
        );
        assert_eq!(result.diagnostics[0]["code"], "invalid_state", "{result:?}");
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("Designer sources only"));
    }

    #[test]
    fn apply_repeats_the_preview_and_attributes_the_infobase_state_to_the_provider() {
        let root = workspace();
        let plan = [("main", full()), ("ext-sales", partial(3))];
        let revision = run(
            root.path(),
            &prepared(root.path(), None, false, true, None),
            &SequenceRunner::new(vec![process(envelope(&plan, false), true)]),
        )
        .rev
        .unwrap();
        let runner = SequenceRunner::new(vec![
            process(envelope(&plan, false), true),
            process(envelope(&plan, true), true),
        ]);

        let result = run(
            root.path(),
            &prepared(root.path(), None, false, false, Some(revision.clone())),
            &runner,
        );

        assert!(result.ok, "{result:?}");
        assert_eq!(runner.call_count(), 2);
        assert!(runner.joined_args(0).ends_with("--dry-run"));
        assert!(runner.joined_args(1).ends_with("--json-message build"));
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["providerDispatched"], true);
        assert_eq!(data["steps"][1]["mode"], "partial");
        assert_eq!(data["steps"][1]["files"], 3);
        assert_eq!(data["targetStateAttestedBy"], "provider");
        assert_eq!(result.changed.len(), 2);
        assert_eq!(result.changed[0]["infobase"], true);
        assert_eq!(result.changed[0]["sourceSet"], "main");
        assert_eq!(result.changed[1]["mode"], "partial");
        assert!(result.changed[0].get("path").is_none());
        assert!(result.artifacts.is_empty());
        assert_eq!(result.rev, Some(revision));
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("1cv8"), "platform path leaked: {encoded}");
    }

    #[test]
    fn apply_refuses_a_stale_revision_and_a_plan_that_changed_underneath() {
        let root = workspace();
        let plan = [("main", full()), ("ext-sales", partial(3))];
        let runner = SequenceRunner::new(vec![process(envelope(&plan, false), true)]);
        let result = run(
            root.path(),
            &prepared(root.path(), None, false, false, Some("stale".to_string())),
            &runner,
        );
        assert_eq!(result.diagnostics[0]["code"], "stale_revision");
        assert_eq!(runner.call_count(), 1);

        let revision = run(
            root.path(),
            &prepared(root.path(), None, false, true, None),
            &SequenceRunner::new(vec![process(envelope(&plan, false), true)]),
        )
        .rev
        .unwrap();
        // Между превью и применением исходники изменились: раннер выбрал
        // полный режим там, где план был частичным.
        let runner = SequenceRunner::new(vec![
            process(envelope(&plan, false), true),
            process(
                envelope(&[("main", full()), ("ext-sales", full())], true),
                true,
            ),
        ]);
        let result = run(
            root.path(),
            &prepared(root.path(), None, false, false, Some(revision)),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "concurrent_change",
            "{result:?}"
        );
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("sources changed between preview and apply"));
    }

    #[test]
    fn runner_refusals_keep_their_outcome() {
        let root = workspace();
        let failure = |code: &str| {
            json!({
                "ok": false,
                "command": "build",
                "duration_ms": 0,
                "data": {"ok": false, "provider_dispatched": true, "steps": [
                    {"source_set": "main", "mode": "full", "ok": false, "message": "refused", "duration_ms": 0}
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
            ("invalid_argument", "bad_value", Outcome::FixCall),
        ] {
            let runner = SequenceRunner::new(vec![process(failure(code), false)]);
            let result = run(
                root.path(),
                &prepared(root.path(), None, false, true, None),
                &runner,
            );
            assert_eq!(result.diagnostics[0]["code"], expected, "{code}");
            assert_eq!(map_runner_code(code).outcome(), outcome, "{code}");
        }
    }
}
