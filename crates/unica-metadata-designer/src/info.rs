use roxmltree::Document;
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_metadata::*;
pub fn typed_properties(
    properties: Option<roxmltree::Node<'_, '_>>,
    kind: MetadataKind,
) -> Vec<MetaInfoPropertyData> {
    let Some(properties) = properties else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    properties
        .children()
        .filter(roxmltree::Node::is_element)
        .filter_map(|node| {
            let name = node.tag_name().name();
            if !seen.insert(name) {
                return None;
            }
            let spec = META_INFO_PROPERTY_PROFILE.resolve(kind, name)?;
            let value = crate::info_projection::observe_meta_info_property(properties, kind, name)
                .ok()??;
            Some(MetaInfoPropertyData {
                key: spec.key,
                value,
            })
        })
        .collect()
}
pub fn typed_relations(
    document: &Document<'_>,
    properties: Option<roxmltree::Node<'_, '_>>,
    owner_kind: MetadataKind,
    target: &MetadataAddress,
    diagnostics: &mut Vec<MetaDiagnostic>,
) -> MetaRelationsData {
    let (owners, register_records, based_on, input_by_string, data_lock_fields) = {
        let mut read = |tag: &str, public_name: &str, kind: &str| {
            if !crate::info_projection::meta_info_relation_is_applicable(owner_kind, tag) {
                return None;
            }
            let Some(properties) = properties else {
                return Some(None);
            };
            let containers = properties
                .children()
                .filter(|node| node.is_element() && node.tag_name().name() == tag)
                .collect::<Vec<_>>();
            if containers.iter().any(|node| {
                node.tag_name().namespace() != Some(super::info_projection::MD_CLASSES_NAMESPACE)
            }) {
                diagnostics.push(
                    MetaDiagnostic::error(
                        MetaDiagnosticCode::ValidationFailed,
                        "metadata relation container has a foreign namespace",
                    )
                    .with_metadata_path(target.clone())
                    .with_field(format!("relations.{public_name}")),
                );
                return Some(None);
            }
            let [container] = containers.as_slice() else {
                if containers.len() > 1 {
                    diagnostics.push(
                        MetaDiagnostic::error(
                            MetaDiagnosticCode::ProviderUnavailable,
                            "metadata relation container occurs more than once",
                        )
                        .with_metadata_path(target.clone())
                        .with_field(format!("relations.{public_name}")),
                    );
                }
                return Some(None);
            };
            if container.attributes().len() != 0
                || container
                    .children()
                    .filter(roxmltree::Node::is_text)
                    .any(|node| node.text().is_some_and(|text| !text.trim().is_empty()))
            {
                diagnostics.push(
                    MetaDiagnostic::error(
                        MetaDiagnosticCode::ValidationFailed,
                        "metadata relation container is malformed",
                    )
                    .with_metadata_path(target.clone())
                    .with_field(format!("relations.{public_name}")),
                );
                return Some(None);
            }
            let mut values = Vec::new();
            let mut complete = true;
            for (index, node) in container
                .children()
                .filter(|node| node.is_element())
                .enumerate()
            {
                let expected_tag = if kind == "field" { "Field" } else { "Item" };
                let base_field = format!("relations.{public_name}[{index}]");
                if node.tag_name().namespace() != Some(super::info_projection::READABLE_NAMESPACE)
                    || node.tag_name().name() != expected_tag
                    || node.children().any(|child| child.is_element())
                {
                    diagnostics.push(
                        MetaDiagnostic::error(
                            MetaDiagnosticCode::ValidationFailed,
                            "metadata relation entry has an unexpected structure",
                        )
                        .with_metadata_path(target.clone())
                        .with_field(base_field),
                    );
                    complete = false;
                    continue;
                }
                if kind == "object" {
                    let xsi_type = node
                        .attributes()
                        .find(|attribute| {
                            attribute.namespace()
                                == Some("http://www.w3.org/2001/XMLSchema-instance")
                                && attribute.name() == "type"
                        })
                        .map(|attribute| attribute.value());
                    if node.attributes().len() != 1
                        || !xsi_type.is_some_and(|value| {
                            crate::info_projection::qname_resolves_to(
                                node,
                                value,
                                super::info_projection::READABLE_NAMESPACE,
                                "MDObjectRef",
                            )
                        })
                    {
                        diagnostics.push(
                            MetaDiagnostic::error(
                                MetaDiagnosticCode::ValidationFailed,
                                "metadata object relation has the wrong xsi:type",
                            )
                            .with_metadata_path(target.clone())
                            .with_field(base_field),
                        );
                        complete = false;
                        continue;
                    }
                } else if node.attributes().len() != 0 {
                    diagnostics.push(
                        MetaDiagnostic::error(
                            MetaDiagnosticCode::ValidationFailed,
                            "metadata field relation has unexpected attributes",
                        )
                        .with_metadata_path(target.clone())
                        .with_field(base_field),
                    );
                    complete = false;
                    continue;
                }
                let raw = unica_xml_format::read::direct_text(node);
                let normalized = crate::xml_model::meta_info_normalize_cfg_prefix(raw.trim());
                let normalized = normalized.strip_prefix("cfg:").unwrap_or(&normalized);
                let valid = if kind == "field" {
                    unica_metadata::MetadataFieldPath::parse(normalized).is_ok()
                } else {
                    MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, normalized).is_ok()
                };
                if !valid {
                    diagnostics.push(
                        MetaDiagnostic::error(
                            MetaDiagnosticCode::ValidationFailed,
                            "metadata relation target is malformed",
                        )
                        .with_metadata_path(target.clone())
                        .with_field(base_field),
                    );
                    complete = false;
                    continue;
                }
                values.push(MetaRelationTargetData {
                    kind: kind.to_string(),
                    value: normalized.to_string(),
                });
            }
            Some(complete.then_some(values))
        };
        let owners = read("Owners", "owners", "object");
        let register_records = read("RegisterRecords", "registerRecords", "object");
        let based_on = read("BasedOn", "basedOn", "object");
        let input_by_string = read("InputByString", "inputByString", "field");
        let data_lock_fields = read("DataLockFields", "dataLockFields", "field");
        (
            owners,
            register_records,
            based_on,
            input_by_string,
            data_lock_fields,
        )
    };
    let source = match crate::xml_model::meta_event_subscription_source_node(document)
        .and_then(crate::xml_model::parse_meta_event_subscription_source)
    {
        Ok(source) => source,
        Err(message) if target.segments().next() == Some("EventSubscription") => {
            diagnostics.push(
                MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, message)
                    .with_metadata_path(target.clone())
                    .with_field("relations.source"),
            );
            Default::default()
        }
        Err(_) => Default::default(),
    };
    MetaRelationsData {
        owners,
        register_records,
        based_on,
        input_by_string,
        data_lock_fields,
        source: source.sources,
        source_type_ids: source.type_ids,
    }
}
pub fn typed_elements_with_diagnostics(
    xml: &str,
    parent: Option<roxmltree::Node<'_, '_>>,
    tag: &str,
    nested_attributes: bool,
    field: &str,
    target: &MetadataAddress,
    diagnostics: &mut Vec<MetaDiagnostic>,
) -> Vec<MetaElementData> {
    typed_elements_profiled(
        xml,
        parent,
        ElementReadRoute {
            tag,
            nested_attributes,
            field,
            profile: None,
        },
        target,
        diagnostics,
    )
}
pub struct ElementReadRoute<'a> {
    pub tag: &'a str,
    pub nested_attributes: bool,
    pub field: &'a str,
    pub profile: Option<unica_metadata::MetaMemberReadProfile>,
}
pub fn typed_elements_profiled(
    xml: &str,
    parent: Option<roxmltree::Node<'_, '_>>,
    route: ElementReadRoute<'_>,
    target: &MetadataAddress,
    diagnostics: &mut Vec<MetaDiagnostic>,
) -> Vec<MetaElementData> {
    let Some(parent) = parent else {
        return Vec::new();
    };
    crate::info_projection::direct_children_with_namespace(
        parent,
        super::info_projection::MD_CLASSES_NAMESPACE,
        route.tag,
    )
    .into_iter()
    .enumerate()
    .filter_map(|(index, node)| {
        typed_element(
            xml,
            node,
            route.nested_attributes,
            &format!("{}[{index}]", route.field),
            target,
            diagnostics,
            route.profile,
        )
    })
    .collect()
}
pub fn typed_element(
    xml: &str,
    node: roxmltree::Node<'_, '_>,
    nested_attributes: bool,
    field: &str,
    target: &MetadataAddress,
    diagnostics: &mut Vec<MetaDiagnostic>,
    member_profile: Option<unica_metadata::MetaMemberReadProfile>,
) -> Option<MetaElementData> {
    let properties_readback =
        match crate::info_projection::project_optional_collection_item_properties(node, field) {
            Ok(properties) => properties,
            Err(error) => {
                diagnostics.push(
                    MetaDiagnostic::error(error.code, error.message)
                        .with_metadata_path(target.clone())
                        .with_field(error.field),
                );
                return None;
            }
        };
    let Some(properties) = crate::info_projection::direct_md_child(node, "Properties") else {
        let name = unica_xml_format::read::direct_text(node).trim().to_string();
        let simple_reference = matches!(
            node.tag_name().name(),
            "Form" | "Template" | "Command" | "Recalculation"
        );
        if simple_reference && name.is_empty() {
            return None;
        }
        return Some(MetaElementData {
            name,
            incomplete: !simple_reference,
            synonym: None,
            comment: None,
            r#type: None,
            required: None,
            fill_value: None,
            addressing_dimension: None,
            properties: properties_readback,
            standard_attributes: None,
            property_limits: Vec::new(),
            attributes: Vec::new(),
        });
    };
    // Диагностики самого элемента (тип, значение заполнения, флаг
    // обязательности, измерение адресации) — с этого места; канонический
    // `view` узнаёт о них из пределов элемента, а не из общего списка (#1933).
    let own_diagnostics = diagnostics.len();
    let raw_name =
        crate::info_projection::direct_md_child_text(properties, "Name").unwrap_or_default();
    let mut incomplete = raw_name.trim().is_empty();
    let name = if incomplete { String::new() } else { raw_name };
    let synonym = crate::info_projection::direct_md_child(properties, "Synonym")
        .map(crate::xml_model::meta_info_ml_text)
        .filter(|value| !value.is_empty());
    let comment = crate::info_projection::direct_md_child_text(properties, "Comment")
        .filter(|value| !value.is_empty());
    let r#type = if crate::info_projection::direct_md_child(properties, "Type").is_some() {
        match crate::info_projection::parse_observed_metadata_type_node(properties) {
            Ok(value) => Some(value),
            Err(diagnostic) => {
                incomplete = true;
                let unmodelled =
                    crate::info_projection::observed_type_is_strict_but_unmodelled(properties);
                let typed = if unmodelled {
                    MetaDiagnostic::warning(
                        MetaDiagnosticCode::ValidationWarning,
                        "metadata type is syntactically valid but not modelled by this format profile",
                    )
                } else {
                    MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, diagnostic.message)
                };
                diagnostics.push(
                    typed
                        .with_metadata_path(target.clone())
                        .with_field(format!("{field}.type")),
                );
                None
            }
        }
    } else {
        None
    };
    let fill_value = if crate::info_projection::direct_md_child(properties, "FillValue").is_some() {
        match crate::edit::parse_typed_fill_value_node(properties) {
            Ok(value) => value,
            Err(diagnostic) => {
                incomplete = true;
                diagnostics.push(
                    MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, diagnostic.message)
                        .with_metadata_path(target.clone())
                        .with_field(format!("{field}.fillValue")),
                );
                None
            }
        }
    } else {
        None
    };
    let required =
        match crate::info_projection::direct_md_child_text(properties, "FillChecking").as_deref() {
            Some("ShowError") => Some(true),
            Some("DontCheck") => Some(false),
            None => None,
            Some(_) => {
                incomplete = true;
                diagnostics.push(
                    MetaDiagnostic::error(
                        MetaDiagnosticCode::ValidationFailed,
                        "metadata required flag is malformed",
                    )
                    .with_metadata_path(target.clone())
                    .with_field(format!("{field}.required")),
                );
                None
            }
        };
    let addressing_dimension = if node.tag_name().name() == "AddressingAttribute" {
        match crate::info_projection::direct_md_child_text(properties, "AddressingDimension")
            .filter(|value| !value.trim().is_empty())
        {
            Some(value) if addressing_dimension_is_valid(value.trim()) => Some(value),
            _ => {
                incomplete = true;
                diagnostics.push(
                    MetaDiagnostic::error(
                        MetaDiagnosticCode::ValidationFailed,
                        "addressing attribute dimension is missing or malformed",
                    )
                    .with_metadata_path(target.clone())
                    .with_field(format!("{field}.addressingDimension")),
                );
                None
            }
        }
    } else {
        None
    };
    let own_limits = diagnostics[own_diagnostics..]
        .iter()
        .map(element_diagnostic_limit)
        .collect::<Vec<_>>();
    let standard_attributes = if node.tag_name().name() == "TabularSection" {
        match crate::info_projection::observe_tabular_standard_attributes(
            properties,
            &format!("{field}.standardAttributes"),
        ) {
            Ok(value) => value.map(Some),
            Err(error) => {
                incomplete = true;
                diagnostics.push(
                    MetaDiagnostic::error(error.code, error.message)
                        .with_metadata_path(target.clone())
                        .with_field(error.field),
                );
                Some(None)
            }
        }
    } else {
        None
    };
    let attributes = if nested_attributes {
        typed_elements_profiled(
            xml,
            crate::info_projection::direct_md_child(node, "ChildObjects"),
            ElementReadRoute {
                tag: "Attribute",
                nested_attributes: false,
                field: &format!("{field}.attributes"),
                profile: member_profile,
            },
            target,
            diagnostics,
        )
    } else {
        Vec::new()
    };
    let (profile_properties, property_limits) = member_profile.map_or_else(
        || (Vec::new(), Vec::new()),
        |profile| super::member_properties::observe(node, profile, field, target, diagnostics),
    );
    incomplete |= !property_limits.is_empty();
    let property_limits = own_limits.into_iter().chain(property_limits).collect();
    let properties_readback = properties_readback
        .into_iter()
        .chain(profile_properties)
        .collect();
    Some(MetaElementData {
        name,
        incomplete,
        synonym,
        comment,
        r#type,
        required,
        fill_value,
        addressing_dimension,
        properties: properties_readback,
        standard_attributes,
        property_limits,
        attributes,
    })
}
/// Предел элемента по его диагностике: поле, код и сообщение. Код отличает
/// предупреждение о немоделируемом типе от повреждённой записи.
fn element_diagnostic_limit(diagnostic: &MetaDiagnostic) -> String {
    let code = serde_json::to_value(diagnostic.code)
        .ok()
        .and_then(|code| code.as_str().map(str::to_owned))
        .unwrap_or_default();
    match diagnostic.field.as_deref() {
        Some(field) => format!("{field}: {code}: {}", diagnostic.message),
        None => format!("{code}: {}", diagnostic.message),
    }
}

