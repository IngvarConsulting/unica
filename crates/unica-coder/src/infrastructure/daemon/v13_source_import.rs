#![allow(clippy::result_large_err)]
//! `push` — импорт исходников рабочего пространства в базу силами
//! `v8-runner push` (A-4 зонтика #871). Пара к `pull`: тот выносит
//! базу в исходники, этот вносит исходники в базу.
//!
//! Аргументы закрыты: `sourceSet` — имя одного объявленного набора (без него
//! импортируются все), `force:true` обязателен; `full` — сбросить кэш изменений раннера и
//! загрузить всё целиком. Превью зовёт `push --dry-run`: раннер выбирает
//! для каждого набора режим (`full` или `partial` по своим правилам частичной
//! загрузки), не запуская конфигуратор. Применение повторяет превью, сверяет
//! забор ревизии и требует тот же состав наборов и те же режимы: иной режим
//! значит, что исходники изменились между превью и применением.
//!
//! Состояние базы после импорта Unica не проверяет — оно засвидетельствовано
//! провайдером, и ответ называет это прямо. Набор, который раннер пропустил
//! по своей памяти, загружен не был: такой ответ называет состояние базы
//! непроверенным, а не импортом. Проза шагов раннера наружу не идёт.

use super::protocol::InvocationRequest;
use super::runner_012::Runner012ProcessRunner;
use super::v13_infobase_exports::{
    digest_optional_workspace_file, digest_required_workspace_file, missing_runner_rejection,
    resolve_bundled_runner, runner_rejection, runner_start_rejection, CONFIG_NAME,
    LOCAL_CONFIG_NAME, RUNNER_OUTPUT_LIMIT,
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
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) const OPERATION: &str = "push";
/// Имя команды раннера и в командной строке, и в конверте: с 0.12 раннер
/// называет её словом словаря.
const RUNNER_COMMAND: &str = "push";
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
                    "push requires dryRun: true to preview or dryRun: false to execute",
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
            .find(|k| !["sourceSet", "full", "force", "noApply"].contains(&k.as_str()))
        {
            return Err(reject(
                RefusalCode::BadValue,
                format!("push does not accept `{unknown}`; use its published argsSchema"),
            ));
        }
        if args.get("force") != Some(&Value::Bool(true)) {
            return Err(reject(RefusalCode::UnsupportedOperation, "push requires force:true on the compatibility adapter; synchronization protection is unavailable"));
        }
        let mut public = args.clone();
        public.remove("force");
        if public.get("noApply").is_some_and(|v| v != false) {
            return Err(reject(RefusalCode::UnsupportedOperation, "push noApply is not supported by this adapter; default push applies the database configuration"));
        }
        public.remove("noApply");
        if let Some(full) = public.remove("full") {
            public.insert("fullRebuild".into(), full);
        }
        let arguments = parse_import_arguments(&public)?;
        Ok(Self {
            arguments,
            dry_run,
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
        execute_with_runner(self, &Runner012ProcessRunner, cancellation)
    }
}

