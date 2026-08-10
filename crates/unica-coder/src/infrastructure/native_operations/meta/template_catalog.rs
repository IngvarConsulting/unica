use super::super::common::escape_xml;
use super::super::common::is_1c_identifier;
use super::publisher::fresh_metadata_uuid;
use super::xml_model::{
    emit_meta_mltext, emit_metadata_xml_type_contents, emit_metadata_xml_value_type,
    MetadataXmlType,
};
use crate::application::metadata::MetaFailure;
use crate::application::ports::MetadataAuxiliaryXmlKind;
use crate::domain::metadata::{MetaDiagnostic, MetaDiagnosticCode, MetadataKind};
use crate::domain::source_target::{
    MetadataAddress, SourceTarget, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bsl_outline::first_exported_bsl_procedure;
use crate::infrastructure::metadata_kinds::metadata_layout;
use crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point;
use crate::infrastructure::platform_xml_source_targets::{
    platform_xml_resource_evidence, resolve_platform_xml_target, ClosedPlatformXmlTarget,
    ResolvedSourceSet, TargetKindPolicy,
};
use roxmltree::Document;
use std::path::{Component, Path, PathBuf};

pub(crate) trait MetadataTemplateCatalog {
    fn minimal_object(
        &self,
        source: &ResolvedSourceSet,
        kind: MetadataKind,
        name: &str,
        overrides: MetadataTemplateOperationOverrides,
        source_set: &str,
        workspace: &WorkspaceContext,
    ) -> Result<MetadataPostImage, MetaFailure>;
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MetadataTemplateOperationOverrides {
    pub(crate) source: bool,
    pub(crate) handler: bool,
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
    pub(crate) guard_handle: Option<ClosedPlatformXmlTarget>,
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

pub(super) struct MinimalTemplateContext {
    pub(super) chart_of_accounts: Option<String>,
    pub(super) chart_of_calculation_types: Option<String>,
    pub(super) task: Option<String>,
    pub(super) registered_documents: Vec<String>,
    pub(super) method_name: Option<String>,
    pub(super) event_source: Option<String>,
    pub(super) event_handler: Option<String>,
    pub(super) dependencies: Vec<MetadataTemplateFile>,
}

impl MinimalTemplateContext {
    fn from_source(
        source: &ResolvedSourceSet,
        kind: MetadataKind,
        new_name: &str,
        overrides: MetadataTemplateOperationOverrides,
        source_set: &str,
        workspace: &WorkspaceContext,
    ) -> Result<Self, MetaFailure> {
        let mut context = Self {
            chart_of_accounts: None,
            task: None,
            registered_documents: Vec::new(),
            chart_of_calculation_types: None,
            method_name: None,
            event_source: None,
            event_handler: None,
            dependencies: Vec::new(),
        };
        match kind {
            MetadataKind::AccumulationRegister => {
                context.dependencies.push(registrar_dependency(
                    source,
                    &format!("AccumulationRegister.{new_name}"),
                    kind,
                )?);
            }
            MetadataKind::BusinessProcess => {
                let task = required_registered_name(source, "Task", kind)?;
                context.task = Some(format!("Task.{task}"));
                context.dependencies.push(read_dependency(
                    source,
                    "Task",
                    &task,
                    MetadataTemplateFileMode::Guard,
                    None,
                )?);
            }
            MetadataKind::DocumentJournal => {
                let document = required_registered_name(source, "Document", kind)?;
                context
                    .registered_documents
                    .push(format!("Document.{document}"));
                context.dependencies.push(read_dependency(
                    source,
                    "Document",
                    &document,
                    MetadataTemplateFileMode::Guard,
                    None,
                )?);
            }
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
                    required_common_module_method(source, source_set, workspace, kind, 0)?;
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
                if !overrides.handler {
                    let (module, method, module_guard) =
                        required_common_module_method(source, source_set, workspace, kind, 2)?;
                    context.event_handler = Some(format!("CommonModule.{module}.{method}"));
                    context.dependencies.push(read_dependency(
                        source,
                        "CommonModule",
                        &module,
                        MetadataTemplateFileMode::Guard,
                        None,
                    )?);
                    context.dependencies.push(module_guard);
                }
                if !overrides.source {
                    let catalog = required_registered_name(source, "Catalog", kind)?;
                    context.event_source = Some(format!("CatalogObject.{catalog}"));
                    context.dependencies.push(read_dependency(
                        source,
                        "Catalog",
                        &catalog,
                        MetadataTemplateFileMode::Guard,
                        None,
                    )?);
                }
            }
            _ => {}
        }
        Ok(context)
    }
}

fn required_common_module_method(
    source: &ResolvedSourceSet,
    source_set: &str,
    workspace: &WorkspaceContext,
    requested: MetadataKind,
    arity: usize,
) -> Result<(String, String, MetadataTemplateFile), MetaFailure> {
    for module in registered_names(&source.owner_preimage, "CommonModule") {
        let relative_path = PathBuf::from("CommonModules")
            .join(&module)
            .join("Ext/Module.bsl");
        let Ok(module_address) = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("CommonModule.{module}.Module"),
        ) else {
            continue;
        };
        let target = SourceTarget {
            source_set: source_set.to_string(),
            metadata_path: Some(module_address),
        };
        let Ok(resolution) =
            resolve_platform_xml_target(workspace, &target, TargetKindPolicy::ModuleOnly)
        else {
            continue;
        };
        if resolution.resolved.target_kind != TargetKind::Module {
            continue;
        }
        let Ok(evidence) = platform_xml_resource_evidence(workspace, &resolution.handle) else {
            continue;
        };
        if evidence.source_root != source.source_root
            || evidence.registration_path != source.owner_path
            || evidence.target_path != source.source_root.join(&relative_path)
        {
            continue;
        }
        let Ok(bytes) = read_contained_regular_file(&source.source_root, &relative_path) else {
            continue;
        };
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let Ok(Some(method)) =
            first_exported_bsl_procedure(text.trim_start_matches('\u{feff}'), arity)
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
                guard_handle: Some(resolution.handle),
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
    let preimage =
        read_contained_regular_file(&source.source_root, &relative_path).map_err(|_| {
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
        guard_handle: None,
    })
}

/// Read a prerequisite only through the exact selected source-root topology.
/// `fs::read` follows linked ancestors, which would let a registered metadata
/// name redirect template discovery outside its source set before the
/// transaction has a chance to bind the dependency.
fn read_contained_regular_file(source_root: &Path, relative_path: &Path) -> Result<Vec<u8>, ()> {
    if relative_path.is_absolute() {
        return Err(());
    }

    let mut current = source_root.to_path_buf();
    let mut components = relative_path.components().peekable();
    while let Some(component) = components.next() {
        let Component::Normal(component) = component else {
            return Err(());
        };
        current.push(component);
        let metadata = std::fs::symlink_metadata(&current).map_err(|_| ())?;
        if metadata_is_link_or_reparse_point(&metadata) {
            return Err(());
        }
        if components.peek().is_some() {
            if !metadata.is_dir() {
                return Err(());
            }
        } else if !metadata.is_file() {
            return Err(());
        }
    }

    std::fs::read(current).map_err(|_| ())
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
    let empty = ["<RegisterRecords/>", "<RegisterRecords />"]
        .into_iter()
        .find(|tag| source.contains(tag));
    let updated = if let Some(tag) = empty {
        source.replacen(
            tag,
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
        overrides: MetadataTemplateOperationOverrides,
        source_set: &str,
        workspace: &WorkspaceContext,
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
        let context = MinimalTemplateContext::from_source(
            source, kind, name, overrides, source_set, workspace,
        )?;
        let (xml, _) = minimal_metadata_xml(kind, name, &source.format_version, &context)
            .map_err(|message| template_failure(&metadata_path, message))?;
        let layout = metadata_layout(kind);
        let mut files = vec![MetadataTemplateFile {
            role: MetadataTemplateFileRole::Descriptor,
            mode: MetadataTemplateFileMode::Create,
            relative_path: PathBuf::from(layout.directory).join(format!("{name}.xml")),
            bytes: with_utf8_bom(xml.as_bytes()),
            preimage: None,
            guard_handle: None,
        }];
        let ext = PathBuf::from(layout.directory).join(name).join("Ext");
        for module in minimal_module_files(kind) {
            files.push(MetadataTemplateFile {
                role: MetadataTemplateFileRole::Module,
                mode: MetadataTemplateFileMode::Create,
                relative_path: ext.join(module),
                bytes: with_utf8_bom(&[]),
                preimage: None,
                guard_handle: None,
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
                guard_handle: None,
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

pub(super) struct MetaTemplateDefinition {
    chart_of_accounts: Option<String>,
    chart_of_calculation_types: Option<String>,
    task: Option<String>,
    registered_documents: Vec<String>,
    method_name: Option<String>,
    sources: Vec<String>,
    handler: Option<String>,
}

fn emit_meta_catalog_xml(
    defn: &MetaTemplateDefinition,
    obj_name: &str,
    format_version: &str,
) -> Result<(String, String), String> {
    let mut next_uuid = fresh_metadata_uuid;
    let obj_uuid = next_uuid();
    let synonym = split_meta_camel_case(obj_name);

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

    lines.push("\t\t<ChildObjects/>".to_string());

    lines.push("\t</Catalog>".to_string());
    lines.push("</MetaDataObject>".to_string());
    Ok((format!("{}\n", lines.join("\n")), obj_uuid))
}

/// Whether 8.3.27 declares a `ChildObjects` collection for the kind.
///
/// The platform refuses the import outright when a childless kind carries the
/// element: `document format error: unexpected read property. Current property:
/// ChildObjects, expected property: <Kind>`. The set below is measured against a
/// real 8.3.27 dump, where the split is total — every kind either always carries
/// the element or never does — and confirmed by the exact platform gate.
fn kind_declares_child_objects(kind: crate::domain::metadata::MetadataKind) -> bool {
    use crate::domain::metadata::MetadataKind;
    !matches!(
        kind,
        MetadataKind::CommonModule
            | MetadataKind::Constant
            | MetadataKind::DefinedType
            | MetadataKind::EventSubscription
            | MetadataKind::ScheduledJob
    )
}

pub(super) fn minimal_metadata_xml(
    kind: crate::domain::metadata::MetadataKind,
    obj_name: &str,
    format_version: &str,
    context: &MinimalTemplateContext,
) -> Result<(String, String), String> {
    let defn = MetaTemplateDefinition {
        chart_of_accounts: context.chart_of_accounts.clone(),
        chart_of_calculation_types: context.chart_of_calculation_types.clone(),
        task: context.task.clone(),
        registered_documents: context.registered_documents.clone(),
        method_name: context.method_name.clone(),
        sources: context.event_source.clone().into_iter().collect(),
        handler: context.event_handler.clone(),
    };
    if kind == crate::domain::metadata::MetadataKind::Catalog {
        return emit_meta_catalog_xml(&defn, obj_name, format_version);
    }

    let object_type = kind.as_str();
    let mut next_uuid = fresh_metadata_uuid;
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
    // Содержимое объекта задаёт вызывающий через `operations`: инструмент не
    // придумывает ресурсы, измерения и значения свойств за него (ADR-0030).
    if kind_declares_child_objects(kind) {
        lines.push("\t\t<ChildObjects/>".to_string());
    }
    lines.push(format!("\t</{object_type}>"));
    lines.push("</MetaDataObject>".to_string());
    Ok((format!("{}\n", lines.join("\n")), obj_uuid))
}

#[cfg(test)]
pub(crate) fn minimal_metadata_xml_for_tests(
    kind: MetadataKind,
    name: &str,
) -> Result<(String, String), String> {
    minimal_metadata_xml(
        kind,
        name,
        "2.20",
        &MinimalTemplateContext {
            chart_of_accounts: None,
            chart_of_calculation_types: None,
            task: None,
            registered_documents: Vec::new(),
            method_name: None,
            event_source: None,
            event_handler: None,
            dependencies: Vec::new(),
        },
    )
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
    defn: &MetaTemplateDefinition,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    let hierarchical = false;
    lines.push(format!(
        "{indent}<Hierarchical>{hierarchical}</Hierarchical>"
    ));
    lines.push(format!(
        "{indent}<HierarchyType>{}</HierarchyType>",
        escape_xml("HierarchyFoldersAndItems")
    ));
    let limit_level_count = false;
    let level_count = 2;
    let folders_on_top = true;
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
        escape_xml("ToItems")
    ));
    let code_length = 9;
    let description_length = 25;
    lines.push(format!("{indent}<CodeLength>{code_length}</CodeLength>"));
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    lines.push(format!(
        "{indent}<CodeType>{}</CodeType>",
        escape_xml("String")
    ));
    lines.push(format!(
        "{indent}<CodeAllowedLength>{}</CodeAllowedLength>",
        escape_xml("Variable")
    ));
    lines.push(format!(
        "{indent}<CodeSeries>{}</CodeSeries>",
        escape_xml("WholeCatalog")
    ));
    let check_unique = false;
    let autonumbering = true;
    lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    lines.push(format!(
        "{indent}<Autonumbering>{autonumbering}</Autonumbering>"
    ));
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        escape_xml("AsDescription")
    ));
    emit_meta_standard_attributes(lines, indent, "Catalog");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!(
        "{indent}<PredefinedDataUpdate>Auto</PredefinedDataUpdate>"
    ));
    lines.push(format!("{indent}<EditType>InDialog</EditType>"));
    let quick_choice = false;
    lines.push(format!("{indent}<QuickChoice>{quick_choice}</QuickChoice>"));
    lines.push(format!(
        "{indent}<ChoiceMode>{}</ChoiceMode>",
        escape_xml("BothWays")
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

pub(super) fn emit_meta_base_properties(
    lines: &mut Vec<String>,
    indent: &str,
    _defn: &MetaTemplateDefinition,
    obj_name: &str,
    synonym: &str,
) {
    lines.push(format!("{indent}<Name>{}</Name>", escape_xml(obj_name)));
    emit_meta_mltext(lines, indent, "Synonym", synonym);
    lines.push(format!("{indent}<Comment/>"));
}

pub(super) fn emit_meta_enum_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition,
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
    defn: &MetaTemplateDefinition,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    emit_metadata_xml_value_type(lines, indent, &[MetadataXmlType::String { length: 10 }]);
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
        escape_xml("Automatic")
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
    defn: &MetaTemplateDefinition,
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
        escape_xml("String")
    ));
    let number_length = 11;
    lines.push(format!(
        "{indent}<NumberLength>{number_length}</NumberLength>"
    ));
    lines.push(format!(
        "{indent}<NumberAllowedLength>{}</NumberAllowedLength>",
        escape_xml("Variable")
    ));
    lines.push(format!(
        "{indent}<NumberPeriodicity>{}</NumberPeriodicity>",
        escape_xml("Year")
    ));
    let check_unique = true;
    let autonumbering = true;
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
        escape_xml("Allow")
    ));
    lines.push(format!(
        "{indent}<RealTimePosting>{}</RealTimePosting>",
        escape_xml("Deny")
    ));
    lines.push(format!(
        "{indent}<RegisterRecordsDeletion>{}</RegisterRecordsDeletion>",
        escape_xml("AutoDelete")
    ));
    lines.push(format!(
        "{indent}<RegisterRecordsWritingOnPost>{}</RegisterRecordsWritingOnPost>",
        escape_xml("WriteModified")
    ));
    lines.push(format!(
        "{indent}<SequenceFilling>{}</SequenceFilling>",
        escape_xml("AutoFill")
    ));
    emit_empty_meta_object_refs(lines, indent, "RegisterRecords");
    let post_in_privileged = true;
    let unpost_in_privileged = true;
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
    defn: &MetaTemplateDefinition,
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
    let periodicity = escape_xml("Nonperiodical");
    let write_mode = escape_xml("Independent");
    let main_filter_on_period = periodicity != "Nonperiodical";
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
        escape_xml("Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        escape_xml("Use")
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
    defn: &MetaTemplateDefinition,
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
        escape_xml("Balance")
    ));
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    emit_meta_standard_attributes(lines, indent, "AccumulationRegister");
    emit_meta_register_tail(lines, indent, defn);
    let enable_totals_splitting = true;
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
    defn: &MetaTemplateDefinition,
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
        defn.chart_of_accounts.as_deref(),
    );
    let correspondence = false;
    let period_adjustment_length = 0;
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
        escape_xml("Automatic")
    ));
    lines.push(format!(
        "{indent}<EnableTotalsSplitting>false</EnableTotalsSplitting>"
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        escape_xml("Use")
    ));
    lines.push(format!("{indent}<ListPresentation/>"));
    lines.push(format!("{indent}<ExtendedListPresentation/>"));
    lines.push(format!("{indent}<Explanation/>"));
}

