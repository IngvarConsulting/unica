//! `CreateOnInput`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "CreateOnInput",
    "CreateOnInput",
    MetaPropertyKey::CreateOnInput,
    &[MetadataKind::Catalog, MetadataKind::Document],
    &["Auto", "Use", "DontUse"],
)
.written_in_place(InPlace::Always);
