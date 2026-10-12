//! Existing extension Designer algorithms; publication stays with trusted composition.
use crate::configuration::*;
use crate::property_states::{
    cfe_patch_direct_md_child, cfe_patch_exact_md_child, cfe_patch_precondition_error,
};
use crate::validation::service_child_semantics;
use roxmltree::Document;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use unica_metadata::configuration::*;
use unica_metadata::extension::*;
use unica_source_contract::discovery::ProjectDiscoveryRead;
use unica_source_contract::proposal::TrackedSourceProposal;
use unica_source_designer::configuration_layout::*;
use unica_source_designer::format_profile::{
    classify_root_version, FormatCompatibility, ACTIVE_FORMAT_PROFILE,
};
use unica_source_designer::read::{
    path_exists, path_is_dir, read_bytes, read_string, read_utf8_sig,
};
use unica_source_designer::roots::root_version_literal;
use unica_xml_format::text::{
    cfe_borrow_ensure_namespace_declaration, cfe_borrow_find_start_tag, cfe_borrow_start_tag_at,
    cfe_borrow_tag_local_name, cfe_borrow_xml_attribute_value_range,
};
const CFE_BORROW_MD_NAMESPACE: &str = "http://v8.1c.ru/8.3/MDClasses";
const CFE_BORROW_NIL_UUID: &str = "00000000-0000-0000-0000-000000000000";

pub const CFE_PATCH_NIL_UUID: &str = "00000000-0000-0000-0000-000000000000";

pub const CFE_PATCH_XR_NAMESPACE: &str = "http://v8.1c.ru/8.3/xcf/readable";

pub const CFE_PATCH_FORM_NAMESPACE: &str = "http://v8.1c.ru/8.3/xcf/logform";

pub const CFE_PATCH_MD_NAMESPACE: &str = "http://v8.1c.ru/8.3/MDClasses";

