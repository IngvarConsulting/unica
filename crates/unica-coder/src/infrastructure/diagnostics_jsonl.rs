use crate::domain::diagnostics::{
    DiagnosticError, DiagnosticObservation, DiagnosticObservationFocus,
    DiagnosticObservationLocation, DiagnosticProviderOutcome, DiagnosticProviderStatus,
    DiagnosticRange, DiagnosticSeverity, DiagnosticTag, BSL_ANALYZER_PROVIDER,
};
use crate::infrastructure::redaction::redactor;
use crate::infrastructure::source_roots::normalize_path_identity;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

pub(crate) const MAX_DIAGNOSTICS_JSONL_LINE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnalyzerDiagnosticsFileTotals {
    pub(crate) discovered: Option<usize>,
    pub(crate) processed: Option<usize>,
    pub(crate) failed: Option<usize>,
}

/// Typed result of the analyzer JSONL protocol. This type is private to the
/// provider boundary: physical resource handles remain in observations until
/// the common diagnostics mapper proves their logical addresses.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AnalyzerDiagnosticsBatch {
    pub(crate) outcome: DiagnosticProviderOutcome,
    pub(crate) files: AnalyzerDiagnosticsFileTotals,
    pub(crate) diagnostics_reported: Option<usize>,
    pub(crate) elapsed_seconds: Option<f64>,
}

#[derive(Debug)]
pub(crate) struct DiagnosticsJsonlParser {
    source_root: PathBuf,
    first_error: Option<String>,
    started: bool,
    done: bool,
    version: Option<String>,
    discovered: Option<usize>,
    files_seen: BTreeSet<String>,
    diagnostics_seen: usize,
    failures_seen: usize,
    observations: Vec<DiagnosticObservation>,
    elapsed_seconds: Option<f64>,
    reported: Option<usize>,
    done_files: Option<usize>,
    done_failures: Option<usize>,
}

impl DiagnosticsJsonlParser {
    pub(crate) fn new(source_root: &Path) -> Result<Self, String> {
        Ok(Self {
            source_root: normalize_absolute_root(source_root)?,
            first_error: None,
            started: false,
            done: false,
            version: None,
            discovered: None,
            files_seen: BTreeSet::new(),
            diagnostics_seen: 0,
            failures_seen: 0,
            observations: Vec::new(),
            elapsed_seconds: None,
            reported: None,
            done_files: None,
            done_failures: None,
        })
    }

    pub(crate) fn push_line(&mut self, line_number: usize, bytes: &[u8]) {
        if self.first_error.is_some() {
            return;
        }
        if bytes.len() > MAX_DIAGNOSTICS_JSONL_LINE_BYTES {
            self.reject_line(line_number, "line exceeds 8388608 bytes");
            return;
        }
        let line = match std::str::from_utf8(bytes) {
            Ok(line) => line.trim_end_matches(['\r', '\n']),
            Err(_) => {
                self.reject_line(line_number, "line is not valid UTF-8");
                return;
            }
        };
        if line.trim().is_empty() {
            self.reject_line(line_number, "line is empty");
            return;
        }
        let event = match serde_json::from_str::<JsonlEvent>(line) {
            Ok(event) => event,
            Err(error) => {
                self.reject_line(line_number, &format!("invalid event: {error}"));
                return;
            }
        };
        if let Err(error) = self.accept_event(event) {
            self.reject_line(line_number, &error);
        }
    }

    pub(crate) fn reject_line(&mut self, line_number: usize, reason: &str) {
        if self.first_error.is_none() {
            self.first_error = Some(format!("line {line_number}: {reason}"));
        }
    }

