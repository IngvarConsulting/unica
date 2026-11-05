//! `UpdateDataHistoryImmediatelyAfterWrite`: its typed property specification.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "UpdateDataHistoryImmediatelyAfterWrite",
    MetaPropertyKey::UpdateDataHistoryImmediatelyAfterWrite,
    MetaPropertyValueKind::Boolean,
    DATA_HISTORY_KINDS,
);