pub(super) fn emit_meta_calculation_register_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition,
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
        escape_xml("Month")
    ));
    let action_period = false;
    let base_period = false;
    lines.push(format!(
        "{indent}<ActionPeriod>{action_period}</ActionPeriod>"
    ));
    lines.push(format!("{indent}<BasePeriod>{base_period}</BasePeriod>"));
    emit_meta_optional_text(lines, indent, "Schedule", None);
    emit_meta_optional_text(lines, indent, "ScheduleValue", None);
    emit_meta_optional_text(lines, indent, "ScheduleDate", None);
    emit_meta_optional_text(
        lines,
        indent,
        "ChartOfCalculationTypes",
        defn.chart_of_calculation_types.as_deref(),
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
    defn: &MetaTemplateDefinition,
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
    let ext_dimension_types = None;
    emit_meta_optional_text(lines, indent, "ExtDimensionTypes", ext_dimension_types);
    let max_ext_dimension_count = 0;
    lines.push(format!(
        "{indent}<MaxExtDimensionCount>{max_ext_dimension_count}</MaxExtDimensionCount>"
    ));
    emit_meta_optional_text(lines, indent, "CodeMask", None);
    let code_length = 8;
    let description_length = 120;
    lines.push(format!("{indent}<CodeLength>{code_length}</CodeLength>"));
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    lines.push(format!(
        "{indent}<CodeSeries>{}</CodeSeries>",
        escape_xml("WholeChartOfAccounts")
    ));
    let check_unique = false;
    lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        escape_xml("AsDescription")
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
    let auto_order_by_code = true;
    let order_length = 5;
    lines.push(format!(
        "{indent}<AutoOrderByCode>{auto_order_by_code}</AutoOrderByCode>"
    ));
    lines.push(format!("{indent}<OrderLength>{order_length}</OrderLength>"));
    lines.push(format!("{indent}<DataLockFields/>"));
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        escape_xml("Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        escape_xml("Use")
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
    defn: &MetaTemplateDefinition,
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
    emit_meta_optional_text(lines, indent, "CharacteristicExtValues", None);
    emit_metadata_xml_value_type(
        lines,
        indent,
        &[
            MetadataXmlType::Boolean,
            MetadataXmlType::String { length: 100 },
            MetadataXmlType::DateTime,
            MetadataXmlType::Number {
                digits: 15,
                fraction: 2,
            },
        ],
    );
    let hierarchical = false;
    lines.push(format!(
        "{indent}<Hierarchical>{hierarchical}</Hierarchical>"
    ));
    let folders_on_top = true;
    lines.push(format!(
        "{indent}<FoldersOnTop>{folders_on_top}</FoldersOnTop>"
    ));
    let code_length = 9;
    let description_length = 25;
    lines.push(format!("{indent}<CodeLength>{code_length}</CodeLength>"));
    lines.push(format!(
        "{indent}<CodeAllowedLength>{}</CodeAllowedLength>",
        escape_xml("Variable")
    ));
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    lines.push(format!(
        "{indent}<CodeSeries>{}</CodeSeries>",
        escape_xml("WholeCharacteristicKind")
    ));
    let check_unique = false;
    let autonumbering = true;
    lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    lines.push(format!(
        "{indent}<Autonumbering>{autonumbering}</Autonumbering>"
    ));
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        escape_xml("AsDescription")
    ));
    emit_meta_standard_attributes(lines, indent, "ChartOfCharacteristicTypes");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!(
        "{indent}<PredefinedDataUpdate>{}</PredefinedDataUpdate>",
        escape_xml("Auto")
    ));
    lines.push(format!(
        "{indent}<EditType>{}</EditType>",
        escape_xml("InDialog")
    ));
    let quick_choice = false;
    lines.push(format!("{indent}<QuickChoice>{quick_choice}</QuickChoice>"));
    lines.push(format!(
        "{indent}<ChoiceMode>{}</ChoiceMode>",
        escape_xml("BothWays")
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
        escape_xml("Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        escape_xml("Use")
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
    defn: &MetaTemplateDefinition,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    let code_length = 9;
    let description_length = 25;
    lines.push(format!("{indent}<CodeLength>{code_length}</CodeLength>"));
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    lines.push(format!(
        "{indent}<CodeType>{}</CodeType>",
        escape_xml("String")
    ));
    lines.push(format!(
        "{indent}<CodeAllowedLength>{}</CodeAllowedLength>",
        escape_xml("Variable")
    ));
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        escape_xml("AsDescription")
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
        escape_xml("DontUse")
    ));
    emit_empty_meta_object_refs(lines, indent, "BaseCalculationTypes");
    let action_period_use = false;
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
        escape_xml("Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        escape_xml("Use")
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
    defn: &MetaTemplateDefinition,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!(
        "{indent}<EditType>{}</EditType>",
        escape_xml("InDialog")
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
        escape_xml("String")
    ));
    let number_length = 11;
    lines.push(format!(
        "{indent}<NumberLength>{number_length}</NumberLength>"
    ));
    lines.push(format!(
        "{indent}<NumberAllowedLength>{}</NumberAllowedLength>",
        escape_xml("Variable")
    ));
    let check_unique = true;
    lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    emit_meta_standard_attributes(lines, indent, "BusinessProcess");
    lines.push(format!("{indent}<Characteristics/>"));
    let autonumbering = true;
    lines.push(format!(
        "{indent}<Autonumbering>{autonumbering}</Autonumbering>"
    ));
    lines.push(format!("{indent}<BasedOn/>"));
    lines.push(format!(
        "{indent}<NumberPeriodicity>{}</NumberPeriodicity>",
        escape_xml("Nonperiodical")
    ));
    emit_meta_optional_text(lines, indent, "Task", defn.task.as_deref());
    let privileged = true;
    lines.push(format!(
        "{indent}<CreateTaskInPrivilegedMode>{privileged}</CreateTaskInPrivilegedMode>"
    ));
    lines.push(format!("{indent}<DataLockFields/>"));
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        escape_xml("Automatic")
    ));
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        escape_xml("Use")
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
    defn: &MetaTemplateDefinition,
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
        escape_xml("BusinessProcessNumber")
    ));
    let description_length = 150;
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    emit_meta_optional_text(lines, indent, "Addressing", None);
    emit_meta_optional_text(lines, indent, "MainAddressingAttribute", None);
    emit_meta_optional_text(lines, indent, "CurrentPerformer", None);
    lines.push(format!("{indent}<BasedOn/>"));
    emit_meta_standard_attributes(lines, indent, "Task");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        escape_xml("AsDescription")
    ));
    lines.push(format!(
        "{indent}<EditType>{}</EditType>",
        escape_xml("InDialog")
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
        escape_xml("Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        escape_xml("Use")
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
    defn: &MetaTemplateDefinition,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    let code_length = 9;
    let description_length = 100;
    lines.push(format!("{indent}<CodeLength>{code_length}</CodeLength>"));
    lines.push(format!(
        "{indent}<CodeAllowedLength>{}</CodeAllowedLength>",
        escape_xml("Variable")
    ));
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        escape_xml("AsDescription")
    ));
    lines.push(format!(
        "{indent}<EditType>{}</EditType>",
        escape_xml("InDialog")
    ));
    let quick_choice = false;
    lines.push(format!("{indent}<QuickChoice>{quick_choice}</QuickChoice>"));
    lines.push(format!(
        "{indent}<ChoiceMode>{}</ChoiceMode>",
        escape_xml("BothWays")
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
    let distributed = false;
    let include_ext = false;
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
        escape_xml("Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        escape_xml("Use")
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
    defn: &MetaTemplateDefinition,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    for tag in ["DefaultForm", "AuxiliaryForm"] {
        emit_meta_optional_text(lines, indent, tag, None);
    }
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    if defn.registered_documents.is_empty() {
        emit_empty_meta_object_refs(lines, indent, "RegisteredDocuments");
    } else {
        lines.push(format!("{indent}<RegisteredDocuments>"));
        for document in &defn.registered_documents {
            lines.push(format!(
                "{indent}\t<xr:Item xsi:type=\"xr:MDObjectRef\">{}</xr:Item>",
                escape_xml(document)
            ));
        }
        lines.push(format!("{indent}</RegisteredDocuments>"));
    }
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
    defn: &MetaTemplateDefinition,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    for tag in [
        "DefaultForm",
        "AuxiliaryForm",
        "MainDataCompositionSchema",
        "DefaultSettingsForm",
        "AuxiliarySettingsForm",
        "DefaultVariantForm",
    ] {
        emit_meta_optional_text(lines, indent, tag, None);
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
    defn: &MetaTemplateDefinition,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>false</UseStandardCommands>"
    ));
    emit_meta_optional_text(lines, indent, "DefaultForm", None);
    emit_meta_optional_text(lines, indent, "AuxiliaryForm", None);
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    lines.push(format!("{indent}<ExtendedPresentation/>"));
    lines.push(format!("{indent}<Explanation/>"));
}

pub(super) fn emit_meta_scheduled_job_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    let method_name =
        canonical_common_module_method(defn.method_name.as_deref().unwrap_or_default());
    lines.push(format!(
        "{indent}<MethodName>{}</MethodName>",
        escape_xml(&method_name)
    ));
    lines.push(format!(
        "{indent}<Description>{}</Description>",
        escape_xml(synonym)
    ));
    emit_meta_optional_text(lines, indent, "Key", None);
    let use_job = false;
    let predefined = false;
    let restart_count = 3;
    let restart_interval = 10;
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
    defn: &MetaTemplateDefinition,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    let sources = defn.sources.clone();
    if sources.is_empty() {
        lines.push(format!("{indent}<Source/>"));
    } else {
        lines.push(format!("{indent}<Source>"));
        let source_types = sources
            .iter()
            .map(|source| MetadataXmlType::Configuration(source))
            .collect::<Vec<_>>();
        emit_metadata_xml_type_contents(lines, &format!("{indent}\t"), &source_types);
        lines.push(format!("{indent}</Source>"));
    }
    lines.push(format!(
        "{indent}<Event>{}</Event>",
        escape_xml("BeforeWrite")
    ));
    let handler = canonical_common_module_method(defn.handler.as_deref().unwrap_or_default());
    lines.push(format!(
        "{indent}<Handler>{}</Handler>",
        escape_xml(&handler)
    ));
}

pub(super) fn emit_meta_http_service_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    let root_url = obj_name.to_lowercase();
    lines.push(format!(
        "{indent}<RootURL>{}</RootURL>",
        escape_xml(&root_url)
    ));
    lines.push(format!(
        "{indent}<ReuseSessions>{}</ReuseSessions>",
        escape_xml("DontUse")
    ));
    let session_max_age = 20;
    lines.push(format!(
        "{indent}<SessionMaxAge>{session_max_age}</SessionMaxAge>"
    ));
}

