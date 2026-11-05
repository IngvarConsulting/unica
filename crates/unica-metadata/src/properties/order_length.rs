//! `OrderLength`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "OrderLength",
    MetaPropertyKey::OrderLength,
    MetaPropertyValueKind::UnsignedInteger,
    CHART_OF_ACCOUNTS_KINDS,
);
