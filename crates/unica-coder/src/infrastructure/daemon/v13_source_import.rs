#![allow(clippy::result_large_err)]
//! `push` — импорт исходников рабочего пространства в базу силами
//! `v8-runner push` (A-4 зонтика #871). Пара к `pull`: тот выносит
//! базу в исходники, этот вносит исходники в базу.
//!
//! Аргументы закрыты: `sourceSet` — имя одного объявленного набора (без него
//! импортируются все); `full` — сбросить кэш изменений раннера и загрузить всё
//! целиком; `force:true` — перезаписать базу (`push --force`): каждый набор
//! грузится целиком без сверки памяти и поколения базы, сделанное в базе
//! теряется. Без `force` раннер 0.14 перед загрузкой набора сверяет поколение
//! базы с записью о прошлом обмене и отказывает `non_fast_forward`, если база
//! ушла вперёд, или `no_memory`, если памяти о базе нет. Превью зовёт
//! `push --dry-run`: раннер выбирает для каждого набора режим, не запуская
//! конфигуратор. Применение повторяет превью и требует тот же состав наборов
//! и те же режимы: иной режим значит, что исходники изменились между превью и
//! применением.
//!
//! Состояние базы после импорта Unica не проверяет — оно засвидетельствовано
//! провайдером, и ответ называет это прямо. Набор, который раннер пропустил
//! по своей памяти, загружен не был, и поколение базы для него не сверялось:
//! такой ответ называет состояние базы непроверенным, а не импортом. Проза
//! шагов и предупреждений раннера наружу не идёт. Запись в базу другой
//! рабочей копии, которую раннер 0.14 больше не отказывает, ответ называет
//! своим предупреждением `infobase_of_another_copy`.

use super::protocol::InvocationRequest;
use super::runner_014::Runner014ProcessRunner;
use super::v13_infobase_exports::{
    digest_optional_workspace_file, digest_required_workspace_file, missing_runner_rejection,
    note_another_copy, resolve_bundled_runner, runner_rejection, runner_start_rejection,
    CONFIG_NAME, LOCAL_CONFIG_NAME, RUNNER_OUTPUT_LIMIT,
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
    /// `push --force`: перезапись базы без сверки памяти и поколения.
    force: bool,
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
        let force = match args.get("force") {
            None => false,
            Some(Value::Bool(force)) => *force,
            Some(_) => return Err(reject(RefusalCode::BadValue, "push force must be boolean")),
        };
        let mut public = args.clone();
        public.remove("force");
        if public.get("noApply").is_some_and(|v| v != false) {
            return Err(reject(RefusalCode::UnsupportedOperation, "push noApply is not supported by this adapter; default push applies the database configuration"));
        }
        public.remove("noApply");
        if let Some(full) = public.remove("full") {
            public.insert("fullRebuild".into(), full);
        }
        let mut arguments = parse_import_arguments(&public)?;
        arguments.force = force;
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
        execute_with_runner(self, &Runner014ProcessRunner, cancellation)
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
        force: false,
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
                "push planned no load for {}: the runner found no changes against its memory of earlier exchanges; executing loads nothing, does not compare the infobase generation and does not verify the infobase state",
                subject_summary(&plan)
            )
        } else if prepared.arguments.force {
            format!(
                "push planned overwriting the infobase with {} without touching it yet; executing loads every set in full without checking the infobase generation, and changes made in the infobase since this working copy's last exchange are lost",
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
        result.warnings.extend(overwrite_warning(prepared, &plan));
        note_another_copy(&mut result, &preview);
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
        // Если при этом что-то загружено, база уже изменена — отказ говорит
        // это прямо и называет наборы, чтобы следующий шаг не исходил из
        // прежнего состояния базы (#1265).
        let loaded: Vec<&PlannedStep> = performed
            .iter()
            .filter(|step| step.mode != StepMode::Skipped)
            .collect();
        if loaded.is_empty() {
            return reject(
                RefusalCode::ConcurrentChange,
                "push performed a different plan than previewed and loaded nothing: the sources changed between preview and apply; run dryRun: true again",
            );
        }
        let names: Vec<&str> = loaded.iter().map(|step| step.source_set.as_str()).collect();
        let mut result = reject(
            RefusalCode::ConcurrentChange,
            format!(
                "push performed a different plan than previewed and already loaded {} into the infobase: the infobase configuration was changed and applied; the sources changed between preview and apply; run dryRun: true again before relying on the infobase state",
                names.join(", ")
            ),
        );
        for step in loaded {
            result.changed.push(json!({
                "infobase": true,
                "kind": "configuration",
                "sourceSet": step.source_set,
                "mode": step.mode.as_str(),
            }));
        }
        note_another_copy(&mut result, &applied);
        return result;
    }
    if !anything_loaded {
        let mut result = nothing_loaded(prepared, &plan);
        note_another_copy(&mut result, &applied);
        return result;
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
    let mut summary = if prepared.arguments.force {
        format!(
            "push overwrote the infobase with {}: every set was loaded in full without checking the infobase generation, and changes made in the infobase since this working copy's last exchange are lost; the infobase state is attested by the provider",
            subject_summary(&loaded)
        )
    } else {
        format!(
            "push imported {}; before loading, the runner compared the infobase generation with its record of the last exchange wherever it had one; the infobase state is attested by the provider",
            subject_summary(&loaded)
        )
    };
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
        "force": prepared.arguments.force,
        "generationProtection": !prepared.arguments.force,
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
    result.warnings.extend(overwrite_warning(prepared, &plan));
    note_another_copy(&mut result, &applied);

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
            "message": "these source sets were neither loaded nor applied: the runner found no changes against its memory of earlier exchanges with this infobase. It compares the infobase generation only before loading a set, so for these sets it did not check the infobase, and the infobase state was not verified. If the infobase may have been changed outside this working copy (another worktree, the Designer, a manual load), a full push loads the sources and, before loading, is refused with non_fast_forward if the infobase moved ahead of this working copy's record; if only this working copy loads it, nothing needs to be done",
        })
    })
}

