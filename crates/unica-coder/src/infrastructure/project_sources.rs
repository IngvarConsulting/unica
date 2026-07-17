use super::contained_fs::{
    canonical_workspace, metadata_is_link_or_reparse_point, normalize_relative, observe_open_file,
    observe_regular_file, open_no_follow, reject_link_components, resolve_contained_directory,
    slash_relative, validate_configured_relative_path,
};
use crate::application::discovery::ports::{
    DiscoveryError, DiscoveryExecutionContext, ProjectSourceResolverPort, SourceReadinessError,
    SourceReadinessReason, SourceRole,
};
use crate::domain::project_sources::{
    ProjectSourceMap, ProjectSourceSet, SourceFormat, SourceSetKind,
};
use crate::domain::source_snapshot::{ResolvedSourceSelection, ResolvedSourceSet};
use serde_yaml::Value as YamlValue;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

const MAPPING_DOMAIN: &[u8] = b"unica.project-source-topology.v1";

#[derive(Debug, Clone)]
struct ConfiguredSourceSet {
    name: String,
    kind: SourceSetKind,
    relative_root: String,
    default_format: Option<SourceFormat>,
}

#[derive(Debug, Clone)]
struct LoadedSourceMap {
    canonical_workspace: PathBuf,
    config_path: Option<PathBuf>,
    configured_format_raw: Option<String>,
    source_sets: Vec<ProjectSourceSet>,
    mapping_digest: String,
}

pub(crate) struct FilesystemProjectSourceResolver;

impl ProjectSourceResolverPort for FilesystemProjectSourceResolver {
    fn resolve_all(
        &self,
        context: &DiscoveryExecutionContext,
        requested_analysis: Option<&str>,
        requested_mutations: &[String],
    ) -> Result<ResolvedSourceSelection, DiscoveryError> {
        resolve_source_selection_typed(
            Path::new(&context.workspace_root),
            requested_analysis,
            requested_mutations,
        )
    }
}

pub fn discover_project_source_map(workspace_root: &Path) -> Result<ProjectSourceMap, String> {
    let loaded = load_source_map(workspace_root, false)?;
    Ok(ProjectSourceMap {
        workspace_root: loaded.canonical_workspace.display().to_string(),
        config_path: loaded.config_path.map(|path| path.display().to_string()),
        source_sets: loaded.source_sets,
        configured_format_raw: loaded.configured_format_raw,
    })
}

pub(crate) fn resolve_source_selection(
    workspace_root: &Path,
    requested_analysis: Option<&str>,
    requested_mutations: &[String],
) -> Result<ResolvedSourceSelection, String> {
    resolve_source_selection_typed(workspace_root, requested_analysis, requested_mutations)
        .map_err(|error| error.to_string())
}

pub(crate) fn resolve_source_selection_typed(
    workspace_root: &Path,
    requested_analysis: Option<&str>,
    requested_mutations: &[String],
) -> Result<ResolvedSourceSelection, DiscoveryError> {
    let loaded = load_source_map(workspace_root, true).map_err(DiscoveryError::Operation)?;
    let eligible = loaded
        .source_sets
        .iter()
        .filter(|source| analysis_readiness(source).is_ok())
        .collect::<Vec<_>>();
    let analysis = match requested_analysis {
        Some(name) => resolve_analysis_named(&loaded, name)?,
        None if eligible.len() == 1 => {
            resolved(&loaded, eligible[0]).map_err(DiscoveryError::Operation)?
        }
        None if eligible.is_empty() => {
            if loaded.source_sets.len() == 1 {
                return Err(analysis_readiness(&loaded.source_sets[0]).unwrap_err());
            }
            return Err(DiscoveryError::Operation(
                "no_eligible_source_set: discovery v1 requires an authoritative source layout"
                    .into(),
            ));
        }
        None => {
            return Err(DiscoveryError::Operation("ambiguous_source_set: sourceSet is required when multiple eligible source sets exist".into()));
        }
    };
    let mut mutation_names = requested_mutations.to_vec();
    mutation_names.sort_by_key(|name| name.to_lowercase());
    mutation_names.dedup_by(|left, right| left.to_lowercase() == right.to_lowercase());
    let mutations = mutation_names
        .iter()
        .map(|name| resolve_mutation_named(&loaded, name))
        .collect::<Result<Vec<_>, _>>()?;
    ResolvedSourceSelection::new(analysis, mutations).map_err(DiscoveryError::Operation)
}

