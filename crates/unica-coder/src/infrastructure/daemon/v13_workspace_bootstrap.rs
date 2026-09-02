use super::protocol::InvocationRequest;
use crate::application::invocation::InvocationResponseDeadline;
use crate::application::invocation_store::ToolIdentity;
use crate::domain::address::QualifiedAddress;
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::invocation::DomainResult;
use crate::domain::project_health::evaluate_project_health;
use crate::domain::project_sources::{ProjectSourceMap, SourceFormat, SourceSetKind};
use crate::infrastructure::project_health::inspect_project_health;
use crate::infrastructure::project_sources::discover_project_source_map_controlled;
use crate::infrastructure::workspace::discover_workspace;
use serde::Serialize;
use serde_json::{Map, Value};
use std::path::PathBuf;

pub(super) fn execute_view_bootstrap(
    request: &InvocationRequest,
    deadline: &InvocationResponseDeadline,
) -> Option<DomainResult> {
    if request.tool() != ToolIdentity::View || request.arguments().contains_key("at") {
        return None;
    }
    if !request.arguments().is_empty() {
        let mut result = DomainResult::canonical_rejection(
            None,
            "bad_value",
            "view filter, limit, and cursor require logical argument `at`; call unica.view with an empty object to inspect the workspace",
        );
        result.next.push(next_action(
            "unica.view",
            Value::Object(Map::new()),
            "discover source sets and canonical logical addresses",
        ));
        return Some(result);
    }

    let context = match discover_workspace(Some(PathBuf::from(request.workspace_hint()))) {
        Ok(context) => context,
        Err(error) => {
            return Some(DomainResult::canonical_rejection(
                None,
                "provider_unavailable",
                format!("workspace discovery failed: {error}"),
            ))
        }
    };
    let config = context.workspace_root.join("v8project.yaml");
    let config_present = match std::fs::symlink_metadata(&config) {
        Ok(_) => true,
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    };
    let mut checkpoint = || deadline.checkpoint_handoff().map_err(str::to_string);
    let source_map =
        match discover_project_source_map_controlled(&context.workspace_root, &mut checkpoint) {
            Ok(source_map) => source_map,
            Err(error) if config_present => {
                let mut result = DomainResult::canonical_rejection(
                    None,
                    "invalid_state",
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
                return Some(result);
            }
            Err(error) => {
                return Some(DomainResult::canonical_rejection(
                    None,
                    "provider_unavailable",
                    format!("workspace source discovery failed: {error}"),
                ))
            }
        };
    Some(bootstrap_result(&context, source_map, deadline))
}

fn bootstrap_result(
    context: &crate::domain::workspace::WorkspaceContext,
    source_map: ProjectSourceMap,
    response_deadline: &InvocationResponseDeadline,
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
        let health_budget = response_deadline.remaining_handoff_budget();
        let cancellation = CancellationToken::new();
        Some(
            inspect_project_health(
                context,
                &cancellation,
                ProviderDeadline::from_budget(health_budget),
            )
            .map_err(|error| format!("{error:?}"))
            .and_then(evaluate_project_health),
        )
    };
    let (mut ready, repository_ready, checks, mut diagnostics, readiness_state) = match health {
        None => (
            false,
            false,
            Value::Array(Vec::new()),
            Value::Array(vec![object([
                ("code", Value::String("source_roots_missing".to_string())),
                (
                    "message",
                    Value::String(
                        "No 1C source roots were found; choose whether to create sources or import them before creating v8project.yaml"
                            .to_string(),
                    ),
                ),
            ])]),
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
    let setup = if config_state == "configured" && source_map.source_sets.is_empty() {
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
                        "No source root can be attached yet; create or import the intended configuration, extension, external processor, or external report first."
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

    let source_sets = serde_json::to_value(&source_map.source_sets)
        .expect("project source sets always serialize");
    let mut result = DomainResult::success(match config_state {
        "configured" => "workspace configuration and source sets discovered",
        "autodetected" => "source sets autodetected; v8project.yaml is not present",
        _ => "workspace has no 1C source roots; create or import sources before attaching them",
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
            "effectiveSourceSet",
            value(&source_map.effective_source_set),
        ),
        (
            "effectiveSourceRoot",
            value(&source_map.effective_source_root),
        ),
        (
            "sourceSelectionError",
            value(&source_map.source_selection_error),
        ),
        ("ready", Value::Bool(ready)),
        ("discoveredReady", Value::Bool(discovered_ready)),
        ("repositoryReady", Value::Bool(repository_ready)),
        ("readinessState", Value::String(readiness_state.to_string())),
        ("checks", checks),
        ("diagnostics", diagnostics),
        ("setup", setup.unwrap_or(Value::Null)),
    ]));
    if config_state == "autodetected" && project_config_recipe(&source_map).is_some() {
        result.next.push(next_action(
            "unica.run",
            object([
                ("op", Value::String("workspace.initialize".to_string())),
                ("args", Value::Object(Map::new())),
                ("dryRun", Value::Bool(true)),
            ]),
            "preview creation of v8project.yaml from the autodetected source sets",
        ));
    }
    if let (true, Some(Ok(at))) = (ready, next_address) {
        result.next.push(next_action(
            "unica.view",
            object([("at", Value::String(at))]),
            "inspect the root logical node of the selected source set",
        ));
        result.next.push(next_action(
            "unica.check",
            Value::Object(Map::new()),
            "confirm source-set admission after discovery",
        ));
    }
    result
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

fn next_action(tool: &str, args: Value, reason: &str) -> Value {
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
    use super::project_config_recipe;
    use crate::domain::project_sources::{
        ProjectSourceMap, ProjectSourceSet, SourceFormat, SourceSetKind,
    };

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
}
