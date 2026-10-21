//! One source-map grammar over the original ordinary, health and captured reads.
use crate::inventory::{classify_source_format, EDT_SOURCE_MARKERS};
use crate::probe::{config_dump_info_xml_kind, ConfigDumpInfoXmlKind};
use crate::selection::select_project_default_source_set;
use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_yaml::Value as YamlValue;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{c_char, CStr};
use std::fmt;
use std::io::Read;
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};
use unica_source_contract::discovery::{
    DirectoryCandidateError, DiscoveryDirectory as DirectoryReader, FollowMode,
    ProjectDiscoveryProvenance, ProjectDiscoveryRead,
};
use unica_source_contract::read::ReadFile;
use unica_source_contract::read::{
    SelectionDirectoryObservation, SelectionRegularObservation, SourceDirectory,
    SourceMapDiscovery, SourceSelectionReadPass,
};
use unica_source_contract::{
    ProjectSourceMap, ProjectSourceSet, SourceFormat, SourceSetKind, SourceSetState,
};
use unsafe_libyaml::{
    yaml_event_delete, yaml_event_t, yaml_parser_delete, yaml_parser_initialize, yaml_parser_parse,
    yaml_parser_set_input_string, yaml_parser_t, YAML_ALIAS_EVENT, YAML_DOCUMENT_START_EVENT,
    YAML_MAPPING_END_EVENT, YAML_MAPPING_START_EVENT, YAML_SCALAR_EVENT, YAML_SEQUENCE_END_EVENT,
    YAML_SEQUENCE_START_EVENT, YAML_STREAM_END_EVENT,
};

const PROJECT_SOURCE_MAP_READ_CHUNK_BYTES: usize = 64 * 1024;
/// The source-set name the base configuration owns. `INV-SOURCE-SINGLE-RESOLVED-ROOT`
/// makes it the deterministic winner of default selection, so exactly one entry may
mod layout_catalog {
    /// carry it.
    pub const BASE_CONFIGURATION_SOURCE_SET: &str = "main";

    /// Every marker that proves a directory is a source root, in both dump formats Unica
    /// reads. Autodetection probes this one list wherever it looks: a layout that
    /// recognised fewer markers than another would find a configuration in one place and
    /// miss the identical one in the next.
    pub const SOURCE_ROOT_MARKERS: &[&str] = &[
        "Configuration.xml",
        "Configuration/Configuration.mdo",
        "src/Configuration/Configuration.mdo",
    ];

    /// Where the base configuration is autodetected, in probe order. The first directory
    /// carrying a marker wins and stops the scan: the rest of the tree below it belongs to
    /// that configuration, not to a second one.
    pub const BASE_CONFIGURATION_LAYOUTS: &[&str] = &[".", "src", "src/cf"];

    /// Where configuration extensions are autodetected.
    ///
    /// Autodetection is a closed catalog, not a growing pile of probes: every place Unica
    /// is willing to find a source root without `v8project.yaml` is listed here once, and
    /// both discovery routes read the same list. Both entries are layouts this repository
    /// already documents — `src/cfe` is one extension's dump root in the `cfe-*` skills and
    /// a container of named extensions in a multi-extension workspace, while
    /// `src/extensions` is only ever a container.
    pub const EXTENSION_LAYOUTS: &[ExtensionLayout] = &[
        ExtensionLayout {
            directory: "src/cfe",
            shape: ExtensionLayoutShape::RootOrContainer,
        },
        ExtensionLayout {
            directory: "src/extensions",
            shape: ExtensionLayoutShape::Container,
        },
    ];

    pub struct ExtensionLayout {
        /// Workspace-relative directory this layout inspects.
        pub directory: &'static str,
        pub shape: ExtensionLayoutShape,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ExtensionLayoutShape {
        /// The directory is either one extension's dump root, or a container holding one
        /// extension per direct child directory.
        RootOrContainer,
        /// The directory only ever holds one extension per direct child directory.
        Container,
    }
}
use layout_catalog::*;

/// A directory autodetection probes, on whichever route discovery is running.
///
/// The hardened route must not resolve a path twice or follow a link, so it carries the
/// directory it already opened; the ordinary route probes by path and records every
/// probed path in provenance. Keeping both behind one type is what lets the layout
/// catalog, the marker list and the entry classification be written once instead of
/// once per route — the three places the two routes had already drifted apart.
enum ProbeDirectory {
    Path(PathBuf),
    Retained(DirectoryReader),
    Actor {
        relative: PathBuf,
        directory: SourceDirectory,
    },
}

#[derive(Debug, Clone)]
struct ConfigSourceSet {
    name: String,
    kind: SourceSetKind,
    path: String,
    default_format: Option<SourceFormat>,
}

#[derive(Debug, Default)]
struct HealthConfig {
    format: Option<YamlValue>,
    base_path: Option<YamlValue>,
    source_sets: HealthSourceSets,
}

#[derive(Debug, Default)]
enum HealthSourceSets {
    #[default]
    Empty,
    Sequence(Vec<YamlValue>),
    Mapping(Vec<(YamlValue, YamlValue)>),
}

struct HealthSourceSetsVisitor;

impl<'de> Visitor<'de> for HealthSourceSetsVisitor {
    type Value = HealthSourceSets;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a source-set list, mapping, or null")
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(HealthSourceSets::Empty)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(HealthSourceSets::Empty)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<YamlValue>()? {
            values.push(value);
        }
        Ok(HealthSourceSets::Sequence(values))
    }

