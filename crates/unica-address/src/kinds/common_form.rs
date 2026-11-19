//! «ОбщаяФорма», `CommonForm`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ОбщаяФорма"],
    metadata: Some(MetadataDeclaration::new(&["CommonForms", "ОбщиеФормы"])),
    root: Some(RootPlace {
        child_objects: 2900,
        tree: Some(1800),
    }),
    layout: Some(LayoutDeclaration {
        directory: "CommonForms",
        display_name_ru: "Общие формы",
        child_objects_8_3_27: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[]),
    ..KindDeclaration::new("CommonForm")
};
