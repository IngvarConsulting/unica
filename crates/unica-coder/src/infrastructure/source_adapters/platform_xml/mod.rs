use std::collections::BTreeSet;

use crate::domain::source_adapters::{
    AdapterManifest, AdapterMaturity, FormatRange, FormatVersion, SourceAccess, SourceAdapterError,
    SourceFamily,
};
use crate::{
    domain::navigation::NavigationEnvelope,
    infrastructure::source_adapters::{SourceInput, SourceReadAdapter},
};

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
        let support = support::read_support_facts_bytes(support_bytes.as_deref());
        projector::project(&native, &support)
    }
}

impl SourceReadAdapter for PlatformXmlReadAdapter {
    fn manifest(&self) -> &AdapterManifest {
        &self.manifest
    }

    fn inspect(
        &self,
        input: &SourceInput,
        descriptor: &crate::domain::source_adapters::SourceDescriptor,
    ) -> Result<NavigationEnvelope, SourceAdapterError> {
        let root = input.target.parent().ok_or_else(|| {
            SourceAdapterError::new(
                crate::domain::source_adapters::SourceAdapterErrorKind::SourceUnavailable,
                "Platform XML descriptor has no aggregate root",
            )
        })?;
        let provider = provider::PlatformXmlProvider::open(root)?;
        self.inspect_provider(&provider, descriptor)
    }

    fn inspect_platform_xml_provider(
        &self,
        provider: &provider::PlatformXmlProvider,
        descriptor: &crate::domain::source_adapters::SourceDescriptor,
    ) -> Option<Result<NavigationEnvelope, SourceAdapterError>> {
        Some(self.inspect_provider(provider, descriptor))
    }
}
