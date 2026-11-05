//! `PeriodAdjustmentLength`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "PeriodAdjustmentLength",
    MetaPropertyKey::PeriodAdjustmentLength,
    MetaPropertyValueKind::UnsignedInteger,
    ACCOUNTING_REGISTER_KINDS,
);
