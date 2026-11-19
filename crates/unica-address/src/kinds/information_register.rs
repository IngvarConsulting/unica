//! «РегистрСведений», `InformationRegister`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["РегистрСведений"],
    metadata: Some(
        MetadataDeclaration::new(&["InformationRegisters", "РегистрыСведений"]).manager(),
    ),
    root: Some(RootPlace {
        child_objects: 3800,
        tree: Some(4300),
    }),
    layout: Some(LayoutDeclaration {
        directory: "InformationRegisters",
        display_name_ru: "Регистры сведений",
        requires_child_objects: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[
        ("InformationRegisterRecord", "Record"),
        ("InformationRegisterManager", "Manager"),
        ("InformationRegisterSelection", "Selection"),
        ("InformationRegisterList", "List"),
        ("InformationRegisterRecordSet", "RecordSet"),
        ("InformationRegisterRecordKey", "RecordKey"),
        ("InformationRegisterRecordManager", "RecordManager"),
    ]),
    child_order: Some(&[
        "Resource",
        "Attribute",
        "Dimension",
        "Form",
        "Template",
        "Command",
    ]),
    ..KindDeclaration::new("InformationRegister")
};
