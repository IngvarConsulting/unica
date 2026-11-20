//! «Куб», `Cube`: куб внешнего источника данных.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Куб"],
    metadata: Some(MetadataDeclaration::new(&["Cubes", "Кубы"])),
    nested: Some(NestedPlace {
        owners: &["ExternalDataSource"],
        directory: Some("Cubes"),
        modules: &["ManagerModule", "RecordSetModule"],
    }),
    ..KindDeclaration::new("Cube")
};
