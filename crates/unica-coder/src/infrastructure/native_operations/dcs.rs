#![allow(dead_code, unused_imports)]

use crate::application::operation_descriptors::TEMPLATE_PATH;
use crate::application::AdapterOutcome;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform_xml_owner::DCS_ROOT;
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::common::*;
use super::compile_transaction::CompileTransaction;
use super::{
    cf::*, cfe::*, form::*, interface::*, meta::*, mxl::*, role::*, subsystem::*, template::*,
};

pub(crate) const DCS_SCHEMA_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
pub(crate) const DCS_SETTINGS_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/settings";
pub(crate) const DCS_CORE_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/core";
pub(crate) const DCS_COMMON_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/common";
pub(crate) const V8_DATA_NS: &str = "http://v8.1c.ru/8.1/data/core";
pub(crate) const XML_SCHEMA_INSTANCE_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

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
    pub(crate) support: ObjectSupportData,
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
    resolved_path: &Path,
) -> DcsInfoData {
    DcsInfoData {
        support: object_support_state(resolved_path),
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

pub(crate) fn analyze_dcs_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    analyze_dcs_info_with_data(args, context).outcome
}

pub(crate) fn analyze_dcs_info_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> DcsInfoExecution {
    const NS_SCHEMA: &str = DCS_SCHEMA_NS;
    const NS_SETTINGS: &str = "http://v8.1c.ru/8.1/data-composition-system/settings";

    let result = (|| -> Result<(DcsInfoData, PathBuf), String> {
        let template_path = resolve_dcs_info_path_for_script(args, context)?;
        let resolved_path = template_path
            .canonicalize()
            .unwrap_or_else(|_| template_path.clone());
        let text = read_utf8_sig(&resolved_path)?;
        let doc = Document::parse(text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("XML parse error in {}: {err}", resolved_path.display()))?;
        let root = doc.root_element();
        require_dcs_root(root)?;
        // Eleven modes sliced one schema into eleven reports. Data answers
        // with every section at once; a caller projects what it needs.
        let data = dcs_info_collect(root, NS_SCHEMA, NS_SETTINGS, &resolved_path);
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
    lines.push(format!(
        "Поддержка: {}",
        support_status_for_path(resolved_path)
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
    let xsi_type = attribute_by_local_name(data_set, "type").unwrap_or("");
    if xsi_type.contains("DataSetQuery") {
        "Query".to_string()
    } else if xsi_type.contains("DataSetObject") {
        "Object".to_string()
    } else if xsi_type.contains("DataSetUnion") {
        "Union".to_string()
    } else {
        "Unknown".to_string()
    }
}

pub(crate) fn dcs_info_structure_item_type(item: roxmltree::Node<'_, '_>) -> &'static str {
    let xsi_type = attribute_by_local_name(item, "type").unwrap_or("");
    if xsi_type.contains("StructureItemGroup") {
        "Group"
    } else if xsi_type.contains("StructureItemTable") {
        "Table"
    } else if xsi_type.contains("StructureItemChart") {
        "Chart"
    } else {
        "Unknown"
    }
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
    const NS_SCHEMA: &str = DCS_SCHEMA_NS;

    let result = (|| -> Result<DcsValidationRun, String> {
        let template_path = resolve_dcs_validate_path(args, context)?;
        let resolved_path = template_path
            .canonicalize()
            .unwrap_or_else(|_| template_path.clone());
        let file_name = resolved_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_string();
        let detailed = bool_arg(args, &["detailed", "Detailed"]);
        let max_errors = int_arg(args, &["maxErrors", "MaxErrors"])
            .unwrap_or(20)
            .max(0) as usize;

        let text = read_utf8_sig(&resolved_path)?;
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
    })();

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
    node.text().unwrap_or("").to_string()
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

pub(crate) fn compile_dcs(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    let write_result = (|| -> Result<(String, PathBuf, Vec<String>, Vec<String>), String> {
        let definition_file = path_arg(args, &["definitionFile", "DefinitionFile"]);
        let value = string_arg(args, &["value", "Value"]);
        if definition_file.is_some() && value.is_some() {
            return Err("Cannot use both -DefinitionFile and -Value".to_string());
        }
        if definition_file.is_none() && value.is_none() {
            return Err("Either -DefinitionFile or -Value is required".to_string());
        }

        let output_path_label = string_arg(args, &["outputPath", "OutputPath"])
            .ok_or_else(|| "missing required OutputPath argument".to_string())?
            .to_string();
        let output_path = absolutize(PathBuf::from(&output_path_label), &context.cwd);
        let show_validation = !bool_arg(args, &["noValidate", "NoValidate"]);

        let mut transaction = CompileTransaction::new();
        let (mut defn, query_base_dir) = if let Some(definition_file) = definition_file {
            let definition_file = absolutize(definition_file, &context.cwd);
            if !definition_file.exists() {
                return Err(format!(
                    "Definition file not found: {}",
                    definition_file.display()
                ));
            }
            let base_dir = definition_file
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| context.cwd.clone());
            let definition = FileBackedJson::read(&definition_file, |err| {
                format!("failed to parse DCS JSON: {err}")
            })?
            .bind_to(&mut transaction)?;
            (definition, base_dir)
        } else {
            let definition = serde_json::from_str(value.unwrap_or(""))
                .map_err(|err| format!("failed to parse DCS JSON: {err}"))?;
            (definition, context.cwd.clone())
        };

        {
            let Some(data_sets) = defn.get_mut("dataSets").and_then(Value::as_array_mut) else {
                return Err("JSON must have at least one entry in 'dataSets'".to_string());
            };
            if data_sets.is_empty() {
                return Err("JSON must have at least one entry in 'dataSets'".to_string());
            }
            for (index, data_set) in data_sets.iter_mut().enumerate() {
                if data_set
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .is_none()
                {
                    if let Some(object) = data_set.as_object_mut() {
                        object.insert(
                            "name".to_string(),
                            Value::String(format!("НаборДанных{}", index + 1)),
                        );
                    }
                }
            }
        }

        let mut query_inputs = Vec::new();
        let content =
            dcs_compile_xml_with_inputs(&defn, &query_base_dir, &context.cwd, &mut query_inputs)?;
        for input in &query_inputs {
            input.bind_to(&mut transaction)?;
        }
        let replacement = utf8_bom_bytes(&content);
        let file_size = replacement.len();

        let empty_data_sets = Vec::new();
        let data_sets = defn
            .get("dataSets")
            .and_then(Value::as_array)
            .unwrap_or(&empty_data_sets);
        let ds_count = data_sets.len();
        let field_count = data_sets
            .iter()
            .map(|data_set| {
                data_set
                    .get("fields")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0)
            })
            .sum::<usize>();
        let calc_count = defn
            .get("calculatedFields")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let total_count = defn
            .get("totalFields")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let param_count = defn
            .get("parameters")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let variant_count = defn
            .get("settingsVariants")
            .and_then(Value::as_array)
            .filter(|items| !items.is_empty())
            .map(Vec::len)
            .unwrap_or(1);
        let mut stdout = format!(
            "OK  {output_path_label}\n    DataSets: {ds_count}  Fields: {field_count}  Calculated: {calc_count}  Totals: {total_count}  Params: {param_count}  Variants: {variant_count}\n    Size: {file_size} bytes\n"
        );

        transaction.create_or_replace_bytes(&output_path, replacement)?;
        guard_active_format_owner_with_exact_root(
            &mut transaction,
            &output_path,
            context,
            DCS_ROOT,
        )?;
        let mut validation_stdout = None;
        let report = transaction.commit_with_post_validation(|| {
            let validation = require_dcs_post_validation(&output_path, context)?;
            validation_stdout = Some(validation);
            Ok(())
        })?;

        if show_validation {
            stdout.push_str("\n--- Running dcs-validate ---\n");
            if let Some(validation) = validation_stdout {
                stdout.push_str(&validation);
            }
        }

        let mut changes = report
            .created
            .iter()
            .map(|path| format!("created {}", path.display()))
            .collect::<Vec<_>>();
        changes.extend(
            report
                .updated
                .iter()
                .map(|path| format!("updated {}", path.display())),
        );
        Ok((stdout, output_path, changes, report.cleanup_warnings))
    })();

    match write_result {
        Ok((stdout, output_path, changes, warnings)) => AdapterOutcome {
            ok: true,
            summary: "unica.dcs.compile completed with native DCS compiler".to_string(),
            changes,
            warnings,
            errors: Vec::new(),
            artifacts: vec![output_path.display().to_string()],
            stdout: Some(stdout),
            stderr: None,
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.dcs.compile failed in native DCS compiler".to_string(),
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

fn require_dcs_post_validation(
    template_path: &Path,
    context: &WorkspaceContext,
) -> Result<String, String> {
    let validation_args = Map::from_iter([(
        "TemplatePath".to_string(),
        Value::String(template_path.display().to_string()),
    )]);
    let outcome = validate_dcs(&validation_args, context);
    let stdout = outcome.stdout.unwrap_or_default();
    if outcome.ok {
        return Ok(stdout);
    }
    let detail = if outcome.errors.is_empty() {
        stdout.trim().to_string()
    } else {
        outcome.errors.join("; ")
    };
    Err(format!(
        "DCS validation failed for {}: {}",
        template_path.display(),
        if detail.is_empty() {
            "validation returned no diagnostics"
        } else {
            &detail
        }
    ))
}

pub(crate) fn dcs_compile_xml(
    defn: &Value,
    query_base_dir: &Path,
    cwd: &Path,
) -> Result<String, String> {
    dcs_compile_xml_with_inputs(defn, query_base_dir, cwd, &mut Vec::new())
}

fn dcs_compile_xml_with_inputs(
    defn: &Value,
    query_base_dir: &Path,
    cwd: &Path,
    query_inputs: &mut Vec<ExactFileInput>,
) -> Result<String, String> {
    let mut query_context = DcsCompileQueryContext {
        query_base_dir,
        cwd,
        inputs: query_inputs,
    };
    let data_sources = dcs_compile_data_sources(defn);
    let default_source = data_sources
        .first()
        .map(|source| source.0.clone())
        .unwrap_or_else(|| "ИсточникДанных1".to_string());
    let mut lines = vec![
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string(),
        "<DataCompositionSchema xmlns=\"http://v8.1c.ru/8.1/data-composition-system/schema\" xmlns:dcscom=\"http://v8.1c.ru/8.1/data-composition-system/common\" xmlns:dcscor=\"http://v8.1c.ru/8.1/data-composition-system/core\" xmlns:dcsset=\"http://v8.1c.ru/8.1/data-composition-system/settings\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">".to_string(),
    ];

    for (name, source_type) in &data_sources {
        lines.push("\t<dataSource>".to_string());
        lines.push(format!("\t\t<name>{}</name>", escape_xml(name)));
        lines.push(format!(
            "\t\t<dataSourceType>{}</dataSourceType>",
            escape_xml(source_type)
        ));
        lines.push("\t</dataSource>".to_string());
    }

    if let Some(data_sets) = defn.get("dataSets").and_then(Value::as_array) {
        for data_set in data_sets {
            dcs_compile_emit_data_set(
                &mut lines,
                data_set,
                "\t",
                &default_source,
                &mut query_context,
            )?;
        }
    }

    dcs_compile_emit_data_set_links(&mut lines, defn);
    dcs_compile_emit_calculated_fields(&mut lines, defn)?;
    dcs_compile_emit_total_fields(&mut lines, defn);
    dcs_compile_emit_parameters(&mut lines, defn)?;
    dcs_compile_emit_settings_variants(&mut lines, defn);
    lines.push("</DataCompositionSchema>".to_string());
    Ok(format!("{}\n", lines.join("\n")))
}

pub(crate) fn dcs_compile_data_sources(defn: &Value) -> Vec<(String, String)> {
    if let Some(items) = defn.get("dataSources").and_then(Value::as_array) {
        let mut result = Vec::new();
        for item in items {
            let name = json_string_field(item, "name").unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let source_type =
                json_string_field(item, "type").unwrap_or_else(|| "Local".to_string());
            result.push((name, source_type));
        }
        if !result.is_empty() {
            return result;
        }
    }
    vec![("ИсточникДанных1".to_string(), "Local".to_string())]
}

pub(crate) struct DcsCompileQueryContext<'a> {
    query_base_dir: &'a Path,
    cwd: &'a Path,
    inputs: &'a mut Vec<ExactFileInput>,
}

pub(crate) fn dcs_compile_emit_data_set(
    lines: &mut Vec<String>,
    data_set: &Value,
    indent: &str,
    default_source: &str,
    query_context: &mut DcsCompileQueryContext<'_>,
) -> Result<(), String> {
    dcs_compile_emit_data_set_element(
        lines,
        data_set,
        indent,
        "dataSet",
        default_source,
        query_context,
    )
}

pub(crate) fn dcs_compile_emit_data_set_element(
    lines: &mut Vec<String>,
    data_set: &Value,
    indent: &str,
    element_name: &str,
    default_source: &str,
    query_context: &mut DcsCompileQueryContext<'_>,
) -> Result<(), String> {
    let ds_type = if data_set.get("items").is_some() {
        "DataSetUnion"
    } else if data_set.get("objectName").is_some() {
        "DataSetObject"
    } else {
        "DataSetQuery"
    };
    lines.push(format!("{indent}<{element_name} xsi:type=\"{ds_type}\">"));
    lines.push(format!(
        "{indent}\t<name>{}</name>",
        escape_xml(&json_string_field(data_set, "name").unwrap_or_default())
    ));
    if let Some(fields) = data_set.get("fields").and_then(Value::as_array) {
        for field in fields {
            dcs_compile_emit_field(
                lines,
                field,
                &format!("{indent}\t"),
                ds_type != "DataSetQuery",
            )?;
        }
    }
    if ds_type != "DataSetUnion" {
        let source =
            json_string_field(data_set, "source").unwrap_or_else(|| default_source.to_string());
        lines.push(format!(
            "{indent}\t<dataSource>{}</dataSource>",
            escape_xml(&source)
        ));
    }
    match ds_type {
        "DataSetQuery" => {
            let query = json_string_field(data_set, "query").unwrap_or_default();
            let query = dcs_compile_resolve_query_value_with_inputs(
                &query,
                query_context.query_base_dir,
                query_context.cwd,
                query_context.inputs,
            )?;
            lines.push(format!("{indent}\t<query>{}</query>", escape_xml(&query)));
            if data_set
                .get("autoFillFields")
                .and_then(Value::as_bool)
                .is_some_and(|value| !value)
            {
                lines.push(format!("{indent}\t<autoFillFields>false</autoFillFields>"));
            }
        }
        "DataSetObject" => {
            let object_name = json_string_field(data_set, "objectName").unwrap_or_default();
            lines.push(format!(
                "{indent}\t<objectName>{}</objectName>",
                escape_xml(&object_name)
            ));
        }
        "DataSetUnion" => {
            if let Some(items) = data_set.get("items").and_then(Value::as_array) {
                for item in items {
                    dcs_compile_emit_data_set_element(
                        lines,
                        item,
                        &format!("{indent}\t"),
                        "item",
                        default_source,
                        query_context,
                    )?;
                }
            }
        }
        _ => {}
    }
    lines.push(format!("{indent}</{element_name}>"));
    Ok(())
}

pub(crate) fn dcs_compile_emit_field(
    lines: &mut Vec<String>,
    field: &Value,
    indent: &str,
    emit_value_type: bool,
) -> Result<(), String> {
    let (data_path, field_name, title, field_type, presentation_expression, type_declared) =
        if let Some(text) = field.as_str() {
            let parsed = dcs_compile_parse_field_shorthand(text);
            (
                parsed.0.clone(),
                parsed.1,
                String::new(),
                dcs_compile_resolve_type(&parsed.2),
                String::new(),
                parsed.3,
            )
        } else {
            let data_path = json_string_field(field, "dataPath")
                .or_else(|| json_string_field(field, "field"))
                .unwrap_or_default();
            let field_name = json_string_field(field, "field").unwrap_or_else(|| data_path.clone());
            let title = json_string_field(field, "title").unwrap_or_default();
            let field_type = field
                .get("type")
                .map(dcs_compile_type_value)
                .unwrap_or_default();
            let presentation_expression =
                json_string_field(field, "presentationExpression").unwrap_or_default();
            (
                data_path,
                field_name,
                title,
                field_type,
                presentation_expression,
                field.get("type").is_some(),
            )
        };

    let value_type_entries = type_declared
        .then(|| dcs_compile_parse_value_type(&field_type))
        .transpose()?;

    lines.push(format!("{indent}<field xsi:type=\"DataSetFieldField\">"));
    lines.push(format!(
        "{indent}\t<dataPath>{}</dataPath>",
        escape_xml(&data_path)
    ));
    lines.push(format!(
        "{indent}\t<field>{}</field>",
        escape_xml(&field_name)
    ));
    if !title.is_empty() {
        dcs_compile_emit_mltext(lines, &format!("{indent}\t"), "title", &title);
    }
    dcs_compile_emit_restriction(
        lines,
        field,
        "restrict",
        "useRestriction",
        &format!("{indent}\t"),
    );
    dcs_compile_emit_restriction(
        lines,
        field,
        "attrRestrict",
        "attributeUseRestriction",
        &format!("{indent}\t"),
    );
    if !presentation_expression.is_empty() {
        lines.push(format!(
            "{indent}\t<presentationExpression>{}</presentationExpression>",
            escape_xml(&presentation_expression)
        ));
    }
    if emit_value_type && value_type_entries.is_some() {
        lines.push(format!("{indent}\t<valueType>"));
        dcs_compile_emit_value_type_entries(
            lines,
            value_type_entries.as_deref().unwrap_or_default(),
            &format!("{indent}\t\t"),
        );
        lines.push(format!("{indent}\t</valueType>"));
    }
    lines.push(format!("{indent}</field>"));
    Ok(())
}

pub(crate) fn dcs_compile_parse_field_shorthand(text: &str) -> (String, String, String, bool) {
    let value = text
        .split_whitespace()
        .filter(|part| !part.starts_with('@') && !part.starts_with('#'))
        .collect::<Vec<_>>()
        .join(" ");
    let value = value.trim();
    if let Some((left, right)) = value.split_once(':') {
        let data_path = left.trim().to_string();
        (
            data_path.clone(),
            data_path,
            dcs_compile_resolve_type(right.trim()),
            true,
        )
    } else {
        (value.to_string(), value.to_string(), String::new(), false)
    }
}

pub(crate) fn dcs_compile_emit_restriction(
    lines: &mut Vec<String>,
    value: &Value,
    source_key: &str,
    tag_name: &str,
    indent: &str,
) {
    dcs_compile_emit_merged_restriction(lines, value, &[source_key], tag_name, indent);
}

pub(crate) fn dcs_compile_emit_merged_restriction(
    lines: &mut Vec<String>,
    value: &Value,
    source_keys: &[&str],
    tag_name: &str,
    indent: &str,
) {
    let mut enabled = [false; 4];
    for source_key in source_keys {
        let Some(items) = dcs_compile_string_items(value.get(*source_key)) else {
            continue;
        };
        for item in items {
            let index = match item.as_str() {
                "noField" | "field" => Some(0),
                "noFilter" | "noCondition" | "condition" => Some(1),
                "noGroup" | "group" => Some(2),
                "noOrder" | "order" => Some(3),
                _ => None,
            };
            if let Some(index) = index {
                enabled[index] = true;
            }
        }
    }
    if !enabled.iter().any(|enabled| *enabled) {
        return;
    }
    lines.push(format!("{indent}<{tag_name}>"));
    for (xml_name, enabled) in ["field", "condition", "group", "order"]
        .into_iter()
        .zip(enabled)
    {
        if enabled {
            lines.push(format!("{indent}\t<{xml_name}>true</{xml_name}>"));
        }
    }
    lines.push(format!("{indent}</{tag_name}>"));
}

pub(crate) fn dcs_compile_string_items(value: Option<&Value>) -> Option<Vec<String>> {
    let value = value?;
    if let Some(items) = value.as_array() {
        return Some(items.iter().map(json_value_to_python_string).collect());
    }
    if let Some(text) = value.as_str() {
        return Some(
            text.split_whitespace()
                .map(|item| item.trim_start_matches('#').to_string())
                .filter(|item| !item.is_empty())
                .collect(),
        );
    }
    if let Some(object) = value.as_object() {
        return Some(
            object
                .iter()
                .filter(|(_, value)| value.as_bool().unwrap_or(false))
                .map(|(key, _)| key.to_string())
                .collect(),
        );
    }
    Some(vec![json_value_to_python_string(value)])
}

pub(crate) fn dcs_compile_type_value(value: &Value) -> String {
    if let Some(items) = value.as_array() {
        return items
            .iter()
            .map(dcs_compile_type_value)
            .collect::<Vec<_>>()
            .join("|");
    }
    json_value_to_python_string(value)
        .split('|')
        .map(str::trim)
        .map(dcs_compile_resolve_type)
        .collect::<Vec<_>>()
        .join("|")
}

pub(crate) fn dcs_compile_resolve_type(type_str: &str) -> String {
    if type_str.is_empty() {
        return String::new();
    }
    if let Some(open) = type_str.find('(') {
        if type_str.ends_with(')') {
            let base = type_str[..open].trim();
            let params = &type_str[open + 1..type_str.len() - 1];
            if let Some(resolved) = dcs_compile_type_synonym(base) {
                return format!("{resolved}({params})");
            }
        }
    }
    if let Some(dot_idx) = type_str.find('.') {
        let prefix = &type_str[..dot_idx];
        if let Some(resolved) = dcs_compile_type_synonym(prefix) {
            return format!("{resolved}{}", &type_str[dot_idx..]);
        }
    }
    dcs_compile_type_synonym(type_str)
        .unwrap_or(type_str)
        .to_string()
}

pub(crate) fn dcs_compile_type_synonym(type_str: &str) -> Option<&'static str> {
    match type_str.to_lowercase().as_str() {
        "число" | "decimal" | "int" | "integer" | "number" | "num" => Some("decimal"),
        "bool" | "boolean" => Some("boolean"),
        "строка" | "str" | "string" => Some("string"),
        "булево" => Some("boolean"),
        "дата" | "date" => Some("date"),
        "датавремя" | "datetime" => Some("dateTime"),
        "время" | "time" => Some("time"),
        "стандартныйпериод" | "standardperiod" => Some("StandardPeriod"),
        "справочникссылка" => Some("CatalogRef"),
        "документссылка" => Some("DocumentRef"),
        "перечислениессылка" => Some("EnumRef"),
        "плансчетовссылка" => Some("ChartOfAccountsRef"),
        "планвидовхарактеристикссылка" => {
            Some("ChartOfCharacteristicTypesRef")
        }
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum DcsTypeNodeKind {
    Type,
    TypeSet,
    TypeId,
}

impl DcsTypeNodeKind {
    fn tag(self) -> &'static str {
        match self {
            Self::Type => "Type",
            Self::TypeSet => "TypeSet",
            Self::TypeId => "TypeId",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DcsTypeQualifier {
    Number {
        digits: u32,
        fraction: u32,
        nonnegative: bool,
    },
    String {
        length: u32,
        fixed: bool,
    },
    Date(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DcsTypeEntry {
    kind: DcsTypeNodeKind,
    wire_name: String,
    configuration_namespace: bool,
    qualifier: Option<DcsTypeQualifier>,
}

pub(crate) fn dcs_compile_emit_value_type(
    lines: &mut Vec<String>,
    type_spec: &str,
    indent: &str,
) -> Result<(), String> {
    let entries = dcs_compile_parse_value_type(type_spec)?;
    dcs_compile_emit_value_type_entries(lines, &entries, indent);
    Ok(())
}

pub(crate) fn dcs_compile_parse_value_type(type_spec: &str) -> Result<Vec<DcsTypeEntry>, String> {
    let raw_parts = type_spec.split('|').collect::<Vec<_>>();
    if raw_parts.is_empty() || raw_parts.iter().any(|part| part.trim().is_empty()) {
        return Err(format!(
            "DCS type '{type_spec}' is not valid for 8.3.27: composite type contains an empty item"
        ));
    }

    let entries = raw_parts
        .iter()
        .map(|part| dcs_compile_parse_type_entry(part.trim()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("DCS type '{type_spec}' is not valid for 8.3.27: {error}"))?;

    let mut seen = BTreeMap::<(DcsTypeNodeKind, String), &str>::new();
    for (raw, entry) in raw_parts.iter().zip(&entries) {
        let key = (entry.kind, entry.wire_name.clone());
        if let Some(previous) = seen.insert(key, raw.trim()) {
            return Err(format!(
                "DCS type '{type_spec}' is not valid for 8.3.27: duplicate platform type '{previous}' and '{}' both map to v8:{} {}",
                raw.trim(),
                entry.kind.tag(),
                entry.wire_name
            ));
        }
    }

    Ok(entries)
}

pub(crate) fn dcs_compile_parse_type_entry(type_name: &str) -> Result<DcsTypeEntry, String> {
    let normalized = dcs_compile_resolve_type(type_name);
    if normalized == "boolean" {
        return Ok(dcs_type_entry(DcsTypeNodeKind::Type, "xs:boolean", false));
    }
    if normalized == "StandardPeriod" {
        return Ok(dcs_type_entry(
            DcsTypeNodeKind::Type,
            "v8:StandardPeriod",
            false,
        ));
    }
    if normalized == "string" {
        return Ok(dcs_type_qualified_entry(
            "xs:string",
            DcsTypeQualifier::String {
                length: 0,
                fixed: false,
            },
        ));
    }
    if normalized.starts_with("string(") {
        let (length, fixed) = parse_form_string_contract(&normalized).ok_or_else(|| {
            format!(
                "type '{type_name}' must be string(integer length 0..=1024[,fixed|variable]); fixed requires length > 0"
            )
        })?;
        return Ok(dcs_type_qualified_entry(
            "xs:string",
            DcsTypeQualifier::String { length, fixed },
        ));
    }
    if normalized == "decimal" {
        return Ok(dcs_type_qualified_entry(
            "xs:decimal",
            DcsTypeQualifier::Number {
                digits: 10,
                fraction: 2,
                nonnegative: false,
            },
        ));
    }
    if normalized.starts_with("decimal(") {
        let (digits, fraction, nonnegative) =
            dcs_compile_parse_decimal_contract(&normalized).ok_or_else(|| {
                format!(
                    "type '{type_name}' must be decimal(integer digits 0..=38[, integer fraction 0..=digits][,nonneg])"
                )
            })?;
        return Ok(dcs_type_qualified_entry(
            "xs:decimal",
            DcsTypeQualifier::Number {
                digits,
                fraction,
                nonnegative,
            },
        ));
    }
    if matches!(normalized.as_str(), "date" | "dateTime" | "time") {
        let fractions = match normalized.as_str() {
            "date" => "Date",
            "dateTime" => "DateTime",
            "time" => "Time",
            _ => unreachable!(),
        };
        return Ok(dcs_type_qualified_entry(
            "xs:dateTime",
            DcsTypeQualifier::Date(fractions),
        ));
    }
    if let Some(type_id) = normalized.strip_prefix("typeid:") {
        if !is_valid_uuid(type_id) {
            return Err(format!("type '{type_name}' has an invalid TypeId UUID"));
        }
        return Ok(dcs_type_entry(
            DcsTypeNodeKind::TypeId,
            &type_id.to_ascii_lowercase(),
            false,
        ));
    }
    if normalized.starts_with("DefinedType.") {
        dcs_compile_validate_configuration_type(type_name, &normalized)?;
        return Err(format!(
            "type '{type_name}' is not supported by the fixed 8.3.27 DCS contract: platform 8.3.27 removes DefinedType.* from valueType during round-trip; use the defined type's expanded constituent types"
        ));
    }
    if normalized.starts_with("Characteristic.") {
        dcs_compile_validate_configuration_type(type_name, &normalized)?;
        return Ok(dcs_type_entry(
            DcsTypeNodeKind::TypeSet,
            &format!("d5p1:{normalized}"),
            true,
        ));
    }
    if let Some((prefix, _)) = normalized.split_once('.') {
        if !form_valid_cfg_prefixes().contains(&prefix) {
            return Err(format!(
                "type '{type_name}' has unknown configuration type prefix '{prefix}'"
            ));
        }
        dcs_compile_validate_configuration_type(type_name, &normalized)?;
        return Ok(dcs_type_entry(
            DcsTypeNodeKind::Type,
            &format!("d5p1:{normalized}"),
            true,
        ));
    }
    // In the DCS DSL a bare XML name deliberately denotes a configuration
    // TypeSet. Its existence can only be checked against a concrete configuration.
    if form_is_xml_ncname(&normalized) {
        return Ok(dcs_type_entry(
            DcsTypeNodeKind::TypeSet,
            &format!("d5p1:{normalized}"),
            true,
        ));
    }
    Err(format!(
        "type '{type_name}' is not supported by the fixed 8.3.27 DCS type contract"
    ))
}

fn dcs_compile_parse_decimal_contract(value: &str) -> Option<(u32, u32, bool)> {
    let rest = value.strip_prefix("decimal(")?.strip_suffix(')')?;
    let parts = rest.split(',').map(str::trim).collect::<Vec<_>>();
    if parts.len() == 1 {
        let digits = parts[0]
            .parse::<u32>()
            .ok()
            .filter(|digits| *digits <= 38)?;
        return Some((digits, 0, false));
    }
    parse_form_decimal_contract(value)
}

fn dcs_compile_validate_configuration_type(raw: &str, normalized: &str) -> Result<(), String> {
    let invalid_name = normalized
        .split_once('.')
        .is_none_or(|(_, name)| name.trim().is_empty() || name.contains('.'));
    if invalid_name || !form_is_xml_ncname(normalized) {
        return Err(format!(
            "type '{raw}' has an invalid or empty configuration type name"
        ));
    }
    Ok(())
}

fn dcs_type_entry(
    kind: DcsTypeNodeKind,
    wire_name: &str,
    configuration_namespace: bool,
) -> DcsTypeEntry {
    DcsTypeEntry {
        kind,
        wire_name: wire_name.to_string(),
        configuration_namespace,
        qualifier: None,
    }
}

fn dcs_type_qualified_entry(wire_name: &str, qualifier: DcsTypeQualifier) -> DcsTypeEntry {
    DcsTypeEntry {
        qualifier: Some(qualifier),
        ..dcs_type_entry(DcsTypeNodeKind::Type, wire_name, false)
    }
}

fn dcs_compile_emit_value_type_entries(
    lines: &mut Vec<String>,
    entries: &[DcsTypeEntry],
    indent: &str,
) {
    for kind in [
        DcsTypeNodeKind::Type,
        DcsTypeNodeKind::TypeSet,
        DcsTypeNodeKind::TypeId,
    ] {
        for entry in entries.iter().filter(|entry| entry.kind == kind) {
            let tag = entry.kind.tag();
            if entry.configuration_namespace {
                lines.push(format!(
                    "{indent}<v8:{tag} xmlns:d5p1=\"http://v8.1c.ru/8.1/data/enterprise/current-config\">{}</v8:{tag}>",
                    escape_xml(&entry.wire_name)
                ));
            } else {
                lines.push(format!(
                    "{indent}<v8:{tag}>{}</v8:{tag}>",
                    escape_xml(&entry.wire_name)
                ));
            }
        }
    }
    for qualifier_rank in [0_u8, 1, 2] {
        for qualifier in entries.iter().filter_map(|entry| entry.qualifier) {
            if dcs_type_qualifier_rank(qualifier) == qualifier_rank {
                dcs_compile_emit_type_qualifier(lines, qualifier, indent);
            }
        }
    }
}

fn dcs_type_qualifier_rank(qualifier: DcsTypeQualifier) -> u8 {
    match qualifier {
        DcsTypeQualifier::Number { .. } => 0,
        DcsTypeQualifier::String { .. } => 1,
        DcsTypeQualifier::Date(_) => 2,
    }
}

fn dcs_compile_emit_type_qualifier(
    lines: &mut Vec<String>,
    qualifier: DcsTypeQualifier,
    indent: &str,
) {
    match qualifier {
        DcsTypeQualifier::Number {
            digits,
            fraction,
            nonnegative,
        } => {
            lines.push(format!("{indent}<v8:NumberQualifiers>"));
            lines.push(format!("{indent}\t<v8:Digits>{digits}</v8:Digits>"));
            lines.push(format!(
                "{indent}\t<v8:FractionDigits>{fraction}</v8:FractionDigits>"
            ));
            lines.push(format!(
                "{indent}\t<v8:AllowedSign>{}</v8:AllowedSign>",
                if nonnegative { "Nonnegative" } else { "Any" }
            ));
            lines.push(format!("{indent}</v8:NumberQualifiers>"));
        }
        DcsTypeQualifier::String { length, fixed } => {
            lines.push(format!("{indent}<v8:StringQualifiers>"));
            lines.push(format!("{indent}\t<v8:Length>{length}</v8:Length>"));
            lines.push(format!(
                "{indent}\t<v8:AllowedLength>{}</v8:AllowedLength>",
                if fixed { "Fixed" } else { "Variable" }
            ));
            lines.push(format!("{indent}</v8:StringQualifiers>"));
        }
        DcsTypeQualifier::Date(fractions) => {
            lines.push(format!("{indent}<v8:DateQualifiers>"));
            lines.push(format!(
                "{indent}\t<v8:DateFractions>{fractions}</v8:DateFractions>"
            ));
            lines.push(format!("{indent}</v8:DateQualifiers>"));
        }
    }
}

pub(crate) fn dcs_compile_emit_mltext(
    lines: &mut Vec<String>,
    indent: &str,
    tag: &str,
    text: &str,
) {
    dcs_compile_emit_mltext_ex(lines, indent, tag, text, false);
}

pub(crate) fn dcs_compile_emit_mltext_ex(
    lines: &mut Vec<String>,
    indent: &str,
    tag: &str,
    text: &str,
    no_xsi_type: bool,
) {
    if text.is_empty() {
        if no_xsi_type {
            lines.push(format!("{indent}<{tag}/>"));
        } else {
            lines.push(format!("{indent}<{tag} xsi:type=\"v8:LocalStringType\"/>"));
        }
        return;
    }
    if no_xsi_type {
        lines.push(format!("{indent}<{tag}>"));
    } else {
        lines.push(format!("{indent}<{tag} xsi:type=\"v8:LocalStringType\">"));
    }
    lines.push(format!("{indent}\t<v8:item>"));
    lines.push(format!("{indent}\t\t<v8:lang>ru</v8:lang>"));
    lines.push(format!(
        "{indent}\t\t<v8:content>{}</v8:content>",
        escape_xml(text)
    ));
    lines.push(format!("{indent}\t</v8:item>"));
    lines.push(format!("{indent}</{tag}>"));
}

pub(crate) fn dcs_compile_emit_default_settings_variant(lines: &mut Vec<String>) {
    lines.push("\t<settingsVariant>".to_string());
    lines.push("\t\t<dcsset:name>Основной</dcsset:name>".to_string());
    dcs_compile_emit_mltext(lines, "\t\t", "dcsset:presentation", "Основной");
    lines.push("\t\t<dcsset:settings xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\">".to_string());
    lines.push("\t\t\t<dcsset:selection>".to_string());
    lines.push("\t\t\t\t<dcsset:item xsi:type=\"dcsset:SelectedItemAuto\"/>".to_string());
    lines.push("\t\t\t</dcsset:selection>".to_string());
    lines.push("\t\t\t<dcsset:item xsi:type=\"dcsset:StructureItemGroup\">".to_string());
    lines.push("\t\t\t\t<dcsset:order>".to_string());
    lines.push("\t\t\t\t\t<dcsset:item xsi:type=\"dcsset:OrderItemAuto\"/>".to_string());
    lines.push("\t\t\t\t</dcsset:order>".to_string());
    lines.push("\t\t\t\t<dcsset:selection>".to_string());
    lines.push("\t\t\t\t\t<dcsset:item xsi:type=\"dcsset:SelectedItemAuto\"/>".to_string());
    lines.push("\t\t\t\t</dcsset:selection>".to_string());
    lines.push("\t\t\t</dcsset:item>".to_string());
    lines.push("\t\t</dcsset:settings>".to_string());
    lines.push("\t</settingsVariant>".to_string());
}

pub(crate) fn dcs_compile_emit_data_set_links(lines: &mut Vec<String>, defn: &Value) {
    let Some(links) = defn.get("dataSetLinks").and_then(Value::as_array) else {
        return;
    };
    for link in links {
        let source = json_string_field(link, "source")
            .or_else(|| json_string_field(link, "sourceDataSet"))
            .unwrap_or_default();
        let destination = json_string_field(link, "dest")
            .or_else(|| json_string_field(link, "destinationDataSet"))
            .unwrap_or_default();
        let source_expression = json_string_field(link, "sourceExpr")
            .or_else(|| json_string_field(link, "sourceExpression"))
            .unwrap_or_default();
        let destination_expression = json_string_field(link, "destExpr")
            .or_else(|| json_string_field(link, "destinationExpression"))
            .unwrap_or_default();
        lines.push("\t<dataSetLink>".to_string());
        lines.push(format!(
            "\t\t<sourceDataSet>{}</sourceDataSet>",
            escape_xml(&source)
        ));
        lines.push(format!(
            "\t\t<destinationDataSet>{}</destinationDataSet>",
            escape_xml(&destination)
        ));
        lines.push(format!(
            "\t\t<sourceExpression>{}</sourceExpression>",
            escape_xml(&source_expression)
        ));
        lines.push(format!(
            "\t\t<destinationExpression>{}</destinationExpression>",
            escape_xml(&destination_expression)
        ));
        if let Some(parameter) =
            json_string_field(link, "parameter").filter(|value| !value.is_empty())
        {
            lines.push(format!(
                "\t\t<parameter>{}</parameter>",
                escape_xml(&parameter)
            ));
        }
        if link
            .get("parameterListAllowed")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            lines.push("\t\t<parameterListAllowed>true</parameterListAllowed>".to_string());
        }
        if let Some(value) = json_string_field(link, "linkConditionExpression") {
            lines.push(format!(
                "\t\t<linkConditionExpression>{}</linkConditionExpression>",
                escape_xml(&value)
            ));
        }
        if let Some(value) = json_string_field(link, "startExpression") {
            lines.push(format!(
                "\t\t<startExpression>{}</startExpression>",
                escape_xml(&value)
            ));
        }
        if link.get("required").and_then(Value::as_bool) == Some(false) {
            lines.push("\t\t<required>false</required>".to_string());
        }
        lines.push("\t</dataSetLink>".to_string());
    }
}

pub(crate) fn dcs_compile_emit_calculated_fields(
    lines: &mut Vec<String>,
    defn: &Value,
) -> Result<(), String> {
    let Some(fields) = defn.get("calculatedFields").and_then(Value::as_array) else {
        return Ok(());
    };
    for field in fields {
        let (data_path, expression, title, field_type, type_declared) =
            if let Some(text) = field.as_str() {
                let parsed = dcs_edit_parse_calc_field(text);
                (
                    parsed.data_path,
                    parsed.expression,
                    parsed.title,
                    parsed.field_type,
                    parsed.type_declared,
                )
            } else {
                (
                    json_string_field(field, "dataPath")
                        .or_else(|| json_string_field(field, "field"))
                        .or_else(|| json_string_field(field, "name"))
                        .unwrap_or_default(),
                    json_string_field(field, "expression").unwrap_or_default(),
                    json_string_field(field, "title").unwrap_or_default(),
                    field
                        .get("type")
                        .map(dcs_compile_type_value)
                        .unwrap_or_default(),
                    field.get("type").is_some(),
                )
            };
        let value_type_entries = type_declared
            .then(|| dcs_compile_parse_value_type(&field_type))
            .transpose()?;
        lines.push("\t<calculatedField>".to_string());
        lines.push(format!(
            "\t\t<dataPath>{}</dataPath>",
            escape_xml(&data_path)
        ));
        lines.push(format!(
            "\t\t<expression>{}</expression>",
            escape_xml(&expression)
        ));
        if !title.is_empty() {
            dcs_compile_emit_mltext(lines, "\t\t", "title", &title);
        }
        dcs_compile_emit_merged_restriction(
            lines,
            field,
            &["restrict", "useRestriction"],
            "useRestriction",
            "\t\t",
        );
        if value_type_entries.is_some() {
            lines.push("\t\t<valueType>".to_string());
            dcs_compile_emit_value_type_entries(
                lines,
                value_type_entries.as_deref().unwrap_or_default(),
                "\t\t\t",
            );
            lines.push("\t\t</valueType>".to_string());
        }
        lines.push("\t</calculatedField>".to_string());
    }
    Ok(())
}

pub(crate) fn dcs_compile_emit_total_fields(lines: &mut Vec<String>, defn: &Value) {
    let Some(fields) = defn.get("totalFields").and_then(Value::as_array) else {
        return;
    };
    for field in fields {
        let (data_path, expression, groups) = if let Some(text) = field.as_str() {
            let (data_path, expression) = text
                .split_once(':')
                .map(|(left, right)| (left.trim().to_string(), right.trim().to_string()))
                .unwrap_or((text.trim().to_string(), String::new()));
            let expression = dcs_edit_total_expression(&data_path, &expression);
            (data_path, expression, Vec::new())
        } else {
            let data_path = json_string_field(field, "dataPath").unwrap_or_default();
            let expression = json_string_field(field, "expression").unwrap_or_default();
            let groups = dcs_compile_string_items(field.get("group")).unwrap_or_default();
            (data_path, expression, groups)
        };
        lines.push("\t<totalField>".to_string());
        lines.push(format!(
            "\t\t<dataPath>{}</dataPath>",
            escape_xml(&data_path)
        ));
        lines.push(format!(
            "\t\t<expression>{}</expression>",
            escape_xml(&expression)
        ));
        for group in groups {
            lines.push(format!("\t\t<group>{}</group>", escape_xml(&group)));
        }
        lines.push("\t</totalField>".to_string());
    }
}

pub(crate) fn dcs_compile_emit_parameters(
    lines: &mut Vec<String>,
    defn: &Value,
) -> Result<(), String> {
    let Some(parameters) = defn.get("parameters").and_then(Value::as_array) else {
        return Ok(());
    };
    for parameter in parameters {
        let (parsed, type_declared) = if let Some(text) = parameter.as_str() {
            let parsed = dcs_edit_parse_parameter(text);
            let type_declared = parsed.type_declared;
            (parsed, type_declared)
        } else {
            (
                DcsEditParameter {
                    name: json_string_field(parameter, "name").unwrap_or_default(),
                    title: json_string_field(parameter, "title")
                        .or_else(|| json_string_field(parameter, "presentation"))
                        .unwrap_or_default(),
                    type_name: parameter
                        .get("type")
                        .map(dcs_compile_type_value)
                        .unwrap_or_default(),
                    values: parameter
                        .get("value")
                        .map(|value| vec![dcs_compile_setting_value_text(value)])
                        .unwrap_or_default(),
                    hidden: parameter
                        .get("hidden")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    always: parameter
                        .get("use")
                        .map(json_value_to_python_string)
                        .is_some_and(|value| value == "Always"),
                    value_list_allowed: parameter
                        .get("valueListAllowed")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    available_values: dcs_compile_parameter_available_values(parameter)?,
                    auto_dates: parameter
                        .get("autoDates")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    expression: json_string_field(parameter, "expression"),
                    type_declared: parameter.get("type").is_some(),
                },
                parameter.get("type").is_some(),
            )
        };
        let value_type_entries = type_declared
            .then(|| dcs_compile_parse_value_type(&parsed.type_name))
            .transpose()?;
        lines.push("\t<parameter>".to_string());
        lines.push(format!("\t\t<name>{}</name>", escape_xml(&parsed.name)));
        if !parsed.title.is_empty() {
            dcs_compile_emit_mltext(lines, "\t\t", "title", &parsed.title);
        }
        if value_type_entries.is_some() {
            lines.push("\t\t<valueType>".to_string());
            dcs_compile_emit_value_type_entries(
                lines,
                value_type_entries.as_deref().unwrap_or_default(),
                "\t\t\t",
            );
            lines.push("\t\t</valueType>".to_string());
        }
        if parsed.values.is_empty() {
            if !parsed.value_list_allowed {
                dcs_compile_emit_empty_value(lines, &parsed.type_name, "\t\t", "value");
            }
        } else {
            for value in &parsed.values {
                dcs_compile_emit_parameter_value(lines, &parsed.type_name, value, "\t\t", "value");
            }
        }
        let use_restriction = parsed.hidden
            || parameter
                .get("useRestriction")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        lines.push(format!(
            "\t\t<useRestriction>{}</useRestriction>",
            if use_restriction { "true" } else { "false" }
        ));
        if let Some(expression) = parsed.expression.as_ref().filter(|value| !value.is_empty()) {
            lines.push(format!(
                "\t\t<expression>{}</expression>",
                escape_xml(expression)
            ));
        }
        for (value, presentation) in &parsed.available_values {
            lines.push("\t\t<availableValue>".to_string());
            dcs_compile_emit_parameter_value(lines, &parsed.type_name, value, "\t\t\t", "value");
            if !presentation.is_empty() {
                dcs_compile_emit_mltext(lines, "\t\t\t", "presentation", presentation);
            }
            lines.push("\t\t</availableValue>".to_string());
        }
        if parsed.value_list_allowed {
            lines.push("\t\t<valueListAllowed>true</valueListAllowed>".to_string());
        }
        if parsed.hidden
            || parameter
                .get("availableAsField")
                .and_then(Value::as_bool)
                .is_some_and(|value| !value)
        {
            lines.push("\t\t<availableAsField>false</availableAsField>".to_string());
        }
        if parameter
            .get("denyIncompleteValues")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            lines.push("\t\t<denyIncompleteValues>true</denyIncompleteValues>".to_string());
        }
        if parsed.always {
            lines.push("\t\t<use>Always</use>".to_string());
        }
        lines.push("\t</parameter>".to_string());
    }
    Ok(())
}

pub(crate) fn dcs_compile_parameter_available_values(
    parameter: &Value,
) -> Result<Vec<(String, String)>, String> {
    let Some(raw_items) = parameter.get("availableValues") else {
        return Ok(Vec::new());
    };
    let items = raw_items
        .as_array()
        .ok_or_else(|| "parameter 'availableValues' must be an array".to_string())?;
    let mut result = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let object = item
            .as_object()
            .ok_or_else(|| format!("parameter 'availableValues[{index}]' must be an object"))?;
        let value = object
            .get("value")
            .ok_or_else(|| format!("parameter 'availableValues[{index}].value' is required"))?;
        if !matches!(value, Value::Bool(_) | Value::Number(_) | Value::String(_)) {
            return Err(format!(
                "parameter 'availableValues[{index}].value' must be a string, number, or boolean"
            ));
        }
        let presentation = match object.get("presentation") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(value)) => value.clone(),
            Some(_) => {
                return Err(format!(
                    "parameter 'availableValues[{index}].presentation' must be a string"
                ));
            }
        };
        result.push((dcs_compile_setting_value_text(value), presentation));
    }
    Ok(result)
}

pub(crate) fn dcs_compile_emit_empty_value(
    lines: &mut Vec<String>,
    type_name: &str,
    indent: &str,
    tag_name: &str,
) {
    let type_name = dcs_edit_normalize_declared_type(type_name);
    if type_name.is_empty() {
        lines.push(format!("{indent}<{tag_name} xsi:nil=\"true\"/>"));
    } else if type_name == "StandardPeriod" {
        lines.push(format!(
            "{indent}<{tag_name} xsi:type=\"v8:StandardPeriod\">"
        ));
        lines.push(format!(
            "{indent}\t<v8:variant xsi:type=\"v8:StandardPeriodVariant\">Custom</v8:variant>"
        ));
        lines.push(format!(
            "{indent}\t<v8:startDate>0001-01-01T00:00:00</v8:startDate>"
        ));
        lines.push(format!(
            "{indent}\t<v8:endDate>0001-01-01T00:00:00</v8:endDate>"
        ));
        lines.push(format!("{indent}</{tag_name}>"));
    } else if type_name.starts_with("string") {
        lines.push(format!("{indent}<{tag_name} xsi:type=\"xs:string\"/>"));
    } else if type_name.starts_with("date") {
        lines.push(format!(
            "{indent}<{tag_name} xsi:type=\"xs:dateTime\">0001-01-01T00:00:00</{tag_name}>"
        ));
    } else if type_name.starts_with("decimal") {
        lines.push(format!(
            "{indent}<{tag_name} xsi:type=\"xs:decimal\">0</{tag_name}>"
        ));
    } else if type_name == "boolean" {
        lines.push(format!(
            "{indent}<{tag_name} xsi:type=\"xs:boolean\">false</{tag_name}>"
        ));
    } else {
        lines.push(format!("{indent}<{tag_name} xsi:nil=\"true\"/>"));
    }
}

pub(crate) fn dcs_compile_emit_parameter_value(
    lines: &mut Vec<String>,
    type_name: &str,
    value: &str,
    indent: &str,
    tag_name: &str,
) {
    if dcs_edit_is_empty_value(value) {
        dcs_compile_emit_empty_value(lines, type_name, indent, tag_name);
        return;
    }
    let normalized_type = dcs_edit_normalize_declared_type(type_name);
    if normalized_type == "StandardPeriod" {
        lines.push(format!(
            "{indent}<{tag_name} xsi:type=\"v8:StandardPeriod\">"
        ));
        lines.push(format!(
            "{indent}\t<v8:variant xsi:type=\"v8:StandardPeriodVariant\">{}</v8:variant>",
            escape_xml(value)
        ));
        if value == "Custom" {
            lines.push(format!(
                "{indent}\t<v8:startDate>0001-01-01T00:00:00</v8:startDate>"
            ));
            lines.push(format!(
                "{indent}\t<v8:endDate>0001-01-01T00:00:00</v8:endDate>"
            ));
        }
        lines.push(format!("{indent}</{tag_name}>"));
        return;
    }
    let xsi_type = dcs_compile_setting_xsi_type(Some(&normalized_type), value);
    let value_text = if xsi_type == "xs:boolean" {
        value.to_lowercase()
    } else {
        value.to_string()
    };
    lines.push(format!(
        "{indent}<{tag_name} xsi:type=\"{xsi_type}\">{}</{tag_name}>",
        escape_xml(&value_text)
    ));
}

pub(crate) fn dcs_compile_emit_settings_variants(lines: &mut Vec<String>, defn: &Value) {
    let Some(variants) = defn.get("settingsVariants").and_then(Value::as_array) else {
        dcs_compile_emit_default_settings_variant(lines);
        return;
    };
    if variants.is_empty() {
        dcs_compile_emit_default_settings_variant(lines);
        return;
    }
    for variant in variants {
        lines.push("\t<settingsVariant>".to_string());
        let name = json_string_field(variant, "name").unwrap_or_default();
        lines.push(format!(
            "\t\t<dcsset:name>{}</dcsset:name>",
            escape_xml(&name)
        ));
        let presentation = json_string_field(variant, "presentation")
            .or_else(|| json_string_field(variant, "title"))
            .unwrap_or_else(|| name.clone());
        dcs_compile_emit_mltext(lines, "\t\t", "dcsset:presentation", &presentation);
        lines.push("\t\t<dcsset:settings xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\">".to_string());
        let settings = variant.get("settings").unwrap_or(&Value::Null);
        if let Some(selection) = settings.get("selection").and_then(Value::as_array) {
            dcs_compile_emit_selection(lines, selection, "\t\t\t");
        }
        if let Some(filter) = settings.get("filter").and_then(Value::as_array) {
            dcs_compile_emit_filter(lines, filter, "\t\t\t");
        }
        if let Some(data_parameters) = settings.get("dataParameters").and_then(Value::as_array) {
            dcs_compile_emit_data_parameters(lines, data_parameters, "\t\t\t");
        }
        if let Some(order) = settings.get("order").and_then(Value::as_array) {
            dcs_compile_emit_order(lines, order, "\t\t\t");
        }
        if let Some(conditional_appearance) = settings
            .get("conditionalAppearance")
            .and_then(Value::as_array)
        {
            dcs_compile_emit_conditional_appearance(lines, conditional_appearance, "\t\t\t");
        }
        if let Some(output_parameters) = settings.get("outputParameters").and_then(Value::as_object)
        {
            dcs_compile_emit_output_parameters(lines, output_parameters, "\t\t\t");
        }
        if let Some(structure) = settings.get("structure") {
            dcs_compile_emit_structure(lines, structure, "\t\t\t");
        }
        lines.push("\t\t</dcsset:settings>".to_string());
        lines.push("\t</settingsVariant>".to_string());
    }
}

pub(crate) fn dcs_compile_emit_selection(lines: &mut Vec<String>, items: &[Value], indent: &str) {
    if items.is_empty() {
        return;
    }
    lines.push(format!("{indent}<dcsset:selection>"));
    for item in items {
        dcs_compile_emit_selection_item(lines, item, &format!("{indent}\t"));
    }
    lines.push(format!("{indent}</dcsset:selection>"));
}

pub(crate) fn dcs_compile_emit_selection_item(lines: &mut Vec<String>, item: &Value, indent: &str) {
    if let Some(text) = item.as_str() {
        if text == "Auto" {
            lines.push(format!(
                "{indent}<dcsset:item xsi:type=\"dcsset:SelectedItemAuto\"/>"
            ));
            return;
        }
        lines.push(format!(
            "{indent}<dcsset:item xsi:type=\"dcsset:SelectedItemField\">"
        ));
        lines.push(format!(
            "{indent}\t<dcsset:field>{}</dcsset:field>",
            escape_xml(text)
        ));
        lines.push(format!("{indent}</dcsset:item>"));
        return;
    }
    if item.get("auto").and_then(Value::as_bool).unwrap_or(false) {
        lines.push(format!(
            "{indent}<dcsset:item xsi:type=\"dcsset:SelectedItemAuto\">"
        ));
        if item
            .get("use")
            .and_then(Value::as_bool)
            .is_some_and(|value| !value)
        {
            lines.push(format!("{indent}\t<dcsset:use>false</dcsset:use>"));
        }
        lines.push(format!("{indent}</dcsset:item>"));
        return;
    }
    let field = json_string_field(item, "field").unwrap_or_default();
    lines.push(format!(
        "{indent}<dcsset:item xsi:type=\"dcsset:SelectedItemField\">"
    ));
    if item
        .get("use")
        .and_then(Value::as_bool)
        .is_some_and(|value| !value)
    {
        lines.push(format!("{indent}\t<dcsset:use>false</dcsset:use>"));
    }
    lines.push(format!(
        "{indent}\t<dcsset:field>{}</dcsset:field>",
        escape_xml(&field)
    ));
    if let Some(title) = json_string_field(item, "title").filter(|value| !value.is_empty()) {
        dcs_compile_emit_mltext_ex(
            lines,
            &format!("{indent}\t"),
            "dcsset:lwsTitle",
            &title,
            true,
        );
    }
    if let Some(view_mode) = json_string_field(item, "viewMode").filter(|value| !value.is_empty()) {
        lines.push(format!(
            "{indent}\t<dcsset:viewMode>{}</dcsset:viewMode>",
            escape_xml(&view_mode)
        ));
    }
    lines.push(format!("{indent}</dcsset:item>"));
}

pub(crate) fn dcs_compile_emit_filter(lines: &mut Vec<String>, items: &[Value], indent: &str) {
    if items.is_empty() {
        return;
    }
    lines.push(format!("{indent}<dcsset:filter>"));
    for item in items {
        dcs_compile_emit_filter_item(lines, item, &format!("{indent}\t"));
    }
    lines.push(format!("{indent}</dcsset:filter>"));
}

pub(crate) fn dcs_compile_emit_conditional_appearance(
    lines: &mut Vec<String>,
    items: &[Value],
    indent: &str,
) {
    if items.is_empty() {
        return;
    }
    lines.push(format!("{indent}<dcsset:conditionalAppearance>"));
    for item in items {
        if let Some(text) = item.as_str() {
            let parsed = dcs_edit_parse_conditional_appearance(text);
            let fragment =
                dcs_edit_conditional_appearance_fragment(&parsed, &format!("{indent}\t"));
            lines.extend(fragment.lines().map(ToOwned::to_owned));
            continue;
        }

        lines.push(format!("{indent}\t<dcsset:item>"));
        if item
            .get("use")
            .and_then(Value::as_bool)
            .is_some_and(|value| !value)
        {
            lines.push(format!("{indent}\t\t<dcsset:use>false</dcsset:use>"));
        }
        if let Some(fields) = item
            .get("selection")
            .or_else(|| item.get("fields"))
            .and_then(Value::as_array)
        {
            lines.push(format!("{indent}\t\t<dcsset:selection>"));
            for field in fields {
                let (name, use_field) = if let Some(name) = field.as_str() {
                    (name.to_string(), true)
                } else {
                    (
                        json_string_field(field, "field").unwrap_or_default(),
                        field.get("use").and_then(Value::as_bool).unwrap_or(true),
                    )
                };
                if name.is_empty() {
                    continue;
                }
                lines.push(format!("{indent}\t\t\t<dcsset:item>"));
                if !use_field {
                    lines.push(format!("{indent}\t\t\t\t<dcsset:use>false</dcsset:use>"));
                }
                lines.push(format!(
                    "{indent}\t\t\t\t<dcsset:field>{}</dcsset:field>",
                    escape_xml(&name)
                ));
                lines.push(format!("{indent}\t\t\t</dcsset:item>"));
            }
            lines.push(format!("{indent}\t\t</dcsset:selection>"));
        }
        if let Some(filter) = item.get("filter").and_then(Value::as_array) {
            dcs_compile_emit_filter(lines, filter, &format!("{indent}\t\t"));
        }
        if let Some(appearance) = item.get("appearance").and_then(Value::as_object) {
            lines.push(format!("{indent}\t\t<dcsset:appearance>"));
            for (parameter, raw_value) in appearance {
                let (value, use_value) = if let Some(object) = raw_value.as_object() {
                    if let Some(value) = object.get("value") {
                        (
                            value,
                            object.get("use").and_then(Value::as_bool).unwrap_or(true),
                        )
                    } else {
                        (raw_value, true)
                    }
                } else {
                    (raw_value, true)
                };
                lines.push(format!(
                    "{indent}\t\t\t<dcscor:item xsi:type=\"dcsset:SettingsParameterValue\">"
                ));
                if !use_value {
                    lines.push(format!("{indent}\t\t\t\t<dcscor:use>false</dcscor:use>"));
                }
                lines.push(format!(
                    "{indent}\t\t\t\t<dcscor:parameter>{}</dcscor:parameter>",
                    escape_xml(parameter)
                ));
                let value_text = dcs_compile_setting_value_text(value);
                lines.extend(dcs_edit_conditional_appearance_value_lines(
                    parameter,
                    &value_text,
                    &format!("{indent}\t\t\t\t"),
                ));
                lines.push(format!("{indent}\t\t\t</dcscor:item>"));
            }
            lines.push(format!("{indent}\t\t</dcsset:appearance>"));
        }
        lines.push(format!("{indent}\t</dcsset:item>"));
    }
    lines.push(format!("{indent}</dcsset:conditionalAppearance>"));
}

pub(crate) fn dcs_compile_emit_filter_item(lines: &mut Vec<String>, item: &Value, indent: &str) {
    let parsed_from_string;
    let item = if let Some(text) = item.as_str() {
        let parsed = dcs_edit_parse_filter(text);
        parsed_from_string = json!({
            "field": parsed.field,
            "op": parsed.operator,
            "value": parsed.value,
            "valueType": parsed.value_type,
            "use": parsed.use_flag.unwrap_or(true),
            "viewMode": parsed.view_mode,
            "userSettingID": parsed.user_setting_id,
        });
        &parsed_from_string
    } else {
        item
    };
    lines.push(format!(
        "{indent}<dcsset:item xsi:type=\"dcsset:FilterItemComparison\">"
    ));
    if item
        .get("use")
        .and_then(Value::as_bool)
        .is_some_and(|value| !value)
    {
        lines.push(format!("{indent}\t<dcsset:use>false</dcsset:use>"));
    }
    let field = json_string_field(item, "field").unwrap_or_default();
    lines.push(format!(
        "{indent}\t<dcsset:left xsi:type=\"dcscor:Field\">{}</dcsset:left>",
        escape_xml(&field)
    ));
    let operator = json_string_field(item, "op").unwrap_or_else(|| "Equal".to_string());
    lines.push(format!(
        "{indent}\t<dcsset:comparisonType>{}</dcsset:comparisonType>",
        escape_xml(dcs_compile_comparison_type(&operator))
    ));
    if let Some(value) = item.get("value").filter(|value| !value.is_null()) {
        let value_text = dcs_compile_setting_value_text(value);
        if !value_text.is_empty() {
            let explicit_type = json_string_field(item, "valueType");
            let xsi_type = dcs_compile_setting_xsi_type(explicit_type.as_deref(), &value_text);
            lines.push(format!(
                "{indent}\t<dcsset:right xsi:type=\"{xsi_type}\">{}</dcsset:right>",
                escape_xml(&value_text)
            ));
        }
    }
    if let Some(view_mode) =
        json_string_field(item, "viewMode").filter(|value| !value.is_empty() && value != "None")
    {
        lines.push(format!(
            "{indent}\t<dcsset:viewMode>{}</dcsset:viewMode>",
            escape_xml(&view_mode)
        ));
    }
    if let Some(user_setting_id) = json_string_field(item, "userSettingID")
        .filter(|value| !value.is_empty() && value != "None")
    {
        lines.push(format!(
            "{indent}\t<dcsset:userSettingID>{}</dcsset:userSettingID>",
            escape_xml(&user_setting_id)
        ));
    }
    lines.push(format!("{indent}</dcsset:item>"));
}

pub(crate) fn dcs_compile_emit_order(lines: &mut Vec<String>, items: &[Value], indent: &str) {
    if items.is_empty() {
        return;
    }
    lines.push(format!("{indent}<dcsset:order>"));
    for item in items {
        let fragment = if let Some(text) = item.as_str() {
            dcs_edit_order_fragment(text, &format!("{indent}\t"))
        } else {
            let field = json_string_field(item, "field").unwrap_or_default();
            let direction =
                json_string_field(item, "direction").unwrap_or_else(|| "Asc".to_string());
            dcs_edit_order_fragment(&format!("{field} {direction}"), &format!("{indent}\t"))
        };
        lines.extend(fragment.lines().map(ToOwned::to_owned));
    }
    lines.push(format!("{indent}</dcsset:order>"));
}

pub(crate) fn dcs_compile_emit_output_parameters(
    lines: &mut Vec<String>,
    params: &Map<String, Value>,
    indent: &str,
) {
    if params.is_empty() {
        return;
    }
    lines.push(format!("{indent}<dcsset:outputParameters>"));
    for (key, raw_value) in params {
        let (value, explicit_type, use_false) = if let Some(object) = raw_value.as_object() {
            if let Some(value) = object.get("value") {
                (
                    value,
                    json_string_field(raw_value, "valueType"),
                    object
                        .get("use")
                        .and_then(Value::as_bool)
                        .is_some_and(|value| !value),
                )
            } else {
                (raw_value, None, false)
            }
        } else {
            (raw_value, None, false)
        };
        let value_text = dcs_compile_setting_value_text(value);
        let xsi_type = explicit_type
            .as_deref()
            .unwrap_or_else(|| dcs_compile_output_parameter_type(key, value));
        lines.push(format!(
            "{indent}\t<dcscor:item xsi:type=\"dcsset:SettingsParameterValue\">"
        ));
        if use_false {
            lines.push(format!("{indent}\t\t<dcscor:use>false</dcscor:use>"));
        }
        lines.push(format!(
            "{indent}\t\t<dcscor:parameter>{}</dcscor:parameter>",
            escape_xml(key)
        ));
        if xsi_type == "mltext" {
            dcs_compile_emit_mltext(lines, &format!("{indent}\t\t"), "dcscor:value", &value_text);
        } else {
            lines.push(format!(
                "{indent}\t\t<dcscor:value xsi:type=\"{xsi_type}\">{}</dcscor:value>",
                escape_xml(&value_text)
            ));
        }
        lines.push(format!("{indent}\t</dcscor:item>"));
    }
    lines.push(format!("{indent}</dcsset:outputParameters>"));
}

pub(crate) fn dcs_compile_emit_data_parameters(
    lines: &mut Vec<String>,
    items: &[Value],
    indent: &str,
) {
    if items.is_empty() {
        return;
    }
    lines.push(format!("{indent}<dcsset:dataParameters>"));
    for item in items {
        lines.push(format!(
            "{indent}\t<dcscor:item xsi:type=\"dcsset:SettingsParameterValue\">"
        ));
        if item
            .get("use")
            .and_then(Value::as_bool)
            .is_some_and(|value| !value)
        {
            lines.push(format!("{indent}\t\t<dcscor:use>false</dcscor:use>"));
        }
        let parameter = json_string_field(item, "parameter").unwrap_or_default();
        lines.push(format!(
            "{indent}\t\t<dcscor:parameter>{}</dcscor:parameter>",
            escape_xml(&parameter)
        ));
        if item
            .get("nilValue")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            lines.push(format!("{indent}\t\t<dcscor:value xsi:nil=\"true\"/>"));
        } else if let Some(value) = item.get("value").filter(|value| !value.is_null()) {
            let value_text = dcs_compile_setting_value_text(value);
            let explicit_type = json_string_field(item, "valueType");
            let xsi_type = dcs_compile_setting_xsi_type(explicit_type.as_deref(), &value_text);
            lines.push(format!(
                "{indent}\t\t<dcscor:value xsi:type=\"{xsi_type}\">{}</dcscor:value>",
                escape_xml(&value_text)
            ));
        }
        if let Some(view_mode) =
            json_string_field(item, "viewMode").filter(|value| !value.is_empty())
        {
            lines.push(format!(
                "{indent}\t\t<dcsset:viewMode>{}</dcsset:viewMode>",
                escape_xml(&view_mode)
            ));
        }
        if let Some(user_setting_id) =
            json_string_field(item, "userSettingID").filter(|value| !value.is_empty())
        {
            lines.push(format!(
                "{indent}\t\t<dcsset:userSettingID>{}</dcsset:userSettingID>",
                escape_xml(&user_setting_id)
            ));
        }
        if let Some(presentation) =
            json_string_field(item, "userSettingPresentation").filter(|value| !value.is_empty())
        {
            dcs_compile_emit_mltext(
                lines,
                &format!("{indent}\t\t"),
                "dcsset:userSettingPresentation",
                &presentation,
            );
        }
        lines.push(format!("{indent}\t</dcscor:item>"));
    }
    lines.push(format!("{indent}</dcsset:dataParameters>"));
}

pub(crate) fn dcs_compile_emit_structure(lines: &mut Vec<String>, structure: &Value, indent: &str) {
    if let Some(text) = structure.as_str() {
        for item in dcs_edit_parse_structure(text) {
            let fragment = dcs_edit_structure_item_fragment(&item, indent);
            lines.extend(fragment.lines().map(ToOwned::to_owned));
        }
        return;
    }
    if let Some(item) = structure.as_object() {
        dcs_compile_emit_structure_item(lines, &Value::Object(item.clone()), indent);
        return;
    }
    if let Some(items) = structure.as_array() {
        for item in items {
            dcs_compile_emit_structure_item(lines, item, indent);
        }
    }
}

pub(crate) fn dcs_compile_emit_structure_item(lines: &mut Vec<String>, item: &Value, indent: &str) {
    let item_type = json_string_field(item, "type").unwrap_or_else(|| "group".to_string());
    if item_type != "group" {
        return;
    }
    lines.push(format!(
        "{indent}<dcsset:item xsi:type=\"dcsset:StructureItemGroup\">"
    ));
    if item
        .get("use")
        .and_then(Value::as_bool)
        .is_some_and(|value| !value)
    {
        lines.push(format!("{indent}\t<dcsset:use>false</dcsset:use>"));
    }
    if let Some(name) = json_string_field(item, "name").filter(|value| !value.is_empty()) {
        lines.push(format!(
            "{indent}\t<dcsset:name>{}</dcsset:name>",
            escape_xml(&name)
        ));
    }
    let group_by = item.get("groupBy").or_else(|| item.get("groupFields"));
    dcs_compile_emit_group_items(lines, group_by, &format!("{indent}\t"));
    if let Some(filter) = item.get("filter").and_then(Value::as_array) {
        dcs_compile_emit_filter(lines, filter, &format!("{indent}\t"));
    }
    if let Some(order) = item.get("order").and_then(Value::as_array) {
        dcs_compile_emit_order(lines, order, &format!("{indent}\t"));
    }
    if let Some(selection) = item.get("selection").and_then(Value::as_array) {
        dcs_compile_emit_selection(lines, selection, &format!("{indent}\t"));
    }
    if let Some(conditional_appearance) =
        item.get("conditionalAppearance").and_then(Value::as_array)
    {
        dcs_compile_emit_conditional_appearance(
            lines,
            conditional_appearance,
            &format!("{indent}\t"),
        );
    }
    if let Some(output_parameters) = item.get("outputParameters").and_then(Value::as_object) {
        dcs_compile_emit_output_parameters(lines, output_parameters, &format!("{indent}\t"));
    }
    if let Some(children) = item.get("children").and_then(Value::as_array) {
        for child in children {
            dcs_compile_emit_structure_item(lines, child, &format!("{indent}\t"));
        }
    }
    lines.push(format!("{indent}</dcsset:item>"));
}

pub(crate) fn dcs_compile_emit_group_items(
    lines: &mut Vec<String>,
    value: Option<&Value>,
    indent: &str,
) {
    let Some(items) = dcs_compile_string_items(value) else {
        return;
    };
    if items.is_empty() {
        return;
    }
    lines.push(format!("{indent}<dcsset:groupItems>"));
    for field in items {
        if field == "Auto" {
            lines.push(format!(
                "{indent}\t<dcsset:item xsi:type=\"dcsset:GroupItemAuto\"/>"
            ));
            continue;
        }
        lines.push(format!(
            "{indent}\t<dcsset:item xsi:type=\"dcsset:GroupItemField\">"
        ));
        lines.push(format!(
            "{indent}\t\t<dcsset:field>{}</dcsset:field>",
            escape_xml(&field)
        ));
        lines.push(format!(
            "{indent}\t\t<dcsset:groupType>Items</dcsset:groupType>"
        ));
        lines.push(format!(
            "{indent}\t\t<dcsset:periodAdditionType>None</dcsset:periodAdditionType>"
        ));
        lines.push(format!("{indent}\t\t<dcsset:periodAdditionBegin xsi:type=\"xs:dateTime\">0001-01-01T00:00:00</dcsset:periodAdditionBegin>"));
        lines.push(format!("{indent}\t\t<dcsset:periodAdditionEnd xsi:type=\"xs:dateTime\">0001-01-01T00:00:00</dcsset:periodAdditionEnd>"));
        lines.push(format!("{indent}\t</dcsset:item>"));
    }
    lines.push(format!("{indent}</dcsset:groupItems>"));
}

pub(crate) fn dcs_compile_comparison_type(operator: &str) -> &str {
    match operator {
        "=" => "Equal",
        "<>" => "NotEqual",
        ">" => "Greater",
        ">=" => "GreaterOrEqual",
        "<" => "Less",
        "<=" => "LessOrEqual",
        "in" => "InList",
        "notIn" => "NotInList",
        "contains" => "Contains",
        "notContains" => "NotContains",
        "beginsWith" => "BeginsWith",
        "notBeginsWith" => "NotBeginsWith",
        "filled" => "Filled",
        "notFilled" => "NotFilled",
        other => other,
    }
}

pub(crate) fn dcs_compile_setting_value_text(value: &Value) -> String {
    match value {
        Value::Bool(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Null => String::new(),
        other => json_value_to_python_string(other),
    }
}

pub(crate) fn dcs_compile_setting_xsi_type(explicit_type: Option<&str>, value: &str) -> String {
    if let Some(explicit_type) = explicit_type {
        if explicit_type.contains(':') {
            return explicit_type.to_string();
        }
        if explicit_type == "boolean" {
            return "xs:boolean".to_string();
        }
        if explicit_type.starts_with("decimal") {
            return "xs:decimal".to_string();
        }
        if explicit_type.starts_with("date") {
            return "xs:dateTime".to_string();
        }
        if explicit_type.starts_with("string") {
            return "xs:string".to_string();
        }
    }
    if matches!(value, "true" | "false") {
        "xs:boolean".to_string()
    } else if is_date_time_literal(value) {
        "xs:dateTime".to_string()
    } else if value.parse::<f64>().is_ok() {
        "xs:decimal".to_string()
    } else if dcs_edit_is_design_time_value(value) {
        "dcscor:DesignTimeValue".to_string()
    } else {
        "xs:string".to_string()
    }
}

pub(crate) fn dcs_compile_output_parameter_type(key: &str, value: &Value) -> &'static str {
    if value.is_object() && !value.get("@type").is_some_and(|value| value == "Font") {
        return "mltext";
    }
    match key {
        "Заголовок" => "mltext",
        "ВыводитьЗаголовок" | "ВыводитьПараметрыДанных" | "ВыводитьОтбор" => {
            "dcsset:DataCompositionTextOutputType"
        }
        "МакетОформления" => "xs:string",
        "РасположениеПолейГруппировки" => {
            "dcsset:DataCompositionGroupFieldsPlacement"
        }
        "РасположениеРеквизитов" => {
            "dcsset:DataCompositionAttributesPlacement"
        }
        "ГоризонтальноеРасположениеОбщихИтогов"
        | "ВертикальноеРасположениеОбщихИтогов"
        | "РасположениеОбщихИтогов"
        | "РасположениеИтогов" => "dcscor:DataCompositionTotalPlacement",
        "РасположениеГруппировки" => {
            "dcsset:DataCompositionFieldGroupPlacement"
        }
        "РасположениеРесурсов" => "dcsset:DataCompositionResourcesPlacement",
        "ТипМакета" => "dcsset:DataCompositionGroupTemplateType",
        _ => "xs:string",
    }
}

pub(crate) fn dcs_compile_resolve_query_value(
    value: &str,
    base_dir: &Path,
    cwd: &Path,
) -> Result<String, String> {
    dcs_compile_resolve_query_value_with_inputs(value, base_dir, cwd, &mut Vec::new())
}

fn dcs_compile_resolve_query_value_with_inputs(
    value: &str,
    base_dir: &Path,
    cwd: &Path,
    inputs: &mut Vec<ExactFileInput>,
) -> Result<String, String> {
    let Some(file_path) = value.strip_prefix('@') else {
        return Ok(value.to_string());
    };
    let raw = PathBuf::from(file_path);
    let candidates = if raw.is_absolute() {
        vec![raw]
    } else {
        vec![base_dir.join(file_path), cwd.join(file_path)]
    };
    for candidate in &candidates {
        if candidate.exists() {
            let snapshot = read_utf8_sig_snapshot(candidate)?;
            inputs.push(ExactFileInput::new(candidate, snapshot.raw));
            return Ok(snapshot.text.trim_end().to_string());
        }
    }
    Err(format!(
        "Query file not found: {file_path} (searched: {})",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

#[cfg(test)]
type DcsEditAfterReadHook = Box<dyn FnOnce(&Path)>;

#[cfg(test)]
std::thread_local! {
    static TEST_DCS_EDIT_AFTER_READ_HOOK: std::cell::RefCell<Option<DcsEditAfterReadHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn with_dcs_edit_after_read_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<DcsEditAfterReadHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_DCS_EDIT_AFTER_READ_HOOK.with(|slot| slot.replace(self.0.take()));
        }
    }

    let previous = TEST_DCS_EDIT_AFTER_READ_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn run_dcs_edit_after_read_hook(path: &Path) {
    if let Some(hook) = TEST_DCS_EDIT_AFTER_READ_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(path);
    }
}

/// Typed answer of `unica.dcs.edit` (ADR-0023). The prose printed `[OK]` or
/// `[WARN]` per item; the data derives the same verdict from whether the schema
/// actually changed, so it cannot drift from what was written.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DcsEditData {
    pub(crate) operation: String,
    /// The data set the operation addressed; `null` when it is schema-wide.
    pub(crate) data_set: Option<String>,
    /// The settings variant the operation addressed; `null` when it is
    /// schema-wide.
    pub(crate) variant: Option<String>,
    pub(crate) items: Vec<DcsEditItemData>,
    /// True when the template file was rewritten.
    pub(crate) changed: bool,
    /// The edit commits only through post-validation; this records that the
    /// validator actually ran over the written schema.
    pub(crate) validated: bool,
    pub(crate) mutation: MutationData,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DcsEditItemData {
    /// One value from the request; a batch request yields one entry per value.
    pub(crate) value: String,
    pub(crate) applied: bool,
    /// Why the item changed nothing; `null` when it applied.
    pub(crate) reason: Option<String>,
}

pub(crate) struct DcsEditExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<DcsEditData>,
}

pub(crate) fn edit_dcs(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    edit_dcs_with_data(args, context).outcome
}

pub(crate) fn edit_dcs_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> DcsEditExecution {
    let edit_result = (|| -> Result<(DcsEditData, PathBuf, bool, Vec<String>), String> {
        let template_path = resolve_dcs_validate_path(args, context)?;
        let operation = required_string(args, &["operation", "Operation"], "Operation")?;
        let value_arg = required_string(args, &["value", "Value"], "Value")?;
        let data_set = string_arg(args, &["dataSet", "DataSet"]).unwrap_or("");
        let variant = string_arg(args, &["variant", "Variant"]).unwrap_or("");
        let no_selection = bool_arg(args, &["noSelection", "NoSelection"]);
        let show_validation = !bool_arg(args, &["noValidate", "NoValidate"]);

        let source = read_utf8_sig_snapshot(&template_path)?;
        let original_bytes = source.raw;
        let mut xml_text = source.text;
        let document =
            Document::parse(&xml_text).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
        require_dcs_root(document.root_element()).map_err(|error| format!("[ERROR] {error}"))?;

        let original_line_ending = if xml_text.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let original_xml_text = xml_text.clone();
        #[cfg(test)]
        run_dcs_edit_after_read_hook(&template_path);
        let base_dir = template_path.parent().unwrap_or(context.cwd.as_path());
        let values = dcs_edit_split_values(operation, value_arg);
        let mut force_save = false;
        let mut stdout = String::new();
        let mut query_inputs = Vec::new();
        let mut items: Vec<DcsEditItemData> = Vec::new();
        for value in values {
            // The verdict comes from the schema itself: an item that left the
            // XML untouched changed nothing, whatever the branch intended.
            let before_text = xml_text.clone();
            let before_force_save = force_save;
            let item_value = value.clone();
            match operation {
                "add-field" => dcs_edit_add_field(
                    &mut xml_text,
                    data_set,
                    variant,
                    &value,
                    no_selection,
                    &mut stdout,
                )?,
                "add-total" => {
                    let (key, expression) = value
                        .split_once(':')
                        .map(|(left, right)| (left.trim(), right.trim()))
                        .unwrap_or((value.trim(), ""));
                    let expression = dcs_edit_total_expression(key, expression);
                    dcs_edit_add_top_level_fragment(
                        &mut xml_text,
                        "totalField",
                        "dataPath",
                        key,
                        &dcs_edit_total_fragment(key, &expression),
                        &format!("[OK] TotalField \"{key}\" = {expression} added\n"),
                        &mut stdout,
                    )?;
                }
                "add-calculated-field" => {
                    let parsed = dcs_edit_parse_calc_field(&value);
                    let fragment = dcs_edit_calc_field_fragment(&parsed, "\t")?;
                    dcs_edit_add_top_level_fragment(
                        &mut xml_text,
                        "calculatedField",
                        "dataPath",
                        &parsed.data_path,
                        &fragment,
                        &format!(
                            "[OK] CalculatedField \"{}\" = {} added\n",
                            parsed.data_path, parsed.expression
                        ),
                        &mut stdout,
                    )?;
                    if !no_selection {
                        let fragment = dcs_edit_selection_fragment(&parsed.data_path, "\t\t\t");
                        if dcs_edit_insert_prefixed_item(
                            &mut xml_text,
                            variant,
                            "dcsset:selection",
                            &fragment,
                        )
                        .is_ok()
                        {}
                    }
                }
                "add-parameter" => {
                    let parsed = dcs_edit_parse_parameter(&value);
                    let fragment = dcs_edit_parameter_fragment(&parsed, "\t")?;
                    dcs_edit_add_top_level_fragment(
                        &mut xml_text,
                        "parameter",
                        "name",
                        &parsed.name,
                        &fragment,
                        &format!("[OK] Parameter \"{}\" added\n", parsed.name),
                        &mut stdout,
                    )?;
                    if parsed.auto_dates {
                        for suffix in ["ДатаНачала", "ДатаОкончания"] {
                            let auto = DcsEditParameter {
                                name: suffix.to_string(),
                                title: if suffix == "ДатаНачала" {
                                    "Начало периода".to_string()
                                } else {
                                    "Конец периода".to_string()
                                },
                                type_name: "dateTime".to_string(),
                                values: vec!["0001-01-01T00:00:00".to_string()],
                                hidden: true,
                                always: false,
                                value_list_allowed: false,
                                available_values: Vec::new(),
                                auto_dates: false,
                                expression: Some(format!("&{}.{}", parsed.name, suffix)),
                                type_declared: true,
                            };
                            let auto_fragment = dcs_edit_parameter_fragment(&auto, "\t")?;
                            let _ = dcs_edit_add_top_level_fragment(
                                &mut xml_text,
                                "parameter",
                                "name",
                                &auto.name,
                                &auto_fragment,
                                "",
                                &mut String::new(),
                            );
                        }
                    }
                }
                "add-filter" => {
                    let parsed = dcs_edit_parse_filter(&value);
                    dcs_edit_validate_filter_literal(&parsed)?;
                    let indent = dcs_edit_settings_container_child_indent(
                        &xml_text,
                        variant,
                        "dcsset:filter",
                    )
                    .unwrap_or_else(|_| "\t\t\t".to_string());
                    let fragment = dcs_edit_filter_fragment(&parsed, &indent);
                    dcs_edit_insert_or_create_settings_item(
                        &mut xml_text,
                        variant,
                        "dcsset:filter",
                        &fragment,
                    )?;
                }
                "add-dataParameter" => {
                    let parsed = dcs_edit_parse_data_parameter(&value);
                    let indent = dcs_edit_settings_container_child_indent(
                        &xml_text,
                        variant,
                        "dcsset:dataParameters",
                    )
                    .unwrap_or_else(|_| "\t\t\t\t".to_string());
                    let fragment = dcs_edit_data_parameter_fragment(&parsed, &indent)?;
                    dcs_edit_insert_or_create_settings_item(
                        &mut xml_text,
                        variant,
                        "dcsset:dataParameters",
                        &fragment,
                    )?;
                }
                "set-query" => {
                    let query = dcs_compile_resolve_query_value_with_inputs(
                        &value,
                        base_dir,
                        &context.cwd,
                        &mut query_inputs,
                    )?;
                    dcs_edit_set_query(&mut xml_text, data_set, &query)?;
                }
                "patch-query" => {
                    let (value, once) = dcs_edit_extract_once_marker(&value);
                    let Some((old, new)) = value.split_once(" => ") else {
                        return Err(
                            "patch-query value must contain ' => ' separator: old => new"
                                .to_string(),
                        );
                    };
                    dcs_edit_patch_query(&mut xml_text, data_set, old, new, once)?;
                }
                "clear-selection" => {
                    dcs_edit_clear_prefixed_container(&mut xml_text, variant, "dcsset:selection")?;
                }
                "clear-order" => {
                    dcs_edit_clear_prefixed_container(&mut xml_text, variant, "dcsset:order")?;
                }
                "clear-filter" => {
                    dcs_edit_clear_prefixed_container(&mut xml_text, variant, "dcsset:filter")?;
                }
                "clear-conditionalAppearance" => {
                    dcs_edit_clear_prefixed_container(
                        &mut xml_text,
                        variant,
                        "dcsset:conditionalAppearance",
                    )?;
                }
                "add-selection" => {
                    let parsed = dcs_edit_parse_selection_value(&value);
                    if let Some(group_name) = &parsed.group {
                        if dcs_edit_insert_selection_into_group(
                            &mut xml_text,
                            variant,
                            group_name,
                            &parsed.field,
                        )? {
                        } else {
                            let indent = dcs_edit_settings_container_child_indent(
                                &xml_text,
                                variant,
                                "dcsset:selection",
                            )
                            .unwrap_or_else(|_| "\t\t\t\t".to_string());
                            let fragment = dcs_edit_selection_fragment(&parsed.field, &indent);
                            dcs_edit_insert_or_create_settings_item(
                                &mut xml_text,
                                variant,
                                "dcsset:selection",
                                &fragment,
                            )?;
                        }
                    } else {
                        let indent = dcs_edit_settings_container_child_indent(
                            &xml_text,
                            variant,
                            "dcsset:selection",
                        )
                        .unwrap_or_else(|_| "\t\t\t\t".to_string());
                        let fragment = dcs_edit_selection_fragment(&parsed.field, &indent);
                        dcs_edit_insert_or_create_settings_item(
                            &mut xml_text,
                            variant,
                            "dcsset:selection",
                            &fragment,
                        )?;
                    }
                }
                "add-order" => {
                    let indent = dcs_edit_settings_container_child_indent(
                        &xml_text,
                        variant,
                        "dcsset:order",
                    )
                    .unwrap_or_else(|_| "\t\t\t\t".to_string());
                    let fragment = dcs_edit_order_fragment(&value, &indent);
                    dcs_edit_insert_or_create_settings_item(
                        &mut xml_text,
                        variant,
                        "dcsset:order",
                        &fragment,
                    )?;
                }
                "add-dataSetLink" => {
                    let parsed = dcs_edit_parse_data_set_link(&value)?;
                    let fragment = dcs_edit_data_set_link_fragment(&parsed, "\t");
                    dcs_edit_insert_top_level_fragment(&mut xml_text, "dataSetLink", &fragment)?;
                    let mut desc = format!(
                        "{} > {} on {} = {}",
                        parsed.source, parsed.dest, parsed.source_expr, parsed.dest_expr
                    );
                    if !parsed.parameter.is_empty() {
                        desc.push_str(&format!(" [param {}]", parsed.parameter));
                    }
                }
                "add-dataSet" => {
                    let parsed = dcs_edit_parse_data_set_with_inputs(
                        &value,
                        base_dir,
                        &context.cwd,
                        &mut query_inputs,
                    )?;
                    if dcs_edit_top_level_contains(&xml_text, "dataSet", "name", &parsed.name) {
                    } else {
                        let source = dcs_edit_first_data_source(&xml_text)
                            .unwrap_or_else(|| "ИсточникДанных1".to_string());
                        let fragment = dcs_edit_data_set_fragment(&parsed, &source, "\t");
                        dcs_edit_insert_top_level_fragment(&mut xml_text, "dataSet", &fragment)?;
                    }
                }
                "add-variant" => {
                    let parsed = dcs_edit_parse_variant(&value);
                    if dcs_edit_variant_exists(&xml_text, &parsed.name) {
                    } else {
                        let fragment = dcs_edit_variant_fragment(&parsed, "\t");
                        dcs_edit_insert_before_root_close(&mut xml_text, &fragment)?;
                    }
                }
                "add-conditionalAppearance" => {
                    let parsed = dcs_edit_parse_conditional_appearance(&value);
                    for filter in &parsed.filters {
                        dcs_edit_validate_filter_literal(filter)?;
                    }
                    let indent = dcs_edit_settings_container_child_indent(
                        &xml_text,
                        variant,
                        "dcsset:conditionalAppearance",
                    )
                    .unwrap_or_else(|_| "\t\t\t\t".to_string());
                    let fragment = dcs_edit_conditional_appearance_fragment(&parsed, &indent);
                    dcs_edit_insert_or_create_settings_item(
                        &mut xml_text,
                        variant,
                        "dcsset:conditionalAppearance",
                        &fragment,
                    )?;
                }
                "add-drilldown" => {
                    // Only a real insertion counts: forcing the save
                    // unconditionally reported `applied` for a resource the
                    // schema has no named template for.
                    if matches!(
                        dcs_edit_add_drilldown(&mut xml_text, &value),
                        DcsEditDrilldownResult::Added
                    ) {
                        force_save = true;
                    }
                }
                "set-outputParameter" => {
                    let parsed = dcs_edit_parse_output_parameter(&value)?;
                    if let Ok(range) = dcs_edit_prefixed_container_range(
                        &xml_text,
                        variant,
                        "dcsset:outputParameters",
                    ) {
                        dcs_edit_remove_item_by_child(
                            &mut xml_text,
                            (range.start, range.end),
                            "dcscor:item",
                            "dcscor:parameter",
                            &parsed.key,
                        )?;
                    }
                    let indent = dcs_edit_settings_container_child_indent(
                        &xml_text,
                        variant,
                        "dcsset:outputParameters",
                    )
                    .unwrap_or_else(|_| "\t\t\t\t".to_string());
                    let fragment = dcs_edit_output_parameter_fragment(&parsed, &indent)?;
                    dcs_edit_insert_or_create_settings_item(
                        &mut xml_text,
                        variant,
                        "dcsset:outputParameters",
                        &fragment,
                    )?;
                }
                "set-structure" => {
                    let parsed = dcs_edit_parse_structure(&value);
                    let fragments = dcs_edit_structure_fragments(&parsed, "\t\t\t");
                    dcs_edit_replace_structure(&mut xml_text, variant, &fragments)?;
                }
                "modify-structure" => {
                    let parsed = dcs_edit_parse_structure(&value);
                    dcs_edit_modify_structure(&mut xml_text, variant, &parsed, &mut stdout)?;
                }
                "remove-field" => {
                    dcs_edit_remove_dataset_item(
                        &mut xml_text,
                        data_set,
                        "field",
                        "dataPath",
                        &value,
                    )?;
                    dcs_edit_remove_prefixed_selection_field(&mut xml_text, variant, &value)?;
                }
                "remove-parameter" => {
                    dcs_edit_remove_top_level_item(&mut xml_text, "parameter", "name", &value)?;
                }
                "modify-field" => {
                    let parsed = dcs_edit_parse_field(&value);
                    dcs_edit_replace_dataset_field(&mut xml_text, data_set, &parsed)?;
                }
                "set-field-role" => {
                    dcs_edit_set_field_role(&mut xml_text, data_set, &value, &mut stdout)?;
                }
                "modify-filter" => {
                    let parsed = dcs_edit_parse_filter(&value);
                    dcs_edit_validate_filter_literal(&parsed)?;
                    force_save |=
                        dcs_edit_modify_filter(&mut xml_text, variant, &parsed, &mut stdout)?;
                }
                "modify-dataParameter" => {
                    let parsed = dcs_edit_parse_data_parameter(&value);
                    force_save |= dcs_edit_modify_data_parameter(
                        &mut xml_text,
                        variant,
                        &parsed,
                        &mut stdout,
                    )?;
                }
                "modify-parameter" => {
                    let parsed = dcs_edit_parse_parameter_patch(&value);
                    dcs_edit_modify_parameter(&mut xml_text, &parsed, &mut stdout)?;
                }
                "rename-parameter" => {
                    dcs_edit_rename_parameter(&mut xml_text, &value, &mut stdout)?;
                }
                "reorder-parameters" => {
                    dcs_edit_reorder_parameters(&mut xml_text, &value, &mut stdout)?;
                }
                "remove-total" => {
                    dcs_edit_remove_top_level_item(
                        &mut xml_text,
                        "totalField",
                        "dataPath",
                        &value,
                    )?;
                }
                "remove-calculated-field" => {
                    dcs_edit_remove_top_level_item(
                        &mut xml_text,
                        "calculatedField",
                        "dataPath",
                        &value,
                    )?;
                    dcs_edit_remove_prefixed_selection_field(&mut xml_text, variant, &value)?;
                }
                "remove-filter" => {
                    let filter_range =
                        dcs_edit_prefixed_container_range(&xml_text, variant, "dcsset:filter")?;
                    dcs_edit_remove_item_by_child(
                        &mut xml_text,
                        (filter_range.start, filter_range.end),
                        "dcsset:item",
                        "dcsset:left",
                        &value,
                    )?;
                }
                other => {
                    return Err(format!(
                        "native dcs-edit does not support Operation '{other}' yet"
                    ));
                }
            }
            let applied = xml_text != before_text || force_save != before_force_save;
            items.push(DcsEditItemData {
                value: item_value,
                applied,
                reason: (!applied)
                    .then(|| "цель не найдена или уже в нужном состоянии".to_string()),
            });
        }

        let changed = force_save || xml_text != original_xml_text;
        let mut warnings = Vec::new();
        let mut validated = false;
        if changed {
            let mut xml_text = xml_text.replacen("encoding=\"UTF-8\"", "encoding=\"utf-8\"", 1);
            if original_line_ending == "\r\n" {
                xml_text = xml_text.replace("\r\n", "\n").replace('\n', "\r\n");
            } else {
                xml_text = xml_text.replace("\r\n", "\n");
            }
            if !xml_text.ends_with('\n') {
                xml_text.push_str(original_line_ending);
            }
            let mut transaction = CompileTransaction::new();
            for input in &query_inputs {
                input.bind_to(&mut transaction)?;
            }
            transaction.replace_bytes(
                &template_path,
                &original_bytes,
                utf8_bom_bytes(&xml_text),
            )?;
            guard_active_format_owner(&mut transaction, &template_path, context)?;
            guard_active_format_owner_with_exact_root(
                &mut transaction,
                &template_path,
                context,
                DCS_ROOT,
            )?;
            let mut validation_stdout = None;
            let report = transaction.commit_with_post_validation(|| {
                let validation = require_dcs_post_validation(&template_path, context)?;
                validation_stdout = Some(validation);
                Ok(())
            })?;
            validated = true;
            warnings = report.cleanup_warnings;
            let _ = validation_stdout;
        }
        let _ = (stdout, show_validation);
        let data = DcsEditData {
            operation: operation.to_string(),
            data_set: (!data_set.is_empty()).then(|| data_set.to_string()),
            variant: (!variant.is_empty()).then(|| variant.to_string()),
            items,
            changed,
            validated,
            mutation: if changed {
                MutationData::new(true).updated(&template_path)
            } else {
                MutationData::new(false)
            },
        };
        Ok((data, template_path, changed, warnings))
    })();

    match edit_result {
        Ok((data, template_path, changed, warnings)) => DcsEditExecution {
            outcome: AdapterOutcome {
                ok: true,
                summary: format!(
                    "unica.dcs.edit applied {} of {} item(s) with {}",
                    data.items.iter().filter(|item| item.applied).count(),
                    data.items.len(),
                    data.operation
                ),
                changes: if changed {
                    vec![format!("updated {}", template_path.display())]
                } else {
                    Vec::new()
                },
                warnings,
                errors: Vec::new(),
                artifacts: vec![template_path.display().to_string()],
                stdout: None,
                stderr: None,
                command: None,
            },
            data: Some(data),
        },
        Err(error) => DcsEditExecution {
            outcome: AdapterOutcome {
                ok: false,
                summary: "unica.dcs.edit failed in native DCS editor".to_string(),
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

pub(crate) fn dcs_edit_split_values(operation: &str, value: &str) -> Vec<String> {
    if matches!(
        operation,
        "set-query" | "set-structure" | "modify-structure" | "add-dataSet"
    ) {
        return vec![value.to_string()];
    }
    if operation == "add-drilldown" && !value.contains(";;") {
        return value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToOwned::to_owned)
            .collect();
    }
    value
        .split(";;")
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(crate) fn dcs_edit_add_field(
    xml_text: &mut String,
    data_set: &str,
    variant: &str,
    value: &str,
    no_selection: bool,
    stdout: &mut String,
) -> Result<(), String> {
    let parsed = dcs_edit_parse_field(value);
    if parsed.type_declared {
        dcs_compile_parse_value_type(&parsed.field_type)?;
    }
    let DcsEditDataSetTarget {
        range,
        name: data_set_name,
        emit_field_value_type,
        field_insert_pos,
        child_indent,
    } = dcs_edit_dataset_target(xml_text, data_set)?;
    if dcs_edit_find_item_by_child(xml_text, range, "field", "dataPath", &parsed.data_path)
        .is_some()
    {
        stdout.push_str(&format!(
            "[WARN] Field \"{}\" already exists in dataset \"{}\" -- skipped\n",
            parsed.data_path, data_set_name
        ));
        return Ok(());
    }

    let mut lines = Vec::new();
    dcs_edit_emit_field(&mut lines, &parsed, &child_indent, emit_field_value_type)?;
    xml_text.insert_str(field_insert_pos, &format!("{}\n", lines.join("\n")));
    stdout.push_str(&format!(
        "[OK] Field \"{}\" added to dataset \"{}\"\n",
        parsed.data_path, data_set_name
    ));

    if !no_selection {
        let selection_indent =
            dcs_edit_settings_container_child_indent(xml_text, variant, "dcsset:selection")
                .unwrap_or_else(|_| "\t\t\t\t".to_string());
        let fragment = dcs_edit_selection_fragment(&parsed.data_path, &selection_indent);
        if dcs_edit_prefixed_container_contains_field(
            xml_text,
            variant,
            "dcsset:selection",
            &parsed.data_path,
        ) {
            stdout.push_str(&format!(
                "[INFO] Field \"{}\" already in selection -- skipped\n",
                parsed.data_path
            ));
        } else if dcs_edit_insert_prefixed_item(xml_text, variant, "dcsset:selection", &fragment)
            .is_ok()
        {
            stdout.push_str(&format!(
                "[OK] Field \"{}\" added to selection of variant \"{}\"\n",
                parsed.data_path,
                dcs_edit_variant_name(xml_text, variant).unwrap_or_else(|| variant.to_string())
            ));
        }
    }
    Ok(())
}

pub(crate) fn dcs_edit_add_top_level(
    xml_text: &mut String,
    item: &str,
    child: &str,
    value: &str,
    stdout: &mut String,
    build: fn(&str, &str) -> String,
) -> Result<(), String> {
    let (key, expression) = value
        .split_once(':')
        .map(|(left, right)| (left.trim(), right.trim()))
        .unwrap_or((value.trim(), ""));
    dcs_edit_add_top_level_fragment(
        xml_text,
        item,
        child,
        key,
        &build(key, expression),
        &format!("[OK] {} \"{}\" added\n", item, key),
        stdout,
    )
}

pub(crate) fn dcs_edit_add_top_level_fragment(
    xml_text: &mut String,
    item: &str,
    child: &str,
    key: &str,
    fragment: &str,
    ok_message: &str,
    stdout: &mut String,
) -> Result<(), String> {
    if dcs_edit_top_level_contains(xml_text, item, child, key) {
        stdout.push_str(&format!(
            "[WARN] {} \"{}\" already exists -- skipped\n",
            item, key
        ));
        return Ok(());
    }
    dcs_edit_insert_top_level_fragment(xml_text, item, fragment)?;
    stdout.push_str(ok_message);
    Ok(())
}

pub(crate) fn dcs_edit_insert_top_level_fragment(
    xml_text: &mut String,
    item: &str,
    fragment: &str,
) -> Result<(), String> {
    let root_range = Document::parse(xml_text)
        .map_err(|error| format!("XML parse error: {error}"))?
        .root_element()
        .range();
    let insert_pos = dcs_edit_canonical_child_insert_pos(
        xml_text,
        (root_range.start, root_range.end),
        item,
        DCS_EDIT_ROOT_CHILD_SEQUENCE,
    )?;
    xml_text.insert_str(insert_pos, &format!("{fragment}\n"));
    Ok(())
}

const DCS_EDIT_ROOT_CHILD_SEQUENCE: &[&str] = &[
    "dataSource",
    "dataSet",
    "dataSetLink",
    "calculatedField",
    "totalField",
    "parameter",
    "nestedSchema",
    "template",
    "fieldTemplate",
    "groupTemplate",
    "groupHeaderTemplate",
    "totalFieldsTemplate",
    "defaultSettings",
    "settingsVariant",
];

const DCS_EDIT_SETTINGS_CHILD_SEQUENCE: &[&str] = &[
    "userFields",
    "selection",
    "filter",
    "dataParameters",
    "order",
    "conditionalAppearance",
    "outputParameters",
    "item",
    "additionalProperties",
    "itemsViewMode",
    "itemsUserSettingID",
    "itemsUserSettingPresentation",
];

const DCS_EDIT_DATA_SET_FIELD_CHILD_SEQUENCE: &[&str] = &[
    "dataPath",
    "field",
    "title",
    "useRestriction",
    "attributeUseRestriction",
    "role",
    "presentationExpression",
    "orderExpression",
    "inHierarchyDataSet",
    "inHierarchyDataSetParameter",
    "valueType",
    "appearance",
    "availableValue",
    "inputParameters",
];

const DCS_EDIT_FIELD_ROLE_CHILD_SEQUENCE: &[&str] = &[
    "periodNumber",
    "periodType",
    "dimension",
    "parentDimension",
    "account",
    "accountTypeExpression",
    "balance",
    "balanceGroupName",
    "balanceType",
    "accountingBalanceType",
    "accountField",
    "ignoreNullValues",
    "required",
    "dimensionAttribute",
];

const DCS_EDIT_FILTER_ITEM_COMPARISON_CHILD_SEQUENCE: &[&str] = &[
    "use",
    "left",
    "comparisonType",
    "right",
    "presentation",
    "application",
    "viewMode",
    "userSettingID",
    "userSettingPresentation",
];

const DCS_EDIT_SETTINGS_PARAMETER_VALUE_CHILD_SEQUENCE: &[&str] = &[
    "use",
    "parameter",
    "value",
    "item",
    "viewMode",
    "userSettingID",
    "userSettingPresentation",
];

const DCS_EDIT_PARAMETER_CHILD_SEQUENCE: &[&str] = &[
    "name",
    "title",
    "valueType",
    "value",
    "useRestriction",
    "expression",
    "availableValue",
    "valueListAllowed",
    "availableAsField",
    "functionalOptionsParameter",
    "inputParameters",
    "denyIncompleteValues",
    "use",
];

const DCS_EDIT_STRUCTURE_GROUP_CHILD_SEQUENCE: &[&str] = &[
    "use",
    "name",
    "groupItems",
    "filter",
    "order",
    "selection",
    "conditionalAppearance",
    "outputParameters",
    "item",
    "id",
    "viewMode",
    "userSettingID",
    "userSettingPresentation",
    "itemsViewMode",
    "itemsUserSettingID",
    "itemsUserSettingPresentation",
    "groupState",
];

pub(crate) fn dcs_edit_canonical_child_insert_pos(
    xml_text: &str,
    parent_range: (usize, usize),
    target_name: &str,
    sequence: &[&str],
) -> Result<usize, String> {
    let target_name = target_name.rsplit(':').next().unwrap_or(target_name);
    let target_rank = sequence
        .iter()
        .position(|name| *name == target_name)
        .ok_or_else(|| format!("'{target_name}' is not in the fixed DCS 8.3.27 XSD sequence"))?;
    let document =
        Document::parse(xml_text).map_err(|error| format!("XML parse error: {error}"))?;
    let parent = document
        .descendants()
        .find(|node| node.is_element() && node.range().start == parent_range.0)
        .ok_or_else(|| format!("DCS parent element at byte {} not found", parent_range.0))?;

    for child in parent.children().filter(roxmltree::Node::is_element) {
        let Some(child_rank) = sequence
            .iter()
            .position(|name| *name == child.tag_name().name())
        else {
            continue;
        };
        if child_rank > target_rank {
            return Ok(dcs_edit_line_start(xml_text, child.range().start));
        }
    }

    let parent_node_range = parent.range();
    let close_rel = xml_text[parent_node_range.start..parent_node_range.end]
        .rfind("</")
        .ok_or_else(|| {
            format!("Cannot insert '{target_name}' into self-closing DCS parent element")
        })?;
    Ok(dcs_edit_line_start(
        xml_text,
        parent_node_range.start + close_rel,
    ))
}

pub(crate) fn dcs_edit_top_level_contains(
    xml_text: &str,
    item: &str,
    child: &str,
    key: &str,
) -> bool {
    let Ok(document) = Document::parse(xml_text) else {
        return false;
    };
    let root = document.root_element();
    root.children()
        .filter(|node| {
            node.is_element() && dcs_edit_requested_name_matches(*node, item, Some(DCS_SCHEMA_NS))
        })
        .any(|node| {
            node.children()
                .filter(roxmltree::Node::is_element)
                .any(|candidate| {
                    dcs_edit_requested_name_matches(candidate, child, node.tag_name().namespace())
                        && dcs_text_of(candidate) == key
                })
        })
}

pub(crate) fn dcs_edit_insert_before_root_close(
    xml_text: &mut String,
    fragment: &str,
) -> Result<(), String> {
    let document =
        Document::parse(xml_text).map_err(|error| format!("XML parse error: {error}"))?;
    let root = document.root_element();
    require_dcs_root(root)?;
    let root_range = root.range();
    let pos = dcs_edit_element_content_range(xml_text, (root_range.start, root_range.end))
        .ok_or_else(|| "Malformed DataCompositionSchema root element".to_string())?
        .end;
    xml_text.insert_str(pos, &format!("{fragment}\n"));
    Ok(())
}

pub(crate) fn dcs_edit_total_fragment(data_path: &str, expression: &str) -> String {
    let expression = dcs_edit_total_expression(data_path, expression);
    format!(
        "\t<totalField>\n\t\t<dataPath>{}</dataPath>\n\t\t<expression>{}</expression>\n\t</totalField>",
        escape_xml(data_path),
        escape_xml(&expression)
    )
}

pub(crate) fn dcs_edit_total_expression(data_path: &str, expression: &str) -> String {
    let expression = expression.trim();
    if expression.is_empty() {
        return format!("Сумма({data_path})");
    }
    if matches!(
        expression,
        "Сумма"
            | "Количество"
            | "Минимум"
            | "Максимум"
            | "Среднее"
            | "Sum"
            | "Count"
            | "Min"
            | "Max"
            | "Avg"
            | "Minimum"
            | "Maximum"
            | "Average"
    ) {
        return format!("{expression}({data_path})");
    }
    expression.to_string()
}

pub(crate) struct DcsEditCalcField {
    pub(crate) data_path: String,
    pub(crate) title: String,
    pub(crate) field_type: String,
    pub(crate) expression: String,
    pub(crate) type_declared: bool,
}

pub(crate) fn dcs_edit_parse_calc_field(value: &str) -> DcsEditCalcField {
    let (left, expression) = value
        .split_once('=')
        .map(|(left, right)| (left.trim(), right.trim()))
        .unwrap_or((value.trim(), ""));
    let (mut name_type, title) = dcs_edit_extract_bracket_title(left);
    name_type = dcs_edit_strip_markers(&name_type);
    let (data_path, field_type, type_declared) = name_type
        .split_once(':')
        .map(|(name, type_name)| {
            (
                name.trim().to_string(),
                dcs_compile_resolve_type(type_name.trim()),
                true,
            )
        })
        .unwrap_or((name_type.trim().to_string(), String::new(), false));
    DcsEditCalcField {
        data_path,
        title,
        field_type,
        expression: expression.to_string(),
        type_declared,
    }
}

pub(crate) fn dcs_edit_calc_field_fragment(
    field: &DcsEditCalcField,
    indent: &str,
) -> Result<String, String> {
    let value_type_entries = field
        .type_declared
        .then(|| dcs_compile_parse_value_type(&field.field_type))
        .transpose()?;
    let mut lines = vec![
        format!("{indent}<calculatedField>"),
        format!(
            "{indent}\t<dataPath>{}</dataPath>",
            escape_xml(&field.data_path)
        ),
        format!(
            "{indent}\t<expression>{}</expression>",
            escape_xml(&field.expression)
        ),
    ];
    if !field.title.is_empty() {
        dcs_compile_emit_mltext(&mut lines, &format!("{indent}\t"), "title", &field.title);
    }
    if let Some(entries) = value_type_entries {
        lines.push(format!("{indent}\t<valueType>"));
        dcs_compile_emit_value_type_entries(&mut lines, &entries, &format!("{indent}\t\t"));
        lines.push(format!("{indent}\t</valueType>"));
    }
    lines.push(format!("{indent}</calculatedField>"));
    Ok(lines.join("\n"))
}

pub(crate) struct DcsEditParameter {
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) type_name: String,
    pub(crate) values: Vec<String>,
    pub(crate) hidden: bool,
    pub(crate) always: bool,
    pub(crate) value_list_allowed: bool,
    pub(crate) available_values: Vec<(String, String)>,
    pub(crate) auto_dates: bool,
    pub(crate) expression: Option<String>,
    pub(crate) type_declared: bool,
}

pub(crate) fn dcs_edit_parse_parameter(value: &str) -> DcsEditParameter {
    let auto_dates = value.contains("@autoDates");
    let hidden = value.contains("@hidden");
    let always = value.contains("@always");
    let value_list_allowed = value.contains("@valueList");
    let available_values = dcs_edit_extract_available_values(value);
    let cleaned = value
        .split("availableValue=")
        .next()
        .unwrap_or(value)
        .replace("@autoDates", "")
        .replace("@hidden", "")
        .replace("@always", "")
        .replace("@valueList", "");
    let (left, values, value_list_allowed) = cleaned
        .split_once('=')
        .map(|(left, right)| {
            let values = dcs_edit_parse_value_list(right.trim());
            let value_list_allowed = value_list_allowed || values.len() >= 2;
            (left.trim(), values, value_list_allowed)
        })
        .unwrap_or((cleaned.trim(), Vec::new(), value_list_allowed));
    let (mut name_type, title) = dcs_edit_extract_bracket_title(left);
    name_type = dcs_edit_strip_markers(&name_type);
    let (name, type_name, type_declared) = name_type
        .split_once(':')
        .map(|(name, type_name)| {
            (
                name.trim().to_string(),
                dcs_compile_resolve_type(type_name.trim()),
                true,
            )
        })
        .unwrap_or((name_type.trim().to_string(), String::new(), false));
    DcsEditParameter {
        name,
        title,
        type_name,
        values,
        hidden,
        always,
        value_list_allowed,
        available_values,
        auto_dates,
        expression: None,
        type_declared,
    }
}

pub(crate) fn dcs_edit_parameter_fragment(
    param: &DcsEditParameter,
    indent: &str,
) -> Result<String, String> {
    let value_type_entries = param
        .type_declared
        .then(|| dcs_compile_parse_value_type(&param.type_name))
        .transpose()?;
    let mut lines = vec![
        format!("{indent}<parameter>"),
        format!("{indent}\t<name>{}</name>", escape_xml(&param.name)),
    ];
    if !param.title.is_empty() {
        dcs_compile_emit_mltext(&mut lines, &format!("{indent}\t"), "title", &param.title);
    }
    if let Some(entries) = value_type_entries {
        lines.push(format!("{indent}\t<valueType>"));
        dcs_compile_emit_value_type_entries(&mut lines, &entries, &format!("{indent}\t\t"));
        lines.push(format!("{indent}\t</valueType>"));
    }
    for value in &param.values {
        lines.extend(dcs_edit_parameter_value_lines(
            &param.type_name,
            value,
            &format!("{indent}\t"),
            "value",
        )?);
    }
    if param.hidden {
        lines.push(format!("{indent}\t<useRestriction>true</useRestriction>"));
    }
    if let Some(expression) = &param.expression {
        lines.push(format!(
            "{indent}\t<expression>{}</expression>",
            escape_xml(expression)
        ));
    }
    if !param.available_values.is_empty() {
        for (value, presentation) in &param.available_values {
            lines.push(format!("{indent}\t<availableValue>"));
            lines.extend(dcs_edit_parameter_value_lines(
                &param.type_name,
                value,
                &format!("{indent}\t\t"),
                "value",
            )?);
            if !presentation.is_empty() {
                dcs_compile_emit_mltext(
                    &mut lines,
                    &format!("{indent}\t\t"),
                    "presentation",
                    presentation,
                );
            }
            lines.push(format!("{indent}\t</availableValue>"));
        }
    }
    if param.value_list_allowed {
        lines.push(format!(
            "{indent}\t<valueListAllowed>true</valueListAllowed>"
        ));
    }
    if param.hidden {
        lines.push(format!(
            "{indent}\t<availableAsField>false</availableAsField>"
        ));
    }
    if param.always {
        lines.push(format!("{indent}\t<use>Always</use>"));
    }
    lines.push(format!("{indent}</parameter>"));
    Ok(lines.join("\n"))
}

pub(crate) fn dcs_edit_parameter_value_lines(
    declared_type: &str,
    value: &str,
    indent: &str,
    tag_name: &str,
) -> Result<Vec<String>, String> {
    let declared_type = dcs_edit_normalize_declared_type(declared_type);
    if dcs_edit_is_empty_value(value) {
        return Ok(vec![format!("{indent}<{tag_name} xsi:nil=\"true\"/>")]);
    }
    if declared_type == "StandardPeriod" {
        if !dcs_edit_is_standard_period_variant(value) {
            return Err(format!(
                "Value '{value}' is not a valid v8:StandardPeriodVariant for the fixed DCS 8.3.27 XSD contract"
            ));
        }
        let mut lines = vec![
            format!("{indent}<{tag_name} xsi:type=\"v8:StandardPeriod\">"),
            format!(
                "{indent}\t<v8:variant xsi:type=\"v8:StandardPeriodVariant\">{}</v8:variant>",
                escape_xml(value)
            ),
        ];
        if value == "Custom" {
            lines.push(format!(
                "{indent}\t<v8:startDate>0001-01-01T00:00:00</v8:startDate>"
            ));
            lines.push(format!(
                "{indent}\t<v8:endDate>0001-01-01T00:00:00</v8:endDate>"
            ));
        }
        lines.push(format!("{indent}</{tag_name}>"));
        return Ok(lines);
    }
    let xsi_type = if declared_type.starts_with("date") {
        if !is_date_time_literal(value) {
            return Err(format!(
                "Value '{value}' is not a valid xs:dateTime literal for the fixed DCS 8.3.27 XSD contract"
            ));
        }
        "xs:dateTime"
    } else if declared_type == "boolean" {
        if !matches!(value, "true" | "false" | "0" | "1") {
            return Err(format!(
                "Value '{value}' is not a valid xs:boolean literal for the fixed DCS 8.3.27 XSD contract"
            ));
        }
        "xs:boolean"
    } else if declared_type.starts_with("decimal") {
        if !dcs_edit_is_valid_xs_decimal(value) {
            return Err(format!(
                "Value '{value}' is not a valid xs:decimal literal for the fixed DCS 8.3.27 XSD contract"
            ));
        }
        "xs:decimal"
    } else if declared_type.starts_with("string") {
        "xs:string"
    } else if dcs_edit_is_design_time_type(&declared_type) {
        "dcscor:DesignTimeValue"
    } else if is_date_time_literal(value) {
        "xs:dateTime"
    } else if dcs_edit_looks_date_time(value) {
        return Err(format!(
            "Value '{value}' is not a valid xs:dateTime literal for the fixed DCS 8.3.27 XSD contract"
        ));
    } else if value == "true" || value == "false" {
        "xs:boolean"
    } else if dcs_edit_is_design_time_value(value) {
        "dcscor:DesignTimeValue"
    } else {
        "xs:string"
    };
    Ok(vec![format!(
        "{indent}<{tag_name} xsi:type=\"{xsi_type}\">{}</{tag_name}>",
        escape_xml(value)
    )])
}

pub(crate) fn dcs_edit_normalize_declared_type(value: &str) -> String {
    let value = value.trim();
    if let Some(rest) = value
        .strip_prefix("xs:")
        .or_else(|| value.strip_prefix("v8:"))
    {
        return rest.to_string();
    }
    if let Some((prefix, rest)) = value.split_once(':') {
        if prefix.starts_with('d') && prefix.contains('p') {
            return rest.to_string();
        }
    }
    value.to_string()
}

pub(crate) fn dcs_edit_is_design_time_type(value: &str) -> bool {
    [
        "CatalogRef.",
        "DocumentRef.",
        "EnumRef.",
        "ChartOfAccountsRef.",
        "ChartOfCharacteristicTypesRef.",
        "ChartOfCalculationTypesRef.",
        "BusinessProcessRef.",
        "TaskRef.",
        "InformationRegisterRef.",
        "ExchangePlanRef.",
    ]
    .iter()
    .any(|prefix| value.starts_with(prefix))
}

pub(crate) fn dcs_edit_is_design_time_value(value: &str) -> bool {
    [
        "Перечисление.",
        "Справочник.",
        "ПланСчетов.",
        "Документ.",
        "ПланВидовХарактеристик.",
        "ПланВидовРасчета.",
        "БизнесПроцесс.",
        "Задача.",
        "РегистрСведений.",
        "ПланОбмена.",
        "Catalog.",
        "Document.",
        "Enum.",
        "ChartOfAccounts.",
        "ChartOfCharacteristicTypes.",
        "ChartOfCalculationTypes.",
        "BusinessProcess.",
        "Task.",
        "InformationRegister.",
        "ExchangePlan.",
    ]
    .iter()
    .any(|prefix| value.starts_with(prefix))
}

pub(crate) struct DcsEditFilter {
    pub(crate) field: String,
    pub(crate) operator: String,
    pub(crate) value: String,
    pub(crate) value_type: String,
    pub(crate) use_flag: Option<bool>,
    pub(crate) user_setting_id: Option<String>,
    pub(crate) view_mode: Option<String>,
}

pub(crate) fn dcs_edit_parse_filter(value: &str) -> DcsEditFilter {
    let use_flag = if value.contains("@off") {
        Some(false)
    } else if value.contains("@on") {
        Some(true)
    } else {
        None
    };
    let user_setting_id = if value.contains("@user") {
        Some(fresh_uuid())
    } else {
        None
    };
    let view_mode = if value.contains("@quickAccess") {
        Some("QuickAccess".to_string())
    } else if value.contains("@normal") {
        Some("Normal".to_string())
    } else if value.contains("@inaccessible") {
        Some("Inaccessible".to_string())
    } else {
        None
    };
    let cleaned = value
        .replace("@off", "")
        .replace("@on", "")
        .replace("@user", "")
        .replace("@quickAccess", "")
        .replace("@normal", "")
        .replace("@inaccessible", "");
    let (field, operator, filter_value) = dcs_edit_parse_filter_expression(&cleaned);
    let (filter_value, value_type) = dcs_edit_filter_value_type(&filter_value);
    DcsEditFilter {
        field,
        operator,
        value: filter_value,
        value_type,
        use_flag,
        user_setting_id,
        view_mode,
    }
}

pub(crate) fn dcs_edit_filter_fragment(filter: &DcsEditFilter, indent: &str) -> String {
    let mut lines = vec![format!(
        "{indent}<dcsset:item xsi:type=\"dcsset:FilterItemComparison\">"
    )];
    if let Some(false) = filter.use_flag {
        lines.push(format!("{indent}\t<dcsset:use>false</dcsset:use>"));
    }
    lines.push(format!(
        "{indent}\t<dcsset:left xsi:type=\"dcscor:Field\">{}</dcsset:left>",
        escape_xml(&filter.field)
    ));
    lines.push(format!(
        "{indent}\t<dcsset:comparisonType>{}</dcsset:comparisonType>",
        escape_xml(&filter.operator)
    ));
    if !filter.value.is_empty() {
        lines.push(format!(
            "{indent}\t<dcsset:right xsi:type=\"{}\">{}</dcsset:right>",
            filter.value_type,
            escape_xml(&filter.value)
        ));
    }
    if let Some(view_mode) = &filter.view_mode {
        lines.push(format!(
            "{indent}\t<dcsset:viewMode>{}</dcsset:viewMode>",
            escape_xml(view_mode)
        ));
    }
    if let Some(user_setting_id) = &filter.user_setting_id {
        lines.push(format!(
            "{indent}\t<dcsset:userSettingID>{}</dcsset:userSettingID>",
            escape_xml(user_setting_id)
        ));
    }
    lines.push(format!("{indent}</dcsset:item>"));
    lines.join("\n")
}

pub(crate) fn dcs_edit_modify_filter(
    xml_text: &mut String,
    variant: &str,
    filter: &DcsEditFilter,
    stdout: &mut String,
) -> Result<bool, String> {
    let var_name = dcs_edit_variant_name(xml_text, variant).unwrap_or_else(|| variant.to_string());
    let Ok(filter_range) = dcs_edit_prefixed_container_range(xml_text, variant, "dcsset:filter")
    else {
        stdout.push_str(&format!(
            "[WARN] No filter section in variant \"{var_name}\"\n"
        ));
        return Ok(false);
    };
    let Some(item_range) = dcs_edit_find_item_by_child(
        xml_text,
        (filter_range.open_end, filter_range.close_start),
        "dcsset:item",
        "dcsset:left",
        &filter.field,
    ) else {
        stdout.push_str(&format!(
            "[WARN] Filter for \"{}\" not found in variant \"{}\"\n",
            filter.field, var_name
        ));
        return Ok(false);
    };
    let item_indent = dcs_edit_line_indent(xml_text, item_range.0);
    let child_indent = format!("{item_indent}\t");
    dcs_edit_replace_or_insert_child_fragment(
        xml_text,
        item_range,
        "dcsset:comparisonType",
        &format!(
            "{child_indent}<dcsset:comparisonType>{}</dcsset:comparisonType>",
            escape_xml(&filter.operator)
        ),
        DCS_EDIT_FILTER_ITEM_COMPARISON_CHILD_SEQUENCE,
    )?;
    let filter_range = dcs_edit_prefixed_container_range(xml_text, variant, "dcsset:filter")?;
    let item_range = dcs_edit_find_item_by_child(
        xml_text,
        (filter_range.open_end, filter_range.close_start),
        "dcsset:item",
        "dcsset:left",
        &filter.field,
    )
    .ok_or_else(|| format!("Filter for \"{}\" not found", filter.field))?;
    if !filter.value.is_empty() {
        dcs_edit_replace_or_insert_child_fragment(
            xml_text,
            item_range,
            "dcsset:right",
            &format!(
                "{child_indent}<dcsset:right xsi:type=\"{}\">{}</dcsset:right>",
                filter.value_type,
                escape_xml(&filter.value)
            ),
            DCS_EDIT_FILTER_ITEM_COMPARISON_CHILD_SEQUENCE,
        )?;
    }
    let filter_range = dcs_edit_prefixed_container_range(xml_text, variant, "dcsset:filter")?;
    let item_range = dcs_edit_find_item_by_child(
        xml_text,
        (filter_range.open_end, filter_range.close_start),
        "dcsset:item",
        "dcsset:left",
        &filter.field,
    )
    .ok_or_else(|| format!("Filter for \"{}\" not found", filter.field))?;
    match filter.use_flag {
        Some(false) => {
            dcs_edit_replace_or_insert_child_fragment(
                xml_text,
                item_range,
                "dcsset:use",
                &format!("{child_indent}<dcsset:use>false</dcsset:use>"),
                DCS_EDIT_FILTER_ITEM_COMPARISON_CHILD_SEQUENCE,
            )?;
        }
        Some(true) => {
            let _ = dcs_edit_remove_child_element(xml_text, item_range, "dcsset:use");
        }
        None => {}
    }
    let filter_range = dcs_edit_prefixed_container_range(xml_text, variant, "dcsset:filter")?;
    let item_range = dcs_edit_find_item_by_child(
        xml_text,
        (filter_range.open_end, filter_range.close_start),
        "dcsset:item",
        "dcsset:left",
        &filter.field,
    )
    .ok_or_else(|| format!("Filter for \"{}\" not found", filter.field))?;
    if let Some(view_mode) = &filter.view_mode {
        dcs_edit_replace_or_insert_child_fragment(
            xml_text,
            item_range,
            "dcsset:viewMode",
            &format!(
                "{child_indent}<dcsset:viewMode>{}</dcsset:viewMode>",
                escape_xml(view_mode)
            ),
            DCS_EDIT_FILTER_ITEM_COMPARISON_CHILD_SEQUENCE,
        )?;
    }
    let filter_range = dcs_edit_prefixed_container_range(xml_text, variant, "dcsset:filter")?;
    let item_range = dcs_edit_find_item_by_child(
        xml_text,
        (filter_range.open_end, filter_range.close_start),
        "dcsset:item",
        "dcsset:left",
        &filter.field,
    )
    .ok_or_else(|| format!("Filter for \"{}\" not found", filter.field))?;
    if let Some(user_setting_id) = &filter.user_setting_id {
        dcs_edit_replace_or_insert_child_fragment(
            xml_text,
            item_range,
            "dcsset:userSettingID",
            &format!(
                "{child_indent}<dcsset:userSettingID>{}</dcsset:userSettingID>",
                escape_xml(user_setting_id)
            ),
            DCS_EDIT_FILTER_ITEM_COMPARISON_CHILD_SEQUENCE,
        )?;
    }
    stdout.push_str(&format!(
        "[OK] Filter \"{}\" modified in variant \"{}\"\n",
        filter.field, var_name
    ));
    Ok(true)
}

pub(crate) fn dcs_edit_parse_filter_expression(value: &str) -> (String, String, String) {
    let operators = [
        ("notBeginsWith", "NotBeginsWith"),
        ("beginsWith", "BeginsWith"),
        ("inListByHierarchy", "InListByHierarchy"),
        ("inHierarchy", "InHierarchy"),
        ("notContains", "NotContains"),
        ("contains", "Contains"),
        ("notFilled", "NotFilled"),
        ("filled", "Filled"),
        ("notIn", "NotInList"),
        ("in", "InList"),
        ("<>", "NotEqual"),
        (">=", "GreaterOrEqual"),
        ("<=", "LessOrEqual"),
        ("=", "Equal"),
        (">", "Greater"),
        ("<", "Less"),
    ];
    for (raw, mapped) in operators {
        let marker = format!(" {raw}");
        if let Some(pos) = value.find(&marker) {
            let field = value[..pos].trim().to_string();
            let right = value[pos + marker.len()..].trim().to_string();
            return (field, mapped.to_string(), right);
        }
    }
    (value.trim().to_string(), "Equal".to_string(), String::new())
}

pub(crate) fn dcs_edit_filter_value_type(value: &str) -> (String, String) {
    if value.is_empty() || value == "_" {
        return (String::new(), "xs:string".to_string());
    }
    if value == "true" || value == "false" {
        return (value.to_string(), "xs:boolean".to_string());
    }
    if is_date_time_literal(value) {
        return (value.to_string(), "xs:dateTime".to_string());
    }
    if dcs_edit_is_valid_xs_decimal(value) {
        return (value.to_string(), "xs:decimal".to_string());
    }
    if [
        "Перечисление.",
        "Справочник.",
        "ПланСчетов.",
        "Документ.",
        "ПланВидовХарактеристик.",
        "ПланВидовРасчета.",
    ]
    .iter()
    .any(|prefix| value.starts_with(prefix))
    {
        return (value.to_string(), "dcscor:DesignTimeValue".to_string());
    }
    (value.to_string(), "xs:string".to_string())
}

pub(crate) fn dcs_edit_validate_filter_literal(filter: &DcsEditFilter) -> Result<(), String> {
    let value = filter.value.trim();
    if value.is_empty() {
        return Ok(());
    }
    if dcs_edit_looks_numeric(value) && !dcs_edit_is_valid_xs_decimal(value) {
        return Err(format!(
            "Filter value '{value}' is not a valid xs:decimal literal for the fixed DCS 8.3.27 XSD contract"
        ));
    }
    if dcs_edit_looks_date_time(value) && !is_date_time_literal(value) {
        return Err(format!(
            "Filter value '{value}' is not a valid xs:dateTime literal for the fixed DCS 8.3.27 XSD contract"
        ));
    }
    Ok(())
}

pub(crate) fn dcs_edit_is_valid_xs_decimal(value: &str) -> bool {
    let value = value.strip_prefix(['+', '-']).unwrap_or(value);
    let Some((integer, fraction)) = value.split_once('.') else {
        return !value.is_empty() && value.chars().all(|character| character.is_ascii_digit());
    };
    !value[integer.len() + 1..].contains('.')
        && (!integer.is_empty() || !fraction.is_empty())
        && integer.chars().all(|character| character.is_ascii_digit())
        && fraction.chars().all(|character| character.is_ascii_digit())
}

pub(crate) fn dcs_edit_looks_numeric(value: &str) -> bool {
    value.chars().any(|character| character.is_ascii_digit())
        && value.chars().next().is_some_and(|character| {
            character.is_ascii_digit() || matches!(character, '+' | '-' | '.')
        })
        && value
            .chars()
            .all(|character| character.is_ascii_digit() || matches!(character, '+' | '-' | '.'))
}

pub(crate) fn dcs_edit_looks_date_time(value: &str) -> bool {
    value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && value.as_bytes().get(10) == Some(&b'T')
}

pub(crate) fn is_date_time_literal(value: &str) -> bool {
    let Some((date, time_and_zone)) = value.split_once('T') else {
        return false;
    };
    if time_and_zone.contains('T') {
        return false;
    }
    let mut date_parts = date.split('-');
    let (Some(year), Some(month), Some(day), None) = (
        date_parts.next(),
        date_parts.next(),
        date_parts.next(),
        date_parts.next(),
    ) else {
        return false;
    };
    if year.len() < 4
        || !year.chars().all(|character| character.is_ascii_digit())
        || year.chars().all(|character| character == '0')
    {
        return false;
    }
    let (Ok(year), Ok(month), Ok(day)) = (
        year.parse::<u32>(),
        month.parse::<u32>(),
        day.parse::<u32>(),
    ) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    if day == 0 || day > max_day {
        return false;
    }

    let (time, zone) = if let Some(time) = time_and_zone.strip_suffix('Z') {
        (time, Some("Z"))
    } else if let Some(index) = time_and_zone
        .char_indices()
        .skip(1)
        .find_map(|(index, character)| matches!(character, '+' | '-').then_some(index))
    {
        (&time_and_zone[..index], Some(&time_and_zone[index..]))
    } else {
        (time_and_zone, None)
    };
    let mut time_parts = time.split(':');
    let (Some(hour), Some(minute), Some(second), None) = (
        time_parts.next(),
        time_parts.next(),
        time_parts.next(),
        time_parts.next(),
    ) else {
        return false;
    };
    let (second, fraction) = second
        .split_once('.')
        .map_or((second, None), |(second, fraction)| {
            (second, Some(fraction))
        });
    if fraction.is_some_and(|fraction| {
        fraction.is_empty() || !fraction.chars().all(|character| character.is_ascii_digit())
    }) {
        return false;
    }
    let (Ok(hour), Ok(minute), Ok(second)) = (
        hour.parse::<u32>(),
        minute.parse::<u32>(),
        second.parse::<u32>(),
    ) else {
        return false;
    };
    if minute > 59 || second > 59 || hour > 24 {
        return false;
    }
    if hour == 24
        && (minute != 0
            || second != 0
            || fraction.is_some_and(|fraction| fraction.chars().any(|digit| digit != '0')))
    {
        return false;
    }
    if let Some(zone) = zone.filter(|zone| *zone != "Z") {
        let zone = &zone[1..];
        let Some((hours, minutes)) = zone.split_once(':') else {
            return false;
        };
        let (Ok(hours), Ok(minutes)) = (hours.parse::<u32>(), minutes.parse::<u32>()) else {
            return false;
        };
        if hours > 14 || minutes > 59 || (hours == 14 && minutes != 0) {
            return false;
        }
    }
    true
}

pub(crate) struct DcsEditDataParameter {
    pub(crate) parameter: String,
    pub(crate) value: Option<String>,
    pub(crate) use_flag: Option<bool>,
    pub(crate) user_setting_id: Option<String>,
    pub(crate) view_mode: Option<String>,
}

pub(crate) fn dcs_edit_parse_data_parameter(value: &str) -> DcsEditDataParameter {
    let use_flag = if value.contains("@off") {
        Some(false)
    } else if value.contains("@on") {
        Some(true)
    } else {
        None
    };
    let user_setting_id = if value.contains("@user") {
        Some(fresh_uuid())
    } else {
        None
    };
    let view_mode = if value.contains("@quickAccess") {
        Some("QuickAccess".to_string())
    } else if value.contains("@normal") {
        Some("Normal".to_string())
    } else {
        None
    };
    let cleaned = value
        .replace("@off", "")
        .replace("@on", "")
        .replace("@user", "")
        .replace("@quickAccess", "")
        .replace("@normal", "");
    let (parameter, val) = cleaned
        .split_once('=')
        .map(|(left, right)| (left.trim().to_string(), Some(right.trim().to_string())))
        .unwrap_or((cleaned.trim().to_string(), None));
    DcsEditDataParameter {
        parameter,
        value: val,
        use_flag,
        user_setting_id,
        view_mode,
    }
}

pub(crate) fn dcs_edit_data_parameter_fragment(
    param: &DcsEditDataParameter,
    indent: &str,
) -> Result<String, String> {
    let mut lines = vec![format!(
        "{indent}<dcscor:item xsi:type=\"dcsset:SettingsParameterValue\">"
    )];
    if let Some(false) = param.use_flag {
        lines.push(format!("{indent}\t<dcscor:use>false</dcscor:use>"));
    }
    lines.push(format!(
        "{indent}\t<dcscor:parameter>{}</dcscor:parameter>",
        escape_xml(&param.parameter)
    ));
    if let Some(value) = &param.value {
        lines.extend(dcs_edit_settings_value_lines(
            "dcscor:value",
            value,
            indent,
        )?);
    }
    if let Some(view_mode) = &param.view_mode {
        lines.push(format!(
            "{indent}\t<dcsset:viewMode>{}</dcsset:viewMode>",
            escape_xml(view_mode)
        ));
    }
    if let Some(user_setting_id) = &param.user_setting_id {
        lines.push(format!(
            "{indent}\t<dcsset:userSettingID>{}</dcsset:userSettingID>",
            escape_xml(user_setting_id)
        ));
    }
    lines.push(format!("{indent}</dcscor:item>"));
    Ok(lines.join("\n"))
}

pub(crate) fn dcs_edit_modify_data_parameter(
    xml_text: &mut String,
    variant: &str,
    param: &DcsEditDataParameter,
    stdout: &mut String,
) -> Result<bool, String> {
    let var_name = dcs_edit_variant_name(xml_text, variant).unwrap_or_else(|| variant.to_string());
    let Ok(range) = dcs_edit_prefixed_container_range(xml_text, variant, "dcsset:dataParameters")
    else {
        stdout.push_str(&format!(
            "[WARN] No dataParameters section in variant \"{var_name}\"\n"
        ));
        return Ok(false);
    };
    let Some(item_range) = dcs_edit_find_item_by_child(
        xml_text,
        (range.open_end, range.close_start),
        "dcscor:item",
        "dcscor:parameter",
        &param.parameter,
    ) else {
        stdout.push_str(&format!(
            "[WARN] DataParameter \"{}\" not found in variant \"{}\"\n",
            param.parameter, var_name
        ));
        return Ok(false);
    };
    let item_indent = dcs_edit_line_indent(xml_text, item_range.0);
    let child_indent = format!("{item_indent}\t");
    match param.use_flag {
        Some(false) => {
            dcs_edit_replace_or_insert_child_fragment(
                xml_text,
                item_range,
                "dcscor:use",
                &format!("{child_indent}<dcscor:use>false</dcscor:use>"),
                DCS_EDIT_SETTINGS_PARAMETER_VALUE_CHILD_SEQUENCE,
            )?;
        }
        Some(true) => {
            let _ = dcs_edit_remove_child_element(xml_text, item_range, "dcscor:use");
        }
        None => {}
    }
    let range = dcs_edit_prefixed_container_range(xml_text, variant, "dcsset:dataParameters")?;
    let item_range = dcs_edit_find_item_by_child(
        xml_text,
        (range.open_end, range.close_start),
        "dcscor:item",
        "dcscor:parameter",
        &param.parameter,
    )
    .ok_or_else(|| format!("DataParameter \"{}\" not found", param.parameter))?;
    if let Some(value) = &param.value {
        let value_fragment =
            dcs_edit_settings_value_lines("dcscor:value", value, &item_indent)?.join("\n");
        dcs_edit_replace_or_insert_child_fragment(
            xml_text,
            item_range,
            "dcscor:value",
            &value_fragment,
            DCS_EDIT_SETTINGS_PARAMETER_VALUE_CHILD_SEQUENCE,
        )?;
    }
    let range = dcs_edit_prefixed_container_range(xml_text, variant, "dcsset:dataParameters")?;
    let item_range = dcs_edit_find_item_by_child(
        xml_text,
        (range.open_end, range.close_start),
        "dcscor:item",
        "dcscor:parameter",
        &param.parameter,
    )
    .ok_or_else(|| format!("DataParameter \"{}\" not found", param.parameter))?;
    if let Some(view_mode) = &param.view_mode {
        dcs_edit_replace_or_insert_child_fragment(
            xml_text,
            item_range,
            "dcsset:viewMode",
            &format!(
                "{child_indent}<dcsset:viewMode>{}</dcsset:viewMode>",
                escape_xml(view_mode)
            ),
            DCS_EDIT_SETTINGS_PARAMETER_VALUE_CHILD_SEQUENCE,
        )?;
    }
    let range = dcs_edit_prefixed_container_range(xml_text, variant, "dcsset:dataParameters")?;
    let item_range = dcs_edit_find_item_by_child(
        xml_text,
        (range.open_end, range.close_start),
        "dcscor:item",
        "dcscor:parameter",
        &param.parameter,
    )
    .ok_or_else(|| format!("DataParameter \"{}\" not found", param.parameter))?;
    if let Some(user_setting_id) = &param.user_setting_id {
        dcs_edit_replace_or_insert_child_fragment(
            xml_text,
            item_range,
            "dcsset:userSettingID",
            &format!(
                "{child_indent}<dcsset:userSettingID>{}</dcsset:userSettingID>",
                escape_xml(user_setting_id)
            ),
            DCS_EDIT_SETTINGS_PARAMETER_VALUE_CHILD_SEQUENCE,
        )?;
    }
    stdout.push_str(&format!(
        "[OK] DataParameter \"{}\" modified in variant \"{}\"\n",
        param.parameter, var_name
    ));
    Ok(true)
}

pub(crate) fn dcs_edit_settings_value_lines(
    tag_name: &str,
    value: &str,
    indent: &str,
) -> Result<Vec<String>, String> {
    if dcs_edit_is_empty_value(value) {
        return Ok(vec![format!("{indent}\t<{tag_name} xsi:nil=\"true\"/>")]);
    }
    if dcs_edit_is_standard_period_variant(value) {
        let mut lines = vec![
            format!("{indent}\t<{tag_name} xsi:type=\"v8:StandardPeriod\">"),
            format!(
                "{indent}\t\t<v8:variant xsi:type=\"v8:StandardPeriodVariant\">{}</v8:variant>",
                escape_xml(value)
            ),
        ];
        if value == "Custom" {
            lines.push(format!(
                "{indent}\t\t<v8:startDate>0001-01-01T00:00:00</v8:startDate>"
            ));
            lines.push(format!(
                "{indent}\t\t<v8:endDate>0001-01-01T00:00:00</v8:endDate>"
            ));
        }
        lines.push(format!("{indent}\t</{tag_name}>"));
        return Ok(lines);
    }
    if dcs_edit_looks_date_time(value) && !is_date_time_literal(value) {
        return Err(format!(
            "Value '{value}' is not a valid xs:dateTime literal for the fixed DCS 8.3.27 XSD contract"
        ));
    }
    let value_type = if is_date_time_literal(value) {
        "xs:dateTime"
    } else if value == "true" || value == "false" {
        "xs:boolean"
    } else {
        "xs:string"
    };
    Ok(vec![format!(
        "{indent}\t<{tag_name} xsi:type=\"{value_type}\">{}</{tag_name}>",
        escape_xml(value)
    )])
}

pub(crate) fn dcs_edit_is_empty_value(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.is_empty() || trimmed == "_" || trimmed.eq_ignore_ascii_case("null")
}

pub(crate) fn dcs_edit_is_standard_period_variant(value: &str) -> bool {
    matches!(
        value,
        "Custom"
            | "Today"
            | "ThisWeek"
            | "ThisTenDays"
            | "ThisMonth"
            | "ThisQuarter"
            | "ThisHalfYear"
            | "ThisYear"
            | "FromBeginningOfThisWeek"
            | "FromBeginningOfThisTenDays"
            | "FromBeginningOfThisMonth"
            | "FromBeginningOfThisQuarter"
            | "FromBeginningOfThisHalfYear"
            | "FromBeginningOfThisYear"
            | "Yesterday"
            | "LastWeek"
            | "LastTenDays"
            | "LastMonth"
            | "LastQuarter"
            | "LastHalfYear"
            | "LastYear"
            | "LastWeekTillSameWeekDay"
            | "LastTenDaysTillSameDayNumber"
            | "LastMonthTillSameDate"
            | "LastQuarterTillSameDate"
            | "LastHalfYearTillSameDate"
            | "LastYearTillSameDate"
            | "Tomorrow"
            | "NextWeek"
            | "NextTenDays"
            | "NextMonth"
            | "NextQuarter"
            | "NextHalfYear"
            | "NextYear"
            | "NextWeekTillSameWeekDay"
            | "NextTenDaysTillSameDayNumber"
            | "NextMonthTillSameDate"
            | "NextQuarterTillSameDate"
            | "NextHalfYearTillSameDate"
            | "NextYearTillSameDate"
            | "TillEndOfThisWeek"
            | "TillEndOfThisTenDays"
            | "TillEndOfThisMonth"
            | "TillEndOfThisQuarter"
            | "TillEndOfThisHalfYear"
            | "TillEndOfThisYear"
            | "Last7Days"
            | "Next7Days"
            | "Month"
    )
}

pub(crate) fn dcs_edit_insert_or_create_settings_item(
    xml_text: &mut String,
    variant: &str,
    container: &str,
    fragment: &str,
) -> Result<(), String> {
    match dcs_edit_insert_prefixed_item(xml_text, variant, container, fragment) {
        Ok(()) => Ok(()),
        Err(error) if error == format!("No <{container}> section found in DCS") => {
            let settings = dcs_edit_settings_element_range(xml_text, variant)?;
            let insert_pos =
                dcs_edit_new_settings_container_insert_pos(xml_text, variant, container)?;
            let settings_indent = dcs_edit_line_indent(xml_text, settings.0);
            let child_indent = format!("{settings_indent}\t");
            xml_text.insert_str(
                insert_pos,
                &format!("{child_indent}<{container}>\n{fragment}\n{child_indent}</{container}>\n"),
            );
            Ok(())
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn dcs_edit_new_settings_container_insert_pos(
    xml_text: &str,
    variant: &str,
    container: &str,
) -> Result<usize, String> {
    let settings = dcs_edit_settings_element_range(xml_text, variant)?;
    dcs_edit_canonical_child_insert_pos(
        xml_text,
        settings,
        container,
        DCS_EDIT_SETTINGS_CHILD_SEQUENCE,
    )
}

pub(crate) fn dcs_edit_settings_container_child_indent(
    xml_text: &str,
    variant: &str,
    container: &str,
) -> Result<String, String> {
    if let Ok(range) = dcs_edit_prefixed_container_range(xml_text, variant, container) {
        return Ok(format!("{}\t", dcs_edit_line_indent(xml_text, range.start)));
    }
    let settings = dcs_edit_settings_element_range(xml_text, variant)?;
    Ok(format!(
        "{}\t\t",
        dcs_edit_line_indent(xml_text, settings.0)
    ))
}

pub(crate) struct DcsEditDataSetLink {
    pub(crate) source: String,
    pub(crate) dest: String,
    pub(crate) source_expr: String,
    pub(crate) dest_expr: String,
    pub(crate) parameter: String,
}

pub(crate) fn dcs_edit_parse_data_set_link(value: &str) -> Result<DcsEditDataSetLink, String> {
    let (source, rest) = value
        .split_once('>')
        .ok_or_else(|| "dataSetLink value must contain '>'".to_string())?;
    let (dest, rest) = rest
        .split_once(" on ")
        .ok_or_else(|| "dataSetLink value must contain ' on '".to_string())?;
    let (expr, parameter) = if let Some((expr, param)) = rest.split_once("[param ") {
        (expr.trim(), param.trim_end_matches(']').trim().to_string())
    } else {
        (rest.trim(), String::new())
    };
    let (source_expr, dest_expr) = expr
        .split_once('=')
        .ok_or_else(|| "dataSetLink expression must contain '='".to_string())?;
    Ok(DcsEditDataSetLink {
        source: source.trim().to_string(),
        dest: dest.trim().to_string(),
        source_expr: source_expr.trim().to_string(),
        dest_expr: dest_expr.trim().to_string(),
        parameter,
    })
}

pub(crate) fn dcs_edit_data_set_link_fragment(link: &DcsEditDataSetLink, indent: &str) -> String {
    let mut lines = vec![
        format!("{indent}<dataSetLink>"),
        format!(
            "{indent}\t<sourceDataSet>{}</sourceDataSet>",
            escape_xml(&link.source)
        ),
        format!(
            "{indent}\t<destinationDataSet>{}</destinationDataSet>",
            escape_xml(&link.dest)
        ),
        format!(
            "{indent}\t<sourceExpression>{}</sourceExpression>",
            escape_xml(&link.source_expr)
        ),
        format!(
            "{indent}\t<destinationExpression>{}</destinationExpression>",
            escape_xml(&link.dest_expr)
        ),
    ];
    if !link.parameter.is_empty() {
        lines.push(format!(
            "{indent}\t<parameter>{}</parameter>",
            escape_xml(&link.parameter)
        ));
    }
    lines.push(format!("{indent}</dataSetLink>"));
    lines.join("\n")
}

pub(crate) struct DcsEditDataSet {
    pub(crate) name: String,
    pub(crate) query: String,
}

pub(crate) fn dcs_edit_parse_data_set(
    value: &str,
    base_dir: &Path,
    cwd: &Path,
) -> Result<DcsEditDataSet, String> {
    dcs_edit_parse_data_set_with_inputs(value, base_dir, cwd, &mut Vec::new())
}

fn dcs_edit_parse_data_set_with_inputs(
    value: &str,
    base_dir: &Path,
    cwd: &Path,
    inputs: &mut Vec<ExactFileInput>,
) -> Result<DcsEditDataSet, String> {
    let (name, query) = if let Some((left, right)) = value.split_once(':') {
        (left.trim().to_string(), right.trim())
    } else {
        ("НаборДанных".to_string(), value.trim())
    };
    let query = dcs_compile_resolve_query_value_with_inputs(query, base_dir, cwd, inputs)?;
    Ok(DcsEditDataSet { name, query })
}

pub(crate) fn dcs_edit_first_data_source(xml_text: &str) -> Option<String> {
    let document = Document::parse(xml_text).ok()?;
    document
        .root_element()
        .children()
        .find(|node| role_info_element(*node, "dataSource", Some(DCS_SCHEMA_NS)))
        .and_then(|node| dcs_child(node, "name", DCS_SCHEMA_NS))
        .map(dcs_text_of)
}

pub(crate) fn dcs_edit_data_set_fragment(
    data_set: &DcsEditDataSet,
    source: &str,
    indent: &str,
) -> String {
    format!(
        "{indent}<dataSet xsi:type=\"DataSetQuery\">\n{indent}\t<name>{}</name>\n{indent}\t<dataSource>{}</dataSource>\n{indent}\t<query>{}</query>\n{indent}</dataSet>",
        escape_xml(&data_set.name),
        escape_xml(source),
        escape_xml(&data_set.query)
    )
}

pub(crate) struct DcsEditVariant {
    pub(crate) name: String,
    pub(crate) presentation: String,
}

pub(crate) fn dcs_edit_parse_variant(value: &str) -> DcsEditVariant {
    let (name, presentation) = dcs_edit_extract_bracket_title(value);
    let name = name.trim().to_string();
    let presentation = if presentation.is_empty() {
        name.clone()
    } else {
        presentation
    };
    DcsEditVariant { name, presentation }
}

pub(crate) fn dcs_edit_variant_exists(xml_text: &str, name: &str) -> bool {
    let Ok(document) = Document::parse(xml_text) else {
        return false;
    };
    document
        .root_element()
        .children()
        .filter(|node| role_info_element(*node, "settingsVariant", Some(DCS_SCHEMA_NS)))
        .any(|node| {
            node.children()
                .find(|child| role_info_element(*child, "name", Some(DCS_SETTINGS_NS)))
                .is_some_and(|candidate| dcs_text_of(candidate) == name)
        })
}

pub(crate) fn dcs_edit_variant_fragment(variant: &DcsEditVariant, indent: &str) -> String {
    let mut lines = vec![
        format!("{indent}<settingsVariant>"),
        format!(
            "{indent}\t<dcsset:name>{}</dcsset:name>",
            escape_xml(&variant.name)
        ),
    ];
    dcs_compile_emit_mltext(
        &mut lines,
        &format!("{indent}\t"),
        "dcsset:presentation",
        &variant.presentation,
    );
    lines.push(format!(
        "{indent}\t<dcsset:settings xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\">"
    ));
    lines.push(format!("{indent}\t\t<dcsset:selection>"));
    lines.push(format!(
        "{indent}\t\t\t<dcsset:item xsi:type=\"dcsset:SelectedItemAuto\"/>"
    ));
    lines.push(format!("{indent}\t\t</dcsset:selection>"));
    lines.push(format!(
        "{indent}\t\t<dcsset:item xsi:type=\"dcsset:StructureItemGroup\">"
    ));
    lines.push(format!("{indent}\t\t\t<dcsset:order>"));
    lines.push(format!(
        "{indent}\t\t\t\t<dcsset:item xsi:type=\"dcsset:OrderItemAuto\"/>"
    ));
    lines.push(format!("{indent}\t\t\t</dcsset:order>"));
    lines.push(format!("{indent}\t\t\t<dcsset:selection>"));
    lines.push(format!(
        "{indent}\t\t\t\t<dcsset:item xsi:type=\"dcsset:SelectedItemAuto\"/>"
    ));
    lines.push(format!("{indent}\t\t\t</dcsset:selection>"));
    lines.push(format!("{indent}\t\t</dcsset:item>"));
    lines.push(format!("{indent}\t</dcsset:settings>"));
    lines.push(format!("{indent}</settingsVariant>"));
    lines.join("\n")
}

pub(crate) struct DcsEditConditionalAppearance {
    pub(crate) parameter: String,
    pub(crate) value: String,
    pub(crate) fields: Vec<String>,
    pub(crate) filters: Vec<DcsEditFilter>,
}

pub(crate) fn dcs_edit_parse_conditional_appearance(value: &str) -> DcsEditConditionalAppearance {
    let (head, fields) = if let Some((left, right)) = value.split_once(" for ") {
        (
            left.trim(),
            right
                .split(',')
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect(),
        )
    } else {
        (value.trim(), Vec::new())
    };
    let (head, filters) = if let Some((left, right)) = head.split_once(" when ") {
        (
            left.trim(),
            right
                .split(" or ")
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(dcs_edit_parse_filter)
                .collect(),
        )
    } else {
        (head, Vec::new())
    };
    let (parameter, val) = head
        .split_once('=')
        .map(|(left, right)| (left.trim().to_string(), right.trim().to_string()))
        .unwrap_or((head.to_string(), String::new()));
    DcsEditConditionalAppearance {
        parameter,
        value: val,
        fields,
        filters,
    }
}

pub(crate) fn dcs_edit_conditional_appearance_fragment(
    item: &DcsEditConditionalAppearance,
    indent: &str,
) -> String {
    let mut lines = vec![format!("{indent}<dcsset:item>")];
    if item.fields.is_empty() {
        lines.push(format!("{indent}\t<dcsset:selection/>"));
    } else {
        lines.push(format!("{indent}\t<dcsset:selection>"));
        for field in &item.fields {
            lines.push(format!("{indent}\t\t<dcsset:item>"));
            lines.push(format!(
                "{indent}\t\t\t<dcsset:field>{}</dcsset:field>",
                escape_xml(field)
            ));
            lines.push(format!("{indent}\t\t</dcsset:item>"));
        }
        lines.push(format!("{indent}\t</dcsset:selection>"));
    }

    if item.filters.is_empty() {
        lines.push(format!("{indent}\t<dcsset:filter/>"));
    } else {
        lines.push(format!("{indent}\t<dcsset:filter>"));
        if item.filters.len() >= 2 {
            lines.push(format!(
                "{indent}\t\t<dcsset:item xsi:type=\"dcsset:FilterItemGroup\">"
            ));
            lines.push(format!(
                "{indent}\t\t\t<dcsset:groupType>OrGroup</dcsset:groupType>"
            ));
            for filter in &item.filters {
                dcs_edit_emit_filter_comparison(&mut lines, filter, &format!("{indent}\t\t\t"));
            }
            lines.push(format!("{indent}\t\t</dcsset:item>"));
        } else if let Some(filter) = item.filters.first() {
            dcs_edit_emit_filter_comparison(&mut lines, filter, &format!("{indent}\t\t"));
        }
        lines.push(format!("{indent}\t</dcsset:filter>"));
    }

    lines.push(format!("{indent}\t<dcsset:appearance>"));
    lines.push(format!(
        "{indent}\t\t<dcscor:item xsi:type=\"dcsset:SettingsParameterValue\">"
    ));
    lines.push(format!(
        "{indent}\t\t\t<dcscor:parameter>{}</dcscor:parameter>",
        escape_xml(&item.parameter)
    ));
    lines.extend(dcs_edit_conditional_appearance_value_lines(
        &item.parameter,
        &item.value,
        &format!("{indent}\t\t\t"),
    ));
    lines.push(format!("{indent}\t\t</dcscor:item>"));
    lines.push(format!("{indent}\t</dcsset:appearance>"));
    lines.push(format!("{indent}</dcsset:item>"));
    lines.join("\n")
}

pub(crate) fn dcs_edit_emit_filter_comparison(
    lines: &mut Vec<String>,
    filter: &DcsEditFilter,
    indent: &str,
) {
    lines.push(format!(
        "{indent}<dcsset:item xsi:type=\"dcsset:FilterItemComparison\">"
    ));
    lines.push(format!(
        "{indent}\t<dcsset:left xsi:type=\"dcscor:Field\">{}</dcsset:left>",
        escape_xml(&filter.field)
    ));
    lines.push(format!(
        "{indent}\t<dcsset:comparisonType>{}</dcsset:comparisonType>",
        escape_xml(&filter.operator)
    ));
    if !filter.value.is_empty() {
        lines.push(format!(
            "{indent}\t<dcsset:right xsi:type=\"{}\">{}</dcsset:right>",
            filter.value_type,
            escape_xml(&filter.value)
        ));
    }
    lines.push(format!("{indent}</dcsset:item>"));
}

pub(crate) fn dcs_edit_conditional_appearance_value_lines(
    parameter: &str,
    value: &str,
    indent: &str,
) -> Vec<String> {
    if value.starts_with("web:") || value.starts_with("style:") || value.starts_with("win:") {
        return vec![format!(
            "{indent}<dcscor:value xsi:type=\"v8ui:Color\">{}</dcscor:value>",
            escape_xml(value)
        )];
    }
    if value == "true" || value == "false" {
        return vec![format!(
            "{indent}<dcscor:value xsi:type=\"xs:boolean\">{}</dcscor:value>",
            escape_xml(value)
        )];
    }
    if matches!(parameter, "Формат" | "Текст" | "Заголовок") {
        let mut lines = vec![format!(
            "{indent}<dcscor:value xsi:type=\"v8:LocalStringType\">"
        )];
        lines.push(format!("{indent}\t<v8:item>"));
        lines.push(format!("{indent}\t\t<v8:lang>ru</v8:lang>"));
        lines.push(format!(
            "{indent}\t\t<v8:content>{}</v8:content>",
            escape_xml(value)
        ));
        lines.push(format!("{indent}\t</v8:item>"));
        lines.push(format!("{indent}</dcscor:value>"));
        return lines;
    }
    vec![format!(
        "{indent}<dcscor:value xsi:type=\"xs:string\">{}</dcscor:value>",
        escape_xml(value)
    )]
}

pub(crate) fn dcs_edit_conditional_appearance_description(
    item: &DcsEditConditionalAppearance,
) -> String {
    let mut desc = format!("{} = {}", item.parameter, item.value);
    if let Some(filter) = item.filters.first() {
        if item.filters.len() >= 2 {
            desc.push_str(&format!(" when OrGroup({} conditions)", item.filters.len()));
        } else {
            desc.push_str(&format!(" when {} {}", filter.field, filter.operator));
        }
    }
    if !item.fields.is_empty() {
        desc.push_str(&format!(" for {}", item.fields.join(", ")));
    }
    desc
}

pub(crate) struct DcsEditOutputParameter {
    pub(crate) key: String,
    pub(crate) value: String,
}

pub(crate) fn dcs_edit_parse_output_parameter(
    value: &str,
) -> Result<DcsEditOutputParameter, String> {
    let (key, val) = value
        .split_once('=')
        .ok_or_else(|| "outputParameter value must contain '='".to_string())?;
    Ok(DcsEditOutputParameter {
        key: key.trim().to_string(),
        value: val.trim().to_string(),
    })
}

pub(crate) fn dcs_edit_output_parameter_fragment(
    item: &DcsEditOutputParameter,
    indent: &str,
) -> Result<String, String> {
    let mut lines = vec![
        format!("{indent}<dcscor:item xsi:type=\"dcsset:SettingsParameterValue\">"),
        format!(
            "{indent}\t<dcscor:parameter>{}</dcscor:parameter>",
            escape_xml(&item.key)
        ),
    ];
    if item.key == "Заголовок" {
        lines.push(format!(
            "{indent}\t<dcscor:value xsi:type=\"v8:LocalStringType\">"
        ));
        lines.push(format!("{indent}\t\t<v8:item>"));
        lines.push(format!("{indent}\t\t\t<v8:lang>ru</v8:lang>"));
        lines.push(format!(
            "{indent}\t\t\t<v8:content>{}</v8:content>",
            escape_xml(&item.value)
        ));
        lines.push(format!("{indent}\t\t</v8:item>"));
        lines.push(format!("{indent}\t</dcscor:value>"));
    } else {
        lines.extend(dcs_edit_settings_value_lines(
            "dcscor:value",
            &item.value,
            indent,
        )?);
    }
    lines.push(format!("{indent}</dcscor:item>"));
    Ok(lines.join("\n"))
}

#[derive(Clone, Debug)]
pub(crate) struct DcsEditStructureItem {
    pub(crate) name: Option<String>,
    pub(crate) group_by: Vec<String>,
    pub(crate) children: Vec<DcsEditStructureItem>,
}

pub(crate) fn dcs_edit_parse_structure(value: &str) -> Vec<DcsEditStructureItem> {
    let segments = value
        .split('>')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let mut innermost = None;
    for segment in segments.into_iter().rev() {
        let (segment, name) = dcs_edit_extract_structure_name(segment);
        let group_by = if segment.eq_ignore_ascii_case("details") || segment == "детали" {
            Vec::new()
        } else {
            segment
                .split(',')
                .map(str::trim)
                .filter(|field| !field.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        };
        let children = innermost.into_iter().collect::<Vec<_>>();
        innermost = Some(DcsEditStructureItem {
            name,
            group_by,
            children,
        });
    }
    innermost.into_iter().collect()
}

pub(crate) fn dcs_edit_extract_structure_name(segment: &str) -> (String, Option<String>) {
    let Some(marker) = segment.find("@name=") else {
        return (segment.trim().to_string(), None);
    };
    let before = segment[..marker].trim_end();
    let after = &segment[marker + "@name=".len()..];
    let (name, consumed) = if let Some(rest) = after.strip_prefix('"') {
        match rest.find('"') {
            Some(end) => (rest[..end].to_string(), end + 2),
            None => (rest.to_string(), after.len()),
        }
    } else if let Some(rest) = after.strip_prefix('\'') {
        match rest.find('\'') {
            Some(end) => (rest[..end].to_string(), end + 2),
            None => (rest.to_string(), after.len()),
        }
    } else {
        let end = after.find(char::is_whitespace).unwrap_or(after.len());
        (after[..end].to_string(), end)
    };
    let rest = after[consumed..].trim_start();
    let mut cleaned = before.to_string();
    if !rest.is_empty() {
        if !cleaned.is_empty() {
            cleaned.push(' ');
        }
        cleaned.push_str(rest);
    }
    (cleaned.trim().to_string(), Some(name.trim().to_string()))
}

pub(crate) fn dcs_edit_structure_fragments(
    structures: &[DcsEditStructureItem],
    indent: &str,
) -> String {
    structures
        .iter()
        .map(|item| dcs_edit_structure_item_fragment(item, indent))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn dcs_edit_structure_item_fragment(
    item: &DcsEditStructureItem,
    indent: &str,
) -> String {
    let mut lines = vec![format!(
        "{indent}<dcsset:item xsi:type=\"dcsset:StructureItemGroup\">"
    )];
    if let Some(name) = &item.name {
        lines.push(format!(
            "{indent}\t<dcsset:name>{}</dcsset:name>",
            escape_xml(name)
        ));
    }
    if !item.group_by.is_empty() {
        lines.push(format!("{indent}\t<dcsset:groupItems>"));
        for field in &item.group_by {
            lines.push(dcs_edit_group_item_field_fragment(
                field,
                &format!("{indent}\t\t"),
            ));
        }
        lines.push(format!("{indent}\t</dcsset:groupItems>"));
    }
    lines.push(format!("{indent}\t<dcsset:order>"));
    lines.push(format!(
        "{indent}\t\t<dcsset:item xsi:type=\"dcsset:OrderItemAuto\"/>"
    ));
    lines.push(format!("{indent}\t</dcsset:order>"));
    lines.push(format!("{indent}\t<dcsset:selection>"));
    lines.push(format!(
        "{indent}\t\t<dcsset:item xsi:type=\"dcsset:SelectedItemAuto\"/>"
    ));
    lines.push(format!("{indent}\t</dcsset:selection>"));
    for child in &item.children {
        lines.push(dcs_edit_structure_item_fragment(
            child,
            &format!("{indent}\t"),
        ));
    }
    lines.push(format!("{indent}</dcsset:item>"));
    lines.join("\n")
}

pub(crate) fn dcs_edit_group_item_field_fragment(field: &str, indent: &str) -> String {
    [
        format!("{indent}<dcsset:item xsi:type=\"dcsset:GroupItemField\">"),
        format!(
            "{indent}\t<dcsset:field>{}</dcsset:field>",
            escape_xml(field)
        ),
        format!("{indent}\t<dcsset:groupType>Items</dcsset:groupType>"),
        format!("{indent}\t<dcsset:periodAdditionType>None</dcsset:periodAdditionType>"),
        format!(
            "{indent}\t<dcsset:periodAdditionBegin xsi:type=\"xs:dateTime\">0001-01-01T00:00:00</dcsset:periodAdditionBegin>"
        ),
        format!(
            "{indent}\t<dcsset:periodAdditionEnd xsi:type=\"xs:dateTime\">0001-01-01T00:00:00</dcsset:periodAdditionEnd>"
        ),
        format!("{indent}</dcsset:item>"),
    ]
    .join("\n")
}

pub(crate) fn dcs_edit_replace_structure(
    xml_text: &mut String,
    variant: &str,
    fragment: &str,
) -> Result<(), String> {
    let settings_range = dcs_edit_settings_element_range(xml_text, variant)?;
    let document =
        Document::parse(xml_text).map_err(|error| format!("XML parse error: {error}"))?;
    let settings = dcs_edit_element_for_range(&document, settings_range)
        .ok_or_else(|| "DCS settings element not found".to_string())?;
    let mut removals = settings
        .children()
        .filter(|node| role_info_element(*node, "item", Some(DCS_SETTINGS_NS)))
        .filter(|node| dcs_edit_xsi_type_matches(*node, DCS_SETTINGS_NS, "StructureItemGroup"))
        .map(|node| {
            let range = node.range();
            dcs_edit_element_line_range(xml_text, range.start, range.end)
        })
        .collect::<Vec<_>>();
    removals.sort_by_key(|range| std::cmp::Reverse(range.start));
    for removal in removals {
        xml_text.replace_range(removal, "");
    }
    let settings = dcs_edit_settings_element_range(xml_text, variant)?;
    let insert_pos = dcs_edit_canonical_child_insert_pos(
        xml_text,
        settings,
        "item",
        DCS_EDIT_SETTINGS_CHILD_SEQUENCE,
    )?;
    xml_text.insert_str(insert_pos, &format!("{fragment}\n"));
    Ok(())
}

pub(crate) fn dcs_edit_modify_structure(
    xml_text: &mut String,
    variant: &str,
    structures: &[DcsEditStructureItem],
    stdout: &mut String,
) -> Result<(), String> {
    let mut targets = Vec::new();
    for structure in structures {
        dcs_edit_collect_structure_targets(structure, &mut targets);
    }
    if targets.is_empty() {
        return Err(format!(
            "modify-structure requires @name= for at least one group: {}",
            dcs_edit_structure_description(structures)
        ));
    }
    for (name, group_by) in targets {
        if dcs_edit_replace_named_group_items(xml_text, variant, &name, &group_by)? {
            let desc = if group_by.is_empty() {
                "details".to_string()
            } else {
                group_by.join(", ")
            };
            stdout.push_str(&format!(
                "[OK] Group \"{}\" groupItems updated: {}\n",
                name, desc
            ));
        } else {
            stdout.push_str(&format!(
                "[WARN] Group with @name=\"{}\" not found -- skipped\n",
                name
            ));
        }
    }
    Ok(())
}

pub(crate) fn dcs_edit_collect_structure_targets(
    item: &DcsEditStructureItem,
    targets: &mut Vec<(String, Vec<String>)>,
) {
    if let Some(name) = &item.name {
        targets.push((name.clone(), item.group_by.clone()));
    }
    for child in &item.children {
        dcs_edit_collect_structure_targets(child, targets);
    }
}

pub(crate) fn dcs_edit_structure_description(structures: &[DcsEditStructureItem]) -> String {
    structures
        .iter()
        .flat_map(|item| item.group_by.iter().cloned())
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn dcs_edit_replace_named_group_items(
    xml_text: &mut String,
    variant: &str,
    name: &str,
    group_by: &[String],
) -> Result<bool, String> {
    let Some(group_range) = dcs_edit_find_named_structure_group(xml_text, variant, name)? else {
        return Ok(false);
    };
    if group_by.is_empty() {
        if let Some(group_items) = dcs_edit_find_group_items_range(xml_text, group_range)? {
            let removal = dcs_edit_element_line_range(xml_text, group_items.start, group_items.end);
            xml_text.replace_range(removal, "");
        }
        return Ok(true);
    }
    let Some(group_items) = dcs_edit_find_group_items_range(xml_text, group_range)? else {
        let insert_pos = dcs_edit_canonical_child_insert_pos(
            xml_text,
            group_range,
            "groupItems",
            DCS_EDIT_STRUCTURE_GROUP_CHILD_SEQUENCE,
        )?;
        let child_indent = format!("{}\t", dcs_edit_line_indent(xml_text, group_range.0));
        let fragment = dcs_edit_group_items_fragment(group_by, &child_indent);
        xml_text.insert_str(insert_pos, &format!("{fragment}\n"));
        return Ok(true);
    };
    if group_items.self_closing {
        let group_indent = dcs_edit_line_indent(xml_text, group_items.start);
        let child_indent = format!("{group_indent}\t");
        let fragment = dcs_edit_group_items_inner_fragment(group_by, &child_indent);
        xml_text.replace_range(
            group_items.start..group_items.end,
            &format!("<dcsset:groupItems>\n{fragment}{group_indent}</dcsset:groupItems>"),
        );
    } else {
        let group_indent = dcs_edit_line_indent(xml_text, group_items.start);
        let child_indent = format!("{group_indent}\t");
        let fragment = dcs_edit_group_items_inner_fragment(group_by, &child_indent);
        xml_text.replace_range(
            group_items.open_end..group_items.close_start,
            &format!("\n{fragment}{group_indent}"),
        );
    }
    Ok(true)
}

pub(crate) fn dcs_edit_group_items_fragment(group_by: &[String], indent: &str) -> String {
    if group_by.is_empty() {
        return String::new();
    }
    format!(
        "{indent}<dcsset:groupItems>\n{}{indent}</dcsset:groupItems>",
        dcs_edit_group_items_inner_fragment(group_by, &(indent.to_string() + "\t")),
    )
}

pub(crate) fn dcs_edit_group_items_inner_fragment(group_by: &[String], indent: &str) -> String {
    group_by
        .iter()
        .map(|field| dcs_edit_group_item_field_fragment(field, indent))
        .map(|fragment| format!("{fragment}\n"))
        .collect::<String>()
}

pub(crate) fn dcs_edit_line_indent(xml_text: &str, pos: usize) -> String {
    let line_start = xml_text[..pos].rfind('\n').map(|idx| idx + 1).unwrap_or(0);
    xml_text[line_start..pos]
        .chars()
        .take_while(|ch| ch.is_whitespace() && *ch != '\n' && *ch != '\r')
        .collect()
}

pub(crate) fn dcs_edit_find_named_structure_group(
    xml_text: &str,
    variant: &str,
    name: &str,
) -> Result<Option<(usize, usize)>, String> {
    let settings_range = dcs_edit_settings_element_range(xml_text, variant)?;
    let document =
        Document::parse(xml_text).map_err(|error| format!("XML parse error: {error}"))?;
    let settings = dcs_edit_element_for_range(&document, settings_range)
        .ok_or_else(|| "DCS settings element not found".to_string())?;
    Ok(settings
        .descendants()
        .skip(1)
        .filter(|node| role_info_element(*node, "item", Some(DCS_SETTINGS_NS)))
        .filter(|node| dcs_edit_xsi_type_matches(*node, DCS_SETTINGS_NS, "StructureItemGroup"))
        .find(|node| {
            dcs_child(*node, "name", DCS_SETTINGS_NS)
                .is_some_and(|candidate| dcs_text_of(candidate) == name)
        })
        .map(|node| {
            let range = node.range();
            (range.start, range.end)
        }))
}

pub(crate) fn dcs_edit_find_group_items_range(
    xml_text: &str,
    group_range: (usize, usize),
) -> Result<Option<DcsEditElementRange>, String> {
    let Some(range) = dcs_edit_child_element_range(xml_text, group_range, "dcsset:groupItems")
    else {
        return Ok(None);
    };
    dcs_edit_element_range_details(xml_text, range, "dcsset:groupItems").map(Some)
}

pub(crate) fn dcs_edit_remove_first_block(
    xml_text: &mut String,
    range: (usize, usize),
    open_prefix: &str,
    close: &str,
) -> bool {
    let Some(open_rel) = xml_text[range.0..range.1].find(open_prefix) else {
        return false;
    };
    let start = range.0 + open_rel;
    let Some(close_rel) = xml_text[start..range.1].find(close) else {
        return false;
    };
    let end = start + close_rel + close.len();
    xml_text.replace_range(start..end, "");
    true
}

pub(crate) fn dcs_edit_matching_dcsset_item_end(
    xml_text: &str,
    start: usize,
    limit: usize,
) -> Option<usize> {
    let open = "<dcsset:item";
    let close = "</dcsset:item>";
    let first_open_end = xml_text[start..limit].find('>')? + start;
    let mut depth = 1usize;
    let mut cursor = first_open_end + 1;
    while cursor < limit {
        let next_open = xml_text[cursor..limit].find(open).map(|rel| cursor + rel);
        let next_close = xml_text[cursor..limit].find(close).map(|rel| cursor + rel);
        match (next_open, next_close) {
            (Some(open_pos), Some(close_pos)) if open_pos < close_pos => {
                let open_end = xml_text[open_pos..limit].find('>')? + open_pos;
                let tag = &xml_text[open_pos..=open_end];
                if !tag.trim_end().ends_with("/>") {
                    depth += 1;
                }
                cursor = open_end + 1;
            }
            (_, Some(close_pos)) => {
                depth = depth.saturating_sub(1);
                let end = close_pos + close.len();
                if depth == 0 {
                    return Some(end);
                }
                cursor = end;
            }
            _ => return None,
        }
    }
    None
}

pub(crate) enum DcsEditDrilldownResult {
    Added,
    NoNamedTemplates,
    NoMatch,
}

pub(crate) fn dcs_edit_add_drilldown(
    xml_text: &mut String,
    resource: &str,
) -> DcsEditDrilldownResult {
    if !xml_text.contains("<template>") {
        return DcsEditDrilldownResult::NoNamedTemplates;
    }
    if !xml_text.contains(resource) {
        return DcsEditDrilldownResult::NoMatch;
    }
    let marker = format!("DrillDown{}", sanitize_xml_identifier(resource));
    if xml_text.contains(&marker) {
        return DcsEditDrilldownResult::NoMatch;
    }
    let fragment = format!(
        "\t<parameter>\n\t\t<name>{}</name>\n\t\t<expression>{}</expression>\n\t</parameter>",
        escape_xml(&marker),
        escape_xml(resource)
    );
    if dcs_edit_insert_top_level_fragment(xml_text, "parameter", &fragment).is_ok() {
        DcsEditDrilldownResult::Added
    } else {
        DcsEditDrilldownResult::NoMatch
    }
}

pub(crate) fn dcs_edit_set_field_role(
    xml_text: &mut String,
    data_set: &str,
    value: &str,
    stdout: &mut String,
) -> Result<(), String> {
    let mut data_path_parts = Vec::new();
    let mut flags = Vec::new();
    let mut kv = Vec::new();
    for part in value.split_whitespace() {
        if let Some(flag) = part.strip_prefix('@') {
            if !flag.is_empty() {
                flags.push(flag.to_string());
            }
        } else if let Some((key, val)) = part.split_once('=') {
            kv.push((key.to_string(), val.to_string()));
        } else {
            data_path_parts.push(part.to_string());
        }
    }
    let data_path = data_path_parts.join(" ");
    if data_path.is_empty() {
        stdout.push_str("[WARN] set-field-role: empty dataPath\n");
        return Ok(());
    }
    let range = dcs_edit_dataset_range(xml_text, data_set)?;
    let field_range = dcs_edit_find_item_by_child(xml_text, range, "field", "dataPath", &data_path);
    let Some(field_range) = field_range else {
        stdout.push_str(&format!("[WARN] Field \"{}\" not found\n", data_path));
        return Ok(());
    };
    let field_child_indent = format!("{}\t", dcs_edit_line_indent(xml_text, field_range.0));
    let role_fragment = dcs_edit_field_role_fragment_with_values(&flags, &kv, &field_child_indent)?;
    let _ = dcs_edit_remove_child_block(xml_text, field_range, "role");
    if !role_fragment.is_empty() {
        let range = dcs_edit_dataset_range(xml_text, data_set)?;
        let field_range =
            dcs_edit_find_item_by_child(xml_text, range, "field", "dataPath", &data_path)
                .ok_or_else(|| format!("Field \"{}\" not found", data_path))?;
        let insert = dcs_edit_canonical_child_insert_pos(
            xml_text,
            field_range,
            "role",
            DCS_EDIT_DATA_SET_FIELD_CHILD_SEQUENCE,
        )?;
        xml_text.insert_str(insert, &format!("{role_fragment}\n"));
        let mut parts = Vec::new();
        if !flags.is_empty() {
            parts.push(
                flags
                    .iter()
                    .map(|flag| format!("@{flag}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        if !kv.is_empty() {
            parts.push(
                kv.iter()
                    .map(|(key, val)| format!("{key}={val}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        stdout.push_str(&format!(
            "[OK] Field \"{}\" role set: {}\n",
            data_path,
            parts.join(" ")
        ));
    } else {
        stdout.push_str(&format!("[OK] Field \"{}\" role cleared\n", data_path));
    }
    Ok(())
}

pub(crate) struct DcsEditParameterPatch {
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) values: Option<Vec<String>>,
    pub(crate) simple_pairs: Vec<(String, String)>,
    pub(crate) available_values: Vec<(String, String)>,
    pub(crate) hidden: bool,
    pub(crate) always: bool,
}

pub(crate) fn dcs_edit_parse_parameter_patch(value: &str) -> DcsEditParameterPatch {
    let hidden = value.contains("@hidden");
    let always = value.contains("@always");
    let cleaned = value.replace("@hidden", "").replace("@always", "");
    let (without_available, available_values) =
        if let Some((head, tail)) = cleaned.split_once("availableValue=") {
            (
                head.trim().to_string(),
                dcs_edit_extract_available_values(&format!("availableValue={tail}")),
            )
        } else {
            (cleaned.trim().to_string(), Vec::new())
        };
    let (head, title) = dcs_edit_extract_bracket_title(&without_available);
    let mut parts = dcs_edit_split_quoted_whitespace(&head)
        .into_iter()
        .peekable();
    let name = parts.next().unwrap_or_default();
    let mut values = None;
    let mut simple_pairs = Vec::new();
    while let Some(token) = parts.next() {
        let Some((key, val)) = token.split_once('=') else {
            continue;
        };
        if key == "value" {
            let mut raw_value = val.to_string();
            while let Some(next) = parts.peek() {
                let looks_like_pair = next.split_once('=').is_some_and(|(next_key, _)| {
                    next_key.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
                });
                if looks_like_pair {
                    break;
                }
                raw_value.push(' ');
                raw_value.push_str(next);
                parts.next();
            }
            values = Some(dcs_edit_parse_value_list(&raw_value));
        } else {
            simple_pairs.push((key.to_string(), dcs_edit_strip_quotes(val)));
        }
    }
    DcsEditParameterPatch {
        name,
        title,
        values,
        simple_pairs,
        available_values,
        hidden,
        always,
    }
}

pub(crate) fn dcs_edit_modify_parameter(
    xml_text: &mut String,
    patch: &DcsEditParameterPatch,
    stdout: &mut String,
) -> Result<(), String> {
    let Some(initial_range) = dcs_edit_parameter_range(xml_text, &patch.name) else {
        stdout.push_str(&format!(
            "[WARN] Parameter \"{}\" not found -- skipped\n",
            patch.name
        ));
        return Ok(());
    };
    let declared_type = dcs_edit_parameter_declared_type(xml_text, initial_range);
    let child_indent = format!("{}\t", dcs_edit_line_indent(xml_text, initial_range.0));

    for (key, value) in &patch.simple_pairs {
        dcs_edit_validate_parameter_simple_pair(key, value)?;
    }
    let mut value_lines = Vec::new();
    if let Some(values) = &patch.values {
        for value in values {
            value_lines.extend(dcs_edit_parameter_value_lines(
                &declared_type,
                value,
                &child_indent,
                "value",
            )?);
        }
    }
    let mut available_value_lines = Vec::new();
    for (value, presentation) in &patch.available_values {
        available_value_lines.push(format!("{child_indent}<availableValue>"));
        available_value_lines.extend(dcs_edit_parameter_value_lines(
            &declared_type,
            value,
            &format!("{child_indent}\t"),
            "value",
        )?);
        if !presentation.is_empty() {
            dcs_compile_emit_mltext(
                &mut available_value_lines,
                &format!("{child_indent}\t"),
                "presentation",
                presentation,
            );
        }
        available_value_lines.push(format!("{child_indent}</availableValue>"));
    }

    if !patch.title.is_empty() {
        let range = dcs_edit_parameter_range(xml_text, &patch.name)
            .ok_or_else(|| format!("Parameter \"{}\" not found", patch.name))?;
        let mut lines = Vec::new();
        dcs_compile_emit_mltext(&mut lines, &child_indent, "title", &patch.title);
        dcs_edit_replace_or_insert_child_fragment(
            xml_text,
            range,
            "title",
            &lines.join("\n"),
            DCS_EDIT_PARAMETER_CHILD_SEQUENCE,
        )?;
        stdout.push_str(&format!(
            "[OK] Parameter \"{}\": title set to \"{}\"\n",
            patch.name, patch.title
        ));
    }
    if let Some(values) = &patch.values {
        let range = dcs_edit_parameter_range(xml_text, &patch.name)
            .ok_or_else(|| format!("Parameter \"{}\" not found", patch.name))?;
        let existed = dcs_edit_remove_parameter_value_children(xml_text, range);
        if !value_lines.is_empty() {
            let range = dcs_edit_parameter_range(xml_text, &patch.name)
                .ok_or_else(|| format!("Parameter \"{}\" not found", patch.name))?;
            dcs_edit_replace_or_insert_child_fragment(
                xml_text,
                range,
                "value",
                &value_lines.join("\n"),
                DCS_EDIT_PARAMETER_CHILD_SEQUENCE,
            )?;
        }
        if values.len() >= 2 {
            let range = dcs_edit_parameter_range(xml_text, &patch.name)
                .ok_or_else(|| format!("Parameter \"{}\" not found", patch.name))?;
            dcs_edit_replace_or_insert_child_fragment(
                xml_text,
                range,
                "valueListAllowed",
                &format!("{child_indent}<valueListAllowed>true</valueListAllowed>"),
                DCS_EDIT_PARAMETER_CHILD_SEQUENCE,
            )?;
            stdout.push_str(&format!(
                "[OK] Parameter \"{}\": value set to list of {} item(s)\n",
                patch.name,
                values.len()
            ));
        } else {
            let value = values.first().cloned().unwrap_or_default();
            stdout.push_str(&format!(
                "[OK] Parameter \"{}\": value {} to {}\n",
                patch.name,
                if existed { "updated" } else { "added" },
                value
            ));
        }
    }
    for (key, value) in &patch.simple_pairs {
        let range = dcs_edit_parameter_range(xml_text, &patch.name)
            .ok_or_else(|| format!("Parameter \"{}\" not found", patch.name))?;
        let existed = dcs_edit_child_element_range(xml_text, range, key).is_some();
        dcs_edit_replace_or_insert_child_fragment(
            xml_text,
            range,
            key,
            &format!("{child_indent}<{key}>{}</{key}>", escape_xml(value)),
            DCS_EDIT_PARAMETER_CHILD_SEQUENCE,
        )?;
        stdout.push_str(&format!(
            "[OK] Parameter \"{}\": {}\n",
            patch.name,
            if existed {
                format!("{key} updated to {value}")
            } else {
                format!("{key}={value} added")
            }
        ));
    }
    if !patch.available_values.is_empty() {
        let range = dcs_edit_parameter_range(xml_text, &patch.name)
            .ok_or_else(|| format!("Parameter \"{}\" not found", patch.name))?;
        dcs_edit_remove_parameter_available_value_children(xml_text, range);
        let range = dcs_edit_parameter_range(xml_text, &patch.name)
            .ok_or_else(|| format!("Parameter \"{}\" not found", patch.name))?;
        dcs_edit_replace_or_insert_child_fragment(
            xml_text,
            range,
            "availableValue",
            &available_value_lines.join("\n"),
            DCS_EDIT_PARAMETER_CHILD_SEQUENCE,
        )?;
        stdout.push_str(&format!(
            "[OK] Parameter \"{}\": availableValue set to {} item(s)\n",
            patch.name,
            patch.available_values.len()
        ));
    }
    if patch.hidden {
        let range = dcs_edit_parameter_range(xml_text, &patch.name)
            .ok_or_else(|| format!("Parameter \"{}\" not found", patch.name))?;
        dcs_edit_replace_or_insert_child_fragment(
            xml_text,
            range,
            "availableAsField",
            &format!("{child_indent}<availableAsField>false</availableAsField>"),
            DCS_EDIT_PARAMETER_CHILD_SEQUENCE,
        )?;
        let range = dcs_edit_parameter_range(xml_text, &patch.name)
            .ok_or_else(|| format!("Parameter \"{}\" not found", patch.name))?;
        dcs_edit_replace_or_insert_child_fragment(
            xml_text,
            range,
            "useRestriction",
            &format!("{child_indent}<useRestriction>true</useRestriction>"),
            DCS_EDIT_PARAMETER_CHILD_SEQUENCE,
        )?;
        stdout.push_str(&format!(
            "[OK] Parameter \"{}\": @hidden applied\n",
            patch.name
        ));
    }
    if patch.always {
        let range = dcs_edit_parameter_range(xml_text, &patch.name)
            .ok_or_else(|| format!("Parameter \"{}\" not found", patch.name))?;
        dcs_edit_replace_or_insert_child_fragment(
            xml_text,
            range,
            "use",
            &format!("{child_indent}<use>Always</use>"),
            DCS_EDIT_PARAMETER_CHILD_SEQUENCE,
        )?;
        stdout.push_str(&format!(
            "[OK] Parameter \"{}\": @always applied\n",
            patch.name
        ));
    }
    Ok(())
}

pub(crate) fn dcs_edit_validate_parameter_simple_pair(
    key: &str,
    value: &str,
) -> Result<(), String> {
    match key {
        "useRestriction" | "valueListAllowed" | "availableAsField" | "denyIncompleteValues" => {
            if !matches!(value, "true" | "false" | "0" | "1") {
                return Err(format!(
                    "Parameter property '{key}' value '{value}' is not a valid xs:boolean literal in the fixed DCS 8.3.27 XSD contract"
                ));
            }
        }
        "use" => {
            if !matches!(value, "Always" | "Auto") {
                return Err(format!(
                    "Parameter property 'use' value '{value}' is not allowed by the fixed DCS 8.3.27 XSD contract"
                ));
            }
        }
        "expression" | "functionalOptionsParameter" => {}
        _ => {
            return Err(format!(
                "Parameter property '{key}' is not allowed by the fixed DCS 8.3.27 Parameter XSD contract"
            ));
        }
    }
    Ok(())
}

pub(crate) fn dcs_edit_replace_or_insert_parameter_child_fragment(
    xml_text: &mut String,
    range: (usize, usize),
    child: &str,
    fragment: &str,
    before_children: &[&str],
) {
    if let Some(child_range) = dcs_edit_child_element_range(xml_text, range, child) {
        let replace = dcs_edit_element_line_range(xml_text, child_range.0, child_range.1);
        xml_text.replace_range(replace, &format!("{fragment}\n"));
        return;
    }
    let block = &xml_text[range.0..range.1];
    let insert = before_children
        .iter()
        .filter_map(|name| block.find(&format!("\n\t\t<{name}")))
        .min()
        .map(|rel| range.0 + rel + 1)
        .unwrap_or_else(|| {
            range.0
                + block
                    .rfind("\n\t</parameter>")
                    .map(|rel| rel + 1)
                    .unwrap_or(range.1 - range.0)
        });
    xml_text.insert_str(insert, &format!("{fragment}\n"));
}

pub(crate) fn dcs_edit_insert_parameter_child_fragment(
    xml_text: &mut String,
    range: (usize, usize),
    fragment: &str,
    before_children: &[&str],
) {
    let block = &xml_text[range.0..range.1];
    let insert = before_children
        .iter()
        .filter_map(|name| block.find(&format!("\n\t\t<{name}")))
        .min()
        .map(|rel| range.0 + rel + 1)
        .unwrap_or_else(|| {
            range.0
                + block
                    .rfind("\n\t</parameter>")
                    .map(|rel| rel + 1)
                    .unwrap_or(range.1 - range.0)
        });
    xml_text.insert_str(insert, &format!("{fragment}\n"));
}

pub(crate) fn dcs_edit_parameter_declared_type(xml_text: &str, range: (usize, usize)) -> String {
    let Some(value_type_range) = dcs_edit_child_element_range(xml_text, range, "valueType") else {
        return String::new();
    };
    let Ok(type_range) = dcs_edit_child_text_range(xml_text, value_type_range, "v8:Type") else {
        return String::new();
    };
    xml_text[type_range].trim().to_string()
}

pub(crate) fn dcs_edit_remove_parameter_value_children(
    xml_text: &mut String,
    range: (usize, usize),
) -> bool {
    dcs_edit_remove_parameter_children_by_name(xml_text, range, "value")
}

pub(crate) fn dcs_edit_remove_parameter_available_value_children(
    xml_text: &mut String,
    range: (usize, usize),
) -> bool {
    dcs_edit_remove_parameter_children_by_name(xml_text, range, "availableValue")
}

pub(crate) fn dcs_edit_remove_parameter_children_by_name(
    xml_text: &mut String,
    range: (usize, usize),
    child: &str,
) -> bool {
    let Ok(document) = Document::parse(xml_text) else {
        return false;
    };
    let Some(parameter) = dcs_edit_element_for_range(&document, range) else {
        return false;
    };
    let namespace = parameter.tag_name().namespace();
    let mut removals = parameter
        .children()
        .filter(|node| {
            node.is_element() && dcs_edit_requested_name_matches(*node, child, namespace)
        })
        .map(|node| {
            let node_range = node.range();
            dcs_edit_element_line_range(xml_text, node_range.start, node_range.end)
        })
        .collect::<Vec<_>>();
    let removed = !removals.is_empty();
    removals.sort_by_key(|range| std::cmp::Reverse(range.start));
    for remove in removals {
        xml_text.replace_range(remove, "");
    }
    removed
}

pub(crate) fn dcs_edit_replace_or_insert_child_fragment(
    xml_text: &mut String,
    range: (usize, usize),
    child: &str,
    fragment: &str,
    sequence: &[&str],
) -> Result<(), String> {
    if let Some(child_range) = dcs_edit_child_element_range(xml_text, range, child) {
        let replace = dcs_edit_element_line_range(xml_text, child_range.0, child_range.1);
        xml_text.replace_range(replace, &format!("{fragment}\n"));
        return Ok(());
    }
    let insert = dcs_edit_canonical_child_insert_pos(xml_text, range, child, sequence)?;
    xml_text.insert_str(insert, &format!("{fragment}\n"));
    Ok(())
}

pub(crate) fn dcs_edit_element_name_at(xml_text: &str, start: usize) -> Option<String> {
    let text = xml_text.get(start..)?;
    let text = text.strip_prefix('<')?;
    let end = text
        .find(|ch: char| ch == '>' || ch == '/' || ch.is_whitespace())
        .unwrap_or(text.len());
    Some(text[..end].to_string())
}

pub(crate) fn dcs_edit_child_element_range(
    xml_text: &str,
    range: (usize, usize),
    child: &str,
) -> Option<(usize, usize)> {
    let document = Document::parse(xml_text).ok()?;
    let parent = dcs_edit_element_for_range(&document, range)?;
    let default_namespace = parent.tag_name().namespace();
    parent
        .children()
        .find(|node| {
            node.is_element() && dcs_edit_requested_name_matches(*node, child, default_namespace)
        })
        .map(|node| {
            let range = node.range();
            (range.start, range.end)
        })
}

pub(crate) fn dcs_edit_element_for_range<'a, 'input>(
    document: &'a Document<'input>,
    range: (usize, usize),
) -> Option<roxmltree::Node<'a, 'input>> {
    document
        .descendants()
        .filter(roxmltree::Node::is_element)
        .filter(|node| {
            let node_range = node.range();
            node_range.start <= range.0 && node_range.end >= range.1
        })
        .min_by_key(|node| {
            let node_range = node.range();
            node_range.end.saturating_sub(node_range.start)
        })
}

pub(crate) fn dcs_edit_requested_name_matches(
    node: roxmltree::Node<'_, '_>,
    requested: &str,
    default_namespace: Option<&str>,
) -> bool {
    let (prefix, local_name) = requested
        .split_once(':')
        .map_or((None, requested), |(prefix, local_name)| {
            (Some(prefix), local_name)
        });
    if node.tag_name().name() != local_name {
        return false;
    }
    let expected_namespace = match prefix {
        Some("dcsset") => Some(DCS_SETTINGS_NS),
        Some("dcscor") => Some(DCS_CORE_NS),
        Some("dcscom") => Some(DCS_COMMON_NS),
        Some("v8") => Some(V8_DATA_NS),
        Some("xs") => Some("http://www.w3.org/2001/XMLSchema"),
        Some("xsi") => Some("http://www.w3.org/2001/XMLSchema-instance"),
        Some(_) => None,
        None => default_namespace,
    };
    expected_namespace.is_none_or(|namespace| node.tag_name().namespace() == Some(namespace))
}

pub(crate) fn dcs_edit_xsi_type_matches(
    node: roxmltree::Node<'_, '_>,
    expected_namespace: &str,
    expected_local_name: &str,
) -> bool {
    let Some(value) = node.attribute((XML_SCHEMA_INSTANCE_NS, "type")) else {
        return false;
    };
    let Some((prefix, local_name)) = value.split_once(':') else {
        return value == expected_local_name
            && node.lookup_namespace_uri(None) == Some(expected_namespace);
    };
    local_name == expected_local_name
        && !prefix.contains(':')
        && node.lookup_namespace_uri(Some(prefix)) == Some(expected_namespace)
}

pub(crate) fn dcs_edit_parameter_range(xml_text: &str, name: &str) -> Option<(usize, usize)> {
    let document = Document::parse(xml_text).ok()?;
    let root = document.root_element();
    root.children()
        .filter(|node| role_info_element(*node, "parameter", Some(DCS_SCHEMA_NS)))
        .find(|node| {
            dcs_child(*node, "name", DCS_SCHEMA_NS)
                .is_some_and(|name_node| dcs_text_of(name_node) == name)
        })
        .map(|node| {
            let range = node.range();
            (range.start, range.end)
        })
}

pub(crate) fn dcs_edit_rename_parameter(
    xml_text: &mut String,
    value: &str,
    stdout: &mut String,
) -> Result<(), String> {
    let Some((old, new)) = value.split_once("=>") else {
        stdout.push_str(&format!(
            "[WARN] rename-parameter expects \"OldName => NewName\", got: {value}\n"
        ));
        return Ok(());
    };
    let old = old.trim();
    let new = new.trim();
    if old == new {
        stdout.push_str("[WARN] rename-parameter: old and new names are equal -- skipped\n");
        return Ok(());
    }
    if dcs_edit_parameter_range(xml_text, old).is_none() {
        stdout.push_str(&format!(
            "[WARN] Parameter \"{}\" not found -- skipped\n",
            old
        ));
        return Ok(());
    }
    let range = dcs_edit_parameter_range(xml_text, old)
        .ok_or_else(|| format!("Parameter \"{}\" not found", old))?;
    dcs_edit_replace_child_text(xml_text, range, "name", new)?;
    let expr_updated = dcs_edit_update_parameter_expression_refs(xml_text, old, new);
    let dp_updated = dcs_edit_replace_exact_data_parameter_refs(xml_text, old, new);
    stdout.push_str(&format!(
        "[OK] Parameter renamed: \"{}\" => \"{}\" (expressions updated: {}, dataParameters updated: {})\n",
        old, new, expr_updated, dp_updated
    ));
    Ok(())
}

pub(crate) fn dcs_edit_parameter_limit(xml_text: &str) -> usize {
    xml_text
        .find("\n\t<settingsVariant")
        .or_else(|| xml_text.find("</DataCompositionSchema>"))
        .unwrap_or(xml_text.len())
}

pub(crate) fn dcs_edit_update_parameter_expression_refs(
    xml_text: &mut String,
    old: &str,
    new: &str,
) -> usize {
    let Ok(document) = Document::parse(xml_text) else {
        return 0;
    };
    let root = document.root_element();
    let mut replacements = root
        .children()
        .filter(|node| role_info_element(*node, "parameter", Some(DCS_SCHEMA_NS)))
        .filter_map(|parameter| dcs_child(parameter, "expression", DCS_SCHEMA_NS))
        .filter_map(|expression| {
            let range = expression.range();
            let content = dcs_edit_element_content_range(xml_text, (range.start, range.end))?;
            let current = &xml_text[content.clone()];
            let (replacement, count) = dcs_edit_replace_parameter_tokens(current, old, new);
            (count > 0).then_some((content, replacement, count))
        })
        .collect::<Vec<_>>();
    let updated = replacements.iter().map(|(_, _, count)| *count).sum();
    replacements.sort_by_key(|(range, _, _)| std::cmp::Reverse(range.start));
    for (range, replacement, _) in replacements {
        xml_text.replace_range(range, &replacement);
    }
    updated
}

pub(crate) fn dcs_edit_replace_parameter_tokens(
    text: &str,
    old: &str,
    new: &str,
) -> (String, usize) {
    let needle = format!("&amp;{}", escape_xml(old));
    let replacement = format!("&amp;{}", escape_xml(new));
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0usize;
    let mut count = 0usize;
    while let Some(rel) = text[cursor..].find(&needle) {
        let start = cursor + rel;
        let end = start + needle.len();
        result.push_str(&text[cursor..start]);
        let boundary = text[end..]
            .chars()
            .next()
            .is_none_or(|ch| !(ch.is_alphanumeric() || ch == '_'));
        if boundary {
            result.push_str(&replacement);
            count += 1;
        } else {
            result.push_str(&text[start..end]);
        }
        cursor = end;
    }
    result.push_str(&text[cursor..]);
    (result, count)
}

pub(crate) fn dcs_edit_replace_exact_data_parameter_refs(
    xml_text: &mut String,
    old: &str,
    new: &str,
) -> usize {
    let old_tag = format!("<dcscor:parameter>{}</dcscor:parameter>", escape_xml(old));
    let new_tag = format!("<dcscor:parameter>{}</dcscor:parameter>", escape_xml(new));
    let count = xml_text.matches(&old_tag).count();
    if count > 0 {
        *xml_text = xml_text.replace(&old_tag, &new_tag);
    }
    count
}

pub(crate) fn dcs_edit_reorder_parameters(
    xml_text: &mut String,
    value: &str,
    stdout: &mut String,
) -> Result<(), String> {
    let order = value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if order.is_empty() {
        stdout.push_str("[WARN] reorder-parameters: empty list -- skipped\n");
        return Ok(());
    }
    let parameter_limit = dcs_edit_parameter_limit(xml_text);
    let mut blocks = dcs_edit_collect_root_parameter_blocks(xml_text, parameter_limit);
    if blocks.is_empty() {
        stdout.push_str("[WARN] reorder-parameters: no parameters in schema\n");
        return Ok(());
    }
    let mut selected = Vec::new();
    let mut remaining = Vec::new();
    for (name, _, _, block) in blocks.drain(..) {
        if order.iter().any(|item| item == &name) {
            selected.push((name, block));
        } else {
            remaining.push((name, block));
        }
    }
    selected.sort_by_key(|(name, _)| {
        order
            .iter()
            .position(|item| item == name)
            .unwrap_or(usize::MAX)
    });
    let all = selected
        .into_iter()
        .chain(remaining)
        .map(|(_, block)| block)
        .collect::<Vec<_>>();
    let current_blocks = dcs_edit_collect_root_parameter_blocks(xml_text, parameter_limit);
    let first_start = current_blocks
        .first()
        .map(|(_, start, _, _)| *start)
        .ok_or_else(|| "No parameter block found".to_string())?;
    let last_end = current_blocks
        .last()
        .map(|(_, _, end, _)| *end)
        .ok_or_else(|| "No parameter block found".to_string())?;
    let indent = dcs_edit_line_indent(xml_text, first_start);
    xml_text.replace_range(first_start..last_end, &all.join(&format!("\n{indent}")));
    stdout.push_str(&format!(
        "[OK] Parameters reordered ({} total, {} explicit)\n",
        all.len(),
        order.len()
    ));
    Ok(())
}

pub(crate) fn dcs_edit_collect_root_parameter_blocks(
    xml_text: &str,
    _limit: usize,
) -> Vec<(String, usize, usize, String)> {
    let Ok(document) = Document::parse(xml_text) else {
        return Vec::new();
    };
    document
        .root_element()
        .children()
        .filter(|node| role_info_element(*node, "parameter", Some(DCS_SCHEMA_NS)))
        .map(|node| {
            let range = node.range();
            let name = dcs_child(node, "name", DCS_SCHEMA_NS)
                .map(dcs_text_of)
                .unwrap_or_default();
            (
                name,
                range.start,
                range.end,
                xml_text[range.start..range.end].to_string(),
            )
        })
        .collect()
}

pub(crate) fn dcs_edit_collect_blocks(xml_text: &str, item: &str) -> Vec<(String, String)> {
    dcs_edit_collect_blocks_in_range(xml_text, item, (0, xml_text.len()))
}

pub(crate) fn dcs_edit_collect_blocks_in_range(
    xml_text: &str,
    item: &str,
    range: (usize, usize),
) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let open_prefix = format!("<{item}");
    let close = format!("</{item}>");
    let mut cursor = range.0;
    while cursor < range.1 {
        let Some(open_rel) = xml_text[cursor..range.1].find(&open_prefix) else {
            break;
        };
        let start = cursor + open_rel;
        let Some(close_rel) = xml_text[start..range.1].find(&close) else {
            break;
        };
        let end = start + close_rel + close.len();
        let block = xml_text[start..end].to_string();
        let name = dcs_edit_child_text_range(xml_text, (start, end), "name")
            .map(|range| xml_text[range].trim().to_string())
            .unwrap_or_default();
        result.push((name, block));
        cursor = end;
    }
    result
}

pub(crate) fn dcs_edit_find_item_by_child(
    xml_text: &str,
    range: (usize, usize),
    item: &str,
    child: &str,
    value: &str,
) -> Option<(usize, usize)> {
    let document = Document::parse(xml_text).ok()?;
    let parent = dcs_edit_element_for_range(&document, range)?;
    let parent_namespace = parent.tag_name().namespace();
    parent
        .children()
        .filter(|node| {
            node.is_element() && dcs_edit_requested_name_matches(*node, item, parent_namespace)
        })
        .find(|node| {
            let item_namespace = node.tag_name().namespace();
            node.children()
                .filter(roxmltree::Node::is_element)
                .any(|candidate| {
                    dcs_edit_requested_name_matches(candidate, child, item_namespace)
                        && dcs_text_of(candidate) == value
                })
        })
        .map(|node| {
            let range = node.range();
            (range.start, range.end)
        })
}

pub(crate) fn dcs_edit_remove_child_block(
    xml_text: &mut String,
    range: (usize, usize),
    child: &str,
) -> bool {
    dcs_edit_remove_child_element(xml_text, range, child)
}

pub(crate) fn dcs_edit_remove_child_element(
    xml_text: &mut String,
    range: (usize, usize),
    child: &str,
) -> bool {
    let Some((start, end)) = dcs_edit_child_element_range(xml_text, range, child) else {
        return false;
    };
    let remove = dcs_edit_element_line_range(xml_text, start, end);
    xml_text.replace_range(remove, "");
    true
}

pub(crate) fn dcs_edit_replace_or_insert_simple_child(
    xml_text: &mut String,
    range: (usize, usize),
    child: &str,
    value: &str,
) {
    if let Ok(text_range) = dcs_edit_child_text_range(xml_text, range, child) {
        xml_text.replace_range(text_range, &escape_xml(value));
        return;
    }
    let close = "</parameter>";
    if let Some(close_rel) = xml_text[range.0..range.1].find(close) {
        let pos = range.0 + close_rel;
        xml_text.insert_str(
            pos,
            &format!("\t\t<{child}>{}</{child}>\n\t", escape_xml(value)),
        );
    }
}

pub(crate) fn dcs_edit_extract_bracket_title(value: &str) -> (String, String) {
    let mut text = value.to_string();
    if let (Some(open), Some(close)) = (text.find('['), text.find(']')) {
        if close > open {
            let title = text[open + 1..close].trim().to_string();
            text.replace_range(open..=close, "");
            return (text.trim().to_string(), title);
        }
    }
    (text.trim().to_string(), String::new())
}

pub(crate) fn dcs_edit_strip_markers(value: &str) -> String {
    value
        .split_whitespace()
        .filter(|part| !part.starts_with('@') && !part.starts_with('#'))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn dcs_edit_extract_available_values(value: &str) -> Vec<(String, String)> {
    let Some((_, tail)) = value.split_once("availableValue=") else {
        return Vec::new();
    };
    dcs_edit_split_quoted_csv(tail)
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty() && !item.starts_with('@'))
        .map(|item| {
            dcs_edit_split_once_unquoted_colon(&item)
                .map(|(left, right)| (dcs_edit_strip_quotes(left), dcs_edit_strip_quotes(right)))
                .unwrap_or((dcs_edit_strip_quotes(&item), String::new()))
        })
        .collect()
}

pub(crate) fn dcs_edit_parse_value_list(value: &str) -> Vec<String> {
    if value.is_empty() {
        return vec![String::new()];
    }
    dcs_edit_split_quoted_csv(value)
        .into_iter()
        .map(|item| dcs_edit_strip_quotes(&item))
        .filter(|item| !item.is_empty())
        .collect()
}

pub(crate) fn dcs_edit_split_quoted_csv(value: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for ch in value.chars() {
        if let Some(active) = quote {
            current.push(ch);
            if ch == active {
                quote = None;
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
            current.push(ch);
        } else if ch == ',' {
            result.push(current);
            current = String::new();
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

pub(crate) fn dcs_edit_split_quoted_whitespace(value: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for ch in value.chars() {
        if let Some(active) = quote {
            current.push(ch);
            if ch == active {
                quote = None;
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
            current.push(ch);
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                result.push(current);
                current = String::new();
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

pub(crate) fn dcs_edit_strip_quotes(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
            || (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
        {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

pub(crate) fn dcs_edit_split_once_unquoted_colon(value: &str) -> Option<(&str, &str)> {
    let mut quote = None;
    for (idx, ch) in value.char_indices() {
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else if ch == ':' {
            return Some((&value[..idx], &value[idx + ch.len_utf8()..]));
        }
    }
    None
}

pub(crate) fn sanitize_xml_identifier(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_alphanumeric() || *ch == '_')
        .collect()
}

pub(crate) struct DcsEditField {
    pub(crate) data_path: String,
    pub(crate) field: String,
    pub(crate) title: String,
    pub(crate) field_type: String,
    pub(crate) roles: Vec<String>,
    pub(crate) restrict: Vec<String>,
    pub(crate) type_declared: bool,
}

pub(crate) fn dcs_edit_parse_field(value: &str) -> DcsEditField {
    let mut text = value.to_string();
    let title = if let (Some(open), Some(close)) = (text.find('['), text.find(']')) {
        if close > open {
            let title = text[open + 1..close].trim().to_string();
            text.replace_range(open..=close, "");
            title
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    let roles = text
        .split_whitespace()
        .filter_map(|part| part.strip_prefix('@').map(ToOwned::to_owned))
        .collect::<Vec<_>>();
    let restrict = text
        .split_whitespace()
        .filter_map(|part| part.strip_prefix('#').map(ToOwned::to_owned))
        .collect::<Vec<_>>();
    let text = text
        .split_whitespace()
        .filter(|part| !part.starts_with('@') && !part.starts_with('#'))
        .collect::<Vec<_>>()
        .join(" ");
    let (data_path, field_type, type_declared) = if let Some((left, right)) = text.split_once(':') {
        (
            left.trim().to_string(),
            dcs_compile_resolve_type(right.trim()),
            true,
        )
    } else {
        (text.trim().to_string(), String::new(), false)
    };
    DcsEditField {
        field: data_path.clone(),
        data_path,
        title,
        field_type,
        roles,
        restrict,
        type_declared,
    }
}

pub(crate) fn dcs_edit_emit_field(
    lines: &mut Vec<String>,
    field: &DcsEditField,
    indent: &str,
    emit_value_type: bool,
) -> Result<(), String> {
    let value_type_entries = field
        .type_declared
        .then(|| dcs_compile_parse_value_type(&field.field_type))
        .transpose()?;
    lines.push(format!("{indent}<field xsi:type=\"DataSetFieldField\">"));
    lines.push(format!(
        "{indent}\t<dataPath>{}</dataPath>",
        escape_xml(&field.data_path)
    ));
    lines.push(format!(
        "{indent}\t<field>{}</field>",
        escape_xml(&field.field)
    ));
    if !field.title.is_empty() {
        dcs_compile_emit_mltext(lines, &format!("{indent}\t"), "title", &field.title);
    }
    let restriction = dcs_edit_field_restriction_fragment(&field.restrict, &format!("{indent}\t"));
    if !restriction.is_empty() {
        lines.push(restriction);
    }
    let role = dcs_edit_field_role_fragment(&field.roles, &format!("{indent}\t"))?;
    if !role.is_empty() {
        lines.push(role);
    }
    if emit_value_type && value_type_entries.is_some() {
        lines.push(format!("{indent}\t<valueType>"));
        dcs_compile_emit_value_type_entries(
            lines,
            value_type_entries.as_deref().unwrap_or_default(),
            &format!("{indent}\t\t"),
        );
        lines.push(format!("{indent}\t</valueType>"));
    }
    lines.push(format!("{indent}</field>"));
    Ok(())
}

pub(crate) fn dcs_edit_field_role_fragment(
    roles: &[String],
    indent: &str,
) -> Result<String, String> {
    dcs_edit_field_role_fragment_with_values(roles, &[], indent)
}

pub(crate) fn dcs_edit_field_role_fragment_with_values(
    flags: &[String],
    values: &[(String, String)],
    indent: &str,
) -> Result<String, String> {
    let entries = dcs_edit_field_role_entries(flags, values)?;
    if entries.is_empty() {
        return Ok(String::new());
    }
    let mut lines = vec![format!("{indent}<role>")];
    for key in DCS_EDIT_FIELD_ROLE_CHILD_SEQUENCE {
        if let Some(value) = entries.get(*key) {
            lines.push(format!(
                "{indent}\t<dcscom:{key}>{}</dcscom:{key}>",
                escape_xml(value)
            ));
        }
    }
    lines.push(format!("{indent}</role>"));
    Ok(lines.join("\n"))
}

pub(crate) fn dcs_edit_field_role_entries(
    flags: &[String],
    values: &[(String, String)],
) -> Result<BTreeMap<String, String>, String> {
    let mut entries = BTreeMap::new();
    for flag in flags {
        match flag.as_str() {
            "period" => {
                entries.insert("periodNumber".to_string(), "1".to_string());
                entries.insert("periodType".to_string(), "Main".to_string());
            }
            "dimension" | "account" | "balance" | "ignoreNullValues" | "required"
            | "dimensionAttribute" => {
                entries.insert(flag.clone(), "true".to_string());
            }
            _ => {
                return Err(format!(
                    "Role flag '@{flag}' is not allowed by the fixed DCS 8.3.27 DataSetFieldRole XSD contract"
                ));
            }
        }
    }
    for (key, value) in values {
        if !DCS_EDIT_FIELD_ROLE_CHILD_SEQUENCE.contains(&key.as_str()) {
            return Err(format!(
                "Role key '{key}' is not allowed by the fixed DCS 8.3.27 DataSetFieldRole XSD contract"
            ));
        }
        dcs_edit_validate_field_role_value(key, value)?;
        entries.insert(key.clone(), value.clone());
    }
    Ok(entries)
}

pub(crate) fn dcs_edit_validate_field_role_value(key: &str, value: &str) -> Result<(), String> {
    let valid = match key {
        "periodNumber" => {
            let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
            !digits.is_empty() && digits.chars().all(|character| character.is_ascii_digit())
        }
        "periodType" => matches!(value, "Main" | "Specify" | "Additional"),
        "dimension" | "account" | "balance" | "ignoreNullValues" | "required"
        | "dimensionAttribute" => matches!(value, "true" | "false" | "0" | "1"),
        "balanceType" => matches!(value, "None" | "OpeningBalance" | "ClosingBalance"),
        "accountingBalanceType" => matches!(value, "None" | "Debit" | "Credit"),
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "Role value '{value}' is invalid for '{key}' in the fixed DCS 8.3.27 XSD contract"
        ))
    }
}

pub(crate) fn dcs_edit_field_restriction_fragment(restrict: &[String], indent: &str) -> String {
    let mut enabled = [false; 4];
    for item in restrict {
        match item.as_str() {
            "noField" => enabled[0] = true,
            "noFilter" | "noCondition" => enabled[1] = true,
            "noGroup" => enabled[2] = true,
            "noOrder" => enabled[3] = true,
            _ => {}
        }
    }
    if !enabled.iter().any(|value| *value) {
        return String::new();
    }
    let mut lines = vec![format!("{indent}<useRestriction>")];
    for (enabled, tag) in enabled
        .into_iter()
        .zip(["field", "condition", "group", "order"])
    {
        if enabled {
            lines.push(format!("{indent}\t<{tag}>true</{tag}>"));
        }
    }
    lines.push(format!("{indent}</useRestriction>"));
    lines.join("\n")
}

pub(crate) fn dcs_edit_replace_dataset_field(
    xml_text: &mut String,
    data_set: &str,
    field: &DcsEditField,
) -> Result<bool, String> {
    let value_type_entries = field
        .type_declared
        .then(|| dcs_compile_parse_value_type(&field.field_type))
        .transpose()?;
    let _ = dcs_edit_field_role_entries(&field.roles, &[])?;
    let target = dcs_edit_dataset_target(xml_text, data_set)?;
    let emit_value_type = target.emit_field_value_type;
    let range = target.range;
    let Some(_) =
        dcs_edit_find_item_by_child(xml_text, range, "field", "dataPath", &field.data_path)
    else {
        return Ok(false);
    };
    if !field.title.is_empty() {
        let range = dcs_edit_dataset_range(xml_text, data_set)?;
        let field_range =
            dcs_edit_find_item_by_child(xml_text, range, "field", "dataPath", &field.data_path)
                .ok_or_else(|| format!("Field \"{}\" not found", field.data_path))?;
        let field_child_indent = format!("{}\t", dcs_edit_line_indent(xml_text, field_range.0));
        let mut lines = Vec::new();
        dcs_compile_emit_mltext(&mut lines, &field_child_indent, "title", &field.title);
        dcs_edit_replace_or_insert_child_fragment(
            xml_text,
            field_range,
            "title",
            &lines.join("\n"),
            DCS_EDIT_DATA_SET_FIELD_CHILD_SEQUENCE,
        )?;
    }
    if !field.restrict.is_empty() {
        let range = dcs_edit_dataset_range(xml_text, data_set)?;
        let field_range =
            dcs_edit_find_item_by_child(xml_text, range, "field", "dataPath", &field.data_path)
                .ok_or_else(|| format!("Field \"{}\" not found", field.data_path))?;
        let field_child_indent = format!("{}\t", dcs_edit_line_indent(xml_text, field_range.0));
        let fragment = dcs_edit_field_restriction_fragment(&field.restrict, &field_child_indent);
        if !fragment.is_empty() {
            dcs_edit_replace_or_insert_child_fragment(
                xml_text,
                field_range,
                "useRestriction",
                &fragment,
                DCS_EDIT_DATA_SET_FIELD_CHILD_SEQUENCE,
            )?;
        }
    }
    if !field.roles.is_empty() {
        let range = dcs_edit_dataset_range(xml_text, data_set)?;
        let field_range =
            dcs_edit_find_item_by_child(xml_text, range, "field", "dataPath", &field.data_path)
                .ok_or_else(|| format!("Field \"{}\" not found", field.data_path))?;
        let field_child_indent = format!("{}\t", dcs_edit_line_indent(xml_text, field_range.0));
        let fragment = dcs_edit_field_role_fragment(&field.roles, &field_child_indent)?;
        dcs_edit_replace_or_insert_child_fragment(
            xml_text,
            field_range,
            "role",
            &fragment,
            DCS_EDIT_DATA_SET_FIELD_CHILD_SEQUENCE,
        )?;
    }
    if !emit_value_type || !field.field_type.is_empty() {
        let range = dcs_edit_dataset_range(xml_text, data_set)?;
        let field_range =
            dcs_edit_find_item_by_child(xml_text, range, "field", "dataPath", &field.data_path)
                .ok_or_else(|| format!("Field \"{}\" not found", field.data_path))?;
        if emit_value_type {
            let field_child_indent = format!("{}\t", dcs_edit_line_indent(xml_text, field_range.0));
            let mut lines = vec![format!("{field_child_indent}<valueType>")];
            dcs_compile_emit_value_type_entries(
                &mut lines,
                value_type_entries.as_deref().unwrap_or_default(),
                &format!("{field_child_indent}\t"),
            );
            lines.push(format!("{field_child_indent}</valueType>"));
            dcs_edit_replace_or_insert_child_fragment(
                xml_text,
                field_range,
                "valueType",
                &lines.join("\n"),
                DCS_EDIT_DATA_SET_FIELD_CHILD_SEQUENCE,
            )?;
        } else {
            dcs_edit_remove_child_element(xml_text, field_range, "valueType");
        }
    }
    Ok(true)
}

pub(crate) struct DcsEditDataSetTarget {
    pub(crate) range: (usize, usize),
    pub(crate) name: String,
    pub(crate) emit_field_value_type: bool,
    pub(crate) field_insert_pos: usize,
    pub(crate) child_indent: String,
}

pub(crate) fn dcs_edit_dataset_target(
    xml_text: &str,
    data_set: &str,
) -> Result<DcsEditDataSetTarget, String> {
    const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

    let document =
        Document::parse(xml_text).map_err(|error| format!("XML parse error: {error}"))?;
    let node = document
        .descendants()
        .filter(|node| {
            role_info_element(*node, "dataSet", Some(DCS_SCHEMA_NS))
                || role_info_element(*node, "item", Some(DCS_SCHEMA_NS))
        })
        .find(|node| {
            data_set.is_empty()
                || dcs_child(*node, "name", DCS_SCHEMA_NS)
                    .is_some_and(|name| dcs_text_of(name) == data_set)
        })
        .ok_or_else(|| {
            if data_set.is_empty() {
                "No dataSet found in DCS".to_string()
            } else {
                format!("DataSet '{data_set}' not found")
            }
        })?;
    let name = dcs_child(node, "name", DCS_SCHEMA_NS)
        .map(dcs_text_of)
        .unwrap_or_else(|| data_set.to_string());
    let data_set_type = node
        .attribute((XSI_NS, "type"))
        .unwrap_or("")
        .rsplit(':')
        .next()
        .unwrap_or("");
    let range = node.range();
    let direct_elements = node
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    let child_indent = direct_elements
        .first()
        .map(|child| dcs_edit_line_indent(xml_text, child.range().start))
        .unwrap_or_else(|| format!("{}\t", dcs_edit_line_indent(xml_text, range.start)));
    let insert_at = direct_elements
        .iter()
        .find(|child| {
            child.tag_name().namespace() != Some(DCS_SCHEMA_NS)
                || !matches!(child.tag_name().name(), "name" | "field")
        })
        .map(|child| child.range().start)
        .unwrap_or_else(|| {
            range.start
                + xml_text[range.clone()]
                    .rfind("</")
                    .unwrap_or(range.end - range.start)
        });
    let line_start = xml_text[..insert_at]
        .rfind('\n')
        .map_or(insert_at, |position| position + 1);
    let field_insert_pos = if xml_text[line_start..insert_at]
        .chars()
        .all(char::is_whitespace)
    {
        line_start
    } else {
        insert_at
    };
    Ok(DcsEditDataSetTarget {
        range: (range.start, range.end),
        name,
        emit_field_value_type: data_set_type != "DataSetQuery",
        field_insert_pos,
        child_indent,
    })
}

pub(crate) fn dcs_edit_set_query(
    xml_text: &mut String,
    data_set: &str,
    query: &str,
) -> Result<(), String> {
    let range = dcs_edit_dataset_range(xml_text, data_set)?;
    dcs_edit_replace_child_text(xml_text, range, "query", query)
}

pub(crate) fn dcs_edit_extract_once_marker(value: &str) -> (String, bool) {
    let mut once = false;
    let cleaned = value
        .split_whitespace()
        .filter(|part| {
            if *part == "@once" {
                once = true;
                false
            } else {
                true
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    (cleaned, once)
}

pub(crate) fn dcs_edit_patch_query(
    xml_text: &mut String,
    data_set: &str,
    old: &str,
    new: &str,
    once: bool,
) -> Result<usize, String> {
    let range = dcs_edit_dataset_range(xml_text, data_set)?;
    let query_range = dcs_edit_child_text_range(xml_text, range, "query")?;
    let current = &xml_text[query_range.clone()];
    let escaped_old = escape_xml(old);
    let count = current.matches(&escaped_old).count();
    if count == 0 {
        return Err(format!(
            "Substring not found in query of dataset '{}': {}",
            dcs_edit_dataset_name(xml_text, data_set).unwrap_or_else(|| data_set.to_string()),
            old
        ));
    }
    if once && count != 1 {
        return Err(format!(
            "@once: expected 1 occurrence of '{}' in dataset '{}', found {}",
            old,
            dcs_edit_dataset_name(xml_text, data_set).unwrap_or_else(|| data_set.to_string()),
            count
        ));
    }
    let patched = current.replace(&escaped_old, &escape_xml(new));
    xml_text.replace_range(query_range, &patched);
    Ok(count)
}

pub(crate) fn dcs_edit_dataset_range(
    xml_text: &str,
    data_set: &str,
) -> Result<(usize, usize), String> {
    Ok(dcs_edit_dataset_target(xml_text, data_set)?.range)
}

pub(crate) fn dcs_edit_dataset_name(xml_text: &str, data_set: &str) -> Option<String> {
    Some(dcs_edit_dataset_target(xml_text, data_set).ok()?.name)
}

pub(crate) fn dcs_edit_variant_name(xml_text: &str, variant: &str) -> Option<String> {
    if !variant.is_empty() {
        return Some(variant.to_string());
    }
    let (start, end) = dcs_edit_variant_range(xml_text, variant).ok()?;
    let name_range = dcs_edit_prefixed_child_text_range(xml_text, (start, end), "dcsset:name")
        .or_else(|_| dcs_edit_child_text_range(xml_text, (start, end), "name"))
        .ok()?;
    Some(xml_text[name_range].trim().to_string())
}

pub(crate) fn dcs_edit_variant_range(
    xml_text: &str,
    variant: &str,
) -> Result<(usize, usize), String> {
    let document =
        Document::parse(xml_text).map_err(|error| format!("XML parse error: {error}"))?;
    let root = document.root_element();
    if let Some(node) = root
        .children()
        .filter(|node| role_info_element(*node, "settingsVariant", Some(DCS_SCHEMA_NS)))
        .find(|node| {
            variant.is_empty()
                || node
                    .children()
                    .filter(roxmltree::Node::is_element)
                    .find(|child| child.tag_name().name() == "name")
                    .is_some_and(|name| dcs_text_of(name) == variant)
        })
    {
        let range = node.range();
        return Ok((range.start, range.end));
    }
    if variant.is_empty() {
        Err("No settingsVariant found in DCS".to_string())
    } else {
        Err(format!("Variant '{variant}' not found"))
    }
}

pub(crate) fn dcs_edit_variant_block_has_name(block: &str, variant: &str) -> bool {
    let escaped = escape_xml(variant);
    block.contains(&format!("<dcsset:name>{escaped}</dcsset:name>"))
        || block.contains(&format!("<name>{escaped}</name>"))
}

pub(crate) fn dcs_edit_settings_element_range(
    xml_text: &str,
    variant: &str,
) -> Result<(usize, usize), String> {
    let variant_range = dcs_edit_variant_range(xml_text, variant)?;
    let document =
        Document::parse(xml_text).map_err(|error| format!("XML parse error: {error}"))?;
    let variant_node = dcs_edit_element_for_range(&document, variant_range)
        .ok_or_else(|| "settingsVariant element not found".to_string())?;
    let settings = variant_node
        .children()
        .find(|node| role_info_element(*node, "settings", Some(DCS_SETTINGS_NS)))
        .ok_or_else(|| "No settings element found in variant".to_string())?;
    let range = settings.range();
    Ok((range.start, range.end))
}

pub(crate) fn dcs_edit_settings_content_range(
    xml_text: &str,
    variant: &str,
) -> Result<(usize, usize), String> {
    let settings = dcs_edit_settings_element_range(xml_text, variant)?;
    let Some(open_end_rel) = xml_text[settings.0..settings.1].find('>') else {
        return Err("Malformed <dcsset:settings> element".to_string());
    };
    let content_start = settings.0 + open_end_rel + 1;
    let content_end = settings.1 - "</dcsset:settings>".len();
    Ok((content_start, content_end))
}

pub(crate) fn dcs_edit_insert_before_dataset_close(
    xml_text: &mut String,
    range: (usize, usize),
    fragment: &str,
) -> Result<(), String> {
    let close = "</dataSet>";
    let Some(close_rel) = xml_text[range.0..range.1].rfind(close) else {
        return Err("No closing </dataSet> found".to_string());
    };
    let pos = range.0 + close_rel;
    xml_text.insert_str(pos, &format!("{fragment}\n\t"));
    Ok(())
}

pub(crate) fn dcs_edit_replace_child_text(
    xml_text: &mut String,
    range: (usize, usize),
    child: &str,
    value: &str,
) -> Result<(), String> {
    let text_range = dcs_edit_child_text_range(xml_text, range, child)?;
    xml_text.replace_range(text_range, &escape_xml(value));
    Ok(())
}

pub(crate) fn dcs_edit_child_text_range(
    xml_text: &str,
    range: (usize, usize),
    child: &str,
) -> Result<std::ops::Range<usize>, String> {
    let (start, end) = dcs_edit_child_element_range(xml_text, range, child)
        .ok_or_else(|| format!("No direct <{child}> element found"))?;
    dcs_edit_element_content_range(xml_text, (start, end))
        .ok_or_else(|| format!("Malformed or self-closing <{child}> element"))
}

pub(crate) fn dcs_edit_element_content_range(
    xml_text: &str,
    range: (usize, usize),
) -> Option<std::ops::Range<usize>> {
    let (start, end) = range;
    let open_end = xml_text[start..end]
        .find('>')
        .map(|relative| start + relative + 1)?;
    if xml_text[start..open_end].trim_end().ends_with("/>") {
        return None;
    }
    let qualified_name = dcs_edit_element_name_at(xml_text, start)?;
    let close = format!("</{qualified_name}>");
    let close_start = xml_text[open_end..end]
        .rfind(&close)
        .map(|relative| open_end + relative)?;
    Some(open_end..close_start)
}

pub(crate) fn dcs_edit_prefixed_child_text_range(
    xml_text: &str,
    range: (usize, usize),
    child: &str,
) -> Result<std::ops::Range<usize>, String> {
    dcs_edit_child_text_range(xml_text, range, child)
}

pub(crate) struct DcsEditSelectionValue {
    pub(crate) field: String,
    pub(crate) group: Option<String>,
}

pub(crate) fn dcs_edit_parse_selection_value(value: &str) -> DcsEditSelectionValue {
    let mut field = value.trim().to_string();
    let mut group = None;
    if let Some(marker) = field.find(" @group=") {
        let tail = field[marker + " @group=".len()..].trim();
        group = tail.split_whitespace().next().map(ToOwned::to_owned);
        field.truncate(marker);
        field = field.trim().to_string();
    }
    DcsEditSelectionValue { field, group }
}

pub(crate) fn dcs_edit_selection_fragment(field_name: &str, indent: &str) -> String {
    if field_name == "Auto" {
        return format!("{indent}<dcsset:item xsi:type=\"dcsset:SelectedItemAuto\"/>");
    }
    if let Some(inner) = field_name
        .strip_prefix("Folder(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let (title, raw_items) = inner
            .split_once(':')
            .map(|(title, items)| (title.trim(), items))
            .unwrap_or(("", inner));
        let items = raw_items
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>();
        let mut lines = vec![format!(
            "{indent}<dcsset:item xsi:type=\"dcsset:SelectedItemFolder\">"
        )];
        if !title.is_empty() {
            lines.push(format!("{indent}\t<dcsset:lwsTitle>"));
            lines.push(format!("{indent}\t\t<v8:item>"));
            lines.push(format!("{indent}\t\t\t<v8:lang>ru</v8:lang>"));
            lines.push(format!(
                "{indent}\t\t\t<v8:content>{}</v8:content>",
                escape_xml(title)
            ));
            lines.push(format!("{indent}\t\t</v8:item>"));
            lines.push(format!("{indent}\t</dcsset:lwsTitle>"));
        }
        for item in items {
            lines.push(format!(
                "{indent}\t<dcsset:item xsi:type=\"dcsset:SelectedItemField\">"
            ));
            lines.push(format!(
                "{indent}\t\t<dcsset:field>{}</dcsset:field>",
                escape_xml(item)
            ));
            lines.push(format!("{indent}\t</dcsset:item>"));
        }
        lines.push(format!(
            "{indent}\t<dcsset:placement>Auto</dcsset:placement>"
        ));
        lines.push(format!("{indent}</dcsset:item>"));
        return lines.join("\n");
    }
    format!(
        "{indent}<dcsset:item xsi:type=\"dcsset:SelectedItemField\">\n{indent}\t<dcsset:field>{}</dcsset:field>\n{indent}</dcsset:item>",
        escape_xml(field_name)
    )
}

pub(crate) fn dcs_edit_insert_selection_into_group(
    xml_text: &mut String,
    variant: &str,
    group_name: &str,
    field_name: &str,
) -> Result<bool, String> {
    let Some(group_range) = dcs_edit_find_named_structure_group(xml_text, variant, group_name)?
    else {
        return Ok(false);
    };
    let Some(selection_range) =
        dcs_edit_find_prefixed_child_range(xml_text, group_range, "dcsset:selection")?
    else {
        return Ok(false);
    };
    let indent = if selection_range.self_closing {
        format!(
            "{}\t",
            dcs_edit_line_indent(xml_text, selection_range.start)
        )
    } else {
        format!(
            "{}\t",
            dcs_edit_line_indent(xml_text, selection_range.close_start)
        )
    };
    let fragment = dcs_edit_selection_fragment(field_name, &indent);
    dcs_edit_insert_into_prefixed_range(xml_text, selection_range, "dcsset:selection", &fragment);
    Ok(true)
}

pub(crate) fn dcs_edit_find_prefixed_child_range(
    xml_text: &str,
    parent_range: (usize, usize),
    child: &str,
) -> Result<Option<DcsEditElementRange>, String> {
    let Some(range) = dcs_edit_child_element_range(xml_text, parent_range, child) else {
        return Ok(None);
    };
    dcs_edit_element_range_details(xml_text, range, child).map(Some)
}

pub(crate) fn dcs_edit_insert_into_prefixed_range(
    xml_text: &mut String,
    range: DcsEditElementRange,
    container: &str,
    fragment: &str,
) {
    if range.self_closing {
        let indent = dcs_edit_line_indent(xml_text, range.start);
        xml_text.replace_range(
            range.start..range.end,
            &format!("<{container}>\n{fragment}\n{indent}</{container}>"),
        );
    } else {
        let insert_pos = xml_text[..range.close_start]
            .rfind('\n')
            .map(|idx| idx + 1)
            .unwrap_or(range.close_start);
        xml_text.insert_str(insert_pos, &format!("{fragment}\n"));
    }
}

pub(crate) fn dcs_edit_order_fragment(value: &str, indent: &str) -> String {
    let value = value.trim();
    if value == "Auto" {
        return format!("{indent}<dcsset:item xsi:type=\"dcsset:OrderItemAuto\"/>");
    }
    let mut parts = value.split_whitespace();
    let field = parts.next().unwrap_or(value);
    let direction = if parts
        .next()
        .is_some_and(|item| item.eq_ignore_ascii_case("desc"))
    {
        "Desc"
    } else {
        "Asc"
    };
    format!(
        "{indent}<dcsset:item xsi:type=\"dcsset:OrderItemField\">\n{indent}\t<dcsset:field>{}</dcsset:field>\n{indent}\t<dcsset:orderType>{direction}</dcsset:orderType>\n{indent}</dcsset:item>",
        escape_xml(field)
    )
}

pub(crate) fn dcs_edit_order_description(value: &str) -> String {
    let value = value.trim();
    if value == "Auto" {
        return "Auto".to_string();
    }
    let mut parts = value.split_whitespace();
    let field = parts.next().unwrap_or(value);
    let direction = if parts
        .next()
        .is_some_and(|item| item.eq_ignore_ascii_case("desc"))
    {
        "Desc"
    } else {
        "Asc"
    };
    format!("{field} {direction}")
}

pub(crate) struct DcsEditElementRange {
    pub(crate) start: usize,
    pub(crate) open_end: usize,
    pub(crate) close_start: usize,
    pub(crate) end: usize,
    pub(crate) self_closing: bool,
}

pub(crate) fn dcs_edit_element_range_details(
    xml_text: &str,
    range: (usize, usize),
    description: &str,
) -> Result<DcsEditElementRange, String> {
    let (start, end) = range;
    let Some(open_end_rel) = xml_text[start..end].find('>') else {
        return Err(format!("Malformed <{description}> element"));
    };
    let open_end = start + open_end_rel + 1;
    if xml_text[start..open_end].trim_end().ends_with("/>") {
        return Ok(DcsEditElementRange {
            start,
            open_end,
            close_start: open_end,
            end: open_end,
            self_closing: true,
        });
    }
    let qualified_name = dcs_edit_element_name_at(xml_text, start)
        .ok_or_else(|| format!("Malformed <{description}> element"))?;
    let close = format!("</{qualified_name}>");
    let close_start = xml_text[open_end..end]
        .rfind(&close)
        .map(|relative| open_end + relative)
        .ok_or_else(|| format!("No closing </{description}> found"))?;
    Ok(DcsEditElementRange {
        start,
        open_end,
        close_start,
        end: close_start + close.len(),
        self_closing: false,
    })
}

pub(crate) fn dcs_edit_prefixed_container_range(
    xml_text: &str,
    variant: &str,
    container: &str,
) -> Result<DcsEditElementRange, String> {
    let settings_element = dcs_edit_settings_element_range(xml_text, variant)?;
    let document =
        Document::parse(xml_text).map_err(|error| format!("XML parse error: {error}"))?;
    let settings = dcs_edit_element_for_range(&document, settings_element)
        .ok_or_else(|| "DCS settings element not found".to_string())?;
    let child = settings
        .children()
        .find(|node| {
            node.is_element()
                && dcs_edit_requested_name_matches(*node, container, Some(DCS_SETTINGS_NS))
        })
        .ok_or_else(|| format!("No <{container}> section found in DCS"))?;
    let child_range = child.range();
    let start = child_range.start;
    let Some(open_end_rel) = xml_text[start..child_range.end].find('>') else {
        return Err(format!("Malformed <{container}> section in DCS"));
    };
    let open_end = start + open_end_rel + 1;
    let open_tag = &xml_text[start..open_end];
    if open_tag.trim_end().ends_with("/>") {
        return Ok(DcsEditElementRange {
            start,
            open_end,
            close_start: open_end,
            end: open_end,
            self_closing: true,
        });
    }
    let qualified_name = dcs_edit_element_name_at(xml_text, start)
        .ok_or_else(|| format!("Malformed <{container}> section in DCS"))?;
    let close = format!("</{qualified_name}>");
    let Some(close_rel) = xml_text[open_end..child_range.end].rfind(&close) else {
        return Err(format!("No </{container}> section found in DCS"));
    };
    let close_start = open_end + close_rel;
    Ok(DcsEditElementRange {
        start,
        open_end,
        close_start,
        end: close_start + close.len(),
        self_closing: false,
    })
}

pub(crate) fn dcs_edit_insert_prefixed_item(
    xml_text: &mut String,
    variant: &str,
    container: &str,
    fragment: &str,
) -> Result<(), String> {
    let range = dcs_edit_prefixed_container_range(xml_text, variant, container)?;
    if range.self_closing {
        let indent = dcs_edit_line_indent(xml_text, range.start);
        xml_text.replace_range(
            range.start..range.end,
            &format!("<{container}>\n{fragment}\n{indent}</{container}>"),
        );
    } else {
        let insert_pos = xml_text[..range.close_start]
            .rfind('\n')
            .map(|idx| idx + 1)
            .unwrap_or(range.close_start);
        xml_text.insert_str(insert_pos, &format!("{fragment}\n"));
    }
    Ok(())
}

pub(crate) fn dcs_edit_clear_prefixed_container(
    xml_text: &mut String,
    variant: &str,
    container: &str,
) -> Result<(), String> {
    let range = dcs_edit_prefixed_container_range(xml_text, variant, container)?;
    if range.self_closing {
        return Ok(());
    }
    let indent = dcs_edit_line_indent(xml_text, range.start);
    xml_text.replace_range(range.open_end..range.close_start, &format!("\n{indent}"));
    Ok(())
}

pub(crate) fn dcs_edit_prefixed_container_contains_field(
    xml_text: &str,
    variant: &str,
    container: &str,
    field: &str,
) -> bool {
    let Ok(range) = dcs_edit_prefixed_container_range(xml_text, variant, container) else {
        return false;
    };
    if range.self_closing {
        return false;
    }
    xml_text[range.open_end..range.close_start].contains(&format!(
        "<dcsset:field>{}</dcsset:field>",
        escape_xml(field)
    ))
}

pub(crate) fn dcs_edit_remove_dataset_item(
    xml_text: &mut String,
    data_set: &str,
    item: &str,
    child: &str,
    value: &str,
) -> Result<bool, String> {
    let range = dcs_edit_dataset_range(xml_text, data_set)?;
    let Some((start, end)) = dcs_edit_find_item_by_child(xml_text, range, item, child, value)
    else {
        return Ok(false);
    };
    let removal = dcs_edit_element_line_range(xml_text, start, end);
    xml_text.replace_range(removal, "");
    Ok(true)
}

pub(crate) fn dcs_edit_remove_top_level_item(
    xml_text: &mut String,
    item: &str,
    child: &str,
    value: &str,
) -> Result<bool, String> {
    let document =
        Document::parse(xml_text).map_err(|error| format!("XML parse error: {error}"))?;
    let root = document.root_element();
    let Some(node) = root
        .children()
        .filter(|node| {
            node.is_element() && dcs_edit_requested_name_matches(*node, item, Some(DCS_SCHEMA_NS))
        })
        .find(|node| {
            node.children()
                .filter(roxmltree::Node::is_element)
                .any(|candidate| {
                    dcs_edit_requested_name_matches(candidate, child, node.tag_name().namespace())
                        && dcs_text_of(candidate) == value
                })
        })
    else {
        return Ok(false);
    };
    let range = node.range();
    let remove = dcs_edit_element_line_range(xml_text, range.start, range.end);
    xml_text.replace_range(remove, "");
    Ok(true)
}

pub(crate) fn dcs_edit_remove_item_by_child(
    xml_text: &mut String,
    range: (usize, usize),
    item: &str,
    child: &str,
    value: &str,
) -> Result<bool, String> {
    let document =
        Document::parse(xml_text).map_err(|error| format!("XML parse error: {error}"))?;
    let parent = dcs_edit_element_for_range(&document, range)
        .ok_or_else(|| format!("DCS parent element at byte {} not found", range.0))?;
    let default_namespace = parent.tag_name().namespace();
    let Some(node) = parent
        .descendants()
        .skip(1)
        .filter(|node| {
            node.is_element() && dcs_edit_requested_name_matches(*node, item, default_namespace)
        })
        .find(|node| {
            let item_namespace = node.tag_name().namespace();
            node.children()
                .filter(roxmltree::Node::is_element)
                .any(|candidate| {
                    dcs_edit_requested_name_matches(candidate, child, item_namespace)
                        && dcs_text_of(candidate) == value
                })
        })
    else {
        return Ok(false);
    };
    let node_range = node.range();
    let removal = dcs_edit_element_line_range(xml_text, node_range.start, node_range.end);
    xml_text.replace_range(removal, "");
    Ok(true)
}

pub(crate) fn dcs_edit_block_has_child_text(block: &str, child: &str, value: &str) -> bool {
    let escaped = escape_xml(value);
    let exact = format!("<{child}>{escaped}</{child}>");
    if block.contains(&exact) {
        return true;
    }
    let open = format!("<{child} ");
    let close = format!("</{child}>");
    let mut cursor = 0usize;
    while let Some(open_rel) = block[cursor..].find(&open) {
        let start = cursor + open_rel;
        let Some(open_end_rel) = block[start..].find('>') else {
            return false;
        };
        let text_start = start + open_end_rel + 1;
        let Some(close_rel) = block[text_start..].find(&close) else {
            return false;
        };
        let text_end = text_start + close_rel;
        if block[text_start..text_end].trim() == escaped {
            return true;
        }
        cursor = text_end + close.len();
    }
    false
}

pub(crate) fn dcs_edit_line_start(xml_text: &str, pos: usize) -> usize {
    xml_text[..pos].rfind('\n').map(|idx| idx + 1).unwrap_or(0)
}

pub(crate) fn dcs_edit_element_line_range(
    xml_text: &str,
    start: usize,
    end: usize,
) -> std::ops::Range<usize> {
    let line_start = dcs_edit_line_start(xml_text, start);
    let remove_start = if xml_text[line_start..start]
        .chars()
        .all(|ch| ch == '\t' || ch == ' ')
    {
        line_start
    } else {
        start
    };
    let remove_end = if xml_text[end..].starts_with("\r\n") {
        end + 2
    } else if xml_text[end..].starts_with('\n') {
        end + 1
    } else {
        end
    };
    remove_start..remove_end
}

pub(crate) fn dcs_edit_matching_element_end(
    xml_text: &str,
    start: usize,
    limit: usize,
    tag: &str,
) -> Option<usize> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let first_open_end = xml_text[start..limit].find('>')? + start;
    let first_tag = &xml_text[start..=first_open_end];
    if first_tag.trim_end().ends_with("/>") {
        return Some(first_open_end + 1);
    }
    let mut depth = 1usize;
    let mut cursor = first_open_end + 1;
    while cursor < limit {
        let next_open = xml_text[cursor..limit].find(&open).map(|rel| cursor + rel);
        let next_close = xml_text[cursor..limit].find(&close).map(|rel| cursor + rel);
        match (next_open, next_close) {
            (Some(open_pos), Some(close_pos)) if open_pos < close_pos => {
                let open_end = xml_text[open_pos..limit].find('>')? + open_pos;
                let open_tag = &xml_text[open_pos..=open_end];
                if !open_tag.trim_end().ends_with("/>") {
                    depth += 1;
                }
                cursor = open_end + 1;
            }
            (_, Some(close_pos)) => {
                depth = depth.saturating_sub(1);
                let end = close_pos + close.len();
                if depth == 0 {
                    return Some(end);
                }
                cursor = end;
            }
            _ => return None,
        }
    }
    None
}

pub(crate) fn dcs_edit_remove_prefixed_selection_field(
    xml_text: &mut String,
    variant: &str,
    field: &str,
) -> Result<bool, String> {
    let Ok(selection) = dcs_edit_prefixed_container_range(xml_text, variant, "dcsset:selection")
    else {
        return Ok(false);
    };
    if selection.self_closing {
        return Ok(false);
    }
    dcs_edit_remove_item_by_child(
        xml_text,
        (selection.start, selection.end),
        "dcsset:item",
        "dcsset:field",
        field,
    )
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
        "dcs-info" => Some(Ok(analyze_dcs_info(args, context))),
        "dcs-validate" => Some(Ok(validate_dcs(args, context))),
        _ => None,
    }
}

pub(crate) fn invoke_mutation(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<AdapterOutcome> {
    match operation {
        "dcs-compile" => Some(compile_dcs(args, context)),
        "dcs-edit" => Some(edit_dcs(args, context)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::compile_transaction::{with_commit_failpoint, CommitFailpoint};
    use super::super::single_file_publisher::{
        with_before_commit_hook, with_publish_failpoints, PublishCheckpoint,
    };
    use super::*;
    use crate::domain::workspace::WorkspaceContext;
    use serde_json::{json, Map};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    const TEST_DCS_SETTINGS_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/settings";
    const TEST_DCS_CORE_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/core";
    const TEST_DCS_COMMON_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/common";

    fn exact_dcs_bytes(xml: &str) -> Vec<u8> {
        let mut bytes = b"\xef\xbb\xbf".to_vec();
        bytes.extend_from_slice(xml.replace("\r\n", "\n").replace('\n', "\r\n").as_bytes());
        bytes
    }

    fn dcs_edit_args(operation: &str, value: &str, no_validate: bool) -> Map<String, Value> {
        Map::from_iter([
            ("TemplatePath".to_string(), json!("Template.xml")),
            ("Operation".to_string(), json!(operation)),
            ("Value".to_string(), json!(value)),
            ("NoValidate".to_string(), json!(no_validate)),
        ])
    }

    fn dcs_compile_args(definition: &Value, output_path: &str) -> Map<String, Value> {
        Map::from_iter([
            ("Value".to_string(), Value::String(definition.to_string())),
            ("OutputPath".to_string(), json!(output_path)),
        ])
    }

    fn valid_compile_definition() -> Value {
        json!({
            "dataSets": [{
                "name": "Data",
                "query": "SELECT 1 AS Value",
                "fields": ["Value"]
            }]
        })
    }

    #[test]
    fn dcs_edit_no_validate_still_rolls_back_semantically_invalid_final_dcs() {
        let context = temp_context("dcs-edit-invalid-post-validation");
        let template_path = context.cwd.join("Template.xml");
        let invalid = base_dcs_xml().replace(
            "<dataSource>ИсточникДанных1</dataSource>",
            "<dataSource>MissingSource</dataSource>",
        );
        let original = exact_dcs_bytes(&invalid);
        fs::write(&template_path, &original).unwrap();
        let args = dcs_edit_args("add-total", "Amount: SUM(Amount)", true);

        let outcome = edit_dcs(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("unknown dataSource"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&template_path).unwrap(), original);
        fs::remove_dir_all(&context.cwd).unwrap();
    }

    #[test]
    fn dcs_edit_post_write_validation_failure_restores_exact_bytes() {
        let context = temp_context("dcs-edit-post-write-rollback");
        let template_path = context.cwd.join("Template.xml");
        let original = exact_dcs_bytes(base_dcs_xml());
        fs::write(&template_path, &original).unwrap();
        let args = dcs_edit_args("add-total", "Amount: SUM(Amount)", false);

        let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            edit_dcs(&args, &context)
        });

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("post-write validation"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&template_path).unwrap(), original);
        fs::remove_dir_all(&context.cwd).unwrap();
    }

    #[test]
    fn dcs_edit_preserves_a_concurrent_replacement_instead_of_overwriting_it() {
        let context = temp_context("dcs-edit-concurrent-replacement");
        let template_path = context.cwd.join("Template.xml");
        fs::write(&template_path, exact_dcs_bytes(base_dcs_xml())).unwrap();
        let concurrent = exact_dcs_bytes(&base_dcs_xml().replace(
            "<query>ВЫБРАТЬ Amount КАК Amount</query>",
            "<query>ВЫБРАТЬ Amount КАК ConcurrentAmount</query>",
        ));
        let concurrent_for_hook = concurrent.clone();
        let expected_target = template_path.clone();
        let args = dcs_edit_args("add-total", "Amount: SUM(Amount)", false);

        let outcome = with_dcs_edit_after_read_hook(
            move |target| {
                assert_eq!(target, expected_target);
                fs::write(target, concurrent_for_hook).unwrap();
            },
            || edit_dcs(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.errors.join("\n").contains("changed"), "{outcome:?}");
        assert_eq!(fs::read(&template_path).unwrap(), concurrent);
        fs::remove_dir_all(&context.cwd).unwrap();
    }

    #[test]
    fn dcs_edit_no_validate_suppresses_output_but_not_validation() {
        for (no_validate, expect_report) in [(false, true), (true, false)] {
            let context = temp_context(&format!("dcs-edit-validation-output-{no_validate}"));
            fs::write(context.cwd.join("Template.xml"), base_dcs_xml()).unwrap();
            let args = dcs_edit_args("add-total", "Amount: SUM(Amount)", no_validate);

            let execution = edit_dcs_with_data(&args, &context);
            let outcome = &execution.outcome;

            assert!(outcome.ok, "{outcome:?}");
            // The report was presentation; validation itself never depended on
            // NoValidate, and the typed answer says so directly.
            let _ = expect_report;
            let data = execution.data.as_ref().expect("dcs.edit answers with data");
            assert!(
                data.validated,
                "post-write validation runs whatever NoValidate says: {data:?}"
            );
            fs::remove_dir_all(&context.cwd).unwrap();
        }
    }

    #[test]
    fn dcs_edit_rejects_a_version_attribute_on_the_versionless_root_without_writing() {
        for file_name in ["Template.xml", "Template"] {
            let context = temp_context(&format!(
                "dcs-edit-versionless-root-policy-{}",
                file_name.replace('.', "-")
            ));
            let template_path = context.cwd.join(file_name);
            let source = base_dcs_xml().replacen(
                "<DataCompositionSchema ",
                "<DataCompositionSchema version=\"2.21\" ",
                1,
            );
            let original = exact_dcs_bytes(&source);
            fs::write(&template_path, &original).unwrap();
            let mut args = dcs_edit_args("add-total", "Amount: SUM(Amount)", false);
            args.insert("TemplatePath".to_string(), json!(file_name));

            let outcome = edit_dcs(&args, &context);

            assert!(!outcome.ok, "{outcome:?}");
            assert!(
                outcome
                    .errors
                    .join("\n")
                    .contains("must not carry a version attribute"),
                "{outcome:?}"
            );
            assert_eq!(fs::read(&template_path).unwrap(), original);
            fs::remove_dir_all(&context.cwd).unwrap();
        }
    }

    #[test]
    fn dcs_compile_rejects_invalid_generated_dcs_without_leaving_a_file() {
        let context = temp_context("dcs-compile-invalid-post-validation");
        let definition = json!({
            "dataSources": [{"name": "KnownSource", "type": "Local"}],
            "dataSets": [{
                "name": "Data",
                "source": "MissingSource",
                "query": "SELECT 1 AS Value",
                "fields": ["Value"]
            }]
        });
        let args = dcs_compile_args(&definition, "out/Template.xml");

        let outcome = compile_dcs(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("unknown dataSource"),
            "{outcome:?}"
        );
        assert!(!context.cwd.join("out/Template.xml").exists());
        assert!(!context.cwd.join("out").exists());
        fs::remove_dir_all(&context.cwd).unwrap();
    }

    #[test]
    fn dcs_compile_post_write_validation_failure_removes_created_file_and_directory() {
        let context = temp_context("dcs-compile-post-write-rollback");
        let args = dcs_compile_args(&valid_compile_definition(), "out/Template.xml");

        let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            compile_dcs(&args, &context)
        });

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("post-write validation"),
            "{outcome:?}"
        );
        assert!(!context.cwd.join("out/Template.xml").exists());
        assert!(!context.cwd.join("out").exists());
        fs::remove_dir_all(&context.cwd).unwrap();
    }

    #[test]
    fn dcs_compile_directly_rejects_existing_wrong_root_without_write() {
        let context = temp_context("dcs-compile-existing-wrong-root");
        let output_path = context.cwd.join("Template.xml");
        let original = b"<garbage/>".to_vec();
        fs::write(&output_path, &original).unwrap();
        let args = dcs_compile_args(&valid_compile_definition(), "Template.xml");

        let outcome = compile_dcs(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("declared platform XML target root")),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&output_path).unwrap(), original);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
        fs::remove_dir_all(&context.cwd).unwrap();
    }

    #[test]
    fn dcs_compile_rolls_back_if_format_owner_changes_during_publication() {
        let context = temp_context("dcs-compile-format-owner-race");
        let source = context.cwd.join("src");
        let output = source.join("Templates/Guarded/Ext/Template.xml");
        let owner = source.join("Configuration.xml");
        fs::create_dir_all(output.parent().unwrap()).unwrap();
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            &owner,
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Configuration uuid=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\"><Properties><Name>Demo</Name></Properties><ChildObjects/></Configuration></MetaDataObject>",
        )
        .unwrap();
        let args = dcs_compile_args(
            &valid_compile_definition(),
            "src/Templates/Guarded/Ext/Template.xml",
        );
        let owner_for_hook = owner.clone();

        let outcome = with_before_commit_hook(
            move |_| {
                fs::write(
                    &owner_for_hook,
                    "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.21\"><Configuration/></MetaDataObject>",
                )
                .unwrap();
            },
            || compile_dcs(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert!(!output.exists());
        fs::remove_dir_all(&context.cwd).unwrap();
    }

    #[test]
    fn dcs_compile_rolls_back_if_selected_query_file_changes_after_read() {
        let context = temp_context("dcs-compile-query-input-race");
        let query_path = context.cwd.join("query.bsl");
        let output_path = context.cwd.join("Template.xml");
        fs::write(&query_path, "SELECT 1 AS Value").unwrap();
        let definition = json!({
            "dataSets": [{
                "name": "Data",
                "query": "@query.bsl",
                "fields": ["Value"]
            }]
        });
        let args = dcs_compile_args(&definition, "Template.xml");
        let concurrent_query = b"SELECT 2 AS ConcurrentValue".to_vec();
        let query_for_hook = query_path.clone();
        let concurrent_for_hook = concurrent_query.clone();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&query_for_hook, &concurrent_for_hook).unwrap(),
            || compile_dcs(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&query_path).unwrap(), concurrent_query);
        assert!(!output_path.exists());
        fs::remove_dir_all(&context.cwd).unwrap();
    }

    #[test]
    fn dcs_edit_rolls_back_if_selected_query_file_changes_after_read() {
        let context = temp_context("dcs-edit-query-input-race");
        let template_path = context.cwd.join("Template.xml");
        let query_path = context.cwd.join("query.bsl");
        let original = exact_dcs_bytes(base_dcs_xml());
        fs::write(&template_path, &original).unwrap();
        fs::write(&query_path, "SELECT 1 AS Value").unwrap();
        let args = Map::from_iter([
            ("TemplatePath".to_string(), json!("Template.xml")),
            ("Operation".to_string(), json!("set-query")),
            ("DataSet".to_string(), json!("НаборДанных1")),
            ("Value".to_string(), json!("@query.bsl")),
        ]);
        let concurrent_query = b"SELECT 2 AS ConcurrentValue".to_vec();
        let query_for_hook = query_path.clone();
        let concurrent_for_hook = concurrent_query.clone();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&query_for_hook, &concurrent_for_hook).unwrap(),
            || edit_dcs(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&query_path).unwrap(), concurrent_query);
        assert_eq!(fs::read(&template_path).unwrap(), original);
        fs::remove_dir_all(&context.cwd).unwrap();
    }

    #[test]
    fn dcs_compile_surfaces_cleanup_warnings_after_a_committed_create() {
        let context = temp_context("dcs-compile-cleanup-warning");
        let args = dcs_compile_args(&valid_compile_definition(), "Template.xml");

        let outcome = with_publish_failpoints(&[PublishCheckpoint::Cleanup], || {
            compile_dcs(&args, &context)
        });

        assert!(outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .warnings
                .iter()
                .any(|warning| warning.contains("injected publication cleanup failure")),
            "{outcome:?}"
        );
        assert!(context.cwd.join("Template.xml").is_file());
        fs::remove_dir_all(&context.cwd).unwrap();
    }

    #[test]
    fn dcs_info_rejects_wrong_root_namespace_without_output() {
        let context = temp_context("dcs-info-wrong-root-ns");
        let template_path = context.cwd.join("Template.xml");
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
            ("TemplatePath".to_string(), json!("Template.xml")),
            ("OutFile".to_string(), json!("info.txt")),
        ]);

        let outcome = analyze_dcs_info(&args, &context);

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
        let context = temp_context("dcs-info-raw-query");
        let template_path = context.cwd.join("Template.xml");
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
            ("TemplatePath".to_string(), json!("Template.xml")),
            ("Mode".to_string(), json!("query")),
            ("Name".to_string(), json!("НаборДанных1")),
            ("Raw".to_string(), json!(true)),
            ("Limit".to_string(), json!(1)),
            ("Offset".to_string(), json!(100)),
        ]);

        let execution = analyze_dcs_info_with_data(&args, &context);

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

    /// ADR-0048 removed the eleven `Mode` values on the claim that the typed
    /// answer is the union of the projections they used to print. That claim is
    /// only true if every fact those reports carried survives, so this fixture
    /// puts one of each into a schema and reads them all back out of `data`.
    #[test]
    fn dcs_info_data_carries_every_fact_the_retired_modes_printed() {
        let context = temp_context("dcs-info-complete-read-model");
        let template_path = context.cwd.join("Template.xml");
        fs::write(&template_path, complete_dcs_xml()).unwrap();

        let execution = analyze_dcs_info_with_data(
            &Map::from_iter([("TemplatePath".to_string(), json!("Template.xml"))]),
            &context,
        );

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        let data = execution.data.expect("dcs.info answers with data");

        // `overview` printed the owner's support state and the data sources.
        assert!(!data.support.state.is_empty(), "{data:?}");
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
    fn dcs_edit_rejects_wrong_root_namespace_without_write() {
        let context = temp_context("dcs-edit-wrong-root-ns");
        let template_path = context.cwd.join("Template.xml");
        let original = base_dcs_xml()
            .replace(
                "http://v8.1c.ru/8.1/data-composition-system/schema",
                "urn:not-dcs",
            )
            .into_bytes();
        fs::write(&template_path, &original).unwrap();
        let args = Map::from_iter([
            ("TemplatePath".to_string(), json!("Template.xml")),
            ("Operation".to_string(), json!("add-field")),
            ("Value".to_string(), json!("Quantity: decimal(10,0)")),
        ]);

        let outcome = edit_dcs(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("urn:not-dcs") && error.contains("DataCompositionSchema")));
        assert_eq!(fs::read(&template_path).unwrap(), original);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn dcs_compile_orders_non_query_field_children_like_platform_8_3_27() {
        let definition = json!({
            "dataSets": [
                {
                    "name": "Query",
                    "query": "SELECT 1 AS Value",
                    "fields": ["Value:string"]
                },
                {
                    "name": "Object",
                    "objectName": "Catalog.Items",
                    "fields": [{
                        "dataPath": "Value",
                        "field": "Value",
                        "type": "string",
                        "presentationExpression": "Value"
                    }]
                },
                {
                    "name": "Union",
                    "fields": [{
                        "dataPath": "UnionValue",
                        "field": "UnionValue",
                        "type": "string",
                        "presentationExpression": "UnionValue"
                    }],
                    "items": [{
                        "name": "UnionQuery",
                        "query": "SELECT 1 AS UnionValue"
                    }]
                }
            ]
        });

        let xml = dcs_compile_xml(&definition, Path::new("."), Path::new(".")).unwrap();
        let document = Document::parse(&xml).unwrap();
        let root = document.root_element();
        let query = test_data_set(root, "Query");
        let object = test_data_set(root, "Object");
        let union = test_data_set(root, "Union");
        let query_field = test_data_set_field(query, "Value");
        let object_field = test_data_set_field(object, "Value");
        let union_field = test_data_set_field(union, "UnionValue");

        assert_eq!(
            test_direct_child_names(query),
            ["name", "field", "dataSource", "query"]
        );
        assert_eq!(test_direct_child_names(query_field), ["dataPath", "field"]);
        assert_eq!(
            test_direct_child_names(object),
            ["name", "field", "dataSource", "objectName"]
        );
        assert_eq!(
            test_direct_child_names(object_field),
            ["dataPath", "field", "presentationExpression", "valueType"]
        );
        assert_eq!(
            test_direct_child_names(union_field),
            ["dataPath", "field", "presentationExpression", "valueType"]
        );
    }

    #[test]
    fn dcs_compile_canonicalizes_string_synonyms_for_object_and_union_fields() {
        const V8_DATA_NS: &str = "http://v8.1c.ru/8.1/data/core";
        let definition = json!({
            "dataSets": [
                {
                    "name": "Object",
                    "objectName": "Catalog.Items",
                    "fields": ["ObjectValue:String"]
                },
                {
                    "name": "Union",
                    "fields": ["UnionValue:string"],
                    "items": [{
                        "name": "UnionQuery",
                        "query": "SELECT 1 AS UnionValue"
                    }]
                }
            ]
        });

        let xml = dcs_compile_xml(&definition, Path::new("."), Path::new(".")).unwrap();
        let document = Document::parse(&xml).unwrap();
        let root = document.root_element();

        for (data_set_name, field_name) in [("Object", "ObjectValue"), ("Union", "UnionValue")] {
            let data_set = test_data_set(root, data_set_name);
            let field = test_data_set_field(data_set, field_name);
            let value_type = dcs_child(field, "valueType", DCS_SCHEMA_NS).unwrap();
            assert_eq!(
                test_direct_child_names(value_type),
                ["Type", "StringQualifiers"],
                "{data_set_name}"
            );
            assert_eq!(
                dcs_child(value_type, "Type", V8_DATA_NS).and_then(|node| node.text()),
                Some("xs:string"),
                "{data_set_name}"
            );
            let qualifiers = dcs_child(value_type, "StringQualifiers", V8_DATA_NS).unwrap();
            assert_eq!(
                test_direct_child_names(qualifiers),
                ["Length", "AllowedLength"],
                "{data_set_name}"
            );
            assert_eq!(
                dcs_child(qualifiers, "Length", V8_DATA_NS).and_then(|node| node.text()),
                Some("0"),
                "{data_set_name}"
            );
            assert_eq!(
                dcs_child(qualifiers, "AllowedLength", V8_DATA_NS).and_then(|node| node.text()),
                Some("Variable"),
                "{data_set_name}"
            );
        }
    }

    #[test]
    fn dcs_compile_value_type_uses_xsd_group_order_after_full_validation() {
        const V8_DATA_NS: &str = "http://v8.1c.ru/8.1/data/core";
        let definition = json!({
            "dataSets": [{
                "name": "Object",
                "objectName": "Catalog.Items",
                "fields": [{
                    "field": "Value",
                    "type": [
                        "ВидыСубконтоХозрасчетные",
                        "typeid:00112233-4455-6677-8899-aabbccddeeff",
                        "date",
                        "string(12)",
                        "decimal(15,2,nonneg)",
                        "CatalogRef.Items",
                        "boolean"
                    ]
                }]
            }]
        });

        let xml = dcs_compile_xml(&definition, Path::new("."), Path::new(".")).unwrap();
        let document = Document::parse(&xml).unwrap();
        let field = test_data_set_field(test_data_set(document.root_element(), "Object"), "Value");
        let value_type = dcs_child(field, "valueType", DCS_SCHEMA_NS).unwrap();

        assert_eq!(
            test_direct_child_names(value_type),
            [
                "Type",
                "Type",
                "Type",
                "Type",
                "Type",
                "TypeSet",
                "TypeId",
                "NumberQualifiers",
                "StringQualifiers",
                "DateQualifiers",
            ]
        );
        assert_eq!(
            dcs_children(value_type, "Type", V8_DATA_NS)
                .into_iter()
                .map(dcs_text_of)
                .collect::<Vec<_>>(),
            [
                "xs:dateTime",
                "xs:string",
                "xs:decimal",
                "d5p1:CatalogRef.Items",
                "xs:boolean",
            ]
        );
        assert_eq!(
            dcs_child(value_type, "TypeSet", V8_DATA_NS).map(dcs_text_of),
            Some("d5p1:ВидыСубконтоХозрасчетные".to_string())
        );
        assert_eq!(
            dcs_child(value_type, "TypeId", V8_DATA_NS).map(dcs_text_of),
            Some("00112233-4455-6677-8899-aabbccddeeff".to_string())
        );
    }

    #[test]
    fn dcs_compile_value_type_rejects_invalid_8_3_27_contract_values() {
        for type_name in [
            "string(1025)",
            "decimal(39,0)",
            "decimal(10,11)",
            "decimal(10,2,nonnegative)",
            "MysteryRef.Items",
            "CatalogRef.Items.Extra",
            "typeid:not-a-uuid",
            "string||boolean",
            "string|String(10)",
        ] {
            let definition = json!({
                "dataSets": [{
                    "name": "Object",
                    "objectName": "Catalog.Items",
                    "fields": [{ "field": "Value", "type": type_name }]
                }]
            });

            let error =
                dcs_compile_xml(&definition, Path::new("."), Path::new(".")).expect_err(type_name);
            assert!(
                error.contains(type_name) || error.contains("duplicate platform type"),
                "unexpected error for {type_name}: {error}"
            );
        }

        let definition = json!({
            "dataSets": [{
                "name": "Query",
                "query": "SELECT 1 AS Value",
                "fields": [{ "field": "Value", "type": ["string", ""] }]
            }]
        });
        let error = dcs_compile_xml(&definition, Path::new("."), Path::new("."))
            .expect_err("empty array item must be rejected before query-field omission");
        assert!(error.contains("empty item"), "{error}");

        let definition = json!({
            "dataSets": [{
                "name": "Query",
                "query": "SELECT 1 AS Value",
                "fields": ["Value:"]
            }]
        });
        let error = dcs_compile_xml(&definition, Path::new("."), Path::new("."))
            .expect_err("explicitly empty shorthand type must be rejected");
        assert!(error.contains("empty item"), "{error}");

        let mut lines = vec!["sentinel".to_string()];
        let error = dcs_compile_emit_value_type(&mut lines, "string|decimal(39,0)", "\t")
            .expect_err("all entries must be checked before the first entry is emitted");
        assert!(error.contains("decimal(39,0)"), "{error}");
        assert_eq!(lines, ["sentinel"]);
    }

    #[test]
    fn dcs_compile_rejects_defined_type_that_platform_8_3_27_drops_without_write() {
        let context = temp_context("dcs-compile-defined-type-no-write");
        let output_path = context.cwd.join("Template.xml");
        let original = base_dcs_xml().as_bytes().to_vec();
        fs::write(&output_path, &original).unwrap();
        let definition = json!({
            "dataSets": [{
                "name": "Main",
                "query": "SELECT 1 AS Value",
                "fields": ["Value"]
            }],
            "parameters": [{
                "name": "NamedType",
                "type": "DefinedType.NamedType"
            }]
        });
        let args = Map::from_iter([
            ("OutputPath".to_string(), json!("Template.xml")),
            ("Value".to_string(), json!(definition.to_string())),
        ]);

        let outcome = compile_dcs(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.iter().any(|error| {
                error.contains("DefinedType.NamedType")
                    && error.contains("8.3.27")
                    && error.contains("round-trip")
            }),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&output_path).unwrap(), original);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn dcs_compile_calculated_field_merges_restrictions_in_8_3_27_order() {
        let definition = json!({
            "calculatedFields": [{
                "dataPath": "Total",
                "expression": "Quantity * Price",
                "title": "Total",
                "type": "decimal(15,2)",
                "restrict": ["noGroup", "noFilter"],
                "useRestriction": ["noField", "noFilter"]
            }]
        });

        let xml = dcs_compile_xml(&definition, Path::new("."), Path::new(".")).unwrap();
        let document = Document::parse(&xml).unwrap();
        let calculated =
            dcs_child(document.root_element(), "calculatedField", DCS_SCHEMA_NS).unwrap();

        assert_eq!(
            test_direct_child_names(calculated),
            [
                "dataPath",
                "expression",
                "title",
                "useRestriction",
                "valueType"
            ]
        );
        let restriction = dcs_child(calculated, "useRestriction", DCS_SCHEMA_NS).unwrap();
        assert_eq!(
            test_direct_child_names(restriction),
            ["field", "condition", "group"]
        );
        assert_eq!(
            dcs_children(calculated, "useRestriction", DCS_SCHEMA_NS).len(),
            1,
            "restrict and useRestriction aliases must merge into one XSD child"
        );
    }

    #[test]
    fn dcs_compile_parameter_combination_follows_8_3_27_order() {
        let definition = json!({
            "parameters": [{
                "name": "Choice",
                "title": "Choice",
                "type": "string(20)",
                "value": "A",
                "useRestriction": true,
                "expression": "&Source.Choice",
                "valueListAllowed": true,
                "availableAsField": false,
                "denyIncompleteValues": true,
                "use": "Always"
            }]
        });

        let xml = dcs_compile_xml(&definition, Path::new("."), Path::new(".")).unwrap();
        let document = Document::parse(&xml).unwrap();
        let parameter = dcs_child(document.root_element(), "parameter", DCS_SCHEMA_NS).unwrap();

        assert_eq!(
            test_direct_child_names(parameter),
            [
                "name",
                "title",
                "valueType",
                "value",
                "useRestriction",
                "expression",
                "valueListAllowed",
                "availableAsField",
                "denyIncompleteValues",
                "use",
            ]
        );
    }

    #[test]
    fn dcs_compile_omits_required_true_default_but_preserves_false() {
        let definition = json!({
            "dataSets": [
                {
                    "name": "Source",
                    "query": "SELECT 1 AS Value",
                    "fields": ["Value"]
                },
                {
                    "name": "RequiredDestination",
                    "query": "SELECT 1 AS Value",
                    "fields": ["Value"]
                },
                {
                    "name": "OptionalDestination",
                    "query": "SELECT 1 AS Value",
                    "fields": ["Value"]
                }
            ],
            "dataSetLinks": [
                {
                    "source": "Source",
                    "dest": "RequiredDestination",
                    "sourceExpr": "Value",
                    "destExpr": "Value",
                    "required": true
                },
                {
                    "source": "Source",
                    "dest": "OptionalDestination",
                    "sourceExpr": "Value",
                    "destExpr": "Value",
                    "required": false
                }
            ]
        });

        let xml = dcs_compile_xml(&definition, Path::new("."), Path::new(".")).unwrap();
        let document = Document::parse(&xml).unwrap();
        let links = dcs_children(document.root_element(), "dataSetLink", DCS_SCHEMA_NS);

        assert_eq!(links.len(), 2);
        assert!(
            dcs_child(links[0], "required", DCS_SCHEMA_NS).is_none(),
            "8.3.27 canonical export omits the default true value\n{xml}"
        );
        assert_eq!(
            dcs_child(links[1], "required", DCS_SCHEMA_NS).map(dcs_text_of),
            Some("false".to_string())
        );
    }

    #[test]
    fn dcs_compile_complex_definition_follows_8_3_27_direct_child_order() {
        const DCS_SETTINGS_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/settings";
        let definition = json!({
            "dataSets": [
                {
                    "name": "Union",
                    "fields": ["Value"],
                    "items": [
                        {
                            "name": "First",
                            "query": "SELECT 1 AS Value",
                            "fields": ["Value"]
                        },
                        {
                            "name": "Second",
                            "objectName": "Catalog.Items",
                            "fields": [{ "field": "Value", "type": "string" }]
                        }
                    ]
                },
                {
                    "name": "Other",
                    "query": "SELECT 1 AS Value",
                    "fields": ["Value"]
                }
            ],
            "dataSetLinks": [{
                "source": "Union",
                "dest": "Other",
                "sourceExpr": "Value",
                "destExpr": "Value",
                "parameter": "LinkParameter",
                "parameterListAllowed": true,
                "linkConditionExpression": "Value <> 0",
                "startExpression": "Value",
                "required": false
            }],
            "parameters": [{
                "name": "Choice",
                "title": "Choice",
                "type": "string(20)",
                "value": "A",
                "useRestriction": true,
                "expression": "&Source.Choice",
                "availableValues": [
                    { "value": "A", "presentation": "Alpha" },
                    { "value": "B", "presentation": "Beta" }
                ],
                "valueListAllowed": true,
                "availableAsField": false,
                "denyIncompleteValues": true,
                "use": "Always"
            }],
            "settingsVariants": [{
                "name": "Main",
                "settings": {
                    "selection": ["Value"],
                    "filter": ["Value > 0"],
                    "dataParameters": [{ "parameter": "LinkParameter", "value": "A" }],
                    "order": [{ "field": "Value", "direction": "Asc" }],
                    "conditionalAppearance": [{
                        "fields": ["Value"],
                        "filter": ["Value > 0"],
                        "appearance": { "ЦветТекста": "web:Red" }
                    }],
                    "outputParameters": { "Title": "Report" },
                    "structure": [{
                        "type": "group",
                        "name": "Group",
                        "groupBy": ["Value"],
                        "filter": ["Value > 0"],
                        "order": [{ "field": "Value", "direction": "Desc" }],
                        "selection": ["Value"],
                        "conditionalAppearance": [{
                            "fields": ["Value"],
                            "filter": ["Value > 0"],
                            "appearance": { "ЦветТекста": "web:Red" }
                        }],
                        "outputParameters": { "Title": "Group" },
                        "children": [{ "type": "group", "name": "Nested" }]
                    }]
                }
            }]
        });

        let xml = dcs_compile_xml(&definition, Path::new("."), Path::new(".")).unwrap();
        let document = Document::parse(&xml).unwrap();
        let root = document.root_element();

        let union = test_data_set(root, "Union");
        assert_eq!(
            test_direct_child_names(union),
            ["name", "field", "item", "item"]
        );

        let link = dcs_child(root, "dataSetLink", DCS_SCHEMA_NS).unwrap();
        assert_eq!(
            test_direct_child_names(link),
            [
                "sourceDataSet",
                "destinationDataSet",
                "sourceExpression",
                "destinationExpression",
                "parameter",
                "parameterListAllowed",
                "linkConditionExpression",
                "startExpression",
                "required",
            ]
        );

        let parameter = dcs_child(root, "parameter", DCS_SCHEMA_NS).unwrap();
        assert_eq!(
            test_direct_child_names(parameter),
            [
                "name",
                "title",
                "valueType",
                "value",
                "useRestriction",
                "expression",
                "availableValue",
                "availableValue",
                "valueListAllowed",
                "availableAsField",
                "denyIncompleteValues",
                "use",
            ]
        );
        for available_value in dcs_children(parameter, "availableValue", DCS_SCHEMA_NS) {
            assert_eq!(
                test_direct_child_names(available_value),
                ["value", "presentation"]
            );
        }

        let variant = dcs_child(root, "settingsVariant", DCS_SCHEMA_NS).unwrap();
        let settings = dcs_child(variant, "settings", DCS_SETTINGS_NS).unwrap();
        assert_eq!(
            test_direct_child_names(settings),
            [
                "selection",
                "filter",
                "dataParameters",
                "order",
                "conditionalAppearance",
                "outputParameters",
                "item",
            ]
        );
        let group = dcs_child(settings, "item", DCS_SETTINGS_NS).unwrap();
        assert_eq!(
            test_direct_child_names(group),
            [
                "name",
                "groupItems",
                "filter",
                "order",
                "selection",
                "conditionalAppearance",
                "outputParameters",
                "item",
            ]
        );
        for (parent, expected_title) in [(settings, "Report"), (group, "Group")] {
            let conditional = dcs_child(parent, "conditionalAppearance", DCS_SETTINGS_NS).unwrap();
            let item = dcs_child(conditional, "item", DCS_SETTINGS_NS).unwrap();
            assert_eq!(
                test_direct_child_names(item),
                ["selection", "filter", "appearance"]
            );
            let appearance = dcs_child(item, "appearance", DCS_SETTINGS_NS).unwrap();
            let appearance_item = dcs_child(appearance, "item", TEST_DCS_CORE_NS).unwrap();
            assert_eq!(
                dcs_child(appearance_item, "parameter", TEST_DCS_CORE_NS)
                    .map(dcs_text_of)
                    .as_deref(),
                Some("ЦветТекста")
            );
            assert_eq!(
                dcs_child(appearance_item, "value", TEST_DCS_CORE_NS)
                    .map(dcs_all_text)
                    .as_deref(),
                Some("web:Red")
            );

            let output_parameters = dcs_child(parent, "outputParameters", DCS_SETTINGS_NS).unwrap();
            let output_item = dcs_child(output_parameters, "item", TEST_DCS_CORE_NS).unwrap();
            assert_eq!(
                dcs_child(output_item, "parameter", TEST_DCS_CORE_NS)
                    .map(dcs_text_of)
                    .as_deref(),
                Some("Title")
            );
            assert_eq!(
                dcs_child(output_item, "value", TEST_DCS_CORE_NS)
                    .map(dcs_all_text)
                    .as_deref(),
                Some(expected_title)
            );
        }
    }

    #[test]
    fn dcs_compile_string_filter_omits_absent_view_mode() {
        const DCS_SETTINGS_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/settings";
        let definition = json!({
            "settingsVariants": [{
                "name": "Main",
                "settings": { "filter": ["Amount > 0"] }
            }]
        });

        let xml = dcs_compile_xml(&definition, Path::new("."), Path::new(".")).unwrap();
        let document = Document::parse(&xml).unwrap();
        let variant = dcs_child(document.root_element(), "settingsVariant", DCS_SCHEMA_NS).unwrap();
        let settings = dcs_child(variant, "settings", DCS_SETTINGS_NS).unwrap();
        let filter = dcs_child(settings, "filter", DCS_SETTINGS_NS).unwrap();
        let item = dcs_child(filter, "item", DCS_SETTINGS_NS).unwrap();

        assert_eq!(
            test_direct_child_names(item),
            ["left", "comparisonType", "right"]
        );
        assert!(!xml.contains("<dcsset:viewMode>None</dcsset:viewMode>"));
    }

    #[test]
    fn dcs_compile_rejects_malformed_available_values_before_writing() {
        let context = temp_context("dcs-compile-malformed-available-values");
        let output_path = context.cwd.join("Template.xml");
        let original = b"existing output".to_vec();
        fs::write(&output_path, &original).unwrap();
        let definition = json!({
            "dataSets": [{
                "name": "Main",
                "query": "SELECT 1 AS Value",
                "fields": ["Value"]
            }],
            "parameters": [{
                "name": "Choice",
                "type": "string",
                "availableValues": [{ "presentation": "Missing value" }]
            }]
        });
        let args = Map::from_iter([
            ("OutputPath".to_string(), json!("Template.xml")),
            ("Value".to_string(), json!(definition.to_string())),
        ]);

        let outcome = compile_dcs(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("availableValues") && error.contains("value")),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&output_path).unwrap(), original);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn dcs_edit_add_parameter_combination_follows_8_3_27_order() {
        let xml = edit_field_test_xml(
            "dcs-edit-parameter-order",
            base_dcs_xml(),
            "add-parameter",
            "Choice [Choice]: string(20) = A,B @hidden @always @valueList availableValue=A:Alpha,B:Beta",
        );
        let document = Document::parse(&xml).unwrap();
        let parameter = dcs_children(document.root_element(), "parameter", DCS_SCHEMA_NS)
            .into_iter()
            .find(|parameter| {
                dcs_child(*parameter, "name", DCS_SCHEMA_NS)
                    .is_some_and(|name| dcs_text_of(name) == "Choice")
            })
            .unwrap();

        assert_eq!(
            test_direct_child_names(parameter),
            [
                "name",
                "title",
                "valueType",
                "value",
                "value",
                "useRestriction",
                "availableValue",
                "availableValue",
                "valueListAllowed",
                "availableAsField",
                "use",
            ]
        );
    }

    #[test]
    fn dcs_edit_auto_dates_parameters_follow_8_3_27_order() {
        let xml = edit_field_test_xml(
            "dcs-edit-auto-date-parameter-order",
            base_dcs_xml(),
            "add-parameter",
            "Period [Period]: StandardPeriod = LastMonth @autoDates",
        );
        let document = Document::parse(&xml).unwrap();

        for name in ["ДатаНачала", "ДатаОкончания"] {
            let parameter = dcs_children(document.root_element(), "parameter", DCS_SCHEMA_NS)
                .into_iter()
                .find(|parameter| {
                    dcs_child(*parameter, "name", DCS_SCHEMA_NS)
                        .is_some_and(|node| dcs_text_of(node) == name)
                })
                .unwrap_or_else(|| panic!("parameter {name} not found"));
            assert_eq!(
                test_direct_child_names(parameter),
                [
                    "name",
                    "title",
                    "valueType",
                    "value",
                    "useRestriction",
                    "expression",
                    "availableAsField",
                ],
                "{name}"
            );
        }
    }

    #[test]
    fn dcs_edit_named_standard_period_omits_custom_only_dates() {
        let xml = edit_field_test_xml(
            "dcs-edit-standard-period-canonical-value",
            base_dcs_xml(),
            "add-parameter",
            "Period [Period]: StandardPeriod = LastMonth @autoDates",
        );
        let document = Document::parse(&xml).unwrap();
        let parameter = dcs_children(document.root_element(), "parameter", DCS_SCHEMA_NS)
            .into_iter()
            .find(|parameter| {
                dcs_child(*parameter, "name", DCS_SCHEMA_NS)
                    .is_some_and(|name| dcs_text_of(name) == "Period")
            })
            .unwrap();
        let value = dcs_child(parameter, "value", DCS_SCHEMA_NS).unwrap();

        assert_eq!(
            dcs_child(value, "variant", V8_DATA_NS)
                .map(dcs_text_of)
                .as_deref(),
            Some("LastMonth")
        );
        assert!(dcs_child(value, "startDate", V8_DATA_NS).is_none());
        assert!(dcs_child(value, "endDate", V8_DATA_NS).is_none());

        let custom = dcs_edit_parameter_value_lines("StandardPeriod", "Custom", "\t", "value")
            .unwrap()
            .join("\n");
        assert!(custom.contains("<v8:startDate>"));
        assert!(custom.contains("<v8:endDate>"));
    }

    #[test]
    fn dcs_compile_invalid_calculated_field_type_does_not_overwrite_output() {
        let context = temp_context("dcs-compile-invalid-calculated-type-no-write");
        let output_path = context.cwd.join("Template.xml");
        let original = b"existing output".to_vec();
        fs::write(&output_path, &original).unwrap();
        let definition = json!({
            "dataSets": [{
                "name": "Main",
                "query": "SELECT 1 AS Value",
                "fields": ["Value"]
            }],
            "calculatedFields": [{
                "dataPath": "Broken",
                "expression": "1",
                "type": "decimal(39,0)",
                "restrict": ["noGroup"]
            }]
        });
        let args = Map::from_iter([
            ("OutputPath".to_string(), json!("Template.xml")),
            ("Value".to_string(), json!(definition.to_string())),
        ]);

        let outcome = compile_dcs(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("decimal(39,0)")),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&output_path).unwrap(), original);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn dcs_edit_invalid_parameter_type_does_not_write() {
        let context = temp_context("dcs-edit-invalid-parameter-type");
        let template_path = context.cwd.join("Template.xml");
        let original = base_dcs_xml().as_bytes().to_vec();
        fs::write(&template_path, &original).unwrap();
        let args = Map::from_iter([
            ("TemplatePath".to_string(), json!("Template.xml")),
            ("Operation".to_string(), json!("add-parameter")),
            ("Value".to_string(), json!("Broken: decimal(39,0) = 1")),
        ]);

        let outcome = edit_dcs(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("decimal(39,0)")));
        assert_eq!(fs::read(&template_path).unwrap(), original);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn dcs_edit_rejects_defined_type_that_platform_8_3_27_drops_without_write() {
        let context = temp_context("dcs-edit-defined-type-no-write");
        let template_path = context.cwd.join("Template.xml");
        let original = base_dcs_xml().as_bytes().to_vec();
        fs::write(&template_path, &original).unwrap();
        let args = Map::from_iter([
            ("TemplatePath".to_string(), json!("Template.xml")),
            ("Operation".to_string(), json!("add-parameter")),
            (
                "Value".to_string(),
                json!("NamedType: DefinedType.NamedType"),
            ),
        ]);

        let outcome = edit_dcs(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.iter().any(|error| {
                error.contains("DefinedType.NamedType")
                    && error.contains("8.3.27")
                    && error.contains("round-trip")
            }),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&template_path).unwrap(), original);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn dcs_compile_duplicate_wire_type_does_not_overwrite_output() {
        let context = temp_context("dcs-compile-duplicate-type-no-write");
        let output_path = context.cwd.join("Template.xml");
        let original = b"existing output".to_vec();
        fs::write(&output_path, &original).unwrap();
        let definition = json!({
            "dataSets": [{
                "name": "Object",
                "objectName": "Catalog.Items",
                "fields": [{ "field": "Value", "type": ["string", "String(10)"] }]
            }]
        });
        let args = Map::from_iter([
            ("OutputPath".to_string(), json!("Template.xml")),
            ("Value".to_string(), json!(definition.to_string())),
        ]);

        let outcome = compile_dcs(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("duplicate platform type")));
        assert_eq!(fs::read(&output_path).unwrap(), original);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn dcs_edit_add_field_uses_dataset_specific_value_type_rule_and_order() {
        let query_xml = edit_field_test_xml(
            "edit-add-query-field",
            base_dcs_xml(),
            "add-field",
            "Added:String",
        );
        let query_document = Document::parse(&query_xml).unwrap();
        let query_data_set = test_data_set(query_document.root_element(), "НаборДанных1");
        let query_field = test_data_set_field(query_data_set, "Added");

        assert_eq!(
            test_direct_child_names(query_data_set),
            ["name", "field", "field", "dataSource", "query"]
        );
        assert_eq!(test_direct_child_names(query_field), ["dataPath", "field"]);

        let object_source = base_dcs_xml()
            .replace("xsi:type=\"DataSetQuery\"", "xsi:type=\"DataSetObject\"")
            .replace(
                "<query>ВЫБРАТЬ Amount КАК Amount</query>",
                "<objectName>Catalog.Items</objectName>",
            );
        let object_xml = edit_field_test_xml(
            "edit-add-object-field",
            &object_source,
            "add-field",
            "Added:String",
        );
        let object_document = Document::parse(&object_xml).unwrap();
        let object_data_set = test_data_set(object_document.root_element(), "НаборДанных1");
        let object_field = test_data_set_field(object_data_set, "Added");

        assert_eq!(
            test_direct_child_names(object_field),
            ["dataPath", "field", "valueType"]
        );
    }

    #[test]
    fn dcs_edit_modify_query_field_removes_noncanonical_value_type() {
        let source = base_dcs_xml().replacen(
            "\t\t</field>",
            "\t\t\t<valueType>\n\t\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t</valueType>\n\t\t</field>",
            1,
        );

        let xml = edit_field_test_xml(
            "edit-modify-query-field",
            &source,
            "modify-field",
            "Amount [Updated]",
        );
        let document = Document::parse(&xml).unwrap();
        let data_set = test_data_set(document.root_element(), "НаборДанных1");
        let field = test_data_set_field(data_set, "Amount");

        assert_eq!(
            test_direct_child_names(field),
            ["dataPath", "field", "title"]
        );
    }

    #[test]
    fn dcs_edit_add_field_inserts_into_nested_query_before_its_payload() {
        let xml = edit_field_test_xml_for_dataset(
            "edit-add-nested-query-field",
            &nested_union_dcs_xml(),
            "InnerQuery",
            "add-field",
            "Added:String",
        );
        let document = Document::parse(&xml).unwrap();
        let union = test_data_set(document.root_element(), "Union");
        let inner = test_data_set(union, "InnerQuery");
        let added = test_data_set_field(inner, "Added");

        assert_eq!(
            test_direct_child_names(inner),
            ["name", "field", "field", "dataSource", "query"]
        );
        assert_eq!(test_direct_child_names(added), ["dataPath", "field"]);
    }

    #[test]
    fn dcs_edit_modify_field_targets_nested_query_when_outer_field_has_same_path() {
        let xml = edit_field_test_xml_for_dataset(
            "edit-modify-nested-query-field",
            &nested_union_dcs_xml(),
            "InnerQuery",
            "modify-field",
            "Value [Inner updated]:String",
        );
        let document = Document::parse(&xml).unwrap();
        let union = test_data_set(document.root_element(), "Union");
        let outer_field = test_data_set_field(union, "Value");
        let inner = test_data_set(union, "InnerQuery");
        let inner_field = test_data_set_field(inner, "Value");

        assert_eq!(test_direct_child_names(outer_field), ["dataPath", "field"]);
        assert_eq!(
            test_direct_child_names(inner_field),
            ["dataPath", "field", "title"]
        );
        assert!(
            xml.contains("\n\t\t\t\t<title xsi:type=\"v8:LocalStringType\">"),
            "nested field child must keep its four-tab indentation:\n{xml}"
        );
    }

    #[test]
    fn native_dcs_edit_accepts_documented_operations_without_script_fallback() {
        let context = temp_context("dcs-edit-ops");
        let template_path = context.cwd.join("Template.xml");
        fs::write(&template_path, base_dcs_xml()).unwrap();

        let operations = [
            ("add-field", "Quantity: decimal(10,0)"),
            ("add-total", "Amount: Сумма(Amount)"),
            (
                "add-calculated-field",
                "Margin: decimal(10,2) = Amount - Cost",
            ),
            (
                "add-parameter",
                "Период [Период]: StandardPeriod = LastMonth @autoDates",
            ),
            ("add-filter", "Amount > 0 @user"),
            ("add-dataParameter", "Период = LastMonth @user"),
            ("add-order", "Amount desc"),
            ("add-selection", "Quantity"),
            ("add-dataSet", "Доп: ВЫБРАТЬ 1 КАК Amount"),
            (
                "add-dataSetLink",
                "НаборДанных1 > Доп on Amount = Amount [param LinkParam]",
            ),
            ("add-variant", "Alt [Alt presentation]"),
            (
                "add-conditionalAppearance",
                "ЦветТекста = web:Red when Amount < 0 for Amount",
            ),
            ("add-drilldown", "Amount"),
            ("set-query", "ВЫБРАТЬ 2 КАК Amount"),
            ("patch-query", "2 => 3"),
            ("set-outputParameter", "Заголовок = Test"),
            ("set-structure", "Amount > details @name=Данные"),
            ("modify-field", "Quantity [Qty]: decimal(15,2)"),
            ("modify-filter", "Amount >= 1 @off"),
            ("modify-dataParameter", "Период = ThisMonth @off"),
            (
                "modify-parameter",
                "Период [Period title] value=ThisYear @hidden @always",
            ),
            ("modify-structure", "Quantity > details @name=Данные"),
            ("set-field-role", "Quantity @dimension"),
            ("rename-parameter", "Период => ПериодОтчета"),
            (
                "reorder-parameters",
                "ПериодОтчета, ДатаНачала, ДатаОкончания",
            ),
            ("clear-selection", "*"),
            ("clear-order", "*"),
            ("clear-filter", "*"),
            ("clear-conditionalAppearance", "*"),
            ("remove-field", "Quantity"),
            ("remove-total", "Amount"),
            ("remove-calculated-field", "Margin"),
            ("remove-parameter", "ПериодОтчета"),
            ("remove-filter", "Amount"),
        ];

        for (operation, value) in operations {
            let mut args = Map::new();
            args.insert("TemplatePath".to_string(), json!("Template.xml"));
            args.insert("Operation".to_string(), json!(operation));
            args.insert("Value".to_string(), json!(value));
            let outcome = edit_dcs(&args, &context);
            assert!(
                outcome.ok,
                "{operation} failed: {:?}\nstderr={:?}",
                outcome.errors, outcome.stderr
            );
            assert_eq!(outcome.command, None);
            let updated = fs::read_to_string(&template_path).unwrap();
            roxmltree::Document::parse(updated.trim_start_matches('\u{feff}'))
                .unwrap_or_else(|err| panic!("{operation} left invalid XML: {err}\n{updated}"));
        }

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn native_dcs_edit_structure_preserves_nested_named_groups() {
        let context = temp_context("dcs-edit-structure");
        let template_path = context.cwd.join("Template.xml");
        fs::write(&template_path, base_dcs_xml()).unwrap();

        let mut args = Map::new();
        args.insert("TemplatePath".to_string(), json!("Template.xml"));
        args.insert("Operation".to_string(), json!("set-structure"));
        args.insert(
            "Value".to_string(),
            json!("Amount @name=G1 > Quantity @name=G2 > details"),
        );
        let outcome = edit_dcs(&args, &context);
        assert!(outcome.ok, "{outcome:?}");

        args.insert("Operation".to_string(), json!("modify-structure"));
        args.insert("Value".to_string(), json!("Price @name=G2"));
        let outcome = edit_dcs(&args, &context);
        assert!(outcome.ok, "{outcome:?}");

        let updated = fs::read_to_string(&template_path).unwrap();
        assert!(
            updated.contains("<dcsset:name>G1</dcsset:name>"),
            "{updated}"
        );
        assert!(
            updated.contains("<dcsset:name>G2</dcsset:name>"),
            "{updated}"
        );
        assert!(
            updated.contains("xsi:type=\"dcsset:GroupItemField\""),
            "{updated}"
        );
        assert!(
            updated.contains("<dcsset:groupType>Items</dcsset:groupType>"),
            "{updated}"
        );
        assert!(
            updated.contains("<dcsset:field>Amount</dcsset:field>"),
            "{updated}"
        );
        assert!(
            updated.contains("<dcsset:field>Price</dcsset:field>"),
            "{updated}"
        );
        assert!(
            !updated.contains("<dcsset:field>Quantity</dcsset:field>"),
            "{updated}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn native_dcs_edit_scopes_settings_changes_to_requested_variant() {
        let context = temp_context("dcs-edit-variant");
        let template_path = context.cwd.join("Template.xml");
        fs::write(&template_path, two_variant_dcs_xml()).unwrap();

        let mut args = Map::new();
        args.insert("TemplatePath".to_string(), json!("Template.xml"));
        args.insert("Operation".to_string(), json!("add-selection"));
        args.insert("Value".to_string(), json!("Amount"));
        args.insert("Variant".to_string(), json!("Дополнительный"));

        let outcome = edit_dcs(&args, &context);
        assert!(outcome.ok, "{outcome:?}");

        let updated = fs::read_to_string(&template_path).unwrap();
        let primary = variant_block(&updated, "Основной");
        let secondary = variant_block(&updated, "Дополнительный");
        assert!(
            !primary.contains("<dcsset:field>Amount</dcsset:field>"),
            "{primary}"
        );
        assert!(
            secondary.contains("<dcsset:field>Amount</dcsset:field>"),
            "{secondary}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn native_dcs_edit_patch_query_honors_once_marker() {
        let context = temp_context("dcs-edit-patch-once");
        let template_path = context.cwd.join("Template.xml");
        fs::write(
            &template_path,
            base_dcs_xml().replace("ВЫБРАТЬ Amount КАК Amount", "ВЫБРАТЬ Code КАК Code"),
        )
        .unwrap();

        let mut args = Map::new();
        args.insert("TemplatePath".to_string(), json!("Template.xml"));
        args.insert("Operation".to_string(), json!("patch-query"));
        args.insert("Value".to_string(), json!("Code => ItemCode @once"));

        let outcome = edit_dcs(&args, &context);
        assert!(!outcome.ok, "{outcome:?}");
        let stderr = outcome.stderr.unwrap_or_default();
        assert!(stderr.contains("@once: expected 1 occurrence"), "{stderr}");
        let unchanged = fs::read_to_string(&template_path).unwrap();
        assert!(unchanged.contains("ВЫБРАТЬ Code КАК Code"), "{unchanged}");

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn native_dcs_edit_rename_parameter_uses_token_boundaries() {
        let context = temp_context("dcs-edit-rename-boundary");
        let template_path = context.cwd.join("Template.xml");
        fs::write(&template_path, parameter_dcs_xml()).unwrap();

        let mut args = Map::new();
        args.insert("TemplatePath".to_string(), json!("Template.xml"));
        args.insert("Operation".to_string(), json!("rename-parameter"));
        args.insert("Value".to_string(), json!("Период => ПериодОтчета"));

        let outcome = edit_dcs(&args, &context);
        assert!(outcome.ok, "{outcome:?}");

        let updated = fs::read_to_string(&template_path).unwrap();
        assert!(updated.contains("<name>ПериодОтчета</name>"), "{updated}");
        assert!(
            updated.contains("<expression>&amp;ПериодОтчета</expression>"),
            "{updated}"
        );
        assert!(
            updated.contains("<expression>&amp;ПериодОтчетаДокумента</expression>"),
            "{updated}"
        );
        assert!(
            updated.contains("<dcscor:parameter>ПериодОтчета</dcscor:parameter>"),
            "{updated}"
        );
        assert!(
            updated.contains("<dcscor:parameter>ПериодОтчетаДокумента</dcscor:parameter>"),
            "{updated}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn native_dcs_edit_noop_leaves_file_untouched() {
        let context = temp_context("dcs-edit-noop");
        let template_path = context.cwd.join("Template.xml");
        fs::write(&template_path, base_dcs_xml()).unwrap();
        let before = fs::read(&template_path).unwrap();

        let mut args = Map::new();
        args.insert("TemplatePath".to_string(), json!("Template.xml"));
        args.insert("Operation".to_string(), json!("remove-filter"));
        args.insert("Value".to_string(), json!("MissingField"));

        let execution = edit_dcs_with_data(&args, &context);
        let outcome = &execution.outcome;
        assert!(outcome.ok, "{outcome:?}");
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        let data = execution.data.as_ref().expect("dcs.edit answers with data");
        assert!(!data.changed, "{data:?}");
        assert!(
            data.items.iter().all(|item| !item.applied),
            "a no-op reports every item as not applied: {:?}",
            data.items
        );
        assert_eq!(fs::read(&template_path).unwrap(), before);

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

    #[test]
    fn dcs_edit_creates_data_parameters_before_order() {
        let xml = edit_field_test_xml(
            "dcs-edit-data-parameters-order",
            base_dcs_xml(),
            "add-dataParameter",
            "Period = Today",
        );

        assert_eq!(
            test_settings_child_names(&xml),
            ["selection", "filter", "dataParameters", "order", "item"]
        );
    }

    #[test]
    fn dcs_edit_creates_order_after_data_parameters() {
        let source = base_dcs_xml().replace(
            "\t\t\t<dcsset:order>\n\t\t\t</dcsset:order>\n",
            "\t\t\t<dcsset:dataParameters>\n\t\t\t\t<dcscor:item xsi:type=\"dcsset:SettingsParameterValue\">\n\t\t\t\t\t<dcscor:parameter>Period</dcscor:parameter>\n\t\t\t\t</dcscor:item>\n\t\t\t</dcsset:dataParameters>\n",
        );
        let xml = edit_field_test_xml(
            "dcs-edit-order-after-data-parameters",
            &source,
            "add-order",
            "Amount desc",
        );

        assert_eq!(
            test_settings_child_names(&xml),
            ["selection", "filter", "dataParameters", "order", "item"]
        );
    }

    #[test]
    fn dcs_edit_creates_conditional_appearance_before_output_parameters() {
        let source = base_dcs_xml().replace(
            "\t\t\t<dcsset:item xsi:type=\"dcsset:StructureItemGroup\">",
            "\t\t\t<dcsset:outputParameters>\n\t\t\t\t<dcscor:item xsi:type=\"dcsset:SettingsParameterValue\">\n\t\t\t\t\t<dcscor:parameter>Title</dcscor:parameter>\n\t\t\t\t</dcscor:item>\n\t\t\t</dcsset:outputParameters>\n\t\t\t<dcsset:item xsi:type=\"dcsset:StructureItemGroup\">",
        );
        let xml = edit_field_test_xml(
            "dcs-edit-conditional-before-output",
            &source,
            "add-conditionalAppearance",
            "TextColor = web:Red",
        );

        assert_eq!(
            test_settings_child_names(&xml),
            [
                "selection",
                "filter",
                "order",
                "conditionalAppearance",
                "outputParameters",
                "item",
            ]
        );
    }

    #[test]
    fn dcs_edit_set_structure_keeps_items_after_settings_containers() {
        let xml = edit_field_test_xml(
            "dcs-edit-set-structure-order",
            base_dcs_xml(),
            "set-structure",
            "Amount @name=Main > details",
        );

        assert_eq!(
            test_settings_child_names(&xml),
            ["selection", "filter", "order", "item"]
        );
    }

    #[test]
    fn dcs_edit_details_group_omits_platform_default_group_items() {
        let mut xml = edit_field_test_xml(
            "dcs-edit-details-group-canonical",
            base_dcs_xml(),
            "set-structure",
            "Amount @name=Main > Quantity @name=Details",
        );

        assert!(dcs_edit_replace_named_group_items(&mut xml, "", "Details", &[]).unwrap());

        let document = Document::parse(&xml).unwrap();
        let details = document
            .descendants()
            .filter(|node| role_info_element(*node, "item", Some(TEST_DCS_SETTINGS_NS)))
            .find(|node| {
                dcs_child(*node, "name", TEST_DCS_SETTINGS_NS)
                    .is_some_and(|name| dcs_text_of(name) == "Details")
            })
            .unwrap();
        assert!(
            dcs_child(details, "groupItems", TEST_DCS_SETTINGS_NS).is_none(),
            "8.3.27 removes an empty groupItems container from a details group"
        );

        let details_fragment =
            dcs_edit_structure_item_fragment(&dcs_edit_parse_structure("details")[0], "\t");
        assert!(!details_fragment.contains("<dcsset:groupItems"));
    }

    #[test]
    fn dcs_edit_root_additions_precede_nested_schema() {
        let source = base_dcs_xml().replace(
            "\t<settingsVariant>",
            "\t<nestedSchema/>\n\t<settingsVariant>",
        );
        for (operation, value, expected) in [
            ("add-calculated-field", "Extra = 1", "calculatedField"),
            ("add-total", "Amount: Sum(Amount)", "totalField"),
            ("add-parameter", "Extra: string = x", "parameter"),
            (
                "add-dataSetLink",
                "НаборДанных1 > НаборДанных1 on Amount = Amount",
                "dataSetLink",
            ),
            ("add-dataSet", "Extra: SELECT 1", "dataSet"),
        ] {
            let xml = edit_field_test_xml(
                &format!("dcs-edit-root-order-{operation}"),
                &source,
                operation,
                value,
            );
            let names = test_root_child_names(&xml);
            let added = names.iter().position(|name| name == expected).unwrap();
            let nested = names
                .iter()
                .position(|name| name == "nestedSchema")
                .unwrap();
            assert!(added < nested, "{operation}: {names:?}\n{xml}");
        }
    }

    #[test]
    fn dcs_edit_add_drilldown_inserts_parameter_before_templates() {
        let source = base_dcs_xml().replace(
            "\t<settingsVariant>",
            "\t<template>\n\t\t<name>Named</name>\n\t\t<template>Amount</template>\n\t</template>\n\t<settingsVariant>",
        );
        let xml = edit_field_test_xml(
            "dcs-edit-drilldown-root-order",
            &source,
            "add-drilldown",
            "Amount",
        );
        let names = test_root_child_names(&xml);
        let parameter = names.iter().position(|name| name == "parameter").unwrap();
        let template = names.iter().position(|name| name == "template").unwrap();

        assert!(parameter < template, "{names:?}\n{xml}");
    }

    #[test]
    fn dcs_edit_field_restrictions_and_roles_are_canonical_and_unique() {
        let xml = edit_field_test_xml(
            "dcs-edit-field-contract-order",
            base_dcs_xml(),
            "add-field",
            "ContractField @required @dimension @dimension @period #noOrder #noField #noField",
        );
        let document = Document::parse(&xml).unwrap();
        let data_set = test_data_set(document.root_element(), "НаборДанных1");
        let field = test_data_set_field(data_set, "ContractField");
        let restriction = dcs_child(field, "useRestriction", DCS_SCHEMA_NS).unwrap();
        let role = dcs_child(field, "role", DCS_SCHEMA_NS).unwrap();

        assert_eq!(test_direct_child_names(restriction), ["field", "order"]);
        assert_eq!(
            test_direct_child_names(role),
            ["periodNumber", "periodType", "dimension", "required"]
        );
    }

    #[test]
    fn dcs_edit_rejects_unknown_field_role_without_write() {
        for (case, operation, value) in [
            ("add", "add-field", "Broken @autoOrder"),
            ("set", "set-field-role", "Amount @autoOrder"),
        ] {
            let context = temp_context(&format!("dcs-edit-invalid-role-{case}-no-write"));
            let template_path = context.cwd.join("Template.xml");
            let original = base_dcs_xml().as_bytes().to_vec();
            fs::write(&template_path, &original).unwrap();
            let args = Map::from_iter([
                ("TemplatePath".to_string(), json!("Template.xml")),
                ("Operation".to_string(), json!(operation)),
                ("Value".to_string(), json!(value)),
            ]);

            let outcome = edit_dcs(&args, &context);

            assert!(!outcome.ok, "{operation}: {outcome:?}");
            assert!(
                outcome
                    .errors
                    .iter()
                    .any(|error| error.contains("autoOrder") && error.contains("8.3.27")),
                "{operation}: {outcome:?}"
            );
            assert_eq!(fs::read(&template_path).unwrap(), original);
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn dcs_edit_set_field_role_is_canonical_unique_and_allows_overrides() {
        let xml = edit_field_test_xml(
            "dcs-edit-set-field-role-contract",
            base_dcs_xml(),
            "set-field-role",
            "Amount @required @period @dimension @dimension periodNumber=2 periodType=Additional balanceType=OpeningBalance",
        );
        let document = Document::parse(&xml).unwrap();
        let field = test_data_set_field(
            test_data_set(document.root_element(), "НаборДанных1"),
            "Amount",
        );
        let role = dcs_child(field, "role", DCS_SCHEMA_NS).unwrap();

        assert_eq!(
            test_direct_child_names(role),
            [
                "periodNumber",
                "periodType",
                "dimension",
                "balanceType",
                "required",
            ]
        );
        assert_eq!(
            dcs_child(role, "periodNumber", TEST_DCS_COMMON_NS)
                .map(dcs_text_of)
                .as_deref(),
            Some("2")
        );
        assert_eq!(
            dcs_child(role, "periodType", TEST_DCS_COMMON_NS)
                .map(dcs_text_of)
                .as_deref(),
            Some("Additional")
        );
    }

    #[test]
    fn dcs_edit_modify_field_uses_full_dataset_field_sequence() {
        let source = base_dcs_xml()
            .replace("DataSetQuery", "DataSetObject")
            .replace(
                "\t\t\t<field>Amount</field>",
                "\t\t\t<field>Amount</field>\n\t\t\t<attributeUseRestriction><condition>true</condition></attributeUseRestriction>\n\t\t\t<presentationExpression>Amount</presentationExpression>\n\t\t\t<appearance/>\n\t\t\t<availableValue/>\n\t\t\t<inputParameters/>",
            )
            .replace(
                "\t\t<query>ВЫБРАТЬ Amount КАК Amount</query>",
                "\t\t<objectName>Catalog.Items</objectName>",
            );
        let xml = edit_field_test_xml(
            "dcs-edit-rich-field-order",
            &source,
            "modify-field",
            "Amount [Amount title]: string @dimension #noField",
        );
        let document = Document::parse(&xml).unwrap();
        let field = test_data_set_field(
            test_data_set(document.root_element(), "НаборДанных1"),
            "Amount",
        );

        assert_eq!(
            test_direct_child_names(field),
            [
                "dataPath",
                "field",
                "title",
                "useRestriction",
                "attributeUseRestriction",
                "role",
                "presentationExpression",
                "valueType",
                "appearance",
                "availableValue",
                "inputParameters",
            ]
        );
    }

    #[test]
    fn dcs_edit_modify_filter_uses_full_filter_item_sequence() {
        let source = base_dcs_xml().replace(
            "\t\t\t<dcsset:filter>\n\t\t\t</dcsset:filter>",
            "\t\t\t<dcsset:filter>\n\t\t\t\t<dcsset:item xsi:type=\"dcsset:FilterItemComparison\">\n\t\t\t\t\t<dcsset:left xsi:type=\"dcscor:Field\">Amount</dcsset:left>\n\t\t\t\t\t<dcsset:comparisonType>Equal</dcsset:comparisonType>\n\t\t\t\t\t<dcsset:presentation/>\n\t\t\t\t\t<dcsset:application>Items</dcsset:application>\n\t\t\t\t\t<dcsset:userSettingPresentation/>\n\t\t\t\t</dcsset:item>\n\t\t\t</dcsset:filter>",
        );
        let xml = edit_field_test_xml(
            "dcs-edit-rich-filter-order",
            &source,
            "modify-filter",
            "Amount >= 1 @quickAccess @user",
        );
        let document = Document::parse(&xml).unwrap();
        let filter = document
            .descendants()
            .find(|node| role_info_element(*node, "filter", Some(TEST_DCS_SETTINGS_NS)))
            .unwrap();
        let item = dcs_child(filter, "item", TEST_DCS_SETTINGS_NS).unwrap();

        assert_eq!(
            test_direct_child_names(item),
            [
                "left",
                "comparisonType",
                "right",
                "presentation",
                "application",
                "viewMode",
                "userSettingID",
                "userSettingPresentation",
            ]
        );
    }

    #[test]
    fn dcs_edit_modify_data_parameter_uses_full_settings_parameter_sequence() {
        let source = base_dcs_xml().replace(
            "\t\t\t<dcsset:order>",
            "\t\t\t<dcsset:dataParameters>\n\t\t\t\t<dcscor:item xsi:type=\"dcsset:SettingsParameterValue\">\n\t\t\t\t\t<dcscor:parameter>Period</dcscor:parameter>\n\t\t\t\t\t<dcsset:userSettingPresentation/>\n\t\t\t\t</dcscor:item>\n\t\t\t</dcsset:dataParameters>\n\t\t\t<dcsset:order>",
        );
        let xml = edit_field_test_xml(
            "dcs-edit-rich-data-parameter-order",
            &source,
            "modify-dataParameter",
            "Period = Today @quickAccess @user",
        );
        let document = Document::parse(&xml).unwrap();
        let data_parameters = document
            .descendants()
            .find(|node| role_info_element(*node, "dataParameters", Some(TEST_DCS_SETTINGS_NS)))
            .unwrap();
        let item = dcs_child(data_parameters, "item", TEST_DCS_CORE_NS).unwrap();

        assert_eq!(
            test_direct_child_names(item),
            [
                "parameter",
                "value",
                "viewMode",
                "userSettingID",
                "userSettingPresentation",
            ]
        );
    }

    #[test]
    fn dcs_edit_reuses_existing_settings_container_independent_of_whitespace() {
        let source = base_dcs_xml()
            .replace(
                "\t\t\t<dcsset:filter>\n\t\t\t</dcsset:filter>",
                "\t\t\t<dcsset:filter>\n\t\t\t\t<dcsset:item xsi:type=\"dcsset:FilterItemComparison\">\n\t\t\t\t\t<dcsset:left xsi:type=\"dcscor:Field\">Amount</dcsset:left>\n\t\t\t\t\t<dcsset:comparisonType>Greater</dcsset:comparisonType>\n\t\t\t\t\t<dcsset:right xsi:type=\"xs:decimal\">0</dcsset:right>\n\t\t\t\t</dcsset:item>\n\t\t\t</dcsset:filter>",
            )
            .replace('\t', "  ");
        let xml = edit_field_test_xml(
            "dcs-edit-existing-container-spaces",
            &source,
            "add-filter",
            "Amount < 100",
        );
        let document = Document::parse(&xml).unwrap();
        let settings = document
            .descendants()
            .find(|node| role_info_element(*node, "settings", Some(TEST_DCS_SETTINGS_NS)))
            .unwrap();
        let filters = dcs_children(settings, "filter", TEST_DCS_SETTINGS_NS);

        assert_eq!(filters.len(), 1, "duplicate singleton filter:\n{xml}");
        assert_eq!(
            dcs_children(filters[0], "item", TEST_DCS_SETTINGS_NS).len(),
            2,
            "new filter item was not appended to the existing container:\n{xml}"
        );
    }

    #[test]
    fn dcs_edit_union_operations_are_scoped_to_direct_dataset_children() {
        let source = nested_union_dcs_xml().replace(
            "\t\t\t\t<dataPath>Value</dataPath>\n\t\t\t\t<field>Value</field>\n\t\t\t\t<valueType>",
            "\t\t\t\t<dataPath>InnerOnly</dataPath>\n\t\t\t\t<field>InnerOnly</field>\n\t\t\t\t<valueType>",
        );

        let unchanged = edit_field_test_xml_for_dataset(
            "dcs-edit-union-direct-modify",
            &source,
            "Union",
            "modify-field",
            "InnerOnly [must stay nested]",
        );
        assert!(
            !unchanged.contains("<v8:content>must stay nested</v8:content>"),
            "outer union operation modified a nested item field:\n{unchanged}"
        );

        let with_outer = edit_field_test_xml_for_dataset(
            "dcs-edit-union-direct-add",
            &source,
            "Union",
            "add-field",
            "InnerOnly:String",
        );
        let document = Document::parse(&with_outer).unwrap();
        let union = test_data_set(document.root_element(), "Union");
        assert!(
            dcs_children(union, "field", DCS_SCHEMA_NS)
                .into_iter()
                .any(|field| dcs_child(field, "dataPath", DCS_SCHEMA_NS)
                    .is_some_and(|node| dcs_text_of(node) == "InnerOnly")),
            "nested dataPath incorrectly blocked a direct union field:\n{with_outer}"
        );

        let context = temp_context("dcs-edit-union-direct-query");
        let template_path = context.cwd.join("Template.xml");
        fs::write(&template_path, &source).unwrap();
        let before = fs::read(&template_path).unwrap();
        let args = Map::from_iter([
            ("TemplatePath".to_string(), json!("Template.xml")),
            ("Operation".to_string(), json!("set-query")),
            ("DataSet".to_string(), json!("Union")),
            ("Value".to_string(), json!("SELECT 2")),
        ]);
        let outcome = edit_dcs(&args, &context);
        assert!(!outcome.ok, "union has no direct query: {outcome:?}");
        assert_eq!(fs::read(&template_path).unwrap(), before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn dcs_edit_rejects_invalid_typed_literals_without_write() {
        for (case, operation, value) in [
            ("filter-decimal", "add-filter", "Amount = 1.2.3"),
            (
                "data-parameter-date",
                "add-dataParameter",
                "Period = 2024-99-99T00:00:00",
            ),
            (
                "parameter-decimal",
                "add-parameter",
                "Bad:decimal(10,2)=abc",
            ),
            (
                "standard-period",
                "add-parameter",
                "BadPeriod:StandardPeriod=NotAStandardPeriod",
            ),
        ] {
            let context = temp_context(&format!("dcs-edit-invalid-literal-{case}"));
            let template_path = context.cwd.join("Template.xml");
            fs::write(&template_path, base_dcs_xml()).unwrap();
            let before = fs::read(&template_path).unwrap();
            let args = Map::from_iter([
                ("TemplatePath".to_string(), json!("Template.xml")),
                ("Operation".to_string(), json!(operation)),
                ("Value".to_string(), json!(value)),
            ]);

            let outcome = edit_dcs(&args, &context);

            assert!(!outcome.ok, "{case}: {outcome:?}");
            assert_eq!(fs::read(&template_path).unwrap(), before, "{case}");
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn dcs_edit_modify_parameter_rejects_unknown_children_without_write() {
        let context = temp_context("dcs-edit-modify-parameter-whitelist");
        let template_path = context.cwd.join("Template.xml");
        let source = dcs_with_contract_parameter(false);
        fs::write(&template_path, &source).unwrap();
        let before = fs::read(&template_path).unwrap();
        let args = Map::from_iter([
            ("TemplatePath".to_string(), json!("Template.xml")),
            ("Operation".to_string(), json!("modify-parameter")),
            ("Value".to_string(), json!("P foo=bar")),
        ]);

        let outcome = edit_dcs(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("foo") && error.contains("8.3.27")),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&template_path).unwrap(), before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn dcs_edit_modify_parameter_uses_canonical_8_3_27_sequence() {
        let xml = edit_field_test_xml(
            "dcs-edit-modify-parameter-order",
            &dcs_with_contract_parameter(false),
            "modify-parameter",
            "P value=A,B @hidden @always",
        );
        let document = Document::parse(&xml).unwrap();
        let parameter = dcs_child(document.root_element(), "parameter", DCS_SCHEMA_NS).unwrap();

        assert_eq!(
            test_direct_child_names(parameter),
            [
                "name",
                "valueType",
                "value",
                "value",
                "useRestriction",
                "expression",
                "availableValue",
                "valueListAllowed",
                "availableAsField",
                "denyIncompleteValues",
                "use",
            ]
        );
    }

    #[test]
    fn dcs_edit_parameter_selector_does_not_match_parameter_list_allowed() {
        let xml = edit_field_test_xml(
            "dcs-edit-parameter-qname-boundary",
            &dcs_with_contract_parameter(true),
            "modify-parameter",
            "P use=Always",
        );
        let document = Document::parse(&xml).unwrap();
        let parameter = dcs_child(document.root_element(), "parameter", DCS_SCHEMA_NS).unwrap();

        assert_eq!(
            dcs_child(parameter, "use", DCS_SCHEMA_NS)
                .map(dcs_text_of)
                .as_deref(),
            Some("Always")
        );
    }

    #[test]
    fn dcs_edit_variant_exists_ignores_names_of_other_root_items() {
        assert!(dcs_edit_variant_exists(base_dcs_xml(), "Основной"));
        assert!(
            !dcs_edit_variant_exists(base_dcs_xml(), "НаборДанных1"),
            "a dataSet name must not be mistaken for a settingsVariant name"
        );
    }

    #[test]
    fn dcs_edit_modify_structure_does_not_reuse_nested_group_items() {
        let mut xml = edit_field_test_xml(
            "dcs-edit-structure-direct-group-items-source",
            base_dcs_xml(),
            "set-structure",
            "Amount @name=Parent > Quantity @name=Child",
        );
        let parent_range = dcs_edit_find_named_structure_group(&xml, "", "Parent")
            .unwrap()
            .unwrap();
        assert!(dcs_edit_remove_child_element(
            &mut xml,
            parent_range,
            "dcsset:groupItems"
        ));

        assert!(
            dcs_edit_replace_named_group_items(&mut xml, "", "Parent", &["Price".to_string()])
                .unwrap()
        );

        let document = Document::parse(&xml).unwrap();
        let group = |name: &str| {
            document
                .descendants()
                .filter(|node| role_info_element(*node, "item", Some(TEST_DCS_SETTINGS_NS)))
                .find(|node| {
                    dcs_child(*node, "name", TEST_DCS_SETTINGS_NS)
                        .is_some_and(|candidate| dcs_text_of(candidate) == name)
                })
                .unwrap()
        };
        let group_item_field = |name: &str| {
            let group_items = dcs_child(group(name), "groupItems", TEST_DCS_SETTINGS_NS).unwrap();
            let item = dcs_child(group_items, "item", TEST_DCS_SETTINGS_NS).unwrap();
            dcs_child(item, "field", TEST_DCS_SETTINGS_NS)
                .map(dcs_text_of)
                .unwrap()
        };

        assert_eq!(group_item_field("Parent"), "Price");
        assert_eq!(group_item_field("Child"), "Quantity");
    }

    #[test]
    fn dcs_edit_remove_selection_field_removes_the_nested_item_not_its_folder() {
        let mut xml = base_dcs_xml().replace(
            "\t\t\t<dcsset:selection>\n\t\t\t</dcsset:selection>",
            "\t\t\t<dcsset:selection>\n\t\t\t\t<dcsset:item xsi:type=\"dcsset:SelectedItemFolder\">\n\t\t\t\t\t<dcsset:item xsi:type=\"dcsset:SelectedItemField\">\n\t\t\t\t\t\t<dcsset:field>Amount</dcsset:field>\n\t\t\t\t\t</dcsset:item>\n\t\t\t\t\t<dcsset:placement>Auto</dcsset:placement>\n\t\t\t\t</dcsset:item>\n\t\t\t</dcsset:selection>",
        );

        assert!(dcs_edit_remove_prefixed_selection_field(&mut xml, "", "Amount").unwrap());

        let document = Document::parse(&xml).unwrap();
        let selection = document
            .descendants()
            .find(|node| role_info_element(*node, "selection", Some(TEST_DCS_SETTINGS_NS)))
            .unwrap();
        let folder = dcs_child(selection, "item", TEST_DCS_SETTINGS_NS)
            .expect("the containing selection folder must be preserved");
        assert!(dcs_child(folder, "placement", TEST_DCS_SETTINGS_NS).is_some());
        assert!(folder
            .descendants()
            .filter(|node| role_info_element(*node, "field", Some(TEST_DCS_SETTINGS_NS)))
            .all(|node| dcs_text_of(node) != "Amount"));
    }

    fn dcs_with_contract_parameter(with_link: bool) -> String {
        let link = if with_link {
            "\t<dataSetLink>\n\t\t<sourceDataSet>НаборДанных1</sourceDataSet>\n\t\t<destinationDataSet>НаборДанных1</destinationDataSet>\n\t\t<sourceExpression>Amount</sourceExpression>\n\t\t<destinationExpression>Amount</destinationExpression>\n\t\t<parameter>P</parameter>\n\t\t<parameterListAllowed>true</parameterListAllowed>\n\t</dataSetLink>\n"
        } else {
            ""
        };
        let parameter = "\t<parameter>\n\t\t<name>P</name>\n\t\t<valueType>\n\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t<v8:StringQualifiers>\n\t\t\t\t<v8:Length>0</v8:Length>\n\t\t\t\t<v8:AllowedLength>Variable</v8:AllowedLength>\n\t\t\t</v8:StringQualifiers>\n\t\t</valueType>\n\t\t<expression>&amp;Source.P</expression>\n\t\t<availableValue>\n\t\t\t<value xsi:type=\"xs:string\">A</value>\n\t\t</availableValue>\n\t\t<denyIncompleteValues>true</denyIncompleteValues>\n\t\t<use>Auto</use>\n\t</parameter>\n";
        base_dcs_xml().replace(
            "\t<settingsVariant>",
            &format!("{link}{parameter}\t<settingsVariant>"),
        )
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

    fn edit_field_test_xml(name: &str, source: &str, operation: &str, value: &str) -> String {
        edit_field_test_xml_for_dataset(name, source, "НаборДанных1", operation, value)
    }

    fn edit_field_test_xml_for_dataset(
        name: &str,
        source: &str,
        data_set: &str,
        operation: &str,
        value: &str,
    ) -> String {
        let context = temp_context(name);
        let template_path = context.cwd.join("Template.xml");
        fs::write(&template_path, source).unwrap();
        let args = Map::from_iter([
            ("TemplatePath".to_string(), json!("Template.xml")),
            ("Operation".to_string(), json!(operation)),
            ("Value".to_string(), json!(value)),
            ("DataSet".to_string(), json!(data_set)),
            ("NoSelection".to_string(), json!(true)),
        ]);

        let outcome = edit_dcs(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        let xml = fs::read_to_string(&template_path)
            .unwrap()
            .trim_start_matches('\u{feff}')
            .to_string();
        let _ = fs::remove_dir_all(&context.cwd);
        xml
    }

    fn nested_union_dcs_xml() -> String {
        base_dcs_xml().replace(
            r#"	<dataSet xsi:type="DataSetQuery">
		<name>НаборДанных1</name>
		<field xsi:type="DataSetFieldField">
			<dataPath>Amount</dataPath>
			<field>Amount</field>
		</field>
		<dataSource>ИсточникДанных1</dataSource>
		<query>ВЫБРАТЬ Amount КАК Amount</query>
	</dataSet>"#,
            r#"	<dataSet xsi:type="DataSetUnion">
		<name>Union</name>
		<field xsi:type="DataSetFieldField">
			<dataPath>Value</dataPath>
			<field>Value</field>
		</field>
		<item xsi:type="DataSetQuery">
			<name>InnerQuery</name>
			<field xsi:type="DataSetFieldField">
				<dataPath>Value</dataPath>
				<field>Value</field>
				<valueType>
					<v8:Type>xs:string</v8:Type>
				</valueType>
			</field>
			<dataSource>ИсточникДанных1</dataSource>
			<query>ВЫБРАТЬ 1 КАК Value</query>
		</item>
	</dataSet>"#,
        )
    }

    fn test_data_set<'a, 'input>(
        root: roxmltree::Node<'a, 'input>,
        name: &str,
    ) -> roxmltree::Node<'a, 'input> {
        root.children()
            .filter(|child| {
                role_info_element(*child, "dataSet", Some(DCS_SCHEMA_NS))
                    || role_info_element(*child, "item", Some(DCS_SCHEMA_NS))
            })
            .find(|data_set| {
                dcs_child(*data_set, "name", DCS_SCHEMA_NS)
                    .is_some_and(|node| dcs_text_of(node) == name)
            })
            .unwrap_or_else(|| panic!("dataSet {name} not found"))
    }

    fn test_data_set_field<'a, 'input>(
        data_set: roxmltree::Node<'a, 'input>,
        data_path: &str,
    ) -> roxmltree::Node<'a, 'input> {
        dcs_children(data_set, "field", DCS_SCHEMA_NS)
            .into_iter()
            .find(|field| {
                dcs_child(*field, "dataPath", DCS_SCHEMA_NS)
                    .is_some_and(|node| dcs_text_of(node) == data_path)
            })
            .unwrap_or_else(|| panic!("field {data_path} not found"))
    }

    fn test_direct_child_names(node: roxmltree::Node<'_, '_>) -> Vec<String> {
        node.children()
            .filter(roxmltree::Node::is_element)
            .map(|child| child.tag_name().name().to_string())
            .collect()
    }

    fn test_settings_child_names(xml_text: &str) -> Vec<String> {
        let document = Document::parse(xml_text).unwrap();
        let settings = document
            .descendants()
            .find(|node| role_info_element(*node, "settings", Some(TEST_DCS_SETTINGS_NS)))
            .unwrap();
        test_direct_child_names(settings)
    }

    fn test_root_child_names(xml_text: &str) -> Vec<String> {
        let document = Document::parse(xml_text).unwrap();
        test_direct_child_names(document.root_element())
    }

    fn two_variant_dcs_xml() -> String {
        base_dcs_xml().replace(
            "</settingsVariant>\n</DataCompositionSchema>",
            "</settingsVariant>\n\t<settingsVariant>\n\t\t<dcsset:name>Дополнительный</dcsset:name>\n\t\t<dcsset:settings>\n\t\t\t<dcsset:selection>\n\t\t\t</dcsset:selection>\n\t\t</dcsset:settings>\n\t</settingsVariant>\n</DataCompositionSchema>",
        )
    }

    fn parameter_dcs_xml() -> String {
        base_dcs_xml().replace(
            "\t<settingsVariant>",
            "\t<parameter>\n\t\t<name>Период</name>\n\t\t<expression>&amp;Период</expression>\n\t</parameter>\n\t<parameter>\n\t\t<name>ПериодОтчетаДокумента</name>\n\t\t<expression>&amp;ПериодОтчетаДокумента</expression>\n\t</parameter>\n\t<settingsVariant>",
        )
        .replace(
            "\t\t\t<dcsset:order>",
            "\t\t\t<dcsset:dataParameters>\n\t\t\t\t<dcscor:item xsi:type=\"dcsset:SettingsParameterValue\">\n\t\t\t\t\t<dcscor:parameter>Период</dcscor:parameter>\n\t\t\t\t</dcscor:item>\n\t\t\t\t<dcscor:item xsi:type=\"dcsset:SettingsParameterValue\">\n\t\t\t\t\t<dcscor:parameter>ПериодОтчетаДокумента</dcscor:parameter>\n\t\t\t\t</dcscor:item>\n\t\t\t</dcsset:dataParameters>\n\t\t\t<dcsset:order>",
        )
    }

    fn variant_block(xml_text: &str, name: &str) -> String {
        let marker = format!("<dcsset:name>{name}</dcsset:name>");
        let name_pos = xml_text
            .find(&marker)
            .unwrap_or_else(|| panic!("{name} not found"));
        let start = xml_text[..name_pos].rfind("<settingsVariant>").unwrap();
        let end = xml_text[name_pos..].find("</settingsVariant>").unwrap()
            + name_pos
            + "</settingsVariant>".len();
        xml_text[start..end].to_string()
    }
}
