//! «РегистрРасчета», `CalculationRegister`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["РегистрРасчета", "РегистрРасчёта"],
    metadata: Some(MetadataDeclaration::new(&["CalculationRegisters", "РегистрыРасчета", "РегистрыРасчёта"]).manager()),
    root: Some(RootPlace { child_objects: 4400, tree: Some(4600) }),
    layout: Some(LayoutDeclaration {
        directory: "CalculationRegisters",
        display_name_ru: "Регистры расчёта",
        child_objects_8_3_27: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[
        ("CalculationRegisterRecord", "Record"),
        ("CalculationRegisterManager", "Manager"),
        ("CalculationRegisterSelection", "Selection"),
        ("CalculationRegisterList", "List"),
        ("CalculationRegisterRecordSet", "RecordSet"),
        ("CalculationRegisterRecordKey", "RecordKey"),
        ("RecalculationsManager", "Recalcs"),
    ]),
    child_order: Some(&["Resource", "Attribute", "Dimension", "Recalculation", "Form", "Template", "Command"]),
    ..KindDeclaration::new("CalculationRegister")
};
