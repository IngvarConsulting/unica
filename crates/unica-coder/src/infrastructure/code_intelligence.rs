use crate::domain::cancellation::{cancelled_error, CancellationToken, CANCELLED_PREFIX};
use crate::domain::code_intelligence::{
    CodeIntelligenceContext, CodeIntelligenceProvider, CodeIntelligenceReadData,
    CodeIntelligenceReadRequest, ProviderCapability, ProviderDeadline, ProviderId,
    ProviderReadOutcome, ProviderSearchHit, ProviderSearchSection, ProviderSectionStatus,
    SearchRequest,
};
use crate::infrastructure::bsl_outline::render_current_source_outline;
use crate::infrastructure::internal_adapters::{
    system_process_runner, ProcessCommand, ProcessOutput, ProcessRunner,
};
use crate::infrastructure::redaction::redactor;
use crate::infrastructure::rlm_navigation::RlmNavigationAdapter;
use crate::infrastructure::workspace_index::IndexReadiness;
use crate::infrastructure::workspace_services::{
    WorkspaceRlmOperation, WorkspaceServiceBslCall, WorkspaceServiceBslOutput,
    WorkspaceServiceManager, WorkspaceServiceRlmCall,
};
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::time::Duration;

const SEARCH_CAPABILITIES: &[ProviderCapability] = &[ProviderCapability::Search];
/// ADR-0020: the outline is built from the current BSL file by the pinned
/// `bsl-parser`, so it belongs to this provider and not to the index.
const BSL_ANALYZER_CAPABILITIES: &[ProviderCapability] =
    &[ProviderCapability::Search, ProviderCapability::Outline];
const RLM_CAPABILITIES: &[ProviderCapability] = &[
    ProviderCapability::Search,
    ProviderCapability::Definition,
    ProviderCapability::ObjectProfile,
];

pub(crate) struct GitGrepProvider<'a> {
    runner: &'a (dyn ProcessRunner + Send + Sync),
}

impl GitGrepProvider<'static> {
    pub(crate) fn new() -> Self {
        Self {
            runner: system_process_runner(),
        }
    }
}

impl<'a> GitGrepProvider<'a> {
    #[cfg(test)]
    fn with_runner(runner: &'a (dyn ProcessRunner + Send + Sync)) -> Self {
        Self { runner }
    }

    fn run_search(
        &self,
        request: &SearchRequest,
        context: &CodeIntelligenceContext,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> ProviderSearchSection {
        if cancellation.is_cancelled() {
            return failed_section(
                ProviderId::GitGrep,
                cancelled_error("git-grep search stopped before process start"),
            );
        }
        let timeout = deadline.remaining();
        if timeout.is_zero() {
            return failed_section(
                ProviderId::GitGrep,
                "git-grep provider deadline exceeded".to_string(),
            );
        }
        let args = vec![
            "-c".to_string(),
            "core.quotepath=false".to_string(),
            "grep".to_string(),
            "--no-color".to_string(),
            "--untracked".to_string(),
            "-n".to_string(),
            "-F".to_string(),
            "-m".to_string(),
            request.limit.to_string(),
            "-e".to_string(),
            request.query.clone(),
            "--".to_string(),
            ".".to_string(),
        ];
        let output = match self.runner.run(&ProcessCommand {
            program: PathBuf::from("git"),
            args,
            cwd: context.source_root.path.clone(),
            timeout: Some(timeout),
            cancellation: cancellation.clone(),
        }) {
            Ok(output) => output,
            Err(error) => {
                return unavailable_section(
                    ProviderId::GitGrep,
                    format!("failed to start git grep: {error}"),
                );
            }
        };
        git_grep_section(output, request.limit)
    }
}

impl CodeIntelligenceProvider for GitGrepProvider<'_> {
    fn id(&self) -> ProviderId {
        ProviderId::GitGrep
    }

    fn capabilities(&self) -> &[ProviderCapability] {
        SEARCH_CAPABILITIES
    }

    fn search(
        &self,
        request: &SearchRequest,
        context: &CodeIntelligenceContext,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> ProviderSearchSection {
        self.run_search(request, context, deadline, cancellation)
    }
}

trait BslSearchClient: Send + Sync {
    fn search(
        &self,
        context: &CodeIntelligenceContext,
        arguments: Value,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceServiceBslOutput, String>;
}

struct WorkspaceBslSearchClient;

impl BslSearchClient for WorkspaceBslSearchClient {
    fn search(
        &self,
        context: &CodeIntelligenceContext,
        arguments: Value,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceServiceBslOutput, String> {
        WorkspaceServiceManager::new().call_bsl_mcp_cancellable_with_budget(
            &context.workspace,
            &context.source_root.path,
            WorkspaceServiceBslCall::new("search", arguments, timeout, timeout),
            cancellation,
        )
    }
}

static WORKSPACE_BSL_SEARCH_CLIENT: WorkspaceBslSearchClient = WorkspaceBslSearchClient;

pub(crate) struct BslAnalyzerProvider<'a> {
    client: &'a (dyn BslSearchClient + Send + Sync),
}

impl BslAnalyzerProvider<'static> {
    pub(crate) fn new() -> Self {
        Self {
            client: &WORKSPACE_BSL_SEARCH_CLIENT,
        }
    }
}

impl<'a> BslAnalyzerProvider<'a> {
    #[cfg(test)]
    fn with_client(client: &'a (dyn BslSearchClient + Send + Sync)) -> Self {
        Self { client }
    }
}

impl CodeIntelligenceProvider for BslAnalyzerProvider<'_> {
    fn id(&self) -> ProviderId {
        ProviderId::BslAnalyzer
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
            provider: ProviderId::BslAnalyzer,
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
        let timeout = deadline.remaining();
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
            .search(context, arguments, timeout, cancellation)
        {
            Ok(output) => {
                let mut section = parse_bsl_analyzer_search(&output.result_text);
                let retain_stderr = match section.status {
                    ProviderSectionStatus::Ok | ProviderSectionStatus::Empty => false,
                    ProviderSectionStatus::Unavailable | ProviderSectionStatus::Failed => true,
                };
                if retain_stderr && !output.stderr.trim().is_empty() {
                    section
                        .diagnostics
                        .push(format!("bsl-analyzer stderr: {}", output.stderr.trim()));
                }
                section
            }
            Err(error) if is_provider_unavailable_error(&error) => {
                unavailable_section(ProviderId::BslAnalyzer, error)
            }
            Err(error) => failed_section(ProviderId::BslAnalyzer, error),
        }
    }
}

