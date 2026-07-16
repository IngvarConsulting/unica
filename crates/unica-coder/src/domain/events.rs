use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DomainEventKind {
    ConfigXmlChanged,
    CfeChanged,
    MetadataChanged,
    FormChanged,
    ModuleChanged,
    RoleChanged,
    SkdChanged,
    MxlChanged,
    SubsystemChanged,
    TemplateChanged,
    SourceSetChanged,
    BuildCompleted,
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
            Self::SkdChanged => "SkdChanged",
            Self::MxlChanged => "MxlChanged",
            Self::SubsystemChanged => "SubsystemChanged",
            Self::TemplateChanged => "TemplateChanged",
            Self::SourceSetChanged => "SourceSetChanged",
            Self::BuildCompleted => "BuildCompleted",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DomainEvent {
    pub kind: DomainEventKind,
    pub artifact: String,
}

impl DomainEvent {
    pub fn new(kind: DomainEventKind, artifact: impl Into<String>) -> Self {
        Self {
            kind,
            artifact: artifact.into(),
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
    use super::{runtime_event_kind, DomainEventKind};

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
}
