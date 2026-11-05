//! `DefaultPresentation`: its typed property specification and checks.
use super::*;

const DEFAULT_PRESENTATION_VALUES: &[&str] = &["AsDescription", "AsCode"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "DefaultPresentation",
    "DefaultPresentation",
    MetaPropertyKey::DefaultPresentation,
    CATALOG_KINDS,
    DEFAULT_PRESENTATION_VALUES,
);
