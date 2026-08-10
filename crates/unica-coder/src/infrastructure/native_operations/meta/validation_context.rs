use super::super::common::read_utf8_sig;
use super::validation::{document_registers, meta_validate_valid_types};
use super::xml_model::parse_metadata_image;
use super::{meta_info_child, meta_info_inner_text};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

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
    UnsupportedKind(String),
    MissingName(String),
    InvalidUuid(String),
}

impl MetaValidationImageIdentityError {
    pub(crate) fn is_structural(&self) -> bool {
        matches!(self, Self::Structural(_) | Self::Ambiguous(_))
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
    let object_name = meta_info_child(*artifact, "Properties")
        .and_then(|properties| meta_info_child(properties, "Name"))
        .map(meta_info_inner_text)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            MetaValidationImageIdentityError::MissingName(format!("{object_type} Name is missing"))
        })?;
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
}
