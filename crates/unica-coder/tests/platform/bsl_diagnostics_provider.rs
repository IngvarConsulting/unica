use std::path::PathBuf;
use unica_adapter_bsl_analyzer::adapter::{AnalyzerRunFailure, AnalyzerRunFailureKind};
use unica_adapter_bsl_analyzer::diagnostics::*;
use unica_adapter_bsl_analyzer::jsonl::*;
use unica_diagnostics::model::*;
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
fn provider_with_backend(
    backend: &(dyn BslDiagnosticBackend + Send + Sync),
) -> BslAnalyzerDiagnosticProvider<'_> {
    BslAnalyzerDiagnosticProvider::new(
        backend,
        crate::infrastructure::diagnostics::findings_module_path,
    )
}
use unica_diagnostics::model::DiagnosticAction;
use unica_diagnostics::model::DiagnosticFilter;
use unica_diagnostics::model::DiagnosticObservation;
use unica_diagnostics::model::DiagnosticProvider;
use unica_diagnostics::model::DiagnosticProviderOutcome;
use unica_diagnostics::model::DiagnosticProviderRequest;
use unica_diagnostics::model::DiagnosticProviderStatus;
use unica_diagnostics::model::DiagnosticReadinessState;
use unica_diagnostics::model::DiagnosticSeverity;
use unica_diagnostics::model::DiagnosticTag;
use unica_diagnostics::model::ProviderDeadline;
use unica_diagnostics::model::BSL_ANALYZER_PROVIDER;
use unica_source_contract::{ProjectSourceSet, SourceFormat, SourceSetKind};

use serde_json::{json, Value};
use std::collections::VecDeque;
use std::fs;
use std::sync::Mutex;
use std::time::Duration;
use tempfile::TempDir;
use unica_address::source_target::{
    MetadataAddress, ResolvedTarget, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use unica_source_contract::workspace::ResolvedSourceRoot;

struct ProviderFixture {
    _temp: TempDir,
    context: DiagnosticContext,
    module: PathBuf,
}

impl ProviderFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        // macOS exposes tempfile paths through `/var` while canonical file
        // identities use `/private/var`. Keep the hand-built context on
        // the same normalized boundary as production application ports.
        let root = unica_workspace::paths::normalize_path_identity(temp.path()).unwrap();
        let source = root.join("src");
        let module = source.join("CommonModules/Smoke/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(
                source.join("Configuration.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Fixture</Name></Properties><ChildObjects><CommonModule>Smoke</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
            )
            .unwrap();
        fs::create_dir_all(source.join("CommonModules")).unwrap();
        fs::write(
                source.join("CommonModules/Smoke.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>Smoke</Name></Properties></CommonModule></MetaDataObject>"#,
            )
            .unwrap();
        fs::write(&module, "Procedure Probe()\nEndProcedure").unwrap();
        let module = unica_workspace::paths::normalize_path_identity(&module).unwrap();
        let metadata_path =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "CommonModule.Smoke.Module")
                .unwrap();
        let workspace = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 7,
        };
        let source_set = ProjectSourceSet {
            name: "main".to_string(),
            kind: SourceSetKind::Configuration,
            path: "src".to_string(),
            source_format: SourceFormat::PlatformXml,
            source_state: unica_source_contract::SourceSetState::Supported,
            format_evidence: Vec::new(),
            format_probe_error: None,
        };
        Self {
            context: DiagnosticContext::new(
                workspace,
                source_set,
                ResolvedSourceRoot {
                    source_set: Some("main".to_string()),
                    path: source,
                },
                ResolvedTarget {
                    source_set: "main".to_string(),
                    metadata_path: Some(metadata_path),
                    target_kind: TargetKind::Module,
                },
            ),
            module,
            _temp: temp,
        }
    }

    fn request(&self, action: DiagnosticAction) -> DiagnosticProviderRequest {
        DiagnosticProviderRequest {
            module_scope: None,
            action,
            source_set: "main".to_string(),
            metadata_path: (action == DiagnosticAction::Findings)
                .then(|| self.context.target.metadata_path.clone().unwrap()),
            target_kind: if action == DiagnosticAction::Findings {
                TargetKind::Module
            } else {
                TargetKind::SourceRoot
            },
            filter: DiagnosticFilter {
                min_severity: Some(DiagnosticSeverity::Error),
                codes: vec![unica_diagnostics::model::DiagnosticCodeFilter {
                    provider: "bsl-analyzer".to_string(),
                    code: "LineLength".to_string(),
                }],
            },
            range: Some(unica_diagnostics::model::DiagnosticRange {
                start_line: 2,
                start_column: 0,
                end_line: 8,
                end_column: 0,
            }),
        }
    }
}

