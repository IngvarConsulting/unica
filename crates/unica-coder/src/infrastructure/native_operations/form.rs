#![allow(dead_code, unused_imports)]

use crate::application::AdapterOutcome;
use crate::domain::workspace::WorkspaceContext;
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::common::*;
use super::form_event_registry::{
    context_from_root, validate_event, FormDefinitionKind, FormElementKind, FormEventBinding,
    FormEventContext, FormEventDiagnostic, FormEventDiagnosticCode, FormEventTarget,
    MainAttributeKind, MainAttributeProvenance,
};
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
            form_validate_events(root, &mut report);
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
    report: &mut FormValidationReporter,
) {
    struct EventValidationState<'report> {
        context: FormEventContext,
        report: &'report mut FormValidationReporter,
        errors: usize,
        checked: usize,
        unverified: usize,
    }

    impl EventValidationState<'_> {
        fn validate_owner(
            &mut self,
            owner: roxmltree::Node<'_, '_>,
            target: Option<FormEventTarget>,
            target_label: &str,
        ) {
            let Some(events) = form_validation_child(owner, "Events") else {
                return;
            };
            let mut names = HashSet::<String>::new();
            for event in form_validation_children(events, "Event") {
                let name = event.attribute("name").unwrap_or("");
                self.checked += 1;
                if !names.insert(name.to_string()) {
                    self.report.error(form_edit_event_diagnostic(
                        FormEventDiagnosticCode::Duplicate,
                        target_label,
                        name,
                        "event names must be unique within an Events section",
                    ));
                    self.errors += 1;
                }
                let handler = event.text().unwrap_or("").trim();
                let binding = if let Some(call_type) = event.attribute("callType") {
                    FormEventBinding::new(name, handler).with_call_type(call_type)
                } else {
                    FormEventBinding::new(name, handler)
                };
                let validation = target.map_or_else(
                    || {
                        Err(FormEventDiagnostic::new(
                            FormEventDiagnosticCode::EventNotAllowed,
                            target_label,
                            name,
                        )
                        .with_detail("event owner has no registered platform event matrix"))
                    },
                    |target| {
                        validate_event_owner_node(owner, target, &binding)
                            .and_then(|_| validate_event(&self.context, target, &binding))
                    },
                );
                if let Err(mut diagnostic) = validation {
                    diagnostic.target = target_label.to_string();
                    if diagnostic.code == FormEventDiagnosticCode::ContextUnknown
                        && self.context.main_attribute_provenance
                            == MainAttributeProvenance::InheritedBaseFormUnavailable
                    {
                        self.report.warn(format!(
                        "{}; inherited base-form context is unavailable, so this binding was not verified",
                        diagnostic
                    ));
                        self.unverified += 1;
                    } else {
                        self.report.error(diagnostic.to_string());
                        self.errors += 1;
                    }
                }
                if self.report.stopped {
                    return;
                }
            }
        }
    }

    let base_form = form_validation_child(root, "BaseForm");
    let mut state = EventValidationState {
        context: context_from_root(root),
        report,
        errors: 0,
        checked: 0,
        unverified: 0,
    };
    state.validate_owner(root, Some(FormEventTarget::Form), "form");

    for owner in root.descendants().skip(1).filter(|node| {
        node.is_element()
            && form_validation_child(*node, "Events").is_some()
            && !base_form.is_some_and(|base| {
                *node == base || node.ancestors().any(|ancestor| ancestor == base)
            })
    }) {
        let tag = owner.tag_name().name();
        let name = owner.attribute("name").unwrap_or("");
        let target_label = if name.is_empty() {
            format!("{tag} element")
        } else {
            format!("{tag} element '{name}'")
        };
        let target = FormElementKind::from_xml_tag(tag).map(FormEventTarget::Element);
        state.validate_owner(owner, target, &target_label);
        if state.report.stopped {
            return;
        }
    }

    if state.errors == 0 && state.unverified == 0 && state.checked > 0 {
        state
            .report
            .ok(format!("Event handlers: {} events checked", state.checked));
    }
}

fn validate_event_owner_node(
    owner: roxmltree::Node<'_, '_>,
    target: FormEventTarget,
    binding: &FormEventBinding<'_>,
) -> Result<(), FormEventDiagnostic> {
    if target == FormEventTarget::Element(FormElementKind::Table)
        && form_validation_child_text(owner, "DataPath").is_none_or(|path| path.trim().is_empty())
    {
        return Err(FormEventDiagnostic::new(
            FormEventDiagnosticCode::EventNotAllowed,
            "table",
            binding.name,
        )
        .with_detail(
            "Table event bindings require a non-empty direct DataPath; the platform drops bindings on unbound tables",
        ));
    }
    Ok(())
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
    _elements: &[FormElementInfo<'_>],
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
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == "Type"
                && form_is_data_type_declaration_type_node(*node)
        })
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

