//! `HierarchyType`: its typed property specification and checks.
use super::*;

const HIERARCHY_TYPE_VALUES: &[&str] = &["HierarchyFoldersAndItems", "HierarchyOfItems"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "HierarchyType",
    "HierarchyType",
    MetaPropertyKey::HierarchyType,
    CATALOG_KINDS,
    HIERARCHY_TYPE_VALUES,
);
