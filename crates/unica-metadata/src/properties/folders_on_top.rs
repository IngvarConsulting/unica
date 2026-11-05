//! `FoldersOnTop`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "FoldersOnTop",
    MetaPropertyKey::FoldersOnTop,
    MetaPropertyValueKind::Boolean,
    CATALOG_KINDS,
);
