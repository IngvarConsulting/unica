//! What every durable Task store shares: the epoch clock, the closed tool
//! identity, the closed failure reason and the canonical result size limits.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Restart-stable time used only for durable timestamps and retention.
///
/// This is deliberately separate from the monotonic Invocation handoff clock:
/// `std::time::Instant` is process-local and cannot cross a daemon restart.
pub trait EpochMillisClock: Send + Sync {
    fn now_epoch_millis(&self) -> u64;
}

pub struct SystemEpochMillisClock;

impl EpochMillisClock for SystemEpochMillisClock {
    fn now_epoch_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or_default()
    }
}

/// Canonical invocation identity which cannot be populated with caller text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolIdentity {
    #[serde(rename = "unica.view")]
    View,
    #[serde(rename = "unica.apply")]
    Apply,
    #[serde(rename = "unica.resolve")]
    Resolve,
    #[serde(rename = "unica.search")]
    Search,
    #[serde(rename = "unica.check")]
    Check,
    #[serde(rename = "unica.diff")]
    Diff,
    #[serde(rename = "unica.run")]
    Run,
    #[serde(rename = "unica.docs")]
    Docs,
}

impl ToolIdentity {
    pub const ALL: [Self; 8] = [
        Self::View,
        Self::Apply,
        Self::Resolve,
        Self::Search,
        Self::Check,
        Self::Diff,
        Self::Run,
        Self::Docs,
    ];

    pub const fn catalog_name(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::Apply => "apply",
            Self::Resolve => "resolve",
            Self::Search => "search",
            Self::Check => "check",
            Self::Diff => "diff",
            Self::Run => "run",
            Self::Docs => "docs",
        }
    }

    pub fn from_wire_name(name: &str) -> Option<Self> {
        match name {
            "unica.view" => Some(Self::View),
            "unica.apply" => Some(Self::Apply),
            "unica.resolve" => Some(Self::Resolve),
            "unica.search" => Some(Self::Search),
            "unica.check" => Some(Self::Check),
            "unica.diff" => Some(Self::Diff),
            "unica.run" => Some(Self::Run),
            "unica.docs" => Some(Self::Docs),
            _ => None,
        }
    }
}

/// Closed reason persisted only for failed schema-v2 records. Runtime/store
/// diagnostics are deliberately not representable here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SafeFailureReason {
    InvocationFailed,
    ResultTooLarge,
    Interrupted,
    ResumeUnsupported,
    PersistenceFailed,
}

#[cfg(test)]
mod tests {
    use super::{SafeFailureReason, ToolIdentity};

    #[test]
    fn persisted_tool_identity_rejects_arbitrary_and_secret_bearing_values() {
        for rejected in [
            r#""unica.task.get""#,
            r#""unica.run?token=TASK_STORE_SECRET_SENTINEL""#,
            r#""https://user:password@example.invalid""#,
        ] {
            assert!(serde_json::from_str::<ToolIdentity>(rejected).is_err());
        }
    }

    #[test]
    fn safe_failure_reason_rejects_runtime_prose_paths_and_secrets() {
        for rejected in [
            r#""process exited with /private/tmp/secret""#,
            r#""TASK_STORE_SECRET_SENTINEL""#,
            r#""resumeOwner-vendor-extension""#,
        ] {
            assert!(serde_json::from_str::<SafeFailureReason>(rejected).is_err());
        }
        assert_eq!(
            serde_json::to_string(&SafeFailureReason::PersistenceFailed).unwrap(),
            r#""persistenceFailed""#
        );
    }
}
