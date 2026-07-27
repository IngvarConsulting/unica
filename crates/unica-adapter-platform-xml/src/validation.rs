use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use roxmltree::{Document, Node};
use unica_format_core::{
    ports::{
        FormatDiagnostic, OwnerResolutionMode, OwnerResolutionRequest, ValidationContext,
        ValidationContextPort, ValidationContextRequest, ValidationContextResult,
        ValidationOwnerKind,
    },
    source::{ConfiguredSourceSetKind, SourceAdapterError},
};

use crate::versions::v2_20;

pub(crate) struct PlatformXmlValidation;

#[derive(Debug)]
struct ObjectIdentity {
    native_type: String,
    name: String,
    registrar_reference: Option<String>,
}

#[derive(Debug)]
struct ConfigurationOwner {
    kind: ValidationOwnerKind,
    path: PathBuf,
    registrations: Vec<(String, String)>,
    registered_languages: Vec<String>,
}

impl ValidationContextPort for PlatformXmlValidation {
    fn inspect(
        &self,
        request: &ValidationContextRequest,
    ) -> Result<ValidationContextResult, SourceAdapterError> {
        Ok(inspect(request))
    }
}

fn inspect(request: &ValidationContextRequest) -> ValidationContextResult {
    let requested_path = request.source.location().target().to_path_buf();
    let object_path = match std::fs::canonicalize(request.source.location().target()) {
        Ok(path) => path,
        Err(error) => {
            return invalid(
                vec![requested_path],
                format!("failed to resolve validation target: {error}"),
            )
        }
    };
    let mut dependencies = vec![requested_path];
    let identity = match read_object_identity(&object_path) {
        Ok(identity) => identity,
        Err(error) => return invalid(dependencies, error),
    };
    if matches!(
        identity.native_type.as_str(),
        "ExternalReport" | "ExternalDataProcessor"
    ) {
        return valid(
            dependencies,
            ValidationContext {
                owner_kind: ValidationOwnerKind::Standalone,
                owner_root: object_path,
                language_codes: Vec::new(),
                registrar_present: None,
            },
        );
    }
    let owner_path = match configuration_owner_path(request, &object_path) {
        Ok(path) => path,
        Err(error) => return invalid(dependencies, error),
    };
    push_unique(&mut dependencies, owner_path.clone());
    let owner =
        match read_configuration_owner(owner_path, request.source.configured_source_set_kind()) {
            Ok(owner) => owner,
            Err(error) => return invalid(dependencies, error),
        };
    if !owner
        .registrations
        .iter()
        .any(|(native_type, name)| native_type == &identity.native_type && name == &identity.name)
    {
        return invalid(
            dependencies,
            format!(
                "{}.{} is not registered in its source aggregate",
                identity.native_type, identity.name
            ),
        );
    }

    let mut language_codes = Vec::new();
    let mut seen_codes = HashSet::new();
    if TYPES_WITH_LIST_PRESENTATION.contains(&identity.native_type.as_str()) {
        for language_name in &owner.registered_languages {
            let language_path = owner
                .path
                .parent()
                .expect("configuration owner has a parent")
                .join("Languages")
                .join(format!("{language_name}.xml"));
            push_unique(&mut dependencies, language_path.clone());
            let code = match read_language_code(&language_path) {
                Ok(code) => code,
                Err(error) => return invalid(dependencies, error),
            };
            if seen_codes.insert(code.clone()) {
                language_codes.push(code);
            }
        }
        if language_codes.is_empty() {
            return invalid(
                dependencies,
                "source aggregate has no registered language profile".to_string(),
            );
        }
    }

    let registrar_present = identity.registrar_reference.as_deref().map(|reference| {
        let documents = owner
            .path
            .parent()
            .expect("configuration owner has a parent")
            .join("Documents");
        if !documents.is_dir() {
            return false;
        }
        match scan_registrar_documents(&documents, reference) {
            Ok((paths, found)) => {
                for path in paths {
                    push_unique(&mut dependencies, path);
                }
                found
            }
            Err(_) => false,
        }
    });

    valid(
        dependencies,
        ValidationContext {
            owner_kind: owner.kind,
            owner_root: owner.path,
            language_codes,
            registrar_present,
        },
    )
}

