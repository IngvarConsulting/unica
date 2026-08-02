#![allow(dead_code, unused_imports)]

use crate::application::AdapterOutcome;
use crate::domain::metadata::MetadataKind;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::metadata_kinds::{metadata_kind, metadata_layout};
use roxmltree::Document;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::super::cf::validate_cf_owner_path;
use super::super::common::{
    absolutize, detect_format_version, escape_xml, is_1c_identifier, json_i64_value, path_arg,
    read_utf8_sig, required_path, string_arg, FileBackedJson,
};
use super::super::compile_transaction::{CompileTransaction, RegistrationStatus};
use super::super::subsystem::validate_subsystem_owner_path;
use super::edit::{
    meta_edit_add_child_value, meta_edit_add_tabular_section_attribute,
    meta_edit_apply_complex_property_action, meta_edit_changes_to_inline,
    meta_edit_child_type_from_inline_target, meta_edit_child_type_key,
    meta_edit_complex_property_from_inline_target, meta_edit_definition_items,
    meta_edit_is_line_number_length_key, meta_edit_modify_object_properties_from_map,
    meta_edit_modify_object_properties_from_pairs, meta_edit_modify_tabular_section_attribute,
    meta_edit_modify_tabular_sections_from_definition, meta_edit_modify_top_child,
    meta_edit_object_node, meta_edit_operation_key, meta_edit_remove_child_value,
    meta_edit_remove_tabular_section_attribute, meta_edit_split_values, meta_edit_value_name,
    meta_edit_values_from_json, split_meta_edit_commas_outside_parens, MetaEditCounts,
    MetaEditLineNumberLengthPolicy,
};
use super::publisher::{
    fresh_meta_compile_uuid, prepare_meta_compile, preview_prepared_meta_compile,
    publish_meta_compile, register_compiled_meta_in_transaction,
};
use super::template_catalog::{
    emit_meta_accounting_register_properties, emit_meta_accumulation_register_properties,
    emit_meta_business_process_properties, emit_meta_calculation_register_properties,
    emit_meta_catalog_properties, emit_meta_chart_of_accounts_properties,
    emit_meta_chart_of_calculation_types_properties,
    emit_meta_chart_of_characteristic_types_properties, emit_meta_child_objects,
    emit_meta_common_module_properties, emit_meta_constant_properties,
    emit_meta_data_processor_properties, emit_meta_defined_type_properties,
    emit_meta_document_journal_properties, emit_meta_document_properties,
    emit_meta_enum_properties, emit_meta_event_subscription_properties,
    emit_meta_exchange_plan_properties, emit_meta_http_service_properties,
    emit_meta_information_register_properties, emit_meta_internal_info,
    emit_meta_report_properties, emit_meta_scheduled_job_properties, emit_meta_task_properties,
    emit_meta_web_service_properties, meta_compile_attributes, meta_compile_catalog_xml,
    meta_compile_enum_values, meta_compile_named_items, meta_compile_parse_attr,
    meta_compile_root_value_type, meta_compile_string_list, meta_compile_synonym,
    meta_compile_tabular_sections, meta_compile_value_items, meta_compile_value_types,
    meta_xmlns_decl, normalize_meta_enum_value, MetaCompileAttr, MetaCompileTabularSection,
    MetaTemplateDefinition,
};
use super::validation::{
    meta_validate_one_with_scope, meta_validate_property_values, meta_validate_valid_types,
    MetaValidationOptions, MetaValidationScope,
};
use super::xml_model::{
    meta_info_child, meta_info_child_text, validate_meta_resolved_type, validate_meta_type_union,
};

#[cfg(test)]
use super::{
    run_meta_compile_after_format_plan_hook, run_meta_compile_after_owner_validation_hook,
};

pub(super) const META_COMPILE_PENDING_TYPES: &[&str] = &[];

pub(super) enum MetaEditDslInput {
    File(PathBuf),
    Inline { operation: String, value: String },
}

pub(super) fn parse_meta_edit_dsl_input(
    args: &Map<String, Value>,
) -> Result<MetaEditDslInput, String> {
    let definition_file = path_arg(args, &["definitionFile", "DefinitionFile"]);
    let operation = string_arg(args, &["operation", "Operation"]);
    if definition_file.is_some() && operation.is_some() {
        return Err("Cannot use both -DefinitionFile and -Operation".to_string());
    }
    match (definition_file, operation) {
        (Some(definition_file), None) => Ok(MetaEditDslInput::File(definition_file)),
        (None, Some(operation)) => Ok(MetaEditDslInput::Inline {
            operation: operation.to_string(),
            value: string_arg(args, &["value", "Value"])
                .unwrap_or_default()
                .to_string(),
        }),
        (None, None) => Err("Either -DefinitionFile or -Operation is required".to_string()),
        (Some(_), Some(_)) => unreachable!("checked above"),
    }
}

pub(super) fn read_meta_edit_definition(
    definition_file: &Path,
    context: &WorkspaceContext,
    transaction: &mut CompileTransaction,
) -> Result<Value, String> {
    let definition_path = absolutize(definition_file.to_path_buf(), &context.cwd);
    if !definition_path.exists() {
        return Err(format!(
            "Definition file not found: {}",
            definition_file.display()
        ));
    }
    FileBackedJson::read(&definition_path, |err| {
        format!("DefinitionFile JSON parse error: {err}")
    })?
    .bind_to(transaction)
}

pub(super) fn meta_compile_type_plural(obj_type: &str) -> Option<&'static str> {
    MetadataKind::parse(obj_type)
        .ok()
        .map(|kind| metadata_layout(kind).directory)
}

fn meta_compile_supported_type_names() -> String {
    MetadataKind::ALL
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn meta_compile_uses_object_subdir(obj_type: &str) -> bool {
    !matches!(
        obj_type,
        "DefinedType" | "ScheduledJob" | "EventSubscription"
    )
}

pub(super) fn meta_compile_module_files(obj_type: &str) -> &'static [&'static str] {
    match obj_type {
        "Catalog"
        | "Document"
        | "ChartOfAccounts"
        | "ChartOfCharacteristicTypes"
        | "ChartOfCalculationTypes"
        | "BusinessProcess"
        | "Task"
        | "ExchangePlan" => &["ObjectModule.bsl"],
        "Enum" => &["ManagerModule.bsl"],
        "Constant" => &["ManagerModule.bsl", "ValueManagerModule.bsl"],
        "InformationRegister"
        | "AccumulationRegister"
        | "AccountingRegister"
        | "CalculationRegister" => &["RecordSetModule.bsl"],
        "Report" | "DataProcessor" => &["ObjectModule.bsl", "ManagerModule.bsl"],
        "CommonModule" | "HTTPService" | "WebService" => &["Module.bsl"],
        _ => &[],
    }
}

pub(super) fn meta_compile_extra_ext_files(
    obj_type: &str,
    format_version: &str,
) -> Vec<(&'static str, String)> {
    match obj_type {
        "ExchangePlan" => vec![(
            "Content.xml",
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<ExchangePlanContent xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"{format_version}\"/>\r\n"
            ),
        )],
        "BusinessProcess" => vec![(
            "Flowchart.xml",
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<GraphicalSchema xmlns=\"http://v8.1c.ru/8.3/xcf/scheme\" xmlns:sch=\"http://v8.1c.ru/8.2/data/graphscheme\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"{format_version}\">\r\n\
\t<BackColor>style:FieldBackColor</BackColor>\r\n\
\t<GridEnabled>true</GridEnabled>\r\n\
\t<DrawGridMode>Lines</DrawGridMode>\r\n\
\t<GridHorizontalStep>20</GridHorizontalStep>\r\n\
\t<GridVerticalStep>20</GridVerticalStep>\r\n\
\t<PrintParameters>\r\n\
\t\t<TopMargin>10</TopMargin>\r\n\
\t\t<LeftMargin>10</LeftMargin>\r\n\
\t\t<BottomMargin>10</BottomMargin>\r\n\
\t\t<RightMargin>10</RightMargin>\r\n\
\t\t<BlackAndWhite>false</BlackAndWhite>\r\n\
\t\t<FitPageMode>Auto</FitPageMode>\r\n\
\t</PrintParameters>\r\n\
\t<Items/>\r\n\
</GraphicalSchema>\r\n"
            ),
        )],
        _ => Vec::new(),
    }
}

