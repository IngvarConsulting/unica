//! «ОбщаяКартинка», `CommonPicture`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ОбщаяКартинка"],
    metadata: Some(MetadataDeclaration::new(&[
        "CommonPictures",
        "ОбщиеКартинки",
    ])),
    root: Some(RootPlace {
        child_objects: 500,
        tree: Some(2000),
    }),
    layout: Some(LayoutDeclaration {
        directory: "CommonPictures",
        display_name_ru: "Общие картинки",
        child_objects_8_3_27: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[]),
    ..KindDeclaration::new("CommonPicture")
};
