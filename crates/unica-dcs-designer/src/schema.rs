//! Existing Designer DCS XML data model, reads and semantic validation.
use crate::xml::*;
use roxmltree::Document;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use unica_metadata::support_state::ObjectSupportData as DomainObjectSupportData;

pub fn require_dcs_root(root: roxmltree::Node<'_, '_>) -> Result<(), String> {
    let local_name = root.tag_name().name();
    if local_name != "DataCompositionSchema" {
        return Err(format!(
            "Root element is '{local_name}', expected 'DataCompositionSchema'"
        ));
    }
    let namespace = root.tag_name().namespace().unwrap_or("");
    if namespace != DCS_SCHEMA_NS {
        return Err(format!(
            "Root namespace is '{namespace}' for DataCompositionSchema, expected '{DCS_SCHEMA_NS}'"
        ));
    }
    Ok(())
}

/// Вид элемента структуры варианта; `None` — узел не элемент структуры.
fn dcs_structure_kind<'a>(item: roxmltree::Node<'a, '_>) -> Option<&'a str> {
    if !(unica_xml_format::names::element_has_name(item, "item", Some(DCS_SETTINGS_NS))
        || dcs_info_axis_structure(item))
    {
        return None;
    }
    let known_kind = [
        ("StructureItemGroup", "Group"),
        ("StructureItemTable", "Table"),
        ("StructureItemChart", "Chart"),
        ("StructureItemNestedObject", "NestedObject"),
    ]
    .into_iter()
    .find_map(|(xml_type, kind)| xsi_type_matches(item, DCS_SETTINGS_NS, xml_type).then_some(kind));
    let xml_type = item
        .attribute((XML_SCHEMA_INSTANCE_NS, "type"))
        .unwrap_or_default();
    let local_type = xml_type.rsplit(':').next().unwrap_or_default();
    known_kind
        .or_else(|| dcs_info_axis_structure(item).then_some("Group"))
        .or_else(|| {
            (local_type.starts_with("StructureItem")
                && xsi_type_matches(item, DCS_SETTINGS_NS, local_type))
            .then_some(local_type)
        })
}

/// Элементы структуры варианта в порядке строк чтения: `i`-й узел — строка
/// `index = i` из `structureItems`. По нему правка находит элемент, который
/// адресует чтение (#1357).
pub fn dcs_structure_nodes<'a, 'i>(
    settings: roxmltree::Node<'a, 'i>,
) -> Vec<roxmltree::Node<'a, 'i>> {
    settings
        .descendants()
        .filter(|node| dcs_structure_kind(*node).is_some())
        .collect()
}

/// Строки структуры варианта, как их публикует чтение.
pub fn dcs_structure_rows(settings: roxmltree::Node<'_, '_>) -> Vec<serde_json::Value> {
    dcs_info_structure_rows(settings)
        .into_iter()
        .filter_map(|row| serde_json::to_value(row).ok())
        .collect()
}

fn dcs_info_structure_rows(settings: roxmltree::Node<'_, '_>) -> Vec<DcsInfoStructureRow> {
    let mut rows = Vec::new();
    let mut indices = HashMap::new();
    for item in dcs_structure_nodes(settings) {
        let kind = dcs_structure_kind(item).expect("a structure node has a kind");
        let mut parent_index = None;
        let mut axis = ["row", "column", "point", "series"]
            .iter()
            .find(|tag| unica_xml_format::names::element_has_name(item, tag, Some(DCS_SETTINGS_NS)))
            .map(|tag| tag.to_string());
        for ancestor in item
            .ancestors()
            .skip(1)
            .take_while(|node| *node != settings)
        {
            if let Some(index) = indices.get(&ancestor.id()) {
                parent_index = Some(*index);
                break;
            }
            if axis.is_none()
                && ancestor.tag_name().namespace() == Some(DCS_SETTINGS_NS)
                && matches!(
                    ancestor.tag_name().name(),
                    "row" | "column" | "point" | "series"
                )
            {
                axis = Some(ancestor.tag_name().name().to_string());
            }
        }
        let index = rows.len();
        indices.insert(item.id(), index);
        rows.push(DcsInfoStructureRow {
            index,
            parent_index,
            axis,
            kind: kind.to_string(),
            name: dcs_info_child_text(item, "name", DCS_SETTINGS_NS),
            group_by: dcs_info_group_fields(item, DCS_SETTINGS_NS),
            properties: dcs_info_structure_properties(item),
        });
    }
    rows
}

fn dcs_info_structure_properties(
    node: roxmltree::Node<'_, '_>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut props = serde_json::Map::new();
    for key in [
        "use",
        "viewMode",
        "itemsViewMode",
        "rowsViewMode",
        "columnsViewMode",
        "pointsViewMode",
        "seriesViewMode",
        "userSettingID",
        "userSettingPresentation",
    ] {
        if let Some(child) =
            unica_xml_format::read::child_element_in_namespace(node, key, DCS_SETTINGS_NS)
        {
            let value = if key == "userSettingPresentation" {
                serde_json::json!(dcs_info_multilang_or_inner_text(child))
            } else if key == "use" {
                serde_json::json!(dcs_text_of(child) == "true")
            } else {
                serde_json::json!(dcs_text_of(child))
            };
            props.insert(key.into(), value);
        }
    }
    props
}

fn dcs_info_selection_rows<'a, 'i>(
    container: roxmltree::Node<'a, 'i>,
    parents: &[usize],
    rows: &mut Vec<(usize, Vec<usize>, roxmltree::Node<'a, 'i>)>,
) {
    for (index, item) in
        unica_xml_format::read::child_elements_in_namespace(container, "item", DCS_SETTINGS_NS)
            .into_iter()
            .enumerate()
    {
        rows.push((index, parents.to_vec(), item));
        if xsi_type_matches(item, DCS_SETTINGS_NS, "SelectedItemFolder") {
            let mut next = parents.to_vec();
            next.push(index);
            dcs_info_selection_rows(item, &next, rows);
        }
    }
}

fn dcs_info_axis_structure(node: roxmltree::Node<'_, '_>) -> bool {
    ["row", "column", "point", "series"]
        .iter()
        .any(|tag| unica_xml_format::names::element_has_name(node, tag, Some(DCS_SETTINGS_NS)))
        && (!node.children().any(|child| {
            unica_xml_format::names::element_has_name(child, "item", Some(DCS_SETTINGS_NS))
        }) || [
            "name",
            "groupItems",
            "filter",
            "order",
            "selection",
            "conditionalAppearance",
            "outputParameters",
        ]
        .iter()
        .any(|tag| {
            unica_xml_format::read::child_element_in_namespace(node, tag, DCS_SETTINGS_NS).is_some()
        }))
}

fn dcs_info_setting_rows(settings: roxmltree::Node<'_, '_>) -> Vec<serde_json::Value> {
    let mut rows = Vec::new();
    for scope in settings.descendants().filter(|node| {
        *node == settings
            || (unica_xml_format::names::element_has_name(*node, "item", Some(DCS_SETTINGS_NS))
                && dcs_info_structure_item_type(*node) != "Unknown")
            || ["row", "column", "point", "series"].iter().any(|tag| {
                unica_xml_format::names::element_has_name(*node, tag, Some(DCS_SETTINGS_NS))
            })
    }) {
        for (tag, kind, ns) in [
            ("selection", "Selection", DCS_SETTINGS_NS),
            ("filter", "Filter", DCS_SETTINGS_NS),
            ("order", "Order", DCS_SETTINGS_NS),
            ("dataParameters", "DataParameter", DCS_CORE_NS),
            ("outputParameters", "OutputParameter", DCS_CORE_NS),
            (
                "conditionalAppearance",
                "ConditionalAppearance",
                DCS_SETTINGS_NS,
            ),
        ] {
            if let Some(container) =
                unica_xml_format::read::child_element_in_namespace(scope, tag, DCS_SETTINGS_NS)
            {
                let mut items = Vec::new();
                if kind == "Selection" {
                    dcs_info_selection_rows(container, &[], &mut items);
                } else {
                    items.extend(
                        unica_xml_format::read::child_elements_in_namespace(container, "item", ns)
                            .into_iter()
                            .enumerate()
                            .map(|(index, item)| (index, Vec::new(), item)),
                    );
                }
                for (index, parents, item) in items {
                    let mut row = serde_json::Map::new();
                    if !parents.is_empty() {
                        row.insert("parentIndexes".into(), serde_json::json!(parents));
                    }
                    if let Some(title) = unica_xml_format::read::child_element_in_namespace(
                        item,
                        "lwsTitle",
                        DCS_SETTINGS_NS,
                    ) {
                        row.insert(
                            "title".into(),
                            serde_json::json!(dcs_info_multilang_or_inner_text(title)),
                        );
                    }
                    if let Some(kind) = item.attribute((XML_SCHEMA_INSTANCE_NS, "type")) {
                        row.insert(
                            "itemType".into(),
                            serde_json::json!(kind.rsplit(':').next().unwrap_or(kind)),
                        );
                    }
                    row.insert("kind".into(), serde_json::json!(kind));
                    row.insert("index".into(), serde_json::json!(index));
                    if scope != settings {
                        if let Some(name) = dcs_info_child_text(scope, "name", DCS_SETTINGS_NS) {
                            row.insert("group".into(), serde_json::json!(name));
                        }
                    }
                    for (input, output) in [
                        ("field", "field"),
                        ("left", "field"),
                        ("parameter", "name"),
                        ("orderType", "direction"),
                        ("comparisonType", "comparison"),
                        ("right", "value"),
                        ("value", "value"),
                        ("use", "use"),
                        ("viewMode", "viewMode"),
                        ("userSettingID", "userSettingID"),
                        ("userSettingPresentation", "userSettingPresentation"),
                    ] {
                        let child =
                            unica_xml_format::read::child_element_in_namespace(item, input, ns)
                                .or_else(|| {
                                    unica_xml_format::read::child_element_in_namespace(
                                        item,
                                        input,
                                        DCS_SETTINGS_NS,
                                    )
                                });
                        if let Some(child) = child {
                            row.insert(output.into(), serde_json::json!(dcs_text_of(child)));
                        }
                    }
                    if kind == "ConditionalAppearance" {
                        if let Some(fields) = unica_xml_format::read::child_element_in_namespace(
                            item,
                            "selection",
                            DCS_SETTINGS_NS,
                        ) {
                            row.insert(
                                "fields".into(),
                                serde_json::json!(
                                    unica_xml_format::read::child_elements_in_namespace(
                                        fields,
                                        "item",
                                        DCS_SETTINGS_NS
                                    )
                                    .into_iter()
                                    .filter_map(|item| dcs_info_child_text(
                                        item,
                                        "field",
                                        DCS_SETTINGS_NS
                                    ))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                                ),
                            );
                        }
                    }
                    rows.push(serde_json::Value::Object(row));
                }
            }
        }
    }
    rows
}

fn dcs_info_opt(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn dcs_info_child_text(
    node: roxmltree::Node<'_, '_>,
    tag: &str,
    ns_schema: &str,
) -> Option<String> {
    unica_xml_format::read::child_element_in_namespace(node, tag, ns_schema)
        .map(dcs_text_of)
        .and_then(dcs_info_opt)
}

/// A declared boolean, or `null` when the schema declares nothing. ADR-0023
/// keeps the two apart: an omitted flag is not a proven `false`.
fn dcs_info_flag(node: roxmltree::Node<'_, '_>, tag: &str, ns_schema: &str) -> Option<bool> {
    unica_xml_format::read::child_element_in_namespace(node, tag, ns_schema)
        .map(dcs_text_of)
        .and_then(|value| match value.trim() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        })
}

/// The uses a structured `useRestriction` bars. Each child set to `true` names
/// one barred use; `None` when the element is absent, so an unrestricted field
/// and a field that says nothing stay distinguishable.
fn dcs_info_restrictions(
    node: roxmltree::Node<'_, '_>,
    tag: &str,
    ns_schema: &str,
) -> Option<Vec<String>> {
    unica_xml_format::read::child_element_in_namespace(node, tag, ns_schema).map(|restriction| {
        restriction
            .children()
            .filter(|child| child.is_element())
            .filter(|child| dcs_text_of(*child).trim() == "true")
            .map(|child| child.tag_name().name().to_string())
            .collect()
    })
}

/// Order items of one variant: the field and, when declared, its direction.
fn dcs_info_order_items(
    settings_node: roxmltree::Node<'_, '_>,
    ns_settings: &str,
) -> Vec<DcsInfoOrderItem> {
    dcs_find_all_path(
        settings_node,
        &[("order", ns_settings), ("item", ns_settings)],
    )
    .into_iter()
    .filter_map(|item| {
        Some(DcsInfoOrderItem {
            field: unica_xml_format::read::child_element_in_namespace(item, "field", ns_settings)
                .map(dcs_text_of)
                .and_then(dcs_info_opt)?,
            direction: unica_xml_format::read::child_element_in_namespace(
                item,
                "orderType",
                ns_settings,
            )
            .map(dcs_text_of)
            .and_then(dcs_info_opt),
        })
    })
    .collect()
}

fn dcs_info_data_set(data_set: roxmltree::Node<'_, '_>, ns_schema: &str) -> DcsInfoDataSet {
    let kind = dcs_info_dataset_type(data_set);
    DcsInfoDataSet {
        name: unica_xml_format::read::child_element_in_namespace(data_set, "name", ns_schema)
            .map(dcs_text_of)
            .unwrap_or_default(),
        object_name: dcs_info_child_text(data_set, "objectName", ns_schema),
        data_source: dcs_info_child_text(data_set, "dataSource", ns_schema),
        // `dcs_inner_text`, not `dcs_all_text`: a 1C query carries meaningful
        // leading whitespace in its `|` continuation lines, and the retired
        // `Raw` mode existed precisely to hand it back untrimmed.
        query: unica_xml_format::read::child_element_in_namespace(data_set, "query", ns_schema)
            .map(dcs_inner_text)
            .and_then(dcs_info_opt),
        fields: unica_xml_format::read::child_elements_in_namespace(data_set, "field", ns_schema)
            .into_iter()
            .filter_map(|field| {
                Some(DcsInfoField {
                    data_path: dcs_info_child_text(field, "dataPath", ns_schema)?,
                    field: dcs_info_child_text(field, "field", ns_schema),
                    title: unica_xml_format::read::child_element_in_namespace(
                        field, "title", ns_schema,
                    )
                    .map(dcs_info_multilang_or_inner_text)
                    .and_then(dcs_info_opt),
                })
            })
            .collect(),
        items: if kind == "Union" {
            unica_xml_format::read::child_elements_in_namespace(data_set, "item", ns_schema)
                .into_iter()
                .map(|item| dcs_info_data_set(item, ns_schema))
                .collect()
        } else {
            Vec::new()
        },
        kind,
    }
}

