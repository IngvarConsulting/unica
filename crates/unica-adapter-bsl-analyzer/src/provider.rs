use serde_json::{json, Map, Value};
use std::path::Path;
use std::time::Duration;
use unica_analysis_services::{WorkspaceServiceBslCall, WorkspaceServiceManager};
use unica_code_intelligence::current_outline::render_current_source_outline;
use unica_code_intelligence::model::*;
use unica_code_intelligence::provider_sections::*;
use unica_code_intelligence::source_locations::{SearchLocationProjector, SearchSourceBindings};
use unica_process::mcp_session::McpToolOutput;
use unica_work_control::cancellation::{cancelled_error, CancellationToken, CANCELLED_PREFIX};
use unica_work_control::provider_deadline::ProviderDeadline;
/// ADR-0020: the outline is built from the current BSL file by the pinned
/// `bsl-parser`, so it belongs to this provider and not to the index.
const BSL_ANALYZER_CAPABILITIES: &[ProviderCapability] = &[
    ProviderCapability::Search,
    ProviderCapability::Outline,
    ProviderCapability::CallGraph,
];

/// Канал к MCP анализатора.
///
/// Имя инструмента — параметр, а не часть имени метода: профиль `workspace`
/// публикует девять инструментов, и поиск с графом идут одним маршрутом.
/// Метод на каждый инструмент отличался бы от соседа одним литералом, а шов
/// для подмены в проверках нужен один — иначе запрос графа не перехватить, не
/// поднимая движок.
pub trait BslToolClient: Send + Sync {
    fn call(
        &self,
        tool: &'static str,
        context: &CodeIntelligenceContext,
        arguments: Value,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<McpToolOutput, String>;
}

pub struct WorkspaceBslToolClient {
    pub environment:
        fn() -> std::sync::Arc<dyn unica_analysis_services::backend::ServiceEnvironment>,
}

impl BslToolClient for WorkspaceBslToolClient {
    fn call(
        &self,
        tool: &'static str,
        context: &CodeIntelligenceContext,
        arguments: Value,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<McpToolOutput, String> {
        WorkspaceServiceManager::new((self.environment)()).call_bsl_mcp_cancellable_with_budget(
            &context.workspace,
            &context.source_root.path,
            WorkspaceServiceBslCall::new(tool, arguments, timeout, timeout),
            cancellation,
        )
    }
}

pub struct BslAnalyzerProvider<'a> {
    bindings: SearchSourceBindings,
    client: &'a (dyn BslToolClient + Send + Sync),
}

impl<'a> BslAnalyzerProvider<'a> {
    pub fn new(
        client: &'a (dyn BslToolClient + Send + Sync),
        bindings: SearchSourceBindings,
    ) -> Self {
        Self { client, bindings }
    }

