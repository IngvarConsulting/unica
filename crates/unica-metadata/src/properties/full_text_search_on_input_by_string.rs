//! `FullTextSearchOnInputByString`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "FullTextSearchOnInputByString",
    "FullTextSearchOnInputByString",
    MetaPropertyKey::FullTextSearchOnInputByString,
    &[MetadataKind::Catalog, MetadataKind::Document],
    &["Use", "DontUse"],
)
.written_in_place(InPlace::Always);