pub(crate) fn meta_compile_format_dependency_paths(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<Vec<PathBuf>, String> {
    let output_dir_label = string_arg(args, &["outputDir", "OutputDir"])
        .ok_or_else(|| "missing required OutputDir argument".to_string())?;
    let output_dir = absolutize(PathBuf::from(output_dir_label), &context.cwd);
    let definition = read_meta_compile_definition(args, context)?;
    Ok(meta_compile_definition_format_dependency_paths(
        &definition,
        &output_dir,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MetaCompileEventSubscriptionDependency {
    subscription_name: String,
    subscription_descriptor_path: PathBuf,
    source_type: String,
    source_descriptor_path: PathBuf,
}

pub(super) fn meta_compile_event_subscription_dependencies(
    definition: &Value,
    output_dir: &Path,
) -> Vec<MetaCompileEventSubscriptionDependency> {
    let definitions = definition
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(definition));
    let mut dependencies = Vec::new();
    for definition in definitions {
        let Some(object) = definition.as_object() else {
            continue;
        };
        let Some(raw_type) = object
            .get("type")
            .or_else(|| object.get("objectType"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        if normalize_meta_object_type(raw_type) != "EventSubscription" {
            continue;
        }
        let Some(subscription_name) = object.get("name").and_then(Value::as_str) else {
            continue;
        };
        if validate_meta_compile_name("metadata object", subscription_name).is_err() {
            continue;
        }
        let subscription_descriptor_path = output_dir
            .join("EventSubscriptions")
            .join(format!("{subscription_name}.xml"));
        for raw_source in meta_compile_string_list(object.get("source")) {
            for source_type in raw_source
                .split('+')
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let resolved = resolve_meta_type(source_type);
                if validate_meta_resolved_type(source_type, &resolved).is_err() {
                    continue;
                }
                let Some((prefix, source_name)) = resolved.split_once('.') else {
                    continue;
                };
                let source_name = source_name.to_string();
                let source_object_type = match prefix {
                    "CatalogRef" | "CatalogObject" => "Catalog",
                    "DocumentRef" | "DocumentObject" => "Document",
                    "EnumRef" => "Enum",
                    "ChartOfAccountsRef" | "ChartOfAccountsObject" => "ChartOfAccounts",
                    "ChartOfCharacteristicTypesRef" | "ChartOfCharacteristicTypesObject" => {
                        "ChartOfCharacteristicTypes"
                    }
                    "ChartOfCalculationTypesRef" | "ChartOfCalculationTypesObject" => {
                        "ChartOfCalculationTypes"
                    }
                    "ExchangePlanRef" | "ExchangePlanObject" => "ExchangePlan",
                    "BusinessProcessRef" | "BusinessProcessObject" => "BusinessProcess",
                    "TaskRef" | "TaskObject" => "Task",
                    "ReportObject" => "Report",
                    "DataProcessorObject" => "DataProcessor",
                    "DefinedType" => "DefinedType",
                    _ => continue,
                };
                let Some(source_directory) =
                    metadata_kind(source_object_type).map(|kind| kind.directory)
                else {
                    continue;
                };
                dependencies.push(MetaCompileEventSubscriptionDependency {
                    subscription_name: subscription_name.to_string(),
                    subscription_descriptor_path: subscription_descriptor_path.clone(),
                    source_type: resolved,
                    source_descriptor_path: output_dir
                        .join(source_directory)
                        .join(format!("{source_name}.xml")),
                });
            }
        }
    }
    dependencies
}

pub(super) fn validate_meta_compile_event_subscription_dependencies(
    dependencies: &[MetaCompileEventSubscriptionDependency],
    transaction: &CompileTransaction,
) -> Result<(), String> {
    let planned_creates = transaction
        .planned_created_paths()
        .into_iter()
        .collect::<HashSet<_>>();
    for dependency in dependencies {
        if !planned_creates.contains(&dependency.subscription_descriptor_path)
            || planned_creates.contains(&dependency.source_descriptor_path)
        {
            continue;
        }
        match fs::symlink_metadata(&dependency.source_descriptor_path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(format!(
                    "EventSubscription '{}' source type '{}' requires a regular metadata descriptor at {}",
                    dependency.subscription_name,
                    dependency.source_type,
                    dependency.source_descriptor_path.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(format!(
                    "EventSubscription '{}' source type '{}' requires an existing or same-batch metadata descriptor at {}; 1C 8.3.27 rejects unknown source types",
                    dependency.subscription_name,
                    dependency.source_type,
                    dependency.source_descriptor_path.display()
                ));
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect EventSubscription '{}' source type '{}' descriptor {}: {error}",
                    dependency.subscription_name,
                    dependency.source_type,
                    dependency.source_descriptor_path.display()
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn meta_compile_definition_format_dependency_paths(
    definition: &Value,
    output_dir: &Path,
) -> Vec<PathBuf> {
    let definitions = definition
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(definition));
    let mut paths = vec![output_dir.join("Configuration.xml")];
    for definition in definitions {
        let Some(object) = definition.as_object() else {
            continue;
        };
        let Some(raw_type) = object
            .get("type")
            .or_else(|| object.get("objectType"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(name) = object.get("name").and_then(Value::as_str) else {
            continue;
        };
        if validate_meta_compile_name("metadata object", name).is_err() {
            continue;
        }
        let object_type = normalize_meta_object_type(raw_type);
        let Some(type_dir) = meta_compile_type_plural(&object_type) else {
            continue;
        };
        let target = output_dir.join(type_dir).join(name);
        let descriptor = target.with_extension("xml");
        let descriptor_exists = descriptor.is_file();
        paths.push(descriptor);
        if descriptor_exists {
            continue;
        }
        let ext_dir = target.join("Ext");
        for (file_name, _) in meta_compile_extra_ext_files(&object_type, "") {
            let path = ext_dir.join(file_name);
            if path.is_file() {
                paths.push(path);
            }
        }
    }
    paths.extend(
        meta_compile_event_subscription_dependencies(definition, output_dir)
            .into_iter()
            .map(|dependency| dependency.source_descriptor_path),
    );
    paths.sort();
    paths.dedup();
    paths
}

pub(super) fn compile_meta(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    publish_meta_compile(prepare_meta_compile(args, context), context)
}

pub(crate) fn preview_meta_compile(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<AdapterOutcome, String> {
    preview_prepared_meta_compile(prepare_meta_compile(args, context))
}

fn read_meta_compile_definition(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<Value, String> {
    let json_path_raw = required_path(args, &["jsonPath", "JsonPath"], "JsonPath")?;
    let json_path = absolutize(json_path_raw.clone(), &context.cwd);
    if !json_path.is_file() {
        return Err(format!("File not found: {}", json_path_raw.display()));
    }

    let json_text = fs::read_to_string(&json_path)
        .map_err(|err| format!("failed to read {}: {err}", json_path.display()))?;
    serde_json::from_str(json_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("failed to parse metadata JSON: {err}"))
}

pub(super) fn read_meta_compile_definition_guarded(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    transaction: &mut CompileTransaction,
) -> Result<Value, String> {
    let json_path_raw = required_path(args, &["jsonPath", "JsonPath"], "JsonPath")?;
    let json_path = absolutize(json_path_raw.clone(), &context.cwd);
    if !json_path.is_file() {
        return Err(format!("File not found: {}", json_path_raw.display()));
    }
    FileBackedJson::read(&json_path, |err| {
        format!("failed to parse metadata JSON: {err}")
    })?
    .bind_to(transaction)
}

pub(super) fn require_meta_configuration_owner_validation(
    config_path: &Path,
    context: &WorkspaceContext,
    operation: &str,
) -> Result<(), String> {
    validate_cf_owner_path(config_path, context).map_err(|detail| {
        format!(
            "{operation} Configuration owner validation failed for {}: {}",
            config_path.display(),
            detail.trim()
        )
    })
}

pub(super) fn compile_meta_value(
    defn: Value,
    output_dir_label: &str,
    output_dir: &Path,
    context: &WorkspaceContext,
    transaction: &mut CompileTransaction,
    format_dependencies: &mut Vec<PathBuf>,
) -> Result<(String, Vec<PathBuf>), String> {
    match defn {
        Value::Array(items) => compile_meta_batch(
            items,
            output_dir_label,
            output_dir,
            context,
            transaction,
            format_dependencies,
        ),
        single => compile_meta_object(
            single,
            output_dir_label,
            output_dir,
            context,
            transaction,
            format_dependencies,
        ),
    }
}

fn compile_meta_batch(
    items: Vec<Value>,
    output_dir_label: &str,
    output_dir: &Path,
    context: &WorkspaceContext,
    transaction: &mut CompileTransaction,
    format_dependencies: &mut Vec<PathBuf>,
) -> Result<(String, Vec<PathBuf>), String> {
    let total = items.len();
    let mut stdout = String::new();
    let mut artifacts = Vec::<PathBuf>::new();
    let mut failed = Vec::<String>::new();

    for (index, item) in items.into_iter().enumerate() {
        match compile_meta_object(
            item,
            output_dir_label,
            output_dir,
            context,
            transaction,
            format_dependencies,
        ) {
            Ok((item_stdout, mut item_artifacts)) => {
                stdout.push_str(&item_stdout);
                artifacts.append(&mut item_artifacts);
            }
            Err(error) => {
                failed.push(format!("#{}: {error}", index + 1));
                stdout.push_str(&format!("[FAIL] #{}: {error}\n", index + 1));
            }
        }
    }

    let compiled = total.saturating_sub(failed.len());
    stdout.push_str(&format!(
        "\n=== Batch: {total} objects, {compiled} compiled, {} failed ===\n",
        failed.len()
    ));

    if failed.is_empty() {
        Ok((stdout, artifacts))
    } else {
        Err(failed.join("\n"))
    }
}

fn compile_meta_object(
    mut defn: Value,
    output_dir_label: &str,
    output_dir: &Path,
    context: &WorkspaceContext,
    transaction: &mut CompileTransaction,
    format_dependencies: &mut Vec<PathBuf>,
) -> Result<(String, Vec<PathBuf>), String> {
    if defn.get("type").is_none() {
        if let Some(object_type) = defn.get("objectType").cloned() {
            defn.as_object_mut()
                .ok_or_else(|| "metadata JSON must be an object".to_string())?
                .insert("type".to_string(), object_type);
        }
    }
    let object = defn
        .as_object()
        .ok_or_else(|| "metadata JSON must be an object".to_string())?;
    let raw_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "JSON must have 'type' field".to_string())?;
    let obj_type = normalize_meta_object_type(raw_type);
    let type_plural = meta_compile_type_plural(&obj_type).ok_or_else(|| {
        format!(
            "Unsupported type: {obj_type}. Supported: {}. Documented pending: {}",
            meta_compile_supported_type_names(),
            META_COMPILE_PENDING_TYPES.join(", ")
        )
    })?;
    let obj_name = object
        .get("name")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "JSON must have 'name' field".to_string())?;
    validate_meta_compile_name("metadata object", obj_name)?;
    let type_dir = output_dir.join(type_plural);
    let main_xml_path = type_dir.join(format!("{obj_name}.xml"));
    let obj_sub_dir = type_dir.join(obj_name);
    let ext_dir = obj_sub_dir.join("Ext");

    match fs::symlink_metadata(&main_xml_path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            return Ok((
                format!(
                    "[SKIP] {obj_type} '{obj_name}' already exists at {}; no files changed\n",
                    main_xml_path.display()
                ),
                Vec::new(),
            ));
        }
        Ok(_) => {
            return Err(format!(
                "existing metadata target is not a regular file: {}",
                main_xml_path.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "failed to inspect metadata target {}: {error}",
                main_xml_path.display()
            ));
        }
    }
    let format_version = detect_format_version(output_dir, context)?.to_string();
    let (metadata_xml, uid) =
        meta_compile_object_xml(object, &obj_type, obj_name, &format_version)?;
    transaction.create_utf8_bom_text(&main_xml_path, &metadata_xml)?;

    let mut artifacts = vec![main_xml_path.clone()];
    let mut modules_created = Vec::<PathBuf>::new();
    for module_name in meta_compile_module_files(&obj_type) {
        let module_path = ext_dir.join(module_name);
        if !module_path.is_file() {
            transaction.create_utf8_bom_text(&module_path, "")?;
            modules_created.push(module_path.clone());
            artifacts.push(module_path.clone());
        }
    }
    for (file_name, content) in meta_compile_extra_ext_files(&obj_type, &format_version) {
        let file_path = ext_dir.join(file_name);
        match fs::symlink_metadata(&file_path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                transaction.create_utf8_bom_text(&file_path, &content)?;
                modules_created.push(file_path.clone());
                artifacts.push(file_path.clone());
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect metadata extra target {}: {error}",
                    file_path.display()
                ));
            }
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                let snapshot = fs::read(&file_path).map_err(|error| {
                    format!(
                        "failed to read metadata extra target {}: {error}",
                        file_path.display()
                    )
                })?;
                transaction.guard_or_verify_exact_preimage(&file_path, &snapshot)?;
                format_dependencies.push(file_path);
            }
            Ok(_) => {
                return Err(format!(
                    "existing metadata extra target is not a regular file: {}",
                    file_path.display()
                ));
            }
        }
    }

    let reg_result =
        register_compiled_meta_in_transaction(transaction, output_dir, &obj_type, obj_name)?;

    let attr_count = object
        .get("attributes")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let ts_count = object
        .get("tabularSections")
        .map(meta_compile_collection_count)
        .unwrap_or(0);
    let enum_value_count = object
        .get("values")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let dim_count = object
        .get("dimensions")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let res_count = object
        .get("resources")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let column_count = object
        .get("columns")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let mut stdout = format!(
        "[OK] {obj_type} '{obj_name}' compiled\n     UUID: {uid}\n     File: {}/{type_plural}/{obj_name}.xml\n",
        output_dir_label.trim_end_matches(['/', '\\'])
    );
    let mut details = Vec::new();
    if attr_count > 0 {
        details.push(format!("Attributes: {attr_count}"));
    }
    if ts_count > 0 {
        details.push(format!("TabularSections: {ts_count}"));
    }
    if enum_value_count > 0 {
        details.push(format!("Values: {enum_value_count}"));
    }
    if dim_count > 0 {
        details.push(format!("Dimensions: {dim_count}"));
    }
    if res_count > 0 {
        details.push(format!("Resources: {res_count}"));
    }
    if column_count > 0 {
        details.push(format!("Columns: {column_count}"));
    }
    if !details.is_empty() {
        stdout.push_str(&format!("     {}\n", details.join(", ")));
    }
    for module in modules_created {
        stdout.push_str(&format!(
            "     Module: {}/{type_plural}/{obj_name}/Ext/{}\n",
            output_dir_label.trim_end_matches(['/', '\\']),
            module
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("ObjectModule.bsl")
        ));
    }
    match reg_result {
        RegistrationStatus::Added => stdout.push_str(&format!(
            "     Configuration.xml: <{obj_type}>{obj_name}</{obj_type}> added to ChildObjects\n"
        )),
        RegistrationStatus::AlreadyPresent => stdout.push_str(&format!(
            "     Configuration.xml: <{obj_type}>{obj_name}</{obj_type}> already registered\n"
        )),
        RegistrationStatus::MissingTarget => stdout.push_str(&format!(
            "     Configuration.xml: not found at {}/Configuration.xml (register manually)\n",
            output_dir_label.trim_end_matches(['/', '\\'])
        )),
    }

    Ok((stdout, artifacts))
}

pub(super) fn meta_compile_collection_count(value: &Value) -> usize {
    value
        .as_array()
        .map(Vec::len)
        .or_else(|| value.as_object().map(Map::len))
        .unwrap_or(0)
}

pub(super) fn normalize_meta_object_type(raw: &str) -> String {
    match raw {
        "Справочник" | "Каталог" => "Catalog",
        "Документ" => "Document",
        "Перечисление" => "Enum",
        "Константа" => "Constant",
        "РегистрСведений" => "InformationRegister",
        "РегистрНакопления" => "AccumulationRegister",
        "РегистрБухгалтерии" => "AccountingRegister",
        "РегистрРасчёта" | "РегистрРасчета" => "CalculationRegister",
        "ПланСчетов" => "ChartOfAccounts",
        "ПланВидовХарактеристик" => "ChartOfCharacteristicTypes",
        "ПланВидовРасчёта" | "ПланВидовРасчета" => {
            "ChartOfCalculationTypes"
        }
        "БизнесПроцесс" => "BusinessProcess",
        "Задача" => "Task",
        "ПланОбмена" => "ExchangePlan",
        "ЖурналДокументов" => "DocumentJournal",
        "Отчёт" | "Отчет" => "Report",
        "Обработка" => "DataProcessor",
        "ОбщийМодуль" => "CommonModule",
        "РегламентноеЗадание" => "ScheduledJob",
        "ПодпискаНаСобытие" => "EventSubscription",
        "HTTPСервис" => "HTTPService",
        "ВебСервис" => "WebService",
        "ОпределяемыйТип" => "DefinedType",
        other => other,
    }
    .to_string()
}

pub(crate) fn meta_compile_object_xml(
    defn: &Map<String, Value>,
    obj_type: &str,
    obj_name: &str,
    format_version: &str,
) -> Result<(String, String), String> {
    validate_meta_compile_name("metadata object", obj_name)?;
    validate_meta_compile_type_contract(defn, obj_type)?;
    if obj_type == "Catalog" {
        return meta_compile_catalog_xml(defn, obj_name, format_version);
    }

    let template = MetaTemplateDefinition::legacy(defn);
    let mut next_uuid = fresh_meta_compile_uuid;
    let obj_uuid = next_uuid();
    let synonym = meta_compile_synonym(&template, obj_name);

    let mut lines = Vec::<String>::new();
    lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
    lines.push(format!(
        "<MetaDataObject {} version=\"{}\">",
        meta_xmlns_decl(),
        escape_xml(format_version)
    ));
    lines.push(format!("\t<{obj_type} uuid=\"{obj_uuid}\">"));
    emit_meta_internal_info(&mut lines, "\t\t", obj_type, obj_name, &mut next_uuid);
    lines.push("\t\t<Properties>".to_string());
    match obj_type {
        "Document" => {
            emit_meta_document_properties(&mut lines, "\t\t\t", &template, obj_name, &synonym)
        }
        "Enum" => emit_meta_enum_properties(&mut lines, "\t\t\t", &template, obj_name, &synonym),
        "Constant" => {
            emit_meta_constant_properties(&mut lines, "\t\t\t", &template, obj_name, &synonym)
        }
        "InformationRegister" => emit_meta_information_register_properties(
            &mut lines, "\t\t\t", &template, obj_name, &synonym,
        ),
        "AccumulationRegister" => emit_meta_accumulation_register_properties(
            &mut lines, "\t\t\t", &template, obj_name, &synonym,
        ),
        "AccountingRegister" => emit_meta_accounting_register_properties(
            &mut lines, "\t\t\t", &template, obj_name, &synonym,
        ),
        "CalculationRegister" => emit_meta_calculation_register_properties(
            &mut lines, "\t\t\t", &template, obj_name, &synonym,
        ),
        "ChartOfAccounts" => emit_meta_chart_of_accounts_properties(
            &mut lines, "\t\t\t", &template, obj_name, &synonym,
        ),
        "ChartOfCharacteristicTypes" => emit_meta_chart_of_characteristic_types_properties(
            &mut lines, "\t\t\t", &template, obj_name, &synonym,
        ),
        "ChartOfCalculationTypes" => emit_meta_chart_of_calculation_types_properties(
            &mut lines, "\t\t\t", &template, obj_name, &synonym,
        ),
        "BusinessProcess" => emit_meta_business_process_properties(
            &mut lines, "\t\t\t", &template, obj_name, &synonym,
        ),
        "Task" => emit_meta_task_properties(&mut lines, "\t\t\t", &template, obj_name, &synonym),
        "ExchangePlan" => {
            emit_meta_exchange_plan_properties(&mut lines, "\t\t\t", &template, obj_name, &synonym)
        }
        "DocumentJournal" => emit_meta_document_journal_properties(
            &mut lines, "\t\t\t", &template, obj_name, &synonym,
        ),
        "Report" => {
            emit_meta_report_properties(&mut lines, "\t\t\t", &template, obj_name, &synonym)
        }
        "DataProcessor" => {
            emit_meta_data_processor_properties(&mut lines, "\t\t\t", &template, obj_name, &synonym)
        }
        "CommonModule" => {
            emit_meta_common_module_properties(&mut lines, "\t\t\t", &template, obj_name, &synonym)
        }
        "ScheduledJob" => {
            emit_meta_scheduled_job_properties(&mut lines, "\t\t\t", &template, obj_name, &synonym)
        }
        "EventSubscription" => emit_meta_event_subscription_properties(
            &mut lines, "\t\t\t", &template, obj_name, &synonym,
        ),
        "HTTPService" => {
            emit_meta_http_service_properties(&mut lines, "\t\t\t", &template, obj_name, &synonym)
        }
        "WebService" => {
            emit_meta_web_service_properties(&mut lines, "\t\t\t", &template, obj_name, &synonym)
        }
        "DefinedType" => {
            emit_meta_defined_type_properties(&mut lines, "\t\t\t", &template, obj_name, &synonym)
        }
        _ => {
            return Err(format!(
                "Unsupported type: {obj_type}. Supported: {}. Documented pending: {}",
                meta_compile_supported_type_names(),
                META_COMPILE_PENDING_TYPES.join(", ")
            ));
        }
    }
    lines.push("\t\t</Properties>".to_string());

    emit_meta_child_objects(
        &mut lines,
        "\t\t",
        &template,
        obj_type,
        obj_name,
        &mut next_uuid,
    )?;

    lines.push(format!("\t</{obj_type}>"));
    lines.push("</MetaDataObject>".to_string());
    Ok((format!("{}\n", lines.join("\n")), obj_uuid))
}

fn validate_meta_compile_type_contract(
    defn: &Map<String, Value>,
    obj_type: &str,
) -> Result<(), String> {
    validate_meta_compile_enum_properties(defn, obj_type)?;
    for field_name in ["attributes", "dimensions", "resources"] {
        for attr in meta_compile_attributes(defn.get(field_name)) {
            validate_meta_compile_attr_type(&attr, field_name)?;
        }
    }
    for section in meta_compile_tabular_sections(defn.get("tabularSections"))? {
        validate_meta_compile_name("tabularSections", &section.name)?;
        for attr in section.columns {
            validate_meta_compile_attr_type(&attr, "tabularSections")?;
        }
    }
    if obj_type == "Task" {
        for value in meta_compile_value_items(defn.get("addressingAttributes")) {
            let attr = meta_compile_parse_attr(&value);
            validate_meta_compile_attr_type(&attr, "addressingAttributes")?;
        }
    }
    if obj_type == "Enum" {
        for value in meta_compile_enum_values(defn.get("values"))? {
            validate_meta_compile_name("enum value", &value.name)?;
        }
    }
    if obj_type == "ChartOfAccounts" {
        for name in meta_compile_named_items(defn.get("accountingFlags")) {
            validate_meta_compile_name("accounting flag", &name)?;
        }
        for name in meta_compile_named_items(defn.get("extDimensionAccountingFlags")) {
            validate_meta_compile_name("ext-dimension accounting flag", &name)?;
        }
    }
    if obj_type == "DocumentJournal" {
        for value in meta_compile_value_items(defn.get("columns")) {
            let name = meta_edit_value_name(&value).unwrap_or_default();
            validate_meta_compile_name("document journal column", &name)?;
            if let Some(indexing) = value
                .as_object()
                .and_then(|object| object.get("indexing"))
                .and_then(Value::as_str)
            {
                validate_meta_8_3_27_property_value(
                    "document journal column",
                    "Indexing",
                    indexing,
                )?;
            }
        }
    }
    if obj_type == "HTTPService" {
        if let Some(templates) = defn.get("urlTemplates").and_then(Value::as_object) {
            for (template_name, template_value) in templates {
                validate_meta_compile_name("URL template", template_name)?;
                if let Some(methods) = template_value
                    .as_object()
                    .and_then(|object| object.get("methods"))
                    .and_then(Value::as_object)
                {
                    for method_name in methods.keys() {
                        validate_meta_compile_name("HTTP method", method_name)?;
                    }
                }
            }
        }
    }
    if obj_type == "WebService" {
        if let Some(operations) = defn.get("operations").and_then(Value::as_object) {
            for (operation_name, operation_value) in operations {
                validate_meta_compile_name("web service operation", operation_name)?;
                if let Some(parameters) = operation_value
                    .as_object()
                    .and_then(|object| object.get("parameters"))
                    .and_then(Value::as_object)
                {
                    for parameter_name in parameters.keys() {
                        validate_meta_compile_name("operation parameter", parameter_name)?;
                    }
                }
            }
        }
    }
    if obj_type == "Constant" {
        let value_type = meta_compile_root_value_type(&MetaTemplateDefinition::legacy(defn));
        validate_meta_type_union(std::iter::once(value_type.as_str()))?;
    }
    if obj_type == "EventSubscription" {
        let sources = meta_compile_string_list(defn.get("source"));
        validate_meta_type_union(sources.iter().map(String::as_str))?;
    }
    if matches!(obj_type, "ChartOfCharacteristicTypes" | "DefinedType") {
        let value_types = meta_compile_value_types(&MetaTemplateDefinition::legacy(defn));
        validate_meta_type_union(value_types.iter().map(String::as_str))?;
    }
    if obj_type == "ChartOfAccounts" {
        let max_count = defn
            .get("maxExtDimensionCount")
            .and_then(json_i64_value)
            .unwrap_or(0);
        let has_type = defn
            .get("extDimensionTypes")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        if max_count > 0 && !has_type {
            return Err(
                "ChartOfAccounts maxExtDimensionCount > 0 requires non-empty extDimensionTypes on 8.3.27"
                    .to_string(),
            );
        }
    }
    Ok(())
}

pub(super) fn validate_meta_compile_name(context: &str, name: &str) -> Result<(), String> {
    if is_1c_identifier(name) {
        Ok(())
    } else {
        Err(format!(
            "{context} name '{name}' is not a valid 1C identifier"
        ))
    }
}

fn validate_meta_compile_enum_properties(
    defn: &Map<String, Value>,
    obj_type: &str,
) -> Result<(), String> {
    for (field_name, property_name) in [
        ("codeType", "CodeType"),
        ("codeAllowedLength", "CodeAllowedLength"),
        ("numberType", "NumberType"),
        ("numberAllowedLength", "NumberAllowedLength"),
        ("posting", "Posting"),
        ("realTimePosting", "RealTimePosting"),
        ("registerRecordsDeletion", "RegisterRecordsDeletion"),
        (
            "registerRecordsWritingOnPost",
            "RegisterRecordsWritingOnPost",
        ),
        ("dataLockControlMode", "DataLockControlMode"),
        ("fullTextSearch", "FullTextSearch"),
        ("defaultPresentation", "DefaultPresentation"),
        ("hierarchyType", "HierarchyType"),
        ("editType", "EditType"),
        ("writeMode", "WriteMode"),
        ("registerType", "RegisterType"),
        ("returnValuesReuse", "ReturnValuesReuse"),
        ("reuseSessions", "ReuseSessions"),
        (
            "dependenceOnCalculationTypes",
            "DependenceOnCalculationTypes",
        ),
    ] {
        validate_meta_compile_enum_field(defn, field_name, property_name)?;
    }
    if obj_type == "InformationRegister" {
        validate_meta_compile_enum_field(defn, "periodicity", "InformationRegisterPeriodicity")?;
    }
    match obj_type {
        "Catalog" => {
            validate_meta_compile_enum_field(defn, "subordinationUse", "SubordinationUse")?;
            validate_meta_compile_enum_field(defn, "codeSeries", "CatalogCodeSeries")?;
            validate_meta_compile_enum_field(defn, "choiceMode", "ChoiceMode")?;
        }
        "ChartOfAccounts" => {
            validate_meta_compile_enum_field(defn, "codeSeries", "ChartOfAccountsCodeSeries")?;
        }
        "ChartOfCharacteristicTypes" => {
            validate_meta_compile_enum_field(defn, "codeSeries", "CharacteristicTypeCodeSeries")?;
            validate_meta_compile_enum_field(defn, "predefinedDataUpdate", "PredefinedDataUpdate")?;
            validate_meta_compile_enum_field(defn, "choiceMode", "ChoiceMode")?;
        }
        "Document" => {
            validate_meta_compile_enum_field(
                defn,
                "numberPeriodicity",
                "DocumentNumberPeriodicity",
            )?;
        }
        "BusinessProcess" => {
            validate_meta_compile_enum_field(
                defn,
                "numberPeriodicity",
                "BusinessProcessNumberPeriodicity",
            )?;
        }
        "CalculationRegister" => {
            validate_meta_compile_enum_field(
                defn,
                "periodicity",
                "CalculationRegisterPeriodicity",
            )?;
        }
        "ExchangePlan" => {
            validate_meta_compile_enum_field(defn, "choiceMode", "ChoiceMode")?;
        }
        "HTTPService" => validate_meta_compile_http_methods(defn)?,
        "WebService" => validate_meta_compile_transfer_directions(defn)?,
        _ => {}
    }
    Ok(())
}

fn validate_meta_compile_http_methods(defn: &Map<String, Value>) -> Result<(), String> {
    let Some(templates) = defn.get("urlTemplates").and_then(Value::as_object) else {
        return Ok(());
    };
    for template in templates.values() {
        let Some(methods) = template
            .as_object()
            .and_then(|object| object.get("methods"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        for method in methods.values() {
            let value = method.as_str().ok_or_else(|| {
                "meta.compile property HTTPMethod must be a string for the fixed 8.3.27 contract"
                    .to_string()
            })?;
            validate_meta_8_3_27_property_value("meta.compile", "HTTPMethod", value)?;
        }
    }
    Ok(())
}

fn validate_meta_compile_transfer_directions(defn: &Map<String, Value>) -> Result<(), String> {
    let Some(operations) = defn.get("operations").and_then(Value::as_object) else {
        return Ok(());
    };
    for operation in operations.values() {
        let Some(parameters) = operation
            .as_object()
            .and_then(|object| object.get("parameters"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        for parameter in parameters.values() {
            let Some(direction) = parameter
                .as_object()
                .and_then(|object| object.get("direction"))
            else {
                continue;
            };
            let value = direction.as_str().ok_or_else(|| {
                "meta.compile property TransferDirection must be a string for the fixed 8.3.27 contract"
                    .to_string()
            })?;
            validate_meta_8_3_27_property_value("meta.compile", "TransferDirection", value)?;
        }
    }
    Ok(())
}

fn validate_meta_compile_enum_field(
    defn: &Map<String, Value>,
    field_name: &str,
    property_name: &str,
) -> Result<(), String> {
    let Some(value) = defn.get(field_name) else {
        return Ok(());
    };
    let raw_value = value.as_str().ok_or_else(|| {
        format!(
            "meta.compile property {property_name} must be a string for the fixed 8.3.27 contract"
        )
    })?;
    validate_meta_8_3_27_property_value("meta.compile", property_name, raw_value)
}

pub(super) fn validate_meta_8_3_27_property_value(
    context: &str,
    property_name: &str,
    raw_value: &str,
) -> Result<(), String> {
    let Some((_, allowed_values)) = meta_validate_property_values()
        .iter()
        .find(|(known_property, _)| *known_property == property_name)
    else {
        return Ok(());
    };
    let normalized = normalize_meta_enum_value(raw_value);
    if allowed_values.contains(&normalized.as_str()) {
        Ok(())
    } else {
        Err(format!(
            "{context} property {property_name} value '{normalized}' is not valid for 8.3.27; expected one of: {}",
            allowed_values.join(", ")
        ))
    }
}

pub(super) fn meta_8_3_27_boolean_properties(object_type: &str) -> &'static [&'static str] {
    match object_type {
        "AccountingFlag" | "AddressingAttribute" | "Attribute" | "ExtDimensionAccountingFlag" => &[
            "PasswordMode",
            "MarkNegatives",
            "MultiLine",
            "ExtendedEdit",
            "FillFromFillingValue",
        ],
        "AccountingRegister" => &[
            "UseStandardCommands",
            "IncludeHelpInContents",
            "Correspondence",
            "EnableTotalsSplitting",
        ],
        "AccumulationRegister" => &[
            "UseStandardCommands",
            "IncludeHelpInContents",
            "EnableTotalsSplitting",
        ],
        "BusinessProcess" => &[
            "UseStandardCommands",
            "CheckUnique",
            "Autonumbering",
            "CreateTaskInPrivilegedMode",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "CalculationRegister" => &[
            "UseStandardCommands",
            "ActionPeriod",
            "BasePeriod",
            "IncludeHelpInContents",
        ],
        "Catalog" => &[
            "Hierarchical",
            "LimitLevelCount",
            "FoldersOnTop",
            "UseStandardCommands",
            "CheckUnique",
            "Autonumbering",
            "QuickChoice",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "ChartOfAccounts" => &[
            "UseStandardCommands",
            "IncludeHelpInContents",
            "CheckUnique",
            "QuickChoice",
            "AutoOrderByCode",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "ChartOfCalculationTypes" => &[
            "UseStandardCommands",
            "QuickChoice",
            "ActionPeriodUse",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "ChartOfCharacteristicTypes" => &[
            "UseStandardCommands",
            "IncludeHelpInContents",
            "Hierarchical",
            "FoldersOnTop",
            "CheckUnique",
            "Autonumbering",
            "QuickChoice",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "Command" => &["ModifiesData"],
        "CommonModule" => &[
            "Global",
            "ClientManagedApplication",
            "Server",
            "ExternalConnection",
            "ClientOrdinaryApplication",
            "Client",
            "ServerCall",
            "Privileged",
        ],
        "Constant" => &[
            "UseStandardCommands",
            "PasswordMode",
            "MarkNegatives",
            "MultiLine",
            "ExtendedEdit",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "DataProcessor" | "DocumentJournal" | "Report" => {
            &["UseStandardCommands", "IncludeHelpInContents"]
        }
        "Dimension" => &[
            "PasswordMode",
            "MarkNegatives",
            "MultiLine",
            "ExtendedEdit",
            "DenyIncompleteValues",
            "BaseDimension",
            "UseInTotals",
            "FillFromFillingValue",
            "Master",
            "MainFilter",
            "Balance",
        ],
        "Document" => &[
            "UseStandardCommands",
            "CheckUnique",
            "Autonumbering",
            "PostInPrivilegedMode",
            "UnpostInPrivilegedMode",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "Enum" => &["UseStandardCommands", "QuickChoice"],
        "ExchangePlan" => &[
            "UseStandardCommands",
            "QuickChoice",
            "DistributedInfoBase",
            "IncludeConfigurationExtensions",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "InformationRegister" => &[
            "UseStandardCommands",
            "MainFilterOnPeriod",
            "IncludeHelpInContents",
            "EnableTotalsSliceFirst",
            "EnableTotalsSliceLast",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "Operation" => &["Nillable", "Transactioned"],
        "Parameter" => &["Nillable"],
        "Resource" => &[
            "PasswordMode",
            "MarkNegatives",
            "MultiLine",
            "ExtendedEdit",
            "Balance",
            "FillFromFillingValue",
        ],
        "ScheduledJob" => &["Use", "Predefined"],
        "Task" => &[
            "UseStandardCommands",
            "CheckUnique",
            "Autonumbering",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        _ => &[],
    }
}

pub(super) fn validate_meta_8_3_27_boolean_property_value(
    context: &str,
    object_type: &str,
    property_name: &str,
    value: &str,
) -> Result<(), String> {
    if !meta_8_3_27_boolean_properties(object_type).contains(&property_name) {
        return Ok(());
    }
    if matches!(value, "true" | "false") {
        Ok(())
    } else {
        Err(format!(
            "{context} property {object_type}.{property_name} value '{value}' is not a canonical xs:boolean for the fixed 8.3.27 contract; expected true or false"
        ))
    }
}

pub(super) fn validate_metadata_8_3_27_boolean_contract(
    xml_text: &str,
    context: &str,
) -> Result<(), String> {
    let document = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("XML parse error: {error}"))?;
    let root_object = meta_edit_object_node(&document)?;
    if !meta_validate_valid_types().contains(&root_object.tag_name().name()) {
        return Ok(());
    }

    for object in root_object
        .descendants()
        .filter(roxmltree::Node::is_element)
    {
        let object_type = object.tag_name().name();
        let boolean_properties = meta_8_3_27_boolean_properties(object_type);
        if boolean_properties.is_empty() {
            continue;
        }
        let Some(properties) = meta_info_child(object, "Properties") else {
            continue;
        };
        for property in properties.children().filter(roxmltree::Node::is_element) {
            let property_name = property.tag_name().name();
            if boolean_properties.contains(&property_name) {
                validate_meta_8_3_27_boolean_property_value(
                    context,
                    object_type,
                    property_name,
                    property.text().unwrap_or(""),
                )?;
            }
        }
    }

    Ok(())
}

pub(super) fn validate_metadata_8_3_27_enum_contract(
    xml_text: &str,
    context: &str,
) -> Result<(), String> {
    let document = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("XML parse error: {error}"))?;
    let root_object = meta_edit_object_node(&document)?;
    if !meta_validate_valid_types().contains(&root_object.tag_name().name()) {
        return Ok(());
    }

    for object in root_object
        .descendants()
        .filter(roxmltree::Node::is_element)
    {
        let Some(properties) = meta_info_child(object, "Properties") else {
            continue;
        };
        for (property_name, allowed) in meta_validate_property_values() {
            let Some(value) =
                meta_info_child_text(properties, property_name).filter(|value| !value.is_empty())
            else {
                continue;
            };
            if !allowed.contains(&value.as_str()) {
                return Err(format!(
                    "{context} property {}.{property_name} value '{value}' is not valid for the fixed 8.3.27 contract; expected one of: {}",
                    object.tag_name().name(),
                    allowed.join(", ")
                ));
            }
        }
    }

    Ok(())
}

pub(crate) fn validate_metadata_owner_shape_8_3_27(
    object_path: &Path,
    workspace: &WorkspaceContext,
    operation: &str,
) -> Result<(), String> {
    let xml_text = read_utf8_sig(object_path)?;
    let document = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("XML parse error: {error}"))?;
    let root_object = meta_edit_object_node(&document)?;
    match root_object.tag_name().name() {
        "Configuration" => return validate_cf_owner_path(object_path, workspace),
        "Subsystem" => return validate_subsystem_owner_path(object_path, workspace),
        _ => {}
    }
    validate_metadata_8_3_27_boolean_contract(&xml_text, operation)?;
    validate_metadata_8_3_27_enum_contract(&xml_text, operation)?;

    let options = MetaValidationOptions {
        detailed: true,
        max_errors: 30,
    };
    let run = meta_validate_one_with_scope(
        object_path.to_path_buf(),
        &options,
        workspace,
        MetaValidationScope::PostWriteLocal,
    )?;
    if run.ok {
        Ok(())
    } else {
        Err(format!(
            "{operation} owner metadata validation failed for {}: {}",
            object_path.display(),
            run.errors.join("; ")
        ))
    }
}

pub(super) fn validate_meta_compile_attr_type(
    attr: &MetaCompileAttr,
    context: &str,
) -> Result<(), String> {
    validate_meta_compile_name(context, &attr.name)?;
    if !attr.fill_checking.is_empty() {
        validate_meta_8_3_27_property_value(context, "FillChecking", &attr.fill_checking)?;
    }
    if !attr.indexing.is_empty() {
        validate_meta_8_3_27_property_value(context, "Indexing", &attr.indexing)?;
    }
    if attr.type_name.trim().is_empty() {
        return Ok(());
    }
    validate_meta_type_union(std::iter::once(attr.type_name.as_str())).map_err(|error| {
        format!(
            "invalid 8.3.27 type for {context} attribute {}: {error}",
            attr.name
        )
    })
}

pub(super) fn validate_meta_compile_tabular_section_types(
    section: &MetaCompileTabularSection,
    context: &str,
) -> Result<(), String> {
    validate_meta_compile_name(context, &section.name)?;
    for attr in &section.columns {
        validate_meta_compile_attr_type(attr, context)?;
    }
    Ok(())
}

pub(super) fn split_meta_edit_batch_items<'a>(
    raw_value: &'a str,
    operation: &str,
) -> Result<Vec<&'a str>, String> {
    let items = raw_value
        .split(";;")
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    if items.is_empty() {
        return Err(format!("{operation} requires non-empty Value"));
    }
    Ok(items)
}

pub(super) fn meta_compile_is_config_type(type_name: &str) -> bool {
    [
        "CatalogRef.",
        "CatalogObject.",
        "DocumentRef.",
        "DocumentObject.",
        "EnumRef.",
        "ChartOfAccountsRef.",
        "ChartOfAccountsObject.",
        "ChartOfCharacteristicTypesRef.",
        "ChartOfCharacteristicTypesObject.",
        "ChartOfCalculationTypesRef.",
        "ChartOfCalculationTypesObject.",
        "ExchangePlanRef.",
        "ExchangePlanObject.",
        "BusinessProcessRef.",
        "BusinessProcessObject.",
        "TaskRef.",
        "TaskObject.",
        "ReportObject.",
        "DataProcessorObject.",
        "DefinedType.",
    ]
    .iter()
    .any(|prefix| type_name.starts_with(prefix))
}

pub(super) fn resolve_meta_type(type_name: &str) -> String {
    if let Some(open) = type_name.find('(') {
        if type_name.ends_with(')') {
            let base = type_name[..open].trim();
            let params = &type_name[open + 1..type_name.len() - 1];
            if let Some(resolved) = meta_type_synonym(base) {
                return format!("{resolved}({params})");
            }
        }
    }
    if let Some(dot) = type_name.find('.') {
        let prefix = &type_name[..dot];
        let suffix = &type_name[dot..];
        if let Some(resolved) = meta_type_synonym(prefix) {
            return format!("{resolved}{suffix}");
        }
    }
    meta_type_synonym(type_name)
        .unwrap_or(type_name)
        .to_string()
}

pub(super) fn meta_type_synonym(value: &str) -> Option<&'static str> {
    match value.to_lowercase().as_str() {
        "число" | "number" => Some("Number"),
        "строка" | "string" => Some("String"),
        "булево" | "boolean" | "bool" => Some("Boolean"),
        "дата" | "date" => Some("Date"),
        "датавремя" | "datetime" => Some("DateTime"),
        "хранилищезначения" | "valuestorage" => Some("ValueStorage"),
        "справочникссылка" | "catalogref" => Some("CatalogRef"),
        "документссылка" | "documentref" => Some("DocumentRef"),
        "перечислениессылка" | "enumref" => Some("EnumRef"),
        "плансчетовссылка" | "chartofaccountsref" => Some("ChartOfAccountsRef"),
        "планвидовхарактеристикссылка" | "chartofcharacteristictypesref" => {
            Some("ChartOfCharacteristicTypesRef")
        }
        "планвидоврасчётассылка" | "планвидоврасчетассылка" | "chartofcalculationtypesref" => {
            Some("ChartOfCalculationTypesRef")
        }
        "планобменассылка" | "exchangeplanref" => Some("ExchangePlanRef"),
        "бизнеспроцессссылка" | "businessprocessref" => {
            Some("BusinessProcessRef")
        }
        "задачассылка" | "taskref" => Some("TaskRef"),
        "определяемыйтип" | "definedtype" => Some("DefinedType"),
        _ => None,
    }
}

pub(super) fn parse_meta_string_type(value: &str) -> Option<u32> {
    let rest = value.strip_prefix("String(")?.strip_suffix(')')?.trim();
    if rest.is_empty() || rest.contains(',') {
        return None;
    }
    rest.parse().ok().filter(|length| *length <= 1024)
}

pub(super) fn parse_meta_number_type(value: &str) -> Option<(u32, u32, bool)> {
    let rest = value.strip_prefix("Number(")?.strip_suffix(')')?;
    let parts = rest.split(',').map(str::trim).collect::<Vec<_>>();
    if !matches!(parts.len(), 2 | 3)
        || parts.iter().any(|part| part.is_empty())
        || (parts.len() == 3 && parts[2] != "nonneg")
    {
        return None;
    }
    let digits = parts[0].parse().ok()?;
    let fraction = parts[1].parse().ok()?;
    if digits > 38 || fraction > digits {
        return None;
    }
    Some((digits, fraction, parts.len() == 3))
}

pub(super) fn meta_edit_changes_request_line_number_length(raw_changes: &str) -> bool {
    split_meta_edit_commas_outside_parens(raw_changes)
        .into_iter()
        .filter_map(|change| change.split_once('='))
        .any(|(key, _)| meta_edit_is_line_number_length_key(key))
}

pub(super) fn meta_edit_inline_requests_line_number_length(operation: &str, value: &str) -> bool {
    if !operation.eq_ignore_ascii_case("modify-ts") {
        return false;
    }
    split_meta_edit_batch_items(value, operation)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| item.split_once(':'))
        .any(|(_, changes)| meta_edit_changes_request_line_number_length(changes))
}

pub(super) fn meta_edit_definition_requests_line_number_length(definition: &Value) -> bool {
    let Some(definition) = definition.as_object() else {
        return false;
    };
    definition.iter().any(|(operation, operation_value)| {
        if meta_edit_operation_key(operation).as_deref() != Some("modify") {
            return false;
        }
        let Some(modify) = operation_value.as_object() else {
            return false;
        };
        modify.iter().any(|(child_type, child_value)| {
            if meta_edit_child_type_key(child_type) != Some("tabularSections") {
                return false;
            }
            child_value
                .as_object()
                .into_iter()
                .flat_map(|sections| sections.values())
                .filter_map(Value::as_object)
                .flat_map(|changes| changes.keys())
                .any(|key| meta_edit_is_line_number_length_key(key))
        })
    })
}

pub(super) fn meta_edit_apply_inline_operation(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    operation: &str,
    value: &str,
    line_number_length_policy: MetaEditLineNumberLengthPolicy,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let (action, target) = operation
        .split_once('-')
        .ok_or_else(|| format!("Invalid meta-edit Operation: {operation}"))?;

    if let Some(property) = meta_edit_complex_property_from_inline_target(target) {
        meta_edit_apply_complex_property_action(
            xml_text,
            object_type,
            object_name,
            action,
            property,
            meta_edit_split_values(value),
            counts,
        )?;
        return Ok(());
    }

    if target == "ts-attribute" {
        match action {
            "add" => {
                for item in split_meta_edit_batch_items(value, operation)? {
                    meta_edit_add_tabular_section_attribute(xml_text, item)?;
                    counts.added += 1;
                }
            }
            "remove" => {
                counts.removed += meta_edit_remove_tabular_section_attribute(xml_text, value)?
            }
            "modify" => {
                counts.modified += meta_edit_modify_tabular_section_attribute(xml_text, value)?
            }
            _ => return Err(format!("Unsupported meta-edit Operation: {operation}")),
        }
        return Ok(());
    }

    if target == "property" {
        if action != "modify" {
            return Err(format!("Unsupported meta-edit Operation: {operation}"));
        }
        counts.modified += meta_edit_modify_object_properties_from_pairs(xml_text, value)?;
        return Ok(());
    }

    let Some(child_type) = meta_edit_child_type_from_inline_target(target) else {
        return Err(format!("Unsupported meta-edit Operation: {operation}"));
    };

    match action {
        "add" => {
            for item in split_meta_edit_batch_items(value, operation)? {
                let item_value = Value::String(item.to_string());
                meta_edit_add_child_value(
                    xml_text,
                    object_type,
                    object_name,
                    child_type,
                    &item_value,
                )?;
                counts.added += 1;
            }
        }
        "remove" => {
            for item in split_meta_edit_batch_items(value, operation)? {
                meta_edit_remove_child_value(
                    xml_text,
                    child_type,
                    &Value::String(item.to_string()),
                )?;
                counts.removed += 1;
            }
        }
        "modify" => {
            for item in split_meta_edit_batch_items(value, operation)? {
                let (name, raw_changes) = item
                    .split_once(':')
                    .ok_or_else(|| format!("{operation} requires Value like Name: key=value"))?;
                counts.modified += meta_edit_modify_top_child(
                    xml_text,
                    child_type,
                    name.trim(),
                    raw_changes.trim(),
                    line_number_length_policy,
                )?;
            }
        }
        _ => return Err(format!("Unsupported meta-edit Operation: {operation}")),
    }

    Ok(())
}

pub(super) fn meta_edit_apply_definition(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    definition: &Value,
    line_number_length_policy: MetaEditLineNumberLengthPolicy,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let definition = definition
        .as_object()
        .ok_or_else(|| "DefinitionFile root must be a JSON object".to_string())?;

    if let Some(Value::Array(items)) = definition.get("_complex") {
        for item in items {
            let object = item
                .as_object()
                .ok_or_else(|| "_complex item must be an object".to_string())?;
            let action = object
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "_complex item is missing action".to_string())?;
            let property = object
                .get("property")
                .and_then(Value::as_str)
                .ok_or_else(|| "_complex item is missing property".to_string())?;
            let values = meta_edit_values_from_json(object.get("values"));
            meta_edit_apply_complex_property_action(
                xml_text,
                object_type,
                object_name,
                action,
                property,
                values,
                counts,
            )?;
        }
    }

    for (raw_key, value) in definition {
        if raw_key == "_complex" {
            continue;
        }
        match meta_edit_operation_key(raw_key).as_deref() {
            Some("add") => {
                meta_edit_apply_definition_add(xml_text, object_type, object_name, value, counts)?
            }
            Some("remove") => meta_edit_apply_definition_remove(xml_text, value, counts)?,
            Some("modify") => meta_edit_apply_definition_modify(
                xml_text,
                object_type,
                object_name,
                value,
                line_number_length_policy,
                counts,
            )?,
            Some(other) => return Err(format!("Unsupported definition operation: {other}")),
            None => return Err(format!("Unknown definition operation: {raw_key}")),
        }
    }

    Ok(())
}

pub(super) fn meta_edit_apply_definition_add(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    value: &Value,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "definition add must be an object".to_string())?;
    for (raw_child_type, items) in object {
        let child_type = meta_edit_child_type_key(raw_child_type)
            .ok_or_else(|| format!("Unknown add child type: {raw_child_type}"))?;
        for item in meta_edit_definition_items(items) {
            meta_edit_add_child_value(xml_text, object_type, object_name, child_type, &item)?;
            counts.added += 1;
        }
    }
    Ok(())
}

pub(super) fn meta_edit_apply_definition_remove(
    xml_text: &mut String,
    value: &Value,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "definition remove must be an object".to_string())?;
    for (raw_child_type, items) in object {
        let child_type = meta_edit_child_type_key(raw_child_type)
            .ok_or_else(|| format!("Unknown remove child type: {raw_child_type}"))?;
        for item in meta_edit_definition_items(items) {
            meta_edit_remove_child_value(xml_text, child_type, &item)?;
            counts.removed += 1;
        }
    }
    Ok(())
}

pub(super) fn meta_edit_apply_definition_modify(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    value: &Value,
    line_number_length_policy: MetaEditLineNumberLengthPolicy,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "definition modify must be an object".to_string())?;
    for (raw_child_type, items) in object {
        let child_type = meta_edit_child_type_key(raw_child_type)
            .ok_or_else(|| format!("Unknown modify child type: {raw_child_type}"))?;
        if child_type == "properties" {
            meta_edit_modify_object_properties_from_map(
                xml_text,
                object_type,
                object_name,
                items,
                counts,
            )?;
        } else if child_type == "tabularSections" {
            meta_edit_modify_tabular_sections_from_definition(
                xml_text,
                items,
                line_number_length_policy,
                counts,
            )?;
        } else {
            let item_object = items
                .as_object()
                .ok_or_else(|| format!("modify {child_type} must be an object"))?;
            for (name, changes) in item_object {
                let raw_changes = meta_edit_changes_to_inline(changes)?;
                counts.modified += meta_edit_modify_top_child(
                    xml_text,
                    child_type,
                    name,
                    &raw_changes,
                    line_number_length_policy,
                )?;
            }
        }
    }
    Ok(())
}
