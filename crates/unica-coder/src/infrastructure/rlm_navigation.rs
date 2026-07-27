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
            other => return Ok(index_unavailable_outcome(tool_name, other)),
        };
        let timeout = deadline.remaining().min(RLM_NAVIGATION_TIMEOUT);
        if timeout.is_zero() {
            return Err(format!("{tool_name} provider deadline exceeded"));
        }
        let operation = operation_for_request(request);
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
            "unica.code.outline" => ("rlm-outline", render_outline(&value)?),
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

fn operation_for_request(request: &CodeIntelligenceReadRequest) -> WorkspaceRlmOperation {
    match request {
        CodeIntelligenceReadRequest::Definition {
            name,
            module_hint,
            limit,
        } => WorkspaceRlmOperation::Definition {
            name: name.clone(),
            module_hint: module_hint.clone(),
            limit: *limit,
        },
        CodeIntelligenceReadRequest::Outline {
            path,
            include_methods,
        } => WorkspaceRlmOperation::Outline {
            path: path.clone(),
            include_methods: *include_methods,
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
    }
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

fn render_outline(value: &Value) -> Result<String, String> {
    let path = required_value_string(value, "path", "RLM outline")?;
    let mut lines = vec![format!("module: {path}")];
    push_optional_line(&mut lines, "object", value.get("object_name"));
    push_optional_line(&mut lines, "category", value.get("category"));
    push_optional_line(&mut lines, "moduleType", value.get("module_type"));
    if let Some(totals) = value.get("totals").and_then(Value::as_object) {
        lines.push(format!(
            "totals: methods={} exports={} regions={} loc={}",
            json_count(totals.get("methods")),
            json_count(totals.get("exports")),
            json_count(totals.get("regions")),
            json_count(totals.get("loc"))
        ));
    }
    if let Some(outline) = value.get("outline").and_then(Value::as_array) {
        render_outline_nodes(outline, 0, &mut lines);
    }
    if let Some(methods) = value.get("orphan_methods").and_then(Value::as_array) {
        render_outline_methods(methods, 0, &mut lines);
    }
    Ok(lines.join("\n"))
}

fn render_outline_nodes(nodes: &[Value], depth: usize, lines: &mut Vec<String>) {
    for node in nodes {
        let name = node
            .get("region")
            .and_then(Value::as_str)
            .unwrap_or("<unnamed>");
        let line = node.get("line").and_then(Value::as_u64).unwrap_or_default();
        let end_line = node
            .get("end_line")
            .and_then(Value::as_u64)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "?".to_string());
        lines.push(format!(
            "{}region {name}: {line}-{end_line}",
            "  ".repeat(depth)
        ));
        if let Some(methods) = node.get("methods").and_then(Value::as_array) {
            render_outline_methods(methods, depth + 1, lines);
        }
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            render_outline_nodes(children, depth + 1, lines);
        }
    }
}

fn render_outline_methods(methods: &[Value], depth: usize, lines: &mut Vec<String>) {
    for method in methods {
        let kind = method
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("method");
        let name = method
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>");
        let params = method
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
        let export = if method
            .get("is_export")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            " export"
        } else {
            ""
        };
        let line = method
            .get("line")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let end_line = method
            .get("end_line")
            .and_then(Value::as_u64)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "?".to_string());
        lines.push(format!(
            "{}{kind} {name}({params}){export} at {line}-{end_line}",
            "  ".repeat(depth)
        ));
    }
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
        lines.push(format!(
            "section {}: {status} total={total} returned={returned}",
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

fn push_optional_line(lines: &mut Vec<String>, label: &str, value: Option<&Value>) {
    if let Some(value) = value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("{label}: {value}"));
    }
}

fn json_count(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or_default()
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

fn index_unavailable_outcome(tool_name: &str, readiness: IndexReadiness) -> AdapterOutcome {
    let warning = readiness_warning(readiness);
    if warning.starts_with(CANCELLED_PREFIX) {
        return AdapterOutcome::cancelled(
            warning
                .strip_prefix(CANCELLED_PREFIX)
                .unwrap_or(&warning)
                .trim(),
        );
    }
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
        operation_for_request, render_definition, render_outline, render_profile,
        RlmNavigationAdapter, RlmNavigationClient,
    };
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
    fn outline_renderer_handles_region_tree_and_orphans() {
        let text = render_outline(&json!({
            "path": "CommonModules/X/Module.bsl",
            "category": "CommonModule",
            "object_name": "X",
            "module_type": "Module",
            "totals": {"methods": 2, "exports": 1, "regions": 1, "loc": 40},
            "outline": [{
                "region": "API",
                "line": 1,
                "end_line": 20,
                "methods": [{
                    "name": "Запустить",
                    "type": "procedure",
                    "params": [],
                    "is_export": true,
                    "line": 3,
                    "end_line": 9
                }],
                "children": []
            }],
            "orphan_methods": [{
                "name": "Внутренняя",
                "type": "function",
                "params": [],
                "is_export": false,
                "line": 22,
                "end_line": 30
            }]
        }))
        .unwrap();

        assert!(text.contains("module: CommonModules/X/Module.bsl"));
        assert!(text.contains("region API: 1-20"));
        assert!(text.contains("procedure Запустить() export at 3-9"));
        assert!(text.contains("function Внутренняя() at 22-30"));
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
        let operation = operation_for_request(&request);

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
