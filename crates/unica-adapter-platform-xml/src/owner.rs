use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    path::{Component, Path, PathBuf},
};

use roxmltree::Document;
use unica_format_core::source::{SourceAdapterError, SourceAdapterErrorKind, SourceContext};

use crate::versions::v2_20::schema::LEGACY_TOP_LEVEL_METADATA_CLASSES;

const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnerKind {
    Configuration,
    Extension,
    ExternalProcessor,
    ExternalReport,
    Standalone,
}

impl OwnerKind {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::Extension => "extension",
            Self::ExternalProcessor => "external_processor",
            Self::ExternalReport => "external_report",
            Self::Standalone => "standalone",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Owner {
    pub(crate) kind: OwnerKind,
    pub(crate) path: PathBuf,
    pub(crate) version: Option<String>,
    pub(crate) raw: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) enum CandidateInput {
    Exact(Vec<u8>),
    Absent,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct OwnerProvenance {
    pub(crate) candidates: BTreeMap<PathBuf, CandidateInput>,
    pub(crate) directory_memberships: BTreeMap<PathBuf, Vec<OsString>>,
}

#[derive(Debug, Clone)]
pub(crate) struct OwnerResolution {
    pub(crate) owners: Vec<Owner>,
    pub(crate) provenance: OwnerProvenance,
}

pub(crate) fn resolve(
    source: &SourceContext,
    expected_root: Option<(&str, &str)>,
    existing_only: bool,
) -> Result<OwnerResolution, SourceAdapterError> {
    let source_root = normalize_path(source.location().source_root())?;
    let target = normalize_path(source.location().target())?;
    reject_link(source.location().target())?;

    let mut provenance = OwnerProvenance::default();
    let mut owners = Vec::new();
    let mut seen = BTreeSet::new();

    for candidate in target_candidates(&target) {
        let expected = (candidate == target).then_some(expected_root).flatten();
        if let Some(owner) = read_version_owner(&candidate, expected, &mut provenance)? {
            if seen.insert(owner.path.clone()) {
                owners.push(owner);
            }
        }
    }

    let configuration = source_root.join("Configuration.xml");
    if let Some(owner) = read_optional_owner(&configuration, &mut provenance)? {
        if seen.insert(owner.path.clone()) {
            owners.push(owner);
        }
        return Ok(OwnerResolution { owners, provenance });
    }

    if target == source_root && source_root.is_dir() {
        let membership = xml_membership(&source_root)?;
        provenance
            .directory_memberships
            .insert(source_root.clone(), membership.clone());
        for name in membership {
            let path = source_root.join(name);
            let Some((path, raw)) = snapshot_candidate(&path, &mut provenance, true)? else {
                continue;
            };
            if is_config_dump_sidecar(&raw) {
                continue;
            }
            let owner = parse_owner(&path, raw)?;
            if seen.insert(owner.path.clone()) {
                owners.push(owner);
            }
        }
    } else if let Some(wrapper) = external_wrapper(&source_root, &target) {
        if let Some(owner) = read_optional_owner(&wrapper, &mut provenance)? {
            if seen.insert(owner.path.clone()) {
                owners.push(owner);
            }
        } else if !existing_only && source.configured_source_set().is_some() {
            return Err(error(format!(
                "platform XML owner is unavailable {}",
                wrapper.display()
            )));
        }
    }

    Ok(OwnerResolution { owners, provenance })
}

fn target_candidates(target: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![target.to_path_buf()];
    if let Some(wrapper) = metadata_wrapper_for_content_path(target) {
        candidates.push(wrapper);
    }
    candidates
}

fn metadata_wrapper_for_content_path(target: &Path) -> Option<PathBuf> {
    let expected_collection = match target.file_name()?.to_str()? {
        "Form.xml" => "Forms",
        "Template.xml" => "Templates",
        "Rights.xml" => "Roles",
        _ => return None,
    };
    let ext_dir = target.parent()?;
    if ext_dir.file_name()?.to_str()? != "Ext" {
        return None;
    }
    let item_dir = ext_dir.parent()?;
    let item_name = item_dir.file_name()?.to_str()?;
    let collection_dir = item_dir.parent()?;
    if collection_dir.file_name()?.to_str()? != expected_collection {
        return None;
    }
    Some(collection_dir.join(format!("{item_name}.xml")))
}

fn external_wrapper(source_root: &Path, target: &Path) -> Option<PathBuf> {
    let relative = target.strip_prefix(source_root).ok()?;
    let first = relative.components().next()?.as_os_str();
    let first_path = Path::new(first);
    let artifact = if first_path.extension().and_then(|ext| ext.to_str()) == Some("xml") {
        first_path.file_stem()?
    } else {
        first
    };
    Some(source_root.join(artifact).with_extension("xml"))
}

fn read_version_owner(
    path: &Path,
    expected_root: Option<(&str, &str)>,
    provenance: &mut OwnerProvenance,
) -> Result<Option<Owner>, SourceAdapterError> {
    let Some((path, raw)) = snapshot_candidate(path, provenance, false)? else {
        return Ok(None);
    };
    if expected_root.is_none()
        && !path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
    {
        return Ok(None);
    }
    let (source, document) = parse_document(&path, &raw)?;
    let root = document.root_element();
    let qname = (root.tag_name().namespace(), root.tag_name().name());
    if let Some((namespace, local_name)) = expected_root {
        if qname != (Some(namespace), local_name) {
            return Err(error(format!(
                "declared platform XML target root is {{{}}}{}, expected {{{namespace}}}{local_name}",
                qname.0.unwrap_or(""),
                qname.1,
            )));
        }
    }
    let supported =
        qname == (Some(MD_CLASSES_NS), "MetaDataObject") || known_standalone_root(qname);
    let version = root_version_literal(source, root);
    if version.is_none() && (!supported || version_is_inherited_when_missing(qname)) {
        return Ok(None);
    }
    if !supported {
        return Err(error(format!(
            "unsupported version-owning platform XML root {{{}}}{}",
            qname.0.unwrap_or(""),
            qname.1
        )));
    }
    parse_owner(&path, raw).map(Some)
}

fn read_optional_owner(
    path: &Path,
    provenance: &mut OwnerProvenance,
) -> Result<Option<Owner>, SourceAdapterError> {
    let Some((path, raw)) = snapshot_candidate(path, provenance, true)? else {
        return Ok(None);
    };
    parse_owner(&path, raw).map(Some)
}

fn snapshot_candidate(
    path: &Path,
    provenance: &mut OwnerProvenance,
    require_file: bool,
) -> Result<Option<(PathBuf, Vec<u8>)>, SourceAdapterError> {
    let normalized = normalize_path(path)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            provenance
                .candidates
                .insert(normalized, CandidateInput::Absent);
            return Ok(None);
        }
        Err(io_error) => {
            return Err(error(format!(
                "failed to inspect {}: {io_error}",
                path.display()
            )))
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(error(format!(
            "platform XML owner must not be a symbolic link or reparse point: {}",
            path.display()
        )));
    }
    if !metadata.is_file() {
        if require_file {
            return Err(error(format!(
                "platform XML owner is not a regular file: {}",
                path.display()
            )));
        }
        return Ok(None);
    }
    let raw = fs::read(path)
        .map_err(|read_error| error(format!("failed to read {}: {read_error}", path.display())))?;
    provenance
        .candidates
        .insert(normalized.clone(), CandidateInput::Exact(raw.clone()));
    Ok(Some((normalized, raw)))
}

fn parse_owner(path: &Path, raw: Vec<u8>) -> Result<Owner, SourceAdapterError> {
    let (source, document) = parse_document(path, &raw)?;
    let root = document.root_element();
    let qname = (root.tag_name().namespace(), root.tag_name().name());
    let artifacts = root
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    let artifact = match artifacts.as_slice() {
        [artifact] => Some(*artifact),
        _ => None,
    };
    let kind = if qname == (Some(MD_CLASSES_NS), "MetaDataObject") {
        let artifact = artifact.ok_or_else(|| {
            error(format!(
                "invalid platform XML owner {}: metadata descriptor must contain exactly one artifact",
                path.display()
            ))
        })?;
        if artifact.tag_name().namespace() != Some(MD_CLASSES_NS)
            || !is_supported_metadata_artifact(artifact.tag_name().name())
        {
            return Err(error(format!(
                "invalid platform XML owner {}: unsupported metadata artifact",
                path.display()
            )));
        }
        match artifact.tag_name().name() {
            "Configuration" if is_configuration_extension_artifact(artifact) => {
                OwnerKind::Extension
            }
            "Configuration" => OwnerKind::Configuration,
            "ExternalDataProcessor" => OwnerKind::ExternalProcessor,
            "ExternalReport" => OwnerKind::ExternalReport,
            _ => OwnerKind::Standalone,
        }
    } else if known_standalone_root(qname) {
        OwnerKind::Standalone
    } else {
        return Err(error(format!(
            "invalid platform XML owner {}: unsupported root",
            path.display()
        )));
    };
    Ok(Owner {
        kind,
        path: path.to_path_buf(),
        version: root_version_literal(source, root),
        raw,
    })
}

fn parse_document<'a>(
    path: &Path,
    raw: &'a [u8],
) -> Result<(&'a str, Document<'a>), SourceAdapterError> {
    let text = std::str::from_utf8(raw).map_err(|utf8_error| {
        error(format!(
            "failed to read {} as UTF-8: {utf8_error}",
            path.display()
        ))
    })?;
    let source = text.trim_start_matches('\u{feff}');
    let document = Document::parse(source).map_err(|parse_error| {
        error(format!("failed to parse {}: {parse_error}", path.display()))
    })?;
    Ok((source, document))
}