pub(super) fn emit_meta_web_service_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    emit_meta_optional_text(lines, indent, "Namespace", None);
    emit_meta_optional_text(lines, indent, "XDTOPackages", None);
    lines.push(format!(
        "{indent}<DescriptorFileName>{}</DescriptorFileName>",
        escape_xml("ws1.1cws")
    ));
    lines.push(format!(
        "{indent}<ReuseSessions>{}</ReuseSessions>",
        escape_xml("DontUse")
    ));
    let session_max_age = 20;
    lines.push(format!(
        "{indent}<SessionMaxAge>{session_max_age}</SessionMaxAge>"
    ));
}

pub(super) fn emit_meta_common_module_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    // A newly added common module must have at least one execution context;
    // otherwise the typed object is structurally valid but cannot be borrowed
    // into an extension or used by code tooling. The typed minimal template
    // uses the server as its conservative executable default.
    let server = true;
    let server_call = false;
    let client_managed = false;
    lines.push(format!("{indent}<Global>{}</Global>", false));
    lines.push(format!(
        "{indent}<ClientManagedApplication>{client_managed}</ClientManagedApplication>"
    ));
    lines.push(format!("{indent}<Server>{server}</Server>"));
    lines.push(format!(
        "{indent}<ExternalConnection>{}</ExternalConnection>",
        false
    ));
    lines.push(format!(
        "{indent}<ClientOrdinaryApplication>{}</ClientOrdinaryApplication>",
        false
    ));
    lines.push(format!("{indent}<ServerCall>{server_call}</ServerCall>"));
    lines.push(format!("{indent}<Privileged>{}</Privileged>", false));
    lines.push(format!(
        "{indent}<ReturnValuesReuse>{}</ReturnValuesReuse>",
        escape_xml("DontUse")
    ));
}

