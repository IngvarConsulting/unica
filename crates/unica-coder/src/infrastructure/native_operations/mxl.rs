#![allow(dead_code, unused_imports)]

use crate::application::AdapterOutcome;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::logical_selector::{
    logical_selection, AttachedResource,
};
use crate::infrastructure::platform_xml_owner::MXL_ROOT;
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::common::*;
use super::compile_transaction::CompileTransaction;

pub(crate) const MXL_DOCUMENT_NS: &str = "http://v8.1c.ru/8.2/data/spreadsheet";

pub(crate) fn empty_spreadsheet_document_xml() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<document xmlns=\"{MXL_DOCUMENT_NS}\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n\t<languageSettings>\n\t\t<currentLanguage>ru</currentLanguage>\n\t\t<defaultLanguage>ru</defaultLanguage>\n\t\t<languageInfo>\n\t\t\t<id>ru</id>\n\t\t\t<code>Русский</code>\n\t\t\t<description>Русский</description>\n\t\t</languageInfo>\n\t</languageSettings>\n\t<columns>\n\t\t<size>0</size>\n\t</columns>\n\t<rowsItem>\n\t\t<index>0</index>\n\t\t<row>\n\t\t\t<empty>true</empty>\n\t\t</row>\n\t</rowsItem>\n\t<templateMode>true</templateMode>\n\t<vgRows>0</vgRows>\n</document>\n"
    )
}

pub(crate) fn require_mxl_document_root(root: roxmltree::Node<'_, '_>) -> Result<(), String> {
    let local_name = root.tag_name().name();
    if local_name != "document" {
        return Err(format!(
            "Root element is '{local_name}', expected 'document'"
        ));
    }
    let namespace = root.tag_name().namespace().unwrap_or("");
    if namespace != MXL_DOCUMENT_NS {
        return Err(format!(
            "Root namespace is '{namespace}', expected '{MXL_DOCUMENT_NS}'"
        ));
    }
    Ok(())
}
use super::{
    cf::*, cfe::*, dcs::*, form::*, interface::*, meta::*, role::*, subsystem::*, template::*,
};
#[derive(Clone)]
pub(crate) struct MxlNamedArea {
    pub(crate) name: String,
    pub(crate) area_type: String,
    pub(crate) begin_row: i64,
    pub(crate) end_row: i64,
    pub(crate) begin_col: i64,
    pub(crate) end_col: i64,
    pub(crate) columns_id: Option<String>,
}

pub(crate) struct MxlAreaInfo {
    pub(crate) area: MxlNamedArea,
    pub(crate) params: Vec<String>,
    pub(crate) details: Vec<String>,
    pub(crate) texts: Vec<String>,
    pub(crate) templates: Vec<String>,
}

pub(crate) enum MxlCellData {
    Parameter(String, Option<String>),
    TemplateParam(String),
    Text(String),
    Template(String),
}