pub fn cfe_registered_xml_dependency_paths_with_reader<F>(
    config_path: &Path,
    read: &mut F,
) -> Result<Vec<PathBuf>, String>
where
    F: FnMut(&Path) -> Result<Option<String>, String>,
{
    let mut paths = vec![config_path.to_path_buf()];
    let Some(config_text) = read(config_path)? else {
        return Ok(paths);
    };
    let config_document = Document::parse(&config_text)
        .map_err(|error| format!("failed to parse {}: {error}", config_path.display()))?;
    let registered = config_document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "Configuration")
        .and_then(|configuration| {
            unica_xml_format::read::child_element(configuration, "ChildObjects")
        })
        .map(|children| {
            children
                .children()
                .filter(|child| child.is_element())
                .map(|child| {
                    (
                        child.tag_name().name().to_string(),
                        child.text().unwrap_or("").to_string(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    drop(config_document);

    let config_dir = config_path.parent().unwrap_or_else(|| Path::new(""));
    for (type_name, object_name) in registered {
        if object_name.is_empty() {
            continue;
        }
        if type_name == "Language" {
            let language_path = config_dir
                .join("Languages")
                .join(format!("{object_name}.xml"));
            paths.push(language_path.clone());
            let _ = read(&language_path);
            continue;
        }
        let Some(type_dir) = cf_validate_child_type_dir(&type_name) else {
            continue;
        };
        let object_path = config_dir.join(type_dir).join(format!("{object_name}.xml"));
        paths.push(object_path.clone());
        let Some(object_text) = read(&object_path).ok().flatten() else {
            continue;
        };
        let Ok(object_document) = Document::parse(&object_text) else {
            continue;
        };
        let forms = object_document
            .root_element()
            .children()
            .find(|node| node.is_element())
            .and_then(|object| unica_xml_format::read::child_element(object, "ChildObjects"))
            .map(|children| {
                unica_xml_format::read::child_elements(children, "Form")
                    .into_iter()
                    .filter_map(|form| form.text().map(ToOwned::to_owned))
                    .filter(|name| !name.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        drop(object_document);
        for form_name in forms {
            let form_dir = config_dir.join(type_dir).join(&object_name).join("Forms");
            let wrapper = form_dir.join(format!("{form_name}.xml"));
            paths.push(wrapper.clone());
            let _ = read(&wrapper);
            let form_xml = form_dir.join(&form_name).join("Ext").join("Form.xml");
            paths.push(form_xml.clone());
            let _ = read(&form_xml);
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

pub fn cfe_borrow_main_attribute_mode(args: &Map<String, Value>) -> Result<Option<String>, String> {
    for name in ["borrowMainAttribute", "BorrowMainAttribute"] {
        if let Some(value) = args.get(name) {
            if value.as_bool() == Some(false) || value.is_null() {
                return Ok(None);
            }
            if value.as_bool() == Some(true) {
                return Ok(Some("Form".to_string()));
            }
            if let Some(text) = value.as_str() {
                if text.trim().is_empty() {
                    return Ok(Some("Form".to_string()));
                }
                return Ok(Some(text.trim().to_string()));
            }
        }
    }
    Ok(None)
}

pub fn cfe_borrow_parse_object_spec(value: &str) -> Result<CfeBorrowSpec, String> {
    let Some(dot_idx) = value.find('.') else {
        return Err(format!(
            "Invalid format '{value}', expected 'Type.Name' or 'Type.Name.Form.FormName'"
        ));
    };
    if dot_idx < 1 {
        return Err(format!(
            "Invalid format '{value}', expected 'Type.Name' or 'Type.Name.Form.FormName'"
        ));
    }
    let raw_type = &value[..dot_idx];
    let type_name = cfe_borrow_type_synonym(raw_type)
        .unwrap_or(raw_type)
        .to_string();
    if cfe_borrow_type_dir(&type_name).is_none() {
        return Err(format!("Unknown type '{type_name}'"));
    }
    if cfe_borrow_generated_types(&type_name).is_none() {
        return Err(format!(
            "Type '{type_name}' has no proven cfe.borrow InternalInfo profile for platform 1C 8.3.27"
        ));
    }
    let remainder = &value[dot_idx + 1..];
    let (object_name, form_name) = if let Some(form_idx) = remainder.find(".Form.") {
        (
            remainder[..form_idx].to_string(),
            Some(remainder[form_idx + 6..].to_string()),
        )
    } else {
        (remainder.to_string(), None)
    };
    cfe_validate_metadata_name("ObjectName", &object_name)?;
    if let Some(form_name) = &form_name {
        cfe_validate_metadata_name("FormName", form_name)?;
    }
    Ok(CfeBorrowSpec {
        type_name,
        object_name,
        form_name,
    })
}

pub fn cfe_validate_metadata_name(argument: &str, value: &str) -> Result<(), String> {
    let mut components = Path::new(value).components();
    let is_single_path_component = matches!(
        components.next(),
        Some(Component::Normal(component)) if component == OsStr::new(value)
    ) && components.next().is_none();
    if unica_xml_format::names::xml_ncname_is_valid(value) && is_single_path_component {
        Ok(())
    } else {
        Err(format!(
            "{argument} must be a valid Unicode XML NCName and a single path component: {value:?}"
        ))
    }
}

pub fn cfe_borrow_validate_source_descriptor<'a, 'input>(
    root: roxmltree::Node<'a, 'input>,
    source_file: &Path,
    expected_type: &str,
    expected_name: &str,
) -> Result<roxmltree::Node<'a, 'input>, String> {
    if root.tag_name().name() != "MetaDataObject"
        || root.tag_name().namespace() != Some(CFE_BORROW_MD_NAMESPACE)
    {
        return Err(format!(
            "Source descriptor {} must have MetaDataObject namespace '{}'",
            source_file.display(),
            CFE_BORROW_MD_NAMESPACE
        ));
    }

    let objects = root
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    if objects.len() != 1
        || objects[0].tag_name().name() != expected_type
        || objects[0].tag_name().namespace() != Some(CFE_BORROW_MD_NAMESPACE)
    {
        return Err(format!(
            "Source descriptor {} expected exactly one {expected_type} in namespace '{}'",
            source_file.display(),
            CFE_BORROW_MD_NAMESPACE
        ));
    }
    let object = objects[0];

    let properties = object
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == "Properties"
                && node.tag_name().namespace() == Some(CFE_BORROW_MD_NAMESPACE)
        })
        .collect::<Vec<_>>();
    if properties.len() != 1 {
        return Err(format!(
            "Source descriptor {} {expected_type}.{expected_name} must contain exactly one Properties element",
            source_file.display()
        ));
    }
    let names = properties[0]
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == "Name"
                && node.tag_name().namespace() == Some(CFE_BORROW_MD_NAMESPACE)
        })
        .collect::<Vec<_>>();
    if names.len() != 1 || names[0].text() != Some(expected_name) {
        return Err(format!(
            "Source descriptor {} Properties/Name must exactly match '{expected_name}'",
            source_file.display()
        ));
    }

    let source_uuid = object.attribute("uuid").unwrap_or_default();
    if !cf_validate_guid(source_uuid) || source_uuid.eq_ignore_ascii_case(CFE_BORROW_NIL_UUID) {
        return Err(format!(
            "Source descriptor {} {expected_type}.{expected_name} must have a valid non-nil uuid",
            source_file.display()
        ));
    }

    Ok(object)
}

pub fn cfe_borrow_type_synonym(value: &str) -> Option<&'static str> {
    match value {
        "Справочник" => Some("Catalog"),
        "Документ" => Some("Document"),
        "Перечисление" => Some("Enum"),
        "ОбщийМодуль" => Some("CommonModule"),
        "ОбщаяКартинка" => Some("CommonPicture"),
        "ОбщаяКоманда" => Some("CommonCommand"),
        "ОбщийМакет" => Some("CommonTemplate"),
        "ПланОбмена" => Some("ExchangePlan"),
        "Отчет" | "Отчёт" => Some("Report"),
        "Обработка" => Some("DataProcessor"),
        "РегистрСведений" => Some("InformationRegister"),
        "РегистрНакопления" => Some("AccumulationRegister"),
        "ПланВидовХарактеристик" => Some("ChartOfCharacteristicTypes"),
        "ПланСчетов" => Some("ChartOfAccounts"),
        "РегистрБухгалтерии" => Some("AccountingRegister"),
        "ПланВидовРасчета" => Some("ChartOfCalculationTypes"),
        "РегистрРасчета" => Some("CalculationRegister"),
        "БизнесПроцесс" => Some("BusinessProcess"),
        "Задача" => Some("Task"),
        "Подсистема" => Some("Subsystem"),
        "Роль" => Some("Role"),
        "Константа" => Some("Constant"),
        "ФункциональнаяОпция" => Some("FunctionalOption"),
        "ОпределяемыйТип" => Some("DefinedType"),
        "ОбщаяФорма" => Some("CommonForm"),
        "ЖурналДокументов" => Some("DocumentJournal"),
        "ПараметрСеанса" => Some("SessionParameter"),
        "ГруппаКоманд" => Some("CommandGroup"),
        "ПодпискаНаСобытие" => Some("EventSubscription"),
        "РегламентноеЗадание" => Some("ScheduledJob"),
        "ОбщийРеквизит" => Some("CommonAttribute"),
        "ПакетXDTO" => Some("XDTOPackage"),
        "HTTPСервис" => Some("HTTPService"),
        "СервисИнтеграции" => Some("IntegrationService"),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn cfe_borrow_object_shell(
    reader: &dyn ProjectDiscoveryRead,
    cfg_dir: &Path,
    ext_dir: &Path,
    write_plan: &mut dyn TrackedSourceProposal,
    type_name: &str,
    object_name: &str,
    format_version: &str,
    ext_text: &mut String,
    log: &mut CfeBorrowLog,
    fresh_uuid: fn() -> String,
) -> Result<PathBuf, String> {
    let dir_name =
        cfe_borrow_type_dir(type_name).ok_or_else(|| format!("Unknown type '{type_name}'"))?;
    let source_file = cfg_dir.join(dir_name).join(format!("{object_name}.xml"));
    if !reader.path_is_file(&source_file) {
        return Err(format!(
            "Source object not found: {}",
            source_file.display()
        ));
    }
    let source_text = write_plan.read_dependency_utf8_sig(&source_file)?;
    let source_doc =
        Document::parse(&source_text).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
    let source_el = cfe_borrow_validate_source_descriptor(
        source_doc.root_element(),
        &source_file,
        type_name,
        object_name,
    )?;
    let source_uuid = source_el
        .attribute("uuid")
        .expect("validated source descriptor has uuid");
    let target_file = cfe_borrow_target_object(ext_dir, type_name, object_name);
    if write_plan.is_planned(&target_file) {
        log.skipped(
            target_file.display().to_string(),
            "уже запланирован к переносу в этом пакете",
        );
        cfe_borrow_add_to_child_objects(ext_text, type_name, object_name, log)?;
        return Ok(target_file);
    }
    let source_props = unica_xml_format::read::child_element(source_el, "Properties");
    let identity = cfe_borrow_existing_identity(write_plan, &target_file, type_name)?;
    let xml = cfe_borrow_object_xml(
        type_name,
        object_name,
        source_uuid,
        source_props,
        format_version,
        &identity,
        fresh_uuid,
    )?;
    write_plan.write_utf8_bom(&target_file, &xml)?;
    cfe_borrow_add_to_child_objects(ext_text, type_name, object_name, log)?;
    Ok(target_file)
}

pub fn cfe_borrow_object_xml(
    type_name: &str,
    object_name: &str,
    source_uuid: &str,
    source_props: Option<roxmltree::Node<'_, '_>>,
    format_version: &str,
    identity: &CfeBorrowIdentity,
    fresh_uuid: fn() -> String,
) -> Result<String, String> {
    let mut lines = Vec::<String>::new();
    lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
    lines.push(format!(
        "<MetaDataObject {} version=\"{}\">",
        cfe_borrow_xmlns_decl(),
        unica_xml_format::text::escape_xml(format_version)
    ));
    lines.push(format!(
        "\t<{type_name} uuid=\"{}\">",
        identity.wrapper_uuid(fresh_uuid)
    ));
    lines.push(cfe_borrow_internal_info_xml(
        type_name,
        object_name,
        "\t\t",
        identity,
        fresh_uuid,
    )?);
    lines.push("\t\t<Properties>".to_string());
    lines.push("\t\t\t<ObjectBelonging>Adopted</ObjectBelonging>".to_string());
    lines.push(format!(
        "\t\t\t<Name>{}</Name>",
        unica_xml_format::text::escape_xml(object_name)
    ));
    lines.push("\t\t\t<Comment/>".to_string());
    lines.push(format!(
        "\t\t\t<ExtendedConfigurationObject>{}</ExtendedConfigurationObject>",
        unica_xml_format::text::escape_xml(source_uuid)
    ));
    if type_name == "CommonModule" {
        for prop_name in [
            "Global",
            "ClientManagedApplication",
            "Server",
            "ExternalConnection",
            "ClientOrdinaryApplication",
            "ServerCall",
        ] {
            let value = source_props
                .and_then(|props| unica_xml_format::read::child_direct_text(props, prop_name))
                .unwrap_or_else(|| "false".to_string());
            lines.push(format!(
                "\t\t\t<{prop_name}>{}</{prop_name}>",
                unica_xml_format::text::escape_xml(&value)
            ));
        }
        let return_values_reuse = source_props
            .and_then(|props| unica_xml_format::read::child_direct_text(props, "ReturnValuesReuse"))
            .unwrap_or_else(|| "DontUse".to_string());
        lines.push(format!(
            "\t\t\t<ReturnValuesReuse>{}</ReturnValuesReuse>",
            unica_xml_format::text::escape_xml(&return_values_reuse)
        ));
    }
    if type_name == "DefinedType" {
        if let Some(type_xml) = source_props
            .and_then(|props| unica_xml_format::read::child_element(props, "Type"))
            .map(cfe_borrow_xml_node)
        {
            lines.push(format!("\t\t\t{type_xml}"));
        }
    }
    lines.push("\t\t</Properties>".to_string());
    if cfe_borrow_type_has_child_objects(type_name)? {
        lines.push("\t\t<ChildObjects/>".to_string());
    }
    lines.push(format!("\t</{type_name}>"));
    lines.push("</MetaDataObject>".to_string());
    Ok(lines.join("\n"))
}

pub fn cfe_borrow_internal_info_xml(
    type_name: &str,
    object_name: &str,
    indent: &str,
    identity: &CfeBorrowIdentity,
    fresh_uuid: fn() -> String,
) -> Result<String, String> {
    let types = cfe_borrow_generated_types(type_name).ok_or_else(|| {
        format!(
            "Type '{type_name}' has no proven cfe.borrow InternalInfo profile for platform 1C 8.3.27"
        )
    })?;
    if types.is_empty() {
        return Ok(format!("{indent}<InternalInfo/>"));
    }
    let mut lines = vec![format!("{indent}<InternalInfo>")];
    if type_name == "ExchangePlan" {
        lines.push(format!(
            "{indent}\t<xr:ThisNode>{}</xr:ThisNode>",
            identity.this_node(fresh_uuid)
        ));
    }
    for (prefix, category) in types {
        let name = format!("{prefix}.{object_name}");
        let (type_id, value_id) = identity.generated_type(&name, fresh_uuid);
        lines.push(format!(
            "{indent}\t<xr:GeneratedType name=\"{}\" category=\"{}\">",
            unica_xml_format::text::escape_xml(&name),
            category
        ));
        lines.push(format!("{indent}\t\t<xr:TypeId>{type_id}</xr:TypeId>"));
        lines.push(format!("{indent}\t\t<xr:ValueId>{value_id}</xr:ValueId>"));
        lines.push(format!("{indent}\t</xr:GeneratedType>"));
    }
    lines.push(format!("{indent}</InternalInfo>"));
    Ok(lines.join("\n"))
}

pub fn cfe_borrow_generated_types(
    type_name: &str,
) -> Option<&'static [(&'static str, &'static str)]> {
    crate::template_catalog::metadata_generated_types_8_3_27(type_name)
}

#[allow(clippy::too_many_arguments)]
pub fn cfe_borrow_main_attribute_artifacts(
    reader: &dyn ProjectDiscoveryRead,
    cfg_dir: &Path,
    ext_dir: &Path,
    write_plan: &mut dyn TrackedSourceProposal,
    spec: &CfeBorrowSpec,
    mode: Option<&str>,
    format_version: &str,
    ext_text: &mut String,
    log: &mut CfeBorrowLog,
    fresh_uuid: fn() -> String,
) -> Result<Vec<PathBuf>, String> {
    let Some(mode) = mode else {
        return Ok(Vec::new());
    };
    let type_name = spec.type_name.as_str();
    let object_name = spec.object_name.as_str();
    let form_name = spec.form_name.as_deref().unwrap_or_default();
    let dir_name =
        cfe_borrow_type_dir(type_name).ok_or_else(|| format!("Unknown type '{type_name}'"))?;

    let form_paths = if mode == "Form" {
        let form_xml_path = cfg_dir
            .join(dir_name)
            .join(object_name)
            .join("Forms")
            .join(form_name)
            .join("Ext")
            .join("Form.xml");
        let paths = cfe_borrow_collect_form_object_paths(write_plan, &form_xml_path)?;
        if paths.first_level.is_empty() && paths.deep_paths.is_empty() {
            return Ok(Vec::new());
        }
        Some(paths)
    } else {
        None
    };

    let wanted = form_paths.as_ref().map(|paths| &paths.first_level);
    let resolved =
        cfe_borrow_resolve_source_attributes(write_plan, cfg_dir, type_name, object_name, wanted)?;

    let object_file = cfe_borrow_target_object(ext_dir, type_name, object_name);
    cfe_borrow_merge_resolved_into_object(write_plan, &object_file, &resolved, fresh_uuid)?;
    let mut artifacts = Vec::new();

    let mut type_xmls = Vec::<String>::new();
    for attr in &resolved.attributes {
        type_xmls.push(attr.type_xml.clone());
    }
    for section in &resolved.tabular_sections {
        for attr in &section.attributes {
            type_xmls.push(attr.type_xml.clone());
        }
    }
    artifacts.extend(cfe_borrow_ensure_reference_shells(
        CfeBorrowBindings {
            reader,
            cfg_dir,
            ext_dir,
        },
        write_plan,
        &type_xmls,
        format_version,
        ext_text,
        log,
        fresh_uuid,
    )?);

    if let Some(paths) = &form_paths {
        artifacts.extend(cfe_borrow_process_deep_paths(
            reader,
            cfg_dir,
            ext_dir,
            write_plan,
            &resolved,
            &paths.deep_paths,
            format_version,
            ext_text,
            log,
            fresh_uuid,
        )?);
    }

    Ok(artifacts)
}

pub fn cfe_borrow_collect_form_object_paths(
    write_plan: &mut dyn TrackedSourceProposal,
    form_xml_path: &Path,
) -> Result<CfeBorrowFormPaths, String> {
    let source = write_plan.read_dependency_utf8_sig(form_xml_path)?;
    let doc = Document::parse(&source).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
    let mut paths = CfeBorrowFormPaths::default();
    let binding_tags = [
        "DataPath",
        "TitleDataPath",
        "FooterDataPath",
        "HeaderDataPath",
        "MultipleValueDataPath",
        "MultipleValuePresentDataPath",
        "RowPictureDataPath",
        "MultipleValuePictureDataPath",
        "Field",
    ];
    let mut deep_seen = HashSet::<String>::new();
    for node in doc.descendants().filter(|node| node.is_element()) {
        if !binding_tags.contains(&node.tag_name().name()) {
            continue;
        }
        let Some(text) = node.text() else {
            continue;
        };
        for segments in cfe_borrow_object_path_segments(text) {
            if segments.is_empty() || cfe_borrow_is_standard_field(&segments[0]) {
                continue;
            }
            paths.first_level.insert(segments[0].clone());
            if segments.len() >= 2 && !cfe_borrow_is_standard_field(&segments[1]) {
                let key = segments.join(".");
                if deep_seen.insert(key) {
                    paths.deep_paths.push(CfeBorrowDeepPath { segments });
                }
            }
        }
    }
    Ok(paths)
}

pub fn cfe_borrow_object_path_segments(text: &str) -> Vec<Vec<String>> {
    let mut result = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find("Объект.") {
        let after = &rest[pos + "Объект.".len()..];
        let path = after
            .chars()
            .take_while(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '.')
            .collect::<String>();
        let segments = path
            .split('.')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if !segments.is_empty() {
            result.push(segments);
        }
        rest = &after[path.len()..];
    }
    result
}

pub fn cfe_borrow_is_standard_field(name: &str) -> bool {
    matches!(
        name,
        "Code"
            | "Description"
            | "Ref"
            | "Parent"
            | "DeletionMark"
            | "Predefined"
            | "IsFolder"
            | "LineNumber"
            | "RowsCount"
            | "PredefinedDataName"
    )
}

pub fn cfe_borrow_resolve_source_attributes(
    write_plan: &mut dyn TrackedSourceProposal,
    cfg_dir: &Path,
    type_name: &str,
    object_name: &str,
    first_level_names: Option<&HashSet<String>>,
) -> Result<CfeBorrowResolvedAttributes, String> {
    let dir_name =
        cfe_borrow_type_dir(type_name).ok_or_else(|| format!("Unknown type '{type_name}'"))?;
    let source_file = cfg_dir.join(dir_name).join(format!("{object_name}.xml"));
    let source = write_plan.read_dependency_utf8_sig(&source_file)?;
    let doc = Document::parse(&source).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
    let source_el = doc
        .root_element()
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| format!("No metadata element found in {dir_name}/{object_name}.xml"))?;
    let Some(child_objects) = unica_xml_format::read::child_element(source_el, "ChildObjects")
    else {
        return Ok(CfeBorrowResolvedAttributes::default());
    };
    let mut resolved = CfeBorrowResolvedAttributes::default();
    for child in child_objects.children().filter(|node| node.is_element()) {
        let local = child.tag_name().name();
        let props = unica_xml_format::read::child_element(child, "Properties");
        let name = props
            .and_then(|props| unica_xml_format::read::child_direct_text(props, "Name"))
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        if first_level_names.is_some_and(|names| !names.contains(&name)) {
            continue;
        }
        if local == "Attribute" {
            resolved.attributes.push(cfe_borrow_source_attribute(child));
        } else if local == "TabularSection" {
            resolved
                .tabular_sections
                .push(cfe_borrow_source_tabular_section(child));
        }
    }
    Ok(resolved)
}

pub fn cfe_borrow_source_attribute(node: roxmltree::Node<'_, '_>) -> CfeBorrowSourceAttribute {
    let props = unica_xml_format::read::child_element(node, "Properties");
    CfeBorrowSourceAttribute {
        name: props
            .and_then(|props| unica_xml_format::read::child_direct_text(props, "Name"))
            .unwrap_or_default(),
        source_uuid: node.attribute("uuid").unwrap_or("").to_string(),
        type_xml: props
            .and_then(|props| unica_xml_format::read::child_element(props, "Type"))
            .map(cfe_borrow_xml_node)
            .unwrap_or_default(),
    }
}

pub fn cfe_borrow_source_tabular_section(
    node: roxmltree::Node<'_, '_>,
) -> CfeBorrowSourceTabularSection {
    let props = unica_xml_format::read::child_element(node, "Properties");
    let mut generated_types = Vec::new();
    if let Some(internal_info) = unica_xml_format::read::child_element(node, "InternalInfo") {
        for generated in unica_xml_format::read::child_elements(internal_info, "GeneratedType") {
            generated_types.push(CfeBorrowGeneratedType {
                name: generated.attribute("name").unwrap_or("").to_string(),
                category: generated.attribute("category").unwrap_or("").to_string(),
            });
        }
    }
    let mut attributes = Vec::new();
    if let Some(child_objects) = unica_xml_format::read::child_element(node, "ChildObjects") {
        for attr in unica_xml_format::read::child_elements(child_objects, "Attribute") {
            attributes.push(cfe_borrow_source_attribute(attr));
        }
    }
    CfeBorrowSourceTabularSection {
        name: props
            .and_then(|props| unica_xml_format::read::child_direct_text(props, "Name"))
            .unwrap_or_default(),
        source_uuid: node.attribute("uuid").unwrap_or("").to_string(),
        generated_types,
        attributes,
    }
}

pub fn cfe_borrow_merge_resolved_into_object(
    write_plan: &mut dyn TrackedSourceProposal,
    object_file: &Path,
    resolved: &CfeBorrowResolvedAttributes,
    fresh_uuid: fn() -> String,
) -> Result<(), String> {
    let mut object_text = write_plan.read_utf8_sig(object_file)?;
    let existing_names = cfe_borrow_existing_names(&object_text);
    let mut child_xml = Vec::<String>::new();
    for attr in &resolved.attributes {
        if !existing_names.contains(&attr.name) {
            child_xml.push(cfe_borrow_adopted_attribute_xml(attr, "\t\t\t", fresh_uuid));
        }
    }
    for section in &resolved.tabular_sections {
        if !existing_names.contains(&section.name) {
            child_xml.push(cfe_borrow_adopted_tabular_section_xml(
                section, "\t\t\t", fresh_uuid,
            ));
        }
    }
    if child_xml.is_empty() {
        return Ok(());
    }
    cfe_borrow_insert_child_objects(&mut object_text, &child_xml.join("\n"))?;
    write_plan.write_utf8_bom(object_file, &object_text)
}

pub fn cfe_borrow_existing_names(object_text: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut rest = object_text;
    while let Some(start) = rest.find("<Name>") {
        let value_start = start + "<Name>".len();
        let Some(end_rel) = rest[value_start..].find("</Name>") else {
            break;
        };
        names.insert(rest[value_start..value_start + end_rel].to_string());
        rest = &rest[value_start + end_rel + "</Name>".len()..];
    }
    names
}

pub fn cfe_borrow_insert_child_objects(
    object_text: &mut String,
    child_xml: &str,
) -> Result<(), String> {
    if object_text.contains("<ChildObjects/>") {
        *object_text = object_text.replacen(
            "<ChildObjects/>",
            &format!("<ChildObjects>\r\n{child_xml}\r\n\t\t</ChildObjects>"),
            1,
        );
        return Ok(());
    }
    if let Some(pos) = object_text.find("</ChildObjects>") {
        object_text.insert_str(pos, &format!("\r\n{child_xml}\r\n\t\t"));
        return Ok(());
    }
    Err("Cannot merge attributes: <ChildObjects> not found".to_string())
}

pub fn cfe_borrow_adopted_attribute_xml(
    attr: &CfeBorrowSourceAttribute,
    indent: &str,
    fresh_uuid: fn() -> String,
) -> String {
    let mut lines = vec![
        format!("{indent}<Attribute uuid=\"{}\">", fresh_uuid()),
        format!("{indent}\t<InternalInfo/>"),
        format!("{indent}\t<Properties>"),
        format!("{indent}\t\t<ObjectBelonging>Adopted</ObjectBelonging>"),
        format!(
            "{indent}\t\t<Name>{}</Name>",
            unica_xml_format::text::escape_xml(&attr.name)
        ),
        format!("{indent}\t\t<Comment/>"),
        format!(
            "{indent}\t\t<ExtendedConfigurationObject>{}</ExtendedConfigurationObject>",
            unica_xml_format::text::escape_xml(&attr.source_uuid)
        ),
    ];
    if !attr.type_xml.is_empty() {
        lines.push(format!("{indent}\t\t{}", attr.type_xml));
    }
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</Attribute>"));
    lines.join("\n")
}

pub fn cfe_borrow_adopted_tabular_section_xml(
    section: &CfeBorrowSourceTabularSection,
    indent: &str,
    fresh_uuid: fn() -> String,
) -> String {
    let mut lines = vec![format!(
        "{indent}<TabularSection uuid=\"{}\">",
        fresh_uuid()
    )];
    if section.generated_types.is_empty() {
        lines.push(format!("{indent}\t<InternalInfo/>"));
    } else {
        lines.push(format!("{indent}\t<InternalInfo>"));
        for generated in &section.generated_types {
            lines.push(format!(
                "{indent}\t\t<xr:GeneratedType name=\"{}\" category=\"{}\">",
                unica_xml_format::text::escape_xml(&generated.name),
                unica_xml_format::text::escape_xml(&generated.category)
            ));
            lines.push(format!(
                "{indent}\t\t\t<xr:TypeId>{}</xr:TypeId>",
                fresh_uuid()
            ));
            lines.push(format!(
                "{indent}\t\t\t<xr:ValueId>{}</xr:ValueId>",
                fresh_uuid()
            ));
            lines.push(format!("{indent}\t\t</xr:GeneratedType>"));
        }
        lines.push(format!("{indent}\t</InternalInfo>"));
    }
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!(
        "{indent}\t\t<ObjectBelonging>Adopted</ObjectBelonging>"
    ));
    lines.push(format!(
        "{indent}\t\t<Name>{}</Name>",
        unica_xml_format::text::escape_xml(&section.name)
    ));
    lines.push(format!("{indent}\t\t<Comment/>"));
    lines.push(format!(
        "{indent}\t\t<ExtendedConfigurationObject>{}</ExtendedConfigurationObject>",
        unica_xml_format::text::escape_xml(&section.source_uuid)
    ));
    lines.push(format!("{indent}\t</Properties>"));
    if section.attributes.is_empty() {
        lines.push(format!("{indent}\t<ChildObjects/>"));
    } else {
        lines.push(format!("{indent}\t<ChildObjects>"));
        for attr in &section.attributes {
            lines.push(cfe_borrow_adopted_attribute_xml(
                attr,
                &format!("{indent}\t\t"),
                fresh_uuid,
            ));
        }
        lines.push(format!("{indent}\t</ChildObjects>"));
    }
    lines.push(format!("{indent}</TabularSection>"));
    lines.join("\n")
}

/// The existing read binding and source/target roots; construction performs no observation.
#[derive(Clone, Copy)]
pub struct CfeBorrowBindings<'a> {
    pub reader: &'a dyn ProjectDiscoveryRead,
    pub cfg_dir: &'a Path,
    pub ext_dir: &'a Path,
}

pub fn cfe_borrow_ensure_reference_shells(
    bindings: CfeBorrowBindings<'_>,
    write_plan: &mut dyn TrackedSourceProposal,
    type_xmls: &[String],
    format_version: &str,
    ext_text: &mut String,
    log: &mut CfeBorrowLog,
    fresh_uuid: fn() -> String,
) -> Result<Vec<PathBuf>, String> {
    let CfeBorrowBindings {
        reader,
        cfg_dir,
        ext_dir,
    } = bindings;

    let mut artifacts = Vec::new();
    let mut seen = HashSet::<String>::new();
    for (type_name, object_name) in cfe_borrow_collect_reference_types(type_xmls) {
        let key = format!("{type_name}.{object_name}");
        if !seen.insert(key) {
            continue;
        }
        if write_plan.exists(&cfe_borrow_target_object(ext_dir, &type_name, &object_name)) {
            continue;
        }
        let source_file = cfg_dir
            .join(cfe_borrow_type_dir(&type_name).unwrap_or(&type_name))
            .join(format!("{object_name}.xml"));
        if !path_exists(reader, &source_file) {
            log.warn(format!("источник не найден: {type_name}.{object_name}"));
            continue;
        }
        let artifact = cfe_borrow_object_shell(
            reader,
            cfg_dir,
            ext_dir,
            write_plan,
            &type_name,
            &object_name,
            format_version,
            ext_text,
            log,
            fresh_uuid,
        )?;
        log.auto_borrowed(format!("{type_name}.{object_name}"));
        artifacts.push(artifact);
    }
    Ok(artifacts)
}

pub fn cfe_borrow_collect_reference_types(type_xmls: &[String]) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for type_xml in type_xmls {
        let mut rest = type_xml.as_str();
        while let Some(pos) = rest.find("cfg:") {
            let after = &rest[pos + "cfg:".len()..];
            let token = after
                .chars()
                .take_while(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '.')
                .collect::<String>();
            if let Some((prefix, object_name)) = token.split_once('.') {
                let type_name = if prefix == "DefinedType" {
                    Some("DefinedType".to_string())
                } else {
                    prefix
                        .strip_suffix("Ref")
                        .map(ToOwned::to_owned)
                        .or_else(|| prefix.strip_suffix("Object").map(ToOwned::to_owned))
                };
                if let Some(type_name) = type_name {
                    let key = format!("{type_name}.{object_name}");
                    if seen.insert(key) {
                        result.push((type_name, object_name.to_string()));
                    }
                }
            }
            rest = &after[token.len()..];
        }
    }
    result
}

#[allow(clippy::too_many_arguments)]
pub fn cfe_borrow_process_deep_paths(
    reader: &dyn ProjectDiscoveryRead,
    cfg_dir: &Path,
    ext_dir: &Path,
    write_plan: &mut dyn TrackedSourceProposal,
    resolved: &CfeBorrowResolvedAttributes,
    deep_paths: &[CfeBorrowDeepPath],
    format_version: &str,
    ext_text: &mut String,
    log: &mut CfeBorrowLog,
    fresh_uuid: fn() -> String,
) -> Result<Vec<PathBuf>, String> {
    let mut artifacts = Vec::new();
    let attrs_by_name = resolved
        .attributes
        .iter()
        .map(|attr| (attr.name.as_str(), attr))
        .collect::<BTreeMap<_, _>>();
    let sections_by_name = resolved
        .tabular_sections
        .iter()
        .map(|section| (section.name.as_str(), section))
        .collect::<BTreeMap<_, _>>();
    for path in deep_paths {
        let Some(first) = path.segments.first() else {
            continue;
        };
        let target = if let Some(attr) = attrs_by_name.get(first.as_str()) {
            if path.segments.len() < 2 {
                continue;
            }
            cfe_borrow_reference_target_from_type_xml(&attr.type_xml)
                .map(|target| (target, path.segments[1].clone()))
        } else if let Some(section) = sections_by_name.get(first.as_str()) {
            if path.segments.len() < 3 {
                continue;
            }
            let column_name = &path.segments[1];
            let Some(column) = section
                .attributes
                .iter()
                .find(|attr| attr.name == *column_name)
            else {
                continue;
            };
            cfe_borrow_reference_target_from_type_xml(&column.type_xml)
                .map(|target| (target, path.segments[2].clone()))
        } else {
            None
        };
        let Some(((target_type, target_object), sub_attr_name)) = target else {
            continue;
        };
        let target_path = cfe_borrow_target_object(ext_dir, &target_type, &target_object);
        if !write_plan.exists(&target_path) {
            let artifact = cfe_borrow_object_shell(
                reader,
                cfg_dir,
                ext_dir,
                write_plan,
                &target_type,
                &target_object,
                format_version,
                ext_text,
                log,
                fresh_uuid,
            )?;
            log.auto_borrowed(format!("{target_type}.{target_object}"));
            artifacts.push(artifact);
        }
        let mut wanted = HashSet::new();
        wanted.insert(sub_attr_name);
        let sub_resolved = cfe_borrow_resolve_source_attributes(
            write_plan,
            cfg_dir,
            &target_type,
            &target_object,
            Some(&wanted),
        )?;
        if !sub_resolved.attributes.is_empty() || !sub_resolved.tabular_sections.is_empty() {
            cfe_borrow_merge_resolved_into_object(
                write_plan,
                &target_path,
                &sub_resolved,
                fresh_uuid,
            )?;
            artifacts.push(target_path.clone());
            let mut sub_type_xmls = Vec::new();
            for attr in &sub_resolved.attributes {
                sub_type_xmls.push(attr.type_xml.clone());
            }
            for section in &sub_resolved.tabular_sections {
                for attr in &section.attributes {
                    sub_type_xmls.push(attr.type_xml.clone());
                }
            }
            artifacts.extend(cfe_borrow_ensure_reference_shells(
                CfeBorrowBindings {
                    reader,
                    cfg_dir,
                    ext_dir,
                },
                write_plan,
                &sub_type_xmls,
                format_version,
                ext_text,
                log,
                fresh_uuid,
            )?);
        }
    }
    Ok(artifacts)
}

pub fn cfe_borrow_reference_target_from_type_xml(type_xml: &str) -> Option<(String, String)> {
    cfe_borrow_collect_reference_types(&[type_xml.to_string()])
        .into_iter()
        .next()
}

pub fn cfe_borrow_add_to_child_objects(
    ext_text: &mut String,
    type_name: &str,
    object_name: &str,
    log: &mut CfeBorrowLog,
) -> Result<(), String> {
    let mut children = cf_edit_child_objects(ext_text)?;
    if children
        .iter()
        .any(|(child_type, child_name)| child_type == type_name && child_name == object_name)
    {
        log.skipped(
            format!("{type_name}.{object_name}"),
            "уже в составе расширения",
        );
        return Ok(());
    }
    children.push((type_name.to_string(), object_name.to_string()));
    children.sort_by(cf_edit_child_object_cmp);
    *ext_text = cf_edit_replace_child_objects(ext_text, &children)?;
    Ok(())
}

pub fn cfe_borrow_normalize_lxml_config_serialization(ext_text: &mut String) {
    if ext_text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>") {
        *ext_text = ext_text.replacen(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>",
            1,
        );
    }
    *ext_text = ext_text.replace("<DefaultRoles></DefaultRoles>", "<DefaultRoles/>");
    if let Some((start, end, _)) = cf_edit_element_range(ext_text, "ChildObjects") {
        let child_objects = ext_text[start..end].replace("\r\n", "&#13;\n");
        ext_text.replace_range(start..end, &child_objects);
    }
    if !ext_text.ends_with('\n') {
        ext_text.push('\n');
    }
}

pub fn cfe_borrow_form_shell(
    bindings: CfeBorrowBindings<'_>,
    write_plan: &mut dyn TrackedSourceProposal,
    spec: &CfeBorrowSpec,
    format_version: &str,
    borrow_main_attr: bool,
    log: &mut CfeBorrowLog,
    fresh_uuid: fn() -> String,
) -> Result<Vec<PathBuf>, String> {
    let CfeBorrowBindings {
        reader,
        cfg_dir,
        ext_dir,
    } = bindings;

    let type_name = spec.type_name.as_str();
    let object_name = spec.object_name.as_str();
    let form_name = spec.form_name.as_deref().unwrap_or_default();
    let dir_name =
        cfe_borrow_type_dir(type_name).ok_or_else(|| format!("Unknown type '{type_name}'"))?;
    let form_meta_source = cfg_dir
        .join(dir_name)
        .join(object_name)
        .join("Forms")
        .join(format!("{form_name}.xml"));
    if !reader.path_is_file(&form_meta_source) {
        return Err(format!(
            "Source form not found: {}",
            form_meta_source.display()
        ));
    }
    let source_text = write_plan.read_dependency_utf8_sig(&form_meta_source)?;
    let source_doc =
        Document::parse(&source_text).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
    let source_form = source_doc
        .root_element()
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| {
            format!(
                "No metadata element found in source form: {}",
                form_meta_source.display()
            )
        })?;
    let source_uuid = source_form.attribute("uuid").unwrap_or("");
    if source_uuid.is_empty() {
        return Err(format!(
            "No uuid attribute on source form element: {}",
            form_meta_source.display()
        ));
    }
    let source_form_xml = cfg_dir
        .join(dir_name)
        .join(object_name)
        .join("Forms")
        .join(form_name)
        .join("Ext")
        .join("Form.xml");
    if !reader.path_is_file(&source_form_xml) {
        return Err(format!(
            "Source Form.xml not found: {}",
            source_form_xml.display()
        ));
    }
    let form_meta_dir = ext_dir.join(dir_name).join(object_name).join("Forms");
    let form_meta_target = form_meta_dir.join(format!("{form_name}.xml"));
    let form_wrapper_uuid = cfe_borrow_existing_identity(write_plan, &form_meta_target, "Form")?
        .wrapper_uuid(fresh_uuid);
    write_plan.write_utf8_bom(
        &form_meta_target,
        &cfe_borrow_form_metadata_xml(form_name, source_uuid, &form_wrapper_uuid, format_version),
    )?;

    let form_xml_target = form_meta_dir.join(form_name).join("Ext").join("Form.xml");
    let source_form_content = write_plan.read_dependency_utf8_sig(&source_form_xml)?;
    let owner_file = cfg_dir.join(dir_name).join(format!("{object_name}.xml"));
    let owner_content = if write_plan.exists(&owner_file) {
        Some(write_plan.read_dependency_utf8_sig(&owner_file)?)
    } else {
        None
    };
    let source_form_content = match cfe_borrow_synthetic_footer_paths(
        &source_form_content,
        owner_content.as_deref(),
        type_name,
        object_name,
    ) {
        Some(paths) => cfe_borrow_prune_omitted_form_bindings(&source_form_content, &paths),
        None => {
            log.warn(
                "owner metadata unavailable or malformed; uncertain footer bindings preserved"
                    .to_string(),
            );
            source_form_content
        }
    };
    let borrowed_form_xml = cfe_borrow_form_xml(
        reader,
        &source_form_content,
        cfg_dir,
        type_name,
        object_name,
        borrow_main_attr,
        log,
    );
    write_plan.write_utf8_bom(&form_xml_target, &borrowed_form_xml)?;

    let module_file = form_meta_dir
        .join(form_name)
        .join("Ext")
        .join("Form")
        .join("Module.bsl");
    let artifacts = vec![form_meta_target, form_xml_target];
    if write_plan.exists(&module_file) {
        log.skipped(
            module_file.display().to_string(),
            "модуль уже существует и не перезаписан",
        );
    } else {
        log.skipped(
            module_file.display().to_string(),
            "перенесённая форма не объявляет модуль расширения",
        );
    }
    Ok(artifacts)
}

pub fn cfe_borrow_form_metadata_xml(
    form_name: &str,
    source_uuid: &str,
    wrapper_uuid: &str,
    format_version: &str,
) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MetaDataObject {} version=\"{}\">\n\t<Form uuid=\"{}\">\n\t\t<InternalInfo>\n\t\t\t<xr:PropertyState>\n\t\t\t\t<xr:Property>Form</xr:Property>\n\t\t\t\t<xr:State>Extended</xr:State>\n\t\t\t</xr:PropertyState>\n\t\t</InternalInfo>\n\t\t<Properties>\n\t\t\t<ObjectBelonging>Adopted</ObjectBelonging>\n\t\t\t<Name>{}</Name>\n\t\t\t<Comment/>\n\t\t\t<ExtendedConfigurationObject>{}</ExtendedConfigurationObject>\n\t\t\t<FormType>Managed</FormType>\n\t\t</Properties>\n\t</Form>\n</MetaDataObject>",
        cfe_borrow_xmlns_decl(),
        unica_xml_format::text::escape_xml(format_version),
        unica_xml_format::text::escape_xml(wrapper_uuid),
        unica_xml_format::text::escape_xml(form_name),
        unica_xml_format::text::escape_xml(source_uuid)
    )
}

