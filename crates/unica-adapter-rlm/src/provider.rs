use crate::client::RlmServiceClient;
use crate::index::{IndexMaintenanceStatusExt, IndexReadiness};
use crate::navigation::RlmNavigationAdapter;
use crate::session::{WorkspaceServiceRlmCall, WorkspaceServiceRlmOutput};
use serde_json::{Map, Value};
use std::path::Path;
use std::time::Duration;
use unica_analysis_services::{protocol::WorkspaceRlmOperation, WorkspaceServiceManager};
use unica_code_intelligence::model::*;
use unica_code_intelligence::provider_sections::*;
use unica_code_intelligence::source_locations::{SearchLocationProjector, SearchSourceBindings};
use unica_redaction::redactor;
use unica_work_control::cancellation::{cancelled_error, CancellationToken, CANCELLED_PREFIX};
use unica_work_control::provider_deadline::ProviderDeadline;
const RLM_CAPABILITIES: &[ProviderCapability] = &[
    ProviderCapability::Search,
    ProviderCapability::Definition,
    ProviderCapability::ObjectProfile,
];

pub trait RlmSearchClient: Send + Sync {
    fn search(
        &self,
        context: &CodeIntelligenceContext,
        query: &str,
        limit: usize,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<RlmSearchAttempt, String>;
}

pub enum RlmSearchAttempt {
    Output(WorkspaceServiceRlmOutput),
    Unready(IndexReadiness),
    DeadlineExceeded,
}

pub struct WorkspaceRlmSearchClient {
    pub environment:
        fn() -> std::sync::Arc<dyn unica_analysis_services::backend::ServiceEnvironment>,
}

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
        let manager = WorkspaceServiceManager::new((self.environment)());
        wait_for_rlm_search(
            context,
            timeout,
            cancellation,
            || started_at.elapsed(),
            |remaining| {
                manager.call_rlm_cancellable(
                    &context.workspace,
                    &context.source_root.path,
                    WorkspaceRlmOperation::Search {
                        query: query.to_string(),
                        limit,
                    },
                    remaining,
                    cancellation,
                )
            },
            |duration| cancellable_sleep(duration, cancellation),
        )
    }
}

pub fn wait_for_rlm_search(
    context: &CodeIntelligenceContext,
    timeout: Duration,
    cancellation: &CancellationToken,
    mut elapsed: impl FnMut() -> Duration,
    mut call: impl FnMut(Duration) -> Result<WorkspaceServiceRlmCall, String>,
    mut wait: impl FnMut(Duration) -> Result<(), String>,
) -> Result<RlmSearchAttempt, String> {
    let mut reported_detail = None;
    let mut last_observed = None;
    let mut backoff = Duration::from_millis(100);
    loop {
        if cancellation.is_cancelled() {
            return Err(cancelled_error("RLM search wait stopped"));
        }
        let Some(remaining) = timeout
            .checked_sub(elapsed())
            .filter(|remaining| !remaining.is_zero())
        else {
            return Ok(last_observed.map_or(
                RlmSearchAttempt::DeadlineExceeded,
                RlmSearchAttempt::Unready,
            ));
        };
        let attempt = call(remaining)?;
        let readiness = match attempt {
            WorkspaceServiceRlmCall::Output(output) => {
                return Ok(RlmSearchAttempt::Output(output));
            }
            WorkspaceServiceRlmCall::Unready(readiness) => readiness,
        };
        if !matches!(readiness, IndexReadiness::Queued | IndexReadiness::Building) {
            return Ok(RlmSearchAttempt::Unready(readiness));
        }
        let detail = rlm_index_progress_detail(&readiness);
        if reported_detail.as_deref() != Some(detail) {
            context.report_progress(ProviderProgressUpdate {
                phase: SearchProviderPhase::Preparing,
                detail_code: Some(detail.to_string()),
                results_found: 0,
            });
            reported_detail = Some(detail.to_string());
        }
        let remaining = timeout.saturating_sub(elapsed());
        if remaining.is_zero() {
            return Ok(RlmSearchAttempt::Unready(readiness));
        }
        last_observed = Some(readiness);
        wait(backoff.min(remaining))?;
        backoff = (backoff * 2).min(Duration::from_secs(1));
    }
}

pub fn rlm_index_progress_detail(readiness: &IndexReadiness) -> &'static str {
    match readiness {
        IndexReadiness::Queued => "indexQueued",
        IndexReadiness::Building => "buildingIndex",
        _ => "indexStatusUnknown",
    }
}

pub fn cancellable_sleep(
    duration: Duration,
    cancellation: &CancellationToken,
) -> Result<(), String> {
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

pub fn rlm_search_unready_error(readiness: IndexReadiness) -> String {
    match readiness {
        IndexReadiness::Ready { .. } => "RLM index readiness changed unexpectedly".to_string(),
        IndexReadiness::Missing => "rlm index is missing; maintenance state is unknown".to_string(),
        IndexReadiness::Stale { status } => format!(
            "rlm index is stale ({}); maintenance state is unknown",
            redactor(&status)
        ),
        IndexReadiness::Building => "rlm index building".to_string(),
        IndexReadiness::Queued => "rlm index maintenance queued".to_string(),
        IndexReadiness::Incomplete => {
            "rlm index is incomplete; maintenance state is unknown".to_string()
        }
        IndexReadiness::MaintenanceUnavailable(evidence) => evidence.summary().to_string(),
        IndexReadiness::Failed(error) | IndexReadiness::Unavailable(error) => redactor(&error),
    }
}

pub struct RlmProvider<'a> {
    navigation: &'a dyn crate::navigation::RlmNavigationClient,
    bindings: SearchSourceBindings,
    client: &'a (dyn RlmSearchClient + Send + Sync),
}

