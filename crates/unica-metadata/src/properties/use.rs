//! `Use`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "Use",
    MetaPropertyKey::Use,
    MetaPropertyValueKind::Boolean,
    SCHEDULED_JOB_KINDS,
);