fn dcs_info_collect(
    root: roxmltree::Node<'_, '_>,
    ns_schema: &str,
    ns_settings: &str,
    support: DomainObjectSupportData,
) -> DcsInfoData {
    DcsInfoData {
        support,
        data_sources: unica_xml_format::read::child_elements_in_namespace(
            root,
            "dataSource",
            ns_schema,
        )
        .into_iter()
        .map(|source| DcsInfoDataSource {
            name: unica_xml_format::read::child_element_in_namespace(source, "name", ns_schema)
                .map(dcs_text_of)
                .unwrap_or_default(),
            kind: dcs_info_child_text(source, "dataSourceType", ns_schema),
        })
        .collect(),
        data_sets: unica_xml_format::read::child_elements_in_namespace(root, "dataSet", ns_schema)
            .into_iter()
            .map(|data_set| dcs_info_data_set(data_set, ns_schema))
            .collect(),
        links: unica_xml_format::read::child_elements_in_namespace(root, "dataSetLink", ns_schema)
            .into_iter()
            .map(|link| {
                let literal = |tag| {
                    unica_xml_format::read::child_element_in_namespace(link, tag, ns_schema)
                        .map(dcs_inner_text)
                };
                let boolean = |tag| {
                    literal(tag).and_then(|value| match value.trim() {
                        "true" | "1" => Some(true),
                        "false" | "0" => Some(false),
                        _ => None,
                    })
                };
                DcsInfoLink {
                    source: literal("sourceDataSet").unwrap_or_default(),
                    destination: literal("destinationDataSet").unwrap_or_default(),
                    parameter: literal("parameter"),
                    condition: literal("linkConditionExpression"),
                    start_expression: literal("startExpression"),
                    source_expression: literal("sourceExpression"),
                    destination_expression: literal("destinationExpression"),
                    parameter_list_allowed: boolean("parameterListAllowed"),
                    required: boolean("required"),
                }
            })
            .collect(),
        calculated_fields: unica_xml_format::read::child_elements_in_namespace(
            root,
            "calculatedField",
            ns_schema,
        )
        .into_iter()
        .filter_map(|field| {
            Some(DcsInfoCalculatedField {
                data_path: dcs_info_child_text(field, "dataPath", ns_schema)?,
                expression: unica_xml_format::read::child_element_in_namespace(
                    field,
                    "expression",
                    ns_schema,
                )
                .map(dcs_all_text)
                .and_then(dcs_info_opt),
                title: unica_xml_format::read::child_element_in_namespace(
                    field, "title", ns_schema,
                )
                .map(dcs_info_multilang_or_inner_text)
                .and_then(dcs_info_opt),
                restrictions: dcs_info_restrictions(field, "useRestriction", ns_schema),
            })
        })
        .collect(),
        total_fields: unica_xml_format::read::child_elements_in_namespace(
            root,
            "totalField",
            ns_schema,
        )
        .into_iter()
        .filter_map(|total| {
            Some(DcsInfoTotalField {
                data_path: dcs_info_child_text(total, "dataPath", ns_schema)?,
                expression: unica_xml_format::read::child_element_in_namespace(
                    total,
                    "expression",
                    ns_schema,
                )
                .map(dcs_all_text)
                .and_then(dcs_info_opt),
                group: dcs_info_child_text(total, "group", ns_schema),
            })
        })
        .collect(),
        parameters: unica_xml_format::read::child_elements_in_namespace(
            root,
            "parameter",
            ns_schema,
        )
        .into_iter()
        .map(|param| DcsInfoParameter {
            name: unica_xml_format::read::child_element_in_namespace(param, "name", ns_schema)
                .map(dcs_text_of)
                .unwrap_or_default(),
            type_name: unica_xml_format::read::child_element_in_namespace(
                param,
                "valueType",
                ns_schema,
            )
            .map(dcs_info_compact_type)
            .and_then(dcs_info_opt),
            value: unica_xml_format::read::child_element_in_namespace(param, "value", ns_schema)
                .map(dcs_info_param_default)
                .and_then(dcs_info_opt),
            expression: unica_xml_format::read::child_element_in_namespace(
                param,
                "expression",
                ns_schema,
            )
            .map(dcs_all_text)
            .and_then(dcs_info_opt),
            restricted: unica_xml_format::read::child_element_in_namespace(
                param,
                "useRestriction",
                ns_schema,
            )
            .map(dcs_text_of)
            .as_deref()
                == Some("true"),
            available_as_field: dcs_info_flag(param, "availableAsField", ns_schema),
        })
        .collect(),
        // Every element below `settingsVariant` belongs to the settings
        // namespace, including the variant's own `name`, `presentation` and
        // `settings`.
        variants: unica_xml_format::read::child_elements_in_namespace(
            root,
            "settingsVariant",
            ns_schema,
        )
        .into_iter()
        .map(|variant| {
            let settings = unica_xml_format::read::child_element_in_namespace(
                variant,
                "settings",
                ns_settings,
            );
            DcsInfoVariant {
                name: unica_xml_format::read::child_element_in_namespace(
                    variant,
                    "name",
                    ns_settings,
                )
                .map(dcs_text_of)
                .unwrap_or_default(),
                presentation: unica_xml_format::read::child_element_in_namespace(
                    variant,
                    "presentation",
                    ns_settings,
                )
                .map(dcs_info_multilang_or_inner_text)
                .and_then(dcs_info_opt),
                // `dcs_info_selection_fields` descends into `selection`
                // itself, so it takes the settings node, not the selection.
                selection: settings
                    .map(|node| dcs_info_selection_fields(node, ns_settings))
                    .unwrap_or_default(),
                order: settings
                    .map(|node| dcs_info_order_items(node, ns_settings))
                    .unwrap_or_default(),
                filters: settings
                    .and_then(|node| {
                        unica_xml_format::read::child_element_in_namespace(
                            node,
                            "filter",
                            ns_settings,
                        )
                    })
                    .map(|node| {
                        unica_xml_format::read::child_elements_in_namespace(
                            node,
                            "item",
                            ns_settings,
                        )
                        .len()
                    })
                    .unwrap_or_default(),
                structure_items: settings.map(dcs_info_structure_rows).unwrap_or_default(),
                setting_items: settings.map(dcs_info_setting_rows).unwrap_or_default(),
                structure: settings
                    .map(|node| {
                        unica_xml_format::read::child_elements_in_namespace(
                            node,
                            "item",
                            ns_settings,
                        )
                        .into_iter()
                        .map(|item| DcsInfoStructureItem {
                            kind: dcs_info_structure_item_type(item).to_string(),
                            group_by: dcs_info_group_fields(item, ns_settings),
                        })
                        .collect()
                    })
                    .unwrap_or_default(),
            }
        })
        .collect(),
        templates: unica_xml_format::read::child_elements_in_namespace(root, "template", ns_schema)
            .into_iter()
            .filter_map(|template| dcs_info_child_text(template, "name", ns_schema))
            .collect(),
    }
}

pub fn parse_dcs_info_xml(
    text: &str,
    support: DomainObjectSupportData,
) -> Result<DcsInfoData, String> {
    const NS_SCHEMA: &str = DCS_SCHEMA_NS;
    const NS_SETTINGS: &str = "http://v8.1c.ru/8.1/data-composition-system/settings";
    let doc = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("DCS XML parse error: {err}"))?;
    let root = doc.root_element();
    require_dcs_root(root)?;
    Ok(dcs_info_collect(root, NS_SCHEMA, NS_SETTINGS, support))
}

