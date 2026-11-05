//! `ReturnValuesReuse`: its typed property specification and checks.
use super::*;

const RETURN_VALUES_REUSE_VALUES: &[&str] = &["DontUse", "DuringRequest", "DuringSession"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "ReturnValuesReuse",
    "ReturnValuesReuse",
    MetaPropertyKey::ReturnValuesReuse,
    COMMON_MODULE_KINDS,
    RETURN_VALUES_REUSE_VALUES,
);
