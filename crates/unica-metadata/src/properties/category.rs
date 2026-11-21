//! `Category`: its typed property specification and checks.
use super::*;

/// CommandGroupCategory платформы.
const CATEGORY_VALUES: &[&str] = &[
    "NavigationPanel",
    "ActionsPanel",
    "FormNavigationPanel",
    "FormCommandBar",
];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "Category",
    "Category",
    MetaPropertyKey::Category,
    &[MetadataKind::CommandGroup],
    CATEGORY_VALUES,
);
