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
use super::{
    cf::*, form::*, interface::*, meta::*, mxl::*, role::*, skd::*, subsystem::*, template::*,
};
pub(crate) struct CfeValidationReporter {
    pub(crate) errors: usize,
    pub(crate) warnings: usize,
    pub(crate) ok_count: usize,
    pub(crate) stopped: bool,
    pub(crate) max_errors: usize,
    pub(crate) detailed: bool,
    pub(crate) lines: Vec<String>,
    pub(crate) obj_name: String,
}

pub(crate) struct CfeValidationRun {
    pub(crate) ok: bool,
    pub(crate) stdout: String,
    pub(crate) out_file: Option<PathBuf>,
    pub(crate) artifact: PathBuf,
    pub(crate) errors: Vec<String>,
}

pub(crate) struct CfeDiffObject {
    pub(crate) obj_type: String,
    pub(crate) name: String,
}

pub(crate) struct CfeDiffObjectInfo {
    pub(crate) borrowed: bool,
    pub(crate) exists: bool,
    pub(crate) dir_name: String,
    pub(crate) attrs: usize,
    pub(crate) forms: usize,
    pub(crate) tabular_sections: usize,
    pub(crate) borrowed_items: usize,
    pub(crate) form_names: Vec<String>,
}

pub(crate) struct CfeDiffInterceptor {
    pub(crate) interceptor_type: String,
    pub(crate) method: String,
    pub(crate) line: usize,
}

pub(crate) struct CfeDiffInsertionBlock {
    pub(crate) code: String,
}

impl CfeValidationReporter {
    pub(crate) fn new(max_errors: usize, detailed: bool) -> Self {
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

    pub(crate) fn out(&mut self, message: impl Into<String>) {
        self.lines.push(message.into());
    }

    pub(crate) fn ok(&mut self, message: impl Into<String>) {
        self.ok_count += 1;
        if self.detailed {
            self.lines.push(format!("[OK]    {}", message.into()));
        }
    }

    pub(crate) fn error(&mut self, message: impl Into<String>) {
        self.errors += 1;
        self.lines.push(format!("[ERROR] {}", message.into()));
        if self.errors >= self.max_errors {
            self.stopped = true;
        }
    }

    pub(crate) fn warn(&mut self, message: impl Into<String>) {
        self.warnings += 1;
        self.lines.push(format!("[WARN]  {}", message.into()));
    }

    pub(crate) fn finalize(mut self) -> (bool, String, Vec<String>) {
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

pub(crate) fn borrow_cfe(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    let result = (|| -> Result<(String, Vec<PathBuf>), String> {
        let ext_path = cfe_borrow_resolve_path(
            args,
            context,
            &["extensionPath", "ExtensionPath"],
            "extension",
        )?;
        let cfg_path =
            cfe_borrow_resolve_path(args, context, &["configPath", "ConfigPath"], "config")?;
        let object_spec = required_string(args, &["object", "Object"], "Object")?;
        let borrow_main_attribute = cfe_borrow_main_attribute_mode(args)?;

        let ext_dir = ext_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| context.cwd.clone());
        let cfg_dir = cfg_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| context.cwd.clone());
        let mut ext_text = fs::read_to_string(&ext_path)
            .map_err(|err| format!("failed to read {}: {err}", ext_path.display()))?;
        if ext_text.starts_with('\u{feff}') {
            ext_text = ext_text.trim_start_matches('\u{feff}').to_string();
        }
        let ext_doc =
            Document::parse(&ext_text).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
        let ext_cfg = ext_doc
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == "Configuration")
            .ok_or_else(|| "No <Configuration> element found in extension".to_string())?;
        let props_el = meta_info_child(ext_cfg, "Properties")
            .ok_or_else(|| "No <Properties> element found in extension".to_string())?;
        if meta_info_child(ext_cfg, "ChildObjects").is_none() {
            return Err("No <ChildObjects> element found in extension".to_string());
        }
        let name_prefix = meta_info_child_text(props_el, "NamePrefix").unwrap_or_default();
        let format_version = ext_doc
            .root_element()
            .attribute("version")
            .unwrap_or("2.17")
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
                return Err(
                    "-BorrowMainAttribute accepts 'Form' or 'All' (default: Form)".to_string(),
                );
            }
            if !items.iter().any(|item| item.contains(".Form.")) {
                return Err(
                    "-BorrowMainAttribute requires a form in -Object (e.g. 'Catalog.X.Form.Y')"
                        .to_string(),
                );
            }
        }

        let mut stdout = format!("[INFO] Extension NamePrefix: {name_prefix}\n");
        let mut artifacts = Vec::<PathBuf>::new();
        let mut borrowed_count = 0usize;
        for item in &items {
            let spec = cfe_borrow_parse_object_spec(item)?;
            if spec.form_name.is_some() {
                stdout.push_str(&format!(
                    "[INFO] Borrowing form {}.{}.Form.{}...\n",
                    spec.type_name,
                    spec.object_name,
                    spec.form_name.as_deref().unwrap_or_default()
                ));
                if !cfe_borrow_target_object(&ext_dir, &spec.type_name, &spec.object_name).exists()
                {
                    stdout.push_str(&format!(
                        "[INFO]   Parent object {}.{} not yet borrowed - borrowing first...\n",
                        spec.type_name, spec.object_name
                    ));
                    let object_artifact = cfe_borrow_object_shell(
                        &cfg_dir,
                        &ext_dir,
                        &spec.type_name,
                        &spec.object_name,
                        &format_version,
                        &mut ext_text,
                        &mut stdout,
                    )?;
                    artifacts.push(object_artifact);
                }
                let form_artifacts = cfe_borrow_form_shell(
                    &cfg_dir,
                    &ext_dir,
                    &spec,
                    &format_version,
                    borrow_main_attribute.is_some(),
                    &mut stdout,
                )?;
                artifacts.extend(cfe_borrow_main_attribute_artifacts(
                    &cfg_dir,
                    &ext_dir,
                    &spec,
                    borrow_main_attribute.as_deref(),
                    &format_version,
                    &mut ext_text,
                    &mut stdout,
                )?);
                cfe_borrow_register_form(
                    &ext_dir,
                    &spec.type_name,
                    &spec.object_name,
                    spec.form_name.as_deref().unwrap_or_default(),
                    &mut stdout,
                )?;
                artifacts.extend(form_artifacts);
                borrowed_count += 1;
            } else {
                stdout.push_str(&format!(
                    "[INFO] Borrowing {}.{}...\n",
                    spec.type_name, spec.object_name
                ));
                let artifact = cfe_borrow_object_shell(
                    &cfg_dir,
                    &ext_dir,
                    &spec.type_name,
                    &spec.object_name,
                    &format_version,
                    &mut ext_text,
                    &mut stdout,
                )?;
                artifacts.push(artifact);
                borrowed_count += 1;
            }
        }

        cfe_borrow_normalize_lxml_config_serialization(&mut ext_text);
        write_utf8_bom(&ext_path, &ext_text)?;
        stdout.push_str(&format!("[INFO] Saved: {}\n\n", ext_path.display()));
        stdout.push_str("=== cfe-borrow summary ===\n");
        stdout.push_str(&format!("  Extension:  {}\n", ext_dir.display()));
        stdout.push_str(&format!("  Config:     {}\n", cfg_dir.display()));
        stdout.push_str(&format!("  Borrowed:   {borrowed_count} object(s)\n"));
        for artifact in &artifacts {
            stdout.push_str(&format!("    - {}\n", artifact.display()));
        }
        artifacts.push(ext_path);
        Ok((stdout, artifacts))
    })();

    match result {
        Ok((stdout, artifacts)) => AdapterOutcome {
            ok: true,
            summary: "unica.cfe.borrow completed with native extension borrower".to_string(),
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
            summary: "unica.cfe.borrow failed in native extension borrower".to_string(),
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

pub(crate) struct CfeBorrowSpec {
    pub(crate) type_name: String,
    pub(crate) object_name: String,
    pub(crate) form_name: Option<String>,
}

pub(crate) fn cfe_borrow_resolve_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    names: &[&str],
    kind: &str,
) -> Result<PathBuf, String> {
    let raw = required_path(
        args,
        names,
        if kind == "extension" {
            "ExtensionPath"
        } else {
            "ConfigPath"
        },
    )?;
    let mut path = absolutize(raw, &context.cwd);
    if path.is_dir() {
        let candidate = path.join("Configuration.xml");
        if candidate.is_file() {
            path = candidate;
        } else if kind == "extension" {
            return Err(format!(
                "No Configuration.xml in extension directory: {}",
                path.display()
            ));
        } else {
            return Err(format!(
                "No Configuration.xml in config directory: {}",
                path.display()
            ));
        }
    }
    if !path.is_file() {
        if kind == "extension" {
            return Err(format!("Extension file not found: {}", path.display()));
        }
        return Err(format!("Config file not found: {}", path.display()));
    }
    Ok(path)
}

pub(crate) fn cfe_borrow_main_attribute_mode(
    args: &Map<String, Value>,
) -> Result<Option<String>, String> {
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

pub(crate) fn cfe_borrow_parse_object_spec(value: &str) -> Result<CfeBorrowSpec, String> {
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
    let remainder = &value[dot_idx + 1..];
    let (object_name, form_name) = if let Some(form_idx) = remainder.find(".Form.") {
        (
            remainder[..form_idx].to_string(),
            Some(remainder[form_idx + 6..].to_string()),
        )
    } else {
        (remainder.to_string(), None)
    };
    Ok(CfeBorrowSpec {
        type_name,
        object_name,
        form_name,
    })
}

pub(crate) fn cfe_borrow_type_synonym(value: &str) -> Option<&'static str> {
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

pub(crate) fn cfe_borrow_type_dir(type_name: &str) -> Option<&'static str> {
    cf_validate_child_type_dir(type_name)
}

pub(crate) fn cfe_borrow_target_object(
    ext_dir: &Path,
    type_name: &str,
    object_name: &str,
) -> PathBuf {
    let dir_name = cfe_borrow_type_dir(type_name).unwrap_or(type_name);
    ext_dir.join(dir_name).join(format!("{object_name}.xml"))
}

pub(crate) fn cfe_borrow_object_shell(
    cfg_dir: &Path,
    ext_dir: &Path,
    type_name: &str,
    object_name: &str,
    format_version: &str,
    ext_text: &mut String,
    stdout: &mut String,
) -> Result<PathBuf, String> {
    let dir_name =
        cfe_borrow_type_dir(type_name).ok_or_else(|| format!("Unknown type '{type_name}'"))?;
    let source_file = cfg_dir.join(dir_name).join(format!("{object_name}.xml"));
    if !source_file.is_file() {
        return Err(format!(
            "Source object not found: {}",
            source_file.display()
        ));
    }
    let source_text = fs::read_to_string(&source_file)
        .map_err(|err| format!("failed to read {}: {err}", source_file.display()))?;
    let source_text = source_text.trim_start_matches('\u{feff}');
    let source_doc =
        Document::parse(source_text).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
    let source_el = source_doc
        .root_element()
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| format!("No metadata element found in {dir_name}/{object_name}.xml"))?;
    let source_uuid = source_el.attribute("uuid").unwrap_or("");
    if source_uuid.is_empty() {
        return Err(format!(
            "No uuid attribute on source element in {dir_name}/{object_name}.xml"
        ));
    }
    stdout.push_str(&format!("[INFO]   Source UUID: {source_uuid}\n"));
    let target_file = cfe_borrow_target_object(ext_dir, type_name, object_name);
    if let Some(parent) = target_file.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let source_props = meta_info_child(source_el, "Properties");
    let xml = cfe_borrow_object_xml(
        type_name,
        object_name,
        source_uuid,
        source_props,
        format_version,
    );
    write_utf8_bom(&target_file, &xml)?;
    stdout.push_str(&format!("[INFO]   Created: {}\n", target_file.display()));
    cfe_borrow_add_to_child_objects(ext_text, type_name, object_name, stdout)?;
    Ok(target_file)
}

