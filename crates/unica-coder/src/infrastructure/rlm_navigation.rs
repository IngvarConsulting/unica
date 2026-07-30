use crate::application::AdapterOutcome;
use crate::domain::cancellation::{CancellationToken, CANCELLED_PREFIX};
use crate::domain::code_intelligence::{
    CodeIntelligenceContext, CodeIntelligenceReadRequest, ProviderDeadline,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::workspace_index::IndexReadiness;
use crate::infrastructure::workspace_services::{
    WorkspaceRlmOperation, WorkspaceServiceManager, WorkspaceServiceRlmOutput,
};
use serde_json::{Map, Value};
use std::path::Path;
use std::time::Duration;

const RLM_NAVIGATION_TIMEOUT: Duration = Duration::from_secs(45);

trait RlmNavigationClient: Send + Sync {
    fn readiness(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<IndexReadiness, String>;

    fn call(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        operation: WorkspaceRlmOperation,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceServiceRlmOutput, String>;
}

struct WorkspaceRlmNavigationClient;

impl RlmNavigationClient for WorkspaceRlmNavigationClient {
    fn readiness(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<IndexReadiness, String> {
        WorkspaceServiceManager::new().rlm_readiness_cancellable_with_timeout(
            context,
            source_root,
            &Map::new(),
            timeout,
            cancellation,
        )
    }

    fn call(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        operation: WorkspaceRlmOperation,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceServiceRlmOutput, String> {
        WorkspaceServiceManager::new().call_rlm_cancellable(
            context,
            source_root,
            operation,
            timeout,
            cancellation,
        )
    }
}

static WORKSPACE_RLM_NAVIGATION_CLIENT: WorkspaceRlmNavigationClient = WorkspaceRlmNavigationClient;

pub(crate) struct RlmNavigationAdapter<'a> {
    client: &'a (dyn RlmNavigationClient + Send + Sync),
}

impl RlmNavigationAdapter<'static> {
    pub(crate) fn new() -> Self {
        Self {
            client: &WORKSPACE_RLM_NAVIGATION_CLIENT,
        }
    }
}

impl<'a> RlmNavigationAdapter<'a> {
    #[cfg(test)]
    fn with_client(client: &'a (dyn RlmNavigationClient + Send + Sync)) -> Self {
        Self { client }
    }

    pub(crate) fn invoke_resolved_cancellable(
        &self,
        request: &CodeIntelligenceReadRequest,
        context: &CodeIntelligenceContext,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<AdapterOutcome, String> {
        let tool_name = request.tool_name();
        let operation = operation_for_request(request)?;
        if cancellation.is_cancelled() {
            return Ok(AdapterOutcome::cancelled(format!(
                "{tool_name} cancelled before provider work"
            )));
        }
        let readiness_timeout = deadline.remaining().min(RLM_NAVIGATION_TIMEOUT);
        if readiness_timeout.is_zero() {
            return Err(format!(
                "{tool_name} provider deadline exceeded before readiness check"
            ));
        }
        let readiness = match self.client.readiness(
            &context.workspace,
            &context.source_root.path,
            readiness_timeout,
            cancellation,
        ) {
            Ok(readiness) => readiness,
            Err(error) if error.starts_with(CANCELLED_PREFIX) => {
                return Ok(cancelled_client_outcome(tool_name, &error));
            }
            Err(error) => return Err(error),
        };
        let db_path = match readiness {
            IndexReadiness::Ready { db_path } => db_path,
            other => return Ok(index_unavailable_outcome(request, other)),
        };
        let timeout = deadline.remaining().min(RLM_NAVIGATION_TIMEOUT);
        if timeout.is_zero() {
            return Err(format!("{tool_name} provider deadline exceeded"));
        }
        let output = match self.client.call(
            &context.workspace,
            &context.source_root.path,
            operation,
            timeout,
            cancellation,
        ) {
            Ok(output) => output,
            Err(error) if error.starts_with(CANCELLED_PREFIX) => {
                return Ok(cancelled_client_outcome(tool_name, &error));
            }
            Err(error) => return Err(error),
        };
        let value: Value = serde_json::from_str(output.result_text.trim())
            .map_err(|error| format!("{tool_name} received invalid RLM helper JSON: {error}"))?;
        if let Some(error) = value
            .get("error")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            return Err(format!("{tool_name} RLM helper failed: {error}"));
        }
        let (section, body) = match tool_name {
            "unica.code.definition" => ("rlm-definition", render_definition(&value)?),
            "unica.meta.profile" => ("rlm-meta-profile", render_profile(&value)?),
            _ => return Err(format!("unsupported RLM navigation tool: {tool_name}")),
        };
        let mut outcome = AdapterOutcome::ok(format!(
            "{tool_name} completed through the persistent RLM MCP API"
        ));
        outcome.artifacts = vec![
            context.source_root.path.display().to_string(),
            db_path.display().to_string(),
        ];
        outcome.stdout = Some(format_section(section, &body));
        if !output.stderr.trim().is_empty() {
            outcome
                .warnings
                .push(format!("RLM stderr: {}", output.stderr.trim()));
            outcome.stderr = Some(output.stderr);
        }
        Ok(outcome)
    }
}

/// ADR-0021: the index serves definition and object profile. The outline is
/// built from the current BSL file, so it has no RLM operation at all and asking
/// for one is a routing defect rather than a runtime condition.
fn operation_for_request(
    request: &CodeIntelligenceReadRequest,
) -> Result<WorkspaceRlmOperation, String> {
    Ok(match request {
        CodeIntelligenceReadRequest::Definition {
            name,
            module_hint,
            limit,
        } => WorkspaceRlmOperation::Definition {
            name: name.clone(),
            module_hint: module_hint.clone(),
            limit: *limit,
        },
        CodeIntelligenceReadRequest::ObjectProfile {
            name,
            sections,
            limit,
        } => WorkspaceRlmOperation::ObjectProfile {
            name: name.clone(),
            sections: sections.clone(),
            limit: *limit,
        },
        CodeIntelligenceReadRequest::Outline { .. } => {
            return Err(format!(
                "{} is built from the current BSL source and has no RLM operation",
                request.tool_name()
            ))
        }
    })
}

fn render_definition(value: &Value) -> Result<String, String> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let definitions = value
        .get("definitions")
        .and_then(Value::as_array)
        .ok_or_else(|| "RLM definition response is missing definitions".to_string())?;
    if definitions.is_empty() {
        return Ok(format!("No RLM definitions found for `{name}`."));
    }
    let mut lines = Vec::new();
    for (index, definition) in definitions.iter().enumerate() {
        let Some(file) = definition
            .get("file")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            lines.push(format!(
                "diagnostic: ignored malformed RLM definition #{}: missing file",
                index + 1
            ));
            continue;
        };
        let line = definition
            .get("line")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let kind = definition
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("method");
        let method_name = if name.is_empty() { "<unknown>" } else { name };
        let params = definition
            .get("params")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let export = if definition
            .get("is_export")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            " export"
        } else {
            ""
        };
        let metadata = [
            ("category", definition.get("category")),
            ("object", definition.get("object_name")),
            ("moduleType", definition.get("module_type")),
        ]
        .into_iter()
        .filter_map(|(label, value)| {
            value
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(|value| format!("{label}={value}"))
        })
        .collect::<Vec<_>>();
        let suffix = if metadata.is_empty() {
            String::new()
        } else {
            format!(" [{}]", metadata.join(", "))
        };
        lines.push(format!(
            "- {file}:{line} {kind} {method_name}({params}){export}{suffix}"
        ));
    }
    Ok(lines.join("\n"))
}

