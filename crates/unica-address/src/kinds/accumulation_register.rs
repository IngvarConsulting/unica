//! «РегистрНакопления», `AccumulationRegister`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["РегистрНакопления"],
    metadata: Some(MetadataDeclaration::new(&["AccumulationRegisters", "РегистрыНакопления"]).manager()),
    root: Some(RootPlace { child_objects: 3900, tree: Some(4400) }),
    layout: Some(LayoutDeclaration {
        directory: "AccumulationRegisters",
        display_name_ru: "Регистры накопления",
        child_objects_8_3_27: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[
        ("AccumulationRegisterRecord", "Record"),
        ("AccumulationRegisterManager", "Manager"),
        ("AccumulationRegisterSelection", "Selection"),
        ("AccumulationRegisterList", "List"),
        ("AccumulationRegisterRecordSet", "RecordSet"),
        ("AccumulationRegisterRecordKey", "RecordKey"),
    ]),
    child_order: Some(&["Resource", "Attribute", "Dimension", "Form", "Template", "Command"]),
    ..KindDeclaration::new("AccumulationRegister")
};