pub fn cfe_borrow_form_xml(
    reader: &dyn ProjectDiscoveryRead,
    source_form_content: &str,
    cfg_dir: &Path,
    type_name: &str,
    object_name: &str,
    borrow_main_attr: bool,
    log: &mut CfeBorrowLog,
) -> String {
    let source = source_form_content.trim_start_matches('\u{feff}');
    let version = ACTIVE_FORMAT_PROFILE.export_format.to_string();
    let blocks = cfe_borrow_form_top_level_blocks(source);
    if blocks.is_empty() {
        return cfe_borrow_form_xml_fallback(source, type_name, object_name, borrow_main_attr);
    }

    let xml_decl = cfe_borrow_xml_declaration(source)
        .unwrap_or("<?xml version=\"1.0\" encoding=\"UTF-8\"?>")
        .to_string();
    let form_tag = cfe_borrow_form_open_tag(source).unwrap_or_else(|| {
        format!(
            "<Form version=\"{}\">",
            unica_xml_format::text::escape_xml(&version)
        )
    });

    let mut form_props = Vec::<String>::new();
    let mut auto_cmd_xml = None::<String>;
    let mut child_items_xml = None::<String>;
    let mut reached_visual = false;
    for block in &blocks {
        match block.local_name.as_str() {
            "AutoCommandBar" if auto_cmd_xml.is_none() => {
                reached_visual = true;
                auto_cmd_xml = Some(cfe_borrow_clean_form_fragment(&block.xml));
            }
            "ChildItems" => {
                reached_visual = true;
                if child_items_xml.is_none() {
                    child_items_xml = Some(cfe_borrow_clean_form_fragment(&block.xml));
                }
            }
            "Events" | "Attributes" | "Commands" | "Parameters" | "CommandSet" => {
                reached_visual = true;
            }
            _ if !reached_visual => {
                form_props.push(cfe_borrow_clean_form_fragment(&block.xml));
            }
            _ => {}
        }
    }

    if let Some(xml) = &mut auto_cmd_xml {
        *xml = cfe_borrow_replace_simple_element_text(xml, "CommandName", "0");
        *xml = xml.replace("<Autofill>true</Autofill>", "<Autofill>false</Autofill>");
        *xml = cfe_borrow_remove_simple_element(xml, "ExcludedCommand");
        if borrow_main_attr {
            *xml = cfe_borrow_remove_simple_element_unless_text_starts_with(
                xml,
                "DataPath",
                "Объект.",
            );
        } else {
            *xml = cfe_borrow_remove_simple_element(xml, "DataPath");
        }
    }

    if let Some(xml) = &mut child_items_xml {
        *xml = cfe_borrow_replace_simple_element_text(xml, "CommandName", "0");
        if borrow_main_attr {
            *xml = cfe_borrow_remove_simple_element_unless_text_starts_with(
                xml,
                "DataPath",
                "Объект.",
            );
            *xml = cfe_borrow_remove_simple_element_unless_text_starts_with(
                xml,
                "TitleDataPath",
                "Объект.",
            );
            *xml = cfe_borrow_remove_simple_element(xml, "RowPictureDataPath");
        } else {
            *xml = cfe_borrow_remove_simple_element(xml, "DataPath");
            *xml = cfe_borrow_remove_simple_element(xml, "TitleDataPath");
            *xml = cfe_borrow_remove_simple_element(xml, "RowPictureDataPath");
        }
        *xml = cfe_borrow_remove_simple_element(xml, "ExcludedCommand");
        *xml = cfe_borrow_remove_element_blocks(xml, "TypeLink", |block| {
            block.contains("<xr:DataPath>Items.")
        });
        *xml = cfe_borrow_remove_element_blocks(xml, "Events", |_| true);

        let mut referenced_pictures = cfe_borrow_collect_common_picture_refs(xml);
        if let Some(auto_xml) = &auto_cmd_xml {
            referenced_pictures.extend(cfe_borrow_collect_common_picture_refs(auto_xml));
        }
        referenced_pictures.sort();
        referenced_pictures.dedup();
        let mut borrowed_pictures = HashSet::<String>::new();
        for picture_name in &referenced_pictures {
            if reader.path_is_file(
                &cfg_dir
                    .join("CommonPictures")
                    .join(format!("{picture_name}.xml")),
            ) {
                borrowed_pictures.insert(picture_name.clone());
            } else {
                log.warn(format!(
                    "CommonPicture.{picture_name} не найдена в исходной конфигурации — будет вырезана из формы"
                ));
            }
        }
        *xml = cfe_borrow_strip_picture_blocks(xml, &borrowed_pictures);
        if let Some(auto_xml) = &mut auto_cmd_xml {
            *auto_xml = cfe_borrow_strip_picture_blocks(auto_xml, &borrowed_pictures);
        }

        let mut referenced_styles = cfe_borrow_collect_style_item_refs(xml);
        referenced_styles.sort();
        referenced_styles.dedup();
        for style_name in referenced_styles {
            if !reader.path_is_file(&cfg_dir.join("StyleItems").join(format!("{style_name}.xml"))) {
                log.warn(format!(
                    "StyleItem.{style_name} не найден в исходной конфигурации"
                ));
            }
        }
    }

    // Omit auxiliary attributes/columns and, when requested, the main attribute.
    // Their bindings must not survive in either snapshot or form-level properties.
    let omitted_paths = cfe_borrow_omitted_form_paths(source, borrow_main_attr);
    for xml in [&mut auto_cmd_xml, &mut child_items_xml]
        .into_iter()
        .flatten()
        .chain(form_props.iter_mut())
    {
        *xml = cfe_borrow_prune_omitted_form_bindings(xml, &omitted_paths);
    }
    if !omitted_paths.is_empty() {
        let mut names = omitted_paths.iter().cloned().collect::<Vec<_>>();
        names.sort();
        log.warn(format!("Borrowed form shell omits {} auxiliary form data paths; unsupported bindings removed (examples: {})", names.len(), names.iter().take(5).cloned().collect::<Vec<_>>().join(", ")));
    }

    let main_attr_type = if borrow_main_attr {
        let object_type_prefix = cfe_borrow_generated_types(type_name)
            .and_then(|items| {
                items
                    .iter()
                    .find(|(_, category)| *category == "Object")
                    .map(|(prefix, _)| *prefix)
            })
            .unwrap_or("");
        Some(format!(
            "cfg:{}.{}",
            unica_xml_format::text::escape_xml(object_type_prefix),
            unica_xml_format::text::escape_xml(object_name)
        ))
    } else {
        None
    };

    let mut parts = vec![xml_decl, "\r\n".to_string(), form_tag, "\r\n".to_string()];
    for prop_xml in &form_props {
        parts.push(format!("\t{prop_xml}\r\n"));
    }
    if let Some(xml) = &auto_cmd_xml {
        parts.push(format!("\t{xml}\r\n"));
    }
    if let Some(xml) = &child_items_xml {
        parts.push(format!("\t{xml}\r\n"));
    }
    parts.push(cfe_borrow_main_form_attributes_xml(
        "\t",
        main_attr_type.as_deref(),
    ));
    parts.push("\r\n".to_string());
    parts.push(format!(
        "\t<BaseForm version=\"{}\">\r\n",
        unica_xml_format::text::escape_xml(&version)
    ));
    for prop_xml in &form_props {
        parts.push(format!("\t\t{prop_xml}\r\n"));
    }
    if let Some(xml) = &auto_cmd_xml {
        parts.push(cfe_borrow_indent_form_fragment_for_base(xml));
    }
    if let Some(xml) = &child_items_xml {
        parts.push(cfe_borrow_indent_form_fragment_for_base(xml));
    }
    parts.push(cfe_borrow_main_form_attributes_xml(
        "\t\t",
        main_attr_type.as_deref(),
    ));
    parts.push("\r\n\t</BaseForm>\r\n</Form>".to_string());
    parts.concat()
}

// Return data roots omitted from the shell, including an unborrowed main attribute.
pub fn cfe_borrow_omitted_form_paths(source: &str, borrow_main_attr: bool) -> HashSet<String> {
    let mut paths = HashSet::new();
    let Ok(doc) = Document::parse(source) else {
        return paths;
    };
    for attrs in doc
        .root_element()
        .children()
        .filter(|n| n.has_tag_name("Attributes"))
    {
        for attr in attrs.children().filter(|n| n.has_tag_name("Attribute")) {
            let Some(name) = attr.attribute("name") else {
                continue;
            };
            let main = attr
                .children()
                .any(|n| n.has_tag_name("MainAttribute") && n.text() == Some("true"));
            if !main || !borrow_main_attr {
                paths.insert(name.to_string());
            }
            for columns in attr
                .descendants()
                .filter(|n| n.has_tag_name("AdditionalColumns"))
            {
                let Some(table) = columns.attribute("table") else {
                    continue;
                };
                for column in columns.children().filter(|n| n.has_tag_name("Column")) {
                    if let Some(name) = column.attribute("name") {
                        paths.insert(format!("{table}.{name}"));
                    }
                }
            }
        }
    }
    paths
}

// Infer only a Total+numeric-attribute footer paired with its exact DataPath.
// Unknown cases and declared Total-prefixed metadata fields stay untouched.
// The emitter currently uses the conventional main attribute "Объект";
// differently named main attributes are deliberately not inferred here.
pub fn cfe_borrow_synthetic_footer_paths(
    source: &str,
    owner: Option<&str>,
    type_name: &str,
    object_name: &str,
) -> Option<HashSet<String>> {
    use unica_xml_format::read::{child_direct_text, child_element};

    let form = Document::parse(source).ok()?;
    let candidates: Vec<_> = form
        .descendants()
        .filter(|node| node.has_tag_name("FooterDataPath"))
        .filter(|node| {
            let Some(path) = node.text() else {
                return false;
            };
            let Some((parent_path, name)) = path.rsplit_once('.') else {
                return false;
            };
            let Some(attribute) = name.strip_prefix("Total").filter(|n| !n.is_empty()) else {
                return false;
            };
            let data_path = format!("{parent_path}.{attribute}");
            node.parent().is_some_and(|parent| {
                parent.children().any(|child| {
                    child.has_tag_name("DataPath") && child.text() == Some(data_path.as_str())
                })
            })
        })
        .filter_map(|node| node.text())
        .filter(|path| {
            path.split('.')
                .next_back()
                .is_some_and(|name| name.starts_with("Total"))
        })
        .collect();
    if candidates.is_empty() {
        return Some(HashSet::new());
    }
    let metadata = Document::parse(owner?.trim_start_matches('\u{feff}')).ok()?;
    let descriptor = metadata
        .root_element()
        .children()
        .find(|node| node.has_tag_name(type_name))?;
    let properties = child_element(descriptor, "Properties")?;
    if child_direct_text(properties, "Name").as_deref() != Some(object_name) {
        return None;
    }
    let children = child_element(descriptor, "ChildObjects")?;
    let mut omitted = HashSet::new();
    for path in candidates {
        let pieces: Vec<_> = path.split('.').collect();
        if pieces.len() != 3 || pieces[0] != "Объект" {
            continue;
        }
        let section = children.children().find(|node| {
            node.has_tag_name("TabularSection")
                && child_element(*node, "Properties")
                    .and_then(|props| child_direct_text(props, "Name"))
                    .as_deref()
                    == Some(pieces[1])
        });
        let Some(section) = section else {
            continue;
        };
        let Some(attributes) = child_element(section, "ChildObjects") else {
            continue;
        };
        let declared = attributes.children().any(|node| {
            node.has_tag_name("Attribute")
                && child_element(node, "Properties")
                    .and_then(|props| child_direct_text(props, "Name"))
                    .as_deref()
                    == Some(pieces[2])
        });
        let numeric_base = pieces[2].strip_prefix("Total").is_some_and(|base| {
            attributes.children().any(|node| {
                if !node.has_tag_name("Attribute") {
                    return false;
                }
                let Some(props) = child_element(node, "Properties") else {
                    return false;
                };
                if child_direct_text(props, "Name").as_deref() != Some(base) {
                    return false;
                }
                let Some(typ) = child_element(props, "Type") else {
                    return false;
                };
                let types: Vec<_> = typ.children().filter(|n| n.has_tag_name("Type")).collect();
                types.len() == 1 && types[0].text() == Some("xs:decimal")
            })
        });
        if !declared && numeric_base {
            omitted.insert(path.to_string());
        }
    }
    Some(omitted)
}

pub fn cfe_borrow_prune_omitted_form_bindings(xml: &str, omitted: &HashSet<String>) -> String {
    let invalid = |value: &str| {
        let value = value.trim();
        omitted.iter().any(|path| {
            value == path
                || value
                    .strip_prefix(path.as_str())
                    .is_some_and(|tail| tail.starts_with('.'))
        })
    };
    let mut result = xml.to_string();
    // A link without its source is not a valid link. Remove the entire link,
    // preserving other links in the same ChoiceParameterLinks container.
    result = cfe_borrow_remove_element_blocks(&result, "Link", |block| {
        let mut cursor = 0;
        while let Some(offset) = block[cursor..].find('<') {
            let start = cursor + offset;
            let Some(tag) = cfe_borrow_start_tag_at(block, start) else {
                cursor = start + 1;
                continue;
            };
            if tag.local_name == "DataPath" {
                if let Some(end) = block[tag.end..].find('<') {
                    if invalid(&block[tag.end..tag.end + end]) {
                        return true;
                    }
                }
            }
            cursor = tag.end;
        }
        false
    });
    for tag in [
        "DataPath",
        "TitleDataPath",
        "FooterDataPath",
        "HeaderDataPath",
        "MultipleValueDataPath",
        "MultipleValuePresentDataPath",
        "RowPictureDataPath",
        "MultipleValuePictureDataPath",
    ] {
        result = cfe_borrow_remove_element_blocks(&result, tag, |block| {
            let Some(open) = cfe_borrow_start_tag_at(block, 0) else {
                return false;
            };
            let Some(end) = block[open.end..].find('<') else {
                return false;
            };
            invalid(&block[open.end..open.end + end])
        });
    }
    result
}

pub fn cfe_borrow_form_xml_fallback(
    source: &str,
    type_name: &str,
    object_name: &str,
    borrow_main_attr: bool,
) -> String {
    let version = ACTIVE_FORMAT_PROFILE.export_format.to_string();
    let mut content = cfe_borrow_normalize_form_root_version(source);
    if !borrow_main_attr {
        content = cfe_borrow_strip_simple_data_paths(&content);
    }
    let main_attr = if borrow_main_attr {
        let object_type_prefix = cfe_borrow_generated_types(type_name)
            .and_then(|items| {
                items
                    .iter()
                    .find(|(_, category)| *category == "Object")
                    .map(|(prefix, _)| *prefix)
            })
            .unwrap_or(type_name);
        format!(
            "<Attributes>\n\t\t<Attribute name=\"Объект\" id=\"1000001\">\n\t\t\t<Type><v8:Type>cfg:{}.{}</v8:Type></Type>\n\t\t\t<MainAttribute>true</MainAttribute>\n\t\t\t<SavedData>true</SavedData>\n\t\t</Attribute>\n\t</Attributes>",
            unica_xml_format::text::escape_xml(object_type_prefix),
            unica_xml_format::text::escape_xml(object_name)
        )
    } else {
        "<Attributes/>".to_string()
    };
    if content.contains("</Form>") && !content.contains("<BaseForm") {
        content = content.replacen(
            "</Form>",
            &format!(
                "\t<BaseForm version=\"{}\">\n\t\t{}\n\t</BaseForm>\n</Form>",
                unica_xml_format::text::escape_xml(&version),
                main_attr
            ),
            1,
        );
    }
    if borrow_main_attr && content.contains("<Attributes/>") {
        content = content.replacen("<Attributes/>", &main_attr, 1);
    }
    unica_xml_format::text::ensure_trailing_lf(&content)
}

pub fn cfe_borrow_strip_simple_data_paths(value: &str) -> String {
    let mut text = cfe_borrow_remove_simple_element(value, "DataPath");
    text = cfe_borrow_remove_simple_element(&text, "TitleDataPath");
    cfe_borrow_remove_simple_element(&text, "RowPictureDataPath")
}

pub fn cfe_borrow_xml_declaration(source: &str) -> Option<&str> {
    if source.starts_with("<?xml") {
        let end = source.find("?>")? + 2;
        Some(&source[..end])
    } else {
        None
    }
}

pub fn cfe_borrow_form_open_tag(source: &str) -> Option<String> {
    let start = cfe_borrow_find_start_tag(source, "Form", 0)?;
    let end = source[start..].find('>')? + start + 1;
    Some(cfe_borrow_normalize_form_open_tag(&source[start..end]))
}

pub fn cfe_borrow_normalize_form_root_version(source: &str) -> String {
    let Some(start) = cfe_borrow_find_start_tag(source, "Form", 0) else {
        return source.to_string();
    };
    let Some(end) = source[start..].find('>').map(|offset| start + offset + 1) else {
        return source.to_string();
    };
    let normalized = cfe_borrow_normalize_form_open_tag(&source[start..end]);
    format!("{}{}{}", &source[..start], normalized, &source[end..])
}

pub fn cfe_borrow_normalize_form_open_tag(open_tag: &str) -> String {
    let active = ACTIVE_FORMAT_PROFILE.export_format;
    let normalized = match cfe_borrow_version_value_range(open_tag) {
        Ok(Some((value_start, value_end))) => format!(
            "{}{}{}",
            &open_tag[..value_start],
            active,
            &open_tag[value_end..]
        ),
        Err(_) => open_tag.to_string(),
        Ok(None) => {
            let insert_at = open_tag
                .rfind("/>")
                .or_else(|| open_tag.rfind('>'))
                .unwrap_or(open_tag.len());
            format!(
                "{} version=\"{}\"{}",
                &open_tag[..insert_at],
                active,
                &open_tag[insert_at..]
            )
        }
    };
    let normalized =
        cfe_borrow_ensure_namespace_declaration(normalized, "v8", "http://v8.1c.ru/8.1/data/core");
    cfe_borrow_ensure_namespace_declaration(
        normalized,
        "cfg",
        "http://v8.1c.ru/8.1/data/enterprise/current-config",
    )
}

pub fn cfe_borrow_has_xml_attribute(open_tag: &str, attribute_name: &str) -> bool {
    let mut search_start = 0usize;
    while let Some(relative) = open_tag[search_start..].find(attribute_name) {
        let start = search_start + relative;
        let end = start + attribute_name.len();
        let preceded_by_space = start > 0 && open_tag.as_bytes()[start - 1].is_ascii_whitespace();
        let followed_by_assignment = open_tag[end..]
            .trim_start_matches(|character: char| character.is_ascii_whitespace())
            .starts_with('=');
        if preceded_by_space && followed_by_assignment {
            return true;
        }
        search_start = end;
    }
    false
}

pub fn cfe_borrow_version_value_range(
    open_tag: &str,
) -> Result<Option<(usize, usize)>, unica_xml_format::text::AttributeSyntaxError> {
    cfe_borrow_xml_attribute_value_range(open_tag, "version")
}

