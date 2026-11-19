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
        child_objects_8_3_27: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[]),
    ..KindDeclaration::new("Style")
};
