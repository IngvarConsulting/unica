#[cfg(test)]
use crate::domain::metadata::META_INFO_PROPERTY_NAMES;
use crate::domain::metadata::{
    metadata_identifier_is_valid, DateFractions, MetaCalculationSchedule, MetaCharacteristic,
    MetaCharacteristicTypes, MetaCharacteristicValues, MetaDiagnostic, MetaDiagnosticCode,
    MetaExpandedName, MetaHttpMethod, MetaHttpUrlTemplate, MetaInfoDeclarations, MetaInfoDetails,
    MetaInfoPropertyValueKind, MetaLocalizedText, MetaObservedProperty, MetaObservedPropertyValue,
    MetaScheduledMethod, MetaStandardAttribute, MetaStandardTabularSection, MetaTransferDirection,
    MetaWebServiceOperation, MetaWebServiceParameter, MetaXdtoPackage, MetadataKind, NumberSign,
    ObservedMetadataType, ObservedMetadataTypeVariant, StringLengthMode,
    META_INFO_PROPERTY_PROFILE,
};
use crate::domain::source_target::{
    xml_ncname_is_valid, MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20,
};
#[cfg(test)]
use roxmltree::Document;

const DATA_CORE_NAMESPACE: &str = "http://v8.1c.ru/8.1/data/core";
const CURRENT_CONFIG_NAMESPACE: &str = "http://v8.1c.ru/8.1/data/enterprise/current-config";
pub(super) const MD_CLASSES_NAMESPACE: &str = "http://v8.1c.ru/8.3/MDClasses";
pub(super) const READABLE_NAMESPACE: &str = "http://v8.1c.ru/8.3/xcf/readable";
const XML_SCHEMA_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema";
const XML_SCHEMA_INSTANCE_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema-instance";

fn invalid_observed_type(message: impl Into<String>) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, message).with_field("type")
}

fn unsupported_observed_type(message: impl Into<String>) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, message).with_field("type")
}

pub(super) fn direct_child_with_namespace<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    namespace: &str,
    local_name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children().find(|child| {
        child.is_element()
            && child.tag_name().namespace() == Some(namespace)
            && child.tag_name().name() == local_name
    })
}

pub(super) fn direct_children_with_namespace<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    namespace: &str,
    local_name: &str,
) -> Vec<roxmltree::Node<'a, 'input>> {
    node.children()
        .filter(|child| {
            child.is_element()
                && child.tag_name().namespace() == Some(namespace)
                && child.tag_name().name() == local_name
        })
        .collect()
}

pub(super) fn direct_md_child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    direct_child_with_namespace(node, MD_CLASSES_NAMESPACE, local_name)
}

pub(super) fn direct_md_child_text(
    node: roxmltree::Node<'_, '_>,
    local_name: &str,
) -> Option<String> {
    direct_md_child(node, local_name).map(direct_text_content)
}

fn unique_direct_md_child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    let children = direct_children_with_namespace(node, MD_CLASSES_NAMESPACE, local_name);
    let [child] = children.as_slice() else {
        return None;
    };
    Some(*child)
}

fn unique_direct_md_text(node: roxmltree::Node<'_, '_>, local_name: &str) -> Option<String> {
    let child = unique_direct_md_child(node, local_name)?;
    strict_text_leaf_is_valid(child).then(|| direct_text_content(child))
}

/// Parse a writer-side metadata type fragment into the read-side algebra.
///
/// Existing writer callers pass one of three historical fragment shapes:
/// `Properties`, its direct `Type` container, or the direct `v8:*` children of
/// that container. The wrapper supplies only the namespaces inherited from the
/// descriptor root; the node parser below still validates exact expanded names
/// and container shape.
#[cfg(test)]
pub(super) fn parse_observed_metadata_type(
    properties_text: &str,
) -> Result<ObservedMetadataType, MetaDiagnostic> {
    const WRAPPER_START: &str = r#"<Root xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config">"#;
    let wrapped = format!("{WRAPPER_START}{properties_text}</Root>");
    let document = Document::parse(&wrapped)
        .map_err(|_| invalid_observed_type("metadata element properties are not valid XML"))?;
    let root = document.root_element();
    let elements = root
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    match elements.as_slice() {
        [properties]
            if properties.tag_name().namespace() == Some(MD_CLASSES_NAMESPACE)
                && properties.tag_name().name() == "Properties" =>
        {
            parse_observed_metadata_type_node(*properties)
        }
        [type_container]
            if type_container.tag_name().namespace() == Some(MD_CLASSES_NAMESPACE)
                && type_container.tag_name().name() == "Type" =>
        {
            parse_observed_metadata_type_container_node(*type_container)
        }
        [] => Err(invalid_observed_type(
            "metadata element properties are missing",
        )),
        _ => parse_observed_metadata_type_container_node(root),
    }
}

pub(super) fn parse_observed_metadata_type_node(
    properties: roxmltree::Node<'_, '_>,
) -> Result<ObservedMetadataType, MetaDiagnostic> {
    let type_containers = direct_children_with_namespace(properties, MD_CLASSES_NAMESPACE, "Type");
    let [type_container] = type_containers.as_slice() else {
        return Err(invalid_observed_type(
            "metadata properties must contain exactly one direct Type",
        ));
    };
    parse_observed_metadata_type_container_node(*type_container)
}

fn parse_observed_metadata_type_container_node(
    type_container: roxmltree::Node<'_, '_>,
) -> Result<ObservedMetadataType, MetaDiagnostic> {
    if type_container.attributes().len() != 0 || !node_has_only_whitespace_text(type_container) {
        return Err(invalid_observed_type(
            "existing metadata Type container is malformed",
        ));
    }
    parse_observed_metadata_type_contents_node(type_container, false)
}

pub(super) fn parse_observed_predefined_type_node(
    type_container: roxmltree::Node<'_, '_>,
) -> Result<ObservedMetadataType, MetaDiagnostic> {
    if !node_has_only_whitespace_text(type_container) {
        return Err(invalid_observed_type(
            "existing predefined Type container is malformed",
        ));
    }
    parse_observed_metadata_type_contents_node(type_container, true)
}

fn parse_observed_metadata_type_contents_node(
    type_container: roxmltree::Node<'_, '_>,
    allow_predefined_extensions: bool,
) -> Result<ObservedMetadataType, MetaDiagnostic> {
    let allowed_type_children = [
        "Type",
        "TypeSet",
        "StringQualifiers",
        "NumberQualifiers",
        "DateQualifiers",
        "BinaryDataQualifiers",
    ];
    if !allow_predefined_extensions
        && type_container
            .children()
            .filter(roxmltree::Node::is_element)
            .any(|child| {
                child.tag_name().namespace() != Some(DATA_CORE_NAMESPACE)
                    || !allowed_type_children.contains(&child.tag_name().name())
            })
    {
        return Err(unsupported_observed_type(
            "existing metadata Type contains an unsupported child",
        ));
    }

    let qualifier_text = |container: &str, name: &str| -> Option<String> {
        direct_child_with_namespace(type_container, DATA_CORE_NAMESPACE, container)
            .and_then(|node| direct_child_with_namespace(node, DATA_CORE_NAMESPACE, name))
            .map(direct_text_content)
    };
    let qualifier_u32 = |container: &str, name: &str| -> Result<u32, MetaDiagnostic> {
        qualifier_text(container, name).map_or(Ok(0), |value| {
            value.parse().map_err(|_| {
                invalid_observed_type(format!(
                    "existing metadata type has malformed {container}.{name}"
                ))
            })
        })
    };

    let mut seen_qualifier_containers = std::collections::HashSet::new();
    for container in type_container.children().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(DATA_CORE_NAMESPACE)
            && matches!(
                node.tag_name().name(),
                "StringQualifiers" | "NumberQualifiers" | "DateQualifiers" | "BinaryDataQualifiers"
            )
    }) {
        let allowed = match container.tag_name().name() {
            "StringQualifiers" | "BinaryDataQualifiers" => &["Length", "AllowedLength"][..],
            "NumberQualifiers" => &["Digits", "FractionDigits", "AllowedSign"][..],
            "DateQualifiers" => &["DateFractions"][..],
            _ => unreachable!(),
        };
        let mut seen = std::collections::HashSet::new();
        if container.attributes().len() != 0
            || !node_has_only_whitespace_text(container)
            || !seen_qualifier_containers.insert(container.tag_name().name())
            || container
                .children()
                .filter(roxmltree::Node::is_element)
                .any(|child| {
                    child.tag_name().namespace() != Some(DATA_CORE_NAMESPACE)
                        || !allowed.contains(&child.tag_name().name())
                        || !seen.insert(child.tag_name().name())
                        || child.attributes().len() != 0
                        || child.children().any(|nested| nested.is_element())
                })
        {
            return Err(invalid_observed_type(
                "existing metadata type qualifier structure is malformed",
            ));
        }
        if seen.len() != allowed.len() {
            return Err(invalid_observed_type(
                "existing metadata type qualifier is incomplete",
            ));
        }
    }

    let mut variants = Vec::new();
    let mut expected_qualifiers = std::collections::HashSet::new();
    for node in type_container.children().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(DATA_CORE_NAMESPACE)
            && matches!(node.tag_name().name(), "Type" | "TypeSet")
    }) {
        if node.attributes().len() != 0 || node.children().any(|child| child.is_element()) {
            return Err(invalid_observed_type(
                "existing metadata type QName must contain text only",
            ));
        }
        let value = direct_text_content(node);
        let (namespace, local_name) = expand_observed_type_qname(node, value.trim())?;
        match (namespace.as_str(), local_name.as_str()) {
            (XML_SCHEMA_NAMESPACE, "string") => {
                expected_qualifiers.insert("StringQualifiers");
            }
            (XML_SCHEMA_NAMESPACE, "decimal") => {
                expected_qualifiers.insert("NumberQualifiers");
            }
            (XML_SCHEMA_NAMESPACE, "dateTime") => {
                expected_qualifiers.insert("DateQualifiers");
            }
            (XML_SCHEMA_NAMESPACE, "binary") => {
                expected_qualifiers.insert("BinaryDataQualifiers");
            }
            _ => {}
        }
        let variant = if node.tag_name().name() == "TypeSet" {
            if namespace != CURRENT_CONFIG_NAMESPACE {
                return Err(invalid_observed_type(
                    "existing defined type uses an unsupported namespace",
                ));
            }
            let metadata_path =
                MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &local_name).map_err(
                    |_| invalid_observed_type("existing defined type is not a metadata address"),
                )?;
            if metadata_path.segments().next() != Some("DefinedType")
                || metadata_path.segments().count() != 2
            {
                return Err(invalid_observed_type(
                    "existing type set does not target a defined type",
                ));
            }
            ObservedMetadataTypeVariant::DefinedType { metadata_path }
        } else {
            match (namespace.as_str(), local_name.as_str()) {
                (XML_SCHEMA_NAMESPACE, "string") => ObservedMetadataTypeVariant::String {
                    length: qualifier_u32("StringQualifiers", "Length")?,
                    allowed_length: match qualifier_text("StringQualifiers", "AllowedLength")
                        .as_deref()
                    {
                        Some("Fixed") => StringLengthMode::Fixed,
                        Some("Variable") | None => StringLengthMode::Variable,
                        Some(_) => {
                            return Err(invalid_observed_type(
                                "existing string length mode is unsupported",
                            ))
                        }
                    },
                },
                (XML_SCHEMA_NAMESPACE, "decimal") => ObservedMetadataTypeVariant::Number {
                    digits: qualifier_u32("NumberQualifiers", "Digits")?,
                    fraction: qualifier_u32("NumberQualifiers", "FractionDigits")?,
                    sign: match qualifier_text("NumberQualifiers", "AllowedSign").as_deref() {
                        Some("Nonnegative") => NumberSign::NonNegative,
                        Some("Any") | None => NumberSign::Any,
                        Some(_) => {
                            return Err(invalid_observed_type(
                                "existing number sign mode is unsupported",
                            ))
                        }
                    },
                },
                (XML_SCHEMA_NAMESPACE, "boolean") => ObservedMetadataTypeVariant::Boolean,
                (XML_SCHEMA_NAMESPACE, "dateTime") => ObservedMetadataTypeVariant::Date {
                    fractions: match qualifier_text("DateQualifiers", "DateFractions").as_deref() {
                        Some("Date") => DateFractions::Date,
                        Some("Time") => DateFractions::Time,
                        Some("DateTime") | None => DateFractions::DateTime,
                        Some(_) => {
                            return Err(invalid_observed_type(
                                "existing date fractions mode is unsupported",
                            ))
                        }
                    },
                },
                (XML_SCHEMA_NAMESPACE, "binary") => ObservedMetadataTypeVariant::BinaryData {
                    length: qualifier_u32("BinaryDataQualifiers", "Length")?,
                    allowed_length: match qualifier_text("BinaryDataQualifiers", "AllowedLength")
                        .as_deref()
                    {
                        Some("Fixed") => StringLengthMode::Fixed,
                        Some("Variable") | None => StringLengthMode::Variable,
                        Some(_) => {
                            return Err(invalid_observed_type(
                                "existing binary length mode is unsupported",
                            ))
                        }
                    },
                },
                (DATA_CORE_NAMESPACE, "ValueStorage") => ObservedMetadataTypeVariant::ValueStorage,
                (DATA_CORE_NAMESPACE, "UUID") => ObservedMetadataTypeVariant::Uuid,
                (CURRENT_CONFIG_NAMESPACE, raw) => {
                    let Some((generated, name)) = raw.split_once('.') else {
                        return Err(invalid_observed_type(
                            "existing reference type is malformed",
                        ));
                    };
                    let kind = generated.strip_suffix("Ref").ok_or_else(|| {
                        invalid_observed_type("existing generated type is not a reference")
                    })?;
                    ObservedMetadataTypeVariant::Reference {
                        metadata_path: MetadataAddress::parse(
                            PLATFORM_XML_8_3_27_FORMAT_2_20,
                            &format!("{kind}.{name}"),
                        )
                        .map_err(|_| {
                            invalid_observed_type(
                                "existing reference type is not a metadata address",
                            )
                        })?,
                    }
                }
                _ => {
                    return Err(invalid_observed_type(
                        "existing metadata type is unsupported by typed read",
                    ))
                }
            }
        };
        variants.push(variant);
    }
    // Platform 8.3.27 emits the default string type without an explicit
    // qualifier container for newly-created attributes. When a qualifier is
    // present it must be complete and belong to a matching primitive; an
    // omitted container means that primitive's canonical default values.
    if !seen_qualifier_containers.is_subset(&expected_qualifiers) {
        return Err(invalid_observed_type(
            "existing metadata type qualifiers do not match its primitive variants",
        ));
    }
    ObservedMetadataType::new(variants)
}

