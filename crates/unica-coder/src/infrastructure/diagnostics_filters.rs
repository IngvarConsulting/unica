//! Coverage limitations of the pinned analyzer's CLI JSONL protocol.
//! JSONL has no out-of-scope/author counters. Configuration precedence and
//! scope precedence below mirror bsl-analyzer 0.2.86; a module diff-filter
//! overrides configured diff_base, but never overrides ignored_authors.
use crate::domain::diagnostics::{
    DiagnosticError, DiagnosticObservation, DiagnosticObservationLocation,
    DiagnosticProviderOutcome, DiagnosticProviderStatus, BSL_ANALYZER_PROVIDER,
};
use serde::Deserialize;
use std::path::Path;

#[derive(Default, Deserialize)]
struct TomlConfig {
    #[serde(default)]
    analysis: TomlAnalysis,
}
#[derive(Default, Deserialize)]
struct TomlAnalysis {
    diff_base: Option<String>,
    #[serde(default)]
    ignored_authors: Vec<String>,
}
#[derive(Default, Deserialize)]
struct JsonConfig {
    #[serde(default)]
    analysis: JsonAnalysis,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonAnalysis {
    diff_base: Option<String>,
    #[serde(default)]
    ignored_authors: Vec<String>,
}

/// Only safe named causes leave this module; config paths, refs and author
/// identities stay private. Malformed/changed config cannot establish coverage.
pub(crate) fn cli_filter_errors(
    source: &Path,
    explicit_config: Option<&Path>,
    whole_module_scope: bool,
) -> Vec<DiagnosticError> {
    let read = || -> Result<(bool, bool), ()> {
        let path = explicit_config.map(Path::to_path_buf).or_else(|| {
            [
                "bsl-analyzer.toml",
                ".bsl-analyzer.json",
                ".bsl-language-server.json",
            ]
            .into_iter()
            .map(|name| source.join(name))
            .find(|path| path.exists())
        });
        let Some(path) = path else {
            return Ok((false, false));
        };
        let text = std::fs::read_to_string(&path).map_err(|_| ())?;
        if path
            .extension()
            .is_some_and(|extension| extension == "toml")
        {
            let config: TomlConfig = toml::from_str(&text).map_err(|_| ())?;
            Ok((
                config.analysis.diff_base.is_some(),
                !config.analysis.ignored_authors.is_empty(),
            ))
        } else {
            let config: JsonConfig = serde_json::from_str(&text).map_err(|_| ())?;
            Ok((
                config.analysis.diff_base.is_some(),
                !config.analysis.ignored_authors.is_empty(),
            ))
        }
    };
    let error = |code: &str, message: &str| DiagnosticError {
        code: code.to_string(),
        message: message.to_string(),
        retryable: false,
    };
    match read() {
        Err(()) => vec![error(
            "analysis_config_unproven",
            "the analyzer's effective filtering configuration could not be established",
        )],
        Ok((diff, authors)) => {
            let mut errors = Vec::new();
            if diff && !whole_module_scope {
                errors.push(error("analysis_scope_unproven", "CLI diff filtering is configured; this analyzer does not prove coverage of the selected resources"));
            }
            if authors {
                errors.push(error("analysis_filter_unproven", "CLI author filtering is configured; this analyzer does not report which findings it hid"));
            }
            errors
        }
    }
}

/// Preserve proven findings and baseline facts; lack of filter evidence adds
/// a resource failure instead of converting those findings to a clean report.
pub(crate) fn mark_cli_filter_limits(
    outcome: &mut DiagnosticProviderOutcome,
    module: Option<&Path>,
    errors: Vec<DiagnosticError>,
) {
    if errors.is_empty()
        || !matches!(
            outcome.status,
            DiagnosticProviderStatus::Completed | DiagnosticProviderStatus::Empty
        )
    {
        return;
    }
    outcome.status = DiagnosticProviderStatus::Completed;
    outcome.complete = false;
    for error in errors {
        outcome
            .observations
            .push(DiagnosticObservation::ResourceFailure {
                provider: BSL_ANALYZER_PROVIDER,
                location: match module {
                    Some(module) => DiagnosticObservationLocation::Resource {
                        handle: module.display().to_string(),
                    },
                    None => DiagnosticObservationLocation::Logical {
                        metadata_path: None,
                    },
                },
                error,
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn effective_cli_config_matches_pinned_precedence_and_module_scope() {
        let root = tempfile::tempdir().unwrap();
        let errors = |module| {
            cli_filter_errors(root.path(), None, module)
                .into_iter()
                .map(|e| e.code)
                .collect::<Vec<_>>()
        };
        assert!(errors(false).is_empty());
        std::fs::write(root.path().join(".bsl-language-server.json"), r#"{"analysis":{"diffBase":"private-ref","ignoredAuthors":["private@example.invalid"]}}"#).unwrap();
        assert_eq!(
            errors(false),
            ["analysis_scope_unproven", "analysis_filter_unproven"]
        );
        assert_eq!(errors(true), ["analysis_filter_unproven"]);
        std::fs::write(
            root.path().join(".bsl-analyzer.json"),
            r#"{"analysis":{"diffBase":"HEAD"}}"#,
        )
        .unwrap();
        assert_eq!(errors(false), ["analysis_scope_unproven"]);
        assert!(errors(true).is_empty());
        std::fs::write(root.path().join("bsl-analyzer.toml"), "[analysis]\n").unwrap();
        assert!(errors(false).is_empty(), "TOML overrides both JSON files");
        std::fs::write(
            root.path().join("bsl-analyzer.toml"),
            "[analysis]\ndiff_base='HEAD'\nignored_authors=['private@example.invalid']\n",
        )
        .unwrap();
        assert_eq!(
            errors(false),
            ["analysis_scope_unproven", "analysis_filter_unproven"]
        );
        assert_eq!(errors(true), ["analysis_filter_unproven"]);
        std::fs::write(root.path().join("bsl-analyzer.toml"), "invalid {{{").unwrap();
        assert_eq!(
            errors(true),
            ["analysis_config_unproven"],
            "invalid highest-priority config must not fall back"
        );
        let override_path = root.path().join("explicit.json");
        std::fs::write(&override_path, "{}").unwrap();
        assert!(cli_filter_errors(root.path(), Some(&override_path), false).is_empty());
        std::fs::write(
            &override_path,
            r#"{"analysis":{"ignoredAuthors":"not-an-array"}}"#,
        )
        .unwrap();
        let invalid = cli_filter_errors(root.path(), Some(&override_path), false);
        assert_eq!(invalid[0].code, "analysis_config_unproven");
        assert!(!invalid[0].message.contains("private"));
    }

    #[test]
    fn cli_missing_filter_counts_preserves_findings_baseline_and_failure_states() {
        use crate::domain::diagnostics::DiagnosticSuppression;
        let mut outcome = DiagnosticProviderOutcome::empty(DiagnosticProviderStatus::Completed);
        outcome.suppressions.push(DiagnosticSuppression::Baseline {
            known: Some(1),
            new: Some(2),
            reason: None,
        });
        let original = outcome.suppressions.clone();
        let finding = DiagnosticObservation::Diagnostic {
            provider: BSL_ANALYZER_PROVIDER,
            location: DiagnosticObservationLocation::Resource {
                handle: "/fixture/Module.bsl".to_string(),
            },
            focus: crate::domain::diagnostics::DiagnosticObservationFocus::Target,
            code: "UnusedLocalVariable".to_string(),
            severity: crate::domain::diagnostics::DiagnosticSeverity::Warning,
            message: "unused variable".to_string(),
            tags: Vec::new(),
        };
        outcome.observations.push(finding.clone());
        let errors = vec![DiagnosticError {
            code: "analysis_filter_unproven".to_string(),
            message: "safe cause".to_string(),
            retryable: false,
        }];
        mark_cli_filter_limits(
            &mut outcome,
            Some(Path::new("/fixture/Module.bsl")),
            errors.clone(),
        );
        assert!(!outcome.complete);
        assert_eq!(outcome.status, DiagnosticProviderStatus::Completed);
        assert_eq!(outcome.suppressions, original);
        assert_eq!(outcome.observations.len(), 2);
        assert_eq!(outcome.observations[0], finding);
        assert!(
            matches!(&outcome.observations[1],DiagnosticObservation::ResourceFailure {error,..} if error.code=="analysis_filter_unproven")
        );
        let mut empty = DiagnosticProviderOutcome::empty(DiagnosticProviderStatus::Empty);
        mark_cli_filter_limits(&mut empty, None, errors.clone());
        assert!(!empty.complete);
        assert_eq!(empty.status, DiagnosticProviderStatus::Completed);
        assert!(empty.suppressions.is_empty());
        for status in [
            DiagnosticProviderStatus::Failed,
            DiagnosticProviderStatus::Unavailable,
            DiagnosticProviderStatus::Unsupported,
        ] {
            let mut failure = DiagnosticProviderOutcome::empty(status);
            failure.error = Some(DiagnosticError {
                code: "original_failure".to_string(),
                message: "original".to_string(),
                retryable: true,
            });
            let before = failure.clone();
            mark_cli_filter_limits(&mut failure, None, errors.clone());
            assert_eq!(failure, before);
        }
    }
}
