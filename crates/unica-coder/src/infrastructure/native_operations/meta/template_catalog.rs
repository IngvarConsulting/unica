#![allow(dead_code, unused_imports)]

use roxmltree::Document;
use serde_json::{Map, Value};

use super::super::common::is_1c_identifier;
use super::super::common::{escape_xml, json_i64_value, json_value_to_python_string};
use super::legacy_dsl::normalize_meta_object_type;
use super::publisher::fresh_meta_compile_uuid;
use super::xml_model::{
    emit_meta_event_subscription_source_type_contents, emit_meta_fill_value, emit_meta_mltext,
    emit_meta_type_contents, emit_meta_value_type,
};
use crate::application::metadata::MetaFailure;
use crate::application::ports::MetadataAuxiliaryXmlKind;
use crate::domain::metadata::{MetaDiagnostic, MetaDiagnosticCode, MetadataKind};
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use crate::infrastructure::metadata_kinds::metadata_layout;
use crate::infrastructure::platform_xml_source_targets::ResolvedSourceSet;
use std::path::PathBuf;

pub(crate) trait MetadataTemplateCatalog {
    fn minimal_object(
        &self,
        source: &ResolvedSourceSet,
        kind: MetadataKind,
        name: &str,
    ) -> Result<MetadataPostImage, MetaFailure>;
}

pub(crate) struct PlatformMetadataTemplateCatalog;

pub(crate) struct MetadataPostImage {
    pub(crate) metadata_path: MetadataAddress,
    pub(crate) files: Vec<MetadataTemplateFile>,
}

pub(crate) struct MetadataTemplateFile {
    pub(crate) role: MetadataTemplateFileRole,
    pub(crate) mode: MetadataTemplateFileMode,
    pub(crate) relative_path: PathBuf,
    pub(crate) bytes: Vec<u8>,
    pub(crate) preimage: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MetadataTemplateFileRole {
    Descriptor,
    Module,
    AuxiliaryXml(MetadataAuxiliaryXmlKind),
    Dependency(MetadataAddress),
    DependencyModule(MetadataAddress),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetadataTemplateFileMode {
    Create,
    Guard,
    Replace,
}

struct MinimalTemplateContext {
    chart_of_accounts: Option<String>,
    chart_of_calculation_types: Option<String>,
    method_name: Option<String>,
    event_source: Option<String>,
    event_handler: Option<String>,
    dependencies: Vec<MetadataTemplateFile>,
}

impl MinimalTemplateContext {
    fn from_source(
        source: &ResolvedSourceSet,
        kind: MetadataKind,
        new_name: &str,
    ) -> Result<Self, MetaFailure> {
        let mut context = Self {
            chart_of_accounts: None,
            chart_of_calculation_types: None,
            method_name: None,
            event_source: None,
            event_handler: None,
            dependencies: Vec::new(),
        };
        match kind {
            MetadataKind::AccountingRegister => {
                let chart = required_registered_name(source, "ChartOfAccounts", kind)?;
                context.chart_of_accounts = Some(format!("ChartOfAccounts.{chart}"));
                context.dependencies.push(read_dependency(
                    source,
                    "ChartOfAccounts",
                    &chart,
                    MetadataTemplateFileMode::Guard,
                    None,
                )?);
                context.dependencies.push(registrar_dependency(
                    source,
                    &format!("AccountingRegister.{new_name}"),
                    kind,
                )?);
            }
            MetadataKind::CalculationRegister => {
                let chart = required_registered_name(source, "ChartOfCalculationTypes", kind)?;
                context.chart_of_calculation_types =
                    Some(format!("ChartOfCalculationTypes.{chart}"));
                context.dependencies.push(read_dependency(
                    source,
                    "ChartOfCalculationTypes",
                    &chart,
                    MetadataTemplateFileMode::Guard,
                    None,
                )?);
                context.dependencies.push(registrar_dependency(
                    source,
                    &format!("CalculationRegister.{new_name}"),
                    kind,
                )?);
            }
            MetadataKind::ScheduledJob => {
                let (module, method, module_guard) =
                    required_common_module_method(source, kind, 0)?;
                context.method_name = Some(format!("CommonModule.{module}.{method}"));
                context.dependencies.push(read_dependency(
                    source,
                    "CommonModule",
                    &module,
                    MetadataTemplateFileMode::Guard,
                    None,
                )?);
                context.dependencies.push(module_guard);
            }
            MetadataKind::EventSubscription => {
                let (module, method, module_guard) =
                    required_common_module_method(source, kind, 2)?;
                let catalog = required_registered_name(source, "Catalog", kind)?;
                context.event_handler = Some(format!("CommonModule.{module}.{method}"));
                context.event_source = Some(format!("CatalogObject.{catalog}"));
                context.dependencies.push(read_dependency(
                    source,
                    "CommonModule",
                    &module,
                    MetadataTemplateFileMode::Guard,
                    None,
                )?);
                context.dependencies.push(module_guard);
                context.dependencies.push(read_dependency(
                    source,
                    "Catalog",
                    &catalog,
                    MetadataTemplateFileMode::Guard,
                    None,
                )?);
            }
            _ => {}
        }
        Ok(context)
    }
}

fn required_common_module_method(
    source: &ResolvedSourceSet,
    requested: MetadataKind,
    arity: usize,
) -> Result<(String, String, MetadataTemplateFile), MetaFailure> {
    for module in registered_names(&source.owner_preimage, "CommonModule") {
        let relative_path = PathBuf::from("CommonModules")
            .join(&module)
            .join("Ext/Module.bsl");
        let Ok(bytes) = std::fs::read(source.source_root.join(&relative_path)) else {
            continue;
        };
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let Some(method) = first_exported_procedure(text.trim_start_matches('\u{feff}'), arity)
        else {
            continue;
        };
        let module_target = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("CommonModule.{module}"),
        )
        .expect("registered common module has a canonical logical address");
        return Ok((
            module,
            method,
            MetadataTemplateFile {
                role: MetadataTemplateFileRole::DependencyModule(module_target),
                mode: MetadataTemplateFileMode::Guard,
                relative_path,
                bytes: bytes.clone(),
                preimage: Some(bytes),
            },
        ));
    }
    Err(MetaDiagnostic::error(
        MetaDiagnosticCode::CapabilityUnavailable,
        format!(
            "sourceSet does not contain an exported {arity}-argument common-module procedure required by {}",
            requested.as_str()
        ),
    )
    .into())
}

fn first_exported_procedure(source: &str, arity: usize) -> Option<String> {
    source.lines().find_map(|line| {
        let line = line.trim();
        let lower = line.to_lowercase();
        let signature = if lower.starts_with("procedure ") {
            &line["procedure ".len()..]
        } else if lower.starts_with("процедура ") {
            &line["процедура ".len()..]
        } else {
            return None;
        };
        let open = signature.find('(')?;
        let close = signature[open + 1..].find(')')? + open + 1;
        let tail = signature[close + 1..].trim().to_lowercase();
        if !tail
            .split_whitespace()
            .any(|part| matches!(part, "export" | "экспорт"))
        {
            return None;
        }
        let parameters = signature[open + 1..close].trim();
        let actual_arity = if parameters.is_empty() {
            0
        } else {
            parameters.split(',').count()
        };
        (actual_arity == arity)
            .then(|| signature[..open].trim().to_string())
            .filter(|name| is_1c_identifier(name))
    })
}

fn required_registered_name(
    source: &ResolvedSourceSet,
    tag: &str,
    requested: MetadataKind,
) -> Result<String, MetaFailure> {
    registered_names(&source.owner_preimage, tag)
        .into_iter()
        .next()
        .ok_or_else(|| {
            MetaDiagnostic::error(
                MetaDiagnosticCode::CapabilityUnavailable,
                format!(
                    "sourceSet does not contain the {tag} prerequisite required by {}",
                    requested.as_str()
                ),
            )
            .into()
        })
}

fn registered_names(owner: &[u8], tag: &str) -> Vec<String> {
    let Ok(xml) = std::str::from_utf8(owner) else {
        return Vec::new();
    };
    let Ok(document) = Document::parse(xml.trim_start_matches('\u{feff}')) else {
        return Vec::new();
    };
    document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == tag)
        .filter_map(|node| node.text().map(str::trim))
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn read_dependency(
    source: &ResolvedSourceSet,
    tag: &str,
    name: &str,
    mode: MetadataTemplateFileMode,
    replacement: Option<Vec<u8>>,
) -> Result<MetadataTemplateFile, MetaFailure> {
    let layout = crate::infrastructure::metadata_kinds::metadata_kind(tag).ok_or_else(|| {
        MetaFailure::from(MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata prerequisite layout is unavailable",
        ))
    })?;
    let relative_path = PathBuf::from(layout.directory).join(format!("{name}.xml"));
    let preimage = std::fs::read(source.source_root.join(&relative_path)).map_err(|_| {
        MetaFailure::from(MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            format!("metadata prerequisite `{tag}.{name}` image is unavailable"),
        ))
    })?;
    let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &format!("{tag}.{name}"))
        .map_err(|_| {
            MetaFailure::from(MetaDiagnostic::error(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata prerequisite identity is invalid",
            ))
        })?;
    Ok(MetadataTemplateFile {
        role: MetadataTemplateFileRole::Dependency(target),
        mode,
        relative_path,
        bytes: replacement.unwrap_or_else(|| preimage.clone()),
        preimage: Some(preimage),
    })
}

fn registrar_dependency(
    source: &ResolvedSourceSet,
    register: &str,
    requested: MetadataKind,
) -> Result<MetadataTemplateFile, MetaFailure> {
    let document = required_registered_name(source, "Document", requested)?;
    let existing = read_dependency(
        source,
        "Document",
        &document,
        MetadataTemplateFileMode::Guard,
        None,
    )?;
    let replacement = add_document_register_record(&existing.bytes, register).map_err(|_| {
        MetaFailure::from(MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            "registrar document image cannot accept the mandatory register relation",
        ))
    })?;
    read_dependency(
        source,
        "Document",
        &document,
        MetadataTemplateFileMode::Replace,
        Some(replacement),
    )
}

fn add_document_register_record(bytes: &[u8], register: &str) -> Result<Vec<u8>, String> {
    let has_bom = bytes.starts_with(b"\xef\xbb\xbf");
    let source = std::str::from_utf8(bytes)
        .map_err(|_| "document is not UTF-8".to_string())?
        .trim_start_matches('\u{feff}');
    let escaped = escape_xml(register);
    let item = format!("<xr:Item xsi:type=\"xr:MDObjectRef\">{escaped}</xr:Item>");
    let updated = if source.contains("<RegisterRecords/>") {
        source.replacen(
            "<RegisterRecords/>",
            &format!("<RegisterRecords>{item}</RegisterRecords>"),
            1,
        )
    } else if let Some(end) = source.find("</RegisterRecords>") {
        let mut updated = source.to_string();
        updated.insert_str(end, &item);
        updated
    } else {
        return Err("document has no RegisterRecords property".to_string());
    };
    let mut output = Vec::new();
    if has_bom {
        output.extend_from_slice(b"\xef\xbb\xbf");
    }
    output.extend_from_slice(updated.as_bytes());
    Ok(output)
}

impl MetadataTemplateCatalog for PlatformMetadataTemplateCatalog {
    fn minimal_object(
        &self,
        source: &ResolvedSourceSet,
        kind: MetadataKind,
        name: &str,
    ) -> Result<MetadataPostImage, MetaFailure> {
        if !is_1c_identifier(name) {
            return Err(MetaDiagnostic::error(
                MetaDiagnosticCode::InvalidArguments,
                format!("metadata name `{name}` is not a valid 1C identifier"),
            )
            .with_field("name")
            .into());
        }
        let metadata_path = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("{}.{name}", kind.as_str()),
        )
        .map_err(|_| {
            MetaFailure::from(
                MetaDiagnostic::error(
                    MetaDiagnosticCode::InvalidArguments,
                    "metadata creation target is not a canonical logical address",
                )
                .with_field("name"),
            )
        })?;
        let context = MinimalTemplateContext::from_source(source, kind, name)?;
        let (xml, _) = minimal_metadata_xml(kind, name, &source.format_version, &context)
            .map_err(|message| template_failure(&metadata_path, message))?;
        let layout = metadata_layout(kind);
        let mut files = vec![MetadataTemplateFile {
            role: MetadataTemplateFileRole::Descriptor,
            mode: MetadataTemplateFileMode::Create,
            relative_path: PathBuf::from(layout.directory).join(format!("{name}.xml")),
            bytes: with_utf8_bom(xml.as_bytes()),
            preimage: None,
        }];
        let ext = PathBuf::from(layout.directory).join(name).join("Ext");
        for module in minimal_module_files(kind) {
            files.push(MetadataTemplateFile {
                role: MetadataTemplateFileRole::Module,
                mode: MetadataTemplateFileMode::Create,
                relative_path: ext.join(module),
                bytes: with_utf8_bom(&[]),
                preimage: None,
            });
        }
        for (file_name, auxiliary_kind, content) in
            minimal_auxiliary_files(kind, &source.format_version)
        {
            files.push(MetadataTemplateFile {
                role: MetadataTemplateFileRole::AuxiliaryXml(auxiliary_kind),
                mode: MetadataTemplateFileMode::Create,
                relative_path: ext.join(file_name),
                bytes: with_utf8_bom(content.as_bytes()),
                preimage: None,
            });
        }
        files.extend(context.dependencies);
        Ok(MetadataPostImage {
            metadata_path,
            files,
        })
    }
}

fn template_failure(target: &MetadataAddress, message: String) -> MetaFailure {
    MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, message)
        .with_metadata_path(target.clone())
        .into()
}

fn with_utf8_bom(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(3 + bytes.len());
    output.extend_from_slice(b"\xef\xbb\xbf");
    output.extend_from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes));
    output
}

pub(crate) fn minimal_module_files(kind: MetadataKind) -> &'static [&'static str] {
    match kind {
        MetadataKind::Catalog
        | MetadataKind::Document
        | MetadataKind::ChartOfAccounts
        | MetadataKind::ChartOfCharacteristicTypes
        | MetadataKind::ChartOfCalculationTypes
        | MetadataKind::BusinessProcess
        | MetadataKind::Task
        | MetadataKind::ExchangePlan => &["ObjectModule.bsl"],
        MetadataKind::Enum => &["ManagerModule.bsl"],
        MetadataKind::Constant => &["ManagerModule.bsl", "ValueManagerModule.bsl"],
        MetadataKind::InformationRegister
        | MetadataKind::AccumulationRegister
        | MetadataKind::AccountingRegister
        | MetadataKind::CalculationRegister => &["RecordSetModule.bsl"],
        MetadataKind::Report | MetadataKind::DataProcessor => {
            &["ObjectModule.bsl", "ManagerModule.bsl"]
        }
        MetadataKind::CommonModule | MetadataKind::HTTPService | MetadataKind::WebService => {
            &["Module.bsl"]
        }
        MetadataKind::ScheduledJob
        | MetadataKind::EventSubscription
        | MetadataKind::DocumentJournal
        | MetadataKind::DefinedType => &[],
    }
}

