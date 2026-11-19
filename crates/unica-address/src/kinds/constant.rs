//! «Константа», `Constant`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Константа"],
    metadata: Some(MetadataDeclaration::new(&["Constants", "Константы"]).manager()),
    root: Some(RootPlace { child_objects: 2800, tree: Some(3100) }),
    layout: Some(LayoutDeclaration {
        directory: "Constants",
        display_name_ru: "Константы",
        child_objects_8_3_27: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[
        ("ConstantManager", "Manager"),
        ("ConstantValueManager", "ValueManager"),
        ("ConstantValueKey", "ValueKey"),
    ]),
    ..KindDeclaration::new("Constant")
};