pub fn cfe_borrow_form_top_level_blocks(source: &str) -> Vec<CfeBorrowFormBlock> {
    let Some(root_start) = cfe_borrow_find_start_tag(source, "Form", 0) else {
        return Vec::new();
    };
    let Some(root_open_end) = source[root_start..]
        .find('>')
        .map(|pos| root_start + pos + 1)
    else {
        return Vec::new();
    };
    let Some(root_close_rel) = source[root_open_end..].rfind("</Form>") else {
        return Vec::new();
    };
    let body = &source[root_open_end..root_open_end + root_close_rel];
    let mut blocks = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel_open) = body[cursor..].find('<') {
        let open = cursor + rel_open;
        if body[open..].starts_with("</")
            || body[open..].starts_with("<?")
            || body[open..].starts_with("<!")
        {
            cursor = open + 1;
            continue;
        }
        let Some(tag) = cfe_borrow_start_tag_at(body, open) else {
            cursor = open + 1;
            continue;
        };
        let Some(end) = cfe_borrow_element_end(body, open) else {
            cursor = tag.end;
            continue;
        };
        let tail_end = body[end..]
            .find('<')
            .map(|pos| end + pos)
            .unwrap_or(body.len());
        blocks.push(CfeBorrowFormBlock {
            local_name: tag.local_name,
            xml: body[open..tail_end].to_string(),
        });
        cursor = tail_end;
    }
    blocks
}

pub fn cfe_borrow_closing_tag_at(text: &str, open: usize) -> Option<(String, usize)> {
    let rest = text.get(open..)?;
    if !rest.starts_with("</") {
        return None;
    }
    let after = open + 2;
    let mut name_end = after;
    for (idx, ch) in text[after..].char_indices() {
        if ch.is_whitespace() || ch == '>' {
            break;
        }
        name_end = after + idx + ch.len_utf8();
    }
    if name_end == after {
        return None;
    }
    let raw_name = &text[after..name_end];
    let gt = text[open..].find('>')? + open;
    Some((cfe_borrow_tag_local_name(raw_name).to_string(), gt + 1))
}

pub fn cfe_borrow_element_end(text: &str, open: usize) -> Option<usize> {
    let start_tag = cfe_borrow_start_tag_at(text, open)?;
    if start_tag.self_closing {
        return Some(start_tag.end);
    }
    let mut depth = 1usize;
    let mut cursor = start_tag.end;
    while let Some(rel_next) = text[cursor..].find('<') {
        let next = cursor + rel_next;
        if let Some((local_name, end)) = cfe_borrow_closing_tag_at(text, next) {
            if local_name == start_tag.local_name {
                depth -= 1;
                if depth == 0 {
                    return Some(end);
                }
            }
            cursor = end;
            continue;
        }
        if let Some(tag) = cfe_borrow_start_tag_at(text, next) {
            if tag.local_name == start_tag.local_name && !tag.self_closing {
                depth += 1;
            }
            cursor = tag.end;
            continue;
        }
        cursor = next + 1;
    }
    None
}

pub fn cfe_borrow_clean_form_fragment(value: &str) -> String {
    let normalized = value.replace("\r\n", "\n").replace('\r', "\n");
    cfe_borrow_strip_xmlns_declarations(&normalized)
}

pub fn cfe_borrow_strip_xmlns_declarations(value: &str) -> String {
    let mut result = value.to_string();
    let mut cursor = 0usize;
    while let Some(rel_pos) = result[cursor..].find("xmlns") {
        let pos = cursor + rel_pos;
        let after_xmlns = pos + "xmlns".len();
        let next = result[after_xmlns..].chars().next();
        if !matches!(next, Some('=') | Some(':')) {
            cursor = after_xmlns;
            continue;
        }
        let Some(prev) = result[..pos].chars().next_back() else {
            cursor = after_xmlns;
            continue;
        };
        if !prev.is_whitespace() {
            cursor = after_xmlns;
            continue;
        }
        let mut remove_start = pos;
        while remove_start > 0 {
            let Some(ch) = result[..remove_start].chars().next_back() else {
                break;
            };
            if ch.is_whitespace() {
                remove_start -= ch.len_utf8();
            } else {
                break;
            }
        }
        let Some(first_quote_rel) = result[pos..].find('"') else {
            cursor = after_xmlns;
            continue;
        };
        let first_quote = pos + first_quote_rel;
        let Some(second_quote_rel) = result[first_quote + 1..].find('"') else {
            cursor = after_xmlns;
            continue;
        };
        let remove_end = first_quote + 1 + second_quote_rel + 1;
        result.replace_range(remove_start..remove_end, "");
        cursor = remove_start;
    }
    result
}

pub fn cfe_borrow_replace_simple_element_text(value: &str, tag: &str, replacement: &str) -> String {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut result = String::with_capacity(value.len());
    let mut cursor = 0usize;
    while let Some(rel_start) = value[cursor..].find(&open) {
        let start = cursor + rel_start;
        let value_start = start + open.len();
        let Some(rel_end) = value[value_start..].find(&close) else {
            break;
        };
        let end = value_start + rel_end;
        result.push_str(&value[cursor..value_start]);
        result.push_str(replacement);
        result.push_str(&value[end..end + close.len()]);
        cursor = end + close.len();
    }
    result.push_str(&value[cursor..]);
    result
}

pub fn cfe_borrow_remove_simple_element(value: &str, tag: &str) -> String {
    cfe_borrow_remove_simple_element_if(value, tag, |_| true)
}

pub fn cfe_borrow_remove_simple_element_unless_text_starts_with(
    value: &str,
    tag: &str,
    prefix: &str,
) -> String {
    cfe_borrow_remove_simple_element_if(value, tag, |text| !text.trim().starts_with(prefix))
}

pub fn cfe_borrow_remove_simple_element_if<F>(value: &str, tag: &str, should_remove: F) -> String
where
    F: Fn(&str) -> bool,
{
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut result = String::with_capacity(value.len());
    let mut copied_until = 0usize;
    let mut cursor = 0usize;
    while let Some(rel_start) = value[cursor..].find(&open) {
        let start = cursor + rel_start;
        let value_start = start + open.len();
        let Some(rel_end) = value[value_start..].find(&close) else {
            break;
        };
        let end = value_start + rel_end + close.len();
        if should_remove(&value[value_start..value_start + rel_end]) {
            let remove_start =
                cfe_borrow_preceding_whitespace_start(value, start).max(copied_until);
            result.push_str(&value[copied_until..remove_start]);
            copied_until = end;
        }
        cursor = end;
    }
    result.push_str(&value[copied_until..]);
    result
}

pub fn cfe_borrow_remove_element_blocks<F>(value: &str, tag: &str, should_remove: F) -> String
where
    F: Fn(&str) -> bool,
{
    let mut result = String::with_capacity(value.len());
    let mut copied_until = 0usize;
    let mut cursor = 0usize;
    while let Some(rel_open) = value[cursor..].find('<') {
        let open = cursor + rel_open;
        let Some(start_tag) = cfe_borrow_start_tag_at(value, open) else {
            cursor = open + 1;
            continue;
        };
        if start_tag.local_name != tag {
            cursor = start_tag.end;
            continue;
        }
        let Some(end) = cfe_borrow_element_end(value, open) else {
            cursor = start_tag.end;
            continue;
        };
        if should_remove(&value[open..end]) {
            let remove_start = cfe_borrow_preceding_whitespace_start(value, open).max(copied_until);
            result.push_str(&value[copied_until..remove_start]);
            copied_until = end;
        }
        cursor = end;
    }
    result.push_str(&value[copied_until..]);
    result
}

pub fn cfe_borrow_preceding_whitespace_start(value: &str, start: usize) -> usize {
    let mut remove_start = start;
    while remove_start > 0 {
        let Some(ch) = value[..remove_start].chars().next_back() else {
            break;
        };
        if ch.is_whitespace() {
            remove_start -= ch.len_utf8();
        } else {
            break;
        }
    }
    remove_start
}

pub fn cfe_borrow_collect_common_picture_refs(value: &str) -> Vec<String> {
    let mut result = Vec::<String>::new();
    let mut cursor = 0usize;
    let needle = "<xr:Ref>CommonPicture.";
    while let Some(rel_pos) = value[cursor..].find(needle) {
        let start = cursor + rel_pos + needle.len();
        let name = value[start..]
            .chars()
            .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
            .collect::<String>();
        if !name.is_empty() {
            result.push(name);
        }
        cursor = start;
    }
    result
}

pub fn cfe_borrow_strip_picture_blocks(
    value: &str,
    borrowed_common_pictures: &HashSet<String>,
) -> String {
    cfe_borrow_remove_element_blocks(value, "Picture", |block| {
        if let Some(name) = cfe_borrow_common_picture_name(block) {
            return !borrowed_common_pictures.contains(&name);
        }
        if let Some(name) = cfe_borrow_std_picture_name(block) {
            return name != "Print";
        }
        false
    })
}

pub fn cfe_borrow_common_picture_name(value: &str) -> Option<String> {
    let needle = "<xr:Ref>CommonPicture.";
    let start = value.find(needle)? + needle.len();
    let name = value[start..]
        .chars()
        .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
        .collect::<String>();
    (!name.is_empty()).then_some(name)
}

pub fn cfe_borrow_std_picture_name(value: &str) -> Option<String> {
    let needle = "<xr:Ref>StdPicture.";
    let start = value.find(needle)? + needle.len();
    let name = value[start..]
        .chars()
        .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
        .collect::<String>();
    (!name.is_empty()).then_some(name)
}

pub fn cfe_borrow_collect_style_item_refs(value: &str) -> Vec<String> {
    let mut result = Vec::<String>::new();
    let mut cursor = 0usize;
    let attr_needle = "ref=\"style:";
    while let Some(rel_pos) = value[cursor..].find(attr_needle) {
        let start = cursor + rel_pos + attr_needle.len();
        let name = cfe_borrow_read_identifier(&value[start..]);
        if !name.is_empty() {
            result.push(name);
        }
        cursor = start;
    }
    cursor = 0;
    let text_needle = ">style:";
    while let Some(rel_pos) = value[cursor..].find(text_needle) {
        let start = cursor + rel_pos + text_needle.len();
        let name = cfe_borrow_read_identifier(&value[start..]);
        if !name.is_empty() {
            result.push(name);
        }
        cursor = start;
    }
    result
}

pub fn cfe_borrow_read_identifier(value: &str) -> String {
    value
        .chars()
        .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
        .collect()
}

pub fn cfe_borrow_main_form_attributes_xml(indent: &str, main_attr_type: Option<&str>) -> String {
    if let Some(main_attr_type) = main_attr_type {
        format!(
            "{indent}<Attributes>\r\n{indent}\t<Attribute name=\"Объект\" id=\"1000001\">\r\n{indent}\t\t<Type><v8:Type>{main_attr_type}</v8:Type></Type>\r\n{indent}\t\t<MainAttribute>true</MainAttribute>\r\n{indent}\t\t<SavedData>true</SavedData>\r\n{indent}\t</Attribute>\r\n{indent}</Attributes>"
        )
    } else {
        format!("{indent}<Attributes/>")
    }
}

pub fn cfe_borrow_indent_form_fragment_for_base(value: &str) -> String {
    let mut result = String::new();
    for (idx, line) in value.split('\n').enumerate() {
        if idx == 0 {
            result.push_str("\t\t");
            result.push_str(line);
        } else {
            result.push('\t');
            result.push_str(line);
        }
        result.push_str("\r\n");
    }
    result
}

pub fn cfe_borrow_register_form(
    ext_dir: &Path,
    write_plan: &mut dyn TrackedSourceProposal,
    type_name: &str,
    object_name: &str,
    form_name: &str,
    log: &mut CfeBorrowLog,
) -> Result<(), String> {
    let object_file = cfe_borrow_target_object(ext_dir, type_name, object_name);
    if !write_plan.exists(&object_file) {
        log.warn(format!(
            "файл объекта-владельца не найден: {} — форма не зарегистрирована в составе",
            object_file.display()
        ));
        return Ok(());
    }
    let mut text = write_plan.read_utf8_sig(&object_file)?;
    let tag = format!(
        "<Form>{}</Form>",
        unica_xml_format::text::escape_xml(form_name)
    );
    if text.contains(&tag) {
        log.skipped(
            format!("{type_name}.{object_name}.Form.{form_name}"),
            "форма уже в составе объекта",
        );
        return Ok(());
    }
    if text.contains("<ChildObjects/>") {
        text = text.replacen(
            "<ChildObjects/>",
            &format!("<ChildObjects>\r\n\t\t\t{tag}\r\n\t\t</ChildObjects>"),
            1,
        );
    } else if text.contains("</ChildObjects>") {
        text = text.replacen(
            "</ChildObjects>",
            &format!("\t\t\t{tag}\r\n\t\t</ChildObjects>"),
            1,
        );
    } else {
        text = text.replacen(
            &format!("</{type_name}>"),
            &format!(
                "\t\t<ChildObjects>\r\n\t\t\t{tag}\r\n\t\t</ChildObjects>\r\n\t</{type_name}>"
            ),
            1,
        );
    }
    cfe_borrow_normalize_lxml_config_serialization(&mut text);
    write_plan.write_utf8_bom(&object_file, &text)?;
    Ok(())
}

pub fn cfe_borrow_xmlns_decl() -> &'static str {
    "xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\" xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\" xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\" xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\""
}

pub fn cfe_borrow_existing_metadata_uuid(
    reader: &dyn ProjectDiscoveryRead,
    path: &Path,
    child_name: &str,
) -> Option<String> {
    let text = read_string(reader, path).ok()?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    doc.root_element()
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == child_name)
        .and_then(|node| node.attribute("uuid"))
        .map(ToOwned::to_owned)
}

pub fn cfe_borrow_xml_node(node: roxmltree::Node<'_, '_>) -> String {
    if node.is_text() {
        return unica_xml_format::text::escape_xml(node.text().unwrap_or_default());
    }
    if !node.is_element() {
        return String::new();
    }
    let tag = cfe_borrow_prefixed_name(node.tag_name().namespace(), node.tag_name().name());
    let mut attrs = String::new();
    for attr in node.attributes() {
        let name = cfe_borrow_prefixed_name(attr.namespace(), attr.name());
        attrs.push_str(&format!(
            " {name}=\"{}\"",
            unica_xml_format::text::escape_xml(attr.value())
        ));
    }
    let children = node
        .children()
        .map(cfe_borrow_xml_node)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>();
    if children.is_empty() {
        format!("<{tag}{attrs}/>")
    } else {
        format!("<{tag}{attrs}>{}</{tag}>", children.join(""), tag = tag)
    }
}

pub fn cfe_borrow_prefixed_name(namespace: Option<&str>, local_name: &str) -> String {
    match namespace {
        Some("http://v8.1c.ru/8.1/data/core") => format!("v8:{local_name}"),
        Some("http://v8.1c.ru/8.3/xcf/readable") => format!("xr:{local_name}"),
        Some("http://www.w3.org/2001/XMLSchema-instance") => format!("xsi:{local_name}"),
        _ => local_name.to_string(),
    }
}

pub fn cfe_diff_objects_data(
    reader: &dyn ProjectDiscoveryRead,
    objects: &[CfeDiffObject],
    extension_path: &Path,
) -> (Vec<CfeDiffObjectData>, CfeDiffTotals) {
    let mut typed = Vec::new();
    let mut totals = CfeDiffTotals {
        borrowed: 0,
        own: 0,
    };
    for object in objects {
        let Some(info) =
            cfe_diff_object_info(reader, &object.obj_type, &object.name, extension_path)
        else {
            typed.push(CfeDiffObjectData {
                kind: object.obj_type.clone(),
                name: object.name.clone(),
                status: "unknownKind".to_string(),
                attributes: 0,
                forms: 0,
                tabular_sections: 0,
                borrowed_items: 0,
                form_names: Vec::new(),
                modules: Vec::new(),
            });
            continue;
        };
        let status = if !info.exists {
            "missing"
        } else if info.borrowed {
            totals.borrowed += 1;
            "borrowed"
        } else {
            totals.own += 1;
            "own"
        };
        let modules = if info.exists {
            cfe_diff_bsl_files(reader, &object.obj_type, &object.name, extension_path)
                .into_iter()
                .map(|bsl| CfeDiffModuleData {
                    path: cfe_diff_relative_path(&bsl, extension_path),
                    interceptors: cfe_diff_interceptors(reader, &bsl)
                        .into_iter()
                        .map(|interceptor| CfeDiffInterceptorData {
                            method: interceptor.method,
                            kind: interceptor.interceptor_type,
                        })
                        .collect(),
                })
                .collect()
        } else {
            Vec::new()
        };
        typed.push(CfeDiffObjectData {
            kind: object.obj_type.clone(),
            name: object.name.clone(),
            status: status.to_string(),
            attributes: info.attrs,
            forms: info.forms,
            tabular_sections: info.tabular_sections,
            borrowed_items: info.borrowed_items,
            form_names: info.form_names,
            modules,
        });
    }
    (typed, totals)
}

pub fn cfe_diff_transfer_data(
    reader: &dyn ProjectDiscoveryRead,
    objects: &[CfeDiffObject],
    extension_path: &Path,
    config_path: &Path,
) -> (Vec<CfeDiffTransferData>, CfeDiffTransferTotals) {
    let mut transfer = Vec::new();
    let mut totals = CfeDiffTransferTotals {
        transferred: 0,
        not_transferred: 0,
        needs_review: 0,
    };
    for object in objects {
        let Some(info) =
            cfe_diff_object_info(reader, &object.obj_type, &object.name, extension_path)
        else {
            continue;
        };
        if !info.exists || !info.borrowed {
            continue;
        }
        for bsl in cfe_diff_bsl_files(reader, &object.obj_type, &object.name, extension_path) {
            let controlled = cfe_diff_interceptors(reader, &bsl)
                .into_iter()
                .filter(|item| item.interceptor_type == "ИзменениеИКонтроль")
                .collect::<Vec<_>>();
            if controlled.is_empty() {
                continue;
            }
            let insert_blocks = cfe_diff_insertion_blocks(reader, &bsl);
            for interceptor in controlled {
                let mut push = |status: &str, blocks: usize, reason: Option<&str>| {
                    match status {
                        "transferred" => totals.transferred += 1,
                        "notTransferred" => totals.not_transferred += 1,
                        _ => totals.needs_review += 1,
                    }
                    transfer.push(CfeDiffTransferData {
                        kind: object.obj_type.clone(),
                        name: object.name.clone(),
                        method: interceptor.method.clone(),
                        status: status.to_string(),
                        blocks,
                        reason: reason.map(str::to_string),
                    });
                };
                if insert_blocks.is_empty() {
                    push("needsReview", 0, Some("no insertion blocks"));
                    continue;
                }
                let relative = bsl.strip_prefix(extension_path).unwrap_or(&bsl);
                let config_bsl = config_path.join(relative);
                if !reader.path_is_file(&config_bsl) {
                    push(
                        "needsReview",
                        insert_blocks.len(),
                        Some("configuration module not found"),
                    );
                    continue;
                }
                let config_norm =
                    cfe_diff_normalized_ws(&read_utf8_sig(reader, &config_bsl).unwrap_or_default());
                let all_transferred = insert_blocks.iter().all(|block| {
                    block.code.is_empty()
                        || config_norm.contains(&cfe_diff_normalized_ws(&block.code))
                });
                if all_transferred {
                    push("transferred", insert_blocks.len(), None);
                } else {
                    push("notTransferred", insert_blocks.len(), None);
                }
            }
        }
    }
    (transfer, totals)
}

pub fn cfe_diff_mode_a(
    reader: &dyn ProjectDiscoveryRead,
    lines: &mut Vec<String>,
    objects: &[CfeDiffObject],
    extension_path: &Path,
) {
    let mut borrowed = 0usize;
    let mut own = 0usize;

    for obj in objects {
        let Some(info) = cfe_diff_object_info(reader, &obj.obj_type, &obj.name, extension_path)
        else {
            lines.push(format!(
                "  [?] {}.{} — unknown type",
                obj.obj_type, obj.name
            ));
            continue;
        };
        if !info.exists {
            lines.push(format!(
                "  [?] {}.{} — file not found",
                obj.obj_type, obj.name
            ));
            continue;
        }

        if info.borrowed {
            borrowed += 1;
            lines.push(format!("  [BORROWED] {}.{}", obj.obj_type, obj.name));

            for bsl in cfe_diff_bsl_files(reader, &obj.obj_type, &obj.name, extension_path) {
                let rel_path = cfe_diff_relative_path(&bsl, extension_path);
                let interceptors = cfe_diff_interceptors(reader, &bsl);
                if interceptors.is_empty() {
                    lines.push(format!("             {rel_path} (no interceptors)"));
                } else {
                    for interceptor in interceptors {
                        lines.push(format!(
                            "             &{}(\"{}\") — line {} in {rel_path}",
                            interceptor.interceptor_type, interceptor.method, interceptor.line
                        ));
                    }
                }
            }

            let mut parts = Vec::<String>::new();
            if info.attrs > 0 {
                parts.push(format!("{} own attrs", info.attrs));
            }
            if info.tabular_sections > 0 {
                parts.push(format!("{} own TS", info.tabular_sections));
            }
            if info.forms > 0 {
                parts.push(format!("{} own forms", info.forms));
            }
            if info.borrowed_items > 0 {
                parts.push(format!("{} borrowed items", info.borrowed_items));
            }
            if !parts.is_empty() {
                lines.push(format!("             ChildObjects: {}", parts.join(", ")));
            }

            for form_name in &info.form_names {
                let form_xml_path = extension_path
                    .join(&info.dir_name)
                    .join(&obj.name)
                    .join("Forms")
                    .join(form_name)
                    .join("Ext")
                    .join("Form.xml");
                let Some(form_info) = cfe_diff_form_interceptors(reader, &form_xml_path) else {
                    lines.push(format!("             Form.{form_name} (?)"));
                    continue;
                };
                let form_tag = if form_info.0 { "borrowed" } else { "own" };
                if form_info.1.is_empty() {
                    lines.push(format!("             Form.{form_name} ({form_tag})"));
                } else {
                    lines.push(format!("             Form.{form_name} ({form_tag}):"));
                    for interceptor in form_info.1 {
                        lines.push(format!("               {interceptor}"));
                    }
                }
            }
        } else {
            own += 1;
            lines.push(format!("  [OWN]      {}.{}", obj.obj_type, obj.name));
            let mut parts = Vec::<String>::new();
            if info.attrs > 0 {
                parts.push(format!("{} attrs", info.attrs));
            }
            if info.tabular_sections > 0 {
                parts.push(format!("{} TS", info.tabular_sections));
            }
            if info.forms > 0 {
                parts.push(format!("{} forms", info.forms));
            }
            if !parts.is_empty() {
                lines.push(format!("             {}", parts.join(", ")));
            }
        }
    }

    lines.push(String::new());
    lines.push(format!(
        "=== Summary: {borrowed} borrowed, {own} own objects ==="
    ));
}

