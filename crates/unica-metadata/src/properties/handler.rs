//! `Handler`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = patterned_string_property(
    "Handler",
    MetaPropertyKey::Handler,
    EVENT_SUBSCRIPTION_KINDS,
    r"^CommonModule\.[^.]+\.[^.]+$",
);
