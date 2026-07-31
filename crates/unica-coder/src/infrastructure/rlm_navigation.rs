use crate::application::AdapterOutcome;
use crate::domain::cancellation::{CancellationToken, CANCELLED_PREFIX};
use crate::domain::code_intelligence::{
    CodeDefinition, CodeDefinitionResult, CodeIntelligenceContext, CodeIntelligenceReadData,
    CodeIntelligenceReadRequest, MetaProfileResult, MetaProfileSection, ProviderDeadline,
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
    ) -> Result<RlmNavigationOutcome, String> {
        let tool_name = request.tool_name();
        let operation = operation_for_request(request)?;
        if cancellation.is_cancelled() {
            return Ok(RlmNavigationOutcome::plain(AdapterOutcome::cancelled(
                format!("{tool_name} cancelled before provider work"),
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
                return Ok(RlmNavigationOutcome::plain(cancelled_client_outcome(
                    tool_name, &error,
                )));
            }
            Err(error) => return Err(error),
        };
        let db_path = match readiness {
            IndexReadiness::Ready { db_path } => db_path,
            other => {
                return Ok(RlmNavigationOutcome::plain(index_unavailable_outcome(
                    request, other,
                )))
            }
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
                return Ok(RlmNavigationOutcome::plain(cancelled_client_outcome(
                    tool_name, &error,
                )));
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
        let mut outcome = AdapterOutcome::ok(format!(
            "{tool_name} completed through the persistent RLM MCP API"
        ));
        let data;
        match tool_name {
            // ADR-0023: the index already answers with structure, so the tool
            // publishes it instead of rendering it into a line grammar.
            "unica.code.definition" => {
                let (result, warnings) = definition_result(&value)?;
                outcome.summary = format!(
                    "{tool_name} found {} definition(s) for {}",
                    result.definitions.len(),
                    result.name
                );
                outcome.warnings.extend(warnings);
                data = Some(CodeIntelligenceReadData::Definition(result));
            }
            "unica.meta.profile" => {
                let result = profile_result(&value)?;
                outcome.summary = format!(
                    "{tool_name} described {} across {} section(s)",
                    result.object_name,
                    result.sections.len()
                );
                data = Some(CodeIntelligenceReadData::ObjectProfile(result));
            }
            _ => return Err(format!("unsupported RLM navigation tool: {tool_name}")),
        }
        outcome.artifacts = vec![
            context.source_root.path.display().to_string(),
            db_path.display().to_string(),
        ];
        if !output.stderr.trim().is_empty() {
            outcome
                .warnings
                .push(format!("RLM stderr: {}", output.stderr.trim()));
            outcome.stderr = Some(output.stderr);
        }
        Ok(RlmNavigationOutcome { outcome, data })
    }
}

/// A navigation answer plus the typed payload the tool publishes, when its
/// contract has one.
#[derive(Debug)]
pub(crate) struct RlmNavigationOutcome {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<CodeIntelligenceReadData>,
}

impl RlmNavigationOutcome {
    fn plain(outcome: AdapterOutcome) -> Self {
        Self {
            outcome,
            data: None,
        }
    }
}

/// ADR-0020: the index serves definition and object profile. The outline is
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

/// Reads the index answer as data. A malformed entry becomes a warning instead
/// of a `diagnostic:` line mixed into the report, so the caller can tell a
/// dropped definition from one that was never there.
fn definition_result(value: &Value) -> Result<(CodeDefinitionResult, Vec<String>), String> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let definitions = value
        .get("definitions")
        .and_then(Value::as_array)
        .ok_or_else(|| "RLM definition response is missing definitions".to_string())?;
    let mut warnings = Vec::new();
    let mut typed = Vec::new();
    for (index, definition) in definitions.iter().enumerate() {
        let Some(file) = definition
            .get("file")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            warnings.push(format!(
                "ignored malformed RLM definition #{}: missing file",
                index + 1
            ));
            continue;
        };
        let optional = |key: &str| {
            definition
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        typed.push(CodeDefinition {
            file: file.to_string(),
            line: definition
                .get("line")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            kind: definition
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("method")
                .to_string(),
            params: definition
                .get("params")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            export: definition
                .get("is_export")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            category: optional("category"),
            object_name: optional("object_name"),
            module_type: optional("module_type"),
        });
    }
    Ok((
        CodeDefinitionResult {
            name,
            definitions: typed,
        },
        warnings,
    ))
}

/// Reads the object profile as data. Section items keep the shape the index
/// gave them instead of being flattened to one line each.
fn profile_result(value: &Value) -> Result<MetaProfileResult, String> {
    let object_name = required_value_string(value, "object_name", "RLM object profile")?;
    let category = value
        .get("category")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let sections = value
        .get("sections")
        .and_then(Value::as_object)
        .ok_or_else(|| "RLM object profile response is missing sections".to_string())?;
    Ok(MetaProfileResult {
        object_name: object_name.to_string(),
        category,
        sections: sections
            .iter()
            .map(|(name, section)| MetaProfileSection {
                name: public_profile_section_name(name).to_string(),
                status: section
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                total: section
                    .get("total")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                // Upstream counts `total` before applying its item limit, so
                // only a section that cannot count marks the value as a floor.
                total_is_lower_bound: section
                    .pointer("/_meta/total_is_lower_bound")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                returned: section
                    .get("returned")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                items: section
                    .get("items")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
                error: section
                    .pointer("/_meta/error")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
            })
            .collect(),
    })
}

fn public_profile_section_name(name: &str) -> &str {
    match name {
        "functional_options" => "functionalOptions",
        "predefined_items" => "predefinedItems",
        other => other,
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
    use super::{operation_for_request, profile_result, RlmNavigationAdapter, RlmNavigationClient};
    use crate::application::AdapterOutcome;
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::CodeIntelligenceReadData;
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
    fn a_definition_keeps_every_field_the_helper_reported() {
        let (result, warnings) = super::definition_result(&json!({
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

        assert!(warnings.is_empty());
        let definition = &result.definitions[0];
        assert_eq!(definition.file, "CommonModules/X/Module.bsl");
        assert_eq!(definition.line, 7);
        assert_eq!(definition.kind, "function");
        assert!(definition.export);
        assert_eq!(definition.params, vec!["Значение".to_string()]);
        assert_eq!(definition.category.as_deref(), Some("CommonModule"));
        assert_eq!(definition.module_type.as_deref(), Some("Module"));
    }

    #[test]
    fn a_malformed_definition_is_dropped_with_a_warning_not_listed_as_one() {
        let (result, warnings) = super::definition_result(&json!({
            "name": "Найти",
            "definitions": [
                {"file": "CommonModules/X/Module.bsl", "line": 7, "type": "function"},
                {"line": 11, "type": "procedure"}
            ]
        }))
        .unwrap();

        assert_eq!(result.definitions.len(), 1);
        assert_eq!(result.definitions[0].line, 7);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("missing file"), "{warnings:?}");
    }

    #[test]
    fn a_profile_maps_upstream_section_names_to_public_names() {
        let result = profile_result(&json!({
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

        assert_eq!(result.object_name, "Заказ");
        assert_eq!(result.category.as_deref(), Some("Document"));
        let names = result
            .sections
            .iter()
            .map(|section| section.name.as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"functionalOptions"), "{names:?}");
        assert!(names.contains(&"predefinedItems"), "{names:?}");
        // Items keep their own shape rather than one line of rendered JSON.
        let options = result
            .sections
            .iter()
            .find(|section| section.name == "functionalOptions")
            .unwrap();
        assert_eq!(options.items[0]["name"], "ИспользоватьЗаказы");
    }

    #[test]
    fn a_section_that_cannot_count_marks_its_total_as_a_lower_bound() {
        let result = profile_result(&json!({
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

        assert!(result.sections[0].total_is_lower_bound);
        assert_eq!(result.sections[0].total, 1);
    }

    /// Upstream counts before applying its item limit, so a limited section
    /// still reports an exact total.
    #[test]
    fn a_limited_section_keeps_its_exact_total() {
        let result = profile_result(&json!({
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

        assert_eq!(result.sections[0].total, 100);
        assert_eq!(result.sections[0].returned, 20);
        assert!(!result.sections[0].total_is_lower_bound);
    }

    #[test]
    fn an_untruncated_section_keeps_its_exact_total() {
        let result = profile_result(&json!({
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

        assert_eq!(result.sections[0].total, 1);
        assert!(!result.sections[0].total_is_lower_bound);
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

    /// The index answers with structure; the tool must publish it rather than
    /// render a line grammar the caller parses back.
    #[test]
    fn a_definition_answer_carries_every_field_the_index_reported() {
        let value = json!({
            "name": "ОбщегоНазначенияКлиентСервер",
            "definitions": [
                {
                    "file": "src/CommonModules/Общий/Ext/Module.bsl",
                    "line": 42,
                    "type": "function",
                    "params": ["Параметр1", "Параметр2 = Неопределено"],
                    "is_export": true,
                    "category": "CommonModule",
                    "object_name": "Общий",
                    "module_type": "Module"
                },
                {"line": 7}
            ]
        });

        let (result, warnings) = super::definition_result(&value).unwrap();

        assert_eq!(result.definitions.len(), 1);
        let definition = &result.definitions[0];
        assert_eq!(definition.line, 42);
        assert_eq!(definition.kind, "function");
        assert_eq!(definition.params.len(), 2);
        assert!(definition.export);
        assert_eq!(definition.object_name.as_deref(), Some("Общий"));
        // A dropped entry is a warning, not a `diagnostic:` line mixed into the
        // report where it reads like a definition.
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("missing file"), "{:?}", warnings);
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
                    .expect("client cancellation must be normalized into an outcome")
                    .outcome;

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
            .outcome
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
        // ADR-0020: the outline is owned by the current-source provider. Reaching
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

        assert!(outcome.outcome.ok);
        // ADR-0023: an empty answer is an empty list, not a sentence.
        assert!(outcome.outcome.stdout.is_none());
        let Some(CodeIntelligenceReadData::Definition(result)) = outcome.data else {
            panic!("code.definition must answer with typed data");
        };
        assert_eq!(result.name, "Найти");
        assert!(result.definitions.is_empty());
        assert!(outcome.outcome.summary.contains("0 definition(s)"));
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
