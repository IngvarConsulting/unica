//! `RealTimePosting`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "RealTimePosting",
    "RealTimePosting",
    MetaPropertyKey::RealTimePosting,
    DOCUMENT_KINDS,
    POSTING_VALUES,
);
