#![allow(dead_code, unused_imports)]

use crate::application::AdapterOutcome;
use crate::domain::workspace::WorkspaceContext;
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::common::*;
use super::{cf::*, cfe::*, form::*, meta::*, mxl::*, role::*, skd::*, subsystem::*, template::*};
pub(crate) const INTERFACE_CI_NS: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";

pub(crate) const INTERFACE_XR_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";

pub(crate) const INTERFACE_XS_NS: &str = "http://www.w3.org/2001/XMLSchema";

pub(crate) const INTERFACE_XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

pub(crate) const INTERFACE_SECTION_ORDER: &[&str] = &[
    "CommandsVisibility",
    "CommandsPlacement",
    "CommandsOrder",
    "SubsystemsOrder",
    "GroupsOrder",
];

#[derive(Default)]
pub(crate) struct InterfaceEditCounters {
    pub(crate) added: usize,
    pub(crate) removed: usize,
    pub(crate) modified: usize,
}

pub(crate) fn edit_interface(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let edit_result = (|| -> Result<(String, PathBuf), String> {
        let definition_file = path_arg(args, &["definitionFile", "DefinitionFile"]);
        let operation = string_arg(args, &["operation", "Operation"]);
        if definition_file.is_some() && operation.is_some() {
            return Err("Cannot use both -DefinitionFile and -Operation".to_string());
        }
        if definition_file.is_none() && operation.is_none() {
            return Err("Either -DefinitionFile or -Operation is required".to_string());
        }

        let mut ci_path = required_path(args, &["ciPath", "CIPath", "path", "Path"], "CIPath")
            .map(|path| absolutize(path, &context.cwd))?;
        let format_version =
            detect_format_version(ci_path.parent().unwrap_or(context.cwd.as_path()));

        let mut stdout = String::new();
        let source_exists = ci_path.is_file();
        let created_new = if !source_exists {
            if bool_arg(args, &["createIfMissing", "CreateIfMissing"]) {
                stdout.push_str(&format!(
                    "[INFO] Created new CommandInterface.xml: {}\n",
                    ci_path.display()
                ));
                true
            } else {
                return Err(format!(
                    "File not found: {} (use -CreateIfMissing to create)",
                    ci_path.display()
                ));
            }
        } else {
            false
        };
        if source_exists {
            ci_path = interface_normalize_lexical_path(&ci_path);
        }

        let source_text = if source_exists {
            read_utf8_sig(&ci_path)?
        } else {
            String::new()
        };
        let mut text = source_text.clone();
        text = lxml_parser_normalized_text(&text);
        if text.is_empty() {
            text = emit_empty_command_interface_document(&format_version);
        }
        let operations = interface_edit_operations(args, &context.cwd, operation, definition_file)?;
        let mut counters = InterfaceEditCounters::default();
        for (op_name, value) in operations {
            match op_name.as_str() {
                "hide" => {
                    let commands = interface_value_list(&value)?;
                    interface_text_do_hide(&mut text, commands, &mut counters, &mut stdout)?;
                }
                "show" => {
                    let commands = interface_value_list(&value)?;
                    interface_text_do_show(&mut text, commands, &mut counters, &mut stdout)?;
                }
                "place" => interface_text_do_place(&mut text, &value, &mut counters, &mut stdout)?,
                "order" => interface_text_do_order(&mut text, &value, &mut counters, &mut stdout)?,
                "subsystem-order" => {
                    interface_text_do_subsystem_order(
                        &mut text,
                        &value,
                        &mut counters,
                        &mut stdout,
                    )?;
                }
                "group-order" => {
                    interface_text_do_group_order(&mut text, &value, &mut counters, &mut stdout)?;
                }
                _ => return Err(format!("Unknown operation: {op_name}")),
            }
        }

        if created_new {
            if let Some(parent) = ci_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
            }
        }
        write_utf8_bom(
            &ci_path,
            &lxml_tree_serialized_text_like_source(&text, &source_text),
        )?;
        if created_new {
            ci_path = interface_normalize_lexical_path(&ci_path);
        }
        stdout.push_str(&format!("[INFO] Saved: {}\n", ci_path.display()));

        if !bool_arg(args, &["noValidate", "NoValidate"]) {
            stdout.push('\n');
            stdout.push_str("--- Running interface-validate ---\n");
            let mut validate_args = Map::new();
            validate_args.insert(
                "CIPath".to_string(),
                Value::String(ci_path.display().to_string()),
            );
            if let Some(validate_stdout) = validate_interface(&validate_args, context).stdout {
                stdout.push_str(&validate_stdout);
            }
        }

        stdout.push('\n');
        stdout.push_str("=== interface-edit summary ===\n");
        stdout.push_str(&format!("  Added:    {}\n", counters.added));
        stdout.push_str(&format!("  Removed:  {}\n", counters.removed));
        stdout.push_str(&format!("  Modified: {}\n", counters.modified));
        Ok((stdout, ci_path))
    })();

    match edit_result {
        Ok((stdout, ci_path)) => AdapterOutcome {
            ok: true,
            summary: "unica.interface.edit completed with native command interface editor"
                .to_string(),
            changes: vec![format!("updated {}", ci_path.display())],
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![ci_path.display().to_string()],
            stdout: Some(stdout),
            stderr: None,
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.interface.edit failed in native command interface editor".to_string(),
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

pub(crate) fn interface_edit_operations(
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
        let mut operations = Vec::new();
        for item in items {
            let op_name = item
                .get("operation")
                .and_then(Value::as_str)
                .unwrap_or(operation.unwrap_or(""))
                .to_string();
            let value = item
                .get("value")
                .cloned()
                .unwrap_or_else(|| Value::String(String::new()));
            operations.push((op_name, value));
        }
        Ok(operations)
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

pub(crate) fn interface_value_list(value: &Value) -> Result<Vec<String>, String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.starts_with('[') {
                let parsed: Value = serde_json::from_str(trimmed)
                    .map_err(|err| format!("failed to parse value list: {err}"))?;
                interface_json_array_strings(&parsed)
            } else {
                Ok(vec![text.to_string()])
            }
        }
        Value::Array(_) => interface_json_array_strings(value),
        _ => Ok(vec![interface_json_string(value)]),
    }
}

