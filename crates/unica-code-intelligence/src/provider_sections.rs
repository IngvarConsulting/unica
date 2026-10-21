//! Existing provider result constructors and scope refusal.
use crate::model::{
    CodeIntelligenceContext, ProviderId, ProviderSearchSection, SearchOrdering, SearchRanking,
};
pub fn empty_section(provider: ProviderId, ranking: SearchRanking) -> ProviderSearchSection {
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

pub fn unavailable_section(provider: ProviderId, diagnostic: String) -> ProviderSearchSection {
    ProviderSearchSection::unavailable(provider.identity(), diagnostic)
}

pub fn has_targeted_search_scope(context: &CodeIntelligenceContext) -> bool {
    context
        .search_scope
        .as_ref()
        .is_some_and(|scope| !scope.filters.is_empty())
}

pub fn unavailable_targeted_scope_section(provider: ProviderId) -> ProviderSearchSection {
    ProviderSearchSection::unsupported_scope(
        provider.identity(),
        format!(
            "{} cannot constrain search to metadataPath; the role was not searched outside the requested logical scope",
            provider.as_str()
        ),
    )
}

pub fn failed_section(provider: ProviderId, diagnostic: String) -> ProviderSearchSection {
    ProviderSearchSection::failed(provider.identity(), diagnostic)
}