fn find_named<'a>(
    loaded: &'a LoadedSourceMap,
    name: &str,
) -> Result<&'a ProjectSourceSet, DiscoveryError> {
    if name.trim().is_empty() || name.len() > 1024 || name.chars().any(char::is_control) {
        return Err(DiscoveryError::Operation(
            "invalid_source_set_name: sourceSet must contain stable non-blank bytes".into(),
        ));
    }
    loaded
        .source_sets
        .iter()
        .find(|source| source.name == name)
        .ok_or_else(|| DiscoveryError::Operation(format!("source_set_not_found: {name}")))
}

fn resolve_analysis_named(
    loaded: &LoadedSourceMap,
    name: &str,
) -> Result<ResolvedSourceSet, DiscoveryError> {
    let source = find_named(loaded, name)?;
    analysis_readiness(source)?;
    resolved(loaded, source).map_err(DiscoveryError::Operation)
}

fn resolve_mutation_named(
    loaded: &LoadedSourceMap,
    name: &str,
) -> Result<ResolvedSourceSet, DiscoveryError> {
    let source = find_named(loaded, name)?;
    if source.kind != SourceSetKind::Extension {
        return Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
            SourceReadinessReason::UnsupportedDestinationKind,
            SourceRole::Destination,
            &source.name,
        )));
    }
    if source.source_format != SourceFormat::PlatformXml {
        return Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
            SourceReadinessReason::UnsupportedDestinationFormat,
            SourceRole::Destination,
            &source.name,
        )));
    }
    resolved(loaded, source).map_err(DiscoveryError::Operation)
}

fn analysis_readiness(source: &ProjectSourceSet) -> Result<(), DiscoveryError> {
    if !matches!(
        source.kind,
        SourceSetKind::Configuration | SourceSetKind::Extension
    ) {
        return Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
            SourceReadinessReason::UnsupportedSourceKind,
            SourceRole::Analysis,
            &source.name,
        )));
    }
    match source.source_format {
        SourceFormat::PlatformXml => Ok(()),
        SourceFormat::Edt
            if source.kind == SourceSetKind::Configuration
                && source
                    .format_evidence
                    .iter()
                    .any(|evidence| !evidence.starts_with("v8project.yaml:")) =>
        {
            Ok(())
        }
        SourceFormat::Edt => Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
            SourceReadinessReason::UnsupportedSourceFormat,
            SourceRole::Analysis,
            &source.name,
        ))),
        SourceFormat::Unknown => Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
            SourceReadinessReason::UnknownSourceFormat,
            SourceRole::Analysis,
            &source.name,
        ))),
        SourceFormat::Invalid => Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
            SourceReadinessReason::InvalidSourceFormat,
            SourceRole::Analysis,
            &source.name,
        ))),
    }
}

fn resolved(
    loaded: &LoadedSourceMap,
    source: &ProjectSourceSet,
) -> Result<ResolvedSourceSet, String> {
    ResolvedSourceSet::new(
        source.name.clone(),
        source.kind,
        source.path.clone(),
        source.source_format,
        loaded.mapping_digest.clone(),
    )
}

