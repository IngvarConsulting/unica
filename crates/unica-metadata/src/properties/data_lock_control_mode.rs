//! `DataLockControlMode`: its typed property specification and checks.
use super::*;

const DATA_LOCK_CONTROL_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::Document,
    MetadataKind::Constant,
    MetadataKind::InformationRegister,
    MetadataKind::AccumulationRegister,
    MetadataKind::AccountingRegister,
    MetadataKind::CalculationRegister,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::BusinessProcess,
    MetadataKind::Task,
    MetadataKind::ExchangePlan,
];
const OBJECT_DATA_LOCK_CONTROL_VALUES: &[&str] = &["Automatic", "Managed"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "DataLockControlMode",
    "DataLockControlMode",
    MetaPropertyKey::DataLockControlMode,
    DATA_LOCK_CONTROL_KINDS,
    OBJECT_DATA_LOCK_CONTROL_VALUES,
)
.written_in_place(InPlace::Always);
