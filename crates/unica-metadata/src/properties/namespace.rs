//! `Namespace`: its typed property specification and checks.
use super::*;

const WEB_SERVICE_KINDS: &[MetadataKind] = &[MetadataKind::WebService];

pub(super) const SPEC: MetadataPropertySpec = property(
    "Namespace",
    MetaPropertyKey::Namespace,
    MetaPropertyValueKind::String,
    WEB_SERVICE_KINDS,
);
