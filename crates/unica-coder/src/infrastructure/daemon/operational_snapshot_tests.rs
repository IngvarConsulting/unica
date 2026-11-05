use super::*;
use crate::infrastructure::{application_ports, operational_config::load_observer};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};
use unica_code_intelligence::model::*;

const OLD: u64 = 7111703;
const NEW: u64 = 8111907;
fn write_settings(root: &std::path::Path, value: u64) {
    std::fs::write(root.join("unica.toml"),format!("[operational.code_intelligence]\nsearch_total_timeout_seconds = {value}\nsearch_rlm_timeout_seconds = {value}\nsearch_git_grep_timeout_seconds = {value}\nprovider_read_timeout_seconds = {value}\n[operational.code_diagnostics]\nanalyze_timeout_seconds = {value}\n")).unwrap();
}
fn no_values(result: &DomainResult, root: &std::path::Path) {
    let text = serde_json::to_string(result).unwrap();
    for secret in [
        OLD.to_string(),
        NEW.to_string(),
        "invalid-setting-secret-969".to_owned(),
        root.to_string_lossy().into_owned(),
    ] {
        assert!(
            !text.contains(&secret),
            "configuration value leaked: {text}"
        );
    }
}
struct Provider {
    role: ProviderRole,
    calls: Mutex<Vec<(usize, Option<Duration>)>>,
    change: AtomicBool,
    root: std::path::PathBuf,
}
impl Provider {
    fn new(root: &std::path::Path, role: ProviderRole) -> Arc<Self> {
        Arc::new(Self {
            role,
            calls: Mutex::new(Vec::new()),
            change: AtomicBool::new(true),
            root: root.to_owned(),
        })
    }
    fn record(&self, limit: usize, deadline: ProviderDeadline) {
        self.calls
            .lock()
            .unwrap()
            .push((limit, deadline.remaining()));
        if self.change.swap(false, Ordering::AcqRel) {
            write_settings(&self.root, NEW);
        }
    }
}
impl CodeIntelligenceProvider for Provider {
    fn identity(&self) -> ProviderIdentity {
        ProviderIdentity::new(self.role, "snapshot-fixture")
    }
    fn capabilities(&self) -> &[ProviderCapability] {
        &[ProviderCapability::CallGraph, ProviderCapability::Search]
    }
    fn search(
        &self,
        _: &SearchRequest,
        _: &CodeIntelligenceContext,
        deadline: ProviderDeadline,
        _: &CancellationToken,
    ) -> ProviderSearchSection {
        self.record(0, deadline);
        ProviderSearchSection::complete(
            self.identity(),
            SearchRanking::Provider,
            SearchOrdering::Provider,
            vec![],
            vec![],
        )
        .unwrap()
    }
    fn read(
        &self,
        request: &CodeIntelligenceReadRequest,
        _: &CodeIntelligenceContext,
        deadline: ProviderDeadline,
        _: &CancellationToken,
    ) -> Result<ProviderReadOutcome, String> {
        let CodeIntelligenceReadRequest::CallGraph {
            id,
            direction,
            limit,
        } = request
        else {
            panic!("unexpected read")
        };
        assert!(
            [
                "method/common/Handlers/Run",
                "method/common/Handlers/Peer00"
            ]
            .contains(&id.as_str()),
            "{id}"
        );
        self.record(*limit, deadline);
        let total = if *direction == CallGraphDirection::Callers {
            51
        } else {
            0
        };
        let edges = (0..total.min(*limit))
            .map(|i| CallGraphEdge {
                id: format!("method/common/Handlers/Peer{i:02}"),
                provenance: CallEdgeProvenance::Resolved,
            })
            .collect();
        Ok(ProviderReadOutcome {
            provider: self.identity(),
            ok: true,
            summary: "graph".into(),
            warnings: vec![],
            errors: vec![],
            artifacts: vec![],
            stdout: None,
            stderr: None,
            data: Some(CodeIntelligenceReadData::CallGraph(CallGraphResult {
                state: CallGraphState::Ready,
                edges,
                total: Some(total as u64),
                revision: Some(7),
                stale: Some(false),
                complete: *limit >= total,
            })),
        })
    }
}
struct Overrides;
impl Drop for Overrides {
    fn drop(&mut self) {
        application_ports::CODE_INTELLIGENCE_PROVIDER_OVERRIDE
            .with(|slot| *slot.borrow_mut() = None);
        application_ports::DIAGNOSTIC_PROVIDER_OVERRIDE.with(|slot| *slot.borrow_mut() = None);
    }
}
fn graph_fixture() -> tempfile::TempDir {
    let mut text = "Procedure Run() Export\nEndProcedure\n".to_owned();
    for i in 0..51 {
        text += &format!("Procedure Peer{i:02}() Export\nEndProcedure\n");
    }
    scheduled_handler_workspace("CONFIGURATION", &text)
}
fn invoke(
    runtime: &V5CanonicalInvocationRuntime,
    root: &std::path::Path,
    tool: ToolIdentity,
    args: serde_json::Value,
    token: CancellationToken,
) -> DomainResult {
    let request = InvocationRequest::new(
        tool,
        args,
        std::fs::canonicalize(root).unwrap().to_string_lossy(),
        7000,
    )
    .unwrap();
    runtime
        .bind(request)
        .unwrap()
        .prepare()
        .unwrap()
        .execute(token)
        .unwrap()
}
fn budget_is(value: Option<Duration>, expected: u64) {
    let actual = value.expect("explicit finite budget");
    assert!(
        actual <= Duration::from_secs(expected) && actual > Duration::from_secs(expected - 10),
        "unexpected remaining budget {actual:?}, expected {expected}"
    );
}
#[test]
fn operational_graph_complete_fetch_keeps_one_snapshot_then_next_call_refreshes() {
    let workspace = graph_fixture();
    let root = std::fs::canonicalize(workspace.path()).unwrap();
    write_settings(&root, OLD);
    let loads = Arc::new(AtomicUsize::new(0));
    let _observer = load_observer::install(
        &root,
        Arc::new({
            let loads = loads.clone();
            move |failed| {
                assert!(!failed);
                loads.fetch_add(1, Ordering::AcqRel);
            }
        }),
    );
    let provider = Provider::new(&root, ProviderRole::Symbol);
    let _overrides = Overrides;
    application_ports::CODE_INTELLIGENCE_PROVIDER_OVERRIDE
        .with(|slot| *slot.borrow_mut() = Some(vec![provider.clone()]));
    let runtime = bootstrap_runtime();
    let plain = invoke(
        &runtime,
        &root,
        ToolIdentity::View,
        serde_json::json!({"at":"main:CommonModule.Handlers.Method.Run"}),
        CancellationToken::new(),
    );
    assert!(plain.ok, "{plain:?}");
    assert_eq!(loads.load(Ordering::Acquire), 0);
    assert!(provider.calls.lock().unwrap().is_empty());
    no_values(&plain, &root);
    let args = serde_json::json!({"at":"main:CommonModule.Handlers.Method.Run.Caller"});
    let result = invoke(
        &runtime,
        &root,
        ToolIdentity::View,
        args.clone(),
        CancellationToken::new(),
    );
    assert!(result.ok, "{result:?}");
    no_values(&result, &root);
    assert_eq!(result.data.as_ref().unwrap()["props"]["total"], 51);
    let calls = provider.calls.lock().unwrap().clone();
    assert!(
        calls.iter().any(|(limit, _)| *limit == 51),
        "must perform complete fetch: {calls:?}"
    );
    assert_eq!(
        loads.load(Ordering::Acquire),
        1,
        "one public graph call reloaded operational settings"
    );
    for (_, budget) in &calls {
        budget_is(*budget, OLD);
    }
    provider.calls.lock().unwrap().clear();
    let result = invoke(
        &runtime,
        &root,
        ToolIdentity::View,
        args,
        CancellationToken::new(),
    );
    assert!(result.ok, "{result:?}");
    no_values(&result, &root);
    assert_eq!(loads.load(Ordering::Acquire), 2);
    for (_, budget) in provider.calls.lock().unwrap().iter() {
        budget_is(*budget, NEW);
    }
}

