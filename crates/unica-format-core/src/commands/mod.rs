//! Closed, format-neutral mutation commands and results.
//!
//! Commands intentionally carry neither source locations nor native payloads.
//! Physical inputs are bound to an opaque operational session before a command
//! reaches a writer port.

use serde::{Deserialize, Serialize};

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
pub enum WriterCommand {
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

impl WriterCommand {
    pub const fn configuration(command: ConfigurationCommand) -> Self {
        Self::Configuration(command)
    }

    pub const fn extension(command: ExtensionCommand) -> Self {
        Self::Extension(command)
    }

    pub const fn external_artifact(command: ExternalArtifactCommand) -> Self {
        Self::ExternalArtifact(command)
    }

    pub const fn metadata(command: MetadataCommand) -> Self {
        Self::Metadata(command)
    }

    pub const fn form(command: FormCommand) -> Self {
        Self::Form(command)
    }

    pub const fn template(command: TemplateCommand) -> Self {
        Self::Template(command)
    }

    pub const fn help(command: HelpCommand) -> Self {
        Self::Help(command)
    }

    pub const fn interface(command: InterfaceCommand) -> Self {
        Self::Interface(command)
    }

    pub const fn role(command: RoleCommand) -> Self {
        Self::Role(command)
    }

    pub const fn subsystem(command: SubsystemCommand) -> Self {
        Self::Subsystem(command)
    }

    pub const fn support(command: SupportCommand) -> Self {
        Self::Support(command)
    }

    pub const fn data_composition(command: DataCompositionCommand) -> Self {
        Self::DataComposition(command)
    }

    pub const fn spreadsheet(command: SpreadsheetCommand) -> Self {
        Self::Spreadsheet(command)
    }

    pub const fn family(self) -> WriterFamily {
        match self {
            Self::Configuration(_) => WriterFamily::Configuration,
            Self::Extension(_) => WriterFamily::Extension,
            Self::ExternalArtifact(_) => WriterFamily::ExternalArtifact,
            Self::Metadata(_) => WriterFamily::Metadata,
            Self::Form(_) => WriterFamily::Form,
            Self::Template(_) => WriterFamily::Template,
            Self::Help(_) => WriterFamily::Help,
            Self::Interface(_) => WriterFamily::Interface,
            Self::Role(_) => WriterFamily::Role,
            Self::Subsystem(_) => WriterFamily::Subsystem,
            Self::Support(_) => WriterFamily::Support,
            Self::DataComposition(_) => WriterFamily::DataComposition,
            Self::Spreadsheet(_) => WriterFamily::Spreadsheet,
        }
    }

    pub const fn intent(self) -> &'static str {
        match self {
            Self::Configuration(ConfigurationCommand::Initialize) => "configuration.initialize",
            Self::Configuration(ConfigurationCommand::Edit) => "configuration.edit",
            Self::Extension(ExtensionCommand::Initialize) => "extension.initialize",
            Self::Extension(ExtensionCommand::Borrow) => "extension.borrow",
            Self::Extension(ExtensionCommand::PatchMethod) => "extension.patchMethod",
            Self::ExternalArtifact(ExternalArtifactCommand::InitializeProcessor) => {
                "externalArtifact.initializeProcessor"
            }
            Self::ExternalArtifact(ExternalArtifactCommand::InitializeReport) => {
                "externalArtifact.initializeReport"
            }
            Self::Metadata(MetadataCommand::Create) => "metadata.create",
            Self::Metadata(MetadataCommand::Edit) => "metadata.edit",
            Self::Metadata(MetadataCommand::Remove) => "metadata.remove",
            Self::Form(FormCommand::Create) => "form.create",
            Self::Form(FormCommand::Compile) => "form.compile",
            Self::Form(FormCommand::Edit) => "form.edit",
            Self::Form(FormCommand::Remove) => "form.remove",
            Self::Template(TemplateCommand::Create) => "template.create",
            Self::Template(TemplateCommand::Remove) => "template.remove",
            Self::Help(HelpCommand::Create) => "help.create",
            Self::Interface(InterfaceCommand::Edit) => "interface.edit",
            Self::Role(RoleCommand::Create) => "role.create",
            Self::Subsystem(SubsystemCommand::Create) => "subsystem.create",
            Self::Subsystem(SubsystemCommand::Edit) => "subsystem.edit",
            Self::Support(SupportCommand::Edit) => "support.edit",
            Self::DataComposition(DataCompositionCommand::Create) => "dataComposition.create",
            Self::DataComposition(DataCompositionCommand::Edit) => "dataComposition.edit",
            Self::Spreadsheet(SpreadsheetCommand::Create) => "spreadsheet.create",
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
