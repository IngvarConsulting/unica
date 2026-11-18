//! `RootURL`: its typed property specification and checks.
// serde-rename: RootURL
use super::*;

const HTTP_SERVICE_KINDS: &[MetadataKind] = &[MetadataKind::HTTPService];

pub(super) const SPEC: MetadataPropertySpec = property(
    "RootURL",
    MetaPropertyKey::RootUrl,
    MetaPropertyValueKind::String,
    HTTP_SERVICE_KINDS,
);
