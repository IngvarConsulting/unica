use crate::domain::project_sources::{
    config_dump_info_xml_kind, ConfigDumpInfoXmlKind, ProjectSourceMap, ProjectSourceSet,
    SourceFormat, SourceSetKind,
};
use crate::domain::source_roots::select_default_source_set;
use crate::infrastructure::native_operations::compile_transaction::CompileTransaction;
use crate::infrastructure::platform::filesystem::{
    host_path_text, is_link_loop_error, open_absolute_directory_path_nofollow,
    open_directory_child_nofollow, open_regular_child_nofollow, read_directory_names_bounded,
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
const EXTENSIONS_SOURCE_DIRECTORY: &str = "src/cfe";
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
        Ok(())
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
    for path in [".", "src", "src/cf"] {
        checkpoint()?;
        let root = workspace_root.join(path);
        let found = if state.require_nofollow_source_probe_route {
            if let Some(directory) = health_source_directory(workspace_root, path)? {
                let mut found = false;
                for marker in [
                    "Configuration.xml",
                    "Configuration/Configuration.mdo",
                    "src/Configuration/Configuration.mdo",
                ] {
                    match secure_regular_file_exists(&directory, Path::new(marker)) {
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
                found
            } else {
                false
            }
        } else {
            let mut found = false;
            for marker in [
                root.join("Configuration.xml"),
                root.join("Configuration/Configuration.mdo"),
                root.join("src/Configuration/Configuration.mdo"),
            ] {
                found |= snapshot_project_map_input(&marker, false, state, checkpoint)?.is_some();
            }
            found
        };
        if found {
            source_sets.push(ConfigSourceSet {
                name: "main".to_string(),
                kind: SourceSetKind::Configuration,
                path: path.to_string(),
                default_format: None,
            });
            break;
        }
    }
    let extension_sets = autodetect_extension_source_sets(workspace_root, state, checkpoint)?;
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

/// Autodetects configuration-extension source sets the same way `autodetect_source_sets`
/// autodetects the base configuration: a bare workspace with no `v8project.yaml` still gets
/// every `src/cfe/<name>/Configuration.xml` (or EDT `Configuration/Configuration.mdo`)
/// reported as its own `SourceSetKind::Extension` entry, named after `<name>`. Declared
/// `v8project.yaml` source sets take precedence over autodetection entirely (see
/// `discover_project_source_map_internal`), so this only runs for workspaces that rely on
/// autodetection in the first place.
fn autodetect_extension_source_sets(
    workspace_root: &Path,
    state: &mut SourceMapDiscoveryState,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<Vec<ConfigSourceSet>, String> {
    checkpoint()?;
    let mut names = if state.require_nofollow_source_probe_route {
        let Some(extensions_directory) =
            health_source_directory(workspace_root, EXTENSIONS_SOURCE_DIRECTORY)?
        else {
            return Ok(Vec::new());
        };
        let limit = state
            .max_source_sets
            .unwrap_or(MAX_HEALTH_SOURCE_SETS)
            .saturating_add(1);
        let mut checkpoint_failure = None;
        let names =
            read_directory_names_bounded(&extensions_directory, limit, || match checkpoint() {
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
                workspace_root.join(EXTENSIONS_SOURCE_DIRECTORY).display()
            )
        })?;
        // The combined total (this count plus the base configuration set, if any) is
        // enforced by the caller once both are known; `limit` above only bounds this
        // enumeration's own I/O cost.
        names
            .into_iter()
            .filter_map(|name| name.into_string().ok())
            .collect::<Vec<_>>()
    } else {
        let extensions_root = workspace_root.join(EXTENSIONS_SOURCE_DIRECTORY);
        match std::fs::read_dir(&extensions_root) {
            Ok(entries) => {
                let mut names = Vec::new();
                for entry in entries {
                    checkpoint()?;
                    let entry = entry.map_err(|error| {
                        format!(
                            "failed to inspect source directory {}: {error}",
                            extensions_root.display()
                        )
                    })?;
                    if entry.path().is_dir() {
                        if let Ok(name) = entry.file_name().into_string() {
                            names.push(name);
                        }
                    }
                }
                names
            }
            // Only an absent `src/cfe` means "no extensions here"; any other failure
            // (permissions, a non-directory in the way, ...) must be reported, not
            // silently treated the same as "there is nothing to autodetect".
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                return Err(format!(
                    "failed to inspect source directory {}: {error}",
                    extensions_root.display()
                ));
            }
        }
    };
    names.sort();

    let mut extension_sets = Vec::new();
    for name in names {
        checkpoint()?;
        // `main` is reserved for the base configuration set: an extension directory that
        // happens to be named `main` would collide with it (named lookups reject the
        // resulting duplicate as ambiguous, while default selection would silently pick
        // whichever came first) — see #559 review discussion. Skip it rather than publish
        // an ambiguous map; the workspace can still declare it explicitly, under a
        // different name, via `v8project.yaml`.
        if name == "main" {
            continue;
        }
        let relative_path = format!("{EXTENSIONS_SOURCE_DIRECTORY}/{name}");
        let found = if state.require_nofollow_source_probe_route {
            match health_source_directory(workspace_root, &relative_path)? {
                Some(directory) => {
                    let mut found = false;
                    for marker in ["Configuration.xml", "Configuration/Configuration.mdo"] {
                        match secure_regular_file_exists(&directory, Path::new(marker)) {
                            Ok(exists) => found |= exists,
                            Err(SecureMarkerProbeError::UnsafeOrWrongKind(_)) => {}
                            Err(SecureMarkerProbeError::Incomplete(reason)) => return Err(reason),
                        }
                    }
                    found
                }
                None => false,
            }
        } else {
            let root = workspace_root.join(&relative_path);
            let mut found = false;
            for marker in [
                root.join("Configuration.xml"),
                root.join("Configuration/Configuration.mdo"),
            ] {
                found |= snapshot_project_map_input(&marker, false, state, checkpoint)?.is_some();
            }
            found
        };
        if found {
            extension_sets.push(ConfigSourceSet {
                name,
                kind: SourceSetKind::Extension,
                path: relative_path,
                default_format: None,
            });
        }
    }
    Ok(extension_sets)
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
    let route = inspect_declared_source_root_route(workspace_root, configured_path)
        .map_err(|error| format!("source route could not be proven safe: {error}"))?;
    let physical_workspace = normalize_path_identity(workspace_root)?;
    let relative = route
        .lexical_path
        .strip_prefix(workspace_root)
        .map_err(|error| format!("source route is outside the workspace lexical root: {error}"))?;
    match open_absolute_directory_path_nofollow(&physical_workspace.join(relative)) {
        Ok(directory) => Ok(Some(directory)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "source directory could not be opened safely: {error}"
        )),
    }
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
    if matches!(
        error.kind(),
        std::io::ErrorKind::InvalidInput
            | std::io::ErrorKind::InvalidData
            | std::io::ErrorKind::NotADirectory
    ) || is_link_loop_error(&error)
    {
        SecureMarkerProbeError::UnsafeOrWrongKind(reason)
    } else {
        SecureMarkerProbeError::Incomplete(reason)
    }
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
    fn autodetect_propagates_non_missing_extensions_directory_errors() {
        let root = temp_workspace("unica-source-map-autodetect-extensions-not-a-directory");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        // `src/cfe` exists but is a regular file, not a directory: this must surface as an
        // error, not be silently treated the same as "no extensions directory at all".
        write(&root.join("src/cfe"), "not a directory");

        let error = discover_project_source_map(&root).expect_err(
            "src/cfe existing as a non-directory must not be silently treated as absent",
        );

        assert!(
            error.contains("cfe"),
            "error should name the offending directory: {error}"
        );
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
