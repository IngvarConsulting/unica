use crate::application::metadata::{MetaFailure, MetaInfoRequest, MetadataRequest};
use crate::application::ports::{
    MetaLocalInfo, MetaRelatedData, MetadataRead, MetadataValidationResult,
    MetadataValidationSubject, PreparedMetadataMutation,
};
use crate::domain::cancellation::CancellationToken;
use crate::domain::metadata::{
    MetaCompleteness, MetaDiagnostic, MetaDiagnosticCode, MetaFreshness, MetaRelatedItem,
    MetaRelatedSection, MetaRelatedSections, MetaRelatedStatus, MetaValidationData,
    MetaValidationStatus,
};
use crate::domain::workspace::WorkspaceContext;

pub(crate) struct MetadataOperations;

impl MetadataOperations {
    pub(crate) fn read_local(
        _request: &MetaInfoRequest,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> Result<MetadataRead, MetaFailure> {
        Err(capability_unavailable(
            "typed metadata read provider is not available yet",
        ))
    }

    pub(crate) fn read_related(
        _request: &MetaInfoRequest,
        _local: &MetaLocalInfo,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> MetaRelatedData {
        MetaRelatedSections {
            modules: unavailable_section(),
            roles: unavailable_section(),
            subscriptions: unavailable_section(),
            functional_options: unavailable_section(),
            predefined_items: Some(unavailable_section()),
        }
    }

    pub(crate) fn validate(
        _subject: &MetadataValidationSubject,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> MetadataValidationResult {
        MetaValidationData {
            status: MetaValidationStatus::Failed,
            diagnostics: capability_unavailable("typed metadata validator is not available yet")
                .diagnostics,
        }
    }

    pub(crate) fn prepare_mutation(
        _request: &MetadataRequest,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> Result<Box<dyn PreparedMetadataMutation>, MetaFailure> {
        Err(capability_unavailable(
            "typed metadata mutation provider is not available yet",
        ))
    }
}

fn unavailable_section() -> MetaRelatedSection<MetaRelatedItem> {
    MetaRelatedSection {
        status: MetaRelatedStatus::Unavailable,
        freshness: MetaFreshness::Unknown,
        completeness: MetaCompleteness::Unknown,
        total: 0,
        returned: 0,
        truncated: false,
        items: Vec::new(),
        diagnostics: capability_unavailable("typed related metadata provider is not available yet")
            .diagnostics,
    }
}

fn capability_unavailable(message: &str) -> MetaFailure {
    MetaDiagnostic::error(MetaDiagnosticCode::CapabilityUnavailable, message).into()
}
