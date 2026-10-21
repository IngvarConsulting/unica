//! Immutable trusted backend bindings captured with an actor instance.
use std::sync::Arc;
use unica_source_contract::read::{SourceArtifactClassifier, SourceMapDiscovery};

#[derive(Clone)]
pub struct WorkspacePolicies {
    pub(crate) discovery: Arc<dyn SourceMapDiscovery>,
    pub(crate) artifacts: Arc<dyn SourceArtifactClassifier>,
}

impl WorkspacePolicies {
    pub fn new(
        discovery: Arc<dyn SourceMapDiscovery>,
        artifacts: Arc<dyn SourceArtifactClassifier>,
    ) -> Self {
        Self {
            discovery,
            artifacts,
        }
    }
}
