#![allow(dead_code, unused_imports)]

use super::*;

#[derive(Clone)]
pub(crate) struct MetaInfoAttr<'a, 'input> {
    pub(crate) name: String,
    pub(crate) type_name: String,
    pub(crate) flags: String,
    pub(crate) _marker: std::marker::PhantomData<roxmltree::Node<'a, 'input>>,
}

pub(crate) struct MetaInfoTabularSection<'a, 'input> {
    pub(crate) name: String,
    pub(crate) columns: Vec<MetaInfoAttr<'a, 'input>>,
}

pub(crate) struct MetaInfoHttpMethod {
    pub(crate) http_method: String,
    pub(crate) handler: String,
}

pub(crate) struct MetaInfoHttpEndpoint {
    pub(crate) name: String,
    pub(crate) template: String,
    pub(crate) methods: Vec<MetaInfoHttpMethod>,
}

pub(crate) struct MetaInfoWsOperation {
    pub(crate) name: String,
    pub(crate) params: String,
    pub(crate) return_type: String,
    pub(crate) proc_name: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// Typed answer of `unica.meta.info` (ADR-0023). The report translated platform
/// properties into Russian prose (`Номер: Строка(9), помесячно, авто`); the data
/// carries the platform's own property names and values instead, so the twenty
/// three metadata kinds need one shape rather than fifteen bespoke sections.
pub(crate) struct MetaInfoData {
    /// The logical address this call resolved. Flattened, because ADR-0021
    /// fixed `sourceSet` and `metadataPath` at the top level of `data` and
    /// `unica.source.locate` answers with the same shape.
    #[serde(flatten)]
    pub(crate) target: ResolvedTarget,
    /// The platform's metadata kind: `Catalog`, `Document`, `CommonModule`, …
    pub(crate) kind: String,
    pub(crate) name: String,
    /// The object's synonym; `null` when it declares none.
    pub(crate) synonym: Option<String>,
    pub(crate) support: ObjectSupportData,
    /// Scalar properties under `Properties`, by their platform names.
    pub(crate) properties: Vec<MetaInfoProperty>,
    /// Owners of a subordinate catalog; empty for everything else.
    pub(crate) owners: Vec<String>,
    pub(crate) attributes: Vec<MetaInfoAttrData>,
    /// Register dimensions; empty for every other kind.
    pub(crate) dimensions: Vec<MetaInfoAttrData>,
    /// Register resources; empty for every other kind.
    pub(crate) resources: Vec<MetaInfoAttrData>,
    pub(crate) tabular_sections: Vec<MetaInfoTabularSectionData>,
    /// Enumeration values; empty for every other kind.
    pub(crate) enum_values: Vec<String>,
    pub(crate) forms: Vec<String>,
    pub(crate) templates: Vec<String>,
    pub(crate) commands: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaInfoProperty {
    pub(crate) name: String,
    pub(crate) value: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaInfoAttrData {
    pub(crate) name: String,
    #[serde(rename = "type")]
    pub(crate) type_name: Option<String>,
    /// Platform flags the report rendered inline, one entry each.
    pub(crate) flags: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaInfoTabularSectionData {
    pub(crate) name: String,
    pub(crate) columns: Vec<MetaInfoAttrData>,
}

fn meta_info_attr_data(attrs: Vec<MetaInfoAttr<'_, '_>>) -> Vec<MetaInfoAttrData> {
    attrs
        .into_iter()
        .map(|attr| MetaInfoAttrData {
            name: attr.name,
            type_name: (!attr.type_name.is_empty()).then_some(attr.type_name),
            // `meta_info_format_flags` renders `  [обязательный, индекс]`;
            // splitting it raw left the brackets on the first and last flag.
            flags: attr
                .flags
                .trim()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .map(str::trim)
                .filter(|flag| !flag.is_empty())
                .map(str::to_string)
                .collect(),
        })
        .collect()
}

/// Scalar `Properties` children, by their platform names. Composite children
/// (`Synonym`, `Type`, `Owners`, …) have their own typed places and are skipped
/// here so the map stays flat.
fn meta_info_properties(props: Option<roxmltree::Node<'_, '_>>) -> Vec<MetaInfoProperty> {
    let Some(props) = props else {
        return Vec::new();
    };
    props
        .children()
        .filter(|child| child.is_element())
        .filter(|child| child.children().all(|node| !node.is_element()))
        .filter_map(|child| {
            let name = child.tag_name().name().to_string();
            if matches!(name.as_str(), "Name") {
                return None;
            }
            let value = child.text().unwrap_or("").trim().to_string();
            (!value.is_empty()).then_some(MetaInfoProperty { name, value })
        })
        .collect()
}

fn meta_info_owner_names(props: Option<roxmltree::Node<'_, '_>>) -> Vec<String> {
    let Some(owners_node) = props.and_then(|node| meta_info_child(node, "Owners")) else {
        return Vec::new();
    };
    meta_info_children(owners_node, "Item")
        .into_iter()
        .map(meta_info_inner_text)
        .map(|owner| owner.trim().to_string())
        .filter(|owner| !owner.is_empty())
        .collect()
}

pub(crate) struct MetaInfoExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<MetaInfoData>,
}

pub(crate) fn analyze_meta_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    analyze_meta_info_with_data(args, context).outcome
}

/// The resolved logical target rides in typed data rather than in the printed
/// report: ADR-0021 asks every exact operation to name the source set it
/// actually resolved, and a machine reader should not have to parse prose for
/// it.
pub(crate) fn analyze_meta_info_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> MetaInfoExecution {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    let result = (|| -> Result<(MetaInfoData, PathBuf), String> {
        let (resolved, object_path) = resolve_metadata_object_descriptor(args, context)?;
        let text = read_utf8_sig(&object_path)?;
        let doc = Document::parse(text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("XML parse error in {}: {err}", object_path.display()))?;
        let root = doc.root_element();
        if root.tag_name().name() != "MetaDataObject" {
            return Err("[ERROR] Not a valid 1C metadata XML file".to_string());
        }

        let Some(type_node) = root
            .children()
            .find(|child| child.is_element() && child.tag_name().namespace() == Some(MD_NS))
        else {
            return Err("[ERROR] Cannot detect metadata type".to_string());
        };
        let md_type = type_node.tag_name().name();
        let props = meta_info_child(type_node, "Properties");
        let child_objs = meta_info_child(type_node, "ChildObjects");
        let obj_name = props
            .and_then(|node| meta_info_child_text(node, "Name"))
            .unwrap_or_default();
        let synonym = props
            .and_then(|node| meta_info_child(node, "Synonym"))
            .map(meta_info_ml_text)
            .unwrap_or_default();
        // Mode and Name sliced one object into shorter reports. Data answers
        // with the whole object once; a caller projects what it needs.
        let is_register = md_type.ends_with("Register");
        let data = MetaInfoData {
            target: resolved,
            kind: md_type.to_string(),
            name: obj_name,
            synonym: (!synonym.is_empty()).then_some(synonym),
            support: object_support_state(&object_path),
            properties: meta_info_properties(props),
            owners: meta_info_owner_names(props),
            attributes: if md_type == "Enum" {
                Vec::new()
            } else {
                meta_info_attr_data(meta_info_attributes(child_objs, "Attribute", false))
            },
            dimensions: if is_register {
                meta_info_attr_data(meta_info_attributes(child_objs, "Dimension", true))
            } else {
                Vec::new()
            },
            resources: if is_register {
                meta_info_attr_data(meta_info_attributes(child_objs, "Resource", false))
            } else {
                Vec::new()
            },
            tabular_sections: meta_info_tabular_sections(child_objs)
                .into_iter()
                .map(|section| MetaInfoTabularSectionData {
                    name: section.name,
                    columns: meta_info_attr_data(section.columns),
                })
                .collect(),
            enum_values: meta_info_enum_values(child_objs),
            forms: meta_info_simple_children(child_objs, "Form"),
            templates: meta_info_simple_children(child_objs, "Template"),
            commands: meta_info_simple_children(child_objs, "Command"),
        };
        Ok((data, object_path))
    })();

    match result {
        Ok((data, artifact)) => MetaInfoExecution {
            outcome: AdapterOutcome {
                ok: true,
                summary: format!(
                    "unica.meta.info described {} {} with {} attribute(s)",
                    data.kind,
                    data.name,
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
        Err(error) => MetaInfoExecution {
            outcome: AdapterOutcome {
                ok: false,
                summary: "unica.meta.info failed in native metadata analyzer".to_string(),
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

pub(crate) fn resolve_meta_info_path(mut object_path: PathBuf) -> Result<PathBuf, String> {
    if object_path.is_dir() {
        let dir_name = object_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let candidate = object_path.join(format!("{dir_name}.xml"));
        let sibling = object_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(format!("{dir_name}.xml"));
        if candidate.is_file() {
            object_path = candidate;
        } else if sibling.is_file() {
            object_path = sibling;
        } else {
            let mut xml_files = fs::read_dir(&object_path)
                .map_err(|err| format!("failed to read {}: {err}", object_path.display()))?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
                })
                .collect::<Vec<_>>();
            xml_files.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
            if let Some(xml_file) = xml_files.into_iter().next() {
                object_path = xml_file;
            } else {
                return Err(format!(
                    "[ERROR] No XML file found in directory: {}",
                    object_path.display()
                ));
            }
        }
    }

    if !object_path.exists() {
        let file_name = object_path.file_stem().and_then(|name| name.to_str());
        let parent_dir = object_path.parent();
        let parent_dir_name = parent_dir
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str());
        if file_name == parent_dir_name {
            if let (Some(parent_dir), Some(file_name)) = (parent_dir, file_name) {
                let candidate = parent_dir
                    .parent()
                    .unwrap_or_else(|| Path::new(""))
                    .join(format!("{file_name}.xml"));
                if candidate.exists() {
                    object_path = candidate;
                }
            }
        }
    }

    if !object_path.exists() {
        return Err(format!("[ERROR] File not found: {}", object_path.display()));
    }
    Ok(object_path)
}

pub(crate) fn meta_info_main_lines(
    md_type: &str,
    props: Option<roxmltree::Node<'_, '_>>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
    obj_name: &str,
    synonym: &str,
    mode: &str,
) -> Result<Vec<String>, String> {
    let mut lines = Vec::<String>::new();
    let ru_type_name = meta_info_type_ru(md_type);
    let mut header = format!("=== {ru_type_name}: {obj_name}");
    if !synonym.is_empty() && synonym != obj_name {
        header.push_str(&format!(" — \"{synonym}\""));
    }
    header.push_str(" ===");
    lines.push(header);

    if meta_info_is_reference_metadata_type(md_type) {
        let object_presentation = meta_info_ml_child_text(props, "ObjectPresentation");
        let extended_object_presentation =
            meta_info_ml_child_text(props, "ExtendedObjectPresentation");
        let list_presentation = meta_info_ml_child_text(props, "ListPresentation");
        let extended_list_presentation = meta_info_ml_child_text(props, "ExtendedListPresentation");
        let type_presentation = object_presentation
            .as_deref()
            .filter(|value| !value.is_empty())
            .or_else(|| (!synonym.is_empty()).then_some(synonym))
            .unwrap_or(obj_name);
        lines.push(format!("Представление типа: {type_presentation}"));
        if mode == "full" {
            if let Some(value) = object_presentation.filter(|value| !value.is_empty()) {
                lines.push(format!("Представление объекта: {value}"));
            }
            if let Some(value) = extended_object_presentation.filter(|value| !value.is_empty()) {
                lines.push(format!("Расширенное представление объекта: {value}"));
            }
            if let Some(value) = list_presentation.filter(|value| !value.is_empty()) {
                lines.push(format!("Представление списка: {value}"));
            }
            if let Some(value) = extended_list_presentation.filter(|value| !value.is_empty()) {
                lines.push(format!("Расширенное представление списка: {value}"));
            }
        }
    }

    if mode == "brief" {
        meta_info_append_brief(&mut lines, md_type, props, child_objs);
    } else if mode == "overview" || mode == "full" {
        meta_info_append_overview_or_full(&mut lines, md_type, props, child_objs, mode);
    } else {
        return Err(format!(
            "argument -Mode: invalid choice: '{mode}' (choose from 'overview', 'brief', 'full')"
        ));
    }
    Ok(lines)
}

pub(crate) fn meta_info_append_brief(
    lines: &mut Vec<String>,
    md_type: &str,
    props: Option<roxmltree::Node<'_, '_>>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
) {
    let attrs = meta_info_attributes(child_objs, "Attribute", false);
    if !attrs.is_empty() {
        lines.push(format!(
            "Реквизиты ({}): {}",
            attrs.len(),
            attrs
                .iter()
                .map(|attr| attr.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if md_type.ends_with("Register") {
        let dims = meta_info_attributes(child_objs, "Dimension", true);
        if !dims.is_empty() {
            lines.push(format!(
                "Измерения ({}): {}",
                dims.len(),
                dims.iter()
                    .map(|attr| attr.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let resources = meta_info_attributes(child_objs, "Resource", false);
        if !resources.is_empty() {
            lines.push(format!(
                "Ресурсы ({}): {}",
                resources.len(),
                resources
                    .iter()
                    .map(|attr| attr.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    let tabular_sections = meta_info_tabular_sections(child_objs);
    if !tabular_sections.is_empty() {
        let parts = tabular_sections
            .iter()
            .map(|section| format!("{}({})", section.name, section.columns.len()))
            .collect::<Vec<_>>();
        lines.push(format!(
            "ТЧ ({}): {}",
            tabular_sections.len(),
            parts.join(", ")
        ));
    }

    if md_type == "Enum" {
        let values = meta_info_enum_values(child_objs);
        if !values.is_empty() {
            lines.push(format!(
                "Значения ({}): {}",
                values.len(),
                values.join(", ")
            ));
        }
    }

    if md_type == "DefinedType" {
        if let Some(type_node) = props.and_then(|node| meta_info_child(node, "Type")) {
            let types = meta_info_children(type_node, "Type")
                .into_iter()
                .map(|node| meta_info_format_single_type(meta_info_inner_text(node), type_node))
                .collect::<Vec<_>>();
            if !types.is_empty() {
                lines.push(format!("Типы ({}): {}", types.len(), types.join(", ")));
            }
        }
    }

    if md_type == "CommonModule" {
        let flags = meta_info_common_module_flags(props);
        if !flags.is_empty() {
            lines.push(flags.join(" | "));
        }
    }

    if md_type == "ScheduledJob" {
        meta_info_append_scheduled_job(lines, props);
    }

    if md_type == "EventSubscription" {
        meta_info_append_event_subscription_brief(lines, props);
    }

    if md_type == "HTTPService" {
        if let Some(root_url) = props.and_then(|node| meta_info_child_text(node, "RootURL")) {
            if !root_url.is_empty() {
                lines.push(format!("Корневой URL: /{root_url}"));
            }
        }
        let endpoints = meta_info_http_endpoints(child_objs);
        if !endpoints.is_empty() {
            let total_methods = endpoints
                .iter()
                .map(|endpoint| endpoint.methods.len())
                .sum::<usize>();
            lines.push(format!(
                "Шаблоны: {} | Методы: {total_methods}",
                endpoints.len()
            ));
        }
    }

    if md_type == "WebService" {
        if let Some(namespace) = props.and_then(|node| meta_info_child_text(node, "Namespace")) {
            if !namespace.is_empty() {
                lines.push(format!("Пространство имён: {namespace}"));
            }
        }
        let operations = meta_info_ws_operations(child_objs);
        if !operations.is_empty() {
            lines.push(format!("Операции: {}", operations.len()));
        }
    }
}

pub(crate) fn meta_info_append_overview_or_full(
    lines: &mut Vec<String>,
    md_type: &str,
    props: Option<roxmltree::Node<'_, '_>>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
    mode: &str,
) {
    meta_info_append_owners(lines, props);
    if md_type == "Document" {
        meta_info_append_document_header(lines, props);
    }
    if md_type == "Catalog" {
        meta_info_append_catalog_header(lines, props);
    }
    if md_type.ends_with("Register") {
        meta_info_append_register_header(lines, md_type, props);
    }
    if md_type == "Constant" {
        if let Some(type_node) = props.and_then(|node| meta_info_child(node, "Type")) {
            let type_name = meta_info_format_type(type_node);
            if !type_name.is_empty() {
                lines.push(format!("Тип: {type_name}"));
            }
        }
    }
    if md_type == "Report" {
        if let Some(main_dcs) =
            props.and_then(|node| meta_info_child_text(node, "MainDataCompositionSchema"))
        {
            if !main_dcs.is_empty() {
                let dcs_name = main_dcs
                    .rsplit_once(".Template.")
                    .map(|(_, name)| name)
                    .unwrap_or(&main_dcs);
                lines.push(format!("Основная СКД: {dcs_name}"));
            }
        }
    }
    if md_type == "DefinedType" {
        meta_info_append_defined_type(lines, props);
    }
    if md_type == "CommonModule" {
        let flags = meta_info_common_module_flags(props);
        if !flags.is_empty() {
            lines.push(flags.join(" | "));
        }
    }
    if md_type == "ScheduledJob" {
        meta_info_append_scheduled_job(lines, props);
    }
    if md_type == "EventSubscription" {
        meta_info_append_event_subscription(lines, props, mode);
    }
    if md_type == "HTTPService" {
        meta_info_append_http_service(lines, props, child_objs);
    }
    if md_type == "WebService" {
        meta_info_append_web_service(lines, props, child_objs);
    }
    if md_type == "Enum" {
        meta_info_append_enum_values(lines, child_objs);
    }
    if md_type.ends_with("Register") {
        meta_info_append_attribute_section(lines, "Измерения", child_objs, "Dimension", true);
        meta_info_append_attribute_section(lines, "Ресурсы", child_objs, "Resource", false);
    }
    if md_type != "Enum" {
        meta_info_append_attribute_section(lines, "Реквизиты", child_objs, "Attribute", false);
        meta_info_append_tabular_sections(lines, child_objs, mode);
    }
    // Forms, templates and commands exist on far more kinds than reports and
    // data processors; overview hid them from every other object.
    if mode == "overview" {
        meta_info_append_simple_children(lines, child_objs);
    }
    if mode == "full" {
        meta_info_append_full_tail(lines, md_type, props, child_objs);
    }
}

pub(crate) fn meta_info_drill_lines(
    md_type: &str,
    child_objs: Option<roxmltree::Node<'_, '_>>,
    drill_name: &str,
    obj_name: &str,
) -> Result<Vec<String>, String> {
    let Some(child_objs) = child_objs else {
        return Err(format!("[ERROR] '{drill_name}' not found in {obj_name}"));
    };
    for (tag, label, is_dimension) in [
        ("Attribute", "Реквизит", false),
        ("Dimension", "Измерение", true),
        ("Resource", "Ресурс", false),
    ] {
        for attr in meta_info_children(child_objs, tag) {
            let Some(props) = meta_info_child(attr, "Properties") else {
                continue;
            };
            let name = meta_info_child_text(props, "Name").unwrap_or_default();
            if name == drill_name {
                return Ok(meta_info_drill_attr_lines(
                    label,
                    &name,
                    props,
                    is_dimension,
                ));
            }
        }
    }

    for section in meta_info_children(child_objs, "TabularSection") {
        let props = meta_info_child(section, "Properties");
        let section_name = props
            .and_then(|node| meta_info_child_text(node, "Name"))
            .unwrap_or_default();
        if section_name == drill_name {
            let section_child_objs = meta_info_child(section, "ChildObjects");
            let columns = meta_info_attributes(section_child_objs, "Attribute", false);
            let mut lines = vec![format!(
                "ТЧ: {section_name} ({} {}):",
                columns.len(),
                meta_info_decline_cols(columns.len())
            )];
            if !columns.is_empty() {
                let width = meta_info_max_name_len(&columns);
                for column in columns {
                    lines.push(meta_info_format_attr_line(&column, width));
                }
            }
            return Ok(lines);
        }
    }

    for value in meta_info_children(child_objs, "EnumValue") {
        let props = meta_info_child(value, "Properties");
        let value_name = props
            .and_then(|node| meta_info_child_text(node, "Name"))
            .unwrap_or_default();
        if value_name == drill_name {
            let mut lines = vec![format!("Значение перечисления: {value_name}")];
            if let Some(synonym) = props
                .and_then(|node| meta_info_child(node, "Synonym"))
                .map(meta_info_ml_text)
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("  Синоним: \"{synonym}\""));
            }
            if let Some(comment) = props
                .and_then(|node| meta_info_child_text(node, "Comment"))
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("  Комментарий: {comment}"));
            }
            return Ok(lines);
        }
    }

    if md_type == "HTTPService" {
        for endpoint in meta_info_http_endpoints(Some(child_objs)) {
            if endpoint.name == drill_name {
                let mut lines = vec![
                    format!("Шаблон URL: {drill_name}"),
                    format!("  Путь: {}", endpoint.template),
                ];
                for method in endpoint.methods {
                    lines.push(format!("  {} → {}", method.http_method, method.handler));
                }
                return Ok(lines);
            }
        }
    }

    if md_type == "WebService" {
        for operation in meta_info_ws_operations(Some(child_objs)) {
            if operation.name == drill_name {
                let mut lines = vec![
                    format!("Операция: {drill_name}"),
                    format!("  Возвращает: {}", operation.return_type),
                ];
                if !operation.proc_name.is_empty() {
                    lines.push(format!("  Процедура: {}", operation.proc_name));
                }
                return Ok(lines);
            }
        }
    }

    Err(format!("[ERROR] '{drill_name}' not found in {obj_name}"))
}

pub(crate) fn meta_info_drill_attr_lines(
    label: &str,
    name: &str,
    props: roxmltree::Node<'_, '_>,
    is_dimension: bool,
) -> Vec<String> {
    let type_name = meta_info_child(props, "Type")
        .map(meta_info_format_type)
        .unwrap_or_default();
    let fill_checking = meta_info_child_text(props, "FillChecking").unwrap_or_default();
    let indexing = meta_info_child_text(props, "Indexing").unwrap_or_default();
    let indexing_ru = match indexing.as_str() {
        "" | "DontIndex" => "нет".to_string(),
        "Index" => "Индекс".to_string(),
        "IndexWithAdditionalOrder" => "Индекс с доп. упорядочиванием".to_string(),
        other => other.to_string(),
    };
    let mut lines = vec![
        format!("{label}: {name}"),
        format!("  Тип: {type_name}"),
        format!(
            "  Обязательный: {}",
            if fill_checking == "ShowError" {
                "да"
            } else {
                "нет"
            }
        ),
        format!("  Индексирование: {indexing_ru}"),
    ];
    if meta_info_child_text(props, "MultiLine").as_deref() == Some("true") {
        lines.push("  Многострочный: да".to_string());
    }
    if let Some(use_value) = meta_info_child_text(props, "Use") {
        if use_value != "ForItem" {
            let use_ru = match use_value.as_str() {
                "ForFolder" => "для папок",
                "ForFolderAndItem" => "для папок и элементов",
                _ => &use_value,
            };
            lines.push(format!("  Использование: {use_ru}"));
        }
    }
    if let Some(fill_value) = meta_info_child(props, "FillValue") {
        let value = meta_info_inner_text(fill_value);
        if meta_info_attr_by_local(fill_value, "nil") != Some("true") && !value.is_empty() {
            let value = match value.as_str() {
                "false" => "Ложь".to_string(),
                "true" => "Истина".to_string(),
                other if other.ends_with(".EmptyRef") => "Пустая ссылка".to_string(),
                other => other.to_string(),
            };
            lines.push(format!("  Значение заполнения: {value}"));
        } else {
            lines.push("  Значение заполнения: —".to_string());
        }
    } else {
        lines.push("  Значение заполнения: —".to_string());
    }
    if is_dimension {
        lines.push(format!(
            "  Ведущее: {}",
            if meta_info_child_text(props, "Master").as_deref() == Some("true") {
                "да"
            } else {
                "нет"
            }
        ));
        lines.push(format!(
            "  Основной отбор: {}",
            if meta_info_child_text(props, "MainFilter").as_deref() == Some("true") {
                "да"
            } else {
                "нет"
            }
        ));
    }
    if let Some(synonym) = meta_info_child(props, "Synonym")
        .map(meta_info_ml_text)
        .filter(|value| !value.is_empty() && value != name)
    {
        lines.push(format!("  Синоним: {synonym}"));
    }
    lines
}

pub(crate) fn meta_info_append_document_header(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props) = props else {
        return;
    };
    let mut parts = Vec::new();
    let number_type = meta_info_child_text(props, "NumberType");
    let number_length = meta_info_child_text(props, "NumberLength");
    if let (Some(number_type), Some(number_length)) = (number_type, number_length) {
        let type_name = if number_type == "String" {
            "Строка"
        } else {
            "Число"
        };
        let mut piece = format!("Номер: {type_name}({number_length})");
        if let Some(periodicity) = meta_info_child_text(props, "NumberPeriodicity") {
            piece.push_str(&format!(", {}", meta_info_number_period_ru(&periodicity)));
        }
        if meta_info_child_text(props, "Autonumbering").as_deref() == Some("true") {
            piece.push_str(", авто");
        }
        parts.push(piece);
    }
    if let Some(posting) = meta_info_child_text(props, "Posting") {
        parts.push(format!(
            "Проведение: {}",
            if posting == "Allow" { "да" } else { "нет" }
        ));
    }
    if !parts.is_empty() {
        lines.push(parts.join(" | "));
    }
}

/// Subordination is a first-class property of the object, so an object that
/// declares `<Owners>` always reports it. Silence would be read as "the tool
/// does not know", which is exactly the ambiguity that sends a reader to the
/// raw XML.
pub(crate) fn meta_info_append_owners(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(owners_node) = props.and_then(|node| meta_info_child(node, "Owners")) else {
        return;
    };
    let owners = meta_info_children(owners_node, "Item")
        .into_iter()
        .map(meta_info_inner_text)
        .map(|owner| owner.trim().to_string())
        .filter(|owner| !owner.is_empty())
        .collect::<Vec<_>>();
    if owners.is_empty() {
        lines.push("Владельцы: нет".to_string());
    } else {
        lines.push(format!(
            "Владельцы ({}): {}",
            owners.len(),
            owners.join(", ")
        ));
    }
}

pub(crate) fn meta_info_append_catalog_header(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props) = props else {
        return;
    };
    let mut parts = Vec::new();
    if meta_info_child_text(props, "Hierarchical").as_deref() == Some("true") {
        let mut hierarchy_type = if meta_info_child_text(props, "HierarchyType").as_deref()
            == Some("HierarchyFoldersAndItems")
        {
            "группы и элементы".to_string()
        } else {
            "элементы".to_string()
        };
        if meta_info_child_text(props, "LimitLevelCount").as_deref() == Some("true") {
            if let Some(level_count) = meta_info_child_text(props, "LevelCount") {
                hierarchy_type.push_str(&format!(", уровней: {level_count}"));
            }
        } else {
            hierarchy_type.push_str(", без ограничения уровней");
        }
        parts.push(format!("Иерархический: {hierarchy_type}"));
    } else {
        // A missing line cannot be told apart from an unreported one, so the
        // negative case is stated instead of skipped.
        parts.push("Иерархический: нет".to_string());
    }
    if let Some(code_length) = meta_info_child_text(props, "CodeLength") {
        if code_length.parse::<i64>().unwrap_or(0) > 0 {
            parts.push(format!("Код({code_length})"));
        } else {
            parts.push("Код: нет".to_string());
        }
    }
    if let Some(description_length) = meta_info_child_text(props, "DescriptionLength") {
        if description_length.parse::<i64>().unwrap_or(0) > 0 {
            parts.push(format!("Наименование({description_length})"));
        }
    }
    if let Some(presentation) = meta_info_child_text(props, "DefaultPresentation") {
        let presentation = match presentation.as_str() {
            "AsDescription" => "наименование",
            "AsCode" => "код",
            other => other,
        };
        if !presentation.is_empty() {
            parts.push(format!("Основное представление: {presentation}"));
        }
    }
    if !parts.is_empty() {
        lines.push(parts.join(" | "));
    }
}

pub(crate) fn meta_info_append_register_header(
    lines: &mut Vec<String>,
    md_type: &str,
    props: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props) = props else {
        return;
    };
    let mut parts = Vec::new();
    if md_type == "InformationRegister" {
        if let Some(periodicity) = meta_info_child_text(props, "InformationRegisterPeriodicity") {
            parts.push(format!(
                "Периодичность: {}",
                meta_info_period_ru(&periodicity)
            ));
        }
        if let Some(write_mode) = meta_info_child_text(props, "WriteMode") {
            parts.push(format!("Запись: {}", meta_info_write_mode_ru(&write_mode)));
        }
    }
    if md_type == "AccumulationRegister" {
        if let Some(register_type) = meta_info_child_text(props, "RegisterType") {
            let register_type = match register_type.as_str() {
                "Balances" => "остатки",
                "Turnovers" => "обороты",
                _ => &register_type,
            };
            parts.push(format!("Вид: {register_type}"));
        }
    }
    if !parts.is_empty() {
        lines.push(parts.join(" | "));
    }
}

pub(crate) fn meta_info_append_defined_type(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(type_node) = props.and_then(|node| meta_info_child(node, "Type")) else {
        return;
    };
    let types = meta_info_children(type_node, "Type")
        .into_iter()
        .map(|node| meta_info_format_single_type(meta_info_inner_text(node), type_node))
        .collect::<Vec<_>>();
    if types.is_empty() {
        return;
    }
    lines.push(format!("Типы ({}):", types.len()));
    for type_name in types {
        lines.push(format!("  {type_name}"));
    }
}

pub(crate) fn meta_info_append_scheduled_job(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props) = props else {
        return;
    };
    if let Some(method) =
        meta_info_child_text(props, "MethodName").filter(|value| !value.is_empty())
    {
        lines.push(format!(
            "Метод: {}",
            method.strip_prefix("CommonModule.").unwrap_or(&method)
        ));
    }
    let mut parts = Vec::new();
    parts.push(format!(
        "Использование: {}",
        if meta_info_child_text(props, "Use").as_deref() == Some("true") {
            "да"
        } else {
            "нет"
        }
    ));
    parts.push(format!(
        "Предопределённое: {}",
        if meta_info_child_text(props, "Predefined").as_deref() == Some("true") {
            "да"
        } else {
            "нет"
        }
    ));
    let restart_count = meta_info_child_text(props, "RestartCountOnFailure")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    if restart_count > 0 {
        let interval = meta_info_child_text(props, "RestartIntervalOnFailure").unwrap_or_default();
        parts.push(format!(
            "Перезапуск: {restart_count} (через {interval} сек)"
        ));
    }
    lines.push(parts.join(" | "));
}

pub(crate) fn meta_info_append_event_subscription_brief(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props) = props else {
        return;
    };
    let mut parts = Vec::new();
    if let Some(event) = meta_info_child_text(props, "Event").filter(|value| !value.is_empty()) {
        parts.push(format!("Событие: {}", meta_info_event_ru(&event)));
    }
    if let Some(handler) = meta_info_child_text(props, "Handler").filter(|value| !value.is_empty())
    {
        parts.push(format!(
            "Обработчик: {}",
            handler.strip_prefix("CommonModule.").unwrap_or(&handler)
        ));
    }
    if let Some(source) = meta_info_child(props, "Source") {
        let source_count = meta_info_children(source, "Type").len();
        if source_count > 0 {
            parts.push(format!("Источники: {source_count}"));
        }
    }
    if !parts.is_empty() {
        lines.push(parts.join(" | "));
    }
}

pub(crate) fn meta_info_append_event_subscription(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
    mode: &str,
) {
    let Some(props) = props else {
        return;
    };
    if let Some(event) = meta_info_child_text(props, "Event").filter(|value| !value.is_empty()) {
        lines.push(format!("Событие: {}", meta_info_event_ru(&event)));
    }
    if let Some(handler) = meta_info_child_text(props, "Handler").filter(|value| !value.is_empty())
    {
        lines.push(format!(
            "Обработчик: {}",
            handler.strip_prefix("CommonModule.").unwrap_or(&handler)
        ));
    }
    if let Some(source) = meta_info_child(props, "Source") {
        let source_types = meta_info_children(source, "Type")
            .into_iter()
            .map(|node| meta_info_format_source_type(&meta_info_inner_text(node)))
            .collect::<Vec<_>>();
        if !source_types.is_empty() {
            if mode == "full" {
                lines.push(format!("Источники ({}):", source_types.len()));
                for source_type in source_types {
                    lines.push(format!("  {source_type}"));
                }
            } else {
                lines.push(format!("Источники ({})", source_types.len()));
            }
        }
    }
}

pub(crate) fn meta_info_append_http_service(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
) {
    if let Some(root_url) = props.and_then(|node| meta_info_child_text(node, "RootURL")) {
        if !root_url.is_empty() {
            lines.push(format!("Корневой URL: /{root_url}"));
        }
    }
    let endpoints = meta_info_http_endpoints(child_objs);
    if endpoints.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push(format!("Шаблоны URL ({}):", endpoints.len()));
    for endpoint in endpoints {
        lines.push(format!("  {}", endpoint.template));
        for method in endpoint.methods {
            lines.push(format!(
                "    {:<6} → {}",
                method.http_method, method.handler
            ));
        }
    }
}

pub(crate) fn meta_info_append_web_service(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
) {
    if let Some(namespace) = props.and_then(|node| meta_info_child_text(node, "Namespace")) {
        if !namespace.is_empty() {
            lines.push(format!("Пространство имён: {namespace}"));
        }
    }
    let operations = meta_info_ws_operations(child_objs);
    if operations.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push(format!("Операции ({}):", operations.len()));
    for operation in operations {
        lines.push(format!(
            "  {}({}) → {}",
            operation.name, operation.params, operation.return_type
        ));
    }
}

pub(crate) fn meta_info_append_enum_values(
    lines: &mut Vec<String>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_objs) = child_objs else {
        return;
    };
    let values = meta_info_children(child_objs, "EnumValue")
        .into_iter()
        .filter_map(|value| {
            let props = meta_info_child(value, "Properties")?;
            let name = meta_info_child_text(props, "Name").unwrap_or_default();
            let synonym = meta_info_child(props, "Synonym")
                .map(meta_info_ml_text)
                .unwrap_or_default();
            Some((name, synonym))
        })
        .collect::<Vec<_>>();
    if values.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push(format!("Значения ({}):", values.len()));
    let max_len = values
        .iter()
        .map(|(name, _)| name.chars().count())
        .max()
        .unwrap_or(10)
        .max(10)
        + 2;
    for (name, synonym) in values {
        let synonym_text = if !synonym.is_empty() && synonym != name {
            format!("\"{synonym}\"")
        } else {
            String::new()
        };
        lines.push(format!("  {name:<max_len$} {synonym_text}"));
    }
}

pub(crate) fn meta_info_append_attribute_section(
    lines: &mut Vec<String>,
    header: &str,
    child_objs: Option<roxmltree::Node<'_, '_>>,
    child_tag: &str,
    is_dimension: bool,
) {
    let attrs = meta_info_attributes(child_objs, child_tag, is_dimension);
    if attrs.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push(format!("{header} ({}):", attrs.len()));
    let sorted_attrs = meta_info_sort_attrs_ref_first(attrs);
    let width = meta_info_max_name_len(&sorted_attrs);
    for attr in sorted_attrs {
        lines.push(meta_info_format_attr_line(&attr, width));
    }
}

pub(crate) fn meta_info_append_tabular_sections(
    lines: &mut Vec<String>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
    mode: &str,
) {
    let tabular_sections = meta_info_tabular_sections(child_objs);
    if tabular_sections.is_empty() {
        return;
    }
    if mode == "full" {
        for section in tabular_sections {
            lines.push(String::new());
            lines.push(format!(
                "ТЧ {} ({} {}):",
                section.name,
                section.columns.len(),
                meta_info_decline_cols(section.columns.len())
            ));
            if !section.columns.is_empty() {
                let sorted_cols = meta_info_sort_attrs_ref_first(section.columns);
                let width = meta_info_max_name_len(&sorted_cols);
                for column in sorted_cols {
                    lines.push(meta_info_format_attr_line(&column, width));
                }
            }
        }
    } else {
        lines.push(String::new());
        let parts = tabular_sections
            .iter()
            .map(|section| format!("{}({})", section.name, section.columns.len()))
            .collect::<Vec<_>>();
        lines.push(format!(
            "ТЧ ({}): {}",
            tabular_sections.len(),
            parts.join(", ")
        ));
    }
}

pub(crate) fn meta_info_append_simple_children(
    lines: &mut Vec<String>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
) {
    let forms = meta_info_simple_children(child_objs, "Form");
    if !forms.is_empty() {
        lines.push(format!("Формы: {}", forms.join(", ")));
    }
    let templates = meta_info_simple_children(child_objs, "Template");
    if !templates.is_empty() {
        lines.push(format!("Макеты: {}", templates.join(", ")));
    }
    let commands = meta_info_simple_children(child_objs, "Command");
    if !commands.is_empty() {
        lines.push(format!("Команды: {}", commands.join(", ")));
    }
}

pub(crate) fn meta_info_append_full_tail(
    lines: &mut Vec<String>,
    md_type: &str,
    props: Option<roxmltree::Node<'_, '_>>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
) {
    if md_type == "Document" {
        let Some(props) = props else {
            return;
        };
        let register_records = meta_info_child(props, "RegisterRecords")
            .map(|node| {
                meta_info_children(node, "Item")
                    .into_iter()
                    .map(|item| {
                        let raw = meta_info_inner_text(item);
                        if let Some((prefix, name)) = raw.split_once('.') {
                            format!("{}.{}", meta_info_register_short(prefix), name)
                        } else {
                            raw
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !register_records.is_empty() {
            lines.push(String::new());
            lines.push(format!(
                "Движения ({}): {}",
                register_records.len(),
                register_records.join(", ")
            ));
        }
        let based_on = meta_info_child(props, "BasedOn")
            .map(|node| {
                meta_info_children(node, "Item")
                    .into_iter()
                    .map(|item| {
                        let raw = meta_info_inner_text(item);
                        raw.split_once('.')
                            .map(|(_, name)| name.to_string())
                            .unwrap_or(raw)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !based_on.is_empty() {
            lines.push(format!("Ввод на основании: {}", based_on.join(", ")));
        }
    }
    meta_info_append_simple_children(lines, child_objs);
}

pub(crate) fn meta_info_attributes<'a, 'input>(
    parent_node: Option<roxmltree::Node<'a, 'input>>,
    child_tag: &str,
    is_dimension: bool,
) -> Vec<MetaInfoAttr<'a, 'input>> {
    let Some(parent_node) = parent_node else {
        return Vec::new();
    };
    meta_info_children(parent_node, child_tag)
        .into_iter()
        .filter_map(|attr| {
            let props = meta_info_child(attr, "Properties")?;
            let name = meta_info_child_text(props, "Name").unwrap_or_default();
            let type_name = meta_info_child(props, "Type")
                .map(meta_info_format_type)
                .unwrap_or_default();
            let flags = meta_info_format_flags(props, is_dimension);
            Some(MetaInfoAttr {
                name,
                type_name,
                flags,
                _marker: std::marker::PhantomData,
            })
        })
        .collect()
}

pub(crate) fn meta_info_tabular_sections<'a, 'input>(
    parent_node: Option<roxmltree::Node<'a, 'input>>,
) -> Vec<MetaInfoTabularSection<'a, 'input>> {
    let Some(parent_node) = parent_node else {
        return Vec::new();
    };
    meta_info_children(parent_node, "TabularSection")
        .into_iter()
        .map(|section| {
            let props = meta_info_child(section, "Properties");
            let name = props
                .and_then(|node| meta_info_child_text(node, "Name"))
                .unwrap_or_default();
            let columns =
                meta_info_attributes(meta_info_child(section, "ChildObjects"), "Attribute", false);
            MetaInfoTabularSection { name, columns }
        })
        .collect()
}

pub(crate) fn meta_info_http_endpoints(
    child_objs: Option<roxmltree::Node<'_, '_>>,
) -> Vec<MetaInfoHttpEndpoint> {
    let Some(child_objs) = child_objs else {
        return Vec::new();
    };
    meta_info_children(child_objs, "URLTemplate")
        .into_iter()
        .map(|template| {
            let props = meta_info_child(template, "Properties");
            let name = props
                .and_then(|node| meta_info_child_text(node, "Name"))
                .unwrap_or_default();
            let template_path = props
                .and_then(|node| meta_info_child_text(node, "Template"))
                .unwrap_or_default();
            let methods = meta_info_child(template, "ChildObjects")
                .map(|node| {
                    meta_info_children(node, "Method")
                        .into_iter()
                        .map(|method| {
                            let props = meta_info_child(method, "Properties");
                            MetaInfoHttpMethod {
                                http_method: props
                                    .and_then(|node| meta_info_child_text(node, "HTTPMethod"))
                                    .unwrap_or_default(),
                                handler: props
                                    .and_then(|node| meta_info_child_text(node, "Handler"))
                                    .unwrap_or_default(),
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            MetaInfoHttpEndpoint {
                name,
                template: template_path,
                methods,
            }
        })
        .collect()
}

pub(crate) fn meta_info_ws_operations(
    child_objs: Option<roxmltree::Node<'_, '_>>,
) -> Vec<MetaInfoWsOperation> {
    let Some(child_objs) = child_objs else {
        return Vec::new();
    };
    meta_info_children(child_objs, "Operation")
        .into_iter()
        .map(|operation| {
            let props = meta_info_child(operation, "Properties");
            let params = meta_info_child(operation, "ChildObjects")
                .map(|node| {
                    meta_info_children(node, "Parameter")
                        .into_iter()
                        .map(|param| {
                            let props = meta_info_child(param, "Properties");
                            let name = props
                                .and_then(|node| meta_info_child_text(node, "Name"))
                                .unwrap_or_default();
                            let type_name = props
                                .and_then(|node| meta_info_child_text(node, "XDTOValueType"))
                                .filter(|value| !value.is_empty())
                                .unwrap_or_else(|| "?".to_string());
                            let direction = props
                                .and_then(|node| meta_info_child_text(node, "TransferDirection"))
                                .filter(|value| value != "In")
                                .map(|value| format!(" [{}]", value.to_lowercase()))
                                .unwrap_or_default();
                            format!("{name}: {type_name}{direction}")
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
                .join(", ");
            let return_type = props
                .and_then(|node| meta_info_child_text(node, "XDTOReturningValueType"))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "void".to_string());
            MetaInfoWsOperation {
                name: props
                    .and_then(|node| meta_info_child_text(node, "Name"))
                    .unwrap_or_default(),
                params,
                return_type,
                proc_name: props
                    .and_then(|node| meta_info_child_text(node, "ProcedureName"))
                    .unwrap_or_default(),
            }
        })
        .collect()
}

pub(crate) fn meta_info_common_module_flags(props: Option<roxmltree::Node<'_, '_>>) -> Vec<String> {
    let Some(props) = props else {
        return Vec::new();
    };
    let mut flags = Vec::new();
    for (flag_name, flag_label) in [
        ("Global", "Глобальный"),
        ("Server", "Сервер"),
        ("ServerCall", "Вызов сервера"),
        ("ClientManagedApplication", "Клиент управляемое"),
        ("ClientOrdinaryApplication", "Обычный клиент"),
        ("ExternalConnection", "Внешнее соединение"),
        ("Privileged", "Привилегированный"),
    ] {
        if meta_info_child_text(props, flag_name).as_deref() == Some("true") {
            flags.push(flag_label.to_string());
        }
    }
    if let Some(reuse) =
        meta_info_child_text(props, "ReturnValuesReuse").filter(|value| value != "DontUse")
    {
        flags.push(format!(
            "Повторное использование: {}",
            meta_info_reuse_ru(&reuse)
        ));
    }
    flags
}

pub(crate) fn meta_info_format_type(type_node: roxmltree::Node<'_, '_>) -> String {
    let mut types = Vec::new();
    for type_item in meta_info_children(type_node, "Type") {
        types.push(meta_info_format_single_type(
            meta_info_inner_text(type_item),
            type_node,
        ));
    }
    for type_set in meta_info_children(type_node, "TypeSet") {
        let raw = meta_info_inner_text(type_set);
        if let Some(name) = raw.strip_prefix("cfg:DefinedType.") {
            types.push(format!("ОпределяемыйТип.{name}"));
        } else if let Some(name) = raw.strip_prefix("cfg:Characteristic.") {
            types.push(format!("Характеристика.{name}"));
        } else {
            types.push(raw);
        }
    }
    types.join(" | ")
}

pub(crate) fn meta_info_format_single_type(
    raw: String,
    parent_node: roxmltree::Node<'_, '_>,
) -> String {
    match raw.as_str() {
        "xs:string" => {
            let length = meta_info_child(parent_node, "StringQualifiers")
                .and_then(|node| meta_info_child_text(node, "Length"))
                .unwrap_or_default();
            if length.is_empty() {
                "Строка".to_string()
            } else {
                format!("Строка({length})")
            }
        }
        "xs:decimal" => {
            let qualifiers = meta_info_child(parent_node, "NumberQualifiers");
            let digits = qualifiers
                .and_then(|node| meta_info_child_text(node, "Digits"))
                .unwrap_or_default();
            let fraction = qualifiers
                .and_then(|node| meta_info_child_text(node, "FractionDigits"))
                .unwrap_or_else(|| "0".to_string());
            if digits.is_empty() {
                "Число".to_string()
            } else {
                format!("Число({digits},{fraction})")
            }
        }
        "xs:boolean" => "Булево".to_string(),
        "xs:dateTime" => {
            let date_fraction = meta_info_child(parent_node, "DateQualifiers")
                .and_then(|node| meta_info_child_text(node, "DateFractions"));
            match date_fraction.as_deref() {
                Some("Date") => "Дата".to_string(),
                Some("Time") => "Время".to_string(),
                Some("DateTime") => "ДатаВремя".to_string(),
                Some(_) => "Дата".to_string(),
                None => "ДатаВремя".to_string(),
            }
        }
        "v8:ValueStorage" => "ХранилищеЗначения".to_string(),
        "v8:UUID" => "УникальныйИдентификатор".to_string(),
        "v8:Null" => "Null".to_string(),
        _ => meta_info_format_cfg_type(&raw),
    }
}

pub(crate) fn meta_info_format_cfg_type(raw: &str) -> String {
    let normalized = meta_info_normalize_cfg_prefix(raw);
    if let Some(rest) = normalized.strip_prefix("cfg:") {
        if let Some((prefix, name)) = rest.split_once('.') {
            if let Some(ref_type) = meta_info_ref_type_ru(prefix) {
                return format!("{ref_type}.{name}");
            }
            if prefix == "Characteristic" {
                return format!("Характеристика.{name}");
            }
            if prefix == "DefinedType" {
                return format!("ОпределяемыйТип.{name}");
            }
        }
        return rest.to_string();
    }
    normalized
}

pub(crate) fn meta_info_format_flags(props: roxmltree::Node<'_, '_>, is_dimension: bool) -> String {
    let mut flags = Vec::new();
    if meta_info_child_text(props, "FillChecking").as_deref() == Some("ShowError") {
        flags.push("обязательный");
    }
    if let Some(indexing) = meta_info_child_text(props, "Indexing") {
        match indexing.as_str() {
            "Index" => flags.push("индекс"),
            "IndexWithAdditionalOrder" => flags.push("индекс+доп"),
            _ => {}
        }
    }
    if is_dimension && meta_info_child_text(props, "Master").as_deref() == Some("true") {
        flags.push("ведущее");
    }
    if meta_info_child_text(props, "MultiLine").as_deref() == Some("true") {
        flags.push("многострочный");
    }
    if let Some(use_value) = meta_info_child_text(props, "Use") {
        match use_value.as_str() {
            "ForFolder" => flags.push("для папок"),
            "ForFolderAndItem" => flags.push("для папок и элементов"),
            _ => {}
        }
    }
    if flags.is_empty() {
        String::new()
    } else {
        format!("  [{}]", flags.join(", "))
    }
}

pub(crate) fn meta_info_sort_attrs_ref_first<'a, 'input>(
    attrs: Vec<MetaInfoAttr<'a, 'input>>,
) -> Vec<MetaInfoAttr<'a, 'input>> {
    let mut refs = Vec::new();
    let mut prims = Vec::new();
    for attr in attrs {
        if meta_info_type_is_reference(&attr.type_name) {
            refs.push(attr);
        } else {
            prims.push(attr);
        }
    }
    refs.extend(prims);
    refs
}

pub(crate) fn meta_info_type_is_reference(type_name: &str) -> bool {
    type_name.contains("Ссылка.")
        || type_name.contains("Характеристика.")
        || type_name.contains("ОпределяемыйТип.")
        || type_name.contains("ПланСчетовСсылка")
        || type_name.contains("ПВХСсылка")
        || type_name.contains("ПВРСсылка")
}

pub(crate) fn meta_info_format_attr_line(attr: &MetaInfoAttr<'_, '_>, width: usize) -> String {
    format!("  {:<width$} {}{}", attr.name, attr.type_name, attr.flags)
}

pub(crate) fn meta_info_max_name_len(attrs: &[MetaInfoAttr<'_, '_>]) -> usize {
    let max_len = attrs
        .iter()
        .map(|attr| attr.name.chars().count())
        .max()
        .unwrap_or(10)
        .max(10);
    (max_len + 2).min(40)
}

pub(crate) fn meta_info_simple_children(
    parent_node: Option<roxmltree::Node<'_, '_>>,
    tag: &str,
) -> Vec<String> {
    let Some(parent_node) = parent_node else {
        return Vec::new();
    };
    meta_info_children(parent_node, tag)
        .into_iter()
        .map(meta_info_inner_text)
        .collect()
}

pub(crate) fn meta_info_enum_values(parent_node: Option<roxmltree::Node<'_, '_>>) -> Vec<String> {
    let Some(parent_node) = parent_node else {
        return Vec::new();
    };
    meta_info_children(parent_node, "EnumValue")
        .into_iter()
        .filter_map(|value| {
            meta_info_child(value, "Properties")
                .and_then(|props| meta_info_child_text(props, "Name"))
        })
        .collect()
}

pub(crate) fn meta_info_paginate(lines: Vec<String>, args: &Map<String, Value>) -> String {
    let total_lines = lines.len();
    let offset = int_arg(args, &["offset", "Offset"]).unwrap_or(0).max(0) as usize;
    let limit = int_arg(args, &["limit", "Limit"]).unwrap_or(150).max(0) as usize;
    if offset >= total_lines && offset > 0 {
        return format!(
            "[INFO] Offset {offset} exceeds total lines ({total_lines}). Nothing to show."
        );
    }
    let mut out_lines = if offset > 0 {
        lines.into_iter().skip(offset).collect::<Vec<_>>()
    } else {
        lines
    };
    if limit > 0 && out_lines.len() > limit {
        let mut shown = out_lines.drain(..limit).collect::<Vec<_>>();
        shown.push(String::new());
        shown.push(format!(
            "[ОБРЕЗАНО] Показано {limit} из {total_lines} строк. Используйте -Offset {} для продолжения.",
            offset + limit
        ));
        out_lines = shown;
    }
    out_lines.join("\n")
}

pub(crate) fn meta_info_type_ru(md_type: &str) -> String {
    match md_type {
        "Catalog" => "Справочник",
        "Document" => "Документ",
        "Enum" => "Перечисление",
        "Constant" => "Константа",
        "InformationRegister" => "Регистр сведений",
        "AccumulationRegister" => "Регистр накопления",
        "AccountingRegister" => "Регистр бухгалтерии",
        "CalculationRegister" => "Регистр расчёта",
        "ChartOfAccounts" => "План счетов",
        "ChartOfCharacteristicTypes" => "План видов характеристик",
        "ChartOfCalculationTypes" => "План видов расчёта",
        "BusinessProcess" => "Бизнес-процесс",
        "Task" => "Задача",
        "ExchangePlan" => "План обмена",
        "DocumentJournal" => "Журнал документов",
        "Report" => "Отчёт",
        "DataProcessor" => "Обработка",
        "DefinedType" => "Определяемый тип",
        "CommonModule" => "Общий модуль",
        "ScheduledJob" => "Регламентное задание",
        "EventSubscription" => "Подписка на событие",
        "HTTPService" => "HTTP-сервис",
        "WebService" => "Веб-сервис",
        _ => md_type,
    }
    .to_string()
}

pub(crate) fn meta_info_is_reference_metadata_type(md_type: &str) -> bool {
    matches!(
        md_type,
        "Catalog"
            | "Document"
            | "Enum"
            | "ChartOfAccounts"
            | "ChartOfCharacteristicTypes"
            | "ChartOfCalculationTypes"
            | "ExchangePlan"
            | "BusinessProcess"
            | "Task"
    )
}

pub(crate) fn meta_info_ref_type_ru(prefix: &str) -> Option<&'static str> {
    match prefix {
        "CatalogRef" => Some("СправочникСсылка"),
        "DocumentRef" => Some("ДокументСсылка"),
        "EnumRef" => Some("ПеречислениеСсылка"),
        "ChartOfAccountsRef" => Some("ПланСчетовСсылка"),
        "ChartOfCharacteristicTypesRef" => Some("ПВХСсылка"),
        "ChartOfCalculationTypesRef" => Some("ПВРСсылка"),
        "ExchangePlanRef" => Some("ПланОбменаСсылка"),
        "BusinessProcessRef" => Some("БизнесПроцессСсылка"),
        "TaskRef" => Some("ЗадачаСсылка"),
        _ => None,
    }
}

pub(crate) fn meta_info_object_type_ru(prefix: &str) -> Option<&'static str> {
    match prefix {
        "CatalogObject" => Some("СправочникОбъект"),
        "DocumentObject" => Some("ДокументОбъект"),
        "ChartOfAccountsObject" => Some("ПланСчетовОбъект"),
        "ChartOfCharacteristicTypesObject" => Some("ПВХОбъект"),
        "BusinessProcessObject" => Some("БизнесПроцессОбъект"),
        "TaskObject" => Some("ЗадачаОбъект"),
        "ExchangePlanObject" => Some("ПланОбменаОбъект"),
        "InformationRegisterRecordSet" => Some("НаборЗаписейРС"),
        "AccumulationRegisterRecordSet" => Some("НаборЗаписейРН"),
        "AccountingRegisterRecordSet" => Some("НаборЗаписейРБ"),
        _ => None,
    }
}

pub(crate) fn meta_info_period_ru(value: &str) -> &str {
    match value {
        "Nonperiodical" => "Непериодический",
        "Day" => "День",
        "Month" => "Месяц",
        "Quarter" => "Квартал",
        "Year" => "Год",
        "Second" => "Секунда",
        _ => value,
    }
}

pub(crate) fn meta_info_write_mode_ru(value: &str) -> &str {
    match value {
        "Independent" => "независимая",
        "RecorderSubordinate" => "подчинение регистратору",
        _ => value,
    }
}

pub(crate) fn meta_info_reuse_ru(value: &str) -> &str {
    match value {
        "DontUse" => "нет",
        "DuringRequest" => "на время вызова",
        "DuringSession" => "на время сеанса",
        _ => value,
    }
}

pub(crate) fn meta_info_event_ru(value: &str) -> &str {
    match value {
        "BeforeWrite" => "ПередЗаписью",
        "OnWrite" => "ПриЗаписи",
        "AfterWrite" => "ПослеЗаписи",
        "BeforeDelete" => "ПередУдалением",
        "Posting" => "ОбработкаПроведения",
        "UndoPosting" => "ОбработкаУдаленияПроведения",
        "OnReadAtServer" => "ПриЧтенииНаСервере",
        "FillCheckProcessing" => "ОбработкаПроверкиЗаполнения",
        _ => value,
    }
}

pub(crate) fn meta_info_number_period_ru(value: &str) -> &str {
    match value {
        "Year" => "по году",
        "Quarter" => "по кварталу",
        "Month" => "по месяцу",
        "Day" => "по дню",
        "WholeCatalog" => "сквозная",
        _ => value,
    }
}

pub(crate) fn meta_info_register_short(value: &str) -> &str {
    match value {
        "AccumulationRegister" => "РН",
        "AccountingRegister" => "РБ",
        "CalculationRegister" => "РР",
        "InformationRegister" => "РС",
        _ => value,
    }
}

pub(crate) fn meta_info_decline_cols(n: usize) -> &'static str {
    let m = n % 10;
    let h = n % 100;
    if (11..=19).contains(&h) {
        "колонок"
    } else if m == 1 {
        "колонка"
    } else if (2..=4).contains(&m) {
        "колонки"
    } else {
        "колонок"
    }
}

pub(crate) struct MetaRemoveError {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) message: String,
}
