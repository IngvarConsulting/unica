//! «ВнешнийИсточникДанных», `ExternalDataSource`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ВнешнийИсточникДанных"],
    metadata: Some(
        MetadataDeclaration::new(&["ExternalDataSources", "ВнешниеИсточникиДанных"]).manager(),
    ),
    root: Some(RootPlace {
        child_objects: 4650,
        tree: Some(4900),
    }),
    // Без `ChildObjects` импорт 8.3.27 отказывает и пустому источнику:
    // «ожидаемое ChildObjects».
    layout: Some(LayoutDeclaration {
        directory: "ExternalDataSources",
        display_name_ru: "Внешние источники данных",
        requires_child_objects: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[
        ("ExternalDataSourceManager", "Manager"),
        ("ExternalDataSourceTablesManager", "TablesManager"),
        ("ExternalDataSourceCubesManager", "CubesManager"),
    ]),
    child_order: Some(&["Table", "Cube", "Function"]),
    ..KindDeclaration::new("ExternalDataSource")
};
