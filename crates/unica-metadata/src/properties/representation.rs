//! `Representation`: its typed property specification and checks.
use super::*;

/// ButtonRepresentation платформы.
const REPRESENTATION_VALUES: &[&str] = &["Auto", "Text", "Picture", "PictureAndText"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "Representation",
    "Representation",
    MetaPropertyKey::Representation,
    &[MetadataKind::CommandGroup],
    REPRESENTATION_VALUES,
);
