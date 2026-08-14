use crate::domain::project_sources::{
    config_dump_info_xml_kind, ConfigDumpInfoXmlKind, ProjectSourceMap, ProjectSourceSet,
    SourceFormat, SourceSetKind,
};
use crate::domain::source_roots::select_default_source_set;
use crate::infrastructure::native_operations::compile_transaction::CompileTransaction;
use crate::infrastructure::platform::filesystem::{host_path_text, path_starts_with_host_root};
use crate::infrastructure::platform::secure_read::read_root_relative_regular_file;
use crate::infrastructure::source_roots::{
    normalize_contained_source_root, normalize_path_identity,
};
use serde_yaml::Value as YamlValue;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const MAX_RESERVED_EXTERNAL_DESCRIPTOR_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const PROJECT_SOURCE_MAP_INPUT_MAX_BYTES: usize = 64 * 1024 * 1024;
const PROJECT_SOURCE_MAP_TOTAL_MAX_BYTES: usize = 64 * 1024 * 1024;
const EDT_SOURCE_MARKERS: &[&str] = &[
    ".project",
    "DT-INF/PROJECT.PMF",
    "Configuration/Configuration.mdo",
    "src/Configuration/Configuration.mdo",
];

#[cfg(test)]
type ProjectInputSnapshotTestHook = Box<dyn FnOnce(&Path)>;

#[cfg(test)]
thread_local! {
    static BEFORE_PROJECT_INPUT_SECURE_READ_HOOK: std::cell::RefCell<Option<ProjectInputSnapshotTestHook>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
struct BeforeProjectInputSecureReadHookGuard;

#[cfg(test)]
impl Drop for BeforeProjectInputSecureReadHookGuard {
    fn drop(&mut self) {
        BEFORE_PROJECT_INPUT_SECURE_READ_HOOK.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

#[cfg(test)]
#[must_use]
fn set_before_project_input_secure_read_hook_for_test(
    hook: impl FnOnce(&Path) + 'static,
) -> BeforeProjectInputSecureReadHookGuard {
    BEFORE_PROJECT_INPUT_SECURE_READ_HOOK.with(|slot| {
        let mut slot = slot.borrow_mut();
        assert!(slot.is_none(), "project input secure-read hook leaked");
        *slot = Some(Box::new(hook));
    });
    BeforeProjectInputSecureReadHookGuard
}

fn run_before_project_input_secure_read_hook(path: &Path) {
    #[cfg(test)]
    BEFORE_PROJECT_INPUT_SECURE_READ_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook(path);
        }
    });
    #[cfg(not(test))]
    let _ = path;
}

