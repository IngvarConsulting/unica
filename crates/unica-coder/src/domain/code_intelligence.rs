use crate::domain::{
    cancellation::CancellationToken, source_location::SourceLocation,
    source_roots::ResolvedSourceRoot, workspace::WorkspaceContext,
};
use serde::Serialize;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum ProviderId {
    #[serde(rename = "rlm")]
    Rlm,
    #[serde(rename = "bsl-analyzer")]
    BslAnalyzer,
    #[serde(rename = "git-grep")]
    GitGrep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderRole {
    Semantic,
    Symbol,
    Lexical,
}

impl ProviderRole {
    pub const ALL: [Self; 3] = [Self::Semantic, Self::Symbol, Self::Lexical];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Symbol => "symbol",
            Self::Lexical => "lexical",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderIdentity {
    pub role: ProviderRole,
    pub provider: String,
}

impl ProviderIdentity {
    pub fn new(role: ProviderRole, provider: impl Into<String>) -> Self {
        Self {
            role,
            provider: provider.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchProviderState {
    Queued,
    Running,
    Completed,
    Unavailable,
    Failed,
    TimedOut,
    Cancelled,
}

impl SearchProviderState {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Unavailable | Self::Failed | Self::TimedOut | Self::Cancelled
        )
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Unavailable => "unavailable",
            Self::Failed => "failed",
            Self::TimedOut => "timed out",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchProviderPhase {
    Preparing,
    Searching,
    Ranking,
}

impl SearchProviderPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Searching => "searching",
            Self::Ranking => "ranking",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchProviderProgress {
    #[serde(flatten)]
    pub identity: ProviderIdentity,
    pub state: SearchProviderState,
    pub phase: SearchProviderPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_code: Option<String>,
    pub results_found: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchProgressSnapshot {
    pub schema_version: u32,
    pub elapsed_ms: u64,
    pub deadline_ms: u64,
    pub next_update_within_ms: u64,
    pub providers: Vec<SearchProviderProgress>,
}

impl SearchProgressSnapshot {
    pub fn terminal_roles(&self) -> usize {
        self.providers
            .iter()
            .filter(|provider| provider.state.is_terminal())
            .count()
    }
}

pub trait SearchProgressSink: Send + Sync {
    fn publish(&self, snapshot: SearchProgressSnapshot);
}

#[derive(Debug, Default)]
pub struct NoopSearchProgressSink;

impl SearchProgressSink for NoopSearchProgressSink {
    fn publish(&self, _snapshot: SearchProgressSnapshot) {}
}

impl ProviderId {
    pub const fn role(self) -> ProviderRole {
        match self {
            Self::Rlm => ProviderRole::Semantic,
            Self::BslAnalyzer => ProviderRole::Symbol,
            Self::GitGrep => ProviderRole::Lexical,
        }
    }

    pub fn identity(self) -> ProviderIdentity {
        ProviderIdentity::new(self.role(), self.as_str())
    }
}

impl ProviderId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rlm => "rlm",
            Self::BslAnalyzer => "bsl-analyzer",
            Self::GitGrep => "git-grep",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderCapability {
    Search,
    Definition,
    Outline,
    ObjectProfile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRequest {
    pub query: String,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeIntelligenceReadRequest {
    Definition {
        name: String,
        module_hint: String,
        limit: usize,
    },
    Outline {
        path: String,
        include_methods: bool,
    },
}

impl CodeIntelligenceReadRequest {
    pub const fn capability(&self) -> ProviderCapability {
        match self {
            Self::Definition { .. } => ProviderCapability::Definition,
            Self::Outline { .. } => ProviderCapability::Outline,
        }
    }

    pub const fn operation_name(&self) -> &'static str {
        match self {
            Self::Definition { .. } => "code definition",
            Self::Outline { .. } => "code outline",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CodeIntelligenceContext {
    pub workspace: WorkspaceContext,
    pub source_root: ResolvedSourceRoot,
    pub search_scope: Option<CodeSearchScope>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelativeSearchFilter {
    Exact(PathBuf),
    Subtree(PathBuf),
}

#[derive(Debug, Clone)]
pub struct CodeSearchScope {
    pub source_set: String,
    pub source_root: PathBuf,
    pub filters: Vec<RelativeSearchFilter>,
    pub legacy_selector: bool,
}

impl CodeSearchScope {
    pub fn all(source_set: String, source_root: PathBuf, legacy_selector: bool) -> Self {
        Self {
            source_set,
            source_root,
            filters: Vec::new(),
            legacy_selector,
        }
    }

    pub fn accepts(&self, relative_path: &Path) -> bool {
        if relative_path.is_absolute()
            || relative_path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return false;
        }
        self.filters.is_empty()
            || self.filters.iter().any(|filter| match filter {
                RelativeSearchFilter::Exact(path) => relative_path == path,
                RelativeSearchFilter::Subtree(path) => relative_path.starts_with(path),
            })
    }
}

impl CodeIntelligenceContext {
    pub fn new(workspace: WorkspaceContext, source_root: ResolvedSourceRoot) -> Self {
        Self {
            workspace,
            source_root,
            search_scope: None,
        }
    }

    pub fn with_search_scope(mut self, scope: CodeSearchScope) -> Self {
        self.search_scope = Some(scope);
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderDeadline {
    started_at: Instant,
    budget: Duration,
    #[cfg(test)]
    now: fn() -> Instant,
}

impl PartialEq for ProviderDeadline {
    fn eq(&self, other: &Self) -> bool {
        if self.started_at <= other.started_at {
            self.budget.checked_sub(other.budget)
                == other.started_at.checked_duration_since(self.started_at)
        } else {
            other.budget.checked_sub(self.budget)
                == self.started_at.checked_duration_since(other.started_at)
        }
    }
}

impl Eq for ProviderDeadline {}

impl ProviderDeadline {
    pub fn new(deadline: Instant) -> Self {
        let started_at = Instant::now();
        let budget = deadline
            .checked_duration_since(started_at)
            .unwrap_or(Duration::ZERO);
        Self::from_started_at(started_at, budget)
    }

    pub(crate) fn from_budget(budget: Duration) -> Self {
        Self::from_started_at(Instant::now(), budget)
    }

    pub(crate) fn from_started_at(started_at: Instant, budget: Duration) -> Self {
        Self {
            started_at,
            budget,
            #[cfg(test)]
            now: Instant::now,
        }
    }

    pub fn remaining(self) -> Duration {
        #[cfg(test)]
        let now = (self.now)();
        #[cfg(not(test))]
        let now = Instant::now();
        let elapsed = now
            .checked_duration_since(self.started_at)
            .unwrap_or(Duration::ZERO);
        self.budget.checked_sub(elapsed).unwrap_or(Duration::ZERO)
    }

    #[cfg(test)]
    pub(crate) fn with_clock(deadline: Instant, now: fn() -> Instant) -> Self {
        let started_at = now();
        let budget = deadline
            .checked_duration_since(started_at)
            .unwrap_or(Duration::ZERO);
        Self {
            started_at,
            budget,
            now,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderSectionStatus {
    Ok,
    Empty,
    LimitReached,
    TimedOut,
    Unavailable,
    Failed,
}

impl ProviderSectionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Empty => "empty",
            Self::LimitReached => "limitReached",
            Self::TimedOut => "timedOut",
            Self::Unavailable => "unavailable",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchRanking {
    Provider,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchOrdering {
    Provider,
    ProviderTraversal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchCountRelation {
    Exact,
    LowerBound,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatchCount {
    pub returned: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
    pub relation: SearchCountRelation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchCoverage {
    Complete,
    Partial,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSearchHit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_score: Option<f64>,
    pub location: SourceLocation,
    pub line: usize,
    pub end_line: Option<usize>,
    pub symbol: Option<String>,
    pub kind: Option<String>,
    pub snippet: String,
    pub attributes: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSearchSection {
    #[serde(flatten)]
    pub identity: ProviderIdentity,
    pub status: ProviderSectionStatus,
    pub search_complete: bool,
    pub ranking: SearchRanking,
    pub ordering: SearchOrdering,
    pub matches: SearchMatchCount,
    pub hits: Vec<ProviderSearchHit>,
    pub diagnostics: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<String>,
}

impl ProviderSearchSection {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        identity: ProviderIdentity,
        status: ProviderSectionStatus,
        search_complete: bool,
        ranking: SearchRanking,
        ordering: SearchOrdering,
        matches: SearchMatchCount,
        hits: Vec<ProviderSearchHit>,
        diagnostics: Vec<String>,
    ) -> Result<Self, String> {
        if matches.returned != hits.len() {
            return Err("search match count must equal the number of returned hits".to_string());
        }
        match status {
            ProviderSectionStatus::Ok => {
                if !search_complete
                    || hits.is_empty()
                    || matches.relation != SearchCountRelation::Exact
                    || matches.total != Some(hits.len())
                {
                    return Err(
                        "ok search section must be complete with an exact non-zero count"
                            .to_string(),
                    );
                }
            }
            ProviderSectionStatus::Empty => {
                if !search_complete
                    || !hits.is_empty()
                    || matches.relation != SearchCountRelation::Exact
                    || matches.total != Some(0)
                {
                    return Err("empty search section must carry an exact zero count".to_string());
                }
            }
            ProviderSectionStatus::LimitReached | ProviderSectionStatus::TimedOut => {
                if search_complete
                    || matches.relation != SearchCountRelation::LowerBound
                    || matches.total.is_none_or(|total| total < hits.len())
                {
                    return Err(
                        "bounded search section must be incomplete with a valid lower bound"
                            .to_string(),
                    );
                }
            }
            ProviderSectionStatus::Unavailable | ProviderSectionStatus::Failed => {
                if search_complete
                    || !hits.is_empty()
                    || matches.relation != SearchCountRelation::Unknown
                    || matches.total.is_some()
                {
                    return Err(
                        "failed search section must be incomplete without result claims"
                            .to_string(),
                    );
                }
            }
        }
        match ranking {
            SearchRanking::None
                if hits
                    .iter()
                    .any(|hit| hit.rank.is_some() || hit.provider_score.is_some()) =>
            {
                return Err("unranked search hits cannot carry rank or provider score".to_string());
            }
            SearchRanking::Provider
                if hits
                    .iter()
                    .enumerate()
                    .any(|(index, hit)| hit.rank != Some(index + 1)) =>
            {
                return Err(
                    "provider-ranked hits must carry consecutive ranks from one".to_string()
                );
            }
            _ => {}
        }
        Ok(Self {
            identity,
            status,
            search_complete,
            ranking,
            ordering,
            matches,
            hits,
            diagnostics,
            artifacts: Vec::new(),
        })
    }

    pub fn complete(
        identity: ProviderIdentity,
        ranking: SearchRanking,
        ordering: SearchOrdering,
        hits: Vec<ProviderSearchHit>,
        diagnostics: Vec<String>,
    ) -> Result<Self, String> {
        let returned = hits.len();
        let status = if returned == 0 {
            ProviderSectionStatus::Empty
        } else {
            ProviderSectionStatus::Ok
        };
        Self::new(
            identity,
            status,
            true,
            ranking,
            ordering,
            SearchMatchCount {
                returned,
                total: Some(returned),
                relation: SearchCountRelation::Exact,
            },
            hits,
            diagnostics,
        )
    }

    pub fn limit_reached(
        identity: ProviderIdentity,
        ranking: SearchRanking,
        ordering: SearchOrdering,
        hits: Vec<ProviderSearchHit>,
        diagnostics: Vec<String>,
    ) -> Result<Self, String> {
        Self::bounded(
            identity,
            ProviderSectionStatus::LimitReached,
            ranking,
            ordering,
            hits,
            diagnostics,
        )
    }

    pub fn timed_out(
        identity: ProviderIdentity,
        ranking: SearchRanking,
        ordering: SearchOrdering,
        hits: Vec<ProviderSearchHit>,
        diagnostics: Vec<String>,
    ) -> Result<Self, String> {
        Self::bounded(
            identity,
            ProviderSectionStatus::TimedOut,
            ranking,
            ordering,
            hits,
            diagnostics,
        )
    }

    fn bounded(
        identity: ProviderIdentity,
        status: ProviderSectionStatus,
        ranking: SearchRanking,
        ordering: SearchOrdering,
        hits: Vec<ProviderSearchHit>,
        diagnostics: Vec<String>,
    ) -> Result<Self, String> {
        let returned = hits.len();
        Self::new(
            identity,
            status,
            false,
            ranking,
            ordering,
            SearchMatchCount {
                returned,
                total: Some(returned),
                relation: SearchCountRelation::LowerBound,
            },
            hits,
            diagnostics,
        )
    }

    pub fn unavailable(identity: ProviderIdentity, diagnostic: String) -> Self {
        Self::problem(identity, ProviderSectionStatus::Unavailable, diagnostic)
    }

    pub fn failed(identity: ProviderIdentity, diagnostic: String) -> Self {
        Self::problem(identity, ProviderSectionStatus::Failed, diagnostic)
    }

    fn problem(
        identity: ProviderIdentity,
        status: ProviderSectionStatus,
        diagnostic: String,
    ) -> Self {
        Self::new(
            identity,
            status,
            false,
            SearchRanking::None,
            SearchOrdering::Provider,
            SearchMatchCount {
                returned: 0,
                total: None,
                relation: SearchCountRelation::Unknown,
            },
            Vec::new(),
            vec![diagnostic],
        )
        .expect("problem section is a valid closed construction")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeSearchResult {
    pub coverage: SearchCoverage,
    pub elapsed_ms: u64,
    pub sections: Vec<ProviderSearchSection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeOutlineResult {
    pub module: String,
    pub identity: CodeOutlineIdentity,
    pub totals: CodeOutlineTotals,
    pub regions: Vec<CodeOutlineRegion>,
    pub methods: Vec<CodeOutlineMethod>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeOutlineIdentity {
    pub category: Option<String>,
    pub object: Option<String>,
    pub module_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeOutlineTotals {
    pub methods: usize,
    pub exports: usize,
    pub regions: usize,
    pub loc: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeOutlineRegion {
    pub name: Option<String>,
    pub line: usize,
    pub end_line: Option<usize>,
    pub regions: Vec<CodeOutlineRegion>,
    pub methods: Vec<CodeOutlineMethod>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeOutlineMethod {
    pub name: String,
    pub kind: CodeOutlineMethodKind,
    pub parameters: Vec<CodeOutlineParameter>,
    #[serde(rename = "export")]
    pub is_export: bool,
    pub line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CodeOutlineMethodKind {
    Procedure,
    Function,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeOutlineParameter {
    pub name: String,
    pub by_value: bool,
    pub default_value: Option<String>,
}

// `Eq` is gone because a profile section carries whatever the index reported,
// and `serde_json::Value` is only `PartialEq`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum CodeIntelligenceReadData {
    Outline(CodeOutlineResult),
    Definition(CodeDefinitionResult),
}

/// Typed answer of `unica.code.definition` (ADR-0023). The index already
/// returns structured definitions; the tool used to render them into a line
/// grammar the caller then had to parse back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeDefinitionResult {
    pub name: String,
    pub definitions: Vec<CodeDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
/// Every field the index does not report is `null` (ADR-0023). An explicit
/// zero, `false` or empty list is a proven answer and keeps its value; only
/// `file` and `line` are required, because a definition nobody can open is not
/// a definition.
pub struct CodeDefinition {
    pub file: String,
    pub line: u64,
    /// Platform kind reported by the index; `null` when it reported none.
    pub kind: Option<String>,
    pub params: Option<Vec<String>>,
    pub export: Option<bool>,
    pub category: Option<String>,
    pub object_name: Option<String>,
    pub module_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderReadOutcome {
    pub provider: ProviderId,
    pub ok: bool,
    pub summary: String,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub artifacts: Vec<String>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub data: Option<CodeIntelligenceReadData>,
}

pub trait CodeIntelligenceProvider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn capabilities(&self) -> &[ProviderCapability];
    fn search(
        &self,
        request: &SearchRequest,
        context: &CodeIntelligenceContext,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> ProviderSearchSection;

    fn read(
        &self,
        request: &CodeIntelligenceReadRequest,
        _context: &CodeIntelligenceContext,
        _deadline: ProviderDeadline,
        _cancellation: &CancellationToken,
    ) -> Result<ProviderReadOutcome, String> {
        Err(format!(
            "provider {} does not implement {:?}",
            self.id().as_str(),
            request.capability()
        ))
    }
}

pub struct CodeIntelligenceRegistry {
    providers: Vec<Arc<dyn CodeIntelligenceProvider>>,
}

impl CodeIntelligenceRegistry {
    pub fn new(providers: Vec<Arc<dyn CodeIntelligenceProvider>>) -> Result<Self, String> {
        let mut ids = std::collections::HashSet::new();
        for provider in &providers {
            if !ids.insert(provider.id()) {
                return Err(format!(
                    "duplicate code intelligence provider: {}",
                    provider.id().as_str()
                ));
            }
        }
        Ok(Self { providers })
    }

    pub fn search_providers(&self) -> impl Iterator<Item = &Arc<dyn CodeIntelligenceProvider>> {
        self.providers.iter().filter(|provider| {
            provider
                .capabilities()
                .contains(&ProviderCapability::Search)
        })
    }

    pub fn search_provider_arcs(&self) -> Vec<Arc<dyn CodeIntelligenceProvider>> {
        self.search_providers().cloned().collect()
    }

    pub fn provider_for(
        &self,
        capability: ProviderCapability,
    ) -> Option<Arc<dyn CodeIntelligenceProvider>> {
        self.providers
            .iter()
            .find(|provider| provider.capabilities().contains(&capability))
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::source_roots::ResolvedSourceRoot;
    use crate::domain::workspace::WorkspaceContext;
    use serde_json::json;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    struct FakeProvider {
        id: ProviderId,
        capabilities: Vec<ProviderCapability>,
    }

    fn context() -> CodeIntelligenceContext {
        CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: PathBuf::from("/workspace"),
                workspace_root: PathBuf::from("/workspace"),
                cache_root: PathBuf::from("/cache"),
                workspace_epoch: 7,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: PathBuf::from("/workspace/src"),
            },
        )
    }

    impl CodeIntelligenceProvider for FakeProvider {
        fn id(&self) -> ProviderId {
            self.id
        }

        fn capabilities(&self) -> &[ProviderCapability] {
            &self.capabilities
        }

        fn search(
            &self,
            _request: &SearchRequest,
            _context: &CodeIntelligenceContext,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> ProviderSearchSection {
            ProviderSearchSection::complete(
                self.id.identity(),
                SearchRanking::Provider,
                SearchOrdering::Provider,
                Vec::new(),
                Vec::new(),
            )
            .unwrap()
        }

        fn read(
            &self,
            request: &CodeIntelligenceReadRequest,
            _context: &CodeIntelligenceContext,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> Result<ProviderReadOutcome, String> {
            Ok(ProviderReadOutcome {
                provider: self.id,
                ok: true,
                summary: format!("{} handled", request.operation_name()),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: Vec::new(),
                stdout: None,
                stderr: None,
                data: None,
            })
        }
    }

    #[test]
    fn registry_preserves_injected_search_provider_order() {
        let registry = CodeIntelligenceRegistry::new(vec![
            Arc::new(FakeProvider {
                id: ProviderId::Rlm,
                capabilities: vec![ProviderCapability::Search],
            }),
            Arc::new(FakeProvider {
                id: ProviderId::BslAnalyzer,
                capabilities: Vec::new(),
            }),
            Arc::new(FakeProvider {
                id: ProviderId::GitGrep,
                capabilities: vec![ProviderCapability::Search],
            }),
        ])
        .unwrap();

        let ids = registry
            .search_providers()
            .map(|provider| provider.id())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec![ProviderId::Rlm, ProviderId::GitGrep]);
    }

    #[test]
    fn registry_rejects_duplicate_provider_ids() {
        let providers: Vec<Arc<dyn CodeIntelligenceProvider>> = vec![
            Arc::new(FakeProvider {
                id: ProviderId::Rlm,
                capabilities: vec![ProviderCapability::Search],
            }),
            Arc::new(FakeProvider {
                id: ProviderId::Rlm,
                capabilities: Vec::new(),
            }),
        ];

        let error = match CodeIntelligenceRegistry::new(providers) {
            Ok(_) => panic!("duplicate provider ids must be rejected"),
            Err(error) => error,
        };

        assert_eq!(error, "duplicate code intelligence provider: rlm");
    }

    #[test]
    fn registry_resolves_an_executable_provider_for_read_capabilities() {
        let registry = CodeIntelligenceRegistry::new(vec![
            Arc::new(FakeProvider {
                id: ProviderId::Rlm,
                capabilities: vec![
                    ProviderCapability::Search,
                    ProviderCapability::Definition,
                    ProviderCapability::Outline,
                    ProviderCapability::ObjectProfile,
                ],
            }),
            Arc::new(FakeProvider {
                id: ProviderId::GitGrep,
                capabilities: vec![ProviderCapability::Search],
            }),
        ])
        .unwrap();
        let request = CodeIntelligenceReadRequest::Definition {
            name: "Найти".to_string(),
            module_hint: String::new(),
            limit: 50,
        };
        let provider = registry
            .provider_for(request.capability())
            .expect("definition capability must resolve to a provider");

        let outcome = provider
            .read(
                &request,
                &context(),
                ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert_eq!(outcome.provider, ProviderId::Rlm);
        assert_eq!(outcome.summary, "code definition handled");
    }

    #[test]
    fn canonical_result_serializes_the_reader_facing_contract() {
        let result = CodeSearchResult {
            coverage: SearchCoverage::Complete,
            elapsed_ms: 12,
            sections: vec![ProviderSearchSection::complete(
                ProviderId::Rlm.identity(),
                SearchRanking::Provider,
                SearchOrdering::Provider,
                vec![ProviderSearchHit {
                    rank: Some(1),
                    provider_score: Some(0.91),
                    location: SourceLocation::Unaddressable {
                        source_set: "main".to_string(),
                        owner_metadata_path: None,
                        path: "CommonModules/Sales/Ext/Module.bsl".to_string(),
                    },
                    line: 42,
                    end_line: Some(58),
                    symbol: Some("Post".to_string()),
                    kind: Some("procedure".to_string()),
                    snippet: "Procedure Post()".to_string(),
                    attributes: Map::new(),
                }],
                Vec::new(),
            )
            .unwrap()],
        };

        assert_eq!(
            serde_json::to_value(result).unwrap(),
            json!({
                "coverage": "complete",
                "elapsedMs": 12,
                "sections": [{
                    "role": "semantic",
                    "provider": "rlm",
                    "status": "ok",
                    "searchComplete": true,
                    "ranking": "provider",
                    "ordering": "provider",
                    "matches": {"returned": 1, "total": 1, "relation": "exact"},
                    "hits": [{
                        "rank": 1,
                        "providerScore": 0.91,
                        "location": {
                            "kind": "unaddressable",
                            "sourceSet": "main",
                            "path": "CommonModules/Sales/Ext/Module.bsl"
                        },
                        "line": 42,
                        "endLine": 58,
                        "symbol": "Post",
                        "kind": "procedure",
                        "snippet": "Procedure Post()",
                        "attributes": {}
                    }],
                    "diagnostics": []
                }]
            })
        );
    }

    #[test]
    fn progress_snapshot_serializes_role_state_for_an_ai_client() {
        let snapshot = SearchProgressSnapshot {
            schema_version: 1,
            elapsed_ms: 2_100,
            deadline_ms: 300_000,
            next_update_within_ms: 2_000,
            providers: vec![SearchProviderProgress {
                identity: ProviderId::GitGrep.identity(),
                state: SearchProviderState::Running,
                phase: SearchProviderPhase::Searching,
                detail_code: None,
                results_found: 4,
            }],
        };

        assert_eq!(
            serde_json::to_value(snapshot).unwrap(),
            serde_json::json!({
                "schemaVersion": 1,
                "elapsedMs": 2100,
                "deadlineMs": 300000,
                "nextUpdateWithinMs": 2000,
                "providers": [{
                    "role": "lexical",
                    "provider": "git-grep",
                    "state": "running",
                    "phase": "searching",
                    "resultsFound": 4
                }]
            })
        );
    }

    #[test]
    fn provider_context_carries_one_resolved_workspace_and_source_identity() {
        let deadline = Instant::now() + Duration::from_secs(120);

        let context = context();
        let provider_deadline = ProviderDeadline::new(deadline);

        assert_eq!(
            context.workspace.workspace_root,
            PathBuf::from("/workspace")
        );
        assert_eq!(context.workspace.workspace_epoch, 7);
        assert_eq!(
            context.source_root,
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: PathBuf::from("/workspace/src"),
            }
        );
        assert!(provider_deadline.remaining() <= Duration::from_secs(120));
        assert!(provider_deadline.remaining() > Duration::from_secs(119));
    }

    #[test]
    fn provider_deadline_preserves_its_equality_contract() {
        fn assert_eq_contract<T: Eq>() {}

        assert_eq_contract::<ProviderDeadline>();
        let deadline = Instant::now() + Duration::from_secs(1);
        assert_eq!(
            ProviderDeadline::new(deadline),
            ProviderDeadline::new(deadline)
        );
    }

    #[test]
    fn search_section_serializes_role_provenance_completeness_and_logical_location() {
        use crate::domain::source_location::SourceLocation;
        use crate::domain::source_target::{
            MetadataAddress, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
        };

        let location = SourceLocation::Addressed {
            source_set: "main".to_string(),
            metadata_path: Some(
                MetadataAddress::parse(
                    PLATFORM_XML_8_3_27_FORMAT_2_20,
                    "CommonModule.Sales.Module",
                )
                .unwrap(),
            ),
            target_kind: TargetKind::Module,
        };
        let section = ProviderSearchSection::complete(
            ProviderIdentity::new(ProviderRole::Semantic, "replacement-semantic"),
            SearchRanking::Provider,
            SearchOrdering::Provider,
            vec![ProviderSearchHit {
                rank: Some(1),
                provider_score: Some(0.91),
                location,
                line: 42,
                end_line: Some(58),
                symbol: Some("Post".to_string()),
                kind: Some("procedure".to_string()),
                snippet: "Procedure Post()".to_string(),
                attributes: Map::new(),
            }],
            Vec::new(),
        )
        .unwrap();

        assert_eq!(
            serde_json::to_value(section).unwrap(),
            json!({
                "role": "semantic",
                "provider": "replacement-semantic",
                "status": "ok",
                "searchComplete": true,
                "ranking": "provider",
                "ordering": "provider",
                "matches": {"returned": 1, "total": 1, "relation": "exact"},
                "hits": [{
                    "rank": 1,
                    "providerScore": 0.91,
                    "location": {
                        "kind": "addressed",
                        "sourceSet": "main",
                        "metadataPath": "CommonModule.Sales.Module",
                        "targetKind": "module"
                    },
                    "line": 42,
                    "endLine": 58,
                    "symbol": "Post",
                    "kind": "procedure",
                    "snippet": "Procedure Post()",
                    "attributes": {}
                }],
                "diagnostics": []
            })
        );
    }

    #[test]
    fn empty_section_rejects_an_incomplete_count() {
        let error = ProviderSearchSection::new(
            ProviderIdentity::new(ProviderRole::Lexical, "git-grep"),
            ProviderSectionStatus::Empty,
            true,
            SearchRanking::None,
            SearchOrdering::ProviderTraversal,
            SearchMatchCount {
                returned: 0,
                total: Some(0),
                relation: SearchCountRelation::LowerBound,
            },
            Vec::new(),
            Vec::new(),
        )
        .unwrap_err();

        assert_eq!(error, "empty search section must carry an exact zero count");
    }
}
