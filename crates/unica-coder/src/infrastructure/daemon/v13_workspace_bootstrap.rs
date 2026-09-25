use super::protocol::InvocationRequest;
use crate::application::invocation::InvocationResponseDeadline;
use crate::application::invocation_store::ToolIdentity;
use crate::domain::address::QualifiedAddress;
use crate::domain::cancellation::{cancelled_error, CancellationToken};
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::invocation::{DomainResult, InvocationFailure, SafeIdentityHash};
use crate::domain::project_health::evaluate_project_health;
use crate::domain::project_sources::{ProjectSourceMap, SourceFormat, SourceSetKind};
use crate::domain::refusal::RefusalCode;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform::secure_read::read_root_relative_regular_file;
use crate::infrastructure::project_health::inspect_project_health;
use crate::infrastructure::project_sources::discover_project_source_map_controlled;
use crate::infrastructure::source_roots::normalize_path_identity;
use crate::infrastructure::workspace::discover_workspace;
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;

const PROJECT_CONFIG_MAX_BYTES: usize = 8 * 1024 * 1024;

#[cfg(test)]
pub(super) mod test_control;

#[derive(Debug, Clone, PartialEq, Eq)]
struct InfobaseTarget {
    configured: bool,
    source: Option<&'static str>,
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
    question: RootQuestion,
    response_deadline: InvocationResponseDeadline,
    workspace_identity_hash: SafeIdentityHash,
}

pub(super) enum Preparation {
    NotApplicable,
    Rejected(Box<DomainResult>),
    Ready(Arc<PreparedWorkspaceInspection>),
}

pub(super) fn prepare(
    request: &InvocationRequest,
    response_deadline: InvocationResponseDeadline,
) -> Preparation {
    if request.arguments().contains_key("at") {
        return Preparation::NotApplicable;
    }
    let question = match request.tool() {
        ToolIdentity::View => RootQuestion::Facts,
        ToolIdentity::Check => RootQuestion::Verdict,
        _ => return Preparation::NotApplicable,
    };
    if !request.arguments().is_empty() {
        let (tool, summary) = match question {
            RootQuestion::Facts => (
                "unica.view",
                "view filter, limit, and cursor require logical argument `at`; call unica.view with an empty object to inspect the workspace",
            ),
            RootQuestion::Verdict => (
                "unica.check",
                "check limit and cursor require `at`; call unica.check with an empty object for the workspace verdict",
            ),
        };
        let mut result = DomainResult::canonical_rejection(None, RefusalCode::BadValue, summary);
        result.next.push(next_action(
            tool,
            Value::Object(Map::new()),
            "discover source sets and canonical logical addresses",
        ));
        return Preparation::Rejected(Box::new(result));
    }

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
    Preparation::Ready(Arc::new(PreparedWorkspaceInspection {
        context,
        question,
        response_deadline,
        workspace_identity_hash: SafeIdentityHash::from_sha256(hasher.finalize().into()),
    }))
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
        check_cancellation(&cancellation)?;
        let context = &self.context;
        let deadline = &self.response_deadline;
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
        let result = bootstrap_result(
            context,
            source_map,
            infobase,
            deadline,
            self.question,
            &cancellation,
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
    context: &crate::domain::workspace::WorkspaceContext,
    source_map: ProjectSourceMap,
    infobase: InfobaseTarget,
    response_deadline: &InvocationResponseDeadline,
    question: RootQuestion,
    cancellation: &CancellationToken,
) -> DomainResult {
    let config_state = if source_map.config_path.is_some() {
        "configured"
    } else if source_map.source_sets.is_empty() {
        "missing"
    } else {
        "autodetected"
    };
    let discovered_ready = source_map.effective_source_set.is_some()
        && source_map.source_selection_error.is_none()
        && !source_map.source_sets.is_empty()
        && source_map
            .source_sets
            .iter()
            .all(|source| source.source_format == SourceFormat::PlatformXml);
    let health = if source_map.source_sets.is_empty() {
        None
    } else {
        #[cfg(test)]
        test_control::pause_before_health(&context.workspace_root);
        let health_budget = response_deadline.remaining_handoff_budget();
        Some(
            inspect_project_health(
                context,
                cancellation,
                ProviderDeadline::from_budget(health_budget),
            )
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
    if ready && next_address.as_ref().is_some_and(Result::is_err) {
        ready = false;
        if let Value::Array(items) = &mut diagnostics {
            let source_name = actionable_source
                .map(|source| source.name.as_str())
                .unwrap_or_default();
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
    let setup = if config_state == "configured"
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
        result.data = Some(object([
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
        ]));
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
        ("setup", setup.unwrap_or(Value::Null)),
    ]));
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
    Ok(InfobaseTarget {
        configured,
        source: configured.then_some(source.expect("configured connection has a source")),
    })
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
    use crate::domain::project_sources::{
        ProjectSourceMap, ProjectSourceSet, SourceFormat, SourceSetKind,
    };

    #[test]
    fn root_inspection_discovers_sources_after_response_handoff() {
        use super::{prepare, Preparation};
        use crate::application::invocation::InvocationResponseDeadline;
        use crate::application::invocation_store::ToolIdentity;
        use crate::application::ports::Clock;
        use crate::domain::cancellation::CancellationToken;
        use crate::infrastructure::daemon::protocol::InvocationRequest;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;
        use std::time::{Duration, Instant};

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
                let Preparation::Ready(inspection) =
                    prepare(&request, InvocationResponseDeadline::capture(clock.clone()))
                else {
                    panic!("root inspection must prepare before response handoff");
                };

                clock.elapsed_ms.store(9_000, Ordering::SeqCst);
                let result = inspection.execute(CancellationToken::new()).unwrap();
                assert!(result.ok, "{tool:?}, configured={configured}: {result:?}");
                let data = result.data.unwrap();
                if tool == ToolIdentity::Check {
                    assert_eq!(data["readinessState"], "incomplete");
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
        use super::{prepare, Preparation};
        use crate::application::invocation::InvocationResponseDeadline;
        use crate::application::invocation_store::ToolIdentity;
        use crate::application::ports::TokioClock;
        use crate::domain::cancellation::CancellationToken;
        use crate::infrastructure::daemon::protocol::InvocationRequest;
        use std::sync::Arc;

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
        ) else {
            panic!("root inspection must prepare before source admission");
        };
        let pause = super::test_control::HealthInspectionPause::install(root);
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
                source_state: crate::domain::project_sources::SourceSetState::Supported,
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
                source_state: crate::domain::project_sources::SourceSetState::Unsupported,
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
}
