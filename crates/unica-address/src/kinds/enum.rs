//! «Перечисление», `Enum`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Перечисление"],
    metadata: Some(MetadataDeclaration::new(&["Enums", "Перечисления"]).manager()),
    root: Some(RootPlace {
        child_objects: 3500,
        tree: Some(3700),
    }),
    layout: Some(LayoutDeclaration {
        directory: "Enums",
        display_name_ru: "Перечисления",
        child_objects_8_3_27: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[
        ("EnumRef", "Ref"),
        ("EnumManager", "Manager"),
        ("EnumList", "List"),
    ]),
    child_order: Some(&["EnumValue", "Form", "Template", "Command"]),
    ..KindDeclaration::new("Enum")
};
