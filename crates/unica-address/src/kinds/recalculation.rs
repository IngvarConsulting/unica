//! «Перерасчет», `Recalculation`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Перерасчет", "Перерасчёт", "Перерасчеты", "Перерасчёты"],
    nested: Some(NestedPlace {
        owners: &["CalculationRegister"],
        directory: Some("Recalculations"),
        modules: &["RecordSetModule"],
    }),
    generated_types: Some(&[("RecalculationRecordSet", "RecordSet")]),
    ..KindDeclaration::new("Recalculation")
};
