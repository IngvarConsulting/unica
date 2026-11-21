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
        requires_child_objects: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    ..KindDeclaration::new("WebService")
};
