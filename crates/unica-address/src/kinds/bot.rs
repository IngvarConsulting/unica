//! «Бот», `Bot`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Бот"],
    metadata: Some(MetadataDeclaration::new(&["Bots", "Боты"])),
    root: Some(RootPlace {
        child_objects: 2300,
        tree: Some(1100),
    }),
    layout: Some(LayoutDeclaration {
        directory: "Bots",
        display_name_ru: "Боты",
        config_dump_prefix: Some("Bot"),
        config_dump_module_suffix: Some(".Module"),
        requires_child_objects: false,
    }),
    generated_types: Some(&[]),
    // Свойства нового объекта — как их пишет Конфигуратор 8.3.27 по «Добавить».
    creation: Some(CreationDeclaration::properties(&[
        "<Predefined>true</Predefined>",
        "<Picture/>",
    ])),
    ..KindDeclaration::new("Bot")
};
