//! Actual source binding, exact preimage guards, publication and terminal projection.
use crate::application::operation_descriptors::*;
use crate::infrastructure::native_operations::common::*;
use crate::infrastructure::native_operations::logical_selector::{
    logical_selection, physical_selection, AttachedResource,
};

use crate::infrastructure::project_sources::discover_project_source_map;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_metadata::subsystem::*;
use unica_metadata::support_state::{
    ObjectSupportData as DomainObjectSupportData, ObjectSupportState, ResolvedSubsystemTarget,
    SupportStateReader,
};
use unica_metadata_designer::subsystem::*;
use unica_source_contract::read::SecureTreeCaptureLimits;
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_contract::SourceSetKind;
use unica_work_control::cancellation::{cancelled_error, CancellationToken};
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::compile_transaction::*;
use unica_workspace::discovery::ProjectDiscoveryReader;
use unica_workspace::paths::normalize_path_identity;
use unica_workspace::read::RetainedRootSecureRead;

fn subsystem_validation_args(path: &Path) -> Map<String, Value> {
    let mut args = Map::new();
    args.insert(
        "SubsystemPath".to_string(),
        Value::String(path.display().to_string()),
    );
    args
}

/// The single args→path entry point of `subsystem.info` and
/// `subsystem.validate`, and the one the format guard calls.
///
/// `sourceSet` with no address means the source root, which for this reader is
/// the registered tree of the whole set — the same answer the `Subsystems`
/// directory path gives today. `subsystem.validate` publishes `metadataPath` as
/// required, so the schema refuses that call before it arrives here; the branch
/// stays because `resolve_subsystem_validate_xml` rejects a directory anyway,
/// and a future schema change must not be able to produce a wrong answer.
pub(crate) fn resolve_subsystem_read_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    // Both selectors are read with `string_arg` everywhere else, so an empty,
    // null or non-string value is absent. Branching on `contains_key` instead
    // would send such a call down the wrong path — and, for `sourceSet`, into
    // an `expect` on a value the shared seam had already rejected.
    if unica_operation_contract::input::string_arg(args, &["sourceSet"]).is_some()
        && unica_operation_contract::input::string_arg(args, &["metadataPath"]).is_none()
    {
        let Some(selection) =
            logical_selection(args, context, AttachedResource::ConfigurationRoot, &[])
        else {
            return Err(
                "source_set_unknown: sourceSet must name an exact project source set".to_string(),
            );
        };
        let selection = selection.map_err(|failure| failure.to_string())?;
        let source_root = selection
            .resource_path
            .parent()
            .ok_or_else(|| "the resolved source root has no directory".to_string())?;
        return Ok(source_root.join("Subsystems"));
    }
    if let Some(selection) =
        logical_selection(args, context, AttachedResource::Descriptor, SUBSYSTEM_KINDS)
    {
        return selection
            .map(|selection| selection.resource_path)
            .map_err(|failure| failure.to_string());
    }
    let raw_path =
        unica_operation_contract::input::required_path(args, SUBSYSTEM_PATH, "SubsystemPath")?;
    reject_subsystem_info_parent_components(&raw_path)?;
    Ok(unica_operation_contract::input::absolutize(
        raw_path,
        &context.cwd,
    ))
}

pub(crate) fn subsystem_read_format_dependency_paths(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    operation: &str,
) -> Result<Vec<PathBuf>, String> {
    let path = resolve_subsystem_read_path(args, context)?;

    if operation == "subsystem-validate" {
        let descriptor =
            resolve_subsystem_validate_xml(&ProjectDiscoveryReader::new(&context.cwd), path)?;
        return Ok(subsystem_validation_format_dependency_paths(&[
            descriptor.as_path()
        ]));
    }

    if operation != "subsystem-info" {
        return Ok(Vec::new());
    }
    Err("subsystem-info format dependencies require prepared registered evidence".to_string())
}

