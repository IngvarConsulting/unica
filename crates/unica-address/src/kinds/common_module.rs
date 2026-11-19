//! «ОбщийМодуль», `CommonModule`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ОбщийМодуль"],
    metadata: Some(MetadataDeclaration::new(&["CommonModules", "ОбщиеМодули"])),
    root: Some(RootPlace {
        child_objects: 1000,
        tree: Some(200),
    }),
    layout: Some(LayoutDeclaration {
        directory: "CommonModules",
        display_name_ru: "Общие модули",
        child_objects_8_3_27: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[]),
    ..KindDeclaration::new("CommonModule")
};