pub(crate) fn minimal_auxiliary_files(
    kind: MetadataKind,
    format_version: &str,
) -> Vec<(&'static str, MetadataAuxiliaryXmlKind, String)> {
    match kind {
        MetadataKind::ExchangePlan => vec![(
            "Content.xml",
            MetadataAuxiliaryXmlKind::ExchangePlanContent,
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<ExchangePlanContent xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"{format_version}\"/>\r\n"
            ),
        )],
        MetadataKind::BusinessProcess => vec![(
            "Flowchart.xml",
            MetadataAuxiliaryXmlKind::BusinessProcessFlowchart,
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

pub(super) enum MetaTemplateDefinition<'a> {
    Legacy(&'a Map<String, Value>),
    Minimal(MinimalTemplateValues),
}

pub(super) struct MinimalTemplateValues {
    chart_of_accounts: Option<String>,
    chart_of_calculation_types: Option<String>,
    method_name: Option<String>,
    sources: Vec<String>,
    handler: Option<String>,
}

impl<'a> MetaTemplateDefinition<'a> {
    pub(super) fn legacy(value: &'a Map<String, Value>) -> Self {
        Self::Legacy(value)
    }

    fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Legacy(value) => value.get(key),
            Self::Minimal(_) => None,
        }
    }

    fn string(&self, key: &str) -> Option<&str> {
        match self {
            Self::Legacy(value) => value.get(key).and_then(Value::as_str),
            Self::Minimal(value) => match key {
                "chartOfAccounts" => value.chart_of_accounts.as_deref(),
                "chartOfCalculationTypes" => value.chart_of_calculation_types.as_deref(),
                "methodName" => value.method_name.as_deref(),
                "handler" => value.handler.as_deref(),
                _ => None,
            },
        }
    }

    fn string_list(&self, key: &str) -> Vec<String> {
        match self {
            Self::Legacy(value) => meta_compile_string_list(value.get(key)),
            Self::Minimal(value) if key == "source" => value.sources.clone(),
            Self::Minimal(_) => Vec::new(),
        }
    }
}

pub(super) fn meta_compile_catalog_xml(
    defn: &Map<String, Value>,
    obj_name: &str,
    format_version: &str,
) -> Result<(String, String), String> {
    emit_meta_catalog_xml(
        &MetaTemplateDefinition::legacy(defn),
        obj_name,
        format_version,
    )
}

fn emit_meta_catalog_xml(
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    format_version: &str,
) -> Result<(String, String), String> {
    let mut next_uuid = fresh_meta_compile_uuid;
    let obj_uuid = next_uuid();
    let synonym = defn
        .get("synonym")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| split_meta_camel_case(obj_name));

    let mut lines = Vec::<String>::new();
    lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
    lines.push(format!(
        "<MetaDataObject {} version=\"{}\">",
        meta_xmlns_decl(),
        escape_xml(format_version)
    ));
    lines.push(format!("\t<Catalog uuid=\"{obj_uuid}\">"));
    emit_meta_internal_info(&mut lines, "\t\t", "Catalog", obj_name, &mut next_uuid);
    lines.push("\t\t<Properties>".to_string());
    emit_meta_catalog_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym);
    lines.push("\t\t</Properties>".to_string());

    let attrs = meta_compile_attributes(defn.get("attributes"));
    let tabular_sections = meta_compile_tabular_sections(defn.get("tabularSections"))?;
    if attrs.is_empty() && tabular_sections.is_empty() {
        lines.push("\t\t<ChildObjects/>".to_string());
    } else {
        lines.push("\t\t<ChildObjects>".to_string());
        for attr in &attrs {
            emit_meta_attribute(&mut lines, "\t\t\t", attr, "catalog", &mut next_uuid);
        }
        for section in &tabular_sections {
            emit_meta_tabular_section(
                &mut lines,
                "\t\t\t",
                section,
                "Catalog",
                obj_name,
                &mut next_uuid,
            );
        }
        lines.push("\t\t</ChildObjects>".to_string());
    }

    lines.push("\t</Catalog>".to_string());
    lines.push("</MetaDataObject>".to_string());
    Ok((format!("{}\n", lines.join("\n")), obj_uuid))
}

fn minimal_metadata_xml(
    kind: crate::domain::metadata::MetadataKind,
    obj_name: &str,
    format_version: &str,
    context: &MinimalTemplateContext,
) -> Result<(String, String), String> {
    let defn = MetaTemplateDefinition::Minimal(MinimalTemplateValues {
        chart_of_accounts: context.chart_of_accounts.clone(),
        chart_of_calculation_types: context.chart_of_calculation_types.clone(),
        method_name: context.method_name.clone(),
        sources: context.event_source.clone().into_iter().collect(),
        handler: context.event_handler.clone(),
    });
    if kind == crate::domain::metadata::MetadataKind::Catalog {
        return emit_meta_catalog_xml(&defn, obj_name, format_version);
    }

    let object_type = kind.as_str();
    let mut next_uuid = fresh_meta_compile_uuid;
    let obj_uuid = next_uuid();
    let synonym = split_meta_camel_case(obj_name);
    let mut lines = vec![
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string(),
        format!(
            "<MetaDataObject {} version=\"{}\">",
            meta_xmlns_decl(),
            escape_xml(format_version)
        ),
        format!("\t<{object_type} uuid=\"{obj_uuid}\">"),
    ];
    emit_meta_internal_info(&mut lines, "\t\t", object_type, obj_name, &mut next_uuid);
    lines.push("\t\t<Properties>".to_string());
    match kind {
        crate::domain::metadata::MetadataKind::Catalog => unreachable!(),
        crate::domain::metadata::MetadataKind::Document => {
            emit_meta_document_properties(&mut lines, "\t\t\t", &defn, obj_name, &synonym)
        }
        crate::domain::metadata::MetadataKind::Enum => {
            emit_meta_enum_properties(&mut lines, "\t\t\t", &defn, obj_name, &synonym)
        }
        crate::domain::metadata::MetadataKind::Constant => {
            emit_meta_constant_properties(&mut lines, "\t\t\t", &defn, obj_name, &synonym)
        }
        crate::domain::metadata::MetadataKind::InformationRegister => {
            emit_meta_information_register_properties(
                &mut lines, "\t\t\t", &defn, obj_name, &synonym,
            )
        }
        crate::domain::metadata::MetadataKind::AccumulationRegister => {
            emit_meta_accumulation_register_properties(
                &mut lines, "\t\t\t", &defn, obj_name, &synonym,
            )
        }
        crate::domain::metadata::MetadataKind::AccountingRegister => {
            emit_meta_accounting_register_properties(
                &mut lines, "\t\t\t", &defn, obj_name, &synonym,
            )
        }
        crate::domain::metadata::MetadataKind::CalculationRegister => {
            emit_meta_calculation_register_properties(
                &mut lines, "\t\t\t", &defn, obj_name, &synonym,
            )
        }
        crate::domain::metadata::MetadataKind::ChartOfAccounts => {
            emit_meta_chart_of_accounts_properties(&mut lines, "\t\t\t", &defn, obj_name, &synonym)
        }
        crate::domain::metadata::MetadataKind::ChartOfCharacteristicTypes => {
            emit_meta_chart_of_characteristic_types_properties(
                &mut lines, "\t\t\t", &defn, obj_name, &synonym,
            )
        }
        crate::domain::metadata::MetadataKind::ChartOfCalculationTypes => {
            emit_meta_chart_of_calculation_types_properties(
                &mut lines, "\t\t\t", &defn, obj_name, &synonym,
            )
        }
        crate::domain::metadata::MetadataKind::BusinessProcess => {
            emit_meta_business_process_properties(&mut lines, "\t\t\t", &defn, obj_name, &synonym)
        }
        crate::domain::metadata::MetadataKind::Task => {
            emit_meta_task_properties(&mut lines, "\t\t\t", &defn, obj_name, &synonym)
        }
        crate::domain::metadata::MetadataKind::ExchangePlan => {
            emit_meta_exchange_plan_properties(&mut lines, "\t\t\t", &defn, obj_name, &synonym)
        }
        crate::domain::metadata::MetadataKind::DocumentJournal => {
            emit_meta_document_journal_properties(&mut lines, "\t\t\t", &defn, obj_name, &synonym)
        }
        crate::domain::metadata::MetadataKind::Report => {
            emit_meta_report_properties(&mut lines, "\t\t\t", &defn, obj_name, &synonym)
        }
        crate::domain::metadata::MetadataKind::DataProcessor => {
            emit_meta_data_processor_properties(&mut lines, "\t\t\t", &defn, obj_name, &synonym)
        }
        crate::domain::metadata::MetadataKind::CommonModule => {
            emit_meta_common_module_properties(&mut lines, "\t\t\t", &defn, obj_name, &synonym)
        }
        crate::domain::metadata::MetadataKind::ScheduledJob => {
            emit_meta_scheduled_job_properties(&mut lines, "\t\t\t", &defn, obj_name, &synonym)
        }
        crate::domain::metadata::MetadataKind::EventSubscription => {
            emit_meta_event_subscription_properties(&mut lines, "\t\t\t", &defn, obj_name, &synonym)
        }
        crate::domain::metadata::MetadataKind::HTTPService => {
            emit_meta_http_service_properties(&mut lines, "\t\t\t", &defn, obj_name, &synonym)
        }
        crate::domain::metadata::MetadataKind::WebService => {
            emit_meta_web_service_properties(&mut lines, "\t\t\t", &defn, obj_name, &synonym)
        }
        crate::domain::metadata::MetadataKind::DefinedType => {
            emit_meta_defined_type_properties(&mut lines, "\t\t\t", &defn, obj_name, &synonym)
        }
    }
    lines.push("\t\t</Properties>".to_string());
    if matches!(
        kind,
        crate::domain::metadata::MetadataKind::AccountingRegister
            | crate::domain::metadata::MetadataKind::CalculationRegister
    ) {
        lines.push("\t\t<ChildObjects>".to_string());
        let field = MetaCompileAttr {
            name: "Value".to_string(),
            type_name: "Number(15,2)".to_string(),
            synonym: "Value".to_string(),
            flags: Vec::new(),
            fill_checking: String::new(),
            indexing: String::new(),
            multi_line: false,
            choice_history_on_input: String::new(),
        };
        emit_meta_register_field(
            &mut lines,
            "\t\t\t",
            "Resource",
            &field,
            kind.as_str(),
            &mut next_uuid,
        );
        lines.push("\t\t</ChildObjects>".to_string());
    } else {
        lines.push("\t\t<ChildObjects/>".to_string());
    }
    lines.push(format!("\t</{object_type}>"));
    lines.push("</MetaDataObject>".to_string());
    Ok((format!("{}\n", lines.join("\n")), obj_uuid))
}

pub(super) fn meta_xmlns_decl() -> &'static str {
    "xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\" xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\" xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\" xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\""
}

pub(crate) fn metadata_generated_types_8_3_27(
    object_type: &str,
) -> Option<&'static [(&'static str, &'static str)]> {
    match object_type {
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
        "BusinessProcess" => Some(&[
            ("BusinessProcessObject", "Object"),
            ("BusinessProcessRef", "Ref"),
            ("BusinessProcessSelection", "Selection"),
            ("BusinessProcessList", "List"),
            ("BusinessProcessManager", "Manager"),
            ("BusinessProcessRoutePointRef", "RoutePointRef"),
        ]),
        "Enum" => Some(&[
            ("EnumRef", "Ref"),
            ("EnumManager", "Manager"),
            ("EnumList", "List"),
        ]),
        "Constant" => Some(&[
            ("ConstantManager", "Manager"),
            ("ConstantValueManager", "ValueManager"),
            ("ConstantValueKey", "ValueKey"),
        ]),
        "InformationRegister" => Some(&[
            ("InformationRegisterRecord", "Record"),
            ("InformationRegisterManager", "Manager"),
            ("InformationRegisterSelection", "Selection"),
            ("InformationRegisterList", "List"),
            ("InformationRegisterRecordSet", "RecordSet"),
            ("InformationRegisterRecordKey", "RecordKey"),
            ("InformationRegisterRecordManager", "RecordManager"),
        ]),
        "AccumulationRegister" => Some(&[
            ("AccumulationRegisterRecord", "Record"),
            ("AccumulationRegisterManager", "Manager"),
            ("AccumulationRegisterSelection", "Selection"),
            ("AccumulationRegisterList", "List"),
            ("AccumulationRegisterRecordSet", "RecordSet"),
            ("AccumulationRegisterRecordKey", "RecordKey"),
        ]),
        "AccountingRegister" => Some(&[
            ("AccountingRegisterRecord", "Record"),
            ("AccountingRegisterExtDimensions", "ExtDimensions"),
            ("AccountingRegisterRecordSet", "RecordSet"),
            ("AccountingRegisterRecordKey", "RecordKey"),
            ("AccountingRegisterSelection", "Selection"),
            ("AccountingRegisterList", "List"),
            ("AccountingRegisterManager", "Manager"),
        ]),
        "CalculationRegister" => Some(&[
            ("CalculationRegisterRecord", "Record"),
            ("CalculationRegisterManager", "Manager"),
            ("CalculationRegisterSelection", "Selection"),
            ("CalculationRegisterList", "List"),
            ("CalculationRegisterRecordSet", "RecordSet"),
            ("CalculationRegisterRecordKey", "RecordKey"),
            ("RecalculationsManager", "Recalcs"),
        ]),
        "ChartOfAccounts" => Some(&[
            ("ChartOfAccountsObject", "Object"),
            ("ChartOfAccountsRef", "Ref"),
            ("ChartOfAccountsSelection", "Selection"),
            ("ChartOfAccountsList", "List"),
            ("ChartOfAccountsManager", "Manager"),
            ("ChartOfAccountsExtDimensionTypes", "ExtDimensionTypes"),
            (
                "ChartOfAccountsExtDimensionTypesRow",
                "ExtDimensionTypesRow",
            ),
        ]),
        "ChartOfCharacteristicTypes" => Some(&[
            ("ChartOfCharacteristicTypesObject", "Object"),
            ("ChartOfCharacteristicTypesRef", "Ref"),
            ("ChartOfCharacteristicTypesSelection", "Selection"),
            ("ChartOfCharacteristicTypesList", "List"),
            ("Characteristic", "Characteristic"),
            ("ChartOfCharacteristicTypesManager", "Manager"),
        ]),
        "ChartOfCalculationTypes" => Some(&[
            ("ChartOfCalculationTypesObject", "Object"),
            ("ChartOfCalculationTypesRef", "Ref"),
            ("ChartOfCalculationTypesSelection", "Selection"),
            ("ChartOfCalculationTypesList", "List"),
            ("ChartOfCalculationTypesManager", "Manager"),
            ("DisplacingCalculationTypes", "DisplacingCalculationTypes"),
            (
                "DisplacingCalculationTypesRow",
                "DisplacingCalculationTypesRow",
            ),
            ("BaseCalculationTypes", "BaseCalculationTypes"),
            ("BaseCalculationTypesRow", "BaseCalculationTypesRow"),
            ("LeadingCalculationTypes", "LeadingCalculationTypes"),
            ("LeadingCalculationTypesRow", "LeadingCalculationTypesRow"),
        ]),
        "Report" => Some(&[("ReportObject", "Object"), ("ReportManager", "Manager")]),
        "DataProcessor" => Some(&[
            ("DataProcessorObject", "Object"),
            ("DataProcessorManager", "Manager"),
        ]),
        "Task" => Some(&[
            ("TaskObject", "Object"),
            ("TaskRef", "Ref"),
            ("TaskSelection", "Selection"),
            ("TaskList", "List"),
            ("TaskManager", "Manager"),
        ]),
        "ExchangePlan" => Some(&[
            ("ExchangePlanObject", "Object"),
            ("ExchangePlanRef", "Ref"),
            ("ExchangePlanSelection", "Selection"),
            ("ExchangePlanList", "List"),
            ("ExchangePlanManager", "Manager"),
        ]),
        "DocumentJournal" => Some(&[
            ("DocumentJournalSelection", "Selection"),
            ("DocumentJournalList", "List"),
            ("DocumentJournalManager", "Manager"),
        ]),
        "FilterCriterion" => Some(&[
            ("FilterCriterionManager", "Manager"),
            ("FilterCriterionList", "List"),
        ]),
        "SettingsStorage" => Some(&[("SettingsStorageManager", "Manager")]),
        "Sequence" => Some(&[("SequenceRecordSet", "RecordSet")]),
        "IntegrationService" => Some(&[("IntegrationServiceManager", "Manager")]),
        "DefinedType" => Some(&[("DefinedType", "DefinedType")]),
        "Language"
        | "Subsystem"
        | "StyleItem"
        | "Style"
        | "CommonPicture"
        | "SessionParameter"
        | "Role"
        | "CommonTemplate"
        | "CommonModule"
        | "Bot"
        | "CommonAttribute"
        | "XDTOPackage"
        | "WebService"
        | "HTTPService"
        | "WSReference"
        | "EventSubscription"
        | "ScheduledJob"
        | "FunctionalOption"
        | "FunctionalOptionsParameter"
        | "CommonCommand"
        | "CommandGroup"
        | "CommonForm"
        | "DocumentNumerator" => Some(&[]),
        _ => None,
    }
}

pub(crate) fn emit_meta_internal_info<F>(
    lines: &mut Vec<String>,
    indent: &str,
    object_type: &str,
    object_name: &str,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    let Some(generated) = metadata_generated_types_8_3_27(object_type) else {
        return;
    };
    if generated.is_empty() {
        return;
    }
    lines.push(format!("{indent}<InternalInfo>"));
    if object_type == "ExchangePlan" {
        lines.push(format!(
            "{indent}\t<xr:ThisNode>{}</xr:ThisNode>",
            next_uuid()
        ));
    }
    for (prefix, category) in generated {
        let generated_name = escape_xml(&format!("{prefix}.{object_name}"));
        lines.push(format!(
            "{indent}\t<xr:GeneratedType name=\"{generated_name}\" category=\"{}\">",
            escape_xml(category)
        ));
        lines.push(format!(
            "{indent}\t\t<xr:TypeId>{}</xr:TypeId>",
            next_uuid()
        ));
        lines.push(format!(
            "{indent}\t\t<xr:ValueId>{}</xr:ValueId>",
            next_uuid()
        ));
        lines.push(format!("{indent}\t</xr:GeneratedType>"));
    }
    lines.push(format!("{indent}</InternalInfo>"));
}

