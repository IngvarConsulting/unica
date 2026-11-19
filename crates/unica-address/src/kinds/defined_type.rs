//! «ОпределяемыйТип», `DefinedType`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ОпределяемыйТип"],
    metadata: Some(MetadataDeclaration::new(&["DefinedTypes", "ОпределяемыеТипы"])),
    root: Some(RootPlace { child_objects: 2200, tree: Some(1400) }),
    layout: Some(LayoutDeclaration {
        directory: "DefinedTypes",
        display_name_ru: "Определяемые типы",
        child_objects_8_3_27: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[
        ("DefinedType", "DefinedType"),
    ]),
    ..KindDeclaration::new("DefinedType")
};
