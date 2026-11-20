//! «Перерасчет», `Recalculation`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Перерасчет", "Перерасчёт", "Перерасчеты", "Перерасчёты"],
    generated_types: Some(&[("RecalculationRecordSet", "RecordSet")]),
    ..KindDeclaration::new("Recalculation")
};