    pub(crate) fn finish(mut self) -> AnalyzerDiagnosticsBatch {
        if self.first_error.is_none() && self.done {
            if let Err(error) = self.validate_totals() {
                self.first_error = Some(error);
            }
        }
        if let Some(message) = self.first_error.take() {
            return self.failure(
                DiagnosticProviderStatus::Failed,
                "diagnostics_invalid",
                false,
                message,
            );
        }
        if !self.started {
            return self.failure(
                DiagnosticProviderStatus::Failed,
                "diagnostics_invalid",
                false,
                "line 0: stream is missing start event".to_string(),
            );
        }
        if !self.done {
            if self.files_seen.is_empty() {
                return self.failure(
                    DiagnosticProviderStatus::Unavailable,
                    "diagnostics_pending",
                    true,
                    "bsl-analyzer emitted start but did not report files or a terminal event"
                        .to_string(),
                );
            }
            return self.failure(
                DiagnosticProviderStatus::Failed,
                "diagnostics_incomplete",
                false,
                "bsl-analyzer stream ended after file events without a terminal event".to_string(),
            );
        }

        let failed = self.done_failures.expect("validated done event");
        let discovered = self.discovered.expect("validated start event");
        let status = if self.observations.is_empty() {
            DiagnosticProviderStatus::Empty
        } else {
            DiagnosticProviderStatus::Completed
        };
        AnalyzerDiagnosticsBatch {
            outcome: DiagnosticProviderOutcome {
                status,
                complete: failed == 0,
                version: self.version,
                observations: self.observations,
                rules: Vec::new(),
                readiness: None,
                error: None,
            },
            files: AnalyzerDiagnosticsFileTotals {
                discovered: Some(discovered),
                processed: Some(discovered - failed),
                failed: Some(failed),
            },
            diagnostics_reported: self.reported,
            elapsed_seconds: self.elapsed_seconds,
        }
    }

    fn accept_event(&mut self, event: JsonlEvent) -> Result<(), String> {
        if self.done {
            return Err("event appeared after terminal done".to_string());
        }
        match event {
            JsonlEvent::Start(event) => {
                if self.started {
                    return Err("duplicate start event".to_string());
                }
                if event.version.trim().is_empty() {
                    return Err("start.version must be non-empty".to_string());
                }
                self.started = true;
                self.discovered = Some(event.total_files);
                self.version = Some(event.version);
            }
            JsonlEvent::File(event) => {
                if !self.started {
                    return Err("file event appeared before start".to_string());
                }
                let path = normalize_reported_path(&self.source_root, &event.path)?;
                if !self.files_seen.insert(path.clone()) {
                    return Err(format!("duplicate normalized file path `{path}`"));
                }
                if let Some(error) = event.error {
                    if error.trim().is_empty() {
                        return Err("file.error must be non-empty".to_string());
                    }
                    if !event.diagnostics.is_empty() || event.metrics.is_some() {
                        return Err(
                            "file.error is mutually exclusive with diagnostics and metrics"
                                .to_string(),
                        );
                    }
                    self.failures_seen += 1;
                    self.observations
                        .push(DiagnosticObservation::ResourceFailure {
                            provider: BSL_ANALYZER_PROVIDER,
                            location: DiagnosticObservationLocation::Resource { handle: path },
                            error: DiagnosticError {
                                code: "source_analysis_failed".to_string(),
                                message: redactor(&error),
                                retryable: false,
                            },
                        });
                    return Ok(());
                }
                for diagnostic in event.diagnostics {
                    self.diagnostics_seen += 1;
                    let (severity, tags) = diagnostic.validate()?;
                    self.observations.push(DiagnosticObservation::Diagnostic {
                        provider: BSL_ANALYZER_PROVIDER,
                        location: DiagnosticObservationLocation::Resource {
                            handle: path.clone(),
                        },
                        focus: DiagnosticObservationFocus::SourceRange(DiagnosticRange {
                            start_line: diagnostic.start_line,
                            start_column: diagnostic.start_column,
                            end_line: diagnostic.end_line,
                            end_column: diagnostic.end_column,
                        }),
                        code: diagnostic.code,
                        severity,
                        message: diagnostic.message,
                        tags,
                    });
                }
            }
            JsonlEvent::Done(event) => {
                if !self.started {
                    return Err("done event appeared before start".to_string());
                }
                if !event.elapsed_secs.is_finite() || event.elapsed_secs < 0.0 {
                    return Err("done.elapsed_secs must be finite and non-negative".to_string());
                }
                self.done = true;
                self.elapsed_seconds = Some(event.elapsed_secs);
                self.reported = Some(event.total_diagnostics);
                self.done_files = Some(event.total_files);
                self.done_failures = Some(event.failed_files);
            }
        }
        Ok(())
    }

