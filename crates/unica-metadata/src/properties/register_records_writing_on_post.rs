//! `RegisterRecordsWritingOnPost`: its typed property specification and checks.
use super::*;

const REGISTER_RECORDS_WRITING_VALUES: &[&str] = &["WriteModified", "WriteSelected", "WriteAll"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "RegisterRecordsWritingOnPost",
    "RegisterRecordsWritingOnPost",
    MetaPropertyKey::RegisterRecordsWritingOnPost,
    DOCUMENT_KINDS,
    REGISTER_RECORDS_WRITING_VALUES,
);