pub(crate) fn interface_json_array_strings(value: &Value) -> Result<Vec<String>, String> {
    let Some(items) = value.as_array() else {
        return Err("value must be an array".to_string());
    };
    Ok(items.iter().map(interface_json_string).collect())
}

pub(crate) fn interface_json_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Null => "None".to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn emit_empty_command_interface_document(format_version: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<CommandInterface xmlns=\"{INTERFACE_CI_NS}\" xmlns:xr=\"{INTERFACE_XR_NS}\" xmlns:xs=\"{INTERFACE_XS_NS}\" xmlns:xsi=\"{INTERFACE_XSI_NS}\" version=\"{}\">\n\
</CommandInterface>",
        escape_xml(format_version)
    )
}

pub(crate) fn interface_text_do_hide(
    text: &mut String,
    commands: Vec<String>,
    counters: &mut InterfaceEditCounters,
    stdout: &mut String,
) -> Result<(), String> {
    let commands = commands
        .into_iter()
        .map(|raw| normalize_interface_command_name(&raw, stdout))
        .collect::<Vec<_>>();
    for cmd in commands {
        match interface_text_command_common(text, "CommandsVisibility", &cmd) {
            Some(common) if common == "false" => {
                stdout.push_str(&format!("[WARN] Already hidden: {cmd}\n"));
            }
            Some(_) => {
                interface_text_replace_command_common(text, "CommandsVisibility", &cmd, "false")?;
                counters.modified += 1;
                stdout.push_str(&format!("[INFO] Changed to hidden: {cmd}\n"));
            }
            None => {
                let fragment = format!(
                    "<Command name=\"{}\"><Visibility><xr:Common>false</xr:Common></Visibility></Command>",
                    escape_xml(&cmd)
                );
                interface_text_append_to_section(text, "CommandsVisibility", &fragment)?;
                counters.added += 1;
                stdout.push_str(&format!("[INFO] Hidden: {cmd}\n"));
            }
        }
    }
    Ok(())
}

pub(crate) fn interface_text_do_show(
    text: &mut String,
    commands: Vec<String>,
    counters: &mut InterfaceEditCounters,
    stdout: &mut String,
) -> Result<(), String> {
    let commands = commands
        .into_iter()
        .map(|raw| normalize_interface_command_name(&raw, stdout))
        .collect::<Vec<_>>();
    for cmd in commands {
        match interface_text_command_common(text, "CommandsVisibility", &cmd) {
            Some(common) if common == "true" => {
                stdout.push_str(&format!("[WARN] Already shown: {cmd}\n"));
            }
            Some(common) if common == "false" => {
                interface_text_replace_command_common(text, "CommandsVisibility", &cmd, "true")?;
                counters.modified += 1;
                stdout.push_str(&format!("[INFO] Changed to shown: {cmd}\n"));
            }
            Some(_) | None => {
                let fragment = format!(
                    "<Command name=\"{}\"><Visibility><xr:Common>true</xr:Common></Visibility></Command>",
                    escape_xml(&cmd)
                );
                interface_text_append_to_section(text, "CommandsVisibility", &fragment)?;
                counters.added += 1;
                stdout.push_str(&format!("[INFO] Shown: {cmd}\n"));
            }
        }
    }
    Ok(())
}

pub(crate) fn interface_text_do_place(
    text: &mut String,
    value: &Value,
    counters: &mut InterfaceEditCounters,
    stdout: &mut String,
) -> Result<(), String> {
    let value = interface_json_object(value)?;
    let command = value
        .get("command")
        .map(interface_json_string)
        .unwrap_or_default();
    let cmd_name = normalize_interface_command_name(&command, stdout);
    let group_name = value
        .get("group")
        .map(interface_json_string)
        .unwrap_or_default();
    if cmd_name.is_empty() || group_name.is_empty() {
        return Err("place requires {command, group}".to_string());
    }

    if interface_text_command_bounds_in_section(text, "CommandsPlacement", &cmd_name).is_some() {
        interface_text_replace_command_child(
            text,
            "CommandsPlacement",
            &cmd_name,
            "CommandGroup",
            &group_name,
        )?;
        counters.modified += 1;
        stdout.push_str(&format!(
            "[INFO] Updated placement: {cmd_name} -> {group_name}\n"
        ));
    } else {
        let fragment = format!(
            "<Command name=\"{}\"><CommandGroup>{}</CommandGroup><Placement>Auto</Placement></Command>",
            escape_xml(&cmd_name),
            escape_xml(&group_name)
        );
        interface_text_append_to_section(text, "CommandsPlacement", &fragment)?;
        counters.added += 1;
        stdout.push_str(&format!("[INFO] Placed: {cmd_name} -> {group_name}\n"));
    }
    Ok(())
}

pub(crate) fn interface_text_do_order(
    text: &mut String,
    value: &Value,
    counters: &mut InterfaceEditCounters,
    stdout: &mut String,
) -> Result<(), String> {
    let value = interface_json_object(value)?;
    let group_name = value
        .get("group")
        .map(interface_json_string)
        .unwrap_or_default();
    let command_values = value
        .get("commands")
        .ok_or_else(|| "order requires {group, commands:[...]}".to_string())?;
    let commands = interface_json_array_strings(command_values)?
        .into_iter()
        .map(|command| normalize_interface_command_name(&command, stdout))
        .collect::<Vec<_>>();
    if group_name.is_empty() || commands.is_empty() {
        return Err("order requires {group, commands:[...]}".to_string());
    }

    counters.removed += interface_text_count_commands_for_group(text, "CommandsOrder", &group_name);
    counters.added += commands.len();
    let fragments = commands
        .iter()
        .map(|cmd_name| {
            format!(
                "<Command name=\"{}\"><CommandGroup>{}</CommandGroup></Command>",
                escape_xml(cmd_name),
                escape_xml(&group_name)
            )
        })
        .collect::<Vec<_>>();
    interface_text_replace_section_items(text, "CommandsOrder", &fragments)?;
    stdout.push_str(&format!(
        "[INFO] Set order for {group_name} : {} commands\n",
        commands.len()
    ));
    Ok(())
}

