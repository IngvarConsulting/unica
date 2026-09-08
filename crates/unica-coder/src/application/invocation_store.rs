//! What every durable Task store shares: the epoch clock, the closed tool
//! identity, the closed failure reason and the canonical result size limits.

use crate::domain::invocation::DomainResult;
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const MAX_CANONICAL_RESULT_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_TASK_RECORD_ENVELOPE_BYTES: usize = 64 * 1024;
pub(crate) const MAX_TASK_RECORD_BYTES: usize =
    MAX_CANONICAL_RESULT_BYTES + MAX_TASK_RECORD_ENVELOPE_BYTES;

/// Restart-stable time used only for durable timestamps and retention.
///
/// This is deliberately separate from the monotonic Invocation handoff clock:
/// `std::time::Instant` is process-local and cannot cross a daemon restart.
pub(crate) trait EpochMillisClock: Send + Sync {
    fn now_epoch_millis(&self) -> u64;
}

pub(crate) struct SystemEpochMillisClock;

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
pub(crate) enum ToolIdentity {
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
    pub(crate) const ALL: [Self; 8] = [
        Self::View,
        Self::Apply,
        Self::Resolve,
        Self::Search,
        Self::Check,
        Self::Diff,
        Self::Run,
        Self::Docs,
    ];

    pub(crate) const fn catalog_name(self) -> &'static str {
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

    pub(crate) fn from_wire_name(name: &str) -> Option<Self> {
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
pub(crate) enum SafeFailureReason {
    InvocationFailed,
    ResultTooLarge,
    Interrupted,
    ResumeUnsupported,
    PersistenceFailed,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CanonicalResultSizeError<E> {
    TooLarge,
    Checkpoint(E),
    Serialization,
}

struct BoundedCountingWriter<'a, E, F>
where
    F: FnMut() -> Result<(), E>,
{
    bytes: usize,
    limit: usize,
    checkpoint: &'a mut F,
    failure: Option<CanonicalResultSizeError<E>>,
}

impl<E, F> Write for BoundedCountingWriter<'_, E, F>
where
    F: FnMut() -> Result<(), E>,
{
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if let Err(error) = (self.checkpoint)() {
            self.failure = Some(CanonicalResultSizeError::Checkpoint(error));
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "canonical result checkpoint rejected",
            ));
        }
        let Some(next) = self.bytes.checked_add(buffer.len()) else {
            self.failure = Some(CanonicalResultSizeError::TooLarge);
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "canonical result exceeds byte limit",
            ));
        };
        if next > self.limit {
            self.failure = Some(CanonicalResultSizeError::TooLarge);
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "canonical result exceeds byte limit",
            ));
        }
        self.bytes = next;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn canonical_result_size_with_checkpoint<E, F>(
    result: &DomainResult,
    mut checkpoint: F,
) -> Result<usize, CanonicalResultSizeError<E>>
where
    F: FnMut() -> Result<(), E>,
{
    let mut writer = BoundedCountingWriter {
        bytes: 0,
        limit: MAX_CANONICAL_RESULT_BYTES,
        checkpoint: &mut checkpoint,
        failure: None,
    };
    let serialized = serde_json::to_writer(&mut writer, result);
    if let Some(error) = writer.failure.take() {
        return Err(error);
    }
    if serialized.is_err() {
        return Err(CanonicalResultSizeError::Serialization);
    }
    let bytes = writer.bytes;
    drop(writer);
    checkpoint().map_err(CanonicalResultSizeError::Checkpoint)?;
    Ok(bytes)
}

pub(crate) fn canonical_result_size(
    result: &DomainResult,
) -> Result<usize, CanonicalResultSizeError<std::convert::Infallible>> {
    canonical_result_size_with_checkpoint(result, || Ok(()))
}

#[cfg(test)]
mod tests {
    use super::{SafeFailureReason, ToolIdentity};
    use crate::application::tool_contracts::SurfaceRelease;
    use crate::application::v13::tool_catalog::catalog_for;

    #[test]
    fn persisted_tool_identity_matches_the_eight_invocation_catalog_entries() {
        let catalog = catalog_for(SurfaceRelease::V13).expect("hidden v0.13 catalog");
        let expected = catalog
            .tools
            .iter()
            .map(|tool| format!("unica.{}", tool.name))
            .collect::<Vec<_>>();
        let actual = ToolIdentity::ALL
            .iter()
            .map(|tool| {
                serde_json::to_value(tool)
                    .expect("tool serializes")
                    .as_str()
                    .expect("tool identity is a string")
                    .to_string()
            })
            .collect::<Vec<_>>();

        assert_eq!(actual, expected);
    }

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