pub(super) fn observed_type_is_strict_but_unmodelled(properties: roxmltree::Node<'_, '_>) -> bool {
    let containers = direct_children_with_namespace(properties, MD_CLASSES_NAMESPACE, "Type");
    let [container] = containers.as_slice() else {
        return false;
    };
    if container.attributes().len() != 0 || !node_has_only_whitespace_text(*container) {
        return false;
    }
    let allowed_children = [
        "Type",
        "TypeSet",
        "StringQualifiers",
        "NumberQualifiers",
        "DateQualifiers",
        "BinaryDataQualifiers",
    ];
    if container
        .children()
        .filter(roxmltree::Node::is_element)
        .any(|child| {
            child.tag_name().namespace() != Some(DATA_CORE_NAMESPACE)
                || !allowed_children.contains(&child.tag_name().name())
        })
    {
        return false;
    }

    let mut seen_qualifiers = std::collections::HashSet::new();
    for qualifier in container.children().filter(|node| {
        node.is_element()
            && matches!(
                node.tag_name().name(),
                "StringQualifiers" | "NumberQualifiers" | "DateQualifiers" | "BinaryDataQualifiers"
            )
    }) {
        let allowed = match qualifier.tag_name().name() {
            "StringQualifiers" | "BinaryDataQualifiers" => &["Length", "AllowedLength"][..],
            "NumberQualifiers" => &["Digits", "FractionDigits", "AllowedSign"][..],
            "DateQualifiers" => &["DateFractions"][..],
            _ => return false,
        };
        let mut seen_children = std::collections::HashSet::new();
        if qualifier.attributes().len() != 0
            || !node_has_only_whitespace_text(qualifier)
            || !seen_qualifiers.insert(qualifier.tag_name().name())
            || qualifier
                .children()
                .filter(roxmltree::Node::is_element)
                .any(|leaf| {
                    leaf.tag_name().namespace() != Some(DATA_CORE_NAMESPACE)
                        || !allowed.contains(&leaf.tag_name().name())
                        || !seen_children.insert(leaf.tag_name().name())
                        || leaf.attributes().len() != 0
                        || leaf.children().any(|nested| nested.is_element())
                })
        {
            return false;
        }
        if seen_children.len() != allowed.len() {
            return false;
        }
        if qualifier
            .children()
            .filter(roxmltree::Node::is_element)
            .any(|leaf| {
                let value = direct_text_content(leaf);
                match leaf.tag_name().name() {
                    "Length" | "Digits" | "FractionDigits" => value.parse::<u32>().is_err(),
                    "AllowedLength" => !matches!(value.as_str(), "Fixed" | "Variable"),
                    "AllowedSign" => !matches!(value.as_str(), "Nonnegative" | "Any"),
                    "DateFractions" => !matches!(value.as_str(), "Date" | "Time" | "DateTime"),
                    _ => true,
                }
            })
        {
            return false;
        }
        let value = |name: &str| {
            direct_child_with_namespace(qualifier, DATA_CORE_NAMESPACE, name)
                .map(direct_text_content)
                .unwrap_or_default()
        };
        let semantically_invalid = match qualifier.tag_name().name() {
            "StringQualifiers" => {
                let length = value("Length").parse::<u32>().unwrap_or(u32::MAX);
                let allowed = value("AllowedLength");
                length > 1024 || (allowed == "Fixed" && length == 0)
            }
            "NumberQualifiers" => {
                let digits = value("Digits").parse::<u32>().unwrap_or(u32::MAX);
                let fraction = value("FractionDigits").parse::<u32>().unwrap_or(u32::MAX);
                digits > 38 || fraction > digits
            }
            "BinaryDataQualifiers" => value("AllowedLength") == "Fixed" && value("Length") == "0",
            "DateQualifiers" => false,
            _ => true,
        };
        if semantically_invalid {
            return false;
        }
    }

    let mut saw_variant = false;
    let mut variant_count = 0usize;
    let mut saw_value_storage = false;
    let mut saw_unknown = false;
    let mut seen_variants = std::collections::HashSet::new();
    let mut expected_qualifiers = std::collections::HashSet::new();
    for node in container.children().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(DATA_CORE_NAMESPACE)
            && matches!(node.tag_name().name(), "Type" | "TypeSet")
    }) {
        saw_variant = true;
        variant_count += 1;
        if node.attributes().len() != 0 || node.children().any(|child| child.is_element()) {
            return false;
        }
        let value = direct_text_content(node);
        let Ok((namespace, local)) = expand_observed_type_qname(node, value.trim()) else {
            return false;
        };
        if !seen_variants.insert((node.tag_name().name(), namespace.clone(), local.clone())) {
            return false;
        }
        if node.tag_name().name() == "TypeSet" {
            if namespace != CURRENT_CONFIG_NAMESPACE {
                return false;
            }
            let Ok(address) = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &local)
            else {
                return false;
            };
            if address.segments().next() != Some("DefinedType") || address.segments().count() != 2 {
                return false;
            }
            continue;
        }
        match (namespace.as_str(), local.as_str()) {
            (XML_SCHEMA_NAMESPACE, "string") => {
                expected_qualifiers.insert("StringQualifiers");
            }
            (XML_SCHEMA_NAMESPACE, "decimal") => {
                expected_qualifiers.insert("NumberQualifiers");
            }
            (XML_SCHEMA_NAMESPACE, "dateTime") => {
                expected_qualifiers.insert("DateQualifiers");
            }
            (XML_SCHEMA_NAMESPACE, "binary") => {
                expected_qualifiers.insert("BinaryDataQualifiers");
            }
            _ => {}
        }
        let known = match (namespace.as_str(), local.as_str()) {
            (XML_SCHEMA_NAMESPACE, "string" | "decimal" | "boolean" | "dateTime" | "binary") => {
                true
            }
            (DATA_CORE_NAMESPACE, "ValueStorage") => {
                saw_value_storage = true;
                true
            }
            (DATA_CORE_NAMESPACE, "UUID") => true,
            (CURRENT_CONFIG_NAMESPACE, raw) => {
                raw.split_once('.').is_some_and(|(generated, name)| {
                    generated.strip_suffix("Ref").is_some_and(|kind| {
                        MetadataKind::parse(kind).is_ok() && metadata_identifier_is_valid(name)
                    })
                })
            }
            (XML_SCHEMA_NAMESPACE | DATA_CORE_NAMESPACE, _) => false,
            _ => return false,
        };
        saw_unknown |= !known;
    }
    saw_variant
        && saw_unknown
        && !(saw_value_storage && variant_count != 1)
        && seen_qualifiers.is_subset(&expected_qualifiers)
}

fn expand_observed_type_qname(
    node: roxmltree::Node<'_, '_>,
    raw: &str,
) -> Result<(String, String), MetaDiagnostic> {
    let (prefix, local_name) = raw.split_once(':').ok_or_else(|| {
        invalid_observed_type("existing metadata type is not a namespace-qualified name")
    })?;
    if !xml_ncname_is_valid(prefix) || !xml_ncname_is_valid(local_name) {
        return Err(invalid_observed_type(
            "existing metadata type qualified name is malformed",
        ));
    }
    let namespace = node.lookup_namespace_uri(Some(prefix)).ok_or_else(|| {
        invalid_observed_type("existing metadata type namespace prefix is not declared")
    })?;
    Ok((namespace.to_string(), local_name.to_string()))
}

