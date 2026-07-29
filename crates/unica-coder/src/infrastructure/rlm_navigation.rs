use crate::application::AdapterOutcome;
use crate::domain::cancellation::{CancellationToken, CANCELLED_PREFIX};
use crate::domain::code_intelligence::{
    CodeIntelligenceContext, CodeIntelligenceReadRequest, ProviderDeadline,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::source_roots::normalize_path_identity;
use crate::infrastructure::workspace_index::{mark_bsl_index_stale, IndexReadiness};
use crate::infrastructure::workspace_services::{
    WorkspaceRlmOperation, WorkspaceServiceManager, WorkspaceServiceRlmOutput,
};
use bsl_syntax::ast::{AstNode, FunctionDef, ProcedureDef};
use bsl_syntax::SyntaxKind;
use serde_json::{Map, Value};
use std::fs;
use std::path::Path;
use std::time::Duration;

const RLM_NAVIGATION_TIMEOUT: Duration = Duration::from_secs(45);

/// Stable machine-readable markers that let a caller tell a retryable pending
/// index from a permanent one without parsing prose.
const INDEX_PENDING_PREFIX: &str = "index_pending:";
const INDEX_UNAVAILABLE_PREFIX: &str = "index_unavailable:";

/// How a read tool renders a not-ready index. `Warn` keeps the call successful
/// with a warning because the tool still answers something useful; `Fail`
/// reports a typed failure because without the index the tool has nothing to
/// return and a success would tell the caller the outline was empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnreadyIndexPolicy {
    Warn,
    Fail,
}

