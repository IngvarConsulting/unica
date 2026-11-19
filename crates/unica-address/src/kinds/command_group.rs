//! «ГруппаКоманд», `CommandGroup`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ГруппаКоманд"],
    metadata: Some(MetadataDeclaration::new(&["CommandGroups", "ГруппыКоманд"])),
    root: Some(RootPlace {
        child_objects: 2700,
        tree: Some(1700),
    }),
    layout: Some(LayoutDeclaration {
        directory: "CommandGroups",
        display_name_ru: "Группы команд",
        child_objects_8_3_27: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[]),
    ..KindDeclaration::new("CommandGroup")
};