    /// Спросить у анализатора рёбра вызовов одного направления.
    ///
    /// Канал тот же, которым идёт поиск: инструмент `graph` живёт в том же
    /// профиле MCP, и второго маршрута под граф не заводится.
    fn read_call_graph(
        &self,
        direction: CallGraphDirection,
        id: &str,
        limit: usize,
        context: &CodeIntelligenceContext,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<ProviderReadOutcome, String> {
        let arguments = json!({
            "action": direction.analyzer_action(),
            "id": id,
            "max_nodes": limit,
            "edge_kinds": ["call"],
        });
        let mut outcome = ProviderReadOutcome {
            provider: ProviderId::BslAnalyzer.identity(),
            ok: true,
            summary: String::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
            data: None,
        };
        let output = match if cancellation.is_cancelled() {
            Err(cancelled_error("bsl-analyzer graph stopped before request"))
        } else {
            deadline
                .finite_remaining()
                .map_err(|error| error.to_string())
                .and_then(|timeout| {
                    self.client
                        .call("graph", context, arguments, timeout, cancellation)
                })
        } {
            Ok(output) => output,
            // Движок не поставлен или не ответил. Это названное состояние, а не
            // нулевой счёт: читателю важно знать, что графа нет, а не что
            // вызовов нет.
            Err(error) => {
                outcome.ok = false;
                outcome.summary = format!(
                    "{} could not reach the call graph",
                    ProviderId::BslAnalyzer.as_str()
                );
                outcome.errors = vec![error];
                outcome.data = Some(CodeIntelligenceReadData::CallGraph(CallGraphResult {
                    state: CallGraphState::Unavailable,
                    total: None,
                    edges: Vec::new(),
                    revision: None,
                    stale: None,
                    complete: false,
                }));
                return Ok(outcome);
            }
        };
        let value: Value = serde_json::from_str(output.result_text.trim())
            .map_err(|error| format!("invalid call graph answer from the analyzer: {error}"))?;
        if value.get("status").and_then(Value::as_str) != Some("loading")
            && value["result"]["error"].is_null()
            && value["result"]["root"]["id"].as_str() != Some(id)
        {
            return Err("call graph provider answered for another method".to_string());
        }
        match parse_call_graph_answer(&value, direction) {
            Ok(result) => {
                outcome.summary = match result.total {
                    Some(total) => format!(
                        "{} answered {total} {} edge(s)",
                        ProviderId::BslAnalyzer.as_str(),
                        direction.analyzer_action()
                    ),
                    None => format!(
                        "{} is still indexing the call graph",
                        ProviderId::BslAnalyzer.as_str()
                    ),
                };
                outcome.data = Some(CodeIntelligenceReadData::CallGraph(result));
            }
            // Неизвестный узел и прочие названные отказы анализатора остаются
            // отказами: подставить за них пустую страницу значило бы сказать
            // «вызовов нет» там, где метода нет вовсе.
            Err(error) => {
                outcome.ok = false;
                outcome.summary = format!(
                    "{} refused the call graph request",
                    ProviderId::BslAnalyzer.as_str()
                );
                outcome.errors = vec![error];
            }
        }
        Ok(outcome)
    }
}

impl CodeIntelligenceProvider for BslAnalyzerProvider<'_> {
    fn identity(&self) -> unica_code_intelligence::model::ProviderIdentity {
        ProviderId::BslAnalyzer.identity()
    }

    fn capabilities(&self) -> &[ProviderCapability] {
        BSL_ANALYZER_CAPABILITIES
    }