pub fn dcs_info_overview(
    root: roxmltree::Node<'_, '_>,
    resolved_path: &Path,
    text: &str,
    lines: &mut Vec<String>,
    ns_schema: &str,
    ns_settings: &str,
) {
    let template_name = dcs_info_template_name(resolved_path);
    let total_xml_lines = text.lines().count();
    lines.push(format!(
        "=== DCS: {template_name} ({total_xml_lines} lines) ==="
    ));
    lines.push(String::new());

    let sources =
        unica_xml_format::read::child_elements_in_namespace(root, "dataSource", ns_schema)
            .into_iter()
            .map(|source| {
                format!(
                    "{} ({})",
                    unica_xml_format::read::child_element_in_namespace(source, "name", ns_schema)
                        .map(dcs_text_of)
                        .unwrap_or_default(),
                    unica_xml_format::read::child_element_in_namespace(
                        source,
                        "dataSourceType",
                        ns_schema
                    )
                    .map(dcs_text_of)
                    .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>();
    lines.push(format!("Sources: {}", sources.join(", ")));
    lines.push(String::new());

    lines.push("Datasets:".to_string());
    for data_set in unica_xml_format::read::child_elements_in_namespace(root, "dataSet", ns_schema)
    {
        dcs_info_dataset_overview(data_set, lines, ns_schema, "  ");
    }

    let links = unica_xml_format::read::child_elements_in_namespace(root, "dataSetLink", ns_schema);
    if !links.is_empty() {
        let mut link_pairs = BTreeMap::<String, usize>::new();
        let mut ordered = Vec::<String>::new();
        for link in links {
            let key = format!(
                "{} -> {}",
                unica_xml_format::read::child_element_in_namespace(
                    link,
                    "sourceDataSet",
                    ns_schema
                )
                .map(dcs_text_of)
                .unwrap_or_default(),
                unica_xml_format::read::child_element_in_namespace(
                    link,
                    "destinationDataSet",
                    ns_schema
                )
                .map(dcs_text_of)
                .unwrap_or_default()
            );
            if !link_pairs.contains_key(&key) {
                ordered.push(key.clone());
            }
            *link_pairs.entry(key).or_insert(0) += 1;
        }
        let link_strs = ordered
            .into_iter()
            .map(|key| {
                let count = link_pairs.get(&key).copied().unwrap_or(0);
                if count > 1 {
                    format!("{key} ({count} fields)")
                } else {
                    key
                }
            })
            .collect::<Vec<_>>();
        lines.push(format!("Links: {}", link_strs.join(", ")));
    }

    let calculated =
        unica_xml_format::read::child_elements_in_namespace(root, "calculatedField", ns_schema);
    if !calculated.is_empty() {
        lines.push(format!("Calculated: {}", calculated.len()));
    }

    let totals = unica_xml_format::read::child_elements_in_namespace(root, "totalField", ns_schema);
    if !totals.is_empty() {
        let mut unique = HashSet::<String>::new();
        let mut has_grouped = false;
        for total in &totals {
            unique.insert(
                unica_xml_format::read::child_element_in_namespace(*total, "dataPath", ns_schema)
                    .map(dcs_text_of)
                    .unwrap_or_default(),
            );
            if unica_xml_format::read::child_element_in_namespace(*total, "group", ns_schema)
                .is_some()
            {
                has_grouped = true;
            }
        }
        let group_note = if has_grouped {
            ", with group formulas"
        } else {
            ""
        };
        if unique.len() == totals.len() {
            lines.push(format!("Resources: {}{group_note}", totals.len()));
        } else {
            lines.push(format!(
                "Resources: {} ({} fields{group_note})",
                totals.len(),
                unique.len()
            ));
        }
    }

    let templates =
        unica_xml_format::read::child_elements_in_namespace(root, "template", ns_schema);
    if !templates.is_empty() {
        let field_templates =
            unica_xml_format::read::child_elements_in_namespace(root, "fieldTemplate", ns_schema);
        let group_count =
            unica_xml_format::read::child_elements_in_namespace(root, "groupTemplate", ns_schema)
                .len()
                + unica_xml_format::read::child_elements_in_namespace(
                    root,
                    "groupHeaderTemplate",
                    ns_schema,
                )
                .len()
                + unica_xml_format::read::child_elements_in_namespace(
                    root,
                    "groupFooterTemplate",
                    ns_schema,
                )
                .len();
        let mut parts = Vec::new();
        if !field_templates.is_empty() {
            parts.push(format!("{} field", field_templates.len()));
        }
        if group_count > 0 {
            parts.push(format!("{group_count} group"));
        }
        if parts.is_empty() {
            lines.push(format!("Templates: {} defined", templates.len()));
        } else {
            lines.push(format!(
                "Templates: {} defined ({} bindings)",
                templates.len(),
                parts.join(", ")
            ));
        }
    }

    let params = unica_xml_format::read::child_elements_in_namespace(root, "parameter", ns_schema);
    if params.is_empty() {
        lines.push("Params: (none)".to_string());
    } else {
        let mut visible_names = Vec::new();
        let mut hidden_count = 0usize;
        for param in &params {
            let name =
                unica_xml_format::read::child_element_in_namespace(*param, "name", ns_schema)
                    .map(dcs_text_of)
                    .unwrap_or_default();
            let hidden = unica_xml_format::read::child_element_in_namespace(
                *param,
                "useRestriction",
                ns_schema,
            )
            .map(dcs_text_of)
            .is_some_and(|value| value == "true");
            if hidden {
                hidden_count += 1;
            } else {
                visible_names.push(name);
            }
        }
        let mut line = format!("Params: {}", params.len());
        if hidden_count > 0 && !visible_names.is_empty() {
            line.push_str(&format!(
                " ({} visible, {hidden_count} hidden)",
                visible_names.len()
            ));
        } else if hidden_count == params.len() {
            line.push_str(" (all hidden)");
        }
        if !visible_names.is_empty() && visible_names.len() <= 8 {
            line.push_str(": ");
            line.push_str(&visible_names.join(", "));
        }
        lines.push(line);
    }

    lines.push(String::new());
    let variants =
        unica_xml_format::read::child_elements_in_namespace(root, "settingsVariant", ns_schema);
    if !variants.is_empty() {
        lines.push("Variants:".to_string());
        for (index, variant) in variants.iter().enumerate() {
            let name =
                unica_xml_format::read::child_element_in_namespace(*variant, "name", ns_settings)
                    .map(dcs_text_of)
                    .unwrap_or_default();
            let presentation = unica_xml_format::read::child_element_in_namespace(
                *variant,
                "presentation",
                ns_settings,
            )
            .map(dcs_info_multilang_or_inner_text)
            .unwrap_or_default();
            let presentation_str = if presentation.is_empty() {
                String::new()
            } else {
                format!("  \"{presentation}\"")
            };
            let settings = unica_xml_format::read::child_element_in_namespace(
                *variant,
                "settings",
                ns_settings,
            );
            let mut struct_items = Vec::new();
            let mut filter_count = 0usize;
            if let Some(settings) = settings {
                for item in unica_xml_format::read::child_elements_in_namespace(
                    settings,
                    "item",
                    ns_settings,
                ) {
                    let item_type = dcs_info_structure_item_type(item);
                    let group_fields = dcs_info_group_fields(item, ns_settings);
                    let group = if group_fields.is_empty() {
                        "(detail)".to_string()
                    } else {
                        format!("({})", group_fields.join(","))
                    };
                    struct_items.push(format!("{item_type}{group}"));
                }
                if let Some(filter) = unica_xml_format::read::child_element_in_namespace(
                    settings,
                    "filter",
                    ns_settings,
                ) {
                    filter_count = unica_xml_format::read::child_elements_in_namespace(
                        filter,
                        "item",
                        ns_settings,
                    )
                    .len();
                }
            }
            let struct_str = if struct_items.is_empty() {
                String::new()
            } else {
                format!("  {}", struct_items.join(", "))
            };
            let filter_str = if filter_count > 0 {
                format!("  {filter_count} filters")
            } else {
                String::new()
            };
            lines.push(format!(
                "  [{}] {name}{presentation_str}{struct_str}{filter_str}",
                index + 1
            ));
        }
    }
}

pub fn dcs_info_dataset_overview(
    data_set: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    indent: &str,
) {
    let ds_type = dcs_info_dataset_type(data_set);
    let name = unica_xml_format::read::child_element_in_namespace(data_set, "name", ns_schema)
        .map(dcs_text_of)
        .unwrap_or_default();
    let field_count =
        unica_xml_format::read::child_elements_in_namespace(data_set, "field", ns_schema).len();
    match ds_type.as_str() {
        "Query" => {
            let query_lines =
                unica_xml_format::read::child_element_in_namespace(data_set, "query", ns_schema)
                    .map(|node| dcs_inner_text(node).split('\n').count())
                    .unwrap_or(0);
            lines.push(format!(
                "{indent}[Query]  {name}   {field_count} fields, query {query_lines} lines"
            ));
        }
        "Object" => {
            let obj_str = unica_xml_format::read::child_element_in_namespace(
                data_set,
                "objectName",
                ns_schema,
            )
            .map(dcs_text_of)
            .filter(|value| !value.is_empty())
            .map(|value| format!("  objectName={value}"))
            .unwrap_or_default();
            lines.push(format!(
                "{indent}[Object] {name}{obj_str}  {field_count} fields"
            ));
        }
        "Union" => {
            lines.push(format!("{indent}[Union]  {name}  {field_count} fields"));
            for sub_ds in
                unica_xml_format::read::child_elements_in_namespace(data_set, "item", ns_schema)
            {
                let sub_type = dcs_info_dataset_type(sub_ds);
                let sub_name =
                    unica_xml_format::read::child_element_in_namespace(sub_ds, "name", ns_schema)
                        .map(dcs_text_of)
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(|| "?".to_string());
                let sub_fields =
                    unica_xml_format::read::child_elements_in_namespace(sub_ds, "field", ns_schema)
                        .len();
                if sub_type == "Query" {
                    let query_lines = unica_xml_format::read::child_element_in_namespace(
                        sub_ds, "query", ns_schema,
                    )
                    .map(|node| dcs_inner_text(node).split('\n').count())
                    .unwrap_or(0);
                    lines.push(format!(
                        "    ├─ [Query] {sub_name}   {sub_fields} fields, query {query_lines} lines"
                    ));
                } else if sub_type == "Object" {
                    let obj_str = unica_xml_format::read::child_element_in_namespace(
                        sub_ds,
                        "objectName",
                        ns_schema,
                    )
                    .map(dcs_text_of)
                    .filter(|value| !value.is_empty())
                    .map(|value| format!("  objectName={value}"))
                    .unwrap_or_default();
                    lines.push(format!(
                        "    ├─ [Object] {sub_name}{obj_str}  {sub_fields} fields"
                    ));
                } else {
                    lines.push(format!(
                        "    ├─ [{sub_type}] {sub_name}  {sub_fields} fields"
                    ));
                }
            }
        }
        _ => lines.push(format!("{indent}[{ds_type}] {name}  {field_count} fields")),
    }
}

pub fn dcs_info_overview_hints(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    ns_settings: &str,
) {
    lines.push(String::new());
    let mut hints = Vec::<String>::new();
    let mut query_names = Vec::<String>::new();
    for data_set in unica_xml_format::read::child_elements_in_namespace(root, "dataSet", ns_schema)
    {
        if dcs_info_dataset_type(data_set) == "Query" {
            query_names.push(
                unica_xml_format::read::child_element_in_namespace(data_set, "name", ns_schema)
                    .map(dcs_text_of)
                    .unwrap_or_default(),
            );
        } else if dcs_info_dataset_type(data_set) == "Union" {
            for sub_ds in
                unica_xml_format::read::child_elements_in_namespace(data_set, "item", ns_schema)
            {
                if dcs_info_dataset_type(sub_ds) == "Query" {
                    query_names.push(
                        unica_xml_format::read::child_element_in_namespace(
                            sub_ds, "name", ns_schema,
                        )
                        .map(dcs_text_of)
                        .unwrap_or_default(),
                    );
                }
            }
        }
    }
    if query_names.len() == 1 {
        hints.push("-Mode query             query text".to_string());
    } else if query_names.len() > 1 {
        hints.push(format!(
            "-Mode query -Name <ds>  query text ({})",
            query_names.join(", ")
        ));
    }
    hints.push("-Mode fields            field tables by dataset".to_string());
    let links = unica_xml_format::read::child_elements_in_namespace(root, "dataSetLink", ns_schema);
    if !links.is_empty() {
        hints.push(format!(
            "-Mode links             dataset connections ({})",
            links.len()
        ));
    }
    let calculated =
        unica_xml_format::read::child_elements_in_namespace(root, "calculatedField", ns_schema);
    if !calculated.is_empty() {
        hints.push(format!(
            "-Mode calculated        calculated field expressions ({})",
            calculated.len()
        ));
    }
    let totals = unica_xml_format::read::child_elements_in_namespace(root, "totalField", ns_schema);
    if !totals.is_empty() {
        hints.push(format!(
            "-Mode resources         resource aggregation ({})",
            totals.len()
        ));
    }
    if !unica_xml_format::read::child_elements_in_namespace(root, "parameter", ns_schema).is_empty()
    {
        hints.push("-Mode params            parameter details".to_string());
    }
    let variants =
        unica_xml_format::read::child_elements_in_namespace(root, "settingsVariant", ns_schema);
    if variants.len() == 1 {
        hints.push("-Mode variant           variant structure".to_string());
    } else if variants.len() > 1 {
        hints.push(format!(
            "-Mode variant -Name <N> variant structure (1..{})",
            variants.len()
        ));
    }
    if !unica_xml_format::read::child_elements_in_namespace(root, "template", ns_schema).is_empty()
    {
        hints.push("-Mode templates         template bindings and expressions".to_string());
    }
    let _ = ns_settings;
    hints.push("-Mode trace -Name <f>   trace field origin (by name or title)".to_string());
    hints.push("-Mode full              all sections at once".to_string());
    lines.push("Next:".to_string());
    for hint in hints {
        lines.push(format!("  {hint}"));
    }
}

pub fn dcs_info_query(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    name: Option<&str>,
) -> Result<(), String> {
    let target = dcs_info_query_target(root, ns_schema, name)?;
    let query = unica_xml_format::read::child_element_in_namespace(target, "query", ns_schema)
        .map(dcs_inner_text)
        .unwrap_or_default();
    let name = unica_xml_format::read::child_element_in_namespace(target, "name", ns_schema)
        .map(dcs_text_of)
        .unwrap_or_default();
    lines.push(format!(
        "=== Query: {name} ({} lines) ===",
        query.split('\n').count()
    ));
    lines.push(String::new());
    for line in query.trim().split('\n') {
        lines.push(line.trim_end().to_string());
    }
    Ok(())
}

pub fn dcs_info_raw_query(
    root: roxmltree::Node<'_, '_>,
    ns_schema: &str,
    name: Option<&str>,
) -> Result<String, String> {
    let target = dcs_info_query_target(root, ns_schema, name)?;
    Ok(
        unica_xml_format::read::child_element_in_namespace(target, "query", ns_schema)
            .map(dcs_inner_text)
            .unwrap_or_default(),
    )
}

fn dcs_info_query_target<'a, 'input>(
    root: roxmltree::Node<'a, 'input>,
    ns_schema: &str,
    name: Option<&str>,
) -> Result<roxmltree::Node<'a, 'input>, String> {
    let mut target = None;
    if let Some(name) = name.filter(|value| !value.is_empty()) {
        for data_set in
            unica_xml_format::read::child_elements_in_namespace(root, "dataSet", ns_schema)
        {
            if dcs_info_dataset_type(data_set) == "Union" {
                for sub_ds in
                    unica_xml_format::read::child_elements_in_namespace(data_set, "item", ns_schema)
                {
                    let ds_name = unica_xml_format::read::child_element_in_namespace(
                        sub_ds, "name", ns_schema,
                    )
                    .map(dcs_text_of)
                    .unwrap_or_default();
                    if ds_name == name {
                        target = Some(sub_ds);
                        break;
                    }
                }
            }
            if target.is_some() {
                break;
            }
        }
        for data_set in
            unica_xml_format::read::child_elements_in_namespace(root, "dataSet", ns_schema)
        {
            if target.is_some() {
                break;
            }
            let ds_name =
                unica_xml_format::read::child_element_in_namespace(data_set, "name", ns_schema)
                    .map(dcs_text_of)
                    .unwrap_or_default();
            if ds_name == name {
                target = Some(data_set);
                break;
            }
        }
        if target.is_none() {
            return Err(format!("Dataset '{name}' not found"));
        }
    } else {
        for data_set in
            unica_xml_format::read::child_elements_in_namespace(root, "dataSet", ns_schema)
        {
            if dcs_info_dataset_type(data_set) == "Query" {
                target = Some(data_set);
                break;
            }
            if dcs_info_dataset_type(data_set) == "Union" {
                for sub_ds in
                    unica_xml_format::read::child_elements_in_namespace(data_set, "item", ns_schema)
                {
                    if dcs_info_dataset_type(sub_ds) == "Query" {
                        target = Some(sub_ds);
                        break;
                    }
                }
            }
            if target.is_some() {
                break;
            }
        }
    }
    let Some(target) = target else {
        return Err("No Query dataset found".to_string());
    };
    if unica_xml_format::read::child_element_in_namespace(target, "query", ns_schema).is_none() {
        if dcs_info_dataset_type(target) == "Union" {
            let sub_names =
                unica_xml_format::read::child_elements_in_namespace(target, "item", ns_schema)
                    .into_iter()
                    .filter_map(|sub_ds| {
                        unica_xml_format::read::child_element_in_namespace(
                            sub_ds, "name", ns_schema,
                        )
                        .map(dcs_text_of)
                    })
                    .collect::<Vec<_>>();
            let ds_name =
                unica_xml_format::read::child_element_in_namespace(target, "name", ns_schema)
                    .map(dcs_text_of)
                    .unwrap_or_default();
            return Err(format!(
                "Dataset '{ds_name}' is a Union. Specify nested: {}",
                sub_names.join(", ")
            ));
        }
        return Err("Dataset has no query element".to_string());
    }
    Ok(target)
}

pub fn dcs_info_fields(root: roxmltree::Node<'_, '_>, lines: &mut Vec<String>, ns_schema: &str) {
    lines.push("=== Fields map ===".to_string());
    for data_set in unica_xml_format::read::child_elements_in_namespace(root, "dataSet", ns_schema)
    {
        dcs_info_field_map(data_set, lines, ns_schema, "");
        if dcs_info_dataset_type(data_set) == "Union" {
            for sub_ds in
                unica_xml_format::read::child_elements_in_namespace(data_set, "item", ns_schema)
            {
                dcs_info_field_map(sub_ds, lines, ns_schema, "  ");
            }
        }
    }
    lines.push(String::new());
    lines.push("Use -Name <field> for details.".to_string());
}

pub fn dcs_info_field_map(
    data_set: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    indent: &str,
) {
    let fields = unica_xml_format::read::child_elements_in_namespace(data_set, "field", ns_schema)
        .into_iter()
        .filter_map(|field| {
            unica_xml_format::read::child_element_in_namespace(field, "dataPath", ns_schema)
                .map(dcs_text_of)
        })
        .collect::<Vec<_>>();
    let name = unica_xml_format::read::child_element_in_namespace(data_set, "name", ns_schema)
        .map(dcs_text_of)
        .unwrap_or_default();
    let mut name_list = fields.join(", ");
    if name_list.chars().count() > 100 {
        name_list = format!("{}...", name_list.chars().take(97).collect::<String>());
    }
    lines.push(format!(
        "{indent}{name} [{}] ({}): {name_list}",
        dcs_info_dataset_type(data_set),
        fields.len()
    ));
}

pub fn dcs_info_links(root: roxmltree::Node<'_, '_>, lines: &mut Vec<String>, ns_schema: &str) {
    let links = unica_xml_format::read::child_elements_in_namespace(root, "dataSetLink", ns_schema);
    if links.is_empty() {
        lines.push("(no links)".to_string());
    } else {
        lines.push(format!("=== Links ({}) ===", links.len()));
    }
}

pub fn dcs_info_calculated(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    name: Option<&str>,
) -> Result<(), String> {
    let calculated =
        unica_xml_format::read::child_elements_in_namespace(root, "calculatedField", ns_schema);
    if calculated.is_empty() {
        lines.push("(no calculated fields)".to_string());
        return Ok(());
    }
    if let Some(name) = name.filter(|value| !value.is_empty()) {
        for field in calculated {
            let path =
                unica_xml_format::read::child_element_in_namespace(field, "dataPath", ns_schema)
                    .map(dcs_text_of)
                    .unwrap_or_default();
            if path != name {
                continue;
            }
            lines.push(format!("=== Calculated: {path} ==="));
            lines.push(String::new());
            lines.push("Expression:".to_string());
            let expression =
                unica_xml_format::read::child_element_in_namespace(field, "expression", ns_schema)
                    .map(dcs_all_text)
                    .unwrap_or_default();
            for line in expression.split('\n') {
                lines.push(format!("  {}", line.trim_end()));
            }
            if let Some(title) =
                unica_xml_format::read::child_element_in_namespace(field, "title", ns_schema)
                    .map(dcs_info_multilang_or_inner_text)
                    .filter(|value| !value.is_empty())
            {
                lines.push(format!("Title: {title}"));
            }
            if let Some(restriction) = unica_xml_format::read::child_element_in_namespace(
                field,
                "useRestriction",
                ns_schema,
            ) {
                let parts = restriction
                    .children()
                    .filter(|child| child.is_element())
                    .filter(|child| dcs_text_of(*child) == "true")
                    .map(|child| child.tag_name().name().to_string())
                    .collect::<Vec<_>>();
                if !parts.is_empty() {
                    lines.push(format!("Restrict: {}", parts.join(", ")));
                }
            }
            return Ok(());
        }
        return Err(format!("Calculated field '{name}' not found"));
    }

    lines.push(format!("=== Calculated fields ({}) ===", calculated.len()));
    for field in calculated {
        let path = unica_xml_format::read::child_element_in_namespace(field, "dataPath", ns_schema)
            .map(dcs_text_of)
            .unwrap_or_default();
        let title = unica_xml_format::read::child_element_in_namespace(field, "title", ns_schema)
            .map(dcs_info_multilang_or_inner_text)
            .unwrap_or_default();
        let title_str = if !title.is_empty() && title != path {
            format!("  \"{title}\"")
        } else {
            String::new()
        };
        lines.push(format!("  {path}{title_str}"));
    }
    lines.push(String::new());
    lines.push("Use -Name <field> for full expression.".to_string());
    Ok(())
}

pub fn dcs_info_resources(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    name: Option<&str>,
) -> Result<(), String> {
    let totals = unica_xml_format::read::child_elements_in_namespace(root, "totalField", ns_schema);
    if totals.is_empty() {
        lines.push("(no resources)".to_string());
        return Ok(());
    }
    if let Some(name) = name.filter(|value| !value.is_empty()) {
        let matched = totals
            .into_iter()
            .filter(|total| {
                unica_xml_format::read::child_element_in_namespace(*total, "dataPath", ns_schema)
                    .map(dcs_text_of)
                    .is_some_and(|path| path == name)
            })
            .collect::<Vec<_>>();
        if matched.is_empty() {
            return Err(format!("Resource '{name}' not found"));
        }
        lines.push(format!("=== Resource: {name} ==="));
        lines.push(String::new());
        for total in matched {
            let expression =
                unica_xml_format::read::child_element_in_namespace(total, "expression", ns_schema)
                    .map(dcs_text_of)
                    .unwrap_or_default();
            let group =
                unica_xml_format::read::child_element_in_namespace(total, "group", ns_schema)
                    .map(dcs_text_of)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| "(overall)".to_string());
            lines.push(format!("  [{group}] {expression}"));
        }
        return Ok(());
    }

    lines.push(format!("=== Resources ({}) ===", totals.len()));
    let mut ordered = Vec::<String>::new();
    let mut has_group = BTreeMap::<String, bool>::new();
    for total in totals {
        let path = unica_xml_format::read::child_element_in_namespace(total, "dataPath", ns_schema)
            .map(dcs_text_of)
            .unwrap_or_default();
        if !has_group.contains_key(&path) {
            ordered.push(path.clone());
        }
        if unica_xml_format::read::child_element_in_namespace(total, "group", ns_schema).is_some() {
            has_group.insert(path, true);
        } else {
            has_group.entry(path).or_insert(false);
        }
    }
    for path in ordered {
        let mark = if has_group.get(&path).copied().unwrap_or(false) {
            " *"
        } else {
            ""
        };
        lines.push(format!("  {path}{mark}"));
    }
    lines.push(String::new());
    lines.push("  * = has group-level formulas".to_string());
    lines.push(String::new());
    lines.push("Use -Name <field> for full formula.".to_string());
    Ok(())
}

pub fn dcs_info_params(root: roxmltree::Node<'_, '_>, lines: &mut Vec<String>, ns_schema: &str) {
    let params = unica_xml_format::read::child_elements_in_namespace(root, "parameter", ns_schema);
    lines.push(format!("=== Parameters ({}) ===", params.len()));
    lines.push("  Name                            Type                   Default          Visible  Expression".to_string());
    for param in params {
        let name = unica_xml_format::read::child_element_in_namespace(param, "name", ns_schema)
            .map(dcs_text_of)
            .unwrap_or_default();
        let type_name =
            unica_xml_format::read::child_element_in_namespace(param, "valueType", ns_schema)
                .map(dcs_info_compact_type)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "-".to_string());
        let default = unica_xml_format::read::child_element_in_namespace(param, "value", ns_schema)
            .map(dcs_info_param_default)
            .unwrap_or_else(|| "-".to_string());
        let visible =
            unica_xml_format::read::child_element_in_namespace(param, "useRestriction", ns_schema)
                .map(dcs_text_of)
                .map(|value| if value == "true" { "hidden" } else { "yes" })
                .unwrap_or("yes");
        let expression =
            unica_xml_format::read::child_element_in_namespace(param, "expression", ns_schema)
                .map(dcs_all_text)
                .map(|value| {
                    if value.is_empty() {
                        "-".to_string()
                    } else {
                        value
                    }
                })
                .unwrap_or_else(|| "-".to_string());
        let no_field = unica_xml_format::read::child_element_in_namespace(
            param,
            "availableAsField",
            ns_schema,
        )
        .map(dcs_text_of)
        .is_some_and(|value| value == "false");
        let suffix = if no_field { " [noField]" } else { "" };
        lines.push(format!(
            "  {:<33} {:<22} {:<16} {:<8} {}{}",
            name, type_name, default, visible, expression, suffix
        ));
    }
}

pub fn dcs_info_variant(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    ns_settings: &str,
) {
    let variants =
        unica_xml_format::read::child_elements_in_namespace(root, "settingsVariant", ns_schema);
    if variants.is_empty() {
        lines.push("=== Variants: (none) ===".to_string());
        return;
    }
    lines.push(format!("=== Variants ({}) ===", variants.len()));
    for (index, variant) in variants.iter().enumerate() {
        let name =
            unica_xml_format::read::child_element_in_namespace(*variant, "name", ns_settings)
                .map(dcs_text_of)
                .unwrap_or_default();
        let presentation = unica_xml_format::read::child_element_in_namespace(
            *variant,
            "presentation",
            ns_settings,
        )
        .map(dcs_info_multilang_or_inner_text)
        .unwrap_or_default();
        let presentation_str = if presentation.is_empty() {
            String::new()
        } else {
            format!("  \"{presentation}\"")
        };
        let settings =
            unica_xml_format::read::child_element_in_namespace(*variant, "settings", ns_settings);
        let mut struct_items = Vec::new();
        let mut filter_count = 0usize;
        let mut selection = Vec::new();
        if let Some(settings) = settings {
            for item in
                unica_xml_format::read::child_elements_in_namespace(settings, "item", ns_settings)
            {
                let item_type = dcs_info_structure_item_type(item);
                let group_fields = dcs_info_group_fields(item, ns_settings);
                let group = if group_fields.is_empty() {
                    "(detail)".to_string()
                } else {
                    format!("({})", group_fields.join(","))
                };
                struct_items.push(format!("{item_type}{group}"));
            }
            if struct_items.len() > 3 {
                let mut counts = BTreeMap::<String, usize>::new();
                for item in &struct_items {
                    *counts.entry(item.clone()).or_insert(0) += 1;
                }
                let mut compact = Vec::new();
                for item in &struct_items {
                    if compact
                        .iter()
                        .any(|existing: &String| existing.ends_with(item))
                    {
                        continue;
                    }
                    let count = counts.get(item).copied().unwrap_or(1);
                    if count > 1 {
                        compact.push(format!("{count}x {item}"));
                    } else {
                        compact.push(item.clone());
                    }
                }
                struct_items = compact;
            }
            if let Some(filter) =
                unica_xml_format::read::child_element_in_namespace(settings, "filter", ns_settings)
            {
                filter_count = unica_xml_format::read::child_elements_in_namespace(
                    filter,
                    "item",
                    ns_settings,
                )
                .len();
            }
            selection = dcs_info_selection_fields(settings, ns_settings);
        }
        let struct_str = if struct_items.is_empty() {
            String::new()
        } else {
            format!("  {}", struct_items.join(", "))
        };
        let filter_str = if filter_count > 0 {
            format!("  {filter_count} filters")
        } else {
            String::new()
        };
        lines.push(format!(
            "  [{}] {name}{presentation_str}{struct_str}{filter_str}",
            index + 1
        ));
        if !selection.is_empty() {
            lines.push(format!("        sel: {}", selection.join(", ")));
        }
    }
}

pub fn dcs_info_trace(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    name: &str,
) -> Result<(), String> {
    let mut dataset_hits = Vec::<String>::new();
    let mut title = String::new();
    for data_set in unica_xml_format::read::child_elements_in_namespace(root, "dataSet", ns_schema)
    {
        dcs_info_collect_field_trace(data_set, ns_schema, name, &mut dataset_hits, &mut title);
        for sub_ds in
            unica_xml_format::read::child_elements_in_namespace(data_set, "item", ns_schema)
        {
            dcs_info_collect_field_trace(sub_ds, ns_schema, name, &mut dataset_hits, &mut title);
        }
    }

    let mut calc_expression = None::<String>;
    let mut calc_operands = Vec::<String>::new();
    for field in
        unica_xml_format::read::child_elements_in_namespace(root, "calculatedField", ns_schema)
    {
        let path = unica_xml_format::read::child_element_in_namespace(field, "dataPath", ns_schema)
            .map(dcs_text_of)
            .unwrap_or_default();
        let field_title =
            unica_xml_format::read::child_element_in_namespace(field, "title", ns_schema)
                .map(dcs_info_multilang_or_inner_text)
                .unwrap_or_default();
        if path == name || field_title == name {
            if title.is_empty() {
                title = field_title;
            }
            let expression =
                unica_xml_format::read::child_element_in_namespace(field, "expression", ns_schema)
                    .map(dcs_all_text)
                    .unwrap_or_default();
            for data_set in
                unica_xml_format::read::child_elements_in_namespace(root, "dataSet", ns_schema)
            {
                for operand in dcs_info_dataset_field_paths(data_set, ns_schema) {
                    if !operand.is_empty() && expression.contains(&operand) {
                        let ds_name = unica_xml_format::read::child_element_in_namespace(
                            data_set, "name", ns_schema,
                        )
                        .map(dcs_text_of)
                        .unwrap_or_default();
                        calc_operands.push(format!(
                            "{operand} -> {ds_name} [{}]",
                            dcs_info_dataset_type(data_set)
                        ));
                    }
                }
            }
            calc_expression = Some(expression);
        }
    }

    let mut resources = Vec::<String>::new();
    for total in unica_xml_format::read::child_elements_in_namespace(root, "totalField", ns_schema)
    {
        let path = unica_xml_format::read::child_element_in_namespace(total, "dataPath", ns_schema)
            .map(dcs_text_of)
            .unwrap_or_default();
        if path == name {
            let group =
                unica_xml_format::read::child_element_in_namespace(total, "group", ns_schema)
                    .map(dcs_text_of)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| "(overall)".to_string());
            let expression =
                unica_xml_format::read::child_element_in_namespace(total, "expression", ns_schema)
                    .map(dcs_text_of)
                    .unwrap_or_default();
            resources.push(format!("  [{group}] {expression}"));
        }
    }

    if dataset_hits.is_empty() && calc_expression.is_none() && resources.is_empty() {
        return Err(format!("Field '{name}' not found by dataPath or title"));
    }

    let title_str = if title.is_empty() {
        String::new()
    } else {
        format!(" \"{title}\"")
    };
    lines.push(format!("=== Trace: {name}{title_str} ==="));
    lines.push(String::new());
    if dataset_hits.is_empty() {
        lines.push("Dataset: (schema-level only, not in dataset fields)".to_string());
    } else {
        lines.push(format!("Dataset: {}", dataset_hits.join(", ")));
    }
    if let Some(expression) = calc_expression {
        lines.push(String::new());
        lines.push("Calculated:".to_string());
        for line in expression.split('\n') {
            lines.push(format!("  {}", line.trim_end()));
        }
        if !calc_operands.is_empty() {
            lines.push("  Operands:".to_string());
            for operand in calc_operands {
                lines.push(format!("    {operand}"));
            }
        }
    }
    if !resources.is_empty() {
        lines.push(String::new());
        lines.push("Resource:".to_string());
        lines.extend(resources);
    }
    Ok(())
}

