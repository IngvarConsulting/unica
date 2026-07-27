//! Closed, format-neutral mutation commands and results.
//!
//! Commands intentionally carry neither source locations nor native payloads.
//! Physical inputs are bound to an opaque operational session before a command
//! reaches a writer port.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

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
pub enum ConfigurationCommand {
    Initialize,
    Edit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionCommand {
    Initialize,
    Borrow,
    PatchMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalArtifactCommand {
    InitializeProcessor,
    InitializeReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataCommand {
    Create,
    Edit,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormCommand {
    Create,
    Compile,
    Edit,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateCommand {
    Create,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpCommand {
    Create,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfaceCommand {
    Edit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleCommand {
    Create,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubsystemCommand {
    Create,
    Edit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportCommand {
    Edit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataCompositionCommand {
    Create,
    Edit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpreadsheetCommand {
    Create,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriterCommandKind {
    Configuration(ConfigurationCommand),
    Extension(ExtensionCommand),
    ExternalArtifact(ExternalArtifactCommand),
    Metadata(MetadataCommand),
    Form(FormCommand),
    Template(TemplateCommand),
    Help(HelpCommand),
    Interface(InterfaceCommand),
    Role(RoleCommand),
    Subsystem(SubsystemCommand),
    Support(SupportCommand),
    DataComposition(DataCompositionCommand),
    Spreadsheet(SpreadsheetCommand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WriterSourceRole {
    Configuration,
    ConfigurationDirectory,
    Extension,
    DestinationDirectory,
    Definition,
    Object,
    SourceCollection,
    Form,
    Interface,
    Subsystem,
    DestinationArtifact,
    ParentSubsystem,
    Template,
    Rights,
    SupportTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterBorrowScope {
    Form,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriterArgument {
    Name(String),
    Synonym(String),
    Vendor(String),
    ArtifactVersion(String),
    Purpose(String),
    BorrowMainAttribute(WriterBorrowScope),
    Mode(String),
    ObjectReference(String),
    NamePrefix(String),
    ModuleReference(String),
    MethodName(String),
    InterceptorType(String),
    ExecutionContext(String),
    ObjectName(String),
    FormName(String),
    TemplateName(String),
    TemplateType(String),
    Language(String),
    MutationVerb(String),
    MutationValue(String),
    DataSet(String),
    Variant(String),
    ProcessorName(String),
    SupportCapability(String),
    SupportRule(String),
    OmitRole(bool),
    Function(bool),
    AssignDefaultForm(bool),
    AssignMainDataComposition(bool),
    SkipValidation(bool),
    ExcludeSelection(bool),
    CreateIfMissing(bool),
    Force(bool),
    KeepFiles(bool),
    DeriveFromObject(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WriterArguments {
    items: Vec<WriterArgument>,
}

impl WriterArguments {
    pub fn new(items: Vec<WriterArgument>) -> Result<Self, WriterCommandError> {
        let mut seen = HashSet::with_capacity(items.len());
        if items
            .iter()
            .any(|argument| !seen.insert(std::mem::discriminant(argument)))
        {
            return Err(WriterCommandError::DuplicateArgument);
        }
        Ok(Self { items })
    }

    pub fn items(&self) -> &[WriterArgument] {
        &self.items
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterCommandError {
    DuplicateArgument,
}

impl std::fmt::Display for WriterCommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("writer command contains a duplicate semantic argument")
    }
}

impl std::error::Error for WriterCommandError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriterCommand {
    kind: WriterCommandKind,
    arguments: WriterArguments,
}

impl WriterCommand {
    pub fn configuration(command: ConfigurationCommand) -> Self {
        Self::new(WriterCommandKind::Configuration(command))
    }

    pub fn extension(command: ExtensionCommand) -> Self {
        Self::new(WriterCommandKind::Extension(command))
    }

    pub fn external_artifact(command: ExternalArtifactCommand) -> Self {
        Self::new(WriterCommandKind::ExternalArtifact(command))
    }

    pub fn metadata(command: MetadataCommand) -> Self {
        Self::new(WriterCommandKind::Metadata(command))
    }

    pub fn form(command: FormCommand) -> Self {
        Self::new(WriterCommandKind::Form(command))
    }

    pub fn template(command: TemplateCommand) -> Self {
        Self::new(WriterCommandKind::Template(command))
    }

    pub fn help(command: HelpCommand) -> Self {
        Self::new(WriterCommandKind::Help(command))
    }

    pub fn interface(command: InterfaceCommand) -> Self {
        Self::new(WriterCommandKind::Interface(command))
    }

    pub fn role(command: RoleCommand) -> Self {
        Self::new(WriterCommandKind::Role(command))
    }

    pub fn subsystem(command: SubsystemCommand) -> Self {
        Self::new(WriterCommandKind::Subsystem(command))
    }

    pub fn support(command: SupportCommand) -> Self {
        Self::new(WriterCommandKind::Support(command))
    }

    pub fn data_composition(command: DataCompositionCommand) -> Self {
        Self::new(WriterCommandKind::DataComposition(command))
    }

    pub fn spreadsheet(command: SpreadsheetCommand) -> Self {
        Self::new(WriterCommandKind::Spreadsheet(command))
    }

    fn new(kind: WriterCommandKind) -> Self {
        Self {
            kind,
            arguments: WriterArguments::default(),
        }
    }

    pub fn with_arguments(mut self, arguments: WriterArguments) -> Self {
        self.arguments = arguments;
        self
    }

    pub fn arguments(&self) -> &WriterArguments {
        &self.arguments
    }

    pub const fn family(&self) -> WriterFamily {
        match self.kind {
            WriterCommandKind::Configuration(_) => WriterFamily::Configuration,
            WriterCommandKind::Extension(_) => WriterFamily::Extension,
            WriterCommandKind::ExternalArtifact(_) => WriterFamily::ExternalArtifact,
            WriterCommandKind::Metadata(_) => WriterFamily::Metadata,
            WriterCommandKind::Form(_) => WriterFamily::Form,
            WriterCommandKind::Template(_) => WriterFamily::Template,
            WriterCommandKind::Help(_) => WriterFamily::Help,
            WriterCommandKind::Interface(_) => WriterFamily::Interface,
            WriterCommandKind::Role(_) => WriterFamily::Role,
            WriterCommandKind::Subsystem(_) => WriterFamily::Subsystem,
            WriterCommandKind::Support(_) => WriterFamily::Support,
            WriterCommandKind::DataComposition(_) => WriterFamily::DataComposition,
            WriterCommandKind::Spreadsheet(_) => WriterFamily::Spreadsheet,
        }
    }

    pub const fn intent(&self) -> &'static str {
        match self.kind {
            WriterCommandKind::Configuration(ConfigurationCommand::Initialize) => {
                "configuration.initialize"
            }
            WriterCommandKind::Configuration(ConfigurationCommand::Edit) => "configuration.edit",
            WriterCommandKind::Extension(ExtensionCommand::Initialize) => "extension.initialize",
            WriterCommandKind::Extension(ExtensionCommand::Borrow) => "extension.borrow",
            WriterCommandKind::Extension(ExtensionCommand::PatchMethod) => "extension.patchMethod",
            WriterCommandKind::ExternalArtifact(ExternalArtifactCommand::InitializeProcessor) => {
                "externalArtifact.initializeProcessor"
            }
            WriterCommandKind::ExternalArtifact(ExternalArtifactCommand::InitializeReport) => {
                "externalArtifact.initializeReport"
            }
            WriterCommandKind::Metadata(MetadataCommand::Create) => "metadata.create",
            WriterCommandKind::Metadata(MetadataCommand::Edit) => "metadata.edit",
            WriterCommandKind::Metadata(MetadataCommand::Remove) => "metadata.remove",
            WriterCommandKind::Form(FormCommand::Create) => "form.create",
            WriterCommandKind::Form(FormCommand::Compile) => "form.compile",
            WriterCommandKind::Form(FormCommand::Edit) => "form.edit",
            WriterCommandKind::Form(FormCommand::Remove) => "form.remove",
            WriterCommandKind::Template(TemplateCommand::Create) => "template.create",
            WriterCommandKind::Template(TemplateCommand::Remove) => "template.remove",
            WriterCommandKind::Help(HelpCommand::Create) => "help.create",
            WriterCommandKind::Interface(InterfaceCommand::Edit) => "interface.edit",
            WriterCommandKind::Role(RoleCommand::Create) => "role.create",
            WriterCommandKind::Subsystem(SubsystemCommand::Create) => "subsystem.create",
            WriterCommandKind::Subsystem(SubsystemCommand::Edit) => "subsystem.edit",
            WriterCommandKind::Support(SupportCommand::Edit) => "support.edit",
            WriterCommandKind::DataComposition(DataCompositionCommand::Create) => {
                "dataComposition.create"
            }
            WriterCommandKind::DataComposition(DataCompositionCommand::Edit) => {
                "dataComposition.edit"
            }
            WriterCommandKind::Spreadsheet(SpreadsheetCommand::Create) => "spreadsheet.create",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WriterStatus {
    Previewed,
    Applied,
    Rejected,
    Cancelled,
    RecoveryRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WriterEffect {
    SourceCreated,
    SourceUpdated,
    SourceRemoved,
    RegistrationUpdated,
    SupportUpdated,
    NoChange,
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
    name: String,
    #[serde(rename = "kind")]
    element_kind: String,
    reason: FormEditRemovalReason,
}

impl FormEditRemoval {
    pub fn new(
        name: impl Into<String>,
        element_kind: impl Into<String>,
        reason: FormEditRemovalReason,
    ) -> Self {
        Self {
            name: name.into(),
            element_kind: element_kind.into(),
            reason,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn element_kind(&self) -> &str {
        &self.element_kind
    }

    pub const fn reason(&self) -> FormEditRemovalReason {
        self.reason
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriterResult {
    status: WriterStatus,
    effects: Vec<WriterEffect>,
    summary: String,
    changes: Vec<String>,
    warnings: Vec<String>,
    errors: Vec<String>,
    artifacts: Vec<String>,
    stdout: Option<String>,
    stderr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    evidence: Option<WriterEvidence>,
}

impl WriterResult {
    pub fn new(status: WriterStatus, effects: Vec<WriterEffect>) -> Self {
        Self {
            status,
            effects,
            summary: String::new(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
            evidence: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        ok: bool,
        mode: MutationMode,
        summary: String,
        changes: Vec<String>,
        warnings: Vec<String>,
        errors: Vec<String>,
        artifacts: Vec<String>,
        stdout: Option<String>,
        stderr: Option<String>,
    ) -> Self {
        let status = if ok {
            if mode.is_preview() {
                WriterStatus::Previewed
            } else {
                WriterStatus::Applied
            }
        } else {
            WriterStatus::Rejected
        };
        Self {
            status,
            effects: if changes.is_empty() {
                vec![WriterEffect::NoChange]
            } else {
                vec![WriterEffect::SourceUpdated]
            },
            summary,
            changes,
            warnings,
            errors,
            artifacts,
            stdout,
            stderr,
            evidence: None,
        }
    }

    pub fn cancelled() -> Self {
        Self {
            status: WriterStatus::Cancelled,
            effects: vec![WriterEffect::NoChange],
            summary: "operation cancelled".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec!["operation cancelled".to_string()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some("operation cancelled\n".to_string()),
            evidence: None,
        }
    }

    pub fn with_evidence(mut self, evidence: Option<WriterEvidence>) -> Self {
        self.evidence = evidence;
        self
    }

    pub const fn status(&self) -> WriterStatus {
        self.status
    }

    pub fn effects(&self) -> &[WriterEffect] {
        &self.effects
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }

    pub fn changes(&self) -> &[String] {
        &self.changes
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    pub fn artifacts(&self) -> &[String] {
        &self.artifacts
    }

    pub fn stdout(&self) -> Option<&str> {
        self.stdout.as_deref()
    }

    pub fn stderr(&self) -> Option<&str> {
        self.stderr.as_deref()
    }

    pub const fn evidence(&self) -> Option<&WriterEvidence> {
        self.evidence.as_ref()
    }
}
