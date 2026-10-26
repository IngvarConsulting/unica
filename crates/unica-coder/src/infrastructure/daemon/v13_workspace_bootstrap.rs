use super::protocol::InvocationRequest;
use crate::application::v13::check::{
    page_root_verdict, RootCheckQuestion, DEFAULT_CHECK_LIMIT, MAX_CHECK_LIMIT,
};
use unica_execution::invocation::{InvocationResponseDeadline, INVOCATION_HANDOFF_WINDOW};
use unica_execution::invocation_store::ToolIdentity;
use unica_project_health::evaluate_project_health;
use unica_source_contract::{ProjectSourceMap, SourceFormat, SourceSetKind};

use crate::infrastructure::project_sources::discover_project_source_map_controlled;
use crate::infrastructure::workspace::discover_workspace;
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use unica_address::address::QualifiedAddress;
use unica_execution_contract::invocation::{DomainResult, InvocationFailure, SafeIdentityHash};
use unica_navigation::result_store::ViewCursorStore;
use unica_project_health::inspection::{
    inspect_project_health_continued,
    resources::{ResourceContinuation, RootCheckContinuationStore},
};
use unica_refusal::RefusalCode;
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::{cancelled_error, CancellationToken};
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::discovery::ProjectDiscoveryReader;
use unica_workspace::paths::normalize_path_identity;
use unica_workspace::read::read_root_relative_regular_file;

const PROJECT_CONFIG_MAX_BYTES: usize = 8 * 1024 * 1024;

#[cfg(test)]
pub(super) mod test_control;

#[derive(Debug, Clone, PartialEq, Eq)]
struct InfobaseTarget {
    configured: bool,
    source: Option<&'static str>,
    /// The layers that name the infobase by a top-level `connection`: the
    /// former runner format, which v8-runner 0.15 refuses with the whole
    /// configuration (#2155).
    legacy_connection: Vec<&'static str>,
}

/// Какой вопрос задан корню рабочего пространства.
///
/// Разделение то же, что на узле: `view` отвечает фактами, `check` — вердиктом.
/// Корень был единственным местом, где это стояло наоборот.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootQuestion {
    /// Что здесь есть: корень, конфигурация, наборы, база, рекомендуемая
    /// заготовка `v8project.yaml`.
    Facts,
    /// Здорово ли: готовность, проверки и диагностики с советом.
    Verdict,
}

pub(super) struct PreparedWorkspaceInspection {
    context: WorkspaceContext,
    requested_directory: String,
    origin: Option<unica_bootstrap::WorkspaceOrigin>,
    question: RootQuestion,
    response_deadline: InvocationResponseDeadline,
    workspace_identity_hash: SafeIdentityHash,
    continuation: Option<Arc<Mutex<ResourceContinuation>>>,
    pages: Arc<ViewCursorStore>,
    limit: usize,
    cursor: Option<String>,
}

pub(super) enum Preparation {
    NotApplicable,
    Rejected(Box<DomainResult>),
    Ready(Arc<PreparedWorkspaceInspection>),
}

pub(super) fn prepare(
    request: &InvocationRequest,
    response_deadline: InvocationResponseDeadline,
    continuations: &RootCheckContinuationStore,
    pages: &Arc<ViewCursorStore>,
) -> Preparation {
    if request.arguments().contains_key("at") {
        return Preparation::NotApplicable;
    }
    let question = match request.tool() {
        ToolIdentity::View => RootQuestion::Facts,
        ToolIdentity::Check => RootQuestion::Verdict,
        _ => return Preparation::NotApplicable,
    };
    if question == RootQuestion::Facts && !request.arguments().is_empty() {
        let mut result = DomainResult::canonical_rejection(
            None,
            RefusalCode::BadValue,
            "view limit and cursor require logical argument `at`; call unica.view with an empty object to inspect the workspace",
        );
        result.next.push(next_action(
            "unica.view",
            Value::Object(Map::new()),
            "discover source sets and canonical logical addresses",
        ));
        return Preparation::Rejected(Box::new(result));
    }
    let (limit, cursor) = match root_check_page(request.arguments()) {
        Ok(page) => page,
        Err(summary) => {
            return Preparation::Rejected(Box::new(DomainResult::canonical_rejection(
                None,
                RefusalCode::BadValue,
                summary,
            )))
        }
    };

    let discovered =
        discover_workspace(Some(PathBuf::from(request.workspace_hint()))).and_then(|context| {
            let canonical_root = normalize_path_identity(&context.workspace_root)?;
            Ok((context, canonical_root))
        });
    let (context, canonical_root) = match discovered {
        Ok(discovered) => discovered,
        Err(error) => {
            return Preparation::Rejected(Box::new(DomainResult::canonical_rejection(
                None,
                RefusalCode::ProviderUnavailable,
                format!("workspace discovery failed: {error}"),
            )))
        }
    };
    let mut hasher = Sha256::new();
    hasher.update(b"unica-v13-workspace-inspection-v1\0");
    hasher.update(canonical_root.as_os_str().as_encoded_bytes());
    // A continuation reads the saved verdict and inspects nothing.
    let continuation = (question == RootQuestion::Verdict && cursor.is_none())
        .then(|| continuations.for_workspace(&canonical_root))
        .flatten();
    Preparation::Ready(Arc::new(PreparedWorkspaceInspection {
        context,
        requested_directory: request.workspace_hint().to_owned(),
        origin: request.workspace_origin().cloned(),
        question,
        response_deadline,
        workspace_identity_hash: SafeIdentityHash::from_sha256(hasher.finalize().into()),
        continuation,
        pages: Arc::clone(pages),
        limit,
        cursor,
    }))
}

/// The page a root check asks for: `limit` and `cursor`, nothing else. An
/// empty or null cursor is omitted before this point and opens page one.
fn root_check_page(arguments: &Map<String, Value>) -> Result<(usize, Option<String>), String> {
    if let Some(unknown) = arguments
        .keys()
        .find(|name| !matches!(name.as_str(), "limit" | "cursor"))
    {
        return Err(format!(
            "root check does not accept `{unknown}`; call unica.check with an empty object for the workspace verdict"
        ));
    }
    let limit = match arguments.get("limit") {
        None => DEFAULT_CHECK_LIMIT,
        Some(value) => value
            .as_u64()
            .and_then(|limit| usize::try_from(limit).ok())
            .filter(|limit| (1..=MAX_CHECK_LIMIT).contains(limit))
            .ok_or_else(|| {
                format!("check limit must be an integer from 1 through {MAX_CHECK_LIMIT}")
            })?,
    };
    let cursor = match arguments.get("cursor") {
        None => None,
        Some(Value::String(cursor)) => Some(cursor.clone()),
        Some(_) => return Err("check cursor must be a string".to_string()),
    };
    Ok((limit, cursor))
}

impl PreparedWorkspaceInspection {
    pub(super) fn workspace_identity_hash(&self) -> &SafeIdentityHash {
        &self.workspace_identity_hash
    }

    pub(super) fn response_deadline(&self) -> &InvocationResponseDeadline {
        &self.response_deadline
    }

