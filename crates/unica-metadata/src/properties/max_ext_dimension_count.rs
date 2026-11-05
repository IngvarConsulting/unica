//! `MaxExtDimensionCount`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "MaxExtDimensionCount",
    MetaPropertyKey::MaxExtDimensionCount,
    MetaPropertyValueKind::UnsignedInteger,
    CHART_OF_ACCOUNTS_KINDS,
);