fn root_version_literal(source: &str, root: roxmltree::Node<'_, '_>) -> Option<String> {
    root.attributes()
        .find(|attribute| attribute.namespace().is_none() && attribute.name() == "version")
        .and_then(|attribute| source.get(attribute.range_value()))
        .map(str::to_owned)
}

fn is_configuration_extension_artifact(artifact: roxmltree::Node<'_, '_>) -> bool {
    artifact
        .children()
        .find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                && node.tag_name().name() == "Properties"
        })
        .is_some_and(|properties| {
            properties.children().any(|node| {
                node.is_element()
                    && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                    && node.tag_name().name() == "ConfigurationExtensionPurpose"
            })
        })
}

fn is_supported_metadata_artifact(tag: &str) -> bool {
    LEGACY_TOP_LEVEL_METADATA_CLASSES.contains(&tag)
        || matches!(
            tag,
            "Configuration" | "ExternalDataProcessor" | "ExternalReport" | "Form" | "Template"
        )
}

fn known_standalone_root(qname: (Option<&str>, &str)) -> bool {
    matches!(
        qname,
        (Some("http://v8.1c.ru/8.3/xcf/logform"), "Form")
            | (
                Some("http://v8.1c.ru/8.3/xcf/extrnprops"),
                "CommandInterface"
            )
            | (Some("http://v8.1c.ru/8.3/xcf/extrnprops"), "Help")
            | (
                Some("http://v8.1c.ru/8.3/xcf/extrnprops"),
                "ExchangePlanContent"
            )
            | (
                Some("http://v8.1c.ru/8.3/xcf/extrnprops"),
                "HomePageWorkArea"
            )
            | (Some("http://v8.1c.ru/8.3/xcf/scheme"), "GraphicalSchema")
            | (Some("http://v8.1c.ru/8.2/roles"), "Rights")
            | (
                Some("http://v8.1c.ru/8.2/managed-application/core"),
                "ClientApplicationInterface"
            )
    )
}