fn load_source_map(
    workspace_root: &Path,
    require_existing_roots: bool,
) -> Result<LoadedSourceMap, String> {
    let canonical_workspace = canonical_workspace(workspace_root)?;
    let config_path = canonical_workspace.join("v8project.yaml");
    let (configured, configured_format_raw, actual_config_path) =
        match std::fs::symlink_metadata(&config_path) {
            Ok(metadata) => {
                if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
                    return Err(format!(
                        "source_map_config_not_regular: {}",
                        config_path.display()
                    ));
                }
                let bytes = read_stable_file(&canonical_workspace, &config_path)?;
                let (configured, format) = parse_configured_source_sets(&bytes)?;
                (configured, format, Some(config_path))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (Vec::new(), None, None),
            Err(error) => {
                return Err(format!(
                    "source_map_config_unavailable: {}: {error}",
                    config_path.display()
                ));
            }
        };

    let configured = if configured.is_empty() {
        autodetect_source_sets(&canonical_workspace)?
    } else {
        configured
    };
    validate_source_set_identities(&canonical_workspace, &configured, require_existing_roots)?;
    let mut source_sets = configured
        .iter()
        .map(|source| {
            detect_source_set_format(&canonical_workspace, source, require_existing_roots)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mapping_digest = mapping_digest(&source_sets)?;
    // Public map preserves configured order. Identity hashing canonicalizes it.
    if actual_config_path.is_none() {
        source_sets.sort_by_key(|source| source.name.to_lowercase());
    }
    Ok(LoadedSourceMap {
        canonical_workspace,
        config_path: actual_config_path,
        configured_format_raw,
        source_sets,
        mapping_digest,
    })
}

fn parse_configured_source_sets(
    bytes: &[u8],
) -> Result<(Vec<ConfiguredSourceSet>, Option<String>), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "source_map_config_invalid: v8project.yaml is not UTF-8")?;
    let yaml = serde_yaml::from_str::<YamlValue>(text)
        .map_err(|error| format!("source_map_config_invalid: {error}"))?;
    if !yaml.is_mapping() {
        return Err("source_map_config_invalid: root must be a mapping".into());
    }
    let configured_format_raw = optional_strict_string(&yaml, "format")?;
    let default_format = configured_format_raw
        .as_deref()
        .and_then(source_format_from_config);
    let base_path = optional_strict_string(&yaml, "basePath")?.unwrap_or_else(|| ".".into());
    validate_configured_relative_path(&base_path, "basePath")?;
    let mut source_sets = Vec::new();
    match yaml_mapping_get(&yaml, "source-set") {
        None | Some(YamlValue::Null) => {}
        Some(YamlValue::Sequence(entries)) => {
            for entry in entries {
                source_sets.push(config_source_set_from_yaml(
                    None,
                    entry,
                    &base_path,
                    default_format,
                )?);
            }
        }
        Some(YamlValue::Mapping(entries)) => {
            for (key, entry) in entries {
                let name = key.as_str().ok_or_else(|| {
                    "source_map_config_invalid: source-set mapping keys must be strings".to_string()
                })?;
                source_sets.push(config_source_set_from_yaml(
                    Some(name),
                    entry,
                    &base_path,
                    default_format,
                )?);
            }
        }
        Some(_) => {
            return Err("source_map_config_invalid: source-set must be a list or mapping".into())
        }
    }
    Ok((source_sets, configured_format_raw))
}

fn config_source_set_from_yaml(
    mapped_name: Option<&str>,
    entry: &YamlValue,
    base_path: &str,
    default_format: Option<SourceFormat>,
) -> Result<ConfiguredSourceSet, String> {
    if !entry.is_mapping() {
        return Err("source_map_config_invalid: source-set entries must be mappings".into());
    }
    let entry_name = optional_strict_string(entry, "name")?;
    if mapped_name.is_some() && entry_name.is_some() {
        return Err("source_map_config_invalid: mapped source-set must not repeat name".into());
    }
    let name = mapped_name
        .map(str::to_string)
        .or(entry_name)
        .unwrap_or_else(|| "main".into());
    validate_source_name(&name)?;
    let source_type = optional_strict_string(entry, "type")?;
    let purpose = optional_strict_string(entry, "purpose")?;
    if source_type.is_some() && purpose.is_some() && source_type != purpose {
        return Err("source_map_config_invalid: source-set type and purpose conflict".into());
    }
    let kind = source_set_kind_from_config(
        source_type
            .or(purpose)
            .as_deref()
            .unwrap_or("CONFIGURATION"),
    )?;
    let path = optional_strict_string(entry, "path")?.unwrap_or_else(|| ".".into());
    let relative_root = normalize_relative(base_path, &path)?;
    Ok(ConfiguredSourceSet {
        name,
        kind,
        relative_root,
        default_format,
    })
}

