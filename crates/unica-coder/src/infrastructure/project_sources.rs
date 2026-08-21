use crate::domain::project_sources::{
    config_dump_info_xml_kind, ConfigDumpInfoXmlKind, ProjectSourceMap, ProjectSourceSet,
    SourceFormat, SourceSetKind,
};
use crate::domain::source_roots::select_default_source_set;
use crate::infrastructure::native_operations::compile_transaction::{
    CompileTransaction, DirectoryMembershipSelector, DirectoryMembershipSnapshot,
    DirectoryTopologyEntry, DirectoryTopologyEntryKind,
};
use crate::infrastructure::platform::filesystem::{
    host_path_text, is_link_loop_error, open_absolute_directory_path_nofollow,
    open_any_child_nofollow, open_child_for_secure_tree_use, open_directory_child_nofollow,
    open_regular_child_nofollow, read_directory_names_bounded, OpenedChildKind,
};
use crate::infrastructure::source_roots::{
    inspect_declared_source_root_route, normalize_contained_source_root, normalize_path_identity,
};
use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_yaml::Value as YamlValue;
use std::collections::BTreeMap;
use std::ffi::CStr;
use std::fmt;
use std::io::Read;
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};
use unsafe_libyaml::{
    yaml_event_delete, yaml_event_t, yaml_parser_delete, yaml_parser_initialize, yaml_parser_parse,
    yaml_parser_set_input_string, yaml_parser_t, YAML_ALIAS_EVENT, YAML_DOCUMENT_START_EVENT,
    YAML_MAPPING_END_EVENT, YAML_MAPPING_START_EVENT, YAML_SCALAR_EVENT, YAML_SEQUENCE_END_EVENT,
    YAML_SEQUENCE_START_EVENT, YAML_STREAM_END_EVENT,
};

const MAX_RESERVED_EXTERNAL_DESCRIPTOR_BYTES: u64 = 8 * 1024 * 1024;
const MAX_HEALTH_SOURCE_MAP_INPUT_BYTES: u64 = 8 * 1024 * 1024;
const PROJECT_SOURCE_MAP_READ_CHUNK_BYTES: usize = 64 * 1024;
const MAX_HEALTH_SOURCE_SETS: usize = 1024;
const MAX_HEALTH_SOURCE_SET_VALUE_BYTES: usize = 2 * 1024 * 1024;
const MAX_HEALTH_YAML_RETAINED_BYTES: usize = 2 * 1024 * 1024;
const MAX_HEALTH_YAML_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_HEALTH_YAML_DOCUMENT_NODES: usize = 64 * 1024;
const MAX_HEALTH_YAML_DOCUMENT_DEPTH: usize = 256;
const MAX_HEALTH_FORMAT_EVIDENCE_ENTRIES: usize = 16 * 1024;
const MAX_HEALTH_FORMAT_EVIDENCE_BYTES: usize = 4 * 1024 * 1024;
/// The source-set name the base configuration owns. `INV-SOURCE-SINGLE-RESOLVED-ROOT`
/// makes it the deterministic winner of default selection, so exactly one entry may
/// carry it.
const BASE_CONFIGURATION_SOURCE_SET: &str = "main";

/// Every marker that proves a directory is a source root, in both dump formats Unica
/// reads. Autodetection probes this one list wherever it looks: a layout that
/// recognised fewer markers than another would find a configuration in one place and
/// miss the identical one in the next.
const SOURCE_ROOT_MARKERS: &[&str] = &[
    "Configuration.xml",
    "Configuration/Configuration.mdo",
    "src/Configuration/Configuration.mdo",
];

/// Where the base configuration is autodetected, in probe order. The first directory
/// carrying a marker wins and stops the scan: the rest of the tree below it belongs to
/// that configuration, not to a second one.
const BASE_CONFIGURATION_LAYOUTS: &[&str] = &[".", "src", "src/cf"];

/// Where configuration extensions are autodetected.
///
/// Autodetection is a closed catalog, not a growing pile of probes: every place Unica
/// is willing to find a source root without `v8project.yaml` is listed here once, and
/// both discovery routes read the same list. Both entries are layouts this repository
/// already documents — `src/cfe` is one extension's dump root in the `cfe-*` skills and
/// a container of named extensions in a multi-extension workspace, while
/// `src/extensions` is only ever a container.
const EXTENSION_LAYOUTS: &[ExtensionLayout] = &[
    ExtensionLayout {
        directory: "src/cfe",
        shape: ExtensionLayoutShape::RootOrContainer,
    },
    ExtensionLayout {
        directory: "src/extensions",
        shape: ExtensionLayoutShape::Container,
    },
];

struct ExtensionLayout {
    /// Workspace-relative directory this layout inspects.
    directory: &'static str,
    shape: ExtensionLayoutShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExtensionLayoutShape {
    /// The directory is either one extension's dump root, or a container holding one
    /// extension per direct child directory.
    RootOrContainer,
    /// The directory only ever holds one extension per direct child directory.
    Container,
}

/// A directory autodetection probes, on whichever route discovery is running.
///
/// The hardened route must not resolve a path twice or follow a link, so it carries the
/// directory it already opened; the ordinary route probes by path and records every
/// probed path in provenance. Keeping both behind one type is what lets the layout
/// catalog, the marker list and the entry classification be written once instead of
/// once per route — the three places the two routes had already drifted apart.
enum ProbeDirectory {
    Path(PathBuf),
    Retained(std::fs::File),
}
const EDT_SOURCE_MARKERS: &[&str] = &[
    ".project",
    "DT-INF/PROJECT.PMF",
    "Configuration/Configuration.mdo",
    "src/Configuration/Configuration.mdo",
];

#[derive(Debug, Clone)]
struct ConfigSourceSet {
    name: String,
    kind: SourceSetKind,
    path: String,
    default_format: Option<SourceFormat>,
}

#[derive(Debug, Default)]
struct BoundedHealthConfig {
    format: Option<YamlValue>,
    base_path: Option<YamlValue>,
    source_sets: BoundedHealthSourceSets,
}

#[derive(Debug, Default)]
enum BoundedHealthSourceSets {
    #[default]
    Empty,
    Sequence(Vec<YamlValue>),
    Mapping(Vec<(YamlValue, YamlValue)>),
}

struct BoundedHealthSourceSetsVisitor;

impl<'de> Visitor<'de> for BoundedHealthSourceSetsVisitor {
    type Value = BoundedHealthSourceSets;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a source-set list, mapping, or null")
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(BoundedHealthSourceSets::Empty)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(BoundedHealthSourceSets::Empty)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        let mut retained_bytes = 0_usize;
        while let Some(value) = sequence.next_element::<YamlValue>()? {
            if values.len() == MAX_HEALTH_SOURCE_SETS {
                return Err(serde::de::Error::custom(format!(
                    "project source-map declares more than {MAX_HEALTH_SOURCE_SETS} source sets"
                )));
            }
            retained_bytes = retained_bytes.saturating_add(yaml_value_retained_bytes(&value));
            if retained_bytes > MAX_HEALTH_SOURCE_SET_VALUE_BYTES {
                return Err(serde::de::Error::custom(format!(
                    "expanded source-set values exceed {MAX_HEALTH_SOURCE_SET_VALUE_BYTES} retained bytes"
                )));
            }
            values.push(value);
        }
        Ok(BoundedHealthSourceSets::Sequence(values))
    }

    fn visit_map<A>(self, mut mapping: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Vec::new();
        let mut retained_bytes = 0_usize;
        while let Some((key, value)) = mapping.next_entry::<YamlValue, YamlValue>()? {
            if values.len() == MAX_HEALTH_SOURCE_SETS {
                return Err(serde::de::Error::custom(format!(
                    "project source-map declares more than {MAX_HEALTH_SOURCE_SETS} source sets"
                )));
            }
            retained_bytes = retained_bytes
                .saturating_add(yaml_value_retained_bytes(&key))
                .saturating_add(yaml_value_retained_bytes(&value));
            if retained_bytes > MAX_HEALTH_SOURCE_SET_VALUE_BYTES {
                return Err(serde::de::Error::custom(format!(
                    "expanded source-set values exceed {MAX_HEALTH_SOURCE_SET_VALUE_BYTES} retained bytes"
                )));
            }
            values.push((key, value));
        }
        Ok(BoundedHealthSourceSets::Mapping(values))
    }
}

fn yaml_value_retained_bytes(value: &YamlValue) -> usize {
    match value {
        YamlValue::Null | YamlValue::Bool(_) | YamlValue::Number(_) => 16,
        YamlValue::String(value) => value.len(),
        YamlValue::Sequence(values) => values.iter().fold(0_usize, |total, value| {
            total.saturating_add(yaml_value_retained_bytes(value))
        }),
        YamlValue::Mapping(values) => values.iter().fold(0_usize, |total, (key, value)| {
            total
                .saturating_add(yaml_value_retained_bytes(key))
                .saturating_add(yaml_value_retained_bytes(value))
        }),
        YamlValue::Tagged(value) => yaml_value_retained_bytes(&value.value),
    }
}

impl<'de> Deserialize<'de> for BoundedHealthSourceSets {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(BoundedHealthSourceSetsVisitor)
    }
}

struct BoundedHealthConfigVisitor;