struct FakeBackend {
    calls: Mutex<Vec<BslDiagnosticBackendRequest>>,
    timeouts: Mutex<Vec<Option<Duration>>>,
    replies: Mutex<VecDeque<Result<BslDiagnosticBackendReply, AnalyzerRunFailure>>>,
}

impl FakeBackend {
    fn new(replies: Vec<BslDiagnosticBackendReply>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            timeouts: Mutex::new(Vec::new()),
            replies: Mutex::new(replies.into_iter().map(Ok).collect()),
        }
    }

    fn failing(kind: AnalyzerRunFailureKind) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            timeouts: Mutex::new(Vec::new()),
            replies: Mutex::new(VecDeque::from([Err(
                unica_adapter_bsl_analyzer::diagnostics::analyze_timeout_probe::failure_for_test(
                    kind,
                ),
            )])),
        }
    }

    fn resident(text: Value) -> BslDiagnosticBackendReply {
        BslDiagnosticBackendReply::Resident(BslDiagnosticResidentReply {
            result_text: text.to_string(),
            stderr: String::new(),
            version: Some("0.2.62".to_string()),
        })
    }
}

impl BslDiagnosticBackend for FakeBackend {
    fn invoke(
        &self,
        _context: &DiagnosticContext,
        request: BslDiagnosticBackendRequest,
        timeout: Option<Duration>,
        _cancellation: &CancellationToken,
    ) -> Result<BslDiagnosticBackendReply, AnalyzerRunFailure> {
        self.calls.lock().unwrap().push(request);
        self.timeouts.lock().unwrap().push(timeout);
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("fake reply")
    }
}

#[test]
fn no_deadline_analyze_dispatches_the_analyzer_without_a_timeout() {
    let fixture = ProviderFixture::new();
    let backend = FakeBackend::new(vec![empty_analyze()]);
    let outcome = provider_with_backend(&backend).execute(
        &fixture.request(DiagnosticAction::Analyze),
        &fixture.context,
        ProviderDeadline::no_deadline(),
        &CancellationToken::new(),
    );
    assert_eq!(
        outcome.status,
        DiagnosticProviderStatus::Empty,
        "{outcome:?}"
    );
    assert!(matches!(
        backend.calls.lock().unwrap().as_slice(),
        [BslDiagnosticBackendRequest::Analyze { .. }]
    ));
    assert_eq!(
        backend.timeouts.lock().unwrap().as_slice(),
        [None],
        "an unconfigured analysis must reach the analyzer without a timeout"
    );
}

/// Each place an analyzer run stops keeps its own closed code instead of
/// one "provider is unavailable", and the backend's text stays inside
/// (#1338).
#[test]
fn backend_failure_keeps_where_the_run_stopped_and_drops_its_text() {
    let fixture = ProviderFixture::new();
    for (kind, status, code, retryable) in [
        (
            AnalyzerRunFailureKind::EngineMissing,
            DiagnosticProviderStatus::Unavailable,
            "bundled_tool_missing",
            false,
        ),
        (
            AnalyzerRunFailureKind::EngineInvalid,
            DiagnosticProviderStatus::Unavailable,
            "provider_install_invalid",
            false,
        ),
        (
            AnalyzerRunFailureKind::StartFailed,
            DiagnosticProviderStatus::Unavailable,
            "provider_start_failed",
            true,
        ),
        (
            AnalyzerRunFailureKind::TimedOut,
            DiagnosticProviderStatus::Failed,
            "provider_timeout",
            true,
        ),
        (
            AnalyzerRunFailureKind::Exited,
            DiagnosticProviderStatus::Failed,
            "provider_exited",
            true,
        ),
        (
            AnalyzerRunFailureKind::Other,
            DiagnosticProviderStatus::Failed,
            "provider_unavailable",
            true,
        ),
        (
            AnalyzerRunFailureKind::Cancelled,
            DiagnosticProviderStatus::Failed,
            "cancelled",
            false,
        ),
    ] {
        let backend = FakeBackend::failing(kind);
        let outcome = execute(
            &provider_with_backend(&backend),
            &fixture,
            DiagnosticAction::Analyze,
        );
        assert_eq!(
            backend.calls.lock().unwrap().len(),
            1,
            "{kind:?}: backend must run"
        );
        assert_eq!(outcome.status, status, "{kind:?}");
        assert!(!outcome.complete, "{kind:?}");
        let error = outcome.error.expect("failed run names its error");
        assert_eq!(error.code, code, "{kind:?}");
        assert_eq!(error.retryable, retryable, "{kind:?}");
        assert!(!error.message.contains("private"), "{kind:?}: {error:?}");
        if kind == AnalyzerRunFailureKind::EngineMissing {
            assert_eq!(
                unica_diagnostics::coordinator::canonical_missing_engine_reason(&error.message)
                    .as_deref(),
                Some(error.message.as_str()),
                "an absent engine is named by its install mode's next step"
            );
        }
    }
}