pub fn dcs_info_full(
    root: roxmltree::Node<'_, '_>,
    resolved_path: &Path,
    text: &str,
    lines: &mut Vec<String>,
    ns_schema: &str,
    ns_settings: &str,
) -> Result<(), String> {
    dcs_info_overview(root, resolved_path, text, lines, ns_schema, ns_settings);
    lines.push(String::new());
    lines.push("--- query ---".to_string());
    lines.push(String::new());
    if unica_xml_format::read::child_elements_in_namespace(root, "dataSet", ns_schema)
        .iter()
        .any(|data_set| dcs_info_dataset_type(*data_set) == "Query")
    {
        dcs_info_query(root, lines, ns_schema, None)?;
    } else {
        let object_names =
            unica_xml_format::read::child_elements_in_namespace(root, "dataSet", ns_schema)
                .into_iter()
                .filter(|data_set| dcs_info_dataset_type(*data_set) == "Object")
                .filter_map(|data_set| {
                    unica_xml_format::read::child_element_in_namespace(
                        data_set,
                        "objectName",
                        ns_schema,
                    )
                    .map(dcs_text_of)
                })
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>();
        if object_names.is_empty() {
            lines.push("(no query datasets)".to_string());
        } else {
            lines.push(format!(
                "(no query datasets; external datasets: {})",
                object_names.join(", ")
            ));
        }
    }
    lines.push(String::new());
    lines.push("--- fields ---".to_string());
    lines.push(String::new());
    dcs_info_fields(root, lines, ns_schema);
    lines.push(String::new());
    lines.push("--- resources ---".to_string());
    lines.push(String::new());
    dcs_info_resources(root, lines, ns_schema, None)?;
    lines.push(String::new());
    lines.push("--- params ---".to_string());
    lines.push(String::new());
    dcs_info_params(root, lines, ns_schema);
    lines.push(String::new());
    lines.push("--- variant ---".to_string());
    lines.push(String::new());
    dcs_info_variant(root, lines, ns_schema, ns_settings);
    Ok(())
}

