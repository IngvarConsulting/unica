//! `MainFilterOnPeriod`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "MainFilterOnPeriod",
    MetaPropertyKey::MainFilterOnPeriod,
    MetaPropertyValueKind::Boolean,
    INFORMATION_REGISTER_KINDS,
);
