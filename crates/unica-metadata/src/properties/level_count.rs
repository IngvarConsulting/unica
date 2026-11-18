//! `LevelCount`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "LevelCount",
    MetaPropertyKey::LevelCount,
    MetaPropertyValueKind::UnsignedInteger,
    CATALOG_KINDS,
);
