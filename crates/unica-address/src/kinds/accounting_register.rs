//! «РегистрБухгалтерии», `AccountingRegister`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["РегистрБухгалтерии"],
    metadata: Some(
        MetadataDeclaration::new(&["AccountingRegisters", "РегистрыБухгалтерии"]).manager(),
    ),
    root: Some(RootPlace {
        child_objects: 4200,
        tree: Some(4500),
    }),
    layout: Some(LayoutDeclaration {
        directory: "AccountingRegisters",
        display_name_ru: "Регистры бухгалтерии",
        requires_child_objects: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[
        ("AccountingRegisterRecord", "Record"),
        ("AccountingRegisterExtDimensions", "ExtDimensions"),
        ("AccountingRegisterRecordSet", "RecordSet"),
        ("AccountingRegisterRecordKey", "RecordKey"),
        ("AccountingRegisterSelection", "Selection"),
        ("AccountingRegisterList", "List"),
        ("AccountingRegisterManager", "Manager"),
    ]),
    child_order: Some(&[
        "Dimension",
        "Resource",
        "Attribute",
        "Form",
        "Template",
        "Command",
    ]),
    ..KindDeclaration::new("AccountingRegister")
};
