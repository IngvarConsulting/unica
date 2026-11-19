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
        child_objects_8_3_27: false,
    }),
    generated_types_8_3_27: Some(&[]),
    ..KindDeclaration::new("Bot")
};
