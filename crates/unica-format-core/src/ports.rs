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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatibilityIssueKind {
    Older,
    Newer,
    Malformed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportState {
    Absent,
    Removed,
    Editable,
    Locked,
    ConfigurationReadOnly,
    UnknownReadOnly,
    Unreadable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationIssueKind {
    SourceUnreadable,
    OwnerUnavailable,
    RegistrationMissing,
    LanguageProfileMissing,
    ReferenceMissing,
    RegistrarMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationMethodReferenceStatus {
    Valid,
    Invalid,
    TargetMissing,
    ImplementationMissing,
    EntryPointMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatDiagnosticDetail {
    Compatibility(CompatibilityIssueKind),
    Support(SupportState),
    Validation(ValidationIssueKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatDiagnostic {
    code: FormatDiagnosticCode,
    message: String,
    details: Vec<FormatDiagnosticDetail>,
}

impl FormatDiagnostic {
    pub fn new(code: FormatDiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: Vec::new(),
        }
    }

    pub fn with_detail(mut self, detail: FormatDiagnosticDetail) -> Self {
        if !self.details.contains(&detail) {
            self.details.push(detail);
        }
        self
    }

    pub const fn code(&self) -> FormatDiagnosticCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn details(&self) -> &[FormatDiagnosticDetail] {
        &self.details
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
}

impl CompatibilityResult {
    pub const fn compatible() -> Self {
        Self { issue: None }
    }

    pub fn incompatible(issue: CompatibilityIssue) -> Self {
        Self { issue: Some(issue) }
    }

    pub fn issue(&self) -> Option<&CompatibilityIssue> {
        self.issue.as_ref()
    }

    pub fn into_issue(self) -> Option<CompatibilityIssue> {
        self.issue
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorabilityViolation {
    diagnostic: FormatDiagnostic,
}

impl AuthorabilityViolation {
    pub const fn new(diagnostic: FormatDiagnostic) -> Self {
        Self { diagnostic }
    }

    pub fn diagnostic(&self) -> &FormatDiagnostic {
        &self.diagnostic
    }

    pub fn into_diagnostic(self) -> FormatDiagnostic {
        self.diagnostic
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportSummary {
    state: SupportState,
    editing_enabled: Option<bool>,
    vendor_count: usize,
    rule_counts: [usize; 3],
}

impl SupportSummary {
    pub const fn new(
        state: SupportState,
        editing_enabled: Option<bool>,
        vendor_count: usize,
        rule_counts: [usize; 3],
    ) -> Self {
        Self {
            state,
            editing_enabled,
            vendor_count,
            rule_counts,
        }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorabilityResult {
    authorability: Authorability,
    summary: SupportSummary,
    violation: Option<AuthorabilityViolation>,
}

impl AuthorabilityResult {
    pub const fn new(
        authorability: Authorability,
        summary: SupportSummary,
        violation: Option<AuthorabilityViolation>,
    ) -> Self {
        Self {
            authorability,
            summary,
            violation,
        }
    }

    pub const fn authorability(&self) -> Authorability {
        self.authorability
    }

    pub fn summary(&self) -> &SupportSummary {
        &self.summary
    }

    pub fn violation(&self) -> Option<&AuthorabilityViolation> {
        self.violation.as_ref()
    }

    pub fn into_violation(self) -> Option<AuthorabilityViolation> {
        self.violation
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationContext {
    owner_kind: ValidationOwnerKind,
    language_codes: Vec<String>,
    command_text_validation_required: bool,
    references_present: Option<bool>,
    registrar_present: Option<bool>,
    method_reference_status: Option<ValidationMethodReferenceStatus>,
}

impl ValidationContext {
    pub fn new(
        owner_kind: ValidationOwnerKind,
        language_codes: Vec<String>,
        command_text_validation_required: bool,
        references_present: Option<bool>,
        registrar_present: Option<bool>,
        method_reference_status: Option<ValidationMethodReferenceStatus>,
    ) -> Result<Self, OperationalContractError> {
        if language_codes.iter().any(|code| {
            code.is_empty()
                || code.len() > 32
                || !code
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-'
                        || character == '_')
        }) {
            return Err(OperationalContractError::InvalidSemanticValue);
        }
        Ok(Self {
            owner_kind,
            language_codes,
            command_text_validation_required,
            references_present,
            registrar_present,
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

    pub const fn registrar_present(&self) -> Option<bool> {
        self.registrar_present
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
}

impl ValidationContextResult {
    pub fn valid(context: ValidationContext) -> Self {
        Self {
            context: Some(context),
            diagnostics: Vec::new(),
        }
    }

    pub fn invalid(diagnostics: Vec<FormatDiagnostic>) -> Result<Self, OperationalContractError> {
        if diagnostics.is_empty() {
            return Err(OperationalContractError::EmptyDiagnostics);
        }
        Ok(Self {
            context: None,
            diagnostics,
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
}

pub trait ValidationContextPort: Send + Sync {
    fn inspect(
        &self,
        request: &ValidationContextRequest,
    ) -> Result<ValidationContextResult, SourceAdapterError>;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationInvocation {
    BuildDump,
    RuntimeExecute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationStatus {
    Published,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationCancellation {
    NotRequested,
    BeforeExecution,
    DuringExecution,
    BeforePublication,
    DuringPublication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationRollback {
    NotNeeded,
    Performed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationCleanup {
    Completed,
    Failed,
    RetainedForRecovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationRecovery {
    NotRequired,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationChange {
    FullSourceReplaced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationResult {
    status: PublicationStatus,
    cancellation: PublicationCancellation,
    rollback: PublicationRollback,
    cleanup: PublicationCleanup,
    recovery: PublicationRecovery,
    summary: String,
    diagnostics: Vec<FormatDiagnostic>,
    changes: Vec<PublicationChange>,
    artifacts: Vec<PublicationArtifact>,
}

impl PublicationResult {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        status: PublicationStatus,
        cancellation: PublicationCancellation,
        rollback: PublicationRollback,
        cleanup: PublicationCleanup,
        recovery: PublicationRecovery,
        summary: impl Into<String>,
        diagnostics: Vec<FormatDiagnostic>,
        changes: Vec<PublicationChange>,
        artifacts: Vec<PublicationArtifact>,
    ) -> Result<Self, OperationalContractError> {
        let valid = match status {
            PublicationStatus::Published => {
                cancellation == PublicationCancellation::NotRequested
                    && recovery == PublicationRecovery::NotRequired
                    && rollback != PublicationRollback::Failed
                    && cleanup == PublicationCleanup::Completed
                    && diagnostics.is_empty()
            }
            PublicationStatus::Cancelled => {
                cancellation != PublicationCancellation::NotRequested
                    && rollback != PublicationRollback::Failed
            }
            PublicationStatus::Failed => !diagnostics.is_empty(),
        } && (rollback != PublicationRollback::Failed
            || recovery == PublicationRecovery::Required)
            && (cleanup == PublicationCleanup::Completed
                || recovery == PublicationRecovery::Required);
        if !valid {
            return Err(OperationalContractError::InvalidStateCombination);
        }
        Ok(Self {
            status,
            cancellation,
            rollback,
            cleanup,
            recovery,
            summary: summary.into(),
            diagnostics,
            changes,
            artifacts,
        })
    }

    pub const fn status(&self) -> PublicationStatus {
        self.status
    }

    pub const fn cancellation(&self) -> PublicationCancellation {
        self.cancellation
    }

    pub const fn rollback(&self) -> PublicationRollback {
        self.rollback
    }

    pub const fn cleanup(&self) -> PublicationCleanup {
        self.cleanup
    }

    pub const fn recovery(&self) -> PublicationRecovery {
        self.recovery
    }

    pub fn summary(&self) -> &str {
        &self.summary
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
}

impl std::fmt::Display for OperationalContractError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::EmptyRequest => "operational request must contain at least one session",
            Self::EmptyDiagnostics => "invalid operational result requires a diagnostic",
            Self::InvalidSemanticValue => "operational semantic value is invalid",
            Self::InvalidStateCombination => "operational result state combination is invalid",
        })
    }
}

impl std::error::Error for OperationalContractError {}

#[derive(Clone)]
pub struct OperationalAdapterRegistration {
    compatibility: Arc<dyn CompatibilityPort>,
    source_compatibility: Arc<dyn SourceCompatibilityPort>,
    authorability: Arc<dyn AuthorabilityPort>,
    validation_context: Arc<dyn ValidationContextPort>,
    publication: Arc<dyn PublicationPort>,
}

impl OperationalAdapterRegistration {
    pub fn new(
        compatibility: Arc<dyn CompatibilityPort>,
        source_compatibility: Arc<dyn SourceCompatibilityPort>,
        authorability: Arc<dyn AuthorabilityPort>,
        validation_context: Arc<dyn ValidationContextPort>,
        publication: Arc<dyn PublicationPort>,
    ) -> Self {
        Self {
            compatibility,
            source_compatibility,
            authorability,
            validation_context,
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

    pub fn validation_context(&self) -> &dyn ValidationContextPort {
        self.validation_context.as_ref()
    }

    pub fn publication(&self) -> &dyn PublicationPort {
        self.publication.as_ref()
    }
}
