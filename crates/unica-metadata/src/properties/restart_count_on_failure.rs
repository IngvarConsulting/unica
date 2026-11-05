//! `RestartCountOnFailure`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "RestartCountOnFailure",
    MetaPropertyKey::RestartCountOnFailure,
    MetaPropertyValueKind::UnsignedInteger,
    SCHEDULED_JOB_KINDS,
);
