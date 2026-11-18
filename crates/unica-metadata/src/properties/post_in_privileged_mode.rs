//! `PostInPrivilegedMode`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "PostInPrivilegedMode",
    MetaPropertyKey::PostInPrivilegedMode,
    MetaPropertyValueKind::Boolean,
    DOCUMENT_KINDS,
);
