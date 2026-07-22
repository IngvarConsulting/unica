use crate::domain::project_sources::SourceSetKind;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::project_sources::discover_project_source_map;
use crate::infrastructure::source_roots::{
    normalize_contained_source_root, normalize_path_identity,
};
use roxmltree::Document;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlatformXmlOwnerKind {
    Configuration,
    Extension,
    ExternalProcessor,
    ExternalReport,
}

impl PlatformXmlOwnerKind {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::Extension => "extension",
            Self::ExternalProcessor => "external_processor",
            Self::ExternalReport => "external_report",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PlatformXmlOwner {
    pub kind: PlatformXmlOwnerKind,
    pub path: PathBuf,
    pub version: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct PlatformXmlOwnerError {
    pub path: PathBuf,
    pub message: String,
}

pub(crate) fn resolve_platform_xml_owner(
    target: &Path,
    context: &WorkspaceContext,
) -> Result<Option<PlatformXmlOwner>, PlatformXmlOwnerError> {
    let target =
        absolute_normalized(target, &context.cwd).map_err(|message| PlatformXmlOwnerError {
            path: target.to_path_buf(),
            message,
        })?;
    let source_map = discover_project_source_map(&context.workspace_root).map_err(|message| {
        PlatformXmlOwnerError {
            path: context.workspace_root.clone(),
            message,
        }
    })?;

    let mut containing = Vec::new();
    for source_set in &source_map.source_sets {
        let source_root =
            normalize_contained_source_root(&context.workspace_root, &source_set.path).map_err(
                |message| PlatformXmlOwnerError {
                    path: context.workspace_root.join(&source_set.path),
                    message,
                },
            )?;
        if target.starts_with(&source_root) {
            containing.push((source_root, source_set.kind));
        }
    }
    containing.sort_by_key(|(root, _)| root.components().count());

    if let Some((source_root, kind)) = containing.pop() {
        let owner_path = if target == source_root
            && matches!(
                kind,
                SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
            ) {
            unique_external_owner(&source_root)?
        } else {
            owner_path_in_source_set(&source_root, &target, kind).ok_or_else(|| {
                PlatformXmlOwnerError {
                    path: source_root.clone(),
                    message: format!("cannot resolve platform XML owner for {}", target.display()),
                }
            })?
        };
        require_regular_owner(&owner_path)?;
        return read_platform_xml_owner(&owner_path, kind).map(Some);
    }

    // A standalone descriptor may be edited directly. Do not walk unrelated
    // ancestors: configured source-set boundaries are the ownership boundary.
    if target.is_file() && target.extension().and_then(|ext| ext.to_str()) == Some("xml") {
        return read_platform_xml_owner(&target, SourceSetKind::Configuration).map(Some);
    }
    Ok(None)
}

fn require_regular_owner(path: &Path) -> Result<(), PlatformXmlOwnerError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(PlatformXmlOwnerError {
            path: path.to_path_buf(),
            message: format!(
                "platform XML owner is not a regular file: {}",
                path.display()
            ),
        }),
        Err(error) => Err(PlatformXmlOwnerError {
            path: path.to_path_buf(),
            message: format!(
                "platform XML owner is unavailable {}: {error}",
                path.display()
            ),
        }),
    }
}

fn unique_external_owner(source_root: &Path) -> Result<PathBuf, PlatformXmlOwnerError> {
    let entries = fs::read_dir(source_root).map_err(|error| PlatformXmlOwnerError {
        path: source_root.to_path_buf(),
        message: format!(
            "failed to inspect external source set {}: {error}",
            source_root.display()
        ),
    })?;
    let mut candidates = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| PlatformXmlOwnerError {
            path: source_root.to_path_buf(),
            message: format!(
                "failed to inspect external source set {}: {error}",
                source_root.display()
            ),
        })?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("xml")
            || path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("ConfigDumpInfo.xml"))
        {
            continue;
        }
        candidates.push(path);
    }
    candidates.sort();
    match candidates.as_slice() {
        [owner] => Ok(owner.clone()),
        [] => Err(PlatformXmlOwnerError {
            path: source_root.to_path_buf(),
            message: format!(
                "external source set has no top-level artifact descriptor: {}",
                source_root.display()
            ),
        }),
        _ => Err(PlatformXmlOwnerError {
            path: source_root.to_path_buf(),
            message: format!(
                "external source set owner is ambiguous at {}: {} descriptors",
                source_root.display(),
                candidates.len()
            ),
        }),
    }
}

fn absolute_normalized(path: &Path, cwd: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    normalize_path_identity(&absolute)
}

fn owner_path_in_source_set(
    source_root: &Path,
    target: &Path,
    kind: SourceSetKind,
) -> Option<PathBuf> {
    match kind {
        SourceSetKind::Configuration | SourceSetKind::Extension => {
            Some(source_root.join("Configuration.xml"))
        }
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport => {
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
    }
}

fn read_platform_xml_owner(
    path: &Path,
    configured_kind: SourceSetKind,
) -> Result<PlatformXmlOwner, PlatformXmlOwnerError> {
    let text = fs::read_to_string(path).map_err(|error| PlatformXmlOwnerError {
        path: path.to_path_buf(),
        message: format!("failed to read {}: {error}", path.display()),
    })?;
    let document = Document::parse(text.trim_start_matches('\u{feff}')).map_err(|error| {
        PlatformXmlOwnerError {
            path: path.to_path_buf(),
            message: format!("failed to parse {}: {error}", path.display()),
        }
    })?;
    let is_extension = document
        .descendants()
        .any(|node| node.is_element() && node.tag_name().name() == "ConfigurationExtensionPurpose");
    let root = document.root_element();
    let has_external_processor = root
        .children()
        .any(|node| node.is_element() && node.tag_name().name() == "ExternalDataProcessor");
    let has_external_report = root
        .children()
        .any(|node| node.is_element() && node.tag_name().name() == "ExternalReport");
    let kind = match (has_external_processor, has_external_report, configured_kind) {
        (true, false, _) => PlatformXmlOwnerKind::ExternalProcessor,
        (false, true, _) => PlatformXmlOwnerKind::ExternalReport,
        (false, false, SourceSetKind::ExternalProcessor) => PlatformXmlOwnerKind::ExternalProcessor,
        (false, false, SourceSetKind::ExternalReport) => PlatformXmlOwnerKind::ExternalReport,
        (false, false, SourceSetKind::Extension) => PlatformXmlOwnerKind::Extension,
        (false, false, SourceSetKind::Configuration) if is_extension => {
            PlatformXmlOwnerKind::Extension
        }
        (false, false, SourceSetKind::Configuration) => PlatformXmlOwnerKind::Configuration,
        (true, true, _) => {
            return Err(PlatformXmlOwnerError {
                path: path.to_path_buf(),
                message: format!(
                    "platform XML owner has conflicting external artifact kinds: {}",
                    path.display()
                ),
            });
        }
    };
    Ok(PlatformXmlOwner {
        kind,
        path: path.to_path_buf(),
        version: root.attribute("version").map(str::to_owned),
    })
}
