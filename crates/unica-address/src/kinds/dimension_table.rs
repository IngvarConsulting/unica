//! «ТаблицаИзмерения», `DimensionTable`: таблица измерения куба внешнего
//! источника данных.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ТаблицаИзмерения"],
    metadata: Some(MetadataDeclaration::new(&["DimensionTables", "ТаблицыИзмерений"])),
    nested: Some(NestedPlace {
        owners: &["Cube"],
        directory: Some("DimensionTables"),
        modules: &["ObjectModule", "ManagerModule"],
    }),
    ..KindDeclaration::new("DimensionTable")
};