pub(super) fn emit_meta_defined_type_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &MetaTemplateDefinition,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!("{indent}<Type/>"));
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

pub(super) fn emit_empty_meta_object_refs(lines: &mut Vec<String>, indent: &str, tag: &str) {
    lines.push(format!("{indent}<{tag}/>"));
}

pub(super) fn canonical_common_module_method(value: &str) -> String {
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
    _defn: &MetaTemplateDefinition,
) {
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        escape_xml("Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        escape_xml("Use")
    ));
}

pub(super) fn emit_meta_number_properties(
    lines: &mut Vec<String>,
    indent: &str,
    _defn: &MetaTemplateDefinition,
    default_number_length: i64,
) {
    lines.push(format!(
        "{indent}<NumberType>{}</NumberType>",
        escape_xml("String")
    ));
    let number_length = default_number_length;
    lines.push(format!(
        "{indent}<NumberLength>{number_length}</NumberLength>"
    ));
    lines.push(format!(
        "{indent}<NumberAllowedLength>{}</NumberAllowedLength>",
        escape_xml("Variable")
    ));
    let check_unique = true;
    let autonumbering = true;
    lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    lines.push(format!(
        "{indent}<Autonumbering>{autonumbering}</Autonumbering>"
    ));
}

pub(super) struct MetadataEnumValueTemplate {
    pub(crate) name: String,
    pub(crate) synonym: String,
    pub(crate) comment: String,
}

