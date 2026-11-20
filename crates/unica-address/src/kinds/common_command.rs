//! «ОбщаяКоманда», `CommonCommand`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ОбщаяКоманда"],
    metadata: Some(MetadataDeclaration::new(&[
        "CommonCommands",
        "ОбщиеКоманды",
    ])),
    root: Some(RootPlace {
        child_objects: 2600,
        tree: Some(1600),
    }),
    layout: Some(LayoutDeclaration {
        directory: "CommonCommands",
        display_name_ru: "Общие команды",
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    ..KindDeclaration::new("CommonCommand")
};
