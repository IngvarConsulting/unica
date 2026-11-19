//! «ОбщийРеквизит», `CommonAttribute`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ОбщийРеквизит"],
    metadata: Some(MetadataDeclaration::new(&["CommonAttributes", "ОбщиеРеквизиты"])),
    root: Some(RootPlace { child_objects: 1100, tree: Some(500) }),
    layout: Some(LayoutDeclaration {
        directory: "CommonAttributes",
        display_name_ru: "Общие реквизиты",
        child_objects_8_3_27: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[]),
    ..KindDeclaration::new("CommonAttribute")
};
