use crate::domain::cancellation::CancellationToken;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::{Path, PathBuf};

pub(crate) const MAX_ARTIFACT_ID_BYTES: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub(crate) struct ArtifactId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArtifactIdError {
    InvalidFormat,
    NormalizedBytesOutOfRange,
}

pub(crate) fn normalize_discovery_identity(value: &str) -> String {
    value.trim().chars().flat_map(char::to_lowercase).collect()
}

impl ArtifactId {
    pub(crate) fn parse(value: &str) -> Result<Self, ArtifactIdError> {
        let trimmed = value.trim();
        if trimmed.contains(['/', '\\']) || trimmed.starts_with('.') || trimmed.ends_with('.') {
            return Err(ArtifactIdError::InvalidFormat);
        }

        let mut segments = trimmed.split('.');
        let Some(kind) = segments.next() else {
            return Err(ArtifactIdError::InvalidFormat);
        };
        let Some(name) = segments.next() else {
            return Err(ArtifactIdError::InvalidFormat);
        };
        if kind.is_empty() || name.is_empty() || segments.any(str::is_empty) {
            return Err(ArtifactIdError::InvalidFormat);
        }

        let normalized = normalize_discovery_identity(trimmed);
        if !(1..=MAX_ARTIFACT_ID_BYTES).contains(&normalized.len()) {
            return Err(ArtifactIdError::NormalizedBytesOutOfRange);
        }
        Ok(Self(normalized))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ArtifactIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat => formatter.write_str("invalid canonical artifact identifier"),
            Self::NormalizedBytesOutOfRange => formatter
                .write_str("normalized artifact identifier must contain 1..=1024 UTF-8 bytes"),
        }
    }
}