fn validate_source_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.len() > 1024 || name.chars().any(char::is_control) {
        return Err("invalid_source_set_name: name must contain stable non-blank bytes".into());
    }
    Ok(())
}

fn optional_strict_string(value: &YamlValue, key: &str) -> Result<Option<String>, String> {
    match yaml_mapping_get(value, key) {
        None => Ok(None),
        Some(YamlValue::String(text)) if !text.is_empty() => Ok(Some(text.clone())),
        Some(YamlValue::String(_)) => Err(format!(
            "source_map_config_invalid: `{key}` must not be empty"
        )),
        Some(_) => Err(format!(
            "source_map_config_invalid: field `{key}` must be a string"
        )),
    }
}

fn yaml_mapping_get<'a>(value: &'a YamlValue, key: &str) -> Option<&'a YamlValue> {
    value.as_mapping()?.get(YamlValue::String(key.to_string()))
}

fn validate_source_set_identities(
    workspace: &Path,
    source_sets: &[ConfiguredSourceSet],
    require_existing_roots: bool,
) -> Result<(), String> {
    let mut names = BTreeSet::new();
    let mut roots = BTreeMap::new();
    for source in source_sets {
        if !names.insert(source.name.to_lowercase()) {
            return Err(format!("duplicate_source_set_name: {}", source.name));
        }
        let configured_root = workspace.join(&source.relative_root);
        let allow_missing = !require_existing_roots && !configured_root.exists();
        let canonical_root = if allow_missing {
            configured_root
        } else {
            resolve_contained_directory(workspace, &source.relative_root)?
        };
        if let Some(previous) = roots.insert(canonical_root.clone(), source.name.clone()) {
            return Err(format!(
                "duplicate_source_root: {} and {} resolve to {}",
                previous,
                source.name,
                canonical_root.display()
            ));
        }
    }
    Ok(())
}

fn autodetect_source_sets(workspace: &Path) -> Result<Vec<ConfiguredSourceSet>, String> {
    for relative_root in [".", "src", "src/cf"] {
        let root = if relative_root == "." {
            workspace.to_path_buf()
        } else {
            workspace.join(relative_root)
        };
        if !root.is_dir() {
            continue;
        }
        if regular_marker(&root.join("Configuration.xml"))?
            || regular_marker(&root.join("Configuration/Configuration.mdo"))?
            || regular_marker(&root.join("src/Configuration/Configuration.mdo"))?
        {
            return Ok(vec![ConfiguredSourceSet {
                name: "main".into(),
                kind: SourceSetKind::Configuration,
                relative_root: relative_root.into(),
                default_format: None,
            }]);
        }
    }
    Ok(Vec::new())
}