fn require_subsystem_validation(outcome: &AdapterOutcome) -> Result<(), String> {
    if outcome.ok {
        return Ok(());
    }
    let errors = outcome
        .errors
        .iter()
        .map(|error| error.trim())
        .filter(|error| !error.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    let stderr = outcome.stderr.as_deref().unwrap_or("").trim();
    let stdout = outcome.stdout.as_deref().unwrap_or("").trim();
    let details = if !errors.is_empty() {
        errors
    } else if !stderr.is_empty() {
        stderr.to_string()
    } else if !stdout.is_empty() {
        stdout.to_string()
    } else {
        outcome.summary.clone()
    };
    Err(format!("subsystem semantic validation failed: {details}"))
}

pub(crate) fn edit_subsystem(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    edit_subsystem_with_data(args, context).outcome
}

pub(crate) fn edit_subsystem_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> SubsystemEditExecution {
    let edit_result = (|| -> Result<SubsystemEditResult, String> {
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

        let raw_path =
            unica_operation_contract::input::required_path(args, SUBSYSTEM_PATH, "SubsystemPath")?;
        let resolved_path = resolve_subsystem_edit_xml(
            &ProjectDiscoveryReader::new(&context.cwd),
            unica_operation_contract::input::absolutize(raw_path, &context.cwd),
        )?;
        let original = fs::read(&resolved_path)
            .map_err(|err| format!("failed to read {}: {err}", resolved_path.display()))?;
        let mut model =
            load_subsystem_edit_model(&ProjectDiscoveryReader::new(&context.cwd), &resolved_path)?;
        let obj_name = model.name.clone();
        let mut transaction = CompileTransaction::new();
        let operations = subsystem_edit_operations_guarded(
            args,
            &context.cwd,
            operation,
            definition_file,
            &mut transaction,
        )?;
        let lists_only = operations.iter().all(|(op, _)| op != "set-property");
        let mut counters = SubsystemEditCounters::default();
        let mut log = SubsystemEditLog::new();
        let mut child_stubs = BTreeMap::<PathBuf, String>::new();
        let mut reused_child_subsystems = BTreeMap::<PathBuf, Vec<u8>>::new();

        for (op_name, value) in operations {
            match op_name.as_str() {
                "add-content" => {
                    subsystem_edit_add_content(&mut model, &value, &mut counters, &mut log)?;
                }
                "remove-content" => {
                    subsystem_edit_remove_content(&mut model, &value, &mut counters, &mut log)?;
                }
                "add-child" => subsystem_edit_add_child(
                    &ProjectDiscoveryReader::new(&context.cwd),
                    &mut model,
                    &resolved_path,
                    &value,
                    unica_metadata_designer::subsystem::SubsystemChildEffects {
                        counters: &mut counters,
                        log: &mut log,
                        child_stubs: &mut child_stubs,
                        reused_child_subsystems: &mut reused_child_subsystems,
                    },
                    crate::infrastructure::source_ids::fresh_uuid,
                )?,
                "remove-child" => {
                    subsystem_edit_remove_child(&mut model, &value, &mut counters, &mut log)?
                }
                "set-property" => {
                    subsystem_edit_set_property(&mut model, &value, &mut counters, &mut log)?
                }
                _ => return Err(format!("Unknown operation: {op_name}")),
            }
        }

        validate_subsystem_metadata_name("Name", &model.name)?;
        if !unica_xml_format::names::is_valid_uuid(&model.uuid) {
            return Err(format!(
                "Subsystem UUID must be a valid UUID: {:?}",
                model.uuid
            ));
        }
        for (property, value) in [
            ("IncludeHelpInContents", &model.include_help),
            ("IncludeInCommandInterface", &model.include_ci),
            ("UseOneCommand", &model.use_one_command),
        ] {
            canonical_subsystem_boolean(&Value::String(value.clone()), property)?;
        }
        for child in &model.children {
            validate_subsystem_metadata_name("Child subsystem name", child)?;
        }
        {
            let final_child_names = model
                .children
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();
            let remains_registered = |path: &Path| {
                path.file_stem()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| final_child_names.contains(name))
            };
            child_stubs.retain(|path, _| remains_registered(path));
            reused_child_subsystems.retain(|path, _| remains_registered(path));
        }

        transaction.replace_bytes(
            &resolved_path,
            &original,
            if lists_only {
                let text = std::str::from_utf8(&original)
                    .map_err(|_| "subsystem descriptor is not UTF-8")?;
                patch_subsystem_lists(text, &model, Some(text))?.into_bytes()
            } else {
                unica_xml_format::text::utf8_bom_bytes(&emit_subsystem_edit_model(&model))
            },
        )?;
        for (path, xml) in &child_stubs {
            transaction.create_utf8_bom_text(path, xml)?;
        }
        for child_path in reused_child_subsystems.keys() {
            let outcome = validate_subsystem(&subsystem_validation_args(child_path), context);
            require_subsystem_validation(&outcome)?;
        }
        for (path, preimage) in &reused_child_subsystems {
            guard_exact_preimage_if_unprotected(&mut transaction, path, preimage)?;
        }
        let mut validation_targets = vec![resolved_path.as_path()];
        validation_targets.extend(reused_child_subsystems.keys().map(PathBuf::as_path));
        let format_dependencies = subsystem_validation_format_dependency_paths(&validation_targets);
        let format_dependency_refs = format_dependencies
            .iter()
            .map(PathBuf::as_path)
            .collect::<Vec<_>>();
        guard_active_format_dependencies(&mut transaction, &format_dependency_refs, context)?;

        let validation_args = subsystem_validation_args(&resolved_path);
        let mut validation_stdout = None;
        let mut validated = false;
        let report = transaction.commit_with_post_validation(|| {
            for child_path in reused_child_subsystems.keys() {
                let outcome = validate_subsystem(&subsystem_validation_args(child_path), context);
                require_subsystem_validation(&outcome)?;
            }
            let outcome = validate_subsystem(&validation_args, context);
            validated = outcome.ok;
            validation_stdout = outcome.stdout.clone();
            require_subsystem_validation(&outcome)
        })?;

        let _ = validation_stdout;
        let mut mutation = unica_execution_contract::mutation::MutationData::new(true);
        for path in &report.created {
            mutation = mutation.created(path);
        }
        for path in &report.updated {
            mutation = mutation.updated(path);
        }
        let data = SubsystemEditData {
            subsystem: obj_name.to_string(),
            operations: log,
            added: counters.added,
            removed: counters.removed,
            modified: counters.modified,
            created_stubs: child_stubs
                .keys()
                .map(|path| path.display().to_string())
                .collect(),
            validated,
            mutation,
        };

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
        let mut artifacts = report.created;
        artifacts.extend(report.updated);
        Ok(SubsystemEditResult {
            data,
            artifacts,
            changes,
            warnings: report.cleanup_warnings,
        })
    })();

    match edit_result {
        Ok(result) => SubsystemEditExecution {
            outcome: AdapterOutcome {
                ok: true,
                summary: format!(
                    "unica.subsystem.edit applied {} of {} operation(s) to {}",
                    result
                        .data
                        .operations
                        .iter()
                        .filter(|op| op.applied)
                        .count(),
                    result.data.operations.len(),
                    result.data.subsystem
                ),
                changes: result.changes,
                warnings: result.warnings,
                errors: Vec::new(),
                artifacts: result
                    .artifacts
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
                stdout: None,
                stderr: None,
                command: None,
            },
            data: Some(result.data),
        },
        Err(error) => SubsystemEditExecution {
            outcome: AdapterOutcome {
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
            data: None,
        },
    }
}

fn subsystem_edit_operations_guarded(
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
        Ok(subsystem_edit_operations_from_value(parsed, operation))
    } else {
        subsystem_edit_operations(
            &ProjectDiscoveryReader::new(cwd),
            args,
            cwd,
            operation,
            None,
        )
    }
}

