//! «ПакетXDTO», `XDTOPackage`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ПакетXDTO"],
    metadata: Some(MetadataDeclaration::new(&["XDTOPackages", "ПакетыXDTO"])),
    root: Some(RootPlace { child_objects: 1300, tree: Some(2100) }),
    layout: Some(LayoutDeclaration {
        directory: "XDTOPackages",
        display_name_ru: "XDTO-пакеты",
        child_objects_8_3_27: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[]),
    ..KindDeclaration::new("XDTOPackage")
};