#[test]
fn no_deadline_diagnostics_refuses_before_legacy_backend_dispatch() {
    let fixture = ProviderFixture::new();
    let backend = FakeBackend::new(Vec::new());
    for action in [
        DiagnosticAction::Findings,
        DiagnosticAction::Status,
        DiagnosticAction::Catalog,
    ] {
        let outcome = provider_with_backend(&backend).execute(
            &fixture.request(action),
            &fixture.context,
            ProviderDeadline::no_deadline(),
            &CancellationToken::new(),
        );
        assert_eq!(outcome.status, DiagnosticProviderStatus::Unsupported);
        let error = outcome.error.unwrap();
        assert_eq!(error.code, "provider_deadline_unsupported");
        assert_eq!(
            error.message,
            unica_work_control::provider_deadline::ProviderDeadlineError::Unsupported.to_string()
        );
    }
    assert!(backend.calls.lock().unwrap().is_empty());
}

#[test]
fn no_deadline_diagnostics_explicit_cancel_wins_before_unsupported_bridge() {
    let fixture = ProviderFixture::new();
    let backend = FakeBackend::new(Vec::new());
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let outcome = provider_with_backend(&backend).execute(
        &fixture.request(DiagnosticAction::Analyze),
        &fixture.context,
        ProviderDeadline::no_deadline(),
        &cancellation,
    );
    assert_eq!(outcome.status, DiagnosticProviderStatus::Failed);
    assert_eq!(outcome.error.unwrap().code, "cancelled");
    assert!(backend.calls.lock().unwrap().is_empty());
}

fn empty_analyze() -> BslDiagnosticBackendReply {
    BslDiagnosticBackendReply::Analyze(Box::new(AnalyzerDiagnosticsBatch {
        outcome: DiagnosticProviderOutcome {
            status: DiagnosticProviderStatus::Empty,
            complete: true,
            version: Some("0.2.62".to_string()),
            observations: Vec::new(),
            rules: Vec::new(),
            readiness: None,
            error: None,
            suppressions: Vec::new(),
        },
        files: AnalyzerDiagnosticsFileTotals {
            discovered: Some(0),
            processed: Some(0),
            failed: Some(0),
        },
        diagnostics_reported: Some(0),
        elapsed_seconds: Some(0.1),
    }))
}

fn execute(
    provider: &BslAnalyzerDiagnosticProvider<'_>,
    fixture: &ProviderFixture,
    action: DiagnosticAction,
) -> DiagnosticProviderOutcome {
    provider.execute(
        &fixture.request(action),
        &fixture.context,
        ProviderDeadline::from_budget(Duration::from_secs(30)),
        &CancellationToken::new(),
    )
}

