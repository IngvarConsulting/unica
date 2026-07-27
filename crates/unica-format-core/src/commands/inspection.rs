use crate::{
    commands::WriterResult,
    ports::{OperationCancellation, OperationalSourceSession},
    source::SourceAdapterError,
};

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
    fn inspect(&self, request: &InspectionRequest) -> Result<WriterResult, SourceAdapterError>;
}