pub(crate) fn validate_subsystem(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let result = (|| -> Result<(bool, String, PathBuf), String> {
        let path = resolve_subsystem_read_path(args, context)?;
        let detailed = unica_operation_contract::input::bool_arg(args, &["detailed", "Detailed"]);
        let xml_path = match resolve_subsystem_validate_xml(
            &ProjectDiscoveryReader::new(&context.cwd),
            path,
        ) {
            Ok(path) => path,
            Err(stdout) => {
                return Ok((false, format!("{stdout}\n"), PathBuf::new()));
            }
        };

        let text = fs::read_to_string(&xml_path)
            .map_err(|err| format!("failed to read {}: {err}", xml_path.display()))?;
        validate_subsystem_content(
            &text,
            xml_path,
            detailed,
            |path| Ok(path.exists()),
            |path| match fs::read_to_string(path) {
                Ok(text) => Ok(Some(text)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(format!("failed to read {}: {error}", path.display())),
            },
        )
    })();
    subsystem_validation_outcome(result)
}

pub(crate) fn subsystem_validation_outcome(
    result: Result<(bool, String, PathBuf), String>,
) -> AdapterOutcome {
    match result {
        Ok((ok, text, artifact)) => {
            let artifacts = if artifact.as_os_str().is_empty() {
                Vec::new()
            } else {
                vec![artifact.display().to_string()]
            };
            AdapterOutcome {
                ok,
                summary: if ok {
                    "unica.subsystem.validate completed with native subsystem validator".to_string()
                } else {
                    "unica.subsystem.validate failed in native subsystem validator".to_string()
                },
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: if ok {
                    Vec::new()
                } else {
                    vec![text.trim().to_string()]
                },
                artifacts,
                stdout: Some(text),
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

pub(crate) fn validate_subsystem_owner_path(
    path: &Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    let outcome = validate_subsystem(&subsystem_validation_args(path), context);
    require_subsystem_validation(&outcome).map_err(|error| {
        format!(
            "subsystem owner validation failed for {}: {error}",
            path.display()
        )
    })
}

#[cfg(test)]
pub(crate) fn analyze_subsystem_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> SubsystemInfoExecution {
    let support_reader =
        crate::infrastructure::support_state::WorkspaceSupportStateReader::new(context);
    prepare_subsystem_info_with_checkpoint(args, context, &support_reader, || Ok(()))
        .map(|prepared| prepared.execution)
        .unwrap_or_else(subsystem_info_failure)
}

pub(crate) fn analyze_subsystem_info_cancellable(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
    support_reader: &dyn SupportStateReader,
) -> SubsystemInfoExecution {
    prepare_subsystem_info(args, context, cancellation, deadline, support_reader)
        .map(|prepared| prepared.execution)
        .unwrap_or_else(|error| subsystem_info_failure(error.to_string()))
}

pub(crate) fn prepare_subsystem_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
    support_reader: &dyn SupportStateReader,
) -> Result<PreparedSubsystemInfo, SubsystemInfoPreparationError> {
    let control_error = std::cell::RefCell::new(None);
    let prepared = prepare_subsystem_info_with_checkpoint(args, context, support_reader, || {
        let checkpoint = subsystem_info_checkpoint(cancellation, deadline);
        if let Err(error) = &checkpoint {
            if let Some(error) = subsystem_info_control_error(error) {
                control_error.replace(Some(error));
            }
        }
        checkpoint
    });
    prepared.map_err(|error| {
        control_error
            .into_inner()
            .unwrap_or(SubsystemInfoPreparationError::Provider(error))
    })
}

fn subsystem_info_checkpoint(
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> io::Result<()> {
    if cancellation.is_cancelled() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            cancelled_error("unica.subsystem.info stopped during registered topology read"),
        ));
    }
    if deadline.is_elapsed() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "unica.subsystem.info provider deadline exceeded",
        ));
    }
    Ok(())
}