pub(super) fn emit_meta_catalog_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    let hierarchical = defn.get("hierarchical").and_then(Value::as_bool) == Some(true);
    lines.push(format!(
        "{indent}<Hierarchical>{hierarchical}</Hierarchical>"
    ));
    lines.push(format!(
        "{indent}<HierarchyType>{}</HierarchyType>",
        meta_enum_prop(defn, "hierarchyType", "HierarchyFoldersAndItems")
    ));
    let limit_level_count = defn.get("limitLevelCount").and_then(Value::as_bool) == Some(true);
    let level_count = defn.get("levelCount").and_then(json_i64_value).unwrap_or(2);
    let folders_on_top = defn.get("foldersOnTop").and_then(Value::as_bool) != Some(false);
    lines.push(format!(
        "{indent}<LimitLevelCount>{limit_level_count}</LimitLevelCount>"
    ));
    lines.push(format!("{indent}<LevelCount>{level_count}</LevelCount>"));
    lines.push(format!(
        "{indent}<FoldersOnTop>{folders_on_top}</FoldersOnTop>"
    ));
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!("{indent}<Owners/>"));
    lines.push(format!(
        "{indent}<SubordinationUse>{}</SubordinationUse>",
        meta_enum_prop(defn, "subordinationUse", "ToItems")
    ));
    let code_length = defn.get("codeLength").and_then(json_i64_value).unwrap_or(9);
    let description_length = defn
        .get("descriptionLength")
        .and_then(json_i64_value)
        .unwrap_or(25);
    lines.push(format!("{indent}<CodeLength>{code_length}</CodeLength>"));
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    lines.push(format!(
        "{indent}<CodeType>{}</CodeType>",
        meta_enum_prop(defn, "codeType", "String")
    ));
    lines.push(format!(
        "{indent}<CodeAllowedLength>{}</CodeAllowedLength>",
        meta_enum_prop(defn, "codeAllowedLength", "Variable")
    ));
    lines.push(format!(
        "{indent}<CodeSeries>{}</CodeSeries>",
        meta_enum_prop(defn, "codeSeries", "WholeCatalog")
    ));
    let check_unique = defn.get("checkUnique").and_then(Value::as_bool) == Some(true);
    let autonumbering = defn.get("autonumbering").and_then(Value::as_bool) != Some(false);
    lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    lines.push(format!(
        "{indent}<Autonumbering>{autonumbering}</Autonumbering>"
    ));
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        meta_enum_prop(defn, "defaultPresentation", "AsDescription")
    ));
    emit_meta_standard_attributes(lines, indent, "Catalog");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!(
        "{indent}<PredefinedDataUpdate>Auto</PredefinedDataUpdate>"
    ));
    lines.push(format!("{indent}<EditType>InDialog</EditType>"));
    let quick_choice = defn.get("quickChoice").and_then(Value::as_bool) == Some(true);
    lines.push(format!("{indent}<QuickChoice>{quick_choice}</QuickChoice>"));
    lines.push(format!(
        "{indent}<ChoiceMode>{}</ChoiceMode>",
        meta_enum_prop(defn, "choiceMode", "BothWays")
    ));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!("Catalog.{obj_name}.StandardAttribute.Description"))
    ));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!("Catalog.{obj_name}.StandardAttribute.Code"))
    ));
    lines.push(format!("{indent}</InputByString>"));
    lines.push(format!(
        "{indent}<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>"
    ));
    lines.push(format!(
        "{indent}<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>"
    ));
    lines.push(format!(
        "{indent}<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>"
    ));
    for tag in [
        "DefaultObjectForm",
        "DefaultFolderForm",
        "DefaultListForm",
        "DefaultChoiceForm",
        "DefaultFolderChoiceForm",
        "AuxiliaryObjectForm",
        "AuxiliaryFolderForm",
        "AuxiliaryListForm",
        "AuxiliaryChoiceForm",
        "AuxiliaryFolderChoiceForm",
    ] {
        lines.push(format!("{indent}<{tag}/>"));
    }
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    for line in [
        "<BasedOn/>",
        "<DataLockFields/>",
        "<DataLockControlMode>Automatic</DataLockControlMode>",
        "<FullTextSearch>Use</FullTextSearch>",
        "<ObjectPresentation/>",
        "<ExtendedObjectPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(super) fn meta_compile_synonym(defn: &MetaTemplateDefinition<'_>, obj_name: &str) -> String {
    defn.get("synonym")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| split_meta_camel_case(obj_name))
}

pub(super) fn emit_meta_base_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    lines.push(format!("{indent}<Name>{}</Name>", escape_xml(obj_name)));
    emit_meta_mltext(lines, indent, "Synonym", synonym);
    match defn.get("comment").and_then(Value::as_str) {
        Some(comment) if !comment.is_empty() => {
            lines.push(format!(
                "{indent}<Comment>{}</Comment>",
                escape_xml(comment)
            ));
        }
        _ => lines.push(format!("{indent}<Comment/>")),
    }
}

pub(super) fn emit_meta_enum_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>false</UseStandardCommands>"
    ));
    emit_meta_standard_attributes(lines, indent, "Enum");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!("{indent}<QuickChoice>false</QuickChoice>"));
    lines.push(format!("{indent}<ChoiceMode>BothWays</ChoiceMode>"));
    for tag in [
        "DefaultListForm",
        "DefaultChoiceForm",
        "AuxiliaryListForm",
        "AuxiliaryChoiceForm",
    ] {
        lines.push(format!("{indent}<{tag}/>"));
    }
    lines.push(format!("{indent}<ListPresentation/>"));
    lines.push(format!("{indent}<ExtendedListPresentation/>"));
    lines.push(format!("{indent}<Explanation/>"));
    lines.push(format!(
        "{indent}<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>"
    ));
}

pub(super) fn emit_meta_constant_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    let value_type = meta_compile_root_value_type(defn);
    emit_meta_value_type(lines, indent, &value_type);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    for tag in ["DefaultForm", "ExtendedPresentation", "Explanation"] {
        lines.push(format!("{indent}<{tag}/>"));
    }
    for line in [
        "<PasswordMode>false</PasswordMode>",
        "<Format/>",
        "<EditFormat/>",
        "<ToolTip/>",
        "<MarkNegatives>false</MarkNegatives>",
        "<Mask/>",
        "<MultiLine>false</MultiLine>",
        "<ExtendedEdit>false</ExtendedEdit>",
        "<MinValue xsi:nil=\"true\"/>",
        "<MaxValue xsi:nil=\"true\"/>",
        "<FillChecking>DontCheck</FillChecking>",
        "<ChoiceFoldersAndItems>Items</ChoiceFoldersAndItems>",
        "<ChoiceParameterLinks/>",
        "<ChoiceParameters/>",
        "<QuickChoice>Auto</QuickChoice>",
        "<ChoiceForm/>",
        "<LinkByType/>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    for line in [
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(super) fn emit_meta_document_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!("{indent}<Numerator/>"));
    lines.push(format!(
        "{indent}<NumberType>{}</NumberType>",
        meta_enum_prop(defn, "numberType", "String")
    ));
    let number_length = defn
        .get("numberLength")
        .and_then(json_i64_value)
        .unwrap_or(11);
    lines.push(format!(
        "{indent}<NumberLength>{number_length}</NumberLength>"
    ));
    lines.push(format!(
        "{indent}<NumberAllowedLength>{}</NumberAllowedLength>",
        meta_enum_prop(defn, "numberAllowedLength", "Variable")
    ));
    lines.push(format!(
        "{indent}<NumberPeriodicity>{}</NumberPeriodicity>",
        meta_enum_prop(defn, "numberPeriodicity", "Year")
    ));
    let check_unique = defn.get("checkUnique").and_then(Value::as_bool) != Some(false);
    let autonumbering = defn.get("autonumbering").and_then(Value::as_bool) != Some(false);
    lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    lines.push(format!(
        "{indent}<Autonumbering>{autonumbering}</Autonumbering>"
    ));
    emit_meta_standard_attributes(lines, indent, "Document");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!("{indent}<BasedOn/>"));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!("Document.{obj_name}.StandardAttribute.Number"))
    ));
    lines.push(format!("{indent}</InputByString>"));
    for line in [
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>",
        "<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>",
        "<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>",
        "<DefaultObjectForm/>",
        "<DefaultListForm/>",
        "<DefaultChoiceForm/>",
        "<AuxiliaryObjectForm/>",
        "<AuxiliaryListForm/>",
        "<AuxiliaryChoiceForm/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    lines.push(format!(
        "{indent}<Posting>{}</Posting>",
        meta_enum_prop(defn, "posting", "Allow")
    ));
    lines.push(format!(
        "{indent}<RealTimePosting>{}</RealTimePosting>",
        meta_enum_prop(defn, "realTimePosting", "Deny")
    ));
    lines.push(format!(
        "{indent}<RegisterRecordsDeletion>{}</RegisterRecordsDeletion>",
        meta_enum_prop(defn, "registerRecordsDeletion", "AutoDelete")
    ));
    lines.push(format!(
        "{indent}<RegisterRecordsWritingOnPost>{}</RegisterRecordsWritingOnPost>",
        meta_enum_prop(defn, "registerRecordsWritingOnPost", "WriteModified")
    ));
    lines.push(format!(
        "{indent}<SequenceFilling>{}</SequenceFilling>",
        escape_xml(
            defn.get("sequenceFilling")
                .and_then(Value::as_str)
                .unwrap_or("AutoFill")
        )
    ));
    emit_meta_md_object_refs(
        lines,
        indent,
        "RegisterRecords",
        &meta_compile_string_list(defn.get("registerRecords")),
    );
    let post_in_privileged =
        defn.get("postInPrivilegedMode").and_then(Value::as_bool) != Some(false);
    let unpost_in_privileged =
        defn.get("unpostInPrivilegedMode").and_then(Value::as_bool) != Some(false);
    lines.push(format!(
        "{indent}<PostInPrivilegedMode>{post_in_privileged}</PostInPrivilegedMode>"
    ));
    lines.push(format!(
        "{indent}<UnpostInPrivilegedMode>{unpost_in_privileged}</UnpostInPrivilegedMode>"
    ));
    emit_meta_lock_search_presentation_tail(lines, indent, "Use");
}

pub(super) fn emit_meta_information_register_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    let _ = obj_name;
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!("{indent}<EditType>InDialog</EditType>"));
    for tag in [
        "DefaultRecordForm",
        "DefaultListForm",
        "AuxiliaryRecordForm",
        "AuxiliaryListForm",
    ] {
        lines.push(format!("{indent}<{tag}/>"));
    }
    emit_meta_standard_attributes(lines, indent, "InformationRegister");
    let periodicity = meta_enum_prop(defn, "periodicity", "Nonperiodical");
    let write_mode = meta_enum_prop(defn, "writeMode", "Independent");
    let main_filter_on_period = defn
        .get("mainFilterOnPeriod")
        .and_then(Value::as_bool)
        .unwrap_or(periodicity != "Nonperiodical");
    lines.push(format!(
        "{indent}<InformationRegisterPeriodicity>{periodicity}</InformationRegisterPeriodicity>"
    ));
    lines.push(format!("{indent}<WriteMode>{write_mode}</WriteMode>"));
    lines.push(format!(
        "{indent}<MainFilterOnPeriod>{main_filter_on_period}</MainFilterOnPeriod>"
    ));
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        meta_enum_prop(defn, "fullTextSearch", "Use")
    ));
    for line in [
        "<EnableTotalsSliceFirst>false</EnableTotalsSliceFirst>",
        "<EnableTotalsSliceLast>false</EnableTotalsSliceLast>",
        "<RecordPresentation/>",
        "<ExtendedRecordPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(super) fn emit_meta_accumulation_register_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!("{indent}<DefaultListForm/>"));
    lines.push(format!("{indent}<AuxiliaryListForm/>"));
    lines.push(format!(
        "{indent}<RegisterType>{}</RegisterType>",
        meta_enum_prop(defn, "registerType", "Balance")
    ));
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    emit_meta_standard_attributes(lines, indent, "AccumulationRegister");
    emit_meta_register_tail(lines, indent, defn);
    let enable_totals_splitting =
        defn.get("enableTotalsSplitting").and_then(Value::as_bool) != Some(false);
    lines.push(format!(
        "{indent}<EnableTotalsSplitting>{enable_totals_splitting}</EnableTotalsSplitting>"
    ));
    lines.push(format!("{indent}<ListPresentation/>"));
    lines.push(format!("{indent}<ExtendedListPresentation/>"));
    lines.push(format!("{indent}<Explanation/>"));
}

pub(super) fn emit_meta_accounting_register_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    emit_meta_optional_text(
        lines,
        indent,
        "ChartOfAccounts",
        defn.string("chartOfAccounts"),
    );
    let correspondence = defn.get("correspondence").and_then(Value::as_bool) == Some(true);
    let period_adjustment_length = defn
        .get("periodAdjustmentLength")
        .and_then(json_i64_value)
        .unwrap_or(0);
    lines.push(format!(
        "{indent}<Correspondence>{correspondence}</Correspondence>"
    ));
    lines.push(format!(
        "{indent}<PeriodAdjustmentLength>{period_adjustment_length}</PeriodAdjustmentLength>"
    ));
    lines.push(format!("{indent}<DefaultListForm/>"));
    lines.push(format!("{indent}<AuxiliaryListForm/>"));
    emit_meta_standard_attributes(lines, indent, "AccountingRegister");
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    lines.push(format!(
        "{indent}<EnableTotalsSplitting>false</EnableTotalsSplitting>"
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        meta_enum_prop(defn, "fullTextSearch", "Use")
    ));
    lines.push(format!("{indent}<ListPresentation/>"));
    lines.push(format!("{indent}<ExtendedListPresentation/>"));
    lines.push(format!("{indent}<Explanation/>"));
}

pub(super) fn emit_meta_calculation_register_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!("{indent}<DefaultListForm/>"));
    lines.push(format!("{indent}<AuxiliaryListForm/>"));
    lines.push(format!(
        "{indent}<Periodicity>{}</Periodicity>",
        meta_enum_prop(defn, "periodicity", "Month")
    ));
    let action_period = defn.get("actionPeriod").and_then(Value::as_bool) == Some(true);
    let base_period = defn.get("basePeriod").and_then(Value::as_bool) == Some(true);
    lines.push(format!(
        "{indent}<ActionPeriod>{action_period}</ActionPeriod>"
    ));
    lines.push(format!("{indent}<BasePeriod>{base_period}</BasePeriod>"));
    emit_meta_optional_text(
        lines,
        indent,
        "Schedule",
        defn.get("schedule").and_then(Value::as_str),
    );
    emit_meta_optional_text(
        lines,
        indent,
        "ScheduleValue",
        defn.get("scheduleValue").and_then(Value::as_str),
    );
    emit_meta_optional_text(
        lines,
        indent,
        "ScheduleDate",
        defn.get("scheduleDate").and_then(Value::as_str),
    );
    emit_meta_optional_text(
        lines,
        indent,
        "ChartOfCalculationTypes",
        defn.string("chartOfCalculationTypes"),
    );
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    emit_meta_standard_attributes(lines, indent, "CalculationRegister");
    emit_meta_register_tail(lines, indent, defn);
    lines.push(format!("{indent}<ListPresentation/>"));
    lines.push(format!("{indent}<ExtendedListPresentation/>"));
    lines.push(format!("{indent}<Explanation/>"));
}

