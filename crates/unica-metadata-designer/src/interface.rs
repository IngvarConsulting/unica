//! Actual Designer interface parsing, read projections, validation and in-memory edit algorithms.
use roxmltree::Document;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use unica_metadata::interface::*;
use unica_source_contract::discovery::ProjectDiscoveryRead;
use unica_source_designer::read::{path_exists, path_is_dir, read_string};
pub const INTERFACE_CI_NS: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";

pub const INTERFACE_XR_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";

pub const INTERFACE_XS_NS: &str = "http://www.w3.org/2001/XMLSchema";

pub const INTERFACE_XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

pub const INTERFACE_SECTION_ORDER: &[&str] = &[
    "CommandsVisibility",
    "CommandsPlacement",
    "CommandsOrder",
    "SubsystemsOrder",
    "GroupsOrder",
];

pub fn interface_edit_operations(
    reader: &dyn ProjectDiscoveryRead,
    args: &Map<String, Value>,
    cwd: &Path,
    operation: Option<&str>,
    definition_file: Option<PathBuf>,
) -> Result<Vec<(String, Value)>, String> {
    if let Some(definition_file) = definition_file {
        let definition_file = unica_operation_contract::input::absolutize(definition_file, cwd);
        let text = read_string(reader, &definition_file)
            .map_err(|err| format!("failed to read {}: {err}", definition_file.display()))?;
        let parsed: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("failed to parse {}: {err}", definition_file.display()))?;
        Ok(interface_edit_operations_from_value(parsed, operation))
    } else {
        Ok(vec![(
            operation.unwrap_or("").to_string(),
            Value::String(
                unica_operation_contract::input::string_arg(args, &["value", "Value"])
                    .unwrap_or_default()
                    .to_string(),
            ),
        )])
    }
}

pub fn interface_edit_operations_from_value(
    parsed: Value,
    operation: Option<&str>,
) -> Vec<(String, Value)> {
    let items = match parsed {
        Value::Array(items) => items,
        other => vec![other],
    };
    items
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
        .collect()
}

pub fn interface_value_list(value: &Value) -> Result<Vec<String>, String> {
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

pub fn interface_json_array_strings(value: &Value) -> Result<Vec<String>, String> {
    let Some(items) = value.as_array() else {
        return Err("value must be an array".to_string());
    };
    Ok(items.iter().map(interface_json_string).collect())
}

pub fn interface_json_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Null => "None".to_string(),
        other => other.to_string(),
    }
}

pub fn emit_empty_command_interface_document(format_version: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<CommandInterface xmlns=\"{INTERFACE_CI_NS}\" xmlns:xr=\"{INTERFACE_XR_NS}\" xmlns:xs=\"{INTERFACE_XS_NS}\" xmlns:xsi=\"{INTERFACE_XSI_NS}\" version=\"{}\">\n\
</CommandInterface>",
        unica_xml_format::text::escape_xml(format_version)
    )
}

/// Командный интерфейс в форме выгрузки: BOM, CRLF, без перевода строки
/// в конце. Пустой интерфейс платформа не выгружает вовсе, поэтому этот текст
/// только основа первой правки.
pub fn platform_empty_command_interface(format_version: &str) -> String {
    format!(
        "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<CommandInterface xmlns=\"{INTERFACE_CI_NS}\" xmlns:xr=\"{INTERFACE_XR_NS}\" xmlns:xs=\"{INTERFACE_XS_NS}\" xmlns:xsi=\"{INTERFACE_XSI_NS}\" version=\"{}\">\r\n\
</CommandInterface>",
        unica_xml_format::text::escape_xml(format_version)
    )
}

pub fn interface_text_do_hide(
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
                    unica_xml_format::text::escape_xml(&cmd)
                );
                interface_text_append_to_section(text, "CommandsVisibility", &fragment)?;
                counters.added += 1;
                stdout.push_str(&format!("[INFO] Hidden: {cmd}\n"));
            }
        }
    }
    Ok(())
}

