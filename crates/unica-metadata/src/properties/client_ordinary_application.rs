//! `ClientOrdinaryApplication`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "ClientOrdinaryApplication",
    MetaPropertyKey::ClientOrdinaryApplication,
    MetaPropertyValueKind::Boolean,
    COMMON_MODULE_KINDS,
);