fn version_is_inherited_when_missing(qname: (Option<&str>, &str)) -> bool {
    qname
        == (
            Some("http://v8.1c.ru/8.2/managed-application/core"),
            "ClientApplicationInterface",
        )
}

fn is_config_dump_sidecar(raw: &[u8]) -> bool {
    std::str::from_utf8(raw)
        .ok()
        .and_then(|text| Document::parse(text.trim_start_matches('\u{feff}')).ok())
        .is_some_and(|document| document.root_element().tag_name().name() == "ConfigDumpInfo")
}

fn xml_membership(directory: &Path) -> Result<Vec<OsString>, SourceAdapterError> {
    let mut names = fs::read_dir(directory)
        .map_err(|read_error| {
            error(format!(
                "failed to inspect {}: {read_error}",
                directory.display()
            ))
        })?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
        })
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();
    names.sort();
    Ok(names)
}

fn reject_link(path: &Path) -> Result<(), SourceAdapterError> {
    if fs::symlink_metadata(path)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(error(format!(
            "platform XML owner must not be a symbolic link or reparse point: {}",
            path.display()
        )));
    }
    Ok(())
}

fn normalize_path(path: &Path) -> Result<PathBuf, SourceAdapterError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|current_error| {
                error(format!(
                    "failed to determine current directory: {current_error}"
                ))
            })?
            .join(path)
    };
    for ancestor in absolute.ancestors() {
        if ancestor.exists() {
            let canonical = fs::canonicalize(ancestor).map_err(|canonical_error| {
                error(format!(
                    "failed to resolve {}: {canonical_error}",
                    ancestor.display()
                ))
            })?;
            let remainder = absolute.strip_prefix(ancestor).map_err(|strip_error| {
                error(format!(
                    "failed to normalize {}: {strip_error}",
                    absolute.display()
                ))
            })?;
            return Ok(normalize_lexically(&canonical.join(remainder)));
        }
    }
    Ok(normalize_lexically(&absolute))
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn error(message: String) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::DecodeCorrupted, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use unica_format_core::source::{SourceFamily, SourceLocation};

    #[test]
    fn resolves_target_and_configuration_owners_from_source_context() {
        let root =
            std::env::temp_dir().join(format!("unica-platform-owner-{}", std::process::id()));
        let target = root.join("Documents/Shipment.xml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(
            root.join("Configuration.xml"),
            format!(r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><Configuration/></MetaDataObject>"#),
        )
        .unwrap();
        fs::write(
            &target,
            format!(r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><Document/></MetaDataObject>"#),
        )
        .unwrap();
        let source = SourceContext::new(
            SourceLocation::new(root.clone(), root.clone(), target.clone()),
            Some("main".to_string()),
            SourceFamily::PlatformXml,
            None,
        );

        let resolution = resolve(&source, None, false).unwrap();

        assert_eq!(
            resolution
                .owners
                .iter()
                .map(|owner| (owner.path.clone(), owner.version.clone()))
                .collect::<Vec<_>>(),
            vec![
                (fs::canonicalize(target).unwrap(), Some("2.20".to_string())),
                (
                    fs::canonicalize(root.join("Configuration.xml")).unwrap(),
                    Some("2.20".to_string())
                ),
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn external_root_skips_runtime_config_dump_sidecar_before_owner_parsing() {
        let root = std::env::temp_dir().join(format!(
            "unica-platform-external-owner-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("ConfigDumpInfo.xml"), "<ConfigDumpInfo/>").unwrap();
        fs::write(
            root.join("Demo.xml"),
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><ExternalDataProcessor/></MetaDataObject>"#
            ),
        )
        .unwrap();
        let source = SourceContext::new(
            SourceLocation::new(root.clone(), root.clone(), root.clone()),
            Some("external".to_string()),
            SourceFamily::PlatformXml,
            None,
        );

        let resolution = resolve(&source, None, false).unwrap();

        assert_eq!(resolution.owners.len(), 1);
        assert_eq!(resolution.owners[0].kind, OwnerKind::ExternalProcessor);
        assert_eq!(
            resolution.owners[0].path,
            fs::canonicalize(root.join("Demo.xml")).unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
