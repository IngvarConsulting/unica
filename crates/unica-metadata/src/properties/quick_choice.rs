//! `QuickChoice`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "QuickChoice",
    MetaPropertyKey::QuickChoice,
    MetaPropertyValueKind::Boolean,
    CATALOG_KINDS,
);
