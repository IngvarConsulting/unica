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
    cf::*, cfe::*, interface::*, meta::*, mxl::*, role::*, skd::*, subsystem::*, template::*,
};

const FORM_LOGFORM_NS: &str = "http://v8.1c.ru/8.3/xcf/logform";
const FORM_V8_NS: &str = "http://v8.1c.ru/8.1/data/core";

pub(crate) struct FormValidationReporter {
    pub(crate) errors: usize,
    pub(crate) warnings: usize,
    pub(crate) ok_count: usize,
    pub(crate) stopped: bool,
    pub(crate) max_errors: usize,
    pub(crate) detailed: bool,
    pub(crate) lines: Vec<String>,
}

impl FormValidationReporter {
    pub(crate) fn new(form_name: &str, max_errors: usize, detailed: bool) -> Self {
        Self {
            errors: 0,
            warnings: 0,
            ok_count: 0,
            stopped: false,
            max_errors,
            detailed,
            lines: vec![
                format!("=== Validation: Form.{form_name} ==="),
                String::new(),
            ],
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

    pub(crate) fn finalize(mut self, form_name: &str) -> (bool, String, Vec<String>) {
        let checks = self.ok_count + self.errors + self.warnings;
        let ok = self.errors == 0;
        if ok && self.warnings == 0 && !self.detailed {
            return (
                true,
                format!("=== Validation OK: Form.{form_name} ({checks} checks) ===\n"),
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
            .filter(|line| line.starts_with("[ERROR]"))
            .cloned()
            .collect::<Vec<_>>();
        (ok, format!("{}\n", self.lines.join("\n")), errors)
    }
}

#[derive(Clone)]
pub(crate) struct FormElementInfo<'a> {
    pub(crate) name: String,
    pub(crate) tag: String,
    pub(crate) id: String,
    pub(crate) node: roxmltree::Node<'a, 'a>,
}

pub(crate) fn validate_form(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let result = (|| -> Result<(bool, String, PathBuf, Vec<String>), String> {
        let raw_path = required_path(args, &["formPath", "FormPath", "path", "Path"], "FormPath")?;
        let form_path = resolve_form_info_path(absolutize(raw_path, &context.cwd));
        if !form_path.is_file() {
            return Err(format!("File not found: {}", form_path.display()));
        }

        let detailed = bool_arg(args, &["detailed", "Detailed"]);
        let max_errors = int_arg(args, &["maxErrors", "MaxErrors"])
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value > 0)
            .unwrap_or(30);
        let form_name = form_validation_name(&form_path);

        let text = read_utf8_sig(&form_path)?;
        let doc = match Document::parse(text.trim_start_matches('\u{feff}')) {
            Ok(doc) => doc,
            Err(err) => {
                let stdout =
                    format!("[ERROR] XML parse error: {err}\n\n---\nErrors: 1, Warnings: 0\n");
                return Ok((
                    false,
                    stdout,
                    form_path,
                    vec![format!("[ERROR] XML parse error: {err}")],
                ));
            }
        };
        let root = doc.root_element();
        let mut report = FormValidationReporter::new(&form_name, max_errors, detailed);

        let has_base_form = form_validation_child(root, "BaseForm").is_some();
        if root.tag_name().name() != "Form" {
            report.error(format!(
                "Root element is '{}', expected 'Form'",
                root.tag_name().name()
            ));
        } else {
            let version = root.attribute("version").unwrap_or("");
            if matches!(version, "2.17" | "2.20") {
                report.ok(format!("Root element: Form version={version}"));
            } else if version.is_empty() {
                report.warn("Form version attribute missing");
            } else {
                report.warn(format!("Form version='{version}' (expected 2.17 or 2.20)"));
            }
        }

        if !report.stopped {
            if let Some(acb) = form_validation_child(root, "AutoCommandBar") {
                let acb_name = acb.attribute("name").unwrap_or("");
                let acb_id = acb.attribute("id").unwrap_or("");
                if acb_id == "-1" {
                    report.ok(format!("AutoCommandBar: name='{acb_name}', id={acb_id}"));
                } else {
                    report.error(format!("AutoCommandBar id='{acb_id}', expected '-1'"));
                }
            } else {
                report.error("AutoCommandBar element missing");
            }
        }

        let mut elements = Vec::new();
        let mut element_ids = HashMap::<String, String>::new();
        let mut element_names = HashMap::<String, String>::new();
        if let Some(child_items) = form_validation_child(root, "ChildItems") {
            form_collect_elements(
                child_items,
                &mut elements,
                &mut element_ids,
                &mut element_names,
                &mut report,
            );
        }
        if let Some(acb_children) = form_validation_child(root, "AutoCommandBar")
            .and_then(|acb| form_validation_child(acb, "ChildItems"))
        {
            form_collect_elements(
                acb_children,
                &mut elements,
                &mut element_ids,
                &mut element_names,
                &mut report,
            );
        }

        if !report.stopped {
            let mut id_counts = HashMap::<String, usize>::new();
            for element in &elements {
                if element.id == "-1" {
                    continue;
                }
                *id_counts.entry(element.id.clone()).or_default() += 1;
            }
            if id_counts.values().all(|count| *count <= 1) {
                report.ok(format!(
                    "Unique element IDs: {} elements",
                    element_ids.len()
                ));
            }
        }

        let attr_nodes = form_validation_child(root, "Attributes")
            .map(|attrs| form_validation_children(attrs, "Attribute"))
            .unwrap_or_default();
        let mut attr_map = HashMap::<String, roxmltree::Node<'_, '_>>::new();
        let mut attr_ids = HashMap::<String, String>::new();
        for attr in &attr_nodes {
            let attr_name = attr.attribute("name").unwrap_or("");
            let attr_id = attr.attribute("id").unwrap_or("");
            if !attr_name.is_empty() {
                if let Some(existing) = attr_map.get(attr_name) {
                    report.error(format!(
                        "Duplicate attribute name '{attr_name}': id={attr_id} and id={}",
                        existing.attribute("id").unwrap_or("")
                    ));
                }
                attr_map.insert(attr_name.to_string(), *attr);
            }
            if !attr_id.is_empty() {
                if let Some(existing) = attr_ids.get(attr_id) {
                    report.error(format!(
                        "Duplicate attribute id={attr_id}: '{attr_name}' and '{existing}'"
                    ));
                } else {
                    attr_ids.insert(attr_id.to_string(), attr_name.to_string());
                }
            }

            if let Some(columns) = form_validation_child(*attr, "Columns") {
                let mut col_ids = HashMap::<String, String>::new();
                let mut col_names = HashMap::<String, String>::new();
                for column in form_validation_children(columns, "Column") {
                    let col_id = column.attribute("id").unwrap_or("");
                    let col_name = column.attribute("name").unwrap_or("");
                    if !col_id.is_empty() {
                        if let Some(existing) = col_ids.get(col_id) {
                            report.error(format!(
                                "Duplicate column id={col_id} in '{attr_name}': '{col_name}' and '{existing}'"
                            ));
                        } else {
                            col_ids.insert(col_id.to_string(), col_name.to_string());
                        }
                    }
                    if !col_name.is_empty() {
                        if let Some(existing) = col_names.get(col_name) {
                            report.error(format!(
                                "Duplicate column name '{col_name}' in '{attr_name}': id={col_id} and id={existing}"
                            ));
                        } else {
                            col_names.insert(col_name.to_string(), col_id.to_string());
                        }
                    }
                }
            }
        }
        if !report.stopped && !attr_ids.is_empty() {
            report.ok(format!("Unique attribute IDs: {} entries", attr_ids.len()));
        }

        let cmd_nodes = form_validation_child(root, "Commands")
            .map(|commands| form_validation_children(commands, "Command"))
            .unwrap_or_default();
        let mut cmd_map = HashMap::<String, roxmltree::Node<'_, '_>>::new();
        let mut cmd_ids = HashMap::<String, String>::new();
        for cmd in &cmd_nodes {
            let cmd_name = cmd.attribute("name").unwrap_or("");
            let cmd_id = cmd.attribute("id").unwrap_or("");
            if !cmd_name.is_empty() {
                if let Some(existing) = cmd_map.get(cmd_name) {
                    report.error(format!(
                        "Duplicate command name '{cmd_name}': id={cmd_id} and id={}",
                        existing.attribute("id").unwrap_or("")
                    ));
                }
                cmd_map.insert(cmd_name.to_string(), *cmd);
            }
            if !cmd_id.is_empty() {
                if let Some(existing) = cmd_ids.get(cmd_id) {
                    report.error(format!(
                        "Duplicate command id={cmd_id}: '{cmd_name}' and '{existing}'"
                    ));
                } else {
                    cmd_ids.insert(cmd_id.to_string(), cmd_name.to_string());
                }
            }
        }
        if !report.stopped && !cmd_ids.is_empty() {
            report.ok(format!("Unique command IDs: {} entries", cmd_ids.len()));
        }

        if !report.stopped {
            let mut param_names = HashSet::<String>::new();
            if let Some(params) = form_validation_child(root, "Parameters") {
                for param in form_validation_children(params, "Parameter") {
                    let param_name = param.attribute("name").unwrap_or("");
                    if !param_name.is_empty() && !param_names.insert(param_name.to_string()) {
                        report.error(format!("Duplicate parameter name '{param_name}'"));
                    }
                }
            }
        }

        if !report.stopped {
            form_validate_companions(&elements, &mut report);
        }
        if !report.stopped {
            form_validate_data_paths(&elements, &attr_map, has_base_form, &mut report);
        }
        if !report.stopped {
            form_validate_button_commands(&elements, &cmd_map, &mut report);
        }
        if !report.stopped {
            form_validate_events(root, &elements, &mut report);
        }
        if !report.stopped {
            form_validate_command_actions(&cmd_nodes, &mut report);
        }
        if !report.stopped {
            let main_count = attr_nodes
                .iter()
                .filter(|attr| {
                    form_validation_child_text(**attr, "MainAttribute").as_deref() == Some("true")
                })
                .count();
            if main_count <= 1 {
                let main_info = if main_count == 1 {
                    "1 main attribute"
                } else {
                    "no main attribute"
                };
                report.ok(format!("MainAttribute: {main_info}"));
            } else {
                report.error(format!(
                    "Multiple MainAttribute=true ({main_count} found, expected 0 or 1)"
                ));
            }
        }
        if !report.stopped {
            if let Some(title) = form_validation_child(root, "Title") {
                let v8_items = form_children_in_ns(title, "item", FORM_V8_NS);
                if v8_items.is_empty() && !title.text().unwrap_or("").trim().is_empty() {
                    report.error(format!(
                        "Form Title is plain text ('{}') — must be multilingual XML (<v8:item>). Use top-level 'title' key in form-compile DSL.",
                        title.text().unwrap_or("").trim()
                    ));
                } else {
                    report.ok("Title: multilingual XML");
                }
            }
        }
        if !report.stopped && has_base_form {
            form_validate_extension(root, &elements, &attr_nodes, &cmd_nodes, &mut report);
        }
        if !report.stopped && !has_base_form && form_has_call_type(root, &cmd_nodes) {
            report.warn("callType attributes found but no BaseForm — possible incorrect structure");
        }
        if !report.stopped {
            form_validate_types(root, form_is_config_context(&form_path), &mut report);
        }

        let (ok, stdout, errors) = report.finalize(&form_name);
        Ok((ok, stdout, form_path, errors))
    })();

    match result {
        Ok((ok, stdout, artifact, validation_errors)) => AdapterOutcome {
            ok,
            summary: if ok {
                "unica.form.validate completed with native form validator".to_string()
            } else {
                "unica.form.validate failed in native form validator".to_string()
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
            summary: "unica.form.validate failed in native form validator".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: Some(String::new()),
            stderr: Some(format!("{error}\n")),
            command: None,
        },
    }
}

pub(crate) fn form_validation_name(form_path: &Path) -> String {
    let parent = form_path.parent();
    if parent
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        == Some("Ext")
    {
        if let Some(form_dir) = parent
            .and_then(|path| path.parent())
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
        {
            return form_dir.to_string();
        }
    }
    form_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Form")
        .to_string()
}

pub(crate) fn form_collect_elements<'a>(
    node: roxmltree::Node<'a, 'a>,
    elements: &mut Vec<FormElementInfo<'a>>,
    element_ids: &mut HashMap<String, String>,
    element_names: &mut HashMap<String, String>,
    report: &mut FormValidationReporter,
) {
    for child in node.children().filter(|child| child.is_element()) {
        let name = child.attribute("name").unwrap_or("");
        let id = child.attribute("id").unwrap_or("");
        if !name.is_empty() && !id.is_empty() {
            let tag = child.tag_name().name().to_string();
            elements.push(FormElementInfo {
                name: name.to_string(),
                tag,
                id: id.to_string(),
                node: child,
            });
            if id != "-1" {
                if let Some(existing) = element_ids.get(id) {
                    report.error(format!(
                        "Duplicate element id={id}: '{name}' and '{existing}'"
                    ));
                } else {
                    element_ids.insert(id.to_string(), name.to_string());
                }
                if let Some(existing) = element_names.get(name) {
                    report.error(format!(
                        "Duplicate element name '{name}': id={id} and id={existing}"
                    ));
                } else {
                    element_names.insert(name.to_string(), id.to_string());
                }
            }
        }
        if let Some(child_items) = form_validation_child(child, "ChildItems") {
            form_collect_elements(child_items, elements, element_ids, element_names, report);
        }
    }
}

pub(crate) fn form_validate_companions(
    elements: &[FormElementInfo<'_>],
    report: &mut FormValidationReporter,
) {
    let mut companion_errors = 0usize;
    let mut companion_checked = 0usize;
    for element in elements {
        let required = match element.tag.as_str() {
            "InputField" | "CheckBoxField" | "LabelDecoration" | "LabelField"
            | "PictureDecoration" | "PictureField" | "CalendarField" => {
                &["ContextMenu", "ExtendedTooltip"][..]
            }
            "UsualGroup" | "Pages" | "Page" | "Button" => &["ExtendedTooltip"][..],
            "Table" => &[
                "ContextMenu",
                "AutoCommandBar",
                "SearchStringAddition",
                "ViewStatusAddition",
                "SearchControlAddition",
            ][..],
            _ => continue,
        };
        companion_checked += 1;
        for tag in required {
            if form_validation_child(element.node, tag).is_none() {
                report.error(format!(
                    "[{}] '{}': missing companion <{}>",
                    element.tag, element.name, tag
                ));
                companion_errors += 1;
            }
        }
        if report.stopped {
            return;
        }
    }
    if companion_errors == 0 && companion_checked > 0 {
        report.ok(format!(
            "Companion elements: {companion_checked} elements checked"
        ));
    }
}

pub(crate) fn form_validate_data_paths(
    elements: &[FormElementInfo<'_>],
    attr_map: &HashMap<String, roxmltree::Node<'_, '_>>,
    has_base_form: bool,
    report: &mut FormValidationReporter,
) {
    let skip_tags = [
        "ContextMenu",
        "ExtendedTooltip",
        "AutoCommandBar",
        "SearchStringAddition",
        "ViewStatusAddition",
        "SearchControlAddition",
    ];
    let binding_tags = [
        "DataPath",
        "TitleDataPath",
        "FooterDataPath",
        "HeaderDataPath",
        "MultipleValueDataPath",
        "MultipleValuePresentDataPath",
        "RowPictureDataPath",
        "MultipleValuePictureDataPath",
    ];
    let mut path_errors = 0usize;
    let mut path_checked = 0usize;
    let mut path_base_skipped = 0usize;
    for element in elements {
        if skip_tags.contains(&element.tag.as_str()) {
            continue;
        }
        if has_base_form
            && !element.id.is_empty()
            && element
                .id
                .parse::<i64>()
                .map(|id| id < 1_000_000)
                .unwrap_or(false)
        {
            path_base_skipped += 1;
            continue;
        }
        for binding_tag in binding_tags {
            let Some(data_path) = form_validation_child_text(element.node, binding_tag) else {
                continue;
            };
            let data_path = data_path.trim();
            if data_path.is_empty() || is_opaque_form_binding(data_path) {
                continue;
            }
            path_checked += 1;

            let mut clean_path = strip_form_binding_prefixes(data_path);
            let mut segments = clean_path.split('.');
            let mut root_attr = segments.next().unwrap_or("").to_string();

            if root_attr == "Items" {
                let table_name = segments.next().unwrap_or("");
                let current_data = segments.next().unwrap_or("");
                if table_name.is_empty() || current_data != "CurrentData" {
                    report.warn(format!(
                        "[{}] '{}': {}='{}' — unknown Items.* shape, expected Items.<Table>.CurrentData.*",
                        element.tag, element.name, binding_tag, data_path
                    ));
                    continue;
                }
                let Some(table_element) = elements
                    .iter()
                    .find(|candidate| candidate.tag == "Table" && candidate.name == table_name)
                else {
                    report.error(format!(
                        "[{}] '{}': {}='{}' — table element '{}' not found",
                        element.tag, element.name, binding_tag, data_path, table_name
                    ));
                    path_errors += 1;
                    if report.stopped {
                        return;
                    }
                    continue;
                };
                let Some(table_path) = form_validation_child_text(table_element.node, "DataPath")
                else {
                    continue;
                };
                let table_path = table_path.trim();
                if table_path.is_empty() {
                    continue;
                }
                clean_path = strip_form_binding_prefixes(table_path);
                root_attr = clean_path.split('.').next().unwrap_or("").to_string();
            }

            if !attr_map.contains_key(root_attr.as_str()) {
                report.error(format!(
                    "[{}] '{}': {}='{}' — attribute '{}' not found",
                    element.tag, element.name, binding_tag, data_path, root_attr
                ));
                path_errors += 1;
            }
            if report.stopped {
                return;
            }
        }
    }
    let mut path_msg = String::new();
    if path_checked > 0 {
        path_msg = format!("{path_checked} paths checked");
    }
    if path_base_skipped > 0 {
        let skip_note = format!("{path_base_skipped} base skipped");
        path_msg = if path_msg.is_empty() {
            skip_note
        } else {
            format!("{path_msg}, {skip_note}")
        };
    }
    if path_errors == 0 && !path_msg.is_empty() {
        report.ok(format!("Binding path references: {path_msg}"));
    }
}

pub(crate) fn strip_form_binding_prefixes(value: &str) -> String {
    strip_numeric_indexes(value)
        .trim_start_matches('~')
        .to_string()
}

pub(crate) fn is_opaque_form_binding(value: &str) -> bool {
    if value.chars().all(|ch| ch.is_ascii_digit()) {
        return true;
    }
    let Some((prefix, uuid)) = value.split_once(':') else {
        return false;
    };
    let Some((left, right)) = prefix.split_once('/') else {
        return false;
    };
    !left.is_empty()
        && !right.is_empty()
        && left.chars().all(|ch| ch.is_ascii_digit())
        && right.chars().all(|ch| ch.is_ascii_digit())
        && !uuid.is_empty()
        && uuid.chars().all(|ch| ch.is_ascii_hexdigit() || ch == '-')
}

pub(crate) fn strip_numeric_indexes(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '[' {
            let mut digits = String::new();
            while let Some(next) = chars.peek().copied() {
                chars.next();
                if next == ']' {
                    break;
                }
                digits.push(next);
            }
            if digits.chars().all(|digit| digit.is_ascii_digit()) {
                continue;
            }
            result.push('[');
            result.push_str(&digits);
            result.push(']');
        } else {
            result.push(ch);
        }
    }
    result
}

pub(crate) fn form_validate_button_commands(
    elements: &[FormElementInfo<'_>],
    cmd_map: &HashMap<String, roxmltree::Node<'_, '_>>,
    report: &mut FormValidationReporter,
) {
    let mut cmd_errors = 0usize;
    let mut cmd_checked = 0usize;
    for element in elements.iter().filter(|element| element.tag == "Button") {
        let Some(cmd_ref) = form_validation_child_text(element.node, "CommandName") else {
            continue;
        };
        let Some(cmd_name) = cmd_ref.strip_prefix("Form.Command.") else {
            continue;
        };
        cmd_checked += 1;
        if !cmd_map.contains_key(cmd_name) {
            report.error(format!(
                "[Button] '{}': CommandName='{}' — command '{}' not found in Commands",
                element.name, cmd_ref, cmd_name
            ));
            cmd_errors += 1;
        }
        if report.stopped {
            return;
        }
    }
    if cmd_errors == 0 && cmd_checked > 0 {
        report.ok(format!("Command references: {cmd_checked} buttons checked"));
    }
}

pub(crate) fn form_validate_events(
    root: roxmltree::Node<'_, '_>,
    elements: &[FormElementInfo<'_>],
    report: &mut FormValidationReporter,
) {
    let mut event_errors = 0usize;
    let mut event_checked = 0usize;
    if let Some(form_events) = form_validation_child(root, "Events") {
        for event in form_validation_children(form_events, "Event") {
            let name = event.attribute("name").unwrap_or("");
            event_checked += 1;
            if event.text().unwrap_or("").trim().is_empty() {
                report.error(format!("Form event '{name}': empty handler name"));
                event_errors += 1;
            }
        }
    }
    for element in elements {
        if let Some(events) = form_validation_child(element.node, "Events") {
            for event in form_validation_children(events, "Event") {
                let event_name = event.attribute("name").unwrap_or("");
                event_checked += 1;
                if event.text().unwrap_or("").trim().is_empty() {
                    report.error(format!(
                        "[{}] '{}' event '{}': empty handler name",
                        element.tag, element.name, event_name
                    ));
                    event_errors += 1;
                }
                if report.stopped {
                    return;
                }
            }
        }
    }
    if event_errors == 0 && event_checked > 0 {
        report.ok(format!("Event handlers: {event_checked} events checked"));
    }
}

pub(crate) fn form_validate_command_actions(
    cmd_nodes: &[roxmltree::Node<'_, '_>],
    report: &mut FormValidationReporter,
) {
    let mut action_errors = 0usize;
    let mut action_checked = 0usize;
    for command in cmd_nodes {
        let cmd_name = command.attribute("name").unwrap_or("");
        let actions = form_validation_children(*command, "Action");
        action_checked += 1;
        if actions
            .first()
            .and_then(|action| action.text())
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .is_none()
        {
            report.error(format!("Command '{cmd_name}': missing or empty Action"));
            action_errors += 1;
        }
        if report.stopped {
            return;
        }
    }
    if action_errors == 0 && action_checked > 0 {
        report.ok(format!(
            "Command actions: {action_checked} commands checked"
        ));
    }
}

pub(crate) fn form_validate_extension(
    root: roxmltree::Node<'_, '_>,
    elements: &[FormElementInfo<'_>],
    attr_nodes: &[roxmltree::Node<'_, '_>],
    cmd_nodes: &[roxmltree::Node<'_, '_>],
    report: &mut FormValidationReporter,
) {
    let Some(base_form) = form_validation_child(root, "BaseForm") else {
        return;
    };
    if let Some(version) = base_form
        .attribute("version")
        .filter(|value| !value.is_empty())
    {
        report.ok(format!("BaseForm: version={version}"));
    } else {
        report.warn("BaseForm: version attribute missing");
    }

    let mut ct_errors = 0usize;
    let mut ct_checked = 0usize;
    for event in form_validation_child(root, "Events")
        .map(|events| form_validation_children(events, "Event"))
        .unwrap_or_default()
    {
        if let Some(call_type) = event
            .attribute("callType")
            .filter(|value| !value.is_empty())
        {
            ct_checked += 1;
            if !form_valid_call_type(call_type) {
                report.error(format!(
                    "Form event '{}': invalid callType='{}' (expected: Before, After, Override)",
                    event.attribute("name").unwrap_or(""),
                    call_type
                ));
                ct_errors += 1;
            }
        }
    }
    for element in elements {
        if let Some(events) = form_validation_child(element.node, "Events") {
            for event in form_validation_children(events, "Event") {
                if let Some(call_type) = event
                    .attribute("callType")
                    .filter(|value| !value.is_empty())
                {
                    ct_checked += 1;
                    if !form_valid_call_type(call_type) {
                        report.error(format!(
                            "[{}] '{}' event '{}': invalid callType='{}'",
                            element.tag,
                            element.name,
                            event.attribute("name").unwrap_or(""),
                            call_type
                        ));
                        ct_errors += 1;
                    }
                }
            }
        }
    }
    for command in cmd_nodes {
        let cmd_name = command.attribute("name").unwrap_or("");
        for action in form_validation_children(*command, "Action") {
            if let Some(call_type) = action
                .attribute("callType")
                .filter(|value| !value.is_empty())
            {
                ct_checked += 1;
                if !form_valid_call_type(call_type) {
                    report.error(format!(
                        "Command '{cmd_name}' Action: invalid callType='{call_type}'"
                    ));
                    ct_errors += 1;
                }
            }
        }
    }
    if !report.stopped && ct_errors == 0 && ct_checked > 0 {
        report.ok(format!("callType values: {ct_checked} checked"));
    }

    let base_attr_names = form_validation_child(base_form, "Attributes")
        .map(|attrs| {
            form_validation_children(attrs, "Attribute")
                .into_iter()
                .filter_map(|attr| attr.attribute("name").map(ToOwned::to_owned))
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let base_cmd_names = form_validation_child(base_form, "Commands")
        .map(|commands| {
            form_validation_children(commands, "Command")
                .into_iter()
                .filter_map(|cmd| cmd.attribute("name").map(ToOwned::to_owned))
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let mut id_warn_count = 0usize;
    for attr in attr_nodes {
        let name = attr.attribute("name").unwrap_or("");
        let id = attr.attribute("id").unwrap_or("");
        if !name.is_empty()
            && !base_attr_names.contains(name)
            && id.parse::<i64>().map(|id| id < 1_000_000).unwrap_or(false)
        {
            report.warn(format!(
                "Attribute '{name}' (id={id}): extension-added attribute has id < 1000000"
            ));
            id_warn_count += 1;
        }
    }
    for command in cmd_nodes {
        let name = command.attribute("name").unwrap_or("");
        let id = command.attribute("id").unwrap_or("");
        if !name.is_empty()
            && !base_cmd_names.contains(name)
            && id.parse::<i64>().map(|id| id < 1_000_000).unwrap_or(false)
        {
            report.warn(format!(
                "Command '{name}' (id={id}): extension-added command has id < 1000000"
            ));
            id_warn_count += 1;
        }
    }
    if !report.stopped && id_warn_count == 0 {
        let ext_attr_count = attr_nodes
            .iter()
            .filter(|attr| {
                attr.attribute("name")
                    .is_some_and(|name| !base_attr_names.contains(name))
            })
            .count();
        let ext_cmd_count = cmd_nodes
            .iter()
            .filter(|cmd| {
                cmd.attribute("name")
                    .is_some_and(|name| !base_cmd_names.contains(name))
            })
            .count();
        if ext_attr_count + ext_cmd_count > 0 {
            report.ok(format!(
                "Extension ID ranges: {ext_attr_count} attr(s), {ext_cmd_count} cmd(s) — all >= 1000000"
            ));
        }
    }
}

pub(crate) fn form_valid_call_type(call_type: &str) -> bool {
    matches!(call_type, "Before" | "After" | "Override")
}

pub(crate) fn form_has_call_type(
    root: roxmltree::Node<'_, '_>,
    cmd_nodes: &[roxmltree::Node<'_, '_>],
) -> bool {
    form_validation_child(root, "Events")
        .map(|events| {
            form_validation_children(events, "Event")
                .iter()
                .any(|event| {
                    event
                        .attribute("callType")
                        .is_some_and(|value| !value.is_empty())
                })
        })
        .unwrap_or(false)
        || cmd_nodes.iter().any(|cmd| {
            form_validation_children(*cmd, "Action")
                .iter()
                .any(|action| {
                    action
                        .attribute("callType")
                        .is_some_and(|value| !value.is_empty())
                })
        })
}

pub(crate) fn form_validate_types(
    root: roxmltree::Node<'_, '_>,
    is_config_context: bool,
    report: &mut FormValidationReporter,
) {
    let type_nodes = root
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "Type")
        .collect::<Vec<_>>();
    let mut type_error_count = 0usize;
    let mut type_warn_count = 0usize;
    for type_node in &type_nodes {
        let value = type_node.text().unwrap_or("").trim();
        if value.is_empty() {
            continue;
        }
        if form_invalid_types().contains(&value) {
            report.error(format!(
                "12. Type \"{value}\": invalid runtime/UI type (not valid in XDTO schema)"
            ));
            type_error_count += 1;
        } else if form_valid_closed_types().contains(&value) {
        } else if let Some(suffix) = value.strip_prefix("cfg:") {
            let prefix = suffix.split('.').next().unwrap_or("");
            if form_valid_cfg_prefixes().contains(&prefix) || suffix == "DynamicList" {
                if is_config_context
                    && matches!(
                        prefix,
                        "ExternalDataProcessorObject" | "ExternalReportObject"
                    )
                {
                    report.error(format!(
                        "12. Type \"{value}\": External* type in configuration context (use DataProcessorObject/ReportObject instead)"
                    ));
                    type_error_count += 1;
                }
            } else {
                report.warn(format!("12. Type \"{value}\": unrecognized cfg prefix"));
                type_warn_count += 1;
            }
        } else if value.contains(':') {
        } else {
            report.error(format!(
                "12. Type \"{value}\": bare type without namespace prefix"
            ));
            type_error_count += 1;
        }
        if report.stopped {
            return;
        }
    }
    if type_error_count == 0 && type_warn_count == 0 {
        if type_nodes.is_empty() {
            report.ok("12. Types: no type values to check");
        } else {
            report.ok(format!("12. Types: {} values, all valid", type_nodes.len()));
        }
    }
}

pub(crate) fn form_is_config_context(form_path: &Path) -> bool {
    let mut walk_dir = form_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf();
    for _ in 0..15 {
        if walk_dir.join("Configuration.xml").is_file() {
            return true;
        }
        let Some(parent) = walk_dir.parent() else {
            break;
        };
        if parent == walk_dir {
            break;
        }
        walk_dir = parent.to_path_buf();
    }
    false
}

pub(crate) fn form_invalid_types() -> &'static [&'static str] {
    &[
        "FormDataStructure",
        "FormDataCollection",
        "FormDataTree",
        "FormDataTreeItem",
        "FormDataCollectionItem",
        "FormGroup",
        "FormField",
        "FormButton",
        "FormDecoration",
        "FormTable",
    ]
}

pub(crate) fn form_valid_closed_types() -> &'static [&'static str] {
    &[
        "xs:boolean",
        "xs:string",
        "xs:decimal",
        "xs:dateTime",
        "xs:binary",
        "v8:FillChecking",
        "v8:Null",
        "v8:StandardPeriod",
        "v8:StandardBeginningDate",
        "v8:Type",
        "v8:TypeDescription",
        "v8:UUID",
        "v8:ValueListType",
        "v8:ValueTable",
        "v8:ValueTree",
        "v8:Universal",
        "v8:FixedArray",
        "v8:FixedStructure",
        "v8ui:Color",
        "v8ui:Font",
        "v8ui:FormattedString",
        "v8ui:HorizontalAlign",
        "v8ui:Picture",
        "v8ui:SizeChangeMode",
        "v8ui:VerticalAlign",
        "dcsset:DataCompositionComparisonType",
        "dcsset:DataCompositionFieldPlacement",
        "dcsset:Filter",
        "dcsset:SettingsComposer",
        "dcsset:DataCompositionSettings",
        "dcssch:DataCompositionSchema",
        "dcscor:DataCompositionComparisonType",
        "dcscor:DataCompositionGroupType",
        "dcscor:DataCompositionPeriodAdditionType",
        "dcscor:DataCompositionSortDirection",
        "dcscor:Field",
        "ent:AccountType",
        "ent:AccumulationRecordType",
        "ent:AccountingRecordType",
    ]
}

pub(crate) fn form_valid_cfg_prefixes() -> &'static [&'static str] {
    &[
        "AccountingRegisterRecordSet",
        "AccumulationRegisterRecordSet",
        "BusinessProcessObject",
        "BusinessProcessRef",
        "CatalogObject",
        "CatalogRef",
        "ChartOfAccountsObject",
        "ChartOfAccountsRef",
        "ChartOfCalculationTypesObject",
        "ChartOfCalculationTypesRef",
        "ChartOfCharacteristicTypesObject",
        "ChartOfCharacteristicTypesRef",
        "ConstantsSet",
        "DataProcessorObject",
        "DocumentObject",
        "DocumentRef",
        "DynamicList",
        "EnumRef",
        "ExchangePlanObject",
        "ExchangePlanRef",
        "ExternalDataProcessorObject",
        "ExternalReportObject",
        "InformationRegisterRecordManager",
        "InformationRegisterRecordSet",
        "ReportObject",
        "TaskObject",
        "TaskRef",
    ]
}

pub(crate) fn analyze_form_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let result = (|| -> Result<(String, PathBuf), String> {
        let raw_path = required_path(args, &["formPath", "FormPath", "path", "Path"], "FormPath")?;
        let form_path = resolve_form_info_path(absolutize(raw_path, &context.cwd));
        if !form_path.is_file() {
            return Err(format!("File not found: {}", form_path.display()));
        }

        let limit = int_arg(args, &["limit", "Limit"])
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(150);
        let offset = int_arg(args, &["offset", "Offset"])
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(0);
        let expand = string_arg(args, &["expand", "Expand"]).unwrap_or("");

        let text = read_utf8_sig(&form_path)?;
        let doc = Document::parse(text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("XML parse error in {}: {err}", form_path.display()))?;
        let root = doc.root_element();
        let is_extension = form_child(root, "BaseForm").is_some();
        let (form_name, object_context) = form_info_context(&form_path);

        let mut lines = Vec::new();
        let form_title = form_child(root, "Title")
            .map(form_ml_text)
            .filter(|value| !value.is_empty());
        let ext_marker = if is_extension { " [EXTENSION]" } else { "" };
        let mut header = format!("=== Form: {form_name}{ext_marker}");
        if let Some(title) = form_title {
            header.push_str(&format!(" — \"{title}\""));
        }
        if !object_context.is_empty() {
            header.push_str(&format!(" ({object_context})"));
        }
        header.push_str(" ===");
        lines.push(header);
        lines.push(format!(
            "Поддержка: {}",
            support_status_for_path(&form_path)
        ));

        let prop_names = [
            "Width",
            "Height",
            "Group",
            "WindowOpeningMode",
            "EnterKeyBehavior",
            "AutoTitle",
            "AutoURL",
            "AutoFillCheck",
            "Customizable",
            "CommandBarLocation",
            "SaveDataInSettings",
            "AutoSaveDataInSettings",
            "AutoTime",
            "UsePostingMode",
            "RepostOnWrite",
            "UseForFoldersAndItems",
            "ReportResult",
            "DetailsData",
            "ReportFormType",
            "VerticalScroll",
            "ScalingMode",
        ];
        let props = prop_names
            .iter()
            .filter_map(|name| {
                form_child(root, name).and_then(|node| {
                    let value = form_ml_text(node);
                    if value.is_empty() {
                        None
                    } else {
                        Some(format!("{name}={value}"))
                    }
                })
            })
            .collect::<Vec<_>>();
        if !props.is_empty() {
            lines.push(String::new());
            lines.push(format!("Properties: {}", props.join(", ")));
        }

        if let Some(events) = form_child(root, "Events") {
            let event_lines = form_event_lines(events);
            if !event_lines.is_empty() {
                lines.push(String::new());
                lines.push("Events:".to_string());
                lines.extend(event_lines);
            }
        }

        let cb_loc = form_child_text(root, "CommandBarLocation")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "Auto".to_string());
        let acb_lines = if cb_loc != "None" {
            form_child(root, "AutoCommandBar")
                .map(form_main_command_bar_lines)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if !acb_lines.is_empty() && matches!(cb_loc.as_str(), "Auto" | "Top") {
            lines.push(String::new());
            lines.extend(acb_lines.clone());
        }

        let mut tree_state = FormTreeState {
            has_collapsed: false,
        };
        if let Some(child_items) = form_child(root, "ChildItems") {
            let mut tree_lines = Vec::new();
            form_build_tree(child_items, "  ", &mut tree_lines, expand, &mut tree_state);
            lines.push(String::new());
            lines.push("Elements:".to_string());
            lines.extend(tree_lines);
        }

        if !acb_lines.is_empty() && cb_loc == "Bottom" {
            lines.push(String::new());
            lines.extend(acb_lines);
        }

        if let Some(attrs) = form_child(root, "Attributes") {
            let attr_lines = form_attribute_lines(attrs);
            if !attr_lines.is_empty() {
                lines.push(String::new());
                lines.push("Attributes:".to_string());
                lines.extend(attr_lines);
            }
        }

        if let Some(params) = form_child(root, "Parameters") {
            let param_lines = form_parameter_lines(params);
            if !param_lines.is_empty() {
                lines.push(String::new());
                lines.push("Parameters:".to_string());
                lines.extend(param_lines);
            }
        }

        if let Some(commands) = form_child(root, "Commands") {
            let command_lines = form_command_lines(commands);
            if !command_lines.is_empty() {
                lines.push(String::new());
                lines.push("Commands:".to_string());
                lines.extend(command_lines);
            }
        }

        if is_extension {
            let base_form = form_child(root, "BaseForm").expect("checked above");
            let version = base_form.attribute("version").unwrap_or("");
            let base_form_text = if version.is_empty() {
                "present".to_string()
            } else {
                format!("present (version {version})")
            };
            lines.push(String::new());
            lines.push(format!("BaseForm: {base_form_text}"));
        }

        if tree_state.has_collapsed {
            lines.push(String::new());
            lines.push(
                "Hint: use -Expand <name> to expand a collapsed section, -Expand * for all"
                    .to_string(),
            );
        }

        let total_lines = lines.len();
        if offset > 0 {
            if offset >= total_lines {
                return Ok((
                    format!(
                        "[INFO] Offset {offset} exceeds total lines ({total_lines}). Nothing to show.\n"
                    ),
                    form_path,
                ));
            }
            lines = lines.into_iter().skip(offset).collect();
        }

        let stdout = if lines.len() > limit {
            let shown = lines.iter().take(limit).cloned().collect::<Vec<_>>();
            format!(
                "{}\n\n[TRUNCATED] Shown {limit} of {total_lines} lines. Use -Offset {} to continue.\n",
                shown.join("\n"),
                offset + limit
            )
        } else {
            format!("{}\n", lines.join("\n"))
        };

        Ok((stdout, form_path))
    })();

    match result {
        Ok((stdout, artifact)) => AdapterOutcome {
            ok: true,
            summary: "unica.form.info completed with native form analyzer".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![artifact.display().to_string()],
            stdout: Some(stdout),
            stderr: Some(String::new()),
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.form.info failed in native form analyzer".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: Some(String::new()),
            stderr: Some(format!("{error}\n")),
            command: None,
        },
    }
}

pub(crate) fn form_info_context(form_path: &Path) -> (String, String) {
    let resolved = form_path
        .canonicalize()
        .unwrap_or_else(|_| form_path.to_path_buf());
    let parts = resolved
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => part.to_str().map(ToOwned::to_owned),
            _ => None,
        })
        .collect::<Vec<_>>();
    if let Some(forms_idx) = parts.iter().rposition(|part| part == "Forms") {
        if forms_idx + 1 < parts.len() {
            let form_name = parts[forms_idx + 1].clone();
            let object_context = if forms_idx >= 2 {
                format!("{}.{}", parts[forms_idx - 2], parts[forms_idx - 1])
            } else {
                String::new()
            };
            return (form_name, object_context);
        }
    }
    if let Some(ext_idx) = parts.iter().rposition(|part| part == "Ext") {
        if ext_idx >= 2 {
            return (parts[ext_idx - 1].clone(), parts[ext_idx - 2].clone());
        }
    }
    (
        form_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("Form")
            .to_string(),
        String::new(),
    )
}

pub(crate) fn form_child<'a>(
    node: roxmltree::Node<'a, 'a>,
    local_name: &str,
) -> Option<roxmltree::Node<'a, 'a>> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == local_name)
}

