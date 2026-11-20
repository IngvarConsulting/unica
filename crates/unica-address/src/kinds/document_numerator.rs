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
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    ..KindDeclaration::new("DocumentNumerator")
};
