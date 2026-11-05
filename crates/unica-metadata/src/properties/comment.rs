//! `Comment`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "Comment",
    MetaPropertyKey::Comment,
    MetaPropertyValueKind::String,
    MetadataKind::ALL,
);
