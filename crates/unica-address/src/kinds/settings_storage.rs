//! «ХранилищеНастроек», `SettingsStorage`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ХранилищеНастроек"],
    metadata: Some(MetadataDeclaration::new(&["SettingsStorages", "ХранилищаНастроек"]).manager()),
    root: Some(RootPlace {
        child_objects: 1900,
        tree: Some(1500),
    }),
    layout: Some(LayoutDeclaration {
        directory: "SettingsStorages",
        display_name_ru: "Хранилища настроек",
        requires_child_objects: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[("SettingsStorageManager", "Manager")]),
    ..KindDeclaration::new("SettingsStorage")
};
