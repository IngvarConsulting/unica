//! `SubordinationUse`: its typed property specification and checks.
use super::*;

const SUBORDINATION_USE_VALUES: &[&str] = &["ToFolders", "ToFoldersAndItems", "ToItems"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "SubordinationUse",
    "SubordinationUse",
    MetaPropertyKey::SubordinationUse,
    CATALOG_KINDS,
    SUBORDINATION_USE_VALUES,
);