pub fn interface_text_do_show(
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
                    unica_xml_format::text::escape_xml(&cmd)
                );
                interface_text_append_to_section(text, "CommandsVisibility", &fragment)?;
                counters.added += 1;
                stdout.push_str(&format!("[INFO] Shown: {cmd}\n"));
            }
        }
    }
    Ok(())
}

pub fn interface_text_do_place(
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
            unica_xml_format::text::escape_xml(&cmd_name),
            unica_xml_format::text::escape_xml(&group_name)
        );
        interface_text_append_to_section(text, "CommandsPlacement", &fragment)?;
        counters.added += 1;
        stdout.push_str(&format!("[INFO] Placed: {cmd_name} -> {group_name}\n"));
    }
    Ok(())
}

pub fn interface_text_do_order(
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
    interface_text_replace_group_commands(text, &group_name, &commands)?;
    stdout.push_str(&format!(
        "[INFO] Set order for {group_name} : {} commands\n",
        commands.len()
    ));
    Ok(())
}

pub fn interface_text_do_subsystem_order(
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
        .map(|sub| {
            format!(
                "<Subsystem>{}</Subsystem>",
                unica_xml_format::text::escape_xml(sub)
            )
        })
        .collect::<Vec<_>>();
    interface_text_replace_section_items(text, "SubsystemsOrder", &fragments)?;
    stdout.push_str(&format!(
        "[INFO] Set subsystem order: {} entries\n",
        subsystems.len()
    ));
    Ok(())
}

pub fn interface_text_do_group_order(
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
        .map(|group| {
            format!(
                "<Group>{}</Group>",
                unica_xml_format::text::escape_xml(group)
            )
        })
        .collect::<Vec<_>>();
    interface_text_replace_section_items(text, "GroupsOrder", &fragments)?;
    stdout.push_str(&format!(
        "[INFO] Set group order: {} entries\n",
        groups.len()
    ));
    Ok(())
}

/// Элемент `<Command>` секции `CommandsOrder`: команда, её группа и
/// абсолютные границы элемента в тексте.
struct InterfaceOrderedCommand {
    name: String,
    group: String,
    start: usize,
    end: usize,
}

fn interface_text_ordered_commands(text: &str) -> Vec<InterfaceOrderedCommand> {
    let Some((_, content_start, close_start, _, _, _)) =
        unica_xml_format::text::find_element_bounds(text, "CommandsOrder", 0)
    else {
        return Vec::new();
    };
    let body = &text[content_start..close_start];
    let mut commands = Vec::new();
    let mut offset = 0usize;
    while let Some(rel_start) = body[offset..].find("<Command") {
        let start = offset + rel_start;
        let after = start + "<Command".len();
        if !body[after..].starts_with(|ch: char| ch.is_whitespace() || ch == '>') {
            offset = after;
            continue;
        }
        let Some(gt_rel) = body[start..].find('>') else {
            break;
        };
        let gt = start + gt_rel;
        let Some(close_rel) = body[gt + 1..].find("</Command>") else {
            break;
        };
        let end = gt + 1 + close_rel + "</Command>".len();
        let open_tag = &body[start..=gt];
        let name = open_tag
            .split_once("name=\"")
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(name, _)| unica_xml_format::text::unescape_xml(name))
            .unwrap_or_default();
        let group = interface_text_element_value(&body[start..end], "CommandGroup")
            .map(|group| unica_xml_format::text::unescape_xml(group.trim()))
            .unwrap_or_default();
        commands.push(InterfaceOrderedCommand {
            name,
            group,
            start: content_start + start,
            end: content_start + end,
        });
        offset = end;
    }
    commands
}

/// Сохранённый порядок команд одной группы.
pub fn interface_text_group_command_order(text: &str, group: &str) -> Vec<String> {
    interface_text_ordered_commands(text)
        .into_iter()
        .filter(|command| command.group == group)
        .map(|command| command.name)
        .collect()
}

