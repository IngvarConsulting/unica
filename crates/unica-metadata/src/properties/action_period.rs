//! `ActionPeriod`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "ActionPeriod",
    MetaPropertyKey::ActionPeriod,
    MetaPropertyValueKind::Boolean,
    CALCULATION_REGISTER_KINDS,
);
