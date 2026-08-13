use crate::application::AdapterOutcome;
use crate::domain::cancellation::{CancellationToken, CANCELLED_PREFIX};
use crate::domain::code_intelligence::{
    CodeDefinition, CodeDefinitionResult, CodeIntelligenceContext, CodeIntelligenceReadData,
    CodeIntelligenceReadRequest, ProviderDeadline,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::redaction::redactor;
use crate::infrastructure::workspace_index::IndexReadiness;
use crate::infrastructure::workspace_services::{
    WorkspaceRlmOperation, WorkspaceServiceManager, WorkspaceServiceRlmCall,
};
use serde_json::{Map, Value};
use std::path::Path;
use std::time::Duration;

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
    ) -> Result<WorkspaceServiceRlmCall, String>;
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
    ) -> Result<WorkspaceServiceRlmCall, String> {
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
        let operation_name = request.operation_name();
        let operation = operation_for_request(request)?;
        if cancellation.is_cancelled() {
            return Ok(RlmNavigationOutcome::plain(AdapterOutcome::cancelled(
                format!("{operation_name} cancelled before provider work"),
            )));
        }
        let readiness_timeout = deadline.remaining();
        if readiness_timeout.is_zero() {
            return Err(format!(
                "{operation_name} provider deadline exceeded before readiness check"
            ));
        }
        let readiness_result = self.client.readiness(
            &context.workspace,
            &context.source_root.path,
            readiness_timeout,
            cancellation,
        );
        if cancellation.is_cancelled() {
            return Ok(RlmNavigationOutcome::plain(AdapterOutcome::cancelled(
                format!("{operation_name} cancelled after readiness check"),
            )));
        }
        let readiness = match readiness_result {
            Ok(readiness) => readiness,
            Err(error) if error.starts_with(CANCELLED_PREFIX) => {
                return Ok(RlmNavigationOutcome::plain(cancelled_client_outcome(
                    operation_name,
                    &error,
                )));
            }
            Err(error) if error.contains("source revision") => {
                return Ok(RlmNavigationOutcome::plain(index_unavailable_outcome(
                    request,
                    IndexReadiness::Unavailable(error),
                )))
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
        let timeout = deadline.remaining();
        if timeout.is_zero() {
            return Err(format!("{operation_name} provider deadline exceeded"));
        }
        let output = match self.client.call(
            &context.workspace,
            &context.source_root.path,
            operation,
            timeout,
            cancellation,
        ) {
            Ok(WorkspaceServiceRlmCall::Output(output)) => output,
            Ok(WorkspaceServiceRlmCall::Unready(readiness)) => {
                return Ok(RlmNavigationOutcome::plain(index_unavailable_outcome(
                    request, readiness,
                )))
            }
            Err(error) if error.starts_with(CANCELLED_PREFIX) => {
                return Ok(RlmNavigationOutcome::plain(cancelled_client_outcome(
                    operation_name,
                    &error,
                )));
            }
            Err(error) => return Err(error),
        };
        let value: Value = serde_json::from_str(output.result_text.trim()).map_err(|error| {
            format!("{operation_name} received invalid index helper JSON: {error}")
        })?;
        if let Some(error) = value
            .get("error")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            return Err(format!("{operation_name} index helper failed: {error}"));
        }
        let mut outcome = AdapterOutcome::ok(format!(
            "{operation_name} completed through the persistent RLM MCP API"
        ));
        let data;
        match request {
            // ADR-0023: the index already answers with structure, so the tool
            // publishes it instead of rendering it into a line grammar.
            CodeIntelligenceReadRequest::Definition { name, .. } => {
                let (result, warnings) = definition_result(&value, name)?;
                // The transport phrase stays: the issue-89 service test proves
                // reuse of the persistent RLM process through this summary.
                outcome.summary = format!(
                    "{operation_name} found {} definition(s) for {} through the persistent RLM MCP API",
                    result.definitions.len(),
                    result.name
                );
                outcome.warnings.extend(warnings);
                data = Some(CodeIntelligenceReadData::Definition(result));
            }
            CodeIntelligenceReadRequest::Outline { .. } => {
                return Err("code outline is not an index navigation capability".to_string())
            }
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

/// ADR-0020: the index serves definition. The outline is built from the current
/// BSL file, so it has no RLM operation at all and asking for one is a routing
/// defect rather than a runtime condition.
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
        CodeIntelligenceReadRequest::Outline { .. } => {
            return Err(format!(
                "{} is built from the current BSL source and has no RLM operation",
                request.operation_name()
            ))
        }
    })
}

