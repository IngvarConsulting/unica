//! Format-neutral ports implemented by concrete source adapters.

use crate::{
    navigation::{
        CapabilityVector, NavigationEnvelope, NavigationQuery, ObjectRef, PropertyValue,
        SourceAdapterDiagnostic,
    },
    semantic_ids::{SemanticPropertyId, SemanticRelationId},
    source::{SourceAdapterError, SourceContext, SourceDescriptor, SourceRevision, SourceSnapshot},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeResult {
    NoMatch,
    Match(SourceDescriptor),
}

pub trait ProbePort: Send + Sync {
    fn probe(&self, source: &SourceContext) -> Result<ProbeResult, SourceAdapterError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureResult {
    NoMatch,
    Captured(SourceSnapshot),
}

pub trait CapturePort: Send + Sync {
    fn capture(&self, source: &SourceContext) -> Result<CaptureResult, SourceAdapterError>;
}

#[derive(Debug, Clone)]
pub struct FormatReadRequest {
    pub source: SourceContext,
    pub snapshot: SourceSnapshot,
    pub query: NavigationQuery,
}

pub trait ReadPort: Send + Sync {
    fn read(&self, request: &FormatReadRequest) -> Result<NavigationEnvelope, SourceAdapterError>;
}

/// Closed mutation language. Native parser nodes and `serde_json::Value` are
/// deliberately absent from this boundary.
///
/// ```compile_fail
/// use unica_format_core::ports::FormatWriteCommand;
/// let command = FormatWriteCommand::Native(serde_json::json!({"unsafe": true}));
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
