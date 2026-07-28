//! Closed, format-neutral mutation commands and outcomes.
//!
//! Commands contain semantic intent only. Source locations, serialized
//! definitions and adapter-specific compatibility data are captured in opaque
//! operational sessions before a command reaches a port.

use serde::{Deserialize, Serialize};

use crate::{
    ports::{
        OperationalContractError, PublicationCancellation, PublicationCleanup, PublicationRecovery,
        PublicationRollback,
    },
    semantic_ids::SemanticObjectKind,
};

mod inspection;
pub use inspection::*;
mod module_locator;
pub use module_locator::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WriterFamily {
    Configuration,
    Extension,
    ExternalArtifact,
    Metadata,
    Form,
    Template,
    Help,
    Interface,
    Role,
    Subsystem,
    Support,
    DataComposition,
    Spreadsheet,
}

impl WriterFamily {
    pub const ALL: [Self; 13] = [
        Self::Configuration,
        Self::Extension,
        Self::ExternalArtifact,
        Self::Metadata,
        Self::Form,
        Self::Template,
        Self::Help,
        Self::Interface,
        Self::Role,
        Self::Subsystem,
        Self::Support,
        Self::DataComposition,
        Self::Spreadsheet,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationMode {
    Preview,
    Apply,
}

impl MutationMode {
    pub const fn is_preview(self) -> bool {
        matches!(self, Self::Preview)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticValueError {
    Empty,
    ControlCharacter,
    TooLong,
    InvalidCombination,
}

impl std::fmt::Display for SemanticValueError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "semantic value must not be empty",
            Self::ControlCharacter => "semantic value must not contain control characters",
            Self::TooLong => "semantic value is too long",
            Self::InvalidCombination => "semantic command fields form an invalid combination",
        })
    }
}

impl std::error::Error for SemanticValueError {}

fn validate_text(value: String, max: usize) -> Result<String, SemanticValueError> {
    if value.trim().is_empty() {
        return Err(SemanticValueError::Empty);
    }
    if value.chars().any(char::is_control) {
        return Err(SemanticValueError::ControlCharacter);
    }
    if value.chars().count() > max {
        return Err(SemanticValueError::TooLong);
    }
    Ok(value)
}

