use crate::domain::source_resources::ResourceRole;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DomainEventKind {
    ConfigXmlChanged,
    CfeChanged,
    MetadataChanged,
    FormChanged,
    ModuleChanged,
    RoleChanged,
    DcsChanged,
    MxlChanged,
    SubsystemChanged,
    TemplateChanged,
    SourceSetChanged,
    BuildCompleted,
    SourceResourcesReplaced,
}

impl DomainEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConfigXmlChanged => "ConfigXmlChanged",
            Self::CfeChanged => "CfeChanged",
            Self::MetadataChanged => "MetadataChanged",
            Self::FormChanged => "FormChanged",
            Self::ModuleChanged => "ModuleChanged",
            Self::RoleChanged => "RoleChanged",
            Self::DcsChanged => "DcsChanged",
            Self::MxlChanged => "MxlChanged",
            Self::SubsystemChanged => "SubsystemChanged",
            Self::TemplateChanged => "TemplateChanged",
            Self::SourceSetChanged => "SourceSetChanged",
            Self::BuildCompleted => "BuildCompleted",
            Self::SourceResourcesReplaced => "SourceResourcesReplaced",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceResourcesReplaced {
    pub source_set: String,
    pub owner: String,
    pub roles: Vec<ResourceRole>,
    pub preimage_hashes: Vec<String>,
    pub postimage_hashes: Vec<String>,
    pub affected_targets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainEvent {
    pub kind: DomainEventKind,
    pub artifact: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<SourceResourcesReplaced>,
}

impl DomainEvent {
    pub fn new(kind: DomainEventKind, artifact: impl Into<String>) -> Self {
        Self {
            kind,
            artifact: artifact.into(),
            details: None,
        }
    }

    pub fn source_resources_replaced(details: SourceResourcesReplaced) -> Self {
        Self {
            kind: DomainEventKind::SourceResourcesReplaced,
            artifact: "unica.source.apply".to_string(),
            details: Some(details),
        }
    }

    pub fn name(&self) -> &'static str {
        self.kind.as_str()
    }
}

pub fn runtime_event_kind(operation: &str) -> Option<DomainEventKind> {
    match operation {
        "config-init" | "init" | "convert" | "dump" => Some(DomainEventKind::SourceSetChanged),
        "build" | "load" | "extensions" | "test" => Some(DomainEventKind::BuildCompleted),
        "make" | "syntax" | "launch" | "tools-download" => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{runtime_event_kind, DomainEvent, DomainEventKind, SourceResourcesReplaced};
    use crate::domain::source_resources::ResourceRole;
    use serde_json::json;

    #[test]
    fn runtime_job_and_synchronous_runtime_share_event_mapping() {
        assert_eq!(
            runtime_event_kind("dump"),
            Some(DomainEventKind::SourceSetChanged)
        );
        assert_eq!(
            runtime_event_kind("build"),
            Some(DomainEventKind::BuildCompleted)
        );
        assert_eq!(runtime_event_kind("make"), None);
    }

    #[test]
    fn source_apply_event_serializes_verified_logical_replacement_details() {
        let event = DomainEvent::source_resources_replaced(SourceResourcesReplaced {
            source_set: "main".to_string(),
            owner: "CommonModule.Shared".to_string(),
            roles: vec![ResourceRole::BslModule],
            preimage_hashes: vec!["sha256:before".to_string()],
            postimage_hashes: vec!["sha256:after".to_string()],
            affected_targets: vec!["CommonModule.Shared.Module".to_string()],
        });

        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({
                "kind": "SourceResourcesReplaced",
                "artifact": "unica.source.apply",
                "details": {
                    "sourceSet": "main",
                    "owner": "CommonModule.Shared",
                    "roles": ["bslModule"],
                    "preimageHashes": ["sha256:before"],
                    "postimageHashes": ["sha256:after"],
                    "affectedTargets": ["CommonModule.Shared.Module"]
                }
            })
        );
    }
}