fn detect_source_set_format(
    workspace: &Path,
    configured: &ConfiguredSourceSet,
    require_existing_roots: bool,
) -> Result<ProjectSourceSet, String> {
    let configured_root = workspace.join(&configured.relative_root);
    let allow_missing = !require_existing_roots && !configured_root.exists();
    let root = if allow_missing {
        configured_root
    } else {
        resolve_contained_directory(workspace, &configured.relative_root)?
    };
    let mut platform_evidence = Vec::new();
    let configuration = root.join("Configuration.xml");
    if regular_marker(&configuration)? {
        platform_evidence.push(slash_relative(workspace, &configuration)?);
    }
    if matches!(
        configured.kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) && root.is_dir()
    {
        for entry in std::fs::read_dir(&root)
            .map_err(|error| format!("source_root_unreadable: {}: {error}", root.display()))?
        {
            let entry = entry
                .map_err(|error| format!("source_root_unreadable: {}: {error}", root.display()))?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) == Some("xml")
                && entry.file_name() != "ConfigDumpInfo.xml"
                && regular_marker(&path)?
            {
                platform_evidence.push(slash_relative(workspace, &path)?);
            }
        }
    }
    let mut edt_evidence = Vec::new();
    for relative in [
        ".project",
        "DT-INF/PROJECT.PMF",
        "Configuration/Configuration.mdo",
        "src/Configuration/Configuration.mdo",
    ] {
        let path = root.join(relative);
        if regular_marker(&path)? {
            edt_evidence.push(slash_relative(workspace, &path)?);
        }
    }
    platform_evidence.sort();
    platform_evidence.dedup();
    edt_evidence.sort();
    edt_evidence.dedup();
    let source_format = match (platform_evidence.is_empty(), edt_evidence.is_empty()) {
        (false, false) => SourceFormat::Invalid,
        (false, true) => SourceFormat::PlatformXml,
        (true, false) => SourceFormat::Edt,
        (true, true) => configured.default_format.unwrap_or(SourceFormat::Unknown),
    };
    let mut format_evidence = platform_evidence;
    format_evidence.extend(edt_evidence);
    if format_evidence.is_empty() {
        if let Some(default) = configured.default_format {
            format_evidence.push(match default {
                SourceFormat::PlatformXml => "v8project.yaml:format=DESIGNER".into(),
                SourceFormat::Edt => "v8project.yaml:format=EDT".into(),
                SourceFormat::Unknown | SourceFormat::Invalid => "v8project.yaml:format".into(),
            });
        }
    }
    Ok(ProjectSourceSet {
        name: configured.name.clone(),
        kind: configured.kind,
        path: configured.relative_root.clone(),
        source_format,
        format_evidence,
    })
}

fn mapping_digest(source_sets: &[ProjectSourceSet]) -> Result<String, String> {
    let mut topology = source_sets.iter().collect::<Vec<_>>();
    topology.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.name.cmp(&right.name))
    });
    let mut hasher = Sha256::new();
    write_hash_bytes(&mut hasher, MAPPING_DOMAIN)?;
    write_hash_u64(&mut hasher, topology.len() as u64);
    for source in topology {
        write_hash_bytes(&mut hasher, source.name.as_bytes())?;
        hasher.update([
            source_kind_tag(source.kind),
            source_format_tag(source.source_format),
        ]);
        write_hash_bytes(&mut hasher, source.path.as_bytes())?;
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn write_hash_bytes(hasher: &mut Sha256, bytes: &[u8]) -> Result<(), String> {
    let length = u64::try_from(bytes.len()).map_err(|_| "mapping value too large")?;
    write_hash_u64(hasher, length);
    hasher.update(bytes);
    Ok(())
}

fn write_hash_u64(hasher: &mut Sha256, value: u64) {
    hasher.update(value.to_be_bytes());
}

fn source_kind_tag(kind: SourceSetKind) -> u8 {
    match kind {
        SourceSetKind::Configuration => 1,
        SourceSetKind::Extension => 2,
        SourceSetKind::ExternalProcessor => 3,
        SourceSetKind::ExternalReport => 4,
    }
}

fn source_format_tag(format: SourceFormat) -> u8 {
    match format {
        SourceFormat::PlatformXml => 1,
        SourceFormat::Edt => 2,
        SourceFormat::Unknown => 3,
        SourceFormat::Invalid => 4,
    }
}

fn source_set_kind_from_config(raw: &str) -> Result<SourceSetKind, String> {
    match raw.to_ascii_uppercase().as_str() {
        "CONFIGURATION" => Ok(SourceSetKind::Configuration),
        "EXTENSION" => Ok(SourceSetKind::Extension),
        "EXTERNAL_DATA_PROCESSORS" => Ok(SourceSetKind::ExternalProcessor),
        "EXTERNAL_REPORTS" => Ok(SourceSetKind::ExternalReport),
        other => Err(format!("unsupported_source_set_type: {other}")),
    }
}

fn source_format_from_config(raw: &str) -> Option<SourceFormat> {
    match raw.to_ascii_uppercase().as_str() {
        "DESIGNER" | "PLATFORM_XML" | "XML" => Some(SourceFormat::PlatformXml),
        "EDT" => Some(SourceFormat::Edt),
        _ => None,
    }
}

fn regular_marker(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata_is_link_or_reparse_point(&metadata) => {
            Err(format!("symlink_or_reparse_marker: {}", path.display()))
        }
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("marker_unavailable: {}: {error}", path.display())),
    }
}

