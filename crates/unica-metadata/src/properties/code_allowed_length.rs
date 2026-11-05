//! `CodeAllowedLength`: its typed property specification and checks.
use super::*;

const CODE_ALLOWED_LENGTH_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::ExchangePlan,
];
const CODE_ALLOWED_LENGTH_VALUES: &[&str] = &["Variable", "Fixed"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "CodeAllowedLength",
    "CodeAllowedLength",
    MetaPropertyKey::CodeAllowedLength,
    CODE_ALLOWED_LENGTH_KINDS,
    CODE_ALLOWED_LENGTH_VALUES,
)
.written_in_place(InPlace::Owners(&[
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::ExchangePlan,
]));
