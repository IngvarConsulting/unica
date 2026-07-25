use std::collections::BTreeSet;

use crate::domain::source_adapters::{
    AdapterManifest, AdapterMaturity, FormatRange, FormatVersion, SourceAccess, SourceFamily,
};

pub(crate) mod decoder;
pub(crate) mod native_model;
pub(crate) mod probe;
pub(crate) mod provider;
pub(crate) mod schema;

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
        maturity: AdapterMaturity::ProbeComplete,
    }
}
