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
//! - A broken baseline reaches the two readers differently. The resident
//!   `diagnostics file` reply carries it as `state: "error"` with
//!   `error_code`/`detail`, and a broken partition adds an entry to `errors`.
//!   The CLI `analyze` validates the baseline before it emits `start`
//!   (`analyze.rs`, `DiagnosticsBaselineSnapshot::load`): a broken one ends the
//!   run with a non-zero exit and `Error: "<detail>"` on stderr, so no `done`
//!   event ever names it. [`cli_baseline_failure`] reads that line.
//!
//! The baseline therefore leaves the result incomplete only when it is broken
//! (`error`, a non-empty `errors`, an `error_code`, a refused CLI run) or its
//! classification did not happen (`partial` without `known`). Everything else
//! is a proven filter.

use crate::domain::diagnostics::{
    DiagnosticProviderOutcome, DiagnosticProviderStatus, DiagnosticSuppression,
    DiagnosticSuppressionReason,
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
        suppression: Some(DiagnosticSuppression::Baseline {
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
                .map(public_detail),
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

/// Where an upstream baseline message spells a physical path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathAt {
    /// The message names no physical path.
    Nowhere,
    /// Everything after the prefix is the path.
    Tail,
    /// The path runs from the prefix to the last `": "`; an I/O error follows.
    BeforeLastColon,
}

/// The code an upstream I/O failure is published under, decided like
/// upstream does (`ErrorKind::NotFound` is `missing`).
const IO_CODE: &str = "io";

/// Every message bsl-analyzer v0.2.86 can end a CLI `analyze` with when its
/// `[diagnostics.baseline]` is broken, with the code the resident reply uses
/// for the same failure (`ide-host-core/src/diagnostics_baseline.rs`):
///
/// - snapshot I/O (`load_once`): `cannot read diagnostics baseline …`,
///   `cannot open diagnostics baseline directory …`;
/// - `ide/src/diagnostics_baseline.rs` `DiagnosticsBaselineError`;
/// - `project-model` `DiagnosticsBaselineProjectError` (`invalid_configuration`);
/// - `ide/src/partitioned_diagnostics_baseline.rs`
///   `PartitionedDiagnosticsBaselineError`, coded by its `info()`.
///
/// A message not listed here is not read as a baseline failure: the run stays
/// an unexplained provider failure rather than a guessed cause.
const CLI_BASELINE_ERRORS: &[(&str, &str, PathAt)] = &[
    (
        "cannot read diagnostics baseline ",
        IO_CODE,
        PathAt::BeforeLastColon,
    ),
    (
        "cannot open diagnostics baseline directory ",
        IO_CODE,
        PathAt::BeforeLastColon,
    ),
    (
        "unsupported diagnostics baseline schema version ",
        "unsupported_schema",
        PathAt::Nowhere,
    ),
    (
        "diagnostics baseline scope does not match the current project",
        "scope_mismatch",
        PathAt::Nowhere,
    ),
    (
        "invalid diagnostics baseline JSON: ",
        "invalid_file",
        PathAt::Nowhere,
    ),
    (
        "diagnostics baseline contains invalid relative path: ",
        "invalid_file",
        PathAt::Nowhere,
    ),
    (
        "diagnostics baseline fingerprint does not match its fields: ",
        "invalid_file",
        PathAt::Nowhere,
    ),
    (
        "diagnostics baseline contains duplicate entry: ",
        "invalid_file",
        PathAt::Nowhere,
    ),
    (
        "diagnostics baseline cannot contain protected diagnostic: ",
        "invalid_file",
        PathAt::Nowhere,
    ),
    (
        "diagnostics baseline target is a symlink: ",
        "invalid_configuration",
        PathAt::Tail,
    ),
    (
        "diagnostics baseline target is not a file: ",
        "invalid_configuration",
        PathAt::Tail,
    ),
    (
        "path is outside the project: ",
        "invalid_configuration",
        PathAt::Tail,
    ),
    (
        "project path is not valid UTF-8: ",
        "invalid_configuration",
        PathAt::Tail,
    ),
    (
        "cannot resolve ",
        "invalid_configuration",
        PathAt::BeforeLastColon,
    ),
    (
        "project roots collide at: ",
        "invalid_configuration",
        PathAt::Nowhere,
    ),
    (
        "invalid diagnostics baseline config: ",
        "invalid_configuration",
        PathAt::Nowhere,
    ),
    (
        "invalid diagnostics baseline group: ",
        "invalid_configuration",
        PathAt::Nowhere,
    ),
    (
        "partitioned diagnostics baseline requires a structured extension entry: ",
        "invalid_configuration",
        PathAt::Nowhere,
    ),
    (
        "invalid partitioned diagnostics baseline JSON: ",
        "invalid_set",
        PathAt::Nowhere,
    ),
    (
        "diagnostics baseline I/O error: ",
        "invalid_set",
        PathAt::Nowhere,
    ),
    (
        "unsupported manifest schema ",
        "invalid_set",
        PathAt::Nowhere,
    ),
    (
        "unsupported partition schema ",
        "invalid_set",
        PathAt::Nowhere,
    ),
    (
        "project scope does not match the manifest",
        "scope_mismatch",
        PathAt::Nowhere,
    ),
    (
        "partition identity does not match: ",
        "partition_identity_mismatch",
        PathAt::Nowhere,
    ),
    (
        "missing diagnostics baseline partitions: ",
        "missing_partition",
        PathAt::Nowhere,
    ),
    (
        "orphan diagnostics baseline partitions: ",
        "orphan_partition",
        PathAt::Nowhere,
    ),
    ("invalid partition id: ", "invalid_set", PathAt::Nowhere),
    ("invalid managed path: ", "invalid_set", PathAt::Nowhere),
    ("invalid BLAKE3 value: ", "invalid_set", PathAt::Nowhere),
    (
        "manifest generation does not match its partition hashes",
        "invalid_set",
        PathAt::Nowhere,
    ),
    (
        "partition object hash mismatch: ",
        "object_hash_mismatch",
        PathAt::Nowhere,
    ),
    ("duplicate partition: ", "invalid_set", PathAt::Nowhere),
    ("duplicate diagnostic: ", "invalid_set", PathAt::Nowhere),
    (
        "diagnostic fingerprint does not match fields: ",
        "invalid_set",
        PathAt::Nowhere,
    ),
    (
        "legacy diagnostics baseline is not in canonical order at ",
        "invalid_set",
        PathAt::Nowhere,
    ),
    (
        "protected diagnostic cannot enter a baseline: ",
        "invalid_set",
        PathAt::Nowhere,
    ),
    (
        "diagnostic path has no unique owner: ",
        "invalid_set",
        PathAt::Nowhere,
    ),
    (
        "diagnostics baseline changed while it was being loaded",
        "invalid_set",
        PathAt::Nowhere,
    ),
];

fn known_message(message: &str) -> Option<(&'static str, &'static str, PathAt)> {
    CLI_BASELINE_ERRORS
        .iter()
        .copied()
        .find(|(prefix, _, _)| message.starts_with(prefix))
}

/// The cause a CLI `analyze` names on stderr when its baseline is broken.
///
/// Upstream `main` returns the boxed detail, so the process prints
/// `Error: "<detail>"` (the `Debug` form of a string) as its last line. Only a
/// message from [`CLI_BASELINE_ERRORS`] is read as a baseline failure.
pub(crate) fn cli_baseline_failure(stderr: &str) -> Option<DiagnosticSuppressionReason> {
    let line = stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    let message = line.strip_prefix("Error: ")?;
    let message = match message.strip_prefix('"') {
        Some(quoted) => unescape_debug_str(quoted.strip_suffix('"')?)?,
        None => message.to_string(),
    };
    let (_, code, _) = known_message(&message)?;
    let code = if code == IO_CODE {
        io_code(&message)
    } else {
        code
    };
    Some(DiagnosticSuppressionReason {
        code: code.to_string(),
        detail: Some(public_detail(&message)),
    })
}

/// The section a refused CLI run publishes: nothing was analysed, so it is
/// incomplete, and the broken baseline is named as the cause. This is the same
/// shape as the resident reply's `state: "error"` branch.
pub(crate) fn broken_baseline_outcome(
    reason: DiagnosticSuppressionReason,
) -> DiagnosticProviderOutcome {
    DiagnosticProviderOutcome {
        status: DiagnosticProviderStatus::Completed,
        complete: false,
        version: None,
        observations: Vec::new(),
        rules: Vec::new(),
        readiness: None,
        error: None,
        suppressions: vec![DiagnosticSuppression::Baseline {
            known: None,
            new: None,
            reason: Some(reason),
        }],
    }
}

/// `std::io::Error` prints `(os error N)`; 2 is "not found" on Unix (ENOENT)
/// and Windows (ERROR_FILE_NOT_FOUND), 3 is ERROR_PATH_NOT_FOUND on Windows.
fn io_code(message: &str) -> &'static str {
    if message.ends_with("(os error 2)") || message.ends_with("(os error 3)") {
        "missing"
    } else {
        "unreadable"
    }
}

/// An upstream baseline detail fit for publishing: the physical path a known
/// message spells is cut to its file name whole, spaces included, and
/// secrets are redacted. A path-splitting redactor further down cannot do
/// this: it stops at the first space of `My Documents`.
pub(crate) fn public_detail(detail: &str) -> String {
    let masked = match known_message(detail) {
        Some((prefix, _, PathAt::Tail)) => {
            format!("{prefix}{}", file_name(&detail[prefix.len()..]))
        }
        Some((prefix, _, PathAt::BeforeLastColon)) => {
            let rest = &detail[prefix.len()..];
            match rest.rsplit_once(": ") {
                Some((path, cause)) => format!("{prefix}{}: {cause}", file_name(path)),
                None => format!("{prefix}{}", file_name(rest)),
            }
        }
        _ => detail.to_string(),
    };
    redactor(&masked)
}

/// The last component of a path in either separator style; the directory
/// that holds the baseline is never published.
fn file_name(path: &str) -> &str {
    path.split(['/', '\\'])
        .rev()
        .find(|part| !part.is_empty())
        .unwrap_or("<baseline path>")
}

/// Reverses `<str as Debug>::fmt`: the escapes it writes are `\\`, `\"`,
/// `\'`, `\n`, `\r`, `\t`, `\0` and `\u{…}`. Anything else is not that form.
fn unescape_debug_str(escaped: &str) -> Option<String> {
    let mut output = String::with_capacity(escaped.len());
    let mut chars = escaped.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }
        output.push(match chars.next()? {
            '\\' => '\\',
            '"' => '"',
            '\'' => '\'',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            '0' => '\0',
            'u' => {
                if chars.next()? != '{' {
                    return None;
                }
                let mut hex = String::new();
                loop {
                    match chars.next()? {
                        '}' => break,
                        digit => hex.push(digit),
                    }
                }
                char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?
            }
            _ => return None,
        });
    }
    Some(output)
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

    /// Real stderr of bsl-analyzer v0.2.86 (`analyze -s . --format jsonl -q`,
    /// release asset `bsl-analyzer-v0.2.86-build.1`, darwin-arm64) for broken
    /// baselines, captured on 2026-10-07 in a project whose path has spaces
    /// and Cyrillic; only the directory above the project is shortened. Each
    /// run exited 1 with an empty stdout: no `start`, no `done`.
    const PROJECT: &str = "/Users/dev/My Projects/Проект с пробелом";
    fn real_stderr(detail: &str) -> String {
        format!("Error: \"{}\"\n", detail.replace("{project}", PROJECT))
    }

    #[test]
    fn cli_names_every_broken_baseline_it_reports_before_start() {
        let cases = [
            (
                "cannot read diagnostics baseline {project}/base dir/missing.json: No such file or directory (os error 2)",
                "missing",
                "cannot read diagnostics baseline missing.json: No such file or directory (os error 2)",
            ),
            (
                "invalid diagnostics baseline JSON: key must be a string at line 1 column 2",
                "invalid_file",
                "invalid diagnostics baseline JSON: key must be a string at line 1 column 2",
            ),
            (
                "unsupported diagnostics baseline schema version 99; expected 1",
                "unsupported_schema",
                "unsupported diagnostics baseline schema version 99; expected 1",
            ),
            (
                "diagnostics baseline target is a symlink: {project}/link.json",
                "invalid_configuration",
                "diagnostics baseline target is a symlink: link.json",
            ),
            (
                "diagnostics baseline target is not a file: {project}/base dir",
                "invalid_configuration",
                "diagnostics baseline target is not a file: base dir",
            ),
            (
                "path is outside the project: /Users/dev/My Projects/outside.json",
                "invalid_configuration",
                "path is outside the project: outside.json",
            ),
            (
                "cannot read diagnostics baseline {project}/base dir/locked.json: Permission denied (os error 13)",
                "unreadable",
                "cannot read diagnostics baseline locked.json: Permission denied (os error 13)",
            ),
            (
                // `Debug` escapes the quotes of the partition id list.
                "missing diagnostics baseline partitions: [\\\"configuration\\\"]",
                "missing_partition",
                "missing diagnostics baseline partitions: [\"configuration\"]",
            ),
        ];
        for (stderr_detail, code, published) in cases {
            let reason = cli_baseline_failure(&real_stderr(stderr_detail))
                .unwrap_or_else(|| panic!("not recognised: {stderr_detail}"));
            assert_eq!(reason.code, code, "{stderr_detail}");
            let detail = reason.detail.unwrap();
            assert_eq!(detail, published);
            assert!(!detail.contains("My Projects"), "{detail}");
        }

        let windows = cli_baseline_failure(
            "Error: \"cannot read diagnostics baseline C:\\\\Users\\\\dev\\\\My Projects\\\\b l.json: \
             The system cannot find the path specified. (os error 3)\"",
        )
        .unwrap();
        assert_eq!(windows.code, "missing");
        assert_eq!(
            windows.detail.as_deref(),
            Some(
                "cannot read diagnostics baseline b l.json: \
                 The system cannot find the path specified. (os error 3)"
            )
        );

        // Not a baseline failure: the provider stays unexplained, not guessed.
        for stderr in [
            "",
            "Error: \"metadata is unreadable\"",
            "Error: ConfigLoadError { path: \"/p/bsl-analyzer.toml\", message: \"unknown field `mode`\" }",
            "cannot read diagnostics baseline /p/x.json: No such file or directory (os error 2)",
            "Error: \"cannot read diagnostics baseline /p/x.json\" trailing",
        ] {
            assert!(cli_baseline_failure(stderr).is_none(), "{stderr}");
        }

        // The refused run is the resident `error` branch: named, incomplete.
        let outcome =
            broken_baseline_outcome(cli_baseline_failure(&real_stderr(cases[0].0)).unwrap());
        assert!(!outcome.complete);
        assert_eq!(outcome.status, DiagnosticProviderStatus::Completed);
        assert!(outcome.error.is_none() && outcome.observations.is_empty());
        let suppression = outcome.suppressions.into_iter().next().unwrap();
        assert_eq!(
            (
                match &suppression {
                    crate::domain::diagnostics::DiagnosticSuppression::Baseline {
                        known, ..
                    } => *known,
                    _ => panic!("expected baseline fact"),
                },
                match &suppression {
                    crate::domain::diagnostics::DiagnosticSuppression::Baseline { new, .. } => *new,
                    _ => panic!("expected baseline fact"),
                }
            ),
            (None, None)
        );
        assert_eq!(suppression.reason().cloned().unwrap().code, "missing");
    }

    /// A path with spaces outside every known physical root: the whole path,
    /// not the part up to its first space, leaves the published detail.
    #[test]
    fn baseline_detail_hides_a_spaced_physical_path_whole() {
        let verdict = baseline_verdict(BaselineFacts {
            error_code: Some("missing"),
            detail: Some(
                "cannot read diagnostics baseline /Volumes/Shared Drive/team baselines/main.json: \
                 No such file or directory (os error 2)",
            ),
            ..facts(BaselineState::Error)
        });
        let detail = verdict
            .suppression
            .unwrap()
            .reason()
            .cloned()
            .unwrap()
            .detail
            .unwrap();
        assert_eq!(
            detail,
            "cannot read diagnostics baseline main.json: No such file or directory (os error 2)"
        );
        for fragment in ["Volumes", "Shared", "Drive", "team baselines"] {
            assert!(!detail.contains(fragment), "{detail}");
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
            assert_eq!(
                serde_json::to_value(&suppression).unwrap()["by"],
                "baseline"
            );
            assert_eq!(
                (
                    match &suppression {
                        crate::domain::diagnostics::DiagnosticSuppression::Baseline {
                            known,
                            ..
                        } => *known,
                        _ => panic!("expected baseline fact"),
                    },
                    match &suppression {
                        crate::domain::diagnostics::DiagnosticSuppression::Baseline {
                            new, ..
                        } => *new,
                        _ => panic!("expected baseline fact"),
                    }
                ),
                (Some(3), Some(1))
            );
            assert!(suppression.reason().is_none(), "{state:?}");
        }

        // Partial without `known`: the classification was interrupted.
        let interrupted = baseline_verdict(facts(BaselineState::Partial));
        assert!(!interrupted.intact);
        let reason = interrupted.suppression.unwrap().reason().cloned().unwrap();
        assert_eq!(reason.code, "baseline_interrupted");
        assert_eq!(reason.detail.as_deref(), Some(INTERRUPTED_DETAIL));

        // Error state: the analyzer's own code and detail, secrets redacted.
        let broken = baseline_verdict(BaselineFacts {
            error_code: Some("invalid_path"),
            detail: Some("baseline path escapes the project; Pwd=hunter2"),
            ..facts(BaselineState::Error)
        });
        assert!(!broken.intact);
        let reason = broken.suppression.unwrap().reason().cloned().unwrap();
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
            partition
                .suppression
                .unwrap()
                .reason()
                .cloned()
                .unwrap()
                .code,
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
        let reason = coded.suppression.unwrap().reason().cloned().unwrap();
        assert_eq!(reason.code, "baseline_error");
        assert!(reason.detail.is_none());
    }
}