/// Код предупреждения о перезаписи базы по `force:true`.
const OVERWRITE_WARNING_CODE: &str = "infobase_overwritten";

/// `force:true` грузит наборы целиком без сверки памяти и поколения: то, что
/// изменили в базе после прошлого обмена этой рабочей копии, теряется. Ответ
/// называет это и в превью, и после исполнения.
fn overwrite_warning(prepared: &PreparedSourceImport, plan: &[PlannedStep]) -> Option<Value> {
    if !prepared.arguments.force {
        return None;
    }
    let sets: Vec<&str> = plan
        .iter()
        .filter(|step| step.mode != StepMode::Skipped)
        .map(|step| step.source_set.as_str())
        .collect();
    (!sets.is_empty()).then(|| {
        json!({
            "code": OVERWRITE_WARNING_CODE,
            "sourceSets": sets,
            "message": if prepared.dry_run {
                "force:true overwrites the infobase: these sets are loaded in full without checking the infobase generation or this working copy's memory of it, and changes made in the infobase since the last exchange are lost"
            } else {
                "force:true overwrote the infobase: these sets were loaded in full without checking the infobase generation or this working copy's memory of it, and changes made in the infobase since the last exchange are lost"
            },
        })
    })
}

/// Исполнение, в котором раннер пропустил все наборы: загрузки не было, базу
/// никто не проверял. Ответ так и говорит и не выдаёт пропуск за импорт.
fn nothing_loaded(prepared: &PreparedSourceImport, plan: &[PlannedStep]) -> DomainResult {
    let mut result = DomainResult::success(format!(
        "push loaded nothing into the infobase for {}: the runner found no changes against its memory of earlier exchanges and, loading nothing, did not compare the infobase generation; the infobase state was not verified",
        subject_summary(plan)
    ));
    result.data = Some(json!({
        "op": OPERATION,
        "dryRun": false,
        "providerDispatched": false,
        "full": prepared.arguments.full_rebuild,
        "force": prepared.arguments.force,
        "generationProtection": !prepared.arguments.force,
        "generationChecked": false,
        "steps": public_steps(plan),
        "targetStateKnownAfterApply": false,
    }));
    result.warnings.extend(skipped_warning(plan));
    result.next.push(full_preview_hint(prepared));
    result
}

