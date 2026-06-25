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
use super::{cf::*, cfe::*, form::*, interface::*, meta::*, mxl::*, role::*, skd::*, template::*};
pub(crate) struct SubsystemInfoData {
    pub(crate) name: String,
    pub(crate) synonym: String,
    pub(crate) comment: String,
    pub(crate) include_ci: String,
    pub(crate) use_one_command: String,
    pub(crate) explanation: String,
    pub(crate) picture: String,
    pub(crate) content_items: Vec<String>,
    pub(crate) groups: Vec<(String, Vec<String>)>,
    pub(crate) child_names: Vec<String>,
    pub(crate) has_ci: bool,
}

pub(crate) struct SubsystemValidationReport {
    pub(crate) errors: usize,
    pub(crate) warnings: usize,
    pub(crate) ok_count: usize,
    pub(crate) detailed: bool,
    pub(crate) lines: Vec<String>,
}

impl SubsystemValidationReport {
    pub(crate) fn new(detailed: bool) -> Self {
        Self {
            errors: 0,
            warnings: 0,
            ok_count: 0,
            detailed,
            lines: Vec::new(),
        }
    }

    pub(crate) fn out(&mut self, msg: impl Into<String>) {
        self.lines.push(msg.into());
    }

    pub(crate) fn ok(&mut self, msg: impl AsRef<str>) {
        self.ok_count += 1;
        if self.detailed {
            self.lines.push(format!("[OK]    {}", msg.as_ref()));
        }
    }

    pub(crate) fn error(&mut self, msg: impl AsRef<str>) {
        self.errors += 1;
        self.lines.push(format!("[ERROR] {}", msg.as_ref()));
    }

    pub(crate) fn warn(&mut self, msg: impl AsRef<str>) {
        self.warnings += 1;
        self.lines.push(format!("[WARN]  {}", msg.as_ref()));
    }

    pub(crate) fn finish(mut self, sub_name: &str) -> String {
        let checks = self.ok_count + self.errors + self.warnings;
        if self.errors == 0 && self.warnings == 0 && !self.detailed {
            format!("=== Validation OK: Subsystem.{sub_name} ({checks} checks) ===")
        } else {
            self.out("");
            self.out(format!(
                "=== Result: {} errors, {} warnings ({checks} checks) ===",
                self.errors, self.warnings
            ));
            format!("{}\r\n", self.lines.join("\r\n"))
        }
    }
}

pub(crate) struct SubsystemEditModel {
    pub(crate) version: String,
    pub(crate) uuid: String,
    pub(crate) name: String,
    pub(crate) synonym: String,
    pub(crate) comment: String,
    pub(crate) include_help: String,
    pub(crate) include_ci: String,
    pub(crate) use_one_command: String,
    pub(crate) explanation: String,
    pub(crate) picture: String,
    pub(crate) content: Vec<String>,
    pub(crate) children: Vec<String>,
}

#[derive(Default)]
pub(crate) struct SubsystemEditCounters {
    pub(crate) added: usize,
    pub(crate) removed: usize,
    pub(crate) modified: usize,
}

pub(crate) fn edit_subsystem(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let edit_result = (|| -> Result<(String, Vec<PathBuf>), String> {
        let definition_file = path_arg(args, &["definitionFile", "DefinitionFile"]);
        let operation = string_arg(args, &["operation", "Operation"]);
        if definition_file.is_some() && operation.is_some() {
            return Err("Cannot use both -DefinitionFile and -Operation".to_string());
        }
        if definition_file.is_none() && operation.is_none() {
            return Err("Either -DefinitionFile or -Operation is required".to_string());
        }

        let raw_path = required_path(
            args,
            &["subsystemPath", "SubsystemPath", "path", "Path"],
            "SubsystemPath",
        )?;
        let resolved_path = resolve_subsystem_edit_xml(absolutize(raw_path, &context.cwd))?;
        let mut model = load_subsystem_edit_model(&resolved_path)?;
        let obj_name = model.name.clone();
        let operations = subsystem_edit_operations(args, &context.cwd, operation, definition_file)?;
        let mut counters = SubsystemEditCounters::default();
        let mut stdout = String::new();
        let mut artifacts = vec![resolved_path.clone()];

        stdout.push_str(&format!("[INFO] Subsystem: {obj_name}\n"));
        for (op_name, value) in operations {
            match op_name.as_str() {
                "add-content" => {
                    subsystem_edit_add_content(&mut model, &value, &mut counters, &mut stdout)?;
                }
                "remove-content" => {
                    subsystem_edit_remove_content(&mut model, &value, &mut counters, &mut stdout)?;
                }
                "add-child" => subsystem_edit_add_child(
                    &mut model,
                    &resolved_path,
                    &value,
                    &mut counters,
                    &mut stdout,
                    &mut artifacts,
                )?,
                "remove-child" => {
                    subsystem_edit_remove_child(&mut model, &value, &mut counters, &mut stdout)
                }
                "set-property" => {
                    subsystem_edit_set_property(&mut model, &value, &mut counters, &mut stdout)?
                }
                _ => return Err(format!("Unknown operation: {op_name}")),
            }
        }

        write_utf8_bom(&resolved_path, &emit_subsystem_edit_model(&model))?;
        stdout.push_str(&format!("[INFO] Saved: {}\n", resolved_path.display()));

        if !bool_arg(args, &["noValidate", "NoValidate"]) {
            stdout.push('\n');
            stdout.push_str("--- Running subsystem-validate ---\n");
            let mut validate_args = Map::new();
            validate_args.insert(
                "SubsystemPath".to_string(),
                Value::String(resolved_path.display().to_string()),
            );
            if let Some(validate_stdout) = validate_subsystem(&validate_args, context).stdout {
                stdout.push_str(&validate_stdout);
            }
        }

        stdout.push('\n');
        stdout.push_str("=== subsystem-edit summary ===\n");
        stdout.push_str(&format!("  Subsystem: {obj_name}\n"));
        stdout.push_str(&format!("  Added:     {}\n", counters.added));
        stdout.push_str(&format!("  Removed:   {}\n", counters.removed));
        stdout.push_str(&format!("  Modified:  {}\n", counters.modified));

        Ok((stdout, artifacts))
    })();

    match edit_result {
        Ok((stdout, artifacts)) => AdapterOutcome {
            ok: true,
            summary: "unica.subsystem.edit completed with native subsystem editor".to_string(),
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
            summary: "unica.subsystem.edit failed in native subsystem editor".to_string(),
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

pub(crate) fn subsystem_edit_operations(
    args: &Map<String, Value>,
    cwd: &Path,
    operation: Option<&str>,
    definition_file: Option<PathBuf>,
) -> Result<Vec<(String, Value)>, String> {
    if let Some(definition_file) = definition_file {
        let definition_file = absolutize(definition_file, cwd);
        let text = fs::read_to_string(&definition_file)
            .map_err(|err| format!("failed to read {}: {err}", definition_file.display()))?;
        let parsed: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("failed to parse {}: {err}", definition_file.display()))?;
        let items = match parsed {
            Value::Array(items) => items,
            other => vec![other],
        };
        Ok(items
            .into_iter()
            .map(|item| {
                let op_name = item
                    .get("operation")
                    .and_then(Value::as_str)
                    .unwrap_or(operation.unwrap_or(""))
                    .to_string();
                let value = item
                    .get("value")
                    .cloned()
                    .unwrap_or_else(|| Value::String(String::new()));
                (op_name, value)
            })
            .collect())
    } else {
        Ok(vec![(
            operation.unwrap_or("").to_string(),
            Value::String(
                string_arg(args, &["value", "Value"])
                    .unwrap_or_default()
                    .to_string(),
            ),
        )])
    }
}

pub(crate) fn subsystem_edit_ml_text(props: roxmltree::Node<'_, '_>, tag: &str) -> String {
    meta_info_child(props, tag)
        .and_then(|node| {
            node.descendants()
                .find(|child| role_info_element(*child, "content", None))
                .and_then(|child| child.text())
                .or_else(|| node.text())
        })
        .map(str::trim)
        .unwrap_or("")
        .to_string()
}

pub(crate) fn subsystem_edit_picture_text(props: roxmltree::Node<'_, '_>) -> String {
    meta_info_child(props, "Picture")
        .and_then(|node| {
            node.children()
                .find(|child| role_info_element(*child, "Ref", None))
                .and_then(|child| child.text())
                .or_else(|| node.text())
        })
        .map(str::trim)
        .unwrap_or("")
        .to_string()
}

pub(crate) fn subsystem_edit_value_list(value: &Value) -> Result<Vec<String>, String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.starts_with('[') {
                let parsed: Value = serde_json::from_str(trimmed)
                    .map_err(|err| format!("failed to parse value list: {err}"))?;
                subsystem_edit_array_strings(&parsed)
            } else {
                Ok(vec![text.to_string()])
            }
        }
        Value::Array(_) => subsystem_edit_array_strings(value),
        _ => Ok(vec![json_value_to_python_string(value)]),
    }
}

