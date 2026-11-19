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
        requires_child_objects: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    ..KindDeclaration::new("HTTPService")
};