pub(crate) fn interface_text_do_subsystem_order(
    text: &mut String,
    value: &Value,
    counters: &mut InterfaceEditCounters,
    stdout: &mut String,
) -> Result<(), String> {
    let value = interface_json_array(value)?;
    let subsystems = interface_json_array_strings(&value)?;
    if subsystems.is_empty() {
        return Err("subsystem-order requires array of subsystem paths".to_string());
    }
    counters.removed += interface_text_count_direct_items(text, "SubsystemsOrder", "Subsystem");
    counters.added += subsystems.len();
    let fragments = subsystems
        .iter()
        .map(|sub| format!("<Subsystem>{}</Subsystem>", escape_xml(sub)))
        .collect::<Vec<_>>();
    interface_text_replace_section_items(text, "SubsystemsOrder", &fragments)?;
    stdout.push_str(&format!(
        "[INFO] Set subsystem order: {} entries\n",
        subsystems.len()
    ));
    Ok(())
}

pub(crate) fn interface_text_do_group_order(
    text: &mut String,
    value: &Value,
    counters: &mut InterfaceEditCounters,
    stdout: &mut String,
) -> Result<(), String> {
    let value = interface_json_array(value)?;
    let groups = interface_json_array_strings(&value)?;
    if groups.is_empty() {
        return Err("group-order requires array of group names".to_string());
    }
    counters.removed += interface_text_count_direct_items(text, "GroupsOrder", "Group");
    counters.added += groups.len();
    let fragments = groups
        .iter()
        .map(|group| format!("<Group>{}</Group>", escape_xml(group)))
        .collect::<Vec<_>>();
    interface_text_replace_section_items(text, "GroupsOrder", &fragments)?;
    stdout.push_str(&format!(
        "[INFO] Set group order: {} entries\n",
        groups.len()
    ));
    Ok(())
}

pub(crate) fn interface_text_command_common(
    text: &str,
    section: &str,
    cmd_name: &str,
) -> Option<String> {
    find_element_bounds(text, section, 0)?;
    let (cmd_start, cmd_end) = interface_text_command_bounds_in_section(text, section, cmd_name)?;
    let command = &text[cmd_start..cmd_end];
    let value = interface_text_element_value(command, "xr:Common")
        .or_else(|| interface_text_element_value(command, "Common"))?;
    Some(value.trim().to_string())
}

pub(crate) fn interface_text_replace_command_common(
    text: &mut String,
    section: &str,
    cmd_name: &str,
    value: &str,
) -> Result<(), String> {
    interface_text_replace_command_child(text, section, cmd_name, "xr:Common", value)
        .or_else(|_| interface_text_replace_command_child(text, section, cmd_name, "Common", value))
}

pub(crate) fn interface_text_replace_command_child(
    text: &mut String,
    section: &str,
    cmd_name: &str,
    child_tag: &str,
    value: &str,
) -> Result<(), String> {
    let (cmd_start, cmd_end) = interface_text_command_bounds_in_section(text, section, cmd_name)
        .ok_or_else(|| format!("Command not found: {cmd_name}"))?;
    let command = &text[cmd_start..cmd_end];
    let start_tag = format!("<{child_tag}>");
    let close_tag = format!("</{child_tag}>");
    let Some(rel_open) = command.find(&start_tag) else {
        return Err(format!("No <{child_tag}> in command: {cmd_name}"));
    };
    let value_start = cmd_start + rel_open + start_tag.len();
    let Some(rel_close) = text[value_start..cmd_end].find(&close_tag) else {
        return Err(format!("No </{child_tag}> in command: {cmd_name}"));
    };
    let value_end = value_start + rel_close;
    text.replace_range(value_start..value_end, &escape_xml(value));
    Ok(())
}

pub(crate) fn interface_text_command_bounds_in_section(
    text: &str,
    section: &str,
    cmd_name: &str,
) -> Option<(usize, usize)> {
    let (_, content_start, close_start, _, _, _) = find_element_bounds(text, section, 0)?;
    let body = &text[content_start..close_start];
    let (rel_start, rel_end) = interface_text_command_bounds(body, cmd_name)?;
    Some((content_start + rel_start, content_start + rel_end))
}

pub(crate) fn interface_text_command_bounds(
    section_body: &str,
    cmd_name: &str,
) -> Option<(usize, usize)> {
    let escaped_name = escape_xml(cmd_name);
    let name_attr = format!("name=\"{escaped_name}\"");
    let mut offset = 0usize;
    while let Some(rel_start) = section_body[offset..].find("<Command") {
        let start = offset + rel_start;
        let gt = start + section_body[start..].find('>')?;
        let open_tag = &section_body[start..=gt];
        let close = "</Command>";
        let close_start = gt + 1 + section_body[gt + 1..].find(close)?;
        let end = close_start + close.len();
        if open_tag.contains(&name_attr) {
            return Some((start, end));
        }
        offset = end;
    }
    None
}

pub(crate) fn interface_text_element_value(text: &str, tag: &str) -> Option<String> {
    let start_tag = format!("<{tag}>");
    let close_tag = format!("</{tag}>");
    let start = text.find(&start_tag)? + start_tag.len();
    let end = start + text[start..].find(&close_tag)?;
    Some(text[start..end].to_string())
}

