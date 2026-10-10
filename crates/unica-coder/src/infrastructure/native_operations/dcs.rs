#![allow(dead_code, unused_imports)]

use crate::application::operation_descriptors::TEMPLATE_PATH;
use crate::application::AdapterOutcome;
use crate::domain::address::QualifiedAddress;
use crate::domain::support_state::{
    ObjectSupportData as DomainObjectSupportData, SupportStateReader,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::logical_selector::{
    logical_selection, physical_selection, typed_reader_metadata_target, AttachedResource,
    ResolvedReadTarget,
};
use crate::infrastructure::native_operations::mxl::TEMPLATE_KINDS;
use crate::infrastructure::platform_xml_owner::DCS_ROOT;
use crate::infrastructure::support_state::WorkspaceSupportStateReader;
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::common::*;
use super::dcs_xml::*;
use super::{
    cf::*, cfe::*, form::*, interface::*, meta::*, mxl::*, role::*, subsystem::*, template::*,
};

pub(crate) const DCS_SCHEMA_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
pub(crate) const DCS_SETTINGS_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/settings";
pub(crate) const DCS_CORE_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/core";
pub(crate) const DCS_COMMON_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/common";
pub(crate) const V8_DATA_NS: &str = "http://v8.1c.ru/8.1/data/core";
pub(crate) const XML_SCHEMA_INSTANCE_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

pub(crate) fn typed_dcs_reader_target(
    address: &QualifiedAddress,
) -> Option<crate::domain::source_target::MetadataAddress> {
    typed_reader_metadata_target(address, TEMPLATE_KINDS)
}

pub(crate) fn require_dcs_root(root: roxmltree::Node<'_, '_>) -> Result<(), String> {
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

pub(crate) struct DcsValidationReporter {
    pub(crate) errors: usize,
    pub(crate) warnings: usize,
    pub(crate) ok_count: usize,
    pub(crate) stopped: bool,
    pub(crate) max_errors: usize,
    pub(crate) detailed: bool,
    pub(crate) lines: Vec<String>,
}

pub(crate) struct DcsValidationRun {
    pub(crate) ok: bool,
    pub(crate) stdout: String,
    pub(crate) artifact: PathBuf,
    pub(crate) errors: Vec<String>,
}

impl DcsValidationReporter {
    pub(crate) fn new(max_errors: usize, detailed: bool, file_name: &str) -> Self {
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

    pub(crate) fn ok(&mut self, message: impl Into<String>) {
        self.ok_count += 1;
        if self.detailed {
            self.lines.push(format!("[OK]    {}", message.into()));
        }
    }

    pub(crate) fn error(&mut self, message: impl Into<String>) {
        self.errors += 1;
        self.lines.push(format!("[ERROR] {}", message.into()));
        if self.errors >= self.max_errors {
            self.stopped = true;
        }
    }

    pub(crate) fn warn(&mut self, message: impl Into<String>) {
        self.warnings += 1;
        self.lines.push(format!("[WARN]  {}", message.into()));
    }

    pub(crate) fn finalize(mut self, file_name: &str) -> (bool, String, Vec<String>) {
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

/// Typed answer of `unica.dcs.info` (ADR-0023). Eleven `Mode` values each
/// rendered its own report; the data carries every section at once and a caller
/// projects what it needs, so `Limit` and `Offset` go away with the line layout.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DcsInfoData {
    pub(crate) support: DomainObjectSupportData,
    pub(crate) data_sources: Vec<DcsInfoDataSource>,
    pub(crate) data_sets: Vec<DcsInfoDataSet>,
    pub(crate) links: Vec<DcsInfoLink>,
    pub(crate) calculated_fields: Vec<DcsInfoCalculatedField>,
    pub(crate) total_fields: Vec<DcsInfoTotalField>,
    pub(crate) parameters: Vec<DcsInfoParameter>,
    pub(crate) variants: Vec<DcsInfoVariant>,
    pub(crate) templates: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DcsInfoDataSource {
    pub(crate) name: String,
    /// `dataSourceType`, for example `Local`; `null` when the source omits it.
    pub(crate) kind: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DcsInfoDataSet {
    pub(crate) name: String,
    /// `Query`, `Object`, `Union` or the raw `xsi:type` when it is none of them.
    pub(crate) kind: String,
    /// The dataset query; `null` for kinds that carry none.
    pub(crate) query: Option<String>,
    /// The platform object an `Object` dataset reads; `null` for other kinds.
    pub(crate) object_name: Option<String>,
    /// The `dataSource` this dataset reads through; `null` when it names none.
    pub(crate) data_source: Option<String>,
    pub(crate) fields: Vec<DcsInfoField>,
    /// Nested datasets of a union; empty for every other kind.
    pub(crate) items: Vec<DcsInfoDataSet>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DcsInfoField {
    pub(crate) data_path: String,
    /// The query column behind the field; `null` when the field declares none.
    pub(crate) field: Option<String>,
    pub(crate) title: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DcsInfoLink {
    pub(crate) source: String,
    pub(crate) destination: String,
    pub(crate) source_expression: Option<String>,
    pub(crate) destination_expression: Option<String>,
    pub(crate) parameter: Option<String>,
    pub(crate) condition: Option<String>,
    pub(crate) start_expression: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DcsInfoCalculatedField {
    pub(crate) data_path: String,
    pub(crate) expression: Option<String>,
    pub(crate) title: Option<String>,
    /// Which uses `useRestriction` bars, in schema order: `field`, `condition`,
    /// `group`, `order`. On a field the element is structured, not a flag, so a
    /// boolean cannot carry the answer the retired `calculated` report printed.
    /// `null` when the field declares no restriction at all.
    pub(crate) restrictions: Option<Vec<String>>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DcsInfoTotalField {
    pub(crate) data_path: String,
    pub(crate) expression: Option<String>,
    /// The grouping this total belongs to; `null` is the overall total, and
    /// without it a per-group total is indistinguishable from an overall one.
    pub(crate) group: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DcsInfoParameter {
    pub(crate) name: String,
    #[serde(rename = "type")]
    pub(crate) type_name: Option<String>,
    pub(crate) value: Option<String>,
    pub(crate) expression: Option<String>,
    /// True when `useRestriction` hides the parameter from the user.
    pub(crate) restricted: bool,
    /// `availableAsField`; `null` when the parameter declares none. `false`
    /// means the parameter cannot be used as a field, which the retired
    /// `params` report printed as `[noField]`.
    pub(crate) available_as_field: Option<bool>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// The platform writes a variant's own elements in the settings namespace, not
/// the schema one. Reading them with the schema namespace answered every
/// variant as an empty shell, so this section is bound to `dcsset` throughout.
pub(crate) struct DcsInfoVariant {
    pub(crate) name: String,
    pub(crate) presentation: Option<String>,
    pub(crate) selection: Vec<String>,
    pub(crate) order: Vec<DcsInfoOrderItem>,
    /// How many filter items the variant declares.
    pub(crate) filters: usize,
    pub(crate) structure: Vec<DcsInfoStructureItem>,
    pub(crate) structure_items: Vec<DcsInfoStructureRow>,
    pub(crate) setting_items: Vec<serde_json::Value>,
}

/// XML facts in document order. Indices identify rows within this variant,
/// not logical addresses: anonymous and duplicate names remain observable.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DcsInfoStructureRow {
    pub(crate) index: usize,
    pub(crate) parent_index: Option<usize>,
    pub(crate) axis: Option<String>,
    pub(crate) kind: String,
    pub(crate) name: Option<String>,
    pub(crate) group_by: Vec<String>,
    #[serde(flatten)]
    pub(crate) properties: serde_json::Map<String, serde_json::Value>,
}

fn dcs_info_structure_rows(settings: roxmltree::Node<'_, '_>) -> Vec<DcsInfoStructureRow> {
    let mut rows = Vec::new();
    let mut indices = HashMap::new();
    for item in settings.descendants().filter(|node| {
        role_info_element(*node, "item", Some(DCS_SETTINGS_NS)) || dcs_info_axis_structure(*node)
    }) {
        let known_kind = [
            ("StructureItemGroup", "Group"),
            ("StructureItemTable", "Table"),
            ("StructureItemChart", "Chart"),
            ("StructureItemNestedObject", "NestedObject"),
        ]
        .into_iter()
        .find_map(|(xml_type, kind)| {
            xsi_type_matches(item, DCS_SETTINGS_NS, xml_type).then_some(kind)
        });
        let xml_type = item
            .attribute((XML_SCHEMA_INSTANCE_NS, "type"))
            .unwrap_or_default();
        let local_type = xml_type.rsplit(':').next().unwrap_or_default();
        let Some(kind) = known_kind
            .or_else(|| dcs_info_axis_structure(item).then_some("Group"))
            .or_else(|| {
                (local_type.starts_with("StructureItem")
                    && xsi_type_matches(item, DCS_SETTINGS_NS, local_type))
                .then_some(local_type)
            })
        else {
            continue;
        };
        let mut parent_index = None;
        let mut axis = ["row", "column", "point", "series"]
            .iter()
            .find(|tag| role_info_element(item, tag, Some(DCS_SETTINGS_NS)))
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
        if let Some(child) = dcs_child(node, key, DCS_SETTINGS_NS) {
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
    for (index, item) in dcs_children(container, "item", DCS_SETTINGS_NS)
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
        .any(|tag| role_info_element(node, tag, Some(DCS_SETTINGS_NS)))
        && (!node
            .children()
            .any(|child| role_info_element(child, "item", Some(DCS_SETTINGS_NS)))
            || [
                "name",
                "groupItems",
                "filter",
                "order",
                "selection",
                "conditionalAppearance",
                "outputParameters",
            ]
            .iter()
            .any(|tag| dcs_child(node, tag, DCS_SETTINGS_NS).is_some()))
}

fn dcs_info_setting_rows(settings: roxmltree::Node<'_, '_>) -> Vec<serde_json::Value> {
    let mut rows = Vec::new();
    for scope in settings.descendants().filter(|node| {
        *node == settings
            || (role_info_element(*node, "item", Some(DCS_SETTINGS_NS))
                && dcs_info_structure_item_type(*node) != "Unknown")
            || ["row", "column", "point", "series"]
                .iter()
                .any(|tag| role_info_element(*node, tag, Some(DCS_SETTINGS_NS)))
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
            if let Some(container) = dcs_child(scope, tag, DCS_SETTINGS_NS) {
                let mut items = Vec::new();
                if kind == "Selection" {
                    dcs_info_selection_rows(container, &[], &mut items);
                } else {
                    items.extend(
                        dcs_children(container, "item", ns)
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
                    if let Some(title) = dcs_child(item, "lwsTitle", DCS_SETTINGS_NS) {
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
                        let child = dcs_child(item, input, ns)
                            .or_else(|| dcs_child(item, input, DCS_SETTINGS_NS));
                        if let Some(child) = child {
                            row.insert(output.into(), serde_json::json!(dcs_text_of(child)));
                        }
                    }
                    if kind == "ConditionalAppearance" {
                        if let Some(fields) = dcs_child(item, "selection", DCS_SETTINGS_NS) {
                            row.insert(
                                "fields".into(),
                                serde_json::json!(dcs_children(fields, "item", DCS_SETTINGS_NS)
                                    .into_iter()
                                    .filter_map(|item| dcs_info_child_text(
                                        item,
                                        "field",
                                        DCS_SETTINGS_NS
                                    ))
                                    .collect::<Vec<_>>()
                                    .join(", ")),
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

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DcsInfoOrderItem {
    pub(crate) field: String,
    /// `Asc` or `Desc`; `null` when the item declares no direction.
    pub(crate) direction: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DcsInfoStructureItem {
    /// `Group`, `Table`, `Chart` or `Unknown`.
    pub(crate) kind: String,
    /// Fields the item groups by; empty for a detail item.
    pub(crate) group_by: Vec<String>,
}

pub(crate) struct DcsInfoExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<DcsInfoData>,
}

fn dcs_info_opt(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn dcs_info_child_text(
    node: roxmltree::Node<'_, '_>,
    tag: &str,
    ns_schema: &str,
) -> Option<String> {
    dcs_child(node, tag, ns_schema)
        .map(dcs_text_of)
        .and_then(dcs_info_opt)
}

/// A declared boolean, or `null` when the schema declares nothing. ADR-0023
/// keeps the two apart: an omitted flag is not a proven `false`.
fn dcs_info_flag(node: roxmltree::Node<'_, '_>, tag: &str, ns_schema: &str) -> Option<bool> {
    dcs_child(node, tag, ns_schema)
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
    dcs_child(node, tag, ns_schema).map(|restriction| {
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
            field: dcs_child(item, "field", ns_settings)
                .map(dcs_text_of)
                .and_then(dcs_info_opt)?,
            direction: dcs_child(item, "orderType", ns_settings)
                .map(dcs_text_of)
                .and_then(dcs_info_opt),
        })
    })
    .collect()
}

fn dcs_info_data_set(data_set: roxmltree::Node<'_, '_>, ns_schema: &str) -> DcsInfoDataSet {
    let kind = dcs_info_dataset_type(data_set);
    DcsInfoDataSet {
        name: dcs_child(data_set, "name", ns_schema)
            .map(dcs_text_of)
            .unwrap_or_default(),
        object_name: dcs_info_child_text(data_set, "objectName", ns_schema),
        data_source: dcs_info_child_text(data_set, "dataSource", ns_schema),
        // `dcs_inner_text`, not `dcs_all_text`: a 1C query carries meaningful
        // leading whitespace in its `|` continuation lines, and the retired
        // `Raw` mode existed precisely to hand it back untrimmed.
        query: dcs_child(data_set, "query", ns_schema)
            .map(dcs_inner_text)
            .and_then(dcs_info_opt),
        fields: dcs_children(data_set, "field", ns_schema)
            .into_iter()
            .filter_map(|field| {
                Some(DcsInfoField {
                    data_path: dcs_info_child_text(field, "dataPath", ns_schema)?,
                    field: dcs_info_child_text(field, "field", ns_schema),
                    title: dcs_child(field, "title", ns_schema)
                        .map(dcs_info_multilang_or_inner_text)
                        .and_then(dcs_info_opt),
                })
            })
            .collect(),
        items: if kind == "Union" {
            dcs_children(data_set, "item", ns_schema)
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
        data_sources: dcs_children(root, "dataSource", ns_schema)
            .into_iter()
            .map(|source| DcsInfoDataSource {
                name: dcs_child(source, "name", ns_schema)
                    .map(dcs_text_of)
                    .unwrap_or_default(),
                kind: dcs_info_child_text(source, "dataSourceType", ns_schema),
            })
            .collect(),
        data_sets: dcs_children(root, "dataSet", ns_schema)
            .into_iter()
            .map(|data_set| dcs_info_data_set(data_set, ns_schema))
            .collect(),
        links: dcs_children(root, "dataSetLink", ns_schema)
            .into_iter()
            .map(|link| DcsInfoLink {
                source: dcs_child(link, "sourceDataSet", ns_schema)
                    .map(dcs_text_of)
                    .unwrap_or_default(),
                destination: dcs_child(link, "destinationDataSet", ns_schema)
                    .map(dcs_text_of)
                    .unwrap_or_default(),
                parameter: dcs_info_child_text(link, "parameter", ns_schema),
                condition: dcs_info_child_text(link, "linkConditionExpression", ns_schema),
                start_expression: dcs_info_child_text(link, "startExpression", ns_schema),
                source_expression: dcs_info_child_text(link, "sourceExpression", ns_schema),
                destination_expression: dcs_info_child_text(
                    link,
                    "destinationExpression",
                    ns_schema,
                ),
            })
            .collect(),
        calculated_fields: dcs_children(root, "calculatedField", ns_schema)
            .into_iter()
            .filter_map(|field| {
                Some(DcsInfoCalculatedField {
                    data_path: dcs_info_child_text(field, "dataPath", ns_schema)?,
                    expression: dcs_child(field, "expression", ns_schema)
                        .map(dcs_all_text)
                        .and_then(dcs_info_opt),
                    title: dcs_child(field, "title", ns_schema)
                        .map(dcs_info_multilang_or_inner_text)
                        .and_then(dcs_info_opt),
                    restrictions: dcs_info_restrictions(field, "useRestriction", ns_schema),
                })
            })
            .collect(),
        total_fields: dcs_children(root, "totalField", ns_schema)
            .into_iter()
            .filter_map(|total| {
                Some(DcsInfoTotalField {
                    data_path: dcs_info_child_text(total, "dataPath", ns_schema)?,
                    expression: dcs_child(total, "expression", ns_schema)
                        .map(dcs_all_text)
                        .and_then(dcs_info_opt),
                    group: dcs_info_child_text(total, "group", ns_schema),
                })
            })
            .collect(),
        parameters: dcs_children(root, "parameter", ns_schema)
            .into_iter()
            .map(|param| DcsInfoParameter {
                name: dcs_child(param, "name", ns_schema)
                    .map(dcs_text_of)
                    .unwrap_or_default(),
                type_name: dcs_child(param, "valueType", ns_schema)
                    .map(dcs_info_compact_type)
                    .and_then(dcs_info_opt),
                value: dcs_child(param, "value", ns_schema)
                    .map(dcs_info_param_default)
                    .and_then(dcs_info_opt),
                expression: dcs_child(param, "expression", ns_schema)
                    .map(dcs_all_text)
                    .and_then(dcs_info_opt),
                restricted: dcs_child(param, "useRestriction", ns_schema)
                    .map(dcs_text_of)
                    .as_deref()
                    == Some("true"),
                available_as_field: dcs_info_flag(param, "availableAsField", ns_schema),
            })
            .collect(),
        // Every element below `settingsVariant` belongs to the settings
        // namespace, including the variant's own `name`, `presentation` and
        // `settings`.
        variants: dcs_children(root, "settingsVariant", ns_schema)
            .into_iter()
            .map(|variant| {
                let settings = dcs_child(variant, "settings", ns_settings);
                DcsInfoVariant {
                    name: dcs_child(variant, "name", ns_settings)
                        .map(dcs_text_of)
                        .unwrap_or_default(),
                    presentation: dcs_child(variant, "presentation", ns_settings)
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
                        .and_then(|node| dcs_child(node, "filter", ns_settings))
                        .map(|node| dcs_children(node, "item", ns_settings).len())
                        .unwrap_or_default(),
                    structure_items: settings.map(dcs_info_structure_rows).unwrap_or_default(),
                    setting_items: settings.map(dcs_info_setting_rows).unwrap_or_default(),
                    structure: settings
                        .map(|node| {
                            dcs_children(node, "item", ns_settings)
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
        templates: dcs_children(root, "template", ns_schema)
            .into_iter()
            .filter_map(|template| dcs_info_child_text(template, "name", ns_schema))
            .collect(),
    }
}

pub(crate) fn parse_dcs_info_xml(
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

pub(crate) fn analyze_dcs_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    support_reader: &dyn SupportStateReader,
) -> AdapterOutcome {
    analyze_dcs_info_with_data(args, context, support_reader).outcome
}

pub(crate) fn analyze_dcs_info_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    support_reader: &dyn SupportStateReader,
) -> DcsInfoExecution {
    let result = (|| -> Result<(DcsInfoData, PathBuf), String> {
        let selection = resolve_dcs_info_target(args, context)?;
        let template_path = selection.resource_path;
        let resolved_path = template_path
            .canonicalize()
            .unwrap_or_else(|_| template_path.clone());
        let text = read_utf8_sig(&resolved_path)?;
        let support = support_reader
            .object_support(&selection.target)
            .map_err(|error| error.to_string())?;
        let data = parse_dcs_info_xml(&text, support)?;
        Ok((data, resolved_path))
    })();

    match result {
        Ok((data, artifact)) => DcsInfoExecution {
            outcome: AdapterOutcome {
                ok: true,
                summary: format!(
                    "unica.dcs.info described {} data set(s) and {} parameter(s)",
                    data.data_sets.len(),
                    data.parameters.len()
                ),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: vec![artifact.display().to_string()],
                stdout: None,
                stderr: None,
                command: None,
            },
            data: Some(data),
        },
        Err(error) => DcsInfoExecution {
            outcome: AdapterOutcome {
                ok: false,
                summary: "unica.dcs.info failed in native DCS inspector".to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![error.clone()],
                artifacts: Vec::new(),
                stdout: None,
                stderr: Some(format!("{error}\n")),
                command: None,
            },
            data: None,
        },
    }
}

pub(crate) fn dcs_info_overview(
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

    let sources = dcs_children(root, "dataSource", ns_schema)
        .into_iter()
        .map(|source| {
            format!(
                "{} ({})",
                dcs_child(source, "name", ns_schema)
                    .map(dcs_text_of)
                    .unwrap_or_default(),
                dcs_child(source, "dataSourceType", ns_schema)
                    .map(dcs_text_of)
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>();
    lines.push(format!("Sources: {}", sources.join(", ")));
    lines.push(String::new());

    lines.push("Datasets:".to_string());
    for data_set in dcs_children(root, "dataSet", ns_schema) {
        dcs_info_dataset_overview(data_set, lines, ns_schema, "  ");
    }

    let links = dcs_children(root, "dataSetLink", ns_schema);
    if !links.is_empty() {
        let mut link_pairs = BTreeMap::<String, usize>::new();
        let mut ordered = Vec::<String>::new();
        for link in links {
            let key = format!(
                "{} -> {}",
                dcs_child(link, "sourceDataSet", ns_schema)
                    .map(dcs_text_of)
                    .unwrap_or_default(),
                dcs_child(link, "destinationDataSet", ns_schema)
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

    let calculated = dcs_children(root, "calculatedField", ns_schema);
    if !calculated.is_empty() {
        lines.push(format!("Calculated: {}", calculated.len()));
    }

    let totals = dcs_children(root, "totalField", ns_schema);
    if !totals.is_empty() {
        let mut unique = HashSet::<String>::new();
        let mut has_grouped = false;
        for total in &totals {
            unique.insert(
                dcs_child(*total, "dataPath", ns_schema)
                    .map(dcs_text_of)
                    .unwrap_or_default(),
            );
            if dcs_child(*total, "group", ns_schema).is_some() {
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

    let templates = dcs_children(root, "template", ns_schema);
    if !templates.is_empty() {
        let field_templates = dcs_children(root, "fieldTemplate", ns_schema);
        let group_count = dcs_children(root, "groupTemplate", ns_schema).len()
            + dcs_children(root, "groupHeaderTemplate", ns_schema).len()
            + dcs_children(root, "groupFooterTemplate", ns_schema).len();
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

    let params = dcs_children(root, "parameter", ns_schema);
    if params.is_empty() {
        lines.push("Params: (none)".to_string());
    } else {
        let mut visible_names = Vec::new();
        let mut hidden_count = 0usize;
        for param in &params {
            let name = dcs_child(*param, "name", ns_schema)
                .map(dcs_text_of)
                .unwrap_or_default();
            let hidden = dcs_child(*param, "useRestriction", ns_schema)
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
    let variants = dcs_children(root, "settingsVariant", ns_schema);
    if !variants.is_empty() {
        lines.push("Variants:".to_string());
        for (index, variant) in variants.iter().enumerate() {
            let name = dcs_child(*variant, "name", ns_settings)
                .map(dcs_text_of)
                .unwrap_or_default();
            let presentation = dcs_child(*variant, "presentation", ns_settings)
                .map(dcs_info_multilang_or_inner_text)
                .unwrap_or_default();
            let presentation_str = if presentation.is_empty() {
                String::new()
            } else {
                format!("  \"{presentation}\"")
            };
            let settings = dcs_child(*variant, "settings", ns_settings);
            let mut struct_items = Vec::new();
            let mut filter_count = 0usize;
            if let Some(settings) = settings {
                for item in dcs_children(settings, "item", ns_settings) {
                    let item_type = dcs_info_structure_item_type(item);
                    let group_fields = dcs_info_group_fields(item, ns_settings);
                    let group = if group_fields.is_empty() {
                        "(detail)".to_string()
                    } else {
                        format!("({})", group_fields.join(","))
                    };
                    struct_items.push(format!("{item_type}{group}"));
                }
                if let Some(filter) = dcs_child(settings, "filter", ns_settings) {
                    filter_count = dcs_children(filter, "item", ns_settings).len();
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

pub(crate) fn dcs_info_dataset_overview(
    data_set: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    indent: &str,
) {
    let ds_type = dcs_info_dataset_type(data_set);
    let name = dcs_child(data_set, "name", ns_schema)
        .map(dcs_text_of)
        .unwrap_or_default();
    let field_count = dcs_children(data_set, "field", ns_schema).len();
    match ds_type.as_str() {
        "Query" => {
            let query_lines = dcs_child(data_set, "query", ns_schema)
                .map(|node| dcs_inner_text(node).split('\n').count())
                .unwrap_or(0);
            lines.push(format!(
                "{indent}[Query]  {name}   {field_count} fields, query {query_lines} lines"
            ));
        }
        "Object" => {
            let obj_str = dcs_child(data_set, "objectName", ns_schema)
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
            for sub_ds in dcs_children(data_set, "item", ns_schema) {
                let sub_type = dcs_info_dataset_type(sub_ds);
                let sub_name = dcs_child(sub_ds, "name", ns_schema)
                    .map(dcs_text_of)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| "?".to_string());
                let sub_fields = dcs_children(sub_ds, "field", ns_schema).len();
                if sub_type == "Query" {
                    let query_lines = dcs_child(sub_ds, "query", ns_schema)
                        .map(|node| dcs_inner_text(node).split('\n').count())
                        .unwrap_or(0);
                    lines.push(format!(
                        "    ├─ [Query] {sub_name}   {sub_fields} fields, query {query_lines} lines"
                    ));
                } else if sub_type == "Object" {
                    let obj_str = dcs_child(sub_ds, "objectName", ns_schema)
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

pub(crate) fn dcs_info_overview_hints(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    ns_settings: &str,
) {
    lines.push(String::new());
    let mut hints = Vec::<String>::new();
    let mut query_names = Vec::<String>::new();
    for data_set in dcs_children(root, "dataSet", ns_schema) {
        if dcs_info_dataset_type(data_set) == "Query" {
            query_names.push(
                dcs_child(data_set, "name", ns_schema)
                    .map(dcs_text_of)
                    .unwrap_or_default(),
            );
        } else if dcs_info_dataset_type(data_set) == "Union" {
            for sub_ds in dcs_children(data_set, "item", ns_schema) {
                if dcs_info_dataset_type(sub_ds) == "Query" {
                    query_names.push(
                        dcs_child(sub_ds, "name", ns_schema)
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
    let links = dcs_children(root, "dataSetLink", ns_schema);
    if !links.is_empty() {
        hints.push(format!(
            "-Mode links             dataset connections ({})",
            links.len()
        ));
    }
    let calculated = dcs_children(root, "calculatedField", ns_schema);
    if !calculated.is_empty() {
        hints.push(format!(
            "-Mode calculated        calculated field expressions ({})",
            calculated.len()
        ));
    }
    let totals = dcs_children(root, "totalField", ns_schema);
    if !totals.is_empty() {
        hints.push(format!(
            "-Mode resources         resource aggregation ({})",
            totals.len()
        ));
    }
    if !dcs_children(root, "parameter", ns_schema).is_empty() {
        hints.push("-Mode params            parameter details".to_string());
    }
    let variants = dcs_children(root, "settingsVariant", ns_schema);
    if variants.len() == 1 {
        hints.push("-Mode variant           variant structure".to_string());
    } else if variants.len() > 1 {
        hints.push(format!(
            "-Mode variant -Name <N> variant structure (1..{})",
            variants.len()
        ));
    }
    if !dcs_children(root, "template", ns_schema).is_empty() {
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

pub(crate) fn dcs_info_query(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    name: Option<&str>,
) -> Result<(), String> {
    let target = dcs_info_query_target(root, ns_schema, name)?;
    let query = dcs_child(target, "query", ns_schema)
        .map(dcs_inner_text)
        .unwrap_or_default();
    let name = dcs_child(target, "name", ns_schema)
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

pub(crate) fn dcs_info_raw_query(
    root: roxmltree::Node<'_, '_>,
    ns_schema: &str,
    name: Option<&str>,
) -> Result<String, String> {
    let target = dcs_info_query_target(root, ns_schema, name)?;
    Ok(dcs_child(target, "query", ns_schema)
        .map(dcs_inner_text)
        .unwrap_or_default())
}

fn dcs_info_query_target<'a, 'input>(
    root: roxmltree::Node<'a, 'input>,
    ns_schema: &str,
    name: Option<&str>,
) -> Result<roxmltree::Node<'a, 'input>, String> {
    let mut target = None;
    if let Some(name) = name.filter(|value| !value.is_empty()) {
        for data_set in dcs_children(root, "dataSet", ns_schema) {
            if dcs_info_dataset_type(data_set) == "Union" {
                for sub_ds in dcs_children(data_set, "item", ns_schema) {
                    let ds_name = dcs_child(sub_ds, "name", ns_schema)
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
        for data_set in dcs_children(root, "dataSet", ns_schema) {
            if target.is_some() {
                break;
            }
            let ds_name = dcs_child(data_set, "name", ns_schema)
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
        for data_set in dcs_children(root, "dataSet", ns_schema) {
            if dcs_info_dataset_type(data_set) == "Query" {
                target = Some(data_set);
                break;
            }
            if dcs_info_dataset_type(data_set) == "Union" {
                for sub_ds in dcs_children(data_set, "item", ns_schema) {
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
    if dcs_child(target, "query", ns_schema).is_none() {
        if dcs_info_dataset_type(target) == "Union" {
            let sub_names = dcs_children(target, "item", ns_schema)
                .into_iter()
                .filter_map(|sub_ds| dcs_child(sub_ds, "name", ns_schema).map(dcs_text_of))
                .collect::<Vec<_>>();
            let ds_name = dcs_child(target, "name", ns_schema)
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

pub(crate) fn dcs_info_fields(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
) {
    lines.push("=== Fields map ===".to_string());
    for data_set in dcs_children(root, "dataSet", ns_schema) {
        dcs_info_field_map(data_set, lines, ns_schema, "");
        if dcs_info_dataset_type(data_set) == "Union" {
            for sub_ds in dcs_children(data_set, "item", ns_schema) {
                dcs_info_field_map(sub_ds, lines, ns_schema, "  ");
            }
        }
    }
    lines.push(String::new());
    lines.push("Use -Name <field> for details.".to_string());
}

pub(crate) fn dcs_info_field_map(
    data_set: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    indent: &str,
) {
    let fields = dcs_children(data_set, "field", ns_schema)
        .into_iter()
        .filter_map(|field| dcs_child(field, "dataPath", ns_schema).map(dcs_text_of))
        .collect::<Vec<_>>();
    let name = dcs_child(data_set, "name", ns_schema)
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

pub(crate) fn dcs_info_links(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
) {
    let links = dcs_children(root, "dataSetLink", ns_schema);
    if links.is_empty() {
        lines.push("(no links)".to_string());
    } else {
        lines.push(format!("=== Links ({}) ===", links.len()));
    }
}

pub(crate) fn dcs_info_calculated(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    name: Option<&str>,
) -> Result<(), String> {
    let calculated = dcs_children(root, "calculatedField", ns_schema);
    if calculated.is_empty() {
        lines.push("(no calculated fields)".to_string());
        return Ok(());
    }
    if let Some(name) = name.filter(|value| !value.is_empty()) {
        for field in calculated {
            let path = dcs_child(field, "dataPath", ns_schema)
                .map(dcs_text_of)
                .unwrap_or_default();
            if path != name {
                continue;
            }
            lines.push(format!("=== Calculated: {path} ==="));
            lines.push(String::new());
            lines.push("Expression:".to_string());
            let expression = dcs_child(field, "expression", ns_schema)
                .map(dcs_all_text)
                .unwrap_or_default();
            for line in expression.split('\n') {
                lines.push(format!("  {}", line.trim_end()));
            }
            if let Some(title) = dcs_child(field, "title", ns_schema)
                .map(dcs_info_multilang_or_inner_text)
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("Title: {title}"));
            }
            if let Some(restriction) = dcs_child(field, "useRestriction", ns_schema) {
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
        let path = dcs_child(field, "dataPath", ns_schema)
            .map(dcs_text_of)
            .unwrap_or_default();
        let title = dcs_child(field, "title", ns_schema)
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

pub(crate) fn dcs_info_resources(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    name: Option<&str>,
) -> Result<(), String> {
    let totals = dcs_children(root, "totalField", ns_schema);
    if totals.is_empty() {
        lines.push("(no resources)".to_string());
        return Ok(());
    }
    if let Some(name) = name.filter(|value| !value.is_empty()) {
        let matched = totals
            .into_iter()
            .filter(|total| {
                dcs_child(*total, "dataPath", ns_schema)
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
            let expression = dcs_child(total, "expression", ns_schema)
                .map(dcs_text_of)
                .unwrap_or_default();
            let group = dcs_child(total, "group", ns_schema)
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
        let path = dcs_child(total, "dataPath", ns_schema)
            .map(dcs_text_of)
            .unwrap_or_default();
        if !has_group.contains_key(&path) {
            ordered.push(path.clone());
        }
        if dcs_child(total, "group", ns_schema).is_some() {
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

pub(crate) fn dcs_info_params(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
) {
    let params = dcs_children(root, "parameter", ns_schema);
    lines.push(format!("=== Parameters ({}) ===", params.len()));
    lines.push("  Name                            Type                   Default          Visible  Expression".to_string());
    for param in params {
        let name = dcs_child(param, "name", ns_schema)
            .map(dcs_text_of)
            .unwrap_or_default();
        let type_name = dcs_child(param, "valueType", ns_schema)
            .map(dcs_info_compact_type)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "-".to_string());
        let default = dcs_child(param, "value", ns_schema)
            .map(dcs_info_param_default)
            .unwrap_or_else(|| "-".to_string());
        let visible = dcs_child(param, "useRestriction", ns_schema)
            .map(dcs_text_of)
            .map(|value| if value == "true" { "hidden" } else { "yes" })
            .unwrap_or("yes");
        let expression = dcs_child(param, "expression", ns_schema)
            .map(dcs_all_text)
            .map(|value| {
                if value.is_empty() {
                    "-".to_string()
                } else {
                    value
                }
            })
            .unwrap_or_else(|| "-".to_string());
        let no_field = dcs_child(param, "availableAsField", ns_schema)
            .map(dcs_text_of)
            .is_some_and(|value| value == "false");
        let suffix = if no_field { " [noField]" } else { "" };
        lines.push(format!(
            "  {:<33} {:<22} {:<16} {:<8} {}{}",
            name, type_name, default, visible, expression, suffix
        ));
    }
}

pub(crate) fn dcs_info_variant(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    ns_settings: &str,
) {
    let variants = dcs_children(root, "settingsVariant", ns_schema);
    if variants.is_empty() {
        lines.push("=== Variants: (none) ===".to_string());
        return;
    }
    lines.push(format!("=== Variants ({}) ===", variants.len()));
    for (index, variant) in variants.iter().enumerate() {
        let name = dcs_child(*variant, "name", ns_settings)
            .map(dcs_text_of)
            .unwrap_or_default();
        let presentation = dcs_child(*variant, "presentation", ns_settings)
            .map(dcs_info_multilang_or_inner_text)
            .unwrap_or_default();
        let presentation_str = if presentation.is_empty() {
            String::new()
        } else {
            format!("  \"{presentation}\"")
        };
        let settings = dcs_child(*variant, "settings", ns_settings);
        let mut struct_items = Vec::new();
        let mut filter_count = 0usize;
        let mut selection = Vec::new();
        if let Some(settings) = settings {
            for item in dcs_children(settings, "item", ns_settings) {
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
            if let Some(filter) = dcs_child(settings, "filter", ns_settings) {
                filter_count = dcs_children(filter, "item", ns_settings).len();
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

pub(crate) fn dcs_info_trace(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    name: &str,
) -> Result<(), String> {
    let mut dataset_hits = Vec::<String>::new();
    let mut title = String::new();
    for data_set in dcs_children(root, "dataSet", ns_schema) {
        dcs_info_collect_field_trace(data_set, ns_schema, name, &mut dataset_hits, &mut title);
        for sub_ds in dcs_children(data_set, "item", ns_schema) {
            dcs_info_collect_field_trace(sub_ds, ns_schema, name, &mut dataset_hits, &mut title);
        }
    }

    let mut calc_expression = None::<String>;
    let mut calc_operands = Vec::<String>::new();
    for field in dcs_children(root, "calculatedField", ns_schema) {
        let path = dcs_child(field, "dataPath", ns_schema)
            .map(dcs_text_of)
            .unwrap_or_default();
        let field_title = dcs_child(field, "title", ns_schema)
            .map(dcs_info_multilang_or_inner_text)
            .unwrap_or_default();
        if path == name || field_title == name {
            if title.is_empty() {
                title = field_title;
            }
            let expression = dcs_child(field, "expression", ns_schema)
                .map(dcs_all_text)
                .unwrap_or_default();
            for data_set in dcs_children(root, "dataSet", ns_schema) {
                for operand in dcs_info_dataset_field_paths(data_set, ns_schema) {
                    if !operand.is_empty() && expression.contains(&operand) {
                        let ds_name = dcs_child(data_set, "name", ns_schema)
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
    for total in dcs_children(root, "totalField", ns_schema) {
        let path = dcs_child(total, "dataPath", ns_schema)
            .map(dcs_text_of)
            .unwrap_or_default();
        if path == name {
            let group = dcs_child(total, "group", ns_schema)
                .map(dcs_text_of)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "(overall)".to_string());
            let expression = dcs_child(total, "expression", ns_schema)
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

pub(crate) fn dcs_info_full(
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
    if dcs_children(root, "dataSet", ns_schema)
        .iter()
        .any(|data_set| dcs_info_dataset_type(*data_set) == "Query")
    {
        dcs_info_query(root, lines, ns_schema, None)?;
    } else {
        let object_names = dcs_children(root, "dataSet", ns_schema)
            .into_iter()
            .filter(|data_set| dcs_info_dataset_type(*data_set) == "Object")
            .filter_map(|data_set| dcs_child(data_set, "objectName", ns_schema).map(dcs_text_of))
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

pub(crate) fn dcs_info_templates(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
) {
    let templates = dcs_children(root, "template", ns_schema);
    let field_count = dcs_children(root, "fieldTemplate", ns_schema).len();
    let group_count = dcs_children(root, "groupTemplate", ns_schema).len()
        + dcs_children(root, "groupHeaderTemplate", ns_schema).len()
        + dcs_children(root, "groupFooterTemplate", ns_schema).len();
    lines.push(format!(
        "=== Templates ({} defined: {field_count} field, {group_count} group) ===",
        templates.len()
    ));
}

pub(crate) fn dcs_info_dataset_type(data_set: roxmltree::Node<'_, '_>) -> String {
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
pub(crate) fn dcs_info_structure_item_type(item: roxmltree::Node<'_, '_>) -> &'static str {
    if ["row", "column", "point", "series"]
        .iter()
        .any(|tag| role_info_element(item, tag, Some(DCS_SETTINGS_NS)))
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

pub(crate) fn dcs_info_multilang_or_inner_text(node: roxmltree::Node<'_, '_>) -> String {
    let value = multilang_text(node);
    if value.is_empty() {
        if let Some(text) = node.text().map(str::trim).filter(|value| !value.is_empty()) {
            return text.to_string();
        }
        dcs_all_text(node)
    } else {
        value
    }
}

pub(crate) fn dcs_info_group_fields(
    item: roxmltree::Node<'_, '_>,
    ns_settings: &str,
) -> Vec<String> {
    let mut fields = Vec::new();
    for group_item in dcs_find_all_path(item, &[("groupItems", ns_settings), ("item", ns_settings)])
    {
        if let Some(field) = dcs_child(group_item, "field", ns_settings) {
            let mut value = dcs_text_of(field);
            let group_type = dcs_child(group_item, "groupType", ns_settings)
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

pub(crate) fn dcs_info_selection_fields(
    item_node: roxmltree::Node<'_, '_>,
    ns_settings: &str,
) -> Vec<String> {
    let mut fields = Vec::new();
    if let Some(selection) = dcs_child(item_node, "selection", ns_settings) {
        for item in dcs_children(selection, "item", ns_settings) {
            let xsi_type = attribute_by_local_name(item, "type").unwrap_or("");
            if xsi_type.contains("SelectedItemAuto") {
                fields.push("Auto".to_string());
            } else if xsi_type.contains("SelectedItemField") {
                if let Some(field) = dcs_child(item, "field", ns_settings) {
                    fields.push(dcs_text_of(field));
                }
            } else if xsi_type.contains("SelectedItemFolder") {
                fields.push("Folder".to_string());
            }
        }
    }
    fields
}

pub(crate) fn dcs_info_compact_type(value_type: roxmltree::Node<'_, '_>) -> String {
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

pub(crate) fn dcs_info_param_default(value_node: roxmltree::Node<'_, '_>) -> String {
    if attribute_by_local_name(value_node, "nil").is_some_and(|value| value == "true") {
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

pub(crate) fn dcs_info_collect_field_trace(
    data_set: roxmltree::Node<'_, '_>,
    ns_schema: &str,
    name: &str,
    dataset_hits: &mut Vec<String>,
    title: &mut String,
) {
    let ds_name = dcs_child(data_set, "name", ns_schema)
        .map(dcs_text_of)
        .unwrap_or_default();
    let ds_type = dcs_info_dataset_type(data_set);
    for field in dcs_children(data_set, "field", ns_schema) {
        let path = dcs_child(field, "dataPath", ns_schema)
            .map(dcs_text_of)
            .unwrap_or_default();
        let field_title = dcs_child(field, "title", ns_schema)
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

pub(crate) fn dcs_info_dataset_field_paths(
    data_set: roxmltree::Node<'_, '_>,
    ns_schema: &str,
) -> Vec<String> {
    dcs_children(data_set, "field", ns_schema)
        .into_iter()
        .filter_map(|field| dcs_child(field, "dataPath", ns_schema).map(dcs_text_of))
        .collect()
}

pub(crate) fn dcs_info_template_name(path: &Path) -> String {
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

struct DcsInfoPathInspection {
    resolution: Result<PathBuf, String>,
    dependencies: Vec<PathBuf>,
}

fn inspect_dcs_info_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> DcsInfoPathInspection {
    // A logical target is already proven down to the file, so none of the
    // `Ext/Template.xml` probing below applies. The resolved resource is still
    // a format dependency: leaving the list empty would let a logical call
    // reach the reader without the guard ever inspecting the file it reads.
    if let Some(selection) =
        logical_selection(args, context, AttachedResource::Template, TEMPLATE_KINDS)
    {
        let resolution = selection
            .map(|selection| selection.resource_path)
            .map_err(|failure| failure.to_string());
        let dependencies = resolution.as_ref().ok().cloned().into_iter().collect();
        return DcsInfoPathInspection {
            resolution,
            dependencies,
        };
    }
    let raw_path = match required_path(args, TEMPLATE_PATH, "TemplatePath") {
        Ok(path) => path,
        Err(error) => {
            return DcsInfoPathInspection {
                resolution: Err(error),
                dependencies: Vec::new(),
            };
        }
    };
    let original_path = raw_path.clone();
    let mut template_path = raw_path.clone();
    let mut dependencies = Vec::new();
    if template_path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| !value.eq_ignore_ascii_case("xml"))
        .unwrap_or(true)
    {
        let candidate = template_path.join("Ext").join("Template.xml");
        if absolutize(candidate.clone(), &context.cwd).is_file() {
            template_path = candidate;
        }
    }

    let abs_template = absolutize(template_path.clone(), &context.cwd);
    if !abs_template.is_file()
        && template_path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| !value.eq_ignore_ascii_case("xml"))
            .unwrap_or(true)
    {
        let templates_dir = absolutize(original_path.join("Templates"), &context.cwd);
        if templates_dir.is_dir() {
            let mut dcs_templates = Vec::<PathBuf>::new();
            let entries = match fs::read_dir(&templates_dir) {
                Ok(entries) => entries,
                Err(err) => {
                    return DcsInfoPathInspection {
                        resolution: Err(format!(
                            "failed to read {}: {err}",
                            templates_dir.display()
                        )),
                        dependencies,
                    };
                }
            };
            let mut entries = match entries.collect::<Result<Vec<_>, _>>() {
                Ok(entries) => entries,
                Err(err) => {
                    return DcsInfoPathInspection {
                        resolution: Err(format!(
                            "failed to read {}: {err}",
                            templates_dir.display()
                        )),
                        dependencies,
                    };
                }
            };
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) != Some("xml") {
                    continue;
                }
                dependencies.push(path.clone());
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(doc) = Document::parse(text.trim_start_matches('\u{feff}')) else {
                    continue;
                };
                let template_type = doc
                    .descendants()
                    .find(|node| node.is_element() && node.tag_name().name() == "TemplateType")
                    .and_then(|node| node.text())
                    .unwrap_or("")
                    .trim();
                if template_type == "DataCompositionSchema" {
                    if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
                        let template = templates_dir.join(stem).join("Ext").join("Template.xml");
                        if template.is_file() {
                            dcs_templates.push(template);
                        }
                    }
                }
            }
            if dcs_templates.len() == 1 {
                let resolved_path = dcs_templates.remove(0);
                dependencies.push(resolved_path.clone());
                return DcsInfoPathInspection {
                    resolution: Ok(resolved_path),
                    dependencies,
                };
            }
            if dcs_templates.len() > 1 {
                return DcsInfoPathInspection {
                    resolution: Err(format!(
                        "Multiple DCS templates found in: {}",
                        original_path.display()
                    )),
                    dependencies,
                };
            }
            return DcsInfoPathInspection {
                resolution: Err(format!(
                    "No DCS templates found in: {}",
                    original_path.display()
                )),
                dependencies,
            };
        }
    }

    let abs_template = absolutize(template_path, &context.cwd);
    if !abs_template.is_file() {
        return DcsInfoPathInspection {
            resolution: Err(format!("File not found: {}", abs_template.display())),
            dependencies,
        };
    }
    dependencies.push(abs_template.clone());
    DcsInfoPathInspection {
        resolution: Ok(abs_template),
        dependencies,
    }
}

pub(crate) fn resolve_dcs_info_path_for_script(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    inspect_dcs_info_path(args, context).resolution
}

pub(crate) fn resolve_dcs_info_target(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<ResolvedReadTarget, String> {
    if let Some(selection) =
        logical_selection(args, context, AttachedResource::Template, TEMPLATE_KINDS)
    {
        return selection.map_err(|failure| failure.to_string());
    }
    let template_path = resolve_dcs_info_path_for_script(args, context)?;
    physical_selection(&template_path, context, AttachedResource::Template)
        .map_err(|failure| failure.to_string())
}

pub(crate) fn dcs_info_format_dependency_paths(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Vec<PathBuf> {
    inspect_dcs_info_path(args, context).dependencies
}

pub(crate) fn validate_dcs(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let result = (|| {
        let template_path = resolve_dcs_validate_path(args, context)?;
        let resolved_path = template_path.canonicalize().unwrap_or(template_path);
        let text = read_utf8_sig(&resolved_path)?;
        validate_dcs_text(args, &text, resolved_path)
    })();
    dcs_validation_outcome(result)
}

/// Validates the exact input already read through the canonical source authority.
/// The path is an artifact label; this entry point never reopens it.
pub(crate) fn validate_dcs_input(
    args: &Map<String, Value>,
    text: &str,
    artifact: PathBuf,
) -> AdapterOutcome {
    dcs_validation_outcome(validate_dcs_text(args, text, artifact))
}

fn validate_dcs_text(
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
    let detailed = bool_arg(args, &["detailed", "Detailed"]);
    let max_errors = int_arg(args, &["maxErrors", "MaxErrors"])
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

    let data_source_nodes = dcs_children(root, "dataSource", NS_SCHEMA);
    let mut data_source_names = HashSet::<String>::new();
    for dsn in &data_source_nodes {
        if let Some(name) = dcs_child(*dsn, "name", NS_SCHEMA) {
            data_source_names.insert(dcs_inner_text(name));
        }
    }

    let data_set_nodes = dcs_children(root, "dataSet", NS_SCHEMA);
    let mut data_set_names = HashSet::<String>::new();
    let mut all_field_paths = HashMap::<String, String>::new();
    for ds in &data_set_nodes {
        if let Some(name_node) = dcs_child(*ds, "name", NS_SCHEMA) {
            let ds_name = dcs_inner_text(name_node);
            data_set_names.insert(ds_name.clone());
            dcs_collect_data_set_fields(*ds, &ds_name, &mut all_field_paths);
        }
    }

    let calc_field_nodes = dcs_children(root, "calculatedField", NS_SCHEMA);
    let mut calc_field_paths = HashSet::<String>::new();
    for cf in &calc_field_nodes {
        if let Some(dp) = dcs_child(*cf, "dataPath", NS_SCHEMA) {
            calc_field_paths.insert(dcs_inner_text(dp));
        }
    }
    let total_field_nodes = dcs_children(root, "totalField", NS_SCHEMA);
    let param_nodes = dcs_children(root, "parameter", NS_SCHEMA);
    let template_nodes = dcs_children(root, "template", NS_SCHEMA);
    let mut template_names = HashSet::<String>::new();
    for template in &template_nodes {
        if let Some(name_node) = dcs_child(*template, "name", NS_SCHEMA) {
            template_names.insert(dcs_inner_text(name_node));
        }
    }
    let group_template_nodes = dcs_children(root, "groupTemplate", NS_SCHEMA);
    let variant_nodes = dcs_children(root, "settingsVariant", NS_SCHEMA);
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
        let ds_name = dcs_child(*ds, "name", NS_SCHEMA)
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

fn dcs_validation_outcome(result: Result<DcsValidationRun, String>) -> AdapterOutcome {
    match result {
        Ok(run) => AdapterOutcome {
            ok: run.ok,
            summary: if run.ok {
                "unica.dcs.validate completed with native DCS validator".to_string()
            } else {
                "unica.dcs.validate failed in native DCS validator".to_string()
            },
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: run.errors,
            artifacts: vec![run.artifact.display().to_string()],
            stdout: Some(run.stdout),
            stderr: Some(String::new()),
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.dcs.validate failed in native DCS validator".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some(format!("{error}\n")),
            command: None,
        },
    }
}

pub(crate) fn dcs_validation_finish(
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

pub(crate) fn dcs_validate_data_sources(
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
        let name = dcs_child(*dsn, "name", NS_SCHEMA);
        let typ = dcs_child(*dsn, "dataSourceType", NS_SCHEMA);
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

pub(crate) fn dcs_validate_value_types(
    report: &mut DcsValidationReporter,
    root: roxmltree::Node<'_, '_>,
) {
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
        .filter(|node| role_info_element(*node, "valueType", Some(NS_SCHEMA)))
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
                        let digits = dcs_child(child, "Digits", NS_V8).map(dcs_text_of);
                        let fraction = dcs_child(child, "FractionDigits", NS_V8).map(dcs_text_of);
                        let sign = dcs_child(child, "AllowedSign", NS_V8).map(dcs_text_of);
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
                        let length = dcs_child(child, "Length", NS_V8).map(dcs_text_of);
                        let allowed_length =
                            dcs_child(child, "AllowedLength", NS_V8).map(dcs_text_of);
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
                        let fractions = dcs_child(child, "DateFractions", NS_V8).map(dcs_text_of);
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

pub(crate) fn dcs_validate_value_contents(
    report: &mut DcsValidationReporter,
    root: roxmltree::Node<'_, '_>,
) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    const NS_CORE: &str = "http://v8.1c.ru/8.1/data-composition-system/core";
    let value_nodes = root
        .descendants()
        .filter(|node| {
            (role_info_element(*node, "value", Some(NS_SCHEMA))
                || role_info_element(*node, "value", Some(NS_CORE)))
                && attribute_by_local_name(*node, "type").is_some()
        })
        .collect::<Vec<_>>();

    let mut checked = 0usize;
    let mut ok = true;
    for value_node in value_nodes {
        checked += 1;
        let xsi_type = attribute_by_local_name(value_node, "type").unwrap_or("");
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

pub(crate) fn dcs_validate_design_time_value_ref_shape(value: &str) -> bool {
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

pub(crate) fn dcs_validate_config_ref_type_shape(local_type: &str) -> bool {
    let Some((kind, name)) = local_type.split_once('.') else {
        return false;
    };
    !kind.is_empty() && !name.is_empty() && kind.chars().all(|ch| ch.is_ascii_alphabetic())
}

pub(crate) fn dcs_validate_system_type_shape(local_type: &str) -> bool {
    let mut chars = local_type.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic() && chars.all(|ch| ch.is_ascii_alphanumeric())
}

pub(crate) fn dcs_validate_data_sets(
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
        let xsi_type = attribute_by_local_name(*ds, "type").unwrap_or("");
        let name_node = dcs_child(*ds, "name", NS_SCHEMA);
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
            if let Some(src_node) = dcs_child(*ds, "dataSource", NS_SCHEMA) {
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
            let query_node = dcs_child(*ds, "query", NS_SCHEMA);
            if query_node.map(dcs_text_of).unwrap_or_default().is_empty() {
                report.warn(format!("DataSet '{ds_name}' (Query) has empty query"));
            }
        }
        if xsi_type == "DataSetObject" {
            let obj_node = dcs_child(*ds, "objectName", NS_SCHEMA);
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

pub(crate) fn dcs_validate_data_set_fields(
    report: &mut DcsValidationReporter,
    ds_node: roxmltree::Node<'_, '_>,
    ds_name: &str,
) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    let fields = dcs_children(ds_node, "field", NS_SCHEMA);
    if fields.is_empty() {
        return;
    }
    let mut paths_seen = HashSet::<String>::new();
    let mut field_ok = true;
    for field in &fields {
        let dp = dcs_child(*field, "dataPath", NS_SCHEMA);
        let field_ref = dcs_child(*field, "field", NS_SCHEMA);
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
    for item in dcs_children(ds_node, "item", NS_SCHEMA) {
        let item_name = dcs_child(item, "name", NS_SCHEMA)
            .map(dcs_inner_text)
            .unwrap_or_else(|| "(unnamed item)".to_string());
        dcs_validate_data_set_fields(report, item, &item_name);
    }
}

pub(crate) fn dcs_validate_data_set_links(
    report: &mut DcsValidationReporter,
    root: roxmltree::Node<'_, '_>,
    data_set_names: &HashSet<String>,
) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    let link_nodes = dcs_children(root, "dataSetLink", NS_SCHEMA);
    if link_nodes.is_empty() {
        return;
    }
    let mut link_ok = true;
    for link in &link_nodes {
        let src = dcs_child(*link, "sourceDataSet", NS_SCHEMA);
        let dst = dcs_child(*link, "destinationDataSet", NS_SCHEMA);
        let src_expr = dcs_child(*link, "sourceExpression", NS_SCHEMA);
        let dst_expr = dcs_child(*link, "destinationExpression", NS_SCHEMA);
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

pub(crate) fn dcs_validate_calculated_fields(
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
        let dp = dcs_child(*calc, "dataPath", NS_SCHEMA);
        let expr = dcs_child(*calc, "expression", NS_SCHEMA);
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

pub(crate) fn dcs_validate_total_fields(
    report: &mut DcsValidationReporter,
    total_field_nodes: &[roxmltree::Node<'_, '_>],
) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    if total_field_nodes.is_empty() {
        return;
    }
    let mut tf_ok = true;
    for total in total_field_nodes {
        let dp = dcs_child(*total, "dataPath", NS_SCHEMA);
        let expr = dcs_child(*total, "expression", NS_SCHEMA);
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

pub(crate) fn dcs_validate_parameters(
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
        let name = dcs_child(*param, "name", NS_SCHEMA)
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

pub(crate) fn dcs_validate_templates(
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
        let name = dcs_child(*template, "name", NS_SCHEMA)
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

pub(crate) fn dcs_validate_group_templates(
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
        let tpl_ref = dcs_child(*group_template, "template", NS_SCHEMA)
            .map(dcs_inner_text)
            .unwrap_or_default();
        let tpl_type = dcs_child(*group_template, "templateType", NS_SCHEMA)
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

pub(crate) fn dcs_validate_settings_variants(
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
        let v_name = dcs_child(*variant, "name", NS_SETTINGS);
        let variant_name = v_name.map(dcs_inner_text).unwrap_or_default();
        if variant_name.is_empty() {
            report.error(format!("SettingsVariant #{} has empty name", idx + 1));
            v_ok = false;
        }
        let settings = dcs_child(*variant, "settings", NS_SETTINGS);
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

pub(crate) fn dcs_check_settings(
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
        let xsi_type = attribute_by_local_name(selected_item, "type").unwrap_or("");
        if xsi_type == "dcsset:SelectedItemField" {
            let field = dcs_child(selected_item, "field", NS_SETTINGS)
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
        let parameter = dcs_child(parameter_item, "parameter", DCS_CORE_NS)
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
        let xsi_type = attribute_by_local_name(order_item, "type").unwrap_or("");
        if xsi_type == "dcsset:OrderItemField" {
            let order_type = dcs_child(order_item, "orderType", NS_SETTINGS)
                .map(dcs_inner_text)
                .unwrap_or_default();
            if !order_type.is_empty() && !matches!(order_type.as_str(), "Asc" | "Desc") {
                report.warn(format!(
                    "Variant '{variant_name}' order: invalid orderType '{order_type}'"
                ));
            }
        }
    }
    for structure_item in dcs_children(settings_node, "item", NS_SETTINGS) {
        dcs_check_structure_item(report, structure_item, variant_name);
    }
}

pub(crate) fn dcs_check_filter_items(
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
        let xsi_type = attribute_by_local_name(filter_item, "type").unwrap_or("");
        if xsi_type == "dcsset:FilterItemComparison" {
            let comp_type = dcs_child(filter_item, "comparisonType", NS_SETTINGS)
                .map(dcs_inner_text)
                .unwrap_or_default();
            if !comp_type.is_empty() && !valid_comparison_types.contains(&comp_type.as_str()) {
                report.error(format!(
                    "Variant '{variant_name}' filter: invalid comparisonType '{comp_type}'"
                ));
            }
        } else if xsi_type == "dcsset:FilterItemGroup" {
            let group_type = dcs_child(filter_item, "groupType", NS_SETTINGS)
                .map(dcs_inner_text)
                .unwrap_or_default();
            if !group_type.is_empty()
                && !matches!(group_type.as_str(), "AndGroup" | "OrGroup" | "NotGroup")
            {
                report.warn(format!(
                    "Variant '{variant_name}' filter group: unusual groupType '{group_type}'"
                ));
            }
            for nested in dcs_children(filter_item, "item", NS_SETTINGS) {
                let nested_type = attribute_by_local_name(nested, "type").unwrap_or("");
                if nested_type == "dcsset:FilterItemComparison" {
                    let comp_type = dcs_child(nested, "comparisonType", NS_SETTINGS)
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

pub(crate) fn dcs_check_structure_item(
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
    let xsi_type = attribute_by_local_name(item_node, "type").unwrap_or("");
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
    for nested in dcs_children(item_node, "item", NS_SETTINGS) {
        dcs_check_structure_item(report, nested, variant_name);
    }
    if xsi_type == "dcsset:StructureItemTable" {
        let columns = dcs_children(item_node, "column", NS_SETTINGS);
        let rows = dcs_children(item_node, "row", NS_SETTINGS);
        if columns.is_empty() {
            report.warn(format!("Variant '{variant_name}': table has no columns"));
        }
        if rows.is_empty() {
            report.warn(format!("Variant '{variant_name}': table has no rows"));
        }
    }
}

pub(crate) fn dcs_collect_data_set_fields(
    ds_node: roxmltree::Node<'_, '_>,
    ds_name: &str,
    all_field_paths: &mut HashMap<String, String>,
) -> HashSet<String> {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    let mut local_paths = HashSet::<String>::new();
    for field in dcs_children(ds_node, "field", NS_SCHEMA) {
        if let Some(dp) = dcs_child(field, "dataPath", NS_SCHEMA) {
            let path = dcs_inner_text(dp);
            local_paths.insert(path.clone());
            all_field_paths.insert(path, ds_name.to_string());
        }
    }
    for item in dcs_children(ds_node, "item", NS_SCHEMA) {
        if let Some(item_name) = dcs_child(item, "name", NS_SCHEMA) {
            dcs_collect_data_set_fields(item, &dcs_inner_text(item_name), all_field_paths);
        }
    }
    local_paths
}

pub(crate) fn dcs_children<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
    namespace: &str,
) -> Vec<roxmltree::Node<'a, 'input>> {
    node.children()
        .filter(|child| role_info_element(*child, local_name, Some(namespace)))
        .collect()
}

pub(crate) fn dcs_child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
    namespace: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children()
        .find(|child| role_info_element(*child, local_name, Some(namespace)))
}

pub(crate) fn dcs_find_all_path<'a, 'input>(
    parent: roxmltree::Node<'a, 'input>,
    path: &[(&str, &str)],
) -> Vec<roxmltree::Node<'a, 'input>> {
    let mut current = vec![parent];
    for (local_name, namespace) in path {
        let mut next = Vec::<roxmltree::Node<'a, 'input>>::new();
        for node in current {
            next.extend(dcs_children(node, local_name, namespace));
        }
        current = next;
    }
    current
}

pub(crate) fn dcs_inner_text(node: roxmltree::Node<'_, '_>) -> String {
    node.children()
        .filter(|child| child.is_text())
        .filter_map(|child| child.text())
        .collect()
}

pub(crate) fn dcs_text_of(node: roxmltree::Node<'_, '_>) -> String {
    node.text().unwrap_or("").trim().to_string()
}

pub(crate) fn dcs_all_text(node: roxmltree::Node<'_, '_>) -> String {
    node.descendants()
        .filter(|child| child.is_text())
        .filter_map(|child| child.text())
        .collect::<String>()
        .trim()
        .to_string()
}

pub(crate) fn resolve_dcs_validate_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    if let Some(selection) =
        logical_selection(args, context, AttachedResource::Template, TEMPLATE_KINDS)
    {
        return selection
            .map(|selection| selection.resource_path)
            .map_err(|failure| failure.to_string());
    }
    let raw_path = required_path(args, TEMPLATE_PATH, "TemplatePath")?;
    let mut display_path = raw_path.clone();
    let mut template_path = absolutize(raw_path, &context.cwd);

    if template_path.is_dir() {
        display_path = display_path.join("Ext").join("Template.xml");
        template_path = template_path.join("Ext").join("Template.xml");
    }
    if !template_path.exists()
        && display_path.file_name().and_then(|value| value.to_str()) == Some("Template.xml")
    {
        let display_candidate = display_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("Ext")
            .join("Template.xml");
        let candidate = template_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("Ext")
            .join("Template.xml");
        if candidate.exists() {
            display_path = display_candidate;
            template_path = candidate;
        }
    }
    if !template_path.exists()
        && display_path
            .extension()
            .and_then(|value| value.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("xml"))
            .unwrap_or(false)
    {
        if let Some(stem) = display_path.file_stem().and_then(|value| value.to_str()) {
            let display_candidate = display_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(stem)
                .join("Ext")
                .join("Template.xml");
            let candidate = template_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(stem)
                .join("Ext")
                .join("Template.xml");
            if candidate.exists() {
                display_path = display_candidate;
                template_path = candidate;
            }
        }
    }
    if !template_path.exists() {
        return Err(format!("File not found: {}", display_path.display()));
    }
    Ok(template_path)
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
        "dcs-info" => Some(Ok(analyze_dcs_info(
            args,
            context,
            &WorkspaceSupportStateReader::new(context),
        ))),
        "dcs-validate" => Some(Ok(validate_dcs(args, context))),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::{json, Map};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    const TEST_DCS_SETTINGS_NS: &str = DCS_SETTINGS_NS;
    const TEST_DCS_CORE_NS: &str = DCS_CORE_NS;
    const TEST_DCS_COMMON_NS: &str = DCS_COMMON_NS;
    fn dcs_info_context(name: &str) -> (WorkspaceContext, PathBuf) {
        let context = temp_context(name);
        let source = context.workspace_root.join("src");
        let template_path = source.join("Reports/Sales/Templates/Dcs/Ext/Template.xml");
        fs::create_dir_all(template_path.parent().unwrap()).unwrap();
        fs::write(
            context.workspace_root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects><Report>Sales</Report></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            source.join("Reports/Sales.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Report><Properties><Name>Sales</Name></Properties><ChildObjects><Template>Dcs</Template></ChildObjects></Report></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            source.join("Reports/Sales/Templates/Dcs.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"><Properties><Name>Dcs</Name><TemplateType>DataCompositionSchema</TemplateType></Properties></Template></MetaDataObject>"#,
        )
        .unwrap();
        (context, template_path)
    }

    #[test]
    fn dcs_info_rejects_wrong_root_namespace_without_output() {
        let (context, template_path) = dcs_info_context("dcs-info-wrong-root-ns");
        let out_file = context.cwd.join("info.txt");
        fs::write(
            &template_path,
            base_dcs_xml().replace(
                "http://v8.1c.ru/8.1/data-composition-system/schema",
                "urn:not-dcs",
            ),
        )
        .unwrap();
        let args = Map::from_iter([
            ("TemplatePath".to_string(), json!(template_path)),
            ("OutFile".to_string(), json!("info.txt")),
        ]);

        let outcome =
            analyze_dcs_info(&args, &context, &WorkspaceSupportStateReader::new(&context));

        assert!(!outcome.ok, "{outcome:?}");
        assert!(!out_file.exists());
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("urn:not-dcs") && error.contains("DataCompositionSchema")));
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn dcs_info_raw_returns_exact_unpaginated_query_text() {
        let (context, template_path) = dcs_info_context("dcs-info-raw-query");
        let query = "  ВЫБРАТЬ 1 КАК Значение  \n|ОБЪЕДИНИТЬ ВСЕ\n|ВЫБРАТЬ 2  ";
        fs::write(
            &template_path,
            base_dcs_xml().replace(
                "<query>ВЫБРАТЬ Amount КАК Amount</query>",
                &format!("<query>{query}</query>"),
            ),
        )
        .unwrap();
        let args = Map::from_iter([
            ("TemplatePath".to_string(), json!(template_path)),
            ("Mode".to_string(), json!("query")),
            ("Name".to_string(), json!("НаборДанных1")),
            ("Raw".to_string(), json!(true)),
            ("Limit".to_string(), json!(1)),
            ("Offset".to_string(), json!(100)),
        ]);

        let execution = analyze_dcs_info_with_data(
            &args,
            &context,
            &WorkspaceSupportStateReader::new(&context),
        );

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        // `Raw` existed because pagination mangled the query; data carries the
        // exact text always, so Limit and Offset cannot truncate it and the
        // flag has nothing left to switch on.
        let data = execution.data.expect("dcs.info answers with data");
        let queries = data
            .data_sets
            .iter()
            .filter_map(|data_set| data_set.query.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(queries, vec![query], "{data:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    /// Support has its own structured field and is not duplicated in overview prose.

    #[test]
    fn dcs_info_overview_does_not_duplicate_structured_support() {
        let xml = complete_dcs_xml();
        let doc = Document::parse(xml).unwrap();
        let mut lines = Vec::new();

        dcs_info_overview(
            doc.root_element(),
            Path::new("Template.xml"),
            xml,
            &mut lines,
            DCS_SCHEMA_NS,
            TEST_DCS_SETTINGS_NS,
        );

        assert!(
            lines.iter().all(|line| !line.starts_with("Поддержка:")),
            "support belongs to the typed `support` field, not overview prose: {lines:?}"
        );
    }

    #[test]
    fn dcs_info_data_carries_every_fact_the_retired_modes_printed() {
        let (context, template_path) = dcs_info_context("dcs-info-complete-read-model");
        fs::write(&template_path, complete_dcs_xml()).unwrap();

        let execution = analyze_dcs_info_with_data(
            &Map::from_iter([("TemplatePath".to_string(), json!(template_path))]),
            &context,
            &WorkspaceSupportStateReader::new(&context),
        );

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        let data = execution.data.expect("dcs.info answers with data");

        // `overview` printed the owner's support state and the data sources.
        assert_eq!(
            data.support.state,
            crate::domain::support_state::ObjectSupportState::NotSupported,
            "{data:?}"
        );
        assert_eq!(
            data.data_sources
                .iter()
                .map(|source| (source.name.as_str(), source.kind.as_deref()))
                .collect::<Vec<_>>(),
            vec![("ИсточникДанных1", Some("Local"))],
            "{data:?}"
        );

        // `overview` distinguished an Object dataset by its objectName, and
        // every dataset named the source it reads.
        let object_set = data
            .data_sets
            .iter()
            .find(|set| set.kind == "Object")
            .expect("the fixture declares an Object dataset");
        assert_eq!(object_set.object_name.as_deref(), Some("Справочник.Товары"));
        assert_eq!(
            data.data_sets[0].data_source.as_deref(),
            Some("ИсточникДанных1")
        );

        // `resources` printed the grouping a total belongs to; without it an
        // overall total and a per-group total look identical.
        assert_eq!(
            data.total_fields
                .iter()
                .map(|total| (total.data_path.as_str(), total.group.as_deref()))
                .collect::<Vec<_>>(),
            vec![("Сумма", Some("Товар")), ("Количество", None)],
            "{data:?}"
        );

        // `calculated -Name` printed which uses a field is barred from; the
        // element is structured, so a boolean cannot carry that answer.
        assert_eq!(
            data.calculated_fields[0].restrictions.as_deref(),
            Some(&["condition".to_string(), "order".to_string()][..]),
            "{data:?}"
        );

        // `params` marked a parameter that cannot be used as a field.
        assert_eq!(
            data.parameters[0].available_as_field,
            Some(false),
            "{data:?}"
        );

        // `variant` printed the name, presentation, selection, filters and the
        // structure with its grouping fields.
        let variant = &data.variants[0];
        assert_eq!(variant.name, "Основной");
        assert_eq!(variant.presentation.as_deref(), Some("Основной вариант"));
        assert_eq!(variant.selection, vec!["Товар".to_string()]);
        assert_eq!(variant.filters, 1);
        assert_eq!(
            variant
                .order
                .iter()
                .map(|item| (item.field.as_str(), item.direction.as_deref()))
                .collect::<Vec<_>>(),
            vec![("Сумма", Some("Desc"))]
        );
        assert_eq!(
            variant
                .structure
                .iter()
                .map(|item| (item.kind.as_str(), item.group_by.clone()))
                .collect::<Vec<_>>(),
            vec![("Group", vec!["Товар".to_string()])]
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn dcs_info_structure_items_preserve_names_unnamed_parents_duplicates_and_axes() {
        let xml = r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema" xmlns:s="http://v8.1c.ru/8.1/data-composition-system/settings" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
        <settingsVariant><s:name>Variant</s:name><s:settings>
          <s:item xsi:type="s:StructureItemGroup"><s:item xsi:type="s:StructureItemGroup"><s:name>Nested</s:name></s:item></s:item>
          <s:item xsi:type="s:StructureItemGroup"/>
          <s:item xsi:type="s:StructureItemTable"><s:row><s:item xsi:type="s:StructureItemGroup"><s:name>Repeated</s:name></s:item></s:row><s:column><s:item xsi:type="s:StructureItemGroup"><s:name>Repeated</s:name></s:item></s:column></s:item>
          <s:item xsi:type="s:StructureItemChart"><s:point><s:item xsi:type="s:StructureItemGroup"><s:name>name with spaces</s:name></s:item></s:point><s:series><s:item xsi:type="s:StructureItemGroup"><s:name>Series</s:name></s:item></s:series></s:item>
          <s:selection><s:item xsi:type="s:SelectedItemField"><s:name>Decoy</s:name></s:item></s:selection>
          <s:item xsi:type="s:StructureItemNestedObject"><s:name>NestedObject</s:name><s:item xsi:type="s:StructureItemGroup"><s:name>NestedChild</s:name></s:item></s:item>
          <s:item xsi:type="s:StructureItemFuture"><s:name>Unknown</s:name><s:item xsi:type="s:StructureItemGroup"><s:name>FutureChild</s:name></s:item></s:item>
        </s:settings></settingsVariant></DataCompositionSchema>"#;
        let data = parse_dcs_info_xml(
            xml,
            DomainObjectSupportData {
                state: crate::domain::support_state::ObjectSupportState::NotSupported,
                direct_edit_safe: None,
            },
        )
        .unwrap();
        let value = serde_json::to_value(data).unwrap();
        let rows = value["variants"][0]["structureItems"]
            .as_array()
            .expect("structure names must be reachable without reading XML");
        assert_eq!(
            rows.iter()
                .map(|row| row["name"].clone())
                .collect::<Vec<_>>(),
            vec![
                json!(null),
                json!("Nested"),
                json!(null),
                json!(null),
                json!("Repeated"),
                json!("Repeated"),
                json!(null),
                json!("name with spaces"),
                json!("Series"),
                json!("NestedObject"),
                json!("NestedChild"),
                json!("Unknown"),
                json!("FutureChild")
            ]
        );
        assert_eq!(rows[1]["parentIndex"], json!(0));
        assert_eq!(rows[4]["parentIndex"], json!(3));
        assert_eq!(rows[4]["axis"], json!("row"));
        assert_eq!(rows[5]["axis"], json!("column"));
        assert_eq!(rows[7]["axis"], json!("point"));
        assert_eq!(rows[8]["axis"], json!("series"));
        assert_eq!(rows[9]["kind"], json!("NestedObject"));
        assert_eq!(rows[10]["parentIndex"], json!(9));
        assert_eq!(rows[11]["kind"], json!("StructureItemFuture"));
        assert_eq!(rows[12]["parentIndex"], json!(11));
        assert!(rows.iter().all(|row| row.get("at").is_none()));
    }

    #[test]
    fn verified_dcs_input_is_validated_without_reopening_the_artifact_path() {
        let root = tempfile::tempdir().unwrap();
        let artifact = root.path().join("Template.xml");
        let replacement = b"not the retained DCS input";
        fs::write(&artifact, replacement).unwrap();
        let result = validate_dcs_input(&Map::new(), base_dcs_xml(), artifact.clone());
        assert!(result.ok, "{result:?}");
        assert_eq!(fs::read(artifact).unwrap(), replacement);
    }

    /// #311. A `SettingsParameterValue` with an empty `<dcscor:parameter>` is
    /// well-formed XML that names no parameter, so the variant carries no value
    /// for it. Validation reported `Validation OK` on exactly that shape.

    #[test]
    fn native_dcs_validate_rejects_a_data_parameter_without_a_name() {
        let context = temp_context("dcs-validate-empty-data-parameter");
        let template_path = context.cwd.join("Template.xml");
        fs::write(
            &template_path,
            base_dcs_xml().replace(
                "\t\t\t<dcsset:order>",
                "\t\t\t<dcsset:dataParameters>\n\t\t\t\t<dcscor:item xsi:type=\"dcsset:SettingsParameterValue\">\n\t\t\t\t\t<dcscor:parameter></dcscor:parameter>\n\t\t\t\t</dcscor:item>\n\t\t\t</dcsset:dataParameters>\n\t\t\t<dcsset:order>",
            ),
        )
        .unwrap();

        let mut args = Map::new();
        args.insert("TemplatePath".to_string(), json!("Template.xml"));
        let outcome = validate_dcs(&args, &context);

        let stdout = outcome.stdout.unwrap_or_default();
        assert!(!outcome.ok, "{stdout}");
        assert!(
            stdout.contains("SettingsParameterValue has empty parameter name"),
            "{stdout}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn native_dcs_validate_rejects_ref_type_bound_to_unexpected_namespace() {
        let context = temp_context("dcs-validate-bad-prefix");
        let template_path = context.cwd.join("Template.xml");
        fs::write(
            &template_path,
            base_dcs_xml().replace(
                "<field>Amount</field>",
                "<field>Amount</field>\n\t\t\t<valueType>\n\t\t\t\t<v8:Type xmlns:bad=\"http://example.com\">bad:CatalogRef.X</v8:Type>\n\t\t\t</valueType>",
            ),
        )
        .unwrap();

        let mut args = Map::new();
        args.insert("TemplatePath".to_string(), json!("Template.xml"));
        let outcome = validate_dcs(&args, &context);
        let stdout = outcome.stdout.unwrap_or_default();
        assert!(!outcome.ok, "{stdout}");
        assert!(
            stdout.contains("uses prefix 'bad' bound to unexpected namespace 'http://example.com'"),
            "{stdout}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    fn temp_context(name: &str) -> WorkspaceContext {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cwd = std::env::temp_dir().join(format!("unica-{name}-{nanos}"));
        fs::create_dir_all(&cwd).unwrap();
        WorkspaceContext {
            cwd: cwd.clone(),
            workspace_root: cwd.clone(),
            cache_root: cwd.join(".build/unica"),
            workspace_epoch: 0,
        }
    }

    /// One schema carrying a representative of every fact the retired `Mode`
    /// reports used to print. The settings elements deliberately sit in the
    /// `dcsset` namespace, which is where the platform puts them.

    fn complete_dcs_xml() -> &'static str {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"
		xmlns:dcscor="http://v8.1c.ru/8.1/data-composition-system/core"
		xmlns:dcsset="http://v8.1c.ru/8.1/data-composition-system/settings"
		xmlns:xs="http://www.w3.org/2001/XMLSchema"
		xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
	<dataSource>
		<name>ИсточникДанных1</name>
		<dataSourceType>Local</dataSourceType>
	</dataSource>
	<dataSet xsi:type="DataSetQuery">
		<name>НаборЗапрос</name>
		<field xsi:type="DataSetFieldField">
			<dataPath>Товар</dataPath>
			<field>Товар</field>
		</field>
		<dataSource>ИсточникДанных1</dataSource>
		<query>ВЫБРАТЬ Товар КАК Товар</query>
	</dataSet>
	<dataSet xsi:type="DataSetObject">
		<name>НаборОбъект</name>
		<objectName>Справочник.Товары</objectName>
		<field xsi:type="DataSetFieldField">
			<dataPath>Наименование</dataPath>
			<field>Наименование</field>
		</field>
	</dataSet>
	<calculatedField>
		<dataPath>Наценка</dataPath>
		<expression>Сумма - Себестоимость</expression>
		<useRestriction>
			<condition>true</condition>
			<order>true</order>
		</useRestriction>
	</calculatedField>
	<totalField>
		<dataPath>Сумма</dataPath>
		<expression>Сумма(Сумма)</expression>
		<group>Товар</group>
	</totalField>
	<totalField>
		<dataPath>Количество</dataPath>
		<expression>Сумма(Количество)</expression>
	</totalField>
	<parameter>
		<name>Период</name>
		<availableAsField>false</availableAsField>
	</parameter>
	<settingsVariant>
		<dcsset:name>Основной</dcsset:name>
		<dcsset:presentation xsi:type="xs:string">Основной вариант</dcsset:presentation>
		<dcsset:settings>
			<dcsset:selection>
				<dcsset:item xsi:type="dcsset:SelectedItemField">
					<dcsset:field>Товар</dcsset:field>
				</dcsset:item>
			</dcsset:selection>
			<dcsset:filter>
				<dcsset:item xsi:type="dcsset:FilterItemComparison">
					<dcsset:left xsi:type="dcscor:Field">Товар</dcsset:left>
					<dcsset:comparisonType>Equal</dcsset:comparisonType>
				</dcsset:item>
			</dcsset:filter>
			<dcsset:order>
				<dcsset:item xsi:type="dcsset:OrderItemField">
					<dcsset:field>Сумма</dcsset:field>
					<dcsset:orderType>Desc</dcsset:orderType>
				</dcsset:item>
			</dcsset:order>
			<dcsset:item xsi:type="dcsset:StructureItemGroup">
				<dcsset:groupItems>
					<dcsset:item xsi:type="dcsset:GroupItemField">
						<dcsset:field>Товар</dcsset:field>
					</dcsset:item>
				</dcsset:groupItems>
			</dcsset:item>
		</dcsset:settings>
	</settingsVariant>
</DataCompositionSchema>
"#
    }

    fn base_dcs_xml() -> &'static str {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"
		xmlns:dcscom="http://v8.1c.ru/8.1/data-composition-system/common"
		xmlns:dcscor="http://v8.1c.ru/8.1/data-composition-system/core"
		xmlns:dcsset="http://v8.1c.ru/8.1/data-composition-system/settings"
		xmlns:v8="http://v8.1c.ru/8.1/data/core"
		xmlns:v8ui="http://v8.1c.ru/8.1/data/ui"
		xmlns:xs="http://www.w3.org/2001/XMLSchema"
		xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
	<dataSource>
		<name>ИсточникДанных1</name>
		<dataSourceType>Local</dataSourceType>
	</dataSource>
	<dataSet xsi:type="DataSetQuery">
		<name>НаборДанных1</name>
		<field xsi:type="DataSetFieldField">
			<dataPath>Amount</dataPath>
			<field>Amount</field>
		</field>
		<dataSource>ИсточникДанных1</dataSource>
		<query>ВЫБРАТЬ Amount КАК Amount</query>
	</dataSet>
	<settingsVariant>
		<dcsset:name>Основной</dcsset:name>
		<dcsset:settings>
			<dcsset:selection>
			</dcsset:selection>
			<dcsset:filter>
			</dcsset:filter>
			<dcsset:order>
			</dcsset:order>
			<dcsset:item xsi:type="dcsset:StructureItemGroup">
				<dcsset:selection>
					<dcsset:item xsi:type="dcsset:SelectedItemAuto"/>
				</dcsset:selection>
			</dcsset:item>
		</dcsset:settings>
	</settingsVariant>
</DataCompositionSchema>
"#
    }
}
