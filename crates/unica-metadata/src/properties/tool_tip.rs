//! `ToolTip`: its typed property specification and checks.
use super::*;

/// Многоязычная подсказка, записывается как синоним.
pub(super) const SPEC: MetadataPropertySpec = property(
    "ToolTip",
    MetaPropertyKey::ToolTip,
    MetaPropertyValueKind::String,
    &[MetadataKind::CommandGroup],
);
