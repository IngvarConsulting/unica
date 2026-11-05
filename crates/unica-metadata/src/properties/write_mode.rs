//! `WriteMode`: its typed property specification and checks.
use super::*;

const WRITE_MODE_VALUES: &[&str] = &["Independent", "RecorderSubordinate"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "WriteMode",
    "WriteMode",
    MetaPropertyKey::WriteMode,
    INFORMATION_REGISTER_KINDS,
    WRITE_MODE_VALUES,
);