pub(super) fn project_meta_info_details(
    kind: MetadataKind,
    properties: Option<roxmltree::Node<'_, '_>>,
    child_objects: Option<roxmltree::Node<'_, '_>>,
    target: &MetadataAddress,
    diagnostics: &mut Vec<MetaDiagnostic>,
) -> MetaInfoDetails {
    let mut observed_type = || {
        let properties = properties?;
        let type_node = direct_md_child(properties, "Type")?;
        if node_is_empty_placeholder(type_node) {
            return None;
        }
        match parse_observed_metadata_type_node(properties) {
            Ok(value) => Some(value),
            Err(diagnostic) => {
                let diagnostic = if observed_type_is_strict_but_unmodelled(properties) {
                    MetaDiagnostic::warning(
                        MetaDiagnosticCode::ValidationFailed,
                        "metadata type is syntactically valid but not modelled by this format profile",
                    )
                } else {
                    MetaDiagnostic::error(diagnostic.code, diagnostic.message)
                };
                diagnostics.push(
                    diagnostic
                        .with_metadata_path(target.clone())
                        .with_field("details.type"),
                );
                None
            }
        }
    };

    let diagnostic = |error: ProjectionError, diagnostics: &mut Vec<MetaDiagnostic>| {
        diagnostics.push(
            MetaDiagnostic::error(error.code, error.message)
                .with_metadata_path(target.clone())
                .with_field(error.field),
        );
    };

    match kind {
        MetadataKind::Constant => MetaInfoDetails::Constant {
            r#type: observed_type(),
        },
        MetadataKind::DefinedType => MetaInfoDetails::DefinedType {
            r#type: observed_type(),
        },
        MetadataKind::ChartOfCharacteristicTypes => MetaInfoDetails::ChartOfCharacteristicTypes {
            r#type: observed_type(),
        },
        MetadataKind::ChartOfCalculationTypes => {
            let base_calculation_types = properties
                .and_then(|node| unique_direct_md_child(node, "BaseCalculationTypes"))
                .map(|node| {
                    parse_metadata_object_refs(
                        node,
                        "details.baseCalculationTypes",
                        "ChartOfCalculationTypes",
                    )
                });
            let base_calculation_types = match base_calculation_types {
                Some(Ok(value)) => Some(value),
                Some(Err(error)) => {
                    diagnostic(error, diagnostics);
                    None
                }
                None => None,
            };
            MetaInfoDetails::ChartOfCalculationTypes {
                base_calculation_types,
            }
        }
        MetadataKind::DocumentJournal => {
            let registered_documents = properties
                .and_then(|node| unique_direct_md_child(node, "RegisteredDocuments"))
                .map(|node| {
                    parse_metadata_object_refs(node, "details.registeredDocuments", "Document")
                });
            let registered_documents = match registered_documents {
                Some(Ok(value)) => Some(value),
                Some(Err(error)) => {
                    diagnostic(error, diagnostics);
                    None
                }
                None => None,
            };
            MetaInfoDetails::DocumentJournal {
                registered_documents,
            }
        }
        MetadataKind::ScheduledJob => {
            let method = properties
                .and_then(|node| unique_direct_md_text(node, "MethodName"))
                .filter(|value| !value.trim().is_empty())
                .and_then(|value| {
                    let parts = value.split('.').collect::<Vec<_>>();
                    let (module, method) = match parts.as_slice() {
                        ["CommonModule", module, method] => (*module, *method),
                        _ => {
                            diagnostic(
                                ProjectionError::malformed(
                                    "details.method",
                                    "scheduled job method must name a common module and procedure",
                                ),
                                diagnostics,
                            );
                            return None;
                        }
                    };
                    if !metadata_identifier_is_valid(module)
                        || !metadata_identifier_is_valid(method)
                    {
                        diagnostic(
                            ProjectionError::malformed(
                                "details.method",
                                "scheduled job method contains an invalid identifier",
                            ),
                            diagnostics,
                        );
                        return None;
                    }
                    let metadata_path = MetadataAddress::parse(
                        PLATFORM_XML_8_3_27_FORMAT_2_20,
                        &format!("CommonModule.{module}"),
                    )
                    .ok()?;
                    Some(MetaScheduledMethod {
                        metadata_path,
                        method: method.to_string(),
                    })
                });
            MetaInfoDetails::ScheduledJob { method }
        }
        MetadataKind::CalculationRegister => {
            let schedule = properties.and_then(|properties| {
                let values = [
                    unique_direct_md_text(properties, "Schedule"),
                    unique_direct_md_text(properties, "ScheduleValue"),
                    unique_direct_md_text(properties, "ScheduleDate"),
                ];
                if values
                    .iter()
                    .all(|value| value.as_deref().is_none_or(str::is_empty))
                {
                    return None;
                }
                let [Some(register), Some(value_field), Some(date_field)] = values else {
                    diagnostic(
                        ProjectionError::malformed(
                            "details.schedule",
                            "calculation register schedule must contain all three logical targets",
                        ),
                        diagnostics,
                    );
                    return None;
                };
                let Ok(register_address) = MetadataAddress::parse(
                    PLATFORM_XML_8_3_27_FORMAT_2_20,
                    &register,
                ) else {
                    diagnostic(
                        ProjectionError::malformed(
                            "details.schedule.register",
                            "calculation register schedule target is not a metadata address",
                        ),
                        diagnostics,
                    );
                    return None;
                };
                if register_address.segments().next() != Some("InformationRegister")
                    || !schedule_field_matches(&register, &value_field, "Resource")
                    || !schedule_field_matches(&register, &date_field, "Dimension")
                {
                    diagnostic(
                        ProjectionError::malformed(
                            "details.schedule",
                            "calculation register schedule fields do not belong to the schedule register",
                        ),
                        diagnostics,
                    );
                    return None;
                }
                Some(MetaCalculationSchedule {
                    register: register_address,
                    value_field,
                    date_field,
                })
            });
            MetaInfoDetails::CalculationRegister { schedule }
        }
        MetadataKind::HTTPService => {
            let url_templates = child_objects.map(|child_objects| {
                ensure_only_direct_md_children(
                    child_objects,
                    &["URLTemplate"],
                    "details.urlTemplates",
                )?;
                direct_children_with_namespace(child_objects, MD_CLASSES_NAMESPACE, "URLTemplate")
                    .into_iter()
                    .enumerate()
                    .map(|(index, node)| parse_http_template(node, index))
                    .collect::<Result<Vec<_>, _>>()
            });
            let url_templates = match url_templates {
                Some(Ok(value)) => Some(value),
                Some(Err(error)) => {
                    diagnostic(error, diagnostics);
                    None
                }
                None => None,
            };
            MetaInfoDetails::HTTPService { url_templates }
        }
        MetadataKind::WebService => {
            let xdto_packages = properties
                .and_then(|node| unique_direct_md_child(node, "XDTOPackages"))
                .map(parse_xdto_packages);
            let xdto_packages = match xdto_packages {
                Some(Ok(value)) => Some(value),
                Some(Err(error)) => {
                    diagnostic(error, diagnostics);
                    None
                }
                None => None,
            };
            let operations = child_objects.map(|child_objects| {
                ensure_only_direct_md_children(
                    child_objects,
                    &["Operation"],
                    "details.operations",
                )?;
                direct_children_with_namespace(child_objects, MD_CLASSES_NAMESPACE, "Operation")
                    .into_iter()
                    .enumerate()
                    .map(|(index, node)| parse_web_operation(node, index))
                    .collect::<Result<Vec<_>, _>>()
            });
            let operations = match operations {
                Some(Ok(value)) => Some(value),
                Some(Err(error)) => {
                    diagnostic(error, diagnostics);
                    None
                }
                None => None,
            };
            MetaInfoDetails::WebService {
                xdto_packages,
                operations,
            }
        }
        _ => MetaInfoDetails::empty(kind),
    }
}

pub(super) fn project_meta_info_declarations(
    kind: MetadataKind,
    properties: Option<roxmltree::Node<'_, '_>>,
    target: &MetadataAddress,
    diagnostics: &mut Vec<MetaDiagnostic>,
) -> MetaInfoDeclarations {
    let Some(properties) = properties else {
        return MetaInfoDeclarations::default();
    };

    let profile = MetaInfoKindProfile::new(kind);
    let standard_attributes = if profile.property_route("StandardAttributes")
        == Some(MetaInfoPropertyRoute::Declarations)
    {
        Some(project_declaration(
            properties,
            "StandardAttributes",
            "standardAttributes",
            parse_standard_attributes,
            target,
            diagnostics,
        ))
    } else {
        None
    };
    let characteristics =
        if profile.property_route("Characteristics") == Some(MetaInfoPropertyRoute::Declarations) {
            Some(project_declaration(
                properties,
                "Characteristics",
                "characteristics",
                parse_characteristics,
                target,
                diagnostics,
            ))
        } else {
            None
        };
    let standard_tabular_sections = if profile.property_route("StandardTabularSections")
        == Some(MetaInfoPropertyRoute::Declarations)
    {
        Some(project_declaration(
            properties,
            "StandardTabularSections",
            "standardTabularSections",
            parse_standard_tabular_sections,
            target,
            diagnostics,
        ))
    } else {
        None
    };
    MetaInfoDeclarations {
        standard_attributes,
        characteristics,
        standard_tabular_sections,
    }
}

fn project_declaration<T>(
    properties: roxmltree::Node<'_, '_>,
    name: &str,
    public_field: &str,
    parser: fn(roxmltree::Node<'_, '_>) -> Result<T, ProjectionError>,
    target: &MetadataAddress,
    diagnostics: &mut Vec<MetaDiagnostic>,
) -> Option<T> {
    let node = unique_direct_md_child(properties, name)?;
    match parser(node) {
        Ok(value) => Some(value),
        Err(error) => {
            diagnostics.push(
                MetaDiagnostic::error(error.code, error.message)
                    .with_metadata_path(target.clone())
                    .with_field(if error.field.is_empty() {
                        public_field.to_string()
                    } else {
                        error.field
                    }),
            );
            None
        }
    }
}

fn schedule_field_matches(register: &str, field: &str, expected_kind: &str) -> bool {
    let Some(rest) = field
        .strip_prefix(register)
        .and_then(|value| value.strip_prefix('.'))
    else {
        return false;
    };
    let Some((kind, name)) = rest.split_once('.') else {
        return false;
    };
    kind == expected_kind && metadata_identifier_is_valid(name) && !name.contains('.')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProjectionError {
    pub(super) field: String,
    pub(super) message: String,
    pub(super) code: MetaDiagnosticCode,
}

impl ProjectionError {
    fn malformed(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
            code: MetaDiagnosticCode::ValidationFailed,
        }
    }

    fn unsupported(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
            code: MetaDiagnosticCode::ProviderUnavailable,
        }
    }
}

impl From<(String, String)> for ProjectionError {
    fn from((field, message): (String, String)) -> Self {
        Self::malformed(field, message)
    }
}