pub(crate) fn form_child_in_ns<'a>(
    node: roxmltree::Node<'a, 'a>,
    local_name: &str,
    namespace: &str,
) -> Option<roxmltree::Node<'a, 'a>> {
    node.children().find(|child| {
        child.is_element()
            && child.tag_name().name() == local_name
            && child.tag_name().namespace() == Some(namespace)
    })
}

pub(crate) fn form_validation_child<'a>(
    node: roxmltree::Node<'a, 'a>,
    local_name: &str,
) -> Option<roxmltree::Node<'a, 'a>> {
    form_child_in_ns(node, local_name, FORM_LOGFORM_NS)
}

pub(crate) fn form_children<'a>(
    node: roxmltree::Node<'a, 'a>,
    local_name: &str,
) -> Vec<roxmltree::Node<'a, 'a>> {
    node.children()
        .filter(|child| child.is_element() && child.tag_name().name() == local_name)
        .collect()
}

pub(crate) fn form_children_in_ns<'a>(
    node: roxmltree::Node<'a, 'a>,
    local_name: &str,
    namespace: &str,
) -> Vec<roxmltree::Node<'a, 'a>> {
    node.children()
        .filter(|child| {
            child.is_element()
                && child.tag_name().name() == local_name
                && child.tag_name().namespace() == Some(namespace)
        })
        .collect()
}