pub(crate) fn ensure_subsystem_info_control(
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<(), SubsystemInfoPreparationError> {
    subsystem_info_checkpoint(cancellation, deadline).map_err(|error| {
        subsystem_info_control_error(&error).unwrap_or_else(|| {
            SubsystemInfoPreparationError::Provider(subsystem_info_checkpoint_message(error))
        })
    })
}

fn subsystem_info_control_error(error: &io::Error) -> Option<SubsystemInfoPreparationError> {
    let message = error.to_string();
    if error.kind() == io::ErrorKind::Interrupted
        && message.starts_with(unica_work_control::cancellation::CANCELLED_PREFIX)
    {
        return Some(SubsystemInfoPreparationError::Control(message));
    }
    if error.kind() == io::ErrorKind::TimedOut {
        return Some(SubsystemInfoPreparationError::Control(format!(
            "subsystem info snapshot failed: {message}"
        )));
    }
    None
}

fn prepare_subsystem_info_with_checkpoint(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    support_reader: &dyn SupportStateReader,
    mut checkpoint: impl FnMut() -> io::Result<()>,
) -> Result<PreparedSubsystemInfo, String> {
    checkpoint().map_err(subsystem_info_checkpoint_message)?;
    let path = resolve_subsystem_read_path(args, context)?;

    // Only the source root's `Subsystems/` folder asks the hierarchy question.
    // A nested folder would omit the required ancestor chain, while a
    // single-subsystem directory is not the public XML address at all.
    if path.is_dir() {
        if path.file_name().and_then(|name| name.to_str()) != Some("Subsystems") {
            return Err(
                "SubsystemPath must name an XML file or the root Subsystems directory".to_string(),
            );
        }
        let (source_root, parent_address) =
            subsystem_tree_scope(&path, context)?.ok_or_else(|| {
                "the requested Subsystems directory is outside every configured source root"
                    .to_string()
            })?;
        if !parent_address.is_empty() {
            return Err(
                "SubsystemPath must name the root Subsystems directory, not a nested Subsystems directory"
                    .to_string(),
            );
        }
        let topology = capture_registered_subsystem_topology(&source_root, &mut checkpoint)
            .map_err(|error| error.to_string())?;
        let tree = topology
            .roots
            .iter()
            .map(subsystem_tree_node)
            .collect::<Vec<_>>();
        let summary = format!(
            "unica.subsystem.info described {} subsystem(s) in the requested scope",
            tree.len()
        );
        let format_documents = topology_format_documents(&source_root, &topology);
        checkpoint().map_err(subsystem_info_checkpoint_message)?;
        return Ok(PreparedSubsystemInfo {
            execution: subsystem_info_success(SubsystemInfoAnswer::Tree { tree }, path, summary),
            format_documents,
        });
    }

    if path.extension().and_then(|extension| extension.to_str()) != Some("xml") {
        return Err(
            "SubsystemPath must name an exact lowercase .xml file or the root Subsystems directory"
                .to_string(),
        );
    }

    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("[ERROR] File not found: {}: {error}", path.display()))?;
    if unica_workspace::paths::metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err(
            "SubsystemPath must name a regular lowercase .xml file, not a filesystem alias"
                .to_string(),
        );
    }
    let xml_path = path;
    let scope = subsystem_descriptor_scope(&xml_path, context)?;
    let mut format_documents = Vec::new();
    let mut registered_support_target = None;
    let (descriptor_bytes, tree) = if let Some((source_root, address_names, address)) = scope {
        let topology = capture_registered_subsystem_topology(&source_root, &mut checkpoint)
            .map_err(|error| error.to_string())?;
        format_documents = topology_format_documents(&source_root, &topology);
        if let Some(registered) = subsystem_node_by_address(&topology.roots, &address) {
            registered_support_target = Some((source_root.clone(), address.clone()));
            let relative = xml_path.strip_prefix(&source_root).map_err(|_| {
                "registered subsystem descriptor escaped its lexical source root".to_string()
            })?;
            let descriptor = topology
                .dependency_documents()
                .iter()
                .find(|document| document.path == relative)
                .ok_or_else(|| {
                    format!("registered subsystem `{address}` has no captured descriptor")
                })?;
            let tree = focused_subsystem_tree_from_topology(
                &topology,
                &address_names,
                &address,
                registered,
            )?;
            (descriptor.bytes.clone(), Some(vec![tree]))
        } else {
            let bytes = read_subsystem_info_file(&xml_path, &mut checkpoint)?;
            format_documents.push(SubsystemInfoFormatDocument {
                path: xml_path.clone(),
                bytes: bytes.clone(),
            });
            (bytes, None)
        }
    } else {
        let bytes = read_subsystem_info_file(&xml_path, &mut checkpoint)?;
        format_documents.push(SubsystemInfoFormatDocument {
            path: xml_path.clone(),
            bytes: bytes.clone(),
        });
        (bytes, None)
    };
    let descriptor_text = std::str::from_utf8(&descriptor_bytes)
        .map_err(|error| format!("failed to read {}: {error}", xml_path.display()))?;
    let (data, _) = parse_subsystem_info_data(
        &ProjectDiscoveryReader::new(&context.cwd),
        &xml_path,
        descriptor_text,
    )?;
    if tree.is_some() {
        let selected_name = xml_path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".xml"))
            .expect("registered subsystem shape was checked before capture");
        if data.name != selected_name {
            return Err(format!(
                "registered subsystem `{selected_name}` declares Name `{}` instead of `{selected_name}`",
                data.name
            ));
        }
    }
    checkpoint().map_err(subsystem_info_checkpoint_message)?;

    let command_interface_path = subsystem_command_interface_path(&xml_path);
    checkpoint().map_err(subsystem_info_checkpoint_message)?;
    let command_interface = if command_interface_path.is_file() {
        let bytes = read_subsystem_info_file(&command_interface_path, &mut checkpoint)?;
        let text = std::str::from_utf8(&bytes).map_err(|error| {
            format!(
                "failed to read {}: {error}",
                command_interface_path.display()
            )
        })?;
        let data = parse_subsystem_command_interface_data(&command_interface_path, text)?;
        format_documents.push(SubsystemInfoFormatDocument {
            path: command_interface_path,
            bytes,
        });
        Some(data)
    } else {
        None
    };
    checkpoint().map_err(subsystem_info_checkpoint_message)?;

    // Overview, content, ci and full were slices of one subsystem chosen to
    // keep a printed report short. Data carries all of them at once.
    let support = match registered_support_target {
        Some((_source_root, address)) if !address.as_str().contains('.') => {
            let selection = physical_selection(&xml_path, context, AttachedResource::Descriptor)
                .map_err(|failure| failure.to_string())?;
            support_reader
                .object_support(&selection.target)
                .map_err(|error| error.to_string())?
        }
        Some((source_root, address)) => {
            let source_set = subsystem_source_set_name(context, &source_root)?;
            support_reader
                .subsystem_support(&ResolvedSubsystemTarget {
                    source_set,
                    address,
                })
                .map_err(|error| error.to_string())?
        }
        None => DomainObjectSupportData {
            state: ObjectSupportState::NotSupported,
            direct_edit_safe: None,
        },
    };
    checkpoint().map_err(subsystem_info_checkpoint_message)?;
    let result = build_subsystem_info_result(data, tree, command_interface, support);
    let summary = format!(
        "unica.subsystem.info described {} with {} content item(s)",
        result.name,
        result.content.len()
    );
    checkpoint().map_err(subsystem_info_checkpoint_message)?;
    Ok(PreparedSubsystemInfo {
        execution: subsystem_info_success(
            SubsystemInfoAnswer::Subsystem(Box::new(result)),
            xml_path,
            summary,
        ),
        format_documents,
    })
}