#[cfg(test)]
pub(super) fn validate_meta_info_profile(
    kind: MetadataKind,
    properties: Option<roxmltree::Node<'_, '_>>,
    child_objects: Option<roxmltree::Node<'_, '_>>,
) -> Result<(), ProjectionError> {
    match meta_info_profile_errors(kind, properties, child_objects)
        .into_iter()
        .next()
    {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

pub(super) fn meta_info_profile_errors(
    kind: MetadataKind,
    properties: Option<roxmltree::Node<'_, '_>>,
    child_objects: Option<roxmltree::Node<'_, '_>>,
) -> Vec<ProjectionError> {
    let mut errors = Vec::new();
    if child_objects.is_some()
        && !super::format_contract::meta_8_3_27_kind_declares_child_objects(kind)
    {
        errors.push(ProjectionError::unsupported(
            "collections",
            format!(
                "{} does not declare a ChildObjects container in platform 8.3.27",
                kind.as_str()
            ),
        ));
    }
    if let Some(properties) = properties {
        let mut seen_properties = std::collections::HashSet::new();
        for child in properties.children().filter(roxmltree::Node::is_element) {
            let name = child.tag_name().name();
            if child.tag_name().namespace() != Some(MD_CLASSES_NAMESPACE) {
                errors.push(ProjectionError::unsupported(
                    format!("properties.{name}"),
                    format!(
                        "{} contains a foreign-namespace property `{name}`",
                        kind.as_str()
                    ),
                ));
                continue;
            }
            if !seen_properties.insert(name) {
                errors.push(ProjectionError::unsupported(
                    format!("properties.{name}"),
                    format!("{} contains a duplicate property `{name}`", kind.as_str()),
                ));
                continue;
            }
            if let Some(spec) = META_INFO_PROPERTY_PROFILE.resolve(kind, name) {
                let malformed = !meta_info_property_value_is_valid(child, spec.value_kind);
                if malformed {
                    errors.push(ProjectionError::malformed(
                        format!("properties.{name}"),
                        format!("{} contains a malformed property `{name}`", kind.as_str()),
                    ));
                }
            } else {
                match MetaInfoKindProfile::new(kind).property_route(name) {
                    Some(MetaInfoPropertyRoute::Ignored(
                        MetaInfoIgnoredReason::ProvenEmptyPlaceholder,
                    )) if !node_is_empty_placeholder(child) => {
                        errors.push(ProjectionError::unsupported(
                            format!("properties.{name}"),
                            format!(
                                "{} contains unsupported semantic content in `{name}`",
                                kind.as_str()
                            ),
                        ))
                    }
                    Some(MetaInfoPropertyRoute::Identity)
                    | Some(MetaInfoPropertyRoute::Details)
                        if matches!(
                            name,
                            "Name" | "MethodName" | "Schedule" | "ScheduleValue" | "ScheduleDate"
                        ) && !strict_text_leaf_is_valid(child) =>
                    {
                        errors.push(ProjectionError::malformed(
                            format!("properties.{name}"),
                            format!("{} contains malformed text in `{name}`", kind.as_str()),
                        ));
                    }
                    Some(_) => {}
                    None => errors.push(ProjectionError::unsupported(
                        format!("properties.{name}"),
                        format!(
                            "{} contains an unclassified property `{name}`",
                            kind.as_str(),
                        ),
                    )),
                }
            }
        }
    }
    if let Some(child_objects) = child_objects {
        for child in child_objects.children().filter(roxmltree::Node::is_element) {
            let name = child.tag_name().name();
            if child.tag_name().namespace() != Some(MD_CLASSES_NAMESPACE)
                || MetaInfoKindProfile::new(kind).child_route(name).is_none()
            {
                errors.push(ProjectionError::unsupported(
                    format!("collections.{name}"),
                    format!(
                        "{} contains an unclassified child object `{name}`",
                        kind.as_str()
                    ),
                ));
            }
        }
    }
    errors
}

pub(super) fn project_optional_collection_item_properties(
    node: roxmltree::Node<'_, '_>,
    field: &str,
) -> Result<Vec<MetaObservedProperty>, ProjectionError> {
    let tag = node.tag_name().name();
    if !matches!(
        tag,
        "Recalculation" | "AccountingFlag" | "ExtDimensionAccountingFlag" | "AddressingAttribute"
    ) {
        return Ok(Vec::new());
    }
    if node.tag_name().namespace() != Some(MD_CLASSES_NAMESPACE)
        || node.attributes().len() > 1
        || node.attributes().any(|attribute| {
            attribute.namespace().is_some()
                || attribute.name() != "uuid"
                || uuid::Uuid::parse_str(attribute.value()).is_err()
        })
    {
        return Err(ProjectionError::unsupported(
            field,
            "metadata collection item has an unknown name, namespace, or attribute",
        ));
    }
    let element_children = node
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    if element_children.is_empty() {
        let value = direct_text_content(node);
        if tag != "Recalculation"
            || value.trim().is_empty()
            || !metadata_identifier_is_valid(value.trim())
        {
            return Err(ProjectionError::malformed(
                format!("{field}.name"),
                "metadata collection reference has no proved identity",
            ));
        }
        return Ok(Vec::new());
    }
    let [properties] = element_children.as_slice() else {
        return Err(ProjectionError::unsupported(
            field,
            "metadata collection item contains unknown or duplicate nested structure",
        ));
    };
    if properties.tag_name().namespace() != Some(MD_CLASSES_NAMESPACE)
        || properties.tag_name().name() != "Properties"
        || properties.attributes().len() != 0
        || !node_has_only_whitespace_text(node)
        || !node_has_only_whitespace_text(*properties)
    {
        return Err(ProjectionError::unsupported(
            field,
            "metadata collection item Properties has unsupported structure",
        ));
    }
    let allowed = match tag {
        "Recalculation" => &["Name", "Synonym", "Comment"][..],
        "AddressingAttribute" => &[
            "Name",
            "Synonym",
            "Comment",
            "Type",
            "AddressingDimension",
            "Indexing",
            "FullTextSearch",
            "DataHistory",
        ][..],
        _ => &[
            "Name",
            "Synonym",
            "Comment",
            "Type",
            "PasswordMode",
            "Format",
            "EditFormat",
            "ToolTip",
            "MarkNegatives",
            "Mask",
            "MultiLine",
            "ExtendedEdit",
            "MinValue",
            "MaxValue",
            "FillChecking",
            "ChoiceParameterLinks",
            "ChoiceParameters",
            "QuickChoice",
            "ChoiceForm",
            "LinkByType",
            "ChoiceHistoryOnInput",
        ][..],
    };
    let mut seen = std::collections::HashSet::new();
    let mut observed = Vec::new();
    for child in properties.children().filter(roxmltree::Node::is_element) {
        let name = child.tag_name().name();
        if child.tag_name().namespace() != Some(MD_CLASSES_NAMESPACE)
            || !allowed.contains(&name)
            || !seen.insert(name)
        {
            return Err(ProjectionError::unsupported(
                format!("{field}.properties.{name}"),
                "metadata collection item contains an unknown or duplicate property",
            ));
        }
        match name {
            "Name" => {
                let value = direct_text_content(child);
                if !strict_text_leaf_is_valid(child) || !metadata_identifier_is_valid(value.trim())
                {
                    return Err(ProjectionError::malformed(
                        format!("{field}.name"),
                        "metadata collection item has no proved identity",
                    ));
                }
            }
            "Synonym" => {
                if !meta_info_property_value_is_valid(
                    child,
                    MetaInfoPropertyValueKind::LegacyLocalizedString,
                ) {
                    return Err(ProjectionError::malformed(
                        format!("{field}.synonym"),
                        "metadata collection item synonym is malformed",
                    ));
                }
            }
            "Comment" | "FillChecking" | "AddressingDimension" => {
                if !strict_text_leaf_is_valid(child) {
                    return Err(ProjectionError::malformed(
                        format!("{field}.{name}"),
                        "metadata collection item scalar contains nested markup",
                    ));
                }
            }
            "Type" | "FillValue" => {}
            _ => observed.push(MetaObservedProperty {
                name: name.to_string(),
                value: parse_standard_attribute_property(
                    child,
                    &format!("{field}.properties.{name}"),
                )?,
            }),
        }
    }
    if !seen.contains("Name")
        || (matches!(
            tag,
            "AccountingFlag" | "ExtDimensionAccountingFlag" | "AddressingAttribute"
        ) && !seen.contains("Type"))
    {
        return Err(ProjectionError::malformed(
            format!("{field}.properties"),
            "metadata collection item is missing a required property",
        ));
    }
    Ok(observed)
}

fn strict_text_leaf_is_valid(node: roxmltree::Node<'_, '_>) -> bool {
    node.attributes().len() == 0 && !node.children().any(|child| child.is_element())
}

pub(super) fn direct_text_content(node: roxmltree::Node<'_, '_>) -> String {
    node.children()
        .filter(roxmltree::Node::is_text)
        .filter_map(|child| child.text())
        .collect()
}

pub(super) fn meta_info_property_value_is_valid(
    node: roxmltree::Node<'_, '_>,
    value_kind: MetaInfoPropertyValueKind,
) -> bool {
    if node.tag_name().namespace() != Some(MD_CLASSES_NAMESPACE) {
        return false;
    }
    let has_element = node.children().any(|child| child.is_element());
    let value = direct_text_content(node);
    match value_kind {
        MetaInfoPropertyValueKind::Boolean => {
            node.attributes().len() == 0
                && !has_element
                && matches!(value.as_str(), "true" | "false")
        }
        MetaInfoPropertyValueKind::UnsignedInteger => {
            node.attributes().len() == 0 && !has_element && value.parse::<u32>().is_ok()
        }
        MetaInfoPropertyValueKind::String => node.attributes().len() == 0 && !has_element,
        MetaInfoPropertyValueKind::LegacyLocalizedString
        | MetaInfoPropertyValueKind::LocalizedString => {
            node.attributes().len() == 0 && localized_string_is_valid(node)
        }
        MetaInfoPropertyValueKind::TypedValue => {
            parse_typed_or_nil_value(node, "properties").is_ok()
        }
    }
}

pub(super) fn parsed_typed_meta_info_property_value(
    node: roxmltree::Node<'_, '_>,
) -> Option<MetaObservedPropertyValue> {
    parse_typed_or_nil_value(node, "properties").ok()
}

pub(super) fn parsed_localized_meta_info_property_value(
    node: roxmltree::Node<'_, '_>,
) -> MetaObservedPropertyValue {
    MetaObservedPropertyValue::LocalizedString {
        values: strict_localized_text(node),
    }
}

fn localized_string_is_valid(node: roxmltree::Node<'_, '_>) -> bool {
    let items = node
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    if items.is_empty() {
        return true;
    }
    if node
        .children()
        .filter(roxmltree::Node::is_text)
        .any(|child| child.text().is_some_and(|text| !text.trim().is_empty()))
    {
        return false;
    }
    let mut languages = std::collections::HashSet::new();
    items.into_iter().all(|item| {
        if item.tag_name().namespace() != Some(DATA_CORE_NAMESPACE)
            || item.tag_name().name() != "item"
            || item.attributes().len() != 0
        {
            return false;
        }
        let children = item
            .children()
            .filter(roxmltree::Node::is_element)
            .collect::<Vec<_>>();
        let exact_child = |name: &str| {
            children
                .iter()
                .filter(|child| {
                    child.tag_name().namespace() == Some(DATA_CORE_NAMESPACE)
                        && child.tag_name().name() == name
                })
                .count()
                == 1
        };
        let language = children.iter().find(|child| {
            child.tag_name().namespace() == Some(DATA_CORE_NAMESPACE)
                && child.tag_name().name() == "lang"
        });
        let content = children.iter().find(|child| {
            child.tag_name().namespace() == Some(DATA_CORE_NAMESPACE)
                && child.tag_name().name() == "content"
        });
        children.len() == 2
            && exact_child("lang")
            && exact_child("content")
            && language.is_some_and(|leaf| {
                leaf.attributes().len() == 0
                    && !leaf.children().any(|nested| nested.is_element())
                    && languages.insert(direct_text_content(*leaf))
            })
            && content.is_some_and(|leaf| {
                leaf.attributes().len() == 0 && !leaf.children().any(|nested| nested.is_element())
            })
            && item
                .children()
                .filter(roxmltree::Node::is_text)
                .all(|child| child.text().is_none_or(|text| text.trim().is_empty()))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetaInfoIgnoredReason {
    ProvenEmptyPlaceholder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetaInfoPropertyRoute {
    Identity,
    Relation,
    Details,
    Declarations,
    Ignored(MetaInfoIgnoredReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetaInfoChildRoute {
    Collection,
    Details,
}

#[derive(Debug, Clone, Copy)]
struct MetaInfoKindProfile {
    kind: MetadataKind,
}

#[derive(Debug, Clone, Copy)]
struct MetaInfoPropertyRouteSpec {
    name: &'static str,
    route: MetaInfoPropertyRoute,
    kinds: &'static [MetadataKind],
}

#[derive(Debug, Clone, Copy)]
struct MetaInfoChildRouteSpec {
    name: &'static str,
    route: MetaInfoChildRoute,
    kinds: &'static [MetadataKind],
}

const REFERENCE_OWNERS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::Document,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::BusinessProcess,
    MetadataKind::Task,
    MetadataKind::ExchangePlan,
];
const COLLECTION_OWNERS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::Document,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::BusinessProcess,
    MetadataKind::Task,
    MetadataKind::ExchangePlan,
    MetadataKind::Report,
    MetadataKind::DataProcessor,
];
const REGISTER_OWNERS: &[MetadataKind] = &[
    MetadataKind::InformationRegister,
    MetadataKind::AccumulationRegister,
    MetadataKind::AccountingRegister,
    MetadataKind::CalculationRegister,
];
const STANDARD_ATTRIBUTE_OWNERS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::Document,
    MetadataKind::Enum,
    MetadataKind::InformationRegister,
    MetadataKind::AccumulationRegister,
    MetadataKind::AccountingRegister,
    MetadataKind::CalculationRegister,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::BusinessProcess,
    MetadataKind::Task,
    MetadataKind::ExchangePlan,
    MetadataKind::DocumentJournal,
];
const CHARACTERISTIC_OWNERS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::Document,
    MetadataKind::Enum,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::BusinessProcess,
    MetadataKind::Task,
    MetadataKind::ExchangePlan,
];
const FORM_OWNERS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::Document,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::BusinessProcess,
    MetadataKind::Task,
    MetadataKind::ExchangePlan,
    MetadataKind::Report,
    MetadataKind::DataProcessor,
    MetadataKind::InformationRegister,
    MetadataKind::AccumulationRegister,
    MetadataKind::AccountingRegister,
    MetadataKind::CalculationRegister,
    MetadataKind::Enum,
    MetadataKind::DocumentJournal,
];

static META_INFO_PROPERTY_ROUTES: &[MetaInfoPropertyRouteSpec] = &[
    MetaInfoPropertyRouteSpec {
        name: "Name",
        route: MetaInfoPropertyRoute::Identity,
        kinds: MetadataKind::ALL,
    },
    MetaInfoPropertyRouteSpec {
        name: "Owners",
        route: MetaInfoPropertyRoute::Relation,
        kinds: &[MetadataKind::Catalog],
    },
    MetaInfoPropertyRouteSpec {
        name: "RegisterRecords",
        route: MetaInfoPropertyRoute::Relation,
        kinds: &[MetadataKind::Document],
    },
    MetaInfoPropertyRouteSpec {
        name: "BasedOn",
        route: MetaInfoPropertyRoute::Relation,
        kinds: REFERENCE_OWNERS,
    },
    MetaInfoPropertyRouteSpec {
        name: "InputByString",
        route: MetaInfoPropertyRoute::Relation,
        kinds: REFERENCE_OWNERS,
    },
    MetaInfoPropertyRouteSpec {
        name: "DataLockFields",
        route: MetaInfoPropertyRoute::Relation,
        kinds: REFERENCE_OWNERS,
    },
    MetaInfoPropertyRouteSpec {
        name: "Source",
        route: MetaInfoPropertyRoute::Relation,
        kinds: &[MetadataKind::EventSubscription],
    },
    MetaInfoPropertyRouteSpec {
        name: "Type",
        route: MetaInfoPropertyRoute::Details,
        kinds: &[
            MetadataKind::Constant,
            MetadataKind::DefinedType,
            MetadataKind::ChartOfCharacteristicTypes,
        ],
    },
    MetaInfoPropertyRouteSpec {
        name: "XDTOPackages",
        route: MetaInfoPropertyRoute::Details,
        kinds: &[MetadataKind::WebService],
    },
    MetaInfoPropertyRouteSpec {
        name: "MethodName",
        route: MetaInfoPropertyRoute::Details,
        kinds: &[MetadataKind::ScheduledJob],
    },
    MetaInfoPropertyRouteSpec {
        name: "Schedule",
        route: MetaInfoPropertyRoute::Details,
        kinds: &[MetadataKind::CalculationRegister],
    },
    MetaInfoPropertyRouteSpec {
        name: "ScheduleValue",
        route: MetaInfoPropertyRoute::Details,
        kinds: &[MetadataKind::CalculationRegister],
    },
    MetaInfoPropertyRouteSpec {
        name: "ScheduleDate",
        route: MetaInfoPropertyRoute::Details,
        kinds: &[MetadataKind::CalculationRegister],
    },
    MetaInfoPropertyRouteSpec {
        name: "BaseCalculationTypes",
        route: MetaInfoPropertyRoute::Details,
        kinds: &[MetadataKind::ChartOfCalculationTypes],
    },
    MetaInfoPropertyRouteSpec {
        name: "RegisteredDocuments",
        route: MetaInfoPropertyRoute::Details,
        kinds: &[MetadataKind::DocumentJournal],
    },
    MetaInfoPropertyRouteSpec {
        name: "StandardAttributes",
        route: MetaInfoPropertyRoute::Declarations,
        kinds: STANDARD_ATTRIBUTE_OWNERS,
    },
    MetaInfoPropertyRouteSpec {
        name: "StandardTabularSections",
        route: MetaInfoPropertyRoute::Declarations,
        kinds: &[MetadataKind::ChartOfAccounts],
    },
    MetaInfoPropertyRouteSpec {
        name: "Characteristics",
        route: MetaInfoPropertyRoute::Declarations,
        kinds: CHARACTERISTIC_OWNERS,
    },
    MetaInfoPropertyRouteSpec {
        name: "ChoiceParameterLinks",
        route: MetaInfoPropertyRoute::Ignored(MetaInfoIgnoredReason::ProvenEmptyPlaceholder),
        kinds: &[MetadataKind::Constant],
    },
    MetaInfoPropertyRouteSpec {
        name: "ChoiceParameters",
        route: MetaInfoPropertyRoute::Ignored(MetaInfoIgnoredReason::ProvenEmptyPlaceholder),
        kinds: &[MetadataKind::Constant],
    },
];

static META_INFO_CHILD_ROUTES: &[MetaInfoChildRouteSpec] = &[
    MetaInfoChildRouteSpec {
        name: "Attribute",
        route: MetaInfoChildRoute::Collection,
        kinds: &[
            MetadataKind::Catalog,
            MetadataKind::Document,
            MetadataKind::ChartOfAccounts,
            MetadataKind::ChartOfCharacteristicTypes,
            MetadataKind::ChartOfCalculationTypes,
            MetadataKind::BusinessProcess,
            MetadataKind::Task,
            MetadataKind::ExchangePlan,
            MetadataKind::Report,
            MetadataKind::DataProcessor,
            MetadataKind::InformationRegister,
            MetadataKind::AccumulationRegister,
            MetadataKind::AccountingRegister,
            MetadataKind::CalculationRegister,
        ],
    },
    MetaInfoChildRouteSpec {
        name: "TabularSection",
        route: MetaInfoChildRoute::Collection,
        kinds: COLLECTION_OWNERS,
    },
    MetaInfoChildRouteSpec {
        name: "Form",
        route: MetaInfoChildRoute::Collection,
        kinds: FORM_OWNERS,
    },
    MetaInfoChildRouteSpec {
        name: "Template",
        route: MetaInfoChildRoute::Collection,
        kinds: FORM_OWNERS,
    },
    MetaInfoChildRouteSpec {
        name: "Command",
        route: MetaInfoChildRoute::Collection,
        kinds: FORM_OWNERS,
    },
    MetaInfoChildRouteSpec {
        name: "EnumValue",
        route: MetaInfoChildRoute::Collection,
        kinds: &[MetadataKind::Enum],
    },
    MetaInfoChildRouteSpec {
        name: "Dimension",
        route: MetaInfoChildRoute::Collection,
        kinds: REGISTER_OWNERS,
    },
    MetaInfoChildRouteSpec {
        name: "Resource",
        route: MetaInfoChildRoute::Collection,
        kinds: REGISTER_OWNERS,
    },
    MetaInfoChildRouteSpec {
        name: "Recalculation",
        route: MetaInfoChildRoute::Collection,
        kinds: &[MetadataKind::CalculationRegister],
    },
    MetaInfoChildRouteSpec {
        name: "Column",
        route: MetaInfoChildRoute::Collection,
        kinds: &[MetadataKind::DocumentJournal],
    },
    MetaInfoChildRouteSpec {
        name: "AccountingFlag",
        route: MetaInfoChildRoute::Collection,
        kinds: &[MetadataKind::ChartOfAccounts],
    },
    MetaInfoChildRouteSpec {
        name: "ExtDimensionAccountingFlag",
        route: MetaInfoChildRoute::Collection,
        kinds: &[MetadataKind::ChartOfAccounts],
    },
    MetaInfoChildRouteSpec {
        name: "AddressingAttribute",
        route: MetaInfoChildRoute::Collection,
        kinds: &[MetadataKind::Task],
    },
    MetaInfoChildRouteSpec {
        name: "URLTemplate",
        route: MetaInfoChildRoute::Details,
        kinds: &[MetadataKind::HTTPService],
    },
    MetaInfoChildRouteSpec {
        name: "Operation",
        route: MetaInfoChildRoute::Details,
        kinds: &[MetadataKind::WebService],
    },
];

impl MetaInfoKindProfile {
    const fn new(kind: MetadataKind) -> Self {
        Self { kind }
    }

    fn property_route(self, name: &str) -> Option<MetaInfoPropertyRoute> {
        META_INFO_PROPERTY_ROUTES
            .iter()
            .find(|spec| spec.name == name && spec.kinds.contains(&self.kind))
            .map(|spec| spec.route)
    }

    fn child_route(self, name: &str) -> Option<MetaInfoChildRoute> {
        META_INFO_CHILD_ROUTES
            .iter()
            .find(|spec| spec.name == name && spec.kinds.contains(&self.kind))
            .map(|spec| spec.route)
    }
}

pub(super) fn meta_info_relation_is_applicable(kind: MetadataKind, name: &str) -> bool {
    matches!(
        MetaInfoKindProfile::new(kind).property_route(name),
        Some(MetaInfoPropertyRoute::Relation)
    )
}

pub(super) fn meta_info_collection_is_applicable(kind: MetadataKind, name: &str) -> bool {
    matches!(
        MetaInfoKindProfile::new(kind).child_route(name),
        Some(MetaInfoChildRoute::Collection)
    )
}

#[cfg(test)]
pub(super) fn declared_meta_info_semantic_routes() -> std::collections::BTreeSet<String> {
    let mut routes = std::collections::BTreeSet::new();
    for spec in META_INFO_PROPERTY_ROUTES {
        for kind in spec.kinds {
            routes.insert(format!("{}.properties.{}", kind.as_str(), spec.name));
        }
    }
    for spec in META_INFO_CHILD_ROUTES {
        for kind in spec.kinds {
            routes.insert(format!("{}.childObjects.{}", kind.as_str(), spec.name));
        }
    }
    for kind in MetadataKind::ALL {
        for name in META_INFO_PROPERTY_NAMES {
            if META_INFO_PROPERTY_PROFILE.resolve(*kind, name).is_some() {
                routes.insert(format!("{}.properties.{name}", kind.as_str()));
            }
        }
    }
    routes
}

#[cfg(test)]
pub(super) fn declared_meta_info_collection_routes() -> Vec<(MetadataKind, &'static str)> {
    META_INFO_CHILD_ROUTES
        .iter()
        .filter(|spec| spec.route == MetaInfoChildRoute::Collection)
        .flat_map(|spec| spec.kinds.iter().map(move |kind| (*kind, spec.name)))
        .collect()
}

#[cfg(test)]
pub(super) fn observed_meta_info_semantic_routes(
    kind: MetadataKind,
    properties: Option<roxmltree::Node<'_, '_>>,
    child_objects: Option<roxmltree::Node<'_, '_>>,
) -> std::collections::BTreeSet<String> {
    let profile = MetaInfoKindProfile::new(kind);
    let mut routes = std::collections::BTreeSet::new();
    for node in properties
        .into_iter()
        .flat_map(|container| container.children())
        .filter(roxmltree::Node::is_element)
    {
        let name = node.tag_name().name();
        if node.tag_name().namespace() == Some(MD_CLASSES_NAMESPACE)
            && (profile.property_route(name).is_some()
                || META_INFO_PROPERTY_PROFILE.resolve(kind, name).is_some())
        {
            routes.insert(format!("{}.properties.{name}", kind.as_str()));
        }
    }
    for node in child_objects
        .into_iter()
        .flat_map(|container| container.children())
        .filter(roxmltree::Node::is_element)
    {
        let name = node.tag_name().name();
        if node.tag_name().namespace() == Some(MD_CLASSES_NAMESPACE)
            && profile.child_route(name).is_some()
        {
            routes.insert(format!("{}.childObjects.{name}", kind.as_str()));
        }
    }
    routes
}

fn node_is_empty_placeholder(node: roxmltree::Node<'_, '_>) -> bool {
    !node
        .children()
        .any(|child| child.is_element() || child.text().is_some_and(|text| !text.trim().is_empty()))
}

fn parse_standard_attributes(
    container: roxmltree::Node<'_, '_>,
) -> Result<Vec<MetaStandardAttribute>, ProjectionError> {
    ensure_container_shape(container, "standardAttributes")?;
    container
        .children()
        .filter(roxmltree::Node::is_element)
        .enumerate()
        .map(|(index, node)| {
            parse_standard_attribute(node, &format!("standardAttributes[{index}]"))
        })
        .collect()
}

fn parse_standard_attribute(
    node: roxmltree::Node<'_, '_>,
    field: &str,
) -> Result<MetaStandardAttribute, ProjectionError> {
    if node.tag_name().namespace() != Some(READABLE_NAMESPACE)
        || node.tag_name().name() != "StandardAttribute"
        || node.attributes().len() != 1
    {
        return Err(ProjectionError::unsupported(
            field.to_string(),
            "standard attribute entry has the wrong name, namespace, or attributes".to_string(),
        ));
    }
    let name = node
        .attribute("name")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ProjectionError::malformed(
                format!("{field}.name"),
                "standard attribute has no name".to_string(),
            )
        })?
        .to_string();
    if !node_has_only_whitespace_text(node) {
        return Err(ProjectionError::malformed(
            field.to_string(),
            "standard attribute contains mixed text".to_string(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    let mut properties = Vec::new();
    for child in node.children().filter(roxmltree::Node::is_element) {
        let property_name = child.tag_name().name();
        if child.tag_name().namespace() != Some(READABLE_NAMESPACE)
            || !standard_attribute_property_names().contains(&property_name)
            || !seen.insert(property_name)
        {
            return Err(ProjectionError::unsupported(
                format!("{field}.properties.{property_name}"),
                "standard attribute contains an unknown or duplicate property".to_string(),
            ));
        }
        let value = parse_standard_attribute_property(
            child,
            &format!("{field}.properties.{property_name}"),
        )?;
        properties.push(MetaObservedProperty {
            name: property_name.to_string(),
            value,
        });
    }
    Ok(MetaStandardAttribute { name, properties })
}

fn standard_attribute_property_names() -> &'static [&'static str] {
    &[
        "LinkByType",
        "FillChecking",
        "MultiLine",
        "FillFromFillingValue",
        "CreateOnInput",
        "TypeReductionMode",
        "MaxValue",
        "ToolTip",
        "ExtendedEdit",
        "Format",
        "ChoiceForm",
        "QuickChoice",
        "ChoiceHistoryOnInput",
        "EditFormat",
        "PasswordMode",
        "DataHistory",
        "MarkNegatives",
        "MinValue",
        "Synonym",
        "Comment",
        "FullTextSearch",
        "ChoiceParameterLinks",
        "FillValue",
        "Mask",
        "ChoiceParameters",
    ]
}

fn parse_standard_attribute_property(
    node: roxmltree::Node<'_, '_>,
    field: &str,
) -> Result<MetaObservedPropertyValue, ProjectionError> {
    let name = node.tag_name().name();
    if matches!(
        name,
        "MultiLine" | "FillFromFillingValue" | "ExtendedEdit" | "PasswordMode" | "MarkNegatives"
    ) {
        if node.attributes().len() != 0 || node.children().any(|child| child.is_element()) {
            return Err(ProjectionError::malformed(
                field,
                "boolean property must contain text only",
            ));
        }
        let value = direct_text_content(node);
        return parse_bool(value.trim(), field)
            .map(|value| MetaObservedPropertyValue::Boolean { value });
    }
    if matches!(name, "ToolTip" | "Format" | "EditFormat" | "Synonym") {
        if node.attributes().len() != 0 || !localized_string_is_valid(node) {
            return Err(ProjectionError::malformed(
                field,
                "localized property is malformed",
            ));
        }
        return Ok(MetaObservedPropertyValue::LocalizedString {
            values: strict_localized_text(node),
        });
    }
    if matches!(name, "MaxValue" | "MinValue" | "FillValue") {
        return parse_typed_or_nil_value(node, field);
    }
    if matches!(
        name,
        "LinkByType" | "ChoiceParameterLinks" | "ChoiceParameters"
    ) {
        if node.attributes().len() != 0 || !node_is_empty_placeholder(node) {
            return Err(ProjectionError::malformed(
                field.to_string(),
                "closed format profile requires this declaration to be empty".to_string(),
            ));
        }
        return Ok(MetaObservedPropertyValue::Empty {});
    }
    if node.attributes().len() != 0 || node.children().any(|child| child.is_element()) {
        return Err(ProjectionError::malformed(
            field,
            "text property contains nested markup",
        ));
    }
    Ok(MetaObservedPropertyValue::Text {
        value: direct_text_content(node),
    })
}

fn parse_typed_or_nil_value(
    node: roxmltree::Node<'_, '_>,
    field: &str,
) -> Result<MetaObservedPropertyValue, ProjectionError> {
    if node.children().any(|child| child.is_element()) {
        return Err(ProjectionError::malformed(
            field,
            "typed value must contain text only",
        ));
    }
    let nil = node.attributes().find(|attribute| {
        attribute.namespace() == Some(XML_SCHEMA_INSTANCE_NAMESPACE) && attribute.name() == "nil"
    });
    if let Some(nil) = nil {
        if node.attributes().len() != 1
            || nil.value() != "true"
            || !direct_text_content(node).trim().is_empty()
        {
            return Err(ProjectionError::malformed(field, "nil value is malformed"));
        }
        return Ok(MetaObservedPropertyValue::Nil {});
    }
    let xsi_type = node
        .attributes()
        .find(|attribute| {
            attribute.namespace() == Some(XML_SCHEMA_INSTANCE_NAMESPACE)
                && attribute.name() == "type"
        })
        .ok_or_else(|| ProjectionError::malformed(field, "typed value has no xsi:type"))?;
    if node.attributes().len() != 1 {
        return Err(ProjectionError::malformed(
            field,
            "typed value has unexpected attributes",
        ));
    }
    Ok(MetaObservedPropertyValue::Typed {
        r#type: expand_qname(node, xsi_type.value(), field)?,
        value: direct_text_content(node),
    })
}

fn parse_characteristics(
    container: roxmltree::Node<'_, '_>,
) -> Result<Vec<MetaCharacteristic>, ProjectionError> {
    ensure_container_shape(container, "characteristics")?;
    container
        .children()
        .filter(roxmltree::Node::is_element)
        .enumerate()
        .map(|(index, characteristic)| {
            let base = format!("characteristics[{index}]");
            if characteristic.attributes().len() != 0 {
                return Err(ProjectionError::unsupported(
                    base,
                    "Characteristic has unexpected attributes",
                ));
            }
            ensure_exact_readable_children(
                characteristic,
                "Characteristic",
                &["CharacteristicTypes", "CharacteristicValues"],
                &base,
            )?;
            let types = direct_child_with_namespace(
                characteristic,
                READABLE_NAMESPACE,
                "CharacteristicTypes",
            )
            .expect("exact child guard");
            let values = direct_child_with_namespace(
                characteristic,
                READABLE_NAMESPACE,
                "CharacteristicValues",
            )
            .expect("exact child guard");
            Ok(MetaCharacteristic {
                types: parse_characteristic_types(types, &format!("{base}.types"))?,
                values: parse_characteristic_values(values, &format!("{base}.values"))?,
            })
        })
        .collect()
}

fn parse_characteristic_types(
    node: roxmltree::Node<'_, '_>,
    base: &str,
) -> Result<MetaCharacteristicTypes, ProjectionError> {
    let source = required_source_attribute(node, base)?;
    ensure_exact_readable_children(
        node,
        "CharacteristicTypes",
        &[
            "KeyField",
            "TypesFilterField",
            "TypesFilterValue",
            "DataPathField",
            "MultipleValuesUseField",
        ],
        base,
    )?;
    let filter = direct_child_with_namespace(node, READABLE_NAMESPACE, "TypesFilterValue")
        .expect("exact child guard");
    Ok(MetaCharacteristicTypes {
        source,
        key_field: required_readable_text(node, "KeyField", &format!("{base}.keyField"))?,
        types_filter_field: required_readable_text(
            node,
            "TypesFilterField",
            &format!("{base}.typesFilterField"),
        )?,
        types_filter_value: parse_typed_or_nil_value(filter, &format!("{base}.typesFilterValue"))?,
        data_path_field: required_readable_text(
            node,
            "DataPathField",
            &format!("{base}.dataPathField"),
        )?,
        multiple_values_use_field: required_readable_text(
            node,
            "MultipleValuesUseField",
            &format!("{base}.multipleValuesUseField"),
        )?,
    })
}

fn parse_characteristic_values(
    node: roxmltree::Node<'_, '_>,
    base: &str,
) -> Result<MetaCharacteristicValues, ProjectionError> {
    let source = required_source_attribute(node, base)?;
    ensure_exact_readable_children(
        node,
        "CharacteristicValues",
        &[
            "ObjectField",
            "TypeField",
            "ValueField",
            "MultipleValuesKeyField",
            "MultipleValuesOrderField",
        ],
        base,
    )?;
    Ok(MetaCharacteristicValues {
        source,
        object_field: required_readable_text(node, "ObjectField", &format!("{base}.objectField"))?,
        type_field: required_readable_text(node, "TypeField", &format!("{base}.typeField"))?,
        value_field: required_readable_text(node, "ValueField", &format!("{base}.valueField"))?,
        multiple_values_key_field: required_readable_text(
            node,
            "MultipleValuesKeyField",
            &format!("{base}.multipleValuesKeyField"),
        )?,
        multiple_values_order_field: required_readable_text(
            node,
            "MultipleValuesOrderField",
            &format!("{base}.multipleValuesOrderField"),
        )?,
    })
}

fn parse_standard_tabular_sections(
    container: roxmltree::Node<'_, '_>,
) -> Result<Vec<MetaStandardTabularSection>, ProjectionError> {
    ensure_container_shape(container, "standardTabularSections")?;
    container
        .children()
        .filter(roxmltree::Node::is_element)
        .enumerate()
        .map(|(index, section)| {
            let base = format!("standardTabularSections[{index}]");
            if section.tag_name().namespace() != Some(READABLE_NAMESPACE)
                || section.tag_name().name() != "StandardTabularSection"
                || section.attributes().len() != 1
            {
                return Err(ProjectionError::unsupported(
                    base,
                    "standard tabular section header is malformed",
                ));
            }
            let name = section
                .attribute("name")
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ProjectionError::malformed(
                        format!("{base}.name"),
                        "standard tabular section has no name".to_string(),
                    )
                })?
                .to_string();
            ensure_exact_readable_children(
                section,
                "StandardTabularSection",
                &[
                    "Synonym",
                    "Comment",
                    "ToolTip",
                    "FillChecking",
                    "StandardAttributes",
                ],
                &base,
            )?;
            let synonym_node = direct_child_with_namespace(section, READABLE_NAMESPACE, "Synonym")
                .expect("exact child guard");
            let tooltip_node = direct_child_with_namespace(section, READABLE_NAMESPACE, "ToolTip")
                .expect("exact child guard");
            if synonym_node.attributes().len() != 0 || tooltip_node.attributes().len() != 0 {
                return Err(ProjectionError::unsupported(
                    base,
                    "standard tabular section localized text has unexpected attributes",
                ));
            }
            if !localized_string_is_valid(synonym_node) || !localized_string_is_valid(tooltip_node)
            {
                return Err(ProjectionError::malformed(
                    base,
                    "standard tabular section localized text is malformed",
                ));
            }
            let attributes =
                direct_child_with_namespace(section, READABLE_NAMESPACE, "StandardAttributes")
                    .expect("exact child guard");
            Ok(MetaStandardTabularSection {
                name,
                synonym: Some(strict_localized_text(synonym_node)),
                comment: Some(required_readable_text(
                    section,
                    "Comment",
                    &format!("{base}.comment"),
                )?),
                tool_tip: Some(strict_localized_text(tooltip_node)),
                fill_checking: required_readable_text(
                    section,
                    "FillChecking",
                    &format!("{base}.fillChecking"),
                )?,
                standard_attributes: parse_standard_attributes(attributes)?,
            })
        })
        .collect()
}

fn ensure_container_shape(
    container: roxmltree::Node<'_, '_>,
    field: &str,
) -> Result<(), ProjectionError> {
    if container.attributes().len() != 0 || !node_has_only_whitespace_text(container) {
        return Err(ProjectionError::malformed(
            field.to_string(),
            "declaration container has unexpected attributes or mixed text".to_string(),
        ));
    }
    Ok(())
}

fn ensure_exact_readable_children(
    node: roxmltree::Node<'_, '_>,
    expected_node: &str,
    expected_children: &[&str],
    field: &str,
) -> Result<(), ProjectionError> {
    if node.tag_name().namespace() != Some(READABLE_NAMESPACE)
        || node.tag_name().name() != expected_node
        || !node_has_only_whitespace_text(node)
    {
        return Err(ProjectionError::unsupported(
            field,
            format!("{expected_node} is malformed"),
        ));
    }
    for expected in expected_children {
        if direct_children_with_namespace(node, READABLE_NAMESPACE, expected).len() != 1 {
            return Err(ProjectionError::unsupported(
                field.to_string(),
                format!("{expected_node}.{expected} must occur exactly once"),
            ));
        }
    }
    if node.children().filter(roxmltree::Node::is_element).count() != expected_children.len() {
        return Err(ProjectionError::unsupported(
            field.to_string(),
            format!("{expected_node} contains an unexpected child"),
        ));
    }
    Ok(())
}

fn required_source_attribute(
    node: roxmltree::Node<'_, '_>,
    field: &str,
) -> Result<String, ProjectionError> {
    if node.attributes().len() != 1 {
        return Err(ProjectionError::malformed(
            format!("{field}.source"),
            "characteristic source attributes are malformed".to_string(),
        ));
    }
    node.attribute("from")
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            ProjectionError::malformed(
                format!("{field}.source"),
                "characteristic source is missing".to_string(),
            )
        })
}

