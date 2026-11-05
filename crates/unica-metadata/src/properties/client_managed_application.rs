//! `ClientManagedApplication`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "ClientManagedApplication",
    MetaPropertyKey::ClientManagedApplication,
    MetaPropertyValueKind::Boolean,
    COMMON_MODULE_KINDS,
);