trait RlmSearchClient: Send + Sync {
    fn readiness(
        &self,
        context: &CodeIntelligenceContext,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<IndexReadiness, String>;

    fn search(
        &self,
        context: &CodeIntelligenceContext,
        query: &str,
        limit: usize,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<String, String>;
}

struct WorkspaceRlmSearchClient;

impl RlmSearchClient for WorkspaceRlmSearchClient {
    fn readiness(
        &self,
        context: &CodeIntelligenceContext,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<IndexReadiness, String> {
        WorkspaceServiceManager::new().rlm_readiness_cancellable_with_timeout(
            &context.workspace,
            &context.source_root.path,
            &Map::new(),
            timeout,
            cancellation,
        )
    }

    fn search(
        &self,
        context: &CodeIntelligenceContext,
        query: &str,
        limit: usize,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<String, String> {
        WorkspaceServiceManager::new()
            .call_rlm_cancellable(
                &context.workspace,
                &context.source_root.path,
                WorkspaceRlmOperation::Search {
                    query: query.to_string(),
                    limit,
                },
                timeout,
                cancellation,
            )
            .and_then(|result| match result {
                WorkspaceServiceRlmCall::Output(output) => Ok(output.result_text),
                WorkspaceServiceRlmCall::Unready(readiness) => {
                    Err(rlm_search_unready_error(readiness))
                }
            })
    }
}

fn rlm_search_unready_error(readiness: IndexReadiness) -> String {
    let detail = match readiness {
        IndexReadiness::Ready { .. } => "index readiness changed unexpectedly".to_string(),
        IndexReadiness::Missing => "index is missing".to_string(),
        IndexReadiness::Stale { status } => format!("index is stale: {}", redactor(&status)),
        IndexReadiness::Building => "index is building".to_string(),
        IndexReadiness::Failed(error) | IndexReadiness::Unavailable(error) => redactor(&error),
    };
    format!("RLM index became unavailable: {detail}")
}

static WORKSPACE_RLM_SEARCH_CLIENT: WorkspaceRlmSearchClient = WorkspaceRlmSearchClient;

pub(crate) struct RlmProvider<'a> {
    client: &'a (dyn RlmSearchClient + Send + Sync),
}

impl RlmProvider<'static> {
    pub(crate) fn new() -> Self {
        Self {
            client: &WORKSPACE_RLM_SEARCH_CLIENT,
        }
    }
}

impl<'a> RlmProvider<'a> {
    #[cfg(test)]
    fn with_client(client: &'a (dyn RlmSearchClient + Send + Sync)) -> Self {
        Self { client }
    }
}

impl CodeIntelligenceProvider for RlmProvider<'_> {
    fn id(&self) -> ProviderId {
        ProviderId::Rlm
    }

    fn capabilities(&self) -> &[ProviderCapability] {
        RLM_CAPABILITIES
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
                ProviderId::Rlm,
                cancelled_error("RLM search stopped before readiness check"),
            );
        }
        let readiness_timeout = deadline.remaining();
        if readiness_timeout.is_zero() {
            return failed_section(
                ProviderId::Rlm,
                "RLM provider deadline exceeded before readiness check".to_string(),
            );
        }
        let readiness_result = self
            .client
            .readiness(context, readiness_timeout, cancellation);
        if cancellation.is_cancelled() {
            return failed_section(
                ProviderId::Rlm,
                cancelled_error("RLM search stopped after readiness check"),
            );
        }
        let readiness = match readiness_result {
            Ok(readiness) => readiness,
            Err(error) if error.starts_with(CANCELLED_PREFIX) => {
                return failed_section(ProviderId::Rlm, error);
            }
            Err(error) => return unavailable_section(ProviderId::Rlm, redactor(&error)),
        };
        match readiness {
            IndexReadiness::Ready { .. } => {}
            IndexReadiness::Missing => {
                return unavailable_section(
                    ProviderId::Rlm,
                    "rlm index is missing; background build requested".to_string(),
                );
            }
            IndexReadiness::Stale { status } => {
                return unavailable_section(
                    ProviderId::Rlm,
                    format!(
                        "rlm index is stale ({}); background update requested",
                        redactor(&status)
                    ),
                );
            }
            IndexReadiness::Building => {
                return unavailable_section(ProviderId::Rlm, "rlm index building".to_string());
            }
            IndexReadiness::Failed(error) | IndexReadiness::Unavailable(error) => {
                return unavailable_section(ProviderId::Rlm, redactor(&error));
            }
        }
        let timeout = deadline.remaining();
        if timeout.is_zero() {
            return failed_section(
                ProviderId::Rlm,
                "RLM provider deadline exceeded".to_string(),
            );
        }
        match self.client.search(
            context,
            &request.query,
            request.limit,
            timeout,
            cancellation,
        ) {
            Ok(result) => parse_rlm_search(&result),
            Err(error) => failed_section(ProviderId::Rlm, error),
        }
    }

    fn read(
        &self,
        request: &CodeIntelligenceReadRequest,
        context: &CodeIntelligenceContext,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<ProviderReadOutcome, String> {
        let navigation = RlmNavigationAdapter::new().invoke_resolved_cancellable(
            request,
            context,
            deadline,
            cancellation,
        )?;
        let outcome = navigation.outcome;
        Ok(ProviderReadOutcome {
            provider: ProviderId::Rlm,
            ok: outcome.ok,
            summary: outcome.summary,
            warnings: outcome.warnings,
            errors: outcome.errors,
            artifacts: outcome.artifacts,
            stdout: outcome.stdout,
            stderr: outcome.stderr,
            data: navigation.data,
        })
    }
}

fn parse_rlm_search(text: &str) -> ProviderSearchSection {
    let value: Value = match serde_json::from_str(text.trim()) {
        Ok(value) => value,
        Err(error) => {
            return failed_section(
                ProviderId::Rlm,
                format!("invalid RLM search helper JSON: {error}"),
            );
        }
    };
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return failed_section(ProviderId::Rlm, error.to_string());
    }
    let Some(rows) = value.as_array() else {
        return failed_section(
            ProviderId::Rlm,
            "RLM search helper returned a non-array result".to_string(),
        );
    };
    let mut hits = Vec::new();
    let mut diagnostics = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        match parse_rlm_search_row(row, hits.len() + 1) {
            Ok(hit) => hits.push(hit),
            Err(error) => {
                diagnostics.push(format!("ignored malformed RLM result #{index}: {error}"))
            }
        }
    }
    let status = if hits.is_empty() {
        if rows.is_empty() {
            ProviderSectionStatus::Empty
        } else {
            diagnostics.insert(0, "RLM search helper returned no valid rows".to_string());
            ProviderSectionStatus::Failed
        }
    } else {
        ProviderSectionStatus::Ok
    };
    ProviderSearchSection {
        provider: ProviderId::Rlm,
        status,
        hits,
        diagnostics,
        artifacts: Vec::new(),
    }
}

