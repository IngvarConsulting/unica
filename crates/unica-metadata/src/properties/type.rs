//! `Type`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "Type",
    MetaPropertyKey::Type,
    MetaPropertyValueKind::Type,
    &[MetadataKind::Constant, MetadataKind::DefinedType],
);