#[test]
fn addressed_analyze_preserves_project_root_and_passes_exact_module() {
    let fixture = ProviderFixture::new();
    let backend = FakeBackend::new(vec![empty_analyze()]);
    let provider = provider_with_backend(&backend);
    let mut request = fixture.request(DiagnosticAction::Analyze);
    request.module_scope = Some(unica_diagnostics::model::DiagnosticModuleScope {
        source_set: "main".into(),
        source_root: fixture.context.source_root.path.clone(),
        module_path: fixture.module.clone(),
    });
    provider.execute(
        &request,
        &fixture.context,
        ProviderDeadline::from_budget(Duration::from_secs(30)),
        &CancellationToken::new(),
    );
    assert_eq!(
        backend.calls.lock().unwrap().as_slice(),
        &[BslDiagnosticBackendRequest::Analyze {
            source_root: fixture.context.source_root.path.clone(),
            module: Some(fixture.module),
        }]
    );
}

#[test]
fn bsl_diagnostics_provider_request_maps_all_actions_without_public_filters() {
    let fixture = ProviderFixture::new();
    let backend = FakeBackend::new(vec![
        empty_analyze(),
        FakeBackend::resident(json!({
            "revision": 1,
            "stale": false,
            "reload": "none",
            "result": {"kind": "full", "truncated": false, "findings": []}
        })),
        FakeBackend::resident(json!({"state":"ready","generation":1,"reload":"none"})),
        FakeBackend::resident(json!({"action":"catalog","locale":"en","count":0,"entries":[]})),
    ]);
    let provider = provider_with_backend(&backend);

    for action in [
        DiagnosticAction::Analyze,
        DiagnosticAction::Findings,
        DiagnosticAction::Status,
        DiagnosticAction::Catalog,
    ] {
        let outcome = execute(&provider, &fixture, action);
        assert!(
            matches!(
                outcome.status,
                DiagnosticProviderStatus::Completed | DiagnosticProviderStatus::Empty
            ),
            "{action:?}: {outcome:?}"
        );
    }

    let calls = backend.calls.lock().unwrap();
    assert_eq!(
        calls[0],
        BslDiagnosticBackendRequest::Analyze {
            source_root: fixture.context.source_root.path.clone(),
            module: None,
        }
    );
    assert_eq!(
        calls[1],
        BslDiagnosticBackendRequest::Resident {
            source_root: fixture.context.source_root.path.clone(),
            tool_name: "diagnostics",
            arguments: json!({
                "action": "file",
                "path": fixture.module,
                "min_severity": "hint",
                "max_findings": 5000
            }),
        }
    );
    assert_eq!(
        calls[2],
        BslDiagnosticBackendRequest::Resident {
            source_root: fixture.context.source_root.path.clone(),
            tool_name: "diagnostics",
            arguments: json!({"action":"status"}),
        }
    );
    assert_eq!(
        calls[3],
        BslDiagnosticBackendRequest::Resident {
            source_root: fixture.context.source_root.path.clone(),
            tool_name: "diagnostics",
            arguments: json!({"action":"catalog","locale":"en"}),
        }
    );
    let wire = serde_json::to_string(&calls[1].arguments()).unwrap();
    for forbidden in ["LineLength", "range_start", "range_end", "limit"] {
        assert!(!wire.contains(forbidden), "forwarded public filter: {wire}");
    }
}

/// Captured from the pinned 0.2.86 native resident protocol. Only the
/// volatile result id is normalized; author and baseline counts are real.
fn captured_baseline_authors_reply() -> Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/analyzer/resident-baseline-authors-0.2.86.json"
    )))
    .unwrap()
}

fn parse_filter_reply(value: Value) -> Result<DiagnosticProviderOutcome, String> {
    parse_resident_findings(
        BslDiagnosticResidentReply {
            result_text: value.to_string(),
            stderr: String::new(),
            version: Some("0.2.86".to_string()),
        },
        PathBuf::from("/fixture/Module.bsl"),
    )
}

#[test]
fn resident_filters_preserve_captured_baseline_and_authors_together() {
    let mut value = captured_baseline_authors_reply();
    value["result"]["author_hint"] = json!("secret author /private/hidden.bsl");
    let actual = parse_filter_reply(value).unwrap();
    assert_eq!(actual.status, DiagnosticProviderStatus::Completed);
    assert!(actual.complete);
    assert!(actual.observations.is_empty());
    assert_eq!(
        serde_json::to_value(&actual.suppressions).unwrap(),
        json!([
            {"by":"baseline","known":1,"new":2}, {"by":"authors","count":2}
        ])
    );
    assert!(!format!("{actual:?}").contains("secret author"));
}

