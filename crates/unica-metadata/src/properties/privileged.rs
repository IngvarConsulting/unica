//! `Privileged`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "Privileged",
    MetaPropertyKey::Privileged,
    MetaPropertyValueKind::Boolean,
    COMMON_MODULE_KINDS,
);
