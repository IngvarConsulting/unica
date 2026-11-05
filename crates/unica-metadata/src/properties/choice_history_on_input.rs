//! `ChoiceHistoryOnInput`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "ChoiceHistoryOnInput",
    "ChoiceHistoryOnInput",
    MetaPropertyKey::ChoiceHistoryOnInput,
    &[MetadataKind::Catalog, MetadataKind::Document],
    &["Auto", "DontUse"],
)
.written_in_place(InPlace::Always);
