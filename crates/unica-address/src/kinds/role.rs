//! «Роль», `Role`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Роль"],
    metadata: Some(MetadataDeclaration::new(&["Roles", "Роли"])),
    root: Some(RootPlace {
        child_objects: 700,
        tree: Some(400),
    }),
    layout: Some(LayoutDeclaration {
        directory: "Roles",
        display_name_ru: "Роли",
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    ..KindDeclaration::new("Role")
};
