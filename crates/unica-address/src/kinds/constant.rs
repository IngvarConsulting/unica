//! «Константа», `Constant`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Константа"],
    metadata: Some(MetadataDeclaration::new(&["Constants", "Константы"]).manager()),
    root: Some(RootPlace {
        child_objects: 2800,
        tree: Some(3100),
    }),
    layout: Some(LayoutDeclaration {
        directory: "Constants",
        display_name_ru: "Константы",
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[
        ("ConstantManager", "Manager"),
        ("ConstantValueManager", "ValueManager"),
        ("ConstantValueKey", "ValueKey"),
    ]),
    ..KindDeclaration::new("Constant")
};