    pub(super) fn execute(
        &self,
        cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure> {
        let question = RootCheckQuestion::new(self.workspace_identity_hash.as_str(), self.limit);
        if let Some(cursor) = &self.cursor {
            check_cancellation(&cancellation)?;
            return Ok(page_root_verdict(
                &self.pages,
                &question,
                None,
                Some(cursor),
            ));
        }
        let mut result = self.inspect(cancellation)?;
        annotate_workspace_origin(&mut result, &self.requested_directory, self.origin.as_ref());
        if self.question == RootQuestion::Verdict {
            result = page_root_verdict(&self.pages, &question, Some(result), None);
        }
        Ok(result)
    }

    fn inspect(&self, cancellation: CancellationToken) -> Result<DomainResult, InvocationFailure> {
        check_cancellation(&cancellation)?;
        let context = &self.context;
        let config_present = project_config_present(&context.workspace_root);
        let mut checkpoint = || {
            if cancellation.is_cancelled() {
                Err(cancelled_error("workspace inspection cancelled"))
            } else {
                Ok(())
            }
        };
        let discovery =
            discover_project_source_map_controlled(&context.workspace_root, &mut checkpoint);
        check_cancellation(&cancellation)?;
        let source_map = match discovery {
            Ok(source_map) => source_map,
            Err(error) if config_present => {
                let mut result = DomainResult::canonical_rejection(
                    None,
                    RefusalCode::InvalidState,
                    format!("v8project.yaml is present but invalid: {error}"),
                );
                result.data = Some(object([
                    ("workspaceRoot", value(&context.workspace_root)),
                    (
                        "config",
                        object([
                            ("state", Value::String("invalid".to_string())),
                            ("path", Value::String("v8project.yaml".to_string())),
                        ]),
                    ),
                ]));
                return Ok(result);
            }
            Err(error) => {
                return Ok(DomainResult::canonical_rejection(
                    None,
                    RefusalCode::ProviderUnavailable,
                    format!("workspace source discovery failed: {error}"),
                ))
            }
        };
        let inspected_infobase = inspect_infobase_target(&context.workspace_root, config_present);
        check_cancellation(&cancellation)?;
        let infobase = match inspected_infobase {
            Ok(target) => target,
            Err(error) => {
                let mut result = DomainResult::canonical_rejection(
                    None,
                    RefusalCode::InvalidState,
                    format!("infobase target configuration is invalid: {error}"),
                );
                result.data = Some(object([
                    ("workspaceRoot", value(&context.workspace_root)),
                    (
                        "config",
                        object([
                            ("state", Value::String("invalid".to_string())),
                            ("path", Value::String("v8project.yaml".to_string())),
                        ]),
                    ),
                ]));
                return Ok(result);
            }
        };
        let mut continuation = self
            .continuation
            .as_ref()
            .and_then(|state| state.try_lock().ok());
        if let Some(state) = continuation.as_deref_mut() {
            state.reset_progress();
        }
        let result = bootstrap_result(
            context,
            source_map,
            infobase,
            self.question,
            &cancellation,
            continuation.as_deref_mut(),
        );
        check_cancellation(&cancellation)?;
        Ok(result)
    }
}

fn check_cancellation(cancellation: &CancellationToken) -> Result<(), InvocationFailure> {
    if cancellation.is_cancelled() {
        Err(InvocationFailure::new(
            "cancelled",
            "workspace inspection cancelled",
        ))
    } else {
        Ok(())
    }
}

fn bootstrap_result(
    context: &unica_source_contract::workspace::WorkspaceContext,
    source_map: ProjectSourceMap,
    infobase: InfobaseTarget,
    question: RootQuestion,
    cancellation: &CancellationToken,
    mut continuation: Option<&mut ResourceContinuation>,
) -> DomainResult {
    let config_state = if source_map.config_path.is_some() {
        "configured"
    } else if source_map.source_sets.is_empty() {
        "missing"
    } else {
        "autodetected"
    };
    let external_only = matches!(
        unica_source_designer::selection::select_project_default_source_set(
            &source_map.source_sets
        ),
        Ok(None)
    );
    let discovered_ready = (source_map.effective_source_set.is_some() || external_only)
        && source_map.source_selection_error.is_none()
        && !source_map.source_sets.is_empty()
        && source_map.source_sets.iter().all(|source| {
            source.source_format == SourceFormat::PlatformXml
                && source.source_state == unica_source_contract::SourceSetState::Supported
        });
    // `view {}` отвечает фактами. Здоровье проекта (репозиторий, атрибуты,
    // рабочие файлы) принадлежит вердикту `check {}` и для фактов не
    // вычисляется (#1457).
    let facts_only = question == RootQuestion::Facts;
    #[cfg(test)]
    if !source_map.source_sets.is_empty() {
        test_control::pause_before_root_answer(&context.workspace_root);
    }
    let health = if source_map.source_sets.is_empty() || facts_only {
        None
    } else {
        // Проверка здоровья проекта идёт в задаче до конца: прежде её
        // обрезало окно передачи в 7 с, и ответ приходил порцией (#1251).
        let inspection = inspect_project_health_continued(
            context,
            cancellation,
            ProviderDeadline::no_deadline(),
            continuation.as_deref_mut(),
            &ProjectDiscoveryReader::new(&context.workspace_root),
            INVOCATION_HANDOFF_WINDOW,
        );
        Some(
            inspection
                .map_err(|error| format!("{error:?}"))
                .and_then(evaluate_project_health),
        )
    };
    let (mut ready, repository_ready, checks, mut diagnostics, readiness_state) = match health {
        None => (
            infobase.configured,
            false,
            Value::Array(Vec::new()),
            if infobase.configured {
                Value::Array(Vec::new())
            } else {
                Value::Array(vec![object([
                    ("code", Value::String("source_roots_missing".to_string())),
                    (
                        "message",
                        Value::String(if config_state == "configured" {
                            "v8project.yaml is present, but it declares neither source sets nor an infobase connection. Add the input that matches the intended operation."
                        } else {
                            "No v8project.yaml or 1C source roots were found. Call unica.run with an empty object to inspect source, CF/DT, and existing-infobase initialization routes."
                        }.to_string()),
                    ),
                ])])
            },
            "complete",
        ),
        Some(Ok(report)) => {
            let readiness_state = if report.inspection_complete {
                "complete"
            } else {
                "incomplete"
            };
            (
                report.ready,
                report.repository_ready,
                serde_json::to_value(report.checks).expect("project checks serialize"),
                serde_json::to_value(report.diagnostics).expect("project diagnostics serialize"),
                readiness_state,
            )
        }
        Some(Err(reason)) => (
            false,
            false,
            Value::Array(Vec::new()),
            Value::Array(vec![object([
                (
                    "code",
                    Value::String("project_health_incomplete".to_string()),
                ),
                ("message", Value::String(reason)),
            ])]),
            "incomplete",
        ),
    };
    if facts_only && !source_map.source_sets.is_empty() {
        // Ссылка на чтение корня зависит от выбора набора, а не от
        // готовности репозитория: её сообщает `check {}`.
        ready = discovered_ready;
    }
    let actionable_source = source_map.source_sets.iter().find(|source| {
        source.source_format == SourceFormat::PlatformXml
            && source.name
                == source_map
                    .effective_source_set
                    .as_deref()
                    .unwrap_or_default()
            && matches!(
                source.kind,
                SourceSetKind::Configuration | SourceSetKind::Extension
            )
    });
    let next_address = actionable_source.map(|source| {
        let encoded = format!("{}:Configuration", source.name);
        QualifiedAddress::parse(&encoded).map(|address| address.to_string())
    });
    let external_addresses = if external_only {
        source_map
            .source_sets
            .iter()
            .map(|source| {
                let kind = if source.kind == SourceSetKind::ExternalProcessor {
                    "ExternalDataProcessor"
                } else {
                    "ExternalReport"
                };
                (
                    source,
                    QualifiedAddress::parse(&format!("{}:{kind}", source.name))
                        .map(|at| at.to_string()),
                )
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let invalid_source_name = if next_address.as_ref().is_some_and(Result::is_err) {
        actionable_source.map(|source| source.name.as_str())
    } else {
        external_addresses
            .iter()
            .find(|(_, at)| at.is_err())
            .map(|(source, _)| source.name.as_str())
    };
    if let (true, Some(source_name)) = (ready, invalid_source_name) {
        ready = false;
        if let Value::Array(items) = &mut diagnostics {
            items.push(object([
                (
                    "code",
                    Value::String("source_set.logical_name_invalid".to_string()),
                ),
                ("severity", Value::String("error".to_string())),
                ("scope", Value::String("sourceSet".to_string())),
                ("sourceSet", Value::String(source_name.to_string())),
                ("paths", Value::Array(Vec::new())),
                ("count", Value::Number(1.into())),
                (
                    "message",
                    Value::String(
                        "The effective source-set name cannot be encoded as a canonical logical address"
                            .to_string(),
                    ),
                ),
                ("evidence", Value::Array(Vec::new())),
                (
                    "remediation",
                    object([
                        (
                            "summary",
                            Value::String(
                                "Rename the source set to a Unicode XML NCName".to_string(),
                            ),
                        ),
                        (
                            "steps",
                            Value::Array(vec![
                                Value::String(
                                    "Choose a source-set name without whitespace, colons, or path separators"
                                        .to_string(),
                                ),
                                Value::String(
                                    "Update the name in v8project.yaml or rename the autodetected source directory"
                                        .to_string(),
                                ),
                                Value::String(
                                    "Run unica.view with an empty object again".to_string(),
                                ),
                            ]),
                        ),
                        ("commands", Value::Array(Vec::new())),
                    ]),
                ),
            ]));
        }
    }
    if !infobase.legacy_connection.is_empty() {
        ready = false;
        if let Value::Array(items) = &mut diagnostics {
            items.retain(|item| item["code"] != "source_roots_missing");
            items.push(legacy_connection_finding(&infobase.legacy_connection));
        }
    }
    // Without `tools.platform` the platform version is left to the runner's
    // own choice. A warning: readiness stays.
    if config_state == "configured" && !platform_pinned(&context.workspace_root) {
        if let Value::Array(items) = &mut diagnostics {
            items.push(platform_unpinned_finding());
        }
    }
    // A former top-level `connection` is the one thing to fix first: the
    // source-set example would lead away from it.
    let setup = if !infobase.legacy_connection.is_empty() {
        None
    } else if config_state == "configured"
        && source_map.source_sets.is_empty()
        && !infobase.configured
    {
        Some(object([
            ("path", Value::String("v8project.yaml".to_string())),
            ("content", Value::Null),
            (
                "sourceSetExample",
                object([
                    ("name", Value::String("main".to_string())),
                    ("type", Value::String("CONFIGURATION".to_string())),
                    ("path", Value::String("src".to_string())),
                ]),
            ),
            (
                "reason",
                Value::String(
                    "Add or replace only the source-set field using this example while preserving every other v8project.yaml field and comment; do not replace the file. The example expects a Configurator XML export in src/."
                        .to_string(),
                ),
            ),
        ]))
    } else if config_state != "configured" {
        match project_config_recipe(&source_map) {
            Some(content) if !source_map.source_sets.is_empty() => Some(object([
                ("path", Value::String("v8project.yaml".to_string())),
                ("content", Value::String(content)),
                ("sourceSetExample", Value::Null),
                ("reason", Value::String(if source_map.source_sets.is_empty() {
                    "Choose a workspace-relative path containing a Configurator XML export, then create this project file. The example expects the export in src/."
                } else {
                    "Persist the autodetected source sets so future discovery is explicit and stable."
                }.to_string())),
            ])),
            Some(_) => Some(object([
                ("path", Value::String("v8project.yaml".to_string())),
                ("content", Value::Null),
                ("sourceSetExample", Value::Null),
                (
                    "reason",
                    Value::String(
                        "No initialization input is selected yet; inspect source, CF/DT, and existing-infobase routes before creating v8project.yaml."
                            .to_string(),
                    ),
                ),
            ])),
            None => Some(object([
                ("path", Value::String("v8project.yaml".to_string())),
                ("content", Value::Null),
                ("sourceSetExample", Value::Null),
                (
                    "reason",
                    Value::String(
                        "No effective source set with a known format was selected, so a global format cannot be chosen safely. Resolve source selection or create the project config manually after choosing one format."
                            .to_string(),
                    ),
                ),
            ])),
        }
    } else {
        None
    };

    let source_selection_error = if infobase.configured && source_map.source_sets.is_empty() {
        None
    } else {
        source_map.source_selection_error.as_deref()
    };
    if question == RootQuestion::Verdict {
        // Вердикт говорит теми же словами, что и вердикт по узлу: `status`
        // и диагностики. `ok` остаётся истиной — неготовое пространство
        // законно, это факт о нём, а не сбой вызова.
        let mut result = DomainResult::success(if ready {
            "workspace is ready"
        } else {
            "workspace readiness reported findings"
        });
        let mut data = object([
            (
                "status",
                Value::String(if ready { "passed" } else { "failed" }.to_string()),
            ),
            ("ready", Value::Bool(ready)),
            ("discoveredReady", Value::Bool(discovered_ready)),
            ("repositoryReady", Value::Bool(repository_ready)),
            ("readinessState", Value::String(readiness_state.to_string())),
            ("checks", checks),
            ("diagnostics", diagnostics),
        ]);
        if readiness_state == "incomplete" {
            if let Some(progress) = continuation.as_ref().map(|state| state.progress()) {
                if progress.text_resources > 0 {
                    if let Value::Object(fields) = &mut data {
                        fields.insert(
                            "inspectionProgress".into(),
                            object([
                                ("textResources", value(progress.text_resources)),
                                ("stagedEolRetained", value(progress.staged_eol_retained)),
                                ("workingEolRetained", value(progress.working_eol_retained)),
                            ]),
                        );
                    }
                }
            }
        }
        result.data = Some(data);
        if !ready {
            result.next.push(next_action(
                "unica.view",
                Value::Object(Map::new()),
                "наборы, база и рекомендуемое содержимое v8project.yaml",
            ));
        }
        if let (true, Some(Ok(at))) = (ready, next_address) {
            result.next.push(next_action(
                "unica.view",
                object([("at", Value::String(at))]),
                "inspect the root logical node of the selected source set",
            ));
        }
        return result;
    }
    let source_sets = serde_json::to_value(&source_map.source_sets)
        .expect("project source sets always serialize");
    let mut result = DomainResult::success(match (config_state, infobase.configured, source_map.source_sets.is_empty()) {
        (_, _, false) if external_only => "external artifact source sets discovered; no configuration default is needed; inspect owners by their qualified source-set address",
        ("configured", true, true) => "workspace configuration and infobase target discovered; no source sets are attached",
        ("configured", _, _) => "workspace configuration and source sets discovered",
        ("autodetected", _, _) => "source sets autodetected; v8project.yaml is not present",
        _ => "workspace is uninitialized; no v8project.yaml or 1C source roots were found",
    });
    result.data = Some(object([
        ("workspaceRoot", value(&context.workspace_root)),
        (
            "config",
            object([
                ("state", Value::String(config_state.to_string())),
                ("path", Value::String("v8project.yaml".to_string())),
            ]),
        ),
        ("sourceSets", source_sets),
        (
            "infobase",
            object([
                ("configured", Value::Bool(infobase.configured)),
                ("source", value(infobase.source)),
            ]),
        ),
        (
            "effectiveSourceSet",
            value(&source_map.effective_source_set),
        ),
        (
            "effectiveSourceRoot",
            value(&source_map.effective_source_root),
        ),
        ("sourceSelectionError", value(source_selection_error)),
        (
            "setup",
            setup
                .map(|setup| with_local_recipe(setup, &context.workspace_root))
                .unwrap_or(Value::Null),
        ),
    ]));
    if !infobase.legacy_connection.is_empty() {
        result
            .warnings
            .push(legacy_connection_finding(&infobase.legacy_connection));
    }
    if config_state == "missing" {
        result.next.push(next_action(
            "unica.run",
            Value::Object(Map::new()),
            "inspect the implemented and planned workspace initialization routes",
        ));
    }
    // Подсказка-действие ушла вместе с операцией: проектный файл заводит тот,
    // кто читает ответ, своими файловыми средствами. На её месте — само
    // рекомендуемое содержимое в `setup`, а вопрос в `next` остаётся вопросом.
    if infobase.configured && source_map.source_sets.is_empty() {
        result.next.push(next_action(
            "unica.run",
            object([
                ("op", Value::String("download".to_string())),
                (
                    "args",
                    object([
                        ("state", Value::String("working".to_string())),
                        ("output", Value::String("dist/main.cf".to_string())),
                    ]),
                ),
                ("dryRun", Value::Bool(true)),
            ]),
            "preview export of the working main configuration without changing the infobase",
        ));
        result.next.push(next_action(
            "unica.run",
            object([
                ("op", Value::String("infobase.dump".to_string())),
                (
                    "args",
                    object([("output", Value::String("dist/base.dt".to_string()))]),
                ),
                ("dryRun", Value::Bool(true)),
            ]),
            "preview a full DT snapshot export without changing the infobase",
        ));
    }
    if let (true, Some(Ok(at))) = (ready, next_address) {
        result.next.push(next_action(
            "unica.view",
            object([("at", Value::String(at))]),
            "inspect the root logical node of the selected source set",
        ));
    }
    if external_only && source_map.source_selection_error.is_none() {
        for (_, address) in external_addresses {
            if let Ok(at) = address {
                result.next.push(next_action(
                    "unica.view",
                    object([("at", Value::String(at))]),
                    "inspect external owners in this source set",
                ));
            }
        }
    }
    // Вердикт живёт в `check {}`, и спросить его уместно всегда — в том числе
    // на ненастроенном пространстве, где вопрос «а что не так» и есть главный.
    result.next.push(next_action(
        "unica.check",
        Value::Object(Map::new()),
        "готовность рабочего пространства, проверки и диагностики",
    ));
    result
}

fn inspect_infobase_target(
    workspace_root: &std::path::Path,
    config_present: bool,
) -> Result<InfobaseTarget, String> {
    if !config_present {
        return Ok(InfobaseTarget {
            configured: false,
            source: None,
            legacy_connection: Vec::new(),
        });
    }
    let base = read_yaml_config(workspace_root, "v8project.yaml")?
        .ok_or_else(|| "v8project.yaml disappeared during inspection".to_string())?;
    let base_connection = yaml_infobase_connection(&base, "v8project.yaml")?;
    let local = read_yaml_config(workspace_root, "v8project.local.yaml")?;
    let local_connection = local
        .as_ref()
        .map(|value| yaml_infobase_connection(value, "v8project.local.yaml"))
        .transpose()?
        .flatten();
    let (connection, source) = match local_connection {
        Some(connection) => (Some(connection), Some("v8project.local.yaml")),
        None => (base_connection, Some("v8project.yaml")),
    };
    let configured = connection
        .as_deref()
        .is_some_and(|connection| !connection.trim().is_empty());
    let has_top_level_connection = |layer: &serde_yaml::Value| {
        layer
            .as_mapping()
            .is_some_and(|mapping| mapping.contains_key(serde_yaml::Value::from("connection")))
    };
    let mut legacy_connection = Vec::new();
    if has_top_level_connection(&base) {
        legacy_connection.push("v8project.yaml");
    }
    if local.as_ref().is_some_and(has_top_level_connection) {
        legacy_connection.push("v8project.local.yaml");
    }
    // The runner refuses the whole configuration, so no infobase is usable
    // even when `infobases.origin` is declared beside the former key.
    let configured = configured && legacy_connection.is_empty();
    Ok(InfobaseTarget {
        configured,
        source: configured.then_some(source.expect("configured connection has a source")),
        legacy_connection,
    })
}

/// Whether either layer names the platform installation: `tools.platform`
/// with a version or a path. An unreadable layer is reported elsewhere and
/// does not add this warning.
fn platform_pinned(workspace_root: &std::path::Path) -> bool {
    ["v8project.yaml", "v8project.local.yaml"]
        .iter()
        .any(|file| match read_yaml_config(workspace_root, file) {
            Ok(Some(layer)) => ["version", "path"]
                .iter()
                .any(|key| !layer["tools"]["platform"][*key].is_null()),
            Ok(None) => false,
            Err(_) => true,
        })
}

/// The recommended machine-local layer next to a project-file recipe, when
/// the workspace has none: the origin infobase without a password and the
/// platform line the export format is written for.
fn with_local_recipe(mut setup: Value, workspace_root: &std::path::Path) -> Value {
    if workspace_root.join("v8project.local.yaml").exists() {
        return setup;
    }
    let line = unica_source_contract::ACTIVE_PLATFORM_LINE;
    if let Value::Object(fields) = &mut setup {
        fields.insert(
            "local".to_string(),
            object([
                ("path", Value::String("v8project.local.yaml".to_string())),
                (
                    "content",
                    Value::String(format!(
                        "infobases:\n  origin:\n    connection: 'File=build/ib'\ntools:\n  platform:\n    version: '{line}'\n"
                    )),
                ),
                (
                    "reason",
                    Value::String(format!(
                        "Create this machine-local layer beside v8project.yaml: it names the origin infobase, created later by infobase.create, and pins the {line} platform line the export format is written for; without tools.platform the runner chooses the platform version itself. If the infobase has a user, add `user` and write its `password` here yourself."
                    )),
                ),
            ]),
        );
    }
    setup
}

/// The warning that no layer names the platform installation.
fn platform_unpinned_finding() -> Value {
    let line = unica_source_contract::ACTIVE_PLATFORM_LINE;
    object([
        ("code", Value::String("config.platform_unpinned".to_string())),
        ("severity", Value::String("warning".to_string())),
        ("scope", Value::String("config".to_string())),
        (
            "message",
            Value::String(format!(
                "the platform version is not set: neither v8project.yaml nor v8project.local.yaml has tools.platform, so v8-runner chooses the version itself. Set tools.platform.version in v8project.local.yaml; the export format is written for the {line} line"
            )),
        ),
        ("evidence", Value::Array(Vec::new())),
        (
            "remediation",
            object([
                (
                    "summary",
                    Value::String(format!("Pin the {line} platform line in v8project.local.yaml")),
                ),
                (
                    "steps",
                    Value::Array(vec![
                        Value::String(format!(
                            "Add `tools: {{platform: {{version: '{line}'}}}}` to v8project.local.yaml"
                        )),
                        Value::String("Run unica.check with an empty object again".to_string()),
                    ]),
                ),
                ("commands", Value::Array(Vec::new())),
            ]),
        ),
    ])
}

/// The finding about a top-level `connection`: the key, the layers that hold
/// it and where v8-runner 0.15 reads the infobase address instead.
fn legacy_connection_finding(files: &[&str]) -> Value {
    let named = files.join(" and ");
    object([
        (
            "code",
            Value::String("config.legacy_top_level_connection".to_string()),
        ),
        ("severity", Value::String("error".to_string())),
        ("scope", Value::String("config".to_string())),
        (
            "paths",
            Value::Array(files.iter().map(|file| Value::String(file.to_string())).collect()),
        ),
        ("count", Value::Number(files.len().into())),
        ("key", Value::String("connection".to_string())),
        (
            "moveTo",
            object([
                ("file", Value::String("v8project.local.yaml".to_string())),
                ("key", Value::String("infobases.origin.connection".to_string())),
            ]),
        ),
        (
            "message",
            Value::String(format!(
                "{named} names the infobase by a top-level `connection`, the former v8-runner format; v8-runner 0.15 refuses the configuration and the infobase is not configured. Move the value to `infobases.origin.connection` in v8project.local.yaml"
            )),
        ),
        ("evidence", Value::Array(Vec::new())),
        (
            "remediation",
            object([
                (
                    "summary",
                    Value::String(
                        "Declare the infobase as infobases.origin in the machine-local layer"
                            .to_string(),
                    ),
                ),
                (
                    "steps",
                    Value::Array(vec![
                        Value::String(format!("Remove the top-level `connection` key from {named}")),
                        Value::String(
                            "Write it to v8project.local.yaml as `infobases: {origin: {connection: '<value>'}}`, or run `v8-runner init --infobase '<value>'`"
                                .to_string(),
                        ),
                        Value::String("Run unica.view with an empty object again".to_string()),
                    ]),
                ),
                ("commands", Value::Array(Vec::new())),
            ]),
        ),
    ])
}

pub(super) fn read_yaml_config(
    workspace_root: &std::path::Path,
    name: &str,
) -> Result<Option<serde_yaml::Value>, String> {
    let workspace_root = normalize_path_identity(workspace_root).map_err(|error| {
        format!(
            "failed to resolve workspace root {}: {error}",
            workspace_root.display()
        )
    })?;
    let path = workspace_root.join(name);
    let read = match read_root_relative_regular_file(
        &workspace_root,
        &path,
        PROJECT_CONFIG_MAX_BYTES,
        |_| {},
    ) {
        Ok(read) => read,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{name} must be a bounded regular file: {error}")),
    };
    serde_yaml::from_slice(&read.bytes)
        .map(Some)
        .map_err(|error| format!("failed to parse {name}: {error}"))
}

fn yaml_infobase_connection(
    root: &serde_yaml::Value,
    source: &str,
) -> Result<Option<String>, String> {
    let Some(mapping) = root.as_mapping() else {
        return Err(format!("{source} document root must be a mapping"));
    };
    let Some(infobase) = mapping
        .get(serde_yaml::Value::from("infobases"))
        .and_then(|v| v.get("origin"))
        .or_else(|| mapping.get(serde_yaml::Value::from("infobase")))
    else {
        return Ok(None);
    };
    let Some(infobase) = infobase.as_mapping() else {
        return Err(format!("{source} infobase must be a mapping"));
    };
    let Some(connection) = infobase.get(serde_yaml::Value::String("connection".to_string())) else {
        return Ok(None);
    };
    connection
        .as_str()
        .map(|connection| Some(connection.to_string()))
        .ok_or_else(|| format!("{source} infobase.connection must be text"))
}

/// Names where `workspaceRoot` came from next to it: the host channel that
/// chose the requested directory, that directory (the root may be one of its
/// parents), and for a channel captured at start a way to another project.
pub(super) fn annotate_workspace_origin(
    result: &mut DomainResult,
    requested_directory: &str,
    origin: Option<&unica_bootstrap::WorkspaceOrigin>,
) {
    let Some(origin) = origin else {
        return;
    };
    let Some(data) = result.data.as_mut().and_then(Value::as_object_mut) else {
        return;
    };
    if !data.contains_key("workspaceRoot") {
        return;
    }
    let Ok(Value::Object(mut described)) = serde_json::to_value(origin) else {
        return;
    };
    described.insert(
        "requestedDirectory".to_string(),
        Value::String(requested_directory.to_string()),
    );
    if let Some(hint) = workspace_origin_hint(origin) {
        described.insert("hint".to_string(), Value::String(hint));
    }
    data.insert("workspaceRootOrigin".to_string(), Value::Object(described));
}

fn workspace_origin_hint(origin: &unica_bootstrap::WorkspaceOrigin) -> Option<String> {
    use unica_bootstrap::{RootsFallback, WorkspaceOrigin};
    if !origin.can_be_stale() {
        return None;
    }
    let (taken_from, roots) = match origin {
        WorkspaceOrigin::StartupEnvironment { variable, roots } => (
            format!("the project variable {variable} captured when the unica MCP server started"),
            roots,
        ),
        WorkspaceOrigin::LaunchCwd { roots } => (
            "the directory the unica MCP server was started in".to_string(),
            roots,
        ),
        WorkspaceOrigin::RequestMetadata { .. } | WorkspaceOrigin::ClientRoots {} => return None,
    };
    let roots = match roots {
        None => String::new(),
        Some(RootsFallback::Empty) => " The client declared MCP roots but listed none.".to_string(),
        Some(RootsFallback::Error) => {
            " The client declared MCP roots, but roots/list returned an error.".to_string()
        }
        Some(RootsFallback::Timeout) => {
            " The client declared MCP roots, but roots/list did not answer in time.".to_string()
        }
        Some(RootsFallback::Closed) => {
            " The client declared MCP roots, but the connection closed during roots/list."
                .to_string()
        }
    };
    Some(format!(
        "The directory comes from {taken_from}; it does not follow a session that later changed its project.{roots} To work in another project, reconnect the unica MCP server from it or open a new session there."
    ))
}

fn object<const N: usize>(entries: [(&str, Value); N]) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

fn value<T: Serialize>(value: T) -> Value {
    serde_json::to_value(value).expect("workspace bootstrap value serializes")
}

/// Лежит ли `v8project.yaml` в корне.
///
/// Существующий, но нечитаемый файл — тоже «лежит»: иначе битую настройку
/// объявили бы отсутствующей и предложили завести пространство заново.
/// Предикат один на корень и на допуск: разойдясь, они назвали бы одному
/// каталогу две разные причины.
pub(super) fn project_config_present(workspace_root: &std::path::Path) -> bool {
    match std::fs::symlink_metadata(workspace_root.join("v8project.yaml")) {
        Ok(_) => true,
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

pub(super) fn next_action(tool: &str, args: Value, reason: &str) -> Value {
    object([
        ("tool", Value::String(tool.to_string())),
        ("args", args),
        ("reason", Value::String(reason.to_string())),
    ])
}

pub(super) fn project_config_recipe(source_map: &ProjectSourceMap) -> Option<String> {
    #[derive(Serialize)]
    struct ProjectRecipe<'a> {
        format: &'static str,
        #[serde(rename = "source-set")]
        source_sets: Vec<SourceSetRecipe<'a>>,
    }

    #[derive(Serialize)]
    struct SourceSetRecipe<'a> {
        name: &'a str,
        #[serde(rename = "type")]
        source_type: &'static str,
        path: &'a str,
    }

    let mut sources = source_map.source_sets.iter().collect::<Vec<_>>();
    sources.sort_by(|left, right| left.name.cmp(&right.name));
    let uniform_format = sources
        .first()
        .map(|source| source.source_format)
        .filter(|first| sources.iter().all(|source| source.source_format == *first));
    let format = match uniform_format {
        Some(SourceFormat::PlatformXml) => "DESIGNER",
        Some(SourceFormat::Edt) => "EDT",
        Some(SourceFormat::Unknown | SourceFormat::Invalid) => return None,
        None if sources.is_empty()
            && source_map
                .configured_format_raw
                .as_deref()
                .is_some_and(|format| format.eq_ignore_ascii_case("EDT")) =>
        {
            "EDT"
        }
        None if sources.is_empty() => "DESIGNER",
        None => return None,
    };
    let source_sets = if sources.is_empty() {
        vec![SourceSetRecipe {
            name: "main",
            source_type: "CONFIGURATION",
            path: "src",
        }]
    } else {
        sources
            .into_iter()
            .map(|source| SourceSetRecipe {
                name: &source.name,
                source_type: match source.kind {
                    SourceSetKind::Configuration => "CONFIGURATION",
                    SourceSetKind::Extension => "EXTENSION",
                    SourceSetKind::ExternalProcessor => "EXTERNAL_DATA_PROCESSORS",
                    SourceSetKind::ExternalReport => "EXTERNAL_REPORTS",
                },
                path: &source.path,
            })
            .collect()
    };
    Some(
        serde_yaml::to_string(&ProjectRecipe {
            format,
            source_sets,
        })
        .expect("workspace setup recipe serializes"),
    )
}

#[cfg(test)]
mod tests {
    use super::{inspect_infobase_target, project_config_recipe};
    use unica_source_contract::{ProjectSourceMap, ProjectSourceSet, SourceFormat, SourceSetKind};

    fn eol_check_fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let workspace = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(workspace.path()).unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src/Configuration.xml"), "<MetaDataObject/>\n").unwrap();
        std::fs::write(root.join("src/A.xml"), "<A/>\n").unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            root.join(".gitignore"),
            "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
        )
        .unwrap();
        std::fs::write(root.join(".gitattributes"), "*.xml text eol=lf\n").unwrap();
        for args in [["init"].as_slice(), ["add", "."].as_slice()] {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(&root)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        (workspace, root)
    }

    /// A store over daemon state outside the workspace, as the daemon keeps it.
    fn continuation_store() -> (tempfile::TempDir, super::RootCheckContinuationStore) {
        let state = tempfile::tempdir().unwrap();
        std::fs::create_dir(state.path().join("project-health")).unwrap();
        let store = super::RootCheckContinuationStore::in_directory(
            state.path().join("project-health"),
            std::time::Duration::from_secs(3600),
        );
        (state, store)
    }

    /// Stage `paths` with one shared blob and no working files, the way a
    /// large repository looks to the index without writing every file.
    fn stage_synthetic_paths(root: &std::path::Path, paths: impl Iterator<Item = String>) {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let blob = Command::new("git")
            .args(["hash-object", "-w", "--stdin"])
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        blob.stdin.as_ref().unwrap().write_all(b"x\n").unwrap();
        let blob = blob.wait_with_output().unwrap();
        assert!(blob.status.success(), "git hash-object failed");
        let oid = String::from_utf8(blob.stdout).unwrap().trim().to_owned();
        let mut input = Vec::new();
        for path in paths {
            input.extend_from_slice(format!("100644 {oid}\t{path}\0").as_bytes());
        }
        let mut update = Command::new("git")
            .args(["update-index", "-z", "--index-info"])
            .current_dir(root)
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        update.stdin.take().unwrap().write_all(&input).unwrap();
        assert!(update.wait().unwrap().success(), "git update-index failed");
    }

    fn root_check(root: &std::path::Path) -> serde_json::Value {
        use super::{prepare, Preparation, RootCheckContinuationStore};
        use crate::infrastructure::daemon::protocol::InvocationRequest;
        use std::sync::Arc;
        use unica_execution::clock::TokioClock;
        use unica_execution::invocation::InvocationResponseDeadline;
        use unica_execution::invocation_store::ToolIdentity;
        use unica_work_control::cancellation::CancellationToken;

        let request = InvocationRequest::new(
            ToolIdentity::Check,
            serde_json::json!({}),
            root.to_string_lossy(),
            7_000,
        )
        .unwrap();
        let Preparation::Ready(inspection) = prepare(
            &request,
            InvocationResponseDeadline::capture(Arc::new(TokioClock)),
            &RootCheckContinuationStore::default(),
            &Default::default(),
        ) else {
            panic!("root check must prepare");
        };
        inspection
            .execute(CancellationToken::new())
            .unwrap()
            .data
            .unwrap()
    }

    fn assert_repository_checks_passed(data: &serde_json::Value) {
        assert_eq!(data["readinessState"], "complete", "{data}");
        assert_eq!(data["repositoryReady"], true, "{data}");
        for check in data["checks"].as_array().unwrap() {
            assert_eq!(check["status"], "passed", "{check}");
        }
    }

    #[test]
    fn root_check_inspects_resources_past_the_former_classification_count() {
        // 65 537 staged binary resources pass the former 65 536-entry refusal.
        // Their working files are absent, which the LFS advice reads as no size.
        let (_workspace, root) = eol_check_fixture();
        std::fs::write(
            root.join(".gitattributes"),
            "*.xml text eol=lf\n*.bin -text\n",
        )
        .unwrap();
        let output = std::process::Command::new("git")
            .args(["add", ".gitattributes"])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(output.status.success());
        stage_synthetic_paths(
            &root,
            (0..65_537).map(|number| format!("src/Blobs/{number:05}.bin")),
        );

        assert_repository_checks_passed(&root_check(&root));
    }

    #[test]
    fn root_check_streams_an_index_past_the_former_capture_limit() {
        // 200 000 long staged paths make `git ls-files --stage -z` print more
        // than the former 64 MiB capture of a Git command's output.
        let (_workspace, root) = eol_check_fixture();
        let directory = "d".repeat(300);
        stage_synthetic_paths(
            &root,
            (0..200_000).map(|number| format!("other/{directory}/{number:06}.dat")),
        );
        let index = std::process::Command::new("git")
            .args(["ls-files", "--stage", "-z"])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(index.stdout.len() > 64 * 1024 * 1024);
        drop(index);

        assert_repository_checks_passed(&root_check(&root));
    }

    #[test]
    fn root_check_keeps_its_continuation_outside_the_workspace_and_a_new_daemon_continues() {
        use super::{prepare, Preparation, RootCheckContinuationStore};
        use crate::infrastructure::daemon::protocol::InvocationRequest;
        use std::sync::Arc;
        use unica_execution::clock::TokioClock;
        use unica_execution::invocation::InvocationResponseDeadline;
        use unica_execution::invocation_store::ToolIdentity;
        use unica_project_health::inspection::resources::stop_staged_eol_after_for_test;
        use unica_work_control::cancellation::CancellationToken;

        let (_workspace, root) = eol_check_fixture();
        let state = tempfile::tempdir().unwrap();
        let directory = state.path().join("project-health");
        std::fs::create_dir(&directory).unwrap();
        let request = InvocationRequest::new(
            ToolIdentity::Check,
            serde_json::json!({}),
            root.to_string_lossy(),
            7_000,
        )
        .unwrap();
        let check = |stop_after: Option<usize>| {
            // Every call opens its own store over the same state, as a
            // restarted daemon would.
            let store = RootCheckContinuationStore::in_directory(
                directory.clone(),
                std::time::Duration::from_secs(3600),
            );
            if let Some(stop_after) = stop_after {
                stop_staged_eol_after_for_test(stop_after);
            }
            let Preparation::Ready(inspection) = prepare(
                &request,
                InvocationResponseDeadline::capture(Arc::new(TokioClock)),
                &store,
                &Default::default(),
            ) else {
                panic!("root check must prepare");
            };
            inspection
                .execute(CancellationToken::new())
                .unwrap()
                .data
                .unwrap()
        };
        let workspace_before = unica_workspace::test_support::tree_snapshot(&root);

        let first = check(Some(1));
        assert_eq!(first["readinessState"], "incomplete");
        assert_eq!(first["inspectionProgress"]["stagedEolRetained"], 1);
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        // The next call proves the path the first one proved without asking
        // Git again; starting over would retain nothing before its stop.
        let second = check(Some(0));
        assert_eq!(second["readinessState"], "incomplete");
        assert_eq!(second["inspectionProgress"]["stagedEolRetained"], 1);
        let third = check(None);
        assert_eq!(third["readinessState"], "complete");
        assert_eq!(third["repositoryReady"], true);

        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&root),
            workspace_before,
            "a root check leaves the workspace unchanged"
        );
    }

    #[test]
    fn root_check_repeats_after_eol_timeout_and_finishes_with_shared_checkpoint() {
        use super::{prepare, Preparation};
        use crate::infrastructure::daemon::protocol::InvocationRequest;
        use std::sync::Arc;
        use unica_execution::clock::TokioClock;
        use unica_execution::invocation::InvocationResponseDeadline;
        use unica_execution::invocation_store::ToolIdentity;
        use unica_project_health::inspection::resources::stop_staged_eol_after_for_test;
        use unica_work_control::cancellation::CancellationToken;

        let (_workspace, root) = eol_check_fixture();
        let request = InvocationRequest::new(
            ToolIdentity::Check,
            serde_json::json!({}),
            root.to_string_lossy(),
            7_000,
        )
        .unwrap();
        let (_state, continuations) = continuation_store();
        stop_staged_eol_after_for_test(1);
        let Preparation::Ready(first) = prepare(
            &request,
            InvocationResponseDeadline::capture(Arc::new(TokioClock)),
            &continuations,
            &Default::default(),
        ) else {
            panic!("first root check must prepare");
        };
        let first = first.execute(CancellationToken::new()).unwrap();
        // A step stopped by the test hook leaves the inspection incomplete;
        // without automatic deadlines the answer no longer invites a repeat
        // with a fresh deadline (#1251), yet a repeat still resumes.
        assert!(!first
            .next
            .iter()
            .any(|action| action["tool"] == "unica.check"));
        let first_data = first.data.unwrap();
        assert_eq!(first_data["readinessState"], "incomplete");
        assert_eq!(first_data["inspectionProgress"]["stagedEolRetained"], 1);

        let Preparation::Ready(second) = prepare(
            &request,
            InvocationResponseDeadline::capture(Arc::new(TokioClock)),
            &continuations,
            &Default::default(),
        ) else {
            panic!("repeated root check must prepare");
        };
        let second = second.execute(CancellationToken::new()).unwrap();
        assert_eq!(second.data.as_ref().unwrap()["readinessState"], "complete");
        assert_eq!(second.data.as_ref().unwrap()["repositoryReady"], true);
    }

    #[test]
    fn root_eol_timeout_keeps_earlier_attribute_failure() {
        use super::{prepare, Preparation};
        use crate::infrastructure::daemon::protocol::InvocationRequest;
        use std::sync::Arc;
        use unica_execution::clock::TokioClock;
        use unica_execution::invocation::InvocationResponseDeadline;
        use unica_execution::invocation_store::ToolIdentity;
        use unica_project_health::inspection::resources::stop_staged_eol_after_for_test;
        use unica_work_control::cancellation::CancellationToken;

        let (_workspace, root) = eol_check_fixture();
        std::fs::write(
            root.join(".gitattributes"),
            "src/Configuration.xml text eol=lf\n",
        )
        .unwrap();
        let output = std::process::Command::new("git")
            .args(["add", ".gitattributes"])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(output.status.success());
        let request = InvocationRequest::new(
            ToolIdentity::Check,
            serde_json::json!({}),
            root.to_string_lossy(),
            7_000,
        )
        .unwrap();
        stop_staged_eol_after_for_test(0);
        let (_state, continuations) = continuation_store();
        let Preparation::Ready(inspection) = prepare(
            &request,
            InvocationResponseDeadline::capture(Arc::new(TokioClock)),
            &continuations,
            &Default::default(),
        ) else {
            panic!("root check must prepare");
        };
        let result = inspection.execute(CancellationToken::new()).unwrap();
        let data = result.data.unwrap();
        assert_eq!(data["readinessState"], "incomplete");
        assert!(data["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| { diagnostic["code"] == "git.text_policy_missing" }));
        assert!(data["checks"].as_array().unwrap().iter().any(|check| {
            check["id"] == "repository.attributes" && check["status"] == "failed"
        }));
        assert!(data["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| { check["id"] == "repository.index_eol" && check["status"] == "notRun" }));
    }

    #[test]
    fn root_check_resumes_inside_working_file_without_losing_attribute_failure() {
        use super::{prepare, Preparation};
        use crate::infrastructure::daemon::protocol::InvocationRequest;
        use std::sync::Arc;
        use unica_execution::clock::TokioClock;
        use unica_execution::invocation::InvocationResponseDeadline;
        use unica_execution::invocation_store::ToolIdentity;
        use unica_project_health::inspection::resources::stop_working_eol_after_block_for_test;
        use unica_work_control::cancellation::CancellationToken;

        let (_workspace, root) = eol_check_fixture();
        std::fs::write(
            root.join(".gitattributes"),
            "src/Configuration.xml text eol=lf\n",
        )
        .unwrap();
        let output = std::process::Command::new("git")
            .args(["add", ".gitattributes"])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(output.status.success());
        // The index keeps its small blob. Only the working file spans three
        // read blocks, so this exercises working-file continuation alone.
        let mut bytes = b"<A>".to_vec();
        bytes.resize(2 * 64 * 1024, b'x');
        bytes.extend_from_slice(b"</A>\n");
        let working_bytes = bytes.len() as u64;
        std::fs::write(root.join("src/A.xml"), bytes).unwrap();
        let request = InvocationRequest::new(
            ToolIdentity::Check,
            serde_json::json!({}),
            root.to_string_lossy(),
            7_000,
        )
        .unwrap();
        let normalized_root = super::normalize_path_identity(&root).unwrap();
        let (_state, continuations) = continuation_store();
        for step in 0..4 {
            if step < 3 {
                stop_working_eol_after_block_for_test();
            }
            let Preparation::Ready(inspection) = prepare(
                &request,
                InvocationResponseDeadline::capture(Arc::new(TokioClock)),
                &continuations,
                &Default::default(),
            ) else {
                panic!("root check must prepare");
            };
            let result = inspection.execute(CancellationToken::new()).unwrap();
            let repeat = result
                .next
                .iter()
                .any(|action| action["tool"] == "unica.check");
            let data = result.data.unwrap();
            assert!(data["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|diagnostic| { diagnostic["code"] == "git.text_policy_missing" }));
            assert_eq!(data["repositoryReady"], false);
            assert!(
                !repeat,
                "no repeat with a fresh deadline is offered after #1251"
            );
            if step < 3 {
                assert_eq!(data["readinessState"], "incomplete");
                let state = continuations.for_workspace(&normalized_root).unwrap();
                assert_eq!(
                    state
                        .lock()
                        .unwrap()
                        .working_eol_offset_for_test("src/A.xml"),
                    Some(((step + 1) * 64 * 1024).min(working_bytes)),
                    "root check must retain and advance the working-file offset",
                );
            } else {
                assert_eq!(data["readinessState"], "complete");
            }
        }
    }

    #[test]
    fn root_check_does_not_recommend_repeat_for_a_fixed_failure() {
        use super::{prepare, Preparation, RootCheckContinuationStore};
        use crate::infrastructure::daemon::protocol::InvocationRequest;
        use std::sync::Arc;
        use unica_execution::clock::TokioClock;
        use unica_execution::invocation::InvocationResponseDeadline;
        use unica_execution::invocation_store::ToolIdentity;
        use unica_work_control::cancellation::CancellationToken;

        let (_workspace, root) = eol_check_fixture();
        let request = InvocationRequest::new(
            ToolIdentity::Check,
            serde_json::json!({}),
            root.to_string_lossy(),
            7_000,
        )
        .unwrap();
        // A malformed Git index is static until the repository is repaired.
        std::fs::write(root.join(".git/index"), b"bad index").unwrap();
        let Preparation::Ready(inspection) = prepare(
            &request,
            InvocationResponseDeadline::capture(Arc::new(TokioClock)),
            &RootCheckContinuationStore::default(),
            &Default::default(),
        ) else {
            panic!("root check must prepare even for a broken index");
        };
        let result = inspection.execute(CancellationToken::new()).unwrap();
        assert_eq!(
            result.data.as_ref().unwrap()["readinessState"],
            "incomplete"
        );
        assert!(!result
            .next
            .iter()
            .any(|action| action["tool"] == "unica.check"));
    }

    #[test]
    fn root_inspection_discovers_sources_after_response_handoff() {
        use super::{prepare, Preparation, RootCheckContinuationStore};
        use crate::infrastructure::daemon::protocol::InvocationRequest;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;
        use std::time::{Duration, Instant};
        use unica_execution::invocation::InvocationResponseDeadline;
        use unica_execution::invocation_store::ToolIdentity;
        use unica_work_control::cancellation::CancellationToken;
        use unica_work_control::clock::Clock;

        struct ManualClock {
            start: Instant,
            elapsed_ms: AtomicU64,
        }

        impl Clock for ManualClock {
            fn now(&self) -> Instant {
                self.start + Duration::from_millis(self.elapsed_ms.load(Ordering::SeqCst))
            }
        }

        for configured in [true, false] {
            for tool in [ToolIdentity::View, ToolIdentity::Check] {
                let workspace = tempfile::tempdir().unwrap();
                let root = std::fs::canonicalize(workspace.path()).unwrap();
                std::fs::create_dir(root.join("src")).unwrap();
                std::fs::write(root.join("src/Configuration.xml"), "<MetaDataObject/>").unwrap();
                if configured {
                    std::fs::write(
                        root.join("v8project.yaml"),
                        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
                    )
                    .unwrap();
                }
                let clock = Arc::new(ManualClock {
                    start: Instant::now(),
                    elapsed_ms: AtomicU64::new(0),
                });
                let request = InvocationRequest::new(
                    tool,
                    serde_json::json!({}),
                    root.to_string_lossy(),
                    7_000,
                )
                .unwrap();
                let continuations = RootCheckContinuationStore::default();
                let Preparation::Ready(inspection) = prepare(
                    &request,
                    InvocationResponseDeadline::capture(clock.clone()),
                    &continuations,
                    &Default::default(),
                ) else {
                    panic!("root inspection must prepare before response handoff");
                };

                clock.elapsed_ms.store(9_000, Ordering::SeqCst);
                let result = inspection.execute(CancellationToken::new()).unwrap();
                assert!(result.ok, "{tool:?}, configured={configured}: {result:?}");
                if tool == ToolIdentity::Check {
                    assert!(!result
                        .next
                        .iter()
                        .any(|action| action["tool"] == "unica.check"));
                }
                let data = result.data.unwrap();
                if tool == ToolIdentity::Check {
                    // Past the handoff moment the inspection still runs to
                    // its end: no handoff-sized portion (#1251).
                    assert_eq!(data["readinessState"], "complete", "{data}");
                } else {
                    assert_eq!(data["sourceSets"][0]["name"], "main");
                    assert_eq!(
                        data["config"]["state"],
                        if configured {
                            "configured"
                        } else {
                            "autodetected"
                        }
                    );
                }
            }
        }
    }

    #[test]
    fn root_inspection_cancellation_prevents_discovery_and_late_publication() {
        use super::{prepare, Preparation, RootCheckContinuationStore};
        use crate::infrastructure::daemon::protocol::InvocationRequest;
        use std::sync::Arc;
        use unica_execution::clock::TokioClock;
        use unica_execution::invocation::InvocationResponseDeadline;
        use unica_execution::invocation_store::ToolIdentity;
        use unica_work_control::cancellation::CancellationToken;

        let workspace = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(workspace.path()).unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(root.join("src/Configuration.xml"), "<MetaDataObject/>").unwrap();
        let request = InvocationRequest::new(
            ToolIdentity::Check,
            serde_json::json!({}),
            root.to_string_lossy(),
            7_000,
        )
        .unwrap();
        let Preparation::Ready(inspection) = prepare(
            &request,
            InvocationResponseDeadline::capture(Arc::new(TokioClock)),
            &RootCheckContinuationStore::default(),
            &Default::default(),
        ) else {
            panic!("root inspection must prepare before source admission");
        };
        let pause = super::test_control::RootInspectionPause::install(root);
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert_eq!(inspection.execute(cancelled).unwrap_err().code, "cancelled");
        assert_eq!(pause.entries(), 0);

        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let worker = std::thread::spawn(move || inspection.execute(worker_cancellation));
        pause.wait_until_entered();
        cancellation.cancel();
        pause.release();
        assert_eq!(worker.join().unwrap().unwrap_err().code, "cancelled");
        assert_eq!(pause.entries(), 1);
    }

    #[test]
    fn project_config_recipe_quotes_yaml_significant_source_identity() {
        let source_map = ProjectSourceMap {
            workspace_root: "/workspace".to_string(),
            config_path: None,
            source_sets: vec![ProjectSourceSet {
                name: "main: # one\ncontinued".to_string(),
                kind: SourceSetKind::Configuration,
                path: "# source: one".to_string(),
                source_format: SourceFormat::PlatformXml,
                source_state: unica_source_contract::SourceSetState::Supported,
                format_evidence: Vec::new(),
                format_probe_error: None,
            }],
            effective_source_set: None,
            effective_source_root: None,
            source_selection_error: None,
            configured_format_raw: None,
        };

        let recipe = project_config_recipe(&source_map).unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&recipe).unwrap();

        assert_eq!(parsed["source-set"][0]["name"], "main: # one\ncontinued");
        assert_eq!(parsed["source-set"][0]["path"], "# source: one");
    }

    #[test]
    fn project_config_recipe_preserves_an_all_edt_discovery_default() {
        let source_map = ProjectSourceMap {
            workspace_root: "/workspace".to_string(),
            config_path: None,
            source_sets: vec![ProjectSourceSet {
                name: "main".to_string(),
                kind: SourceSetKind::Configuration,
                path: "src".to_string(),
                source_format: SourceFormat::Edt,
                source_state: unica_source_contract::SourceSetState::Unsupported,
                format_evidence: Vec::new(),
                format_probe_error: None,
            }],
            effective_source_set: Some("main".to_string()),
            effective_source_root: Some("src".to_string()),
            source_selection_error: None,
            configured_format_raw: None,
        };

        let parsed: serde_yaml::Value =
            serde_yaml::from_str(&project_config_recipe(&source_map).unwrap()).unwrap();

        assert_eq!(parsed["format"], "EDT");
    }

    #[test]
    fn infobase_target_uses_the_machine_local_connection_override() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\n",
        )
        .unwrap();
        std::fs::write(
            workspace.path().join("v8project.local.yaml"),
            "infobase:\n  connection: 'Srvr=server;Ref=base'\n",
        )
        .unwrap();

        let target = inspect_infobase_target(workspace.path(), true).unwrap();

        assert!(target.configured);
        assert_eq!(target.source, Some("v8project.local.yaml"));
    }

    /// #2155: a top-level `connection`, the former runner format, configures
    /// nothing, and both root answers name the key and where it belongs.
    #[test]
    fn a_top_level_connection_is_named_with_the_place_it_belongs() {
        use super::{prepare, Preparation};
        use crate::infrastructure::daemon::protocol::InvocationRequest;
        use std::sync::Arc;
        use unica_execution::clock::TokioClock;
        use unica_execution::invocation::InvocationResponseDeadline;
        use unica_execution::invocation_store::ToolIdentity;
        use unica_work_control::cancellation::CancellationToken;

        for (layers, base, local) in [
            (
                vec!["v8project.yaml"],
                "format: DESIGNER\nconnection: 'File=build/ib'\n",
                None,
            ),
            (
                vec!["v8project.local.yaml"],
                "format: DESIGNER\n",
                Some("connection: 'Srvr=server;Ref=base'\n"),
            ),
            // The runner refuses the configuration even beside a declared
            // origin, so the origin does not make the infobase usable.
            (
                vec!["v8project.yaml", "v8project.local.yaml"],
                "format: DESIGNER\nconnection: 'File=build/ib'\n",
                Some("connection: 'File=other'\ninfobases:\n  origin:\n    connection: 'File=build/ib'\n"),
            ),
        ] {
            let workspace = tempfile::tempdir().unwrap();
            std::fs::write(workspace.path().join("v8project.yaml"), base).unwrap();
            if let Some(local) = local {
                std::fs::write(workspace.path().join("v8project.local.yaml"), local).unwrap();
            }
            let target = inspect_infobase_target(workspace.path(), true).unwrap();
            assert!(!target.configured, "{layers:?}");
            assert_eq!(target.legacy_connection, layers);

            let (_state, continuations) = continuation_store();
            let root = std::fs::canonicalize(workspace.path()).unwrap();
            let answer = |tool| {
                let request = InvocationRequest::new(
                    tool,
                    serde_json::json!({}),
                    root.to_string_lossy(),
                    7_000,
                )
                .unwrap();
                let Preparation::Ready(inspection) = prepare(
                    &request,
                    InvocationResponseDeadline::capture(Arc::new(TokioClock)),
                    &continuations,
                    &Default::default(),
                ) else {
                    panic!("root inspection must prepare");
                };
                inspection.execute(CancellationToken::new()).unwrap()
            };
            let assert_finding = |finding: &serde_json::Value| {
                assert_eq!(finding["key"], "connection", "{finding}");
                assert_eq!(finding["paths"], serde_json::json!(layers), "{finding}");
                assert_eq!(
                    finding["moveTo"],
                    serde_json::json!({"file": "v8project.local.yaml", "key": "infobases.origin.connection"})
                );
                let message = finding["message"].as_str().unwrap();
                assert!(
                    message.contains("infobases.origin.connection")
                        && message.contains("v8project.local.yaml"),
                    "{message}"
                );
            };

            let facts = answer(ToolIdentity::View);
            let data = facts.data.as_ref().unwrap();
            assert_eq!(data["infobase"]["configured"], false, "{data}");
            assert!(data["setup"].is_null(), "no competing setup advice: {data}");
            assert!(
                facts.next.iter().all(|next| next["tool"] != "unica.run"),
                "no runner preview against a refused configuration: {:?}",
                facts.next
            );
            let warning = facts
                .warnings
                .iter()
                .find(|warning| warning["code"] == "config.legacy_top_level_connection")
                .unwrap_or_else(|| panic!("view names the key: {facts:?}"));
            assert_finding(warning);

            let verdict = answer(ToolIdentity::Check);
            let data = verdict.data.as_ref().unwrap();
            assert_eq!(data["ready"], false, "{data}");
            let diagnostic = data["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .find(|diagnostic| diagnostic["code"] == "config.legacy_top_level_connection")
                .unwrap_or_else(|| panic!("check names the key: {data}"));
            assert_finding(diagnostic);
        }
    }

    #[test]
    fn empty_local_connection_does_not_claim_runtime_readiness() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "infobase:\n  connection: 'File=base'\n",
        )
        .unwrap();
        std::fs::write(
            workspace.path().join("v8project.local.yaml"),
            "infobase:\n  connection: ''\n",
        )
        .unwrap();

        let target = inspect_infobase_target(workspace.path(), true).unwrap();

        assert!(!target.configured);
        assert_eq!(target.source, None);
    }

    /// Источник ставится рядом с корнем, где бы корень ни был назван, —
    /// в фактах `view {}` и в отказах допуска, — и не появляется там, где
    /// корня нет: без корня объяснять нечего.
    #[test]
    fn workspace_origin_is_named_next_to_every_workspace_root() {
        use serde_json::Value;
        use unica_bootstrap::{RootsFallback, WorkspaceOrigin};
        use unica_execution_contract::invocation::DomainResult;
        let annotated = |data: Value, origin: WorkspaceOrigin| {
            let mut result = DomainResult::success("facts");
            result.data = Some(data);
            super::annotate_workspace_origin(&mut result, "/requested", Some(&origin));
            result.data.unwrap()
        };

        let stale = annotated(
            serde_json::json!({"workspaceRoot": "/root"}),
            WorkspaceOrigin::LaunchCwd {
                roots: Some(RootsFallback::Closed),
            },
        );
        let origin = &stale["workspaceRootOrigin"];
        assert_eq!(origin["channel"], "launchCwd");
        assert_eq!(origin["roots"], "closed");
        assert_eq!(origin["requestedDirectory"], "/requested");
        assert!(origin["hint"].as_str().unwrap().contains("roots/list"));

        let current = annotated(
            serde_json::json!({"workspaceRoot": "/root"}),
            WorkspaceOrigin::ClientRoots {},
        );
        assert_eq!(
            current["workspaceRootOrigin"],
            serde_json::json!({"channel": "clientRoots", "requestedDirectory": "/requested"})
        );

        let without_root = annotated(
            serde_json::json!({"ready": false}),
            WorkspaceOrigin::ClientRoots {},
        );
        assert!(without_root.get("workspaceRootOrigin").is_none());
    }
}