fn render_profile(value: &Value) -> Result<String, String> {
    let object_name = required_value_string(value, "object_name", "RLM object profile")?;
    let category = value
        .get("category")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    let mut lines = vec![format!(
        "object: {}",
        category
            .map(|category| format!("{category}.{object_name}"))
            .unwrap_or_else(|| object_name.to_string())
    )];
    if let Some(category) = category {
        lines.push(format!("category: {category}"));
    }
    lines.push(format!("name: {object_name}"));
    let sections = value
        .get("sections")
        .and_then(Value::as_object)
        .ok_or_else(|| "RLM object profile response is missing sections".to_string())?;
    for (name, section) in sections {
        let status = section
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let total = section
            .get("total")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let returned = section
            .get("returned")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        // Upstream sections compute `total` before applying their item limit, so
        // `has_more` and `_meta.truncated` do not by themselves make that count
        // approximate. Only composed sections that cannot obtain a count mark the
        // value explicitly as a lower bound.
        let total_is_lower_bound = section
            .pointer("/_meta/total_is_lower_bound")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let more = if total_is_lower_bound { "+" } else { "" };
        lines.push(format!(
            "section {}: {status} total={total}{more} returned={returned}",
            public_profile_section_name(name)
        ));
        if let Some(items) = section.get("items").and_then(Value::as_array) {
            for item in items {
                lines.push(format!("- {}", compact_json(item)?));
            }
        }
        if let Some(error) = section
            .pointer("/_meta/error")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            lines.push(format!("  error: {error}"));
        }
    }
    Ok(lines.join("\n"))
}