pub(crate) fn form_is_data_type_declaration_type_node(node: roxmltree::Node<'_, '_>) -> bool {
    let Some(parent) = node.parent_element() else {
        return false;
    };
    match parent.tag_name().name() {
        "Attribute" | "Parameter" | "Column" => true,
        "Type" => parent.parent_element().is_some_and(|grandparent| {
            matches!(
                grandparent.tag_name().name(),
                "Attribute" | "Parameter" | "Column"
            )
        }),
        _ => false,
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
        "CalculationRegisterRecordSet",
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
        let base_form = form_child(root, "BaseForm");
        let is_extension = base_form.is_some();
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

        if let Some(base_form) = base_form {
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
        let set_default = optional_bool_arg(args, &["setDefault", "SetDefault"]);

        let object_xml_full =
            resolve_form_add_object_path(absolutize(object_path_raw, &context.cwd))?;
        let object_source_text = fs::read_to_string(&object_xml_full)
            .map_err(|err| format!("failed to read {}: {err}", object_xml_full.display()))?
            .trim_start_matches('\u{feff}')
            .to_string();
        let mut object_text = object_source_text.clone();
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
        let default_updated = match set_default {
            Some(false) => false,
            Some(true) => {
                let (updated_text, updated) = replace_form_default_property(
                    &object_text,
                    default_prop_name,
                    &default_value,
                    true,
                );
                object_text = updated_text;
                updated
            }
            None => {
                let (updated_text, updated) = replace_form_default_property(
                    &object_text,
                    default_prop_name,
                    &default_value,
                    false,
                );
                object_text = updated_text;
                updated
            }
        };
        write_utf8_bom(
            &object_xml_full,
            &lxml_tree_serialized_text_like_source_preserving_final_newline(
                &object_text,
                &object_source_text,
            ),
        )?;

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

        let source_xml_text = fs::read_to_string(&root_xml_path)
            .map_err(|err| format!("failed to read {}: {err}", root_xml_path.display()))?;
        let form_ref_suffix = format!("Form.{form_name}");
        let (xml_text, removed_form_refs) =
            remove_form_reference_elements(&source_xml_text, &form_ref_suffix);
        let (mut xml_text, cleared_form_slots) =
            clear_form_slot_references(&xml_text, &form_ref_suffix);
        if !cleared_form_slots.is_empty() {
            let tags = cleared_form_slots
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            xml_text = collapse_empty_xml_elements(&xml_text, &tags);
        }
        xml_text = preserve_source_final_newline(xml_text, &source_xml_text);
        if removed_form_refs > 0 {
            changes.push(format!("removed {removed_form_refs} Form reference(s)"));
        }
        for tag in cleared_form_slots {
            changes.push(format!("cleared {tag}"));
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

pub(crate) fn remove_form_reference_elements(xml_text: &str, suffix: &str) -> (String, usize) {
    remove_form_reference_elements_with_parent_context(xml_text, suffix)
        .unwrap_or_else(|| rewrite_simple_form_references(xml_text, suffix, true))
}

pub(crate) fn clear_form_slot_references(xml_text: &str, suffix: &str) -> (String, Vec<String>) {
    let (text, _cleared_count) = rewrite_simple_form_references(xml_text, suffix, false);
    let mut cleared = Vec::new();
    let mut cursor = 0;
    while let Some(open_rel) = xml_text[cursor..].find('<') {
        let open_start = cursor + open_rel;
        let Some(open_end_rel) = xml_text[open_start..].find('>') else {
            break;
        };
        let open_end = open_start + open_end_rel;
        let tag = xml_text[open_start + 1..open_end]
            .split_whitespace()
            .next()
            .unwrap_or("");
        let local_name = tag.rsplit_once(':').map(|(_, name)| name).unwrap_or(tag);
        if local_name != "Form" && local_name.ends_with("Form") {
            let close = format!("</{tag}>");
            let content_start = open_end + 1;
            if let Some(close_rel) = xml_text[content_start..].find(&close) {
                let close_start = content_start + close_rel;
                let content = &xml_text[content_start..close_start];
                if !content.contains('<') && content.trim().ends_with(suffix) {
                    let tag_name = local_name.to_string();
                    if !cleared.contains(&tag_name) {
                        cleared.push(tag_name);
                    }
                }
                cursor = close_start + close.len();
                continue;
            }
        }
        cursor = open_end + 1;
    }
    (text, cleared)
}

fn rewrite_simple_form_references(
    xml_text: &str,
    suffix: &str,
    remove_form_elements: bool,
) -> (String, usize) {
    let mut result = String::with_capacity(xml_text.len());
    let mut cursor = 0;
    let mut changed = 0;
    while let Some(open_rel) = xml_text[cursor..].find('<') {
        let open_start = cursor + open_rel;
        let Some(open_end_rel) = xml_text[open_start..].find('>') else {
            break;
        };
        let open_end = open_start + open_end_rel;
        let raw_tag = &xml_text[open_start + 1..open_end];
        if raw_tag.starts_with('/')
            || raw_tag.starts_with('?')
            || raw_tag.starts_with('!')
            || raw_tag.ends_with('/')
        {
            result.push_str(&xml_text[cursor..=open_end]);
            cursor = open_end + 1;
            continue;
        }
        let tag = raw_tag.split_whitespace().next().unwrap_or("");
        let local_name = tag.rsplit_once(':').map(|(_, name)| name).unwrap_or(tag);
        let should_consider = if remove_form_elements {
            local_name == "Form"
        } else {
            local_name != "Form" && local_name.ends_with("Form")
        };
        if !should_consider {
            result.push_str(&xml_text[cursor..=open_end]);
            cursor = open_end + 1;
            continue;
        }
        let close = format!("</{tag}>");
        let content_start = open_end + 1;
        let Some(close_rel) = xml_text[content_start..].find(&close) else {
            result.push_str(&xml_text[cursor..=open_end]);
            cursor = open_end + 1;
            continue;
        };
        let close_start = content_start + close_rel;
        let close_end = close_start + close.len();
        let content = &xml_text[content_start..close_start];
        let trimmed = content.trim();
        let short_name = suffix
            .rsplit_once('.')
            .map(|(_, name)| name)
            .unwrap_or(suffix);
        let matches_reference = if remove_form_elements {
            trimmed == short_name || trimmed.ends_with(suffix)
        } else {
            trimmed.ends_with(suffix)
        };
        if content.contains('<') || !matches_reference {
            result.push_str(&xml_text[cursor..content_start]);
            cursor = content_start;
            continue;
        }
        let prefix = &xml_text[cursor..open_start];
        if !(remove_form_elements && prefix.trim().is_empty()) {
            result.push_str(prefix);
        }
        if !remove_form_elements {
            result.push_str(&xml_text[open_start..content_start]);
            result.push_str(&xml_text[close_start..close_end]);
        }
        cursor = if remove_form_elements {
            skip_xml_whitespace(xml_text, close_end)
        } else {
            close_end
        };
        changed += 1;
    }
    result.push_str(&xml_text[cursor..]);
    (result, changed)
}

#[derive(Debug)]
struct XmlTextReplacement {
    range: Range<usize>,
    replacement: String,
}

fn remove_form_reference_elements_with_parent_context(
    xml_text: &str,
    suffix: &str,
) -> Option<(String, usize)> {
    let parse_text = xml_text.trim_start_matches('\u{feff}');
    let offset = xml_text.len() - parse_text.len();
    let doc = Document::parse(parse_text).ok()?;
    let short_name = suffix
        .rsplit_once('.')
        .map(|(_, name)| name)
        .unwrap_or(suffix);
    let mut replacements = Vec::new();

    for node in doc.descendants().filter(|node| node.is_element()) {
        if node.tag_name().name() != "Form" {
            continue;
        }
        let trimmed = node.text().unwrap_or("").trim();
        if trimmed != short_name && !trimmed.ends_with(suffix) {
            continue;
        }
        let range = offset_xml_range(node.range(), offset);
        let parent = node.parent()?;
        if parent.is_element()
            && parent.tag_name().name() == "ChildObjects"
            && parent.children().all(|child| {
                child == node || (child.is_text() && child.text().unwrap_or("").trim().is_empty())
            })
        {
            let parent_range = offset_xml_range(parent.range(), offset);
            replacements.push(XmlTextReplacement {
                replacement: self_closing_xml_element(xml_text, &parent_range)?,
                range: parent_range,
            });
        } else {
            replacements.push(XmlTextReplacement {
                range: xml_element_line_range(xml_text, range),
                replacement: String::new(),
            });
        }
    }

    if replacements.is_empty() {
        return Some((xml_text.to_string(), 0));
    }
    replacements.sort_by_key(|replacement| std::cmp::Reverse(replacement.range.start));
    let mut updated = xml_text.to_string();
    for replacement in &replacements {
        updated.replace_range(replacement.range.clone(), &replacement.replacement);
    }
    Some((updated, replacements.len()))
}

fn offset_xml_range(range: Range<usize>, offset: usize) -> Range<usize> {
    range.start + offset..range.end + offset
}

fn xml_element_line_range(xml_text: &str, range: Range<usize>) -> Range<usize> {
    let line_start = xml_text[..range.start].rfind('\n').map_or(0, |pos| pos + 1);
    let prefix_is_indent = xml_text[line_start..range.start]
        .chars()
        .all(|ch| ch == '\t' || ch == ' ');
    let start = if prefix_is_indent {
        line_start
    } else {
        range.start
    };
    let rest = &xml_text[range.end..];
    let end = if rest.starts_with("\r\n") {
        range.end + 2
    } else if rest.starts_with('\n') || rest.starts_with('\r') {
        range.end + 1
    } else {
        range.end
    };
    start..end
}

fn self_closing_xml_element(xml_text: &str, range: &Range<usize>) -> Option<String> {
    let open_end = range.start + xml_text[range.start..].find('>')?;
    let raw_tag = &xml_text[range.start + 1..open_end];
    Some(format!("<{}/>", raw_tag.trim_end()))
}

fn collapse_empty_xml_elements(xml_text: &str, local_names: &[&str]) -> String {
    let mut result = String::with_capacity(xml_text.len());
    let mut cursor = 0;
    while let Some(open_rel) = xml_text[cursor..].find('<') {
        let open_start = cursor + open_rel;
        let Some(open_end_rel) = xml_text[open_start..].find('>') else {
            break;
        };
        let open_end = open_start + open_end_rel;
        let raw_tag = &xml_text[open_start + 1..open_end];
        if raw_tag.starts_with('/')
            || raw_tag.starts_with('?')
            || raw_tag.starts_with('!')
            || raw_tag.trim_end().ends_with('/')
        {
            result.push_str(&xml_text[cursor..=open_end]);
            cursor = open_end + 1;
            continue;
        }
        let tag = raw_tag.split_whitespace().next().unwrap_or("");
        let local_name = tag.rsplit_once(':').map(|(_, name)| name).unwrap_or(tag);
        if !local_names.contains(&local_name) {
            result.push_str(&xml_text[cursor..=open_end]);
            cursor = open_end + 1;
            continue;
        }
        let close = format!("</{tag}>");
        let content_start = open_end + 1;
        let Some(close_rel) = xml_text[content_start..].find(&close) else {
            result.push_str(&xml_text[cursor..=open_end]);
            cursor = open_end + 1;
            continue;
        };
        let close_start = content_start + close_rel;
        let close_end = close_start + close.len();
        let content = &xml_text[content_start..close_start];
        if !content.trim().is_empty() {
            result.push_str(&xml_text[cursor..=open_end]);
            cursor = open_end + 1;
            continue;
        }
        result.push_str(&xml_text[cursor..open_start]);
        result.push('<');
        result.push_str(raw_tag.trim_end());
        result.push_str("/>");
        cursor = close_end;
    }
    result.push_str(&xml_text[cursor..]);
    result
}

fn skip_xml_whitespace(xml_text: &str, mut cursor: usize) -> usize {
    let bytes = xml_text.as_bytes();
    while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t' | b'\r' | b'\n') {
        cursor += 1;
    }
    cursor
}

pub(crate) struct FormCompileObjectField {
    pub(crate) name: String,
    pub(crate) type_name: String,
}

pub(crate) struct FormCompileObjectTabularSection {
    pub(crate) name: String,
    pub(crate) synonym: String,
    pub(crate) columns: Vec<FormCompileObjectField>,
}

pub(crate) struct FormCompileObjectMeta {
    pub(crate) object_type: String,
    pub(crate) name: String,
    pub(crate) synonym: String,
    pub(crate) attributes: Vec<FormCompileObjectField>,
    pub(crate) tabular_sections: Vec<FormCompileObjectTabularSection>,
    pub(crate) code_length: i64,
    pub(crate) hierarchical: bool,
    pub(crate) hierarchy_type: String,
    pub(crate) owners: Vec<String>,
}

pub(crate) fn form_compile_normalize_from_object_output_label(
    output_label: &str,
) -> Option<(String, String)> {
    let trimmed = output_label.trim_end_matches(['/', '\\']);
    if trimmed.ends_with("/Ext/Form.xml") || trimmed.ends_with("\\Ext\\Form.xml") {
        return None;
    }
    let normalized = if trimmed.ends_with("/Ext") || trimmed.ends_with("\\Ext") {
        format!("{trimmed}/Form.xml")
    } else {
        format!("{trimmed}/Ext/Form.xml")
    };
    Some((
        normalized.clone(),
        format!("[resolved] OutputPath -> {normalized}\n"),
    ))
}

pub(crate) fn form_compile_infer_from_object_target(
    output_path: &Path,
    context: &WorkspaceContext,
) -> (Option<PathBuf>, Option<&'static str>) {
    let components = output_path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let Some(forms_index) = components
        .iter()
        .rposition(|component| component == "Forms")
    else {
        return (None, None);
    };
    if forms_index < 2 || forms_index + 1 >= components.len() {
        return (None, None);
    }

    let form_name = components[forms_index + 1].as_str();
    let purpose = match form_name {
        "ФормаЭлемента" | "ФормаДокумента" | "ФормаСчета" => {
            Some("Item")
        }
        "ФормаГруппы" => Some("Folder"),
        "ФормаСписка" => Some("List"),
        "ФормаВыбора" => Some("Choice"),
        "ФормаЗаписи" => Some("Record"),
        _ => None,
    };

    let object_name = components[forms_index - 1].as_str();
    let mut object_path = PathBuf::new();
    for component in &components[..forms_index - 1] {
        object_path.push(component);
    }
    object_path.push(format!("{object_name}.xml"));
    let object_path = absolutize(object_path, &context.cwd);
    if object_path.exists() {
        (Some(object_path), purpose)
    } else {
        (None, purpose)
    }
}

pub(crate) fn form_compile_definition_from_object(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    output_path: &Path,
) -> Result<(Value, String), String> {
    let (inferred_object_path, inferred_purpose) =
        form_compile_infer_from_object_target(output_path, context);
    let (object_path, mut stdout) = if let Some(object_path_raw) =
        path_arg(args, &["objectPath", "ObjectPath"])
    {
        let mut object_path = absolutize(object_path_raw, &context.cwd);
        if object_path.extension().is_none() {
            object_path.set_extension("xml");
        }
        (object_path, String::new())
    } else if let Some(object_path) = inferred_object_path {
        (
            object_path.clone(),
            format!("[resolved] ObjectPath -> {}\n", object_path.display()),
        )
    } else {
        return Err(
            "Cannot derive object path from OutputPath. Use -ObjectPath explicitly.".to_string(),
        );
    };
    if !object_path.exists() {
        return Err(format!("Object file not found: {}", object_path.display()));
    }
    let object_text = read_utf8_sig(&object_path)?;
    let meta = form_compile_parse_object_meta(&object_text)?;
    let purpose = string_arg(args, &["purpose", "Purpose"])
        .or(inferred_purpose)
        .unwrap_or("Item");
    if string_arg(args, &["purpose", "Purpose"]).is_none() && inferred_purpose.is_some() {
        stdout.push_str(&format!("[resolved] Purpose -> {purpose}\n"));
    }

    let defn = match (meta.object_type.as_str(), purpose) {
        ("Catalog", "List") => form_compile_catalog_list_definition(&meta),
        ("Catalog", "Item") => form_compile_catalog_item_definition(&meta),
        ("Catalog", other) => {
            return Err(format!(
                "native form compiler from-object currently supports Catalog List, Catalog Item, Document List, and Document Item only; got Catalog {other}"
            ));
        }
        ("Document", "List") => form_compile_document_list_definition(&meta),
        ("Document", "Item") => form_compile_document_item_definition(&meta),
        ("Document", other) => {
            return Err(format!(
                "native form compiler from-object currently supports Document List and Document Item only; got Document {other}"
            ));
        }
        (other, _) => {
            return Err(format!(
                "Object type '{other}' not supported. Supported: Catalog, Document."
            ));
        }
    };
    stdout.push_str(&format!(
        "[from-object] Type={}, Name={}, Attrs={}, TS={}\n",
        meta.object_type,
        meta.name,
        meta.attributes.len(),
        meta.tabular_sections.len()
    ));
    Ok((defn, stdout))
}

pub(crate) fn form_compile_parse_object_meta(
    object_text: &str,
) -> Result<FormCompileObjectMeta, String> {
    let doc = Document::parse(object_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let root = doc.root_element();
    let type_node = root
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| "Not a 1C metadata XML".to_string())?;
    let object_type = type_node.tag_name().name().to_string();
    let props = meta_info_child(type_node, "Properties")
        .ok_or_else(|| "No <Properties> element found".to_string())?;
    let name = meta_info_child_text(props, "Name").unwrap_or_default();
    let synonym = form_compile_meta_synonym(props).unwrap_or_else(|| name.clone());
    let child_objects = meta_info_child(type_node, "ChildObjects");
    let attributes = child_objects
        .map(|node| form_compile_object_fields(node, "Attribute"))
        .unwrap_or_default();
    let tabular_sections = child_objects
        .map(form_compile_object_tabular_sections)
        .unwrap_or_default();
    let code_length = meta_info_child_text(props, "CodeLength")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let hierarchical = meta_info_child_text(props, "Hierarchical").as_deref() == Some("true");
    let hierarchy_type = meta_info_child_text(props, "HierarchyType").unwrap_or_default();
    let owners = meta_info_child(props, "Owners")
        .map(form_compile_meta_collection_values)
        .unwrap_or_default();

    Ok(FormCompileObjectMeta {
        object_type,
        name,
        synonym,
        attributes,
        tabular_sections,
        code_length,
        hierarchical,
        hierarchy_type,
        owners,
    })
}

pub(crate) fn form_compile_meta_synonym(props: roxmltree::Node<'_, '_>) -> Option<String> {
    let synonym = meta_info_child(props, "Synonym")?;
    for item in meta_info_children(synonym, "item") {
        let lang = meta_info_child_text(item, "lang").unwrap_or_default();
        if lang == "ru" {
            if let Some(content) = meta_info_child_text(item, "content") {
                if !content.is_empty() {
                    return Some(content);
                }
            }
        }
    }
    meta_info_child(synonym, "content")
        .map(meta_info_inner_text)
        .filter(|value| !value.is_empty())
}

pub(crate) fn form_compile_object_fields(
    child_objects: roxmltree::Node<'_, '_>,
    tag_name: &str,
) -> Vec<FormCompileObjectField> {
    meta_info_children(child_objects, tag_name)
        .into_iter()
        .filter_map(|field| {
            let props = meta_info_child(field, "Properties")?;
            let name = meta_info_child_text(props, "Name")?;
            let type_name = meta_info_child(props, "Type")
                .map(form_compile_type_xml_text)
                .unwrap_or_else(|| "string".to_string());
            Some(FormCompileObjectField { name, type_name })
        })
        .collect()
}

pub(crate) fn form_compile_object_tabular_sections(
    child_objects: roxmltree::Node<'_, '_>,
) -> Vec<FormCompileObjectTabularSection> {
    meta_info_children(child_objects, "TabularSection")
        .into_iter()
        .filter_map(|tabular_section| {
            let props = meta_info_child(tabular_section, "Properties")?;
            let name = meta_info_child_text(props, "Name")?;
            let synonym = form_compile_meta_synonym(props).unwrap_or_else(|| name.clone());
            let columns = meta_info_child(tabular_section, "ChildObjects")
                .map(|node| form_compile_object_fields(node, "Attribute"))
                .unwrap_or_default();
            Some(FormCompileObjectTabularSection {
                name,
                synonym,
                columns,
            })
        })
        .collect()
}

pub(crate) fn form_compile_meta_collection_values(node: roxmltree::Node<'_, '_>) -> Vec<String> {
    node.descendants()
        .filter(|child| child.is_element() && child != &node)
        .filter_map(|child| child.text())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(crate) fn form_compile_type_xml_text(type_node: roxmltree::Node<'_, '_>) -> String {
    let types = meta_info_children(type_node, "Type")
        .into_iter()
        .filter_map(|node| node.text().map(str::to_string))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if types.is_empty() {
        "string".to_string()
    } else {
        types.join(" | ")
    }
}

pub(crate) fn form_compile_displayable_type(type_name: &str) -> bool {
    !["ValueStorage", "v8:ValueStorage", "ХранилищеЗначения"]
        .iter()
        .any(|needle| type_name.contains(needle))
}

pub(crate) fn form_compile_catalog_list_definition(meta: &FormCompileObjectMeta) -> Value {
    let mut columns = Vec::new();
    columns.push(json!({"labelField": "Наименование", "path": "Список.Description"}));
    if meta.code_length > 0 {
        columns.push(json!({"labelField": "Код", "path": "Список.Code"}));
    }
    for attr in &meta.attributes {
        if form_compile_displayable_type(&attr.type_name) {
            columns.push(json!({
                "labelField": attr.name,
                "path": format!("Список.{}", attr.name),
            }));
        }
    }
    columns.push(json!({
        "labelField": "Ссылка",
        "path": "Список.Ref",
        "userVisible": false,
    }));

    let mut table = json!({
        "table": "Список",
        "path": "Список",
        "rowPictureDataPath": "Список.DefaultPicture",
        "commandBarLocation": "None",
        "tableAutofill": false,
        "_dynList": true,
        "columns": columns,
    });
    if meta.hierarchical {
        if let Some(object) = table.as_object_mut() {
            object.insert("initialTreeView".to_string(), json!("ExpandTopLevel"));
            object.insert("enableStartDrag".to_string(), json!(true));
            object.insert("enableDrag".to_string(), json!(true));
        }
    }

    json!({
        "title": meta.synonym,
        "elements": [table],
        "attributes": [{
            "name": "Список",
            "type": "DynamicList",
            "main": true,
            "settings": {
                "mainTable": format!("Catalog.{}", meta.name),
                "dynamicDataRead": true,
            },
        }],
    })
}

pub(crate) fn form_compile_catalog_item_definition(meta: &FormCompileObjectMeta) -> Value {
    let mut header_children = Vec::new();
    if !meta.owners.is_empty() {
        header_children.push(json!({
            "input": "Владелец",
            "path": "Объект.Owner",
            "readOnly": true,
        }));
    }

    if meta.code_length > 0 {
        header_children.push(json!({
            "group": "horizontal",
            "name": "ГруппаКодНаименование",
            "showTitle": false,
            "representation": "none",
            "children": [
                {"input": "Наименование", "path": "Объект.Description"},
                {"input": "Код", "path": "Объект.Code"},
            ],
        }));
    } else {
        header_children.push(json!({"input": "Наименование", "path": "Объект.Description"}));
    }

    if meta.hierarchical {
        header_children.push(json!({
            "input": "Родитель",
            "path": "Объект.Parent",
            "title": "Входит в группу",
        }));
    }

    for attr in &meta.attributes {
        if form_compile_displayable_type(&attr.type_name) {
            header_children.push(form_compile_object_field_element(
                &attr.name,
                &format!("Объект.{}", attr.name),
                &attr.type_name,
            ));
        }
    }

    let mut root_elements = vec![json!({
        "group": "vertical",
        "name": "ГруппаШапка",
        "showTitle": false,
        "representation": "none",
        "children": header_children,
    })];

    for tabular_section in meta.tabular_sections.iter().filter(|section| {
        section.name != "ДополнительныеРеквизиты" && section.name != "Представления"
    }) {
        let mut columns = vec![json!({
            "labelField": format!("{}НомерСтроки", tabular_section.name),
            "path": format!("Объект.{}.LineNumber", tabular_section.name),
        })];
        for column in &tabular_section.columns {
            columns.push(form_compile_object_field_element(
                &format!("{}{}", tabular_section.name, column.name),
                &format!("Объект.{}.{}", tabular_section.name, column.name),
                &column.type_name,
            ));
        }
        root_elements.push(json!({
            "table": tabular_section.name,
            "path": format!("Объект.{}", tabular_section.name),
            "columns": columns,
        }));
    }

    root_elements.push(json!({
        "group": "vertical",
        "name": "ГруппаДополнительныеРеквизиты",
    }));

    let mut defn = json!({
        "title": meta.synonym,
        "properties": {},
        "elements": root_elements,
        "attributes": [{
            "name": "Объект",
            "type": format!("CatalogObject.{}", meta.name),
            "main": true,
        }],
    });
    if meta.hierarchical && meta.hierarchy_type == "HierarchyFoldersAndItems" {
        if let Some(properties) = defn.get_mut("properties").and_then(Value::as_object_mut) {
            properties.insert("useForFoldersAndItems".to_string(), json!("Items"));
        }
    }
    defn
}

pub(crate) fn form_compile_document_list_definition(meta: &FormCompileObjectMeta) -> Value {
    let mut columns = Vec::new();
    columns.push(json!({"labelField": "Номер", "path": "Список.Number"}));
    columns.push(json!({"labelField": "Дата", "path": "Список.Date"}));
    for attr in &meta.attributes {
        if form_compile_displayable_type(&attr.type_name) {
            columns.push(json!({
                "labelField": attr.name,
                "path": format!("Список.{}", attr.name),
            }));
        }
    }
    columns.push(json!({
        "labelField": "Ссылка",
        "path": "Список.Ref",
        "userVisible": false,
    }));

    json!({
        "title": meta.synonym,
        "properties": {},
        "elements": [{
            "table": "Список",
            "path": "Список",
            "rowPictureDataPath": "Список.DefaultPicture",
            "commandBarLocation": "None",
            "tableAutofill": false,
            "_dynList": true,
            "columns": columns,
        }],
        "attributes": [{
            "name": "Список",
            "type": "DynamicList",
            "main": true,
            "settings": {
                "mainTable": format!("Document.{}", meta.name),
                "dynamicDataRead": true,
            },
        }],
    })
}

pub(crate) fn form_compile_document_item_definition(meta: &FormCompileObjectMeta) -> Value {
    let footer_fields = ["Комментарий"];
    let mut claimed = HashSet::<&str>::new();
    for field in footer_fields {
        claimed.insert(field);
    }

    let unclaimed = meta
        .attributes
        .iter()
        .filter(|attr| {
            !claimed.contains(attr.name.as_str()) && form_compile_displayable_type(&attr.type_name)
        })
        .collect::<Vec<_>>();
    let half = unclaimed.len().div_ceil(2);
    let (left_attrs, right_attrs) = unclaimed.split_at(half);

    let number_date_group = json!({
        "group": "horizontal",
        "name": "ГруппаНомерДата",
        "showTitle": false,
        "children": [
            {"input": "Номер", "path": "Объект.Number", "autoMaxWidth": false, "width": 9},
            {"input": "Дата", "path": "Объект.Date", "title": "от"},
        ],
    });

    let mut left_children = vec![number_date_group];
    for attr in left_attrs {
        left_children.push(form_compile_object_field_element(
            &attr.name,
            &format!("Объект.{}", attr.name),
            &attr.type_name,
        ));
    }

    let mut right_children = Vec::new();
    for attr in right_attrs {
        right_children.push(form_compile_object_field_element(
            &attr.name,
            &format!("Объект.{}", attr.name),
            &attr.type_name,
        ));
    }

    let header_children = if right_children.is_empty() {
        vec![json!({
            "group": "vertical",
            "name": "ГруппаШапкаЛево",
            "showTitle": false,
            "children": left_children,
        })]
    } else {
        vec![
            json!({
                "group": "vertical",
                "name": "ГруппаШапкаЛево",
                "showTitle": false,
                "children": left_children,
            }),
            json!({
                "group": "vertical",
                "name": "ГруппаШапкаПраво",
                "showTitle": false,
                "children": right_children,
            }),
        ]
    };

    let header_group = json!({
        "group": "horizontal",
        "name": "ГруппаШапка",
        "showTitle": false,
        "representation": "none",
        "children": header_children,
    });

    let mut main_page_children = vec![header_group];
    for field in footer_fields {
        if let Some(attr) = meta.attributes.iter().find(|attr| attr.name == field) {
            main_page_children.push(form_compile_object_field_element(
                &attr.name,
                &format!("Объект.{}", attr.name),
                &attr.type_name,
            ));
        }
    }

    let mut pages_children = vec![json!({
        "page": "ГруппаОсновное",
        "title": "Основное",
        "children": main_page_children,
    })];

    for tabular_section in meta
        .tabular_sections
        .iter()
        .filter(|section| section.name != "ДополнительныеРеквизиты")
    {
        let mut columns = vec![json!({
            "labelField": format!("{}НомерСтроки", tabular_section.name),
            "path": format!("Объект.{}.LineNumber", tabular_section.name),
        })];
        for column in &tabular_section.columns {
            columns.push(form_compile_object_field_element(
                &format!("{}{}", tabular_section.name, column.name),
                &format!("Объект.{}.{}", tabular_section.name, column.name),
                &column.type_name,
            ));
        }
        pages_children.push(json!({
            "page": format!("Группа{}", tabular_section.name),
            "title": tabular_section.synonym,
            "children": [{
                "table": tabular_section.name,
                "path": format!("Объект.{}", tabular_section.name),
                "columns": columns,
            }],
        }));
    }

    pages_children.push(json!({
        "page": "ГруппаДополнительно",
        "title": "Дополнительно",
        "children": [
            {
                "group": "horizontal",
                "name": "ГруппаПараметры",
                "showTitle": false,
                "children": [
                    {"group": "vertical", "name": "ГруппаПараметрыЛево", "showTitle": false, "children": []},
                    {"group": "vertical", "name": "ГруппаПараметрыПраво", "showTitle": false, "children": []},
                ],
            },
            {"group": "vertical", "name": "ГруппаДополнительныеРеквизиты"},
        ],
    }));

    json!({
        "title": meta.synonym,
        "properties": {
            "autoTitle": false,
        },
        "elements": [{
            "pages": "ГруппаСтраницы",
            "children": pages_children,
        }],
        "attributes": [{
            "name": "Объект",
            "type": format!("DocumentObject.{}", meta.name),
            "main": true,
        }],
    })
}

pub(crate) fn form_compile_object_field_element(name: &str, path: &str, type_name: &str) -> Value {
    if type_name.trim() == "xs:boolean" || type_name == "boolean" || type_name.contains("Boolean") {
        json!({"check": name, "path": path})
    } else {
        json!({"input": name, "path": path})
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
    overwrite: bool,
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
    if !overwrite && !text[value_start..value_end].trim().is_empty() {
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

struct FormCompilePlan {
    output_label: String,
    output_path: PathBuf,
    stdout: String,
    xml: String,
    stats: FormCompileStats,
}

pub(crate) fn compile_form(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let write_result = plan_form_compile(args, context).and_then(|mut plan| {
        let output_path = plan.output_path.clone();
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        write_utf8_bom(&output_path, &plan.xml)?;

        if let Some(registered) = register_form_in_parent_object(&output_path)? {
            plan.stdout.push_str(&registered);
        }
        plan.stdout
            .push_str(&format!("[OK] Compiled: {}\n", plan.output_label));
        append_form_compile_stats(&mut plan.stdout, &plan.stats);

        Ok((plan.stdout, output_path))
    });

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

pub(crate) fn preview_form_compile(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<AdapterOutcome, String> {
    let mut plan = plan_form_compile(args, context)?;
    plan.stdout
        .push_str(&format!("[DRY-RUN] Would compile: {}\n", plan.output_label));
    append_form_compile_stats(&mut plan.stdout, &plan.stats);
    Ok(AdapterOutcome {
        ok: true,
        summary: "dry run: unica.form.compile planned native managed form compilation".to_string(),
        changes: vec![format!("would update {}", plan.output_path.display())],
        warnings: Vec::new(),
        errors: Vec::new(),
        artifacts: Vec::new(),
        stdout: Some(plan.stdout),
        stderr: None,
        command: None,
    })
}

pub(crate) fn has_compile_payload(args: &Map<String, Value>) -> bool {
    const KEYS: &[&str] = &[
        "JsonPath",
        "jsonPath",
        "FromObject",
        "fromObject",
        "ObjectPath",
        "objectPath",
        "OutputPath",
        "outputPath",
    ];
    KEYS.iter().any(|key| args.contains_key(*key))
}

fn plan_form_compile(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<FormCompilePlan, String> {
    let json_path_raw = path_arg(args, &["jsonPath", "JsonPath"]);
    let from_object = bool_arg(args, &["fromObject", "FromObject"]);
    if from_object && json_path_raw.is_some() {
        return Err("Cannot use both -JsonPath and -FromObject. Choose one mode.".to_string());
    }
    if !from_object && json_path_raw.is_none() {
        return Err("Either -JsonPath or -FromObject is required.".to_string());
    }

    let mut output_label = string_arg(args, &["outputPath", "OutputPath"])
        .ok_or_else(|| "missing required OutputPath argument".to_string())?
        .to_string();
    let mut stdout = String::new();
    if from_object {
        if let Some((normalized, resolved_line)) =
            form_compile_normalize_from_object_output_label(&output_label)
        {
            output_label = normalized;
            stdout.push_str(&resolved_line);
        }
    }
    let output_path = absolutize(PathBuf::from(&output_label), &context.cwd);
    let defn = if from_object {
        let (defn, from_object_stdout) =
            form_compile_definition_from_object(args, context, &output_path)?;
        stdout.push_str(&from_object_stdout);
        defn
    } else {
        let json_path_raw = json_path_raw
            .ok_or_else(|| "Either -JsonPath or -FromObject is required.".to_string())?;
        let json_path = absolutize(json_path_raw.clone(), &context.cwd);
        if !json_path.exists() {
            return Err(format!("File not found: {}", json_path_raw.display()));
        }
        let json_text = fs::read_to_string(&json_path)
            .map_err(|err| format!("failed to read {}: {err}", json_path.display()))?;
        serde_json::from_str(json_text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("failed to parse Form JSON: {err}"))?
    };

    let format_version = detect_format_version(output_path.parent().unwrap_or(&context.cwd));
    let (xml, stats) = form_compile_xml(&defn, &format_version)?;
    Ok(FormCompilePlan {
        output_label,
        output_path,
        stdout,
        xml,
        stats,
    })
}

fn append_form_compile_stats(stdout: &mut String, stats: &FormCompileStats) {
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
}

pub(crate) fn edit_form(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    edit_form_with_mode(args, context, FormEditMode::Apply)
}

pub(crate) fn preview_form_edit(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    edit_form_with_mode(args, context, FormEditMode::Preview)
}

pub(crate) fn has_edit_payload(args: &Map<String, Value>) -> bool {
    const KEYS: &[&str] = &[
        "FormPath",
        "formPath",
        "Path",
        "path",
        "JsonPath",
        "jsonPath",
        "definition",
    ];
    args.keys().any(|key| KEYS.contains(&key.as_str()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormEditMode {
    Apply,
    Preview,
}

impl FormEditMode {
    const fn is_preview(self) -> bool {
        matches!(self, Self::Preview)
    }
}

pub(crate) fn edit_form_with_mode(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    mode: FormEditMode,
) -> AdapterOutcome {
    let edit_result = (|| -> Result<(String, PathBuf, bool), String> {
        let form_path_raw =
            required_path(args, &["formPath", "FormPath", "path", "Path"], "FormPath")?;
        let form_path = absolutize(form_path_raw.clone(), &context.cwd);
        if !form_path.exists() {
            return Err(format!("File not found: {}", form_path_raw.display()));
        }

        let defn = form_edit_resolve_definition(args, context)?;
        let original_bytes = fs::read(&form_path)
            .map_err(|err| format!("failed to read {}: {err}", form_path.display()))?;
        let bom = if original_bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
            Utf8Bom::Present
        } else {
            Utf8Bom::Absent
        };
        let content_bytes = if bom == Utf8Bom::Present {
            &original_bytes[3..]
        } else {
            original_bytes.as_slice()
        };
        let mut xml_text = String::from_utf8(content_bytes.to_vec())
            .map_err(|err| format!("{} is not valid UTF-8: {err}", form_path.display()))?;
        let original_xml_text = xml_text.clone();
        Document::parse(&xml_text).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
        if let Some(attributes) = defn.get("attributes").and_then(Value::as_array) {
            form_edit_validate_named_objects(&xml_text, attributes, "Attribute", "attribute")?;
        }
        if let Some(commands) = defn.get("commands").and_then(Value::as_array) {
            form_edit_validate_named_objects(&xml_text, commands, "Command", "command")?;
        }
        let planned_events = form_edit_plan_events(&defn, &xml_text)?;
        let form_root_start = Document::parse(&xml_text)
            .map_err(|err| format!("[ERROR] XML parse error: {err}"))?
            .root_element()
            .range()
            .start;

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
                    "CommandBar",
                ],
            ),
        };
        let mut attr_ids = FormIdAllocator {
            next: form_edit_next_id(&xml_text, &["Attribute", "Column"]),
        };
        let mut cmd_ids = FormIdAllocator {
            next: form_edit_next_id(&xml_text, &["Command"]),
        };
        if form_edit_is_extension_form(&xml_text) {
            elem_ids.next = elem_ids.next.max(999_999);
            attr_ids.next = attr_ids.next.max(999_999);
            cmd_ids.next = cmd_ids.next.max(999_999);
        }

        let mut added_elements = Vec::<String>::new();
        let mut emitted_fragments = String::new();
        let mut companion_count = 0usize;
        if let Some(elements) = defn.get("elements").and_then(Value::as_array) {
            if !elements.is_empty() {
                form_edit_validate_element_names(&xml_text, elements)?;
                let insert_target = form_edit_target_child_items_range(
                    &xml_text,
                    defn.get("into").and_then(Value::as_str),
                    defn.get("after").and_then(Value::as_str),
                )?;
                let element_indent = insert_target.child_indent().to_string();
                let start = elem_ids.next;
                let mut lines = Vec::<String>::new();
                for element in elements {
                    let summary = form_edit_element_summary(element);
                    emit_form_element(&mut lines, element, &element_indent, &mut elem_ids)?;
                    if let Some(summary) = summary {
                        added_elements.push(summary);
                    }
                }
                emitted_fragments.push_str(&lines.join("\n"));
                form_edit_insert_lines_into_target(&mut xml_text, insert_target, &lines)?;
                companion_count = elem_ids.next.saturating_sub(start + added_elements.len());
            }
        }

        let mut added_attrs = Vec::<String>::new();
        if let Some(attrs) = defn.get("attributes").and_then(Value::as_array) {
            if !attrs.is_empty() {
                form_edit_validate_attribute_columns(attrs)?;
                let mut lines = Vec::<String>::new();
                for attr in attrs {
                    let Some(object) = attr.as_object() else {
                        continue;
                    };
                    let Some(name) = object.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    let id = attr_ids.next();
                    emit_form_edit_attribute_item(&mut lines, object, name, id, "\t\t")?;
                    let type_name = object
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("(no type)");
                    added_attrs.push(format!("  + {name}: {type_name} (id={id})"));
                }
                emitted_fragments.push_str(&lines.join("\n"));
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
                emitted_fragments.push_str(&lines.join("\n"));
                form_edit_insert_section_items(&mut xml_text, "Commands", &lines)?;
            }
        }

        let mut added_form_events = Vec::<String>::new();
        let mut added_element_events = Vec::<String>::new();
        for event in &planned_events {
            form_edit_apply_planned_event(&mut xml_text, event)?;
            match &event.owner {
                FormEditEventOwner::Form => added_form_events.push(event.summary()),
                FormEditEventOwner::Element(_) => added_element_events.push(event.summary()),
            }
        }

        form_edit_ensure_emitted_namespaces(&mut xml_text, form_root_start, &emitted_fragments)?;
        Document::parse(&xml_text)
            .map_err(|err| format!("[ERROR] XML parse error after edit: {err}"))?;
        let changed = xml_text != original_xml_text;
        if changed && !mode.is_preview() {
            form_edit_write_preserving_bom(&form_path, &xml_text, bom)?;
        }

        let mut stdout = format!("=== form-edit: {form_name} ===\n\n");
        if !added_form_events.is_empty() {
            stdout.push_str(if mode.is_preview() {
                "Planned form events:\n"
            } else {
                "Added form events:\n"
            });
            stdout.push_str(&added_form_events.join("\n"));
            stdout.push_str("\n\n");
        }
        if !added_element_events.is_empty() {
            stdout.push_str(if mode.is_preview() {
                "Planned element events:\n"
            } else {
                "Added element events:\n"
            });
            stdout.push_str(&added_element_events.join("\n"));
            stdout.push_str("\n\n");
        }
        if !added_elements.is_empty() {
            stdout.push_str(if mode.is_preview() {
                "Planned elements:\n"
            } else {
                "Added elements:\n"
            });
            stdout.push_str(&added_elements.join("\n"));
            stdout.push_str("\n\n");
        }
        if !added_attrs.is_empty() {
            stdout.push_str(if mode.is_preview() {
                "Planned attributes:\n"
            } else {
                "Added attributes:\n"
            });
            stdout.push_str(&added_attrs.join("\n"));
            stdout.push_str("\n\n");
        }
        if !added_cmds.is_empty() {
            stdout.push_str(if mode.is_preview() {
                "Planned commands:\n"
            } else {
                "Added commands:\n"
            });
            stdout.push_str(&added_cmds.join("\n"));
            stdout.push_str("\n\n");
        }
        let mut total_parts = Vec::new();
        if !added_form_events.is_empty() {
            total_parts.push(format!("{} form event(s)", added_form_events.len()));
        }
        if !added_element_events.is_empty() {
            total_parts.push(format!("{} element event(s)", added_element_events.len()));
        }
        if !added_elements.is_empty() {
            if companion_count > 0 {
                total_parts.push(format!(
                    "{} element(s) (+{} companions)",
                    added_elements.len(),
                    companion_count
                ));
            } else {
                total_parts.push(format!("{} element(s)", added_elements.len()));
            }
        }
        if !added_attrs.is_empty() {
            total_parts.push(format!("{} attribute(s)", added_attrs.len()));
        }
        if !added_cmds.is_empty() {
            total_parts.push(format!("{} command(s)", added_cmds.len()));
        }
        stdout.push_str("---\n");
        if changed {
            stdout.push_str(&format!("Total: {}\n", total_parts.join(", ")));
        } else {
            stdout.push_str("Total: idempotent no-op; source bytes preserved.\n");
        }
        stdout.push_str("Run /form-validate to verify.\n");

        Ok((stdout, form_path, changed))
    })();

    match edit_result {
        Ok((stdout, form_path, changed)) => AdapterOutcome {
            ok: true,
            summary: if mode.is_preview() && !changed {
                "dry run: unica.form.edit found an idempotent no-op".to_string()
            } else if !changed {
                "unica.form.edit completed with idempotent no-op".to_string()
            } else if mode.is_preview() {
                "dry run: unica.form.edit planned native managed form changes".to_string()
            } else {
                "unica.form.edit completed with native managed form editor".to_string()
            },
            changes: if changed {
                vec![format!(
                    "{} {}",
                    if mode.is_preview() {
                        "would update"
                    } else {
                        "updated"
                    },
                    form_path.display()
                )]
            } else {
                Vec::new()
            },
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum FormEditEventOwner {
    Form,
    Element(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FormEditEventSlot {
    owner: FormEditEventOwner,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FormEditRequestedBinding {
    handler: String,
    call_type: Option<String>,
}

struct FormEditEventPlanner {
    context: FormEventContext,
    requested: HashMap<FormEditEventSlot, FormEditRequestedBinding>,
    planned: Vec<FormEditPlannedEvent>,
}

impl FormEditEventPlanner {
    fn new(context: FormEventContext) -> Self {
        Self {
            context,
            requested: HashMap::new(),
            planned: Vec::new(),
        }
    }

    fn request(
        &mut self,
        owner_node: roxmltree::Node<'_, '_>,
        target: FormEventTarget,
        event: FormEditPlannedEvent,
    ) -> Result<(), String> {
        let binding = if let Some(call_type) = event.call_type.as_deref() {
            FormEventBinding::new(&event.name, &event.handler).with_call_type(call_type)
        } else {
            FormEventBinding::new(&event.name, &event.handler)
        };
        validate_event_owner_node(owner_node, target, &binding)
            .and_then(|_| validate_event(&self.context, target, &binding))
            .map_err(|diagnostic| diagnostic.to_string())?;

        let slot = FormEditEventSlot {
            owner: event.owner.clone(),
            name: event.name.clone(),
        };
        let requested_binding = FormEditRequestedBinding {
            handler: event.handler.clone(),
            call_type: event.call_type.clone(),
        };
        if let Some(seen) = self.requested.get(&slot) {
            if seen == &requested_binding {
                return Ok(());
            }
            return Err(form_edit_event_diagnostic(
                FormEventDiagnosticCode::BindingConflict,
                form_edit_event_owner_label(&event.owner),
                &event.name,
                "the request contains conflicting handlers or callType values for the same event slot",
            ));
        }
        self.requested.insert(slot, requested_binding);

        let existing = form_validation_child(owner_node, "Events")
            .map(|events| {
                form_validation_children(events, "Event")
                    .into_iter()
                    .filter(|candidate| candidate.attribute("name") == Some(event.name.as_str()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if existing.len() > 1 {
            return Err(form_edit_event_diagnostic(
                FormEventDiagnosticCode::Duplicate,
                form_edit_event_owner_label(&event.owner),
                &event.name,
                "the source form already contains duplicate bindings for this event slot",
            ));
        }
        if let Some(existing_event) = existing.first() {
            let existing_handler = existing_event.text().unwrap_or("").trim();
            let existing_call_type = existing_event.attribute("callType");
            if existing_handler == event.handler && existing_call_type == event.call_type.as_deref()
            {
                return Ok(());
            }
            return Err(form_edit_event_diagnostic(
                FormEventDiagnosticCode::BindingConflict,
                form_edit_event_owner_label(&event.owner),
                &event.name,
                format!(
                    "existing binding is handler='{existing_handler}', callType={existing_call_type:?}"
                ),
            ));
        }

        self.planned.push(event);
        Ok(())
    }

    fn finish(self) -> Vec<FormEditPlannedEvent> {
        self.planned
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FormEditPlannedEvent {
    pub(crate) owner: FormEditEventOwner,
    pub(crate) name: String,
    pub(crate) handler: String,
    pub(crate) call_type: Option<String>,
}

impl FormEditPlannedEvent {
    pub(crate) fn summary(&self) -> String {
        let call_type = self
            .call_type
            .as_deref()
            .map(|value| format!("[{value}]"))
            .unwrap_or_default();
        match &self.owner {
            FormEditEventOwner::Form => {
                format!("  + {}{call_type} -> {}", self.name, self.handler)
            }
            FormEditEventOwner::Element(element) => {
                format!("  + {element}.{}{call_type} -> {}", self.name, self.handler)
            }
        }
    }
}

pub(crate) fn form_edit_resolve_definition(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<Value, String> {
    let inline = args.get("definition");
    let json_path_raw = path_arg(args, &["jsonPath", "JsonPath"]);
    match (inline, json_path_raw) {
        (Some(_), Some(_)) => {
            Err("unica.form.edit accepts exactly one of JsonPath or inline definition".to_string())
        }
        (Some(definition), None) => {
            if !definition.is_object() {
                return Err("unica.form.edit argument `definition` must be object".to_string());
            }
            Ok(definition.clone())
        }
        (None, Some(json_path_raw)) => {
            let json_path = absolutize(json_path_raw.clone(), &context.cwd);
            if !json_path.exists() {
                return Err(format!("File not found: {}", json_path_raw.display()));
            }
            let json_text = fs::read_to_string(&json_path)
                .map_err(|err| format!("failed to read {}: {err}", json_path.display()))?;
            let definition: Value = serde_json::from_str(json_text.trim_start_matches('\u{feff}'))
                .map_err(|err| format!("failed to parse form edit JSON: {err}"))?;
            if !definition.is_object() {
                return Err("form edit JSON root must be an object".to_string());
            }
            Ok(definition)
        }
        (None, None) => {
            Err("unica.form.edit requires exactly one of JsonPath or definition".to_string())
        }
    }
}

pub(crate) fn form_edit_plan_events(
    definition: &Value,
    xml_text: &str,
) -> Result<Vec<FormEditPlannedEvent>, String> {
    let document = Document::parse(xml_text)
        .map_err(|err| format!("[ERROR] XML parse error while planning events: {err}"))?;
    let root = document.root_element();
    let direct_main_count = form_edit_direct_main_attribute_count(root);
    form_edit_validate_projected_main_attribute_count(direct_main_count, definition)?;
    let context =
        form_project_event_context(context_from_root(root), direct_main_count, definition);
    let mut planner = FormEditEventPlanner::new(context.clone());

    if let Some(values) = definition.get("formEvents") {
        let events = values.as_array().ok_or_else(|| {
            form_edit_event_diagnostic(
                FormEventDiagnosticCode::EventNotAllowed,
                "form",
                "<definition>",
                "formEvents must be an array",
            )
        })?;
        for value in events {
            let (name, handler, call_type) = form_edit_definition_event(value, "name", "form")?;
            planner.request(
                root,
                FormEventTarget::Form,
                FormEditPlannedEvent {
                    owner: FormEditEventOwner::Form,
                    name,
                    handler,
                    call_type,
                },
            )?;
        }
    }

    if let Some(values) = definition.get("elementEvents") {
        let events = values.as_array().ok_or_else(|| {
            form_edit_event_diagnostic(
                FormEventDiagnosticCode::EventNotAllowed,
                "element",
                "<definition>",
                "elementEvents must be an array",
            )
        })?;
        for value in events {
            let object = value.as_object().ok_or_else(|| {
                form_edit_event_diagnostic(
                    FormEventDiagnosticCode::EventNotAllowed,
                    "element",
                    "<definition>",
                    "elementEvents items must be objects",
                )
            })?;
            let element_name = object
                .get("element")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| {
                    form_edit_event_diagnostic(
                        FormEventDiagnosticCode::TargetNotFound,
                        "element",
                        "<definition>",
                        "element must be a non-empty string",
                    )
                })?;
            let event_name = object
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("<definition>");
            let element_node = form_edit_resolve_element_node(root, element_name, event_name)?;
            let kind =
                FormElementKind::from_xml_tag(element_node.tag_name().name()).ok_or_else(|| {
                    form_edit_event_diagnostic(
                        FormEventDiagnosticCode::EventNotAllowed,
                        format!("element '{element_name}'"),
                        object
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("<definition>"),
                        format!(
                            "element tag '{}' has no registered event matrix",
                            element_node.tag_name().name()
                        ),
                    )
                })?;
            let (name, handler, call_type) =
                form_edit_definition_event(value, "name", &format!("element '{element_name}'"))?;
            planner.request(
                element_node,
                FormEventTarget::Element(kind),
                FormEditPlannedEvent {
                    owner: FormEditEventOwner::Element(element_name.to_string()),
                    name,
                    handler,
                    call_type,
                },
            )?;
        }
    }

    if let Some(elements) = definition.get("elements").and_then(Value::as_array) {
        form_edit_validate_new_element_event_tree(elements, &context)?;
    }

    Ok(planner.finish())
}

fn form_project_event_context(
    mut context: FormEventContext,
    direct_main_count: usize,
    definition: &Value,
) -> FormEventContext {
    if direct_main_count != 0 {
        return context;
    }

    let projected_main_attributes = definition
        .get("attributes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter(|attribute| attribute.get("main").and_then(Value::as_bool) == Some(true))
        .filter(|attribute| {
            attribute
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| !name.is_empty())
        })
        .collect::<Vec<_>>();

    if let [attribute] = projected_main_attributes.as_slice() {
        let type_name = attribute
            .get("type")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        context.main_attribute = type_name
            .map(MainAttributeKind::from_type_name)
            .unwrap_or(MainAttributeKind::Unknown);
        context.main_attribute_type = type_name.map(ToOwned::to_owned);
        context.main_attribute_provenance = MainAttributeProvenance::DirectForm;
    }
    context
}

fn form_edit_direct_main_attribute_count(root: roxmltree::Node<'_, '_>) -> usize {
    form_validation_child(root, "Attributes")
        .map(|attributes| {
            form_validation_children(attributes, "Attribute")
                .into_iter()
                .filter(|attribute| {
                    form_validation_child_text(*attribute, "MainAttribute").as_deref()
                        == Some("true")
                })
                .count()
        })
        .unwrap_or(0)
}

fn form_edit_validate_projected_main_attribute_count(
    direct_main_count: usize,
    definition: &Value,
) -> Result<(), String> {
    let added_main_count = definition
        .get("attributes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter(|attribute| attribute.get("main").and_then(Value::as_bool) == Some(true))
        .filter(|attribute| {
            attribute
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| !name.is_empty())
        })
        .count();
    let resulting_count = direct_main_count + added_main_count;
    if resulting_count > 1 {
        return Err(format!(
            "[ERROR] Resulting form would contain {resulting_count} direct MainAttribute=true entries; expected at most one"
        ));
    }
    Ok(())
}

pub(crate) fn form_edit_definition_event(
    value: &Value,
    name_key: &str,
    target: &str,
) -> Result<(String, String, Option<String>), String> {
    let object = value.as_object().ok_or_else(|| {
        form_edit_event_diagnostic(
            FormEventDiagnosticCode::EventNotAllowed,
            target,
            "<definition>",
            "event definition must be an object",
        )
    })?;
    let name = object
        .get(name_key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            form_edit_event_diagnostic(
                FormEventDiagnosticCode::EventNotAllowed,
                target,
                "<definition>",
                format!("{name_key} must be a non-empty string"),
            )
        })?
        .to_string();
    let handler = object
        .get("handler")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            form_edit_event_diagnostic(
                FormEventDiagnosticCode::EmptyHandler,
                target,
                &name,
                "handler must be a string",
            )
        })?
        .trim()
        .to_string();
    let call_type = match object.get("callType") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.clone()),
        Some(_) => {
            return Err(form_edit_event_diagnostic(
                FormEventDiagnosticCode::InvalidCallType,
                target,
                &name,
                "callType must be a string",
            ));
        }
    };
    Ok((name, handler, call_type))
}

fn form_edit_optional_event_handler(
    value: Option<&Value>,
    target: &str,
    event: &str,
) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(form_edit_event_diagnostic(
            FormEventDiagnosticCode::EmptyHandler,
            target,
            event,
            "handler must be a string or null",
        )),
    }
}

fn form_edit_validate_element_event_payload_types(
    object: &Map<String, Value>,
    kind: FormEditElementDefinitionKind,
    element_name: &str,
) -> Result<(), String> {
    let handlers = match object.get("handlers") {
        None | Some(Value::Null) => None,
        Some(Value::Object(values)) => Some(values),
        Some(_) => {
            return Err(form_edit_event_diagnostic(
                FormEventDiagnosticCode::EventNotAllowed,
                format!("new element '{element_name}'"),
                "<definition>",
                "handlers must be an object",
            ));
        }
    };
    let Some(events_value) = object.get("on") else {
        return Ok(());
    };
    let events = events_value.as_array().ok_or_else(|| {
        form_edit_event_diagnostic(
            FormEventDiagnosticCode::EventNotAllowed,
            format!("new element '{element_name}'"),
            "<definition>",
            "on must be an array",
        )
    })?;
    let target = format!("new element '{element_name}'");
    if kind == FormEditElementDefinitionKind::Table
        && !events.is_empty()
        && object
            .get("path")
            .and_then(Value::as_str)
            .is_none_or(|path| path.trim().is_empty())
    {
        return Err(form_edit_event_diagnostic(
            FormEventDiagnosticCode::EventNotAllowed,
            &target,
            "<definition>",
            "Table event bindings require a non-empty path/DataPath; the platform drops bindings on unbound tables",
        ));
    }
    for value in events {
        if let Some(event_name) = value.as_str() {
            form_edit_optional_event_handler(
                handlers.and_then(|values| values.get(event_name)),
                &target,
                event_name,
            )?;
            continue;
        }
        let event = value.as_object().ok_or_else(|| {
            form_edit_event_diagnostic(
                FormEventDiagnosticCode::EventNotAllowed,
                &target,
                "<definition>",
                "on items must be strings or objects",
            )
        })?;
        let event_name = event
            .get("event")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                form_edit_event_diagnostic(
                    FormEventDiagnosticCode::EventNotAllowed,
                    &target,
                    "<definition>",
                    "event must be a non-empty string",
                )
            })?;
        if form_edit_optional_event_handler(event.get("handler"), &target, event_name)?.is_none() {
            form_edit_optional_event_handler(
                handlers.and_then(|values| values.get(event_name)),
                &target,
                event_name,
            )?;
        }
    }
    Ok(())
}

pub(crate) fn form_edit_validate_new_element_event_tree(
    elements: &[Value],
    context: &FormEventContext,
) -> Result<(), String> {
    for element in elements {
        let Some(object) = element.as_object() else {
            continue;
        };
        let definition_kind = FormEditElementDefinitionKind::from_object(object)?;
        let element_name = definition_kind.name(object)?;
        form_edit_validate_element_event_payload_types(object, definition_kind, element_name)?;
        let kind = definition_kind.event_kind();
        let handlers = match object.get("handlers") {
            None | Some(Value::Null) => None,
            Some(Value::Object(values)) => Some(values),
            Some(_) => {
                return Err(form_edit_event_diagnostic(
                    FormEventDiagnosticCode::EventNotAllowed,
                    "new element",
                    "<definition>",
                    "handlers must be an object",
                ));
            }
        };
        if let Some(events_value) = object.get("on") {
            let events = events_value.as_array().ok_or_else(|| {
                form_edit_event_diagnostic(
                    FormEventDiagnosticCode::EventNotAllowed,
                    "new element",
                    "<definition>",
                    "on must be an array",
                )
            })?;
            let element_name = element_name.to_string();
            let mut seen = HashSet::<String>::new();
            for value in events {
                let (name, handler, call_type) = if let Some(name) = value.as_str() {
                    let target = format!("new element '{element_name}'");
                    let handler = form_edit_optional_event_handler(
                        handlers.and_then(|values| values.get(name)),
                        &target,
                        name,
                    )?
                    .unwrap_or_else(|| form_event_handler_name(&element_name, name));
                    (name.to_string(), handler, None)
                } else {
                    let object = value.as_object().ok_or_else(|| {
                        form_edit_event_diagnostic(
                            FormEventDiagnosticCode::EventNotAllowed,
                            format!("new element '{element_name}'"),
                            "<definition>",
                            "on items must be strings or objects",
                        )
                    })?;
                    let name = object
                        .get("event")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| {
                            form_edit_event_diagnostic(
                                FormEventDiagnosticCode::EventNotAllowed,
                                format!("new element '{element_name}'"),
                                "<definition>",
                                "event must be a non-empty string",
                            )
                        })?
                        .to_string();
                    let target = format!("new element '{element_name}'");
                    let handler = match form_edit_optional_event_handler(
                        object.get("handler"),
                        &target,
                        &name,
                    )? {
                        Some(handler) => Some(handler),
                        None => form_edit_optional_event_handler(
                            handlers.and_then(|values| values.get(&name)),
                            &target,
                            &name,
                        )?,
                    }
                    .unwrap_or_else(|| form_event_handler_name(&element_name, &name));
                    let call_type = match object.get("callType") {
                        None | Some(Value::Null) => None,
                        Some(Value::String(value)) => Some(value.clone()),
                        Some(_) => {
                            return Err(form_edit_event_diagnostic(
                                FormEventDiagnosticCode::InvalidCallType,
                                format!("new element '{element_name}'"),
                                &name,
                                "callType must be a string",
                            ));
                        }
                    };
                    (name, handler, call_type)
                };
                if !seen.insert(name.clone()) {
                    return Err(form_edit_event_diagnostic(
                        FormEventDiagnosticCode::Duplicate,
                        format!("new element '{element_name}'"),
                        &name,
                        "the on array contains the same event more than once",
                    ));
                }
                let binding = if let Some(call_type) = call_type.as_deref() {
                    FormEventBinding::new(&name, &handler).with_call_type(call_type)
                } else {
                    FormEventBinding::new(&name, &handler)
                };
                validate_event(context, FormEventTarget::Element(kind), &binding)
                    .map_err(|diagnostic| diagnostic.to_string())?;
            }
        }
        for child_key in ["children", "columns"] {
            if let Some(children) = object.get(child_key).and_then(Value::as_array) {
                form_edit_validate_new_element_event_tree(children, context)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn form_edit_event_owner_label(owner: &FormEditEventOwner) -> String {
    match owner {
        FormEditEventOwner::Form => "form".to_string(),
        FormEditEventOwner::Element(name) => format!("element '{name}'"),
    }
}

pub(crate) fn form_edit_event_diagnostic(
    code: FormEventDiagnosticCode,
    target: impl Into<String>,
    event: impl Into<String>,
    detail: impl Into<String>,
) -> String {
    FormEventDiagnostic::new(code, target, event)
        .with_detail(detail)
        .to_string()
}

pub(crate) fn form_edit_find_element_nodes<'a, 'input>(
    root: roxmltree::Node<'a, 'input>,
    name: &str,
) -> Vec<roxmltree::Node<'a, 'input>> {
    let auto_command_bar = root.children().find(|child| {
        child.is_element()
            && child.tag_name().namespace() == Some(FORM_LOGFORM_NS)
            && child.tag_name().name() == "AutoCommandBar"
    });
    let mut matches = auto_command_bar
        .filter(|node| node.attribute("name") == Some(name))
        .into_iter()
        .collect::<Vec<_>>();

    let mut containers = root
        .children()
        .filter(|child| {
            child.is_element()
                && child.tag_name().namespace() == Some(FORM_LOGFORM_NS)
                && child.tag_name().name() == "ChildItems"
        })
        .collect::<Vec<_>>();
    if let Some(child_items) = auto_command_bar.and_then(|node| {
        node.children().find(|child| {
            child.is_element()
                && child.tag_name().namespace() == Some(FORM_LOGFORM_NS)
                && child.tag_name().name() == "ChildItems"
        })
    }) {
        containers.push(child_items);
    }
    for container in containers {
        matches.extend(container.descendants().filter(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(FORM_LOGFORM_NS)
                && node.attribute("name") == Some(name)
                && FormElementKind::from_xml_tag(node.tag_name().name()).is_some()
        }));
    }
    matches
}

pub(crate) fn form_edit_resolve_element_node<'a, 'input>(
    root: roxmltree::Node<'a, 'input>,
    name: &str,
    event_name: &str,
) -> Result<roxmltree::Node<'a, 'input>, String> {
    let matches = form_edit_find_element_nodes(root, name);
    match matches.as_slice() {
        [] => Err(form_edit_event_diagnostic(
            FormEventDiagnosticCode::TargetNotFound,
            format!("element '{name}'"),
            event_name,
            "target element was not found in Form ChildItems or AutoCommandBar",
        )),
        [node] => Ok(*node),
        _ => Err(form_edit_event_diagnostic(
            FormEventDiagnosticCode::Duplicate,
            format!("element '{name}'"),
            event_name,
            format!(
                "target name is ambiguous: the source form contains {} matching elements",
                matches.len()
            ),
        )),
    }
}

pub(crate) fn form_edit_apply_planned_event(
    xml_text: &mut String,
    event: &FormEditPlannedEvent,
) -> Result<(), String> {
    enum InsertTarget {
        ExistingEvents(Range<usize>),
        AfterRootAutoCommandBar {
            pos: usize,
            indent: String,
        },
        IntoElement {
            range: Range<usize>,
            tag: String,
            indent: String,
        },
        BeforeChildItems {
            pos: usize,
            indent: String,
        },
    }

    let target = {
        let document = Document::parse(xml_text)
            .map_err(|err| format!("[ERROR] XML parse error while applying event: {err}"))?;
        let root = document.root_element();
        let owner_node = match &event.owner {
            FormEditEventOwner::Form => root,
            FormEditEventOwner::Element(name) => {
                form_edit_resolve_element_node(root, name, &event.name)?
            }
        };
        if let Some(events) = form_validation_child(owner_node, "Events") {
            InsertTarget::ExistingEvents(events.range())
        } else {
            match &event.owner {
                FormEditEventOwner::Form => {
                    let auto_command_bar = form_validation_child(root, "AutoCommandBar")
                        .ok_or_else(|| {
                            form_edit_event_diagnostic(
                                FormEventDiagnosticCode::TargetNotFound,
                                "form",
                                &event.name,
                                "AutoCommandBar is required to position the Events section",
                            )
                        })?;
                    InsertTarget::AfterRootAutoCommandBar {
                        pos: auto_command_bar.range().end,
                        indent: form_edit_whitespace_indent_at(
                            xml_text,
                            auto_command_bar.range().start,
                        ),
                    }
                }
                FormEditEventOwner::Element(_) => {
                    match form_validation_child(owner_node, "ChildItems")
                        .filter(|_| matches!(owner_node.tag_name().name(), "Pages" | "Table"))
                    {
                        Some(child_items) => {
                            let indent =
                                form_edit_whitespace_indent_at(xml_text, child_items.range().start);
                            InsertTarget::BeforeChildItems {
                                pos: child_items.range().start.saturating_sub(indent.len()),
                                indent,
                            }
                        }
                        None => InsertTarget::IntoElement {
                            range: owner_node.range(),
                            tag: owner_node.tag_name().name().to_string(),
                            indent: form_edit_whitespace_indent_at(
                                xml_text,
                                owner_node.range().start,
                            ),
                        },
                    }
                }
            }
        }
    };

    let call_type = event
        .call_type
        .as_deref()
        .map(|value| format!(" callType=\"{}\"", escape_xml(value)))
        .unwrap_or_default();
    let event_xml = format!(
        "<Event name=\"{}\"{}>{}</Event>",
        escape_xml(&event.name),
        call_type,
        escape_xml(&event.handler)
    );
    let eol = form_edit_eol(xml_text);
    match target {
        InsertTarget::ExistingEvents(range) => {
            form_edit_insert_event_into_events(xml_text, range, &event_xml, eol)
        }
        InsertTarget::AfterRootAutoCommandBar { pos, indent } => {
            let event_indent = format!("{indent}\t");
            let block =
                format!("{indent}<Events>{eol}{event_indent}{event_xml}{eol}{indent}</Events>");
            let suffix_has_eol = xml_text[pos..].starts_with(eol);
            let after = if suffix_has_eol {
                String::new()
            } else {
                format!("{eol}{indent}")
            };
            xml_text.insert_str(pos, &format!("{eol}{block}{after}"));
            Ok(())
        }
        InsertTarget::IntoElement { range, tag, indent } => {
            form_edit_insert_events_into_element(xml_text, range, &tag, &indent, &event_xml, eol)
        }
        InsertTarget::BeforeChildItems { pos, indent } => {
            let event_indent = format!("{indent}\t");
            let block = format!(
                "{indent}<Events>{eol}{event_indent}{event_xml}{eol}{indent}</Events>{eol}"
            );
            xml_text.insert_str(pos, &block);
            Ok(())
        }
    }
}

pub(crate) fn form_edit_insert_event_into_events(
    xml_text: &mut String,
    range: Range<usize>,
    event_xml: &str,
    eol: &str,
) -> Result<(), String> {
    let section = &xml_text[range.clone()];
    let events_indent = form_edit_whitespace_indent_at(xml_text, range.start);
    let event_indent = format!("{events_indent}\t");
    if section.trim_end().ends_with("/>") {
        let relative_close = section
            .rfind("/>")
            .ok_or_else(|| "Self-closing Events section has no '/>' terminator".to_string())?;
        let opening = section[..relative_close].trim_end();
        let replacement =
            format!("{opening}>{eol}{event_indent}{event_xml}{eol}{events_indent}</Events>");
        xml_text.replace_range(range, &replacement);
        return Ok(());
    }

    let relative_close = section
        .rfind("</Events>")
        .ok_or_else(|| "No closing </Events> found in form event section".to_string())?;
    let close_pos = range.start + relative_close;
    let line_start = xml_text[..close_pos]
        .rfind('\n')
        .map(|position| position + 1)
        .unwrap_or(close_pos);
    if xml_text[line_start..close_pos].trim().is_empty() {
        xml_text.insert_str(line_start, &format!("{event_indent}{event_xml}{eol}"));
    } else {
        xml_text.insert_str(
            close_pos,
            &format!("{eol}{event_indent}{event_xml}{eol}{events_indent}"),
        );
    }
    Ok(())
}

pub(crate) fn form_edit_insert_events_into_element(
    xml_text: &mut String,
    range: Range<usize>,
    tag: &str,
    element_indent: &str,
    event_xml: &str,
    eol: &str,
) -> Result<(), String> {
    let element = &xml_text[range.clone()];
    let section_indent = format!("{element_indent}\t");
    let event_indent = format!("{section_indent}\t");
    let block = format!(
        "{section_indent}<Events>{eol}{event_indent}{event_xml}{eol}{section_indent}</Events>"
    );
    if element.trim_end().ends_with("/>") {
        let relative_close = element
            .rfind("/>")
            .ok_or_else(|| "Self-closing form element has no '/>' terminator".to_string())?;
        let opening = element[..relative_close].trim_end();
        let replacement = format!("{opening}>{eol}{block}{eol}{element_indent}</{tag}>");
        xml_text.replace_range(range, &replacement);
        return Ok(());
    }

    let close = format!("</{tag}>");
    let relative_close = element
        .rfind(&close)
        .ok_or_else(|| format!("No closing {close} found in element target"))?;
    let close_pos = range.start + relative_close;
    let line_start = xml_text[..close_pos]
        .rfind('\n')
        .map(|position| position + 1)
        .unwrap_or(close_pos);
    if xml_text[line_start..close_pos].trim().is_empty() {
        xml_text.insert_str(line_start, &format!("{block}{eol}"));
    } else {
        xml_text.insert_str(close_pos, &format!("{eol}{block}{eol}{element_indent}"));
    }
    Ok(())
}

pub(crate) fn form_edit_eol(xml_text: &str) -> &'static str {
    if xml_text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

pub(crate) fn form_edit_whitespace_indent_at(xml_text: &str, pos: usize) -> String {
    let line_start = xml_text[..pos]
        .rfind('\n')
        .map(|position| position + 1)
        .unwrap_or(0);
    xml_text[line_start..pos]
        .chars()
        .take_while(|character| matches!(character, ' ' | '\t' | '\r'))
        .filter(|character| *character != '\r')
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Utf8Bom {
    Present,
    Absent,
}

pub(crate) fn form_edit_write_preserving_bom(
    path: &Path,
    xml_text: &str,
    bom: Utf8Bom,
) -> Result<(), String> {
    let bom_len = if bom == Utf8Bom::Present { 3 } else { 0 };
    let mut bytes = Vec::with_capacity(xml_text.len() + bom_len);
    if bom == Utf8Bom::Present {
        bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
    }
    bytes.extend_from_slice(xml_text.as_bytes());
    super::common::atomic_replace(path, &bytes)
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

pub(crate) fn form_edit_is_extension_form(xml_text: &str) -> bool {
    Document::parse(xml_text).ok().is_some_and(|doc| {
        doc.descendants()
            .any(|node| node.is_element() && node.tag_name().name() == "BaseForm")
    })
}

pub(crate) fn form_edit_validate_element_names(
    xml_text: &str,
    elements: &[Value],
) -> Result<(), String> {
    let mut names = HashSet::new();
    for element in elements {
        form_edit_validate_element_name_tree(xml_text, element, &mut names)?;
    }
    Ok(())
}

pub(crate) fn form_edit_validate_element_name_tree(
    xml_text: &str,
    element: &Value,
    names: &mut HashSet<String>,
) -> Result<(), String> {
    if let Some(name) = form_edit_element_display_name(element) {
        if !names.insert(name.clone()) {
            return Err(format!(
                "[ERROR] Element '{name}' already exists in edit definition -- element names must be unique"
            ));
        }
        if form_edit_element_name_exists(xml_text, &name) {
            return Err(format!(
                "[ERROR] Element '{name}' already exists in form -- element names must be unique"
            ));
        }
    }
    let Some(object) = element.as_object() else {
        return Ok(());
    };
    for key in ["children", "columns"] {
        if let Some(children) = object.get(key).and_then(Value::as_array) {
            for child in children {
                form_edit_validate_element_name_tree(xml_text, child, names)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn form_edit_validate_named_objects(
    xml_text: &str,
    values: &[Value],
    tag: &str,
    label: &str,
) -> Result<(), String> {
    let mut names = HashSet::new();
    for value in values {
        let Some(name) = value
            .as_object()
            .and_then(|object| object.get("name"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        if name.is_empty() {
            return Err(format!(
                "[ERROR] Empty {label} name in edit definition -- names must be non-empty"
            ));
        }
        if !names.insert(name.to_string()) {
            return Err(format!(
                "[ERROR] Duplicate {label} name '{name}' in edit definition -- names must be unique"
            ));
        }
        if form_edit_name_exists(xml_text, tag, name) {
            return Err(format!(
                "[ERROR] {tag} '{name}' already exists in form -- {label} names must be unique"
            ));
        }
    }
    Ok(())
}

pub(crate) fn form_edit_validate_attribute_columns(attrs: &[Value]) -> Result<(), String> {
    for attr in attrs {
        let Some(object) = attr.as_object() else {
            continue;
        };
        let Some(attr_name) = object.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(columns_value) = object.get("columns") else {
            continue;
        };
        let columns = columns_value
            .as_array()
            .ok_or_else(|| format!("[ERROR] Attribute '{attr_name}' columns must be an array"))?;
        let attr_type = object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !matches!(attr_type, "ValueTable" | "ValueTree") {
            return Err(format!(
                "[ERROR] Attribute '{attr_name}' of type '{attr_type}' cannot define columns; columns are supported only for ValueTable or ValueTree"
            ));
        }
        let mut names = HashSet::new();
        for (index, column) in columns.iter().enumerate() {
            let column = column.as_object().ok_or_else(|| {
                format!(
                    "[ERROR] Attribute '{attr_name}' column #{} must be an object",
                    index + 1
                )
            })?;
            let name = column
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| {
                    format!(
                        "[ERROR] Attribute '{attr_name}' column #{} requires non-empty name",
                        index + 1
                    )
                })?;
            column
                .get("type")
                .and_then(Value::as_str)
                .filter(|column_type| !column_type.trim().is_empty())
                .ok_or_else(|| {
                    format!(
                        "[ERROR] Attribute '{attr_name}' column '{name}' requires non-empty type"
                    )
                })?;
            if !names.insert(name.to_string()) {
                return Err(format!(
                    "[ERROR] Duplicate column name '{name}' in attribute '{attr_name}' edit definition -- column names must be unique"
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn form_edit_name_exists(xml_text: &str, tag: &str, name: &str) -> bool {
    let Ok(doc) = Document::parse(xml_text) else {
        return false;
    };
    doc.descendants().any(|node| {
        node.is_element() && node.tag_name().name() == tag && node.attribute("name") == Some(name)
    })
}

pub(crate) fn form_edit_element_name_exists(xml_text: &str, name: &str) -> bool {
    let Ok(doc) = Document::parse(xml_text) else {
        return false;
    };
    doc.descendants().any(|node| {
        node.is_element()
            && FormElementKind::from_xml_tag(node.tag_name().name()).is_some()
            && node.attribute("name") == Some(name)
    })
}

pub(crate) fn form_edit_element_display_name(element: &Value) -> Option<String> {
    let object = element.as_object()?;
    let kind = FormEditElementDefinitionKind::from_object(object).ok()?;
    kind.name(object).ok().map(ToOwned::to_owned)
}

pub(crate) fn form_edit_element_summary(element: &Value) -> Option<String> {
    let object = element.as_object()?;
    let kind = FormEditElementDefinitionKind::from_object(object).ok()?;
    let tag = match kind {
        FormEditElementDefinitionKind::Table => "Table",
        FormEditElementDefinitionKind::LabelField => "LabelField",
        FormEditElementDefinitionKind::Button => "Button",
        FormEditElementDefinitionKind::CommandBar => "CommandBar",
        FormEditElementDefinitionKind::Pages => "Pages",
        FormEditElementDefinitionKind::Page => "Page",
        FormEditElementDefinitionKind::Group => "Group",
        FormEditElementDefinitionKind::CheckBox => "CheckBox",
        FormEditElementDefinitionKind::InputField => "Input",
        FormEditElementDefinitionKind::AutoCommandBar => return None,
    };
    let name = form_edit_element_display_name(element)?;
    let path = object
        .get("path")
        .and_then(Value::as_str)
        .map(|value| format!(" -> {value}"))
        .unwrap_or_default();
    let events = form_edit_element_events_summary(object);
    Some(format!("  + [{tag}] {name}{path}{events}"))
}

pub(crate) fn form_edit_element_events_summary(element: &Map<String, Value>) -> String {
    let Some(events) = element.get("on").and_then(Value::as_array) else {
        return String::new();
    };
    if events.is_empty() {
        return String::new();
    }
    let names = events
        .iter()
        .map(|event| {
            event
                .as_str()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| form_edit_python_repr(event))
        })
        .collect::<Vec<_>>();
    format!(" {{{}}}", names.join(", "))
}

pub(crate) fn form_edit_python_repr(value: &Value) -> String {
    match value {
        Value::String(value) => format!("'{}'", form_edit_python_repr_string(value)),
        Value::Bool(value) => {
            if *value {
                "True".to_string()
            } else {
                "False".to_string()
            }
        }
        Value::Number(value) => value.to_string(),
        Value::Null => "None".to_string(),
        Value::Array(values) => {
            let items = values.iter().map(form_edit_python_repr).collect::<Vec<_>>();
            format!("[{}]", items.join(", "))
        }
        Value::Object(object) => {
            let items = object
                .iter()
                .map(|(key, value)| {
                    format!(
                        "'{}': {}",
                        form_edit_python_repr_string(key),
                        form_edit_python_repr(value)
                    )
                })
                .collect::<Vec<_>>();
            format!("{{{}}}", items.join(", "))
        }
    }
}

pub(crate) fn form_edit_python_repr_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
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
    let Some(pos) = form_edit_find_section_close(xml_text, section) else {
        return Err(format!("No <{section}> section found in form"));
    };
    let insert_pos = xml_text[..pos]
        .rfind('\n')
        .map(|idx| idx + 1)
        .unwrap_or(pos);
    xml_text.insert_str(insert_pos, &format!("{content}\n"));
    Ok(())
}

pub(crate) enum FormEditInsertTarget {
    ExistingChildItems {
        range: std::ops::Range<usize>,
        child_indent: String,
    },
    ElementNeedsChildItems {
        range: std::ops::Range<usize>,
        tag: String,
        element_indent: String,
        child_items_indent: String,
        child_indent: String,
    },
    AfterElement {
        pos: usize,
        child_indent: String,
    },
}

impl FormEditInsertTarget {
    pub(crate) fn child_indent(&self) -> &str {
        match self {
            Self::ExistingChildItems { child_indent, .. }
            | Self::ElementNeedsChildItems { child_indent, .. }
            | Self::AfterElement { child_indent, .. } => child_indent,
        }
    }
}

pub(crate) fn form_edit_target_child_items_range(
    xml_text: &str,
    into_name: Option<&str>,
    after_name: Option<&str>,
) -> Result<FormEditInsertTarget, String> {
    let doc = Document::parse(xml_text).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
    let root = doc.root_element();
    let root_child_items = form_child(root, "ChildItems");
    if let Some(into_name) = into_name.filter(|name| !name.is_empty()) {
        let Some(target) =
            root_child_items.and_then(|child_items| form_edit_find_element(child_items, into_name))
        else {
            return Err(format!("[ERROR] Target group '{into_name}' not found"));
        };
        if let Some(child_items) = form_child(target, "ChildItems") {
            return Ok(FormEditInsertTarget::ExistingChildItems {
                child_indent: form_edit_child_indent_for_section(xml_text, child_items.range()),
                range: child_items.range(),
            });
        }
        let element_indent = form_edit_line_indent_at(xml_text, target.range().start);
        let child_items_indent = format!("{element_indent}\t");
        let child_indent = format!("{child_items_indent}\t");
        return Ok(FormEditInsertTarget::ElementNeedsChildItems {
            range: target.range(),
            tag: target.tag_name().name().to_string(),
            element_indent,
            child_items_indent,
            child_indent,
        });
    }
    if let Some(after_name) = after_name.filter(|name| !name.is_empty()) {
        let Some(after_element) = root_child_items
            .and_then(|child_items| form_edit_find_element(child_items, after_name))
        else {
            return Err(format!("[ERROR] Element '{after_name}' not found"));
        };
        let child_items = after_element
            .ancestors()
            .find(|node| node.is_element() && node.tag_name().name() == "ChildItems")
            .ok_or_else(|| {
                format!("No parent <ChildItems> section found for form element '{after_name}'")
            })?;
        return Ok(FormEditInsertTarget::AfterElement {
            child_indent: form_edit_child_indent_for_section(xml_text, child_items.range()),
            pos: after_element.range().end,
        });
    }
    let Some(child_items) = root_child_items else {
        return Err("No <ChildItems> section found in form".to_string());
    };
    Ok(FormEditInsertTarget::ExistingChildItems {
        child_indent: form_edit_child_indent_for_section(xml_text, child_items.range()),
        range: child_items.range(),
    })
}

pub(crate) fn form_edit_find_element<'a>(
    child_items: roxmltree::Node<'a, 'a>,
    name: &str,
) -> Option<roxmltree::Node<'a, 'a>> {
    for child in child_items.children().filter(|child| child.is_element()) {
        if child.attribute("name") == Some(name) {
            return Some(child);
        }
        if let Some(nested_child_items) = form_child(child, "ChildItems") {
            if let Some(found) = form_edit_find_element(nested_child_items, name) {
                return Some(found);
            }
        }
    }
    None
}

pub(crate) fn form_edit_insert_lines_into_target(
    xml_text: &mut String,
    target: FormEditInsertTarget,
    lines: &[String],
) -> Result<(), String> {
    if lines.is_empty() {
        return Ok(());
    }
    match target {
        FormEditInsertTarget::ExistingChildItems { range, .. } => {
            form_edit_insert_lines_into_range(xml_text, range, "ChildItems", lines)
        }
        FormEditInsertTarget::ElementNeedsChildItems {
            range,
            tag,
            element_indent,
            child_items_indent,
            ..
        } => form_edit_insert_child_items_into_element(
            xml_text,
            range,
            &tag,
            &element_indent,
            &child_items_indent,
            lines,
        ),
        FormEditInsertTarget::AfterElement { pos, .. } => {
            let content = lines.join("\n");
            xml_text.insert_str(pos, &format!("\n{content}"));
            Ok(())
        }
    }
}

pub(crate) fn form_edit_insert_lines_into_range(
    xml_text: &mut String,
    range: std::ops::Range<usize>,
    section: &str,
    lines: &[String],
) -> Result<(), String> {
    let content = lines.join("\n");
    let child_indent = form_edit_line_indent(lines.first().map(String::as_str).unwrap_or(""));
    let parent_indent = form_edit_parent_indent(&child_indent);
    let section_text = &xml_text[range.clone()];
    if section_text.trim_end().ends_with("/>") {
        xml_text.replace_range(
            range,
            &format!("<{section}>\n{content}\n{parent_indent}</{section}>"),
        );
        return Ok(());
    }
    let close = format!("</{section}>");
    let Some(relative_pos) = section_text.rfind(&close) else {
        return Err(format!("No <{section}> section found in form target"));
    };
    let insert_pos = section_text[..relative_pos]
        .rfind('\n')
        .map(|idx| range.start + idx + 1)
        .unwrap_or(range.start + relative_pos);
    xml_text.insert_str(insert_pos, &format!("{content}\n"));
    Ok(())
}

pub(crate) fn form_edit_line_indent(line: &str) -> String {
    line.chars().take_while(|ch| *ch == '\t').collect()
}

pub(crate) fn form_edit_line_indent_at(xml_text: &str, pos: usize) -> String {
    let line_start = xml_text[..pos].rfind('\n').map(|idx| idx + 1).unwrap_or(0);
    form_edit_line_indent(&xml_text[line_start..pos])
}

pub(crate) fn form_edit_parent_indent(child_indent: &str) -> String {
    child_indent
        .strip_suffix('\t')
        .unwrap_or(child_indent)
        .to_string()
}

pub(crate) fn form_edit_child_indent_for_section(
    xml_text: &str,
    range: std::ops::Range<usize>,
) -> String {
    let section_text = &xml_text[range.clone()];
    let open_end = section_text.find('>').map(|idx| idx + 1).unwrap_or(0);
    if let Some(close_pos) = section_text.rfind("</ChildItems>") {
        let body = &section_text[open_end..close_pos];
        if let Some(indent) = form_edit_first_element_indent(body) {
            return indent;
        }
        if let Some(parent_indent) = form_edit_trailing_tab_indent(&section_text[..close_pos]) {
            return format!("{parent_indent}\t");
        }
    }
    format!("{}\t", form_edit_line_indent_at(xml_text, range.start))
}

pub(crate) fn form_edit_first_element_indent(text: &str) -> Option<String> {
    for (idx, _) in text.match_indices('<') {
        if text[idx..].starts_with("</") {
            continue;
        }
        if let Some(indent) = form_edit_trailing_tab_indent(&text[..idx]) {
            return Some(indent);
        }
    }
    None
}

pub(crate) fn form_edit_trailing_tab_indent(text: &str) -> Option<String> {
    let line = text.rsplit('\n').next()?;
    if line.chars().all(|ch| ch == '\t') {
        Some(line.to_string())
    } else {
        None
    }
}

pub(crate) fn form_edit_insert_child_items_into_element(
    xml_text: &mut String,
    range: std::ops::Range<usize>,
    tag: &str,
    element_indent: &str,
    child_items_indent: &str,
    lines: &[String],
) -> Result<(), String> {
    let content = lines.join("\n");
    let element_text = &xml_text[range.clone()];
    let open_tag_end = form_edit_opening_tag_end(element_text, 0)
        .ok_or_else(|| format!("No opening <{tag}> tag found in form target"))?;
    let opening_tag = &element_text[..=open_tag_end];
    if opening_tag.trim_end().ends_with("/>") {
        let relative_pos = opening_tag
            .rfind("/>")
            .ok_or_else(|| format!("Self-closing <{tag}> tag has no '/>' terminator"))?;
        let pos = range.start + relative_pos;
        xml_text.replace_range(
            pos..pos + 2,
            &format!(
                ">\n{child_items_indent}<ChildItems>\n{content}\n{child_items_indent}</ChildItems>\n{element_indent}</{tag}>"
            ),
        );
        return Ok(());
    }
    let close = format!("</{tag}>");
    let Some(relative_pos) = element_text.rfind(&close) else {
        return Err(format!("No closing </{tag}> found in form target"));
    };
    let insert_pos = element_text[..relative_pos]
        .rfind('\n')
        .map(|idx| range.start + idx + 1)
        .unwrap_or(range.start + relative_pos);
    xml_text.insert_str(
        insert_pos,
        &format!(
            "{child_items_indent}<ChildItems>\n{content}\n{child_items_indent}</ChildItems>\n"
        ),
    );
    Ok(())
}

pub(crate) fn form_edit_opening_tag_end(text: &str, start: usize) -> Option<usize> {
    let mut quote = None::<char>;
    for (relative_idx, ch) in text[start..].char_indices() {
        if let Some(quote_ch) = quote {
            if ch == quote_ch {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '>' => return Some(start + relative_idx),
            _ => {}
        }
    }
    None
}

pub(crate) fn form_edit_ensure_emitted_namespaces(
    xml_text: &mut String,
    root_start: usize,
    emitted_fragments: &str,
) -> Result<(), String> {
    if emitted_fragments.is_empty() {
        return Ok(());
    }
    let root_open_end = form_edit_opening_tag_end(xml_text, root_start)
        .ok_or_else(|| "No opening <Form> tag found in form".to_string())?;
    let additions = {
        let root_opening = &xml_text[root_start..=root_open_end];
        let mut additions = String::new();
        for (prefix, uri) in form_edit_emitter_namespaces() {
            let needed = emitted_fragments.contains(&format!("{prefix}:"));
            if needed && !form_edit_opening_tag_declares_namespace(root_opening, prefix) {
                additions.push_str(&format!(" xmlns:{prefix}=\"{uri}\""));
            }
        }
        additions
    };
    if !additions.is_empty() {
        xml_text.insert_str(root_open_end, &additions);
    }
    Ok(())
}

pub(crate) fn form_edit_emitter_namespaces() -> [(&'static str, &'static str); 6] {
    [
        ("app", "http://v8.1c.ru/8.2/managed-application/core"),
        ("cfg", "http://v8.1c.ru/8.1/data/enterprise/current-config"),
        ("v8", FORM_V8_NS),
        ("xr", "http://v8.1c.ru/8.3/xcf/readable"),
        ("xs", "http://www.w3.org/2001/XMLSchema"),
        ("xsi", "http://www.w3.org/2001/XMLSchema-instance"),
    ]
}

pub(crate) fn form_edit_opening_tag_declares_namespace(opening: &str, prefix: &str) -> bool {
    let needle = format!("xmlns:{prefix}");
    let mut search_start = 0usize;
    while let Some(relative_start) = opening[search_start..].find(&needle) {
        let start = search_start + relative_start + needle.len();
        let remainder = opening[start..].trim_start();
        if remainder.starts_with('=') {
            return true;
        }
        search_start = start;
    }
    false
}

pub(crate) fn form_edit_find_section_close(xml_text: &str, section: &str) -> Option<usize> {
    let open = format!("<{section}");
    let close = format!("</{section}>");
    let mut offset = 0usize;
    let mut depth = 0usize;
    let mut started = false;

    loop {
        let next_open = xml_text[offset..].find(&open).map(|idx| offset + idx);
        let next_close = xml_text[offset..].find(&close).map(|idx| offset + idx);
        let next = (match (next_open, next_close) {
            (Some(open_idx), Some(close_idx)) => {
                Some((open_idx.min(close_idx), open_idx <= close_idx))
            }
            (Some(open_idx), None) => Some((open_idx, true)),
            (None, Some(close_idx)) => Some((close_idx, false)),
            (None, None) => None,
        })?;

        let (idx, is_open) = next;
        if is_open {
            let after_name = idx + open.len();
            let next_char = xml_text[after_name..].chars().next()?;
            if !(next_char == '>' || next_char == '/' || next_char.is_whitespace()) {
                offset = after_name;
                continue;
            }
            let tag_end = xml_text[idx..].find('>').map(|end| idx + end)?;
            let tag = &xml_text[idx..=tag_end];
            started = true;
            if !tag.trim_end().ends_with("/>") {
                depth += 1;
            }
            offset = tag_end + 1;
        } else {
            if started {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(idx);
                }
            }
            offset = idx + close.len();
        }
    }
}

pub(crate) fn emit_form_edit_attribute_item(
    lines: &mut Vec<String>,
    attr: &Map<String, Value>,
    name: &str,
    id: usize,
    indent: &str,
) -> Result<(), String> {
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
    emit_form_attribute_columns(lines, attr.get("columns"), &inner)?;
    lines.push(format!("{indent}</Attribute>"));
    Ok(())
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

struct FormCompileEvent {
    name: String,
    handler: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormEditElementDefinitionKind {
    Table,
    LabelField,
    Button,
    CommandBar,
    Pages,
    Page,
    Group,
    CheckBox,
    InputField,
    AutoCommandBar,
}

impl FormEditElementDefinitionKind {
    fn from_object(object: &Map<String, Value>) -> Result<Self, String> {
        // `commandBar` is also a nested Table property, and `group` is also a
        // Page layout property. Prefer unambiguous primary discriminators before
        // considering those two standalone-element shorthands.
        let primary_candidates = [
            (Self::Table, "table", object.contains_key("table")),
            (
                Self::LabelField,
                "labelField",
                object.contains_key("labelField"),
            ),
            (Self::Button, "button", object.contains_key("button")),
            (Self::Pages, "pages", object.contains_key("pages")),
            (Self::Page, "page", object.contains_key("page")),
            (Self::CheckBox, "check", object.contains_key("check")),
            (Self::InputField, "input", object.contains_key("input")),
            (
                Self::AutoCommandBar,
                "autoCmdBar/autoCommandBar",
                object.contains_key("autoCmdBar") || object.contains_key("autoCommandBar"),
            ),
        ]
        .into_iter()
        .filter_map(|(kind, label, present)| present.then_some((kind, label)))
        .collect::<Vec<_>>();

        match primary_candidates.as_slice() {
            [(kind, _label)] => Ok(*kind),
            [] => {
                let command_bar =
                    object.contains_key("cmdBar") || object.contains_key("commandBar");
                let group = object.contains_key("group");
                match (command_bar, group, object.contains_key("name")) {
                    (true, false, _) => Ok(Self::CommandBar),
                    (false, true, _) => Ok(Self::Group),
                    (false, false, true) => Ok(Self::InputField),
                    (true, true, _) => Err(
                        "Form element has ambiguous commandBar and group discriminators"
                            .to_string(),
                    ),
                    (false, false, false) => Err(format!(
                        "Unsupported form element in native compiler: {}",
                        serde_json::to_string(object).unwrap_or_else(|_| "<invalid>".to_string())
                    )),
                }
            }
            multiple => Err(format!(
                "Form element must contain exactly one type discriminator; found: {}",
                multiple
                    .iter()
                    .map(|(_kind, label)| *label)
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    const fn event_kind(self) -> FormElementKind {
        match self {
            Self::Table => FormElementKind::Table,
            Self::LabelField => FormElementKind::LabelField,
            Self::Button => FormElementKind::Button,
            Self::CommandBar | Self::AutoCommandBar => FormElementKind::CommandBar,
            Self::Pages => FormElementKind::Pages,
            Self::Page => FormElementKind::Page,
            Self::Group => FormElementKind::Group,
            Self::CheckBox => FormElementKind::CheckBoxField,
            Self::InputField => FormElementKind::InputField,
        }
    }

    fn name(self, object: &Map<String, Value>) -> Result<&str, String> {
        let (keys, description): (&[&str], &str) = match self {
            Self::Table => (&["table"], "table"),
            Self::LabelField => (&["labelField"], "label field"),
            Self::Button => (&["button"], "button"),
            Self::CommandBar => (&["cmdBar", "commandBar"], "command bar"),
            Self::Pages => (&["pages"], "pages container"),
            Self::Page => (&["page"], "page"),
            Self::Group => (&["group"], "group"),
            Self::CheckBox => (&["check"], "checkbox"),
            Self::InputField => (&["input"], "input"),
            Self::AutoCommandBar => (&["autoCmdBar", "autoCommandBar"], "auto command bar"),
        };
        form_edit_definition_element_name(object, keys, description)
    }
}

fn form_edit_definition_element_name<'a>(
    object: &'a Map<String, Value>,
    keys: &[&str],
    description: &str,
) -> Result<&'a str, String> {
    object
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| {
            keys.iter()
                .find_map(|key| object.get(*key).and_then(Value::as_str))
        })
        .ok_or_else(|| format!("Form {description} is missing name"))
}

pub(crate) fn form_compile_xml(
    defn: &Value,
    format_version: &str,
) -> Result<(String, FormCompileStats), String> {
    let context = form_project_event_context(
        FormEventContext {
            definition: FormDefinitionKind::Regular,
            main_attribute: MainAttributeKind::Unknown,
            main_attribute_type: None,
            main_attribute_provenance: MainAttributeProvenance::Missing,
        },
        0,
        defn,
    );
    let events = form_compile_plan_events(defn, &context)?;

    if let Some(elements) = defn.get("elements").and_then(Value::as_array) {
        form_edit_validate_new_element_event_tree(elements, &context)?;
    }

    let mut ids = FormIdAllocator::new();
    let mut lines = Vec::<String>::new();
    lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
    lines.push(format!(
        "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:dcscor=\"http://v8.1c.ru/8.1/data-composition-system/core\" xmlns:dcssch=\"http://v8.1c.ru/8.1/data-composition-system/schema\" xmlns:dcsset=\"http://v8.1c.ru/8.1/data-composition-system/settings\" xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\" xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"{format_version}\">"
    ));

    let form_title = json_string_field(defn, "title").or_else(|| {
        defn.get("properties")
            .and_then(|props| json_string_field(props, "title"))
    });
    if let Some(title) = form_title.as_deref() {
        emit_form_mltext(&mut lines, "\t", "Title", title);
    }

    let props_src = defn.get("properties").and_then(Value::as_object);
    let mut props = Map::new();
    if form_title.is_some() && !props_src.is_some_and(|values| values.contains_key("autoTitle")) {
        props.insert("autoTitle".to_string(), Value::Bool(false));
    }
    if let Some(values) = props_src {
        for (key, value) in values {
            props.insert(key.clone(), value.clone());
        }
    }
    if !props.is_empty() {
        emit_form_properties(&mut lines, &props, "\t");
    }

    emit_form_auto_command_bar(&mut lines, defn, "\t");
    emit_form_events(&mut lines, &events, "\t");

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

fn form_compile_plan_events(
    defn: &Value,
    context: &FormEventContext,
) -> Result<Vec<FormCompileEvent>, String> {
    let Some(value) = defn.get("events") else {
        return Ok(Vec::new());
    };
    let events = value.as_object().ok_or_else(|| {
        FormEventDiagnostic::new(FormEventDiagnosticCode::EventNotAllowed, "form", "events")
            .with_detail("events must be an object mapping event names to string handlers")
            .to_string()
    })?;

    events
        .iter()
        .map(|(name, value)| {
            let handler = value.as_str().ok_or_else(|| {
                let (code, detail) = if value
                    .as_object()
                    .is_some_and(|event| event.contains_key("callType"))
                {
                    (
                        FormEventDiagnosticCode::EventNotAllowed,
                        "events map accepts only string handlers; callType is not supported",
                    )
                } else {
                    (
                        FormEventDiagnosticCode::EmptyHandler,
                        "event handler must be a string",
                    )
                };
                FormEventDiagnostic::new(code, "form", name)
                    .with_detail(detail)
                    .to_string()
            })?;
            let binding = FormEventBinding::new(name, handler);
            validate_event(context, FormEventTarget::Form, &binding)
                .map_err(|diagnostic| diagnostic.to_string())?;
            Ok(FormCompileEvent {
                name: name.clone(),
                handler: handler.to_string(),
            })
        })
        .collect()
}

fn emit_form_events(lines: &mut Vec<String>, events: &[FormCompileEvent], indent: &str) {
    if events.is_empty() {
        return;
    }

    lines.push(format!("{indent}<Events>"));
    for event in events {
        lines.push(format!(
            "{indent}\t<Event name=\"{}\">{}</Event>",
            escape_xml(&event.name),
            escape_xml(&event.handler)
        ));
    }
    lines.push(format!("{indent}</Events>"));
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
    let kind = FormEditElementDefinitionKind::from_object(object)?;
    form_edit_validate_element_event_payload_types(object, kind, kind.name(object)?)?;
    match kind {
        FormEditElementDefinitionKind::Table => {
            emit_form_table(lines, object, kind.name(object)?, indent, ids)
        }
        FormEditElementDefinitionKind::LabelField => {
            emit_form_label_field(lines, object, kind.name(object)?, indent, ids);
            Ok(())
        }
        FormEditElementDefinitionKind::Button => {
            emit_form_button(lines, object, kind.name(object)?, indent, ids);
            Ok(())
        }
        FormEditElementDefinitionKind::CommandBar => {
            emit_form_command_bar_element(lines, object, kind.name(object)?, indent, ids)
        }
        FormEditElementDefinitionKind::Pages => {
            emit_form_pages(lines, object, kind.name(object)?, indent, ids)
        }
        FormEditElementDefinitionKind::Page => {
            emit_form_page(lines, object, kind.name(object)?, indent, ids)
        }
        FormEditElementDefinitionKind::Group => {
            emit_form_group(lines, object, kind.name(object)?, indent, ids)
        }
        FormEditElementDefinitionKind::CheckBox => {
            emit_form_check(lines, object, kind.name(object)?, indent, ids);
            Ok(())
        }
        FormEditElementDefinitionKind::InputField => {
            emit_form_input(lines, object, kind.name(object)?, indent, ids);
            Ok(())
        }
        FormEditElementDefinitionKind::AutoCommandBar => Ok(()),
    }
}

pub(crate) fn emit_form_group(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
) -> Result<(), String> {
    let id = ids.next();
    lines.push(format!(
        "{indent}<UsualGroup name=\"{}\" id=\"{id}\">",
        escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if let Some(title) = element.get("title").and_then(Value::as_str) {
        emit_form_mltext(lines, &inner, "Title", title);
    }
    if let Some(value) = element
        .get("group")
        .and_then(Value::as_str)
        .and_then(form_compile_group_orientation)
    {
        lines.push(format!("{inner}<Group>{value}</Group>"));
    }
    if let Some(value) = element.get("behavior").and_then(Value::as_str) {
        if let Some(behavior) = form_compile_group_behavior(value) {
            lines.push(format!("{inner}<Behavior>{behavior}</Behavior>"));
        }
    } else if element.get("group").and_then(Value::as_str) == Some("collapsible") {
        lines.push(format!("{inner}<Behavior>Collapsible</Behavior>"));
    }
    if element.get("collapsed").and_then(Value::as_bool) == Some(true) {
        lines.push(format!("{inner}<Collapsed>true</Collapsed>"));
    }
    if let Some(value) = element.get("representation").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<Representation>{}</Representation>",
            form_compile_group_representation(value)
        ));
    }
    if let Some(value) = element.get("currentRowUse").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<CurrentRowUse>{}</CurrentRowUse>",
            escape_xml(value)
        ));
    }
    if let Some(value) = element.get("showTitle").and_then(Value::as_bool) {
        lines.push(format!(
            "{inner}<ShowTitle>{}</ShowTitle>",
            if value { "true" } else { "false" }
        ));
    }
    emit_form_common_flags(lines, element, &inner);
    emit_form_companion(
        lines,
        "ExtendedTooltip",
        &format!("{name}РасширеннаяПодсказка"),
        &inner,
        ids,
    );
    emit_form_children(lines, element, &inner, ids)?;
    lines.push(format!("{indent}</UsualGroup>"));
    Ok(())
}

pub(crate) fn emit_form_pages(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
) -> Result<(), String> {
    let id = ids.next();
    lines.push(format!(
        "{indent}<Pages name=\"{}\" id=\"{id}\">",
        escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if let Some(title) = element.get("title").and_then(Value::as_str) {
        emit_form_mltext(lines, &inner, "Title", title);
    }
    if let Some(value) = element.get("pagesRepresentation").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<PagesRepresentation>{}</PagesRepresentation>",
            escape_xml(value)
        ));
    }
    if let Some(value) = element.get("currentRowUse").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<CurrentRowUse>{}</CurrentRowUse>",
            escape_xml(value)
        ));
    }
    emit_form_common_flags(lines, element, &inner);
    emit_form_companion(
        lines,
        "ExtendedTooltip",
        &format!("{name}РасширеннаяПодсказка"),
        &inner,
        ids,
    );
    emit_form_element_events(lines, element, name, &inner);
    emit_form_children(lines, element, &inner, ids)?;
    lines.push(format!("{indent}</Pages>"));
    Ok(())
}

pub(crate) fn emit_form_page(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
) -> Result<(), String> {
    let id = ids.next();
    lines.push(format!(
        "{indent}<Page name=\"{}\" id=\"{id}\">",
        escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if let Some(title) = element.get("title").and_then(Value::as_str) {
        emit_form_mltext(lines, &inner, "Title", title);
    }
    emit_form_common_flags(lines, element, &inner);
    if let Some(value) = element
        .get("group")
        .and_then(Value::as_str)
        .and_then(form_compile_group_orientation)
    {
        lines.push(format!("{inner}<Group>{value}</Group>"));
    }
    if let Some(value) = element.get("showTitle").and_then(Value::as_bool) {
        lines.push(format!(
            "{inner}<ShowTitle>{}</ShowTitle>",
            if value { "true" } else { "false" }
        ));
    }
    emit_form_companion(
        lines,
        "ExtendedTooltip",
        &format!("{name}РасширеннаяПодсказка"),
        &inner,
        ids,
    );
    emit_form_children(lines, element, &inner, ids)?;
    lines.push(format!("{indent}</Page>"));
    Ok(())
}

pub(crate) fn emit_form_children(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    indent: &str,
    ids: &mut FormIdAllocator,
) -> Result<(), String> {
    let Some(children) = element.get("children").and_then(Value::as_array) else {
        return Ok(());
    };
    if children.is_empty() {
        return Ok(());
    }
    lines.push(format!("{indent}<ChildItems>"));
    for child in children {
        emit_form_element(lines, child, &format!("{indent}\t"), ids)?;
    }
    lines.push(format!("{indent}</ChildItems>"));
    Ok(())
}

pub(crate) fn form_compile_group_orientation(value: &str) -> Option<&'static str> {
    match value.to_lowercase().as_str() {
        "horizontal" => Some("Horizontal"),
        "vertical" | "collapsible" => Some("Vertical"),
        "alwayshorizontal" => Some("AlwaysHorizontal"),
        "alwaysvertical" => Some("AlwaysVertical"),
        "horizontalifpossible" => Some("HorizontalIfPossible"),
        _ => None,
    }
}

pub(crate) fn form_compile_group_behavior(value: &str) -> Option<&'static str> {
    match value.to_lowercase().as_str() {
        "usual" => Some("Usual"),
        "collapsible" => Some("Collapsible"),
        "popup" => Some("PopUp"),
        _ => None,
    }
}

pub(crate) fn form_compile_group_representation(value: &str) -> String {
    match value {
        "none" => "None".to_string(),
        "normal" => "NormalSeparation".to_string(),
        "weak" => "WeakSeparation".to_string(),
        "strong" => "StrongSeparation".to_string(),
        other => escape_xml(other),
    }
}

pub(crate) fn emit_form_check(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
) {
    let id = ids.next();
    lines.push(format!(
        "{indent}<CheckBoxField name=\"{}\" id=\"{id}\">",
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
    if let Some(value) = element.get("checkBoxType").and_then(Value::as_str) {
        if !value.is_empty() {
            let mapped = match value.to_lowercase().as_str() {
                "auto" => "Auto",
                "checkbox" => "CheckBox",
                "switcher" => "Switcher",
                "tumbler" => "Tumbler",
                _ => value,
            };
            lines.push(format!("{inner}<CheckBoxType>{mapped}</CheckBoxType>"));
        }
    } else {
        lines.push(format!("{inner}<CheckBoxType>Auto</CheckBoxType>"));
    }
    let title_location = element
        .get("titleLocation")
        .and_then(Value::as_str)
        .map(|value| match value {
            "none" => "None",
            "left" => "Left",
            "right" => "Right",
            "top" => "Top",
            "bottom" => "Bottom",
            "auto" => "Auto",
            other => other,
        })
        .unwrap_or("Right");
    lines.push(format!(
        "{inner}<TitleLocation>{title_location}</TitleLocation>"
    ));
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
    emit_form_element_events(lines, element, name, &inner);
    lines.push(format!("{indent}</CheckBoxField>"));
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
    emit_form_element_events(lines, element, name, &inner);
    lines.push(format!("{indent}</InputField>"));
}

pub(crate) fn emit_form_button(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
) {
    let id = ids.next();
    lines.push(format!(
        "{indent}<Button name=\"{}\" id=\"{id}\">",
        escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if let Some(button_type) = element.get("type").and_then(Value::as_str) {
        let mapped = match button_type {
            "usual" => "UsualButton",
            "hyperlink" => "Hyperlink",
            "commandBar" => "CommandBarButton",
            other => other,
        };
        lines.push(format!("{inner}<Type>{}</Type>", escape_xml(mapped)));
    }
    if let Some(command) = element.get("command").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<CommandName>Form.Command.{}</CommandName>",
            escape_xml(command)
        ));
    }
    if let Some(std_command) = element.get("stdCommand").and_then(Value::as_str) {
        let command_name = if let Some((item, command)) = std_command.rsplit_once('.') {
            format!("Form.Item.{item}.StandardCommand.{command}")
        } else {
            format!("Form.StandardCommand.{std_command}")
        };
        lines.push(format!(
            "{inner}<CommandName>{}</CommandName>",
            escape_xml(&command_name)
        ));
    }
    if let Some(title) = element.get("title").and_then(Value::as_str) {
        emit_form_mltext(lines, &inner, "Title", title);
    }
    emit_form_common_flags(lines, element, &inner);
    if element.get("defaultButton").and_then(Value::as_bool) == Some(true) {
        lines.push(format!("{inner}<DefaultButton>true</DefaultButton>"));
    }
    if let Some(picture) = element.get("picture").and_then(Value::as_str) {
        lines.push(format!("{inner}<Picture>"));
        lines.push(format!("{inner}\t<xr:Ref>{}</xr:Ref>", escape_xml(picture)));
        lines.push(format!(
            "{inner}\t<xr:LoadTransparent>true</xr:LoadTransparent>"
        ));
        lines.push(format!("{inner}</Picture>"));
    }
    if let Some(representation) = element.get("representation").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<Representation>{}</Representation>",
            escape_xml(representation)
        ));
    }
    if let Some(location) = element.get("locationInCommandBar").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<LocationInCommandBar>{}</LocationInCommandBar>",
            escape_xml(location)
        ));
    }
    emit_form_companion(
        lines,
        "ExtendedTooltip",
        &format!("{name}РасширеннаяПодсказка"),
        &inner,
        ids,
    );
    emit_form_element_events(lines, element, name, &inner);
    lines.push(format!("{indent}</Button>"));
}

pub(crate) fn emit_form_command_bar_element(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
) -> Result<(), String> {
    let id = ids.next();
    lines.push(format!(
        "{indent}<CommandBar name=\"{}\" id=\"{id}\">",
        escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if element.get("autofill").and_then(Value::as_bool) == Some(true) {
        lines.push(format!("{inner}<Autofill>true</Autofill>"));
    }
    emit_form_common_flags(lines, element, &inner);
    if let Some(children) = element.get("children").and_then(Value::as_array) {
        if !children.is_empty() {
            lines.push(format!("{inner}<ChildItems>"));
            for child in children {
                emit_form_element(lines, child, &format!("{inner}\t"), ids)?;
            }
            lines.push(format!("{inner}</ChildItems>"));
        }
    }
    lines.push(format!("{indent}</CommandBar>"));
    Ok(())
}

pub(crate) fn emit_form_label_field(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
) {
    let id = ids.next();
    lines.push(format!(
        "{indent}<LabelField name=\"{}\" id=\"{id}\">",
        escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if let Some(path) = element.get("path").and_then(Value::as_str) {
        lines.push(format!("{inner}<DataPath>{}</DataPath>", escape_xml(path)));
    }
    emit_form_common_flags(lines, element, &inner);
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
    emit_form_element_events(lines, element, name, &inner);
    lines.push(format!("{indent}</LabelField>"));
}

pub(crate) fn emit_form_table(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
) -> Result<(), String> {
    let id = ids.next();
    lines.push(format!(
        "{indent}<Table name=\"{}\" id=\"{id}\">",
        escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if let Some(path) = element.get("path").and_then(Value::as_str) {
        lines.push(format!("{inner}<DataPath>{}</DataPath>", escape_xml(path)));
    }
    emit_form_common_flags(lines, element, &inner);
    if let Some(value) = element.get("commandBarLocation").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<CommandBarLocation>{}</CommandBarLocation>",
            escape_xml(value)
        ));
    }
    if let Some(value) = element.get("initialTreeView").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<InitialTreeView>{}</InitialTreeView>",
            escape_xml(value)
        ));
    }
    if element.get("enableDrag").and_then(Value::as_bool).is_some() {
        let value = if element.get("enableDrag").and_then(Value::as_bool) == Some(true) {
            "true"
        } else {
            "false"
        };
        lines.push(format!("{inner}<EnableDrag>{value}</EnableDrag>"));
    }
    if let Some(value) = element.get("rowPictureDataPath").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<RowPictureDataPath>{}</RowPictureDataPath>",
            escape_xml(value)
        ));
    }
    if element.get("_dynList").and_then(Value::as_bool) == Some(true) {
        emit_form_dynamic_list_table_block(lines, element, &inner);
    }
    emit_form_companion(
        lines,
        "ContextMenu",
        &format!("{name}КонтекстноеМеню"),
        &inner,
        ids,
    );
    if element.get("tableAutofill").is_some() {
        let id = ids.next();
        let value = if element.get("tableAutofill").and_then(Value::as_bool) == Some(true) {
            "true"
        } else {
            "false"
        };
        lines.push(format!(
            "{inner}<AutoCommandBar name=\"{}КоманднаяПанель\" id=\"{id}\">",
            escape_xml(name)
        ));
        lines.push(format!("{inner}\t<Autofill>{value}</Autofill>"));
        lines.push(format!("{inner}</AutoCommandBar>"));
    } else {
        emit_form_companion(
            lines,
            "AutoCommandBar",
            &format!("{name}КоманднаяПанель"),
            &inner,
            ids,
        );
    }
    emit_form_companion(
        lines,
        "ExtendedTooltip",
        &format!("{name}РасширеннаяПодсказка"),
        &inner,
        ids,
    );
    emit_form_table_addition(
        lines,
        "SearchStringAddition",
        name,
        "СтрокаПоиска",
        "SearchStringRepresentation",
        &inner,
        ids,
    );
    emit_form_table_addition(
        lines,
        "ViewStatusAddition",
        name,
        "СостояниеПросмотра",
        "ViewStatusRepresentation",
        &inner,
        ids,
    );
    emit_form_table_addition(
        lines,
        "SearchControlAddition",
        name,
        "УправлениеПоиском",
        "SearchControl",
        &inner,
        ids,
    );

    emit_form_element_events(lines, element, name, &inner);
    if let Some(columns) = element.get("columns").and_then(Value::as_array) {
        if !columns.is_empty() {
            lines.push(format!("{inner}<ChildItems>"));
            for column in columns {
                emit_form_element(lines, column, &format!("{inner}\t"), ids)?;
            }
            lines.push(format!("{inner}</ChildItems>"));
        }
    }
    lines.push(format!("{indent}</Table>"));
    Ok(())
}

pub(crate) fn emit_form_dynamic_list_table_block(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    indent: &str,
) {
    let auto_refresh = if element.get("autoRefresh").and_then(Value::as_bool) == Some(true) {
        "true"
    } else {
        "false"
    };
    let auto_refresh_period = element
        .get("autoRefreshPeriod")
        .and_then(json_i64_value)
        .unwrap_or(60);
    let choice = element
        .get("choiceFoldersAndItems")
        .and_then(Value::as_str)
        .unwrap_or("Items");
    let restore = if element.get("restoreCurrentRow").and_then(Value::as_bool) == Some(true) {
        "true"
    } else {
        "false"
    };
    let show_root = if element.get("showRoot").and_then(Value::as_bool) == Some(false) {
        "false"
    } else {
        "true"
    };
    let allow_root_choice = if element.get("allowRootChoice").and_then(Value::as_bool) == Some(true)
    {
        "true"
    } else {
        "false"
    };
    let update_on_data_change = element
        .get("updateOnDataChange")
        .and_then(Value::as_str)
        .unwrap_or("Auto");
    let allow_url = if element
        .get("allowGettingCurrentRowURL")
        .and_then(Value::as_bool)
        == Some(false)
    {
        "false"
    } else {
        "true"
    };

    lines.push(format!("{indent}<AutoRefresh>{auto_refresh}</AutoRefresh>"));
    lines.push(format!(
        "{indent}<AutoRefreshPeriod>{auto_refresh_period}</AutoRefreshPeriod>"
    ));
    lines.push(format!("{indent}<Period>"));
    lines.push(format!(
        "{indent}\t<v8:variant xsi:type=\"v8:StandardPeriodVariant\">Custom</v8:variant>"
    ));
    lines.push(format!(
        "{indent}\t<v8:startDate>0001-01-01T00:00:00</v8:startDate>"
    ));
    lines.push(format!(
        "{indent}\t<v8:endDate>0001-01-01T00:00:00</v8:endDate>"
    ));
    lines.push(format!("{indent}</Period>"));
    lines.push(format!(
        "{indent}<ChoiceFoldersAndItems>{choice}</ChoiceFoldersAndItems>"
    ));
    lines.push(format!(
        "{indent}<RestoreCurrentRow>{restore}</RestoreCurrentRow>"
    ));
    lines.push(format!("{indent}<TopLevelParent xsi:nil=\"true\"/>"));
    lines.push(format!("{indent}<ShowRoot>{show_root}</ShowRoot>"));
    lines.push(format!(
        "{indent}<AllowRootChoice>{allow_root_choice}</AllowRootChoice>"
    ));
    lines.push(format!(
        "{indent}<UpdateOnDataChange>{update_on_data_change}</UpdateOnDataChange>"
    ));
    lines.push(format!(
        "{indent}<AllowGettingCurrentRowURL>{allow_url}</AllowGettingCurrentRowURL>"
    ));
}

pub(crate) fn emit_form_table_addition(
    lines: &mut Vec<String>,
    tag: &str,
    table_name: &str,
    suffix: &str,
    source_type: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
) {
    let name = format!("{table_name}{suffix}");
    let id = ids.next();
    lines.push(format!(
        "{indent}<{tag} name=\"{}\" id=\"{id}\">",
        escape_xml(&name)
    ));
    let inner = format!("{indent}\t");
    lines.push(format!("{inner}<AdditionSource>"));
    lines.push(format!("{inner}\t<Item>{}</Item>", escape_xml(table_name)));
    lines.push(format!("{inner}\t<Type>{source_type}</Type>"));
    lines.push(format!("{inner}</AdditionSource>"));
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
    lines.push(format!("{indent}</{tag}>"));
}

pub(crate) fn emit_form_element_events(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    element_name: &str,
    indent: &str,
) {
    let Some(events) = element.get("on").and_then(Value::as_array) else {
        return;
    };
    if events.is_empty() {
        return;
    }

    let handlers = element.get("handlers").and_then(Value::as_object);
    lines.push(format!("{indent}<Events>"));
    for event in events {
        let (event_name, handler, call_type) = if let Some(event_name) = event.as_str() {
            let handler = handlers
                .and_then(|values| values.get(event_name))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| form_event_handler_name(element_name, event_name));
            (event_name.to_string(), handler, None::<String>)
        } else if let Some(object) = event.as_object() {
            let event_name = object
                .get("event")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| json_value_to_python_string(event));
            let handler = object
                .get("handler")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| {
                    handlers
                        .and_then(|values| values.get(&event_name))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .unwrap_or_else(|| form_event_handler_name(element_name, &event_name));
            let call_type = object
                .get("callType")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            (event_name, handler, call_type)
        } else {
            let event_name = json_value_to_python_string(event);
            let handler = handlers
                .and_then(|values| values.get(&event_name))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| form_event_handler_name(element_name, &event_name));
            (event_name, handler, None)
        };
        let call_type_attr = call_type
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(|value| format!(" callType=\"{}\"", escape_xml(value)))
            .unwrap_or_default();
        lines.push(format!(
            "{indent}\t<Event name=\"{}\"{}>{}</Event>",
            escape_xml(&event_name),
            call_type_attr,
            escape_xml(&handler)
        ));
    }
    lines.push(format!("{indent}</Events>"));
}

pub(crate) fn form_event_handler_name(element_name: &str, event_name: &str) -> String {
    let suffix = match event_name {
        "Click" => "Нажатие",
        "OnChange" => "ПриИзменении",
        "StartChoice" => "НачалоВыбора",
        "ChoiceProcessing" => "ОбработкаВыбора",
        "AutoComplete" => "АвтоПодбор",
        "Clearing" => "Очистка",
        "Opening" => "Открытие",
        "OnActivateRow" => "ПриАктивизацииСтроки",
        "BeforeAddRow" => "ПередНачаломДобавления",
        "BeforeDeleteRow" => "ПередУдалением",
        "BeforeRowChange" => "ПередНачаломИзменения",
        "OnStartEdit" => "ПриНачалеРедактирования",
        "OnEndEdit" => "ПриОкончанииРедактирования",
        "Selection" => "ВыборСтроки",
        "OnCurrentPageChange" => "ПриСменеСтраницы",
        "TextEditEnd" => "ОкончаниеВводаТекста",
        "URLProcessing" => "ОбработкаНавигационнойСсылки",
        "DragStart" => "НачалоПеретаскивания",
        "Drag" => "Перетаскивание",
        "DragCheck" => "ПроверкаПеретаскивания",
        "Drop" => "Помещение",
        "AfterDeleteRow" => "ПослеУдаления",
        _ => event_name,
    };
    format!("{element_name}{suffix}")
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

pub(crate) fn form_compile_main_attribute_saves_data(type_name: &str) -> bool {
    [
        "CatalogObject.",
        "DocumentObject.",
        "ChartOfAccountsObject.",
        "ChartOfCalculationTypesObject.",
        "ChartOfCharacteristicTypesObject.",
        "ExchangePlanObject.",
        "BusinessProcessObject.",
        "TaskObject.",
    ]
    .iter()
    .any(|prefix| type_name.starts_with(prefix))
        || type_name.contains("RecordManager.")
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
        let type_name = object.get("type").and_then(Value::as_str);
        if let Some(type_name) = type_name {
            emit_form_type(lines, type_name, &inner);
        } else {
            lines.push(format!("{inner}<Type/>"));
        }
        let main_attribute = object.get("main").and_then(Value::as_bool) == Some(true);
        if main_attribute {
            lines.push(format!("{inner}<MainAttribute>true</MainAttribute>"));
        }
        let saved_data = if object.contains_key("savedData") {
            object.get("savedData").and_then(Value::as_bool) == Some(true)
        } else {
            main_attribute && type_name.is_some_and(form_compile_main_attribute_saves_data)
        };
        if saved_data {
            lines.push(format!("{inner}<SavedData>true</SavedData>"));
        }
        if object.get("type").and_then(Value::as_str) == Some("DynamicList") {
            if let Some(settings) = object.get("settings").and_then(Value::as_object) {
                emit_form_dynamic_list_attribute_settings(lines, settings, &inner);
            }
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

pub(crate) fn emit_form_attribute_columns(
    lines: &mut Vec<String>,
    columns: Option<&Value>,
    indent: &str,
) -> Result<(), String> {
    let Some(columns) = columns else {
        return Ok(());
    };
    let columns = columns
        .as_array()
        .ok_or_else(|| "Form attribute columns must be an array".to_string())?;
    if columns.is_empty() {
        return Ok(());
    }

    lines.push(format!("{indent}<Columns>"));
    for (idx, column) in columns.iter().enumerate() {
        let object = column
            .as_object()
            .ok_or_else(|| format!("Form attribute column #{} must be an object", idx + 1))?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Form attribute column #{} is missing name", idx + 1))?;
        let column_indent = format!("{indent}\t");
        lines.push(format!(
            "{column_indent}<Column name=\"{}\" id=\"{}\">",
            escape_xml(name),
            idx + 1
        ));
        let inner = format!("{column_indent}\t");
        if let Some(title) = object.get("title").and_then(Value::as_str) {
            emit_form_mltext(lines, &inner, "Title", title);
        }
        let type_name = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Form attribute column '{name}' is missing type"))?;
        emit_form_type(lines, type_name, &inner);
        lines.push(format!("{column_indent}</Column>"));
    }
    lines.push(format!("{indent}</Columns>"));
    Ok(())
}

pub(crate) fn emit_form_dynamic_list_attribute_settings(
    lines: &mut Vec<String>,
    settings: &Map<String, Value>,
    indent: &str,
) {
    const CANON_FILTER_ID: &str = "dfcece9d-5077-440b-b6b3-45a5cb4538eb";
    const CANON_ORDER_ID: &str = "88619765-ccb3-46c6-ac52-38e9c992ebd4";
    const CANON_CA_ID: &str = "b75fecce-942b-4aed-abc9-e6a02e460fb3";
    const CANON_ITEMS_ID: &str = "911b6018-f537-43e8-a417-da56b22f9aec";

    let manual_query = if settings.get("manualQuery").and_then(Value::as_bool) == Some(true) {
        "true"
    } else {
        "false"
    };
    let dynamic_data_read = if settings
        .get("dynamicDataRead")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "true"
    } else {
        "false"
    };

    lines.push(format!("{indent}<Settings xsi:type=\"DynamicList\">"));
    lines.push(format!(
        "{indent}\t<ManualQuery>{manual_query}</ManualQuery>"
    ));
    lines.push(format!(
        "{indent}\t<DynamicDataRead>{dynamic_data_read}</DynamicDataRead>"
    ));
    if let Some(main_table) = settings.get("mainTable").and_then(Value::as_str) {
        lines.push(format!(
            "{indent}\t<MainTable>{}</MainTable>",
            escape_xml(main_table)
        ));
    }
    lines.push(format!("{indent}\t<ListSettings>"));
    lines.push(format!("{indent}\t\t<dcsset:filter>"));
    lines.push(format!(
        "{indent}\t\t\t<dcsset:viewMode>Normal</dcsset:viewMode>"
    ));
    lines.push(format!(
        "{indent}\t\t\t<dcsset:userSettingID>{CANON_FILTER_ID}</dcsset:userSettingID>"
    ));
    lines.push(format!("{indent}\t\t</dcsset:filter>"));
    lines.push(format!("{indent}\t\t<dcsset:order>"));
    lines.push(format!(
        "{indent}\t\t\t<dcsset:viewMode>Normal</dcsset:viewMode>"
    ));
    lines.push(format!(
        "{indent}\t\t\t<dcsset:userSettingID>{CANON_ORDER_ID}</dcsset:userSettingID>"
    ));
    lines.push(format!("{indent}\t\t</dcsset:order>"));
    lines.push(format!("{indent}\t\t<dcsset:conditionalAppearance>"));
    lines.push(format!(
        "{indent}\t\t\t<dcsset:viewMode>Normal</dcsset:viewMode>"
    ));
    lines.push(format!(
        "{indent}\t\t\t<dcsset:userSettingID>{CANON_CA_ID}</dcsset:userSettingID>"
    ));
    lines.push(format!("{indent}\t\t</dcsset:conditionalAppearance>"));
    lines.push(format!(
        "{indent}\t\t<dcsset:itemsViewMode>Normal</dcsset:itemsViewMode>"
    ));
    lines.push(format!(
        "{indent}\t\t<dcsset:itemsUserSettingID>{CANON_ITEMS_ID}</dcsset:itemsUserSettingID>"
    ));
    lines.push(format!("{indent}\t</ListSettings>"));
    lines.push(format!("{indent}</Settings>"));
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
    } else if matches!(normalized.as_str(), "DynamicList" | "ConstantsSet") {
        lines.push(format!("{indent}<v8:Type>cfg:{normalized}</v8:Type>"));
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
        || normalized.starts_with("CalculationRegisterRecordSet.")
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
    use crate::infrastructure::native_operations::NativeOperationAdapter;
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

    fn empty_catalog_xml(line_ending: &str, trailing_newline: bool) -> String {
        let mut text = [
            r#"<?xml version="1.0" encoding="utf-8"?>"#,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.17">"#,
            r#"	<Catalog uuid="00000000-0000-0000-0000-000000000001">"#,
            r#"		<Properties>"#,
            r#"			<Name>Goods</Name>"#,
            r#"			<Synonym>Goods</Synonym>"#,
            r#"			<DefaultListForm/>"#,
            r#"		</Properties>"#,
            r#"		<ChildObjects/>"#,
            r#"	</Catalog>"#,
            r#"</MetaDataObject>"#,
        ]
        .join(line_ending);
        if trailing_newline {
            text.push_str(line_ending);
        }
        text
    }

    fn add_list_form_args(object_path: &Path, form_name: &str) -> Map<String, serde_json::Value> {
        let mut args = Map::new();
        args.insert(
            "ObjectPath".to_string(),
            json!(object_path.display().to_string()),
        );
        args.insert("FormName".to_string(), json!(form_name));
        args.insert("Purpose".to_string(), json!("List"));
        args.insert("Synonym".to_string(), json!("List form"));
        args
    }

    #[test]
    fn add_form_set_default_false_leaves_empty_default_slot() {
        let context = temp_context("add-set-default-false");
        let root_xml = context.cwd.join("src").join("Catalogs").join("Goods.xml");
        write_file(&root_xml, &empty_catalog_xml("\n", true));

        let mut args = add_list_form_args(&root_xml, "ListForm");
        args.insert("SetDefault".to_string(), json!(false));

        let outcome = add_form(&args, &context);

        assert!(outcome.ok, "{:?}", outcome.errors);
        let updated = fs::read_to_string(&root_xml).unwrap();
        assert!(updated.contains("<DefaultListForm/>"), "{updated}");
        assert!(
            !updated.contains("<DefaultListForm>Catalog.Goods.Form.ListForm</DefaultListForm>"),
            "{updated}"
        );
        assert!(updated.contains("<Form>ListForm</Form>"), "{updated}");
        assert!(
            !outcome
                .stdout
                .as_deref()
                .unwrap_or("")
                .contains("DefaultListForm: Catalog.Goods.Form.ListForm"),
            "{:?}",
            outcome.stdout
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn add_form_sets_default_when_explicit_true_or_omitted_for_empty_slot() {
        for (case, set_default_arg) in [("explicit-true", Some(true)), ("omitted", None)] {
            let context = temp_context(case);
            let root_xml = context.cwd.join("src").join("Catalogs").join("Goods.xml");
            write_file(&root_xml, &empty_catalog_xml("\n", true));

            let mut args = add_list_form_args(&root_xml, "ListForm");
            if let Some(value) = set_default_arg {
                args.insert("SetDefault".to_string(), json!(value));
            }

            let outcome = add_form(&args, &context);

            assert!(outcome.ok, "{case}: {:?}", outcome.errors);
            let updated = fs::read_to_string(&root_xml).unwrap();
            assert!(
                updated.contains("<DefaultListForm>Catalog.Goods.Form.ListForm</DefaultListForm>"),
                "{case}: {updated}"
            );

            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn add_form_set_default_true_overwrites_existing_default_slot() {
        let context = temp_context("add-set-default-overwrite");
        let root_xml = context.cwd.join("src").join("Catalogs").join("Goods.xml");
        let source = empty_catalog_xml("\n", true).replace(
            "<DefaultListForm/>",
            "<DefaultListForm>Catalog.Goods.Form.OldListForm</DefaultListForm>",
        );
        write_file(&root_xml, &source);

        let mut args = add_list_form_args(&root_xml, "ListForm");
        args.insert("SetDefault".to_string(), json!(true));

        let outcome = add_form(&args, &context);

        assert!(outcome.ok, "{:?}", outcome.errors);
        let updated = fs::read_to_string(&root_xml).unwrap();
        assert!(
            updated.contains("<DefaultListForm>Catalog.Goods.Form.ListForm</DefaultListForm>"),
            "{updated}"
        );
        assert!(
            !updated.contains("Catalog.Goods.Form.OldListForm"),
            "{updated}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn add_then_remove_form_round_trips_empty_catalog_parent_xml() {
        let context = temp_context("add-remove-roundtrip");
        let root_xml = context.cwd.join("Catalogs").join("Goods.xml");
        let original = empty_catalog_xml("\r\n", false);
        write_file(&root_xml, &original);

        let mut add_args = add_list_form_args(&root_xml, "ListForm");
        add_args.insert("SetDefault".to_string(), json!(false));
        let add_outcome = add_form(&add_args, &context);
        assert!(add_outcome.ok, "{:?}", add_outcome.errors);

        let mut remove_args = Map::new();
        remove_args.insert("ObjectName".to_string(), json!("Goods"));
        remove_args.insert("FormName".to_string(), json!("ListForm"));
        remove_args.insert("SrcDir".to_string(), json!("Catalogs"));
        let remove_outcome = remove_form(&remove_args, &context);
        assert!(remove_outcome.ok, "{:?}", remove_outcome.errors);

        let updated = fs::read_to_string(&root_xml)
            .unwrap()
            .trim_start_matches('\u{feff}')
            .to_string();
        assert_eq!(updated, original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn remove_form_does_not_collapse_unrelated_empty_child_objects() {
        let context = temp_context("remove-preserve-unrelated-childobjects");
        let root_xml = context.cwd.join("src").join("Catalogs").join("Goods.xml");
        let form_meta = context
            .cwd
            .join("src")
            .join("Catalogs")
            .join("Goods")
            .join("Forms")
            .join("ListForm.xml");
        let form_content = context
            .cwd
            .join("src")
            .join("Catalogs")
            .join("Goods")
            .join("Forms")
            .join("ListForm")
            .join("Ext")
            .join("Form.xml");
        write_file(
            &root_xml,
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
	<Catalog uuid="00000000-0000-0000-0000-000000000001">
		<Properties>
			<Name>Goods</Name>
		</Properties>
		<ChildObjects>
			<Attribute>
				<ChildObjects></ChildObjects>
			</Attribute>
			<Form>ListForm</Form>
		</ChildObjects>
	</Catalog>
</MetaDataObject>
"#,
        );
        write_file(&form_meta, "<MetaDataObject/>\n");
        write_file(&form_content, "<Form/>\n");

        let mut args = Map::new();
        args.insert("ObjectName".to_string(), json!("Goods"));
        args.insert("FormName".to_string(), json!("ListForm"));
        args.insert("SrcDir".to_string(), json!("src/Catalogs"));

        let outcome = remove_form(&args, &context);

        assert!(outcome.ok, "{:?}", outcome.errors);
        let updated = fs::read_to_string(&root_xml).unwrap();
        assert!(
            updated.contains("<ChildObjects></ChildObjects>"),
            "{updated}"
        );
        assert!(!updated.contains("<Form>ListForm</Form>"), "{updated}");

        let _ = fs::remove_dir_all(&context.cwd);
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
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
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

    #[test]
    fn validate_form_ignores_ui_element_type_properties() {
        let context = temp_context("ui-type-property");
        let form_path = context.cwd.join("Form.xml");
        write_file(
            &form_path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems>
		<Button name="RunParityActionButton" id="1">
			<Type>CommandBarButton</Type>
			<CommandName>Form.Command.RunParityAction</CommandName>
			<ExtendedTooltip name="RunParityActionButtonРасширеннаяПодсказка" id="2"/>
		</Button>
	</ChildItems>
	<Commands>
		<Command name="RunParityAction" id="1">
			<Action>RunParityAction</Action>
		</Command>
	</Commands>
</Form>
"#,
        );

        let mut args = Map::new();
        args.insert(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        );
        args.insert("Detailed".to_string(), json!(true));

        let outcome = validate_form(&args, &context);
        let stdout = outcome.stdout.as_deref().unwrap_or("");

        assert!(outcome.ok, "{stdout}");
        assert!(
            !stdout.contains("CommandBarButton\": bare type"),
            "{stdout}"
        );
        assert!(
            stdout.contains("12. Types: no type values to check"),
            "{stdout}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn remove_form_clears_all_default_form_slots_referencing_removed_form() {
        let context = temp_context("remove-default-slots");
        let root_xml = context.cwd.join("src").join("Catalogs").join("Goods.xml");
        let form_meta = context
            .cwd
            .join("src")
            .join("Catalogs")
            .join("Goods")
            .join("Forms")
            .join("ListForm.xml");
        let form_content = context
            .cwd
            .join("src")
            .join("Catalogs")
            .join("Goods")
            .join("Forms")
            .join("ListForm")
            .join("Ext")
            .join("Form.xml");
        write_file(
            &root_xml,
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
	<Catalog name="Goods">
		<Forms>
			<Form>Catalog.Goods.Form.ListForm</Form>
			<Form>Catalog.Goods.Form.OtherForm</Form>
		</Forms>
		<DefaultObjectForm>Catalog.Goods.Form.ListForm</DefaultObjectForm>
		<DefaultListForm>Catalog.Goods.Form.ListForm</DefaultListForm>
		<DefaultChoiceForm>Catalog.Goods.Form.ListForm</DefaultChoiceForm>
		<DefaultRecordForm>Catalog.Goods.Form.ListForm</DefaultRecordForm>
		<DefaultForm>Catalog.Goods.Form.OtherForm</DefaultForm>
	</Catalog>
</MetaDataObject>
"#,
        );
        write_file(&form_meta, "<MetaDataObject/>\n");
        write_file(&form_content, "<Form/>\n");

        let mut args = Map::new();
        args.insert("ObjectName".to_string(), json!("Goods"));
        args.insert("FormName".to_string(), json!("ListForm"));
        args.insert("SrcDir".to_string(), json!("src/Catalogs"));

        let outcome = remove_form(&args, &context);

        assert!(outcome.ok, "{:?}", outcome.errors);
        let updated = fs::read_to_string(&root_xml).unwrap();
        assert!(
            !updated.contains("<Form>Catalog.Goods.Form.ListForm</Form>"),
            "{updated}"
        );
        for tag in [
            "DefaultObjectForm",
            "DefaultListForm",
            "DefaultChoiceForm",
            "DefaultRecordForm",
        ] {
            assert!(updated.contains(&format!("<{tag}/>")), "{tag}: {updated}");
        }
        assert!(
            updated.contains("<DefaultForm>Catalog.Goods.Form.OtherForm</DefaultForm>"),
            "{updated}"
        );
        assert!(!form_meta.exists());
        assert!(!form_content.exists());

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_rejects_duplicate_attribute_and_command_names() {
        let context = temp_context("edit-duplicates");
        let form_path = context.cwd.join("Form.xml");
        let json_path = context.cwd.join("edit.json");
        write_file(&form_path, editable_form_xml(false));
        write_file(
            &json_path,
            r#"{
  "attributes": [
    {"name": "Object", "type": "CatalogObject.ParityCatalog"}
  ],
  "commands": [
    {"name": "Refresh", "title": "Refresh again"}
  ]
}
"#,
        );

        let mut args = Map::new();
        args.insert(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        );
        args.insert(
            "JsonPath".to_string(),
            json!(json_path.display().to_string()),
        );

        let outcome = edit_form(&args, &context);
        assert!(!outcome.ok, "{outcome:?}");
        let stderr = outcome.stderr.unwrap_or_default();
        assert!(
            stderr.contains("Attribute 'Object' already exists in form"),
            "{stderr}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_uses_extension_id_floor_when_base_form_exists() {
        let context = temp_context("edit-extension-ids");
        let form_path = context.cwd.join("Form.xml");
        let json_path = context.cwd.join("edit.json");
        write_file(&form_path, editable_form_xml(true));
        write_file(
            &json_path,
            r#"{
  "attributes": [
    {"name": "NewAttribute", "type": "string"}
  ],
  "commands": [
    {"name": "NewCommand", "title": "New command"}
  ],
  "elements": [
    {"input": "NewAttribute", "path": "NewAttribute"}
  ]
}
"#,
        );

        let mut args = Map::new();
        args.insert(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        );
        args.insert(
            "JsonPath".to_string(),
            json!(json_path.display().to_string()),
        );

        let outcome = edit_form(&args, &context);
        assert!(outcome.ok, "{outcome:?}");
        let updated = fs::read_to_string(&form_path).unwrap();
        assert!(updated.contains("id=\"1000000\""), "{updated}");
        assert!(updated.contains("id=\"1000001\""), "{updated}");

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_emits_valuetable_attribute_columns() {
        let context = temp_context("edit-valuetable-columns");
        let form_path = context.cwd.join("Form.xml");
        let json_path = context.cwd.join("edit.json");
        write_file(
            &form_path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1">
		<Autofill>true</Autofill>
	</AutoCommandBar>
	<ChildItems>
		<UsualGroup name="ГруппаДанных" id="1">
			<ChildItems/>
			<ExtendedTooltip name="ГруппаДанныхРасширеннаяПодсказка" id="2"/>
		</UsualGroup>
	</ChildItems>
	<Attributes/>
	<Commands/>
</Form>
"#,
        );
        write_file(
            &json_path,
            r#"{
  "into": "ГруппаДанных",
  "attributes": [
    {
      "name": "ТаблицаДанных",
      "type": "ValueTable",
      "columns": [
        {"name": "НомерСтроки", "type": "decimal(5,0)"},
        {"name": "Значение", "type": "string(200)"}
      ]
    }
  ],
  "elements": [
    {
      "table": "ТаблицаДанных",
      "path": "ТаблицаДанных",
      "columns": [
        {"input": "НомерСтроки", "path": "ТаблицаДанных.НомерСтроки"},
        {"input": "Значение", "path": "ТаблицаДанных.Значение"}
      ]
    }
  ]
}
"#,
        );

        let mut args = Map::new();
        args.insert(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        );
        args.insert(
            "JsonPath".to_string(),
            json!(json_path.display().to_string()),
        );

        let outcome = edit_form(&args, &context);
        assert!(outcome.ok, "{outcome:?}");
        let updated = fs::read_to_string(&form_path).unwrap();
        assert!(updated.contains("<Columns>"), "{updated}");
        assert!(
            updated.contains("<Column name=\"НомерСтроки\" id=\"1\">"),
            "{updated}"
        );
        assert!(
            updated.contains("<Column name=\"Значение\" id=\"2\">"),
            "{updated}"
        );
        assert!(
            updated.contains("<v8:Type>xs:decimal</v8:Type>"),
            "{updated}"
        );
        assert!(updated.contains("<v8:NumberQualifiers>"), "{updated}");
        assert!(
            updated.contains("<v8:Type>xs:string</v8:Type>"),
            "{updated}"
        );
        assert!(updated.contains("<v8:StringQualifiers>"), "{updated}");
        assert!(
            updated.contains("<DataPath>ТаблицаДанных.НомерСтроки</DataPath>"),
            "{updated}"
        );
        assert!(
            updated.contains("<DataPath>ТаблицаДанных.Значение</DataPath>"),
            "{updated}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_rejects_duplicate_attribute_column_names() {
        let context = temp_context("edit-duplicate-attribute-columns");
        let form_path = context.cwd.join("Form.xml");
        let json_path = context.cwd.join("edit.json");
        write_file(&form_path, editable_form_xml(false));
        write_file(
            &json_path,
            r#"{
  "attributes": [
    {
      "name": "ТаблицаДанных",
      "type": "ValueTable",
      "columns": [
        {"name": "Значение", "type": "string"},
        {"name": "Значение", "type": "string"}
      ]
    }
  ]
}
"#,
        );

        let mut args = Map::new();
        args.insert(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        );
        args.insert(
            "JsonPath".to_string(),
            json!(json_path.display().to_string()),
        );

        let outcome = edit_form(&args, &context);
        assert!(!outcome.ok, "{outcome:?}");
        let stderr = outcome.stderr.unwrap_or_default();
        assert!(
            stderr.contains(
                "Duplicate column name 'Значение' in attribute 'ТаблицаДанных' edit definition"
            ),
            "{stderr}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn form_edit_rejects_malformed_or_unsupported_attribute_columns() {
        let cases = [
            (
                json!([{"name": "Data", "type": "ValueTable", "columns": {"name": "A", "type": "string"}}]),
                "columns must be an array",
            ),
            (
                json!([{"name": "Data", "type": "ValueTable", "columns": [null]}]),
                "column #1 must be an object",
            ),
            (
                json!([{"name": "Data", "type": "ValueTable", "columns": [{"type": "string"}]}]),
                "column #1 requires non-empty name",
            ),
            (
                json!([{"name": "Data", "type": "ValueTable", "columns": [{"name": "A"}]}]),
                "column 'A' requires non-empty type",
            ),
            (
                json!([{"name": "Data", "type": "string", "columns": [{"name": "A", "type": "string"}]}]),
                "columns are supported only for ValueTable or ValueTree",
            ),
        ];

        for (attrs, expected) in cases {
            let error =
                form_edit_validate_attribute_columns(attrs.as_array().unwrap()).unwrap_err();
            assert!(
                error.contains(expected),
                "expected {expected:?}, got {error:?}"
            );
        }
    }

    #[test]
    fn edit_form_emits_button_bound_to_form_command() {
        let context = temp_context("edit-button");
        let form_path = context.cwd.join("Form.xml");
        let json_path = context.cwd.join("edit.json");
        write_file(&form_path, editable_form_xml(false));
        write_file(
            &json_path,
            r#"{
  "commands": [
    {"name": "RunParityAction", "title": "Run parity action"}
  ],
  "elements": [
    {
      "button": "RunParityActionButton",
      "type": "commandBar",
      "command": "RunParityAction",
      "title": "Run parity action"
    }
  ]
}
"#,
        );

        let mut args = Map::new();
        args.insert(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        );
        args.insert(
            "JsonPath".to_string(),
            json!(json_path.display().to_string()),
        );

        let outcome = edit_form(&args, &context);
        assert!(outcome.ok, "{outcome:?}");
        let updated = fs::read_to_string(&form_path).unwrap();
        assert!(
            updated.contains("<Button name=\"RunParityActionButton\""),
            "{updated}"
        );
        assert!(
            updated.contains("<Type>CommandBarButton</Type>"),
            "{updated}"
        );
        assert!(
            updated.contains("<CommandName>Form.Command.RunParityAction</CommandName>"),
            "{updated}"
        );
        assert!(
            updated.contains("<ExtendedTooltip name=\"RunParityActionButtonРасширеннаяПодсказка\""),
            "{updated}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_adds_command_bar_button_into_existing_container() {
        let context = temp_context("edit-command-bar");
        let form_path = context.cwd.join("Form.xml");
        let json_path = context.cwd.join("edit.json");
        write_file(
            &form_path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.17">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems>
		<CommandBar name="ПанельДействий" id="1">
			<ChildItems/>
		</CommandBar>
	</ChildItems>
	<Attributes/>
	<Commands/>
</Form>
"#,
        );
        write_file(
            &json_path,
            r#"{
  "into": "ПанельДействий",
  "elements": [
    {
      "button": "Заполнить",
      "type": "commandBar",
      "command": "Заполнить",
      "locationInCommandBar": "InAdditionalSubmenu"
    }
  ],
  "commands": [
    { "name": "Заполнить", "action": "ЗаполнитьОбработка" }
  ]
}
"#,
        );

        let mut args = Map::new();
        args.insert(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        );
        args.insert(
            "JsonPath".to_string(),
            json!(json_path.display().to_string()),
        );

        let outcome = edit_form(&args, &context);
        assert!(outcome.ok, "{outcome:?}");
        let updated = fs::read_to_string(&form_path).unwrap();
        assert_eq!(
            updated
                .matches("<CommandBar name=\"ПанельДействий\"")
                .count(),
            1
        );
        assert_eq!(updated.matches("<Command name=\"Заполнить\"").count(), 1);
        assert!(updated.contains("<Button name=\"Заполнить\""), "{updated}");
        assert!(
            updated.contains("<Type>CommandBarButton</Type>"),
            "{updated}"
        );
        assert!(
            updated.contains("<CommandName>Form.Command.Заполнить</CommandName>"),
            "{updated}"
        );
        assert!(
            updated.contains("<LocationInCommandBar>InAdditionalSubmenu</LocationInCommandBar>"),
            "{updated}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_creates_child_items_for_target_container() {
        let context = temp_context("edit-command-bar-no-child-items");
        let form_path = context.cwd.join("Form.xml");
        let json_path = context.cwd.join("edit.json");
        write_file(
            &form_path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.17">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems>
		<CommandBar name="ПанельДействий" id="1"/>
	</ChildItems>
	<Attributes/>
	<Commands/>
</Form>
"#,
        );
        write_file(
            &json_path,
            r#"{
  "into": "ПанельДействий",
  "elements": [
    {
      "button": "Заполнить",
      "type": "commandBar",
      "command": "Заполнить"
    }
  ],
  "commands": [
    { "name": "Заполнить", "action": "ЗаполнитьОбработка" }
  ]
}
"#,
        );

        let mut args = Map::new();
        args.insert(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        );
        args.insert(
            "JsonPath".to_string(),
            json!(json_path.display().to_string()),
        );

        let outcome = edit_form(&args, &context);
        assert!(outcome.ok, "{outcome:?}");
        let updated = fs::read_to_string(&form_path).unwrap();
        assert!(
            updated.contains("<CommandBar name=\"ПанельДействий\" id=\"1\">"),
            "{updated}"
        );
        assert!(
            updated.contains("\t\t\t<ChildItems>\n\t\t\t\t<Button name=\"Заполнить\""),
            "{updated}"
        );
        assert!(updated.contains("<Button name=\"Заполнить\""), "{updated}");

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_creates_child_items_after_self_closing_extended_tooltip() {
        let context = temp_context("edit-group-with-extended-tooltip");
        let form_path = context.cwd.join("Form.xml");
        let json_path = context.cwd.join("edit.json");
        write_file(
            &form_path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.17">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems>
		<UsualGroup name="ГруппаЗамены" id="1">
			<ExtendedTooltip name="ГруппаЗаменыРасширеннаяПодсказка" id="2"/>
		</UsualGroup>
	</ChildItems>
	<Attributes/>
	<Commands/>
</Form>
"#,
        );
        write_file(
            &json_path,
            r#"{
  "into": "ГруппаЗамены",
  "elements": [
    { "table": "ТаблицаЗамены" }
  ]
}
"#,
        );

        let mut args = Map::new();
        args.insert(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        );
        args.insert(
            "JsonPath".to_string(),
            json!(json_path.display().to_string()),
        );

        let outcome = edit_form(&args, &context);
        assert!(outcome.ok, "{outcome:?}");
        let updated = fs::read_to_string(&form_path).unwrap();
        let tooltip_pos = updated
            .find("<ExtendedTooltip name=\"ГруппаЗаменыРасширеннаяПодсказка\" id=\"2\"/>")
            .unwrap();
        let child_items_pos = updated[tooltip_pos..]
            .find("<ChildItems>")
            .map(|pos| tooltip_pos + pos)
            .unwrap();
        assert!(tooltip_pos < child_items_pos, "{updated}");
        assert!(
            updated.contains("<Table name=\"ТаблицаЗамены\""),
            "{updated}"
        );
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let validate_outcome = validate_form(&args, &context);
        assert!(validate_outcome.ok, "{validate_outcome:?}");

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn form_edit_namespace_repair_accepts_whitespace_around_equals() {
        let mut xml = r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:v8 = "http://v8.1c.ru/8.1/data/core"><ChildItems/></Form>"#.to_string();
        let root_start = Document::parse(&xml).unwrap().root_element().range().start;

        form_edit_ensure_emitted_namespaces(&mut xml, root_start, "<v8:item/>").unwrap();

        assert_eq!(xml.matches("xmlns:v8").count(), 1, "{xml}");
        Document::parse(&xml).unwrap();
    }

    #[test]
    fn form_edit_namespace_repair_uses_parsed_root_after_comment() {
        let mut xml = r#"<!-- misleading <Form marker --><Form xmlns="http://v8.1c.ru/8.3/xcf/logform"><ChildItems/></Form>"#.to_string();
        let root_start = Document::parse(&xml).unwrap().root_element().range().start;

        form_edit_ensure_emitted_namespaces(&mut xml, root_start, "<v8:item/>").unwrap();

        assert!(xml.starts_with("<!-- misleading <Form marker -->"), "{xml}");
        assert!(xml[root_start..].starts_with("<Form "), "{xml}");
        Document::parse(&xml).unwrap();
    }

    #[test]
    fn form_edit_namespace_repair_is_noop_without_emitted_prefixes() {
        let mut xml =
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"><ChildItems/></Form>"#.to_string();
        let original = xml.clone();
        let root_start = Document::parse(&xml).unwrap().root_element().range().start;

        form_edit_ensure_emitted_namespaces(&mut xml, root_start, "<Table/>").unwrap();

        assert_eq!(xml, original);
    }

    #[test]
    fn edit_form_inserts_element_after_existing_element() {
        let context = temp_context("edit-after-element");
        let form_path = context.cwd.join("Form.xml");
        let json_path = context.cwd.join("edit.json");
        write_file(
            &form_path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.17">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems>
		<Button name="ExistingButton" id="1"/>
	</ChildItems>
	<Attributes/>
	<Commands/>
</Form>
"#,
        );
        write_file(
            &json_path,
            r#"{
  "after": "ExistingButton",
  "elements": [
    {"button": "InsertedButton", "type": "commandBar"}
  ]
}
"#,
        );

        let mut args = Map::new();
        args.insert(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        );
        args.insert(
            "JsonPath".to_string(),
            json!(json_path.display().to_string()),
        );

        let outcome = edit_form(&args, &context);
        assert!(outcome.ok, "{outcome:?}");
        let updated = fs::read_to_string(&form_path).unwrap();
        let existing_pos = updated.find("ExistingButton").unwrap();
        let inserted_pos = updated.find("InsertedButton").unwrap();
        assert!(existing_pos < inserted_pos, "{updated}");

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_rejects_duplicate_element_name() {
        let context = temp_context("edit-duplicate-element-command");
        let form_path = context.cwd.join("Form.xml");
        let json_path = context.cwd.join("edit.json");
        write_file(
            &form_path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.17">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems>
		<Button name="Заполнить" id="1"/>
	</ChildItems>
	<Attributes/>
	<Commands>
		<Command name="Заполнить" id="2"/>
	</Commands>
</Form>
"#,
        );
        let original = fs::read_to_string(&form_path).unwrap();
        write_file(
            &json_path,
            r#"{
  "elements": [
    {"button": "Заполнить", "type": "commandBar"}
  ]
}
"#,
        );

        let mut args = Map::new();
        args.insert(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        );
        args.insert(
            "JsonPath".to_string(),
            json!(json_path.display().to_string()),
        );

        let outcome = edit_form(&args, &context);
        assert!(!outcome.ok, "{outcome:?}");
        let stderr = outcome.stderr.unwrap_or_default();
        assert!(
            stderr.contains("Element 'Заполнить' already exists in form"),
            "{stderr}"
        );
        assert_eq!(fs::read_to_string(&form_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_rejects_into_target_outside_child_items_tree() {
        let context = temp_context("edit-into-command-name");
        let form_path = context.cwd.join("Form.xml");
        let json_path = context.cwd.join("edit.json");
        write_file(
            &form_path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.17">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems/>
	<Attributes/>
	<Commands>
		<Command name="Заполнить" id="1"/>
	</Commands>
</Form>
"#,
        );
        let original = fs::read_to_string(&form_path).unwrap();
        write_file(
            &json_path,
            r#"{
  "into": "Заполнить",
  "elements": [
    {"button": "InsertedButton", "type": "commandBar"}
  ]
}
"#,
        );

        let mut args = Map::new();
        args.insert(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        );
        args.insert(
            "JsonPath".to_string(),
            json!(json_path.display().to_string()),
        );

        let outcome = edit_form(&args, &context);
        assert!(!outcome.ok, "{outcome:?}");
        let stderr = outcome.stderr.unwrap_or_default();
        assert!(
            stderr.contains("Target group 'Заполнить' not found"),
            "{stderr}"
        );
        assert_eq!(fs::read_to_string(&form_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_rejects_nested_duplicate_element_name() {
        let context = temp_context("edit-nested-duplicate-element");
        let form_path = context.cwd.join("Form.xml");
        let json_path = context.cwd.join("edit.json");
        write_file(
            &form_path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.17">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems>
		<Button name="Заполнить" id="1"/>
	</ChildItems>
	<Attributes/>
	<Commands/>
</Form>
"#,
        );
        let original = fs::read_to_string(&form_path).unwrap();
        write_file(
            &json_path,
            r#"{
  "elements": [
    {
      "cmdBar": "ПанельДействий",
      "children": [
        {"button": "Заполнить", "type": "commandBar"}
      ]
    }
  ]
}
"#,
        );

        let mut args = Map::new();
        args.insert(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        );
        args.insert(
            "JsonPath".to_string(),
            json!(json_path.display().to_string()),
        );

        let outcome = edit_form(&args, &context);
        assert!(!outcome.ok, "{outcome:?}");
        let stderr = outcome.stderr.unwrap_or_default();
        assert!(
            stderr.contains("Element 'Заполнить' already exists in form"),
            "{stderr}"
        );
        assert_eq!(fs::read_to_string(&form_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_rejects_duplicate_element_name_inside_definition_tree() {
        let context = temp_context("edit-nested-duplicate-definition");
        let form_path = context.cwd.join("Form.xml");
        let json_path = context.cwd.join("edit.json");
        write_file(
            &form_path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.17">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems/>
	<Attributes/>
	<Commands/>
</Form>
"#,
        );
        let original = fs::read_to_string(&form_path).unwrap();
        write_file(
            &json_path,
            r#"{
  "elements": [
    {
      "cmdBar": "ПанельДействий",
      "children": [
        {"button": "Заполнить", "type": "commandBar"},
        {"button": "Заполнить", "type": "commandBar"}
      ]
    }
  ]
}
"#,
        );

        let mut args = Map::new();
        args.insert(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        );
        args.insert(
            "JsonPath".to_string(),
            json!(json_path.display().to_string()),
        );

        let outcome = edit_form(&args, &context);
        assert!(!outcome.ok, "{outcome:?}");
        let stderr = outcome.stderr.unwrap_or_default();
        assert!(
            stderr.contains("Element 'Заполнить' already exists in edit definition"),
            "{stderr}"
        );
        assert_eq!(fs::read_to_string(&form_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_keeps_child_items_self_closing_when_no_elements_are_emitted() {
        let context = temp_context("edit-empty-elements");
        let form_path = context.cwd.join("Form.xml");
        let json_path = context.cwd.join("edit.json");
        write_file(
            &form_path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.17">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems/>
	<Attributes/>
	<Commands/>
</Form>
"#,
        );
        write_file(
            &json_path,
            r#"{
  "elements": [
    {"autoCmdBar": "IgnoredAutoCommandBar"}
  ]
}
"#,
        );

        let mut args = Map::new();
        args.insert(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        );
        args.insert(
            "JsonPath".to_string(),
            json!(json_path.display().to_string()),
        );

        let outcome = edit_form(&args, &context);
        assert!(outcome.ok, "{outcome:?}");
        let updated = fs::read_to_string(&form_path).unwrap();
        assert!(updated.contains("<ChildItems/>"), "{updated}");
        assert!(
            !updated.contains("<ChildItems>\n\n</ChildItems>"),
            "{updated}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_rejects_unparseable_mutation_without_writing_file() {
        let context = temp_context("edit-invalid-generated-xml");
        let form_path = context.cwd.join("Form.xml");
        let json_path = context.cwd.join("edit.json");
        write_file(
            &form_path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.17">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems/>
	<Attributes/>
	<Commands/>
</Form>
"#,
        );
        let original = fs::read_to_string(&form_path).unwrap();
        write_file(
            &json_path,
            r#"{
  "elements": [
    {
      "check": "ФлагПроверки",
      "checkBoxType": "Bad<Name"
    }
  ]
}
"#,
        );

        let mut args = Map::new();
        args.insert(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        );
        args.insert(
            "JsonPath".to_string(),
            json!(json_path.display().to_string()),
        );

        let outcome = edit_form(&args, &context);
        assert!(!outcome.ok, "{outcome:?}");
        let stderr = outcome.stderr.unwrap_or_default();
        assert!(stderr.contains("XML parse error"), "{stderr}");
        assert_eq!(fs::read_to_string(&form_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_rejects_duplicate_command_name() {
        let context = temp_context("edit-duplicate-command");
        let form_path = context.cwd.join("Form.xml");
        let json_path = context.cwd.join("edit.json");
        write_file(
            &form_path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.17">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems/>
	<Attributes/>
	<Commands>
		<Command name="Заполнить" id="1"/>
	</Commands>
</Form>
"#,
        );
        let original = fs::read_to_string(&form_path).unwrap();
        write_file(
            &json_path,
            r#"{
  "commands": [
    {"name": "Заполнить", "action": "ЗаполнитьОбработка"}
  ]
}
"#,
        );

        let mut args = Map::new();
        args.insert(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        );
        args.insert(
            "JsonPath".to_string(),
            json!(json_path.display().to_string()),
        );

        let outcome = edit_form(&args, &context);
        assert!(!outcome.ok, "{outcome:?}");
        let stderr = outcome.stderr.unwrap_or_default();
        assert!(
            stderr.contains("Command 'Заполнить' already exists in form"),
            "{stderr}"
        );
        assert_eq!(fs::read_to_string(&form_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_emits_std_command_with_multi_dot_item_path() {
        let context = temp_context("edit-button-std-command");
        let form_path = context.cwd.join("Form.xml");
        let json_path = context.cwd.join("edit.json");
        write_file(&form_path, editable_form_xml(false));
        write_file(
            &json_path,
            r#"{
  "elements": [
    {
      "button": "AddGroupButton",
      "type": "commandBar",
      "stdCommand": "Table.Group.Add"
    }
  ]
}
"#,
        );

        let mut args = Map::new();
        args.insert(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        );
        args.insert(
            "JsonPath".to_string(),
            json!(json_path.display().to_string()),
        );

        let outcome = edit_form(&args, &context);
        assert!(outcome.ok, "{outcome:?}");
        let updated = fs::read_to_string(&form_path).unwrap();
        assert!(
            updated
                .contains("<CommandName>Form.Item.Table.Group.StandardCommand.Add</CommandName>"),
            "{updated}"
        );
        assert!(
            !updated
                .contains("<CommandName>Form.Item.Table.StandardCommand.Group.Add</CommandName>"),
            "{updated}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn validate_form_checks_form_event_against_main_attribute_context() {
        let cases = [
            (Some("CatalogObject.Goods"), true, None),
            (
                Some("DataProcessorObject.EventProbe"),
                false,
                Some("FORM_EVENT_NOT_ALLOWED"),
            ),
            (Some("DynamicList"), false, Some("FORM_EVENT_NOT_ALLOWED")),
            (None, false, Some("FORM_EVENT_CONTEXT_UNKNOWN")),
        ];

        for (main_type, expected_ok, expected_code) in cases {
            let context = temp_context("validate-form-event-context");
            let form_path = context.cwd.join("Form.xml");
            write_file(
                &form_path,
                &event_form_xml(
                    main_type,
                    r#"\t<Events>\n\t\t<Event name="OnReadAtServer">OnReadAtServer</Event>\n\t</Events>\n"#,
                    "",
                    false,
                ),
            );
            let args = Map::from_iter([(
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            )]);

            let outcome = validate_form(&args, &context);

            assert_eq!(outcome.ok, expected_ok, "{main_type:?}: {outcome:?}");
            if let Some(code) = expected_code {
                assert!(
                    outcome.errors.iter().any(|error| error.contains(code)),
                    "{main_type:?}: {:?}",
                    outcome.errors
                );
            }
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn validate_form_reports_unresolved_borrowed_event_context_without_false_failure() {
        let context = temp_context("validate-borrowed-event-context");
        let form_path = context.cwd.join("Form.xml");
        write_file(
            &form_path,
            &event_form_xml(
                None,
                r#"\t<Events>\n\t\t<Event name="OnReadAtServer" callType="Before">OnReadAtServer</Event>\n\t</Events>\n"#,
                "",
                true,
            ),
        );
        let args = Map::from_iter([(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        )]);

        let outcome = validate_form(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        let stdout = outcome.stdout.unwrap_or_default();
        assert!(stdout.contains("[WARN]"), "{stdout}");
        assert!(stdout.contains("FORM_EVENT_CONTEXT_UNKNOWN"), "{stdout}");
        assert!(stdout.contains("was not verified"), "{stdout}");

        let borrowed_reference = event_form_xml(
            None,
            r#"\t<Events>\n\t\t<Event name="OnReadAtServer" callType="Before">OnReadAtServer</Event>\n\t</Events>\n"#,
            "",
            true,
        )
        .replace(
            "\t<BaseForm version=\"2.20\"/>",
            "\t<BaseForm version=\"2.20\">Catalog.Goods.Form.ItemForm</BaseForm>",
        );
        write_file(&form_path, &borrowed_reference);
        let borrowed_reference_outcome = validate_form(&args, &context);
        assert!(
            borrowed_reference_outcome.ok,
            "{borrowed_reference_outcome:?}"
        );
        assert!(
            borrowed_reference_outcome
                .stdout
                .as_deref()
                .unwrap_or_default()
                .contains("was not verified"),
            "{borrowed_reference_outcome:?}"
        );

        let embedded_base_context = event_form_xml(
            None,
            r#"\t<Events>\n\t\t<Event name="OnReadAtServer" callType="Before">OnReadAtServer</Event>\n\t</Events>\n"#,
            "",
            true,
        )
        .replace(
            "\t<BaseForm version=\"2.20\"/>",
            concat!(
                "\t<BaseForm version=\"2.20\">\n",
                "\t\t<Attributes>\n",
                "\t\t\t<Attribute name=\"BaseObject\" id=\"1\">\n",
                "\t\t\t\t<Type><v8:Type>cfg:CatalogObject.Goods</v8:Type></Type>\n",
                "\t\t\t\t<MainAttribute>true</MainAttribute>\n",
                "\t\t\t</Attribute>\n",
                "\t\t</Attributes>\n",
                "\t</BaseForm>"
            ),
        );
        write_file(&form_path, &embedded_base_context);
        let embedded_base_outcome = validate_form(&args, &context);
        assert!(embedded_base_outcome.ok, "{embedded_base_outcome:?}");
        assert!(
            !embedded_base_outcome
                .stdout
                .as_deref()
                .unwrap_or_default()
                .contains("FORM_EVENT_CONTEXT_UNKNOWN"),
            "{embedded_base_outcome:?}"
        );

        for (index, malformed_context) in [
            event_form_xml(
                None,
                r#"\t<Events>\n\t\t<Event name="OnReadAtServer" callType="Before">OnReadAtServer</Event>\n\t</Events>\n"#,
                "",
                true,
            )
            .replace(
                "\t<Attributes/>",
                concat!(
                    "\t<Attributes>\n",
                    "\t\t<Attribute name=\"Object\" id=\"1\">\n",
                    "\t\t\t<Type/>\n",
                    "\t\t\t<MainAttribute>true</MainAttribute>\n",
                    "\t\t</Attribute>\n",
                    "\t</Attributes>"
                ),
            ),
            event_form_xml(
                None,
                r#"\t<Events>\n\t\t<Event name="OnReadAtServer" callType="Before">OnReadAtServer</Event>\n\t</Events>\n"#,
                "",
                true,
            )
            .replace(
                "\t<BaseForm version=\"2.20\"/>",
                concat!(
                    "\t<BaseForm version=\"2.20\">\n",
                    "\t\t<Attributes>\n",
                    "\t\t\t<Attribute name=\"BaseObject\" id=\"1\">\n",
                    "\t\t\t\t<Type/>\n",
                    "\t\t\t\t<MainAttribute>true</MainAttribute>\n",
                    "\t\t\t</Attribute>\n",
                    "\t\t</Attributes>\n",
                    "\t</BaseForm>"
                ),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            write_file(&form_path, &malformed_context);
            let invalid_context = validate_form(&args, &context);
            assert!(!invalid_context.ok, "case {index}: {invalid_context:?}");
            assert!(
                invalid_context
                    .errors
                    .iter()
                    .any(|error| error.contains("FORM_EVENT_CONTEXT_UNKNOWN")),
                "case {index}: {invalid_context:?}"
            );
            assert!(
                !invalid_context
                    .stdout
                    .as_deref()
                    .unwrap_or_default()
                    .contains("was not verified"),
                "case {index}: {invalid_context:?}"
            );
        }

        write_file(
            &form_path,
            &event_form_xml(
                None,
                r#"\t<Events>\n\t\t<Event name="OnReadAtServer" callType="after">OnReadAtServer</Event>\n\t</Events>\n"#,
                "",
                true,
            ),
        );
        let invalid_call_type = validate_form(&args, &context);
        assert!(!invalid_call_type.ok, "{invalid_call_type:?}");
        assert!(invalid_call_type
            .errors
            .iter()
            .any(|error| error.contains("FORM_EVENT_INVALID_CALL_TYPE")));

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn validate_form_rejects_event_for_wrong_element_kind_and_duplicates() {
        let context = temp_context("validate-element-events");
        let form_path = context.cwd.join("Form.xml");
        write_file(
            &form_path,
            &event_form_xml(
                Some("CatalogObject.Goods"),
                r#"\t<Events>\n\t\t<Event name="OnOpen">OnOpen</Event>\n\t\t<Event name="OnOpen">OnOpenAgain</Event>\n\t</Events>\n"#,
                r#"\t\t<InputField name="Name" id="1">\n\t\t\t<DataPath>Object.Name</DataPath>\n\t\t\t<Events>\n\t\t\t\t<Event name="OnCreateAtServer">NameOnCreateAtServer</Event>\n\t\t\t</Events>\n\t\t\t<ContextMenu name="NameContextMenu" id="2"/>\n\t\t\t<ExtendedTooltip name="NameExtendedTooltip" id="3"/>\n\t\t</InputField>\n"#,
                false,
            ),
        );
        let args = Map::from_iter([(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        )]);

        let outcome = validate_form(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("FORM_EVENT_DUPLICATE")),
            "{:?}",
            outcome.errors
        );
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("FORM_EVENT_NOT_ALLOWED")),
            "{:?}",
            outcome.errors
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn validate_form_rejects_table_event_without_direct_data_path() {
        let context = temp_context("validate-unbound-table-event");
        let form_path = context.cwd.join("Form.xml");
        write_file(
            &form_path,
            &event_form_xml(
                Some("CatalogObject.Goods"),
                "",
                concat!(
                    "\t\t<Table name=\"Rows\" id=\"1\">\n",
                    "\t\t\t<DataPath>   </DataPath>\n",
                    "\t\t\t<Events>\n",
                    "\t\t\t\t<Event name=\"Selection\">RowsSelection</Event>\n",
                    "\t\t\t</Events>\n",
                    "\t\t\t<ContextMenu name=\"RowsContextMenu\" id=\"2\"/>\n",
                    "\t\t\t<AutoCommandBar name=\"RowsCommandBar\" id=\"3\"/>\n",
                    "\t\t\t<ExtendedTooltip name=\"RowsTooltip\" id=\"4\"/>\n",
                    "\t\t</Table>\n"
                ),
                false,
            ),
        );
        let args = Map::from_iter([(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        )]);

        let outcome = validate_form(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.iter().any(|error| {
                error.contains("FORM_EVENT_NOT_ALLOWED")
                    && error.contains("non-empty direct DataPath")
            }),
            "{:?}",
            outcome.errors
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn validate_form_checks_root_command_bar_and_companion_event_owners() {
        let context = temp_context("validate-companion-events");
        let form_path = context.cwd.join("Form.xml");
        let xml = event_form_xml(
            Some("CatalogObject.Goods"),
            "",
            concat!(
                "\t\t<InputField name=\"Name\" id=\"1\">\n",
                "\t\t\t<ContextMenu name=\"NameContextMenu\" id=\"2\"/>\n",
                "\t\t\t<ExtendedTooltip name=\"NameTooltip\" id=\"3\">\n",
                "\t\t\t\t<Events><Event name=\"OnCreateAtServer\">BadTooltipEvent</Event></Events>\n",
                "\t\t\t</ExtendedTooltip>\n",
                "\t\t</InputField>\n"
            ),
            false,
        )
        .replace(
            "\t<AutoCommandBar name=\"FormCommandBar\" id=\"-1\"/>",
            concat!(
                "\t<AutoCommandBar name=\"FormCommandBar\" id=\"-1\">\n",
                "\t\t<Events><Event name=\"Click\">BadBarEvent</Event></Events>\n",
                "\t</AutoCommandBar>"
            ),
        );
        write_file(&form_path, &xml);
        let args = Map::from_iter([(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        )]);

        let outcome = validate_form(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        let errors = outcome.errors.join("\n");
        assert!(errors.contains("FormCommandBar"), "{errors}");
        assert!(errors.contains("NameTooltip"), "{errors}");
        assert!(
            errors.matches("FORM_EVENT_NOT_ALLOWED").count() >= 2,
            "{errors}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_adds_supported_inline_form_and_element_events() {
        let context = temp_context("edit-events-inline");
        let form_path = context.cwd.join("Form.xml");
        let original = event_form_xml(
            Some("CatalogObject.Goods"),
            "",
            r#"\t\t<InputField name="Name" id="1">\n\t\t\t<DataPath>Object.Name</DataPath>\n\t\t\t<ContextMenu name="NameContextMenu" id="2"/>\n\t\t\t<ExtendedTooltip name="NameExtendedTooltip" id="3"/>\n\t\t</InputField>\n"#,
            false,
        );
        write_file(&form_path, &original);
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({
                    "formEvents": [
                        {"name": "OnCreateAtServer", "handler": "OnCreateAtServer"}
                    ],
                    "elementEvents": [
                        {"element": "Name", "name": "OnChange", "handler": "NameOnChange"}
                    ]
                }),
            ),
        ]);

        let outcome = edit_form(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        assert_eq!(outcome.changes.len(), 1, "{outcome:?}");
        let updated = fs::read_to_string(&form_path).unwrap();
        assert_eq!(updated.matches("name=\"OnCreateAtServer\"").count(), 1);
        assert_eq!(updated.matches("name=\"OnChange\"").count(), 1);
        assert!(
            updated.contains("<Event name=\"OnChange\">NameOnChange</Event>"),
            "{updated}"
        );
        let validate_args = Map::from_iter([(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        )]);
        let validation = validate_form(&validate_args, &context);
        assert!(validation.ok, "{validation:?}");
        let info = analyze_form_info(&validate_args, &context);
        assert!(info.ok, "{info:?}");
        assert!(
            info.stdout
                .as_deref()
                .is_some_and(|stdout| stdout.contains("OnCreateAtServer -> OnCreateAtServer")),
            "{info:?}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn form_element_dispatcher_ignores_nested_table_and_page_properties() {
        let definition = json!({
            "elements": [
                {
                    "table": "Rows",
                    "commandBar": {"autofill": false},
                    "columns": []
                },
                {
                    "pages": "Tabs",
                    "children": [
                        {"page": "Main", "group": "horizontal"}
                    ]
                }
            ]
        });

        let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();

        assert!(xml.contains("<Table name=\"Rows\""), "{xml}");
        assert!(xml.contains("<Page name=\"Main\""), "{xml}");
        assert!(xml.contains("<Group>Horizontal</Group>"), "{xml}");
    }

    #[test]
    fn form_compile_rejects_non_string_element_event_handlers() {
        for definition in [
            json!({
                "elements": [{
                    "input": "Field",
                    "on": [{"event": "OnChange", "handler": 123}]
                }]
            }),
            json!({
                "elements": [{
                    "input": "Field",
                    "handlers": {"OnChange": 123},
                    "on": ["OnChange"]
                }]
            }),
        ] {
            let Err(error) = form_compile_xml(&definition, "2.20") else {
                panic!("non-string event handler must be rejected");
            };
            assert!(error.contains("FORM_EVENT_EMPTY_HANDLER"), "{error}");
        }
    }

    #[test]
    fn form_compile_rejects_table_events_without_path() {
        let definition = json!({
            "elements": [{
                "table": "Rows",
                "on": ["Selection"],
                "columns": []
            }]
        });

        let Err(error) = form_compile_xml(&definition, "2.20") else {
            panic!("an unbound Table event must be rejected");
        };
        assert!(error.contains("FORM_EVENT_NOT_ALLOWED"), "{error}");
        assert!(error.contains("non-empty path/DataPath"), "{error}");
    }

    #[test]
    fn form_compile_uses_shared_element_event_matrix() {
        let valid = json!({
            "attributes": [{"name": "Rows", "type": "ValueTable"}],
            "elements": [{
                "table": "Rows",
                "path": "Rows",
                "on": ["Selection"],
                "columns": []
            }]
        });
        let (xml, _) = form_compile_xml(&valid, "2.20").unwrap();
        assert!(
            xml.contains("<Event name=\"Selection\">RowsВыборСтроки</Event>"),
            "{xml}"
        );

        for (definition, expected_code) in [
            (
                json!({
                    "elements": [{
                        "table": "Rows",
                        "path": "Rows",
                        "on": ["OnCreateAtServer"],
                        "columns": []
                    }]
                }),
                "FORM_EVENT_NOT_ALLOWED",
            ),
            (
                json!({"elements": [{"button": "Run", "on": ["Click"]}]}),
                "FORM_EVENT_NOT_ALLOWED",
            ),
            (
                json!({
                    "elements": [{
                        "input": "Name",
                        "on": [{
                            "event": "OnChange",
                            "handler": "NameOnChange",
                            "callType": "After"
                        }]
                    }]
                }),
                "FORM_EVENT_CALL_TYPE_NOT_ALLOWED",
            ),
        ] {
            let Err(error) = form_compile_xml(&definition, "2.20") else {
                panic!("invalid element event must be rejected: {definition}");
            };
            assert!(error.contains(expected_code), "{error}");
        }
    }

    #[test]
    fn form_compile_emits_root_events_from_json_map_and_passes_validation() {
        let context = temp_context("compile-root-events");
        let definition_path = context.cwd.join("form.json");
        let form_path = context.cwd.join("Form.xml");
        write_file(
            &definition_path,
            r#"{"events":{"OnCreateAtServer":"ПриСозданииНаСервере"}}"#,
        );
        let args = Map::from_iter([
            (
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            ),
            (
                "OutputPath".to_string(),
                json!(form_path.display().to_string()),
            ),
        ]);

        let outcome = compile_form(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        let xml = read_utf8_sig(&form_path).unwrap();
        assert_eq!(xml.matches("<Events>").count(), 1, "{xml}");
        assert_eq!(xml.matches("name=\"OnCreateAtServer\"").count(), 1, "{xml}");
        assert!(
            xml.contains("<Event name=\"OnCreateAtServer\">ПриСозданииНаСервере</Event>"),
            "{xml}"
        );
        let validation_args = Map::from_iter([(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        )]);
        let validation = validate_form(&validation_args, &context);
        assert!(validation.ok, "{validation:?}");

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn form_compile_accepts_persistent_object_and_record_event_families() {
        for main_type in [
            "ChartOfAccountsObject.Main",
            "ChartOfCalculationTypesObject.Payroll",
            "AccumulationRegisterRecordSet.Stock",
            "AccountingRegisterRecordSet.Accounting",
            "CalculationRegisterRecordSet.Payroll",
        ] {
            let definition = json!({
                "attributes": [{"name": "Object", "type": main_type, "main": true}],
                "events": {"OnReadAtServer": "ObjectOnReadAtServer"}
            });

            let (xml, _) = form_compile_xml(&definition, "2.20")
                .unwrap_or_else(|error| panic!("{main_type}: {error}"));

            assert!(
                xml.contains(&format!("<v8:Type>cfg:{main_type}</v8:Type>")),
                "{main_type}: {xml}"
            );
            assert!(
                xml.contains("<Event name=\"OnReadAtServer\">ObjectOnReadAtServer</Event>"),
                "{main_type}: {xml}"
            );
        }
    }

    #[test]
    fn form_compile_rejects_invalid_root_events_before_writing() {
        let context = temp_context("compile-invalid-root-events");
        let definition_path = context.cwd.join("form.json");
        let form_path = context.cwd.join("Form.xml");
        let original = b"do-not-replace-invalid-form";
        let cases = [
            (
                "event outside root registry",
                json!({"events": {"Opening": "OnOpening"}}),
                "FORM_EVENT_NOT_ALLOWED",
            ),
            (
                "record event without main attribute",
                json!({"events": {"OnReadAtServer": "OnReadAtServer"}}),
                "FORM_EVENT_CONTEXT_UNKNOWN",
            ),
            (
                "record event with non-persistent main attribute",
                json!({
                    "attributes": [{"name": "List", "type": "DynamicList", "main": true}],
                    "events": {"OnReadAtServer": "OnReadAtServer"}
                }),
                "FORM_EVENT_NOT_ALLOWED",
            ),
            (
                "events payload is not a map",
                json!({"events": ["OnOpen"]}),
                "FORM_EVENT_NOT_ALLOWED",
            ),
            (
                "event handler is not a string",
                json!({"events": {"OnOpen": 42}}),
                "FORM_EVENT_EMPTY_HANDLER",
            ),
            (
                "map does not silently accept call type",
                json!({
                    "events": {"OnOpen": {"handler": "OnOpen", "callType": "Before"}}
                }),
                "FORM_EVENT_NOT_ALLOWED",
            ),
        ];

        for (name, definition, expected_code) in cases {
            write_file(
                &definition_path,
                &serde_json::to_string(&definition).unwrap(),
            );
            fs::write(&form_path, original).unwrap();
            let args = Map::from_iter([
                (
                    "JsonPath".to_string(),
                    json!(definition_path.display().to_string()),
                ),
                (
                    "OutputPath".to_string(),
                    json!(form_path.display().to_string()),
                ),
            ]);

            let outcome = compile_form(&args, &context);

            assert!(!outcome.ok, "{name}: {outcome:?}");
            assert!(
                outcome.errors.join("\n").contains(expected_code),
                "{name}: {outcome:?}"
            );
            assert!(outcome.changes.is_empty(), "{name}: {outcome:?}");
            assert!(outcome.artifacts.is_empty(), "{name}: {outcome:?}");
            assert!(outcome.stdout.is_none(), "{name}: {outcome:?}");
            assert_eq!(fs::read(&form_path).unwrap(), original, "{name}");
        }

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn form_compile_dry_run_rejects_invalid_root_event_without_writing() {
        let context = temp_context("compile-invalid-root-event-dry-run");
        let definition_path = context.cwd.join("form.json");
        let form_path = context.cwd.join("Form.xml");
        let original = b"do-not-replace-invalid-form";
        write_file(&definition_path, r#"{"events":{"Opening":"OnOpening"}}"#);
        fs::write(&form_path, original).unwrap();
        let args = Map::from_iter([
            (
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            ),
            (
                "OutputPath".to_string(),
                json!(form_path.display().to_string()),
            ),
        ]);

        let outcome = NativeOperationAdapter::invoke(
            "form-compile",
            "unica.form.compile",
            &args,
            &context,
            true,
            true,
        )
        .unwrap();

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("FORM_EVENT_NOT_ALLOWED")),
            "{outcome:?}"
        );
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert_eq!(fs::read(&form_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn form_compile_dry_run_plans_valid_root_event_without_writing() {
        let context = temp_context("compile-valid-root-event-dry-run");
        let definition_path = context.cwd.join("form.json");
        let form_path = context.cwd.join("Form.xml");
        let original = b"do-not-replace-valid-form-during-preview";
        write_file(
            &definition_path,
            r#"{"events":{"OnCreateAtServer":"OnCreateAtServer"}}"#,
        );
        fs::write(&form_path, original).unwrap();
        let args = Map::from_iter([
            (
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            ),
            (
                "OutputPath".to_string(),
                json!(form_path.display().to_string()),
            ),
        ]);

        let outcome = NativeOperationAdapter::invoke(
            "form-compile",
            "unica.form.compile",
            &args,
            &context,
            true,
            true,
        )
        .unwrap();

        assert!(outcome.ok, "{outcome:?}");
        assert_eq!(outcome.changes.len(), 1, "{outcome:?}");
        assert!(
            outcome.changes[0].contains("would update")
                && outcome.changes[0].contains(&form_path.display().to_string()),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&form_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn form_compile_skill_tables_document_only_supported_dsl_keys() {
        const SKILL: &str =
            include_str!("../../../../../plugins/unica/skills/form-compile/SKILL.md");
        const ELEMENTS_START: &str = "### Элементы (ключ определяет тип)\n\n";
        const START: &str = "<!-- form-event-registry:start -->";
        const END: &str = "<!-- form-event-registry:end -->";
        let skill = SKILL.replace("\r\n", "\n");

        let element_table = skill
            .split_once(ELEMENTS_START)
            .and_then(|(_, tail)| tail.split_once("\n\n### ").map(|(table, _)| table))
            .expect("form-compile element table must remain present for contract checks");
        let documented_element_keys = element_table.lines().filter_map(|line| {
            line.strip_prefix("| `\"")
                .and_then(|line| line.split_once("\"`"))
                .map(|(key, _)| key)
        });

        for key in documented_element_keys {
            let element = Map::from_iter([(key.to_string(), json!("DocumentedElement"))]);
            assert!(
                FormEditElementDefinitionKind::from_object(&element).is_ok(),
                "form-compile element table documents unsupported DSL key `{key}`"
            );
        }

        let section = skill
            .split_once(START)
            .and_then(|(_, tail)| tail.split_once(END).map(|(section, _)| section))
            .expect("form-compile event table must be delimited for contract checks");
        let documented_keys = section.lines().filter_map(|line| {
            line.strip_prefix("| `")
                .and_then(|line| line.split_once("` | "))
                .map(|(key, _)| key)
        });

        for key in documented_keys {
            let element = Map::from_iter([(key.to_string(), json!("DocumentedElement"))]);
            assert!(
                FormEditElementDefinitionKind::from_object(&element).is_ok(),
                "form-compile event table documents unsupported DSL key `{key}`"
            );
        }
    }

    #[test]
    fn edit_form_emits_and_validates_new_table_and_column_events() {
        let context = temp_context("edit-new-element-events");
        let form_path = context.cwd.join("Form.xml");
        write_file(
            &form_path,
            &event_form_xml(Some("CatalogObject.Goods"), "", "", false),
        );
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({
                    "attributes": [{"name": "Rows", "type": "ValueTable"}],
                    "elements": [{
                        "table": "Rows",
                        "path": "Rows",
                        "commandBar": {"autofill": false},
                        "on": [{"event": "Selection", "handler": "RowsSelection"}],
                        "columns": [{
                            "labelField": "Description",
                            "on": [{
                                "event": "OnChange",
                                "handler": "DescriptionOnChange"
                            }]
                        }]
                    }]
                }),
            ),
        ]);

        let outcome = edit_form(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        let updated = fs::read_to_string(&form_path).unwrap();
        assert!(
            updated.contains("<Event name=\"Selection\">RowsSelection</Event>"),
            "{updated}"
        );
        assert!(
            updated.contains("<Event name=\"OnChange\">DescriptionOnChange</Event>"),
            "{updated}"
        );
        let table_start = updated.find("<Table name=\"Rows\"").unwrap();
        let table_end = updated[table_start..].find("</Table>").unwrap() + table_start;
        let table_xml = &updated[table_start..table_end];
        let table_event_pos = table_xml.find("<Event name=\"Selection\"").unwrap();
        let child_items_pos = table_xml.find("<ChildItems>").unwrap();
        assert!(table_event_pos < child_items_pos, "{table_xml}");
        let validation = validate_form(&args, &context);
        assert!(validation.ok, "{validation:?}");

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_rejects_invalid_nested_new_element_events_atomically() {
        let cases = [
            json!({
                "elements": [{
                    "table": "Rows",
                    "columns": [{"labelField": "Description", "on": ["Opening"]}]
                }]
            }),
            json!({"elements": [{"name": "FallbackInput", "on": ["OnCurrentPageChange"]}]}),
            json!({"elements": [{"input": "Field", "button": "Button", "on": ["OnChange"]}]}),
            json!({"elements": [{"input": "Field", "on": {"event": "OnChange"}}]}),
            json!({"elements": [{"input": "Field", "handlers": "invalid", "on": ["OnChange"]}]}),
            json!({"elements": [{"input": "Field", "on": [{"event": "OnChange", "handler": 123}]}]}),
            json!({"elements": [{"input": "Field", "handlers": {"OnChange": 123}, "on": ["OnChange"]}]}),
            json!({"elements": [{"table": "Rows", "on": ["Selection"], "columns": []}]}),
        ];

        for (index, definition) in cases.into_iter().enumerate() {
            let context = temp_context(&format!("edit-invalid-new-event-{index}"));
            let form_path = context.cwd.join("Form.xml");
            let original = event_form_xml(Some("CatalogObject.Goods"), "", "", false).into_bytes();
            fs::write(&form_path, &original).unwrap();
            let args = Map::from_iter([
                (
                    "FormPath".to_string(),
                    json!(form_path.display().to_string()),
                ),
                ("definition".to_string(), definition),
            ]);

            let outcome = edit_form(&args, &context);

            assert!(!outcome.ok, "case {index}: {outcome:?}");
            assert!(outcome.changes.is_empty(), "case {index}: {outcome:?}");
            assert_eq!(fs::read(&form_path).unwrap(), original, "case {index}");
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn edit_form_emits_constants_set_for_projected_object_event_context() {
        let context = temp_context("edit-projected-constants-set-context");
        let form_path = context.cwd.join("Form.xml");
        write_file(&form_path, &event_form_xml(None, "", "", false));
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({
                    "attributes": [{
                        "name": "Constants",
                        "type": "ConstantsSet",
                        "main": true
                    }],
                    "formEvents": [{
                        "name": "OnReadAtServer",
                        "handler": "OnReadAtServer"
                    }]
                }),
            ),
        ]);

        let outcome = edit_form(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        let updated = fs::read_to_string(&form_path).unwrap();
        assert!(
            updated.contains("<v8:Type>cfg:ConstantsSet</v8:Type>"),
            "{updated}"
        );
        assert!(!updated.contains("<v8:Type>ConstantsSet</v8:Type>"));
        let validation = validate_form(&args, &context);
        assert!(validation.ok, "{validation:?}");

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_rejects_empty_main_attribute_name_before_event_planning() {
        let context = temp_context("edit-empty-main-attribute-name");
        let form_path = context.cwd.join("Form.xml");
        let base_form = concat!(
            "\t<BaseForm version=\"2.20\">\n",
            "\t\t<Attributes>\n",
            "\t\t\t<Attribute name=\"BaseObject\" id=\"1\">\n",
            "\t\t\t\t<Type><v8:Type>cfg:CatalogObject.Base</v8:Type></Type>\n",
            "\t\t\t\t<MainAttribute>true</MainAttribute>\n",
            "\t\t\t</Attribute>\n",
            "\t\t</Attributes>\n",
            "\t</BaseForm>\n"
        );
        let original = event_form_xml(None, "", "", true)
            .replace("\t<BaseForm version=\"2.20\"/>\n", base_form)
            .into_bytes();
        fs::write(&form_path, &original).unwrap();
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({
                    "attributes": [{
                        "name": "",
                        "type": "DataProcessorObject.Override",
                        "main": true
                    }],
                    "formEvents": [{
                        "name": "OnReadAtServer",
                        "handler": "OnReadAtServer"
                    }]
                }),
            ),
        ]);

        let outcome = edit_form(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("Empty attribute name")),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&form_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_projects_only_attributes_that_can_be_emitted() {
        let cases = [
            (
                json!({
                    "attributes": [{
                        "name": "Object",
                        "type": "CatalogObject.Goods",
                        "main": true
                    }],
                    "formEvents": [{
                        "name": "OnReadAtServer",
                        "handler": "OnReadAtServer"
                    }]
                }),
                true,
            ),
            (
                json!({
                    "attributes": [{"type": "CatalogObject.Goods", "main": true}],
                    "formEvents": [{
                        "name": "OnReadAtServer",
                        "handler": "OnReadAtServer"
                    }]
                }),
                false,
            ),
        ];

        for (index, (definition, expected_ok)) in cases.into_iter().enumerate() {
            let context = temp_context(&format!("edit-projected-context-{index}"));
            let form_path = context.cwd.join("Form.xml");
            let original = event_form_xml(None, "", "", false).into_bytes();
            fs::write(&form_path, &original).unwrap();
            let args = Map::from_iter([
                (
                    "FormPath".to_string(),
                    json!(form_path.display().to_string()),
                ),
                ("definition".to_string(), definition),
            ]);

            let outcome = edit_form(&args, &context);

            assert_eq!(outcome.ok, expected_ok, "case {index}: {outcome:?}");
            if expected_ok {
                let validation = validate_form(&args, &context);
                assert!(validation.ok, "case {index}: {validation:?}");
            } else {
                assert!(outcome
                    .errors
                    .iter()
                    .any(|error| { error.contains("FORM_EVENT_CONTEXT_UNKNOWN") }));
                assert_eq!(fs::read(&form_path).unwrap(), original, "case {index}");
            }
            let _ = fs::remove_dir_all(&context.cwd);
        }

        let context = temp_context("edit-existing-unknown-main-context");
        let form_path = context.cwd.join("Form.xml");
        let original = event_form_xml(None, "", "", false)
            .replace(
                "\t<Attributes/>",
                concat!(
                    "\t<Attributes>\n",
                    "\t\t<Attribute name=\"Existing\" id=\"1\">\n",
                    "\t\t\t<Type/>\n",
                    "\t\t\t<MainAttribute>true</MainAttribute>\n",
                    "\t\t</Attribute>\n",
                    "\t</Attributes>"
                ),
            )
            .into_bytes();
        fs::write(&form_path, &original).unwrap();
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({
                    "attributes": [{
                        "name": "Object",
                        "type": "CatalogObject.Goods",
                        "main": true
                    }],
                    "formEvents": [{
                        "name": "OnReadAtServer",
                        "handler": "OnReadAtServer"
                    }]
                }),
            ),
        ]);

        let outcome = edit_form(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("MainAttribute=true")));
        assert_eq!(fs::read(&form_path).unwrap(), original);
        let _ = fs::remove_dir_all(&context.cwd);

        let override_cases = [
            (
                "CatalogObject.Base",
                Some(json!("DynamicList")),
                Some("FORM_EVENT_NOT_ALLOWED"),
            ),
            ("DynamicList", Some(json!("CatalogObject.Override")), None),
            (
                "CatalogObject.Base",
                None,
                Some("FORM_EVENT_CONTEXT_UNKNOWN"),
            ),
            (
                "CatalogObject.Base",
                Some(Value::Null),
                Some("FORM_EVENT_CONTEXT_UNKNOWN"),
            ),
            (
                "CatalogObject.Base",
                Some(json!("")),
                Some("FORM_EVENT_CONTEXT_UNKNOWN"),
            ),
            (
                "CatalogObject.Base",
                Some(json!(42)),
                Some("FORM_EVENT_CONTEXT_UNKNOWN"),
            ),
        ];
        for (index, (base_type, added_type, expected_code)) in
            override_cases.into_iter().enumerate()
        {
            let context = temp_context(&format!("edit-base-context-override-{index}"));
            let form_path = context.cwd.join("Form.xml");
            let base_form = format!(
                concat!(
                    "\t<BaseForm version=\"2.20\">\n",
                    "\t\t<Attributes>\n",
                    "\t\t\t<Attribute name=\"BaseObject\" id=\"1\">\n",
                    "\t\t\t\t<Type><v8:Type>cfg:{base_type}</v8:Type></Type>\n",
                    "\t\t\t\t<MainAttribute>true</MainAttribute>\n",
                    "\t\t\t</Attribute>\n",
                    "\t\t</Attributes>\n",
                    "\t</BaseForm>\n"
                ),
                base_type = base_type
            );
            let original = event_form_xml(None, "", "", true)
                .replace("\t<BaseForm version=\"2.20\"/>\n", &base_form)
                .into_bytes();
            fs::write(&form_path, &original).unwrap();
            let mut projected_attribute = json!({
                "name": "Object",
                "main": true
            });
            if let Some(added_type) = added_type {
                projected_attribute["type"] = added_type;
            }
            let args = Map::from_iter([
                (
                    "FormPath".to_string(),
                    json!(form_path.display().to_string()),
                ),
                (
                    "definition".to_string(),
                    json!({
                        "attributes": [projected_attribute],
                        "formEvents": [{
                            "name": "OnReadAtServer",
                            "handler": "OnReadAtServer"
                        }]
                    }),
                ),
            ]);

            let outcome = edit_form(&args, &context);

            assert_eq!(
                outcome.ok,
                expected_code.is_none(),
                "case {index}: {outcome:?}"
            );
            if expected_code.is_none() {
                let validation = validate_form(&args, &context);
                assert!(validation.ok, "case {index}: {validation:?}");
            } else {
                let expected_code = expected_code.unwrap_or_default();
                assert!(
                    outcome
                        .errors
                        .iter()
                        .any(|error| error.contains(expected_code)),
                    "case {index}: {outcome:?}"
                );
                assert_eq!(fs::read(&form_path).unwrap(), original);
            }
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn edit_form_rejects_ambiguous_existing_element_target() {
        let context = temp_context("edit-ambiguous-event-target");
        let form_path = context.cwd.join("Form.xml");
        let children = concat!(
            "\t\t<InputField name=\"Duplicate\" id=\"1\"/>\n",
            "\t\t<InputField name=\"Duplicate\" id=\"2\"/>\n"
        );
        let original =
            event_form_xml(Some("CatalogObject.Goods"), "", children, false).into_bytes();
        fs::write(&form_path, &original).unwrap();
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({
                    "elementEvents": [{
                        "element": "Duplicate",
                        "name": "OnChange",
                        "handler": "DuplicateOnChange"
                    }]
                }),
            ),
        ]);

        let outcome = edit_form(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("FORM_EVENT_DUPLICATE")),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&form_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_event_success_preserves_source_byte_layout() {
        let context = temp_context("edit-event-byte-layout");
        let form_path = context.cwd.join("Form.xml");
        let text = event_form_xml(Some("CatalogObject.Goods"), "", "", false)
            .replace("encoding=\"utf-8\"", "encoding=\"UTF-8\"")
            .replace('\n', "\r\n")
            .trim_end_matches("\r\n")
            .to_string();
        let mut original = vec![0xef, 0xbb, 0xbf];
        original.extend_from_slice(text.as_bytes());
        fs::write(&form_path, &original).unwrap();
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({
                    "formEvents": [{
                        "name": "OnCreateAtServer",
                        "handler": "OnCreateAtServer"
                    }]
                }),
            ),
        ]);

        let outcome = edit_form(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        let updated = fs::read(&form_path).unwrap();
        assert!(updated.starts_with(&[0xef, 0xbb, 0xbf]));
        let content = &updated[3..];
        assert!(content.starts_with(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n"));
        assert!(!content.ends_with(b"\n"));
        assert!(content
            .iter()
            .enumerate()
            .all(|(index, byte)| *byte != b'\n' || index > 0 && content[index - 1] == b'\r'));

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_expands_self_closing_event_and_element_targets() {
        let context = temp_context("edit-self-closing-event-targets");
        let form_path = context.cwd.join("Form.xml");
        let original = event_form_xml(
            Some("CatalogObject.Goods"),
            "\t<Events/>\n",
            "\t\t<InputField name=\"Name\" id=\"1\"/>\n",
            false,
        );
        write_file(&form_path, &original);
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({
                    "formEvents": [{"name": "OnOpen", "handler": "OnOpen"}],
                    "elementEvents": [{
                        "element": "Name",
                        "name": "OnChange",
                        "handler": "NameOnChange"
                    }]
                }),
            ),
        ]);

        let outcome = edit_form(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        let updated = fs::read_to_string(&form_path).unwrap();
        assert!(updated.contains("<Events>\n\t\t<Event name=\"OnOpen\">OnOpen</Event>"));
        assert!(updated.contains("<InputField name=\"Name\" id=\"1\">"));
        assert!(updated.contains("<Event name=\"OnChange\">NameOnChange</Event>"));

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_inserts_existing_pages_event_before_child_items() {
        let context = temp_context("edit-pages-event-order");
        let form_path = context.cwd.join("Form.xml");
        let children = concat!(
            "\t\t<Pages name=\"Tabs\" id=\"1\">\n",
            "\t\t\t<ExtendedTooltip name=\"TabsTooltip\" id=\"2\"/>\n",
            "\t\t\t<ChildItems>\n",
            "\t\t\t\t<Page name=\"MainPage\" id=\"3\">\n",
            "\t\t\t\t\t<ExtendedTooltip name=\"MainPageTooltip\" id=\"4\"/>\n",
            "\t\t\t\t</Page>\n",
            "\t\t\t</ChildItems>\n",
            "\t\t</Pages>\n"
        );
        write_file(
            &form_path,
            &event_form_xml(Some("CatalogObject.Goods"), "", children, false),
        );
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({
                    "elementEvents": [{
                        "element": "Tabs",
                        "name": "OnCurrentPageChange",
                        "handler": "TabsOnCurrentPageChange"
                    }]
                }),
            ),
        ]);

        let outcome = edit_form(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        let updated = fs::read_to_string(&form_path).unwrap();
        let event_pos = updated.find("<Events>").unwrap();
        let child_items_pos = updated.rfind("<ChildItems>").unwrap();
        assert!(event_pos < child_items_pos, "{updated}");
        let validation = validate_form(&args, &context);
        assert!(validation.ok, "{validation:?}");

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_inserts_existing_table_event_before_child_items() {
        let context = temp_context("edit-table-event-order");
        let form_path = context.cwd.join("Form.xml");
        let children = concat!(
            "\t\t<Table name=\"Rows\" id=\"1\">\n",
            "\t\t\t<DataPath>Rows</DataPath>\n",
            "\t\t\t<ContextMenu name=\"RowsContextMenu\" id=\"2\"/>\n",
            "\t\t\t<AutoCommandBar name=\"RowsCommandBar\" id=\"3\"/>\n",
            "\t\t\t<ExtendedTooltip name=\"RowsTooltip\" id=\"4\"/>\n",
            "\t\t\t<ChildItems>\n",
            "\t\t\t\t<LabelField name=\"Description\" id=\"5\">\n",
            "\t\t\t\t\t<ExtendedTooltip name=\"DescriptionTooltip\" id=\"6\"/>\n",
            "\t\t\t\t</LabelField>\n",
            "\t\t\t</ChildItems>\n",
            "\t\t</Table>\n"
        );
        write_file(
            &form_path,
            &event_form_xml(Some("CatalogObject.Goods"), "", children, false),
        );
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({
                    "elementEvents": [{
                        "element": "Rows",
                        "name": "Selection",
                        "handler": "RowsSelection"
                    }]
                }),
            ),
        ]);

        let outcome = edit_form(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        let updated = fs::read_to_string(&form_path).unwrap();
        let table_start = updated.find("<Table name=\"Rows\"").unwrap();
        let table_end = updated[table_start..].find("</Table>").unwrap() + table_start;
        let table_xml = &updated[table_start..table_end];
        let event_pos = table_xml.find("<Event name=\"Selection\"").unwrap();
        let child_items_pos = table_xml.find("<ChildItems>").unwrap();
        assert!(event_pos < child_items_pos, "{table_xml}");

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_rejects_unbound_table_event_without_byte_changes() {
        let context = temp_context("edit-unbound-table-event");
        let form_path = context.cwd.join("Form.xml");
        let original = event_form_xml(
            Some("CatalogObject.Goods"),
            "",
            concat!(
                "\t\t<Table name=\"Rows\" id=\"1\">\n",
                "\t\t\t<ContextMenu name=\"RowsContextMenu\" id=\"2\"/>\n",
                "\t\t\t<AutoCommandBar name=\"RowsCommandBar\" id=\"3\"/>\n",
                "\t\t\t<ExtendedTooltip name=\"RowsTooltip\" id=\"4\"/>\n",
                "\t\t</Table>\n"
            ),
            false,
        )
        .into_bytes();
        fs::write(&form_path, &original).unwrap();
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({
                    "elementEvents": [{
                        "element": "Rows",
                        "name": "Selection",
                        "handler": "RowsSelection"
                    }]
                }),
            ),
        ]);

        let outcome = edit_form(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(
            outcome.errors.iter().any(|error| {
                error.contains("FORM_EVENT_NOT_ALLOWED")
                    && error.contains("non-empty direct DataPath")
            }),
            "{:?}",
            outcome.errors
        );
        assert_eq!(fs::read(&form_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_rejects_mixed_event_batch_without_serializer_diff() {
        let context = temp_context("edit-events-rollback");
        let form_path = context.cwd.join("Form.xml");
        let original = event_form_xml(Some("DataProcessorObject.EventProbe"), "", "", false)
            .replace("encoding=\"utf-8\"", "encoding=\"UTF-8\"")
            .replace('\n', "\r\n")
            .trim_end_matches("\r\n")
            .to_string();
        write_file(&form_path, &original);
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                serde_json::from_str(include_str!(
                    "../../../../../tests/fixtures/unica_mcp_script_parity/form-edit/invalid-events.json"
                ))
                .unwrap(),
            ),
        ]);

        let outcome = edit_form(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("FORM_EVENT_NOT_ALLOWED")),
            "{:?}",
            outcome.errors
        );
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert_eq!(fs::read_to_string(&form_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_identical_event_is_byte_exact_idempotent_noop() {
        let context = temp_context("edit-event-idempotent");
        let form_path = context.cwd.join("Form.xml");
        let original = event_form_xml(
            Some("CatalogObject.Goods"),
            r#"\t<Events>\n\t\t<Event name="OnCreateAtServer">OnCreateAtServer</Event>\n\t</Events>\n"#,
            "",
            false,
        )
        .replace("encoding=\"utf-8\"", "encoding=\"UTF-8\"")
        .replace('\n', "\r\n")
        .trim_end_matches("\r\n")
        .to_string();
        write_file(&form_path, &original);
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({
                    "formEvents": [
                        {"name": "OnCreateAtServer", "handler": "OnCreateAtServer"}
                    ]
                }),
            ),
        ]);

        let outcome = edit_form(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(
            outcome.summary.contains("no-op")
                || outcome
                    .stdout
                    .as_deref()
                    .is_some_and(|stdout| stdout.contains("no-op")),
            "{outcome:?}"
        );
        assert_eq!(fs::read_to_string(&form_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_rejects_conflicting_event_binding_without_byte_changes() {
        let context = temp_context("edit-event-conflict");
        let form_path = context.cwd.join("Form.xml");
        let original = event_form_xml(
            Some("CatalogObject.Goods"),
            r#"\t<Events>\n\t\t<Event name="OnCreateAtServer">ExistingHandler</Event>\n\t</Events>\n"#,
            "",
            false,
        )
        .into_bytes();
        fs::write(&form_path, &original).unwrap();
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({
                    "formEvents": [
                        {"name": "OnCreateAtServer", "handler": "DifferentHandler"}
                    ]
                }),
            ),
        ]);

        let outcome = edit_form(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("FORM_EVENT_BINDING_CONFLICT")),
            "{:?}",
            outcome.errors
        );
        assert_eq!(fs::read(&form_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_rejects_missing_element_event_target_atomically() {
        let context = temp_context("edit-event-missing-target");
        let form_path = context.cwd.join("Form.xml");
        let original = event_form_xml(Some("CatalogObject.Goods"), "", "", false).into_bytes();
        fs::write(&form_path, &original).unwrap();
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({
                    "elementEvents": [{
                        "element": "MissingField",
                        "name": "OnChange",
                        "handler": "MissingFieldOnChange"
                    }]
                }),
            ),
        ]);

        let outcome = edit_form(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("FORM_EVENT_TARGET_NOT_FOUND")),
            "{:?}",
            outcome.errors
        );
        assert_eq!(fs::read(&form_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn validate_form_enforces_event_call_type_scope_for_root_and_element_events() {
        let child_events = r#"\t\t<InputField name="Name" id="1">\n\t\t\t<DataPath>Object.Name</DataPath>\n\t\t\t<Events>\n\t\t\t\t<Event name="OnChange" callType="Override">NameOnChange</Event>\n\t\t\t</Events>\n\t\t\t<ContextMenu name="NameContextMenu" id="2"/>\n\t\t\t<ExtendedTooltip name="NameExtendedTooltip" id="3"/>\n\t\t</InputField>\n"#;
        let cases = [
            (
                false,
                r#"\t<Events>\n\t\t<Event name="OnOpen" callType="After">OnOpen</Event>\n\t</Events>\n"#,
                child_events,
                false,
                Some("FORM_EVENT_CALL_TYPE_NOT_ALLOWED"),
            ),
            (
                true,
                r#"\t<Events>\n\t\t<Event name="OnOpen" callType="Before">OnOpen</Event>\n\t</Events>\n"#,
                child_events,
                true,
                None,
            ),
            (
                true,
                r#"\t<Events>\n\t\t<Event name="OnOpen" callType="after">OnOpen</Event>\n\t</Events>\n"#,
                "",
                false,
                Some("FORM_EVENT_INVALID_CALL_TYPE"),
            ),
            (
                true,
                r#"\t<Events>\n\t\t<Event name="OnOpen" callType="">OnOpen</Event>\n\t</Events>\n"#,
                "",
                false,
                Some("FORM_EVENT_INVALID_CALL_TYPE"),
            ),
        ];

        for (extension, form_events, child_items, expected_ok, expected_code) in cases {
            let context = temp_context("validate-event-call-type");
            let form_path = context.cwd.join("Form.xml");
            write_file(
                &form_path,
                &event_form_xml(
                    Some("CatalogObject.Goods"),
                    form_events,
                    child_items,
                    extension,
                ),
            );
            let args = Map::from_iter([(
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            )]);

            let outcome = validate_form(&args, &context);

            assert_eq!(outcome.ok, expected_ok, "{outcome:?}");
            if let Some(code) = expected_code {
                assert!(
                    outcome.errors.iter().any(|error| error.contains(code)),
                    "{:?}",
                    outcome.errors
                );
            }
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn form_edit_dry_run_uses_event_planner_without_writing() {
        let cases = [
            ("CatalogObject.Goods", "OnCreateAtServer", true, None),
            (
                "DataProcessorObject.EventProbe",
                "OnReadAtServer",
                false,
                Some("FORM_EVENT_NOT_ALLOWED"),
            ),
        ];
        for (main_type, event, expected_ok, expected_code) in cases {
            let context = temp_context("edit-event-dry-run");
            let form_path = context.cwd.join("Form.xml");
            let original = event_form_xml(Some(main_type), "", "", false).into_bytes();
            fs::write(&form_path, &original).unwrap();
            let args = Map::from_iter([
                (
                    "FormPath".to_string(),
                    json!(form_path.display().to_string()),
                ),
                (
                    "definition".to_string(),
                    json!({"formEvents": [{"name": event, "handler": event}]}),
                ),
            ]);

            let outcome = NativeOperationAdapter::invoke(
                "form-edit",
                "unica.form.edit",
                &args,
                &context,
                true,
                true,
            )
            .unwrap();

            assert_eq!(outcome.ok, expected_ok, "{outcome:?}");
            if expected_ok {
                assert!(
                    outcome
                        .changes
                        .iter()
                        .any(|change| change.contains("would update")),
                    "{outcome:?}"
                );
            } else {
                assert!(outcome.changes.is_empty(), "{outcome:?}");
            }
            if let Some(code) = expected_code {
                assert!(
                    outcome.errors.iter().any(|error| error.contains(code)),
                    "{outcome:?}"
                );
            }
            assert_eq!(fs::read(&form_path).unwrap(), original);
            let _ = fs::remove_dir_all(&context.cwd);
        }

        let context = temp_context("edit-element-dry-run-wording");
        let form_path = context.cwd.join("Form.xml");
        let original = event_form_xml(Some("CatalogObject.Goods"), "", "", false).into_bytes();
        fs::write(&form_path, &original).unwrap();
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({"elements": [{"input": "Name"}]}),
            ),
        ]);
        let outcome = preview_form_edit(&args, &context);
        assert!(outcome.ok, "{outcome:?}");
        let stdout = outcome.stdout.unwrap_or_default();
        assert!(stdout.contains("Planned elements:"), "{stdout}");
        assert!(!stdout.contains("Added elements:"), "{stdout}");
        assert_eq!(fs::read(&form_path).unwrap(), original);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_form_writes_call_type_only_for_extension_events() {
        let child_items = r#"\t\t<InputField name="Name" id="1">\n\t\t\t<DataPath>Object.Name</DataPath>\n\t\t\t<ContextMenu name="NameContextMenu" id="2"/>\n\t\t\t<ExtendedTooltip name="NameExtendedTooltip" id="3"/>\n\t\t</InputField>\n"#;
        let definition = json!({
            "formEvents": [{
                "name": "OnOpen",
                "handler": "OnOpenAfter",
                "callType": "After"
            }],
            "elementEvents": [{
                "element": "Name",
                "name": "OnChange",
                "handler": "NameOnChangeBefore",
                "callType": "Before"
            }]
        });

        for extension in [false, true] {
            let context = temp_context("edit-event-call-type");
            let form_path = context.cwd.join("Form.xml");
            let original = event_form_xml(Some("CatalogObject.Goods"), "", child_items, extension)
                .into_bytes();
            fs::write(&form_path, &original).unwrap();
            let args = Map::from_iter([
                (
                    "FormPath".to_string(),
                    json!(form_path.display().to_string()),
                ),
                ("definition".to_string(), definition.clone()),
            ]);

            let outcome = edit_form(&args, &context);

            if extension {
                assert!(outcome.ok, "{outcome:?}");
                let updated = fs::read_to_string(&form_path).unwrap();
                assert!(updated.contains("name=\"OnOpen\" callType=\"After\""));
                assert!(updated.contains("name=\"OnChange\" callType=\"Before\""));
                let validation = validate_form(&args, &context);
                assert!(validation.ok, "{validation:?}");
            } else {
                assert!(!outcome.ok, "{outcome:?}");
                assert!(outcome
                    .errors
                    .iter()
                    .any(|error| { error.contains("FORM_EVENT_CALL_TYPE_NOT_ALLOWED") }));
                assert_eq!(fs::read(&form_path).unwrap(), original);
            }
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    fn event_form_xml(
        main_type: Option<&str>,
        form_events: &str,
        child_items: &str,
        extension: bool,
    ) -> String {
        let base_form = if extension {
            "\t<BaseForm version=\"2.20\"/>\n"
        } else {
            ""
        };
        let attributes = main_type.map_or_else(
            || "\t<Attributes/>\n".to_string(),
            |main_type| {
                format!(
                    "\t<Attributes>\n\t\t<Attribute name=\"Object\" id=\"1\">\n\t\t\t<Type><v8:Type>cfg:{main_type}</v8:Type></Type>\n\t\t\t<MainAttribute>true</MainAttribute>\n\t\t</Attribute>\n\t</Attributes>\n"
                )
            },
        );
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<Form xmlns=\"{FORM_LOGFORM_NS}\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:v8=\"{FORM_V8_NS}\" version=\"2.20\">\n{base_form}\t<AutoCommandBar name=\"FormCommandBar\" id=\"-1\"/>\n{form_events}\t<ChildItems>\n{child_items}\t</ChildItems>\n{attributes}\t<Commands/>\n</Form>\n"
        )
    }

    fn editable_form_xml(extension: bool) -> &'static str {
        if extension {
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<BaseForm>Catalog.ParityCatalog.Form.ItemForm</BaseForm>
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1">
		<Autofill>true</Autofill>
	</AutoCommandBar>
	<ChildItems>
	</ChildItems>
	<Attributes>
		<Attribute name="Object" id="1">
			<Type>CatalogObject.ParityCatalog</Type>
		</Attribute>
	</Attributes>
	<Commands>
		<Command name="Refresh" id="2"/>
	</Commands>
</Form>
"#
        } else {
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1">
		<Autofill>true</Autofill>
	</AutoCommandBar>
	<ChildItems>
	</ChildItems>
	<Attributes>
		<Attribute name="Object" id="1">
			<Type>CatalogObject.ParityCatalog</Type>
		</Attribute>
	</Attributes>
	<Commands>
		<Command name="Refresh" id="2"/>
	</Commands>
</Form>
"#
        }
    }
}
