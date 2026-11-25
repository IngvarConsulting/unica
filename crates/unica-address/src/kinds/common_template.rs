//! «ОбщийМакет», `CommonTemplate`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ОбщийМакет"],
    metadata: Some(MetadataDeclaration::new(&[
        "CommonTemplates",
        "ОбщиеМакеты",
    ])),
    root: Some(RootPlace {
        child_objects: 800,
        tree: Some(1900),
    }),
    layout: Some(LayoutDeclaration {
        directory: "CommonTemplates",
        display_name_ru: "Общие макеты",
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    // An empty BinaryData template round-trips as its descriptor alone.
    creation: Some(CreationDeclaration {
        main_configuration_only: true,
        ..CreationDeclaration::properties(&["<TemplateType>BinaryData</TemplateType>"])
    }),
    ..KindDeclaration::new("CommonTemplate")
};