fn parse_rlm_search_row(row: &Value, rank: usize) -> Result<ProviderSearchHit, String> {
    let object = row
        .as_object()
        .ok_or_else(|| "row is not an object".to_string())?;
    let path = object
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| "path is missing".to_string())?
        .replace('\\', "/");
    let text = object
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| "text is missing".to_string())?;
    let detail = object
        .get("detail")
        .and_then(Value::as_object)
        .ok_or_else(|| "detail is missing or is not an object".to_string())?;
    let line = detail
        .get("line")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(1)
        .max(1);
    let end_line = detail
        .get("end_line")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok());
    let source_type = object
        .get("source_type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let symbol = detail
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| (!text.is_empty()).then_some(text))
        .map(str::to_string);
    let kind = detail
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or(source_type)
        .to_string();
    let mut attributes = Map::new();
    if let Some(value) = object.get("object_name").filter(|value| !value.is_null()) {
        attributes.insert("objectName".to_string(), value.clone());
    }
    if let Some(value) = object.get("path_kind").filter(|value| !value.is_null()) {
        attributes.insert("pathKind".to_string(), value.clone());
    }
    attributes.insert("detail".to_string(), Value::Object(detail.clone()));
    Ok(ProviderSearchHit {
        rank,
        provider_score: detail.get("rank").and_then(Value::as_f64),
        path,
        line,
        end_line,
        symbol,
        kind: Some(kind),
        snippet: text.to_string(),
        attributes,
    })
}

fn parse_bsl_analyzer_search(text: &str) -> ProviderSearchSection {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed == "No results found." {
        return empty_section(ProviderId::BslAnalyzer);
    }
    if let Ok(envelope) = serde_json::from_str::<Value>(trimmed) {
        if envelope.get("status").and_then(Value::as_str) == Some("not_ready") {
            let detail = envelope
                .get("detail")
                .and_then(Value::as_str)
                .unwrap_or("bsl-analyzer search index is not ready");
            return unavailable_section(ProviderId::BslAnalyzer, detail.to_string());
        }
        return failed_section(
            ProviderId::BslAnalyzer,
            format!("unexpected bsl-analyzer search envelope: {trimmed}"),
        );
    }

    let mut hits = Vec::new();
    let mut diagnostics = Vec::new();
    let mut current: Option<ProviderSearchHit> = None;
    for raw_line in text.lines() {
        let structural = raw_line.trim_start();
        let line = structural.trim_end();
        if line.starts_with('#') {
            if let Some(hit) = current.take() {
                hits.push(hit);
            }
            match parse_bsl_analyzer_header(line) {
                Some(hit) => current = Some(hit),
                None => diagnostics.push(format!(
                    "ignored malformed bsl-analyzer search header: {line}"
                )),
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
        hit.rank = index + 1;
    }

    let status = if hits.is_empty() {
        diagnostics.insert(
            0,
            "bsl-analyzer returned non-empty output without any valid search hits".to_string(),
        );
        ProviderSectionStatus::Failed
    } else {
        ProviderSectionStatus::Ok
    };
    ProviderSearchSection {
        provider: ProviderId::BslAnalyzer,
        status,
        hits,
        diagnostics,
        artifacts: Vec::new(),
    }
}

fn parse_bsl_analyzer_header(line: &str) -> Option<ProviderSearchHit> {
    let after_hash = line.strip_prefix('#')?;
    let (rank, after_rank) = after_hash.split_once(' ')?;
    let rank = rank.parse::<usize>().ok()?;
    let after_open = after_rank.strip_prefix('[')?;
    let (modality, after_modality) = after_open.split_once("] ")?;
    let (location, symbol_and_kind) = after_modality.split_once(" :: ")?;
    let line_separator = location.rfind(':')?;
    let path = location[..line_separator].trim();
    let line_range = &location[line_separator + 1..];
    let (line_start, end_line) = match line_range.split_once('-') {
        Some((start, end)) => (
            start.parse::<usize>().ok()?,
            Some(end.parse::<usize>().ok()?),
        ),
        None => (line_range.parse::<usize>().ok()?, None),
    };
    let symbol_and_kind = symbol_and_kind.strip_suffix(')')?;
    let (symbol, kind) = symbol_and_kind.rsplit_once(" (")?;
    let mut attributes = Map::new();
    attributes.insert("modality".to_string(), json!(modality));
    Some(ProviderSearchHit {
        rank,
        provider_score: None,
        path: path.replace('\\', "/"),
        line: line_start,
        end_line,
        symbol: Some(symbol.to_string()),
        kind: Some(kind.to_string()),
        snippet: String::new(),
        attributes,
    })
}

fn git_grep_section(output: ProcessOutput, limit: usize) -> ProviderSearchSection {
    if output.cancelled {
        return failed_section(
            ProviderId::GitGrep,
            cancelled_error("git-grep search process stopped"),
        );
    }
    if output.timed_out {
        return failed_section(
            ProviderId::GitGrep,
            "git-grep provider deadline exceeded".to_string(),
        );
    }
    let no_matches = !output.status_success
        && output.stdout.trim().is_empty()
        && output.stderr.trim().is_empty()
        && process_exit_code_is(&output.status, 1);
    if no_matches {
        return empty_section(ProviderId::GitGrep);
    }
    if !output.status_success
        && output.stderr.contains("not a git repository")
        && process_exit_code_is(&output.status, 128)
    {
        return unavailable_section(ProviderId::GitGrep, output.stderr.trim().to_string());
    }
    let captured_success = output.stdout_truncated && process_exit_code_is(&output.status, 0);
    if !output.status_success && !captured_success {
        let detail = if output.stderr.trim().is_empty() {
            format!("git grep exited with status {}", output.status)
        } else {
            output.stderr.trim().to_string()
        };
        return failed_section(ProviderId::GitGrep, detail);
    }

    let mut diagnostics = Vec::new();
    let mut hits = Vec::new();
    let mut rows = output.stdout.lines();
    if output.stdout_truncated {
        // The runner keeps the tail of the capture, so the first retained row lost
        // its leading bytes. Such a fragment still parses as `path:line:snippet`
        // and would publish a path that does not exist, so drop it. At worst one
        // whole row is lost when the cut landed on a line boundary.
        rows.next();
    }
    for line in rows.filter(|line| !line.trim().is_empty()) {
        match parse_git_grep_line(line) {
            Some(hit) => hits.push(hit),
            None => diagnostics.push(format!("ignored malformed git-grep result: {line}")),
        }
    }
    hits.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.snippet.cmp(&right.snippet))
    });
    hits.truncate(limit);
    for (index, hit) in hits.iter_mut().enumerate() {
        hit.rank = index + 1;
    }
    if output.stdout_truncated {
        diagnostics.push("git-grep output was truncated by the process runner".to_string());
    }
    let status = if hits.is_empty() {
        if output.stdout_truncated {
            diagnostics.insert(
                0,
                "git-grep capture contained no complete result after truncation".to_string(),
            );
            ProviderSectionStatus::Failed
        } else if output.stdout.trim().is_empty() {
            ProviderSectionStatus::Empty
        } else {
            diagnostics.insert(
                0,
                "git-grep returned non-empty output without any valid results".to_string(),
            );
            ProviderSectionStatus::Failed
        }
    } else {
        ProviderSectionStatus::Ok
    };
    ProviderSearchSection {
        provider: ProviderId::GitGrep,
        status,
        hits,
        diagnostics,
        artifacts: Vec::new(),
    }
}