fn required_readable_text(
    node: roxmltree::Node<'_, '_>,
    name: &str,
    field: &str,
) -> Result<String, ProjectionError> {
    let child = direct_child_with_namespace(node, READABLE_NAMESPACE, name).ok_or_else(|| {
        ProjectionError::unsupported(field.to_string(), format!("{name} is missing"))
    })?;
    if child.attributes().len() != 0 || child.children().any(|nested| nested.is_element()) {
        return Err(ProjectionError::malformed(
            field,
            format!("{name} must contain text only"),
        ));
    }
    Ok(direct_text_content(child))
}

fn node_has_only_whitespace_text(node: roxmltree::Node<'_, '_>) -> bool {
    node.children()
        .filter(roxmltree::Node::is_text)
        .all(|text| text.text().is_none_or(|value| value.trim().is_empty()))
}

fn strict_localized_text(node: roxmltree::Node<'_, '_>) -> Vec<MetaLocalizedText> {
    node.children()
        .filter(|item| {
            item.is_element()
                && item.tag_name().namespace() == Some(DATA_CORE_NAMESPACE)
                && item.tag_name().name() == "item"
        })
        .map(|item| {
            let language = direct_child_with_namespace(item, DATA_CORE_NAMESPACE, "lang")
                .map(direct_text_content)
                .unwrap_or_default();
            let content = direct_child_with_namespace(item, DATA_CORE_NAMESPACE, "content")
                .map(direct_text_content)
                .unwrap_or_default();
            MetaLocalizedText { language, content }
        })
        .collect()
}