pub fn addressing_dimension_is_valid(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 4
        && parts[0] == "InformationRegister"
        && parts[2] == "Dimension"
        && unica_metadata::identifier::is_1c_identifier(parts[1])
        && unica_metadata::identifier::is_1c_identifier(parts[3])
}

/// Projects local typed metadata from one exact descriptor image. The caller
/// owns both the bytes and the support observation; this function performs no
/// workspace resolution and no filesystem reads.
pub fn parse_typed_meta_local_info(
    descriptor_preimage: &[u8],
    target: &MetadataAddress,
    support: MetaSupportStatus,
) -> Result<unica_metadata::validation_subject::MetaLocalInfo, unica_metadata::request::MetaFailure>
{
    parse_typed_meta_local_info_inner(descriptor_preimage, target, support, None)
}
pub fn parse_typed_meta_local_info_for_source(
    descriptor_preimage: &[u8],
    target: &MetadataAddress,
    support: MetaSupportStatus,
    source_kind: unica_source_contract::SourceSetKind,
) -> Result<unica_metadata::validation_subject::MetaLocalInfo, unica_metadata::request::MetaFailure>
{
    parse_typed_meta_local_info_inner(descriptor_preimage, target, support, Some(source_kind))
}
pub fn parse_typed_meta_local_info_inner(
    descriptor_preimage: &[u8],
    target: &MetadataAddress,
    support: MetaSupportStatus,
    source_kind: Option<unica_source_contract::SourceSetKind>,
) -> Result<unica_metadata::validation_subject::MetaLocalInfo, unica_metadata::request::MetaFailure>
{
    let identity = crate::validation_context::inspect_metadata_image_identity(descriptor_preimage)
        .map_err(|error| {
            let diagnostic = MetaDiagnostic::error(
                MetaDiagnosticCode::ProviderUnavailable,
                format!("metadata descriptor identity proof is invalid: {error}"),
            )
            .with_metadata_path(target.clone());
            unica_metadata::request::MetaFailure::from(match error.field() {
                Some(field) => diagnostic.with_field(field),
                None => diagnostic,
            })
        })?;
    let text = std::str::from_utf8(descriptor_preimage)
        .expect("the exact metadata identity proof parsed these bytes as UTF-8");
    let xml = text.trim_start_matches('\u{feff}');
    let doc = Document::parse(xml)
        .expect("the exact metadata identity proof parsed the same XML document");
    let object = doc
        .root_element()
        .children()
        .find(|node| node.is_element())
        .expect("the exact metadata identity proof found one object");
    let kind = MetadataKind::parse(&identity.object_type).map_err(|diagnostic| {
        unica_metadata::request::MetaFailure::from(
            MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, diagnostic.message)
                .with_metadata_path(target.clone()),
        )
    })?;
    let mut expected = target.segments();
    let expected_type = expected.next().unwrap_or_default();
    let expected_name = expected.next().unwrap_or_default();
    if identity.object_type != expected_type || identity.object_name != expected_name {
        return Err(MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            format!(
                "metadata descriptor identity {}.{} does not match target {target}",
                identity.object_type, identity.object_name
            ),
        )
        .with_metadata_path(target.clone())
        .into());
    }
    let property_containers = object
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Properties")
        .collect::<Vec<_>>();
    let [properties] = property_containers.as_slice() else {
        return Err(MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor must contain exactly one direct Properties container",
        )
        .with_metadata_path(target.clone())
        .with_field("properties")
        .into());
    };
    if properties.tag_name().namespace() != Some(super::info_projection::MD_CLASSES_NAMESPACE) {
        return Err(MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor Properties container has the wrong namespace",
        )
        .with_metadata_path(target.clone())
        .with_field("properties")
        .into());
    }
    let child_object_containers = object
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "ChildObjects")
        .collect::<Vec<_>>();
    if child_object_containers.len() > 1
        || child_object_containers.iter().any(|node| {
            node.tag_name().namespace() != Some(super::info_projection::MD_CLASSES_NAMESPACE)
        })
    {
        return Err(MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor contains a duplicate or foreign ChildObjects container",
        )
        .with_metadata_path(target.clone())
        .with_field("collections")
        .into());
    }
    let properties = Some(*properties);
    let child_objects = child_object_containers.first().copied();
    let name_nodes = crate::info_projection::direct_children_with_namespace(
        *properties.as_ref().expect("proved Properties"),
        super::info_projection::MD_CLASSES_NAMESPACE,
        "Name",
    );
    let name = match name_nodes.as_slice() {
        [node]
            if node.attributes().len() == 0 && !node.children().any(|child| child.is_element()) =>
        {
            crate::info_projection::direct_text_content(*node)
        }
        _ => String::new(),
    };
    if name.is_empty() {
        return Err(MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor has no object name",
        )
        .with_metadata_path(target.clone())
        .into());
    }
    let synonym = properties.and_then(|properties| {
        let nodes = crate::info_projection::direct_children_with_namespace(
            properties,
            super::info_projection::MD_CLASSES_NAMESPACE,
            "Synonym",
        );
        let [node] = nodes.as_slice() else {
            return None;
        };
        crate::info_projection::meta_info_property_value_is_valid(
            *node,
            MetaInfoPropertyValueKind::LegacyLocalizedString,
        )
        .then(|| crate::xml_model::meta_info_ml_text(*node))
        .filter(|value| !value.is_empty())
    });

    let mut diagnostics = Vec::new();
    for error in crate::info_projection::meta_info_profile_errors(kind, properties, child_objects) {
        diagnostics.push(
            MetaDiagnostic::error(error.code, error.message)
                .with_metadata_path(target.clone())
                .with_field(error.field),
        );
    }
    let details = crate::info_projection::project_meta_info_details(
        kind,
        properties,
        child_objects,
        target,
        &mut diagnostics,
    );
    let declarations = crate::info_projection::project_meta_info_declarations(
        kind,
        properties,
        target,
        &mut diagnostics,
    );
    let format = unica_source_designer::roots::root_version_literal(xml, doc.root_element());
    let member_read_profile = source_kind.and_then(|source| {
        unica_metadata::MetaMemberReadProfile::for_source(source, kind, format.as_deref())
    });
    let collection_route = |tag, nested_attributes, field| {
        TypedRootCollectionRoute::new(kind, tag, nested_attributes, field)
    };
    Ok(unica_metadata::validation_subject::MetaLocalInfo {
        member_read_profile,
        metadata_path: target.clone(),
        kind,
        details,
        name,
        synonym,
        support,
        properties: crate::info::typed_properties(properties, kind),
        declarations,
        predefined_code_type: predefined_code_type_for_info(properties, kind),
        relations: crate::info::typed_relations(&doc, properties, kind, target, &mut diagnostics),
        collections: MetaCollectionsData {
            attributes: typed_root_collection(
                xml,
                child_objects,
                collection_route("Attribute", false, "collections.attributes"),
                target,
                &mut diagnostics,
            ),
            tabular_sections: typed_root_collection(
                xml,
                child_objects,
                collection_route("TabularSection", true, "collections.tabularSections")
                    .with_member_read_profile(member_read_profile),
                target,
                &mut diagnostics,
            ),
            dimensions: typed_root_collection(
                xml,
                child_objects,
                collection_route("Dimension", false, "collections.dimensions"),
                target,
                &mut diagnostics,
            ),
            resources: typed_root_collection(
                xml,
                child_objects,
                collection_route("Resource", false, "collections.resources"),
                target,
                &mut diagnostics,
            ),
            recalculations: typed_optional_root_collection(
                xml,
                child_objects,
                collection_route("Recalculation", false, "collections.recalculations"),
                target,
                &mut diagnostics,
            ),
            accounting_flags: typed_optional_root_collection(
                xml,
                child_objects,
                collection_route("AccountingFlag", false, "collections.accountingFlags"),
                target,
                &mut diagnostics,
            ),
            ext_dimension_accounting_flags: typed_optional_root_collection(
                xml,
                child_objects,
                collection_route(
                    "ExtDimensionAccountingFlag",
                    false,
                    "collections.extDimensionAccountingFlags",
                ),
                target,
                &mut diagnostics,
            ),
            addressing_attributes: typed_optional_root_collection(
                xml,
                child_objects,
                collection_route(
                    "AddressingAttribute",
                    false,
                    "collections.addressingAttributes",
                ),
                target,
                &mut diagnostics,
            ),
            enum_values: typed_root_collection(
                xml,
                child_objects,
                collection_route("EnumValue", false, "collections.enumValues"),
                target,
                &mut diagnostics,
            ),
            columns: typed_root_collection(
                xml,
                child_objects,
                collection_route("Column", false, "collections.columns"),
                target,
                &mut diagnostics,
            ),
            forms: typed_root_collection(
                xml,
                child_objects,
                collection_route("Form", false, "collections.forms"),
                target,
                &mut diagnostics,
            ),
            templates: typed_root_collection(
                xml,
                child_objects,
                collection_route("Template", false, "collections.templates"),
                target,
                &mut diagnostics,
            ),
            commands: typed_root_collection(
                xml,
                child_objects,
                collection_route("Command", false, "collections.commands"),
                target,
                &mut diagnostics,
            ),
        },
        diagnostics,
    })
}
pub fn predefined_code_type_for_info(
    properties: Option<roxmltree::Node<'_, '_>>,
    kind: MetadataKind,
) -> Option<String> {
    match kind {
        MetadataKind::Catalog | MetadataKind::ChartOfCalculationTypes => {
            match properties.and_then(|properties| {
                crate::info_projection::observe_meta_info_property(properties, kind, "CodeType")
                    .ok()
                    .flatten()
            }) {
                Some(unica_metadata::MetaInfoPropertyValue::String(value)) => {
                    Some(value.trim().to_string())
                }
                _ => None,
            }
        }

        MetadataKind::ChartOfAccounts | MetadataKind::ChartOfCharacteristicTypes => {
            Some("String".to_string())
        }
        _ => None,
    }
}
#[derive(Debug, Clone, Copy)]
pub struct TypedRootCollectionRoute<'a> {
    pub member_read_profile: Option<unica_metadata::MetaMemberReadProfile>,
    pub owner_kind: MetadataKind,
    pub tag: &'a str,
    pub nested_attributes: bool,
    pub field: &'a str,
}
pub fn typed_root_collection(
    xml: &str,
    parent: Option<roxmltree::Node<'_, '_>>,
    route: TypedRootCollectionRoute<'_>,
    target: &MetadataAddress,
    diagnostics: &mut Vec<MetaDiagnostic>,
) -> Vec<MetaElementData> {
    if !crate::info_projection::meta_info_collection_is_applicable(route.owner_kind, route.tag) {
        return Vec::new();
    }
    crate::info::typed_elements_profiled(
        xml,
        parent,
        crate::info::ElementReadRoute {
            tag: route.tag,
            nested_attributes: route.nested_attributes,
            field: route.field,
            profile: route.member_read_profile,
        },
        target,
        diagnostics,
    )
}
pub fn typed_optional_root_collection(
    xml: &str,
    parent: Option<roxmltree::Node<'_, '_>>,
    route: TypedRootCollectionRoute<'_>,
    target: &MetadataAddress,
    diagnostics: &mut Vec<MetaDiagnostic>,
) -> Option<Option<Vec<MetaElementData>>> {
    if !crate::info_projection::meta_info_collection_is_applicable(route.owner_kind, route.tag) {
        return None;
    }
    let Some(parent) = parent else {
        return Some(None);
    };
    let expected = crate::info_projection::direct_children_with_namespace(
        parent,
        super::info_projection::MD_CLASSES_NAMESPACE,
        route.tag,
    )
    .len();
    let errors_before = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == MetaDiagnosticSeverity::Error)
        .count();
    let values = crate::info::typed_elements_with_diagnostics(
        xml,
        Some(parent),
        route.tag,
        route.nested_attributes,
        route.field,
        target,
        diagnostics,
    );
    let errors_after = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == MetaDiagnosticSeverity::Error)
        .count();
    if values.len() != expected && errors_after == errors_before {
        diagnostics.push(
            MetaDiagnostic::error(
                MetaDiagnosticCode::ValidationFailed,
                "metadata child collection contains an element without a proved identity",
            )
            .with_metadata_path(target.clone())
            .with_field(route.field),
        );
    }
    if errors_after != errors_before || values.len() != expected {
        Some(None)
    } else {
        Some(Some(values))
    }
}