pub fn cfe_diff_mode_b(
    reader: &dyn ProjectDiscoveryRead,
    lines: &mut Vec<String>,
    objects: &[CfeDiffObject],
    extension_path: &Path,
    config_path: &Path,
) {
    let mut transferred = 0usize;
    let mut not_transferred = 0usize;
    let mut needs_review = 0usize;

    for obj in objects {
        let Some(info) = cfe_diff_object_info(reader, &obj.obj_type, &obj.name, extension_path)
        else {
            continue;
        };
        if !info.exists || !info.borrowed {
            continue;
        }

        for bsl in cfe_diff_bsl_files(reader, &obj.obj_type, &obj.name, extension_path) {
            let mac_interceptors = cfe_diff_interceptors(reader, &bsl)
                .into_iter()
                .filter(|item| item.interceptor_type == "ИзменениеИКонтроль")
                .collect::<Vec<_>>();
            if mac_interceptors.is_empty() {
                continue;
            }
            let insert_blocks = cfe_diff_insertion_blocks(reader, &bsl);
            for interceptor in mac_interceptors {
                if insert_blocks.is_empty() {
                    lines.push(format!(
                        "  [NEEDS_REVIEW] {}.{} — &ИзменениеИКонтроль(\"{}\") — no #Вставка blocks",
                        obj.obj_type, obj.name, interceptor.method
                    ));
                    needs_review += 1;
                    continue;
                }

                let rel_path = bsl.strip_prefix(extension_path).unwrap_or(&bsl);
                let config_bsl = config_path.join(rel_path);
                if !reader.path_is_file(&config_bsl) {
                    lines.push(format!(
                        "  [NEEDS_REVIEW] {}.{} — &ИзменениеИКонтроль(\"{}\") — config module not found",
                        obj.obj_type, obj.name, interceptor.method
                    ));
                    needs_review += 1;
                    continue;
                }

                let config_content = read_utf8_sig(reader, &config_bsl).unwrap_or_default();
                let config_norm = cfe_diff_normalized_ws(&config_content);
                let all_transferred = insert_blocks.iter().all(|block| {
                    block.code.is_empty()
                        || config_norm.contains(&cfe_diff_normalized_ws(&block.code))
                });
                if all_transferred {
                    lines.push(format!(
                        "  [TRANSFERRED]     {}.{} — &ИзменениеИКонтроль(\"{}\") — {} block(s)",
                        obj.obj_type,
                        obj.name,
                        interceptor.method,
                        insert_blocks.len()
                    ));
                    transferred += 1;
                } else {
                    lines.push(format!(
                        "  [NOT_TRANSFERRED] {}.{} — &ИзменениеИКонтроль(\"{}\") — some blocks not found in config",
                        obj.obj_type, obj.name, interceptor.method
                    ));
                    not_transferred += 1;
                }
            }
        }
    }

    lines.push(String::new());
    lines.push(format!(
        "=== Transfer check: {transferred} transferred, {not_transferred} not transferred, {needs_review} needs review ==="
    ));
}

pub fn cfe_diff_object_info(
    reader: &dyn ProjectDiscoveryRead,
    obj_type: &str,
    obj_name: &str,
    extension_path: &Path,
) -> Option<CfeDiffObjectInfo> {
    let dir_name = cf_validate_child_type_dir(obj_type)?;
    let obj_file = extension_path
        .join(dir_name)
        .join(format!("{obj_name}.xml"));
    if !reader.path_is_file(&obj_file) {
        return Some(CfeDiffObjectInfo {
            borrowed: false,
            exists: false,
            dir_name: dir_name.to_string(),
            attrs: 0,
            forms: 0,
            tabular_sections: 0,
            borrowed_items: 0,
            form_names: Vec::new(),
        });
    }

    let text = read_utf8_sig(reader, &obj_file).ok()?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    let obj_el = doc
        .root_element()
        .children()
        .find(|child| child.is_element())?;
    let props_el = unica_xml_format::read::child_element(obj_el, "Properties");
    let borrowed = props_el
        .and_then(|props| unica_xml_format::read::child_direct_text(props, "ObjectBelonging"))
        .map(|value| value == "Adopted")
        .unwrap_or(false);

    let mut attrs = 0usize;
    let mut forms = 0usize;
    let mut tabular_sections = 0usize;
    let mut borrowed_items = 0usize;
    let mut form_names = Vec::<String>::new();
    if let Some(child_objects) = unica_xml_format::read::child_element(obj_el, "ChildObjects") {
        for child in child_objects.children().filter(|node| node.is_element()) {
            if borrowed {
                let child_borrowed = unica_xml_format::read::child_element(child, "Properties")
                    .and_then(|props| {
                        unica_xml_format::read::child_direct_text(props, "ObjectBelonging")
                    })
                    .map(|value| value == "Adopted")
                    .unwrap_or(false);
                if child_borrowed {
                    borrowed_items += 1;
                    continue;
                }
            }
            match child.tag_name().name() {
                "Attribute" => attrs += 1,
                "TabularSection" => tabular_sections += 1,
                "Form" => {
                    forms += 1;
                    if borrowed {
                        form_names.push(child.text().unwrap_or("").to_string());
                    }
                }
                _ => {}
            }
        }
    }

    Some(CfeDiffObjectInfo {
        borrowed,
        exists: true,
        dir_name: dir_name.to_string(),
        attrs,
        forms,
        tabular_sections,
        borrowed_items,
        form_names,
    })
}

pub fn cfe_diff_bsl_files(
    reader: &dyn ProjectDiscoveryRead,
    obj_type: &str,
    obj_name: &str,
    extension_path: &Path,
) -> Vec<PathBuf> {
    let Some(dir_name) = cf_validate_child_type_dir(obj_type) else {
        return Vec::new();
    };
    let obj_dir = extension_path.join(dir_name).join(obj_name);
    if !path_is_dir(reader, &obj_dir) {
        return Vec::new();
    }

    let mut result = Vec::<PathBuf>::new();
    let ext_dir = obj_dir.join("Ext");
    if let Ok(entries) = reader.read_entries(&ext_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .and_then(|value| value.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("bsl"))
                .unwrap_or(false)
            {
                result.push(path);
            }
        }
    }
    let forms_dir = obj_dir.join("Forms");
    cfe_diff_collect_form_modules(reader, &forms_dir, &mut result);
    result.sort();
    result
}

pub fn cfe_diff_collect_form_modules(
    reader: &dyn ProjectDiscoveryRead,
    dir: &Path,
    result: &mut Vec<PathBuf>,
) {
    let Ok(entries) = reader.read_entries(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path_is_dir(reader, &path) {
            cfe_diff_collect_form_modules(reader, &path, result);
        } else if path.file_name().and_then(|value| value.to_str()) == Some("Module.bsl") {
            result.push(path);
        }
    }
}

pub fn cfe_diff_interceptors(
    reader: &dyn ProjectDiscoveryRead,
    bsl_path: &Path,
) -> Vec<CfeDiffInterceptor> {
    let Ok(text) = read_utf8_sig(reader, bsl_path) else {
        return Vec::new();
    };
    let mut result = Vec::<CfeDiffInterceptor>::new();
    for (idx, line) in text.lines().enumerate() {
        let stripped = line.trim();
        for interceptor_type in ["Перед", "После", "ИзменениеИКонтроль", "Вместо"]
        {
            let prefix = format!("&{interceptor_type}(\"");
            if let Some(rest) = stripped.strip_prefix(&prefix) {
                if let Some(end) = rest.find("\")") {
                    result.push(CfeDiffInterceptor {
                        interceptor_type: interceptor_type.to_string(),
                        method: rest[..end].to_string(),
                        line: idx + 1,
                    });
                }
            }
        }
    }
    result
}

pub fn cfe_diff_insertion_blocks(
    reader: &dyn ProjectDiscoveryRead,
    bsl_path: &Path,
) -> Vec<CfeDiffInsertionBlock> {
    let Ok(text) = read_utf8_sig(reader, bsl_path) else {
        return Vec::new();
    };
    let mut blocks = Vec::<CfeDiffInsertionBlock>::new();
    let mut in_block = false;
    let mut block_lines = Vec::<String>::new();
    for line in text.lines() {
        let stripped = line.trim();
        if stripped == "#Вставка" {
            in_block = true;
            block_lines.clear();
        } else if stripped == "#КонецВставки" && in_block {
            in_block = false;
            blocks.push(CfeDiffInsertionBlock {
                code: block_lines.join("\n").trim().to_string(),
            });
        } else if in_block {
            block_lines.push(line.trim_end_matches('\r').to_string());
        }
    }
    blocks
}

pub fn cfe_diff_form_interceptors(
    reader: &dyn ProjectDiscoveryRead,
    form_xml_path: &Path,
) -> Option<(bool, Vec<String>)> {
    const FORM_NS: &str = "http://v8.1c.ru/8.3/xcf/logform";
    let text = read_utf8_sig(reader, form_xml_path).ok()?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    let root = doc.root_element();
    let is_borrowed =
        unica_xml_format::read::child_element_in_namespace(root, "BaseForm", FORM_NS).is_some();
    let mut interceptors = Vec::<String>::new();

    if let Some(events) =
        unica_xml_format::read::child_element_in_namespace(root, "Events", FORM_NS)
    {
        for event in unica_xml_format::read::child_elements_in_namespace(events, "Event", FORM_NS) {
            let call_type = event.attribute("callType").unwrap_or("");
            if !call_type.is_empty() {
                let event_name = event.attribute("name").unwrap_or("");
                let event_text = event.text().unwrap_or("");
                interceptors.push(format!("Event:{event_name} [{call_type}] -> {event_text}"));
            }
        }
    }

    if let Some(child_items) =
        unica_xml_format::read::child_element_in_namespace(root, "ChildItems", FORM_NS)
    {
        for element in child_items.descendants().filter(|node| node.is_element()) {
            let element_name = element.attribute("name").unwrap_or("");
            if element_name.is_empty() {
                continue;
            }
            let Some(events) =
                unica_xml_format::read::child_element_in_namespace(element, "Events", FORM_NS)
            else {
                continue;
            };
            for event in
                unica_xml_format::read::child_elements_in_namespace(events, "Event", FORM_NS)
            {
                let call_type = event.attribute("callType").unwrap_or("");
                if !call_type.is_empty() {
                    let event_name = event.attribute("name").unwrap_or("");
                    let event_text = event.text().unwrap_or("");
                    interceptors.push(format!(
                        "Element:{element_name}.{event_name} [{call_type}] -> {event_text}"
                    ));
                }
            }
        }
    }

    if let Some(commands) =
        unica_xml_format::read::child_element_in_namespace(root, "Commands", FORM_NS)
    {
        for command in
            unica_xml_format::read::child_elements_in_namespace(commands, "Command", FORM_NS)
        {
            let command_name = command.attribute("name").unwrap_or("");
            for action in
                unica_xml_format::read::child_elements_in_namespace(command, "Action", FORM_NS)
            {
                let call_type = action.attribute("callType").unwrap_or("");
                if !call_type.is_empty() {
                    let action_text = action.text().unwrap_or("");
                    interceptors.push(format!(
                        "Command:{command_name} [{call_type}] -> {action_text}"
                    ));
                }
            }
        }
    }

    Some((is_borrowed, interceptors))
}

pub fn cfe_diff_relative_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

pub fn cfe_diff_normalized_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn cfe_validation_finish(
    report: CfeValidationReporter,
    artifact: PathBuf,
) -> Result<CfeValidationRun, String> {
    let (ok, stdout, errors) = report.finalize();
    Ok(CfeValidationRun {
        ok,
        stdout,
        artifact,
        errors,
    })
}

pub fn cfe_validate_internal_info(
    report: &mut CfeValidationReporter,
    cfg_node: roxmltree::Node<'_, '_>,
) {
    let Some(internal_info) = unica_xml_format::read::child_element(cfg_node, "InternalInfo")
    else {
        report.error("2. InternalInfo: missing");
        return;
    };
    let contained = unica_xml_format::read::child_elements(internal_info, "ContainedObject");
    if contained.len() != 7 {
        report.warn(format!(
            "2. InternalInfo: expected 7 ContainedObject, found {}",
            contained.len()
        ));
    }
    let mut check_ok = true;
    let mut found = HashSet::<String>::new();
    for item in &contained {
        let class_id =
            unica_xml_format::read::child_direct_text(*item, "ClassId").unwrap_or_default();
        let object_id =
            unica_xml_format::read::child_direct_text(*item, "ObjectId").unwrap_or_default();
        if class_id.is_empty() {
            report.error("2. ContainedObject missing ClassId");
            check_ok = false;
            continue;
        }
        if !cf_validate_class_ids().contains(&class_id.as_str()) {
            report.error(format!("2. Unknown ClassId: {class_id}"));
            check_ok = false;
        }
        if !found.insert(class_id.clone()) {
            report.error(format!("2. Duplicate ClassId: {class_id}"));
            check_ok = false;
        }
        if object_id.is_empty() {
            report.error(format!(
                "2. ContainedObject missing ObjectId for ClassId {class_id}"
            ));
            check_ok = false;
        } else if !cf_validate_guid(&object_id) {
            report.error(format!(
                "2. Invalid ObjectId '{object_id}' for ClassId {class_id}"
            ));
            check_ok = false;
        }
    }
    let missing = cf_validate_class_ids()
        .iter()
        .filter(|class_id| !found.contains(**class_id))
        .count();
    if missing > 0 {
        report.warn(format!("2. Missing ClassIds: {missing} of 7"));
    }
    if check_ok {
        report.ok(format!(
            "2. InternalInfo: {} ContainedObject, all ClassIds valid",
            contained.len()
        ));
    }
}

pub fn cfe_validate_properties(
    report: &mut CfeValidationReporter,
    props_node: Option<roxmltree::Node<'_, '_>>,
    obj_name: &str,
) -> String {
    let Some(props_node) = props_node else {
        report.error("3. Properties block missing");
        return String::new();
    };
    let mut check_ok = true;
    let object_belonging = unica_xml_format::read::child_direct_text(props_node, "ObjectBelonging")
        .unwrap_or_default();
    if object_belonging != "Adopted" {
        report.error(format!(
            "3. ObjectBelonging must be 'Adopted', got '{object_belonging}'"
        ));
        check_ok = false;
    }
    if obj_name == "(unknown)" || obj_name.is_empty() {
        report.error("3. Name is missing or empty");
        check_ok = false;
    } else if !unica_metadata::identifier::is_1c_identifier(obj_name) {
        report.error(format!("3. Name '{obj_name}' is not a valid 1C identifier"));
        check_ok = false;
    }
    let purpose =
        unica_xml_format::read::child_direct_text(props_node, "ConfigurationExtensionPurpose")
            .unwrap_or_default();
    if purpose.is_empty() {
        report.error("3. ConfigurationExtensionPurpose is missing");
        check_ok = false;
    } else if !["Patch", "Customization", "AddOn"].contains(&purpose.as_str()) {
        report.error(format!(
            "3. ConfigurationExtensionPurpose '{purpose}' invalid (expected: Patch, Customization, AddOn)"
        ));
        check_ok = false;
    }
    let prefix =
        unica_xml_format::read::child_direct_text(props_node, "NamePrefix").unwrap_or_default();
    if prefix.is_empty() {
        report.warn("3. NamePrefix is empty");
    }
    if unica_xml_format::read::child_element(
        props_node,
        "KeepMappingToExtendedConfigurationObjectsByIDs",
    )
    .is_none()
    {
        report.warn("3. KeepMappingToExtendedConfigurationObjectsByIDs is missing");
    }
    let def_lang = unica_xml_format::read::child_direct_text(props_node, "DefaultLanguage")
        .unwrap_or_default();
    if check_ok {
        let prefix_text = if prefix.is_empty() {
            "(empty)"
        } else {
            prefix.as_str()
        };
        let purpose_text = if purpose.is_empty() {
            "?"
        } else {
            purpose.as_str()
        };
        report.ok(format!(
            "3. Extension properties: Name=\"{obj_name}\", Purpose={purpose_text}, Prefix={prefix_text}"
        ));
    }
    def_lang
}

pub fn cfe_validate_enum_properties(
    report: &mut CfeValidationReporter,
    props_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props_node) = props_node else {
        report.warn("4. No Properties block to check");
        return;
    };
    let mut checked = 0usize;
    let mut check_ok = true;
    for property in cfe_validate_enum_properties_list() {
        let allowed = cfe_validate_enum_allowed(property);
        if let Some(value) = unica_xml_format::read::child_direct_text(props_node, property)
            .filter(|value| !value.is_empty())
        {
            if !allowed.contains(&value.as_str()) {
                report.error(format!(
                    "4. Property '{property}' has invalid value '{value}'"
                ));
                check_ok = false;
            }
            checked += 1;
        }
    }
    if check_ok {
        report.ok(format!(
            "4. Property values: {checked} enum properties checked"
        ));
    }
}

pub fn cfe_validate_child_objects(
    report: &mut CfeValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) -> HashMap<String, HashSet<String>> {
    let mut child_index = HashMap::<String, HashSet<String>>::new();
    let Some(child_obj_node) = child_obj_node else {
        report.error("5. ChildObjects block missing");
        return child_index;
    };
    let mut check_ok = true;
    let mut total_count = 0usize;
    let mut duplicates = HashSet::<String>::new();
    let mut last_type_order = 0usize;
    let mut order_ok = true;
    let mut first_type = true;
    for child in child_obj_node.children().filter(|child| child.is_element()) {
        let type_name = child.tag_name().name();
        let object_name = child.text().unwrap_or("").to_string();
        if let Some(type_index) =
            unica_source_designer::registration::cf_validate_child_object_type_index(type_name)
        {
            if !first_type && type_index < last_type_order {
                report.warn(format!(
                    "5. Type '{type_name}' is out of canonical order (after type at position {last_type_order})"
                ));
                order_ok = false;
            }
            if first_type || type_index >= last_type_order {
                last_type_order = type_index;
            }
            first_type = false;
        } else {
            report.error(format!("5. Unknown type '{type_name}' in ChildObjects"));
            check_ok = false;
        }
        let type_items = child_index.entry(type_name.to_string()).or_default();
        if !type_items.insert(object_name.clone()) {
            let dup_key = format!("{type_name}.{object_name}");
            if duplicates.insert(dup_key.clone()) {
                report.error(format!("5. Duplicate: {dup_key}"));
                check_ok = false;
            }
        }
        total_count += 1;
    }
    if check_ok {
        let order_info = if order_ok { ", order correct" } else { "" };
        report.ok(format!(
            "5. ChildObjects: {} types, {total_count} objects{order_info}",
            child_index.len()
        ));
    }
    child_index
}

pub fn cfe_validate_default_language(
    report: &mut CfeValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
    def_lang: &str,
) {
    if def_lang.is_empty() {
        report.warn("6. Cannot check DefaultLanguage (empty)");
        return;
    }
    let Some(child_obj_node) = child_obj_node else {
        report.warn("6. Cannot check DefaultLanguage (no ChildObjects)");
        return;
    };
    let lang_name = def_lang.strip_prefix("Language.").unwrap_or(def_lang);
    let found = unica_xml_format::read::child_elements(child_obj_node, "Language")
        .iter()
        .any(|child| child.text().unwrap_or("") == lang_name);
    if found {
        report.ok(format!(
            "6. DefaultLanguage \"{def_lang}\" found in ChildObjects"
        ));
    } else {
        report.error(format!(
            "6. DefaultLanguage \"{def_lang}\" not found in ChildObjects"
        ));
    }
}

pub fn cfe_validate_language_files(
    reader: &dyn ProjectDiscoveryRead,
    report: &mut CfeValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
    config_dir: &Path,
) {
    let Some(child_obj_node) = child_obj_node else {
        report.warn("7. Cannot check language files (no ChildObjects)");
        return;
    };
    let lang_names = unica_xml_format::read::child_elements(child_obj_node, "Language")
        .into_iter()
        .map(|child| child.text().unwrap_or("").to_string())
        .collect::<Vec<_>>();
    if lang_names.is_empty() {
        report.warn("7. No Language entries in ChildObjects");
        return;
    }
    let mut exist_count = 0usize;
    for lang_name in &lang_names {
        if path_exists(
            reader,
            &config_dir
                .join("Languages")
                .join(format!("{lang_name}.xml")),
        ) {
            exist_count += 1;
        } else {
            report.warn(format!(
                "7. Language file missing: Languages/{lang_name}.xml"
            ));
        }
    }
    if exist_count == lang_names.len() {
        report.ok(format!(
            "7. Language files: {exist_count}/{} exist",
            lang_names.len()
        ));
    }
}

