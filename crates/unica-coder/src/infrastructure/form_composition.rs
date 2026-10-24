//! Concrete source binding, preimage guards, publication and terminal projection for Forms.
use crate::application::operation_descriptors::{FORM_PATH, OBJECT_PATH};
use crate::infrastructure::native_operations::common::*;
use crate::infrastructure::native_operations::logical_selector::{
    logical_selection, physical_selection, typed_reader_metadata_target, AttachedResource,
    ResolvedReadTarget,
};
use crate::infrastructure::native_operations::meta::validate_metadata_owner_shape_8_3_27;
use crate::infrastructure::support_state::WorkspaceSupportStateReader;
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use unica_address::address::QualifiedAddress;
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_forms::facts::Utf8Bom;
use unica_forms::facts::*;
use unica_forms::validate_form_edit_definition;
use unica_forms_designer::codec::*;
use unica_forms_designer::read::{resolve_form_add_object_path, resolve_form_info_path};
use unica_metadata::support_state::SupportStateReader;
use unica_operation_contract::input::*;
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_designer::owner::MANAGED_FORM_ROOT;
use unica_workspace::compile_transaction::CompileTransaction;
use unica_workspace::discovery::ProjectDiscoveryReader;
use unica_workspace::paths::normalize_path_identity;
use unica_xml_format::text::*;
const FORM_KINDS: &[&str] = &["Form", "CommonForm"];
#[cfg(test)]
type FormCompileAfterParentOwnerProbeHook = Box<dyn FnOnce(&Path)>;

#[cfg(test)]
thread_local! {
    static FORM_COMPILE_AFTER_PARENT_OWNER_PROBE_HOOK:
        std::cell::RefCell<Option<FormCompileAfterParentOwnerProbeHook>> =
        const { std::cell::RefCell::new(None) };
}

pub(crate) fn validate_form(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    validate_form_inner(args, context, None, HandlerSignatures::Check)
}

/// Validation inside `form.edit`: the verdict only, without handler
/// signature warnings that the edit path does not report.
pub(crate) fn validate_form_for_edit(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    source_override: Option<(&Path, &str)>,
) -> AdapterOutcome {
    validate_form_inner(args, context, source_override, HandlerSignatures::Skip)
}

fn validate_form_inner(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    source_override: Option<(&Path, &str)>,
    signatures: HandlerSignatures,
) -> AdapterOutcome {
    let result = (|| {
        let reader = ProjectDiscoveryReader::new(&context.cwd);
        let form_path = match source_override {
            Some((path, _)) => path.to_path_buf(),
            None => {
                let path = resolve_form_read_path(args, context)?;
                if !path.is_file() {
                    return Err(format!("File not found: {}", path.display()));
                }
                path
            }
        };
        let text = match source_override {
            Some((_, source)) => source.to_string(),
            None => read_utf8_sig(&form_path)?,
        };
        validate_form_image(args, &reader, &form_path, &text, signatures)
    })();
    form_validation_outcome(result)
}