fn subsystem_info_success(
    data: SubsystemInfoAnswer,
    artifact: PathBuf,
    summary: String,
) -> SubsystemInfoExecution {
    SubsystemInfoExecution {
        outcome: AdapterOutcome {
            ok: true,
            summary,
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![artifact.display().to_string()],
            stdout: None,
            stderr: Some(String::new()),
            command: None,
        },
        data: Some(data),
    }
}

pub(crate) fn subsystem_info_failure(error: String) -> SubsystemInfoExecution {
    SubsystemInfoExecution {
        outcome: AdapterOutcome {
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
        data: None,
    }
}

fn read_subsystem_info_file(
    path: &Path,
    mut checkpoint: impl FnMut() -> io::Result<()>,
) -> Result<Vec<u8>, String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("subsystem info path has no parent: {}", path.display()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("subsystem info path has no file name: {}", path.display()))?;
    let mut reader = RetainedRootSecureRead::open(
        parent,
        SecureTreeCaptureLimits { maximum_depth: 0 },
        &mut checkpoint,
    )
    .map_err(|error| format!("failed to open {} securely: {error}", path.display()))?;
    let read = reader
        .read_regular_file(Path::new(file_name), &mut checkpoint)
        .map_err(|error| format!("failed to read {} securely: {error}", path.display()))?;
    reader
        .complete(&mut checkpoint)
        .map_err(|error| format!("failed to prove {} securely: {error}", path.display()))?;
    Ok(read.bytes)
}

