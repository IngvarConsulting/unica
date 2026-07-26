use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    path::{Component, Path, PathBuf},
};

use roxmltree::Document;
use sha2::{Digest, Sha256};
use unica_format_core::{
    ports::{
        FormatCompatibility, OwnerResolutionMode, OwnerResolutionRequest, OwnerResolutionResult,
        SourceInputEvidence, SourceOwnerEvidence,
    },
    source::{
        ConfiguredSourceSetKind, FormatVersion, SourceAdapterError, SourceAdapterErrorKind,
        SourceContext,
    },
};

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

fn resolve_native(
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
    } else if matches!(
        source.configured_source_set_kind(),
        Some(ConfiguredSourceSetKind::ExternalProcessor | ConfiguredSourceSetKind::ExternalReport)
    ) {
        let Some(wrapper) = external_wrapper(&source_root, &target) else {
            return Ok(OwnerResolution { owners, provenance });
        };
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

pub(crate) fn resolve(
    request: &OwnerResolutionRequest,
) -> Result<OwnerResolutionResult, SourceAdapterError> {
    let resolution = resolve_native(
        &request.source,
        None,
        matches!(request.mode, OwnerResolutionMode::ExistingForNewOutput),
    )?;
    validate_configured_kind(&request.source, &resolution.owners)?;
    let owners = resolution
        .owners
        .iter()
        .map(owner_evidence)
        .collect::<Result<Vec<_>, _>>()?;
    let mut evidence = resolution
        .provenance
        .candidates
        .into_iter()
        .map(|(path, input)| match input {
            CandidateInput::Exact(raw) => SourceInputEvidence::ExactFileSha256 {
                path,
                sha256: format!("{:x}", Sha256::digest(raw)),
            },
            CandidateInput::Absent => SourceInputEvidence::PathAbsent { path },
        })
        .collect::<Vec<_>>();
    evidence.extend(
        resolution
            .provenance
            .directory_memberships
            .into_iter()
            .map(
                |(directory, names)| SourceInputEvidence::DirectoryMembership { directory, names },
            ),
    );
    Ok(OwnerResolutionResult { owners, evidence })
}

fn validate_configured_kind(
    source: &SourceContext,
    owners: &[Owner],
) -> Result<(), SourceAdapterError> {
    let Some(expected) = source.configured_source_set_kind() else {
        return Ok(());
    };
    let expected = match expected {
        ConfiguredSourceSetKind::Configuration => OwnerKind::Configuration,
        ConfiguredSourceSetKind::Extension => OwnerKind::Extension,
        ConfiguredSourceSetKind::ExternalProcessor => OwnerKind::ExternalProcessor,
        ConfiguredSourceSetKind::ExternalReport => OwnerKind::ExternalReport,
    };
    for actual in owners.iter().map(|owner| owner.kind).filter(|kind| {
        matches!(
            kind,
            OwnerKind::Configuration
                | OwnerKind::Extension
                | OwnerKind::ExternalProcessor
                | OwnerKind::ExternalReport
        )
    }) {
        if actual != expected {
            return Err(error(format!(
                "configured source-set kind {} does not match platform XML owner kind {}",
                expected.label(),
                actual.label()
            )));
        }
    }
    Ok(())
}

fn owner_evidence(owner: &Owner) -> Result<SourceOwnerEvidence, SourceAdapterError> {
    let compatibility = crate::versions::v2_20::profile::classify_root_version(
        owner.version.as_deref(),
    )
    .map_err(|format_error| error(format!("{} in {}", format_error, owner.path.display())))?;
    let (actual, compatibility) = match compatibility {
        crate::versions::v2_20::profile::FormatCompatibility::Older { actual } => (actual, 0),
        crate::versions::v2_20::profile::FormatCompatibility::Supported { actual } => (actual, 1),
        crate::versions::v2_20::profile::FormatCompatibility::Newer { actual } => (actual, 2),
    };
    let actual = FormatVersion::parse(&actual.to_string())?;
    let target = FormatVersion::parse(crate::versions::v2_20::EXPORT_FORMAT)?;
    let format = match compatibility {
        0 => FormatCompatibility::Older { actual, target },
        1 => FormatCompatibility::Supported { actual, target },
        _ => FormatCompatibility::Newer { actual, target },
    };
    Ok(SourceOwnerEvidence {
        configured_source_kind: match owner.kind {
            OwnerKind::Configuration => Some(ConfiguredSourceSetKind::Configuration),
            OwnerKind::Extension => Some(ConfiguredSourceSetKind::Extension),
            OwnerKind::ExternalProcessor => Some(ConfiguredSourceSetKind::ExternalProcessor),
            OwnerKind::ExternalReport => Some(ConfiguredSourceSetKind::ExternalReport),
            OwnerKind::Standalone => None,
        },
        path: owner.path.clone(),
        format,
        producer_version: producer_version(owner),
    })
}

fn producer_version(owner: &Owner) -> Option<FormatVersion> {
    let property_name = match owner.kind {
        OwnerKind::Configuration => "CompatibilityMode",
        OwnerKind::Extension => "ConfigurationExtensionCompatibilityMode",
        _ => return None,
    };
    let Ok((_, document)) = parse_document(&owner.path, &owner.raw) else {
        return None;
    };
    let Some(mode) = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == property_name)
        .and_then(|node| node.text())
        .map(str::trim)
        .filter(|mode| !mode.is_empty())
    else {
        return None;
    };
    let version = if mode == "DontUse" {
        parse_platform_version(crate::versions::v2_20::PLATFORM_LINE, '.')
    } else {
        mode.strip_prefix("Version")
            .and_then(|value| parse_platform_version(value, '_'))
    };
    let (major, minor, patch) = version?;
    FormatVersion::parse(&format!("{major}.{minor}.{patch}")).ok()
}

