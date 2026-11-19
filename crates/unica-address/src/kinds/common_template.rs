//! «ОбщийМакет», `CommonTemplate`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ОбщийМакет"],
    metadata: Some(MetadataDeclaration::new(&["CommonTemplates", "ОбщиеМакеты"])),
    root: Some(RootPlace { child_objects: 800, tree: Some(1900) }),
    layout: Some(LayoutDeclaration {
        directory: "CommonTemplates",
        display_name_ru: "Общие макеты",
        child_objects_8_3_27: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[]),
    ..KindDeclaration::new("CommonTemplate")
};
