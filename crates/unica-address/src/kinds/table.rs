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
    generated_types: Some(&[
        ("ExternalDataSourceTableManager", "Manager"),
        ("ExternalDataSourceTableObject", "Object"),
        ("ExternalDataSourceTableRef", "Ref"),
        ("ExternalDataSourceTableList", "List"),
        ("ExternalDataSourceTableRecord", "Record"),
        ("ExternalDataSourceTableRecordSet", "RecordSet"),
        ("ExternalDataSourceTableRecordKey", "RecordKey"),
        ("ExternalDataSourceTableRecordManager", "RecordManager"),
    ]),
    child_order: Some(&["Field"]),
    ..KindDeclaration::new("Table")
};