#[test]
fn operational_search_keeps_loaded_budget_and_refreshes_next_call() {
    let workspace = graph_fixture();
    let root = std::fs::canonicalize(workspace.path()).unwrap();
    write_settings(&root, OLD);
    let loads = Arc::new(AtomicUsize::new(0));
    let _observer = load_observer::install(
        &root,
        Arc::new({
            let loads = loads.clone();
            move |failed| {
                assert!(!failed);
                loads.fetch_add(1, Ordering::AcqRel);
            }
        }),
    );
    let provider = Provider::new(&root, ProviderRole::Semantic);
    let service = Arc::new(
        super::super::super::v13_service::CanonicalV13ReadService::with_search_providers_for_test(
            vec![provider.clone()],
        ),
    );
    let runtime = V5CanonicalInvocationRuntime::new(service, Arc::new(TokioClock));
    for (index, value) in [OLD, NEW].into_iter().enumerate() {
        let result = invoke(
            &runtime,
            &root,
            ToolIdentity::Search,
            serde_json::json!({"query":"Run","role":"semantic","scope":"main:Configuration"}),
            CancellationToken::new(),
        );
        assert!(result.ok, "{result:?}");
        no_values(&result, &root);
        assert_eq!(loads.load(Ordering::Acquire), index + 1);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls.len(), index + 1);
        budget_is(calls[index].1, value);
    }
}

