//! `Predefined`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "Predefined",
    MetaPropertyKey::Predefined,
    MetaPropertyValueKind::Boolean,
    SCHEDULED_JOB_KINDS,
);