impl<'de> Visitor<'de> for BoundedHealthConfigVisitor {
    type Value = BoundedHealthConfig;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a v8project.yaml mapping")
    }

    fn visit_map<A>(self, mut mapping: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut result = BoundedHealthConfig::default();
        let mut seen = BTreeMap::<String, ()>::new();
        while let Some(key) = mapping.next_key::<String>()? {
            if matches!(key.as_str(), "format" | "basePath" | "source-set")
                && seen.insert(key.clone(), ()).is_some()
            {
                return Err(serde::de::Error::custom(format!(
                    "duplicate top-level field `{key}`"
                )));
            }
            match key.as_str() {
                "format" => result.format = Some(mapping.next_value::<YamlValue>()?),
                "basePath" => result.base_path = mapping.next_value::<Option<YamlValue>>()?,
                "source-set" => result.source_sets = mapping.next_value()?,
                _ => {
                    mapping.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(result)
    }
}

impl<'de> Deserialize<'de> for BoundedHealthConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(BoundedHealthConfigVisitor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProjectSourceMapInput {
    ExactFile(Vec<u8>),
    Absent,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ProjectSourceMapProvenance {
    inputs: BTreeMap<PathBuf, ProjectSourceMapInput>,
    /// Directories whose *listing* discovery consumed. Autodetection derives source-set
    /// names from the filesystem, so which direct children of an extension container are
    /// directories is an input in its own right: a concurrent checkout that adds or
    /// removes `src/cfe/<name>` changes the computed map while every recorded file input
    /// still agrees, and the transaction would otherwise commit against a stale map.
    directories: BTreeMap<PathBuf, DirectoryMembershipSnapshot>,
}

struct SourceMapDiscoveryState {
    provenance: Option<ProjectSourceMapProvenance>,
    max_source_sets: Option<usize>,
    max_input_bytes: Option<u64>,
    format_evidence_entry_limit: Option<usize>,
    format_evidence_byte_limit: Option<usize>,
    remaining_format_evidence_entries: Option<usize>,
    remaining_format_evidence_bytes: Option<usize>,
    require_nofollow_source_probe_route: bool,
}

impl SourceMapDiscoveryState {
    fn ordinary() -> Self {
        Self {
            provenance: None,
            max_source_sets: None,
            max_input_bytes: None,
            format_evidence_entry_limit: None,
            format_evidence_byte_limit: None,
            remaining_format_evidence_entries: None,
            remaining_format_evidence_bytes: None,
            require_nofollow_source_probe_route: false,
        }
    }

    fn health() -> Self {
        Self {
            provenance: None,
            max_source_sets: Some(MAX_HEALTH_SOURCE_SETS),
            max_input_bytes: Some(MAX_HEALTH_SOURCE_MAP_INPUT_BYTES),
            format_evidence_entry_limit: Some(MAX_HEALTH_FORMAT_EVIDENCE_ENTRIES),
            format_evidence_byte_limit: Some(MAX_HEALTH_FORMAT_EVIDENCE_BYTES),
            remaining_format_evidence_entries: Some(MAX_HEALTH_FORMAT_EVIDENCE_ENTRIES),
            remaining_format_evidence_bytes: Some(MAX_HEALTH_FORMAT_EVIDENCE_BYTES),
            require_nofollow_source_probe_route: true,
        }
    }

    fn transactional() -> Self {
        Self {
            provenance: Some(ProjectSourceMapProvenance::default()),
            max_source_sets: None,
            max_input_bytes: None,
            format_evidence_entry_limit: None,
            format_evidence_byte_limit: None,
            remaining_format_evidence_entries: None,
            remaining_format_evidence_bytes: None,
            require_nofollow_source_probe_route: false,
        }
    }

    #[cfg(test)]
    fn health_with_evidence_limits(entries: usize, bytes: usize) -> Self {
        Self {
            format_evidence_entry_limit: Some(entries),
            format_evidence_byte_limit: Some(bytes),
            remaining_format_evidence_entries: Some(entries),
            remaining_format_evidence_bytes: Some(bytes),
            ..Self::health()
        }
    }

    #[cfg(test)]
    fn health_with_source_set_limit(limit: usize) -> Self {
        Self {
            max_source_sets: Some(limit),
            ..Self::health()
        }
    }

    fn captures_provenance(&self) -> bool {
        self.provenance.is_some()
    }

    fn record_exact_file(&mut self, path: PathBuf, raw: &[u8]) -> Result<(), String> {
        if let Some(provenance) = &mut self.provenance {
            provenance.record_exact_file(path, raw.to_vec())?;
        }
        Ok(())
    }

    fn record_absence(&mut self, path: PathBuf) -> Result<(), String> {
        if let Some(provenance) = &mut self.provenance {
            provenance.record_absence(path)?;
        }
        Ok(())
    }

    fn record_directory_membership(
        &mut self,
        path: PathBuf,
        snapshot: DirectoryMembershipSnapshot,
    ) -> Result<(), String> {
        if let Some(provenance) = &mut self.provenance {
            provenance.record_directory_membership(path, snapshot)?;
        }
        Ok(())
    }

    fn record_format_evidence(&mut self, evidence: &str) -> Result<(), String> {
        if let Some(remaining) = &mut self.remaining_format_evidence_entries {
            if *remaining == 0 {
                return Err(format!(
                    "project source-map format evidence exceeds {} entries",
                    self.format_evidence_entry_limit
                        .expect("bounded evidence entries have an initial limit")
                ));
            }
            *remaining -= 1;
        }
        if let Some(remaining) = &mut self.remaining_format_evidence_bytes {
            let bytes = evidence.len();
            if bytes > *remaining {
                return Err(format!(
                    "project source-map format evidence exceeds {} bytes",
                    self.format_evidence_byte_limit
                        .expect("bounded evidence bytes have an initial limit")
                ));
            }
            *remaining -= bytes;
        }
        Ok(())
    }
}

impl ProjectSourceMapProvenance {
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
        for (directory, expected) in &self.directories {
            transaction.guard_or_verify_directory_membership(
                directory,
                DirectoryMembershipSelector::DirectChildDirectories,
                expected.clone(),
            )?;
        }
        Ok(())
    }

    fn record_directory_membership(
        &mut self,
        path: PathBuf,
        snapshot: DirectoryMembershipSnapshot,
    ) -> Result<(), String> {
        match self.directories.get(&path) {
            Some(existing) if existing == &snapshot => Ok(()),
            Some(_) => Err(format!(
                "project source-map input changed while resolving: {}",
                path.display()
            )),
            None => {
                self.directories.insert(path, snapshot);
                Ok(())
            }
        }
    }

    fn record_exact_file(&mut self, path: PathBuf, raw: Vec<u8>) -> Result<(), String> {
        match self.inputs.get(&path) {
            Some(ProjectSourceMapInput::ExactFile(existing)) if existing == &raw => Ok(()),
            Some(_) => Err(format!(
                "project source-map input changed while resolving: {}",
                path.display()
            )),
            None => {
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
    let mut checkpoint = || Ok(());
    discover_project_source_map_internal(
        workspace_root,
        &mut checkpoint,
        &mut SourceMapDiscoveryState::ordinary(),
    )
}

pub(crate) fn discover_project_source_map_controlled(
    workspace_root: &Path,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<ProjectSourceMap, String> {
    discover_project_source_map_internal(
        workspace_root,
        checkpoint,
        &mut SourceMapDiscoveryState::health(),
    )
}

pub(crate) fn discover_project_source_map_with_provenance(
    workspace_root: &Path,
) -> Result<(ProjectSourceMap, ProjectSourceMapProvenance), String> {
    let mut checkpoint = || Ok(());
    let mut state = SourceMapDiscoveryState::transactional();
    let source_map =
        discover_project_source_map_internal(workspace_root, &mut checkpoint, &mut state)?;
    Ok((
        source_map,
        state
            .provenance
            .expect("transactional source-map discovery captures provenance"),
    ))
}

fn discover_project_source_map_internal(
    workspace_root: &Path,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
    state: &mut SourceMapDiscoveryState,
) -> Result<ProjectSourceMap, String> {
    checkpoint()?;
    let project_config = workspace_root.join("v8project.yaml");
    let project_config_raw = snapshot_project_map_input(&project_config, true, state, checkpoint)?;
    let config_path = project_config_raw.as_ref().map(|_| project_config.clone());
    let (mut source_sets, configured_format_raw) = if let Some(raw) = &project_config_raw {
        read_config_source_sets(
            workspace_root,
            &project_config,
            raw,
            state.max_source_sets,
            checkpoint,
        )?
    } else {
        (Vec::new(), None)
    };

    if source_sets.is_empty() {
        source_sets = autodetect_source_sets(workspace_root, state, checkpoint)?;
    }

    let mut project_source_sets = Vec::with_capacity(source_sets.len());
    for source_set in source_sets {
        checkpoint()?;
        project_source_sets.push(detect_source_set_format(
            workspace_root,
            source_set,
            state,
            checkpoint,
        )?);
    }
    checkpoint()?;
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

    Ok(ProjectSourceMap {
        workspace_root: workspace_root.display().to_string(),
        config_path: config_path.map(|path| path.display().to_string()),
        source_sets: project_source_sets,
        effective_source_set,
        effective_source_root,
        source_selection_error,
        configured_format_raw,
    })
}

fn read_config_source_sets(
    workspace_root: &Path,
    config_path: &Path,
    raw: &[u8],
    max_source_sets: Option<usize>,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<(Vec<ConfigSourceSet>, Option<String>), String> {
    checkpoint()?;
    let text = std::str::from_utf8(raw)
        .map_err(|err| format!("failed to read {} as UTF-8: {err}", config_path.display()))?;
    if max_source_sets.is_some() {
        return read_health_config_source_sets(workspace_root, config_path, text, checkpoint);
    }
    let yaml = serde_yaml::from_str::<YamlValue>(text)
        .map_err(|err| format!("failed to parse {}: {err}", config_path.display()))?;
    checkpoint()?;
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

    let declared_count = match source_set_value {
        Some(YamlValue::Sequence(entries)) => entries.len(),
        Some(YamlValue::Mapping(entries)) => entries.len(),
        _ => 0,
    };
    if max_source_sets.is_some_and(|limit| declared_count > limit) {
        return Err(format!(
            "project source-map declares {declared_count} source sets; health inspection supports at most {MAX_HEALTH_SOURCE_SETS}"
        ));
    }

    match source_set_value {
        Some(YamlValue::Sequence(entries)) => {
            for entry in entries {
                checkpoint()?;
                source_sets.push(config_source_set_from_yaml(entry, default_format)?);
            }
        }
        Some(YamlValue::Mapping(entries)) => {
            for (key, entry) in entries {
                checkpoint()?;
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
        checkpoint()?;
        source_set.path = normalize_configured_path(workspace_root, &base_path, &source_set.path);
    }

    Ok((source_sets, configured_format_raw))
}

fn read_health_config_source_sets(
    workspace_root: &Path,
    config_path: &Path,
    text: &str,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<(Vec<ConfigSourceSet>, Option<String>), String> {
    bound_health_yaml_alias_expansion(text, checkpoint)?;
    let config = serde_yaml::from_str::<BoundedHealthConfig>(text)
        .map_err(|error| format!("failed to parse {}: {error}", config_path.display()))?;
    checkpoint()?;
    let configured_format_raw = match config.format {
        None => None,
        Some(YamlValue::String(value)) => Some(value),
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
    let base_path = match config.base_path {
        Some(YamlValue::String(value)) => value,
        Some(YamlValue::Number(value)) => value.to_string(),
        _ => ".".into(),
    };
    let mut source_sets = Vec::new();
    match config.source_sets {
        BoundedHealthSourceSets::Empty => {}
        BoundedHealthSourceSets::Sequence(entries) => {
            for entry in &entries {
                checkpoint()?;
                source_sets.push(config_source_set_from_yaml(entry, default_format)?);
            }
        }
        BoundedHealthSourceSets::Mapping(entries) => {
            for (key, entry) in &entries {
                checkpoint()?;
                let name = key.as_str().unwrap_or("main");
                source_sets.push(config_source_set_from_named_yaml(
                    name,
                    entry,
                    default_format,
                )?);
            }
        }
    }
    for source_set in &mut source_sets {
        checkpoint()?;
        source_set.path = normalize_configured_path(workspace_root, &base_path, &source_set.path);
    }
    Ok((source_sets, configured_format_raw))
}

fn bound_health_yaml_alias_expansion(
    text: &str,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<(), String> {
    struct ParserGuard(*mut yaml_parser_t);

    impl Drop for ParserGuard {
        fn drop(&mut self) {
            // SAFETY: the guard is created only after yaml_parser_initialize succeeds
            // and owns the single matching yaml_parser_delete call.
            unsafe { yaml_parser_delete(self.0) };
        }
    }

    struct EventGuard(*mut yaml_event_t);

    impl Drop for EventGuard {
        fn drop(&mut self) {
            // SAFETY: the guard is created only after yaml_parser_parse succeeds
            // and owns the single matching yaml_event_delete call.
            unsafe { yaml_event_delete(self.0) };
        }
    }

    struct Frame {
        expanded_bytes: usize,
        expanded_nodes: usize,
        expanded_depth: usize,
        anchor: Option<Vec<u8>>,
    }

    #[derive(Clone)]
    struct AnchorValue {
        expanded_bytes: usize,
        expanded_nodes: usize,
        expanded_depth: usize,
        scalar_value: Option<Vec<u8>>,
    }

    fn anchor_bytes(pointer: *const u8) -> Option<Vec<u8>> {
        (!pointer.is_null()).then(|| unsafe { CStr::from_ptr(pointer.cast()).to_bytes().to_vec() })
    }

    fn add_node(
        stack: &mut [Frame],
        anchors: &mut BTreeMap<Vec<u8>, AnchorValue>,
        anchor: Option<Vec<u8>>,
        expanded_bytes: usize,
        expanded_nodes: usize,
        expanded_depth: usize,
        scalar_value: Option<Vec<u8>>,
    ) {
        if let Some(anchor) = anchor {
            anchors.insert(
                anchor,
                AnchorValue {
                    expanded_bytes,
                    expanded_nodes,
                    expanded_depth,
                    scalar_value,
                },
            );
        }
        if let Some(parent) = stack.last_mut() {
            parent.expanded_bytes = parent.expanded_bytes.saturating_add(expanded_bytes);
            parent.expanded_nodes = parent.expanded_nodes.saturating_add(expanded_nodes);
            parent.expanded_depth = parent.expanded_depth.max(expanded_depth.saturating_add(1));
        }
    }

    fn retain_health_value_bytes(total: &mut usize, bytes: usize) -> Result<(), String> {
        *total = total.saturating_add(bytes);
        if *total > MAX_HEALTH_YAML_RETAINED_BYTES {
            Err(format!(
                "expanded YAML values retained by health inspection exceed {MAX_HEALTH_YAML_RETAINED_BYTES} bytes"
            ))
        } else {
            Ok(())
        }
    }

    fn account_health_document_expansion(
        nodes: &mut usize,
        expanded_bytes: &mut usize,
        added_nodes: usize,
        bytes: usize,
    ) -> Result<(), String> {
        *nodes = nodes.saturating_add(added_nodes);
        if *nodes > MAX_HEALTH_YAML_DOCUMENT_NODES {
            return Err(format!(
                "expanded YAML document for health inspection exceeds {MAX_HEALTH_YAML_DOCUMENT_NODES} nodes"
            ));
        }
        *expanded_bytes = expanded_bytes.saturating_add(bytes);
        if *expanded_bytes > MAX_HEALTH_YAML_DOCUMENT_BYTES {
            return Err(format!(
                "expanded YAML document for health inspection exceeds {MAX_HEALTH_YAML_DOCUMENT_BYTES} bytes"
            ));
        }
        Ok(())
    }

    // serde_yaml resolves aliases while deserializing a `YamlValue`. Preflight the
    // libyaml event stream first and account for the expanded size of each alias.
    // This keeps small, ordinary anchors compatible with project.map while rejecting
    // an alias bomb before any owned value tree is constructed.
    unsafe {
        let mut parser = MaybeUninit::<yaml_parser_t>::uninit();
        let parser = parser.as_mut_ptr();
        if yaml_parser_initialize(parser).fail {
            return Err("failed to initialize YAML scanner for health inspection".into());
        }
        let _parser_guard = ParserGuard(parser);
        yaml_parser_set_input_string(parser, text.as_ptr(), text.len() as u64);
        let mut anchors = BTreeMap::<Vec<u8>, AnchorValue>::new();
        let mut stack = Vec::<Frame>::new();
        let mut root_mapping_depth = None;
        let mut root_expects_key = true;
        let mut next_root_value_is_retained = false;
        let mut retained_container_depth = None;
        let mut retained_bytes = 0_usize;
        let mut document_nodes = 0_usize;
        let mut document_expanded_bytes = 0_usize;
        let result = loop {
            if let Err(reason) = checkpoint() {
                break Err(reason);
            }
            let mut event = MaybeUninit::<yaml_event_t>::uninit();
            let event = event.as_mut_ptr();
            if yaml_parser_parse(parser, event).fail {
                let problem_ptr = (&(*parser)).problem;
                let problem = if problem_ptr.is_null() {
                    "unknown YAML scanner error".into()
                } else {
                    CStr::from_ptr(problem_ptr).to_string_lossy().into_owned()
                };
                break Err(format!(
                    "failed to parse YAML before health inspection: {problem}"
                ));
            }
            let _event_guard = EventGuard(event);
            let event_type = (*event).type_;
            let event_result = match event_type {
                YAML_DOCUMENT_START_EVENT => {
                    anchors.clear();
                    stack.clear();
                    root_mapping_depth = None;
                    root_expects_key = true;
                    next_root_value_is_retained = false;
                    retained_container_depth = None;
                    retained_bytes = 0;
                    document_nodes = 0;
                    document_expanded_bytes = 0;
                    Ok(())
                }
                YAML_MAPPING_START_EVENT => {
                    let new_depth = stack.len() + 1;
                    if new_depth > MAX_HEALTH_YAML_DOCUMENT_DEPTH {
                        return Err(format!(
                            "expanded YAML document for health inspection exceeds depth {MAX_HEALTH_YAML_DOCUMENT_DEPTH}"
                        ));
                    }
                    account_health_document_expansion(
                        &mut document_nodes,
                        &mut document_expanded_bytes,
                        1,
                        16,
                    )?;
                    if stack.is_empty() {
                        root_mapping_depth = Some(new_depth);
                    }
                    if root_mapping_depth == Some(stack.len())
                        && !root_expects_key
                        && next_root_value_is_retained
                    {
                        retained_container_depth = Some(new_depth);
                    }
                    if retained_container_depth.is_some() {
                        retain_health_value_bytes(&mut retained_bytes, 16)?;
                    }
                    stack.push(Frame {
                        expanded_bytes: 16,
                        expanded_nodes: 1,
                        expanded_depth: 1,
                        anchor: anchor_bytes((*event).data.mapping_start.anchor),
                    });
                    Ok(())
                }
                YAML_SEQUENCE_START_EVENT => {
                    let new_depth = stack.len() + 1;
                    if new_depth > MAX_HEALTH_YAML_DOCUMENT_DEPTH {
                        return Err(format!(
                            "expanded YAML document for health inspection exceeds depth {MAX_HEALTH_YAML_DOCUMENT_DEPTH}"
                        ));
                    }
                    account_health_document_expansion(
                        &mut document_nodes,
                        &mut document_expanded_bytes,
                        1,
                        16,
                    )?;
                    if root_mapping_depth == Some(stack.len())
                        && !root_expects_key
                        && next_root_value_is_retained
                    {
                        retained_container_depth = Some(new_depth);
                    }
                    if retained_container_depth.is_some() {
                        retain_health_value_bytes(&mut retained_bytes, 16)?;
                    }
                    stack.push(Frame {
                        expanded_bytes: 16,
                        expanded_nodes: 1,
                        expanded_depth: 1,
                        anchor: anchor_bytes((*event).data.sequence_start.anchor),
                    });
                    Ok(())
                }
                YAML_MAPPING_END_EVENT | YAML_SEQUENCE_END_EVENT => {
                    let closing_depth = stack.len();
                    let frame = stack.pop().ok_or_else(|| {
                        "YAML container ended without a matching start event".to_string()
                    })?;
                    add_node(
                        &mut stack,
                        &mut anchors,
                        frame.anchor,
                        frame.expanded_bytes,
                        frame.expanded_nodes,
                        frame.expanded_depth,
                        None,
                    );
                    if retained_container_depth == Some(closing_depth) {
                        retained_container_depth = None;
                    }
                    if root_mapping_depth == Some(stack.len()) {
                        root_expects_key = !root_expects_key;
                        if root_expects_key {
                            next_root_value_is_retained = false;
                        }
                    }
                    Ok(())
                }
                YAML_SCALAR_EVENT => {
                    let scalar = (*event).data.scalar;
                    let expanded_bytes = (scalar.length as usize).saturating_add(16);
                    account_health_document_expansion(
                        &mut document_nodes,
                        &mut document_expanded_bytes,
                        1,
                        expanded_bytes,
                    )?;
                    let is_root_child = root_mapping_depth == Some(stack.len());
                    if retained_container_depth.is_some()
                        || (is_root_child && !root_expects_key && next_root_value_is_retained)
                    {
                        retain_health_value_bytes(&mut retained_bytes, expanded_bytes)?;
                    }
                    add_node(
                        &mut stack,
                        &mut anchors,
                        anchor_bytes(scalar.anchor),
                        expanded_bytes,
                        1,
                        1,
                        (!scalar.anchor.is_null()).then(|| {
                            std::slice::from_raw_parts(scalar.value, scalar.length as usize)
                                .to_vec()
                        }),
                    );
                    if is_root_child {
                        if root_expects_key {
                            let value =
                                std::slice::from_raw_parts(scalar.value, scalar.length as usize);
                            next_root_value_is_retained =
                                matches!(value, b"format" | b"basePath" | b"source-set");
                            root_expects_key = false;
                        } else {
                            root_expects_key = true;
                            next_root_value_is_retained = false;
                        }
                    }
                    Ok(())
                }
                YAML_ALIAS_EVENT => {
                    let alias = anchor_bytes((*event).data.alias.anchor)
                        .ok_or_else(|| "YAML alias event has no anchor name".to_string())?;
                    let anchor = anchors.get(&alias).cloned().ok_or_else(|| {
                        "YAML alias refers to an unresolved or recursive anchor".to_string()
                    })?;
                    let expanded_bytes = anchor.expanded_bytes;
                    if stack.len().saturating_add(anchor.expanded_depth)
                        > MAX_HEALTH_YAML_DOCUMENT_DEPTH
                    {
                        return Err(format!(
                            "expanded YAML document for health inspection exceeds depth {MAX_HEALTH_YAML_DOCUMENT_DEPTH}"
                        ));
                    }
                    account_health_document_expansion(
                        &mut document_nodes,
                        &mut document_expanded_bytes,
                        anchor.expanded_nodes,
                        expanded_bytes,
                    )?;
                    let is_root_child = root_mapping_depth == Some(stack.len());
                    if retained_container_depth.is_some()
                        || (is_root_child && !root_expects_key && next_root_value_is_retained)
                    {
                        retain_health_value_bytes(&mut retained_bytes, expanded_bytes)?;
                    }
                    add_node(
                        &mut stack,
                        &mut anchors,
                        None,
                        expanded_bytes,
                        anchor.expanded_nodes,
                        anchor.expanded_depth,
                        None,
                    );
                    if is_root_child {
                        if root_expects_key {
                            next_root_value_is_retained =
                                anchor.scalar_value.as_deref().is_some_and(|value| {
                                    matches!(value, b"format" | b"basePath" | b"source-set")
                                });
                            root_expects_key = false;
                        } else {
                            root_expects_key = true;
                            next_root_value_is_retained = false;
                        }
                    }
                    Ok(())
                }
                _ => Ok(()),
            };
            if let Err(reason) = event_result {
                break Err(reason);
            }
            if event_type == YAML_STREAM_END_EVENT {
                break Ok(());
            }
        };
        result
    }
}

fn snapshot_project_map_input(
    path: &Path,
    read_contents: bool,
    state: &mut SourceMapDiscoveryState,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<Option<Vec<u8>>, String> {
    checkpoint()?;
    if state.require_nofollow_source_probe_route {
        return snapshot_health_project_map_input(path, read_contents, state, checkpoint);
    }
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let identity = normalize_path_identity(path)?;
            state.record_absence(identity)?;
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
    if !read_contents && !state.captures_provenance() {
        return Ok(Some(Vec::new()));
    }
    if let Some(limit) = state.max_input_bytes {
        if metadata.len() > limit {
            return Err(format!(
                "project source-map input {} exceeds {limit} bytes",
                path.display()
            ));
        }
    }
    let mut file = std::fs::File::open(path).map_err(|error| {
        format!(
            "failed to read project source-map input {}: {error}",
            path.display()
        )
    })?;
    let mut raw = Vec::with_capacity(metadata.len() as usize);
    let mut chunk = [0_u8; PROJECT_SOURCE_MAP_READ_CHUNK_BYTES];
    loop {
        checkpoint()?;
        let read = file.read(&mut chunk).map_err(|error| {
            format!(
                "failed to read project source-map input {}: {error}",
                path.display()
            )
        })?;
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..read]);
        if state
            .max_input_bytes
            .is_some_and(|limit| raw.len() as u64 > limit)
        {
            return Err(format!(
                "project source-map input {} exceeds {} bytes",
                path.display(),
                state.max_input_bytes.expect("checked above")
            ));
        }
        checkpoint()?;
    }
    let identity = normalize_path_identity(path)?;
    state.record_exact_file(identity, &raw)?;
    Ok(Some(raw))
}

fn snapshot_health_project_map_input(
    path: &Path,
    read_contents: bool,
    state: &mut SourceMapDiscoveryState,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<Option<Vec<u8>>, String> {
    let parent = path.parent().ok_or_else(|| {
        format!(
            "project source-map input has no parent directory: {}",
            path.display()
        )
    })?;
    let file_name = path.file_name().ok_or_else(|| {
        format!(
            "project source-map input has no file name: {}",
            path.display()
        )
    })?;
    let physical_parent = normalize_path_identity(parent)?;
    let directory = match open_absolute_directory_path_nofollow(&physical_parent) {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            state.record_absence(normalize_path_identity(path)?)?;
            return Ok(None);
        }
        Err(error) => {
            return Err(format!(
                "failed to open project source-map input parent {} safely: {error}",
                parent.display()
            ));
        }
    };
    let mut file = match open_regular_child_nofollow(&directory, file_name) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            state.record_absence(normalize_path_identity(path)?)?;
            return Ok(None);
        }
        Err(error) => {
            return Err(format!(
                "failed to open project source-map input {} safely: {error}",
                path.display()
            ));
        }
    };
    let metadata = file.metadata().map_err(|error| {
        format!(
            "failed to inspect opened project source-map input {}: {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() {
        return Err(format!(
            "project source-map input is not a regular file: {}",
            path.display()
        ));
    }
    if !read_contents && !state.captures_provenance() {
        return Ok(Some(Vec::new()));
    }
    if state
        .max_input_bytes
        .is_some_and(|limit| metadata.len() > limit)
    {
        return Err(format!(
            "project source-map input {} exceeds {} bytes",
            path.display(),
            state.max_input_bytes.expect("checked above")
        ));
    }
    let mut raw = Vec::with_capacity(metadata.len() as usize);
    let mut chunk = [0_u8; PROJECT_SOURCE_MAP_READ_CHUNK_BYTES];
    loop {
        checkpoint()?;
        let read = file.read(&mut chunk).map_err(|error| {
            format!(
                "failed to read opened project source-map input {}: {error}",
                path.display()
            )
        })?;
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..read]);
        if state
            .max_input_bytes
            .is_some_and(|limit| raw.len() as u64 > limit)
        {
            return Err(format!(
                "project source-map input {} exceeds {} bytes",
                path.display(),
                state.max_input_bytes.expect("checked above")
            ));
        }
    }
    state.record_exact_file(normalize_path_identity(path)?, &raw)?;
    Ok(Some(raw))
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
    state: &mut SourceMapDiscoveryState,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<Vec<ConfigSourceSet>, String> {
    let mut source_sets = Vec::new();
    for path in BASE_CONFIGURATION_LAYOUTS {
        checkpoint()?;
        let Some(directory) = open_probe_directory(workspace_root, path, state)? else {
            continue;
        };
        if probe_source_root_markers(&directory, state, checkpoint)? {
            source_sets.push(ConfigSourceSet {
                name: BASE_CONFIGURATION_SOURCE_SET.to_string(),
                kind: SourceSetKind::Configuration,
                path: (*path).to_string(),
                default_format: None,
            });
            break;
        }
    }
    let mut reserved_name_taken = !source_sets.is_empty();
    let extension_sets = autodetect_extension_source_sets(
        workspace_root,
        &mut reserved_name_taken,
        state,
        checkpoint,
    )?;
    if let Some(limit) = state.max_source_sets {
        let combined = source_sets.len().saturating_add(extension_sets.len());
        if combined > limit {
            return Err(format!(
                "workspace declares more than {limit} autodetected source sets; health inspection supports at most {limit}"
            ));
        }
    }
    source_sets.extend(extension_sets);
    Ok(source_sets)
}

/// Autodetects configuration-extension source sets from the same catalog, the same
/// marker list and the same probe the base configuration scan uses, so a bare workspace
/// with no `v8project.yaml` reports its extensions instead of only its configuration.
/// Declared `v8project.yaml` source sets take precedence over autodetection entirely
/// (see `discover_project_source_map_internal`), so this only runs for workspaces that
/// rely on autodetection in the first place.
fn autodetect_extension_source_sets(
    workspace_root: &Path,
    reserved_name_taken: &mut bool,
    state: &mut SourceMapDiscoveryState,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<Vec<ConfigSourceSet>, String> {
    let mut extension_sets = Vec::new();
    for layout in EXTENSION_LAYOUTS {
        checkpoint()?;
        let Some(container) = open_extension_container(workspace_root, layout.directory, state)?
        else {
            state.record_directory_membership(
                workspace_root.join(layout.directory),
                DirectoryMembershipSnapshot::Absent,
            )?;
            continue;
        };
        if layout.shape == ExtensionLayoutShape::RootOrContainer
            && probe_source_root_markers(&container, state, checkpoint)?
        {
            // The container is itself one extension's dump root, so its children are
            // that extension's object directories, not sibling extensions — the same
            // stop the base configuration scan makes at its first match. Its listing is
            // not an input either, so no membership is recorded for it.
            push_autodetected_extension(
                &mut extension_sets,
                layout_root_source_set_name(layout.directory),
                layout.directory.to_string(),
                reserved_name_taken,
            );
            continue;
        }
        for (name, directory) in enumerate_source_root_candidates(
            workspace_root,
            layout.directory,
            &container,
            state,
            checkpoint,
        )? {
            checkpoint()?;
            if probe_source_root_markers(&directory, state, checkpoint)? {
                let path = format!("{}/{name}", layout.directory);
                push_autodetected_extension(&mut extension_sets, name, path, reserved_name_taken);
            }
        }
    }
    Ok(extension_sets)
}

/// The source set an autodetected layout directory publishes when it is a source root
/// itself. An autodetected extension is named after the directory that holds it, so a
/// root at `src/cfe` is `cfe`, exactly as a root at `src/cfe/test_ed` is `test_ed`.
fn layout_root_source_set_name(directory: &str) -> String {
    Path::new(directory)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| directory.to_string())
}

/// Publishes one autodetected extension under the source-set naming policy.
///
/// Names are the directory name verbatim — the same string the filesystem reports, with
/// no case folding and no other normalization, exactly as a declared `v8project.yaml`
/// entry would carry it.
///
/// `main` is reserved for the base configuration (`INV-SOURCE-SINGLE-RESOLVED-ROOT`),
/// and exactly one entry may carry it: a second would make `sourceSet: "main"` ambiguous
/// for named lookup while `select_default_source_set` silently kept resolving it to
/// whichever entry came first — turning a resolution defined to be deterministic into a
/// coin flip. The base scan claims it when it found a configuration; otherwise it falls
/// to the first extension claiming it in catalog order, which is a fixed documented
/// order rather than a filesystem accident. Reserving the name against an absent owner
/// instead would empty the map of the workspace's only source root.
///
/// Two extensions from *different* layouts can still derive the same non-reserved name,
/// and both are published. A duplicate-name group is a represented state that
/// `unica.project.status` diagnoses and named lookup rejects as ambiguous; dropping one
/// would instead resolve the name silently to an arbitrary one of two real source roots.
fn push_autodetected_extension(
    extension_sets: &mut Vec<ConfigSourceSet>,
    name: String,
    path: String,
    reserved_name_taken: &mut bool,
) {
    if name == BASE_CONFIGURATION_SOURCE_SET {
        if *reserved_name_taken {
            return;
        }
        *reserved_name_taken = true;
    }
    extension_sets.push(ConfigSourceSet {
        name,
        kind: SourceSetKind::Extension,
        path,
        default_format: None,
    });
}

/// Opens one autodetection directory on the route discovery is running.
///
/// `None` means the hardened route proved the directory absent. The ordinary route
/// probes by path and answers `Some` unconditionally: it learns absence from the probes
/// themselves, and each probed path — present or absent — is an input it must record.
fn open_probe_directory(
    workspace_root: &Path,
    relative: &str,
    state: &SourceMapDiscoveryState,
) -> Result<Option<ProbeDirectory>, String> {
    if state.require_nofollow_source_probe_route {
        Ok(health_source_directory(workspace_root, relative)?.map(ProbeDirectory::Retained))
    } else {
        Ok(Some(ProbeDirectory::Path(workspace_root.join(relative))))
    }
}

/// Opens one extension-layout container, telling apart the two answers a container path
/// can give.
///
/// A path that definitely holds no container — absent, a regular file, a link — yields
/// `None`, and that layout simply contributes nothing. A path that *might* hold one but
/// could not be read — denied permissions, an I/O failure — is still an error, because
/// answering "no extensions" there would report a map the filesystem never confirmed.
///
/// The distinction is the same one entry classification makes one level down, and it has
/// to be made here too: a linked `src/cfe` otherwise aborts the hardened route while the
/// ordinary route enumerates straight through the link and publishes source roots outside
/// the workspace that `resolve_named_source_set` then refuses to open.
fn open_extension_container(
    workspace_root: &Path,
    relative: &str,
    state: &SourceMapDiscoveryState,
) -> Result<Option<ProbeDirectory>, String> {
    if !state.require_nofollow_source_probe_route {
        let path = workspace_root.join(relative);
        return match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => Ok(Some(ProbeDirectory::Path(path))),
            Ok(_) => Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!(
                "failed to inspect source directory {}: {error}",
                path.display()
            )),
        };
    }
    // An unsafe or uncontained route is not a container either: the path leaves the
    // workspace, and nothing reachable through it could be resolved later anyway. Only
    // that verdict is turned into "no container here" — a failure to normalize the
    // workspace itself is about the workspace, not about this layout, and still
    // propagates.
    if inspect_declared_source_root_route(workspace_root, relative).is_err() {
        return Ok(None);
    }
    match health_source_directory_opened(workspace_root, relative)? {
        Ok(directory) => Ok(directory.map(ProbeDirectory::Retained)),
        Err(error) if open_failure_is_wrong_kind(&error) => Ok(None),
        Err(error) => Err(format!(
            "source directory could not be opened safely: {error}"
        )),
    }
}

/// Probes one directory for every source-root marker, on either route.
fn probe_source_root_markers(
    directory: &ProbeDirectory,
    state: &mut SourceMapDiscoveryState,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<bool, String> {
    let mut found = false;
    for marker in SOURCE_ROOT_MARKERS {
        checkpoint()?;
        match directory {
            ProbeDirectory::Retained(handle) => {
                match secure_regular_file_exists(handle, Path::new(marker)) {
                    Ok(exists) => found |= exists,
                    Err(SecureMarkerProbeError::UnsafeOrWrongKind(_)) => {
                        // An autodetected candidate has no declared source set on which to
                        // report an incomplete format probe. Unsafe or wrong-kind routes are
                        // not evidence and must never be followed; declared source sets still
                        // retain the same error as a per-set NotRun outcome.
                    }
                    Err(SecureMarkerProbeError::Incomplete(reason)) => return Err(reason),
                }
            }
            ProbeDirectory::Path(root) => {
                found |= snapshot_project_map_input(&root.join(marker), false, state, checkpoint)?
                    .is_some();
            }
        }
    }
    Ok(found)
}

/// Direct children of an autodetection container that can host a source root: real
/// directories, reached without following a link, whose name can be a source-set name.
/// Sorted by name, so both routes and every run agree on the order they are published
/// in.
///
/// Enumeration classifies and skips. A container holds whatever the workspace put there
/// — `.gitkeep`, `README.md`, `.DS_Store`, a symlink — and none of those is a source
/// root; a route that aborted on the first of them would report an incomplete layout
/// for a workspace whose layout is complete. Only a genuine I/O failure stops discovery.
/// A link is skipped rather than published because `resolve_named_source_set` rejects a
/// linked source root, so publishing one would advertise a source set every
/// `unica.code.*` tool then refuses to open.
///
/// Enumerating the container is itself a discovery input, so the same listing is
/// recorded in provenance here rather than re-read later.
fn enumerate_source_root_candidates(
    workspace_root: &Path,
    container_relative: &str,
    container: &ProbeDirectory,
    state: &mut SourceMapDiscoveryState,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<Vec<(String, ProbeDirectory)>, String> {
    let container_path = workspace_root.join(container_relative);
    match container {
        ProbeDirectory::Retained(handle) => {
            // `limit` bounds this enumeration's own I/O cost. The combined source-set
            // total is enforced by `autodetect_source_sets` once the base scan and every
            // layout are known.
            let limit = state
                .max_source_sets
                .unwrap_or(MAX_HEALTH_SOURCE_SETS)
                .saturating_add(1);
            let mut checkpoint_failure = None;
            let names = read_directory_names_bounded(handle, limit, || match checkpoint() {
                Ok(()) => Ok(()),
                Err(reason) => {
                    checkpoint_failure = Some(reason);
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "project source discovery checkpoint stopped enumeration",
                    ))
                }
            });
            if let Some(reason) = checkpoint_failure {
                return Err(reason);
            }
            let names = names.map_err(|error| {
                format!(
                    "failed to inspect source directory {} securely: {error}",
                    container_path.display()
                )
            })?;
            let mut candidates = Vec::new();
            for name in names {
                checkpoint()?;
                let Ok(text) = name.clone().into_string() else {
                    // A source-set name is a JSON string, so a directory name that is not
                    // valid UTF-8 cannot become one. Skipping it keeps its siblings
                    // visible; failing would hide every extension beside it.
                    continue;
                };
                let (anchor, kind) = match open_any_child_nofollow(handle, &name) {
                    Ok(opened) => opened,
                    // The entry raced away, is a link, or is not a directory. None of
                    // those is a candidate, and none is a reason to stop looking at its
                    // siblings.
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) if open_failure_is_wrong_kind(&error) => continue,
                    Err(error) => {
                        return Err(format!(
                            "failed to inspect source directory entry {} securely: {error}",
                            container_path.join(&text).display()
                        ));
                    }
                };
                if kind != OpenedChildKind::Directory {
                    continue;
                }
                let directory = open_child_for_secure_tree_use(handle, &name, anchor, kind)
                    .map_err(|error| {
                        format!(
                            "failed to open source directory entry {} securely: {error}",
                            container_path.join(&text).display()
                        )
                    })?;
                candidates.push((text, ProbeDirectory::Retained(directory)));
            }
            Ok(candidates)
        }
        ProbeDirectory::Path(path) => {
            let entries = match std::fs::read_dir(path) {
                Ok(entries) => entries,
                // Only an absent container means "no extensions here"; any other failure
                // (permissions, a regular file in the way, ...) must be reported, not
                // silently treated the same as "there is nothing to autodetect".
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    state.record_directory_membership(
                        container_path,
                        DirectoryMembershipSnapshot::Absent,
                    )?;
                    return Ok(Vec::new());
                }
                Err(error) => {
                    return Err(format!(
                        "failed to inspect source directory {}: {error}",
                        path.display()
                    ));
                }
            };
            let mut candidates = Vec::new();
            for entry in entries {
                checkpoint()?;
                let entry = entry.map_err(|error| {
                    format!(
                        "failed to inspect source directory {}: {error}",
                        path.display()
                    )
                })?;
                // `DirEntry::file_type` reports the entry itself, so a link pointing at a
                // directory is not a directory here.
                let file_type = entry.file_type().map_err(|error| {
                    format!(
                        "failed to inspect source directory entry {}: {error}",
                        entry.path().display()
                    )
                })?;
                if !file_type.is_dir() {
                    continue;
                }
                let Ok(name) = entry.file_name().into_string() else {
                    continue;
                };
                let child = path.join(&name);
                candidates.push((name, ProbeDirectory::Path(child)));
            }
            candidates.sort_by(|left, right| left.0.cmp(&right.0));
            state.record_directory_membership(
                container_path,
                DirectoryMembershipSnapshot::Present(
                    candidates
                        .iter()
                        .map(|(name, _)| DirectoryTopologyEntry {
                            name: std::ffi::OsString::from(name),
                            kind: DirectoryTopologyEntryKind::Directory,
                        })
                        .collect(),
                ),
            )?;
            Ok(candidates)
        }
    }
}

fn detect_source_set_format(
    workspace_root: &Path,
    source_set: ConfigSourceSet,
    state: &mut SourceMapDiscoveryState,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<ProjectSourceSet, String> {
    let source_root = workspace_root.join(&source_set.path);
    let mut format_probe_error = None;
    let (platform_evidence, edt_evidence) = if state.require_nofollow_source_probe_route {
        match health_format_evidence(
            workspace_root,
            &source_root,
            source_set.kind,
            state,
            checkpoint,
        ) {
            Ok(evidence) => evidence,
            Err(reason) => {
                format_probe_error = Some(reason);
                (Vec::new(), Vec::new())
            }
        }
    } else {
        (
            platform_xml_evidence(
                workspace_root,
                &source_root,
                source_set.kind,
                state,
                checkpoint,
            )?,
            edt_evidence(workspace_root, &source_root, state, checkpoint)?,
        )
    };
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
            let evidence = match default_format {
                SourceFormat::PlatformXml => "v8project.yaml:format=DESIGNER".to_string(),
                SourceFormat::Edt => "v8project.yaml:format=EDT".to_string(),
                SourceFormat::Unknown | SourceFormat::Invalid => {
                    "v8project.yaml:format".to_string()
                }
            };
            state.record_format_evidence(&evidence)?;
            format_evidence.push(evidence);
        }
    }

    Ok(ProjectSourceSet {
        name: source_set.name,
        kind: source_set.kind,
        path: source_set.path,
        source_format,
        format_evidence,
        format_probe_error,
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

fn health_format_evidence(
    workspace_root: &Path,
    source_root: &Path,
    kind: SourceSetKind,
    state: &mut SourceMapDiscoveryState,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<(Vec<String>, Vec<String>), String> {
    checkpoint()?;
    let configured_path = path_relative_to(workspace_root, source_root);
    let Some(source_directory) = health_source_directory(workspace_root, &configured_path)? else {
        return Ok((Vec::new(), Vec::new()));
    };
    let mut platform = Vec::new();
    let mut edt = Vec::new();
    if secure_regular_file_exists(&source_directory, Path::new("Configuration.xml"))
        .map_err(SecureMarkerProbeError::into_reason)?
    {
        push_health_evidence(
            &mut platform,
            workspace_root,
            &source_root.join("Configuration.xml"),
            state,
        )?;
    }
    for marker in EDT_SOURCE_MARKERS {
        checkpoint()?;
        if secure_regular_file_exists(&source_directory, Path::new(marker))
            .map_err(SecureMarkerProbeError::into_reason)?
        {
            push_health_evidence(&mut edt, workspace_root, &source_root.join(marker), state)?;
        }
    }
    if matches!(
        kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        let limit = state
            .remaining_format_evidence_entries
            .unwrap_or(MAX_HEALTH_FORMAT_EVIDENCE_ENTRIES)
            .saturating_add(1);
        let mut checkpoint_failure = None;
        let names = read_directory_names_bounded(&source_directory, limit, || match checkpoint() {
            Ok(()) => Ok(()),
            Err(reason) => {
                checkpoint_failure = Some(reason);
                Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "project source discovery checkpoint stopped enumeration",
                ))
            }
        });
        if let Some(reason) = checkpoint_failure {
            return Err(reason);
        }
        let names = names.map_err(|error| {
            format!(
                "failed to inspect source directory {} securely: {error}",
                source_root.display()
            )
        })?;
        for name in names {
            checkpoint()?;
            let path = Path::new(&name);
            if !path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
            {
                continue;
            }
            let mut file = match open_regular_child_nofollow(&source_directory, &name) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => match marker_probe_open_error("file", error) {
                    SecureMarkerProbeError::UnsafeOrWrongKind(_) => continue,
                    SecureMarkerProbeError::Incomplete(reason) => return Err(reason),
                },
            };
            if has_config_dump_info_filename(path)
                && !secure_config_dump_info_is_source_descriptor(&mut file, kind, checkpoint)?
            {
                continue;
            }
            push_health_evidence(
                &mut platform,
                workspace_root,
                &source_root.join(&name),
                state,
            )?;
        }
    }
    platform.sort();
    platform.dedup();
    edt.sort();
    edt.dedup();
    checkpoint()?;
    Ok((platform, edt))
}

fn health_source_directory(
    workspace_root: &Path,
    configured_path: &str,
) -> Result<Option<std::fs::File>, String> {
    health_source_directory_opened(workspace_root, configured_path)?
        .map_err(|error| format!("source directory could not be opened safely: {error}"))
}

/// Opens a source directory on the hardened route, keeping the open failure typed.
///
/// The outer `Result` carries the route failures that are never an open at all; the
/// inner one carries the open itself, so a caller that must tell "nothing usable is
/// here" from "this could not be read" classifies on `ErrorKind` rather than on the
/// text of an OS message, which differs by platform and by locale. `Ok(None)` is the
/// absent directory, which every caller treats the same way.
fn health_source_directory_opened(
    workspace_root: &Path,
    configured_path: &str,
) -> Result<Result<Option<std::fs::File>, std::io::Error>, String> {
    let route = inspect_declared_source_root_route(workspace_root, configured_path)
        .map_err(|error| format!("source route could not be proven safe: {error}"))?;
    let physical_workspace = normalize_path_identity(workspace_root)?;
    let relative = route
        .lexical_path
        .strip_prefix(workspace_root)
        .map_err(|error| format!("source route is outside the workspace lexical root: {error}"))?;
    Ok(
        match open_absolute_directory_path_nofollow(&physical_workspace.join(relative)) {
            Ok(directory) => Ok(Some(directory)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        },
    )
}

enum SecureMarkerProbeError {
    UnsafeOrWrongKind(String),
    Incomplete(String),
}

impl SecureMarkerProbeError {
    fn into_reason(self) -> String {
        match self {
            Self::UnsafeOrWrongKind(reason) | Self::Incomplete(reason) => reason,
        }
    }
}

fn secure_regular_file_exists(
    source_directory: &std::fs::File,
    relative: &Path,
) -> Result<bool, SecureMarkerProbeError> {
    use std::path::Component;

    let mut components = relative.components().peekable();
    let mut parent = source_directory.try_clone().map_err(|error| {
        SecureMarkerProbeError::Incomplete(format!(
            "source directory handle could not be cloned: {error}"
        ))
    })?;
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(SecureMarkerProbeError::UnsafeOrWrongKind(
                "source format marker path is not normal".into(),
            ));
        };
        if components.peek().is_some() {
            let directory = match open_directory_child_nofollow(&parent, name) {
                Ok(directory) => directory,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(marker_probe_open_error("parent", error)),
            };
            parent = directory;
        } else {
            return match open_regular_child_nofollow(&parent, name) {
                Ok(_) => Ok(true),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(marker_probe_open_error("file", error)),
            };
        }
    }
    Ok(false)
}

fn marker_probe_open_error(context: &str, error: std::io::Error) -> SecureMarkerProbeError {
    let reason = format!("source format marker {context} could not be opened safely: {error}");
    if open_failure_is_wrong_kind(&error) {
        SecureMarkerProbeError::UnsafeOrWrongKind(reason)
    } else {
        SecureMarkerProbeError::Incomplete(reason)
    }
}

/// Whether an open failure answered the question — the path is the wrong kind of thing —
/// or left it open.
///
/// Unix rejects a link at open with a loop error and a non-directory with
/// `NotADirectory`; Windows opens the reparse point and then classifies it, and a
/// non-directory, as `InvalidInput`. Everything else — denied permissions, an I/O
/// failure — means the path might still be what the caller was looking for, so it is
/// never silently treated as absent. Every hardened probe classifies through this one
/// predicate: a marker, a container, and a container's entries have to agree on what
/// counts as "nothing usable here".
fn open_failure_is_wrong_kind(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::InvalidInput
            | std::io::ErrorKind::InvalidData
            | std::io::ErrorKind::NotADirectory
    ) || is_link_loop_error(error)
}

fn push_health_evidence(
    evidence: &mut Vec<String>,
    workspace_root: &Path,
    path: &Path,
    state: &mut SourceMapDiscoveryState,
) -> Result<(), String> {
    let relative = path_relative_to(workspace_root, path);
    state.record_format_evidence(&relative)?;
    evidence.push(relative);
    Ok(())
}

fn secure_config_dump_info_is_source_descriptor(
    file: &mut std::fs::File,
    kind: SourceSetKind,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<bool, String> {
    let metadata = file
        .metadata()
        .map_err(|error| format!("source descriptor metadata failed: {error}"))?;
    if !metadata.is_file() || metadata.len() > MAX_RESERVED_EXTERNAL_DESCRIPTOR_BYTES {
        return Ok(false);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    let mut limited = file.take(MAX_RESERVED_EXTERNAL_DESCRIPTOR_BYTES + 1);
    let mut chunk = [0_u8; PROJECT_SOURCE_MAP_READ_CHUNK_BYTES];
    loop {
        checkpoint()?;
        let read = limited
            .read(&mut chunk)
            .map_err(|error| format!("source descriptor read failed: {error}"))?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() as u64 > MAX_RESERVED_EXTERNAL_DESCRIPTOR_BYTES {
            return Ok(false);
        }
    }
    Ok(matches!(
        (config_dump_info_xml_kind(&bytes), kind),
        (
            ConfigDumpInfoXmlKind::ExternalProcessor,
            SourceSetKind::ExternalProcessor
        ) | (
            ConfigDumpInfoXmlKind::ExternalReport,
            SourceSetKind::ExternalReport
        )
    ))
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
    state: &mut SourceMapDiscoveryState,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<Vec<String>, String> {
    checkpoint()?;
    let mut evidence = Vec::new();
    // The exact configuration descriptor is authorization-bound by the
    // platform-owner resolver. Keeping format detection itself non-owning also
    // preserves the structured owner diagnostic for a link/reparse candidate.
    push_existing(
        &mut evidence,
        workspace_root,
        &source_root.join("Configuration.xml"),
        state,
    )?;

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
                checkpoint()?;
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("xml")
                    && !is_config_dump_info_sidecar(&path, kind, checkpoint)?
                {
                    push_existing(&mut evidence, workspace_root, &path, state)?;
                }
            }
        }
    }
    evidence.sort();
    evidence.dedup();
    checkpoint()?;
    Ok(evidence)
}

fn is_config_dump_info_sidecar(
    path: &Path,
    kind: SourceSetKind,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<bool, String> {
    if !has_config_dump_info_filename(path) {
        return Ok(false);
    }
    Ok(!matches!(
        (config_dump_info_xml_file_kind(path, checkpoint)?, kind),
        (
            ConfigDumpInfoXmlKind::ExternalProcessor,
            SourceSetKind::ExternalProcessor
        ) | (
            ConfigDumpInfoXmlKind::ExternalReport,
            SourceSetKind::ExternalReport
        )
    ))
}

fn config_dump_info_xml_file_kind(
    path: &Path,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<ConfigDumpInfoXmlKind, String> {
    checkpoint()?;
    if !has_config_dump_info_filename(path) {
        return Ok(ConfigDumpInfoXmlKind::Other);
    }
    let Ok(link_metadata) = std::fs::symlink_metadata(path) else {
        return Ok(ConfigDumpInfoXmlKind::Other);
    };
    if link_metadata.file_type().is_symlink() || !link_metadata.file_type().is_file() {
        return Ok(ConfigDumpInfoXmlKind::Other);
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return Ok(ConfigDumpInfoXmlKind::Other);
    };
    let Ok(metadata) = file.metadata() else {
        return Ok(ConfigDumpInfoXmlKind::Other);
    };
    if !metadata.file_type().is_file() || metadata.len() > MAX_RESERVED_EXTERNAL_DESCRIPTOR_BYTES {
        return Ok(ConfigDumpInfoXmlKind::Other);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    let mut limited = (&mut file).take(MAX_RESERVED_EXTERNAL_DESCRIPTOR_BYTES + 1);
    let mut chunk = [0_u8; PROJECT_SOURCE_MAP_READ_CHUNK_BYTES];
    loop {
        checkpoint()?;
        let Ok(read) = limited.read(&mut chunk) else {
            return Ok(ConfigDumpInfoXmlKind::Other);
        };
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        checkpoint()?;
    }
    if bytes.len() as u64 > MAX_RESERVED_EXTERNAL_DESCRIPTOR_BYTES {
        return Ok(ConfigDumpInfoXmlKind::Other);
    }
    Ok(config_dump_info_xml_kind(&bytes))
}

fn has_config_dump_info_filename(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("ConfigDumpInfo.xml"))
}

fn edt_evidence(
    workspace_root: &Path,
    source_root: &Path,
    state: &mut SourceMapDiscoveryState,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<Vec<String>, String> {
    let mut evidence = Vec::new();
    for rel in EDT_SOURCE_MARKERS {
        push_snapshot_existing(
            &mut evidence,
            workspace_root,
            &source_root.join(rel),
            state,
            checkpoint,
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
    state: &mut SourceMapDiscoveryState,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<(), String> {
    if snapshot_project_map_input(path, false, state, checkpoint)?.is_some() {
        let relative = path_relative_to(workspace_root, path);
        state.record_format_evidence(&relative)?;
        evidence.push(relative);
    }
    Ok(())
}

fn push_existing(
    evidence: &mut Vec<String>,
    workspace_root: &Path,
    path: &Path,
    state: &mut SourceMapDiscoveryState,
) -> Result<(), String> {
    if path.is_file() {
        let relative = path_relative_to(workspace_root, path);
        state.record_format_evidence(&relative)?;
        evidence.push(relative);
    }
    Ok(())
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
    fn source_map_input_read_honors_cooperative_checkpoint_between_chunks() {
        let root = temp_workspace("unica-source-map-controlled-read");
        let config = root.join("v8project.yaml");
        let mut raw = b"format: DESIGNER\nsource-set: []\n".to_vec();
        raw.extend(std::iter::repeat_n(b'#', 192 * 1024));
        fs::write(&config, &raw).unwrap();
        let mut state = SourceMapDiscoveryState::transactional();
        let mut checkpoints = 0;

        let error =
            snapshot_project_map_input(&config, true, &mut state, &mut || -> Result<(), String> {
                checkpoints += 1;
                if checkpoints == 4 {
                    Err("source discovery stopped by checkpoint".to_string())
                } else {
                    Ok(())
                }
            })
            .unwrap_err();

        assert_eq!(error, "source discovery stopped by checkpoint");
        assert_eq!(checkpoints, 4);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ordinary_source_map_keeps_accepting_large_valid_project_config() {
        let root = temp_workspace("unica-source-map-large-ordinary-config");
        let mut config =
            b"format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n"
                .to_vec();
        config.extend(std::iter::repeat_n(b'#', 8 * 1024 * 1024));
        fs::write(root.join("v8project.yaml"), config).unwrap();
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");

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
    fn transactional_provenance_keeps_accepting_large_source_marker() {
        let root = temp_workspace("unica-source-map-large-provenance-marker");
        write(
            &root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        let marker = root.join("src/Configuration.xml");
        fs::create_dir_all(marker.parent().unwrap()).unwrap();
        fs::File::create(&marker)
            .unwrap()
            .set_len(9 * 1024 * 1024)
            .unwrap();

        let (map, provenance) = discover_project_source_map_with_provenance(&root).unwrap();
        assert_eq!(map.source_sets[0].source_format, SourceFormat::PlatformXml);
        provenance
            .bind_to(&mut CompileTransaction::new())
            .expect("unchanged large marker must remain a valid transaction precondition");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn health_source_map_bounds_aggregate_format_evidence() {
        let root = temp_workspace("unica-source-map-health-evidence-budget");
        write(
            &root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: epf\n",
        );
        for name in ["A.xml", "B.xml", "C.xml"] {
            write(
                &root.join("epf").join(name),
                "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
            );
        }
        let mut checkpoint = || Ok(());
        let mut state = SourceMapDiscoveryState::health_with_evidence_limits(2, usize::MAX);

        let error = discover_project_source_map_internal(&root, &mut checkpoint, &mut state)
            .expect_err("health evidence must stay within its aggregate budget");

        assert!(error.contains("format evidence"), "{error}");
        assert!(error.contains("2 entries"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn health_source_map_does_not_probe_through_linked_source_root() {
        use crate::infrastructure::platform::testing::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let root = temp_workspace("unica-source-map-health-linked-root");
        let external = temp_workspace("unica-source-map-health-external");
        write(
            &root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: epf\n",
        );
        write(
            &external.join("SecretPayroll.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        );
        match create_directory_link_fixture_for_test(&external, root.join("epf")).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }
        let mut checkpoint = || Ok(());

        let map = discover_project_source_map_controlled(&root, &mut checkpoint).unwrap();

        assert_eq!(map.source_sets.len(), 1);
        assert_eq!(
            map.source_sets[0].format_evidence,
            vec!["v8project.yaml:format=DESIGNER"]
        );
        assert!(!map.source_sets[0]
            .format_evidence
            .iter()
            .any(|evidence| evidence.contains("SecretPayroll")));
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(external).unwrap();
    }

    #[test]
    fn health_source_map_does_not_probe_through_linked_marker_parent() {
        use crate::infrastructure::platform::testing::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let root = temp_workspace("unica-source-map-health-linked-marker");
        let external = temp_workspace("unica-source-map-health-marker-external");
        write(
            &root.join("v8project.yaml"),
            "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        fs::create_dir_all(root.join("src")).unwrap();
        write(
            &external.join("Configuration.mdo"),
            "<mdclass:Configuration secret='payroll'/>",
        );
        match create_directory_link_fixture_for_test(&external, root.join("src/Configuration"))
            .unwrap()
        {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }
        let mut checkpoint = || Ok(());

        let result = discover_project_source_map_controlled(&root, &mut checkpoint);

        match result {
            Ok(map) => {
                assert_ne!(map.source_sets[0].source_format, SourceFormat::Edt);
                assert!(!map.source_sets[0]
                    .format_evidence
                    .iter()
                    .any(|evidence| evidence.contains("Configuration/Configuration.mdo")));
            }
            Err(reason) => assert!(!reason.contains("payroll"), "{reason}"),
        }
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(external).unwrap();
    }

    #[test]
    fn health_source_map_does_not_probe_absolute_external_source_root() {
        let root = temp_workspace("unica-source-map-health-absolute-root");
        let external = temp_workspace("unica-source-map-health-absolute-external");
        write(
            &external.join("SecretPayroll.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        );
        write(
            &root.join("v8project.yaml"),
            &format!(
                "source-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: {}\n",
                external.display()
            ),
        );
        let mut checkpoint = || Ok(());

        let map = discover_project_source_map_controlled(&root, &mut checkpoint).unwrap();

        assert!(map.source_sets[0].format_evidence.is_empty());
        assert_eq!(map.source_sets[0].source_format, SourceFormat::Unknown);
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(external).unwrap();
    }

    #[test]
    fn health_source_map_bounds_unknown_yaml_subtrees_before_serde_materialization() {
        let root = temp_workspace("unica-source-map-health-unknown-yaml-depth");
        let depth = 512;
        let mut config = String::from(
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\nignored: ",
        );
        config.push_str(&"[".repeat(depth));
        config.push('0');
        config.push_str(&"]".repeat(depth));
        write(&root.join("v8project.yaml"), &config);
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
        let mut checkpoint = || Ok(());

        let error = discover_project_source_map_controlled(&root, &mut checkpoint)
            .expect_err("health YAML depth must be rejected before serde materialization");

        assert!(error.contains("YAML") && error.contains("depth"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn health_source_map_counts_nodes_expanded_through_unknown_aliases() {
        let root = temp_workspace("unica-source-map-health-unknown-alias-nodes");
        let anchored_nodes = 512;
        let alias_count = 128;
        let mut config = String::from(
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ntemplate: &many [",
        );
        config.push_str(&"{},".repeat(anchored_nodes));
        config.push_str("]\nignored:\n");
        config.push_str(&"  - *many\n".repeat(alias_count));
        write(&root.join("v8project.yaml"), &config);
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
        let mut checkpoint = || Ok(());

        let error = discover_project_source_map_controlled(&root, &mut checkpoint)
            .expect_err("expanded alias nodes must be rejected before serde materialization");

        assert!(error.contains("YAML") && error.contains("nodes"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn health_source_map_bounds_depth_expanded_at_alias_site() {
        let root = temp_workspace("unica-source-map-health-alias-expanded-depth");
        let anchor_depth = 180;
        let alias_site_depth = 100;
        let mut config = String::from(
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ntemplate: &deep ",
        );
        config.push_str(&"[".repeat(anchor_depth));
        config.push('0');
        config.push_str(&"]".repeat(anchor_depth));
        config.push_str("\nignored: ");
        config.push_str(&"[".repeat(alias_site_depth));
        config.push_str("*deep");
        config.push_str(&"]".repeat(alias_site_depth));
        write(&root.join("v8project.yaml"), &config);
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
        let mut checkpoint = || Ok(());

        let error = discover_project_source_map_controlled(&root, &mut checkpoint)
            .expect_err("alias-site expanded depth must be bounded before serde");

        assert!(error.contains("YAML") && error.contains("depth"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn health_autodetect_does_not_follow_linked_marker_parent() {
        use crate::infrastructure::platform::testing::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let root = temp_workspace("unica-source-map-health-autodetect-linked");
        let external = temp_workspace("unica-source-map-health-autodetect-external");
        fs::create_dir_all(root.join("src")).unwrap();
        write(
            &external.join("Configuration.mdo"),
            "<mdclass:Configuration secret='payroll'/>",
        );
        match create_directory_link_fixture_for_test(&external, root.join("src/Configuration"))
            .unwrap()
        {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }
        let mut checkpoint = || Ok(());

        let map = discover_project_source_map_controlled(&root, &mut checkpoint).unwrap();

        assert!(map.source_sets.is_empty(), "{:?}", map.source_sets);
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(external).unwrap();
    }

    #[test]
    fn autodetect_discovers_configuration_extension_source_sets() {
        let root = temp_workspace("unica-source-map-autodetect-extensions");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        write(
            &root.join("src/cfe/test_edreg/Configuration.xml"),
            "<MetaDataObject/>",
        );
        // A stray file that is not itself an extension source root must not be reported.
        write(&root.join("src/cfe/README.md"), "not an extension");

        let map = discover_project_source_map(&root).unwrap();

        assert_eq!(map.source_sets.len(), 3, "{:?}", map.source_sets);
        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            &["src/cf/Configuration.xml"],
        );
        assert_source_set(
            &map,
            "test_ed",
            SourceSetKind::Extension,
            SourceFormat::PlatformXml,
            &["src/cfe/test_ed/Configuration.xml"],
        );
        assert_source_set(
            &map,
            "test_edreg",
            SourceSetKind::Extension,
            SourceFormat::PlatformXml,
            &["src/cfe/test_edreg/Configuration.xml"],
        );
        assert_eq!(map.effective_source_set.as_deref(), Some("main"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn health_autodetect_discovers_configuration_extension_source_sets() {
        let root = temp_workspace("unica-source-map-health-autodetect-extensions");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        let mut checkpoint = || Ok(());

        let map = discover_project_source_map_controlled(&root, &mut checkpoint).unwrap();

        assert_eq!(map.source_sets.len(), 2, "{:?}", map.source_sets);
        assert_source_set(
            &map,
            "test_ed",
            SourceSetKind::Extension,
            SourceFormat::PlatformXml,
            &["src/cfe/test_ed/Configuration.xml"],
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_without_extensions_directory_keeps_reporting_only_main() {
        let root = temp_workspace("unica-source-map-autodetect-no-extensions");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");

        let map = discover_project_source_map(&root).unwrap();

        assert_eq!(map.source_sets.len(), 1, "{:?}", map.source_sets);
        assert_eq!(map.source_sets[0].name, "main");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn health_autodetect_enforces_combined_source_set_limit_including_main() {
        let root = temp_workspace("unica-source-map-health-autodetect-combined-limit");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        let mut checkpoint = || Ok(());
        let mut state = SourceMapDiscoveryState::health_with_source_set_limit(1);

        let error = discover_project_source_map_internal(&root, &mut checkpoint, &mut state)
            .expect_err("main (1) + one autodetected extension (1) exceeds a limit of 1");

        assert!(error.contains("limit") || error.contains('1'), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn health_autodetect_accepts_combined_source_set_count_at_the_limit() {
        let root = temp_workspace("unica-source-map-health-autodetect-limit-boundary");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        let mut checkpoint = || Ok(());
        let mut state = SourceMapDiscoveryState::health_with_source_set_limit(2);

        let map = discover_project_source_map_internal(&root, &mut checkpoint, &mut state)
            .expect("main (1) + one autodetected extension (1) is exactly the limit of 2");

        assert_eq!(map.source_sets.len(), 2, "{:?}", map.source_sets);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_skips_a_container_that_is_not_a_directory() {
        let root = temp_workspace("unica-source-map-autodetect-extensions-not-a-directory");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        // `src/cfe` exists as a regular file: a definite answer that no extension
        // container lives here, exactly like a link or an absent path. The layout
        // contributes nothing and the base configuration beside it is untouched —
        // collapsing the whole map instead would report an incomplete layout for a
        // workspace whose configuration is perfectly readable.
        write(&root.join("src/cfe"), "not a directory");

        assert_routes_agree(&root, &[("main", SourceSetKind::Configuration, "src/cf")]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_propagates_an_unreadable_extensions_container() {
        use crate::infrastructure::platform::testing::set_unix_mode_for_test;

        let root = temp_workspace("unica-source-map-autodetect-extensions-unreadable");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        let container = root.join("src/cfe");
        if !set_unix_mode_for_test(&container, 0o000).unwrap() {
            fs::remove_dir_all(&root).unwrap();
            return;
        }
        if std::fs::read_dir(&container).is_ok() {
            // Running with a privilege that ignores the mode; the case does not exist.
            set_unix_mode_for_test(&container, 0o755).unwrap();
            fs::remove_dir_all(&root).unwrap();
            return;
        }

        // A container that might hold extensions but could not be read is *not* a
        // definite "no extensions here". Reporting a map the filesystem never confirmed
        // would hide every extension behind one permission bit.
        let mut checkpoint = || Ok(());
        let ordinary = discover_project_source_map(&root)
            .expect_err("an unreadable extensions container must not be reported as empty");
        let hardened = discover_project_source_map_controlled(&root, &mut checkpoint)
            .expect_err("the hardened route must not report an unreadable container as empty");

        assert!(ordinary.contains("cfe"), "{ordinary}");
        assert!(
            hardened.contains("cfe") || hardened.contains("directory"),
            "{hardened}"
        );
        set_unix_mode_for_test(&container, 0o755).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_skips_extension_directory_named_main() {
        let root = temp_workspace("unica-source-map-autodetect-extension-named-main");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        // An extension directory literally named `main` would otherwise collide with the
        // base configuration's reserved name: named lookups (`resolve_named_source_set`)
        // would reject it as ambiguous while default selection would silently pick whichever
        // came first. Autodetection must not produce that collision.
        write(
            &root.join("src/cfe/main/Configuration.xml"),
            "<MetaDataObject/>",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert_eq!(map.source_sets.len(), 1, "{:?}", map.source_sets);
        assert_eq!(map.source_sets[0].kind, SourceSetKind::Configuration);
        fs::remove_dir_all(root).unwrap();
    }

    /// Both discovery routes must report the same source map for the same workspace.
    /// The ordinary and the hardened route are two implementations of one policy, and
    /// the defects this matrix pins down were all route drift: a marker list that grew
    /// on one side only, an entry filter that existed on one side only. Asserting the
    /// two agree on every supported layout is what keeps them from drifting again.
    fn assert_routes_agree(root: &Path, expected: &[(&str, SourceSetKind, &str)]) {
        let mut checkpoint = || Ok(());
        let ordinary = discover_project_source_map(root).unwrap_or_else(|error| {
            panic!("ordinary route failed for {}: {error}", root.display())
        });
        let hardened = discover_project_source_map_controlled(root, &mut checkpoint)
            .unwrap_or_else(|error| {
                panic!("hardened route failed for {}: {error}", root.display())
            });

        let render = |map: &ProjectSourceMap| {
            map.source_sets
                .iter()
                .map(|source_set| {
                    (
                        source_set.name.clone(),
                        source_set.kind,
                        source_set.path.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let expected = expected
            .iter()
            .map(|(name, kind, path)| ((*name).to_string(), *kind, (*path).to_string()))
            .collect::<Vec<_>>();
        assert_eq!(render(&ordinary), expected, "ordinary route");
        assert_eq!(render(&hardened), expected, "hardened route");
    }

    #[test]
    fn autodetect_skips_entries_that_are_not_extension_directories() {
        let root = temp_workspace("unica-source-map-autodetect-extensions-stray-entries");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        // A container holds whatever the workspace put there. `.gitkeep` is how git
        // carries an otherwise empty directory, `.DS_Store` appears from one Finder
        // visit, and neither is an extension. Discovery classifies and skips them; a
        // route that aborts on the first of them reports an incomplete layout for a
        // workspace whose layout is complete.
        write(&root.join("src/cfe/.gitkeep"), "");
        write(&root.join("src/cfe/.DS_Store"), "\0\0");
        write(&root.join("src/cfe/README.md"), "not an extension");

        assert_routes_agree(
            &root,
            &[
                ("main", SourceSetKind::Configuration, "src/cf"),
                ("test_ed", SourceSetKind::Extension, "src/cfe/test_ed"),
            ],
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_skips_linked_entries_in_the_extensions_container() {
        use crate::infrastructure::platform::testing::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let root = temp_workspace("unica-source-map-autodetect-extensions-link");
        let external = temp_workspace("unica-source-map-autodetect-extensions-link-external");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        write(&external.join("Configuration.xml"), "<MetaDataObject/>");
        fs::create_dir_all(root.join("src/cfe")).unwrap();
        match create_directory_link_fixture_for_test(&external, root.join("src/cfe/escaped"))
            .unwrap()
        {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                fs::remove_dir_all(&root).unwrap();
                fs::remove_dir_all(&external).unwrap();
                return;
            }
        }

        // A linked source root is rejected by `resolve_named_source_set`, so publishing
        // one would advertise a source set every `unica.code.*` tool then refuses to
        // open. It is not a candidate on either route, and it does not abort discovery
        // of the real extension beside it.
        assert_routes_agree(
            &root,
            &[
                ("main", SourceSetKind::Configuration, "src/cf"),
                ("test_ed", SourceSetKind::Extension, "src/cfe/test_ed"),
            ],
        );
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(external).unwrap();
    }

    #[test]
    fn autodetect_skips_a_linked_extensions_container() {
        use crate::infrastructure::platform::testing::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let root = temp_workspace("unica-source-map-autodetect-linked-container");
        let external = temp_workspace("unica-source-map-autodetect-linked-container-external");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &external.join("test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        fs::create_dir_all(root.join("src")).unwrap();
        match create_directory_link_fixture_for_test(&external, root.join("src/cfe")).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                fs::remove_dir_all(&root).unwrap();
                fs::remove_dir_all(&external).unwrap();
                return;
            }
        }

        // The container itself is a link, so there is no real container here and no
        // extension to autodetect. Enumerating through it would publish source roots
        // outside the workspace that `resolve_named_source_set` then refuses, and
        // failing would hide the base configuration that is perfectly fine.
        assert_routes_agree(&root, &[("main", SourceSetKind::Configuration, "src/cf")]);
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(external).unwrap();
    }

    #[test]
    fn autodetect_discovers_a_single_extension_at_the_container_root() {
        let root = temp_workspace("unica-source-map-autodetect-extension-at-container-root");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        // The canonical layout of this repository: `src/cfe` *is* one extension's dump
        // root (`ExtensionPath: "src/cfe"` in the cfe-* skills, and the `path: src/cfe`
        // fixture in `tests/ci/test_unica_mcp_script_parity.py`).
        write(&root.join("src/cfe/Configuration.xml"), "<MetaDataObject/>");

        assert_routes_agree(
            &root,
            &[
                ("main", SourceSetKind::Configuration, "src/cf"),
                ("cfe", SourceSetKind::Extension, "src/cfe"),
            ],
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_at_the_container_root_does_not_report_object_directories_as_extensions() {
        let root = temp_workspace("unica-source-map-autodetect-container-root-objects");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(&root.join("src/cfe/Configuration.xml"), "<MetaDataObject/>");
        // When the container is itself an extension root, its children are that
        // extension's object directories, not sibling extensions — exactly as the base
        // configuration scan stops at its first match.
        write(
            &root.join("src/cfe/Catalogs/Товары/Configuration.xml"),
            "<MetaDataObject/>",
        );

        assert_routes_agree(
            &root,
            &[
                ("main", SourceSetKind::Configuration, "src/cf"),
                ("cfe", SourceSetKind::Extension, "src/cfe"),
            ],
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_discovers_extensions_under_the_named_extensions_container() {
        let root = temp_workspace("unica-source-map-autodetect-extensions-container");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        // The second documented layout of this repository: `src/extensions/<Name>`
        // (`cfe-borrow`, `cfe-init` and `cfe-patch-method` all address it that way).
        write(
            &root.join("src/extensions/MyExtension/Configuration.xml"),
            "<MetaDataObject/>",
        );

        assert_routes_agree(
            &root,
            &[
                ("main", SourceSetKind::Configuration, "src/cf"),
                (
                    "MyExtension",
                    SourceSetKind::Extension,
                    "src/extensions/MyExtension",
                ),
            ],
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_discovers_edt_extension_source_sets() {
        let root = temp_workspace("unica-source-map-autodetect-edt-extensions");
        // The base scan recognises three markers; an extension scan that recognised
        // fewer would find a configuration in EDT layout and miss the identical
        // extension beside it.
        write(
            &root.join("src/cf/src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        );
        write(
            &root.join("src/cfe/test_ed/src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        );

        assert_routes_agree(
            &root,
            &[
                ("main", SourceSetKind::Configuration, "src/cf"),
                ("test_ed", SourceSetKind::Extension, "src/cfe/test_ed"),
            ],
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_publishes_an_extension_named_main_when_nothing_else_claims_the_name() {
        let root = temp_workspace("unica-source-map-autodetect-only-extension-named-main");
        // `main` is reserved for the base configuration, but reserving it when no base
        // configuration was found empties the map: the workspace's only source root
        // disappears and `unica.project.map` reports nothing at all.
        write(
            &root.join("src/cfe/main/Configuration.xml"),
            "<MetaDataObject/>",
        );

        assert_routes_agree(&root, &[("main", SourceSetKind::Extension, "src/cfe/main")]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_gives_the_reserved_name_to_one_claimant_only() {
        let root = temp_workspace("unica-source-map-autodetect-two-extensions-named-main");
        // No base configuration, and two layouts both derive `main`. Exactly one entry
        // may carry the reserved name: two would make `sourceSet: "main"` ambiguous for
        // named lookup while `select_default_source_set` silently kept resolving it to
        // whichever came first. The name goes to the first claimant in catalog order —
        // a fixed, documented order rather than a filesystem accident.
        write(
            &root.join("src/cfe/main/Configuration.xml"),
            "<MetaDataObject/>",
        );
        write(
            &root.join("src/extensions/main/Configuration.xml"),
            "<MetaDataObject/>",
        );

        assert_routes_agree(&root, &[("main", SourceSetKind::Extension, "src/cfe/main")]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_keeps_a_non_utf8_extension_name_from_hiding_its_siblings() {
        // The fixture lives behind the platform facade (ADR-0009): only some platforms
        // and filesystems accept a name that is not valid UTF-8, and none of that
        // knowledge belongs in a discovery test.
        let Some(invalid_component) =
            crate::infrastructure::platform::testing::non_utf8_relative_path_for_test()
        else {
            return;
        };

        let root = temp_workspace("unica-source-map-autodetect-non-utf8-extension");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        // A source-set name is a JSON string, so a directory name that is not valid
        // UTF-8 cannot become one. It is skipped deliberately — and a sibling that
        // cannot be named must not hide the ones that can.
        let invalid = root.join("src/cfe").join(&invalid_component);
        // APFS and NTFS refuse to store the name at all; the case only exists on a
        // filesystem that accepts arbitrary bytes, so skip where it cannot be built.
        if fs::create_dir_all(&invalid).is_err() {
            fs::remove_dir_all(&root).unwrap();
            return;
        }
        write(&invalid.join("Configuration.xml"), "<MetaDataObject/>");

        assert_routes_agree(
            &root,
            &[
                ("main", SourceSetKind::Configuration, "src/cf"),
                ("test_ed", SourceSetKind::Extension, "src/cfe/test_ed"),
            ],
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transactional_autodetect_binds_the_extensions_container_listing() {
        let root = temp_workspace("unica-source-map-autodetect-container-provenance");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );

        let (map, provenance) = discover_project_source_map_with_provenance(&root).unwrap();
        assert_eq!(map.source_sets.len(), 2, "{:?}", map.source_sets);
        // Discovery now depends on the *listing* of the container, not only on files it
        // read. A concurrent checkout that adds an extension between discovery and
        // commit changes the computed map while every recorded file input still agrees,
        // so the listing itself has to be bound to the transaction.
        write(
            &root.join("src/cfe/late_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );

        let error = provenance
            .bind_to(&mut CompileTransaction::new())
            .expect_err("an extension that appeared after discovery must invalidate the map");

        assert!(error.contains("cfe"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    /// The scenario matrix is derived from the production layout and marker
    /// catalogs. Exact catalog assertions make an intentional new layout a
    /// review event instead of allowing it to bypass the near-miss matrix.
    #[test]
    fn autodetect_catalog_contract_is_closed_over_production_layouts() {
        assert_eq!(BASE_CONFIGURATION_LAYOUTS, [".", "src", "src/cf"]);
        assert_eq!(
            SOURCE_ROOT_MARKERS,
            [
                "Configuration.xml",
                "Configuration/Configuration.mdo",
                "src/Configuration/Configuration.mdo",
            ]
        );
        assert_eq!(EXTENSION_LAYOUTS.len(), 2);
        assert_eq!(EXTENSION_LAYOUTS[0].directory, "src/cfe");
        assert_eq!(
            EXTENSION_LAYOUTS[0].shape,
            ExtensionLayoutShape::RootOrContainer
        );
        assert_eq!(EXTENSION_LAYOUTS[1].directory, "src/extensions");
        assert_eq!(EXTENSION_LAYOUTS[1].shape, ExtensionLayoutShape::Container);

        for layout in BASE_CONFIGURATION_LAYOUTS {
            for marker in SOURCE_ROOT_MARKERS {
                let root = temp_workspace(&format!(
                    "unica-source-map-catalog-base-{}-{}",
                    layout.replace('.', "root").replace('/', "root"),
                    marker.replace('/', "-")
                ));
                write(&root.join(layout).join(marker), "<marker/>");
                let winning_layout = BASE_CONFIGURATION_LAYOUTS
                    .iter()
                    .find(|candidate| {
                        SOURCE_ROOT_MARKERS.iter().any(|candidate_marker| {
                            root.join(candidate).join(candidate_marker).is_file()
                        })
                    })
                    .expect("the written marker belongs to a catalog layout");
                assert_routes_agree(
                    &root,
                    &[("main", SourceSetKind::Configuration, *winning_layout)],
                );
                fs::remove_dir_all(root).unwrap();
            }
        }

        for layout in EXTENSION_LAYOUTS {
            for marker in SOURCE_ROOT_MARKERS {
                let root = temp_workspace(&format!(
                    "unica-source-map-catalog-extension-{}-{}",
                    layout.directory.replace('/', "-"),
                    marker.replace('/', "-")
                ));
                let child = root.join(layout.directory).join("Named");
                write(&child.join(marker), "<marker/>");
                assert_routes_agree(
                    &root,
                    &[(
                        "Named",
                        SourceSetKind::Extension,
                        &format!("{}/Named", layout.directory),
                    )],
                );
                fs::remove_dir_all(root).unwrap();
            }
        }

        for near_miss in ["sources", "src/cfes", "src/extension"] {
            let root = temp_workspace(&format!(
                "unica-source-map-catalog-near-miss-{}",
                near_miss.replace('/', "-")
            ));
            write(
                &root.join(near_miss).join("Named/Configuration.xml"),
                "<marker/>",
            );
            assert_routes_agree(&root, &[]);
            fs::remove_dir_all(root).unwrap();
        }

        autodetect_skips_a_container_that_is_not_a_directory();
        autodetect_propagates_an_unreadable_extensions_container();
        autodetect_skips_entries_that_are_not_extension_directories();
        autodetect_skips_linked_entries_in_the_extensions_container();
        autodetect_skips_a_linked_extensions_container();
        autodetect_discovers_a_single_extension_at_the_container_root();
        autodetect_at_the_container_root_does_not_report_object_directories_as_extensions();
        autodetect_discovers_extensions_under_the_named_extensions_container();
        autodetect_gives_the_reserved_name_to_one_claimant_only();
        transactional_autodetect_binds_the_extensions_container_listing();
    }

    #[test]
    fn transactional_autodetect_binds_an_absent_extensions_container() {
        let root = temp_workspace("unica-source-map-autodetect-absent-container-provenance");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");

        let (map, provenance) = discover_project_source_map_with_provenance(&root).unwrap();
        assert_eq!(map.source_sets.len(), 1, "{:?}", map.source_sets);
        // The absence of the container is an input too: a checkout that creates it wins
        // a source set that discovery never saw.
        write(
            &root.join("src/cfe/late_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );

        let error = provenance
            .bind_to(&mut CompileTransaction::new())
            .expect_err(
                "an extensions container that appeared after discovery invalidates the map",
            );

        assert!(error.contains("cfe"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transactional_autodetect_tolerates_a_link_beside_the_extensions() {
        use crate::infrastructure::platform::testing::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let root = temp_workspace("unica-source-map-autodetect-container-provenance-link");
        let external = temp_workspace("unica-source-map-autodetect-container-provenance-external");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        fs::create_dir_all(external.join("payload")).unwrap();
        match create_directory_link_fixture_for_test(&external, root.join("src/cfe/linked"))
            .unwrap()
        {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                fs::remove_dir_all(&root).unwrap();
                fs::remove_dir_all(&external).unwrap();
                return;
            }
        }

        let (map, provenance) = discover_project_source_map_with_provenance(&root).unwrap();

        assert_eq!(map.source_sets.len(), 2, "{:?}", map.source_sets);
        // Discovery skipped the link, so the guard must not refuse to represent the
        // container that holds it: a guard that cannot be taken is a guarantee that is
        // silently absent.
        provenance
            .bind_to(&mut CompileTransaction::new())
            .expect("a link beside the extensions must not make the container unguardable");
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(external).unwrap();
    }

    #[test]
    fn transactional_autodetect_binds_a_container_path_that_is_not_a_directory() {
        let root = temp_workspace("unica-source-map-autodetect-file-container-provenance");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(&root.join("src/cfe"), "not a directory");

        let (map, provenance) = discover_project_source_map_with_provenance(&root).unwrap();

        assert_eq!(map.source_sets.len(), 1, "{:?}", map.source_sets);
        // Discovery read "no child directories here" from that path, and the guard has
        // to be able to say exactly that. A guard that cannot be taken turns every
        // compile transaction in the workspace into a planning failure.
        provenance
            .bind_to(&mut CompileTransaction::new())
            .expect("a non-directory container must still be representable as a guard");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transactional_autodetect_binds_a_linked_container_path() {
        use crate::infrastructure::platform::testing::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let root = temp_workspace("unica-source-map-autodetect-linked-container-provenance");
        let external = temp_workspace("unica-source-map-autodetect-linked-container-prov-external");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        fs::create_dir_all(external.join("test_ed")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        match create_directory_link_fixture_for_test(&external, root.join("src/cfe")).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                fs::remove_dir_all(&root).unwrap();
                fs::remove_dir_all(&external).unwrap();
                return;
            }
        }

        let (map, provenance) = discover_project_source_map_with_provenance(&root).unwrap();

        assert_eq!(map.source_sets.len(), 1, "{:?}", map.source_sets);
        provenance
            .bind_to(&mut CompileTransaction::new())
            .expect("a linked container must still be representable as a guard");
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(external).unwrap();
    }

    #[test]
    fn transactional_autodetect_catches_a_container_path_that_became_a_directory() {
        let root = temp_workspace("unica-source-map-autodetect-file-container-became-dir");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        let container = root.join("src/cfe");
        write(&container, "not a directory");

        let (_, provenance) = discover_project_source_map_with_provenance(&root).unwrap();
        // Recording "holds no child directories" for a non-directory path is only safe
        // while the transition that *would* change the map is still caught: replacing
        // the file with a real container wins source sets discovery never saw.
        fs::remove_file(&container).unwrap();
        write(
            &container.join("late_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );

        let error = provenance
            .bind_to(&mut CompileTransaction::new())
            .expect_err("a container that appeared after discovery must invalidate the map");

        assert!(error.contains("cfe"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transactional_autodetect_tolerates_stray_entries_beside_the_extensions() {
        let root = temp_workspace("unica-source-map-autodetect-container-provenance-stray");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        write(&root.join("src/cfe/.gitkeep"), "");

        let (_, provenance) = discover_project_source_map_with_provenance(&root).unwrap();

        // The guard binds what discovery consumed — which direct children are
        // directories — and nothing else. A stray file beside the extensions is not an
        // input, so it must not turn every compile transaction into a conflict.
        provenance
            .bind_to(&mut CompileTransaction::new())
            .expect("a stray file beside the extensions is not a discovery input");
        fs::remove_dir_all(root).unwrap();
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
    fn controlled_discovery_accepts_uppercase_external_xml_extension() {
        let root = temp_workspace("unica-source-map-uppercase-external-xml");
        write(
            &root.join("v8project.yaml"),
            r#"
format: EDT
source-set:
  - name: external-reports
    type: EXTERNAL_REPORTS
    path: erf
"#,
        );
        write(
            &root.join("erf/Report.XML"),
            "<MetaDataObject><ExternalReport/></MetaDataObject>",
        );
        let mut checkpoint = || Ok(());

        let map = discover_project_source_map_controlled(&root, &mut checkpoint).unwrap();

        assert_source_set(
            &map,
            "external-reports",
            SourceSetKind::ExternalReport,
            SourceFormat::PlatformXml,
            &["erf/Report.XML"],
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn controlled_discovery_skips_directory_with_external_xml_name() {
        let root = temp_workspace("unica-source-map-external-xml-directory");
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
            &root.join("erf/Report.xml"),
            "<MetaDataObject><ExternalReport/></MetaDataObject>",
        );
        fs::create_dir_all(root.join("erf/Archive.xml")).unwrap();
        let mut checkpoint = || Ok(());

        let map = discover_project_source_map_controlled(&root, &mut checkpoint).unwrap();

        assert_source_set(
            &map,
            "external-reports",
            SourceSetKind::ExternalReport,
            SourceFormat::PlatformXml,
            &["erf/Report.xml"],
        );
        assert!(map.source_sets[0].format_probe_error.is_none());
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
