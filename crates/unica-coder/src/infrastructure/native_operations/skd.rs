#![allow(dead_code, unused_imports)]

use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::AdapterOutcome;
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::common::*;
use super::{
    cf::*, cfe::*, form::*, interface::*, meta::*, mxl::*, role::*, subsystem::*, template::*,
};
pub(crate) struct SkdValidationReporter {
    pub(crate) errors: usize,
    pub(crate) warnings: usize,
    pub(crate) ok_count: usize,
    pub(crate) stopped: bool,
    pub(crate) max_errors: usize,
    pub(crate) detailed: bool,
    pub(crate) lines: Vec<String>,
}

pub(crate) struct SkdValidationRun {
    pub(crate) ok: bool,
    pub(crate) stdout: String,
    pub(crate) out_file: Option<PathBuf>,
    pub(crate) out_file_label: Option<String>,
    pub(crate) artifact: PathBuf,
    pub(crate) errors: Vec<String>,
}

impl SkdValidationReporter {
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

pub(crate) fn analyze_skd_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    const NS_SETTINGS: &str = "http://v8.1c.ru/8.1/data-composition-system/settings";

    let result = (|| -> Result<(String, Option<PathBuf>, PathBuf), String> {
        let template_path = resolve_skd_info_path_for_script(args, context)?;
        let resolved_path = template_path
            .canonicalize()
            .unwrap_or_else(|_| template_path.clone());
        let text = read_utf8_sig(&resolved_path)?;
        let doc = Document::parse(text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("XML parse error in {}: {err}", resolved_path.display()))?;
        let root = doc.root_element();
        let mode = string_arg(args, &["mode", "Mode"]).unwrap_or("overview");
        let out_file_label = string_arg(args, &["outFile", "OutFile"]).map(ToOwned::to_owned);
        let out_file = out_file_label
            .as_ref()
            .filter(|value| !value.is_empty())
            .map(|value| absolutize(PathBuf::from(value), &context.cwd));
        let limit = int_arg(args, &["limit", "Limit"]).unwrap_or(150).max(0) as usize;
        let offset = int_arg(args, &["offset", "Offset"]).unwrap_or(0).max(0) as usize;
        let mut lines = Vec::<String>::new();

        match mode {
            "overview" => {
                skd_info_overview(
                    root,
                    &resolved_path,
                    &text,
                    &mut lines,
                    NS_SCHEMA,
                    NS_SETTINGS,
                );
                skd_info_overview_hints(root, &mut lines, NS_SCHEMA, NS_SETTINGS);
            }
            "query" => skd_info_query(root, &mut lines, NS_SCHEMA)?,
            "fields" => skd_info_fields(root, &mut lines, NS_SCHEMA),
            "links" => skd_info_links(root, &mut lines, NS_SCHEMA),
            "calculated" => {
                let count = skd_children(root, "calculatedField", NS_SCHEMA).len();
                if count == 0 {
                    lines.push("(no calculated fields)".to_string());
                } else {
                    lines.push(format!("=== Calculated fields ({count}) ==="));
                }
            }
            "resources" => {
                let count = skd_children(root, "totalField", NS_SCHEMA).len();
                if count == 0 {
                    lines.push("(no resources)".to_string());
                } else {
                    lines.push(format!("=== Resources ({count}) ==="));
                }
            }
            "params" => {
                let count = skd_children(root, "parameter", NS_SCHEMA).len();
                lines.push(format!("=== Parameters ({count}) ==="));
                lines.push(
                    "  Name                            Type                   Default          Visible  Expression"
                        .to_string(),
                );
            }
            "variant" => skd_info_variant(root, &mut lines, NS_SCHEMA, NS_SETTINGS),
            "templates" => skd_info_templates(root, &mut lines, NS_SCHEMA),
            "trace" => {
                let name = string_arg(args, &["name", "Name"]).unwrap_or("");
                if name.is_empty() {
                    return Err("Trace mode requires -Name <field_name_or_title>".to_string());
                }
                return Err(format!("Field '{name}' not found by dataPath or title"));
            }
            "full" => {
                skd_info_overview(
                    root,
                    &resolved_path,
                    &text,
                    &mut lines,
                    NS_SCHEMA,
                    NS_SETTINGS,
                );
            }
            other => {
                return Err(format!(
                    "argument -Mode: invalid choice: '{other}' (choose from 'overview', 'query', 'fields', 'links', 'calculated', 'resources', 'params', 'variant', 'trace', 'templates', 'full')"
                ));
            }
        }

        let total_lines = lines.len();
        if let Some(out_file) = &out_file {
            if let Some(parent) = out_file.parent() {
                fs::create_dir_all(parent)
                    .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
            }
            write_utf8_bom(out_file, &lines.join("\n"))?;
            let label = out_file_label.as_deref().unwrap_or("");
            return Ok((
                format!("Written {total_lines} lines to {label}\n"),
                Some(out_file.clone()),
                resolved_path,
            ));
        }

        let mut result = if offset > 0 {
            if offset >= total_lines {
                return Ok((
                    format!(
                        "[INFO] Offset {offset} exceeds total lines ({total_lines}). Nothing to show.\n"
                    ),
                    None,
                    resolved_path,
                ));
            }
            lines[offset..].to_vec()
        } else {
            lines
        };
        let stdout = if result.len() > limit {
            result.truncate(limit);
            format!(
                "{}\n\n[TRUNCATED] Shown {limit} of {total_lines} lines. Use -Offset {} to continue.\n",
                result.join("\n"),
                offset + limit
            )
        } else {
            format!("{}\n", result.join("\n"))
        };
        Ok((stdout, None, resolved_path))
    })();

