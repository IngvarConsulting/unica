use crate::domain::discovery_registry::{metadata_kind, module_kind};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

pub(crate) trait StableTag {
    fn stable_tag(self) -> u16;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DiscoverMode {
    Explore,
    Validate,
}

impl StableTag for DiscoverMode {
    fn stable_tag(self) -> u16 {
        match self {
            Self::Explore => 1,
            Self::Validate => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArtifactKind {
    MetadataObject,
    MetadataAttribute,
    TabularSection,
    TabularSectionAttribute,
    Module,
    Method,
    Form,
    FormCommand,
    CommonCommand,
    EventSubscription,
    ScheduledJob,
    HttpRoute,
    ExchangePlan,
    Report,
    DataProcessor,
}

impl ArtifactKind {
    pub(crate) const ALL: [Self; 15] = [
        Self::MetadataObject,
        Self::MetadataAttribute,
        Self::TabularSection,
        Self::TabularSectionAttribute,
        Self::Module,
        Self::Method,
        Self::Form,
        Self::FormCommand,
        Self::CommonCommand,
        Self::EventSubscription,
        Self::ScheduledJob,
        Self::HttpRoute,
        Self::ExchangePlan,
        Self::Report,
        Self::DataProcessor,
    ];
}

impl StableTag for ArtifactKind {
    fn stable_tag(self) -> u16 {
        match self {
            Self::MetadataObject => 1,
            Self::MetadataAttribute => 2,
            Self::TabularSection => 3,
            Self::TabularSectionAttribute => 4,
            Self::Module => 5,
            Self::Method => 6,
            Self::Form => 7,
            Self::FormCommand => 8,
            Self::CommonCommand => 9,
            Self::EventSubscription => 10,
            Self::ScheduledJob => 11,
            Self::HttpRoute => 12,
            Self::ExchangePlan => 13,
            Self::Report => 14,
            Self::DataProcessor => 15,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ArtifactRef {
    pub(crate) kind: ArtifactKind,
    #[serde(rename = "ref")]
    pub(crate) canonical_ref: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ArtifactRefWire {
    kind: ArtifactKind,
    #[serde(rename = "ref")]
    canonical_ref: String,
}

impl<'de> Deserialize<'de> for ArtifactRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ArtifactRefWire::deserialize(deserializer)?;
        Self::parse(wire.kind, &wire.canonical_ref).map_err(D::Error::custom)
    }
}

impl ArtifactRef {
    pub(crate) fn parse(kind: ArtifactKind, canonical_ref: &str) -> Result<Self, String> {
        validate_canonical_ref(kind, canonical_ref)?;
        Ok(Self {
            kind,
            canonical_ref: canonical_ref.to_string(),
        })
    }

    pub(crate) fn identity_key(&self) -> (u16, String) {
        (
            self.kind.stable_tag(),
            unicode_lowercase(&self.canonical_ref),
        )
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_canonical_ref(self.kind, &self.canonical_ref)
    }
}

impl PartialEq for ArtifactRef {
    fn eq(&self, other: &Self) -> bool {
        self.identity_key() == other.identity_key()
    }
}

impl Eq for ArtifactRef {}

impl PartialOrd for ArtifactRef {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ArtifactRef {
    fn cmp(&self, other: &Self) -> Ordering {
        self.identity_key().cmp(&other.identity_key())
    }
}

impl Hash for ArtifactRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity_key().hash(state);
    }
}

fn unicode_lowercase(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).collect()
}

const SPECIALIZED_ROOTS: &[&str] = &[
    "CommonModule",
    "CommonCommand",
    "EventSubscription",
    "ScheduledJob",
    "ExchangePlan",
    "Report",
    "DataProcessor",
];

fn is_registered_metadata_root(root: &str) -> bool {
    metadata_kind(root).is_some()
}

fn is_metadata_object_root(root: &str) -> bool {
    is_registered_metadata_root(root) && !SPECIALIZED_ROOTS.contains(&root)
}

fn is_owner_ref(segments: &[&str]) -> bool {
    segments.len() == 2 && is_registered_metadata_root(segments[0]) && segments[0] != "CommonModule"
}

fn is_form_ref(segments: &[&str]) -> bool {
    segments.len() == 4 && is_owner_ref(&segments[..2]) && segments[2] == "Form"
}

fn is_module_ref(segments: &[&str]) -> bool {
    (segments.len() == 2 && segments[0] == "CommonModule")
        || (segments.len() == 3
            && is_owner_ref(&segments[..2])
            && module_kind(segments[2]).is_some())
        || (segments.len() == 5 && is_form_ref(&segments[..4]) && segments[4] == "FormModule")
}

fn validate_canonical_ref(kind: ArtifactKind, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 1024 {
        return Err("canonical artifact ref must contain 1..=1024 UTF-8 bytes".to_string());
    }
    if value.contains('/') || value.contains('\\') || value.chars().any(char::is_control) {
        return Err(
            "canonical artifact ref contains a path separator or control character".to_string(),
        );
    }
    let segments: Vec<_> = value.split('.').collect();
    if segments.iter().any(|segment| {
        segment.is_empty()
            || segment.chars().count() > 128
            || !segment
                .chars()
                .all(|character| character.is_alphanumeric() || character == '_')
    }) {
        return Err("canonical artifact ref contains an invalid identifier segment".to_string());
    }

    let valid = match kind {
        ArtifactKind::MetadataObject => segments.len() == 2 && is_metadata_object_root(segments[0]),
        ArtifactKind::MetadataAttribute => {
            segments.len() == 4 && is_owner_ref(&segments[..2]) && segments[2] == "Attribute"
        }
        ArtifactKind::TabularSection => {
            segments.len() == 4 && is_owner_ref(&segments[..2]) && segments[2] == "TabularSection"
        }
        ArtifactKind::TabularSectionAttribute => {
            segments.len() == 6
                && is_owner_ref(&segments[..2])
                && segments[2] == "TabularSection"
                && segments[4] == "Attribute"
        }
        ArtifactKind::Module => is_module_ref(&segments),
        ArtifactKind::Method => {
            segments.len() >= 3 && is_module_ref(&segments[..segments.len() - 1])
        }
        ArtifactKind::Form => is_form_ref(&segments),
        ArtifactKind::FormCommand => {
            segments.len() == 6 && is_form_ref(&segments[..4]) && segments[4] == "Command"
        }
        ArtifactKind::CommonCommand => segments.len() == 2 && segments[0] == "CommonCommand",
        ArtifactKind::EventSubscription => {
            segments.len() == 2 && segments[0] == "EventSubscription"
        }
        ArtifactKind::ScheduledJob => segments.len() == 2 && segments[0] == "ScheduledJob",
        ArtifactKind::HttpRoute => {
            segments.len() == 6
                && segments[0] == "HTTPService"
                && segments[2] == "URLTemplate"
                && segments[4] == "Method"
        }
        ArtifactKind::ExchangePlan => segments.len() == 2 && segments[0] == "ExchangePlan",
        ArtifactKind::Report => segments.len() == 2 && segments[0] == "Report",
        ArtifactKind::DataProcessor => segments.len() == 2 && segments[0] == "DataProcessor",
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "artifact kind {kind:?} does not match canonical ref shape"
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum InterceptorType {
    Before,
    After,
    ModificationAndControl,
}

impl StableTag for InterceptorType {
    fn stable_tag(self) -> u16 {
        match self {
            Self::Before => 1,
            Self::After => 2,
            Self::ModificationAndControl => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ExecutionContext {
    #[default]
    #[serde(rename = "НаСервере")]
    Server,
    #[serde(rename = "НаКлиенте")]
    Client,
    #[serde(rename = "НаСервереБезКонтекста")]
    ServerWithoutContext,
    #[serde(rename = "НаКлиентеНаСервереБезКонтекста")]
    ClientAndServerWithoutContext,
}

impl StableTag for ExecutionContext {
    fn stable_tag(self) -> u16 {
        match self {
            Self::Server => 1,
            Self::Client => 2,
            Self::ServerWithoutContext => 3,
            Self::ClientAndServerWithoutContext => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CfePatchMethodArguments {
    #[serde(rename = "ExtensionPath")]
    pub(crate) extension_path: String,
    #[serde(rename = "ModulePath")]
    pub(crate) module_path: String,
    #[serde(rename = "MethodName")]
    pub(crate) method_name: String,
    #[serde(rename = "InterceptorType")]
    pub(crate) interceptor_type: InterceptorType,
    #[serde(rename = "Context")]
    pub(crate) context: ExecutionContext,
    #[serde(rename = "IsFunction")]
    pub(crate) is_function: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CfePatchMethodArgumentsWire {
    #[serde(rename = "ExtensionPath")]
    extension_path: String,
    #[serde(rename = "ModulePath")]
    module_path: String,
    #[serde(rename = "MethodName")]
    method_name: String,
    #[serde(rename = "InterceptorType")]
    interceptor_type: InterceptorType,
    #[serde(rename = "Context", default)]
    context: ExecutionContext,
    #[serde(rename = "IsFunction", default)]
    is_function: bool,
}

impl<'de> Deserialize<'de> for CfePatchMethodArguments {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = CfePatchMethodArgumentsWire::deserialize(deserializer)?;
        let extension_path =
            bounded_string(wire.extension_path, "ExtensionPath", 1024).map_err(D::Error::custom)?;
        let module_path =
            bounded_string(wire.module_path, "ModulePath", 1024).map_err(D::Error::custom)?;
        let method_name =
            bounded_identifier(wire.method_name, "MethodName").map_err(D::Error::custom)?;
        Ok(Self {
            extension_path,
            module_path,
            method_name,
            interceptor_type: wire.interceptor_type,
            context: wire.context,
            is_function: wire.is_function,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tool", deny_unknown_fields)]
pub(crate) enum MutationIntent {
    #[serde(rename = "unica.cfe.patch_method")]
    CfePatchMethod {
        #[serde(rename = "destinationSourceSet")]
        destination_source_set: String,
        arguments: CfePatchMethodArguments,
    },
}

impl MutationIntent {
    fn validate(self) -> Result<Self, String> {
        match self {
            Self::CfePatchMethod {
                destination_source_set,
                arguments,
            } => Ok(Self::CfePatchMethod {
                destination_source_set: bounded_string(
                    destination_source_set,
                    "destinationSourceSet",
                    1024,
                )?,
                arguments,
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Proposal {
    pub(crate) id: String,
    pub(crate) target: ArtifactRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) mutation_intent: Option<MutationIntent>,
    pub(crate) intent: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ProposalWire {
    id: String,
    target: ArtifactRef,
    #[serde(default)]
    mutation_intent: FieldPresence<MutationIntent>,
    intent: String,
}

impl<'de> Deserialize<'de> for Proposal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ProposalWire::deserialize(deserializer)?;
        let id = validate_proposal_id(wire.id).map_err(D::Error::custom)?;
        let intent =
            bounded_string(wire.intent, "proposal.intent", 2048).map_err(D::Error::custom)?;
        let mutation_intent = match wire.mutation_intent {
            FieldPresence::Missing => None,
            FieldPresence::Present(Some(intent)) => {
                Some(intent.validate().map_err(D::Error::custom)?)
            }
            FieldPresence::Present(None) => {
                return Err(D::Error::custom("mutationIntent must not be null"));
            }
        };
        Ok(Self {
            id,
            target: wire.target,
            mutation_intent,
            intent,
        })
    }
}

pub(crate) fn validate_proposal_id_value(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 64 || !id.is_ascii() {
        return Err("proposal id must contain 1..=64 ASCII characters".to_string());
    }
    let mut characters = id.chars();
    if !characters
        .next()
        .is_some_and(|value| value.is_ascii_alphanumeric())
        || !characters
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '.' | '_' | '-'))
    {
        return Err("proposal id has an invalid shape".to_string());
    }
    Ok(())
}

fn validate_proposal_id(id: String) -> Result<String, String> {
    validate_proposal_id_value(&id)?;
    Ok(id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct DiscoverLimits {
    pub(crate) max_candidates: u16,
    pub(crate) max_graph_depth: u8,
    pub(crate) max_evidence: u16,
}

impl Default for DiscoverLimits {
    fn default() -> Self {
        Self {
            max_candidates: 20,
            max_graph_depth: 4,
            max_evidence: 200,
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DiscoverLimitsWire {
    #[serde(default)]
    max_candidates: FieldPresence<u16>,
    #[serde(default)]
    max_graph_depth: FieldPresence<u8>,
    #[serde(default)]
    max_evidence: FieldPresence<u16>,
}

impl<'de> Deserialize<'de> for DiscoverLimits {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = DiscoverLimitsWire::deserialize(deserializer)?;
        let defaults = Self::default();
        let limits = Self {
            max_candidates: non_null_default(
                wire.max_candidates,
                defaults.max_candidates,
                "maxCandidates",
            )?,
            max_graph_depth: non_null_default(
                wire.max_graph_depth,
                defaults.max_graph_depth,
                "maxGraphDepth",
            )?,
            max_evidence: non_null_default(
                wire.max_evidence,
                defaults.max_evidence,
                "maxEvidence",
            )?,
        };
        if !(1..=100).contains(&limits.max_candidates) {
            return Err(D::Error::custom("maxCandidates must be in 1..=100"));
        }
        if !(1..=12).contains(&limits.max_graph_depth) {
            return Err(D::Error::custom("maxGraphDepth must be in 1..=12"));
        }
        if !(1..=2000).contains(&limits.max_evidence) {
            return Err(D::Error::custom("maxEvidence must be in 1..=2000"));
        }
        Ok(limits)
    }
}

fn non_null_default<E, T>(presence: FieldPresence<T>, default: T, field: &str) -> Result<T, E>
where
    E: serde::de::Error,
{
    match presence {
        FieldPresence::Missing => Ok(default),
        FieldPresence::Present(Some(value)) => Ok(value),
        FieldPresence::Present(None) => Err(E::custom(format!("{field} must not be null"))),
    }
}

#[derive(Default)]
enum FieldPresence<T> {
    #[default]
    Missing,
    Present(Option<T>),
}

impl<'de, T> Deserialize<'de> for FieldPresence<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<T>::deserialize(deserializer).map(Self::Present)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DiscoverRequestWire {
    mode: DiscoverMode,
    task: String,
    concepts: Vec<String>,
    #[serde(default)]
    search_terms: Vec<String>,
    #[serde(default)]
    known_artifacts: Vec<ArtifactRef>,
    #[serde(default)]
    proposals: FieldPresence<Vec<Proposal>>,
    #[serde(default)]
    source_set: FieldPresence<String>,
    #[serde(default)]
    limits: DiscoverLimits,
    #[serde(default)]
    cwd: FieldPresence<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct DiscoverRequest {
    pub(crate) mode: DiscoverMode,
    pub(crate) task: String,
    pub(crate) concepts: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) search_terms: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) known_artifacts: Vec<ArtifactRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) proposals: Vec<Proposal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_set: Option<String>,
    pub(crate) limits: DiscoverLimits,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cwd: Option<String>,
}

impl<'de> Deserialize<'de> for DiscoverRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = DiscoverRequestWire::deserialize(deserializer)?;
        let task = bounded_string(wire.task, "task", 8192).map_err(D::Error::custom)?;
        let concepts = validate_string_list(wire.concepts, "concepts", 1, 64, 256)
            .map_err(D::Error::custom)?;
        let search_terms = validate_string_list(wire.search_terms, "searchTerms", 0, 128, 256)
            .map_err(D::Error::custom)?;
        let known_artifacts =
            validate_unique_artifacts(wire.known_artifacts, 128).map_err(D::Error::custom)?;
        let proposals = match (wire.mode, wire.proposals) {
            (DiscoverMode::Explore, FieldPresence::Missing) => Vec::new(),
            (DiscoverMode::Explore, FieldPresence::Present(_)) => {
                return Err(D::Error::custom("proposals is forbidden in explore mode"));
            }
            (DiscoverMode::Validate, FieldPresence::Missing | FieldPresence::Present(None)) => {
                return Err(D::Error::custom(
                    "validate mode requires a non-null proposals array",
                ));
            }
            (DiscoverMode::Validate, FieldPresence::Present(Some(proposals))) => {
                validate_proposals(proposals).map_err(D::Error::custom)?
            }
        };
        let source_set = non_null_optional(wire.source_set, "sourceSet")?
            .map(|value| bounded_string(value, "sourceSet", 1024))
            .transpose()
            .map_err(D::Error::custom)?;
        // `cwd` is the existing common transport argument. Its path policy is
        // owned by workspace resolution; discovery adds no private length or
        // normalization rule beyond rejecting JSON null.
        let cwd = non_null_optional(wire.cwd, "cwd")?;
        Ok(Self {
            mode: wire.mode,
            task,
            concepts,
            search_terms,
            known_artifacts,
            proposals,
            source_set,
            limits: wire.limits,
            cwd,
        })
    }
}

fn non_null_optional<E, T>(presence: FieldPresence<T>, field: &str) -> Result<Option<T>, E>
where
    E: serde::de::Error,
{
    match presence {
        FieldPresence::Missing => Ok(None),
        FieldPresence::Present(Some(value)) => Ok(Some(value)),
        FieldPresence::Present(None) => Err(E::custom(format!("{field} must not be null"))),
    }
}

fn bounded_string(value: String, field: &str, max_bytes: usize) -> Result<String, String> {
    if value.trim().is_empty() || value.len() > max_bytes {
        return Err(format!("{field} must contain 1..={max_bytes} UTF-8 bytes"));
    }
    Ok(value)
}

fn bounded_identifier(value: String, field: &str) -> Result<String, String> {
    let value = bounded_string(value, field, 512)?;
    if value.chars().count() > 128
        || !value
            .chars()
            .all(|character| character.is_alphanumeric() || character == '_')
    {
        return Err(format!("{field} must be one canonical identifier segment"));
    }
    Ok(value)
}

fn validate_string_list(
    values: Vec<String>,
    field: &str,
    minimum: usize,
    maximum: usize,
    max_bytes: usize,
) -> Result<Vec<String>, String> {
    if !(minimum..=maximum).contains(&values.len()) {
        return Err(format!(
            "{field} must contain {minimum}..={maximum} entries"
        ));
    }
    let mut identities = BTreeSet::new();
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let value = bounded_string(value, field, max_bytes)?;
        if !identities.insert(value.clone()) {
            return Err(format!("{field} contains duplicate entries"));
        }
        normalized.push(value);
    }
    Ok(normalized)
}

fn validate_unique_artifacts(
    artifacts: Vec<ArtifactRef>,
    maximum: usize,
) -> Result<Vec<ArtifactRef>, String> {
    if artifacts.len() > maximum {
        return Err(format!(
            "knownArtifacts must contain at most {maximum} entries"
        ));
    }
    let mut identities = BTreeSet::new();
    for artifact in &artifacts {
        if !identities.insert(artifact.clone()) {
            return Err("knownArtifacts contains duplicate entries".to_string());
        }
    }
    Ok(artifacts)
}

fn validate_proposals(proposals: Vec<Proposal>) -> Result<Vec<Proposal>, String> {
    if !(1..=32).contains(&proposals.len()) {
        return Err("proposals must contain 1..=32 entries".to_string());
    }
    let mut ids = BTreeSet::new();
    for proposal in &proposals {
        if !ids.insert(proposal.id.as_str()) {
            return Err("proposal ids must be unique".to_string());
        }
    }
    Ok(proposals)
}
