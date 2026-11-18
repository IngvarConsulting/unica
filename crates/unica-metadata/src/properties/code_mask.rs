//! `CodeMask`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "CodeMask",
    MetaPropertyKey::CodeMask,
    MetaPropertyValueKind::String,
    CHART_OF_ACCOUNTS_KINDS,
);
