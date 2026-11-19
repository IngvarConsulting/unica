//! «НумераторДокументов», `DocumentNumerator`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["НумераторДокументов"],
    metadata: Some(MetadataDeclaration::new(&[
        "DocumentNumerators",
        "НумераторыДокументов",
    ])),
    root: Some(RootPlace {
        child_objects: 3200,
        tree: Some(3400),
    }),
    layout: Some(LayoutDeclaration {
        directory: "DocumentNumerators",
        display_name_ru: "Нумераторы",
        child_objects_8_3_27: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[]),
    ..KindDeclaration::new("DocumentNumerator")
};