pub(crate) fn cfe_borrow_object_xml(
    type_name: &str,
    object_name: &str,
    source_uuid: &str,
    source_props: Option<roxmltree::Node<'_, '_>>,
    format_version: &str,
) -> String {
    let mut lines = Vec::<String>::new();
    lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
    lines.push(format!(
        "<MetaDataObject {} version=\"{}\">",
        cfe_borrow_xmlns_decl(),
        escape_xml(format_version)
    ));
    lines.push(format!("\t<{type_name} uuid=\"{}\">", fresh_uuid()));
    lines.push(cfe_borrow_internal_info_xml(type_name, object_name, "\t\t"));
    lines.push("\t\t<Properties>".to_string());
    lines.push("\t\t\t<ObjectBelonging>Adopted</ObjectBelonging>".to_string());
    lines.push(format!("\t\t\t<Name>{}</Name>", escape_xml(object_name)));
    lines.push("\t\t\t<Comment/>".to_string());
    lines.push(format!(
        "\t\t\t<ExtendedConfigurationObject>{}</ExtendedConfigurationObject>",
        escape_xml(source_uuid)
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
                .and_then(|props| meta_info_child_text(props, prop_name))
                .unwrap_or_else(|| "false".to_string());
            lines.push(format!(
                "\t\t\t<{prop_name}>{}</{prop_name}>",
                escape_xml(&value)
            ));
        }
    }
    if type_name == "DefinedType" {
        if let Some(type_xml) = source_props
            .and_then(|props| meta_info_child(props, "Type"))
            .map(cfe_borrow_xml_node)
        {
            lines.push(format!("\t\t\t{type_xml}"));
        }
    }
    lines.push("\t\t</Properties>".to_string());
    if cfe_borrow_type_has_child_objects(type_name) {
        lines.push("\t\t<ChildObjects/>".to_string());
    }
    lines.push(format!("\t</{type_name}>"));
    lines.push("</MetaDataObject>".to_string());
    lines.join("\n")
}

pub(crate) fn cfe_borrow_internal_info_xml(
    type_name: &str,
    object_name: &str,
    indent: &str,
) -> String {
    let Some(types) = cfe_borrow_generated_types(type_name) else {
        return format!("{indent}<InternalInfo/>");
    };
    let mut lines = vec![format!("{indent}<InternalInfo>")];
    for (prefix, category) in types {
        lines.push(format!(
            "{indent}\t<xr:GeneratedType name=\"{}.{}\" category=\"{}\">",
            prefix,
            escape_xml(object_name),
            category
        ));
        lines.push(format!(
            "{indent}\t\t<xr:TypeId>{}</xr:TypeId>",
            fresh_uuid()
        ));
        lines.push(format!(
            "{indent}\t\t<xr:ValueId>{}</xr:ValueId>",
            fresh_uuid()
        ));
        lines.push(format!("{indent}\t</xr:GeneratedType>"));
    }
    lines.push(format!("{indent}</InternalInfo>"));
    lines.join("\n")
}

pub(crate) fn cfe_borrow_generated_types(
    type_name: &str,
) -> Option<&'static [(&'static str, &'static str)]> {
    match type_name {
        "Catalog" => Some(&[
            ("CatalogObject", "Object"),
            ("CatalogRef", "Ref"),
            ("CatalogSelection", "Selection"),
            ("CatalogList", "List"),
            ("CatalogManager", "Manager"),
        ]),
        "Document" => Some(&[
            ("DocumentObject", "Object"),
            ("DocumentRef", "Ref"),
            ("DocumentSelection", "Selection"),
            ("DocumentList", "List"),
            ("DocumentManager", "Manager"),
        ]),
        "Enum" => Some(&[
            ("EnumRef", "Ref"),
            ("EnumManager", "Manager"),
            ("EnumList", "List"),
        ]),
        "Report" => Some(&[("ReportObject", "Object"), ("ReportManager", "Manager")]),
        "DataProcessor" => Some(&[
            ("DataProcessorObject", "Object"),
            ("DataProcessorManager", "Manager"),
        ]),
        "DefinedType" => Some(&[("DefinedType", "DefinedType")]),
        _ => None,
    }
}

pub(crate) fn cfe_borrow_type_has_child_objects(type_name: &str) -> bool {
    matches!(
        type_name,
        "Catalog"
            | "Document"
            | "ExchangePlan"
            | "ChartOfAccounts"
            | "ChartOfCharacteristicTypes"
            | "ChartOfCalculationTypes"
            | "BusinessProcess"
            | "Task"
            | "Enum"
            | "InformationRegister"
            | "AccumulationRegister"
            | "AccountingRegister"
            | "CalculationRegister"
    )
}

#[derive(Clone, Debug)]
pub(crate) struct CfeBorrowSourceAttribute {
    name: String,
    source_uuid: String,
    type_xml: String,
}

#[derive(Clone, Debug)]
pub(crate) struct CfeBorrowGeneratedType {
    name: String,
    category: String,
}

#[derive(Clone, Debug)]
pub(crate) struct CfeBorrowSourceTabularSection {
    name: String,
    source_uuid: String,
    generated_types: Vec<CfeBorrowGeneratedType>,
    attributes: Vec<CfeBorrowSourceAttribute>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CfeBorrowResolvedAttributes {
    attributes: Vec<CfeBorrowSourceAttribute>,
    tabular_sections: Vec<CfeBorrowSourceTabularSection>,
}

#[derive(Clone, Debug)]
pub(crate) struct CfeBorrowDeepPath {
    segments: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CfeBorrowFormPaths {
    first_level: HashSet<String>,
    deep_paths: Vec<CfeBorrowDeepPath>,
}

pub(crate) fn cfe_borrow_main_attribute_artifacts(
    cfg_dir: &Path,
    ext_dir: &Path,
    spec: &CfeBorrowSpec,
    mode: Option<&str>,
    format_version: &str,
    ext_text: &mut String,
    stdout: &mut String,
) -> Result<Vec<PathBuf>, String> {
    let Some(mode) = mode else {
        return Ok(Vec::new());
    };
    let type_name = spec.type_name.as_str();
    let object_name = spec.object_name.as_str();
    let form_name = spec.form_name.as_deref().unwrap_or_default();
    let dir_name =
        cfe_borrow_type_dir(type_name).ok_or_else(|| format!("Unknown type '{type_name}'"))?;
    stdout.push_str(&format!(
        "[INFO] Borrowing main attribute for {type_name}.{object_name} (mode: {mode})...\n"
    ));

    let form_paths = if mode == "Form" {
        let form_xml_path = cfg_dir
            .join(dir_name)
            .join(object_name)
            .join("Forms")
            .join(form_name)
            .join("Ext")
            .join("Form.xml");
        let paths = cfe_borrow_collect_form_object_paths(&form_xml_path)?;
        stdout.push_str(&format!(
            "[INFO]   Collected {} first-level DataPath references, {} deep paths\n",
            paths.first_level.len(),
            paths.deep_paths.len()
        ));
        if paths.first_level.is_empty() && paths.deep_paths.is_empty() {
            stdout.push_str("[INFO]   No main-attribute object paths found in form\n");
            return Ok(Vec::new());
        }
        Some(paths)
    } else {
        stdout.push_str("[INFO]   Mode All: borrowing all attributes and tabular sections\n");
        None
    };

    let wanted = form_paths.as_ref().map(|paths| &paths.first_level);
    let resolved = cfe_borrow_resolve_source_attributes(cfg_dir, type_name, object_name, wanted)?;
    stdout.push_str(&format!(
        "[INFO]   Resolved: {} attributes, {} tabular section(s)\n",
        resolved.attributes.len(),
        resolved.tabular_sections.len()
    ));

    let object_file = cfe_borrow_target_object(ext_dir, type_name, object_name);
    cfe_borrow_merge_resolved_into_object(&object_file, &resolved)?;
    let mut artifacts = vec![object_file];

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
        cfg_dir,
        ext_dir,
        &type_xmls,
        format_version,
        ext_text,
        stdout,
    )?);

    if let Some(paths) = &form_paths {
        artifacts.extend(cfe_borrow_process_deep_paths(
            cfg_dir,
            ext_dir,
            &resolved,
            &paths.deep_paths,
            format_version,
            ext_text,
            stdout,
        )?);
    }

    stdout.push_str("[INFO]   Main attribute borrowing complete\n");
    Ok(artifacts)
}

