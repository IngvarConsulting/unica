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

    /// Описание вида `kind` с подчинёнными `children` в `ChildObjects`, в
    /// форме выгрузки платформы 2.20.
    fn descriptor(kind: &str, children: &str) -> String {
        format!(
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\"              xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"              xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" version=\"2.20\">             <{kind} uuid=\"be05a1ec-2f25-54df-816a-7baed4264299\">             <Properties><Name>Probe</Name><Synonym/><Comment/></Properties>             <ChildObjects>{children}</ChildObjects></{kind}></MetaDataObject>"
        )
    }

    /// Член, описанный прямо в `ChildObjects`: имя и строковый тип.
    fn member(tag: &str, name: &str) -> String {
        format!(
            "<{tag} uuid=\"9ff1e2c8-e0b1-51c5-b64d-68b6b4c8f866\"><Properties>             <Name>{name}</Name><Synonym/><Comment/>             <Type><v8:Type>xs:string</v8:Type><v8:StringQualifiers>             <v8:Length>10</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength>             </v8:StringQualifiers></Type></Properties></{tag}>"
        )
    }

    fn read(kind: &str, children: &str, target: &str) -> Value {
        let mut payload = json!({"collections": {"forms": []}});
        enrich_external_data_source_members(
            descriptor(kind, children).as_bytes(),
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
            "ExternalDataSource",
            &format!(
                "<Table>Items</Table><Cube>Sales</Cube>{}",
                member("Function", "Total")
            ),
            "ExternalDataSource.Probe",
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
            "Cube",
            &format!(
                "<DimensionTable>Products</DimensionTable>{}{}",
                member("Dimension", "Product"),
                member("Resource", "Amount")
            ),
            "ExternalDataSource.Shop.Cube.Probe",
        );
        assert_eq!(names(&payload, "dimensionTables"), ["Products"]);
        assert_eq!(names(&payload, "dimensions"), ["Product"]);
        assert_eq!(names(&payload, "resources"), ["Amount"]);
    }

    #[test]
    fn tables_and_dimension_tables_list_their_fields() {
        for (kind, target) in [
            ("Table", "ExternalDataSource.Shop.Table.Probe"),
            (
                "DimensionTable",
                "ExternalDataSource.Shop.Cube.Sales.DimensionTable.Probe",
            ),
        ] {
            let payload = read(kind, &member("Field", "Code"), target);
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
        let text = descriptor("ExternalDataSource", "<Table/>");
        let mut payload = json!({});
        enrich_external_data_source_members(
            text.as_bytes(),
            "ExternalDataSource",
            &MetadataAddress::parse_logical("ExternalDataSource.Probe").unwrap(),
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
