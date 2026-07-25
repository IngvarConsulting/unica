use std::path::PathBuf;

use crate::domain::{
    navigation::NavigationEnvelope,
    source_adapters::{AdapterManifest, SourceAdapterError, SourceDescriptor},
};

pub(crate) mod registry;
pub(crate) mod platform_xml;

pub(crate) struct SourceInput {
    pub(crate) workspace_root: PathBuf,
    pub(crate) target: PathBuf,
    pub(crate) configured_source_set: Option<String>,
}

pub(crate) enum ProbeOutcome {
    NoMatch,
    Match(SourceDescriptor),
}

pub(crate) trait SourceProbe: Send + Sync {
    fn probe(&self, input: &SourceInput) -> Result<ProbeOutcome, SourceAdapterError>;
}

pub(crate) trait SourceReadAdapter: Send + Sync {
    fn manifest(&self) -> &AdapterManifest;

    fn inspect(
        &self,
        input: &SourceInput,
        descriptor: &SourceDescriptor,
    ) -> Result<NavigationEnvelope, SourceAdapterError>;
}