pub(crate) fn form_validation_children<'a>(
    node: roxmltree::Node<'a, 'a>,
    local_name: &str,
) -> Vec<roxmltree::Node<'a, 'a>> {
    form_children_in_ns(node, local_name, FORM_LOGFORM_NS)
}

pub(crate) fn form_child_text(node: roxmltree::Node<'_, '_>, local_name: &str) -> Option<String> {
    form_child(node, local_name)
        .map(form_ml_text)
        .filter(|value| !value.is_empty())
}

pub(crate) fn form_validation_child_text(
    node: roxmltree::Node<'_, '_>,
    local_name: &str,
) -> Option<String> {
    form_validation_child(node, local_name)
        .map(form_ml_text)
        .filter(|value| !value.is_empty())
}

pub(crate) fn form_ml_text(node: roxmltree::Node<'_, '_>) -> String {
    for item in form_children(node, "item") {
        for child in item.children().filter(|child| child.is_element()) {
            if child.tag_name().name() == "content" {
                if let Some(text) = child.text() {
                    if !text.is_empty() {
                        return text.to_string();
                    }
                }
            }
        }
    }
    node.text().unwrap_or("").trim().to_string()
}

pub(crate) fn form_event_lines(events: roxmltree::Node<'_, '_>) -> Vec<String> {
    form_children(events, "Event")
        .into_iter()
        .map(|event| {
            let name = event.attribute("name").unwrap_or("");
            let handler = event.text().unwrap_or("");
            let call_type = event.attribute("callType").unwrap_or("");
            let call_type = if call_type.is_empty() {
                String::new()
            } else {
                format!("[{call_type}]")
            };
            format!("  {name}{call_type} -> {handler}")
        })
        .collect()
}

