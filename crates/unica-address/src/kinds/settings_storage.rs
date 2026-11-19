//! «ХранилищеНастроек», `SettingsStorage`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ХранилищеНастроек"],
    metadata: Some(MetadataDeclaration::new(&["SettingsStorages", "ХранилищаНастроек"]).manager()),
    root: Some(RootPlace { child_objects: 1900, tree: Some(1500) }),
    layout: Some(LayoutDeclaration {
        directory: "SettingsStorages",
        display_name_ru: "Хранилища настроек",
        child_objects_8_3_27: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[
        ("SettingsStorageManager", "Manager"),
    ]),
    ..KindDeclaration::new("SettingsStorage")
};
