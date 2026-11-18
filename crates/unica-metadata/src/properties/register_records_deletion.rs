//! `RegisterRecordsDeletion`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "RegisterRecordsDeletion",
    "RegisterRecordsDeletion",
    MetaPropertyKey::RegisterRecordsDeletion,
    DOCUMENT_KINDS,
    REGISTER_RECORDS_DELETION_VALUES,
);
