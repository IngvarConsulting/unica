//! «Таблица», `Table`: таблица внешнего источника данных.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Таблица"],
    metadata: Some(MetadataDeclaration::new(&["Tables", "Таблицы"])),
    nested: Some(NestedPlace {
        owners: &["ExternalDataSource"],
        directory: Some("Tables"),
        modules: &["ObjectModule", "ManagerModule", "RecordSetModule"],
    }),
    ..KindDeclaration::new("Table")
};
