use super::TextEngine;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use unica_code_intelligence::coordinator::CodeSearchExecution;
use unica_code_intelligence::deadlines::CodeIntelligenceDeadlines;
use unica_code_intelligence::model::CodeIntelligenceContext;
use unica_code_intelligence::model::CodeIntelligenceProvider;
use unica_code_intelligence::model::CodeIntelligenceRegistry;
use unica_code_intelligence::model::CodeSearchScope;
use unica_code_intelligence::model::ProviderCapability;
use unica_code_intelligence::model::ProviderIdentity;
use unica_code_intelligence::model::ProviderRole;
use unica_code_intelligence::model::ProviderSearchSection;
use unica_code_intelligence::model::SearchOrdering;
use unica_code_intelligence::model::SearchRanking;
use unica_code_intelligence::model::SearchRequest;
use unica_navigation::result_store::SearchCursorBinding;
use unica_source_contract::workspace::ResolvedSourceRoot;
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

#[test]
fn saved_apply_execution_refusal_distinguishes_token_from_internal_state() {
    use super::SavedApplyExecutionError;
    use unica_workspace::actor::ApplyPublicationError;
    for (error, code, outcome) in [
        (
            SavedApplyExecutionError::Busy,
            "task_backend_failed",
            "retry",
        ),
        (
            SavedApplyExecutionError::RegistryUnavailable,
            "invalid_state",
            "needsHuman",
        ),
        (
            SavedApplyExecutionError::StateUnavailable("saved apply state is unavailable"),
            "invalid_state",
            "needsHuman",
        ),
        (
            SavedApplyExecutionError::Unavailable("request a fresh plan".into()),
            "bad_value",
            "fixCall",
        ),
        (
            SavedApplyExecutionError::Publication(ApplyPublicationError::new(
                super::ApplyPublicationErrorKind::Deadline,
                "execution wait expired",
            )),
            "deadline_exceeded",
            "retry",
        ),
        (
            SavedApplyExecutionError::Publication(ApplyPublicationError::new(
                super::ApplyPublicationErrorKind::Cancelled,
                "execution wait cancelled",
            )),
            "cancelled",
            "deadEnd",
        ),
    ] {
        let result = super::saved_apply_execution_error_result(error);
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], code);
        assert_eq!(result.diagnostics[0]["outcome"], outcome);
    }
}

/// #955, #1332: a staging failure names the violated precondition and
/// what to do next. Only a source the filesystem cannot read stays
/// `provider_unavailable`, refined to `source_unreadable`.
#[test]
fn staging_refusals_keep_their_kind_instead_of_an_absent_provider() {
    use unica_source_contract::proposal::{
        ApplyPlanError, ApplyStagingError, ApplyStagingErrorKind,
    };
    for (kind, code, outcome, detail) in [
        (
            ApplyStagingErrorKind::Cancelled,
            "cancelled",
            "deadEnd",
            None,
        ),
        (
            ApplyStagingErrorKind::Deadline,
            "deadline_exceeded",
            "retry",
            None,
        ),
        (
            ApplyStagingErrorKind::ConcurrentRevision,
            "stale_revision",
            "fixCall",
            None,
        ),
        (
            ApplyStagingErrorKind::ContainmentIdentity,
            "containment_denied",
            "fixCall",
            None,
        ),
        (
            ApplyStagingErrorKind::AbsentChainOccupied,
            "invalid_source",
            "fixSource",
            None,
        ),
        (
            ApplyStagingErrorKind::Invariant,
            "invalid_state",
            "needsHuman",
            None,
        ),
        (
            ApplyStagingErrorKind::UnsupportedProvider,
            "provider_unavailable",
            "fixSource",
            Some("source_unreadable"),
        ),
    ] {
        let error = ApplyPlanError::staging(
            ApplyStagingError::new(kind, "staged replace preimage changed: Documents/A.xml"),
            "ops[1].args.values.name",
        );
        let planned = super::apply_plan_error_result(&error);
        let prepared =
            super::apply_staging_refusal(Some("ops".to_string()), kind, "prepared".to_string());
        for result in [&planned, &prepared] {
            assert!(!result.ok, "{kind:?}");
            assert_eq!(result.diagnostics[0]["code"], code, "{kind:?}");
            assert_eq!(result.diagnostics[0]["outcome"], outcome, "{kind:?}");
            assert_eq!(
                result.diagnostics[0]
                    .get("detailCode")
                    .and_then(|v| v.as_str()),
                detail,
                "{kind:?}"
            );
        }
        assert_eq!(planned.at.as_deref(), Some("ops[1].args.values.name"));
        let message = planned.diagnostics[0]["message"].as_str().unwrap();
        assert!(
            message.contains("staged replace preimage changed: Documents/A.xml"),
            "{kind:?}: the cause was lost: {message}"
        );
        assert!(!message.contains("evidence is unavailable"), "{message}");
    }
    // Publication re-proves the plan's containment; a failure there is a
    // physical change after the plan, answered like a stale plan.
    assert_eq!(
        super::apply_publication_error_code(super::ApplyPublicationErrorKind::ContainmentIdentity),
        unica_refusal::RefusalCode::StaleRevision
    );
}

fn provider_search_test_binding(role: ProviderRole, limit: usize) -> SearchCursorBinding {
    SearchCursorBinding {
        workspace_identity: "workspace".into(),
        query: "Needle".into(),
        scope: None,
        mode: role.as_str().into(),
        kind: None,
        source_sets: vec!["main".into()],
        result_fingerprint: None,
        page_limit: limit,
    }
}

fn provider_search_test_execution(
    count: usize,
    snippet_bytes: usize,
    provider_limit_reached: bool,
) -> CodeSearchExecution {
    use unica_address::source_target::TargetKind;
    use unica_code_intelligence::model::CodeSearchResult;
    use unica_code_intelligence::model::ProviderSearchHit;
    use unica_code_intelligence::model::SearchCoverage;
    use unica_source_contract::location::SourceLocation;

    let hits = (1..=count)
        .map(|rank| ProviderSearchHit {
            rank: Some(rank),
            provider_score: Some((count - rank) as f64),
            location: SourceLocation::Addressed {
                source_set: "main".to_string(),
                metadata_path: None,
                target_kind: TargetKind::Module,
            },
            line: rank,
            end_line: None,
            symbol: None,
            kind: None,
            snippet: "x".repeat(snippet_bytes),
            attributes: serde_json::Map::new(),
        })
        .collect();
    let identity = ProviderIdentity::new(ProviderRole::Semantic, "semantic-fixture");
    let section = if provider_limit_reached {
        ProviderSearchSection::limit_reached(
            identity,
            SearchRanking::Provider,
            SearchOrdering::Provider,
            hits,
            Vec::new(),
        )
    } else {
        ProviderSearchSection::complete(
            identity,
            SearchRanking::Provider,
            SearchOrdering::Provider,
            hits,
            Vec::new(),
        )
    }
    .expect("valid provider section");
    CodeSearchExecution {
        ok: true,
        result: CodeSearchResult {
            coverage: if provider_limit_reached {
                SearchCoverage::Partial
            } else {
                SearchCoverage::Complete
            },
            elapsed_ms: 1,
            sections: vec![section],
        },
        warnings: Vec::new(),
        errors: Vec::new(),
    }
}

/// rule: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
#[test]
fn maintenance_transition_invalidates_the_cursor_for_the_previous_answer() {
    use unica_code_intelligence::model::IndexMaintenanceEvidence;
    let store = Default::default();
    let mut queued = provider_search_test_execution(75, 8, false);
    queued.result.sections[0].index_maintenance = Some(IndexMaintenanceEvidence::Queued);
    let first = super::provider_search_page(
        &store,
        ProviderRole::Semantic,
        queued,
        provider_search_test_binding(ProviderRole::Semantic, 20),
        None,
        &CancellationToken::new(),
    );
    assert!(first.ok, "{first:?}");
    let mut running = provider_search_test_execution(75, 8, false);
    running.result.sections[0].index_maintenance = Some(IndexMaintenanceEvidence::Running);
    let next = super::provider_search_page(
        &store,
        ProviderRole::Semantic,
        running,
        provider_search_test_binding(ProviderRole::Semantic, 20),
        first.cursor.as_deref(),
        &CancellationToken::new(),
    );
    assert!(!next.ok, "{next:?}");
    assert_eq!(next.diagnostics[0]["code"], "stale_cursor");
}

/// rule: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
#[test]
fn provider_search_pages_all_received_hits_and_rejects_changed_answers() {
    let store = unica_navigation::result_store::SearchCursorStore::default();
    let cancellation = CancellationToken::new();
    let mut cursor = None;
    let mut ranks = Vec::new();
    for page_index in 0..4 {
        let result = super::provider_search_page(
            &store,
            ProviderRole::Semantic,
            provider_search_test_execution(75, 8, false),
            provider_search_test_binding(ProviderRole::Semantic, 20),
            cursor.as_deref(),
            &cancellation,
        );
        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().expect("page data");
        let section = &data["matches"][0];
        let hits = section["hits"].as_array().expect("page hits");
        assert_eq!(section["matches"]["returned"], hits.len());
        assert_eq!(section["matches"]["total"], 75);
        assert_eq!(section["matches"]["relation"], "exact");
        ranks.extend(hits.iter().map(|hit| hit["rank"].as_u64().unwrap()));
        if page_index == 0 {
            let replay = super::provider_search_page(
                &store,
                ProviderRole::Semantic,
                provider_search_test_execution(75, 8, false),
                provider_search_test_binding(ProviderRole::Semantic, 20),
                result.cursor.as_deref(),
                &cancellation,
            );
            assert_eq!(
                replay.data,
                super::provider_search_page(
                    &store,
                    ProviderRole::Semantic,
                    provider_search_test_execution(75, 8, false),
                    provider_search_test_binding(ProviderRole::Semantic, 20),
                    result.cursor.as_deref(),
                    &cancellation,
                )
                .data
            );
            assert_eq!(
                replay.cursor,
                super::provider_search_page(
                    &store,
                    ProviderRole::Semantic,
                    provider_search_test_execution(75, 8, false),
                    provider_search_test_binding(ProviderRole::Semantic, 20),
                    result.cursor.as_deref(),
                    &cancellation,
                )
                .cursor
            );
            let changed = super::provider_search_page(
                &store,
                ProviderRole::Semantic,
                provider_search_test_execution(75, 9, false),
                provider_search_test_binding(ProviderRole::Semantic, 20),
                result.cursor.as_deref(),
                &cancellation,
            );
            assert!(!changed.ok);
            assert_eq!(changed.diagnostics[0]["code"], "stale_cursor");
            let mut oversized_change = provider_search_test_execution(75, 8, false);
            oversized_change.result.sections[0].hits[74].snippet =
                "x".repeat(unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES);
            let stale_before_size = super::provider_search_page(
                &store,
                ProviderRole::Semantic,
                oversized_change,
                provider_search_test_binding(ProviderRole::Semantic, 20),
                result.cursor.as_deref(),
                &cancellation,
            );
            assert_eq!(stale_before_size.diagnostics[0]["code"], "stale_cursor");
        }
        cursor = result.cursor;
    }
    assert_eq!(ranks, (1..=75).collect::<Vec<_>>());
    assert!(cursor.is_none());
}

/// rule: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
#[test]
fn provider_limit_remains_visible_after_the_last_received_page() {
    let store = unica_navigation::result_store::SearchCursorStore::default();
    let cancellation = CancellationToken::new();
    let mut cursor = None;
    let mut last = None;
    for _ in 0..4 {
        let result = super::provider_search_page(
            &store,
            ProviderRole::Semantic,
            provider_search_test_execution(200, 8, true),
            provider_search_test_binding(ProviderRole::Semantic, 50),
            cursor.as_deref(),
            &cancellation,
        );
        assert!(result.ok, "{result:?}");
        assert!(result.warnings.iter().any(|warning| {
            warning
                .as_str()
                .is_some_and(|text| text.contains("200") && text.contains("narrow the query"))
        }));
        cursor = result.cursor.clone();
        last = Some(result);
    }
    let last = last.unwrap();
    assert!(last.cursor.is_none());
    assert_eq!(last.page.as_ref().unwrap()["stoppedBy"], "complete");
    assert!(last.summary.contains("incomplete"));
    let section = &last.data.as_ref().unwrap()["matches"][0];
    assert_eq!(section["status"], "limitReached");
    assert_eq!(section["searchComplete"], false);
    assert_eq!(section["termination"]["code"], "limitReached");
    assert_eq!(section["matches"]["returned"], 50);
    assert_eq!(section["matches"]["total"], 200);
    assert_eq!(section["matches"]["relation"], "lowerBound");
    assert_eq!(section["hits"][0]["rank"], 151);
}

/// rule: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
#[test]
fn late_oversized_provider_hit_refuses_before_first_cursor() {
    let store = unica_navigation::result_store::SearchCursorStore::default();
    let cancellation = CancellationToken::new();
    let mut execution = provider_search_test_execution(25, 8, false);
    execution.result.sections[0].hits[24].snippet =
        "x".repeat(unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES);
    let result = super::provider_search_page(
        &store,
        ProviderRole::Semantic,
        execution,
        provider_search_test_binding(ProviderRole::Semantic, 20),
        None,
        &cancellation,
    );
    assert!(!result.ok);
    assert_eq!(result.diagnostics[0]["code"], "result_too_large");
    assert!(result.cursor.is_none());
}

#[test]
fn selected_provider_with_multiple_sections_reports_failed_result() {
    let mut execution = provider_search_test_execution(1, 8, false);
    execution
        .result
        .sections
        .push(execution.result.sections[0].clone());
    let result = super::provider_search_page(
        &unica_navigation::result_store::SearchCursorStore::default(),
        ProviderRole::Semantic,
        execution,
        provider_search_test_binding(ProviderRole::Semantic, 20),
        None,
        &CancellationToken::new(),
    );
    assert_eq!(result.diagnostics[0]["code"], "provider_failed");
}

/// rule: INV.WIRE.CHECK-DIAGNOSTIC-PAGES
#[test]
fn bsl_check_never_calls_a_truncated_analyzer_result_complete() {
    use unica_diagnostics::model::DiagnosticAction;
    use unica_diagnostics::model::DiagnosticResult;
    use unica_diagnostics::model::DiagnosticResultState;
    use unica_diagnostics::model::DiagnosticSelection;
    let mut result = DiagnosticResult {
        ok: true,
        action: DiagnosticAction::Analyze,
        selection: DiagnosticSelection {
            source_set: "main".to_string(),
            metadata_path: None,
            target_kind: None,
            providers: vec!["bsl-language-server"],
            filter: None,
            limit: Some(200),
        },
        state: DiagnosticResultState::Completed,
        complete: true,
        providers: Vec::new(),
        items_total: Some(201),
        items_returned: Some(200),
        truncated: Some(true),
        items: Vec::new(),
    };
    assert!(!super::bsl_result_proves_full_verdict(&result));
    result.truncated = Some(false);
    result.items_total = Some(0);
    result.items_returned = Some(0);
    assert!(super::bsl_result_proves_full_verdict(&result));
}

/// `check` модуля под базовой линией: доказанный фильтр — полный
/// вердикт с названным фактом подавления; сломанная линия — отказ,
/// который называет причину поставщика, а не «анализ не закончился».
/// rule: INV.APP.DIAGNOSTIC-SUPPRESSION-EVIDENCE
#[test]
fn bsl_check_passes_under_a_proven_baseline_and_names_a_broken_one() {
    use unica_diagnostics::model::DiagnosticAction;
    use unica_diagnostics::model::DiagnosticProviderSection;
    use unica_diagnostics::model::DiagnosticProviderStatus;
    use unica_diagnostics::model::DiagnosticResult;
    use unica_diagnostics::model::DiagnosticResultState;
    use unica_diagnostics::model::DiagnosticSelection;
    use unica_diagnostics::model::DiagnosticSuppression;
    use unica_diagnostics::model::DiagnosticSuppressionReason;
    let result = |complete: bool, reason: Option<DiagnosticSuppressionReason>| DiagnosticResult {
        ok: true,
        action: DiagnosticAction::Analyze,
        selection: DiagnosticSelection {
            source_set: "main".to_string(),
            metadata_path: None,
            target_kind: None,
            providers: vec!["bsl-analyzer"],
            filter: None,
            limit: None,
        },
        state: if complete {
            DiagnosticResultState::Completed
        } else {
            DiagnosticResultState::Partial
        },
        complete,
        providers: vec![DiagnosticProviderSection {
            id: "bsl-analyzer",
            status: DiagnosticProviderStatus::Completed,
            complete,
            version: None,
            capabilities: None,
            readiness: None,
            items_total: Some(0),
            items_returned: Some(0),
            resource_failures: Some(0),
            truncated: Some(false),
            error: None,
            suppressions: vec![DiagnosticSuppression::Baseline {
                known: Some(2),
                new: Some(0),
                reason,
            }],
        }],
        items_total: Some(0),
        items_returned: Some(0),
        truncated: Some(false),
        items: Vec::new(),
    };

    let filtered = result(true, None);
    assert!(super::bsl_result_proves_full_verdict(&filtered));
    assert!(super::bsl_findings_passed(&filtered.items));
    assert_eq!(
        super::bsl_suppression_facts(&filtered),
        vec![serde_json::json!({
            "provider": "bsl-analyzer", "by": "baseline", "known": 2, "new": 0
        })]
    );

    let mut combined = filtered.clone();
    combined.providers[0]
        .suppressions
        .push(DiagnosticSuppression::Authors {
            count: std::num::NonZeroUsize::new(3).unwrap(),
        });
    assert!(super::bsl_result_proves_full_verdict(&combined));
    assert_eq!(
        super::bsl_suppression_facts(&combined),
        vec![
            serde_json::json!({"provider":"bsl-analyzer","by":"baseline","known":2,"new":0}),
            serde_json::json!({"provider":"bsl-analyzer","by":"authors","count":3}),
        ]
    );
    combined.complete = false;
    combined.state = DiagnosticResultState::Partial;
    combined.providers[0].complete = false;
    combined.items.push(unica_diagnostics::model::DiagnosticItem::ResourceFailure {
            provider:"bsl-analyzer",
            location:unica_source_contract::location::SourceLocation::Addressed {
                source_set:"main".to_string(),metadata_path:None,
                target_kind:unica_address::source_target::TargetKind::Module,
            },location_reason:None,
            error:unica_diagnostics::model::DiagnosticError {
                code:"analysis_filter_unproven".to_string(),
                message:"CLI author filtering is configured; this analyzer does not report which findings it hid".to_string(),retryable:false,
            },
        });
    assert!(!super::bsl_result_proves_full_verdict(&combined));
    assert_eq!(super::bsl_incomplete_message(&combined),
            "BSL analysis did not complete, so the module is unproven: CLI author filtering is configured; this analyzer does not report which findings it hid");

    let broken = result(
        false,
        Some(DiagnosticSuppressionReason {
            code: "invalid_schema".to_string(),
            detail: Some("baseline schema 7 is newer".to_string()),
        }),
    );
    assert!(!super::bsl_result_proves_full_verdict(&broken));
    assert_eq!(
        super::bsl_incomplete_message(&broken),
        "BSL analysis did not complete, so the module is unproven: \
             diagnostics baseline invalid_schema: baseline schema 7 is newer"
    );

    // Без названной причины отказ всё равно говорит, что делать.
    let mut unnamed = result(false, None);
    unnamed.providers[0].suppressions.clear();
    assert_eq!(
        super::bsl_incomplete_message(&unnamed),
        format!(
            "{}: {}",
            super::BSL_INCOMPLETE_MESSAGE,
            super::BSL_UNNAMED_INCOMPLETENESS
        )
    );
}

