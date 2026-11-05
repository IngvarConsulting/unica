//! `Posting`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "Posting",
    "Posting",
    MetaPropertyKey::Posting,
    DOCUMENT_KINDS,
    POSTING_VALUES,
);