pub fn dcs_info_templates(root: roxmltree::Node<'_, '_>, lines: &mut Vec<String>, ns_schema: &str) {
    let templates =
        unica_xml_format::read::child_elements_in_namespace(root, "template", ns_schema);
    let field_count =
        unica_xml_format::read::child_elements_in_namespace(root, "fieldTemplate", ns_schema).len();
    let group_count =
        unica_xml_format::read::child_elements_in_namespace(root, "groupTemplate", ns_schema).len()
            + unica_xml_format::read::child_elements_in_namespace(
                root,
                "groupHeaderTemplate",
                ns_schema,
            )
            .len()
            + unica_xml_format::read::child_elements_in_namespace(
                root,
                "groupFooterTemplate",
                ns_schema,
            )
            .len();
    lines.push(format!(
        "=== Templates ({} defined: {field_count} field, {group_count} group) ===",
        templates.len()
    ));
}

pub fn dcs_info_dataset_type(data_set: roxmltree::Node<'_, '_>) -> String {
    [
        ("DataSetQuery", "Query"),
        ("DataSetObject", "Object"),
        ("DataSetUnion", "Union"),
    ]
    .into_iter()
    .find_map(|(xml, kind)| xsi_type_matches(data_set, DCS_SCHEMA_NS, xml).then_some(kind))
    .unwrap_or("Unknown")
    .to_string()
}

pub fn dcs_info_structure_item_type(item: roxmltree::Node<'_, '_>) -> &'static str {
    if ["row", "column", "point", "series"]
        .iter()
        .any(|tag| unica_xml_format::names::element_has_name(item, tag, Some(DCS_SETTINGS_NS)))
    {
        return "Group";
    }
    [
        ("StructureItemGroup", "Group"),
        ("StructureItemTable", "Table"),
        ("StructureItemChart", "Chart"),
    ]
    .into_iter()
    .find_map(|(xml, kind)| xsi_type_matches(item, DCS_SETTINGS_NS, xml).then_some(kind))
    .unwrap_or("Unknown")
}

pub fn dcs_info_multilang_or_inner_text(node: roxmltree::Node<'_, '_>) -> String {
    let value = unica_xml_format::read::multilang_text(node);
    if value.is_empty() {
        if let Some(text) = node.text().map(str::trim).filter(|value| !value.is_empty()) {
            return text.to_string();
        }
        dcs_all_text(node)
    } else {
        value
    }
}

pub fn dcs_info_group_fields(item: roxmltree::Node<'_, '_>, ns_settings: &str) -> Vec<String> {
    let mut fields = Vec::new();
    for group_item in dcs_find_all_path(item, &[("groupItems", ns_settings), ("item", ns_settings)])
    {
        if let Some(field) =
            unica_xml_format::read::child_element_in_namespace(group_item, "field", ns_settings)
        {
            let mut value = dcs_text_of(field);
            let group_type = unica_xml_format::read::child_element_in_namespace(
                group_item,
                "groupType",
                ns_settings,
            )
            .map(dcs_text_of)
            .unwrap_or_default();
            if !group_type.is_empty() && group_type != "Items" {
                value.push_str(&format!("({group_type})"));
            }
            fields.push(value);
        }
    }
    fields
}

pub fn dcs_info_selection_fields(
    item_node: roxmltree::Node<'_, '_>,
    ns_settings: &str,
) -> Vec<String> {
    let mut fields = Vec::new();
    if let Some(selection) =
        unica_xml_format::read::child_element_in_namespace(item_node, "selection", ns_settings)
    {
        for item in
            unica_xml_format::read::child_elements_in_namespace(selection, "item", ns_settings)
        {
            let xsi_type =
                unica_xml_format::read::attribute_by_local_name(item, "type").unwrap_or("");
            if xsi_type.contains("SelectedItemAuto") {
                fields.push("Auto".to_string());
            } else if xsi_type.contains("SelectedItemField") {
                if let Some(field) =
                    unica_xml_format::read::child_element_in_namespace(item, "field", ns_settings)
                {
                    fields.push(dcs_text_of(field));
                }
            } else if xsi_type.contains("SelectedItemFolder") {
                fields.push("Folder".to_string());
            }
        }
    }
    fields
}

pub fn dcs_info_compact_type(value_type: roxmltree::Node<'_, '_>) -> String {
    let mut types = Vec::new();
    for type_node in value_type
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "Type")
    {
        let raw = dcs_text_of(type_node);
        let mapped = match raw.as_str() {
            "xs:string" => "String".to_string(),
            "xs:decimal" => "Number".to_string(),
            "xs:boolean" => "Boolean".to_string(),
            "xs:dateTime" => "DateTime".to_string(),
            "v8:StandardPeriod" => "StandardPeriod".to_string(),
            "v8:StandardBeginningDate" => "StandardBeginningDate".to_string(),
            "v8:AccountType" => "AccountType".to_string(),
            "v8:Null" => "Null".to_string(),
            _ => raw
                .split_once(':')
                .map(|(_, local)| local.to_string())
                .unwrap_or(raw),
        };
        types.push(mapped);
    }
    types.join(" | ")
}

pub fn dcs_info_param_default(value_node: roxmltree::Node<'_, '_>) -> String {
    if unica_xml_format::read::attribute_by_local_name(value_node, "nil")
        .is_some_and(|value| value == "true")
    {
        return "null".to_string();
    }
    let raw = dcs_all_text(value_node);
    if raw == "0001-01-01T00:00:00" || raw.is_empty() {
        return "-".to_string();
    }
    if let Some(variant) = value_node
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "variant")
    {
        return dcs_text_of(variant);
    }
    if raw.chars().count() > 15 {
        format!("{}...", raw.chars().take(12).collect::<String>())
    } else {
        raw
    }
}

pub fn dcs_info_collect_field_trace(
    data_set: roxmltree::Node<'_, '_>,
    ns_schema: &str,
    name: &str,
    dataset_hits: &mut Vec<String>,
    title: &mut String,
) {
    let ds_name = unica_xml_format::read::child_element_in_namespace(data_set, "name", ns_schema)
        .map(dcs_text_of)
        .unwrap_or_default();
    let ds_type = dcs_info_dataset_type(data_set);
    for field in unica_xml_format::read::child_elements_in_namespace(data_set, "field", ns_schema) {
        let path = unica_xml_format::read::child_element_in_namespace(field, "dataPath", ns_schema)
            .map(dcs_text_of)
            .unwrap_or_default();
        let field_title =
            unica_xml_format::read::child_element_in_namespace(field, "title", ns_schema)
                .map(dcs_info_multilang_or_inner_text)
                .unwrap_or_default();
        if path == name || field_title == name {
            if title.is_empty() {
                *title = field_title;
            }
            dataset_hits.push(format!("{ds_name} [{ds_type}]"));
        }
    }
}

pub fn dcs_info_dataset_field_paths(
    data_set: roxmltree::Node<'_, '_>,
    ns_schema: &str,
) -> Vec<String> {
    unica_xml_format::read::child_elements_in_namespace(data_set, "field", ns_schema)
        .into_iter()
        .filter_map(|field| {
            unica_xml_format::read::child_element_in_namespace(field, "dataPath", ns_schema)
                .map(dcs_text_of)
        })
        .collect()
}

pub fn dcs_info_template_name(path: &Path) -> String {
    let parts = path
        .components()
        .map(|part| part.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    for index in (0..parts.len()).rev() {
        if parts[index] == "Ext" && index >= 1 {
            return parts[index - 1].clone();
        }
    }
    path.display().to_string()
}

pub fn validate_dcs_text(
    args: &Map<String, Value>,
    text: &str,
    resolved_path: PathBuf,
) -> Result<DcsValidationRun, String> {
    const NS_SCHEMA: &str = DCS_SCHEMA_NS;
    let file_name = resolved_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_string();
    let detailed = unica_operation_contract::input::bool_arg(args, &["detailed", "Detailed"]);
    let max_errors = unica_operation_contract::input::int_arg(args, &["maxErrors", "MaxErrors"])
        .unwrap_or(20)
        .max(0) as usize;
    let mut report = DcsValidationReporter::new(max_errors, detailed, &file_name);
    let doc = match Document::parse(text.trim_start_matches('\u{feff}')) {
        Ok(doc) => {
            report.ok("XML parsed successfully");
            doc
        }
        Err(err) => {
            report.error(format!("XML parse failed: {err}"));
            let errors = report
                .lines
                .iter()
                .filter(|line| line.starts_with("[ERROR] "))
                .cloned()
                .collect::<Vec<_>>();
            return Ok(DcsValidationRun {
                ok: false,
                stdout: format!("{}\n", report.lines.join("\n")),
                artifact: resolved_path,
                errors,
            });
        }
    };

    let root = doc.root_element();
    if let Err(error) = require_dcs_root(root) {
        report.error(error);
        return dcs_validation_finish(report, &file_name, resolved_path);
    }
    report.ok("Root element: DataCompositionSchema");
    report.ok("Default namespace correct");

    let data_source_nodes =
        unica_xml_format::read::child_elements_in_namespace(root, "dataSource", NS_SCHEMA);
    let mut data_source_names = HashSet::<String>::new();
    for dsn in &data_source_nodes {
        if let Some(name) =
            unica_xml_format::read::child_element_in_namespace(*dsn, "name", NS_SCHEMA)
        {
            data_source_names.insert(dcs_inner_text(name));
        }
    }

    let data_set_nodes =
        unica_xml_format::read::child_elements_in_namespace(root, "dataSet", NS_SCHEMA);
    let mut data_set_names = HashSet::<String>::new();
    let mut all_field_paths = HashMap::<String, String>::new();
    for ds in &data_set_nodes {
        if let Some(name_node) =
            unica_xml_format::read::child_element_in_namespace(*ds, "name", NS_SCHEMA)
        {
            let ds_name = dcs_inner_text(name_node);
            data_set_names.insert(ds_name.clone());
            dcs_collect_data_set_fields(*ds, &ds_name, &mut all_field_paths);
        }
    }

    let calc_field_nodes =
        unica_xml_format::read::child_elements_in_namespace(root, "calculatedField", NS_SCHEMA);
    let mut calc_field_paths = HashSet::<String>::new();
    for cf in &calc_field_nodes {
        if let Some(dp) =
            unica_xml_format::read::child_element_in_namespace(*cf, "dataPath", NS_SCHEMA)
        {
            calc_field_paths.insert(dcs_inner_text(dp));
        }
    }
    let total_field_nodes =
        unica_xml_format::read::child_elements_in_namespace(root, "totalField", NS_SCHEMA);
    let param_nodes =
        unica_xml_format::read::child_elements_in_namespace(root, "parameter", NS_SCHEMA);
    let template_nodes =
        unica_xml_format::read::child_elements_in_namespace(root, "template", NS_SCHEMA);
    let mut template_names = HashSet::<String>::new();
    for template in &template_nodes {
        if let Some(name_node) =
            unica_xml_format::read::child_element_in_namespace(*template, "name", NS_SCHEMA)
        {
            template_names.insert(dcs_inner_text(name_node));
        }
    }
    let group_template_nodes =
        unica_xml_format::read::child_elements_in_namespace(root, "groupTemplate", NS_SCHEMA);
    let variant_nodes =
        unica_xml_format::read::child_elements_in_namespace(root, "settingsVariant", NS_SCHEMA);
    let mut known_fields = all_field_paths.keys().cloned().collect::<HashSet<String>>();
    known_fields.extend(calc_field_paths.iter().cloned());

    dcs_validate_data_sources(&mut report, &data_source_nodes);
    if report.stopped {
        return dcs_validation_finish(report, &file_name, resolved_path);
    }
    dcs_validate_data_sets(&mut report, &data_set_nodes, &data_source_names);
    if report.stopped {
        return dcs_validation_finish(report, &file_name, resolved_path);
    }
    for ds in &data_set_nodes {
        let ds_name = unica_xml_format::read::child_element_in_namespace(*ds, "name", NS_SCHEMA)
            .map(dcs_inner_text)
            .unwrap_or_else(|| "(unnamed)".to_string());
        dcs_validate_data_set_fields(&mut report, *ds, &ds_name);
        if report.stopped {
            return dcs_validation_finish(report, &file_name, resolved_path.clone());
        }
    }
    if report.stopped {
        return dcs_validation_finish(report, &file_name, resolved_path);
    }
    dcs_validate_data_set_links(&mut report, root, &data_set_names);
    if report.stopped {
        return dcs_validation_finish(report, &file_name, resolved_path);
    }
    dcs_validate_calculated_fields(&mut report, &calc_field_nodes, &all_field_paths);
    if report.stopped {
        return dcs_validation_finish(report, &file_name, resolved_path);
    }
    dcs_validate_total_fields(&mut report, &total_field_nodes);
    if report.stopped {
        return dcs_validation_finish(report, &file_name, resolved_path);
    }
    dcs_validate_parameters(&mut report, &param_nodes);
    if report.stopped {
        return dcs_validation_finish(report, &file_name, resolved_path);
    }
    dcs_validate_templates(&mut report, &template_nodes);
    if report.stopped {
        return dcs_validation_finish(report, &file_name, resolved_path);
    }
    dcs_validate_group_templates(&mut report, &group_template_nodes, &template_names);
    if report.stopped {
        return dcs_validation_finish(report, &file_name, resolved_path);
    }
    dcs_validate_settings_variants(&mut report, &variant_nodes, &known_fields);
    if report.stopped {
        return dcs_validation_finish(report, &file_name, resolved_path);
    }
    dcs_validate_value_types(&mut report, root);
    if report.stopped {
        return dcs_validation_finish(report, &file_name, resolved_path);
    }
    dcs_validate_value_contents(&mut report, root);
    dcs_validation_finish(report, &file_name, resolved_path)
}

pub fn dcs_validation_finish(
    report: DcsValidationReporter,
    file_name: &str,
    artifact: PathBuf,
) -> Result<DcsValidationRun, String> {
    let (ok, stdout, errors) = report.finalize(file_name);
    Ok(DcsValidationRun {
        ok,
        stdout,
        artifact,
        errors,
    })
}

pub fn dcs_validate_data_sources(
    report: &mut DcsValidationReporter,
    data_source_nodes: &[roxmltree::Node<'_, '_>],
) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    if data_source_nodes.is_empty() {
        report.warn("No dataSource elements found (settings-only DCS?)");
        return;
    }
    let mut names_seen = HashSet::<String>::new();
    let mut ds_ok = true;
    for dsn in data_source_nodes {
        let name = unica_xml_format::read::child_element_in_namespace(*dsn, "name", NS_SCHEMA);
        let typ =
            unica_xml_format::read::child_element_in_namespace(*dsn, "dataSourceType", NS_SCHEMA);
        let name_text = name.map(dcs_inner_text).unwrap_or_default();
        if name_text.is_empty() {
            report.error("DataSource has empty name");
            ds_ok = false;
        } else if !names_seen.insert(name_text.clone()) {
            report.error(format!("Duplicate dataSource name: {name_text}"));
            ds_ok = false;
        }
        if let Some(typ) = typ {
            let type_text = dcs_inner_text(typ);
            if !matches!(type_text.as_str(), "Local" | "External") {
                report.warn(format!(
                    "DataSource '{name_text}' has unusual type: {type_text}"
                ));
            }
        }
    }
    if ds_ok {
        report.ok(format!(
            "{} dataSource(s) found, names unique",
            data_source_nodes.len()
        ));
    }
}