pub(crate) fn interface_text_append_to_section(
    text: &mut String,
    section: &str,
    fragment: &str,
) -> Result<(), String> {
    if find_element_bounds(text, section, 0).is_none() {
        interface_text_insert_section(text, section, &[])?;
    }
    let (_, content_start, close_start, _, _, _) = find_element_bounds(text, section, 0)
        .ok_or_else(|| format!("No <{section}> element found"))?;
    let body = &text[content_start..close_start];
    let child_indent = interface_text_detect_indent(body)
        .or_else(|| interface_text_detect_empty_indent(body))
        .unwrap_or_else(|| "\t\t".to_string());
    let replacement = if body.trim().is_empty() {
        let parent_indent = interface_parent_indent(&child_indent);
        format!("\r\n{child_indent}{fragment}\r\n{parent_indent}")
    } else {
        let tail_start = interface_trailing_ws_start(text, content_start, close_start);
        let old_tail = &text[tail_start..close_start];
        format!("\r\n{child_indent}{fragment}{old_tail}")
    };
    let replace_start = if body.trim().is_empty() {
        content_start
    } else {
        interface_trailing_ws_start(text, content_start, close_start)
    };
    text.replace_range(replace_start..close_start, &replacement);
    Ok(())
}

pub(crate) fn interface_text_replace_section_items(
    text: &mut String,
    section: &str,
    fragments: &[String],
) -> Result<(), String> {
    if let Some((_, content_start, close_start, _, _, _)) = find_element_bounds(text, section, 0) {
        let body = &text[content_start..close_start];
        let child_indent = interface_text_detect_indent(body)
            .or_else(|| interface_text_detect_empty_indent(body))
            .unwrap_or_else(|| "\t\t".to_string());
        let parent_indent = interface_parent_indent(&child_indent);
        let replacement = interface_text_section_body(&child_indent, &parent_indent, fragments);
        text.replace_range(content_start..close_start, &replacement);
        return Ok(());
    }

    interface_text_insert_section(text, section, fragments)
}

pub(crate) fn interface_text_insert_section(
    text: &mut String,
    section: &str,
    fragments: &[String],
) -> Result<(), String> {
    let (_, content_start, close_start, _, _, _) = find_element_bounds(text, "CommandInterface", 0)
        .ok_or_else(|| "No <CommandInterface> root element found".to_string())?;
    let root_body = &text[content_start..close_start];
    let root_indent = interface_text_detect_indent(root_body).unwrap_or_else(|| "\t".to_string());
    let body = if fragments.is_empty() {
        format!("\r\n{root_indent}")
    } else {
        interface_text_section_body(&root_indent, "", fragments)
    };

    if let Some(next_section_start) =
        interface_text_next_ordered_section_start(text, section, content_start, close_start)
    {
        let insert_start = interface_trailing_ws_start(text, content_start, next_section_start);
        let section_xml =
            format!("\r\n{root_indent}<{section}>{body}</{section}>\r\n{root_indent}");
        text.replace_range(insert_start..next_section_start, &section_xml);
    } else {
        let section_xml = format!("\r\n{root_indent}<{section}>{body}</{section}>\r\n");
        let tail_start = interface_trailing_ws_start(text, content_start, close_start);
        text.replace_range(tail_start..close_start, &section_xml);
    }
    Ok(())
}

pub(crate) fn interface_text_next_ordered_section_start(
    text: &str,
    section: &str,
    content_start: usize,
    close_start: usize,
) -> Option<usize> {
    let section_index = interface_section_order_index(section)?;
    INTERFACE_SECTION_ORDER
        .iter()
        .skip(section_index + 1)
        .filter_map(|candidate| {
            find_element_bounds(text, candidate, content_start).and_then(
                |(open_start, _, _, _, _, _)| {
                    if open_start < close_start {
                        Some(open_start)
                    } else {
                        None
                    }
                },
            )
        })
        .min()
}

pub(crate) fn interface_section_order_index(section: &str) -> Option<usize> {
    INTERFACE_SECTION_ORDER
        .iter()
        .position(|candidate| *candidate == section)
}

pub(crate) fn interface_text_section_body(
    child_indent: &str,
    parent_indent: &str,
    fragments: &[String],
) -> String {
    let mut body = String::new();
    for fragment in fragments {
        body.push_str("\r\n");
        body.push_str(child_indent);
        body.push_str(fragment);
    }
    body.push_str("\r\n");
    body.push_str(parent_indent);
    body
}

pub(crate) fn interface_text_count_commands_for_group(
    text: &str,
    section: &str,
    group_name: &str,
) -> usize {
    let Some((_, content_start, close_start, _, _, _)) = find_element_bounds(text, section, 0)
    else {
        return 0;
    };
    let body = &text[content_start..close_start];
    let mut count = 0usize;
    let mut offset = 0usize;
    while let Some(rel_start) = body[offset..].find("<Command") {
        let start = offset + rel_start;
        let Some(gt_rel) = body[start..].find('>') else {
            break;
        };
        let gt = start + gt_rel;
        let Some(close_rel) = body[gt + 1..].find("</Command>") else {
            break;
        };
        let end = gt + 1 + close_rel + "</Command>".len();
        let command = &body[start..end];
        if interface_text_element_value(command, "CommandGroup")
            .is_some_and(|value| value.trim() == group_name)
        {
            count += 1;
        }
        offset = end;
    }
    count
}

pub(crate) fn interface_text_count_direct_items(text: &str, section: &str, item: &str) -> usize {
    let Some((_, content_start, close_start, _, _, _)) = find_element_bounds(text, section, 0)
    else {
        return 0;
    };
    text[content_start..close_start]
        .match_indices(&format!("<{item}>"))
        .count()
}

