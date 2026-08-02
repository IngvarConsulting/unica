use crate::application::metadata::{MetaFailure, MetaInfoRequest, MetadataRequest};
use crate::application::ports::{
    MetaLocalInfo, MetaRelatedData, MetadataRead, MetadataValidationResult,
    MetadataValidationSubject, PreparedMetadataMutation,
};
use crate::domain::cancellation::CancellationToken;
use crate::domain::metadata::{
    MetaCompleteness, MetaDiagnostic, MetaDiagnosticCode, MetaFreshness, MetaRelatedItem,
    MetaRelatedSection, MetaRelatedSections, MetaRelatedStatus,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::meta::{prepare_meta_add, MetadataValidator};

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
        subject: &MetadataValidationSubject,
        context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> MetadataValidationResult {
        MetadataValidator.validate(subject, context)
    }

    pub(crate) fn prepare_mutation(
        request: &MetadataRequest,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<Box<dyn PreparedMetadataMutation>, MetaFailure> {
        match request {
            MetadataRequest::Add(request) => prepare_meta_add(request, context, cancellation),
            MetadataRequest::Info(_) | MetadataRequest::Edit(_) | MetadataRequest::Remove(_) => {
                Err(capability_unavailable(
                    "typed metadata mutation provider is not available yet",
                ))
            }
        }
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