    fn validate_totals(&self) -> Result<(), String> {
        let discovered = self.discovered.expect("start required before done");
        let done_files = self.done_files.expect("done total_files recorded");
        let reported = self.reported.expect("done total_diagnostics recorded");
        let failed = self.done_failures.expect("done failed_files recorded");
        if done_files != discovered || self.files_seen.len() != discovered {
            return Err(format!(
                "file totals disagree: start={discovered}, events={}, done={done_files}",
                self.files_seen.len()
            ));
        }
        if reported != self.diagnostics_seen {
            return Err(format!(
                "diagnostic totals disagree: events={}, done={reported}",
                self.diagnostics_seen
            ));
        }
        if failed != self.failures_seen || failed > discovered {
            return Err(format!(
                "failed file totals disagree: events={}, done={failed}",
                self.failures_seen
            ));
        }
        Ok(())
    }

    fn failure(
        self,
        status: DiagnosticProviderStatus,
        code: &'static str,
        retryable: bool,
        message: String,
    ) -> AnalyzerDiagnosticsBatch {
        AnalyzerDiagnosticsBatch {
            outcome: DiagnosticProviderOutcome {
                status,
                complete: false,
                version: self.version,
                observations: Vec::new(),
                rules: Vec::new(),
                readiness: None,
                error: Some(DiagnosticError {
                    code: code.to_string(),
                    message,
                    retryable,
                }),
            },
            files: AnalyzerDiagnosticsFileTotals {
                discovered: self.discovered,
                processed: None,
                failed: None,
            },
            diagnostics_reported: None,
            elapsed_seconds: None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum JsonlEvent {
    #[serde(rename = "start")]
    Start(StartEvent),
    #[serde(rename = "file")]
    File(FileEvent),
    #[serde(rename = "done")]
    Done(DoneEvent),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StartEvent {
    total_files: usize,
    version: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileEvent {
    path: String,
    diagnostics: Vec<UpstreamDiagnostic>,
    #[allow(dead_code)]
    metrics: Option<FileMetrics>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileMetrics {
    #[allow(dead_code)]
    functions: usize,
    #[allow(dead_code)]
    complexity: u32,
    #[allow(dead_code)]
    cognitive_complexity: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DoneEvent {
    elapsed_secs: f64,
    total_files: usize,
    total_diagnostics: usize,
    failed_files: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpstreamDiagnostic {
    code: String,
    message: String,
    severity: String,
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
    #[serde(default)]
    tags: Vec<UpstreamTag>,
}

impl UpstreamDiagnostic {
    fn validate(&self) -> Result<(DiagnosticSeverity, Vec<DiagnosticTag>), String> {
        if self.code.trim().is_empty() {
            return Err("diagnostic.code must be non-empty".to_string());
        }
        if self.message.trim().is_empty() {
            return Err("diagnostic.message must be non-empty".to_string());
        }
        let severity = map_severity(&self.severity)?;
        if (self.end_line, self.end_column) < (self.start_line, self.start_column) {
            return Err("diagnostic range end precedes its start".to_string());
        }
        let mut unique_tags = BTreeSet::new();
        let mut tags = Vec::with_capacity(self.tags.len());
        for tag in &self.tags {
            if !unique_tags.insert(*tag) {
                return Err("diagnostic tags must be unique".to_string());
            }
            tags.push((*tag).into());
        }
        Ok((severity, tags))
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
enum UpstreamTag {
    Unnecessary,
    Deprecated,
}

impl UpstreamTag {
    const fn into(self) -> DiagnosticTag {
        match self {
            Self::Unnecessary => DiagnosticTag::Unnecessary,
            Self::Deprecated => DiagnosticTag::Deprecated,
        }
    }
}

fn map_severity(value: &str) -> Result<DiagnosticSeverity, String> {
    match value {
        "Blocker" | "Critical" | "Major" | "Error" => Ok(DiagnosticSeverity::Error),
        "Warning" => Ok(DiagnosticSeverity::Warning),
        "Information" => Ok(DiagnosticSeverity::Info),
        "Hint" => Ok(DiagnosticSeverity::Hint),
        _ => Err(format!("unknown diagnostic severity `{value}`")),
    }
}

fn normalize_absolute_root(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!(
            "diagnostics source root must be absolute: {}",
            path.display()
        ));
    }
    normalize_path_identity(&normalize_lexical(path)?)
}

fn normalize_reported_path(source_root: &Path, raw: &str) -> Result<String, String> {
    if raw.trim().is_empty() {
        return Err("file.path must be non-empty".to_string());
    }
    let path = Path::new(raw);
    let candidate = if path.is_absolute() {
        normalize_lexical(path).map_err(|_| "file.path contains invalid traversal".to_string())?
    } else {
        normalize_lexical(&source_root.join(path))
            .map_err(|_| "file.path contains invalid traversal".to_string())?
    };
    let identity = normalize_path_identity(&candidate)
        .map_err(|_| "file.path could not be resolved safely".to_string())?;
    let relative = identity
        .strip_prefix(source_root)
        .map_err(|_| "file.path resolves outside diagnostics source root".to_string())?;
    if relative.as_os_str().is_empty() {
        return Err("file.path must name a file below the diagnostics source root".to_string());
    }
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn normalize_lexical(path: &Path) -> Result<PathBuf, String> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!(
                    "path contains parent traversal: {}",
                    path.display()
                ));
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::diagnostics::{
        DiagnosticObservation, DiagnosticObservationFocus, DiagnosticObservationLocation,
        DiagnosticProviderStatus, DiagnosticRange, DiagnosticSeverity, DiagnosticTag,
        BSL_ANALYZER_PROVIDER,
    };
    use crate::infrastructure::platform::testing::{
        create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    use serde_json::json;
    use tempfile::TempDir;

    fn parser() -> DiagnosticsJsonlParser {
        let source_root = std::env::temp_dir().join("unica-diagnostics-jsonl-tests");
        DiagnosticsJsonlParser::new(&source_root).unwrap()
    }

    fn feed(parser: &mut DiagnosticsJsonlParser, lines: &[&str]) {
        for (index, line) in lines.iter().enumerate() {
            parser.push_line(index + 1, line.as_bytes());
        }
    }

    fn diagnostic(code: &str, severity: &str, line: usize) -> String {
        json!({
            "type": "file",
            "path": "CommonModules/Sales/Ext/Module.bsl",
            "diagnostics": [{
                "code": code,
                "message": "Line too long",
                "severity": severity,
                "start_line": line,
                "start_column": 0,
                "end_line": line,
                "end_column": 150,
                "tags": ["Unnecessary"]
            }]
        })
        .to_string()
    }

    #[test]
    fn complete_stream_projects_typed_data_without_upstream_shape() {
        let file = diagnostic("LineLength", "Warning", 10);
        let mut parser = parser();
        feed(
            &mut parser,
            &[
                r#"{"type":"start","total_files":1,"version":"0.2.62"}"#,
                &file,
                r#"{"type":"done","elapsed_secs":0.4,"total_files":1,"total_diagnostics":1,"failed_files":0}"#,
            ],
        );

        let batch = parser.finish();
        assert_eq!(batch.outcome.status, DiagnosticProviderStatus::Completed);
        assert!(batch.outcome.complete);
        assert_eq!(batch.outcome.version.as_deref(), Some("0.2.62"));
        assert_eq!(batch.files.discovered, Some(1));
        assert_eq!(batch.files.processed, Some(1));
        assert_eq!(batch.files.failed, Some(0));
        assert_eq!(batch.diagnostics_reported, Some(1));
        assert_eq!(batch.elapsed_seconds, Some(0.4));
        assert_eq!(
            batch.outcome.observations,
            vec![DiagnosticObservation::Diagnostic {
                provider: BSL_ANALYZER_PROVIDER,
                location: DiagnosticObservationLocation::Resource {
                    handle: "CommonModules/Sales/Ext/Module.bsl".to_string(),
                },
                focus: DiagnosticObservationFocus::SourceRange(DiagnosticRange {
                    start_line: 10,
                    start_column: 0,
                    end_line: 10,
                    end_column: 150,
                }),
                code: "LineLength".to_string(),
                severity: DiagnosticSeverity::Warning,
                message: "Line too long".to_string(),
                tags: vec![DiagnosticTag::Unnecessary],
            }]
        );
    }

    #[test]
    fn parser_keeps_all_observations_and_redacts_resource_failures() {
        let mut parser = parser();
        let keep = diagnostic("Keep", "Information", 3);
        let drop =
            diagnostic("Drop", "Blocker", 1).replace("CommonModules/Sales", "CommonModules/Drop");
        feed(
            &mut parser,
            &[
                r#"{"type":"start","total_files":3,"version":"0.2.62"}"#,
                &keep,
                r#"{"type":"file","path":"ZCatalogs/Broken/Ext/Module.bsl","diagnostics":[],"error":"Pwd=secret parse failed"}"#,
                &drop,
                r#"{"type":"done","elapsed_secs":1.0,"total_files":3,"total_diagnostics":2,"failed_files":1}"#,
            ],
        );

        let batch = parser.finish();
        assert_eq!(batch.outcome.status, DiagnosticProviderStatus::Completed);
        assert_eq!(batch.diagnostics_reported, Some(2));
        assert_eq!(batch.outcome.observations.len(), 3);
        assert!(batch
            .outcome
            .observations
            .iter()
            .any(|observation| matches!(
                observation,
                DiagnosticObservation::Diagnostic { code, severity: DiagnosticSeverity::Info, .. }
                    if code == "Keep"
            )));
        assert!(batch
            .outcome
            .observations
            .iter()
            .any(|observation| matches!(
                observation,
                DiagnosticObservation::Diagnostic { code, severity: DiagnosticSeverity::Error, .. }
                    if code == "Drop"
            )));
        let failure = batch
            .outcome
            .observations
            .iter()
            .find_map(|observation| match observation {
                DiagnosticObservation::ResourceFailure { error, .. } => Some(error),
                _ => None,
            })
            .expect("resource failure");
        assert_eq!(failure.code, "source_analysis_failed");
        assert!(!failure.message.contains("secret"));
    }

    #[test]
    fn only_start_is_pending_and_file_without_done_is_incomplete() {
        let mut pending = parser();
        feed(
            &mut pending,
            &[r#"{"type":"start","total_files":1,"version":"0.2.62"}"#],
        );
        let pending = pending.finish();
        assert_eq!(
            pending.outcome.status,
            DiagnosticProviderStatus::Unavailable
        );
        assert_eq!(
            pending.outcome.error.as_ref().unwrap().code,
            "diagnostics_pending"
        );
        assert!(pending.outcome.error.as_ref().unwrap().retryable);
        assert!(pending.outcome.observations.is_empty());

        let mut incomplete = parser();
        feed(
            &mut incomplete,
            &[
                r#"{"type":"start","total_files":1,"version":"0.2.62"}"#,
                r#"{"type":"file","path":"Module.bsl","diagnostics":[]}"#,
            ],
        );
        let incomplete = incomplete.finish();
        assert_eq!(
            incomplete.outcome.error.as_ref().unwrap().code,
            "diagnostics_incomplete"
        );
        assert_eq!(incomplete.outcome.status, DiagnosticProviderStatus::Failed);
        assert!(!incomplete.outcome.error.as_ref().unwrap().retryable);
        assert!(incomplete.outcome.observations.is_empty());
    }

    #[test]
    fn invalid_grammar_and_totals_fail_closed_without_partial_items() {
        let cases: Vec<Vec<&str>> = vec![
            vec![r#"{"type":"wat"}"#],
            vec![r#"{"type":"start","total_files":0,"version":"0.2.62","extra":true}"#],
            vec![r#"{"type":"start","total_files":0,"version":""}"#],
            vec![
                r#"{"type":"start","total_files":1,"version":"0.2.62"}"#,
                r#"{"type":"file","path":"../escape.bsl","diagnostics":[]}"#,
            ],
            vec![
                r#"{"type":"start","total_files":0,"version":"0.2.62"}"#,
                r#"{"type":"done","elapsed_secs":-1.0,"total_files":0,"total_diagnostics":0,"failed_files":0}"#,
            ],
            vec![
                r#"{"type":"start","total_files":0,"version":"0.2.62"}"#,
                r#"{"type":"done","elapsed_secs":0.0,"total_files":1,"total_diagnostics":0,"failed_files":0}"#,
            ],
        ];
        for lines in cases {
            let mut parser = parser();
            feed(&mut parser, &lines);
            let result = parser.finish();
            assert_eq!(
                result.outcome.error.as_ref().unwrap().code,
                "diagnostics_invalid",
                "{lines:?}"
            );
            assert_eq!(result.outcome.status, DiagnosticProviderStatus::Failed);
            assert!(result.outcome.observations.is_empty());
        }
    }

    #[test]
    fn diagnostic_validation_rejects_unknown_severity_tags_and_range() {
        for diagnostic in [
            json!({"code":"X","message":"m","severity":"Notice","start_line":0,"start_column":0,"end_line":0,"end_column":0,"tags":[]}),
            json!({"code":"X","message":"m","severity":"Warning","start_line":2,"start_column":0,"end_line":1,"end_column":0,"tags":[]}),
            json!({"code":"X","message":"m","severity":"Warning","start_line":0,"start_column":0,"end_line":0,"end_column":0,"tags":["Unknown"]}),
            json!({"code":"X","message":"m","severity":"Warning","start_line":0,"start_column":0,"end_line":0,"end_column":0,"tags":["Deprecated","Deprecated"]}),
        ] {
            let file =
                json!({"type":"file","path":"Module.bsl","diagnostics":[diagnostic]}).to_string();
            let mut parser = parser();
            feed(
                &mut parser,
                &[
                    r#"{"type":"start","total_files":1,"version":"0.2.62"}"#,
                    &file,
                ],
            );
            assert_eq!(
                parser.finish().outcome.error.unwrap().code,
                "diagnostics_invalid"
            );
        }
    }

    #[test]
    fn duplicate_start_done_and_normalized_path_are_invalid() {
        for lines in [
            vec![
                r#"{"type":"start","total_files":0,"version":"0.2.62"}"#,
                r#"{"type":"start","total_files":0,"version":"0.2.62"}"#,
            ],
            vec![
                r#"{"type":"start","total_files":1,"version":"0.2.62"}"#,
                r#"{"type":"file","path":"Module.bsl","diagnostics":[]}"#,
                r#"{"type":"file","path":"./Module.bsl","diagnostics":[]}"#,
            ],
            vec![
                r#"{"type":"start","total_files":0,"version":"0.2.62"}"#,
                r#"{"type":"done","elapsed_secs":0.0,"total_files":0,"total_diagnostics":0,"failed_files":0}"#,
                r#"{"type":"done","elapsed_secs":0.0,"total_files":0,"total_diagnostics":0,"failed_files":0}"#,
            ],
        ] {
            let mut parser = parser();
            feed(&mut parser, &lines);
            assert_eq!(
                parser.finish().outcome.error.unwrap().code,
                "diagnostics_invalid"
            );
        }
    }

    #[test]
    fn line_length_failure_names_the_physical_line_without_copying_it() {
        let mut parser = parser();
        parser.reject_line(7, "line exceeds 8388608 bytes");
        let result = parser.finish();
        let error = result.outcome.error.unwrap();
        assert_eq!(error.code, "diagnostics_invalid");
        assert!(error.message.contains("line 7"));
        assert!(!error.message.contains("sensitive contents"));
    }

    #[test]
    fn reported_path_cannot_escape_the_source_root_through_a_symlink() {
        let fixture = TempDir::new().unwrap();
        let source_root = fixture.path().join("source");
        let outside = fixture.path().join("outside");
        std::fs::create_dir_all(&source_root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(
            outside.join("Module.bsl"),
            "Procedure Outside()\nEndProcedure",
        )
        .unwrap();
        if create_directory_link_fixture_for_test(&outside, source_root.join("escape")).unwrap()
            != FileLinkFixtureOutcome::Created
        {
            return;
        }

        let mut parser = DiagnosticsJsonlParser::new(&source_root).unwrap();
        feed(
            &mut parser,
            &[
                r#"{"type":"start","total_files":1,"version":"0.2.62"}"#,
                r#"{"type":"file","path":"escape/Module.bsl","diagnostics":[]}"#,
                r#"{"type":"done","elapsed_secs":0.0,"total_files":1,"total_diagnostics":0,"failed_files":0}"#,
            ],
        );

        let result = parser.finish();
        assert_eq!(result.outcome.error.unwrap().code, "diagnostics_invalid");
        assert!(result.outcome.observations.is_empty());
    }

    #[test]
    fn absolute_outside_path_is_rejected_without_copying_physical_handles() {
        let fixture = TempDir::new().unwrap();
        let source_root = fixture.path().join("source");
        let outside = fixture.path().join("outside/Secret.bsl");
        std::fs::create_dir_all(&source_root).unwrap();
        std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
        std::fs::write(&outside, "Procedure Secret()\nEndProcedure").unwrap();
        let event = json!({"type":"file","path":outside,"diagnostics":[]}).to_string();
        let mut parser = DiagnosticsJsonlParser::new(&source_root).unwrap();
        feed(
            &mut parser,
            &[
                r#"{"type":"start","total_files":1,"version":"0.2.62"}"#,
                &event,
            ],
        );

        let error = parser.finish().outcome.error.unwrap();

        assert_eq!(error.code, "diagnostics_invalid");
        assert_eq!(
            error.message,
            "line 2: file.path resolves outside diagnostics source root"
        );
        assert!(!error
            .message
            .contains(&source_root.to_string_lossy().as_ref()));
        assert!(!error.message.contains(&outside.to_string_lossy().as_ref()));
    }

    #[test]
    fn invalid_utf8_fails_closed_without_copying_input() {
        let mut parser = parser();
        parser.push_line(3, &[0xff, b's', b'e', b'c', b'r', b'e', b't']);

        let batch = parser.finish();
        let error = batch.outcome.error.unwrap();
        assert_eq!(error.code, "diagnostics_invalid");
        assert!(error.message.contains("line 3"));
        assert!(!error.message.contains("secret"));
        assert!(batch.outcome.observations.is_empty());
    }

    #[test]
    fn cyrillic_path_is_preserved_as_a_private_resource_handle() {
        let mut parser = parser();
        feed(
            &mut parser,
            &[
                r#"{"type":"start","total_files":1,"version":"0.2.62"}"#,
                r#"{"type":"file","path":"ОбщиеМодули/Продажи/Ext/Module.bsl","diagnostics":[]}"#,
                r#"{"type":"done","elapsed_secs":0.1,"total_files":1,"total_diagnostics":0,"failed_files":0}"#,
            ],
        );

        let batch = parser.finish();
        assert_eq!(batch.outcome.status, DiagnosticProviderStatus::Empty);
        assert_eq!(batch.files.processed, Some(1));
    }
}
