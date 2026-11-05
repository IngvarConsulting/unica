//! `DataHistory`: its typed property specification.
use super::*;

const DATA_HISTORY_VALUES: &[&str] = &["Use", "DontUse"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "DataHistory",
    "DataHistory",
    MetaPropertyKey::DataHistory,
    DATA_HISTORY_KINDS,
    DATA_HISTORY_VALUES,
);
