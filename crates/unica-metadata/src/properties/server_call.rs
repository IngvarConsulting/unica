//! `ServerCall`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "ServerCall",
    MetaPropertyKey::ServerCall,
    MetaPropertyValueKind::Boolean,
    COMMON_MODULE_KINDS,
);