fn subsystem_info_checkpoint_message(error: io::Error) -> String {
    if error.kind() == io::ErrorKind::Interrupted
        && error
            .to_string()
            .starts_with(unica_work_control::cancellation::CANCELLED_PREFIX)
    {
        return error.to_string();
    }
    format!("subsystem info snapshot failed: {error}")
}

fn subsystem_descriptor_scope(
    descriptor: &Path,
    context: &WorkspaceContext,
) -> Result<Option<(PathBuf, Vec<String>, SubsystemAddress)>, String> {
    let Some(file_name) = descriptor.file_name().and_then(|name| name.to_str()) else {
        return Ok(None);
    };
    let Some(file_name) = file_name.strip_suffix(".xml") else {
        return Ok(None);
    };
    let Some(parent) = descriptor.parent() else {
        return Ok(None);
    };
    if parent.file_name().and_then(|name| name.to_str()) != Some("Subsystems") {
        return Ok(None);
    }
    let Some((source_root, mut address_names)) = subsystem_tree_scope(parent, context)? else {
        return Ok(None);
    };
    match fs::symlink_metadata(source_root.join("Configuration.xml")) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "failed to inspect Configuration.xml for subsystem scope: {error}"
            ))
        }
    }
    address_names.push(file_name.to_string());
    let address = SubsystemAddress::from_names(address_names.iter().map(String::as_str))
        .map_err(|error| error.to_string())?;
    Ok(Some((source_root, address_names, address)))
}

