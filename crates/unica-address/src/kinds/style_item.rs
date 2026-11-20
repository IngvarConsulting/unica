//! «ЭлементСтиля», `StyleItem`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ЭлементСтиля"],
    metadata: Some(MetadataDeclaration::new(&["StyleItems", "ЭлементыСтиля"])),
    root: Some(RootPlace {
        child_objects: 300,
        tree: Some(2800),
    }),
    layout: Some(LayoutDeclaration {
        directory: "StyleItems",
        display_name_ru: "Элементы стиля",
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    ..KindDeclaration::new("StyleItem")
};
