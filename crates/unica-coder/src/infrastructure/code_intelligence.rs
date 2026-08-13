use crate::domain::cancellation::{cancelled_error, CancellationToken, CANCELLED_PREFIX};
use crate::domain::code_intelligence::{
    CodeIntelligenceContext, CodeIntelligenceProvider, CodeIntelligenceReadData,
    CodeIntelligenceReadRequest, ProviderCapability, ProviderDeadline, ProviderId,
    ProviderProgressUpdate, ProviderReadOutcome, ProviderSearchHit, ProviderSearchSection,
    ProviderSectionStatus, SearchOrdering, SearchProviderPhase, SearchRanking, SearchRequest,
};
use crate::domain::source_location::SourceLocation;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bsl_outline::render_current_source_outline;
use crate::infrastructure::internal_adapters::{
    system_process_runner, ProcessCommand, ProcessRunner, ProcessStreamOutput,
};
use crate::infrastructure::platform::StreamControl;
use crate::infrastructure::redaction::redactor;
use crate::infrastructure::rlm_navigation::RlmNavigationAdapter;
use crate::infrastructure::workspace_index::{read_bsl_index_status, IndexReadiness};
use crate::infrastructure::workspace_services::{
    WorkspaceRlmOperation, WorkspaceServiceBslCall, WorkspaceServiceBslOutput,
    WorkspaceServiceManager, WorkspaceServiceRlmCall,
};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
        let mut args = vec![
            "-c".to_string(),
            "core.quotepath=false".to_string(),
            "grep".to_string(),
            "--no-color".to_string(),
            "--untracked".to_string(),
            "--null".to_string(),
            "-n".to_string(),
            "-F".to_string(),
            "-e".to_string(),
            request.query.clone(),
            "--".to_string(),
        ];
        let scope = context.search_scope.as_ref();
        if let Some(scope) = scope.filter(|scope| !scope.filters.is_empty()) {
            for filter in &scope.filters {
                let path = match filter {
                    crate::domain::code_intelligence::RelativeSearchFilter::Exact(path)
                    | crate::domain::code_intelligence::RelativeSearchFilter::Subtree(path) => path,
                };
                args.push(path.to_string_lossy().replace('\\', "/"));
            }
        } else {
            args.push(".".to_string());
        }
        let command = ProcessCommand {
            program: PathBuf::from("git"),
            args,
            cwd: context.source_root.path.clone(),
            timeout: Some(timeout),
            cancellation: cancellation.clone(),
        };
        let mut hits = Vec::new();
        let mut diagnostics = Vec::new();
        let mut fatal = None;
        let mut locations = SearchLocationProjector::new(context, cancellation);
        let mut consume =
            |line_number: usize, bytes: &[u8]| match parse_git_grep_record(bytes, &mut locations) {
                Ok(hit) => {
                    hits.push(hit);
                    if hits.len() >= request.limit {
                        StreamControl::Stop
                    } else {
                        StreamControl::Continue
                    }
                }
                Err(error) => {
                    diagnostics.push(format!(
                        "ignored malformed git-grep record #{line_number}: {error}"
                    ));
                    StreamControl::Continue
                }
            };
        let output = match self
            .runner
            .run_streaming(&command, 1024 * 1024, &mut consume)
        {
            Ok(output) => output,
            Err(error) => {
                return unavailable_section(
                    ProviderId::GitGrep,
                    format!("failed to start git grep: {error}"),
                );
            }
        };
        if let Some((line, error)) = &output.line_error {
            fatal = Some(format!("git-grep record #{line}: {error}"));
        }
        git_grep_stream_section(output, hits, diagnostics, fatal)
    }
}

