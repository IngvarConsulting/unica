//! `EnableTotalsSplitting`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "EnableTotalsSplitting",
    MetaPropertyKey::EnableTotalsSplitting,
    MetaPropertyValueKind::Boolean,
    ACCUMULATION_REGISTER_KINDS,
);