pub(crate) fn cfe_borrow_collect_form_object_paths(
    form_xml_path: &Path,
) -> Result<CfeBorrowFormPaths, String> {
    let source_text = fs::read_to_string(form_xml_path)
        .map_err(|err| format!("failed to read {}: {err}", form_xml_path.display()))?;
    let source = source_text.trim_start_matches('\u{feff}');
    let doc = Document::parse(source).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
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

pub(crate) fn cfe_borrow_object_path_segments(text: &str) -> Vec<Vec<String>> {
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

pub(crate) fn cfe_borrow_is_standard_field(name: &str) -> bool {
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

pub(crate) fn cfe_borrow_resolve_source_attributes(
    cfg_dir: &Path,
    type_name: &str,
    object_name: &str,
    first_level_names: Option<&HashSet<String>>,
) -> Result<CfeBorrowResolvedAttributes, String> {
    let dir_name =
        cfe_borrow_type_dir(type_name).ok_or_else(|| format!("Unknown type '{type_name}'"))?;
    let source_file = cfg_dir.join(dir_name).join(format!("{object_name}.xml"));
    let source_text = fs::read_to_string(&source_file)
        .map_err(|err| format!("failed to read {}: {err}", source_file.display()))?;
    let source = source_text.trim_start_matches('\u{feff}');
    let doc = Document::parse(source).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
    let source_el = doc
        .root_element()
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| format!("No metadata element found in {dir_name}/{object_name}.xml"))?;
    let Some(child_objects) = meta_info_child(source_el, "ChildObjects") else {
        return Ok(CfeBorrowResolvedAttributes::default());
    };
    let mut resolved = CfeBorrowResolvedAttributes::default();
    for child in child_objects.children().filter(|node| node.is_element()) {
        let local = child.tag_name().name();
        let props = meta_info_child(child, "Properties");
        let name = props
            .and_then(|props| meta_info_child_text(props, "Name"))
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

pub(crate) fn cfe_borrow_source_attribute(
    node: roxmltree::Node<'_, '_>,
) -> CfeBorrowSourceAttribute {
    let props = meta_info_child(node, "Properties");
    CfeBorrowSourceAttribute {
        name: props
            .and_then(|props| meta_info_child_text(props, "Name"))
            .unwrap_or_default(),
        source_uuid: node.attribute("uuid").unwrap_or("").to_string(),
        type_xml: props
            .and_then(|props| meta_info_child(props, "Type"))
            .map(cfe_borrow_xml_node)
            .unwrap_or_default(),
    }
}

pub(crate) fn cfe_borrow_source_tabular_section(
    node: roxmltree::Node<'_, '_>,
) -> CfeBorrowSourceTabularSection {
    let props = meta_info_child(node, "Properties");
    let mut generated_types = Vec::new();
    if let Some(internal_info) = meta_info_child(node, "InternalInfo") {
        for generated in meta_info_children(internal_info, "GeneratedType") {
            generated_types.push(CfeBorrowGeneratedType {
                name: generated.attribute("name").unwrap_or("").to_string(),
                category: generated.attribute("category").unwrap_or("").to_string(),
            });
        }
    }
    let mut attributes = Vec::new();
    if let Some(child_objects) = meta_info_child(node, "ChildObjects") {
        for attr in meta_info_children(child_objects, "Attribute") {
            attributes.push(cfe_borrow_source_attribute(attr));
        }
    }
    CfeBorrowSourceTabularSection {
        name: props
            .and_then(|props| meta_info_child_text(props, "Name"))
            .unwrap_or_default(),
        source_uuid: node.attribute("uuid").unwrap_or("").to_string(),
        generated_types,
        attributes,
    }
}

pub(crate) fn cfe_borrow_merge_resolved_into_object(
    object_file: &Path,
    resolved: &CfeBorrowResolvedAttributes,
) -> Result<(), String> {
    let mut object_text = fs::read_to_string(object_file)
        .map_err(|err| format!("failed to read {}: {err}", object_file.display()))?;
    let existing_names = cfe_borrow_existing_names(&object_text);
    let mut child_xml = Vec::<String>::new();
    for attr in &resolved.attributes {
        if !existing_names.contains(&attr.name) {
            child_xml.push(cfe_borrow_adopted_attribute_xml(attr, "\t\t\t"));
        }
    }
    for section in &resolved.tabular_sections {
        if !existing_names.contains(&section.name) {
            child_xml.push(cfe_borrow_adopted_tabular_section_xml(section, "\t\t\t"));
        }
    }
    if child_xml.is_empty() {
        return Ok(());
    }
    cfe_borrow_insert_child_objects(&mut object_text, &child_xml.join("\n"))?;
    write_utf8_bom(object_file, &object_text)
}

pub(crate) fn cfe_borrow_existing_names(object_text: &str) -> HashSet<String> {
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

pub(crate) fn cfe_borrow_insert_child_objects(
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

pub(crate) fn cfe_borrow_adopted_attribute_xml(
    attr: &CfeBorrowSourceAttribute,
    indent: &str,
) -> String {
    let mut lines = vec![
        format!("{indent}<Attribute uuid=\"{}\">", fresh_uuid()),
        format!("{indent}\t<InternalInfo/>"),
        format!("{indent}\t<Properties>"),
        format!("{indent}\t\t<ObjectBelonging>Adopted</ObjectBelonging>"),
        format!("{indent}\t\t<Name>{}</Name>", escape_xml(&attr.name)),
        format!("{indent}\t\t<Comment/>"),
        format!(
            "{indent}\t\t<ExtendedConfigurationObject>{}</ExtendedConfigurationObject>",
            escape_xml(&attr.source_uuid)
        ),
    ];
    if !attr.type_xml.is_empty() {
        lines.push(format!("{indent}\t\t{}", attr.type_xml));
    }
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</Attribute>"));
    lines.join("\n")
}

pub(crate) fn cfe_borrow_adopted_tabular_section_xml(
    section: &CfeBorrowSourceTabularSection,
    indent: &str,
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
                escape_xml(&generated.name),
                escape_xml(&generated.category)
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
        escape_xml(&section.name)
    ));
    lines.push(format!("{indent}\t\t<Comment/>"));
    lines.push(format!(
        "{indent}\t\t<ExtendedConfigurationObject>{}</ExtendedConfigurationObject>",
        escape_xml(&section.source_uuid)
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
            ));
        }
        lines.push(format!("{indent}\t</ChildObjects>"));
    }
    lines.push(format!("{indent}</TabularSection>"));
    lines.join("\n")
}

pub(crate) fn cfe_borrow_ensure_reference_shells(
    cfg_dir: &Path,
    ext_dir: &Path,
    type_xmls: &[String],
    format_version: &str,
    ext_text: &mut String,
    stdout: &mut String,
) -> Result<Vec<PathBuf>, String> {
    let mut artifacts = Vec::new();
    let mut seen = HashSet::<String>::new();
    for (type_name, object_name) in cfe_borrow_collect_reference_types(type_xmls) {
        let key = format!("{type_name}.{object_name}");
        if !seen.insert(key) {
            continue;
        }
        if cfe_borrow_target_object(ext_dir, &type_name, &object_name).exists() {
            continue;
        }
        let source_file = cfg_dir
            .join(cfe_borrow_type_dir(&type_name).unwrap_or(&type_name))
            .join(format!("{object_name}.xml"));
        if !source_file.exists() {
            stdout.push_str(&format!(
                "[WARN]   Source not found: {type_name}.{object_name}\n"
            ));
            continue;
        }
        let artifact = cfe_borrow_object_shell(
            cfg_dir,
            ext_dir,
            &type_name,
            &object_name,
            format_version,
            ext_text,
            stdout,
        )?;
        stdout.push_str(&format!(
            "[INFO]   Auto-borrowed: {type_name}.{object_name}\n"
        ));
        artifacts.push(artifact);
    }
    Ok(artifacts)
}

pub(crate) fn cfe_borrow_collect_reference_types(type_xmls: &[String]) -> Vec<(String, String)> {
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

pub(crate) fn cfe_borrow_process_deep_paths(
    cfg_dir: &Path,
    ext_dir: &Path,
    resolved: &CfeBorrowResolvedAttributes,
    deep_paths: &[CfeBorrowDeepPath],
    format_version: &str,
    ext_text: &mut String,
    stdout: &mut String,
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
        if !target_path.exists() {
            let artifact = cfe_borrow_object_shell(
                cfg_dir,
                ext_dir,
                &target_type,
                &target_object,
                format_version,
                ext_text,
                stdout,
            )?;
            stdout.push_str(&format!(
                "[INFO]   Auto-borrowed for deep path: {target_type}.{target_object}\n"
            ));
            artifacts.push(artifact);
        }
        let mut wanted = HashSet::new();
        wanted.insert(sub_attr_name);
        let sub_resolved = cfe_borrow_resolve_source_attributes(
            cfg_dir,
            &target_type,
            &target_object,
            Some(&wanted),
        )?;
        if !sub_resolved.attributes.is_empty() || !sub_resolved.tabular_sections.is_empty() {
            cfe_borrow_merge_resolved_into_object(&target_path, &sub_resolved)?;
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
                cfg_dir,
                ext_dir,
                &sub_type_xmls,
                format_version,
                ext_text,
                stdout,
            )?);
        }
    }
    Ok(artifacts)
}

pub(crate) fn cfe_borrow_reference_target_from_type_xml(
    type_xml: &str,
) -> Option<(String, String)> {
    cfe_borrow_collect_reference_types(&[type_xml.to_string()])
        .into_iter()
        .next()
}

pub(crate) fn cfe_borrow_add_to_child_objects(
    ext_text: &mut String,
    type_name: &str,
    object_name: &str,
    stdout: &mut String,
) -> Result<(), String> {
    let mut children = cf_edit_child_objects(ext_text)?;
    if children
        .iter()
        .any(|(child_type, child_name)| child_type == type_name && child_name == object_name)
    {
        stdout.push_str(&format!(
            "[WARN] Already in ChildObjects: {type_name}.{object_name}\n"
        ));
        return Ok(());
    }
    children.push((type_name.to_string(), object_name.to_string()));
    children.sort_by(cf_edit_child_object_cmp);
    *ext_text = cf_edit_replace_child_objects(ext_text, &children)?;
    stdout.push_str(&format!(
        "[INFO] Added to ChildObjects: {type_name}.{object_name}\n"
    ));
    Ok(())
}

pub(crate) fn cfe_borrow_normalize_lxml_config_serialization(ext_text: &mut String) {
    if ext_text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>") {
        *ext_text = ext_text.replacen(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>",
            1,
        );
    }
    *ext_text = ext_text.replace("<DefaultRoles></DefaultRoles>", "<DefaultRoles/>");
    *ext_text = ext_text.replace("</Language>\r\n", "</Language>&#13;\n");
    if !ext_text.ends_with('\n') {
        ext_text.push('\n');
    }
}

pub(crate) fn cfe_borrow_form_shell(
    cfg_dir: &Path,
    ext_dir: &Path,
    spec: &CfeBorrowSpec,
    format_version: &str,
    borrow_main_attr: bool,
    stdout: &mut String,
) -> Result<Vec<PathBuf>, String> {
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
    if !form_meta_source.is_file() {
        return Err(format!(
            "Source form not found: {}",
            form_meta_source.display()
        ));
    }
    let source_text = fs::read_to_string(&form_meta_source)
        .map_err(|err| format!("failed to read {}: {err}", form_meta_source.display()))?;
    let source_doc = Document::parse(source_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
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
    stdout.push_str(&format!("[INFO]   Source form UUID: {source_uuid}\n"));
    let source_form_xml = cfg_dir
        .join(dir_name)
        .join(object_name)
        .join("Forms")
        .join(form_name)
        .join("Ext")
        .join("Form.xml");
    if !source_form_xml.is_file() {
        return Err(format!(
            "Source Form.xml not found: {}",
            source_form_xml.display()
        ));
    }
    let form_meta_dir = ext_dir.join(dir_name).join(object_name).join("Forms");
    fs::create_dir_all(&form_meta_dir)
        .map_err(|err| format!("failed to create {}: {err}", form_meta_dir.display()))?;
    let form_meta_target = form_meta_dir.join(format!("{form_name}.xml"));
    let form_wrapper_uuid =
        cfe_borrow_existing_metadata_uuid(&form_meta_target, "Form").unwrap_or_else(fresh_uuid);
    write_utf8_bom(
        &form_meta_target,
        &cfe_borrow_form_metadata_xml(form_name, source_uuid, &form_wrapper_uuid, format_version),
    )?;
    stdout.push_str(&format!(
        "[INFO]   Created: {}\n",
        form_meta_target.display()
    ));

    let form_xml_target = form_meta_dir.join(form_name).join("Ext").join("Form.xml");
    if let Some(parent) = form_xml_target.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let source_form_content = fs::read_to_string(&source_form_xml)
        .map_err(|err| format!("failed to read {}: {err}", source_form_xml.display()))?;
    let borrowed_form_xml = cfe_borrow_form_xml(
        &source_form_content,
        type_name,
        object_name,
        borrow_main_attr,
    );
    write_utf8_bom(&form_xml_target, &borrowed_form_xml)?;
    stdout.push_str(&format!(
        "[INFO]   Created: {}\n",
        form_xml_target.display()
    ));

    let module_file = form_meta_dir
        .join(form_name)
        .join("Ext")
        .join("Form")
        .join("Module.bsl");
    if let Some(parent) = module_file.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let mut artifacts = vec![form_meta_target, form_xml_target];
    if module_file.exists() {
        stdout.push_str(&format!(
            "[SKIP] Module.bsl already exists: {} - not overwriting\n",
            module_file.display()
        ));
    } else {
        write_utf8_bom(&module_file, "")?;
        stdout.push_str(&format!("[INFO]   Created: {}\n", module_file.display()));
        artifacts.push(module_file);
    }
    Ok(artifacts)
}

pub(crate) fn cfe_borrow_form_metadata_xml(
    form_name: &str,
    source_uuid: &str,
    wrapper_uuid: &str,
    format_version: &str,
) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MetaDataObject {} version=\"{}\">\n\t<Form uuid=\"{}\">\n\t\t<InternalInfo/>\n\t\t<Properties>\n\t\t\t<ObjectBelonging>Adopted</ObjectBelonging>\n\t\t\t<Name>{}</Name>\n\t\t\t<Comment/>\n\t\t\t<ExtendedConfigurationObject>{}</ExtendedConfigurationObject>\n\t\t\t<FormType>Managed</FormType>\n\t\t</Properties>\n\t</Form>\n</MetaDataObject>\n",
        cfe_borrow_xmlns_decl(),
        escape_xml(format_version),
        escape_xml(wrapper_uuid),
        escape_xml(form_name),
        escape_xml(source_uuid)
    )
}

