use super::super::common::read_utf8_sig;
use super::template_catalog::metadata_generated_types_8_3_27;
use super::validation::{document_registers, is_guid, meta_validate_valid_types};
use super::xml_model::parse_metadata_image;
use super::{meta_info_child, meta_info_inner_text};
use crate::domain::format_profile::ACTIVE_FORMAT_PROFILE;
use crate::infrastructure::platform_xml_owner::root_version_literal;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
const READABLE_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";

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

pub(crate) fn validate_event_source_registration(
    owner_bytes: &[u8],
    expected_kind: &str,
    expected_name: &str,
) -> Result<(), String> {
    let (source, document) = parse_metadata_image(owner_bytes)?;
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

pub(crate) fn validate_event_source_dependency_descriptor(
    descriptor_bytes: &[u8],
    expected_kind: &str,
    expected_name: &str,
    expected_generated_name: Option<&str>,
    expected_generated_prefixes: &[String],
) -> Result<(), String> {
    let (source, document) = parse_metadata_image(descriptor_bytes)?;
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
pub(crate) struct MetaValidationImageIdentity {
    pub(crate) object_type: String,
    pub(crate) object_name: String,
    pub(crate) object_uuid: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MetaValidationImageIdentityError {
    Structural(String),
    Ambiguous(String),
    ForeignProperties(String),
    ForeignName(String),
    UnsupportedKind(String),
    MissingName(String),
    InvalidUuid(String),
}

impl MetaValidationImageIdentityError {
    pub(crate) fn is_structural(&self) -> bool {
        matches!(
            self,
            Self::Structural(_)
                | Self::Ambiguous(_)
                | Self::ForeignProperties(_)
                | Self::ForeignName(_)
        )
    }

    pub(crate) fn field(&self) -> Option<&'static str> {
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
pub(crate) struct MetaValidationRegistrationImage {
    pub(crate) registrations: Vec<(String, String)>,
    pub(crate) registered_languages: Vec<String>,
}

pub(crate) fn inspect_metadata_image_identity(
    bytes: &[u8],
) -> Result<MetaValidationImageIdentity, MetaValidationImageIdentityError> {
    let (_, document) =
        parse_metadata_image(bytes).map_err(MetaValidationImageIdentityError::Structural)?;
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
    if !meta_validate_valid_types().contains(&object_type) {
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

pub(crate) fn inspect_metadata_registration_image(
    bytes: &[u8],
) -> Result<MetaValidationRegistrationImage, String> {
    let (_, document) = parse_metadata_image(bytes)?;
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
    if let Some(children) = meta_info_child(*configuration, "ChildObjects") {
        for child in children.children().filter(roxmltree::Node::is_element) {
            if child.tag_name().namespace() != Some(MD_CLASSES_NS) {
                continue;
            }
            let object_type = child.tag_name().name();
            let object_name = meta_info_inner_text(child).trim().to_string();
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

pub(crate) fn inspect_metadata_language_image(
    bytes: &[u8],
) -> Result<Option<(String, String)>, String> {
    let (_, document) = parse_metadata_image(bytes)?;
    let root = document.root_element();
    let language = root.children().find(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(MD_CLASSES_NS)
            && node.tag_name().name() == "Language"
    });
    let Some(language) = language else {
        return Ok(None);
    };
    let properties = meta_info_child(language, "Properties");
    let name = properties
        .and_then(|properties| meta_info_child(properties, "Name"))
        .map(meta_info_inner_text)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Language Name is missing".to_string())?;
    let code = properties
        .and_then(|properties| meta_info_child(properties, "LanguageCode"))
        .map(meta_info_inner_text)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("Language {name} has empty LanguageCode"))?;
    Ok(Some((name, code)))
}

pub(crate) fn meta_validate_types_with_list_presentation() -> &'static [&'static str] {
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

pub(crate) fn meta_validate_registrar_document_scan(
    documents_dir: &Path,
    register_reference: &str,
) -> Result<(Vec<PathBuf>, bool), String> {
    let mut entries = fs::read_dir(documents_dir)
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
            || !path.is_file()
        {
            continue;
        }
        dependencies.push(path.clone());
        let content = read_utf8_sig(&path)?;
        if document_registers(content.as_bytes(), register_reference) {
            return Ok((dependencies, true));
        }
    }
    Ok((dependencies, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metadata::MetadataKind;

    fn xr_leaf_text<'a>(source: &'a str, label: &str) -> &'a str {
        let opening = format!("<xr:{label}>");
        let closing = format!("</xr:{label}>");
        let start = source.find(&opening).unwrap() + opening.len();
        let end = start + source[start..].find(&closing).unwrap();
        &source[start..end]
    }

    fn replace_first_xr_leaf(source: &str, label: &str, replacement: &str) -> String {
        let current = xr_leaf_text(source, label);
        source.replacen(
            &format!("<xr:{label}>{current}</xr:{label}>",),
            &format!("<xr:{label}>{replacement}</xr:{label}>",),
            1,
        )
    }

    fn temp_documents(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "unica-registrar-scan-{label}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn registrar_scan_matches_only_register_records_items() {
        let root = temp_documents("typed-path");
        let reference = "InformationRegister.Ledger";
        fs::write(
            root.join("CommentOnly.xml"),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Document><Properties><Name>CommentOnly</Name><Comment>{reference}</Comment><RegisterRecords/></Properties></Document></MetaDataObject>"#
            ),
        )
        .unwrap();

        let (_, found) = meta_validate_registrar_document_scan(&root, reference).unwrap();
        assert!(
            !found,
            "an unrelated property must not count as registrar evidence"
        );

        fs::write(
            root.join("Registrar.xml"),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Document><Properties><Name>Registrar</Name><RegisterRecords><Item>{reference}</Item></RegisterRecords></Properties></Document></MetaDataObject>"#
            ),
        )
        .unwrap();
        let (_, found) = meta_validate_registrar_document_scan(&root, reference).unwrap();
        assert!(found);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registrar_scan_propagates_unreadable_document_evidence() {
        let root = temp_documents("invalid-utf8");
        fs::write(root.join("Broken.xml"), [0xff, 0xfe]).unwrap();

        let error = meta_validate_registrar_document_scan(&root, "InformationRegister.Ledger")
            .expect_err("unreadable registrar evidence must stay unavailable");

        assert!(error.contains("UTF-8"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn event_source_registration_requires_one_exact_direct_owner_entry() {
        let owner = |registrations: &str| {
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><Configuration><Properties><Name>Configuration</Name></Properties><ChildObjects>{registrations}</ChildObjects></Configuration></MetaDataObject>"#
            )
        };
        validate_event_source_registration(
            owner("<Catalog>Items</Catalog>").as_bytes(),
            "Catalog",
            "Items",
        )
        .unwrap();

        let duplicate = owner("<Catalog>Items</Catalog><Catalog>Items</Catalog>");
        assert!(
            validate_event_source_registration(duplicate.as_bytes(), "Catalog", "Items")
                .unwrap_err()
                .contains("found 2")
        );

        let padded = owner("<Catalog> Items </Catalog>");
        assert!(
            validate_event_source_registration(padded.as_bytes(), "Catalog", "Items")
                .unwrap_err()
                .contains("surrounding whitespace")
        );
    }

    #[test]
    fn event_source_dependency_requires_exact_generated_type_shape_and_ids() {
        let (descriptor, _) = super::super::template_catalog::minimal_metadata_xml_for_tests(
            MetadataKind::Catalog,
            "Items",
        )
        .unwrap();
        let prefixes = vec!["CatalogObject".to_string(), "CatalogRef".to_string()];
        validate_event_source_dependency_descriptor(
            descriptor.as_bytes(),
            "Catalog",
            "Items",
            None,
            &prefixes,
        )
        .unwrap();

        let missing =
            descriptor.replacen("name=\"CatalogRef.Items\"", "name=\"CatalogRef.Shadow\"", 1);
        assert!(validate_event_source_dependency_descriptor(
            missing.as_bytes(),
            "Catalog",
            "Items",
            None,
            &prefixes,
        )
        .unwrap_err()
        .contains("exactly one GeneratedType"));

        let invalid_id = descriptor.replacen("<xr:TypeId>", "<xr:TypeId>not-a-uuid<!--", 1);
        assert!(validate_event_source_dependency_descriptor(
            invalid_id.as_bytes(),
            "Catalog",
            "Items",
            None,
            &prefixes,
        )
        .is_err());
    }

    #[test]
    fn event_source_dependency_rejects_nil_generated_type_ids() {
        let (descriptor, _) = super::super::template_catalog::minimal_metadata_xml_for_tests(
            MetadataKind::Catalog,
            "Items",
        )
        .unwrap();
        let prefixes = vec!["CatalogObject".to_string()];

        for label in ["TypeId", "ValueId"] {
            let nil_id =
                replace_first_xr_leaf(&descriptor, label, "00000000-0000-0000-0000-000000000000");
            let error = validate_event_source_dependency_descriptor(
                nil_id.as_bytes(),
                "Catalog",
                "Items",
                None,
                &prefixes,
            )
            .expect_err("nil GeneratedType identifiers must be rejected");
            assert!(error.contains("non-nil"), "{error}");
        }
    }

    #[test]
    fn event_source_dependency_rejects_non_v4_generated_type_ids() {
        let (descriptor, _) = super::super::template_catalog::minimal_metadata_xml_for_tests(
            MetadataKind::Catalog,
            "Items",
        )
        .unwrap();
        let prefixes = vec!["CatalogObject".to_string()];

        for label in ["TypeId", "ValueId"] {
            let version_one =
                replace_first_xr_leaf(&descriptor, label, "67e55044-10b1-11eb-adc1-0242ac120002");
            let error = validate_event_source_dependency_descriptor(
                version_one.as_bytes(),
                "Catalog",
                "Items",
                None,
                &prefixes,
            )
            .expect_err("non-v4 GeneratedType identifiers must be rejected");
            assert!(error.contains("version 4"), "{error}");
        }
    }

    #[test]
    fn event_source_dependency_rejects_equal_generated_type_ids() {
        let (descriptor, _) = super::super::template_catalog::minimal_metadata_xml_for_tests(
            MetadataKind::Catalog,
            "Items",
        )
        .unwrap();
        let prefixes = vec!["CatalogObject".to_string()];
        let type_id = xr_leaf_text(&descriptor, "TypeId");
        let equal_ids = replace_first_xr_leaf(&descriptor, "ValueId", type_id);

        let error = validate_event_source_dependency_descriptor(
            equal_ids.as_bytes(),
            "Catalog",
            "Items",
            None,
            &prefixes,
        )
        .expect_err("equal GeneratedType identifiers must be rejected");
        assert!(error.contains("distinct"), "{error}");
    }
}