pub(crate) fn subsystem_edit_array_strings(value: &Value) -> Result<Vec<String>, String> {
    let Some(items) = value.as_array() else {
        return Err("value must be an array".to_string());
    };
    Ok(items.iter().map(json_value_to_python_string).collect())
}

pub(crate) fn subsystem_edit_object(value: &Value) -> Result<Value, String> {
    if value.is_object() {
        Ok(value.clone())
    } else if let Some(text) = value.as_str() {
        serde_json::from_str(text).map_err(|err| format!("failed to parse JSON value: {err}"))
    } else {
        Err("value must be a JSON object".to_string())
    }
}

pub(crate) fn subsystem_edit_add_content(
    model: &mut SubsystemEditModel,
    value: &Value,
    counters: &mut SubsystemEditCounters,
    stdout: &mut String,
) -> Result<(), String> {
    let mut existing = model.content.iter().cloned().collect::<HashSet<_>>();
    for raw in subsystem_edit_value_list(value)? {
        let item = normalize_subsystem_content_ref(&raw);
        if item != raw {
            stdout.push_str(&format!("[NORM] Content: {raw} -> {item}\n"));
        }
        if existing.contains(&item) {
            stdout.push_str(&format!("[WARN] Content already contains: {item}\n"));
            continue;
        }
        model.content.push(item.clone());
        existing.insert(item.clone());
        counters.added += 1;
        stdout.push_str(&format!("[INFO] Added content: {item}\n"));
    }
    Ok(())
}

pub(crate) fn subsystem_edit_remove_content(
    model: &mut SubsystemEditModel,
    value: &Value,
    counters: &mut SubsystemEditCounters,
    stdout: &mut String,
) -> Result<(), String> {
    for item in subsystem_edit_value_list(value)? {
        if let Some(index) = model.content.iter().position(|value| value == &item) {
            model.content.remove(index);
            counters.removed += 1;
            stdout.push_str(&format!("[INFO] Removed content: {item}\n"));
        } else {
            stdout.push_str(&format!("[WARN] Content item not found: {item}\n"));
        }
    }
    Ok(())
}

pub(crate) fn subsystem_edit_add_child(
    model: &mut SubsystemEditModel,
    resolved_path: &Path,
    value: &Value,
    counters: &mut SubsystemEditCounters,
    stdout: &mut String,
    artifacts: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let child_name = json_value_to_python_string(value);
    if model.children.iter().any(|value| value == &child_name) {
        stdout.push_str(&format!(
            "[WARN] ChildObjects already contains: {child_name}\n"
        ));
        return Ok(());
    }

    model.children.push(child_name.clone());
    counters.added += 1;
    stdout.push_str(&format!("[INFO] Added child subsystem: {child_name}\n"));

    let parent_dir = resolved_path.parent().unwrap_or_else(|| Path::new(""));
    let parent_base = resolved_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let child_subs_dir = parent_dir.join(parent_base).join("Subsystems");
    if !child_subs_dir.exists() {
        fs::create_dir_all(&child_subs_dir)
            .map_err(|err| format!("failed to create {}: {err}", child_subs_dir.display()))?;
        stdout.push_str(&format!(
            "[INFO] Created directory: {}\n",
            child_subs_dir.display()
        ));
    }
    let child_xml = child_subs_dir.join(format!("{child_name}.xml"));
    if !child_xml.exists() {
        write_child_subsystem_stub(&child_xml, &child_name, &model.version, 71)?;
        stdout.push_str(&format!("[INFO] Created stub: {}\n", child_xml.display()));
        artifacts.push(child_xml);
    }
    Ok(())
}

pub(crate) fn subsystem_edit_remove_child(
    model: &mut SubsystemEditModel,
    value: &Value,
    counters: &mut SubsystemEditCounters,
    stdout: &mut String,
) {
    let child_name = json_value_to_python_string(value);
    if let Some(index) = model.children.iter().position(|value| value == &child_name) {
        model.children.remove(index);
        counters.removed += 1;
        stdout.push_str(&format!("[INFO] Removed child subsystem: {child_name}\n"));
    } else {
        stdout.push_str(&format!("[WARN] Child subsystem not found: {child_name}\n"));
    }
}

pub(crate) fn subsystem_edit_set_property(
    model: &mut SubsystemEditModel,
    value: &Value,
    counters: &mut SubsystemEditCounters,
    stdout: &mut String,
) -> Result<(), String> {
    let value = subsystem_edit_object(value)?;
    let prop_name = value
        .get("name")
        .map(json_value_to_python_string)
        .ok_or_else(|| "set-property requires {name, value}".to_string())?;
    let prop_value = value
        .get("value")
        .map(json_value_to_python_string)
        .unwrap_or_default();
    match prop_name.as_str() {
        "IncludeInCommandInterface" => {
            model.include_ci = prop_value.to_lowercase();
            counters.modified += 1;
            stdout.push_str(&format!("[INFO] Set {prop_name} = {prop_value}\n"));
        }
        "UseOneCommand" => {
            model.use_one_command = prop_value.to_lowercase();
            counters.modified += 1;
            stdout.push_str(&format!("[INFO] Set {prop_name} = {prop_value}\n"));
        }
        "IncludeHelpInContents" => {
            model.include_help = prop_value.to_lowercase();
            counters.modified += 1;
            stdout.push_str(&format!("[INFO] Set {prop_name} = {prop_value}\n"));
        }
        "Synonym" => {
            model.synonym = prop_value.clone();
            counters.modified += 1;
            if prop_value.is_empty() {
                stdout.push_str("[INFO] Cleared Synonym\n");
            } else {
                stdout.push_str(&format!("[INFO] Set Synonym = \"{prop_value}\"\n"));
            }
        }
        "Explanation" => {
            model.explanation = prop_value.clone();
            counters.modified += 1;
            if prop_value.is_empty() {
                stdout.push_str("[INFO] Cleared Explanation\n");
            } else {
                stdout.push_str(&format!("[INFO] Set Explanation = \"{prop_value}\"\n"));
            }
        }
        "Comment" => {
            model.comment = prop_value.clone();
            counters.modified += 1;
            stdout.push_str(&format!("[INFO] Set Comment = \"{prop_value}\"\n"));
        }
        "Picture" => {
            model.picture = prop_value.clone();
            counters.modified += 1;
            stdout.push_str(&format!("[INFO] Set Picture = \"{prop_value}\"\n"));
        }
        "Name" => {
            model.name = prop_value.clone();
            counters.modified += 1;
            stdout.push_str(&format!("[INFO] Set Name = \"{prop_value}\"\n"));
        }
        _ => {
            return Err(format!("Property '{prop_name}' not found in Properties"));
        }
    }
    Ok(())
}