impl CodeIntelligenceProvider for GitGrepProvider<'_> {
    fn identity(&self) -> crate::domain::code_intelligence::ProviderIdentity {
        ProviderId::GitGrep.identity()
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
    fn identity(&self) -> crate::domain::code_intelligence::ProviderIdentity {
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
                let mut section =
                    parse_bsl_analyzer_search(&output.result_text, context, cancellation);
                let retain_stderr = match section.status {
                    ProviderSectionStatus::Ok
                    | ProviderSectionStatus::Empty
                    | ProviderSectionStatus::LimitReached
                    | ProviderSectionStatus::TimedOut => false,
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
    fn search(
        &self,
        context: &CodeIntelligenceContext,
        query: &str,
        limit: usize,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<RlmSearchAttempt, String>;
}

enum RlmSearchAttempt {
    Output(String),
    Unready(IndexReadiness),
}

struct WorkspaceRlmSearchClient;

impl RlmSearchClient for WorkspaceRlmSearchClient {
    fn search(
        &self,
        context: &CodeIntelligenceContext,
        query: &str,
        limit: usize,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<RlmSearchAttempt, String> {
        let started_at = std::time::Instant::now();
        let manager = WorkspaceServiceManager::new();
        let mut reported_detail = None;
        let mut backoff = Duration::from_millis(100);
        loop {
            if cancellation.is_cancelled() {
                return Err(cancelled_error("RLM search wait stopped"));
            }
            let Some(remaining) = timeout.checked_sub(started_at.elapsed()) else {
                return Ok(RlmSearchAttempt::Unready(IndexReadiness::Building));
            };
            if remaining.is_zero() {
                return Ok(RlmSearchAttempt::Unready(IndexReadiness::Building));
            }
            let attempt = manager.call_rlm_cancellable(
                &context.workspace,
                &context.source_root.path,
                WorkspaceRlmOperation::Search {
                    query: query.to_string(),
                    limit,
                },
                remaining,
                cancellation,
            )?;
            let readiness = match attempt {
                WorkspaceServiceRlmCall::Output(output) => {
                    return Ok(RlmSearchAttempt::Output(output.result_text));
                }
                WorkspaceServiceRlmCall::Unready(readiness) => readiness,
            };
            if matches!(
                readiness,
                IndexReadiness::Failed(_) | IndexReadiness::Unavailable(_)
            ) {
                return Ok(RlmSearchAttempt::Unready(readiness));
            }
            let detail = rlm_index_progress_detail(&context.workspace);
            if reported_detail.as_deref() != Some(detail) {
                context.report_progress(ProviderProgressUpdate {
                    phase: SearchProviderPhase::Preparing,
                    detail_code: Some(detail.to_string()),
                    results_found: 0,
                });
                reported_detail = Some(detail.to_string());
            }
            let Some(remaining) = timeout.checked_sub(started_at.elapsed()) else {
                return Ok(RlmSearchAttempt::Unready(readiness));
            };
            if remaining.is_zero() {
                return Ok(RlmSearchAttempt::Unready(readiness));
            }
            cancellable_sleep(backoff.min(remaining), cancellation)?;
            backoff = (backoff * 2).min(Duration::from_secs(1));
        }
    }
}

fn rlm_index_progress_detail(context: &WorkspaceContext) -> &'static str {
    let is_update = read_bsl_index_status(context)
        .and_then(|status| status.message)
        .is_some_and(|message| message.contains("update"));
    if is_update {
        "updatingIndex"
    } else {
        "buildingIndex"
    }
}

fn cancellable_sleep(duration: Duration, cancellation: &CancellationToken) -> Result<(), String> {
    let started_at = std::time::Instant::now();
    while started_at.elapsed() < duration {
        if cancellation.is_cancelled() {
            return Err(cancelled_error("RLM search wait stopped"));
        }
        let remaining = duration.saturating_sub(started_at.elapsed());
        std::thread::sleep(remaining.min(Duration::from_millis(50)));
    }
    Ok(())
}

fn rlm_search_unready_error(readiness: IndexReadiness) -> String {
    match readiness {
        IndexReadiness::Ready { .. } => "RLM index readiness changed unexpectedly".to_string(),
        IndexReadiness::Missing => "rlm index is missing; background build requested".to_string(),
        IndexReadiness::Stale { status } => format!(
            "rlm index is stale ({}); background update requested",
            redactor(&status)
        ),
        IndexReadiness::Building => "rlm index building".to_string(),
        IndexReadiness::Failed(error) | IndexReadiness::Unavailable(error) => redactor(&error),
    }
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
    fn identity(&self) -> crate::domain::code_intelligence::ProviderIdentity {
        ProviderId::Rlm.identity()
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
        if has_targeted_search_scope(context) {
            return unavailable_targeted_scope_section(ProviderId::Rlm);
        }
        context.report_progress(ProviderProgressUpdate {
            phase: SearchProviderPhase::Preparing,
            detail_code: Some("reconcilingSources".to_string()),
            results_found: 0,
        });
        let timeout = deadline.remaining();
        if timeout.is_zero() {
            return failed_section(
                ProviderId::Rlm,
                "RLM provider deadline exceeded before search".to_string(),
            );
        }
        context.report_progress(ProviderProgressUpdate {
            phase: SearchProviderPhase::Searching,
            detail_code: Some("executingQuery".to_string()),
            results_found: 0,
        });
        let search_result = self.client.search(
            context,
            &request.query,
            request.limit,
            timeout,
            cancellation,
        );
        if cancellation.is_cancelled() {
            return failed_section(
                ProviderId::Rlm,
                cancelled_error("RLM search stopped after provider operation"),
            );
        }
        let attempt = match search_result {
            Ok(attempt) => attempt,
            Err(error) if error.starts_with(CANCELLED_PREFIX) => {
                return failed_section(ProviderId::Rlm, error);
            }
            Err(error)
                if error.contains("source revision") || is_provider_unavailable_error(&error) =>
            {
                if error.contains("source revision") {
                    context.report_progress(ProviderProgressUpdate {
                        phase: SearchProviderPhase::Preparing,
                        detail_code: Some("sourceRevisionUntrusted".to_string()),
                        results_found: 0,
                    });
                }
                return unavailable_section(ProviderId::Rlm, redactor(&error));
            }
            Err(error) => return failed_section(ProviderId::Rlm, error),
        };
        match attempt {
            RlmSearchAttempt::Output(result) => parse_rlm_search(&result, context, cancellation),
            RlmSearchAttempt::Unready(readiness) => {
                let dependency_detail = match &readiness {
                    IndexReadiness::Missing | IndexReadiness::Building => Some("buildingIndex"),
                    IndexReadiness::Stale { .. } => Some("updatingIndex"),
                    IndexReadiness::Ready { .. }
                    | IndexReadiness::Failed(_)
                    | IndexReadiness::Unavailable(_) => None,
                };
                let diagnostic = rlm_search_unready_error(readiness);
                if diagnostic.contains("source revision") {
                    context.report_progress(ProviderProgressUpdate {
                        phase: SearchProviderPhase::Preparing,
                        detail_code: Some("sourceRevisionUntrusted".to_string()),
                        results_found: 0,
                    });
                }
                if let Some(detail_code) = dependency_detail {
                    ProviderSearchSection::dependency_pending(
                        ProviderId::Rlm.identity(),
                        SearchRanking::None,
                        SearchOrdering::Provider,
                        Vec::new(),
                        vec![diagnostic],
                        detail_code,
                    )
                    .expect("dependency-pending RLM section is valid")
                } else {
                    unavailable_section(ProviderId::Rlm, diagnostic)
                }
            }
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
            provider: ProviderId::Rlm.identity(),
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

fn parse_rlm_search(
    text: &str,
    context: &CodeIntelligenceContext,
    cancellation: &CancellationToken,
) -> ProviderSearchSection {
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
    let mut locations = SearchLocationProjector::new(context, cancellation);
    for (index, row) in rows.iter().enumerate() {
        match parse_rlm_search_row(row, hits.len() + 1, &mut locations) {
            Ok(hit) => hits.push(hit),
            Err(error) => {
                diagnostics.push(format!("ignored malformed RLM result #{index}: {error}"))
            }
        }
    }
    if hits.is_empty() && !rows.is_empty() {
        diagnostics.insert(0, "RLM search helper returned no valid rows".to_string());
        return ProviderSearchSection::failed_with_diagnostics(
            ProviderId::Rlm.identity(),
            diagnostics,
        );
    }
    ProviderSearchSection::complete(
        ProviderId::Rlm.identity(),
        SearchRanking::Provider,
        SearchOrdering::Provider,
        hits,
        diagnostics,
    )
    .unwrap_or_else(|error| ProviderSearchSection::failed(ProviderId::Rlm.identity(), error))
}

fn parse_rlm_search_row(
    row: &Value,
    rank: usize,
    locations: &mut SearchLocationProjector<'_>,
) -> Result<ProviderSearchHit, String> {
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
        rank: Some(rank),
        provider_score: detail.get("rank").and_then(Value::as_f64),
        location: locations.project(Path::new(&path))?,
        line,
        end_line,
        symbol,
        kind: Some(kind),
        snippet: text.to_string(),
        attributes,
    })
}

fn parse_bsl_analyzer_search(
    text: &str,
    context: &CodeIntelligenceContext,
    cancellation: &CancellationToken,
) -> ProviderSearchSection {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed == "No results found." {
        return empty_section(ProviderId::BslAnalyzer, SearchRanking::Provider);
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
    let mut locations = SearchLocationProjector::new(context, cancellation);
    for raw_line in text.lines() {
        let structural = raw_line.trim_start();
        let line = structural.trim_end();
        if line.starts_with('#') {
            if let Some(hit) = current.take() {
                hits.push(hit);
            }
            match parse_bsl_analyzer_header(line, &mut locations) {
                Ok(hit) => current = Some(hit),
                Err(error) => diagnostics.push(format!(
                    "ignored malformed bsl-analyzer search header: {error}"
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
    ProviderSearchSection::complete(
        ProviderId::BslAnalyzer.identity(),
        SearchRanking::Provider,
        SearchOrdering::Provider,
        hits,
        diagnostics,
    )
    .unwrap_or_else(|error| {
        ProviderSearchSection::failed(ProviderId::BslAnalyzer.identity(), error)
    })
}

fn parse_bsl_analyzer_header(
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

fn git_grep_stream_section(
    output: ProcessStreamOutput,
    hits: Vec<ProviderSearchHit>,
    mut diagnostics: Vec<String>,
    fatal: Option<String>,
) -> ProviderSearchSection {
    if output.cancelled {
        return failed_section(
            ProviderId::GitGrep,
            cancelled_error("git-grep search process stopped"),
        );
    }
    if let Some(fatal) = fatal {
        return failed_section(ProviderId::GitGrep, fatal);
    }
    if output.stopped_by_consumer {
        return ProviderSearchSection::limit_reached(
            ProviderId::GitGrep.identity(),
            SearchRanking::None,
            SearchOrdering::ProviderTraversal,
            hits,
            diagnostics,
        )
        .expect("bounded git-grep section is valid");
    }
    if output.timed_out {
        diagnostics.push("git-grep provider deadline exceeded".to_string());
        return ProviderSearchSection::timed_out(
            ProviderId::GitGrep.identity(),
            SearchRanking::None,
            SearchOrdering::ProviderTraversal,
            hits,
            diagnostics,
        )
        .expect("timed-out git-grep section is valid");
    }
    let no_matches = !output.status_success
        && hits.is_empty()
        && output.stderr.trim().is_empty()
        && process_exit_code_is(&output.status, 1);
    if no_matches {
        return empty_section(ProviderId::GitGrep, SearchRanking::None);
    }
    if !output.status_success
        && output.stderr.contains("not a git repository")
        && process_exit_code_is(&output.status, 128)
    {
        return unavailable_section(ProviderId::GitGrep, output.stderr.trim().to_string());
    }
    if !output.status_success {
        let detail = if output.stderr.trim().is_empty() {
            format!("git grep exited with status {}", output.status)
        } else {
            output.stderr.trim().to_string()
        };
        return failed_section(ProviderId::GitGrep, detail);
    }

    let status = if hits.is_empty() {
        if diagnostics.is_empty() {
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
    match status {
        ProviderSectionStatus::Ok | ProviderSectionStatus::Empty => {
            ProviderSearchSection::complete(
                ProviderId::GitGrep.identity(),
                SearchRanking::None,
                SearchOrdering::ProviderTraversal,
                hits,
                diagnostics,
            )
            .unwrap_or_else(|error| {
                ProviderSearchSection::failed(ProviderId::GitGrep.identity(), error)
            })
        }
        _ => ProviderSearchSection::failed(ProviderId::GitGrep.identity(), diagnostics.join("; ")),
    }
}

fn parse_git_grep_record(
    record: &[u8],
    locations: &mut SearchLocationProjector<'_>,
) -> Result<ProviderSearchHit, String> {
    let mut parts = record.splitn(3, |byte| *byte == 0);
    let path = std::str::from_utf8(parts.next().ok_or("path is missing")?)
        .map_err(|_| "path is not UTF-8")?
        .trim();
    let line_number = std::str::from_utf8(parts.next().ok_or("line is missing")?)
        .map_err(|_| "line is not UTF-8")?
        .parse::<usize>()
        .map_err(|_| "line is not a positive integer")?;
    let snippet = std::str::from_utf8(parts.next().ok_or("snippet is missing")?)
        .map_err(|_| "snippet is not UTF-8")?
        .trim_end();
    if path.is_empty() {
        return Err("path is empty".to_string());
    }
    let relative = PathBuf::from(path.replace('\\', "/"));
    Ok(ProviderSearchHit {
        rank: None,
        provider_score: None,
        location: locations.project(&relative)?,
        line: line_number,
        end_line: None,
        symbol: None,
        kind: None,
        snippet: snippet.to_string(),
        attributes: Map::new(),
    })
}

type SearchLocationResolver =
    fn(
        &WorkspaceContext,
        &crate::application::source_navigation::SourceLocateRequest,
        &CancellationToken,
    ) -> Result<crate::application::source_navigation::SourceLocateResult, String>;

struct SearchLocationProjector<'a> {
    context: &'a CodeIntelligenceContext,
    cancellation: &'a CancellationToken,
    resolver: SearchLocationResolver,
    cache: HashMap<PathBuf, SourceLocation>,
}

impl<'a> SearchLocationProjector<'a> {
    fn new(context: &'a CodeIntelligenceContext, cancellation: &'a CancellationToken) -> Self {
        Self::with_resolver(
            context,
            cancellation,
            crate::infrastructure::platform_xml_source_targets::locate_platform_xml_source_path,
        )
    }

    fn with_resolver(
        context: &'a CodeIntelligenceContext,
        cancellation: &'a CancellationToken,
        resolver: SearchLocationResolver,
    ) -> Self {
        Self {
            context,
            cancellation,
            resolver,
            cache: HashMap::new(),
        }
    }

    fn project(&mut self, provider_path: &Path) -> Result<SourceLocation, String> {
        if self.cancellation.is_cancelled() {
            return Err(cancelled_error("search result location projection stopped"));
        }
        let relative_path = contained_provider_relative_path(self.context, provider_path)?;
        if self
            .context
            .search_scope
            .as_ref()
            .is_some_and(|scope| !scope.accepts(&relative_path))
        {
            return Err("provider path is outside the logical search scope".to_string());
        }
        if let Some(location) = self.cache.get(&relative_path) {
            return Ok(location.clone());
        }

        let source_set = self
            .context
            .source_root
            .source_set
            .clone()
            .unwrap_or_else(|| "legacy".to_string());
        let relative_text = relative_path.to_string_lossy().replace('\\', "/");
        let request = crate::application::source_navigation::SourceLocateRequest {
            source_set: source_set.clone(),
            path: relative_text.clone(),
        };
        let location = match (self.resolver)(&self.context.workspace, &request, self.cancellation) {
            Ok(located) if located.rejection.is_none() => SourceLocation::Addressed {
                source_set,
                metadata_path: located.metadata_path,
                target_kind: located
                    .target_kind
                    .unwrap_or(crate::domain::source_target::TargetKind::SourceRoot),
            },
            Ok(located)
                if located.rejection
                    == Some(crate::domain::source_location::LocateRejection::OutsideSourceSet) =>
            {
                return Err("provider path is outside sourceSet".to_string());
            }
            Ok(located) => SourceLocation::Unaddressable {
                source_set,
                owner_metadata_path: located.owner_metadata_path,
                path: located.relative_path,
            },
            Err(error) if error.starts_with(CANCELLED_PREFIX) => return Err(error),
            Err(_) => SourceLocation::Unaddressable {
                source_set,
                owner_metadata_path: None,
                path: relative_text,
            },
        };
        self.cache.insert(relative_path, location.clone());
        Ok(location)
    }
}

fn contained_provider_relative_path(
    context: &CodeIntelligenceContext,
    provider_path: &Path,
) -> Result<PathBuf, String> {
    let raw_text = provider_path.to_string_lossy().replace('\\', "/");
    if raw_text.is_empty()
        || crate::infrastructure::platform::filesystem::is_foreign_absolute_path(&raw_text)
    {
        return Err("provider path is outside sourceSet".to_string());
    }
    let normalized_root =
        crate::infrastructure::source_roots::normalize_path_identity(&context.source_root.path)
            .map_err(|_| "sourceSet path identity is unavailable".to_string())?;
    let raw = Path::new(&raw_text);
    let candidates = if raw.is_absolute() {
        vec![raw.to_path_buf()]
    } else {
        vec![
            context.workspace.workspace_root.join(raw),
            context.source_root.path.join(raw),
        ]
    };
    for candidate in candidates {
        let Ok(normalized) =
            crate::infrastructure::source_roots::normalize_path_identity(&candidate)
        else {
            continue;
        };
        let Ok(relative) = normalized.strip_prefix(&normalized_root) else {
            continue;
        };
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            continue;
        }
        return Ok(relative.to_path_buf());
    }
    Err("provider path is outside sourceSet".to_string())
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

fn empty_section(provider: ProviderId, ranking: SearchRanking) -> ProviderSearchSection {
    ProviderSearchSection::complete(
        provider.identity(),
        ranking,
        if ranking == SearchRanking::None {
            SearchOrdering::ProviderTraversal
        } else {
            SearchOrdering::Provider
        },
        Vec::new(),
        Vec::new(),
    )
    .expect("empty section is valid")
}

fn unavailable_section(provider: ProviderId, diagnostic: String) -> ProviderSearchSection {
    ProviderSearchSection::unavailable(provider.identity(), diagnostic)
}

fn has_targeted_search_scope(context: &CodeIntelligenceContext) -> bool {
    context
        .search_scope
        .as_ref()
        .is_some_and(|scope| !scope.filters.is_empty())
}

fn unavailable_targeted_scope_section(provider: ProviderId) -> ProviderSearchSection {
    ProviderSearchSection::unsupported_scope(
        provider.identity(),
        format!(
            "{} cannot constrain search to metadataPath; the role was not searched outside the requested logical scope",
            provider.as_str()
        ),
    )
}

fn failed_section(provider: ProviderId, diagnostic: String) -> ProviderSearchSection {
    ProviderSearchSection::failed(provider.identity(), diagnostic)
}

#[cfg(test)]
fn location_path(location: &SourceLocation) -> &str {
    match location {
        SourceLocation::Unaddressable { path, .. } => path,
        SourceLocation::Addressed { .. } => "",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        location_path, rlm_search_unready_error, BslAnalyzerProvider, BslSearchClient,
        GitGrepProvider, RlmProvider, RlmSearchAttempt, RlmSearchClient,
    };
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::cancellation::CANCELLED_PREFIX;
    use crate::domain::code_intelligence::{
        CodeIntelligenceContext, CodeIntelligenceProvider, CodeIntelligenceReadRequest,
        CodeIntelligenceRegistry, CodeSearchScope, ProviderCapability, ProviderDeadline,
        ProviderId, ProviderSectionStatus, RelativeSearchFilter, SearchRequest,
    };
    use crate::domain::source_location::SourceLocation;
    use crate::domain::source_roots::ResolvedSourceRoot;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::internal_adapters::{ProcessCommand, ProcessOutput, ProcessRunner};
    use crate::infrastructure::workspace_index::IndexReadiness;
    use crate::infrastructure::workspace_services::WorkspaceServiceBslOutput;
    use serde_json::{json, Value};
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
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

    fn parse_bsl_analyzer_search(
        text: &str,
        context: &CodeIntelligenceContext,
    ) -> crate::domain::code_intelligence::ProviderSearchSection {
        super::parse_bsl_analyzer_search(text, context, &CancellationToken::new())
    }

    fn parse_rlm_search(
        text: &str,
        context: &CodeIntelligenceContext,
    ) -> crate::domain::code_intelligence::ProviderSearchSection {
        super::parse_rlm_search(text, context, &CancellationToken::new())
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

    fn metadata_scoped_context() -> CodeIntelligenceContext {
        context().with_search_scope(CodeSearchScope {
            source_set: "main".to_string(),
            source_root: PathBuf::from("/workspace/src"),
            filters: vec![RelativeSearchFilter::Exact(PathBuf::from(
                "CommonModules/Scoped/Ext/Module.bsl",
            ))],
            legacy_selector: false,
        })
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

    fn assert_sales_module_location(location: &SourceLocation) {
        let SourceLocation::Addressed {
            source_set,
            metadata_path,
            target_kind,
        } = location
        else {
            panic!("expected an addressed search location, got {location:?}");
        };
        assert_eq!(source_set, "main");
        assert_eq!(
            metadata_path.as_ref().map(|path| path.as_str()),
            Some("CommonModule.Sales.Module")
        );
        assert_eq!(
            *target_kind,
            crate::domain::source_target::TargetKind::Module
        );
    }

    #[test]
    fn every_search_provider_uses_the_same_logical_location_projection() {
        let temporary = tempfile::tempdir().unwrap();
        let workspace_root = temporary.path().join("workspace");
        let source_root = workspace_root.join("src");
        let descriptor = source_root.join("CommonModules/Sales.xml");
        let module = source_root.join("CommonModules/Sales/Ext/Module.bsl");
        std::fs::create_dir_all(module.parent().unwrap()).unwrap();
        std::fs::write(
            workspace_root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>Sales</Name></Properties></CommonModule></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(&module, "Procedure Post()\nEndProcedure\n").unwrap();
        let context = CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: workspace_root.clone(),
                workspace_root: workspace_root.clone(),
                cache_root: workspace_root.join(".build/unica"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: source_root,
            },
        );

        let rlm = parse_rlm_search(
            r#"[{"text":"Post","source_type":"method","path":"CommonModules/Sales/Ext/Module.bsl","detail":{"line":1}}]"#,
            &context,
        );
        let analyzer = parse_bsl_analyzer_search(
            "#1 [L] CommonModules/Sales/Ext/Module.bsl:1 :: Post (procedure)\n",
            &context,
        );
        let runner = FakeRunner {
            output: output("CommonModules/Sales/Ext/Module.bsl\x001\x00Procedure Post()\n"),
            commands: Mutex::new(Vec::new()),
        };
        let lexical = GitGrepProvider::with_runner(&runner).search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context,
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        for section in [&rlm, &analyzer, &lexical] {
            assert_eq!(section.status, ProviderSectionStatus::Ok);
            assert_sales_module_location(&section.hits[0].location);
        }
    }

    #[test]
    fn search_location_projection_is_cached_per_source_file() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static RESOLUTIONS: AtomicUsize = AtomicUsize::new(0);

        fn resolve_for_test(
            _workspace: &WorkspaceContext,
            request: &crate::application::source_navigation::SourceLocateRequest,
            _cancellation: &CancellationToken,
        ) -> Result<crate::application::source_navigation::SourceLocateResult, String> {
            RESOLUTIONS.fetch_add(1, Ordering::SeqCst);
            Ok(crate::application::source_navigation::SourceLocateResult {
                source_set: request.source_set.clone(),
                relative_path: request.path.clone(),
                metadata_path: None,
                target_kind: None,
                owner_metadata_path: None,
                rejection: Some(crate::domain::source_location::LocateRejection::NotAddressable),
            })
        }

        RESOLUTIONS.store(0, Ordering::SeqCst);
        let temporary = tempfile::tempdir().unwrap();
        let workspace_root = temporary.path().join("workspace");
        let source_root = workspace_root.join("src");
        let first = source_root.join("Catalogs/Items/Ext/ObjectModule.bsl");
        let second = source_root.join("Catalogs/Items/Ext/ManagerModule.bsl");
        std::fs::create_dir_all(first.parent().unwrap()).unwrap();
        std::fs::write(&first, "first").unwrap();
        std::fs::write(&second, "second").unwrap();
        let context = CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: workspace_root.clone(),
                workspace_root: workspace_root.clone(),
                cache_root: workspace_root.join(".build/unica"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: source_root,
            },
        );
        let cancellation = CancellationToken::new();
        let mut locations = super::SearchLocationProjector::with_resolver(
            &context,
            &cancellation,
            resolve_for_test,
        );

        locations
            .project(Path::new("Catalogs/Items/Ext/ObjectModule.bsl"))
            .unwrap();
        locations
            .project(Path::new("Catalogs/Items/Ext/ObjectModule.bsl"))
            .unwrap();
        assert_eq!(RESOLUTIONS.load(Ordering::SeqCst), 1);

        locations
            .project(Path::new("Catalogs/Items/Ext/ManagerModule.bsl"))
            .unwrap();
        assert_eq!(
            RESOLUTIONS.load(Ordering::SeqCst),
            2,
            "different modules in one parent directory must not share an address"
        );

        cancellation.cancel();
        let error = locations
            .project(Path::new("Catalogs/Items/Ext/ObjectModule.bsl"))
            .expect_err("cancellation must win even over a cached location");
        assert!(error.starts_with(CANCELLED_PREFIX), "{error}");
        assert_eq!(RESOLUTIONS.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn git_grep_is_literal_source_scoped_and_uses_the_upstream_deadline() {
        let runner = FakeRunner {
            output: output("CommonModules/Sales/Ext/Module.bsl\x004\x00Post();\n"),
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

        assert_eq!(section.identity, ProviderId::GitGrep.identity());
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
                "--null",
                "-n",
                "-F",
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
    fn git_grep_returns_the_first_unranked_provider_traversal_prefix() {
        let runner = FakeRunner {
            output: output(
                "b/Module.bsl\x009\x00Second\n\
                 a/Module.bsl\x007\x00Later\n\
                 a/Module.bsl\x002\x00First\n",
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

        assert_eq!(section.status, ProviderSectionStatus::LimitReached);
        assert_eq!(section.hits.len(), 2);
        assert_eq!(section.hits[0].rank, None);
        assert_eq!(location_path(&section.hits[0].location), "b/Module.bsl");
        assert_eq!(section.hits[0].line, 9);
        assert_eq!(section.hits[1].rank, None);
        assert_eq!(location_path(&section.hits[1].location), "a/Module.bsl");
        assert_eq!(section.hits[1].line, 7);
        assert!(section.hits.iter().all(|hit| hit.provider_score.is_none()));
    }

    #[test]
    fn git_grep_preserves_utf8_paths() {
        let runner = FakeRunner {
            output: output("Catalogs/Номенклатура.xml\x007\x00<Name>Номенклатура</Name>\n"),
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
        assert_eq!(
            location_path(&section.hits[0].location),
            "Catalogs/Номенклатура.xml"
        );
    }

    #[test]
    fn git_grep_timeout_preserves_already_streamed_hits_as_a_lower_bound() {
        let runner = FakeRunner {
            output: ProcessOutput {
                status_success: false,
                status: "timeout".to_string(),
                stdout: "a/Module.bsl\x002\x00First\nb/Module.bsl\x009\x00Second\n".to_string(),
                stderr: String::new(),
                timed_out: true,
                cancelled: false,
                stdout_truncated: false,
            },
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

        assert_eq!(section.status, ProviderSectionStatus::TimedOut);
        assert_eq!(section.hits.len(), 2);
        assert!(!section.search_complete);
        assert_eq!(
            section.matches.relation,
            crate::domain::code_intelligence::SearchCountRelation::LowerBound
        );
    }

    #[test]
    fn git_grep_keeps_every_row_when_the_capture_was_not_truncated() {
        let runner = FakeRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: concat!(
                    "CommonModules/Other/Ext/Module.bsl\x0012\x00Procedure Other()\n",
                    "CommonModules/Test/Ext/Module.bsl\x001\x00Procedure Test()\n"
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
    fn git_grep_does_not_publish_a_parent_escape_as_a_location() {
        let runner = FakeRunner {
            output: output("../outside/Secret.bsl\x002\x00Password = 1;\n"),
            commands: Mutex::new(Vec::new()),
        };

        let section = GitGrepProvider::with_runner(&runner).search(
            &SearchRequest {
                query: "Password".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Failed);
        assert!(section.hits.is_empty());
        assert!(!serde_json::to_string(&section)
            .unwrap()
            .contains("Secret.bsl"));
    }

    #[test]
    fn bsl_analyzer_ranked_text_parser_projects_absolute_paths_inside_source_set() {
        let section = parse_bsl_analyzer_search(
            "#1 [L+S] /workspace/src/CommonModules/Sales/Ext/Module.bsl:42-58 :: Post (procedure)\n\
               graph_id: method/common/Sales/Post\n\
               │ Procedure Post()\n\
               │     Return;\n\
             \n\
             #2 [L] Catalogs/Goods/Ext/ManagerModule.bsl:7-9 :: Find (function)\n\
               │ Function Find()\n\
             \n\
             -- semantic skipped: not configured --\n",
            &context(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Ok);
        assert_eq!(section.hits.len(), 2);
        assert_eq!(
            location_path(&section.hits[0].location),
            "CommonModules/Sales/Ext/Module.bsl"
        );
        assert_eq!(section.hits[0].line, 42);
        assert_eq!(section.hits[0].end_line, Some(58));
        assert_eq!(section.hits[0].attributes["modality"], "L+S");
        assert_eq!(
            section.hits[0].attributes["graphId"],
            "method/common/Sales/Post"
        );
        assert_eq!(section.hits[0].snippet, "Procedure Post()\n    Return;");
        assert_eq!(section.hits[1].rank, Some(2));
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
            &context(),
        );

        assert_eq!(
            section.hits.iter().map(|hit| hit.rank).collect::<Vec<_>>(),
            vec![Some(1), Some(2)]
        );
    }

    #[test]
    fn bsl_analyzer_does_not_publish_an_absolute_path_outside_source_set() {
        let section = parse_bsl_analyzer_search(
            "#1 [L] /outside/Secret.bsl:2 :: Password (variable)\n\
               │ Password = 1;\n",
            &context(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Failed);
        assert!(section.hits.is_empty());
        assert!(!serde_json::to_string(&section)
            .unwrap()
            .contains("Secret.bsl"));
    }

    #[test]
    fn bsl_analyzer_not_ready_envelope_is_unavailable() {
        let section = parse_bsl_analyzer_search(
            r#"{"status":"not_ready","detail":"indexing 40%","retry_after_ms":1500}"#,
            &context(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Unavailable);
        assert_eq!(section.diagnostics, vec!["indexing 40%".to_string()]);
    }

    #[test]
    fn bsl_analyzer_non_empty_malformed_output_is_failed() {
        let section = parse_bsl_analyzer_search(
            "plain output from an incompatible analyzer\n#broken header\n",
            &context(),
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
            &context(),
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
    fn bsl_analyzer_does_not_broaden_metadata_scoped_search() {
        let client = FakeBslClient {
            calls: Mutex::new(Vec::new()),
            output: WorkspaceServiceBslOutput {
                result_text: "No results found.".to_string(),
                stderr: String::new(),
            },
        };

        let section = BslAnalyzerProvider::with_client(&client).search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &metadata_scoped_context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Unavailable);
        assert!(section.diagnostics.join(" ").contains("metadataPath"));
        assert!(client.calls.lock().unwrap().is_empty());
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
            &context(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Ok);
        assert_eq!(section.hits.len(), 2);
        assert_eq!(section.hits[0].rank, Some(1));
        assert_eq!(section.hits[0].provider_score, Some(-2.75));
        assert_eq!(section.hits[0].line, 42);
        assert_eq!(section.hits[0].end_line, Some(58));
        assert_eq!(section.hits[0].symbol.as_deref(), Some("Post"));
        assert_eq!(section.hits[0].kind.as_deref(), Some("procedure"));
        assert_eq!(section.hits[1].rank, Some(2));
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
            &context(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Failed);
        assert!(section.hits.is_empty());
        assert_eq!(section.diagnostics.len(), 3);
        assert!(section.diagnostics[0].contains("no valid rows"));
    }

    #[test]
    fn rlm_does_not_publish_an_absolute_path_outside_source_set() {
        let section = parse_rlm_search(
            r#"[{
                "text": "Password",
                "source_type": "variable",
                "path": "/outside/Secret.bsl",
                "detail": {"line": 2}
            }]"#,
            &context(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Failed);
        assert!(section.hits.is_empty());
        assert!(!serde_json::to_string(&section)
            .unwrap()
            .contains("Secret.bsl"));
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
            &context(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Ok);
        assert_eq!(section.hits.len(), 1);
        assert_eq!(section.hits[0].rank, Some(1));
        assert_eq!(section.hits[0].line, 7);
        assert_eq!(section.diagnostics.len(), 1);
    }

    struct FakeRlmClient {
        readiness: IndexReadiness,
        calls: Mutex<Vec<(PathBuf, String, usize, Duration)>>,
        result: String,
    }

    impl RlmSearchClient for FakeRlmClient {
        fn search(
            &self,
            context: &CodeIntelligenceContext,
            query: &str,
            limit: usize,
            timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<RlmSearchAttempt, String> {
            self.calls.lock().unwrap().push((
                context.source_root.path.clone(),
                query.to_string(),
                limit,
                timeout,
            ));
            match &self.readiness {
                IndexReadiness::Ready { .. } => Ok(RlmSearchAttempt::Output(self.result.clone())),
                readiness => Ok(RlmSearchAttempt::Unready(readiness.clone())),
            }
        }
    }

    struct CancellingRlmSearchClient {
        readiness: IndexReadiness,
        search_calls: Mutex<Vec<Duration>>,
    }

    impl RlmSearchClient for CancellingRlmSearchClient {
        fn search(
            &self,
            _context: &CodeIntelligenceContext,
            _query: &str,
            _limit: usize,
            timeout: Duration,
            cancellation: &CancellationToken,
        ) -> Result<RlmSearchAttempt, String> {
            self.search_calls.lock().unwrap().push(timeout);
            cancellation.cancel();
            match &self.readiness {
                IndexReadiness::Ready { .. } => Ok(RlmSearchAttempt::Output("[]".to_string())),
                readiness => Ok(RlmSearchAttempt::Unready(readiness.clone())),
            }
        }
    }

    struct DeadlineRecordingRlmSearchClient {
        timeouts: Mutex<Vec<Duration>>,
    }

    impl RlmSearchClient for DeadlineRecordingRlmSearchClient {
        fn search(
            &self,
            _context: &CodeIntelligenceContext,
            _query: &str,
            _limit: usize,
            timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<RlmSearchAttempt, String> {
            self.timeouts.lock().unwrap().push(timeout);
            Ok(RlmSearchAttempt::Output("[]".to_string()))
        }
    }

    struct CancelledRlmSearchClient;

    impl RlmSearchClient for CancelledRlmSearchClient {
        fn search(
            &self,
            _context: &CodeIntelligenceContext,
            _query: &str,
            _limit: usize,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<RlmSearchAttempt, String> {
            Err("cancelled: readiness transport stopped".to_string())
        }
    }

    #[test]
    fn rlm_provider_delegates_readiness_and_execution_to_one_client_operation() {
        let client = FakeRlmClient {
            readiness: IndexReadiness::Ready {
                db_path: PathBuf::from("/cache/index.db"),
            },
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
        let calls = client.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, PathBuf::from("/workspace/src"));
        assert_eq!(calls[0].1, "Post");
        assert_eq!(calls[0].2, 20);
        assert!(calls[0].3 > Duration::from_secs(45));
        assert!(calls[0].3 <= Duration::from_secs(90));
    }

    #[test]
    fn rlm_does_not_broaden_metadata_scoped_search() {
        let client = FakeRlmClient {
            readiness: IndexReadiness::Ready {
                db_path: PathBuf::from("/cache/index.db"),
            },
            calls: Mutex::new(Vec::new()),
            result: "[]".to_string(),
        };

        let section = RlmProvider::with_client(&client).search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &metadata_scoped_context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Unavailable);
        assert!(section.diagnostics.join(" ").contains("metadataPath"));
        assert!(client.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn rlm_search_passes_the_single_remaining_deadline_to_the_client() {
        let started_at = Instant::now();
        set_manual_now(started_at);
        let client = DeadlineRecordingRlmSearchClient {
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
            &[Duration::from_millis(200)]
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
        assert!(client.search_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn rlm_search_checks_cancellation_after_the_client_operation() {
        let started_at = Instant::now();
        set_manual_now(started_at);
        let cancellation = CancellationToken::new();
        let client = CancellingRlmSearchClient {
            readiness: IndexReadiness::Ready {
                db_path: PathBuf::from("/cache/index.db"),
            },
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
        assert_eq!(client.search_calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn rlm_search_preserves_prefixed_client_cancellation_without_a_set_token() {
        let section = RlmProvider::with_client(&CancelledRlmSearchClient).search(
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
    fn rlm_search_unready_diagnostic_is_redacted() {
        let error = rlm_search_unready_error(IndexReadiness::Failed(
            "token=top-secret index generation changed".to_string(),
        ));

        assert_eq!(error, "token=<redacted>");
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
                .map(|provider| provider.identity()),
            Some(ProviderId::BslAnalyzer.identity())
        );
    }

    #[test]
    fn rlm_provider_reports_building_without_opening_session() {
        let client = FakeRlmClient {
            readiness: IndexReadiness::Building,
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

        assert_eq!(section.status, ProviderSectionStatus::TimedOut);
        assert_eq!(section.diagnostics, vec!["rlm index building".to_string()]);
        assert_eq!(
            serde_json::to_value(&section).unwrap()["termination"],
            json!({
                "code": "dependencyPending",
                "retryable": true,
                "detailCode": "buildingIndex"
            })
        );
        assert_eq!(client.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn rlm_provider_reports_pending_update_as_a_retryable_dependency() {
        let client = FakeRlmClient {
            readiness: IndexReadiness::Stale {
                status: "source revision changed".to_string(),
            },
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

        assert_eq!(section.status, ProviderSectionStatus::TimedOut);
        assert_eq!(
            serde_json::to_value(&section).unwrap()["termination"],
            json!({
                "code": "dependencyPending",
                "retryable": true,
                "detailCode": "updatingIndex"
            })
        );
    }

    #[test]
    fn rlm_provider_redacts_unready_failures() {
        let client = FakeRlmClient {
            readiness: IndexReadiness::Failed(
                "token=top-secret index generation failed".to_string(),
            ),
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
        assert_eq!(client.calls.lock().unwrap().len(), 1);
    }
}