#[test]
fn resident_analysis_scope_is_incomplete_without_erasing_proven_findings_or_filters() {
    let mut value = captured_baseline_authors_reply();
    value["result"]["out_of_scope"] = json!(true);
    value["result"]["hint"] = json!("secret author /private/hidden.bsl");
    // Combination is a protocol counterexample, not a claim that the
    // captured live request itself returned an out-of-scope finding.
    value["result"]["findings"] = json!([{"code":"UnusedLocalVariable",
            "severity":"warning", "message":"unused variable", "tags":[],
            "range":{"start_line":1,"start_column":8,"end_line":1,"end_column":14}}]);
    let actual = parse_filter_reply(value.clone()).unwrap();
    assert_eq!(actual.status, DiagnosticProviderStatus::Completed);
    assert!(!actual.complete);
    assert_eq!(actual.suppressions.len(), 2);
    assert_eq!(actual.observations.len(), 2);
    assert!(
        matches!(&actual.observations[0], DiagnosticObservation::Diagnostic{code,..} if code=="UnusedLocalVariable")
    );
    assert!(
        matches!(&actual.observations[1], DiagnosticObservation::ResourceFailure {error,..} if error.code=="analysis_out_of_scope")
    );
    assert!(!format!("{actual:?}").contains("secret author"));
    value["result"]
        .as_object_mut()
        .unwrap()
        .remove("out_of_scope");
    value["result"]["truncated"] = json!(true);
    assert!(!parse_filter_reply(value.clone()).unwrap().complete);
    value["result"]["truncated"] = json!(false);
    value["result"]["baseline"] =
        json!({"state":"error","error_code":"missing","detail":"cannot read baseline"});
    let broken = parse_filter_reply(value).unwrap();
    assert!(!broken.complete);
    assert_eq!(broken.suppressions.len(), 2);
    assert_eq!(broken.suppressions[0].reason().unwrap().code, "missing");
}

#[test]
fn resident_filter_absence_and_zero_are_clean_but_malformed_flags_are_rejected() {
    let plain = json!({"revision":1,"stale":false,"reload":"none",
            "result":{"kind":"full","truncated":false,"findings":[]}});
    for field in [None, Some(json!(0))] {
        let mut value = plain.clone();
        value["result"]["out_of_scope"] = json!(false);
        if let Some(field) = field {
            value["result"]["findings_ignored_by_author"] = field;
        }
        let actual = parse_filter_reply(value).unwrap();
        assert_eq!(actual.status, DiagnosticProviderStatus::Empty);
        assert!(actual.complete);
        assert!(actual.suppressions.is_empty());
    }
    for invalid in [json!(-1), json!(1.5), json!("2"), json!(true), json!(1e30)] {
        let mut value = plain.clone();
        value["result"]["findings_ignored_by_author"] = invalid;
        assert!(parse_filter_reply(value).is_err());
    }
    for invalid in [json!(0), json!("false"), json!(null)] {
        let mut value = plain.clone();
        value["result"]["out_of_scope"] = invalid;
        assert!(parse_filter_reply(value).is_err());
    }
    let mut scope_only = plain;
    scope_only["result"]["out_of_scope"] = json!(true);
    let actual = parse_filter_reply(scope_only).unwrap();
    assert_eq!(actual.status, DiagnosticProviderStatus::Completed);
    assert!(!actual.complete);
    assert!(actual.suppressions.is_empty());
    assert_eq!(actual.observations.len(), 1);
}

#[test]
fn truncated_empty_resident_findings_are_incomplete_not_empty() {
    let fixture = ProviderFixture::new();
    let backend = FakeBackend::new(vec![FakeBackend::resident(json!({
        "revision": 1,
        "stale": false,
        "reload": "none",
        "result": {"kind": "full", "truncated": true, "findings": []}
    }))]);
    let provider = provider_with_backend(&backend);

    let outcome = execute(&provider, &fixture, DiagnosticAction::Findings);

    assert_eq!(outcome.status, DiagnosticProviderStatus::Completed);
    assert!(!outcome.complete);
    assert!(outcome.observations.is_empty());
}