fn required_child_text(
    properties: roxmltree::Node<'_, '_>,
    name: &str,
    field: impl Into<String>,
) -> Result<String, ProjectionError> {
    let field = field.into();
    let children = direct_children_with_namespace(properties, MD_CLASSES_NAMESPACE, name);
    let [child] = children.as_slice() else {
        return Err(ProjectionError::unsupported(
            field,
            format!("{name} must occur exactly once, found {}", children.len()),
        ));
    };
    if child.attributes().len() != 0 || child.children().any(|node| node.is_element()) {
        return Err(ProjectionError::malformed(
            field,
            format!("{name} must contain text only"),
        ));
    }
    let value = direct_text_content(*child);
    (!value.trim().is_empty())
        .then_some(value)
        .ok_or_else(|| ProjectionError::malformed(field, format!("{name} is missing or empty")))
}

fn ensure_only_direct_md_children(
    container: roxmltree::Node<'_, '_>,
    allowed: &[&str],
    field: &str,
) -> Result<(), ProjectionError> {
    if container.attributes().len() != 0 || !node_has_only_whitespace_text(container) {
        return Err(ProjectionError::unsupported(
            field,
            "nested metadata container has unexpected attributes or mixed text",
        ));
    }
    if let Some(unexpected) = container.children().find(|child| {
        child.is_element()
            && (child.tag_name().namespace() != Some(MD_CLASSES_NAMESPACE)
                || !allowed.contains(&child.tag_name().name()))
    }) {
        return Err(ProjectionError::unsupported(
            field.to_string(),
            format!(
                "unexpected nested metadata child `{}`",
                unexpected.tag_name().name()
            ),
        ));
    }
    Ok(())
}