pub(crate) fn form_validation_outcome(
    result: Result<unica_forms_designer::codec::FormValidationRun, String>,
) -> AdapterOutcome {
    match result {
        Ok((ok, stdout, artifact, validation_errors, validation_warnings)) => AdapterOutcome {
            ok,
            summary: if ok {
                "unica.form.validate completed with native form validator".to_string()
            } else {
                "unica.form.validate failed in native form validator".to_string()
            },
            changes: Vec::new(),
            warnings: validation_warnings,
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

/// Нужен ли модуль формы для проверки сигнатур обработчиков; решает кодек формы.
pub(crate) fn form_validation_requires_module(
    text: &str,
    metadata_owner: Option<unica_address::address::NodeKind>,
) -> bool {
    unica_forms_designer::codec::form_validation_requires_module(text, metadata_owner)
}

/// Связывает сохранённые факты чтения с кодеком формы.
pub(crate) fn validate_form_input(
    artifact: &Path,
    text: &str,
    module: Option<&str>,
    config_context: bool,
    metadata_owner: Option<unica_address::address::NodeKind>,
) -> AdapterOutcome {
    form_validation_outcome(
        unica_forms_designer::codec::validate_form_image_with_context(
            &Map::new(),
            artifact,
            text,
            unica_forms_designer::codec::FormValidationContext {
                config_context,
                metadata_owner,
                module,
            },
        ),
    )
}

pub(crate) fn analyze_form_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    support_reader: &dyn SupportStateReader,
) -> AdapterOutcome {
    analyze_form_info_with_data(args, context, support_reader).outcome
}

pub(crate) fn typed_form_reader_target(
    address: &QualifiedAddress,
) -> Option<unica_address::source_target::MetadataAddress> {
    typed_reader_metadata_target(address, FORM_KINDS)
}

/// The single args→path entry point of `form.info` and `form.validate`, and the
/// one the format guard calls.
pub(crate) fn resolve_form_read_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    if let Some(selection) = logical_selection(args, context, AttachedResource::Form, FORM_KINDS) {
        return selection
            .map(|selection| selection.resource_path)
            .map_err(|failure| failure.to_string());
    }
    let raw_path = unica_operation_contract::input::required_path(args, FORM_PATH, "FormPath")?;
    Ok(resolve_form_info_path(
        &ProjectDiscoveryReader::new(&context.cwd),
        unica_operation_contract::input::absolutize(raw_path, &context.cwd),
    ))
}

pub(crate) fn resolve_form_info_target(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<ResolvedReadTarget, String> {
    if let Some(selection) = logical_selection(args, context, AttachedResource::Form, FORM_KINDS) {
        return selection.map_err(|failure| failure.to_string());
    }
    let form_path = resolve_form_read_path(args, context)?;
    physical_selection(&form_path, context, AttachedResource::Form)
        .map_err(|failure| failure.to_string())
}

pub(crate) fn analyze_form_info_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    support_reader: &dyn SupportStateReader,
) -> FormInfoExecution {
    let result = (|| -> Result<(FormInfoData, PathBuf), String> {
        let selection = resolve_form_info_target(args, context)?;
        let form_path = selection.resource_path;
        if !form_path.is_file() {
            return Err(format!("File not found: {}", form_path.display()));
        }

        let text = read_utf8_sig(&form_path)?;
        let (form_name, object_context) =
            form_info_context(&ProjectDiscoveryReader::new(&context.cwd), &form_path);
        let support = support_reader
            .object_support(&selection.target)
            .map_err(|error| error.to_string())?;
        let data = parse_form_info_xml(&text, form_name, object_context, support)?;

        Ok((data, form_path))
    })();

    match result {
        Ok((data, artifact)) => FormInfoExecution {
            outcome: AdapterOutcome {
                ok: true,
                summary: format!(
                    "unica.form.info described {} with {} element(s) and {} attribute(s)",
                    data.name,
                    data.elements.len(),
                    data.attributes.len()
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
        Err(error) => FormInfoExecution {
            outcome: AdapterOutcome {
                ok: false,
                summary: "unica.form.info failed in native form analyzer".to_string(),
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

pub(crate) fn add_form(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    add_form_with_data(args, context).outcome
}

pub(crate) fn add_form_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> FormAddExecution {
    let result = (|| -> Result<(FormAddData, Vec<PathBuf>, Vec<String>), String> {
        let object_path_raw =
            unica_operation_contract::input::required_path(args, OBJECT_PATH, "ObjectPath")?;
        let form_name = required_string(args, &["formName", "FormName"], "FormName")?;
        validate_form_metadata_path_name("FormName", form_name)?;
        let synonym = string_arg(args, &["synonym", "Synonym"]).unwrap_or(form_name);
        let purpose_raw = string_arg(args, &["purpose", "Purpose"]).unwrap_or("Object");
        let set_default = optional_bool_arg(args, &["setDefault", "SetDefault"]);

        let object_xml_full = resolve_form_add_object_path(
            &ProjectDiscoveryReader::new(&context.cwd),
            unica_operation_contract::input::absolutize(object_path_raw, &context.cwd),
        )?;
        validate_metadata_owner_shape_8_3_27(&object_xml_full, context, "form.add")?;
        let object_source = read_utf8_sig_snapshot(&object_xml_full)?;
        let object_source_text = object_source.text;
        let mut object_text = object_source_text.clone();
        let (object_type, object_name) = detect_form_add_object(&object_text)?;
        let format_version = detect_format_version(&object_xml_full, context)?.to_string();

        let purpose = normalize_form_purpose(purpose_raw);
        validate_form_purpose(&object_type, &purpose)?;

        let object_dir = object_xml_full.with_extension("");
        let forms_dir = object_dir.join("Forms");
        let form_meta_path = forms_dir.join(format!("{form_name}.xml"));

        let form_dir = forms_dir.join(form_name);
        let form_ext_dir = form_dir.join("Ext");
        let form_module_dir = form_ext_dir.join("Form");
        let form_metadata = form_add_metadata_xml(
            form_name,
            synonym,
            &object_type,
            &format_version,
            &uuid::Uuid::new_v4().to_string(),
            LEGACY_FORM_TEXT_LANGUAGE,
        );

        let form_xml_path = form_ext_dir.join("Form.xml");
        let form_content =
            form_add_content_xml(&object_type, &object_name, &purpose, &format_version)?;
        let mut stdout = String::new();
        stdout.push('\n');
        stdout.push_str("=== form-add ===\n\n");
        stdout.push_str(&format!("Object: {object_type}.{object_name}\n"));

        let module_path = form_module_dir.join("Module.bsl");

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
        let object_replacement = utf8_bom_bytes(
            &lxml_tree_serialized_text_like_source_preserving_final_newline(
                &object_text,
                &object_source_text,
            ),
        );

        let mut transaction = CompileTransaction::new();
        transaction.create_utf8_bom_text(&form_meta_path, &form_metadata)?;
        transaction.create_utf8_bom_text(&form_xml_path, &form_content)?;
        transaction.create_utf8_bom_text(&module_path, form_add_module_bsl())?;
        transaction.replace_bytes(&object_xml_full, &object_source.raw, object_replacement)?;
        guard_active_format_owner(&mut transaction, &object_xml_full, context)?;
        let validation_path = object_xml_full.clone();
        let report = transaction.commit_with_post_validation(|| {
            validate_metadata_owner_shape_8_3_27(&validation_path, context, "form.add")
        })?;

        let obj_dir_name = object_xml_full
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .display()
            .to_string();
        let obj_base_name = object_xml_full
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let _ = (stdout, obj_dir_name, obj_base_name, default_value);
        let data = FormAddData {
            object_kind: object_type.to_string(),
            object_name: object_name.to_string(),
            form: form_name.to_string(),
            registered_in: object_xml_full.display().to_string(),
            default_property: default_updated.then(|| default_prop_name.to_string()),
            mutation: unica_execution_contract::mutation::MutationData::new(true)
                .updated(&object_xml_full)
                .created(&form_meta_path)
                .created(&form_xml_path)
                .created(&module_path),
        };

        Ok((
            data,
            vec![object_xml_full, form_meta_path, form_xml_path, module_path],
            report.cleanup_warnings,
        ))
    })();

    match result {
        Ok((data, artifacts, warnings)) => FormAddExecution {
            outcome: AdapterOutcome {
                ok: true,
                summary: format!(
                    "unica.form.add created form {} for {}.{}",
                    data.form, data.object_kind, data.object_name
                ),
                changes: artifacts
                    .iter()
                    .map(|path| format!("updated {}", path.display()))
                    .collect(),
                warnings,
                errors: Vec::new(),
                artifacts: artifacts
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
                stdout: None,
                stderr: None,
                command: None,
            },
            data: Some(data),
        },
        Err(error) => FormAddExecution {
            outcome: AdapterOutcome {
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
            data: None,
        },
    }
}

pub(crate) fn remove_form(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    remove_form_with_data(args, context).outcome
}

pub(crate) fn remove_form_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> FormRemoveExecution {
    let result = (|| -> Result<(unica_execution_contract::mutation::MutationData, Vec<String>, Vec<String>), String> {
        let object_name = required_string(
            args,
            &["objectName", "ObjectName", "processorName", "ProcessorName"],
            "ObjectName",
        )?;
        let form_name = required_string(args, &["formName", "FormName"], "FormName")?;
        validate_form_metadata_path_name("ObjectName", object_name)?;
        validate_form_metadata_path_name("FormName", form_name)?;
        let src_dir_raw = string_arg(args, &["srcDir", "SrcDir"]).unwrap_or("src");
        let src_dir_display = PathBuf::from(src_dir_raw);
        let src_dir_abs = unica_operation_contract::input::absolutize(src_dir_display.clone(), &context.cwd);

        let root_xml_display = src_dir_display.join(format!("{object_name}.xml"));
        let root_xml_path = src_dir_abs.join(format!("{object_name}.xml"));
        if !root_xml_path.exists() {
            return Err(format!(
                "Корневой файл обработки не найден: {}",
                root_xml_display.display()
            ));
        }
        validate_metadata_owner_shape_8_3_27(&root_xml_path, context, "form.remove")?;

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

        let form_dir_exists = match fs::symlink_metadata(&form_dir_path) {
            Ok(metadata) if metadata.is_dir() => true,
            Ok(_) => {
                return Err(format!(
                    "Каталог формы не является каталогом: {}",
                    form_dir_display.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(format!(
                    "failed to inspect {}: {error}",
                    form_dir_path.display()
                ));
            }
        };

        let source = read_utf8_sig_snapshot(&root_xml_path)?;
        let source_xml_text = source.text;
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

        let collection_targets = if form_dir_exists {
            vec![form_meta_path.as_path(), form_dir_path.as_path()]
        } else {
            vec![form_meta_path.as_path()]
        };

        let mut transaction = CompileTransaction::new();
        transaction.replace_bytes(&root_xml_path, &source.raw, utf8_bom_bytes(&xml_text))?;
        let remove_forms_collection = transaction.remove_directory_if_only_direct_entries(
            &forms_dir_abs,
            collection_targets
                .iter()
                .map(|path| {
                    path.file_name()
                        .expect("form collection target must have a file name")
                        .to_os_string()
                })
                .collect(),
        )?;
        if !remove_forms_collection {
            if form_dir_exists {
                transaction.remove_path(&form_dir_path)?;
            } else {
                transaction.guard_path_absent(&form_dir_path)?;
            }
            transaction.remove_path(&form_meta_path)?;
        }
        let trees = if form_dir_exists {
            vec![form_meta_path.as_path(), form_dir_path.as_path()]
        } else {
            vec![form_meta_path.as_path()]
        };
        guard_active_format_dependencies_and_xml_trees(
            &mut transaction,
            &[root_xml_path.as_path()],
            &trees,
            context,
        )?;
        let validation_path = root_xml_path.clone();
        let validation_form_meta = form_meta_path.clone();
        let validation_form_dir = form_dir_path.clone();
        let report = transaction.commit_with_post_validation(move || {
            validate_metadata_owner_shape_8_3_27(&validation_path, context, "form.remove")?;
            for path in [&validation_form_meta, &validation_form_dir] {
                match fs::symlink_metadata(path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Ok(_) => {
                        return Err(format!(
                            "form.remove post-write validation found removed pair member still present: {}",
                            path.display()
                        ));
                    }
                    Err(error) => {
                        return Err(format!(
                            "form.remove post-write validation failed to inspect {}: {error}",
                            path.display()
                        ));
                    }
                }
            }
            Ok(())
        })?;

        let mut stdout = String::new();
        let mut changes = Vec::new();
        let mut mutation = unica_execution_contract::mutation::MutationData::new(true);
        if form_dir_exists {
            stdout.push_str(&format!(
                "[OK] Удалён каталог: {}\n",
                form_dir_display.display()
            ));
            changes.push(format!("removed directory {}", form_dir_path.display()));
            mutation = mutation.removed(&form_dir_path);
        }
        if remove_forms_collection {
            changes.push(format!(
                "removed empty collection directory {}",
                forms_dir_abs.display()
            ));
        }
        stdout.push_str(&format!(
            "[OK] Удалён файл: {}\n",
            form_meta_display.display()
        ));
        changes.push(format!("removed file {}", form_meta_path.display()));
        mutation = mutation.removed(&form_meta_path);
        if removed_form_refs > 0 {
            changes.push(format!("removed {removed_form_refs} Form reference(s)"));
        }
        for tag in cleared_form_slots {
            changes.push(format!("cleared {tag}"));
        }
        changes.push(format!("updated {}", root_xml_path.display()));
        mutation = mutation.updated(&root_xml_path);

        stdout.push_str(&format!(
            "[OK] Форма {form_name} удалена из {}\n",
            root_xml_display.display()
        ));
        let _ = stdout;
        Ok((mutation, changes, report.cleanup_warnings))
    })();

    match result {
        Ok((mutation, changes, warnings)) => FormRemoveExecution {
            outcome: AdapterOutcome {
                ok: true,
                summary: format!(
                    "unica.form.remove removed {} path(s)",
                    mutation.removed.len()
                ),
                changes,
                warnings,
                errors: Vec::new(),
                artifacts: Vec::new(),
                stdout: None,
                stderr: Some(String::new()),
                command: None,
            },
            data: Some(mutation),
        },
        Err(error) => FormRemoveExecution {
            outcome: AdapterOutcome {
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
            data: None,
        },
    }
}

fn form_compile_derivation_snapshot_for_path(
    inputs: &[FormCompileDerivationInput],
    target: &Path,
) -> Result<Option<Utf8TextSnapshot>, String> {
    let target = normalize_path_identity(target)?;
    for input in inputs {
        if normalize_path_identity(&input.path)? == target {
            return Ok(Some(input.snapshot.clone()));
        }
    }
    Ok(None)
}

pub(crate) fn compile_form(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let write_result = plan_form_compile(
        args,
        &ProjectDiscoveryReader::new(&context.cwd),
        &context.cwd,
        &|path| detect_format_version(path, context).map(|version| version.to_string()),
    )
    .and_then(|mut plan| {
        let output_path = plan.output_path.clone();
        let owner_candidate = form_parent_metadata_owner_candidate(&output_path)?;
        let owner_validation_snapshot = match owner_candidate.as_deref() {
            Some(owner_path) => {
                match form_compile_derivation_snapshot_for_path(
                    &plan.derivation_inputs,
                    owner_path,
                )? {
                    Some(snapshot) => Some(snapshot),
                    None => match fs::symlink_metadata(owner_path) {
                        Ok(metadata)
                            if metadata.is_file() && !metadata.file_type().is_symlink() =>
                        {
                            Some(read_utf8_sig_snapshot(owner_path)?)
                        }
                        Ok(_) => {
                            return Err(format!(
                                "form parent metadata owner is not a regular file: {}",
                                owner_path.display()
                            ));
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                        Err(error) => {
                            return Err(format!(
                                "failed to inspect form parent metadata owner {}: {error}",
                                owner_path.display()
                            ));
                        }
                    },
                }
            }
            None => None,
        };
        let mut transaction = CompileTransaction::new();
        if let (Some(owner_path), None) = (
            owner_candidate.as_deref(),
            owner_validation_snapshot.as_ref(),
        ) {
            transaction.guard_path_absent(owner_path)?;
        }
        #[cfg(test)]
        run_form_compile_after_parent_owner_probe_hook(&output_path);
        if let (Some(owner_path), Some(_)) = (
            owner_candidate.as_deref(),
            owner_validation_snapshot.as_ref(),
        ) {
            validate_metadata_owner_shape_8_3_27(owner_path, context, "form.compile")?;
        }
        let registration = match (
            owner_candidate.as_deref(),
            owner_validation_snapshot.as_ref(),
        ) {
            (Some(owner_path), Some(owner_snapshot)) => {
                plan_form_registration_in_parent_object(&output_path, owner_path, owner_snapshot)?
            }
            _ => None,
        };
        transaction.create_or_replace_bytes(&output_path, utf8_bom_bytes(&plan.xml))?;
        let mut registration_stdout = None;
        if let Some(FormParentRegistrationPlan {
            path,
            original,
            replacement,
            stdout,
        }) = registration
        {
            transaction.replace_bytes(path, &original, replacement)?;
            registration_stdout = Some(stdout);
        }
        if let (Some(owner_path), Some(owner_snapshot)) = (
            owner_candidate.as_deref(),
            owner_validation_snapshot.as_ref(),
        ) {
            if !transaction.protects_path(owner_path)? {
                transaction.guard_exact_preimage(owner_path, &owner_snapshot.raw)?;
            }
        }
        for input in &plan.derivation_inputs {
            guard_exact_preimage_if_unprotected(
                &mut transaction,
                &input.path,
                &input.snapshot.raw,
            )?;
        }
        guard_active_format_owner_with_exact_root(
            &mut transaction,
            &output_path,
            context,
            MANAGED_FORM_ROOT,
        )?;
        let mut format_dependencies = Vec::new();
        if let Some(owner_path) = owner_candidate.as_deref() {
            format_dependencies.push(owner_path);
        }
        format_dependencies.extend(
            plan.derivation_inputs
                .iter()
                .filter(|input| input.platform_xml)
                .map(|input| input.path.as_path()),
        );
        guard_active_format_dependencies(&mut transaction, &format_dependencies, context)?;
        let report = transaction.commit_with_post_validation(|| {
            if let (Some(owner_path), Some(_)) = (
                owner_candidate.as_deref(),
                owner_validation_snapshot.as_ref(),
            ) {
                validate_metadata_owner_shape_8_3_27(owner_path, context, "form.compile")
            } else {
                Ok(())
            }
        })?;

        if let Some(registration_stdout) = registration_stdout {
            plan.stdout.push_str(&registration_stdout);
        }
        plan.stdout
            .push_str(&format!("[OK] Compiled: {}\n", plan.output_label));
        append_form_compile_stats(&mut plan.stdout, &plan.stats);

        Ok((plan.stdout, output_path, report.cleanup_warnings))
    });

    match write_result {
        Ok((stdout, output_path, warnings)) => AdapterOutcome {
            ok: true,
            summary: "unica.form.compile completed with native managed form compiler".to_string(),
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
    let mut plan = plan_form_compile(
        args,
        &ProjectDiscoveryReader::new(&context.cwd),
        &context.cwd,
        &|path| detect_format_version(path, context).map(|version| version.to_string()),
    )?;
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

pub(crate) fn edit_form(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    apply_with_data(args, context).outcome
}

pub(crate) fn preview_form_edit(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    preview_with_data(args, context).outcome
}

pub(crate) fn apply_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> FormEditExecution {
    form_edit_with_mode_data(args, context, FormEditMode::Apply)
}

pub(crate) fn preview_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> FormEditExecution {
    form_edit_with_mode_data(args, context, FormEditMode::Preview)
}

fn form_edit_with_mode_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    mode: FormEditMode,
) -> FormEditExecution {
    let edit_result = (|| -> Result<FormEditSuccess, String> {
        let form_path_raw =
            unica_operation_contract::input::required_path(args, FORM_PATH, "FormPath")?;
        let form_path =
            unica_operation_contract::input::absolutize(form_path_raw.clone(), &context.cwd);
        if !form_path.exists() {
            return Err(format!("File not found: {}", form_path_raw.display()));
        }

        let mut transaction = CompileTransaction::new();
        let defn = form_edit_resolve_definition_guarded(args, context, &mut transaction)?;
        let original_bytes = fs::read(&form_path)
            .map_err(|err| format!("failed to read {}: {err}", form_path.display()))?;
        let (mut xml_text, bom, form_root_start) =
            form_edit_decode_image(&original_bytes, &form_path)?;
        let original_xml_text = xml_text.clone();
        let FormEditApplied {
            added_elements,
            added_attrs,
            added_cmds,
            added_form_events,
            added_element_events,
            planned_removals,
            companion_count,
            form_name,
        } = form_edit_apply_definition(
            &mut xml_text,
            &defn,
            &form_path,
            form_root_start,
            Some(LEGACY_FORM_TEXT_LANGUAGE),
        )?;
        form_edit_require_valid(validate_form_for_edit(
            args,
            context,
            Some((&form_path, &xml_text)),
        ))?;
        let changed = xml_text != original_xml_text;
        let mut warnings = Vec::new();
        if changed && !mode.is_preview() {
            warnings = form_edit_publish_preserving_bom(
                transaction,
                &form_path,
                &original_bytes,
                &xml_text,
                bom,
                context,
            )?;
        }

        let _ = &form_name;
        let _ = companion_count;
        Ok(FormEditSuccess {
            added_elements,
            added_attributes: added_attrs,
            added_commands: added_cmds,
            added_events: added_form_events
                .into_iter()
                .chain(added_element_events)
                .collect(),
            form_path,
            changed,
            warnings,
            removals: planned_removals,
        })
    })();

    match edit_result {
        Ok(FormEditSuccess {
            added_elements,
            added_attributes,
            added_commands,
            added_events,
            form_path,
            changed,
            warnings,
            removals,
        }) => FormEditExecution {
            outcome: AdapterOutcome {
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
                warnings,
                errors: Vec::new(),
                artifacts: vec![form_path.display().to_string()],
                stdout: None,
                stderr: None,
                command: None,
            },
            data: Some(FormEditData {
                changed,
                removed: form_edit_removed_elements(&removals),
                added_elements,
                added_attributes,
                added_commands,
                added_events,
                validation: FormEditValidation::Passed,
            }),
        },
        Err(error) => FormEditExecution {
            outcome: AdapterOutcome {
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
            data: None,
        },
    }
}

fn form_edit_resolve_definition_guarded(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    transaction: &mut CompileTransaction,
) -> Result<Value, String> {
    let inline = args.get("definition");
    let json_path_raw = unica_operation_contract::input::path_arg(args, &["jsonPath", "JsonPath"]);
    match (inline, json_path_raw) {
        (Some(_), Some(_)) => {
            Err("unica.form.edit accepts exactly one of JsonPath or inline definition".to_string())
        }
        (Some(definition), None) => {
            validate_form_edit_definition(definition)?;
            Ok(definition.clone())
        }
        (None, Some(json_path_raw)) => {
            let json_path =
                unica_operation_contract::input::absolutize(json_path_raw.clone(), &context.cwd);
            if !json_path.exists() {
                return Err(format!("File not found: {}", json_path_raw.display()));
            }
            let definition = FileBackedJson::read(&json_path, |err| {
                format!("failed to parse form edit JSON: {err}")
            })?
            .bind_to(transaction)?;
            validate_form_edit_definition(&definition)?;
            Ok(definition)
        }
        (None, None) => {
            Err("unica.form.edit requires exactly one of JsonPath or definition".to_string())
        }
    }
}

pub(crate) fn form_edit_publish_preserving_bom(
    mut transaction: CompileTransaction,
    path: &Path,
    expected_preimage: &[u8],
    xml_text: &str,
    bom: Utf8Bom,
    context: &WorkspaceContext,
) -> Result<Vec<String>, String> {
    let bytes = form_edit_encoded_bytes(xml_text, bom);
    transaction.replace_bytes(path, expected_preimage, bytes)?;
    guard_active_format_owner(&mut transaction, path, context)?;
    let validation_path = path.to_path_buf();
    let report = transaction.commit_with_post_validation(move || {
        let validation_args = Map::from_iter([(
            "FormPath".to_string(),
            Value::String(validation_path.display().to_string()),
        )]);
        form_edit_require_valid(validate_form_for_edit(&validation_args, context, None))
    })?;
    Ok(report.cleanup_warnings)
}

pub(crate) fn form_edit_require_valid(outcome: AdapterOutcome) -> Result<(), String> {
    if outcome.ok {
        return Ok(());
    }
    let details = if outcome.errors.is_empty() {
        outcome
            .stdout
            .unwrap_or_else(|| "validation returned no diagnostics".to_string())
    } else {
        outcome.errors.join("; ")
    };
    Err(format!("form validation failed: {details}"))
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
        "form-info" => Some(Ok(analyze_form_info(
            args,
            context,
            &WorkspaceSupportStateReader::new(context),
        ))),
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
fn with_form_compile_after_parent_owner_probe_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<FormCompileAfterParentOwnerProbeHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            FORM_COMPILE_AFTER_PARENT_OWNER_PROBE_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous =
        FORM_COMPILE_AFTER_PARENT_OWNER_PROBE_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn run_form_compile_after_parent_owner_probe_hook(path: &Path) {
    if let Some(hook) =
        FORM_COMPILE_AFTER_PARENT_OWNER_PROBE_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook(path);
    }
}

pub(crate) struct FormInfoExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<FormInfoData>,
}

pub(crate) struct FormAddExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<FormAddData>,
}

pub(crate) struct FormRemoveExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<unica_execution_contract::mutation::MutationData>,
}

pub(crate) struct FormEditExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<FormEditData>,
}

struct FormEditSuccess {
    added_elements: Vec<FormEditAddedElement>,
    added_attributes: Vec<FormEditAddedAttribute>,
    added_commands: Vec<FormEditAddedCommand>,
    added_events: Vec<FormEditAddedEvent>,
    form_path: PathBuf,
    changed: bool,
    warnings: Vec<String>,
    removals: Vec<FormEditPlannedRemoval>,
}

#[cfg(test)]
#[path = "../../tests/platform/form_read_selector.rs"]
pub(crate) mod form_read_selector_bridge_tests;
#[cfg(test)]
#[path = "../../tests/platform/form_composition.rs"]
pub(crate) mod tests;