fn subsystem_source_set_name(
    context: &WorkspaceContext,
    source_root: &Path,
) -> Result<String, String> {
    let source_root = normalize_path_identity(source_root)?;
    let mut matches = discover_project_source_map(&context.workspace_root)?
        .source_sets
        .into_iter()
        .filter(|source_set| {
            matches!(
                source_set.kind,
                SourceSetKind::Configuration | SourceSetKind::Extension
            )
        })
        .filter_map(|source_set| {
            let configured = PathBuf::from(&source_set.path);
            let configured = if configured.is_absolute() {
                configured
            } else {
                context.workspace_root.join(configured)
            };
            normalize_path_identity(&configured)
                .ok()
                .filter(|configured| configured == &source_root)
                .map(|_| source_set.name)
        })
        .collect::<Vec<_>>();
    matches.sort();
    matches.dedup();
    match matches.as_slice() {
        [source_set] => Ok(source_set.clone()),
        [] => Err("provider_unavailable: the subsystem source set is unavailable".to_string()),
        _ => Err("provider_unavailable: the subsystem source set is ambiguous".to_string()),
    }
}

fn subsystem_tree_scope(
    path: &Path,
    context: &WorkspaceContext,
) -> Result<Option<(PathBuf, Vec<String>)>, String> {
    let subsystem_ancestors = path
        .ancestors()
        .filter(|ancestor| {
            ancestor.file_name().and_then(|name| name.to_str()) == Some("Subsystems")
        })
        .collect::<Vec<_>>();
    let configured_source_roots = discover_project_source_map(&context.workspace_root)?
        .source_sets
        .into_iter()
        .filter(|source_set| {
            matches!(
                source_set.kind,
                SourceSetKind::Configuration | SourceSetKind::Extension
            )
        })
        .map(|source_set| {
            let configured = PathBuf::from(source_set.path);
            let configured = if configured.is_absolute() {
                configured
            } else {
                context.workspace_root.join(configured)
            };
            normalize_path_identity(&configured)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let configured_outermost = subsystem_ancestors.iter().copied().find(|ancestor| {
        ancestor.parent().is_some_and(|parent| {
            normalize_path_identity(parent)
                .is_ok_and(|parent| configured_source_roots.contains(&parent))
        })
    });
    let outermost = if configured_source_roots.is_empty() {
        subsystem_ancestors.last().copied()
    } else {
        configured_outermost
    };
    let Some(outermost) = outermost else {
        return Ok(None);
    };
    let source_root = outermost
        .parent()
        .ok_or_else(|| "Subsystems directory has no source root".to_string())?
        .to_path_buf();
    let relative = path
        .strip_prefix(outermost)
        .map_err(|_| "failed to identify the requested subsystem scope".to_string())?;
    let parts = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| "subsystem scope contains a non-Unicode name".to_string()),
            _ => Err("subsystem scope contains an invalid path component".to_string()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if parts.len() % 2 != 0
        || parts
            .as_chunks::<2>()
            .0
            .iter()
            .any(|pair| pair.get(1).map(String::as_str) != Some("Subsystems"))
    {
        return Err("subsystem scope does not follow the nested Subsystems layout".to_string());
    }
    let parent_address = parts
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| pair[0].clone())
        .collect();
    Ok(Some((source_root, parent_address)))
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
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
        "subsystem-edit" => Some(edit_subsystem(args, context)),
        _ => None,
    }
}