pub(super) fn emit_meta_chart_of_accounts_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    lines.push(format!("{indent}<BasedOn/>"));
    let ext_dimension_types = defn
        .get("extDimensionTypes")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    emit_meta_optional_text(lines, indent, "ExtDimensionTypes", ext_dimension_types);
    let max_ext_dimension_count = defn
        .get("maxExtDimensionCount")
        .and_then(json_i64_value)
        .unwrap_or_else(|| if ext_dimension_types.is_some() { 3 } else { 0 });
    lines.push(format!(
        "{indent}<MaxExtDimensionCount>{max_ext_dimension_count}</MaxExtDimensionCount>"
    ));
    emit_meta_optional_text(
        lines,
        indent,
        "CodeMask",
        defn.get("codeMask").and_then(Value::as_str),
    );
    let code_length = defn.get("codeLength").and_then(json_i64_value).unwrap_or(8);
    let description_length = defn
        .get("descriptionLength")
        .and_then(json_i64_value)
        .unwrap_or(120);
    lines.push(format!("{indent}<CodeLength>{code_length}</CodeLength>"));
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    lines.push(format!(
        "{indent}<CodeSeries>{}</CodeSeries>",
        meta_enum_prop(defn, "codeSeries", "WholeChartOfAccounts")
    ));
    let check_unique = defn.get("checkUnique").and_then(Value::as_bool) == Some(true);
    lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        meta_enum_prop(defn, "defaultPresentation", "AsDescription")
    ));
    emit_meta_standard_attributes(lines, indent, "ChartOfAccounts");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!("{indent}<StandardTabularSections>"));
    lines.push(format!(
        "{indent}\t<xr:StandardTabularSection name=\"ExtDimensionTypes\">"
    ));
    lines.push(format!("{indent}\t\t<xr:Synonym>"));
    lines.push(format!("{indent}\t\t\t<v8:item>"));
    lines.push(format!("{indent}\t\t\t\t<v8:lang/>"));
    lines.push(format!(
        "{indent}\t\t\t\t<v8:content>Extra dimension types</v8:content>"
    ));
    lines.push(format!("{indent}\t\t\t</v8:item>"));
    lines.push(format!("{indent}\t\t</xr:Synonym>"));
    lines.push(format!("{indent}\t\t<xr:Comment/>"));
    lines.push(format!("{indent}\t\t<xr:ToolTip/>"));
    lines.push(format!(
        "{indent}\t\t<xr:FillChecking>DontCheck</xr:FillChecking>"
    ));
    lines.push(format!("{indent}\t\t<xr:StandardAttributes>"));
    for attr in [
        "TurnoversOnly",
        "Predefined",
        "ExtDimensionType",
        "LineNumber",
    ] {
        emit_meta_standard_attribute(
            lines,
            &format!("{indent}\t\t\t"),
            "ChartOfAccounts.ExtDimensionTypes",
            attr,
        );
    }
    lines.push(format!("{indent}\t\t</xr:StandardAttributes>"));
    lines.push(format!("{indent}\t</xr:StandardTabularSection>"));
    lines.push(format!("{indent}</StandardTabularSections>"));
    lines.push(format!(
        "{indent}<PredefinedDataUpdate>Auto</PredefinedDataUpdate>"
    ));
    lines.push(format!("{indent}<EditType>InDialog</EditType>"));
    lines.push(format!("{indent}<QuickChoice>false</QuickChoice>"));
    lines.push(format!("{indent}<ChoiceMode>BothWays</ChoiceMode>"));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "ChartOfAccounts.{obj_name}.StandardAttribute.Description"
        ))
    ));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "ChartOfAccounts.{obj_name}.StandardAttribute.Code"
        ))
    ));
    lines.push(format!("{indent}</InputByString>"));
    for line in [
        "<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>",
        "<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>",
        "<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>",
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
        "<DefaultObjectForm/>",
        "<DefaultListForm/>",
        "<DefaultChoiceForm/>",
        "<AuxiliaryObjectForm/>",
        "<AuxiliaryListForm/>",
        "<AuxiliaryChoiceForm/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    let auto_order_by_code = defn.get("autoOrderByCode").and_then(Value::as_bool) != Some(false);
    let order_length = defn
        .get("orderLength")
        .and_then(json_i64_value)
        .unwrap_or(5);
    lines.push(format!(
        "{indent}<AutoOrderByCode>{auto_order_by_code}</AutoOrderByCode>"
    ));
    lines.push(format!("{indent}<OrderLength>{order_length}</OrderLength>"));
    lines.push(format!("{indent}<DataLockFields/>"));
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        meta_enum_prop(defn, "fullTextSearch", "Use")
    ));
    for line in [
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
        "<ObjectPresentation/>",
        "<ExtendedObjectPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(super) fn emit_meta_chart_of_characteristic_types_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    emit_meta_optional_text(
        lines,
        indent,
        "CharacteristicExtValues",
        defn.get("characteristicExtValues").and_then(Value::as_str),
    );
    let value_types = meta_compile_value_types(defn);
    if value_types.is_empty() {
        lines.push(format!("{indent}<Type>"));
        emit_meta_type_contents(
            lines,
            &format!("{indent}\t"),
            ["Boolean", "String(100)", "Number(15,2)", "DateTime"],
        );
        lines.push(format!("{indent}</Type>"));
    } else {
        lines.push(format!("{indent}<Type>"));
        emit_meta_type_contents(
            lines,
            &format!("{indent}\t"),
            value_types.iter().map(String::as_str),
        );
        lines.push(format!("{indent}</Type>"));
    }
    let hierarchical = defn.get("hierarchical").and_then(Value::as_bool) == Some(true);
    lines.push(format!(
        "{indent}<Hierarchical>{hierarchical}</Hierarchical>"
    ));
    let folders_on_top = defn.get("foldersOnTop").and_then(Value::as_bool) != Some(false);
    lines.push(format!(
        "{indent}<FoldersOnTop>{folders_on_top}</FoldersOnTop>"
    ));
    let code_length = defn.get("codeLength").and_then(json_i64_value).unwrap_or(9);
    let description_length = defn
        .get("descriptionLength")
        .and_then(json_i64_value)
        .unwrap_or(25);
    lines.push(format!("{indent}<CodeLength>{code_length}</CodeLength>"));
    lines.push(format!(
        "{indent}<CodeAllowedLength>{}</CodeAllowedLength>",
        meta_enum_prop(defn, "codeAllowedLength", "Variable")
    ));
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    lines.push(format!(
        "{indent}<CodeSeries>{}</CodeSeries>",
        meta_enum_prop(defn, "codeSeries", "WholeCharacteristicKind")
    ));
    let check_unique = defn.get("checkUnique").and_then(Value::as_bool) == Some(true);
    let autonumbering = defn.get("autonumbering").and_then(Value::as_bool) != Some(false);
    lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    lines.push(format!(
        "{indent}<Autonumbering>{autonumbering}</Autonumbering>"
    ));
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        meta_enum_prop(defn, "defaultPresentation", "AsDescription")
    ));
    emit_meta_standard_attributes(lines, indent, "ChartOfCharacteristicTypes");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!(
        "{indent}<PredefinedDataUpdate>{}</PredefinedDataUpdate>",
        meta_enum_prop(defn, "predefinedDataUpdate", "Auto")
    ));
    lines.push(format!(
        "{indent}<EditType>{}</EditType>",
        meta_enum_prop(defn, "editType", "InDialog")
    ));
    let quick_choice = defn.get("quickChoice").and_then(Value::as_bool) == Some(true);
    lines.push(format!("{indent}<QuickChoice>{quick_choice}</QuickChoice>"));
    lines.push(format!(
        "{indent}<ChoiceMode>{}</ChoiceMode>",
        meta_enum_prop(defn, "choiceMode", "BothWays")
    ));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "ChartOfCharacteristicTypes.{obj_name}.StandardAttribute.Description"
        ))
    ));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "ChartOfCharacteristicTypes.{obj_name}.StandardAttribute.Code"
        ))
    ));
    lines.push(format!("{indent}</InputByString>"));
    for line in [
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>",
        "<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>",
        "<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
        "<DefaultObjectForm/>",
        "<DefaultFolderForm/>",
        "<DefaultListForm/>",
        "<DefaultChoiceForm/>",
        "<DefaultFolderChoiceForm/>",
        "<AuxiliaryObjectForm/>",
        "<AuxiliaryFolderForm/>",
        "<AuxiliaryListForm/>",
        "<AuxiliaryChoiceForm/>",
        "<AuxiliaryFolderChoiceForm/>",
        "<BasedOn/>",
        "<DataLockFields/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        meta_enum_prop(defn, "fullTextSearch", "Use")
    ));
    for line in [
        "<ObjectPresentation/>",
        "<ExtendedObjectPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(super) fn emit_meta_chart_of_calculation_types_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    let code_length = defn.get("codeLength").and_then(json_i64_value).unwrap_or(9);
    let description_length = defn
        .get("descriptionLength")
        .and_then(json_i64_value)
        .unwrap_or(25);
    lines.push(format!("{indent}<CodeLength>{code_length}</CodeLength>"));
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    lines.push(format!(
        "{indent}<CodeType>{}</CodeType>",
        meta_enum_prop(defn, "codeType", "String")
    ));
    lines.push(format!(
        "{indent}<CodeAllowedLength>{}</CodeAllowedLength>",
        meta_enum_prop(defn, "codeAllowedLength", "Variable")
    ));
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        meta_enum_prop(defn, "defaultPresentation", "AsDescription")
    ));
    lines.push(format!("{indent}<EditType>InDialog</EditType>"));
    lines.push(format!("{indent}<QuickChoice>false</QuickChoice>"));
    lines.push(format!("{indent}<ChoiceMode>BothWays</ChoiceMode>"));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "ChartOfCalculationTypes.{obj_name}.StandardAttribute.Description"
        ))
    ));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "ChartOfCalculationTypes.{obj_name}.StandardAttribute.Code"
        ))
    ));
    lines.push(format!("{indent}</InputByString>"));
    for line in [
        "<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>",
        "<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>",
        "<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>",
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
        "<DefaultObjectForm/>",
        "<DefaultListForm/>",
        "<DefaultChoiceForm/>",
        "<AuxiliaryObjectForm/>",
        "<AuxiliaryListForm/>",
        "<AuxiliaryChoiceForm/>",
        "<BasedOn/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    lines.push(format!(
        "{indent}<DependenceOnCalculationTypes>{}</DependenceOnCalculationTypes>",
        meta_enum_prop(defn, "dependenceOnCalculationTypes", "DontUse")
    ));
    emit_meta_md_object_refs(
        lines,
        indent,
        "BaseCalculationTypes",
        &meta_compile_string_list(defn.get("baseCalculationTypes")),
    );
    let action_period_use = defn.get("actionPeriodUse").and_then(Value::as_bool) == Some(true);
    lines.push(format!(
        "{indent}<ActionPeriodUse>{action_period_use}</ActionPeriodUse>"
    ));
    emit_meta_standard_attributes(lines, indent, "ChartOfCalculationTypes");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!(
        "{indent}<PredefinedDataUpdate>Auto</PredefinedDataUpdate>"
    ));
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    lines.push(format!("{indent}<DataLockFields/>"));
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        meta_enum_prop(defn, "fullTextSearch", "Use")
    ));
    for line in [
        "<ObjectPresentation/>",
        "<ExtendedObjectPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(super) fn emit_meta_business_process_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!(
        "{indent}<EditType>{}</EditType>",
        meta_enum_prop(defn, "editType", "InDialog")
    ));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "BusinessProcess.{obj_name}.StandardAttribute.Number"
        ))
    ));
    lines.push(format!("{indent}</InputByString>"));
    for line in [
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>",
        "<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>",
        "<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>",
        "<DefaultObjectForm/>",
        "<DefaultListForm/>",
        "<DefaultChoiceForm/>",
        "<AuxiliaryObjectForm/>",
        "<AuxiliaryListForm/>",
        "<AuxiliaryChoiceForm/>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    lines.push(format!(
        "{indent}<NumberType>{}</NumberType>",
        meta_enum_prop(defn, "numberType", "String")
    ));
    let number_length = defn
        .get("numberLength")
        .and_then(json_i64_value)
        .unwrap_or(11);
    lines.push(format!(
        "{indent}<NumberLength>{number_length}</NumberLength>"
    ));
    lines.push(format!(
        "{indent}<NumberAllowedLength>{}</NumberAllowedLength>",
        meta_enum_prop(defn, "numberAllowedLength", "Variable")
    ));
    let check_unique = defn.get("checkUnique").and_then(Value::as_bool) != Some(false);
    lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    emit_meta_standard_attributes(lines, indent, "BusinessProcess");
    lines.push(format!("{indent}<Characteristics/>"));
    let autonumbering = defn.get("autonumbering").and_then(Value::as_bool) != Some(false);
    lines.push(format!(
        "{indent}<Autonumbering>{autonumbering}</Autonumbering>"
    ));
    lines.push(format!("{indent}<BasedOn/>"));
    lines.push(format!(
        "{indent}<NumberPeriodicity>{}</NumberPeriodicity>",
        meta_enum_prop(defn, "numberPeriodicity", "Nonperiodical")
    ));
    emit_meta_optional_text(
        lines,
        indent,
        "Task",
        defn.get("task").and_then(Value::as_str),
    );
    let privileged = defn
        .get("createTaskInPrivilegedMode")
        .and_then(Value::as_bool)
        != Some(false);
    lines.push(format!(
        "{indent}<CreateTaskInPrivilegedMode>{privileged}</CreateTaskInPrivilegedMode>"
    ));
    lines.push(format!("{indent}<DataLockFields/>"));
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        meta_enum_prop(defn, "fullTextSearch", "Use")
    ));
    for line in [
        "<ObjectPresentation/>",
        "<ExtendedObjectPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(super) fn emit_meta_task_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    emit_meta_number_properties(lines, indent, defn, 14);
    lines.push(format!(
        "{indent}<TaskNumberAutoPrefix>{}</TaskNumberAutoPrefix>",
        escape_xml(
            defn.get("taskNumberAutoPrefix")
                .and_then(Value::as_str)
                .unwrap_or("BusinessProcessNumber")
        )
    ));
    let description_length = defn
        .get("descriptionLength")
        .and_then(json_i64_value)
        .unwrap_or(150);
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    emit_meta_optional_text(
        lines,
        indent,
        "Addressing",
        defn.get("addressing").and_then(Value::as_str),
    );
    emit_meta_optional_text(
        lines,
        indent,
        "MainAddressingAttribute",
        defn.get("mainAddressingAttribute").and_then(Value::as_str),
    );
    emit_meta_optional_text(
        lines,
        indent,
        "CurrentPerformer",
        defn.get("currentPerformer").and_then(Value::as_str),
    );
    lines.push(format!("{indent}<BasedOn/>"));
    emit_meta_standard_attributes(lines, indent, "Task");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        meta_enum_prop(defn, "defaultPresentation", "AsDescription")
    ));
    lines.push(format!(
        "{indent}<EditType>{}</EditType>",
        meta_enum_prop(defn, "editType", "InDialog")
    ));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!("Task.{obj_name}.StandardAttribute.Number"))
    ));
    lines.push(format!("{indent}</InputByString>"));
    for line in [
        "<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>",
        "<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>",
        "<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>",
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<DefaultObjectForm/>",
        "<DefaultListForm/>",
        "<DefaultChoiceForm/>",
        "<AuxiliaryObjectForm/>",
        "<AuxiliaryListForm/>",
        "<AuxiliaryChoiceForm/>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
        "<IncludeHelpInContents>false</IncludeHelpInContents>",
        "<DataLockFields/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        meta_enum_prop(defn, "fullTextSearch", "Use")
    ));
    for line in [
        "<ObjectPresentation/>",
        "<ExtendedObjectPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(super) fn emit_meta_exchange_plan_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    let code_length = defn.get("codeLength").and_then(json_i64_value).unwrap_or(9);
    let description_length = defn
        .get("descriptionLength")
        .and_then(json_i64_value)
        .unwrap_or(100);
    lines.push(format!("{indent}<CodeLength>{code_length}</CodeLength>"));
    lines.push(format!(
        "{indent}<CodeAllowedLength>{}</CodeAllowedLength>",
        meta_enum_prop(defn, "codeAllowedLength", "Variable")
    ));
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        meta_enum_prop(defn, "defaultPresentation", "AsDescription")
    ));
    lines.push(format!(
        "{indent}<EditType>{}</EditType>",
        meta_enum_prop(defn, "editType", "InDialog")
    ));
    let quick_choice = defn.get("quickChoice").and_then(Value::as_bool) == Some(true);
    lines.push(format!("{indent}<QuickChoice>{quick_choice}</QuickChoice>"));
    lines.push(format!(
        "{indent}<ChoiceMode>{}</ChoiceMode>",
        meta_enum_prop(defn, "choiceMode", "BothWays")
    ));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "ExchangePlan.{obj_name}.StandardAttribute.Description"
        ))
    ));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!("ExchangePlan.{obj_name}.StandardAttribute.Code"))
    ));
    lines.push(format!("{indent}</InputByString>"));
    for line in [
        "<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>",
        "<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>",
        "<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>",
        "<DefaultObjectForm/>",
        "<DefaultListForm/>",
        "<DefaultChoiceForm/>",
        "<AuxiliaryObjectForm/>",
        "<AuxiliaryListForm/>",
        "<AuxiliaryChoiceForm/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    emit_meta_standard_attributes(lines, indent, "ExchangePlan");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!("{indent}<BasedOn/>"));
    let distributed = defn.get("distributedInfoBase").and_then(Value::as_bool) == Some(true);
    let include_ext = defn
        .get("includeConfigurationExtensions")
        .and_then(Value::as_bool)
        == Some(true);
    lines.push(format!(
        "{indent}<DistributedInfoBase>{distributed}</DistributedInfoBase>"
    ));
    lines.push(format!(
        "{indent}<IncludeConfigurationExtensions>{include_ext}</IncludeConfigurationExtensions>"
    ));
    for line in [
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
        "<IncludeHelpInContents>false</IncludeHelpInContents>",
        "<DataLockFields/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        meta_enum_prop(defn, "fullTextSearch", "Use")
    ));
    for line in [
        "<ObjectPresentation/>",
        "<ExtendedObjectPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(super) fn emit_meta_document_journal_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    for tag in ["DefaultForm", "AuxiliaryForm"] {
        let field = if tag == "DefaultForm" {
            "defaultForm"
        } else {
            "auxiliaryForm"
        };
        emit_meta_optional_text(lines, indent, tag, defn.get(field).and_then(Value::as_str));
    }
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    emit_meta_md_object_refs(
        lines,
        indent,
        "RegisteredDocuments",
        &meta_compile_string_list(defn.get("registeredDocuments")),
    );
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    emit_meta_standard_attributes(lines, indent, "DocumentJournal");
    lines.push(format!("{indent}<ListPresentation/>"));
    lines.push(format!("{indent}<ExtendedListPresentation/>"));
    lines.push(format!("{indent}<Explanation/>"));
}

