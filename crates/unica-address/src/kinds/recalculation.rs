//! «Перерасчет», `Recalculation`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Перерасчет", "Перерасчёт", "Перерасчеты", "Перерасчёты"],
    generated_types_8_3_27: Some(&[
        ("RecalculationRecordSet", "RecordSet"),
    ]),
    ..KindDeclaration::new("Recalculation")
};
