//! `SearchStringModeOnInputByString`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "SearchStringModeOnInputByString",
    "SearchStringModeOnInputByString",
    MetaPropertyKey::SearchStringModeOnInputByString,
    &[MetadataKind::Catalog, MetadataKind::Document],
    &["Begin", "AnyPart"],
)
.written_in_place(InPlace::Always);