pub(super) fn emit_meta_report_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    for (tag, field) in [
        ("DefaultForm", "defaultForm"),
        ("AuxiliaryForm", "auxiliaryForm"),
        ("MainDataCompositionSchema", "mainDataCompositionSchema"),
        ("DefaultSettingsForm", "defaultSettingsForm"),
        ("AuxiliarySettingsForm", "auxiliarySettingsForm"),
        ("DefaultVariantForm", "defaultVariantForm"),
    ] {
        emit_meta_optional_text(lines, indent, tag, defn.get(field).and_then(Value::as_str));
    }
    for line in [
        "<VariantsStorage/>",
        "<SettingsStorage/>",
        "<IncludeHelpInContents>false</IncludeHelpInContents>",
        "<ExtendedPresentation/>",
        "<Explanation/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(super) fn emit_meta_data_processor_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>false</UseStandardCommands>"
    ));
    emit_meta_optional_text(
        lines,
        indent,
        "DefaultForm",
        defn.get("defaultForm").and_then(Value::as_str),
    );
    emit_meta_optional_text(
        lines,
        indent,
        "AuxiliaryForm",
        defn.get("auxiliaryForm").and_then(Value::as_str),
    );
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    lines.push(format!("{indent}<ExtendedPresentation/>"));
    lines.push(format!("{indent}<Explanation/>"));
}

pub(super) fn emit_meta_scheduled_job_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    let method_name =
        meta_compile_common_module_method(defn.string("methodName").unwrap_or_default());
    lines.push(format!(
        "{indent}<MethodName>{}</MethodName>",
        escape_xml(&method_name)
    ));
    let description = defn
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or(synonym);
    lines.push(format!(
        "{indent}<Description>{}</Description>",
        escape_xml(description)
    ));
    emit_meta_optional_text(
        lines,
        indent,
        "Key",
        defn.get("key").and_then(Value::as_str),
    );
    let use_job = defn.get("use").and_then(Value::as_bool) == Some(true);
    let predefined = defn.get("predefined").and_then(Value::as_bool) == Some(true);
    let restart_count = defn
        .get("restartCountOnFailure")
        .and_then(json_i64_value)
        .unwrap_or(3);
    let restart_interval = defn
        .get("restartIntervalOnFailure")
        .and_then(json_i64_value)
        .unwrap_or(10);
    lines.push(format!("{indent}<Use>{use_job}</Use>"));
    lines.push(format!("{indent}<Predefined>{predefined}</Predefined>"));
    lines.push(format!(
        "{indent}<RestartCountOnFailure>{restart_count}</RestartCountOnFailure>"
    ));
    lines.push(format!(
        "{indent}<RestartIntervalOnFailure>{restart_interval}</RestartIntervalOnFailure>"
    ));
}

pub(super) fn emit_meta_event_subscription_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    let sources = defn.string_list("source");
    if sources.is_empty() {
        lines.push(format!("{indent}<Source/>"));
    } else {
        lines.push(format!("{indent}<Source>"));
        emit_meta_event_subscription_source_type_contents(
            lines,
            &format!("{indent}\t"),
            sources.iter().map(String::as_str),
        );
        lines.push(format!("{indent}</Source>"));
    }
    lines.push(format!(
        "{indent}<Event>{}</Event>",
        escape_xml(
            defn.get("event")
                .and_then(Value::as_str)
                .unwrap_or("BeforeWrite")
        )
    ));
    let handler = meta_compile_common_module_method(defn.string("handler").unwrap_or_default());
    lines.push(format!(
        "{indent}<Handler>{}</Handler>",
        escape_xml(&handler)
    ));
}

pub(super) fn emit_meta_http_service_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    let root_url = defn
        .get("rootURL")
        .or_else(|| defn.get("rootUrl"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| obj_name.to_lowercase());
    lines.push(format!(
        "{indent}<RootURL>{}</RootURL>",
        escape_xml(&root_url)
    ));
    lines.push(format!(
        "{indent}<ReuseSessions>{}</ReuseSessions>",
        meta_enum_prop(defn, "reuseSessions", "DontUse")
    ));
    let session_max_age = defn
        .get("sessionMaxAge")
        .and_then(json_i64_value)
        .unwrap_or(20);
    lines.push(format!(
        "{indent}<SessionMaxAge>{session_max_age}</SessionMaxAge>"
    ));
}

pub(super) fn emit_meta_web_service_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    emit_meta_optional_text(
        lines,
        indent,
        "Namespace",
        defn.get("namespace").and_then(Value::as_str),
    );
    emit_meta_optional_text(
        lines,
        indent,
        "XDTOPackages",
        defn.get("xdtoPackages").and_then(Value::as_str),
    );
    lines.push(format!(
        "{indent}<DescriptorFileName>{}</DescriptorFileName>",
        escape_xml(
            defn.get("descriptorFileName")
                .and_then(Value::as_str)
                .unwrap_or("ws1.1cws")
        )
    ));
    lines.push(format!(
        "{indent}<ReuseSessions>{}</ReuseSessions>",
        meta_enum_prop(defn, "reuseSessions", "DontUse")
    ));
    let session_max_age = defn
        .get("sessionMaxAge")
        .and_then(json_i64_value)
        .unwrap_or(20);
    lines.push(format!(
        "{indent}<SessionMaxAge>{session_max_age}</SessionMaxAge>"
    ));
}

pub(super) fn meta_compile_root_value_type(defn: &MetaTemplateDefinition<'_>) -> String {
    let mut type_name = defn
        .get("valueType")
        .and_then(Value::as_str)
        .unwrap_or("String")
        .to_string();
    if !type_name.is_empty() && !type_name.contains('(') {
        if type_name == "String" {
            if let Some(length) = defn.get("length").and_then(json_i64_value) {
                type_name = format!("String({length})");
            }
        } else if type_name == "Number" {
            if let Some(length) = defn.get("length").and_then(json_i64_value) {
                let precision = defn.get("precision").and_then(json_i64_value).unwrap_or(0);
                let nn = if defn.get("nonneg").and_then(Value::as_bool) == Some(true)
                    || defn.get("nonnegative").and_then(Value::as_bool) == Some(true)
                {
                    ",nonneg"
                } else {
                    ""
                };
                type_name = format!("Number({length},{precision}{nn})");
            }
        }
    }
    type_name
}

pub(super) fn emit_meta_common_module_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    let context = defn
        .get("context")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut server = bool_arg_from_json(defn, "server");
    let mut server_call = bool_arg_from_json(defn, "serverCall");
    let mut client_managed = bool_arg_from_json(defn, "clientManagedApplication");
    match context {
        "server" | "serverCall" => {
            server = true;
            server_call = true;
        }
        "client" => client_managed = true,
        "serverClient" => {
            server = true;
            client_managed = true;
        }
        _ => {}
    }
    lines.push(format!(
        "{indent}<Global>{}</Global>",
        bool_arg_from_json(defn, "global")
    ));
    lines.push(format!(
        "{indent}<ClientManagedApplication>{client_managed}</ClientManagedApplication>"
    ));
    lines.push(format!("{indent}<Server>{server}</Server>"));
    lines.push(format!(
        "{indent}<ExternalConnection>{}</ExternalConnection>",
        bool_arg_from_json(defn, "externalConnection")
    ));
    lines.push(format!(
        "{indent}<ClientOrdinaryApplication>{}</ClientOrdinaryApplication>",
        bool_arg_from_json(defn, "clientOrdinaryApplication")
    ));
    lines.push(format!("{indent}<ServerCall>{server_call}</ServerCall>"));
    lines.push(format!(
        "{indent}<Privileged>{}</Privileged>",
        bool_arg_from_json(defn, "privileged")
    ));
    lines.push(format!(
        "{indent}<ReturnValuesReuse>{}</ReturnValuesReuse>",
        meta_enum_prop(defn, "returnValuesReuse", "DontUse")
    ));
}

pub(super) fn emit_meta_defined_type_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    let value_types = meta_compile_value_types(defn);
    if value_types.is_empty() {
        lines.push(format!("{indent}<Type/>"));
        return;
    }
    lines.push(format!("{indent}<Type>"));
    emit_meta_type_contents(
        lines,
        &format!("{indent}\t"),
        value_types.iter().map(String::as_str),
    );
    lines.push(format!("{indent}</Type>"));
}

pub(super) fn bool_arg_from_json(defn: &MetaTemplateDefinition<'_>, field_name: &str) -> bool {
    defn.get(field_name).and_then(Value::as_bool) == Some(true)
}

pub(super) fn meta_compile_value_types(defn: &MetaTemplateDefinition<'_>) -> Vec<String> {
    let value = defn.get("valueTypes").or_else(|| defn.get("valueType"));
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect(),
        Some(Value::String(value)) if !value.is_empty() => vec![value.to_string()],
        _ => Vec::new(),
    }
}

pub(super) fn emit_meta_optional_text(
    lines: &mut Vec<String>,
    indent: &str,
    tag: &str,
    value: Option<&str>,
) {
    match value.filter(|value| !value.is_empty()) {
        Some(value) => lines.push(format!("{indent}<{tag}>{}</{tag}>", escape_xml(value))),
        None => lines.push(format!("{indent}<{tag}/>")),
    }
}

pub(super) fn meta_compile_string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| {
                if let Some(text) = item.as_str() {
                    Some(text.to_string())
                } else {
                    item.as_object()
                        .and_then(|object| object.get("name"))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                }
            })
            .collect(),
        Some(Value::String(value)) if !value.is_empty() => vec![value.to_string()],
        Some(Value::Object(object)) => object.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

pub(super) fn meta_compile_named_items(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| {
                item.as_str().map(ToOwned::to_owned).or_else(|| {
                    item.as_object()
                        .and_then(|object| object.get("name"))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
            })
            .collect(),
        Some(Value::Object(object)) => object.keys().cloned().collect(),
        Some(Value::String(value)) if !value.is_empty() => vec![value.to_string()],
        _ => Vec::new(),
    }
}

pub(super) fn normalize_meta_object_ref(value: &str) -> String {
    let Some((prefix, suffix)) = value.split_once('.') else {
        return value.to_string();
    };
    let normalized = normalize_meta_object_type(prefix);
    format!("{normalized}.{suffix}")
}

pub(super) fn emit_meta_md_object_refs(
    lines: &mut Vec<String>,
    indent: &str,
    tag: &str,
    refs: &[String],
) {
    if refs.is_empty() {
        lines.push(format!("{indent}<{tag}/>"));
        return;
    }
    lines.push(format!("{indent}<{tag}>"));
    for item in refs {
        lines.push(format!(
            "{indent}\t<xr:Item xsi:type=\"xr:MDObjectRef\">{}</xr:Item>",
            escape_xml(&normalize_meta_object_ref(item))
        ));
    }
    lines.push(format!("{indent}</{tag}>"));
}

pub(super) fn meta_compile_common_module_method(value: &str) -> String {
    if value.is_empty() || value.starts_with("CommonModule.") {
        value.to_string()
    } else {
        format!("CommonModule.{value}")
    }
}

pub(super) fn emit_meta_lock_search_presentation_tail(
    lines: &mut Vec<String>,
    indent: &str,
    full_text_search_default: &str,
) {
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    lines.push(format!("{indent}<DataLockFields/>"));
    lines.push(format!(
        "{indent}<DataLockControlMode>Automatic</DataLockControlMode>"
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        escape_xml(full_text_search_default)
    ));
    for line in [
        "<ObjectPresentation/>",
        "<ExtendedObjectPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(super) fn emit_meta_register_tail(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
) {
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        meta_enum_prop(defn, "fullTextSearch", "Use")
    ));
}

pub(super) fn emit_meta_code_description_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    default_code_length: i64,
    default_description_length: i64,
    include_check_unique: bool,
    include_autonumbering: bool,
) {
    let code_length = defn
        .get("codeLength")
        .and_then(json_i64_value)
        .unwrap_or(default_code_length);
    let description_length = defn
        .get("descriptionLength")
        .and_then(json_i64_value)
        .unwrap_or(default_description_length);
    lines.push(format!("{indent}<CodeLength>{code_length}</CodeLength>"));
    lines.push(format!(
        "{indent}<CodeAllowedLength>{}</CodeAllowedLength>",
        meta_enum_prop(defn, "codeAllowedLength", "Variable")
    ));
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    if include_check_unique {
        let check_unique = defn.get("checkUnique").and_then(Value::as_bool) == Some(true);
        lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    } else {
        lines.push(format!("{indent}<CheckUnique>false</CheckUnique>"));
    }
    if include_autonumbering {
        let autonumbering = defn.get("autonumbering").and_then(Value::as_bool) != Some(false);
        lines.push(format!(
            "{indent}<Autonumbering>{autonumbering}</Autonumbering>"
        ));
    }
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        meta_enum_prop(defn, "defaultPresentation", "AsDescription")
    ));
}

pub(super) fn emit_meta_choice_object_tail(
    lines: &mut Vec<String>,
    indent: &str,
    object_type: &str,
    obj_name: &str,
    include_characteristics: bool,
) {
    if include_characteristics {
        lines.push(format!("{indent}<Characteristics/>"));
        lines.push(format!(
            "{indent}<PredefinedDataUpdate>Auto</PredefinedDataUpdate>"
        ));
    }
    lines.push(format!("{indent}<EditType>InDialog</EditType>"));
    lines.push(format!("{indent}<QuickChoice>false</QuickChoice>"));
    lines.push(format!("{indent}<ChoiceMode>BothWays</ChoiceMode>"));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "{object_type}.{obj_name}.StandardAttribute.Description"
        ))
    ));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!("{object_type}.{obj_name}.StandardAttribute.Code"))
    ));
    lines.push(format!("{indent}</InputByString>"));
    for line in [
        "<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>",
        "<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>",
        "<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>",
        "<DefaultObjectForm/>",
        "<DefaultListForm/>",
        "<DefaultChoiceForm/>",
        "<AuxiliaryObjectForm/>",
        "<AuxiliaryListForm/>",
        "<AuxiliaryChoiceForm/>",
        "<IncludeHelpInContents>false</IncludeHelpInContents>",
        "<BasedOn/>",
        "<DataLockFields/>",
        "<DataLockControlMode>Automatic</DataLockControlMode>",
        "<FullTextSearch>Use</FullTextSearch>",
        "<ObjectPresentation/>",
        "<ExtendedObjectPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(super) fn emit_meta_number_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    default_number_length: i64,
) {
    lines.push(format!(
        "{indent}<NumberType>{}</NumberType>",
        meta_enum_prop(defn, "numberType", "String")
    ));
    let number_length = defn
        .get("numberLength")
        .and_then(json_i64_value)
        .unwrap_or(default_number_length);
    lines.push(format!(
        "{indent}<NumberLength>{number_length}</NumberLength>"
    ));
    lines.push(format!(
        "{indent}<NumberAllowedLength>{}</NumberAllowedLength>",
        meta_enum_prop(defn, "numberAllowedLength", "Variable")
    ));
    let check_unique = defn.get("checkUnique").and_then(Value::as_bool) != Some(false);
    let autonumbering = defn.get("autonumbering").and_then(Value::as_bool) != Some(false);
    lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    lines.push(format!(
        "{indent}<Autonumbering>{autonumbering}</Autonumbering>"
    ));
}

