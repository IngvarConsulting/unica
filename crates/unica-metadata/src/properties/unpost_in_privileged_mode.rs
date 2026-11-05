//! `UnpostInPrivilegedMode`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "UnpostInPrivilegedMode",
    MetaPropertyKey::UnpostInPrivilegedMode,
    MetaPropertyValueKind::Boolean,
    DOCUMENT_KINDS,
);
