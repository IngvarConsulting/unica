use unica_format_core::{
    navigation::{NavigationSelection, ObjectKey, OpaqueNavigationCursor},
    ports::{
        AuthorabilityPort, AuthorabilityRequest, AuthorabilityResult, CompatibilityPort,
        CompatibilityRequest, FormatDiagnostic, OperationalValidationPort,
        OperationalValidationRequest, OperationalValidationResult, PublicationPort,
        PublicationRequest, PublicationResult, SourceCompatibilityPort,
        SourceCompatibilityRequest, ValidationContextPort, ValidationContextRequest,
        ValidationContextResult,
    },
    source::SourceAdapterError,
    source::{SourceId, SourceRevision},
};

#[derive(Debug, Clone)]
pub struct MetadataNavigationCommand {
    pub target: MetadataNavigationTarget,
    pub selection: Option<NavigationSelection>,
}

#[derive(Debug, Clone)]
pub enum MetadataNavigationTarget {
    Source,
    ObjectRef {
        source_id: SourceId,
        object_key: ObjectKey,
        snapshot_revision: SourceRevision,
    },
    Cursor(OpaqueNavigationCursor),
}

#[derive(Debug, Clone)]
pub struct CompatibilityPolicyCommand {
    request: CompatibilityRequest,
    mutating: bool,
}

impl CompatibilityPolicyCommand {
    pub const fn new(request: CompatibilityRequest, mutating: bool) -> Self {
        Self { request, mutating }
    }
}

#[derive(Debug, Clone)]
pub struct AuthorabilityPolicyCommand {
    request: AuthorabilityRequest,
    enforcement: GuardEnforcement,
}

impl AuthorabilityPolicyCommand {
    pub const fn new(request: AuthorabilityRequest, enforcement: GuardEnforcement) -> Self {
        Self {
            request,
            enforcement,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardEnforcement {
    Deny,
    Warn,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationalPolicyDecision {
    Allow,
    Warn(FormatDiagnostic),
    Block(FormatDiagnostic),
}

impl OperationalPolicyDecision {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Warn(_) => "warn",
            Self::Block(_) => "block",
        }
    }
}

pub struct OperationalPolicyService;

impl OperationalPolicyService {
    pub fn check_compatibility(
        port: &dyn CompatibilityPort,
        command: CompatibilityPolicyCommand,
    ) -> Result<OperationalPolicyDecision, SourceAdapterError> {
        let result = port.inspect(&command.request)?;
        Ok(match result.into_issue() {
            None => OperationalPolicyDecision::Allow,
            Some(issue) if command.mutating => {
                OperationalPolicyDecision::Block(issue.into_diagnostic())
            }
            Some(issue) => OperationalPolicyDecision::Warn(issue.into_diagnostic()),
        })
    }

    pub fn check_source_compatibility(
        port: &dyn SourceCompatibilityPort,
        request: &SourceCompatibilityRequest,
    ) -> Result<OperationalPolicyDecision, SourceAdapterError> {
        let result = port.inspect_source(request)?;
        Ok(match result.into_diagnostic() {
            Some(diagnostic) => OperationalPolicyDecision::Block(diagnostic),
            None => OperationalPolicyDecision::Allow,
        })
    }

    pub fn check_authorability(
        port: &dyn AuthorabilityPort,
        command: AuthorabilityPolicyCommand,
    ) -> Result<OperationalPolicyDecision, SourceAdapterError> {
        let result = port.inspect(&command.request)?;
        Ok(match result {
            AuthorabilityResult::Allowed(_) => OperationalPolicyDecision::Allow,
            AuthorabilityResult::Denied(denial) => {
                Self::decide_authorability(denial.diagnostic().clone(), command.enforcement)
            }
        })
    }

    pub fn decide_authorability(
        denial: FormatDiagnostic,
        enforcement: GuardEnforcement,
    ) -> OperationalPolicyDecision {
        match enforcement {
            GuardEnforcement::Off => OperationalPolicyDecision::Allow,
            GuardEnforcement::Warn => OperationalPolicyDecision::Warn(denial),
            GuardEnforcement::Deny => OperationalPolicyDecision::Block(denial),
        }
    }

    pub fn validation_context(
        port: &dyn ValidationContextPort,
        request: &ValidationContextRequest,
    ) -> Result<ValidationContextResult, SourceAdapterError> {
        port.inspect(request)
    }

    pub fn publish(
        port: &dyn PublicationPort,
        request: &PublicationRequest,
    ) -> Result<PublicationResult, SourceAdapterError> {
        port.publish(request)
    }

    pub fn validate(
        port: &dyn OperationalValidationPort,
        request: &OperationalValidationRequest,
    ) -> Result<OperationalValidationResult, SourceAdapterError> {
        port.validate(request)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn application_command_boundary_has_no_json_or_filesystem_transport_shapes() {
        let commands = include_str!("commands.rs");
        let orchestration = include_str!("navigation.rs");

        assert!(!commands.contains(concat!("serde_json", "::Value")));
        assert!(!commands.contains(concat!("ObjectPath", "(String)")));
        assert!(!orchestration.contains("source_target_path"));
    }
}
