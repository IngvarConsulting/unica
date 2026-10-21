use serde::{Deserialize, Serialize};

/// Classifies whether interrupting an operation can leave its workspace inconsistent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CancelPolicy {
    Safe,
    Critical,
}

/// The operation classes deliberately accepted by the durable core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeJobOperation {
    Make,
    Syntax,
    Test,
    ConfigInit,
    Init,
    Build,
    Dump,
    Convert,
    Load,
    Launch,
    Extensions,
}

impl RuntimeJobOperation {
    pub fn cancel_policy(self) -> CancelPolicy {
        match self {
            Self::Make | Self::Syntax | Self::Test => CancelPolicy::Safe,
            Self::ConfigInit
            | Self::Init
            | Self::Build
            | Self::Dump
            | Self::Convert
            | Self::Load
            | Self::Launch
            | Self::Extensions => CancelPolicy::Critical,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Make => "make",
            Self::Syntax => "syntax",
            Self::Test => "test",
            Self::ConfigInit => "config-init",
            Self::Init => "init",
            Self::Build => "build",
            Self::Dump => "dump",
            Self::Convert => "convert",
            Self::Load => "load",
            Self::Launch => "launch",
            Self::Extensions => "extensions",
        }
    }

    pub fn from_label(label: &str) -> Result<Self, String> {
        match label {
            "make" => Ok(Self::Make),
            "syntax" => Ok(Self::Syntax),
            "test" => Ok(Self::Test),
            "config-init" => Ok(Self::ConfigInit),
            "init" => Ok(Self::Init),
            "build" => Ok(Self::Build),
            "dump" => Ok(Self::Dump),
            "convert" => Ok(Self::Convert),
            "load" => Ok(Self::Load),
            "launch" => Ok(Self::Launch),
            "extensions" => Ok(Self::Extensions),
            _ => Err(unica_redaction::redactor(
                "unsupported runtime job operation",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Загрузку зависимостей продукт стороннему исполнителю не делегирует:
    /// `tools-download` снят с провода вместе с v0.12 и не должен доживать
    /// в словаре заданий как принимаемая операция (#871, B-2).
    #[test]
    fn tools_download_is_not_a_runtime_job_operation() {
        assert!(RuntimeJobOperation::from_label("tools-download").is_err());
    }
}

use unica_source_contract::events::DomainEventKind;
pub fn runtime_event_kind(operation: &str) -> Option<DomainEventKind> {
    match operation {
        "config-init" | "init" | "convert" | "dump" => Some(DomainEventKind::SourceSetChanged),
        "build" | "load" | "extensions" | "test" => Some(DomainEventKind::BuildCompleted),
        "make" | "syntax" | "launch" => None,
        _ => None,
    }
}

#[cfg(test)]
mod event_tests {
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
