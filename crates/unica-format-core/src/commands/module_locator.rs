//! Format-neutral semantic result for locating a source module.

use crate::{ports::SemanticArtifactLease, semantic_ids::SemanticObjectKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSetName(String);

impl SourceSetName {
    pub fn new(value: impl Into<String>) -> Result<Self, ModuleLocatorContractError> {
        let value = value.into();
        if value.trim().is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
            return Err(ModuleLocatorContractError::InvalidSourceSetName);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

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
    source_set: Option<SourceSetName>,
}

impl ModuleArtifactLocation {
    pub const fn new(owner: ModuleOwner, role: ModuleRole) -> Self {
        Self {
            owner,
            role,
            source_set: None,
        }
    }

    pub fn with_source_set(mut self, source_set: SourceSetName) -> Self {
        self.source_set = Some(source_set);
        self
    }

    pub const fn owner(&self) -> &ModuleOwner {
        &self.owner
    }

    pub const fn role(&self) -> ModuleRole {
        self.role
    }

    pub const fn source_set(&self) -> Option<&SourceSetName> {
        self.source_set.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleLocatorContractError {
    EmptyObjectName,
    InvalidSourceSetName,
}

impl std::fmt::Display for ModuleLocatorContractError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::EmptyObjectName => "module owner object name must not be empty",
            Self::InvalidSourceSetName => "source set name is invalid",
        })
    }
}

impl std::error::Error for ModuleLocatorContractError {}

#[derive(Debug, Clone)]
pub struct LocatedModuleArtifact {
    location: ModuleArtifactLocation,
    lease: SemanticArtifactLease,
}

impl LocatedModuleArtifact {
    pub const fn new(location: ModuleArtifactLocation, lease: SemanticArtifactLease) -> Self {
        Self { location, lease }
    }

    pub const fn location(&self) -> &ModuleArtifactLocation {
        &self.location
    }

    pub const fn lease(&self) -> &SemanticArtifactLease {
        &self.lease
    }
}