    fn visit_map<A>(self, mut mapping: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some((key, value)) = mapping.next_entry::<YamlValue, YamlValue>()? {
            values.push((key, value));
        }
        Ok(HealthSourceSets::Mapping(values))
    }
}

impl<'de> Deserialize<'de> for HealthSourceSets {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(HealthSourceSetsVisitor)
    }
}

struct HealthConfigVisitor;

impl<'de> Visitor<'de> for HealthConfigVisitor {
    type Value = HealthConfig;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a v8project.yaml mapping")
    }

    fn visit_map<A>(self, mut mapping: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut result = HealthConfig::default();
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

impl<'de> Deserialize<'de> for HealthConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(HealthConfigVisitor)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProjectMapReadGrammar {
    FullValue,
    Health,
}

struct SourceMapDiscoveryState<'a> {
    read_grammar: ProjectMapReadGrammar,
    provenance: Option<&'a mut dyn ProjectDiscoveryProvenance>,
    reader: Option<&'a dyn ProjectDiscoveryRead>,
    host_text_fn: fn(String) -> String,
    max_source_sets: Option<usize>,
    max_input_bytes: Option<u64>,
    format_evidence_entry_limit: Option<usize>,
    format_evidence_byte_limit: Option<usize>,
    remaining_format_evidence_entries: Option<usize>,
    remaining_format_evidence_bytes: Option<usize>,
    require_nofollow_source_probe_route: bool,
    actor_pass: Option<&'a mut dyn SourceSelectionReadPass>,
}

impl<'a> SourceMapDiscoveryState<'a> {
    fn ordinary(reader: &'a dyn ProjectDiscoveryRead) -> Self {
        Self {
            provenance: None,
            reader: Some(reader),
            host_text_fn: reader.host_text_fn(),
            read_grammar: ProjectMapReadGrammar::FullValue,
            max_source_sets: None,
            max_input_bytes: None,
            format_evidence_entry_limit: None,
            format_evidence_byte_limit: None,
            remaining_format_evidence_entries: None,
            remaining_format_evidence_bytes: None,
            require_nofollow_source_probe_route: false,
            actor_pass: None,
        }
    }

    fn health(reader: &'a dyn ProjectDiscoveryRead) -> Self {
        Self {
            provenance: None,
            reader: Some(reader),
            host_text_fn: reader.host_text_fn(),
            read_grammar: ProjectMapReadGrammar::Health,
            max_source_sets: None,
            max_input_bytes: None,
            format_evidence_entry_limit: None,
            format_evidence_byte_limit: None,
            remaining_format_evidence_entries: None,
            remaining_format_evidence_bytes: None,
            require_nofollow_source_probe_route: true,
            actor_pass: None,
        }
    }

    fn transactional(
        reader: &'a dyn ProjectDiscoveryRead,
        provenance: &'a mut dyn ProjectDiscoveryProvenance,
    ) -> Self {
        Self {
            provenance: Some(provenance),
            reader: Some(reader),
            host_text_fn: reader.host_text_fn(),
            read_grammar: ProjectMapReadGrammar::FullValue,
            max_source_sets: None,
            max_input_bytes: None,
            format_evidence_entry_limit: None,
            format_evidence_byte_limit: None,
            remaining_format_evidence_entries: None,
            remaining_format_evidence_bytes: None,
            require_nofollow_source_probe_route: false,
            actor_pass: None,
        }
    }

    fn actor(
        pass: &'a mut dyn SourceSelectionReadPass,
        host_text_fn: fn(String) -> String,
    ) -> Self {
        Self {
            provenance: None,
            reader: None,
            host_text_fn,
            read_grammar: ProjectMapReadGrammar::Health,
            max_source_sets: None,
            max_input_bytes: None,
            format_evidence_entry_limit: None,
            format_evidence_byte_limit: None,
            remaining_format_evidence_entries: None,
            remaining_format_evidence_bytes: None,
            require_nofollow_source_probe_route: true,
            actor_pass: Some(pass),
        }
    }

    #[cfg(feature = "test-support")]
    fn health_with_evidence_limits(
        reader: &'a dyn ProjectDiscoveryRead,
        entries: Option<usize>,
        bytes: Option<usize>,
    ) -> Self {
        Self {
            format_evidence_entry_limit: entries,
            format_evidence_byte_limit: bytes,
            remaining_format_evidence_entries: entries,
            remaining_format_evidence_bytes: bytes,
            ..Self::health(reader)
        }
    }

    #[cfg(feature = "test-support")]
    fn health_with_source_set_limit(reader: &'a dyn ProjectDiscoveryRead, limit: usize) -> Self {
        Self {
            max_source_sets: Some(limit),
            ..Self::health(reader)
        }
    }

    fn reader(&self) -> &dyn ProjectDiscoveryRead {
        self.reader
            .expect("physical discovery route has an issued readonly session")
    }

    fn captures_provenance(&self) -> bool {
        self.provenance.is_some()
    }

    fn captures_actor_evidence(&self) -> bool {
        self.actor_pass.is_some()
    }

