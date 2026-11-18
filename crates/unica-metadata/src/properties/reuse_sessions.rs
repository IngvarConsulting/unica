//! `ReuseSessions`: its typed property specification and checks.
use super::*;

const REUSE_SESSIONS_VALUES: &[&str] = &["DontUse", "AutoUse"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "ReuseSessions",
    "ReuseSessions",
    MetaPropertyKey::ReuseSessions,
    WEB_SERVICE_SESSION_KINDS,
    REUSE_SESSIONS_VALUES,
);
