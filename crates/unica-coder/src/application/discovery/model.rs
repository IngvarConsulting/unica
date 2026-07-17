use super::contract::{validate_proposal_id_value, ArtifactRef, StableTag};
use crate::domain::discovery_registry::{metadata_kind, module_kind};
use crate::domain::project_sources::SourceFormat;
use serde::de::Error as _;
use serde::ser::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DiscoveryStatus {
    Complete,
    Partial,
    Insufficient,
}

impl StableTag for DiscoveryStatus {
    fn stable_tag(self) -> u16 {
        match self {
            Self::Complete => 1,
            Self::Partial => 2,
            Self::Insufficient => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceLevel {
    Lexical,
    Observed,
    Connected,
    Actionable,
}

impl StableTag for EvidenceLevel {
    fn stable_tag(self) -> u16 {
        match self {
            Self::Lexical => 1,
            Self::Observed => 2,
            Self::Connected => 3,
            Self::Actionable => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Coverage {
    Complete,
    Bounded,
    Unknown,
}

impl StableTag for Coverage {
    fn stable_tag(self) -> u16 {
        match self {
            Self::Complete => 1,
            Self::Bounded => 2,
            Self::Unknown => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CheckState {
    Passed,
    Unavailable,
    Failed,
    Skipped,
}

impl StableTag for CheckState {
    fn stable_tag(self) -> u16 {
        match self {
            Self::Passed => 1,
            Self::Unavailable => 2,
            Self::Failed => 3,
            Self::Skipped => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CheckOutcome {
    Satisfied,
    NoMatch,
    Inconclusive,
    Conflict,
    NotApplicable,
}

impl StableTag for CheckOutcome {
    fn stable_tag(self) -> u16 {
        match self {
            Self::Satisfied => 1,
            Self::NoMatch => 2,
            Self::Inconclusive => 3,
            Self::Conflict => 4,
            Self::NotApplicable => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CheckSeverity {
    Info,
    Warning,
    Blocking,
}

impl StableTag for CheckSeverity {
    fn stable_tag(self) -> u16 {
        match self {
            Self::Info => 1,
            Self::Warning => 2,
            Self::Blocking => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SupportState {
    Editable,
    Locked,
    ConfigurationReadOnly,
    Removed,
    NotUnderSupport,
    ExtensionOwned,
    ExtensionRequired,
    Unknown,
}

impl StableTag for SupportState {
    fn stable_tag(self) -> u16 {
        match self {
            Self::Editable => 1,
            Self::Locked => 2,
            Self::ConfigurationReadOnly => 3,
            Self::Removed => 4,
            Self::NotUnderSupport => 5,
            Self::ExtensionOwned => 6,
            Self::ExtensionRequired => 7,
            Self::Unknown => 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceType {
    Metadata,
    CodeOccurrence,
    Definition,
    Binding,
    Call,
    PlatformCallback,
    Support,
}

impl StableTag for EvidenceType {
    fn stable_tag(self) -> u16 {
        match self {
            Self::Metadata => 1,
            Self::CodeOccurrence => 2,
            Self::Definition => 3,
            Self::Binding => 4,
            Self::Call => 5,
            Self::PlatformCallback => 6,
            Self::Support => 7,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum EvidencePort {
    #[serde(rename = "MetadataCatalogPort")]
    MetadataCatalog,
    #[serde(rename = "CodeSearchPort")]
    CodeSearch,
    #[serde(rename = "DefinitionPort")]
    Definition,
    #[serde(rename = "CallGraphPort")]
    CallGraph,
    #[serde(rename = "FormInspectionPort")]
    FormInspection,
    #[serde(rename = "SupportStatePort")]
    SupportState,
}

impl EvidencePort {
    pub(crate) const ALL: [Self; 6] = [
        Self::MetadataCatalog,
        Self::CodeSearch,
        Self::Definition,
        Self::CallGraph,
        Self::FormInspection,
        Self::SupportState,
    ];

    pub(crate) fn wire_name(self) -> &'static str {
        match self {
            Self::MetadataCatalog => "MetadataCatalogPort",
            Self::CodeSearch => "CodeSearchPort",
            Self::Definition => "DefinitionPort",
            Self::CallGraph => "CallGraphPort",
            Self::FormInspection => "FormInspectionPort",
            Self::SupportState => "SupportStatePort",
        }
    }

    pub(crate) fn parse_wire_name(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|port| port.wire_name() == value)
    }
}

impl StableTag for EvidencePort {
    fn stable_tag(self) -> u16 {
        match self {
            Self::MetadataCatalog => 1,
            Self::CodeSearch => 2,
            Self::Definition => 3,
            Self::CallGraph => 4,
            Self::FormInspection => 5,
            Self::SupportState => 6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderReadiness {
    Ready,
    Unavailable,
    Failed,
}

impl StableTag for ProviderReadiness {
    fn stable_tag(self) -> u16 {
        match self {
            Self::Ready => 1,
            Self::Unavailable => 2,
            Self::Failed => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FlowKind {
    Contains,
    Defines,
    Calls,
    Handles,
    Subscribes,
    Uses,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallResolution {
    Resolved,
    Dynamic,
    Ambiguous,
    Unresolved,
}

impl StableTag for CallResolution {
    fn stable_tag(self) -> u16 {
        match self {
            Self::Resolved => 1,
            Self::Dynamic => 2,
            Self::Ambiguous => 3,
            Self::Unresolved => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallType {
    Direct,
    Method,
    Callback,
    Dynamic,
}

impl StableTag for CallType {
    fn stable_tag(self) -> u16 {
        match self {
            Self::Direct => 1,
            Self::Method => 2,
            Self::Callback => 3,
            Self::Dynamic => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HttpVerb {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
}

impl StableTag for HttpVerb {
    fn stable_tag(self) -> u16 {
        match self {
            Self::Get => 1,
            Self::Post => 2,
            Self::Put => 3,
            Self::Patch => 4,
            Self::Delete => 5,
            Self::Head => 6,
            Self::Options => 7,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BindingDetails {
    Structural,
    EventSubscription {
        event: String,
        context: super::contract::ExecutionContext,
    },
    FormCommand {
        action: String,
        context: super::contract::ExecutionContext,
    },
    CommonCommand {
        action: String,
        context: super::contract::ExecutionContext,
    },
    ScheduledJob {
        enabled: bool,
        context: super::contract::ExecutionContext,
    },
    HttpRoute {
        verb: HttpVerb,
        url_template: String,
        context: super::contract::ExecutionContext,
    },
    ExchangePlan {
        event: String,
        context: super::contract::ExecutionContext,
    },
}

impl BindingDetails {
    pub(crate) const VARIANT_STABLE_TAGS: [u16; 7] = [1, 2, 3, 4, 5, 6, 7];

    pub(crate) fn stable_tag(&self) -> u16 {
        match self {
            Self::Structural => 1,
            Self::EventSubscription { .. } => 2,
            Self::FormCommand { .. } => 3,
            Self::CommonCommand { .. } => 4,
            Self::ScheduledJob { .. } => 5,
            Self::HttpRoute { .. } => 6,
            Self::ExchangePlan { .. } => 7,
        }
    }
}

impl StableTag for FlowKind {
    fn stable_tag(self) -> u16 {
        match self {
            Self::Contains => 1,
            Self::Defines => 2,
            Self::Calls => 3,
            Self::Handles => 4,
            Self::Subscribes => 5,
            Self::Uses => 6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Verdict {
    Supported,
    Contradicted,
    Unknown,
}

impl StableTag for Verdict {
    fn stable_tag(self) -> u16 {
        match self {
            Self::Supported => 1,
            Self::Contradicted => 2,
            Self::Unknown => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FactAnswer {
    Yes,
    No,
    Unknown,
}

impl StableTag for FactAnswer {
    fn stable_tag(self) -> u16 {
        match self {
            Self::Yes => 1,
            Self::No => 2,
            Self::Unknown => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SourceSnapshotRole {
    Analysis,
    Mutation,
}

impl StableTag for SourceSnapshotRole {
    fn stable_tag(self) -> u16 {
        match self {
            Self::Analysis => 1,
            Self::Mutation => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SourceLocation {
    pub(crate) path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) column: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SourceLocationWire {
    path: String,
    line: Option<u32>,
    column: Option<u32>,
}

impl SourceLocation {
    pub(crate) fn new(path: &str, line: Option<u32>, column: Option<u32>) -> Result<Self, String> {
        if path.is_empty()
            || path.len() > 4096
            || path.starts_with('/')
            || path.starts_with('\\')
            || path.contains('\\')
            || path.contains(':')
            || path.chars().any(char::is_control)
            || path
                .split('/')
                .any(|component| component.is_empty() || matches!(component, "." | ".."))
        {
            return Err(
                "location path must be a contained workspace-relative slash path".to_string(),
            );
        }
        if line == Some(0) || column == Some(0) {
            return Err("location line and column are 1-based".to_string());
        }
        if column.is_some() && line.is_none() {
            return Err("location column requires a line".to_string());
        }
        Ok(Self {
            path: path.to_string(),
            line,
            column,
        })
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        Self::new(&self.path, self.line, self.column).map(|_| ())
    }
}

impl<'de> Deserialize<'de> for SourceLocation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SourceLocationWire::deserialize(deserializer)?;
        Self::new(&wire.path, wire.line, wire.column).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct EvidenceProvider {
    pub(crate) port: EvidencePort,
    pub(crate) name: String,
    pub(crate) version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct EvidenceProviderWire {
    port: EvidencePort,
    name: String,
    version: String,
}

impl EvidenceProvider {
    pub(crate) fn new(port: EvidencePort, name: &str, version: &str) -> Result<Self, String> {
        Ok(Self {
            port,
            name: stable_component(name, "provider.name", 128)?,
            version: stable_component(version, "provider.version", 128)?,
        })
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        stable_component(&self.name, "provider.name", 128)?;
        stable_component(&self.version, "provider.version", 128)?;
        Ok(())
    }
}

impl<'de> Deserialize<'de> for EvidenceProvider {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = EvidenceProviderWire::deserialize(deserializer)?;
        Self::new(wire.port, &wire.name, &wire.version).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Freshness {
    pub(crate) source_set: String,
    pub(crate) source_fingerprint: String,
    pub(crate) workspace_epoch: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct FreshnessWire {
    source_set: String,
    source_fingerprint: String,
    workspace_epoch: u64,
}

impl Freshness {
    pub(crate) fn new(
        source_set: &str,
        source_fingerprint: &str,
        workspace_epoch: u64,
    ) -> Result<Self, String> {
        Ok(Self {
            source_set: stable_component(source_set, "freshness.sourceSet", 1024)?,
            source_fingerprint: validate_sha256_fingerprint(source_fingerprint)?,
            workspace_epoch,
        })
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        stable_component(&self.source_set, "freshness.sourceSet", 1024)?;
        validate_sha256_fingerprint(&self.source_fingerprint)?;
        Ok(())
    }
}

impl<'de> Deserialize<'de> for Freshness {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = FreshnessWire::deserialize(deserializer)?;
        Self::new(
            &wire.source_set,
            &wire.source_fingerprint,
            wire.workspace_epoch,
        )
        .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefinitionParameter {
    pub(crate) name: String,
    pub(crate) by_value: bool,
    pub(crate) has_default: bool,
}

impl DefinitionParameter {
    pub(crate) fn new(name: &str, by_value: bool, has_default: bool) -> Result<Self, String> {
        Ok(Self {
            name: stable_identifier(name, "definition.parameter")?,
            by_value,
            has_default,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefinitionShape {
    pub(crate) is_function: bool,
    pub(crate) exported: bool,
    pub(crate) parameters: Vec<DefinitionParameter>,
}

impl DefinitionShape {
    pub(crate) fn new(
        is_function: bool,
        exported: bool,
        parameters: Vec<DefinitionParameter>,
    ) -> Result<Self, String> {
        let mut names = BTreeSet::new();
        for parameter in &parameters {
            if !names.insert(
                parameter
                    .name
                    .chars()
                    .flat_map(char::to_lowercase)
                    .collect::<String>(),
            ) {
                return Err("definition parameters must have unique names".to_string());
            }
        }
        Ok(Self {
            is_function,
            exported,
            parameters,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlatformCallbackShape {
    pub(crate) platform_variant: String,
    pub(crate) metadata_kind: String,
    pub(crate) module_kind: String,
    pub(crate) method_name: String,
    pub(crate) exported: bool,
    pub(crate) parameters: Vec<DefinitionParameter>,
}

impl PlatformCallbackShape {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        platform_variant: &str,
        metadata_kind_name: &str,
        module_kind_name: &str,
        method_name: &str,
        exported: bool,
        parameters: Vec<DefinitionParameter>,
    ) -> Result<Self, String> {
        if metadata_kind(metadata_kind_name).is_none() {
            return Err("callback metadata kind is not registered".to_string());
        }
        if module_kind(module_kind_name).is_none()
            && !matches!(module_kind_name, "FormModule" | "CommonModule")
        {
            return Err("callback module kind is not registered".to_string());
        }
        DefinitionShape::new(false, exported, parameters.clone())?;
        Ok(Self {
            platform_variant: stable_component(platform_variant, "callback.platformVariant", 128)?,
            metadata_kind: metadata_kind_name.to_string(),
            module_kind: module_kind_name.to_string(),
            method_name: stable_identifier(method_name, "callback.methodName")?,
            exported,
            parameters,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderFact {
    MetadataPresent {
        subject: ArtifactRef,
    },
    MetadataAbsent {
        subject: ArtifactRef,
    },
    CodeOccurrence {
        subject: ArtifactRef,
        search_term: String,
    },
    DefinitionPresent {
        subject: ArtifactRef,
        definition: DefinitionShape,
    },
    DefinitionAbsent {
        subject: ArtifactRef,
    },
    Binding {
        subject: ArtifactRef,
        object: ArtifactRef,
        relation: FlowKind,
        details: BindingDetails,
    },
    Call {
        subject: ArtifactRef,
        object: ArtifactRef,
        resolution: CallResolution,
        call_type: CallType,
        context: super::contract::ExecutionContext,
    },
    PlatformCallback {
        subject: ArtifactRef,
        object: ArtifactRef,
        callback: PlatformCallbackShape,
    },
    Support {
        subject: ArtifactRef,
        state: SupportState,
    },
}

impl ProviderFact {
    pub(crate) const VARIANT_STABLE_TAGS: [u16; 9] = [1, 2, 3, 4, 5, 6, 7, 8, 9];

    pub(crate) fn stable_tag(&self) -> u16 {
        match self {
            Self::MetadataPresent { .. } => 1,
            Self::MetadataAbsent { .. } => 2,
            Self::CodeOccurrence { .. } => 3,
            Self::DefinitionPresent { .. } => 4,
            Self::DefinitionAbsent { .. } => 5,
            Self::Binding { .. } => 6,
            Self::Call { .. } => 7,
            Self::PlatformCallback { .. } => 8,
            Self::Support { .. } => 9,
        }
    }

    pub(crate) fn evidence_type(&self) -> EvidenceType {
        match self {
            Self::MetadataPresent { .. } | Self::MetadataAbsent { .. } => EvidenceType::Metadata,
            Self::CodeOccurrence { .. } => EvidenceType::CodeOccurrence,
            Self::DefinitionPresent { .. } | Self::DefinitionAbsent { .. } => {
                EvidenceType::Definition
            }
            Self::Binding { .. } => EvidenceType::Binding,
            Self::Call { .. } => EvidenceType::Call,
            Self::PlatformCallback { .. } => EvidenceType::PlatformCallback,
            Self::Support { .. } => EvidenceType::Support,
        }
    }

    pub(crate) fn subject(&self) -> &ArtifactRef {
        match self {
            Self::MetadataPresent { subject }
            | Self::MetadataAbsent { subject }
            | Self::CodeOccurrence { subject, .. }
            | Self::DefinitionPresent { subject, .. }
            | Self::DefinitionAbsent { subject }
            | Self::Binding { subject, .. }
            | Self::Call { subject, .. }
            | Self::PlatformCallback { subject, .. }
            | Self::Support { subject, .. } => subject,
        }
    }

    pub(crate) fn object(&self) -> Option<&ArtifactRef> {
        match self {
            Self::Binding { object, .. }
            | Self::Call { object, .. }
            | Self::PlatformCallback { object, .. } => Some(object),
            _ => None,
        }
    }

    pub(crate) fn fact_code(&self) -> String {
        match self {
            Self::MetadataPresent { .. } => "metadata_present".to_string(),
            Self::MetadataAbsent { .. } => "metadata_absent".to_string(),
            Self::CodeOccurrence { .. } => "code_occurrence".to_string(),
            Self::DefinitionPresent { .. } => "definition_present".to_string(),
            Self::DefinitionAbsent { .. } => "definition_absent".to_string(),
            Self::Binding { relation, .. } => format!(
                "binding_{}",
                match relation {
                    FlowKind::Contains => "contains",
                    FlowKind::Defines => "defines",
                    FlowKind::Calls => "calls",
                    FlowKind::Handles => "handles",
                    FlowKind::Subscribes => "subscribes",
                    FlowKind::Uses => "uses",
                }
            ),
            Self::Call { .. } => "call".to_string(),
            Self::PlatformCallback { .. } => "platform_callback".to_string(),
            Self::Support { state, .. } => match state {
                SupportState::Editable => "support_editable",
                SupportState::Locked => "support_locked",
                SupportState::ConfigurationReadOnly => "support_configuration_read_only",
                SupportState::Removed => "support_removed",
                SupportState::NotUnderSupport => "support_not_under_support",
                SupportState::ExtensionOwned => "support_extension_owned",
                SupportState::ExtensionRequired => "support_extension_required",
                SupportState::Unknown => "support_unknown",
            }
            .to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvidenceRecord {
    pub(crate) fact: ProviderFact,
    pub(crate) location: Option<SourceLocation>,
    pub(crate) provider: EvidenceProvider,
    pub(crate) coverage: Coverage,
    pub(crate) freshness: Freshness,
}

impl EvidenceRecord {
    pub(crate) fn from_fact(
        fact: ProviderFact,
        location: Option<SourceLocation>,
        provider: EvidenceProvider,
        coverage: Coverage,
        freshness: Freshness,
    ) -> Self {
        Self {
            fact,
            location,
            provider,
            coverage,
            freshness,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Evidence {
    pub(crate) id: String,
    pub(crate) evidence_type: EvidenceType,
    pub(crate) subject: ArtifactRef,
    pub(crate) fact_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) object: Option<ArtifactRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) location: Option<SourceLocation>,
    pub(crate) provider: EvidenceProvider,
    pub(crate) coverage: Coverage,
    pub(crate) freshness: Freshness,
}

impl Evidence {
    pub(crate) fn from_record(id: String, record: EvidenceRecord) -> Self {
        let evidence_type = record.fact.evidence_type();
        let subject = record.fact.subject().clone();
        let fact_code = record.fact.fact_code();
        let object = record.fact.object().cloned();
        Self {
            id,
            evidence_type,
            subject,
            fact_code,
            object,
            location: record.location,
            provider: record.provider,
            coverage: record.coverage,
            freshness: record.freshness,
        }
    }

    pub(crate) fn is_byte_identical(&self, other: &Self) -> bool {
        fn exact_artifact(left: &ArtifactRef, right: &ArtifactRef) -> bool {
            left.kind == right.kind && left.canonical_ref == right.canonical_ref
        }

        self.id == other.id
            && self.evidence_type == other.evidence_type
            && exact_artifact(&self.subject, &other.subject)
            && self.fact_code == other.fact_code
            && match (&self.object, &other.object) {
                (Some(left), Some(right)) => exact_artifact(left, right),
                (None, None) => true,
                _ => false,
            }
            && self.location == other.location
            && self.provider == other.provider
            && self.coverage == other.coverage
            && self.freshness == other.freshness
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RelatedArtifact {
    pub(crate) artifact: ArtifactRef,
    pub(crate) evidence_level: EvidenceLevel,
    pub(crate) reason_codes: Vec<String>,
    pub(crate) evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct FlowEdge {
    pub(crate) from: ArtifactRef,
    pub(crate) to: ArtifactRef,
    pub(crate) kind: FlowKind,
    pub(crate) evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Candidate {
    pub(crate) target: ArtifactRef,
    pub(crate) evidence_level: EvidenceLevel,
    pub(crate) support_state: SupportState,
    pub(crate) reason_codes: Vec<String>,
    pub(crate) evidence_ids: Vec<String>,
    pub(crate) blockers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ProposalFacts {
    pub(crate) exists: FactAnswer,
    pub(crate) runtime_reachable: FactAnswer,
    pub(crate) support: SupportState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ProposalVerdict {
    pub(crate) proposal_id: String,
    pub(crate) verdict: Verdict,
    pub(crate) facts: ProposalFacts,
    pub(crate) evidence_ids: Vec<String>,
    pub(crate) coverage_gaps: Vec<String>,
    pub(crate) blockers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ReceiptEligibility {
    pub(crate) eligible: bool,
    pub(crate) blockers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct LinkedSourceSnapshot {
    pub(crate) source_set: String,
    pub(crate) role: SourceSnapshotRole,
    pub(crate) source_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct DiscoverySource {
    pub(crate) analysis_source_set: String,
    pub(crate) source_format: SourceFormat,
    pub(crate) workspace_epoch: u64,
    pub(crate) linked_source_snapshots: Vec<LinkedSourceSnapshot>,
    pub(crate) composite_source_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Check {
    pub(crate) code: String,
    pub(crate) provider: String,
    pub(crate) state: CheckState,
    pub(crate) outcome: CheckOutcome,
    pub(crate) coverage: Coverage,
    pub(crate) severity: CheckSeverity,
    pub(crate) affects: Vec<String>,
    pub(crate) reason_code: String,
    pub(crate) retryable: bool,
    pub(crate) details: Vec<String>,
    pub(crate) evidence_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CheckWire {
    code: String,
    provider: String,
    state: CheckState,
    outcome: CheckOutcome,
    coverage: Coverage,
    severity: CheckSeverity,
    affects: Vec<String>,
    reason_code: String,
    retryable: bool,
    details: Vec<String>,
    evidence_ids: Vec<String>,
}

impl Check {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        code: &str,
        provider: &str,
        state: CheckState,
        outcome: CheckOutcome,
        coverage: Coverage,
        severity: CheckSeverity,
        affects: Vec<String>,
        reason_code: &str,
        retryable: bool,
        details: Vec<String>,
        evidence_ids: Vec<String>,
    ) -> Result<Self, String> {
        let check = Self {
            code: code.to_string(),
            provider: provider.to_string(),
            state,
            outcome,
            coverage,
            severity,
            affects,
            reason_code: reason_code.to_string(),
            retryable,
            details,
            evidence_ids,
        };
        check.validate()?;
        Ok(check)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        stable_code(&self.code, "check.code")?;
        if EvidencePort::parse_wire_name(&self.provider).is_none() {
            return Err("check.provider must name one of the six evidence ports".to_string());
        }
        validate_bounded_list(self.affects.clone(), "check.affects", 128, 256)?;
        stable_code(&self.reason_code, "check.reasonCode")?;
        if self.details.len() > 32 {
            return Err("check details must contain at most 32 entries".to_string());
        }
        for detail in &self.details {
            stable_component(detail, "check.details", 512)?;
        }
        validate_bounded_list(self.evidence_ids.clone(), "check.evidenceIds", 2000, 80)?;
        Ok(())
    }
}

impl<'de> Deserialize<'de> for Check {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = CheckWire::deserialize(deserializer)?;
        Self::new(
            &wire.code,
            &wire.provider,
            wire.state,
            wire.outcome,
            wire.coverage,
            wire.severity,
            wire.affects,
            &wire.reason_code,
            wire.retryable,
            wire.details,
            wire.evidence_ids,
        )
        .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscoveryReport {
    pub(crate) schema_version: u16,
    pub(crate) status: DiscoveryStatus,
    pub(crate) analysis_id: String,
    pub(crate) source: DiscoverySource,
    pub(crate) related_artifacts: Vec<RelatedArtifact>,
    pub(crate) flow_edges: Vec<FlowEdge>,
    pub(crate) extension_point_candidates: Vec<Candidate>,
    pub(crate) proposal_verdicts: Vec<ProposalVerdict>,
    pub(crate) evidence: Vec<Evidence>,
    pub(crate) checks: Vec<Check>,
    pub(crate) receipt_eligibility: ReceiptEligibility,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DiscoveryReportWire {
    schema_version: u16,
    status: DiscoveryStatus,
    analysis_id: String,
    source: DiscoverySource,
    related_artifacts: Vec<RelatedArtifact>,
    flow_edges: Vec<FlowEdge>,
    extension_point_candidates: Vec<Candidate>,
    proposal_verdicts: Vec<ProposalVerdict>,
    evidence: Vec<Evidence>,
    checks: Vec<Check>,
    receipt_eligibility: ReceiptEligibility,
}

impl From<DiscoveryReport> for DiscoveryReportWire {
    fn from(report: DiscoveryReport) -> Self {
        Self {
            schema_version: report.schema_version,
            status: report.status,
            analysis_id: report.analysis_id,
            source: report.source,
            related_artifacts: report.related_artifacts,
            flow_edges: report.flow_edges,
            extension_point_candidates: report.extension_point_candidates,
            proposal_verdicts: report.proposal_verdicts,
            evidence: report.evidence,
            checks: report.checks,
            receipt_eligibility: report.receipt_eligibility,
        }
    }
}

impl From<DiscoveryReportWire> for DiscoveryReport {
    fn from(report: DiscoveryReportWire) -> Self {
        Self {
            schema_version: report.schema_version,
            status: report.status,
            analysis_id: report.analysis_id,
            source: report.source,
            related_artifacts: report.related_artifacts,
            flow_edges: report.flow_edges,
            extension_point_candidates: report.extension_point_candidates,
            proposal_verdicts: report.proposal_verdicts,
            evidence: report.evidence,
            checks: report.checks,
            receipt_eligibility: report.receipt_eligibility,
        }
    }
}

impl Serialize for DiscoveryReport {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut report = self.clone();
        report.canonicalize().map_err(S::Error::custom)?;
        DiscoveryReportWire::from(report).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DiscoveryReport {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut report = Self::from(DiscoveryReportWire::deserialize(deserializer)?);
        report.canonicalize().map_err(D::Error::custom)?;
        Ok(report)
    }
}

impl DiscoveryReport {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        status: DiscoveryStatus,
        analysis_id: String,
        source: DiscoverySource,
        related_artifacts: Vec<RelatedArtifact>,
        flow_edges: Vec<FlowEdge>,
        extension_point_candidates: Vec<Candidate>,
        proposal_verdicts: Vec<ProposalVerdict>,
        evidence: Vec<Evidence>,
        checks: Vec<Check>,
        receipt_eligibility: ReceiptEligibility,
    ) -> Result<Self, String> {
        let mut report = Self {
            schema_version: 1,
            status,
            analysis_id,
            source,
            related_artifacts,
            flow_edges,
            extension_point_candidates,
            proposal_verdicts,
            evidence,
            checks,
            receipt_eligibility,
        };
        report.canonicalize()?;
        Ok(report)
    }

    pub(crate) fn canonicalize(&mut self) -> Result<(), String> {
        // Validate before collapsing exact duplicates so an identifier
        // collision can never be hidden by canonical ordering.
        self.validate()?;
        super::determinism::canonicalize_report(self);
        self.validate()
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("discovery report schemaVersion must equal 1".to_string());
        }
        validate_digest_id(&self.analysis_id, "analysis_", "analysisId")?;
        self.validate_source()?;

        let mut evidence_by_id: BTreeMap<&str, &Evidence> = BTreeMap::new();
        for item in &self.evidence {
            validate_digest_id(&item.id, "ev_", "evidence.id")?;
            if let Some(previous) = evidence_by_id.get(item.id.as_str()) {
                if !previous.is_byte_identical(item) {
                    return Err(format!("evidence identifier collision: {}", item.id));
                }
            } else {
                evidence_by_id.insert(item.id.as_str(), item);
            }
            item.subject.validate()?;
            if let Some(object) = &item.object {
                object.validate()?;
            }
            validate_evidence_fact_shape(item)?;
            if let Some(location) = &item.location {
                location.validate()?;
            }
            item.provider.validate()?;
            item.freshness.validate()?;
            let matching_snapshot = self.source.linked_source_snapshots.iter().any(|snapshot| {
                snapshot.source_set == item.freshness.source_set
                    && snapshot.source_fingerprint == item.freshness.source_fingerprint
            });
            if !matching_snapshot {
                return Err("evidence freshness does not match a linked source snapshot".into());
            }
        }

        let validate_references = |ids: &[String], field: &str| -> Result<(), String> {
            if ids.len() > 2000 {
                return Err(format!("{field} must contain at most 2000 entries"));
            }
            for id in ids {
                validate_digest_id(id, "ev_", field)?;
                if !evidence_by_id.contains_key(id.as_str()) {
                    return Err(format!("{field} references unknown evidence id {id}"));
                }
            }
            Ok(())
        };

        let mut related_identities = BTreeSet::new();
        for related in &self.related_artifacts {
            related.artifact.validate()?;
            if !related_identities.insert(related.artifact.clone()) {
                return Err("relatedArtifacts contains duplicate artifact identities".into());
            }
            validate_code_list(&related.reason_codes, "relatedArtifacts.reasonCodes", 128)?;
            validate_references(&related.evidence_ids, "relatedArtifacts.evidenceIds")?;
        }

        let mut edge_identities = BTreeSet::new();
        for edge in &self.flow_edges {
            edge.from.validate()?;
            edge.to.validate()?;
            if !edge_identities.insert((edge.from.clone(), edge.to.clone(), edge.kind)) {
                return Err("flowEdges contains duplicate edge identities".into());
            }
            validate_references(&edge.evidence_ids, "flowEdges.evidenceIds")?;
        }

        let mut candidate_identities = BTreeSet::new();
        let mut candidate_refs = BTreeSet::new();
        for candidate in &self.extension_point_candidates {
            candidate.target.validate()?;
            if !candidate_identities.insert(candidate.target.clone()) {
                return Err("extensionPointCandidates contains duplicate target identities".into());
            }
            candidate_refs.insert(candidate.target.canonical_ref.as_str());
            validate_code_list(
                &candidate.reason_codes,
                "extensionPointCandidates.reasonCodes",
                128,
            )?;
            validate_code_list(
                &candidate.blockers,
                "extensionPointCandidates.blockers",
                128,
            )?;
            validate_references(
                &candidate.evidence_ids,
                "extensionPointCandidates.evidenceIds",
            )?;
            if candidate.evidence_level == EvidenceLevel::Actionable
                && candidate.support_state == SupportState::Unknown
            {
                return Err("an actionable candidate must have a known support state".into());
            }
        }

        let mut proposal_ids = BTreeSet::new();
        for verdict in &self.proposal_verdicts {
            validate_proposal_id_value(&verdict.proposal_id)?;
            if !proposal_ids.insert(verdict.proposal_id.as_str()) {
                return Err("proposalVerdicts contains duplicate proposalId values".into());
            }
            validate_references(&verdict.evidence_ids, "proposalVerdicts.evidenceIds")?;
            validate_code_list(&verdict.coverage_gaps, "proposalVerdicts.coverageGaps", 128)?;
            validate_code_list(&verdict.blockers, "proposalVerdicts.blockers", 128)?;
            validate_verdict_consistency(verdict)?;
        }

        for check in &self.checks {
            check.validate()?;
            validate_references(&check.evidence_ids, "checks.evidenceIds")?;
            for affect in &check.affects {
                if let Some(proposal_id) = affect.strip_prefix("proposal:") {
                    if !proposal_ids.contains(proposal_id) {
                        return Err(format!("check affects unknown proposal {proposal_id}"));
                    }
                } else if let Some(candidate_ref) = affect.strip_prefix("candidate:") {
                    if !candidate_refs.contains(candidate_ref) {
                        return Err(format!("check affects unknown candidate {candidate_ref}"));
                    }
                } else {
                    return Err(format!("check has unsupported affects reference {affect}"));
                }
            }
        }

        validate_code_list(
            &self.receipt_eligibility.blockers,
            "receiptEligibility.blockers",
            128,
        )?;
        self.validate_receipt_eligibility()?;
        Ok(())
    }

    fn validate_source(&self) -> Result<(), String> {
        stable_component(
            &self.source.analysis_source_set,
            "source.analysisSourceSet",
            1024,
        )?;
        validate_sha256_fingerprint(&self.source.composite_source_fingerprint)?;
        if self.source.linked_source_snapshots.is_empty() {
            return Err("source.linkedSourceSnapshots must not be empty".into());
        }
        let mut snapshot_identities = BTreeSet::new();
        let mut fingerprint_by_source = BTreeMap::new();
        let mut analysis_snapshots = 0usize;
        for snapshot in &self.source.linked_source_snapshots {
            stable_component(
                &snapshot.source_set,
                "linkedSourceSnapshots.sourceSet",
                1024,
            )?;
            validate_sha256_fingerprint(&snapshot.source_fingerprint)?;
            if !snapshot_identities.insert((snapshot.source_set.as_str(), snapshot.role)) {
                return Err("linkedSourceSnapshots contains duplicate source-set roles".into());
            }
            if let Some(previous) = fingerprint_by_source
                .insert(snapshot.source_set.as_str(), &snapshot.source_fingerprint)
            {
                if previous != &snapshot.source_fingerprint {
                    return Err("one linked source-set cannot have conflicting fingerprints".into());
                }
            }
            if snapshot.role == SourceSnapshotRole::Analysis {
                analysis_snapshots += 1;
                if snapshot.source_set != self.source.analysis_source_set {
                    return Err("analysis snapshot does not match analysisSourceSet".into());
                }
            }
        }
        if analysis_snapshots != 1 {
            return Err("exactly one analysis linked source snapshot is required".into());
        }
        Ok(())
    }

    fn validate_receipt_eligibility(&self) -> Result<(), String> {
        if !self.receipt_eligibility.eligible {
            return Ok(());
        }
        if !self.receipt_eligibility.blockers.is_empty() {
            return Err("eligible receipt cannot contain blockers".into());
        }
        if self.proposal_verdicts.is_empty()
            || self.proposal_verdicts.iter().any(|verdict| {
                verdict.verdict != Verdict::Supported
                    || verdict.facts.exists != FactAnswer::Yes
                    || verdict.facts.runtime_reachable != FactAnswer::Yes
                    || verdict.facts.support == SupportState::Unknown
                    || !verdict.coverage_gaps.is_empty()
                    || !verdict.blockers.is_empty()
            })
        {
            return Err("receipt eligibility requires fully supported proposal verdicts".into());
        }
        let blocking_proposal_check = self.checks.iter().any(|check| {
            check.severity == CheckSeverity::Blocking
                && !matches!(
                    check.outcome,
                    CheckOutcome::Satisfied | CheckOutcome::NotApplicable
                )
                && check
                    .affects
                    .iter()
                    .any(|affect| affect.starts_with("proposal:"))
        });
        if blocking_proposal_check {
            return Err("receipt eligibility cannot ignore a blocking proposal check".into());
        }
        Ok(())
    }
}

fn validate_digest_id(value: &str, prefix: &str, field: &str) -> Result<(), String> {
    let Some(digest) = value.strip_prefix(prefix) else {
        return Err(format!("{field} must start with {prefix}"));
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "{field} must contain a full 64-character lowercase SHA-256 digest"
        ));
    }
    Ok(())
}

fn validate_code_list(values: &[String], field: &str, maximum: usize) -> Result<(), String> {
    if values.len() > maximum {
        return Err(format!("{field} must contain at most {maximum} entries"));
    }
    for value in values {
        stable_code(value, field)?;
    }
    Ok(())
}

fn validate_evidence_fact_shape(evidence: &Evidence) -> Result<(), String> {
    stable_code(&evidence.fact_code, "evidence.factCode")?;
    let object_is_present = evidence.object.is_some();
    let compatible = match evidence.evidence_type {
        EvidenceType::Metadata => {
            !object_is_present
                && matches!(
                    evidence.fact_code.as_str(),
                    "metadata_present" | "metadata_absent"
                )
        }
        EvidenceType::CodeOccurrence => {
            !object_is_present && evidence.fact_code == "code_occurrence"
        }
        EvidenceType::Definition => {
            !object_is_present
                && matches!(
                    evidence.fact_code.as_str(),
                    "definition_present" | "definition_absent"
                )
        }
        EvidenceType::Binding => {
            object_is_present
                && matches!(
                    evidence.fact_code.as_str(),
                    "binding_contains"
                        | "binding_defines"
                        | "binding_calls"
                        | "binding_handles"
                        | "binding_subscribes"
                        | "binding_uses"
                )
        }
        EvidenceType::Call => object_is_present && evidence.fact_code == "call",
        EvidenceType::PlatformCallback => {
            object_is_present && evidence.fact_code == "platform_callback"
        }
        EvidenceType::Support => {
            !object_is_present
                && matches!(
                    evidence.fact_code.as_str(),
                    "support_editable"
                        | "support_locked"
                        | "support_configuration_read_only"
                        | "support_removed"
                        | "support_not_under_support"
                        | "support_extension_owned"
                        | "support_extension_required"
                        | "support_unknown"
                )
        }
    };
    if compatible {
        Ok(())
    } else {
        Err("evidence type, factCode, and object shape are inconsistent".into())
    }
}

fn validate_verdict_consistency(verdict: &ProposalVerdict) -> Result<(), String> {
    match verdict.verdict {
        Verdict::Supported
            if verdict.facts.exists == FactAnswer::Yes
                && verdict.facts.runtime_reachable == FactAnswer::Yes
                && verdict.facts.support != SupportState::Unknown =>
        {
            Ok(())
        }
        Verdict::Contradicted
            if verdict.facts.exists == FactAnswer::No
                || verdict.facts.runtime_reachable == FactAnswer::No =>
        {
            Ok(())
        }
        Verdict::Unknown
            if verdict.facts.exists == FactAnswer::Unknown
                || verdict.facts.runtime_reachable == FactAnswer::Unknown
                || verdict.facts.support == SupportState::Unknown
                || !verdict.coverage_gaps.is_empty()
                || !verdict.blockers.is_empty() =>
        {
            Ok(())
        }
        _ => Err("proposal verdict and typed facts are inconsistent".into()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderOutcomeSnapshot {
    pub(crate) port: EvidencePort,
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) readiness: ProviderReadiness,
    pub(crate) coverage: Coverage,
    pub(crate) reason_code: Option<String>,
    pub(crate) record_digests: Vec<String>,
}

impl ProviderOutcomeSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        port: EvidencePort,
        name: &str,
        version: &str,
        readiness: ProviderReadiness,
        coverage: Coverage,
        reason_code: Option<String>,
        record_digests: Vec<String>,
    ) -> Result<Self, String> {
        let reason_code = reason_code
            .map(|value| stable_code(&value, "provider.reasonCode"))
            .transpose()?;
        let mut record_digests = record_digests
            .into_iter()
            .map(|value| validate_sha256_fingerprint(&value))
            .collect::<Result<Vec<_>, _>>()?;
        record_digests.sort();
        record_digests.dedup();
        let snapshot = Self {
            port,
            name: stable_component(name, "provider.name", 128)?,
            version: stable_component(version, "provider.version", 128)?,
            readiness,
            coverage,
            reason_code,
            record_digests,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        stable_component(&self.name, "provider.name", 128)?;
        stable_component(&self.version, "provider.version", 128)?;
        if let Some(reason) = &self.reason_code {
            stable_code(reason, "provider.reasonCode")?;
        }
        for digest in &self.record_digests {
            validate_sha256_fingerprint(digest)?;
        }
        match (self.readiness, self.coverage, self.reason_code.is_some()) {
            (ProviderReadiness::Ready, Coverage::Complete, false)
            | (ProviderReadiness::Ready, Coverage::Bounded, true) => {}
            (ProviderReadiness::Unavailable, Coverage::Unknown | Coverage::Bounded, true)
            | (ProviderReadiness::Failed, Coverage::Unknown, true)
                if self.record_digests.is_empty() => {}
            _ => {
                return Err(
                    "provider readiness, coverage, reason, and records are inconsistent"
                        .to_string(),
                );
            }
        }
        Ok(())
    }
}

pub(crate) fn validate_sha256_fingerprint(value: &str) -> Result<String, String> {
    let Some(digest) = value.strip_prefix("sha256:") else {
        return Err("fingerprint must start with sha256:".to_string());
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("fingerprint must contain 64 lowercase hexadecimal characters".to_string());
    }
    Ok(value.to_string())
}

fn stable_component(value: &str, field: &str, max_bytes: usize) -> Result<String, String> {
    if value.trim().is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(format!(
            "{field} must contain 1..={max_bytes} stable UTF-8 bytes"
        ));
    }
    Ok(value.to_string())
}

fn stable_identifier(value: &str, field: &str) -> Result<String, String> {
    let value = stable_component(value, field, 512)?;
    if value.chars().count() > 128
        || !value
            .chars()
            .all(|character| character.is_alphanumeric() || character == '_')
    {
        return Err(format!("{field} must be one canonical identifier segment"));
    }
    Ok(value)
}

fn stable_code(value: &str, field: &str) -> Result<String, String> {
    let value = stable_component(value, field, 128)?;
    if !value.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'.' | b'-')
    }) {
        return Err(format!("{field} is not a stable code"));
    }
    Ok(value)
}

fn validate_bounded_list(
    values: Vec<String>,
    field: &str,
    maximum: usize,
    max_bytes: usize,
) -> Result<Vec<String>, String> {
    if values.len() > maximum {
        return Err(format!("{field} must contain at most {maximum} entries"));
    }
    values
        .into_iter()
        .map(|value| stable_component(&value, field, max_bytes))
        .collect()
}