fn parse_import_arguments(args: &Map<String, Value>) -> Result<ImportArguments, DomainResult> {
    const ACCEPTED: [&str; 2] = ["sourceSet", "fullRebuild"];
    if let Some(unknown) = args.keys().find(|key| !ACCEPTED.contains(&key.as_str())) {
        return Err(reject(
            RefusalCode::BadValue,
            format!(
                "push does not accept `{unknown}`; the closed args are sourceSet and fullRebuild"
            ),
        ));
    }
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
    let full_rebuild = match args.get("fullRebuild") {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => {
            return Err(reject(
                RefusalCode::BadValue,
                "push fullRebuild must be boolean",
            ))
        }
    };
    Ok(ImportArguments {
        source_set,
        full_rebuild,
    })
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
                    "push sourceSet `{source_set}` is not declared in v8project.yaml; declared source sets: {}",
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
    let runner_tool = match resolve_bundled_runner(&prepared.context.cwd) {
        Ok(resolved) => resolved,
        Err(message) => return reject_absent_runner(message),
    };
    let tool = runner_tool.tool;
    let runner_version = runner_tool.version;
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
                "v8-runner push answered with an empty or oversized step list",
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
                    "v8-runner push reported a step without a valid source set name",
                )
            })?
            .to_string();
        if planned.iter().any(|step| step.source_set == source_set) {
            return Err(reject(
                RefusalCode::InvalidResult,
                format!("v8-runner push reported source set `{source_set}` twice"),
            ));
        }
        if step["ok"] != true {
            return Err(reject(
                RefusalCode::InvalidResult,
                format!("v8-runner push reported success with a failed step for `{source_set}`"),
            ));
        }
        let mode = StepMode::parse(&step["mode"]).ok_or_else(|| {
            if step["mode"] == "edt_export" {
                reject(
                    RefusalCode::InvalidState,
                    format!("source set `{source_set}` is declared in a format that needs EDT; push serves Designer sources only"),
                )
            } else {
                reject(
                    RefusalCode::InvalidResult,
                    format!("v8-runner push reported an unknown mode for `{source_set}`"),
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
    _runner_version: &str,
) -> DomainResult {
    if cancellation.is_cancelled() {
        return reject(RefusalCode::Cancelled, "push cancelled before preflight");
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
            "push inputs changed during preview; run dryRun: true again",
        );
    }

    if prepared.dry_run {
        let nothing_planned = plan.iter().all(|step| step.mode == StepMode::Skipped);
        let summary = if nothing_planned {
            format!(
                "push planned no load for {}: the runner found no changes against its own memory; executing will not verify the infobase state",
                subject_summary(&plan)
            )
        } else {
            let mut summary = format!(
                "push planned importing {} without touching the infobase",
                subject_summary(&plan)
            );
            let skipped_count = plan
                .iter()
                .filter(|step| step.mode == StepMode::Skipped)
                .count();
            if skipped_count > 0 {
                summary.push_str(&format!(
                    "; {skipped_count} of them skipped by the runner's memory and will not be loaded"
                ));
            }
            summary
        };
        let mut result = DomainResult::success(summary);
        result.warnings.extend(skipped_warning(&plan));
        result.data = Some(json!({
            "op": OPERATION,
            "dryRun": true,
            "plan": public_plan(prepared, &plan),
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
            "reason": if nothing_planned {
                "execute with the current arguments; it loads nothing and does not verify the infobase state"
            } else {
                "execute with the current arguments"
            }
        }));
        // Исполнение пустого плана ничего не загрузит. Полная загрузка нужна,
        // только если базу могли изменить вне этой рабочей копии, поэтому она
        // идёт второй и с условием.
        if nothing_planned {
            result.next.push(full_preview_hint(prepared));
        }
        return result;
    }
    if cancellation.is_cancelled() {
        return reject(
            RefusalCode::Cancelled,
            "push cancelled before provider launch",
        );
    }
    let applied = match invoke_runner(prepared, tool, runner, &cancellation, false) {
        Ok(envelope) => envelope,
        Err(result) => return result,
    };
    let performed = match planned_steps(&applied) {
        Ok(steps) => steps,
        Err(result) => return result,
    };
    // Раннер запускает платформу, только если какому-то набору есть что
    // грузить. Все наборы пропущены — платформы нет; иначе она обязана быть.
    let anything_loaded = performed.iter().any(|step| step.mode != StepMode::Skipped);
    let Some(dispatched) = applied["data"]["provider_dispatched"].as_bool() else {
        return reject(
            RefusalCode::InvalidResult,
            "v8-runner push did not report whether it dispatched the platform",
        );
    };
    if dispatched != anything_loaded {
        return reject(
            RefusalCode::InvalidResult,
            if anything_loaded {
                "v8-runner reported success without dispatching the platform"
            } else {
                "v8-runner reported dispatching the platform although it skipped every source set"
            },
        );
    }
    if performed != plan {
        // Иной состав или режим значит, что раннер импортировал не тот план,
        // который одобрен: исходники или проектный файл сменились после превью.
        return reject(
            RefusalCode::ConcurrentChange,
            "push performed a different plan than previewed: the sources changed between preview and apply; run dryRun: true again",
        );
    }
    if !anything_loaded {
        return nothing_loaded(prepared, &plan);
    }
    // **Улику о состоянии базы Unica не подделывает.** База живёт за
    // соединением, и её состояние здесь засвидетельствовано провайдером, а не
    // проверено нами; источник признания назван прямо.
    let loaded: Vec<PlannedStep> = plan
        .iter()
        .filter(|step| step.mode != StepMode::Skipped)
        .cloned()
        .collect();
    let skipped_count = plan.len() - loaded.len();
    let mut summary = format!(
        "push imported {}; the infobase state is attested by the provider",
        subject_summary(&loaded)
    );
    if skipped_count > 0 {
        summary.push_str(&format!(
            "; {skipped_count} skipped source set(s) were not loaded and their infobase state was not verified"
        ));
    }
    let mut result = DomainResult::success(summary);
    result.data = Some(json!({
        "op": OPERATION,
        "dryRun": false,
        "providerDispatched": true,
        "full": prepared.arguments.full_rebuild,
        "force": true,
        "generationProtection": false,
        "appliesDatabaseConfiguration": true,
        "steps": public_steps(&plan),
        "targetStateAttestedBy": "provider",
    }));
    // Изменилась база, а не файл рабочего пространства: путь сюда не кладётся.
    // Пропущенный набор раннер в базу не грузил — его в `changed` нет.
    for step in &loaded {
        result.changed.push(json!({
            "infobase": true,
            "kind": "configuration",
            "sourceSet": step.source_set,
            "mode": step.mode.as_str(),
        }));
    }
    if let Some(warning) = skipped_warning(&plan) {
        result.warnings.push(warning);
    }

    result
}

/// Код предупреждения о наборах, которые раннер не загрузил.
const SKIPPED_WARNING_CODE: &str = "infobase_state_unverified";

/// Раннер пропускает набор, когда исходники совпали с его памятью о прошлой
/// загрузке. Память лежит в рабочем каталоге раннера, а не в базе: другой
/// worktree, другой инструмент или отказ на полпути могли изменить базу,
/// не тронув её. Пропуск поэтому не свидетельствует о состоянии базы.
fn skipped_warning(plan: &[PlannedStep]) -> Option<Value> {
    let skipped: Vec<&str> = plan
        .iter()
        .filter(|step| step.mode == StepMode::Skipped)
        .map(|step| step.source_set.as_str())
        .collect();
    (!skipped.is_empty()).then(|| {
        json!({
            "code": SKIPPED_WARNING_CODE,
            "sourceSets": skipped,
            "message": "these source sets were neither loaded nor applied: the runner found no changes against its own memory of earlier loads, which it does not check against the infobase; the infobase state was not verified. If the infobase may have been changed outside this working copy (another worktree, the Designer, a manual load), only a full push restores it; if only this working copy loads it, nothing needs to be done",
        })
    })
}

/// Исполнение, в котором раннер пропустил все наборы: загрузки не было, базу
/// никто не проверял. Ответ так и говорит и не выдаёт пропуск за импорт.
fn nothing_loaded(prepared: &PreparedSourceImport, plan: &[PlannedStep]) -> DomainResult {
    let mut result = DomainResult::success(format!(
        "push loaded nothing into the infobase for {}: the runner found no changes against its own memory; the infobase state was not verified",
        subject_summary(plan)
    ));
    result.data = Some(json!({
        "op": OPERATION,
        "dryRun": false,
        "providerDispatched": false,
        "full": prepared.arguments.full_rebuild,
        "force": true,
        "generationProtection": false,
        "steps": public_steps(plan),
        "targetStateKnownAfterApply": false,
    }));
    result.warnings.extend(skipped_warning(plan));
    result.next.push(full_preview_hint(prepared));
    result
}

/// Полная загрузка не смотрит в память раннера: это путь к известному
/// состоянию базы. Нужна она, только если базу могли изменить вне этой
/// рабочей копии; обычный повтор без правок её не требует. Поэтому причина
/// названа с условием, и предлагается превью, не исполнение.
fn full_preview_hint(prepared: &PreparedSourceImport) -> Value {
    let mut args = public_arguments(prepared);
    args["full"] = Value::Bool(true);
    json!({
        "tool": "unica.run",
        "args": {"op": OPERATION, "args": args, "dryRun": true},
        "reason": "only if the infobase may have been changed outside this working copy: preview a full push, which loads the sources regardless of the runner's memory"
    })
}

fn public_steps(plan: &[PlannedStep]) -> Vec<Value> {
    plan.iter()
        .map(|step| {
            let mut value = step.mode.public();
            value["sourceSet"] = json!(step.source_set);
            value
        })
        .collect()
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
    if cancellation.is_cancelled() {
        return Err(reject(
            RefusalCode::Cancelled,
            "push cancelled before provider launch",
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
    // Набор — позиционный аргумент: `--source-set` и `--full-rebuild` у раннера 0.12
    // скрытые синонимы прежнего словаря.
    if let Some(source_set) = &prepared.arguments.source_set {
        args.push(source_set.clone());
    }
    if prepared.arguments.full_rebuild {
        args.push("--full".to_string());
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
            // A dispatched push may be inside the runner's non-abortable
            // database phase. Cancellation remains effective before launch.
            cancellation: if dry_run {
                cancellation.clone()
            } else {
                cancellation.protect_process_on_spawn()
            },
        })
        .map_err(|error| runner_start_rejection(Some(OPERATION.to_string()), &error))?;
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

fn subject_summary(plan: &[PlannedStep]) -> String {
    match plan {
        [single] => format!("source set `{}`", single.source_set),
        steps => format!("{} source sets", steps.len()),
    }
}

fn public_plan(prepared: &PreparedSourceImport, plan: &[PlannedStep]) -> Value {
    json!({
        "full": prepared.arguments.full_rebuild,
        "force": true,
        "generationProtection": false,
        "appliesDatabaseConfiguration": true,
        "steps": public_steps(plan),
        // Что превью узнать не может, названо, а не умолчано.
        "targetStateKnownBeforeApply": false,
    })
}

fn public_arguments(prepared: &PreparedSourceImport) -> Value {
    let mut args = Map::new();
    args.insert("force".to_string(), Value::Bool(true));
    if let Some(source_set) = &prepared.arguments.source_set {
        args.insert("sourceSet".to_string(), Value::String(source_set.clone()));
    }
    if prepared.arguments.full_rebuild {
        args.insert("full".to_string(), Value::Bool(true));
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
    ) -> PreparedSourceImport {
        PreparedSourceImport {
            arguments: ImportArguments {
                source_set: source_set.map(str::to_string),
                full_rebuild,
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

    /// Конверт `push` раннера 0.12.0 (форма шагов та же, что у `build` 0.9.0 с живой
    /// пробы): шаг на набор с режимом, `partial` приходит объектом с числом файлов.
    fn envelope(steps: &[(&str, Value)], dispatched: bool) -> Value {
        json!({
            "ok": true,
            "command": "push",
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
    fn compatibility_cycle_accepts_explicit_force_and_rejects_silent_overwrite() {
        let root = workspace();
        let request = |args| {
            InvocationRequest::new(
                ToolIdentity::Run,
                json!({"op":"push","args":args,"dryRun":true}),
                root.path().display().to_string(),
                7000,
            )
            .unwrap()
        };
        assert!(PreparedSourceImport::parse(&request(json!({"force":true,"full":true}))).is_ok());
        assert!(PreparedSourceImport::parse(&request(json!({}))).is_err());
        assert!(PreparedSourceImport::parse(&request(json!({"force":false}))).is_err());
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
            &prepared(root.path(), Some("ext-purchases"), false, true),
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
            &prepared(root.path(), None, false, true),
            &runner,
        );

        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["providerDispatched"], false);
        assert_eq!(data["plan"]["full"], false);
        assert_eq!(data["plan"]["steps"][0]["sourceSet"], "main");
        assert_eq!(data["plan"]["steps"][0]["mode"], "full");
        assert_eq!(data["plan"]["steps"][1]["sourceSet"], "ext-sales");
        assert_eq!(data["plan"]["steps"][1]["mode"], "partial");
        assert_eq!(data["plan"]["steps"][1]["files"], 3);
        assert_eq!(data["plan"]["targetStateKnownBeforeApply"], false);
        assert!(result.rev.is_none());
        assert!(result.next[0]["args"].get("ifRev").is_none());
        assert_eq!(result.next[0]["args"]["dryRun"], false);
        assert_eq!(result.next[0]["args"]["args"], json!({"force":true}));
        assert!(result.changed.is_empty());
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("1cv8"), "platform path leaked: {encoded}");
        assert!(!encoded.contains("--config"));
        assert!(runner
            .joined_args(0)
            .ends_with("--json-message push --dry-run"));
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
            &prepared(root.path(), Some("ext-sales"), true, true),
            &runner,
        );

        assert!(result.ok, "{result:?}");
        assert_eq!(
            result.next[0]["args"]["args"],
            json!({"sourceSet": "ext-sales", "full": true, "force":true})
        );
        assert!(result.summary.contains("source set `ext-sales`"));
        assert!(runner
            .joined_args(0)
            .ends_with("push ext-sales --full --dry-run"));

        // Частичный план вопреки полной пересборке — не наш план.
        let runner = SequenceRunner::new(vec![process(
            envelope(&[("ext-sales", partial(1))], false),
            true,
        )]);
        let result = run(
            root.path(),
            &prepared(root.path(), Some("ext-sales"), true, true),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
    }

    #[test]
    fn preview_accepts_cyrillic_and_legacy_ascii_source_set_from_public_run_request() {
        for name in ["Доработки", "1foo"] {
            let root = workspace();
            let config = root.path().join(CONFIG_NAME);
            let yaml = fs::read_to_string(&config)
                .unwrap()
                .replace("ext-sales", name);
            fs::write(config, yaml).unwrap();
            let request = InvocationRequest::new(
                ToolIdentity::Run,
                json!({"op": "push", "args": {"sourceSet": name, "force": true}, "dryRun": true}),
                root.path().display().to_string(),
                7000,
            )
            .unwrap();
            let prepared = PreparedSourceImport::parse(&request).expect("declared source set");
            let runner =
                SequenceRunner::new(vec![process(envelope(&[(name, full())], false), true)]);
            let result = run(root.path(), &prepared, &runner);
            assert!(result.ok, "{name}: {result:?}");
            assert_eq!(
                result.data.as_ref().unwrap()["plan"]["steps"][0]["sourceSet"],
                name
            );
            assert!(runner
                .joined_args(0)
                .ends_with(&format!("push {name} --dry-run")));
            assert!(result.changed.is_empty());
        }
    }

    #[test]
    fn preview_of_all_sets_accepts_cyrillic_runner_step() {
        let root = workspace();
        let config = root.path().join(CONFIG_NAME);
        let yaml = fs::read_to_string(&config)
            .unwrap()
            .replace("ext-sales", "Доработки");
        fs::write(config, yaml).unwrap();
        let request = InvocationRequest::new(
            ToolIdentity::Run,
            json!({"op": "push", "args": {"force": true}, "dryRun": true}),
            root.path().display().to_string(),
            7000,
        )
        .unwrap();
        let prepared = PreparedSourceImport::parse(&request).unwrap();
        let runner = SequenceRunner::new(vec![process(
            envelope(&[("main", full()), ("Доработки", full())], false),
            true,
        )]);
        let result = run(root.path(), &prepared, &runner);
        assert!(result.ok, "{result:?}");
        assert_eq!(
            result.data.as_ref().unwrap()["plan"]["steps"][1]["sourceSet"],
            "Доработки"
        );
        assert!(result.changed.is_empty());
    }

    #[test]
    fn preview_refuses_other_sets_a_dispatched_designer_and_edt_sources() {
        let root = workspace();
        let runner = SequenceRunner::new(vec![process(envelope(&[("main", full())], false), true)]);
        let result = run(
            root.path(),
            &prepared(root.path(), None, false, true),
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
            &prepared(root.path(), None, false, true),
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
            &prepared(root.path(), None, false, true),
            &runner,
        );
        assert_eq!(result.diagnostics[0]["code"], "invalid_state", "{result:?}");
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("Designer sources only"));
    }

    #[test]
    fn direct_apply_checks_its_plan_and_attributes_the_infobase_state_to_the_provider() {
        let root = workspace();
        let plan = [("main", full()), ("ext-sales", partial(3))];
        let runner = SequenceRunner::new(vec![
            process(envelope(&plan, false), true),
            process(envelope(&plan, true), true),
        ]);

        let result = run(
            root.path(),
            &prepared(root.path(), None, false, false),
            &runner,
        );

        assert!(result.ok, "{result:?}");
        assert_eq!(runner.call_count(), 2);
        assert!(runner.joined_args(0).ends_with("--dry-run"));
        assert!(runner.joined_args(1).ends_with("--json-message push"));
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
        assert!(result.rev.is_none());
        // Настоящая загрузка всех наборов отвечает как прежде: импорт,
        // засвидетельствованный провайдером, без оговорок о пропуске.
        assert_eq!(
            result.summary,
            "push imported 2 source sets; the infobase state is attested by the provider"
        );
        assert!(result.warnings.is_empty(), "{result:?}");
        assert!(result.next.is_empty(), "{result:?}");
        assert!(data.get("targetStateKnownAfterApply").is_none());
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("1cv8"), "platform path leaked: {encoded}");
    }

    fn skipped() -> Value {
        json!("skipped")
    }

    /// Пропуск раннера 0.12.0, снятый вживую (#1240): шаг `skipped` с
    /// `no changes`, платформа не запускалась.
    fn skipped_envelope(sets: &[&str]) -> Value {
        let mut envelope = envelope(
            &sets.iter().map(|set| (*set, skipped())).collect::<Vec<_>>(),
            false,
        );
        for step in envelope["data"]["steps"].as_array_mut().unwrap() {
            step["message"] = json!("no changes");
        }
        envelope
    }

    #[test]
    fn apply_where_the_runner_skipped_every_set_names_the_infobase_state_unverified() {
        let root = workspace();
        let sets = ["main", "ext-sales"];
        let runner = SequenceRunner::new(vec![
            process(skipped_envelope(&sets), true),
            process(skipped_envelope(&sets), true),
        ]);

        let result = run(
            root.path(),
            &prepared(root.path(), None, false, false),
            &runner,
        );

        assert!(result.ok, "{result:?}");
        assert_eq!(runner.call_count(), 2);
        assert_eq!(
            result.summary,
            "push loaded nothing into the infobase for 2 source sets: the runner found no changes against its own memory; the infobase state was not verified"
        );
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["providerDispatched"], false);
        assert_eq!(data["targetStateKnownAfterApply"], false);
        assert!(data.get("targetStateAttestedBy").is_none(), "{data}");
        assert!(data.get("appliesDatabaseConfiguration").is_none(), "{data}");
        assert_eq!(data["steps"][0]["mode"], "skipped");
        assert_eq!(data["steps"][1]["mode"], "skipped");
        assert!(result.changed.is_empty(), "{result:?}");
        assert_eq!(result.warnings.len(), 1, "{result:?}");
        assert_eq!(result.warnings[0]["code"], "infobase_state_unverified");
        assert_eq!(
            result.warnings[0]["sourceSets"],
            json!(["main", "ext-sales"])
        );
        // Полная загрузка нужна не всегда: предупреждение называет условие.
        let message = result.warnings[0]["message"].as_str().unwrap();
        assert!(
            message.contains("changed outside this working copy")
                && message.contains("nothing needs to be done"),
            "{message}"
        );
        assert!(result.diagnostics.is_empty(), "{result:?}");
        // Путь к известному состоянию — превью полной загрузки, а не исполнение.
        assert_eq!(result.next.len(), 1, "{result:?}");
        assert_eq!(result.next[0]["args"]["dryRun"], true);
        assert_eq!(
            result.next[0]["args"]["args"],
            json!({"force": true, "full": true})
        );
        assert!(
            result.next[0]["reason"].as_str().unwrap().starts_with(
                "only if the infobase may have been changed outside this working copy"
            ),
            "{:?}",
            result.next[0]
        );

        // Превью того же пропуска говорит то же заранее.
        let runner = SequenceRunner::new(vec![process(skipped_envelope(&sets), true)]);
        let preview = run(
            root.path(),
            &prepared(root.path(), None, false, true),
            &runner,
        );
        assert!(preview.ok, "{preview:?}");
        assert!(
            preview
                .summary
                .starts_with("push planned no load for 2 source sets"),
            "{}",
            preview.summary
        );
        assert_eq!(preview.warnings[0]["code"], "infobase_state_unverified");
        // Первой идёт исполнение текущих аргументов, полная загрузка — второй
        // и с условием.
        assert_eq!(preview.next[0]["args"]["dryRun"], false);
        assert!(preview.next[0]["args"]["args"].get("full").is_none());
        assert_eq!(preview.next[1]["args"]["args"]["full"], true);
        assert_eq!(preview.next[1]["args"]["dryRun"], true);
        assert!(preview.next[1]["reason"]
            .as_str()
            .unwrap()
            .starts_with("only if"));
    }

    #[test]
    fn apply_that_skips_a_previewed_load_is_a_concurrent_change() {
        let root = workspace();
        let sets = ["main", "ext-sales"];
        let runner = SequenceRunner::new(vec![
            process(
                envelope(&[("main", full()), ("ext-sales", partial(3))], false),
                true,
            ),
            process(skipped_envelope(&sets), true),
        ]);
        let result = run(
            root.path(),
            &prepared(root.path(), None, false, false),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "concurrent_change",
            "{result:?}"
        );
    }

    #[test]
    fn apply_names_only_loaded_sets_as_changed_and_the_skipped_ones_as_unverified() {
        let root = workspace();
        let plan = [("main", full()), ("ext-sales", skipped())];
        let runner = SequenceRunner::new(vec![
            process(envelope(&plan, false), true),
            process(envelope(&plan, true), true),
        ]);

        let result = run(
            root.path(),
            &prepared(root.path(), None, false, false),
            &runner,
        );

        assert!(result.ok, "{result:?}");
        assert_eq!(
            result.data.as_ref().unwrap()["targetStateAttestedBy"],
            "provider"
        );
        assert_eq!(result.changed.len(), 1, "{result:?}");
        assert_eq!(result.changed[0]["sourceSet"], "main");
        assert_eq!(result.warnings.len(), 1, "{result:?}");
        assert_eq!(result.warnings[0]["code"], "infobase_state_unverified");
        assert_eq!(result.warnings[0]["sourceSets"], json!(["ext-sales"]));
        assert_eq!(
            result.summary,
            "push imported source set `main`; the infobase state is attested by the provider; 1 skipped source set(s) were not loaded and their infobase state was not verified"
        );
    }

    fn captured(name: &str) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/v8_runner_012")
            .join(name);
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    /// Конверты раннера 0.12.0, снятые вживую на сценарии #1240: в базе
    /// конфигурация другого рабочего каталога, а раннер пропустил набор.
    #[test]
    fn captured_runner_skip_is_answered_as_an_unverified_infobase_state() {
        let root = workspace();
        let runner = SequenceRunner::new(vec![
            process(captured("push-skipped-preview.json"), true),
            process(captured("push-skipped-apply.json"), true),
        ]);
        let result = run(
            root.path(),
            &prepared(root.path(), Some("main"), false, false),
            &runner,
        );
        assert!(result.ok, "{result:?}");
        assert!(
            result.summary.starts_with("push loaded nothing"),
            "{result:?}"
        );
        assert!(result.changed.is_empty(), "{result:?}");
        assert_eq!(result.warnings[0]["code"], "infobase_state_unverified");
        assert_eq!(result.warnings[0]["sourceSets"], json!(["main"]));
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["targetStateKnownAfterApply"], false);
        assert!(data.get("targetStateAttestedBy").is_none(), "{data}");

        // Без признака запуска платформы ответ раннера не принимается.
        let mut apply = captured("push-skipped-apply.json");
        apply["data"]
            .as_object_mut()
            .unwrap()
            .remove("provider_dispatched");
        let runner = SequenceRunner::new(vec![
            process(captured("push-skipped-preview.json"), true),
            process(apply, true),
        ]);
        let result = run(
            root.path(),
            &prepared(root.path(), Some("main"), false, false),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("did not report whether"));
    }

    #[test]
    fn apply_refuses_a_dispatch_flag_that_contradicts_the_steps() {
        let root = workspace();
        // Все наборы пропущены, а раннер говорит, что запускал платформу.
        let sets = [("main", skipped()), ("ext-sales", skipped())];
        let runner = SequenceRunner::new(vec![
            process(envelope(&sets, false), true),
            process(envelope(&sets, true), true),
        ]);
        let result = run(
            root.path(),
            &prepared(root.path(), None, false, false),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("although it skipped every source set"));

        // Набор загружен, а платформа не запускалась.
        let plan = [("main", full()), ("ext-sales", skipped())];
        let runner = SequenceRunner::new(vec![
            process(envelope(&plan, false), true),
            process(envelope(&plan, false), true),
        ]);
        let result = run(
            root.path(),
            &prepared(root.path(), None, false, false),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("without dispatching the platform"));
    }

    #[test]
    fn apply_refuses_a_plan_that_changed_during_execution() {
        let root = workspace();
        let plan = [("main", full()), ("ext-sales", partial(3))];
        // После внутреннего preview этого вызова раннер выбрал
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
            &prepared(root.path(), None, false, false),
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
                "command": "push",
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
                &prepared(root.path(), None, false, true),
                &runner,
            );
            assert_eq!(result.diagnostics[0]["code"], expected, "{code}");
            assert_eq!(map_runner_code(code).outcome(), outcome, "{code}");
        }
    }

    #[test]
    fn push_detaches_only_its_executing_runner_call() {
        let root = workspace();
        let prepared = prepared(root.path(), None, false, false);
        let plan = [("main", full()), ("ext-sales", partial(3))];
        let runner = SequenceRunner::new(vec![process(envelope(&plan, true), true)]);
        let cancellation = CancellationToken::new();
        assert!(
            invoke_runner(&prepared, &tool(root.path()), &runner, &cancellation, false,).is_ok()
        );
        let (_, child) = runner.calls.lock().unwrap()[0]
            .cancellation
            .spawn_with_gate(|| Ok(()))
            .unwrap();
        cancellation.cancel();
        assert!(!child.is_cancelled());
        assert!(cancellation.protected_process_started());

        let runner = SequenceRunner::new(vec![process(envelope(&plan, false), true)]);
        let cancellation = CancellationToken::new();
        assert!(
            invoke_runner(&prepared, &tool(root.path()), &runner, &cancellation, true,).is_ok()
        );
        let (_, child) = runner.calls.lock().unwrap()[0]
            .cancellation
            .spawn_with_gate(|| Ok(()))
            .unwrap();
        cancellation.cancel();
        assert!(child.is_cancelled());
        assert!(!cancellation.protected_process_started());
    }
}