pub(super) fn emit_meta_enum_value<F>(
    lines: &mut Vec<String>,
    indent: &str,
    value: &MetadataEnumValueTemplate,
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

pub(super) fn emit_meta_register_field<F>(
    lines: &mut Vec<String>,
    indent: &str,
    field_tag: &str,
    attr: &MetadataAttributeTemplate,
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
    let type_indent = format!("{indent}\t\t");
    lines.push(format!("{type_indent}<Type>"));
    if field_tag == "Resource" {
        lines.push(format!("{type_indent}\t<v8:Type>xs:decimal</v8:Type>"));
        lines.push(format!("{type_indent}\t<v8:NumberQualifiers>"));
        lines.push(format!("{type_indent}\t\t<v8:Digits>15</v8:Digits>"));
        lines.push(format!(
            "{type_indent}\t\t<v8:FractionDigits>2</v8:FractionDigits>"
        ));
        lines.push(format!(
            "{type_indent}\t\t<v8:AllowedSign>Any</v8:AllowedSign>"
        ));
        lines.push(format!("{type_indent}\t</v8:NumberQualifiers>"));
    } else {
        lines.push(format!("{type_indent}\t<v8:Type>xs:string</v8:Type>"));
        lines.push(format!("{type_indent}\t<v8:StringQualifiers>"));
        lines.push(format!("{type_indent}\t\t<v8:Length>10</v8:Length>"));
        lines.push(format!(
            "{type_indent}\t\t<v8:AllowedLength>Variable</v8:AllowedLength>"
        ));
        lines.push(format!("{type_indent}\t</v8:StringQualifiers>"));
    }
    lines.push(format!("{type_indent}</Type>"));
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
    lines.push(format!("{indent}\t\t<MultiLine>false</MultiLine>"));
    lines.push(format!("{indent}\t\t<ExtendedEdit>false</ExtendedEdit>"));
    lines.push(format!("{indent}\t\t<MinValue xsi:nil=\"true\"/>"));
    lines.push(format!("{indent}\t\t<MaxValue xsi:nil=\"true\"/>"));
    if register_type == "InformationRegister" {
        lines.push(format!(
            "{indent}\t\t<FillFromFillingValue>false</FillFromFillingValue>"
        ));
        lines.push(format!("{indent}\t\t<FillValue xsi:nil=\"true\"/>"));
    }
    let fill_checking = if attr.required {
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
            lines.push(format!("{indent}\t\t<Master>false</Master>"));
            lines.push(format!("{indent}\t\t<MainFilter>false</MainFilter>"));
        }
        if matches!(
            register_type,
            "InformationRegister"
                | "AccumulationRegister"
                | "AccountingRegister"
                | "CalculationRegister"
        ) {
            lines.push(format!(
                "{indent}\t\t<DenyIncompleteValues>false</DenyIncompleteValues>"
            ));
        }
        if register_type == "CalculationRegister" {
            lines.push(format!("{indent}\t\t<BaseDimension>false</BaseDimension>"));
            lines.push(format!("{indent}\t\t<ScheduleLink/>"));
        }
    }
    if field_tag == "Dimension" || register_type == "InformationRegister" {
        lines.push(format!("{indent}\t\t<Indexing>DontIndex</Indexing>"));
    }
    lines.push(format!("{indent}\t\t<FullTextSearch>Use</FullTextSearch>"));
    if field_tag == "Dimension" && register_type == "AccumulationRegister" {
        lines.push(format!("{indent}\t\t<UseInTotals>true</UseInTotals>"));
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

pub(super) fn emit_meta_standard_attributes(
    lines: &mut Vec<String>,
    indent: &str,
    object_type: &str,
) {
    let attrs = metadata_standard_attribute_names(object_type);
    if attrs.is_empty() {
        return;
    }
    lines.push(format!("{indent}<StandardAttributes>"));
    for attr in attrs {
        emit_meta_standard_attribute(lines, &format!("{indent}\t"), object_type, attr);
    }
    lines.push(format!("{indent}</StandardAttributes>"));
}

pub(super) fn metadata_standard_attribute_names(object_type: &str) -> &'static [&'static str] {
    match object_type {
        "Catalog" => &[
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
        "Document" => &["Posted", "Ref", "DeletionMark", "Date", "Number"],
        "Enum" => &["Order", "Ref"],
        "InformationRegister" => &["Active", "LineNumber", "Recorder", "Period"],
        "AccumulationRegister" => &["RecordType", "Active", "LineNumber", "Recorder", "Period"],
        "AccountingRegister" => &[
            "Account",
            "RecordType",
            "Active",
            "LineNumber",
            "Recorder",
            "Period",
        ],
        "CalculationRegister" => &[
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
        "ChartOfAccounts" => &[
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
        "ChartOfCharacteristicTypes" => &[
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
        "ChartOfCalculationTypes" => &[
            "PredefinedDataName",
            "Predefined",
            "Ref",
            "DeletionMark",
            "ActionPeriodIsBasic",
            "Description",
            "Code",
        ],
        "BusinessProcess" => &[
            "Started",
            "HeadTask",
            "Completed",
            "Ref",
            "DeletionMark",
            "Date",
            "Number",
        ],
        "Task" => &[
            "Executed",
            "Description",
            "RoutePoint",
            "BusinessProcess",
            "Ref",
            "DeletionMark",
            "Date",
            "Number",
        ],
        "ExchangePlan" => &[
            "ExchangeDate",
            "ThisNode",
            "ReceivedNo",
            "SentNo",
            "Ref",
            "DeletionMark",
            "Description",
            "Code",
        ],
        "DocumentJournal" => &["Type", "Ref", "Date", "Posted", "DeletionMark", "Number"],
        "TabularSection" => &["LineNumber"],
        _ => &[],
    }
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
pub(super) struct MetadataAttributeTemplate {
    pub(crate) name: String,
    pub(crate) synonym: String,
    pub(crate) required: bool,
}

pub(super) struct MetadataTabularSectionTemplate {
    pub(crate) name: String,
    pub(crate) columns: Vec<MetadataAttributeTemplate>,
}

pub(super) fn emit_meta_attribute<F>(
    lines: &mut Vec<String>,
    indent: &str,
    attr: &MetadataAttributeTemplate,
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
    lines.push(format!("{indent}\t\t<Type>"));
    lines.push(format!("{indent}\t\t\t<v8:Type>xs:string</v8:Type>"));
    lines.push(format!("{indent}\t\t</Type>"));
    lines.push(format!("{indent}\t\t<PasswordMode>false</PasswordMode>"));
    lines.push(format!("{indent}\t\t<Format/>"));
    lines.push(format!("{indent}\t\t<EditFormat/>"));
    lines.push(format!("{indent}\t\t<ToolTip/>"));
    lines.push(format!("{indent}\t\t<MarkNegatives>false</MarkNegatives>"));
    lines.push(format!("{indent}\t\t<Mask/>"));
    lines.push(format!("{indent}\t\t<MultiLine>false</MultiLine>"));
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
        lines.push(format!("{indent}\t\t<FillValue xsi:nil=\"true\"/>"));
    }
    let fill_checking = if attr.required {
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
    lines.push(format!(
        "{indent}\t\t<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>"
    ));
    if context == "catalog" {
        lines.push(format!("{indent}\t\t<Use>ForItem</Use>"));
    }
    if !matches!(context, "processor" | "processor-tabular") {
        lines.push(format!("{indent}\t\t<Indexing>DontIndex</Indexing>"));
        lines.push(format!("{indent}\t\t<FullTextSearch>Use</FullTextSearch>"));
        if !matches!(context, "chart" | "register-other") {
            lines.push(format!("{indent}\t\t<DataHistory>Use</DataHistory>"));
        }
    }
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</Attribute>"));
}

pub(super) fn meta_attribute_context(object_type: &str) -> &'static str {
    match object_type {
        "Catalog" => "catalog",
        "Document" => "document",
        "Report" | "DataProcessor" | "ExternalReport" | "ExternalDataProcessor" => "processor",
        "ChartOfAccounts" | "ChartOfCharacteristicTypes" | "ChartOfCalculationTypes" => "chart",
        "InformationRegister" => "register-info",
        "AccumulationRegister" | "AccountingRegister" | "CalculationRegister" => "register-other",
        _ => "object",
    }
}

pub(super) fn emit_meta_tabular_section<F>(
    lines: &mut Vec<String>,
    indent: &str,
    section: &MetadataTabularSectionTemplate,
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
    let mut previous: Option<char> = None;
    for ch in name.chars() {
        if previous.is_some_and(|previous| meta_synonym_word_boundary(previous, ch)) {
            result.push(' ');
        }
        result.push(ch);
        previous = Some(ch);
    }
    let mut chars = result.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first, chars.as_str().to_lowercase()),
        None => result,
    }
}

/// A generated synonym breaks a word where the platform name itself changes
/// class: after a lowercase letter before an uppercase one, and on both sides of
/// a digit run. A digit is neither uppercase nor lowercase, so without the
/// second rule `SumFor30Days` would keep its digits glued to the words around
/// them once the tail is lowercased.
fn meta_synonym_word_boundary(previous: char, current: char) -> bool {
    if previous.is_lowercase() && current.is_uppercase() {
        return true;
    }
    (previous.is_alphabetic() && current.is_ascii_digit())
        || (previous.is_ascii_digit() && current.is_alphabetic())
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
            task: Some("Task.MetaAddTask".to_string()),
            registered_documents: vec!["Document.MetaAddDocument".to_string()],
            method_name: Some("CommonModule.MetaAddHandlers.Run".to_string()),
            event_source: Some("CatalogRef.MetaAddSource".to_string()),
            event_handler: Some("CommonModule.MetaAddHandlers.Handle".to_string()),
            dependencies: Vec::new(),
        }
    }

    #[test]
    fn document_register_record_accepts_both_self_closing_spellings() {
        for empty in ["<RegisterRecords/>", "<RegisterRecords />"] {
            let source = format!("<Properties>{empty}</Properties>");
            let updated =
                add_document_register_record(source.as_bytes(), "AccountingRegister.Ledger")
                    .expect("both platform self-closing spellings must be accepted");
            let updated = String::from_utf8(updated).unwrap();
            assert!(updated.contains(
                "<RegisterRecords><xr:Item xsi:type=\"xr:MDObjectRef\">AccountingRegister.Ledger</xr:Item></RegisterRecords>"
            ));
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
            "// Procedure Shadow() Export\n",
            "Function Ignore() Export\nEndFunction\n",
            "PROCEDURE Run() EXPORT\nEndProcedure\n",
            "Процедура Обработать(Источник, Отказ) Экспорт\nКонецПроцедуры\n",
            "Procedure Private(One, Two)\nEndProcedure\n",
        );

        assert_eq!(
            first_exported_bsl_procedure(module, 0).unwrap().as_deref(),
            Some("Run")
        );
        assert_eq!(
            first_exported_bsl_procedure(module, 2).unwrap().as_deref(),
            Some("Обработать")
        );
        assert_eq!(first_exported_bsl_procedure(module, 1).unwrap(), None);
    }
}
