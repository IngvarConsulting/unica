//! Format-neutral ports implemented by concrete source adapters.

use std::{
    any::Any,
    ffi::OsString,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    navigation::{
        Authorability, CapabilityVector, NavigationEnvelope, NavigationQuery, ObjectKey, ObjectRef,
        PropertyValue, SourceAdapterDiagnostic,
    },
    semantic_ids::{SemanticPropertyId, SemanticRelationId},
    source::{
        AdapterManifest, ConfiguredSourceSetKind, FormatVersion, SourceAdapterError, SourceBinding,
        SourceContext, SourceDescriptor, SourceRevision, SourceSnapshot,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeResult {
    NoMatch,
    Match(SourceDescriptor),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceSetMatch {
    NoMatch,
    Match,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReservedSourceArtifactKind {
    RuntimeState,
    AuthoredSource,
    Unknown,
}

pub trait ProbePort: Send + Sync {
    fn probe(&self, captured: &CapturedSource) -> Result<ProbeResult, SourceAdapterError>;
}

pub trait CapturedSourceSession: Send + Sync {
    fn source(&self) -> &SourceContext;
    fn snapshot(&self) -> &SourceSnapshot;
    fn binding(&self) -> &SourceBinding;
    fn as_any(&self) -> &dyn Any;
}

#[derive(Clone)]
pub struct CapturedSource(Arc<dyn CapturedSourceSession>);

impl CapturedSource {
    pub fn new(session: impl CapturedSourceSession + 'static) -> Self {
        Self(Arc::new(session))
    }

    pub fn source(&self) -> &SourceContext {
        self.0.source()
    }

    pub fn snapshot(&self) -> &SourceSnapshot {
        self.0.snapshot()
    }

    pub fn binding(&self) -> &SourceBinding {
        self.0.binding()
    }

    pub fn adapter_state<T: Any>(&self) -> Option<&T> {
        self.0.as_any().downcast_ref()
    }
}

pub enum CaptureResult {
    NoMatch,
    Captured(CapturedSource),
}

pub trait CapturePort: Send + Sync {
    fn capture(&self, source: &SourceContext) -> Result<CaptureResult, SourceAdapterError>;
}

#[derive(Clone)]
pub struct FormatReadRequest {
    pub captured: CapturedSource,
    pub query: NavigationQuery,
}

pub trait ReadPort: Send + Sync {
    fn read(&self, request: &FormatReadRequest) -> Result<NavigationEnvelope, SourceAdapterError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatCompatibilityKind {
    Older,
    Supported,
    Newer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatCompatibility {
    Older {
        actual: FormatVersion,
        target: FormatVersion,
    },
    Supported {
        actual: FormatVersion,
        target: FormatVersion,
    },
    Newer {
        actual: FormatVersion,
        target: FormatVersion,
    },
}

impl FormatCompatibility {
    pub fn actual(&self) -> &FormatVersion {
        match self {
            Self::Older { actual, .. }
            | Self::Supported { actual, .. }
            | Self::Newer { actual, .. } => actual,
        }
    }

    pub fn target(&self) -> &FormatVersion {
        match self {
            Self::Older { target, .. }
            | Self::Supported { target, .. }
            | Self::Newer { target, .. } => target,
        }
    }

    pub const fn kind(&self) -> FormatCompatibilityKind {
        match self {
            Self::Older { .. } => FormatCompatibilityKind::Older,
            Self::Supported { .. } => FormatCompatibilityKind::Supported,
            Self::Newer { .. } => FormatCompatibilityKind::Newer,
        }
    }

    pub const fn label(&self) -> &'static str {
        match self {
            Self::Older { .. } => "older",
            Self::Supported { .. } => "supported",
            Self::Newer { .. } => "newer",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceOwnerEvidence {
    pub configured_source_kind: Option<ConfiguredSourceSetKind>,
    pub path: PathBuf,
    pub format: FormatCompatibility,
    pub producer_version: Option<FormatVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceInputEvidence {
    ExactFileSha256 {
        path: PathBuf,
        sha256: String,
    },
    PathAbsent {
        path: PathBuf,
    },
    DirectoryMembership {
        directory: PathBuf,
        names: Vec<OsString>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerResolutionMode {
    Existing,
    ExistingForNewOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerResolutionRequest {
    pub source: SourceContext,
    pub mode: OwnerResolutionMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerResolutionResult {
    pub owners: Vec<SourceOwnerEvidence>,
    pub evidence: Vec<SourceInputEvidence>,
}

pub trait OwnershipPort: Send + Sync {
    fn resolve(
        &self,
        request: &OwnerResolutionRequest,
    ) -> Result<OwnerResolutionResult, SourceAdapterError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatInspectionMode {
    Versioned,
    Versionless,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatInspectionRequest {
    pub source: SourceContext,
    pub mode: FormatInspectionMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatInspectionResult {
    pub compatibility: Option<FormatCompatibility>,
}

pub trait FormatInspectionPort: Send + Sync {
    fn inspect(
        &self,
        request: &FormatInspectionRequest,
    ) -> Result<FormatInspectionResult, SourceAdapterError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupportSourceState {
    Absent,
    Removed,
    Parsed,
    Unreadable {
        context: String,
        offset: Option<usize>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectiveSupportRule {
    Absent,
    Removed,
    Editable,
    Locked,
    ConfigurationReadOnly,
    UnknownReadOnly,
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportVendorEvidence {
    pub version: String,
    pub vendor: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportEvidence {
    pub source: SupportSourceState,
    pub effective_rule: EffectiveSupportRule,
    pub authorability: Authorability,
    pub global_editing_enabled: Option<bool>,
    pub rule_counts: [usize; 3],
    pub vendors: Vec<SupportVendorEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportInspectionRequest {
    pub source: SourceContext,
    pub object: Option<ObjectKey>,
}

pub trait SupportPort: Send + Sync {
    fn inspect(
        &self,
        request: &SupportInspectionRequest,
    ) -> Result<SupportEvidence, SourceAdapterError>;
}

#[derive(Clone)]
pub struct SourceAdapterRegistration {
    pub manifest: AdapterManifest,
    pub capture: Arc<dyn CapturePort>,
    pub probe: Arc<dyn ProbePort>,
    pub read: Arc<dyn ReadPort>,
    pub ownership: Arc<dyn OwnershipPort>,
    pub format_inspection: Arc<dyn FormatInspectionPort>,
    pub support: Arc<dyn SupportPort>,
}

/// Closed mutation language. Native parser nodes and `serde_json::Value` are
/// deliberately absent from this boundary.
///
/// ```compile_fail
/// use unica_format_core::{
///     ports::FormatWriteCommand,
///     semantic_ids::SemanticPropertyId,
/// };
/// let command = FormatWriteCommand::SetProperty {
///     target: todo!(),
///     property: SemanticPropertyId::METADATA_NAME,
///     value: serde_json::json!({"unsafe": true}),
/// };
/// ```
///
/// ```compile_fail
/// use unica_format_core::ports::FormatWriteCommand;
/// let command: FormatWriteCommand = serde_json::json!({
///     "operation": "adapter-native",
///     "payload": {"raw": true}
/// }).into();
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatWriteCommand {
    SetProperty {
        target: ObjectRef,
        property: SemanticPropertyId,
        value: PropertyValue,
    },
    RemoveProperty {
        target: ObjectRef,
        property: SemanticPropertyId,
    },
    AddRelation {
        source: ObjectRef,
        relation: SemanticRelationId,
        target: ObjectRef,
    },
    RemoveRelation {
        source: ObjectRef,
        relation: SemanticRelationId,
        target: ObjectRef,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatWriteRequest {
    pub source: SourceContext,
    pub snapshot: SourceSnapshot,
    pub commands: Vec<FormatWriteCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatWriteResult {
    pub revision: SourceRevision,
    pub diagnostics: Vec<SourceAdapterDiagnostic>,
}

pub trait WritePort: Send + Sync {
    fn write(&self, request: &FormatWriteRequest) -> Result<FormatWriteResult, SourceAdapterError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationScope {
    Source,
    Object(ObjectRef),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatValidationRequest {
    pub source: SourceContext,
    pub snapshot: SourceSnapshot,
    pub scope: ValidationScope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatValidationResult {
    pub valid: bool,
    pub diagnostics: Vec<SourceAdapterDiagnostic>,
}

pub trait ValidationPort: Send + Sync {
    fn validate(
        &self,
        request: &FormatValidationRequest,
    ) -> Result<FormatValidationResult, SourceAdapterError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRequest {
    pub source: SourceContext,
    pub snapshot: SourceSnapshot,
    pub target: ObjectRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityResult {
    pub capability: CapabilityVector,
    pub diagnostics: Vec<SourceAdapterDiagnostic>,
}

pub trait CapabilityPort: Send + Sync {
    fn capabilities(
        &self,
        request: &CapabilityRequest,
    ) -> Result<CapabilityResult, SourceAdapterError>;
}

#[derive(Clone)]
pub struct OperationalSourceSession {
    state: Arc<dyn Any + Send + Sync>,
}

impl std::fmt::Debug for OperationalSourceSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OperationalSourceSession(<opaque>)")
    }
}

impl OperationalSourceSession {
    pub fn new<T>(state: T) -> Self
    where
        T: Any + Send + Sync,
    {
        Self {
            state: Arc::new(state),
        }
    }

    pub fn adapter_state<T>(&self) -> Option<&T>
    where
        T: Any + Send + Sync,
    {
        self.state.downcast_ref::<T>()
    }
}

#[derive(Clone)]
pub struct SemanticArtifactLease {
    state: Arc<dyn Any + Send + Sync>,
}

impl std::fmt::Debug for SemanticArtifactLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SemanticArtifactLease(<opaque>)")
    }
}

impl SemanticArtifactLease {
    pub fn new<T>(state: T) -> Self
    where
        T: Any + Send + Sync,
    {
        Self {
            state: Arc::new(state),
        }
    }

    pub fn adapter_state<T>(&self) -> Option<&T>
    where
        T: Any + Send + Sync,
    {
        self.state.downcast_ref::<T>()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectKindSelector(String);

impl ObjectKindSelector {
    pub fn new(value: impl Into<String>) -> Result<Self, OperationalContractError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || value.chars().any(|character| {
                character.is_control()
                    || matches!(character, '/' | '\\' | ':' | '<' | '>' | '"' | '\'')
            })
        {
            return Err(OperationalContractError::InvalidSemanticValue);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectKindProjection {
    kind: crate::semantic_ids::SemanticObjectKind,
    canonical_selector: ObjectKindSelector,
    collection_selector: ObjectKindSelector,
    display_label: String,
}

impl ObjectKindProjection {
    pub fn new(
        kind: crate::semantic_ids::SemanticObjectKind,
        canonical_selector: ObjectKindSelector,
        collection_selector: ObjectKindSelector,
        display_label: impl Into<String>,
    ) -> Result<Self, OperationalContractError> {
        let display_label = display_label.into();
        if matches!(
            kind,
            crate::semantic_ids::SemanticObjectKind::SourceRoot
                | crate::semantic_ids::SemanticObjectKind::Unknown
        ) || display_label.is_empty()
            || display_label.len() > 256
            || display_label.chars().any(char::is_control)
        {
            return Err(OperationalContractError::InvalidSemanticValue);
        }
        Ok(Self {
            kind,
            canonical_selector,
            collection_selector,
            display_label,
        })
    }

    pub const fn kind(&self) -> crate::semantic_ids::SemanticObjectKind {
        self.kind
    }

    pub fn canonical_selector(&self) -> &ObjectKindSelector {
        &self.canonical_selector
    }

    pub fn collection_selector(&self) -> &ObjectKindSelector {
        &self.collection_selector
    }

    pub fn display_label(&self) -> &str {
        &self.display_label
    }
}

pub trait ObjectKindRegistryPort: Send + Sync {
    fn resolve(
        &self,
        selector: &ObjectKindSelector,
    ) -> Option<crate::semantic_ids::SemanticObjectKind>;

    fn ordered_kinds(&self) -> Vec<crate::semantic_ids::SemanticObjectKind>;

    fn lease(&self, kind: crate::semantic_ids::SemanticObjectKind)
        -> Option<SemanticArtifactLease>;

    fn project(&self, lease: &SemanticArtifactLease) -> Option<&'static ObjectKindProjection>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticArtifactRole {
    FormDefinition,
    DataCompositionSchema,
    SpreadsheetDocument,
}

#[derive(Debug, Clone)]
pub struct SemanticArtifactReadRequest {
    session: OperationalSourceSession,
    role: SemanticArtifactRole,
}

impl SemanticArtifactReadRequest {
    pub const fn new(session: OperationalSourceSession, role: SemanticArtifactRole) -> Self {
        Self { session, role }
    }

    pub fn session(&self) -> &OperationalSourceSession {
        &self.session
    }

    pub const fn role(&self) -> SemanticArtifactRole {
        self.role
    }
}

#[derive(Debug, Clone)]
pub enum SemanticArtifactReadResult {
    Absent,
    Present(SemanticArtifactLease),
}

pub trait SemanticArtifactPort: Send + Sync {
    fn read(
        &self,
        request: &SemanticArtifactReadRequest,
    ) -> Result<SemanticArtifactReadResult, SourceAdapterError>;

    fn bytes<'a>(&self, lease: &'a SemanticArtifactLease) -> Option<&'a [u8]>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FormatDiagnosticCode {
    SourceRevisionOlder,
    SourceRevisionNewer,
    SourceMalformed,
    SourceFamilyIncompatible,
    SupportStateUnreadable,
    SupportCapabilityDisabled,
    SupportLocked,
    SupportRemovalRequired,
    ValidationContextUnavailable,
    ValidationReferenceMissing,
    ValidationRegistrarMissing,
    PublicationFailed,
    PublicationCancelled,
    PublicationRecoveryRequired,
    PublicationCleanupFailed,
}

impl FormatDiagnosticCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SourceRevisionOlder => "sourceRevisionOlder",
            Self::SourceRevisionNewer => "sourceRevisionNewer",
            Self::SourceMalformed => "sourceMalformed",
            Self::SourceFamilyIncompatible => "sourceFamilyIncompatible",
            Self::SupportStateUnreadable => "supportStateUnreadable",
            Self::SupportCapabilityDisabled => "supportCapabilityDisabled",
            Self::SupportLocked => "supportLocked",
            Self::SupportRemovalRequired => "supportRemovalRequired",
            Self::ValidationContextUnavailable => "validationContextUnavailable",
            Self::ValidationReferenceMissing => "validationReferenceMissing",
            Self::ValidationRegistrarMissing => "validationRegistrarMissing",
            Self::PublicationFailed => "publicationFailed",
            Self::PublicationCancelled => "publicationCancelled",
            Self::PublicationRecoveryRequired => "publicationRecoveryRequired",
            Self::PublicationCleanupFailed => "publicationCleanupFailed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompatibilityIssueKind {
    Older,
    Newer,
    Malformed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupportState {
    Absent,
    Removed,
    Editable,
    Locked,
    ConfigurationReadOnly,
    UnknownReadOnly,
    Unreadable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValidationIssueKind {
    SourceUnreadable,
    OwnerUnavailable,
    RegistrationMissing,
    LanguageProfileMissing,
    ReferenceMissing,
    RegistrarMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValidationMethodReferenceStatus {
    Valid,
    Invalid,
    TargetMissing,
    ImplementationMissing,
    EntryPointMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicationIssueKind {
    Failed,
    Cancelled,
    RecoveryRequired,
    CleanupFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FormatDiagnosticDetail {
    Compatibility(CompatibilityIssueKind),
    Support(SupportState),
    Validation(ValidationIssueKind),
    Publication(PublicationIssueKind),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatDiagnostic {
    code: FormatDiagnosticCode,
    detail: FormatDiagnosticDetail,
}

impl FormatDiagnostic {
    pub fn new(
        code: FormatDiagnosticCode,
        detail: FormatDiagnosticDetail,
    ) -> Result<Self, OperationalContractError> {
        if !diagnostic_detail_matches(code, detail) {
            return Err(OperationalContractError::InvalidDiagnostic);
        }
        Ok(Self { code, detail })
    }

    pub const fn code(&self) -> FormatDiagnosticCode {
        self.code
    }

    pub const fn detail(&self) -> FormatDiagnosticDetail {
        self.detail
    }
}

impl<'de> Deserialize<'de> for FormatDiagnostic {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            code: FormatDiagnosticCode,
            detail: FormatDiagnosticDetail,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.code, wire.detail).map_err(D::Error::custom)
    }
}

const fn diagnostic_detail_matches(
    code: FormatDiagnosticCode,
    detail: FormatDiagnosticDetail,
) -> bool {
    match (code, detail) {
        (
            FormatDiagnosticCode::SourceRevisionOlder,
            FormatDiagnosticDetail::Compatibility(CompatibilityIssueKind::Older),
        )
        | (
            FormatDiagnosticCode::SourceRevisionNewer,
            FormatDiagnosticDetail::Compatibility(CompatibilityIssueKind::Newer),
        )
        | (
            FormatDiagnosticCode::SourceMalformed | FormatDiagnosticCode::SourceFamilyIncompatible,
            FormatDiagnosticDetail::Compatibility(CompatibilityIssueKind::Malformed),
        )
        | (
            FormatDiagnosticCode::SupportStateUnreadable,
            FormatDiagnosticDetail::Support(SupportState::Unreadable),
        )
        | (
            FormatDiagnosticCode::SupportCapabilityDisabled,
            FormatDiagnosticDetail::Support(SupportState::ConfigurationReadOnly),
        )
        | (
            FormatDiagnosticCode::SupportLocked,
            FormatDiagnosticDetail::Support(SupportState::Locked),
        )
        | (
            FormatDiagnosticCode::SupportRemovalRequired,
            FormatDiagnosticDetail::Support(
                SupportState::Absent
                | SupportState::Editable
                | SupportState::Locked
                | SupportState::UnknownReadOnly
                | SupportState::Unreadable,
            ),
        )
        | (
            FormatDiagnosticCode::ValidationContextUnavailable,
            FormatDiagnosticDetail::Validation(
                ValidationIssueKind::SourceUnreadable
                | ValidationIssueKind::OwnerUnavailable
                | ValidationIssueKind::RegistrationMissing
                | ValidationIssueKind::LanguageProfileMissing,
            ),
        )
        | (
            FormatDiagnosticCode::ValidationReferenceMissing,
            FormatDiagnosticDetail::Validation(ValidationIssueKind::ReferenceMissing),
        )
        | (
            FormatDiagnosticCode::ValidationRegistrarMissing,
            FormatDiagnosticDetail::Validation(ValidationIssueKind::RegistrarMissing),
        )
        | (
            FormatDiagnosticCode::PublicationFailed,
            FormatDiagnosticDetail::Publication(PublicationIssueKind::Failed),
        )
        | (
            FormatDiagnosticCode::PublicationCancelled,
            FormatDiagnosticDetail::Publication(PublicationIssueKind::Cancelled),
        )
        | (
            FormatDiagnosticCode::PublicationRecoveryRequired,
            FormatDiagnosticDetail::Publication(PublicationIssueKind::RecoveryRequired),
        )
        | (
            FormatDiagnosticCode::PublicationCleanupFailed,
            FormatDiagnosticDetail::Publication(PublicationIssueKind::CleanupFailed),
        ) => true,
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationalEvidenceRevision([u8; 32]);

impl OperationalEvidenceRevision {
    pub const fn from_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    pub const fn digest(&self) -> &[u8; 32] {
        &self.0
    }
}

impl Serialize for OperationalEvidenceRevision {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut encoded = String::with_capacity(64);
        for byte in self.0 {
            use std::fmt::Write;
            write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
        }
        serializer.serialize_str(&encoded)
    }
}

impl<'de> Deserialize<'de> for OperationalEvidenceRevision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        if encoded.len() != 64
            || !encoded
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(D::Error::custom(
                OperationalContractError::InvalidSemanticValue,
            ));
        }
        let mut digest = [0_u8; 32];
        for (index, byte) in digest.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&encoded[index * 2..index * 2 + 2], 16)
                .map_err(D::Error::custom)?;
        }
        Ok(Self(digest))
    }
}

#[derive(Debug, Clone)]
pub struct CompatibilityRequest {
    sessions: Vec<OperationalSourceSession>,
}

impl CompatibilityRequest {
    pub fn new(sessions: Vec<OperationalSourceSession>) -> Result<Self, OperationalContractError> {
        if sessions.is_empty() {
            return Err(OperationalContractError::EmptyRequest);
        }
        Ok(Self { sessions })
    }

    pub fn sessions(&self) -> &[OperationalSourceSession] {
        &self.sessions
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityIssue {
    kind: CompatibilityIssueKind,
    diagnostic: FormatDiagnostic,
}

impl CompatibilityIssue {
    pub fn new(kind: CompatibilityIssueKind, diagnostic: FormatDiagnostic) -> Self {
        Self { kind, diagnostic }
    }

    pub const fn kind(&self) -> CompatibilityIssueKind {
        self.kind
    }

    pub fn diagnostic(&self) -> &FormatDiagnostic {
        &self.diagnostic
    }

    pub fn into_diagnostic(self) -> FormatDiagnostic {
        self.diagnostic
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityResult {
    issue: Option<CompatibilityIssue>,
    evidence_revision: OperationalEvidenceRevision,
}

impl CompatibilityResult {
    pub const fn compatible(evidence_revision: OperationalEvidenceRevision) -> Self {
        Self {
            issue: None,
            evidence_revision,
        }
    }

    pub fn incompatible(
        issue: CompatibilityIssue,
        evidence_revision: OperationalEvidenceRevision,
    ) -> Self {
        Self {
            issue: Some(issue),
            evidence_revision,
        }
    }

    pub fn issue(&self) -> Option<&CompatibilityIssue> {
        self.issue.as_ref()
    }

    pub fn into_issue(self) -> Option<CompatibilityIssue> {
        self.issue
    }

    pub const fn evidence_revision(&self) -> &OperationalEvidenceRevision {
        &self.evidence_revision
    }
}

pub trait CompatibilityPort: Send + Sync {
    fn inspect(
        &self,
        request: &CompatibilityRequest,
    ) -> Result<CompatibilityResult, SourceAdapterError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceCompatibilityEvidence {
    Compatible,
    AlternateFamily,
    Ambiguous,
    UnsupportedDeclaration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceCompatibilityRequest {
    evidence: SourceCompatibilityEvidence,
}

impl SourceCompatibilityRequest {
    pub const fn new(evidence: SourceCompatibilityEvidence) -> Self {
        Self { evidence }
    }

    pub const fn evidence(&self) -> SourceCompatibilityEvidence {
        self.evidence
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCompatibilityResult {
    diagnostic: Option<FormatDiagnostic>,
}

impl SourceCompatibilityResult {
    pub const fn compatible() -> Self {
        Self { diagnostic: None }
    }

    pub fn incompatible(diagnostic: FormatDiagnostic) -> Self {
        Self {
            diagnostic: Some(diagnostic),
        }
    }

    pub fn into_diagnostic(self) -> Option<FormatDiagnostic> {
        self.diagnostic
    }
}

pub trait SourceCompatibilityPort: Send + Sync {
    fn inspect_source(
        &self,
        request: &SourceCompatibilityRequest,
    ) -> Result<SourceCompatibilityResult, SourceAdapterError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorabilityRequirement {
    Editable,
    Removed,
}

#[derive(Debug, Clone)]
pub struct AuthorabilityRequest {
    session: OperationalSourceSession,
    requirement: AuthorabilityRequirement,
}

impl AuthorabilityRequest {
    pub const fn new(
        session: OperationalSourceSession,
        requirement: AuthorabilityRequirement,
    ) -> Self {
        Self {
            session,
            requirement,
        }
    }

    pub fn session(&self) -> &OperationalSourceSession {
        &self.session
    }

    pub const fn requirement(&self) -> AuthorabilityRequirement {
        self.requirement
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorabilityDenial {
    authorability: Authorability,
    summary: SupportSummary,
    diagnostic: FormatDiagnostic,
    evidence_revision: Option<OperationalEvidenceRevision>,
}

impl AuthorabilityDenial {
    fn new(
        authorability: Authorability,
        summary: SupportSummary,
        diagnostic: FormatDiagnostic,
        evidence_revision: Option<OperationalEvidenceRevision>,
    ) -> Result<Self, OperationalContractError> {
        let requirement_specific_denial = authorability == Authorability::Authorable
            && diagnostic.code() == FormatDiagnosticCode::SupportRemovalRequired;
        let unverified_source_unreadable = authorability == Authorability::UnknownSupportState
            && summary.state() == SupportState::Unreadable
            && summary.editing_enabled().is_none()
            && summary.vendor_count() == 0
            && summary.rule_counts() == [0; 3]
            && diagnostic.code() == FormatDiagnosticCode::SupportStateUnreadable
            && matches!(
                diagnostic.detail(),
                FormatDiagnosticDetail::Support(SupportState::Unreadable)
            );
        if (authorability == Authorability::Authorable && !requirement_specific_denial)
            || !matches!(
                diagnostic.detail(),
                FormatDiagnosticDetail::Support(state) if state == summary.state()
            )
            || (authorability == Authorability::UnknownSupportState
                && summary.state() != SupportState::Unreadable)
            || (evidence_revision.is_none() && !unverified_source_unreadable)
        {
            return Err(OperationalContractError::InvalidStateCombination);
        }
        Ok(Self {
            authorability,
            summary,
            diagnostic,
            evidence_revision,
        })
    }

    pub const fn authorability(&self) -> Authorability {
        self.authorability
    }

    pub const fn summary(&self) -> &SupportSummary {
        &self.summary
    }

    pub fn diagnostic(&self) -> &FormatDiagnostic {
        &self.diagnostic
    }

    pub const fn evidence_revision(&self) -> Option<&OperationalEvidenceRevision> {
        self.evidence_revision.as_ref()
    }
}

impl<'de> Deserialize<'de> for AuthorabilityDenial {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            authorability: Authorability,
            summary: SupportSummary,
            diagnostic: FormatDiagnostic,
            evidence_revision: Option<OperationalEvidenceRevision>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.authorability,
            wire.summary,
            wire.diagnostic,
            wire.evidence_revision,
        )
        .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportSummary {
    state: SupportState,
    editing_enabled: Option<bool>,
    vendor_count: usize,
    rule_counts: [usize; 3],
}

impl SupportSummary {
    pub fn new(
        state: SupportState,
        editing_enabled: Option<bool>,
        vendor_count: usize,
        rule_counts: [usize; 3],
    ) -> Result<Self, OperationalContractError> {
        let empty = vendor_count == 0 && rule_counts == [0; 3];
        if matches!(state, SupportState::Absent | SupportState::Unreadable)
            && (editing_enabled.is_some() || !empty)
        {
            return Err(OperationalContractError::InvalidStateCombination);
        }
        Ok(Self {
            state,
            editing_enabled,
            vendor_count,
            rule_counts,
        })
    }

    pub const fn state(&self) -> SupportState {
        self.state
    }

    pub const fn editing_enabled(&self) -> Option<bool> {
        self.editing_enabled
    }

    pub const fn vendor_count(&self) -> usize {
        self.vendor_count
    }

    pub const fn rule_counts(&self) -> [usize; 3] {
        self.rule_counts
    }
}

impl<'de> Deserialize<'de> for SupportSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            state: SupportState,
            editing_enabled: Option<bool>,
            vendor_count: usize,
            rule_counts: [usize; 3],
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.state,
            wire.editing_enabled,
            wire.vendor_count,
            wire.rule_counts,
        )
        .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorabilityEvidence {
    summary: SupportSummary,
    evidence_revision: OperationalEvidenceRevision,
}

impl AuthorabilityEvidence {
    pub const fn summary(&self) -> &SupportSummary {
        &self.summary
    }

    pub const fn evidence_revision(&self) -> &OperationalEvidenceRevision {
        &self.evidence_revision
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorabilityResult {
    Allowed(AuthorabilityEvidence),
    Denied(AuthorabilityDenial),
}

impl AuthorabilityResult {
    pub fn allowed(
        summary: SupportSummary,
        evidence_revision: OperationalEvidenceRevision,
    ) -> Result<Self, OperationalContractError> {
        if matches!(
            summary.state(),
            SupportState::Locked
                | SupportState::ConfigurationReadOnly
                | SupportState::UnknownReadOnly
                | SupportState::Unreadable
        ) {
            return Err(OperationalContractError::InvalidStateCombination);
        }
        Ok(Self::Allowed(AuthorabilityEvidence {
            summary,
            evidence_revision,
        }))
    }

    pub fn denied(
        authorability: Authorability,
        summary: SupportSummary,
        diagnostic: FormatDiagnostic,
        evidence_revision: OperationalEvidenceRevision,
    ) -> Result<Self, OperationalContractError> {
        AuthorabilityDenial::new(authorability, summary, diagnostic, Some(evidence_revision))
            .map(Self::Denied)
    }

    pub fn source_unreadable() -> Self {
        Self::Denied(
            AuthorabilityDenial::new(
                Authorability::UnknownSupportState,
                SupportSummary::new(SupportState::Unreadable, None, 0, [0; 3])
                    .expect("closed unreadable support summary is valid"),
                FormatDiagnostic::new(
                    FormatDiagnosticCode::SupportStateUnreadable,
                    FormatDiagnosticDetail::Support(SupportState::Unreadable),
                )
                .expect("closed unreadable support diagnostic is valid"),
                None,
            )
            .expect("closed source-unreadable denial is valid"),
        )
    }

    pub const fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed(_))
    }

    pub const fn authorability(&self) -> Authorability {
        match self {
            Self::Allowed(_) => Authorability::Authorable,
            Self::Denied(denial) => denial.authorability(),
        }
    }

    pub fn summary(&self) -> &SupportSummary {
        match self {
            Self::Allowed(evidence) => evidence.summary(),
            Self::Denied(denial) => denial.summary(),
        }
    }

    pub const fn evidence_revision(&self) -> Option<&OperationalEvidenceRevision> {
        match self {
            Self::Allowed(evidence) => Some(evidence.evidence_revision()),
            Self::Denied(denial) => denial.evidence_revision(),
        }
    }

    pub const fn denial(&self) -> Option<&AuthorabilityDenial> {
        match self {
            Self::Allowed(_) => None,
            Self::Denied(denial) => Some(denial),
        }
    }

    pub fn into_denial(self) -> Option<AuthorabilityDenial> {
        match self {
            Self::Allowed(_) => None,
            Self::Denied(denial) => Some(denial),
        }
    }
}

impl Serialize for AuthorabilityResult {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;

        match self {
            Self::Allowed(evidence) => {
                let mut state = serializer.serialize_struct("AuthorabilityResult", 4)?;
                state.serialize_field("decision", "allowed")?;
                state.serialize_field("authorability", &Authorability::Authorable)?;
                state.serialize_field("summary", evidence.summary())?;
                state.serialize_field("evidenceRevision", evidence.evidence_revision())?;
                state.end()
            }
            Self::Denied(denial) => {
                let mut state = serializer.serialize_struct("AuthorabilityResult", 5)?;
                state.serialize_field("decision", "denied")?;
                state.serialize_field("authorability", &denial.authorability())?;
                state.serialize_field("summary", denial.summary())?;
                state.serialize_field("diagnostic", denial.diagnostic())?;
                state.serialize_field("evidenceRevision", &denial.evidence_revision())?;
                state.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for AuthorabilityResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "decision", rename_all = "camelCase", deny_unknown_fields)]
        enum Wire {
            Allowed {
                authorability: Authorability,
                summary: SupportSummary,
                #[serde(rename = "evidenceRevision")]
                evidence_revision: OperationalEvidenceRevision,
            },
            Denied {
                authorability: Authorability,
                summary: SupportSummary,
                diagnostic: FormatDiagnostic,
                #[serde(rename = "evidenceRevision")]
                evidence_revision: Option<OperationalEvidenceRevision>,
            },
        }

        match Wire::deserialize(deserializer)? {
            Wire::Allowed {
                authorability: Authorability::Authorable,
                summary,
                evidence_revision,
            } => Self::allowed(summary, evidence_revision).map_err(D::Error::custom),
            Wire::Allowed { .. } => Err(D::Error::custom(
                OperationalContractError::InvalidStateCombination,
            )),
            Wire::Denied {
                authorability,
                summary,
                diagnostic,
                evidence_revision,
            } => AuthorabilityDenial::new(authorability, summary, diagnostic, evidence_revision)
                .map(Self::Denied)
                .map_err(D::Error::custom),
        }
    }
}

pub trait AuthorabilityPort: Send + Sync {
    fn inspect(
        &self,
        request: &AuthorabilityRequest,
    ) -> Result<AuthorabilityResult, SourceAdapterError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationOwnerKind {
    Aggregate,
    Extension,
    Standalone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValidationCoverage {
    Complete,
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValidationRelationCoverage {
    NotApplicable,
    NotEvaluated,
    Partial,
    CompletePresent,
    CompleteMissing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationContext {
    owner_kind: ValidationOwnerKind,
    language_codes: Vec<String>,
    command_text_validation_required: bool,
    references_present: Option<bool>,
    registrar_coverage: ValidationRelationCoverage,
    method_reference_status: Option<ValidationMethodReferenceStatus>,
}

impl ValidationContext {
    pub fn new(
        owner_kind: ValidationOwnerKind,
        language_codes: Vec<String>,
        command_text_validation_required: bool,
        references_present: Option<bool>,
        registrar_coverage: ValidationRelationCoverage,
        method_reference_status: Option<ValidationMethodReferenceStatus>,
    ) -> Result<Self, OperationalContractError> {
        if language_codes.iter().any(|code| {
            code.is_empty()
                || code.len() > 32
                || !code.chars().all(|character| {
                    character.is_ascii_alphanumeric() || character == '-' || character == '_'
                })
        }) {
            return Err(OperationalContractError::InvalidSemanticValue);
        }
        Ok(Self {
            owner_kind,
            language_codes,
            command_text_validation_required,
            references_present,
            registrar_coverage,
            method_reference_status,
        })
    }

    pub const fn owner_kind(&self) -> ValidationOwnerKind {
        self.owner_kind
    }

    pub fn language_codes(&self) -> &[String] {
        &self.language_codes
    }

    pub const fn command_text_validation_required(&self) -> bool {
        self.command_text_validation_required
    }

    pub const fn references_present(&self) -> Option<bool> {
        self.references_present
    }

    pub const fn registrar_coverage(&self) -> ValidationRelationCoverage {
        self.registrar_coverage
    }

    pub const fn method_reference_status(&self) -> Option<ValidationMethodReferenceStatus> {
        self.method_reference_status
    }
}

#[derive(Debug, Clone)]
pub struct ValidationContextRequest {
    session: OperationalSourceSession,
}

impl ValidationContextRequest {
    pub const fn new(session: OperationalSourceSession) -> Self {
        Self { session }
    }

    pub fn session(&self) -> &OperationalSourceSession {
        &self.session
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationContextResult {
    context: Option<ValidationContext>,
    diagnostics: Vec<FormatDiagnostic>,
    evidence_revision: OperationalEvidenceRevision,
}

impl ValidationContextResult {
    pub fn valid(
        context: ValidationContext,
        evidence_revision: OperationalEvidenceRevision,
    ) -> Self {
        Self {
            context: Some(context),
            diagnostics: Vec::new(),
            evidence_revision,
        }
    }

    pub fn invalid(
        diagnostics: Vec<FormatDiagnostic>,
        evidence_revision: OperationalEvidenceRevision,
    ) -> Result<Self, OperationalContractError> {
        if diagnostics.is_empty() {
            return Err(OperationalContractError::EmptyDiagnostics);
        }
        Ok(Self {
            context: None,
            diagnostics,
            evidence_revision,
        })
    }

    pub fn context(&self) -> Option<&ValidationContext> {
        self.context.as_ref()
    }

    pub fn into_context(self) -> Option<ValidationContext> {
        self.context
    }

    pub fn diagnostics(&self) -> &[FormatDiagnostic] {
        &self.diagnostics
    }

    pub const fn evidence_revision(&self) -> &OperationalEvidenceRevision {
        &self.evidence_revision
    }
}

pub trait ValidationContextPort: Send + Sync {
    fn inspect(
        &self,
        request: &ValidationContextRequest,
    ) -> Result<ValidationContextResult, SourceAdapterError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValidationFindingSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValidationFindingCode {
    SourceUnreadable,
    SourceMalformed,
    RevisionUnsupported,
    SemanticStructureInvalid,
    SemanticValueInvalid,
    IdentityMissing,
    IdentityInvalid,
    NameMissing,
    RegistrationMissing,
    LanguageProfileMissing,
    ReferenceMissing,
    RegistrarMissing,
    MethodReferenceInvalid,
    DuplicateSemanticItem,
    CommandPresentationTooLong,
    UnsupportedCombination,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationFinding {
    severity: ValidationFindingSeverity,
    code: ValidationFindingCode,
}

impl ValidationFinding {
    pub const fn new(severity: ValidationFindingSeverity, code: ValidationFindingCode) -> Self {
        Self { severity, code }
    }

    pub const fn severity(&self) -> ValidationFindingSeverity {
        self.severity
    }

    pub const fn code(&self) -> ValidationFindingCode {
        self.code
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct SemanticArtifactId(String);

impl SemanticArtifactId {
    pub fn new(value: impl Into<String>) -> Result<Self, OperationalContractError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, ':' | '-' | '_')
            })
        {
            return Err(OperationalContractError::InvalidSemanticValue);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for SemanticArtifactId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValidationStatus {
    Valid,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationReport {
    subject: SemanticArtifactId,
    status: ValidationStatus,
    coverage: ValidationCoverage,
    checks: u16,
    findings: Vec<ValidationFinding>,
}

impl ValidationReport {
    pub fn new(
        subject: SemanticArtifactId,
        checks: u16,
        findings: Vec<ValidationFinding>,
    ) -> Result<Self, OperationalContractError> {
        Self::new_with_coverage(subject, checks, findings, ValidationCoverage::Complete)
    }

    pub fn new_with_coverage(
        subject: SemanticArtifactId,
        checks: u16,
        findings: Vec<ValidationFinding>,
        coverage: ValidationCoverage,
    ) -> Result<Self, OperationalContractError> {
        if checks == 0
            || findings.len() > usize::from(checks)
            || findings.len() > usize::from(u16::MAX)
            || (coverage != ValidationCoverage::Complete
                && findings
                    .iter()
                    .any(|finding| finding.code == ValidationFindingCode::RegistrarMissing))
        {
            return Err(OperationalContractError::InvalidStateCombination);
        }
        let status = if findings
            .iter()
            .any(|finding| finding.severity == ValidationFindingSeverity::Error)
        {
            ValidationStatus::Invalid
        } else {
            ValidationStatus::Valid
        };
        Ok(Self {
            subject,
            status,
            coverage,
            checks,
            findings,
        })
    }

    pub fn subject(&self) -> &SemanticArtifactId {
        &self.subject
    }

    pub const fn status(&self) -> ValidationStatus {
        self.status
    }

    pub const fn coverage(&self) -> ValidationCoverage {
        self.coverage
    }

    pub const fn checks(&self) -> u16 {
        self.checks
    }

    pub fn findings(&self) -> &[ValidationFinding] {
        &self.findings
    }
}

impl<'de> Deserialize<'de> for ValidationReport {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            subject: SemanticArtifactId,
            status: ValidationStatus,
            coverage: ValidationCoverage,
            checks: u16,
            findings: Vec<ValidationFinding>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let report =
            Self::new_with_coverage(wire.subject, wire.checks, wire.findings, wire.coverage)
                .map_err(D::Error::custom)?;
        if report.status != wire.status {
            return Err(D::Error::custom(
                OperationalContractError::InvalidStateCombination,
            ));
        }
        Ok(report)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidationOptions {
    detailed: bool,
    max_findings: u16,
}

impl ValidationOptions {
    pub fn new(detailed: bool, max_findings: u16) -> Result<Self, OperationalContractError> {
        if max_findings == 0 || max_findings > 1_000 {
            return Err(OperationalContractError::InvalidSemanticValue);
        }
        Ok(Self {
            detailed,
            max_findings,
        })
    }

    pub const fn detailed(self) -> bool {
        self.detailed
    }

    pub const fn max_findings(self) -> u16 {
        self.max_findings
    }
}

#[derive(Debug, Clone)]
pub struct OperationalValidationRequest {
    sessions: Vec<OperationalSourceSession>,
    options: ValidationOptions,
}

impl OperationalValidationRequest {
    pub fn new(
        sessions: Vec<OperationalSourceSession>,
        options: ValidationOptions,
    ) -> Result<Self, OperationalContractError> {
        if sessions.is_empty() {
            return Err(OperationalContractError::EmptyRequest);
        }
        Ok(Self { sessions, options })
    }

    pub fn sessions(&self) -> &[OperationalSourceSession] {
        &self.sessions
    }

    pub const fn options(&self) -> ValidationOptions {
        self.options
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationalValidationResult {
    reports: Vec<ValidationReport>,
    evidence_revision: OperationalEvidenceRevision,
}

impl OperationalValidationResult {
    pub fn new(
        reports: Vec<ValidationReport>,
        evidence_revision: OperationalEvidenceRevision,
    ) -> Result<Self, OperationalContractError> {
        if reports.is_empty() {
            return Err(OperationalContractError::EmptyRequest);
        }
        Ok(Self {
            reports,
            evidence_revision,
        })
    }

    pub fn reports(&self) -> &[ValidationReport] {
        &self.reports
    }

    pub const fn evidence_revision(&self) -> &OperationalEvidenceRevision {
        &self.evidence_revision
    }
}

pub trait OperationalValidationPort: Send + Sync {
    fn validate(
        &self,
        request: &OperationalValidationRequest,
    ) -> Result<OperationalValidationResult, SourceAdapterError>;
}

#[derive(Debug, Clone, Default)]
pub struct OperationCancellation(Arc<AtomicBool>);

impl OperationCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicationInvocation {
    BuildDump,
    RuntimeExecute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicationStatus {
    Published,
    DryRun,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicationCancellation {
    NotRequested,
    BeforeExecution,
    DuringExecution,
    BeforePublication,
    DuringPublication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicationRollback {
    NotNeeded,
    Performed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicationCleanup {
    Completed,
    Failed,
    RetainedForRecovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicationRecovery {
    NotRequired,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicationChange {
    FullSourceReplaced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicationArtifact {
    PublishedSource,
    RecoveryState,
}

#[derive(Debug, Clone)]
pub struct PublicationRequest {
    session: OperationalSourceSession,
    invocation: PublicationInvocation,
    cancellation: OperationCancellation,
}

impl PublicationRequest {
    pub const fn new(
        session: OperationalSourceSession,
        invocation: PublicationInvocation,
        cancellation: OperationCancellation,
    ) -> Self {
        Self {
            session,
            invocation,
            cancellation,
        }
    }

    pub fn session(&self) -> &OperationalSourceSession {
        &self.session
    }

    pub const fn invocation(&self) -> PublicationInvocation {
        self.invocation
    }

    pub fn cancellation(&self) -> &OperationCancellation {
        &self.cancellation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicationFailureKind {
    Preparation,
    Execution,
    Publication,
    Cleanup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicationInterruption {
    cancellation: PublicationCancellation,
    rollback: PublicationRollback,
    cleanup: PublicationCleanup,
    recovery: PublicationRecovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicationFailure {
    kind: PublicationFailureKind,
    cancellation: PublicationCancellation,
    rollback: PublicationRollback,
    cleanup: PublicationCleanup,
    recovery: PublicationRecovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationLifecycle {
    Published,
    DryRun,
    Cancelled(PublicationInterruption),
    Failed(PublicationFailure),
}

impl PublicationLifecycle {
    pub const fn published() -> Self {
        Self::Published
    }

    pub const fn dry_run() -> Self {
        Self::DryRun
    }

    pub fn cancelled(
        cancellation: PublicationCancellation,
        rollback: PublicationRollback,
        cleanup: PublicationCleanup,
        recovery: PublicationRecovery,
    ) -> Result<Self, OperationalContractError> {
        let phase_is_valid = match cancellation {
            PublicationCancellation::NotRequested => false,
            PublicationCancellation::BeforeExecution
            | PublicationCancellation::DuringExecution
            | PublicationCancellation::BeforePublication => {
                rollback == PublicationRollback::NotNeeded
            }
            PublicationCancellation::DuringPublication => {
                rollback == PublicationRollback::Performed
            }
        };
        if !phase_is_valid
            || cleanup != PublicationCleanup::Completed
            || recovery != PublicationRecovery::NotRequired
        {
            return Err(OperationalContractError::InvalidStateCombination);
        }
        Ok(Self::Cancelled(PublicationInterruption {
            cancellation,
            rollback,
            cleanup,
            recovery,
        }))
    }

    pub fn failed(
        kind: PublicationFailureKind,
        cancellation: PublicationCancellation,
        rollback: PublicationRollback,
        cleanup: PublicationCleanup,
        recovery: PublicationRecovery,
    ) -> Result<Self, OperationalContractError> {
        if !publication_failure_is_consistent(kind, cancellation, rollback, cleanup, recovery) {
            return Err(OperationalContractError::InvalidStateCombination);
        }
        Ok(Self::Failed(PublicationFailure {
            kind,
            cancellation,
            rollback,
            cleanup,
            recovery,
        }))
    }

    pub const fn status(self) -> PublicationStatus {
        match self {
            Self::Published => PublicationStatus::Published,
            Self::DryRun => PublicationStatus::DryRun,
            Self::Cancelled(_) => PublicationStatus::Cancelled,
            Self::Failed(_) => PublicationStatus::Failed,
        }
    }

    pub const fn is_published(self) -> bool {
        matches!(self, Self::Published)
    }

    pub const fn is_failed(self) -> bool {
        matches!(self, Self::Failed(_))
    }

    pub const fn failure_kind(self) -> Option<PublicationFailureKind> {
        match self {
            Self::Failed(state) => Some(state.kind),
            _ => None,
        }
    }

    pub const fn cancellation(self) -> PublicationCancellation {
        match self {
            Self::Published | Self::DryRun => PublicationCancellation::NotRequested,
            Self::Cancelled(state) => state.cancellation,
            Self::Failed(state) => state.cancellation,
        }
    }

    pub const fn rollback(self) -> PublicationRollback {
        match self {
            Self::Published | Self::DryRun => PublicationRollback::NotNeeded,
            Self::Cancelled(state) => state.rollback,
            Self::Failed(state) => state.rollback,
        }
    }

    pub const fn cleanup(self) -> PublicationCleanup {
        match self {
            Self::Published | Self::DryRun => PublicationCleanup::Completed,
            Self::Cancelled(state) => state.cleanup,
            Self::Failed(state) => state.cleanup,
        }
    }

    pub const fn recovery(self) -> PublicationRecovery {
        match self {
            Self::Published | Self::DryRun => PublicationRecovery::NotRequired,
            Self::Cancelled(state) => state.recovery,
            Self::Failed(state) => state.recovery,
        }
    }
}

const fn publication_recovery_is_consistent(
    rollback: PublicationRollback,
    cleanup: PublicationCleanup,
    recovery: PublicationRecovery,
) -> bool {
    let requires_recovery = matches!(rollback, PublicationRollback::Failed)
        || matches!(
            cleanup,
            PublicationCleanup::Failed | PublicationCleanup::RetainedForRecovery
        );
    requires_recovery == matches!(recovery, PublicationRecovery::Required)
}

const fn publication_failure_is_consistent(
    kind: PublicationFailureKind,
    cancellation: PublicationCancellation,
    rollback: PublicationRollback,
    cleanup: PublicationCleanup,
    recovery: PublicationRecovery,
) -> bool {
    if !publication_recovery_is_consistent(rollback, cleanup, recovery)
        || matches!(cancellation, PublicationCancellation::BeforeExecution)
    {
        return false;
    }
    match kind {
        PublicationFailureKind::Preparation | PublicationFailureKind::Execution => {
            matches!(cancellation, PublicationCancellation::NotRequested)
                && matches!(rollback, PublicationRollback::NotNeeded)
                && matches!(cleanup, PublicationCleanup::Completed)
                && matches!(recovery, PublicationRecovery::NotRequired)
        }
        PublicationFailureKind::Publication => {
            !matches!(cancellation, PublicationCancellation::DuringExecution)
                && !matches!(cleanup, PublicationCleanup::Failed)
                && match cancellation {
                    PublicationCancellation::BeforePublication => {
                        matches!(rollback, PublicationRollback::NotNeeded)
                    }
                    PublicationCancellation::DuringPublication => {
                        !matches!(rollback, PublicationRollback::NotNeeded)
                    }
                    PublicationCancellation::NotRequested => true,
                    PublicationCancellation::BeforeExecution
                    | PublicationCancellation::DuringExecution => false,
                }
        }
        PublicationFailureKind::Cleanup => {
            matches!(cleanup, PublicationCleanup::Failed)
                && matches!(recovery, PublicationRecovery::Required)
                && match cancellation {
                    PublicationCancellation::BeforePublication
                    | PublicationCancellation::DuringExecution => {
                        matches!(rollback, PublicationRollback::NotNeeded)
                    }
                    PublicationCancellation::DuringPublication => {
                        !matches!(rollback, PublicationRollback::NotNeeded)
                    }
                    PublicationCancellation::NotRequested => true,
                    PublicationCancellation::BeforeExecution => false,
                }
        }
    }
}

impl Serialize for PublicationLifecycle {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;

        match *self {
            Self::Published | Self::DryRun => {
                let mut state = serializer.serialize_struct("PublicationLifecycle", 1)?;
                state.serialize_field(
                    "state",
                    if matches!(self, Self::Published) {
                        "published"
                    } else {
                        "dryRun"
                    },
                )?;
                state.end()
            }
            Self::Cancelled(interruption) => {
                let mut state = serializer.serialize_struct("PublicationLifecycle", 5)?;
                state.serialize_field("state", "cancelled")?;
                state.serialize_field("cancellation", &interruption.cancellation)?;
                state.serialize_field("rollback", &interruption.rollback)?;
                state.serialize_field("cleanup", &interruption.cleanup)?;
                state.serialize_field("recovery", &interruption.recovery)?;
                state.end()
            }
            Self::Failed(failure) => {
                let mut state = serializer.serialize_struct("PublicationLifecycle", 6)?;
                state.serialize_field("state", "failed")?;
                state.serialize_field("failure", &failure.kind)?;
                state.serialize_field("cancellation", &failure.cancellation)?;
                state.serialize_field("rollback", &failure.rollback)?;
                state.serialize_field("cleanup", &failure.cleanup)?;
                state.serialize_field("recovery", &failure.recovery)?;
                state.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for PublicationLifecycle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        enum State {
            Published,
            DryRun,
            Cancelled,
            Failed,
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            state: State,
            failure: Option<PublicationFailureKind>,
            cancellation: Option<PublicationCancellation>,
            rollback: Option<PublicationRollback>,
            cleanup: Option<PublicationCleanup>,
            recovery: Option<PublicationRecovery>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let no_detail = wire.failure.is_none()
            && wire.cancellation.is_none()
            && wire.rollback.is_none()
            && wire.cleanup.is_none()
            && wire.recovery.is_none();
        match wire.state {
            State::Published if no_detail => Ok(Self::published()),
            State::DryRun if no_detail => Ok(Self::dry_run()),
            State::Cancelled if wire.failure.is_none() => Self::cancelled(
                wire.cancellation
                    .ok_or_else(|| D::Error::custom("missing cancellation"))?,
                wire.rollback
                    .ok_or_else(|| D::Error::custom("missing rollback"))?,
                wire.cleanup
                    .ok_or_else(|| D::Error::custom("missing cleanup"))?,
                wire.recovery
                    .ok_or_else(|| D::Error::custom("missing recovery"))?,
            )
            .map_err(D::Error::custom),
            State::Failed => Self::failed(
                wire.failure
                    .ok_or_else(|| D::Error::custom("missing failure"))?,
                wire.cancellation
                    .ok_or_else(|| D::Error::custom("missing cancellation"))?,
                wire.rollback
                    .ok_or_else(|| D::Error::custom("missing rollback"))?,
                wire.cleanup
                    .ok_or_else(|| D::Error::custom("missing cleanup"))?,
                wire.recovery
                    .ok_or_else(|| D::Error::custom("missing recovery"))?,
            )
            .map_err(D::Error::custom),
            _ => Err(D::Error::custom(
                OperationalContractError::InvalidStateCombination,
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicationResult {
    lifecycle: PublicationLifecycle,
    diagnostics: Vec<FormatDiagnostic>,
    changes: Vec<PublicationChange>,
    artifacts: Vec<PublicationArtifact>,
}

impl PublicationResult {
    pub fn new(
        lifecycle: PublicationLifecycle,
        diagnostics: Vec<FormatDiagnostic>,
        changes: Vec<PublicationChange>,
        artifacts: Vec<PublicationArtifact>,
    ) -> Result<Self, OperationalContractError> {
        let expected_diagnostics = publication_diagnostic_codes(lifecycle);
        let diagnostics_are_exact = diagnostics.len() == expected_diagnostics.len()
            && expected_diagnostics.iter().all(|expected| {
                diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.code() == *expected)
                    .count()
                    == 1
            });
        let valid = diagnostics_are_exact
            && match lifecycle {
                PublicationLifecycle::Published | PublicationLifecycle::DryRun => {
                    diagnostics.is_empty()
                }
                PublicationLifecycle::Cancelled(_) | PublicationLifecycle::Failed(_) => {
                    diagnostics.iter().all(|diagnostic| {
                        matches!(diagnostic.detail(), FormatDiagnosticDetail::Publication(_))
                    })
                }
            }
            && match lifecycle {
                PublicationLifecycle::Published => {
                    changes == [PublicationChange::FullSourceReplaced]
                        && artifacts == [PublicationArtifact::PublishedSource]
                }
                PublicationLifecycle::DryRun | PublicationLifecycle::Cancelled(_) => {
                    changes.is_empty() && artifacts.is_empty()
                }
                PublicationLifecycle::Failed(_) => {
                    changes.is_empty()
                        && if lifecycle.recovery() == PublicationRecovery::Required {
                            artifacts == [PublicationArtifact::RecoveryState]
                        } else {
                            artifacts.is_empty()
                        }
                }
            };
        if !valid {
            return Err(OperationalContractError::InvalidStateCombination);
        }
        Ok(Self {
            lifecycle,
            diagnostics,
            changes,
            artifacts,
        })
    }

    pub const fn lifecycle(&self) -> PublicationLifecycle {
        self.lifecycle
    }

    pub const fn status(&self) -> PublicationStatus {
        self.lifecycle.status()
    }

    pub const fn cancellation(&self) -> PublicationCancellation {
        self.lifecycle.cancellation()
    }

    pub const fn rollback(&self) -> PublicationRollback {
        self.lifecycle.rollback()
    }

    pub const fn cleanup(&self) -> PublicationCleanup {
        self.lifecycle.cleanup()
    }

    pub const fn recovery(&self) -> PublicationRecovery {
        self.lifecycle.recovery()
    }

    pub fn diagnostics(&self) -> &[FormatDiagnostic] {
        &self.diagnostics
    }

    pub fn changes(&self) -> &[PublicationChange] {
        &self.changes
    }

    pub fn artifacts(&self) -> &[PublicationArtifact] {
        &self.artifacts
    }
}

fn publication_diagnostic_codes(lifecycle: PublicationLifecycle) -> Vec<FormatDiagnosticCode> {
    match lifecycle {
        PublicationLifecycle::Published | PublicationLifecycle::DryRun => Vec::new(),
        PublicationLifecycle::Cancelled(_) => {
            vec![FormatDiagnosticCode::PublicationCancelled]
        }
        PublicationLifecycle::Failed(_) => {
            let mut codes = vec![FormatDiagnosticCode::PublicationFailed];
            if lifecycle.cancellation() != PublicationCancellation::NotRequested {
                codes.push(FormatDiagnosticCode::PublicationCancelled);
            }
            if lifecycle.cleanup() == PublicationCleanup::Failed {
                codes.push(FormatDiagnosticCode::PublicationCleanupFailed);
            }
            if lifecycle.recovery() == PublicationRecovery::Required {
                codes.push(FormatDiagnosticCode::PublicationRecoveryRequired);
            }
            codes
        }
    }
}

impl<'de> Deserialize<'de> for PublicationResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            lifecycle: PublicationLifecycle,
            diagnostics: Vec<FormatDiagnostic>,
            changes: Vec<PublicationChange>,
            artifacts: Vec<PublicationArtifact>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.lifecycle,
            wire.diagnostics,
            wire.changes,
            wire.artifacts,
        )
        .map_err(D::Error::custom)
    }
}

pub trait PublicationPort: Send + Sync {
    fn publish(
        &self,
        request: &PublicationRequest,
    ) -> Result<PublicationResult, SourceAdapterError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationalContractError {
    EmptyRequest,
    EmptyDiagnostics,
    InvalidSemanticValue,
    InvalidStateCombination,
    InvalidDiagnostic,
}

impl std::fmt::Display for OperationalContractError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::EmptyRequest => "operational request must contain at least one session",
            Self::EmptyDiagnostics => "invalid operational result requires a diagnostic",
            Self::InvalidSemanticValue => "operational semantic value is invalid",
            Self::InvalidStateCombination => "operational result state combination is invalid",
            Self::InvalidDiagnostic => "diagnostic code and semantic detail do not match",
        })
    }
}

impl std::error::Error for OperationalContractError {}

#[derive(Clone)]
pub struct OperationalAdapterRegistration {
    compatibility: Arc<dyn CompatibilityPort>,
    source_compatibility: Arc<dyn SourceCompatibilityPort>,
    authorability: Arc<dyn AuthorabilityPort>,
    object_kinds: Arc<dyn ObjectKindRegistryPort>,
    semantic_artifacts: Arc<dyn SemanticArtifactPort>,
    validation_context: Arc<dyn ValidationContextPort>,
    validation: Arc<dyn OperationalValidationPort>,
    publication: Arc<dyn PublicationPort>,
}

impl OperationalAdapterRegistration {
    pub fn new(
        compatibility: Arc<dyn CompatibilityPort>,
        source_compatibility: Arc<dyn SourceCompatibilityPort>,
        authorability: Arc<dyn AuthorabilityPort>,
        object_kinds: Arc<dyn ObjectKindRegistryPort>,
        semantic_artifacts: Arc<dyn SemanticArtifactPort>,
        validation_context: Arc<dyn ValidationContextPort>,
        validation: Arc<dyn OperationalValidationPort>,
        publication: Arc<dyn PublicationPort>,
    ) -> Self {
        Self {
            compatibility,
            source_compatibility,
            authorability,
            object_kinds,
            semantic_artifacts,
            validation_context,
            validation,
            publication,
        }
    }

    pub fn compatibility(&self) -> &dyn CompatibilityPort {
        self.compatibility.as_ref()
    }

    pub fn source_compatibility(&self) -> &dyn SourceCompatibilityPort {
        self.source_compatibility.as_ref()
    }

    pub fn authorability(&self) -> &dyn AuthorabilityPort {
        self.authorability.as_ref()
    }

    pub fn object_kinds(&self) -> &dyn ObjectKindRegistryPort {
        self.object_kinds.as_ref()
    }

    pub fn semantic_artifacts(&self) -> &dyn SemanticArtifactPort {
        self.semantic_artifacts.as_ref()
    }

    pub fn validation_context(&self) -> &dyn ValidationContextPort {
        self.validation_context.as_ref()
    }

    pub fn validation(&self) -> &dyn OperationalValidationPort {
        self.validation.as_ref()
    }

    pub fn publication(&self) -> &dyn PublicationPort {
        self.publication.as_ref()
    }
}
