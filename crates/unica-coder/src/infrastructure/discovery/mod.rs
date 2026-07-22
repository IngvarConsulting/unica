pub(crate) mod bsl;
pub(crate) mod forms;
pub(crate) mod inventory;
pub(crate) mod metadata;
pub(crate) mod support;

use crate::domain::discovery::{DiscoveryQuery, ProviderDiagnostic, ProviderOutcome};

pub(crate) fn cancellation_diagnostic() -> ProviderDiagnostic {
    ProviderDiagnostic::material("discovery_cancelled", "discovery cancelled")
}

pub(crate) fn check_cancellation(query: &DiscoveryQuery<'_>) -> Result<(), ProviderDiagnostic> {
    if query.is_cancelled() {
        Err(cancellation_diagnostic())
    } else {
        Ok(())
    }
}

pub(crate) fn cancellation_outcome<T>(query: &DiscoveryQuery<'_>) -> Option<ProviderOutcome<T>> {
    query
        .is_cancelled()
        .then(|| ProviderOutcome::Failed(cancellation_diagnostic()))
}

pub(crate) fn is_cancellation_diagnostic(diagnostic: &ProviderDiagnostic) -> bool {
    diagnostic.code == "discovery_cancelled"
}
