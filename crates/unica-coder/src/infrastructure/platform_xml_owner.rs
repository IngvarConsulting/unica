use crate::domain::project_sources::{
    classify_already_read_config_dump_info_xml, ConfigDumpInfoXmlKind, SourceSetKind,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::metadata_kinds::METADATA_KIND_TAGS;
use crate::infrastructure::native_operations::compile_transaction::{
    snapshot_directory_membership, CompileTransaction, DirectoryMembershipSelector,
};
use crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point;
use crate::infrastructure::project_sources::{
    discover_project_source_map_with_provenance, ProjectSourceMapProvenance,
};
use crate::infrastructure::source_roots::{
    normalize_contained_source_root, normalize_path_identity,
    select_unique_deepest_source_set_match,
};
use roxmltree::Document;
use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlatformXmlOwnerKind {
    Configuration,
    Extension,
    ExternalProcessor,
    ExternalReport,
    Standalone,
}

impl PlatformXmlOwnerKind {
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
pub(crate) struct PlatformXmlOwner {
    pub kind: PlatformXmlOwnerKind,
    pub path: PathBuf,
    pub version: Option<String>,
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct PlatformXmlOwnerError {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PlatformXmlOwnerCandidateInput {
    ExactFile(Vec<u8>),
    Absent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlatformXmlOwnerProvenance {
    source_map: ProjectSourceMapProvenance,
    candidates: BTreeMap<PathBuf, PlatformXmlOwnerCandidateInput>,
    directory_memberships: BTreeMap<PathBuf, Vec<OsString>>,
}

impl PlatformXmlOwnerProvenance {
    pub(crate) fn bind_to(&self, transaction: &mut CompileTransaction) -> Result<(), String> {
        self.source_map.bind_to(transaction)?;
        for (path, input) in &self.candidates {
            match input {
                PlatformXmlOwnerCandidateInput::ExactFile(raw) => {
                    if !transaction.protects_path(path)? {
                        transaction.guard_or_verify_exact_preimage(path, raw)?;
                    }
                }
                PlatformXmlOwnerCandidateInput::Absent => {
                    if !transaction.protects_path(path)? {
                        transaction.guard_path_absent(path)?;
                    }
                }
            }
        }
        for (directory, expected_names) in &self.directory_memberships {
            transaction.guard_or_verify_directory_membership(
                directory,
                DirectoryMembershipSelector::XmlFiles,
                expected_names.clone(),
            )?;
        }
        Ok(())
    }

    fn record_exact(
        &mut self,
        path: PathBuf,
        raw: impl Into<Vec<u8>>,
    ) -> Result<(), PlatformXmlOwnerError> {
        let raw = raw.into();
        match self.candidates.get(&path) {
            Some(PlatformXmlOwnerCandidateInput::ExactFile(existing)) if existing == &raw => Ok(()),
            Some(_) => Err(changed_during_resolution(&path)),
            None => {
                self.candidates
                    .insert(path, PlatformXmlOwnerCandidateInput::ExactFile(raw));
                Ok(())
            }
        }
    }

    fn record_absence(&mut self, path: PathBuf) -> Result<(), PlatformXmlOwnerError> {
        match self.candidates.get(&path) {
            Some(PlatformXmlOwnerCandidateInput::Absent) => Ok(()),
            Some(_) => Err(changed_during_resolution(&path)),
            None => {
                self.candidates
                    .insert(path, PlatformXmlOwnerCandidateInput::Absent);
                Ok(())
            }
        }
    }

    fn record_directory_membership(
        &mut self,
        directory: PathBuf,
        expected_names: Vec<OsString>,
    ) -> Result<(), PlatformXmlOwnerError> {
        match self.directory_memberships.get(&directory) {
            Some(existing) if existing == &expected_names => Ok(()),
            Some(_) => Err(changed_during_resolution(&directory)),
            None => {
                self.directory_memberships.insert(directory, expected_names);
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PlatformXmlOwnerResolution {
    pub owners: Vec<PlatformXmlOwner>,
    pub provenance: PlatformXmlOwnerProvenance,
}

#[derive(Debug, Clone, Copy)]
enum OwnerExpectation {
    SourceSet(SourceSetKind),
    Standalone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlatformXmlRootExpectation {
    pub(crate) namespace: &'static str,
    pub(crate) local_name: &'static str,
}

impl PlatformXmlRootExpectation {
    pub(crate) const fn new(namespace: &'static str, local_name: &'static str) -> Self {
        Self {
            namespace,
            local_name,
        }
    }
}

pub(crate) const MANAGED_FORM_ROOT: PlatformXmlRootExpectation =
    PlatformXmlRootExpectation::new("http://v8.1c.ru/8.3/xcf/logform", "Form");
pub(crate) const DCS_ROOT: PlatformXmlRootExpectation = PlatformXmlRootExpectation::new(
    "http://v8.1c.ru/8.1/data-composition-system/schema",
    "DataCompositionSchema",
);
pub(crate) const MXL_ROOT: PlatformXmlRootExpectation =
    PlatformXmlRootExpectation::new("http://v8.1c.ru/8.2/data/spreadsheet", "document");

const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

pub(crate) fn root_version_literal(source: &str, root: roxmltree::Node<'_, '_>) -> Option<String> {
    root.attributes()
        .find(|attribute| attribute.namespace().is_none() && attribute.name() == "version")
        .and_then(|attribute| source.get(attribute.range_value()))
        .map(str::to_owned)
}

pub(crate) fn resolve_platform_xml_owners(
    target: &Path,
    context: &WorkspaceContext,
) -> Result<Vec<PlatformXmlOwner>, PlatformXmlOwnerError> {
    resolve_platform_xml_owners_with_provenance(target, context).map(|resolution| resolution.owners)
}

pub(crate) fn resolve_platform_xml_owners_for_exact_root(
    target: &Path,
    context: &WorkspaceContext,
    expected_root: PlatformXmlRootExpectation,
) -> Result<Vec<PlatformXmlOwner>, PlatformXmlOwnerError> {
    resolve_platform_xml_owners_for_exact_root_with_provenance(target, context, expected_root)
        .map(|resolution| resolution.owners)
}

pub(crate) fn resolve_platform_xml_owners_with_provenance(
    target: &Path,
    context: &WorkspaceContext,
) -> Result<PlatformXmlOwnerResolution, PlatformXmlOwnerError> {
    resolve_platform_xml_owners_with_optional_exact_root(target, context, None)
}

pub(crate) fn resolve_platform_xml_owners_for_exact_root_with_provenance(
    target: &Path,
    context: &WorkspaceContext,
    expected_root: PlatformXmlRootExpectation,
) -> Result<PlatformXmlOwnerResolution, PlatformXmlOwnerError> {
    resolve_platform_xml_owners_with_optional_exact_root(target, context, Some(expected_root))
}

fn resolve_platform_xml_owners_with_optional_exact_root(
    target: &Path,
    context: &WorkspaceContext,
    expected_root: Option<PlatformXmlRootExpectation>,
) -> Result<PlatformXmlOwnerResolution, PlatformXmlOwnerError> {
    let absolute_target = if target.is_absolute() {
        target.to_path_buf()
    } else {
        context.cwd.join(target)
    };
    if let Ok(metadata) = fs::symlink_metadata(&absolute_target) {
        if metadata_is_link_or_reparse_point(&metadata) {
            return Err(link_owner_error(&absolute_target));
        }
    }
    let target =
        absolute_normalized(target, &context.cwd).map_err(|message| PlatformXmlOwnerError {
            path: target.to_path_buf(),
            message,
        })?;
    let (source_map, source_map_provenance) = discover_project_source_map_with_provenance(
        &context.workspace_root,
    )
    .map_err(|message| PlatformXmlOwnerError {
        path: context.workspace_root.clone(),
        message,
    })?;
    let mut provenance = PlatformXmlOwnerProvenance {
        source_map: source_map_provenance,
        candidates: BTreeMap::new(),
        directory_memberships: BTreeMap::new(),
    };

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
            containing.push((source_set, source_root));
        }
    }
    let containing =
        select_unique_deepest_source_set_match(&target, containing).map_err(|message| {
            PlatformXmlOwnerError {
                path: target.clone(),
                message,
            }
        })?;

    if let Some((source_set, source_root)) = containing {
        let kind = source_set.kind;
        if target == source_root
            && matches!(
                kind,
                SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
            )
        {
            let owners = read_external_source_set_owners(&source_root, kind, &mut provenance)?;
            if owners.is_empty() {
                return Err(PlatformXmlOwnerError {
                    path: source_root.clone(),
                    message: format!(
                        "external source set has no top-level artifact descriptor: {}",
                        source_root.display()
                    ),
                });
            }
            return Ok(PlatformXmlOwnerResolution { owners, provenance });
        }
        let owner_path =
            owner_path_in_source_set(&source_root, &target, kind).ok_or_else(|| {
                PlatformXmlOwnerError {
                    path: source_root.clone(),
                    message: format!("cannot resolve platform XML owner for {}", target.display()),
                }
            })?;
        let source_set_owner = read_required_platform_xml_owner(
            &owner_path,
            OwnerExpectation::SourceSet(kind),
            &mut provenance,
        )?;
        let mut owners = if target == owner_path && expected_root.is_none() {
            Vec::new()
        } else {
            read_bounded_target_version_owners(&target, &mut provenance, expected_root)?
        };
        if !owners
            .iter()
            .any(|owner| owner.path == source_set_owner.path)
        {
            owners.push(source_set_owner);
        }
        return Ok(PlatformXmlOwnerResolution { owners, provenance });
    }

    // Several native handlers accept an explicit configuration root directory
    // outside a configured project source set. Treat only its direct
    // Configuration.xml child as the owner; do not search arbitrary ancestors.
    if target.is_dir() {
        let owner_path = target.join("Configuration.xml");
        if let Some(owner) = read_optional_platform_xml_owner(
            &owner_path,
            OwnerExpectation::SourceSet(SourceSetKind::Configuration),
            &mut provenance,
        )? {
            return Ok(PlatformXmlOwnerResolution {
                owners: vec![owner],
                provenance,
            });
        }
    }

    // A standalone descriptor may be edited directly. Do not walk unrelated
    // ancestors: configured source-set boundaries are the ownership boundary.
    let owners = read_bounded_target_version_owners(&target, &mut provenance, expected_root)?;
    if !owners.is_empty() {
        return Ok(PlatformXmlOwnerResolution { owners, provenance });
    }
    Ok(PlatformXmlOwnerResolution {
        owners: Vec::new(),
        provenance,
    })
}

pub(crate) fn resolve_existing_platform_xml_owners_for_new_output(
    target: &Path,
    context: &WorkspaceContext,
) -> Result<Vec<PlatformXmlOwner>, PlatformXmlOwnerError> {
    resolve_existing_platform_xml_owners_for_new_output_with_provenance(target, context)
        .map(|resolution| resolution.owners)
}

pub(crate) fn resolve_existing_platform_xml_owners_for_new_output_with_provenance(
    target: &Path,
    context: &WorkspaceContext,
) -> Result<PlatformXmlOwnerResolution, PlatformXmlOwnerError> {
    resolve_existing_platform_xml_owners_for_new_output_with_optional_exact_root(
        target, context, None,
    )
}

fn resolve_existing_platform_xml_owners_for_new_output_with_optional_exact_root(
    target: &Path,
    context: &WorkspaceContext,
    expected_root: Option<PlatformXmlRootExpectation>,
) -> Result<PlatformXmlOwnerResolution, PlatformXmlOwnerError> {
    let absolute_target = if target.is_absolute() {
        target.to_path_buf()
    } else {
        context.cwd.join(target)
    };
    if let Ok(metadata) = fs::symlink_metadata(&absolute_target) {
        if metadata_is_link_or_reparse_point(&metadata) {
            return Err(link_owner_error(&absolute_target));
        }
    }
    let target =
        absolute_normalized(target, &context.cwd).map_err(|message| PlatformXmlOwnerError {
            path: target.to_path_buf(),
            message,
        })?;
    let (source_map, source_map_provenance) = discover_project_source_map_with_provenance(
        &context.workspace_root,
    )
    .map_err(|message| PlatformXmlOwnerError {
        path: context.workspace_root.clone(),
        message,
    })?;
    let mut provenance = PlatformXmlOwnerProvenance {
        source_map: source_map_provenance,
        candidates: BTreeMap::new(),
        directory_memberships: BTreeMap::new(),
    };
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
            containing.push((source_set, source_root));
        }
    }
    let containing =
        select_unique_deepest_source_set_match(&target, containing).map_err(|message| {
            PlatformXmlOwnerError {
                path: target.clone(),
                message,
            }
        })?;

    let mut owners = read_bounded_target_version_owners(&target, &mut provenance, expected_root)?;
    let mut seen = HashSet::new();
    for owner in &owners {
        seen.insert(owner.path.clone());
    }
    if target.is_dir() {
        let owner_path = target.join("Configuration.xml");
        if let Some(owner) = read_optional_platform_xml_owner(
            &owner_path,
            OwnerExpectation::SourceSet(SourceSetKind::Configuration),
            &mut provenance,
        )? {
            seen.insert(owner.path.clone());
            owners.push(owner);
        }
    }
    if let Some((source_set, source_root)) = containing {
        let kind = source_set.kind;
        if target == source_root
            && matches!(
                kind,
                SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
            )
        {
            for owner in read_external_source_set_owners(&source_root, kind, &mut provenance)? {
                if seen.insert(owner.path.clone()) {
                    owners.push(owner);
                }
            }
            return Ok(PlatformXmlOwnerResolution { owners, provenance });
        }
        let owner_path = {
            let Some(owner) = owner_path_in_source_set(&source_root, &target, kind) else {
                return Ok(PlatformXmlOwnerResolution { owners, provenance });
            };
            owner
        };
        if let Some(owner) = read_optional_platform_xml_owner(
            &owner_path,
            OwnerExpectation::SourceSet(kind),
            &mut provenance,
        )? {
            if seen.insert(owner.path.clone()) {
                owners.push(owner);
            }
        }
    }
    Ok(PlatformXmlOwnerResolution { owners, provenance })
}

fn read_bounded_target_version_owners(
    target: &Path,
    provenance: &mut PlatformXmlOwnerProvenance,
    expected_root: Option<PlatformXmlRootExpectation>,
) -> Result<Vec<PlatformXmlOwner>, PlatformXmlOwnerError> {
    let mut candidates = vec![target.to_path_buf()];
    if let Some(wrapper) = metadata_wrapper_for_content_path(target) {
        candidates.push(wrapper);
    }
    let mut owners = Vec::new();
    let mut seen = HashSet::new();
    for (index, candidate) in candidates.into_iter().enumerate() {
        let candidate_expected_root = (index == 0).then_some(expected_root).flatten();
        if let Some(owner) =
            read_version_owning_target(&candidate, provenance, candidate_expected_root)?
        {
            if seen.insert(owner.path.clone()) {
                owners.push(owner);
            }
        }
    }
    Ok(owners)
}

fn metadata_wrapper_for_content_path(target: &Path) -> Option<PathBuf> {
    let content_name = target.file_name()?.to_str()?;
    let expected_collection = match content_name {
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

fn read_version_owning_target(
    path: &Path,
    provenance: &mut PlatformXmlOwnerProvenance,
    expected_root: Option<PlatformXmlRootExpectation>,
) -> Result<Option<PlatformXmlOwner>, PlatformXmlOwnerError> {
    let Some((path, raw)) = snapshot_candidate_file(path, provenance, false)? else {
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
    let text = std::str::from_utf8(&raw).map_err(|error| PlatformXmlOwnerError {
        path: path.clone(),
        message: format!("failed to read {} as UTF-8: {error}", path.display()),
    })?;
    let source = text.trim_start_matches('\u{feff}');
    let document = Document::parse(source).map_err(|error| PlatformXmlOwnerError {
        path: path.clone(),
        message: format!("failed to parse {}: {error}", path.display()),
    })?;
    let root = document.root_element();
    let root_qname = (root.tag_name().namespace(), root.tag_name().name());
    if let Some(expected_root) = expected_root {
        if root_qname != (Some(expected_root.namespace), expected_root.local_name) {
            return invalid_owner(
                &path,
                &format!(
                    "declared platform XML target root is {{{}}}{}, expected {{{}}}{}",
                    root_qname.0.unwrap_or(""),
                    root_qname.1,
                    expected_root.namespace,
                    expected_root.local_name
                ),
            );
        }
    }
    let is_supported_version_root =
        root_qname == (Some(MD_CLASSES_NS), "MetaDataObject") || known_standalone_root(root_qname);
    if root_version_literal(source, root).is_none()
        && (!is_supported_version_root || version_is_inherited_when_missing(root_qname))
    {
        return Ok(None);
    }
    if !is_supported_version_root {
        return invalid_owner(
            &path,
            &format!(
                "unsupported version-owning platform XML root {{{}}}{}",
                root_qname.0.unwrap_or(""),
                root_qname.1
            ),
        );
    }
    parse_platform_xml_owner(&path, raw, OwnerExpectation::Standalone).map(Some)
}

fn snapshot_candidate_file(
    path: &Path,
    provenance: &mut PlatformXmlOwnerProvenance,
    require_file_if_present: bool,
) -> Result<Option<(PathBuf, Vec<u8>)>, PlatformXmlOwnerError> {
    let candidate = path.to_path_buf();
    let metadata = match fs::symlink_metadata(&candidate) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let path =
                normalize_path_identity(&candidate).map_err(|message| PlatformXmlOwnerError {
                    path: candidate.clone(),
                    message,
                })?;
            provenance.record_absence(path)?;
            return Ok(None);
        }
        Err(error) => {
            return Err(PlatformXmlOwnerError {
                path: candidate.clone(),
                message: format!("failed to inspect {}: {error}", candidate.display()),
            });
        }
    };
    if metadata_is_link_or_reparse_point(&metadata) {
        return Err(link_owner_error(&candidate));
    }
    if !metadata.is_file() {
        if require_file_if_present {
            return Err(PlatformXmlOwnerError {
                path: candidate.clone(),
                message: format!(
                    "platform XML owner is not a regular file: {}",
                    candidate.display()
                ),
            });
        }
        return Ok(None);
    }
    let raw = fs::read(&candidate).map_err(|error| PlatformXmlOwnerError {
        path: candidate.clone(),
        message: format!("failed to read {}: {error}", candidate.display()),
    })?;
    let path = normalize_path_identity(&candidate).map_err(|message| PlatformXmlOwnerError {
        path: candidate,
        message,
    })?;
    provenance.record_exact(path.clone(), raw.clone())?;
    Ok(Some((path, raw)))
}

fn read_optional_platform_xml_owner(
    path: &Path,
    expectation: OwnerExpectation,
    provenance: &mut PlatformXmlOwnerProvenance,
) -> Result<Option<PlatformXmlOwner>, PlatformXmlOwnerError> {
    let Some((path, raw)) = snapshot_candidate_file(path, provenance, true)? else {
        return Ok(None);
    };
    parse_platform_xml_owner(&path, raw, expectation).map(Some)
}

fn read_required_platform_xml_owner(
    path: &Path,
    expectation: OwnerExpectation,
    provenance: &mut PlatformXmlOwnerProvenance,
) -> Result<PlatformXmlOwner, PlatformXmlOwnerError> {
    read_optional_platform_xml_owner(path, expectation, provenance)?.ok_or_else(|| {
        PlatformXmlOwnerError {
            path: path.to_path_buf(),
            message: format!("platform XML owner is unavailable {}", path.display()),
        }
    })
}

#[cfg(test)]
fn require_regular_owner(path: &Path) -> Result<(), PlatformXmlOwnerError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata_is_link_or_reparse_point(&metadata) => Err(link_owner_error(path)),
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

fn link_owner_error(path: &Path) -> PlatformXmlOwnerError {
    PlatformXmlOwnerError {
        path: path.to_path_buf(),
        message: format!(
            "platform XML owner must not be a symbolic link or reparse point: {}",
            path.display()
        ),
    }
}

fn changed_during_resolution(path: &Path) -> PlatformXmlOwnerError {
    PlatformXmlOwnerError {
        path: path.to_path_buf(),
        message: format!(
            "platform XML owner candidate changed while resolving: {}",
            path.display()
        ),
    }
}

fn read_external_source_set_owners(
    source_root: &Path,
    kind: SourceSetKind,
    provenance: &mut PlatformXmlOwnerProvenance,
) -> Result<Vec<PlatformXmlOwner>, PlatformXmlOwnerError> {
    let membership =
        snapshot_directory_membership(source_root, DirectoryMembershipSelector::XmlFiles).map_err(
            |message| PlatformXmlOwnerError {
                path: source_root.to_path_buf(),
                message,
            },
        )?;
    provenance.record_directory_membership(source_root.to_path_buf(), membership)?;
    let entries = match fs::read_dir(source_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(PlatformXmlOwnerError {
                path: source_root.to_path_buf(),
                message: format!(
                    "failed to inspect external source set {}: {error}",
                    source_root.display()
                ),
            });
        }
    };
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
        if path.extension().and_then(|ext| ext.to_str()) != Some("xml") {
            continue;
        }
        candidates.push(path);
    }
    candidates.sort();

    let mut owners = Vec::new();
    for candidate in candidates {
        let Some((path, raw)) = snapshot_candidate_file(&candidate, provenance, true)? else {
            return Err(changed_during_resolution(&candidate));
        };
        let is_reserved_sidecar = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("ConfigDumpInfo.xml"))
            && classify_already_read_config_dump_info_xml(&raw)
                == ConfigDumpInfoXmlKind::RuntimeSidecar;
        if is_reserved_sidecar {
            continue;
        }
        owners.push(parse_platform_xml_owner(
            &path,
            raw,
            OwnerExpectation::SourceSet(kind),
        )?);
    }
    Ok(owners)
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

fn parse_platform_xml_owner(
    path: &Path,
    raw: Vec<u8>,
    expectation: OwnerExpectation,
) -> Result<PlatformXmlOwner, PlatformXmlOwnerError> {
    let text = std::str::from_utf8(&raw).map_err(|error| PlatformXmlOwnerError {
        path: path.to_path_buf(),
        message: format!("failed to read {} as UTF-8: {error}", path.display()),
    })?;
    let source = text.trim_start_matches('\u{feff}');
    let document = Document::parse(source).map_err(|error| PlatformXmlOwnerError {
        path: path.to_path_buf(),
        message: format!("failed to parse {}: {error}", path.display()),
    })?;
    let root = document.root_element();
    let root_qname = (root.tag_name().namespace(), root.tag_name().name());
    let artifact_children = root
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    let direct_artifact = match artifact_children.as_slice() {
        [artifact] => Some(*artifact),
        _ => None,
    };
    let is_extension = direct_artifact.is_some_and(is_configuration_extension_artifact);
    let kind = match expectation {
        OwnerExpectation::SourceSet(configured_kind) => {
            validate_source_set_owner(root, configured_kind, path)?;
            match configured_kind {
                SourceSetKind::ExternalProcessor => PlatformXmlOwnerKind::ExternalProcessor,
                SourceSetKind::ExternalReport => PlatformXmlOwnerKind::ExternalReport,
                SourceSetKind::Extension => PlatformXmlOwnerKind::Extension,
                SourceSetKind::Configuration if is_extension => PlatformXmlOwnerKind::Extension,
                SourceSetKind::Configuration => PlatformXmlOwnerKind::Configuration,
            }
        }
        OwnerExpectation::Standalone => {
            if root_qname == (Some(MD_CLASSES_NS), "MetaDataObject") {
                let Some(artifact) = direct_artifact else {
                    return invalid_owner(
                        path,
                        &format!(
                            "standalone metadata descriptor must contain exactly one direct {{{MD_CLASSES_NS}}} artifact child"
                        ),
                    );
                };
                if artifact.tag_name().namespace() != Some(MD_CLASSES_NS) {
                    return invalid_owner(
                        path,
                        &format!(
                            "standalone metadata descriptor direct artifact child must use namespace {{{MD_CLASSES_NS}}}"
                        ),
                    );
                }
                if !is_supported_metadata_artifact(artifact.tag_name().name()) {
                    return invalid_owner(
                        path,
                        &format!(
                            "unsupported 8.3.27 metadata artifact family {{{MD_CLASSES_NS}}}{}",
                            artifact.tag_name().name()
                        ),
                    );
                }
                match artifact.tag_name().name() {
                    "ExternalDataProcessor" => PlatformXmlOwnerKind::ExternalProcessor,
                    "ExternalReport" => PlatformXmlOwnerKind::ExternalReport,
                    "Configuration" if is_extension => PlatformXmlOwnerKind::Extension,
                    _ => PlatformXmlOwnerKind::Standalone,
                }
            } else if known_standalone_root(root_qname) {
                PlatformXmlOwnerKind::Standalone
            } else {
                return invalid_owner(
                    path,
                    &format!(
                        "unsupported standalone platform XML root {{{}}}{}",
                        root_qname.0.unwrap_or(""),
                        root_qname.1
                    ),
                );
            }
        }
    };
    Ok(PlatformXmlOwner {
        kind,
        path: path.to_path_buf(),
        version: root_version_literal(source, root),
        raw,
    })
}

fn validate_source_set_owner(
    root: roxmltree::Node<'_, '_>,
    configured_kind: SourceSetKind,
    path: &Path,
) -> Result<(), PlatformXmlOwnerError> {
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        return invalid_owner(
            path,
            "source-set owner root must be {http://v8.1c.ru/8.3/MDClasses}MetaDataObject",
        );
    }
    let expected_child = match configured_kind {
        SourceSetKind::Configuration | SourceSetKind::Extension => "Configuration",
        SourceSetKind::ExternalProcessor => "ExternalDataProcessor",
        SourceSetKind::ExternalReport => "ExternalReport",
    };
    let artifact_children = root
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    if artifact_children.len() != 1
        || artifact_children[0].tag_name().namespace() != Some(MD_CLASSES_NS)
        || artifact_children[0].tag_name().name() != expected_child
    {
        return invalid_owner(
            path,
            &format!(
                "source-set owner must contain exactly one direct {{{MD_CLASSES_NS}}}{expected_child} artifact child"
            ),
        );
    }
    Ok(())
}

fn is_configuration_extension_artifact(artifact: roxmltree::Node<'_, '_>) -> bool {
    artifact.tag_name().namespace() == Some(MD_CLASSES_NS)
        && artifact.tag_name().name() == "Configuration"
        && artifact
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
    METADATA_KIND_TAGS.contains(&tag)
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

fn invalid_owner<T>(path: &Path, reason: &str) -> Result<T, PlatformXmlOwnerError> {
    Err(PlatformXmlOwnerError {
        path: path.to_path_buf(),
        message: format!("invalid platform XML owner {}: {reason}", path.display()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::platform::testing::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_context(name: &str) -> WorkspaceContext {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must follow epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "unica-platform-xml-owner-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("temporary workspace must be created");
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    fn normalized_path(path: &Path) -> PathBuf {
        normalize_path_identity(path).expect("test path identity must normalize")
    }

    fn create_file_link_or_skip(source: &Path, target: &Path) -> bool {
        match create_file_link_fixture_for_test(source, target)
            .expect("unexpected file-link creation error must fail the test")
        {
            FileLinkFixtureOutcome::Created => true,
            FileLinkFixtureOutcome::Unsupported => {
                eprintln!("[SKIPPED FIXTURE] file links are unsupported on this host");
                false
            }
            FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                eprintln!("[SKIPPED FIXTURE] Windows file-link privilege is unavailable");
                false
            }
        }
    }

    #[test]
    fn exact_declared_target_rejects_a_source_set_owner_with_the_wrong_root() {
        let context = temp_context("exact-target-source-owner");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let target = context.cwd.join("src/Configuration.xml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(
            &target,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();

        let error = resolve_platform_xml_owners_for_exact_root(&target, &context, MXL_ROOT)
            .expect_err("a declared MXL output must validate its own root");

        assert!(
            error.message.contains("declared platform XML target root"),
            "{}",
            error.message
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn exact_declared_versionless_roots_and_absent_outputs_have_no_owner() {
        let context = temp_context("exact-versionless-or-absent");
        let cases = [
            (
                "Spreadsheet.XML",
                r#"<document xmlns="http://v8.1c.ru/8.2/data/spreadsheet"/>"#,
                MXL_ROOT,
            ),
            (
                "CompositionSchema",
                r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>"#,
                DCS_ROOT,
            ),
        ];

        for (name, xml, expected_root) in cases {
            let target = context.cwd.join(name);
            fs::write(&target, xml).unwrap();

            let owners =
                resolve_platform_xml_owners_for_exact_root(&target, &context, expected_root)
                    .expect("a correct versionless declared target must be accepted");

            assert!(owners.is_empty(), "{name}: {owners:?}");
        }

        let missing = context.cwd.join("MissingOutput");
        let owners = resolve_platform_xml_owners_for_exact_root(&missing, &context, MXL_ROOT)
            .expect("an absent declared output must be accepted");
        assert!(owners.is_empty(), "{owners:?}");

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn graphical_schema_is_a_version_owning_standalone_root() {
        let context = temp_context("graphical-schema-owner");
        let path = context.cwd.join("Flowchart.xml");
        fs::write(
            &path,
            br#"<?xml version="1.0" encoding="UTF-8"?>
<GraphicalSchema xmlns="http://v8.1c.ru/8.3/xcf/scheme" version="2.20"><Items/></GraphicalSchema>
"#,
        )
        .unwrap();

        let owners = resolve_platform_xml_owners(&path, &context).unwrap();

        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].path, normalized_path(&path));
        assert_eq!(owners[0].version.as_deref(), Some("2.20"));
        assert_eq!(owners[0].kind, PlatformXmlOwnerKind::Standalone);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn standalone_metadata_owner_requires_exactly_one_direct_artifact_child() {
        let cases = [
            (
                "empty",
                br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"/>"#
                    .as_slice(),
            ),
            (
                "multiple",
                br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog/><Document/></MetaDataObject>"#
                    .as_slice(),
            ),
        ];

        for (label, xml) in cases {
            let context = temp_context(&format!("standalone-{label}-artifact"));
            let path = context.cwd.join("Object.xml");
            fs::write(&path, xml).unwrap();

            let error = resolve_platform_xml_owners(&path, &context)
                .expect_err("standalone metadata owner cardinality must fail closed");

            assert!(
                error.message.contains("exactly one direct"),
                "{label}: {error:?}"
            );
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn standalone_extension_kind_requires_direct_configuration_properties_marker() {
        let cases = [
            (
                "catalog-descendant",
                br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose></Properties></Catalog></MetaDataObject>"#
                    .as_slice(),
                PlatformXmlOwnerKind::Standalone,
            ),
            (
                "configuration-wrong-branch",
                br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><ChildObjects><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose></ChildObjects></Configuration></MetaDataObject>"#
                    .as_slice(),
                PlatformXmlOwnerKind::Standalone,
            ),
            (
                "extension",
                br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose></Properties></Configuration></MetaDataObject>"#
                    .as_slice(),
                PlatformXmlOwnerKind::Extension,
            ),
        ];

        for (label, xml, expected_kind) in cases {
            let context = temp_context(&format!("standalone-kind-{label}"));
            let path = context.cwd.join("Object.xml");
            fs::write(&path, xml).unwrap();

            let owners = resolve_platform_xml_owners(&path, &context).unwrap();

            assert_eq!(owners.len(), 1, "{label}");
            assert_eq!(owners[0].kind, expected_kind, "{label}");
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn standalone_metadata_owner_rejects_unknown_artifact_family() {
        let context = temp_context("standalone-unknown-artifact");
        let path = context.cwd.join("Object.xml");
        fs::write(
            &path,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Garbage/></MetaDataObject>"#,
        )
        .unwrap();

        let error = resolve_platform_xml_owners(&path, &context)
            .expect_err("unknown MDClasses artifact family must fail closed");

        assert!(error.message.contains("unsupported"), "{error:?}");
        assert!(error.message.contains("Garbage"), "{error:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn require_regular_owner_rejects_symlink_without_following_target() {
        let context = temp_context("direct-symlink");
        let real = context.cwd.join("real.xml");
        let owner = context.cwd.join("Configuration.xml");
        let original = b"<MetaDataObject/>";
        fs::write(&real, original).unwrap();
        if !create_file_link_or_skip(&real, &owner) {
            let _ = fs::remove_dir_all(&context.cwd);
            return;
        }

        let error = require_regular_owner(&owner)
            .expect_err("a platform XML owner link must never be followed");

        assert!(
            error.message.contains("symbolic link") || error.message.contains("reparse point"),
            "{}",
            error.message
        );
        assert_eq!(fs::read(&real).unwrap(), original);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn standalone_owner_resolution_rejects_symlink_without_following_target() {
        let context = temp_context("standalone-symlink");
        let real = context.cwd.join("real-form.xml");
        let owner = context.cwd.join("Form.xml");
        let original = br#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#;
        fs::write(&real, original).unwrap();
        if !create_file_link_or_skip(&real, &owner) {
            let _ = fs::remove_dir_all(&context.cwd);
            return;
        }

        let error = resolve_platform_xml_owners(&owner, &context)
            .expect_err("standalone owner links must never be followed");

        assert!(
            error.message.contains("symbolic link") || error.message.contains("reparse point"),
            "{}",
            error.message
        );
        assert_eq!(fs::read(&real).unwrap(), original);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn source_set_owner_resolution_rejects_symlink_without_following_target() {
        let context = temp_context("source-owner-symlink");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let real = context.cwd.join("real-configuration.xml");
        let owner = context.cwd.join("src/Configuration.xml");
        let target = context.cwd.join("src/Catalogs/Goods/Ext/ObjectModule.bsl");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(
            &real,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        if !create_file_link_or_skip(&real, &owner) {
            let _ = fs::remove_dir_all(&context.cwd);
            return;
        }

        let error = resolve_platform_xml_owners(&target, &context)
            .expect_err("source-set owner links must never be followed");

        assert!(
            error.message.contains("symbolic link") || error.message.contains("reparse point"),
            "{}",
            error.message
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn metadata_wrapper_resolution_rejects_symlink_without_following_target() {
        let context = temp_context("wrapper-symlink");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let configuration = context.cwd.join("src/Configuration.xml");
        let real_wrapper = context.cwd.join("real-template-wrapper.xml");
        let wrapper = context.cwd.join("src/Reports/Sales/Templates/Planned.xml");
        let content = context
            .cwd
            .join("src/Reports/Sales/Templates/Planned/Ext/Template.xml");
        fs::create_dir_all(content.parent().unwrap()).unwrap();
        fs::write(
            &configuration,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            &real_wrapper,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            &content,
            br#"<Template xmlns="http://v8.1c.ru/8.3/xcf/data"/>"#,
        )
        .unwrap();
        if !create_file_link_or_skip(&real_wrapper, &wrapper) {
            let _ = fs::remove_dir_all(&context.cwd);
            return;
        }

        let error = resolve_platform_xml_owners(&content, &context)
            .expect_err("metadata wrapper links must never be followed");

        assert!(
            error.message.contains("symbolic link") || error.message.contains("reparse point"),
            "{}",
            error.message
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn explicit_configuration_root_directory_resolves_its_direct_owner() {
        let context = temp_context("direct-configuration-root");
        let owner = context.cwd.join("dump/Configuration.xml");
        fs::create_dir_all(owner.parent().unwrap()).unwrap();
        fs::write(
            &owner,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();

        let owners =
            resolve_platform_xml_owners(owner.parent().unwrap(), &context).expect("owner resolves");

        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].path, normalized_path(&owner));
        assert_eq!(owners[0].version.as_deref(), Some("2.21"));
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn new_output_reauthorizes_existing_direct_configuration_root() {
        let context = temp_context("new-output-configuration-root");
        let owner = context.cwd.join("dump/Configuration.xml");
        fs::create_dir_all(owner.parent().unwrap()).unwrap();
        fs::write(
            &owner,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();

        let owners =
            resolve_existing_platform_xml_owners_for_new_output(owner.parent().unwrap(), &context)
                .expect("owner resolves");

        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].path, normalized_path(&owner));
        assert_eq!(owners[0].version.as_deref(), Some("2.19"));
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn new_output_checks_an_existing_exact_version_owner_before_its_container() {
        let context = temp_context("new-output-exact-version-owner");
        let target = context.cwd.join("src/Languages/Русский.xml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(
            &target,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Language/></MetaDataObject>"#,
        )
        .unwrap();

        let owners = resolve_existing_platform_xml_owners_for_new_output(&target, &context)
            .expect("exact planned owner resolves");

        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].path, normalized_path(&target));
        assert_eq!(owners[0].version.as_deref(), Some("2.21"));
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn new_output_treats_an_existing_version_root_without_version_as_format_1_0() {
        let context = temp_context("new-output-exact-versionless-metadata");
        let target = context.cwd.join("src/Languages/Русский.xml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(
            &target,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Language/></MetaDataObject>"#,
        )
        .unwrap();

        let owners = resolve_existing_platform_xml_owners_for_new_output(&target, &context)
            .expect("versionless exact planned owner resolves");

        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].path, normalized_path(&target));
        assert_eq!(owners[0].version, None);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn new_output_versionless_client_application_interface_inherits_configuration_owner() {
        let context = temp_context("new-output-versionless-cai");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let owner = context.cwd.join("src/Configuration.xml");
        let target = context.cwd.join("src/Ext/ClientApplicationInterface.xml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(
            &owner,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            &target,
            br#"<ClientApplicationInterface xmlns="http://v8.1c.ru/8.2/managed-application/core"/>"#,
        )
        .unwrap();

        let owners = resolve_existing_platform_xml_owners_for_new_output(&target, &context)
            .expect("versionless CAI inherits its source-set owner");

        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].path, normalized_path(&owner));
        assert_eq!(owners[0].version.as_deref(), Some("2.20"));
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn existing_form_content_resolves_exact_wrapper_and_source_set_owners() {
        let context = temp_context("form-content-bounded-owners");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let configuration = context.cwd.join("src/Configuration.xml");
        let wrapper = context.cwd.join("src/Catalogs/Goods/Forms/Main.xml");
        let content = context
            .cwd
            .join("src/Catalogs/Goods/Forms/Main/Ext/Form.xml");
        fs::create_dir_all(content.parent().unwrap()).unwrap();
        fs::write(
            &configuration,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            &wrapper,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Form/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            &content,
            br#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.22"/>"#,
        )
        .unwrap();

        let owners = resolve_platform_xml_owners(&content, &context).unwrap();
        let actual = owners
            .iter()
            .map(|owner| {
                (
                    owner.path.clone(),
                    owner.version.as_deref().unwrap_or("1.0").to_string(),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            actual,
            vec![
                (normalized_path(&content), "2.22".to_string()),
                (normalized_path(&wrapper), "2.21".to_string()),
                (normalized_path(&configuration), "2.20".to_string()),
            ]
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn new_template_content_resolves_only_its_exact_wrapper_and_source_set_owner() {
        let context = temp_context("template-content-bounded-owners");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let configuration = context.cwd.join("src/Configuration.xml");
        let wrapper = context.cwd.join("src/Reports/Sales/Templates/Planned.xml");
        let unrelated = context
            .cwd
            .join("src/Reports/Sales/Templates/Unrelated.xml");
        let content = context
            .cwd
            .join("src/Reports/Sales/Templates/Planned/Ext/Template.xml");
        fs::create_dir_all(content.parent().unwrap()).unwrap();
        fs::write(
            &configuration,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            &wrapper,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Template/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            &unrelated,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.23"><Template/></MetaDataObject>"#,
        )
        .unwrap();

        let owners = resolve_existing_platform_xml_owners_for_new_output(&content, &context)
            .expect("bounded new-output owners resolve");
        let paths = owners
            .iter()
            .map(|owner| owner.path.clone())
            .collect::<Vec<_>>();

        assert_eq!(
            paths,
            vec![normalized_path(&wrapper), normalized_path(&configuration),]
        );
        assert!(!paths.contains(&normalized_path(&unrelated)));
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn rights_content_resolves_its_role_wrapper() {
        let context = temp_context("rights-content-bounded-owners");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let configuration = context.cwd.join("src/Configuration.xml");
        let wrapper = context.cwd.join("src/Roles/Reader.xml");
        let rights = context.cwd.join("src/Roles/Reader/Ext/Rights.xml");
        fs::create_dir_all(rights.parent().unwrap()).unwrap();
        fs::write(
            &configuration,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            &wrapper,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Role/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            &rights,
            br#"<Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20"/>"#,
        )
        .unwrap();

        let owners = resolve_platform_xml_owners(&rights, &context).unwrap();
        let paths = owners
            .iter()
            .map(|owner| owner.path.clone())
            .collect::<Vec<_>>();

        assert_eq!(
            paths,
            vec![
                normalized_path(&rights),
                normalized_path(&wrapper),
                normalized_path(&configuration),
            ]
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn new_output_at_empty_nested_source_root_does_not_inherit_outer_owner() {
        let context = temp_context("empty-nested-source-root");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: nested\n    type: CONFIGURATION\n    path: src/new\n",
        )
        .unwrap();
        fs::create_dir_all(context.cwd.join("src/new")).unwrap();
        fs::write(
            context.cwd.join("src/Configuration.xml"),
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();

        let owners = resolve_existing_platform_xml_owners_for_new_output(
            &context.cwd.join("src/new"),
            &context,
        )
        .expect("nested ownership boundary resolves");

        assert!(owners.is_empty(), "{owners:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn new_output_uses_deepest_nested_owner_and_ignores_outer_newer_owner() {
        let context = temp_context("owned-nested-source-root");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: nested\n    type: CONFIGURATION\n    path: src/new\n",
        )
        .unwrap();
        fs::create_dir_all(context.cwd.join("src/new")).unwrap();
        fs::write(
            context.cwd.join("src/Configuration.xml"),
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        let nested_owner = context.cwd.join("src/new/Configuration.xml");
        fs::write(
            &nested_owner,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();

        let owners = resolve_existing_platform_xml_owners_for_new_output(
            &context.cwd.join("src/new"),
            &context,
        )
        .expect("nested ownership boundary resolves");

        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].path, normalized_path(&nested_owner));
        assert_eq!(owners[0].version.as_deref(), Some("2.20"));
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn external_root_resolves_all_artifact_owners_including_content_named_config_dump_info() {
        let cases = [
            (
                "processor",
                "EXTERNAL_DATA_PROCESSORS",
                "ExternalDataProcessor",
                PlatformXmlOwnerKind::ExternalProcessor,
            ),
            (
                "report",
                "EXTERNAL_REPORTS",
                "ExternalReport",
                PlatformXmlOwnerKind::ExternalReport,
            ),
        ];

        for (label, source_type, artifact_tag, expected_kind) in cases {
            let context = temp_context(&format!("external-all-owners-{label}"));
            fs::write(
                context.cwd.join("v8project.yaml"),
                format!(
                    "format: DESIGNER\nsource-set:\n  - name: external\n    type: {source_type}\n    path: external\n"
                ),
            )
            .unwrap();
            let source_root = context.cwd.join("external");
            fs::create_dir_all(&source_root).unwrap();
            for name in ["ConfigDumpInfo.xml", "Second.xml"] {
                fs::write(
                    source_root.join(name),
                    format!(
                        r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><{artifact_tag}/></MetaDataObject>"#
                    ),
                )
                .unwrap();
            }

            let owners = resolve_platform_xml_owners(&source_root, &context)
                .expect("every external artifact owner must resolve");
            let paths = owners
                .iter()
                .map(|owner| owner.path.file_name().unwrap().to_owned())
                .collect::<Vec<_>>();

            assert_eq!(paths, vec!["ConfigDumpInfo.xml", "Second.xml"], "{label}");
            assert!(
                owners.iter().all(|owner| owner.kind == expected_kind),
                "{label}: {owners:?}"
            );
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn external_root_ignores_only_content_confirmed_config_dump_info_sidecar() {
        let context = temp_context("external-runtime-sidecar");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external\n",
        )
        .unwrap();
        let source_root = context.cwd.join("external");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(source_root.join("ConfigDumpInfo.xml"), "<ConfigDumpInfo/>").unwrap();
        fs::write(
            source_root.join("Real.xml"),
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><ExternalDataProcessor/></MetaDataObject>"#
            ),
        )
        .unwrap();

        let owners = resolve_platform_xml_owners(&source_root, &context)
            .expect("the runtime sidecar is not a platform owner");

        assert_eq!(owners.len(), 1, "{owners:?}");
        assert_eq!(owners[0].path.file_name().unwrap(), "Real.xml");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn external_root_ignores_large_content_confirmed_config_dump_info_sidecar() {
        let context = temp_context("external-large-runtime-sidecar");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external\n",
        )
        .unwrap();
        let source_root = context.cwd.join("external");
        fs::create_dir_all(&source_root).unwrap();
        let large_size = 8 * 1024 * 1024 + 1;
        let mut sidecar = b"<ConfigDumpInfo/>".to_vec();
        sidecar.resize(large_size, b' ');
        fs::write(source_root.join("ConfigDumpInfo.xml"), sidecar).unwrap();
        let mut large_descriptor = format!(
            r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><ExternalDataProcessor/></MetaDataObject>"#
        )
        .into_bytes();
        large_descriptor.resize(large_size, b' ');
        fs::write(source_root.join("Large.xml"), large_descriptor).unwrap();
        fs::write(
            source_root.join("Real.xml"),
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><ExternalDataProcessor/></MetaDataObject>"#
            ),
        )
        .unwrap();

        let owners = resolve_platform_xml_owners(&source_root, &context)
            .expect("already-read sidecar classification must not impose an 8 MiB I/O cap");
        let names = owners
            .iter()
            .map(|owner| owner.path.file_name().unwrap().to_owned())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["Large.xml", "Real.xml"]);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn external_root_fails_closed_on_every_non_sidecar_xml_entry() {
        let cases = [
            (
                "wrong-kind",
                format!(
                    r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><ExternalReport/></MetaDataObject>"#
                ),
                "ExternalDataProcessor",
            ),
            (
                "unknown-artifact",
                format!(
                    r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><Garbage/></MetaDataObject>"#
                ),
                "ExternalDataProcessor",
            ),
            (
                "malformed",
                "<MetaDataObject".to_string(),
                "failed to parse",
            ),
        ];

        for (label, rejected_xml, expected_error) in cases {
            let context = temp_context(&format!("external-reject-{label}"));
            fs::write(
                context.cwd.join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external\n",
            )
            .unwrap();
            let source_root = context.cwd.join("external");
            fs::create_dir_all(&source_root).unwrap();
            fs::write(source_root.join("Rejected.xml"), rejected_xml).unwrap();
            fs::write(
                source_root.join("Valid.xml"),
                format!(
                    r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><ExternalDataProcessor/></MetaDataObject>"#
                ),
            )
            .unwrap();

            let error = resolve_platform_xml_owners(&source_root, &context)
                .expect_err("every non-sidecar XML entry must be a valid expected-kind owner");

            assert!(error.message.contains(expected_error), "{label}: {error:?}");
            assert!(!error.message.contains("ambiguous"), "{label}: {error:?}");
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn equal_depth_source_set_owners_are_ambiguous_for_existing_and_new_outputs() {
        let source_set_orders = [
            "  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: src\n  - name: configuration\n    type: CONFIGURATION\n    path: src\n",
            "  - name: configuration\n    type: CONFIGURATION\n    path: src\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: src\n",
        ];

        for (index, source_sets) in source_set_orders.iter().enumerate() {
            let context = temp_context(&format!("ambiguous-same-root-{index}"));
            fs::write(
                context.cwd.join("v8project.yaml"),
                format!("format: DESIGNER\nsource-set:\n{source_sets}"),
            )
            .unwrap();
            let target = context.cwd.join("src/Demo/Ext/ObjectModule.bsl");
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(
                context.cwd.join("src/Configuration.xml"),
                br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
            )
            .unwrap();
            fs::write(
                context.cwd.join("src/Demo.xml"),
                br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><ExternalDataProcessor/></MetaDataObject>"#,
            )
            .unwrap();

            for resolve in [
                resolve_platform_xml_owners
                    as fn(
                        &Path,
                        &WorkspaceContext,
                    )
                        -> Result<Vec<PlatformXmlOwner>, PlatformXmlOwnerError>,
                resolve_existing_platform_xml_owners_for_new_output,
            ] {
                let error = resolve(&target, &context)
                    .expect_err("equal-depth source-set ownership must fail closed");

                assert!(error.message.contains("ambiguous source-set"), "{error:?}");
                assert!(error.message.contains("external"), "{error:?}");
                assert!(error.message.contains("configuration"), "{error:?}");
                assert!(error.message.contains("2 equally specific"), "{error:?}");
            }
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn owner_provenance_rejects_project_map_remap_before_binding() {
        let context = temp_context("project-map-remap-provenance");
        let project_map = context.cwd.join("v8project.yaml");
        fs::write(
            &project_map,
            "format: DESIGNER\nsource-set:\n  - name: configuration\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let target = context.cwd.join("src/Demo/Ext/ObjectModule.bsl");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(
            context.cwd.join("src/Configuration.xml"),
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();

        let resolution = resolve_platform_xml_owners_with_provenance(&target, &context)
            .expect("initial configuration owner resolves");
        fs::write(
            &project_map,
            "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: src\n",
        )
        .unwrap();

        let error = resolution
            .provenance
            .bind_to(&mut CompileTransaction::new())
            .expect_err("the exact source-map snapshot must be bound to the transaction");

        assert!(error.contains("v8project.yaml"), "{error}");
        assert!(error.contains("changed"), "{error}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn owner_provenance_rejects_late_metadata_wrapper_before_binding() {
        let context = temp_context("late-wrapper-provenance");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: configuration\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::create_dir_all(context.cwd.join("src")).unwrap();
        fs::write(
            context.cwd.join("src/Configuration.xml"),
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        let wrapper = context.cwd.join("src/Reports/Sales/Templates/Planned.xml");
        let content = context
            .cwd
            .join("src/Reports/Sales/Templates/Planned/Ext/Template.xml");
        fs::create_dir_all(content.parent().unwrap()).unwrap();
        fs::write(
            &content,
            br#"<Template xmlns="http://v8.1c.ru/8.3/xcf/data"/>"#,
        )
        .unwrap();

        let resolution = resolve_platform_xml_owners_with_provenance(&content, &context)
            .expect("versionless content inherits the configuration owner");
        fs::write(
            &wrapper,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Template/></MetaDataObject>"#,
        )
        .unwrap();

        let error = resolution
            .provenance
            .bind_to(&mut CompileTransaction::new())
            .expect_err("an absent wrapper candidate must stay absent");

        assert!(error.contains("Planned.xml"), "{error}");
        assert!(error.contains("absence guard"), "{error}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn owner_provenance_rejects_late_external_root_descriptor_before_binding() {
        let context = temp_context("late-external-descriptor-provenance");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external\n",
        )
        .unwrap();
        let source_root = context.cwd.join("external");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(
            source_root.join("Existing.xml"),
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><ExternalDataProcessor/></MetaDataObject>"#
            ),
        )
        .unwrap();

        let resolution = resolve_existing_platform_xml_owners_for_new_output_with_provenance(
            &source_root,
            &context,
        )
        .expect("initial external owner set resolves");
        fs::write(
            source_root.join("Late.xml"),
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.21"><ExternalDataProcessor/></MetaDataObject>"#
            ),
        )
        .unwrap();

        let error = resolution
            .provenance
            .bind_to(&mut CompileTransaction::new())
            .expect_err("external root membership must stay unchanged");

        assert!(error.contains("directory membership"), "{error}");
        assert!(error.contains("Late.xml"), "{error}");
        let _ = fs::remove_dir_all(&context.cwd);
    }
}
