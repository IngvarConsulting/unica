//! `ChoiceMode`: its typed property specification and checks.
use super::*;

const CHOICE_MODE_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::Enum,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ExchangePlan,
];
const CHOICE_MODE_VALUES: &[&str] = &["BothWays", "FromForm", "QuickChoice"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "ChoiceMode",
    "ChoiceMode",
    MetaPropertyKey::ChoiceMode,
    CHOICE_MODE_KINDS,
    CHOICE_MODE_VALUES,
)
.written_in_place(InPlace::Owners(&[
    MetadataKind::Enum,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ExchangePlan,
]));
