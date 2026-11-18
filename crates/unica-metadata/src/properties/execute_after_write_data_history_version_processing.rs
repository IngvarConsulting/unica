//! `ExecuteAfterWriteDataHistoryVersionProcessing`: its typed property specification.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "ExecuteAfterWriteDataHistoryVersionProcessing",
    MetaPropertyKey::ExecuteAfterWriteDataHistoryVersionProcessing,
    MetaPropertyValueKind::Boolean,
    DATA_HISTORY_KINDS,
);