    fn read(
        &self,
        request: &CodeIntelligenceReadRequest,
        context: &CodeIntelligenceContext,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<ProviderReadOutcome, String> {
        if let CodeIntelligenceReadRequest::CallGraph {
            id,
            direction,
            limit,
        } = request
        {
            return self.read_call_graph(*direction, id, *limit, context, deadline, cancellation);
        }
        let CodeIntelligenceReadRequest::Outline {
            path,
            include_methods,
        } = request
        else {
            return Err(format!(
                "provider {} does not implement {:?}",
                ProviderId::BslAnalyzer.as_str(),
                request.capability()
            ));
        };
        let tool_name = request.operation_name();
        let mut outcome = ProviderReadOutcome {
            provider: ProviderId::BslAnalyzer.identity(),
            ok: true,
            summary: format!("{tool_name} completed from the current BSL source"),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
            data: None,
        };
        match render_current_source_outline(path, *include_methods, context, deadline, cancellation)
        {
            Ok((result, module)) => {
                // The successful renderer returns the identity of the same file
                // it read. Resolving the argument independently here could
                // claim a missing path or race a symlink change.
                outcome.artifacts = vec![module.display().to_string()];
                outcome.data = Some(CodeIntelligenceReadData::Outline(result));
            }
            Err(error) if error.starts_with(CANCELLED_PREFIX) => {
                // Same shape as `AdapterOutcome::cancelled`: the prefixed error
                // is the summary, and a stopped call claims no artifacts.
                outcome.ok = false;
                outcome.summary = error.clone();
                outcome.errors = vec![error];
                outcome.artifacts = Vec::new();
            }
            Err(error) => {
                outcome.ok = false;
                outcome.summary = format!("{tool_name} could not outline the current module");
                outcome.errors = vec![error];
            }
        }
        Ok(outcome)
    }

    fn search(
        &self,
        request: &SearchRequest,
        context: &CodeIntelligenceContext,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> ProviderSearchSection {
        if cancellation.is_cancelled() {
            return failed_section(
                ProviderId::BslAnalyzer,
                cancelled_error("bsl-analyzer search stopped before request"),
            );
        }
        if has_targeted_search_scope(context) {
            return unavailable_targeted_scope_section(ProviderId::BslAnalyzer);
        }
        let timeout = match deadline.finite_remaining() {
            Ok(timeout) => timeout,
            Err(error) => return unavailable_section(ProviderId::BslAnalyzer, error.to_string()),
        };
        if timeout.is_zero() {
            return failed_section(
                ProviderId::BslAnalyzer,
                "bsl-analyzer provider deadline exceeded".to_string(),
            );
        }
        let arguments = json!({
            "action": "search_code",
            "query": request.query,
            "limit": request.limit,
        });
        match self
            .client
            .call("search", context, arguments, timeout, cancellation)
        {
            Ok(output) => {
                let mut section = parse_bsl_analyzer_search(
                    &output.result_text,
                    context,
                    request.limit,
                    cancellation,
                    self.bindings,
                );
                if matches!(
                    section.status,
                    ProviderSectionStatus::Ok
                        | ProviderSectionStatus::Empty
                        | ProviderSectionStatus::LimitReached
                        | ProviderSectionStatus::Partial
                ) {
                    section.index_freshness = Some("unknown".to_string());
                }
                // Keep stderr wherever the section is the provider's own
                // account of why it cannot serve — that includes waiting on
                // the index, whose stderr names the dependency. A plain
                // deadline timeout stays quiet: its stderr is a truncated
                // stream, not an explanation.
                let retain_stderr = match section.status {
                    ProviderSectionStatus::Ok
                    | ProviderSectionStatus::Empty
                    | ProviderSectionStatus::LimitReached => false,
                    ProviderSectionStatus::TimedOut => section
                        .termination
                        .as_ref()
                        .is_some_and(|termination| {
                            termination.code
                                == unica_code_intelligence::model::SearchTerminationCode::DependencyPending
                        }),
                    ProviderSectionStatus::Partial
                    | ProviderSectionStatus::Unavailable
                    | ProviderSectionStatus::Failed => true,
                };
                if retain_stderr && !output.stderr.trim().is_empty() {
                    section
                        .diagnostics
                        .push(format!("bsl-analyzer stderr: {}", output.stderr.trim()));
                }
                section
            }
            Err(error) if (self.bindings.engine_absent)(&context.workspace.cwd, "bsl-analyzer") => {
                unavailable_section(ProviderId::BslAnalyzer, error)
            }
            Err(error) => failed_section(ProviderId::BslAnalyzer, error),
        }
    }
}

pub fn parse_bsl_analyzer_search(
    text: &str,
    context: &CodeIntelligenceContext,
    requested_limit: usize,
    cancellation: &CancellationToken,
    bindings: SearchSourceBindings,
) -> ProviderSearchSection {
    // The pinned bsl-analyzer clamps search_code's requested limit to 50.
    const BSL_ANALYZER_SEARCH_MAX_LIMIT: usize = 50;
    let effective_limit = requested_limit.min(BSL_ANALYZER_SEARCH_MAX_LIMIT);
    let trimmed = text.trim();
    // The bundled analyzer keeps the no-hits sentence but appends a separate
    // indexing text block. Only a ready footer is evidence of an empty result;
    // pending, failed or unknown output must not become proven absence.
    let empty_lines: Vec<_> = trimmed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let empty_with_ready_footer = matches!(
        empty_lines.as_slice(),
        ["No results found."]
            | [
                "No results found.",
                "Indexing lexical: ready",
                "Indexing semantic: ready" | "Indexing semantic: disabled"
            ]
            | [
                "No results found.",
                "Indexing semantic: ready" | "Indexing semantic: disabled",
                "Indexing lexical: ready"
            ]
    );
    if trimmed.is_empty() || empty_with_ready_footer {
        return empty_section(ProviderId::BslAnalyzer, SearchRanking::Provider);
    }
    // A busy or still-indexing analyzer answers in plain text: no hits and
    // no proof of absence, but a later answer is promised. That is a
    // retryable wait, not a dead end. Only these whole answers qualify; an
    // index state outside the known set, a failed one or any other line
    // keeps the output failed.
    let index_state = |line: &&str| {
        line.starts_with("Indexing pending: ")
            || ["Indexing lexical: ", "Indexing semantic: "]
                .iter()
                .filter_map(|prefix| line.strip_prefix(prefix))
                .any(|state| matches!(state, "ready" | "disabled") || state.starts_with("running"))
    };
    let index_waits = |line: &&str| {
        line.starts_with("Indexing pending: ")
            || line.starts_with("Indexing lexical: running")
            || line.starts_with("Indexing semantic: running")
    };
    if matches!(
        empty_lines.as_slice(),
        ["Search index is busy (a long operation is holding it); please try again in a moment."]
            | ["Semantic search is busy (a long operation is holding the index). Lexical search is available in the meantime."]
    ) {
        return ProviderSearchSection::capacity_exhausted(
            ProviderId::BslAnalyzer.identity(),
            "bsl-analyzer search index is busy".to_string(),
        );
    }
    let pending = match empty_lines.split_first() {
        Some((
            &"No results found (overlay is warming up, only baseline search available).",
            tail,
        )) if tail.iter().all(index_state) => Some("updatingIndex"),
        Some((&"No results found.", tail))
            if tail.iter().all(index_state) && tail.iter().any(index_waits) =>
        {
            Some("buildingIndex")
        }
        _ => None,
    };
    if let Some(detail) = pending {
        return ProviderSearchSection::dependency_pending(
            ProviderId::BslAnalyzer.identity(),
            SearchRanking::Provider,
            SearchOrdering::Provider,
            Vec::new(),
            vec!["bsl-analyzer search index is not ready".to_string()],
            detail,
        )
        .expect("dependency-pending bsl-analyzer section is valid");
    }
    if let Ok(envelope) = serde_json::from_str::<Value>(trimmed) {
        if envelope.get("status").and_then(Value::as_str) == Some("not_ready") {
            // The analyzer models this as pending and even ships
            // `retry_after_ms` next to it: the next attempt is promised to
            // differ. That is the retryable dependency the search contract
            // spells `dependencyPending` — a permanent `unavailable` here
            // told waiting consumers to give up on an index that was about
            // to finish.
            let detail = envelope
                .get("detail")
                .and_then(Value::as_str)
                .unwrap_or("bsl-analyzer search index is not ready");
            return ProviderSearchSection::dependency_pending(
                ProviderId::BslAnalyzer.identity(),
                SearchRanking::Provider,
                SearchOrdering::Provider,
                Vec::new(),
                vec![detail.to_string()],
                "buildingIndex",
            )
            .expect("dependency-pending bsl-analyzer section is valid");
        }
        return failed_section(
            ProviderId::BslAnalyzer,
            format!("unexpected bsl-analyzer search envelope: {trimmed}"),
        );
    }

    let mut hits = Vec::new();
    let mut diagnostics = Vec::new();
    let mut current: Option<ProviderSearchHit> = None;
    let mut returned_headers = 0;
    let mut discarded_result = false;
    let mut output_budget_truncated = false;
    let mut locations =
        SearchLocationProjector::with_resolver(context, cancellation, bindings.resolver);
    for raw_line in text.lines() {
        let structural = raw_line.trim_start();
        let line = structural.trim_end();
        if line.starts_with('#') {
            returned_headers += 1;
            if let Some(hit) = current.take() {
                hits.push(hit);
            }
            match parse_bsl_analyzer_header(line, &mut locations) {
                Ok(hit) => current = Some(hit),
                Err(error) if error.starts_with(CANCELLED_PREFIX) => {
                    return failed_section(ProviderId::BslAnalyzer, error);
                }
                Err(error) => {
                    discarded_result = true;
                    diagnostics.push(format!(
                        "ignored malformed bsl-analyzer search header: {error}"
                    ));
                }
            }
        } else if let Some(graph_id) = line.strip_prefix("graph_id:") {
            if let Some(hit) = current.as_mut() {
                hit.attributes
                    .insert("graphId".to_string(), json!(graph_id.trim()));
            } else {
                diagnostics.push(format!("orphan bsl-analyzer graph id: {}", graph_id.trim()));
            }
        } else if let Some(snippet_line) = structural.strip_prefix('│') {
            if let Some(hit) = current.as_mut() {
                if !hit.snippet.is_empty() {
                    hit.snippet.push('\n');
                }
                hit.snippet
                    .push_str(snippet_line.strip_prefix(' ').unwrap_or(snippet_line));
            }
        } else if line.starts_with("--") && line.ends_with("--") {
            let diagnostic = line.trim_matches('-').trim();
            if !diagnostic.is_empty() {
                output_budget_truncated |=
                    diagnostic.contains("truncated to fit max_output_tokens");
                diagnostics.push(diagnostic.to_string());
            }
        } else if line.is_empty() {
            if let Some(hit) = current.take() {
                hits.push(hit);
            }
        } else {
            diagnostics.push(format!("unparsed bsl-analyzer search output: {line}"));
        }
    }
    if let Some(hit) = current {
        hits.push(hit);
    }
    for (index, hit) in hits.iter_mut().enumerate() {
        hit.rank = Some(index + 1);
    }

    if hits.is_empty() {
        diagnostics.insert(
            0,
            "bsl-analyzer returned non-empty output without any valid search hits".to_string(),
        );
        return ProviderSearchSection::failed(
            ProviderId::BslAnalyzer.identity(),
            diagnostics.join("; "),
        );
    }
    let provider_limit_reached = returned_headers >= effective_limit;
    if (discarded_result || output_budget_truncated) && provider_limit_reached {
        diagnostics.push("bsl-analyzer result limit was also reached".to_string());
    }
    let section = if discarded_result || output_budget_truncated {
        ProviderSearchSection::partial(
            ProviderId::BslAnalyzer.identity(),
            SearchRanking::Provider,
            SearchOrdering::Provider,
            hits,
            diagnostics,
        )
    } else if provider_limit_reached {
        ProviderSearchSection::limit_reached(
            ProviderId::BslAnalyzer.identity(),
            SearchRanking::Provider,
            SearchOrdering::Provider,
            hits,
            diagnostics,
        )
    } else {
        ProviderSearchSection::complete(
            ProviderId::BslAnalyzer.identity(),
            SearchRanking::Provider,
            SearchOrdering::Provider,
            hits,
            diagnostics,
        )
    };
    section.unwrap_or_else(|error| {
        ProviderSearchSection::failed(ProviderId::BslAnalyzer.identity(), error)
    })
}

pub fn parse_bsl_analyzer_header(
    line: &str,
    locations: &mut SearchLocationProjector<'_>,
) -> Result<ProviderSearchHit, String> {
    let after_hash = line
        .strip_prefix('#')
        .ok_or_else(|| "rank marker is missing".to_string())?;
    let (rank, after_rank) = after_hash
        .split_once(' ')
        .ok_or_else(|| "rank separator is missing".to_string())?;
    let rank = rank
        .parse::<usize>()
        .map_err(|_| "rank is not a positive integer".to_string())?;
    let after_open = after_rank
        .strip_prefix('[')
        .ok_or_else(|| "modality marker is missing".to_string())?;
    let (modality, after_modality) = after_open
        .split_once("] ")
        .ok_or_else(|| "modality terminator is missing".to_string())?;
    let (location, symbol_and_kind) = after_modality
        .split_once(" :: ")
        .ok_or_else(|| "symbol separator is missing".to_string())?;
    let line_separator = location
        .rfind(':')
        .ok_or_else(|| "line separator is missing".to_string())?;
    let path = location[..line_separator].trim();
    let line_range = &location[line_separator + 1..];
    let (line_start, end_line) = match line_range.split_once('-') {
        Some((start, end)) => (
            start
                .parse::<usize>()
                .map_err(|_| "line start is not a positive integer".to_string())?,
            Some(
                end.parse::<usize>()
                    .map_err(|_| "line end is not a positive integer".to_string())?,
            ),
        ),
        None => (
            line_range
                .parse::<usize>()
                .map_err(|_| "line is not a positive integer".to_string())?,
            None,
        ),
    };
    let symbol_and_kind = symbol_and_kind
        .strip_suffix(')')
        .ok_or_else(|| "symbol kind terminator is missing".to_string())?;
    let (symbol, kind) = symbol_and_kind
        .rsplit_once(" (")
        .ok_or_else(|| "symbol kind is missing".to_string())?;
    let mut attributes = Map::new();
    attributes.insert("modality".to_string(), json!(modality));
    Ok(ProviderSearchHit {
        rank: Some(rank),
        provider_score: None,
        location: locations.project(Path::new(path))?,
        line: line_start,
        end_line,
        symbol: Some(symbol.to_string()),
        kind: Some(kind.to_string()),
        snippet: String::new(),
        attributes,
    })
}

/// Разбор ответа анализатора о графе вызовов.
///
/// Форма снята с живого `bsl-analyzer 0.2.67` (профиль MCP `workspace`,
/// инструмент `graph`, contract 1.4) и закреплена фикстурой
/// `tests/fixtures/bsl_analyzer/graph-0.2.67.json`. Замер изменил три решения
/// проекта, и разбор их держит:
///
/// * счёт берётся из `inTotal`/`outTotal` анализатора, а не из длины страницы,
///   поэтому он верен и при урезанной странице;
/// * незавершённый индекс — названное состояние `loading`, а не ошибка и не
///   отсутствие счёта: иначе «не посчитано» не отличить от «вызовов нет»;
/// * происхождение ребра сохраняется: выведенное ребро не выдаётся за
///   разрешённое.
pub fn parse_call_graph_answer(
    value: &Value,
    direction: CallGraphDirection,
) -> Result<CallGraphResult, String> {
    // Индекс ещё строится. Анализатор говорит это явно и при `isError: false`,
    // поэтому состояние переносится, а не превращается в отказ.
    if value.get("status").and_then(Value::as_str) == Some("loading") {
        return Ok(CallGraphResult {
            state: CallGraphState::Indexing,
            total: None,
            edges: Vec::new(),
            revision: None,
            stale: None,
            complete: false,
        });
    }
    let revision = value
        .get("revision")
        .and_then(Value::as_u64)
        .ok_or_else(|| "call graph answer has no revision".to_string())?;
    let stale = value.get("stale").and_then(Value::as_bool);
    let result = value
        .get("result")
        .ok_or_else(|| "call graph answer has no result".to_string())?;
    // Неизвестный узел анализатор называет в теле ответа, а не протоколом.
    if let Some(error) = result.get("error").and_then(Value::as_str) {
        return Err(error.to_string());
    }
    let total = result
        .get(direction.total_field())
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("call graph answer has no {} count", direction.total_field()))?;
    if result.get("total").and_then(Value::as_u64) != Some(total) {
        return Err("call graph direction and neighbour counts disagree".to_string());
    }
    let returned = result
        .get("returned")
        .and_then(Value::as_u64)
        .ok_or_else(|| "call graph answer has no returned count".to_string())?;
    let dropped = result
        .get("dropped_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| "call graph answer has no dropped count".to_string())?;
    if returned.checked_add(dropped) != Some(total) {
        return Err("call graph counts disagree".to_string());
    }
    let connectors_dropped = result
        .get("connectors_dropped")
        .and_then(Value::as_bool)
        .ok_or_else(|| "call graph answer has no connector completeness".to_string())?;
    let root = result
        .get("root")
        .and_then(|root| root.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| "call graph answer has no root id".to_string())?;
    let mut edges = Vec::new();
    let mut self_call = false;
    let provider_edges = result
        .get("edges")
        .and_then(Value::as_array)
        .ok_or_else(|| "call graph answer has no edge array".to_string())?;
    for edge in provider_edges {
        // Направление решает, какой конец ребра называет соседа: у входящего
        // это `from`, у исходящего `to`.
        if edge.get("kind").and_then(Value::as_str) != Some("call") {
            return Err("call graph provider returned a non-call edge".to_string());
        }
        let peer = match direction {
            CallGraphDirection::Callers => edge.get("from"),
            CallGraphDirection::Callees => edge.get("to"),
        }
        .and_then(Value::as_str);
        let peer = match peer {
            Some(peer) => peer,
            None if edge.get("from").is_none() && edge.get("to").is_none() => {
                self_call = true;
                root
            }
            None => return Err("call graph edge names no peer".to_string()),
        };
        let provenance = match edge.get("provenance").and_then(Value::as_str) {
            Some("resolved") => CallEdgeProvenance::Resolved,
            Some("inferred") => CallEdgeProvenance::Inferred,
            // Неизвестное происхождение не приравнивается к разрешённому:
            // словарь закрыт, и незнакомое значение — повод отказать.
            other => {
                return Err(format!(
                    "call graph edge has an unknown provenance: {}",
                    other.unwrap_or("none")
                ))
            }
        };
        edges.push(CallGraphEdge {
            id: peer.to_string(),
            provenance,
        });
    }
    let total = total
        .checked_add(u64::from(self_call))
        .ok_or_else(|| "call graph count overflow".to_string())?;
    let complete = dropped == 0 && !connectors_dropped;
    if complete && u64::try_from(edges.len()).ok() != Some(total) {
        return Err("call graph edge count disagrees with the complete result".to_string());
    }
    Ok(CallGraphResult {
        state: CallGraphState::Ready,
        total: Some(total),
        edges,
        revision: Some(revision),
        stale,
        complete,
    })
}
