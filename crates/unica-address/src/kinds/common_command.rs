//! «ОбщаяКоманда», `CommonCommand`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ОбщаяКоманда"],
    metadata: Some(MetadataDeclaration::new(&["CommonCommands", "ОбщиеКоманды"])),
    root: Some(RootPlace { child_objects: 2600, tree: Some(1600) }),
    layout: Some(LayoutDeclaration {
        directory: "CommonCommands",
        display_name_ru: "Общие команды",
        child_objects_8_3_27: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[]),
    ..KindDeclaration::new("CommonCommand")
};
