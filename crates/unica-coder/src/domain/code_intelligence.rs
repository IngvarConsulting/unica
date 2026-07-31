use crate::domain::{
    cancellation::CancellationToken, source_roots::ResolvedSourceRoot, workspace::WorkspaceContext,
};
use serde::Serialize;
use serde_json::{Map, Value};
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
    ObjectProfile {
        name: String,
        sections: Option<Vec<String>>,
        limit: usize,
    },
}

impl CodeIntelligenceReadRequest {
    pub const fn capability(&self) -> ProviderCapability {
        match self {
            Self::Definition { .. } => ProviderCapability::Definition,
            Self::Outline { .. } => ProviderCapability::Outline,
            Self::ObjectProfile { .. } => ProviderCapability::ObjectProfile,
        }
    }

    pub const fn tool_name(&self) -> &'static str {
        match self {
            Self::Definition { .. } => "unica.code.definition",
            Self::Outline { .. } => "unica.code.outline",
            Self::ObjectProfile { .. } => "unica.meta.profile",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CodeIntelligenceContext {
    pub workspace: WorkspaceContext,
    pub source_root: ResolvedSourceRoot,
}

impl CodeIntelligenceContext {
    pub fn new(workspace: WorkspaceContext, source_root: ResolvedSourceRoot) -> Self {
        Self {
            workspace,
            source_root,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderDeadline(Instant);

impl ProviderDeadline {
    pub fn new(deadline: Instant) -> Self {
        Self(deadline)
    }

    pub fn remaining(self) -> Duration {
        self.0.saturating_duration_since(Instant::now())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderSectionStatus {
    Ok,
    Empty,
    Unavailable,
    Failed,
}

impl ProviderSectionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Empty => "empty",
            Self::Unavailable => "unavailable",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSearchHit {
    pub rank: usize,
    pub provider_score: Option<f64>,
    pub path: String,
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
    pub provider: ProviderId,
    pub status: ProviderSectionStatus,
    pub hits: Vec<ProviderSearchHit>,
    pub diagnostics: Vec<String>,
    pub artifacts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeSearchResult {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
pub struct CodeDefinition {
    pub file: String,
    pub line: u64,
    /// Platform kind reported by the index, `method` when it says nothing.
    pub kind: String,
    pub params: Vec<String>,
    pub export: bool,
    pub category: Option<String>,
    pub object_name: Option<String>,
    pub module_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
            ProviderSearchSection {
                provider: self.id,
                status: ProviderSectionStatus::Empty,
                hits: Vec::new(),
                diagnostics: Vec::new(),
                artifacts: Vec::new(),
            }
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
                summary: format!("{} handled", request.tool_name()),
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
        assert_eq!(outcome.summary, "unica.code.definition handled");
    }

    #[test]
    fn canonical_result_serializes_the_reader_facing_contract() {
        let result = CodeSearchResult {
            sections: vec![ProviderSearchSection {
                provider: ProviderId::Rlm,
                status: ProviderSectionStatus::Ok,
                hits: vec![ProviderSearchHit {
                    rank: 1,
                    provider_score: Some(0.91),
                    path: "CommonModules/Sales/Ext/Module.bsl".to_string(),
                    line: 42,
                    end_line: Some(58),
                    symbol: Some("Post".to_string()),
                    kind: Some("procedure".to_string()),
                    snippet: "Procedure Post()".to_string(),
                    attributes: Map::new(),
                }],
                diagnostics: Vec::new(),
                artifacts: Vec::new(),
            }],
        };

        assert_eq!(
            serde_json::to_value(result).unwrap(),
            json!({
                "sections": [{
                    "provider": "rlm",
                    "status": "ok",
                    "hits": [{
                        "rank": 1,
                        "providerScore": 0.91,
                        "path": "CommonModules/Sales/Ext/Module.bsl",
                        "line": 42,
                        "endLine": 58,
                        "symbol": "Post",
                        "kind": "procedure",
                        "snippet": "Procedure Post()",
                        "attributes": {}
                    }],
                    "diagnostics": [],
                    "artifacts": []
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
}
