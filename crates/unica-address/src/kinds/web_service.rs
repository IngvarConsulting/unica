//! «ВебСервис», `WebService`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ВебСервис"],
    metadata: Some(MetadataDeclaration::new(&["WebServices", "ВебСервисы"])),
    root: Some(RootPlace {
        child_objects: 1400,
        tree: Some(2200),
    }),
    layout: Some(LayoutDeclaration {
        directory: "WebServices",
        display_name_ru: "Веб-сервисы",
        child_objects_8_3_27: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[]),
    ..KindDeclaration::new("WebService")
};
