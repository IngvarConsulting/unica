use crate::domain::{cancellation::CancellationToken, workspace::WorkspaceContext};
use serde_json::{Map, Value};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderId {
    Rlm,
    BslAnalyzer,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRequest {
    pub query: String,
    pub limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderSearchSection {
    pub provider: ProviderId,
    pub status: ProviderSectionStatus,
    pub hits: Vec<ProviderSearchHit>,
    pub diagnostics: Vec<String>,
    pub artifacts: Vec<String>,
}

pub trait CodeIntelligenceProvider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn capabilities(&self) -> &[ProviderCapability];
    fn search(
        &self,
        request: &SearchRequest,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> ProviderSearchSection;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeProvider {
        id: ProviderId,
        capabilities: Vec<ProviderCapability>,
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
            _context: &WorkspaceContext,
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
    }

    #[test]
    fn capabilities_match_the_executable_provider_operations() {
        let capability = ProviderCapability::Search;

        let operation = match capability {
            ProviderCapability::Search => "search",
        };

        assert_eq!(operation, "search");
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
}
