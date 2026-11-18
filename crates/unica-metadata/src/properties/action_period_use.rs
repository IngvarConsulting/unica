//! `ActionPeriodUse`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "ActionPeriodUse",
    MetaPropertyKey::ActionPeriodUse,
    MetaPropertyValueKind::Boolean,
    CHART_OF_CALCULATION_TYPES_KINDS,
);
