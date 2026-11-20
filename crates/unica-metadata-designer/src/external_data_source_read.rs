//! Подчинённые внешнего источника данных и его вложенных объектов.
//!
//! Таблицы и кубы источника, таблицы измерения куба лежат в своих файлах:
//! описание владельца только перечисляет их имена в `ChildObjects`. Поля
//! таблиц, измерения и ресурсы куба, функции источника описаны прямо
//! в описании владельца и читаются общим чтением элемента коллекции: имя,
//! синоним, комментарий, наблюдаемый тип и свойства.
//!
//! Повреждённый элемент не исчезает молча: он называется в `unreadable`
//! своим полем, остальные элементы сохраняются.

use crate::info_projection::{
    direct_children_with_namespace, direct_md_child, MD_CLASSES_NAMESPACE,
};
use roxmltree::Document;
use serde_json::{json, Value};
use unica_address::source_target::MetadataAddress;

/// Коллекция владельца: ключ в `collections`, тег в `ChildObjects` и
/// способ чтения.
struct Collection {
    key: &'static str,
    tag: &'static str,
    /// Элемент — ссылка на объект со своим описанием: во владельце только имя.
    reference: bool,
}

const fn inline(key: &'static str, tag: &'static str) -> Collection {
    Collection {
        key,
        tag,
        reference: false,
    }
}

const fn reference(key: &'static str, tag: &'static str) -> Collection {
    Collection {
        key,
        tag,
        reference: true,
    }
}

/// Закрытый профиль: вид владельца и его коллекции в порядке платформы.
const PROFILE: &[(&str, &[Collection])] = &[
    (
        "ExternalDataSource",
        &[
            reference("tables", "Table"),
            reference("cubes", "Cube"),
            inline("functions", "Function"),
        ],
    ),
    ("Table", &[inline("fields", "Field")]),
    (
        "Cube",
        &[
            reference("dimensionTables", "DimensionTable"),
            inline("dimensions", "Dimension"),
            inline("resources", "Resource"),
        ],
    ),
    ("DimensionTable", &[inline("fields", "Field")]),
];

/// Виды, у которых профиль читает подчинённых.
pub fn external_data_source_member_kinds() -> impl Iterator<Item = &'static str> {
    PROFILE.iter().map(|(kind, _)| *kind)
}

/// Добавляет в `payload.collections` подчинённых владельца `target` вида
/// `kind`. Вид вне профиля оставляет `payload` как есть.
pub fn enrich_external_data_source_members(
    bytes: &[u8],
    kind: &str,
    target: &MetadataAddress,
    payload: &mut Value,
) -> Result<(), String> {
    let Some((_, collections)) = PROFILE.iter().find(|(profiled, _)| *profiled == kind) else {
        return Ok(());
    };
    let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    let text = text.trim_start_matches('\u{feff}');
    let document = Document::parse(text).map_err(|error| error.to_string())?;
    let properties = crate::descriptor_comment::descriptor_properties(&document, kind)?;
    let owner = properties
        .parent_element()
        .ok_or_else(|| format!("{kind} descriptor has no owner element"))?;
    let children = direct_md_child(owner, "ChildObjects");
    let mut unreadable = Vec::new();
    for collection in *collections {
        let field = format!("collections.{}", collection.key);
        let items = if collection.reference {
            let mut items = Vec::new();
            for (index, node) in direct_children_with_namespace_opt(children, collection.tag)
                .into_iter()
                .enumerate()
            {
                let name = node.text().map(str::trim).unwrap_or_default();
                if name.is_empty() || node.children().any(|child| child.is_element()) {
                    unreadable.push(json!({
                        "field": format!("{field}[{index}]"),
                        "message": format!("{} reference has no name", collection.tag),
                    }));
                    continue;
                }
                items.push(json!({"name": name}));
            }
            Value::Array(items)
        } else {
            let mut diagnostics = Vec::new();
            let items = crate::info::typed_elements_with_diagnostics(
                text,
                children,
                collection.tag,
                false,
                &field,
                target,
                &mut diagnostics,
            );
            unreadable.extend(diagnostics.into_iter().map(|diagnostic| {
                json!({
                    "field": diagnostic.field.unwrap_or_else(|| field.clone()),
                    "message": diagnostic.message,
                })
            }));
            serde_json::to_value(items).map_err(|error| error.to_string())?
        };
        if !payload["collections"].is_object() {
            payload["collections"] = json!({});
        }
        payload["collections"][collection.key] = items;
    }
    if !unreadable.is_empty() {
        if let Some(existing) = payload["unreadable"].as_array_mut() {
            existing.extend(unreadable);
        } else {
            payload["unreadable"] = json!(unreadable);
        }
    }
    Ok(())
}

