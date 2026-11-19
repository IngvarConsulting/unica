//! «ФункциональнаяОпция», `FunctionalOption`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ФункциональнаяОпция"],
    metadata: Some(MetadataDeclaration::new(&[
        "FunctionalOptions",
        "ФункциональныеОпции",
    ])),
    root: Some(RootPlace {
        child_objects: 2000,
        tree: Some(1200),
    }),
    layout: Some(LayoutDeclaration {
        directory: "FunctionalOptions",
        display_name_ru: "Функциональные опции",
        child_objects_8_3_27: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[]),
    ..KindDeclaration::new("FunctionalOption")
};
