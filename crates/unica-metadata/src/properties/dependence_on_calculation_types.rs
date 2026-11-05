//! `DependenceOnCalculationTypes`: its typed property specification and checks.
use super::*;

const DEPENDENCE_VALUES: &[&str] = &["DontUse", "OnActionPeriod"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "DependenceOnCalculationTypes",
    "DependenceOnCalculationTypes",
    MetaPropertyKey::DependenceOnCalculationTypes,
    CHART_OF_CALCULATION_TYPES_KINDS,
    DEPENDENCE_VALUES,
);