pub fn cfe_validate_object_dirs(
    reader: &dyn ProjectDiscoveryRead,
    report: &mut CfeValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
    config_dir: &Path,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let mut dirs = HashMap::<String, usize>::new();
    for child in child_obj_node.children().filter(|child| child.is_element()) {
        let type_name = child.tag_name().name();
        if type_name == "Language" {
            continue;
        }
        if let Some(dir_name) = cf_validate_child_type_dir(type_name) {
            *dirs.entry(dir_name.to_string()).or_default() += 1;
        }
    }
    let mut missing = dirs
        .iter()
        .filter(|(dir_name, _)| !path_is_dir(reader, &config_dir.join(*dir_name)))
        .map(|(dir_name, count)| format!("{dir_name} ({count} objects)"))
        .collect::<Vec<_>>();
    missing.sort();
    if missing.is_empty() {
        report.ok(format!(
            "8. Object directories: {} directories, all exist",
            dirs.len()
        ));
    } else {
        for missing_dir in missing {
            report.warn(format!("8. Missing directory: {missing_dir}"));
        }
    }
}

pub fn cfe_validate_service_objects(
    reader: &dyn ProjectDiscoveryRead,
    report: &mut CfeValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
    config_dir: &Path,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let mut service_count = 0usize;
    let mut unreadable_count = 0usize;
    for child in child_obj_node.children().filter(|child| child.is_element()) {
        let type_name = child.tag_name().name();
        if !matches!(type_name, "HTTPService" | "WebService") {
            continue;
        }
        let child_name = child.text().unwrap_or("");
        let Some(dir_name) = cf_validate_child_type_dir(type_name) else {
            continue;
        };
        let obj_file = config_dir.join(dir_name).join(format!("{child_name}.xml"));
        let Ok(text) = read_utf8_sig(reader, &obj_file) else {
            unreadable_count += 1;
            report.warn(format!(
                "14. {type_name}.{child_name}: cannot read {dir_name}/{child_name}.xml"
            ));
            continue;
        };
        let Ok(doc) = Document::parse(text.trim_start_matches('\u{feff}')) else {
            unreadable_count += 1;
            report.warn(format!(
                "14. {type_name}.{child_name}: cannot parse {dir_name}/{child_name}.xml"
            ));
            continue;
        };
        let Some(obj_el) = doc.root_element().children().find(|node| node.is_element()) else {
            continue;
        };
        let Some(semantics) = service_child_semantics(
            type_name,
            unica_xml_format::read::child_element(obj_el, "ChildObjects"),
        ) else {
            continue;
        };
        service_count += 1;
        let context_label = format!("{type_name}.{child_name}");
        for finding in &semantics.findings {
            let message = format!("14. {context_label} {}", finding.detail);
            if finding.error {
                report.error(message);
            } else {
                report.warn(message);
            }
        }
        if semantics.clean {
            report.ok(format!("14. {context_label}: {}", semantics.summary));
        }
        if report.stopped {
            return;
        }
    }
    if service_count == 0 && unreadable_count == 0 {
        report.ok("14. Services: none found");
    }
}

type CfeModuleLayouts<'a> =
    dyn FnMut(&str, &str) -> Result<Vec<(PathBuf, &'static str)>, String> + 'a;

pub fn cfe_validate_borrowed_objects(
    module_layouts: &mut CfeModuleLayouts<'_>,
    reader: &dyn ProjectDiscoveryRead,
    report: &mut CfeValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
    config_dir: &Path,
    _child_index: &HashMap<String, HashSet<String>>,
) -> Vec<CfeBorrowedForm> {
    let mut forms = Vec::new();
    let Some(child_obj_node) = child_obj_node else {
        return forms;
    };
    let mut borrowed_count = 0usize;
    let mut borrowed_ok_count = 0usize;
    let mut check9_ok = true;
    let mut check10_ok = true;
    let mut sub_item_count = 0usize;
    let mut module_count = 0usize;
    for child in child_obj_node.children().filter(|child| child.is_element()) {
        let type_name = child.tag_name().name();
        let child_name = child.text().unwrap_or("");
        if type_name == "Language" {
            continue;
        }
        let Some(dir_name) = cf_validate_child_type_dir(type_name) else {
            continue;
        };
        let obj_file = config_dir.join(dir_name).join(format!("{child_name}.xml"));
        if !path_exists(reader, &obj_file) {
            continue;
        }
        let Ok(text) = read_utf8_sig(reader, &obj_file) else {
            continue;
        };
        let Ok(doc) = Document::parse(text.trim_start_matches('\u{feff}')) else {
            report.warn(format!("9. Cannot parse {dir_name}/{child_name}.xml"));
            continue;
        };
        let Some(obj_el) = doc.root_element().children().find(|node| node.is_element()) else {
            continue;
        };
        let context = format!("{type_name}.{child_name}");
        match unica_source_designer::layout::metadata_kind_requires_child_objects_8_3_27(type_name)
        {
            Some(true) => {
                if !cfe_validate_has_exact_md_child(obj_el, "ChildObjects") {
                    report.error(format!("9. {context}: ChildObjects is required"));
                    check9_ok = false;
                } else if !cfe_validate_md_child_follows(obj_el, "ChildObjects", "Properties") {
                    report.error(format!("9. {context}: ChildObjects must follow Properties"));
                    check9_ok = false;
                }
            }
            Some(false) => {}
            None => {
                report.error(format!("9. {context}: no ChildObjects profile"));
                check9_ok = false;
            }
        }
        let Some(obj_props) = unica_xml_format::read::child_element(obj_el, "Properties") else {
            continue;
        };
        let borrowed = unica_xml_format::read::child_direct_text(obj_props, "ObjectBelonging")
            .as_deref()
            == Some("Adopted");
        if borrowed {
            borrowed_count += 1;
            let extended =
                unica_xml_format::read::child_direct_text(obj_props, "ExtendedConfigurationObject")
                    .unwrap_or_default();
            if extended.is_empty() {
                report.error(format!(
                    "9. Borrowed {type_name}.{child_name}: missing ExtendedConfigurationObject"
                ));
                check9_ok = false;
            } else if !cf_validate_guid(&extended) {
                report.error(format!(
                    "9. Borrowed {type_name}.{child_name}: invalid ExtendedConfigurationObject UUID '{extended}'"
                ));
                check9_ok = false;
            } else {
                borrowed_ok_count += 1;
            }
            match module_layouts(type_name, child_name) {
                Ok(modules) => {
                    for (relative_module, property) in modules {
                        if reader.path_is_file(&config_dir.join(&relative_module)) {
                            module_count += 1;
                            if !cfe_validate_has_extended_property_state(obj_el, property) {
                                report.error(format!(
                                    "9. {context}: Ext/{property}.bsl exists but PropertyState {property}=Extended is missing or invalid"
                                ));
                                check9_ok = false;
                            }
                        }
                    }
                }
                Err(error) => {
                    report.error(format!("9. {context}: {error}"));
                    check9_ok = false;
                }
            }
        }
        if let Some(child_objects) = unica_xml_format::read::child_element(obj_el, "ChildObjects") {
            for sub_item in child_objects.children().filter(|node| node.is_element()) {
                let sub_type = sub_item.tag_name().name();
                if matches!(sub_type, "Attribute" | "TabularSection" | "EnumValue")
                    && cfe_is_borrowed_sub_item(sub_item)
                {
                    sub_item_count += 1;
                    if !cfe_validate_borrowed_sub_item(report, "10", &context, sub_type, sub_item) {
                        check10_ok = false;
                    }
                } else if sub_type == "Form" {
                    let form_name = sub_item.text().unwrap_or("");
                    if !form_name.is_empty() {
                        let form_meta = config_dir
                            .join(dir_name)
                            .join(child_name)
                            .join("Forms")
                            .join(format!("{form_name}.xml"));
                        if !path_exists(reader, &form_meta) {
                            report.error(format!(
                                "10. {context}: Form.{form_name} metadata file missing"
                            ));
                            check10_ok = false;
                        }
                        let form_xml = config_dir
                            .join(dir_name)
                            .join(child_name)
                            .join("Forms")
                            .join(form_name)
                            .join("Ext")
                            .join("Form.xml");
                        if let Ok(raw_text) = read_utf8_sig(reader, &form_xml) {
                            forms.push(CfeBorrowedForm {
                                raw_text,
                                context: format!("{context}.Form.{form_name}"),
                            });
                        }
                        sub_item_count += 1;
                    }
                }
            }
        }
    }
    if borrowed_count == 0 {
        report.ok("9. Borrowed objects: none found");
    } else if check9_ok {
        report.ok(format!(
            "9. Borrowed objects: {borrowed_ok_count}/{borrowed_count} validated; {module_count} module state(s) checked"
        ));
    }
    if sub_item_count == 0 {
        report.ok("10. Sub-items: none found");
    } else if check10_ok {
        report.ok(format!(
            "10. Sub-items: {sub_item_count} validated (Attributes, TabularSections, EnumValues, Forms)"
        ));
    }
    forms
}

pub fn cfe_validate_has_extended_property_state(
    object: roxmltree::Node<'_, '_>,
    property: &str,
) -> bool {
    if !cfe_validate_has_exact_md_child(object, "InternalInfo") {
        return false;
    }
    let internal = object
        .children()
        .find(|node| node.has_tag_name((CFE_PATCH_MD_NAMESPACE, "InternalInfo")))
        .expect("exact InternalInfo verified");
    crate::property_states::read_property_states(internal)
        .is_ok_and(|states| states.get(property) == Some(&"Extended"))
}

pub fn cfe_validate_has_exact_md_child(object: roxmltree::Node<'_, '_>, local_name: &str) -> bool {
    let matching = object
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == local_name)
        .collect::<Vec<_>>();
    matching.len() == 1 && matching[0].tag_name().namespace() == Some(CFE_PATCH_MD_NAMESPACE)
}

pub fn cfe_validate_md_child_follows(
    object: roxmltree::Node<'_, '_>,
    child_name: &str,
    predecessor_name: &str,
) -> bool {
    let children = object
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    let child = children
        .iter()
        .position(|node| node.has_tag_name((CFE_PATCH_MD_NAMESPACE, child_name)));
    let predecessor = children
        .iter()
        .position(|node| node.has_tag_name((CFE_PATCH_MD_NAMESPACE, predecessor_name)));
    matches!((predecessor, child), (Some(before), Some(after)) if before < after)
}

pub fn cfe_is_borrowed_sub_item(sub_item: roxmltree::Node<'_, '_>) -> bool {
    let Some(props) = unica_xml_format::read::child_element(sub_item, "Properties") else {
        return false;
    };
    unica_xml_format::read::child_direct_text(props, "ObjectBelonging")
        .is_some_and(|value| !value.is_empty())
        || unica_xml_format::read::child_direct_text(props, "ExtendedConfigurationObject")
            .is_some_and(|value| !value.is_empty())
}

pub fn cfe_validate_borrowed_sub_item(
    report: &mut CfeValidationReporter,
    check_num: &str,
    context: &str,
    sub_type: &str,
    sub_item: roxmltree::Node<'_, '_>,
) -> bool {
    let Some(props) = unica_xml_format::read::child_element(sub_item, "Properties") else {
        report.error(format!(
            "{check_num}. {context}: {sub_type} missing Properties"
        ));
        return false;
    };
    let mut ok = true;
    if unica_xml_format::read::child_direct_text(props, "ObjectBelonging").as_deref()
        != Some("Adopted")
    {
        report.error(format!(
            "{check_num}. {context}: {sub_type} ObjectBelonging must be 'Adopted'"
        ));
        ok = false;
    }
    let name = unica_xml_format::read::child_direct_text(props, "Name").unwrap_or_default();
    if name.is_empty() {
        report.error(format!("{check_num}. {context}: {sub_type} missing Name"));
        ok = false;
    }
    let extended = unica_xml_format::read::child_direct_text(props, "ExtendedConfigurationObject")
        .unwrap_or_default();
    if extended.is_empty() {
        report.error(format!(
            "{check_num}. {context}: {sub_type}.{name} missing ExtendedConfigurationObject"
        ));
        ok = false;
    } else if !cf_validate_guid(&extended) {
        report.error(format!(
            "{check_num}. {context}: {sub_type}.{name} invalid ExtendedConfigurationObject"
        ));
        ok = false;
    }
    ok
}

pub fn cfe_validate_borrowed_forms(
    report: &mut CfeValidationReporter,
    borrowed_forms: &[CfeBorrowedForm],
) {
    if borrowed_forms.is_empty() {
        report.ok("11. Borrowed forms: none found");
    } else {
        let with_base = borrowed_forms
            .iter()
            .filter(|form| form.raw_text.contains("<BaseForm"))
            .count();
        report.ok(format!(
            "11. Borrowed forms: {} validated ({with_base} with BaseForm)",
            borrowed_forms.len()
        ));
    }
}

pub fn cfe_validate_form_dependencies(
    report: &mut CfeValidationReporter,
    borrowed_forms: &[CfeBorrowedForm],
    child_index: &HashMap<String, HashSet<String>>,
) {
    if borrowed_forms.is_empty() {
        report.ok("12. Form dependencies: no borrowed forms with tree");
        return;
    }
    let mut check_ok = true;
    let mut dep_count = 0usize;
    for form in borrowed_forms {
        for picture in cfe_capture_refs(&form.raw_text, "<xr:Ref>CommonPicture.", "</xr:Ref>") {
            dep_count += 1;
            if !child_index
                .get("CommonPicture")
                .is_some_and(|items| items.contains(&picture))
            {
                report.warn(format!(
                    "12. {}: references CommonPicture.{picture} not borrowed in extension",
                    form.context
                ));
                check_ok = false;
            }
        }
    }
    if check_ok {
        report.ok(format!(
            "12. Form dependencies: {dep_count} references checked"
        ));
    }
}

pub fn cfe_validate_typelinks(
    report: &mut CfeValidationReporter,
    borrowed_forms: &[CfeBorrowedForm],
) {
    if borrowed_forms.is_empty() {
        report.ok("13. TypeLink: no borrowed forms with tree");
        return;
    }
    let mut check_ok = true;
    for form in borrowed_forms {
        let count = form
            .raw_text
            .matches("<TypeLink>")
            .filter(|_| form.raw_text.contains("<xr:DataPath>Items."))
            .count();
        if count > 0 {
            report.warn(format!(
                "13. {}: {count} TypeLink(s) with human-readable Items.* DataPath (should be stripped)",
                form.context
            ));
            check_ok = false;
        }
    }
    if check_ok {
        report.ok("13. TypeLink: clean");
    }
}

pub fn cfe_capture_refs(raw: &str, prefix: &str, suffix: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = raw;
    while let Some(start) = rest.find(prefix) {
        let value_start = start + prefix.len();
        let Some(end) = rest[value_start..].find(suffix) else {
            break;
        };
        values.push(rest[value_start..value_start + end].to_string());
        rest = &rest[value_start + end + suffix.len()..];
    }
    values
}

pub fn cfe_init_name_prefix(args: &Map<String, Value>, name: &str) -> String {
    unica_operation_contract::input::string_arg(args, &["namePrefix", "NamePrefix"])
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("{name}_"))
}

pub fn cfe_patch_module_target(
    extension_path: &Path,
    module_path: &str,
) -> Result<CfePatchModuleTarget, String> {
    let parts = module_path.split('.').collect::<Vec<_>>();
    let invalid_format = || {
        format!(
            "Invalid ModulePath format: {module_path}. Expected exactly Type.Name.Module, Type.Name.Form.FormName, or CommonModule.Name"
        )
    };

    match parts.as_slice() {
        ["CommonModule", object_name] => {
            cfe_validate_metadata_name("ModulePath object name", object_name)?;
            Ok(CfePatchModuleTarget {
                type_name: "CommonModule",
                object_name: (*object_name).to_string(),
                role: CfePatchModuleRole::CommonModule,
                form_name: None,
                descriptor: extension_path
                    .join("CommonModules")
                    .join(format!("{object_name}.xml")),
                bsl_file: extension_path
                    .join("CommonModules")
                    .join(object_name)
                    .join("Ext")
                    .join("Module.bsl"),
            })
        }
        [type_name, object_name, "Form", form_name] => {
            cfe_validate_metadata_name("ModulePath object name", object_name)?;
            cfe_validate_metadata_name("ModulePath form name", form_name)?;
            if !cfe_patch_form_role_is_supported(type_name) {
                return Err(cfe_patch_unsupported_role(module_path));
            }
            let directory = cf_validate_child_type_dir(type_name)
                .ok_or_else(|| format!("Unknown object type: {type_name}"))?;
            Ok(CfePatchModuleTarget {
                type_name: cfe_patch_canonical_type(type_name)
                    .ok_or_else(|| cfe_patch_unsupported_role(module_path))?,
                object_name: (*object_name).to_string(),
                role: CfePatchModuleRole::Form,
                form_name: Some((*form_name).to_string()),
                descriptor: extension_path
                    .join(directory)
                    .join(format!("{object_name}.xml")),
                bsl_file: extension_path
                    .join(directory)
                    .join(object_name)
                    .join("Forms")
                    .join(form_name)
                    .join("Ext")
                    .join("Form")
                    .join("Module.bsl"),
            })
        }
        [type_name, object_name, module_name] if *type_name != "CommonModule" => {
            cfe_validate_metadata_name("ModulePath object name", object_name)?;
            cfe_validate_metadata_name("ModulePath module name", module_name)?;
            let role = match *module_name {
                "ObjectModule" => CfePatchModuleRole::ObjectModule,
                "ManagerModule" => CfePatchModuleRole::ManagerModule,
                "RecordSetModule" => CfePatchModuleRole::RecordSetModule,
                "ValueManagerModule" => CfePatchModuleRole::ValueManagerModule,
                _ => return Err(cfe_patch_unsupported_role(module_path)),
            };
            if !cfe_patch_direct_role_is_supported(type_name, role) {
                return Err(cfe_patch_unsupported_role(module_path));
            }
            let directory = cf_validate_child_type_dir(type_name)
                .ok_or_else(|| format!("Unknown object type: {type_name}"))?;
            Ok(CfePatchModuleTarget {
                type_name: cfe_patch_canonical_type(type_name)
                    .ok_or_else(|| cfe_patch_unsupported_role(module_path))?,
                object_name: (*object_name).to_string(),
                role,
                form_name: None,
                descriptor: extension_path
                    .join(directory)
                    .join(format!("{object_name}.xml")),
                bsl_file: extension_path
                    .join(directory)
                    .join(object_name)
                    .join("Ext")
                    .join(format!("{module_name}.bsl")),
            })
        }
        _ => Err(invalid_format()),
    }
}

pub fn cfe_patch_unsupported_role(module_path: &str) -> String {
    format!(
        "ModulePath '{module_path}' is not supported by the cfe.patch_method grammar for platform 1C 8.3.27. Supported paths are CommonModule.Name, supported Type.Name.ObjectModule/ManagerModule/RecordSetModule/ValueManagerModule roles, and supported Type.Name.Form.FormName"
    )
}

pub fn cfe_patch_validate_bsl_identifier(argument: &str, value: &str) -> Result<(), String> {
    if unica_metadata::identifier::is_1c_identifier(value)
        && unica_xml_format::names::xml_ncname_is_valid(value)
    {
        Ok(())
    } else {
        Err(format!(
            "{argument} must be a valid 1C identifier and Unicode XML NCName: {value:?}"
        ))
    }
}

pub fn cfe_patch_supported_document<'a>(
    path: &Path,
    raw: &'a [u8],
) -> Result<Document<'a>, String> {
    let text = std::str::from_utf8(raw)
        .map_err(|error| format!("{} is not valid UTF-8: {error}", path.display()))?;
    let source = text.trim_start_matches('\u{feff}');
    let document = Document::parse(source)
        .map_err(|error| format!("failed to parse platform XML {}: {error}", path.display()))?;
    let version_literal = root_version_literal(source, document.root_element());
    match classify_root_version(version_literal.as_deref()).map_err(|error| error.to_string())? {
        FormatCompatibility::Supported { .. } => Ok(document),
        compatibility => {
            Err(unica_source_designer::format_profile::format_compatibility_warning(&compatibility))
        }
    }
}

pub fn cfe_patch_exact_md_text(
    parent: roxmltree::Node<'_, '_>,
    local_name: &str,
    module_path: &str,
    path: &Path,
) -> Result<String, String> {
    let child = cfe_patch_exact_md_child(parent, local_name, module_path, path)?;
    Ok(child.text().unwrap_or_default().to_string())
}

pub fn cfe_patch_required_exact_md_bool(
    parent: roxmltree::Node<'_, '_>,
    local_name: &str,
    module_path: &str,
    path: &Path,
) -> Result<bool, String> {
    match cfe_patch_exact_md_text(parent, local_name, module_path, path)?.as_str() {
        "false" => Ok(false),
        "true" => Ok(true),
        value => Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "{local_name} in {} must be the exact boolean true or false, got {value:?}",
                path.display()
            ),
        )),
    }
}