#[test]
fn operational_bsl_check_holds_loaded_timeout_and_refreshes_next_call() {
    let workspace = graph_fixture();
    let root = std::fs::canonicalize(workspace.path()).unwrap();
    write_settings(&root, OLD);
    let loads = Arc::new(AtomicUsize::new(0));
    let _observer = load_observer::install(
        &root,
        Arc::new({
            let loads = loads.clone();
            let root = root.clone();
            move |failed| {
                assert!(!failed);
                if loads.fetch_add(1, Ordering::AcqRel) == 0 {
                    write_settings(&root, NEW);
                }
            }
        }),
    );
    let (provider, backend) =
        unica_adapter_bsl_analyzer::diagnostics::analyze_timeout_probe::recording_provider();
    let _overrides = Overrides;
    application_ports::DIAGNOSTIC_PROVIDER_OVERRIDE
        .with(|slot| *slot.borrow_mut() = Some(provider));
    let runtime = bootstrap_runtime();
    for (index, value) in [OLD, NEW].into_iter().enumerate() {
        let result = invoke(
            &runtime,
            &root,
            ToolIdentity::Check,
            serde_json::json!({"at":"main:CommonModule.Handlers.Body"}),
            CancellationToken::new(),
        );
        assert!(result.ok, "{result:?}");
        no_values(&result, &root);
        assert_eq!(loads.load(Ordering::Acquire), index + 1);
        let timeouts = backend.timeouts();
        assert_eq!(timeouts.len(), index + 1);
        budget_is(timeouts[index], value);
    }
}