pub(crate) fn cfe_borrow_form_xml(
    source_form_content: &str,
    type_name: &str,
    object_name: &str,
    borrow_main_attr: bool,
) -> String {
    let source = source_form_content.trim_start_matches('\u{feff}');
    let version = Document::parse(source)
        .ok()
        .and_then(|doc| {
            doc.root_element()
                .attribute("version")
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "2.17".to_string());
    let mut content = source.to_string();
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
            escape_xml(object_type_prefix),
            escape_xml(object_name)
        )
    } else {
        "<Attributes/>".to_string()
    };
    if content.contains("</Form>") && !content.contains("<BaseForm") {
        content = content.replacen(
            "</Form>",
            &format!(
                "\t<BaseForm version=\"{}\">\n\t\t{}\n\t</BaseForm>\n</Form>",
                escape_xml(&version),
                main_attr
            ),
            1,
        );
    }
    if borrow_main_attr && content.contains("<Attributes/>") {
        content = content.replacen("<Attributes/>", &main_attr, 1);
    }
    ensure_trailing_lf(&content)
}

pub(crate) fn cfe_borrow_strip_simple_data_paths(value: &str) -> String {
    let mut text = cfe_borrow_remove_simple_element(value, "DataPath");
    text = cfe_borrow_remove_simple_element(&text, "TitleDataPath");
    cfe_borrow_remove_simple_element(&text, "RowPictureDataPath")
}

pub(crate) fn cfe_borrow_remove_simple_element(value: &str, tag: &str) -> String {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut result = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find(&open) {
        let Some(end_rel) = rest[start + open.len()..].find(&close) else {
            break;
        };
        result.push_str(&rest[..start]);
        rest = &rest[start + open.len() + end_rel + close.len()..];
    }
    result.push_str(rest);
    result
}

pub(crate) fn cfe_borrow_register_form(
    ext_dir: &Path,
    type_name: &str,
    object_name: &str,
    form_name: &str,
    stdout: &mut String,
) -> Result<(), String> {
    let object_file = cfe_borrow_target_object(ext_dir, type_name, object_name);
    if !object_file.is_file() {
        stdout.push_str(&format!(
            "[WARN] Parent object file not found: {} - form not registered in ChildObjects\n",
            object_file.display()
        ));
        return Ok(());
    }
    let mut text = fs::read_to_string(&object_file)
        .map_err(|err| format!("failed to read {}: {err}", object_file.display()))?;
    if text.starts_with('\u{feff}') {
        text = text.trim_start_matches('\u{feff}').to_string();
    }
    let tag = format!("<Form>{}</Form>", escape_xml(form_name));
    if text.contains(&tag) {
        stdout.push_str(&format!(
            "[WARN] Form '{form_name}' already in ChildObjects of {type_name}.{object_name}\n"
        ));
        return Ok(());
    }
    if text.contains("<ChildObjects/>") {
        text = text.replacen(
            "<ChildObjects/>",
            &format!("<ChildObjects>\n\t\t\t{tag}\n\t\t</ChildObjects>"),
            1,
        );
    } else if text.contains("</ChildObjects>") {
        text = text.replacen(
            "</ChildObjects>",
            &format!("\t\t\t{tag}\n\t\t</ChildObjects>"),
            1,
        );
    } else {
        text = text.replacen(
            &format!("</{type_name}>"),
            &format!("\t\t<ChildObjects>\n\t\t\t{tag}\n\t\t</ChildObjects>\n\t</{type_name}>"),
            1,
        );
    }
    write_utf8_bom(&object_file, &text)?;
    stdout.push_str(&format!(
        "[INFO]   Registered form in: {}\n",
        object_file.display()
    ));
    Ok(())
}

pub(crate) fn cfe_borrow_xmlns_decl() -> &'static str {
    "xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\" xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\" xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\" xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\""
}

pub(crate) fn cfe_borrow_existing_metadata_uuid(path: &Path, child_name: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    doc.root_element()
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == child_name)
        .and_then(|node| node.attribute("uuid"))
        .map(ToOwned::to_owned)
}