fn direct_children_with_namespace_opt<'a, 'input>(
    parent: Option<roxmltree::Node<'a, 'input>>,
    tag: &str,
) -> Vec<roxmltree::Node<'a, 'input>> {
    parent
        .map(|parent| direct_children_with_namespace(parent, MD_CLASSES_NAMESPACE, tag))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPORT: &str = "../../tests/fixtures/formats/platform_xml/2.20/all_kinds/src/ExternalDataSources/AllKindsExternalSource";

    fn read(relative: &str, kind: &str, target: &str) -> Value {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(EXPORT)
                .join(relative),
        )
        .unwrap();
        let mut payload = json!({"collections": {"forms": []}});
        enrich_external_data_source_members(
            &bytes,
            kind,
            &MetadataAddress::parse_logical(target).unwrap(),
            &mut payload,
        )
        .unwrap();
        payload
    }

    fn names(payload: &Value, key: &str) -> Vec<String> {
        payload["collections"][key]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["name"].as_str().unwrap().to_string())
            .collect()
    }

    /// Источник перечисляет таблицы и кубы по именам, а функции описывает
    /// сам: у функции читается тип возвращаемого значения.
    #[test]
    fn a_source_lists_its_tables_cubes_and_functions() {
        let payload = read(
            "../AllKindsExternalSource.xml",
            "ExternalDataSource",
            "ExternalDataSource.AllKindsExternalSource",
        );
        assert_eq!(names(&payload, "tables"), ["Items"]);
        assert_eq!(names(&payload, "cubes"), ["Sales"]);
        assert_eq!(names(&payload, "functions"), ["Total"]);
        assert!(
            payload["collections"]["functions"][0]["type"].is_object(),
            "{payload}"
        );
        assert!(payload["collections"]["forms"].is_array());
        assert!(payload.get("unreadable").is_none(), "{payload}");
    }

    #[test]
    fn a_cube_lists_its_dimension_tables_dimensions_and_resources() {
        let payload = read(
            "Cubes/Sales.xml",
            "Cube",
            "ExternalDataSource.AllKindsExternalSource.Cube.Sales",
        );
        assert_eq!(names(&payload, "dimensionTables"), ["Products"]);
        assert_eq!(names(&payload, "dimensions"), ["Product"]);
        assert_eq!(names(&payload, "resources"), ["Amount"]);
    }

    #[test]
    fn tables_and_dimension_tables_list_their_fields() {
        for (relative, kind, target) in [
            (
                "Tables/Items.xml",
                "Table",
                "ExternalDataSource.AllKindsExternalSource.Table.Items",
            ),
            (
                "Cubes/Sales/DimensionTables/Products.xml",
                "DimensionTable",
                "ExternalDataSource.AllKindsExternalSource.Cube.Sales.DimensionTable.Products",
            ),
        ] {
            let payload = read(relative, kind, target);
            assert_eq!(names(&payload, "fields"), ["Code"], "{kind}");
            assert!(
                payload["collections"]["fields"][0]["type"].is_object(),
                "{payload}"
            );
        }
    }

    /// Ссылка без имени — повреждённая запись, а не отсутствие таблицы.
    #[test]
    fn a_reference_without_a_name_is_reported_unreadable() {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(EXPORT)
                .join("../AllKindsExternalSource.xml"),
        )
        .unwrap()
        .replacen("<Table>Items</Table>", "<Table/>", 1);
        let mut payload = json!({});
        enrich_external_data_source_members(
            text.as_bytes(),
            "ExternalDataSource",
            &MetadataAddress::parse_logical("ExternalDataSource.AllKindsExternalSource").unwrap(),
            &mut payload,
        )
        .unwrap();
        assert_eq!(names(&payload, "tables"), Vec::<String>::new());
        assert_eq!(payload["unreadable"][0]["field"], "collections.tables[0]");
    }

    #[test]
    fn a_kind_outside_the_profile_is_left_as_is() {
        let mut payload = json!({"kind": "Catalog"});
        enrich_external_data_source_members(
            b"<x/>",
            "Catalog",
            &MetadataAddress::parse_logical("Catalog.Items").unwrap(),
            &mut payload,
        )
        .unwrap();
        assert_eq!(payload, json!({"kind": "Catalog"}));
    }
}