impl std::error::Error for ArtifactIdError {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub(crate) struct PortableRelativePath(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PortableRelativePathError {
    NonUtf8,
    Empty,
    Absolute,
    AmbiguousComponent,
    UnsafeComponent,
}

impl PortableRelativePath {
    pub(crate) fn parse(path: &Path) -> Result<Self, PortableRelativePathError> {
        let value = path.to_str().ok_or(PortableRelativePathError::NonUtf8)?;
        Self::parse_str(value)
    }

    pub(crate) fn parse_str(value: &str) -> Result<Self, PortableRelativePathError> {
        if value.is_empty() {
            return Err(PortableRelativePathError::Empty);
        }
        if value.starts_with(['/', '\\']) {
            return Err(PortableRelativePathError::Absolute);
        }

        let portable = value.replace('\\', "/");
        let mut components = Vec::new();
        for component in portable.split('/') {
            match component {
                "" | "." | ".." => {
                    return Err(PortableRelativePathError::AmbiguousComponent);
                }
                normal if portable_path_component_is_valid(normal) => components.push(normal),
                _ => return Err(PortableRelativePathError::UnsafeComponent),
            }
        }
        Ok(Self(components.join("/")))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PortableRelativePathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonUtf8 => formatter.write_str("evidence path must be valid UTF-8"),
            Self::Empty => formatter.write_str("evidence path must not be empty"),
            Self::Absolute => formatter.write_str("evidence path must be relative"),
            Self::AmbiguousComponent => formatter
                .write_str("evidence path must not contain empty, current, or parent components"),
            Self::UnsafeComponent => {
                formatter.write_str("evidence path contains a non-portable component")
            }
        }
    }
}

impl std::error::Error for PortableRelativePathError {}

fn portable_path_component_is_valid(component: &str) -> bool {
    !component.ends_with(['.', ' '])
        && !component.chars().any(|character| {
            character.is_ascii_control()
                || matches!(character, '<' | '>' | '"' | '|' | '?' | '*' | ':')
        })
        && !is_reserved_win32_path_component(component)
}

fn is_reserved_win32_path_component(component: &str) -> bool {
    let basename = match component.split_once('.') {
        Some((basename, _extension)) => basename,
        None => component,
    };
    let basename = basename.to_ascii_uppercase();
    if matches!(basename.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    ["COM", "LPT"].iter().any(|prefix| {
        basename.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}

macro_rules! digest_newtype {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub(crate) struct $name(String);

        impl $name {
            pub(crate) fn as_str(&self) -> &str {
                &self.0
            }

            fn from_hasher(hasher: Sha256) -> Self {
                Self(format!("{:x}", hasher.finalize()))
            }
        }
    };
}

digest_newtype!(EvidenceId);
digest_newtype!(ContentHash);
digest_newtype!(MappingFingerprint);
digest_newtype!(SnapshotFingerprint);

impl ContentHash {
    pub(crate) fn sha256(bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        Self::from_hasher(hasher)
    }
}

impl MappingFingerprint {
    pub(crate) fn from_identity(identity: &str) -> Self {
        let mut hasher = StableHasher::new("unica.discovery.mapping.v1");
        hasher.field(identity.as_bytes());
        Self::from_hasher(hasher.finish())
    }
}

impl EvidenceId {
    pub(crate) fn from_fact(
        provider: ProviderKind,
        kind: EvidenceKind,
        target: &ArtifactId,
        relation: &EvidenceRelation,
        location: &EvidenceLocation,
        raw_hash: &ContentHash,
    ) -> Self {
        let mut hasher = StableHasher::new("unica.discovery.evidence.v1");
        hasher.field(provider.stable_name().as_bytes());
        hasher.field(kind.stable_name().as_bytes());
        hasher.field(target.as_str().as_bytes());
        hasher.field(relation.stable_name().as_bytes());
        hasher.field(location.relative_path.as_str().as_bytes());
        hasher.optional_u32(location.line);
        hasher.optional_u32(location.column);
        hasher.optional_str(location.xml_path.as_deref());
        hasher.field(raw_hash.as_str().as_bytes());
        Self::from_hasher(hasher.finish())
    }
}

impl SnapshotFingerprint {
    pub(crate) fn from_manifest(
        mapping: &MappingFingerprint,
        contributors: &[AnalyzedFile],
    ) -> Self {
        let mut sorted = contributors.to_vec();
        sorted.sort();
        sorted.dedup();

        let mut hasher = StableHasher::new("unica.discovery.snapshot.v1");
        hasher.field(mapping.as_str().as_bytes());
        for contributor in sorted {
            hasher.field(contributor.relative_path.as_str().as_bytes());
            hasher.field(contributor.raw_hash.as_str().as_bytes());
            hasher.field(&contributor.bytes.to_be_bytes());
        }
        Self::from_hasher(hasher.finish())
    }
}

struct StableHasher(Sha256);

impl StableHasher {
    fn new(domain: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update((domain.len() as u64).to_be_bytes());
        hasher.update(domain.as_bytes());
        Self(hasher)
    }

    fn field(&mut self, value: &[u8]) {
        self.0.update((value.len() as u64).to_be_bytes());
        self.0.update(value);
    }

    fn optional_u32(&mut self, value: Option<u32>) {
        match value {
            Some(value) => {
                self.field(b"some");
                self.field(&value.to_be_bytes());
            }
            None => self.field(b"none"),
        }
    }

    fn optional_str(&mut self, value: Option<&str>) {
        match value {
            Some(value) => {
                self.field(b"some");
                self.field(value.as_bytes());
            }
            None => self.field(b"none"),
        }
    }

    fn finish(self) -> Sha256 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArtifactKind {
    MetadataObject,
    TabularSection,
    Attribute,
    Form,
    FormControl,
    Command,
    Module,
    Method,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConceptProvenance {
    TaskDerived,
    Explicit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderKind {
    SourceInventory,
    MetadataCatalog,
    ManagedForms,
    BslSearch,
    Definitions,
    RuntimeFlow,
    SupportState,
}

impl ProviderKind {
    fn stable_name(self) -> &'static str {
        match self {
            Self::SourceInventory => "source_inventory",
            Self::MetadataCatalog => "metadata_catalog",
            Self::ManagedForms => "managed_forms",
            Self::BslSearch => "bsl_search",
            Self::Definitions => "definitions",
            Self::RuntimeFlow => "runtime_flow",
            Self::SupportState => "support_state",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderOutcomeKind {
    Complete,
    Bounded,
    Unavailable,
    Failed,
    ContractViolation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DiscoveryStatus {
    Complete,
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StructuralRelationKind {
    Contains,
    Defines,
    DataBinding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuntimeFlowRelationKind {
    Callback,
    Action,
    EventSubscription,
    Calls,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceKind {
    Metadata,
    FormBinding,
    Lexical,
    Definition,
    RuntimeFlow,
    SupportState,
}

impl EvidenceKind {
    fn stable_name(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::FormBinding => "form_binding",
            Self::Lexical => "lexical",
            Self::Definition => "definition",
            Self::RuntimeFlow => "runtime_flow",
            Self::SupportState => "support_state",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SupportStateKind {
    NotOnSupport,
    Locked,
    Editable,
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MissingCheckMateriality {
    Material,
    NonMaterial,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "category", content = "kind", rename_all = "snake_case")]
pub(crate) enum EvidenceRelation {
    None,
    Structural(StructuralRelationKind),
    RuntimeFlow(RuntimeFlowRelationKind),
}

impl EvidenceRelation {
    fn stable_name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Structural(StructuralRelationKind::Contains) => "structural:contains",
            Self::Structural(StructuralRelationKind::Defines) => "structural:defines",
            Self::Structural(StructuralRelationKind::DataBinding) => "structural:data_binding",
            Self::RuntimeFlow(RuntimeFlowRelationKind::Callback) => "runtime_flow:callback",
            Self::RuntimeFlow(RuntimeFlowRelationKind::Action) => "runtime_flow:action",
            Self::RuntimeFlow(RuntimeFlowRelationKind::EventSubscription) => {
                "runtime_flow:event_subscription"
            }
            Self::RuntimeFlow(RuntimeFlowRelationKind::Calls) => "runtime_flow:calls",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceFile {
    pub relative_path: PortableRelativePath,
    pub bytes: Vec<u8>,
    pub raw_hash: ContentHash,
}

impl SourceFile {
    pub(crate) fn analyzed_file(&self) -> AnalyzedFile {
        AnalyzedFile {
            relative_path: self.relative_path.clone(),
            raw_hash: self.raw_hash.clone(),
            bytes: self.bytes.len() as u64,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyzedFile {
    pub relative_path: PortableRelativePath,
    pub raw_hash: ContentHash,
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderCoverage {
    /// Provider-eligible files observed for this exact query.
    pub files_seen: u32,
    /// Exact identity count returned as inventory files or analyzed fact files.
    pub files_analyzed: u32,
    /// Exact byte sum of returned inventory or analyzed-file identities.
    pub bytes_analyzed: u64,
    /// Exact number of returned inventory files or full pre-truncation facts.
    pub records: u32,
}

impl ProviderCoverage {
    pub(crate) const fn new(
        files_seen: u32,
        files_analyzed: u32,
        bytes_analyzed: u64,
        records: u32,
    ) -> Self {
        Self {
            files_seen,
            files_analyzed,
            bytes_analyzed,
            records,
        }
    }

    pub(crate) const fn empty() -> Self {
        Self::new(0, 0, 0, 0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceInventory {
    pub files: Vec<SourceFile>,
    pub coverage: ProviderCoverage,
}

impl SourceInventory {
    pub(crate) fn empty() -> Self {
        Self {
            files: Vec::new(),
            coverage: ProviderCoverage::empty(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FactBatch<T> {
    pub records: Vec<T>,
    /// Exact analyzed identities, normalized to sorted path order before promotion.
    pub analyzed_files: Vec<AnalyzedFile>,
    /// Evidence-only identities whose paths occur in `records`.
    pub contributors: Vec<AnalyzedFile>,
    pub coverage: ProviderCoverage,
}

impl<T> FactBatch<T> {
    pub(crate) fn empty() -> Self {
        Self {
            records: Vec::new(),
            analyzed_files: Vec::new(),
            contributors: Vec::new(),
            coverage: ProviderCoverage::empty(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct MetadataFact {
    pub artifact: ArtifactId,
    pub search_name: String,
    pub artifact_kind: ArtifactKind,
    pub container: Option<ArtifactId>,
    pub container_kind: Option<ArtifactKind>,
    pub relation: StructuralRelationKind,
    pub location: EvidenceLocation,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FormBinding {
    Data {
        target: ArtifactId,
        target_kind: ArtifactKind,
        data_path: String,
    },
    Command {
        command: String,
        handler: String,
        target: ArtifactId,
        target_kind: ArtifactKind,
    },
    Event {
        event: String,
        handler: String,
        target: ArtifactId,
        target_kind: ArtifactKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FormFact {
    pub form: ArtifactId,
    pub binding: FormBinding,
    pub location: EvidenceLocation,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct BslFact {
    pub artifact: ArtifactId,
    pub artifact_kind: ArtifactKind,
    pub matched_text: String,
    pub location: EvidenceLocation,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DefinitionFact {
    pub owner: ArtifactId,
    pub definition: ArtifactId,
    pub name: String,
    pub location: EvidenceLocation,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RuntimeFlowFact {
    pub source: ArtifactId,
    pub source_kind: ArtifactKind,
    pub target: ArtifactId,
    pub target_kind: ArtifactKind,
    pub relation: RuntimeFlowRelationKind,
    pub location: EvidenceLocation,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SupportFact {
    pub artifact: ArtifactId,
    pub artifact_kind: ArtifactKind,
    pub state: SupportStateKind,
    pub location: EvidenceLocation,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EvidenceLocation {
    pub relative_path: PortableRelativePath,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xml_path: Option<String>,
}

pub(crate) trait LocatedFact {
    fn location(&self) -> &EvidenceLocation;
}

impl LocatedFact for MetadataFact {
    fn location(&self) -> &EvidenceLocation {
        &self.location
    }
}

impl LocatedFact for FormFact {
    fn location(&self) -> &EvidenceLocation {
        &self.location
    }
}

impl LocatedFact for BslFact {
    fn location(&self) -> &EvidenceLocation {
        &self.location
    }
}

impl LocatedFact for DefinitionFact {
    fn location(&self) -> &EvidenceLocation {
        &self.location
    }
}

impl LocatedFact for RuntimeFlowFact {
    fn location(&self) -> &EvidenceLocation {
        &self.location
    }
}

impl LocatedFact for SupportFact {
    fn location(&self) -> &EvidenceLocation {
        &self.location
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderDiagnostic {
    pub code: String,
    pub message: String,
    pub materiality: MissingCheckMateriality,
}

impl ProviderDiagnostic {
    pub(crate) fn material(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            materiality: MissingCheckMateriality::Material,
        }
    }

    pub(crate) fn non_material(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            materiality: MissingCheckMateriality::NonMaterial,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderOutcome<T> {
    Complete(T),
    Bounded {
        data: T,
        diagnostic: ProviderDiagnostic,
    },
    Unavailable(ProviderDiagnostic),
    Failed(ProviderDiagnostic),
    ContractViolation(ProviderDiagnostic),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DiscoveryQueryLimits {
    pub max_files: u32,
    pub max_bytes: u64,
    pub max_evidence: u16,
    pub max_candidates: u16,
    pub max_graph_depth: u8,
}

#[derive(Debug, Clone)]
pub(crate) struct DiscoveryQuery<'a> {
    task: &'a str,
    concepts: &'a [DiscoveryConcept],
    search_terms: &'a [String],
    objects: &'a [ArtifactId],
    limits: DiscoveryQueryLimits,
    cancellation: Option<CancellationToken>,
}

impl<'a> DiscoveryQuery<'a> {
    pub(crate) fn new(
        task: &'a str,
        concepts: &'a [DiscoveryConcept],
        search_terms: &'a [String],
        objects: &'a [ArtifactId],
        limits: DiscoveryQueryLimits,
    ) -> Self {
        Self {
            task,
            concepts,
            search_terms,
            objects,
            limits,
            cancellation: None,
        }
    }

    pub(crate) fn with_cancellation(mut self, cancellation: &CancellationToken) -> Self {
        self.cancellation = Some(cancellation.clone());
        self
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancellation
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    }

    pub(crate) fn task(&self) -> &'a str {
        self.task
    }

    pub(crate) fn concepts(&self) -> &'a [DiscoveryConcept] {
        self.concepts
    }

    pub(crate) fn search_terms(&self) -> &'a [String] {
        self.search_terms
    }

    pub(crate) fn objects(&self) -> &'a [ArtifactId] {
        self.objects
    }

    pub(crate) fn limits(&self) -> DiscoveryQueryLimits {
        self.limits
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscoveryEnvironment {
    source_root: PathBuf,
    mapping_fingerprint: MappingFingerprint,
}

impl DiscoveryEnvironment {
    pub(crate) fn new(source_root: PathBuf, mapping_fingerprint: MappingFingerprint) -> Self {
        Self {
            source_root,
            mapping_fingerprint,
        }
    }

    pub(crate) fn source_root(&self) -> &Path {
        &self.source_root
    }

    pub(crate) fn mapping_fingerprint(&self) -> &MappingFingerprint {
        &self.mapping_fingerprint
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiscoveryError {
    EmptySourceRoot,
    Cancelled,
    ProjectSources(String),
    NoConfigurationSource,
    AmbiguousConfigurationSources(Vec<String>),
    InvalidSourceRoot(String),
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySourceRoot => formatter.write_str("discovery source root must not be empty"),
            Self::Cancelled => formatter.write_str("discovery cancelled"),
            Self::ProjectSources(message) => {
                write!(
                    formatter,
                    "could not discover project source sets: {message}"
                )
            }
            Self::NoConfigurationSource => formatter
                .write_str("sourceDir is required because no configuration source set was found"),
            Self::AmbiguousConfigurationSources(candidates) => write!(
                formatter,
                "sourceDir is required because configuration source sets are ambiguous: {}",
                candidates.join(", ")
            ),
            Self::InvalidSourceRoot(message) => {
                write!(formatter, "invalid discovery source root: {message}")
            }
        }
    }
}

impl std::error::Error for DiscoveryError {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiscoveryConcept {
    pub value: String,
    pub provenance: ConceptProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiscoverySource {
    pub root: PathBuf,
    pub mapping_fingerprint: MappingFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalysisSnapshot {
    pub mapping_fingerprint: MappingFingerprint,
    pub fingerprint: SnapshotFingerprint,
    pub contributors: Vec<AnalyzedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderReport {
    pub provider: ProviderKind,
    pub outcome: ProviderOutcomeKind,
    pub coverage: ProviderCoverage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<ProviderDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RelatedArtifact {
    pub artifact: ArtifactId,
    pub kind: ArtifactKind,
    pub evidence_ids: Vec<EvidenceId>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StructuralEdge {
    pub source: ArtifactId,
    pub target: ArtifactId,
    pub relation: StructuralRelationKind,
    pub evidence_ids: Vec<EvidenceId>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeFlowEdge {
    pub source: ArtifactId,
    pub target: ArtifactId,
    pub relation: RuntimeFlowRelationKind,
    pub evidence_ids: Vec<EvidenceId>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExtensionPointCandidate {
    pub target: ArtifactId,
    pub kind: ArtifactKind,
    pub evidence_ids: Vec<EvidenceId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub support_state: Option<SupportStateKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiscoveryWarning {
    pub code: String,
    pub message: String,
    pub blocking: bool,
    pub evidence_ids: Vec<EvidenceId>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MissingCheck {
    pub provider: ProviderKind,
    pub materiality: MissingCheckMateriality,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Evidence {
    pub id: EvidenceId,
    pub provider: ProviderKind,
    pub kind: EvidenceKind,
    pub target: ArtifactId,
    pub relation: EvidenceRelation,
    pub location: EvidenceLocation,
    pub raw_content_hash: ContentHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiscoveryReport {
    pub schema_version: u32,
    pub status: DiscoveryStatus,
    pub source: DiscoverySource,
    pub analysis_snapshot: AnalysisSnapshot,
    pub concepts: Vec<DiscoveryConcept>,
    pub provider_outcomes: Vec<ProviderReport>,
    pub related_artifacts: Vec<RelatedArtifact>,
    pub structural_edges: Vec<StructuralEdge>,
    pub runtime_flow_edges: Vec<RuntimeFlowEdge>,
    pub candidates: Vec<ExtensionPointCandidate>,
    pub warnings: Vec<DiscoveryWarning>,
    pub missing_checks: Vec<MissingCheck>,
    pub evidence: Vec<Evidence>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_identity_uses_trim_plus_rust_unicode_lowercase_mapping() {
        let mixed = ArtifactId::parse(" Document.Order ").expect("mixed-case artifact");
        let lower = ArtifactId::parse("document.order").expect("lowercase artifact");

        assert_eq!(mixed.as_str(), "document.order");
        assert_eq!(mixed, lower);
    }

    #[test]
    fn discovery_identity_is_trim_plus_rust_unicode_lowercase_mapping() {
        assert_eq!(normalize_discovery_identity("  SERIES  "), "series");
        assert_eq!(normalize_discovery_identity("  СЕРИИ  "), "серии");
        assert_eq!(normalize_discovery_identity("Straße"), "straße");
        assert_ne!(
            normalize_discovery_identity("Straße"),
            normalize_discovery_identity("STRASSE")
        );
    }

    #[test]
    fn artifact_rejects_unicode_lowercase_expansion_beyond_byte_limit() {
        let raw = format!("K.{}", "\u{0130}".repeat(511));
        assert_eq!(raw.len(), MAX_ARTIFACT_ID_BYTES);

        let error = ArtifactId::parse(&raw).unwrap_err();

        assert_eq!(error, ArtifactIdError::NormalizedBytesOutOfRange);
    }

    #[test]
    fn artifact_order_uses_the_normalized_identity() {
        let mut artifacts = [
            ArtifactId::parse("Document.Zed").expect("zed artifact"),
            ArtifactId::parse("document.Alpha").expect("alpha artifact"),
        ];

        artifacts.sort();

        assert_eq!(artifacts[0].as_str(), "document.alpha");
        assert_eq!(artifacts[1].as_str(), "document.zed");
    }

    #[test]
    fn typed_discovery_enums_have_stable_snake_case_serialization() {
        let cases = [
            serde_json::to_string(&ArtifactKind::MetadataObject).expect("artifact kind"),
            serde_json::to_string(&ArtifactKind::TabularSection).expect("tabular section"),
            serde_json::to_string(&ArtifactKind::Attribute).expect("attribute"),
            serde_json::to_string(&ArtifactKind::Form).expect("form"),
            serde_json::to_string(&ArtifactKind::FormControl).expect("form control"),
            serde_json::to_string(&ArtifactKind::Command).expect("command"),
            serde_json::to_string(&ArtifactKind::Module).expect("module"),
            serde_json::to_string(&ArtifactKind::Method).expect("method"),
            serde_json::to_string(&ConceptProvenance::TaskDerived).expect("task provenance"),
            serde_json::to_string(&ConceptProvenance::Explicit).expect("explicit provenance"),
            serde_json::to_string(&RuntimeFlowRelationKind::Callback).expect("callback"),
            serde_json::to_string(&RuntimeFlowRelationKind::Action).expect("action"),
            serde_json::to_string(&RuntimeFlowRelationKind::EventSubscription)
                .expect("event subscription"),
            serde_json::to_string(&RuntimeFlowRelationKind::Calls).expect("calls"),
            serde_json::to_string(&SupportStateKind::NotOnSupport).expect("off support"),
            serde_json::to_string(&SupportStateKind::Locked).expect("locked"),
            serde_json::to_string(&SupportStateKind::Editable).expect("editable"),
            serde_json::to_string(&SupportStateKind::Removed).expect("removed"),
        ];

        assert!(cases.contains(&"\"task_derived\"".to_string()));
        assert!(cases.contains(&"\"event_subscription\"".to_string()));
        assert!(cases.contains(&"\"not_on_support\"".to_string()));
    }

    #[test]
    fn provider_facing_types_preserve_typed_data_and_borrowed_query_values() {
        let target = ArtifactId::parse("Document.Order.Attribute.Series").expect("target artifact");
        let form = ArtifactId::parse("Document.Order.Form.Main").expect("form artifact");
        let bindings = [
            FormBinding::Data {
                target: target.clone(),
                target_kind: ArtifactKind::Attribute,
                data_path: "Object.Series".to_string(),
            },
            FormBinding::Command {
                command: "Check".to_string(),
                handler: "CheckSeries".to_string(),
                target: target.clone(),
                target_kind: ArtifactKind::Method,
            },
            FormBinding::Event {
                event: "OnChange".to_string(),
                handler: "SeriesOnChange".to_string(),
                target: target.clone(),
                target_kind: ArtifactKind::Method,
            },
        ];
        let binding_targets = bindings
            .iter()
            .map(|binding| match binding {
                FormBinding::Data { target, .. }
                | FormBinding::Command { target, .. }
                | FormBinding::Event { target, .. } => target,
            })
            .collect::<Vec<_>>();
        assert!(binding_targets.iter().all(|item| *item == &target));
        assert_ne!(form, target);

        let source = SourceFile {
            relative_path: PortableRelativePath::parse_str("Documents/Order.xml")
                .expect("source path"),
            bytes: b"raw".to_vec(),
            raw_hash: ContentHash::sha256(b"raw"),
        };
        assert_eq!(source.analyzed_file().bytes, 3);

        let concepts = [DiscoveryConcept {
            value: "series".to_string(),
            provenance: ConceptProvenance::Explicit,
        }];
        let search_terms = ["CheckSeries".to_string()];
        let objects = [target];
        let limits = DiscoveryQueryLimits {
            max_files: 10,
            max_bytes: 100,
            max_evidence: 5,
            max_candidates: 2,
            max_graph_depth: 1,
        };
        let query =
            DiscoveryQuery::new("original task", &concepts, &search_terms, &objects, limits);
        assert_eq!(query.task(), "original task");
        assert_eq!(query.limits(), limits);
        assert_eq!(query.concepts(), &concepts);

        let diagnostic = ProviderDiagnostic::non_material("optional", "optional check");
        assert_eq!(diagnostic.materiality, MissingCheckMateriality::NonMaterial);
    }

    #[test]
    fn empty_fact_batch_has_distinct_analyzed_and_evidence_contributor_sets() {
        let batch = FactBatch::<MetadataFact>::empty();

        assert!(batch.analyzed_files.is_empty());
        assert!(batch.contributors.is_empty());
    }

    #[test]
    fn digest_newtypes_are_stable_and_domain_separated() {
        let mapping = MappingFingerprint::from_identity("configuration:src");
        let contributor = AnalyzedFile {
            relative_path: PortableRelativePath::parse_str("Document.xml")
                .expect("contributor path"),
            raw_hash: ContentHash::sha256(b"raw"),
            bytes: 3,
        };
        let snapshot = SnapshotFingerprint::from_manifest(&mapping, &[contributor.clone()]);
        let target = ArtifactId::parse("Document.Order").expect("artifact");
        let location = EvidenceLocation {
            relative_path: contributor.relative_path,
            line: Some(1),
            column: None,
            xml_path: None,
        };
        let evidence = EvidenceId::from_fact(
            ProviderKind::MetadataCatalog,
            EvidenceKind::Metadata,
            &target,
            &EvidenceRelation::Structural(StructuralRelationKind::Contains),
            &location,
            &contributor.raw_hash,
        );

        assert_eq!(
            mapping.as_str(),
            "43627a9241ec1048c7ce709422c5082fe7529eda9664cac2fccabcfeedb5e1d1"
        );
        assert_eq!(
            snapshot.as_str(),
            "6fd21fab849e53de538989afc3a3ac6bb9459142908992bc07219211a860ba13"
        );
        assert_eq!(
            evidence.as_str(),
            "fe36f47ed429c5b84ced6892e52af0a8b5ba96113507f7507613c2fe965d7939"
        );
    }

    #[test]
    fn portable_relative_path_canonicalizes_cross_platform_separators() {
        let slash = PortableRelativePath::parse_str("Documents/Order/Ext/ObjectModule.bsl")
            .expect("slash path");
        let backslash = PortableRelativePath::parse_str("Documents\\Order\\Ext\\ObjectModule.bsl")
            .expect("backslash path");

        assert_eq!(slash, backslash);
        assert_eq!(slash.as_str(), "Documents/Order/Ext/ObjectModule.bsl");
        assert_eq!(
            serde_json::to_string(&slash).expect("serialize path"),
            "\"Documents/Order/Ext/ObjectModule.bsl\""
        );
    }

    #[test]
    fn canonical_path_bytes_make_cross_separator_hashes_identical() {
        let target = ArtifactId::parse("Document.Order").expect("artifact");
        let raw_hash = ContentHash::sha256(b"raw");
        let slash_path =
            PortableRelativePath::parse_str("Documents/Order.xml").expect("slash path");
        let backslash_path =
            PortableRelativePath::parse_str("Documents\\Order.xml").expect("backslash path");
        let slash_location = EvidenceLocation {
            relative_path: slash_path.clone(),
            line: Some(1),
            column: None,
            xml_path: None,
        };
        let backslash_location = EvidenceLocation {
            relative_path: backslash_path.clone(),
            line: Some(1),
            column: None,
            xml_path: None,
        };
        let slash_evidence = EvidenceId::from_fact(
            ProviderKind::MetadataCatalog,
            EvidenceKind::Metadata,
            &target,
            &EvidenceRelation::Structural(StructuralRelationKind::Contains),
            &slash_location,
            &raw_hash,
        );
        let backslash_evidence = EvidenceId::from_fact(
            ProviderKind::MetadataCatalog,
            EvidenceKind::Metadata,
            &target,
            &EvidenceRelation::Structural(StructuralRelationKind::Contains),
            &backslash_location,
            &raw_hash,
        );
        let mapping = MappingFingerprint::from_identity("configuration:src");
        let slash_snapshot = SnapshotFingerprint::from_manifest(
            &mapping,
            &[AnalyzedFile {
                relative_path: slash_path,
                raw_hash: raw_hash.clone(),
                bytes: 3,
            }],
        );
        let backslash_snapshot = SnapshotFingerprint::from_manifest(
            &mapping,
            &[AnalyzedFile {
                relative_path: backslash_path,
                raw_hash,
                bytes: 3,
            }],
        );

        assert_eq!(slash_evidence, backslash_evidence);
        assert_eq!(slash_snapshot, backslash_snapshot);
    }

    #[test]
    fn portable_relative_path_rejects_ambiguous_or_unsafe_spellings() {
        for unsafe_path in [
            "",
            "/absolute",
            "\\absolute",
            "C:\\absolute",
            "dir//file.xml",
            "dir\\\\file.xml",
            "dir/./file.xml",
            "dir/../file.xml",
            "dir/file.xml/",
            "dir./file.xml",
            "dir /file.xml",
            "dir/NUL.xml",
            "dir/file?.xml",
        ] {
            assert!(
                PortableRelativePath::parse_str(unsafe_path).is_err(),
                "unsafe path was accepted: {unsafe_path:?}"
            );
        }
    }
}