/// Полная загрузка не смотрит в хеш-память раннера, но поколение базы перед
/// загрузкой сверяет: ушедшую вперёд базу она не перезапишет, а откажет
/// `non_fast_forward`. Нужна она, только если базу могли изменить вне этой
/// рабочей копии; обычный повтор без правок её не требует. Поэтому причина
/// названа с условием, и предлагается превью, не исполнение.
fn full_preview_hint(prepared: &PreparedSourceImport) -> Value {
    let mut args = public_arguments(prepared);
    args["full"] = Value::Bool(true);
    json!({
        "tool": "unica.run",
        "args": {"op": OPERATION, "args": args, "dryRun": true},
        "reason": "only if the infobase may have been changed outside this working copy: preview a full push; executing it loads every set in full and, before loading, is refused with non_fast_forward if the infobase moved ahead of this working copy's record"
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
    if (prepared.arguments.full_rebuild || prepared.arguments.force)
        && plan.iter().any(|step| step.mode != StepMode::Full)
    {
        return Err(reject(
            RefusalCode::InvalidResult,
            "v8-runner preview planned a partial import despite fullRebuild or force",
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
    // Единственный ключ, который обходит сверку памяти и поколения базы.
    if prepared.arguments.force {
        args.push("--force".to_string());
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
    parse_runner_output(output).map_err(|(rejection, envelope)| {
        let mut result = exchange_refusal(prepared, rejection, &envelope["error"]);
        note_another_copy(&mut result, &envelope);
        result
    })
}

/// Отказ обмена с базой: `non_fast_forward` — база ушла вперёд записанного
/// поколения, `no_memory` — памяти рабочей копии о базе нет.
///
/// Выбор за тем, кто знает, чья правка верна, поэтому отказ ничего не
/// перезаписывает сам и называет оба выхода превью: выгрузить базу в набор
/// (`pull`) или перезаписать базу набором (`push` с `force:true`). Поколения
/// базы и записи идут в `data` так, как их назвал раннер.
fn exchange_refusal(
    prepared: &PreparedSourceImport,
    rejection: DomainResult,
    error: &Value,
) -> DomainResult {
    let Some(code @ ("non_fast_forward" | "no_memory")) = error["code"].as_str() else {
        return rejection;
    };
    debug_assert_eq!(rejection.diagnostics[0]["code"], "invalid_state");
    let source_set = error["next"]["source_set"]
        .as_str()
        .filter(|name| valid_source_set_name(name))
        .map(str::to_owned)
        .or_else(|| prepared.arguments.source_set.clone());
    let mut data = json!({"op": OPERATION, "runnerCode": code});
    if let Some(source_set) = &source_set {
        data["sourceSet"] = json!(source_set);
    }
    for (from, to) in [
        ("base_generation", "baseGeneration"),
        ("local_generation", "localGeneration"),
    ] {
        if let Some(generation) = error[from].as_str() {
            data[to] = json!(generation);
        }
    }
    // Проза раннера здесь не идёт наружу: она называет команды его
    // командной строки с путями проекта, в том числе `push --force`, а выход
    // через Unica — превью в `next`. Факты отказа — в `data`.
    let subject = source_set.as_deref().map_or_else(
        || "the selected source sets".to_string(),
        |set| format!("source set `{set}`"),
    );
    let message = if code == "non_fast_forward" {
        format!(
            "push loaded nothing: the infobase moved ahead of this working copy's record of the last exchange for {subject} (its configuration generation differs from the recorded one; both are in data), so loading would overwrite changes made in the infobase. Choose: pull the infobase into the source set, or overwrite the infobase with push force:true"
        )
    } else {
        format!(
            "push loaded nothing: this working copy has no memory of the infobase for {subject}, so nothing proves that the sources derive from its state. Choose: pull the infobase into the source set, or overwrite the infobase with push force:true"
        )
    };
    let mut rejection =
        DomainResult::canonical_rejection(rejection.at.clone(), RefusalCode::InvalidState, message);
    rejection.data = Some(data);
    let mut pull_args = json!({"force": true});
    if let Some(source_set) = &source_set {
        pull_args["sourceSet"] = json!(source_set);
    }
    rejection.next.push(json!({
        "tool": "unica.run",
        "args": {"op": "pull", "args": pull_args, "dryRun": true},
        "reason": "if the infobase holds the right state: preview a full pull that takes it into the source set, replacing the set and discarding its uncommitted work; an extension source set also needs extension"
    }));
    // Перезапись — того же набора, о котором отказ, а не всех наборов вызова.
    let mut push_args = public_arguments(prepared);
    push_args["force"] = Value::Bool(true);
    if let Some(source_set) = &source_set {
        push_args["sourceSet"] = json!(source_set);
    }
    rejection.next.push(json!({
        "tool": "unica.run",
        "args": {"op": OPERATION, "args": push_args, "dryRun": true},
        "reason": "if the sources hold the right state: preview push with force:true, which loads every set in full and overwrites the infobase; changes made in the infobase since this working copy's last exchange are lost"
    }));
    rejection
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
    if envelope["command"] != RUNNER_COMMAND {
        return Err((
            reject(
                RefusalCode::InvalidResult,
                "v8-runner returned a result for a different operation",
            ),
            Value::Null,
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
        return Err((
            runner_rejection(Some(OPERATION.to_string()), code, message),
            envelope,
        ));
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
        "force": prepared.arguments.force,
        "generationProtection": !prepared.arguments.force,
        "appliesDatabaseConfiguration": true,
        "steps": public_steps(plan),
        // Что превью узнать не может, названо, а не умолчано.
        "targetStateKnownBeforeApply": false,
    })
}

fn public_arguments(prepared: &PreparedSourceImport) -> Value {
    let mut args = Map::new();
    if prepared.arguments.force {
        args.insert("force".to_string(), Value::Bool(true));
    }
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
pub(super) mod tests {
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
                force: false,
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

    /// Без `force` push идёт под защитой раннера: память и поколение базы
    /// сверяются, ключа `--force` в командной строке нет. `force:true` — и
    /// только он — передаёт раннеру `--force`, а ответ называет перезапись.
    #[test]
    fn force_is_optional_and_only_force_overwrites_the_infobase() {
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
        for args in [json!({}), json!({"force": false}), json!({"full": true})] {
            let parsed = PreparedSourceImport::parse(&request(args.clone()))
                .unwrap_or_else(|_| panic!("{args} must be accepted"));
            assert!(!parsed.arguments.force, "{args}");
        }
        let forced = PreparedSourceImport::parse(&request(json!({"force":true,"full":true})))
            .unwrap_or_else(|_| panic!("force:true must be accepted"));
        assert!(forced.arguments.force && forced.arguments.full_rebuild);
        let refused = PreparedSourceImport::parse(&request(json!({"force":"yes"})))
            .expect_err("non-boolean force is refused");
        assert_eq!(refused.diagnostics[0]["code"], "bad_value");

        let runner = SequenceRunner::new(vec![process(
            envelope(&[("main", full()), ("ext-sales", full())], false),
            true,
        )]);
        let mut preview = prepared(root.path(), None, false, true);
        preview.arguments.force = true;
        let result = run(root.path(), &preview, &runner);
        assert!(result.ok, "{result:?}");
        assert!(runner
            .joined_args(0)
            .ends_with("--json-message push --force --dry-run"));
        let plan = &result.data.as_ref().unwrap()["plan"];
        assert_eq!(plan["force"], true);
        assert_eq!(plan["generationProtection"], false);
        assert_eq!(result.warnings[0]["code"], "infobase_overwritten");
        assert_eq!(
            result.warnings[0]["sourceSets"],
            json!(["main", "ext-sales"])
        );
        assert!(result.summary.contains("changes made in the infobase"));
        assert_eq!(result.next[0]["args"]["args"], json!({"force": true}));

        // Перезапись грузит всё целиком: частичный план под force — не наш план.
        let runner = SequenceRunner::new(vec![process(
            envelope(&[("main", full()), ("ext-sales", partial(1))], false),
            true,
        )]);
        let result = run(root.path(), &preview, &runner);
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );

        let runner = SequenceRunner::new(vec![
            process(
                envelope(&[("main", full()), ("ext-sales", full())], false),
                true,
            ),
            process(
                envelope(&[("main", full()), ("ext-sales", full())], true),
                true,
            ),
        ]);
        let mut apply = prepared(root.path(), None, false, false);
        apply.arguments.force = true;
        let result = run(root.path(), &apply, &runner);
        assert!(result.ok, "{result:?}");
        assert!(runner
            .joined_args(1)
            .ends_with("--json-message push --force"));
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["force"], true);
        assert_eq!(data["generationProtection"], false);
        assert_eq!(result.warnings[0]["code"], "infobase_overwritten");
        assert!(result.summary.starts_with("push overwrote the infobase"));
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
        assert_eq!(data["plan"]["force"], false);
        assert_eq!(data["plan"]["generationProtection"], true);
        assert!(result.warnings.is_empty(), "{result:?}");
        assert!(result.rev.is_none());
        assert!(result.next[0]["args"].get("ifRev").is_none());
        assert_eq!(result.next[0]["args"]["dryRun"], false);
        assert_eq!(result.next[0]["args"]["args"], json!({}));
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
            json!({"sourceSet": "ext-sales", "full": true})
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
                json!({"op": "push", "args": {"sourceSet": name}, "dryRun": true}),
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
            "push imported 2 source sets; before loading, the runner compared the infobase generation with its record of the last exchange wherever it had one; the infobase state is attested by the provider"
        );
        assert_eq!(data["force"], false);
        assert_eq!(data["generationProtection"], true);
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
            "push loaded nothing into the infobase for 2 source sets: the runner found no changes against its memory of earlier exchanges and, loading nothing, did not compare the infobase generation; the infobase state was not verified"
        );
        let data = result.data.as_ref().unwrap();
        // Защита поколения включена, но сверки не было: раннер сверяет
        // поколение только перед загрузкой набора.
        assert_eq!(data["generationProtection"], true);
        assert_eq!(data["generationChecked"], false);
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
                && message.contains("did not check the infobase")
                && message.contains("non_fast_forward")
                && message.contains("nothing needs to be done"),
            "{message}"
        );
        assert!(result.diagnostics.is_empty(), "{result:?}");
        // Путь к известному состоянию — превью полной загрузки, а не исполнение.
        assert_eq!(result.next.len(), 1, "{result:?}");
        assert_eq!(result.next[0]["args"]["dryRun"], true);
        // Полная загрузка без force: перед загрузкой она сверит поколение и
        // ушедшую вперёд базу не перезапишет.
        assert_eq!(result.next[0]["args"]["args"], json!({"full": true}));
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
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("loaded nothing"));
        assert!(result.changed.is_empty(), "{result:?}");
    }

    /// Обратная гонка (#1265): превью пропустило все наборы, а исполнение
    /// загрузило. База уже изменена — отказ говорит это и называет наборы,
    /// чтобы следующий шаг не исходил из прежнего состояния базы.
    #[test]
    fn apply_that_loads_a_previewed_skip_names_the_changed_infobase() {
        let root = workspace();
        let sets = ["main", "ext-sales"];
        let runner = SequenceRunner::new(vec![
            process(skipped_envelope(&sets), true),
            process(
                envelope(&[("main", partial(2)), ("ext-sales", skipped())], true),
                true,
            ),
        ]);
        let result = run(
            root.path(),
            &prepared(root.path(), None, false, false),
            &runner,
        );
        assert!(!result.ok);
        assert_eq!(
            result.diagnostics[0]["code"], "concurrent_change",
            "{result:?}"
        );
        let message = result.diagnostics[0]["message"].as_str().unwrap();
        assert!(
            message.contains("already loaded main into the infobase")
                && message.contains("infobase configuration was changed"),
            "{message}"
        );
        assert_eq!(result.changed.len(), 1, "{result:?}");
        assert_eq!(result.changed[0]["infobase"], true);
        assert_eq!(result.changed[0]["sourceSet"], "main");
        assert_eq!(result.changed[0]["mode"], "partial");
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
            "push imported source set `main`; before loading, the runner compared the infobase generation with its record of the last exchange wherever it had one; the infobase state is attested by the provider; 1 skipped source set(s) were not loaded and their infobase state was not verified"
        );
    }

    fn captured(name: &str) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/v8_runner_014")
            .join(name);
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    /// Конверты раннера 0.14.0, снятые вживую на сценарии #1240: базу
    /// перезаписала другая рабочая копия, а раннер этой копии пропустил набор
    /// по своей памяти и поколение базы не сверял.
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

    /// Первые `push` после `infobase create` раннера 0.14.0, снятые вживую
    /// (`tests/fixtures/v8_runner_014/README.md`). Создание собрало базу
    /// с основной конфигурацией и записало память о наборе: первый `push` без
    /// правок ничего не грузит и честно называет состояние базы непроверенным,
    /// а `push` после правки грузит набор без отказа `no_memory`. Записи
    /// поколения до этой загрузки нет, и ответ не выдаёт её за сверенную.
    #[test]
    fn captured_first_pushes_after_create_skip_the_assembled_set_and_load_an_edit() {
        let root = workspace();
        let runner = SequenceRunner::new(vec![
            process(captured("push-after-create-preview.json"), true),
            process(captured("push-after-create-apply.json"), true),
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

        let runner = SequenceRunner::new(vec![
            process(captured("push-after-edit-preview.json"), true),
            process(captured("push-after-edit-apply.json"), true),
        ]);
        let result = run(
            root.path(),
            &prepared(root.path(), Some("main"), false, false),
            &runner,
        );
        assert!(result.ok, "{result:?}");
        assert!(!runner.joined_args(1).contains("--force"));
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["force"], false);
        assert_eq!(data["providerDispatched"], true);
        assert_eq!(data["steps"][0]["mode"], "full");
        assert_eq!(data["targetStateAttestedBy"], "provider");
        assert_eq!(result.changed.len(), 1, "{result:?}");
        assert_eq!(result.changed[0]["sourceSet"], "main");
        assert_eq!(result.changed[0]["mode"], "full");
        // Режим защиты включён, но записи поколения у раннера ещё нет:
        // ответ не выдаёт эту загрузку за сверенную.
        assert_eq!(data["generationProtection"], true);
        assert_eq!(
            result.summary,
            "push imported source set `main`; before loading, the runner compared the infobase generation with its record of the last exchange wherever it had one; the infobase state is attested by the provider"
        );
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
            ("infobase_busy", "concurrent_change", Outcome::RetryAsIs),
            ("non_fast_forward", "invalid_state", Outcome::NeedsHuman),
            ("no_memory", "invalid_state", Outcome::NeedsHuman),
            ("invalid_argument", "bad_value", Outcome::FixCall),
        ] {
            let runner = SequenceRunner::new(vec![process(failure(code), false)]);
            let result = run(
                root.path(),
                &prepared(root.path(), None, false, true),
                &runner,
            );
            assert_eq!(result.diagnostics[0]["code"], expected, "{code}");
            assert_eq!(result.diagnostics[0]["outcome"], outcome.as_str(), "{code}");
            assert_eq!(map_runner_code(code).outcome(), outcome, "{code}");
        }
    }

    /// Живые отказы раннера 0.14.0 (`tests/fixtures/v8_runner_014/`): база ушла
    /// вперёд записи и памяти о базе нет. Unica ничего не перезаписывает сама:
    /// отказ несёт оба поколения и два выхода превью — выгрузку набора и
    /// перезапись базы через `force:true`.
    #[test]
    fn captured_exchange_refusals_name_both_ways_out_and_the_generations() {
        let root = workspace();
        let runner = SequenceRunner::new(vec![
            process(captured("push-full-preview.json"), true),
            process(captured("push-non-fast-forward-apply.json"), false),
        ]);
        let result = run(
            root.path(),
            &prepared(root.path(), Some("main"), true, false),
            &runner,
        );
        assert!(runner.joined_args(1).ends_with("push main --full"));
        assert_eq!(result.diagnostics[0]["code"], "invalid_state", "{result:?}");
        assert_eq!(result.diagnostics[0]["outcome"], "needsHuman");
        let message = result.diagnostics[0]["message"].as_str().unwrap();
        assert!(message.contains("moved ahead"), "{message}");
        // Команды командной строки раннера и пути проекта наружу не идут.
        assert!(
            !message.contains("v8-runner")
                && !message.contains("--force")
                && !message.contains("/workspace"),
            "{message}"
        );
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["runnerCode"], "non_fast_forward");
        assert_eq!(data["sourceSet"], "main");
        assert_eq!(
            data["baseGeneration"],
            "cff9bf3b55c7644487128a9f7787059000000000"
        );
        assert_eq!(
            data["localGeneration"],
            "ed175a7e431ea74a9b3d8def289ced9700000000"
        );
        assert!(result.changed.is_empty(), "{result:?}");
        assert_eq!(result.next.len(), 2, "{result:?}");
        assert_eq!(result.next[0]["args"]["op"], "pull");
        assert_eq!(result.next[0]["args"]["dryRun"], true);
        assert_eq!(
            result.next[0]["args"]["args"],
            json!({"force": true, "sourceSet": "main"})
        );
        assert_eq!(result.next[1]["args"]["op"], "push");
        assert_eq!(result.next[1]["args"]["dryRun"], true);
        assert_eq!(
            result.next[1]["args"]["args"],
            json!({"force": true, "sourceSet": "main", "full": true})
        );

        // Превью без памяти о базе отказывает тем же родом и до запуска платформы.
        let runner = SequenceRunner::new(vec![process(
            captured("push-no-memory-preview.json"),
            false,
        )]);
        let result = run(
            root.path(),
            &prepared(root.path(), None, false, true),
            &runner,
        );
        assert_eq!(runner.call_count(), 1);
        assert_eq!(result.diagnostics[0]["code"], "invalid_state", "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["runnerCode"], "no_memory");
        assert_eq!(data["sourceSet"], "main");
        assert!(data.get("baseGeneration").is_none(), "{data}");
        assert_eq!(result.next[0]["args"]["op"], "pull");
        // Перезапись — того набора, о котором отказ, а не всех наборов.
        assert_eq!(
            result.next[1]["args"]["args"],
            json!({"force": true, "sourceSet": "main"})
        );
    }

    /// Живой отказ занятой базы (раннер 0.14.0): другая команда держит базу —
    /// повтор. База другой рабочей копии отказом больше не бывает: раннер 0.14
    /// пишет в неё с предупреждением, а `force:true` называет перезапись.
    #[test]
    fn captured_base_contention_refusals_keep_their_outcomes() {
        let root = workspace();
        let runner = SequenceRunner::new(vec![
            process(captured("push-full-preview.json"), true),
            process(captured("push-infobase-busy-apply.json"), false),
        ]);
        let result = run(
            root.path(),
            &prepared(root.path(), Some("main"), true, false),
            &runner,
        );
        assert_eq!(
            result.diagnostics[0]["code"], "concurrent_change",
            "{result:?}"
        );
        assert_eq!(result.diagnostics[0]["outcome"], "retry");
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("retry when it finishes"));

        let runner = SequenceRunner::new(vec![
            process(captured("push-other-copy-force-preview.json"), true),
            process(captured("push-other-copy-force-apply.json"), true),
        ]);
        let mut forced = prepared(root.path(), Some("main"), false, false);
        forced.arguments.force = true;
        let result = run(root.path(), &forced, &runner);
        assert!(result.ok, "{result:?}");
        assert!(runner.joined_args(1).ends_with("push main --force"));
        assert_eq!(result.warnings[0]["code"], "infobase_overwritten");
        // Раннер 0.14 пишет в базу другой рабочей копии с предупреждением.
        // Ответ называет его своим предупреждением и выходом к своей базе —
        // ни в превью, ни после исполнения не теряет и прозы раннера не несёт.
        assert_another_copy_is_named(&result);

        let runner = SequenceRunner::new(vec![process(
            captured("push-other-copy-force-preview.json"),
            true,
        )]);
        forced.dry_run = true;
        let preview = run(root.path(), &forced, &runner);
        assert!(preview.ok, "{preview:?}");
        assert_another_copy_is_named(&preview);

        // Отказ без памяти в чужой копии называет её так же, третьим выходом.
        let runner = SequenceRunner::new(vec![process(
            captured("push-no-memory-preview.json"),
            false,
        )]);
        let refused = run(
            root.path(),
            &prepared(root.path(), None, false, true),
            &runner,
        );
        assert_eq!(refused.diagnostics[0]["code"], "invalid_state");
        assert_another_copy_is_named(&refused);
        assert_eq!(refused.next[0]["args"]["op"], "pull");
        assert_eq!(refused.next[1]["args"]["op"], "push");
    }

    /// Своё предупреждение Unica о базе другой копии: код, текст без путей,
    /// метки владельца и команд раннера, выход — превью `infobase.create`.
    /// Прочие предупреждения раннера наружу не идут.
    pub(in super::super) fn assert_another_copy_is_named(result: &DomainResult) {
        let named: Vec<&Value> = result
            .warnings
            .iter()
            .filter(|warning| warning["code"] == "infobase_of_another_copy")
            .collect();
        assert_eq!(named.len(), 1, "{result:?}");
        assert!(
            result
                .warnings
                .iter()
                .all(|warning| warning["code"] != "runner_warning"),
            "{result:?}"
        );
        let encoded = serde_json::to_string(result).unwrap();
        for leaked in [
            "/workspace",
            "owners.json",
            "v8-runner",
            "--from",
            "init --infobase",
            "held by the working copy",
        ] {
            assert!(!encoded.contains(leaked), "{leaked} leaked: {encoded}");
        }
        let create = result
            .next
            .iter()
            .find(|next| next["args"]["op"] == "infobase.create")
            .unwrap_or_else(|| panic!("no infobase.create preview: {result:?}"));
        assert_eq!(create["args"]["dryRun"], true);
        assert_eq!(create["args"]["args"], json!({}));
    }

    /// Раннер 0.14.0 убрал ключ `shared` секции базы. Unica его не пишет и не
    /// толкует: старый местный слой с ним доходит до вызывающего типизированным
    /// отказом раннера, а не сбоем и не молчаливым пропуском.
    #[test]
    fn captured_removed_shared_key_is_a_typed_refusal() {
        let root = workspace();
        let runner = SequenceRunner::new(vec![process(captured("shared-key-refused.json"), false)]);
        let result = run(
            root.path(),
            &prepared(root.path(), None, false, true),
            &runner,
        );
        assert_eq!(runner.call_count(), 1);
        assert_eq!(result.diagnostics[0]["code"], "bad_value", "{result:?}");
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("unknown field `shared`"));
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