/// The resident reply carries the same baseline summary as `analyze`, in
/// the shapes upstream `diagnostics file` publishes: a file answer is
/// always `partial` with exact `known`/`new`, and a broken baseline is an
/// `error` branch with `error_code`/`detail` and no findings.
#[test]
fn resident_baseline_is_a_named_filter_and_a_broken_one_names_its_cause() {
    let reply = |baseline: Value| {
        let mut result = json!({"kind": "full", "baseline": baseline});
        if baseline["state"] != "error" {
            result["truncated"] = json!(false);
            result["findings"] = json!([]);
        }
        FakeBackend::resident(json!({
            "revision": 1, "stale": false, "reload": "none", "result": result
        }))
    };
    let run = |baseline: Value| {
        let fixture = ProviderFixture::new();
        let backend = FakeBackend::new(vec![reply(baseline)]);
        let provider = provider_with_backend(&backend);
        execute(&provider, &fixture, DiagnosticAction::Findings)
    };

    let disabled = run(json!({"state": "disabled", "complete": true}));
    assert_eq!(disabled.status, DiagnosticProviderStatus::Empty);
    assert!(disabled.complete);
    assert!(disabled.suppressions.is_empty());

    for state in ["full", "partial"] {
        let filtered = run(json!({
            "state": state, "complete": state == "full", "new": 0, "known": 2,
            "path": "baseline.json", "partitions": [], "partitions_total": 0,
            "partitions_returned": 0, "partitions_truncated": false
        }));
        assert_eq!(
            filtered.status,
            DiagnosticProviderStatus::Completed,
            "{state}"
        );
        assert!(filtered.complete, "{state}");
        assert!(filtered.observations.is_empty(), "{state}");
        let suppression = filtered
            .suppressions
            .into_iter()
            .next()
            .expect("filter is named");
        assert_eq!(
            (
                match &suppression {
                    unica_diagnostics::model::DiagnosticSuppression::Baseline { known, .. } =>
                        *known,
                    _ => panic!("expected baseline fact"),
                },
                match &suppression {
                    unica_diagnostics::model::DiagnosticSuppression::Baseline { new, .. } => *new,
                    _ => panic!("expected baseline fact"),
                }
            ),
            (Some(2), Some(0))
        );
        assert!(suppression.reason().is_none(), "{state}");
    }

    let broken = run(json!({
        "state": "error", "complete": false, "path": "baseline.json",
        "error_code": "invalid_schema", "detail": "baseline schema 7 is newer; token=abc",
        "errors_total": 1,
        "errors": [{"partition_id": null, "code": "invalid_schema", "detail": "baseline schema 7 is newer; token=abc"}]
    }));
    assert_eq!(broken.status, DiagnosticProviderStatus::Completed);
    assert!(!broken.complete);
    assert!(broken.error.is_none());
    let reason = broken
        .suppressions
        .into_iter()
        .next()
        .unwrap()
        .reason()
        .cloned()
        .expect("cause is named");
    assert_eq!(reason.code, "invalid_schema");
    let detail = reason.detail.unwrap();
    assert!(detail.starts_with("baseline schema 7 is newer"), "{detail}");
    assert!(!detail.contains("abc"), "{detail}");

    // The state set is closed: an unknown state is a protocol change.
    let unknown = run(json!({"state": "suppressed", "complete": true}));
    assert_eq!(unknown.status, DiagnosticProviderStatus::Failed);
    assert!(unknown.suppressions.is_empty());
}

