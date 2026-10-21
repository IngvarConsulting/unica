use super::template_catalog::metadata_generated_types_8_3_27;
use crate::validation::{is_guid, meta_validate_valid_types};
use std::fmt;
use std::path::{Path, PathBuf};
use unica_source_designer::format_profile::ACTIVE_FORMAT_PROFILE;
use unica_source_designer::roots::root_version_literal;
use uuid::Uuid;
const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
pub const READABLE_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";
fn strict_leaf_text(node: roxmltree::Node<'_, '_>, label: &str) -> Result<String, String> {
    if node.attributes().len() != 0 {
        return Err(format!("{label} must not have attributes"));
    }
    let children = node.children().collect::<Vec<_>>();
    let [text] = children.as_slice() else {
        return Err(format!("{label} must contain exactly one text node"));
    };
    if !text.is_text() {
        return Err(format!("{label} must contain text only"));
    }
    let value = text.text().unwrap_or_default();
    if value.trim().is_empty() || value != value.trim() {
        return Err(format!(
            "{label} must contain non-empty text without surrounding whitespace"
        ));
    }
    Ok(value.to_string())
}
fn direct_children<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    namespace: &str,
    name: &str,
) -> Vec<roxmltree::Node<'a, 'input>> {
    node.children()
        .filter(|child| {
            child.is_element()
                && child.tag_name().namespace() == Some(namespace)
                && child.tag_name().name() == name
        })
        .collect()
}
pub fn validate_event_source_registration(
    owner_bytes: &[u8],
    expected_kind: &str,
    expected_name: &str,
) -> Result<(), String> {
    let (source, document) = unica_xml_format::read::parse_metadata_image(owner_bytes)?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
        || root_version_literal(source, root).as_deref()
            != Some(ACTIVE_FORMAT_PROFILE.export_format)
    {
        return Err(format!(
            "EventSubscription source owner must be a Platform XML MetaDataObject version {}",
            ACTIVE_FORMAT_PROFILE.export_format
        ));
    }
    let artifacts = root
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    let [configuration] = artifacts.as_slice() else {
        return Err(format!(
            "EventSubscription source owner must contain exactly one Configuration, found {}",
            artifacts.len()
        ));
    };
    if configuration.tag_name().namespace() != Some(MD_CLASSES_NS)
        || configuration.tag_name().name() != "Configuration"
    {
        return Err(
            "EventSubscription source owner must contain a direct Configuration".to_string(),
        );
    }
    let child_objects = direct_children(*configuration, MD_CLASSES_NS, "ChildObjects");
    let [child_objects] = child_objects.as_slice() else {
        return Err(format!(
            "EventSubscription source owner must contain exactly one direct ChildObjects, found {}",
            child_objects.len()
        ));
    };
    let mut registrations = 0usize;
    for registration in child_objects.children().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(MD_CLASSES_NS)
            && node.tag_name().name() == expected_kind
    }) {
        let value = strict_leaf_text(
            registration,
            &format!("EventSubscription source registration <{expected_kind}>"),
        )?;
        if value == expected_name {
            registrations += 1;
        }
    }
    if registrations != 1 {
        return Err(format!(
            "EventSubscription source must be registered exactly once as <{expected_kind}>{expected_name}</{expected_kind}>, found {registrations}"
        ));
    }
    Ok(())
}
pub fn event_source_dependency_contract(
    target: &unica_address::source_target::MetadataAddress,
) -> Result<(&str, &str, &str, &str, Option<String>), &'static str> {
    let segments = target.segments().collect::<Vec<_>>();
    match segments.as_slice() {
        [kind, name] => Ok((*kind, *name, *kind, *name, None)),
        ["CalculationRegister", register, "Recalculation", recalculation] => Ok((
            "CalculationRegister",
            *register,
            "Recalculation",
            *recalculation,
            Some(format!("{register}.{recalculation}")),
        )),
        _ => Err("event source dependency must identify one metadata object"),
    }
}
pub fn validate_event_source_dependency_descriptor(
    descriptor_bytes: &[u8],
    expected_kind: &str,
    expected_name: &str,
    expected_generated_name: Option<&str>,
    expected_generated_prefixes: &[String],
) -> Result<(), String> {
    let (source, document) = unica_xml_format::read::parse_metadata_image(descriptor_bytes)?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
        || root_version_literal(source, root).as_deref()
            != Some(ACTIVE_FORMAT_PROFILE.export_format)
    {
        return Err(format!(
            "EventSubscription source descriptor must be a Platform XML MetaDataObject version {}",
            ACTIVE_FORMAT_PROFILE.export_format
        ));
    }
    let artifacts = root
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    let [object] = artifacts.as_slice() else {
        return Err(format!(
            "EventSubscription source descriptor must contain exactly one {expected_kind}, found {} artifacts",
            artifacts.len()
        ));
    };
    if object.tag_name().namespace() != Some(MD_CLASSES_NS)
        || object.tag_name().name() != expected_kind
    {
        return Err(format!(
            "EventSubscription source descriptor must contain direct {expected_kind}"
        ));
    }
    let properties = direct_children(*object, MD_CLASSES_NS, "Properties");
    let [properties] = properties.as_slice() else {
        return Err(format!(
            "EventSubscription source descriptor must contain exactly one direct Properties, found {}",
            properties.len()
        ));
    };
    let names = direct_children(*properties, MD_CLASSES_NS, "Name");
    let [name] = names.as_slice() else {
        return Err(format!(
            "EventSubscription source descriptor must contain exactly one direct Name, found {}",
            names.len()
        ));
    };
    let actual_name = strict_leaf_text(*name, "EventSubscription source descriptor Name")?;
    if actual_name != expected_name {
        return Err(format!(
            "EventSubscription source descriptor declares {expected_kind}.{actual_name}, expected {expected_kind}.{expected_name}"
        ));
    }
    let internal_infos = direct_children(*object, MD_CLASSES_NS, "InternalInfo");
    let [internal_info] = internal_infos.as_slice() else {
        return Err(format!(
            "EventSubscription source descriptor must contain exactly one direct InternalInfo, found {}",
            internal_infos.len()
        ));
    };
    for expected_prefix in expected_generated_prefixes {
        let expected_generated_type = format!(
            "{expected_prefix}.{}",
            expected_generated_name.unwrap_or(expected_name)
        );
        let expected_category = metadata_generated_types_8_3_27(expected_kind)
            .and_then(|contracts| {
                contracts.iter().find_map(|(prefix, category)| {
                    (*prefix == expected_prefix).then_some(*category)
                })
            })
            .ok_or_else(|| {
                format!(
                    "EventSubscription source profile has no GeneratedType contract for {expected_generated_type}"
                )
            })?;
        let generated_types = internal_info
            .children()
            .filter(|node| {
                node.is_element()
                    && node.tag_name().namespace() == Some(READABLE_NS)
                    && node.tag_name().name() == "GeneratedType"
                    && node.attribute("name") == Some(expected_generated_type.as_str())
            })
            .collect::<Vec<_>>();
        let [generated_type] = generated_types.as_slice() else {
            return Err(format!(
                "EventSubscription source descriptor must declare exactly one GeneratedType name='{expected_generated_type}', found {}",
                generated_types.len()
            ));
        };
        if generated_type.attribute("category") != Some(expected_category) {
            return Err(format!(
                "EventSubscription source descriptor GeneratedType '{expected_generated_type}' must have category '{expected_category}'"
            ));
        }
        let attribute_names = generated_type
            .attributes()
            .map(|attribute| (attribute.namespace(), attribute.name()))
            .collect::<Vec<_>>();
        if attribute_names.len() != 2
            || !attribute_names.contains(&(None, "name"))
            || !attribute_names.contains(&(None, "category"))
        {
            return Err(format!(
                "EventSubscription source descriptor GeneratedType '{expected_generated_type}' must have exactly name and category attributes"
            ));
        }
        let mut identifiers = Vec::new();
        for child in generated_type.children() {
            if child.is_text() && child.text().is_some_and(|text| text.trim().is_empty()) {
                continue;
            }
            if !child.is_element()
                || child.tag_name().namespace() != Some(READABLE_NS)
                || !matches!(child.tag_name().name(), "TypeId" | "ValueId")
            {
                return Err(format!(
                    "EventSubscription source descriptor GeneratedType '{expected_generated_type}' may contain only direct xr:TypeId and xr:ValueId"
                ));
            }
            identifiers.push(child);
        }
        if identifiers.len() != 2
            || identifiers[0].tag_name().name() != "TypeId"
            || identifiers[1].tag_name().name() != "ValueId"
        {
            return Err(format!(
                "EventSubscription source descriptor GeneratedType '{expected_generated_type}' must contain exactly one direct xr:TypeId followed by exactly one direct xr:ValueId"
            ));
        }
        let mut generated_ids = Vec::with_capacity(identifiers.len());
        for identifier in identifiers {
            let label = identifier.tag_name().name();
            let value = strict_leaf_text(
                identifier,
                &format!(
                    "EventSubscription source descriptor GeneratedType '{expected_generated_type}' xr:{label}"
                ),
            )?;
            if !is_guid(&value) {
                return Err(format!(
                    "EventSubscription source descriptor GeneratedType '{expected_generated_type}' has invalid xr:{label} UUID '{value}'"
                ));
            }
            let parsed = uuid::Uuid::parse_str(&value).map_err(|_| {
                format!(
                    "EventSubscription source descriptor GeneratedType '{expected_generated_type}' has invalid xr:{label} UUID '{value}'"
                )
            })?;
            if parsed.is_nil() {
                return Err(format!(
                    "EventSubscription source descriptor GeneratedType '{expected_generated_type}' xr:{label} must be a non-nil UUID"
                ));
            }
            if parsed.get_version() != Some(uuid::Version::Random) {
                return Err(format!(
                    "EventSubscription source descriptor GeneratedType '{expected_generated_type}' xr:{label} must be a version 4 UUID"
                ));
            }
            generated_ids.push(parsed);
        }
        if generated_ids[0] == generated_ids[1] {
            return Err(format!(
                "EventSubscription source descriptor GeneratedType '{expected_generated_type}' xr:TypeId and xr:ValueId must be distinct UUIDs"
            ));
        }
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaValidationImageIdentity {
    pub object_type: String,
    pub object_name: String,
    pub object_uuid: Uuid,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaValidationImageIdentityError {
    Structural(String),
    Ambiguous(String),
    ForeignProperties(String),
    ForeignName(String),
    UnsupportedKind(String),
    MissingName(String),
    InvalidUuid(String),
}
impl MetaValidationImageIdentityError {
    pub fn is_structural(&self) -> bool {
        matches!(
            self,
            Self::Structural(_)
                | Self::Ambiguous(_)
                | Self::ForeignProperties(_)
                | Self::ForeignName(_)
        )
    }

    pub fn field(&self) -> Option<&'static str> {
        matches!(self, Self::Ambiguous(_) | Self::InvalidUuid(_)).then_some("uuid")
    }
}
impl fmt::Display for MetaValidationImageIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Structural(message)
            | Self::Ambiguous(message)
            | Self::ForeignProperties(message)
            | Self::ForeignName(message)
            | Self::UnsupportedKind(message)
            | Self::MissingName(message)
            | Self::InvalidUuid(message) => formatter.write_str(message),
        }
    }
}
impl std::error::Error for MetaValidationImageIdentityError {}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaValidationRegistrationImage {
    pub registrations: Vec<(String, String)>,
    pub registered_languages: Vec<String>,
}
pub fn inspect_metadata_image_identity(
    bytes: &[u8],
) -> Result<MetaValidationImageIdentity, MetaValidationImageIdentityError> {
    inspect_metadata_image_identity_for_kinds(bytes, meta_validate_valid_types())
}
pub fn inspect_form_metadata_image_identity(
    bytes: &[u8],
) -> Result<MetaValidationImageIdentity, MetaValidationImageIdentityError> {
    inspect_metadata_image_identity_for_kinds(bytes, &["Form", "CommonForm"])
}
fn inspect_metadata_image_identity_for_kinds(
    bytes: &[u8],
    accepted_kinds: &[&str],
) -> Result<MetaValidationImageIdentity, MetaValidationImageIdentityError> {
    let (_, document) = unica_xml_format::read::parse_metadata_image(bytes)
        .map_err(MetaValidationImageIdentityError::Structural)?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        return Err(MetaValidationImageIdentityError::Structural(
            "image is not an MDClasses MetaDataObject".to_string(),
        ));
    }
    let artifacts = root
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    let [artifact] = artifacts.as_slice() else {
        return Err(MetaValidationImageIdentityError::Ambiguous(
            "image must contain exactly one metadata descriptor".to_string(),
        ));
    };
    if artifact.tag_name().namespace() != Some(MD_CLASSES_NS) {
        return Err(MetaValidationImageIdentityError::Structural(
            "metadata descriptor is outside the MDClasses namespace".to_string(),
        ));
    }
    let object_type = artifact.tag_name().name();
    if !accepted_kinds.contains(&object_type) {
        return Err(MetaValidationImageIdentityError::UnsupportedKind(format!(
            "unrecognized metadata type: {object_type}"
        )));
    }
    let properties = artifact
        .children()
        .filter(roxmltree::Node::is_element)
        .filter(|child| child.tag_name().name() == "Properties")
        .collect::<Vec<_>>();
    if properties
        .iter()
        .any(|child| child.tag_name().namespace() != Some(MD_CLASSES_NS))
    {
        return Err(MetaValidationImageIdentityError::ForeignProperties(
            format!("{object_type} Properties is outside the MDClasses namespace"),
        ));
    }
    let [properties] = properties.as_slice() else {
        return Err(if properties.is_empty() {
            MetaValidationImageIdentityError::Structural(format!(
                "{object_type} must contain exactly one MDClasses Properties"
            ))
        } else {
            MetaValidationImageIdentityError::Ambiguous(format!(
                "{object_type} contains ambiguous Properties"
            ))
        });
    };
    let names = properties
        .children()
        .filter(roxmltree::Node::is_element)
        .filter(|child| child.tag_name().name() == "Name")
        .collect::<Vec<_>>();
    if names
        .iter()
        .any(|child| child.tag_name().namespace() != Some(MD_CLASSES_NS))
    {
        return Err(MetaValidationImageIdentityError::ForeignName(format!(
            "{object_type} Name is outside the MDClasses namespace"
        )));
    }
    let [name] = names.as_slice() else {
        return Err(if names.is_empty() {
            MetaValidationImageIdentityError::MissingName(format!("{object_type} Name is missing"))
        } else {
            MetaValidationImageIdentityError::Ambiguous(format!(
                "{object_type} contains ambiguous Name properties"
            ))
        });
    };
    if name.children().any(|child| child.is_element()) {
        return Err(MetaValidationImageIdentityError::Structural(format!(
            "{object_type} Name is not one scalar MDClasses value"
        )));
    }
    let object_name = name
        .children()
        .filter(roxmltree::Node::is_text)
        .filter_map(|child| child.text())
        .collect::<String>()
        .trim()
        .to_string();
    if object_name.is_empty() {
        return Err(MetaValidationImageIdentityError::MissingName(format!(
            "{object_type} Name is missing"
        )));
    }
    let object_uuid = artifact
        .attribute("uuid")
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| {
            MetaValidationImageIdentityError::InvalidUuid(format!(
                "{object_type} UUID is missing or invalid"
            ))
        })?;
    Ok(MetaValidationImageIdentity {
        object_type: object_type.to_string(),
        object_name,
        object_uuid,
    })
}
pub fn inspect_metadata_registration_image(
    bytes: &[u8],
) -> Result<MetaValidationRegistrationImage, String> {
    let (_, document) = unica_xml_format::read::parse_metadata_image(bytes)?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        return Err("registration image is not an MDClasses MetaDataObject".to_string());
    }
    let artifacts = root
        .children()
        .filter(|node| node.is_element() && node.tag_name().namespace() == Some(MD_CLASSES_NS))
        .collect::<Vec<_>>();
    let [configuration] = artifacts.as_slice() else {
        return Err("registration image must contain exactly one Configuration".to_string());
    };
    if configuration.tag_name().name() != "Configuration" {
        return Err("registration image does not contain Configuration".to_string());
    }

    let mut registrations = Vec::new();
    let mut registered_languages = Vec::new();
    if let Some(children) = unica_xml_format::read::child_element(*configuration, "ChildObjects") {
        for child in children.children().filter(roxmltree::Node::is_element) {
            if child.tag_name().namespace() != Some(MD_CLASSES_NS) {
                continue;
            }
            let object_type = child.tag_name().name();
            let object_name = unica_xml_format::read::direct_text(child)
                .trim()
                .to_string();
            if object_name.is_empty() {
                continue;
            }
            if object_type == "Language" {
                registered_languages.push(object_name);
            } else {
                registrations.push((object_type.to_string(), object_name));
            }
        }
    }
    Ok(MetaValidationRegistrationImage {
        registrations,
        registered_languages,
    })
}
pub fn meta_validate_types_with_list_presentation() -> &'static [&'static str] {
    &[
        "ExchangePlan",
        "Catalog",
        "Document",
        "DocumentJournal",
        "Enum",
        "ChartOfCharacteristicTypes",
        "ChartOfAccounts",
        "ChartOfCalculationTypes",
        "InformationRegister",
        "AccumulationRegister",
        "AccountingRegister",
        "CalculationRegister",
        "BusinessProcess",
        "Task",
    ]
}

pub fn meta_validate_registrar_document_scan(
    reader: &dyn unica_source_contract::discovery::ProjectDiscoveryRead,
    documents_dir: &Path,
    register_reference: &str,
) -> Result<(Vec<PathBuf>, bool), String> {
    let mut entries = reader
        .read_entries(documents_dir)
        .map_err(|error| format!("failed to read {}: {error}", documents_dir.display()))?;
    let mut entries = entries
        .by_ref()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "failed to inspect a registrar candidate in {}: {error}",
                documents_dir.display()
            )
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut dependencies = Vec::new();
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("xml")
            || !reader.path_is_file(&path)
        {
            continue;
        }
        dependencies.push(path.clone());
        let content = unica_source_designer::read::read_utf8_sig(reader, &path)?;
        if crate::validation::document_registers(content.as_bytes(), register_reference) {
            return Ok((dependencies, true));
        }
    }
    Ok((dependencies, false))
}
