use crate::{
    ports::{OperationCancellation, OperationalSourceSession},
    source::SourceAdapterError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectionResult {
    ok: bool,
    summary: String,
    changes: Vec<String>,
    warnings: Vec<String>,
    errors: Vec<String>,
    artifacts: Vec<String>,
    stdout: Option<String>,
    stderr: Option<String>,
}

impl InspectionResult {
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        ok: bool,
        summary: String,
        changes: Vec<String>,
        warnings: Vec<String>,
        errors: Vec<String>,
        artifacts: Vec<String>,
        stdout: Option<String>,
        stderr: Option<String>,
    ) -> Self {
        Self {
            ok,
            summary,
            changes,
            warnings,
            errors,
            artifacts,
            stdout,
            stderr,
        }
    }

    pub fn cancelled() -> Self {
        Self::from_parts(
            false,
            "operation cancelled".to_string(),
            Vec::new(),
            Vec::new(),
            vec!["operation cancelled".to_string()],
            Vec::new(),
            None,
            None,
        )
    }

    pub const fn ok(&self) -> bool {
        self.ok
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigurationInspection {
    Describe,
    Validate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionInspection {
    Compare,
    Validate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataInspection {
    Describe,
    Validate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormInspection {
    Describe,
    Validate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfaceInspection {
    Validate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubsystemInspection {
    Describe,
    Validate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateInspection {
    Describe,
    Validate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataCompositionInspection {
    Describe,
    Validate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpreadsheetInspection {
    Decompile,
    Describe,
    Validate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleInspection {
    Describe,
    Validate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectionCommand {
    Configuration(ConfigurationInspection),
    Extension(ExtensionInspection),
    Metadata(MetadataInspection),
    Form(FormInspection),
    Interface(InterfaceInspection),
    Subsystem(SubsystemInspection),
    Template(TemplateInspection),
    DataComposition(DataCompositionInspection),
    Spreadsheet(SpreadsheetInspection),
    Role(RoleInspection),
}

impl InspectionCommand {
    pub const fn intent(self) -> &'static str {
        match self {
            Self::Configuration(ConfigurationInspection::Describe) => "configuration.describe",
            Self::Configuration(ConfigurationInspection::Validate) => "configuration.validate",
            Self::Extension(ExtensionInspection::Compare) => "extension.compare",
            Self::Extension(ExtensionInspection::Validate) => "extension.validate",
            Self::Metadata(MetadataInspection::Describe) => "metadata.describe",
            Self::Metadata(MetadataInspection::Validate) => "metadata.validate",
            Self::Form(FormInspection::Describe) => "form.describe",
            Self::Form(FormInspection::Validate) => "form.validate",
            Self::Interface(InterfaceInspection::Validate) => "interface.validate",
            Self::Subsystem(SubsystemInspection::Describe) => "subsystem.describe",
            Self::Subsystem(SubsystemInspection::Validate) => "subsystem.validate",
            Self::Template(TemplateInspection::Describe) => "template.describe",
            Self::Template(TemplateInspection::Validate) => "template.validate",
            Self::DataComposition(DataCompositionInspection::Describe) => {
                "dataComposition.describe"
            }
            Self::DataComposition(DataCompositionInspection::Validate) => {
                "dataComposition.validate"
            }
            Self::Spreadsheet(SpreadsheetInspection::Decompile) => "spreadsheet.decompile",
            Self::Spreadsheet(SpreadsheetInspection::Describe) => "spreadsheet.describe",
            Self::Spreadsheet(SpreadsheetInspection::Validate) => "spreadsheet.validate",
            Self::Role(RoleInspection::Describe) => "role.describe",
            Self::Role(RoleInspection::Validate) => "role.validate",
        }
    }
}

#[derive(Debug, Clone)]
pub struct InspectionRequest {
    session: OperationalSourceSession,
    command: InspectionCommand,
    cancellation: OperationCancellation,
}

impl InspectionRequest {
    pub fn new(
        session: OperationalSourceSession,
        command: InspectionCommand,
        cancellation: OperationCancellation,
    ) -> Self {
        Self {
            session,
            command,
            cancellation,
        }
    }

    pub const fn session(&self) -> &OperationalSourceSession {
        &self.session
    }

    pub const fn command(&self) -> InspectionCommand {
        self.command
    }

    pub const fn cancellation(&self) -> &OperationCancellation {
        &self.cancellation
    }
}

pub trait InspectionPort: Send + Sync {
    fn inspect(&self, request: &InspectionRequest) -> Result<InspectionResult, SourceAdapterError>;
}