pub(crate) fn interface_text_detect_indent(body: &str) -> Option<String> {
    for segment in body.split_inclusive('\n') {
        let Some(after_newline) = segment.split('\n').next_back() else {
            continue;
        };
        let indent = after_newline
            .chars()
            .take_while(|ch| *ch == '\t' || *ch == ' ')
            .collect::<String>();
        if !indent.is_empty() && after_newline[indent.len()..].starts_with('<') {
            return Some(indent);
        }
    }
    None
}

pub(crate) fn interface_text_detect_empty_indent(body: &str) -> Option<String> {
    if !body.trim().is_empty() {
        return None;
    }
    let after_newline = body.rsplit('\n').next().unwrap_or(body);
    let after_newline = after_newline.trim_end_matches('\r');
    let indent = after_newline
        .chars()
        .take_while(|ch| *ch == '\t' || *ch == ' ')
        .collect::<String>();
    if !indent.is_empty() && after_newline[indent.len()..].trim().is_empty() {
        Some(indent)
    } else {
        None
    }
}

pub(crate) fn interface_parent_indent(child_indent: &str) -> String {
    child_indent
        .strip_suffix('\t')
        .or_else(|| child_indent.strip_suffix("    "))
        .unwrap_or("")
        .to_string()
}

pub(crate) fn interface_trailing_ws_start(text: &str, min: usize, end: usize) -> usize {
    let bytes = text.as_bytes();
    let mut start = end;
    while start > min {
        match bytes[start - 1] {
            b' ' | b'\t' | b'\n' | b'\r' => start -= 1,
            _ => break,
        }
    }
    start
}

pub(crate) fn interface_normalize_lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

pub(crate) fn interface_json_object(value: &Value) -> Result<Value, String> {
    if value.is_object() {
        Ok(value.clone())
    } else if let Some(text) = value.as_str() {
        serde_json::from_str(text).map_err(|err| format!("failed to parse JSON value: {err}"))
    } else {
        Err("value must be a JSON object".to_string())
    }
}

pub(crate) fn interface_json_array(value: &Value) -> Result<Value, String> {
    if value.is_array() {
        Ok(value.clone())
    } else if let Some(text) = value.as_str() {
        serde_json::from_str(text).map_err(|err| format!("failed to parse JSON value: {err}"))
    } else {
        Err("value must be a JSON array".to_string())
    }
}

pub(crate) fn normalize_interface_command_name(name: &str, stdout: &mut String) -> String {
    let Some(dot_idx) = name.find('.') else {
        return name.to_string();
    };
    let first = &name[..dot_idx];
    let rest = &name[dot_idx..];
    if let Some(normalized_type) = interface_type_norm(first) {
        let normalized = format!("{normalized_type}{rest}");
        if normalized != name {
            stdout.push_str(&format!("[NORM] Command: {name} -> {normalized}\n"));
        }
        normalized
    } else {
        name.to_string()
    }
}

pub(crate) fn interface_type_norm(value: &str) -> Option<&'static str> {
    match value {
        "Catalogs" | "Справочник" | "Справочники" => Some("Catalog"),
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
        "CalculationRegisters" => Some("CalculationRegister"),
        "ChartsOfAccounts" | "ПланСчетов" | "ПланыСчетов" => {
            Some("ChartOfAccounts")
        }
        "ChartsOfCharacteristicTypes" | "ПланВидовХарактеристик" | "ПланыВидовХарактеристик" => {
            Some("ChartOfCharacteristicTypes")
        }
        "ChartsOfCalculationTypes" => Some("ChartOfCalculationTypes"),
        "BusinessProcesses" | "БизнесПроцесс" | "БизнесПроцессы" => {
            Some("BusinessProcess")
        }
        "Tasks" | "Задача" | "Задачи" => Some("Task"),
        "ExchangePlans" | "ПланОбмена" | "ПланыОбмена" => Some("ExchangePlan"),
        "DocumentJournals" | "ЖурналДокументов" | "ЖурналыДокументов" => {
            Some("DocumentJournal")
        }
        "CommonModules" | "ОбщийМодуль" => Some("CommonModule"),
        "CommonCommands" | "ОбщаяКоманда" => Some("CommonCommand"),
        "CommonForms" | "ОбщаяФорма" => Some("CommonForm"),
        "CommonPictures" => Some("CommonPicture"),
        "CommonTemplates" => Some("CommonTemplate"),
        "CommonAttributes" => Some("CommonAttribute"),
        "CommandGroups" => Some("CommandGroup"),
        "Roles" => Some("Role"),
        "Subsystems" | "Подсистема" | "Подсистемы" => Some("Subsystem"),
        "StyleItems" => Some("StyleItem"),
        _ => None,
    }
}

