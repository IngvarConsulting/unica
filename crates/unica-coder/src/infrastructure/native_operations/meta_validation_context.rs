use super::common::read_utf8_sig;
use super::meta::{meta_info_child, meta_info_inner_text, meta_validate_valid_types};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform_xml_owner::{
    resolve_platform_xml_owners, PlatformXmlOwnerKind,
};
use crate::infrastructure::source_roots::normalize_path_identity;
use roxmltree::Document;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetaValidationOwnerKind {
    Configuration,
    Extension,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObjectIdentity {
    object_type: String,
    object_name: String,
    registrar_reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnerCandidate {
    kind: MetaValidationOwnerKind,
    path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnerCandidateError {
    attempted_path: Option<PathBuf>,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigurationOwner {
    kind: MetaValidationOwnerKind,
    path: PathBuf,
    registrations: Vec<(String, String)>,
    registered_languages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaValidationOwnerContext {
    pub object_type: String,
    pub object_name: String,
    pub owner_kind: MetaValidationOwnerKind,
    pub owner_path: PathBuf,
    pub language_codes: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct MetaValidationReadInspection {
    pub paths: Vec<PathBuf>,
    pub context: Result<MetaValidationOwnerContext, String>,
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

pub(crate) fn inspect_meta_validation_reads(
    object_path: &Path,
    workspace: &WorkspaceContext,
) -> MetaValidationReadInspection {
    let object_path = match normalize_path_identity(object_path) {
        Ok(path) => path,
        Err(error) => {
            return inspection_error(vec![object_path.to_path_buf()], error);
        }
    };
    let mut paths = vec![object_path.clone()];
    let identity = match read_object_identity(&object_path) {
        Ok(identity) => identity,
        Err(error) => return inspection_error(paths, error),
    };

    if matches!(
        identity.object_type.as_str(),
        "ExternalReport" | "ExternalDataProcessor"
    ) {
        return inspection_ok(
            paths,
            MetaValidationOwnerContext {
                object_type: identity.object_type,
                object_name: identity.object_name,
                owner_kind: MetaValidationOwnerKind::External,
                owner_path: object_path.to_path_buf(),
                language_codes: Vec::new(),
            },
        );
    }

    let candidate = match resolve_configuration_owner_candidate(&object_path, workspace) {
        Ok(candidate) => candidate,
        Err(error) => {
            if let Some(path) = error.attempted_path {
                stable_push(&mut paths, path);
            }
            let message = if error.message == "Configuration.xml owner not found" {
                format!(
                    "Configuration.xml owner not found for {}.{}",
                    identity.object_type, identity.object_name
                )
            } else {
                error.message
            };
            return inspection_error(paths, message);
        }
    };
    stable_push(&mut paths, candidate.path.clone());
    let owner = match read_configuration_owner(candidate.path, candidate.kind) {
        Ok(owner) => owner,
        Err(error) => return inspection_error(paths, error),
    };
    if !owner
        .registrations
        .iter()
        .any(|(object_type, object_name)| {
            object_type == &identity.object_type && object_name == &identity.object_name
        })
    {
        return inspection_error(
            paths,
            format!(
                "{}.{} is not registered in {}",
                identity.object_type,
                identity.object_name,
                owner.path.display()
            ),
        );
    }

    let mut language_codes = Vec::new();
    let mut seen_codes = HashSet::new();
    if meta_validate_types_with_list_presentation().contains(&identity.object_type.as_str()) {
        for language_name in &owner.registered_languages {
            let language_path = owner
                .path
                .parent()
                .expect("Configuration.xml has a parent")
                .join("Languages")
                .join(format!("{language_name}.xml"));
            stable_push(&mut paths, language_path.clone());
            let code = match read_required_language_code(&language_path) {
                Ok(code) => code,
                Err(error) => return inspection_error(paths, error),
            };
            if seen_codes.insert(code.clone()) {
                language_codes.push(code);
            }
        }
        if language_codes.is_empty() {
            return inspection_error(
                paths,
                format!(
                    "{} has no registered language profile",
                    owner.path.display()
                ),
            );
        }
    }

    if let Some(register_reference) = &identity.registrar_reference {
        let documents_dir = owner
            .path
            .parent()
            .expect("Configuration.xml has a parent")
            .join("Documents");
        if documents_dir.is_dir() {
            let registrar_paths =
                match meta_validate_registrar_document_scan(&documents_dir, register_reference) {
                    Ok((registrar_paths, _)) => registrar_paths,
                    Err(error) => return inspection_error(paths, error),
                };
            for registrar_path in registrar_paths {
                stable_push(&mut paths, registrar_path);
            }
        }
    }

    inspection_ok(
        paths,
        MetaValidationOwnerContext {
            object_type: identity.object_type,
            object_name: identity.object_name,
            owner_kind: owner.kind,
            owner_path: owner.path,
            language_codes,
        },
    )
}

fn resolve_configuration_owner_candidate(
    object_path: &Path,
    workspace: &WorkspaceContext,
) -> Result<OwnerCandidate, OwnerCandidateError> {
    match resolve_platform_xml_owners(object_path, workspace) {
        Ok(owners) => {
            if let Some(owner) = owners.into_iter().find(|owner| {
                matches!(
                    owner.kind,
                    PlatformXmlOwnerKind::Configuration | PlatformXmlOwnerKind::Extension
                )
            }) {
                let kind = match owner.kind {
                    PlatformXmlOwnerKind::Extension => MetaValidationOwnerKind::Extension,
                    PlatformXmlOwnerKind::Configuration => MetaValidationOwnerKind::Configuration,
                    _ => unreachable!("filtered above"),
                };
                return Ok(OwnerCandidate {
                    kind,
                    path: owner.path,
                });
            }
        }
        Err(error) => {
            return Err(OwnerCandidateError {
                attempted_path: Some(error.path),
                message: error.message,
            });
        }
    }

    let workspace_root = &workspace.workspace_root;
    let mut directory = object_path.parent();
    while let Some(current) = directory {
        if !current.starts_with(workspace_root) {
            break;
        }
        let candidate = current.join("Configuration.xml");
        if candidate.is_file() {
            return Ok(OwnerCandidate {
                kind: MetaValidationOwnerKind::Configuration,
                path: candidate,
            });
        }
        if current == workspace_root {
            break;
        }
        directory = current.parent();
    }

    Err(OwnerCandidateError {
        attempted_path: None,
        message: "Configuration.xml owner not found".to_string(),
    })
}

fn read_object_identity(path: &Path) -> Result<ObjectIdentity, String> {
    let text = read_utf8_sig(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let document = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        return Err(format!(
            "{} is not an MDClasses MetaDataObject",
            path.display()
        ));
    }
    let artifacts = root
        .children()
        .filter(|node| node.is_element() && node.tag_name().namespace() == Some(MD_CLASSES_NS))
        .collect::<Vec<_>>();
    let [artifact] = artifacts.as_slice() else {
        return Err(format!(
            "{} must contain exactly one metadata descriptor",
            path.display()
        ));
    };
    let object_type = artifact.tag_name().name();
    if !meta_validate_valid_types().contains(&object_type) {
        return Err(format!("unrecognized metadata type: {object_type}"));
    }
    let properties = meta_info_child(*artifact, "Properties");
    let object_name = properties
        .and_then(|properties| meta_info_child(properties, "Name"))
        .map(meta_info_inner_text)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("{object_type} Name is missing in {}", path.display()))?;
    let reads_registrars = matches!(
        object_type,
        "AccumulationRegister" | "AccountingRegister" | "CalculationRegister"
    ) || (object_type == "InformationRegister"
        && properties
            .and_then(|properties| meta_info_child(properties, "WriteMode"))
            .map(meta_info_inner_text)
            .as_deref()
            == Some("RecorderSubordinate"));
    let registrar_reference = reads_registrars.then(|| format!("{object_type}.{object_name}"));
    Ok(ObjectIdentity {
        object_type: object_type.to_string(),
        object_name,
        registrar_reference,
    })
}

fn read_configuration_owner(
    path: PathBuf,
    candidate_kind: MetaValidationOwnerKind,
) -> Result<ConfigurationOwner, String> {
    let text = read_utf8_sig(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let document = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        return Err(format!(
            "{} is not an MDClasses MetaDataObject",
            path.display()
        ));
    }
    let artifacts = root
        .children()
        .filter(|node| node.is_element() && node.tag_name().namespace() == Some(MD_CLASSES_NS))
        .collect::<Vec<_>>();
    let [configuration] = artifacts.as_slice() else {
        return Err(format!(
            "{} must contain exactly one Configuration descriptor",
            path.display()
        ));
    };
    if configuration.tag_name().name() != "Configuration" {
        return Err(format!("{} does not contain Configuration", path.display()));
    }
    let properties = meta_info_child(*configuration, "Properties");
    let is_extension = properties.is_some_and(|properties| {
        meta_info_child(properties, "ConfigurationExtensionPurpose").is_some()
    });
    let kind = if candidate_kind == MetaValidationOwnerKind::Extension || is_extension {
        MetaValidationOwnerKind::Extension
    } else {
        MetaValidationOwnerKind::Configuration
    };
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
    Ok(ConfigurationOwner {
        kind,
        path,
        registrations,
        registered_languages,
    })
}

fn read_required_language_code(path: &Path) -> Result<String, String> {
    if !path.is_file() {
        return Err(format!(
            "registered language file not found: {}",
            path.display()
        ));
    }
    let text = read_utf8_sig(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let document = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        return Err(format!(
            "registered language descriptor is not Language: {}",
            path.display()
        ));
    }
    let artifacts = root
        .children()
        .filter(|node| node.is_element() && node.tag_name().namespace() == Some(MD_CLASSES_NS))
        .collect::<Vec<_>>();
    let [language] = artifacts.as_slice() else {
        return Err(format!(
            "registered language descriptor is not Language: {}",
            path.display()
        ));
    };
    if language.tag_name().name() != "Language" {
        return Err(format!(
            "registered language descriptor is not Language: {}",
            path.display()
        ));
    }
    meta_info_child(*language, "Properties")
        .and_then(|properties| meta_info_child(properties, "LanguageCode"))
        .map(meta_info_inner_text)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("empty LanguageCode in {}", path.display()))
}

pub(crate) fn meta_validate_registrar_document_scan(
    documents_dir: &Path,
    register_reference: &str,
) -> Result<(Vec<PathBuf>, bool), String> {
    let mut entries = fs::read_dir(documents_dir)
        .map_err(|error| format!("failed to read {}: {error}", documents_dir.display()))?;
    let mut entries = entries.by_ref().filter_map(Result::ok).collect::<Vec<_>>();
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
        if read_utf8_sig(&path)
            .map(|content| content.contains(register_reference))
            .unwrap_or(false)
        {
            return Ok((dependencies, true));
        }
    }
    Ok((dependencies, false))
}

fn stable_push(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !paths.contains(&candidate) {
        paths.push(candidate);
    }
}

fn inspection_ok(
    paths: Vec<PathBuf>,
    context: MetaValidationOwnerContext,
) -> MetaValidationReadInspection {
    MetaValidationReadInspection {
        paths,
        context: Ok(context),
    }
}

fn inspection_error(paths: Vec<PathBuf>, error: impl Into<String>) -> MetaValidationReadInspection {
    MetaValidationReadInspection {
        paths,
        context: Err(error.into()),
    }
}
