//! `NumberAllowedLength`: its typed property specification and checks.
use super::*;

const NUMBER_ALLOWED_LENGTH_VALUES: &[&str] = &["Variable", "Fixed"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "NumberAllowedLength",
    "NumberAllowedLength",
    MetaPropertyKey::NumberAllowedLength,
    DOCUMENT_NUMBER_KINDS,
    NUMBER_ALLOWED_LENGTH_VALUES,
)
.written_in_place(InPlace::Owners(&[
    MetadataKind::BusinessProcess,
    MetadataKind::Task,
]));