pub fn dcs_validate_value_types(report: &mut DcsValidationReporter, root: roxmltree::Node<'_, '_>) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    const NS_V8: &str = "http://v8.1c.ru/8.1/data/core";
    const NS_CONFIG: &str = "http://v8.1c.ru/8.1/data/enterprise/current-config";
    const NS_ENTERPRISE: &str = "http://v8.1c.ru/8.1/data/enterprise";
    let valid_types = [
        "xs:decimal",
        "xs:string",
        "xs:dateTime",
        "xs:boolean",
        "v8:StandardPeriod",
        "v8:UUID",
        "v8:Null",
        "v8:Type",
        "v8:ValueStorage",
    ];
    let valid_sign = ["Any", "Nonnegative", "Negative"];
    let valid_length = ["Variable", "Fixed"];
    let valid_fractions = ["Date", "DateTime", "Time"];
    let value_types = root
        .descendants()
        .filter(|node| {
            unica_xml_format::names::element_has_name(*node, "valueType", Some(NS_SCHEMA))
        })
        .collect::<Vec<_>>();
    if value_types.is_empty() {
        return;
    }

    let mut all_ok = true;
    for value_type in &value_types {
        let mut types = HashSet::<String>::new();
        let mut qualifiers = Vec::<String>::new();
        for child in value_type.children().filter(|child| child.is_element()) {
            if child.tag_name().namespace().unwrap_or("") != NS_V8 {
                continue;
            }
            let local = child.tag_name().name();
            if local == "Type" {
                let type_text = dcs_text_of(child);
                if type_text.is_empty() {
                    report.error("valueType: <v8:Type> is empty");
                    all_ok = false;
                    if report.stopped {
                        return;
                    }
                    continue;
                }
                let Some((prefix, local_type)) = type_text.split_once(':') else {
                    report.error(format!(
                        "valueType: type '{type_text}' has no namespace prefix (expected xs:/v8:/d5p1: — e.g. xs:decimal not decimal)"
                    ));
                    all_ok = false;
                    if report.stopped {
                        return;
                    }
                    continue;
                };
                if matches!(prefix, "xs" | "v8") {
                    if !valid_types.contains(&type_text.as_str()) {
                        report.error(format!(
                            "valueType: unknown type '{type_text}' (allowed: xs:decimal/xs:string/xs:dateTime/xs:boolean/v8:StandardPeriod or <prefix>:*Ref.X)"
                        ));
                        all_ok = false;
                    } else {
                        types.insert(type_text);
                    }
                } else {
                    let prefix_ns = child.lookup_namespace_uri(Some(prefix));
                    if prefix_ns == Some(NS_CONFIG) {
                        if !dcs_validate_config_ref_type_shape(local_type) {
                            report.error(format!(
                                "valueType: ref type '{type_text}' must look like '<prefix>:<Kind>.<Name>' (e.g. d5p1:CatalogRef.X)"
                            ));
                            all_ok = false;
                        } else {
                            types.insert(String::new());
                        }
                    } else if prefix_ns == Some(NS_ENTERPRISE) {
                        if !dcs_validate_system_type_shape(local_type) {
                            report.error(format!(
                                "valueType: system type '{type_text}' has unexpected local-name shape"
                            ));
                            all_ok = false;
                        } else {
                            types.insert(String::new());
                        }
                    } else {
                        report.error(format!(
                            "valueType: type '{type_text}' uses prefix '{prefix}' bound to unexpected namespace '{}'",
                            prefix_ns.unwrap_or("None")
                        ));
                        all_ok = false;
                    }
                }
                if report.stopped {
                    return;
                }
            } else if local.ends_with("Qualifiers") {
                let q_name = format!("v8:{local}");
                qualifiers.push(q_name.clone());
                match q_name.as_str() {
                    "v8:NumberQualifiers" => {
                        let digits = unica_xml_format::read::child_element_in_namespace(
                            child, "Digits", NS_V8,
                        )
                        .map(dcs_text_of);
                        let fraction = unica_xml_format::read::child_element_in_namespace(
                            child,
                            "FractionDigits",
                            NS_V8,
                        )
                        .map(dcs_text_of);
                        let sign = unica_xml_format::read::child_element_in_namespace(
                            child,
                            "AllowedSign",
                            NS_V8,
                        )
                        .map(dcs_text_of);
                        if digits
                            .as_deref()
                            .filter(|value| {
                                !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit())
                            })
                            .is_none()
                        {
                            report.error(
                                "v8:NumberQualifiers: <v8:Digits> missing or not a non-negative integer",
                            );
                            all_ok = false;
                        }
                        if fraction
                            .as_deref()
                            .filter(|value| {
                                !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit())
                            })
                            .is_none()
                        {
                            report.error(
                                "v8:NumberQualifiers: <v8:FractionDigits> missing or not a non-negative integer",
                            );
                            all_ok = false;
                        }
                        if let Some(sign) = sign.as_deref().filter(|value| !value.is_empty()) {
                            if !valid_sign.contains(&sign) {
                                report.error(format!(
                                    "v8:NumberQualifiers: <v8:AllowedSign>{sign}</v8:AllowedSign> — must be one of: {}",
                                    valid_sign.join(", ")
                                ));
                                all_ok = false;
                            }
                        }
                    }
                    "v8:StringQualifiers" => {
                        let length = unica_xml_format::read::child_element_in_namespace(
                            child, "Length", NS_V8,
                        )
                        .map(dcs_text_of);
                        let allowed_length = unica_xml_format::read::child_element_in_namespace(
                            child,
                            "AllowedLength",
                            NS_V8,
                        )
                        .map(dcs_text_of);
                        if length
                            .as_deref()
                            .filter(|value| {
                                !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit())
                            })
                            .is_none()
                        {
                            report.error(
                                "v8:StringQualifiers: <v8:Length> missing or not a non-negative integer",
                            );
                            all_ok = false;
                        }
                        if let Some(allowed_length) =
                            allowed_length.as_deref().filter(|value| !value.is_empty())
                        {
                            if !valid_length.contains(&allowed_length) {
                                report.error(format!(
                                    "v8:StringQualifiers: <v8:AllowedLength>{allowed_length}</v8:AllowedLength> — must be one of: {}",
                                    valid_length.join(", ")
                                ));
                                all_ok = false;
                            }
                        }
                    }
                    "v8:DateQualifiers" => {
                        let fractions = unica_xml_format::read::child_element_in_namespace(
                            child,
                            "DateFractions",
                            NS_V8,
                        )
                        .map(dcs_text_of);
                        if let Some(fractions) =
                            fractions.as_deref().filter(|value| !value.is_empty())
                        {
                            if !valid_fractions.contains(&fractions) {
                                report.error(format!(
                                    "v8:DateQualifiers: <v8:DateFractions>{fractions}</v8:DateFractions> — must be one of: {}",
                                    valid_fractions.join(", ")
                                ));
                                all_ok = false;
                            }
                        }
                    }
                    _ => {}
                }
                if report.stopped {
                    return;
                }
            }
        }

        for qualifier in qualifiers {
            let producer = match qualifier.as_str() {
                "v8:NumberQualifiers" => Some("xs:decimal"),
                "v8:StringQualifiers" => Some("xs:string"),
                "v8:DateQualifiers" => Some("xs:dateTime"),
                _ => None,
            };
            if let Some(producer) = producer {
                if !types.contains(producer) {
                    report.error(format!(
                        "valueType: <{qualifier}> has no matching <v8:Type>{producer}</v8:Type> in this valueType"
                    ));
                    all_ok = false;
                    if report.stopped {
                        return;
                    }
                }
            }
        }
    }

    if all_ok {
        report.ok(format!(
            "{} valueType block(s): structure and qualifiers OK",
            value_types.len()
        ));
    }
}

pub fn dcs_validate_value_contents(
    report: &mut DcsValidationReporter,
    root: roxmltree::Node<'_, '_>,
) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    const NS_CORE: &str = "http://v8.1c.ru/8.1/data-composition-system/core";
    let value_nodes = root
        .descendants()
        .filter(|node| {
            (unica_xml_format::names::element_has_name(*node, "value", Some(NS_SCHEMA))
                || unica_xml_format::names::element_has_name(*node, "value", Some(NS_CORE)))
                && unica_xml_format::read::attribute_by_local_name(*node, "type").is_some()
        })
        .collect::<Vec<_>>();

    let mut checked = 0usize;
    let mut ok = true;
    for value_node in value_nodes {
        checked += 1;
        let xsi_type =
            unica_xml_format::read::attribute_by_local_name(value_node, "type").unwrap_or("");
        let text = value_node.text().unwrap_or("");
        if xsi_type == "dcscor:DesignTimeValue" {
            let stripped = text.trim();
            if stripped.is_empty() || stripped == "_" {
                report.error(format!(
                    "<value xsi:type=\"dcscor:DesignTimeValue\">{text}</value> — DesignTimeValue must be a reference path (e.g. Перечисление.X.Y), not '{text}'"
                ));
                ok = false;
                if report.stopped {
                    return;
                }
            } else if !dcs_validate_design_time_value_ref_shape(stripped) {
                report.warn(format!(
                    "<value xsi:type=\"dcscor:DesignTimeValue\">{text}</value> — doesn't look like a typical ref path"
                ));
            }
        }
    }

    if checked > 0 && ok {
        report.ok(format!(
            "{checked} <value> element(s) with xsi:type: content OK"
        ));
    }
}

pub fn dcs_validate_design_time_value_ref_shape(value: &str) -> bool {
    let Some((prefix, rest)) = value.split_once('.') else {
        return false;
    };
    !prefix.is_empty()
        && prefix
            .chars()
            .all(|ch| ch.is_ascii_alphabetic() || matches!(ch, 'А'..='Я' | 'а'..='я' | 'Ё' | 'ё'))
        && rest.chars().next().is_some_and(|ch| {
            ch.is_ascii_alphabetic()
                || ch.is_ascii_digit()
                || ch == '_'
                || matches!(ch, 'А'..='Я' | 'а'..='я' | 'Ё' | 'ё')
        })
}

pub fn dcs_validate_config_ref_type_shape(local_type: &str) -> bool {
    let Some((kind, name)) = local_type.split_once('.') else {
        return false;
    };
    !kind.is_empty() && !name.is_empty() && kind.chars().all(|ch| ch.is_ascii_alphabetic())
}

pub fn dcs_validate_system_type_shape(local_type: &str) -> bool {
    let mut chars = local_type.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic() && chars.all(|ch| ch.is_ascii_alphanumeric())
}