fn parse_git_grep_line(line: &str) -> Option<ProviderSearchHit> {
    let mut parts = line.splitn(3, ':');
    let path = parts.next()?.trim();
    let line_number = parts.next()?.parse::<usize>().ok()?;
    let snippet = parts.next()?.trim();
    if path.is_empty() {
        return None;
    }
    Some(ProviderSearchHit {
        rank: 0,
        provider_score: None,
        path: path.replace('\\', "/"),
        line: line_number,
        end_line: None,
        symbol: None,
        kind: None,
        snippet: snippet.to_string(),
        attributes: Map::new(),
    })
}

/// Causes that mean "this provider is not present here", as opposed to "this
/// provider ran and failed". `code.search` degrades on them and `code.graph`
/// reports them instead of failing the call, so the classification stays in one
/// place rather than being restated per caller.
pub(crate) fn is_provider_unavailable_error(error: &str) -> bool {
    [
        "could not locate Unica plugin root",
        "Unica third-party manifest not found",
        "bundled tool binary is not present",
        "tool not found in manifest",
        "tool not found in tools lock",
        "No such file or directory",
    ]
    .iter()
    .any(|marker| error.contains(marker))
}

fn process_exit_code_is(status: &str, code: i32) -> bool {
    let status = status.trim();
    status == code.to_string() || status.ends_with(&format!(": {code}"))
}

fn empty_section(provider: ProviderId) -> ProviderSearchSection {
    ProviderSearchSection {
        provider,
        status: ProviderSectionStatus::Empty,
        hits: Vec::new(),
        diagnostics: Vec::new(),
        artifacts: Vec::new(),
    }
}

fn unavailable_section(provider: ProviderId, diagnostic: String) -> ProviderSearchSection {
    ProviderSearchSection {
        provider,
        status: ProviderSectionStatus::Unavailable,
        hits: Vec::new(),
        diagnostics: vec![diagnostic],
        artifacts: Vec::new(),
    }
}

