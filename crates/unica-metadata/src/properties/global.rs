//! `Global`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "Global",
    MetaPropertyKey::Global,
    MetaPropertyValueKind::Boolean,
    COMMON_MODULE_KINDS,
);
