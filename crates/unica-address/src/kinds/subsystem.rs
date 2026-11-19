//! «Подсистема», `Subsystem`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Подсистема"],
    metadata: Some(MetadataDeclaration::new(&["Subsystems", "Подсистемы"])),
    root: Some(RootPlace { child_objects: 200, tree: Some(100) }),
    layout: Some(LayoutDeclaration {
        directory: "Subsystems",
        display_name_ru: "Подсистемы",
        child_objects_8_3_27: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[]),
    ..KindDeclaration::new("Subsystem")
};
