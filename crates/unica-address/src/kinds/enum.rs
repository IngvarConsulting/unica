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
        requires_child_objects: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[
        ("EnumRef", "Ref"),
        ("EnumManager", "Manager"),
        ("EnumList", "List"),
    ]),
    child_order: Some(&["EnumValue", "Form", "Template", "Command"]),
    ..KindDeclaration::new("Enum")
};