fn failed_section(provider: ProviderId, diagnostic: String) -> ProviderSearchSection {
    ProviderSearchSection {
        provider,
        status: ProviderSectionStatus::Failed,
        hits: Vec::new(),
        diagnostics: vec![diagnostic],
        artifacts: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_bsl_analyzer_search, parse_rlm_search, rlm_search_unready_error, BslAnalyzerProvider,
        BslSearchClient, GitGrepProvider, RlmProvider, RlmSearchClient,
    };
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::cancellation::CANCELLED_PREFIX;
    use crate::domain::code_intelligence::{
        CodeIntelligenceContext, CodeIntelligenceProvider, CodeIntelligenceReadRequest,
        CodeIntelligenceRegistry, ProviderCapability, ProviderDeadline, ProviderId,
        ProviderSectionStatus, SearchRequest,
    };
    use crate::domain::source_roots::ResolvedSourceRoot;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::internal_adapters::{ProcessCommand, ProcessOutput, ProcessRunner};
    use crate::infrastructure::workspace_index::IndexReadiness;
    use crate::infrastructure::workspace_services::WorkspaceServiceBslOutput;
    use serde_json::Value;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
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

    struct FakeRunner {
        output: ProcessOutput,
        commands: Mutex<Vec<ProcessCommand>>,
    }

    impl ProcessRunner for FakeRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.commands.lock().unwrap().push(command.clone());
            Ok(self.output.clone())
        }
    }

    fn context() -> CodeIntelligenceContext {
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

    fn output(stdout: &str) -> ProcessOutput {
        ProcessOutput {
            status_success: true,
            status: "exit status: 0".to_string(),
            stdout: stdout.to_string(),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
        }
    }

    #[test]
    fn git_grep_is_literal_source_scoped_and_uses_the_upstream_deadline() {
        let runner = FakeRunner {
            output: output("CommonModules/Sales/Ext/Module.bsl:4:Post();\n"),
            commands: Mutex::new(Vec::new()),
        };
        let provider = GitGrepProvider::with_runner(&runner);

        let section = provider.search(
            &SearchRequest {
                query: "Post.*".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(60)),
            &CancellationToken::new(),
        );

        assert_eq!(section.provider, ProviderId::GitGrep);
        assert_eq!(section.status, ProviderSectionStatus::Ok);
        let commands = runner.commands.lock().unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].cwd, PathBuf::from("/workspace/src"));
        assert_eq!(
            commands[0].args,
            [
                "-c",
                "core.quotepath=false",
                "grep",
                "--no-color",
                "--untracked",
                "-n",
                "-F",
                "-m",
                "20",
                "-e",
                "Post.*",
                "--",
                ".",
            ]
        );
        assert!(commands[0].args.iter().any(|arg| arg == "-F"));
        assert!(commands[0].args.iter().any(|arg| arg == "Post.*"));
        assert!(!commands[0].args.iter().any(|arg| arg == "-i"));
        let timeout = commands[0].timeout.unwrap();
        assert!(timeout > Duration::from_secs(15), "{timeout:?}");
        assert!(timeout <= Duration::from_secs(60), "{timeout:?}");
    }

    #[test]
    fn git_grep_parses_sorts_and_ranks_hits_locally() {
        let runner = FakeRunner {
            output: output(
                "b/Module.bsl:9:Second\n\
                 a/Module.bsl:7:Later\n\
                 a/Module.bsl:2:First\n",
            ),
            commands: Mutex::new(Vec::new()),
        };

        let section = GitGrepProvider::with_runner(&runner).search(
            &SearchRequest {
                query: "needle".to_string(),
                limit: 2,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Ok);
        assert_eq!(section.hits.len(), 2);
        assert_eq!(section.hits[0].rank, 1);
        assert_eq!(section.hits[0].path, "a/Module.bsl");
        assert_eq!(section.hits[0].line, 2);
        assert_eq!(section.hits[1].rank, 2);
        assert_eq!(section.hits[1].path, "a/Module.bsl");
        assert_eq!(section.hits[1].line, 7);
        assert!(section.hits.iter().all(|hit| hit.provider_score.is_none()));
    }

    #[test]
    fn git_grep_preserves_utf8_paths() {
        let runner = FakeRunner {
            output: output("Catalogs/Номенклатура.xml:7:<Name>Номенклатура</Name>\n"),
            commands: Mutex::new(Vec::new()),
        };

        let section = GitGrepProvider::with_runner(&runner).search(
            &SearchRequest {
                query: "Номенклатура".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Ok);
        assert_eq!(section.hits[0].path, "Catalogs/Номенклатура.xml");
    }

    #[test]
    fn git_grep_keeps_valid_hits_when_capture_was_truncated() {
        let runner = FakeRunner {
            output: ProcessOutput {
                status_success: false,
                status: "exit status: 0".to_string(),
                stdout: concat!(
                    "dules/Broken/Ext/Module.bsl:12:Procedure Broken()\n",
                    "CommonModules/Test/Ext/Module.bsl:1:Procedure Test()\n"
                )
                .to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: true,
            },
            commands: Mutex::new(Vec::new()),
        };

        let section = GitGrepProvider::with_runner(&runner).search(
            &SearchRequest {
                query: "Procedure".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Ok);
        assert_eq!(section.hits.len(), 1);
        assert!(section
            .diagnostics
            .iter()
            .any(|item| item.contains("truncated")));
    }

    #[test]
    fn git_grep_drops_the_partial_first_row_of_a_truncated_capture() {
        let runner = FakeRunner {
            output: ProcessOutput {
                status_success: false,
                status: "exit status: 0".to_string(),
                stdout: concat!(
                    "dules/Broken/Ext/Module.bsl:12:Procedure Broken()\n",
                    "CommonModules/Test/Ext/Module.bsl:1:Procedure Test()\n"
                )
                .to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: true,
            },
            commands: Mutex::new(Vec::new()),
        };

        let section = GitGrepProvider::with_runner(&runner).search(
            &SearchRequest {
                query: "Procedure".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.hits.len(), 1);
        assert_eq!(section.hits[0].path, "CommonModules/Test/Ext/Module.bsl");
        assert!(
            !section
                .hits
                .iter()
                .any(|hit| hit.path == "dules/Broken/Ext/Module.bsl"),
            "{:?}",
            section.hits
        );
    }

    #[test]
    fn git_grep_does_not_report_empty_when_truncation_removed_the_only_row() {
        let runner = FakeRunner {
            output: ProcessOutput {
                status_success: false,
                status: "exit status: 0".to_string(),
                stdout: "dules/Broken/Ext/Module.bsl:12:Procedure Broken()\n".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: true,
            },
            commands: Mutex::new(Vec::new()),
        };

        let section = GitGrepProvider::with_runner(&runner).search(
            &SearchRequest {
                query: "Procedure".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Failed);
        assert!(section.hits.is_empty());
        assert!(section
            .diagnostics
            .iter()
            .any(|item| item.contains("truncated")));
    }

    #[test]
    fn git_grep_keeps_every_row_when_the_capture_was_not_truncated() {
        let runner = FakeRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: concat!(
                    "CommonModules/Other/Ext/Module.bsl:12:Procedure Other()\n",
                    "CommonModules/Test/Ext/Module.bsl:1:Procedure Test()\n"
                )
                .to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
            },
            commands: Mutex::new(Vec::new()),
        };

        let section = GitGrepProvider::with_runner(&runner).search(
            &SearchRequest {
                query: "Procedure".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Ok);
        assert_eq!(section.hits.len(), 2);
    }

    #[test]
    fn git_grep_reports_non_repository_workspace_as_unavailable() {
        let runner = FakeRunner {
            output: ProcessOutput {
                status_success: false,
                status: "exit status: 128".to_string(),
                stdout: String::new(),
                stderr: "fatal: not a git repository".to_string(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
            },
            commands: Mutex::new(Vec::new()),
        };

        let section = GitGrepProvider::with_runner(&runner).search(
            &SearchRequest {
                query: "Procedure".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Unavailable);
        assert!(section.hits.is_empty());
    }

    #[test]
    fn git_grep_exit_one_without_stderr_is_empty() {
        let runner = FakeRunner {
            output: ProcessOutput {
                status_success: false,
                status: "exit status: 1".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
            },
            commands: Mutex::new(Vec::new()),
        };

        let section = GitGrepProvider::with_runner(&runner).search(
            &SearchRequest {
                query: "absent".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Empty);
        assert!(section.hits.is_empty());
    }

    #[test]
    fn git_grep_non_empty_malformed_output_is_failed() {
        let runner = FakeRunner {
            output: output("not-a-git-grep-row\n"),
            commands: Mutex::new(Vec::new()),
        };

        let section = GitGrepProvider::with_runner(&runner).search(
            &SearchRequest {
                query: "needle".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Failed);
        assert!(section.hits.is_empty());
        assert!(section.diagnostics[0].contains("without any valid results"));
    }

    #[test]
    fn bsl_analyzer_ranked_text_parser_preserves_modality_graph_id_and_windows_paths() {
        let section = parse_bsl_analyzer_search(
            "#1 [L+S] C:\\repo\\src\\CommonModules\\Sales\\Ext\\Module.bsl:42-58 :: Post (procedure)\n\
               graph_id: method/common/Sales/Post\n\
               │ Procedure Post()\n\
               │     Return;\n\
             \n\
             #2 [L] Catalogs/Goods/Ext/ManagerModule.bsl:7-9 :: Find (function)\n\
               │ Function Find()\n\
             \n\
             -- semantic skipped: not configured --\n",
        );

        assert_eq!(section.status, ProviderSectionStatus::Ok);
        assert_eq!(section.hits.len(), 2);
        assert_eq!(
            section.hits[0].path,
            "C:/repo/src/CommonModules/Sales/Ext/Module.bsl"
        );
        assert_eq!(section.hits[0].line, 42);
        assert_eq!(section.hits[0].end_line, Some(58));
        assert_eq!(section.hits[0].attributes["modality"], "L+S");
        assert_eq!(
            section.hits[0].attributes["graphId"],
            "method/common/Sales/Post"
        );
        assert_eq!(section.hits[0].snippet, "Procedure Post()\n    Return;");
        assert_eq!(section.hits[1].rank, 2);
        assert_eq!(section.hits[1].attributes["modality"], "L");
        assert_eq!(
            section.diagnostics,
            vec!["semantic skipped: not configured".to_string()]
        );
    }

    #[test]
    fn bsl_analyzer_parser_normalizes_provider_local_ranks_from_result_order() {
        let section = parse_bsl_analyzer_search(
            "#7 [L] a/Module.bsl:2 :: First (procedure)\n\
               │ Procedure First()\n\
             \n\
             #9 [S] b/Module.bsl:4 :: Second (procedure)\n\
               │ Procedure Second()\n",
        );

        assert_eq!(
            section.hits.iter().map(|hit| hit.rank).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn bsl_analyzer_not_ready_envelope_is_unavailable() {
        let section = parse_bsl_analyzer_search(
            r#"{"status":"not_ready","detail":"indexing 40%","retry_after_ms":1500}"#,
        );

        assert_eq!(section.status, ProviderSectionStatus::Unavailable);
        assert_eq!(section.diagnostics, vec!["indexing 40%".to_string()]);
    }

    #[test]
    fn bsl_analyzer_non_empty_malformed_output_is_failed() {
        let section = parse_bsl_analyzer_search(
            "plain output from an incompatible analyzer\n#broken header\n",
        );

        assert_eq!(section.status, ProviderSectionStatus::Failed);
        assert!(section.hits.is_empty());
        assert!(section.diagnostics[0].contains("without any valid search hits"));
        assert!(section
            .diagnostics
            .iter()
            .any(|item| item.contains("malformed bsl-analyzer search header")));
    }

    #[test]
    fn bsl_analyzer_separator_without_text_does_not_add_empty_diagnostic() {
        let section = parse_bsl_analyzer_search(
            "#1 [L] CommonModules/X/Ext/Module.bsl:2 :: First (procedure)\n\
               │ Procedure First()\n\
             \n\
             ----\n",
        );

        assert_eq!(section.status, ProviderSectionStatus::Ok);
        assert_eq!(section.hits.len(), 1);
        assert!(section
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.is_empty()));
    }

    struct FakeBslClient {
        calls: Mutex<Vec<(PathBuf, Value, Duration)>>,
        output: WorkspaceServiceBslOutput,
    }

    struct FailingBslClient {
        error: String,
    }

    impl BslSearchClient for FailingBslClient {
        fn search(
            &self,
            _context: &CodeIntelligenceContext,
            _arguments: Value,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<WorkspaceServiceBslOutput, String> {
            Err(self.error.clone())
        }
    }

    #[test]
    fn bsl_analyzer_missing_bundled_runtime_is_unavailable() {
        let client = FailingBslClient {
            error: "could not locate Unica plugin root for workspace bsl-analyzer service"
                .to_string(),
        };

        let section = BslAnalyzerProvider::with_client(&client).search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Unavailable);
        assert_eq!(section.diagnostics, vec![client.error]);
    }

    impl BslSearchClient for FakeBslClient {
        fn search(
            &self,
            context: &CodeIntelligenceContext,
            arguments: Value,
            timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<WorkspaceServiceBslOutput, String> {
            self.calls
                .lock()
                .unwrap()
                .push((context.source_root.path.clone(), arguments, timeout));
            Ok(self.output.clone())
        }
    }

    #[test]
    fn bsl_analyzer_provider_uses_persistent_search_tool_with_resolved_context() {
        let client = FakeBslClient {
            calls: Mutex::new(Vec::new()),
            output: WorkspaceServiceBslOutput {
                result_text: "No results found.".to_string(),
                stderr: "building call graph for unrelated modules".to_string(),
            },
        };
        let provider = BslAnalyzerProvider::with_client(&client);

        let section = provider.search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 50,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(120)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Empty);
        assert!(section.diagnostics.is_empty());
        let calls = client.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, PathBuf::from("/workspace/src"));
        assert_eq!(calls[0].1["action"], "search_code");
        assert_eq!(calls[0].1["query"], "Post");
        assert_eq!(calls[0].1["limit"], 50);
        assert!(calls[0].2 <= Duration::from_secs(120));
    }

    #[test]
    fn bsl_analyzer_successful_search_omits_provider_stderr() {
        let client = FakeBslClient {
            calls: Mutex::new(Vec::new()),
            output: WorkspaceServiceBslOutput {
                result_text: "#1 [L] CommonModules/Sales/Ext/Module.bsl:42 :: Post (procedure)\n"
                    .to_string(),
                stderr: "building call graph for unrelated modules\nwarning: unrelated module"
                    .to_string(),
            },
        };

        let section = BslAnalyzerProvider::with_client(&client).search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Ok);
        assert_eq!(section.hits.len(), 1);
        assert!(section.diagnostics.is_empty());
    }

    #[test]
    fn bsl_analyzer_unavailable_search_keeps_provider_stderr() {
        let client = FakeBslClient {
            calls: Mutex::new(Vec::new()),
            output: WorkspaceServiceBslOutput {
                result_text:
                    r#"{"status":"not_ready","detail":"indexing 40%","retry_after_ms":1500}"#
                        .to_string(),
                stderr: "waiting for the BSL index".to_string(),
            },
        };

        let section = BslAnalyzerProvider::with_client(&client).search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Unavailable);
        assert!(section
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("waiting for the BSL index")));
    }

    #[test]
    fn bsl_analyzer_failed_search_keeps_provider_stderr() {
        let client = FakeBslClient {
            calls: Mutex::new(Vec::new()),
            output: WorkspaceServiceBslOutput {
                result_text: "incompatible analyzer output".to_string(),
                stderr: "fatal: graph database is unavailable".to_string(),
            },
        };

        let section = BslAnalyzerProvider::with_client(&client).search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Failed);
        assert!(section
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("fatal: graph database is unavailable")));
    }

    #[test]
    fn rlm_parser_maps_unified_helper_rows_without_cross_provider_scoring() {
        let section = parse_rlm_search(
            r#"[
                {
                    "text": "Post",
                    "source_type": "method",
                    "object_name": "Sales",
                    "path": "CommonModules/Sales/Ext/Module.bsl",
                    "path_kind": "bsl",
                    "detail": {
                        "name": "Post",
                        "type": "procedure",
                        "line": 42,
                        "end_line": 58,
                        "rank": -2.75
                    }
                },
                {
                    "text": "Goods",
                    "source_type": "object",
                    "object_name": "Goods",
                    "path": "Catalogs/Goods.xml",
                    "path_kind": "metadata",
                    "detail": {}
                }
            ]"#,
        );

        assert_eq!(section.status, ProviderSectionStatus::Ok);
        assert_eq!(section.hits.len(), 2);
        assert_eq!(section.hits[0].rank, 1);
        assert_eq!(section.hits[0].provider_score, Some(-2.75));
        assert_eq!(section.hits[0].line, 42);
        assert_eq!(section.hits[0].end_line, Some(58));
        assert_eq!(section.hits[0].symbol.as_deref(), Some("Post"));
        assert_eq!(section.hits[0].kind.as_deref(), Some("procedure"));
        assert_eq!(section.hits[1].rank, 2);
        assert_eq!(section.hits[1].provider_score, None);
        assert_eq!(section.hits[1].line, 1);
    }

    #[test]
    fn rlm_parser_fails_when_every_non_empty_row_is_malformed() {
        let section = parse_rlm_search(
            r#"[
                {"text": "missing path", "detail": {}},
                {"path": "CommonModules/X/Ext/Module.bsl", "detail": {}}
            ]"#,
        );

        assert_eq!(section.status, ProviderSectionStatus::Failed);
        assert!(section.hits.is_empty());
        assert_eq!(section.diagnostics.len(), 3);
        assert!(section.diagnostics[0].contains("no valid rows"));
    }

    #[test]
    fn rlm_parser_keeps_valid_rows_and_reports_malformed_siblings() {
        let section = parse_rlm_search(
            r#"[
                {},
                {
                    "text": "Post",
                    "source_type": "method",
                    "path": "CommonModules/X/Ext/Module.bsl",
                    "detail": {"line": 7}
                }
            ]"#,
        );

        assert_eq!(section.status, ProviderSectionStatus::Ok);
        assert_eq!(section.hits.len(), 1);
        assert_eq!(section.hits[0].rank, 1);
        assert_eq!(section.hits[0].line, 7);
        assert_eq!(section.diagnostics.len(), 1);
    }

    struct FakeRlmClient {
        readiness: IndexReadiness,
        readiness_calls: Mutex<Vec<Duration>>,
        calls: Mutex<Vec<(PathBuf, String, usize, Duration)>>,
        result: String,
    }

    impl RlmSearchClient for FakeRlmClient {
        fn readiness(
            &self,
            _context: &CodeIntelligenceContext,
            timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<IndexReadiness, String> {
            self.readiness_calls.lock().unwrap().push(timeout);
            Ok(self.readiness.clone())
        }

        fn search(
            &self,
            context: &CodeIntelligenceContext,
            query: &str,
            limit: usize,
            timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<String, String> {
            self.calls.lock().unwrap().push((
                context.source_root.path.clone(),
                query.to_string(),
                limit,
                timeout,
            ));
            Ok(self.result.clone())
        }
    }

    struct CancellingRlmSearchClient {
        readiness: IndexReadiness,
        readiness_calls: Mutex<Vec<Duration>>,
        search_calls: Mutex<Vec<Duration>>,
    }

    impl RlmSearchClient for CancellingRlmSearchClient {
        fn readiness(
            &self,
            _context: &CodeIntelligenceContext,
            timeout: Duration,
            cancellation: &CancellationToken,
        ) -> Result<IndexReadiness, String> {
            self.readiness_calls.lock().unwrap().push(timeout);
            cancellation.cancel();
            Ok(self.readiness.clone())
        }

        fn search(
            &self,
            _context: &CodeIntelligenceContext,
            _query: &str,
            _limit: usize,
            timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<String, String> {
            self.search_calls.lock().unwrap().push(timeout);
            Ok("[]".to_string())
        }
    }

    struct DeadlineConsumingRlmSearchClient {
        timeouts: Mutex<Vec<Duration>>,
    }

    impl RlmSearchClient for DeadlineConsumingRlmSearchClient {
        fn readiness(
            &self,
            _context: &CodeIntelligenceContext,
            timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<IndexReadiness, String> {
            self.timeouts.lock().unwrap().push(timeout);
            advance_manual_now(Duration::from_millis(20));
            Ok(IndexReadiness::Ready {
                db_path: PathBuf::from("/cache/index.db"),
            })
        }

        fn search(
            &self,
            _context: &CodeIntelligenceContext,
            _query: &str,
            _limit: usize,
            timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<String, String> {
            self.timeouts.lock().unwrap().push(timeout);
            Ok("[]".to_string())
        }
    }

    struct CancelledReadinessRlmSearchClient;

    impl RlmSearchClient for CancelledReadinessRlmSearchClient {
        fn readiness(
            &self,
            _context: &CodeIntelligenceContext,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<IndexReadiness, String> {
            Err("cancelled: readiness transport stopped".to_string())
        }

        fn search(
            &self,
            _context: &CodeIntelligenceContext,
            _query: &str,
            _limit: usize,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<String, String> {
            panic!("cancelled readiness must stop before RLM search")
        }
    }

    #[test]
    fn rlm_provider_requires_ready_index_and_shares_the_upstream_deadline() {
        let client = FakeRlmClient {
            readiness: IndexReadiness::Ready {
                db_path: PathBuf::from("/cache/index.db"),
            },
            readiness_calls: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
            result: "[]".to_string(),
        };

        let section = RlmProvider::with_client(&client).search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(90)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Empty);
        assert_eq!(client.readiness_calls.lock().unwrap().len(), 1);
        let readiness_timeout = client.readiness_calls.lock().unwrap()[0];
        assert!(readiness_timeout > Duration::from_secs(45));
        assert!(readiness_timeout <= Duration::from_secs(90));
        let calls = client.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, PathBuf::from("/workspace/src"));
        assert_eq!(calls[0].1, "Post");
        assert_eq!(calls[0].2, 20);
        assert!(calls[0].3 > Duration::from_secs(45));
        assert!(calls[0].3 <= Duration::from_secs(90));
    }

    #[test]
    fn rlm_search_readiness_and_search_consume_one_manual_deadline() {
        let started_at = Instant::now();
        set_manual_now(started_at);
        let client = DeadlineConsumingRlmSearchClient {
            timeouts: Mutex::new(Vec::new()),
        };

        let section = RlmProvider::with_client(&client).search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::with_clock(started_at + Duration::from_millis(200), manual_now),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Empty);
        assert_eq!(
            client.timeouts.lock().unwrap().as_slice(),
            &[Duration::from_millis(200), Duration::from_millis(180)]
        );
    }

    #[test]
    fn rlm_search_checks_cancellation_before_interpreting_an_expired_deadline() {
        let started_at = Instant::now();
        set_manual_now(started_at);
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let client = CancellingRlmSearchClient {
            readiness: IndexReadiness::Missing,
            readiness_calls: Mutex::new(Vec::new()),
            search_calls: Mutex::new(Vec::new()),
        };

        let section = RlmProvider::with_client(&client).search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::with_clock(started_at, manual_now),
            &cancellation,
        );

        assert_eq!(section.status, ProviderSectionStatus::Failed);
        assert!(
            section.diagnostics[0].starts_with(CANCELLED_PREFIX),
            "{:?}",
            section.diagnostics
        );
        assert!(client.readiness_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn rlm_search_checks_cancellation_between_readiness_and_search() {
        let started_at = Instant::now();
        set_manual_now(started_at);
        let cancellation = CancellationToken::new();
        let client = CancellingRlmSearchClient {
            readiness: IndexReadiness::Ready {
                db_path: PathBuf::from("/cache/index.db"),
            },
            readiness_calls: Mutex::new(Vec::new()),
            search_calls: Mutex::new(Vec::new()),
        };

        let section = RlmProvider::with_client(&client).search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::with_clock(started_at + Duration::from_secs(1), manual_now),
            &cancellation,
        );

        assert_eq!(section.status, ProviderSectionStatus::Failed);
        assert!(
            section.diagnostics[0].starts_with(CANCELLED_PREFIX),
            "{:?}",
            section.diagnostics
        );
        assert_eq!(client.readiness_calls.lock().unwrap().len(), 1);
        assert!(client.search_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn rlm_search_preserves_prefixed_readiness_cancellation_without_a_set_token() {
        let section = RlmProvider::with_client(&CancelledReadinessRlmSearchClient).search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Failed);
        assert_eq!(
            section.diagnostics,
            vec!["cancelled: readiness transport stopped"]
        );
    }

    #[test]
    fn post_execution_rlm_search_readiness_is_redacted() {
        let error = rlm_search_unready_error(IndexReadiness::Failed(
            "token=top-secret index generation changed".to_string(),
        ));

        assert!(error.starts_with("RLM index became unavailable:"));
        assert!(!error.contains("top-secret"));
    }

    #[test]
    fn rlm_provider_declares_its_search_and_navigation_capabilities() {
        let provider = RlmProvider::new();

        assert_eq!(
            provider.capabilities(),
            &[
                ProviderCapability::Search,
                ProviderCapability::Definition,
                ProviderCapability::ObjectProfile,
            ]
        );
    }

    #[test]
    fn an_outline_names_the_module_it_read_as_an_absolute_artifact() {
        // Every other adapter reports a filesystem artifact as a path a caller
        // can open, so echoing the relative argument back would name no location.
        let root = std::env::temp_dir().join(format!(
            "unica-outline-artifacts-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let source_root = root.join("src");
        let module = source_root.join("CommonModules/X/Ext/Module.bsl");
        std::fs::create_dir_all(module.parent().unwrap()).unwrap();
        std::fs::write(&module, "Процедура П() Экспорт\nКонецПроцедуры\n").unwrap();
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
        let request = CodeIntelligenceReadRequest::Outline {
            path: "CommonModules/X/Ext/Module.bsl".to_string(),
            include_methods: true,
        };

        let outcome = BslAnalyzerProvider::new()
            .read(
                &request,
                &context,
                ProviderDeadline::new(Instant::now() + Duration::from_secs(30)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert!(outcome.ok, "{:?}", outcome.errors);
        assert_eq!(outcome.artifacts.len(), 1);
        let artifact = PathBuf::from(&outcome.artifacts[0]);
        assert!(artifact.is_absolute(), "{artifact:?}");
        assert!(artifact.is_file(), "{artifact:?}");
        assert!(
            artifact.ends_with("CommonModules/X/Ext/Module.bsl"),
            "{artifact:?}"
        );

        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let stopped = BslAnalyzerProvider::new()
            .read(
                &request,
                &context,
                ProviderDeadline::new(Instant::now() + Duration::from_secs(30)),
                &cancelled,
            )
            .unwrap();

        // Same shape as `AdapterOutcome::cancelled`: the prefixed error is the
        // summary and a stopped call claims no artifacts.
        assert!(!stopped.ok);
        assert!(
            stopped.summary.starts_with(CANCELLED_PREFIX),
            "{}",
            stopped.summary
        );
        assert_eq!(stopped.errors, vec![stopped.summary.clone()]);
        assert!(stopped.artifacts.is_empty(), "{:?}", stopped.artifacts);
        assert!(stopped.data.is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unsuccessful_outline_claims_no_module_artifact() {
        let root = std::env::temp_dir().join(format!(
            "unica-outline-failed-artifacts-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let source_root = root.join("src");
        let valid_module = source_root.join("CommonModules/Valid/Ext/Module.bsl");
        let invalid_module = source_root.join("CommonModules/Invalid/Ext/Module.bsl");
        std::fs::create_dir_all(valid_module.parent().unwrap()).unwrap();
        std::fs::create_dir_all(invalid_module.parent().unwrap()).unwrap();
        std::fs::write(
            &valid_module,
            "Процедура Проверить() Экспорт\nКонецПроцедуры\n",
        )
        .unwrap();
        std::fs::write(
            &invalid_module,
            "Процедура Сломана(\nКонецПроцедуры\nЕсли Тогда\n",
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
        let cancellation = CancellationToken::new();
        let cases = [
            (
                "missing module",
                "CommonModules/Missing/Ext/Module.bsl",
                ProviderDeadline::new(Instant::now() + Duration::from_secs(30)),
            ),
            (
                "directory instead of module",
                "CommonModules",
                ProviderDeadline::new(Instant::now() + Duration::from_secs(30)),
            ),
            (
                "parser diagnostic",
                "CommonModules/Invalid/Ext/Module.bsl",
                ProviderDeadline::new(Instant::now() + Duration::from_secs(30)),
            ),
            (
                "deadline before reading",
                "CommonModules/Valid/Ext/Module.bsl",
                ProviderDeadline::new(Instant::now()),
            ),
        ];

        for (label, path, deadline) in cases {
            let outcome = BslAnalyzerProvider::new()
                .read(
                    &CodeIntelligenceReadRequest::Outline {
                        path: path.to_string(),
                        include_methods: true,
                    },
                    &context,
                    deadline,
                    &cancellation,
                )
                .unwrap();

            assert!(!outcome.ok, "{label}: {outcome:?}");
            assert!(
                outcome.artifacts.is_empty(),
                "{label} claimed artifacts: {:?}",
                outcome.artifacts
            );
            assert!(outcome.data.is_none(), "{label}: {:?}", outcome.data);
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_outline_capability_belongs_to_the_current_source_provider() {
        // ADR-0020: the outline is proved from the BSL file on disk, so the
        // index-backed provider must not claim it and the registry must not be
        // able to route it there.
        assert!(!RlmProvider::new()
            .capabilities()
            .contains(&ProviderCapability::Outline));
        assert!(BslAnalyzerProvider::new()
            .capabilities()
            .contains(&ProviderCapability::Outline));

        let registry = CodeIntelligenceRegistry::new(vec![
            Arc::new(RlmProvider::new()) as Arc<dyn CodeIntelligenceProvider>,
            Arc::new(BslAnalyzerProvider::new()),
        ])
        .unwrap();

        assert_eq!(
            registry
                .provider_for(ProviderCapability::Outline)
                .map(|provider| provider.id()),
            Some(ProviderId::BslAnalyzer)
        );
    }

    #[test]
    fn rlm_provider_reports_building_without_opening_session() {
        let client = FakeRlmClient {
            readiness: IndexReadiness::Building,
            readiness_calls: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
            result: "[]".to_string(),
        };

        let section = RlmProvider::with_client(&client).search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(45)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Unavailable);
        assert_eq!(section.diagnostics, vec!["rlm index building".to_string()]);
        assert!(client.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn rlm_provider_redacts_pre_execution_readiness_failures() {
        let client = FakeRlmClient {
            readiness: IndexReadiness::Failed(
                "token=top-secret index generation failed".to_string(),
            ),
            readiness_calls: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
            result: "[]".to_string(),
        };

        let section = RlmProvider::with_client(&client).search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(45)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Unavailable);
        assert!(!section.diagnostics.join(" ").contains("top-secret"));
        assert!(client.calls.lock().unwrap().is_empty());
    }
}