/// rule: INV.APP.DIAGNOSTIC-PROVIDERS
#[test]
fn bsl_check_preserves_only_canonical_jsonl_failure_recovery() {
    use unica_diagnostics::model::DiagnosticAction;
    use unica_diagnostics::model::DiagnosticError;
    use unica_diagnostics::model::DiagnosticProviderSection;
    use unica_diagnostics::model::DiagnosticProviderStatus;
    use unica_diagnostics::model::DiagnosticResult;
    use unica_diagnostics::model::DiagnosticResultState;
    use unica_diagnostics::model::DiagnosticSelection;
    use unica_diagnostics::stream_error::StreamErrorKind;
    use unica_diagnostics::stream_error::MISSING_START_MESSAGE;
    let result = |code: &str, message: String| DiagnosticResult {
        ok: false,
        action: DiagnosticAction::Analyze,
        selection: DiagnosticSelection {
            source_set: "main".to_string(),
            metadata_path: None,
            target_kind: None,
            providers: vec!["bsl-analyzer"],
            filter: None,
            limit: None,
        },
        state: DiagnosticResultState::Failed,
        complete: false,
        providers: vec![DiagnosticProviderSection {
            id: "bsl-analyzer",
            status: DiagnosticProviderStatus::Failed,
            complete: false,
            version: None,
            capabilities: None,
            readiness: None,
            items_total: Some(0),
            items_returned: Some(0),
            resource_failures: Some(0),
            truncated: Some(false),
            suppressions: Vec::new(),
            error: Some(DiagnosticError {
                code: code.to_string(),
                message,
                retryable: false,
            }),
        }],
        items_total: Some(0),
        items_returned: Some(0),
        truncated: Some(false),
        items: Vec::new(),
    };
    for safe in [
        StreamErrorKind::InvalidEvent.message(2),
        StreamErrorKind::UnknownSeverity.message(3),
        MISSING_START_MESSAGE.to_string(),
    ] {
        let failed = result("diagnostics_invalid", safe.clone());
        assert!(!super::bsl_result_proves_full_verdict(&failed));
        assert_eq!(
            super::bsl_incomplete_message(&failed),
            format!("{}: {safe}", super::BSL_INCOMPLETE_MESSAGE)
        );
    }
    // A message that is not the closed formatter's whole output is never
    // published; the closed code still names the cause in fixed words.
    let safe = StreamErrorKind::InvalidEvent.message(2);
    for (code, message) in [
        ("provider_failed", safe.clone()),
        ("diagnostics_invalid", format!("private-prefix {safe}")),
        (
            "diagnostics_invalid",
            format!("{safe} secret=/private/synthetic-secret"),
        ),
        ("diagnostics_invalid", safe.replace("line 2:", "line 02:")),
    ] {
        let published = super::bsl_incomplete_message(&result(code, message));
        assert_eq!(
            published,
            format!(
                "{}: bsl-analyzer: {}",
                super::BSL_INCOMPLETE_MESSAGE,
                unica_diagnostics::coordinator::public_diagnostic_error_message(code)
            )
        );
        assert!(!published.contains("line 2"), "{published}");
        assert!(!published.contains("private"), "{published}");
    }
}

/// A provider that did not run names which engine, what happened to it
/// and what restores it; the provider's own text is never the reason
/// (#1338).
/// rule: INV.APP.DIAGNOSTIC-PROVIDERS
#[test]
fn bsl_check_names_why_the_engine_gave_no_result_and_how_to_recover() {
    use unica_diagnostics::model::DiagnosticAction;
    use unica_diagnostics::model::DiagnosticError;
    use unica_diagnostics::model::DiagnosticProviderSection;
    use unica_diagnostics::model::DiagnosticProviderStatus;
    use unica_diagnostics::model::DiagnosticResult;
    use unica_diagnostics::model::DiagnosticResultState;
    use unica_diagnostics::model::DiagnosticSelection;
    let failed = |code: &str| DiagnosticResult {
        ok: false,
        action: DiagnosticAction::Analyze,
        selection: DiagnosticSelection {
            source_set: "erf".to_string(),
            metadata_path: None,
            target_kind: None,
            providers: vec!["bsl-analyzer"],
            filter: None,
            limit: None,
        },
        state: DiagnosticResultState::Failed,
        complete: false,
        providers: vec![DiagnosticProviderSection {
            id: "bsl-analyzer",
            status: DiagnosticProviderStatus::Unavailable,
            complete: false,
            version: None,
            capabilities: None,
            readiness: None,
            items_total: Some(0),
            items_returned: Some(0),
            resource_failures: Some(0),
            truncated: Some(false),
            suppressions: Vec::new(),
            error: Some(DiagnosticError {
                code: code.to_string(),
                message: "private-token /private/engine stderr".to_string(),
                retryable: false,
            }),
        }],
        items_total: Some(0),
        items_returned: Some(0),
        truncated: Some(false),
        items: Vec::new(),
    };
    for (code, cause, recovery, absent) in [
        (
            "bundled_tool_missing",
            "could not locate its plugin installation",
            "Reinstall the Unica plugin",
            true,
        ),
        (
            "provider_install_invalid",
            "failed verification",
            "Reinstall the Unica plugin",
            true,
        ),
        (
            "provider_start_failed",
            "could not be started",
            "then repeat the call",
            true,
        ),
        (
            "provider_timeout",
            "exceeded its time limit",
            "raise the configured diagnostics time limit",
            false,
        ),
        (
            "provider_exited",
            "exited with an error",
            "Repeat the call",
            false,
        ),
        (
            "provider_unavailable",
            "unclassified reason",
            "Repeat the call",
            false,
        ),
    ] {
        let result = failed(code);
        assert!(!super::bsl_result_proves_full_verdict(&result));
        let message = super::bsl_incomplete_message(&result);
        assert!(
            message.starts_with(&format!(
                "{}: bsl-analyzer: ",
                super::BSL_INCOMPLETE_MESSAGE
            )),
            "{code}: {message}"
        );
        assert!(message.contains(cause), "{code}: {message}");
        assert!(message.contains(recovery), "{code}: {message}");
        assert!(!message.contains("private"), "{code}: {message}");
        assert_eq!(super::bsl_provider_absent(&result), absent, "{code}");
    }

    // An absent engine described by its install mode keeps that next
    // step; the expected path never reaches the answer.
    for mode in [
        unica_artifact_delivery::engine::InstallMode::Source,
        unica_artifact_delivery::engine::InstallMode::Marketplace,
    ] {
        let missing = unica_artifact_delivery::engine::MissingEngine::new(
            "bsl-analyzer",
            "win-x64",
            "C:/private/engine/bsl-analyzer.exe",
            None,
            mode,
        );
        let mut result = failed("bundled_tool_missing");
        result.providers[0].error.as_mut().unwrap().message =
            unica_diagnostics::coordinator::missing_engine_public_reason(&missing);
        let message = super::bsl_incomplete_message(&result);
        assert_eq!(
                message,
                format!(
                    "{}: bsl-analyzer is not available for win-x64: its file is not on this machine; {}",
                    super::BSL_INCOMPLETE_MESSAGE,
                    missing.next_step
                )
            );
        assert!(!message.contains("private"), "{message}");
    }
}

/// rule: INV.WIRE.CHECK-DIAGNOSTIC-PAGES
#[test]
fn bsl_check_sees_an_error_after_two_hundred_warnings() {
    use unica_address::source_target::TargetKind;
    use unica_diagnostics::model::DiagnosticFocus;
    use unica_diagnostics::model::DiagnosticItem;
    use unica_diagnostics::model::DiagnosticSeverity;
    use unica_source_contract::location::SourceLocation;
    let finding = |severity| DiagnosticItem::Diagnostic {
        provider: "bsl-language-server",
        location: SourceLocation::Addressed {
            source_set: "main".to_string(),
            metadata_path: None,
            target_kind: TargetKind::Module,
        },
        location_reason: None,
        focus: DiagnosticFocus::Target,
        code: "late-error".to_string(),
        severity,
        message: "finding".to_string(),
        tags: Vec::new(),
    };
    let mut items = vec![finding(DiagnosticSeverity::Warning); 200];
    assert!(super::bsl_findings_passed(&items));
    items.push(finding(DiagnosticSeverity::Error));
    assert!(!super::bsl_findings_passed(&items));
}

struct SectionSearchProvider {
    section: ProviderSearchSection,
    calls: Arc<AtomicUsize>,
}

impl CodeIntelligenceProvider for SectionSearchProvider {
    fn identity(&self) -> ProviderIdentity {
        self.section.identity.clone()
    }

    fn capabilities(&self) -> &[ProviderCapability] {
        &[ProviderCapability::Search]
    }

    fn search(
        &self,
        request: &SearchRequest,
        _context: &CodeIntelligenceContext,
        _deadline: ProviderDeadline,
        _cancellation: &CancellationToken,
    ) -> ProviderSearchSection {
        assert_eq!(request.query, "Needle");
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.section.clone()
    }
}