    fn record_exact_file(&mut self, path: PathBuf, raw: &[u8]) -> Result<(), String> {
        if let Some(provenance) = &mut self.provenance {
            provenance.record_exact_file(path, raw)?;
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
        names: Option<Vec<String>>,
    ) -> Result<(), String> {
        if let Some(provenance) = &mut self.provenance {
            provenance.record_directories(path, names.as_deref())?;
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

pub fn discover_project_source_map(
    reader: &dyn ProjectDiscoveryRead,
) -> Result<ProjectSourceMap, String> {
    discover_project_source_map_internal(
        reader.workspace_root(),
        &mut || Ok(()),
        &mut SourceMapDiscoveryState::ordinary(reader),
    )
}
pub fn discover_project_source_map_controlled(
    reader: &dyn ProjectDiscoveryRead,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<ProjectSourceMap, String> {
    discover_project_source_map_internal(
        reader.workspace_root(),
        checkpoint,
        &mut SourceMapDiscoveryState::health(reader),
    )
}
pub fn discover_project_source_map_with_provenance(
    reader: &dyn ProjectDiscoveryRead,
    provenance: &mut dyn ProjectDiscoveryProvenance,
) -> Result<ProjectSourceMap, String> {
    discover_project_source_map_internal(
        reader.workspace_root(),
        &mut || Ok(()),
        &mut SourceMapDiscoveryState::transactional(reader, provenance),
    )
}
pub struct ActorSourceMapDiscovery {
    host_text_fn: fn(String) -> String,
}
impl ActorSourceMapDiscovery {
    pub fn new(host_text_fn: fn(String) -> String) -> Self {
        Self { host_text_fn }
    }
}
impl SourceMapDiscovery for ActorSourceMapDiscovery {
    fn discover(
        &self,
        pass: &mut dyn SourceSelectionReadPass,
        checkpoint: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<ProjectSourceMap, String> {
        discover_project_source_map_actor_pass(pass, checkpoint, self.host_text_fn)
    }
}
pub fn discover_project_source_map_actor_pass(
    pass: &mut dyn SourceSelectionReadPass,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
    host_text_fn: fn(String) -> String,
) -> Result<ProjectSourceMap, String> {
    let workspace_root = pass.workspace_path().to_path_buf();
    discover_project_source_map_internal(
        &workspace_root,
        checkpoint,
        &mut SourceMapDiscoveryState::actor(pass, host_text_fn),
    )
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
            state.read_grammar,
            state.max_source_sets,
            checkpoint,
            state.host_text_fn,
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
        match select_project_default_source_set(&project_source_sets) {
            Ok(None) => (None, None, None),
            Ok(Some(source_set)) => {
                let normalized = if state.captures_actor_evidence() {
                    retained_actor_source_root_path(workspace_root, &source_set.path)
                } else {
                    state
                        .reader()
                        .normalize_contained_root(Path::new(&source_set.path))
                };
                match normalized {
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

fn retained_actor_source_root_path(
    workspace_root: &Path,
    configured_path: &str,
) -> Result<PathBuf, String> {
    let mut relative = PathBuf::new();
    for component in Path::new(configured_path).components() {
        match component {
            std::path::Component::Normal(name) => relative.push(name),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err(format!(
                    "configured source root is not a closed retained route: {configured_path}"
                ));
            }
        }
    }
    Ok(workspace_root.join(relative))
}

fn read_config_source_sets(
    workspace_root: &Path,
    config_path: &Path,
    raw: &[u8],
    read_grammar: ProjectMapReadGrammar,
    max_source_sets: Option<usize>,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
    host_path_text: fn(String) -> String,
) -> Result<(Vec<ConfigSourceSet>, Option<String>), String> {
    checkpoint()?;
    let text = std::str::from_utf8(raw)
        .map_err(|err| format!("failed to read {} as UTF-8: {err}", config_path.display()))?;
    if read_grammar == ProjectMapReadGrammar::Health {
        let (source_sets, configured_format_raw) = read_health_config_source_sets(
            workspace_root,
            config_path,
            text,
            checkpoint,
            host_path_text,
        )?;
        if let Some(limit) = max_source_sets {
            if source_sets.len() > limit {
                return Err(format!(
                    "project source-map declares {} source sets; health inspection supports at most {limit}",
                    source_sets.len()
                ));
            }
        }
        return Ok((source_sets, configured_format_raw));
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
    if let Some(limit) = max_source_sets {
        if declared_count > limit {
            return Err(format!(
                "project source-map declares {declared_count} source sets; health inspection supports at most {limit}"
            ));
        }
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
        source_set.path =
            normalize_configured_path(workspace_root, &base_path, &source_set.path, host_path_text);
    }

    Ok((source_sets, configured_format_raw))
}

fn read_health_config_source_sets(
    workspace_root: &Path,
    config_path: &Path,
    text: &str,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
    host_path_text: fn(String) -> String,
) -> Result<(Vec<ConfigSourceSet>, Option<String>), String> {
    validate_health_yaml_anchor_availability(text, checkpoint)?;
    let config = serde_yaml::from_str::<HealthConfig>(text)
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
        HealthSourceSets::Empty => {}
        HealthSourceSets::Sequence(entries) => {
            for entry in &entries {
                checkpoint()?;
                source_sets.push(config_source_set_from_yaml(entry, default_format)?);
            }
        }
        HealthSourceSets::Mapping(entries) => {
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
        source_set.path =
            normalize_configured_path(workspace_root, &base_path, &source_set.path, host_path_text);
    }
    Ok((source_sets, configured_format_raw))
}

fn validate_health_yaml_anchor_availability(
    text: &str,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<(), String> {
    struct ParserGuard(*mut yaml_parser_t);

    impl Drop for ParserGuard {
        fn drop(&mut self) {
            // SAFETY: the guard owns the initialized parser and its matching delete.
            unsafe { yaml_parser_delete(self.0) };
        }
    }

    struct EventGuard(*mut yaml_event_t);

    impl Drop for EventGuard {
        fn drop(&mut self) {
            // SAFETY: the guard owns the parsed event and its matching delete.
            unsafe { yaml_event_delete(self.0) };
        }
    }

    fn anchor_bytes(pointer: *const u8) -> Option<Vec<u8>> {
        (!pointer.is_null()).then(|| unsafe { CStr::from_ptr(pointer.cast()).to_bytes().to_vec() })
    }

    // Preserve the health route's END-bound availability check. An unknown
    // subtree is skipped by IgnoredAny, but an unresolved alias still refuses.
    // Do not replace this with a graph-wide cycle gate: a container may shadow
    // an earlier scalar anchor of the same name without expanding that subtree.
    unsafe {
        let mut parser = MaybeUninit::<yaml_parser_t>::uninit();
        let parser = parser.as_mut_ptr();
        if yaml_parser_initialize(parser).fail {
            return Err("failed to initialize YAML scanner for health inspection".into());
        }
        let _parser_guard = ParserGuard(parser);
        yaml_parser_set_input_string(parser, text.as_ptr(), text.len() as u64);
        let mut anchors = BTreeSet::<Vec<u8>>::new();
        let mut containers = Vec::<Option<Vec<u8>>>::new();
        loop {
            checkpoint()?;
            let mut event = MaybeUninit::<yaml_event_t>::uninit();
            let event = event.as_mut_ptr();
            if yaml_parser_parse(parser, event).fail {
                let problem_ptr = (&(*parser)).problem;
                let problem = if problem_ptr.is_null() {
                    "unknown YAML scanner error".into()
                } else {
                    CStr::from_ptr(problem_ptr.cast::<c_char>())
                        .to_string_lossy()
                        .into_owned()
                };
                return Err(format!(
                    "failed to parse YAML before health inspection: {problem}"
                ));
            }
            let _event_guard = EventGuard(event);
            match (*event).type_ {
                YAML_DOCUMENT_START_EVENT => {
                    anchors.clear();
                    containers.clear();
                }
                YAML_MAPPING_START_EVENT => {
                    containers.push(anchor_bytes((*event).data.mapping_start.anchor));
                }
                YAML_SEQUENCE_START_EVENT => {
                    containers.push(anchor_bytes((*event).data.sequence_start.anchor));
                }
                YAML_MAPPING_END_EVENT | YAML_SEQUENCE_END_EVENT => {
                    let anchor = containers.pop().ok_or_else(|| {
                        "YAML container ended without a matching start event".to_string()
                    })?;
                    if let Some(anchor) = anchor {
                        anchors.insert(anchor);
                    }
                }
                YAML_SCALAR_EVENT => {
                    if let Some(anchor) = anchor_bytes((*event).data.scalar.anchor) {
                        anchors.insert(anchor);
                    }
                }
                YAML_ALIAS_EVENT => {
                    let alias = anchor_bytes((*event).data.alias.anchor)
                        .ok_or_else(|| "YAML alias event has no anchor name".to_string())?;
                    if !anchors.contains(&alias) {
                        return Err(
                            "YAML alias refers to an unresolved or recursive anchor".to_string()
                        );
                    }
                }
                YAML_STREAM_END_EVENT => return Ok(()),
                _ => {}
            }
        }
    }
}

fn snapshot_project_map_input(
    path: &Path,
    read_contents: bool,
    state: &mut SourceMapDiscoveryState,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<Option<Vec<u8>>, String> {
    checkpoint()?;
    if state.captures_actor_evidence() {
        let actor_pass = state
            .actor_pass
            .as_mut()
            .expect("actor discovery state has a retained pass");
        let relative = path
            .strip_prefix(actor_pass.workspace_path())
            .map_err(|_| {
                format!(
                    "project source-map input escaped retained workspace: {}",
                    path.display()
                )
            })?;
        let observed = if read_contents {
            match state.max_input_bytes {
                Some(limit) => actor_pass.observe_regular(relative, limit as usize, checkpoint)?,
                None => actor_pass.observe_regular_unbounded(relative, checkpoint)?,
            }
        } else {
            actor_pass.observe_regular_presence(relative, checkpoint)?
        };
        return match observed {
            SelectionRegularObservation::Exact(raw) => {
                if read_contents {
                    Ok(Some(raw.as_ref().to_vec()))
                } else {
                    Ok(Some(Vec::new()))
                }
            }
            SelectionRegularObservation::Present if !read_contents => Ok(Some(Vec::new())),
            SelectionRegularObservation::Present => Err(format!(
                "project source-map input bytes were not retained: {}",
                path.display()
            )),
            SelectionRegularObservation::Oversized => Err(match state.max_input_bytes {
                Some(limit) => format!("project source-map input exceeds {limit} bytes"),
                None => {
                    "unbounded project source-map read unexpectedly reported oversized input".into()
                }
            }),
            SelectionRegularObservation::Absent => Ok(None),
            SelectionRegularObservation::WrongKind => Err(format!(
                "project source-map input is not a retained regular file: {}",
                path.display()
            )),
        };
    }
    if state.require_nofollow_source_probe_route {
        return snapshot_health_project_map_input(path, read_contents, state, checkpoint);
    }
    let metadata = match state.reader().inspect_input(path, FollowMode::NoFollow) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let identity = state.reader().normalize_identity(path)?;
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
    if metadata.is_link_or_reparse() {
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
    let mut file = state.reader().open_input(path).map_err(|error| {
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
    let identity = state.reader().normalize_identity(path)?;
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
    let physical_parent = state.reader().normalize_identity(parent)?;
    let directory = match state.reader().open_nofollow_directory(&physical_parent) {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            state.record_absence(state.reader().normalize_identity(path)?)?;
            return Ok(None);
        }
        Err(error) => {
            return Err(format!(
                "failed to open project source-map input parent {} safely: {error}",
                parent.display()
            ));
        }
    };
    let mut file = match directory.open_regular_child(file_name) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            state.record_absence(state.reader().normalize_identity(path)?)?;
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
    state.record_exact_file(state.reader().normalize_identity(path)?, &raw)?;
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

fn normalize_configured_path(
    workspace_root: &Path,
    base_path: &str,
    raw_path: &str,
    host_path_text: fn(String) -> String,
) -> String {
    let base = PathBuf::from(base_path);
    let path = PathBuf::from(raw_path);
    let resolved = if path.is_absolute() {
        path
    } else if base.is_absolute() {
        base.join(path)
    } else {
        workspace_root.join(base).join(path)
    };
    path_relative_to(workspace_root, &resolved, host_path_text)
}

fn autodetect_source_sets(
    workspace_root: &Path,
    state: &mut SourceMapDiscoveryState,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<Vec<ConfigSourceSet>, String> {
    let mut source_sets = Vec::new();
    for path in BASE_CONFIGURATION_LAYOUTS {
        checkpoint()?;
        let Some(directory) = open_probe_directory(workspace_root, path, state, checkpoint)? else {
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
        let Some(container) =
            open_extension_container(workspace_root, layout.directory, state, checkpoint)?
        else {
            state.record_directory_membership(workspace_root.join(layout.directory), None)?;
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
    state: &mut SourceMapDiscoveryState,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<Option<ProbeDirectory>, String> {
    if let Some(actor_pass) = state.actor_pass.as_mut() {
        return match actor_pass.observe_directory(Path::new(relative), checkpoint)? {
            SelectionDirectoryObservation::Present(directory) => Ok(Some(ProbeDirectory::Actor {
                relative: PathBuf::from(relative),
                directory,
            })),
            SelectionDirectoryObservation::AbsentOrWrongKind => Ok(None),
        };
    }
    if state.require_nofollow_source_probe_route {
        Ok(
            health_source_directory(state.reader(), workspace_root, relative)?
                .map(ProbeDirectory::Retained),
        )
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
    state: &mut SourceMapDiscoveryState,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<Option<ProbeDirectory>, String> {
    if let Some(actor_pass) = state.actor_pass.as_mut() {
        return match actor_pass.observe_directory(Path::new(relative), checkpoint)? {
            SelectionDirectoryObservation::Present(directory) => Ok(Some(ProbeDirectory::Actor {
                relative: PathBuf::from(relative),
                directory,
            })),
            SelectionDirectoryObservation::AbsentOrWrongKind => Ok(None),
        };
    }
    if !state.require_nofollow_source_probe_route {
        let path = workspace_root.join(relative);
        return match state.reader().inspect_input(&path, FollowMode::NoFollow) {
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
    if state.reader().inspect_declared_route(relative).is_err() {
        return Ok(None);
    }
    match health_source_directory_opened(state.reader(), workspace_root, relative)? {
        Ok(directory) => Ok(directory.map(ProbeDirectory::Retained)),
        Err(error) if open_failure_is_wrong_kind(state.reader(), &error) => Ok(None),
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
            ProbeDirectory::Actor { relative, .. } => {
                let marker = relative.join(marker);
                let observed = state
                    .actor_pass
                    .as_mut()
                    .expect("actor probe has retained evidence state")
                    .observe_regular_presence(&marker, checkpoint)?;
                found |= matches!(
                    observed,
                    SelectionRegularObservation::Exact(_) | SelectionRegularObservation::Present
                );
            }
            ProbeDirectory::Retained(handle) => {
                match secure_regular_file_exists(state.reader(), handle, Path::new(marker)) {
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
        ProbeDirectory::Actor {
            relative,
            directory,
        } => {
            let entries = state
                .actor_pass
                .as_mut()
                .expect("actor container has retained evidence state")
                .observe_membership_with_limit(
                    relative,
                    directory,
                    state.max_source_sets,
                    checkpoint,
                )?;
            let mut candidates = Vec::new();
            for entry in entries {
                let Some(directory) = entry.directory else {
                    continue;
                };
                let Ok(name) = entry.name.into_string() else {
                    continue;
                };
                candidates.push((
                    name.clone(),
                    ProbeDirectory::Actor {
                        relative: relative.join(&name),
                        directory,
                    },
                ));
            }
            Ok(candidates)
        }
        ProbeDirectory::Retained(handle) => {
            // An injected finite allowance preserves the combined-count refusal.
            // Production visits every immediate sibling through the retained handle.
            let limit = state.max_source_sets.map(|limit| limit.saturating_add(1));
            let mut checkpoint_failure = None;
            let names = handle.read_names_with_limit(limit, || match checkpoint() {
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
                let directory = match handle.open_directory_candidate(&name) {
                    Ok(Some(directory)) => directory,
                    Ok(None) => continue,
                    Err(DirectoryCandidateError::Inspect(error))
                        if error.kind() == std::io::ErrorKind::NotFound =>
                    {
                        continue
                    }
                    Err(DirectoryCandidateError::Inspect(error))
                        if open_failure_is_wrong_kind(state.reader(), &error) =>
                    {
                        continue
                    }
                    Err(DirectoryCandidateError::Inspect(error)) => {
                        return Err(format!(
                            "failed to inspect source directory entry {} securely: {error}",
                            container_path.join(&text).display()
                        ));
                    }
                    Err(DirectoryCandidateError::Open(error)) => {
                        return Err(format!(
                            "failed to open source directory entry {} securely: {error}",
                            container_path.join(&text).display()
                        ));
                    }
                };
                candidates.push((text, ProbeDirectory::Retained(directory)));
            }
            Ok(candidates)
        }
        ProbeDirectory::Path(path) => {
            let entries = match state.reader().read_entries(path) {
                Ok(entries) => entries,
                // Only an absent container means "no extensions here"; any other failure
                // (permissions, a regular file in the way, ...) must be reported, not
                // silently treated the same as "there is nothing to autodetect".
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    state.record_directory_membership(container_path, None)?;
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
                Some(candidates.iter().map(|(name, _)| name.clone()).collect()),
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
    let (platform_evidence, edt_evidence) = if state.captures_actor_evidence() {
        actor_format_evidence(
            workspace_root,
            &source_root,
            source_set.kind,
            state,
            checkpoint,
        )?
    } else if state.require_nofollow_source_probe_route {
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
    // Наблюдение — это доказательство из файлов. Всё, что добавится ниже,
    // приходит из объявления и наблюдением не является.
    let observed = !format_evidence.is_empty();
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

    // Состояние выводится из того, наблюдался ли формат, а не из самого
    // формата: без файловых доказательств формат берётся из объявления, и
    // приписывать набору наблюдённое состояние по заявленному формату значило
    // бы выдать заявку за факт.
    let source_state = if observed {
        if source_format.is_supported() {
            SourceSetState::Supported
        } else {
            SourceSetState::Unsupported
        }
    } else {
        SourceSetState::Declared
    };

    Ok(ProjectSourceSet {
        name: source_set.name,
        kind: source_set.kind,
        path: source_set.path,
        source_format,
        source_state,
        format_evidence,
        format_probe_error,
    })
}

fn actor_format_evidence(
    workspace_root: &Path,
    source_root: &Path,
    kind: SourceSetKind,
    state: &mut SourceMapDiscoveryState,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<(Vec<String>, Vec<String>), String> {
    checkpoint()?;
    let configured_path = PathBuf::from(path_relative_to(
        workspace_root,
        source_root,
        state.host_text_fn,
    ));
    let source_directory = {
        let actor_pass = state
            .actor_pass
            .as_mut()
            .expect("actor format discovery has retained evidence state");
        match actor_pass.observe_directory(&configured_path, checkpoint)? {
            SelectionDirectoryObservation::Present(directory) => directory,
            SelectionDirectoryObservation::AbsentOrWrongKind => {
                return Ok((Vec::new(), Vec::new()));
            }
        }
    };
    let mut platform = Vec::new();
    let mut edt = Vec::new();

    let configuration = configured_path.join("Configuration.xml");
    let observed = state
        .actor_pass
        .as_mut()
        .expect("actor format discovery has retained evidence state")
        .observe_regular_presence(&configuration, checkpoint)?;
    if matches!(
        observed,
        SelectionRegularObservation::Exact(_) | SelectionRegularObservation::Present
    ) {
        let evidence = path_relative_to(
            workspace_root,
            &workspace_root.join(&configuration),
            state.host_text_fn,
        );
        state.record_format_evidence(&evidence)?;
        platform.push(evidence);
    }

    for marker in EDT_SOURCE_MARKERS {
        checkpoint()?;
        let marker_path = configured_path.join(marker);
        let observed = state
            .actor_pass
            .as_mut()
            .expect("actor format discovery has retained evidence state")
            .observe_regular_presence(&marker_path, checkpoint)?;
        if matches!(
            observed,
            SelectionRegularObservation::Exact(_) | SelectionRegularObservation::Present
        ) {
            let evidence = path_relative_to(
                workspace_root,
                &workspace_root.join(&marker_path),
                state.host_text_fn,
            );
            state.record_format_evidence(&evidence)?;
            edt.push(evidence);
        }
    }

    if matches!(
        kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        let evidence_limit = state.remaining_format_evidence_entries;
        let pass = state
            .actor_pass
            .as_mut()
            .expect("actor format discovery has retained evidence state");
        // Non-marker entries consume actor work too, so the remaining format
        // evidence allowance can exceed the remaining enumeration capacity.
        let limit = match (evidence_limit, pass.remaining_member_budget()) {
            (Some(evidence), Some(members)) => Some(evidence.min(members)),
            (Some(limit), None) | (None, Some(limit)) => Some(limit),
            (None, None) => None,
        };
        let entries = pass.observe_membership_with_limit(
            &configured_path,
            &source_directory,
            limit,
            checkpoint,
        )?;
        for entry in entries {
            checkpoint()?;
            let path = Path::new(&entry.name);
            if !path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
            {
                continue;
            }
            let relative = configured_path.join(&entry.name);
            let config_dump_info = has_config_dump_info_filename(path);
            let observed = if config_dump_info {
                state
                    .actor_pass
                    .as_mut()
                    .expect("actor format discovery has retained evidence state")
                    .observe_regular_unbounded(&relative, checkpoint)?
            } else {
                state
                    .actor_pass
                    .as_mut()
                    .expect("actor format discovery has retained evidence state")
                    .observe_regular_presence(&relative, checkpoint)?
            };
            match observed {
                SelectionRegularObservation::Exact(bytes) => {
                    if config_dump_info
                        && !matches!(
                            (config_dump_info_xml_kind(&bytes), kind),
                            (
                                ConfigDumpInfoXmlKind::ExternalProcessor,
                                SourceSetKind::ExternalProcessor
                            ) | (
                                ConfigDumpInfoXmlKind::ExternalReport,
                                SourceSetKind::ExternalReport
                            )
                        )
                    {
                        continue;
                    }
                }
                SelectionRegularObservation::Present if !config_dump_info => {}
                SelectionRegularObservation::Absent | SelectionRegularObservation::WrongKind => {
                    continue;
                }
                SelectionRegularObservation::Present | SelectionRegularObservation::Oversized => {
                    continue;
                }
            }
            let evidence = path_relative_to(
                workspace_root,
                &workspace_root.join(&relative),
                state.host_text_fn,
            );
            state.record_format_evidence(&evidence)?;
            platform.push(evidence);
        }
    }
    platform.sort();
    platform.dedup();
    edt.sort();
    edt.dedup();
    checkpoint()?;
    Ok((platform, edt))
}

fn health_format_evidence(
    workspace_root: &Path,
    source_root: &Path,
    kind: SourceSetKind,
    state: &mut SourceMapDiscoveryState,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<(Vec<String>, Vec<String>), String> {
    checkpoint()?;
    let configured_path = path_relative_to(workspace_root, source_root, state.host_text_fn);
    let Some(source_directory) =
        health_source_directory(state.reader(), workspace_root, &configured_path)?
    else {
        return Ok((Vec::new(), Vec::new()));
    };
    let mut platform = Vec::new();
    let mut edt = Vec::new();
    if secure_regular_file_exists(
        state.reader(),
        &source_directory,
        Path::new("Configuration.xml"),
    )
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
        if secure_regular_file_exists(state.reader(), &source_directory, Path::new(marker))
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
            .map(|remaining| {
                remaining.checked_add(1).ok_or_else(|| {
                    "project source-map format evidence allowance overflow".to_owned()
                })
            })
            .transpose()?;
        let mut checkpoint_failure = None;
        let names = source_directory.read_names_with_limit(limit, || match checkpoint() {
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
            let mut file = match source_directory.open_regular_child(&name) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => match marker_probe_open_error(state.reader(), "file", error) {
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
    reader: &dyn ProjectDiscoveryRead,
    workspace_root: &Path,
    configured_path: &str,
) -> Result<Option<DirectoryReader>, String> {
    health_source_directory_opened(reader, workspace_root, configured_path)?
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
    reader: &dyn ProjectDiscoveryRead,
    workspace_root: &Path,
    configured_path: &str,
) -> Result<Result<Option<DirectoryReader>, std::io::Error>, String> {
    let route = reader
        .inspect_declared_route(configured_path)
        .map_err(|error| format!("source route could not be proven safe: {error}"))?;
    let physical_workspace = reader.normalize_identity(workspace_root)?;
    let relative = route
        .lexical_path
        .strip_prefix(workspace_root)
        .map_err(|error| format!("source route is outside the workspace lexical root: {error}"))?;
    Ok(
        match reader.open_nofollow_directory(&physical_workspace.join(relative)) {
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
    reader: &dyn ProjectDiscoveryRead,
    source_directory: &DirectoryReader,
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
            let directory = match parent.open_directory_child(name) {
                Ok(directory) => directory,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(marker_probe_open_error(reader, "parent", error)),
            };
            parent = directory;
        } else {
            return match parent.open_regular_child(name) {
                Ok(_) => Ok(true),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(marker_probe_open_error(reader, "file", error)),
            };
        }
    }
    Ok(false)
}

fn marker_probe_open_error(
    reader: &dyn ProjectDiscoveryRead,
    context: &str,
    error: std::io::Error,
) -> SecureMarkerProbeError {
    let reason = format!("source format marker {context} could not be opened safely: {error}");
    if open_failure_is_wrong_kind(reader, &error) {
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
fn open_failure_is_wrong_kind(reader: &dyn ProjectDiscoveryRead, error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::InvalidInput
            | std::io::ErrorKind::InvalidData
            | std::io::ErrorKind::NotADirectory
    ) || reader.is_link_loop_error(error)
}

fn push_health_evidence(
    evidence: &mut Vec<String>,
    workspace_root: &Path,
    path: &Path,
    state: &mut SourceMapDiscoveryState,
) -> Result<(), String> {
    let relative = path_relative_to(workspace_root, path, state.host_text_fn);
    state.record_format_evidence(&relative)?;
    evidence.push(relative);
    Ok(())
}

fn secure_config_dump_info_is_source_descriptor(
    file: &mut ReadFile,
    kind: SourceSetKind,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<bool, String> {
    let metadata = file
        .metadata()
        .map_err(|error| format!("source descriptor metadata failed: {error}"))?;
    if !metadata.is_file() {
        return Ok(false);
    }
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; PROJECT_SOURCE_MAP_READ_CHUNK_BYTES];
    loop {
        checkpoint()?;
        let read = file
            .read(&mut chunk)
            .map_err(|error| format!("source descriptor read failed: {error}"))?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
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
        if let Ok(entries) = state.reader().read_entries(source_root) {
            for entry in entries.flatten() {
                checkpoint()?;
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("xml")
                    && !is_config_dump_info_sidecar(&path, kind, state.reader(), checkpoint)?
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
    reader: &dyn ProjectDiscoveryRead,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<bool, String> {
    if !has_config_dump_info_filename(path) {
        return Ok(false);
    }
    Ok(!matches!(
        (
            config_dump_info_xml_file_kind(path, reader, checkpoint)?,
            kind
        ),
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
    reader: &dyn ProjectDiscoveryRead,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<ConfigDumpInfoXmlKind, String> {
    checkpoint()?;
    if !has_config_dump_info_filename(path) {
        return Ok(ConfigDumpInfoXmlKind::Other);
    }
    let Ok(link_metadata) = reader.inspect_input(path, FollowMode::NoFollow) else {
        return Ok(ConfigDumpInfoXmlKind::Other);
    };
    if link_metadata.file_type().is_symlink() || !link_metadata.is_file() {
        return Ok(ConfigDumpInfoXmlKind::Other);
    }
    let Ok(mut file) = reader.open_input(path) else {
        return Ok(ConfigDumpInfoXmlKind::Other);
    };
    let Ok(metadata) = file.metadata() else {
        return Ok(ConfigDumpInfoXmlKind::Other);
    };
    if !metadata.is_file() {
        return Ok(ConfigDumpInfoXmlKind::Other);
    }
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; PROJECT_SOURCE_MAP_READ_CHUNK_BYTES];
    loop {
        checkpoint()?;
        let Ok(read) = file.read(&mut chunk) else {
            return Ok(ConfigDumpInfoXmlKind::Other);
        };
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        checkpoint()?;
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
        let relative = path_relative_to(workspace_root, path, state.host_text_fn);
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
    if state.reader().path_is_file(path) {
        let relative = path_relative_to(workspace_root, path, state.host_text_fn);
        state.record_format_evidence(&relative)?;
        evidence.push(relative);
    }
    Ok(())
}

fn path_relative_to(root: &Path, path: &Path, host_path_text: fn(String) -> String) -> String {
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

#[cfg(feature = "test-support")]
pub mod test_support {
    pub use super::layout_catalog::{
        ExtensionLayoutShape, BASE_CONFIGURATION_LAYOUTS, EXTENSION_LAYOUTS, SOURCE_ROOT_MARKERS,
    };
    use super::*;
    pub struct DiscoveryFixtureState<'a>(SourceMapDiscoveryState<'a>);
    impl<'a> DiscoveryFixtureState<'a> {
        pub fn transactional(
            reader: &'a dyn ProjectDiscoveryRead,
            provenance: &'a mut dyn ProjectDiscoveryProvenance,
        ) -> Self {
            Self(SourceMapDiscoveryState::transactional(reader, provenance))
        }
        pub fn health_with_evidence_limits(
            reader: &'a dyn ProjectDiscoveryRead,
            entries: Option<usize>,
            bytes: Option<usize>,
        ) -> Self {
            Self(SourceMapDiscoveryState::health_with_evidence_limits(
                reader, entries, bytes,
            ))
        }
        pub fn health_with_source_set_limit(
            reader: &'a dyn ProjectDiscoveryRead,
            limit: usize,
        ) -> Self {
            Self(SourceMapDiscoveryState::health_with_source_set_limit(
                reader, limit,
            ))
        }
        pub fn discover(
            &mut self,
            root: &Path,
            checkpoint: &mut dyn FnMut() -> Result<(), String>,
        ) -> Result<ProjectSourceMap, String> {
            discover_project_source_map_internal(root, checkpoint, &mut self.0)
        }
        pub fn snapshot_input(
            &mut self,
            path: &Path,
            read_contents: bool,
            checkpoint: &mut dyn FnMut() -> Result<(), String>,
        ) -> Result<Option<Vec<u8>>, String> {
            snapshot_project_map_input(path, read_contents, &mut self.0, checkpoint)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const HEALTH_LEAF_MAIN_CONFIG: &str = concat!(
        "format: DESIGNER\n",
        "source-set:\n",
        "  - name: main\n",
        "    type: CONFIGURATION\n",
        "    path: src\n"
    );
    #[test]
    fn health_yaml_visitor_retains_a_selected_value_past_two_mib() {
        let padding = "x".repeat(2 * 1024 * 1024 + 1);
        let config = format!("{HEALTH_LEAF_MAIN_CONFIG}    ignored: '{padding}'\n");
        let parsed = serde_yaml::from_str::<HealthConfig>(&config)
            .expect("the actual health visitor must retain the selected value past2MiB");
        let HealthSourceSets::Sequence(entries) = parsed.source_sets else {
            panic!("source-set sequence changed its grammar");
        };
        let [entry] = entries.as_slice() else {
            panic!("expected exactly one useful selected source");
        };
        assert_eq!(yaml_string(entry, "name").as_deref(), Some("main"));
        assert_eq!(yaml_string(entry, "path").as_deref(), Some("src"));
        assert_eq!(yaml_string(entry, "ignored"), Some(padding));
    }
}
