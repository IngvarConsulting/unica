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
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    ..KindDeclaration::new("FunctionalOption")
};
