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
    ..KindDeclaration::new("Bot")
};
