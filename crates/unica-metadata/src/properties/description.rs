//! `Description`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "Description",
    MetaPropertyKey::Description,
    MetaPropertyValueKind::String,
    SCHEDULED_JOB_KINDS,
);
