//! `Synonym`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "Synonym",
    MetaPropertyKey::Synonym,
    MetaPropertyValueKind::String,
    MetadataKind::ALL,
);