fn role_search_workspace() -> tempfile::TempDir {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::create_dir(workspace.path().join("src")).unwrap();
    std::fs::write(
        workspace.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(workspace.path().join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#).unwrap();
    workspace
}

fn resolve_exact_workspace() -> tempfile::TempDir {
    let workspace = role_search_workspace();
    let source = workspace.path().join("src");
    let descriptor = |kind: &str, name: &str, children: &str| {
        let (uuid, extra) = match kind {
                "Configuration" => ("f3317000-0000-4000-8000-000000000001", ""),
                "CommonModule" => ("f3317000-0000-4000-8000-000000000002", "<Global>false</Global><ClientManagedApplication>false</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>false</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse>"),
                "Catalog" => ("f3317000-0000-4000-8000-000000000003", ""),
                "Form" => ("f3317000-0000-4000-8000-000000000004", "<FormType>Managed</FormType>"),
                "CommonForm" => ("f3317000-0000-4000-8000-000000000005", "<FormType>Managed</FormType>"),
                _ => unreachable!(),
            };
        format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{kind} uuid="{uuid}"><Properties><Name>{name}</Name>{extra}</Properties><ChildObjects>{children}</ChildObjects></{kind}></MetaDataObject>"#
        )
    };
    std::fs::write(source.join("Configuration.xml"), descriptor("Configuration", "Store", "<CommonModule>Probe</CommonModule><Catalog>Items</Catalog><CommonForm>Main</CommonForm>")).unwrap();
    for directory in [
        "CommonModules/Probe/Ext",
        "Catalogs/Items/Ext",
        "Catalogs/Items/Forms/Main/Ext",
        "CommonForms/Main/Ext",
    ] {
        std::fs::create_dir_all(source.join(directory)).unwrap();
    }
    std::fs::write(
        source.join("CommonModules/Probe.xml"),
        descriptor("CommonModule", "Probe", ""),
    )
    .unwrap();
    std::fs::write(source.join("Catalogs/Items.xml"), descriptor("Catalog", "Items", r#"<Form>Main</Form><Attribute><Properties><Name>Value</Name></Properties></Attribute><TabularSection><Properties><Name>Rows</Name></Properties><ChildObjects><Attribute><Properties><Name>Amount</Name></Properties></Attribute></ChildObjects></TabularSection>"#)).unwrap();
    std::fs::write(
        source.join("Catalogs/Items/Forms/Main.xml"),
        descriptor("Form", "Main", ""),
    )
    .unwrap();
    std::fs::write(
        source.join("CommonForms/Main.xml"),
        descriptor("CommonForm", "Main", ""),
    )
    .unwrap();
    let form = r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems><InputField name="Value" id="1"/><UsualGroup name="Group" id="2"><ChildItems><InputField name="Inside" id="3"/></ChildItems></UsualGroup></ChildItems><Attributes><Attribute name="FormValue" id="1"/></Attributes></Form>"#;
    for file in [
        "Catalogs/Items/Forms/Main/Ext/Form.xml",
        "CommonForms/Main/Ext/Form.xml",
    ] {
        std::fs::write(source.join(file), form).unwrap();
    }
    let body = "    Сообщить(\"probe\");\n".repeat(61);
    let module = format!("#Область A\n#Область Inner\nПроцедура Ping() Экспорт\n{body}КонецПроцедуры\n#КонецОбласти\n#КонецОбласти\n#Область B\n#Область Inner\n// second\n#КонецОбласти\n#КонецОбласти\n");
    for file in [
        "CommonModules/Probe/Ext/Module.bsl",
        "Catalogs/Items/Ext/ObjectModule.bsl",
        "Catalogs/Items/Ext/ManagerModule.bsl",
        "Catalogs/Items/Forms/Main/Ext/Form/Module.bsl",
        "CommonForms/Main/Ext/Form/Module.bsl",
    ] {
        let target = source.join(file);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, &module).unwrap();
    }
    assert!(std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(workspace.path())
        .status()
        .unwrap()
        .success());
    workspace
}

fn invoke_exact_read(
    workspace: &std::path::Path,
    tool: unica_execution::invocation_store::ToolIdentity,
    arguments: serde_json::Value,
) -> unica_execution_contract::invocation::DomainResult {
    invoke_exact_read_with_service(
        workspace,
        tool,
        arguments,
        Arc::new(super::CanonicalV13ReadService::default()),
    )
}

fn invoke_exact_read_with_service(
    workspace: &std::path::Path,
    tool: unica_execution::invocation_store::ToolIdentity,
    arguments: serde_json::Value,
    service: Arc<super::CanonicalV13ReadService>,
) -> unica_execution_contract::invocation::DomainResult {
    use crate::infrastructure::daemon::protocol::InvocationRequest;
    use crate::infrastructure::daemon::server::V5CanonicalInvocationRuntime;
    use unica_execution::clock::TokioClock;
    let runtime = V5CanonicalInvocationRuntime::new(service, Arc::new(TokioClock));
    runtime
        .bind(
            InvocationRequest::new(
                tool,
                arguments,
                std::fs::canonicalize(workspace).unwrap().to_string_lossy(),
                7_000,
            )
            .unwrap(),
        )
        .unwrap()
        .prepare()
        .unwrap()
        .execute(CancellationToken::new())
        .unwrap()
}

/// rule: INV.WIRE.RESOLVED-SOURCE-LOCATION
#[test]
fn resolve_exact_bsl_nodes_across_module_roles() {
    use unica_execution::invocation_store::ToolIdentity;
    let workspace = resolve_exact_workspace();
    for (module, path) in [
        ("CommonModule.Probe", "CommonModules/Probe/Ext/Module.bsl"),
        (
            "Catalog.Items.Module.Object",
            "Catalogs/Items/Ext/ObjectModule.bsl",
        ),
        (
            "Catalog.Items.Module.Manager",
            "Catalogs/Items/Ext/ManagerModule.bsl",
        ),
        (
            "Catalog.Items.Form.Main.Module.Form",
            "Catalogs/Items/Forms/Main/Ext/Form/Module.bsl",
        ),
        (
            "CommonForm.Main.Module.Form",
            "CommonForms/Main/Ext/Form/Module.bsl",
        ),
    ] {
        for (suffix, kind, from, to) in [
            (".Method.Ping", "Method", 3, 65),
            (".Method.Ping.Body", "Body", 3, 65),
            (".Body", "Body", 1, 72),
            (".Region.A.Region.Inner", "Region", 2, 66),
            (".Region.B.Region.Inner", "Region", 69, 71),
        ] {
            let at = format!("main:{module}{suffix}");
            let result =
                invoke_exact_read(workspace.path(), ToolIdentity::Resolve, json!({"at":at}));
            assert!(result.ok, "{at}: {result:?}");
            let data = result.data.as_ref().unwrap();
            assert_eq!(data["at"], at);
            assert_eq!(data["kind"], kind, "{result:?}");
            assert_eq!(data["path"], path, "{result:?}");
            assert_eq!(
                data["lines"],
                json!({"state":"range","from":from,"to":to}),
                "{result:?}"
            );
            assert!(result.rev.is_none());
        }
        for suffix in [
            ".Method.Missing",
            ".Method.Missing.Body",
            ".Region.Missing",
            ".Region.Missing.Region.A",
            ".Region.A.Region.Missing",
            ".Region.A.Region.Inner.Region.B",
        ] {
            let result = invoke_exact_read(
                workspace.path(),
                ToolIdentity::Resolve,
                json!({"at":format!("main:{module}{suffix}")}),
            );
            assert!(!result.ok, "{result:?}");
            assert_eq!(
                serde_json::to_value(&result).unwrap()["diagnostics"][0]["code"],
                "not_found",
                "{result:?}"
            );
        }
    }
}

/// Ответ пакета по позиции входа: `items[i]` отвечает на `paths[i]`.
fn resolve_paths_items(workspace: &std::path::Path, paths: &[&str]) -> Vec<serde_json::Value> {
    let result = invoke_exact_read(
        workspace,
        unica_execution::invocation_store::ToolIdentity::Resolve,
        json!({ "paths": paths }),
    );
    assert!(result.ok, "{result:?}");
    assert!(result.rev.is_none(), "{result:?}");
    let items = result.data.as_ref().unwrap()["items"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(items.len(), paths.len(), "{result:?}");
    items
}

fn single_path(workspace: &std::path::Path, path: &str) -> serde_json::Value {
    let result = invoke_exact_read(
        workspace,
        unica_execution::invocation_store::ToolIdentity::Resolve,
        json!({ "path": path }),
    );
    assert!(result.ok, "{path}: {result:?}");
    result.data.unwrap()
}

/// rule: INV.WIRE.RESOLVE-PATH-BATCH
#[test]
fn resolve_paths_translates_paths_and_lines_in_input_order() {
    let workspace = resolve_exact_workspace();
    let root = workspace.path();
    let common = "src/CommonModules/Probe/Ext/Module.bsl";
    let object = "src/Catalogs/Items/Ext/ObjectModule.bsl";
    let form = "src/Catalogs/Items/Forms/Main/Ext/Form/Module.bsl";
    let owner = |path: &str| {
        let single = single_path(root, path);
        json!({"at": single["at"], "kind": single["kind"]})
    };
    let with_line = |path: &str, line: &str| {
        let mut item = owner(path);
        item["line"] = json!(line);
        item
    };
    let method = |at: &str| json!({"at": at, "kind": "Method", "line": "inMethod"});
    let items = resolve_paths_items(
        root,
        &[
            &format!("{common}:4"),
            &format!("{}:70", common.replace('/', "\\")),
            &format!("{object}:3"),
            &format!("{object}:65"),
            object,
            &format!("{form}:10"),
            "src/Catalogs/Items.xml:3",
            &format!("{common}:4"),
            &format!("{object}:1"),
            "src/Catalogs/Items/Forms/Main/Ext/Form.xml:1",
            &format!(
                "{}:4",
                std::fs::canonicalize(root).unwrap().join(object).display()
            ),
        ],
    );
    assert_eq!(
        items,
        vec![
            method("main:CommonModule.Probe.Method.Ping"),
            with_line(common, "outsideMethod"),
            method("main:Catalog.Items.Module.Object.Method.Ping"),
            method("main:Catalog.Items.Module.Object.Method.Ping"),
            owner(object),
            method("main:Catalog.Items.Form.Main.Module.Form.Method.Ping"),
            with_line("src/Catalogs/Items.xml", "notLineBased"),
            method("main:CommonModule.Probe.Method.Ping"),
            with_line(object, "outsideMethod"),
            with_line("src/Catalogs/Items/Forms/Main/Ext/Form.xml", "notLineBased"),
            method("main:Catalog.Items.Module.Object.Method.Ping"),
        ]
    );
    assert_eq!(items[9]["at"], "main:Catalog.Items.Form.Main");
    assert_eq!(items[1]["at"], "main:CommonModule.Probe", "{items:?}");
    assert_eq!(items[8]["at"], "main:Catalog.Items.Module.Object");
    assert_eq!(items[6]["at"], "main:Catalog.Items");
}

/// rule: INV.WIRE.RESOLVE-PATH-BATCH
#[test]
fn resolve_paths_refuses_only_the_failing_item() {
    let workspace = resolve_exact_workspace();
    let root = workspace.path();
    let object = "src/Catalogs/Items/Ext/ObjectModule.bsl";
    let outside = root
        .parent()
        .unwrap()
        .join("Elsewhere/Ext/ObjectModule.bsl");
    // Дескриптор и модуль есть, но конфигурация каталог не регистрирует.
    std::fs::write(
            root.join("src/Catalogs/Orphan.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="f3317000-0000-4000-8000-0000000000aa"><Properties><Name>Orphan</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
        )
        .unwrap();
    let orphan = root.join("src/Catalogs/Orphan/Ext");
    std::fs::create_dir_all(&orphan).unwrap();
    std::fs::write(
        orphan.join("ObjectModule.bsl"),
        "Процедура Ping()\nКонецПроцедуры\n",
    )
    .unwrap();
    let items = resolve_paths_items(
        root,
        &[
            &format!("{object}:73"),
            "src/Catalogs/Missing/Ext/ObjectModule.bsl:1",
            &format!("{}:1", outside.display()),
            &format!("{object}:0"),
            &format!("{object}:3:5"),
            "",
            &format!("{object}:3"),
            "src/Catalogs/Orphan/Ext/ObjectModule.bsl:1",
        ],
    );
    // Владелец не зарегистрирован в конфигурации, хотя файл модуля есть.
    assert_eq!(items[7]["code"], "not_found", "{items:?}");
    assert!(
        items[7]["message"]
            .as_str()
            .unwrap()
            .contains("not registered"),
        "{items:?}"
    );
    let items = &items[..7];
    let codes = items
        .iter()
        .map(|item| item.get("code").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(
        codes,
        [
            Some("not_found"),
            Some("not_found"),
            Some("not_found"),
            Some("bad_value"),
            Some("bad_value"),
            Some("bad_value"),
            None,
        ],
        "{items:?}"
    );
    assert_eq!(items[0]["at"], "main:Catalog.Items.Module.Object");
    assert!(
        items[0]["message"].as_str().unwrap().contains("72 lines"),
        "{items:?}"
    );
    for refused in &items[..6] {
        assert!(refused.get("kind").is_none(), "{refused:?}");
        assert!(refused.get("outcome").is_none(), "{refused:?}");
        assert!(refused["message"].as_str().is_some(), "{refused:?}");
    }
    assert_eq!(
        items[6]["at"],
        "main:Catalog.Items.Module.Object.Method.Ping"
    );
}

/// rule: INV.WIRE.RESOLVE-PATH-BATCH
#[test]
fn resolve_paths_places_lines_by_the_method_range_resolve_names() {
    let workspace = resolve_exact_workspace();
    let module = concat!(
            "// Документация.\n\n&НаСервере\n&Перед(\"Ping\")\nПроцедура Annotated(\n",
            "    Value) Экспорт\n    Сообщить(Value);\nКонецПроцедуры\n\n",
            "Процедура Empty()\nКонецПроцедуры\nФункция Inline() Возврат 1; КонецФункции\n",
            "Асинх\nПроцедура Waiting()\nКонецПроцедуры\n",
            "Async\nProcedure Waiting2()\nEndProcedure\n",
            "// Doc\n// Параметры:\n&НаСервере\nПроцедура Documented()\n    Сообщить(1);\nКонецПроцедуры\n",
        );
    let path = "src/CommonModules/Probe/Ext/Module.bsl";
    std::fs::write(workspace.path().join(path), module).unwrap();
    let lines = [1, 3, 8, 9, 12, 13, 19, 20, 21, 24];
    let items = resolve_paths_items(
        workspace.path(),
        &lines
            .iter()
            .map(|line| format!("{path}:{line}"))
            .collect::<Vec<_>>()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    let placed = items
        .iter()
        .map(|item| (item["at"].as_str().unwrap(), item["line"].as_str().unwrap()))
        .collect::<Vec<_>>();
    let method = |name: &'static str| -> (&'static str, &'static str) {
        (
            Box::leak(format!("main:CommonModule.Probe.Method.{name}").into_boxed_str()),
            "inMethod",
        )
    };
    let outside = ("main:CommonModule.Probe", "outsideMethod");
    // Документирующий комментарий входит в `Body`, но не в диапазон
    // `Method`: строка над методом отвечает модулем, как и `resolve at`.
    assert_eq!(
        placed,
        vec![
            outside,
            method("Annotated"),
            method("Annotated"),
            outside,
            method("Inline"),
            method("Waiting"),
            outside,
            outside,
            method("Documented"),
            method("Documented"),
        ]
    );
}

/// rule: INV.WIRE.RESOLVE-EXACT-BRIDGE
#[test]
fn resolve_paths_is_exclusive_with_at_and_path_and_takes_only_strings() {
    use crate::infrastructure::daemon::protocol::InvocationRequest;
    use crate::infrastructure::daemon::server::V5CanonicalInvocationRuntime;
    use unica_execution::clock::TokioClock;
    use unica_execution::invocation_store::ToolIdentity;
    use unica_execution::receipt_runtime::ports::V5CanonicalPrepareError;
    let workspace = resolve_exact_workspace();
    let root = std::fs::canonicalize(workspace.path()).unwrap();
    for arguments in [
        json!({"paths": ["src/Catalogs/Items.xml"], "path": "src/Catalogs/Items.xml"}),
        json!({"paths": ["src/Catalogs/Items.xml"], "at": "main:Catalog.Items"}),
        json!({"paths": []}),
        json!({"paths": [1]}),
        json!({"paths": "src/Catalogs/Items.xml"}),
    ] {
        // Форму вызова отклоняет либо проверка схемы на входе, либо сам
        // мост; исход для вызывающего один — `bad_value` всего вызова.
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(super::CanonicalV13ReadService::default()),
            Arc::new(TokioClock),
        );
        let request = InvocationRequest::new(
            ToolIdentity::Resolve,
            arguments.clone(),
            root.to_string_lossy(),
            7_000,
        )
        .unwrap();
        let result = match runtime.bind(request) {
            Err(V5CanonicalPrepareError::Rejected(result)) => *result,
            Err(error) => panic!("{arguments}: {error:?}"),
            Ok(bound) => bound
                .prepare()
                .unwrap()
                .execute(CancellationToken::new())
                .unwrap(),
        };
        assert!(!result.ok, "{arguments}: {result:?}");
        assert_eq!(
            serde_json::to_value(&result).unwrap()["diagnostics"][0]["code"],
            "bad_value",
            "{arguments}: {result:?}"
        );
    }
}

/// rule: INV.WIRE.RESOLVED-SOURCE-LOCATION
#[test]
fn view_nested_regions_never_substitutes_the_final_name() {
    use unica_execution::invocation_store::ToolIdentity;
    let workspace = resolve_exact_workspace();
    for prefix in ["CommonModule.Probe", "Catalog.Items.Module.Object"] {
        for (suffix, ok) in [
            (".Region.A.Region.Inner", true),
            (".Region.B.Region.Inner", true),
            (".Region.Missing.Region.A", false),
            (".Region.A.Region.Inner.Region.B", false),
        ] {
            let at = format!("main:{prefix}{suffix}");
            let result = invoke_exact_read(workspace.path(), ToolIdentity::View, json!({"at":at}));
            assert_eq!(result.ok, ok, "{at}: {result:?}");
            if !ok {
                assert_eq!(
                    serde_json::to_value(&result).unwrap()["diagnostics"][0]["code"],
                    "not_found"
                );
            }
        }
    }
    // Уникальное имя вложенной области тоже не заменяет её полный адрес.
    std::fs::write(
        workspace
            .path()
            .join("src/CommonModules/Probe/Ext/Module.bsl"),
        "#Область Outer\n#Область Uniq\n#КонецОбласти\n#КонецОбласти\n",
    )
    .unwrap();
    for tool in [ToolIdentity::View, ToolIdentity::Resolve] {
        for (suffix, ok) in [(".Region.Outer.Region.Uniq", true), (".Region.Uniq", false)] {
            let at = format!("main:CommonModule.Probe{suffix}");
            let result = invoke_exact_read(workspace.path(), tool, json!({"at":at}));
            assert_eq!(result.ok, ok, "{tool:?} {at}: {result:?}");
            if !ok {
                assert_eq!(
                    serde_json::to_value(&result).unwrap()["diagnostics"][0]["code"],
                    "not_found"
                );
            }
        }
    }
}

/// A name search proves its scope only through the addresses it
/// watched while walking; a form scope needs its owner among them.
#[test]
fn name_scope_witnesses_cover_every_address_the_scope_check_asks_for() {
    use unica_address::address::QualifiedAddress;
    use unica_refusal::RefusalCode;
    use unica_source_contract::SourceSetKind;

    let scope = QualifiedAddress::parse("main:Catalog.Валюты.Form.ФормаЭлемента").unwrap();
    let witnesses = super::name_scope_witnesses(&scope);
    let mut asked = Vec::new();
    let recorded = std::cell::RefCell::new(&mut asked);
    let _ = super::validate_name_scope(
        &|at| {
            recorded.borrow_mut().push(at.to_string());
            true
        },
        &scope,
        SourceSetKind::Configuration,
    );
    assert!(!asked.is_empty());
    for at in &asked {
        assert!(witnesses.contains(at), "unwatched address {at}");
    }
    let present = |missing: &str| {
        let witnesses = witnesses.clone();
        let missing = missing.to_string();
        move |at: &str| witnesses.iter().any(|known| known == at) && at != missing
    };
    assert!(
        super::validate_name_scope(&present("none"), &scope, SourceSetKind::Configuration).is_ok()
    );
    assert_eq!(
        super::validate_name_scope(
            &present("main:Catalog.Валюты"),
            &scope,
            SourceSetKind::Configuration
        )
        .unwrap_err()
        .0,
        RefusalCode::NotFound
    );
    assert_eq!(
        super::validate_name_scope(
            &present("main:Configuration"),
            &scope,
            SourceSetKind::Configuration
        )
        .unwrap_err()
        .0,
        RefusalCode::InvalidState
    );
}

/// Resolving a metadata object by its own place answers exactly what the
/// full layout directory answers, for every object shape it handles (#1457).
#[test]
fn resolve_metadata_point_matches_the_full_layout_directory() {
    use crate::infrastructure::v13_find::{LayoutFindSource, WorkspaceFindDirectoryBuilder};
    use unica_execution::invocation_store::ToolIdentity;
    use unica_workspace::read::open_source_directory;
    let workspace = resolve_exact_workspace();
    let source = std::fs::canonicalize(workspace.path().join("src")).unwrap();
    std::fs::create_dir_all(source.join("Catalogs/Items/Commands/Refresh/Ext")).unwrap();
    std::fs::write(
        source.join("Catalogs/Items/Commands/Refresh/Ext/CommandModule.bsl"),
        "",
    )
    .unwrap();
    let root = open_source_directory(&source).unwrap();
    let directory = WorkspaceFindDirectoryBuilder::default()
        .build(
            &[LayoutFindSource::new(
                "main",
                unica_source_contract::SourceSetKind::Configuration,
                &root,
            )],
            ProviderDeadline::no_deadline(),
            &CancellationToken::new(),
        )
        .unwrap();
    for at in [
        "main:Configuration",
        "main:CommonModule.Probe",
        "main:Catalog.Items",
        "main:Catalog.Items.Form.Main",
        "main:Catalog.Items.Command.Refresh",
        "main:CommonForm.Main",
    ] {
        let expected = directory
            .locate_address(at)
            .unwrap_or_else(|| panic!("the directory places {at}"));
        let is_form = matches!(expected.kind(), "Form" | "CommonForm");
        let query = if is_form {
            format!("{at}.Metadata")
        } else {
            at.to_string()
        };
        let result =
            invoke_exact_read(workspace.path(), ToolIdentity::Resolve, json!({"at":query}));
        assert!(result.ok, "{query}: {result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(
            data["kind"],
            if is_form { "Metadata" } else { expected.kind() },
            "{query}"
        );
        assert_eq!(data["path"], expected.placed_path().unwrap(), "{query}");
        if is_form {
            let content =
                invoke_exact_read(workspace.path(), ToolIdentity::Resolve, json!({"at":at}));
            assert!(content.ok, "{at}: {content:?}");
            assert_eq!(
                content.data.unwrap()["path"],
                format!(
                    "{}/Ext/Form.xml",
                    expected
                        .placed_path()
                        .unwrap()
                        .strip_suffix(".xml")
                        .unwrap()
                )
            );
        }
    }
    for at in ["main:Catalog.Missing", "main:Catalog.Items.Command.Missing"] {
        assert!(directory.locate_address(at).is_none());
        let result = invoke_exact_read(workspace.path(), ToolIdentity::Resolve, json!({"at":at}));
        assert!(!result.ok, "{at}: {result:?}");
        assert_eq!(result.diagnostics[0]["code"], "not_found", "{at}");
    }
    std::fs::remove_file(source.join("Catalogs/Items.xml")).unwrap();
    let orphan = invoke_exact_read(
        workspace.path(),
        ToolIdentity::Resolve,
        json!({"at":"main:Catalog.Items.Form.Main"}),
    );
    assert!(!orphan.ok);
    assert_eq!(orphan.diagnostics[0]["code"], "invalid_source");
    assert!(
        orphan.diagnostics[0]["message"]
            .as_str()
            .is_some_and(
                |message| message.contains("`Catalogs/Items.xml`") && message.contains("missing")
            ),
        "the refusal names the missing owner descriptor: {:?}",
        orphan.diagnostics[0]["message"]
    );
}

/// rule: INV.WIRE.RESOLVED-SOURCE-LOCATION
#[test]
fn resolve_exact_tree_children_use_their_containing_xml() {
    use unica_execution::invocation_store::ToolIdentity;
    let workspace = resolve_exact_workspace();
    for (node, kind, path) in [
        (
            "CommonModule.Probe",
            "CommonModule",
            "CommonModules/Probe.xml",
        ),
        (
            "Catalog.Items.Attribute.Value",
            "Attribute",
            "Catalogs/Items.xml",
        ),
        (
            "Catalog.Items.TabularSection.Rows.Attribute.Amount",
            "Attribute",
            "Catalogs/Items.xml",
        ),
        (
            "Catalog.Items.Form.Main.Item.Value",
            "Item",
            "Catalogs/Items/Forms/Main/Ext/Form.xml",
        ),
        (
            "Catalog.Items.Form.Main.Item.Group.Item.Inside",
            "Item",
            "Catalogs/Items/Forms/Main/Ext/Form.xml",
        ),
        (
            "Catalog.Items.Form.Main.Attribute.FormValue",
            "Attribute",
            "Catalogs/Items/Forms/Main/Ext/Form.xml",
        ),
        (
            "CommonForm.Main.Item.Value",
            "Item",
            "CommonForms/Main/Ext/Form.xml",
        ),
    ] {
        let at = format!("main:{node}");
        let view = invoke_exact_read(workspace.path(), ToolIdentity::View, json!({"at":at}));
        assert!(view.ok, "fixture must independently expose {at}: {view:?}");
        let result = invoke_exact_read(workspace.path(), ToolIdentity::Resolve, json!({"at":at}));
        assert!(result.ok, "{at}: {result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["at"], at);
        assert_eq!(data["kind"], kind, "{result:?}");
        assert_eq!(data["path"], path, "{result:?}");
        assert_eq!(data["lines"], json!({"state":"notLineBased"}));
        assert!(result.rev.is_none());
    }
    for node in [
        "Catalog.Items.Attribute.Missing",
        "Catalog.Items.TabularSection.Missing.Attribute.Amount",
        "Catalog.Items.Form.Main.Item.Missing",
        "Catalog.Items.Form.Main.Item.Missing.Item.Value",
        "Catalog.Items.Form.Main.Attribute.Missing",
        "CommonForm.Main.Item.Missing",
        "Catalog.Missing.Attribute.Value",
    ] {
        let result = invoke_exact_read(
            workspace.path(),
            ToolIdentity::Resolve,
            json!({"at":format!("main:{node}")}),
        );
        assert!(!result.ok, "{result:?}");
        assert_eq!(
            serde_json::to_value(&result).unwrap()["diagnostics"][0]["code"],
            "not_found"
        );
    }
}

/// rule: INV.WIRE.RESOLVED-SOURCE-LOCATION
#[test]
fn resolve_exact_method_spans_annotations_through_its_closing_line() {
    use unica_execution::invocation_store::ToolIdentity;
    let workspace = resolve_exact_workspace();
    // Документация, отделённая пустой строкой, методу не принадлежит;
    // прилегающая входит в диапазон `Body` — тот же, что текст `view`.
    let module = concat!(
            "// Документация.\n\n&НаСервере\n&Перед(\"Ping\")\nПроцедура Annotated(\n",
            "    Value) Экспорт\n    Сообщить(Value);\nКонецПроцедуры\n\n",
            "Процедура Empty()\nКонецПроцедуры\nФункция Inline() Возврат 1; КонецФункции\n",
            "Асинх\nПроцедура Waiting()\nКонецПроцедуры\n",
            "Async\nProcedure Waiting2()\nEndProcedure\n",
            "// Doc\n// Параметры:\n&НаСервере\nПроцедура Documented()\n    Сообщить(1);\nКонецПроцедуры\n",
        );
    for (prefix, path) in [
        ("CommonModule.Probe", "CommonModules/Probe/Ext/Module.bsl"),
        (
            "Catalog.Items.Form.Main.Module.Form",
            "Catalogs/Items/Forms/Main/Ext/Form/Module.bsl",
        ),
    ] {
        std::fs::write(workspace.path().join("src").join(path), module).unwrap();
        for (suffix, kind, lines) in [
            (
                ".Method.Annotated",
                "Method",
                json!({"state":"range","from":3,"to":8}),
            ),
            (
                ".Method.Annotated.Body",
                "Body",
                json!({"state":"range","from":3,"to":8}),
            ),
            (
                ".Method.Empty",
                "Method",
                json!({"state":"range","from":10,"to":11}),
            ),
            (
                ".Method.Empty.Body",
                "Body",
                json!({"state":"range","from":10,"to":11}),
            ),
            (
                ".Method.Inline",
                "Method",
                json!({"state":"range","from":12,"to":12}),
            ),
            (
                ".Method.Waiting",
                "Method",
                json!({"state":"range","from":13,"to":15}),
            ),
            (
                ".Method.Waiting2",
                "Method",
                json!({"state":"range","from":16,"to":18}),
            ),
            (
                ".Method.Documented",
                "Method",
                json!({"state":"range","from":21,"to":24}),
            ),
            (
                ".Method.Documented.Body",
                "Body",
                json!({"state":"range","from":19,"to":24}),
            ),
            (".Body", "Body", json!({"state":"range","from":1,"to":24})),
        ] {
            let at = format!("main:{prefix}{suffix}");
            let result =
                invoke_exact_read(workspace.path(), ToolIdentity::Resolve, json!({"at":at}));
            assert!(result.ok, "{at}: {result:?}");
            let data = result.data.as_ref().unwrap();
            assert_eq!(data["kind"], kind, "{at}: {result:?}");
            assert_eq!(data["path"], path, "{at}: {result:?}");
            assert_eq!(data["lines"], lines, "{at}: {result:?}");
        }
        // View names the same lines as the method text and its resolve.
        let view = invoke_exact_read(
            workspace.path(),
            ToolIdentity::View,
            json!({"at":format!("main:{prefix}.Method.Annotated")}),
        );
        assert!(view.ok, "{view:?}");
        let props = &view.data.as_ref().unwrap()["props"];
        assert_eq!((&props["line"], &props["endLine"]), (&json!(3), &json!(8)));
    }
}

/// rule: INV.SOURCE.MODULE-COMPILATION-AND-BODY
#[test]
fn body_line_view_and_resolve_share_absolute_byte_exact_ranges() {
    use unica_execution::invocation_store::ToolIdentity;
    let workspace = resolve_exact_workspace();
    let source = "\u{feff}\u{feff}#Область A\r\n\t// comment\r\nПроцедура Ping() Экспорт\r\n\tСообщить(\"needle\");\r\nКонецПроцедуры\r\n#КонецОбласти";
    for (prefix, path) in [
        ("CommonModule.Probe", "CommonModules/Probe/Ext/Module.bsl"),
        (
            "Catalog.Items.Module.Object",
            "Catalogs/Items/Ext/ObjectModule.bsl",
        ),
        (
            "Catalog.Items.Form.Main.Module.Form",
            "Catalogs/Items/Forms/Main/Ext/Form/Module.bsl",
        ),
    ] {
        std::fs::write(workspace.path().join("src").join(path), source).unwrap();
        for (suffix, expected, from, to) in [
            (".Body#L1", "#Область A\r\n", 1, 1),
            (
                ".Body#L2-L3",
                "\t// comment\r\nПроцедура Ping() Экспорт\r\n",
                2,
                3,
            ),
            (
                ".Method.Ping.Body#L4-L5",
                "\tСообщить(\"needle\");\r\nКонецПроцедуры\r\n",
                4,
                5,
            ),
            (".Body#L6", "#КонецОбласти", 6, 6),
        ] {
            let at = format!("main:{prefix}{suffix}");
            let viewed = invoke_exact_read(workspace.path(), ToolIdentity::View, json!({"at":at}));
            assert!(viewed.ok, "{at}: {viewed:?}");
            assert_eq!(viewed.source_text.as_deref(), Some(expected), "{at}");
            let resolved =
                invoke_exact_read(workspace.path(), ToolIdentity::Resolve, json!({"at":at}));
            assert!(resolved.ok, "{at}: {resolved:?}");
            let data = resolved.data.unwrap();
            assert_eq!(data["at"], at);
            assert_eq!(data["path"], path);
            assert_eq!(data["lines"], json!({"state":"range","from":from,"to":to}));
        }
    }
}

/// rule: INV.SOURCE.MODULE-COMPILATION-AND-BODY
#[test]
fn body_line_ranges_refuse_file_eof_and_method_boundary_crossings() {
    use unica_execution::invocation_store::ToolIdentity;
    let workspace = resolve_exact_workspace();
    let file = workspace
        .path()
        .join("src/CommonModules/Probe/Ext/Module.bsl");
    std::fs::write(&file, "// outside\n\nПроцедура Ping()\nКонецПроцедуры\n").unwrap();
    for suffix in [".Body#L5", ".Body#L3-L5", ".Method.Ping.Body#L1-L3"] {
        for tool in [ToolIdentity::View, ToolIdentity::Resolve] {
            let at = format!("main:CommonModule.Probe{suffix}");
            let result = invoke_exact_read(workspace.path(), tool, json!({"at":at}));
            assert!(!result.ok, "{at}: {result:?}");
            assert_eq!(
                result.diagnostics[0]["code"], "bad_value",
                "{at}: {result:?}"
            );
            assert!(result.source_text.is_none());
        }
    }
}

/// rule: INV.SOURCE.MODULE-COMPILATION-AND-BODY
#[test]
fn body_line_apply_refuses_before_staging_and_can_has_no_writes() {
    use unica_execution::invocation_store::ToolIdentity;
    let workspace = resolve_exact_workspace();
    let file = workspace
        .path()
        .join("src/CommonModules/Probe/Ext/Module.bsl");
    let source = "// outside\n\nПроцедура Ping()\n\tСообщить(\"needle\");\nКонецПроцедуры\n";
    std::fs::write(&file, source).unwrap();
    for (suffix, target) in [
        (".Body#L4", "main:CommonModule.Probe.Method.Ping"),
        (
            ".Method.Ping.Body#L4-L5",
            "main:CommonModule.Probe.Method.Ping",
        ),
        (".Body#L1-L4", "main:CommonModule.Probe.Body"),
    ] {
        let at = format!("main:CommonModule.Probe{suffix}");
        let view = invoke_exact_read(workspace.path(), ToolIdentity::View, json!({"at":at}));
        assert!(view.ok, "{view:?}");
        assert_eq!(view.data.as_ref().unwrap()["can"], json!([]));
        for op in ["code.replace", "code.insert"] {
            let refused = invoke_exact_read(
                workspace.path(),
                ToolIdentity::Apply,
                json!({"at":at,"ops":[{"op":op,"args":{"text":"Возврат;"}}]}),
            );
            assert!(!refused.ok, "{refused:?}");
            assert_eq!(refused.diagnostics[0]["code"], "bad_value");
            assert_eq!(refused.next[0]["args"]["at"], target);
            assert!(refused
                .data
                .as_ref()
                .is_none_or(|data| data.get("executionToken").is_none()));
            assert_eq!(std::fs::read_to_string(&file).unwrap(), source);
        }
    }
}

/// rule: INV.SOURCE.MODULE-COMPILATION-AND-BODY
/// rule: INV.WIRE.SOURCE-TEXT-RESPONSE-BUDGET
#[test]
fn body_line_read_validates_the_tail_and_continuation_reopens_exact_bytes() {
    use unica_execution::invocation_store::ToolIdentity;
    let workspace = resolve_exact_workspace();
    let file = workspace
        .path()
        .join("src/CommonModules/Probe/Ext/Module.bsl");
    let module = format!("{}end", "// юникод\r\n".repeat(12_000));
    let method = format!(
        "// outside\r\n\r\nПроцедура Ping()\r\n{}КонецПроцедуры",
        "\tСообщить(\"юникод\");\r\n".repeat(6_000)
    );
    for (source, body, first) in [
        (&module, "main:CommonModule.Probe.Body", 1),
        (&method, "main:CommonModule.Probe.Method.Ping.Body", 3),
        (
            &method,
            "main:CommonModule.Probe.Method.Ping.Body#L100-L5000",
            100,
        ),
    ] {
        std::fs::write(&file, source).unwrap();
        let whole = invoke_exact_read(workspace.path(), ToolIdentity::View, json!({"at":body}));
        assert!(!whole.ok, "{whole:?}");
        let budgeted = unica_execution_contract::source_text_budget::apply_response_budget(
            &whole,
            16_384,
            unica_bootstrap::MAX_RESPONSE_BUDGET,
        )
        .into_owned();
        let at = budgeted.next[0]["args"]["at"].as_str().unwrap();
        let address = unica_address::address::QualifiedAddress::parse(at).unwrap();
        let range = address.line_range().unwrap();
        assert_eq!(range.from, first);
        let part = invoke_exact_read(workspace.path(), ToolIdentity::View, json!({"at":at}));
        assert!(part.ok, "{part:?}");
        assert!(
            unica_execution_contract::source_text_budget::apply_response_budget(
                &part,
                16_384,
                unica_bootstrap::MAX_RESPONSE_BUDGET
            )
            .ok
        );
        let expected = source
            .split_inclusive('\n')
            .skip(range.from - 1)
            .take(range.to - range.from + 1)
            .collect::<String>();
        assert_eq!(part.source_text.as_deref(), Some(expected.as_str()));
        let next = format!(
            "{}#L{}-L{}",
            address.without_line_range(),
            range.from,
            range.to + 1
        );
        let one_more = invoke_exact_read(workspace.path(), ToolIdentity::View, json!({"at":next}));
        assert!(one_more.ok, "{one_more:?}");
        assert!(
            !unica_execution_contract::source_text_budget::apply_response_budget(
                &one_more,
                16_384,
                unica_bootstrap::MAX_RESPONSE_BUDGET
            )
            .ok,
            "one extra complete line must exceed this host budget: {at}"
        );
    }
    let mut broken = module.into_bytes();
    broken.push(0xff);
    std::fs::write(&file, broken).unwrap();
    let refused = invoke_exact_read(
        workspace.path(),
        ToolIdentity::View,
        json!({"at":"main:CommonModule.Probe.Body#L1"}),
    );
    assert!(!refused.ok);
    assert_ne!(refused.diagnostics[0]["code"], "result_too_large");
    assert!(refused.source_text.is_none());
}

/// rule: INV.APP.CODE-SEARCH-HIT-ADDRESS
#[test]
fn body_line_provider_search_uses_current_spans_and_preserves_evidence() {
    use unica_address::source_target::{
        MetadataAddress, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
    };
    use unica_code_intelligence::model::ProviderSearchHit;
    use unica_execution::invocation_store::ToolIdentity;
    use unica_source_contract::location::SourceLocation;
    let workspace = resolve_exact_workspace();
    std::fs::write(workspace.path().join("src/CommonModules/Probe/Ext/Module.bsl"), "// outside Needle\n\n// Needle doc\nПроцедура Ping()\n\tСообщить(\"Needle\");\nКонецПроцедуры\n").unwrap();
    for role in [ProviderRole::Symbol, ProviderRole::Semantic] {
        let hits = [
            (1, "// outside Needle"),
            (3, "// Needle doc"),
            (5, "Сообщить(\"Needle\");"),
            (5, "stale Needle"),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (line, snippet))| ProviderSearchHit {
            rank: Some(index + 1),
            provider_score: Some(0.5),
            location: SourceLocation::Addressed {
                source_set: "main".into(),
                metadata_path: Some(
                    MetadataAddress::parse(
                        PLATFORM_XML_8_3_27_FORMAT_2_20,
                        "CommonModule.Probe.Module",
                    )
                    .unwrap(),
                ),
                target_kind: TargetKind::Module,
            },
            line,
            end_line: None,
            symbol: Some("InventedMethodName".into()),
            kind: Some("procedure".into()),
            snippet: snippet.into(),
            attributes: serde_json::Map::from_iter([("column".into(), json!(2))]),
        })
        .collect();
        let section = ProviderSearchSection::complete(
            ProviderIdentity::new(role, "fixture"),
            SearchRanking::Provider,
            SearchOrdering::Provider,
            hits,
            vec![],
        )
        .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let service = Arc::new(
            super::CanonicalV13ReadService::with_search_providers_for_test(vec![Arc::new(
                SectionSearchProvider {
                    section,
                    calls: calls.clone(),
                },
            )]),
        );
        let result = invoke_exact_read_with_service(
            workspace.path(),
            ToolIdentity::Search,
            json!({"query":"Needle","role":role.as_str(),"scope":"main:CommonModule.Probe"}),
            service,
        );
        assert!(result.ok, "{role:?}: {result:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let data = result.data.unwrap();
        let hits = data["matches"][0]["hits"].as_array().unwrap();
        for (index, at) in [
            "main:CommonModule.Probe.Body#L1",
            "main:CommonModule.Probe.Method.Ping.Body#L3",
            "main:CommonModule.Probe.Method.Ping.Body#L5",
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(hits[index]["at"], at, "{role:?}: {data:#}");
            assert_eq!(hits[index]["symbol"], "InventedMethodName");
            assert_eq!(hits[index]["attributes"]["column"], 2);
            let view = invoke_exact_read(workspace.path(), ToolIdentity::View, json!({"at":at}));
            assert!(view.ok, "{view:?}");
            assert!(view
                .source_text
                .unwrap()
                .contains(hits[index]["snippet"].as_str().unwrap()));
        }
        assert!(hits[3].get("at").is_none());
        assert!(hits[3]["limits"].is_array());
        assert_eq!(hits[3]["snippet"], "stale Needle");
    }
}

/// rule: INV.APP.CODE-SEARCH-HIT-ADDRESS
#[test]
fn a_text_search_hit_opens_the_narrowest_body_line_address() {
    use unica_execution::invocation_store::ToolIdentity;
    let workspace = resolve_exact_workspace();
    std::fs::write(
        workspace
            .path()
            .join("src/CommonModules/Probe/Ext/Module.bsl"),
        "// outside\n\nПроцедура Ping()\n\tСообщить(\"needle\");\nКонецПроцедуры\n",
    )
    .unwrap();
    let result = invoke_exact_read(
        workspace.path(),
        ToolIdentity::Search,
        json!({
            "query":"needle", "scope":"main:CommonModule.Probe"
        }),
    );
    assert!(result.ok, "{result:?}");
    let data = result.data.unwrap();
    let hits = data["matches"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "{data:?}");
    let hit = &hits[0];
    assert_eq!(hit["at"], "main:CommonModule.Probe.Method.Ping.Body#L4");
    assert_eq!(hit["line"], 4);
    let opened = invoke_exact_read(
        workspace.path(),
        ToolIdentity::View,
        json!({"at":hit["at"]}),
    );
    assert!(opened.ok, "{opened:?}");
    assert!(opened
        .source_text
        .as_deref()
        .unwrap()
        .contains(hit["snippet"].as_str().unwrap()));
}

/// Literal search under a module scope opens no publication artifact:
/// an unreadable staged copy or private directory left beside the module
/// right after apply does not refuse the search. An unreadable entry the
/// search does need is named in the refusal.
/// rule: INV.APP.LITERAL-SEARCH
#[test]
fn literal_search_skips_publication_artifacts_and_names_an_unreadable_entry() {
    use unica_execution::invocation_store::ToolIdentity;
    use unica_workspace::test_support::set_unix_mode_for_test;

    /// Restores the mode even when an assertion fails, so the
    /// temporary workspace can still be removed.
    struct Unlocked(Vec<std::path::PathBuf>);
    impl Drop for Unlocked {
        fn drop(&mut self) {
            for path in &self.0 {
                let _ = set_unix_mode_for_test(path, 0o755);
            }
        }
    }

    let workspace = resolve_exact_workspace();
    let ext = workspace.path().join("src/CommonModules/Probe/Ext");
    std::fs::write(
        ext.join("Module.bsl"),
        "Процедура Ping()\n\tСообщить(\"needle\");\nКонецПроцедуры\n",
    )
    .unwrap();
    let artifact = ext.join(".unica-apply-left-behind");
    std::fs::write(&artifact, b"preimage").unwrap();
    let staged = ext.join(".unica-apply-dir-left-behind");
    std::fs::create_dir(&staged).unwrap();
    std::fs::write(staged.join("Module.bsl"), "stale needle\n").unwrap();
    let locked = ext.join("Locked");
    std::fs::create_dir(&locked).unwrap();
    let mut unlocked = Unlocked(Vec::new());
    for path in [&artifact, &staged] {
        if !set_unix_mode_for_test(path, 0o000).unwrap() {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::UnixPermissionBits,
                "the test sets Unix permission bits",
            );
            return;
        }
        unlocked.0.push(path.clone());
    }
    if std::fs::read(&artifact).is_ok() {
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::EnforcedPermissionBits,
            "the test process ignores permission bits",
        );
        return;
    }
    let search = |engine| {
        invoke_exact_read_with_service(
            workspace.path(),
            ToolIdentity::Search,
            json!({"query":"needle", "scope":"main:CommonModule.Probe"}),
            Arc::new(super::CanonicalV13ReadService::with_text_engine_for_test(
                engine,
            )),
        )
    };
    for engine in [TextEngine::Walk, TextEngine::Git] {
        let found = search(engine);
        assert!(found.ok, "{engine:?}: {found:?}");
        let matches = found.data.as_ref().unwrap()["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1, "{engine:?}: {found:?}");
        assert_eq!(
            matches[0]["at"],
            "main:CommonModule.Probe.Method.Ping.Body#L2"
        );
    }

    set_unix_mode_for_test(&locked, 0o000).unwrap();
    unlocked.0.push(locked);
    // Neither engine answers a corpus it could not read in full.
    for (engine, named) in [
        (
            TextEngine::Walk,
            "could not open source entry `CommonModules/Probe/Ext/Locked`",
        ),
        (TextEngine::Git, "Locked"),
    ] {
        let refused = search(engine);
        assert!(!refused.ok, "{engine:?}: {refused:?}");
        assert_eq!(refused.diagnostics[0]["code"], "provider_unavailable");
        assert!(
            refused.diagnostics[0]["message"]
                .as_str()
                .is_some_and(|message| message.contains(named)),
            "{engine:?}: {refused:?}"
        );
    }
}

/// rule: INV.WIRE.RESOLVED-SOURCE-LOCATION
#[test]
fn resolve_exact_module_body_numbers_lines_like_view() {
    use unica_execution::invocation_store::ToolIdentity;
    let workspace = resolve_exact_workspace();
    let file = workspace
        .path()
        .join("src/CommonModules/Probe/Ext/Module.bsl");
    let at = json!({"at":"main:CommonModule.Probe.Body"});
    for (source, expected) in [
        ("", json!({"state":"notLineBased"})),
        ("\u{feff}\u{feff}", json!({"state":"notLineBased"})),
        ("\n", json!({"state":"range","from":1,"to":1})),
        ("a", json!({"state":"range","from":1,"to":1})),
        ("\u{feff}a\r\nb", json!({"state":"range","from":1,"to":2})),
        ("a\r\n\r\nb\r\n", json!({"state":"range","from":1,"to":3})),
    ] {
        std::fs::write(&file, source).unwrap();
        let view = invoke_exact_read(workspace.path(), ToolIdentity::View, at.clone());
        assert!(view.ok, "{source:?}: {view:?}");
        // Строки текста `Body` агент считает сам — так же, как `str::lines`.
        let lines = view
            .source_text
            .as_deref()
            .unwrap_or_default()
            .lines()
            .count();
        let viewed = if lines == 0 {
            json!({"state":"notLineBased"})
        } else {
            json!({"state":"range","from":1,"to":lines})
        };
        assert_eq!(viewed, expected, "view numbering of {source:?}");
        let resolved = invoke_exact_read(workspace.path(), ToolIdentity::Resolve, at.clone());
        assert!(resolved.ok, "{source:?}: {resolved:?}");
        assert_eq!(
            resolved.data.unwrap()["lines"],
            expected,
            "resolve numbering of {source:?}"
        );
    }
}

/// rule: INV.WIRE.RESOLVED-SOURCE-LOCATION
#[test]
fn resolve_exact_module_read_failures_are_never_successful_locations() {
    use unica_execution::invocation_store::ToolIdentity;
    let workspace = resolve_exact_workspace();
    for (prefix, path) in [
        ("CommonModule.Probe", "CommonModules/Probe/Ext/Module.bsl"),
        (
            "Catalog.Items.Module.Object",
            "Catalogs/Items/Ext/ObjectModule.bsl",
        ),
        (
            "Catalog.Items.Form.Main.Module.Form",
            "Catalogs/Items/Forms/Main/Ext/Form/Module.bsl",
        ),
    ] {
        let file = workspace.path().join("src").join(path);
        std::fs::write(&file, "").unwrap();
        let empty = invoke_exact_read(
            workspace.path(),
            ToolIdentity::Resolve,
            json!({"at":format!("main:{prefix}.Body")}),
        );
        assert!(empty.ok, "{empty:?}");
        assert_eq!(
            empty.data.unwrap()["lines"],
            json!({"state":"notLineBased"})
        );
        std::fs::remove_file(&file).unwrap();
        let absent = invoke_exact_read(
            workspace.path(),
            ToolIdentity::Resolve,
            json!({"at":format!("main:{prefix}.Body")}),
        );
        assert!(
            !absent.ok,
            "an absent BSL file is not an empty body: {absent:?}"
        );
        assert_eq!(
            serde_json::to_value(&absent).unwrap()["diagnostics"][0]["code"],
            "not_found"
        );
        std::fs::write(&file, [0xff]).unwrap();
        let unreadable = invoke_exact_read(
            workspace.path(),
            ToolIdentity::Resolve,
            json!({"at":format!("main:{prefix}.Body")}),
        );
        assert!(!unreadable.ok, "{unreadable:?}");
        assert_eq!(
            serde_json::to_value(&unreadable).unwrap()["diagnostics"][0]["code"],
            "provider_unavailable"
        );
    }
}

/// rule: INV.WIRE.RESOLVED-SOURCE-LOCATION
#[test]
fn resolve_exact_cancelled_read_never_claims_a_location() {
    use crate::infrastructure::daemon::protocol::InvocationRequest;
    use crate::infrastructure::daemon::server::V5CanonicalInvocationRuntime;
    use unica_execution::clock::TokioClock;
    use unica_execution::invocation_store::ToolIdentity;
    use unica_execution::receipt_runtime::ports::ExecutionClass;
    use unica_execution_contract::invocation::{DomainResult, InvocationFailure};
    struct CancelRead(super::CanonicalV13ReadService);
    impl super::CanonicalInvocationService for CancelRead {
        fn prepare(
            &self,
            invocation: &super::ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            self.0.prepare(invocation)
        }
        fn execute(
            &self,
            execution: &super::ActorBoundExecution,
            _: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            // Cancel after actor admission, at the actual resolve read boundary.
            let cancellation = CancellationToken::new();
            cancellation.cancel();
            self.0.execute(execution, cancellation)
        }
    }
    let workspace = resolve_exact_workspace();
    let runtime = V5CanonicalInvocationRuntime::new(
        Arc::new(CancelRead(super::CanonicalV13ReadService::default())),
        Arc::new(TokioClock),
    );
    let result = runtime
        .bind(
            InvocationRequest::new(
                ToolIdentity::Resolve,
                json!({"at":"main:CommonModule.Probe.Method.Ping"}),
                std::fs::canonicalize(workspace.path())
                    .unwrap()
                    .to_string_lossy(),
                7_000,
            )
            .unwrap(),
        )
        .unwrap()
        .prepare()
        .unwrap()
        .execute(CancellationToken::new())
        .unwrap();
    assert!(
        !result.ok,
        "cancelled resolution claimed a location: {result:?}"
    );
    assert_eq!(
        serde_json::to_value(&result).unwrap()["diagnostics"][0]["code"],
        "cancelled"
    );
    assert!(
        result.data.is_none(),
        "cancelled read returned a location: {result:?}"
    );
}

/// rule: INV.WIRE.RESOLVE-PATH-BATCH
#[test]
fn resolve_paths_cancelled_read_refuses_the_whole_call() {
    use crate::infrastructure::daemon::protocol::InvocationRequest;
    use crate::infrastructure::daemon::server::V5CanonicalInvocationRuntime;
    use unica_execution::clock::TokioClock;
    use unica_execution::invocation_store::ToolIdentity;
    use unica_execution::receipt_runtime::ports::ExecutionClass;
    use unica_execution_contract::invocation::{DomainResult, InvocationFailure};
    struct CancelRead(super::CanonicalV13ReadService);
    impl super::CanonicalInvocationService for CancelRead {
        fn prepare(
            &self,
            invocation: &super::ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            self.0.prepare(invocation)
        }
        fn execute(
            &self,
            execution: &super::ActorBoundExecution,
            _: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            let cancellation = CancellationToken::new();
            cancellation.cancel();
            self.0.execute(execution, cancellation)
        }
    }
    let workspace = resolve_exact_workspace();
    let runtime = V5CanonicalInvocationRuntime::new(
        Arc::new(CancelRead(super::CanonicalV13ReadService::default())),
        Arc::new(TokioClock),
    );
    let result = runtime
        .bind(
            InvocationRequest::new(
                ToolIdentity::Resolve,
                json!({"paths": [
                    "src/CommonModules/Probe/Ext/Module.bsl:4",
                    "src/Catalogs/Items.xml",
                ]}),
                std::fs::canonicalize(workspace.path())
                    .unwrap()
                    .to_string_lossy(),
                7_000,
            )
            .unwrap(),
        )
        .unwrap()
        .prepare()
        .unwrap()
        .execute(CancellationToken::new())
        .unwrap();
    // Отмена не превращается в отказы элементов внутри успешного пакета.
    assert!(!result.ok, "cancelled batch claimed answers: {result:?}");
    assert_eq!(
        serde_json::to_value(&result).unwrap()["diagnostics"][0]["code"],
        "cancelled"
    );
    assert!(result.data.is_none(), "{result:?}");
}

/// rule: INV.APP.LITERAL-SEARCH
#[test]
fn literal_module_scope_searches_only_the_selected_module_with_either_engine() {
    use crate::infrastructure::daemon::protocol::InvocationRequest;
    use crate::infrastructure::daemon::server::V5CanonicalInvocationRuntime;
    use unica_execution::clock::TokioClock;
    use unica_execution::invocation_store::ToolIdentity;

    let workspace = role_search_workspace();
    let source = workspace.path().join("src");
    let descriptor = |kind: &str, name: &str, children: &str| {
        format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{kind}><Properties><Name>{name}</Name></Properties><ChildObjects>{children}</ChildObjects></{kind}></MetaDataObject>"#
        )
    };
    std::fs::write(source.join("Configuration.xml"), descriptor("Configuration", "Store", "<Catalog>Items</Catalog><Catalog>Neighbor</Catalog><CommonModule>Shared</CommonModule><CommonModule>Empty</CommonModule>")).unwrap();
    for (path, kind, name, children) in [
        (
            "Catalogs/Items.xml",
            "Catalog",
            "Items",
            "<Form>Main</Form><Command>Act</Command>",
        ),
        ("Catalogs/Neighbor.xml", "Catalog", "Neighbor", ""),
        ("Catalogs/Items/Forms/Main.xml", "Form", "Main", ""),
        ("Catalogs/Items/Commands/Act.xml", "Command", "Act", ""),
        ("CommonModules/Shared.xml", "CommonModule", "Shared", ""),
        ("CommonModules/Empty.xml", "CommonModule", "Empty", ""),
    ] {
        let path = source.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, descriptor(kind, name, children)).unwrap();
    }
    let modules = [
        (
            "main:Catalog.Items.Module.Object",
            "Catalogs/Items/Ext/ObjectModule.bsl",
            "main:Catalog.Items.Module.Object.Body#L1",
        ),
        (
            "main:Catalog.Items.Module.Manager",
            "Catalogs/Items/Ext/ManagerModule.bsl",
            "main:Catalog.Items.Module.Manager.Body#L1",
        ),
        (
            "main:Catalog.Items.Form.Main.Module.Form",
            "Catalogs/Items/Forms/Main/Ext/Form/Module.bsl",
            "main:Catalog.Items.Form.Main.Module.Form.Body#L1",
        ),
        (
            "main:Catalog.Items.Command.Act.Module.Command",
            "Catalogs/Items/Commands/Act/Ext/CommandModule.bsl",
            "main:Catalog.Items.Command.Act.Module.Command.Body#L1",
        ),
        (
            "main:CommonModule.Shared",
            "CommonModules/Shared/Ext/Module.bsl",
            "main:CommonModule.Shared.Body#L1",
        ),
        (
            "main:Module.Session",
            "Ext/SessionModule.bsl",
            "main:Module.Session.Body#L1",
        ),
        (
            "main:Catalog.Neighbor.Module.Object",
            "Catalogs/Neighbor/Ext/ObjectModule.bsl",
            "main:Catalog.Neighbor.Module.Object.Body#L1",
        ),
    ];
    for (_, path, _) in modules {
        let path = source.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "// Needle\n").unwrap();
    }
    assert!(std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(workspace.path())
        .status()
        .unwrap()
        .success());
    for engine in [TextEngine::Walk, TextEngine::Git] {
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(super::CanonicalV13ReadService::with_text_engine_for_test(
                engine,
            )),
            Arc::new(TokioClock),
        );
        let search = |scope: &str| {
            let request = InvocationRequest::new(
                ToolIdentity::Search,
                json!({"query":"Needle", "scope":scope, "limit":50}),
                std::fs::canonicalize(workspace.path())
                    .unwrap()
                    .to_string_lossy(),
                7_000,
            )
            .unwrap();
            runtime
                .bind(request)
                .unwrap()
                .prepare()
                .unwrap()
                .execute(CancellationToken::new())
                .unwrap()
        };
        let hits = |result: &unica_execution_contract::invocation::DomainResult| {
            result.data.as_ref().unwrap()["matches"]
                .as_array()
                .unwrap()
                .iter()
                .map(|hit| hit["at"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        for (scope, _, at) in modules {
            let result = search(scope);
            assert!(result.ok, "{engine:?} {scope}: {result:?}");
            assert!(
                result.diagnostics.is_empty(),
                "{engine:?} {scope}: {result:?}"
            );
            assert_eq!(
                hits(&result),
                vec![at],
                "{engine:?} {scope} must not include a neighbor"
            );
        }
        for (scope, count) in [("main:Catalog.Items", 4), ("main:Configuration", 7)] {
            let result = search(scope);
            assert!(result.ok, "{engine:?} {scope}: {result:?}");
            assert_eq!(hits(&result).len(), count, "{engine:?} {scope}: {result:?}");
        }
        let result = search("main:CommonModule.Empty");
        assert!(
            result.ok && hits(&result).is_empty(),
            "{engine:?}: {result:?}"
        );
        let result = search("main:Catalog.Unknown.Module.Object");
        assert!(
            !result.ok,
            "{engine:?}: an unknown owner must not become a source-wide search: {result:?}"
        );
    }
    // A registered module without its file is an empty answer.
    std::fs::remove_file(source.join("Catalogs/Items/Ext/ManagerModule.bsl")).unwrap();
    for engine in [TextEngine::Walk, TextEngine::Git] {
        let result = invoke_exact_read_with_service(
            workspace.path(),
            ToolIdentity::Search,
            json!({"query":"Needle", "scope":"main:Catalog.Items.Module.Manager"}),
            Arc::new(super::CanonicalV13ReadService::with_text_engine_for_test(
                engine,
            )),
        );
        assert!(result.ok, "{engine:?}: {result:?}");
        assert!(
            result.data.as_ref().unwrap()["matches"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{engine:?}: {result:?}"
        );
    }
}

/// rule: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
#[test]
fn canonical_role_search_preserves_typed_failure_without_private_provider_text() {
    use crate::infrastructure::daemon::protocol::InvocationRequest;
    use crate::infrastructure::daemon::server::V5CanonicalInvocationRuntime;
    use unica_execution::clock::TokioClock;
    use unica_execution::invocation_store::ToolIdentity;

    let workspace = role_search_workspace();
    let private = "/private/provider/cache/index.db: engine stderr contains secret";
    let identity = ProviderIdentity::new(ProviderRole::Symbol, "replacement-symbol");
    let timed_out = ProviderSearchSection::timed_out(
        identity.clone(),
        SearchRanking::Provider,
        SearchOrdering::Provider,
        vec![],
        vec![],
    )
    .unwrap();
    let pending = |detail| {
        ProviderSearchSection::dependency_pending(
            identity.clone(),
            SearchRanking::Provider,
            SearchOrdering::Provider,
            vec![],
            vec![private.into()],
            detail,
        )
        .unwrap()
    };
    for (mut section, code, outcome, detail) in [
        (timed_out, "deadline_exceeded", "retry", None),
        (
            pending("buildingIndex"),
            "provider_unavailable",
            "retry",
            Some("dependency_pending"),
        ),
        (
            pending("indexQueued"),
            "provider_unavailable",
            "retry",
            Some("dependency_pending"),
        ),
        (
            pending(private),
            "provider_unavailable",
            "retry",
            Some("dependency_pending"),
        ),
        (
            ProviderSearchSection::failed(identity.clone(), private.into()),
            "provider_failed",
            "deadEnd",
            None,
        ),
        (
            ProviderSearchSection::unavailable(identity.clone(), private.into()),
            "provider_unavailable",
            "needsHuman",
            None,
        ),
        (
            ProviderSearchSection::unsupported_scope(identity.clone(), private.into()),
            "unsupported_scope",
            "fixCall",
            None,
        ),
        (
            ProviderSearchSection::capacity_exhausted(identity.clone(), private.into()),
            "task_backend_failed",
            "retry",
            Some("backend_busy"),
        ),
    ] {
        section.index_freshness = Some("fresh".into());
        section.index_maintenance =
            Some(unica_code_intelligence::model::IndexMaintenanceEvidence::StartFailed);
        section.index_build_id = Some("known-build".into());
        section.artifacts.push(private.into());
        let expected = serde_json::to_value(&section).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let neighbor_calls = Arc::new(AtomicUsize::new(0));
        let service = super::CanonicalV13ReadService {
            search_providers: Some(vec![
                Arc::new(SectionSearchProvider {
                    section,
                    calls: calls.clone(),
                }),
                Arc::new(SectionSearchProvider {
                    section: ProviderSearchSection::complete(
                        ProviderIdentity::new(ProviderRole::Semantic, "neighbor"),
                        SearchRanking::Provider,
                        SearchOrdering::Provider,
                        vec![],
                        vec![],
                    )
                    .unwrap(),
                    calls: neighbor_calls.clone(),
                }),
            ]),
            ..Default::default()
        };
        let runtime = V5CanonicalInvocationRuntime::new(Arc::new(service), Arc::new(TokioClock));
        let request = InvocationRequest::new(
            ToolIdentity::Search,
            json!({"query":"Needle", "corpus":"text", "role":"symbol", "limit":10}),
            std::fs::canonicalize(workspace.path())
                .unwrap()
                .to_string_lossy(),
            7_000,
        )
        .unwrap();
        let result = runtime
            .bind(request)
            .unwrap()
            .prepare()
            .unwrap()
            .execute(CancellationToken::new())
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(neighbor_calls.load(Ordering::SeqCst), 0);
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], code);
        assert_eq!(result.diagnostics[0]["outcome"], outcome);
        assert_eq!(
            result.diagnostics[0]
                .get("detailCode")
                .and_then(|v| v.as_str()),
            detail
        );
        let data = &result.data.as_ref().unwrap()["matches"][0];
        for field in [
            "role",
            "provider",
            "status",
            "searchComplete",
            "matches",
            "indexBuildId",
            "indexMaintenance",
        ] {
            assert_eq!(data[field], expected[field], "{field}");
        }
        assert_eq!(data["indexFreshness"], "unknown");
        assert_eq!(data["termination"]["code"], expected["termination"]["code"]);
        assert_eq!(
            data["termination"]["retryable"],
            expected["termination"]["retryable"]
        );
        let expected_detail = expected["termination"]["detailCode"]
            .as_str()
            .filter(|detail| matches!(*detail, "buildingIndex" | "updatingIndex" | "indexQueued"));
        assert_eq!(
            data["termination"]
                .get("detailCode")
                .and_then(|v| v.as_str()),
            expected_detail
        );
        assert!(data.get("diagnostics").is_none());
        assert!(data.get("artifacts").is_none());
        assert!(result.rev.is_none());
        assert!(!serde_json::to_string(&result).unwrap().contains(private));
    }
}

/// A provider whose index is still cold: a search waits until the test
/// opens the gate. A call that reached it before the gate opens would
/// hang the test instead of returning.
struct ColdIndexProvider {
    calls: Arc<AtomicUsize>,
    warm: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
}

impl CodeIntelligenceProvider for ColdIndexProvider {
    fn identity(&self) -> ProviderIdentity {
        ProviderIdentity::new(ProviderRole::Symbol, "cold-symbol")
    }

    fn capabilities(&self) -> &[ProviderCapability] {
        &[ProviderCapability::Search]
    }

    fn search(
        &self,
        _request: &SearchRequest,
        _context: &CodeIntelligenceContext,
        _deadline: ProviderDeadline,
        _cancellation: &CancellationToken,
    ) -> ProviderSearchSection {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let (warm, changed) = &*self.warm;
        let guard = warm.lock().unwrap();
        let _ = changed
            .wait_timeout_while(guard, Duration::from_secs(30), |warm| !*warm)
            .unwrap();
        ProviderSearchSection::dependency_pending(
            self.identity(),
            SearchRanking::Provider,
            SearchOrdering::Provider,
            vec![],
            vec![],
            "buildingIndex",
        )
        .unwrap()
    }
}

/// #1216: the arguments of `search` are decided before execution. A
/// cursor that is not a continuation of this question, and a limit out
/// of range, are refused by `prepare` — the step before execution and
/// before a call can become a task — without asking a provider whose
/// index is still cold. A continuation of the same question passes and
/// reaches the provider.
#[test]
fn search_arguments_are_refused_before_a_cold_provider_is_asked() {
    use crate::infrastructure::daemon::protocol::InvocationRequest;
    use crate::infrastructure::daemon::server::V5CanonicalInvocationRuntime;
    use unica_execution::clock::TokioClock;
    use unica_execution::invocation_store::ToolIdentity;
    use unica_navigation::result_store::SearchCursorStore;

    let workspace = role_search_workspace();
    let calls = Arc::new(AtomicUsize::new(0));
    let warm = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let service = Arc::new(
        super::CanonicalV13ReadService::with_search_providers_for_test(vec![Arc::new(
            ColdIndexProvider {
                calls: calls.clone(),
                warm: warm.clone(),
            },
        )]),
    );
    let runtime = V5CanonicalInvocationRuntime::new(service.clone(), Arc::new(TokioClock));
    let bind = |arguments: serde_json::Value| {
        let request = InvocationRequest::new(
            ToolIdentity::Search,
            arguments,
            std::fs::canonicalize(workspace.path())
                .unwrap()
                .to_string_lossy(),
            7_000,
        )
        .unwrap();
        runtime.bind(request).unwrap()
    };
    let refused = |arguments: serde_json::Value| match bind(arguments.clone()).prepare() {
        Ok(_) => panic!("{arguments} must be refused before execution"),
        Err(result) => *result,
    };

    let question = SearchCursorBinding {
        workspace_identity: bind(json!({"query": "Needle", "role": "symbol"}))
            .workspace_identity_hash()
            .as_str()
            .to_owned(),
        query: "Needle".into(),
        scope: None,
        mode: "symbol".into(),
        kind: None,
        source_sets: vec!["main".into()],
        result_fingerprint: None,
        page_limit: 5,
    };
    let issue = |store: &SearchCursorStore, change: &dyn Fn(&mut SearchCursorBinding)| {
        let mut binding = question.clone();
        change(&mut binding);
        store.insert_first(binding, 5).unwrap()
    };
    let foreign = [
        issue(&service.search_cursors, &|binding| binding.page_limit = 10),
        issue(&service.search_cursors, &|binding| {
            binding.query = "Другое".into()
        }),
        issue(&service.search_cursors, &|binding| {
            binding.mode = "semantic".into()
        }),
        issue(&service.search_cursors, &|binding| {
            binding.source_sets = vec!["ext".into()]
        }),
        issue(&SearchCursorStore::default(), &|_| {}),
        "не-наш-курсор".to_string(),
    ];
    for cursor in foreign {
        let result =
            refused(json!({"query": "Needle", "role": "symbol", "limit": 5, "cursor": cursor}));
        assert!(!result.ok);
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_cursor",
            "{result:?}"
        );
        assert_eq!(result.diagnostics[0]["outcome"], "fixCall", "{result:?}");
    }
    for arguments in [
        json!({"query": "Needle", "role": "symbol", "limit": 51}),
        json!({"query": "Needle", "limit": 51}),
        json!({"query": "Needle", "corpus": "names", "limit": 51}),
        json!({"query": "Needle", "role": "symbol", "scope": "main:Нет такого"}),
        json!({"query": "Needle", "role": "index"}),
        json!({"query": "  ", "role": "symbol"}),
    ] {
        let result = refused(arguments.clone());
        assert_eq!(
            result.diagnostics[0]["code"], "bad_value",
            "{arguments}: {result:?}"
        );
        assert_eq!(
            result.diagnostics[0]["outcome"], "fixCall",
            "{arguments}: {result:?}"
        );
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "a refused argument must not wait for the cold index"
    );

    let continuation = issue(&service.search_cursors, &|_| {});
    let prepared = match bind(
        json!({"query": "Needle", "role": "symbol", "limit": 5, "cursor": continuation}),
    )
    .prepare()
    {
        Ok(prepared) => prepared,
        Err(result) => panic!("a continuation of the same question is admitted: {result:?}"),
    };
    *warm.0.lock().unwrap() = true;
    warm.1.notify_all();
    let result = prepared.execute(CancellationToken::new()).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_ne!(
        result
            .diagnostics
            .first()
            .map(|diagnostic| &diagnostic["code"]),
        Some(&json!("invalid_cursor")),
        "{result:?}"
    );
}

#[test]
fn canonical_role_search_preserves_partial_hits_and_their_cursor() {
    use crate::infrastructure::daemon::protocol::InvocationRequest;
    use crate::infrastructure::daemon::server::V5CanonicalInvocationRuntime;
    use unica_execution::clock::TokioClock;
    use unica_execution::invocation_store::ToolIdentity;

    let workspace = role_search_workspace();
    let complete = provider_search_test_execution(2, 10, false)
        .result
        .sections
        .remove(0);
    let section = ProviderSearchSection::partial(
        ProviderIdentity::new(ProviderRole::Symbol, "replacement-symbol"),
        complete.ranking,
        complete.ordering,
        complete.hits,
        vec!["one result was malformed".into()],
    )
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let runtime = V5CanonicalInvocationRuntime::new(
        Arc::new(super::CanonicalV13ReadService {
            search_providers: Some(vec![Arc::new(SectionSearchProvider {
                section,
                calls: calls.clone(),
            })]),
            ..Default::default()
        }),
        Arc::new(TokioClock),
    );
    let mut arguments = json!({"query":"Needle", "corpus":"text", "role":"symbol", "limit":1});
    for line in [1, 2] {
        let request = InvocationRequest::new(
            ToolIdentity::Search,
            arguments.clone(),
            std::fs::canonicalize(workspace.path())
                .unwrap()
                .to_string_lossy(),
            7_000,
        )
        .unwrap();
        let result = runtime
            .bind(request)
            .unwrap()
            .prepare()
            .unwrap()
            .execute(CancellationToken::new())
            .unwrap();
        assert!(result.ok, "{result:?}");
        let section = &result.data.as_ref().unwrap()["matches"][0];
        assert_eq!(section["status"], "partial");
        assert_eq!(section["searchComplete"], false);
        assert_eq!(section["termination"]["code"], "providerFailed");
        assert_eq!(section["hits"][0]["line"], line);
        assert_eq!(section["hits"].as_array().unwrap().len(), 1);
        assert!(!result.warnings.is_empty());
        assert!(result.rev.is_none());
        if line == 1 {
            arguments["cursor"] = json!(result
                .cursor
                .expect("partial first page has a continuation"));
        } else {
            assert!(result.cursor.is_none());
        }
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

/// rule: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
#[test]
fn provider_search_failure_requires_one_selected_section_and_yields_to_cancellation() {
    let selected = ProviderSearchSection::failed(
        ProviderIdentity::new(ProviderRole::Symbol, "registered-symbol"),
        "/private/index.db".into(),
    );
    let neighbor = ProviderSearchSection::failed(
        ProviderIdentity::new(ProviderRole::Semantic, "neighbor"),
        "/private/neighbor.db".into(),
    );
    for (sections, valid) in [
        (vec![selected.clone()], true),
        (vec![], false),
        (vec![neighbor], false),
        (vec![selected.clone(), selected], false),
    ] {
        for cancel in [false, true] {
            let mut execution = provider_search_test_execution(0, 0, false);
            execution.ok = false;
            execution.result.sections = sections.clone();
            execution.warnings.push("/private/warning.db".into());
            execution.errors.push("/private/stderr.db".into());
            let cancellation = CancellationToken::new();
            if cancel {
                cancellation.cancel();
            }
            let result = super::provider_search_page(
                &Default::default(),
                ProviderRole::Symbol,
                execution,
                provider_search_test_binding(ProviderRole::Symbol, 10),
                Some("unused-cursor"),
                &cancellation,
            );
            assert!(!result.ok);
            assert_eq!(
                result.diagnostics[0]["code"],
                if cancel {
                    "cancelled"
                } else {
                    "provider_failed"
                }
            );
            if valid && !cancel {
                let section = &result.data.as_ref().unwrap()["matches"][0];
                assert_eq!(section["provider"], "registered-symbol");
                assert!(section.get("indexBuildId").is_none());
            } else {
                assert!(result.data.is_none());
            }
            assert!(result.page.is_none());
            assert!(result.warnings.is_empty());
            assert!(!serde_json::to_string(&result)
                .unwrap()
                .contains("/private/"));
        }
    }
}

struct CountingSearchProvider {
    identity: ProviderIdentity,
    calls: Arc<AtomicUsize>,
    fails: bool,
}

impl CodeIntelligenceProvider for CountingSearchProvider {
    fn identity(&self) -> ProviderIdentity {
        self.identity.clone()
    }

    fn capabilities(&self) -> &[ProviderCapability] {
        &[ProviderCapability::Search]
    }

    fn search(
        &self,
        request: &SearchRequest,
        context: &CodeIntelligenceContext,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> ProviderSearchSection {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.query, "Needle");
        assert_eq!(request.limit, 7);
        assert_eq!(context.search_scope.as_ref().unwrap().source_set, "main");
        assert!(deadline.remaining().unwrap() <= Duration::from_secs(1));
        assert!(!cancellation.is_cancelled());
        if self.fails {
            ProviderSearchSection::failed(self.identity.clone(), "selected failure".into())
        } else {
            ProviderSearchSection::complete(
                self.identity.clone(),
                SearchRanking::Provider,
                SearchOrdering::Provider,
                Vec::new(),
                Vec::new(),
            )
            .expect("complete section")
        }
    }
}

fn selected_role_test_context() -> CodeIntelligenceContext {
    CodeIntelligenceContext::new(
        WorkspaceContext {
            cwd: PathBuf::from("/workspace"),
            workspace_root: PathBuf::from("/workspace"),
            cache_root: PathBuf::from("/cache"),
            workspace_epoch: 1,
        },
        ResolvedSourceRoot {
            source_set: Some("main".into()),
            path: PathBuf::from("/workspace"),
        },
        std::sync::Arc::new(unica_workspace::discovery::ProjectDiscoveryReader::new(
            std::path::Path::new("/workspace"),
        )),
    )
    .with_search_scope(CodeSearchScope::all(
        "main".into(),
        PathBuf::from("/workspace"),
        false,
    ))
}

#[test]
fn explicit_search_roles_name_only_the_engine_they_execute() {
    let lock_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugins/unica/third-party/tools.lock.json");
    let lock: serde_json::Value =
        serde_json::from_slice(&std::fs::read(lock_path).unwrap()).unwrap();
    {
        let (role, tool, artifact) = (ProviderRole::Symbol, "bsl-analyzer", "bsl-analyzer");
        assert_eq!(super::selected_role_engine(role), Some(tool));
        let pinned = lock["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == tool)
            .unwrap();
        assert_eq!(pinned["releaseName"].as_str().unwrap_or(tool), artifact);
    }
}

/// rule: INV.WIRE.SEARCH-SELECTED-ROLE
#[test]
fn public_role_search_runs_only_selected_provider_and_cannot_use_a_neighbor_success() {
    for selected_role in ProviderRole::ALL {
        for selected_fails in [false, true] {
            let counters = (0..3)
                .map(|_| Arc::new(AtomicUsize::new(0)))
                .collect::<Vec<_>>();
            let providers = ProviderRole::ALL
                .into_iter()
                .enumerate()
                .map(|(index, role)| {
                    Arc::new(CountingSearchProvider {
                        identity: ProviderIdentity::new(role, format!("provider-{index}")),
                        calls: Arc::clone(&counters[index]),
                        fails: selected_fails && role == selected_role,
                    }) as Arc<dyn CodeIntelligenceProvider>
                })
                .collect();
            let execution = super::search_selected_role(
                CodeIntelligenceRegistry::new(providers).unwrap(),
                selected_role,
                &SearchRequest {
                    query: "Needle".into(),
                    limit: 7,
                },
                &selected_role_test_context(),
                &CancellationToken::new(),
                CodeIntelligenceDeadlines::for_test(Duration::from_secs(1)),
            )
            .expect("selected search executes");
            assert_eq!(
                execution.ok, !selected_fails,
                "selected provider owns outcome"
            );
            assert_eq!(execution.result.sections.len(), 1);
            assert_eq!(execution.result.sections[0].identity.role, selected_role);
            for (index, role) in ProviderRole::ALL.into_iter().enumerate() {
                assert_eq!(
                    counters[index].load(Ordering::SeqCst),
                    usize::from(role == selected_role),
                    "{role:?} called during {selected_role:?} search"
                );
            }
        }
    }
}

/// rule: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
#[test]
fn public_role_search_surfaces_partial_provider_warning_and_hit() {
    use unica_code_intelligence::coordinator::CodeSearchExecution;
    use unica_code_intelligence::model::CodeSearchResult;
    use unica_code_intelligence::model::ProviderSearchHit;
    use unica_code_intelligence::model::SearchCoverage;
    use unica_source_contract::location::SourceLocation;

    let section = ProviderSearchSection::partial(
        ProviderIdentity::new(ProviderRole::Semantic, "semantic-fixture"),
        SearchRanking::Provider,
        SearchOrdering::Provider,
        vec![ProviderSearchHit {
            rank: Some(1),
            provider_score: None,
            location: SourceLocation::Unaddressable {
                source_set: "main".to_string(),
                owner_metadata_path: None,
                path: "CommonModules/Sales/Ext/Module.bsl".to_string(),
            },
            line: 1,
            end_line: None,
            symbol: Some("Post".to_string()),
            kind: Some("procedure".to_string()),
            snippet: "Post".to_string(),
            attributes: serde_json::Map::new(),
        }],
        vec!["ignored malformed provider result #1".to_string()],
    )
    .unwrap();
    let result = super::provider_search_page(
        &unica_navigation::result_store::SearchCursorStore::default(),
        ProviderRole::Semantic,
        CodeSearchExecution {
            ok: true,
            result: CodeSearchResult {
                coverage: SearchCoverage::Partial,
                elapsed_ms: 1,
                sections: vec![section],
            },
            warnings: vec!["semantic-fixture: ignored malformed provider result #1".to_string()],
            errors: Vec::new(),
        },
        provider_search_test_binding(ProviderRole::Semantic, 20),
        None,
        &CancellationToken::new(),
    );
    let serialized = serde_json::to_value(result).unwrap();
    assert_eq!(serialized["ok"], true);
    assert!(serialized["summary"]
        .as_str()
        .unwrap()
        .contains("incomplete"));
    assert_eq!(serialized["warnings"].as_array().map(Vec::len), Some(1));
    assert_eq!(serialized["data"]["matches"][0]["status"], "partial");
    assert_eq!(
        serialized["data"]["matches"][0]["hits"][0]["symbol"],
        "Post"
    );
}

/// rule: INV.WIRE.SEARCH-SELECTED-ROLE
#[test]
fn selected_role_search_does_not_start_a_provider_after_parent_cancellation() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(CountingSearchProvider {
        identity: ProviderIdentity::new(ProviderRole::Lexical, "lexical"),
        calls: Arc::clone(&calls),
        fails: false,
    });
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let error = super::search_selected_role(
        CodeIntelligenceRegistry::new(vec![provider]).unwrap(),
        ProviderRole::Lexical,
        &SearchRequest {
            query: "Needle".into(),
            limit: 7,
        },
        &selected_role_test_context(),
        &cancellation,
        CodeIntelligenceDeadlines::for_test(Duration::from_secs(1)),
    )
    .expect_err("cancelled search must not start");
    assert!(error.starts_with(unica_work_control::cancellation::CANCELLED_PREFIX));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn computed_can_section_may_exceed_the_preferred_page_size() {
    let at = "main:Catalog.Товары";
    let mut result = unica_execution_contract::invocation::DomainResult::success(
        "logical collection page resolved",
    );
    result.at = Some(at.to_string());
    result.rev = Some("rev-1".to_string());
    result.page = Some(json!({"stoppedBy": "complete"}));
    let data = json!({"at": at, "kind": "Catalog", "title": "Товары", "items": [
        {"name": "А".repeat(32_600)}
    ]});
    result.data = Some(data.clone());
    assert!(
        serde_json::to_vec(&result).unwrap().len()
            <= crate::application::v13::view::PREFERRED_PAGE_BYTES
    );
    let mut enriched = data;
    super::super::v13_read_modes::attach_can(
        &mut enriched,
        super::super::v13_read_modes::configuration_context(),
    );
    result.data = Some(enriched);
    assert!(
        serde_json::to_vec(&result).unwrap().len()
            > crate::application::v13::view::PREFERRED_PAGE_BYTES
    );
    let checked = super::enforce_view_result_limit(result, at);
    assert!(checked.ok);
}

#[test]
fn response_enrichment_cannot_exceed_the_transport_result_limit() {
    let at = "main:Catalog.Товары";
    let mut result = unica_execution_contract::invocation::DomainResult::success(
        "logical collection page resolved",
    );
    result.at = Some(at.to_string());
    result.page = Some(json!({"stoppedBy": "complete"}));
    result.data = Some(json!({"at": at, "items": ["X".repeat(
        unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES
    )]}));
    let checked = super::enforce_view_result_limit(result, at);
    assert!(!checked.ok);
    assert_eq!(checked.diagnostics[0]["code"], "result_too_large");
}

#[test]
fn unfinished_work_outlives_task_handoff_and_completes_once() {
    crate::infrastructure::daemon::server::actor_capacity_tests::assert_unfinished_work_survives_handoff_and_completes_once();
}

/// Every page of a text search, collected through the canonical entry.
fn text_search_pages(
    workspace: &std::path::Path,
    engine: TextEngine,
    question: serde_json::Value,
) -> Vec<unica_execution_contract::invocation::DomainResult> {
    text_search_pages_with(
        workspace,
        Arc::new(super::CanonicalV13ReadService::with_text_engine_for_test(
            engine,
        )),
        question,
    )
}

fn text_search_pages_with(
    workspace: &std::path::Path,
    service: Arc<super::CanonicalV13ReadService>,
    question: serde_json::Value,
) -> Vec<unica_execution_contract::invocation::DomainResult> {
    use unica_execution::invocation_store::ToolIdentity;
    let mut pages = Vec::new();
    let mut arguments = question;
    loop {
        let page = invoke_exact_read_with_service(
            workspace,
            ToolIdentity::Search,
            arguments.clone(),
            service.clone(),
        );
        assert!(page.ok, "{arguments} after {} pages: {page:?}", pages.len());
        let cursor = page.cursor.clone();
        pages.push(page);
        match cursor {
            Some(cursor) => arguments["cursor"] = json!(cursor),
            None => break pages,
        }
    }
}

fn text_search_matches(
    pages: &[unica_execution_contract::invocation::DomainResult],
) -> Vec<serde_json::Value> {
    pages
        .iter()
        .flat_map(|page| {
            page.data.as_ref().unwrap()["matches"]
                .as_array()
                .unwrap()
                .clone()
        })
        .collect()
}

/// The text-search corpus of the engine tests: modules, files outside
/// the module layout, names that Git orders before a same-named
/// directory, a byte-order mark, CRLF, several matches on one line and
/// Cyrillic case.
fn text_engine_workspace() -> tempfile::TempDir {
    let workspace = resolve_exact_workspace();
    let source = workspace.path().join("src");
    for (path, text) in [
            (
                "CommonModules/Probe/Ext/Module.bsl",
                "\u{feff}// Needle at the start\r\nПроцедура Ping()\r\n\tNeedle(Needle); // NEEDLE needle\r\nКонецПроцедуры\r\n",
            ),
            ("Catalogs/Items/Ext/ObjectModule.bsl", "// ПРОЦЕДУРА Needle\n// процедура needle\n"),
            ("Extra.bsl", "Needle outside the layout\n"),
            ("Extra-b.bsl", "Needle sorts before the dot\n"),
            ("Extra/inner.bsl", "Needle in a same-named directory\n"),
            ("Extra/Deep/z.bsl", "Needle deeper\n"),
        ] {
            let target = source.join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, text).unwrap();
        }
    workspace
}

/// #2095 (regex), the case flag and the seam of the two engines: a first
/// page read through Git and its continuation by the walk give the same
/// pages, matches, order and addresses as the walk alone, wherever the
/// Git page stops.
/// rule: INV.APP.LITERAL-SEARCH
#[test]
fn literal_search_engines_agree_on_matches_order_and_addresses() {
    let workspace = text_engine_workspace();
    // A user's grep defaults change neither the records nor the paths.
    for (key, value) in [("grep.column", "true"), ("grep.fullName", "true")] {
        assert!(std::process::Command::new("git")
            .args(["config", key, value])
            .current_dir(workspace.path())
            .status()
            .unwrap()
            .success());
    }
    for question in [
        // One match per page re-enters every file, `Extra.bsl` and
        // `Extra/` included.
        json!({"query":"Needle","limit":1}),
        json!({"query":"Needle","limit":3}),
        json!({"query":"needle","ignoreCase":true,"limit":2}),
        json!({"query":"процедура","ignoreCase":true}),
        // A regular expression is read by the walk in either case.
        json!({"query":"Need(le|ed)","regex":true,"limit":4}),
    ] {
        let walk = text_search_pages(workspace.path(), TextEngine::Walk, question.clone());
        let git = text_search_pages(workspace.path(), TextEngine::Git, question.clone());
        let walk_matches = text_search_matches(&walk);
        assert!(!walk_matches.is_empty(), "{question}: {walk:?}");
        assert_eq!(
            walk_matches,
            text_search_matches(&git),
            "{question}: both engines give one answer"
        );
        assert_eq!(walk.len(), git.len(), "{question}: the same pages");
        let last = walk.last().unwrap().data.as_ref().unwrap();
        assert_eq!(last["fileCoverage"]["complete"], true, "{question}");
        assert_eq!(
            last["fileCoverage"],
            git.last().unwrap().data.as_ref().unwrap()["fileCoverage"],
            "{question}: the same coverage"
        );
    }
    let walk = text_search_matches(&text_search_pages(
        workspace.path(),
        TextEngine::Walk,
        json!({"query":"Needle","limit":50}),
    ));
    let places = walk
        .iter()
        .map(|hit| {
            (
                hit.get("at").or(hit.get("unaddressed")).cloned().unwrap(),
                hit["line"].as_u64().unwrap(),
                hit["column"].as_u64().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        places,
        vec![
            (json!("main:Catalog.Items.Module.Object.Body#L1"), 1, 14),
            (json!("main:CommonModule.Probe.Method.Ping.Body#L1"), 1, 4),
            (json!("main:CommonModule.Probe.Method.Ping.Body#L3"), 3, 2),
            (json!("main:CommonModule.Probe.Method.Ping.Body#L3"), 3, 9),
            (json!("outside_module_layout"), 1, 1),
            (json!("outside_module_layout"), 1, 1),
            (json!("outside_module_layout"), 1, 1),
            (json!("outside_module_layout"), 1, 1),
        ],
        "Git's path order: `Extra-b.bsl` < `Extra.bsl` < `Extra/` and the BOM is no column"
    );
    assert_eq!(walk[4]["snippet"], "Needle sorts before the dot");
    assert_eq!(walk[5]["snippet"], "Needle outside the layout");
    assert_eq!(walk[6]["snippet"], "Needle deeper");
    assert_eq!(walk[7]["snippet"], "Needle in a same-named directory");
    let case = text_search_matches(&text_search_pages(
        workspace.path(),
        TextEngine::Walk,
        json!({"query":"процедура","ignoreCase":true}),
    ));
    assert_eq!(case.len(), 6, "{case:?}");
}

/// The first page of either engine carries the exact volume of the
/// whole corpus, however small the page: counts by kind, the modules
/// and the matches without an address. A continuation does not repeat it.
/// rule: INV.APP.LITERAL-SEARCH
/// rule: INV.WIRE.SEARCH-CORPUS
#[test]
fn first_literal_page_counts_the_whole_corpus_by_kind_and_module() {
    let workspace = text_engine_workspace();
    let expected = json!({
        "matches": 8,
        "files": 6,
        "kinds": {
            "CommonModule": {"matches": 3, "files": 1},
            "Catalog": {"matches": 1, "files": 1},
        },
        "unaddressed": {"matches": 4, "files": 4},
        "modules": [
            {"at": "main:CommonModule.Probe", "matches": 3},
            {"at": "main:Catalog.Items.Module.Object", "matches": 1},
        ],
    });
    for limit in [1, 50] {
        for engine in [TextEngine::Git, TextEngine::Walk] {
            let pages = text_search_pages(
                workspace.path(),
                engine,
                json!({"query":"Needle","limit":limit}),
            );
            let first = pages[0].data.as_ref().unwrap();
            assert_eq!(first["totals"], expected, "{engine:?} limit {limit}");
            assert_eq!(text_search_matches(&pages).len(), 8);
            for page in &pages[1..] {
                assert!(
                    page.data.as_ref().unwrap().get("totals").is_none(),
                    "{engine:?}: a continuation repeats no volume"
                );
            }
            if limit == 1 {
                // The page itself still stops after one match.
                assert_eq!(first["matches"].as_array().unwrap().len(), 1);
                assert_eq!(first["fileCoverage"]["scanComplete"], false);
                assert_eq!(pages.len(), 8);
            }
        }
    }
    let empty = text_search_pages(workspace.path(), TextEngine::Git, json!({"query":"Absent"}));
    assert_eq!(empty.len(), 1);
    assert_eq!(
        empty[0].data.as_ref().unwrap()["totals"],
        json!({"matches": 0, "files": 0, "kinds": {}, "modules": []})
    );
    assert_eq!(empty[0].page.as_ref().unwrap()["stoppedBy"], "complete");
}

/// A file left uncovered past the first page counts in the volume of
/// the corpus, not in the coverage of that page; the page that reads
/// it reports it. The configuration's own module counts as its kind.
/// rule: INV.APP.LITERAL-SEARCH
#[test]
fn literal_volume_counts_uncovered_files_past_the_page_apart_from_its_coverage() {
    let workspace = text_engine_workspace();
    let source = workspace.path().join("src");
    std::fs::write(source.join("Zzz.bsl"), b"Needle\n\xffNeedle\xff\nNeedle\n").unwrap();
    std::fs::create_dir_all(source.join("Ext")).unwrap();
    std::fs::write(source.join("Ext/SessionModule.bsl"), "// Needle\n").unwrap();
    for engine in [TextEngine::Git, TextEngine::Walk] {
        let pages = text_search_pages(
            workspace.path(),
            engine,
            json!({"query":"Needle","limit":1}),
        );
        let first = pages[0].data.as_ref().unwrap();
        assert_eq!(first["fileCoverage"]["uncovered"], 0, "{engine:?}: {first}");
        let totals = &first["totals"];
        assert_eq!(totals["uncovered"], 1, "{engine:?}: {totals}");
        // The unreadable line stops the file: only its first match counts.
        assert_eq!(totals["matches"], 10, "{engine:?}: {totals}");
        assert_eq!(
            totals["kinds"]["Configuration"],
            json!({"matches": 1, "files": 1}),
            "{engine:?}: {totals}"
        );
        assert_eq!(
            text_search_matches(&pages).len(),
            10,
            "the pages deliver what the volume counts"
        );
        let last = pages.last().unwrap().data.as_ref().unwrap();
        assert_eq!(last["fileCoverage"]["uncovered"], 1, "{engine:?}: {last}");
    }
}

/// Past twenty modules the volume names the twenty with the most
/// matches, a tie in traversal order, and the rest as one line; both
/// engines count the same.
/// rule: INV.APP.LITERAL-SEARCH
#[test]
fn literal_volume_names_twenty_leading_modules_and_counts_the_rest() {
    let workspace = resolve_exact_workspace();
    let source = workspace.path().join("src");
    let names = (1..=23)
        .map(|index| format!("M{index:02}"))
        .collect::<Vec<_>>();
    let registered = names
        .iter()
        .map(|name| format!("<CommonModule>{name}</CommonModule>"))
        .collect::<String>();
    std::fs::write(
            source.join("Configuration.xml"),
            format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="f3317000-0000-4000-8000-000000000001"><Properties><Name>Store</Name></Properties><ChildObjects><CommonModule>Probe</CommonModule><Catalog>Items</Catalog><CommonForm>Main</CommonForm>{registered}</ChildObjects></Configuration></MetaDataObject>"#),
        )
        .unwrap();
    for (index, name) in names.iter().enumerate() {
        let number = index + 1;
        // M20 ties with M21: the earlier one in traversal order leads.
        let count = if number == 20 { 21 } else { number };
        std::fs::write(
                source.join(format!("CommonModules/{name}.xml")),
                format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="f3317000-0000-4000-8000-0000000001{number:02}"><Properties><Name>{name}</Name><Global>false</Global><ClientManagedApplication>false</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>false</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties><ChildObjects/></CommonModule></MetaDataObject>"#),
            )
            .unwrap();
        let module = source.join(format!("CommonModules/{name}/Ext/Module.bsl"));
        std::fs::create_dir_all(module.parent().unwrap()).unwrap();
        std::fs::write(module, "// Lead\n".repeat(count)).unwrap();
    }
    let leaders = [23, 22, 20, 21]
        .into_iter()
        .chain((4..=19).rev())
        .map(|number| {
            let matches = if number == 20 { 21 } else { number };
            json!({"at": format!("main:CommonModule.M{number:02}"), "matches": matches})
        })
        .collect::<Vec<_>>();
    let expected = json!({
        "matches": 277,
        "files": 23,
        "kinds": {"CommonModule": {"matches": 277, "files": 23}},
        "modules": leaders,
        "rest": {"modules": 3, "matches": 6},
    });
    for engine in [TextEngine::Git, TextEngine::Walk] {
        let first = invoke_exact_read_with_service(
            workspace.path(),
            unica_execution::invocation_store::ToolIdentity::Search,
            json!({"query":"Lead"}),
            Arc::new(super::CanonicalV13ReadService::with_text_engine_for_test(
                engine,
            )),
        );
        assert!(first.ok, "{first:?}");
        assert_eq!(
            first.data.as_ref().unwrap()["totals"],
            expected,
            "{engine:?}"
        );
        assert!(first.cursor.is_some());
    }
}

/// A search cancelled while it counts past its full page answers
/// neither the volume nor the page: the refusal stays as before.
/// rule: INV.APP.LITERAL-SEARCH
#[test]
fn cancelled_literal_search_answers_no_volume() {
    let workspace = text_engine_workspace();
    for engine in [TextEngine::Git, TextEngine::Walk] {
        let service = Arc::new(super::CanonicalV13ReadService {
            text_engine: Some(engine),
            cancel_past_page: true,
            ..super::CanonicalV13ReadService::default()
        });
        let result = invoke_exact_read_with_service(
            workspace.path(),
            unica_execution::invocation_store::ToolIdentity::Search,
            json!({"query":"Needle","limit":1}),
            service,
        );
        assert!(!result.ok, "{engine:?}: {result:?}");
        assert_eq!(result.diagnostics[0]["code"], "cancelled", "{result:?}");
        assert!(result.data.is_none(), "{engine:?}: {result:?}");
        assert!(result.cursor.is_none());
    }
}

/// `git grep` reads neither a submodule nor an embedded repository. A
/// source root holding either is read by the walk: a match there that
/// comes first in file order is on the first page, and the answer is
/// the walk's own.
/// rule: INV.APP.LITERAL-SEARCH
#[test]
fn literal_search_reads_submodules_and_embedded_repositories() {
    let git = |directory: &std::path::Path, args: &[&str]| {
        let status = std::process::Command::new("git")
            .args([
                "-c",
                "user.email=probe@example.invalid",
                "-c",
                "user.name=probe",
            ])
            .args(args)
            .current_dir(directory)
            .output()
            .unwrap();
        assert!(status.status.success(), "{args:?}: {status:?}");
    };
    for gitlink in [false, true] {
        let workspace = text_engine_workspace();
        let nested = workspace.path().join("src/Aaa");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("first.bsl"), "Needle in a nested work tree\n").unwrap();
        git(&nested, &["init", "--quiet"]);
        if gitlink {
            git(&nested, &["add", "first.bsl"]);
            git(
                &nested,
                &["commit", "--quiet", "--no-gpg-sign", "-m", "nested"],
            );
            git(workspace.path(), &["add", "src/Aaa"]);
        }
        let corpus = unica_adapter_lexical::lines::CandidateCorpus {
            subtree: None,
            excluded: Vec::new(),
            excluded_name_prefixes: Vec::new(),
        };
        let foreign = |root: &std::path::Path| {
            unica_adapter_lexical::lines::holds_foreign_work_tree(
                unica_process::runner::system_process_runner(),
                root,
                &corpus,
                1 << 16,
                &CancellationToken::new(),
            )
        };
        assert!(foreign(&workspace.path().join("src")), "gitlink {gitlink}");
        // Outside the nested tree Git reads the corpus whole.
        assert!(!foreign(&workspace.path().join("src/CommonModules")));
        let chosen = text_search_pages_with(
            workspace.path(),
            Arc::new(super::CanonicalV13ReadService::default()),
            json!({"query":"Needle","limit":1}),
        );
        let walk = text_search_pages(
            workspace.path(),
            TextEngine::Walk,
            json!({"query":"Needle","limit":1}),
        );
        let first = &chosen[0].data.as_ref().unwrap()["matches"][0];
        assert_eq!(
            first["snippet"], "Needle in a nested work tree",
            "gitlink {gitlink}"
        );
        assert_eq!(
            text_search_matches(&chosen),
            text_search_matches(&walk),
            "gitlink {gitlink}"
        );
        assert_eq!(text_search_matches(&walk).len(), 9, "gitlink {gitlink}");
    }
}

/// #2096: a match without a proven module names its file by an
/// identifier, never by path, in either engine.
/// rule: INV.WIRE.SEARCH-CORPUS
#[test]
fn literal_search_hit_without_an_address_carries_no_path() {
    let workspace = text_engine_workspace();
    for engine in [TextEngine::Walk, TextEngine::Git] {
        let pages = text_search_pages(
            workspace.path(),
            engine,
            json!({"query":"outside the layout"}),
        );
        let matches = text_search_matches(&pages);
        assert_eq!(matches.len(), 1, "{engine:?}: {matches:?}");
        let hit = &matches[0];
        assert_eq!(hit["unaddressed"], "outside_module_layout");
        assert!(hit.get("at").is_none(), "{hit}");
        let file_id = hit["fileId"].as_str().unwrap();
        assert!(
            file_id.len() == 20
                && file_id.starts_with("bsl-")
                && file_id[4..].chars().all(|c| c.is_ascii_hexdigit()),
            "{file_id}"
        );
        let text = serde_json::to_string(&pages[0]).unwrap();
        assert!(
            !text.contains("Extra.bsl") && !text.contains(".bsl\""),
            "{text}"
        );
    }
}

/// #2095: the former `lexical` role is refused with the call that
/// answers it; the literal search takes a regular expression itself.
/// rule: INV.WIRE.SEARCH-PROVIDER-ROLES
#[test]
fn retired_lexical_role_is_refused_with_the_search_without_a_role() {
    use crate::infrastructure::daemon::protocol::InvocationRequest;
    use crate::infrastructure::daemon::server::V5CanonicalInvocationRuntime;
    use unica_execution::clock::TokioClock;
    use unica_execution::invocation_store::ToolIdentity;
    let workspace = text_engine_workspace();
    let runtime = V5CanonicalInvocationRuntime::new(
        Arc::new(super::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    );
    let refused = *runtime
        .bind(
            InvocationRequest::new(
                ToolIdentity::Search,
                json!({"query":"Need(le|ed)","regex":true,"role":"lexical"}),
                std::fs::canonicalize(workspace.path())
                    .unwrap()
                    .to_string_lossy(),
                7_000,
            )
            .unwrap(),
        )
        .unwrap()
        .prepare()
        .err()
        .expect("admission refuses the retired role");
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(refused.diagnostics[0]["code"], "bad_value");
    assert!(
        refused.summary.contains("omit `role`"),
        "{}",
        refused.summary
    );
    assert_eq!(refused.next[0]["tool"], "unica.search");
    assert_eq!(
        refused.next[0]["args"],
        json!({"query":"Need(le|ed)","regex":true}),
        "the same question without the role"
    );
    assert!(refused.data.is_none(), "no empty answer: {refused:?}");
}

/// A first page read through Git continues past 200 matches through the
/// walk at the same place: a replay repeats the page, an edit after the
/// cursor is read, an edit of the delivered part is stale, and a corpus
/// that now ends at the cursor ends the search.
/// rule: INV.APP.LITERAL-SEARCH
#[test]
fn git_first_page_continues_by_the_walk_to_the_last_match() {
    use unica_execution::invocation_store::ToolIdentity;
    let workspace = resolve_exact_workspace();
    let module = workspace
        .path()
        .join("src/CommonModules/Probe/Ext/Module.bsl");
    let lines = |count: usize| {
        (1..=count)
            .map(|line| format!("// Needle {line}\n"))
            .collect::<String>()
    };
    std::fs::write(&module, lines(450)).unwrap();
    let service = Arc::new(super::CanonicalV13ReadService::with_text_engine_for_test(
        TextEngine::Git,
    ));
    let search = |cursor: Option<&str>| {
        let mut arguments = json!({"query":"Needle","scope":"main:CommonModule.Probe","limit":50});
        if let Some(cursor) = cursor {
            arguments["cursor"] = json!(cursor);
        }
        invoke_exact_read_with_service(
            workspace.path(),
            ToolIdentity::Search,
            arguments,
            service.clone(),
        )
    };
    let mut cursors = Vec::new();
    let mut seen = Vec::new();
    loop {
        let page = search(cursors.last().map(String::as_str));
        assert!(page.ok, "{page:?}");
        seen.extend(
            page.data.as_ref().unwrap()["matches"]
                .as_array()
                .unwrap()
                .iter()
                .map(|hit| hit["line"].as_u64().unwrap()),
        );
        match page.cursor {
            Some(cursor) => cursors.push(cursor),
            None => break,
        }
    }
    assert_eq!(
        seen,
        (1..=450).collect::<Vec<_>>(),
        "past 200, no loss, no repeat"
    );
    let page = search(Some(&cursors[4]));
    let replay = search(Some(&cursors[4]));
    assert_eq!((&page.data, &page.cursor), (&replay.data, &replay.cursor));
    assert_eq!(page.data.as_ref().unwrap()["matches"][0]["line"], 251);

    let mut tail = lines(450);
    tail.push_str("// Needle appended\n");
    std::fs::write(&module, &tail).unwrap();
    let after_tail = search(Some(&cursors[4]));
    assert_eq!(after_tail.data, page.data, "{after_tail:?}");

    std::fs::write(&module, lines(250)).unwrap();
    let ended = search(Some(&cursors[4]));
    assert!(ended.ok, "{ended:?}");
    assert_eq!(ended.data.as_ref().unwrap()["matches"], json!([]));
    assert_eq!(
        ended.data.as_ref().unwrap()["fileCoverage"]["scanComplete"],
        true
    );
    assert!(ended.cursor.is_none());

    std::fs::write(
        &module,
        lines(450).replacen("// Needle 11\n", "// Needle eleven\n", 1),
    )
    .unwrap();
    let edited = search(Some(&cursors[4]));
    assert_eq!(edited.diagnostics[0]["code"], "stale_cursor", "{edited:?}");
    // The cursor the Git page issued is bound the same way.
    let edited_first = search(Some(&cursors[0]));
    assert_eq!(
        edited_first.diagnostics[0]["code"], "stale_cursor",
        "{edited_first:?}"
    );
}

/// A continuation resumes at the file of the last delivered match
/// instead of reading the corpus again: a replay repeats the page, an
/// edit in that file before the cursor or its removal is stale, and
/// files passed before it are not read again.
/// rule: INV.APP.LITERAL-SEARCH
#[test]
fn walk_literal_continuation_resumes_at_its_file() {
    use unica_execution::invocation_store::ToolIdentity;
    let workspace = resolve_exact_workspace();
    let source = workspace.path().join("src");
    for name in ["A", "B", "C"] {
        let directory = source.join(format!("Loose/{name}"));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("x.bsl"), "Needle\n".repeat(30)).unwrap();
    }
    let walk = Arc::new(super::CanonicalV13ReadService::with_text_engine_for_test(
        TextEngine::Walk,
    ));
    let search = |service: &Arc<super::CanonicalV13ReadService>, cursor: Option<&str>| {
        let mut arguments = json!({"query":"Needle","limit":20});
        if let Some(cursor) = cursor {
            arguments["cursor"] = json!(cursor);
        }
        invoke_exact_read_with_service(
            workspace.path(),
            ToolIdentity::Search,
            arguments,
            service.clone(),
        )
    };
    let all = text_search_matches(&text_search_pages(
        workspace.path(),
        TextEngine::Walk,
        json!({"query":"Needle","limit":20}),
    ));
    assert_eq!(all.len(), 90);
    let first = search(&walk, None);
    let second = search(&walk, first.cursor.as_deref());
    let third = search(&walk, second.cursor.as_deref());
    assert!(third.ok, "{third:?}");
    let replay = search(&walk, second.cursor.as_deref());
    assert_eq!((&replay.data, &replay.cursor), (&third.data, &third.cursor));
    // The second cursor re-enters `Loose/B/x.bsl` after its tenth match.
    assert_eq!(
        text_search_matches(&[first.clone(), second.clone(), third.clone()]),
        all[..60].to_vec()
    );

    // An edit after the cursor is read by the next page.
    std::fs::write(source.join("Loose/C/x.bsl"), "Needle\n".repeat(31)).unwrap();
    let after = search(&walk, second.cursor.as_deref());
    assert_eq!(after.data, third.data, "{after:?}");
    // A delivered match of the resumed file changed.
    std::fs::write(
        source.join("Loose/B/x.bsl"),
        format!("Needle!\n{}", "Needle\n".repeat(29)),
    )
    .unwrap();
    let stale = search(&walk, second.cursor.as_deref());
    assert_eq!(stale.diagnostics[0]["code"], "stale_cursor", "{stale:?}");
    std::fs::write(source.join("Loose/B/x.bsl"), "Needle\n".repeat(30)).unwrap();
    // A file before the resumed one is not read again.
    std::fs::remove_file(source.join("Loose/A/x.bsl")).unwrap();
    let behind = search(&walk, second.cursor.as_deref());
    assert_eq!((&behind.data, &behind.cursor), (&third.data, &third.cursor));
    // The resumed file disappeared with its directory; later files remain.
    std::fs::remove_dir_all(source.join("Loose/B")).unwrap();
    let moved = search(&walk, second.cursor.as_deref());
    assert_eq!(moved.diagnostics[0]["code"], "stale_cursor", "{moved:?}");
}

/// A walk page ends at its time share of the response budget at a
/// file boundary, even with no match yet, and the cursor carries the
/// walk on from there: every match is reached, none repeats.
/// rule: INV.APP.LITERAL-SEARCH
#[test]
fn walk_pages_end_by_time_and_continue_without_loss() {
    let workspace = resolve_exact_workspace();
    let source = workspace.path().join("src");
    for index in 0..40 {
        let directory = source.join(format!("Loose/F{index:02}"));
        std::fs::create_dir_all(&directory).unwrap();
        let text = if index % 13 == 12 {
            "Needle\n"
        } else {
            "// nothing\n"
        };
        std::fs::write(directory.join("x.bsl"), text).unwrap();
    }
    let service = Arc::new(super::CanonicalV13ReadService {
        text_engine: Some(TextEngine::Walk),
        walk_budget: Some(Duration::ZERO),
        ..super::CanonicalV13ReadService::default()
    });
    // The first page counts the whole corpus and does not end by time;
    // its continuations do.
    let pages = text_search_pages_with(
        workspace.path(),
        service,
        json!({"query":"Needle","limit":1}),
    );
    let matches = text_search_matches(&pages);
    assert_eq!(matches.len(), 3, "{matches:?}");
    assert_eq!(pages[0].page.as_ref().unwrap()["stoppedBy"], "limit");
    assert_eq!(pages[0].data.as_ref().unwrap()["totals"]["matches"], 3);
    let by_time = pages
        .iter()
        .filter(|page| page.page.as_ref().unwrap()["stoppedBy"] == "deadline")
        .collect::<Vec<_>>();
    assert!(by_time.len() > 3, "{} pages", pages.len());
    assert!(by_time.iter().all(|page| page.cursor.is_some()));
    assert!(by_time
        .iter()
        .any(|page| page.data.as_ref().unwrap()["matches"]
            .as_array()
            .unwrap()
            .is_empty()));
    let last = pages.last().unwrap().data.as_ref().unwrap();
    assert_eq!(last["fileCoverage"]["scanComplete"], true);
}

/// The former `lexical` deadline key stays accepted in `unica.toml` for
/// one release and no longer limits the literal search: even a value the
/// configuration would refuse leaves the search whole.
/// rule: INV.APP.LITERAL-SEARCH
#[test]
fn literal_search_ignores_the_retired_git_grep_timeout_key() {
    let workspace = text_engine_workspace();
    let expected = text_search_matches(&text_search_pages(
        workspace.path(),
        TextEngine::Git,
        json!({"query":"Needle"}),
    ));
    for value in ["1", "0"] {
        std::fs::write(
            workspace.path().join("unica.toml"),
            format!("[operational.code_intelligence]\nsearch_git_grep_timeout_seconds = {value}\n"),
        )
        .unwrap();
        for engine in [TextEngine::Git, TextEngine::Walk] {
            let pages = text_search_pages(workspace.path(), engine, json!({"query":"Needle"}));
            assert_eq!(text_search_matches(&pages), expected, "{value} {engine:?}");
        }
    }
}

/// A line that is not UTF-8 and cannot hold a match is no gap: the
/// first page read through Git and the walk that continues it agree on
/// the file, and the continuation reaches every later match.
#[test]
fn unreadable_line_without_a_match_does_not_break_the_continuation() {
    let workspace = text_engine_workspace();
    std::fs::write(
        workspace
            .path()
            .join("src/CommonModules/Probe/Ext/Module.bsl"),
        b"// Mark 1\n\xff\n// Mark 2\n// Mark 3\n",
    )
    .unwrap();
    for engine in [TextEngine::Git, TextEngine::Walk] {
        let pages = text_search_pages(workspace.path(), engine, json!({"query":"Mark","limit":2}));
        let lines = text_search_matches(&pages)
            .iter()
            .map(|hit| hit["line"].as_u64().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(lines, vec![1, 3, 4], "{engine:?}");
        let last = pages.last().unwrap().data.as_ref().unwrap();
        assert_eq!(last["fileCoverage"]["complete"], true, "{engine:?}: {last}");
    }
}

/// A line too long to read that Git selects names no file; the first
/// page then reads its corpus by the walk, so every page counts the same
/// uncovered file and no count is lost to the cursor.
#[test]
fn overlong_candidate_line_keeps_coverage_across_the_continuation() {
    let workspace = text_engine_workspace();
    std::fs::write(
        workspace
            .path()
            .join("src/Catalogs/Items/Ext/ManagerModule.bsl"),
        format!("// {}Needle\n", "x".repeat(8 * 1024 * 1024)),
    )
    .unwrap();
    for engine in [TextEngine::Git, TextEngine::Walk] {
        let pages = text_search_pages(
            workspace.path(),
            engine,
            json!({"query":"Needle","limit":2}),
        );
        assert!(pages.len() > 1, "{engine:?}");
        for page in &pages {
            let coverage = &page.data.as_ref().unwrap()["fileCoverage"];
            assert_eq!(coverage["uncovered"], 1, "{engine:?}: {coverage}");
        }
        let last = &pages.last().unwrap().data.as_ref().unwrap()["fileCoverage"];
        assert_eq!(last["uncoveredIsLowerBound"], false, "{engine:?}: {last}");
        assert_eq!(last["complete"], false, "{engine:?}: {last}");
    }
}
