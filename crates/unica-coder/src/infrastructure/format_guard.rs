use crate::application::operation_descriptors::{
    native_operation_descriptor, FormatGuardPolicy, FormatPathPolicy,
};
use crate::application::ports::FormatGuardCheck;
use crate::application::{AdapterOutcome, ToolHandler, ToolSpec};
use crate::domain::format_profile::{
    classify_root_version, FormatCompatibility, ACTIVE_FORMAT_PROFILE,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::cf::{
    cf_edit_format_dependency_paths, cf_init_planned_xml, cf_init_post_validation_dependency_paths,
    cf_read_format_dependency_paths,
};
use crate::infrastructure::native_operations::cfe::{
    cfe_borrow_format_dependency_inspection, cfe_borrow_resolve_path, cfe_init_planned_xml,
    cfe_registered_xml_dependency_paths,
};
use crate::infrastructure::native_operations::common::{
    find_support_config_dir, resolve_cf_edit_config_path, resolve_cf_read_config_path,
    resolve_cfe_validate_config_path, resolve_form_add_object_path, resolve_form_info_path,
    resolve_role_read_rights_path, resolve_subsystem_edit_xml, support_uuid_dependency_paths,
};
use crate::infrastructure::native_operations::dcs::{
    dcs_info_format_dependency_paths, resolve_dcs_validate_path,
};
use crate::infrastructure::native_operations::external::external_init_planned_xml_paths;
use crate::infrastructure::native_operations::form::{
    form_compile_infer_from_object_target, form_compile_normalize_from_object_output_label,
    form_parent_metadata_owner_candidate,
};
use crate::infrastructure::native_operations::help::resolve_help_object_dir_for_format_guard;
use crate::infrastructure::native_operations::interface::{
    interface_metadata_owner_path, resolve_interface_validate_path,
};
use crate::infrastructure::native_operations::meta::{
    meta_compile_format_dependency_paths, meta_remove_reference_xml_dependency_paths,
    meta_remove_subsystem_dependency_paths, meta_remove_type_plural,
    meta_validate_format_dependency_paths, resolve_meta_edit_object_path, resolve_meta_info_path,
};
use crate::infrastructure::native_operations::mxl::resolve_mxl_validate_path;
use crate::infrastructure::native_operations::role::role_read_format_dependency_paths;
use crate::infrastructure::native_operations::subsystem::{
    subsystem_edit_operations, subsystem_read_format_dependency_paths,
    subsystem_validation_format_dependency_paths,
};
use crate::infrastructure::native_operations::support::support_edit_reads_uuid_dependency;
use crate::infrastructure::native_operations::template::template_add_object_type_folders;
use crate::infrastructure::platform_xml_owner::{
    resolve_existing_platform_xml_owners_for_new_output, resolve_platform_xml_owners,
    resolve_platform_xml_owners_for_exact_root, PlatformXmlRootExpectation, DCS_ROOT,
    MANAGED_FORM_ROOT, MXL_ROOT,
};
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn evaluate_format_guard(
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<FormatGuardCheck, String> {
    let ToolHandler::NativeOperation { operation, .. } = spec.handler else {
        return Ok(FormatGuardCheck::Allow);
    };
    let Some(descriptor) = native_operation_descriptor(operation) else {
        return Ok(FormatGuardCheck::Allow);
    };
    let planned_new_outputs = create_only_planned_xml_paths(descriptor.operation, args, context);
    let mut targets = effective_format_paths_with_planned_outputs(
        descriptor,
        args,
        context,
        &planned_new_outputs,
    )?
    .into_iter()
    .map(|path| {
        let new_output = matches!(descriptor.format_guard, FormatGuardPolicy::NewDump)
            || planned_new_outputs.contains(&path);
        (path, new_output)
    })
    .collect::<Vec<_>>();
    deduplicate_targets(&mut targets);
    let mut owners = Vec::new();
    let mut owner_paths = HashSet::new();
    for (target, new_output) in targets {
        let expected_root =
            declared_output_root_expectation(descriptor.operation, args, context, &target);
        let resolved = if let Some(expected_root) = expected_root {
            resolve_platform_xml_owners_for_exact_root(&target, context, expected_root)
        } else if new_output {
            resolve_existing_platform_xml_owners_for_new_output(&target, context)
        } else {
            resolve_platform_xml_owners(&target, context)
        };
        let resolved_owners = match resolved {
            Ok(resolved_owners) => resolved_owners,
            Err(error) => {
                let warning = format!(
                    "Некорректный корневой файл формата выгрузки {}: {}",
                    error.path.display(),
                    error.message
                );
                let diagnostic = json!({
                    "code": "formatVersionInvalid",
                    "actualFormat": Value::Null,
                    "targetFormat": ACTIVE_FORMAT_PROFILE.export_format,
                    "targetPlatform": ACTIVE_FORMAT_PROFILE.platform_line,
                    "compatibility": "invalid",
                    "root": error.path.display().to_string(),
                });
                return Ok(format_check(spec, warning, diagnostic));
            }
        };
        for owner in resolved_owners {
            if owner_paths.insert(owner.path.clone()) {
                owners.push(owner);
            }
        }
    }
    let mut older = None;
    let mut newer = None;
    for owner in owners {
        let compatibility = match classify_root_version(owner.version.as_deref()) {
            Ok(compatibility) => compatibility,
            Err(error) => {
                let diagnostic = json!({
                    "code": error.code(),
                    "actualFormat": owner.version,
                    "targetFormat": ACTIVE_FORMAT_PROFILE.export_format,
                    "targetPlatform": ACTIVE_FORMAT_PROFILE.platform_line,
                    "compatibility": "invalid",
                    "root": owner.path.display().to_string(),
                    "ownerKind": owner.kind.label(),
                });
                return Ok(format_check(
                    spec,
                    format!(
                        "Некорректная версия формата выгрузки в {}",
                        owner.path.display()
                    ),
                    diagnostic,
                ));
            }
        };
        match compatibility {
            FormatCompatibility::Supported { .. } => {}
            FormatCompatibility::Older { .. } if older.is_none() => {
                older = Some((owner, compatibility));
            }
            FormatCompatibility::Newer { .. } if newer.is_none() => {
                newer = Some((owner, compatibility));
            }
            FormatCompatibility::Older { .. } | FormatCompatibility::Newer { .. } => {}
        }
    }
    if let Some((owner, compatibility)) = newer.or(older) {
        let actual = compatibility.actual().to_string();
        let (code, warning) = match compatibility {
            FormatCompatibility::Older { .. } => {
                let access = if spec.mutating {
                    "Изменение отменено."
                } else {
                    "Доступен только режим чтения."
                };
                let warning = format!(
                    "Формат выгрузки {actual} старше поддерживаемого {} для платформы 1С {}. {access} Чтобы редактировать исходники, явно перенесите выгрузку средствами платформы 1С 8.3.27: загрузите исходники и повторно выгрузите их. Unica не выполняет эту миграцию автоматически.",
                    ACTIVE_FORMAT_PROFILE.export_format, ACTIVE_FORMAT_PROFILE.platform_line
                );
                ("formatMigrationAvailable", warning)
            }
            FormatCompatibility::Newer { .. } => (
                "platformVersionUnsupported",
                format!(
                    "Формат выгрузки {actual} новее поддерживаемого {} для платформы 1С {}. Unica пока не поддерживает работу с этой выгрузкой. Поддержка платформы 1С 8.5 планируется в ближайших версиях.",
                    ACTIVE_FORMAT_PROFILE.export_format, ACTIVE_FORMAT_PROFILE.platform_line
                ),
            ),
            FormatCompatibility::Supported { .. } => unreachable!(),
        };
        let diagnostic = json!({
            "code": code,
            "actualFormat": actual,
            "targetFormat": ACTIVE_FORMAT_PROFILE.export_format,
            "targetPlatform": ACTIVE_FORMAT_PROFILE.platform_line,
            "compatibility": compatibility.label(),
            "root": owner.path.display().to_string(),
            "ownerKind": owner.kind.label(),
        });
        return Ok(format_check(spec, warning, diagnostic));
    }
    Ok(FormatGuardCheck::Allow)
}

fn declared_output_root_expectation(
    operation: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    target: &Path,
) -> Option<PlatformXmlRootExpectation> {
    let (output, expected_root) = match operation {
        "dcs-compile" => (output_path_arg(args, context), DCS_ROOT),
        "mxl-compile" => (output_path_arg(args, context), MXL_ROOT),
        "form-compile" => (
            form_compile_format_paths(args, context).into_iter().next(),
            MANAGED_FORM_ROOT,
        ),
        _ => return None,
    };
    output
        .filter(|output| output == target)
        .map(|_| expected_root)
}

fn output_path_arg(args: &Map<String, Value>, context: &WorkspaceContext) -> Option<PathBuf> {
    ["OutputPath", "outputPath"]
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
        .map(|path| absolutize(path, &context.cwd))
}

fn format_check(spec: ToolSpec, warning: String, diagnostic: Value) -> FormatGuardCheck {
    if !spec.mutating {
        return FormatGuardCheck::Warn {
            warning,
            diagnostic,
        };
    }
    FormatGuardCheck::Block {
        outcome: AdapterOutcome {
            ok: false,
            summary: format!("{} blocked by export format guard", spec.name),
            changes: Vec::new(),
            warnings: vec![warning.clone()],
            errors: vec![warning.clone()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some(format!("{warning}\n")),
            command: None,
        },
        diagnostic,
    }
}

#[cfg(test)]
fn effective_format_paths(
    descriptor: &crate::application::operation_descriptors::OperationDescriptor,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<Vec<PathBuf>, String> {
    let planned_new_outputs = create_only_planned_xml_paths(descriptor.operation, args, context);
    effective_format_paths_with_planned_outputs(descriptor, args, context, &planned_new_outputs)
}

fn effective_format_paths_with_planned_outputs(
    descriptor: &crate::application::operation_descriptors::OperationDescriptor,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    planned_new_outputs: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    let mut paths = if matches!(
        descriptor.operation,
        "cf-init" | "epf-init" | "erf-init" | "support-edit"
    ) {
        Vec::new()
    } else {
        match descriptor.format_path_policy {
            FormatPathPolicy::DeclaredArgs => descriptor
                .source_path_args
                .iter()
                .filter_map(|name| args.get(*name).and_then(Value::as_str))
                .map(|raw| absolutize(raw, &context.cwd))
                .collect(),
            FormatPathPolicy::HandlerResolved => {
                handler_resolved_format_paths(descriptor, args, context)
            }
            FormatPathPolicy::DefaultSrcObject => {
                let src = ["SrcDir", "srcDir"]
                    .iter()
                    .find_map(|name| args.get(*name).and_then(Value::as_str))
                    .unwrap_or("src");
                let object = ["ObjectName", "objectName", "ProcessorName", "processorName"]
                    .iter()
                    .find_map(|name| args.get(*name).and_then(Value::as_str));
                object
                    .map(|name| {
                        absolutize(src, &context.cwd)
                            .join(name)
                            .with_extension("xml")
                    })
                    .into_iter()
                    .collect()
            }
            FormatPathPolicy::FormCompile => form_compile_format_paths(args, context),
        }
    };
    add_operation_format_dependencies(descriptor.operation, args, context, &mut paths)?;
    if matches!(descriptor.operation, "epf-init" | "erf-init") {
        if let Some(output_dir) = planned_new_outputs
            .first()
            .and_then(|path| path.parent())
            .map(Path::to_path_buf)
        {
            paths.push(output_dir);
        }
    }
    paths.extend(planned_new_outputs.iter().cloned());
    deduplicate_paths(&mut paths);
    Ok(paths)
}

fn create_only_planned_xml_paths(
    operation: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Vec<PathBuf> {
    match operation {
        "cf-init" => {
            let planned = cf_init_planned_xml(args, context);
            vec![
                planned.configuration,
                planned.language,
                planned.client_application_interface,
            ]
        }
        "cfe-init" => {
            let planned = cfe_init_planned_xml(args, context);
            let mut paths = vec![planned.configuration, planned.language];
            paths.extend(planned.role);
            paths
        }
        "epf-init" | "erf-init" => {
            external_init_planned_xml_paths(operation, args, context).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

fn deduplicate_targets(targets: &mut Vec<(PathBuf, bool)>) {
    let mut deduplicated: Vec<(PathBuf, bool)> = Vec::with_capacity(targets.len());
    for (path, new_output) in targets.drain(..) {
        if let Some((_, existing_new_output)) = deduplicated
            .iter_mut()
            .find(|(existing, _)| *existing == path)
        {
            *existing_new_output |= new_output;
        } else {
            deduplicated.push((path, new_output));
        }
    }
    *targets = deduplicated;
}

fn add_operation_format_dependencies(
    operation: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    paths: &mut Vec<PathBuf>,
) -> Result<(), String> {
    match operation {
        "cf-edit" => {
            if let Ok(dependencies) = cf_edit_format_dependency_paths(args, context) {
                paths.extend(dependencies);
            }
        }
        "cf-init" => {
            let planned = cf_init_planned_xml(args, context);
            paths.extend(cf_init_post_validation_dependency_paths(&planned));
        }
        "cf-info" | "cf-validate" => {
            if let Ok(dependencies) = cf_read_format_dependency_paths(args, context, operation) {
                paths.extend(dependencies);
            }
        }
        "dcs-info" => {
            paths.extend(dcs_info_format_dependency_paths(args, context));
        }
        "role-info" | "role-validate" => {
            if let Ok(dependencies) = role_read_format_dependency_paths(args, context, operation) {
                paths.extend(dependencies);
            }
        }
        "support-edit" => add_support_edit_format_dependencies(args, context, paths),
        "cfe-borrow" => {
            let inspection = cfe_borrow_format_dependency_inspection(args, context);
            paths.extend(inspection.paths);
        }
        "cfe-validate" | "cfe-diff" => {
            add_cfe_read_format_dependencies(operation, args, context, paths)
        }
        "cfe-init" => add_cfe_init_format_dependencies(args, context, paths),
        "meta-compile" => {
            if let Ok(dependencies) = meta_compile_format_dependency_paths(args, context) {
                paths.extend(dependencies);
            }
        }
        "meta-validate" => {
            if let Ok(dependencies) = meta_validate_format_dependency_paths(args, context) {
                paths.extend(dependencies);
            }
        }
        "meta-remove" => add_meta_remove_format_dependencies(args, context, paths)?,
        "help-add" => add_help_format_dependencies(args, context, paths)?,
        "form-add" => add_form_add_format_dependencies(args, paths)?,
        "form-remove" => {
            add_named_child_tree_format_dependencies(args, paths, "Forms", "FormName")?
        }
        "interface-edit" => add_interface_format_dependencies(args, context, paths),
        "subsystem-info" | "subsystem-validate" => {
            if let Ok(dependencies) =
                subsystem_read_format_dependency_paths(args, context, operation)
            {
                paths.extend(dependencies);
            }
        }
        "subsystem-compile" => add_subsystem_compile_format_dependencies(args, context, paths)?,
        "subsystem-edit" => add_subsystem_edit_format_dependencies(args, context, paths)?,
        "template-add" => add_template_add_format_dependencies(args, context, paths)?,
        "template-remove" => {
            add_named_child_tree_format_dependencies(args, paths, "Templates", "TemplateName")?
        }
        "role-compile" => add_role_compile_format_dependencies(args, context, paths),
        _ => {}
    }
    Ok(())
}

fn add_cfe_read_format_dependencies(
    operation: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    paths: &mut Vec<PathBuf>,
) {
    // CFE read tools share the registered extension source graph as their
    // compatibility boundary. This deliberately includes related registered
    // wrappers/languages even when one diff mode does not open their bytes,
    // while excluding every unregistered neighboring XML file. For cfe-diff,
    // the declared ConfigPath remains a separate boundary because Mode B reads
    // base modules and Configuration.xml defines that dump's format.
    let config_path = match operation {
        "cfe-validate" => resolve_cfe_validate_config_path(args, context).ok(),
        "cfe-diff" => cfe_borrow_resolve_path(
            args,
            context,
            &["extensionPath", "ExtensionPath"],
            "extension",
        )
        .ok(),
        _ => None,
    };
    if let Some(config_path) = config_path {
        if let Ok(dependencies) = cfe_registered_xml_dependency_paths(&config_path) {
            paths.extend(dependencies);
        }
    }
}

fn add_support_edit_format_dependencies(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    paths: &mut Vec<PathBuf>,
) {
    let Some(raw) = ["Path", "path", "TargetPath", "targetPath"]
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
    else {
        return;
    };
    let target = absolutize(raw, &context.cwd);
    if let Some(config_dir) = find_support_config_dir(&target) {
        paths.push(config_dir.join("Configuration.xml"));
    }
    if support_edit_reads_uuid_dependency(args) {
        paths.extend(support_uuid_dependency_paths(&target));
    }
}

fn add_cfe_init_format_dependencies(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    paths: &mut Vec<PathBuf>,
) {
    let Some(base_config) =
        cfe_borrow_resolve_path(args, context, &["configPath", "ConfigPath"], "config").ok()
    else {
        return;
    };
    paths.push(base_config.clone());
    if let Some(base_dir) = base_config.parent() {
        paths.push(base_dir.join("Languages").join("Русский.xml"));
    }
}

fn add_meta_remove_format_dependencies(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    paths: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let Some(config_dir) = ["configDir", "ConfigDir"]
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
        .map(|raw| absolutize(raw, &context.cwd))
    else {
        return Ok(());
    };
    paths.push(config_dir.join("Configuration.xml"));
    let Some(object) = ["object", "Object"]
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
    else {
        return Ok(());
    };
    let Some((object_type, object_name)) = object.split_once('.') else {
        return Ok(());
    };
    if !is_safe_single_path_component(object_name) {
        return Ok(());
    }
    let Some(type_dir) = meta_remove_type_plural(object_type) else {
        return Ok(());
    };
    let object_base = config_dir.join(type_dir).join(object_name);
    let object_xml = object_base.with_extension("xml");
    let has_xml = object_xml.is_file();
    let has_dir = object_base.is_dir();
    paths.push(object_xml.clone());
    collect_existing_xml_tree(&object_base, paths)?;
    paths.extend(meta_remove_reference_xml_dependency_paths(
        &config_dir,
        &object_xml,
        &object_base,
        has_xml,
        has_dir,
    )?);
    let subsystem_dir = config_dir.join("Subsystems");
    if subsystem_dir.is_dir() {
        paths.extend(meta_remove_subsystem_dependency_paths(
            &subsystem_dir,
            object,
        )?);
    }
    Ok(())
}

fn add_help_format_dependencies(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    paths: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let Some(object_name) = ["objectName", "ObjectName", "processorName", "ProcessorName"]
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
    else {
        return Ok(());
    };
    let src_dir = ["srcDir", "SrcDir"]
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
        .unwrap_or("src");
    let object_dir =
        resolve_help_object_dir_for_format_guard(&absolutize(src_dir, &context.cwd), object_name)?;
    paths.push(object_dir.with_extension("xml"));
    paths.push(object_dir.join("Ext").join("Help.xml"));
    collect_direct_xml_files(&object_dir.join("Forms"), paths)
}

fn add_form_add_format_dependencies(
    args: &Map<String, Value>,
    paths: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let Some(owner) = paths.first().cloned() else {
        return Ok(());
    };
    let Some(form_name) = ["formName", "FormName"]
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
    else {
        return Ok(());
    };
    if !is_safe_single_path_component(form_name) {
        return Ok(());
    }
    let form_base = owner.with_extension("").join("Forms").join(form_name);
    paths.push(form_base.with_extension("xml"));
    paths.push(form_base.join("Ext").join("Form.xml"));
    Ok(())
}

fn add_named_child_tree_format_dependencies(
    args: &Map<String, Value>,
    paths: &mut Vec<PathBuf>,
    collection: &str,
    name_argument: &str,
) -> Result<(), String> {
    let Some(owner_path) = paths.first().cloned() else {
        return Ok(());
    };
    let aliases = match name_argument {
        "FormName" => ["formName", "FormName"],
        "TemplateName" => ["templateName", "TemplateName"],
        _ => unreachable!("known named child argument"),
    };
    let Some(child_name) = aliases
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
    else {
        return Ok(());
    };
    if !is_safe_single_path_component(child_name) {
        return Ok(());
    }
    let child_base = owner_path
        .with_extension("")
        .join(collection)
        .join(child_name);
    paths.push(child_base.with_extension("xml"));
    collect_existing_xml_tree(&child_base, paths)
}

fn add_interface_format_dependencies(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    paths: &mut Vec<PathBuf>,
) {
    let Some(raw) = ["CIPath", "ciPath", "path", "Path"]
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
    else {
        return;
    };
    if let Ok(owner) = interface_metadata_owner_path(&absolutize(raw, &context.cwd)) {
        paths.push(owner);
    }
}

fn add_subsystem_edit_format_dependencies(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    paths: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let Some(target) = paths.first().cloned() else {
        return Ok(());
    };
    let mut validation_descriptors = vec![target.clone()];
    let source = match fs::read_to_string(&target) {
        Ok(source) => source,
        Err(_) => return Ok(()),
    };
    let document = match Document::parse(source.trim_start_matches('\u{feff}')) {
        Ok(document) => document,
        Err(_) => return Ok(()),
    };
    let mut registered = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "ChildObjects")
        .into_iter()
        .flat_map(|node| node.children())
        .filter(|node| node.is_element() && node.tag_name().name() == "Subsystem")
        .filter_map(|node| node.text())
        .map(str::to_string)
        .collect::<HashSet<_>>();
    let definition_file = ["definitionFile", "DefinitionFile"]
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
        .map(PathBuf::from);
    let operation = ["operation", "Operation"]
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str));
    let Ok(operations) = subsystem_edit_operations(args, &context.cwd, operation, definition_file)
    else {
        return Ok(());
    };
    let mut newly_added = HashSet::new();
    for (operation, value) in operations {
        let Some(child_name) = value.as_str() else {
            continue;
        };
        if !is_safe_single_path_component(child_name) {
            continue;
        }
        match operation.as_str() {
            "add-child" if registered.insert(child_name.to_string()) => {
                newly_added.insert(child_name.to_string());
            }
            "remove-child" if registered.remove(child_name) => {
                newly_added.remove(child_name);
            }
            _ => {}
        }
    }
    let parent = target.parent().unwrap_or(context.cwd.as_path());
    let Some(parent_name) = target.file_stem().and_then(|name| name.to_str()) else {
        return Ok(());
    };
    for child_name in newly_added {
        let child = parent
            .join(parent_name)
            .join("Subsystems")
            .join(child_name)
            .with_extension("xml");
        paths.push(child.clone());
        validation_descriptors.push(child);
    }
    let descriptor_refs = validation_descriptors
        .iter()
        .map(PathBuf::as_path)
        .collect::<Vec<_>>();
    paths.extend(subsystem_validation_format_dependency_paths(
        &descriptor_refs,
    ));
    Ok(())
}

fn add_subsystem_compile_format_dependencies(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    paths: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let Some(definition) = subsystem_compile_definition(args, context) else {
        return Ok(());
    };
    let Some(name) = definition.get("name").and_then(Value::as_str) else {
        return Ok(());
    };
    if !is_safe_single_path_component(name) {
        return Ok(());
    }
    let Some(output_dir) = ["outputDir", "OutputDir"]
        .iter()
        .find_map(|argument| args.get(*argument).and_then(Value::as_str))
        .map(|raw| absolutize(raw, &context.cwd))
    else {
        return Ok(());
    };
    let parent = ["parent", "Parent"]
        .iter()
        .find_map(|argument| args.get(*argument).and_then(Value::as_str))
        .map(|raw| absolutize(raw, &context.cwd));
    let mut validation_descriptors = Vec::new();
    let subsystems_dir = if let Some(parent) = parent.as_ref() {
        paths.push(parent.clone());
        validation_descriptors.push(parent.clone());
        let parent_dir = parent.parent().unwrap_or(output_dir.as_path());
        let parent_name = parent.file_stem().and_then(|value| value.to_str());
        let Some(parent_name) = parent_name else {
            return Ok(());
        };
        parent_dir.join(parent_name).join("Subsystems")
    } else {
        let configuration = output_dir.join("Configuration.xml");
        paths.push(configuration.clone());
        validation_descriptors.push(configuration);
        output_dir.join("Subsystems")
    };
    let target = subsystems_dir.join(name);
    let target = target.with_extension("xml");
    paths.push(target.clone());
    validation_descriptors.push(target.clone());
    if let Some(children) = definition.get("children").and_then(Value::as_array) {
        for child in children.iter().filter_map(Value::as_str) {
            if is_safe_single_path_component(child) {
                let child = target
                    .with_extension("")
                    .join("Subsystems")
                    .join(child)
                    .with_extension("xml");
                paths.push(child.clone());
                validation_descriptors.push(child);
            }
        }
    }
    let descriptor_refs = validation_descriptors
        .iter()
        .map(PathBuf::as_path)
        .collect::<Vec<_>>();
    paths.extend(subsystem_validation_format_dependency_paths(
        &descriptor_refs,
    ));
    Ok(())
}

fn subsystem_compile_definition(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Value> {
    let text = if let Some(raw) = ["definitionFile", "DefinitionFile"]
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
    {
        fs::read_to_string(absolutize(raw, &context.cwd)).ok()?
    } else {
        ["value", "Value"]
            .iter()
            .find_map(|name| args.get(*name).and_then(Value::as_str))?
            .to_string()
    };
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

fn add_template_add_format_dependencies(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    paths: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let Some(object_name) = ["objectName", "ObjectName", "processorName", "ProcessorName"]
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
    else {
        return Ok(());
    };
    if !is_safe_single_path_component(object_name) {
        return Ok(());
    }
    let src_dir = ["srcDir", "SrcDir"]
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
        .unwrap_or("src");
    let src_dir = absolutize(src_dir, &context.cwd);
    let direct = src_dir.join(object_name).with_extension("xml");
    if direct.is_file() {
        add_template_target_paths(&direct, args, paths);
        return Ok(());
    }
    for folder in template_add_object_type_folders() {
        let candidate = src_dir.join(folder).join(object_name).with_extension("xml");
        if candidate.is_file() {
            paths.push(candidate.clone());
            add_template_target_paths(&candidate, args, paths);
        }
    }
    Ok(())
}

fn add_template_target_paths(owner: &Path, args: &Map<String, Value>, paths: &mut Vec<PathBuf>) {
    let Some(template_name) = ["templateName", "TemplateName"]
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
    else {
        return;
    };
    if !is_safe_single_path_component(template_name) {
        return;
    }
    let target = owner
        .with_extension("")
        .join("Templates")
        .join(template_name);
    paths.push(target.with_extension("xml"));
    paths.push(target.join("Ext").join("Template.xml"));
}

fn add_role_compile_format_dependencies(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    paths: &mut Vec<PathBuf>,
) {
    let Some(output_dir) = ["outputDir", "OutputDir"]
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
        .map(|raw| absolutize(raw, &context.cwd))
    else {
        return;
    };
    let config_dir = if output_dir.file_name().and_then(|name| name.to_str()) == Some("Roles") {
        output_dir.parent().unwrap_or(context.cwd.as_path())
    } else {
        output_dir.as_path()
    };
    paths.push(config_dir.join("Configuration.xml"));
    let Some(definition) = load_json_file_argument(args, context, &["jsonPath", "JsonPath"]) else {
        return;
    };
    let Some(role_name) = definition.get("name").and_then(Value::as_str) else {
        return;
    };
    if !is_safe_single_path_component(role_name) {
        return;
    }
    let roles_dir = if output_dir.file_name().and_then(|name| name.to_str()) == Some("Roles") {
        output_dir
    } else {
        output_dir.join("Roles")
    };
    let role_base = roles_dir.join(role_name);
    paths.push(role_base.with_extension("xml"));
    paths.push(role_base.join("Ext").join("Rights.xml"));
}

fn load_json_file_argument(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    names: &[&str],
) -> Option<Value> {
    let raw = names
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))?;
    let text = fs::read_to_string(absolutize(raw, &context.cwd)).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

fn is_safe_single_path_component(value: &str) -> bool {
    let mut components = Path::new(value).components();
    matches!(components.next(), Some(std::path::Component::Normal(component)) if component == value)
        && components.next().is_none()
}

fn collect_existing_xml_tree(root: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!("failed to inspect {}: {error}", root.display()));
        }
    };
    if metadata.file_type().is_symlink() {
        paths.push(root.to_path_buf());
        return Ok(());
    }
    if metadata.is_file() {
        if root
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
        {
            paths.push(root.to_path_buf());
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        return Ok(());
    }
    let mut entries = fs::read_dir(root)
        .map_err(|error| format!("failed to inspect {}: {error}", root.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to inspect entry in {}: {error}", root.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        collect_existing_xml_tree(&entry.path(), paths)?;
    }
    Ok(())
}

fn collect_direct_xml_files(root: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!("failed to inspect {}: {error}", root.display()));
        }
    };
    if metadata.file_type().is_symlink() {
        paths.push(root.to_path_buf());
        return Ok(());
    }
    if !metadata.is_dir() {
        return Ok(());
    }
    let mut entries = fs::read_dir(root)
        .map_err(|error| format!("failed to inspect {}: {error}", root.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to inspect entry in {}: {error}", root.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        let is_xml = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"));
        if is_xml && (metadata.file_type().is_symlink() || metadata.is_file()) {
            paths.push(path);
        }
    }
    Ok(())
}

fn deduplicate_paths(paths: &mut Vec<PathBuf>) {
    let mut seen = HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
}

fn handler_resolved_format_paths(
    descriptor: &crate::application::operation_descriptors::OperationDescriptor,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Vec<PathBuf> {
    let raw = descriptor
        .source_path_args
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str));
    let fallback = raw.map(|path| absolutize(path, &context.cwd));
    let resolved =
        match descriptor.operation {
            "cf-edit" => resolve_cf_edit_config_path(args, context).ok(),
            "cf-info" | "cf-validate" => resolve_cf_read_config_path(args, context).ok(),
            "cfe-validate" => resolve_cfe_validate_config_path(args, context).ok(),
            "meta-edit" => raw
                .and_then(|path| resolve_meta_edit_object_path(Path::new(path), &context.cwd).ok()),
            "meta-info" | "meta-validate" => {
                raw.and_then(|path| resolve_meta_info_path(absolutize(path, &context.cwd)).ok())
            }
            "form-add" => raw
                .and_then(|path| resolve_form_add_object_path(absolutize(path, &context.cwd)).ok()),
            "form-info" | "form-validate" => {
                raw.map(|path| resolve_form_info_path(absolutize(path, &context.cwd)))
            }
            "interface-validate" => resolve_interface_validate_path(args, context).ok(),
            "subsystem-edit" => {
                raw.and_then(|path| resolve_subsystem_edit_xml(absolutize(path, &context.cwd)).ok())
            }
            "dcs-edit" | "dcs-validate" => resolve_dcs_validate_path(args, context).ok(),
            "mxl-validate" => resolve_mxl_validate_path(args, context).ok(),
            "role-info" | "role-validate" => resolve_role_read_rights_path(args, context).ok(),
            _ => None,
        };
    resolved.or(fallback).into_iter().collect()
}