pub(crate) fn cfe_borrow_xml_node(node: roxmltree::Node<'_, '_>) -> String {
    if node.is_text() {
        return escape_xml(node.text().unwrap_or_default());
    }
    if !node.is_element() {
        return String::new();
    }
    let tag = cfe_borrow_prefixed_name(node.tag_name().namespace(), node.tag_name().name());
    let mut attrs = String::new();
    for attr in node.attributes() {
        let name = cfe_borrow_prefixed_name(attr.namespace(), attr.name());
        attrs.push_str(&format!(" {name}=\"{}\"", escape_xml(attr.value())));
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

pub(crate) fn cfe_borrow_prefixed_name(namespace: Option<&str>, local_name: &str) -> String {
    match namespace {
        Some("http://v8.1c.ru/8.1/data/core") => format!("v8:{local_name}"),
        Some("http://v8.1c.ru/8.3/xcf/readable") => format!("xr:{local_name}"),
        Some("http://www.w3.org/2001/XMLSchema-instance") => format!("xsi:{local_name}"),
        _ => local_name.to_string(),
    }
}

pub(crate) fn diff_cfe(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    let result = (|| -> Result<(String, PathBuf), String> {
        let extension_path_raw =
            required_path(args, &["extensionPath", "ExtensionPath"], "ExtensionPath")?;
        let config_path_raw = required_path(args, &["configPath", "ConfigPath"], "ConfigPath")?;
        let mut extension_path = absolutize(extension_path_raw, &context.cwd);
        let mut config_path = absolutize(config_path_raw, &context.cwd);
        if extension_path.is_file() {
            extension_path = extension_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_path_buf();
        }
        if config_path.is_file() {
            config_path = config_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_path_buf();
        }

        let ext_cfg = extension_path.join("Configuration.xml");
        let src_cfg = config_path.join("Configuration.xml");
        if !ext_cfg.is_file() {
            return Err(format!(
                "Extension Configuration.xml not found: {}",
                ext_cfg.display()
            ));
        }
        if !src_cfg.is_file() {
            return Err(format!(
                "Config Configuration.xml not found: {}",
                src_cfg.display()
            ));
        }

        let ext_text = read_utf8_sig(&ext_cfg)?;
        let ext_doc = Document::parse(ext_text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("XML parse error in {}: {err}", ext_cfg.display()))?;
        let ext_root = ext_doc.root_element();
        let ext_cfg_node = ext_root
            .descendants()
            .find(|node| role_info_element(*node, "Configuration", Some(MD_NS)));
        let ext_props = ext_cfg_node.and_then(|node| meta_info_child(node, "Properties"));
        let ext_name = ext_props
            .and_then(|props| meta_info_child_text(props, "Name"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "?".to_string());
        let name_prefix = ext_props
            .and_then(|props| meta_info_child_text(props, "NamePrefix"))
            .unwrap_or_default();
        let purpose = ext_props
            .and_then(|props| meta_info_child_text(props, "ConfigurationExtensionPurpose"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "?".to_string());
        let mode = string_arg(args, &["mode", "Mode"]).unwrap_or("A");
        if !matches!(mode, "A" | "B") {
            return Err(format!(
                "argument -Mode: invalid choice: '{mode}' (choose from 'A', 'B')"
            ));
        }

        let mut lines = vec![
            format!("=== cfe-diff Mode {mode}: {ext_name} ({purpose}) ==="),
            format!("    NamePrefix: {name_prefix}"),
            String::new(),
        ];

        let child_obj_node = ext_cfg_node.and_then(|node| meta_info_child(node, "ChildObjects"));
        let Some(child_obj_node) = child_obj_node else {
            lines.push("[WARN] No ChildObjects in extension".to_string());
            return Ok((format!("{}\n", lines.join("\n")), ext_cfg));
        };

        let mut objects = Vec::<CfeDiffObject>::new();
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

        if objects.is_empty() {
            lines.push("No objects (besides Language) in extension.".to_string());
            return Ok((format!("{}\n", lines.join("\n")), ext_cfg));
        }

        if mode == "A" {
            cfe_diff_mode_a(&mut lines, &objects, &extension_path);
        } else {
            cfe_diff_mode_b(&mut lines, &objects, &extension_path, &config_path);
        }

        Ok((format!("{}\n", lines.join("\n")), ext_cfg))
    })();

    match result {
        Ok((stdout, artifact)) => AdapterOutcome {
            ok: true,
            summary: "unica.cfe.diff completed with native extension diff analyzer".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![artifact.display().to_string()],
            stdout: Some(stdout),
            stderr: Some(String::new()),
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.cfe.diff failed in native extension diff analyzer".to_string(),
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

pub(crate) fn cfe_diff_mode_a(
    lines: &mut Vec<String>,
    objects: &[CfeDiffObject],
    extension_path: &Path,
) {
    let mut borrowed = 0usize;
    let mut own = 0usize;

    for obj in objects {
        let Some(info) = cfe_diff_object_info(&obj.obj_type, &obj.name, extension_path) else {
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

            for bsl in cfe_diff_bsl_files(&obj.obj_type, &obj.name, extension_path) {
                let rel_path = cfe_diff_relative_path(&bsl, extension_path);
                let interceptors = cfe_diff_interceptors(&bsl);
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
                let Some(form_info) = cfe_diff_form_interceptors(&form_xml_path) else {
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

pub(crate) fn cfe_diff_mode_b(
    lines: &mut Vec<String>,
    objects: &[CfeDiffObject],
    extension_path: &Path,
    config_path: &Path,
) {
    let mut transferred = 0usize;
    let mut not_transferred = 0usize;
    let mut needs_review = 0usize;

    for obj in objects {
        let Some(info) = cfe_diff_object_info(&obj.obj_type, &obj.name, extension_path) else {
            continue;
        };
        if !info.exists || !info.borrowed {
            continue;
        }

        for bsl in cfe_diff_bsl_files(&obj.obj_type, &obj.name, extension_path) {
            let mac_interceptors = cfe_diff_interceptors(&bsl)
                .into_iter()
                .filter(|item| item.interceptor_type == "ИзменениеИКонтроль")
                .collect::<Vec<_>>();
            if mac_interceptors.is_empty() {
                continue;
            }
            let insert_blocks = cfe_diff_insertion_blocks(&bsl);
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
                if !config_bsl.is_file() {
                    lines.push(format!(
                        "  [NEEDS_REVIEW] {}.{} — &ИзменениеИКонтроль(\"{}\") — config module not found",
                        obj.obj_type, obj.name, interceptor.method
                    ));
                    needs_review += 1;
                    continue;
                }

                let config_content = read_utf8_sig(&config_bsl).unwrap_or_default();
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

pub(crate) fn cfe_diff_object_info(
    obj_type: &str,
    obj_name: &str,
    extension_path: &Path,
) -> Option<CfeDiffObjectInfo> {
    let dir_name = cf_validate_child_type_dir(obj_type)?;
    let obj_file = extension_path
        .join(dir_name)
        .join(format!("{obj_name}.xml"));
    if !obj_file.is_file() {
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

    let text = read_utf8_sig(&obj_file).ok()?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    let obj_el = doc
        .root_element()
        .children()
        .find(|child| child.is_element())?;
    let props_el = meta_info_child(obj_el, "Properties");
    let borrowed = props_el
        .and_then(|props| meta_info_child_text(props, "ObjectBelonging"))
        .map(|value| value == "Adopted")
        .unwrap_or(false);

    let mut attrs = 0usize;
    let mut forms = 0usize;
    let mut tabular_sections = 0usize;
    let mut borrowed_items = 0usize;
    let mut form_names = Vec::<String>::new();
    if let Some(child_objects) = meta_info_child(obj_el, "ChildObjects") {
        for child in child_objects.children().filter(|node| node.is_element()) {
            if borrowed {
                let child_borrowed = meta_info_child(child, "Properties")
                    .and_then(|props| meta_info_child_text(props, "ObjectBelonging"))
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

pub(crate) fn cfe_diff_bsl_files(
    obj_type: &str,
    obj_name: &str,
    extension_path: &Path,
) -> Vec<PathBuf> {
    let Some(dir_name) = cf_validate_child_type_dir(obj_type) else {
        return Vec::new();
    };
    let obj_dir = extension_path.join(dir_name).join(obj_name);
    if !obj_dir.is_dir() {
        return Vec::new();
    }

    let mut result = Vec::<PathBuf>::new();
    let ext_dir = obj_dir.join("Ext");
    if let Ok(entries) = fs::read_dir(ext_dir) {
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
    cfe_diff_collect_form_modules(&forms_dir, &mut result);
    result.sort();
    result
}

pub(crate) fn cfe_diff_collect_form_modules(dir: &Path, result: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            cfe_diff_collect_form_modules(&path, result);
        } else if path.file_name().and_then(|value| value.to_str()) == Some("Module.bsl") {
            result.push(path);
        }
    }
}

pub(crate) fn cfe_diff_interceptors(bsl_path: &Path) -> Vec<CfeDiffInterceptor> {
    let Ok(text) = read_utf8_sig(bsl_path) else {
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

pub(crate) fn cfe_diff_insertion_blocks(bsl_path: &Path) -> Vec<CfeDiffInsertionBlock> {
    let Ok(text) = read_utf8_sig(bsl_path) else {
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

pub(crate) fn cfe_diff_form_interceptors(form_xml_path: &Path) -> Option<(bool, Vec<String>)> {
    const FORM_NS: &str = "http://v8.1c.ru/8.3/xcf/logform";
    let text = read_utf8_sig(form_xml_path).ok()?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    let root = doc.root_element();
    let is_borrowed = skd_child(root, "BaseForm", FORM_NS).is_some();
    let mut interceptors = Vec::<String>::new();

    if let Some(events) = skd_child(root, "Events", FORM_NS) {
        for event in skd_children(events, "Event", FORM_NS) {
            let call_type = event.attribute("callType").unwrap_or("");
            if !call_type.is_empty() {
                let event_name = event.attribute("name").unwrap_or("");
                let event_text = event.text().unwrap_or("");
                interceptors.push(format!("Event:{event_name} [{call_type}] -> {event_text}"));
            }
        }
    }

    if let Some(child_items) = skd_child(root, "ChildItems", FORM_NS) {
        for element in child_items.descendants().filter(|node| node.is_element()) {
            let element_name = element.attribute("name").unwrap_or("");
            if element_name.is_empty() {
                continue;
            }
            let Some(events) = skd_child(element, "Events", FORM_NS) else {
                continue;
            };
            for event in skd_children(events, "Event", FORM_NS) {
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

    if let Some(commands) = skd_child(root, "Commands", FORM_NS) {
        for command in skd_children(commands, "Command", FORM_NS) {
            let command_name = command.attribute("name").unwrap_or("");
            for action in skd_children(command, "Action", FORM_NS) {
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

pub(crate) fn cfe_diff_relative_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

pub(crate) fn cfe_diff_normalized_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn validate_cfe(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    let result = (|| -> Result<CfeValidationRun, String> {
        let raw_path = required_path(
            args,
            &["extensionPath", "ExtensionPath", "path", "Path"],
            "ExtensionPath",
        )?;
        let mut extension_path = absolutize(raw_path, &context.cwd);
        if extension_path.is_dir() {
            let candidate = extension_path.join("Configuration.xml");
            if candidate.exists() {
                extension_path = candidate;
            } else {
                return Err(format!(
                    "[ERROR] No Configuration.xml found in directory: {}",
                    extension_path.display()
                ));
            }
        }
        if !extension_path.exists() {
            return Err(format!(
                "[ERROR] File not found: {}",
                extension_path.display()
            ));
        }
        let resolved_path = extension_path
            .canonicalize()
            .unwrap_or_else(|_| extension_path.clone());
        let config_dir = resolved_path.parent().unwrap_or(context.cwd.as_path());
        let out_file =
            path_arg(args, &["outFile", "OutFile"]).map(|path| absolutize(path, &context.cwd));
        let detailed = bool_arg(args, &["detailed", "Detailed"]);
        let max_errors = int_arg(args, &["maxErrors", "MaxErrors"])
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value > 0)
            .unwrap_or(30);

        let text = read_utf8_sig(&resolved_path)?;
        let doc = match Document::parse(text.trim_start_matches('\u{feff}')) {
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
                    out_file,
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
                out_file,
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
        let version = root.attribute("version").unwrap_or("");
        if version.is_empty() {
            report.warn("1. Missing version attribute on MetaDataObject");
        } else if !matches!(version, "2.17" | "2.20" | "2.21") {
            report.warn(format!(
                "1. Unusual version '{version}' (expected 2.17, 2.20 or 2.21)"
            ));
        }

        let Some(cfg_node) = root
            .children()
            .find(|child| role_info_element(*child, "Configuration", Some(MD_NS)))
        else {
            report.error("1. No <Configuration> element found inside MetaDataObject");
            let (ok, stdout, errors) = report.finalize();
            return Ok(CfeValidationRun {
                ok,
                stdout,
                out_file,
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

        let props_node = meta_info_child(cfg_node, "Properties");
        let obj_name = props_node
            .and_then(|props| meta_info_child_text(props, "Name"))
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
            return cfe_validation_finish(report, out_file, resolved_path);
        }

        cfe_validate_internal_info(&mut report, cfg_node);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }
        let def_lang = cfe_validate_properties(&mut report, props_node, &obj_name);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }
        cfe_validate_enum_properties(&mut report, props_node);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }

        let child_obj_node = meta_info_child(cfg_node, "ChildObjects");
        let child_index = cfe_validate_child_objects(&mut report, child_obj_node);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }
        cfe_validate_default_language(&mut report, child_obj_node, &def_lang);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }
        cfe_validate_language_files(&mut report, child_obj_node, config_dir);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }
        cfe_validate_object_dirs(&mut report, child_obj_node, config_dir);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }
        let borrowed_forms =
            cfe_validate_borrowed_objects(&mut report, child_obj_node, config_dir, &child_index);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }
        cfe_validate_borrowed_forms(&mut report, &borrowed_forms);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }
        cfe_validate_form_dependencies(&mut report, &borrowed_forms, &child_index);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }
        cfe_validate_typelinks(&mut report, &borrowed_forms);

        cfe_validation_finish(report, out_file, resolved_path)
    })();

    match result {
        Ok(run) => {
            let mut stdout = run.stdout.clone();
            let mut artifacts = vec![run.artifact.display().to_string()];
            if let Some(out_file) = &run.out_file {
                match write_utf8_bom(out_file, run.stdout.trim_end_matches('\n')) {
                    Ok(()) => {
                        stdout.push_str(&format!("Written to: {}\n", out_file.display()));
                        artifacts.push(out_file.display().to_string());
                    }
                    Err(error) => {
                        return AdapterOutcome {
                            ok: false,
                            summary: "unica.cfe.validate failed in native extension validator"
                                .to_string(),
                            changes: Vec::new(),
                            warnings: Vec::new(),
                            errors: vec![error.clone()],
                            artifacts,
                            stdout: None,
                            stderr: Some(format!("{error}\n")),
                            command: None,
                        };
                    }
                }
            }
            AdapterOutcome {
                ok: run.ok,
                summary: if run.ok {
                    "unica.cfe.validate completed with native extension validator".to_string()
                } else {
                    "unica.cfe.validate failed in native extension validator".to_string()
                },
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: run.errors,
                artifacts,
                stdout: Some(stdout),
                stderr: Some(String::new()),
                command: None,
            }
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.cfe.validate failed in native extension validator".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: Some(format!("{error}\n")),
            stderr: Some(String::new()),
            command: None,
        },
    }
}

pub(crate) fn cfe_validation_finish(
    report: CfeValidationReporter,
    out_file: Option<PathBuf>,
    artifact: PathBuf,
) -> Result<CfeValidationRun, String> {
    let (ok, stdout, errors) = report.finalize();
    Ok(CfeValidationRun {
        ok,
        stdout,
        out_file,
        artifact,
        errors,
    })
}

pub(crate) fn cfe_validate_internal_info(
    report: &mut CfeValidationReporter,
    cfg_node: roxmltree::Node<'_, '_>,
) {
    let Some(internal_info) = meta_info_child(cfg_node, "InternalInfo") else {
        report.error("2. InternalInfo: missing");
        return;
    };
    let contained = meta_info_children(internal_info, "ContainedObject");
    if contained.len() != 7 {
        report.warn(format!(
            "2. InternalInfo: expected 7 ContainedObject, found {}",
            contained.len()
        ));
    }
    let mut check_ok = true;
    let mut found = HashSet::<String>::new();
    for item in &contained {
        let class_id = meta_info_child_text(*item, "ClassId").unwrap_or_default();
        let object_id = meta_info_child_text(*item, "ObjectId").unwrap_or_default();
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

pub(crate) fn cfe_validate_properties(
    report: &mut CfeValidationReporter,
    props_node: Option<roxmltree::Node<'_, '_>>,
    obj_name: &str,
) -> String {
    let Some(props_node) = props_node else {
        report.error("3. Properties block missing");
        return String::new();
    };
    let mut check_ok = true;
    let object_belonging = meta_info_child_text(props_node, "ObjectBelonging").unwrap_or_default();
    if object_belonging != "Adopted" {
        report.error(format!(
            "3. ObjectBelonging must be 'Adopted', got '{object_belonging}'"
        ));
        check_ok = false;
    }
    if obj_name == "(unknown)" || obj_name.is_empty() {
        report.error("3. Name is missing or empty");
        check_ok = false;
    } else if !cf_validate_identifier(obj_name) {
        report.error(format!("3. Name '{obj_name}' is not a valid 1C identifier"));
        check_ok = false;
    }
    let purpose =
        meta_info_child_text(props_node, "ConfigurationExtensionPurpose").unwrap_or_default();
    if purpose.is_empty() {
        report.error("3. ConfigurationExtensionPurpose is missing");
        check_ok = false;
    } else if !["Patch", "Customization", "AddOn"].contains(&purpose.as_str()) {
        report.error(format!(
            "3. ConfigurationExtensionPurpose '{purpose}' invalid (expected: Patch, Customization, AddOn)"
        ));
        check_ok = false;
    }
    let prefix = meta_info_child_text(props_node, "NamePrefix").unwrap_or_default();
    if prefix.is_empty() {
        report.warn("3. NamePrefix is empty");
    }
    if meta_info_child(props_node, "KeepMappingToExtendedConfigurationObjectsByIDs").is_none() {
        report.warn("3. KeepMappingToExtendedConfigurationObjectsByIDs is missing");
    }
    let def_lang = meta_info_child_text(props_node, "DefaultLanguage").unwrap_or_default();
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

pub(crate) fn cfe_validate_enum_properties(
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
        if let Some(value) =
            meta_info_child_text(props_node, property).filter(|value| !value.is_empty())
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

pub(crate) fn cfe_validate_child_objects(
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
        if let Some(type_index) = cf_validate_child_object_type_index(type_name) {
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

pub(crate) fn cfe_validate_default_language(
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
    let found = meta_info_children(child_obj_node, "Language")
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

pub(crate) fn cfe_validate_language_files(
    report: &mut CfeValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
    config_dir: &Path,
) {
    let Some(child_obj_node) = child_obj_node else {
        report.warn("7. Cannot check language files (no ChildObjects)");
        return;
    };
    let lang_names = meta_info_children(child_obj_node, "Language")
        .into_iter()
        .map(|child| child.text().unwrap_or("").to_string())
        .collect::<Vec<_>>();
    if lang_names.is_empty() {
        report.warn("7. No Language entries in ChildObjects");
        return;
    }
    let mut exist_count = 0usize;
    for lang_name in &lang_names {
        if config_dir
            .join("Languages")
            .join(format!("{lang_name}.xml"))
            .exists()
        {
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

pub(crate) fn cfe_validate_object_dirs(
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
        .filter(|(dir_name, _)| !config_dir.join(*dir_name).is_dir())
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

pub(crate) struct CfeBorrowedForm {
    pub(crate) raw_text: String,
    pub(crate) context: String,
}

pub(crate) fn cfe_validate_borrowed_objects(
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
        if !obj_file.exists() {
            continue;
        }
        let Ok(text) = read_utf8_sig(&obj_file) else {
            continue;
        };
        let Ok(doc) = Document::parse(text.trim_start_matches('\u{feff}')) else {
            report.warn(format!("9. Cannot parse {dir_name}/{child_name}.xml"));
            continue;
        };
        let Some(obj_el) = doc.root_element().children().find(|node| node.is_element()) else {
            continue;
        };
        let Some(obj_props) = meta_info_child(obj_el, "Properties") else {
            continue;
        };
        if meta_info_child_text(obj_props, "ObjectBelonging").as_deref() == Some("Adopted") {
            borrowed_count += 1;
            let extended =
                meta_info_child_text(obj_props, "ExtendedConfigurationObject").unwrap_or_default();
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
        }
        if let Some(child_objects) = meta_info_child(obj_el, "ChildObjects") {
            let context = format!("{type_name}.{child_name}");
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
                        if !form_meta.exists() {
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
                        if let Ok(raw_text) = read_utf8_sig(&form_xml) {
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
            "9. Borrowed objects: {borrowed_ok_count}/{borrowed_count} validated"
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

pub(crate) fn cfe_is_borrowed_sub_item(sub_item: roxmltree::Node<'_, '_>) -> bool {
    let Some(props) = meta_info_child(sub_item, "Properties") else {
        return false;
    };
    meta_info_child_text(props, "ObjectBelonging").is_some_and(|value| !value.is_empty())
        || meta_info_child_text(props, "ExtendedConfigurationObject")
            .is_some_and(|value| !value.is_empty())
}

pub(crate) fn cfe_validate_borrowed_sub_item(
    report: &mut CfeValidationReporter,
    check_num: &str,
    context: &str,
    sub_type: &str,
    sub_item: roxmltree::Node<'_, '_>,
) -> bool {
    let Some(props) = meta_info_child(sub_item, "Properties") else {
        report.error(format!(
            "{check_num}. {context}: {sub_type} missing Properties"
        ));
        return false;
    };
    let mut ok = true;
    if meta_info_child_text(props, "ObjectBelonging").as_deref() != Some("Adopted") {
        report.error(format!(
            "{check_num}. {context}: {sub_type} ObjectBelonging must be 'Adopted'"
        ));
        ok = false;
    }
    let name = meta_info_child_text(props, "Name").unwrap_or_default();
    if name.is_empty() {
        report.error(format!("{check_num}. {context}: {sub_type} missing Name"));
        ok = false;
    }
    let extended = meta_info_child_text(props, "ExtendedConfigurationObject").unwrap_or_default();
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

pub(crate) fn cfe_validate_borrowed_forms(
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

pub(crate) fn cfe_validate_form_dependencies(
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

pub(crate) fn cfe_validate_typelinks(
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

pub(crate) fn cfe_capture_refs(raw: &str, prefix: &str, suffix: &str) -> Vec<String> {
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

pub(crate) fn cfe_validate_enum_properties_list() -> &'static [&'static str] {
    &[
        "ConfigurationExtensionCompatibilityMode",
        "DefaultRunMode",
        "ScriptVariant",
        "InterfaceCompatibilityMode",
    ]
}

pub(crate) fn cfe_validate_enum_allowed(property: &str) -> &'static [&'static str] {
    match property {
        "ConfigurationExtensionCompatibilityMode" => {
            cf_validate_enum_allowed("ConfigurationExtensionCompatibilityMode")
        }
        "DefaultRunMode" => &["ManagedApplication", "OrdinaryApplication", "Auto"],
        "ScriptVariant" => &["Russian", "English"],
        "InterfaceCompatibilityMode" => &[
            "Version8_2",
            "Version8_2EnableTaxi",
            "Taxi",
            "TaxiEnableVersion8_2",
            "TaxiEnableVersion8_5",
            "Version8_5EnableTaxi",
            "Version8_5",
        ],
        _ => &[],
    }
}

pub(crate) fn patch_extension_method(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let write_result = (|| -> Result<(String, PathBuf), String> {
        let mut extension_path =
            required_path(args, &["extensionPath", "ExtensionPath"], "ExtensionPath")
                .map(|path| absolutize(path, &context.cwd))?;
        if extension_path.is_file() {
            extension_path = extension_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| context.cwd.clone());
        }

        let cfg_file = extension_path.join("Configuration.xml");
        if !cfg_file.is_file() {
            return Err(format!(
                "Configuration.xml not found in: {}",
                extension_path.display()
            ));
        }

        let module_path = required_string(args, &["modulePath", "ModulePath"], "ModulePath")?;
        let method_name = required_string(args, &["methodName", "MethodName"], "MethodName")?;
        let interceptor_type = required_string(
            args,
            &["interceptorType", "InterceptorType"],
            "InterceptorType",
        )?;
        let context_name = string_arg(args, &["context", "Context"]).unwrap_or("НаСервере");
        let is_function = bool_arg(args, &["isFunction", "IsFunction"]);

        let name_prefix = extension_name_prefix(&cfg_file).unwrap_or_else(|| "Расш_".to_string());
        let bsl_file = bsl_file_for_module_path(&extension_path, module_path)?;
        let decorator = match interceptor_type {
            "Before" => "&Перед",
            "After" => "&После",
            "ModificationAndControl" => "&ИзменениеИКонтроль",
            _ => {
                return Err(format!(
                    "invalid InterceptorType: {interceptor_type}. Expected: Before, After, ModificationAndControl"
                ));
            }
        };
        let context_annotation = match context_name {
            "НаСервере" => "&НаСервере".to_string(),
            "НаКлиенте" => "&НаКлиенте".to_string(),
            "НаСервереБезКонтекста" => "&НаСервереБезКонтекста".to_string(),
            other => format!("&{other}"),
        };
        let proc_name = format!("{name_prefix}{method_name}");

        let keyword = if is_function {
            "Функция"
        } else {
            "Процедура"
        };
        let end_keyword = if is_function {
            "КонецФункции"
        } else {
            "КонецПроцедуры"
        };

        let mut body_lines = Vec::new();
        match interceptor_type {
            "Before" => body_lines.push("\t// TODO: код перед вызовом оригинального метода"),
            "After" => body_lines.push("\t// TODO: код после вызова оригинального метода"),
            "ModificationAndControl" => {
                body_lines.push("\t// Скопируйте тело оригинального метода и внесите изменения,");
                body_lines.push(
                    "\t// используя маркеры #Удаление / #КонецУдаления и #Вставка / #КонецВставки",
                );
            }
            _ => {}
        }
        if is_function {
            body_lines.push("\t");
            body_lines.push(
                "\tВозврат Неопределено; // TODO: заменить на реальное возвращаемое значение",
            );
        }

        let mut bsl_code = vec![
            context_annotation.clone(),
            format!("{decorator}(\"{method_name}\")"),
            format!("{keyword} {proc_name}()"),
        ];
        bsl_code.extend(body_lines.into_iter().map(ToOwned::to_owned));
        bsl_code.push(end_keyword.to_string());
        let bsl_text = format!("{}\r\n", bsl_code.join("\r\n"));

        let mut stdout = String::new();
        if let Some((obj_type, obj_name, form_name)) = form_module_parts(module_path) {
            let dir_name = object_type_dir(obj_type)
                .ok_or_else(|| format!("Unknown object type: {obj_type}"))?;
            let form_meta_file = extension_path
                .join(dir_name)
                .join(obj_name)
                .join("Forms")
                .join(format!("{form_name}.xml"));
            let form_xml_file = extension_path
                .join(dir_name)
                .join(obj_name)
                .join("Forms")
                .join(form_name)
                .join("Ext")
                .join("Form.xml");
            if !form_meta_file.is_file() || !form_xml_file.is_file() {
                stdout.push_str(&format!(
                    "[WARN] Form '{form_name}' metadata or Form.xml not found in extension.\n"
                ));
                stdout.push_str("       Run /cfe-borrow first:\n");
                stdout.push_str(&format!(
                    "       /cfe-borrow -ExtensionPath {} -ConfigPath <ConfigPath> -Object \"{module_path}\"\n\n",
                    extension_path.display()
                ));
            }
        }

        if let Some(parent) = bsl_file.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }

        let created = if bsl_file.is_file() {
            let existing = fs::read_to_string(&bsl_file)
                .map_err(|err| format!("failed to read {}: {err}", bsl_file.display()))?
                .trim_start_matches('\u{feff}')
                .to_string();
            let separator = if !existing.is_empty() && !existing.ends_with('\n') {
                "\r\n\r\n"
            } else {
                "\r\n"
            };
            write_utf8_bom(&bsl_file, &format!("{existing}{separator}{bsl_text}"))?;
            false
        } else {
            write_utf8_bom(&bsl_file, &bsl_text)?;
            true
        };

        if created {
            stdout.push_str("[OK] Создан файл модуля\n");
        } else {
            stdout.push_str("[OK] Добавлен перехватчик в существующий файл\n");
        }
        stdout.push_str(&format!("     Файл:         {}\n", bsl_file.display()));
        stdout.push_str(&format!(
            "     Декоратор:    {decorator}(\"{method_name}\")\n"
        ));
        stdout.push_str(&format!("     Процедура:    {proc_name}()\n"));
        stdout.push_str(&format!("     Контекст:     {context_annotation}\n"));

        Ok((stdout, bsl_file))
    })();

    match write_result {
        Ok((stdout, bsl_file)) => AdapterOutcome {
            ok: true,
            summary: "unica.cfe.patch_method completed with native BSL interceptor writer"
                .to_string(),
            changes: vec![format!("updated {}", bsl_file.display())],
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![bsl_file.display().to_string()],
            stdout: Some(stdout),
            stderr: None,
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.cfe.patch_method failed in native BSL interceptor writer".to_string(),
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

pub(crate) fn create_extension_scaffold(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let name = string_arg(args, &["name", "Name"]).unwrap_or("");
    if name.is_empty() {
        return AdapterOutcome {
            ok: false,
            summary: "unica.cfe.init failed in native XML scaffold writer".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec!["missing required Name argument".to_string()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some("missing required Name argument\n".to_string()),
            command: None,
        };
    }
    let synonym = string_arg(args, &["synonym", "Synonym"]).unwrap_or(name);
    let name_prefix = string_arg(args, &["namePrefix", "NamePrefix"])
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("{name}_"));
    let out_dir = output_dir_arg(
        args,
        context,
        &["outputDir", "OutputDir", "extensionPath", "ExtensionPath"],
        "src",
    );
    let config = out_dir.join("Configuration.xml");
    let languages = out_dir.join("Languages");
    let language = languages.join("Русский.xml");
    let no_role = bool_arg(args, &["noRole", "NoRole"]);
    let role_name = format!("{name_prefix}ОсновнаяРоль");
    let role = (!no_role).then(|| out_dir.join("Roles").join(format!("{role_name}.xml")));

    let write_result = (|| -> Result<String, String> {
        if config.exists() {
            return Err(format!(
                "Configuration.xml already exists: {}",
                config.display()
            ));
        }

        let mut stdout_prefix = String::new();
        let mut base_lang_uuid = "00000000-0000-0000-0000-000000000000".to_string();
        let mut compatibility = string_arg(args, &["compatibilityMode", "CompatibilityMode"])
            .unwrap_or("Version8_3_24")
            .to_string();
        let mut format_version = "2.17".to_string();
        let interface_mode = if let Some(config_path) =
            path_arg(args, &["configPath", "ConfigPath"])
        {
            let mut config_path = absolutize(config_path, &context.cwd);
            if config_path.is_dir() {
                let candidate = config_path.join("Configuration.xml");
                if candidate.exists() {
                    config_path = candidate;
                } else {
                    return Err(format!(
                        "No Configuration.xml in config directory: {}",
                        config_path.display()
                    ));
                }
            }
            if !config_path.exists() {
                return Err(format!("Config file not found: {}", config_path.display()));
            }

            let cfg_dir = config_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| context.cwd.clone());
            let base_lang_file = cfg_dir.join("Languages").join("Русский.xml");
            if base_lang_file.exists() {
                match fs::read_to_string(&base_lang_file) {
                    Ok(text) => match Document::parse(text.trim_start_matches('\u{feff}')) {
                        Ok(doc) => {
                            if let Some(uuid) = doc
                                .descendants()
                                .find(|node| {
                                    node.is_element() && node.tag_name().name() == "Language"
                                })
                                .and_then(|node| node.attribute("uuid"))
                            {
                                base_lang_uuid = uuid.to_string();
                                stdout_prefix.push_str(&format!(
                                    "[INFO] Base config Language UUID: {base_lang_uuid}\n"
                                ));
                            }
                        }
                        Err(_) => {
                            stdout_prefix.push_str(&format!(
                                "[WARN] Could not parse {}\n",
                                base_lang_file.display()
                            ));
                        }
                    },
                    Err(_) => {
                        stdout_prefix.push_str(&format!(
                            "[WARN] Could not parse {}\n",
                            base_lang_file.display()
                        ));
                    }
                }
            } else {
                stdout_prefix.push_str(&format!(
                    "[WARN] Base config language not found: {}\n",
                    base_lang_file.display()
                ));
            }

            match fs::read_to_string(&config_path) {
                Ok(text) => match Document::parse(text.trim_start_matches('\u{feff}')) {
                    Ok(doc) => {
                        if let Some(value) = doc.root_element().attribute("version") {
                            format_version = value.to_string();
                            stdout_prefix.push_str(&format!(
                                "[INFO] Base config MDClasses format version: {format_version}\n"
                            ));
                        }
                        if let Some(value) = first_text(&doc, "CompatibilityMode") {
                            compatibility = value;
                            stdout_prefix.push_str(&format!(
                                "[INFO] Base config CompatibilityMode: {compatibility}\n"
                            ));
                        } else {
                            stdout_prefix.push_str(&format!(
                                "[WARN] CompatibilityMode not found in base config, using default: {compatibility}\n"
                            ));
                        }
                        if let Some(value) = first_text(&doc, "InterfaceCompatibilityMode") {
                            stdout_prefix.push_str(&format!(
                                "[INFO] Base config InterfaceCompatibilityMode: {value}\n"
                            ));
                            value
                        } else {
                            let value = "TaxiEnableVersion8_2".to_string();
                            stdout_prefix.push_str(&format!(
                                "[WARN] InterfaceCompatibilityMode not found in base config, using default: {value}\n"
                            ));
                            value
                        }
                    }
                    Err(_) => {
                        stdout_prefix.push_str(&format!(
                            "[WARN] Could not parse base config, using default CompatibilityMode: {compatibility}\n"
                        ));
                        "TaxiEnableVersion8_2".to_string()
                    }
                },
                Err(_) => {
                    stdout_prefix.push_str(&format!(
                        "[WARN] Could not parse base config, using default CompatibilityMode: {compatibility}\n"
                    ));
                    "TaxiEnableVersion8_2".to_string()
                }
            }
        } else {
            stdout_prefix.push_str("[WARN] Language ExtendedConfigurationObject set to zeros. Use -ConfigPath to auto-resolve from base config, or fix manually before loading.\n");
            "TaxiEnableVersion8_2".to_string()
        };

        let uuid_cfg = stable_uuid(20);
        let uuid_lang = stable_uuid(21);
        let uuid_role = stable_uuid(22);
        let contained_object_ids = (23..30).map(stable_uuid).collect::<Vec<_>>();
        let contained_objects = contained_objects_xml(&contained_object_ids);
        let purpose = string_arg(args, &["purpose", "Purpose"]).unwrap_or("Customization");
        let format_version_xml = escape_xml(&format_version);
        let vendor_xml = string_arg(args, &["vendor", "Vendor"])
            .map(escape_xml)
            .unwrap_or_default();
        let version_xml = string_arg(args, &["version", "Version"])
            .map(escape_xml)
            .unwrap_or_default();
        let synonym_xml = format!(
            "\r\n\t\t\t\t<v8:item>\r\n\t\t\t\t\t<v8:lang>ru</v8:lang>\r\n\t\t\t\t\t<v8:content>{}</v8:content>\r\n\t\t\t\t</v8:item>\r\n\t\t\t",
            escape_xml(synonym)
        );
        let default_roles_xml = if no_role {
            String::new()
        } else {
            format!(
                "\r\n\t\t\t\t<xr:Item xsi:type=\"xr:MDObjectRef\">Role.{}</xr:Item>\r\n\t\t\t",
                escape_xml(&role_name)
            )
        };
        let mut child_objects_xml = "\r\n\t\t\t<Language>Русский</Language>".to_string();
        if !no_role {
            child_objects_xml.push_str(&format!(
                "\r\n\t\t\t<Role>{}</Role>",
                escape_xml(&role_name)
            ));
        }
        child_objects_xml.push_str("\r\n\t\t");

        fs::create_dir_all(&out_dir)
            .map_err(|err| format!("failed to create {}: {err}", out_dir.display()))?;
        fs::create_dir_all(&languages)
            .map_err(|err| format!("failed to create {}: {err}", languages.display()))?;

        write_utf8_bom(
            &config,
            &format!(
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
			<ConfigurationExtensionPurpose>{purpose}</ConfigurationExtensionPurpose>
			<KeepMappingToExtendedConfigurationObjectsByIDs>true</KeepMappingToExtendedConfigurationObjectsByIDs>
			<NamePrefix>{name_prefix}</NamePrefix>
			<ConfigurationExtensionCompatibilityMode>{compatibility}</ConfigurationExtensionCompatibilityMode>
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
			<InterfaceCompatibilityMode>{interface_mode}</InterfaceCompatibilityMode>
		</Properties>
		<ChildObjects>{child_objects_xml}</ChildObjects>
	</Configuration>
</MetaDataObject>"#,
                name = escape_xml(name),
                name_prefix = escape_xml(&name_prefix),
            ),
        )?;
        write_utf8_bom(
            &language,
            &format!(
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
            ),
        )?;

        if let Some(role) = &role {
            let role_dir = role.parent().ok_or_else(|| {
                format!("failed to resolve role directory for {}", role.display())
            })?;
            fs::create_dir_all(role_dir)
                .map_err(|err| format!("failed to create {}: {err}", role_dir.display()))?;
            write_utf8_bom(
                role,
                &format!(
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
                    role_name = escape_xml(&role_name),
                ),
            )?;
        }

        let mut stdout = format!(
            "{stdout_prefix}[OK] Создано расширение: {name}\n     Каталог:            {}\n     Назначение:         {purpose}\n     Префикс:           {name_prefix}\n     Совместимость:     {compatibility}\n     Configuration.xml:  {}\n     Languages:          {}\n",
            out_dir.display(),
            config.display(),
            language.display()
        );
        if let Some(role) = &role {
            stdout.push_str(&format!("     Role:               {}\n", role.display()));
        }
        Ok(stdout)
    })();

    match write_result {
        Ok(stdout) => {
            let mut changes = vec![
                format!("created {}", config.display()),
                format!("created {}", language.display()),
            ];
            let mut artifacts = vec![config.display().to_string(), language.display().to_string()];
            if let Some(role) = &role {
                changes.push(format!("created {}", role.display()));
                artifacts.push(role.display().to_string());
            }
            AdapterOutcome {
                ok: true,
                summary: "unica.cfe.init completed with native XML scaffold writer".to_string(),
                changes,
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts,
                stdout: Some(stdout),
                stderr: None,
                command: None,
            }
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.cfe.init failed in native XML scaffold writer".to_string(),
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

pub(crate) fn bsl_file_for_module_path(
    extension_path: &Path,
    module_path: &str,
) -> Result<PathBuf, String> {
    let parts = module_path.split('.').collect::<Vec<_>>();
    if parts.len() < 2 {
        return Err(format!(
            "Invalid ModulePath format: {module_path}. Expected: Type.Name.Module or CommonModule.Name"
        ));
    }

    let obj_type = parts[0];
    let obj_name = parts[1];
    let dir_name =
        object_type_dir(obj_type).ok_or_else(|| format!("Unknown object type: {obj_type}"))?;

    if obj_type == "CommonModule" {
        Ok(extension_path
            .join(dir_name)
            .join(obj_name)
            .join("Ext")
            .join("Module.bsl"))
    } else if parts.len() >= 4 && parts[2] == "Form" {
        let form_name = parts[3];
        Ok(extension_path
            .join(dir_name)
            .join(obj_name)
            .join("Forms")
            .join(form_name)
            .join("Ext")
            .join("Form")
            .join("Module.bsl"))
    } else if parts.len() >= 3 {
        let module_file_name = match parts[2] {
            "ObjectModule" => "ObjectModule.bsl",
            "ManagerModule" => "ManagerModule.bsl",
            "RecordSetModule" => "RecordSetModule.bsl",
            "CommandModule" => "CommandModule.bsl",
            other => {
                return Ok(extension_path
                    .join(dir_name)
                    .join(obj_name)
                    .join("Ext")
                    .join(format!("{other}.bsl")))
            }
        };
        Ok(extension_path
            .join(dir_name)
            .join(obj_name)
            .join("Ext")
            .join(module_file_name))
    } else {
        Err(format!(
            "Invalid ModulePath format: {module_path}. Expected: Type.Name.Module, Type.Name.Form.FormName, or CommonModule.Name"
        ))
    }
}

pub(crate) fn form_module_parts(module_path: &str) -> Option<(&str, &str, &str)> {
    let parts = module_path.split('.').collect::<Vec<_>>();
    if parts.len() >= 4 && parts[2] == "Form" {
        Some((parts[0], parts[1], parts[3]))
    } else {
        None
    }
}

pub(crate) fn object_type_dir(obj_type: &str) -> Option<&'static str> {
    match obj_type {
        "Catalog" | "Catalogs" => Some("Catalogs"),
        "Document" | "Documents" => Some("Documents"),
        "Enum" | "Enums" => Some("Enums"),
        "CommonModule" | "CommonModules" => Some("CommonModules"),
        "Report" | "Reports" => Some("Reports"),
        "DataProcessor" | "DataProcessors" => Some("DataProcessors"),
        "ExchangePlan" | "ExchangePlans" => Some("ExchangePlans"),
        "ChartOfAccounts" | "ChartsOfAccounts" => Some("ChartsOfAccounts"),
        "ChartOfCharacteristicTypes" | "ChartsOfCharacteristicTypes" => {
            Some("ChartsOfCharacteristicTypes")
        }
        "ChartOfCalculationTypes" | "ChartsOfCalculationTypes" => Some("ChartsOfCalculationTypes"),
        "BusinessProcess" | "BusinessProcesses" => Some("BusinessProcesses"),
        "Task" | "Tasks" => Some("Tasks"),
        "InformationRegister" | "InformationRegisters" => Some("InformationRegisters"),
        "AccumulationRegister" | "AccumulationRegisters" => Some("AccumulationRegisters"),
        "AccountingRegister" | "AccountingRegisters" => Some("AccountingRegisters"),
        "CalculationRegister" | "CalculationRegisters" => Some("CalculationRegisters"),
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
        "cfe-validate" => Some(Ok(validate_cfe(args, context))),
        "cfe-diff" => Some(Ok(diff_cfe(args, context))),
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
        "cfe-borrow" => Some(borrow_cfe(args, context)),
        "cfe-init" => Some(create_extension_scaffold(args, context)),
        "cfe-patch-method" => Some(patch_extension_method(args, context)),
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
        let root = std::env::temp_dir().join(format!("unica-cfe-borrow-{name}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build").join("unica"),
            workspace_epoch: 1,
        }
    }

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn borrow_cfe_preserves_existing_form_module_on_repeated_form_borrow() {
        let context = temp_context("preserve-form-module");
        let src = context.cwd.join("src");
        let ext = context.cwd.join("ext");
        write_file(
            &src.join("Configuration.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.17">
	<Configuration uuid="55555555-5555-5555-5555-555555555555">
		<Properties>
			<Name>ParityConfiguration</Name>
			<NamePrefix/>
		</Properties>
		<ChildObjects>
			<Catalog>ParityCatalog</Catalog>
		</ChildObjects>
	</Configuration>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs")
                .join("ParityCatalog")
                .join("Forms")
                .join("MainForm.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.17">
	<Form uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa">
		<Properties>
			<Name>MainForm</Name>
			<FormType>Managed</FormType>
		</Properties>
	</Form>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs")
                .join("ParityCatalog")
                .join("Forms")
                .join("MainForm")
                .join("Ext")
                .join("Form.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.17">
	<Attributes/>
</Form>
"#,
        );
        write_file(
            &ext.join("Configuration.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.17">
	<Configuration uuid="66666666-6666-6666-6666-666666666666">
		<Properties>
			<Name>ParityExtension</Name>
			<NamePrefix>PE_</NamePrefix>
		</Properties>
		<ChildObjects>
			<Catalog>ParityCatalog</Catalog>
		</ChildObjects>
	</Configuration>
</MetaDataObject>
"#,
        );
        write_file(
            &ext.join("Catalogs").join("ParityCatalog.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.17">
	<Catalog uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb">
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>ParityCatalog</Name>
			<ExtendedConfigurationObject>11111111-1111-1111-1111-111111111111</ExtendedConfigurationObject>
		</Properties>
		<ChildObjects>
			<Form>MainForm</Form>
		</ChildObjects>
	</Catalog>
</MetaDataObject>
"#,
        );
        let form_meta_path = ext
            .join("Catalogs")
            .join("ParityCatalog")
            .join("Forms")
            .join("MainForm.xml");
        let existing_form_meta_uuid = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        write_file(
            &form_meta_path,
            &format!(
                r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.17">
	<Form uuid="{existing_form_meta_uuid}">
		<InternalInfo/>
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>MainForm</Name>
			<ExtendedConfigurationObject>aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa</ExtendedConfigurationObject>
			<FormType>Managed</FormType>
		</Properties>
	</Form>
</MetaDataObject>
"#
            ),
        );
        let module_path = ext
            .join("Catalogs")
            .join("ParityCatalog")
            .join("Forms")
            .join("MainForm")
            .join("Ext")
            .join("Form")
            .join("Module.bsl");
        let existing_module = "Procedure ExistingHandler()\nEndProcedure\n";
        write_file(&module_path, existing_module);

        let mut args = Map::new();
        args.insert("ExtensionPath".to_string(), json!("ext"));
        args.insert("ConfigPath".to_string(), json!("src"));
        args.insert(
            "Object".to_string(),
            json!("Catalog.ParityCatalog.Form.MainForm"),
        );
        args.insert("BorrowMainAttribute".to_string(), json!("Form"));

        let outcome = borrow_cfe(&args, &context);

        assert!(outcome.ok, "{:?}", outcome.errors);
        assert_eq!(fs::read_to_string(&module_path).unwrap(), existing_module);
        assert!(
            fs::read_to_string(&form_meta_path)
                .unwrap()
                .contains(existing_form_meta_uuid),
            "existing form metadata uuid must survive re-borrow"
        );
        let stdout = outcome.stdout.as_deref().unwrap_or_default();
        assert!(
            stdout.contains("[SKIP] Module.bsl already exists"),
            "{stdout}"
        );
        let module_artifact = module_path.display().to_string();
        assert!(!outcome.artifacts.contains(&module_artifact));
        assert!(!outcome
            .changes
            .iter()
            .any(|change| change.contains(&module_artifact)));

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_init_inherits_mdclasses_format_version_from_base_config() {
        let context = temp_context("init-format-version");
        let src = context.cwd.join("src");
        write_file(
            &src.join("Configuration.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="55555555-5555-5555-5555-555555555555">
		<Properties>
			<Name>ParityConfiguration</Name>
			<CompatibilityMode>Version8_3_25</CompatibilityMode>
			<InterfaceCompatibilityMode>TaxiEnableVersion8_5</InterfaceCompatibilityMode>
		</Properties>
	</Configuration>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Languages").join("Русский.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Language uuid="77777777-7777-7777-7777-777777777777"/>
</MetaDataObject>
"#,
        );

        let mut args = Map::new();
        args.insert("Name".to_string(), json!("ParityExtension"));
        args.insert("OutputDir".to_string(), json!("ext"));
        args.insert("ConfigPath".to_string(), json!("src/Configuration.xml"));

        let outcome = create_extension_scaffold(&args, &context);

        assert!(outcome.ok, "{:?}", outcome.errors);
        for path in [
            context.cwd.join("ext").join("Configuration.xml"),
            context
                .cwd
                .join("ext")
                .join("Languages")
                .join("Русский.xml"),
            context
                .cwd
                .join("ext")
                .join("Roles")
                .join("ParityExtension_ОсновнаяРоль.xml"),
        ] {
            let text = fs::read_to_string(&path).unwrap();
            assert!(
                text.contains(r#"version="2.20""#),
                "{} did not inherit base MDClasses format version:\n{text}",
                path.display()
            );
        }

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn borrow_cfe_enriches_main_attribute_paths_and_reference_shells() {
        let context = temp_context("borrow-main-attributes");
        let src = context.cwd.join("src");
        let ext = context.cwd.join("ext");
        write_file(
            &src.join("Configuration.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="55555555-5555-5555-5555-555555555555">
		<Properties>
			<Name>ParityConfiguration</Name>
			<NamePrefix/>
		</Properties>
		<ChildObjects>
			<Catalog>Orders</Catalog>
			<Catalog>Counterparty</Catalog>
			<Catalog>Products</Catalog>
			<DefinedType>StatusType</DefinedType>
		</ChildObjects>
	</Configuration>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs").join("Orders.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
	<Catalog uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa">
		<InternalInfo/>
		<Properties>
			<Name>Orders</Name>
		</Properties>
		<ChildObjects>
			<Attribute uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb">
				<Properties>
					<Name>Customer</Name>
					<Type><v8:Type>cfg:CatalogRef.Counterparty</v8:Type></Type>
				</Properties>
			</Attribute>
			<Attribute uuid="cccccccc-cccc-cccc-cccc-cccccccccccc">
				<Properties>
					<Name>Agreement</Name>
					<Type><v8:Type>cfg:DefinedType.StatusType</v8:Type></Type>
				</Properties>
			</Attribute>
			<TabularSection uuid="dddddddd-dddd-dddd-dddd-dddddddddddd">
				<InternalInfo>
					<xr:GeneratedType name="CatalogTabularSection.Orders.Items" category="TabularSection">
						<xr:TypeId>11111111-1111-1111-1111-111111111111</xr:TypeId>
						<xr:ValueId>22222222-2222-2222-2222-222222222222</xr:ValueId>
					</xr:GeneratedType>
				</InternalInfo>
				<Properties>
					<Name>Items</Name>
				</Properties>
				<ChildObjects>
					<Attribute uuid="eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee">
						<Properties>
							<Name>Product</Name>
							<Type><v8:Type>cfg:CatalogRef.Products</v8:Type></Type>
						</Properties>
					</Attribute>
					<Attribute uuid="ffffffff-ffff-ffff-ffff-ffffffffffff">
						<Properties>
							<Name>Quantity</Name>
							<Type><v8:Type>xs:decimal</v8:Type></Type>
						</Properties>
					</Attribute>
				</ChildObjects>
			</TabularSection>
		</ChildObjects>
	</Catalog>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs").join("Counterparty.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<Catalog uuid="12345678-1234-1234-1234-123456789abc">
		<Properties><Name>Counterparty</Name></Properties>
		<ChildObjects>
			<Attribute uuid="11111111-aaaa-aaaa-aaaa-aaaaaaaaaaaa">
				<Properties>
					<Name>TaxId</Name>
					<Type><v8:Type>xs:string</v8:Type></Type>
				</Properties>
			</Attribute>
		</ChildObjects>
	</Catalog>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs").join("Products.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<Catalog uuid="87654321-4321-4321-4321-cba987654321">
		<Properties><Name>Products</Name></Properties>
		<ChildObjects>
			<Attribute uuid="22222222-bbbb-bbbb-bbbb-bbbbbbbbbbbb">
				<Properties>
					<Name>Sku</Name>
					<Type><v8:Type>xs:string</v8:Type></Type>
				</Properties>
			</Attribute>
		</ChildObjects>
	</Catalog>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("DefinedTypes").join("StatusType.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<DefinedType uuid="99999999-9999-9999-9999-999999999999">
		<Properties>
			<Name>StatusType</Name>
			<Type><v8:Type>xs:string</v8:Type></Type>
		</Properties>
	</DefinedType>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs")
                .join("Orders")
                .join("Forms")
                .join("MainForm.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Form uuid="aaaaaaaa-1111-1111-1111-aaaaaaaaaaaa">
		<Properties><Name>MainForm</Name></Properties>
	</Form>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs")
                .join("Orders")
                .join("Forms")
                .join("MainForm")
                .join("Ext")
                .join("Form.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<ChildItems>
		<InputField name="CustomerTaxId" id="1000001"><DataPath>Объект.Customer.TaxId</DataPath></InputField>
		<InputField name="ProductSku" id="1000002"><DataPath>Объект.Items.Product.Sku</DataPath></InputField>
		<InputField name="Quantity" id="1000003"><DataPath>Объект.Items.Quantity</DataPath></InputField>
		<CommandBar name="AgreementCommand" id="1000004"><Field>Объект.Agreement</Field></CommandBar>
	</ChildItems>
	<Attributes/>
</Form>
"#,
        );
        write_file(
            &ext.join("Configuration.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="66666666-6666-6666-6666-666666666666">
		<Properties>
			<Name>ParityExtension</Name>
			<NamePrefix>PE_</NamePrefix>
		</Properties>
		<ChildObjects/>
	</Configuration>
</MetaDataObject>
"#,
        );

        let mut args = Map::new();
        args.insert("ExtensionPath".to_string(), json!("ext"));
        args.insert("ConfigPath".to_string(), json!("src"));
        args.insert("Object".to_string(), json!("Catalog.Orders.Form.MainForm"));
        args.insert("BorrowMainAttribute".to_string(), json!("Form"));

        let outcome = borrow_cfe(&args, &context);

        assert!(outcome.ok, "{:?}", outcome.errors);
        let order_xml = fs::read_to_string(ext.join("Catalogs").join("Orders.xml")).unwrap();
        for expected in [
            "<Name>Customer</Name>",
            "<Name>Agreement</Name>",
            "<Name>Items</Name>",
            "<Name>Product</Name>",
            "<Name>Quantity</Name>",
        ] {
            assert!(
                order_xml.contains(expected),
                "missing {expected} in:\n{order_xml}"
            );
        }
        assert!(
            order_xml.contains("cfg:DefinedType.StatusType"),
            "{order_xml}"
        );

        let counterparty_xml =
            fs::read_to_string(ext.join("Catalogs").join("Counterparty.xml")).unwrap();
        assert!(
            counterparty_xml.contains("<Name>TaxId</Name>"),
            "{counterparty_xml}"
        );
        let products_xml = fs::read_to_string(ext.join("Catalogs").join("Products.xml")).unwrap();
        assert!(products_xml.contains("<Name>Sku</Name>"), "{products_xml}");
        let defined_type_xml =
            fs::read_to_string(ext.join("DefinedTypes").join("StatusType.xml")).unwrap();
        assert!(
            defined_type_xml.contains("<Type><v8:Type>xs:string</v8:Type></Type>"),
            "{defined_type_xml}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }
}