/// Reads the index answer as data. A malformed entry becomes a warning instead
/// of a `diagnostic:` line mixed into the report, so the caller can tell a
/// dropped definition from one that was never there.
///
/// ADR-0023 separates three answers the index can give about one field, and the
/// reader keeps them apart: a reported value is published as it stands, an
/// absent optional field is `null`, and a value of the wrong type is evidence
/// of nothing at all, so the entry is dropped with a warning naming the field
/// rather than published with a plausible substitute. `file` and `line` are
/// required: a definition without them cannot be opened.
///
/// `requested` is the name the caller asked about. It stands in when the index
/// reports no name of its own, so the subject of the answer is always proven —
/// either by the index or by the request — and never an empty string.
/// Cases that are not about the subject name read the answer as if the caller
/// had asked for exactly what the index reported.
#[cfg(test)]
fn definition_result_for_test(
    value: &Value,
) -> Result<(CodeDefinitionResult, Vec<String>), String> {
    let requested = value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("requested")
        .to_string();
    definition_result(value, &requested)
}

fn definition_result(
    value: &Value,
    requested: &str,
) -> Result<(CodeDefinitionResult, Vec<String>), String> {
    let name = match value.get("name") {
        None | Some(Value::Null) => requested.to_string(),
        Some(value) => {
            let reported = value.as_str().ok_or_else(|| {
                "RLM definition response reports a name that is not text".to_string()
            })?;
            // An empty string names nothing, so it is absence, not an answer.
            if reported.is_empty() {
                requested.to_string()
            } else {
                reported.to_string()
            }
        }
    };
    let definitions = value
        .get("definitions")
        .and_then(Value::as_array)
        .ok_or_else(|| "RLM definition response is missing definitions".to_string())?;
    let mut warnings = Vec::new();
    let mut typed = Vec::new();
    for (index, definition) in definitions.iter().enumerate() {
        match read_definition(definition) {
            Ok(definition) => typed.push(definition),
            Err(reason) => warnings.push(format!(
                "ignored malformed RLM definition #{}: {reason}",
                index + 1
            )),
        }
    }
    Ok((
        CodeDefinitionResult {
            name,
            definitions: typed,
        },
        warnings,
    ))
}

fn read_definition(definition: &Value) -> Result<CodeDefinition, String> {
    let file = definition
        .get("file")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "missing file".to_string())?;
    let line = match definition.get("line") {
        None | Some(Value::Null) => return Err("missing line".to_string()),
        Some(value) => value
            .as_u64()
            .ok_or_else(|| "line is not a line number".to_string())?,
    };
    let optional_text = |key: &str| -> Result<Option<String>, String> {
        match definition.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value
                .as_str()
                .map(|value| (!value.is_empty()).then(|| value.to_string()))
                .ok_or_else(|| format!("{key} is not text")),
        }
    };
    let params = match definition.get("params") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let items = value
                .as_array()
                .ok_or_else(|| "params is not a list".to_string())?;
            Some(
                items
                    .iter()
                    .map(|item| {
                        item.as_str()
                            .map(str::to_string)
                            .ok_or_else(|| "params holds a value that is not text".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            )
        }
    };
    let export = match definition.get("is_export") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_bool()
                .ok_or_else(|| "is_export is not a flag".to_string())?,
        ),
    };
    Ok(CodeDefinition {
        file: file.to_string(),
        line,
        kind: optional_text("type")?,
        params,
        export,
        category: optional_text("category")?,
        object_name: optional_text("object_name")?,
        module_type: optional_text("module_type")?,
    })
}
fn index_unavailable_outcome(
    request: &CodeIntelligenceReadRequest,
    readiness: IndexReadiness,
) -> AdapterOutcome {
    let tool_name = request.operation_name();
    let message = readiness_message(readiness);
    if message.starts_with(CANCELLED_PREFIX) {
        return AdapterOutcome::cancelled(
            message
                .strip_prefix(CANCELLED_PREFIX)
                .unwrap_or(&message)
                .trim(),
        );
    }
    let mut outcome = AdapterOutcome::ok(format!(
        "{tool_name} could not use the persistent RLM MCP API"
    ));
    outcome.ok = false;
    outcome.errors.push(message);
    outcome
}

fn cancelled_client_outcome(tool_name: &str, error: &str) -> AdapterOutcome {
    AdapterOutcome::cancelled(format!(
        "{tool_name} {}",
        error.strip_prefix(CANCELLED_PREFIX).unwrap_or(error).trim()
    ))
}