pub(crate) fn form_main_command_bar_lines(acb: roxmltree::Node<'_, '_>) -> Vec<String> {
    let autofill = form_child_text(acb, "Autofill")
        .map(|value| value != "false")
        .unwrap_or(true);
    let h_align = form_child_text(acb, "HorizontalAlign");
    let mut flags = vec![if autofill { "autofill" } else { "no-autofill" }.to_string()];
    if let Some(align) = h_align {
        flags.push(format!("align={align}"));
    }

    let mut buttons = Vec::new();
    if let Some(child_items) = form_child(acb, "ChildItems") {
        for button in child_items.children().filter(|child| {
            child.is_element() && !form_skip_elements().contains(&child.tag_name().name())
        }) {
            let name = button.attribute("name").unwrap_or("");
            let cmd_ref = form_child_text(button, "CommandName").unwrap_or_default();
            let loc = form_child_text(button, "LocationInCommandBar")
                .map(|value| format!(" [{value}]"))
                .unwrap_or_default();
            let tag = form_element_tag(button);
            if cmd_ref.is_empty() {
                buttons.push(format!("  {tag} {name}{loc}"));
            } else {
                buttons.push(format!("  {tag} {name} -> {cmd_ref}{loc}"));
            }
        }
    }
    if buttons.is_empty() && autofill && flags.len() == 1 {
        return vec!["AutoCommandBar [autofill]".to_string()];
    }
    let mut lines = vec![format!("AutoCommandBar [{}]", flags.join(", "))];
    lines.extend(buttons);
    lines
}

pub(crate) struct FormTreeState {
    pub(crate) has_collapsed: bool,
}

