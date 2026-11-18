//! `ChoiceDataGetModeOnInputByString`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "ChoiceDataGetModeOnInputByString",
    "ChoiceDataGetModeOnInputByString",
    MetaPropertyKey::ChoiceDataGetModeOnInputByString,
    &[MetadataKind::Catalog, MetadataKind::Document],
    &["Directly", "Background"],
)
.written_in_place(InPlace::Always);