#[derive(Debug, Clone)]
struct ConfigSourceSet {
    name: String,
    kind: SourceSetKind,
    path: String,
    default_format: Option<SourceFormat>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectSourceMapEvidencePolicy {
    General,
    RuntimeAttested,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProjectSourceMapInput {
    ExactFile(Arc<[u8]>),
    Absent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectSourceMapProvenance {
    workspace_root: PathBuf,
    logical_workspace_root: PathBuf,
    inputs: BTreeMap<PathBuf, ProjectSourceMapInput>,
    exact_bytes: usize,
}

impl ProjectSourceMapProvenance {
    fn new(workspace_root: &Path) -> Result<Self, String> {
        Ok(Self {
            workspace_root: normalize_path_identity(workspace_root)?,
            logical_workspace_root: absolute_lexical_path(workspace_root)?,
            inputs: BTreeMap::new(),
            exact_bytes: 0,
        })
    }

    pub(crate) fn bind_to(&self, transaction: &mut CompileTransaction) -> Result<(), String> {
        for (path, input) in &self.inputs {
            match input {
                ProjectSourceMapInput::ExactFile(raw) => {
                    if !transaction.protects_path(path)? {
                        transaction.guard_or_verify_exact_preimage(path, raw)?;
                    }
                }
                ProjectSourceMapInput::Absent => {
                    if !transaction.protects_path(path)? {
                        transaction.guard_path_absent(path)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn revalidate(&self) -> Result<(), String> {
        for (path, input) in &self.inputs {
            let current = read_root_relative_regular_file(
                &self.workspace_root,
                path,
                PROJECT_SOURCE_MAP_INPUT_MAX_BYTES,
                |_| {},
            );
            match (input, current) {
                (ProjectSourceMapInput::ExactFile(expected), Ok(read))
                    if read.bytes.as_slice() == expected.as_ref() => {}
                (ProjectSourceMapInput::Absent, Err(error))
                    if error.kind() == std::io::ErrorKind::NotFound => {}
                (_, Ok(_)) => {
                    return Err(format!(
                        "project source-map input changed after resolving: {}",
                        path.display()
                    ));
                }
                (_, Err(error)) => {
                    return Err(format!(
                        "project source-map input became unavailable after resolving: {}: {error}",
                        path.display()
                    ));
                }
            }
        }
        Ok(())
    }

    fn record_exact_file(&mut self, path: PathBuf, raw: Arc<[u8]>) -> Result<(), String> {
        match self.inputs.get(&path) {
            Some(ProjectSourceMapInput::ExactFile(existing)) if existing == &raw => Ok(()),
            Some(_) => Err(format!(
                "project source-map input changed while resolving: {}",
                path.display()
            )),
            None => {
                self.exact_bytes = self.exact_bytes.checked_add(raw.len()).ok_or_else(|| {
                    "project source-map evidence exceeded its total byte limit".to_string()
                })?;
                if self.exact_bytes > PROJECT_SOURCE_MAP_TOTAL_MAX_BYTES {
                    return Err(format!(
                        "project source-map evidence exceeds the {} byte total limit",
                        PROJECT_SOURCE_MAP_TOTAL_MAX_BYTES
                    ));
                }
                self.inputs
                    .insert(path, ProjectSourceMapInput::ExactFile(raw));
                Ok(())
            }
        }
    }

    fn record_absence(&mut self, path: PathBuf) -> Result<(), String> {
        match self.inputs.get(&path) {
            Some(ProjectSourceMapInput::Absent) => Ok(()),
            Some(_) => Err(format!(
                "project source-map input changed while resolving: {}",
                path.display()
            )),
            None => {
                self.inputs.insert(path, ProjectSourceMapInput::Absent);
                Ok(())
            }
        }
    }
}

pub(crate) fn discover_project_source_map(
    workspace_root: &Path,
) -> Result<ProjectSourceMap, String> {
    discover_project_source_map_with_provenance(workspace_root).map(|(source_map, _)| source_map)
}

pub(crate) fn discover_project_source_map_with_provenance(
    workspace_root: &Path,
) -> Result<(ProjectSourceMap, ProjectSourceMapProvenance), String> {
    discover_project_source_map_with_provenance_policy(
        workspace_root,
        ProjectSourceMapEvidencePolicy::General,
    )
}

pub(crate) fn discover_runtime_project_source_map_with_provenance(
    workspace_root: &Path,
) -> Result<(ProjectSourceMap, ProjectSourceMapProvenance), String> {
    discover_project_source_map_with_provenance_policy(
        workspace_root,
        ProjectSourceMapEvidencePolicy::RuntimeAttested,
    )
}

fn discover_project_source_map_with_provenance_policy(
    workspace_root: &Path,
    evidence_policy: ProjectSourceMapEvidencePolicy,
) -> Result<(ProjectSourceMap, ProjectSourceMapProvenance), String> {
    let mut provenance = ProjectSourceMapProvenance::new(workspace_root)?;
    let project_config = workspace_root.join("v8project.yaml");
    let project_config_raw =
        snapshot_project_map_input(&project_config, &mut provenance, evidence_policy)?;
    let config_path = project_config_raw.as_ref().map(|_| project_config.clone());
    let (mut source_sets, configured_format_raw) = if let Some(raw) = &project_config_raw {
        read_config_source_sets(workspace_root, &project_config, raw)?
    } else {
        (Vec::new(), None)
    };

    if source_sets.is_empty() {
        source_sets = autodetect_source_sets(workspace_root, &mut provenance, evidence_policy)?;
    }

    let project_source_sets = source_sets
        .into_iter()
        .map(|source_set| {
            detect_source_set_format(workspace_root, source_set, &mut provenance, evidence_policy)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (effective_source_set, effective_source_root, source_selection_error) =
        match select_default_source_set(&project_source_sets) {
            Ok(source_set) => {
                match normalize_contained_source_root(workspace_root, &source_set.path) {
                    Ok(root) => (
                        Some(source_set.name.clone()),
                        Some(root.display().to_string()),
                        None,
                    ),
                    Err(error) => (None, None, Some(format!("invalid_source_root: {error}"))),
                }
            }
            Err(error) => (None, None, Some(format!("invalid_source_root: {error}"))),
        };

    Ok((
        ProjectSourceMap {
            workspace_root: workspace_root.display().to_string(),
            config_path: config_path.map(|path| path.display().to_string()),
            source_sets: project_source_sets,
            effective_source_set,
            effective_source_root,
            source_selection_error,
            configured_format_raw,
        },
        provenance,
    ))
}

fn read_config_source_sets(
    workspace_root: &Path,
    config_path: &Path,
    raw: &[u8],
) -> Result<(Vec<ConfigSourceSet>, Option<String>), String> {
    let text = std::str::from_utf8(raw)
        .map_err(|err| format!("failed to read {} as UTF-8: {err}", config_path.display()))?;
    let yaml = serde_yaml::from_str::<YamlValue>(text)
        .map_err(|err| format!("failed to parse {}: {err}", config_path.display()))?;
    let configured_format_raw = match yaml_mapping_get(&yaml, "format") {
        None => None,
        Some(YamlValue::String(value)) => Some(value.clone()),
        Some(_) => {
            return Err(format!(
                "{} field `format` must be a string",
                config_path.display()
            ));
        }
    };
    let default_format = configured_format_raw
        .clone()
        .and_then(source_format_from_config);
    let base_path = yaml_string(&yaml, "basePath").unwrap_or_else(|| ".".to_string());
    let source_set_value = yaml_mapping_get(&yaml, "source-set");
    let mut source_sets = Vec::new();

    match source_set_value {
        Some(YamlValue::Sequence(entries)) => {
            for entry in entries {
                source_sets.push(config_source_set_from_yaml(entry, default_format)?);
            }
        }
        Some(YamlValue::Mapping(entries)) => {
            for (key, entry) in entries {
                let name = key.as_str().unwrap_or("main");
                source_sets.push(config_source_set_from_named_yaml(
                    name,
                    entry,
                    default_format,
                )?);
            }
        }
        Some(YamlValue::Null) | None => {}
        Some(_) => {
            return Err(format!(
                "{} field `source-set` must be a list or mapping",
                config_path.display()
            ));
        }
    }

    for source_set in &mut source_sets {
        source_set.path = normalize_configured_path(workspace_root, &base_path, &source_set.path);
    }

    Ok((source_sets, configured_format_raw))
}

fn snapshot_project_map_input(
    path: &Path,
    provenance: &mut ProjectSourceMapProvenance,
    evidence_policy: ProjectSourceMapEvidencePolicy,
) -> Result<Option<Arc<[u8]>>, String> {
    if evidence_policy == ProjectSourceMapEvidencePolicy::General {
        return snapshot_general_project_map_input(path, provenance);
    }

    // Resolve the logical workspace-relative identity before the testable race
    // point, then open that same route beneath a retained, no-follow root. A
    // leaf or parent replacement can therefore only make the snapshot fail;
    // it cannot silently retarget the provenance to another file.
    let identity = logical_project_input_identity(
        &provenance.workspace_root,
        &provenance.logical_workspace_root,
        path,
    )?;
    run_before_project_input_secure_read_hook(path);
    let remaining = PROJECT_SOURCE_MAP_TOTAL_MAX_BYTES.saturating_sub(provenance.exact_bytes);
    let maximum_bytes = if provenance.inputs.contains_key(&identity) {
        PROJECT_SOURCE_MAP_INPUT_MAX_BYTES
    } else {
        PROJECT_SOURCE_MAP_INPUT_MAX_BYTES.min(remaining)
    };
    let raw = match read_root_relative_regular_file(
        &provenance.workspace_root,
        &identity,
        maximum_bytes,
        |_| {},
    ) {
        Ok(read) => Arc::<[u8]>::from(read.bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            provenance.record_absence(identity)?;
            return Ok(None);
        }
        Err(error) => {
            return Err(format!(
                "failed to securely read project source-map input {} within its byte limit: \
                 {error}",
                path.display()
            ));
        }
    };
    provenance.record_exact_file(identity, Arc::clone(&raw))?;
    Ok(Some(raw))
}

fn snapshot_general_project_map_input(
    path: &Path,
    provenance: &mut ProjectSourceMapProvenance,
) -> Result<Option<Arc<[u8]>>, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let identity = normalize_path_identity(path)?;
            provenance.record_absence(identity)?;
            return Ok(None);
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect project source-map input {}: {error}",
                path.display()
            ));
        }
    };
    if crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point(&metadata) {
        return Err(format!(
            "project source-map input must not be a symbolic link or reparse point: {}",
            path.display()
        ));
    }
    if !metadata.is_file() {
        return Err(format!(
            "project source-map input is not a regular file: {}",
            path.display()
        ));
    }

    let identity = normalize_path_identity(path)?;
    let remaining = PROJECT_SOURCE_MAP_TOTAL_MAX_BYTES.saturating_sub(provenance.exact_bytes);
    let already_bound = provenance.inputs.contains_key(&identity);
    let maximum_bytes = if already_bound {
        PROJECT_SOURCE_MAP_INPUT_MAX_BYTES
    } else {
        PROJECT_SOURCE_MAP_INPUT_MAX_BYTES.min(remaining)
    };
    if metadata.len() > maximum_bytes as u64 {
        return Err(project_input_byte_limit_error(
            path,
            maximum_bytes,
            remaining,
            already_bound,
        ));
    }
    let file = std::fs::File::open(path).map_err(|error| {
        format!(
            "failed to read project source-map input {}: {error}",
            path.display()
        )
    })?;
    let mut raw = Vec::with_capacity((metadata.len() as usize).min(maximum_bytes));
    file.take(maximum_bytes as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|error| {
            format!(
                "failed to read project source-map input {}: {error}",
                path.display()
            )
        })?;
    if raw.len() > maximum_bytes {
        return Err(project_input_byte_limit_error(
            path,
            maximum_bytes,
            remaining,
            already_bound,
        ));
    }
    let raw = Arc::<[u8]>::from(raw);
    provenance.record_exact_file(identity, Arc::clone(&raw))?;
    Ok(Some(raw))
}

fn project_input_byte_limit_error(
    path: &Path,
    maximum_bytes: usize,
    remaining_total_bytes: usize,
    already_bound: bool,
) -> String {
    if already_bound {
        return format!(
            "project source-map input {} exceeds its per-input limit {} bytes",
            path.display(),
            PROJECT_SOURCE_MAP_INPUT_MAX_BYTES
        );
    }
    format!(
        "project source-map input {} exceeds its effective byte limit of {} bytes (per-input \
         limit {} bytes; remaining total budget {} bytes)",
        path.display(),
        maximum_bytes,
        PROJECT_SOURCE_MAP_INPUT_MAX_BYTES,
        remaining_total_bytes
    )
}

fn logical_project_input_identity(
    workspace_root: &Path,
    logical_workspace_root: &Path,
    path: &Path,
) -> Result<PathBuf, String> {
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        logical_workspace_root.join(path)
    };
    let normalized = absolute_lexical_path(&candidate)?;
    let relative = relative_to_host_root(&normalized, logical_workspace_root)
        .or_else(|| relative_to_host_root(&normalized, workspace_root))
        .ok_or_else(|| {
            format!(
                "project source-map input is outside workspace root {}: {}",
                logical_workspace_root.display(),
                normalized.display()
            )
        })?;
    Ok(workspace_root.join(relative))
}

fn relative_to_host_root(path: &Path, root: &Path) -> Option<PathBuf> {
    if !path_starts_with_host_root(path, root) {
        return None;
    }
    Some(path.components().skip(root.components().count()).collect())
}

fn absolute_lexical_path(path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("failed to determine current directory: {error}"))?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(format!(
                        "project source-map input escapes the filesystem root: {}",
                        path.display()
                    ));
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    if !normalized.is_absolute() {
        return Err(format!(
            "project source-map input did not resolve to an absolute path: {}",
            path.display()
        ));
    }
    Ok(normalized)
}

fn config_source_set_from_yaml(
    entry: &YamlValue,
    default_format: Option<SourceFormat>,
) -> Result<ConfigSourceSet, String> {
    let name = yaml_string(entry, "name").unwrap_or_else(|| "main".to_string());
    config_source_set_from_named_yaml(&name, entry, default_format)
}

fn config_source_set_from_named_yaml(
    name: &str,
    entry: &YamlValue,
    default_format: Option<SourceFormat>,
) -> Result<ConfigSourceSet, String> {
    let source_type = yaml_string(entry, "type")
        .or_else(|| yaml_string(entry, "purpose"))
        .unwrap_or_else(|| "CONFIGURATION".to_string());
    let kind = source_set_kind_from_config(&source_type)?;
    let path = yaml_string(entry, "path").unwrap_or_else(|| ".".to_string());
    Ok(ConfigSourceSet {
        name: name.to_string(),
        kind,
        path,
        default_format,
    })
}

fn normalize_configured_path(workspace_root: &Path, base_path: &str, raw_path: &str) -> String {
    let base = PathBuf::from(base_path);
    let path = PathBuf::from(raw_path);
    let resolved = if path.is_absolute() {
        path
    } else if base.is_absolute() {
        base.join(path)
    } else {
        workspace_root.join(base).join(path)
    };
    path_relative_to(workspace_root, &resolved)
}

fn autodetect_source_sets(
    workspace_root: &Path,
    provenance: &mut ProjectSourceMapProvenance,
    evidence_policy: ProjectSourceMapEvidencePolicy,
) -> Result<Vec<ConfigSourceSet>, String> {
    for path in [".", "src", "src/cf"] {
        let root = workspace_root.join(path);
        let mut found = false;
        for marker in [
            root.join("Configuration.xml"),
            root.join("Configuration/Configuration.mdo"),
            root.join("src/Configuration/Configuration.mdo"),
        ] {
            found |= snapshot_project_map_input(&marker, provenance, evidence_policy)?.is_some();
        }
        if found {
            return Ok(vec![ConfigSourceSet {
                name: "main".to_string(),
                kind: SourceSetKind::Configuration,
                path: path.to_string(),
                default_format: None,
            }]);
        }
    }
    Ok(Vec::new())
}

fn detect_source_set_format(
    workspace_root: &Path,
    source_set: ConfigSourceSet,
    provenance: &mut ProjectSourceMapProvenance,
    evidence_policy: ProjectSourceMapEvidencePolicy,
) -> Result<ProjectSourceSet, String> {
    let source_root = workspace_root.join(&source_set.path);
    if evidence_policy == ProjectSourceMapEvidencePolicy::RuntimeAttested
        && normalize_contained_source_root(workspace_root, &source_set.path).is_err()
    {
        let source_format = source_set.default_format.unwrap_or(SourceFormat::Unknown);
        let format_evidence = source_set
            .default_format
            .map(|format| match format {
                SourceFormat::PlatformXml => "v8project.yaml:format=DESIGNER".to_string(),
                SourceFormat::Edt => "v8project.yaml:format=EDT".to_string(),
                SourceFormat::Unknown | SourceFormat::Invalid => {
                    "v8project.yaml:format".to_string()
                }
            })
            .into_iter()
            .collect();
        return Ok(ProjectSourceSet {
            name: source_set.name,
            kind: source_set.kind,
            path: source_set.path,
            source_format,
            format_evidence,
        });
    }
    let platform_evidence = platform_xml_evidence(
        workspace_root,
        &source_root,
        source_set.kind,
        provenance,
        evidence_policy,
    )?;
    let edt_evidence = edt_evidence(workspace_root, &source_root, provenance, evidence_policy)?;
    let source_format = classify_source_format(
        !platform_evidence.is_empty(),
        !edt_evidence.is_empty(),
        source_set.default_format,
    );
    let mut format_evidence = Vec::new();
    format_evidence.extend(platform_evidence);
    format_evidence.extend(edt_evidence);
    if format_evidence.is_empty() {
        if let Some(default_format) = source_set.default_format {
            format_evidence.push(match default_format {
                SourceFormat::PlatformXml => "v8project.yaml:format=DESIGNER".to_string(),
                SourceFormat::Edt => "v8project.yaml:format=EDT".to_string(),
                SourceFormat::Unknown | SourceFormat::Invalid => {
                    "v8project.yaml:format".to_string()
                }
            });
        }
    }

    Ok(ProjectSourceSet {
        name: source_set.name,
        kind: source_set.kind,
        path: source_set.path,
        source_format,
        format_evidence,
    })
}

fn classify_source_format(
    has_platform_evidence: bool,
    has_edt_evidence: bool,
    default_format: Option<SourceFormat>,
) -> SourceFormat {
    match (has_platform_evidence, has_edt_evidence) {
        (true, true) => SourceFormat::Invalid,
        (true, false) => SourceFormat::PlatformXml,
        (false, true) => SourceFormat::Edt,
        (false, false) => default_format.unwrap_or(SourceFormat::Unknown),
    }
}

pub(crate) fn classify_physical_source_inventory<'a>(
    kind: SourceSetKind,
    relative_files: impl IntoIterator<Item = &'a Path>,
) -> SourceFormat {
    let mut has_platform_evidence = false;
    let mut has_edt_evidence = false;
    for relative in relative_files {
        if relative == Path::new("Configuration.xml") {
            has_platform_evidence = true;
        }
        if EDT_SOURCE_MARKERS
            .iter()
            .any(|marker| relative == Path::new(marker))
        {
            has_edt_evidence = true;
        }
        if matches!(
            kind,
            SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
        ) && relative.parent().is_none()
            && relative
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
        {
            has_platform_evidence = true;
        }
    }
    classify_source_format(has_platform_evidence, has_edt_evidence, None)
}

fn platform_xml_evidence(
    workspace_root: &Path,
    source_root: &Path,
    kind: SourceSetKind,
    provenance: &mut ProjectSourceMapProvenance,
    evidence_policy: ProjectSourceMapEvidencePolicy,
) -> Result<Vec<String>, String> {
    let mut evidence = Vec::new();
    let configuration_descriptor = source_root.join("Configuration.xml");
    if evidence_policy == ProjectSourceMapEvidencePolicy::RuntimeAttested {
        push_snapshot_existing(
            &mut evidence,
            workspace_root,
            &configuration_descriptor,
            provenance,
            evidence_policy,
        )?;
    } else {
        // General project discovery deliberately leaves the descriptor to the
        // platform owner/containment boundary. That boundary owns the stable
        // link diagnostic; consuming the link here would turn a structured
        // denial into an unrelated source-map I/O failure. Runtime incremental
        // authorization opts into the exact descriptor snapshot above.
        push_existing(&mut evidence, workspace_root, &configuration_descriptor);
    }

    if matches!(
        kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        // Multiplicity does not affect source-format classification: one or
        // several descriptor files are the same `has_platform_evidence=true`.
        // The target-specific owner resolver binds exact descriptor bytes, and
        // root-level external mutations additionally bind directory membership.
        if let Ok(entries) = std::fs::read_dir(source_root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("xml")
                    && !is_config_dump_info_sidecar(&path, kind)
                {
                    push_existing(&mut evidence, workspace_root, &path);
                }
            }
        }
    }
    evidence.sort();
    evidence.dedup();
    Ok(evidence)
}

fn is_config_dump_info_sidecar(path: &Path, kind: SourceSetKind) -> bool {
    if !has_config_dump_info_filename(path) {
        return false;
    }
    !matches!(
        (config_dump_info_xml_file_kind(path), kind),
        (
            ConfigDumpInfoXmlKind::ExternalProcessor,
            SourceSetKind::ExternalProcessor
        ) | (
            ConfigDumpInfoXmlKind::ExternalReport,
            SourceSetKind::ExternalReport
        )
    )
}

fn config_dump_info_xml_file_kind(path: &Path) -> ConfigDumpInfoXmlKind {
    if !has_config_dump_info_filename(path) {
        return ConfigDumpInfoXmlKind::Other;
    }
    let Ok(link_metadata) = std::fs::symlink_metadata(path) else {
        return ConfigDumpInfoXmlKind::Other;
    };
    if link_metadata.file_type().is_symlink() || !link_metadata.file_type().is_file() {
        return ConfigDumpInfoXmlKind::Other;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return ConfigDumpInfoXmlKind::Other;
    };
    let Ok(metadata) = file.metadata() else {
        return ConfigDumpInfoXmlKind::Other;
    };
    if !metadata.file_type().is_file() || metadata.len() > MAX_RESERVED_EXTERNAL_DESCRIPTOR_BYTES {
        return ConfigDumpInfoXmlKind::Other;
    }
    let mut bytes = Vec::new();
    if (&mut file)
        .take(MAX_RESERVED_EXTERNAL_DESCRIPTOR_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > MAX_RESERVED_EXTERNAL_DESCRIPTOR_BYTES
    {
        return ConfigDumpInfoXmlKind::Other;
    }
    config_dump_info_xml_kind(&bytes)
}

fn has_config_dump_info_filename(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("ConfigDumpInfo.xml"))
}

fn edt_evidence(
    workspace_root: &Path,
    source_root: &Path,
    provenance: &mut ProjectSourceMapProvenance,
    evidence_policy: ProjectSourceMapEvidencePolicy,
) -> Result<Vec<String>, String> {
    let mut evidence = Vec::new();
    for rel in EDT_SOURCE_MARKERS {
        push_snapshot_existing(
            &mut evidence,
            workspace_root,
            &source_root.join(rel),
            provenance,
            evidence_policy,
        )?;
    }
    evidence.sort();
    evidence.dedup();
    Ok(evidence)
}

fn push_snapshot_existing(
    evidence: &mut Vec<String>,
    workspace_root: &Path,
    path: &Path,
    provenance: &mut ProjectSourceMapProvenance,
    evidence_policy: ProjectSourceMapEvidencePolicy,
) -> Result<(), String> {
    if snapshot_project_map_input(path, provenance, evidence_policy)?.is_some() {
        evidence.push(path_relative_to(workspace_root, path));
    }
    Ok(())
}

fn push_existing(evidence: &mut Vec<String>, workspace_root: &Path, path: &Path) {
    if path.is_file() {
        evidence.push(path_relative_to(workspace_root, path));
    }
}

fn path_relative_to(root: &Path, path: &Path) -> String {
    host_path_text(
        path.strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string(),
    )
}

fn source_set_kind_from_config(raw: &str) -> Result<SourceSetKind, String> {
    match raw.to_ascii_uppercase().as_str() {
        "CONFIGURATION" => Ok(SourceSetKind::Configuration),
        "EXTENSION" => Ok(SourceSetKind::Extension),
        "EXTERNAL_DATA_PROCESSORS" => Ok(SourceSetKind::ExternalProcessor),
        "EXTERNAL_REPORTS" => Ok(SourceSetKind::ExternalReport),
        other => Err(format!("unsupported source-set type `{other}`")),
    }
}

fn source_format_from_config(raw: String) -> Option<SourceFormat> {
    match raw.to_ascii_uppercase().as_str() {
        "DESIGNER" | "PLATFORM_XML" | "XML" => Some(SourceFormat::PlatformXml),
        "EDT" => Some(SourceFormat::Edt),
        _ => None,
    }
}

fn yaml_string(value: &YamlValue, key: &str) -> Option<String> {
    yaml_mapping_get(value, key).and_then(|value| match value {
        YamlValue::String(text) => Some(text.clone()),
        YamlValue::Number(number) => Some(number.to_string()),
        _ => None,
    })
}

fn yaml_mapping_get<'a>(value: &'a YamlValue, key: &str) -> Option<&'a YamlValue> {
    let mapping = value.as_mapping()?;
    mapping.get(YamlValue::String(key.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::source_roots::resolve_source_root;
    use crate::infrastructure::workspace::discover_workspace;
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_WORKSPACE_NONCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn unconsumed_project_input_hook_does_not_leak_past_its_scope() {
        {
            let _guard = set_before_project_input_secure_read_hook_for_test(|_| {});
        }

        let _guard = set_before_project_input_secure_read_hook_for_test(|_| {});
        run_before_project_input_secure_read_hook(Path::new("v8project.yaml"));
    }

    #[test]
    fn general_project_input_reports_per_file_and_remaining_total_limits() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("v8project.yaml");
        fs::write(&input, b"12345").unwrap();
        let mut provenance = ProjectSourceMapProvenance::new(root.path()).unwrap();
        provenance.exact_bytes = PROJECT_SOURCE_MAP_TOTAL_MAX_BYTES - 4;

        let error = snapshot_general_project_map_input(&input, &mut provenance)
            .expect_err("the remaining aggregate budget is smaller than this input");

        assert!(
            error.contains(&format!(
                "per-input limit {PROJECT_SOURCE_MAP_INPUT_MAX_BYTES} bytes"
            )),
            "{error}"
        );
        assert!(error.contains("remaining total budget 4 bytes"), "{error}");
    }

    #[test]
    fn detects_edt_configuration_and_platform_external_processor_source_sets() {
        let root = temp_workspace("unica-source-map-multi");
        write(
            &root.join("v8project.yaml"),
            r#"
format: EDT
source-set:
  - name: main
    type: CONFIGURATION
    path: src
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: epf
"#,
        );
        write(&root.join("src/.project"), "<projectDescription/>");
        write(
            &root.join("src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        );
        write(
            &root.join("epf/PriceLoader.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        );
        write(&root.join("epf/ConfigDumpInfo.xml"), "<ConfigDumpInfo/>");

        let map = discover_project_source_map(&root).unwrap();

        assert_eq!(map.source_sets.len(), 2);
        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::Edt,
            &["src/.project", "src/Configuration/Configuration.mdo"],
        );
        assert_source_set(
            &map,
            "external-processors",
            SourceSetKind::ExternalProcessor,
            SourceFormat::PlatformXml,
            &["epf/PriceLoader.xml"],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_dump_info_alone_is_not_external_source_format_evidence() {
        let root = temp_workspace("unica-source-map-external-cdfi-runtime-state");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: epf
  - name: external-reports
    type: EXTERNAL_REPORTS
    path: erf
"#,
        );
        write(&root.join("epf/ConfigDumpInfo.xml"), "<ConfigDumpInfo/>");
        write(&root.join("erf/configdumpinfo.xml"), "<ConfigDumpInfo/>");

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "external-processors",
            SourceSetKind::ExternalProcessor,
            SourceFormat::Unknown,
            &[],
        );
        assert_source_set(
            &map,
            "external-reports",
            SourceSetKind::ExternalReport,
            SourceFormat::Unknown,
            &[],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn external_object_named_config_dump_info_remains_platform_xml_evidence() {
        let root = temp_workspace("unica-source-map-external-object-named-cdfi");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: epf
"#,
        );
        write(
            &root.join("epf/ConfigDumpInfo.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "external-processors",
            SourceSetKind::ExternalProcessor,
            SourceFormat::PlatformXml,
            &["epf/ConfigDumpInfo.xml"],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nested_external_tag_does_not_make_config_dump_info_source_evidence() {
        let root = temp_workspace("unica-source-map-nested-external-tag-cdfi");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: epf
"#,
        );
        write(
            &root.join("epf/ConfigDumpInfo.xml"),
            "<MetaDataObject><Properties><ExternalDataProcessor/></Properties></MetaDataObject>",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "external-processors",
            SourceSetKind::ExternalProcessor,
            SourceFormat::Unknown,
            &[],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_config_dump_info_is_not_external_source_format_evidence() {
        let root = temp_workspace("unica-source-map-malformed-external-cdfi");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: external-reports
    type: EXTERNAL_REPORTS
    path: erf
"#,
        );
        write(
            &root.join("erf/ConfigDumpInfo.xml"),
            "<<<<<<< ours\n<ConfigDumpInfo/>\n=======\n<ConfigDumpInfo/>\n>>>>>>> theirs",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "external-reports",
            SourceSetKind::ExternalReport,
            SourceFormat::Unknown,
            &[],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn symlinked_config_dump_info_is_not_external_source_format_evidence() {
        let root = temp_workspace("unica-source-map-symlinked-external-cdfi");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: epf
"#,
        );
        write(
            &root.join("outside.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        );
        fs::create_dir_all(root.join("epf")).unwrap();
        let Some(symlink_result) =
            crate::infrastructure::platform::filesystem::create_file_symlink_for_test(
                root.join("outside.xml"),
                root.join("epf/ConfigDumpInfo.xml"),
            )
        else {
            return;
        };
        symlink_result.unwrap();

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "external-processors",
            SourceSetKind::ExternalProcessor,
            SourceFormat::Unknown,
            &[],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn detects_single_platform_configuration_source_set() {
        let root = temp_workspace("unica-source-map-platform");
        write(
            &root.join("v8project.yaml"),
            r#"
format: DESIGNER
source-set:
  - name: main
    type: CONFIGURATION
    path: src
"#,
        );
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
        write(&root.join("src/ConfigDumpInfo.xml"), "<ConfigDumpInfo/>");

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            &["src/Configuration.xml"],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_dump_info_is_not_platform_source_format_evidence() {
        let root = temp_workspace("unica-source-map-cdfi-runtime-state");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: main
    type: CONFIGURATION
    path: src
"#,
        );
        write(&root.join("src/ConfigDumpInfo.xml"), "<ConfigDumpInfo/>");

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::Unknown,
            &[],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ignores_legacy_v8tr_config_environment_override() {
        let _environment_lock = crate::infrastructure::V8TR_CONFIG_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = temp_workspace("unica-source-map-ignore-v8tr-config");
        write(
            &root.join("v8project.yaml"),
            r#"
format: DESIGNER
source-set:
  - name: main
    type: CONFIGURATION
    path: src
"#,
        );
        write(
            &root.join("custom.yaml"),
            r#"
format: DESIGNER
source-set:
  - name: env
    type: CONFIGURATION
    path: env-src
"#,
        );
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
        write(&root.join("env-src/Configuration.xml"), "<MetaDataObject/>");
        let _guard = EnvVarGuard::set("V8TR_CONFIG", root.join("custom.yaml"));

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            &["src/Configuration.xml"],
        );
        assert!(
            map.source_sets
                .iter()
                .all(|source_set| source_set.name != "env"),
            "legacy V8TR_CONFIG source set must be ignored: {map:?}"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn detects_single_edt_configuration_source_set() {
        let root = temp_workspace("unica-source-map-edt");
        write(
            &root.join("v8project.yaml"),
            r#"
format: EDT
source-set:
  - name: main
    type: CONFIGURATION
    path: src
"#,
        );
        write(&root.join("src/.project"), "<projectDescription/>");
        write(
            &root.join("src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::Edt,
            &["src/.project", "src/Configuration/Configuration.mdo"],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn conflicting_markers_inside_one_source_set_are_invalid_not_mixed() {
        let root = temp_workspace("unica-source-map-invalid");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: main
    type: CONFIGURATION
    path: src
"#,
        );
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::Invalid,
            &[
                "src/Configuration.xml",
                "src/Configuration/Configuration.mdo",
            ],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configured_source_provenance_rejects_late_edt_marker() {
        let root = temp_workspace("unica-source-map-late-edt-marker");
        write(
            &root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");

        let (map, provenance) = discover_project_source_map_with_provenance(&root).unwrap();
        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            &["src/Configuration.xml"],
        );
        write(&root.join("src/.project"), "<projectDescription/>");

        let error = provenance
            .bind_to(&mut CompileTransaction::new())
            .expect_err("an EDT marker absent during classification must stay absent");

        assert!(error.contains(".project"), "{error}");
        assert!(error.contains("absence guard"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configured_source_provenance_rejects_changed_edt_marker() {
        let root = temp_workspace("unica-source-map-changed-edt-marker");
        write(
            &root.join("v8project.yaml"),
            "format: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        let marker = root.join("src/.project");
        write(&marker, "<projectDescription/>");

        let (_, provenance) = discover_project_source_map_with_provenance(&root).unwrap();
        write(
            &marker,
            "<projectDescription><name>changed</name></projectDescription>",
        );

        let error = provenance
            .bind_to(&mut CompileTransaction::new())
            .expect_err("the exact EDT marker bytes must stay unchanged");

        assert!(error.contains(".project"), "{error}");
        assert!(error.contains("changed"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configured_source_provenance_rejects_changed_platform_descriptor() {
        let root = temp_workspace("unica-source-map-changed-platform-descriptor");
        write(
            &root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        let descriptor = root.join("src/Configuration.xml");
        write(&descriptor, "<MetaDataObject/>");

        let (_, provenance) = discover_runtime_project_source_map_with_provenance(&root).unwrap();
        write(
            &descriptor,
            "<MetaDataObject version=\"2.20\"></MetaDataObject>",
        );

        let error = provenance
            .revalidate()
            .expect_err("the exact platform descriptor bytes must stay unchanged");

        assert!(error.contains("Configuration.xml"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn project_source_map_rejects_an_oversized_input_before_parsing() {
        let root = temp_workspace("unica-source-map-oversized-config");
        fs::File::create(root.join("v8project.yaml"))
            .unwrap()
            .set_len((PROJECT_SOURCE_MAP_INPUT_MAX_BYTES + 1) as u64)
            .unwrap();

        let error = discover_project_source_map_with_provenance(&root)
            .expect_err("an oversized project config must fail at the bounded read");

        assert!(error.contains("byte limit"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configured_source_provenance_binds_the_logical_primary_during_leaf_replacement() {
        let root = temp_workspace("unica-source-map-primary-leaf-replacement");
        let primary = root.join("v8project.yaml");
        let alternate = root.join("alternate.yaml");
        let probe = root.join("symlink-probe.yaml");
        let original = concat!(
            "format: DESIGNER\n",
            "source-set:\n",
            "  - name: main\n",
            "    type: CONFIGURATION\n",
            "    path: src-a\n",
        );
        let replacement = concat!(
            "format: DESIGNER\n",
            "source-set:\n",
            "  - name: main\n",
            "    type: CONFIGURATION\n",
            "    path: src-b\n",
        );
        write(&primary, original);
        write(&alternate, replacement);
        write(&root.join("src-a/Configuration.xml"), "<MetaDataObject/>");
        write(&root.join("src-b/Configuration.xml"), "<MetaDataObject/>");
        let Some(probe_result) =
            crate::infrastructure::platform::filesystem::create_file_symlink_for_test(
                &alternate, &probe,
            )
        else {
            fs::remove_dir_all(root).unwrap();
            return;
        };
        probe_result.expect("create file-link probe");
        fs::remove_file(&probe).expect("remove file-link probe");
        let primary_for_hook = primary.clone();
        let alternate_for_hook = alternate.clone();
        let _hook_guard = set_before_project_input_secure_read_hook_for_test(move |path| {
            assert_eq!(path, primary_for_hook);
            fs::remove_file(&primary_for_hook).expect("remove primary after metadata check");
            crate::infrastructure::platform::filesystem::create_file_symlink_for_test(
                &alternate_for_hook,
                &primary_for_hook,
            )
            .expect("file links are supported")
            .expect("replace primary with a file link");
        });

        let error = discover_runtime_project_source_map_with_provenance(&root)
            .expect_err("a transient primary link must fail the secure logical-path read");
        fs::remove_file(&primary).expect("remove transient primary link or file");
        write(&primary, original);

        assert!(error.contains("securely read"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn effective_source_root_rejects_relative_workspace_escape() {
        let root = temp_workspace("unica-source-map-relative-escape");
        write(
            &root.join("v8project.yaml"),
            "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: ../outside\n",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert!(map.effective_source_set.is_none());
        assert!(map.effective_source_root.is_none());
        assert!(map
            .source_selection_error
            .as_deref()
            .is_some_and(
                |error| error.starts_with("invalid_source_root:") && error.contains("workspace")
            ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn effective_source_root_rejects_absolute_workspace_escape() {
        let root = temp_workspace("unica-source-map-absolute-escape");
        let outside = temp_workspace("unica-source-map-outside");
        write(
            &root.join("v8project.yaml"),
            &format!(
                "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: {}\n",
                outside.display()
            ),
        );

        let map = discover_project_source_map(&root).unwrap();

        assert!(map.effective_source_root.is_none());
        assert!(map
            .source_selection_error
            .as_deref()
            .is_some_and(|error| error.starts_with("invalid_source_root:")));
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn effective_source_root_uses_resolver_path_identity() {
        let root = temp_workspace("unica-source-map-normalized");
        write(
            &root.join("v8project.yaml"),
            "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: src/../src/cf\n",
        );
        fs::create_dir_all(root.join("src/cf")).unwrap();
        let context = discover_workspace(Some(root.clone())).unwrap();

        let map = discover_project_source_map(&root).unwrap();
        let resolved = resolve_source_root(&context, None).unwrap();

        assert_eq!(map.effective_source_set.as_deref(), Some("main"));
        assert_eq!(
            map.effective_source_root.as_deref(),
            Some(resolved.path.to_string_lossy().as_ref())
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selection_errors_use_the_stable_invalid_source_root_prefix() {
        let ambiguous = temp_workspace("unica-source-map-ambiguous-prefix");
        write(&ambiguous.join("v8project.yaml"), "source-set:\n  - name: app\n    type: CONFIGURATION\n    path: app\n  - name: tests\n    type: CONFIGURATION\n    path: tests\n");
        let map = discover_project_source_map(&ambiguous).unwrap();
        assert_eq!(map.source_selection_error.as_deref(), Some("invalid_source_root: sourceDir is required because configuration source sets are ambiguous: app, tests"));

        let missing = temp_workspace("unica-source-map-missing-prefix");
        let map = discover_project_source_map(&missing).unwrap();
        assert_eq!(map.source_selection_error.as_deref(), Some("invalid_source_root: sourceDir is required because no configuration source set was found"));
        fs::remove_dir_all(ambiguous).unwrap();
        fs::remove_dir_all(missing).unwrap();
    }

    fn assert_source_set(
        map: &ProjectSourceMap,
        name: &str,
        kind: SourceSetKind,
        source_format: SourceFormat,
        expected_evidence: &[&str],
    ) {
        let source_set = map
            .source_sets
            .iter()
            .find(|source_set| source_set.name == name)
            .unwrap_or_else(|| panic!("source set {name} not found in {map:?}"));
        assert_eq!(source_set.kind, kind);
        assert_eq!(source_set.source_format, source_format);
        assert_eq!(
            source_set.format_evidence,
            expected_evidence
                .iter()
                .map(|evidence| (*evidence).to_string())
                .collect::<Vec<_>>()
        );
    }

    fn temp_workspace(prefix: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let nonce = TEMP_WORKSPACE_NONCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "{prefix}-{}-{timestamp}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}
