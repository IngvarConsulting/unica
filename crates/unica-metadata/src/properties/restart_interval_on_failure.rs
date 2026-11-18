//! `RestartIntervalOnFailure`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "RestartIntervalOnFailure",
    MetaPropertyKey::RestartIntervalOnFailure,
    MetaPropertyValueKind::UnsignedInteger,
    SCHEDULED_JOB_KINDS,
);
