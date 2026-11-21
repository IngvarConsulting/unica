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
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    ..KindDeclaration::new("CommonForm")
};