pub(super) fn emit_meta_numbered_object_tail(
    lines: &mut Vec<String>,
    indent: &str,
    object_type: &str,
    obj_name: &str,
) {
    lines.push(format!("{indent}<BasedOn/>"));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "{object_type}.{obj_name}.StandardAttribute.Number"
        ))
    ));
    lines.push(format!("{indent}</InputByString>"));
    for line in [
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>",
        "<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>",
        "<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>",
        "<DefaultObjectForm/>",
        "<DefaultListForm/>",
        "<DefaultChoiceForm/>",
        "<AuxiliaryObjectForm/>",
        "<AuxiliaryListForm/>",
        "<AuxiliaryChoiceForm/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    emit_meta_lock_search_presentation_tail(lines, indent, "Use");
}

pub(super) struct MetaCompileEnumValue {
    pub(crate) name: String,
    pub(crate) synonym: String,
    pub(crate) comment: String,
}

pub(super) fn meta_compile_enum_values(
    value: Option<&Value>,
) -> Result<Vec<MetaCompileEnumValue>, String> {
    let Some(Value::Array(items)) = value else {
        return Ok(Vec::new());
    };
    let mut values = Vec::new();
    for item in items {
        if let Some(name) = item.as_str() {
            values.push(MetaCompileEnumValue {
                name: name.to_string(),
                synonym: split_meta_camel_case(name),
                comment: String::new(),
            });
            continue;
        }
        let object = item
            .as_object()
            .ok_or_else(|| "enum value must be a string or object".to_string())?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "enum value is missing name".to_string())?;
        values.push(MetaCompileEnumValue {
            name: name.to_string(),
            synonym: object
                .get("synonym")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| split_meta_camel_case(name)),
            comment: object
                .get("comment")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });
    }
    Ok(values)
}

pub(super) fn emit_meta_enum_value<F>(
    lines: &mut Vec<String>,
    indent: &str,
    value: &MetaCompileEnumValue,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    lines.push(format!("{indent}<EnumValue uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!(
        "{indent}\t\t<Name>{}</Name>",
        escape_xml(&value.name)
    ));
    emit_meta_mltext(lines, &format!("{indent}\t\t"), "Synonym", &value.synonym);
    if value.comment.is_empty() {
        lines.push(format!("{indent}\t\t<Comment/>"));
    } else {
        lines.push(format!(
            "{indent}\t\t<Comment>{}</Comment>",
            escape_xml(&value.comment)
        ));
    }
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</EnumValue>"));
}

