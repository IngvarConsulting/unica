//! `NumberType`: its typed property specification and checks.
use super::*;

const NUMBER_TYPE_VALUES: &[&str] = &["String", "Number"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "NumberType",
    "NumberType",
    MetaPropertyKey::NumberType,
    DOCUMENT_NUMBER_KINDS,
    NUMBER_TYPE_VALUES,
);