impl<'a> RlmProvider<'a> {
    pub fn new(
        client: &'a (dyn RlmSearchClient + Send + Sync),
        navigation: &'a dyn crate::navigation::RlmNavigationClient,
        bindings: SearchSourceBindings,
    ) -> Self {
        Self {
            client,
            navigation,
            bindings,
        }
    }
}

impl CodeIntelligenceProvider for RlmProvider<'_> {
    fn identity(&self) -> unica_code_intelligence::model::ProviderIdentity {
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
        let timeout = match deadline.finite_remaining() {
            Ok(timeout) => timeout,
            Err(error) => return unavailable_section(ProviderId::Rlm, error.to_string()),
        };
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
            Err(error) if (self.bindings.engine_absent)(&context.workspace.cwd, "rlm-bsl-mcp") => {
                return unavailable_section(ProviderId::Rlm, redactor(&error));
            }
            Err(error) => return failed_section(ProviderId::Rlm, error),
        };
        match attempt {
            RlmSearchAttempt::DeadlineExceeded => ProviderSearchSection::timed_out(
                ProviderId::Rlm.identity(),
                SearchRanking::None,
                SearchOrdering::Provider,
                Vec::new(),
                vec![
                    "RLM provider deadline exceeded before observing index maintenance".to_string(),
                ],
            )
            .expect("a deadline-expired RLM section is valid"),
            RlmSearchAttempt::Output(result) => {
                let mut section = parse_rlm_search(
                    &result.result_text,
                    context,
                    request.limit,
                    cancellation,
                    self.bindings,
                );
                section.index_freshness =
                    Some(result.freshness.unwrap_or_else(|| "unknown".to_string()));
                section.index_build_id = result.build_id;
                if let Some(maintenance) = result.maintenance {
                    section.index_maintenance = Some(maintenance.evidence());
                }
                section
            }
            RlmSearchAttempt::Unready(readiness) => {
                let maintenance = readiness.maintenance_evidence();
                let dependency_detail = match &readiness {
                    IndexReadiness::Queued => Some("indexQueued"),
                    IndexReadiness::Building => Some("buildingIndex"),
                    _ => None,
                };
                let diagnostic = rlm_search_unready_error(readiness);
                let mut section = if let Some(detail_code) = dependency_detail {
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
                };
                section.index_maintenance = maintenance;
                section
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
        let navigation = RlmNavigationAdapter::new(self.navigation).invoke_resolved_cancellable(
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

pub fn parse_rlm_search(
    text: &str,
    context: &CodeIntelligenceContext,
    requested_limit: usize,
    cancellation: &CancellationToken,
    bindings: SearchSourceBindings,
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
    // The pinned RLM helper searches six sources independently with this
    // quota, then concatenates their answers. A short final array can still
    // conceal more matches in one saturated source.
    let per_source = (requested_limit / 6).max(3);
    let mut source_counts = [0usize; 6];
    for row in rows {
        let source = row.get("source_type").and_then(Value::as_str);
        let index = match source {
            Some("method") => Some(0),
            Some("object") => Some(1),
            Some("region") => Some(2),
            Some("header") => Some(3),
            Some("attribute") => Some(4),
            Some("predefined") => Some(5),
            _ => None,
        };
        if let Some(index) = index {
            source_counts[index] += 1;
        }
    }
    let provider_limit_reached =
        rows.len() >= requested_limit || source_counts.iter().any(|&count| count >= per_source);
    let mut hits = Vec::new();
    let mut diagnostics = Vec::new();
    let mut discarded_result = false;
    let mut locations =
        SearchLocationProjector::with_resolver(context, cancellation, bindings.resolver);
    for (index, row) in rows.iter().enumerate() {
        match parse_rlm_search_row(row, hits.len() + 1, &mut locations) {
            Ok(hit) => hits.push(hit),
            Err(error) if error.starts_with(CANCELLED_PREFIX) => {
                return failed_section(ProviderId::Rlm, error);
            }
            Err(error) => {
                discarded_result = true;
                diagnostics.push(format!("ignored malformed RLM result #{index}: {error}"));
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
    if discarded_result && provider_limit_reached {
        diagnostics.push("RLM result limit was also reached".to_string());
    }
    let section = if discarded_result {
        ProviderSearchSection::partial(
            ProviderId::Rlm.identity(),
            SearchRanking::Provider,
            SearchOrdering::Provider,
            hits,
            diagnostics,
        )
    } else if provider_limit_reached {
        ProviderSearchSection::limit_reached(
            ProviderId::Rlm.identity(),
            SearchRanking::Provider,
            SearchOrdering::Provider,
            hits,
            diagnostics,
        )
    } else {
        ProviderSearchSection::complete(
            ProviderId::Rlm.identity(),
            SearchRanking::Provider,
            SearchOrdering::Provider,
            hits,
            diagnostics,
        )
    };
    section.unwrap_or_else(|error| ProviderSearchSection::failed(ProviderId::Rlm.identity(), error))
}

pub fn parse_rlm_search_row(
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