pub(crate) fn validate_interface(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    const NS_CI: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";
    const NS_XR: &str = "http://v8.1c.ru/8.3/xcf/readable";

    let result = (|| -> Result<(bool, String, String, Option<PathBuf>, PathBuf), String> {
        let raw_path = required_path(args, &["ciPath", "CIPath", "path", "Path"], "CIPath")?;
        let mut ci_path = absolutize(raw_path, &context.cwd);
        if ci_path.is_dir() {
            ci_path = ci_path.join("Ext").join("CommandInterface.xml");
        }
        if !ci_path.exists()
            && ci_path.file_name().and_then(|value| value.to_str()) == Some("CommandInterface.xml")
        {
            let candidate = ci_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join("Ext")
                .join("CommandInterface.xml");
            if candidate.exists() {
                ci_path = candidate;
            }
        }
        if !ci_path.exists() {
            let stdout = format!("[ERROR] File not found: {}\n", ci_path.display());
            return Ok((false, stdout.clone(), String::new(), None, ci_path));
        }

        let context_name = interface_context_name(&ci_path);
        let detailed = bool_arg(args, &["detailed", "Detailed"]);
        let max_errors = int_arg(args, &["maxErrors", "MaxErrors"])
            .unwrap_or(30)
            .max(0) as usize;
        let out_file =
            path_arg(args, &["outFile", "OutFile"]).map(|path| absolutize(path, &context.cwd));
        let mut report = MxlValidationReporter::new(max_errors, detailed);
        let mut all_command_names = Vec::<String>::new();

        report.lines.push(format!(
            "=== Validation: CommandInterface ({context_name}) ==="
        ));
        report.lines.push(String::new());

        let text = fs::read_to_string(&ci_path)
            .map_err(|err| format!("failed to read {}: {err}", ci_path.display()))?;
        let doc = match Document::parse(text.trim_start_matches('\u{feff}')) {
            Ok(doc) => doc,
            Err(error) => {
                report.error(format!("1. XML parse error: {error}"));
                report.stopped = true;
                let output = finish_interface_validation(report, &context_name);
                return Ok((false, output, String::new(), out_file, ci_path));
            }
        };

        let root = doc.root_element();
        if root.tag_name().name() != "CommandInterface" {
            report.error(format!(
                "1. Root element: expected <CommandInterface>, got <{}>",
                root.tag_name().name()
            ));
            report.stopped = true;
        } else {
            let ns_uri = root.tag_name().namespace().unwrap_or("");
            let version = root.attribute("version").unwrap_or("");
            if ns_uri != NS_CI {
                report.error(format!("1. Root namespace: expected {NS_CI}, got {ns_uri}"));
            } else if version.is_empty() {
                report.warn(
                    "1. Root structure: CommandInterface, namespace valid, but no version attribute",
                );
            } else {
                report.ok(format!(
                    "1. Root structure: CommandInterface, version {version}, namespace valid"
                ));
            }
        }

        let mut found_sections = Vec::<String>::new();
        if !report.stopped {
            let mut invalid_elements = Vec::<String>::new();
            for child in root.children().filter(|child| child.is_element()) {
                let local_name = child.tag_name().name();
                if INTERFACE_SECTION_ORDER.contains(&local_name) {
                    found_sections.push(local_name.to_string());
                } else {
                    invalid_elements.push(local_name.to_string());
                }
            }
            if invalid_elements.is_empty() {
                report.ok(format!(
                    "2. Child elements: {} valid sections",
                    found_sections.len()
                ));
            } else {
                report.error(format!(
                    "2. Invalid child elements: {}",
                    invalid_elements.join(", ")
                ));
            }
        }

        if !report.stopped {
            let mut order_ok = true;
            let mut last_idx = -1isize;
            for section in &found_sections {
                let idx = INTERFACE_SECTION_ORDER
                    .iter()
                    .position(|candidate| candidate == section)
                    .map(|idx| idx as isize)
                    .unwrap_or(-1);
                if idx < last_idx {
                    report.error(format!("3. Section order: '{section}' appears after a later section (expected: CommandsVisibility -> CommandsPlacement -> CommandsOrder -> SubsystemsOrder -> GroupsOrder)"));
                    order_ok = false;
                    break;
                }
                last_idx = idx;
            }
            if order_ok {
                report.ok("3. Section order: correct");
            }
        }

        if !report.stopped {
            let dupes = duplicates_preserve_order(&found_sections);
            if dupes.is_empty() {
                report.ok("4. No duplicate sections");
            } else {
                report.error(format!("4. Duplicate sections: {}", dupes.join(", ")));
            }
        }

        let mut vis_names = Vec::<String>::new();
        if !report.stopped {
            if let Some(section) = interface_child(root, "CommandsVisibility", NS_CI) {
                let mut vis_ok = true;
                let mut vis_count = 0usize;
                for cmd in section.children().filter(|child| child.is_element()) {
                    vis_count += 1;
                    let cmd_name = cmd.attribute("name").unwrap_or("");
                    if cmd_name.is_empty() {
                        report.error(
                            "5. CommandsVisibility: Command element without 'name' attribute",
                        );
                        vis_ok = false;
                        continue;
                    }
                    vis_names.push(cmd_name.to_string());
                    all_command_names.push(cmd_name.to_string());
                    let Some(visibility) = interface_child(cmd, "Visibility", NS_CI) else {
                        report.error(format!(
                            "5. CommandsVisibility[{cmd_name}]: missing <Visibility>"
                        ));
                        vis_ok = false;
                        continue;
                    };
                    let Some(common) = interface_child(visibility, "Common", NS_XR) else {
                        report.error(format!(
                            "5. CommandsVisibility[{cmd_name}]: missing <xr:Common>"
                        ));
                        vis_ok = false;
                        continue;
                    };
                    let value = common.text().unwrap_or("").trim();
                    if value != "true" && value != "false" {
                        report.error(format!("5. CommandsVisibility[{cmd_name}]: xr:Common='{value}' (expected true/false)"));
                        vis_ok = false;
                    }
                }
                if vis_ok {
                    report.ok(format!(
                        "5. CommandsVisibility: {vis_count} entries, all valid"
                    ));
                }
            }
        }

        if !report.stopped && !vis_names.is_empty() {
            let dupes = duplicates_preserve_order(&vis_names);
            if dupes.is_empty() {
                report.ok("6. CommandsVisibility: no duplicates");
            } else {
                report.warn(format!(
                    "6. CommandsVisibility: duplicates: {}",
                    dupes.join(", ")
                ));
            }
        }

        if !report.stopped {
            if let Some(section) = interface_child(root, "CommandsPlacement", NS_CI) {
                let mut placement_ok = true;
                let mut placement_count = 0usize;
                for cmd in section.children().filter(|child| child.is_element()) {
                    placement_count += 1;
                    let cmd_name = cmd.attribute("name").unwrap_or("");
                    if cmd_name.is_empty() {
                        report.error("7. CommandsPlacement: Command without 'name' attribute");
                        placement_ok = false;
                        continue;
                    }
                    all_command_names.push(cmd_name.to_string());
                    let group = interface_child_text(cmd, "CommandGroup", NS_CI);
                    if group.trim().is_empty() {
                        report.error(format!(
                            "7. CommandsPlacement[{cmd_name}]: missing or empty <CommandGroup>"
                        ));
                        placement_ok = false;
                        continue;
                    }
                    let placement = interface_child(cmd, "Placement", NS_CI);
                    if placement.is_none() {
                        report.error(format!(
                            "7. CommandsPlacement[{cmd_name}]: missing <Placement>"
                        ));
                        placement_ok = false;
                    } else {
                        let value = placement.and_then(|node| node.text()).unwrap_or("").trim();
                        if value != "Auto" {
                            report.warn(format!(
                                "7. CommandsPlacement[{cmd_name}]: Placement='{value}' (expected Auto)"
                            ));
                        }
                    }
                }
                if placement_ok {
                    report.ok(format!(
                        "7. CommandsPlacement: {placement_count} entries, all valid"
                    ));
                }
            }
        }

        if !report.stopped {
            if let Some(section) = interface_child(root, "CommandsOrder", NS_CI) {
                let mut order_ok = true;
                let mut order_count = 0usize;
                for cmd in section.children().filter(|child| child.is_element()) {
                    order_count += 1;
                    let cmd_name = cmd.attribute("name").unwrap_or("");
                    if cmd_name.is_empty() {
                        report.error("8. CommandsOrder: Command without 'name' attribute");
                        order_ok = false;
                        continue;
                    }
                    all_command_names.push(cmd_name.to_string());
                    let group = interface_child_text(cmd, "CommandGroup", NS_CI);
                    if group.trim().is_empty() {
                        report.error(format!(
                            "8. CommandsOrder[{cmd_name}]: missing or empty <CommandGroup>"
                        ));
                        order_ok = false;
                    }
                }
                if order_ok {
                    report.ok(format!(
                        "8. CommandsOrder: {order_count} entries, all valid"
                    ));
                }
            }
        }

        let mut sub_names = Vec::<String>::new();
        if !report.stopped {
            if let Some(section) = interface_child(root, "SubsystemsOrder", NS_CI) {
                let mut sub_ok = true;
                let mut sub_count = 0usize;
                for sub in section.children().filter(|child| child.is_element()) {
                    sub_count += 1;
                    let value = sub.text().unwrap_or("").trim().to_string();
                    sub_names.push(value.clone());
                    if value.is_empty() {
                        report.error("9. SubsystemsOrder: empty <Subsystem> element");
                        sub_ok = false;
                    } else if !value.starts_with("Subsystem.") {
                        report.error(format!(
                            "9. SubsystemsOrder: '{value}' - expected format Subsystem.X..."
                        ));
                        sub_ok = false;
                    }
                }
                if sub_ok {
                    report.ok(format!(
                        "9. SubsystemsOrder: {sub_count} entries, all valid format"
                    ));
                }
            }
        }

        if !report.stopped && !sub_names.is_empty() {
            let dupes = duplicates_preserve_order(&sub_names);
            if dupes.is_empty() {
                report.ok("10. SubsystemsOrder: no duplicates");
            } else {
                report.warn(format!(
                    "10. SubsystemsOrder: duplicates: {}",
                    dupes.join(", ")
                ));
            }
        }

        let mut group_names = Vec::<String>::new();
        if !report.stopped {
            if let Some(section) = interface_child(root, "GroupsOrder", NS_CI) {
                let mut group_ok = true;
                let mut group_count = 0usize;
                for group in section.children().filter(|child| child.is_element()) {
                    group_count += 1;
                    let value = group.text().unwrap_or("").trim().to_string();
                    group_names.push(value.clone());
                    if value.is_empty() {
                        report.error("11. GroupsOrder: empty <Group> element");
                        group_ok = false;
                    }
                }
                if group_ok {
                    report.ok(format!("11. GroupsOrder: {group_count} entries, all valid"));
                }
            }
        }

        if !report.stopped && !group_names.is_empty() {
            let dupes = duplicates_preserve_order(&group_names);
            if dupes.is_empty() {
                report.ok("12. GroupsOrder: no duplicates");
            } else {
                report.warn(format!("12. GroupsOrder: duplicates: {}", dupes.join(", ")));
            }
        }

        if !report.stopped && !all_command_names.is_empty() {
            let bad_refs = all_command_names
                .iter()
                .filter(|name| !interface_command_ref_valid(name))
                .cloned()
                .collect::<Vec<_>>();
            if bad_refs.is_empty() {
                report.ok(format!(
                    "13. Command reference format: all {} valid",
                    all_command_names.len()
                ));
            } else {
                let shown = bad_refs.iter().take(5).cloned().collect::<Vec<_>>();
                let suffix = if bad_refs.len() > 5 { " ..." } else { "" };
                report.warn(format!(
                    "13. Command reference format: {} unrecognized: {}{suffix}",
                    bad_refs.len(),
                    shown.join(", ")
                ));
            }
        }

        let ok = report.errors == 0;
        let mut output = finish_interface_validation(report, &context_name);
        if let Some(out_file) = &out_file {
            write_utf8_bom(out_file, &output)?;
            output.push_str(&format!("Written to: {}\n", out_file.display()));
        }
        Ok((ok, output, String::new(), out_file, ci_path))
    })();

    match result {
        Ok((ok, stdout, stderr, out_file, artifact)) => {
            let mut artifacts = vec![artifact.display().to_string()];
            if let Some(out_file) = out_file {
                artifacts.push(out_file.display().to_string());
            }
            AdapterOutcome {
                ok,
                summary: if ok {
                    "unica.interface.validate completed with native command interface validator"
                        .to_string()
                } else {
                    "unica.interface.validate failed in native command interface validator"
                        .to_string()
                },
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: if ok {
                    Vec::new()
                } else {
                    vec![stdout.trim().to_string()]
                },
                artifacts,
                stdout: Some(stdout),
                stderr: Some(stderr),
                command: None,
            }
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.interface.validate failed in native command interface validator"
                .to_string(),
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

pub(crate) fn finish_interface_validation(
    report: MxlValidationReporter,
    context_name: &str,
) -> String {
    let checks = report.ok_count + report.errors + report.warnings;
    if report.errors == 0 && report.warnings == 0 && !report.detailed {
        format!("=== Validation OK: CommandInterface ({context_name}) ({checks} checks) ===\n")
    } else {
        let mut lines = report.lines;
        lines.push(String::new());
        lines.push(format!(
            "=== Result: {} errors, {} warnings ({checks} checks) ===",
            report.errors, report.warnings
        ));
        format!("{}\r\n", lines.join("\r\n"))
    }
}

pub(crate) fn interface_context_name(path: &Path) -> String {
    let parts = path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();
    for index in 0..parts.len() {
        if parts[index] == "Subsystems" && index + 1 < parts.len() {
            return parts[index + 1].to_string();
        }
    }
    "Root".to_string()
}

pub(crate) fn interface_child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
    namespace: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children().find(|child| {
        child.is_element()
            && child.tag_name().name() == local_name
            && child.tag_name().namespace() == Some(namespace)
    })
}

pub(crate) fn interface_child_text(
    node: roxmltree::Node<'_, '_>,
    local_name: &str,
    namespace: &str,
) -> String {
    interface_child(node, local_name, namespace)
        .and_then(|child| child.text())
        .unwrap_or("")
        .to_string()
}

pub(crate) fn interface_command_ref_valid(value: &str) -> bool {
    if let Some(uuid) = value.strip_prefix("0:") {
        return is_valid_uuid(uuid);
    }
    let parts = value.split('.').collect::<Vec<_>>();
    if parts.len() == 2 && parts[0] == "CommonCommand" {
        return interface_word(parts[1]);
    }
    if parts.len() == 4 && parts[0].chars().all(|ch| ch.is_ascii_alphabetic()) {
        if parts[1].is_empty() || parts[1].contains(char::is_whitespace) {
            return false;
        }
        return (parts[2] == "StandardCommand" || parts[2] == "Command")
            && interface_word(parts[3]);
    }
    false
}

pub(crate) fn interface_word(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|ch| ch == '_' || ch.is_alphanumeric())
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
        "interface-validate" => Some(Ok(validate_interface(args, context))),
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
        "interface-edit" => Some(edit_interface(args, context)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workspace::WorkspaceContext;
    use serde_json::{Map, Value};
    use std::fs;

    fn temp_context(name: &str) -> WorkspaceContext {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("unica-interface-{name}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build").join("unica"),
            workspace_epoch: 1,
        }
    }

    #[test]
    fn append_to_missing_commands_visibility_inserts_section_before_later_sections() {
        let mut text = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<CommandInterface xmlns=\"{INTERFACE_CI_NS}\" xmlns:xr=\"{INTERFACE_XR_NS}\" version=\"2.17\">\n\
\t<CommandsPlacement>\n\
\t</CommandsPlacement>\n\
</CommandInterface>"
        );

        interface_text_append_to_section(
            &mut text,
            "CommandsVisibility",
            "<Command name=\"Catalog.Products.StandardCommand.OpenList\"><Visibility><xr:Common>false</xr:Common></Visibility></Command>",
        )
        .unwrap();

        let visibility_index = text.find("<CommandsVisibility>").unwrap();
        let placement_index = text.find("<CommandsPlacement>").unwrap();
        assert!(visibility_index < placement_index, "{text}");
        assert!(text.contains("<xr:Common>false</xr:Common>"));
    }

    #[test]
    fn create_if_missing_does_not_leave_file_after_failed_operation() {
        let context = temp_context("create-if-missing-error");
        let ci_rel = "src/Subsystems/NewSales/Ext/CommandInterface.xml";
        let ci_path = context.cwd.join(ci_rel);
        let mut args = Map::new();
        args.insert("CIPath".to_string(), Value::String(ci_rel.to_string()));
        args.insert(
            "Operation".to_string(),
            Value::String("subsystem-order".to_string()),
        );
        args.insert("Value".to_string(), Value::String("[]".to_string()));
        args.insert("CreateIfMissing".to_string(), Value::Bool(true));
        args.insert("NoValidate".to_string(), Value::Bool(true));

        let outcome = edit_interface(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            !ci_path.exists(),
            "partial file left at {}",
            ci_path.display()
        );
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn create_if_missing_hide_show_create_valid_command_interface() {
        for (operation, expected_common) in [("hide", "false"), ("show", "true")] {
            let context = temp_context(&format!("create-if-missing-{operation}"));
            let ci_rel = format!("src/Subsystems/New{operation}/Ext/CommandInterface.xml");
            let ci_path = context.cwd.join(&ci_rel);
            let mut args = Map::new();
            args.insert("CIPath".to_string(), Value::String(ci_rel));
            args.insert(
                "Operation".to_string(),
                Value::String(operation.to_string()),
            );
            args.insert(
                "Value".to_string(),
                Value::String("Catalog.Products.StandardCommand.OpenList".to_string()),
            );
            args.insert("CreateIfMissing".to_string(), Value::Bool(true));

            let outcome = edit_interface(&args, &context);

            assert!(outcome.ok, "{operation}: {outcome:?}");
            assert!(
                outcome
                    .stdout
                    .as_deref()
                    .is_some_and(|stdout| stdout.contains("Validation OK")),
                "{operation}: validation did not run successfully: {outcome:?}"
            );
            let text = fs::read_to_string(&ci_path).unwrap();
            assert!(text.contains("<CommandsVisibility>"), "{operation}: {text}");
            assert!(
                text.contains(&format!("<xr:Common>{expected_common}</xr:Common>")),
                "{operation}: {text}"
            );
            let _ = fs::remove_dir_all(&context.workspace_root);
        }
    }
}
