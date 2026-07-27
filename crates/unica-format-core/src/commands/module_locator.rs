//! Format-neutral semantic result for locating a source module.

use crate::semantic_ids::SemanticObjectKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleOwner {
    Configuration,
    Object {
        kind: SemanticObjectKind,
        name: String,
    },
}

impl ModuleOwner {
    pub fn object(
        kind: SemanticObjectKind,
        name: impl Into<String>,
    ) -> Result<Self, ModuleLocatorContractError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(ModuleLocatorContractError::EmptyObjectName);
        }
        Ok(Self::Object { kind, name })
    }

    pub const fn kind(&self) -> SemanticObjectKind {
        match self {
            Self::Configuration => SemanticObjectKind::Configuration,
            Self::Object { kind, .. } => *kind,
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Configuration => None,
            Self::Object { name, .. } => Some(name),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleRole {
    Module,
    Object,
    Manager,
    RecordSet,
    ValueManager,
    Form,
    Command,
    ManagedApplication,
    OrdinaryApplication,
    Session,
    ExternalConnection,
}

impl ModuleRole {
    pub const fn public_label(self) -> &'static str {
        match self {
            Self::Module => "Module",
            Self::Object => "ObjectModule",
            Self::Manager => "ManagerModule",
            Self::RecordSet => "RecordSetModule",
            Self::ValueManager => "ValueManagerModule",
            Self::Form => "FormModule",
            Self::Command => "CommandModule",
            Self::ManagedApplication => "ManagedApplicationModule",
            Self::OrdinaryApplication => "OrdinaryApplicationModule",
            Self::Session => "SessionModule",
            Self::ExternalConnection => "ExternalConnectionModule",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleArtifactLocation {
    owner: ModuleOwner,
    role: ModuleRole,
}

impl ModuleArtifactLocation {
    pub const fn new(owner: ModuleOwner, role: ModuleRole) -> Self {
        Self { owner, role }
    }

    pub const fn owner(&self) -> &ModuleOwner {
        &self.owner
    }

    pub const fn role(&self) -> ModuleRole {
        self.role
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleLocatorContractError {
    EmptyObjectName,
}

impl std::fmt::Display for ModuleLocatorContractError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("module owner object name must not be empty")
    }
}

impl std::error::Error for ModuleLocatorContractError {}
