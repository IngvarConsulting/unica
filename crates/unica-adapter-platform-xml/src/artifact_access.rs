use unica_format_core::{
    ports::{
        SemanticArtifactLease, SemanticArtifactPort, SemanticArtifactReadRequest,
        SemanticArtifactReadResult,
    },
    source::{SourceAdapterError, SourceAdapterErrorKind},
};

use crate::versions::v2_20;

pub(crate) struct PlatformXmlSemanticArtifacts;

#[derive(Debug)]
struct PlatformXmlArtifactBytes {
    bytes: Vec<u8>,
}

impl SemanticArtifactPort for PlatformXmlSemanticArtifacts {
    fn read(
        &self,
        request: &SemanticArtifactReadRequest,
    ) -> Result<SemanticArtifactReadResult, SourceAdapterError> {
        let session = v2_20::operations::session_from_handle(request.session())?;
        match session.semantic_artifact_bytes(request.role()) {
            Ok(Some(bytes)) => Ok(SemanticArtifactReadResult::Present(
                SemanticArtifactLease::new(PlatformXmlArtifactBytes { bytes }),
            )),
            Ok(None) => Ok(SemanticArtifactReadResult::Absent),
            Err(_) => Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "semantic source artifact could not be read safely",
            )),
        }
    }

    fn bytes<'a>(&self, lease: &'a SemanticArtifactLease) -> Option<&'a [u8]> {
        lease
            .adapter_state::<PlatformXmlArtifactBytes>()
            .map(|artifact| artifact.bytes.as_slice())
    }
}