fn public_profile_section_name(name: &str) -> &str {
    match name {
        "functional_options" => "functionalOptions",
        "predefined_items" => "predefinedItems",
        other => other,
    }
}

fn compact_json(value: &Value) -> Result<String, String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        _ => serde_json::to_string(value)
            .map_err(|error| format!("failed to render RLM helper item: {error}")),
    }
}

fn required_value_string<'a>(
    value: &'a Value,
    key: &str,
    description: &str,
) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{description} response is missing {key}"))
}

fn format_section(name: &str, text: &str) -> String {
    let body = text.trim_end();
    if body.is_empty() {
        format!("=== {name} ===")
    } else {
        format!("=== {name} ===\n{body}")
    }
}

fn index_unavailable_outcome(
    request: &CodeIntelligenceReadRequest,
    readiness: IndexReadiness,
) -> AdapterOutcome {
    let tool_name = request.tool_name();
    let warning = readiness_warning(readiness);
    if warning.starts_with(CANCELLED_PREFIX) {
        return AdapterOutcome::cancelled(
            warning
                .strip_prefix(CANCELLED_PREFIX)
                .unwrap_or(&warning)
                .trim(),
        );
    }
    // Definition and object profile still answer something useful without the
    // index, so an unready index is a warning rather than a typed failure.
    let mut outcome = AdapterOutcome::ok(format!(
        "{tool_name} could not use the persistent RLM MCP API"
    ));
    outcome.warnings.push(warning);
    outcome
}

fn cancelled_client_outcome(tool_name: &str, error: &str) -> AdapterOutcome {
    AdapterOutcome::cancelled(format!(
        "{tool_name} {}",
        error.strip_prefix(CANCELLED_PREFIX).unwrap_or(error).trim()
    ))
}

