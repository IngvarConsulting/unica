//! «ОпределяемыйТип», `DefinedType`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ОпределяемыйТип"],
    metadata: Some(MetadataDeclaration::new(&[
        "DefinedTypes",
        "ОпределяемыеТипы",
    ])),
    root: Some(RootPlace {
        child_objects: 2200,
        tree: Some(1400),
    }),
    layout: Some(LayoutDeclaration {
        directory: "DefinedTypes",
        display_name_ru: "Определяемые типы",
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[("DefinedType", "DefinedType")]),
    ..KindDeclaration::new("DefinedType")
};