pub(crate) fn validate_subsystem(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let result = (|| -> Result<(bool, String, PathBuf, Option<PathBuf>, String), String> {
        let raw_path = required_path(
            args,
            &["subsystemPath", "SubsystemPath", "path", "Path"],
            "SubsystemPath",
        )?;
        let path = absolutize(raw_path, &context.cwd);
        let detailed = bool_arg(args, &["detailed", "Detailed"]);
        let out_file =
            path_arg(args, &["outFile", "OutFile"]).map(|path| absolutize(path, &context.cwd));
        let xml_path = match resolve_subsystem_validate_xml(path) {
            Ok(path) => path,
            Err(stdout) => {
                return Ok((
                    false,
                    format!("{stdout}\n"),
                    PathBuf::new(),
                    out_file,
                    String::new(),
                ));
            }
        };

        let mut report = SubsystemValidationReport::new(detailed);
        let text = fs::read_to_string(&xml_path)
            .map_err(|err| format!("failed to read {}: {err}", xml_path.display()))?;
        let doc = match Document::parse(text.trim_start_matches('\u{feff}')) {
            Ok(doc) => doc,
            Err(err) => {
                report.error(format!("1. XML parse error: {err}"));
                let result = report.finish("");
                return Ok((false, result, xml_path, out_file, String::new()));
            }
        };

        let root = doc.root_element();
        let version = root.attribute("version").unwrap_or("");
        let Some(sub) = root.children().find(|node| {
            role_info_element(*node, "Subsystem", Some("http://v8.1c.ru/8.3/MDClasses"))
        }) else {
            report.error("1. Root structure: expected MetaDataObject/Subsystem, not found");
            let result = report.finish("");
            return Ok((false, result, xml_path, out_file, String::new()));
        };
        let uuid_val = sub.attribute("uuid").unwrap_or("");
        if !uuid_val.is_empty() && is_valid_uuid(uuid_val) {
            report.ok(format!(
                "1. Root structure: MetaDataObject/Subsystem, uuid={uuid_val}, version {version}"
            ));
        } else {
            report.error("1. Root structure: invalid or missing uuid");
        }

        let Some(props) = sub.children().find(|node| {
            role_info_element(*node, "Properties", Some("http://v8.1c.ru/8.3/MDClasses"))
        }) else {
            report.error("2. Properties: <Properties> element not found");
            let result = report.finish("");
            return Ok((false, result, xml_path, out_file, String::new()));
        };

        let required_props = [
            "Name",
            "Synonym",
            "Comment",
            "IncludeHelpInContents",
            "IncludeInCommandInterface",
            "UseOneCommand",
            "Explanation",
            "Picture",
            "Content",
        ];
        let missing = required_props
            .iter()
            .filter(|prop| {
                props.children().all(|node| {
                    !role_info_element(node, prop, Some("http://v8.1c.ru/8.3/MDClasses"))
                })
            })
            .copied()
            .collect::<Vec<_>>();
        if missing.is_empty() {
            report.ok("2. Properties: all 9 required properties present");
        } else {
            report.error(format!("2. Properties: missing: {}", missing.join(", ")));
        }

        let sub_name = child_text(props, "Name", Some("http://v8.1c.ru/8.3/MDClasses"))
            .trim()
            .to_string();
        let header = format!("=== Validation: Subsystem.{sub_name} ===");
        report.out("");
        report.out(header.clone());
        report.lines.insert(0, String::new());
        report.lines.insert(0, header);

        if !sub_name.is_empty() && is_1c_identifier(&sub_name) {
            report.ok(format!("3. Name: \"{sub_name}\" - valid identifier"));
        } else if sub_name.is_empty() {
            report.error("3. Name: empty");
        } else {
            report.error(format!("3. Name: \"{sub_name}\" - invalid identifier"));
        }

        if let Some(syn) = props
            .children()
            .find(|node| role_info_element(*node, "Synonym", Some("http://v8.1c.ru/8.3/MDClasses")))
        {
            let items = syn
                .children()
                .filter(|node| role_info_element(*node, "item", None));
            let mut item_count = 0usize;
            let mut first_content = String::new();
            for item in items {
                item_count += 1;
                if first_content.is_empty() {
                    if let Some(content) = item
                        .children()
                        .find(|node| role_info_element(*node, "content", None))
                        .and_then(|node| node.text())
                    {
                        if !content.is_empty() {
                            first_content = content.to_string();
                        }
                    }
                }
            }
            if item_count > 0 {
                report.ok(format!(
                    "4. Synonym: \"{first_content}\" ({item_count} lang(s))"
                ));
            } else {
                report.warn("4. Synonym: element exists but no v8:item children");
            }
        } else {
            report.warn("4. Synonym: empty or missing");
        }

        let mut bool_ok = true;
        let mut use_one = String::new();
        for prop in [
            "IncludeHelpInContents",
            "IncludeInCommandInterface",
            "UseOneCommand",
        ] {
            if let Some(node) = props
                .children()
                .find(|node| role_info_element(*node, prop, Some("http://v8.1c.ru/8.3/MDClasses")))
            {
                let value = node.text().unwrap_or("").trim().to_string();
                if prop == "UseOneCommand" {
                    use_one = value.clone();
                }
                if value != "true" && value != "false" {
                    report.error(format!(
                        "5. Boolean property {prop} = \"{value}\" (expected true/false)"
                    ));
                    bool_ok = false;
                }
            }
        }
        if bool_ok {
            report.ok("5. Boolean properties: valid");
        }

        let mut content_items = Vec::<String>::new();
        if let Some(content) = props
            .children()
            .find(|node| role_info_element(*node, "Content", Some("http://v8.1c.ru/8.3/MDClasses")))
        {
            let xr_items = content
                .children()
                .filter(|node| role_info_element(*node, "Item", None))
                .collect::<Vec<_>>();
            if xr_items.is_empty() {
                report.ok("6. Content: empty (no items)");
            } else {
                let mut content_ok = true;
                for item in &xr_items {
                    let type_attr = attribute_by_local_name(*item, "type").unwrap_or("");
                    let text = item.text().unwrap_or("").trim().to_string();
                    content_items.push(text.clone());
                    if type_attr != "xr:MDObjectRef" {
                        report.error(format!(
                            "6. Content item \"{text}\": xsi:type=\"{type_attr}\" (expected xr:MDObjectRef)"
                        ));
                        content_ok = false;
                    }
                    if !is_subsystem_content_ref(&text) && !is_valid_uuid(&text) {
                        report.error(format!(
                            "6. Content item \"{text}\": invalid format (expected Type.Name or UUID)"
                        ));
                        content_ok = false;
                    }
                    if let Some((prefix, _)) = text.split_once('.') {
                        if subsystem_known_plural_types().contains(&prefix) {
                            report.error(format!(
                                "6. Content item \"{text}\": uses plural form \"{prefix}\" (platform requires singular, e.g. Catalog not Catalogs)"
                            ));
                            content_ok = false;
                        }
                    }
                }
                if content_ok {
                    report.ok(format!(
                        "6. Content: {} items, all valid MDObjectRef format",
                        xr_items.len()
                    ));
                }
            }
        } else {
            report.ok("6. Content: empty (no items)");
        }

        if !content_items.is_empty() {
            let duplicates = duplicates_preserve_order(&content_items);
            if duplicates.is_empty() {
                report.ok("7. Content: no duplicates");
            } else {
                report.warn(format!(
                    "7. Content: duplicates found: {}",
                    duplicates.join(", ")
                ));
            }
        }

        let mut child_names = Vec::<String>::new();
        if let Some(child_objs) = sub.children().find(|node| {
            role_info_element(*node, "ChildObjects", Some("http://v8.1c.ru/8.3/MDClasses"))
        }) {
            let children = child_objs
                .children()
                .filter(|node| node.is_element())
                .collect::<Vec<_>>();
            if children.is_empty() {
                report.ok("8. ChildObjects: empty (leaf subsystem)");
            } else {
                let mut child_ok = true;
                for child in children {
                    let local = child.tag_name().name();
                    if local != "Subsystem" {
                        report.error(format!("8. ChildObjects: unexpected element <{local}>"));
                        child_ok = false;
                    } else {
                        let text = child.text().unwrap_or("").trim();
                        if text.is_empty() {
                            report.error("8. ChildObjects: empty <Subsystem> element");
                            child_ok = false;
                        } else {
                            child_names.push(text.to_string());
                        }
                    }
                }
                if child_ok {
                    report.ok(format!(
                        "8. ChildObjects: {} entries, all non-empty",
                        child_names.len()
                    ));
                }
            }
        } else {
            report.ok("8. ChildObjects: empty (leaf subsystem)");
        }

        if !child_names.is_empty() {
            let duplicates = duplicates_preserve_order(&child_names);
            if duplicates.is_empty() {
                report.ok("9. ChildObjects: no duplicates");
            } else {
                report.error(format!(
                    "9. ChildObjects: duplicates: {}",
                    duplicates.join(", ")
                ));
            }

            let subs_dir = subsystem_dir_for_xml(&xml_path).join("Subsystems");
            let missing_files = child_names
                .iter()
                .filter(|name| !subs_dir.join(format!("{name}.xml")).exists())
                .cloned()
                .collect::<Vec<_>>();
            if missing_files.is_empty() {
                report.ok(format!(
                    "10. ChildObjects files: all {} files exist",
                    child_names.len()
                ));
            } else {
                report.warn(format!(
                    "10. ChildObjects files: missing: {}",
                    missing_files.join(", ")
                ));
            }
        }

        let ci_path = subsystem_dir_for_xml(&xml_path)
            .join("Ext")
            .join("CommandInterface.xml");
        if ci_path.exists() {
            match fs::read_to_string(&ci_path)
                .map_err(|err| format!("failed to read {}: {err}", ci_path.display()))
                .and_then(|text| {
                    Document::parse(text.trim_start_matches('\u{feff}'))
                        .map(|_| ())
                        .map_err(|err| format!("{err}"))
                }) {
                Ok(()) => report.ok("11. CommandInterface: exists, well-formed"),
                Err(err) => report.warn(format!(
                    "11. CommandInterface: exists but NOT well-formed: {err}"
                )),
            }
        } else {
            report.ok("11. CommandInterface: not present");
        }

        if let Some(picture) = props
            .children()
            .find(|node| role_info_element(*node, "Picture", Some("http://v8.1c.ru/8.3/MDClasses")))
        {
            let children = picture
                .children()
                .filter(|node| node.is_element())
                .collect::<Vec<_>>();
            if children.is_empty() {
                report.ok("12. Picture: empty (not set)");
            } else if let Some(pic_ref) = children
                .iter()
                .find(|node| role_info_element(**node, "Ref", None))
            {
                let ref_text = pic_ref.text().unwrap_or("");
                if ref_text.starts_with("CommonPicture.") {
                    report.ok(format!("12. Picture: {ref_text}"));
                } else {
                    report.warn(format!(
                        "12. Picture: \"{ref_text}\" (expected CommonPicture.XXX)"
                    ));
                }
            } else {
                report.warn("12. Picture: has children but no xr:Ref content");
            }
        } else {
            report.ok("12. Picture: empty (not set)");
        }

        if use_one == "true" {
            if content_items.len() == 1 {
                report.ok("13. UseOneCommand: true, Content has exactly 1 item");
            } else {
                report.warn(format!(
                    "13. UseOneCommand: true but Content has {} items (expected 1)",
                    content_items.len()
                ));
            }
        } else {
            report.ok("13. UseOneCommand: false (no constraint)");
        }

        let ok = report.errors == 0;
        let result = report.finish(&sub_name);
        Ok((ok, result, xml_path, out_file, String::new()))
    })();

    match result {
        Ok((ok, text, artifact, out_file, error_slot)) => {
            let mut stdout = text.clone();
            let mut artifacts = if artifact.as_os_str().is_empty() {
                Vec::new()
            } else {
                vec![artifact.display().to_string()]
            };
            if let Some(out_file) = out_file {
                if let Err(error) = write_utf8_bom(&out_file, &text) {
                    return AdapterOutcome {
                        ok: false,
                        summary: "unica.subsystem.validate failed in native subsystem validator"
                            .to_string(),
                        changes: Vec::new(),
                        warnings: Vec::new(),
                        errors: vec![error.clone()],
                        artifacts: Vec::new(),
                        stdout: None,
                        stderr: Some(format!("{error}\n")),
                        command: None,
                    };
                }
                stdout.push_str(&format!("Written to: {}\n", out_file.display()));
                artifacts.push(out_file.display().to_string());
            }
            AdapterOutcome {
                ok,
                summary: if ok {
                    "unica.subsystem.validate completed with native subsystem validator".to_string()
                } else {
                    "unica.subsystem.validate failed in native subsystem validator".to_string()
                },
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: if ok { Vec::new() } else { vec![error_slot] },
                artifacts,
                stdout: Some(stdout),
                stderr: Some(String::new()),
                command: None,
            }
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.subsystem.validate failed in native subsystem validator".to_string(),
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

pub(crate) fn analyze_subsystem_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let result = (|| -> Result<(String, Option<PathBuf>, PathBuf), String> {
        let raw_path = required_path(
            args,
            &["subsystemPath", "SubsystemPath", "path", "Path"],
            "SubsystemPath",
        )?;
        let path = absolutize(raw_path, &context.cwd);
        let mode = string_arg(args, &["mode", "Mode"]).unwrap_or("overview");
        let name_filter = string_arg(args, &["name", "Name"]).unwrap_or("");
        let out_file =
            path_arg(args, &["outFile", "OutFile"]).map(|path| absolutize(path, &context.cwd));

        let (mut lines, artifact) = match mode {
            "tree" => subsystem_info_tree(&path, name_filter)?,
            "ci" => {
                if path.is_dir() {
                    return Err(
                        "[ERROR] ci mode requires a subsystem .xml file, not a directory"
                            .to_string(),
                    );
                }
                let xml_path = resolve_subsystem_info_xml(path, false)?;
                let (data, _) = load_subsystem_info_data(&xml_path)?;
                (subsystem_info_ci_lines(&data.name, &xml_path)?, xml_path)
            }
            "overview" | "content" | "full" => {
                let xml_path = resolve_subsystem_info_xml(path, true)?;
                let (data, _) = load_subsystem_info_data(&xml_path)?;
                let mut lines = Vec::<String>::new();
                match mode {
                    "overview" => {
                        append_subsystem_overview(&mut lines, &data);
                        lines.insert(
                            1,
                            format!("Поддержка: {}", support_status_for_path(&xml_path)),
                        );
                    }
                    "content" => append_subsystem_content(&mut lines, &data, name_filter),
                    "full" => {
                        append_subsystem_overview(&mut lines, &data);
                        lines.insert(
                            1,
                            format!("Поддержка: {}", support_status_for_path(&xml_path)),
                        );
                        lines.push(String::new());
                        lines.push("--- content ---".to_string());
                        lines.push(String::new());
                        append_subsystem_content(&mut lines, &data, name_filter);
                        lines.push(String::new());
                        lines.push("--- ci ---".to_string());
                        lines.push(String::new());
                        lines.extend(subsystem_info_ci_lines(&data.name, &xml_path)?);
                    }
                    _ => unreachable!(),
                }
                (lines, xml_path)
            }
            other => {
                return Err(format!(
                    "argument -Mode: invalid choice: '{other}' (choose from 'overview', 'content', 'ci', 'tree', 'full')"
                ));
            }
        };

        if let Some(stdout) = paginate_subsystem_info(&mut lines, args) {
            return Ok((stdout, None, artifact));
        }

        if let Some(out_file) = out_file {
            write_utf8_bom(&out_file, &lines.join("\n"))?;
            Ok((
                format!("Output written to {}\n", out_file.display()),
                Some(out_file),
                artifact,
            ))
        } else {
            Ok((format!("{}\n", lines.join("\n")), None, artifact))
        }
    })();

    match result {
        Ok((stdout, out_file, artifact)) => {
            let mut artifacts = vec![artifact.display().to_string()];
            if let Some(out_file) = out_file {
                artifacts.push(out_file.display().to_string());
            }
            AdapterOutcome {
                ok: true,
                summary: "unica.subsystem.info completed with native subsystem analyzer"
                    .to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts,
                stdout: Some(stdout),
                stderr: Some(String::new()),
                command: None,
            }
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.subsystem.info failed in native subsystem analyzer".to_string(),
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

pub(crate) fn subsystem_info_ci_lines(
    sub_name: &str,
    subsystem_path: &Path,
) -> Result<Vec<String>, String> {
    let sub_dir = subsystem_dir_for_xml(subsystem_path);
    let ci_path = sub_dir.join("Ext").join("CommandInterface.xml");
    let mut lines = vec![format!("Командный интерфейс: {sub_name}"), String::new()];
    if !ci_path.is_file() {
        lines.push("Файл CommandInterface.xml не найден.".to_string());
        lines.push(format!("Путь: {}", ci_path.display()));
        return Ok(lines);
    }

    let text = fs::read_to_string(&ci_path)
        .map_err(|err| format!("failed to read {}: {err}", ci_path.display()))?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error in {}: {err}", ci_path.display()))?;
    let root = doc.root_element();
    const CI_NS: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";

    if let Some(section) = root
        .children()
        .find(|node| role_info_element(*node, "CommandsVisibility", Some(CI_NS)))
    {
        let mut hidden = Vec::new();
        let mut shown = Vec::new();
        for cmd in section
            .children()
            .filter(|node| role_info_element(*node, "Command", Some(CI_NS)))
        {
            let name = cmd.attribute("name").unwrap_or("").to_string();
            let common = cmd
                .descendants()
                .find(|node| role_info_element(*node, "Common", None))
                .and_then(|node| node.text());
            if common == Some("false") {
                hidden.push(name);
            } else {
                shown.push(name);
            }
        }
        let total = hidden.len() + shown.len();
        lines.push(format!("Видимость ({total}):"));
        if !hidden.is_empty() {
            lines.push(format!("  СКРЫТО ({}):", hidden.len()));
            for item in hidden {
                lines.push(format!("    {item}"));
            }
        }
        if !shown.is_empty() {
            lines.push(format!("  ПОКАЗАНО ({}):", shown.len()));
            for item in shown {
                lines.push(format!("    {item}"));
            }
        }
        lines.push(String::new());
    }

    if let Some(section) = root
        .children()
        .find(|node| role_info_element(*node, "CommandsPlacement", Some(CI_NS)))
    {
        let placements = section
            .children()
            .filter(|node| role_info_element(*node, "Command", Some(CI_NS)))
            .map(|cmd| {
                let name = cmd.attribute("name").unwrap_or("");
                let group = child_text(cmd, "CommandGroup", Some(CI_NS));
                let placement = child_text(cmd, "Placement", Some(CI_NS));
                format!(
                    "  {name} → {} ({})",
                    if group.is_empty() { "?" } else { &group },
                    if placement.is_empty() {
                        "?"
                    } else {
                        &placement
                    }
                )
            })
            .collect::<Vec<_>>();
        if !placements.is_empty() {
            lines.push(format!("Размещение ({}):", placements.len()));
            lines.extend(placements);
            lines.push(String::new());
        }
    }

    if let Some(section) = root
        .children()
        .find(|node| role_info_element(*node, "CommandsOrder", Some(CI_NS)))
    {
        let mut groups = Vec::<(String, Vec<String>)>::new();
        for cmd in section
            .children()
            .filter(|node| role_info_element(*node, "Command", Some(CI_NS)))
        {
            let name = cmd.attribute("name").unwrap_or("").to_string();
            let group = child_text(cmd, "CommandGroup", Some(CI_NS));
            push_group_item(
                &mut groups,
                if group.is_empty() { "?" } else { &group },
                name,
            );
        }
        let total = groups.iter().map(|(_, items)| items.len()).sum::<usize>();
        if total > 0 {
            lines.push(format!("Порядок команд ({total}):"));
            for (group, items) in groups {
                lines.push(format!("  [{group}]:"));
                for item in items {
                    lines.push(format!("    {item}"));
                }
            }
            lines.push(String::new());
        }
    }

    Ok(lines)
}

pub(crate) fn subsystem_info_tree(
    path: &Path,
    name_filter: &str,
) -> Result<(Vec<String>, PathBuf), String> {
    let mut lines = Vec::<String>::new();
    if path.is_dir() {
        let label = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        lines.push(format!("Дерево подсистем от: {label}/"));
        lines.push(String::new());
        let mut files = fs::read_dir(path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|entry| {
                entry.is_file()
                    && entry
                        .extension()
                        .and_then(|value| value.to_str())
                        .map(|ext| ext.eq_ignore_ascii_case("xml"))
                        .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        files.sort_by_key(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_lowercase()
        });
        if !name_filter.is_empty() {
            files.retain(|path| {
                path.file_stem().and_then(|value| value.to_str()) == Some(name_filter)
            });
            if files.is_empty() {
                return Err(format!(
                    "[ERROR] Subsystem '{name_filter}' not found in {}",
                    path.display()
                ));
            }
        }
        for (index, file) in files.iter().enumerate() {
            build_subsystem_tree_entry(file, "", index == files.len() - 1, true, &mut lines)?;
        }
        Ok((lines, path.to_path_buf()))
    } else {
        if !path.is_file() {
            return Err(format!("[ERROR] File not found: {}", path.display()));
        }
        build_subsystem_tree_entry(path, "", true, true, &mut lines)?;
        Ok((lines, path.to_path_buf()))
    }
}

pub(crate) fn subsystem_dir_for_xml(xml_path: &Path) -> PathBuf {
    let dir = xml_path.parent().unwrap_or_else(|| Path::new(""));
    let base = xml_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    dir.join(base)
}

pub(crate) fn subsystem_content_items(props: roxmltree::Node<'_, '_>) -> Vec<String> {
    props
        .children()
        .find(|node| role_info_element(*node, "Content", Some("http://v8.1c.ru/8.3/MDClasses")))
        .map(|content| {
            content
                .children()
                .filter(|node| role_info_element(*node, "Item", None))
                .filter_map(|node| node.text().map(ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(crate) fn subsystem_child_names(sub: roxmltree::Node<'_, '_>) -> Vec<String> {
    sub.children()
        .find(|node| {
            role_info_element(*node, "ChildObjects", Some("http://v8.1c.ru/8.3/MDClasses"))
        })
        .map(|children| {
            children
                .children()
                .filter(|node| role_info_element(*node, "Subsystem", None))
                .map(|node| node.text().unwrap_or("").to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(crate) fn subsystem_group_content(items: &[String]) -> Vec<(String, Vec<String>)> {
    let mut groups = Vec::<(String, Vec<String>)>::new();
    for item in items {
        let (type_name, name) = if let Some((type_name, name)) = item.split_once('.') {
            (type_name.to_string(), name.to_string())
        } else if looks_like_uuid_prefix(item) {
            ("[UUID]".to_string(), item.clone())
        } else {
            ("[Other]".to_string(), item.clone())
        };
        push_group_item(&mut groups, &type_name, name);
    }
    groups
}

pub(crate) fn subsystem_known_plural_types() -> &'static [&'static str] {
    &[
        "Catalogs",
        "Documents",
        "Enums",
        "Constants",
        "Reports",
        "DataProcessors",
        "InformationRegisters",
        "AccumulationRegisters",
        "AccountingRegisters",
        "CalculationRegisters",
        "ChartsOfAccounts",
        "ChartsOfCharacteristicTypes",
        "ChartsOfCalculationTypes",
        "BusinessProcesses",
        "Tasks",
        "ExchangePlans",
        "DocumentJournals",
        "CommonModules",
        "CommonCommands",
        "CommonForms",
        "CommonPictures",
        "CommonTemplates",
        "CommonAttributes",
        "CommandGroups",
        "Roles",
        "SessionParameters",
        "FilterCriteria",
        "XDTOPackages",
        "WebServices",
        "HTTPServices",
        "WSReferences",
        "EventSubscriptions",
        "ScheduledJobs",
        "SettingsStorages",
        "FunctionalOptions",
        "FunctionalOptionsParameters",
        "DefinedTypes",
        "DocumentNumerators",
        "Sequences",
        "Subsystems",
        "StyleItems",
        "IntegrationServices",
    ]
}

pub(crate) fn compile_subsystem(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let write_result = (|| -> Result<(String, Vec<PathBuf>), String> {
        let definition_file = path_arg(args, &["definitionFile", "DefinitionFile"]);
        let value_arg = string_arg(args, &["value", "Value"]);
        if definition_file.is_some() && value_arg.is_some() {
            return Err("Cannot use both -DefinitionFile and -Value".to_string());
        }
        if definition_file.is_none() && value_arg.is_none() {
            return Err("Either -DefinitionFile or -Value is required".to_string());
        }

        let json_text = if let Some(definition_file) = definition_file {
            let definition_file = absolutize(definition_file, &context.cwd);
            if !definition_file.exists() {
                return Err(format!(
                    "Definition file not found: {}",
                    definition_file.display()
                ));
            }
            fs::read_to_string(&definition_file)
                .map_err(|err| format!("failed to read {}: {err}", definition_file.display()))?
        } else {
            value_arg.unwrap_or_default().to_string()
        };
        let defn: Value = serde_json::from_str(json_text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("failed to parse subsystem JSON: {err}"))?;

        let obj_name = json_string_field(&defn, "name")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "JSON must have 'name' field".to_string())?;

        let output_dir = required_path(args, &["outputDir", "OutputDir"], "OutputDir")
            .map(|path| absolutize(path, &context.cwd))?;
        let format_version = detect_format_version(&output_dir);

        let synonym = json_string_field(&defn, "synonym")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| split_camel_case(&obj_name));
        let comment = json_string_field(&defn, "comment").unwrap_or_default();
        let include_help_in_contents = "true".to_string();
        let include_in_ci = defn
            .get("includeInCommandInterface")
            .map(json_value_to_python_lower)
            .unwrap_or_else(|| "true".to_string());
        let use_one_command = defn
            .get("useOneCommand")
            .map(json_value_to_python_lower)
            .unwrap_or_else(|| "false".to_string());
        let explanation = json_string_field(&defn, "explanation").unwrap_or_default();
        let picture = json_string_field(&defn, "picture").unwrap_or_default();
        let subsystem_uuid = json_string_field(&defn, "uuid")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(fresh_uuid);

        let mut stdout = String::new();
        let mut normalized_count = 0usize;
        let mut content_items = Vec::new();
        if let Some(content) = defn.get("content").or_else(|| defn.get("objects")) {
            if let Some(items) = content.as_array() {
                for item in items {
                    let raw = json_value_to_python_string(item);
                    let normalized = normalize_subsystem_content_ref(&raw);
                    if normalized != raw {
                        stdout.push_str(&format!("[NORM] Content: {raw} -> {normalized}\n"));
                        normalized_count += 1;
                    }
                    content_items.push(normalized);
                }
            }
        }
        if normalized_count > 0 {
            stdout.push_str(&format!(
                "[INFO] Normalized {normalized_count} content reference(s) to singular English form\n"
            ));
        }

        let children = defn
            .get("children")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(json_value_to_python_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let mut lines = Vec::new();
        lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
        lines.push(format!(
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\" xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\" xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\" xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"{format_version}\">"
        ));
        lines.push(format!(
            "\t<Subsystem uuid=\"{}\">",
            escape_xml(&subsystem_uuid)
        ));
        lines.push("\t\t<Properties>".to_string());
        lines.push(format!("\t\t\t<Name>{}</Name>", escape_xml(&obj_name)));
        emit_mltext(&mut lines, "\t\t\t", "Synonym", &synonym);
        if comment.is_empty() {
            lines.push("\t\t\t<Comment/>".to_string());
        } else {
            lines.push(format!("\t\t\t<Comment>{}</Comment>", escape_xml(&comment)));
        }
        lines.push(format!(
            "\t\t\t<IncludeHelpInContents>{include_help_in_contents}</IncludeHelpInContents>"
        ));
        lines.push(format!(
            "\t\t\t<IncludeInCommandInterface>{include_in_ci}</IncludeInCommandInterface>"
        ));
        lines.push(format!(
            "\t\t\t<UseOneCommand>{use_one_command}</UseOneCommand>"
        ));
        emit_mltext(&mut lines, "\t\t\t", "Explanation", &explanation);
        if picture.is_empty() {
            lines.push("\t\t\t<Picture/>".to_string());
        } else {
            lines.push("\t\t\t<Picture>".to_string());
            lines.push(format!("\t\t\t\t<xr:Ref>{picture}</xr:Ref>"));
            lines.push("\t\t\t\t<xr:LoadTransparent>false</xr:LoadTransparent>".to_string());
            lines.push("\t\t\t</Picture>".to_string());
        }
        if content_items.is_empty() {
            lines.push("\t\t\t<Content/>".to_string());
        } else {
            lines.push("\t\t\t<Content>".to_string());
            for item in &content_items {
                lines.push(format!(
                    "\t\t\t\t<xr:Item xsi:type=\"xr:MDObjectRef\">{}</xr:Item>",
                    escape_xml(item)
                ));
            }
            lines.push("\t\t\t</Content>".to_string());
        }
        lines.push("\t\t</Properties>".to_string());
        if children.is_empty() {
            lines.push("\t\t<ChildObjects/>".to_string());
        } else {
            lines.push("\t\t<ChildObjects>".to_string());
            for child in &children {
                lines.push(format!(
                    "\t\t\t<Subsystem>{}</Subsystem>",
                    escape_xml(child)
                ));
            }
            lines.push("\t\t</ChildObjects>".to_string());
        }
        lines.push("\t</Subsystem>".to_string());
        lines.push("</MetaDataObject>".to_string());

        let parent = path_arg(args, &["parent", "Parent"]);
        let subs_dir = if let Some(parent_path) = &parent {
            let parent_path = absolutize(parent_path.clone(), &context.cwd);
            if !parent_path.exists() {
                return Err(format!(
                    "Parent subsystem not found: {}",
                    parent_path.display()
                ));
            }
            let parent_dir = parent_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| output_dir.clone());
            let parent_base_name = parent_path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            parent_dir.join(parent_base_name).join("Subsystems")
        } else {
            output_dir.join("Subsystems")
        };

        fs::create_dir_all(&subs_dir)
            .map_err(|err| format!("failed to create {}: {err}", subs_dir.display()))?;
        let target_xml = subs_dir.join(format!("{obj_name}.xml"));
        write_utf8_bom(&target_xml, &format!("{}\n", lines.join("\n")))?;
        let mut artifacts = vec![target_xml.clone()];
        stdout.push_str(&format!("[OK] Created: {}\n", target_xml.display()));

        if !children.is_empty() {
            let child_subs_dir = subs_dir.join(&obj_name).join("Subsystems");
            if !child_subs_dir.exists() {
                fs::create_dir_all(&child_subs_dir).map_err(|err| {
                    format!("failed to create {}: {err}", child_subs_dir.display())
                })?;
                stdout.push_str(&format!(
                    "[OK] Created directory: {}\n",
                    child_subs_dir.display()
                ));
            }
            let mut seen = Vec::<String>::new();
            for (idx, child) in children.iter().enumerate() {
                if seen.iter().any(|value| value == child) {
                    continue;
                }
                seen.push(child.clone());
                let child_xml = child_subs_dir.join(format!("{child}.xml"));
                if !child_xml.exists() {
                    write_child_subsystem_stub(&child_xml, child, &format_version, 61 + idx)?;
                    stdout.push_str(&format!("[OK] Created stub: {}\n", child_xml.display()));
                    artifacts.push(child_xml);
                }
            }
        }

        let parent_xml_path = if let Some(parent_path) = parent {
            Some(absolutize(parent_path, &context.cwd))
        } else {
            let config_xml = output_dir.join("Configuration.xml");
            config_xml.exists().then_some(config_xml)
        };

        if let Some(parent_xml_path) = parent_xml_path {
            if parent_xml_path.exists() {
                let mut raw_text = fs::read_to_string(&parent_xml_path)
                    .map_err(|err| format!("failed to read {}: {err}", parent_xml_path.display()))?
                    .trim_start_matches('\u{feff}')
                    .to_string();
                let tag = format!("<Subsystem>{}</Subsystem>", escape_xml(&obj_name));
                if raw_text.contains(&tag) {
                    stdout.push_str(&format!(
                        "[SKIP] Already registered in: {}\n",
                        parent_xml_path.display()
                    ));
                } else if raw_text.contains("<ChildObjects/>") {
                    let replacement = format!("<ChildObjects>\n\t\t\t{tag}\n\t\t</ChildObjects>");
                    raw_text = raw_text.replacen("<ChildObjects/>", &replacement, 1);
                    write_utf8_bom(&parent_xml_path, &raw_text)?;
                    stdout.push_str(&format!(
                        "[OK] Registered in: {}\n",
                        parent_xml_path.display()
                    ));
                    artifacts.push(parent_xml_path);
                } else if raw_text.contains("</ChildObjects>") {
                    raw_text = raw_text.replacen(
                        "</ChildObjects>",
                        &format!("\t\t\t{tag}\n\t\t</ChildObjects>"),
                        1,
                    );
                    write_utf8_bom(&parent_xml_path, &raw_text)?;
                    stdout.push_str(&format!(
                        "[OK] Registered in: {}\n",
                        parent_xml_path.display()
                    ));
                    artifacts.push(parent_xml_path);
                } else {
                    stdout.push_str(&format!(
                        "[WARN] ChildObjects not found in: {}\n",
                        parent_xml_path.display()
                    ));
                }
            } else {
                stdout.push_str("[INFO] No parent XML to register in\n");
            }
        } else {
            stdout.push_str("[INFO] No parent XML to register in\n");
        }

        stdout.push('\n');
        stdout.push_str("=== subsystem-compile summary ===\n");
        stdout.push_str(&format!("  Name:     {obj_name}\n"));
        stdout.push_str(&format!("  UUID:     {subsystem_uuid}\n"));
        stdout.push_str(&format!("  Content:  {} objects\n", content_items.len()));
        stdout.push_str(&format!("  Children: {}\n", children.len()));
        stdout.push_str(&format!("  File:     {}\n", target_xml.display()));

        Ok((stdout, artifacts))
    })();

    match write_result {
        Ok((stdout, artifacts)) => AdapterOutcome {
            ok: true,
            summary: "unica.subsystem.compile completed with native XML writer".to_string(),
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
            summary: "unica.subsystem.compile failed in native XML writer".to_string(),
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

pub(crate) fn write_child_subsystem_stub(
    child_path: &Path,
    child_name: &str,
    format_version: &str,
    uuid_index: usize,
) -> Result<(), String> {
    let mut lines = Vec::new();
    lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
    lines.push(format!(
        "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\" xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\" xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\" xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"{format_version}\">"
    ));
    lines.push(format!(
        "\t<Subsystem uuid=\"{}\">",
        stable_uuid(uuid_index)
    ));
    lines.push("\t\t<Properties>".to_string());
    lines.push(format!("\t\t\t<Name>{}</Name>", escape_xml(child_name)));
    lines.push("\t\t\t<Synonym/>".to_string());
    lines.push("\t\t\t<Comment/>".to_string());
    lines.push("\t\t\t<IncludeHelpInContents>true</IncludeHelpInContents>".to_string());
    lines.push("\t\t\t<IncludeInCommandInterface>true</IncludeInCommandInterface>".to_string());
    lines.push("\t\t\t<UseOneCommand>false</UseOneCommand>".to_string());
    lines.push("\t\t\t<Explanation/>".to_string());
    lines.push("\t\t\t<Picture/>".to_string());
    lines.push("\t\t\t<Content/>".to_string());
    lines.push("\t\t</Properties>".to_string());
    lines.push("\t\t<ChildObjects/>".to_string());
    lines.push("\t</Subsystem>".to_string());
    lines.push("</MetaDataObject>".to_string());
    write_utf8_bom(child_path, &format!("{}\n", lines.join("\n")))
}

pub(crate) fn normalize_subsystem_content_ref(raw: &str) -> String {
    let Some(dot_idx) = raw.find('.') else {
        return raw.to_string();
    };
    let type_part = &raw[..dot_idx];
    let name_part = &raw[dot_idx + 1..];
    let normalized = subsystem_content_type(type_part).unwrap_or(type_part);
    format!("{normalized}.{name_part}")
}

pub(crate) fn subsystem_content_type(type_part: &str) -> Option<&'static str> {
    match type_part {
        "Catalogs" | "Справочник" | "Каталог" | "Справочники" => {
            Some("Catalog")
        }
        "Documents" | "Документ" | "Документы" => Some("Document"),
        "Enums" | "Перечисление" | "Перечисления" => Some("Enum"),
        "Constants" | "Константа" | "Константы" => Some("Constant"),
        "Reports" | "Отчёт" | "Отчет" | "Отчёты" | "Отчеты" => Some("Report"),
        "DataProcessors" | "Обработка" | "Обработки" => Some("DataProcessor"),
        "InformationRegisters" | "РегистрСведений" | "РегистрыСведений" => {
            Some("InformationRegister")
        }
        "AccumulationRegisters" | "РегистрНакопления" | "РегистрыНакопления" => {
            Some("AccumulationRegister")
        }
        "AccountingRegisters" | "РегистрБухгалтерии" | "РегистрыБухгалтерии" => {
            Some("AccountingRegister")
        }
        "CalculationRegisters"
        | "РегистрРасчёта"
        | "РегистрРасчета"
        | "РегистрыРасчёта"
        | "РегистрыРасчета" => Some("CalculationRegister"),
        "ChartsOfAccounts" | "ПланСчетов" | "ПланыСчетов" => {
            Some("ChartOfAccounts")
        }
        "ChartsOfCharacteristicTypes" | "ПланВидовХарактеристик" | "ПланыВидовХарактеристик" => {
            Some("ChartOfCharacteristicTypes")
        }
        "ChartsOfCalculationTypes"
        | "ПланВидовРасчёта"
        | "ПланВидовРасчета"
        | "ПланыВидовРасчёта"
        | "ПланыВидовРасчета" => Some("ChartOfCalculationTypes"),
        "BusinessProcesses" | "БизнесПроцесс" | "БизнесПроцессы" => {
            Some("BusinessProcess")
        }
        "Tasks" | "Задача" | "Задачи" => Some("Task"),
        "ExchangePlans" | "ПланОбмена" | "ПланыОбмена" => Some("ExchangePlan"),
        "DocumentJournals" | "ЖурналДокументов" | "ЖурналыДокументов" => {
            Some("DocumentJournal")
        }
        "CommonModules" | "ОбщийМодуль" | "ОбщиеМодули" => {
            Some("CommonModule")
        }
        "CommonCommands" | "ОбщаяКоманда" | "ОбщиеКоманды" => {
            Some("CommonCommand")
        }
        "CommonForms" | "ОбщаяФорма" | "ОбщиеФормы" => Some("CommonForm"),
        "CommonPictures" | "ОбщаяКартинка" | "ОбщиеКартинки" => {
            Some("CommonPicture")
        }
        "CommonTemplates" | "ОбщийМакет" | "ОбщиеМакеты" => {
            Some("CommonTemplate")
        }
        "CommonAttributes" | "ОбщийРеквизит" | "ОбщиеРеквизиты" => {
            Some("CommonAttribute")
        }
        "CommandGroups" | "ГруппаКоманд" | "ГруппыКоманд" => {
            Some("CommandGroup")
        }
        "Roles" | "Роль" | "Роли" => Some("Role"),
        "SessionParameters" | "ПараметрСеанса" | "ПараметрыСеанса" => {
            Some("SessionParameter")
        }
        "FilterCriteria" | "КритерийОтбора" | "КритерииОтбора" => {
            Some("FilterCriterion")
        }
        "XDTOPackages" | "ПакетXDTO" | "ПакетыXDTO" => Some("XDTOPackage"),
        "WebServices" | "ВебСервис" | "ВебСервисы" => Some("WebService"),
        "HTTPServices" | "HTTPСервис" | "HTTPСервисы" => Some("HTTPService"),
        "WSReferences" | "WSСсылка" | "WSСсылки" => Some("WSReference"),
        "EventSubscriptions" | "ПодпискаНаСобытие" | "ПодпискиНаСобытия" => {
            Some("EventSubscription")
        }
        "ScheduledJobs" | "РегламентноеЗадание" | "РегламентныеЗадания" => {
            Some("ScheduledJob")
        }
        "SettingsStorages" | "ХранилищеНастроек" | "ХранилищаНастроек" => {
            Some("SettingsStorage")
        }
        "FunctionalOptions" | "ФункциональнаяОпция" | "ФункциональныеОпции" => {
            Some("FunctionalOption")
        }
        "FunctionalOptionsParameters" | "ПараметрФункциональныхОпций" => {
            Some("FunctionalOptionsParameter")
        }
        "DefinedTypes" | "ОпределяемыйТип" | "ОпределяемыеТипы" => {
            Some("DefinedType")
        }
        "DocumentNumerators" | "НумераторДокументов" => {
            Some("DocumentNumerator")
        }
        "Sequences" | "Последовательность" => Some("Sequence"),
        "Subsystems" | "Подсистема" | "Подсистемы" => Some("Subsystem"),
        "StyleItems" | "ЭлементСтиля" | "ЭлементыСтиля" => {
            Some("StyleItem")
        }
        "IntegrationServices" | "СервисИнтеграции" | "СервисыИнтеграции" => {
            Some("IntegrationService")
        }
        _ => None,
    }
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
        "subsystem-info" => Some(Ok(analyze_subsystem_info(args, context))),
        "subsystem-validate" => Some(Ok(validate_subsystem(args, context))),
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
        "subsystem-compile" => Some(compile_subsystem(args, context)),
        "subsystem-edit" => Some(edit_subsystem(args, context)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workspace::WorkspaceContext;
    use serde_json::{json, Map, Value};
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_context(name: &str) -> WorkspaceContext {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("unica-subsystem-compile-{name}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build").join("unica"),
            workspace_epoch: 1,
        }
    }

    fn compile_args(output_dir: &Path, definition: Value) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert(
            "OutputDir".to_string(),
            Value::String(output_dir.display().to_string()),
        );
        args.insert("Value".to_string(), Value::String(definition.to_string()));
        args
    }

    fn subsystem_uuid(output_dir: &Path, name: &str) -> String {
        let xml_path = output_dir.join("Subsystems").join(format!("{name}.xml"));
        let xml = fs::read_to_string(&xml_path).unwrap();
        let marker = "<Subsystem uuid=\"";
        let start = xml.find(marker).unwrap() + marker.len();
        let end = xml[start..].find('"').unwrap() + start;
        xml[start..end].to_string()
    }

    #[test]
    fn compile_subsystem_preserves_explicit_uuid() {
        let context = temp_context("explicit-uuid");
        let explicit_uuid = "11111111-2222-3333-4444-555555555555";
        let args = compile_args(
            &context.cwd,
            json!({
                "name": "ExplicitUuidSubsystem",
                "uuid": explicit_uuid
            }),
        );

        let outcome = compile_subsystem(&args, &context);

        assert!(outcome.ok, "{:?}", outcome.errors);
        assert_eq!(
            subsystem_uuid(&context.cwd, "ExplicitUuidSubsystem"),
            explicit_uuid
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn compile_subsystem_generates_unique_uuid_when_missing() {
        let context = temp_context("generated-uuid");
        for name in ["GeneratedUuidA", "GeneratedUuidB"] {
            let args = compile_args(
                &context.cwd,
                json!({
                    "name": name
                }),
            );

            let outcome = compile_subsystem(&args, &context);
            assert!(outcome.ok, "{:?}", outcome.errors);
        }

        let first_uuid = subsystem_uuid(&context.cwd, "GeneratedUuidA");
        let second_uuid = subsystem_uuid(&context.cwd, "GeneratedUuidB");
        assert_ne!(first_uuid, second_uuid);
        let _ = fs::remove_dir_all(&context.cwd);
    }
}