fn configuration_owner_path(
    request: &ValidationContextRequest,
    object_path: &Path,
) -> Result<PathBuf, String> {
    match crate::owner::resolve(&OwnerResolutionRequest {
        source: request.source.clone(),
        mode: OwnerResolutionMode::Existing,
    }) {
        Ok(resolution) => {
            if let Some(owner) = resolution.owners.into_iter().find(|owner| {
                matches!(
                    owner.configured_source_kind,
                    Some(
                        ConfiguredSourceSetKind::Configuration | ConfiguredSourceSetKind::Extension
                    )
                )
            }) {
                return Ok(owner.path);
            }
        }
        Err(error) => return Err(error.message),
    }
    let source_root = request.source.location().source_root();
    let mut current = object_path.parent();
    while let Some(directory) = current {
        if !directory.starts_with(source_root) {
            break;
        }
        let candidate = directory.join("Configuration.xml");
        if candidate.is_file() {
            return Ok(candidate);
        }
        if directory == source_root {
            break;
        }
        current = directory.parent();
    }
    Err("source aggregate owner not found".to_string())
}

fn read_object_identity(path: &Path) -> Result<ObjectIdentity, String> {
    let source = read_metadata_source(path)?;
    let document = parse_metadata_document(&source)?;
    let root = document.root_element();
    let artifacts = metadata_children(root);
    let [artifact] = artifacts.as_slice() else {
        return Err("metadata descriptor must contain exactly one object".to_string());
    };
    let native_type = artifact.tag_name().name();
    if v2_20::schema::metadata_class_profile(native_type).is_none() {
        return Err("metadata descriptor has an unsupported object kind".to_string());
    }
    let properties = child(*artifact, "Properties");
    let name = properties
        .and_then(|properties| child(properties, "Name"))
        .map(inner_text)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "metadata object name is missing".to_string())?;
    let reads_registrars = matches!(
        native_type,
        "AccumulationRegister" | "AccountingRegister" | "CalculationRegister"
    ) || (native_type == "InformationRegister"
        && properties
            .and_then(|properties| child(properties, "WriteMode"))
            .map(inner_text)
            .as_deref()
            == Some("RecorderSubordinate"));
    Ok(ObjectIdentity {
        native_type: native_type.to_string(),
        registrar_reference: reads_registrars.then(|| format!("{native_type}.{name}")),
        name,
    })
}

