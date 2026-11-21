//! «Стиль», `Style`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Стиль"],
    metadata: Some(MetadataDeclaration::new(&["Styles", "Стили"])),
    root: Some(RootPlace {
        child_objects: 400,
        tree: Some(2900),
    }),
    layout: Some(LayoutDeclaration {
        directory: "Styles",
        display_name_ru: "Стили",
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    // Свойства нового объекта — как их пишет Конфигуратор 8.3.27 по «Добавить».
    creation: Some(CreationDeclaration::properties(&[])),
    ..KindDeclaration::new("Style")
};