/// Значения прямых элементов секции: `Group` у `GroupsOrder`, `Subsystem`
/// у `SubsystemsOrder`.
pub fn interface_text_direct_item_values(text: &str, section: &str, item: &str) -> Vec<String> {
    let Some((_, content_start, close_start, _, _, _)) =
        unica_xml_format::text::find_element_bounds(text, section, 0)
    else {
        return Vec::new();
    };
    let body = &text[content_start..close_start];
    let open = format!("<{item}>");
    let close = format!("</{item}>");
    let mut values = Vec::new();
    let mut offset = 0usize;
    while let Some(rel_start) = body[offset..].find(&open) {
        let value_start = offset + rel_start + open.len();
        let Some(rel_end) = body[value_start..].find(&close) else {
            break;
        };
        values.push(unica_xml_format::text::unescape_xml(
            body[value_start..value_start + rel_end].trim(),
        ));
        offset = value_start + rel_end + close.len();
    }
    values
}

/// Изменение порядка — перестановка уже сохранённого состава.
///
/// Список, где элемент пропущен, повторён или добавлен, не порядок прежних
/// элементов: пропуск не означает удаления настройки, а добавка — её
/// расширения. Пока порядок не сохранён, список задаёт его впервые; повтор
/// недопустим и тогда.
pub fn interface_order_membership(
    stored: &[String],
    requested: &[String],
    scope: &str,
) -> Result<(), String> {
    let mut seen = HashSet::new();
    if let Some(repeated) = requested.iter().find(|name| !seen.insert(name.as_str())) {
        return Err(format!(
            "`{repeated}` is listed more than once; an order names each entry once"
        ));
    }
    if stored.is_empty() {
        return Ok(());
    }
    let quoted = |names: Vec<&String>| {
        names
            .into_iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let missing = stored
        .iter()
        .filter(|name| !requested.contains(name))
        .collect::<Vec<_>>();
    let foreign = requested
        .iter()
        .filter(|name| !stored.contains(name))
        .collect::<Vec<_>>();
    let mut differences = Vec::new();
    if !missing.is_empty() {
        differences.push(format!("missing {}", quoted(missing)));
    }
    if !foreign.is_empty() {
        differences.push(format!("not stored {}", quoted(foreign)));
    }
    if differences.is_empty() && stored.len() != requested.len() {
        differences.push("the stored order repeats an entry".to_string());
    }
    if differences.is_empty() {
        return Ok(());
    }
    Err(format!(
        "not a permutation of the stored {scope}: {}",
        differences.join("; ")
    ))
}

/// Переписывает порядок одной группы на месте её прежних элементов.
///
/// Элементы других групп и их взаимное расположение не меняются. Переставленная
/// команда переносит свой прежний элемент без переформатирования; новая
/// получает элемент той же формы, что пишет платформа. Если у группы порядка
/// ещё нет, её команды дописываются в конец секции.
pub fn interface_text_replace_group_commands(
    text: &mut String,
    group: &str,
    commands: &[String],
) -> Result<(), String> {
    let slots = interface_text_ordered_commands(text)
        .into_iter()
        .filter(|command| command.group == group)
        .collect::<Vec<_>>();
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let indent = slots.first().map_or("", |first| {
        let line = text[..first.start].rsplit('\n').next().unwrap_or_default();
        &line[..line.len() - line.trim_start_matches(['\t', ' ']).len()]
    });
    let elements = commands
        .iter()
        .map(|name| {
            slots
                .iter()
                .find(|slot| slot.name == *name)
                .map(|slot| text[slot.start..slot.end].to_string())
                .unwrap_or_else(|| {
                    interface_platform_element(
                        &format!(
                            "<Command name=\"{}\"><CommandGroup>{}</CommandGroup></Command>",
                            unica_xml_format::text::escape_xml(name),
                            unica_xml_format::text::escape_xml(group)
                        ),
                        indent,
                        newline,
                    )
                })
        })
        .collect::<Vec<_>>();
    let Some(last) = slots.last() else {
        for element in &elements {
            interface_text_append_to_section(text, "CommandsOrder", element)?;
        }
        return Ok(());
    };
    let (_, content_start, _, _, _, _) =
        unica_xml_format::text::find_element_bounds(text, "CommandsOrder", 0)
            .ok_or_else(|| "No <CommandsOrder> element found".to_string())?;
    let mut edits = Vec::with_capacity(slots.len() + 1);
    for (index, slot) in slots.iter().enumerate() {
        match elements.get(index) {
            Some(element) => edits.push((slot.start, slot.end, element.clone())),
            None => {
                let lead = interface_trailing_ws_start(text, content_start, slot.start);
                edits.push((lead, slot.end, String::new()));
            }
        }
    }
    if elements.len() > slots.len() {
        let lead = &text[interface_trailing_ws_start(text, content_start, last.start)..last.start];
        let separator = if lead.contains('\n') && !lead.contains("\r\n") {
            "\n"
        } else {
            "\r\n"
        };
        let indent = lead.rsplit('\n').next().unwrap_or_default();
        let tail = elements[slots.len()..]
            .iter()
            .map(|element| format!("{separator}{indent}{element}"))
            .collect::<String>();
        edits.push((last.end, last.end, tail));
    }
    edits.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    for (start, end, replacement) in edits {
        text.replace_range(start..end, &replacement);
    }
    Ok(())
}

pub fn interface_text_command_common(text: &str, section: &str, cmd_name: &str) -> Option<String> {
    unica_xml_format::text::find_element_bounds(text, section, 0)?;
    let (cmd_start, cmd_end) = interface_text_command_bounds_in_section(text, section, cmd_name)?;
    let command = &text[cmd_start..cmd_end];
    let value = interface_text_element_value(command, "xr:Common")
        .or_else(|| interface_text_element_value(command, "Common"))?;
    Some(value.trim().to_string())
}

pub fn interface_text_replace_command_common(
    text: &mut String,
    section: &str,
    cmd_name: &str,
    value: &str,
) -> Result<(), String> {
    interface_text_replace_command_child(text, section, cmd_name, "xr:Common", value)
        .or_else(|_| interface_text_replace_command_child(text, section, cmd_name, "Common", value))
}

pub fn interface_text_replace_command_child(
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
    text.replace_range(
        value_start..value_end,
        &unica_xml_format::text::escape_xml(value),
    );
    Ok(())
}

pub fn interface_text_command_bounds_in_section(
    text: &str,
    section: &str,
    cmd_name: &str,
) -> Option<(usize, usize)> {
    let (_, content_start, close_start, _, _, _) =
        unica_xml_format::text::find_element_bounds(text, section, 0)?;
    let body = &text[content_start..close_start];
    let (rel_start, rel_end) = interface_text_command_bounds(body, cmd_name)?;
    Some((content_start + rel_start, content_start + rel_end))
}

pub fn interface_text_command_bounds(section_body: &str, cmd_name: &str) -> Option<(usize, usize)> {
    let escaped_name = unica_xml_format::text::escape_xml(cmd_name);
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

pub fn interface_text_element_value(text: &str, tag: &str) -> Option<String> {
    let start_tag = format!("<{tag}>");
    let close_tag = format!("</{tag}>");
    let start = text.find(&start_tag)? + start_tag.len();
    let end = start + text[start..].find(&close_tag)?;
    Some(text[start..end].to_string())
}

pub fn interface_text_append_to_section(
    text: &mut String,
    section: &str,
    fragment: &str,
) -> Result<(), String> {
    if unica_xml_format::text::find_element_bounds(text, section, 0).is_none() {
        return interface_text_insert_section(text, section, &[fragment.to_string()]);
    }
    let (_, content_start, close_start, _, _, _) =
        unica_xml_format::text::find_element_bounds(text, section, 0)
            .ok_or_else(|| format!("No <{section}> element found"))?;
    let body = &text[content_start..close_start];
    let child_indent = interface_text_detect_indent(body)
        .or_else(|| interface_text_detect_empty_indent(body))
        .unwrap_or_else(|| "\t\t".to_string());
    let fragment = interface_platform_element(fragment, &child_indent, "\r\n");
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

pub fn interface_text_replace_section_items(
    text: &mut String,
    section: &str,
    fragments: &[String],
) -> Result<(), String> {
    if let Some((_, content_start, close_start, _, _, _)) =
        unica_xml_format::text::find_element_bounds(text, section, 0)
    {
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

pub fn interface_text_insert_section(
    text: &mut String,
    section: &str,
    fragments: &[String],
) -> Result<(), String> {
    let (_, content_start, close_start, _, _, _) =
        unica_xml_format::text::find_element_bounds(text, "CommandInterface", 0)
            .ok_or_else(|| "No <CommandInterface> root element found".to_string())?;
    let root_body = &text[content_start..close_start];
    let root_indent = interface_text_detect_indent(root_body).unwrap_or_else(|| "\t".to_string());
    let body = if fragments.is_empty() {
        format!("\r\n{root_indent}")
    } else {
        interface_text_section_body(&format!("{root_indent}\t"), &root_indent, fragments)
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

pub fn interface_text_next_ordered_section_start(
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
            unica_xml_format::text::find_element_bounds(text, candidate, content_start).and_then(
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

pub fn interface_section_order_index(section: &str) -> Option<usize> {
    INTERFACE_SECTION_ORDER
        .iter()
        .position(|candidate| *candidate == section)
}

pub fn interface_text_section_body(
    child_indent: &str,
    parent_indent: &str,
    fragments: &[String],
) -> String {
    let mut body = String::new();
    for fragment in fragments {
        body.push_str("\r\n");
        body.push_str(child_indent);
        body.push_str(&interface_platform_element(fragment, child_indent, "\r\n"));
    }
    body.push_str("\r\n");
    body.push_str(parent_indent);
    body
}

/// Раскладывает сжатый элемент так, как его выгружает платформа: элемент
/// с дочерними элементами занимает строки, лист с текстом — одну строку,
/// каждый уровень вложенности добавляет табуляцию к `indent`. Первая строка
/// идёт без отступа: его ставит место вставки.
fn interface_platform_element(fragment: &str, indent: &str, newline: &str) -> String {
    let mut tokens = Vec::new();
    let mut rest = fragment;
    while !rest.is_empty() {
        let end = if rest.starts_with('<') {
            rest.find('>').map_or(rest.len(), |close| close + 1)
        } else {
            rest.find('<').unwrap_or(rest.len())
        };
        // Пробелы между тегами — прежняя раскладка, а не текст элемента.
        if !rest[..end].trim().is_empty() {
            tokens.push(&rest[..end]);
        }
        rest = &rest[end..];
    }
    let mut lines = Vec::new();
    let mut depth = 0usize;
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index];
        let line = if token.starts_with("</") {
            depth = depth.saturating_sub(1);
            index += 1;
            (depth, token.to_string())
        } else if token.ends_with("/>") || !token.starts_with('<') {
            index += 1;
            (depth, token.to_string())
        } else if let Some(leaf) = tokens
            .get(index + 1..index + 3)
            .filter(|leaf| !leaf[0].starts_with('<') && leaf[1].starts_with("</"))
        {
            index += 3;
            (depth, format!("{token}{}{}", leaf[0], leaf[1]))
        } else if tokens
            .get(index + 1)
            .is_some_and(|next| next.starts_with("</"))
        {
            index += 2;
            (depth, format!("{token}{}", tokens[index - 1]))
        } else {
            index += 1;
            depth += 1;
            (depth - 1, token.to_string())
        };
        lines.push(line);
    }
    let mut element = String::new();
    for (position, (depth, line)) in lines.into_iter().enumerate() {
        if position > 0 {
            element.push_str(newline);
            element.push_str(indent);
            element.push_str(&"\t".repeat(depth));
        }
        element.push_str(&line);
    }
    element
}

pub fn interface_text_count_commands_for_group(
    text: &str,
    section: &str,
    group_name: &str,
) -> usize {
    let Some((_, content_start, close_start, _, _, _)) =
        unica_xml_format::text::find_element_bounds(text, section, 0)
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

pub fn interface_text_count_direct_items(text: &str, section: &str, item: &str) -> usize {
    let Some((_, content_start, close_start, _, _, _)) =
        unica_xml_format::text::find_element_bounds(text, section, 0)
    else {
        return 0;
    };
    text[content_start..close_start]
        .match_indices(&format!("<{item}>"))
        .count()
}

pub fn interface_text_detect_indent(body: &str) -> Option<String> {
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

pub fn interface_text_detect_empty_indent(body: &str) -> Option<String> {
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

pub fn interface_parent_indent(child_indent: &str) -> String {
    child_indent
        .strip_suffix('\t')
        .or_else(|| child_indent.strip_suffix("    "))
        .unwrap_or("")
        .to_string()
}

pub fn interface_trailing_ws_start(text: &str, min: usize, end: usize) -> usize {
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

pub fn interface_normalize_lexical_path(path: &Path) -> PathBuf {
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

pub fn interface_metadata_owner_path(
    reader: &dyn ProjectDiscoveryRead,
    ci_path: &Path,
) -> Result<PathBuf, String> {
    let ci_path = interface_normalize_lexical_path(ci_path);
    if ci_path.file_name().and_then(|value| value.to_str()) != Some("CommandInterface.xml") {
        return Err(format!(
            "interface.edit CIPath must point to CommandInterface.xml: {}",
            ci_path.display()
        ));
    }
    let ext_dir = ci_path.parent().ok_or_else(|| {
        format!(
            "interface.edit cannot resolve metadata owner for {}",
            ci_path.display()
        )
    })?;
    if ext_dir.file_name().and_then(|value| value.to_str()) != Some("Ext") {
        return Err(format!(
            "interface.edit CommandInterface.xml must be located in an Ext directory: {}",
            ci_path.display()
        ));
    }
    let object_dir = ext_dir.parent().ok_or_else(|| {
        format!(
            "interface.edit cannot resolve metadata owner for {}",
            ci_path.display()
        )
    })?;
    let configuration_owner = object_dir.join("Configuration.xml");
    let owner_path = if reader.path_is_file(&configuration_owner) {
        configuration_owner
    } else {
        object_dir.with_extension("xml")
    };
    if !reader.path_is_file(&owner_path) {
        return Err(format!(
            "interface.edit metadata owner is unavailable for {}: expected {}",
            ci_path.display(),
            owner_path.display()
        ));
    }
    Ok(owner_path)
}

pub fn interface_json_object(value: &Value) -> Result<Value, String> {
    if value.is_object() {
        Ok(value.clone())
    } else if let Some(text) = value.as_str() {
        serde_json::from_str(text).map_err(|err| format!("failed to parse JSON value: {err}"))
    } else {
        Err("value must be a JSON object".to_string())
    }
}

pub fn interface_json_array(value: &Value) -> Result<Value, String> {
    if value.is_array() {
        Ok(value.clone())
    } else if let Some(text) = value.as_str() {
        serde_json::from_str(text).map_err(|err| format!("failed to parse JSON value: {err}"))
    } else {
        Err("value must be a JSON array".to_string())
    }
}

pub fn normalize_interface_command_name(name: &str, stdout: &mut String) -> String {
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

pub fn interface_type_norm(value: &str) -> Option<&'static str> {
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

pub fn finish_interface_validation(
    report: unica_xml_format::validation::XmlValidationReporter,
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

pub fn interface_context_name(path: &Path) -> String {
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

pub fn interface_child<'a, 'input>(
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

pub fn interface_child_text(
    node: roxmltree::Node<'_, '_>,
    local_name: &str,
    namespace: &str,
) -> String {
    interface_child(node, local_name, namespace)
        .and_then(|child| child.text())
        .unwrap_or("")
        .to_string()
}

pub fn interface_command_ref_valid(value: &str) -> bool {
    if let Some(uuid) = value.strip_prefix("0:") {
        return unica_xml_format::names::is_valid_uuid(uuid);
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

pub fn interface_word(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|ch| ch == '_' || ch.is_alphanumeric())
}

/// Resolve the exact CommandInterface.xml file opened by `interface.validate`.
///
/// Missing targets are returned unchanged so the handler retains its
/// established file-not-found diagnostic.
pub fn resolve_interface_path(
    args: &Map<String, Value>,
    reader: &dyn ProjectDiscoveryRead,
    cwd: &Path,
) -> Result<PathBuf, String> {
    let raw_path = unica_operation_contract::input::required_path(
        args,
        &["ciPath", "CIPath", "path", "Path"],
        "CIPath",
    )?;
    let mut ci_path = unica_operation_contract::input::absolutize(raw_path, cwd);
    if path_is_dir(reader, &ci_path) {
        ci_path = ci_path.join("Ext").join("CommandInterface.xml");
    }
    if !path_exists(reader, &ci_path)
        && ci_path.file_name().and_then(|value| value.to_str()) == Some("CommandInterface.xml")
    {
        let candidate = ci_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("Ext")
            .join("CommandInterface.xml");
        if path_exists(reader, &candidate) {
            ci_path = candidate;
        }
    }
    Ok(ci_path)
}
pub fn validate_interface_image(
    args: &Map<String, Value>,
    ci_path: &Path,
    text: &str,
) -> Result<(bool, String, String, PathBuf), String> {
    const NS_CI: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";
    const NS_XR: &str = "http://v8.1c.ru/8.3/xcf/readable";
    let ci_path = ci_path.to_path_buf();
    let context_name = interface_context_name(&ci_path);
    let detailed = unica_operation_contract::input::bool_arg(args, &["detailed", "Detailed"]);
    let max_errors = unica_operation_contract::input::int_arg(args, &["maxErrors", "MaxErrors"])
        .unwrap_or(30)
        .max(0) as usize;
    let mut report = unica_xml_format::validation::XmlValidationReporter::new(max_errors, detailed);
    let mut all_command_names = Vec::<String>::new();

    report.lines.push(format!(
        "=== Validation: CommandInterface ({context_name}) ==="
    ));
    report.lines.push(String::new());

    let doc = match Document::parse(text.trim_start_matches('\u{feff}')) {
        Ok(doc) => doc,
        Err(error) => {
            report.error(format!("1. XML parse error: {error}"));
            report.stopped = true;
            let output = finish_interface_validation(report, &context_name);
            return Ok((false, output, String::new(), ci_path));
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
            if INTERFACE_SECTION_ORDER.contains(&local_name)
                && child.tag_name().namespace() == Some(NS_CI)
            {
                found_sections.push(local_name.to_string());
            } else {
                let namespace = child.tag_name().namespace().unwrap_or("");
                invalid_elements.push(format!("{{{namespace}}}{local_name}"));
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
        let dupes =
            unica_operation_contract::collections::duplicates_preserve_order(&found_sections);
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
                    report.error("5. CommandsVisibility: Command element without 'name' attribute");
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
        let dupes = unica_operation_contract::collections::duplicates_preserve_order(&vis_names);
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
        let dupes = unica_operation_contract::collections::duplicates_preserve_order(&sub_names);
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
        let dupes = unica_operation_contract::collections::duplicates_preserve_order(&group_names);
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
    let output = finish_interface_validation(report, &context_name);
    Ok((ok, output, String::new(), ci_path))
}

#[cfg(test)]
#[path = "../tests/platform/interface_codec.rs"]
mod tests;
