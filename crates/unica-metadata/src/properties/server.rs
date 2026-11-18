//! `Server`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "Server",
    MetaPropertyKey::Server,
    MetaPropertyValueKind::Boolean,
    COMMON_MODULE_KINDS,
);
