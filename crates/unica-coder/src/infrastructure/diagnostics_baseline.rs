//! Verdict on the diagnostics baseline summary of bsl-analyzer.
//!
//! The analyzer reports its `[diagnostics.baseline]` the same way in the JSONL
//! `done` event of `analyze` and next to the findings of the resident
//! `diagnostics file` reply. Both readers ask this module one question: did the
//! baseline act as a proven filter, or could it not classify the findings?
//!
//! The baseline is the user's own filter. Known findings it hides are asked
//! away, not missing, so a proven baseline keeps the provider result complete
//! and is named as a fact instead. Upstream (bsl-analyzer v0.2.86):
//!
//! - `ide/src/diagnostics_baseline.rs` (`classify_diagnostics_with`): `state`
//!   is `full` only when the whole project was covered and `partial` otherwise;
//!   `known` and `new` are exact for every analysed file in both states.
//!   `partial` only means that `resolved` could not be computed project-wide.
//! - `bsl-analyzer/src/bin/cli/analyze.rs` (`analysis_is_full`): any scope,
//!   including the `--diff-filter` that scopes a module check, makes the run
//!   `partial`; the resident `diagnostics file` reply is always `partial`.
//! - `DiagnosticsBaselineSummary::interrupted`: a classification that never ran
//!   to a verdict is `partial` without `known`.
//! - A broken baseline is `state: "error"` with `error_code`/`detail`, and a
//!   broken partition adds an entry to `errors`.
//!
//! The baseline therefore leaves the result incomplete only when it is broken
//! (`error`, a non-empty `errors`, an `error_code`) or its classification did
//! not happen (`partial` without `known`). Everything else is a proven filter.

use crate::domain::diagnostics::{
    DiagnosticSuppression, DiagnosticSuppressionReason, DiagnosticSuppressionSource,
};
use crate::infrastructure::redaction::redactor;
use serde::Deserialize;

/// The closed set of baseline states the analyzer publishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BaselineState {
    Disabled,
    Full,
    Partial,
    Error,
}

/// One baseline error as the analyzer names it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BaselineIssue<'a> {
    pub(crate) code: &'a str,
    pub(crate) detail: &'a str,
}

/// The fields of a baseline summary the verdict depends on.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BaselineFacts<'a> {
    pub(crate) state: BaselineState,
    pub(crate) known: Option<usize>,
    pub(crate) new: Option<usize>,
    pub(crate) error_code: Option<&'a str>,
    pub(crate) detail: Option<&'a str>,
    pub(crate) first_error: Option<BaselineIssue<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BaselineVerdict {
    /// The baseline did not prevent the findings from being classified.
    pub(crate) intact: bool,
    /// The fact published with the result; `None` when no baseline is configured.
    pub(crate) suppression: Option<DiagnosticSuppression>,
}

impl BaselineVerdict {
    pub(crate) fn disabled() -> Self {
        Self {
            intact: true,
            suppression: None,
        }
    }
}

const INTERRUPTED_DETAIL: &str = "the diagnostics baseline did not classify the analysed findings";

pub(crate) fn baseline_verdict(facts: BaselineFacts<'_>) -> BaselineVerdict {
    if facts.state == BaselineState::Disabled {
        return BaselineVerdict::disabled();
    }
    let reason = failure_reason(&facts);
    BaselineVerdict {
        intact: reason.is_none(),
        suppression: Some(DiagnosticSuppression {
            by: DiagnosticSuppressionSource::Baseline,
            known: facts.known,
            new: facts.new,
            reason,
        }),
    }
}

