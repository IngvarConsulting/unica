//! «HTTPСервис», `HTTPService`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["HTTPСервис"],
    metadata: Some(MetadataDeclaration::new(&["HTTPServices", "HTTPСервисы"])),
    root: Some(RootPlace {
        child_objects: 1500,
        tree: Some(2300),
    }),
    layout: Some(LayoutDeclaration {
        directory: "HTTPServices",
        display_name_ru: "HTTP-сервисы",
        child_objects_8_3_27: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[]),
    ..KindDeclaration::new("HTTPService")
};