pub fn dcs_validate_data_sets(
    report: &mut DcsValidationReporter,
    data_set_nodes: &[roxmltree::Node<'_, '_>],
    data_source_names: &HashSet<String>,
) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    let valid_ds_types = ["DataSetQuery", "DataSetObject", "DataSetUnion"];
    if data_set_nodes.is_empty() {
        report.warn("No dataSet elements found (settings-only DCS?)");
        return;
    }
    let mut names_seen = HashSet::<String>::new();
    let mut ds_ok = true;
    for ds in data_set_nodes {
        let xsi_type = unica_xml_format::read::attribute_by_local_name(*ds, "type").unwrap_or("");
        let name_node = unica_xml_format::read::child_element_in_namespace(*ds, "name", NS_SCHEMA);
        let ds_name = name_node
            .map(dcs_inner_text)
            .unwrap_or_else(|| "(unnamed)".to_string());
        if name_node.is_none() || ds_name.is_empty() {
            report.error("DataSet has empty name");
            ds_ok = false;
        } else if !names_seen.insert(ds_name.clone()) {
            report.error(format!("Duplicate dataSet name: {ds_name}"));
            ds_ok = false;
        }
        if xsi_type.is_empty() {
            report.error(format!("DataSet '{ds_name}' missing xsi:type"));
            ds_ok = false;
        } else if !valid_ds_types.contains(&xsi_type) {
            report.warn(format!(
                "DataSet '{ds_name}' has unusual xsi:type: {xsi_type}"
            ));
        }
        if xsi_type != "DataSetUnion" {
            if let Some(src_node) =
                unica_xml_format::read::child_element_in_namespace(*ds, "dataSource", NS_SCHEMA)
            {
                let source = dcs_inner_text(src_node);
                if !source.is_empty() && !data_source_names.contains(&source) {
                    report.error(format!(
                        "DataSet '{ds_name}' references unknown dataSource: {source}"
                    ));
                    ds_ok = false;
                }
            }
        }
        if xsi_type == "DataSetQuery" {
            let query_node =
                unica_xml_format::read::child_element_in_namespace(*ds, "query", NS_SCHEMA);
            if query_node.map(dcs_text_of).unwrap_or_default().is_empty() {
                report.warn(format!("DataSet '{ds_name}' (Query) has empty query"));
            }
        }
        if xsi_type == "DataSetObject" {
            let obj_node =
                unica_xml_format::read::child_element_in_namespace(*ds, "objectName", NS_SCHEMA);
            if obj_node.map(dcs_text_of).unwrap_or_default().is_empty() {
                report.error(format!("DataSet '{ds_name}' (Object) has empty objectName"));
                ds_ok = false;
            }
        }
    }
    if ds_ok {
        report.ok(format!(
            "{} dataSet(s) found, names unique",
            data_set_nodes.len()
        ));
    }
}

pub fn dcs_validate_data_set_fields(
    report: &mut DcsValidationReporter,
    ds_node: roxmltree::Node<'_, '_>,
    ds_name: &str,
) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    let fields = unica_xml_format::read::child_elements_in_namespace(ds_node, "field", NS_SCHEMA);
    if fields.is_empty() {
        return;
    }
    let mut paths_seen = HashSet::<String>::new();
    let mut field_ok = true;
    for field in &fields {
        let dp = unica_xml_format::read::child_element_in_namespace(*field, "dataPath", NS_SCHEMA);
        let field_ref =
            unica_xml_format::read::child_element_in_namespace(*field, "field", NS_SCHEMA);
        let path = dp.map(dcs_inner_text).unwrap_or_default();
        if path.is_empty() {
            report.error(format!("DataSet '{ds_name}': field has empty dataPath"));
            field_ok = false;
            continue;
        }
        if !paths_seen.insert(path.clone()) {
            report.warn(format!("DataSet '{ds_name}': duplicate dataPath '{path}'"));
        }
        if field_ref.map(dcs_inner_text).unwrap_or_default().is_empty() {
            report.warn(format!(
                "DataSet '{ds_name}': field '{path}' has empty <field> element"
            ));
        }
    }
    if field_ok {
        report.ok(format!(
            "DataSet \"{ds_name}\": {} fields, dataPath unique",
            fields.len()
        ));
    }
    for item in unica_xml_format::read::child_elements_in_namespace(ds_node, "item", NS_SCHEMA) {
        let item_name = unica_xml_format::read::child_element_in_namespace(item, "name", NS_SCHEMA)
            .map(dcs_inner_text)
            .unwrap_or_else(|| "(unnamed item)".to_string());
        dcs_validate_data_set_fields(report, item, &item_name);
    }
}

pub fn dcs_validate_data_set_links(
    report: &mut DcsValidationReporter,
    root: roxmltree::Node<'_, '_>,
    data_set_names: &HashSet<String>,
) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    let link_nodes =
        unica_xml_format::read::child_elements_in_namespace(root, "dataSetLink", NS_SCHEMA);
    if link_nodes.is_empty() {
        return;
    }
    let mut link_ok = true;
    for link in &link_nodes {
        let src =
            unica_xml_format::read::child_element_in_namespace(*link, "sourceDataSet", NS_SCHEMA);
        let dst = unica_xml_format::read::child_element_in_namespace(
            *link,
            "destinationDataSet",
            NS_SCHEMA,
        );
        let src_expr = unica_xml_format::read::child_element_in_namespace(
            *link,
            "sourceExpression",
            NS_SCHEMA,
        );
        let dst_expr = unica_xml_format::read::child_element_in_namespace(
            *link,
            "destinationExpression",
            NS_SCHEMA,
        );
        let src_text = src.map(dcs_inner_text).unwrap_or_default();
        if !src_text.is_empty() && !data_set_names.contains(&src_text) {
            report.error(format!("DataSetLink: sourceDataSet '{src_text}' not found"));
            link_ok = false;
        }
        let dst_text = dst.map(dcs_inner_text).unwrap_or_default();
        if !dst_text.is_empty() && !data_set_names.contains(&dst_text) {
            report.error(format!(
                "DataSetLink: destinationDataSet '{dst_text}' not found"
            ));
            link_ok = false;
        }
        if src_expr.map(dcs_text_of).unwrap_or_default().is_empty() {
            report.error("DataSetLink: empty sourceExpression");
            link_ok = false;
        }
        if dst_expr.map(dcs_text_of).unwrap_or_default().is_empty() {
            report.error("DataSetLink: empty destinationExpression");
            link_ok = false;
        }
    }
    if link_ok {
        report.ok(format!(
            "{} dataSetLink(s): references valid",
            link_nodes.len()
        ));
    }
}

pub fn dcs_validate_calculated_fields(
    report: &mut DcsValidationReporter,
    calc_field_nodes: &[roxmltree::Node<'_, '_>],
    all_field_paths: &HashMap<String, String>,
) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    if calc_field_nodes.is_empty() {
        return;
    }
    let mut cf_ok = true;
    let mut cf_seen = HashSet::<String>::new();
    for calc in calc_field_nodes {
        let dp = unica_xml_format::read::child_element_in_namespace(*calc, "dataPath", NS_SCHEMA);
        let expr =
            unica_xml_format::read::child_element_in_namespace(*calc, "expression", NS_SCHEMA);
        let path = dp.map(dcs_inner_text).unwrap_or_default();
        if path.is_empty() {
            report.error("CalculatedField has empty dataPath");
            cf_ok = false;
            continue;
        }
        if !cf_seen.insert(path.clone()) {
            report.error(format!("Duplicate calculatedField dataPath: {path}"));
            cf_ok = false;
        }
        if expr.map(dcs_text_of).unwrap_or_default().is_empty() {
            report.error(format!("CalculatedField '{path}' has empty expression"));
            cf_ok = false;
        }
        if let Some(ds_name) = all_field_paths.get(&path) {
            report.warn(format!(
                "CalculatedField '{path}' shadows dataSet field in '{ds_name}'"
            ));
        }
    }
    if cf_ok {
        report.ok(format!(
            "{} calculatedField(s): dataPath and expression valid",
            calc_field_nodes.len()
        ));
    }
}

pub fn dcs_validate_total_fields(
    report: &mut DcsValidationReporter,
    total_field_nodes: &[roxmltree::Node<'_, '_>],
) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    if total_field_nodes.is_empty() {
        return;
    }
    let mut tf_ok = true;
    for total in total_field_nodes {
        let dp = unica_xml_format::read::child_element_in_namespace(*total, "dataPath", NS_SCHEMA);
        let expr =
            unica_xml_format::read::child_element_in_namespace(*total, "expression", NS_SCHEMA);
        let path = dp.map(dcs_inner_text).unwrap_or_default();
        if path.is_empty() {
            report.error("TotalField has empty dataPath");
            tf_ok = false;
            continue;
        }
        if expr.map(dcs_text_of).unwrap_or_default().is_empty() {
            report.error(format!("TotalField '{path}' has empty expression"));
            tf_ok = false;
        }
    }
    if tf_ok {
        report.ok(format!(
            "{} totalField(s): dataPath and expression present",
            total_field_nodes.len()
        ));
    }
}

pub fn dcs_validate_parameters(
    report: &mut DcsValidationReporter,
    param_nodes: &[roxmltree::Node<'_, '_>],
) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    if param_nodes.is_empty() {
        return;
    }
    let mut param_ok = true;
    let mut param_seen = HashSet::<String>::new();
    for param in param_nodes {
        let name = unica_xml_format::read::child_element_in_namespace(*param, "name", NS_SCHEMA)
            .map(dcs_inner_text)
            .unwrap_or_default();
        if name.is_empty() {
            report.error("Parameter has empty name");
            param_ok = false;
            continue;
        }
        if !param_seen.insert(name.clone()) {
            report.error(format!("Duplicate parameter name: {name}"));
            param_ok = false;
        }
    }
    if param_ok {
        report.ok(format!("{} parameter(s): names unique", param_nodes.len()));
    }
}

pub fn dcs_validate_templates(
    report: &mut DcsValidationReporter,
    template_nodes: &[roxmltree::Node<'_, '_>],
) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    if template_nodes.is_empty() {
        return;
    }
    let mut tpl_ok = true;
    let mut tpl_seen = HashSet::<String>::new();
    for template in template_nodes {
        let name = unica_xml_format::read::child_element_in_namespace(*template, "name", NS_SCHEMA)
            .map(dcs_inner_text)
            .unwrap_or_default();
        if name.is_empty() {
            report.error("Template has empty name");
            tpl_ok = false;
            continue;
        }
        if !tpl_seen.insert(name.clone()) {
            report.error(format!("Duplicate template name: {name}"));
            tpl_ok = false;
        }
    }
    if tpl_ok {
        report.ok(format!(
            "{} template(s): names unique",
            template_nodes.len()
        ));
    }
}

pub fn dcs_validate_group_templates(
    report: &mut DcsValidationReporter,
    group_template_nodes: &[roxmltree::Node<'_, '_>],
    template_names: &HashSet<String>,
) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    if group_template_nodes.is_empty() {
        return;
    }
    let valid_tpl_types = [
        "Header",
        "Footer",
        "Overall",
        "OverallHeader",
        "OverallFooter",
    ];
    let mut gt_ok = true;
    for group_template in group_template_nodes {
        let tpl_ref = unica_xml_format::read::child_element_in_namespace(
            *group_template,
            "template",
            NS_SCHEMA,
        )
        .map(dcs_inner_text)
        .unwrap_or_default();
        let tpl_type = unica_xml_format::read::child_element_in_namespace(
            *group_template,
            "templateType",
            NS_SCHEMA,
        )
        .map(dcs_inner_text)
        .unwrap_or_default();
        if !tpl_ref.is_empty() && !template_names.contains(&tpl_ref) {
            report.error(format!(
                "GroupTemplate references unknown template: {tpl_ref}"
            ));
            gt_ok = false;
        }
        if !tpl_type.is_empty() && !valid_tpl_types.contains(&tpl_type.as_str()) {
            report.warn(format!(
                "GroupTemplate has unusual templateType: {tpl_type}"
            ));
        }
    }
    if gt_ok {
        report.ok(format!(
            "{} groupTemplate(s): references valid",
            group_template_nodes.len()
        ));
    }
}

pub fn dcs_validate_settings_variants(
    report: &mut DcsValidationReporter,
    variant_nodes: &[roxmltree::Node<'_, '_>],
    known_fields: &HashSet<String>,
) {
    const NS_SETTINGS: &str = "http://v8.1c.ru/8.1/data-composition-system/settings";
    if variant_nodes.is_empty() {
        report.warn("No settingsVariant elements found");
        return;
    }
    let mut v_ok = true;
    for (idx, variant) in variant_nodes.iter().enumerate() {
        let v_name =
            unica_xml_format::read::child_element_in_namespace(*variant, "name", NS_SETTINGS);
        let variant_name = v_name.map(dcs_inner_text).unwrap_or_default();
        if variant_name.is_empty() {
            report.error(format!("SettingsVariant #{} has empty name", idx + 1));
            v_ok = false;
        }
        let settings =
            unica_xml_format::read::child_element_in_namespace(*variant, "settings", NS_SETTINGS);
        let Some(settings) = settings else {
            report.error(format!(
                "SettingsVariant '{variant_name}' has no settings element"
            ));
            v_ok = false;
            continue;
        };
        dcs_check_settings(report, settings, &variant_name, known_fields);
    }
    if v_ok {
        report.ok(format!("{} settingsVariant(s) found", variant_nodes.len()));
    }
}

pub fn dcs_check_settings(
    report: &mut DcsValidationReporter,
    settings_node: roxmltree::Node<'_, '_>,
    variant_name: &str,
    known_fields: &HashSet<String>,
) {
    const NS_SETTINGS: &str = "http://v8.1c.ru/8.1/data-composition-system/settings";
    if report.stopped {
        return;
    }
    for selected_item in dcs_find_all_path(
        settings_node,
        &[("selection", NS_SETTINGS), ("item", NS_SETTINGS)],
    ) {
        let xsi_type =
            unica_xml_format::read::attribute_by_local_name(selected_item, "type").unwrap_or("");
        if xsi_type == "dcsset:SelectedItemField" {
            let field = unica_xml_format::read::child_element_in_namespace(
                selected_item,
                "field",
                NS_SETTINGS,
            )
            .map(dcs_inner_text)
            .unwrap_or_default();
            if !field.is_empty() && field != "SystemFields.Number" {
                let base_path = field.split('.').next().unwrap_or("");
                if !known_fields.contains(&field) && !known_fields.contains(base_path) {
                    // Soft check in the reference script: autoFillFields may add implicit fields.
                }
            }
        }
    }
    // A `SettingsParameterValue` without a name is a well-formed element that
    // names nothing, so the platform reads the variant as having no value for
    // that parameter. Validation used to pass it (#311).
    for parameter_item in dcs_find_all_path(
        settings_node,
        &[("dataParameters", NS_SETTINGS), ("item", DCS_CORE_NS)],
    ) {
        let parameter = unica_xml_format::read::child_element_in_namespace(
            parameter_item,
            "parameter",
            DCS_CORE_NS,
        )
        .map(dcs_inner_text)
        .unwrap_or_default();
        if parameter.trim().is_empty() {
            report.error(format!(
                "Variant '{variant_name}' dataParameters: SettingsParameterValue has empty parameter name"
            ));
        }
    }
    dcs_check_filter_items(report, settings_node, variant_name);
    for order_item in dcs_find_all_path(
        settings_node,
        &[("order", NS_SETTINGS), ("item", NS_SETTINGS)],
    ) {
        let xsi_type =
            unica_xml_format::read::attribute_by_local_name(order_item, "type").unwrap_or("");
        if xsi_type == "dcsset:OrderItemField" {
            let order_type = unica_xml_format::read::child_element_in_namespace(
                order_item,
                "orderType",
                NS_SETTINGS,
            )
            .map(dcs_inner_text)
            .unwrap_or_default();
            if !order_type.is_empty() && !matches!(order_type.as_str(), "Asc" | "Desc") {
                report.warn(format!(
                    "Variant '{variant_name}' order: invalid orderType '{order_type}'"
                ));
            }
        }
    }
    for structure_item in
        unica_xml_format::read::child_elements_in_namespace(settings_node, "item", NS_SETTINGS)
    {
        dcs_check_structure_item(report, structure_item, variant_name);
    }
}