fn unready_index_policy(request: &CodeIntelligenceReadRequest) -> UnreadyIndexPolicy {
    match request {
        CodeIntelligenceReadRequest::Outline { .. } => UnreadyIndexPolicy::Fail,
        CodeIntelligenceReadRequest::Definition { .. }
        | CodeIntelligenceReadRequest::ObjectProfile { .. } => UnreadyIndexPolicy::Warn,
    }
}

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
            other => return Ok(index_unavailable_outcome(request, other)),
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
        if let CodeIntelligenceReadRequest::Outline {
            path,
            include_methods,
        } = request
        {
            // RLM's global `index info` content check is sampled, while an
            // index-backed outline otherwise trusts its SQLite snapshot
            // completely. Prove this requested module at the public boundary
            // before reporting either the outline or a fresh cache.
            if let Err(error) = validate_indexed_outline(&value, path, *include_methods, context) {
                let message = format!("requested module `{path}`: {error}");
                let mut outcome = AdapterOutcome {
                    ok: false,
                    summary: "unica.code.outline detected a stale RLM index snapshot".to_string(),
                    changes: Vec::new(),
                    warnings: Vec::new(),
                    errors: vec![format!(
                        "{INDEX_UNAVAILABLE_PREFIX} rlm index stale: {message}"
                    )],
                    artifacts: vec![path.clone()],
                    stdout: None,
                    stderr: None,
                    command: None,
                };
                if let Err(status_error) =
                    mark_bsl_index_stale(&context.workspace, &context.source_root.path, &message)
                {
                    outcome.warnings.push(format!(
                        "failed to persist stale RLM index state: {status_error}"
                    ));
                }
                return Ok(outcome);
            }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum OutlineMethodKind {
    Procedure,
    Function,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct OutlineMethodFingerprint {
    line: usize,
    end_line: usize,
    kind: OutlineMethodKind,
    name: String,
    is_export: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct OutlineRegionFingerprint {
    line: usize,
    end_line: Option<usize>,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OutlineFingerprint {
    methods: Vec<OutlineMethodFingerprint>,
    regions: Vec<OutlineRegionFingerprint>,
    method_count: usize,
    export_count: usize,
}

fn validate_indexed_outline(
    value: &Value,
    request_path: &str,
    include_methods: bool,
    context: &CodeIntelligenceContext,
) -> Result<(), String> {
    match value.pointer("/_meta/index_used").and_then(Value::as_bool) {
        Some(false) => return Ok(()),
        Some(true) => {}
        None => {
            return Err(
                "RLM outline response did not identify whether the index was used".to_string(),
            )
        }
    }

    let reported_path = required_value_string(value, "path", "RLM outline")?;
    if reported_path != request_path {
        return Err(format!(
            "RLM returned path `{reported_path}` instead of the requested path"
        ));
    }

    let source_root = normalize_path_identity(&context.source_root.path)
        .map_err(|error| format!("could not validate source root: {error}"))?;
    let module_path = normalize_path_identity(&source_root.join(Path::new(request_path)))
        .map_err(|error| format!("could not validate current module path: {error}"))?;
    if !module_path.starts_with(&source_root) {
        return Err("current module resolves outside the selected source root".to_string());
    }
    let text = fs::read_to_string(&module_path)
        .map_err(|error| format!("could not read the current module: {error}"))?;
    let live = live_outline_fingerprint(&text)?;
    let indexed = indexed_outline_fingerprint(value, include_methods)?;

    if live.regions != indexed.regions {
        return Err(format!(
            "indexed regions differ from the current filesystem structure (indexed {}, current {})",
            indexed.regions.len(),
            live.regions.len()
        ));
    }
    if live.method_count != indexed.method_count || live.export_count != indexed.export_count {
        return Err(format!(
            "indexed method totals differ from the current filesystem structure (indexed methods/exports {}/{}, current {}/{})",
            indexed.method_count,
            indexed.export_count,
            live.method_count,
            live.export_count
        ));
    }
    if include_methods && live.methods != indexed.methods {
        return Err(
            "indexed method declarations differ from the current filesystem structure".to_string(),
        );
    }
    Ok(())
}

fn live_outline_fingerprint(text: &str) -> Result<OutlineFingerprint, String> {
    if text.len() > u32::MAX as usize {
        return Err("current BSL module is too large for structural validation".to_string());
    }
    let parsed = bsl_parser::parse(text);
    if !parsed.errors().is_empty() {
        return Err(format!(
            "current BSL module cannot be structurally validated because the parser reported {} diagnostic(s)",
            parsed.errors().len()
        ));
    }
    let root = parsed.syntax_node();
    let mut methods = Vec::new();
    for node in root.descendants() {
        if let Some(procedure) = ProcedureDef::cast(node.clone()) {
            methods.push(method_fingerprint(
                text,
                procedure.syntax(),
                procedure
                    .name_or_keyword()
                    .map(|token| token.text().to_string()),
                procedure.export_keyword().is_some(),
                OutlineMethodKind::Procedure,
                SyntaxKind::KW_PROCEDURE,
                SyntaxKind::KW_END_PROCEDURE,
            )?);
        } else if let Some(function) = FunctionDef::cast(node) {
            methods.push(method_fingerprint(
                text,
                function.syntax(),
                function
                    .name_or_keyword()
                    .map(|token| token.text().to_string()),
                function.export_keyword().is_some(),
                OutlineMethodKind::Function,
                SyntaxKind::KW_FUNCTION,
                SyntaxKind::KW_END_FUNCTION,
            )?);
        }
    }
    methods.sort();
    let export_count = methods.iter().filter(|method| method.is_export).count();
    let method_count = methods.len();
    Ok(OutlineFingerprint {
        methods,
        regions: live_region_fingerprints(text),
        method_count,
        export_count,
    })
}

#[allow(clippy::too_many_arguments)]
fn method_fingerprint(
    text: &str,
    syntax: &bsl_syntax::SyntaxNode,
    name: Option<String>,
    is_export: bool,
    kind: OutlineMethodKind,
    start_kind: SyntaxKind,
    end_kind: SyntaxKind,
) -> Result<OutlineMethodFingerprint, String> {
    let name = name.ok_or_else(|| "current BSL method is missing a name".to_string())?;
    let mut tokens = syntax
        .descendants_with_tokens()
        .filter_map(|element| element.into_token());
    let start = tokens
        .find(|token| token.kind() == start_kind)
        .ok_or_else(|| format!("current BSL method `{name}` is missing its opening keyword"))?;
    let end = syntax
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind() == end_kind)
        .ok_or_else(|| format!("current BSL method `{name}` is missing its closing keyword"))?;
    Ok(OutlineMethodFingerprint {
        line: line_number(text, usize::from(start.text_range().start())),
        end_line: line_number(text, usize::from(end.text_range().start())),
        kind,
        name,
        is_export,
    })
}

fn live_region_fingerprints(text: &str) -> Vec<OutlineRegionFingerprint> {
    let mut regions = Vec::new();
    let mut open = Vec::new();
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    for (offset, line) in normalized.lines().enumerate() {
        let line_number = offset + 1;
        let trimmed = line
            .trim()
            .strip_prefix('\u{feff}')
            .unwrap_or_else(|| line.trim())
            .trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        if let Some(name) = region_start_name(trimmed) {
            regions.push(OutlineRegionFingerprint {
                line: line_number,
                end_line: None,
                name,
            });
            open.push(regions.len() - 1);
        } else if is_region_end(trimmed) {
            if let Some(index) = open.pop() {
                regions[index].end_line = Some(line_number);
            }
        }
    }
    regions.sort();
    regions
}

fn region_start_name(line: &str) -> Option<String> {
    let lowercase = line.to_lowercase();
    for keyword in ["#область", "#region"] {
        let Some(rest) = lowercase.strip_prefix(keyword) else {
            continue;
        };
        if !rest.chars().next().is_some_and(char::is_whitespace) {
            continue;
        }
        let name = line[keyword.len()..].trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    None
}

fn is_region_end(line: &str) -> bool {
    let lowercase = line.to_lowercase();
    ["#конецобласти", "#endregion"].iter().any(|keyword| {
        lowercase.strip_prefix(keyword).is_some_and(|rest| {
            rest.chars()
                .next()
                .is_none_or(|next| !next.is_alphanumeric() && next != '_')
        })
    })
}

fn indexed_outline_fingerprint(
    value: &Value,
    include_methods: bool,
) -> Result<OutlineFingerprint, String> {
    let totals = value
        .get("totals")
        .and_then(Value::as_object)
        .ok_or_else(|| "RLM outline response is missing totals".to_string())?;
    let method_count = json_usize(totals.get("methods"), "totals.methods")?;
    let export_count = json_usize(totals.get("exports"), "totals.exports")?;
    let region_count = json_usize(totals.get("regions"), "totals.regions")?;
    let mut methods = Vec::new();
    let mut regions = Vec::new();
    let outline = value
        .get("outline")
        .and_then(Value::as_array)
        .ok_or_else(|| "RLM outline response is missing outline".to_string())?;
    collect_indexed_outline(outline, include_methods, &mut methods, &mut regions)?;
    if include_methods {
        let orphan_methods = value
            .get("orphan_methods")
            .and_then(Value::as_array)
            .ok_or_else(|| "RLM outline response is missing orphan_methods".to_string())?;
        collect_indexed_methods(orphan_methods, &mut methods)?;
    }
    methods.sort();
    regions.sort();
    if regions.len() != region_count {
        return Err(format!(
            "RLM outline region total is {}, but the response contains {}",
            region_count,
            regions.len()
        ));
    }
    if include_methods && methods.len() != method_count {
        return Err(format!(
            "RLM outline method total is {}, but the response contains {}",
            method_count,
            methods.len()
        ));
    }
    Ok(OutlineFingerprint {
        methods,
        regions,
        method_count,
        export_count,
    })
}

fn collect_indexed_outline(
    nodes: &[Value],
    include_methods: bool,
    methods: &mut Vec<OutlineMethodFingerprint>,
    regions: &mut Vec<OutlineRegionFingerprint>,
) -> Result<(), String> {
    for node in nodes {
        regions.push(OutlineRegionFingerprint {
            line: json_usize(node.get("line"), "region.line")?,
            end_line: json_optional_usize(node.get("end_line"), "region.end_line")?,
            name: required_value_string(node, "region", "RLM outline region")?.to_string(),
        });
        if include_methods {
            let region_methods = node
                .get("methods")
                .and_then(Value::as_array)
                .ok_or_else(|| "RLM outline region is missing methods".to_string())?;
            collect_indexed_methods(region_methods, methods)?;
        }
        let children = node
            .get("children")
            .and_then(Value::as_array)
            .ok_or_else(|| "RLM outline region is missing children".to_string())?;
        collect_indexed_outline(children, include_methods, methods, regions)?;
    }
    Ok(())
}

fn collect_indexed_methods(
    values: &[Value],
    methods: &mut Vec<OutlineMethodFingerprint>,
) -> Result<(), String> {
    for value in values {
        let kind_text = required_value_string(value, "type", "RLM outline method")?;
        let normalized_kind = kind_text.to_lowercase();
        let kind = if normalized_kind.starts_with("процедур")
            || normalized_kind.starts_with("procedure")
        {
            OutlineMethodKind::Procedure
        } else if normalized_kind.starts_with("функц") || normalized_kind.starts_with("function")
        {
            OutlineMethodKind::Function
        } else {
            return Err(format!(
                "RLM outline method has unsupported type `{kind_text}`"
            ));
        };
        methods.push(OutlineMethodFingerprint {
            line: json_usize(value.get("line"), "method.line")?,
            end_line: json_usize(value.get("end_line"), "method.end_line")?,
            kind,
            name: required_value_string(value, "name", "RLM outline method")?.to_string(),
            is_export: value
                .get("is_export")
                .and_then(Value::as_bool)
                .ok_or_else(|| "RLM outline method is missing is_export".to_string())?,
        });
    }
    Ok(())
}

fn json_usize(value: Option<&Value>, field: &str) -> Result<usize, String> {
    value
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| format!("RLM outline response has invalid {field}"))
}

fn json_optional_usize(value: Option<&Value>, field: &str) -> Result<Option<usize>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => json_usize(Some(value), field).map(Some),
    }
}

fn line_number(text: &str, byte_offset: usize) -> usize {
    let bytes = &text.as_bytes()[..byte_offset];
    bytes
        .iter()
        .enumerate()
        .filter(|(index, byte)| {
            **byte == b'\n' || (**byte == b'\r' && bytes.get(index + 1) != Some(&b'\n'))
        })
        .count()
        + 1
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

fn index_unavailable_outcome(
    request: &CodeIntelligenceReadRequest,
    readiness: IndexReadiness,
) -> AdapterOutcome {
    let tool_name = request.tool_name();
    let pending = matches!(readiness, IndexReadiness::Building);
    let warning = readiness_warning(readiness);
    if warning.starts_with(CANCELLED_PREFIX) {
        return AdapterOutcome::cancelled(
            warning
                .strip_prefix(CANCELLED_PREFIX)
                .unwrap_or(&warning)
                .trim(),
        );
    }
    if unready_index_policy(request) == UnreadyIndexPolicy::Warn {
        let mut outcome = AdapterOutcome::ok(format!(
            "{tool_name} could not use the persistent RLM MCP API"
        ));
        outcome.warnings.push(warning);
        return outcome;
    }
    let (summary, error) = if pending {
        (
            "pending RLM index build",
            format!("{INDEX_PENDING_PREFIX} {warning}"),
        )
    } else {
        (
            "could not read RLM index",
            format!(
                "{INDEX_UNAVAILABLE_PREFIX} {}",
                warning
                    .strip_prefix("rlm index unavailable: ")
                    .unwrap_or(&warning)
            ),
        )
    };
    index_failure_outcome(tool_name, summary, error)
}

fn index_failure_outcome(tool_name: &str, summary: &str, error: String) -> AdapterOutcome {
    AdapterOutcome {
        ok: false,
        summary: format!("{tool_name} {summary}"),
        changes: Vec::new(),
        warnings: Vec::new(),
        errors: vec![error],
        artifacts: Vec::new(),
        stdout: None,
        stderr: None,
        command: None,
    }
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
        validate_indexed_outline, RlmNavigationAdapter, RlmNavigationClient,
    };
    use crate::application::AdapterOutcome;
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::{
        CodeIntelligenceContext, CodeIntelligenceReadRequest, ProviderDeadline,
    };
    use crate::domain::source_roots::ResolvedSourceRoot;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::workspace_index::{
        bsl_index_is_ready, read_bsl_index_status, status_path, IndexReadiness,
    };
    use crate::infrastructure::workspace_services::{
        WorkspaceRlmOperation, WorkspaceServiceRlmOutput,
    };
    use serde_json::json;
    use std::fs;
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

    struct IndexedOutlineClient;

    impl RlmNavigationClient for IndexedOutlineClient {
        fn readiness(
            &self,
            context: &WorkspaceContext,
            _source_root: &Path,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<IndexReadiness, String> {
            Ok(IndexReadiness::Ready {
                db_path: context.cache_root.join("rlm-tools-bsl/bsl_index.db"),
            })
        }

        fn call(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _operation: WorkspaceRlmOperation,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<WorkspaceServiceRlmOutput, String> {
            Ok(WorkspaceServiceRlmOutput {
                result_text: json!({
                    "path": "CommonModules/X/Ext/Module.bsl",
                    "category": "CommonModules",
                    "object_name": "X",
                    "module_type": "Module",
                    "totals": {
                        "methods": 1,
                        "exports": 0,
                        "regions": 0,
                        "loc": 2
                    },
                    "outline": [],
                    "orphan_methods": [{
                        "name": "Old",
                        "type": "Procedure",
                        "is_export": false,
                        "line": 1,
                        "end_line": 2
                    }],
                    "_meta": {
                        "index_used": true,
                        "fallback_reason": null,
                        "resolved_from_name": false
                    }
                })
                .to_string(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn indexed_outline_matching_current_module_is_accepted() {
        let root = std::env::temp_dir().join(format!(
            "unica-rlm-matching-outline-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let source_root = root.join("src");
        let module = source_root.join("CommonModules/X/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(
            &module,
            "\u{feff}#Region API\rProcedure Current()\rEndProcedure\r#EndRegion\r",
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
                path: source_root,
            },
        );
        let outline = json!({
            "path": "CommonModules/X/Ext/Module.bsl",
            "totals": {
                "methods": 1,
                "exports": 0,
                "regions": 1,
                "loc": 2
            },
            "outline": [{
                "region": "API",
                "line": 1,
                "end_line": 4,
                "totals": {"methods": 1, "exports": 0},
                "children": [],
                "methods": [{
                    "name": "Current",
                    "type": "Procedure",
                    "is_export": false,
                    "line": 2,
                    "end_line": 3,
                    "loc": 2
                }]
            }],
            "orphan_methods": [],
            "_meta": {"index_used": true}
        });

        assert_eq!(
            validate_indexed_outline(&outline, "CommonModules/X/Ext/Module.bsl", true, &context),
            Ok(())
        );
        let compact_outline = json!({
            "path": "CommonModules/X/Ext/Module.bsl",
            "totals": {
                "methods": 1,
                "exports": 0,
                "regions": 1,
                "loc": 2
            },
            "outline": [{
                "region": "API",
                "line": 1,
                "end_line": 4,
                "totals": {"methods": 1, "exports": 0},
                "children": []
            }],
            "_meta": {"index_used": true}
        });
        assert_eq!(
            validate_indexed_outline(
                &compact_outline,
                "CommonModules/X/Ext/Module.bsl",
                false,
                &context
            ),
            Ok(())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn outline_rejects_index_snapshot_that_disagrees_with_current_module() {
        let root = std::env::temp_dir().join(format!(
            "unica-rlm-stale-outline-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let source_root = root.join("src");
        let module = source_root.join("CommonModules/X/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(
            &module,
            "Procedure Old()\nEndProcedure\nProcedure Current()\nEndProcedure\n",
        )
        .unwrap();
        let workspace = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        };
        fs::create_dir_all(status_path(&workspace).parent().unwrap()).unwrap();
        fs::write(
            status_path(&workspace),
            format!(
                "{{\"status\":\"ready\",\"source_root\":{},\"db_path\":{},\"message\":null,\"updated_at\":1}}\n",
                serde_json::to_string(&source_root.display().to_string()).unwrap(),
                serde_json::to_string(
                    &workspace
                        .cache_root
                        .join("rlm-tools-bsl/bsl_index.db")
                        .display()
                        .to_string()
                )
                .unwrap()
            ),
        )
        .unwrap();
        let context = CodeIntelligenceContext::new(
            workspace.clone(),
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: source_root,
            },
        );

        let outcome = RlmNavigationAdapter::with_client(&IndexedOutlineClient)
            .invoke_resolved_cancellable(
                &outline_request(),
                &context,
                ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert!(!outcome.ok);
        assert_eq!(
            outcome.summary,
            "unica.code.outline detected a stale RLM index snapshot"
        );
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.starts_with("index_unavailable: rlm index stale:")));
        assert!(outcome.stdout.is_none());
        let status = read_bsl_index_status(&workspace).unwrap();
        assert_eq!(status.status, "stale");
        assert!(!bsl_index_is_ready(&workspace));
        assert!(status
            .message
            .as_deref()
            .is_some_and(|message| message.contains("CommonModules/X/Ext/Module.bsl")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn outline_reports_building_index_as_retryable_failure() {
        let outcome = unready_index_outcome(&outline_request(), IndexReadiness::Building);

        assert!(!outcome.ok);
        assert_eq!(
            outcome.summary,
            "unica.code.outline pending RLM index build"
        );
        assert_eq!(outcome.errors, vec!["index_pending: rlm index building"]);
        assert!(outcome.warnings.is_empty());
        assert!(outcome.stdout.is_none());
    }

    #[test]
    fn outline_reports_unready_index_as_typed_failure() {
        for (readiness, expected) in [
            (
                IndexReadiness::Missing,
                "index_unavailable: index is missing",
            ),
            (
                IndexReadiness::Failed("helper crashed".to_string()),
                "index_unavailable: helper crashed",
            ),
            (
                IndexReadiness::Stale {
                    status: "dump is newer".to_string(),
                },
                "index_unavailable: rlm index stale: dump is newer",
            ),
        ] {
            let outcome = unready_index_outcome(&outline_request(), readiness);

            assert!(!outcome.ok, "{expected}");
            assert_eq!(
                outcome.summary,
                "unica.code.outline could not read RLM index"
            );
            assert_eq!(outcome.errors, vec![expected.to_string()]);
            assert!(outcome.stdout.is_none(), "{expected}");
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
    fn outline_reports_a_cancelled_readiness_as_cancellation_not_index_failure() {
        let outcome = unready_index_outcome(
            &outline_request(),
            IndexReadiness::Unavailable("cancelled: readiness stopped".to_string()),
        );

        assert!(!outcome.ok);
        assert!(outcome.summary.contains("cancelled"), "{}", outcome.summary);
        assert!(
            !outcome
                .errors
                .iter()
                .any(|error| error.starts_with(super::INDEX_UNAVAILABLE_PREFIX)),
            "{:?}",
            outcome.errors
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
