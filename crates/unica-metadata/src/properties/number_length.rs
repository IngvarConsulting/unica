//! `NumberLength`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "NumberLength",
    MetaPropertyKey::NumberLength,
    MetaPropertyValueKind::UnsignedInteger,
    DOCUMENT_NUMBER_KINDS,
);
