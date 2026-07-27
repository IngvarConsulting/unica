//! Format-neutral ports implemented by concrete source adapters.

use std::{any::Any, ffi::OsString, path::PathBuf, sync::Arc};

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
pub struct AdapterFormatProfile {
    pub platform_line: &'static str,
    pub export_format: &'static str,
    pub legacy_metadata_classes: &'static [&'static str],
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
    pub profile: AdapterFormatProfile,
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