fn parse_platform_version(value: &str, separator: char) -> Option<(u32, u32, u32)> {
    let mut parts = value.split(separator);
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().map(str::parse).transpose().ok()?.unwrap_or(0);
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
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
    if version.is_none()
        && (expected_root.is_some() || !supported || version_is_inherited_when_missing(qname))
    {
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
            | (
                Some("http://v8.1c.ru/8.1/data-composition-system/schema"),
                "DataCompositionSchema"
            )
            | (Some("http://v8.1c.ru/8.2/data/spreadsheet"), "document")
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

        let resolution = resolve_native(&source, None, false).unwrap();

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

        let resolution = resolve_native(&source, None, false).unwrap();

        assert_eq!(resolution.owners.len(), 1);
        assert_eq!(resolution.owners[0].kind, OwnerKind::ExternalProcessor);
        assert_eq!(
            resolution.owners[0].path,
            fs::canonicalize(root.join("Demo.xml")).unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exact_declared_target_rejects_wrong_root() {
        let root = temp_root("wrong-exact-root");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("Schema.xml");
        fs::write(
            &target,
            r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema" version="2.20"/>"#,
        )
        .unwrap();
        let source = context(&root, &target);

        let error = resolve_native(
            &source,
            Some(("http://v8.1c.ru/8.3/xcf/logform", "Form")),
            false,
        )
        .unwrap_err();

        assert!(error.message.contains("expected"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn versionless_inherited_root_is_not_a_version_owner() {
        let root = temp_root("versionless-inherited");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("ClientApplicationInterface.xml");
        fs::write(
            &target,
            r#"<ClientApplicationInterface xmlns="http://v8.1c.ru/8.2/managed-application/core"/>"#,
        )
        .unwrap();

        let resolution = resolve_native(&context(&root, &target), None, false).unwrap();

        assert!(resolution.owners.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn metadata_owner_requires_exactly_one_direct_artifact() {
        let root = temp_root("multiple-artifacts");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("Demo.xml");
        fs::write(
            &target,
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><Document/><Catalog/></MetaDataObject>"#
            ),
        )
        .unwrap();

        let error = resolve_native(&context(&root, &target), None, false).unwrap_err();

        assert!(error.message.contains("exactly one artifact"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn extension_kind_requires_direct_configuration_properties_marker() {
        let root = temp_root("extension-marker");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("Configuration.xml");
        fs::write(
            &target,
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><Configuration><Properties><Nested><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose></Nested></Properties></Configuration></MetaDataObject>"#
            ),
        )
        .unwrap();

        let resolution = resolve_native(&context(&root, &target), None, false).unwrap();

        assert_eq!(resolution.owners[0].kind, OwnerKind::Configuration);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn misleading_config_dump_info_filename_is_parsed_by_content() {
        let root = temp_root("misleading-sidecar-name");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("ConfigDumpInfo.xml");
        fs::write(
            &target,
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><ExternalDataProcessor/></MetaDataObject>"#
            ),
        )
        .unwrap();

        let resolution = resolve_native(&context(&root, &root), None, false).unwrap();

        assert_eq!(resolution.owners.len(), 1);
        assert_eq!(resolution.owners[0].kind, OwnerKind::ExternalProcessor);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn new_output_inherits_versionless_configuration_as_legacy_1_0() {
        let root = temp_root("legacy-new-output");
        fs::create_dir_all(root.join("Documents")).unwrap();
        fs::write(
            root.join("Configuration.xml"),
            format!(r#"<MetaDataObject xmlns="{MD_CLASSES_NS}"><Configuration/></MetaDataObject>"#),
        )
        .unwrap();
        let target = root.join("Documents/New.xml");

        let resolution = resolve(&OwnerResolutionRequest {
            source: context(&root, &target),
            mode: OwnerResolutionMode::ExistingForNewOutput,
        })
        .unwrap();

        assert!(matches!(
            resolution.owners[0].format,
            FormatCompatibility::Older { ref actual, .. } if actual.to_string() == "1.0"
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn public_owner_evidence_contains_digest_not_raw_xml() {
        let root = temp_root("digest-evidence");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("Demo.xml");
        fs::write(
            &target,
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><Document/></MetaDataObject>"#
            ),
        )
        .unwrap();

        let resolution = resolve(&OwnerResolutionRequest {
            source: context(&root, &target),
            mode: OwnerResolutionMode::Existing,
        })
        .unwrap();

        assert!(resolution.evidence.iter().any(|evidence| matches!(
            evidence,
            SourceInputEvidence::ExactFileSha256 { sha256, .. } if sha256.len() == 64
        )));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn owner_resolution_rejects_symbolic_links() {
        let root = temp_root("symlink");
        fs::create_dir_all(&root).unwrap();
        let real = root.join("Real.xml");
        let target = root.join("Alias.xml");
        fs::write(
            &real,
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><Document/></MetaDataObject>"#
            ),
        )
        .unwrap();
        std::os::unix::fs::symlink(&real, &target).unwrap();

        let error = resolve_native(&context(&root, &target), None, false).unwrap_err();

        assert!(error.message.contains("symbolic link"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exact_versionless_roots_and_absent_outputs_have_no_owner() {
        let root = temp_root("exact-versionless");
        fs::create_dir_all(&root).unwrap();
        let cases = [
            (
                "Spreadsheet.xml",
                r#"<document xmlns="http://v8.1c.ru/8.2/data/spreadsheet"/>"#,
                ("http://v8.1c.ru/8.2/data/spreadsheet", "document"),
            ),
            (
                "CompositionSchema.xml",
                r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>"#,
                (
                    "http://v8.1c.ru/8.1/data-composition-system/schema",
                    "DataCompositionSchema",
                ),
            ),
        ];
        for (name, xml, expected) in cases {
            let target = root.join(name);
            fs::write(&target, xml).unwrap();
            assert!(
                resolve_native(&context(&root, &target), Some(expected), false)
                    .unwrap()
                    .owners
                    .is_empty()
            );
        }
        let missing = root.join("Missing.xml");
        assert!(resolve_native(
            &context(&root, &missing),
            Some(("http://v8.1c.ru/8.2/data/spreadsheet", "document")),
            false,
        )
        .unwrap()
        .owners
        .is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn standalone_graphical_schema_and_metadata_classification_are_closed() {
        let root = temp_root("standalone-closed");
        fs::create_dir_all(&root).unwrap();
        let graphical = root.join("Flowchart.xml");
        fs::write(
            &graphical,
            r#"<GraphicalSchema xmlns="http://v8.1c.ru/8.3/xcf/scheme" version="2.20"><Items/></GraphicalSchema>"#,
        )
        .unwrap();
        let resolution = resolve_native(&context(&root, &graphical), None, false).unwrap();
        assert_eq!(resolution.owners.len(), 1);
        assert_eq!(resolution.owners[0].kind, OwnerKind::Standalone);
        assert_eq!(resolution.owners[0].version.as_deref(), Some("2.20"));

        for (name, xml, expected) in [
            (
                "Empty.xml",
                format!(r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"/>"#),
                "exactly one artifact",
            ),
            (
                "Unknown.xml",
                format!(
                    r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><Garbage/></MetaDataObject>"#
                ),
                "unsupported",
            ),
        ] {
            let path = root.join(name);
            fs::write(&path, xml).unwrap();
            let error = resolve_native(&context(&root, &path), None, false).unwrap_err();
            assert!(error.message.contains(expected), "{error}");
        }

        let extension = root.join("Extension.xml");
        fs::write(
            &extension,
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><Configuration><Properties><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose></Properties></Configuration></MetaDataObject>"#
            ),
        )
        .unwrap();
        assert_eq!(
            resolve_native(&context(&root, &extension), None, false)
                .unwrap()
                .owners[0]
                .kind,
            OwnerKind::Extension
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn direct_root_existing_targets_and_versionless_children_preserve_owner_order() {
        let root = temp_root("owner-order");
        fs::create_dir_all(root.join("Ext")).unwrap();
        let configuration = root.join("Configuration.xml");
        fs::write(
            &configuration,
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><Configuration/></MetaDataObject>"#
            ),
        )
        .unwrap();
        let exact = root.join("Language.xml");
        fs::write(
            &exact,
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.21"><Language/></MetaDataObject>"#
            ),
        )
        .unwrap();
        let exact_owners = resolve_native(&context(&root, &exact), None, true)
            .unwrap()
            .owners;
        assert_eq!(
            exact_owners
                .iter()
                .map(|owner| owner.version.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("2.21"), Some("2.20")]
        );

        let interface = root.join("Ext/ClientApplicationInterface.xml");
        fs::write(
            &interface,
            r#"<ClientApplicationInterface xmlns="http://v8.1c.ru/8.2/managed-application/core"/>"#,
        )
        .unwrap();
        let inherited = resolve_native(&context(&root, &interface), None, true)
            .unwrap()
            .owners;
        assert_eq!(inherited.len(), 1);
        assert_eq!(inherited[0].path, fs::canonicalize(&configuration).unwrap());

        let direct = resolve_native(&context(&root, &root), None, true)
            .unwrap()
            .owners;
        assert_eq!(direct.len(), 1);
        assert_eq!(direct[0].path, fs::canonicalize(configuration).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn form_template_and_rights_content_resolve_only_their_exact_wrappers() {
        let root = temp_root("bounded-wrappers");
        fs::create_dir_all(&root).unwrap();
        let configuration = root.join("Configuration.xml");
        fs::write(
            &configuration,
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><Configuration/></MetaDataObject>"#
            ),
        )
        .unwrap();
        for (collection, item, content_name, artifact, content) in [
            (
                "Forms",
                "Main",
                "Form.xml",
                "Form",
                r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.22"/>"#,
            ),
            (
                "Templates",
                "Plan",
                "Template.xml",
                "Template",
                r#"<Template xmlns="http://v8.1c.ru/8.3/xcf/data"/>"#,
            ),
            (
                "Roles",
                "Reader",
                "Rights.xml",
                "Role",
                r#"<Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20"/>"#,
            ),
        ] {
            let wrapper = root.join(collection).join(format!("{item}.xml"));
            let content_path = root
                .join(collection)
                .join(item)
                .join("Ext")
                .join(content_name);
            fs::create_dir_all(content_path.parent().unwrap()).unwrap();
            fs::write(
                &wrapper,
                format!(
                    r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.21"><{artifact}/></MetaDataObject>"#
                ),
            )
            .unwrap();
            fs::write(&content_path, content).unwrap();
            let owners = resolve_native(&context(&root, &content_path), None, false)
                .unwrap()
                .owners;
            assert_eq!(
                owners.last().unwrap().path,
                fs::canonicalize(&configuration).unwrap()
            );
            assert!(owners
                .iter()
                .any(|owner| owner.path == fs::canonicalize(&wrapper).unwrap()));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nested_source_context_never_inherits_an_outer_owner() {
        let workspace = temp_root("nested-boundary");
        let source_root = workspace.join("src/new");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(
            workspace.join("src/Configuration.xml"),
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.21"><Configuration/></MetaDataObject>"#
            ),
        )
        .unwrap();
        let target = source_root.join("Missing.xml");
        let source = SourceContext::new(
            SourceLocation::new(workspace.clone(), source_root.clone(), target.clone()),
            Some("nested".to_string()),
            SourceFamily::PlatformXml,
            None,
        )
        .with_configured_source_set_kind(Some(ConfiguredSourceSetKind::Configuration));
        assert!(resolve_native(&source, None, true)
            .unwrap()
            .owners
            .is_empty());

        let nested = source_root.join("Configuration.xml");
        fs::write(
            &nested,
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><Configuration/></MetaDataObject>"#
            ),
        )
        .unwrap();
        let owners = resolve_native(&source, None, true).unwrap().owners;
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].path, fs::canonicalize(nested).unwrap());
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn external_root_classifies_every_entry_and_only_ignores_a_real_sidecar() {
        let root = temp_root("external-entry-matrix");
        fs::create_dir_all(&root).unwrap();
        for name in ["ConfigDumpInfo.xml", "Second.xml"] {
            fs::write(
                root.join(name),
                format!(
                    r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><ExternalDataProcessor/></MetaDataObject>"#
                ),
            )
            .unwrap();
        }
        let source = configured_context(&root, &root, ConfiguredSourceSetKind::ExternalProcessor);
        assert_eq!(
            resolve(&OwnerResolutionRequest {
                source: source.clone(),
                mode: OwnerResolutionMode::Existing,
            })
            .unwrap()
            .owners
            .len(),
            2
        );

        fs::write(root.join("ConfigDumpInfo.xml"), "<ConfigDumpInfo/>").unwrap();
        assert_eq!(
            resolve(&OwnerResolutionRequest {
                source: source.clone(),
                mode: OwnerResolutionMode::Existing,
            })
            .unwrap()
            .owners
            .len(),
            1
        );

        let large_size = 8 * 1024 * 1024 + 1;
        let mut sidecar = b"<ConfigDumpInfo/>".to_vec();
        sidecar.resize(large_size, b' ');
        fs::write(root.join("ConfigDumpInfo.xml"), sidecar).unwrap();
        let mut large_owner = format!(
            r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><ExternalDataProcessor/></MetaDataObject>"#
        )
        .into_bytes();
        large_owner.resize(large_size, b' ');
        fs::write(root.join("Large.xml"), large_owner).unwrap();
        assert_eq!(
            resolve(&OwnerResolutionRequest {
                source: source.clone(),
                mode: OwnerResolutionMode::Existing,
            })
            .unwrap()
            .owners
            .len(),
            2
        );
        fs::remove_file(root.join("Large.xml")).unwrap();

        for (name, xml, expected) in [
            (
                "Unknown.xml",
                format!(
                    r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><Garbage/></MetaDataObject>"#
                ),
                "unsupported",
            ),
            (
                "Malformed.xml",
                "<MetaDataObject".to_string(),
                "failed to parse",
            ),
        ] {
            let path = root.join(name);
            fs::write(&path, xml).unwrap();
            let error = resolve(&OwnerResolutionRequest {
                source: source.clone(),
                mode: OwnerResolutionMode::Existing,
            })
            .unwrap_err();
            assert!(error.message.contains(expected), "{error}");
            fs::remove_file(path).unwrap();
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn configuration_and_wrapper_candidates_reject_symbolic_links() {
        let root = temp_root("candidate-symlinks");
        fs::create_dir_all(root.join("Templates/Plan/Ext")).unwrap();
        let real_configuration = root.join("RealConfiguration.xml");
        fs::write(
            &real_configuration,
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><Configuration/></MetaDataObject>"#
            ),
        )
        .unwrap();
        std::os::unix::fs::symlink(&real_configuration, root.join("Configuration.xml")).unwrap();
        let target = root.join("Templates/Plan/Ext/Template.xml");
        fs::write(
            &target,
            r#"<Template xmlns="http://v8.1c.ru/8.3/xcf/data"/>"#,
        )
        .unwrap();
        let error = resolve_native(&context(&root, &target), None, false).unwrap_err();
        assert!(error.message.contains("symbolic link"), "{error}");
        fs::remove_file(root.join("Configuration.xml")).unwrap();

        fs::write(
            root.join("Configuration.xml"),
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><Configuration/></MetaDataObject>"#
            ),
        )
        .unwrap();
        let real_wrapper = root.join("RealTemplate.xml");
        fs::write(
            &real_wrapper,
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><Template/></MetaDataObject>"#
            ),
        )
        .unwrap();
        std::os::unix::fs::symlink(&real_wrapper, root.join("Templates/Plan.xml")).unwrap();
        let error = resolve_native(&context(&root, &target), None, false).unwrap_err();
        assert!(error.message.contains("symbolic link"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    fn context(root: &Path, target: &Path) -> SourceContext {
        SourceContext::new(
            SourceLocation::new(root.to_path_buf(), root.to_path_buf(), target.to_path_buf()),
            Some("main".to_string()),
            SourceFamily::PlatformXml,
            None,
        )
    }

    fn configured_context(
        root: &Path,
        target: &Path,
        kind: ConfiguredSourceSetKind,
    ) -> SourceContext {
        context(root, target).with_configured_source_set_kind(Some(kind))
    }

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "unica-platform-owner-{label}-{}",
            std::process::id()
        ))
    }
}
