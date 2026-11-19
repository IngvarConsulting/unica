//! «Язык», `Language`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Язык"],
    metadata: Some(MetadataDeclaration::new(&["Languages", "Языки"])),
    root: Some(RootPlace {
        child_objects: 100,
        tree: Some(3000),
    }),
    layout: Some(LayoutDeclaration {
        directory: "Languages",
        display_name_ru: "Языки",
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    ..KindDeclaration::new("Language")
};
