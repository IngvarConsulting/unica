//! Actual source binding, exact preimage guards, publication and terminal projection.
use crate::infrastructure::native_operations::common::*;

use crate::infrastructure::native_operations::meta::validate_metadata_owner_shape_8_3_27;
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_metadata::interface::*;
use unica_metadata_designer::interface::*;
use unica_source_contract::workspace::WorkspaceContext;
use unica_workspace::compile_transaction::*;
use unica_workspace::discovery::ProjectDiscoveryReader;
pub(crate) fn edit_interface(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    edit_interface_with_data(args, context).outcome
}

pub(crate) fn edit_interface_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> InterfaceEditExecution {
    let edit_result = (|| -> Result<(InterfaceEditData, PathBuf, Vec<String>), String> {
        let definition_file =
            unica_operation_contract::input::path_arg(args, &["definitionFile", "DefinitionFile"]);
        let operation =
            unica_operation_contract::input::string_arg(args, &["operation", "Operation"]);
        if definition_file.is_some() && operation.is_some() {
            return Err("Cannot use both -DefinitionFile and -Operation".to_string());
        }
        if definition_file.is_none() && operation.is_none() {
            return Err("Either -DefinitionFile or -Operation is required".to_string());
        }

        let mut ci_path = unica_operation_contract::input::required_path(
            args,
            &["ciPath", "CIPath", "path", "Path"],
            "CIPath",
        )
        .map(|path| unica_operation_contract::input::absolutize(path, &context.cwd))?;
        let format_version = unica_source_designer::format_profile::ACTIVE_FORMAT_PROFILE
            .export_format
            .to_string();

        let mut stdout = String::new();
        let source_exists = ci_path.is_file();
        let created_new = if !source_exists {
            if unica_operation_contract::input::bool_arg(
                args,
                &["createIfMissing", "CreateIfMissing"],
            ) {
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
        let metadata_owner_path =
            interface_metadata_owner_path(&ProjectDiscoveryReader::new(&context.cwd), &ci_path)?;
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
        text = unica_xml_format::text::lxml_parser_normalized_text(&text);
        if text.is_empty() {
            text = emit_empty_command_interface_document(&format_version);
        }
        let mut transaction = CompileTransaction::new();
        let operations = interface_edit_operations_guarded(
            args,
            &context.cwd,
            operation,
            definition_file,
            &mut transaction,
        )?;
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

        let serialized =
            unica_xml_format::text::lxml_tree_serialized_text_like_source(&text, &source_text);
        let replacement = unica_xml_format::text::utf8_bom_bytes(&serialized);
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
        validate_metadata_owner_shape_8_3_27(&metadata_owner_path, context, "interface.edit")?;

        let show_validation =
            !unica_operation_contract::input::bool_arg(args, &["noValidate", "NoValidate"]);
        let validate_args = Map::from_iter([(
            "CIPath".to_string(),
            Value::String(ci_path.display().to_string()),
        )]);
        let mut validation_stdout = None;
        let mut validated = false;
        let report = transaction.commit_with_post_validation(|| {
            validate_metadata_owner_shape_8_3_27(&metadata_owner_path, context, "interface.edit")?;
            let outcome = validate_interface(&validate_args, context);
            validated = outcome.ok;
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

        let _ = (stdout, show_validation, validation_stdout);
        let data = InterfaceEditData {
            added: counters.added,
            removed: counters.removed,
            modified: counters.modified,
            validated,
            mutation: if created_new {
                unica_execution_contract::mutation::MutationData::new(true).created(&ci_path)
            } else {
                unica_execution_contract::mutation::MutationData::new(true).updated(&ci_path)
            },
        };
        Ok((data, ci_path, report.cleanup_warnings))
    })();

    match edit_result {
        Ok((data, ci_path, warnings)) => InterfaceEditExecution {
            outcome: AdapterOutcome {
                ok: true,
                summary: format!(
                    "unica.interface.edit added {}, removed {}, modified {}",
                    data.added, data.removed, data.modified
                ),
                changes: vec![format!("updated {}", ci_path.display())],
                warnings,
                errors: Vec::new(),
                artifacts: vec![ci_path.display().to_string()],
                stdout: None,
                stderr: None,
                command: None,
            },
            data: Some(data),
        },
        Err(error) => InterfaceEditExecution {
            outcome: AdapterOutcome {
                ok: false,
                summary: "unica.interface.edit failed in native command interface editor"
                    .to_string(),
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

fn interface_edit_operations_guarded(
    args: &Map<String, Value>,
    cwd: &Path,
    operation: Option<&str>,
    definition_file: Option<PathBuf>,
    transaction: &mut CompileTransaction,
) -> Result<Vec<(String, Value)>, String> {
    if let Some(definition_file) = definition_file {
        let definition_file = unica_operation_contract::input::absolutize(definition_file, cwd);
        let parsed = FileBackedJson::read(&definition_file, |err| {
            format!("failed to parse {}: {err}", definition_file.display())
        })?
        .bind_to(transaction)?;
        Ok(interface_edit_operations_from_value(parsed, operation))
    } else {
        interface_edit_operations(
            &ProjectDiscoveryReader::new(cwd),
            args,
            cwd,
            operation,
            None,
        )
    }
}

pub(crate) fn validate_interface(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let result = (|| {
        let ci_path = resolve_interface_validate_path(args, context)?;
        if !ci_path.exists() {
            let stdout = format!("[ERROR] File not found: {}\n", ci_path.display());
            return Ok((false, stdout.clone(), String::new(), ci_path));
        }
        let text = fs::read_to_string(&ci_path)
            .map_err(|err| format!("failed to read {}: {err}", ci_path.display()))?;
        validate_interface_image(args, &ci_path, &text)
    })();
    match result {
        Ok((ok, stdout, stderr, artifact)) => AdapterOutcome {
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
            command: None,
        },
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

pub(crate) fn resolve_interface_validate_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    resolve_interface_path(
        args,
        &ProjectDiscoveryReader::new(&context.cwd),
        &context.cwd,
    )
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

pub(crate) struct InterfaceEditExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<InterfaceEditData>,
}

#[cfg(test)]
#[path = "../../tests/platform/interface_tests.rs"]
pub(crate) mod tests;
