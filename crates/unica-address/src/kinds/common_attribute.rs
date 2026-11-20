//! «ОбщийРеквизит», `CommonAttribute`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ОбщийРеквизит"],
    metadata: Some(MetadataDeclaration::new(&[
        "CommonAttributes",
        "ОбщиеРеквизиты",
    ])),
    root: Some(RootPlace {
        child_objects: 1100,
        tree: Some(500),
    }),
    layout: Some(LayoutDeclaration {
        directory: "CommonAttributes",
        display_name_ru: "Общие реквизиты",
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    ..KindDeclaration::new("CommonAttribute")
};