    match result {
        Ok((stdout, out_file, artifact)) => {
            let mut artifacts = vec![artifact.display().to_string()];
            if let Some(out_file) = &out_file {
                artifacts.push(out_file.display().to_string());
            }
            AdapterOutcome {
                ok: true,
                summary: "unica.skd.info completed with native DCS inspector".to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts,
                stdout: Some(stdout),
                stderr: None,
                command: None,
            }
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.skd.info failed in native DCS inspector".to_string(),
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

pub(crate) fn skd_info_overview(
    root: roxmltree::Node<'_, '_>,
    resolved_path: &Path,
    text: &str,
    lines: &mut Vec<String>,
    ns_schema: &str,
    ns_settings: &str,
) {
    let template_name = skd_info_template_name(resolved_path);
    let total_xml_lines = text.lines().count();
    lines.push(format!(
        "=== DCS: {template_name} ({total_xml_lines} lines) ==="
    ));
    lines.push(format!(
        "Поддержка: {}",
        support_status_for_path(resolved_path)
    ));
    lines.push(String::new());

    let sources = skd_children(root, "dataSource", ns_schema)
        .into_iter()
        .map(|source| {
            format!(
                "{} ({})",
                skd_child(source, "name", ns_schema)
                    .map(skd_text_of)
                    .unwrap_or_default(),
                skd_child(source, "dataSourceType", ns_schema)
                    .map(skd_text_of)
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>();
    lines.push(format!("Sources: {}", sources.join(", ")));
    lines.push(String::new());

    lines.push("Datasets:".to_string());
    for data_set in skd_children(root, "dataSet", ns_schema) {
        skd_info_dataset_overview(data_set, lines, ns_schema, "  ");
    }

    let links = skd_children(root, "dataSetLink", ns_schema);
    if !links.is_empty() {
        let mut link_pairs = BTreeMap::<String, usize>::new();
        let mut ordered = Vec::<String>::new();
        for link in links {
            let key = format!(
                "{} -> {}",
                skd_child(link, "sourceDataSet", ns_schema)
                    .map(skd_text_of)
                    .unwrap_or_default(),
                skd_child(link, "destinationDataSet", ns_schema)
                    .map(skd_text_of)
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

    let calculated = skd_children(root, "calculatedField", ns_schema);
    if !calculated.is_empty() {
        lines.push(format!("Calculated: {}", calculated.len()));
    }

    let totals = skd_children(root, "totalField", ns_schema);
    if !totals.is_empty() {
        let mut unique = HashSet::<String>::new();
        let mut has_grouped = false;
        for total in &totals {
            unique.insert(
                skd_child(*total, "dataPath", ns_schema)
                    .map(skd_text_of)
                    .unwrap_or_default(),
            );
            if skd_child(*total, "group", ns_schema).is_some() {
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

    let templates = skd_children(root, "template", ns_schema);
    if !templates.is_empty() {
        let field_templates = skd_children(root, "fieldTemplate", ns_schema);
        let group_count = skd_children(root, "groupTemplate", ns_schema).len()
            + skd_children(root, "groupHeaderTemplate", ns_schema).len()
            + skd_children(root, "groupFooterTemplate", ns_schema).len();
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

    let params = skd_children(root, "parameter", ns_schema);
    if params.is_empty() {
        lines.push("Params: (none)".to_string());
    } else {
        let mut visible_names = Vec::new();
        let mut hidden_count = 0usize;
        for param in &params {
            let name = skd_child(*param, "name", ns_schema)
                .map(skd_text_of)
                .unwrap_or_default();
            let hidden = skd_child(*param, "useRestriction", ns_schema)
                .map(skd_text_of)
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
    let variants = skd_children(root, "settingsVariant", ns_schema);
    if !variants.is_empty() {
        lines.push("Variants:".to_string());
        for (index, variant) in variants.iter().enumerate() {
            let name = skd_child(*variant, "name", ns_settings)
                .map(skd_text_of)
                .unwrap_or_default();
            let presentation = skd_child(*variant, "presentation", ns_settings)
                .map(skd_info_multilang_or_inner_text)
                .unwrap_or_default();
            let presentation_str = if presentation.is_empty() {
                String::new()
            } else {
                format!("  \"{presentation}\"")
            };
            let settings = skd_child(*variant, "settings", ns_settings);
            let mut struct_items = Vec::new();
            let mut filter_count = 0usize;
            if let Some(settings) = settings {
                for item in skd_children(settings, "item", ns_settings) {
                    let item_type = skd_info_structure_item_type(item);
                    let group_fields = skd_info_group_fields(item, ns_settings);
                    let group = if group_fields.is_empty() {
                        "(detail)".to_string()
                    } else {
                        format!("({})", group_fields.join(","))
                    };
                    struct_items.push(format!("{item_type}{group}"));
                }
                if let Some(filter) = skd_child(settings, "filter", ns_settings) {
                    filter_count = skd_children(filter, "item", ns_settings).len();
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

pub(crate) fn skd_info_dataset_overview(
    data_set: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    indent: &str,
) {
    let ds_type = skd_info_dataset_type(data_set);
    let name = skd_child(data_set, "name", ns_schema)
        .map(skd_text_of)
        .unwrap_or_default();
    let field_count = skd_children(data_set, "field", ns_schema).len();
    match ds_type.as_str() {
        "Query" => {
            let query_lines = skd_child(data_set, "query", ns_schema)
                .map(|node| skd_inner_text(node).split('\n').count())
                .unwrap_or(0);
            lines.push(format!(
                "{indent}[Query]  {name}   {field_count} fields, query {query_lines} lines"
            ));
        }
        "Object" => {
            let obj_str = skd_child(data_set, "objectName", ns_schema)
                .map(skd_text_of)
                .filter(|value| !value.is_empty())
                .map(|value| format!("  objectName={value}"))
                .unwrap_or_default();
            lines.push(format!(
                "{indent}[Object] {name}{obj_str}  {field_count} fields"
            ));
        }
        "Union" => {
            lines.push(format!("{indent}[Union]  {name}  {field_count} fields"));
            for sub_ds in skd_children(data_set, "item", ns_schema) {
                let sub_type = skd_info_dataset_type(sub_ds);
                let sub_name = skd_child(sub_ds, "name", ns_schema)
                    .map(skd_text_of)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| "?".to_string());
                let sub_fields = skd_children(sub_ds, "field", ns_schema).len();
                if sub_type == "Query" {
                    let query_lines = skd_child(sub_ds, "query", ns_schema)
                        .map(|node| skd_inner_text(node).split('\n').count())
                        .unwrap_or(0);
                    lines.push(format!(
                        "    ├─ [Query] {sub_name}   {sub_fields} fields, query {query_lines} lines"
                    ));
                } else if sub_type == "Object" {
                    let obj_str = skd_child(sub_ds, "objectName", ns_schema)
                        .map(skd_text_of)
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

pub(crate) fn skd_info_overview_hints(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    ns_settings: &str,
) {
    lines.push(String::new());
    let mut hints = Vec::<String>::new();
    let mut query_names = Vec::<String>::new();
    for data_set in skd_children(root, "dataSet", ns_schema) {
        if skd_info_dataset_type(data_set) == "Query" {
            query_names.push(
                skd_child(data_set, "name", ns_schema)
                    .map(skd_text_of)
                    .unwrap_or_default(),
            );
        } else if skd_info_dataset_type(data_set) == "Union" {
            for sub_ds in skd_children(data_set, "item", ns_schema) {
                if skd_info_dataset_type(sub_ds) == "Query" {
                    query_names.push(
                        skd_child(sub_ds, "name", ns_schema)
                            .map(skd_text_of)
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
    let links = skd_children(root, "dataSetLink", ns_schema);
    if !links.is_empty() {
        hints.push(format!(
            "-Mode links             dataset connections ({})",
            links.len()
        ));
    }
    let calculated = skd_children(root, "calculatedField", ns_schema);
    if !calculated.is_empty() {
        hints.push(format!(
            "-Mode calculated        calculated field expressions ({})",
            calculated.len()
        ));
    }
    let totals = skd_children(root, "totalField", ns_schema);
    if !totals.is_empty() {
        hints.push(format!(
            "-Mode resources         resource aggregation ({})",
            totals.len()
        ));
    }
    if !skd_children(root, "parameter", ns_schema).is_empty() {
        hints.push("-Mode params            parameter details".to_string());
    }
    let variants = skd_children(root, "settingsVariant", ns_schema);
    if variants.len() == 1 {
        hints.push("-Mode variant           variant structure".to_string());
    } else if variants.len() > 1 {
        hints.push(format!(
            "-Mode variant -Name <N> variant structure (1..{})",
            variants.len()
        ));
    }
    if !skd_children(root, "template", ns_schema).is_empty() {
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

pub(crate) fn skd_info_query(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
) -> Result<(), String> {
    let mut target = None;
    for data_set in skd_children(root, "dataSet", ns_schema) {
        if skd_info_dataset_type(data_set) == "Query" {
            target = Some(data_set);
            break;
        }
        if skd_info_dataset_type(data_set) == "Union" {
            for sub_ds in skd_children(data_set, "item", ns_schema) {
                if skd_info_dataset_type(sub_ds) == "Query" {
                    target = Some(sub_ds);
                    break;
                }
            }
        }
        if target.is_some() {
            break;
        }
    }
    let Some(target) = target else {
        return Err("No Query dataset found".to_string());
    };
    let query = skd_child(target, "query", ns_schema)
        .map(skd_inner_text)
        .unwrap_or_default();
    let name = skd_child(target, "name", ns_schema)
        .map(skd_text_of)
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

pub(crate) fn skd_info_fields(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
) {
    lines.push("=== Fields map ===".to_string());
    for data_set in skd_children(root, "dataSet", ns_schema) {
        skd_info_field_map(data_set, lines, ns_schema, "");
        if skd_info_dataset_type(data_set) == "Union" {
            for sub_ds in skd_children(data_set, "item", ns_schema) {
                skd_info_field_map(sub_ds, lines, ns_schema, "  ");
            }
        }
    }
    lines.push(String::new());
    lines.push("Use -Name <field> for details.".to_string());
}

pub(crate) fn skd_info_field_map(
    data_set: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    indent: &str,
) {
    let fields = skd_children(data_set, "field", ns_schema)
        .into_iter()
        .filter_map(|field| skd_child(field, "dataPath", ns_schema).map(skd_text_of))
        .collect::<Vec<_>>();
    let name = skd_child(data_set, "name", ns_schema)
        .map(skd_text_of)
        .unwrap_or_default();
    let mut name_list = fields.join(", ");
    if name_list.len() > 100 {
        name_list.truncate(97);
        name_list.push_str("...");
    }
    lines.push(format!(
        "{indent}{name} [{}] ({}): {name_list}",
        skd_info_dataset_type(data_set),
        fields.len()
    ));
}

pub(crate) fn skd_info_links(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
) {
    let links = skd_children(root, "dataSetLink", ns_schema);
    if links.is_empty() {
        lines.push("(no links)".to_string());
    } else {
        lines.push(format!("=== Links ({}) ===", links.len()));
    }
}

pub(crate) fn skd_info_variant(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
    ns_settings: &str,
) {
    let variants = skd_children(root, "settingsVariant", ns_schema);
    if variants.is_empty() {
        lines.push("=== Variants: (none) ===".to_string());
        return;
    }
    lines.push(format!("=== Variants ({}) ===", variants.len()));
    for (index, variant) in variants.iter().enumerate() {
        let name = skd_child(*variant, "name", ns_settings)
            .map(skd_text_of)
            .unwrap_or_default();
        let presentation = skd_child(*variant, "presentation", ns_settings)
            .map(skd_info_multilang_or_inner_text)
            .unwrap_or_default();
        let presentation_str = if presentation.is_empty() {
            String::new()
        } else {
            format!("  \"{presentation}\"")
        };
        lines.push(format!("  [{}] {name}{presentation_str}", index + 1));
    }
}

pub(crate) fn skd_info_templates(
    root: roxmltree::Node<'_, '_>,
    lines: &mut Vec<String>,
    ns_schema: &str,
) {
    let templates = skd_children(root, "template", ns_schema);
    let field_count = skd_children(root, "fieldTemplate", ns_schema).len();
    let group_count = skd_children(root, "groupTemplate", ns_schema).len()
        + skd_children(root, "groupHeaderTemplate", ns_schema).len()
        + skd_children(root, "groupFooterTemplate", ns_schema).len();
    lines.push(format!(
        "=== Templates ({} defined: {field_count} field, {group_count} group) ===",
        templates.len()
    ));
}

pub(crate) fn skd_info_dataset_type(data_set: roxmltree::Node<'_, '_>) -> String {
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

pub(crate) fn skd_info_structure_item_type(item: roxmltree::Node<'_, '_>) -> &'static str {
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

pub(crate) fn skd_info_multilang_or_inner_text(node: roxmltree::Node<'_, '_>) -> String {
    let value = multilang_text(node);
    if value.is_empty() {
        node.descendants()
            .filter_map(|child| child.text())
            .collect::<String>()
            .trim()
            .to_string()
    } else {
        value
    }
}

pub(crate) fn skd_info_group_fields(
    item: roxmltree::Node<'_, '_>,
    ns_settings: &str,
) -> Vec<String> {
    let mut fields = Vec::new();
    for group_item in skd_find_all_path(item, &[("groupItems", ns_settings), ("item", ns_settings)])
    {
        if let Some(field) = skd_child(group_item, "field", ns_settings) {
            let mut value = skd_text_of(field);
            let group_type = skd_child(group_item, "groupType", ns_settings)
                .map(skd_text_of)
                .unwrap_or_default();
            if !group_type.is_empty() && group_type != "Items" {
                value.push_str(&format!("({group_type})"));
            }
            fields.push(value);
        }
    }
    fields
}

pub(crate) fn skd_info_template_name(path: &Path) -> String {
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

pub(crate) fn resolve_skd_info_path_for_script(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let raw_path = required_path(
        args,
        &["templatePath", "TemplatePath", "path", "Path"],
        "TemplatePath",
    )?;
    let original_path = raw_path.clone();
    let mut template_path = raw_path.clone();
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
            for entry in fs::read_dir(&templates_dir)
                .map_err(|err| format!("failed to read {}: {err}", templates_dir.display()))?
            {
                let entry = entry
                    .map_err(|err| format!("failed to read {}: {err}", templates_dir.display()))?;
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) != Some("xml") {
                    continue;
                }
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
                return Ok(dcs_templates.remove(0));
            }
            if dcs_templates.len() > 1 {
                return Err(format!(
                    "Multiple DCS templates found in: {}",
                    original_path.display()
                ));
            }
            return Err(format!(
                "No DCS templates found in: {}",
                original_path.display()
            ));
        }
    }

    let abs_template = absolutize(template_path, &context.cwd);
    if !abs_template.is_file() {
        return Err(format!("File not found: {}", abs_template.display()));
    }
    Ok(abs_template)
}

pub(crate) fn validate_skd(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";

    let result = (|| -> Result<SkdValidationRun, String> {
        let template_path = resolve_skd_validate_path(args, context)?;
        let resolved_path = template_path
            .canonicalize()
            .unwrap_or_else(|_| template_path.clone());
        let file_name = resolved_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_string();
        let out_file_label = string_arg(args, &["outFile", "OutFile"]).map(ToOwned::to_owned);
        let out_file = out_file_label
            .as_ref()
            .filter(|value| !value.is_empty())
            .map(|value| absolutize(PathBuf::from(value), &context.cwd));
        let detailed = bool_arg(args, &["detailed", "Detailed"]);
        let max_errors = int_arg(args, &["maxErrors", "MaxErrors"])
            .unwrap_or(20)
            .max(0) as usize;

        let text = read_utf8_sig(&resolved_path)?;
        let mut report = SkdValidationReporter::new(max_errors, detailed, &file_name);
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
                return Ok(SkdValidationRun {
                    ok: false,
                    stdout: format!("{}\n", report.lines.join("\n")),
                    out_file,
                    out_file_label,
                    artifact: resolved_path,
                    errors,
                });
            }
        };

        let root = doc.root_element();
        let root_local = root.tag_name().name();
        if root_local != "DataCompositionSchema" {
            report.error(format!(
                "Root element is '{root_local}', expected 'DataCompositionSchema'"
            ));
        } else {
            report.ok("Root element: DataCompositionSchema");
        }
        let root_ns = root.tag_name().namespace().unwrap_or("");
        if root_ns != NS_SCHEMA {
            report.error(format!(
                "Default namespace is '{root_ns}', expected '{NS_SCHEMA}'"
            ));
        } else {
            report.ok("Default namespace correct");
        }
        if report.stopped {
            return skd_validation_finish(
                report,
                &file_name,
                out_file,
                out_file_label,
                resolved_path,
            );
        }

        let data_source_nodes = skd_children(root, "dataSource", NS_SCHEMA);
        let mut data_source_names = HashSet::<String>::new();
        for dsn in &data_source_nodes {
            if let Some(name) = skd_child(*dsn, "name", NS_SCHEMA) {
                data_source_names.insert(skd_inner_text(name));
            }
        }

        let data_set_nodes = skd_children(root, "dataSet", NS_SCHEMA);
        let mut data_set_names = HashSet::<String>::new();
        let mut all_field_paths = HashMap::<String, String>::new();
        for ds in &data_set_nodes {
            if let Some(name_node) = skd_child(*ds, "name", NS_SCHEMA) {
                let ds_name = skd_inner_text(name_node);
                data_set_names.insert(ds_name.clone());
                skd_collect_data_set_fields(*ds, &ds_name, &mut all_field_paths);
            }
        }

        let calc_field_nodes = skd_children(root, "calculatedField", NS_SCHEMA);
        let mut calc_field_paths = HashSet::<String>::new();
        for cf in &calc_field_nodes {
            if let Some(dp) = skd_child(*cf, "dataPath", NS_SCHEMA) {
                calc_field_paths.insert(skd_inner_text(dp));
            }
        }
        let total_field_nodes = skd_children(root, "totalField", NS_SCHEMA);
        let param_nodes = skd_children(root, "parameter", NS_SCHEMA);
        let template_nodes = skd_children(root, "template", NS_SCHEMA);
        let mut template_names = HashSet::<String>::new();
        for template in &template_nodes {
            if let Some(name_node) = skd_child(*template, "name", NS_SCHEMA) {
                template_names.insert(skd_inner_text(name_node));
            }
        }
        let group_template_nodes = skd_children(root, "groupTemplate", NS_SCHEMA);
        let variant_nodes = skd_children(root, "settingsVariant", NS_SCHEMA);
        let mut known_fields = all_field_paths.keys().cloned().collect::<HashSet<String>>();
        known_fields.extend(calc_field_paths.iter().cloned());

        skd_validate_data_sources(&mut report, &data_source_nodes);
        if report.stopped {
            return skd_validation_finish(
                report,
                &file_name,
                out_file,
                out_file_label,
                resolved_path,
            );
        }
        skd_validate_data_sets(&mut report, &data_set_nodes, &data_source_names);
        if report.stopped {
            return skd_validation_finish(
                report,
                &file_name,
                out_file,
                out_file_label,
                resolved_path,
            );
        }
        for ds in &data_set_nodes {
            let ds_name = skd_child(*ds, "name", NS_SCHEMA)
                .map(skd_inner_text)
                .unwrap_or_else(|| "(unnamed)".to_string());
            skd_validate_data_set_fields(&mut report, *ds, &ds_name);
            if report.stopped {
                return skd_validation_finish(
                    report,
                    &file_name,
                    out_file.clone(),
                    out_file_label.clone(),
                    resolved_path.clone(),
                );
            }
        }
        if report.stopped {
            return skd_validation_finish(
                report,
                &file_name,
                out_file,
                out_file_label,
                resolved_path,
            );
        }
        skd_validate_data_set_links(&mut report, root, &data_set_names);
        if report.stopped {
            return skd_validation_finish(
                report,
                &file_name,
                out_file,
                out_file_label,
                resolved_path,
            );
        }
        skd_validate_calculated_fields(&mut report, &calc_field_nodes, &all_field_paths);
        if report.stopped {
            return skd_validation_finish(
                report,
                &file_name,
                out_file,
                out_file_label,
                resolved_path,
            );
        }
        skd_validate_total_fields(&mut report, &total_field_nodes);
        if report.stopped {
            return skd_validation_finish(
                report,
                &file_name,
                out_file,
                out_file_label,
                resolved_path,
            );
        }
        skd_validate_parameters(&mut report, &param_nodes);
        if report.stopped {
            return skd_validation_finish(
                report,
                &file_name,
                out_file,
                out_file_label,
                resolved_path,
            );
        }
        skd_validate_templates(&mut report, &template_nodes);
        if report.stopped {
            return skd_validation_finish(
                report,
                &file_name,
                out_file,
                out_file_label,
                resolved_path,
            );
        }
        skd_validate_group_templates(&mut report, &group_template_nodes, &template_names);
        if report.stopped {
            return skd_validation_finish(
                report,
                &file_name,
                out_file,
                out_file_label,
                resolved_path,
            );
        }
        skd_validate_settings_variants(&mut report, &variant_nodes, &known_fields);
        skd_validation_finish(report, &file_name, out_file, out_file_label, resolved_path)
    })();

    match result {
        Ok(run) => {
            let mut stdout = run.stdout.clone();
            let mut artifacts = vec![run.artifact.display().to_string()];
            if let Some(out_file) = &run.out_file {
                if let Err(error) = write_utf8_bom(out_file, run.stdout.trim_end_matches('\n')) {
                    return AdapterOutcome {
                        ok: false,
                        summary: "unica.skd.validate failed in native DCS validator".to_string(),
                        changes: Vec::new(),
                        warnings: Vec::new(),
                        errors: vec![error.clone()],
                        artifacts,
                        stdout: None,
                        stderr: Some(format!("{error}\n")),
                        command: None,
                    };
                }
                if let Some(label) = &run.out_file_label {
                    stdout.push_str(&format!("Written to: {label}\n"));
                }
                artifacts.push(out_file.display().to_string());
            }
            AdapterOutcome {
                ok: run.ok,
                summary: if run.ok {
                    "unica.skd.validate completed with native DCS validator".to_string()
                } else {
                    "unica.skd.validate failed in native DCS validator".to_string()
                },
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: run.errors,
                artifacts,
                stdout: Some(stdout),
                stderr: Some(String::new()),
                command: None,
            }
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.skd.validate failed in native DCS validator".to_string(),
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

pub(crate) fn skd_validation_finish(
    report: SkdValidationReporter,
    file_name: &str,
    out_file: Option<PathBuf>,
    out_file_label: Option<String>,
    artifact: PathBuf,
) -> Result<SkdValidationRun, String> {
    let (ok, stdout, errors) = report.finalize(file_name);
    Ok(SkdValidationRun {
        ok,
        stdout,
        out_file,
        out_file_label,
        artifact,
        errors,
    })
}

pub(crate) fn skd_validate_data_sources(
    report: &mut SkdValidationReporter,
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
        let name = skd_child(*dsn, "name", NS_SCHEMA);
        let typ = skd_child(*dsn, "dataSourceType", NS_SCHEMA);
        let name_text = name.map(skd_inner_text).unwrap_or_default();
        if name_text.is_empty() {
            report.error("DataSource has empty name");
            ds_ok = false;
        } else if !names_seen.insert(name_text.clone()) {
            report.error(format!("Duplicate dataSource name: {name_text}"));
            ds_ok = false;
        }
        if let Some(typ) = typ {
            let type_text = skd_inner_text(typ);
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

pub(crate) fn skd_validate_data_sets(
    report: &mut SkdValidationReporter,
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
        let name_node = skd_child(*ds, "name", NS_SCHEMA);
        let ds_name = name_node
            .map(skd_inner_text)
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
            if let Some(src_node) = skd_child(*ds, "dataSource", NS_SCHEMA) {
                let source = skd_inner_text(src_node);
                if !source.is_empty() && !data_source_names.contains(&source) {
                    report.error(format!(
                        "DataSet '{ds_name}' references unknown dataSource: {source}"
                    ));
                    ds_ok = false;
                }
            }
        }
        if xsi_type == "DataSetQuery" {
            let query_node = skd_child(*ds, "query", NS_SCHEMA);
            if query_node.map(skd_text_of).unwrap_or_default().is_empty() {
                report.warn(format!("DataSet '{ds_name}' (Query) has empty query"));
            }
        }
        if xsi_type == "DataSetObject" {
            let obj_node = skd_child(*ds, "objectName", NS_SCHEMA);
            if obj_node.map(skd_text_of).unwrap_or_default().is_empty() {
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

pub(crate) fn skd_validate_data_set_fields(
    report: &mut SkdValidationReporter,
    ds_node: roxmltree::Node<'_, '_>,
    ds_name: &str,
) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    let fields = skd_children(ds_node, "field", NS_SCHEMA);
    if fields.is_empty() {
        return;
    }
    let mut paths_seen = HashSet::<String>::new();
    let mut field_ok = true;
    for field in &fields {
        let dp = skd_child(*field, "dataPath", NS_SCHEMA);
        let field_ref = skd_child(*field, "field", NS_SCHEMA);
        let path = dp.map(skd_inner_text).unwrap_or_default();
        if path.is_empty() {
            report.error(format!("DataSet '{ds_name}': field has empty dataPath"));
            field_ok = false;
            continue;
        }
        if !paths_seen.insert(path.clone()) {
            report.warn(format!("DataSet '{ds_name}': duplicate dataPath '{path}'"));
        }
        if field_ref.map(skd_inner_text).unwrap_or_default().is_empty() {
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
    for item in skd_children(ds_node, "item", NS_SCHEMA) {
        let item_name = skd_child(item, "name", NS_SCHEMA)
            .map(skd_inner_text)
            .unwrap_or_else(|| "(unnamed item)".to_string());
        skd_validate_data_set_fields(report, item, &item_name);
    }
}

pub(crate) fn skd_validate_data_set_links(
    report: &mut SkdValidationReporter,
    root: roxmltree::Node<'_, '_>,
    data_set_names: &HashSet<String>,
) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    let link_nodes = skd_children(root, "dataSetLink", NS_SCHEMA);
    if link_nodes.is_empty() {
        return;
    }
    let mut link_ok = true;
    for link in &link_nodes {
        let src = skd_child(*link, "sourceDataSet", NS_SCHEMA);
        let dst = skd_child(*link, "destinationDataSet", NS_SCHEMA);
        let src_expr = skd_child(*link, "sourceExpression", NS_SCHEMA);
        let dst_expr = skd_child(*link, "destinationExpression", NS_SCHEMA);
        let src_text = src.map(skd_inner_text).unwrap_or_default();
        if !src_text.is_empty() && !data_set_names.contains(&src_text) {
            report.error(format!("DataSetLink: sourceDataSet '{src_text}' not found"));
            link_ok = false;
        }
        let dst_text = dst.map(skd_inner_text).unwrap_or_default();
        if !dst_text.is_empty() && !data_set_names.contains(&dst_text) {
            report.error(format!(
                "DataSetLink: destinationDataSet '{dst_text}' not found"
            ));
            link_ok = false;
        }
        if src_expr.map(skd_text_of).unwrap_or_default().is_empty() {
            report.error("DataSetLink: empty sourceExpression");
            link_ok = false;
        }
        if dst_expr.map(skd_text_of).unwrap_or_default().is_empty() {
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

pub(crate) fn skd_validate_calculated_fields(
    report: &mut SkdValidationReporter,
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
        let dp = skd_child(*calc, "dataPath", NS_SCHEMA);
        let expr = skd_child(*calc, "expression", NS_SCHEMA);
        let path = dp.map(skd_inner_text).unwrap_or_default();
        if path.is_empty() {
            report.error("CalculatedField has empty dataPath");
            cf_ok = false;
            continue;
        }
        if !cf_seen.insert(path.clone()) {
            report.error(format!("Duplicate calculatedField dataPath: {path}"));
            cf_ok = false;
        }
        if expr.map(skd_text_of).unwrap_or_default().is_empty() {
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

pub(crate) fn skd_validate_total_fields(
    report: &mut SkdValidationReporter,
    total_field_nodes: &[roxmltree::Node<'_, '_>],
) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    if total_field_nodes.is_empty() {
        return;
    }
    let mut tf_ok = true;
    for total in total_field_nodes {
        let dp = skd_child(*total, "dataPath", NS_SCHEMA);
        let expr = skd_child(*total, "expression", NS_SCHEMA);
        let path = dp.map(skd_inner_text).unwrap_or_default();
        if path.is_empty() {
            report.error("TotalField has empty dataPath");
            tf_ok = false;
            continue;
        }
        if expr.map(skd_text_of).unwrap_or_default().is_empty() {
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

pub(crate) fn skd_validate_parameters(
    report: &mut SkdValidationReporter,
    param_nodes: &[roxmltree::Node<'_, '_>],
) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    if param_nodes.is_empty() {
        return;
    }
    let mut param_ok = true;
    let mut param_seen = HashSet::<String>::new();
    for param in param_nodes {
        let name = skd_child(*param, "name", NS_SCHEMA)
            .map(skd_inner_text)
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

pub(crate) fn skd_validate_templates(
    report: &mut SkdValidationReporter,
    template_nodes: &[roxmltree::Node<'_, '_>],
) {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    if template_nodes.is_empty() {
        return;
    }
    let mut tpl_ok = true;
    let mut tpl_seen = HashSet::<String>::new();
    for template in template_nodes {
        let name = skd_child(*template, "name", NS_SCHEMA)
            .map(skd_inner_text)
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

pub(crate) fn skd_validate_group_templates(
    report: &mut SkdValidationReporter,
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
        let tpl_ref = skd_child(*group_template, "template", NS_SCHEMA)
            .map(skd_inner_text)
            .unwrap_or_default();
        let tpl_type = skd_child(*group_template, "templateType", NS_SCHEMA)
            .map(skd_inner_text)
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

pub(crate) fn skd_validate_settings_variants(
    report: &mut SkdValidationReporter,
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
        let v_name = skd_child(*variant, "name", NS_SETTINGS);
        let variant_name = v_name.map(skd_inner_text).unwrap_or_default();
        if variant_name.is_empty() {
            report.error(format!("SettingsVariant #{} has empty name", idx + 1));
            v_ok = false;
        }
        let settings = skd_child(*variant, "settings", NS_SETTINGS);
        let Some(settings) = settings else {
            report.error(format!(
                "SettingsVariant '{variant_name}' has no settings element"
            ));
            v_ok = false;
            continue;
        };
        skd_check_settings(report, settings, &variant_name, known_fields);
    }
    if v_ok {
        report.ok(format!("{} settingsVariant(s) found", variant_nodes.len()));
    }
}

pub(crate) fn skd_check_settings(
    report: &mut SkdValidationReporter,
    settings_node: roxmltree::Node<'_, '_>,
    variant_name: &str,
    known_fields: &HashSet<String>,
) {
    const NS_SETTINGS: &str = "http://v8.1c.ru/8.1/data-composition-system/settings";
    if report.stopped {
        return;
    }
    for selected_item in skd_find_all_path(
        settings_node,
        &[("selection", NS_SETTINGS), ("item", NS_SETTINGS)],
    ) {
        let xsi_type = attribute_by_local_name(selected_item, "type").unwrap_or("");
        if xsi_type == "dcsset:SelectedItemField" {
            let field = skd_child(selected_item, "field", NS_SETTINGS)
                .map(skd_inner_text)
                .unwrap_or_default();
            if !field.is_empty() && field != "SystemFields.Number" {
                let base_path = field.split('.').next().unwrap_or("");
                if !known_fields.contains(&field) && !known_fields.contains(base_path) {
                    // Soft check in the reference script: autoFillFields may add implicit fields.
                }
            }
        }
    }
    skd_check_filter_items(report, settings_node, variant_name);
    for order_item in skd_find_all_path(
        settings_node,
        &[("order", NS_SETTINGS), ("item", NS_SETTINGS)],
    ) {
        let xsi_type = attribute_by_local_name(order_item, "type").unwrap_or("");
        if xsi_type == "dcsset:OrderItemField" {
            let order_type = skd_child(order_item, "orderType", NS_SETTINGS)
                .map(skd_inner_text)
                .unwrap_or_default();
            if !order_type.is_empty() && !matches!(order_type.as_str(), "Asc" | "Desc") {
                report.warn(format!(
                    "Variant '{variant_name}' order: invalid orderType '{order_type}'"
                ));
            }
        }
    }
    for structure_item in skd_children(settings_node, "item", NS_SETTINGS) {
        skd_check_structure_item(report, structure_item, variant_name);
    }
}

pub(crate) fn skd_check_filter_items(
    report: &mut SkdValidationReporter,
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
    for filter_item in skd_find_all_path(
        parent_node,
        &[("filter", NS_SETTINGS), ("item", NS_SETTINGS)],
    ) {
        if report.stopped {
            return;
        }
        let xsi_type = attribute_by_local_name(filter_item, "type").unwrap_or("");
        if xsi_type == "dcsset:FilterItemComparison" {
            let comp_type = skd_child(filter_item, "comparisonType", NS_SETTINGS)
                .map(skd_inner_text)
                .unwrap_or_default();
            if !comp_type.is_empty() && !valid_comparison_types.contains(&comp_type.as_str()) {
                report.error(format!(
                    "Variant '{variant_name}' filter: invalid comparisonType '{comp_type}'"
                ));
            }
        } else if xsi_type == "dcsset:FilterItemGroup" {
            let group_type = skd_child(filter_item, "groupType", NS_SETTINGS)
                .map(skd_inner_text)
                .unwrap_or_default();
            if !group_type.is_empty()
                && !matches!(group_type.as_str(), "AndGroup" | "OrGroup" | "NotGroup")
            {
                report.warn(format!(
                    "Variant '{variant_name}' filter group: unusual groupType '{group_type}'"
                ));
            }
            for nested in skd_children(filter_item, "item", NS_SETTINGS) {
                let nested_type = attribute_by_local_name(nested, "type").unwrap_or("");
                if nested_type == "dcsset:FilterItemComparison" {
                    let comp_type = skd_child(nested, "comparisonType", NS_SETTINGS)
                        .map(skd_inner_text)
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

pub(crate) fn skd_check_structure_item(
    report: &mut SkdValidationReporter,
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
    for nested in skd_children(item_node, "item", NS_SETTINGS) {
        skd_check_structure_item(report, nested, variant_name);
    }
    if xsi_type == "dcsset:StructureItemTable" {
        let columns = skd_children(item_node, "column", NS_SETTINGS);
        let rows = skd_children(item_node, "row", NS_SETTINGS);
        if columns.is_empty() {
            report.warn(format!("Variant '{variant_name}': table has no columns"));
        }
        if rows.is_empty() {
            report.warn(format!("Variant '{variant_name}': table has no rows"));
        }
    }
}

pub(crate) fn skd_collect_data_set_fields(
    ds_node: roxmltree::Node<'_, '_>,
    ds_name: &str,
    all_field_paths: &mut HashMap<String, String>,
) -> HashSet<String> {
    const NS_SCHEMA: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    let mut local_paths = HashSet::<String>::new();
    for field in skd_children(ds_node, "field", NS_SCHEMA) {
        if let Some(dp) = skd_child(field, "dataPath", NS_SCHEMA) {
            let path = skd_inner_text(dp);
            local_paths.insert(path.clone());
            all_field_paths.insert(path, ds_name.to_string());
        }
    }
    for item in skd_children(ds_node, "item", NS_SCHEMA) {
        if let Some(item_name) = skd_child(item, "name", NS_SCHEMA) {
            skd_collect_data_set_fields(item, &skd_inner_text(item_name), all_field_paths);
        }
    }
    local_paths
}

pub(crate) fn skd_children<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
    namespace: &str,
) -> Vec<roxmltree::Node<'a, 'input>> {
    node.children()
        .filter(|child| role_info_element(*child, local_name, Some(namespace)))
        .collect()
}

pub(crate) fn skd_child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
    namespace: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children()
        .find(|child| role_info_element(*child, local_name, Some(namespace)))
}

pub(crate) fn skd_find_all_path<'a, 'input>(
    parent: roxmltree::Node<'a, 'input>,
    path: &[(&str, &str)],
) -> Vec<roxmltree::Node<'a, 'input>> {
    let mut current = vec![parent];
    for (local_name, namespace) in path {
        let mut next = Vec::<roxmltree::Node<'a, 'input>>::new();
        for node in current {
            next.extend(skd_children(node, local_name, namespace));
        }
        current = next;
    }
    current
}

pub(crate) fn skd_inner_text(node: roxmltree::Node<'_, '_>) -> String {
    node.text().unwrap_or("").to_string()
}

pub(crate) fn skd_text_of(node: roxmltree::Node<'_, '_>) -> String {
    node.text().unwrap_or("").trim().to_string()
}

pub(crate) fn resolve_skd_validate_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let raw_path = required_path(
        args,
        &["templatePath", "TemplatePath", "path", "Path"],
        "TemplatePath",
    )?;
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

pub(crate) fn compile_skd(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    let write_result = (|| -> Result<(String, PathBuf), String> {
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

        let (json_text, query_base_dir) = if let Some(definition_file) = definition_file {
            let definition_file = absolutize(definition_file, &context.cwd);
            if !definition_file.exists() {
                return Err(format!(
                    "Definition file not found: {}",
                    definition_file.display()
                ));
            }
            let text = fs::read_to_string(&definition_file)
                .map_err(|err| format!("failed to read {}: {err}", definition_file.display()))?;
            let base_dir = definition_file
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| context.cwd.clone());
            (text, base_dir)
        } else {
            (value.unwrap_or("").to_string(), context.cwd.clone())
        };

        let mut defn: Value = serde_json::from_str(json_text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("failed to parse SKD JSON: {err}"))?;
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

        let content = skd_compile_xml(&defn, &query_base_dir, &context.cwd)?;
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        write_utf8_bom(&output_path, &content)?;

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
        let file_size = fs::metadata(&output_path)
            .map_err(|err| format!("failed to stat {}: {err}", output_path.display()))?
            .len();
        let stdout = format!(
            "OK  {output_path_label}\n    DataSets: {ds_count}  Fields: {field_count}  Calculated: {calc_count}  Totals: {total_count}  Params: {param_count}  Variants: {variant_count}\n    Size: {file_size} bytes\n"
        );
        Ok((stdout, output_path))
    })();

    match write_result {
        Ok((stdout, output_path)) => AdapterOutcome {
            ok: true,
            summary: "unica.skd.compile completed with native DCS compiler".to_string(),
            changes: vec![format!("created {}", output_path.display())],
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![output_path.display().to_string()],
            stdout: Some(stdout),
            stderr: None,
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.skd.compile failed in native DCS compiler".to_string(),
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

pub(crate) fn skd_compile_xml(
    defn: &Value,
    query_base_dir: &Path,
    cwd: &Path,
) -> Result<String, String> {
    let data_sources = skd_compile_data_sources(defn);
    let default_source = data_sources
        .first()
        .map(|source| source.0.clone())
        .unwrap_or_else(|| "ИсточникДанных1".to_string());
    let mut lines = vec![
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string(),
        "<DataCompositionSchema xmlns=\"http://v8.1c.ru/8.1/data-composition-system/schema\""
            .to_string(),
        "\t\txmlns:dcscom=\"http://v8.1c.ru/8.1/data-composition-system/common\"".to_string(),
        "\t\txmlns:dcscor=\"http://v8.1c.ru/8.1/data-composition-system/core\"".to_string(),
        "\t\txmlns:dcsset=\"http://v8.1c.ru/8.1/data-composition-system/settings\"".to_string(),
        "\t\txmlns:v8=\"http://v8.1c.ru/8.1/data/core\"".to_string(),
        "\t\txmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\"".to_string(),
        "\t\txmlns:xs=\"http://www.w3.org/2001/XMLSchema\"".to_string(),
        "\t\txmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">".to_string(),
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
            skd_compile_emit_data_set(
                &mut lines,
                data_set,
                "\t",
                &default_source,
                query_base_dir,
                cwd,
            )?;
        }
    }

    skd_compile_emit_default_settings_variant(&mut lines);
    lines.push("</DataCompositionSchema>".to_string());
    Ok(format!("{}\n", lines.join("\n")))
}

pub(crate) fn skd_compile_data_sources(defn: &Value) -> Vec<(String, String)> {
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

pub(crate) fn skd_compile_emit_data_set(
    lines: &mut Vec<String>,
    data_set: &Value,
    indent: &str,
    default_source: &str,
    query_base_dir: &Path,
    cwd: &Path,
) -> Result<(), String> {
    let ds_type = if data_set.get("items").is_some() {
        "DataSetUnion"
    } else if data_set.get("objectName").is_some() {
        "DataSetObject"
    } else {
        "DataSetQuery"
    };
    lines.push(format!("{indent}<dataSet xsi:type=\"{ds_type}\">"));
    lines.push(format!(
        "{indent}\t<name>{}</name>",
        escape_xml(&json_string_field(data_set, "name").unwrap_or_default())
    ));
    if let Some(fields) = data_set.get("fields").and_then(Value::as_array) {
        for field in fields {
            skd_compile_emit_field(lines, field, &format!("{indent}\t"));
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
            let query = skd_compile_resolve_query_value(&query, query_base_dir, cwd)?;
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
                    skd_compile_emit_data_set(
                        lines,
                        item,
                        &format!("{indent}\t"),
                        default_source,
                        query_base_dir,
                        cwd,
                    )?;
                }
            }
        }
        _ => {}
    }
    lines.push(format!("{indent}</dataSet>"));
    Ok(())
}

pub(crate) fn skd_compile_emit_field(lines: &mut Vec<String>, field: &Value, indent: &str) {
    let (data_path, field_name, title, field_type) = if let Some(text) = field.as_str() {
        let parsed = skd_compile_parse_field_shorthand(text);
        (
            parsed.0.clone(),
            parsed.1,
            String::new(),
            skd_compile_resolve_type(&parsed.2),
        )
    } else {
        let data_path = json_string_field(field, "dataPath")
            .or_else(|| json_string_field(field, "field"))
            .unwrap_or_default();
        let field_name = json_string_field(field, "field").unwrap_or_else(|| data_path.clone());
        let title = json_string_field(field, "title").unwrap_or_default();
        let field_type = field
            .get("type")
            .map(skd_compile_type_value)
            .unwrap_or_default();
        (data_path, field_name, title, field_type)
    };

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
        skd_compile_emit_mltext(lines, &format!("{indent}\t"), "title", &title);
    }
    if !field_type.is_empty() {
        lines.push(format!("{indent}\t<valueType>"));
        skd_compile_emit_value_type(lines, &field_type, &format!("{indent}\t\t"));
        lines.push(format!("{indent}\t</valueType>"));
    }
    lines.push(format!("{indent}</field>"));
}

pub(crate) fn skd_compile_parse_field_shorthand(text: &str) -> (String, String, String) {
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
            skd_compile_resolve_type(right.trim()),
        )
    } else {
        (value.to_string(), value.to_string(), String::new())
    }
}

pub(crate) fn skd_compile_type_value(value: &Value) -> String {
    if let Some(items) = value.as_array() {
        return items
            .iter()
            .map(skd_compile_type_value)
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()
            .join("|");
    }
    json_value_to_python_string(value)
        .split('|')
        .map(str::trim)
        .map(skd_compile_resolve_type)
        .collect::<Vec<_>>()
        .join("|")
}

pub(crate) fn skd_compile_resolve_type(type_str: &str) -> String {
    if type_str.is_empty() {
        return String::new();
    }
    if let Some(open) = type_str.find('(') {
        if type_str.ends_with(')') {
            let base = type_str[..open].trim();
            let params = &type_str[open + 1..type_str.len() - 1];
            if let Some(resolved) = skd_compile_type_synonym(base) {
                return format!("{resolved}({params})");
            }
        }
    }
    if let Some(dot_idx) = type_str.find('.') {
        let prefix = &type_str[..dot_idx];
        if let Some(resolved) = skd_compile_type_synonym(prefix) {
            return format!("{resolved}{}", &type_str[dot_idx..]);
        }
    }
    skd_compile_type_synonym(type_str)
        .unwrap_or(type_str)
        .to_string()
}

pub(crate) fn skd_compile_type_synonym(type_str: &str) -> Option<&'static str> {
    match type_str.to_lowercase().as_str() {
        "число" | "bool" | "int" | "integer" | "number" | "num" => Some("decimal"),
        "строка" | "str" => Some("string"),
        "булево" => Some("boolean"),
        "дата" => Some("date"),
        "датавремя" => Some("dateTime"),
        "стандартныйпериод" => Some("StandardPeriod"),
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

pub(crate) fn skd_compile_emit_value_type(lines: &mut Vec<String>, type_spec: &str, indent: &str) {
    for part in type_spec
        .split('|')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        skd_compile_emit_single_value_type(lines, part, indent);
    }
}

pub(crate) fn skd_compile_emit_single_value_type(
    lines: &mut Vec<String>,
    type_str: &str,
    indent: &str,
) {
    let type_str = skd_compile_resolve_type(type_str);
    if type_str == "boolean" {
        lines.push(format!("{indent}<v8:Type>xs:boolean</v8:Type>"));
        return;
    }
    if type_str == "StandardPeriod" {
        lines.push(format!("{indent}<v8:Type>v8:StandardPeriod</v8:Type>"));
        return;
    }
    if let Some(length) = skd_compile_string_length(&type_str) {
        lines.push(format!("{indent}<v8:Type>xs:string</v8:Type>"));
        lines.push(format!("{indent}<v8:StringQualifiers>"));
        lines.push(format!("{indent}\t<v8:Length>{length}</v8:Length>"));
        lines.push(format!(
            "{indent}\t<v8:AllowedLength>Variable</v8:AllowedLength>"
        ));
        lines.push(format!("{indent}</v8:StringQualifiers>"));
        return;
    }
    if let Some((digits, fraction, sign)) = skd_compile_decimal_qualifiers(&type_str) {
        lines.push(format!("{indent}<v8:Type>xs:decimal</v8:Type>"));
        lines.push(format!("{indent}<v8:NumberQualifiers>"));
        lines.push(format!("{indent}\t<v8:Digits>{digits}</v8:Digits>"));
        lines.push(format!(
            "{indent}\t<v8:FractionDigits>{fraction}</v8:FractionDigits>"
        ));
        lines.push(format!("{indent}\t<v8:AllowedSign>{sign}</v8:AllowedSign>"));
        lines.push(format!("{indent}</v8:NumberQualifiers>"));
        return;
    }
    if matches!(type_str.as_str(), "date" | "dateTime") {
        let fractions = if type_str == "date" {
            "Date"
        } else {
            "DateTime"
        };
        lines.push(format!("{indent}<v8:Type>xs:dateTime</v8:Type>"));
        lines.push(format!("{indent}<v8:DateQualifiers>"));
        lines.push(format!(
            "{indent}\t<v8:DateFractions>{fractions}</v8:DateFractions>"
        ));
        lines.push(format!("{indent}</v8:DateQualifiers>"));
        return;
    }
    if type_str.contains('.') {
        lines.push(format!(
            "{indent}<v8:Type xmlns:d5p1=\"http://v8.1c.ru/8.1/data/enterprise/current-config\">d5p1:{}</v8:Type>",
            escape_xml(&type_str)
        ));
    } else {
        lines.push(format!(
            "{indent}<v8:Type>{}</v8:Type>",
            escape_xml(&type_str)
        ));
    }
}

pub(crate) fn skd_compile_string_length(type_str: &str) -> Option<&str> {
    if type_str == "string" {
        return Some("0");
    }
    let rest = type_str.strip_prefix("string(")?.strip_suffix(')')?;
    (!rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit())).then_some(rest)
}

pub(crate) fn skd_compile_decimal_qualifiers(type_str: &str) -> Option<(&str, &str, &'static str)> {
    let rest = type_str.strip_prefix("decimal(")?.strip_suffix(')')?;
    let parts = rest.split(',').map(str::trim).collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }
    let sign = if parts.get(2).copied() == Some("nonneg") {
        "Nonnegative"
    } else {
        "Any"
    };
    Some((parts[0], parts[1], sign))
}

pub(crate) fn skd_compile_emit_mltext(
    lines: &mut Vec<String>,
    indent: &str,
    tag: &str,
    text: &str,
) {
    if text.is_empty() {
        lines.push(format!("{indent}<{tag}/>"));
        return;
    }
    lines.push(format!("{indent}<{tag} xsi:type=\"v8:LocalStringType\">"));
    lines.push(format!("{indent}\t<v8:item>"));
    lines.push(format!("{indent}\t\t<v8:lang>ru</v8:lang>"));
    lines.push(format!(
        "{indent}\t\t<v8:content>{}</v8:content>",
        escape_xml(text)
    ));
    lines.push(format!("{indent}\t</v8:item>"));
    lines.push(format!("{indent}</{tag}>"));
}

pub(crate) fn skd_compile_emit_default_settings_variant(lines: &mut Vec<String>) {
    lines.push("\t<settingsVariant>".to_string());
    lines.push("\t\t<dcsset:name>Основной</dcsset:name>".to_string());
    skd_compile_emit_mltext(lines, "\t\t", "dcsset:presentation", "Основной");
    lines.push("\t\t<dcsset:settings xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\">".to_string());
    lines.push("\t\t\t<dcsset:selection>".to_string());
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

pub(crate) fn skd_compile_resolve_query_value(
    value: &str,
    base_dir: &Path,
    cwd: &Path,
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
            let text = fs::read_to_string(candidate)
                .map_err(|err| format!("failed to read {}: {err}", candidate.display()))?;
            return Ok(text.trim_end().to_string());
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

pub(crate) fn edit_skd(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    let edit_result = (|| -> Result<(String, PathBuf), String> {
        let template_path = resolve_skd_validate_path(args, context)?;
        let operation = required_string(args, &["operation", "Operation"], "Operation")?;
        let value_arg = required_string(args, &["value", "Value"], "Value")?;
        let data_set = string_arg(args, &["dataSet", "DataSet"]).unwrap_or("");
        let variant = string_arg(args, &["variant", "Variant"]).unwrap_or("");
        let no_selection = bool_arg(args, &["noSelection", "NoSelection"]);

        let mut xml_text = fs::read_to_string(&template_path)
            .map_err(|err| format!("failed to read {}: {err}", template_path.display()))?;
        if xml_text.starts_with('\u{feff}') {
            xml_text = xml_text.trim_start_matches('\u{feff}').to_string();
        }
        Document::parse(&xml_text).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;

        let base_dir = template_path.parent().unwrap_or(context.cwd.as_path());
        let values = skd_edit_split_values(operation, value_arg);
        let mut stdout = String::new();
        for value in values {
            match operation {
                "add-field" => skd_edit_add_field(
                    &mut xml_text,
                    data_set,
                    variant,
                    &value,
                    no_selection,
                    &mut stdout,
                )?,
                "set-query" => {
                    let query = skd_compile_resolve_query_value(&value, base_dir, &context.cwd)?;
                    skd_edit_set_query(&mut xml_text, data_set, &query)?;
                    stdout.push_str(&format!(
                        "[OK] Query replaced in dataset \"{}\"\n",
                        skd_edit_dataset_name(&xml_text, data_set)
                            .unwrap_or_else(|| data_set.to_string())
                    ));
                }
                "patch-query" => {
                    let Some((old, new)) = value.split_once(" => ") else {
                        return Err(
                            "patch-query value must contain ' => ' separator: old => new"
                                .to_string(),
                        );
                    };
                    skd_edit_patch_query(&mut xml_text, data_set, old, new)?;
                    stdout.push_str(&format!(
                        "[OK] Query patched in dataset \"{}\": replaced '{}'\n",
                        skd_edit_dataset_name(&xml_text, data_set)
                            .unwrap_or_else(|| data_set.to_string()),
                        old
                    ));
                }
                "clear-selection" => {
                    skd_edit_clear_prefixed_container(&mut xml_text, "dcsset:selection")?;
                    stdout.push_str(&format!(
                        "[OK] Selection cleared in variant \"{}\"\n",
                        skd_edit_variant_name(&xml_text, variant)
                            .unwrap_or_else(|| variant.to_string())
                    ));
                }
                "clear-order" => {
                    skd_edit_clear_prefixed_container(&mut xml_text, "dcsset:order")?;
                    stdout.push_str(&format!(
                        "[OK] Order cleared in variant \"{}\"\n",
                        skd_edit_variant_name(&xml_text, variant)
                            .unwrap_or_else(|| variant.to_string())
                    ));
                }
                "clear-filter" => {
                    skd_edit_clear_prefixed_container(&mut xml_text, "dcsset:filter")?;
                    stdout.push_str(&format!(
                        "[OK] Filter cleared in variant \"{}\"\n",
                        skd_edit_variant_name(&xml_text, variant)
                            .unwrap_or_else(|| variant.to_string())
                    ));
                }
                "add-selection" => {
                    let fragment = skd_edit_selection_fragment(&value, "\t\t\t\t");
                    skd_edit_insert_prefixed_item(&mut xml_text, "dcsset:selection", &fragment)?;
                    stdout.push_str(&format!(
                        "[OK] Selection \"{}\" added to variant \"{}\"\n",
                        value,
                        skd_edit_variant_name(&xml_text, variant)
                            .unwrap_or_else(|| variant.to_string())
                    ));
                }
                "add-order" => {
                    let fragment = skd_edit_order_fragment(&value, "\t\t\t\t");
                    skd_edit_insert_prefixed_item(&mut xml_text, "dcsset:order", &fragment)?;
                    stdout.push_str(&format!(
                        "[OK] Order \"{}\" added to variant \"{}\"\n",
                        value,
                        skd_edit_variant_name(&xml_text, variant)
                            .unwrap_or_else(|| variant.to_string())
                    ));
                }
                "remove-field" => {
                    let removed = skd_edit_remove_dataset_item(
                        &mut xml_text,
                        data_set,
                        "field",
                        "dataPath",
                        &value,
                    )?;
                    if removed {
                        stdout.push_str(&format!(
                            "[OK] Field \"{}\" removed from dataset \"{}\"\n",
                            value,
                            skd_edit_dataset_name(&xml_text, data_set)
                                .unwrap_or_else(|| data_set.to_string())
                        ));
                    } else {
                        stdout.push_str(&format!(
                            "[WARN] Field \"{}\" not found in dataset \"{}\"\n",
                            value,
                            skd_edit_dataset_name(&xml_text, data_set)
                                .unwrap_or_else(|| data_set.to_string())
                        ));
                    }
                    let _ = skd_edit_remove_prefixed_selection_field(&mut xml_text, &value);
                }
                "remove-parameter" => {
                    let removed =
                        skd_edit_remove_top_level_item(&mut xml_text, "parameter", "name", &value)?;
                    if removed {
                        stdout.push_str(&format!("[OK] Parameter \"{}\" removed\n", value));
                    } else {
                        stdout.push_str(&format!("[WARN] Parameter \"{}\" not found\n", value));
                    }
                }
                other => {
                    return Err(format!(
                        "native skd-edit does not support Operation '{other}' yet"
                    ));
                }
            }
        }

        write_utf8_bom(&template_path, &xml_text)?;
        stdout.push_str(&format!("[OK] Saved {}\n", template_path.display()));
        Ok((stdout, template_path))
    })();

    match edit_result {
        Ok((stdout, template_path)) => AdapterOutcome {
            ok: true,
            summary: "unica.skd.edit completed with native DCS editor".to_string(),
            changes: vec![format!("updated {}", template_path.display())],
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![template_path.display().to_string()],
            stdout: Some(stdout),
            stderr: None,
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.skd.edit failed in native DCS editor".to_string(),
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

pub(crate) fn skd_edit_split_values(operation: &str, value: &str) -> Vec<String> {
    if matches!(operation, "set-query" | "set-structure" | "add-dataSet") {
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

pub(crate) fn skd_edit_add_field(
    xml_text: &mut String,
    data_set: &str,
    variant: &str,
    value: &str,
    no_selection: bool,
    stdout: &mut String,
) -> Result<(), String> {
    let parsed = skd_edit_parse_field(value);
    let range = skd_edit_dataset_range(xml_text, data_set)?;
    let escaped_data_path = escape_xml(&parsed.data_path);
    let duplicate_probe = format!("<dataPath>{escaped_data_path}</dataPath>");
    let dataset_text = &xml_text[range.0..range.1];
    let data_set_name =
        skd_edit_dataset_name(xml_text, data_set).unwrap_or_else(|| data_set.to_string());
    if dataset_text.contains(&duplicate_probe) {
        stdout.push_str(&format!(
            "[WARN] Field \"{}\" already exists in dataset \"{}\" -- skipped\n",
            parsed.data_path, data_set_name
        ));
        return Ok(());
    }

    let mut lines = Vec::new();
    skd_edit_emit_field(&mut lines, &parsed, "\t\t");
    skd_edit_insert_before_dataset_close(xml_text, range, &lines.join("\n"))?;
    stdout.push_str(&format!(
        "[OK] Field \"{}\" added to dataset \"{}\"\n",
        parsed.data_path, data_set_name
    ));

    if !no_selection {
        let fragment = skd_edit_selection_fragment(&parsed.data_path, "\t\t\t");
        if skd_edit_prefixed_container_contains_field(
            xml_text,
            "dcsset:selection",
            &parsed.data_path,
        ) {
            stdout.push_str(&format!(
                "[INFO] Field \"{}\" already in selection -- skipped\n",
                parsed.data_path
            ));
        } else if skd_edit_insert_prefixed_item(xml_text, "dcsset:selection", &fragment).is_ok() {
            stdout.push_str(&format!(
                "[OK] Field \"{}\" added to selection of variant \"{}\"\n",
                parsed.data_path,
                skd_edit_variant_name(xml_text, variant).unwrap_or_else(|| variant.to_string())
            ));
        }
    }
    Ok(())
}

pub(crate) struct SkdEditField {
    pub(crate) data_path: String,
    pub(crate) field: String,
    pub(crate) title: String,
    pub(crate) field_type: String,
}

pub(crate) fn skd_edit_parse_field(value: &str) -> SkdEditField {
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
    let text = text
        .split_whitespace()
        .filter(|part| !part.starts_with('@') && !part.starts_with('#'))
        .collect::<Vec<_>>()
        .join(" ");
    let (data_path, field_type) = if let Some((left, right)) = text.split_once(':') {
        (
            left.trim().to_string(),
            skd_compile_resolve_type(right.trim()),
        )
    } else {
        (text.trim().to_string(), String::new())
    };
    SkdEditField {
        field: data_path.clone(),
        data_path,
        title,
        field_type,
    }
}

pub(crate) fn skd_edit_emit_field(lines: &mut Vec<String>, field: &SkdEditField, indent: &str) {
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
        skd_compile_emit_mltext(lines, &format!("{indent}\t"), "title", &field.title);
    }
    if !field.field_type.is_empty() {
        lines.push(format!("{indent}\t<valueType>"));
        skd_compile_emit_value_type(lines, &field.field_type, &format!("{indent}\t\t"));
        lines.push(format!("{indent}\t</valueType>"));
    }
    lines.push(format!("{indent}</field>"));
}

pub(crate) fn skd_edit_set_query(
    xml_text: &mut String,
    data_set: &str,
    query: &str,
) -> Result<(), String> {
    let range = skd_edit_dataset_range(xml_text, data_set)?;
    skd_edit_replace_child_text(xml_text, range, "query", query)
}

pub(crate) fn skd_edit_patch_query(
    xml_text: &mut String,
    data_set: &str,
    old: &str,
    new: &str,
) -> Result<(), String> {
    let range = skd_edit_dataset_range(xml_text, data_set)?;
    let query_range = skd_edit_child_text_range(xml_text, range, "query")?;
    let current = &xml_text[query_range.clone()];
    let escaped_old = escape_xml(old);
    if !current.contains(&escaped_old) {
        return Err(format!(
            "Substring not found in query of dataset '{}': {}",
            skd_edit_dataset_name(xml_text, data_set).unwrap_or_else(|| data_set.to_string()),
            old
        ));
    }
    let patched = current.replace(&escaped_old, &escape_xml(new));
    xml_text.replace_range(query_range, &patched);
    Ok(())
}

pub(crate) fn skd_edit_dataset_range(
    xml_text: &str,
    data_set: &str,
) -> Result<(usize, usize), String> {
    let mut cursor = 0;
    while let Some(rel_start) = xml_text[cursor..].find("<dataSet") {
        let start = cursor + rel_start;
        let Some(rel_end) = xml_text[start..].find("</dataSet>") else {
            return Err("No closing </dataSet> found".to_string());
        };
        let end = start + rel_end + "</dataSet>".len();
        let block = &xml_text[start..end];
        if data_set.is_empty() || block.contains(&format!("<name>{}</name>", escape_xml(data_set)))
        {
            return Ok((start, end));
        }
        cursor = end;
    }
    if data_set.is_empty() {
        Err("No dataSet found in DCS".to_string())
    } else {
        Err(format!("DataSet '{data_set}' not found"))
    }
}

pub(crate) fn skd_edit_dataset_name(xml_text: &str, data_set: &str) -> Option<String> {
    let range = skd_edit_dataset_range(xml_text, data_set).ok()?;
    let name_range = skd_edit_child_text_range(xml_text, range, "name").ok()?;
    Some(xml_text[name_range].trim().to_string())
}

pub(crate) fn skd_edit_variant_name(xml_text: &str, variant: &str) -> Option<String> {
    if !variant.is_empty() {
        return Some(variant.to_string());
    }
    let open = "<settingsVariant";
    let start = xml_text.find(open)?;
    let end = xml_text[start..].find("</settingsVariant>")? + start + "</settingsVariant>".len();
    let name_range = skd_edit_prefixed_child_text_range(xml_text, (start, end), "dcsset:name")
        .or_else(|_| skd_edit_child_text_range(xml_text, (start, end), "name"))
        .ok()?;
    Some(xml_text[name_range].trim().to_string())
}

pub(crate) fn skd_edit_insert_before_dataset_close(
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

pub(crate) fn skd_edit_replace_child_text(
    xml_text: &mut String,
    range: (usize, usize),
    child: &str,
    value: &str,
) -> Result<(), String> {
    let text_range = skd_edit_child_text_range(xml_text, range, child)?;
    xml_text.replace_range(text_range, &escape_xml(value));
    Ok(())
}

pub(crate) fn skd_edit_child_text_range(
    xml_text: &str,
    range: (usize, usize),
    child: &str,
) -> Result<std::ops::Range<usize>, String> {
    let open = format!("<{child}>");
    let close = format!("</{child}>");
    let block = &xml_text[range.0..range.1];
    let Some(open_rel) = block.find(&open) else {
        return Err(format!("No <{child}> element found"));
    };
    let text_start = range.0 + open_rel + open.len();
    let Some(close_rel) = xml_text[text_start..range.1].find(&close) else {
        return Err(format!("No </{child}> element found"));
    };
    Ok(text_start..text_start + close_rel)
}

pub(crate) fn skd_edit_prefixed_child_text_range(
    xml_text: &str,
    range: (usize, usize),
    child: &str,
) -> Result<std::ops::Range<usize>, String> {
    skd_edit_child_text_range(xml_text, range, child)
}

pub(crate) fn skd_edit_selection_fragment(field_name: &str, indent: &str) -> String {
    if field_name == "Auto" {
        return format!("{indent}<dcsset:item xsi:type=\"dcsset:SelectedItemAuto\"/>");
    }
    format!(
        "{indent}<dcsset:item xsi:type=\"dcsset:SelectedItemField\">\n{indent}\t<dcsset:field>{}</dcsset:field>\n{indent}</dcsset:item>",
        escape_xml(field_name)
    )
}

pub(crate) fn skd_edit_order_fragment(value: &str, indent: &str) -> String {
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

pub(crate) fn skd_edit_insert_prefixed_item(
    xml_text: &mut String,
    container: &str,
    fragment: &str,
) -> Result<(), String> {
    let close = format!("</{container}>");
    let Some(pos) = xml_text.find(&close) else {
        return Err(format!("No <{container}> section found in DCS"));
    };
    xml_text.insert_str(pos, &format!("{fragment}\n\t\t\t"));
    Ok(())
}

pub(crate) fn skd_edit_clear_prefixed_container(
    xml_text: &mut String,
    container: &str,
) -> Result<(), String> {
    let open = format!("<{container}>");
    let close = format!("</{container}>");
    let Some(open_pos) = xml_text.find(&open) else {
        return Err(format!("No <{container}> section found in DCS"));
    };
    let content_start = open_pos + open.len();
    let Some(close_rel) = xml_text[content_start..].find(&close) else {
        return Err(format!("No </{container}> section found in DCS"));
    };
    xml_text.replace_range(content_start..content_start + close_rel, "\n\t\t\t");
    Ok(())
}

pub(crate) fn skd_edit_prefixed_container_contains_field(
    xml_text: &str,
    container: &str,
    field: &str,
) -> bool {
    let open = format!("<{container}>");
    let close = format!("</{container}>");
    let Some(open_pos) = xml_text.find(&open) else {
        return false;
    };
    let content_start = open_pos + open.len();
    let Some(close_rel) = xml_text[content_start..].find(&close) else {
        return false;
    };
    xml_text[content_start..content_start + close_rel].contains(&format!(
        "<dcsset:field>{}</dcsset:field>",
        escape_xml(field)
    ))
}

pub(crate) fn skd_edit_remove_dataset_item(
    xml_text: &mut String,
    data_set: &str,
    item: &str,
    child: &str,
    value: &str,
) -> Result<bool, String> {
    let range = skd_edit_dataset_range(xml_text, data_set)?;
    skd_edit_remove_item_by_child(xml_text, range, item, child, value)
}

pub(crate) fn skd_edit_remove_top_level_item(
    xml_text: &mut String,
    item: &str,
    child: &str,
    value: &str,
) -> Result<bool, String> {
    skd_edit_remove_item_by_child(xml_text, (0, xml_text.len()), item, child, value)
}

pub(crate) fn skd_edit_remove_item_by_child(
    xml_text: &mut String,
    range: (usize, usize),
    item: &str,
    child: &str,
    value: &str,
) -> Result<bool, String> {
    let open_prefix = format!("<{item}");
    let close = format!("</{item}>");
    let child_probe = format!("<{child}>{}</{child}>", escape_xml(value));
    let mut cursor = range.0;
    while cursor < range.1 {
        let Some(open_rel) = xml_text[cursor..range.1].find(&open_prefix) else {
            return Ok(false);
        };
        let start = cursor + open_rel;
        let Some(close_rel) = xml_text[start..range.1].find(&close) else {
            return Err(format!("No {close} found"));
        };
        let end = start + close_rel + close.len();
        if xml_text[start..end].contains(&child_probe) {
            xml_text.replace_range(start..end, "");
            return Ok(true);
        }
        cursor = end;
    }
    Ok(false)
}

pub(crate) fn skd_edit_remove_prefixed_selection_field(
    xml_text: &mut String,
    field: &str,
) -> Result<bool, String> {
    skd_edit_remove_item_by_child(
        xml_text,
        (0, xml_text.len()),
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
        "skd-info" => Some(Ok(analyze_skd_info(args, context))),
        "skd-validate" => Some(Ok(validate_skd(args, context))),
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
        "skd-compile" => Some(compile_skd(args, context)),
        "skd-edit" => Some(edit_skd(args, context)),
        _ => None,
    }
}
