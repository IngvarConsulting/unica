//! `Periodicity`: its typed property specification and checks.
use super::*;

const INFORMATION_REGISTER_PERIODICITY_VALUES: &[&str] = &[
    "Nonperiodical",
    "Second",
    "Day",
    "Month",
    "Quarter",
    "Year",
    "RecorderPosition",
];
const CALCULATION_REGISTER_PERIODICITY_VALUES: &[&str] = &["Day", "Month", "Quarter", "Year"];

pub(super) const SPEC_1: MetadataPropertySpec = enum_property(
    "Periodicity",
    "InformationRegisterPeriodicity",
    MetaPropertyKey::Periodicity,
    INFORMATION_REGISTER_KINDS,
    INFORMATION_REGISTER_PERIODICITY_VALUES,
);

pub(super) const SPEC_2: MetadataPropertySpec = enum_property(
    "Periodicity",
    "Periodicity",
    MetaPropertyKey::Periodicity,
    CALCULATION_REGISTER_KINDS,
    CALCULATION_REGISTER_PERIODICITY_VALUES,
);