fn form_compile_format_paths(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Vec<PathBuf> {
    let Some(raw_output) = ["OutputPath", "outputPath"]
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
    else {
        return Vec::new();
    };
    let from_object = ["FromObject", "fromObject"]
        .iter()
        .any(|name| args.get(*name).and_then(Value::as_bool).unwrap_or(false));
    let output_label = if from_object {
        form_compile_normalize_from_object_output_label(raw_output)
            .map(|(path, _)| path)
            .unwrap_or_else(|| raw_output.to_string())
    } else {
        raw_output.to_string()
    };
    let output = absolutize(&output_label, &context.cwd);
    let mut paths = vec![output.clone()];
    if let Ok(Some(parent)) = form_parent_metadata_owner_candidate(&output) {
        if !paths.contains(&parent) {
            paths.push(parent);
        }
    }
    if from_object {
        if let Some(raw_object) = ["ObjectPath", "objectPath"]
            .iter()
            .find_map(|name| args.get(*name).and_then(Value::as_str))
        {
            let mut object = absolutize(raw_object, &context.cwd);
            if object.extension().is_none() {
                object.set_extension("xml");
            }
            paths.push(object);
        } else if let (Some(inferred), _) = form_compile_infer_from_object_target(&output, context)
        {
            paths.push(inferred);
        }
    }
    paths
}

fn absolutize(raw: &str, cwd: &Path) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::{effective_format_paths, evaluate_format_guard};
    use crate::application::operation_descriptors::native_operation_descriptor;
    use crate::application::ports::FormatGuardCheck;
    use crate::application::tools;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::native_operations::cfe::cfe_borrow_format_dependency_inspection;
    use crate::infrastructure::native_operations::dcs::analyze_dcs_info;
    use crate::infrastructure::source_roots::normalize_path_identity;
    use serde_json::{Map, Value};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_ROOT_NONCE: AtomicU64 = AtomicU64::new(0);

    fn test_root(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "unica-format-guard-{label}-{}-{}",
            std::process::id(),
            TEST_ROOT_NONCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn normalized_path(path: &std::path::Path) -> std::path::PathBuf {
        normalize_path_identity(path).expect("test path identity must normalize")
    }

    fn context(root: &std::path::Path) -> WorkspaceContext {
        WorkspaceContext {
            cwd: root.to_path_buf(),
            workspace_root: root.to_path_buf(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    fn config(root: &std::path::Path, version: Option<&str>) -> std::path::PathBuf {
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let version = version
            .map(|value| format!(r#" version="{value}""#))
            .unwrap_or_default();
        std::fs::write(
            src.join("Configuration.xml"),
            format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"{version}><Configuration/></MetaDataObject>"#),
        )
        .unwrap();
        src.join("Configuration.xml")
    }

    struct CfeReadGraph {
        extension: std::path::PathBuf,
        object: std::path::PathBuf,
        language: std::path::PathBuf,
        form_wrapper: std::path::PathBuf,
        form_xml: std::path::PathBuf,
        unregistered: std::path::PathBuf,
    }

    fn cfe_read_graph(
        root: &std::path::Path,
        object_version: &str,
        language_version: &str,
        form_wrapper_version: &str,
        form_version: &str,
        unregistered_version: &str,
    ) -> CfeReadGraph {
        let extension = root.join("extension");
        let object = extension.join("Catalogs/Registered.xml");
        let language = extension.join("Languages/Russian.xml");
        let form_wrapper = extension.join("Catalogs/Registered/Forms/Main.xml");
        let form_xml = extension.join("Catalogs/Registered/Forms/Main/Ext/Form.xml");
        let unregistered = extension.join("Catalogs/Unregistered.xml");
        for path in [
            extension.join("Configuration.xml"),
            object.clone(),
            language.clone(),
            form_wrapper.clone(),
            form_xml.clone(),
            unregistered.clone(),
        ] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        std::fs::write(
            extension.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
<Configuration>
<Properties><Name>Extension</Name><NamePrefix>Ext</NamePrefix><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose></Properties>
<ChildObjects><Language>Russian</Language><Catalog>Registered</Catalog></ChildObjects>
</Configuration>
</MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &object,
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{object_version}">
<Catalog>
<Properties><Name>Registered</Name><ObjectBelonging>Adopted</ObjectBelonging></Properties>
<ChildObjects><Form>Main</Form></ChildObjects>
</Catalog>
</MetaDataObject>"#
            ),
        )
        .unwrap();
        std::fs::write(
            &language,
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{language_version}"><Language/></MetaDataObject>"#
            ),
        )
        .unwrap();
        std::fs::write(
            &form_wrapper,
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{form_wrapper_version}"><Form/></MetaDataObject>"#
            ),
        )
        .unwrap();
        std::fs::write(
            &form_xml,
            format!(r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="{form_version}"/>"#),
        )
        .unwrap();
        std::fs::write(
            &unregistered,
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{unregistered_version}"><Catalog/></MetaDataObject>"#
            ),
        )
        .unwrap();
        CfeReadGraph {
            extension,
            object,
            language,
            form_wrapper,
            form_xml,
            unregistered,
        }
    }

    fn spec(name: &str) -> crate::application::ToolSpec {
        tools().into_iter().find(|tool| tool.name == name).unwrap()
    }

    fn assert_platform_reexport_warning(warning: &str) {
        assert!(warning.contains("платформы 1С 8.3.27"), "{warning}");
        assert!(warning.contains("повторно выгруз"), "{warning}");
        assert!(
            warning.contains("не выполняет эту миграцию автоматически"),
            "{warning}"
        );
        assert!(!warning.contains("migrate_format"), "{warning}");
        assert!(!warning.contains("unica."), "{warning}");
    }

    fn external_source_set(
        root: &std::path::Path,
        kind: &str,
        dir: &str,
        artifact: &str,
        version: &str,
    ) -> std::path::PathBuf {
        std::fs::write(
            root.join("v8project.yaml"),
            format!(
                "format: DESIGNER\nsource-set:\n  - name: external\n    type: {kind}\n    path: {dir}\n"
            ),
        )
        .unwrap();
        let source_root = root.join(dir);
        std::fs::create_dir_all(source_root.join(artifact)).unwrap();
        let tag = if kind == "EXTERNAL_REPORTS" {
            "ExternalReport"
        } else {
            "ExternalDataProcessor"
        };
        std::fs::write(
            source_root.join(format!("{artifact}.xml")),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}"><{tag}/></MetaDataObject>"#
            ),
        )
        .unwrap();
        source_root
    }

    #[test]
    fn cfe_validate_warns_for_newer_registered_form_wrapper() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-cfe-validate-form-wrapper-{}",
            std::process::id()
        ));
        let graph = cfe_read_graph(&root, "2.20", "2.20", "2.21", "2.20", "2.21");
        let args = Map::from_iter([(
            "ExtensionPath".to_string(),
            Value::String(graph.extension.display().to_string()),
        )]);

        let check =
            evaluate_format_guard(spec("unica.cfe.validate"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Warn { diagnostic, .. } = check else {
            panic!("full CFE validation must warn for a newer registered form wrapper");
        };
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        assert_eq!(
            diagnostic["root"],
            normalized_path(&graph.form_wrapper).display().to_string()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cfe_diff_mode_a_warns_for_newer_registered_form_content() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-cfe-diff-mode-a-form-{}",
            std::process::id()
        ));
        let base = config(&root, Some("2.20"));
        let graph = cfe_read_graph(&root, "2.20", "2.20", "2.20", "2.21", "2.21");
        let args = Map::from_iter([
            (
                "ExtensionPath".to_string(),
                Value::String(graph.extension.display().to_string()),
            ),
            (
                "ConfigPath".to_string(),
                Value::String(base.parent().unwrap().display().to_string()),
            ),
            ("Mode".to_string(), Value::String("A".to_string())),
        ]);

        let check = evaluate_format_guard(spec("unica.cfe.diff"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Warn { diagnostic, .. } = check else {
            panic!("CFE diff mode A must warn for registered Form.xml that it reads");
        };
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        assert_eq!(
            diagnostic["root"],
            normalized_path(&graph.form_xml).display().to_string()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cfe_diff_ignores_only_newer_unregistered_neighbor() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-cfe-diff-unregistered-{}",
            std::process::id()
        ));
        let base = config(&root, Some("2.20"));
        let graph = cfe_read_graph(&root, "2.20", "2.20", "2.20", "2.20", "2.21");
        let args = Map::from_iter([
            (
                "ExtensionPath".to_string(),
                Value::String(graph.extension.display().to_string()),
            ),
            (
                "ConfigPath".to_string(),
                Value::String(base.parent().unwrap().display().to_string()),
            ),
            ("Mode".to_string(), Value::String("A".to_string())),
        ]);

        let descriptor = native_operation_descriptor("cfe-diff").unwrap();
        let dependencies = effective_format_paths(descriptor, &args, &context(&root)).unwrap();
        assert!(dependencies.contains(&graph.object), "{dependencies:?}");
        assert!(dependencies.contains(&graph.language), "{dependencies:?}");
        assert!(
            dependencies.contains(&graph.form_wrapper),
            "{dependencies:?}"
        );
        assert!(dependencies.contains(&graph.form_xml), "{dependencies:?}");
        assert!(
            !dependencies.contains(&graph.unregistered),
            "{dependencies:?}"
        );
        assert!(matches!(
            evaluate_format_guard(spec("unica.cfe.diff"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Allow
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cfe_diff_mode_b_warns_for_newer_registered_form_content() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-cfe-diff-mode-b-form-{}",
            std::process::id()
        ));
        let base = config(&root, Some("2.20"));
        let graph = cfe_read_graph(&root, "2.20", "2.20", "2.20", "2.21", "2.21");
        let args = Map::from_iter([
            (
                "ExtensionPath".to_string(),
                Value::String(graph.extension.display().to_string()),
            ),
            (
                "ConfigPath".to_string(),
                Value::String(base.parent().unwrap().display().to_string()),
            ),
            ("Mode".to_string(), Value::String("B".to_string())),
        ]);

        let descriptor = native_operation_descriptor("cfe-diff").unwrap();
        let dependencies = effective_format_paths(descriptor, &args, &context(&root)).unwrap();
        assert!(dependencies.contains(&graph.object), "{dependencies:?}");
        assert!(dependencies.contains(&graph.form_xml), "{dependencies:?}");
        assert!(
            !dependencies.contains(&graph.unregistered),
            "{dependencies:?}"
        );
        let check = evaluate_format_guard(spec("unica.cfe.diff"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Warn { diagnostic, .. } = check else {
            panic!("CFE diff mode B must guard the full registered source graph");
        };
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        assert_eq!(
            diagnostic["root"],
            normalized_path(&graph.form_xml).display().to_string()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn older_dump_blocks_mutation_and_recommends_platform_reexport() {
        let root = test_root("old");
        let path = config(&root, Some("2.19"));
        let before = std::fs::read(&path).unwrap();
        let mut args = Map::new();
        args.insert(
            "ConfigPath".into(),
            Value::String(path.display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.cf.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block {
            outcome,
            diagnostic,
        } = check
        else {
            panic!("older mutation must be blocked");
        };
        assert!(!outcome.ok);
        assert_eq!(diagnostic["code"], "formatMigrationAvailable");
        assert_eq!(diagnostic["actualFormat"], "2.19");
        let warning = outcome.warnings.join("\n");
        assert_platform_reexport_warning(&warning);
        assert!(warning.contains("Изменение отменено."), "{warning}");
        assert!(
            !warning.contains("Доступен только режим чтения."),
            "{warning}"
        );
        assert_eq!(std::fs::read(path).unwrap(), before);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn version_owning_target_cannot_hide_behind_supported_source_set_owner() {
        let root = test_root("target-version");
        config(&root, Some("2.20"));
        let form = root.join("src/Catalogs/Items/Forms/Item/Ext/Form.xml");
        std::fs::create_dir_all(form.parent().unwrap()).unwrap();
        std::fs::write(
            &form,
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.21"/>"#,
        )
        .unwrap();
        let before = std::fs::read(&form).unwrap();
        let mut args = Map::new();
        args.insert("FormPath".into(), Value::String(form.display().to_string()));

        let check = evaluate_format_guard(spec("unica.form.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block {
            outcome,
            diagnostic,
        } = check
        else {
            panic!("a newer target root inside a supported source set must block mutation");
        };

        assert!(!outcome.ok);
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        assert_eq!(
            diagnostic["root"],
            normalized_path(&form).display().to_string()
        );
        assert_eq!(std::fs::read(&form).unwrap(), before);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn support_edit_public_guard_blocks_newer_nearest_uuid_descriptor_before_handler() {
        let root = test_root("support-nearest");
        config(&root, Some("2.20"));
        let bin = root.join("src/Ext/ParentConfigurations.bin");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, b"support-bin-preimage").unwrap();
        let descriptor = root.join("src/Catalogs/Items.xml");
        let target = root.join("src/Catalogs/Items/Ext/ObjectModule.bsl");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(
            &descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Catalog uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"/></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(&target, "Процедура Тест()\nКонецПроцедуры").unwrap();
        let bin_before = std::fs::read(&bin).unwrap();
        let args = Map::from_iter([
            (
                "Path".to_string(),
                Value::String(target.display().to_string()),
            ),
            ("Set".to_string(), Value::String("editable".to_string())),
        ]);

        let check =
            evaluate_format_guard(spec("unica.support.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block {
            outcome,
            diagnostic,
        } = check
        else {
            panic!("support-edit must be refused by public preflight before the handler");
        };

        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        assert_eq!(
            diagnostic["root"],
            normalized_path(&descriptor).display().to_string()
        );
        assert!(outcome.summary.contains("export format guard"));
        assert_eq!(std::fs::read(&bin).unwrap(), bin_before);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cf_init_public_guard_blocks_newer_existing_post_validation_dependency() {
        let root = test_root("cf-init-home-page");
        let home_page = root.join("src/Ext/HomePageWorkArea.xml");
        std::fs::create_dir_all(home_page.parent().unwrap()).unwrap();
        std::fs::write(
            &home_page,
            r#"<HomePageWorkArea xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.21"/>"#,
        )
        .unwrap();
        let args = Map::from_iter([
            ("Name".to_string(), Value::String("Demo".to_string())),
            ("OutputDir".to_string(), Value::String("src".to_string())),
        ]);

        let check = evaluate_format_guard(spec("unica.cf.init"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("cf.init must authorize XML read by its post-validator before writing");
        };

        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        assert_eq!(
            diagnostic["root"],
            normalized_path(&home_page).display().to_string()
        );
        assert!(!root.join("src/Configuration.xml").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn support_edit_public_guard_blocks_newer_uuid_probe_without_uuid() {
        let root = test_root("support-probe");
        config(&root, Some("2.20"));
        let bin = root.join("src/Ext/ParentConfigurations.bin");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, b"support-bin-preimage").unwrap();
        let descriptor = root.join("src/Catalogs/Items.xml");
        let target = root.join("src/Catalogs/Items/Ext/ObjectModule.bsl");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(
            &descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Catalog/></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(&target, "Процедура Тест()\nКонецПроцедуры").unwrap();
        let bin_before = std::fs::read(&bin).unwrap();
        let args = Map::from_iter([
            (
                "Path".to_string(),
                Value::String(target.display().to_string()),
            ),
            ("Set".to_string(), Value::String("editable".to_string())),
        ]);

        let check =
            evaluate_format_guard(spec("unica.support.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("every XML read used for UUID resolution must be format-authorized");
        };

        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        assert_eq!(
            diagnostic["root"],
            normalized_path(&descriptor).display().to_string()
        );
        assert_eq!(std::fs::read(&bin).unwrap(), bin_before);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn support_edit_capability_does_not_guard_an_unread_uuid_descriptor() {
        let root = test_root("support-capability");
        config(&root, Some("2.20"));
        let bin = root.join("src/Ext/ParentConfigurations.bin");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, b"support-bin-preimage").unwrap();
        let descriptor = root.join("src/Catalogs/Items.xml");
        let target = root.join("src/Catalogs/Items/Ext/ObjectModule.bsl");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(
            &descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Catalog uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"/></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(&target, "Процедура Тест()\nКонецПроцедуры").unwrap();
        let args = Map::from_iter([
            (
                "Path".to_string(),
                Value::String(target.display().to_string()),
            ),
            ("Capability".to_string(), Value::String("on".to_string())),
        ]);

        assert!(matches!(
            evaluate_format_guard(spec("unica.support.edit"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Allow
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn support_edit_capability_does_not_guard_a_direct_unread_object_xml() {
        let root = test_root("support-capability-direct");
        config(&root, Some("2.20"));
        let bin = root.join("src/Ext/ParentConfigurations.bin");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, b"support-bin-preimage").unwrap();
        let descriptor = root.join("src/Catalogs/Items.xml");
        std::fs::create_dir_all(descriptor.parent().unwrap()).unwrap();
        std::fs::write(
            &descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Catalog uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"/></MetaDataObject>"#,
        )
        .unwrap();
        let args = Map::from_iter([
            (
                "Path".to_string(),
                Value::String(descriptor.display().to_string()),
            ),
            ("Capability".to_string(), Value::String("on".to_string())),
        ]);

        assert!(matches!(
            evaluate_format_guard(spec("unica.support.edit"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Allow
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn support_edit_public_guard_ignores_newer_unrelated_uuid_descriptor() {
        let root = test_root("support-unrelated");
        config(&root, Some("2.20"));
        let bin = root.join("src/Ext/ParentConfigurations.bin");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, b"support-bin-preimage").unwrap();
        let descriptor = root.join("src/Catalogs/Items.xml");
        let unrelated = root.join("src/Catalogs/Unrelated.xml");
        let target = root.join("src/Catalogs/Items/Ext/ObjectModule.bsl");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(
            &descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"/></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &unrelated,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Catalog uuid="cccccccc-cccc-cccc-cccc-cccccccccccc"/></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(&target, "Процедура Тест()\nКонецПроцедуры").unwrap();
        let bin_before = std::fs::read(&bin).unwrap();
        let args = Map::from_iter([
            (
                "Path".to_string(),
                Value::String(target.display().to_string()),
            ),
            ("Set".to_string(), Value::String("editable".to_string())),
        ]);

        let check =
            evaluate_format_guard(spec("unica.support.edit"), &args, &context(&root)).unwrap();

        assert!(matches!(check, FormatGuardCheck::Allow));
        assert_eq!(std::fs::read(&bin).unwrap(), bin_before);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn content_tools_preflight_their_exact_metadata_wrappers() {
        let root = test_root("content-wrapper");
        config(&root, Some("2.20"));
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let form_wrapper = root.join("src/Catalogs/Goods/Forms/Main.xml");
        let form_content = root.join("src/Catalogs/Goods/Forms/Main/Ext/Form.xml");
        let template_wrapper = root.join("src/Reports/Sales/Templates/Main.xml");
        let template_content = root.join("src/Reports/Sales/Templates/Main/Ext/Template.xml");
        let mxl_wrapper = root.join("src/Reports/Spreadsheet/Templates/Main.xml");
        let mxl_content = root.join("src/Reports/Spreadsheet/Templates/Main/Ext/Template.xml");
        let role_wrapper = root.join("src/Roles/Reader.xml");
        let rights_content = root.join("src/Roles/Reader/Ext/Rights.xml");
        for path in [
            &form_content,
            &template_content,
            &mxl_content,
            &rights_content,
        ] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        for (path, child) in [
            (&form_wrapper, "Form"),
            (&template_wrapper, "Template"),
            (&mxl_wrapper, "Template"),
            (&role_wrapper, "Role"),
        ] {
            std::fs::write(
                path,
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><{child}/></MetaDataObject>"#
                ),
            )
            .unwrap();
        }
        std::fs::write(
            &form_content,
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#,
        )
        .unwrap();
        std::fs::write(
            &template_content,
            r#"<GraphicalSchema xmlns="http://v8.1c.ru/8.3/xcf/scheme" version="2.20"/>"#,
        )
        .unwrap();
        std::fs::write(
            &mxl_content,
            r#"<document xmlns="http://v8.1c.ru/8.2/data/spreadsheet"/>"#,
        )
        .unwrap();
        std::fs::write(
            &rights_content,
            r#"<Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20"/>"#,
        )
        .unwrap();

        for (tool, argument, path) in [
            ("unica.form.compile", "OutputPath", &form_content),
            ("unica.dcs.edit", "TemplatePath", &template_content),
            ("unica.mxl.compile", "OutputPath", &mxl_content),
        ] {
            let args = Map::from_iter([(
                argument.to_string(),
                Value::String(path.display().to_string()),
            )]);
            let check = evaluate_format_guard(spec(tool), &args, &context(&root)).unwrap();
            let FormatGuardCheck::Block { diagnostic, .. } = check else {
                panic!("{tool} must block on its newer exact metadata wrapper");
            };
            assert_eq!(diagnostic["code"], "platformVersionUnsupported", "{tool}");
            assert_eq!(diagnostic["actualFormat"], "2.21", "{tool}");
        }
        for tool in ["unica.role.info", "unica.role.validate"] {
            let args = Map::from_iter([(
                "RightsPath".to_string(),
                Value::String(rights_content.display().to_string()),
            )]);
            let check = evaluate_format_guard(spec(tool), &args, &context(&root)).unwrap();
            let FormatGuardCheck::Warn { diagnostic, .. } = check else {
                panic!("{tool} must warn on its newer exact role wrapper");
            };
            assert_eq!(diagnostic["code"], "platformVersionUnsupported", "{tool}");
            assert_eq!(diagnostic["actualFormat"], "2.21", "{tool}");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn detached_role_reads_preflight_the_exact_xml_read_by_each_handler() {
        let root = test_root("detached-role-read");
        let role_dir = root.join("detached/Roles/Reader");
        let rights = role_dir.join("Ext/Rights.xml");
        let wrapper = root.join("detached/Roles/Reader.xml");
        let configuration = root.join("detached/Configuration.xml");
        std::fs::create_dir_all(rights.parent().unwrap()).unwrap();
        std::fs::write(
            &rights,
            r#"<Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20"/>"#,
        )
        .unwrap();
        std::fs::write(
            &wrapper,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Role/></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &configuration,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        let args = Map::from_iter([(
            "RightsPath".to_string(),
            Value::String(rights.display().to_string()),
        )]);

        let info_descriptor = native_operation_descriptor("role-info").unwrap();
        assert_eq!(
            effective_format_paths(info_descriptor, &args, &context(&root)).unwrap(),
            vec![rights.clone(), wrapper.clone()],
            "role.info reads Rights.xml and its sibling Role wrapper"
        );
        let check = evaluate_format_guard(spec("unica.role.info"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Warn { diagnostic, .. } = check else {
            panic!("role.info must warn on the newer detached Role wrapper it reads");
        };
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");

        let validate_descriptor = native_operation_descriptor("role-validate").unwrap();
        assert_eq!(
            effective_format_paths(validate_descriptor, &args, &context(&root)).unwrap(),
            vec![rights.clone(), wrapper, configuration],
            "role.validate additionally reads the detached Configuration.xml owner"
        );
        std::fs::write(
            root.join("detached/Roles/Reader.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Role/></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            root.join("detached/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        let check =
            evaluate_format_guard(spec("unica.role.validate"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Warn { diagnostic, .. } = check else {
            panic!("role.validate must warn on the newer detached Configuration.xml it reads");
        };
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn dcs_info_auto_discovery_preflights_every_scanned_wrapper_and_selected_content() {
        let root = test_root("dcs-info-discovery");
        let report = root.join("detached/Reports/Sales");
        let templates = report.join("Templates");
        let selected_wrapper = templates.join("Main.xml");
        let scanned_wrapper = templates.join("Print.xml");
        let selected_content = templates.join("Main/Ext/Template.xml");
        std::fs::create_dir_all(selected_content.parent().unwrap()).unwrap();
        std::fs::write(
            &selected_wrapper,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><Template uuid="11111111-1111-1111-1111-111111111111"><Properties><Name>Main</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Main</v8:content></v8:item></Synonym><Comment/><TemplateType>DataCompositionSchema</TemplateType></Properties></Template></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &scanned_wrapper,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.21"><Template uuid="22222222-2222-2222-2222-222222222222"><Properties><Name>Print</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Print</v8:content></v8:item></Synonym><Comment/><TemplateType>SpreadsheetDocument</TemplateType></Properties></Template></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &selected_content,
            r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>"#,
        )
        .unwrap();
        let args = Map::from_iter([(
            "TemplatePath".to_string(),
            Value::String(report.display().to_string()),
        )]);

        let descriptor = native_operation_descriptor("dcs-info").unwrap();
        assert_eq!(
            effective_format_paths(descriptor, &args, &context(&root)).unwrap(),
            vec![report, selected_wrapper, scanned_wrapper, selected_content],
            "guard dependencies must mirror deterministic DCS auto-discovery reads"
        );
        let check = evaluate_format_guard(spec("unica.dcs.info"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Warn { diagnostic, .. } = check else {
            panic!("dcs.info must warn on a newer wrapper scanned during auto-discovery");
        };
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn dcs_info_multiple_candidates_preflights_scanned_prefix_before_handler_error() {
        let root = test_root("dcs-info-multiple");
        let report = root.join("detached/Reports/Sales");
        let templates = report.join("Templates");
        let newer_wrapper = templates.join("A_Newer.xml");
        let supported_wrapper = templates.join("Z_Supported.xml");
        for name in ["A_Newer", "Z_Supported"] {
            let content = templates.join(name).join("Ext/Template.xml");
            std::fs::create_dir_all(content.parent().unwrap()).unwrap();
            std::fs::write(
                content,
                r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>"#,
            )
            .unwrap();
        }
        std::fs::write(
            &newer_wrapper,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Template><Properties><TemplateType>DataCompositionSchema</TemplateType></Properties></Template></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &supported_wrapper,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template><Properties><TemplateType>DataCompositionSchema</TemplateType></Properties></Template></MetaDataObject>"#,
        )
        .unwrap();
        let args = Map::from_iter([(
            "TemplatePath".to_string(),
            Value::String(report.display().to_string()),
        )]);

        let descriptor = native_operation_descriptor("dcs-info").unwrap();
        assert_eq!(
            effective_format_paths(descriptor, &args, &context(&root)).unwrap(),
            vec![report.clone(), newer_wrapper.clone(), supported_wrapper],
            "format guard must preserve the sorted wrapper prefix already read before ambiguity"
        );
        let check = evaluate_format_guard(spec("unica.dcs.info"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Warn { diagnostic, .. } = check else {
            panic!("dcs.info must warn on the newer wrapper before reporting ambiguity");
        };
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        assert_eq!(
            diagnostic["root"],
            Value::String(normalized_path(&newer_wrapper).display().to_string()),
            "sorted inspection order must make warning attribution deterministic"
        );

        let outcome = analyze_dcs_info(&args, &context(&root));
        assert!(!outcome.ok);
        assert_eq!(
            outcome.errors,
            vec![format!(
                "Multiple DCS templates found in: {}",
                report.display()
            )],
            "handler must retain its original ambiguity error"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn external_init_format_preflight_defers_missing_and_semantic_arguments_to_handler() {
        let root = test_root("external-invalid-args");
        std::fs::create_dir_all(&root).unwrap();

        for tool in ["unica.epf.init", "unica.erf.init"] {
            for args in [
                Map::from_iter([(
                    "OutputDir".to_string(),
                    Value::String("external".to_string()),
                )]),
                Map::from_iter([
                    ("Name".to_string(), Value::String("../Escape".to_string())),
                    (
                        "OutputDir".to_string(),
                        Value::String("external".to_string()),
                    ),
                ]),
            ] {
                let check = evaluate_format_guard(spec(tool), &args, &context(&root))
                    .expect("format preflight must not own ordinary argument errors");
                assert!(
                    matches!(check, FormatGuardCheck::Allow),
                    "{tool} format preflight must allow the handler to report the argument error"
                );
            }
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn newer_dependency_takes_precedence_over_older_migration_advice() {
        let root = test_root("mixed-versions");
        config(&root, Some("2.21"));
        let form = root.join("src/Catalogs/Items/Forms/Item/Ext/Form.xml");
        std::fs::create_dir_all(form.parent().unwrap()).unwrap();
        std::fs::write(
            &form,
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.19"/>"#,
        )
        .unwrap();
        let mut args = Map::new();
        args.insert("FormPath".into(), Value::String(form.display().to_string()));

        let check = evaluate_format_guard(spec("unica.form.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block {
            outcome,
            diagnostic,
        } = check
        else {
            panic!("mixed older/newer dependencies must block mutation");
        };

        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        let warning = outcome.warnings.join("\n");
        assert!(warning.contains("1С 8.5"), "{warning}");
        assert!(!warning.contains("повторно выгруз"), "{warning}");
        assert!(!warning.contains("миграц"), "{warning}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn newer_dependency_takes_precedence_across_independent_effective_paths() {
        let root = test_root("multi-path-priority");
        std::fs::create_dir_all(&root).unwrap();
        let extension = root.join("Extension.xml");
        let base = root.join("Base.xml");
        std::fs::write(
            &extension,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration><Properties><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose></Properties></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &base,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "ExtensionPath".into(),
            Value::String(extension.display().to_string()),
        );
        args.insert(
            "ConfigPath".into(),
            Value::String(base.display().to_string()),
        );

        let check =
            evaluate_format_guard(spec("unica.cfe.borrow"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block {
            outcome,
            diagnostic,
        } = check
        else {
            panic!("a newer dependency on any effective path must dominate an older one");
        };

        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        let warning = outcome.warnings.join("\n");
        assert!(!warning.contains("повторно выгруз"), "{warning}");
        assert!(!warning.contains("миграц"), "{warning}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cfe_borrow_guard_keeps_newer_source_prefix_before_late_missing_form_error() {
        let root = test_root("cfe-borrow-error-prefix");
        let base = root.join("src/Configuration.xml");
        let source_object = root.join("src/Catalogs/Items.xml");
        let extension = root.join("ext/Configuration.xml");
        for path in [&base, &source_object, &extension] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        std::fs::write(
            &base,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="55555555-5555-5555-5555-555555555555"/></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &source_object,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Catalog uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"><Properties><Name>Items</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &extension,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="66666666-6666-6666-6666-666666666666"><InternalInfo/><Properties><ObjectBelonging>Adopted</ObjectBelonging><Name>GuardedExtension</Name><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose><NamePrefix>GE_</NamePrefix></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let args = Map::from_iter([
            (
                "ExtensionPath".to_string(),
                Value::String(root.join("ext").display().to_string()),
            ),
            (
                "ConfigPath".to_string(),
                Value::String(root.join("src").display().to_string()),
            ),
            (
                "Object".to_string(),
                Value::String("Catalog.Items.Form.Missing".to_string()),
            ),
        ]);

        let inspection = cfe_borrow_format_dependency_inspection(&args, &context(&root));
        assert!(
            inspection.paths.contains(&source_object),
            "{:?}",
            inspection.paths
        );
        assert!(
            inspection
                .planning_error
                .as_deref()
                .is_some_and(|error| error.contains("Source form not found")),
            "{:?}",
            inspection.planning_error
        );
        let check =
            evaluate_format_guard(spec("unica.cfe.borrow"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!(
                "the inspected newer source object must block before the late missing-form error"
            );
        };

        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        assert_eq!(
            diagnostic["root"],
            normalized_path(&source_object).display().to_string()
        );
        assert!(!root.join("ext/Catalogs/Items.xml").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn new_dump_inside_older_source_set_requires_user_driven_reexport() {
        let root = test_root("new-dump-nested");
        let owner = config(&root, Some("2.19"));
        let before = std::fs::read(&owner).unwrap();
        let output = root.join("src/Nested");
        let mut args = Map::new();
        args.insert(
            "OutputDir".into(),
            Value::String(output.display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.cf.init"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block {
            outcome,
            diagnostic,
        } = check
        else {
            panic!("a new dump nested in an older source set must block mutation");
        };

        assert!(!outcome.ok);
        assert_eq!(diagnostic["code"], "formatMigrationAvailable");
        assert_eq!(diagnostic["actualFormat"], "2.19");
        assert_eq!(
            diagnostic["root"],
            normalized_path(&owner).display().to_string()
        );
        assert_eq!(std::fs::read(&owner).unwrap(), before);
        assert!(!output.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn new_dump_allows_an_exact_empty_configured_source_set_root() {
        let root = test_root("new-empty-root");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "OutputDir".into(),
            Value::String(root.join("src").display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.cf.init"), &args, &context(&root)).unwrap();

        assert!(matches!(check, FormatGuardCheck::Allow));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cfe_init_output_inside_older_source_set_is_not_hidden_by_missing_base() {
        let root = test_root("cfe-output-owner");
        let owner = config(&root, Some("2.19"));
        let output = root.join("src/NestedExtension");
        let mut args = Map::new();
        args.insert(
            "OutputDir".into(),
            Value::String(output.display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.cfe.init"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("cfe.init output inside an older source set must block");
        };

        assert_eq!(diagnostic["code"], "formatMigrationAvailable");
        assert_eq!(diagnostic["actualFormat"], "2.19");
        assert!(std::fs::read_to_string(owner).unwrap().contains("2.19"));
        assert!(!output.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn code_patch_inside_older_source_set_uses_the_same_format_boundary() {
        let root = test_root("code-patch");
        config(&root, Some("2.19"));
        let module = root.join("src/CommonModules/Core/Ext/Module.bsl");
        std::fs::create_dir_all(module.parent().unwrap()).unwrap();
        std::fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();
        let before = std::fs::read(&module).unwrap();
        let mut args = Map::new();
        args.insert("path".into(), Value::String(module.display().to_string()));

        let check =
            evaluate_format_guard(spec("unica.code.patch"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("code.patch inside an older platform source set must block");
        };

        assert_eq!(diagnostic["code"], "formatMigrationAvailable");
        assert_eq!(diagnostic["actualFormat"], "2.19");
        assert_eq!(std::fs::read(module).unwrap(), before);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn older_extension_dump_recommends_platform_reexport() {
        let root =
            std::env::temp_dir().join(format!("unica-format-guard-old-cfe-{}", std::process::id()));
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let path = src.join("Configuration.xml");
        std::fs::write(
            &path,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration><Properties><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose></Properties></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "ExtensionPath".into(),
            Value::String(path.display().to_string()),
        );

        let check =
            evaluate_format_guard(spec("unica.cfe.patch_method"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { outcome, .. } = check else {
            panic!("older extension mutation must be blocked");
        };
        let warning = outcome.warnings.join("\n");
        assert_platform_reexport_warning(&warning);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cfe_init_preflights_its_optional_cf_base_with_platform_reexport() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-cfe-init-{}",
            std::process::id()
        ));
        let path = config(&root, Some("2.19"));
        let mut args = Map::new();
        args.insert(
            "ConfigPath".into(),
            Value::String(path.display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.cfe.init"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block {
            outcome,
            diagnostic,
        } = check
        else {
            panic!("older optional CF base must block CFE init");
        };
        assert_eq!(diagnostic["code"], "formatMigrationAvailable");
        let warning = outcome.warnings.join("\n");
        assert_platform_reexport_warning(&warning);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn supported_dump_allows_mutation_preflight() {
        let root =
            std::env::temp_dir().join(format!("unica-format-guard-ok-{}", std::process::id()));
        let path = config(&root, Some("2.20"));
        let mut args = Map::new();
        args.insert(
            "ConfigPath".into(),
            Value::String(path.display().to_string()),
        );
        assert!(matches!(
            evaluate_format_guard(spec("unica.cf.edit"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Allow
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn newer_dump_warns_for_read_only_with_roadmap_copy() {
        let root =
            std::env::temp_dir().join(format!("unica-format-guard-new-{}", std::process::id()));
        let path = config(&root, Some("2.21"));
        let mut args = Map::new();
        args.insert(
            "ConfigPath".into(),
            Value::String(path.display().to_string()),
        );
        let check = evaluate_format_guard(spec("unica.cf.info"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Warn {
            warning,
            diagnostic,
        } = check
        else {
            panic!("newer read-only input must warn and continue");
        };
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert!(warning.contains("Поддержка платформы 1С 8.5 планируется"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_root_version_is_classified_as_1_0() {
        let root =
            std::env::temp_dir().join(format!("unica-format-guard-v1-{}", std::process::id()));
        let path = config(&root, None);
        let mut args = Map::new();
        args.insert(
            "ConfigPath".into(),
            Value::String(path.display().to_string()),
        );
        let check =
            evaluate_format_guard(spec("unica.cf.validate"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Warn { diagnostic, .. } = check else {
            panic!("missing root version must be old-format warning");
        };
        assert_eq!(diagnostic["actualFormat"], "1.0");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn versionless_known_standalone_form_is_classified_as_1_0_owner() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-versionless-standalone-form-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("Form.xml");
        std::fs::write(
            &target,
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"/>"#,
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "FormPath".into(),
            Value::String(target.display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.form.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("a versionless standalone Form is a 1.0 owner and must block mutation");
        };
        assert_eq!(diagnostic["actualFormat"], "1.0");
        assert_eq!(diagnostic["ownerKind"], "standalone");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn effective_format_paths_match_mutating_handler_directory_and_alias_resolution() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-handler-paths-{}",
            std::process::id()
        ));
        let config_path = config(&root, Some("2.19"));
        let src = config_path.parent().unwrap();

        let object_dir = src.join("Catalogs/Items");
        std::fs::create_dir_all(&object_dir).unwrap();
        let object_xml = src.join("Catalogs/Items.xml");
        std::fs::write(&object_xml, "object bytes").unwrap();

        let subsystem_dir = src.join("Subsystems/Sales");
        std::fs::create_dir_all(&subsystem_dir).unwrap();
        let subsystem_xml = src.join("Subsystems/Sales.xml");
        std::fs::write(&subsystem_xml, "subsystem bytes").unwrap();

        let template_dir = src.join("Reports/Sales/Templates/Print");
        let template_xml = template_dir.join("Ext/Template.xml");
        std::fs::create_dir_all(template_xml.parent().unwrap()).unwrap();
        std::fs::write(&template_xml, "template bytes").unwrap();

        let form_xml = src.join("Catalogs/Items/Forms/Main/Ext/Form.xml");
        std::fs::create_dir_all(form_xml.parent().unwrap()).unwrap();
        std::fs::write(&form_xml, "form bytes").unwrap();

        let protected_paths = [
            config_path.clone(),
            object_xml.clone(),
            subsystem_xml.clone(),
            template_xml.clone(),
            form_xml.clone(),
        ];
        let before = protected_paths
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect::<Vec<_>>();

        let cases = [
            (
                "cf-edit",
                "path",
                src.to_path_buf(),
                vec![config_path.clone()],
            ),
            (
                "meta-edit",
                "Path",
                object_dir.clone(),
                vec![object_xml.clone()],
            ),
            (
                "form-add",
                "path",
                object_dir,
                vec![object_xml.canonicalize().unwrap()],
            ),
            (
                "subsystem-edit",
                "Path",
                subsystem_dir,
                vec![subsystem_xml.canonicalize().unwrap()],
            ),
            ("dcs-edit", "path", template_dir, vec![template_xml]),
            ("form-edit", "Path", form_xml.clone(), vec![form_xml]),
        ];

        for (operation, alias, raw, expected) in cases {
            let mut args = Map::new();
            args.insert(alias.into(), Value::String(raw.display().to_string()));
            let descriptor = native_operation_descriptor(operation).unwrap();
            assert_eq!(
                effective_format_paths(descriptor, &args, &context(&root)).unwrap(),
                expected,
                "{operation} must guard the same effective XML path as its handler"
            );
            assert!(
                matches!(
                    evaluate_format_guard(
                        spec(&format!("unica.{}", operation.replace('-', "."))),
                        &args,
                        &context(&root)
                    )
                    .unwrap(),
                    FormatGuardCheck::Block { .. }
                ),
                "{operation} alias {alias} must be blocked before its handler can write"
            );
        }
        for (path, expected) in protected_paths.iter().zip(before) {
            assert_eq!(
                std::fs::read(path).unwrap(),
                expected,
                "format preflight must not mutate {}",
                path.display()
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn specialized_format_path_policies_resolve_representative_defaults() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-specialized-paths-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let context = context(&root);

        for operation in ["help-add", "form-remove", "template-add", "template-remove"] {
            let mut args = Map::new();
            args.insert("ObjectName".into(), Value::String("Reports/Sales".into()));
            let descriptor = native_operation_descriptor(operation).unwrap();
            let mut expected = vec![root.join("src/Reports/Sales.xml")];
            if operation == "help-add" {
                expected.push(root.join("src/Reports/Sales/Ext/Help.xml"));
            }
            assert_eq!(
                effective_format_paths(descriptor, &args, &context).unwrap(),
                expected,
                "{operation} must compose its handler default SrcDir=src with ObjectName"
            );
        }

        let mut form_compile_args = Map::new();
        form_compile_args.insert(
            "OutputPath".into(),
            Value::String("src/Catalogs/Items/Forms/Main/Ext/Form.xml".into()),
        );
        form_compile_args.insert("FromObject".into(), Value::Bool(true));
        form_compile_args.insert(
            "ObjectPath".into(),
            Value::String("src/Catalogs/Items".into()),
        );
        let descriptor = native_operation_descriptor("form-compile").unwrap();
        assert_eq!(
            effective_format_paths(descriptor, &form_compile_args, &context).unwrap(),
            vec![
                root.join("src/Catalogs/Items/Forms/Main/Ext/Form.xml"),
                root.join("src/Catalogs/Items.xml"),
            ],
            "form-compile must guard both its normalized output and from-object input"
        );

        let json_form_args = Map::from_iter([
            (
                "OutputPath".into(),
                Value::String("src/Catalogs/Detached/Forms/Main/Ext/Form.xml".into()),
            ),
            ("JsonPath".into(), Value::String("form.json".into())),
        ]);
        assert_eq!(
            effective_format_paths(descriptor, &json_form_args, &context).unwrap(),
            vec![
                root.join("src/Catalogs/Detached/Forms/Main/Ext/Form.xml"),
                root.join("src/Catalogs/Detached.xml"),
            ],
            "form-compile must guard the structural parent candidate before it exists"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn read_only_path_aliases_still_reach_the_format_warning() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-read-aliases-{}",
            std::process::id()
        ));
        let config_path = config(&root, Some("2.19"));
        let src = config_path.parent().unwrap();
        let cases = [
            (
                "unica.form.info",
                "Path",
                src.join("Catalogs/Items/Forms/Main/Ext/Form.xml"),
            ),
            (
                "unica.dcs.info",
                "path",
                src.join("Reports/Sales/Templates/Main/Ext/Template.xml"),
            ),
            (
                "unica.mxl.info",
                "Path",
                src.join("Reports/Sales/Templates/Print/Ext/Template.xml"),
            ),
            (
                "unica.subsystem.info",
                "path",
                src.join("Subsystems/Sales.xml"),
            ),
        ];

        for (tool, alias, path) in cases {
            let mut args = Map::new();
            args.insert(alias.into(), Value::String(path.display().to_string()));
            let check = evaluate_format_guard(spec(tool), &args, &context(&root)).unwrap();
            let FormatGuardCheck::Warn { diagnostic, .. } = check else {
                panic!("{tool} alias {alias} must resolve the old owner and warn");
            };
            assert_eq!(diagnostic["actualFormat"], "2.19", "{tool}");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn read_only_handler_resolved_paths_match_directory_inputs() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-read-handler-paths-{}",
            std::process::id()
        ));
        let configuration = config(&root, Some("2.19"));
        let src = configuration.parent().unwrap().to_path_buf();
        let canonical_configuration = configuration.canonicalize().unwrap();
        let home_page = canonical_configuration
            .parent()
            .unwrap()
            .join("Ext/HomePageWorkArea.xml");
        let extension = root.join("extension");
        std::fs::create_dir_all(&extension).unwrap();
        let extension_configuration = extension.join("Configuration.xml");
        std::fs::write(&extension_configuration, "extension").unwrap();
        let role_dir = src.join("Roles/Reader");
        let rights = role_dir.join("Ext/Rights.xml");
        std::fs::create_dir_all(rights.parent().unwrap()).unwrap();
        std::fs::write(&rights, "rights").unwrap();

        for (operation, alias, directory, expected) in [
            (
                "cf-info",
                "Path",
                src.clone(),
                vec![canonical_configuration.clone()],
            ),
            (
                "cf-validate",
                "path",
                src.clone(),
                vec![canonical_configuration, home_page],
            ),
            (
                "cfe-validate",
                "Path",
                extension,
                vec![extension_configuration.canonicalize().unwrap()],
            ),
            ("role-info", "path", role_dir.clone(), vec![rights.clone()]),
            (
                "role-validate",
                "Path",
                role_dir,
                vec![rights, src.join("Configuration.xml")],
            ),
        ] {
            let mut args = Map::new();
            args.insert(alias.into(), Value::String(directory.display().to_string()));
            let descriptor = native_operation_descriptor(operation).unwrap();
            assert_eq!(
                effective_format_paths(descriptor, &args, &context(&root)).unwrap(),
                expected,
                "{operation} must guard the same resolved file as its handler"
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn read_only_xml_analyzers_preflight_the_exact_file_resolved_from_a_directory() {
        let root = test_root("read-resolved-xml");
        let catalog_dir = root.join("detached/Catalogs/Goods");
        let catalog_xml = root.join("detached/Catalogs/Goods.xml");
        let form_dir = catalog_dir.join("Forms/Main");
        let form_xml = form_dir.join("Ext/Form.xml");
        let dcs_dir = catalog_dir.join("Templates/Schema");
        let dcs_xml = dcs_dir.join("Ext/Template.xml");
        let mxl_dir = catalog_dir.join("Templates/Print");
        let mxl_xml = mxl_dir.join("Ext/Template.xml");
        let interface_dir = root.join("detached/Subsystems/Sales");
        let interface_xml = interface_dir.join("Ext/CommandInterface.xml");
        for path in [&catalog_dir, &form_xml, &dcs_xml, &mxl_xml, &interface_xml] {
            std::fs::create_dir_all(if path.extension().is_some() {
                path.parent().unwrap()
            } else {
                path
            })
            .unwrap();
        }
        std::fs::write(
            &catalog_xml,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog/></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &form_xml,
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#,
        )
        .unwrap();
        std::fs::write(
            &dcs_xml,
            r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>"#,
        )
        .unwrap();
        std::fs::write(
            &mxl_xml,
            r#"<document xmlns="http://v8.1c.ru/8.2/data/spreadsheet"/>"#,
        )
        .unwrap();
        std::fs::write(
            &interface_xml,
            r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"/>"#,
        )
        .unwrap();

        for (operation, argument, directory, expected) in [
            (
                "meta-info",
                "path",
                catalog_dir.clone(),
                catalog_xml.clone(),
            ),
            ("meta-validate", "Path", catalog_dir, catalog_xml),
            ("form-info", "path", form_dir.clone(), form_xml.clone()),
            ("form-validate", "Path", form_dir, form_xml),
            ("dcs-validate", "path", dcs_dir, dcs_xml),
            ("mxl-validate", "Path", mxl_dir, mxl_xml),
            ("interface-validate", "path", interface_dir, interface_xml),
        ] {
            let args = Map::from_iter([(
                argument.to_string(),
                Value::String(directory.display().to_string()),
            )]);
            let descriptor = native_operation_descriptor(operation).unwrap();
            assert_eq!(
                effective_format_paths(descriptor, &args, &context(&root)).unwrap(),
                vec![expected],
                "{operation} must guard the exact XML file opened by its handler"
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn read_only_xml_analyzers_warn_for_resolved_newer_roots_and_allow_exact_2_20_roots() {
        let root = test_root("read-resolved-version");
        let catalog_dir = root.join("detached/Catalogs/Goods");
        let catalog_xml = root.join("detached/Catalogs/Goods.xml");
        let form_dir = catalog_dir.join("Forms/Main");
        let form_xml = form_dir.join("Ext/Form.xml");
        let dcs_dir = catalog_dir.join("Templates/Schema");
        let dcs_wrapper = root.join("detached/Catalogs/Goods/Templates/Schema.xml");
        let dcs_xml = dcs_dir.join("Ext/Template.xml");
        let mxl_dir = catalog_dir.join("Templates/Print");
        let mxl_wrapper = root.join("detached/Catalogs/Goods/Templates/Print.xml");
        let mxl_xml = mxl_dir.join("Ext/Template.xml");
        let interface_dir = root.join("detached/Subsystems/Sales");
        let interface_xml = interface_dir.join("Ext/CommandInterface.xml");
        for path in [
            &catalog_dir,
            &form_xml,
            &dcs_wrapper,
            &dcs_xml,
            &mxl_wrapper,
            &mxl_xml,
            &interface_xml,
        ] {
            std::fs::create_dir_all(if path.extension().is_some() {
                path.parent().unwrap()
            } else {
                path
            })
            .unwrap();
        }
        std::fs::write(
            &catalog_xml,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Catalog/></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &form_xml,
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.21"/>"#,
        )
        .unwrap();
        for wrapper in [&dcs_wrapper, &mxl_wrapper] {
            std::fs::write(
                wrapper,
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Template/></MetaDataObject>"#,
            )
            .unwrap();
        }
        std::fs::write(
            &dcs_xml,
            r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>"#,
        )
        .unwrap();
        std::fs::write(
            &mxl_xml,
            r#"<document xmlns="http://v8.1c.ru/8.2/data/spreadsheet"/>"#,
        )
        .unwrap();
        std::fs::write(
            &interface_xml,
            r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.21"/>"#,
        )
        .unwrap();

        for (tool, argument, directory) in [
            ("unica.meta.info", "ObjectPath", catalog_dir.clone()),
            ("unica.form.info", "FormPath", form_dir.clone()),
            ("unica.form.validate", "FormPath", form_dir),
            ("unica.dcs.validate", "TemplatePath", dcs_dir),
            ("unica.mxl.validate", "TemplatePath", mxl_dir),
            ("unica.interface.validate", "CIPath", interface_dir),
        ] {
            let args = Map::from_iter([(
                argument.to_string(),
                Value::String(directory.display().to_string()),
            )]);
            let check = evaluate_format_guard(spec(tool), &args, &context(&root)).unwrap();
            let FormatGuardCheck::Warn { diagnostic, .. } = check else {
                panic!("{tool} must warn for the newer XML resolved from its directory input");
            };
            assert_eq!(diagnostic["code"], "platformVersionUnsupported", "{tool}");
            assert_eq!(diagnostic["actualFormat"], "2.21", "{tool}");
        }

        std::fs::write(
            &catalog_xml,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog/></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &form_xml,
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#,
        )
        .unwrap();
        std::fs::write(
            &interface_xml,
            r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"/>"#,
        )
        .unwrap();
        for wrapper in [&dcs_wrapper, &mxl_wrapper] {
            std::fs::write(
                wrapper,
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template/></MetaDataObject>"#,
            )
            .unwrap();
        }
        for (tool, argument, exact) in [
            ("unica.meta.info", "ObjectPath", catalog_xml),
            ("unica.form.info", "FormPath", form_xml),
            ("unica.dcs.validate", "TemplatePath", dcs_xml),
            ("unica.mxl.validate", "TemplatePath", mxl_xml),
            ("unica.interface.validate", "CIPath", interface_xml),
        ] {
            let args = Map::from_iter([(
                argument.to_string(),
                Value::String(exact.display().to_string()),
            )]);
            assert!(
                matches!(
                    evaluate_format_guard(spec(tool), &args, &context(&root)).unwrap(),
                    FormatGuardCheck::Allow
                ),
                "{tool} must allow an exact 2.20 root"
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn dcs_edit_blocks_old_external_source_set_via_owner_descriptor() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-old-external-dcs-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source_root = external_source_set(
            &root,
            "EXTERNAL_DATA_PROCESSORS",
            "epf",
            "PriceLoader",
            "2.19",
        );
        let target = source_root.join("PriceLoader/Templates/Main/Ext/Template.xml");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "<DataCompositionSchema/>").unwrap();
        let mut args = Map::new();
        args.insert(
            "TemplatePath".into(),
            Value::String(target.display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.dcs.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block {
            outcome,
            diagnostic,
        } = check
        else {
            panic!("old EPF owner must block DCS edit");
        };
        assert_eq!(diagnostic["actualFormat"], "2.19");
        assert_platform_reexport_warning(&outcome.warnings.join("\n"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn mxl_info_warns_old_external_source_set_via_owner_descriptor() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-old-external-mxl-info-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source_root = external_source_set(&root, "EXTERNAL_REPORTS", "erf", "Sales", "2.19");
        let target = source_root.join("Sales/Templates/Print/Ext/Template.xml");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "<document/>").unwrap();
        let mut args = Map::new();
        args.insert(
            "TemplatePath".into(),
            Value::String(target.display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.mxl.info"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Warn {
            warning,
            diagnostic,
        } = check
        else {
            panic!("old ERF owner must warn for read-only MXL info");
        };
        assert_eq!(diagnostic["actualFormat"], "2.19");
        assert_platform_reexport_warning(&warning);
        assert!(
            warning.contains("Доступен только режим чтения."),
            "{warning}"
        );
        assert!(!warning.contains("Изменение отменено."), "{warning}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn standalone_owner_warning_recommends_platform_reexport_without_tool_name() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-old-standalone-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let form = root.join("Form.xml");
        std::fs::write(
            &form,
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.19"/>"#,
        )
        .unwrap();
        let mut args = Map::new();
        args.insert("FormPath".into(), Value::String(form.display().to_string()));

        let check = evaluate_format_guard(spec("unica.form.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { outcome, .. } = check else {
            panic!("old standalone owner must block form edit");
        };
        assert_platform_reexport_warning(&outcome.warnings.join("\n"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn form_remove_default_src_blocks_older_dump() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-default-form-remove-{}",
            std::process::id()
        ));
        config(&root, Some("2.19"));
        let mut args = Map::new();
        args.insert("ObjectName".into(), Value::String("Catalogs/Items".into()));

        assert!(matches!(
            evaluate_format_guard(spec("unica.form.remove"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Block { .. }
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn template_remove_default_src_blocks_newer_dump() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-default-template-remove-{}",
            std::process::id()
        ));
        config(&root, Some("2.21"));
        let mut args = Map::new();
        args.insert("ObjectName".into(), Value::String("Reports/Sales".into()));

        let check =
            evaluate_format_guard(spec("unica.template.remove"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("newer default source dump must block template removal");
        };
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn help_add_default_src_blocks_old_external_source_set() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-default-help-external-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        external_source_set(
            &root,
            "EXTERNAL_DATA_PROCESSORS",
            "src",
            "PriceLoader",
            "2.19",
        );
        let mut args = Map::new();
        args.insert("ObjectName".into(), Value::String("PriceLoader".into()));

        assert!(matches!(
            evaluate_format_guard(spec("unica.help.add"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Block { .. }
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn format_guard_normalizes_parent_segments_before_owner_lookup() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-normalized-parent-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source_root = external_source_set(
            &root,
            "EXTERNAL_DATA_PROCESSORS",
            "epf",
            "PriceLoader",
            "2.19",
        );
        let target = source_root.join("PriceLoader/Templates/../Templates/Main/Ext/Template.xml");
        let mut args = Map::new();
        args.insert(
            "TemplatePath".into(),
            Value::String(target.display().to_string()),
        );

        assert!(matches!(
            evaluate_format_guard(spec("unica.dcs.edit"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Block { .. }
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn standalone_compile_does_not_inherit_unrelated_workspace_configuration() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-standalone-output-{}",
            std::process::id()
        ));
        config(&root, Some("2.19"));
        let standalone = root.join("generated/report.xml");
        let mut args = Map::new();
        args.insert(
            "OutputPath".into(),
            Value::String(standalone.display().to_string()),
        );

        assert!(matches!(
            evaluate_format_guard(spec("unica.mxl.compile"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Allow
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn mxl_compile_blocks_write_inside_older_dump() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-mxl-compile-old-{}",
            std::process::id()
        ));
        config(&root, Some("2.19"));
        let output = root.join("src/Reports/Sales/Templates/Print/Ext/Template.xml");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        std::fs::write(&output, b"original bytes").unwrap();
        let before = std::fs::read(&output).unwrap();
        let mut args = Map::new();
        args.insert(
            "OutputPath".into(),
            Value::String(output.display().to_string()),
        );

        assert!(matches!(
            evaluate_format_guard(spec("unica.mxl.compile"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Block { .. }
        ));
        assert_eq!(std::fs::read(&output).unwrap(), before);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn form_compile_blocks_old_external_source_set_before_create() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-form-compile-old-external-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source_root = external_source_set(
            &root,
            "EXTERNAL_DATA_PROCESSORS",
            "epf",
            "PriceLoader",
            "2.19",
        );
        let output = source_root.join("PriceLoader/Forms/Main/Ext/Form.xml");
        let mut args = Map::new();
        args.insert(
            "OutputPath".into(),
            Value::String(output.display().to_string()),
        );

        assert!(matches!(
            evaluate_format_guard(spec("unica.form.compile"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Block { .. }
        ));
        assert!(!output.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn form_compile_from_object_checks_input_and_output_formats() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-form-compile-input-output-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("old")).unwrap();
        std::fs::create_dir_all(root.join("active")).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: old\n    type: CONFIGURATION\n    path: old\n  - name: active\n    type: CONFIGURATION\n    path: active\n",
        )
        .unwrap();
        std::fs::write(
            root.join("old/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            root.join("active/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        let object = root.join("old/Catalogs/Items.xml");
        std::fs::create_dir_all(object.parent().unwrap()).unwrap();
        std::fs::write(&object, "<MetaDataObject/>").unwrap();
        let output = root.join("active/Catalogs/Items/Forms/Main/Ext/Form.xml");
        let mut args = Map::new();
        args.insert(
            "OutputPath".into(),
            Value::String(output.display().to_string()),
        );
        args.insert("FromObject".into(), Value::Bool(true));
        args.insert(
            "ObjectPath".into(),
            Value::String(object.display().to_string()),
        );

        let check =
            evaluate_format_guard(spec("unica.form.compile"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("old from-object input must block even when output is active");
        };
        assert_eq!(diagnostic["actualFormat"], "2.19");
        assert!(!output.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_owner_returns_structured_format_version_invalid() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-malformed-owner-{}",
            std::process::id()
        ));
        let owner = config(&root, Some("2.20"));
        std::fs::write(&owner, "<broken").unwrap();
        let mut args = Map::new();
        args.insert(
            "ObjectPath".into(),
            Value::String(root.join("src/Catalogs/Items.xml").display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.meta.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("malformed owner must produce a structured blocking diagnostic");
        };
        assert_eq!(diagnostic["code"], "formatVersionInvalid");
        assert!(diagnostic["root"]
            .as_str()
            .is_some_and(|path| std::path::Path::new(path).ends_with("src/Configuration.xml")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unreadable_utf8_owner_returns_structured_format_version_invalid() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-unreadable-owner-{}",
            std::process::id()
        ));
        let owner = config(&root, Some("2.20"));
        std::fs::write(&owner, [0xff, 0xfe, 0xfd]).unwrap();
        let mut args = Map::new();
        args.insert(
            "ObjectPath".into(),
            Value::String(root.join("src/Catalogs/Items.xml").display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.meta.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("non-UTF-8 owner must produce a structured blocking diagnostic");
        };
        assert_eq!(diagnostic["code"], "formatVersionInvalid");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_owner_inside_recognized_source_set_is_invalid() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-missing-owner-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("src/Catalogs")).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "ObjectPath".into(),
            Value::String(root.join("src/Catalogs/Items.xml").display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.meta.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("missing owner in a configured source set must block");
        };
        assert_eq!(diagnostic["code"], "formatVersionInvalid");
        assert!(diagnostic["root"]
            .as_str()
            .is_some_and(|path| std::path::Path::new(path).ends_with("src/Configuration.xml")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_existing_standalone_xml_is_invalid_not_new_output() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-malformed-standalone-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let output = root.join("standalone.xml");
        std::fs::write(&output, "<broken").unwrap();
        let mut args = Map::new();
        args.insert(
            "OutputPath".into(),
            Value::String(output.display().to_string()),
        );

        let check =
            evaluate_format_guard(spec("unica.mxl.compile"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("malformed existing standalone XML must not be treated as a new output");
        };
        assert_eq!(diagnostic["code"], "formatVersionInvalid");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn valid_standalone_mxl_without_owner_version_is_not_an_old_dump() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-valid-standalone-mxl-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let document = root.join("standalone.xml");
        std::fs::write(
            &document,
            r#"<document xmlns="http://v8.1c.ru/8.2/data/spreadsheet"/>"#,
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "TemplatePath".into(),
            Value::String(document.display().to_string()),
        );

        assert!(matches!(
            evaluate_format_guard(spec("unica.mxl.info"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Allow
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn direct_external_descriptor_uses_external_owner_copy() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-direct-external-owner-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let descriptor = root.join("PriceLoader.xml");
        std::fs::write(
            &descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><ExternalDataProcessor/></MetaDataObject>"#,
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "ObjectPath".into(),
            Value::String(descriptor.display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.meta.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block {
            outcome,
            diagnostic,
        } = check
        else {
            panic!("old direct EPF descriptor must block");
        };
        assert_eq!(diagnostic["ownerKind"], "external_processor");
        assert_platform_reexport_warning(&outcome.warnings.join("\n"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn direct_external_descriptor_rejects_extra_artifact_children() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-direct-external-extra-child-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let descriptor = root.join("PriceLoader.xml");
        std::fs::write(
            &descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><ExternalDataProcessor/><Catalog/></MetaDataObject>"#,
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "ObjectPath".into(),
            Value::String(descriptor.display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.meta.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("direct EPF owner with extra artifact child must be invalid");
        };
        assert_eq!(diagnostic["code"], "formatVersionInvalid");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn external_source_root_with_one_descriptor_resolves_that_owner() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-external-root-owner-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source_root = external_source_set(&root, "EXTERNAL_REPORTS", "erf", "Sales", "2.19");
        let mut args = Map::new();
        args.insert(
            "OutputPath".into(),
            Value::String(source_root.display().to_string()),
        );

        let check =
            evaluate_format_guard(spec("unica.mxl.compile"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("external source root with one descriptor must resolve its owner");
        };
        assert_eq!(diagnostic["ownerKind"], "external_report");
        assert!(diagnostic["root"]
            .as_str()
            .is_some_and(|path| std::path::Path::new(path).ends_with("erf/Sales.xml")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn configured_owner_rejects_wrong_root_qname_and_artifact_type() {
        for (case, owner) in [
            (
                "wrong-root",
                r#"<garbage xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></garbage>"#,
            ),
            (
                "wrong-namespace",
                r#"<MetaDataObject xmlns="urn:wrong" version="2.20"><Configuration/></MetaDataObject>"#,
            ),
        ] {
            let root = std::env::temp_dir().join(format!(
                "unica-format-guard-configured-{case}-{}",
                std::process::id()
            ));
            std::fs::create_dir_all(root.join("src/Catalogs")).unwrap();
            std::fs::write(
                root.join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
            std::fs::write(root.join("src/Configuration.xml"), owner).unwrap();
            let mut args = Map::new();
            args.insert(
                "ObjectPath".into(),
                Value::String(root.join("src/Catalogs/Items.xml").display().to_string()),
            );

            let check =
                evaluate_format_guard(spec("unica.meta.edit"), &args, &context(&root)).unwrap();
            let FormatGuardCheck::Block { diagnostic, .. } = check else {
                panic!("{case}: wrong configured owner contract must block");
            };
            assert_eq!(diagnostic["code"], "formatVersionInvalid", "{case}");
            let _ = std::fs::remove_dir_all(root);
        }

        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-wrong-external-kind-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("epf/PriceLoader")).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: epf\n",
        )
        .unwrap();
        std::fs::write(
            root.join("epf/PriceLoader.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><ExternalReport/></MetaDataObject>"#,
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "TemplatePath".into(),
            Value::String(
                root.join("epf/PriceLoader/Templates/Main/Ext/Template.xml")
                    .display()
                    .to_string(),
            ),
        );
        let check = evaluate_format_guard(spec("unica.dcs.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("EPF source set must reject an ERF owner descriptor");
        };
        assert_eq!(diagnostic["code"], "formatVersionInvalid");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unknown_version_bearing_standalone_root_is_invalid() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-unknown-standalone-root-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("unknown.xml");
        std::fs::write(&target, r#"<garbage version="2.20"/>"#).unwrap();
        let mut args = Map::new();
        args.insert(
            "FormPath".into(),
            Value::String(target.display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.form.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("unknown version-bearing standalone root must be invalid");
        };
        assert_eq!(diagnostic["code"], "formatVersionInvalid");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn known_standalone_form_root_remains_supported() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-known-standalone-form-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("Form.xml");
        std::fs::write(
            &target,
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#,
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "FormPath".into(),
            Value::String(target.display().to_string()),
        );

        assert!(matches!(
            evaluate_format_guard(spec("unica.form.edit"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Allow
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn subsystem_validate_warns_for_newer_direct_command_interface() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-subsystem-validate-ci-{}",
            std::process::id()
        ));
        let subsystem = root.join("Subsystems/Sales.xml");
        std::fs::create_dir_all(subsystem.parent().unwrap()).unwrap();
        std::fs::write(
            &subsystem,
            crate::infrastructure::native_operations::subsystem::child_subsystem_stub_xml(
                "Sales", "2.20",
            ),
        )
        .unwrap();
        let command_interface = root.join("Subsystems/Sales/Ext/CommandInterface.xml");
        std::fs::create_dir_all(command_interface.parent().unwrap()).unwrap();
        std::fs::write(
            &command_interface,
            r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.21"/>"#,
        )
        .unwrap();
        let args = Map::from_iter([(
            "SubsystemPath".to_string(),
            Value::String(subsystem.display().to_string()),
        )]);

        let check = evaluate_format_guard(spec("unica.subsystem.validate"), &args, &context(&root))
            .unwrap();
        let FormatGuardCheck::Warn { diagnostic, .. } = check else {
            panic!("newer direct command interface must produce a read-only warning");
        };
        assert_eq!(diagnostic["actualFormat"], "2.21");
        assert_eq!(
            normalized_path(&std::path::PathBuf::from(
                diagnostic["root"].as_str().unwrap()
            )),
            normalized_path(&command_interface)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn subsystem_tree_warns_for_newer_registered_child_descriptor() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-subsystem-tree-child-{}",
            std::process::id()
        ));
        let parent = root.join("Subsystems/Parent.xml");
        let child = root.join("Subsystems/Parent/Subsystems/Child.xml");
        std::fs::create_dir_all(child.parent().unwrap()).unwrap();
        let parent_xml =
            crate::infrastructure::native_operations::subsystem::child_subsystem_stub_xml(
                "Parent", "2.20",
            )
            .replacen(
                "<ChildObjects/>",
                "<ChildObjects><Subsystem>Child</Subsystem></ChildObjects>",
                1,
            );
        std::fs::write(&parent, parent_xml).unwrap();
        std::fs::write(
            &child,
            crate::infrastructure::native_operations::subsystem::child_subsystem_stub_xml(
                "Child", "2.21",
            ),
        )
        .unwrap();
        let args = Map::from_iter([
            (
                "SubsystemPath".to_string(),
                Value::String(parent.display().to_string()),
            ),
            ("Mode".to_string(), Value::String("tree".to_string())),
        ]);

        let check =
            evaluate_format_guard(spec("unica.subsystem.info"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Warn { diagnostic, .. } = check else {
            panic!("newer registered tree child must produce a read-only warning");
        };
        assert_eq!(diagnostic["actualFormat"], "2.21");
        assert_eq!(
            normalized_path(&std::path::PathBuf::from(
                diagnostic["root"].as_str().unwrap()
            )),
            normalized_path(&child)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_validate_warns_for_newer_registrar_document_it_reads() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-meta-validate-registrar-{}",
            std::process::id()
        ));
        config(&root, Some("2.20"));
        let register = root.join("src/AccumulationRegisters/Sales.xml");
        let document = root.join("src/Documents/Recorder.xml");
        std::fs::create_dir_all(register.parent().unwrap()).unwrap();
        std::fs::create_dir_all(document.parent().unwrap()).unwrap();
        std::fs::write(
            &register,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><AccumulationRegister><Properties><Name>Sales</Name></Properties><ChildObjects/></AccumulationRegister></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &document,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Document><Properties><Name>Recorder</Name></Properties><ChildObjects/></Document></MetaDataObject>"#,
        )
        .unwrap();
        let args = Map::from_iter([(
            "ObjectPath".to_string(),
            Value::String(register.display().to_string()),
        )]);

        let check =
            evaluate_format_guard(spec("unica.meta.validate"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Warn { diagnostic, .. } = check else {
            panic!("newer registrar document read by meta.validate must warn");
        };
        assert_eq!(diagnostic["actualFormat"], "2.21");
        assert_eq!(
            normalized_path(&std::path::PathBuf::from(
                diagnostic["root"].as_str().unwrap()
            )),
            normalized_path(&document)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_validate_registrar_dependencies_follow_sorted_handler_read_order() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-meta-validate-sorted-registrar-{}",
            std::process::id()
        ));
        config(&root, Some("2.20"));
        let register = root.join("src/AccumulationRegisters/Sales.xml");
        let later = root.join("src/Documents/z-later.xml");
        let first = root.join("src/Documents/a-first.xml");
        for path in [&register, &later, &first] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        std::fs::write(
            &register,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><AccumulationRegister><Properties><Name>Sales</Name></Properties><ChildObjects/></AccumulationRegister></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &later,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Document><Properties><Name>Later</Name></Properties><ChildObjects/></Document></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &first,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document><Properties><Name>First</Name><RegisterRecords>AccumulationRegister.Sales</RegisterRecords></Properties><ChildObjects/></Document></MetaDataObject>"#,
        )
        .unwrap();
        let args = Map::from_iter([(
            "ObjectPath".to_string(),
            Value::String(register.display().to_string()),
        )]);
        let descriptor = native_operation_descriptor("meta-validate").unwrap();

        let dependencies = effective_format_paths(descriptor, &args, &context(&root)).unwrap();

        assert!(dependencies.contains(&first), "{dependencies:?}");
        assert!(!dependencies.contains(&later), "{dependencies:?}");
        assert!(matches!(
            evaluate_format_guard(spec("unica.meta.validate"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Allow
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn internal_local_meta_validation_excludes_registrar_documents() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-meta-validate-local-only-{}",
            std::process::id()
        ));
        config(&root, Some("2.20"));
        let register = root.join("src/AccumulationRegisters/Sales.xml");
        let document = root.join("src/Documents/Recorder.xml");
        for path in [&register, &document] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        std::fs::write(
            &register,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><AccumulationRegister><Properties><Name>Sales</Name></Properties><ChildObjects/></AccumulationRegister></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &document,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Document/></MetaDataObject>"#,
        )
        .unwrap();
        let args = Map::from_iter([
            (
                "ObjectPath".to_string(),
                Value::String(register.display().to_string()),
            ),
            ("InternalLocalOwnerOnly".to_string(), Value::Bool(true)),
        ]);
        let descriptor = native_operation_descriptor("meta-validate").unwrap();

        let dependencies = effective_format_paths(descriptor, &args, &context(&root)).unwrap();

        assert!(!dependencies.contains(&document), "{dependencies:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_compile_does_not_treat_unrelated_documents_as_format_dependencies() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-meta-compile-local-owner-{}",
            std::process::id()
        ));
        config(&root, Some("2.20"));
        let document = root.join("src/Documents/Unrelated.xml");
        std::fs::create_dir_all(document.parent().unwrap()).unwrap();
        std::fs::write(
            &document,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Document/></MetaDataObject>"#,
        )
        .unwrap();
        let definition = root.join("register.json");
        std::fs::write(
            &definition,
            r#"{"type":"AccumulationRegister","name":"Generated"}"#,
        )
        .unwrap();
        let args = Map::from_iter([
            (
                "JsonPath".to_string(),
                Value::String(definition.display().to_string()),
            ),
            (
                "OutputDir".to_string(),
                Value::String(root.join("src").display().to_string()),
            ),
        ]);

        assert!(matches!(
            evaluate_format_guard(spec("unica.meta.compile"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Allow
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_compile_guards_detached_configuration_owner() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-meta-compile-detached-owner-{}",
            std::process::id()
        ));
        let configuration = config(&root, Some("2.21"));
        let definition = root.join("catalog.json");
        std::fs::write(&definition, r#"{"type":"Catalog","name":"Generated"}"#).unwrap();
        let args = Map::from_iter([
            (
                "JsonPath".to_string(),
                Value::String(definition.display().to_string()),
            ),
            (
                "OutputDir".to_string(),
                Value::String(root.join("src").display().to_string()),
            ),
        ]);

        let descriptor = native_operation_descriptor("meta-compile").unwrap();
        let dependencies = effective_format_paths(descriptor, &args, &context(&root)).unwrap();
        assert!(dependencies.contains(&configuration), "{dependencies:?}");
        let check =
            evaluate_format_guard(spec("unica.meta.compile"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("detached Configuration.xml must participate in meta.compile preflight");
        };
        assert_eq!(diagnostic["actualFormat"], "2.21");
        assert_eq!(
            normalized_path(&std::path::PathBuf::from(
                diagnostic["root"].as_str().unwrap()
            )),
            normalized_path(&configuration)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_remove_guards_every_xml_content_read_but_not_probe_only_xml() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-meta-remove-read-set-{}",
            std::process::id()
        ));
        config(&root, Some("2.20"));
        let victim = root.join("src/Catalogs/Victim.xml");
        let referrer = root.join("src/Documents/Referrer.xml");
        let subsystem = root.join("src/Subsystems/Empty.xml");
        let probe_only = root.join("src/Subsystems/Empty/Ext/CommandInterface.xml");
        let dump_info = root.join("src/ConfigDumpInfo.xml");
        for path in [&victim, &referrer, &subsystem, &probe_only, &dump_info] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        std::fs::write(
            &victim,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog/></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &referrer,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Document/></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &subsystem,
            crate::infrastructure::native_operations::subsystem::child_subsystem_stub_xml(
                "Empty", "2.20",
            ),
        )
        .unwrap();
        std::fs::write(
            &probe_only,
            r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.21"/>"#,
        )
        .unwrap();
        std::fs::write(
            &dump_info,
            r#"<ConfigDumpInfo xmlns="http://v8.1c.ru/8.3/xcf/dumpinfo" version="2.21"/>"#,
        )
        .unwrap();
        let args = Map::from_iter([
            (
                "ConfigDir".to_string(),
                Value::String(root.join("src").display().to_string()),
            ),
            (
                "Object".to_string(),
                Value::String("Catalog.Victim".to_string()),
            ),
            ("Force".to_string(), Value::Bool(true)),
        ]);
        let descriptor = native_operation_descriptor("meta-remove").unwrap();
        let dependencies = effective_format_paths(descriptor, &args, &context(&root)).unwrap();

        assert!(dependencies.contains(&referrer), "{dependencies:?}");
        assert!(dependencies.contains(&subsystem), "{dependencies:?}");
        assert!(!dependencies.contains(&probe_only), "{dependencies:?}");
        assert!(!dependencies.contains(&dump_info), "{dependencies:?}");
        let check =
            evaluate_format_guard(spec("unica.meta.remove"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("newer XML read by reference scan must block meta.remove");
        };
        assert_eq!(diagnostic["actualFormat"], "2.21");
        assert_eq!(
            normalized_path(&std::path::PathBuf::from(
                diagnostic["root"].as_str().unwrap()
            )),
            normalized_path(&referrer)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_remove_guards_subsystem_descriptor_even_without_target_reference() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-meta-remove-subsystem-read-{}",
            std::process::id()
        ));
        config(&root, Some("2.20"));
        let victim = root.join("src/Catalogs/Victim.xml");
        let subsystem = root.join("src/Subsystems/Empty.xml");
        for path in [&victim, &subsystem] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        std::fs::write(
            &victim,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog/></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &subsystem,
            crate::infrastructure::native_operations::subsystem::child_subsystem_stub_xml(
                "Empty", "2.21",
            ),
        )
        .unwrap();
        let args = Map::from_iter([
            (
                "ConfigDir".to_string(),
                Value::String(root.join("src").display().to_string()),
            ),
            (
                "Object".to_string(),
                Value::String("Catalog.Victim".to_string()),
            ),
            ("Force".to_string(), Value::Bool(true)),
        ]);

        let check =
            evaluate_format_guard(spec("unica.meta.remove"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("every subsystem descriptor read by the planner must be guarded");
        };
        assert_eq!(diagnostic["actualFormat"], "2.21");
        assert_eq!(
            normalized_path(&std::path::PathBuf::from(
                diagnostic["root"].as_str().unwrap()
            )),
            normalized_path(&subsystem)
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