fn readiness_warning(readiness: IndexReadiness) -> String {
    match readiness {
        IndexReadiness::Ready { .. } => {
            unreachable!("ready indexes are handled before readiness warnings")
        }
        IndexReadiness::Missing => "rlm index unavailable: index is missing".to_string(),
        IndexReadiness::Stale { status } => format!("rlm index stale: {status}"),
        IndexReadiness::Building => "rlm index building".to_string(),
        IndexReadiness::Failed(error) | IndexReadiness::Unavailable(error)
            if error.starts_with(CANCELLED_PREFIX) =>
        {
            error
        }
        IndexReadiness::Failed(error) | IndexReadiness::Unavailable(error) => {
            format!("rlm index unavailable: {error}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        operation_for_request, render_definition, render_profile, RlmNavigationAdapter,
        RlmNavigationClient,
    };
    use crate::application::AdapterOutcome;
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::{
        CodeIntelligenceContext, CodeIntelligenceReadRequest, ProviderDeadline,
    };
    use crate::domain::source_roots::ResolvedSourceRoot;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::workspace_index::IndexReadiness;
    use crate::infrastructure::workspace_services::{
        WorkspaceRlmOperation, WorkspaceServiceRlmOutput,
    };
    use serde_json::json;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    #[test]
    fn definition_renderer_preserves_public_text_contract_from_helper_json() {
        let text = render_definition(&json!({
            "name": "Найти",
            "definitions": [{
                "file": "CommonModules/X/Module.bsl",
                "line": 7,
                "type": "function",
                "is_export": true,
                "params": ["Значение"],
                "category": "CommonModule",
                "object_name": "X",
                "module_type": "Module"
            }]
        }))
        .unwrap();

        assert_eq!(
            text,
            "- CommonModules/X/Module.bsl:7 function Найти(Значение) export [category=CommonModule, object=X, moduleType=Module]"
        );
    }

    #[test]
    fn definition_renderer_keeps_valid_rows_and_reports_malformed_siblings() {
        let text = render_definition(&json!({
            "name": "Найти",
            "definitions": [
                {
                    "file": "CommonModules/X/Module.bsl",
                    "line": 7,
                    "type": "function"
                },
                {
                    "line": 11,
                    "type": "procedure"
                }
            ]
        }))
        .unwrap();

        assert!(text.contains("CommonModules/X/Module.bsl:7"));
        assert!(text.contains("diagnostic: ignored malformed RLM definition #2"));
    }

    #[test]
    fn profile_renderer_maps_upstream_section_names_to_public_names() {
        let text = render_profile(&json!({
            "object_name": "Заказ",
            "category": "Document",
            "sections": {
                "functional_options": {
                    "status": "ok",
                    "items": [{"name": "ИспользоватьЗаказы"}],
                    "total": 1,
                    "returned": 1
                },
                "predefined_items": {
                    "status": "empty",
                    "items": [],
                    "total": 0,
                    "returned": 0
                }
            }
        }))
        .unwrap();

        assert!(text.contains("object: Document.Заказ"));
        assert!(text.contains("section functionalOptions: ok total=1 returned=1"));
        assert!(text.contains("section predefinedItems: empty total=0 returned=0"));
    }

    #[test]
    fn profile_renderer_reports_a_truncated_section_as_a_lower_bound() {
        let text = render_profile(&json!({
            "object_name": "Заказ",
            "category": "Document",
            "sections": {
                "predefined_items": {
                    "status": "ok",
                    "items": [{"name": "Основной"}],
                    "total": 1,
                    "returned": 1,
                    "has_more": true,
                    "_meta": {
                        "source": "index",
                        "truncated": true,
                        "total_is_lower_bound": true
                    }
                }
            }
        }))
        .unwrap();

        assert!(
            text.contains("section predefinedItems: ok total=1+ returned=1"),
            "{text}"
        );
    }

    #[test]
    fn profile_renderer_keeps_upstream_exact_total_when_items_are_limited() {
        let text = render_profile(&json!({
            "object_name": "Заказ",
            "category": "Document",
            "sections": {
                "structure": {
                    "status": "ok",
                    "items": [{"name": "Реквизит1"}],
                    "total": 100,
                    "returned": 20,
                    "has_more": true
                }
            }
        }))
        .unwrap();

        assert!(
            text.contains("section structure: ok total=100 returned=20"),
            "{text}"
        );
        assert!(!text.contains("total=100+"), "{text}");
    }

    #[test]
    fn profile_renderer_keeps_an_exact_total_when_nothing_was_truncated() {
        let text = render_profile(&json!({
            "object_name": "Заказ",
            "category": "Document",
            "sections": {
                "predefined_items": {
                    "status": "ok",
                    "items": [{"name": "Основной"}],
                    "total": 1,
                    "returned": 1,
                    "has_more": false,
                    "_meta": {"source": "index", "truncated": false}
                }
            }
        }))
        .unwrap();

        assert!(
            text.contains("section predefinedItems: ok total=1 returned=1"),
            "{text}"
        );
        assert!(!text.contains("total=1+"), "{text}");
    }

    #[test]
    fn object_profile_operation_keeps_predefined_items_request() {
        let request = CodeIntelligenceReadRequest::ObjectProfile {
            name: "Document.Заказ".to_string(),
            sections: Some(vec![
                "structure".to_string(),
                "functionalOptions".to_string(),
                "predefinedItems".to_string(),
            ]),
            limit: 11,
        };
        let operation = operation_for_request(&request).unwrap();

        assert_eq!(
            operation,
            WorkspaceRlmOperation::ObjectProfile {
                name: "Document.Заказ".to_string(),
                sections: Some(vec![
                    "structure".to_string(),
                    "functionalOptions".to_string(),
                    "predefinedItems".to_string()
                ]),
                limit: 11
            }
        );
    }

    struct RecordingClient {
        operations: Mutex<Vec<WorkspaceRlmOperation>>,
    }

    struct CancelledClient {
        cancel_during_call: bool,
    }

    impl RlmNavigationClient for CancelledClient {
        fn readiness(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<IndexReadiness, String> {
            if self.cancel_during_call {
                Ok(IndexReadiness::Ready {
                    db_path: PathBuf::from("/tmp/index.db"),
                })
            } else {
                Err("cancelled: readiness stopped".to_string())
            }
        }

        fn call(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _operation: WorkspaceRlmOperation,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<WorkspaceServiceRlmOutput, String> {
            Err("cancelled: provider call stopped".to_string())
        }
    }

    #[test]
    fn adapter_normalizes_client_cancellation_from_readiness_and_call() {
        let request = CodeIntelligenceReadRequest::Definition {
            name: "Найти".to_string(),
            module_hint: String::new(),
            limit: 50,
        };
        let context = CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: PathBuf::from("/workspace"),
                workspace_root: PathBuf::from("/workspace"),
                cache_root: PathBuf::from("/cache"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: PathBuf::from("/workspace/src"),
            },
        );

        for cancel_during_call in [false, true] {
            let outcome =
                RlmNavigationAdapter::with_client(&CancelledClient { cancel_during_call })
                    .invoke_resolved_cancellable(
                        &request,
                        &context,
                        ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
                        &CancellationToken::new(),
                    )
                    .expect("client cancellation must be normalized into an outcome");

            assert!(!outcome.ok);
            assert!(outcome.summary.contains("cancelled"));
        }
    }

    struct UnreadyClient {
        readiness: IndexReadiness,
    }

    impl RlmNavigationClient for UnreadyClient {
        fn readiness(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<IndexReadiness, String> {
            Ok(self.readiness.clone())
        }

        fn call(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _operation: WorkspaceRlmOperation,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<WorkspaceServiceRlmOutput, String> {
            panic!("a not-ready index must never reach the RLM call")
        }
    }

    fn unready_index_outcome(
        request: &CodeIntelligenceReadRequest,
        readiness: IndexReadiness,
    ) -> AdapterOutcome {
        RlmNavigationAdapter::with_client(&UnreadyClient { readiness })
            .invoke_resolved_cancellable(
                request,
                &unready_index_context(),
                ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
                &CancellationToken::new(),
            )
            .expect("a not-ready index must be reported as an outcome")
    }

    fn unready_index_context() -> CodeIntelligenceContext {
        CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: PathBuf::from("/workspace"),
                workspace_root: PathBuf::from("/workspace"),
                cache_root: PathBuf::from("/cache"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: PathBuf::from("/workspace/src"),
            },
        )
    }

    fn outline_request() -> CodeIntelligenceReadRequest {
        CodeIntelligenceReadRequest::Outline {
            path: "CommonModules/X/Ext/Module.bsl".to_string(),
            include_methods: true,
        }
    }

    fn definition_request() -> CodeIntelligenceReadRequest {
        CodeIntelligenceReadRequest::Definition {
            name: "ОбщегоНазначения".to_string(),
            module_hint: String::new(),
            limit: 50,
        }
    }

    #[test]
    fn definition_and_profile_keep_the_warning_only_contract_for_an_unready_index() {
        for request in [
            CodeIntelligenceReadRequest::Definition {
                name: "Найти".to_string(),
                module_hint: String::new(),
                limit: 50,
            },
            CodeIntelligenceReadRequest::ObjectProfile {
                name: "Справочники.Номенклатура".to_string(),
                sections: None,
                limit: 20,
            },
        ] {
            let outcome = unready_index_outcome(&request, IndexReadiness::Missing);

            assert!(outcome.ok, "{}", request.tool_name());
            assert!(outcome.errors.is_empty(), "{}", request.tool_name());
            assert_eq!(
                outcome.warnings,
                vec!["rlm index unavailable: index is missing".to_string()]
            );
        }
    }

    #[test]
    fn a_cancelled_readiness_is_reported_as_cancellation_not_as_an_index_failure() {
        let outcome = unready_index_outcome(
            &definition_request(),
            IndexReadiness::Unavailable("cancelled: readiness stopped".to_string()),
        );

        assert!(!outcome.ok);
        assert!(outcome.summary.contains("cancelled"), "{}", outcome.summary);
        assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    }

    #[test]
    fn the_index_adapter_refuses_to_serve_the_outline() {
        // ADR-0021: the outline is owned by the current-source provider. Reaching
        // this adapter with it is a routing defect, so it fails before any RLM
        // work rather than answering from the index.
        let client = RecordingClient {
            operations: Mutex::new(Vec::new()),
        };
        let error = RlmNavigationAdapter::with_client(&client)
            .invoke_resolved_cancellable(
                &outline_request(),
                &unready_index_context(),
                ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
                &CancellationToken::new(),
            )
            .unwrap_err();

        assert!(
            error.contains("built from the current BSL source"),
            "{error}"
        );
        assert!(operation_for_request(&outline_request()).is_err());
        assert!(
            client.operations.lock().unwrap().is_empty(),
            "a misrouted outline must not reach the RLM client"
        );
    }

    impl RlmNavigationClient for RecordingClient {
        fn readiness(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<IndexReadiness, String> {
            Ok(IndexReadiness::Ready {
                db_path: PathBuf::from("/tmp/index.db"),
            })
        }

        fn call(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            operation: WorkspaceRlmOperation,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<WorkspaceServiceRlmOutput, String> {
            self.operations.lock().unwrap().push(operation);
            Ok(WorkspaceServiceRlmOutput {
                result_text: json!({
                    "name": "Найти",
                    "definitions": [],
                    "total": 0,
                    "truncated": false
                })
                .to_string(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn adapter_routes_definition_through_rlm_client() {
        let root = std::env::temp_dir().join(format!(
            "unica-rlm-navigation-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "source-set:\n  - name: main\n    path: src\n    type: CONFIGURATION\n",
        )
        .unwrap();
        let context = CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: root.join("src"),
            },
        );
        let client = RecordingClient {
            operations: Mutex::new(Vec::new()),
        };
        let request = CodeIntelligenceReadRequest::Definition {
            name: "Найти".to_string(),
            module_hint: String::new(),
            limit: 50,
        };

        let outcome = RlmNavigationAdapter::with_client(&client)
            .invoke_resolved_cancellable(
                &request,
                &context,
                ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert!(outcome.ok);
        assert!(outcome
            .stdout
            .as_deref()
            .unwrap()
            .contains("No RLM definitions found"));
        assert_eq!(
            client.operations.lock().unwrap().as_slice(),
            &[WorkspaceRlmOperation::Definition {
                name: "Найти".to_string(),
                module_hint: String::new(),
                limit: 50
            }]
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