fn read_configuration_owner(
    path: PathBuf,
    configured_kind: Option<ConfiguredSourceSetKind>,
) -> Result<ConfigurationOwner, String> {
    let source = read_metadata_source(&path)?;
    let document = parse_metadata_document(&source)?;
    let root = document.root_element();
    let artifacts = metadata_children(root);
    let [configuration] = artifacts.as_slice() else {
        return Err("source aggregate must contain exactly one descriptor".to_string());
    };
    if configuration.tag_name().name() != "Configuration" {
        return Err("source aggregate owner is not a configuration".to_string());
    }
    let properties = child(*configuration, "Properties");
    let is_extension = properties
        .is_some_and(|properties| child(properties, "ConfigurationExtensionPurpose").is_some());
    let kind = if configured_kind == Some(ConfiguredSourceSetKind::Extension) || is_extension {
        ValidationOwnerKind::Extension
    } else {
        ValidationOwnerKind::Aggregate
    };
    let mut registrations = Vec::new();
    let mut registered_languages = Vec::new();
    if let Some(children) = child(*configuration, "ChildObjects") {
        for item in children.children().filter(Node::is_element) {
            if item.tag_name().namespace() != Some(v2_20::xml::MD_CLASSES_NS) {
                continue;
            }
            let native_type = item.tag_name().name();
            let name = inner_text(item).trim().to_string();
            if name.is_empty() {
                continue;
            }
            if native_type == "Language" {
                registered_languages.push(name);
            } else {
                registrations.push((native_type.to_string(), name));
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

fn read_language_code(path: &Path) -> Result<String, String> {
    if !path.is_file() {
        return Err("registered language file not found".to_string());
    }
    let source = read_metadata_source(path)?;
    let document = parse_metadata_document(&source)?;
    let artifacts = metadata_children(document.root_element());
    let [language] = artifacts.as_slice() else {
        return Err("registered language descriptor is invalid".to_string());
    };
    if language.tag_name().name() != "Language" {
        return Err("registered language descriptor is invalid".to_string());
    }
    child(*language, "Properties")
        .and_then(|properties| child(properties, "LanguageCode"))
        .map(inner_text)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "registered language code is empty".to_string())
}

fn read_metadata_source(path: &Path) -> Result<String, String> {
    let raw = fs::read(path).map_err(|error| format!("failed to read metadata source: {error}"))?;
    String::from_utf8(raw)
        .map(|text| text.trim_start_matches('\u{feff}').to_string())
        .map_err(|error| format!("metadata source is not UTF-8: {error}"))
}

fn parse_metadata_document<'a>(source: &'a str) -> Result<Document<'a>, String> {
    let document = Document::parse(source)
        .map_err(|error| format!("failed to parse metadata source: {error}"))?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(v2_20::xml::MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        return Err("source is not a metadata descriptor".to_string());
    }
    Ok(document)
}

fn metadata_children<'a, 'input>(root: Node<'a, 'input>) -> Vec<Node<'a, 'input>> {
    root.children()
        .filter(|node| {
            node.is_element() && node.tag_name().namespace() == Some(v2_20::xml::MD_CLASSES_NS)
        })
        .collect()
}

fn child<'a>(node: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    node.children().find(|candidate| {
        candidate.is_element()
            && candidate.tag_name().namespace() == Some(v2_20::xml::MD_CLASSES_NS)
            && candidate.tag_name().name() == name
    })
}

fn inner_text(node: Node<'_, '_>) -> String {
    node.descendants()
        .filter(Node::is_text)
        .filter_map(|descendant| descendant.text())
        .collect::<String>()
        .trim()
        .to_string()
}

pub(crate) fn scan_registrar_documents(
    documents_dir: &Path,
    register_reference: &str,
) -> Result<(Vec<PathBuf>, bool), String> {
    let mut entries = fs::read_dir(documents_dir)
        .map_err(|error| format!("failed to read registrar directory: {error}"))?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
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
        if fs::read_to_string(&path)
            .map(|content| content.contains(register_reference))
            .unwrap_or(false)
        {
            return Ok((dependencies, true));
        }
    }
    Ok((dependencies, false))
}

fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.contains(&path) {
        paths.push(path);
    }
}

fn valid(dependencies: Vec<PathBuf>, context: ValidationContext) -> ValidationContextResult {
    ValidationContextResult {
        dependencies,
        context: Some(context),
        diagnostics: Vec::new(),
    }
}

fn invalid(dependencies: Vec<PathBuf>, message: String) -> ValidationContextResult {
    ValidationContextResult {
        dependencies,
        context: None,
        diagnostics: vec![FormatDiagnostic::new("validationContextInvalid", message)],
    }
}

const TYPES_WITH_LIST_PRESENTATION: &[&str] = &[
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
];

pub(crate) fn type_uses_list_presentation(value: &str) -> bool {
    TYPES_WITH_LIST_PRESENTATION.contains(&value)
}

pub(crate) fn types_with_list_presentation() -> &'static [&'static str] {
    TYPES_WITH_LIST_PRESENTATION
}
