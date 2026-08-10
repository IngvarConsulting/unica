use crate::infrastructure::redaction::redactor;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

pub(crate) const MAX_DIAGNOSTICS_JSONL_LINE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub(crate) struct DiagnosticsProtocolError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) retryable: bool,
}

#[derive(Debug)]
pub(crate) struct DiagnosticsProjection {
    pub(crate) data: Value,
    pub(crate) error: Option<DiagnosticsProtocolError>,
}

#[derive(Debug)]
pub(crate) struct DiagnosticsJsonlParser {
    source_root: PathBuf,
    codes: Option<BTreeSet<String>>,
    min_severity: PublicSeverity,
    detailed: bool,
    limit: usize,
    first_error: Option<String>,
    started: bool,
    done: bool,
    version: Option<String>,
    discovered: Option<usize>,
    files_seen: BTreeSet<String>,
    diagnostics_seen: usize,
    failures_seen: usize,
    matched: usize,
    items: Vec<StoredItem>,
    elapsed_seconds: Option<f64>,
    reported: Option<usize>,
    done_files: Option<usize>,
    done_failures: Option<usize>,
}

impl DiagnosticsJsonlParser {
    pub(crate) fn new(source_root: &Path, args: Map<String, Value>) -> Result<Self, String> {
        let codes = args
            .get("codes")
            .and_then(Value::as_array)
            .map(|codes| {
                codes
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<BTreeSet<_>>()
            })
            .filter(|codes| !codes.is_empty());
        let min_severity = match args
            .get("minSeverity")
            .and_then(Value::as_str)
            .unwrap_or("warning")
        {
            "error" => PublicSeverity::Error,
            "warning" => PublicSeverity::Warning,
            "info" => PublicSeverity::Info,
            "hint" => PublicSeverity::Hint,
            value => return Err(format!("unsupported diagnostics severity filter: {value}")),
        };
        let detailed = match args
            .get("detail")
            .and_then(Value::as_str)
            .unwrap_or("concise")
        {
            "concise" => false,
            "detailed" => true,
            value => return Err(format!("unsupported diagnostics detail: {value}")),
        };
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(200) as usize;
        if !(1..=200).contains(&limit) {
            return Err("diagnostics limit must be between 1 and 200".to_string());
        }
        Ok(Self {
            source_root: normalize_absolute_root(source_root)?,
            codes,
            min_severity,
            detailed,
            limit,
            first_error: None,
            started: false,
            done: false,
            version: None,
            discovered: None,
            files_seen: BTreeSet::new(),
            diagnostics_seen: 0,
            failures_seen: 0,
            matched: 0,
            items: Vec::new(),
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

    pub(crate) fn finish(mut self) -> DiagnosticsProjection {
        if self.first_error.is_none() && self.done {
            if let Err(error) = self.validate_totals() {
                self.first_error = Some(error);
            }
        }
        if let Some(message) = self.first_error.take() {
            return self.failure("invalid", "diagnostics_invalid:", false, message);
        }
        if !self.started {
            return self.failure(
                "invalid",
                "diagnostics_invalid:",
                false,
                "line 0: stream is missing start event".to_string(),
            );
        }
        if !self.done {
            if self.files_seen.is_empty() {
                return self.failure(
                    "pending",
                    "diagnostics_pending:",
                    true,
                    "bsl-analyzer emitted start but did not report files or a terminal event"
                        .to_string(),
                );
            }
            return self.failure(
                "incomplete",
                "diagnostics_incomplete:",
                false,
                "bsl-analyzer stream ended after file events without a terminal event".to_string(),
            );
        }

        let failed = self.done_failures.expect("validated done event");
        let discovered = self.discovered.expect("validated start event");
        let mut items = self.items;
        items.sort_by(StoredItem::compare);
        let items = items
            .into_iter()
            .map(StoredItem::into_json)
            .collect::<Vec<_>>();
        let items_total = self.matched + failed;
        DiagnosticsProjection {
            data: json!({
                "action": "analyze",
                "state": "completed",
                "complete": true,
                "retryable": false,
                "analyzerVersion": self.version,
                "files": {
                    "discovered": discovered,
                    "processed": discovered - failed,
                    "failed": failed,
                },
                "diagnostics": {
                    "reported": self.reported,
                    "matched": self.matched,
                },
                "itemsTotal": items_total,
                "itemsReturned": items.len(),
                "truncated": items.len() < items_total,
                "items": items,
                "elapsedSeconds": self.elapsed_seconds,
            }),
            error: None,
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
                    self.retain_item(StoredItem::FileFailure {
                        path,
                        message: redactor(&error),
                    });
                    return Ok(());
                }
                for diagnostic in event.diagnostics {
                    self.diagnostics_seen += 1;
                    let severity = diagnostic.validate()?;
                    if self
                        .codes
                        .as_ref()
                        .is_some_and(|codes| !codes.contains(&diagnostic.code))
                        || severity < self.min_severity
                    {
                        continue;
                    }
                    self.matched += 1;
                    self.retain_item(StoredItem::Diagnostic {
                        path: path.clone(),
                        code: diagnostic.code,
                        severity,
                        internal_severity: self.detailed.then_some(diagnostic.severity),
                        message: diagnostic.message,
                        start_line: diagnostic.start_line,
                        start_column: diagnostic.start_column,
                        end_line: diagnostic.end_line,
                        end_column: diagnostic.end_column,
                        tags: diagnostic
                            .tags
                            .into_iter()
                            .map(|tag| tag.public_name().to_string())
                            .collect(),
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

    fn retain_item(&mut self, item: StoredItem) {
        self.items.push(item);
        self.items.sort_by(StoredItem::compare);
        if self.items.len() > self.limit {
            self.items.pop();
        }
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
        state: &'static str,
        code: &'static str,
        retryable: bool,
        message: String,
    ) -> DiagnosticsProjection {
        DiagnosticsProjection {
            data: json!({
                "action": "analyze",
                "state": state,
                "complete": false,
                "retryable": retryable,
                "analyzerVersion": self.version,
                "files": {
                    "discovered": self.discovered,
                    "processed": Value::Null,
                    "failed": Value::Null,
                },
                "diagnostics": {
                    "reported": Value::Null,
                    "matched": Value::Null,
                },
                "itemsTotal": 0,
                "itemsReturned": 0,
                "truncated": false,
                "items": [],
                "elapsedSeconds": Value::Null,
            }),
            error: Some(DiagnosticsProtocolError {
                code,
                message,
                retryable,
            }),
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
    fn validate(&self) -> Result<PublicSeverity, String> {
        if self.code.trim().is_empty() {
            return Err("diagnostic.code must be non-empty".to_string());
        }
        if self.message.trim().is_empty() {
            return Err("diagnostic.message must be non-empty".to_string());
        }
        let severity = PublicSeverity::from_internal(&self.severity)?;
        if (self.end_line, self.end_column) < (self.start_line, self.start_column) {
            return Err("diagnostic range end precedes its start".to_string());
        }
        let mut tags = BTreeSet::new();
        for tag in &self.tags {
            if !tags.insert(*tag) {
                return Err("diagnostic tags must be unique".to_string());
            }
        }
        Ok(severity)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
enum UpstreamTag {
    Unnecessary,
    Deprecated,
}

impl UpstreamTag {
    const fn public_name(self) -> &'static str {
        match self {
            Self::Unnecessary => "unnecessary",
            Self::Deprecated => "deprecated",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PublicSeverity {
    Hint,
    Info,
    Warning,
    Error,
}

impl PublicSeverity {
    fn from_internal(value: &str) -> Result<Self, String> {
        match value {
            "Blocker" | "Critical" | "Major" | "Error" => Ok(Self::Error),
            "Warning" => Ok(Self::Warning),
            "Information" => Ok(Self::Info),
            "Hint" => Ok(Self::Hint),
            _ => Err(format!("unknown diagnostic severity `{value}`")),
        }
    }

    const fn public_name(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
            Self::Hint => "hint",
        }
    }
}

#[derive(Debug)]
enum StoredItem {
    FileFailure {
        path: String,
        message: String,
    },
    Diagnostic {
        path: String,
        code: String,
        severity: PublicSeverity,
        internal_severity: Option<String>,
        message: String,
        start_line: usize,
        start_column: usize,
        end_line: usize,
        end_column: usize,
        tags: Vec<String>,
    },
}

impl StoredItem {
    fn compare(left: &Self, right: &Self) -> Ordering {
        left.path()
            .cmp(right.path())
            .then_with(|| left.kind_order().cmp(&right.kind_order()))
            .then_with(|| left.range_key().cmp(&right.range_key()))
            .then_with(|| left.code().cmp(right.code()))
            .then_with(|| left.message().cmp(right.message()))
    }

    fn path(&self) -> &str {
        match self {
            Self::FileFailure { path, .. } | Self::Diagnostic { path, .. } => path,
        }
    }

    const fn kind_order(&self) -> u8 {
        match self {
            Self::FileFailure { .. } => 0,
            Self::Diagnostic { .. } => 1,
        }
    }

    const fn range_key(&self) -> (usize, usize, usize, usize) {
        match self {
            Self::FileFailure { .. } => (0, 0, 0, 0),
            Self::Diagnostic {
                start_line,
                start_column,
                end_line,
                end_column,
                ..
            } => (*start_line, *start_column, *end_line, *end_column),
        }
    }

    fn code(&self) -> &str {
        match self {
            Self::FileFailure { .. } => "",
            Self::Diagnostic { code, .. } => code,
        }
    }

    fn message(&self) -> &str {
        match self {
            Self::FileFailure { message, .. } | Self::Diagnostic { message, .. } => message,
        }
    }

    fn into_json(self) -> Value {
        match self {
            Self::FileFailure { path, message } => json!({
                "kind": "fileFailure",
                "path": path,
                "message": message,
            }),
            Self::Diagnostic {
                path,
                code,
                severity,
                internal_severity,
                message,
                start_line,
                start_column,
                end_line,
                end_column,
                tags,
            } => {
                let mut value = json!({
                    "kind": "diagnostic",
                    "path": path,
                    "code": code,
                    "severity": severity.public_name(),
                    "message": message,
                    "range": {
                        "startLine": start_line,
                        "startColumn": start_column,
                        "endLine": end_line,
                        "endColumn": end_column,
                    },
                    "tags": tags,
                });
                if let Some(internal_severity) = internal_severity {
                    value["internalSeverity"] = Value::String(internal_severity);
                }
                value
            }
        }
    }
}

fn normalize_absolute_root(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!(
            "diagnostics source root must be absolute: {}",
            path.display()
        ));
    }
    normalize_lexical(path)
}

fn normalize_reported_path(source_root: &Path, raw: &str) -> Result<String, String> {
    if raw.trim().is_empty() {
        return Err("file.path must be non-empty".to_string());
    }
    let path = Path::new(raw);
    let candidate = if path.is_absolute() {
        normalize_lexical(path)?
    } else {
        normalize_lexical(&source_root.join(path))?
    };
    let relative = candidate.strip_prefix(source_root).map_err(|_| {
        format!(
            "file.path `{raw}` escapes diagnostics source root {}",
            source_root.display()
        )
    })?;
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
    use serde_json::{json, Map, Value};
    use std::path::Path;

    fn parser(args: Value) -> DiagnosticsJsonlParser {
        DiagnosticsJsonlParser::new(
            Path::new("/workspace/src"),
            args.as_object().cloned().unwrap_or_default(),
        )
        .unwrap()
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
        let mut parser = parser(json!({}));
        feed(
            &mut parser,
            &[
                r#"{"type":"start","total_files":1,"version":"0.2.62"}"#,
                &file,
                r#"{"type":"done","elapsed_secs":0.4,"total_files":1,"total_diagnostics":1,"failed_files":0}"#,
            ],
        );

        let result = parser.finish();
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.data["action"], "analyze");
        assert_eq!(result.data["state"], "completed");
        assert_eq!(result.data["complete"], true);
        assert_eq!(result.data["retryable"], false);
        assert_eq!(result.data["analyzerVersion"], "0.2.62");
        assert_eq!(
            result.data["files"],
            json!({"discovered": 1, "processed": 1, "failed": 0})
        );
        assert_eq!(
            result.data["diagnostics"],
            json!({"reported": 1, "matched": 1})
        );
        assert_eq!(result.data["itemsTotal"], 1);
        assert_eq!(result.data["itemsReturned"], 1);
        assert_eq!(result.data["truncated"], false);
        assert_eq!(result.data["items"][0]["severity"], "warning");
        assert_eq!(result.data["items"][0]["tags"], json!(["unnecessary"]));
        assert!(result.data["items"][0].get("internalSeverity").is_none());
        assert_eq!(result.data["elapsedSeconds"], 0.4);
    }

    #[test]
    fn filters_sort_limit_and_file_failures_have_closed_semantics() {
        let mut args = Map::new();
        args.insert("codes".to_string(), json!(["Keep"]));
        args.insert("minSeverity".to_string(), json!("info"));
        args.insert("detail".to_string(), json!("detailed"));
        args.insert("limit".to_string(), json!(1));
        let mut parser = DiagnosticsJsonlParser::new(Path::new("/workspace/src"), args).unwrap();
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

        let result = parser.finish();
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(
            result.data["diagnostics"],
            json!({"reported": 2, "matched": 1})
        );
        assert_eq!(result.data["itemsTotal"], 2);
        assert_eq!(result.data["itemsReturned"], 1);
        assert_eq!(result.data["truncated"], true);
        assert_eq!(result.data["items"][0]["kind"], "diagnostic");
        assert_eq!(result.data["items"][0]["internalSeverity"], "Information");
        assert!(!result.data.to_string().contains("secret"));
    }

    #[test]
    fn only_start_is_pending_and_file_without_done_is_incomplete() {
        let mut pending = parser(json!({}));
        feed(
            &mut pending,
            &[r#"{"type":"start","total_files":1,"version":"0.2.62"}"#],
        );
        let pending = pending.finish();
        assert_eq!(pending.error.as_ref().unwrap().code, "diagnostics_pending:");
        assert!(pending.error.as_ref().unwrap().retryable);
        assert_eq!(pending.data["state"], "pending");
        assert_eq!(pending.data["items"], json!([]));

        let mut incomplete = parser(json!({}));
        feed(
            &mut incomplete,
            &[
                r#"{"type":"start","total_files":1,"version":"0.2.62"}"#,
                r#"{"type":"file","path":"Module.bsl","diagnostics":[]}"#,
            ],
        );
        let incomplete = incomplete.finish();
        assert_eq!(
            incomplete.error.as_ref().unwrap().code,
            "diagnostics_incomplete:"
        );
        assert!(!incomplete.error.as_ref().unwrap().retryable);
        assert_eq!(incomplete.data["state"], "incomplete");
        assert_eq!(incomplete.data["items"], json!([]));
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
            let mut parser = parser(json!({}));
            feed(&mut parser, &lines);
            let result = parser.finish();
            assert_eq!(
                result.error.as_ref().unwrap().code,
                "diagnostics_invalid:",
                "{lines:?}"
            );
            assert_eq!(result.data["state"], "invalid");
            assert_eq!(result.data["items"], json!([]));
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
            let mut parser = parser(json!({}));
            feed(
                &mut parser,
                &[
                    r#"{"type":"start","total_files":1,"version":"0.2.62"}"#,
                    &file,
                ],
            );
            assert_eq!(parser.finish().error.unwrap().code, "diagnostics_invalid:");
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
            let mut parser = parser(json!({}));
            feed(&mut parser, &lines);
            assert_eq!(parser.finish().error.unwrap().code, "diagnostics_invalid:");
        }
    }

    #[test]
    fn line_length_failure_names_the_physical_line_without_copying_it() {
        let mut parser = parser(json!({}));
        parser.reject_line(7, "line exceeds 8388608 bytes");
        let result = parser.finish();
        let error = result.error.unwrap();
        assert_eq!(error.code, "diagnostics_invalid:");
        assert!(error.message.contains("line 7"));
        assert!(!error.message.contains("sensitive contents"));
    }
}
