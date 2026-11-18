//! `SessionMaxAge`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "SessionMaxAge",
    MetaPropertyKey::SessionMaxAge,
    MetaPropertyValueKind::UnsignedInteger,
    WEB_SERVICE_SESSION_KINDS,
);