fn ensure_unique_direct_md_children(
    container: roxmltree::Node<'_, '_>,
    allowed: &[&str],
    field: &str,
) -> Result<(), ProjectionError> {
    if matches!(container.tag_name().name(), "Properties" | "ChildObjects") {
        ensure_only_direct_md_children(container, allowed, field)?;
    } else {
        let attributes_are_canonical = match container.attributes().len() {
            0 => true,
            1 => container.attributes().next().is_some_and(|attribute| {
                attribute.namespace().is_none()
                    && attribute.name() == "uuid"
                    && uuid::Uuid::parse_str(attribute.value()).is_ok()
            }),
            _ => false,
        };
        if !attributes_are_canonical || !node_has_only_whitespace_text(container) {
            return Err(ProjectionError::unsupported(
                field,
                "nested metadata object has unexpected attributes or mixed text",
            ));
        }
        if let Some(unexpected) = container.children().find(|child| {
            child.is_element()
                && (child.tag_name().namespace() != Some(MD_CLASSES_NAMESPACE)
                    || !allowed.contains(&child.tag_name().name()))
        }) {
            return Err(ProjectionError::unsupported(
                field,
                format!(
                    "unexpected nested metadata child `{}`",
                    unexpected.tag_name().name()
                ),
            ));
        }
    }
    let mut seen = std::collections::HashSet::new();
    if let Some(duplicate) = container
        .children()
        .filter(roxmltree::Node::is_element)
        .find(|child| !seen.insert(child.tag_name().name()))
    {
        return Err(ProjectionError::unsupported(
            field.to_string(),
            format!(
                "duplicate nested metadata child `{}`",
                duplicate.tag_name().name()
            ),
        ));
    }
    Ok(())
}

fn parse_bool(value: &str, field: impl Into<String>) -> Result<bool, ProjectionError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ProjectionError::malformed(
            field,
            "boolean value is malformed",
        )),
    }
}

