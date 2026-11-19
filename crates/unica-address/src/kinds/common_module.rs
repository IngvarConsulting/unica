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
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    ..KindDeclaration::new("CommonModule")
};