pub fn cfe_patch_optional_exact_md_bool(
    parent: roxmltree::Node<'_, '_>,
    local_name: &str,
    module_path: &str,
    path: &Path,
) -> Result<Option<bool>, String> {
    let children = parent
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == local_name)
        .collect::<Vec<_>>();
    if children.is_empty() {
        return Ok(None);
    }
    if children.len() != 1 || children[0].tag_name().namespace() != Some(CFE_PATCH_MD_NAMESPACE) {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "{} must contain at most one direct {{{CFE_PATCH_MD_NAMESPACE}}}{local_name}",
                path.display()
            ),
        ));
    }
    match children[0].text().unwrap_or_default() {
        "false" => Ok(Some(false)),
        "true" => Ok(Some(true)),
        value => Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "{local_name} in {} must be the exact boolean true or false, got {value:?}",
                path.display()
            ),
        )),
    }
}

pub fn cfe_patch_validate_non_nil_uuid(
    module_path: &str,
    value: Option<&str>,
    field: &str,
    path: &Path,
) -> Result<(), String> {
    let value = value.unwrap_or_default();
    if !cf_validate_guid(value) || value.eq_ignore_ascii_case(CFE_PATCH_NIL_UUID) {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!("{field} in {} must be a valid non-nil UUID", path.display()),
        ));
    }
    Ok(())
}

pub fn cfe_patch_validate_adopted_descriptor(
    module_path: &str,
    path: &Path,
    raw: &[u8],
    expected_type: &str,
    expected_name: &str,
) -> Result<Option<CfePatchCommonModuleCapabilities>, String> {
    let document = cfe_patch_supported_document(path, raw)?;
    let object = cfe_patch_direct_md_child(&document, expected_type, module_path, path)?;
    cfe_patch_validate_non_nil_uuid(module_path, object.attribute("uuid"), "uuid", path)?;
    let properties = cfe_patch_exact_md_child(object, "Properties", module_path, path)?;
    if cfe_patch_exact_md_text(properties, "Name", module_path, path)? != expected_name {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "Name in {} must exactly match '{expected_name}'",
                path.display()
            ),
        ));
    }
    if cfe_patch_exact_md_text(properties, "ObjectBelonging", module_path, path)? != "Adopted" {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!("ObjectBelonging in {} must be Adopted", path.display()),
        ));
    }
    let extended =
        cfe_patch_exact_md_text(properties, "ExtendedConfigurationObject", module_path, path)?;
    cfe_patch_validate_non_nil_uuid(
        module_path,
        Some(&extended),
        "ExtendedConfigurationObject",
        path,
    )?;
    if expected_type != "CommonModule" {
        return Ok(None);
    }
    let global = cfe_patch_required_exact_md_bool(properties, "Global", module_path, path)?;
    let capabilities = CfePatchCommonModuleCapabilities {
        client_managed_application: cfe_patch_required_exact_md_bool(
            properties,
            "ClientManagedApplication",
            module_path,
            path,
        )?,
        server: cfe_patch_required_exact_md_bool(properties, "Server", module_path, path)?,
        external_connection: cfe_patch_required_exact_md_bool(
            properties,
            "ExternalConnection",
            module_path,
            path,
        )?,
        client_ordinary_application: cfe_patch_required_exact_md_bool(
            properties,
            "ClientOrdinaryApplication",
            module_path,
            path,
        )?,
    };
    let privileged = cfe_patch_optional_exact_md_bool(properties, "Privileged", module_path, path)?
        .unwrap_or(false);
    if global && capabilities.server {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "global server CommonModule cannot be extended: {}",
                path.display()
            ),
        ));
    }
    if privileged && !capabilities.server {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "Privileged CommonModule must also have Server=true: {}",
                path.display()
            ),
        ));
    }
    if !capabilities.any_execution_context() {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "CommonModule has no enabled execution context: {}",
                path.display()
            ),
        ));
    }
    Ok(Some(capabilities))
}

pub fn cfe_patch_validate_registration(
    module_path: &str,
    cfg_file: &Path,
    cfg_preimage: &[u8],
    target: &CfePatchModuleTarget,
) -> Result<String, String> {
    let document = cfe_patch_supported_document(cfg_file, cfg_preimage)?;
    let configuration =
        cfe_patch_direct_md_child(&document, "Configuration", module_path, cfg_file)?;
    cfe_patch_validate_non_nil_uuid(
        module_path,
        configuration.attribute("uuid"),
        "Configuration uuid",
        cfg_file,
    )?;
    let properties = cfe_patch_exact_md_child(configuration, "Properties", module_path, cfg_file)?;
    let object_belonging = cfe_patch_exact_md_text(
        properties,
        "ObjectBelonging",
        module_path,
        cfg_file,
    )
    .map_err(|_| {
        cfe_patch_precondition_error(module_path, "Configuration ObjectBelonging must be Adopted")
    })?;
    if object_belonging != "Adopted" {
        return Err(cfe_patch_precondition_error(
            module_path,
            "Configuration ObjectBelonging must be Adopted",
        ));
    }
    let purpose = cfe_patch_exact_md_text(
        properties,
        "ConfigurationExtensionPurpose",
        module_path,
        cfg_file,
    )?;
    if !matches!(purpose.as_str(), "Patch" | "Customization" | "AddOn") {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "ConfigurationExtensionPurpose in {} must be one of Patch, Customization, AddOn; got {purpose:?}",
                cfg_file.display()
            ),
        ));
    }
    let name_prefix = cfe_patch_exact_md_text(properties, "NamePrefix", module_path, cfg_file)?;
    if name_prefix.is_empty() {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "NamePrefix in {} must be non-empty and configured by the extension",
                cfg_file.display()
            ),
        ));
    }
    let child_objects =
        cfe_patch_exact_md_child(configuration, "ChildObjects", module_path, cfg_file)?;
    let registrations = child_objects
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == target.type_name
                && node.text() == Some(target.object_name.as_str())
        })
        .collect::<Vec<_>>();
    let registered = registrations.len() == 1
        && registrations[0].tag_name().namespace() == Some(CFE_PATCH_MD_NAMESPACE);
    if !registered {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "{}.{} is not registered in extension Configuration.xml",
                target.type_name, target.object_name
            ),
        ));
    }
    Ok(name_prefix)
}

pub fn cfe_patch_validate_form_registration(
    module_path: &str,
    descriptor: &Path,
    descriptor_raw: &[u8],
    expected_type: &str,
    form_name: &str,
) -> Result<(), String> {
    let document = cfe_patch_supported_document(descriptor, descriptor_raw)?;
    let object = cfe_patch_direct_md_child(&document, expected_type, module_path, descriptor)?;
    let child_objects = cfe_patch_exact_md_child(object, "ChildObjects", module_path, descriptor)?;
    let forms = child_objects
        .children()
        .filter(|node| {
            node.is_element() && node.tag_name().name() == "Form" && node.text() == Some(form_name)
        })
        .collect::<Vec<_>>();
    if forms.len() != 1 || forms[0].tag_name().namespace() != Some(CFE_PATCH_MD_NAMESPACE) {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "Form.{form_name} is not registered in ChildObjects of {}",
                descriptor.display()
            ),
        ));
    }
    Ok(())
}

pub fn cfe_patch_validate_form_xml(
    module_path: &str,
    path: &Path,
    raw: &[u8],
) -> Result<(), String> {
    let document = cfe_patch_supported_document(path, raw)?;
    let root = document.root_element();
    if root.tag_name().name() != "Form"
        || root.tag_name().namespace() != Some(CFE_PATCH_FORM_NAMESPACE)
    {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "borrowed Form.xml {} must have root {{{CFE_PATCH_FORM_NAMESPACE}}}Form",
                path.display()
            ),
        ));
    }
    let base_forms = root
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "BaseForm")
        .collect::<Vec<_>>();
    if base_forms.len() != 1
        || base_forms[0].tag_name().namespace() != Some(CFE_PATCH_FORM_NAMESPACE)
        || base_forms[0].attribute("version") != Some(ACTIVE_FORMAT_PROFILE.export_format)
    {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "borrowed Form.xml {} must contain exactly one direct BaseForm version=\"{}\"",
                path.display(),
                ACTIVE_FORMAT_PROFILE.export_format
            ),
        ));
    }
    Ok(())
}

pub fn bsl_file_for_module_path(
    extension_path: &Path,
    module_path: &str,
) -> Result<PathBuf, String> {
    cfe_patch_module_target(extension_path, module_path).map(|target| target.bsl_file)
}

pub struct CfeValidationReporter {
    pub errors: usize,
    pub warnings: usize,
    pub ok_count: usize,
    pub stopped: bool,
    pub max_errors: usize,
    pub detailed: bool,
    pub lines: Vec<String>,
    pub obj_name: String,
}

pub struct CfeValidationRun {
    pub ok: bool,
    pub stdout: String,
    pub artifact: PathBuf,
    pub errors: Vec<String>,
}

pub struct CfeDiffObject {
    pub obj_type: String,
    pub name: String,
}

pub struct CfeDiffObjectInfo {
    pub borrowed: bool,
    pub exists: bool,
    pub dir_name: String,
    pub attrs: usize,
    pub forms: usize,
    pub tabular_sections: usize,
    pub borrowed_items: usize,
    pub form_names: Vec<String>,
}

pub struct CfeDiffInterceptor {
    pub interceptor_type: String,
    pub method: String,
    pub line: usize,
}

pub struct CfeDiffInsertionBlock {
    pub code: String,
}

/// Identity a borrowed descriptor is issued once and keeps: the wrapper uuid,
/// the exchange-plan node, and the `TypeId`/`ValueId` pair of every
/// `xr:GeneratedType`.
///
/// `cfe.borrow` rewrites the whole descriptor on every call, so without this a
/// second borrow of the same object minted all of it again and moved an
/// identity that the extension and its callers may already reference.
/// To preserve that identity, a value read from the existing
/// descriptor reuses what is there, and the default value — what a first borrow
/// gets — mints fresh.
#[derive(Clone, Debug, Default)]
pub struct CfeBorrowIdentity {
    plan_seed: Option<[u8; 32]>,
    wrapper_uuid: Option<String>,
    this_node: Option<String>,
    generated_types: BTreeMap<String, (Option<String>, Option<String>)>,
}

#[derive(Clone, Debug)]
pub struct CfeBorrowSourceAttribute {
    pub name: String,
    pub source_uuid: String,
    pub type_xml: String,
}

#[derive(Clone, Debug)]
pub struct CfeBorrowGeneratedType {
    pub name: String,
    pub category: String,
}

#[derive(Clone, Debug)]
pub struct CfeBorrowSourceTabularSection {
    pub name: String,
    pub source_uuid: String,
    pub generated_types: Vec<CfeBorrowGeneratedType>,
    pub attributes: Vec<CfeBorrowSourceAttribute>,
}

#[derive(Clone, Debug, Default)]
pub struct CfeBorrowResolvedAttributes {
    pub attributes: Vec<CfeBorrowSourceAttribute>,
    pub tabular_sections: Vec<CfeBorrowSourceTabularSection>,
}

#[derive(Clone, Debug)]
pub struct CfeBorrowDeepPath {
    pub segments: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct CfeBorrowFormPaths {
    pub first_level: HashSet<String>,
    pub deep_paths: Vec<CfeBorrowDeepPath>,
}

#[derive(Clone, Debug)]
pub struct CfeBorrowFormBlock {
    pub local_name: String,
    pub xml: String,
}

pub struct CfeBorrowedForm {
    pub raw_text: String,
    pub context: String,
}

#[derive(Debug, Clone)]
pub struct CfeInitPlannedXml {
    pub output_dir: PathBuf,
    pub configuration: PathBuf,
    pub language: PathBuf,
    pub role: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfePatchModuleTarget {
    pub type_name: &'static str,
    pub object_name: String,
    pub role: CfePatchModuleRole,
    pub form_name: Option<String>,
    pub descriptor: PathBuf,
    pub bsl_file: PathBuf,
}

impl CfeValidationReporter {
    pub fn new(max_errors: usize, detailed: bool) -> Self {
        Self {
            errors: 0,
            warnings: 0,
            ok_count: 0,
            stopped: false,
            max_errors,
            detailed,
            lines: Vec::new(),
            obj_name: "(unknown)".to_string(),
        }
    }

    pub fn out(&mut self, message: impl Into<String>) {
        self.lines.push(message.into());
    }

    pub fn ok(&mut self, message: impl Into<String>) {
        self.ok_count += 1;
        if self.detailed {
            self.lines.push(format!("[OK]    {}", message.into()));
        }
    }

    pub fn error(&mut self, message: impl Into<String>) {
        self.errors += 1;
        self.lines.push(format!("[ERROR] {}", message.into()));
        if self.errors >= self.max_errors {
            self.stopped = true;
        }
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings += 1;
        self.lines.push(format!("[WARN]  {}", message.into()));
    }

    pub fn finalize(mut self) -> (bool, String, Vec<String>) {
        let checks = self.ok_count + self.errors + self.warnings;
        let ok = self.errors == 0;
        if ok && self.warnings == 0 && !self.detailed {
            return (
                true,
                format!(
                    "=== Validation OK: Extension.{} ({checks} checks) ===\n",
                    self.obj_name
                ),
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

impl CfeBorrowIdentity {
    /// Canonical preview and apply regenerate the same identities from their
    /// input plan. Legacy callers without a seed continue issuing fresh IDs.
    pub fn for_plan(seed: [u8; 32]) -> Self {
        Self {
            plan_seed: Some(seed),
            ..Self::default()
        }
    }

    pub fn new_uuid(&self, role: &str, name: &str, fresh_uuid: fn() -> String) -> String {
        let Some(seed) = self.plan_seed else {
            return fresh_uuid();
        };
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"unica-canonical-borrow-uuid-v1\0");
        hash.update(seed);
        for value in [role, name] {
            hash.update((value.len() as u64).to_be_bytes());
            hash.update(value.as_bytes());
        }
        let digest = hash.finalize();
        let mut bytes = [0_u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        uuid::Builder::from_custom_bytes(bytes)
            .into_uuid()
            .to_string()
    }

    pub fn read(text: &str, child_name: &str) -> Result<Self, String> {
        let document = Document::parse(text.trim_start_matches('\u{feff}'))
            .map_err(|error| error.to_string())?;
        let Some(wrapper) = document
            .root_element()
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == child_name)
        else {
            return Ok(Self::default());
        };
        let mut identity = Self {
            wrapper_uuid: wrapper.attribute("uuid").map(ToOwned::to_owned),
            ..Self::default()
        };
        if let Some(internal_info) = unica_xml_format::read::child_element(wrapper, "InternalInfo")
        {
            identity.this_node =
                unica_xml_format::read::child_direct_text(internal_info, "ThisNode");
            for generated in unica_xml_format::read::child_elements(internal_info, "GeneratedType")
            {
                // Each identifier is preserved on its own: a descriptor that
                // lost one of them must keep the other, or reissuing both would
                // move an identity that is still referenced. Rejecting damaged
                // descriptors instead of repairing them remains a gap in #935/#931.
                let Some(name) = generated.attribute("name") else {
                    continue;
                };
                identity.generated_types.insert(
                    name.to_string(),
                    (
                        unica_xml_format::read::child_direct_text(generated, "TypeId"),
                        unica_xml_format::read::child_direct_text(generated, "ValueId"),
                    ),
                );
            }
        }
        Ok(identity)
    }

    pub fn wrapper_uuid(&self, fresh_uuid: fn() -> String) -> String {
        self.wrapper_uuid
            .clone()
            .unwrap_or_else(|| self.new_uuid("wrapper", "", fresh_uuid))
    }

    pub fn this_node(&self, fresh_uuid: fn() -> String) -> String {
        self.this_node
            .clone()
            .unwrap_or_else(|| self.new_uuid("this-node", "", fresh_uuid))
    }

    pub fn generated_type(&self, name: &str, fresh_uuid: fn() -> String) -> (String, String) {
        let (type_id, value_id) = self
            .generated_types
            .get(name)
            .cloned()
            .unwrap_or((None, None));
        (
            type_id.unwrap_or_else(|| self.new_uuid("type", name, fresh_uuid)),
            value_id.unwrap_or_else(|| self.new_uuid("value", name, fresh_uuid)),
        )
    }
}

pub fn cfe_borrow_existing_identity(
    write_plan: &mut dyn TrackedSourceProposal,
    path: &Path,
    child_name: &str,
) -> Result<CfeBorrowIdentity, String> {
    if !write_plan.exists(path) {
        return Ok(CfeBorrowIdentity::default());
    }
    let text = write_plan.read_utf8_sig(path)?;
    CfeBorrowIdentity::read(&text, child_name)
        .map_err(|error| format!("[ERROR] XML parse error in {}: {error}", path.display()))
}

pub fn validate_cfe_source(
    reader: &dyn ProjectDiscoveryRead,
    args: &Map<String, Value>,
    resolved_path: PathBuf,
    cwd: &Path,
    module_layouts: &mut CfeModuleLayouts<'_>,
) -> Result<CfeValidationRun, String> {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    let config_dir = resolved_path.parent().unwrap_or(cwd);
    let detailed = unica_operation_contract::input::bool_arg(args, &["detailed", "Detailed"]);
    let max_errors = unica_operation_contract::input::int_arg(args, &["maxErrors", "MaxErrors"])
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(30);

    let text = read_utf8_sig(reader, &resolved_path)?;
    let source = text.trim_start_matches('\u{feff}');
    let doc = match Document::parse(source) {
        Ok(doc) => doc,
        Err(err) => {
            let mut report = CfeValidationReporter::new(max_errors, detailed);
            report.lines.insert(
                0,
                "=== Validation: Extension (parse failed) ===".to_string(),
            );
            report.out("");
            report.error(format!("1. XML parse failed: {err}"));
            let (ok, stdout, errors) = report.finalize();
            return Ok(CfeValidationRun {
                ok,
                stdout,
                artifact: resolved_path,
                errors,
            });
        }
    };

    let root = doc.root_element();
    let mut report = CfeValidationReporter::new(max_errors, detailed);
    report.out("");

    let root_local = root.tag_name().name();
    let root_ns = root.tag_name().namespace().unwrap_or("");
    if root_local != "MetaDataObject" {
        report.error(format!(
            "1. Root element is '{root_local}', expected 'MetaDataObject'"
        ));
        let (ok, stdout, errors) = report.finalize();
        return Ok(CfeValidationRun {
            ok,
            stdout,
            artifact: resolved_path,
            errors,
        });
    }

    let mut check1_ok = true;
    if root_ns != MD_NS {
        report.error(format!(
            "1. Root namespace is '{root_ns}', expected '{MD_NS}'"
        ));
        check1_ok = false;
    }
    let version_literal = root_version_literal(source, root);
    match classify_root_version(version_literal.as_deref()) {
        Ok(FormatCompatibility::Supported { .. }) => report.ok("Export format: 2.20"),
        Ok(compatibility) => report.warn(
            unica_source_designer::format_profile::format_compatibility_warning(&compatibility),
        ),
        Err(error) => report.error(error.to_string()),
    }
    let version = version_literal.as_deref().unwrap_or("");

    let Some(cfg_node) = root.children().find(|child| {
        unica_xml_format::names::element_has_name(*child, "Configuration", Some(MD_NS))
    }) else {
        report.error("1. No <Configuration> element found inside MetaDataObject");
        let (ok, stdout, errors) = report.finalize();
        return Ok(CfeValidationRun {
            ok,
            stdout,
            artifact: resolved_path,
            errors,
        });
    };

    let cfg_uuid = cfg_node.attribute("uuid").unwrap_or("");
    if cfg_uuid.is_empty() {
        report.error("1. Missing uuid on <Configuration>");
        check1_ok = false;
    } else if !cf_validate_guid(cfg_uuid) {
        report.error(format!("1. Invalid uuid '{cfg_uuid}' on <Configuration>"));
        check1_ok = false;
    }

    let props_node = unica_xml_format::read::child_element(cfg_node, "Properties");
    let obj_name = props_node
        .and_then(|props| unica_xml_format::read::child_direct_text(props, "Name"))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "(unknown)".to_string());
    report.obj_name = obj_name.clone();
    report
        .lines
        .insert(0, format!("=== Validation: Extension.{obj_name} ==="));
    if check1_ok {
        report.ok(format!(
            "1. Root structure: MetaDataObject/Configuration, version {version}"
        ));
    }
    if report.stopped {
        return cfe_validation_finish(report, resolved_path);
    }

    cfe_validate_internal_info(&mut report, cfg_node);
    if report.stopped {
        return cfe_validation_finish(report, resolved_path);
    }
    let def_lang = cfe_validate_properties(&mut report, props_node, &obj_name);
    if report.stopped {
        return cfe_validation_finish(report, resolved_path);
    }
    cfe_validate_enum_properties(&mut report, props_node);
    if report.stopped {
        return cfe_validation_finish(report, resolved_path);
    }

    let child_obj_node = unica_xml_format::read::child_element(cfg_node, "ChildObjects");
    let child_index = cfe_validate_child_objects(&mut report, child_obj_node);
    if report.stopped {
        return cfe_validation_finish(report, resolved_path);
    }
    cfe_validate_default_language(&mut report, child_obj_node, &def_lang);
    if report.stopped {
        return cfe_validation_finish(report, resolved_path);
    }
    cfe_validate_language_files(reader, &mut report, child_obj_node, config_dir);
    if report.stopped {
        return cfe_validation_finish(report, resolved_path);
    }
    cfe_validate_object_dirs(reader, &mut report, child_obj_node, config_dir);
    if report.stopped {
        return cfe_validation_finish(report, resolved_path);
    }
    let borrowed_forms = cfe_validate_borrowed_objects(
        module_layouts,
        reader,
        &mut report,
        child_obj_node,
        config_dir,
        &child_index,
    );
    if report.stopped {
        return cfe_validation_finish(report, resolved_path);
    }
    cfe_validate_borrowed_forms(&mut report, &borrowed_forms);
    if report.stopped {
        return cfe_validation_finish(report, resolved_path);
    }
    cfe_validate_form_dependencies(&mut report, &borrowed_forms, &child_index);
    if report.stopped {
        return cfe_validation_finish(report, resolved_path);
    }
    cfe_validate_typelinks(&mut report, &borrowed_forms);
    if report.stopped {
        return cfe_validation_finish(report, resolved_path);
    }
    cfe_validate_service_objects(reader, &mut report, child_obj_node, config_dir);

    cfe_validation_finish(report, resolved_path)
}

pub struct CfeInitImageInput<'a> {
    pub args: &'a Map<String, Value>,
    pub name: &'a str,
    pub synonym: &'a str,
    pub name_prefix: &'a str,
    pub purpose: &'a str,
    pub compatibility: &'a str,
    pub interface_mode: &'a str,
    pub base_lang_uuid: &'a str,
    pub role_name: &'a str,
    pub no_role: bool,
}
pub struct CfeInitImages {
    pub configuration: String,
    pub language: String,
    pub role: Option<String>,
}
pub fn cfe_init_images(input: CfeInitImageInput<'_>) -> CfeInitImages {
    let CfeInitImageInput {
        args,
        name,
        synonym,
        name_prefix,
        purpose,
        compatibility,
        interface_mode,
        base_lang_uuid,
        role_name,
        no_role,
    } = input;
    let uuid_cfg = unica_xml_format::names::stable_uuid(20);
    let uuid_lang = unica_xml_format::names::stable_uuid(21);
    let uuid_role = unica_xml_format::names::stable_uuid(22);
    let contained_object_ids = (23..30)
        .map(unica_xml_format::names::stable_uuid)
        .collect::<Vec<_>>();
    let contained_objects = contained_objects_xml(&contained_object_ids);
    let format_version_xml = ACTIVE_FORMAT_PROFILE.export_format;
    let purpose_xml = unica_xml_format::text::escape_xml(purpose);
    let compatibility_xml = unica_xml_format::text::escape_xml(compatibility);
    let interface_mode_xml = unica_xml_format::text::escape_xml(interface_mode);
    let vendor_xml = unica_operation_contract::input::string_arg(args, &["vendor", "Vendor"])
        .map(unica_xml_format::text::escape_xml)
        .unwrap_or_default();
    let version_xml = unica_operation_contract::input::string_arg(args, &["version", "Version"])
        .map(unica_xml_format::text::escape_xml)
        .unwrap_or_default();
    let synonym_xml = format!(
            "\r\n\t\t\t\t<v8:item>\r\n\t\t\t\t\t<v8:lang>ru</v8:lang>\r\n\t\t\t\t\t<v8:content>{}</v8:content>\r\n\t\t\t\t</v8:item>\r\n\t\t\t",
            unica_xml_format::text::escape_xml(synonym)
        );
    let default_roles_xml = if no_role {
        String::new()
    } else {
        format!(
            "\r\n\t\t\t\t<xr:Item xsi:type=\"xr:MDObjectRef\">Role.{}</xr:Item>\r\n\t\t\t",
            unica_xml_format::text::escape_xml(role_name)
        )
    };
    let mut child_objects_xml = "\r\n\t\t\t<Language>Русский</Language>".to_string();
    if !no_role {
        child_objects_xml.push_str(&format!(
            "\r\n\t\t\t<Role>{}</Role>",
            unica_xml_format::text::escape_xml(role_name)
        ));
    }
    child_objects_xml.push_str("\r\n\t\t");

    let configuration_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xen="http://v8.1c.ru/8.3/xcf/enums" xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="{format_version_xml}">
	<Configuration uuid="{uuid_cfg}">
		<InternalInfo>
{contained_objects}		</InternalInfo>
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>{name}</Name>
			<Synonym>{synonym_xml}</Synonym>
			<Comment/>
			<ConfigurationExtensionPurpose>{purpose_xml}</ConfigurationExtensionPurpose>
			<KeepMappingToExtendedConfigurationObjectsByIDs>true</KeepMappingToExtendedConfigurationObjectsByIDs>
			<NamePrefix>{name_prefix}</NamePrefix>
			<ConfigurationExtensionCompatibilityMode>{compatibility_xml}</ConfigurationExtensionCompatibilityMode>
			<DefaultRunMode>ManagedApplication</DefaultRunMode>
			<UsePurposes>
				<v8:Value xsi:type="app:ApplicationUsePurpose">PlatformApplication</v8:Value>
			</UsePurposes>
			<ScriptVariant>Russian</ScriptVariant>
			<DefaultRoles>{default_roles_xml}</DefaultRoles>
			<Vendor>{vendor_xml}</Vendor>
			<Version>{version_xml}</Version>
			<DefaultLanguage>Language.Русский</DefaultLanguage>
			<BriefInformation/>
			<DetailedInformation/>
			<Copyright/>
			<VendorInformationAddress/>
			<ConfigurationInformationAddress/>
			<InterfaceCompatibilityMode>{interface_mode_xml}</InterfaceCompatibilityMode>
		</Properties>
		<ChildObjects>{child_objects_xml}</ChildObjects>
	</Configuration>
</MetaDataObject>"#,
        name = unica_xml_format::text::escape_xml(name),
        name_prefix = unica_xml_format::text::escape_xml(name_prefix),
    );
    let language_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xen="http://v8.1c.ru/8.3/xcf/enums" xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="{format_version_xml}">
	<Language uuid="{uuid_lang}">
		<InternalInfo/>
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>Русский</Name>
			<Comment/>
			<ExtendedConfigurationObject>{base_lang_uuid}</ExtendedConfigurationObject>
			<LanguageCode>ru</LanguageCode>
		</Properties>
	</Language>
</MetaDataObject>"#
    );
    let role_xml = if no_role {
        None
    } else {
        Some(format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xen="http://v8.1c.ru/8.3/xcf/enums" xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="{format_version_xml}">
	<Role uuid="{uuid_role}">
		<Properties>
			<Name>{role_name}</Name>
			<Synonym/>
			<Comment/>
		</Properties>
	</Role>
</MetaDataObject>"#,
            role_name = unica_xml_format::text::escape_xml(role_name),
        ))
    };
    CfeInitImages {
        configuration: configuration_xml,
        language: language_xml,
        role: role_xml,
    }
}

