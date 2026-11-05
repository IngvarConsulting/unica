//! `NumberPeriodicity`: its typed property specification and checks.
use super::*;

const NUMBER_PERIODICITY_VALUES: &[&str] = &["Day", "Month", "Nonperiodical", "Quarter", "Year"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "NumberPeriodicity",
    "NumberPeriodicity",
    MetaPropertyKey::NumberPeriodicity,
    NUMBER_PERIODICITY_KINDS,
    NUMBER_PERIODICITY_VALUES,
)
.written_in_place(InPlace::Owners(&[MetadataKind::BusinessProcess]));
