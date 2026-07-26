use std::{any::Any, path::PathBuf};

use crate::domain::{
    navigation::NavigationEnvelope,
    source_adapters::{
        AdapterManifest, SourceAdapterError, SourceDescriptor, SourceFamily, SourceRevision,
    },
};

#[cfg(test)]
pub(crate) mod certification;
pub(crate) mod platform_xml;
pub(crate) mod registry;

pub(crate) struct SourceInput {
    pub(crate) workspace_root: PathBuf,
    /// Authorized canonical source root.  Capture adapters may use it for
    /// source-level evidence, but it is never part of a public snapshot.
    pub(crate) source_root: PathBuf,
    pub(crate) target: PathBuf,
    pub(crate) configured_source_set: Option<String>,
}

#[derive(Debug)]
pub(crate) enum ProbeOutcome {
    NoMatch,
    Match(SourceDescriptor),
}

/// Immutable source-family session captured before probing.  The common
/// boundary intentionally contains no family-specific provider type.
pub(crate) trait CapturedSourceSession: Send + Sync {
    fn source_family(&self) -> SourceFamily;
    fn revision(&self) -> Result<SourceRevision, SourceAdapterError>;
    fn evidence(&self) -> &[String];
    fn as_any(&self) -> &dyn Any;
}

pub(crate) enum CaptureOutcome {
    NoMatch,
    Captured(Box<dyn CapturedSourceSession>),
}

pub(crate) trait SourceCaptureAdapter: Send + Sync {
    fn capture(&self, input: &SourceInput) -> Result<CaptureOutcome, SourceAdapterError>;
}

pub(crate) trait SourceProbe: Send + Sync {
    fn probe(
        &self,
        input: &SourceInput,
        session: &dyn CapturedSourceSession,
    ) -> Result<ProbeOutcome, SourceAdapterError>;
}

pub(crate) trait SourceReadAdapter: Send + Sync {
    fn manifest(&self) -> &AdapterManifest;

    fn inspect_captured(
        &self,
        session: &dyn CapturedSourceSession,
        descriptor: &SourceDescriptor,
    ) -> Result<NavigationEnvelope, SourceAdapterError>;
}
