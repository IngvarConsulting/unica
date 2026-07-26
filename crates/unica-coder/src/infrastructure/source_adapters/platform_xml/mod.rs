use std::collections::BTreeSet;

use crate::domain::source_adapters::{
    AdapterManifest, AdapterMaturity, FormatRange, FormatVersion, SourceAccess, SourceAdapterError,
    SourceFamily,
};
use crate::{
    domain::navigation::NavigationEnvelope,
    infrastructure::source_adapters::{
        CaptureOutcome, CapturedSourceSession, SourceCaptureAdapter, SourceInput, SourceReadAdapter,
    },
};
use std::any::Any;

pub(crate) mod decoder;
pub(crate) mod native_model;
pub(crate) mod probe;
pub(crate) mod projector;
pub(crate) mod provider;
pub(crate) mod schema;
pub(crate) mod support;

pub(crate) fn manifest() -> AdapterManifest {
    AdapterManifest {
        adapter_id: "platform-xml-2.20",
        adapter_version: env!("CARGO_PKG_VERSION"),
        source_family: SourceFamily::PlatformXml,
        supported_formats: vec![FormatRange::exact(
            FormatVersion::parse("2.20").expect("constant version"),
        )],
        required_features: BTreeSet::new(),
        excluded_features: BTreeSet::new(),
        source_access: SourceAccess::ReadOnly,
        maturity: AdapterMaturity::ReadCompatible,
    }
}

pub(crate) struct PlatformXmlReadAdapter {
    manifest: AdapterManifest,
}

pub(crate) struct PlatformXmlCapturedSession {
    provider: provider::PlatformXmlProvider,
    descriptor_key: String,
    evidence: Vec<String>,
}

impl PlatformXmlCapturedSession {
    pub(crate) fn provider(&self) -> &provider::PlatformXmlProvider {
        &self.provider
    }

    pub(crate) fn descriptor_key(&self) -> &str {
        &self.descriptor_key
    }
}

impl CapturedSourceSession for PlatformXmlCapturedSession {
    fn source_family(&self) -> SourceFamily {
        SourceFamily::PlatformXml
    }

    fn revision(
        &self,
    ) -> Result<crate::domain::source_adapters::SourceRevision, SourceAdapterError> {
        self.provider.revision()
    }

    fn evidence(&self) -> &[String] {
        &self.evidence
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub(crate) struct PlatformXmlCaptureAdapter;

impl PlatformXmlCaptureAdapter {
    pub(crate) const fn new() -> Self {
        Self
    }
}

impl SourceCaptureAdapter for PlatformXmlCaptureAdapter {
    fn capture(&self, input: &SourceInput) -> Result<CaptureOutcome, SourceAdapterError> {
        let descriptor_key = input
            .target
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                SourceAdapterError::new(
                    crate::domain::source_adapters::SourceAdapterErrorKind::SourceUnavailable,
                    "Platform XML descriptor does not have a UTF-8 file name",
                )
            })?
            .to_string();
        let provider = provider::PlatformXmlProvider::capture(&input.target, &input.source_root)?;
        Ok(CaptureOutcome::Captured(Box::new(
            PlatformXmlCapturedSession {
                provider,
                descriptor_key,
                evidence: vec!["platform-xml:immutable-capture".to_string()],
            },
        )))
    }
}

impl PlatformXmlReadAdapter {
    pub(crate) fn new() -> Self {
        Self {
            manifest: manifest(),
        }
    }

    pub(crate) fn inspect_provider(
        &self,
        provider: &provider::PlatformXmlProvider,
        descriptor: &crate::domain::source_adapters::SourceDescriptor,
    ) -> Result<NavigationEnvelope, SourceAdapterError> {
        let native = decoder::decode(provider, descriptor)?;
        let support_bytes = provider.parent_configurations_bytes();
        let support = match support_bytes.as_deref() {
            None => support::read_support_facts_bytes(None),
            Some(bytes) => match provider.configuration_uuid() {
                Ok(configuration_uuid) => support::read_support_facts_bytes_for_configuration(
                    Some(bytes),
                    &configuration_uuid,
                ),
                Err(_) => support::unreadable_configuration_evidence(),
            },
        };
        projector::project(&native, &support)
    }
}

impl SourceReadAdapter for PlatformXmlReadAdapter {
    fn manifest(&self) -> &AdapterManifest {
        &self.manifest
    }

    fn inspect_captured(
        &self,
        session: &dyn CapturedSourceSession,
        descriptor: &crate::domain::source_adapters::SourceDescriptor,
    ) -> Result<NavigationEnvelope, SourceAdapterError> {
        let session = session
            .as_any()
            .downcast_ref::<PlatformXmlCapturedSession>()
            .ok_or_else(|| {
                SourceAdapterError::new(
                    crate::domain::source_adapters::SourceAdapterErrorKind::SourceUnavailable,
                    "captured source session does not contain Platform XML evidence",
                )
            })?;
        self.inspect_provider(session.provider(), descriptor)
    }
}