pub struct CfeBorrowImageInput<'a> {
    pub ext_text: String,
    pub cfg_dir: &'a Path,
    pub ext_dir: &'a Path,
    pub ext_path: &'a Path,
    pub object_spec: &'a str,
    pub borrow_main_attribute: Option<String>,
}
pub struct CfeBorrowImagePlan {
    pub name_prefix: String,
    pub log: CfeBorrowLog,
    pub borrowed: Vec<CfeBorrowedItem>,
    pub artifacts: Vec<PathBuf>,
}
pub fn cfe_borrow_plan_image(
    reader: &dyn ProjectDiscoveryRead,
    write_plan: &mut dyn TrackedSourceProposal,
    input: CfeBorrowImageInput<'_>,
    fresh_uuid: fn() -> String,
) -> Result<CfeBorrowImagePlan, String> {
    let CfeBorrowImageInput {
        mut ext_text,
        cfg_dir,
        ext_dir,
        ext_path,
        object_spec,
        borrow_main_attribute,
    } = input;
    let ext_doc =
        Document::parse(&ext_text).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
    let ext_cfg = ext_doc
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "Configuration")
        .ok_or_else(|| "No <Configuration> element found in extension".to_string())?;
    let props_el = unica_xml_format::read::child_element(ext_cfg, "Properties")
        .ok_or_else(|| "No <Properties> element found in extension".to_string())?;
    if unica_xml_format::read::child_element(ext_cfg, "ChildObjects").is_none() {
        return Err("No <ChildObjects> element found in extension".to_string());
    }
    let name_prefix =
        unica_xml_format::read::child_direct_text(props_el, "NamePrefix").unwrap_or_default();
    let format_version = ext_doc
        .root_element()
        .attribute("version")
        .unwrap_or(ACTIVE_FORMAT_PROFILE.export_format)
        .to_string();

    let items = object_spec
        .split(";;")
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if items.is_empty() {
        return Err("No objects specified in -Object".to_string());
    }
    if let Some(mode) = &borrow_main_attribute {
        if !matches!(mode.as_str(), "Form" | "All") {
            return Err("-BorrowMainAttribute accepts 'Form' or 'All' (default: Form)".to_string());
        }
        if !items.iter().any(|item| item.contains(".Form.")) {
            return Err(
                "-BorrowMainAttribute requires a form in -Object (e.g. 'Catalog.X.Form.Y')"
                    .to_string(),
            );
        }
    }

    let mut log = CfeBorrowLog::default();
    let mut borrowed = Vec::<CfeBorrowedItem>::new();
    let mut artifacts = Vec::<PathBuf>::new();
    for item in &items {
        let spec = cfe_borrow_parse_object_spec(item)?;
        if spec.form_name.is_some() {
            borrowed.push(CfeBorrowedItem {
                kind: spec.type_name.clone(),
                name: spec.object_name.clone(),
                form: spec.form_name.clone(),
            });
            if !write_plan.exists(&cfe_borrow_target_object(
                ext_dir,
                &spec.type_name,
                &spec.object_name,
            )) {
                log.auto_borrowed(format!("{}.{}", spec.type_name, spec.object_name));
                let object_artifact = cfe_borrow_object_shell(
                    reader,
                    cfg_dir,
                    ext_dir,
                    write_plan,
                    &spec.type_name,
                    &spec.object_name,
                    &format_version,
                    &mut ext_text,
                    &mut log,
                    fresh_uuid,
                )?;
                artifacts.push(object_artifact);
            }
            let form_artifacts = cfe_borrow_form_shell(
                CfeBorrowBindings {
                    reader,
                    cfg_dir,
                    ext_dir,
                },
                write_plan,
                &spec,
                &format_version,
                borrow_main_attribute.is_some(),
                &mut log,
                fresh_uuid,
            )?;
            cfe_borrow_register_form(
                ext_dir,
                write_plan,
                &spec.type_name,
                &spec.object_name,
                spec.form_name.as_deref().unwrap_or_default(),
                &mut log,
            )?;
            artifacts.extend(form_artifacts);
            artifacts.extend(cfe_borrow_main_attribute_artifacts(
                reader,
                cfg_dir,
                ext_dir,
                write_plan,
                &spec,
                borrow_main_attribute.as_deref(),
                &format_version,
                &mut ext_text,
                &mut log,
                fresh_uuid,
            )?);
        } else {
            borrowed.push(CfeBorrowedItem {
                kind: spec.type_name.clone(),
                name: spec.object_name.clone(),
                form: None,
            });
            let artifact = cfe_borrow_object_shell(
                reader,
                cfg_dir,
                ext_dir,
                write_plan,
                &spec.type_name,
                &spec.object_name,
                &format_version,
                &mut ext_text,
                &mut log,
                fresh_uuid,
            )?;
            artifacts.push(artifact);
        }
    }

    cfe_borrow_normalize_lxml_config_serialization(&mut ext_text);
    write_plan.write_utf8_bom(ext_path, &ext_text)?;

    Ok(CfeBorrowImagePlan {
        name_prefix,
        log,
        borrowed,
        artifacts,
    })
}

pub fn cfe_diff_source(
    reader: &dyn ProjectDiscoveryRead,
    ext_text: &str,
    ext_cfg: PathBuf,
    extension_path: &Path,
    config_path: &Path,
) -> Result<(CfeDiffData, PathBuf), String> {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
    let ext_doc = Document::parse(ext_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error in {}: {err}", ext_cfg.display()))?;
    let ext_root = ext_doc.root_element();
    let ext_cfg_node = ext_root.descendants().find(|node| {
        unica_xml_format::names::element_has_name(*node, "Configuration", Some(MD_NS))
    });
    let ext_props =
        ext_cfg_node.and_then(|node| unica_xml_format::read::child_element(node, "Properties"));
    let ext_name = ext_props
        .and_then(|props| unica_xml_format::read::child_direct_text(props, "Name"))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "?".to_string());
    let name_prefix = ext_props
        .and_then(|props| unica_xml_format::read::child_direct_text(props, "NamePrefix"))
        .unwrap_or_default();
    let purpose = ext_props
        .and_then(|props| {
            unica_xml_format::read::child_direct_text(props, "ConfigurationExtensionPurpose")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "?".to_string());
    // Mode A listed contents, Mode B checked transfer. Both are facts about
    // the same extension, so the typed answer carries them together.
    let mut objects = Vec::<CfeDiffObject>::new();
    if let Some(child_obj_node) =
        ext_cfg_node.and_then(|node| unica_xml_format::read::child_element(node, "ChildObjects"))
    {
        for child in child_obj_node.children().filter(|node| node.is_element()) {
            let obj_type = child.tag_name().name();
            if obj_type == "Language" {
                continue;
            }
            objects.push(CfeDiffObject {
                obj_type: obj_type.to_string(),
                name: child.text().unwrap_or("").to_string(),
            });
        }
    }
    let (objects_data, totals) = cfe_diff_objects_data(reader, &objects, extension_path);
    let (transfer, transfer_totals) =
        cfe_diff_transfer_data(reader, &objects, extension_path, config_path);
    Ok((
        CfeDiffData {
            name: ext_name,
            purpose,
            name_prefix,
            objects: objects_data,
            totals,
            transfer,
            transfer_totals,
        },
        ext_cfg,
    ))
}

pub fn cfe_borrow_validate_planned_xml<'a>(
    files: impl IntoIterator<Item = (&'a Path, &'a [u8])>,
) -> Result<(), String> {
    for (path, updated) in files {
        let is_xml = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"));
        if !is_xml {
            continue;
        }
        let text = std::str::from_utf8(updated)
            .map_err(|error| format!("{} is not valid UTF-8: {error}", path.display()))?
            .trim_start_matches('\u{feff}');
        Document::parse(text)
            .map_err(|error| format!("XML parse error in {}: {error}", path.display()))?;
    }
    Ok(())
}

pub struct CfeBaseDerivation {
    pub language_uuid: String,
    pub compatibility: String,
    pub interface_mode: String,
    pub derived: Vec<CfeInitDerivedProperty>,
    pub warnings: Vec<String>,
    pub language_preimage: Option<(PathBuf, Vec<u8>)>,
}
pub fn cfe_init_derive_base(
    reader: &dyn ProjectDiscoveryRead,
    config_path: &Path,
    cfg_dir: &Path,
    base_raw: &[u8],
    mut compatibility: String,
) -> Result<CfeBaseDerivation, String> {
    let mut base_lang_uuid = "00000000-0000-0000-0000-000000000000".to_string();
    let mut derived = Vec::new();
    let mut init_warnings = Vec::new();
    let mut base_language_preimage = None;
    let base_text = std::str::from_utf8(base_raw).map_err(|error| {
        format!(
            "failed to read base config {} as UTF-8: {error}",
            config_path.display()
        )
    })?;
    let base_document =
        Document::parse(base_text.trim_start_matches('\u{feff}')).map_err(|error| {
            format!(
                "failed to parse base config {}: {error}",
                config_path.display()
            )
        })?;
    let base_root = base_document.root_element();
    if base_root.tag_name().name() != "MetaDataObject" {
        return Err(format!(
            "base config root must be MetaDataObject, got {}",
            base_root.tag_name().name()
        ));
    }
    if base_root.tag_name().namespace() != Some("http://v8.1c.ru/8.3/MDClasses") {
        return Err("base config root must use the MDClasses namespace".to_string());
    }
    // The base config only gates whether this is emitted; the value
    // itself is this build's active profile, so `baseConfig` would be
    // a false provenance.
    derived.push(CfeInitDerivedProperty::defaulted(
        "mdClassesFormatVersion",
        ACTIVE_FORMAT_PROFILE.export_format,
    ));
    let base_lang_file = cfg_dir.join("Languages").join("Русский.xml");
    if path_exists(reader, &base_lang_file) {
        match read_bytes(reader, &base_lang_file) {
            Ok(raw) => {
                base_language_preimage = Some((base_lang_file.clone(), raw.clone()));
                match std::str::from_utf8(&raw)
                    .ok()
                    .and_then(|text| Document::parse(text.trim_start_matches('\u{feff}')).ok())
                {
                    Some(doc) => {
                        if let Some(uuid) = doc
                            .descendants()
                            .find(|node| node.is_element() && node.tag_name().name() == "Language")
                            .and_then(|node| node.attribute("uuid"))
                        {
                            if !unica_xml_format::names::is_valid_uuid(uuid)
                                || uuid == "00000000-0000-0000-0000-000000000000"
                            {
                                return Err(format!(
                                    "Base language {} has invalid UUID {uuid:?}",
                                    base_lang_file.display()
                                ));
                            }
                            base_lang_uuid = uuid.to_string();
                            derived.push(CfeInitDerivedProperty::from_base(
                                "baseLanguageUuid",
                                &base_lang_uuid,
                            ));
                        }
                    }
                    None => {
                        init_warnings
                            .push(format!("не удалось разобрать {}", base_lang_file.display()));
                    }
                }
            }
            Err(_) => {
                init_warnings.push(format!("не удалось прочитать {}", base_lang_file.display()));
            }
        }
    } else {
        init_warnings.push(format!(
            "язык базовой конфигурации не найден: {}",
            base_lang_file.display()
        ));
    }

    if let Some(value) = unica_xml_format::read::first_text(&base_document, "CompatibilityMode") {
        compatibility = value;
        derived.push(CfeInitDerivedProperty::from_base(
            "compatibilityMode",
            &compatibility,
        ));
    } else {
        derived.push(CfeInitDerivedProperty::defaulted(
            "compatibilityMode",
            &compatibility,
        ));
    }
    let interface_mode = if let Some(value) =
        unica_xml_format::read::first_text(&base_document, "InterfaceCompatibilityMode")
    {
        derived.push(CfeInitDerivedProperty::from_base(
            "interfaceCompatibilityMode",
            &value,
        ));
        value
    } else {
        let value = "TaxiEnableVersion8_2".to_string();
        derived.push(CfeInitDerivedProperty::defaulted(
            "interfaceCompatibilityMode",
            &value,
        ));
        value
    };

    Ok(CfeBaseDerivation {
        language_uuid: base_lang_uuid,
        compatibility,
        interface_mode,
        derived,
        warnings: init_warnings,
        language_preimage: base_language_preimage,
    })
}

pub fn cfe_patch_context_annotation(
    role: CfePatchModuleRole,
    context_name: Option<&str>,
    common_module_capabilities: Option<CfePatchCommonModuleCapabilities>,
    module_path: &str,
) -> Result<Option<&'static str>, String> {
    let context_annotation = match role {
        CfePatchModuleRole::CommonModule => {
            let capabilities = common_module_capabilities.ok_or_else(|| {
                cfe_patch_precondition_error(
                    module_path,
                    "CommonModule execution capabilities were not validated",
                )
            })?;
            match context_name {
                None => None,
                Some("НаСервере") if capabilities.server => Some("&НаСервере"),
                Some("НаСервере") => {
                    return Err(
                        "Context НаСервере requires Server=true in the borrowed CommonModule"
                            .to_string(),
                    );
                }
                Some("НаКлиенте") if capabilities.client() => Some("&НаКлиенте"),
                Some("НаКлиенте") => {
                    return Err(
                            "Context НаКлиенте requires ClientManagedApplication=true or ClientOrdinaryApplication=true in the borrowed CommonModule"
                                .to_string(),
                        );
                }
                Some("НаСервереБезКонтекста") => {
                    return Err(
                            "Context НаСервереБезКонтекста is not available in a CommonModule on platform 1C 8.3.27"
                                .to_string(),
                        );
                }
                Some(_) => unreachable!("Context was validated above"),
            }
        }
        CfePatchModuleRole::Form => match context_name.unwrap_or("НаСервере") {
            "НаСервере" => Some("&НаСервере"),
            "НаКлиенте" => Some("&НаКлиенте"),
            "НаСервереБезКонтекста" => Some("&НаСервереБезКонтекста"),
            _ => unreachable!("Context was validated above"),
        },
        CfePatchModuleRole::ObjectModule
        | CfePatchModuleRole::ManagerModule
        | CfePatchModuleRole::RecordSetModule
        | CfePatchModuleRole::ValueManagerModule => {
            if context_name.is_some() {
                return Err(format!(
                        "Context is not available for {} in platform 1C 8.3.27; omit Context so no compilation directive is emitted",
                        role.as_str()
                    ));
            }
            None
        }
    };

    Ok(context_annotation)
}

pub fn cfe_patch_interceptor_bsl(
    context_annotation: Option<&str>,
    decorator: &str,
    method_name: &str,
    proc_name: &str,
    interceptor_type: &str,
) -> String {
    let body_line = match interceptor_type {
        "Before" => "\t// TODO: код перед вызовом оригинального метода",
        "After" => "\t// TODO: код после вызова оригинального метода",
        _ => unreachable!("InterceptorType was validated above"),
    };
    let mut bsl_code = Vec::new();
    if let Some(context_annotation) = context_annotation {
        bsl_code.push(context_annotation.to_string());
    }
    bsl_code.extend([
        format!("{decorator}(\"{method_name}\")"),
        format!("Процедура {proc_name}()"),
        body_line.to_string(),
        "КонецПроцедуры".to_string(),
    ]);

    format!("{}\r\n", bsl_code.join("\r\n"))
}

#[cfg(test)]
#[path = "../tests/platform/extension_codec.rs"]
mod tests;