struct SubsystemEditResult {
    data: SubsystemEditData,
    artifacts: Vec<PathBuf>,
    changes: Vec<String>,
    warnings: Vec<String>,
}

pub(crate) struct SubsystemEditExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<SubsystemEditData>,
}

pub(crate) struct SubsystemInfoExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<SubsystemInfoAnswer>,
}

pub(crate) struct PreparedSubsystemInfo {
    pub(crate) execution: SubsystemInfoExecution,
    pub(crate) format_documents: Vec<SubsystemInfoFormatDocument>,
}

#[derive(Debug)]
pub(crate) enum SubsystemInfoPreparationError {
    Control(String),
    Provider(String),
}

impl SubsystemInfoPreparationError {
    pub(crate) fn is_control(&self) -> bool {
        matches!(self, Self::Control(_))
    }
}

impl std::fmt::Display for SubsystemInfoPreparationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Control(message) | Self::Provider(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for SubsystemInfoPreparationError {}

use unica_metadata::subsystem_topology::*;
use unica_metadata_designer::subsystem_topology::*;
pub(crate) fn capture_registered_subsystem_topology(
    source_root: &Path,
    mut checkpoint: impl FnMut() -> io::Result<()>,
) -> Result<SubsystemTopology, SubsystemTopologyError> {
    let reader = RetainedRootSecureRead::open(
        source_root,
        SecureTreeCaptureLimits {
            maximum_depth: SUBSYSTEM_ADDRESS_MAX_DEPTH * 2,
        },
        &mut checkpoint,
    )?;
    read_registered_subsystem_topology(Box::new(reader), &mut checkpoint)
}

#[cfg(test)]
#[path = "../../tests/platform/subsystem_subsystem_info_typed_result_tests.rs"]
pub(crate) mod subsystem_info_typed_result_tests;

#[cfg(test)]
#[path = "../../tests/platform/subsystem_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "../../tests/platform/subsystem_subsystem_read_selector_bridge_tests.rs"]
pub(crate) mod subsystem_read_selector_bridge_tests;