pub(crate) struct MxlValidationReporter {
    pub(crate) errors: usize,
    pub(crate) warnings: usize,
    pub(crate) ok_count: usize,
    pub(crate) stopped: bool,
    pub(crate) max_errors: usize,
    pub(crate) detailed: bool,
    pub(crate) lines: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct MxlRawFont {
    pub(crate) face: String,
    pub(crate) size: i64,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    pub(crate) underline: bool,
    pub(crate) strikeout: bool,
}

#[derive(Clone, Default)]
pub(crate) struct MxlRawFormat {
    pub(crate) font_idx: i64,
    pub(crate) lb: i64,
    pub(crate) tb: i64,
    pub(crate) rb: i64,
    pub(crate) bb: i64,
    pub(crate) width: i64,
    pub(crate) height: i64,
    pub(crate) ha: String,
    pub(crate) va: String,
    pub(crate) wrap: bool,
    pub(crate) fill_type: String,
    pub(crate) data_format: String,
}

#[derive(Clone)]
pub(crate) struct MxlDecompiledCell {
    pub(crate) col: i64,
    pub(crate) format_idx: i64,
    pub(crate) param: Option<String>,
    pub(crate) detail: Option<String>,
    pub(crate) text: Option<String>,
}

#[derive(Clone)]
pub(crate) struct MxlDecompiledRow {
    pub(crate) format_idx: i64,
    pub(crate) cells: Vec<MxlDecompiledCell>,
    pub(crate) empty: bool,
}

pub(crate) enum OrderedJson {
    Obj(Vec<(String, OrderedJson)>),
    Arr(Vec<OrderedJson>),
    Str(String),
    Int(i64),
    Bool(bool),
}

type MxlDecompileStyleResult = (
    BTreeMap<String, String>,
    Vec<(String, OrderedJson)>,
    BTreeMap<i64, String>,
);

impl MxlValidationReporter {
    pub(crate) fn new(max_errors: usize, detailed: bool) -> Self {
        Self {
            errors: 0,
            warnings: 0,
            ok_count: 0,
            stopped: false,
            max_errors,
            detailed,
            lines: Vec::new(),
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
}

/// Typed answer of `unica.mxl.info` (ADR-0023).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MxlInfoData {
    pub(crate) name: String,
    pub(crate) support: ObjectSupportData,
    pub(crate) rows: i64,
    pub(crate) columns: i64,
    pub(crate) column_sets: Vec<MxlColumnSetData>,
    pub(crate) areas: Vec<MxlAreaData>,
    pub(crate) outside: MxlOutsideData,
    pub(crate) merge_count: usize,
    pub(crate) drawing_count: usize,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MxlColumnSetData {
    pub(crate) id: String,
    pub(crate) size: i64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MxlAreaData {
    pub(crate) name: String,
    /// `Rows`, `Columns`, `Rectangle` or `Drawing`.
    pub(crate) kind: String,
    pub(crate) begin_row: i64,
    pub(crate) end_row: i64,
    pub(crate) begin_col: i64,
    pub(crate) end_col: i64,
    pub(crate) columns_id: Option<String>,
    pub(crate) drawing_id: Option<String>,
    pub(crate) params: Vec<String>,
    pub(crate) details: Vec<String>,
    /// `null` unless `WithText` asked for cell content, which stays a genuine
    /// content selector rather than a print-size lever.
    pub(crate) texts: Option<Vec<String>>,
    pub(crate) templates: Option<Vec<String>>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MxlOutsideData {
    pub(crate) params: Vec<String>,
    pub(crate) details: Vec<String>,
    pub(crate) texts: Option<Vec<String>>,
    pub(crate) templates: Option<Vec<String>>,
}

pub(crate) struct MxlInfoExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<MxlInfoData>,
}

pub(crate) fn analyze_mxl_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> MxlInfoExecution {
    let result = (|| -> Result<(MxlInfoData, PathBuf), String> {
        let template_path = resolve_mxl_info_path(args, context)?;
        if !template_path.is_file() {
            return Err(format!("File not found: {}", template_path.display()));
        }
        let text = fs::read_to_string(&template_path)
            .map_err(|err| format!("failed to read {}: {err}", template_path.display()))?;
        let doc = Document::parse(text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("XML parse error in {}: {err}", template_path.display()))?;
        let root = doc.root_element();
        require_mxl_document_root(root)?;
        let include_text = bool_arg(args, &["withText", "WithText"]);

        let mut column_sets = Vec::<(String, i64)>::new();
        let mut default_col_count = 0i64;
        for cols in root
            .children()
            .filter(|node| role_info_element(*node, "columns", None))
        {
            let size = child_text(cols, "size", None).parse::<i64>().unwrap_or(0);
            let id = child_text(cols, "id", None);
            if id.is_empty() {
                default_col_count = size;
            } else {
                column_sets.push((id, size));
            }
        }

        let row_nodes = root
            .children()
            .filter(|node| role_info_element(*node, "rowsItem", None))
            .collect::<Vec<_>>();
        let doc_height = mxl_logical_height(root, &row_nodes);
        let mut row_map = Vec::<(i64, roxmltree::Node<'_, '_>)>::new();
        for row_item in &row_nodes {
            if let Ok(index) = child_text(*row_item, "index", None).parse::<i64>() {
                row_map.push((index, *row_item));
            }
        }

        let mut named_areas = Vec::<MxlNamedArea>::new();
        let mut named_drawings = Vec::<(String, String)>::new();
        for item in root
            .children()
            .filter(|node| role_info_element(*node, "namedItem", None))
        {
            let item_type = attribute_by_local_name(item, "type").unwrap_or("");
            let name = child_text(item, "name", None);
            if item_type.contains("NamedItemCells") {
                if let Some(area) = item
                    .children()
                    .find(|node| role_info_element(*node, "area", None))
                {
                    named_areas.push(MxlNamedArea {
                        name,
                        area_type: child_text(area, "type", None),
                        begin_row: child_text(area, "beginRow", None).parse().unwrap_or(0),
                        end_row: child_text(area, "endRow", None).parse().unwrap_or(0),
                        begin_col: child_text(area, "beginColumn", None).parse().unwrap_or(0),
                        end_col: child_text(area, "endColumn", None).parse().unwrap_or(0),
                        columns_id: {
                            let value = child_text(area, "columnsID", None);
                            (!value.is_empty()).then_some(value)
                        },
                    });
                }
            } else if item_type.contains("NamedItemDrawing") {
                named_drawings.push((name, child_text(item, "drawingID", None)));
            }
        }
        named_areas.sort_by(|left, right| {
            let left_key = if left.area_type == "Columns" {
                (left.begin_col, &left.name)
            } else {
                (left.begin_row, &left.name)
            };
            let right_key = if right.area_type == "Columns" {
                (right.begin_col, &right.name)
            } else {
                (right.begin_row, &right.name)
            };
            left_key.cmp(&right_key)
        });

        let mut area_data = Vec::<MxlAreaInfo>::new();
        let mut covered_rows = Vec::<i64>::new();
        for area in &named_areas {
            let (params, details, texts, templates) =
                mxl_area_cell_data(area, &row_map, doc_height, include_text);
            if area.begin_row != -1 && area.end_row != -1 {
                for row in area.begin_row..=area.end_row {
                    if !covered_rows.contains(&row) {
                        covered_rows.push(row);
                    }
                }
            }
            area_data.push(MxlAreaInfo {
                area: area.clone(),
                params,
                details,
                texts,
                templates,
            });
        }

        let mut outside_params = Vec::<String>::new();
        let mut outside_details = Vec::<String>::new();
        let mut outside_texts = Vec::<String>::new();
        let mut outside_templates = Vec::<String>::new();
        row_map.sort_by_key(|(index, _)| *index);
        for (row_index, row_node) in &row_map {
            if covered_rows.contains(row_index) {
                continue;
            }
            for cell in mxl_cell_data(*row_node, include_text) {
                match cell {
                    MxlCellData::Parameter(value, detail) => {
                        if let Some(detail) = detail {
                            outside_details.push(format!("{value}->{detail}"));
                        }
                        outside_params.push(value);
                    }
                    MxlCellData::TemplateParam(value) => {
                        outside_params.push(format!("{value} [tpl]"))
                    }
                    MxlCellData::Text(value) => outside_texts.push(value),
                    MxlCellData::Template(value) => outside_templates.push(value),
                }
            }
        }

        let merge_count = root
            .children()
            .filter(|node| role_info_element(*node, "merge", None))
            .count();
        let drawing_count = root
            .children()
            .filter(|node| role_info_element(*node, "drawing", None))
            .count();
        let template_name = template_path
            .parent()
            .and_then(Path::parent)
            .and_then(|path| path.file_name())
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_string();

        // Both branches described the same template: one as prose, the other as
        // a JSON string inside the JSON envelope. Data replaces both.
        let areas = area_data
            .iter()
            .map(|item| MxlAreaData {
                name: item.area.name.clone(),
                kind: item.area.area_type.clone(),
                begin_row: item.area.begin_row,
                end_row: item.area.end_row,
                begin_col: item.area.begin_col,
                end_col: item.area.end_col,
                columns_id: item.area.columns_id.clone(),
                drawing_id: None,
                params: item.params.clone(),
                details: item.details.clone(),
                texts: include_text.then(|| item.texts.clone()),
                templates: include_text.then(|| item.templates.clone()),
            })
            .chain(named_drawings.iter().map(|(name, drawing_id)| MxlAreaData {
                name: name.clone(),
                kind: "Drawing".to_string(),
                begin_row: 0,
                end_row: 0,
                begin_col: 0,
                end_col: 0,
                columns_id: None,
                drawing_id: Some(drawing_id.clone()),
                params: Vec::new(),
                details: Vec::new(),
                texts: include_text.then(Vec::new),
                templates: include_text.then(Vec::new),
            }))
            .collect::<Vec<_>>();

        let data = MxlInfoData {
            name: template_name,
            support: object_support_state(&template_path),
            rows: doc_height,
            columns: default_col_count,
            column_sets: column_sets
                .iter()
                .map(|(id, size)| MxlColumnSetData {
                    id: id.clone(),
                    size: *size,
                })
                .collect(),
            areas,
            outside: MxlOutsideData {
                params: outside_params,
                details: outside_details,
                texts: include_text.then_some(outside_texts),
                templates: include_text.then_some(outside_templates),
            },
            merge_count,
            drawing_count,
        };
        Ok((data, template_path))
    })();

    match result {
        Ok((data, artifact)) => MxlInfoExecution {
            outcome: AdapterOutcome {
                ok: true,
                summary: format!(
                    "unica.mxl.info described {} with {} named area(s)",
                    data.name,
                    data.areas.len()
                ),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: vec![artifact.display().to_string()],
                stdout: None,
                stderr: Some(String::new()),
                command: None,
            },
            data: Some(data),
        },
        Err(error) => MxlInfoExecution {
            outcome: AdapterOutcome {
                ok: false,
                summary: "unica.mxl.info failed in native spreadsheet analyzer".to_string(),
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

pub(crate) fn validate_mxl(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    const NS_D: &str = "http://v8.1c.ru/8.2/data/spreadsheet";

    let result = (|| -> Result<(bool, String, PathBuf, Vec<String>), String> {
        let template_path = resolve_mxl_validate_path(args, context)?;
        let text = fs::read_to_string(&template_path)
            .map_err(|err| format!("failed to read {}: {err}", template_path.display()))?;
        let doc = Document::parse(text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("XML parse error in {}: {err}", template_path.display()))?;
        let root = doc.root_element();

        let detailed = bool_arg(args, &["detailed", "Detailed"]);
        let max_errors = int_arg(args, &["maxErrors", "MaxErrors"])
            .unwrap_or(20)
            .max(0) as usize;
        let mut report = MxlValidationReporter::new(max_errors, detailed);
        let template_display_name = template_path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_string();
        report.lines.push(format!(
            "=== Validation: Template.{template_display_name} ==="
        ));
        report.lines.push(String::new());
        if let Err(error) = require_mxl_document_root(root) {
            report.error(error);
            return Ok(finish_mxl_validation(
                report,
                detailed,
                &template_display_name,
                template_path,
            ));
        }

        let line_count = mxl_direct_children(root, "line", Some(NS_D)).len();
        let font_count = mxl_direct_children(root, "font", None).len();
        let format_nodes = mxl_direct_children(root, "format", None);
        let format_count = format_nodes.len();
        let picture_count = mxl_direct_children(root, "picture", Some(NS_D)).len();

        let mut column_sets = Vec::<(String, i64)>::new();
        let mut default_col_count = 0i64;
        for cols in mxl_direct_children(root, "columns", Some(NS_D)) {
            let size = mxl_int_child(cols, "size", Some(NS_D));
            let id = mxl_child_text(cols, "id", Some(NS_D));
            if id.is_empty() {
                default_col_count = size;
            } else {
                column_sets.push((id, size));
            }
        }

        let row_nodes = mxl_direct_children(root, "rowsItem", Some(NS_D));
        let doc_height = mxl_int_child(root, "height", Some(NS_D));
        let is_empty_sentinel = is_platform_empty_mxl_sentinel(root, &row_nodes);
        let mut max_row_index = -1i64;
        for row_item in &row_nodes {
            if let Some(idx) = mxl_optional_int_child(*row_item, "index", Some(NS_D)) {
                if idx > max_row_index {
                    max_row_index = idx;
                }
            }
        }
        let expected_min_height = if is_empty_sentinel {
            0
        } else {
            max_row_index + 1
        };
        if doc_height >= expected_min_height {
            report.ok(format!(
                "height ({doc_height}) >= max row index + 1 ({expected_min_height}), rowsItem count={}",
                row_nodes.len()
            ));
        } else {
            report.error(format!(
                "height={doc_height} but max row index={max_row_index} (need at least {expected_min_height})"
            ));
        }

        if let Some(vg_rows) = mxl_optional_int_child(root, "vgRows", Some(NS_D)) {
            if vg_rows <= doc_height {
                report.ok(format!("vgRows ({vg_rows}) <= height ({doc_height})"));
            } else {
                report.warn(format!("vgRows ({vg_rows}) > height ({doc_height})"));
            }
        }

        let mut max_font_ref = 0i64;
        let mut max_line_ref = 0i64;
        for format in &format_nodes {
            if let Some(value) = mxl_optional_int_child(*format, "font", Some(NS_D)) {
                max_font_ref = max_font_ref.max(value);
            }
            for border_name in [
                "leftBorder",
                "topBorder",
                "rightBorder",
                "bottomBorder",
                "drawingBorder",
            ] {
                if let Some(value) = mxl_optional_int_child(*format, border_name, Some(NS_D)) {
                    max_line_ref = max_line_ref.max(value);
                }
            }
        }

        if font_count > 0 {
            if max_font_ref < font_count as i64 {
                report.ok(format!(
                    "Font refs: max={max_font_ref}, palette size={font_count}"
                ));
            } else {
                report.error(format!(
                    "Font index {max_font_ref} exceeds palette size ({font_count})"
                ));
            }
        } else if max_font_ref > 0 {
            report.error(format!(
                "Font index {max_font_ref} referenced but no fonts defined"
            ));
        }

        if line_count > 0 {
            if max_line_ref < line_count as i64 {
                report.ok(format!(
                    "Line/border refs: max={max_line_ref}, palette size={line_count}"
                ));
            } else {
                report.error(format!(
                    "Line index {max_line_ref} exceeds palette size ({line_count})"
                ));
            }
        } else if max_line_ref > 0 {
            report.error(format!(
                "Line index {max_line_ref} referenced but no lines defined"
            ));
        }

        let mut max_cell_format_ref = 0i64;
        let mut max_row_format_ref = 0i64;
        let mut max_default_col_idx = 0i64;
        let mut row_index = 0i64;

        for row_item in &row_nodes {
            if report.stopped {
                break;
            }
            if let Some(idx) = mxl_optional_int_child(*row_item, "index", Some(NS_D)) {
                row_index = idx;
            }
            let Some(row) = mxl_child(*row_item, "row", Some(NS_D)) else {
                row_index += 1;
                continue;
            };

            if let Some(value) = mxl_optional_int_child(row, "formatIndex", Some(NS_D)) {
                max_row_format_ref = max_row_format_ref.max(value);
                if value > format_count as i64 {
                    report.error(format!(
                        "Row {row_index}: formatIndex={value} > format palette size ({format_count})"
                    ));
                }
            }

            let mut row_cols_id = None::<String>;
            let cols_id = mxl_child_text(row, "columnsID", Some(NS_D));
            if !cols_id.is_empty() {
                if !column_sets.iter().any(|(id, _)| id == &cols_id) {
                    report.error(format!(
                        "Row {row_index}: columnsID '{}...' not found in column sets",
                        mxl_prefix(&cols_id, 8)
                    ));
                }
                row_cols_id = Some(cols_id);
            }

            let mut row_col_count = default_col_count;
            if let Some(cols_id) = row_cols_id.as_deref() {
                if let Some((_, size)) = column_sets.iter().find(|(id, _)| id == cols_id) {
                    row_col_count = *size;
                }
            }

            for cell_group in mxl_direct_children(row, "c", Some(NS_D)) {
                if let Some(col_idx) = mxl_optional_int_child(cell_group, "i", Some(NS_D)) {
                    if row_cols_id.is_none() && col_idx > max_default_col_idx {
                        max_default_col_idx = col_idx;
                    }
                    if row_col_count > 0 && col_idx >= row_col_count {
                        report.error(format!(
                            "Row {row_index}: column index {col_idx} >= column count ({row_col_count})"
                        ));
                    }
                }

                if let Some(cell) = mxl_child(cell_group, "c", Some(NS_D)) {
                    if let Some(value) = mxl_optional_int_child(cell, "f", Some(NS_D)) {
                        max_cell_format_ref = max_cell_format_ref.max(value);
                        if value > format_count as i64 {
                            report.error(format!(
                                "Row {row_index}: cell format index {value} > format palette size ({format_count})"
                            ));
                        }
                    }
                }
            }

            row_index += 1;
        }

        if !report.stopped
            && max_cell_format_ref <= format_count as i64
            && max_row_format_ref <= format_count as i64
        {
            report.ok(format!(
                "Format refs: max cell={max_cell_format_ref}, max row={max_row_format_ref}, palette size={format_count}"
            ));
        }

        for cols in mxl_direct_children(root, "columns", Some(NS_D)) {
            if report.stopped {
                break;
            }
            for item in mxl_direct_children(cols, "columnsItem", Some(NS_D)) {
                if let Some(column) = mxl_child(item, "column", Some(NS_D)) {
                    if let Some(value) = mxl_optional_int_child(column, "formatIndex", Some(NS_D)) {
                        if value > format_count as i64 {
                            let col_idx = mxl_child_text(item, "index", Some(NS_D));
                            report.error(format!(
                                "Column {}: formatIndex={value} > format palette size ({format_count})",
                                if col_idx.is_empty() { "?" } else { &col_idx }
                            ));
                        }
                    }
                }
            }
        }

        if !report.stopped {
            report.ok(format!(
                "Column indices: max in default set={max_default_col_idx}, default column count={default_col_count}"
            ));
        }

        for named in mxl_direct_children(root, "namedItem", Some(NS_D)) {
            if report.stopped {
                break;
            }
            let item_type = attribute_by_local_name(named, "type").unwrap_or("");
            let name = mxl_child_text(named, "name", Some(NS_D));
            if !item_type.contains("NamedItemCells") {
                continue;
            }
            let Some(area) = mxl_child(named, "area", Some(NS_D)) else {
                continue;
            };
            let begin_row = mxl_int_child(area, "beginRow", Some(NS_D));
            let end_row = mxl_int_child(area, "endRow", Some(NS_D));
            if begin_row != -1 && begin_row >= doc_height {
                report.error(format!(
                    "Area '{name}': beginRow={begin_row} >= height={doc_height}"
                ));
            }
            if end_row != -1 && end_row >= doc_height {
                report.error(format!(
                    "Area '{name}': endRow={end_row} >= height={doc_height}"
                ));
            }
            let cols_id = mxl_child_text(area, "columnsID", Some(NS_D));
            if !cols_id.is_empty() && !column_sets.iter().any(|(id, _)| id == &cols_id) {
                report.error(format!(
                    "Area '{name}': columnsID '{}...' not found",
                    mxl_prefix(&cols_id, 8)
                ));
            }
        }

        for merge in mxl_direct_children(root, "merge", Some(NS_D)) {
            if report.stopped {
                break;
            }
            let merge_r = mxl_int_child(merge, "r", Some(NS_D));
            let merge_c = mxl_int_child(merge, "c", Some(NS_D));
            if merge_r != -1 && merge_r >= doc_height {
                report.error(format!(
                    "Merge at row={merge_r}, col={merge_c}: row >= height ({doc_height})"
                ));
            }
            if let Some(h) = mxl_optional_int_child(merge, "h", Some(NS_D)) {
                if merge_r != -1 && merge_r + h >= doc_height {
                    report.error(format!(
                        "Merge at row={merge_r}: extends to row {} >= height ({doc_height})",
                        merge_r + h
                    ));
                }
            }
            let cols_id = mxl_child_text(merge, "columnsID", Some(NS_D));
            if !cols_id.is_empty() && !column_sets.iter().any(|(id, _)| id == &cols_id) {
                report.error(format!(
                    "Merge at row={merge_r}, col={merge_c}: columnsID '{}...' not found",
                    mxl_prefix(&cols_id, 8)
                ));
            }
        }

        for drawing in mxl_direct_children(root, "drawing", Some(NS_D)) {
            if report.stopped {
                break;
            }
            if let Some(pic_idx) = mxl_optional_int_child(drawing, "pictureIndex", Some(NS_D)) {
                if pic_idx > picture_count as i64 {
                    let draw_id = mxl_child_text(drawing, "id", Some(NS_D));
                    report.error(format!(
                        "Drawing id={}: pictureIndex={pic_idx} > picture count ({picture_count})",
                        if draw_id.is_empty() { "?" } else { &draw_id }
                    ));
                }
            }
        }

        Ok(finish_mxl_validation(
            report,
            detailed,
            &template_display_name,
            template_path,
        ))
    })();

    match result {
        Ok((ok, stdout, artifact, validation_errors)) => AdapterOutcome {
            ok,
            summary: if ok {
                "unica.mxl.validate completed with native spreadsheet validator".to_string()
            } else {
                "unica.mxl.validate failed in native spreadsheet validator".to_string()
            },
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: validation_errors,
            artifacts: vec![artifact.display().to_string()],
            stdout: Some(stdout),
            stderr: Some(String::new()),
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.mxl.validate failed in native spreadsheet validator".to_string(),
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

pub(crate) fn decompile_mxl(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    const NS_D: &str = "http://v8.1c.ru/8.2/data/spreadsheet";
    const NS_V8: &str = "http://v8.1c.ru/8.1/data/core";

    let result = (|| -> Result<(String, String, PathBuf), String> {
        let template_path = resolve_mxl_decompile_path(args, context)?;
        if !template_path.is_file() {
            // Report the selector the caller gave, not the absolutised path:
            // a logical call never reaches here — the selector refuses an
            // absent resource before this point — so the raw argument is the
            // only thing worth echoing back.
            let shown = path_arg(args, &["templatePath", "TemplatePath", "path", "Path"])
                .unwrap_or_else(|| template_path.clone());
            return Err(format!("File not found: {}", shown.display()));
        }
        let text = read_utf8_sig(&template_path)?;
        let doc = Document::parse(text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("XML parse error in {}: {err}", template_path.display()))?;
        let root = doc.root_element();
        require_mxl_document_root(root)?;

        let raw_fonts = mxl_decompile_fonts(root, NS_D);
        let raw_lines = mxl_decompile_lines(root, NS_D);
        let raw_formats = mxl_decompile_formats(root, NS_D, NS_V8);

        let columns_node = mxl_child(root, "columns", Some(NS_D));
        let total_columns = columns_node
            .map(|node| mxl_int_child(node, "size", Some(NS_D)))
            .unwrap_or(0);
        let mut col_format_indices = BTreeMap::<i64, i64>::new();
        if let Some(columns_node) = columns_node {
            for item in mxl_direct_children(columns_node, "columnsItem", Some(NS_D)) {
                let col_idx = mxl_int_child(item, "index", Some(NS_D));
                let fmt_idx = mxl_child(item, "column", Some(NS_D))
                    .map(|column| mxl_int_child(column, "formatIndex", Some(NS_D)))
                    .unwrap_or(0);
                col_format_indices.insert(col_idx, fmt_idx);
            }
        }

        let default_fmt_idx =
            mxl_optional_int_child(root, "defaultFormatIndex", Some(NS_D)).unwrap_or(0);
        let mut default_width = 10;
        if let Some(format) = mxl_decompile_format(&raw_formats, default_fmt_idx) {
            if format.width > 0 {
                default_width = format.width;
            }
        }

        let mut col_width_map = BTreeMap::<i64, i64>::new();
        for (col0, fmt_idx) in &col_format_indices {
            if let Some(format) = mxl_decompile_format(&raw_formats, *fmt_idx) {
                if format.width > 0 && format.width != default_width {
                    col_width_map.insert(*col0 + 1, format.width);
                }
            }
        }

        let mut merge_map = BTreeMap::<(i64, i64), (i64, i64)>::new();
        for merge in mxl_direct_children(root, "merge", Some(NS_D)) {
            let row = mxl_int_child(merge, "r", Some(NS_D));
            let col = mxl_int_child(merge, "c", Some(NS_D));
            let width = mxl_int_child(merge, "w", Some(NS_D));
            let height = mxl_optional_int_child(merge, "h", Some(NS_D)).unwrap_or(0);
            merge_map.insert((row, col), (width, height));
        }

        let mut named_areas = Vec::<MxlNamedItem>::new();
        for named in mxl_direct_children(root, "namedItem", Some(NS_D)) {
            if attribute_by_local_name(named, "type").unwrap_or("") != "NamedItemCells" {
                continue;
            }
            let Some(area) = mxl_child(named, "area", Some(NS_D)) else {
                continue;
            };
            if mxl_child_text(area, "type", Some(NS_D)) != "Rows" {
                continue;
            }
            named_areas.push(MxlNamedItem {
                name: mxl_child_text(named, "name", Some(NS_D)),
                begin_row: mxl_int_child(area, "beginRow", Some(NS_D)),
                end_row: mxl_int_child(area, "endRow", Some(NS_D)),
            });
        }

        let row_nodes = mxl_direct_children(root, "rowsItem", Some(NS_D));
        let is_empty_sentinel = is_platform_empty_mxl_sentinel(root, &row_nodes);
        let logical_height = mxl_logical_height(root, &row_nodes);
        let mut row_data = BTreeMap::<i64, MxlDecompiledRow>::new();
        for row_item in row_nodes {
            if is_empty_sentinel {
                break;
            }
            let row_idx = mxl_int_child(row_item, "index", Some(NS_D));
            let index_to =
                mxl_optional_int_child(row_item, "indexTo", Some(NS_D)).unwrap_or(row_idx);
            let Some(row_node) = mxl_child(row_item, "row", Some(NS_D)) else {
                continue;
            };
            let row_fmt_idx =
                mxl_optional_int_child(row_node, "formatIndex", Some(NS_D)).unwrap_or(0);
            let is_empty = mxl_child_text(row_node, "empty", Some(NS_D)) == "true";
            let mut cells = Vec::<MxlDecompiledCell>::new();
            if !is_empty {
                let mut col = -1;
                for c_group in mxl_direct_children(row_node, "c", Some(NS_D)) {
                    if let Some(idx) = mxl_optional_int_child(c_group, "i", Some(NS_D)) {
                        col = idx;
                    } else {
                        col += 1;
                    }
                    let Some(cell) = mxl_child(c_group, "c", Some(NS_D)) else {
                        continue;
                    };
                    let format_idx = mxl_optional_int_child(cell, "f", Some(NS_D)).unwrap_or(0);
                    let param = non_empty_string(mxl_child_text(cell, "parameter", Some(NS_D)));
                    let detail =
                        non_empty_string(mxl_child_text(cell, "detailParameter", Some(NS_D)));
                    let text = mxl_child(cell, "tl", Some(NS_D)).and_then(|tl| {
                        tl.descendants()
                            .find(|node| role_info_element(*node, "content", Some(NS_V8)))
                            .and_then(|node| node.text())
                            .filter(|value| !value.is_empty())
                            .map(ToOwned::to_owned)
                    });
                    cells.push(MxlDecompiledCell {
                        col,
                        format_idx,
                        param,
                        detail,
                        text,
                    });
                }
            }
            for row in row_idx..=index_to {
                row_data.insert(
                    row,
                    MxlDecompiledRow {
                        format_idx: row_fmt_idx,
                        cells: cells.clone(),
                        empty: is_empty,
                    },
                );
            }
        }

        let (font_names, font_defs) = mxl_decompile_name_fonts(&raw_fonts);
        let (style_names, mut style_defs, format_to_style_key) =
            mxl_decompile_styles(&row_data, &raw_formats, &raw_lines, &font_names);
        let areas = mxl_decompile_areas(
            &named_areas,
            &row_data,
            &raw_formats,
            &raw_lines,
            &merge_map,
            &style_names,
            &format_to_style_key,
        );
        let column_widths = mxl_decompile_compress_widths(&col_width_map);

        if style_defs
            .iter()
            .any(|(name, props)| name == "default" && ordered_json_is_empty_object(props))
        {
            style_defs.retain(|(name, _)| name != "default");
        }
        let used_styles = mxl_decompile_used_styles(&areas);
        style_defs.retain(|(name, _)| used_styles.contains(name));
        let style_count = style_defs.len();

        let mut result_fields = vec![
            ("columns".to_string(), OrderedJson::Int(total_columns)),
            ("defaultWidth".to_string(), OrderedJson::Int(default_width)),
        ];
        if !column_widths.is_empty() {
            result_fields.push(("columnWidths".to_string(), OrderedJson::Obj(column_widths)));
        }
        result_fields.push(("fonts".to_string(), OrderedJson::Obj(font_defs)));
        result_fields.push(("styles".to_string(), OrderedJson::Obj(style_defs)));
        result_fields.push(("areas".to_string(), OrderedJson::Arr(areas)));
        let json_text = render_ordered_json(&OrderedJson::Obj(result_fields));

        let stdout = format!("{json_text}\n");
        let stderr = format!(
            "     Areas: {}, Rows: {logical_height}, Columns: {total_columns}\n     Fonts: {}, Styles: {}, Merges: {}\n",
            named_areas.len(),
            raw_fonts.len(),
            style_count,
            merge_map.len()
        );

        Ok((stdout, stderr, template_path))
    })();

    match result {
        Ok((stdout, stderr, template_path)) => {
            let artifacts = vec![template_path.display().to_string()];
            AdapterOutcome {
                ok: true,
                summary: "unica.mxl.decompile completed with native spreadsheet decompiler"
                    .to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts,
                stdout: Some(stdout),
                stderr: Some(stderr),
                command: None,
            }
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.mxl.decompile failed in native spreadsheet decompiler".to_string(),
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

pub(crate) fn finish_mxl_validation(
    mut report: MxlValidationReporter,
    detailed: bool,
    template_display_name: &str,
    template_path: PathBuf,
) -> (bool, String, PathBuf, Vec<String>) {
    let checks = report.ok_count + report.errors + report.warnings;
    let stdout = if report.errors == 0 && report.warnings == 0 && !detailed {
        format!("=== Validation OK: Template.{template_display_name} ({checks} checks) ===\n")
    } else {
        report.lines.push(String::new());
        report.lines.push(format!(
            "=== Result: {} errors, {} warnings ({checks} checks) ===",
            report.errors, report.warnings
        ));
        format!("{}\n", report.lines.join("\n"))
    };
    let ok = report.errors == 0;
    let validation_errors = report
        .lines
        .iter()
        .filter(|line| line.starts_with("[ERROR] "))
        .cloned()
        .collect::<Vec<_>>();
    (ok, stdout, template_path, validation_errors)
}

pub(crate) fn is_platform_empty_mxl_sentinel(
    root: roxmltree::Node<'_, '_>,
    row_nodes: &[roxmltree::Node<'_, '_>],
) -> bool {
    let root_children = root
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    let expected_root_children = [
        "languageSettings",
        "columns",
        "rowsItem",
        "templateMode",
        "vgRows",
    ];
    if row_nodes.len() != 1
        || root_children.len() != expected_root_children.len()
        || !root_children
            .iter()
            .zip(expected_root_children)
            .all(|(node, expected_name)| {
                node.tag_name().name() == expected_name
                    && node.tag_name().namespace() == Some(MXL_DOCUMENT_NS)
            })
    {
        return false;
    }

    let column_children = root_children[1]
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    if column_children.len() != 1
        || column_children[0].tag_name().name() != "size"
        || column_children[0].tag_name().namespace() != Some(MXL_DOCUMENT_NS)
        || column_children[0]
            .text()
            .and_then(|value| value.trim().parse::<i64>().ok())
            != Some(0)
        || root_children[3]
            .text()
            .is_none_or(|value| value.trim() != "true")
        || root_children[4]
            .text()
            .and_then(|value| value.trim().parse::<i64>().ok())
            != Some(0)
    {
        return false;
    }

    let row_item_children = root_children[2]
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    if row_item_children.len() != 2
        || row_item_children[0].tag_name().name() != "index"
        || row_item_children[0].tag_name().namespace() != Some(MXL_DOCUMENT_NS)
        || row_item_children[0]
            .text()
            .and_then(|value| value.trim().parse::<i64>().ok())
            != Some(0)
        || row_item_children[1].tag_name().name() != "row"
        || row_item_children[1].tag_name().namespace() != Some(MXL_DOCUMENT_NS)
    {
        return false;
    }

    let row = row_item_children[1];
    let element_children = row
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    element_children.len() == 1
        && element_children[0].tag_name().name() == "empty"
        && element_children[0].tag_name().namespace() == Some(MXL_DOCUMENT_NS)
        && element_children[0]
            .text()
            .is_some_and(|value| value.trim() == "true")
}

pub(crate) fn mxl_logical_height(
    root: roxmltree::Node<'_, '_>,
    row_nodes: &[roxmltree::Node<'_, '_>],
) -> i64 {
    if is_platform_empty_mxl_sentinel(root, row_nodes) {
        return 0;
    }

    let declared_height = child_text(root, "height", None).parse::<i64>().unwrap_or(0);
    let represented_height = row_nodes
        .iter()
        .filter_map(|row_item| {
            let index = child_text(*row_item, "index", None).parse::<i64>().ok()?;
            let index_to = child_text(*row_item, "indexTo", None)
                .parse::<i64>()
                .unwrap_or(index);
            Some(index.max(index_to) + 1)
        })
        .max()
        .unwrap_or(row_nodes.len() as i64);

    declared_height
        .max(represented_height)
        .max(row_nodes.len() as i64)
        .max(1)
}

pub(crate) fn mxl_decompile_fonts(root: roxmltree::Node<'_, '_>, ns: &str) -> Vec<MxlRawFont> {
    mxl_direct_children(root, "font", Some(ns))
        .into_iter()
        .map(|font| MxlRawFont {
            face: font.attribute("faceName").unwrap_or("").to_string(),
            size: font
                .attribute("height")
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(0),
            bold: font.attribute("bold") == Some("true"),
            italic: font.attribute("italic") == Some("true"),
            underline: font.attribute("underline") == Some("true"),
            strikeout: font.attribute("strikeout") == Some("true"),
        })
        .collect()
}

pub(crate) fn mxl_decompile_lines(root: roxmltree::Node<'_, '_>, ns: &str) -> Vec<i64> {
    mxl_direct_children(root, "line", Some(ns))
        .into_iter()
        .map(|line| {
            line.attribute("width")
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(0)
        })
        .collect()
}

pub(crate) fn mxl_decompile_formats(
    root: roxmltree::Node<'_, '_>,
    ns: &str,
    v8_ns: &str,
) -> Vec<MxlRawFormat> {
    mxl_direct_children(root, "format", Some(ns))
        .into_iter()
        .map(|format| {
            let data_format = mxl_child(format, "format", Some(ns))
                .and_then(|node| {
                    node.descendants()
                        .find(|child| role_info_element(*child, "content", Some(v8_ns)))
                })
                .and_then(|node| node.text())
                .unwrap_or("")
                .to_string();
            MxlRawFormat {
                font_idx: mxl_optional_int_child(format, "font", Some(ns)).unwrap_or(-1),
                lb: mxl_optional_int_child(format, "leftBorder", Some(ns)).unwrap_or(-1),
                tb: mxl_optional_int_child(format, "topBorder", Some(ns)).unwrap_or(-1),
                rb: mxl_optional_int_child(format, "rightBorder", Some(ns)).unwrap_or(-1),
                bb: mxl_optional_int_child(format, "bottomBorder", Some(ns)).unwrap_or(-1),
                width: mxl_optional_int_child(format, "width", Some(ns)).unwrap_or(0),
                height: mxl_optional_int_child(format, "height", Some(ns)).unwrap_or(0),
                ha: mxl_child_text(format, "horizontalAlignment", Some(ns)),
                va: mxl_child_text(format, "verticalAlignment", Some(ns)),
                wrap: mxl_child_text(format, "textPlacement", Some(ns)) == "Wrap",
                fill_type: mxl_child_text(format, "fillType", Some(ns)),
                data_format,
            }
        })
        .collect()
}

pub(crate) fn mxl_decompile_format(formats: &[MxlRawFormat], idx: i64) -> Option<&MxlRawFormat> {
    if idx <= 0 || idx as usize > formats.len() {
        return None;
    }
    formats.get(idx as usize - 1)
}

pub(crate) fn mxl_decompile_name_fonts(
    raw_fonts: &[MxlRawFont],
) -> (BTreeMap<i64, String>, Vec<(String, OrderedJson)>) {
    let mut font_names = BTreeMap::<i64, String>::new();
    let mut font_defs = Vec::<(String, OrderedJson)>::new();
    if raw_fonts.is_empty() {
        return (font_names, font_defs);
    }

    font_names.insert(0, "default".to_string());
    font_defs.push((
        "default".to_string(),
        mxl_decompile_font_json(&raw_fonts[0]),
    ));
    let mut font_key_map = BTreeMap::<String, String>::new();
    font_key_map.insert(mxl_decompile_font_key(&raw_fonts[0]), "default".to_string());

    for (idx, font) in raw_fonts.iter().enumerate().skip(1) {
        let key = mxl_decompile_font_key(font);
        if let Some(existing) = font_key_map.get(&key) {
            font_names.insert(idx as i64, existing.clone());
            continue;
        }

        let default = &raw_fonts[0];
        let mut name = if font.face == default.face && font.size == default.size {
            if font.bold && !default.bold && !font.italic && !font.underline && !font.strikeout {
                "bold".to_string()
            } else if font.italic && !default.italic && !font.bold {
                "italic".to_string()
            } else if font.underline && !default.underline && !font.bold && !font.italic {
                "underline".to_string()
            } else {
                String::new()
            }
        } else if font.face == default.face && font.size > default.size && font.bold {
            "header".to_string()
        } else if font.face == default.face && font.size < default.size {
            "small".to_string()
        } else {
            String::new()
        };

        if name.is_empty() {
            let mut parts = Vec::<String>::new();
            if !font.face.is_empty() && font.face != default.face {
                parts.push(font.face.to_lowercase());
            }
            parts.push(font.size.to_string());
            if font.bold {
                parts.push("bold".to_string());
            }
            if font.italic {
                parts.push("italic".to_string());
            }
            if font.underline {
                parts.push("underline".to_string());
            }
            if font.strikeout {
                parts.push("strikeout".to_string());
            }
            name = parts.join("-");
        }

        let base_name = name.clone();
        let mut suffix = 2;
        while font_defs.iter().any(|(existing, _)| existing == &name) {
            name = format!("{base_name}{suffix}");
            suffix += 1;
        }

        font_names.insert(idx as i64, name.clone());
        font_defs.push((name.clone(), mxl_decompile_font_json(font)));
        font_key_map.insert(key, name);
    }

    (font_names, font_defs)
}

pub(crate) fn mxl_decompile_font_key(font: &MxlRawFont) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}",
        font.face, font.size, font.bold, font.italic, font.underline, font.strikeout
    )
}

pub(crate) fn mxl_decompile_font_json(font: &MxlRawFont) -> OrderedJson {
    let mut fields = vec![
        ("face".to_string(), OrderedJson::Str(font.face.clone())),
        ("size".to_string(), OrderedJson::Int(font.size)),
    ];
    if font.bold {
        fields.push(("bold".to_string(), OrderedJson::Bool(true)));
    }
    if font.italic {
        fields.push(("italic".to_string(), OrderedJson::Bool(true)));
    }
    if font.underline {
        fields.push(("underline".to_string(), OrderedJson::Bool(true)));
    }
    if font.strikeout {
        fields.push(("strikeout".to_string(), OrderedJson::Bool(true)));
    }
    OrderedJson::Obj(fields)
}

pub(crate) fn mxl_decompile_styles(
    row_data: &BTreeMap<i64, MxlDecompiledRow>,
    raw_formats: &[MxlRawFormat],
    raw_lines: &[i64],
    font_names: &BTreeMap<i64, String>,
) -> MxlDecompileStyleResult {
    let mut style_keys = Vec::<(String, MxlRawFormat)>::new();
    let mut format_to_style_key = BTreeMap::<i64, String>::new();
    for row in row_data.values() {
        for cell in &row.cells {
            let Some(format) = mxl_decompile_format(raw_formats, cell.format_idx) else {
                continue;
            };
            let key = mxl_decompile_style_key(Some(format), raw_lines);
            if !style_keys.iter().any(|(existing, _)| existing == &key) {
                style_keys.push((key.clone(), format.clone()));
            }
            format_to_style_key.insert(cell.format_idx, key);
        }
    }

    let mut style_names = BTreeMap::<String, String>::new();
    let mut style_defs = Vec::<(String, OrderedJson)>::new();
    for (key, format) in style_keys {
        let mut name = mxl_decompile_style_name(&format, raw_lines, font_names);
        let base_name = name.clone();
        let mut suffix = 2;
        while style_defs.iter().any(|(existing, _)| existing == &name) {
            name = format!("{base_name}{suffix}");
            suffix += 1;
        }
        style_names.insert(key, name.clone());
        style_defs.push((
            name,
            mxl_decompile_style_json(&format, raw_lines, font_names),
        ));
    }
    (style_names, style_defs, format_to_style_key)
}

pub(crate) fn mxl_decompile_style_key(format: Option<&MxlRawFormat>, raw_lines: &[i64]) -> String {
    let Some(format) = format else {
        return "empty".to_string();
    };
    let font_idx = if format.font_idx >= 0 {
        format.font_idx
    } else {
        0
    };
    let (border, thick) = mxl_decompile_border_desc(format, raw_lines);
    format!(
        "f={font_idx}|b={border}|bw={thick}|ha={}|va={}|wr={}|df={}",
        format.ha, format.va, format.wrap, format.data_format
    )
}

pub(crate) fn mxl_decompile_border_desc(
    format: &MxlRawFormat,
    raw_lines: &[i64],
) -> (String, bool) {
    let lb = format.lb >= 0;
    let tb = format.tb >= 0;
    let rb = format.rb >= 0;
    let bb = format.bb >= 0;
    if !lb && !tb && !rb && !bb {
        return ("none".to_string(), false);
    }
    let thick = [format.lb, format.tb, format.rb, format.bb]
        .iter()
        .any(|idx| *idx >= 0 && raw_lines.get(*idx as usize).copied().unwrap_or(0) >= 2);
    if lb && tb && rb && bb {
        return ("all".to_string(), thick);
    }
    let mut sides = Vec::<&str>::new();
    if tb {
        sides.push("top");
    }
    if bb {
        sides.push("bottom");
    }
    if lb {
        sides.push("left");
    }
    if rb {
        sides.push("right");
    }
    (sides.join(","), thick)
}

pub(crate) fn mxl_decompile_style_name(
    format: &MxlRawFormat,
    raw_lines: &[i64],
    font_names: &BTreeMap<i64, String>,
) -> String {
    let mut parts = Vec::<String>::new();
    let font_idx = if format.font_idx >= 0 {
        format.font_idx
    } else {
        0
    };
    if let Some(font_name) = font_names.get(&font_idx) {
        if font_name != "default" {
            parts.push(font_name.clone());
        }
    }
    let (border, thick) = mxl_decompile_border_desc(format, raw_lines);
    if border != "none" {
        if border == "all" {
            parts.push("bordered".to_string());
        } else {
            parts.push(format!("border-{border}"));
        }
    }
    match format.ha.as_str() {
        "Center" => parts.push("center".to_string()),
        "Right" => parts.push("right".to_string()),
        _ => {}
    }
    match format.va.as_str() {
        "Center" => parts.push("vcenter".to_string()),
        "Top" => parts.push("vtop".to_string()),
        _ => {}
    }
    if format.wrap {
        parts.push("wrap".to_string());
    }
    if !format.data_format.is_empty() {
        parts.push("fmt".to_string());
    }
    if parts.is_empty() {
        "default".to_string()
    } else {
        let mut name = parts.join("-");
        if thick && !name.contains("bordered") && !name.contains("border-") {
            name.push_str("-thick");
        }
        name
    }
}

pub(crate) fn mxl_decompile_style_json(
    format: &MxlRawFormat,
    raw_lines: &[i64],
    font_names: &BTreeMap<i64, String>,
) -> OrderedJson {
    let mut fields = Vec::<(String, OrderedJson)>::new();
    let font_idx = if format.font_idx >= 0 {
        format.font_idx
    } else {
        0
    };
    if let Some(font_name) = font_names.get(&font_idx) {
        if font_name != "default" {
            fields.push(("font".to_string(), OrderedJson::Str(font_name.clone())));
        }
    }
    match format.ha.as_str() {
        "Left" => fields.push(("align".to_string(), OrderedJson::Str("left".to_string()))),
        "Center" => fields.push(("align".to_string(), OrderedJson::Str("center".to_string()))),
        "Right" => fields.push(("align".to_string(), OrderedJson::Str("right".to_string()))),
        _ => {}
    }
    match format.va.as_str() {
        "Top" => fields.push(("valign".to_string(), OrderedJson::Str("top".to_string()))),
        "Center" => fields.push(("valign".to_string(), OrderedJson::Str("center".to_string()))),
        _ => {}
    }
    let (border, thick) = mxl_decompile_border_desc(format, raw_lines);
    if border != "none" {
        fields.push(("border".to_string(), OrderedJson::Str(border)));
        if thick {
            fields.push((
                "borderWidth".to_string(),
                OrderedJson::Str("thick".to_string()),
            ));
        }
    }
    if format.wrap {
        fields.push(("wrap".to_string(), OrderedJson::Bool(true)));
    }
    if !format.data_format.is_empty() {
        fields.push((
            "format".to_string(),
            OrderedJson::Str(format.data_format.clone()),
        ));
    }
    OrderedJson::Obj(fields)
}

pub(crate) fn mxl_decompile_areas(
    named_areas: &[MxlNamedItem],
    row_data: &BTreeMap<i64, MxlDecompiledRow>,
    raw_formats: &[MxlRawFormat],
    raw_lines: &[i64],
    merge_map: &BTreeMap<(i64, i64), (i64, i64)>,
    style_names: &BTreeMap<String, String>,
    format_to_style_key: &BTreeMap<i64, String>,
) -> Vec<OrderedJson> {
    let mut areas = Vec::<OrderedJson>::new();
    for area in named_areas {
        let mut area_rows = Vec::<OrderedJson>::new();
        for global_row in area.begin_row..=area.end_row {
            let Some(row) = row_data.get(&global_row) else {
                area_rows.push(OrderedJson::Obj(Vec::new()));
                continue;
            };
            if row.empty {
                area_rows.push(OrderedJson::Obj(Vec::new()));
                continue;
            }

            let mut row_fields = Vec::<(String, OrderedJson)>::new();
            if row.format_idx > 0 {
                if let Some(row_format) = mxl_decompile_format(raw_formats, row.format_idx) {
                    if row_format.height > 0 {
                        row_fields
                            .push(("height".to_string(), OrderedJson::Int(row_format.height)));
                    }
                }
            }

            let mut content_cells = Vec::<MxlDecompiledCell>::new();
            let mut gap_cells = Vec::<MxlDecompiledCell>::new();
            for cell in &row.cells {
                let has_content = cell.param.is_some() || cell.text.is_some();
                let has_merge = merge_map.contains_key(&(global_row, cell.col));
                if has_content || has_merge {
                    content_cells.push(cell.clone());
                } else {
                    gap_cells.push(cell.clone());
                }
            }

            let mut row_style_key = None::<String>;
            let mut row_style_name = None::<String>;
            if !gap_cells.is_empty() {
                let mut gap_keys = Vec::<String>::new();
                for cell in &gap_cells {
                    let key = mxl_decompile_style_key(
                        mxl_decompile_format(raw_formats, cell.format_idx),
                        raw_lines,
                    );
                    if !gap_keys.iter().any(|existing| existing == &key) {
                        gap_keys.push(key);
                    }
                }
                if gap_keys.len() == 1 {
                    let key = gap_keys.remove(0);
                    if let Some(name) = style_names.get(&key) {
                        row_style_key = Some(key);
                        row_style_name = Some(name.clone());
                    }
                }
            }
            if let Some(name) = &row_style_name {
                if name != "default" {
                    row_fields.push(("rowStyle".to_string(), OrderedJson::Str(name.clone())));
                }
            }

            content_cells.sort_by_key(|cell| cell.col);
            let mut dsl_cells = Vec::<OrderedJson>::new();
            for cell in content_cells {
                let mut cell_fields = vec![("col".to_string(), OrderedJson::Int(cell.col + 1))];
                if let Some((width, height)) = merge_map.get(&(global_row, cell.col)) {
                    if *width > 0 {
                        cell_fields.push(("span".to_string(), OrderedJson::Int(width + 1)));
                    }
                    if *height > 0 {
                        cell_fields.push(("rowspan".to_string(), OrderedJson::Int(height + 1)));
                    }
                }

                let cell_format = mxl_decompile_format(raw_formats, cell.format_idx);
                let cell_style_key = mxl_decompile_style_key(cell_format, raw_lines);
                if row_style_key.as_deref() != Some(cell_style_key.as_str()) {
                    let style_name = mxl_decompile_style_name_for_format(
                        cell.format_idx,
                        format_to_style_key,
                        style_names,
                    );
                    if style_name != "default" || row_style_name.is_none() {
                        cell_fields.push(("style".to_string(), OrderedJson::Str(style_name)));
                    }
                }

                let fill_type = cell_format
                    .map(|format| format.fill_type.as_str())
                    .unwrap_or("");
                if let Some(param) = cell.param {
                    cell_fields.push(("param".to_string(), OrderedJson::Str(param)));
                    if let Some(detail) = cell.detail {
                        cell_fields.push(("detail".to_string(), OrderedJson::Str(detail)));
                    }
                } else if fill_type == "Template" {
                    if let Some(text) = cell.text {
                        cell_fields.push(("template".to_string(), OrderedJson::Str(text)));
                    }
                } else if let Some(text) = cell.text {
                    cell_fields.push(("text".to_string(), OrderedJson::Str(text)));
                }
                dsl_cells.push(OrderedJson::Obj(cell_fields));
            }

            if !dsl_cells.is_empty() {
                row_fields.push(("cells".to_string(), OrderedJson::Arr(dsl_cells)));
            }
            area_rows.push(OrderedJson::Obj(row_fields));
        }

        let compressed_rows = mxl_decompile_compress_empty_rows(area_rows);
        areas.push(OrderedJson::Obj(vec![
            ("name".to_string(), OrderedJson::Str(area.name.clone())),
            ("rows".to_string(), OrderedJson::Arr(compressed_rows)),
        ]));
    }
    areas
}

pub(crate) fn mxl_decompile_style_name_for_format(
    format_idx: i64,
    format_to_style_key: &BTreeMap<i64, String>,
    style_names: &BTreeMap<String, String>,
) -> String {
    format_to_style_key
        .get(&format_idx)
        .and_then(|key| style_names.get(key))
        .cloned()
        .unwrap_or_else(|| "default".to_string())
}

pub(crate) fn mxl_decompile_compress_empty_rows(rows: Vec<OrderedJson>) -> Vec<OrderedJson> {
    let mut result = Vec::<OrderedJson>::new();
    let mut empty_run = 0i64;
    for row in rows {
        if matches!(&row, OrderedJson::Obj(fields) if fields.is_empty()) {
            empty_run += 1;
        } else {
            if empty_run > 0 {
                if empty_run == 1 {
                    result.push(OrderedJson::Obj(Vec::new()));
                } else {
                    result.push(OrderedJson::Obj(vec![(
                        "empty".to_string(),
                        OrderedJson::Int(empty_run),
                    )]));
                }
                empty_run = 0;
            }
            result.push(row);
        }
    }
    if empty_run > 0 {
        if empty_run == 1 {
            result.push(OrderedJson::Obj(Vec::new()));
        } else {
            result.push(OrderedJson::Obj(vec![(
                "empty".to_string(),
                OrderedJson::Int(empty_run),
            )]));
        }
    }
    result
}

pub(crate) fn mxl_decompile_compress_widths(
    widths: &BTreeMap<i64, i64>,
) -> Vec<(String, OrderedJson)> {
    let mut by_width = Vec::<(i64, Vec<i64>)>::new();
    for (col, width) in widths {
        if let Some((_, cols)) = by_width.iter_mut().find(|(existing, _)| existing == width) {
            cols.push(*col);
        } else {
            by_width.push((*width, vec![*col]));
        }
    }

    let mut result = Vec::<(String, OrderedJson)>::new();
    for (width, mut cols) in by_width {
        cols.sort();
        if cols.is_empty() {
            continue;
        }
        let mut range_start = cols[0];
        let mut range_prev = cols[0];
        for col in cols.iter().skip(1) {
            if *col == range_prev + 1 {
                range_prev = *col;
            } else {
                if range_start == range_prev {
                    result.push((range_start.to_string(), OrderedJson::Int(width)));
                } else {
                    result.push((
                        format!("{range_start}-{range_prev}"),
                        OrderedJson::Int(width),
                    ));
                }
                range_start = *col;
                range_prev = *col;
            }
        }
        if range_start == range_prev {
            result.push((range_start.to_string(), OrderedJson::Int(width)));
        } else {
            result.push((
                format!("{range_start}-{range_prev}"),
                OrderedJson::Int(width),
            ));
        }
    }
    result
}

pub(crate) fn mxl_decompile_used_styles(areas: &[OrderedJson]) -> HashSet<String> {
    let mut result = HashSet::<String>::new();
    for area in areas {
        if let OrderedJson::Obj(area_fields) = area {
            if let Some(OrderedJson::Arr(rows)) = area_fields
                .iter()
                .find(|(key, _)| key == "rows")
                .map(|(_, value)| value)
            {
                for row in rows {
                    if let OrderedJson::Obj(row_fields) = row {
                        if let Some(OrderedJson::Str(style)) = row_fields
                            .iter()
                            .find(|(key, _)| key == "rowStyle")
                            .map(|(_, value)| value)
                        {
                            result.insert(style.clone());
                        }
                        if let Some(OrderedJson::Arr(cells)) = row_fields
                            .iter()
                            .find(|(key, _)| key == "cells")
                            .map(|(_, value)| value)
                        {
                            for cell in cells {
                                if let OrderedJson::Obj(cell_fields) = cell {
                                    if let Some(OrderedJson::Str(style)) = cell_fields
                                        .iter()
                                        .find(|(key, _)| key == "style")
                                        .map(|(_, value)| value)
                                    {
                                        result.insert(style.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    result
}

pub(crate) fn render_ordered_json(value: &OrderedJson) -> String {
    render_ordered_json_at(value, 0)
}

pub(crate) fn ordered_json_is_empty_object(value: &OrderedJson) -> bool {
    matches!(value, OrderedJson::Obj(fields) if fields.is_empty())
}

pub(crate) fn render_ordered_json_at(value: &OrderedJson, indent: usize) -> String {
    match value {
        OrderedJson::Obj(fields) => {
            if fields.is_empty() {
                return "{}".to_string();
            }
            let child_indent = indent + 2;
            let mut out = String::from("{\n");
            for (idx, (key, child)) in fields.iter().enumerate() {
                out.push_str(&" ".repeat(child_indent));
                out.push_str(&serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string()));
                out.push_str(": ");
                out.push_str(&render_ordered_json_at(child, child_indent));
                if idx + 1 != fields.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&" ".repeat(indent));
            out.push('}');
            out
        }
        OrderedJson::Arr(items) => {
            if items.is_empty() {
                return "[]".to_string();
            }
            let child_indent = indent + 2;
            let mut out = String::from("[\n");
            for (idx, child) in items.iter().enumerate() {
                out.push_str(&" ".repeat(child_indent));
                out.push_str(&render_ordered_json_at(child, child_indent));
                if idx + 1 != items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&" ".repeat(indent));
            out.push(']');
            out
        }
        OrderedJson::Str(value) => {
            serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
        }
        OrderedJson::Int(value) => value.to_string(),
        OrderedJson::Bool(value) => {
            if *value {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
    }
}

pub(crate) fn non_empty_string(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

/// The address kinds a template reader accepts. `Report.X.Template.Y` names a
/// nested template; `CommonTemplate.Y` names a top-level one.
pub(crate) const TEMPLATE_KINDS: &[&str] = &["Template", "CommonTemplate"];

/// `unica.mxl.decompile` reads its input the way the other template readers do;
/// only its output is a file it writes.
pub(crate) fn resolve_mxl_decompile_path(
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
    let raw = required_path(
        args,
        &["templatePath", "TemplatePath", "path", "Path"],
        "TemplatePath",
    )?;
    Ok(absolutize(raw, &context.cwd))
}

/// `unica.mxl.info` publishes two addresses: the logical selector and
/// `TemplatePath`. The composite `ProcessorName`/`TemplateName`/`SrcDir` form
/// guessed a physical layout and could never be reached past the schema, so it
/// is gone rather than left as an unreachable branch.
///
/// The physical address must still accept both documented forms — the
/// descriptor file and the layout directory holding it — the way
/// `resolve_mxl_validate_path` already does.
pub(crate) fn resolve_mxl_info_path(
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
    let path = path_arg(args, &["templatePath", "TemplatePath", "path", "Path"])
        .ok_or_else(|| "Specify -TemplatePath".to_string())?;
    let path = absolutize(path, &context.cwd);
    Ok(if path.is_dir() {
        path.join("Ext").join("Template.xml")
    } else {
        path
    })
}

pub(crate) fn resolve_mxl_validate_path(
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
    let mut template_path = if let Some(path) =
        path_arg(args, &["templatePath", "TemplatePath", "path", "Path"])
    {
        path
    } else {
        let processor_name =
            required_string(args, &["processorName", "ProcessorName"], "ProcessorName")?;
        let template_name =
            required_string(args, &["templateName", "TemplateName"], "TemplateName")?;
        let src_dir = path_arg(args, &["srcDir", "SrcDir"]).unwrap_or_else(|| PathBuf::from("src"));
        src_dir
            .join(processor_name)
            .join("Templates")
            .join(template_name)
            .join("Ext")
            .join("Template.xml")
    };

    template_path = absolutize(template_path, &context.cwd);

    if template_path.is_dir() {
        template_path = template_path.join("Ext").join("Template.xml");
    }
    if !template_path.exists()
        && template_path.file_name().and_then(|value| value.to_str()) == Some("Template.xml")
    {
        let candidate = template_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("Ext")
            .join("Template.xml");
        if candidate.exists() {
            template_path = candidate;
        }
    }
    if !template_path.exists()
        && template_path
            .extension()
            .and_then(|value| value.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("xml"))
            .unwrap_or(false)
    {
        if let Some(stem) = template_path.file_stem().and_then(|value| value.to_str()) {
            let candidate = template_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(stem)
                .join("Ext")
                .join("Template.xml");
            if candidate.exists() {
                template_path = candidate;
            }
        }
    }

    if !template_path.exists() {
        return Err(format!("File not found: {}", template_path.display()));
    }

    Ok(template_path)
}

pub(crate) fn mxl_direct_children<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
    namespace: Option<&str>,
) -> Vec<roxmltree::Node<'a, 'input>> {
    node.children()
        .filter(|child| role_info_element(*child, local_name, namespace))
        .collect()
}

pub(crate) fn mxl_child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
    namespace: Option<&str>,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children()
        .find(|child| role_info_element(*child, local_name, namespace))
}

pub(crate) fn mxl_child_text(
    node: roxmltree::Node<'_, '_>,
    local_name: &str,
    namespace: Option<&str>,
) -> String {
    mxl_child(node, local_name, namespace)
        .and_then(|child| child.text())
        .unwrap_or("")
        .to_string()
}

pub(crate) fn mxl_int_child(
    node: roxmltree::Node<'_, '_>,
    local_name: &str,
    namespace: Option<&str>,
) -> i64 {
    mxl_optional_int_child(node, local_name, namespace).unwrap_or(0)
}

pub(crate) fn mxl_optional_int_child(
    node: roxmltree::Node<'_, '_>,
    local_name: &str,
    namespace: Option<&str>,
) -> Option<i64> {
    mxl_child(node, local_name, namespace)
        .and_then(|child| child.text())
        .and_then(|text| text.parse::<i64>().ok())
}

pub(crate) fn mxl_prefix(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

pub(crate) fn mxl_area_cell_data(
    area: &MxlNamedArea,
    row_map: &[(i64, roxmltree::Node<'_, '_>)],
    doc_height: i64,
    include_text: bool,
) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let mut params = Vec::new();
    let mut details = Vec::new();
    let mut texts = Vec::new();
    let mut templates = Vec::new();
    let start_row = if area.begin_row == -1 {
        0
    } else {
        area.begin_row
    };
    let end_row = if area.end_row == -1 {
        doc_height - 1
    } else {
        area.end_row
    };
    for row in start_row..=end_row {
        if let Some((_, row_node)) = row_map.iter().find(|(idx, _)| *idx == row) {
            for cell in mxl_cell_data(*row_node, include_text) {
                match cell {
                    MxlCellData::Parameter(value, detail) => {
                        if let Some(detail) = detail {
                            details.push(format!("{value}->{detail}"));
                        }
                        params.push(value);
                    }
                    MxlCellData::TemplateParam(value) => params.push(format!("{value} [tpl]")),
                    MxlCellData::Text(value) => texts.push(value),
                    MxlCellData::Template(value) => templates.push(value),
                }
            }
        }
    }
    (params, details, texts, templates)
}

pub(crate) fn mxl_cell_data(
    row_item: roxmltree::Node<'_, '_>,
    include_text: bool,
) -> Vec<MxlCellData> {
    let Some(row) = row_item
        .children()
        .find(|node| role_info_element(*node, "row", None))
    else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for c_group in row
        .children()
        .filter(|node| role_info_element(*node, "c", None))
    {
        let Some(cell) = c_group
            .children()
            .find(|node| role_info_element(*node, "c", None))
        else {
            continue;
        };
        let parameter = child_text(cell, "parameter", None);
        let detail = child_text(cell, "detailParameter", None);
        if !parameter.is_empty() {
            result.push(MxlCellData::Parameter(
                parameter,
                (!detail.is_empty()).then_some(detail),
            ));
        }
        if let Some(tl) = cell
            .children()
            .find(|node| role_info_element(*node, "tl", None))
        {
            let content = tl
                .descendants()
                .find(|node| role_info_element(*node, "content", None))
                .and_then(|node| node.text())
                .unwrap_or("");
            if !content.is_empty() {
                let placeholders = mxl_template_placeholders(content);
                if !placeholders.is_empty() {
                    for placeholder in placeholders {
                        result.push(MxlCellData::TemplateParam(placeholder));
                    }
                    if include_text {
                        result.push(MxlCellData::Template(content.to_string()));
                    }
                } else if include_text {
                    result.push(MxlCellData::Text(content.to_string()));
                }
            }
        }
    }
    result
}

pub(crate) fn mxl_template_placeholders(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find(']') else {
            break;
        };
        let value = &after_start[..end];
        if !value.is_empty() && !value.chars().all(|ch| ch.is_ascii_digit()) {
            result.push(value.to_string());
        }
        rest = &after_start[end + 1..];
    }
    result
}

pub(crate) fn truncate_mxl_list(items: &[String], max_count: usize) -> String {
    if items.len() <= max_count {
        return items.join(", ");
    }
    let shown = items[..max_count].join(", ");
    format!("{shown}, ... (+{})", items.len() - max_count)
}

#[derive(Clone)]
pub(crate) struct MxlFontEntry {
    pub(crate) face: String,
    pub(crate) size: i64,
    pub(crate) bold: &'static str,
    pub(crate) italic: &'static str,
    pub(crate) underline: &'static str,
    pub(crate) strikeout: &'static str,
}

#[derive(Clone, Default)]
pub(crate) struct MxlFormatProps {
    pub(crate) font_idx: Option<i64>,
    pub(crate) lb: Option<i64>,
    pub(crate) tb: Option<i64>,
    pub(crate) rb: Option<i64>,
    pub(crate) bb: Option<i64>,
    pub(crate) ha: String,
    pub(crate) va: String,
    pub(crate) wrap: bool,
    pub(crate) fill_type: String,
    pub(crate) number_format: String,
    pub(crate) width: Option<i64>,
    pub(crate) height: Option<i64>,
}

pub(crate) struct MxlFormatRegistry {
    pub(crate) entries: Vec<(String, MxlFormatProps)>,
}

impl MxlFormatRegistry {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub(crate) fn register(&mut self, key: String, props: MxlFormatProps) -> usize {
        if let Some(index) = self
            .entries
            .iter()
            .position(|(existing, _)| existing == &key)
        {
            return index + 1;
        }
        self.entries.push((key, props));
        self.entries.len()
    }

    pub(crate) fn index_of(&self, key: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|(existing, _)| existing == key)
            .map(|index| index + 1)
    }
}

pub(crate) fn compile_mxl(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    let write_result = (|| -> Result<(String, PathBuf, Vec<String>), String> {
        let json_path_raw = required_path(args, &["jsonPath", "JsonPath"], "JsonPath")?;
        let output_path_raw = required_path(args, &["outputPath", "OutputPath"], "OutputPath")?;
        let json_path = absolutize(json_path_raw, &context.cwd);
        if !json_path.exists() {
            return Err(format!("File not found: {}", json_path.display()));
        }
        let mut transaction = CompileTransaction::new();
        let defn =
            FileBackedJson::read(&json_path, |err| format!("failed to parse MXL JSON: {err}"))?
                .bind_to(&mut transaction)?;

        if !truthy_json_field(&defn, "columns") {
            return Err("Required field 'columns' is missing".to_string());
        }
        if !truthy_json_field(&defn, "areas") {
            return Err("Required field 'areas' is missing".to_string());
        }

        let total_columns = json_i64_field(&defn, "columns").unwrap_or(0);
        let mut default_width = json_i64_field(&defn, "defaultWidth").unwrap_or(10);

        let mut font_map = std::collections::BTreeMap::<String, usize>::new();
        let mut font_entries = Vec::<MxlFontEntry>::new();
        let mut has_default = false;
        if let Some(fonts) = defn.get("fonts").and_then(Value::as_object) {
            for (name, font_def) in fonts {
                if name == "default" {
                    has_default = true;
                }
                add_mxl_font(name, Some(font_def), &mut font_map, &mut font_entries);
            }
        }
        if !has_default {
            add_mxl_font("default", None, &mut font_map, &mut font_entries);
        }

        let mut has_thin_borders = false;
        let mut has_thick_borders = false;
        if let Some(styles) = defn.get("styles").and_then(Value::as_object) {
            for style in styles.values() {
                let border = json_string_field(style, "border").unwrap_or_default();
                if !border.is_empty() && border != "none" {
                    if json_string_field(style, "borderWidth").as_deref() == Some("thick") {
                        has_thick_borders = true;
                    } else {
                        has_thin_borders = true;
                    }
                }
            }
        }
        let mut line_count = 0usize;
        let thin_line_index = if has_thin_borders {
            let index = line_count as i64;
            line_count += 1;
            index
        } else {
            -1
        };
        let thick_line_index = if has_thick_borders {
            let index = line_count as i64;
            line_count += 1;
            index
        } else {
            -1
        };

        let mut page_name = None::<String>;
        let mut target_width = None::<i64>;
        if let Some(page) = json_string_field(&defn, "page") {
            page_name = Some(page.clone());
            target_width = if page.chars().all(|ch| ch.is_ascii_digit()) {
                page.parse::<i64>().ok()
            } else {
                match page.as_str() {
                    "A4-landscape" => Some(780),
                    "A4-portrait" => Some(540),
                    _ => None,
                }
            };

            if let Some(target) = target_width {
                let mut total_units = 0.0f64;
                let mut absolute_sum = 0i64;
                let mut specified_cols = std::collections::BTreeMap::<i64, bool>::new();
                if let Some(widths) = defn.get("columnWidths").and_then(Value::as_object) {
                    for (spec, value) in widths {
                        let value = json_value_to_python_string(value);
                        for column in parse_mxl_column_spec(spec)? {
                            specified_cols.insert(column, true);
                            if let Some(multiplier) = value.strip_suffix('x') {
                                total_units += multiplier.parse::<f64>().unwrap_or(0.0);
                            } else {
                                absolute_sum += value.parse::<i64>().unwrap_or(0);
                            }
                        }
                    }
                }
                for column in 1..=total_columns {
                    if !specified_cols.contains_key(&column) {
                        total_units += 1.0;
                    }
                }
                if total_units > 0.0 {
                    default_width = ((target - absolute_sum) as f64 / total_units).round() as i64;
                }
            }
        }

        let mut col_width_map = std::collections::BTreeMap::<i64, i64>::new();
        if let Some(widths) = defn.get("columnWidths").and_then(Value::as_object) {
            for (spec, value) in widths {
                let value = json_value_to_python_string(value);
                let width = if let Some(multiplier) = value.strip_suffix('x') {
                    (multiplier.parse::<f64>().unwrap_or(0.0) * default_width as f64).round() as i64
                } else {
                    value.parse::<i64>().unwrap_or(0)
                };
                for column in parse_mxl_column_spec(spec)? {
                    col_width_map.insert(column, width);
                }
            }
        }

        let mut registry = MxlFormatRegistry::new();
        let mut col_format_map = std::collections::BTreeMap::<i64, usize>::new();
        for (column, width) in &col_width_map {
            let props = MxlFormatProps {
                width: Some(*width),
                ..Default::default()
            };
            let index = registry.register(mxl_format_key(&props), props);
            col_format_map.insert(*column, index);
        }

        let areas = defn
            .get("areas")
            .and_then(Value::as_array)
            .ok_or_else(|| "Required field 'areas' is missing".to_string())?;

        for area in areas {
            if let Some(rows) = area.get("rows").and_then(Value::as_array) {
                for row in rows {
                    let Some(row_object) = row.as_object() else {
                        continue;
                    };
                    if truthy_value(row_object.get("empty")) {
                        continue;
                    }
                    if let Some(height) = row_object.get("height").and_then(json_i64_value) {
                        let props = MxlFormatProps {
                            height: Some(height),
                            ..Default::default()
                        };
                        registry.register(mxl_format_key(&props), props);
                    }
                    if let Some(row_style) = row_object.get("rowStyle").and_then(Value::as_str) {
                        register_mxl_cell_format(
                            row_style,
                            "",
                            &defn,
                            &font_map,
                            thin_line_index,
                            thick_line_index,
                            &mut registry,
                        );
                    }
                    if let Some(cells) = row_object.get("cells").and_then(Value::as_array) {
                        for cell in cells {
                            let cell_style = cell
                                .get("style")
                                .and_then(Value::as_str)
                                .or_else(|| row_object.get("rowStyle").and_then(Value::as_str))
                                .unwrap_or("default");
                            let fill_type = mxl_fill_type(cell);
                            register_mxl_cell_format(
                                cell_style,
                                fill_type,
                                &defn,
                                &font_map,
                                thin_line_index,
                                thick_line_index,
                                &mut registry,
                            );
                        }
                    }
                }
            }
        }

        let default_key = mxl_format_key(&MxlFormatProps {
            width: Some(default_width),
            ..Default::default()
        });
        let default_format_index = registry.register(
            default_key,
            MxlFormatProps {
                width: Some(default_width),
                ..Default::default()
            },
        );

        let mut lines = Vec::<String>::new();
        lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
        lines.push("<document xmlns=\"http://v8.1c.ru/8.2/data/spreadsheet\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">".to_string());
        lines.push("\t<languageSettings>".to_string());
        lines.push("\t\t<currentLanguage>ru</currentLanguage>".to_string());
        lines.push("\t\t<defaultLanguage>ru</defaultLanguage>".to_string());
        lines.push("\t\t<languageInfo>".to_string());
        lines.push("\t\t\t<id>ru</id>".to_string());
        lines.push("\t\t\t<code>Русский</code>".to_string());
        lines.push("\t\t\t<description>Русский</description>".to_string());
        lines.push("\t\t</languageInfo>".to_string());
        lines.push("\t</languageSettings>".to_string());
        lines.push("\t<columns>".to_string());
        lines.push(format!("\t\t<size>{total_columns}</size>"));
        for (column, format_index) in &col_format_map {
            lines.push("\t\t<columnsItem>".to_string());
            lines.push(format!("\t\t\t<index>{}</index>", column - 1));
            lines.push("\t\t\t<column>".to_string());
            lines.push(format!("\t\t\t\t<formatIndex>{format_index}</formatIndex>"));
            lines.push("\t\t\t</column>".to_string());
            lines.push("\t\t</columnsItem>".to_string());
        }
        lines.push("\t</columns>".to_string());

        let mut global_row = 0i64;
        let mut merges = Vec::<MxlMerge>::new();
        let mut named_items = Vec::<MxlNamedItem>::new();
        for area in areas {
            let area_start_row = global_row;
            let area_name = json_string_field(area, "name").unwrap_or_default();
            let mut active_rowspans = Vec::<MxlRowspan>::new();
            let mut local_row = 0i64;
            if let Some(rows) = area.get("rows").and_then(Value::as_array) {
                for row_value in rows {
                    let empty_row = Value::Object(Map::new());
                    let row = if row_value.is_array() {
                        empty_row.as_object().unwrap()
                    } else {
                        row_value
                            .as_object()
                            .ok_or_else(|| "MXL row must be an object or array".to_string())?
                    };
                    if let Some(count) = row.get("empty").and_then(json_i64_value) {
                        for _ in 0..count {
                            lines.push("\t<rowsItem>".to_string());
                            lines.push(format!("\t\t<index>{global_row}</index>"));
                            lines.push("\t\t<row>".to_string());
                            lines.push("\t\t\t<empty>true</empty>".to_string());
                            lines.push("\t\t</row>".to_string());
                            lines.push("\t</rowsItem>".to_string());
                            global_row += 1;
                            local_row += 1;
                        }
                        continue;
                    }

                    let mut rowspan_occupied = std::collections::BTreeMap::<i64, bool>::new();
                    for rowspan in &active_rowspans {
                        if local_row > rowspan.start_local_row && local_row <= rowspan.end_local_row
                        {
                            for column in rowspan.col_start..=rowspan.col_end {
                                rowspan_occupied.insert(column, true);
                            }
                        }
                    }

                    let mut row_has_content = false;
                    let mut row_cells = Vec::<MxlCellInfo>::new();
                    let mut row_format_idx = 0usize;
                    if let Some(height) = row.get("height").and_then(json_i64_value) {
                        let props = MxlFormatProps {
                            height: Some(height),
                            ..Default::default()
                        };
                        row_format_idx = registry.index_of(&mxl_format_key(&props)).unwrap_or(0);
                    }

                    if let Some(cells) = row.get("cells").and_then(Value::as_array) {
                        if !cells.is_empty() {
                            row_has_content = true;
                            let mut occupied_cols = rowspan_occupied.clone();
                            for cell in cells {
                                let col_start =
                                    cell.get("col").and_then(json_i64_value).unwrap_or(0);
                                let col_span =
                                    cell.get("span").and_then(json_i64_value).unwrap_or(1);
                                for column in col_start..(col_start + col_span) {
                                    occupied_cols.insert(column, true);
                                }
                            }

                            for cell in cells {
                                let col_start =
                                    cell.get("col").and_then(json_i64_value).unwrap_or(0);
                                let col_span =
                                    cell.get("span").and_then(json_i64_value).unwrap_or(1);
                                let rowspan =
                                    cell.get("rowspan").and_then(json_i64_value).unwrap_or(1);
                                let cell_style = cell
                                    .get("style")
                                    .and_then(Value::as_str)
                                    .or_else(|| row.get("rowStyle").and_then(Value::as_str))
                                    .unwrap_or("default");
                                let fill_type = mxl_fill_type(cell);
                                let fmt_idx = register_mxl_cell_format(
                                    cell_style,
                                    fill_type,
                                    &defn,
                                    &font_map,
                                    thin_line_index,
                                    thick_line_index,
                                    &mut registry,
                                );

                                row_cells.push(MxlCellInfo {
                                    col: col_start - 1,
                                    col_span,
                                    format_idx: fmt_idx,
                                    param: json_string_field(cell, "param"),
                                    detail: json_string_field(cell, "detail"),
                                    text: json_string_field(cell, "text"),
                                    template: json_string_field(cell, "template"),
                                });

                                if rowspan > 1 {
                                    active_rowspans.push(MxlRowspan {
                                        col_start,
                                        col_end: col_start + col_span - 1,
                                        start_local_row: local_row,
                                        end_local_row: local_row + rowspan - 1,
                                    });
                                }
                                if col_span > 1 || rowspan > 1 {
                                    merges.push(MxlMerge {
                                        row: global_row,
                                        column: col_start - 1,
                                        width: col_span - 1,
                                        height: (rowspan > 1).then_some(rowspan - 1),
                                    });
                                }
                            }

                            if let Some(row_style) = row.get("rowStyle").and_then(Value::as_str) {
                                let gap_fmt_idx = register_mxl_cell_format(
                                    row_style,
                                    "",
                                    &defn,
                                    &font_map,
                                    thin_line_index,
                                    thick_line_index,
                                    &mut registry,
                                );
                                for column in 1..=total_columns {
                                    if !occupied_cols.contains_key(&column) {
                                        row_cells.push(MxlCellInfo {
                                            col: column - 1,
                                            col_span: 1,
                                            format_idx: gap_fmt_idx,
                                            param: None,
                                            detail: None,
                                            text: None,
                                            template: None,
                                        });
                                    }
                                }
                            }
                            row_cells.sort_by_key(|cell| cell.col);
                        }
                    } else if let Some(row_style) = row.get("rowStyle").and_then(Value::as_str) {
                        row_has_content = true;
                        let gap_fmt_idx = register_mxl_cell_format(
                            row_style,
                            "",
                            &defn,
                            &font_map,
                            thin_line_index,
                            thick_line_index,
                            &mut registry,
                        );
                        for column in 1..=total_columns {
                            if !rowspan_occupied.contains_key(&column) {
                                row_cells.push(MxlCellInfo {
                                    col: column - 1,
                                    col_span: 1,
                                    format_idx: gap_fmt_idx,
                                    param: None,
                                    detail: None,
                                    text: None,
                                    template: None,
                                });
                            }
                        }
                    }

                    lines.push("\t<rowsItem>".to_string());
                    lines.push(format!("\t\t<index>{global_row}</index>"));
                    lines.push("\t\t<row>".to_string());
                    if row_format_idx > 0 {
                        lines.push(format!("\t\t\t<formatIndex>{row_format_idx}</formatIndex>"));
                    }
                    if !row_has_content {
                        lines.push("\t\t\t<empty>true</empty>".to_string());
                    } else {
                        let mut expected_col = 0;
                        for cell in &row_cells {
                            emit_mxl_cell(&mut lines, cell, expected_col);
                            expected_col = cell.col + cell.col_span;
                        }
                    }
                    lines.push("\t\t</row>".to_string());
                    lines.push("\t</rowsItem>".to_string());

                    local_row += 1;
                    global_row += 1;
                }
            }
            named_items.push(MxlNamedItem {
                name: area_name,
                begin_row: area_start_row,
                end_row: global_row - 1,
            });
        }

        if global_row == 0 {
            return Err(
                "MXL definition must contain at least one row; a zero-row definition cannot preserve columns or named areas in the canonical platform 8.3.27 empty sentinel"
                    .to_string(),
            );
        }

        lines.push("\t<templateMode>true</templateMode>".to_string());
        lines.push(format!(
            "\t<defaultFormatIndex>{default_format_index}</defaultFormatIndex>"
        ));
        lines.push(format!("\t<height>{global_row}</height>"));
        lines.push(format!("\t<vgRows>{global_row}</vgRows>"));
        for merge in &merges {
            lines.push("\t<merge>".to_string());
            lines.push(format!("\t\t<r>{}</r>", merge.row));
            lines.push(format!("\t\t<c>{}</c>", merge.column));
            if let Some(height) = merge.height {
                lines.push(format!("\t\t<h>{height}</h>"));
            }
            lines.push(format!("\t\t<w>{}</w>", merge.width));
            lines.push("\t</merge>".to_string());
        }
        for item in &named_items {
            lines.push("\t<namedItem xsi:type=\"NamedItemCells\">".to_string());
            lines.push(format!(
                "\t\t<name>{}</name>",
                escape_mxl_xml_text(&item.name)
            ));
            lines.push("\t\t<area>".to_string());
            lines.push("\t\t\t<type>Rows</type>".to_string());
            lines.push(format!("\t\t\t<beginRow>{}</beginRow>", item.begin_row));
            lines.push(format!("\t\t\t<endRow>{}</endRow>", item.end_row));
            lines.push("\t\t\t<beginColumn>-1</beginColumn>".to_string());
            lines.push("\t\t\t<endColumn>-1</endColumn>".to_string());
            lines.push("\t\t</area>".to_string());
            lines.push("\t</namedItem>".to_string());
        }
        if has_thin_borders {
            lines.push("\t<line width=\"1\" gap=\"false\">".to_string());
            lines.push("\t\t<v8ui:style xsi:type=\"v8ui:SpreadsheetDocumentCellLineType\">Solid</v8ui:style>".to_string());
            lines.push("\t</line>".to_string());
        }
        if has_thick_borders {
            lines.push("\t<line width=\"2\" gap=\"false\">".to_string());
            lines.push("\t\t<v8ui:style xsi:type=\"v8ui:SpreadsheetDocumentCellLineType\">Solid</v8ui:style>".to_string());
            lines.push("\t</line>".to_string());
        }
        for font in &font_entries {
            lines.push(format!(
                "\t<font faceName=\"{}\" height=\"{}\" bold=\"{}\" italic=\"{}\" underline=\"{}\" strikeout=\"{}\" kind=\"Absolute\" scale=\"100\"/>",
                escape_mxl_xml_attribute(&font.face), font.size, font.bold, font.italic, font.underline, font.strikeout
            ));
        }
        for (_, format) in &registry.entries {
            emit_mxl_format(&mut lines, format);
        }
        lines.push("</document>".to_string());

        let xml = format!("{}\n", lines.join("\n"));
        Document::parse(&xml).map_err(|error| format!("compiled MXL is not valid XML: {error}"))?;
        let output_path = absolutize(output_path_raw.clone(), &context.cwd);
        transaction.create_or_replace_bytes(&output_path, utf8_bom_bytes(&xml))?;
        guard_active_format_owner_with_exact_root(
            &mut transaction,
            &output_path,
            context,
            MXL_ROOT,
        )?;
        let report = transaction
            .commit_with_post_validation(|| require_mxl_post_validation(&output_path, context))?;

        let mut stdout = format!("[OK] Compiled: {}\n", output_path_raw.display());
        if let Some(page_name) = page_name {
            stdout.push_str(&format!(
                "     Page: {page_name} -> target {}, defaultWidth={default_width}\n",
                target_width.unwrap_or(0)
            ));
        }
        stdout.push_str(&format!(
            "     Areas: {}, Rows: {global_row}, Columns: {total_columns}\n",
            named_items.len()
        ));
        stdout.push_str(&format!(
            "     Fonts: {}, Lines: {line_count}, Formats: {}\n",
            font_entries.len(),
            registry.entries.len()
        ));
        stdout.push_str(&format!("     Merges: {}\n", merges.len()));

        Ok((stdout, output_path, report.cleanup_warnings))
    })();

    match write_result {
        Ok((stdout, output_path, warnings)) => AdapterOutcome {
            ok: true,
            summary: "unica.mxl.compile completed with native spreadsheet writer".to_string(),
            changes: vec![format!("updated {}", output_path.display())],
            warnings,
            errors: Vec::new(),
            artifacts: vec![output_path.display().to_string()],
            stdout: Some(stdout),
            stderr: None,
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.mxl.compile failed in native spreadsheet writer".to_string(),
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

fn require_mxl_post_validation(
    template_path: &Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    let validation_args = Map::from_iter([(
        "TemplatePath".to_string(),
        Value::String(template_path.display().to_string()),
    )]);
    let outcome = validate_mxl(&validation_args, context);
    if outcome.ok {
        return Ok(());
    }
    let detail = if outcome.errors.is_empty() {
        outcome
            .stdout
            .unwrap_or_else(|| "validation returned no diagnostics".to_string())
    } else {
        outcome.errors.join("; ")
    };
    Err(format!(
        "MXL validation failed for {}: {}",
        template_path.display(),
        detail.trim()
    ))
}

pub(crate) fn parse_mxl_column_spec(spec: &str) -> Result<Vec<i64>, String> {
    let mut columns = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if let Some((from, to)) = part.split_once('-') {
            let from = from
                .parse::<i64>()
                .map_err(|_| format!("invalid column spec: {spec}"))?;
            let to = to
                .parse::<i64>()
                .map_err(|_| format!("invalid column spec: {spec}"))?;
            for column in from..=to {
                columns.push(column);
            }
        } else {
            columns.push(
                part.parse::<i64>()
                    .map_err(|_| format!("invalid column spec: {spec}"))?,
            );
        }
    }
    Ok(columns)
}

pub(crate) fn add_mxl_font(
    name: &str,
    font_def: Option<&Value>,
    font_map: &mut std::collections::BTreeMap<String, usize>,
    font_entries: &mut Vec<MxlFontEntry>,
) {
    let face = font_def
        .and_then(|value| json_string_field(value, "face"))
        .unwrap_or_else(|| "Arial".to_string());
    let size = font_def
        .and_then(|value| json_i64_field(value, "size"))
        .unwrap_or(10);
    let bool_prop = |prop: &str| {
        if font_def
            .and_then(|value| value.get(prop))
            .and_then(Value::as_bool)
            == Some(true)
        {
            "true"
        } else {
            "false"
        }
    };
    let index = font_entries.len();
    font_map.insert(name.to_string(), index);
    font_entries.push(MxlFontEntry {
        face,
        size,
        bold: bool_prop("bold"),
        italic: bool_prop("italic"),
        underline: bool_prop("underline"),
        strikeout: bool_prop("strikeout"),
    });
}

pub(crate) fn mxl_format_key(props: &MxlFormatProps) -> String {
    format!(
        "f={}|lb={}|tb={}|rb={}|bb={}|ha={}|va={}|wr={}|ft={}|nf={}|w={}|h={}",
        props.font_idx.unwrap_or(-1),
        props.lb.unwrap_or(-1),
        props.tb.unwrap_or(-1),
        props.rb.unwrap_or(-1),
        props.bb.unwrap_or(-1),
        props.ha,
        props.va,
        if props.wrap { "True" } else { "False" },
        props.fill_type,
        props.number_format,
        props.width.unwrap_or(-1),
        props.height.unwrap_or(-1)
    )
}

pub(crate) fn mxl_fill_type(cell: &Value) -> &'static str {
    if truthy_value(cell.get("param")) {
        "Parameter"
    } else if truthy_value(cell.get("template")) {
        "Template"
    } else if truthy_value(cell.get("text")) {
        "Text"
    } else {
        ""
    }
}

pub(crate) fn mxl_resolve_style(
    style_name: &str,
    fill_type: &str,
    defn: &Value,
    font_map: &std::collections::BTreeMap<String, usize>,
    thin_line_index: i64,
    thick_line_index: i64,
) -> MxlFormatProps {
    let mut props = MxlFormatProps {
        font_idx: Some(*font_map.get("default").unwrap_or(&0) as i64),
        lb: Some(-1),
        tb: Some(-1),
        rb: Some(-1),
        bb: Some(-1),
        fill_type: fill_type.to_string(),
        ..Default::default()
    };

    let style = if style_name.is_empty() {
        None
    } else {
        defn.get("styles")
            .and_then(Value::as_object)
            .and_then(|styles| styles.get(style_name))
    };
    let Some(style) = style else {
        return props;
    };

    if let Some(font_name) = json_string_field(style, "font") {
        if let Some(font_idx) = font_map.get(&font_name) {
            props.font_idx = Some(*font_idx as i64);
        }
    }

    if let Some(border) = json_string_field(style, "border") {
        if !border.is_empty() && border != "none" {
            let line_idx = if json_string_field(style, "borderWidth").as_deref() == Some("thick") {
                thick_line_index
            } else {
                thin_line_index
            };
            for side in border.split(',').map(str::trim) {
                match side {
                    "all" => {
                        props.lb = Some(line_idx);
                        props.tb = Some(line_idx);
                        props.rb = Some(line_idx);
                        props.bb = Some(line_idx);
                    }
                    "left" => props.lb = Some(line_idx),
                    "top" => props.tb = Some(line_idx),
                    "right" => props.rb = Some(line_idx),
                    "bottom" => props.bb = Some(line_idx),
                    _ => {}
                }
            }
        }
    }

    if let Some(align) = json_string_field(style, "align") {
        props.ha = match align.as_str() {
            "left" => "Left",
            "center" => "Center",
            "right" => "Right",
            _ => "",
        }
        .to_string();
    }
    if let Some(valign) = json_string_field(style, "valign") {
        props.va = match valign.as_str() {
            "top" => "Top",
            "center" => "Center",
            _ => "",
        }
        .to_string();
    }
    props.wrap = style.get("wrap").and_then(Value::as_bool) == Some(true);
    props.number_format = json_string_field(style, "format").unwrap_or_default();
    props
}

pub(crate) struct MxlCellInfo {
    pub(crate) col: i64,
    pub(crate) col_span: i64,
    pub(crate) format_idx: usize,
    pub(crate) param: Option<String>,
    pub(crate) detail: Option<String>,
    pub(crate) text: Option<String>,
    pub(crate) template: Option<String>,
}

pub(crate) struct MxlRowspan {
    pub(crate) col_start: i64,
    pub(crate) col_end: i64,
    pub(crate) start_local_row: i64,
    pub(crate) end_local_row: i64,
}

pub(crate) struct MxlMerge {
    pub(crate) row: i64,
    pub(crate) column: i64,
    pub(crate) width: i64,
    pub(crate) height: Option<i64>,
}

pub(crate) struct MxlNamedItem {
    pub(crate) name: String,
    pub(crate) begin_row: i64,
    pub(crate) end_row: i64,
}

fn escape_mxl_xml_text(value: &str) -> String {
    escape_xml(value).replace('\r', "&#13;")
}

fn escape_mxl_xml_attribute(value: &str) -> String {
    escape_mxl_xml_text(value)
        .replace('\t', "&#9;")
        .replace('\n', "&#10;")
}

pub(crate) fn emit_mxl_cell(lines: &mut Vec<String>, cell: &MxlCellInfo, expected_col: i64) {
    lines.push("\t\t\t<c>".to_string());
    if cell.col != expected_col {
        lines.push(format!("\t\t\t\t<i>{}</i>", cell.col));
    }
    lines.push("\t\t\t\t<c>".to_string());
    lines.push(format!("\t\t\t\t\t<f>{}</f>", cell.format_idx));
    if let Some(param) = &cell.param {
        lines.push(format!(
            "\t\t\t\t\t<parameter>{}</parameter>",
            escape_mxl_xml_text(param)
        ));
        if let Some(detail) = &cell.detail {
            lines.push(format!(
                "\t\t\t\t\t<detailParameter>{}</detailParameter>",
                escape_mxl_xml_text(detail)
            ));
        }
    }
    if let Some(text) = &cell.text {
        emit_mxl_text(lines, text);
    }
    if let Some(template) = &cell.template {
        emit_mxl_text(lines, template);
    }
    lines.push("\t\t\t\t</c>".to_string());
    lines.push("\t\t\t</c>".to_string());
}

pub(crate) fn emit_mxl_text(lines: &mut Vec<String>, text: &str) {
    lines.push("\t\t\t\t\t<tl>".to_string());
    lines.push("\t\t\t\t\t\t<v8:item>".to_string());
    lines.push("\t\t\t\t\t\t\t<v8:lang>ru</v8:lang>".to_string());
    lines.push(format!(
        "\t\t\t\t\t\t\t<v8:content>{}</v8:content>",
        escape_mxl_xml_text(text)
    ));
    lines.push("\t\t\t\t\t\t</v8:item>".to_string());
    lines.push("\t\t\t\t\t</tl>".to_string());
}

pub(crate) fn emit_mxl_format(lines: &mut Vec<String>, format: &MxlFormatProps) {
    lines.push("\t<format>".to_string());
    if let Some(font_idx) = format.font_idx {
        if font_idx >= 0 {
            lines.push(format!("\t\t<font>{font_idx}</font>"));
        }
    }
    for (tag, value) in [
        ("leftBorder", format.lb),
        ("topBorder", format.tb),
        ("rightBorder", format.rb),
        ("bottomBorder", format.bb),
    ] {
        if let Some(value) = value {
            if value >= 0 {
                lines.push(format!("\t\t<{tag}>{value}</{tag}>"));
            }
        }
    }
    if let Some(width) = format.width {
        if width != 0 {
            lines.push(format!("\t\t<width>{width}</width>"));
        }
    }
    if let Some(height) = format.height {
        if height != 0 {
            lines.push(format!("\t\t<height>{height}</height>"));
        }
    }
    if !format.ha.is_empty() {
        lines.push(format!(
            "\t\t<horizontalAlignment>{}</horizontalAlignment>",
            format.ha
        ));
    }
    if !format.va.is_empty() {
        lines.push(format!(
            "\t\t<verticalAlignment>{}</verticalAlignment>",
            format.va
        ));
    }
    if format.wrap {
        lines.push("\t\t<textPlacement>Wrap</textPlacement>".to_string());
    }
    if !format.fill_type.is_empty() {
        lines.push(format!("\t\t<fillType>{}</fillType>", format.fill_type));
    }
    if !format.number_format.is_empty() {
        lines.push("\t\t<format>".to_string());
        lines.push("\t\t\t<v8:item>".to_string());
        lines.push("\t\t\t\t<v8:lang>ru</v8:lang>".to_string());
        lines.push(format!(
            "\t\t\t\t<v8:content>{}</v8:content>",
            escape_mxl_xml_text(&format.number_format)
        ));
        lines.push("\t\t\t</v8:item>".to_string());
        lines.push("\t\t</format>".to_string());
    }
    lines.push("\t</format>".to_string());
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
        // Typed answer; the data reaches the envelope through typed_result.rs.
        "mxl-info" => Some(Ok(analyze_mxl_info(args, context).outcome)),
        "mxl-validate" => Some(Ok(validate_mxl(args, context))),
        "mxl-decompile" => Some(Ok(decompile_mxl(args, context))),
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
        "mxl-compile" => Some(compile_mxl(args, context)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::UnicaApplication;
    use crate::infrastructure::native_operations::compile_transaction::{
        with_commit_failpoint, CommitFailpoint,
    };
    use crate::infrastructure::native_operations::single_file_publisher::with_before_commit_hook;

    #[test]
    fn empty_spreadsheet_document_has_no_rows_container() {
        let xml = empty_spreadsheet_document_xml();
        let document = Document::parse(&xml).unwrap();
        let root = document.root_element();
        let child_names = root
            .children()
            .filter(roxmltree::Node::is_element)
            .map(|node| node.tag_name().name())
            .collect::<Vec<_>>();

        assert_eq!(
            child_names,
            [
                "languageSettings",
                "columns",
                "rowsItem",
                "templateMode",
                "vgRows"
            ]
        );
        assert!(!xml.contains("<rows>"));
    }

    #[test]
    fn mxl_validate_rejects_non_document_root() {
        let (context, template_path) = write_test_mxl(
            "validate-root",
            &format!("<SpreadsheetDocument xmlns=\"{MXL_DOCUMENT_NS}\"/>"),
        );

        let outcome = validate_mxl(&path_args(&template_path), &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.errors.iter().any(|error| {
            error.contains("SpreadsheetDocument") && error.contains("expected 'document'")
        }));
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mxl_validate_rejects_wrong_document_namespace() {
        let (context, template_path) =
            write_test_mxl("validate-namespace", "<document xmlns=\"urn:not-mxl\"/>");

        let outcome = validate_mxl(&path_args(&template_path), &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("urn:not-mxl") && error.contains(MXL_DOCUMENT_NS)));
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mxl_validate_accepts_platform_8_3_27_empty_fixture() {
        let context = test_context("validate-platform-empty");
        let fixture = platform_mxl_fixture();

        let outcome = validate_mxl(&path_args(&fixture), &context);

        assert!(outcome.ok, "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mxl_validate_rejects_noncanonical_empty_sentinel_lookalike() {
        let xml = empty_spreadsheet_document_xml().replace(
            "<templateMode>true</templateMode>",
            "<templateMode>false</templateMode>",
        );
        let (context, template_path) = write_test_mxl("validate-empty-lookalike", &xml);

        let outcome = validate_mxl(&path_args(&template_path), &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("height=0")));
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mxl_info_reports_zero_rows_for_platform_8_3_27_empty_fixture() {
        let context = test_context("info-platform-empty");
        let fixture = platform_mxl_fixture();
        let args = path_args(&fixture);

        let execution = analyze_mxl_info(&args, &context);

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        // ADR-0023: the template description is data, not a JSON string in stdout.
        let report = serde_json::to_value(execution.data.unwrap()).unwrap();
        assert_eq!(report["rows"], 0);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mxl_info_preserves_explicit_height_for_first_empty_row() {
        let xml = empty_spreadsheet_document_xml().replace(
            "\t<vgRows>0</vgRows>",
            "\t<height>1</height>\n\t<vgRows>0</vgRows>",
        );
        let (context, template_path) = write_test_mxl("info-first-empty-row", &xml);
        let args = path_args(&template_path);

        let execution = analyze_mxl_info(&args, &context);

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        // ADR-0023: the template description is data, not a JSON string in stdout.
        let report = serde_json::to_value(execution.data.unwrap()).unwrap();
        assert_eq!(report["rows"], 1);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mxl_info_does_not_treat_height_zero_lookalike_as_platform_empty() {
        let xml = empty_spreadsheet_document_xml().replace(
            "\t<vgRows>0</vgRows>",
            "\t<height>0</height>\n\t<vgRows>0</vgRows>",
        );
        let (context, template_path) = write_test_mxl("info-height-zero-lookalike", &xml);
        let args = path_args(&template_path);

        let execution = analyze_mxl_info(&args, &context);

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        // ADR-0023: the template description is data, not a JSON string in stdout.
        let report = serde_json::to_value(execution.data.unwrap()).unwrap();
        assert_eq!(report["rows"], 1);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    /// `TemplatePath` is the only address this reader publishes, so both
    /// documented forms of it must work: the descriptor file and the layout
    /// directory that resolves to `Ext/Template.xml`.
    #[test]
    fn mxl_info_resolves_a_layout_directory_to_its_template() {
        let context = test_context("info-layout-directory");
        let layout = context.cwd.join("Templates").join("ПФ_MXL_Продажи");
        let template_path = layout.join("Ext").join("Template.xml");
        fs::create_dir_all(template_path.parent().unwrap()).unwrap();
        fs::write(&template_path, empty_spreadsheet_document_xml()).unwrap();

        let execution = analyze_mxl_info(&path_args(&layout), &context);

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        assert_eq!(
            execution.outcome.artifacts,
            vec![template_path.display().to_string()]
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mxl_info_rejects_wrong_document_namespace() {
        let (context, template_path) =
            write_test_mxl("info-namespace", "<document xmlns=\"urn:not-mxl\"/>");

        let execution = analyze_mxl_info(&path_args(&template_path), &context);

        let outcome = execution.outcome;
        assert!(!outcome.ok, "{outcome:?}");
        assert!(execution.data.is_none());
        assert!(outcome.artifacts.is_empty());
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("urn:not-mxl") && error.contains(MXL_DOCUMENT_NS)));
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mxl_decompile_rejects_wrong_root_without_output() {
        let (context, template_path) = write_test_mxl(
            "decompile-root",
            "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\"/>",
        );
        let output_path = context.cwd.join("decompiled.json");
        let mut args = path_args(&template_path);
        args.insert(
            "OutputPath".to_string(),
            json!(output_path.display().to_string()),
        );

        let outcome = decompile_mxl(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(!output_path.exists());
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("Form") && error.contains("expected 'document'")));
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mxl_decompile_does_not_honor_legacy_output_path() {
        let context = test_context("decompile-no-output-file");
        let template_path = context.cwd.join("Template.xml");
        fs::copy(platform_mxl_fixture(), &template_path).unwrap();
        let original = fs::read(&template_path).unwrap();
        let mut args = path_args(&template_path);
        args.insert(
            "OutputPath".to_string(),
            json!(template_path.display().to_string()),
        );

        let outcome = decompile_mxl(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        assert_eq!(fs::read(&template_path).unwrap(), original);
        assert!(outcome
            .stdout
            .as_deref()
            .is_some_and(|stdout| stdout.starts_with('{')));
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mxl_decompile_counts_only_exact_platform_sentinel_as_zero_rows() {
        let exact_context = test_context("decompile-platform-empty");
        let exact = decompile_mxl(&path_args(&platform_mxl_fixture()), &exact_context);

        assert!(exact.ok, "{exact:?}");
        assert!(
            exact
                .stderr
                .as_deref()
                .is_some_and(|stderr| stderr.contains("Rows: 0")),
            "{exact:?}"
        );
        let _ = fs::remove_dir_all(&exact_context.cwd);

        let lookalike_xml = empty_spreadsheet_document_xml().replace(
            "\t<vgRows>0</vgRows>",
            "\t<height>0</height>\n\t<vgRows>0</vgRows>",
        );
        let (lookalike_context, lookalike_path) =
            write_test_mxl("decompile-height-zero-lookalike", &lookalike_xml);
        let lookalike = decompile_mxl(&path_args(&lookalike_path), &lookalike_context);

        assert!(lookalike.ok, "{lookalike:?}");
        assert!(
            lookalike
                .stderr
                .as_deref()
                .is_some_and(|stderr| stderr.contains("Rows: 1")),
            "{lookalike:?}"
        );
        let _ = fs::remove_dir_all(&lookalike_context.cwd);

        let missing_row_xml = empty_spreadsheet_document_xml().replace(
            "\t<rowsItem>\n\t\t<index>0</index>\n\t\t<row>\n\t\t\t<empty>true</empty>\n\t\t</row>\n\t</rowsItem>\n",
            "",
        );
        let (missing_row_context, missing_row_path) =
            write_test_mxl("decompile-missing-sentinel-row", &missing_row_xml);
        let missing_row = decompile_mxl(&path_args(&missing_row_path), &missing_row_context);

        assert!(missing_row.ok, "{missing_row:?}");
        assert!(
            missing_row
                .stderr
                .as_deref()
                .is_some_and(|stderr| stderr.contains("Rows: 1")),
            "{missing_row:?}"
        );
        let _ = fs::remove_dir_all(&missing_row_context.cwd);
    }

    #[test]
    fn mxl_compile_rejects_zero_row_definition_atomically_at_public_boundary() {
        let context = test_context("compile-zero-row-public-boundary");
        let source = context.cwd.join("src");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        let definition_path = context.cwd.join("zero-rows.json");
        fs::write(
            &definition_path,
            r#"{"columns":2,"areas":[{"name":"Empty","rows":[]}]}"#,
        )
        .unwrap();
        let output_path = context.cwd.join("generated/nested/Template.xml");
        let args = Map::from_iter([
            ("cwd".to_string(), json!(context.cwd.display().to_string())),
            (
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            ),
            (
                "OutputPath".to_string(),
                json!(output_path.display().to_string()),
            ),
            ("dryRun".to_string(), json!(false)),
        ]);

        let result = UnicaApplication::new()
            .call_tool("unica.mxl.compile", &args)
            .unwrap();

        assert!(!result.ok, "{result:?}");
        assert!(
            result
                .errors
                .iter()
                .any(|error| error.contains("at least one row")),
            "{result:?}"
        );
        assert!(!output_path.exists());
        assert!(!output_path.parent().unwrap().exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mxl_compile_directly_rejects_existing_wrong_root_without_write() {
        let context = test_context("compile-existing-wrong-root");
        let definition_path = context.cwd.join("definition.json");
        let output_path = context.cwd.join("Template.xml");
        fs::write(
            &definition_path,
            serde_json::to_vec(&json!({
                "columns": 1,
                "areas": [{
                    "name": "Area",
                    "rows": [{"cells": [{"col": 1, "text": "value"}]}]
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let original = b"<garbage/>".to_vec();
        fs::write(&output_path, &original).unwrap();
        let args = Map::from_iter([
            ("JsonPath".to_string(), json!(definition_path)),
            ("OutputPath".to_string(), json!(output_path)),
        ]);

        let outcome = compile_mxl(&args, &context);

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
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mxl_compile_escapes_all_user_xml_text_and_attributes() {
        let context = test_context("compile-xml-escaping");
        let definition_path = context.cwd.join("definition.json");
        let output_path = context.cwd.join("Template.xml");
        let special = "A&B <C> \"quoted\"\tline\nnext\rreturn";
        fs::write(
            &definition_path,
            serde_json::to_vec(&json!({
                "columns": 1,
                "fonts": {"default": {"face": special}},
                "areas": [{
                    "name": special,
                    "rows": [{"cells": [{
                        "col": 1,
                        "param": special,
                        "detail": special,
                        "text": special
                    }]}]
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let args = Map::from_iter([
            ("JsonPath".to_string(), json!(definition_path)),
            ("OutputPath".to_string(), json!(output_path)),
        ]);

        let outcome = compile_mxl(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        let bytes = fs::read(&output_path).unwrap();
        let xml = std::str::from_utf8(&bytes)
            .unwrap()
            .trim_start_matches('\u{feff}');
        let document = Document::parse(xml).expect("compiled MXL must be well-formed XML");
        let root = document.root_element();
        let named_item_name = root
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == "namedItem")
            .and_then(|node| {
                node.children()
                    .find(|child| child.is_element() && child.tag_name().name() == "name")
            })
            .and_then(|node| node.text());
        let font_face = root
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == "font")
            .and_then(|node| node.attribute("faceName"));
        let parameter = root
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == "parameter")
            .and_then(|node| node.text());
        let detail = root
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == "detailParameter")
            .and_then(|node| node.text());

        assert_eq!(named_item_name, Some(special));
        assert_eq!(font_face, Some(special));
        assert_eq!(parameter, Some(special));
        assert_eq!(detail, Some(special));
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mxl_compile_invalid_xml_character_does_not_overwrite_existing_target() {
        let context = test_context("compile-invalid-xml-character");
        let definition_path = context.cwd.join("definition.json");
        let output_path = context.cwd.join("Template.xml");
        let original = b"existing-target-bytes".to_vec();
        fs::write(
            &definition_path,
            serde_json::to_vec(&json!({
                "columns": 1,
                "areas": [{
                    "name": "invalid\u{0001}name",
                    "rows": [{"cells": [{"col": 1, "text": "value"}]}]
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(&output_path, &original).unwrap();
        let args = Map::from_iter([
            ("JsonPath".to_string(), json!(definition_path)),
            ("OutputPath".to_string(), json!(output_path)),
        ]);

        let outcome = compile_mxl(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert_eq!(fs::read(&output_path).unwrap(), original);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mxl_compile_final_failure_restores_existing_target_bytes() {
        let context = test_context("compile-final-failure");
        let definition_path = context.cwd.join("definition.json");
        let output_path = context.cwd.join("Template.xml");
        let original = b"existing-target-bytes".to_vec();
        fs::write(
            &definition_path,
            serde_json::to_vec(&json!({
                "columns": 1,
                "areas": [{
                    "name": "Area",
                    "rows": [{"cells": [{"col": 1, "text": "value"}]}]
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(&output_path, &original).unwrap();
        let args = Map::from_iter([
            ("JsonPath".to_string(), json!(definition_path)),
            ("OutputPath".to_string(), json!(output_path)),
        ]);

        let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            compile_mxl(&args, &context)
        });

        assert!(!outcome.ok, "{outcome:?}");
        assert_eq!(fs::read(&output_path).unwrap(), original);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mxl_compile_rolls_back_if_format_owner_changes_during_publication() {
        let context = test_context("compile-format-owner-race");
        let source = context.cwd.join("src");
        let output_path = source.join("Templates/Guarded/Ext/Template.xml");
        fs::create_dir_all(output_path.parent().unwrap()).unwrap();
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let owner = source.join("Configuration.xml");
        let owner_before = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#.to_vec();
        fs::write(&owner, &owner_before).unwrap();
        let definition_path = context.cwd.join("definition.json");
        fs::write(
            &definition_path,
            serde_json::to_vec(&json!({
                "columns": 1,
                "areas": [{
                    "name": "Area",
                    "rows": [{"cells": [{"col": 1, "text": "value"}]}]
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let args = Map::from_iter([
            ("JsonPath".to_string(), json!(definition_path)),
            ("OutputPath".to_string(), json!(output_path)),
        ]);
        let mut concurrent_owner = owner_before;
        concurrent_owner.extend_from_slice(b" ");
        let owner_for_hook = owner.clone();
        let concurrent_for_hook = concurrent_owner.clone();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&owner_for_hook, &concurrent_for_hook).unwrap(),
            || compile_mxl(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&owner).unwrap(), concurrent_owner);
        assert!(!source.join("Templates/Guarded/Ext/Template.xml").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mxl_compile_rolls_back_if_missing_wrapper_owner_appears_during_publication() {
        let context = test_context("compile-wrapper-owner-race");
        let source = context.cwd.join("src");
        let output_path = source.join("Templates/Guarded/Ext/Template.xml");
        let wrapper = source.join("Templates/Guarded.xml");
        fs::create_dir_all(output_path.parent().unwrap()).unwrap();
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        let original_output = utf8_bom_bytes(&empty_spreadsheet_document_xml());
        fs::write(&output_path, &original_output).unwrap();
        let definition_path = context.cwd.join("definition.json");
        fs::write(
            &definition_path,
            serde_json::to_vec(&json!({
                "columns": 1,
                "areas": [{
                    "name": "Area",
                    "rows": [{"cells": [{"col": 1, "text": "planned"}]}]
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let args = Map::from_iter([
            ("JsonPath".to_string(), json!(definition_path)),
            ("OutputPath".to_string(), json!(output_path)),
        ]);
        let concurrent_wrapper = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Template/></MetaDataObject>"#.to_vec();
        let wrapper_for_hook = wrapper.clone();
        let concurrent_for_hook = concurrent_wrapper.clone();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&wrapper_for_hook, &concurrent_for_hook).unwrap(),
            || compile_mxl(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        let error = outcome.errors.join("\n");
        assert!(error.contains("absence guard"), "{error}");
        assert!(error.contains("Guarded.xml"), "{error}");
        assert_eq!(fs::read(&output_path).unwrap(), original_output);
        assert_eq!(fs::read(&wrapper).unwrap(), concurrent_wrapper);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mxl_compile_rolls_back_if_json_definition_changes_after_parse() {
        let context = test_context("compile-json-input-race");
        let definition_path = context.cwd.join("definition.json");
        let output_path = context.cwd.join("Template.xml");
        let original_output = utf8_bom_bytes(&empty_spreadsheet_document_xml());
        fs::write(
            &definition_path,
            serde_json::to_vec(&json!({
                "columns": 1,
                "areas": [{
                    "name": "Area",
                    "rows": [{"cells": [{"col": 1, "text": "planned"}]}]
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(&output_path, &original_output).unwrap();
        let concurrent_definition = serde_json::to_vec(&json!({
            "columns": 1,
            "areas": [{
                "name": "Area",
                "rows": [{"cells": [{"col": 1, "text": "concurrent"}]}]
            }]
        }))
        .unwrap();
        let args = Map::from_iter([
            ("JsonPath".to_string(), json!(definition_path)),
            ("OutputPath".to_string(), json!(output_path)),
        ]);
        let definition_for_hook = definition_path.clone();
        let concurrent_for_hook = concurrent_definition.clone();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&definition_for_hook, &concurrent_for_hook).unwrap(),
            || compile_mxl(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&definition_path).unwrap(), concurrent_definition);
        assert_eq!(fs::read(&output_path).unwrap(), original_output);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mxl_compile_semantic_validation_failure_rolls_back_create_and_replace() {
        let context = test_context("compile-semantic-validation-rollback");
        let definition_path = context.cwd.join("definition.json");
        let replaced_path = context.cwd.join("Replaced.xml");
        let created_path = context.cwd.join("Created.xml");
        let original = utf8_bom_bytes(&empty_spreadsheet_document_xml().replace('\n', "\r\n"));
        fs::write(
            &definition_path,
            serde_json::to_vec(&json!({
                "columns": 1,
                "areas": [{
                    "name": "Area",
                    "rows": [{"cells": [{"col": 2, "text": "outside column set"}]}]
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(&replaced_path, &original).unwrap();

        let compile_to = |output_path: &Path| {
            compile_mxl(
                &Map::from_iter([
                    ("JsonPath".to_string(), json!(definition_path)),
                    ("OutputPath".to_string(), json!(output_path)),
                ]),
                &context,
            )
        };

        let replace_outcome = compile_to(&replaced_path);
        let create_outcome = compile_to(&created_path);

        for outcome in [&replace_outcome, &create_outcome] {
            assert!(!outcome.ok, "{outcome:?}");
            assert!(
                outcome
                    .errors
                    .iter()
                    .any(|error| error.contains("column index 1 >= column count (1)")),
                "{outcome:?}"
            );
        }
        assert_eq!(fs::read(&replaced_path).unwrap(), original);
        assert!(!created_path.exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mxl_compile_matches_platform_first_cell_and_format_order() {
        let xml = compile_test_mxl(
            "compile-platform-first-cell",
            &json!({
                "columns": 1,
                "areas": [{
                    "name": "A",
                    "rows": [{"cells": [{"col": 1, "text": "x"}]}]
                }]
            }),
        );
        let document = Document::parse(&xml).unwrap();
        let root = document.root_element();
        let row = root
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == "row")
            .unwrap();
        let outer_cell = row
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == "c")
            .unwrap();

        assert_eq!(direct_child_names(outer_cell), ["c"]);
        let inner_cell = outer_cell
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == "c")
            .unwrap();
        assert_eq!(direct_child_text(inner_cell, "f"), "1");
        assert_eq!(direct_child_text(root, "defaultFormatIndex"), "2");

        let formats = root
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "format")
            .collect::<Vec<_>>();
        assert_eq!(formats.len(), 2);
        assert_eq!(
            direct_child_name_texts(formats[0]),
            ["font=0", "fillType=Text"]
        );
        assert_eq!(direct_child_name_texts(formats[1]), ["width=10"]);
    }

    #[test]
    fn mxl_compile_omits_adjacent_cell_indices_and_emits_index_after_gap() {
        let xml = compile_test_mxl(
            "compile-platform-cell-indices",
            &json!({
                "columns": 4,
                "areas": [{
                    "name": "A",
                    "rows": [{
                        "cells": [
                            {"col": 1, "text": "first"},
                            {"col": 2, "text": "adjacent"},
                            {"col": 4, "text": "after gap"}
                        ]
                    }]
                }]
            }),
        );
        let document = Document::parse(&xml).unwrap();
        let row = document
            .root_element()
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == "row")
            .unwrap();
        let cells = row
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "c")
            .collect::<Vec<_>>();

        assert_eq!(cells.len(), 3);
        assert_eq!(direct_child_names(cells[0]), ["c"]);
        assert_eq!(direct_child_names(cells[1]), ["c"]);
        assert_eq!(direct_child_names(cells[2]), ["i", "c"]);
        assert_eq!(direct_child_text(cells[2], "i"), "3");
    }

    #[test]
    fn mxl_compile_advances_implicit_cell_index_by_column_span() {
        let xml = compile_test_mxl(
            "compile-platform-spanned-cell-indices",
            &json!({
                "columns": 5,
                "areas": [{
                    "name": "A",
                    "rows": [{
                        "cells": [
                            {"col": 1, "span": 2, "text": "spanned"},
                            {"col": 3, "text": "adjacent"},
                            {"col": 5, "text": "after gap"}
                        ]
                    }]
                }]
            }),
        );
        let document = Document::parse(&xml).unwrap();
        let row = document
            .root_element()
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == "row")
            .unwrap();
        let cells = row
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "c")
            .collect::<Vec<_>>();

        assert_eq!(cells.len(), 3);
        assert_eq!(direct_child_names(cells[0]), ["c"]);
        assert_eq!(direct_child_names(cells[1]), ["c"]);
        assert_eq!(direct_child_names(cells[2]), ["i", "c"]);
        assert_eq!(direct_child_text(cells[2], "i"), "4");
    }

    #[test]
    fn mxl_compile_orders_explicit_widths_before_cells_and_default_width_last() {
        let xml = compile_test_mxl(
            "compile-platform-multi-column",
            &json!({
                "columns": 2,
                "defaultWidth": 10,
                "columnWidths": {"2": 20},
                "styles": {"right": {"align": "right"}},
                "areas": [{
                    "name": "A",
                    "rows": [{
                        "cells": [
                            {"col": 1, "text": "left"},
                            {"col": 2, "text": "right", "style": "right"}
                        ]
                    }]
                }]
            }),
        );
        let document = Document::parse(&xml).unwrap();
        let root = document.root_element();
        let row = root
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == "row")
            .unwrap();
        let cells = row
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "c")
            .collect::<Vec<_>>();

        assert_eq!(cells.len(), 2);
        assert_eq!(direct_child_names(cells[0]), ["c"]);
        assert_eq!(
            direct_child_text(
                cells[0]
                    .children()
                    .find(|node| node.is_element() && node.tag_name().name() == "c")
                    .unwrap(),
                "f"
            ),
            "2"
        );
        assert_eq!(direct_child_names(cells[1]), ["c"]);
        assert_eq!(
            direct_child_text(
                cells[1]
                    .children()
                    .find(|node| node.is_element() && node.tag_name().name() == "c")
                    .unwrap(),
                "f"
            ),
            "3"
        );
        assert_eq!(direct_child_text(root, "defaultFormatIndex"), "4");

        let columns_item = root
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == "columnsItem")
            .unwrap();
        assert_eq!(direct_child_text(columns_item, "index"), "1");
        let column = columns_item
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == "column")
            .unwrap();
        assert_eq!(direct_child_text(column, "formatIndex"), "1");

        let formats = root
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "format")
            .collect::<Vec<_>>();
        assert_eq!(formats.len(), 4);
        assert_eq!(direct_child_name_texts(formats[0]), ["width=20"]);
        assert_eq!(
            direct_child_name_texts(formats[1]),
            ["font=0", "fillType=Text"]
        );
        assert_eq!(
            direct_child_name_texts(formats[2]),
            ["font=0", "horizontalAlignment=Right", "fillType=Text"]
        );
        assert_eq!(direct_child_name_texts(formats[3]), ["width=10"]);
    }

    fn path_args(path: &Path) -> Map<String, Value> {
        Map::from_iter([(
            "TemplatePath".to_string(),
            json!(path.display().to_string()),
        )])
    }

    fn compile_test_mxl(name: &str, definition: &Value) -> String {
        let context = test_context(name);
        let definition_path = context.cwd.join("definition.json");
        let output_path = context.cwd.join("Template.xml");
        fs::write(&definition_path, serde_json::to_vec(definition).unwrap()).unwrap();
        let args = Map::from_iter([
            (
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            ),
            (
                "OutputPath".to_string(),
                json!(output_path.display().to_string()),
            ),
        ]);

        let outcome = compile_mxl(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        let xml = fs::read_to_string(&output_path)
            .unwrap()
            .trim_start_matches('\u{feff}')
            .to_string();
        let _ = fs::remove_dir_all(&context.cwd);
        xml
    }

    fn direct_child_names(node: roxmltree::Node<'_, '_>) -> Vec<String> {
        node.children()
            .filter(roxmltree::Node::is_element)
            .map(|child| child.tag_name().name().to_string())
            .collect()
    }

    fn direct_child_text(node: roxmltree::Node<'_, '_>, name: &str) -> String {
        node.children()
            .find(|child| child.is_element() && child.tag_name().name() == name)
            .and_then(|child| child.text())
            .unwrap_or("")
            .to_string()
    }

    fn direct_child_name_texts(node: roxmltree::Node<'_, '_>) -> Vec<String> {
        node.children()
            .filter(roxmltree::Node::is_element)
            .map(|child| format!("{}={}", child.tag_name().name(), child.text().unwrap_or("")))
            .collect()
    }

    fn platform_mxl_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/platform_8_3_27/mxl/Template.xml")
    }

    fn write_test_mxl(name: &str, xml: &str) -> (WorkspaceContext, PathBuf) {
        let context = test_context(name);
        let template_path = context.cwd.join("Template.xml");
        fs::write(&template_path, xml).unwrap();
        (context, template_path)
    }

    fn test_context(name: &str) -> WorkspaceContext {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cwd = std::env::temp_dir().join(format!("unica-mxl-{name}-{nanos}"));
        fs::create_dir_all(&cwd).unwrap();
        WorkspaceContext {
            cwd: cwd.clone(),
            workspace_root: cwd.clone(),
            cache_root: cwd.join(".build/unica"),
            workspace_epoch: 0,
        }
    }
}

#[cfg(test)]
mod mxl_read_selector_bridge_tests {
    use super::*;
    use crate::infrastructure::native_operations::dcs::{analyze_dcs_info_with_data, validate_dcs};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NONCE: AtomicU64 = AtomicU64::new(0);

    /// A report template reachable both ways, plus a binary common template
    /// whose descriptor exists and whose XML body does not.
    pub(crate) fn workspace(name: &str) -> WorkspaceContext {
        let root = std::env::temp_dir().join(format!(
            "unica-template-read-bridge-{name}-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        let src = root.join("src");
        fs::create_dir_all(src.join("Reports/Sales/Templates/Main/Ext")).unwrap();
        fs::create_dir_all(src.join("CommonTemplates/Logo/Ext")).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            src.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects><Report>Sales</Report><CommonTemplate>Logo</CommonTemplate></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            src.join("Reports/Sales.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Report><Properties><Name>Sales</Name></Properties><ChildObjects><Template>Main</Template><Template>Dcs</Template></ChildObjects></Report></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            src.join("Reports/Sales/Templates/Main.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template><Properties><Name>Main</Name><TemplateType>SpreadsheetDocument</TemplateType></Properties></Template></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            src.join("Reports/Sales/Templates/Main/Ext/Template.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><document xmlns="http://v8.1c.ru/8.2/data/spreadsheet" xmlns:v8="http://v8.1c.ru/8.1/data/core"><columnsID>0</columnsID><format><formatIndex>0</formatIndex></format></document>"#,
        )
        .unwrap();
        fs::write(
            src.join("CommonTemplates/Logo.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonTemplate><Properties><Name>Logo</Name><TemplateType>BinaryData</TemplateType></Properties></CommonTemplate></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(src.join("CommonTemplates/Logo/Ext/Template.bin"), [0u8, 1]).unwrap();
        // The DCS readers parse a composition schema, not a spreadsheet. Giving
        // them the MXL body would make both sides of a parity test fail the
        // same way and compare two errors as if they were an answer.
        fs::create_dir_all(src.join("Reports/Sales/Templates/Dcs/Ext")).unwrap();
        fs::write(
            src.join("Reports/Sales/Templates/Dcs.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template><Properties><Name>Dcs</Name><TemplateType>DataCompositionSchema</TemplateType></Properties></Template></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            src.join("Reports/Sales/Templates/Dcs/Ext/Template.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dataSource><name>ИсточникДанных1</name><dataSourceType>Local</dataSourceType></dataSource><dataSet xsi:type="DataSetQuery"><name>НаборДанных1</name><field xsi:type="DataSetFieldField"><dataPath>Код</dataPath><field>Код</field></field><dataSource>ИсточникДанных1</dataSource><query>ВЫБРАТЬ Код ИЗ Справочник.Товары</query></dataSet></DataCompositionSchema>"#,
        )
        .unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    pub(crate) fn physical_args() -> Map<String, Value> {
        Map::from_iter([(
            "TemplatePath".to_string(),
            json!("src/Reports/Sales/Templates/Main/Ext/Template.xml"),
        )])
    }

    pub(crate) fn logical_args() -> Map<String, Value> {
        Map::from_iter([
            ("sourceSet".to_string(), json!("main")),
            (
                "metadataPath".to_string(),
                json!("Report.Sales.Template.Main"),
            ),
        ])
    }

    #[test]
    fn mxl_info_answers_identically_for_a_logical_and_a_physical_selector() {
        let context = workspace("mxl-info");

        let physical = analyze_mxl_info(&physical_args(), &context);
        let logical = analyze_mxl_info(&logical_args(), &context);

        assert!(logical.outcome.ok, "{:?}", logical.outcome);
        assert_eq!(
            serde_json::to_value(&logical.data).unwrap(),
            serde_json::to_value(&physical.data).unwrap()
        );
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn mxl_validate_answers_identically_for_a_logical_and_a_physical_selector() {
        let context = workspace("mxl-validate");

        let physical = validate_mxl(&physical_args(), &context);
        let logical = validate_mxl(&logical_args(), &context);

        assert_eq!(logical.ok, physical.ok, "{logical:?} vs {physical:?}");
        assert_eq!(logical.errors, physical.errors);
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn mxl_decompile_answers_identically_for_a_logical_and_a_physical_selector() {
        let context = workspace("mxl-decompile");

        let physical = decompile_mxl(&physical_args(), &context);
        let logical = decompile_mxl(&logical_args(), &context);

        assert_eq!(logical.ok, physical.ok, "{logical:?} vs {physical:?}");
        assert_eq!(logical.stdout, physical.stdout);
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    fn dcs_physical_args() -> Map<String, Value> {
        Map::from_iter([(
            "TemplatePath".to_string(),
            json!("src/Reports/Sales/Templates/Dcs/Ext/Template.xml"),
        )])
    }

    fn dcs_logical_args() -> Map<String, Value> {
        Map::from_iter([
            ("sourceSet".to_string(), json!("main")),
            (
                "metadataPath".to_string(),
                json!("Report.Sales.Template.Dcs"),
            ),
        ])
    }

    #[test]
    fn dcs_info_answers_identically_for_a_logical_and_a_physical_selector() {
        let context = workspace("dcs-info");

        let physical = analyze_dcs_info_with_data(&dcs_physical_args(), &context);
        let logical = analyze_dcs_info_with_data(&dcs_logical_args(), &context);

        // Both must succeed: comparing two identical failures would pass while
        // proving nothing about the bridge.
        assert!(physical.outcome.ok, "{:?}", physical.outcome);
        assert!(logical.outcome.ok, "{:?}", logical.outcome);
        assert!(
            logical.data.is_some(),
            "a successful read answers with data"
        );
        assert_eq!(
            serde_json::to_value(&logical.data).unwrap(),
            serde_json::to_value(&physical.data).unwrap()
        );
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn dcs_validate_answers_identically_for_a_logical_and_a_physical_selector() {
        let context = workspace("dcs-validate");

        let physical = validate_dcs(&dcs_physical_args(), &context);
        let logical = validate_dcs(&dcs_logical_args(), &context);

        assert!(physical.ok, "{physical:?}");
        assert_eq!(logical.ok, physical.ok, "{logical:?} vs {physical:?}");
        assert_eq!(logical.errors, physical.errors);
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    /// A `TemplateType` other than an XML one writes `Template.bin`. The object
    /// is registered and addressable, so "not found" would be the wrong answer:
    /// the target exists, this body does not.
    #[test]
    fn dcs_info_reports_a_binary_template_as_an_absent_resource_not_a_missing_target() {
        let context = workspace("dcs-binary");

        let outcome = analyze_dcs_info_with_data(
            &Map::from_iter([
                ("sourceSet".to_string(), json!("main")),
                ("metadataPath".to_string(), json!("CommonTemplate.Logo")),
            ]),
            &context,
        )
        .outcome;

        assert!(!outcome.ok);
        let text = outcome
            .errors
            .iter()
            .cloned()
            .chain(std::iter::once(outcome.summary.clone()))
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(text.contains("resource_absent"), "{text}");
        assert!(!text.contains("target_not_found"), "{text}");
        let _ = fs::remove_dir_all(&context.workspace_root);
    }
}
