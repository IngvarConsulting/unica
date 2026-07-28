#![allow(dead_code, unused_imports)]
use super::inspection_arguments::ArgumentAccess;

use crate::application::NativeWriterResult;
use crate::domain::workspace::WorkspaceContext;
use crate::operations::PlatformWriterSession;
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::common::*;
use super::compile_transaction::CompileTransaction;
use super::{cf::*, cfe::*, dcs::*, form::*, meta::*, mxl::*, role::*, subsystem::*, template::*};
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

struct InterfaceEditInput {
    source: InterfaceEditSource,
    interface_path: PathBuf,
    create_if_missing: bool,
    skip_validation_output: bool,
}

enum InterfaceEditSource {
    Semantic(unica_format_core::commands::InterfaceEdit),
    #[cfg(test)]
    Legacy {
        definition_file: Option<PathBuf>,
        operation: String,
        value: String,
    },
}

enum InterfaceNativeEdit {
    Replace(unica_format_core::commands::CommandInterfaceDefinition),
    Hide(Vec<String>),
    Show(Vec<String>),
    Place(unica_format_core::commands::InterfacePlacement),
    Order(unica_format_core::commands::InterfaceCommandOrder),
    OrderSubsystems(Vec<unica_format_core::commands::SubsystemName>),
    OrderGroups(Vec<unica_format_core::commands::InterfaceGroupName>),
    #[cfg(test)]
    LegacyPlace(Value),
    #[cfg(test)]
    LegacyOrder(Value),
    #[cfg(test)]
    LegacyOrderSubsystems(Value),
    #[cfg(test)]
    LegacyOrderGroups(Value),
}

#[cfg(test)]
pub(crate) fn edit_interface(
    args: &impl ArgumentAccess,
    context: &WorkspaceContext,
) -> NativeWriterResult {
    let parsed = (|| -> Result<InterfaceEditInput, String> {
        let definition_file = path_arg(args, &["definitionFile", "DefinitionFile"]);
        let operation = string_arg(args, &["operation", "Operation"]);
        if definition_file.is_some() && operation.is_some() {
            return Err("Cannot use both -DefinitionFile and -Operation".to_string());
        }
        if definition_file.is_none() && operation.is_none() {
            return Err("Either -DefinitionFile or -Operation is required".to_string());
        }
        Ok(InterfaceEditInput {
            source: InterfaceEditSource::Legacy {
                definition_file,
                operation: operation.unwrap_or_default().to_string(),
                value: string_arg(args, &["value", "Value"])
                    .unwrap_or_default()
                    .to_string(),
            },
            interface_path: required_path(args, &["ciPath", "CIPath", "path", "Path"], "CIPath")?,
            create_if_missing: bool_arg(args, &["createIfMissing", "CreateIfMissing"]),
            skip_validation_output: bool_arg(args, &["noValidate", "NoValidate"]),
        })
    })();
    match parsed {
        Ok(input) => edit_interface_input(input, context),
        Err(error) => interface_edit_failure(error),
    }
}

