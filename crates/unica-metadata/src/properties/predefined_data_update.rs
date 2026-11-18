//! `PredefinedDataUpdate`: its typed property specification and checks.
use super::*;

const PREDEFINED_DATA_UPDATE_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
];
const PREDEFINED_DATA_UPDATE_VALUES: &[&str] = &["Auto", "AutoUpdate", "DontAutoUpdate"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "PredefinedDataUpdate",
    "PredefinedDataUpdate",
    MetaPropertyKey::PredefinedDataUpdate,
    PREDEFINED_DATA_UPDATE_KINDS,
    PREDEFINED_DATA_UPDATE_VALUES,
)
.written_in_place(InPlace::Always);