fn read_stable_file(workspace: &Path, path: &Path) -> Result<Vec<u8>, String> {
    reject_link_components(workspace, path)?;
    let path_before = std::fs::symlink_metadata(path)
        .map_err(|error| format!("source_map_config_unavailable: {}: {error}", path.display()))?;
    if metadata_is_link_or_reparse_point(&path_before) || !path_before.is_file() {
        return Err(format!("source_map_config_not_regular: {}", path.display()));
    }
    #[cfg(unix)]
    let before = observe_regular_file(&path_before, path)?;
    let before_length = path_before.len();
    let mut contained = open_no_follow(workspace, path)?;
    let opened = observe_open_file(contained.file(), path)?;
    #[cfg(unix)]
    if before != opened {
        return Err("source_mapping_changed: source map changed during resolution".into());
    }
    #[cfg(windows)]
    if before_length != opened.length {
        return Err("source_mapping_changed: source map changed during resolution".into());
    }
    let capacity = usize::try_from(before_length.min(64 * 1024))
        .map_err(|_| "source_map_config_too_large: cannot address file")?;
    let mut bytes = Vec::with_capacity(capacity);
    let read_limit = before_length
        .checked_add(1)
        .ok_or("source_map_config_too_large: length overflow")?;
    contained
        .file_mut()
        .take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("source_map_config_unavailable: {}: {error}", path.display()))?;
    if bytes.len() as u64 > before_length {
        return Err("source_mapping_changed: source map changed during resolution".into());
    }
    let after_handle = observe_open_file(contained.file(), path)?;
    contained.validate_after_read()?;
    #[cfg(unix)]
    let after_path = observe_regular_file(
        &std::fs::symlink_metadata(path).map_err(|error| {
            format!("source_map_config_unavailable: {}: {error}", path.display())
        })?,
        path,
    )?;
    #[cfg(windows)]
    let after_path = {
        let reopened = open_no_follow(workspace, path)?;
        let observation = observe_open_file(reopened.file(), path)?;
        reopened.validate_after_read()?;
        observation
    };
    #[cfg(unix)]
    let baseline = before;
    #[cfg(windows)]
    let baseline = opened;
    if baseline != after_handle || baseline != after_path || bytes.len() as u64 != before_length {
        return Err("source_mapping_changed: source map changed during resolution".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn legacy_map_preserves_external_detection_and_edt_analysis_readiness() {
        let root = fixture("source-map-legacy");
        write(
            &root.join("v8project.yaml"),
            "format: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: edt\n  - name: epf\n    type: EXTERNAL_DATA_PROCESSORS\n    path: epf\n",
        );
        write(&root.join("edt/.project"), "x");
        write(&root.join("epf/Tool.xml"), "x");
        let map = discover_project_source_map(&root).unwrap();
        assert_eq!(map.source_sets[0].source_format, SourceFormat::Edt);
        assert_eq!(map.source_sets[1].kind, SourceSetKind::ExternalProcessor);
        let edt = resolve_source_selection_typed(&root, Some("main"), &[]).unwrap();
        assert_eq!(edt.analysis.source_format, SourceFormat::Edt);
        let external = resolve_source_selection_typed(&root, Some("epf"), &[]).unwrap_err();
        let DiscoveryError::SourceReadiness(external) = external else {
            panic!("expected typed readiness error");
        };
        assert_eq!(external.reason_code(), "unsupported_source_kind");
        assert_eq!(external.role, SourceRole::Analysis);
        assert!(!external.retryable());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_dump_info_alone_does_not_prove_platform_xml_format() {
        let root = fixture("source-map-config-dump-info-only");
        write(
            &root.join("v8project.yaml"),
            "source-set:\n - { name: main, type: CONFIGURATION, path: main }\n",
        );
        write(&root.join("main/ConfigDumpInfo.xml"), "x");

        let map = discover_project_source_map(&root).unwrap();
        assert_eq!(map.source_sets[0].source_format, SourceFormat::Unknown);
        assert!(map.source_sets[0].format_evidence.is_empty());

        write(&root.join("main/Configuration.xml"), "x");
        let map = discover_project_source_map(&root).unwrap();
        assert_eq!(map.source_sets[0].source_format, SourceFormat::PlatformXml);
        assert_eq!(
            map.source_sets[0].format_evidence,
            vec!["main/Configuration.xml"]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn external_config_dump_info_alone_does_not_prove_platform_xml_format() {
        let root = fixture("source-map-external-config-dump-info-only");
        write(
            &root.join("v8project.yaml"),
            "source-set:\n - { name: tool, type: EXTERNAL_DATA_PROCESSORS, path: tool }\n",
        );
        write(&root.join("tool/ConfigDumpInfo.xml"), "x");

        let map = discover_project_source_map(&root).unwrap();
        assert_eq!(map.source_sets[0].source_format, SourceFormat::Unknown);
        assert!(map.source_sets[0].format_evidence.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_duplicate_names_absolute_traversal_empty_and_duplicate_roots() {
        let cases = [
            (
                "source-set:\n - { name: Main, path: a }\n - { name: main, path: b }\n",
                "duplicate_source_set_name",
            ),
            (
                "source-set:\n - { name: main, path: /tmp }\n",
                "absolute_source_root",
            ),
            (
                "source-set:\n - { name: main, path: ../a }\n",
                "path_traversal",
            ),
            (
                "source-set:\n - { name: main, path: '' }\n",
                "must not be empty",
            ),
            (
                "source-set:\n - { name: one }\n - { name: two, path: . }\n",
                "duplicate_source_root",
            ),
        ];
        for (index, (yaml, reason)) in cases.iter().enumerate() {
            let root = fixture(&format!("source-map-invalid-{index}"));
            fs::create_dir_all(root.join("a")).unwrap();
            fs::create_dir_all(root.join("b")).unwrap();
            write(&root.join("v8project.yaml"), yaml);
            let error = discover_project_source_map(&root).unwrap_err();
            assert!(error.contains(reason), "expected {reason}, got {error}");
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn mapping_digest_is_semantic_and_batch_resolution_is_canonical() {
        let root = fixture("source-map-semantic-digest");
        fs::create_dir_all(root.join("main")).unwrap();
        fs::create_dir_all(root.join("ext")).unwrap();
        write(&root.join("main/Configuration.xml"), "x");
        write(&root.join("ext/Configuration.xml"), "x");
        write(&root.join("v8project.yaml"), "# comment\ninfobase: ignored\nformat: DESIGNER\nsource-set:\n - { name: main, type: CONFIGURATION, path: main }\n - { name: Extension, type: EXTENSION, path: ext }\n");
        let before = resolve_source_selection(&root, Some("main"), &["Extension".into()]).unwrap();
        write(&root.join("v8project.yaml"), "source-set:\n - { path: ext, type: EXTENSION, name: Extension }\n - { path: main, name: main, type: CONFIGURATION }\nformat: DESIGNER\nother: value\n");
        let reordered = resolve_source_selection(
            &root,
            Some("main"),
            &["Extension".into(), "Extension".into()],
        )
        .unwrap();
        assert_eq!(
            before.analysis.mapping_digest,
            reordered.analysis.mapping_digest
        );
        assert_eq!(reordered.mutations.len(), 1);
        write(&root.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n - { name: renamed, type: CONFIGURATION, path: main }\n - { name: Extension, type: EXTENSION, path: ext }\n");
        let changed =
            resolve_source_selection(&root, Some("renamed"), &["Extension".into()]).unwrap();
        assert_ne!(
            before.analysis.mapping_digest,
            changed.analysis.mapping_digest
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn auto_selection_requires_exactly_one_eligible_platform_xml_set() {
        let root = fixture("source-map-ambiguous");
        for dir in ["main", "ext"] {
            fs::create_dir_all(root.join(dir)).unwrap();
            write(&root.join(dir).join("Configuration.xml"), "x");
        }
        write(&root.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n - { name: main, type: CONFIGURATION, path: main }\n - { name: ext, type: EXTENSION, path: ext }\n");
        assert!(resolve_source_selection(&root, None, &[])
            .unwrap_err()
            .contains("ambiguous_source_set"));
        assert_eq!(
            resolve_source_selection(&root, Some("main"), &[])
                .unwrap()
                .analysis
                .relative_root,
            "main"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn destination_role_requires_platform_xml_extension() {
        let root = fixture("source-map-destination-role");
        for dir in ["main", "edt-ext"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        write(&root.join("main/Configuration.xml"), "x");
        write(&root.join("edt-ext/.project"), "x");
        write(&root.join("v8project.yaml"), "source-set:\n - { name: main, type: CONFIGURATION, path: main }\n - { name: edt, type: EXTENSION, path: edt-ext }\n");
        for (name, code) in [
            ("main", "unsupported_destination_kind"),
            ("edt", "unsupported_destination_format"),
        ] {
            let error =
                resolve_source_selection_typed(&root, Some("main"), &[name.into()]).unwrap_err();
            let DiscoveryError::SourceReadiness(error) = error else {
                panic!("expected typed readiness error")
            };
            assert_eq!(error.reason_code(), code);
            assert_eq!(error.role, SourceRole::Destination);
            assert!(!error.retryable());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn edt_analysis_still_resolves_and_validates_requested_destinations() {
        let root = fixture("source-map-edt-with-destinations");
        for directory in ["edt", "valid-ext", "invalid-ext"] {
            fs::create_dir_all(root.join(directory)).unwrap();
        }
        write(&root.join("edt/.project"), "x");
        write(&root.join("valid-ext/Configuration.xml"), "x");
        write(&root.join("invalid-ext/.project"), "x");
        write(
            &root.join("v8project.yaml"),
            "source-set:\n - { name: edt, type: CONFIGURATION, path: edt }\n - { name: valid, type: EXTENSION, path: valid-ext }\n - { name: invalid, type: EXTENSION, path: invalid-ext }\n",
        );

        let selection =
            resolve_source_selection_typed(&root, Some("edt"), &["valid".into()]).unwrap();
        assert_eq!(selection.analysis.source_format, SourceFormat::Edt);
        assert_eq!(selection.mutations.len(), 1);
        assert_eq!(selection.mutations[0].name, "valid");

        let error =
            resolve_source_selection_typed(&root, Some("edt"), &["invalid".into()]).unwrap_err();
        let DiscoveryError::SourceReadiness(error) = error else {
            panic!("expected typed destination readiness error")
        };
        assert_eq!(error.reason_code(), "unsupported_destination_format");
        assert_eq!(error.role, SourceRole::Destination);
        assert!(!error.retryable());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn markerless_declared_edt_is_typed_unsupported_analysis_format() {
        let root = fixture("source-map-markerless-edt");
        fs::create_dir_all(root.join("edt")).unwrap();
        write(
            &root.join("v8project.yaml"),
            "format: EDT\nsource-set:\n - { name: main, type: CONFIGURATION, path: edt }\n",
        );

        let error = resolve_source_selection_typed(&root, Some("main"), &[]).unwrap_err();
        let DiscoveryError::SourceReadiness(error) = error else {
            panic!("expected typed readiness error");
        };
        assert_eq!(error.reason_code(), "unsupported_source_format");
        assert_eq!(error.role, SourceRole::Analysis);
        assert!(!error.retryable());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_source_root() {
        use std::os::unix::fs::symlink;
        let root = fixture("source-map-symlink");
        fs::create_dir_all(root.join("real")).unwrap();
        symlink(root.join("real"), root.join("linked")).unwrap();
        write(
            &root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n - { name: main, path: linked }\n",
        );
        assert!(discover_project_source_map(&root)
            .unwrap_err()
            .contains("symlink"));
        fs::remove_dir_all(root).unwrap();
    }

    fn fixture(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("unica-{name}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }
}
