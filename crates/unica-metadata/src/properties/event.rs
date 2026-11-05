//! `Event`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = patterned_string_property(
    "Event",
    MetaPropertyKey::Event,
    EVENT_SUBSCRIPTION_KINDS,
    "^[A-Z][A-Za-z0-9]*$",
);