fn parse_http_template(
    node: roxmltree::Node<'_, '_>,
    index: usize,
) -> Result<MetaHttpUrlTemplate, ProjectionError> {
    let base = format!("details.urlTemplates[{index}]");
    ensure_unique_direct_md_children(node, &["Properties", "ChildObjects"], &base)?;
    let properties = direct_md_child(node, "Properties").ok_or_else(|| {
        ProjectionError::unsupported(base.clone(), "URL template has no Properties")
    })?;
    ensure_unique_direct_md_children(properties, &["Name", "Template"], &base)?;
    let name = required_child_text(properties, "Name", format!("{base}.name"))?;
    if !metadata_identifier_is_valid(&name) {
        return Err(ProjectionError::malformed(
            format!("{base}.name"),
            "URL template name is not a platform identifier",
        ));
    }
    let template = required_child_text(properties, "Template", format!("{base}.template"))?;
    let method_container = direct_md_child(node, "ChildObjects").ok_or_else(|| {
        ProjectionError::unsupported(
            format!("{base}.methods"),
            "URL template has no method collection".to_string(),
        )
    })?;
    ensure_only_direct_md_children(method_container, &["Method"], &format!("{base}.methods"))?;
    let methods = direct_children_with_namespace(method_container, MD_CLASSES_NAMESPACE, "Method")
        .into_iter()
        .enumerate()
        .map(|(method_index, method)| {
            let field = format!("{base}.methods[{method_index}]");
            ensure_unique_direct_md_children(method, &["Properties"], &field)?;
            let properties = direct_md_child(method, "Properties").ok_or_else(|| {
                ProjectionError::unsupported(field.clone(), "HTTP method has no Properties")
            })?;
            ensure_unique_direct_md_children(
                properties,
                &["Name", "HTTPMethod", "Handler"],
                &field,
            )?;
            let name = required_child_text(properties, "Name", format!("{field}.name"))?;
            let http_method =
                required_child_text(properties, "HTTPMethod", format!("{field}.httpMethod"))?;
            if !super::validation::meta_validate_valid_http_methods()
                .contains(&http_method.as_str())
            {
                return Err(ProjectionError::malformed(
                    format!("{field}.httpMethod"),
                    "HTTP method is outside the supported platform enum".to_string(),
                ));
            }
            let handler = required_child_text(properties, "Handler", format!("{field}.handler"))?;
            if !metadata_identifier_is_valid(&name) || !metadata_identifier_is_valid(&handler) {
                return Err(ProjectionError::malformed(
                    field,
                    "HTTP method name or handler is not a platform identifier".to_string(),
                ));
            }
            Ok(MetaHttpMethod {
                name,
                http_method,
                handler,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(MetaHttpUrlTemplate {
        name,
        template,
        methods,
    })
}

fn parse_xdto_packages(
    container: roxmltree::Node<'_, '_>,
) -> Result<Vec<MetaXdtoPackage>, ProjectionError> {
    if container.attributes().len() != 0 || !node_has_only_whitespace_text(container) {
        return Err(ProjectionError::unsupported(
            "details.xdtoPackages",
            "XDTO package container has unexpected attributes or mixed text",
        ));
    }
    container
        .children()
        .filter(roxmltree::Node::is_element)
        .enumerate()
        .map(|(index, item)| {
            let field = format!("details.xdtoPackages[{index}]");
            if item.tag_name().namespace() != Some(READABLE_NAMESPACE)
                || item.tag_name().name() != "Item"
                || item.attributes().len() != 0
                || !node_has_only_whitespace_text(item)
            {
                return Err(ProjectionError::unsupported(
                    field,
                    "XDTO package entry is not xr:Item",
                ));
            }
            let values = item
                .children()
                .filter(|node| node.is_element())
                .collect::<Vec<_>>();
            let mut seen = std::collections::HashSet::new();
            if values.iter().any(|child| {
                child.tag_name().namespace() != Some(READABLE_NAMESPACE)
                    || !["Presentation", "CheckState", "Value"].contains(&child.tag_name().name())
                    || !seen.insert(child.tag_name().name())
            }) {
                return Err(ProjectionError::unsupported(
                    field,
                    "XDTO package entry contains an unexpected or duplicate child".to_string(),
                ));
            }
            let value = values
                .iter()
                .find(|child| child.tag_name().name() == "Value")
                .copied();
            let Some(value) = value else {
                return Err(ProjectionError::unsupported(
                    field,
                    "XDTO package entry has no xr:Value",
                ));
            };
            if value.children().any(|node| node.is_element()) {
                return Err(ProjectionError::malformed(
                    field,
                    "XDTO package value must contain text only".to_string(),
                ));
            }
            let raw_value = direct_text_content(value);
            let raw = raw_value.trim();
            let xsi_type = value
                .attributes()
                .find(|attribute| {
                    attribute.namespace() == Some(XML_SCHEMA_INSTANCE_NAMESPACE)
                        && attribute.name() == "type"
                })
                .map(|attribute| attribute.value())
                .ok_or_else(|| {
                    ProjectionError::malformed(
                        field.clone(),
                        "XDTO package value has no xsi:type".to_string(),
                    )
                })?;
            if value.attributes().len() != 1 {
                return Err(ProjectionError::unsupported(
                    field.clone(),
                    "XDTO package value has unexpected attributes",
                ));
            }
            for child in &values {
                if child.tag_name().name() != "Value"
                    && (child.attributes().len() != 0
                        || child.children().any(|nested| nested.is_element()))
                {
                    return Err(ProjectionError::unsupported(
                        field.clone(),
                        "XDTO package entry contains an attributed or compound display field",
                    ));
                }
            }
            let expanded_type = expand_qname(value, xsi_type, &field)?;
            match (
                expanded_type.namespace.as_str(),
                expanded_type.local_name.as_str(),
            ) {
                (READABLE_NAMESPACE, "MDObjectRef") => {
                    let metadata_path =
                        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw).map_err(
                            |_| {
                                ProjectionError::malformed(
                                    field.clone(),
                                    "XDTO package reference is malformed".to_string(),
                                )
                            },
                        )?;
                    if metadata_path.segments().next() != Some("XDTOPackage")
                        || metadata_path.segments().count() != 2
                    {
                        return Err(ProjectionError::malformed(
                            field,
                            "XDTO package reference has the wrong metadata kind".to_string(),
                        ));
                    }
                    Ok(MetaXdtoPackage::Package { metadata_path })
                }
                (XML_SCHEMA_NAMESPACE, "string") if !raw.is_empty() => {
                    Ok(MetaXdtoPackage::Namespace {
                        namespace: raw.to_string(),
                    })
                }
                _ => Err(ProjectionError::unsupported(
                    field,
                    "XDTO package entry has an unsupported typed value".to_string(),
                )),
            }
        })
        .collect()
}

fn parse_metadata_object_refs(
    container: roxmltree::Node<'_, '_>,
    public_field: &str,
    expected_kind: &str,
) -> Result<Vec<MetadataAddress>, ProjectionError> {
    if container.attributes().len() != 0 || !node_has_only_whitespace_text(container) {
        return Err(ProjectionError::unsupported(
            public_field,
            "metadata reference container has unexpected attributes or mixed text",
        ));
    }
    container
        .children()
        .filter(roxmltree::Node::is_element)
        .enumerate()
        .map(|(index, item)| {
            let field = format!("{public_field}[{index}]");
            if item.tag_name().namespace() != Some(READABLE_NAMESPACE)
                || item.tag_name().name() != "Item"
            {
                return Err(ProjectionError::unsupported(
                    field,
                    "metadata reference entry is not xr:Item",
                ));
            }
            if item.children().any(|child| child.is_element()) {
                return Err(ProjectionError::malformed(
                    field,
                    "metadata reference entry must contain text only".to_string(),
                ));
            }
            let xsi_type = item
                .attributes()
                .find(|attribute| {
                    attribute.namespace() == Some(XML_SCHEMA_INSTANCE_NAMESPACE)
                        && attribute.name() == "type"
                })
                .map(|attribute| attribute.value())
                .ok_or_else(|| {
                    ProjectionError::malformed(
                        field.clone(),
                        "metadata reference has no xsi:type".to_string(),
                    )
                })?;
            if item.attributes().len() != 1 {
                return Err(ProjectionError::unsupported(
                    field,
                    "metadata reference entry has unexpected attributes",
                ));
            }
            let expanded_type = expand_qname(item, xsi_type, &field)?;
            if expanded_type.namespace != READABLE_NAMESPACE
                || expanded_type.local_name != "MDObjectRef"
            {
                return Err(ProjectionError::malformed(
                    field,
                    "metadata reference has the wrong typed value".to_string(),
                ));
            }
            let raw_value = direct_text_content(item);
            let raw = raw_value.trim();
            let address =
                MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw).map_err(|_| {
                    ProjectionError::malformed(field.clone(), "metadata reference is malformed")
                })?;
            if address.segments().next() != Some(expected_kind) || address.segments().count() != 2 {
                return Err(ProjectionError::malformed(
                    field,
                    format!("metadata reference must target {expected_kind}"),
                ));
            }
            Ok(address)
        })
        .collect()
}

fn parse_web_operation(
    node: roxmltree::Node<'_, '_>,
    index: usize,
) -> Result<MetaWebServiceOperation, ProjectionError> {
    let base = format!("details.operations[{index}]");
    ensure_unique_direct_md_children(node, &["Properties", "ChildObjects"], &base)?;
    let properties = direct_md_child(node, "Properties").ok_or_else(|| {
        ProjectionError::unsupported(
            base.clone(),
            "Web service operation has no Properties".to_string(),
        )
    })?;
    ensure_unique_direct_md_children(
        properties,
        &[
            "Name",
            "XDTOReturningValueType",
            "Nillable",
            "Transactioned",
            "ProcedureName",
        ],
        &base,
    )?;
    let name = required_child_text(properties, "Name", format!("{base}.name"))?;
    let return_node = direct_md_child(properties, "XDTOReturningValueType").ok_or_else(|| {
        ProjectionError::unsupported(
            format!("{base}.returnType"),
            "Web service operation has no returning XDTO type".to_string(),
        )
    })?;
    let return_type = expand_qname(
        return_node,
        &required_child_text(
            properties,
            "XDTOReturningValueType",
            format!("{base}.returnType"),
        )?,
        &format!("{base}.returnType"),
    )?;
    let nillable = parse_bool(
        &required_child_text(properties, "Nillable", format!("{base}.nillable"))?,
        format!("{base}.nillable"),
    )?;
    let transactioned = parse_bool(
        &required_child_text(properties, "Transactioned", format!("{base}.transactioned"))?,
        format!("{base}.transactioned"),
    )?;
    let procedure = required_child_text(properties, "ProcedureName", format!("{base}.procedure"))?;
    if !metadata_identifier_is_valid(&name) || !metadata_identifier_is_valid(&procedure) {
        return Err(ProjectionError::malformed(
            base,
            "Web service operation name or procedure is not a platform identifier".to_string(),
        ));
    }
    let parameter_container = direct_md_child(node, "ChildObjects").ok_or_else(|| {
        ProjectionError::unsupported(
            format!("{base}.parameters"),
            "Web service operation has no parameter collection".to_string(),
        )
    })?;
    ensure_only_direct_md_children(
        parameter_container,
        &["Parameter"],
        &format!("{base}.parameters"),
    )?;
    let parameters =
        direct_children_with_namespace(parameter_container, MD_CLASSES_NAMESPACE, "Parameter")
            .into_iter()
            .enumerate()
            .map(|(parameter_index, parameter)| {
                parse_web_parameter(parameter, index, parameter_index)
            })
            .collect::<Result<Vec<_>, _>>()?;
    Ok(MetaWebServiceOperation {
        name,
        return_type,
        nillable,
        transactioned,
        procedure,
        parameters,
    })
}

fn parse_web_parameter(
    node: roxmltree::Node<'_, '_>,
    operation_index: usize,
    parameter_index: usize,
) -> Result<MetaWebServiceParameter, ProjectionError> {
    let base = format!("details.operations[{operation_index}].parameters[{parameter_index}]");
    ensure_unique_direct_md_children(node, &["Properties"], &base)?;
    let properties = direct_md_child(node, "Properties").ok_or_else(|| {
        ProjectionError::unsupported(
            base.clone(),
            "Web service parameter has no Properties".to_string(),
        )
    })?;
    ensure_unique_direct_md_children(
        properties,
        &["Name", "XDTOValueType", "Nillable", "TransferDirection"],
        &base,
    )?;
    let name = required_child_text(properties, "Name", format!("{base}.name"))?;
    let type_node = direct_md_child(properties, "XDTOValueType").ok_or_else(|| {
        ProjectionError::unsupported(
            format!("{base}.type"),
            "Web service parameter has no XDTO type".to_string(),
        )
    })?;
    let r#type = expand_qname(
        type_node,
        &required_child_text(properties, "XDTOValueType", format!("{base}.type"))?,
        &format!("{base}.type"),
    )?;
    let nillable = parse_bool(
        &required_child_text(properties, "Nillable", format!("{base}.nillable"))?,
        format!("{base}.nillable"),
    )?;
    let direction =
        match required_child_text(properties, "TransferDirection", format!("{base}.direction"))?
            .as_str()
        {
            "In" => MetaTransferDirection::In,
            "Out" => MetaTransferDirection::Out,
            "InOut" => MetaTransferDirection::InOut,
            _ => {
                return Err(ProjectionError::malformed(
                    format!("{base}.direction"),
                    "Web service transfer direction is outside the platform enum".to_string(),
                ))
            }
        };
    if !metadata_identifier_is_valid(&name) {
        return Err(ProjectionError::malformed(
            format!("{base}.name"),
            "Web service parameter name is not a platform identifier".to_string(),
        ));
    }
    Ok(MetaWebServiceParameter {
        name,
        r#type,
        nillable,
        direction,
    })
}

fn expand_qname(
    node: roxmltree::Node<'_, '_>,
    raw: &str,
    field: &str,
) -> Result<MetaExpandedName, ProjectionError> {
    let (prefix, local_name) = raw.split_once(':').ok_or_else(|| {
        ProjectionError::malformed(
            field.to_string(),
            "XDTO type must be a namespace-qualified name".to_string(),
        )
    })?;
    if !xml_ncname_is_valid(prefix) || !xml_ncname_is_valid(local_name) {
        return Err(ProjectionError::malformed(
            field.to_string(),
            "XDTO type QName is malformed".to_string(),
        ));
    }
    let namespace = node.lookup_namespace_uri(Some(prefix)).ok_or_else(|| {
        ProjectionError::malformed(
            field.to_string(),
            "XDTO type QName prefix is not declared".to_string(),
        )
    })?;
    Ok(MetaExpandedName {
        namespace: namespace.to_string(),
        local_name: local_name.to_string(),
    })
}

pub(super) fn qname_resolves_to(
    node: roxmltree::Node<'_, '_>,
    raw: &str,
    expected_namespace: &str,
    expected_local_name: &str,
) -> bool {
    let Some((prefix, local_name)) = raw.split_once(':') else {
        return false;
    };
    xml_ncname_is_valid(prefix)
        && xml_ncname_is_valid(local_name)
        && local_name == expected_local_name
        && node.lookup_namespace_uri(Some(prefix)) == Some(expected_namespace)
}
