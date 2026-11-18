//! `BasePeriod`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "BasePeriod",
    MetaPropertyKey::BasePeriod,
    MetaPropertyValueKind::Boolean,
    CALCULATION_REGISTER_KINDS,
);
