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
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    // A native empty picture has no Ext directory or image resource.
    creation: Some(CreationDeclaration {
        main_configuration_only: true,
        ..CreationDeclaration::properties(&[
            "<AvailabilityForChoice>false</AvailabilityForChoice>",
            "<AvailabilityForAppearance>false</AvailabilityForAppearance>",
        ])
    }),
    ..KindDeclaration::new("CommonPicture")
};