fn edit_interface_input(
    input: InterfaceEditInput,
    context: &WorkspaceContext,
) -> NativeWriterResult {
    let edit_result = (|| -> Result<(String, PathBuf, Vec<String>), String> {
        let InterfaceEditInput {
            source,
            interface_path,
            create_if_missing,
            skip_validation_output,
        } = input;
        let mut ci_path = absolutize(interface_path, &context.cwd);
        let format_version = crate::domain::format_profile::ACTIVE_FORMAT_PROFILE
            .export_format
            .to_string();

        let mut stdout = String::new();
        let source_exists = ci_path.is_file();
        let created_new = if !source_exists {
            if create_if_missing {
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
        let metadata_owner_path = interface_metadata_owner_path(&ci_path)?;
        let metadata_owner_preimage = fs::read(&metadata_owner_path).map_err(|error| {
            format!(
                "failed to read interface metadata owner {}: {error}",
                metadata_owner_path.display()
            )
        })?;
        let source_snapshot = source_exists
            .then(|| read_utf8_sig_snapshot(&ci_path))
            .transpose()?;
        let source_text = source_snapshot
            .as_ref()
            .map(|snapshot| snapshot.text.clone())
            .unwrap_or_default();
        let mut text = source_text.clone();
        text = lxml_parser_normalized_text(&text);
        if text.is_empty() {
            text = emit_empty_command_interface_document(&format_version);
        }
        let mut transaction = CompileTransaction::new();
        let operations = match source {
            InterfaceEditSource::Semantic(edit) => vec![interface_native_edit(edit)],
            #[cfg(test)]
            InterfaceEditSource::Legacy {
                definition_file,
                operation,
                value,
            } => interface_edit_operations_guarded(
                &context.cwd,
                (!operation.is_empty()).then_some(operation.as_str()),
                &value,
                definition_file,
                &mut transaction,
            )?
            .into_iter()
            .map(|(operation, value)| interface_legacy_edit(&operation, value))
            .collect::<Result<Vec<_>, _>>()?,
        };
        let mut counters = InterfaceEditCounters::default();
        for operation in operations {
            match operation {
                InterfaceNativeEdit::Replace(definition) => {
                    interface_text_replace_section_items(&mut text, "CommandsPlacement", &[])?;
                    for placement in definition.items() {
                        interface_text_do_semantic_place(
                            &mut text,
                            placement,
                            &mut counters,
                            &mut stdout,
                        )?;
                    }
                }
                InterfaceNativeEdit::Hide(commands) => {
                    interface_text_do_hide(&mut text, commands, &mut counters, &mut stdout)?;
                }
                InterfaceNativeEdit::Show(commands) => {
                    interface_text_do_show(&mut text, commands, &mut counters, &mut stdout)?;
                }
                InterfaceNativeEdit::Place(value) => {
                    interface_text_do_semantic_place(&mut text, &value, &mut counters, &mut stdout)?
                }
                InterfaceNativeEdit::Order(value) => {
                    interface_text_do_semantic_order(&mut text, &value, &mut counters, &mut stdout)?
                }
                InterfaceNativeEdit::OrderSubsystems(value) => {
                    interface_text_do_semantic_subsystem_order(
                        &mut text,
                        &value,
                        &mut counters,
                        &mut stdout,
                    )?;
                }
                InterfaceNativeEdit::OrderGroups(value) => {
                    interface_text_do_semantic_group_order(
                        &mut text,
                        &value,
                        &mut counters,
                        &mut stdout,
                    )?;
                }
                #[cfg(test)]
                InterfaceNativeEdit::LegacyPlace(value) => {
                    interface_text_do_place(&mut text, &value, &mut counters, &mut stdout)?
                }
                #[cfg(test)]
                InterfaceNativeEdit::LegacyOrder(value) => {
                    interface_text_do_order(&mut text, &value, &mut counters, &mut stdout)?
                }
                #[cfg(test)]
                InterfaceNativeEdit::LegacyOrderSubsystems(value) => {
                    interface_text_do_subsystem_order(
                        &mut text,
                        &value,
                        &mut counters,
                        &mut stdout,
                    )?;
                }
                #[cfg(test)]
                InterfaceNativeEdit::LegacyOrderGroups(value) => {
                    interface_text_do_group_order(&mut text, &value, &mut counters, &mut stdout)?;
                }
            }
        }

        let serialized = lxml_tree_serialized_text_like_source(&text, &source_text);
        let replacement = utf8_bom_bytes(&serialized);
        if let Some(snapshot) = &source_snapshot {
            transaction.replace_bytes(&ci_path, &snapshot.raw, replacement)?;
        } else {
            transaction.create_bytes(&ci_path, replacement)?;
        }
        if !transaction.protects_path(&metadata_owner_path)? {
            transaction.guard_exact_preimage(&metadata_owner_path, metadata_owner_preimage)?;
        }
        guard_active_format_dependencies(
            &mut transaction,
            &[ci_path.as_path(), metadata_owner_path.as_path()],
            context,
        )?;
        validate_semantic_metadata_artifact(&metadata_owner_path, context, "interface.edit")?;

        let show_validation = !skip_validation_output;
        let validate_args = Map::from_iter([(
            "CIPath".to_string(),
            Value::String(ci_path.display().to_string()),
        )]);
        let mut validation_stdout = None;
        let report = transaction.commit_with_post_validation(|| {
            validate_semantic_metadata_artifact(&metadata_owner_path, context, "interface.edit")?;
            let outcome = validate_interface(&validate_args, context);
            validation_stdout = outcome.stdout.clone();
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
            Err(format!("interface validation failed: {detail}"))
        })?;

        if created_new {
            ci_path = interface_normalize_lexical_path(&ci_path);
        }
        stdout.push_str(&format!("[INFO] Saved: {}\n", ci_path.display()));

        if show_validation {
            stdout.push('\n');
            stdout.push_str("--- Running interface-validate ---\n");
            if let Some(validate_stdout) = validation_stdout {
                stdout.push_str(&validate_stdout);
            }
        }

        stdout.push('\n');
        stdout.push_str("=== interface-edit summary ===\n");
        stdout.push_str(&format!("  Added:    {}\n", counters.added));
        stdout.push_str(&format!("  Removed:  {}\n", counters.removed));
        stdout.push_str(&format!("  Modified: {}\n", counters.modified));
        Ok((stdout, ci_path, report.cleanup_warnings))
    })();

    match edit_result {
        Ok((stdout, ci_path, warnings)) => NativeWriterResult {
            ok: true,
            summary: "unica.interface.edit completed with native command interface editor"
                .to_string(),
            changes: vec![format!("updated {}", ci_path.display())],
            warnings,
            errors: Vec::new(),
            artifacts: vec![ci_path.display().to_string()],
            stdout: Some(stdout),
            stderr: None,
        },
        Err(error) => interface_edit_failure(error),
    }
}

fn interface_edit_failure(error: String) -> NativeWriterResult {
    NativeWriterResult {
        ok: false,
        summary: "unica.interface.edit failed in native command interface editor".to_string(),
        changes: Vec::new(),
        warnings: Vec::new(),
        errors: vec![error.clone()],
        artifacts: Vec::new(),
        stdout: None,
        stderr: Some(format!("{error}\n")),
    }
}

pub(crate) fn interface_edit_operations(
    args: &impl ArgumentAccess,
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
        Ok(interface_edit_operations_from_value(parsed, operation))
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

fn interface_edit_operations_guarded(
    cwd: &Path,
    operation: Option<&str>,
    value: &str,
    definition_file: Option<PathBuf>,
    transaction: &mut CompileTransaction,
) -> Result<Vec<(String, Value)>, String> {
    if let Some(definition_file) = definition_file {
        let definition_file = absolutize(definition_file, cwd);
        let parsed = FileBackedJson::read(&definition_file, |err| {
            format!("failed to parse {}: {err}", definition_file.display())
        })?
        .bind_to(transaction)?;
        Ok(interface_edit_operations_from_value(parsed, operation))
    } else {
        Ok(vec![(
            operation.unwrap_or("").to_string(),
            Value::String(value.to_string()),
        )])
    }
}

fn interface_edit_operations_from_value(
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

fn interface_text_do_semantic_place(
    text: &mut String,
    placement: &unica_format_core::commands::InterfacePlacement,
    counters: &mut InterfaceEditCounters,
    stdout: &mut String,
) -> Result<(), String> {
    let command = normalize_interface_command_name(placement.item().name().as_str(), stdout);
    let group = placement.group().as_str();
    if interface_text_command_bounds_in_section(text, "CommandsPlacement", &command).is_some() {
        interface_text_replace_command_child(
            text,
            "CommandsPlacement",
            &command,
            "CommandGroup",
            group,
        )?;
        counters.modified += 1;
        stdout.push_str(&format!("[INFO] Updated placement: {command} -> {group}\n"));
    } else {
        let fragment = format!(
            "<Command name=\"{}\"><CommandGroup>{}</CommandGroup><Placement>Auto</Placement></Command>",
            escape_xml(&command),
            escape_xml(group)
        );
        interface_text_append_to_section(text, "CommandsPlacement", &fragment)?;
        counters.added += 1;
        stdout.push_str(&format!("[INFO] Placed: {command} -> {group}\n"));
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

fn interface_text_do_semantic_order(
    text: &mut String,
    order: &unica_format_core::commands::InterfaceCommandOrder,
    counters: &mut InterfaceEditCounters,
    stdout: &mut String,
) -> Result<(), String> {
    let group = order.group().as_str();
    let commands = order
        .commands()
        .iter()
        .map(|command| normalize_interface_command_name(command.name().as_str(), stdout))
        .collect::<Vec<_>>();
    if commands.is_empty() {
        return Err("semantic command order requires at least one command".to_string());
    }
    counters.removed += interface_text_count_commands_for_group(text, "CommandsOrder", group);
    counters.added += commands.len();
    let fragments = commands
        .iter()
        .map(|command| {
            format!(
                "<Command name=\"{}\"><CommandGroup>{}</CommandGroup></Command>",
                escape_xml(command),
                escape_xml(group)
            )
        })
        .collect::<Vec<_>>();
    interface_text_replace_section_items(text, "CommandsOrder", &fragments)?;
    stdout.push_str(&format!(
        "[INFO] Set order for {group} : {} commands\n",
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

fn interface_text_do_semantic_subsystem_order(
    text: &mut String,
    subsystems: &[unica_format_core::commands::SubsystemName],
    counters: &mut InterfaceEditCounters,
    stdout: &mut String,
) -> Result<(), String> {
    if subsystems.is_empty() {
        return Err("semantic subsystem order requires at least one subsystem".to_string());
    }
    counters.removed += interface_text_count_direct_items(text, "SubsystemsOrder", "Subsystem");
    counters.added += subsystems.len();
    let fragments = subsystems
        .iter()
        .map(|subsystem| format!("<Subsystem>{}</Subsystem>", escape_xml(subsystem.as_str())))
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

fn interface_text_do_semantic_group_order(
    text: &mut String,
    groups: &[unica_format_core::commands::InterfaceGroupName],
    counters: &mut InterfaceEditCounters,
    stdout: &mut String,
) -> Result<(), String> {
    if groups.is_empty() {
        return Err("semantic group order requires at least one group".to_string());
    }
    counters.removed += interface_text_count_direct_items(text, "GroupsOrder", "Group");
    counters.added += groups.len();
    let fragments = groups
        .iter()
        .map(|group| format!("<Group>{}</Group>", escape_xml(group.as_str())))
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
    let empty = format!("<{section}/>");
    if let Some(start) = text.find(&empty) {
        let line_start = text[..start]
            .rfind(['\r', '\n'])
            .map_or(0, |index| index + 1);
        let parent_indent = text[line_start..start]
            .chars()
            .all(|character| character == '\t' || character == ' ')
            .then(|| text[line_start..start].to_string())
            .unwrap_or_else(|| "\t".to_string());
        let child_indent = format!("{parent_indent}\t");
        text.replace_range(
            start..start + empty.len(),
            &format!("<{section}>\r\n{child_indent}{fragment}\r\n{parent_indent}</{section}>"),
        );
        return Ok(());
    }
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

pub(crate) fn interface_metadata_owner_path(ci_path: &Path) -> Result<PathBuf, String> {
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
    let owner_path = if configuration_owner.is_file() {
        configuration_owner
    } else {
        object_dir.with_extension("xml")
    };
    if !owner_path.is_file() {
        return Err(format!(
            "interface.edit metadata owner is unavailable for {}: expected {}",
            ci_path.display(),
            owner_path.display()
        ));
    }
    Ok(owner_path)
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
    args: &impl ArgumentAccess,
    context: &WorkspaceContext,
) -> NativeWriterResult {
    const NS_CI: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";
    const NS_XR: &str = "http://v8.1c.ru/8.3/xcf/readable";

    let result = (|| -> Result<(bool, String, String, PathBuf), String> {
        let ci_path = resolve_interface_validate_path(args, context)?;
        if !ci_path.exists() {
            let stdout = format!("[ERROR] File not found: {}\n", ci_path.display());
            return Ok((false, stdout.clone(), String::new(), ci_path));
        }

        let context_name = interface_context_name(&ci_path);
        let detailed = bool_arg(args, &["detailed", "Detailed"]);
        let max_errors = int_arg(args, &["maxErrors", "MaxErrors"])
            .unwrap_or(30)
            .max(0) as usize;
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
        let output = finish_interface_validation(report, &context_name);
        Ok((ok, output, String::new(), ci_path))
    })();

    match result {
        Ok((ok, stdout, stderr, artifact)) => NativeWriterResult {
            ok,
            summary: if ok {
                "unica.interface.validate completed with native command interface validator"
                    .to_string()
            } else {
                "unica.interface.validate failed in native command interface validator".to_string()
            },
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: if ok {
                Vec::new()
            } else {
                vec![stdout.trim().to_string()]
            },
            artifacts: vec![artifact.display().to_string()],
            stdout: Some(stdout),
            stderr: Some(stderr),
        },
        Err(error) => NativeWriterResult {
            ok: false,
            summary: "unica.interface.validate failed in native command interface validator"
                .to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some(format!("{error}\n")),
        },
    }
}

/// Resolve the exact CommandInterface.xml file opened by `interface.validate`.
///
/// Missing targets are returned unchanged so the handler retains its
/// established file-not-found diagnostic.
pub(crate) fn resolve_interface_validate_path(
    args: &impl ArgumentAccess,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
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
    Ok(ci_path)
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
    args: &impl ArgumentAccess,
    context: &WorkspaceContext,
) -> Option<Result<NativeWriterResult, String>> {
    match operation {
        "interface-validate" => Some(Ok(validate_interface(args, context))),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn invoke_mutation(
    operation: &str,
    _tool_name: &str,
    args: &impl ArgumentAccess,
    context: &WorkspaceContext,
) -> Option<NativeWriterResult> {
    match operation {
        "interface-edit" => Some(edit_interface(args, context)),
        _ => None,
    }
}

pub(crate) fn edit_interface_typed(
    command: &unica_format_core::commands::InterfaceEdit,
    session: &PlatformWriterSession,
    context: &WorkspaceContext,
) -> NativeWriterResult {
    let interface_path = match session.required_source(
        unica_format_core::commands::WriterSourceRole::Interface,
        "command interface mutation",
    ) {
        Ok(path) => path.to_path_buf(),
        Err(error) => return interface_edit_failure(error),
    };
    edit_interface_input(
        InterfaceEditInput {
            source: InterfaceEditSource::Semantic(command.clone()),
            interface_path,
            create_if_missing: false,
            skip_validation_output: false,
        },
        context,
    )
}

#[cfg(test)]
fn interface_legacy_edit(operation: &str, value: Value) -> Result<InterfaceNativeEdit, String> {
    match operation {
        "hide" => Ok(InterfaceNativeEdit::Hide(interface_value_list(&value)?)),
        "show" => Ok(InterfaceNativeEdit::Show(interface_value_list(&value)?)),
        "place" => Ok(InterfaceNativeEdit::LegacyPlace(value)),
        "order" => Ok(InterfaceNativeEdit::LegacyOrder(value)),
        "subsystem-order" => Ok(InterfaceNativeEdit::LegacyOrderSubsystems(value)),
        "group-order" => Ok(InterfaceNativeEdit::LegacyOrderGroups(value)),
        _ => Err(format!("Unknown operation: {operation}")),
    }
}

fn interface_native_edit(edit: unica_format_core::commands::InterfaceEdit) -> InterfaceNativeEdit {
    use unica_format_core::commands::InterfaceEdit as Edit;

    match edit {
        Edit::Replace(definition) => InterfaceNativeEdit::Replace(definition),
        Edit::Hide(value) => InterfaceNativeEdit::Hide(vec![value.name().as_str().to_string()]),
        Edit::Show(value) => InterfaceNativeEdit::Show(vec![value.name().as_str().to_string()]),
        Edit::Place(value) => InterfaceNativeEdit::Place(value),
        Edit::Order(value) => InterfaceNativeEdit::Order(value),
        Edit::OrderSubsystems(value) => InterfaceNativeEdit::OrderSubsystems(value),
        Edit::OrderGroups(value) => InterfaceNativeEdit::OrderGroups(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::UnicaApplication;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::native_operations::compile_transaction::{
        with_commit_failpoint, CommitFailpoint,
    };
    use crate::infrastructure::native_operations::single_file_publisher::with_before_commit_hook;
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

    fn command_interface_document(namespace: &str, body: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<CommandInterface xmlns=\"{namespace}\" xmlns:xr=\"{INTERFACE_XR_NS}\" version=\"2.20\">\n\
{body}\n\
</CommandInterface>"
        )
    }

    fn hide_args(ci_path: &str, create_if_missing: bool) -> Map<String, Value> {
        Map::from_iter([
            ("CIPath".to_string(), Value::String(ci_path.to_string())),
            ("Operation".to_string(), Value::String("hide".to_string())),
            (
                "Value".to_string(),
                Value::String("Catalog.Products.StandardCommand.OpenList".to_string()),
            ),
            (
                "CreateIfMissing".to_string(),
                Value::Bool(create_if_missing),
            ),
            ("NoValidate".to_string(), Value::Bool(true)),
        ])
    }

    fn write_command_interface(context: &WorkspaceContext, ci_rel: &str, text: &str) -> PathBuf {
        let ci_path = context.cwd.join(ci_rel);
        fs::create_dir_all(ci_path.parent().unwrap()).unwrap();
        fs::write(&ci_path, text.as_bytes()).unwrap();
        ci_path
    }

    fn subsystem_document(name: &str, include_help: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" version=\"2.20\">\n\
\t<Subsystem uuid=\"11111111-2222-4333-8444-555555555555\">\n\
\t\t<Properties>\n\
\t\t\t<Name>{name}</Name>\n\
\t\t\t<Synonym/>\n\
\t\t\t<Comment/>\n\
\t\t\t<IncludeHelpInContents>{include_help}</IncludeHelpInContents>\n\
\t\t\t<IncludeInCommandInterface>true</IncludeInCommandInterface>\n\
\t\t\t<UseOneCommand>false</UseOneCommand>\n\
\t\t\t<Explanation/>\n\
\t\t\t<Picture/>\n\
\t\t\t<Content/>\n\
\t\t</Properties>\n\
\t\t<ChildObjects/>\n\
\t</Subsystem>\n\
</MetaDataObject>"
        )
    }

    fn write_valid_subsystem_owner(context: &WorkspaceContext, ci_rel: &str) -> (PathBuf, Vec<u8>) {
        let ci_path = context.cwd.join(ci_rel);
        let object_dir = ci_path.parent().unwrap().parent().unwrap();
        let owner_path = object_dir.with_extension("xml");
        let name = owner_path.file_stem().unwrap().to_str().unwrap();
        let bytes = subsystem_document(name, "true").into_bytes();
        fs::create_dir_all(owner_path.parent().unwrap()).unwrap();
        fs::write(&owner_path, &bytes).unwrap();
        (owner_path, bytes)
    }

    fn configuration_owner_document(include_help: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\n\
\t<Configuration uuid=\"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\">\n\
\t\t<Properties><Name>Demo</Name><IncludeHelpInContents>{include_help}</IncludeHelpInContents></Properties>\n\
\t\t<ChildObjects><Subsystem>AuditSubsystem</Subsystem></ChildObjects>\n\
\t</Configuration>\n\
</MetaDataObject>"
        )
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
        write_valid_subsystem_owner(&context, ci_rel);
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
            write_valid_subsystem_owner(&context, &ci_rel);
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

    #[test]
    fn interface_edit_rejects_invalid_existing_document_without_mutating_preimage() {
        let invalid_cases = [
            (
                "bogus-direct-child",
                command_interface_document(INTERFACE_CI_NS, "\t<Bogus/>"),
            ),
            (
                "invalid-common",
                command_interface_document(
                    INTERFACE_CI_NS,
                    "\t<CommandsVisibility>\n\
\t\t<Command name=\"Catalog.Existing.StandardCommand.OpenList\"><Visibility><xr:Common>banana</xr:Common></Visibility></Command>\n\
\t</CommandsVisibility>",
                ),
            ),
            (
                "malformed",
                format!(
                    "<CommandInterface xmlns=\"{INTERFACE_CI_NS}\"><CommandsVisibility>"
                ),
            ),
            (
                "wrong-root",
                format!("<WrongRoot xmlns=\"{INTERFACE_CI_NS}\"/>"),
            ),
            (
                "wrong-root-namespace",
                command_interface_document("urn:not-command-interface", ""),
            ),
            (
                "wrong-section-namespace",
                command_interface_document(
                    INTERFACE_CI_NS,
                    "\t<bad:CommandsVisibility xmlns:bad=\"urn:not-command-interface\"/>",
                ),
            ),
            (
                "wrong-section-order",
                command_interface_document(
                    INTERFACE_CI_NS,
                    "\t<CommandsPlacement/>\n\t<CommandsVisibility/>",
                ),
            ),
            (
                "duplicate-section",
                command_interface_document(
                    INTERFACE_CI_NS,
                    "\t<CommandsVisibility/>\n\t<CommandsVisibility/>",
                ),
            ),
        ];

        for (case, source) in invalid_cases {
            let context = temp_context(case);
            let ci_rel = "src/Subsystems/Existing/Ext/CommandInterface.xml";
            write_valid_subsystem_owner(&context, ci_rel);
            let ci_path = write_command_interface(&context, ci_rel, &source);
            let before = fs::read(&ci_path).unwrap();

            let validation = validate_interface(
                &Map::from_iter([("CIPath".to_string(), Value::String(ci_rel.to_string()))]),
                &context,
            );
            assert!(!validation.ok, "{case}: {validation:?}");
            assert!(!validation.errors.is_empty(), "{case}: {validation:?}");

            let outcome = edit_interface(&hide_args(ci_rel, false), &context);

            assert!(!outcome.ok, "{case}: {outcome:?}");
            assert!(!outcome.errors.is_empty(), "{case}: {outcome:?}");
            assert_eq!(fs::read(&ci_path).unwrap(), before, "{case}");
            assert!(outcome.changes.is_empty(), "{case}: {outcome:?}");
            assert!(outcome.artifacts.is_empty(), "{case}: {outcome:?}");
            let _ = fs::remove_dir_all(&context.workspace_root);
        }
    }

    #[test]
    fn public_interface_edit_rejects_invalid_subsystem_owner_without_mutating_any_file() {
        let context = temp_context("public-invalid-subsystem-owner");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::create_dir_all(context.cwd.join("src/Subsystems")).unwrap();
        let configuration_path = context.cwd.join("src/Configuration.xml");
        let configuration = configuration_owner_document("false").into_bytes();
        fs::write(&configuration_path, &configuration).unwrap();
        let owner_path = context.cwd.join("src/Subsystems/AuditSubsystem.xml");
        let invalid_owner = subsystem_document("AuditSubsystem", "banana").into_bytes();
        fs::write(&owner_path, &invalid_owner).unwrap();
        let ci_rel = "src/Subsystems/AuditSubsystem/Ext/CommandInterface.xml";
        let ci_path = write_command_interface(
            &context,
            ci_rel,
            &command_interface_document(INTERFACE_CI_NS, ""),
        );
        let ci_before = fs::read(&ci_path).unwrap();
        let args = Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(context.cwd.display().to_string()),
            ),
            ("CIPath".to_string(), Value::String(ci_rel.to_string())),
            ("Operation".to_string(), Value::String("hide".to_string())),
            (
                "Value".to_string(),
                Value::String("Catalog.Products.StandardCommand.OpenList".to_string()),
            ),
            ("dryRun".to_string(), Value::Bool(false)),
            ("confirm".to_string(), Value::Bool(true)),
        ]);

        let outcome = UnicaApplication::new()
            .call_tool("unica.interface.edit", &args)
            .unwrap();

        assert!(!outcome.ok, "{outcome:?}");
        let errors = outcome.errors.join("\n");
        assert!(errors.contains("IncludeHelpInContents"), "{outcome:?}");
        assert!(errors.contains("banana"), "{outcome:?}");
        assert_eq!(fs::read(&configuration_path).unwrap(), configuration);
        assert_eq!(fs::read(&owner_path).unwrap(), invalid_owner);
        assert_eq!(fs::read(&ci_path).unwrap(), ci_before);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn public_interface_edit_rejects_invalid_configuration_owner_without_mutating_any_file() {
        let context = temp_context("public-invalid-configuration-owner");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::create_dir_all(context.cwd.join("src")).unwrap();
        let configuration_path = context.cwd.join("src/Configuration.xml");
        let invalid_configuration = configuration_owner_document("banana").into_bytes();
        fs::write(&configuration_path, &invalid_configuration).unwrap();
        let ci_rel = "src/Ext/CommandInterface.xml";
        let ci_path = write_command_interface(
            &context,
            ci_rel,
            &command_interface_document(INTERFACE_CI_NS, ""),
        );
        let ci_before = fs::read(&ci_path).unwrap();
        let args = Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(context.cwd.display().to_string()),
            ),
            ("CIPath".to_string(), Value::String(ci_rel.to_string())),
            ("Operation".to_string(), Value::String("hide".to_string())),
            (
                "Value".to_string(),
                Value::String("Catalog.Products.StandardCommand.OpenList".to_string()),
            ),
            ("dryRun".to_string(), Value::Bool(false)),
            ("confirm".to_string(), Value::Bool(true)),
        ]);

        let outcome = UnicaApplication::new()
            .call_tool("unica.interface.edit", &args)
            .unwrap();

        assert!(!outcome.ok, "{outcome:?}");
        let errors = outcome.errors.join("\n");
        assert!(errors.contains("IncludeHelpInContents"), "{outcome:?}");
        assert!(errors.contains("banana"), "{outcome:?}");
        assert_eq!(
            fs::read(&configuration_path).unwrap(),
            invalid_configuration
        );
        assert_eq!(fs::read(&ci_path).unwrap(), ci_before);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn public_interface_edit_prioritizes_newer_metadata_owner_over_older_command_interface() {
        let context = temp_context("public-mixed-owner-versions");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::create_dir_all(context.cwd.join("src/Subsystems")).unwrap();
        let configuration_path = context.cwd.join("src/Configuration.xml");
        let configuration = configuration_owner_document("false").into_bytes();
        fs::write(&configuration_path, &configuration).unwrap();

        let owner_path = context.cwd.join("src/Subsystems/AuditSubsystem.xml");
        let newer_owner = subsystem_document("AuditSubsystem", "true")
            .replacen(r#"version="2.20""#, r#"version="2.21""#, 1)
            .into_bytes();
        fs::write(&owner_path, &newer_owner).unwrap();

        let ci_rel = "src/Subsystems/AuditSubsystem/Ext/CommandInterface.xml";
        let older_ci = command_interface_document(INTERFACE_CI_NS, "").replacen(
            r#"version="2.20""#,
            r#"version="2.19""#,
            1,
        );
        let ci_path = write_command_interface(&context, ci_rel, &older_ci);
        let ci_before = fs::read(&ci_path).unwrap();
        let args = Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(context.cwd.display().to_string()),
            ),
            ("CIPath".to_string(), Value::String(ci_rel.to_string())),
            ("Operation".to_string(), Value::String("hide".to_string())),
            (
                "Value".to_string(),
                Value::String("Catalog.Products.StandardCommand.OpenList".to_string()),
            ),
            ("dryRun".to_string(), Value::Bool(false)),
            ("confirm".to_string(), Value::Bool(true)),
        ]);

        let outcome = UnicaApplication::new()
            .call_tool("unica.interface.edit", &args)
            .unwrap();

        assert!(!outcome.ok, "{outcome:?}");
        let diagnostic = &outcome.diagnostics.as_ref().unwrap()["formatCompatibility"];
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        let warning = outcome.warnings.join("\n");
        assert!(warning.contains("1С 8.5"), "{warning}");
        assert!(!warning.contains("миграц"), "{warning}");
        assert!(!warning.contains("повторно выгруз"), "{warning}");
        assert!(!warning.contains("re-export"), "{warning}");
        assert_eq!(fs::read(&configuration_path).unwrap(), configuration);
        assert_eq!(fs::read(&owner_path).unwrap(), newer_owner);
        assert_eq!(fs::read(&ci_path).unwrap(), ci_before);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn interface_edit_post_write_failure_restores_existing_preimage() {
        let context = temp_context("post-write-existing");
        let ci_rel = "src/Subsystems/Existing/Ext/CommandInterface.xml";
        write_valid_subsystem_owner(&context, ci_rel);
        let source = command_interface_document(INTERFACE_CI_NS, "");
        let ci_path = write_command_interface(&context, ci_rel, &source);
        let before = fs::read(&ci_path).unwrap();

        let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            edit_interface(&hide_args(ci_rel, false), &context)
        });

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("post-write validation"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&ci_path).unwrap(), before);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn interface_edit_post_write_failure_removes_new_file_and_directories() {
        let context = temp_context("post-write-create");
        let ci_rel = "src/Subsystems/New/Ext/CommandInterface.xml";
        let ci_path = context.cwd.join(ci_rel);
        let (owner_path, owner_bytes) = write_valid_subsystem_owner(&context, ci_rel);

        let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            edit_interface(&hide_args(ci_rel, true), &context)
        });

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("post-write validation"),
            "{outcome:?}"
        );
        assert!(!ci_path.exists(), "{outcome:?}");
        assert!(
            !ci_path.parent().unwrap().parent().unwrap().exists(),
            "{outcome:?}"
        );
        assert_eq!(fs::read(owner_path).unwrap(), owner_bytes);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn interface_edit_rejects_stale_preimage_without_overwriting_concurrent_change() {
        let context = temp_context("stale-preimage");
        let ci_rel = "src/Subsystems/Existing/Ext/CommandInterface.xml";
        write_valid_subsystem_owner(&context, ci_rel);
        let source = command_interface_document(INTERFACE_CI_NS, "");
        let ci_path = write_command_interface(&context, ci_rel, &source);
        let concurrent = command_interface_document(
            INTERFACE_CI_NS,
            "\t<GroupsOrder><Group>Concurrent</Group></GroupsOrder>",
        )
        .into_bytes();
        let hook_path = ci_path.clone();
        let hook_bytes = concurrent.clone();

        let outcome = with_before_commit_hook(
            move |path| {
                assert_eq!(path, hook_path);
                fs::write(path, &hook_bytes).unwrap();
            },
            || edit_interface(&hide_args(ci_rel, false), &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        let errors = outcome.errors.join("\n");
        assert!(
            errors.contains("changed") || errors.contains("preimage"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&ci_path).unwrap(), concurrent);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn interface_edit_rolls_back_if_unchanged_metadata_owner_changes_during_publication() {
        let context = temp_context("metadata-owner-race");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::create_dir_all(context.cwd.join("src")).unwrap();
        fs::write(
            context.cwd.join("src/Configuration.xml"),
            configuration_owner_document("true"),
        )
        .unwrap();
        let ci_rel = "src/Subsystems/AuditSubsystem/Ext/CommandInterface.xml";
        let (owner_path, _) = write_valid_subsystem_owner(&context, ci_rel);
        let ci_path = write_command_interface(
            &context,
            ci_rel,
            &command_interface_document(INTERFACE_CI_NS, ""),
        );
        let ci_before = fs::read(&ci_path).unwrap();
        let concurrent_owner = subsystem_document("AuditSubsystem", "true")
            .replace("<Explanation/>", "<Explanation>Concurrent</Explanation>");
        let owner_for_hook = owner_path.clone();
        let owner_bytes_for_hook = concurrent_owner.as_bytes().to_vec();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&owner_for_hook, &owner_bytes_for_hook).unwrap(),
            || edit_interface(&hide_args(ci_rel, false), &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&ci_path).unwrap(), ci_before);
        assert_eq!(fs::read_to_string(&owner_path).unwrap(), concurrent_owner);
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn interface_edit_classifies_ci_and_metadata_owner_as_one_dependency_set() {
        let context = temp_context("mixed-dependency-versions");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::create_dir_all(context.cwd.join("src")).unwrap();
        fs::write(
            context.cwd.join("src/Configuration.xml"),
            configuration_owner_document("true"),
        )
        .unwrap();
        let ci_rel = "src/Subsystems/AuditSubsystem/Ext/CommandInterface.xml";
        let (owner_path, _) = write_valid_subsystem_owner(&context, ci_rel);
        let newer_owner = subsystem_document("AuditSubsystem", "true").replacen(
            r#"version="2.20""#,
            r#"version="2.21""#,
            1,
        );
        fs::write(&owner_path, &newer_owner).unwrap();
        let older_ci = command_interface_document(INTERFACE_CI_NS, "").replacen(
            r#"version="2.20""#,
            r#"version="2.19""#,
            1,
        );
        let ci_path = write_command_interface(&context, ci_rel, &older_ci);
        let ci_before = fs::read(&ci_path).unwrap();

        let outcome = edit_interface(&hide_args(ci_rel, false), &context);

        assert!(!outcome.ok, "{outcome:?}");
        let errors = outcome.errors.join("\n");
        assert!(errors.contains("newer than supported 2.20"), "{outcome:?}");
        assert!(errors.contains("1C 8.5 support is planned"), "{outcome:?}");
        assert!(!errors.contains("re-export the source"), "{outcome:?}");
        assert_eq!(fs::read(&ci_path).unwrap(), ci_before);
        assert_eq!(fs::read_to_string(&owner_path).unwrap(), newer_owner);
        let _ = fs::remove_dir_all(&context.workspace_root);
    }
}