impl<'a> TypedRootCollectionRoute<'a> {
    pub const fn new(
        owner_kind: MetadataKind,
        tag: &'a str,
        nested_attributes: bool,
        field: &'a str,
    ) -> Self {
        Self {
            member_read_profile: None,
            owner_kind,
            tag,
            nested_attributes,
            field,
        }
    }
    pub fn with_member_read_profile(
        mut self,
        profile: Option<unica_metadata::MetaMemberReadProfile>,
    ) -> Self {
        self.member_read_profile = profile;
        self
    }
}

#[cfg(test)]
mod predefined_code_type_tests {
    use roxmltree::Document;
    use unica_metadata::MetadataKind;

    #[test]
    fn info_uses_the_complete_direct_code_type_text() {
        let document = Document::parse(
            r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses"><CodeType>Num<!--keep-->ber</CodeType></Properties>"#,
        )
        .unwrap();
        assert_eq!(
            crate::info::predefined_code_type_for_info(
                Some(document.root_element()),
                MetadataKind::Catalog
            )
            .as_deref(),
            Some("Number")
        );

        let suffixed = Document::parse(
            r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses"><CodeType>Number<!--keep-->Suffix</CodeType></Properties>"#,
        )
        .unwrap();
        assert_eq!(
            crate::info::predefined_code_type_for_info(
                Some(suffixed.root_element()),
                MetadataKind::Catalog
            )
            .as_deref(),
            None
        );
        assert!(crate::info_projection::meta_info_profile_errors(
            MetadataKind::Catalog,
            Some(suffixed.root_element()),
            None
        )
        .iter()
        .any(|error| error.field == "properties.CodeType"));
    }
}