pub fn dcs_check_filter_items(
    report: &mut DcsValidationReporter,
    parent_node: roxmltree::Node<'_, '_>,
    variant_name: &str,
) {
    const NS_SETTINGS: &str = "http://v8.1c.ru/8.1/data-composition-system/settings";
    let valid_comparison_types = [
        "Equal",
        "NotEqual",
        "Greater",
        "GreaterOrEqual",
        "Less",
        "LessOrEqual",
        "InList",
        "NotInList",
        "InHierarchy",
        "InListByHierarchy",
        "Contains",
        "NotContains",
        "BeginsWith",
        "NotBeginsWith",
        "Filled",
        "NotFilled",
    ];
    for filter_item in dcs_find_all_path(
        parent_node,
        &[("filter", NS_SETTINGS), ("item", NS_SETTINGS)],
    ) {
        if report.stopped {
            return;
        }
        let xsi_type =
            unica_xml_format::read::attribute_by_local_name(filter_item, "type").unwrap_or("");
        if xsi_type == "dcsset:FilterItemComparison" {
            let comp_type = unica_xml_format::read::child_element_in_namespace(
                filter_item,
                "comparisonType",
                NS_SETTINGS,
            )
            .map(dcs_inner_text)
            .unwrap_or_default();
            if !comp_type.is_empty() && !valid_comparison_types.contains(&comp_type.as_str()) {
                report.error(format!(
                    "Variant '{variant_name}' filter: invalid comparisonType '{comp_type}'"
                ));
            }
        } else if xsi_type == "dcsset:FilterItemGroup" {
            let group_type = unica_xml_format::read::child_element_in_namespace(
                filter_item,
                "groupType",
                NS_SETTINGS,
            )
            .map(dcs_inner_text)
            .unwrap_or_default();
            if !group_type.is_empty()
                && !matches!(group_type.as_str(), "AndGroup" | "OrGroup" | "NotGroup")
            {
                report.warn(format!(
                    "Variant '{variant_name}' filter group: unusual groupType '{group_type}'"
                ));
            }
            for nested in unica_xml_format::read::child_elements_in_namespace(
                filter_item,
                "item",
                NS_SETTINGS,
            ) {
                let nested_type =
                    unica_xml_format::read::attribute_by_local_name(nested, "type").unwrap_or("");
                if nested_type == "dcsset:FilterItemComparison" {
                    let comp_type = unica_xml_format::read::child_element_in_namespace(
                        nested,
                        "comparisonType",
                        NS_SETTINGS,
                    )
                    .map(dcs_inner_text)
                    .unwrap_or_default();
                    if !comp_type.is_empty()
                        && !valid_comparison_types.contains(&comp_type.as_str())
                    {
                        report.error(format!(
                            "Variant '{variant_name}' filter: invalid comparisonType '{comp_type}'"
                        ));
                    }
                }
            }
        }
    }
}

pub fn dcs_check_structure_item(
    report: &mut DcsValidationReporter,
    item_node: roxmltree::Node<'_, '_>,
    variant_name: &str,
) {
    const NS_SETTINGS: &str = "http://v8.1c.ru/8.1/data-composition-system/settings";
    if report.stopped {
        return;
    }
    let valid_structure_types = [
        "dcsset:StructureItemGroup",
        "dcsset:StructureItemTable",
        "dcsset:StructureItemChart",
        "dcsset:StructureItemNestedObject",
    ];
    let xsi_type = unica_xml_format::read::attribute_by_local_name(item_node, "type").unwrap_or("");
    if xsi_type.is_empty() {
        report.error(format!(
            "Variant '{variant_name}': structure item missing xsi:type"
        ));
        return;
    }
    if !valid_structure_types.contains(&xsi_type) {
        report.warn(format!(
            "Variant '{variant_name}': unusual structure item type '{xsi_type}'"
        ));
    }
    for nested in
        unica_xml_format::read::child_elements_in_namespace(item_node, "item", NS_SETTINGS)
    {
        dcs_check_structure_item(report, nested, variant_name);
    }
    if xsi_type == "dcsset:StructureItemTable" {
        let columns =
            unica_xml_format::read::child_elements_in_namespace(item_node, "column", NS_SETTINGS);
        let rows =
            unica_xml_format::read::child_elements_in_namespace(item_node, "row", NS_SETTINGS);
        if columns.is_empty() {
            report.warn(format!("Variant '{variant_name}': table has no columns"));
        }
        if rows.is_empty() {
            report.warn(format!("Variant '{variant_name}': table has no rows"));
        }
    }
}

pub fn dcs_collect_data_set_fields(
    ds_node: roxmltree::Node<'_, '_>,
    ds_name: &str,
    all_field_paths: &mut HashMap<String, String>,
) -> HashSet<String> {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    let mut local_paths = HashSet::<String>::new();
    for field in unica_xml_format::read::child_elements_in_namespace(ds_node, "field", NS_SCHEMA) {
        if let Some(dp) =
            unica_xml_format::read::child_element_in_namespace(field, "dataPath", NS_SCHEMA)
        {
            let path = dcs_inner_text(dp);
            local_paths.insert(path.clone());
            all_field_paths.insert(path, ds_name.to_string());
        }
    }
    for item in unica_xml_format::read::child_elements_in_namespace(ds_node, "item", NS_SCHEMA) {
        if let Some(item_name) =
            unica_xml_format::read::child_element_in_namespace(item, "name", NS_SCHEMA)
        {
            dcs_collect_data_set_fields(item, &dcs_inner_text(item_name), all_field_paths);
        }
    }
    local_paths
}

pub fn dcs_find_all_path<'a, 'input>(
    parent: roxmltree::Node<'a, 'input>,
    path: &[(&str, &str)],
) -> Vec<roxmltree::Node<'a, 'input>> {
    let mut current = vec![parent];
    for (local_name, namespace) in path {
        let mut next = Vec::<roxmltree::Node<'a, 'input>>::new();
        for node in current {
            next.extend(unica_xml_format::read::child_elements_in_namespace(
                node, local_name, namespace,
            ));
        }
        current = next;
    }
    current
}

pub fn dcs_inner_text(node: roxmltree::Node<'_, '_>) -> String {
    node.children()
        .filter(|child| child.is_text())
        .filter_map(|child| child.text())
        .collect()
}

pub fn dcs_text_of(node: roxmltree::Node<'_, '_>) -> String {
    node.text().unwrap_or("").trim().to_string()
}

pub fn dcs_all_text(node: roxmltree::Node<'_, '_>) -> String {
    node.descendants()
        .filter(|child| child.is_text())
        .filter_map(|child| child.text())
        .collect::<String>()
        .trim()
        .to_string()
}

pub struct DcsValidationReporter {
    pub errors: usize,
    pub warnings: usize,
    pub ok_count: usize,
    pub stopped: bool,
    pub max_errors: usize,
    pub detailed: bool,
    pub lines: Vec<String>,
}

pub struct DcsValidationRun {
    pub ok: bool,
    pub stdout: String,
    pub artifact: PathBuf,
    pub errors: Vec<String>,
}

/// Typed answer of `unica.dcs.info` (ADR-0023). Eleven `Mode` values each
/// rendered its own report; the data carries every section at once and a caller
/// projects what it needs, so `Limit` and `Offset` go away with the line layout.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoData {
    pub support: DomainObjectSupportData,
    pub data_sources: Vec<DcsInfoDataSource>,
    pub data_sets: Vec<DcsInfoDataSet>,
    pub links: Vec<DcsInfoLink>,
    pub calculated_fields: Vec<DcsInfoCalculatedField>,
    pub total_fields: Vec<DcsInfoTotalField>,
    pub parameters: Vec<DcsInfoParameter>,
    pub variants: Vec<DcsInfoVariant>,
    pub templates: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoDataSource {
    pub name: String,
    /// `dataSourceType`, for example `Local`; `null` when the source omits it.
    pub kind: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoDataSet {
    pub name: String,
    /// `Query`, `Object`, `Union` or the raw `xsi:type` when it is none of them.
    pub kind: String,
    /// The dataset query; `null` for kinds that carry none.
    pub query: Option<String>,
    /// The platform object an `Object` dataset reads; `null` for other kinds.
    pub object_name: Option<String>,
    /// The `dataSource` this dataset reads through; `null` when it names none.
    pub data_source: Option<String>,
    pub fields: Vec<DcsInfoField>,
    /// Nested datasets of a union; empty for every other kind.
    pub items: Vec<DcsInfoDataSet>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoField {
    pub data_path: String,
    /// The query column behind the field; `null` when the field declares none.
    pub field: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoLink {
    pub source: String,
    pub destination: String,
    pub source_expression: Option<String>,
    pub destination_expression: Option<String>,
    pub parameter: Option<String>,
    pub condition: Option<String>,
    pub start_expression: Option<String>,
    pub parameter_list_allowed: Option<bool>,
    pub required: Option<bool>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoCalculatedField {
    pub data_path: String,
    pub expression: Option<String>,
    pub title: Option<String>,
    /// Which uses `useRestriction` bars, in schema order: `field`, `condition`,
    /// `group`, `order`. On a field the element is structured, not a flag, so a
    /// boolean cannot carry the answer the retired `calculated` report printed.
    /// `null` when the field declares no restriction at all.
    pub restrictions: Option<Vec<String>>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoTotalField {
    pub data_path: String,
    pub expression: Option<String>,
    /// The grouping this total belongs to; `null` is the overall total, and
    /// without it a per-group total is indistinguishable from an overall one.
    pub group: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoParameter {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: Option<String>,
    pub value: Option<String>,
    pub expression: Option<String>,
    /// True when `useRestriction` hides the parameter from the user.
    pub restricted: bool,
    /// `availableAsField`; `null` when the parameter declares none. `false`
    /// means the parameter cannot be used as a field, which the retired
    /// `params` report printed as `[noField]`.
    pub available_as_field: Option<bool>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// The platform writes a variant's own elements in the settings namespace, not
/// the schema one. Reading them with the schema namespace answered every
/// variant as an empty shell, so this section is bound to `dcsset` throughout.
pub struct DcsInfoVariant {
    pub name: String,
    pub presentation: Option<String>,
    pub selection: Vec<String>,
    pub order: Vec<DcsInfoOrderItem>,
    /// How many filter items the variant declares.
    pub filters: usize,
    pub structure: Vec<DcsInfoStructureItem>,
    pub structure_items: Vec<DcsInfoStructureRow>,
    pub setting_items: Vec<serde_json::Value>,
}

/// XML facts in document order. Indices identify rows within this variant,
/// not logical addresses: anonymous and duplicate names remain observable.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoStructureRow {
    pub index: usize,
    pub parent_index: Option<usize>,
    pub axis: Option<String>,
    pub kind: String,
    pub name: Option<String>,
    pub group_by: Vec<String>,
    #[serde(flatten)]
    pub properties: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoOrderItem {
    pub field: String,
    /// `Asc` or `Desc`; `null` when the item declares no direction.
    pub direction: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoStructureItem {
    /// `Group`, `Table`, `Chart` or `Unknown`.
    pub kind: String,
    /// Fields the item groups by; empty for a detail item.
    pub group_by: Vec<String>,
}

/// Inert read-path inspection facts composed by the existing source adapter.
pub struct DcsInfoPathInspection {
    pub resolution: Result<PathBuf, String>,
    pub dependencies: Vec<PathBuf>,
}

pub const DCS_SCHEMA_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";

pub const DCS_SETTINGS_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/settings";

pub const DCS_CORE_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/core";

pub const DCS_COMMON_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/common";

pub const V8_DATA_NS: &str = "http://v8.1c.ru/8.1/data/core";

pub const XML_SCHEMA_INSTANCE_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

impl DcsValidationReporter {
    pub fn new(max_errors: usize, detailed: bool, file_name: &str) -> Self {
        Self {
            errors: 0,
            warnings: 0,
            ok_count: 0,
            stopped: false,
            max_errors,
            detailed,
            lines: vec![format!("=== Validation: {file_name} ==="), String::new()],
        }
    }

    pub fn ok(&mut self, message: impl Into<String>) {
        self.ok_count += 1;
        if self.detailed {
            self.lines.push(format!("[OK]    {}", message.into()));
        }
    }

    pub fn error(&mut self, message: impl Into<String>) {
        self.errors += 1;
        self.lines.push(format!("[ERROR] {}", message.into()));
        if self.errors >= self.max_errors {
            self.stopped = true;
        }
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings += 1;
        self.lines.push(format!("[WARN]  {}", message.into()));
    }

    pub fn finalize(mut self, file_name: &str) -> (bool, String, Vec<String>) {
        let checks = self.ok_count + self.errors + self.warnings;
        let ok = self.errors == 0;
        if ok && self.warnings == 0 && !self.detailed {
            return (
                true,
                format!("=== Validation OK: {file_name} ({checks} checks) ===\n"),
                Vec::new(),
            );
        }
        self.lines.push(String::new());
        self.lines.push(format!(
            "=== Result: {} errors, {} warnings ({checks} checks) ===",
            self.errors, self.warnings
        ));
        let errors = self
            .lines
            .iter()
            .filter(|line| line.starts_with("[ERROR] "))
            .cloned()
            .collect::<Vec<_>>();
        (ok, format!("{}\n", self.lines.join("\n")), errors)
    }
}
