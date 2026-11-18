//! `ExternalConnection`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "ExternalConnection",
    MetaPropertyKey::ExternalConnection,
    MetaPropertyValueKind::Boolean,
    COMMON_MODULE_KINDS,
);