macro_rules! semantic_text {
    ($name:ident, $max:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, SemanticValueError> {
                validate_text(value.into(), $max).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

mod writer_payloads;
pub use writer_payloads::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "command",
    content = "payload",
    deny_unknown_fields
)]
pub enum WriterCommand {
    ConfigurationInitialize(ConfigurationInitialize),
    ConfigurationEdit(ConfigurationEdit),
    ExtensionInitialize(ExtensionInitialize),
    ExtensionBorrow(ExtensionBorrow),
    ExtensionPatchMethod(ExtensionPatchMethod),
    ExternalProcessorInitialize(ExternalArtifactInitialize),
    ExternalReportInitialize(ExternalArtifactInitialize),
    MetadataCreate(MetadataCreate),
    MetadataEdit(MetadataEdit),
    MetadataRemove(MetadataRemove),
    FormCreate(FormCreate),
    FormCompile(FormCompile),
    FormEdit(FormEdit),
    FormRemove(FormRemove),
    TemplateCreate(TemplateCreate),
    TemplateRemove(TemplateRemove),
    HelpCreate(HelpCreate),
    InterfaceEdit(InterfaceEdit),
    RoleCreate(RoleCreate),
    SubsystemCreate(SubsystemCreate),
    SubsystemEdit(SubsystemEdit),
    SupportEdit(SupportEdit),
    DataCompositionCreate(DataCompositionCreate),
    DataCompositionEdit(DataCompositionEdit),
    SpreadsheetCreate(SpreadsheetCreate),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WriterCommandKind {
    ConfigurationInitialize,
    ConfigurationEdit,
    ExtensionInitialize,
    ExtensionBorrow,
    ExtensionPatchMethod,
    ExternalProcessorInitialize,
    ExternalReportInitialize,
    MetadataCreate,
    MetadataEdit,
    MetadataRemove,
    FormCreate,
    FormCompile,
    FormEdit,
    FormRemove,
    TemplateCreate,
    TemplateRemove,
    HelpCreate,
    InterfaceEdit,
    RoleCreate,
    SubsystemCreate,
    SubsystemEdit,
    SupportEdit,
    DataCompositionCreate,
    DataCompositionEdit,
    SpreadsheetCreate,
}

impl WriterCommandKind {
    pub const ALL: [Self; 25] = [
        Self::ConfigurationInitialize,
        Self::ConfigurationEdit,
        Self::ExtensionInitialize,
        Self::ExtensionBorrow,
        Self::ExtensionPatchMethod,
        Self::ExternalProcessorInitialize,
        Self::ExternalReportInitialize,
        Self::MetadataCreate,
        Self::MetadataEdit,
        Self::MetadataRemove,
        Self::FormCreate,
        Self::FormCompile,
        Self::FormEdit,
        Self::FormRemove,
        Self::TemplateCreate,
        Self::TemplateRemove,
        Self::HelpCreate,
        Self::InterfaceEdit,
        Self::RoleCreate,
        Self::SubsystemCreate,
        Self::SubsystemEdit,
        Self::SupportEdit,
        Self::DataCompositionCreate,
        Self::DataCompositionEdit,
        Self::SpreadsheetCreate,
    ];
}

impl WriterCommand {
    pub const fn kind(&self) -> WriterCommandKind {
        match self {
            Self::ConfigurationInitialize(_) => WriterCommandKind::ConfigurationInitialize,
            Self::ConfigurationEdit(_) => WriterCommandKind::ConfigurationEdit,
            Self::ExtensionInitialize(_) => WriterCommandKind::ExtensionInitialize,
            Self::ExtensionBorrow(_) => WriterCommandKind::ExtensionBorrow,
            Self::ExtensionPatchMethod(_) => WriterCommandKind::ExtensionPatchMethod,
            Self::ExternalProcessorInitialize(_) => WriterCommandKind::ExternalProcessorInitialize,
            Self::ExternalReportInitialize(_) => WriterCommandKind::ExternalReportInitialize,
            Self::MetadataCreate(_) => WriterCommandKind::MetadataCreate,
            Self::MetadataEdit(_) => WriterCommandKind::MetadataEdit,
            Self::MetadataRemove(_) => WriterCommandKind::MetadataRemove,
            Self::FormCreate(_) => WriterCommandKind::FormCreate,
            Self::FormCompile(_) => WriterCommandKind::FormCompile,
            Self::FormEdit(_) => WriterCommandKind::FormEdit,
            Self::FormRemove(_) => WriterCommandKind::FormRemove,
            Self::TemplateCreate(_) => WriterCommandKind::TemplateCreate,
            Self::TemplateRemove(_) => WriterCommandKind::TemplateRemove,
            Self::HelpCreate(_) => WriterCommandKind::HelpCreate,
            Self::InterfaceEdit(_) => WriterCommandKind::InterfaceEdit,
            Self::RoleCreate(_) => WriterCommandKind::RoleCreate,
            Self::SubsystemCreate(_) => WriterCommandKind::SubsystemCreate,
            Self::SubsystemEdit(_) => WriterCommandKind::SubsystemEdit,
            Self::SupportEdit(_) => WriterCommandKind::SupportEdit,
            Self::DataCompositionCreate(_) => WriterCommandKind::DataCompositionCreate,
            Self::DataCompositionEdit(_) => WriterCommandKind::DataCompositionEdit,
            Self::SpreadsheetCreate(_) => WriterCommandKind::SpreadsheetCreate,
        }
    }

    pub const fn family(&self) -> WriterFamily {
        match self {
            Self::ConfigurationInitialize(_) | Self::ConfigurationEdit(_) => {
                WriterFamily::Configuration
            }
            Self::ExtensionInitialize(_)
            | Self::ExtensionBorrow(_)
            | Self::ExtensionPatchMethod(_) => WriterFamily::Extension,
            Self::ExternalProcessorInitialize(_) | Self::ExternalReportInitialize(_) => {
                WriterFamily::ExternalArtifact
            }
            Self::MetadataCreate(_) | Self::MetadataEdit(_) | Self::MetadataRemove(_) => {
                WriterFamily::Metadata
            }
            Self::FormCreate(_)
            | Self::FormCompile(_)
            | Self::FormEdit(_)
            | Self::FormRemove(_) => WriterFamily::Form,
            Self::TemplateCreate(_) | Self::TemplateRemove(_) => WriterFamily::Template,
            Self::HelpCreate(_) => WriterFamily::Help,
            Self::InterfaceEdit(_) => WriterFamily::Interface,
            Self::RoleCreate(_) => WriterFamily::Role,
            Self::SubsystemCreate(_) | Self::SubsystemEdit(_) => WriterFamily::Subsystem,
            Self::SupportEdit(_) => WriterFamily::Support,
            Self::DataCompositionCreate(_) | Self::DataCompositionEdit(_) => {
                WriterFamily::DataComposition
            }
            Self::SpreadsheetCreate(_) => WriterFamily::Spreadsheet,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WriterFailureKind {
    InvalidRequest,
    UnsupportedState,
    GuardRejected,
    Conflict,
    Validation,
    Planning,
    Publication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriterInterruption {
    cancellation: PublicationCancellation,
    rollback: PublicationRollback,
    cleanup: PublicationCleanup,
    recovery: PublicationRecovery,
}

impl WriterInterruption {
    pub fn new(
        cancellation: PublicationCancellation,
        rollback: PublicationRollback,
        cleanup: PublicationCleanup,
        recovery: PublicationRecovery,
    ) -> Result<Self, OperationalContractError> {
        let valid = cancellation != PublicationCancellation::NotRequested
            && cleanup == PublicationCleanup::Completed
            && recovery == PublicationRecovery::NotRequired
            && match cancellation {
                PublicationCancellation::DuringPublication => {
                    rollback == PublicationRollback::Performed
                }
                _ => rollback == PublicationRollback::NotNeeded,
            };
        if !valid {
            return Err(OperationalContractError::InvalidStateCombination);
        }
        Ok(Self {
            cancellation,
            rollback,
            cleanup,
            recovery,
        })
    }
    pub const fn cancellation(self) -> PublicationCancellation {
        self.cancellation
    }
    pub const fn rollback(self) -> PublicationRollback {
        self.rollback
    }
    pub const fn cleanup(self) -> PublicationCleanup {
        self.cleanup
    }
    pub const fn recovery(self) -> PublicationRecovery {
        self.recovery
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriterFailure {
    kind: WriterFailureKind,
    rollback: PublicationRollback,
    cleanup: PublicationCleanup,
    recovery: PublicationRecovery,
}

impl WriterFailure {
    pub fn new(
        kind: WriterFailureKind,
        rollback: PublicationRollback,
        cleanup: PublicationCleanup,
        recovery: PublicationRecovery,
    ) -> Result<Self, OperationalContractError> {
        if recovery == PublicationRecovery::Required
            && cleanup != PublicationCleanup::RetainedForRecovery
        {
            return Err(OperationalContractError::InvalidStateCombination);
        }
        Ok(Self {
            kind,
            rollback,
            cleanup,
            recovery,
        })
    }
    pub const fn kind(self) -> WriterFailureKind {
        self.kind
    }
    pub const fn rollback(self) -> PublicationRollback {
        self.rollback
    }
    pub const fn cleanup(self) -> PublicationCleanup {
        self.cleanup
    }
    pub const fn recovery(self) -> PublicationRecovery {
        self.recovery
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state", content = "detail")]
pub enum WriterLifecycle {
    Previewed,
    Applied,
    Rejected(WriterFailure),
    Cancelled(WriterInterruption),
}

impl WriterLifecycle {
    pub fn rejected(kind: WriterFailureKind) -> Self {
        Self::Rejected(
            WriterFailure::new(
                kind,
                PublicationRollback::NotNeeded,
                PublicationCleanup::Completed,
                PublicationRecovery::NotRequired,
            )
            .expect("rejected lifecycle is valid"),
        )
    }

    pub fn cancelled_before_execution() -> Self {
        Self::Cancelled(
            WriterInterruption::new(
                PublicationCancellation::BeforeExecution,
                PublicationRollback::NotNeeded,
                PublicationCleanup::Completed,
                PublicationRecovery::NotRequired,
            )
            .expect("pre-execution cancellation lifecycle is valid"),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SemanticChange {
    SourceCreated,
    SourceUpdated,
    SourceRemoved,
    RegistrationUpdated,
    SupportUpdated,
    ModuleUpdated,
    NoChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SemanticArtifact {
    Configuration,
    Extension,
    ExternalProcessor,
    ExternalReport,
    MetadataObject,
    Form,
    Template,
    Help,
    Interface,
    Role,
    Subsystem,
    SupportState,
    DataComposition,
    Spreadsheet,
    Module,
    RecoveryState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "identity", content = "value")]
pub enum SemanticObjectIdentity {
    Unspecified,
    ExternalObject {
        kind: ExternalArtifactKind,
        name: ExternalArtifactName,
    },
    ExternalObjectModule {
        kind: ExternalArtifactKind,
        owner: ExternalArtifactName,
    },
    ExternalPrimaryForm {
        kind: ExternalArtifactKind,
        owner: ExternalArtifactName,
        form: FormName,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SemanticArtifactRef {
    kind: SemanticArtifact,
    object: SemanticObjectIdentity,
}

impl SemanticArtifactRef {
    pub const fn new(kind: SemanticArtifact, object: SemanticObjectIdentity) -> Self {
        Self { kind, object }
    }
    pub const fn unidentified(kind: SemanticArtifact) -> Self {
        Self::new(kind, SemanticObjectIdentity::Unspecified)
    }
    pub const fn kind(&self) -> SemanticArtifact {
        self.kind
    }
    pub const fn object(&self) -> &SemanticObjectIdentity {
        &self.object
    }
}

impl From<SemanticArtifact> for SemanticArtifactRef {
    fn from(value: SemanticArtifact) -> Self {
        Self::unidentified(value)
    }
}

impl PartialEq<SemanticArtifact> for SemanticArtifactRef {
    fn eq(&self, other: &SemanticArtifact) -> bool {
        self.kind == *other && matches!(self.object, SemanticObjectIdentity::Unspecified)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticCode {
    Cancelled,
    InvalidRequest,
    InvalidDefinition,
    NotFound,
    AlreadyExists,
    UnsupportedState,
    UnsupportedFormat,
    AuthorabilityBlocked,
    SupportBlocked,
    NoDowngrade,
    Conflict,
    ValidationFailed,
    PlannerRejected,
    OwnerResolutionFailed,
    PublicationFailed,
    RollbackFailed,
    RecoveryRequired,
    PathRejected,
    ReadOnlyArtifact,
    AliasedArtifact,
    InvalidMutation,
    InvalidObjectReference,
    UnknownObjectKind,
    MissingFormCompanion,
    SupportCapabilityDisabled,
    InvalidModuleReference,
    ObjectNotBorrowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticField {
    Name,
    Owner,
    Module,
    Method,
    Definition,
    Mutation,
    SupportRule,
    Artifact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum DiagnosticDetail {
    Field(DiagnosticField),
    ObjectKind(SemanticObjectKind),
    Object(MetadataObjectReference),
    MetadataKind(MetadataKindName),
    Method(MethodName),
    FormElement(FormElementName),
    ConflictCount(u32),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriterDiagnostic {
    code: DiagnosticCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<DiagnosticDetail>,
}

impl WriterDiagnostic {
    pub const fn new(code: DiagnosticCode, detail: Option<DiagnosticDetail>) -> Self {
        Self { code, detail }
    }
    pub const fn code(&self) -> DiagnosticCode {
        self.code
    }
    pub const fn detail(&self) -> Option<&DiagnosticDetail> {
        self.detail.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "result")]
pub enum WriterEvidence {
    FormEdit(FormEditEvidence),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormEditEvidence {
    changed: bool,
    removed: Vec<FormEditRemoval>,
    validation: FormEditValidation,
}

impl FormEditEvidence {
    pub fn new(
        changed: bool,
        removed: Vec<FormEditRemoval>,
        validation: FormEditValidation,
    ) -> Self {
        Self {
            changed,
            removed,
            validation,
        }
    }
    pub const fn changed(&self) -> bool {
        self.changed
    }
    pub fn removed(&self) -> &[FormEditRemoval] {
        &self.removed
    }
    pub const fn validation(&self) -> FormEditValidation {
        self.validation
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormEditRemoval {
    name: FormElementName,
    #[serde(rename = "kind")]
    element_kind: FormElementKind,
    reason: FormEditRemovalReason,
}

impl FormEditRemoval {
    pub const fn new(
        name: FormElementName,
        element_kind: FormElementKind,
        reason: FormEditRemovalReason,
    ) -> Self {
        Self {
            name,
            element_kind,
            reason,
        }
    }
    pub const fn name(&self) -> &FormElementName {
        &self.name
    }
    pub const fn element_kind(&self) -> FormElementKind {
        self.element_kind
    }
    pub const fn reason(&self) -> FormEditRemovalReason {
        self.reason
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FormElementKind {
    Element,
    Input,
    ContextMenu,
    Tooltip,
    Group,
    Table,
    Button,
    CommandBar,
    Attribute,
    Command,
    Parameter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FormEditRemovalReason {
    Requested,
    Contained,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FormEditValidation {
    Passed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WriterMessageCode {
    Applied,
    PreviewPlanned,
    NoChange,
    Rejected,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriterResult {
    lifecycle: WriterLifecycle,
    message_code: WriterMessageCode,
    changes: Vec<SemanticChange>,
    artifacts: Vec<SemanticArtifactRef>,
    diagnostics: Vec<WriterDiagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    evidence: Option<WriterEvidence>,
}

impl WriterResult {
    pub fn new<A>(
        lifecycle: WriterLifecycle,
        changes: impl IntoIterator<Item = SemanticChange>,
        artifacts: impl IntoIterator<Item = A>,
        diagnostics: impl IntoIterator<Item = DiagnosticCode>,
    ) -> Result<Self, OperationalContractError>
    where
        A: Into<SemanticArtifactRef>,
    {
        Self::with_diagnostics(
            lifecycle,
            changes,
            artifacts,
            diagnostics
                .into_iter()
                .map(|code| WriterDiagnostic::new(code, None)),
        )
    }

    pub fn with_diagnostics<A>(
        lifecycle: WriterLifecycle,
        changes: impl IntoIterator<Item = SemanticChange>,
        artifacts: impl IntoIterator<Item = A>,
        diagnostics: impl IntoIterator<Item = WriterDiagnostic>,
    ) -> Result<Self, OperationalContractError>
    where
        A: Into<SemanticArtifactRef>,
    {
        let changes = changes.into_iter().collect::<Vec<_>>();
        let artifacts = artifacts.into_iter().map(Into::into).collect::<Vec<_>>();
        let diagnostics = diagnostics.into_iter().collect::<Vec<_>>();
        let valid = match lifecycle {
            WriterLifecycle::Previewed => {
                diagnostics.is_empty()
                    && !changes.is_empty()
                    && (changes == [SemanticChange::NoChange]
                        || !changes.contains(&SemanticChange::NoChange))
            }
            WriterLifecycle::Applied => {
                diagnostics.is_empty()
                    && !changes.is_empty()
                    && !artifacts
                        .iter()
                        .any(|artifact| artifact.kind() == SemanticArtifact::RecoveryState)
            }
            WriterLifecycle::Rejected(failure) => {
                changes.is_empty()
                    && artifacts
                        == if failure.recovery() == PublicationRecovery::Required {
                            vec![SemanticArtifactRef::unidentified(
                                SemanticArtifact::RecoveryState,
                            )]
                        } else {
                            Vec::new()
                        }
                    && !diagnostics.is_empty()
                    && !diagnostics
                        .iter()
                        .any(|item| item.code() == DiagnosticCode::Cancelled)
            }
            WriterLifecycle::Cancelled(_) => {
                changes.is_empty()
                    && artifacts.is_empty()
                    && diagnostics.len() == 1
                    && diagnostics[0].code() == DiagnosticCode::Cancelled
            }
        };
        if !valid {
            return Err(OperationalContractError::InvalidStateCombination);
        }
        let message_code = match lifecycle {
            WriterLifecycle::Previewed if changes == [SemanticChange::NoChange] => {
                WriterMessageCode::NoChange
            }
            WriterLifecycle::Previewed => WriterMessageCode::PreviewPlanned,
            WriterLifecycle::Applied if changes == [SemanticChange::NoChange] => {
                WriterMessageCode::NoChange
            }
            WriterLifecycle::Applied => WriterMessageCode::Applied,
            WriterLifecycle::Rejected(_) => WriterMessageCode::Rejected,
            WriterLifecycle::Cancelled(_) => WriterMessageCode::Cancelled,
        };
        Ok(Self {
            lifecycle,
            message_code,
            changes,
            artifacts,
            diagnostics,
            evidence: None,
        })
    }

    pub fn previewed(changed: bool) -> Self {
        Self::new(
            WriterLifecycle::Previewed,
            [if changed {
                SemanticChange::SourceUpdated
            } else {
                SemanticChange::NoChange
            }],
            std::iter::empty::<SemanticArtifactRef>(),
            [],
        )
        .expect("preview outcome is valid")
    }

    pub fn previewed_with_changes(
        changes: impl IntoIterator<Item = SemanticChange>,
    ) -> Result<Self, OperationalContractError> {
        Self::new(
            WriterLifecycle::Previewed,
            changes,
            std::iter::empty::<SemanticArtifactRef>(),
            [],
        )
    }

    pub fn cancelled() -> Self {
        Self::new(
            WriterLifecycle::cancelled_before_execution(),
            [],
            std::iter::empty::<SemanticArtifactRef>(),
            [DiagnosticCode::Cancelled],
        )
        .expect("cancelled outcome is valid")
    }

    pub fn cancelled_during_execution() -> Self {
        Self::new(
            WriterLifecycle::Cancelled(
                WriterInterruption::new(
                    PublicationCancellation::DuringExecution,
                    PublicationRollback::NotNeeded,
                    PublicationCleanup::Completed,
                    PublicationRecovery::NotRequired,
                )
                .expect("execution cancellation lifecycle is valid"),
            ),
            [],
            std::iter::empty::<SemanticArtifactRef>(),
            [DiagnosticCode::Cancelled],
        )
        .expect("cancelled outcome is valid")
    }

    pub fn cancelled_during_publication() -> Self {
        Self::new(
            WriterLifecycle::Cancelled(
                WriterInterruption::new(
                    PublicationCancellation::DuringPublication,
                    PublicationRollback::Performed,
                    PublicationCleanup::Completed,
                    PublicationRecovery::NotRequired,
                )
                .expect("publication cancellation lifecycle is valid"),
            ),
            [],
            std::iter::empty::<SemanticArtifactRef>(),
            [DiagnosticCode::Cancelled],
        )
        .expect("cancelled outcome is valid")
    }

    pub fn publication_recovery_required() -> Self {
        Self::new(
            WriterLifecycle::Rejected(
                WriterFailure::new(
                    WriterFailureKind::Publication,
                    PublicationRollback::Failed,
                    PublicationCleanup::RetainedForRecovery,
                    PublicationRecovery::Required,
                )
                .expect("publication recovery lifecycle is valid"),
            ),
            [],
            [SemanticArtifact::RecoveryState],
            [DiagnosticCode::RecoveryRequired],
        )
        .expect("publication recovery result is valid")
    }

    pub fn rejected(code: DiagnosticCode, kind: WriterFailureKind) -> Self {
        Self::new(
            WriterLifecycle::rejected(kind),
            [],
            std::iter::empty::<SemanticArtifactRef>(),
            [code],
        )
        .expect("rejected outcome is valid")
    }

    pub fn rejected_with_diagnostic(diagnostic: WriterDiagnostic, kind: WriterFailureKind) -> Self {
        Self::with_diagnostics(
            WriterLifecycle::rejected(kind),
            [],
            std::iter::empty::<SemanticArtifactRef>(),
            [diagnostic],
        )
        .expect("rejected outcome is valid")
    }

    pub fn with_evidence(mut self, evidence: Option<WriterEvidence>) -> Self {
        self.evidence = evidence;
        self
    }

    pub const fn lifecycle(&self) -> WriterLifecycle {
        self.lifecycle
    }
    pub const fn message_code(&self) -> WriterMessageCode {
        self.message_code
    }
    pub fn changes(&self) -> &[SemanticChange] {
        &self.changes
    }
    pub fn artifacts(&self) -> &[SemanticArtifactRef] {
        &self.artifacts
    }
    pub fn diagnostics(&self) -> &[WriterDiagnostic] {
        &self.diagnostics
    }
    pub const fn evidence(&self) -> Option<&WriterEvidence> {
        self.evidence.as_ref()
    }
}