fn invalid_load_matrix(cancel: bool) {
    for mode in 0..3 {
        let workspace = graph_fixture();
        let root = std::fs::canonicalize(workspace.path()).unwrap();
        std::fs::write(root.join("unica.toml"),"[operational.code_intelligence]\nprovider_read_timeout_seconds='invalid-setting-secret-969'\n").unwrap();
        let loads = Arc::new(AtomicUsize::new(0));
        let token = CancellationToken::new();
        let _observer = load_observer::install(
            &root,
            Arc::new({
                let loads = loads.clone();
                let token = token.clone();
                move |failed| {
                    assert!(failed, "the real loader must reject the invalid value");
                    loads.fetch_add(1, Ordering::AcqRel);
                    assert!(
                        !token.is_cancelled(),
                        "cancellation must race with the actual failed load"
                    );
                    if cancel {
                        token.cancel();
                    }
                }
            }),
        );
        let provider = Provider::new(
            &root,
            if mode == 0 {
                ProviderRole::Semantic
            } else {
                ProviderRole::Symbol
            },
        );
        let (diagnostic, backend) =
            unica_adapter_bsl_analyzer::diagnostics::analyze_timeout_probe::recording_provider();
        let _overrides = Overrides;
        application_ports::CODE_INTELLIGENCE_PROVIDER_OVERRIDE
            .with(|slot| *slot.borrow_mut() = Some(vec![provider.clone()]));
        application_ports::DIAGNOSTIC_PROVIDER_OVERRIDE
            .with(|slot| *slot.borrow_mut() = Some(diagnostic));
        let service=Arc::new(super::super::super::v13_service::CanonicalV13ReadService::with_search_providers_for_test(vec![provider.clone()]));
        let runtime = V5CanonicalInvocationRuntime::new(service, Arc::new(TokioClock));
        let (tool, args) = match mode {
            0 => (
                ToolIdentity::Search,
                serde_json::json!({"query":"Run","role":"semantic","scope":"main:Configuration"}),
            ),
            1 => (
                ToolIdentity::Check,
                serde_json::json!({"at":"main:CommonModule.Handlers.Body"}),
            ),
            _ => (
                ToolIdentity::View,
                serde_json::json!({"at":"main:CommonModule.Handlers.Method.Run.Caller"}),
            ),
        };
        // Prepared execution includes ActorBoundExecution::publish: a local
        // invalid_state result cannot masquerade as the final cancellation.
        let result = invoke(&runtime, &root, tool, args, token.clone());
        assert!(!result.ok, "{result:?}");
        let expected = if cancel { "cancelled" } else { "invalid_state" };
        assert_eq!(
            result.diagnostics.first().unwrap()["code"],
            expected,
            "mode={mode}: {result:?}"
        );
        if cancel {
            assert!(
                !result
                    .diagnostics
                    .iter()
                    .any(|d| d["code"] == "invalid_state"),
                "config error displaced cancellation: {result:?}"
            );
        }
        assert_eq!(loads.load(Ordering::Acquire), 1);
        assert!(provider.calls.lock().unwrap().is_empty());
        assert!(backend.timeouts().is_empty());
        no_values(&result, &root);
    }
}
#[test]
fn operational_invalid_load_stops_all_canonical_providers_and_redacts_values() {
    invalid_load_matrix(false);
}
#[test]
fn operational_cancellation_during_failed_load_wins_at_actor_publication() {
    invalid_load_matrix(true);
}

#[test]
fn operational_diff_graph_sides_share_one_lazy_snapshot() {
    let workspace = graph_fixture();
    let root = std::fs::canonicalize(workspace.path()).unwrap();
    write_settings(&root, OLD);
    let loads = Arc::new(AtomicUsize::new(0));
    let _observer = load_observer::install(
        &root,
        Arc::new({
            let loads = loads.clone();
            move |failed| {
                assert!(!failed);
                loads.fetch_add(1, Ordering::AcqRel);
            }
        }),
    );
    let provider = Provider::new(&root, ProviderRole::Symbol);
    let _overrides = Overrides;
    application_ports::CODE_INTELLIGENCE_PROVIDER_OVERRIDE
        .with(|slot| *slot.borrow_mut() = Some(vec![provider.clone()]));
    let runtime = bootstrap_runtime();
    let method = "main:CommonModule.Handlers.Method.Run";
    let plain = invoke(
        &runtime,
        &root,
        ToolIdentity::Diff,
        serde_json::json!({"left":method,"right":method}),
        CancellationToken::new(),
    );
    assert!(plain.ok, "{plain:?}");
    assert_eq!(loads.load(Ordering::Acquire), 0);
    assert!(provider.calls.lock().unwrap().is_empty());
    let args = serde_json::json!({"left":format!("{method}.Caller"),"right":"main:CommonModule.Handlers.Method.Peer00.Caller"});
    for (index, budget) in [OLD, NEW].into_iter().enumerate() {
        let result = invoke(
            &runtime,
            &root,
            ToolIdentity::Diff,
            args.clone(),
            CancellationToken::new(),
        );
        assert!(result.ok, "{result:?}");
        no_values(&result, &root);
        let calls = provider.calls.lock().unwrap().clone();
        assert!(
            calls.len() >= 8,
            "must execute both graph sides plus the full 51-neighbor fetch: {calls:?}"
        );
        assert_eq!(
            loads.load(Ordering::Acquire),
            index + 1,
            "the sides of one public diff reloaded settings"
        );
        for (_, value) in calls {
            budget_is(value, budget);
        }
        provider.calls.lock().unwrap().clear();
    }
}
