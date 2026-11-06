//! `SequenceFilling`: its typed property specification.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = enum_property("SequenceFilling", "SequenceFilling", MetaPropertyKey::SequenceFilling, DOCUMENT_KINDS, &["AutoFill", "AutoFillOff"]);