fn failure_reason(facts: &BaselineFacts<'_>) -> Option<DiagnosticSuppressionReason> {
    let named = |code: Option<&str>, detail: Option<&str>, fallback: &str| {
        Some(DiagnosticSuppressionReason {
            code: public_code(code, fallback),
            detail: detail
                .filter(|detail| !detail.trim().is_empty())
                .map(redactor),
        })
    };
    if facts.state == BaselineState::Error
        || facts.error_code.is_some()
        || facts.first_error.is_some()
    {
        // The summary's own error comes first; a partition error names the
        // cause when the summary does not.
        let code = facts
            .error_code
            .or(facts.first_error.map(|issue| issue.code));
        let detail = facts.detail.or(facts.first_error.map(|issue| issue.detail));
        return named(code, detail, "baseline_error");
    }
    if facts.state == BaselineState::Partial && facts.known.is_none() {
        return named(None, Some(INTERRUPTED_DETAIL), "baseline_interrupted");
    }
    None
}

/// The analyzer's error code, kept only when it is a plain identifier: it is
/// published as a code, not as prose.
fn public_code(code: Option<&str>, fallback: &str) -> String {
    match code {
        Some(code)
            if !code.is_empty()
                && code.len() <= 64
                && code
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')) =>
        {
            code.to_string()
        }
        _ => fallback.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(state: BaselineState) -> BaselineFacts<'static> {
        BaselineFacts {
            state,
            known: None,
            new: None,
            error_code: None,
            detail: None,
            first_error: None,
        }
    }

    #[test]
    fn baseline_verdict_is_a_filter_unless_broken_or_unclassified() {
        // Disabled: nothing configured, nothing to name.
        assert_eq!(
            baseline_verdict(facts(BaselineState::Disabled)),
            BaselineVerdict::disabled()
        );

        // Full and partial with an exact classification: a proven filter.
        for state in [BaselineState::Full, BaselineState::Partial] {
            let verdict = baseline_verdict(BaselineFacts {
                known: Some(3),
                new: Some(1),
                ..facts(state)
            });
            assert!(verdict.intact, "{state:?}");
            let suppression = verdict.suppression.expect("fact is named");
            assert_eq!(suppression.by, DiagnosticSuppressionSource::Baseline);
            assert_eq!((suppression.known, suppression.new), (Some(3), Some(1)));
            assert!(suppression.reason.is_none(), "{state:?}");
        }

        // Partial without `known`: the classification was interrupted.
        let interrupted = baseline_verdict(facts(BaselineState::Partial));
        assert!(!interrupted.intact);
        let reason = interrupted.suppression.unwrap().reason.unwrap();
        assert_eq!(reason.code, "baseline_interrupted");
        assert_eq!(reason.detail.as_deref(), Some(INTERRUPTED_DETAIL));

        // Error state: the analyzer's own code and detail, secrets redacted.
        let broken = baseline_verdict(BaselineFacts {
            error_code: Some("invalid_path"),
            detail: Some("baseline path escapes the project; Pwd=hunter2"),
            ..facts(BaselineState::Error)
        });
        assert!(!broken.intact);
        let reason = broken.suppression.unwrap().reason.unwrap();
        assert_eq!(reason.code, "invalid_path");
        let detail = reason.detail.unwrap();
        assert!(detail.starts_with("baseline path escapes the project"));
        assert!(!detail.contains("hunter2"), "{detail}");

        // A partition error breaks an otherwise full summary and names itself.
        let partition = baseline_verdict(BaselineFacts {
            known: Some(0),
            first_error: Some(BaselineIssue {
                code: "missing_partition",
                detail: "partition extension:Ext has no object",
            }),
            ..facts(BaselineState::Full)
        });
        assert!(!partition.intact);
        assert_eq!(
            partition.suppression.unwrap().reason.unwrap().code,
            "missing_partition"
        );

        // An `error_code` alone is enough, and a code that is not an
        // identifier is not published as one.
        let coded = baseline_verdict(BaselineFacts {
            known: Some(2),
            error_code: Some("not a code"),
            ..facts(BaselineState::Full)
        });
        assert!(!coded.intact);
        let reason = coded.suppression.unwrap().reason.unwrap();
        assert_eq!(reason.code, "baseline_error");
        assert!(reason.detail.is_none());
    }
}