pub(crate) fn form_build_tree(
    child_items: roxmltree::Node<'_, '_>,
    prefix: &str,
    tree_lines: &mut Vec<String>,
    expand: &str,
    state: &mut FormTreeState,
) {
    let children = child_items
        .children()
        .filter(|child| {
            child.is_element() && !form_skip_elements().contains(&child.tag_name().name())
        })
        .collect::<Vec<_>>();

    for (index, child) in children.iter().enumerate() {
        let last = index + 1 == children.len();
        let connector = if last { "└─" } else { "├─" };
        let continuation = if last { "  " } else { "│ " };
        let tag = form_element_tag(*child);
        let name = child.attribute("name").unwrap_or("");
        let flags = form_flags(*child);
        let events = form_events_str(*child);
        let binding = form_binding(*child);
        let title = form_title_differs(*child, name)
            .map(|title| format!(" [title:{title}]"))
            .unwrap_or_default();
        tree_lines.push(format!(
            "{prefix}{connector} {tag} {name}{binding}{flags}{title}{events}"
        ));

        match child.tag_name().name() {
            "Page" => {
                let child_items = form_child(*child, "ChildItems");
                let page_name = child.attribute("name").unwrap_or("");
                let page_title = form_title_differs(*child, page_name);
                let should_expand = expand == "*"
                    || expand == page_name
                    || page_title.as_deref().is_some_and(|title| expand == title);
                if should_expand {
                    if let Some(child_items) = child_items {
                        form_build_tree(
                            child_items,
                            &format!("{prefix}{continuation}"),
                            tree_lines,
                            expand,
                            state,
                        );
                    }
                } else {
                    let count = child_items
                        .map(form_count_significant_children)
                        .unwrap_or(0);
                    if let Some(line) = tree_lines.last_mut() {
                        line.push_str(&format!(" ({count} items)"));
                    }
                    state.has_collapsed = true;
                }
            }
            "UsualGroup" | "Pages" | "Table" | "CommandBar" | "ButtonGroup" | "Popup" => {
                if let Some(child_items) = form_child(*child, "ChildItems") {
                    form_build_tree(
                        child_items,
                        &format!("{prefix}{continuation}"),
                        tree_lines,
                        expand,
                        state,
                    );
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn form_skip_elements() -> &'static [&'static str] {
    &[
        "ExtendedTooltip",
        "ContextMenu",
        "AutoCommandBar",
        "SearchStringAddition",
        "ViewStatusAddition",
        "SearchControlAddition",
        "ColumnGroup",
    ]
}

pub(crate) fn form_element_tag(node: roxmltree::Node<'_, '_>) -> String {
    match node.tag_name().name() {
        "UsualGroup" => {
            let orient = match form_child_text(node, "Group").as_deref() {
                Some("Vertical") => ":V",
                Some("Horizontal") => ":H",
                Some("AlwaysHorizontal") => ":AH",
                Some("AlwaysVertical") => ":AV",
                _ => "",
            };
            let collapse = if form_child_text(node, "Behavior").as_deref() == Some("Collapsible") {
                ",collapse"
            } else {
                ""
            };
            format!("[Group{orient}{collapse}]")
        }
        "InputField" => "[Input]".to_string(),
        "CheckBoxField" => "[Check]".to_string(),
        "LabelDecoration" => "[Label]".to_string(),
        "LabelField" => "[LabelField]".to_string(),
        "PictureDecoration" => "[Picture]".to_string(),
        "PictureField" => "[PicField]".to_string(),
        "CalendarField" => "[Calendar]".to_string(),
        "Table" => "[Table]".to_string(),
        "Button" => "[Button]".to_string(),
        "CommandBar" => "[CmdBar]".to_string(),
        "Pages" => "[Pages]".to_string(),
        "Page" => "[Page]".to_string(),
        "Popup" => "[Popup]".to_string(),
        "ButtonGroup" => "[BtnGroup]".to_string(),
        other => format!("[{other}]"),
    }
}

pub(crate) fn form_flags(node: roxmltree::Node<'_, '_>) -> String {
    let mut flags = Vec::new();
    if form_child_text(node, "Visible").as_deref() == Some("false") {
        flags.push("visible:false");
    }
    if form_child_text(node, "Enabled").as_deref() == Some("false") {
        flags.push("enabled:false");
    }
    if form_child_text(node, "ReadOnly").as_deref() == Some("true") {
        flags.push("ro");
    }
    if flags.is_empty() {
        String::new()
    } else {
        format!(" [{}]", flags.join(","))
    }
}

pub(crate) fn form_events_str(node: roxmltree::Node<'_, '_>) -> String {
    let Some(events) = form_child(node, "Events") else {
        return String::new();
    };
    let events = form_children(events, "Event")
        .into_iter()
        .map(|event| {
            let name = event.attribute("name").unwrap_or("");
            let call_type = event.attribute("callType").unwrap_or("");
            if call_type.is_empty() {
                name.to_string()
            } else {
                format!("{name}[{call_type}]")
            }
        })
        .collect::<Vec<_>>();
    if events.is_empty() {
        String::new()
    } else {
        format!(" {{{}}}", events.join(", "))
    }
}

pub(crate) fn form_binding(node: roxmltree::Node<'_, '_>) -> String {
    if let Some(data_path) = form_child_text(node, "DataPath") {
        return format!(" -> {data_path}");
    }
    let Some(command_name) = form_child_text(node, "CommandName") else {
        return String::new();
    };
    if let Some(name) = command_name.strip_prefix("Form.StandardCommand.") {
        format!(" -> {name} [std]")
    } else if let Some(name) = command_name.strip_prefix("Form.Command.") {
        format!(" -> {name} [cmd]")
    } else {
        format!(" -> {command_name}")
    }
}

pub(crate) fn form_title_differs(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    let title = form_child(node, "Title").map(form_ml_text)?;
    if title.is_empty() || title.replace(' ', "").to_lowercase() == name.to_lowercase() {
        None
    } else {
        Some(title)
    }
}

pub(crate) fn form_count_significant_children(child_items: roxmltree::Node<'_, '_>) -> usize {
    child_items
        .children()
        .filter(|child| {
            child.is_element() && !form_skip_elements().contains(&child.tag_name().name())
        })
        .count()
}

pub(crate) fn form_attribute_lines(attrs: roxmltree::Node<'_, '_>) -> Vec<String> {
    form_children(attrs, "Attribute")
        .into_iter()
        .map(|attr| {
            let name = attr.attribute("name").unwrap_or("");
            let type_str = form_child(attr, "Type")
                .map(form_format_type)
                .unwrap_or_default();
            let is_main = form_child_text(attr, "MainAttribute").as_deref() == Some("true");
            let prefix = if is_main { "*" } else { " " };
            let main_suffix = if is_main { " (main)" } else { "" };
            let mut dyn_table = String::new();
            if type_str == "DynamicList" {
                if let Some(settings) = form_child(attr, "Settings") {
                    if let Some(main_table) = form_child_text(settings, "MainTable") {
                        dyn_table = format!(" -> {main_table}");
                    }
                }
            }
            let mut col_str = String::new();
            if matches!(type_str.as_str(), "ValueTable" | "ValueTree") {
                if let Some(columns) = form_child(attr, "Columns") {
                    let cols = form_children(columns, "Column")
                        .into_iter()
                        .map(|column| {
                            let column_name = column.attribute("name").unwrap_or("");
                            let column_type = form_child(column, "Type")
                                .map(form_format_type)
                                .unwrap_or_default();
                            if column_type.is_empty() {
                                column_name.to_string()
                            } else {
                                format!("{column_name}: {column_type}")
                            }
                        })
                        .collect::<Vec<_>>();
                    if !cols.is_empty() {
                        col_str = format!(" [{}]", cols.join(", "));
                    }
                }
            }
            if type_str.is_empty() && col_str.is_empty() && dyn_table.is_empty() {
                format!("  {prefix}{name}{main_suffix}")
            } else {
                format!("  {prefix}{name}: {type_str}{col_str}{dyn_table}{main_suffix}")
            }
        })
        .collect()
}

pub(crate) fn form_parameter_lines(params: roxmltree::Node<'_, '_>) -> Vec<String> {
    form_children(params, "Parameter")
        .into_iter()
        .map(|param| {
            let name = param.attribute("name").unwrap_or("");
            let type_str = form_child(param, "Type")
                .map(form_format_type)
                .unwrap_or_default();
            let key_suffix = if form_child_text(param, "KeyParameter").as_deref() == Some("true") {
                " (key)"
            } else {
                ""
            };
            if type_str.is_empty() {
                format!("  {name}{key_suffix}")
            } else {
                format!("  {name}: {type_str}{key_suffix}")
            }
        })
        .collect()
}

pub(crate) fn form_command_lines(commands: roxmltree::Node<'_, '_>) -> Vec<String> {
    form_children(commands, "Command")
        .into_iter()
        .map(|command| {
            let name = command.attribute("name").unwrap_or("");
            let shortcut = form_child_text(command, "Shortcut")
                .map(|value| format!(" [{value}]"))
                .unwrap_or_default();
            let actions = form_children(command, "Action");
            let action = if actions.len() > 1 {
                let parts = actions
                    .into_iter()
                    .map(|action| {
                        let text = action.text().unwrap_or("");
                        let call_type = action.attribute("callType").unwrap_or("");
                        if call_type.is_empty() {
                            text.to_string()
                        } else {
                            format!("{text}[{call_type}]")
                        }
                    })
                    .collect::<Vec<_>>();
                format!(" -> {}", parts.join(", "))
            } else if actions.len() == 1 {
                let action_node = actions[0];
                let text = action_node.text().unwrap_or("");
                let call_type = action_node.attribute("callType").unwrap_or("");
                if call_type.is_empty() {
                    format!(" -> {text}")
                } else {
                    format!(" -> {text}[{call_type}]")
                }
            } else {
                String::new()
            };
            format!("  {name}{action}{shortcut}")
        })
        .collect()
}

pub(crate) fn form_format_type(type_node: roxmltree::Node<'_, '_>) -> String {
    if let Some(type_set) = form_child_text(type_node, "TypeSet") {
        return type_set
            .strip_prefix("cfg:")
            .unwrap_or(&type_set)
            .to_string();
    }
    let mut parts = Vec::new();
    for type_item in form_children(type_node, "Type") {
        let raw = type_item.text().unwrap_or("");
        let part = match raw {
            "xs:string" => {
                let length = form_child(type_node, "StringQualifiers")
                    .and_then(|node| form_child_text(node, "Length"))
                    .unwrap_or_else(|| "0".to_string());
                if length != "0" {
                    format!("string({length})")
                } else {
                    "string".to_string()
                }
            }
            "xs:decimal" => {
                if let Some(qualifiers) = form_child(type_node, "NumberQualifiers") {
                    let digits =
                        form_child_text(qualifiers, "Digits").unwrap_or_else(|| "0".to_string());
                    let fraction = form_child_text(qualifiers, "FractionDigits")
                        .unwrap_or_else(|| "0".to_string());
                    format!("decimal({digits},{fraction})")
                } else {
                    "decimal".to_string()
                }
            }
            "xs:boolean" => "boolean".to_string(),
            "xs:dateTime" => match form_child(type_node, "DateQualifiers")
                .and_then(|node| form_child_text(node, "DateFractions"))
                .as_deref()
            {
                Some("Date") => "date".to_string(),
                Some("Time") => "time".to_string(),
                _ => "dateTime".to_string(),
            },
            "xs:binary" => "binary".to_string(),
            "v8:ValueTable" => "ValueTable".to_string(),
            "v8:ValueTree" => "ValueTree".to_string(),
            "v8:ValueListType" => "ValueList".to_string(),
            "v8:TypeDescription" => "TypeDescription".to_string(),
            "v8:Universal" => "Universal".to_string(),
            "v8:FixedArray" => "FixedArray".to_string(),
            "v8:FixedStructure" => "FixedStructure".to_string(),
            "v8ui:FormattedString" => "FormattedString".to_string(),
            "v8ui:Picture" => "Picture".to_string(),
            "v8ui:Color" => "Color".to_string(),
            "v8ui:Font" => "Font".to_string(),
            other if other.starts_with("cfg:") => other[4..].to_string(),
            other if other.starts_with("dcsset:") => other.replacen("dcsset:", "DCS.", 1),
            other if other.starts_with("dcssch:") => other.replacen("dcssch:", "DCS.", 1),
            other if other.starts_with("dcscor:") => other.replacen("dcscor:", "DCS.", 1),
            other => {
                if let Some((prefix, suffix)) = other.split_once(':') {
                    if prefix.starts_with('d')
                        && prefix[1..]
                            .chars()
                            .all(|ch| ch.is_ascii_digit() || ch == 'p')
                    {
                        suffix.to_string()
                    } else {
                        other.to_string()
                    }
                } else {
                    other.to_string()
                }
            }
        };
        parts.push(part);
    }
    parts.join(" | ")
}

pub(crate) fn add_form(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    let result = (|| -> Result<(String, Vec<PathBuf>), String> {
        let object_path_raw = required_path(args, &["objectPath", "ObjectPath"], "ObjectPath")?;
        let form_name = required_string(args, &["formName", "FormName"], "FormName")?;
        let synonym = string_arg(args, &["synonym", "Synonym"]).unwrap_or(form_name);
        let purpose_raw = string_arg(args, &["purpose", "Purpose"]).unwrap_or("Object");
        let set_default = bool_arg(args, &["setDefault", "SetDefault"]);

        let object_xml_full =
            resolve_form_add_object_path(absolutize(object_path_raw, &context.cwd))?;
        let mut object_text = fs::read_to_string(&object_xml_full)
            .map_err(|err| format!("failed to read {}: {err}", object_xml_full.display()))?
            .trim_start_matches('\u{feff}')
            .to_string();
        let (object_type, object_name) = detect_form_add_object(&object_text)?;
        let format_version =
            detect_format_version(object_xml_full.parent().unwrap_or(context.cwd.as_path()));

        let purpose = normalize_form_purpose(purpose_raw);
        validate_form_purpose(&object_type, &purpose)?;

        let object_dir = object_xml_full.with_extension("");
        let forms_dir = object_dir.join("Forms");
        let form_meta_path = forms_dir.join(format!("{form_name}.xml"));
        if form_meta_path.exists() {
            return Err(format!(
                "Форма уже существует: {}",
                form_meta_path.display()
            ));
        }

        let form_dir = forms_dir.join(form_name);
        let form_ext_dir = form_dir.join("Ext");
        let form_module_dir = form_ext_dir.join("Form");
        fs::create_dir_all(&form_module_dir)
            .map_err(|err| format!("failed to create {}: {err}", form_module_dir.display()))?;

        write_utf8_bom(
            &form_meta_path,
            &form_add_metadata_xml(
                form_name,
                synonym,
                &object_type,
                &format_version,
                &fresh_uuid(),
            ),
        )?;

        let form_xml_path = form_ext_dir.join("Form.xml");
        let mut stdout = String::new();
        stdout.push('\n');
        stdout.push_str("=== form-add ===\n\n");
        stdout.push_str(&format!("Object: {object_type}.{object_name}\n"));
        if form_xml_path.exists() {
            stdout.push_str(&format!(
                "[SKIP] Form.xml already exists: {} — not overwriting\n",
                form_xml_path.display()
            ));
        } else {
            write_utf8_bom(
                &form_xml_path,
                &form_add_content_xml(&object_type, &object_name, &purpose, &format_version)?,
            )?;
        }

        let module_path = form_module_dir.join("Module.bsl");
        if module_path.exists() {
            stdout.push_str(&format!(
                "[SKIP] Module.bsl already exists: {} — not overwriting\n",
                module_path.display()
            ));
        } else {
            write_utf8_bom(&module_path, form_add_module_bsl())?;
        }

        object_text = register_form_in_object_text(&object_text, form_name);
        let default_prop_name = form_default_property(&object_type, &purpose);
        let default_value = format!("{object_type}.{object_name}.Form.{form_name}");
        let default_updated =
            if set_default || object_text.contains(&format!("<{default_prop_name}/>")) {
                let (updated_text, updated) =
                    replace_form_default_property(&object_text, default_prop_name, &default_value);
                object_text = updated_text;
                updated
            } else {
                false
            };
        write_utf8_bom(&object_xml_full, &lxml_tree_serialized_text(&object_text))?;

        let obj_dir_name = object_xml_full
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .display()
            .to_string();
        let obj_base_name = object_xml_full
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        stdout.push_str("Created:\n");
        stdout.push_str(&format!(
            "  Metadata: {obj_dir_name}\\{obj_base_name}\\Forms\\{form_name}.xml\n"
        ));
        stdout.push_str(&format!(
            "  Form:     {obj_dir_name}\\{obj_base_name}\\Forms\\{form_name}\\Ext\\Form.xml\n"
        ));
        stdout.push_str(&format!(
            "  Module:   {obj_dir_name}\\{obj_base_name}\\Forms\\{form_name}\\Ext\\Form\\Module.bsl\n"
        ));
        stdout.push('\n');
        stdout.push_str(&format!(
            "Registered: <Form>{form_name}</Form> in ChildObjects\n"
        ));
        if default_updated {
            stdout.push_str(&format!("{default_prop_name}: {default_value}\n"));
        }
        stdout.push('\n');

        Ok((
            stdout,
            vec![object_xml_full, form_meta_path, form_xml_path, module_path],
        ))
    })();

    match result {
        Ok((stdout, artifacts)) => AdapterOutcome {
            ok: true,
            summary: "unica.form.add completed with native form scaffold writer".to_string(),
            changes: artifacts
                .iter()
                .map(|path| format!("updated {}", path.display()))
                .collect(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: artifacts
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            stdout: Some(stdout),
            stderr: None,
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.form.add failed in native form scaffold writer".to_string(),
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

pub(crate) fn remove_form(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    let result = (|| -> Result<(String, Vec<String>), String> {
        let object_name = required_string(
            args,
            &["objectName", "ObjectName", "processorName", "ProcessorName"],
            "ObjectName",
        )?;
        let form_name = required_string(args, &["formName", "FormName"], "FormName")?;
        let src_dir_raw = string_arg(args, &["srcDir", "SrcDir"]).unwrap_or("src");
        let src_dir_display = PathBuf::from(src_dir_raw);
        let src_dir_abs = absolutize(src_dir_display.clone(), &context.cwd);

        let root_xml_display = src_dir_display.join(format!("{object_name}.xml"));
        let root_xml_path = src_dir_abs.join(format!("{object_name}.xml"));
        if !root_xml_path.exists() {
            return Err(format!(
                "Корневой файл обработки не найден: {}",
                root_xml_display.display()
            ));
        }

        let processor_dir_display = src_dir_display.join(object_name);
        let processor_dir_abs = src_dir_abs.join(object_name);
        let forms_dir_display = processor_dir_display.join("Forms");
        let forms_dir_abs = processor_dir_abs.join("Forms");
        let form_meta_display = forms_dir_display.join(format!("{form_name}.xml"));
        let form_meta_path = forms_dir_abs.join(format!("{form_name}.xml"));
        let form_dir_display = forms_dir_display.join(form_name);
        let form_dir_path = forms_dir_abs.join(form_name);

        if !form_meta_path.exists() {
            return Err(format!(
                "Метаданные формы не найдены: {}",
                form_meta_display.display()
            ));
        }

        let mut stdout = String::new();
        let mut changes = Vec::new();
        if form_dir_path.is_dir() {
            fs::remove_dir_all(&form_dir_path)
                .map_err(|err| format!("failed to remove {}: {err}", form_dir_path.display()))?;
            stdout.push_str(&format!(
                "[OK] Удалён каталог: {}\n",
                form_dir_display.display()
            ));
            changes.push(format!("removed directory {}", form_dir_path.display()));
        }

        fs::remove_file(&form_meta_path)
            .map_err(|err| format!("failed to remove {}: {err}", form_meta_path.display()))?;
        stdout.push_str(&format!(
            "[OK] Удалён файл: {}\n",
            form_meta_display.display()
        ));
        changes.push(format!("removed file {}", form_meta_path.display()));

        let xml_text = fs::read_to_string(&root_xml_path)
            .map_err(|err| format!("failed to read {}: {err}", root_xml_path.display()))?;
        let xml_text = remove_metadata_child_text_lxml(&xml_text, "Form", form_name);
        let (mut xml_text, default_form_cleared) =
            clear_metadata_reference_text(&xml_text, "DefaultForm", &format!("Form.{form_name}"));
        if !xml_text.ends_with('\n') {
            xml_text.push('\n');
        }
        if default_form_cleared {
            changes.push("cleared DefaultForm".to_string());
        }
        write_utf8_bom(&root_xml_path, &xml_text)?;
        changes.push(format!("updated {}", root_xml_path.display()));

        stdout.push_str(&format!(
            "[OK] Форма {form_name} удалена из {}\n",
            root_xml_display.display()
        ));
        Ok((stdout, changes))
    })();

    match result {
        Ok((stdout, changes)) => AdapterOutcome {
            ok: true,
            summary: "unica.form.remove completed with native form remover".to_string(),
            changes,
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: Vec::new(),
            stdout: Some(stdout),
            stderr: Some(String::new()),
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.form.remove failed in native form remover".to_string(),
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

pub(crate) fn form_add_supported_object_types() -> &'static [&'static str] {
    &[
        "Document",
        "Catalog",
        "DataProcessor",
        "Report",
        "ExternalDataProcessor",
        "ExternalReport",
        "InformationRegister",
        "AccumulationRegister",
        "ChartOfAccounts",
        "ChartOfCharacteristicTypes",
        "ExchangePlan",
        "BusinessProcess",
        "Task",
    ]
}

pub(crate) fn form_add_processor_like(object_type: &str) -> bool {
    matches!(
        object_type,
        "DataProcessor" | "Report" | "ExternalDataProcessor" | "ExternalReport"
    )
}

pub(crate) fn normalize_form_purpose(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    format!(
        "{}{}",
        first.to_uppercase().collect::<String>(),
        chars.as_str().to_lowercase()
    )
}

pub(crate) fn form_add_metadata_xml(
    form_name: &str,
    synonym: &str,
    object_type: &str,
    format_version: &str,
    form_uuid: &str,
) -> String {
    let extended_presentation = if form_add_processor_like(object_type) {
        "\t\t\t<ExtendedPresentation/>\n"
    } else {
        ""
    };
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\"",
            " xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\"",
            " xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\"",
            " xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\"",
            " xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\"",
            " xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\"",
            " xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\"",
            " xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\"",
            " xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"",
            " xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\"",
            " xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\"",
            " xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\"",
            " xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\"",
            " xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\"",
            " xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\"",
            " xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"",
            " xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"",
            " version=\"{format_version}\">\n",
            "\t<Form uuid=\"{form_uuid}\">\n",
            "\t\t<Properties>\n",
            "\t\t\t<Name>{form_name}</Name>\n",
            "\t\t\t<Synonym>\n",
            "\t\t\t\t<v8:item>\n",
            "\t\t\t\t\t<v8:lang>ru</v8:lang>\n",
            "\t\t\t\t\t<v8:content>{synonym}</v8:content>\n",
            "\t\t\t\t</v8:item>\n",
            "\t\t\t</Synonym>\n",
            "\t\t\t<Comment/>\n",
            "\t\t\t<FormType>Managed</FormType>\n",
            "\t\t\t<IncludeHelpInContents>false</IncludeHelpInContents>\n",
            "\t\t\t<UsePurposes>\n",
            "\t\t\t\t<v8:Value xsi:type=\"app:ApplicationUsePurpose\">PlatformApplication</v8:Value>\n",
            "\t\t\t\t<v8:Value xsi:type=\"app:ApplicationUsePurpose\">MobilePlatformApplication</v8:Value>\n",
            "\t\t\t</UsePurposes>\n",
            "{extended_presentation}",
            "\t\t</Properties>\n",
            "\t</Form>\n",
            "</MetaDataObject>"
        ),
        format_version = escape_xml(format_version),
        form_uuid = escape_xml(form_uuid),
        form_name = escape_xml(form_name),
        synonym = escape_xml(synonym),
        extended_presentation = extended_presentation,
    )
}

pub(crate) fn form_add_content_xml(
    object_type: &str,
    object_name: &str,
    purpose: &str,
    format_version: &str,
) -> Result<String, String> {
    let ns = concat!(
        "xmlns=\"http://v8.1c.ru/8.3/xcf/logform\"",
        " xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\"",
        " xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\"",
        " xmlns:dcscor=\"http://v8.1c.ru/8.1/data-composition-system/core\"",
        " xmlns:dcsset=\"http://v8.1c.ru/8.1/data-composition-system/settings\"",
        " xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\"",
        " xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\"",
        " xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\"",
        " xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\"",
        " xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"",
        " xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\"",
        " xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\"",
        " xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\"",
        " xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\"",
        " xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"",
        " xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\""
    );
    if matches!(purpose, "List" | "Choice") {
        let main_table = format!("{object_type}.{object_name}");
        return Ok(format!(
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
                "<Form {ns} version=\"{format_version}\">\n",
                "\t<AutoCommandBar name=\"ФормаКоманднаяПанель\" id=\"-1\">\n",
                "\t\t<Autofill>true</Autofill>\n",
                "\t</AutoCommandBar>\n",
                "\t<ChildItems/>\n",
                "\t<Attributes>\n",
                "\t\t<Attribute name=\"Список\" id=\"1\">\n",
                "\t\t\t<Type>\n",
                "\t\t\t\t<v8:Type>cfg:DynamicList</v8:Type>\n",
                "\t\t\t</Type>\n",
                "\t\t\t<MainAttribute>true</MainAttribute>\n",
                "\t\t\t<Settings xsi:type=\"DynamicList\">\n",
                "\t\t\t\t<MainTable>{main_table}</MainTable>\n",
                "\t\t\t</Settings>\n",
                "\t\t</Attribute>\n",
                "\t</Attributes>\n",
                "</Form>"
            ),
            ns = ns,
            format_version = escape_xml(format_version),
            main_table = escape_xml(&main_table),
        ));
    }
    if purpose == "Record" {
        let main_attr_type = format!("InformationRegisterRecordManager.{object_name}");
        return Ok(format!(
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
                "<Form {ns} version=\"{format_version}\">\n",
                "\t<AutoCommandBar name=\"ФормаКоманднаяПанель\" id=\"-1\">\n",
                "\t\t<Autofill>true</Autofill>\n",
                "\t</AutoCommandBar>\n",
                "\t<ChildItems/>\n",
                "\t<Attributes>\n",
                "\t\t<Attribute name=\"Запись\" id=\"1\">\n",
                "\t\t\t<Type>\n",
                "\t\t\t\t<v8:Type>cfg:{main_attr_type}</v8:Type>\n",
                "\t\t\t</Type>\n",
                "\t\t\t<MainAttribute>true</MainAttribute>\n",
                "\t\t\t<SavedData>true</SavedData>\n",
                "\t\t</Attribute>\n",
                "\t</Attributes>\n",
                "</Form>"
            ),
            ns = ns,
            format_version = escape_xml(format_version),
            main_attr_type = escape_xml(&main_attr_type),
        ));
    }

    let attr_prefix = match object_type {
        "Document" => "DocumentObject",
        "Catalog" => "CatalogObject",
        "DataProcessor" => "DataProcessorObject",
        "Report" => "ReportObject",
        "ExternalDataProcessor" => "ExternalDataProcessorObject",
        "ExternalReport" => "ExternalReportObject",
        "ChartOfAccounts" => "ChartOfAccountsObject",
        "ChartOfCharacteristicTypes" => "ChartOfCharacteristicTypesObject",
        "ExchangePlan" => "ExchangePlanObject",
        "BusinessProcess" => "BusinessProcessObject",
        "Task" => "TaskObject",
        "InformationRegister" => "InformationRegisterRecordManager",
        "AccumulationRegister" => "AccumulationRegisterRecordSet",
        other => return Err(format!("unsupported form object type: {other}")),
    };
    let main_attr_type = format!("{attr_prefix}.{object_name}");
    let saved_data_line = if form_add_processor_like(object_type) {
        ""
    } else {
        "\t\t\t<SavedData>true</SavedData>\n"
    };
    Ok(format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<Form {ns} version=\"{format_version}\">\n",
            "\t<AutoCommandBar name=\"ФормаКоманднаяПанель\" id=\"-1\">\n",
            "\t\t<Autofill>true</Autofill>\n",
            "\t</AutoCommandBar>\n",
            "\t<ChildItems/>\n",
            "\t<Attributes>\n",
            "\t\t<Attribute name=\"Объект\" id=\"1\">\n",
            "\t\t\t<Type>\n",
            "\t\t\t\t<v8:Type>cfg:{main_attr_type}</v8:Type>\n",
            "\t\t\t</Type>\n",
            "\t\t\t<MainAttribute>true</MainAttribute>\n",
            "{saved_data_line}",
            "\t\t</Attribute>\n",
            "\t</Attributes>\n",
            "</Form>"
        ),
        ns = ns,
        format_version = escape_xml(format_version),
        main_attr_type = escape_xml(&main_attr_type),
        saved_data_line = saved_data_line,
    ))
}

pub(crate) fn form_add_module_bsl() -> &'static str {
    concat!(
        "#Область ОбработчикиСобытийФормы\n\n",
        "#КонецОбласти\n\n",
        "#Область ОбработчикиСобытийЭлементовФормы\n\n",
        "#КонецОбласти\n\n",
        "#Область ОбработчикиКомандФормы\n\n",
        "#КонецОбласти\n\n",
        "#Область ОбработчикиОповещений\n\n",
        "#КонецОбласти\n\n",
        "#Область СлужебныеПроцедурыИФункции\n\n",
        "#КонецОбласти"
    )
}

pub(crate) fn form_default_property<'a>(object_type: &str, purpose: &'a str) -> &'a str {
    match purpose {
        "Object" => {
            if form_add_processor_like(object_type) {
                "DefaultForm"
            } else {
                "DefaultObjectForm"
            }
        }
        "List" => "DefaultListForm",
        "Choice" => "DefaultChoiceForm",
        "Record" => "DefaultRecordForm",
        _ => "DefaultForm",
    }
}

pub(crate) fn replace_form_default_property(
    text: &str,
    prop_name: &str,
    default_value: &str,
) -> (String, bool) {
    let empty = format!("<{prop_name}/>");
    if text.contains(&empty) {
        return (
            text.replacen(
                &empty,
                &format!("<{prop_name}>{default_value}</{prop_name}>"),
                1,
            ),
            true,
        );
    }
    let start_tag = format!("<{prop_name}>");
    let end_tag = format!("</{prop_name}>");
    let Some(start) = text.find(&start_tag) else {
        return (text.to_string(), false);
    };
    let value_start = start + start_tag.len();
    let Some(relative_end) = text[value_start..].find(&end_tag) else {
        return (text.to_string(), false);
    };
    let value_end = value_start + relative_end;
    if !text[value_start..value_end].trim().is_empty() {
        return (text.to_string(), false);
    }
    (
        format!(
            "{}{}{}",
            &text[..value_start],
            default_value,
            &text[value_end..]
        ),
        true,
    )
}

pub(crate) fn compile_form(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let write_result = (|| -> Result<(String, PathBuf), String> {
        let json_path_raw = path_arg(args, &["jsonPath", "JsonPath"]);
        let from_object = bool_arg(args, &["fromObject", "FromObject"]);
        if from_object && json_path_raw.is_some() {
            return Err("Cannot use both -JsonPath and -FromObject. Choose one mode.".to_string());
        }
        if !from_object && json_path_raw.is_none() {
            return Err("Either -JsonPath or -FromObject is required.".to_string());
        }
        if from_object {
            return Err("native form compiler currently supports -JsonPath mode only".to_string());
        }

        let output_label = string_arg(args, &["outputPath", "OutputPath"])
            .ok_or_else(|| "missing required OutputPath argument".to_string())?
            .to_string();
        let output_path = absolutize(PathBuf::from(&output_label), &context.cwd);
        let json_path_raw = json_path_raw.expect("checked above");
        let json_path = absolutize(json_path_raw.clone(), &context.cwd);
        if !json_path.exists() {
            return Err(format!("File not found: {}", json_path_raw.display()));
        }
        let json_text = fs::read_to_string(&json_path)
            .map_err(|err| format!("failed to read {}: {err}", json_path.display()))?;
        let defn: Value = serde_json::from_str(json_text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("failed to parse Form JSON: {err}"))?;

        let format_version = detect_format_version(output_path.parent().unwrap_or(&context.cwd));
        let (xml, stats) = form_compile_xml(&defn, &format_version)?;

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        write_utf8_bom(&output_path, &xml)?;

        let mut stdout = String::new();
        if let Some(registered) = register_form_in_parent_object(&output_path)? {
            stdout.push_str(&registered);
        }
        stdout.push_str(&format!("[OK] Compiled: {output_label}\n"));
        stdout.push_str(&format!("     Elements+IDs: {}\n", stats.element_ids));
        if stats.attributes > 0 {
            stdout.push_str(&format!("     Attributes: {}\n", stats.attributes));
        }
        if stats.commands > 0 {
            stdout.push_str(&format!("     Commands: {}\n", stats.commands));
        }
        if stats.parameters > 0 {
            stdout.push_str(&format!("     Parameters: {}\n", stats.parameters));
        }

        Ok((stdout, output_path))
    })();

    match write_result {
        Ok((stdout, output_path)) => AdapterOutcome {
            ok: true,
            summary: "unica.form.compile completed with native managed form compiler".to_string(),
            changes: vec![format!("updated {}", output_path.display())],
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![output_path.display().to_string()],
            stdout: Some(stdout),
            stderr: None,
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.form.compile failed in native managed form compiler".to_string(),
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

pub(crate) fn edit_form(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    let edit_result = (|| -> Result<(String, PathBuf), String> {
        let form_path_raw =
            required_path(args, &["formPath", "FormPath", "path", "Path"], "FormPath")?;
        let json_path_raw = required_path(args, &["jsonPath", "JsonPath"], "JsonPath")?;
        let form_path = absolutize(form_path_raw.clone(), &context.cwd);
        if !form_path.exists() {
            return Err(format!("File not found: {}", form_path_raw.display()));
        }
        let json_path = absolutize(json_path_raw.clone(), &context.cwd);
        if !json_path.exists() {
            return Err(format!("File not found: {}", json_path_raw.display()));
        }

        let json_text = fs::read_to_string(&json_path)
            .map_err(|err| format!("failed to read {}: {err}", json_path.display()))?;
        let defn: Value = serde_json::from_str(json_text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("failed to parse form edit JSON: {err}"))?;
        let mut xml_text = fs::read_to_string(&form_path)
            .map_err(|err| format!("failed to read {}: {err}", form_path.display()))?;
        if xml_text.starts_with('\u{feff}') {
            xml_text = xml_text.trim_start_matches('\u{feff}').to_string();
        }
        Document::parse(&xml_text).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;

        let form_name = form_edit_form_name(&form_path);
        let mut elem_ids = FormIdAllocator {
            next: form_edit_next_id(
                &xml_text,
                &[
                    "InputField",
                    "ContextMenu",
                    "ExtendedTooltip",
                    "UsualGroup",
                    "Table",
                    "Button",
                ],
            ),
        };
        let mut attr_ids = FormIdAllocator {
            next: form_edit_next_id(&xml_text, &["Attribute", "Column"]),
        };
        let mut cmd_ids = FormIdAllocator {
            next: form_edit_next_id(&xml_text, &["Command"]),
        };

        let mut added_elements = Vec::<String>::new();
        if let Some(elements) = defn.get("elements").and_then(Value::as_array) {
            if !elements.is_empty() {
                let start = elem_ids.next;
                let mut lines = Vec::<String>::new();
                for element in elements {
                    let name = form_edit_element_display_name(element);
                    emit_form_element(&mut lines, element, "\t\t", &mut elem_ids)?;
                    if let Some(name) = name {
                        let path = element
                            .get("path")
                            .and_then(Value::as_str)
                            .map(|value| format!(" -> {value}"))
                            .unwrap_or_default();
                        added_elements.push(format!("  + [Input] {name}{path}"));
                    }
                }
                form_edit_insert_section_items(&mut xml_text, "ChildItems", &lines)?;
                let _companion_count = elem_ids.next.saturating_sub(start + added_elements.len());
            }
        }

        let mut added_attrs = Vec::<String>::new();
        if let Some(attrs) = defn.get("attributes").and_then(Value::as_array) {
            if !attrs.is_empty() {
                let mut lines = Vec::<String>::new();
                for attr in attrs {
                    let Some(object) = attr.as_object() else {
                        continue;
                    };
                    let Some(name) = object.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    let id = attr_ids.next();
                    emit_form_edit_attribute_item(&mut lines, object, name, id, "\t\t");
                    let type_name = object
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("(no type)");
                    added_attrs.push(format!("  + {name}: {type_name} (id={id})"));
                }
                form_edit_insert_section_items(&mut xml_text, "Attributes", &lines)?;
            }
        }

        let mut added_cmds = Vec::<String>::new();
        if let Some(commands) = defn.get("commands").and_then(Value::as_array) {
            if !commands.is_empty() {
                let mut lines = Vec::<String>::new();
                for cmd in commands {
                    let Some(object) = cmd.as_object() else {
                        continue;
                    };
                    let Some(name) = object.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    let id = cmd_ids.next();
                    emit_form_edit_command_item(&mut lines, object, name, id, "\t\t");
                    let action = object
                        .get("action")
                        .and_then(Value::as_str)
                        .map(|value| format!(" -> {value}"))
                        .unwrap_or_default();
                    added_cmds.push(format!("  + {name}{action} (id={id})"));
                }
                form_edit_insert_section_items(&mut xml_text, "Commands", &lines)?;
            }
        }

        write_utf8_bom(&form_path, &xml_text)?;

        let mut stdout = format!("=== form-edit: {form_name} ===\n\n");
        if !added_elements.is_empty() {
            stdout.push_str("Added elements:\n");
            stdout.push_str(&added_elements.join("\n"));
            stdout.push_str("\n\n");
        }
        if !added_attrs.is_empty() {
            stdout.push_str("Added attributes:\n");
            stdout.push_str(&added_attrs.join("\n"));
            stdout.push_str("\n\n");
        }
        if !added_cmds.is_empty() {
            stdout.push_str("Added commands:\n");
            stdout.push_str(&added_cmds.join("\n"));
            stdout.push_str("\n\n");
        }
        let mut total_parts = Vec::new();
        if !added_elements.is_empty() {
            total_parts.push(format!("{} element(s)", added_elements.len()));
        }
        if !added_attrs.is_empty() {
            total_parts.push(format!("{} attribute(s)", added_attrs.len()));
        }
        if !added_cmds.is_empty() {
            total_parts.push(format!("{} command(s)", added_cmds.len()));
        }
        stdout.push_str("---\n");
        stdout.push_str(&format!("Total: {}\n", total_parts.join(", ")));
        stdout.push_str("Run /form-validate to verify.\n");

        Ok((stdout, form_path))
    })();

    match edit_result {
        Ok((stdout, form_path)) => AdapterOutcome {
            ok: true,
            summary: "unica.form.edit completed with native managed form editor".to_string(),
            changes: vec![format!("updated {}", form_path.display())],
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![form_path.display().to_string()],
            stdout: Some(stdout),
            stderr: None,
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.form.edit failed in native managed form editor".to_string(),
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

pub(crate) fn form_edit_form_name(path: &Path) -> String {
    if path.file_name().and_then(|value| value.to_str()) == Some("Form.xml")
        && path
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            == Some("Ext")
    {
        if let Some(name) = path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
        {
            return name.to_string();
        }
    }
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Form")
        .to_string()
}

pub(crate) fn form_edit_next_id(xml_text: &str, tags: &[&str]) -> usize {
    let Ok(doc) = Document::parse(xml_text) else {
        return 0;
    };
    doc.descendants()
        .filter(|node| node.is_element() && tags.contains(&node.tag_name().name()))
        .filter_map(|node| node.attribute("id"))
        .filter(|value| *value != "-1")
        .filter_map(|value| value.parse::<usize>().ok())
        .max()
        .unwrap_or(0)
}

pub(crate) fn form_edit_element_display_name(element: &Value) -> Option<String> {
    let object = element.as_object()?;
    object
        .get("input")
        .and_then(Value::as_str)
        .or_else(|| object.get("name").and_then(Value::as_str))
        .map(ToOwned::to_owned)
}

pub(crate) fn form_edit_insert_section_items(
    xml_text: &mut String,
    section: &str,
    lines: &[String],
) -> Result<(), String> {
    if lines.is_empty() {
        return Ok(());
    }
    let content = lines.join("\n");
    let empty = format!("<{section}/>");
    if xml_text.contains(&empty) {
        *xml_text = xml_text.replacen(
            &empty,
            &format!("<{section}>\n{content}\n\t</{section}>"),
            1,
        );
        return Ok(());
    }
    let close = format!("</{section}>");
    let Some(pos) = xml_text.find(&close) else {
        return Err(format!("No <{section}> section found in form"));
    };
    xml_text.insert_str(pos, &format!("{content}\n\t"));
    Ok(())
}

pub(crate) fn emit_form_edit_attribute_item(
    lines: &mut Vec<String>,
    attr: &Map<String, Value>,
    name: &str,
    id: usize,
    indent: &str,
) {
    lines.push(format!(
        "{indent}<Attribute name=\"{}\" id=\"{id}\">",
        escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if let Some(title) = attr.get("title").and_then(Value::as_str) {
        emit_form_mltext(lines, &inner, "Title", title);
    }
    if let Some(type_name) = attr.get("type").and_then(Value::as_str) {
        emit_form_type(lines, type_name, &inner);
    } else {
        lines.push(format!("{inner}<Type/>"));
    }
    if attr.get("main").and_then(Value::as_bool) == Some(true) {
        lines.push(format!("{inner}<MainAttribute>true</MainAttribute>"));
    }
    if attr.get("savedData").and_then(Value::as_bool) == Some(true) {
        lines.push(format!("{inner}<SavedData>true</SavedData>"));
    }
    if let Some(fill_checking) = attr.get("fillChecking").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<FillChecking>{}</FillChecking>",
            escape_xml(fill_checking)
        ));
    }
    lines.push(format!("{indent}</Attribute>"));
}

pub(crate) fn emit_form_edit_command_item(
    lines: &mut Vec<String>,
    cmd: &Map<String, Value>,
    name: &str,
    id: usize,
    indent: &str,
) {
    lines.push(format!(
        "{indent}<Command name=\"{}\" id=\"{id}\">",
        escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if let Some(title) = cmd.get("title").and_then(Value::as_str) {
        emit_form_mltext(lines, &inner, "Title", title);
    }
    if let Some(action) = cmd.get("action").and_then(Value::as_str) {
        lines.push(format!("{inner}<Action>{}</Action>", escape_xml(action)));
    }
    lines.push(format!("{indent}</Command>"));
}

pub(crate) struct FormCompileStats {
    pub(crate) element_ids: usize,
    pub(crate) attributes: usize,
    pub(crate) commands: usize,
    pub(crate) parameters: usize,
}

pub(crate) struct FormIdAllocator {
    pub(crate) next: usize,
}

impl FormIdAllocator {
    pub(crate) fn new() -> Self {
        Self { next: 0 }
    }

    pub(crate) fn next(&mut self) -> usize {
        self.next += 1;
        self.next
    }
}

pub(crate) fn form_compile_xml(
    defn: &Value,
    format_version: &str,
) -> Result<(String, FormCompileStats), String> {
    let mut ids = FormIdAllocator::new();
    let mut lines = Vec::<String>::new();
    lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
    lines.push(format!(
        "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:dcscor=\"http://v8.1c.ru/8.1/data-composition-system/core\" xmlns:dcssch=\"http://v8.1c.ru/8.1/data-composition-system/schema\" xmlns:dcsset=\"http://v8.1c.ru/8.1/data-composition-system/settings\" xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\" xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"{format_version}\">"
    ));

    if let Some(title) = json_string_field(defn, "title").or_else(|| {
        defn.get("properties")
            .and_then(|props| json_string_field(props, "title"))
    }) {
        emit_form_mltext(&mut lines, "\t", "Title", &title);
    }

    if let Some(props) = defn.get("properties").and_then(Value::as_object) {
        emit_form_properties(&mut lines, props, "\t");
    }

    emit_form_auto_command_bar(&mut lines, defn, "\t");

    if let Some(elements) = defn.get("elements").and_then(Value::as_array) {
        if !elements.is_empty() {
            lines.push("\t<ChildItems>".to_string());
            for element in elements {
                emit_form_element(&mut lines, element, "\t\t", &mut ids)?;
            }
            lines.push("\t</ChildItems>".to_string());
        }
    }

    let attributes = defn
        .get("attributes")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    emit_form_attributes(&mut lines, defn.get("attributes"), "\t", &mut ids)?;

    let parameters = defn
        .get("parameters")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    emit_form_parameters(&mut lines, defn.get("parameters"), "\t")?;

    let commands = defn
        .get("commands")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    emit_form_commands(&mut lines, defn.get("commands"), "\t", &mut ids)?;

    lines.push("</Form>".to_string());
    Ok((
        format!("{}\n", lines.join("\n")),
        FormCompileStats {
            element_ids: ids.next,
            attributes,
            commands,
            parameters,
        },
    ))
}

pub(crate) fn emit_form_auto_command_bar(lines: &mut Vec<String>, defn: &Value, indent: &str) {
    let mut explicit_bar = None::<&Value>;
    if let Some(elements) = defn.get("elements").and_then(Value::as_array) {
        explicit_bar = elements.iter().find(|element| {
            element.as_object().is_some_and(|object| {
                object.contains_key("autoCmdBar") || object.contains_key("autoCommandBar")
            })
        });
    }

    let mut name = "ФормаКоманднаяПанель".to_string();
    let mut halign = None::<String>;
    let mut autofill = true;
    let mut has_children = false;
    if let Some(bar) = explicit_bar.and_then(Value::as_object) {
        if let Some(value) = bar
            .get("autoCmdBar")
            .or_else(|| bar.get("autoCommandBar"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            name = value.to_string();
        }
        if let Some(value) = bar.get("name").and_then(Value::as_str) {
            name = value.to_string();
        }
        halign = bar
            .get("horizontalAlign")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        autofill = bar.get("autofill").and_then(Value::as_bool).unwrap_or(true);
        has_children = bar
            .get("children")
            .and_then(Value::as_array)
            .is_some_and(|children| !children.is_empty());
    } else if let Some(elements) = defn.get("elements").and_then(Value::as_array) {
        if elements.iter().any(form_element_has_command_bar) {
            autofill = false;
        }
    }

    if halign.is_some() || !autofill || has_children {
        lines.push(format!(
            "{indent}<AutoCommandBar name=\"{}\" id=\"-1\">",
            escape_xml(&name)
        ));
        if let Some(halign) = halign {
            lines.push(format!(
                "{indent}\t<HorizontalAlign>{halign}</HorizontalAlign>"
            ));
        }
        if !autofill {
            lines.push(format!("{indent}\t<Autofill>false</Autofill>"));
        }
        lines.push(format!("{indent}</AutoCommandBar>"));
    } else {
        lines.push(format!(
            "{indent}<AutoCommandBar name=\"{}\" id=\"-1\"/>",
            escape_xml(&name)
        ));
    }
}

pub(crate) fn form_element_has_command_bar(element: &Value) -> bool {
    let Some(object) = element.as_object() else {
        return false;
    };
    if object.contains_key("cmdBar") || object.contains_key("commandBar") {
        return true;
    }
    for key in ["children", "columns"] {
        if let Some(children) = object.get(key).and_then(Value::as_array) {
            if children.iter().any(form_element_has_command_bar) {
                return true;
            }
        }
    }
    false
}

pub(crate) fn emit_form_mltext(lines: &mut Vec<String>, indent: &str, tag: &str, text: &str) {
    if text.is_empty() {
        lines.push(format!("{indent}<{tag}/>"));
        return;
    }
    lines.push(format!("{indent}<{tag}>"));
    lines.push(format!("{indent}\t<v8:item>"));
    lines.push(format!("{indent}\t\t<v8:lang>ru</v8:lang>"));
    lines.push(format!(
        "{indent}\t\t<v8:content>{}</v8:content>",
        escape_xml(text)
    ));
    lines.push(format!("{indent}\t</v8:item>"));
    lines.push(format!("{indent}</{tag}>"));
}

pub(crate) fn emit_form_properties(
    lines: &mut Vec<String>,
    props: &Map<String, Value>,
    indent: &str,
) {
    for (name, value) in props {
        if name == "title" {
            continue;
        }
        let xml_name = match name.as_str() {
            "autoTitle" => "AutoTitle".to_string(),
            "windowOpeningMode" => "WindowOpeningMode".to_string(),
            "commandBarLocation" => "CommandBarLocation".to_string(),
            "saveDataInSettings" => "SaveDataInSettings".to_string(),
            "autoSaveDataInSettings" => "AutoSaveDataInSettings".to_string(),
            "autoTime" => "AutoTime".to_string(),
            "usePostingMode" => "UsePostingMode".to_string(),
            "repostOnWrite" => "RepostOnWrite".to_string(),
            "autoURL" => "AutoURL".to_string(),
            "autoFillCheck" => "AutoFillCheck".to_string(),
            "customizable" => "Customizable".to_string(),
            "enterKeyBehavior" => "EnterKeyBehavior".to_string(),
            "verticalScroll" => "VerticalScroll".to_string(),
            "scalingMode" => "ScalingMode".to_string(),
            "useForFoldersAndItems" => "UseForFoldersAndItems".to_string(),
            "reportResult" => "ReportResult".to_string(),
            "detailsData" => "DetailsData".to_string(),
            "reportFormType" => "ReportFormType".to_string(),
            "autoShowState" => "AutoShowState".to_string(),
            "width" => "Width".to_string(),
            "height" => "Height".to_string(),
            "group" => "Group".to_string(),
            other => {
                let mut chars = other.chars();
                match chars.next() {
                    Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                    None => continue,
                }
            }
        };
        let text = if let Some(flag) = value.as_bool() {
            if flag { "true" } else { "false" }.to_string()
        } else {
            json_value_to_python_string(value)
        };
        lines.push(format!(
            "{indent}<{xml_name}>{}</{xml_name}>",
            escape_xml(&text)
        ));
    }
}

pub(crate) fn emit_form_element(
    lines: &mut Vec<String>,
    element: &Value,
    indent: &str,
    ids: &mut FormIdAllocator,
) -> Result<(), String> {
    let Some(object) = element.as_object() else {
        return Ok(());
    };
    if let Some(name) = object
        .get("input")
        .and_then(Value::as_str)
        .or_else(|| object.get("name").and_then(Value::as_str))
    {
        emit_form_input(lines, object, name, indent, ids);
        return Ok(());
    }
    if object.contains_key("autoCmdBar") || object.contains_key("autoCommandBar") {
        return Ok(());
    }
    Err(format!(
        "Unsupported form element in native compiler: {}",
        serde_json::to_string(element).unwrap_or_else(|_| "<invalid>".to_string())
    ))
}

pub(crate) fn emit_form_input(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
) {
    let id = ids.next();
    lines.push(format!(
        "{indent}<InputField name=\"{}\" id=\"{id}\">",
        escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if let Some(path) = element.get("path").and_then(Value::as_str) {
        lines.push(format!("{inner}<DataPath>{}</DataPath>", escape_xml(path)));
    }
    if let Some(title) = element.get("title").and_then(Value::as_str) {
        emit_form_mltext(lines, &inner, "Title", title);
    }
    emit_form_common_flags(lines, element, &inner);
    if let Some(value) = element.get("titleLocation").and_then(Value::as_str) {
        let location = match value {
            "none" => "None",
            "left" => "Left",
            "right" => "Right",
            "top" => "Top",
            "bottom" => "Bottom",
            other => other,
        };
        lines.push(format!("{inner}<TitleLocation>{location}</TitleLocation>"));
    }
    for (key, tag) in [
        ("multiLine", "MultiLine"),
        ("passwordMode", "PasswordMode"),
        ("clearButton", "ClearButton"),
        ("spinButton", "SpinButton"),
        ("dropListButton", "DropListButton"),
        ("markIncomplete", "AutoMarkIncomplete"),
        ("skipOnInput", "SkipOnInput"),
        ("horizontalStretch", "HorizontalStretch"),
        ("verticalStretch", "VerticalStretch"),
    ] {
        if element.get(key).and_then(Value::as_bool) == Some(true) {
            lines.push(format!("{inner}<{tag}>true</{tag}>"));
        }
    }
    for (key, tag) in [
        ("choiceButton", "ChoiceButton"),
        ("autoMaxWidth", "AutoMaxWidth"),
        ("autoMaxHeight", "AutoMaxHeight"),
    ] {
        if element.get(key).and_then(Value::as_bool) == Some(false) {
            lines.push(format!("{inner}<{tag}>false</{tag}>"));
        }
    }
    if let Some(width) = element.get("width").and_then(json_i64_value) {
        lines.push(format!("{inner}<Width>{width}</Width>"));
    }
    if let Some(height) = element.get("height").and_then(json_i64_value) {
        lines.push(format!("{inner}<Height>{height}</Height>"));
    }
    if let Some(hint) = element.get("inputHint").and_then(Value::as_str) {
        emit_form_mltext(lines, &inner, "InputHint", hint);
    }
    emit_form_companion(
        lines,
        "ContextMenu",
        &format!("{name}КонтекстноеМеню"),
        &inner,
        ids,
    );
    emit_form_companion(
        lines,
        "ExtendedTooltip",
        &format!("{name}РасширеннаяПодсказка"),
        &inner,
        ids,
    );
    lines.push(format!("{indent}</InputField>"));
}

pub(crate) fn emit_form_common_flags(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    indent: &str,
) {
    if element.get("visible").and_then(Value::as_bool) == Some(false)
        || element.get("hidden").and_then(Value::as_bool) == Some(true)
    {
        lines.push(format!("{indent}<Visible>false</Visible>"));
    }
    if element.get("userVisible").and_then(Value::as_bool) == Some(false) {
        lines.push(format!("{indent}<UserVisible>"));
        lines.push(format!("{indent}\t<xr:Common>false</xr:Common>"));
        lines.push(format!("{indent}</UserVisible>"));
    }
    if element.get("enabled").and_then(Value::as_bool) == Some(false)
        || element.get("disabled").and_then(Value::as_bool) == Some(true)
    {
        lines.push(format!("{indent}<Enabled>false</Enabled>"));
    }
    if element.get("readOnly").and_then(Value::as_bool) == Some(true) {
        lines.push(format!("{indent}<ReadOnly>true</ReadOnly>"));
    }
}

pub(crate) fn emit_form_companion(
    lines: &mut Vec<String>,
    tag: &str,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
) {
    let id = ids.next();
    lines.push(format!(
        "{indent}<{tag} name=\"{}\" id=\"{id}\"/>",
        escape_xml(name)
    ));
}

pub(crate) fn emit_form_attributes(
    lines: &mut Vec<String>,
    attrs: Option<&Value>,
    indent: &str,
    ids: &mut FormIdAllocator,
) -> Result<(), String> {
    let Some(attrs) = attrs.and_then(Value::as_array) else {
        return Ok(());
    };
    if attrs.is_empty() {
        return Ok(());
    }
    lines.push(format!("{indent}<Attributes>"));
    for attr in attrs {
        let Some(object) = attr.as_object() else {
            continue;
        };
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "Form attribute is missing name".to_string())?;
        let attr_id = ids.next();
        lines.push(format!(
            "{indent}\t<Attribute name=\"{}\" id=\"{attr_id}\">",
            escape_xml(name)
        ));
        let inner = format!("{indent}\t\t");
        if let Some(title) = object.get("title").and_then(Value::as_str) {
            emit_form_mltext(lines, &inner, "Title", title);
        }
        if let Some(type_name) = object.get("type").and_then(Value::as_str) {
            emit_form_type(lines, type_name, &inner);
        } else {
            lines.push(format!("{inner}<Type/>"));
        }
        if object.get("main").and_then(Value::as_bool) == Some(true) {
            lines.push(format!("{inner}<MainAttribute>true</MainAttribute>"));
        }
        if object.get("savedData").and_then(Value::as_bool) == Some(true) {
            lines.push(format!("{inner}<SavedData>true</SavedData>"));
        }
        if let Some(fill_checking) = object.get("fillChecking").and_then(Value::as_str) {
            lines.push(format!(
                "{inner}<FillChecking>{}</FillChecking>",
                escape_xml(fill_checking)
            ));
        }
        lines.push(format!("{indent}\t</Attribute>"));
    }
    lines.push(format!("{indent}</Attributes>"));
    Ok(())
}

pub(crate) fn emit_form_parameters(
    lines: &mut Vec<String>,
    params: Option<&Value>,
    indent: &str,
) -> Result<(), String> {
    let Some(params) = params.and_then(Value::as_array) else {
        return Ok(());
    };
    if params.is_empty() {
        return Ok(());
    }
    lines.push(format!("{indent}<Parameters>"));
    for param in params {
        let Some(object) = param.as_object() else {
            continue;
        };
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "Form parameter is missing name".to_string())?;
        lines.push(format!(
            "{indent}\t<Parameter name=\"{}\">",
            escape_xml(name)
        ));
        let inner = format!("{indent}\t\t");
        emit_form_type(
            lines,
            object
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            &inner,
        );
        if object.get("key").and_then(Value::as_bool) == Some(true) {
            lines.push(format!("{inner}<KeyParameter>true</KeyParameter>"));
        }
        lines.push(format!("{indent}\t</Parameter>"));
    }
    lines.push(format!("{indent}</Parameters>"));
    Ok(())
}

pub(crate) fn emit_form_commands(
    lines: &mut Vec<String>,
    cmds: Option<&Value>,
    indent: &str,
    ids: &mut FormIdAllocator,
) -> Result<(), String> {
    let Some(cmds) = cmds.and_then(Value::as_array) else {
        return Ok(());
    };
    if cmds.is_empty() {
        return Ok(());
    }
    lines.push(format!("{indent}<Commands>"));
    for cmd in cmds {
        let Some(object) = cmd.as_object() else {
            continue;
        };
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "Form command is missing name".to_string())?;
        let cmd_id = ids.next();
        lines.push(format!(
            "{indent}\t<Command name=\"{}\" id=\"{cmd_id}\">",
            escape_xml(name)
        ));
        let inner = format!("{indent}\t\t");
        if let Some(title) = object.get("title").and_then(Value::as_str) {
            emit_form_mltext(lines, &inner, "Title", title);
        }
        for (key, tag) in [
            ("action", "Action"),
            ("shortcut", "Shortcut"),
            ("representation", "Representation"),
        ] {
            if let Some(value) = object.get(key).and_then(Value::as_str) {
                lines.push(format!("{inner}<{tag}>{}</{tag}>", escape_xml(value)));
            }
        }
        if let Some(picture) = object.get("picture").and_then(Value::as_str) {
            lines.push(format!("{inner}<Picture>"));
            lines.push(format!("{inner}\t<xr:Ref>{}</xr:Ref>", escape_xml(picture)));
            lines.push(format!(
                "{inner}\t<xr:LoadTransparent>true</xr:LoadTransparent>"
            ));
            lines.push(format!("{inner}</Picture>"));
        }
        lines.push(format!("{indent}\t</Command>"));
    }
    lines.push(format!("{indent}</Commands>"));
    Ok(())
}

pub(crate) fn emit_form_type(lines: &mut Vec<String>, type_name: &str, indent: &str) {
    if type_name.is_empty() {
        lines.push(format!("{indent}<Type/>"));
        return;
    }
    lines.push(format!("{indent}<Type>"));
    for part in type_name
        .split(['|', '+'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        emit_form_single_type(lines, part, &format!("{indent}\t"));
    }
    lines.push(format!("{indent}</Type>"));
}

pub(crate) fn emit_form_single_type(lines: &mut Vec<String>, type_name: &str, indent: &str) {
    let normalized = normalize_form_type(type_name);
    if normalized == "boolean" {
        lines.push(format!("{indent}<v8:Type>xs:boolean</v8:Type>"));
    } else if normalized == "DynamicList" {
        lines.push(format!("{indent}<v8:Type>cfg:DynamicList</v8:Type>"));
    } else if matches!(
        normalized.as_str(),
        "ValueTable"
            | "ValueTree"
            | "ValueList"
            | "TypeDescription"
            | "Universal"
            | "FixedArray"
            | "FixedStructure"
    ) {
        let mapped = match normalized.as_str() {
            "ValueTable" => "v8:ValueTable",
            "ValueTree" => "v8:ValueTree",
            "ValueList" => "v8:ValueListType",
            "TypeDescription" => "v8:TypeDescription",
            "Universal" => "v8:Universal",
            "FixedArray" => "v8:FixedArray",
            "FixedStructure" => "v8:FixedStructure",
            _ => unreachable!(),
        };
        lines.push(format!("{indent}<v8:Type>{mapped}</v8:Type>"));
    } else if normalized.starts_with("CatalogRef.")
        || normalized.starts_with("CatalogObject.")
        || normalized.starts_with("DocumentRef.")
        || normalized.starts_with("DocumentObject.")
        || normalized.starts_with("EnumRef.")
        || normalized.starts_with("ChartOfAccountsRef.")
        || normalized.starts_with("ChartOfAccountsObject.")
        || normalized.starts_with("ChartOfCharacteristicTypesRef.")
        || normalized.starts_with("ChartOfCharacteristicTypesObject.")
        || normalized.starts_with("ChartOfCalculationTypesRef.")
        || normalized.starts_with("ChartOfCalculationTypesObject.")
        || normalized.starts_with("ExchangePlanRef.")
        || normalized.starts_with("ExchangePlanObject.")
        || normalized.starts_with("BusinessProcessRef.")
        || normalized.starts_with("BusinessProcessObject.")
        || normalized.starts_with("TaskRef.")
        || normalized.starts_with("TaskObject.")
        || normalized.starts_with("InformationRegisterRecordSet.")
        || normalized.starts_with("InformationRegisterRecordManager.")
        || normalized.starts_with("AccumulationRegisterRecordSet.")
        || normalized.starts_with("AccountingRegisterRecordSet.")
        || normalized.starts_with("ConstantsSet.")
        || normalized.starts_with("DataProcessorObject.")
        || normalized.starts_with("ReportObject.")
    {
        lines.push(format!("{indent}<v8:Type>cfg:{normalized}</v8:Type>"));
    } else if let Some(length) = normalized
        .strip_prefix("string(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        lines.push(format!("{indent}<v8:Type>xs:string</v8:Type>"));
        lines.push(format!("{indent}<v8:StringQualifiers>"));
        lines.push(format!("{indent}\t<v8:Length>{length}</v8:Length>"));
        lines.push(format!(
            "{indent}\t<v8:AllowedLength>Variable</v8:AllowedLength>"
        ));
        lines.push(format!("{indent}</v8:StringQualifiers>"));
    } else if normalized == "string" {
        lines.push(format!("{indent}<v8:Type>xs:string</v8:Type>"));
        lines.push(format!("{indent}<v8:StringQualifiers>"));
        lines.push(format!("{indent}\t<v8:Length>0</v8:Length>"));
        lines.push(format!(
            "{indent}\t<v8:AllowedLength>Variable</v8:AllowedLength>"
        ));
        lines.push(format!("{indent}</v8:StringQualifiers>"));
    } else if let Some((digits, fraction, nonnegative)) = parse_form_decimal_type(&normalized) {
        lines.push(format!("{indent}<v8:Type>xs:decimal</v8:Type>"));
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
    } else if matches!(normalized.as_str(), "date" | "dateTime" | "time") {
        let fractions = match normalized.as_str() {
            "date" => "Date",
            "dateTime" => "DateTime",
            "time" => "Time",
            _ => unreachable!(),
        };
        lines.push(format!("{indent}<v8:Type>xs:dateTime</v8:Type>"));
        lines.push(format!("{indent}<v8:DateQualifiers>"));
        lines.push(format!(
            "{indent}\t<v8:DateFractions>{fractions}</v8:DateFractions>"
        ));
        lines.push(format!("{indent}</v8:DateQualifiers>"));
    } else if normalized.contains('.') {
        lines.push(format!("{indent}<v8:Type>cfg:{normalized}</v8:Type>"));
    } else {
        lines.push(format!(
            "{indent}<v8:Type>{}</v8:Type>",
            escape_xml(&normalized)
        ));
    }
}

pub(crate) fn normalize_form_type(type_name: &str) -> String {
    let stripped = type_name.strip_prefix("cfg:").unwrap_or(type_name);
    if let Some(open) = stripped.find('(') {
        if stripped.ends_with(')') {
            let base = stripped[..open].trim();
            let params = &stripped[open + 1..stripped.len() - 1];
            let normalized = normalize_form_type_base(base).unwrap_or(base);
            return format!("{normalized}({params})");
        }
    }
    if let Some(dot) = stripped.find('.') {
        let prefix = &stripped[..dot];
        let suffix = &stripped[dot..];
        if let Some(normalized) = normalize_form_type_base(prefix) {
            return format!("{normalized}{suffix}");
        }
    }
    normalize_form_type_base(stripped)
        .unwrap_or(stripped)
        .to_string()
}

pub(crate) fn normalize_form_type_base(base: &str) -> Option<&'static str> {
    match base.to_lowercase().as_str() {
        "строка" => Some("string"),
        "число" | "number" => Some("decimal"),
        "булево" | "bool" => Some("boolean"),
        "дата" => Some("date"),
        "датавремя" => Some("dateTime"),
        "справочникссылка" => Some("CatalogRef"),
        "справочникобъект" => Some("CatalogObject"),
        "документссылка" => Some("DocumentRef"),
        "документобъект" => Some("DocumentObject"),
        "перечислениессылка" => Some("EnumRef"),
        "плансчетовссылка" => Some("ChartOfAccountsRef"),
        "планвидовхарактеристикссылка" => {
            Some("ChartOfCharacteristicTypesRef")
        }
        "планвидоврасчётассылка" | "планвидоврасчетассылка" => {
            Some("ChartOfCalculationTypesRef")
        }
        "планобменассылка" => Some("ExchangePlanRef"),
        "бизнеспроцессссылка" => Some("BusinessProcessRef"),
        "задачассылка" => Some("TaskRef"),
        "определяемыйтип" => Some("DefinedType"),
        _ => None,
    }
}

pub(crate) fn parse_form_decimal_type(value: &str) -> Option<(&str, &str, bool)> {
    let rest = value.strip_prefix("decimal(")?.strip_suffix(')')?;
    let parts = rest.split(',').map(str::trim).collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }
    Some((parts[0], parts[1], parts.get(2) == Some(&"nonneg")))
}

pub(crate) fn register_form_in_parent_object(output_path: &Path) -> Result<Option<String>, String> {
    let Some(form_xml_dir) = output_path.parent() else {
        return Ok(None);
    };
    let Some(form_name_dir) = form_xml_dir.parent() else {
        return Ok(None);
    };
    let Some(forms_dir) = form_name_dir.parent() else {
        return Ok(None);
    };
    let Some(object_dir) = forms_dir.parent() else {
        return Ok(None);
    };
    let Some(type_plural_dir) = object_dir.parent() else {
        return Ok(None);
    };
    if forms_dir.file_name().and_then(|value| value.to_str()) != Some("Forms") {
        return Ok(None);
    }
    let Some(form_name) = form_name_dir.file_name().and_then(|value| value.to_str()) else {
        return Ok(None);
    };
    let Some(object_name) = object_dir.file_name().and_then(|value| value.to_str()) else {
        return Ok(None);
    };
    let object_xml_path = type_plural_dir.join(format!("{object_name}.xml"));
    if !object_xml_path.exists() {
        return Ok(None);
    }
    let mut raw_text = fs::read_to_string(&object_xml_path)
        .map_err(|err| format!("failed to read {}: {err}", object_xml_path.display()))?;
    if raw_text.contains(&format!("<Form>{form_name}</Form>")) {
        return Ok(None);
    }
    if raw_text.contains("</ChildObjects>") {
        raw_text = raw_text.replacen(
            "</ChildObjects>",
            &format!("\t\t\t<Form>{form_name}</Form>\n\t\t</ChildObjects>"),
            1,
        );
    } else if raw_text.contains("<ChildObjects/>") {
        raw_text = raw_text.replacen(
            "<ChildObjects/>",
            &format!("<ChildObjects>\n\t\t\t<Form>{form_name}</Form>\n\t\t</ChildObjects>"),
            1,
        );
    } else {
        return Ok(None);
    }
    write_utf8_bom(&object_xml_path, &raw_text)?;
    Ok(Some(format!(
        "     Registered: <Form>{form_name}</Form> in {object_name}.xml\n"
    )))
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
        "form-info" => Some(Ok(analyze_form_info(args, context))),
        "form-validate" => Some(Ok(validate_form(args, context))),
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
        "form-add" => Some(add_form(args, context)),
        "form-remove" => Some(remove_form(args, context)),
        "form-compile" => Some(compile_form(args, context)),
        "form-edit" => Some(edit_form(args, context)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workspace::WorkspaceContext;
    use serde_json::{json, Map};
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_context(name: &str) -> WorkspaceContext {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("unica-form-{name}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build").join("unica"),
            workspace_epoch: 1,
        }
    }

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn validate_form_rejects_bare_type_values() {
        let context = temp_context("bare-type");
        let form_path = context
            .cwd
            .join("Catalogs")
            .join("Goods")
            .join("Forms")
            .join("ItemForm")
            .join("Ext")
            .join("Form.xml");
        write_file(
            &form_path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1">
		<Autofill>true</Autofill>
	</AutoCommandBar>
	<Attributes>
		<Attribute name="BrokenButtonType" id="1">
			<Type>CommandBarButton</Type>
		</Attribute>
	</Attributes>
</Form>
"#,
        );

        let mut args = Map::new();
        args.insert(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        );

        let outcome = validate_form(&args, &context);
        let stdout = outcome.stdout.as_deref().unwrap_or("");

        assert!(!outcome.ok, "{stdout}");
        assert!(
            stdout.contains(
                "[ERROR] 12. Type \"CommandBarButton\": bare type without namespace prefix"
            ),
            "{stdout}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }
}
