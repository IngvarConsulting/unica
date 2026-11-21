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
    // Конфигуратор 8.3.27 с русским интерфейсом даёт новому языку код «ru1»,
    // следующему — «ru2», независимо от языков конфигурации: первое
    // свободное N. Расширение языки заимствует.
    creation: Some(CreationDeclaration {
        numbered_language_code: Some("ru"),
        main_configuration_only: true,
        ..CreationDeclaration::properties(&[])
    }),
    ..KindDeclaration::new("Language")
};