#[test]
fn bsl_diagnostics_provider_maps_findings_and_loading_to_common_outcomes() {
    let fixture = ProviderFixture::new();
    let backend = FakeBackend::new(vec![
        FakeBackend::resident(json!({
            "revision": 3,
            "stale": false,
            "reload": "none",
            "result": {
                "kind": "full",
                "truncated": false,
                "findings": [{
                    "code": "LineLength",
                    "severity": "warning",
                    "message": "Line too long",
                    "range": {"start_line":0,"start_column":10,"end_line":0,"end_column":18},
                    "tags": ["Unnecessary"],
                    "has_fix": false,
                    "future_field": "ignored"
                }]
            },
            "future_envelope": true
        })),
        FakeBackend::resident(json!({
            "status":"loading","detail":"building","state":"loading","generation":4
        })),
    ]);
    let provider = provider_with_backend(&backend);

    let complete = execute(&provider, &fixture, DiagnosticAction::Findings);
    assert_eq!(complete.status, DiagnosticProviderStatus::Completed);
    assert_eq!(complete.version.as_deref(), Some("0.2.62"));
    assert!(matches!(
        &complete.observations[0],
        DiagnosticObservation::Diagnostic {
            provider,
            location: DiagnosticObservationLocation::Resource { handle },
            focus: DiagnosticObservationFocus::SourceRange(range),
            code,
            severity: DiagnosticSeverity::Warning,
            tags,
            ..
        } if *provider == BSL_ANALYZER_PROVIDER
            && handle == &fixture.module.display().to_string()
            && code == "LineLength"
            && *range == unica_diagnostics::model::DiagnosticRange {
                start_line: 0, start_column: 10, end_line: 0, end_column: 18
            }
            && tags == &vec![DiagnosticTag::Unnecessary]
    ));

    let loading = execute(&provider, &fixture, DiagnosticAction::Findings);
    assert_eq!(loading.status, DiagnosticProviderStatus::Unavailable);
    assert!(loading.observations.is_empty());
    let error = loading.error.unwrap();
    assert_eq!(error.code, "provider_not_ready");
    assert!(error.retryable);
}

#[test]
fn bsl_diagnostics_provider_maps_readiness_catalog_failures_and_resource_errors() {
    let fixture = ProviderFixture::new();
    let replies = vec![
        FakeBackend::resident(json!({"state":"idle","generation":0,"reload":"none"})),
        FakeBackend::resident(json!({"state":"loading","generation":1,"reload":"running"})),
        FakeBackend::resident(json!({"state":"ready","generation":2,"reload":"none","files":3})),
        FakeBackend::resident(json!({"state":"ready","generation":2,"reload":"running","files":3})),
        FakeBackend::resident(json!({
            "action":"catalog","locale":"en","count":1,
            "entries":[{
                "code":"LineLength","title":"Line length","default_severity":"warning",
                "type":"code_smell","activated_by_default":true,"tags":["unused"]
            }]
        })),
        FakeBackend::resident(json!({
            "revision": 1, "stale": false, "reload": "none",
            "result":{"error":"not_in_workspace","detail":"module is not resident"}
        })),
        BslDiagnosticBackendReply::Resident(BslDiagnosticResidentReply {
            result_text: "not-json C:\\secret\\Module.bsl".to_string(),
            stderr: "private stderr".to_string(),
            version: Some("0.2.62".to_string()),
        }),
    ];
    let backend = FakeBackend::new(replies);
    let provider = provider_with_backend(&backend);

    for expected in [
        DiagnosticReadinessState::NotStarted,
        DiagnosticReadinessState::Building,
        DiagnosticReadinessState::Ready,
        DiagnosticReadinessState::Stale,
    ] {
        let status = execute(&provider, &fixture, DiagnosticAction::Status);
        assert_eq!(status.status, DiagnosticProviderStatus::Completed);
        assert_eq!(status.readiness.unwrap().state, expected);
    }

    let catalog = execute(&provider, &fixture, DiagnosticAction::Catalog);
    assert_eq!(catalog.status, DiagnosticProviderStatus::Completed);
    assert_eq!(catalog.rules.len(), 1);
    assert_eq!(catalog.rules[0].code, "LineLength");
    assert_eq!(
        catalog.rules[0].default_severity,
        DiagnosticSeverity::Warning
    );
    assert_eq!(catalog.rules[0].tags, vec![DiagnosticTag::Unnecessary]);

    let resource = execute(&provider, &fixture, DiagnosticAction::Findings);
    assert!(matches!(
        &resource.observations[0],
        DiagnosticObservation::ResourceFailure { error, .. }
            if error.code == "source_analysis_failed"
                && error.message == "module is not resident"
    ));

    let malformed = execute(&provider, &fixture, DiagnosticAction::Findings);
    assert_eq!(malformed.status, DiagnosticProviderStatus::Failed);
    assert!(malformed.observations.is_empty());
    let error = malformed.error.unwrap();
    assert_eq!(error.code, "provider_reply_invalid");
    assert!(!error.message.contains("secret"));
    assert!(!error.message.contains("stderr"));
}
