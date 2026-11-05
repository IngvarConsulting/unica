//! `LimitLevelCount`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "LimitLevelCount",
    MetaPropertyKey::LimitLevelCount,
    MetaPropertyValueKind::Boolean,
    CATALOG_KINDS,
);