pub(super) fn emit_meta_child_objects<F>(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition<'_>,
    obj_type: &str,
    obj_name: &str,
    next_uuid: &mut F,
) -> Result<(), String>
where
    F: FnMut() -> String,
{
    match obj_type {
        "Enum" => {
            let values = meta_compile_enum_values(defn.get("values"))?;
            if values.is_empty() {
                lines.push(format!("{indent}<ChildObjects/>"));
            } else {
                lines.push(format!("{indent}<ChildObjects>"));
                for value in &values {
                    emit_meta_enum_value(lines, &format!("{indent}\t"), value, next_uuid);
                }
                lines.push(format!("{indent}</ChildObjects>"));
            }
        }
        "Document"
        | "Report"
        | "DataProcessor"
        | "ExchangePlan"
        | "ChartOfCharacteristicTypes"
        | "ChartOfAccounts"
        | "ChartOfCalculationTypes"
        | "BusinessProcess"
        | "Task" => {
            let attrs = meta_compile_attributes(defn.get("attributes"));
            let tabular_sections = meta_compile_tabular_sections(defn.get("tabularSections"))?;
            let accounting_flags = if obj_type == "ChartOfAccounts" {
                meta_compile_named_items(defn.get("accountingFlags"))
            } else {
                Vec::new()
            };
            let ext_dimension_flags = if obj_type == "ChartOfAccounts" {
                meta_compile_named_items(defn.get("extDimensionAccountingFlags"))
            } else {
                Vec::new()
            };
            let addressing_attrs = if obj_type == "Task" {
                meta_compile_value_items(defn.get("addressingAttributes"))
            } else {
                Vec::new()
            };
            if attrs.is_empty()
                && tabular_sections.is_empty()
                && accounting_flags.is_empty()
                && ext_dimension_flags.is_empty()
                && addressing_attrs.is_empty()
            {
                lines.push(format!("{indent}<ChildObjects/>"));
                return Ok(());
            }
            lines.push(format!("{indent}<ChildObjects>"));
            let attr_context = match obj_type {
                "Document" => "document",
                "Report" | "DataProcessor" => "processor",
                "ChartOfAccounts" | "ChartOfCharacteristicTypes" | "ChartOfCalculationTypes" => {
                    "chart"
                }
                _ => "object",
            };
            for attr in &attrs {
                emit_meta_attribute(lines, &format!("{indent}\t"), attr, attr_context, next_uuid);
            }
            for section in &tabular_sections {
                emit_meta_tabular_section(
                    lines,
                    &format!("{indent}\t"),
                    section,
                    obj_type,
                    obj_name,
                    next_uuid,
                );
            }
            for name in accounting_flags {
                emit_meta_boolean_child(
                    lines,
                    &format!("{indent}\t"),
                    "AccountingFlag",
                    &name,
                    next_uuid,
                );
            }
            for name in ext_dimension_flags {
                emit_meta_boolean_child(
                    lines,
                    &format!("{indent}\t"),
                    "ExtDimensionAccountingFlag",
                    &name,
                    next_uuid,
                );
            }
            for item in addressing_attrs {
                emit_meta_addressing_attribute(lines, &format!("{indent}\t"), &item, next_uuid);
            }
            lines.push(format!("{indent}</ChildObjects>"));
        }
        "InformationRegister"
        | "AccumulationRegister"
        | "AccountingRegister"
        | "CalculationRegister" => {
            let dimensions = meta_compile_attributes(defn.get("dimensions"));
            let resources = meta_compile_attributes(defn.get("resources"));
            let attrs = meta_compile_attributes(defn.get("attributes"));
            if dimensions.is_empty() && resources.is_empty() && attrs.is_empty() {
                lines.push(format!("{indent}<ChildObjects/>"));
                return Ok(());
            }
            lines.push(format!("{indent}<ChildObjects>"));
            if obj_type == "AccountingRegister" {
                for dimension in &dimensions {
                    emit_meta_register_field(
                        lines,
                        &format!("{indent}\t"),
                        "Dimension",
                        dimension,
                        obj_type,
                        next_uuid,
                    );
                }
                for resource in &resources {
                    emit_meta_register_field(
                        lines,
                        &format!("{indent}\t"),
                        "Resource",
                        resource,
                        obj_type,
                        next_uuid,
                    );
                }
            } else {
                for resource in &resources {
                    emit_meta_register_field(
                        lines,
                        &format!("{indent}\t"),
                        "Resource",
                        resource,
                        obj_type,
                        next_uuid,
                    );
                }
                for dimension in &dimensions {
                    emit_meta_register_field(
                        lines,
                        &format!("{indent}\t"),
                        "Dimension",
                        dimension,
                        obj_type,
                        next_uuid,
                    );
                }
            }
            let attr_context = if obj_type == "InformationRegister" {
                "register-info"
            } else {
                "register-other"
            };
            for attr in &attrs {
                emit_meta_attribute(lines, &format!("{indent}\t"), attr, attr_context, next_uuid);
            }
            lines.push(format!("{indent}</ChildObjects>"));
        }
        "DocumentJournal" => {
            let columns = meta_compile_value_items(defn.get("columns"));
            if columns.is_empty() {
                lines.push(format!("{indent}<ChildObjects/>"));
                return Ok(());
            }
            lines.push(format!("{indent}<ChildObjects>"));
            for column in columns {
                emit_meta_column(lines, &format!("{indent}\t"), &column, next_uuid);
            }
            lines.push(format!("{indent}</ChildObjects>"));
        }
        "HTTPService" => {
            let templates = defn.get("urlTemplates").and_then(Value::as_object);
            if templates.is_none_or(Map::is_empty) {
                lines.push(format!("{indent}<ChildObjects/>"));
                return Ok(());
            }
            lines.push(format!("{indent}<ChildObjects>"));
            let mut ordered = templates.unwrap().iter().collect::<Vec<_>>();
            ordered.sort_by(|left, right| left.0.cmp(right.0));
            for (name, value) in ordered {
                emit_meta_url_template(lines, &format!("{indent}\t"), name, value, next_uuid);
            }
            lines.push(format!("{indent}</ChildObjects>"));
        }
        "WebService" => {
            let operations = defn.get("operations").and_then(Value::as_object);
            if operations.is_none_or(Map::is_empty) {
                lines.push(format!("{indent}<ChildObjects/>"));
                return Ok(());
            }
            lines.push(format!("{indent}<ChildObjects>"));
            let mut ordered = operations.unwrap().iter().collect::<Vec<_>>();
            ordered.sort_by(|left, right| left.0.cmp(right.0));
            for (name, value) in ordered {
                emit_meta_operation(lines, &format!("{indent}\t"), name, value, next_uuid);
            }
            lines.push(format!("{indent}</ChildObjects>"));
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn meta_compile_value_items(value: Option<&Value>) -> Vec<Value> {
    match value {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::Object(object)) => object
            .iter()
            .map(|(name, value)| {
                if let Some(mut cloned) = value.as_object().cloned() {
                    cloned
                        .entry("name".to_string())
                        .or_insert_with(|| Value::String(name.to_string()));
                    Value::Object(cloned)
                } else {
                    Value::String(name.to_string())
                }
            })
            .collect(),
        Some(Value::String(value)) => vec![Value::String(value.to_string())],
        _ => Vec::new(),
    }
}

pub(super) fn emit_meta_register_field<F>(
    lines: &mut Vec<String>,
    indent: &str,
    field_tag: &str,
    attr: &MetaCompileAttr,
    register_type: &str,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    lines.push(format!("{indent}<{field_tag} uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!(
        "{indent}\t\t<Name>{}</Name>",
        escape_xml(&attr.name)
    ));
    emit_meta_mltext(lines, &format!("{indent}\t\t"), "Synonym", &attr.synonym);
    lines.push(format!("{indent}\t\t<Comment/>"));
    if attr.type_name.is_empty() {
        if field_tag == "Resource" {
            emit_meta_value_type(lines, &format!("{indent}\t\t"), "Number(15,2)");
        } else {
            emit_meta_value_type(lines, &format!("{indent}\t\t"), "String");
        }
    } else {
        emit_meta_value_type(lines, &format!("{indent}\t\t"), &attr.type_name);
    }
    for line in [
        "<PasswordMode>false</PasswordMode>",
        "<Format/>",
        "<EditFormat/>",
        "<ToolTip/>",
        "<MarkNegatives>false</MarkNegatives>",
        "<Mask/>",
    ] {
        lines.push(format!("{indent}\t\t{line}"));
    }
    let multi_line = attr.multi_line || attr.flags.iter().any(|flag| flag == "multiline");
    lines.push(format!("{indent}\t\t<MultiLine>{multi_line}</MultiLine>"));
    lines.push(format!("{indent}\t\t<ExtendedEdit>false</ExtendedEdit>"));
    lines.push(format!("{indent}\t\t<MinValue xsi:nil=\"true\"/>"));
    lines.push(format!("{indent}\t\t<MaxValue xsi:nil=\"true\"/>"));
    if register_type == "InformationRegister" {
        let fill_from = field_tag == "Dimension" && attr.flags.iter().any(|flag| flag == "master");
        lines.push(format!(
            "{indent}\t\t<FillFromFillingValue>{fill_from}</FillFromFillingValue>"
        ));
        lines.push(format!("{indent}\t\t<FillValue xsi:nil=\"true\"/>"));
    }
    let fill_checking = if !attr.fill_checking.is_empty() {
        attr.fill_checking.as_str()
    } else if attr.flags.iter().any(|flag| flag == "req") {
        "ShowError"
    } else {
        "DontCheck"
    };
    lines.push(format!(
        "{indent}\t\t<FillChecking>{}</FillChecking>",
        escape_xml(fill_checking)
    ));
    for line in [
        "<ChoiceFoldersAndItems>Items</ChoiceFoldersAndItems>",
        "<ChoiceParameterLinks/>",
        "<ChoiceParameters/>",
        "<QuickChoice>Auto</QuickChoice>",
        "<CreateOnInput>Auto</CreateOnInput>",
        "<ChoiceForm/>",
        "<LinkByType/>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
    ] {
        lines.push(format!("{indent}\t\t{line}"));
    }
    if register_type == "AccountingRegister" {
        lines.push(format!("{indent}\t\t<Balance>true</Balance>"));
        lines.push(format!("{indent}\t\t<AccountingFlag/>"));
        if field_tag == "Resource" {
            lines.push(format!("{indent}\t\t<ExtDimensionAccountingFlag/>"));
        }
    }
    if field_tag == "Dimension" {
        if register_type == "InformationRegister" {
            let master = attr.flags.iter().any(|flag| flag == "master");
            let main_filter = attr.flags.iter().any(|flag| flag == "mainfilter");
            let deny_incomplete = attr.flags.iter().any(|flag| flag == "denyincomplete");
            lines.push(format!("{indent}\t\t<Master>{master}</Master>"));
            lines.push(format!(
                "{indent}\t\t<MainFilter>{main_filter}</MainFilter>"
            ));
            lines.push(format!(
                "{indent}\t\t<DenyIncompleteValues>{deny_incomplete}</DenyIncompleteValues>"
            ));
        } else if register_type == "AccumulationRegister" {
            let deny_incomplete = attr.flags.iter().any(|flag| flag == "denyincomplete");
            lines.push(format!(
                "{indent}\t\t<DenyIncompleteValues>{deny_incomplete}</DenyIncompleteValues>"
            ));
        } else if register_type == "AccountingRegister" {
            let deny_incomplete = attr.flags.iter().any(|flag| flag == "denyincomplete");
            lines.push(format!(
                "{indent}\t\t<DenyIncompleteValues>{deny_incomplete}</DenyIncompleteValues>"
            ));
        } else if register_type == "CalculationRegister" {
            let deny_incomplete = attr.flags.iter().any(|flag| flag == "denyincomplete");
            lines.push(format!(
                "{indent}\t\t<DenyIncompleteValues>{deny_incomplete}</DenyIncompleteValues>"
            ));
            lines.push(format!("{indent}\t\t<BaseDimension>false</BaseDimension>"));
            lines.push(format!("{indent}\t\t<ScheduleLink/>"));
        }
    }
    let indexing = if !attr.indexing.is_empty() {
        attr.indexing.as_str()
    } else if attr.flags.iter().any(|flag| flag == "index") {
        "Index"
    } else {
        "DontIndex"
    };
    if field_tag == "Dimension" || register_type == "InformationRegister" {
        lines.push(format!(
            "{indent}\t\t<Indexing>{}</Indexing>",
            escape_xml(indexing)
        ));
    }
    lines.push(format!("{indent}\t\t<FullTextSearch>Use</FullTextSearch>"));
    if field_tag == "Dimension" && register_type == "AccumulationRegister" {
        let use_in_totals = !attr.flags.iter().any(|flag| flag == "nouseintotals");
        lines.push(format!(
            "{indent}\t\t<UseInTotals>{use_in_totals}</UseInTotals>"
        ));
    }
    if register_type == "InformationRegister" {
        lines.push(format!("{indent}\t\t<DataHistory>Use</DataHistory>"));
        if field_tag == "Dimension" {
            lines.push(format!(
                "{indent}\t\t<TypeReductionMode>TransformValues</TypeReductionMode>"
            ));
        }
    }
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</{field_tag}>"));
}

pub(super) fn emit_meta_boolean_child<F>(
    lines: &mut Vec<String>,
    indent: &str,
    tag: &str,
    name: &str,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    lines.push(format!("{indent}<{tag} uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!("{indent}\t\t<Name>{}</Name>", escape_xml(name)));
    emit_meta_mltext(
        lines,
        &format!("{indent}\t\t"),
        "Synonym",
        &split_meta_camel_case(name),
    );
    lines.push(format!("{indent}\t\t<Comment/>"));
    emit_meta_value_type(lines, &format!("{indent}\t\t"), "Boolean");
    for line in [
        "<PasswordMode>false</PasswordMode>",
        "<Format/>",
        "<EditFormat/>",
        "<ToolTip/>",
        "<MarkNegatives>false</MarkNegatives>",
        "<Mask/>",
        "<MultiLine>false</MultiLine>",
        "<ExtendedEdit>false</ExtendedEdit>",
        "<MinValue xsi:nil=\"true\"/>",
        "<MaxValue xsi:nil=\"true\"/>",
        "<FillFromFillingValue>false</FillFromFillingValue>",
        "<FillValue xsi:nil=\"true\"/>",
        "<FillChecking>DontCheck</FillChecking>",
        "<ChoiceFoldersAndItems>Items</ChoiceFoldersAndItems>",
        "<ChoiceParameterLinks/>",
        "<ChoiceParameters/>",
        "<QuickChoice>Auto</QuickChoice>",
        "<CreateOnInput>Auto</CreateOnInput>",
        "<ChoiceForm/>",
        "<LinkByType/>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
        "<DataHistory>Use</DataHistory>",
    ] {
        lines.push(format!("{indent}\t\t{line}"));
    }
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</{tag}>"));
}

pub(super) fn emit_meta_addressing_attribute<F>(
    lines: &mut Vec<String>,
    indent: &str,
    value: &Value,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    let attr = meta_compile_parse_attr(value);
    let object = value.as_object();
    lines.push(format!(
        "{indent}<AddressingAttribute uuid=\"{}\">",
        next_uuid()
    ));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!(
        "{indent}\t\t<Name>{}</Name>",
        escape_xml(&attr.name)
    ));
    emit_meta_mltext(lines, &format!("{indent}\t\t"), "Synonym", &attr.synonym);
    lines.push(format!("{indent}\t\t<Comment/>"));
    if attr.type_name.is_empty() {
        emit_meta_value_type(lines, &format!("{indent}\t\t"), "String");
    } else {
        emit_meta_value_type(lines, &format!("{indent}\t\t"), &attr.type_name);
    }
    emit_meta_optional_text(
        lines,
        &format!("{indent}\t\t"),
        "AddressingDimension",
        object
            .and_then(|object| object.get("addressingDimension"))
            .and_then(Value::as_str),
    );
    let indexing = object
        .and_then(|object| object.get("indexing"))
        .and_then(Value::as_str)
        .unwrap_or("Index");
    lines.push(format!(
        "{indent}\t\t<Indexing>{}</Indexing>",
        escape_xml(indexing)
    ));
    lines.push(format!("{indent}\t\t<FullTextSearch>Use</FullTextSearch>"));
    lines.push(format!("{indent}\t\t<DataHistory>Use</DataHistory>"));
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</AddressingAttribute>"));
}

pub(super) fn emit_meta_column<F>(
    lines: &mut Vec<String>,
    indent: &str,
    value: &Value,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    let object = value.as_object();
    let name = value
        .as_str()
        .or_else(|| {
            object
                .and_then(|object| object.get("name"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default();
    let synonym = object
        .and_then(|object| object.get("synonym"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| split_meta_camel_case(name));
    let indexing = object
        .and_then(|object| object.get("indexing"))
        .and_then(Value::as_str)
        .unwrap_or("DontIndex");
    let references = object
        .and_then(|object| object.get("references"))
        .map(|value| meta_compile_string_list(Some(value)))
        .unwrap_or_default();
    lines.push(format!("{indent}<Column uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!("{indent}\t\t<Name>{}</Name>", escape_xml(name)));
    emit_meta_mltext(lines, &format!("{indent}\t\t"), "Synonym", &synonym);
    lines.push(format!("{indent}\t\t<Comment/>"));
    lines.push(format!(
        "{indent}\t\t<Indexing>{}</Indexing>",
        escape_xml(indexing)
    ));
    emit_meta_md_object_refs(lines, &format!("{indent}\t\t"), "References", &references);
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</Column>"));
}

pub(super) fn emit_meta_url_template<F>(
    lines: &mut Vec<String>,
    indent: &str,
    name: &str,
    value: &Value,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    let object = value.as_object();
    let template = value
        .as_str()
        .or_else(|| {
            object
                .and_then(|object| object.get("template"))
                .and_then(Value::as_str)
        })
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("/{}", name.to_lowercase()));
    lines.push(format!("{indent}<URLTemplate uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!("{indent}\t\t<Name>{}</Name>", escape_xml(name)));
    emit_meta_mltext(
        lines,
        &format!("{indent}\t\t"),
        "Synonym",
        &split_meta_camel_case(name),
    );
    lines.push(format!("{indent}\t\t<Comment/>"));
    lines.push(format!(
        "{indent}\t\t<Template>{}</Template>",
        escape_xml(&template)
    ));
    lines.push(format!("{indent}\t</Properties>"));
    let methods = object
        .and_then(|object| object.get("methods"))
        .and_then(Value::as_object);
    if methods.is_none_or(Map::is_empty) {
        lines.push(format!("{indent}\t<ChildObjects/>"));
    } else {
        lines.push(format!("{indent}\t<ChildObjects>"));
        let mut ordered = methods.unwrap().iter().collect::<Vec<_>>();
        ordered.sort_by(|left, right| left.0.cmp(right.0));
        for (method_name, http_method_value) in ordered {
            let http_method = http_method_value.as_str().unwrap_or("GET");
            emit_meta_http_method(
                lines,
                &format!("{indent}\t\t"),
                name,
                method_name,
                http_method,
                next_uuid,
            );
        }
        lines.push(format!("{indent}\t</ChildObjects>"));
    }
    lines.push(format!("{indent}</URLTemplate>"));
}

pub(super) fn emit_meta_http_method<F>(
    lines: &mut Vec<String>,
    indent: &str,
    template_name: &str,
    method_name: &str,
    http_method: &str,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    lines.push(format!("{indent}<Method uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!(
        "{indent}\t\t<Name>{}</Name>",
        escape_xml(method_name)
    ));
    emit_meta_mltext(
        lines,
        &format!("{indent}\t\t"),
        "Synonym",
        &split_meta_camel_case(method_name),
    );
    lines.push(format!("{indent}\t\t<Comment/>"));
    lines.push(format!(
        "{indent}\t\t<HTTPMethod>{}</HTTPMethod>",
        escape_xml(http_method)
    ));
    lines.push(format!(
        "{indent}\t\t<Handler>{}{}</Handler>",
        escape_xml(template_name),
        escape_xml(method_name)
    ));
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</Method>"));
}

pub(super) fn emit_meta_operation<F>(
    lines: &mut Vec<String>,
    indent: &str,
    name: &str,
    value: &Value,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    let object = value.as_object();
    let return_type = value
        .as_str()
        .or_else(|| {
            object
                .and_then(|object| object.get("returnType"))
                .and_then(Value::as_str)
        })
        .unwrap_or("xs:string");
    let nillable = object
        .and_then(|object| object.get("nillable"))
        .and_then(Value::as_bool)
        == Some(true);
    let transactioned = object
        .and_then(|object| object.get("transactioned"))
        .and_then(Value::as_bool)
        == Some(true);
    let handler = object
        .and_then(|object| object.get("handler"))
        .and_then(Value::as_str)
        .unwrap_or(name);
    lines.push(format!("{indent}<Operation uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!("{indent}\t\t<Name>{}</Name>", escape_xml(name)));
    emit_meta_mltext(
        lines,
        &format!("{indent}\t\t"),
        "Synonym",
        &split_meta_camel_case(name),
    );
    lines.push(format!("{indent}\t\t<Comment/>"));
    lines.push(format!(
        "{indent}\t\t<XDTOReturningValueType>{}</XDTOReturningValueType>",
        escape_xml(return_type)
    ));
    lines.push(format!("{indent}\t\t<Nillable>{nillable}</Nillable>"));
    lines.push(format!(
        "{indent}\t\t<Transactioned>{transactioned}</Transactioned>"
    ));
    lines.push(format!(
        "{indent}\t\t<ProcedureName>{}</ProcedureName>",
        escape_xml(handler)
    ));
    lines.push(format!(
        "{indent}\t\t<DataLockControlMode>Managed</DataLockControlMode>"
    ));
    lines.push(format!("{indent}\t</Properties>"));
    let parameters = object
        .and_then(|object| object.get("parameters"))
        .and_then(Value::as_object);
    if parameters.is_none_or(Map::is_empty) {
        lines.push(format!("{indent}\t<ChildObjects/>"));
    } else {
        lines.push(format!("{indent}\t<ChildObjects>"));
        let mut ordered = parameters.unwrap().iter().collect::<Vec<_>>();
        ordered.sort_by(|left, right| left.0.cmp(right.0));
        for (param_name, param_value) in ordered {
            emit_meta_operation_parameter(
                lines,
                &format!("{indent}\t\t"),
                param_name,
                param_value,
                next_uuid,
            );
        }
        lines.push(format!("{indent}\t</ChildObjects>"));
    }
    lines.push(format!("{indent}</Operation>"));
}

pub(super) fn emit_meta_operation_parameter<F>(
    lines: &mut Vec<String>,
    indent: &str,
    name: &str,
    value: &Value,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    let object = value.as_object();
    let value_type = value
        .as_str()
        .or_else(|| {
            object
                .and_then(|object| object.get("type"))
                .and_then(Value::as_str)
        })
        .unwrap_or("xs:string");
    let nillable = object
        .and_then(|object| object.get("nillable"))
        .and_then(Value::as_bool)
        != Some(false);
    let direction = object
        .and_then(|object| object.get("direction"))
        .and_then(Value::as_str)
        .unwrap_or("In");
    lines.push(format!("{indent}<Parameter uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!("{indent}\t\t<Name>{}</Name>", escape_xml(name)));
    emit_meta_mltext(
        lines,
        &format!("{indent}\t\t"),
        "Synonym",
        &split_meta_camel_case(name),
    );
    lines.push(format!("{indent}\t\t<Comment/>"));
    lines.push(format!(
        "{indent}\t\t<XDTOValueType>{}</XDTOValueType>",
        escape_xml(value_type)
    ));
    lines.push(format!("{indent}\t\t<Nillable>{nillable}</Nillable>"));
    lines.push(format!(
        "{indent}\t\t<TransferDirection>{}</TransferDirection>",
        escape_xml(direction)
    ));
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</Parameter>"));
}

pub(super) fn meta_enum_prop(
    defn: &MetaTemplateDefinition<'_>,
    field_name: &str,
    default: &str,
) -> String {
    defn.get(field_name)
        .and_then(Value::as_str)
        .map(normalize_meta_enum_value)
        .map(|value| escape_xml(&value))
        .unwrap_or_else(|| escape_xml(default))
}

pub(super) fn normalize_meta_enum_value(value: &str) -> String {
    match value {
        // Keep old DSL requests readable while emitting only the platform enum value.
        "HierarchyItemsOnly" => "HierarchyOfItems",
        "Balances" => "Balance",
        "Остатки" => "Balance",
        "Обороты" => "Turnovers",
        "None" => "Nonperiodical",
        "Daily" => "Day",
        "Monthly" => "Month",
        "Quarterly" => "Quarter",
        "Yearly" => "Year",
        "Непериодический" => "Nonperiodical",
        "Секунда" => "Second",
        "День" => "Day",
        "Месяц" => "Month",
        "Квартал" => "Quarter",
        "Год" => "Year",
        "ПозицияРегистратора" => "RecorderPosition",
        "RecordSubordinate" | "Subordinate" | "ПодчинениеРегистратору" => {
            "RecorderSubordinate"
        }
        "Независимый" => "Independent",
        "NotDependOnCalculationTypes" | "NoDependence" | "NotUsed" => "DontUse",
        "Depend" | "ПоПериодуДействия" => "OnActionPeriod",
        "Автоматический" => "Automatic",
        "Управляемый" => "Managed",
        "Использовать" => "Use",
        "НеИспользовать" => "DontUse",
        "Разрешить" => "Allow",
        "Запретить" => "Deny",
        "ВВидеНаименования" => "AsDescription",
        "ВВидеКода" => "AsCode",
        "ВДиалоге" => "InDialog",
        "ВСписке" => "InList",
        "ОбаСпособа" => "BothWays",
        "НеПроверять" => "DontCheck",
        "Ошибка" => "ShowError",
        "НеИндексировать" => "DontIndex",
        "Индексировать" => "Index",
        "ИндексироватьСДопУпорядочиванием" => {
            "IndexWithAdditionalOrder"
        }
        other => other,
    }
    .to_string()
}

pub(super) fn emit_meta_standard_attributes(
    lines: &mut Vec<String>,
    indent: &str,
    object_type: &str,
) {
    let attrs = match object_type {
        "Catalog" => vec![
            "PredefinedDataName",
            "Predefined",
            "Ref",
            "DeletionMark",
            "IsFolder",
            "Owner",
            "Parent",
            "Description",
            "Code",
        ],
        "Document" => vec!["Posted", "Ref", "DeletionMark", "Date", "Number"],
        "Enum" => vec!["Order", "Ref"],
        "InformationRegister" => vec!["Active", "LineNumber", "Recorder", "Period"],
        "AccumulationRegister" => vec!["RecordType", "Active", "LineNumber", "Recorder", "Period"],
        "AccountingRegister" => vec![
            "Account",
            "RecordType",
            "Active",
            "LineNumber",
            "Recorder",
            "Period",
        ],
        "CalculationRegister" => vec![
            "RegistrationPeriod",
            "ReversingEntry",
            "Active",
            "EndOfBasePeriod",
            "BegOfBasePeriod",
            "EndOfActionPeriod",
            "BegOfActionPeriod",
            "ActionPeriod",
            "CalculationType",
            "LineNumber",
            "Recorder",
        ],
        "ChartOfAccounts" => vec![
            "PredefinedDataName",
            "Order",
            "OffBalance",
            "Type",
            "Description",
            "Code",
            "Parent",
            "Predefined",
            "DeletionMark",
            "Ref",
        ],
        "ChartOfCharacteristicTypes" => vec![
            "PredefinedDataName",
            "ValueType",
            "Description",
            "Code",
            "IsFolder",
            "Parent",
            "Predefined",
            "DeletionMark",
            "Ref",
        ],
        "ChartOfCalculationTypes" => vec![
            "PredefinedDataName",
            "Predefined",
            "Ref",
            "DeletionMark",
            "ActionPeriodIsBasic",
            "Description",
            "Code",
        ],
        "BusinessProcess" => vec![
            "Started",
            "HeadTask",
            "Completed",
            "Ref",
            "DeletionMark",
            "Date",
            "Number",
        ],
        "Task" => vec![
            "Executed",
            "Description",
            "RoutePoint",
            "BusinessProcess",
            "Ref",
            "DeletionMark",
            "Date",
            "Number",
        ],
        "ExchangePlan" => vec![
            "ExchangeDate",
            "ThisNode",
            "ReceivedNo",
            "SentNo",
            "Ref",
            "DeletionMark",
            "Description",
            "Code",
        ],
        "DocumentJournal" => vec!["Type", "Ref", "Date", "Posted", "DeletionMark", "Number"],
        "TabularSection" => vec!["LineNumber"],
        _ => Vec::new(),
    };
    if attrs.is_empty() {
        return;
    }
    lines.push(format!("{indent}<StandardAttributes>"));
    for attr in attrs {
        emit_meta_standard_attribute(lines, &format!("{indent}\t"), object_type, attr);
    }
    lines.push(format!("{indent}</StandardAttributes>"));
}

pub(super) fn meta_standard_attribute_type_reduction_mode(
    object_type: &str,
    attr_name: &str,
) -> Option<&'static str> {
    if object_type == "Catalog" && attr_name == "Owner" {
        Some("Deny")
    } else {
        Some("TransformValues")
    }
}

pub(super) fn emit_meta_standard_attribute(
    lines: &mut Vec<String>,
    indent: &str,
    object_type: &str,
    attr_name: &str,
) {
    lines.push(format!(
        "{indent}<xr:StandardAttribute name=\"{}\">",
        escape_xml(attr_name)
    ));
    for line in [
        "<xr:LinkByType/>",
        "<xr:FillChecking>DontCheck</xr:FillChecking>",
        "<xr:MultiLine>false</xr:MultiLine>",
        "<xr:FillFromFillingValue>false</xr:FillFromFillingValue>",
        "<xr:CreateOnInput>Auto</xr:CreateOnInput>",
    ] {
        lines.push(format!("{indent}\t{line}"));
    }
    if let Some(mode) = meta_standard_attribute_type_reduction_mode(object_type, attr_name) {
        lines.push(format!(
            "{indent}\t<xr:TypeReductionMode>{}</xr:TypeReductionMode>",
            escape_xml(mode)
        ));
    }
    for line in [
        "<xr:MaxValue xsi:nil=\"true\"/>",
        "<xr:ToolTip/>",
        "<xr:ExtendedEdit>false</xr:ExtendedEdit>",
        "<xr:Format/>",
        "<xr:ChoiceForm/>",
        "<xr:QuickChoice>Auto</xr:QuickChoice>",
        "<xr:ChoiceHistoryOnInput>Auto</xr:ChoiceHistoryOnInput>",
        "<xr:EditFormat/>",
        "<xr:PasswordMode>false</xr:PasswordMode>",
        "<xr:DataHistory>Use</xr:DataHistory>",
        "<xr:MarkNegatives>false</xr:MarkNegatives>",
        "<xr:MinValue xsi:nil=\"true\"/>",
        "<xr:Synonym/>",
        "<xr:Comment/>",
        "<xr:FullTextSearch>Use</xr:FullTextSearch>",
        "<xr:ChoiceParameterLinks/>",
        "<xr:FillValue xsi:nil=\"true\"/>",
        "<xr:Mask/>",
        "<xr:ChoiceParameters/>",
    ] {
        lines.push(format!("{indent}\t{line}"));
    }
    lines.push(format!("{indent}</xr:StandardAttribute>"));
}

#[derive(Clone)]
pub(super) struct MetaCompileAttr {
    pub(crate) name: String,
    pub(crate) type_name: String,
    pub(crate) synonym: String,
    pub(crate) flags: Vec<String>,
    pub(crate) fill_checking: String,
    pub(crate) indexing: String,
    pub(crate) multi_line: bool,
    pub(crate) choice_history_on_input: String,
}

pub(super) struct MetaCompileTabularSection {
    pub(crate) name: String,
    pub(crate) columns: Vec<MetaCompileAttr>,
}

pub(super) fn meta_compile_attributes(value: Option<&Value>) -> Vec<MetaCompileAttr> {
    let Some(value) = value else {
        return Vec::new();
    };
    if let Some(object) = value.as_object() {
        return object
            .iter()
            .map(|(key, value)| {
                meta_compile_parse_attr(&Value::String(format!(
                    "{key}:{}",
                    json_value_to_python_string(value)
                )))
            })
            .collect();
    }
    value
        .as_array()
        .map(|items| items.iter().map(meta_compile_parse_attr).collect())
        .unwrap_or_default()
}

pub(super) fn meta_compile_tabular_sections(
    value: Option<&Value>,
) -> Result<Vec<MetaCompileTabularSection>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    if let Some(items) = value.as_array() {
        for item in items {
            let object = item
                .as_object()
                .ok_or_else(|| "tabular section must be an object".to_string())?;
            let name = object
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "tabular section is missing name".to_string())?
                .to_string();
            result.push(MetaCompileTabularSection {
                name,
                columns: meta_compile_attributes(object.get("attributes")),
            });
        }
    } else if let Some(object) = value.as_object() {
        for (name, columns) in object {
            result.push(MetaCompileTabularSection {
                name: name.to_string(),
                columns: meta_compile_attributes(Some(columns)),
            });
        }
    }
    Ok(result)
}

pub(super) fn meta_compile_parse_attr(value: &Value) -> MetaCompileAttr {
    if let Some(text) = value.as_str() {
        let mut pieces = text.splitn(2, '|');
        let main = pieces.next().unwrap_or_default().trim();
        let flags = pieces
            .next()
            .map(|part| {
                part.split(',')
                    .map(str::trim)
                    .filter(|flag| !flag.is_empty())
                    .map(|flag| flag.to_lowercase())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut colon = main.splitn(2, ':');
        let name = colon.next().unwrap_or_default().trim().to_string();
        let type_name = colon.next().unwrap_or_default().trim().to_string();
        let synonym = split_meta_camel_case(&name);
        return MetaCompileAttr {
            name,
            type_name,
            synonym,
            flags,
            fill_checking: String::new(),
            indexing: String::new(),
            multi_line: false,
            choice_history_on_input: String::new(),
        };
    }
    let object = value.as_object();
    let name = object
        .and_then(|object| object.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let type_name = object.map(meta_compile_build_type).unwrap_or_default();
    let synonym = object
        .and_then(|object| object.get("synonym"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| split_meta_camel_case(&name));
    let flags = object
        .and_then(|object| object.get("flags"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    MetaCompileAttr {
        name,
        type_name,
        synonym,
        flags,
        fill_checking: object
            .and_then(|object| object.get("fillChecking"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        indexing: object
            .and_then(|object| object.get("indexing"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        multi_line: object
            .and_then(|object| object.get("multiLine"))
            .and_then(Value::as_bool)
            == Some(true),
        choice_history_on_input: object
            .and_then(|object| object.get("choiceHistoryOnInput"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

pub(super) fn meta_compile_build_type(object: &Map<String, Value>) -> String {
    let mut type_name = object
        .get("valueType")
        .or_else(|| object.get("type"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if !type_name.is_empty() && !type_name.contains('(') {
        if type_name == "String" {
            if let Some(length) = object.get("length").and_then(json_i64_value) {
                type_name = format!("String({length})");
            }
        } else if type_name == "Number" {
            if let Some(length) = object.get("length").and_then(json_i64_value) {
                let precision = object
                    .get("precision")
                    .and_then(json_i64_value)
                    .unwrap_or(0);
                let nn = if object.get("nonneg").and_then(Value::as_bool) == Some(true)
                    || object.get("nonnegative").and_then(Value::as_bool) == Some(true)
                {
                    ",nonneg"
                } else {
                    ""
                };
                type_name = format!("Number({length},{precision}{nn})");
            }
        }
    }
    type_name
}

pub(super) fn emit_meta_attribute<F>(
    lines: &mut Vec<String>,
    indent: &str,
    attr: &MetaCompileAttr,
    context: &str,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    lines.push(format!("{indent}<Attribute uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!(
        "{indent}\t\t<Name>{}</Name>",
        escape_xml(&attr.name)
    ));
    emit_meta_mltext(lines, &format!("{indent}\t\t"), "Synonym", &attr.synonym);
    lines.push(format!("{indent}\t\t<Comment/>"));
    if attr.type_name.is_empty() {
        lines.push(format!("{indent}\t\t<Type>"));
        lines.push(format!("{indent}\t\t\t<v8:Type>xs:string</v8:Type>"));
        lines.push(format!("{indent}\t\t</Type>"));
    } else {
        emit_meta_value_type(lines, &format!("{indent}\t\t"), &attr.type_name);
    }
    lines.push(format!("{indent}\t\t<PasswordMode>false</PasswordMode>"));
    lines.push(format!("{indent}\t\t<Format/>"));
    lines.push(format!("{indent}\t\t<EditFormat/>"));
    lines.push(format!("{indent}\t\t<ToolTip/>"));
    lines.push(format!("{indent}\t\t<MarkNegatives>false</MarkNegatives>"));
    lines.push(format!("{indent}\t\t<Mask/>"));
    let multi_line = attr.multi_line || attr.flags.iter().any(|flag| flag == "multiline");
    lines.push(format!("{indent}\t\t<MultiLine>{multi_line}</MultiLine>"));
    lines.push(format!("{indent}\t\t<ExtendedEdit>false</ExtendedEdit>"));
    lines.push(format!("{indent}\t\t<MinValue xsi:nil=\"true\"/>"));
    lines.push(format!("{indent}\t\t<MaxValue xsi:nil=\"true\"/>"));
    if !matches!(
        context,
        "tabular" | "processor" | "chart" | "register-other"
    ) {
        lines.push(format!(
            "{indent}\t\t<FillFromFillingValue>false</FillFromFillingValue>"
        ));
    }
    if !matches!(
        context,
        "tabular" | "processor" | "chart" | "register-other"
    ) {
        emit_meta_fill_value(lines, &format!("{indent}\t\t"), &attr.type_name);
    }
    let fill_checking = if !attr.fill_checking.is_empty() {
        attr.fill_checking.as_str()
    } else if attr.flags.iter().any(|flag| flag == "req") {
        "ShowError"
    } else {
        "DontCheck"
    };
    lines.push(format!(
        "{indent}\t\t<FillChecking>{}</FillChecking>",
        escape_xml(fill_checking)
    ));
    for line in [
        "<ChoiceFoldersAndItems>Items</ChoiceFoldersAndItems>",
        "<ChoiceParameterLinks/>",
        "<ChoiceParameters/>",
        "<QuickChoice>Auto</QuickChoice>",
        "<CreateOnInput>Auto</CreateOnInput>",
        "<ChoiceForm/>",
        "<LinkByType/>",
    ] {
        lines.push(format!("{indent}\t\t{line}"));
    }
    let choice_history_on_input = if attr.choice_history_on_input.is_empty() {
        "Auto"
    } else {
        attr.choice_history_on_input.as_str()
    };
    lines.push(format!(
        "{indent}\t\t<ChoiceHistoryOnInput>{}</ChoiceHistoryOnInput>",
        escape_xml(choice_history_on_input)
    ));
    if context == "catalog" {
        lines.push(format!("{indent}\t\t<Use>ForItem</Use>"));
    }
    if !matches!(context, "processor" | "processor-tabular") {
        let indexing = if !attr.indexing.is_empty() {
            attr.indexing.as_str()
        } else if attr.flags.iter().any(|flag| flag == "indexadditional") {
            "IndexWithAdditionalOrder"
        } else if attr.flags.iter().any(|flag| flag == "index") {
            "Index"
        } else {
            "DontIndex"
        };
        lines.push(format!(
            "{indent}\t\t<Indexing>{}</Indexing>",
            escape_xml(indexing)
        ));
        lines.push(format!("{indent}\t\t<FullTextSearch>Use</FullTextSearch>"));
        if !matches!(context, "chart" | "register-other") {
            lines.push(format!("{indent}\t\t<DataHistory>Use</DataHistory>"));
        }
    }
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</Attribute>"));
}

pub(super) fn emit_meta_tabular_section<F>(
    lines: &mut Vec<String>,
    indent: &str,
    section: &MetaCompileTabularSection,
    object_type: &str,
    object_name: &str,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    lines.push(format!("{indent}<TabularSection uuid=\"{}\">", next_uuid()));
    let type_prefix = format!("{object_type}TabularSection");
    let row_prefix = format!("{object_type}TabularSectionRow");
    let generated_type_name = escape_xml(&format!("{type_prefix}.{object_name}.{}", section.name));
    let generated_row_name = escape_xml(&format!("{row_prefix}.{object_name}.{}", section.name));
    lines.push(format!("{indent}\t<InternalInfo>"));
    lines.push(format!(
        "{indent}\t\t<xr:GeneratedType name=\"{generated_type_name}\" category=\"TabularSection\">"
    ));
    lines.push(format!(
        "{indent}\t\t\t<xr:TypeId>{}</xr:TypeId>",
        next_uuid()
    ));
    lines.push(format!(
        "{indent}\t\t\t<xr:ValueId>{}</xr:ValueId>",
        next_uuid()
    ));
    lines.push(format!("{indent}\t\t</xr:GeneratedType>"));
    lines.push(format!(
        "{indent}\t\t<xr:GeneratedType name=\"{generated_row_name}\" category=\"TabularSectionRow\">"
    ));
    lines.push(format!(
        "{indent}\t\t\t<xr:TypeId>{}</xr:TypeId>",
        next_uuid()
    ));
    lines.push(format!(
        "{indent}\t\t\t<xr:ValueId>{}</xr:ValueId>",
        next_uuid()
    ));
    lines.push(format!("{indent}\t\t</xr:GeneratedType>"));
    lines.push(format!("{indent}\t</InternalInfo>"));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!(
        "{indent}\t\t<Name>{}</Name>",
        escape_xml(&section.name)
    ));
    emit_meta_mltext(
        lines,
        &format!("{indent}\t\t"),
        "Synonym",
        &split_meta_camel_case(&section.name),
    );
    lines.push(format!("{indent}\t\t<Comment/>"));
    lines.push(format!("{indent}\t\t<ToolTip/>"));
    lines.push(format!(
        "{indent}\t\t<FillChecking>DontCheck</FillChecking>"
    ));
    emit_meta_standard_attributes(lines, &format!("{indent}\t\t"), "TabularSection");
    if meta_line_number_length_is_applicable(object_type) {
        lines.push(format!(
            "{indent}\t\t<LineNumberLength>9</LineNumberLength>"
        ));
    }
    if object_type == "Catalog" {
        lines.push(format!("{indent}\t\t<Use>ForItem</Use>"));
    }
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}\t<ChildObjects>"));
    let column_context = if matches!(object_type, "DataProcessor" | "Report") {
        "processor-tabular"
    } else {
        "tabular"
    };
    for column in &section.columns {
        emit_meta_attribute(
            lines,
            &format!("{indent}\t\t"),
            column,
            column_context,
            next_uuid,
        );
    }
    lines.push(format!("{indent}\t</ChildObjects>"));
    lines.push(format!("{indent}</TabularSection>"));
}

pub(super) fn meta_line_number_length_is_applicable(object_type: &str) -> bool {
    !matches!(
        object_type,
        "Report" | "DataProcessor" | "ExternalReport" | "ExternalDataProcessor"
    )
}

pub(super) fn split_meta_camel_case(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    let mut result = String::new();
    let mut previous_lower = false;
    for ch in name.chars() {
        if previous_lower && ch.is_uppercase() {
            result.push(' ');
        }
        result.push(ch);
        previous_lower = ch.is_lowercase();
    }
    let mut chars = result.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first, chars.as_str().to_lowercase()),
        None => result,
    }
}

#[cfg(test)]
mod typed_template_tests {
    use super::*;

    fn context() -> MinimalTemplateContext {
        MinimalTemplateContext {
            chart_of_accounts: Some("ChartOfAccounts.MetaAddAccounts".to_string()),
            chart_of_calculation_types: Some(
                "ChartOfCalculationTypes.MetaAddCalculationTypes".to_string(),
            ),
            method_name: Some("CommonModule.MetaAddHandlers.Run".to_string()),
            event_source: Some("CatalogRef.MetaAddSource".to_string()),
            event_handler: Some("CommonModule.MetaAddHandlers.Handle".to_string()),
            dependencies: Vec::new(),
        }
    }

    #[test]
    fn typed_minimal_catalog_emits_parseable_platform_xml_for_every_metadata_kind() {
        let context = context();
        for kind in MetadataKind::ALL {
            let name = format!("Typed{}", kind.as_str());
            let (xml, uuid) = minimal_metadata_xml(*kind, &name, "2.20", &context)
                .unwrap_or_else(|error| panic!("{}: {error}", kind.as_str()));
            let document =
                Document::parse(&xml).unwrap_or_else(|error| panic!("{}: {error}", kind.as_str()));
            let object = document
                .root_element()
                .children()
                .find(|node| node.is_element())
                .expect("metadata object");
            assert_eq!(object.tag_name().name(), kind.as_str());
            assert_eq!(object.attribute("uuid"), Some(uuid.as_str()));
            assert!(object.descendants().any(|node| {
                node.is_element()
                    && node.tag_name().name() == "Name"
                    && node.text() == Some(name.as_str())
            }));
            let generated_types = metadata_generated_types_8_3_27(kind.as_str()).unwrap();
            assert_eq!(
                object
                    .children()
                    .any(|node| node.is_element() && node.tag_name().name() == "InternalInfo"),
                !generated_types.is_empty(),
                "{}",
                kind.as_str()
            );
        }
    }

    #[test]
    fn typed_minimal_catalog_declares_mandatory_modules_and_auxiliary_resources() {
        assert_eq!(
            minimal_module_files(MetadataKind::Catalog),
            &["ObjectModule.bsl"]
        );
        assert_eq!(
            minimal_module_files(MetadataKind::Constant),
            &["ManagerModule.bsl", "ValueManagerModule.bsl"]
        );
        assert_eq!(
            minimal_auxiliary_files(MetadataKind::ExchangePlan, "2.20")[0].0,
            "Content.xml"
        );
        assert_eq!(
            minimal_auxiliary_files(MetadataKind::BusinessProcess, "2.20")[0].0,
            "Flowchart.xml"
        );
    }

    #[test]
    fn typed_minimal_catalog_selects_only_exported_procedures_with_required_arity() {
        let module = concat!(
            "Function Ignore() Export\nEndFunction\n",
            "PROCEDURE Run() EXPORT\nEndProcedure\n",
            "Процедура Обработать(Источник, Отказ) Экспорт\nКонецПроцедуры\n",
            "Procedure Private(One, Two)\nEndProcedure\n",
        );

        assert_eq!(first_exported_procedure(module, 0).as_deref(), Some("Run"));
        assert_eq!(
            first_exported_procedure(module, 2).as_deref(),
            Some("Обработать")
        );
        assert_eq!(first_exported_procedure(module, 1), None);
    }
}