fn readiness_message(readiness: IndexReadiness) -> String {
    match readiness {
        IndexReadiness::Ready { .. } => {
            unreachable!("ready indexes are handled before readiness warnings")
        }
        IndexReadiness::Missing => "index_unavailable: index is missing".to_string(),
        IndexReadiness::Stale { status } => {
            format!(
                "index_unavailable: index is stale: {}",
                redactor(status.trim())
            )
        }
        IndexReadiness::Building => {
            "index_pending: rlm index building; retry once index maintenance completes".to_string()
        }
        IndexReadiness::Failed(error) | IndexReadiness::Unavailable(error)
            if error.starts_with(CANCELLED_PREFIX) =>
        {
            error
        }
        IndexReadiness::Failed(error) | IndexReadiness::Unavailable(error) => {
            format!("index_unavailable: {}", redactor(error.trim()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{operation_for_request, RlmNavigationAdapter, RlmNavigationClient};
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::CodeIntelligenceReadData;
    use crate::domain::code_intelligence::{
        CodeIntelligenceContext, CodeIntelligenceReadRequest, ProviderDeadline,
    };
    use crate::domain::source_roots::ResolvedSourceRoot;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::workspace_index::IndexReadiness;
    use crate::infrastructure::workspace_services::{
        WorkspaceRlmOperation, WorkspaceServiceRlmCall, WorkspaceServiceRlmOutput,
    };
    use serde_json::json;
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    thread_local! {
        static MANUAL_NOW: RefCell<Option<Instant>> = const { RefCell::new(None) };
    }

    fn manual_now() -> Instant {
        MANUAL_NOW.with(|now| now.borrow().expect("manual test clock must be initialized"))
    }

    fn set_manual_now(now: Instant) {
        MANUAL_NOW.with(|current| *current.borrow_mut() = Some(now));
    }

    fn advance_manual_now(duration: Duration) {
        MANUAL_NOW.with(|current| {
            let now = current
                .borrow()
                .expect("manual test clock must be initialized");
            *current.borrow_mut() = Some(now + duration);
        });
    }

    fn secure_temp_root(label: &str) -> PathBuf {
        std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "{label}-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ))
    }

    #[test]
    fn a_definition_keeps_every_field_the_helper_reported() {
        let (result, warnings) = super::definition_result_for_test(&json!({
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
        assert_eq!(definition.kind.as_deref(), Some("function"));
        assert_eq!(definition.export, Some(true));
        assert_eq!(
            definition.params.as_deref(),
            Some(&["Значение".to_string()][..])
        );
        assert_eq!(definition.category.as_deref(), Some("CommonModule"));
        assert_eq!(definition.module_type.as_deref(), Some("Module"));
    }

    #[test]
    fn a_malformed_definition_is_dropped_with_a_warning_not_listed_as_one() {
        let (result, warnings) = super::definition_result_for_test(&json!({
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

    /// ADR-0023: an unproven value is `null`. A definition the index reported
    /// without a kind, parameters or an export flag must not be published as a
    /// method with no parameters that is not exported — that is a subject
    /// claim the index never made.
    #[test]
    fn an_unreported_definition_field_is_null_rather_than_a_fabricated_default() {
        let (result, warnings) = super::definition_result_for_test(&json!({
            "name": "Найти",
            "definitions": [{"file": "CommonModules/X/Module.bsl", "line": 7}]
        }))
        .unwrap();

        assert!(warnings.is_empty(), "{warnings:?}");
        let definition = &result.definitions[0];
        assert_eq!(definition.line, 7);
        assert_eq!(definition.kind, None);
        assert_eq!(definition.params, None);
        assert_eq!(definition.export, None);
    }

    /// The other half of the same rule: an explicit zero, `false` or empty list
    /// is a proven negative and survives untouched.
    #[test]
    fn an_explicit_empty_definition_value_stays_a_proven_answer() {
        let (result, warnings) = super::definition_result_for_test(&json!({
            "name": "Найти",
            "definitions": [{
                "file": "CommonModules/X/Module.bsl",
                "line": 0,
                "type": "procedure",
                "params": [],
                "is_export": false
            }]
        }))
        .unwrap();

        assert!(warnings.is_empty(), "{warnings:?}");
        let definition = &result.definitions[0];
        assert_eq!(definition.line, 0);
        assert_eq!(definition.kind.as_deref(), Some("procedure"));
        assert_eq!(definition.params.as_deref(), Some(&[][..]));
        assert_eq!(definition.export, Some(false));
    }

    /// One row per upstream field: a missing or wrongly typed value never
    /// becomes a plausible answer, and the message names the field.
    #[test]
    fn every_definition_field_fails_closed_on_a_wrong_type() {
        let cases: [(&str, serde_json::Value, &str); 6] = [
            ("file", json!(7), "missing file"),
            ("line", json!("seven"), "line"),
            ("type", json!(7), "type"),
            ("params", json!("Значение"), "params"),
            ("params", json!([7]), "params"),
            ("is_export", json!("yes"), "is_export"),
        ];

        for (field, value, expected) in cases {
            let mut definition = json!({
                "file": "CommonModules/X/Module.bsl",
                "line": 7,
                "type": "function",
                "params": ["Значение"],
                "is_export": true
            });
            definition[field] = value.clone();
            let (result, warnings) = super::definition_result_for_test(&json!({
                "name": "Найти",
                "definitions": [definition]
            }))
            .unwrap();

            assert!(
                result.definitions.is_empty(),
                "{field}={value} must not publish a definition"
            );
            assert_eq!(warnings.len(), 1, "{field}={value}: {warnings:?}");
            assert!(
                warnings[0].contains(expected),
                "{field}={value}: {warnings:?}"
            );
        }
    }

    /// The result's `name` is the subject the answer is about. An index that
    /// reports it as something other than text is malformed, and substituting
    /// an empty subject would publish an answer about nothing.
    #[test]
    fn a_definition_result_name_is_never_fabricated() {
        let requested = "Найти";

        let (result, _) = super::definition_result(
            &json!({"name": "НайтиПоСсылке", "definitions": []}),
            requested,
        )
        .unwrap();
        assert_eq!(result.name, "НайтиПоСсылке", "upstream keeps its answer");

        // Absent upstream name falls back to the proven subject of the request,
        // which the caller supplied, rather than to an empty string.
        let (result, _) = super::definition_result(&json!({"definitions": []}), requested).unwrap();
        assert_eq!(result.name, requested);
        let (result, _) =
            super::definition_result(&json!({"name": null, "definitions": []}), requested).unwrap();
        assert_eq!(result.name, requested);
        // An empty string is no more a subject than a missing field is.
        let (result, _) =
            super::definition_result(&json!({"name": "", "definitions": []}), requested).unwrap();
        assert_eq!(result.name, requested);

        for wrong in [json!(7), json!(["Найти"]), json!({"value": "Найти"})] {
            let error =
                super::definition_result(&json!({"name": wrong, "definitions": []}), requested)
                    .expect_err("a non-text name must fail closed");
            assert!(error.contains("name"), "{wrong}: {error}");
        }
    }

    /// A missing `line` is not a definition anybody can open, so it is dropped
    /// with the same evidence rule that already governs `file`.
    #[test]
    fn a_definition_without_a_line_is_dropped_rather_than_anchored_at_zero() {
        let (result, warnings) = super::definition_result_for_test(&json!({
            "name": "Найти",
            "definitions": [{"file": "CommonModules/X/Module.bsl", "type": "function"}]
        }))
        .unwrap();

        assert!(result.definitions.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("missing line"), "{warnings:?}");
    }

    /// Upstream counts before applying its item limit, so a limited section
    /// still reports an exact total.
    struct RecordingClient {
        operations: Mutex<Vec<WorkspaceRlmOperation>>,
    }

    struct CancelledClient {
        cancel_during_call: bool,
    }

    struct DeadlineRecordingClient {
        timeouts: Mutex<Vec<Duration>>,
    }

    struct CancellingReadinessClient {
        readiness: IndexReadiness,
        elapsed: Duration,
    }

    impl RlmNavigationClient for CancellingReadinessClient {
        fn readiness(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _timeout: Duration,
            cancellation: &CancellationToken,
        ) -> Result<IndexReadiness, String> {
            cancellation.cancel();
            advance_manual_now(self.elapsed);
            Ok(self.readiness.clone())
        }

        fn call(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _operation: WorkspaceRlmOperation,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<WorkspaceServiceRlmCall, String> {
            panic!("cancellation after readiness must stop before the RLM call")
        }
    }

    impl RlmNavigationClient for DeadlineRecordingClient {
        fn readiness(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<IndexReadiness, String> {
            self.timeouts.lock().unwrap().push(timeout);
            advance_manual_now(Duration::from_millis(20));
            Ok(IndexReadiness::Ready {
                db_path: PathBuf::from("/tmp/index.db"),
            })
        }

        fn call(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _operation: WorkspaceRlmOperation,
            timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<WorkspaceServiceRlmCall, String> {
            self.timeouts.lock().unwrap().push(timeout);
            Ok(WorkspaceServiceRlmCall::Output(WorkspaceServiceRlmOutput {
                result_text: json!({
                    "name": "Найти",
                    "definitions": [],
                    "total": 0,
                    "truncated": false
                })
                .to_string(),
                stderr: String::new(),
            }))
        }
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
        ) -> Result<WorkspaceServiceRlmCall, String> {
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

        let (result, warnings) = super::definition_result_for_test(&value).unwrap();

        assert_eq!(result.definitions.len(), 1);
        let definition = &result.definitions[0];
        assert_eq!(definition.line, 42);
        assert_eq!(definition.kind.as_deref(), Some("function"));
        assert_eq!(definition.params.as_ref().map(Vec::len), Some(2));
        assert_eq!(definition.export, Some(true));
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

    #[test]
    fn definition_readiness_and_call_share_one_remaining_deadline() {
        let client = DeadlineRecordingClient {
            timeouts: Mutex::new(Vec::new()),
        };
        let started_at = Instant::now();
        set_manual_now(started_at);

        let outcome = RlmNavigationAdapter::with_client(&client)
            .invoke_resolved_cancellable(
                &definition_request(),
                &unready_index_context(),
                ProviderDeadline::with_clock(started_at + Duration::from_millis(200), manual_now),
                &CancellationToken::new(),
            )
            .unwrap();

        assert!(outcome.outcome.ok);
        let timeouts = client.timeouts.lock().unwrap();
        assert_eq!(timeouts.len(), 2);
        assert!(timeouts[0] > Duration::from_millis(100), "{:?}", timeouts);
        assert!(
            timeouts[1] + Duration::from_millis(10) < timeouts[0],
            "readiness and call must consume one deadline: {:?}",
            timeouts
        );
    }

    #[test]
    fn cancellation_after_readiness_wins_when_the_deadline_expires_at_the_same_time() {
        let started_at = Instant::now();
        set_manual_now(started_at);
        let cancellation = CancellationToken::new();
        let client = CancellingReadinessClient {
            readiness: IndexReadiness::Ready {
                db_path: PathBuf::from("/tmp/index.db"),
            },
            elapsed: Duration::from_millis(201),
        };

        let outcome = RlmNavigationAdapter::with_client(&client)
            .invoke_resolved_cancellable(
                &definition_request(),
                &unready_index_context(),
                ProviderDeadline::with_clock(started_at + Duration::from_millis(200), manual_now),
                &cancellation,
            )
            .expect("cancellation must be normalized before deadline interpretation")
            .outcome;

        assert!(!outcome.ok);
        assert!(
            outcome.summary.starts_with("cancelled:"),
            "{}",
            outcome.summary
        );
    }

    #[test]
    fn cancellation_after_readiness_wins_before_any_readiness_state_is_interpreted() {
        let started_at = Instant::now();
        set_manual_now(started_at);
        let cancellation = CancellationToken::new();
        let client = CancellingReadinessClient {
            readiness: IndexReadiness::Missing,
            elapsed: Duration::ZERO,
        };

        let outcome = RlmNavigationAdapter::with_client(&client)
            .invoke_resolved_cancellable(
                &definition_request(),
                &unready_index_context(),
                ProviderDeadline::with_clock(started_at + Duration::from_millis(200), manual_now),
                &cancellation,
            )
            .expect("cancellation must be normalized before readiness interpretation")
            .outcome;

        assert!(!outcome.ok);
        assert!(
            outcome.summary.starts_with("cancelled:"),
            "{}",
            outcome.summary
        );
        assert!(outcome.warnings.is_empty());
    }

    struct UnreadyClient {
        readiness: IndexReadiness,
    }

    struct UnsupportedRevisionFenceClient;

    struct PostExecutionUnreadyClient;

    impl RlmNavigationClient for UnsupportedRevisionFenceClient {
        fn readiness(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<IndexReadiness, String> {
            Err("source revision fence is unsupported; freshness cannot be proven".to_string())
        }

        fn call(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _operation: WorkspaceRlmOperation,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<WorkspaceServiceRlmCall, String> {
            panic!("an unavailable revision fence must stop before the RLM call")
        }
    }

    impl RlmNavigationClient for PostExecutionUnreadyClient {
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
            _operation: WorkspaceRlmOperation,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<WorkspaceServiceRlmCall, String> {
            Ok(WorkspaceServiceRlmCall::Unready(IndexReadiness::Stale {
                status: "source generation changed".to_string(),
            }))
        }
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
        ) -> Result<WorkspaceServiceRlmCall, String> {
            panic!("a not-ready index must never reach the RLM call")
        }
    }

    fn unready_index_outcome(
        request: &CodeIntelligenceReadRequest,
        readiness: IndexReadiness,
    ) -> super::RlmNavigationOutcome {
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
        let source_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/unica_mcp_script_parity/meta-validate-language-aware");
        CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: PathBuf::from("/workspace"),
                workspace_root: PathBuf::from("/workspace"),
                cache_root: PathBuf::from("/cache"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: source_root,
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
    fn definition_readiness_matrix_never_reports_false_typed_success() {
        let request = definition_request();
        for (readiness, prefix, retryable) in [
            (IndexReadiness::Building, "index_pending:", true),
            (IndexReadiness::Missing, "index_unavailable:", false),
            (
                IndexReadiness::Stale {
                    status: "source generation changed".to_string(),
                },
                "index_unavailable:",
                false,
            ),
            (
                IndexReadiness::Failed("Pwd=secret build failed".to_string()),
                "index_unavailable:",
                false,
            ),
            (
                IndexReadiness::Unavailable("service absent".to_string()),
                "index_unavailable:",
                false,
            ),
        ] {
            let result = unready_index_outcome(&request, readiness);
            let outcome = result.outcome;
            assert!(!outcome.ok, "{prefix}: {outcome:?}");
            assert!(outcome.warnings.is_empty(), "{prefix}: {outcome:?}");
            assert_eq!(outcome.errors.len(), 1, "{prefix}: {outcome:?}");
            assert!(outcome.errors[0].starts_with(prefix), "{outcome:?}");
            assert_eq!(
                outcome.errors[0].contains("retry"),
                retryable,
                "{outcome:?}"
            );
            assert!(!outcome.errors[0].contains("secret"), "{outcome:?}");
            assert!(outcome.stdout.is_none(), "{outcome:?}");
            assert!(result.data.is_none());
        }
    }

    #[test]
    fn unsupported_revision_fence_is_a_typed_unavailable_read_not_a_transport_error() {
        let result = RlmNavigationAdapter::with_client(&UnsupportedRevisionFenceClient)
            .invoke_resolved_cancellable(
                &definition_request(),
                &unready_index_context(),
                ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
                &CancellationToken::new(),
            )
            .expect("provider availability belongs in the typed tool result");

        assert!(!result.outcome.ok);
        assert_eq!(result.outcome.errors.len(), 1);
        assert!(
            result.outcome.errors[0].starts_with("index_unavailable:"),
            "{:?}",
            result.outcome.errors
        );
        assert!(result.data.is_none());
    }

    #[test]
    fn post_execution_stale_generation_uses_the_same_readiness_mapper() {
        let result = RlmNavigationAdapter::with_client(&PostExecutionUnreadyClient)
            .invoke_resolved_cancellable(
                &definition_request(),
                &unready_index_context(),
                ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert!(!result.outcome.ok);
        assert_eq!(result.outcome.errors.len(), 1);
        assert!(result.outcome.errors[0].starts_with("index_unavailable:"));
        assert!(result.outcome.stdout.is_none());
        assert!(result.outcome.artifacts.is_empty());
        assert!(result.data.is_none());
    }

    #[test]
    fn a_cancelled_readiness_is_reported_as_cancellation_not_as_an_index_failure() {
        let outcome = unready_index_outcome(
            &definition_request(),
            IndexReadiness::Unavailable("cancelled: readiness stopped".to_string()),
        )
        .outcome;

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
        ) -> Result<WorkspaceServiceRlmCall, String> {
            self.operations.lock().unwrap().push(operation);
            Ok(WorkspaceServiceRlmCall::Output(WorkspaceServiceRlmOutput {
                result_text: json!({
                    "name": "Найти",
                    "definitions": [],
                    "total": 0,
                    "truncated": false
                })
                .to_string(),
                stderr: String::new(),
            }))
        }
    }

    #[test]
    fn adapter_routes_definition_through_rlm_client() {
        let root = secure_temp_root("unica-rlm-navigation");
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
