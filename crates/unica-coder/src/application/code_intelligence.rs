use crate::domain::cancellation::{cancelled_error, CancellationToken};
use crate::domain::code_intelligence::{
    CodeIntelligenceContext, CodeIntelligenceProvider, CodeIntelligenceReadRequest,
    CodeIntelligenceRegistry, CodeSearchResult, ProviderDeadline, ProviderId, ProviderReadOutcome,
    ProviderSearchSection, ProviderSectionStatus, SearchRequest,
};
use crate::domain::operational_config::CodeIntelligenceDeadlines;
use std::any::Any;
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const MAX_CONCURRENT_WORKERS_PER_PROVIDER: usize = 32;

#[derive(Debug)]
pub(crate) struct CodeSearchExecution {
    pub(crate) ok: bool,
    pub(crate) result: CodeSearchResult,
    pub(crate) warnings: Vec<String>,
    pub(crate) errors: Vec<String>,
}

pub(crate) struct CodeSearchCoordinator {
    registry: CodeIntelligenceRegistry,
    deadlines: CodeIntelligenceDeadlines,
    worker_admission: Arc<ProviderWorkerAdmission>,
    worker_lifecycle: Arc<ProviderWorkerLifecycle>,
}

impl CodeSearchCoordinator {
    #[cfg(test)]
    pub(crate) fn new(registry: CodeIntelligenceRegistry) -> Self {
        Self::with_deadlines(
            registry,
            crate::domain::operational_config::OperationalConfig::compiled_defaults()
                .code_intelligence(),
        )
    }

    pub(crate) fn with_deadlines(
        registry: CodeIntelligenceRegistry,
        deadlines: CodeIntelligenceDeadlines,
    ) -> Self {
        Self {
            registry,
            deadlines,
            worker_admission: global_provider_worker_admission(),
            worker_lifecycle: global_provider_worker_lifecycle(),
        }
    }

    #[cfg(test)]
    fn with_policy(
        registry: CodeIntelligenceRegistry,
        public_search_budget: Duration,
        worker_admission: Arc<ProviderWorkerAdmission>,
        worker_lifecycle: Arc<ProviderWorkerLifecycle>,
    ) -> Self {
        Self {
            registry,
            deadlines: CodeIntelligenceDeadlines::for_test(public_search_budget),
            worker_admission,
            worker_lifecycle,
        }
    }

    pub(crate) fn search(
        &self,
        request: &SearchRequest,
        context: &CodeIntelligenceContext,
        cancellation: &CancellationToken,
    ) -> Result<CodeSearchExecution, String> {
        if cancellation.is_cancelled() {
            return Err(cancelled_error(
                "unica.code.search stopped before providers started",
            ));
        }

        let providers = self.registry.search_provider_arcs();
        let provider_ids = providers
            .iter()
            .map(|provider| provider.id())
            .collect::<Vec<_>>();
        let started_at = Instant::now();
        let public_search_budget = self.deadlines.search_total_timeout();
        let (tx, rx) = mpsc::channel();
        let mut slots = vec![None; providers.len()];
        let provider_budgets = provider_ids
            .iter()
            .map(|provider| self.provider_budget(*provider).min(public_search_budget))
            .collect::<Vec<_>>();
        let provider_cancellations = provider_ids
            .iter()
            .map(|_| cancellation.linked_child())
            .collect::<Vec<_>>();
        for (index, provider) in providers.into_iter().enumerate() {
            let provider_id = provider.id();
            let Some(worker_permit) = self.worker_admission.try_acquire(provider_id) else {
                slots[index] = Some(provider_admission_exhausted_section(
                    provider_id,
                    self.worker_admission.per_provider_limit,
                ));
                continue;
            };
            let tx = tx.clone();
            let request = request.clone();
            let context = context.clone();
            let budget = provider_budgets[index];
            let worker_cancellation = provider_cancellations[index].clone();
            let spawn_result = thread::Builder::new()
                .name(format!("unica-code-search-{}", provider_id.as_str()))
                .spawn(move || {
                    let _worker_permit = worker_permit;
                    let mut section = catch_unwind(AssertUnwindSafe(|| {
                        provider.search(
                            &request,
                            &context,
                            ProviderDeadline::from_started_at(started_at, budget),
                            &worker_cancellation,
                        )
                    }))
                    .unwrap_or_else(|panic| failed_after_panic(provider_id, panic));
                    if section.provider != provider_id {
                        section.diagnostics.push(format!(
                            "provider returned mismatched id {}; normalized to {}",
                            section.provider.as_str(),
                            provider_id.as_str()
                        ));
                        section.provider = provider_id;
                    }
                    let _ = tx.send((index, section));
                });
            match spawn_result {
                Ok(handle) => self.worker_lifecycle.track(handle),
                Err(error) => {
                    slots[index] = Some(ProviderSearchSection {
                        provider: provider_id,
                        status: ProviderSectionStatus::Failed,
                        hits: Vec::new(),
                        diagnostics: vec![format!("failed to start provider worker: {error}")],
                        artifacts: Vec::new(),
                    });
                }
            }
        }
        drop(tx);

        while slots.iter().any(Option::is_none) {
            if cancellation.is_cancelled() {
                for token in &provider_cancellations {
                    token.cancel();
                }
                return Err(cancelled_error(
                    "unica.code.search stopped while providers were running",
                ));
            }

            let elapsed = started_at.elapsed();
            for (index, slot) in slots.iter_mut().enumerate() {
                if slot.is_none() && elapsed >= provider_budgets[index] {
                    provider_cancellations[index].cancel();
                    *slot = Some(provider_timeout_section(
                        provider_ids[index],
                        provider_budgets[index],
                    ));
                }
            }
            if slots.iter().all(Option::is_some) {
                break;
            }

            let next_deadline = slots
                .iter()
                .enumerate()
                .filter(|(_, slot)| slot.is_none())
                .filter_map(|(index, _)| provider_budgets[index].checked_sub(started_at.elapsed()))
                .min()
                .unwrap_or(Duration::ZERO);
            let wait = next_deadline.min(Duration::from_millis(50));
            match rx.recv_timeout(wait) {
                Ok((index, section)) if slots[index].is_none() => slots[index] = Some(section),
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        if cancellation.is_cancelled() {
            for token in &provider_cancellations {
                token.cancel();
            }
            return Err(cancelled_error(
                "unica.code.search stopped while providers were running",
            ));
        }
        for token in &provider_cancellations {
            token.cancel();
        }

        let sections = slots
            .into_iter()
            .zip(provider_ids)
            .map(|(section, provider)| {
                normalize_provider_section(
                    section.unwrap_or_else(|| ProviderSearchSection {
                        provider,
                        status: ProviderSectionStatus::Failed,
                        hits: Vec::new(),
                        diagnostics: vec!["provider worker ended without a result".to_string()],
                        artifacts: Vec::new(),
                    }),
                    request.limit,
                )
            })
            .collect::<Vec<_>>();
        self.worker_lifecycle.reap();
        let ok = sections.iter().any(|section| {
            matches!(
                section.status,
                ProviderSectionStatus::Ok | ProviderSectionStatus::Empty
            )
        });
        let mut warnings = Vec::new();
        let mut errors = Vec::new();
        if sections.is_empty() {
            errors.push("no search-capable code intelligence providers are registered".to_string());
        }
        for section in &sections {
            if matches!(
                section.status,
                ProviderSectionStatus::Failed | ProviderSectionStatus::Unavailable
            ) {
                let message = section_problem(section);
                if ok {
                    warnings.push(message);
                } else {
                    errors.push(message);
                }
            }
        }
        let result = CodeSearchResult { sections };

        Ok(CodeSearchExecution {
            ok,
            result,
            warnings,
            errors,
        })
    }

    fn provider_budget(&self, provider: ProviderId) -> Duration {
        match provider {
            ProviderId::Rlm => self.deadlines.search_rlm_timeout(),
            ProviderId::BslAnalyzer => self.deadlines.search_total_timeout(),
            ProviderId::GitGrep => self.deadlines.search_git_grep_timeout(),
        }
    }
}

struct ProviderWorkerAdmission {
    per_provider_limit: usize,
    active: Mutex<HashMap<ProviderId, usize>>,
}

struct ProviderWorkerLifecycle {
    handles: Mutex<Vec<thread::JoinHandle<()>>>,
}

impl ProviderWorkerLifecycle {
    fn new() -> Self {
        Self {
            handles: Mutex::new(Vec::new()),
        }
    }

    fn track(&self, handle: thread::JoinHandle<()>) {
        let mut handles = self
            .handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::reap_finished(&mut handles);
        handles.push(handle);
    }

    fn drain(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            {
                let mut handles = self
                    .handles
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                Self::reap_finished(&mut handles);
                if handles.is_empty() {
                    return true;
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            thread::sleep(remaining.min(Duration::from_millis(10)));
        }
    }

    fn reap(&self) {
        let mut handles = self
            .handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::reap_finished(&mut handles);
    }

    fn reap_finished(handles: &mut Vec<thread::JoinHandle<()>>) {
        let mut index = 0;
        while index < handles.len() {
            if handles[index].is_finished() {
                let handle = handles.swap_remove(index);
                let _ = handle.join();
            } else {
                index += 1;
            }
        }
    }

    #[cfg(test)]
    fn pending_count(&self) -> usize {
        let mut handles = self
            .handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::reap_finished(&mut handles);
        handles.len()
    }
}

fn normalize_provider_section(
    mut section: ProviderSearchSection,
    limit: usize,
) -> ProviderSearchSection {
    if matches!(
        section.status,
        ProviderSectionStatus::Failed | ProviderSectionStatus::Unavailable
    ) {
        section.hits.clear();
        return section;
    }
    section.hits.truncate(limit);
    for (index, hit) in section.hits.iter_mut().enumerate() {
        hit.rank = index + 1;
    }
    section.status = if section.hits.is_empty() {
        ProviderSectionStatus::Empty
    } else {
        ProviderSectionStatus::Ok
    };
    section
}

impl ProviderWorkerAdmission {
    fn new(per_provider_limit: usize) -> Self {
        Self {
            per_provider_limit: per_provider_limit.max(1),
            active: Mutex::new(HashMap::new()),
        }
    }

    fn try_acquire(self: &Arc<Self>, provider: ProviderId) -> Option<ProviderWorkerPermit> {
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let count = active.entry(provider).or_default();
        if *count >= self.per_provider_limit {
            return None;
        }
        *count += 1;
        Some(ProviderWorkerPermit {
            admission: Arc::clone(self),
            provider,
        })
    }

    #[cfg(test)]
    fn active_count(&self, provider: ProviderId) -> usize {
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&provider)
            .copied()
            .unwrap_or_default()
    }
}

struct ProviderWorkerPermit {
    admission: Arc<ProviderWorkerAdmission>,
    provider: ProviderId,
}

impl Drop for ProviderWorkerPermit {
    fn drop(&mut self) {
        let mut active = self
            .admission
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(count) = active.get_mut(&self.provider) else {
            return;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            active.remove(&self.provider);
        }
    }
}

fn global_provider_worker_admission() -> Arc<ProviderWorkerAdmission> {
    static ADMISSION: OnceLock<Arc<ProviderWorkerAdmission>> = OnceLock::new();
    Arc::clone(ADMISSION.get_or_init(|| {
        Arc::new(ProviderWorkerAdmission::new(
            MAX_CONCURRENT_WORKERS_PER_PROVIDER,
        ))
    }))
}

fn global_provider_worker_lifecycle() -> Arc<ProviderWorkerLifecycle> {
    static LIFECYCLE: OnceLock<Arc<ProviderWorkerLifecycle>> = OnceLock::new();
    Arc::clone(LIFECYCLE.get_or_init(|| Arc::new(ProviderWorkerLifecycle::new())))
}

pub(crate) fn drain_code_search_workers(timeout: Duration) -> bool {
    global_provider_worker_lifecycle().drain(timeout)
}

pub(crate) fn execute_provider_read(
    provider: Arc<dyn CodeIntelligenceProvider>,
    request: CodeIntelligenceReadRequest,
    context: CodeIntelligenceContext,
    budget: Duration,
    cancellation: &CancellationToken,
) -> Result<ProviderReadOutcome, String> {
    execute_provider_read_with_policy(
        provider,
        request,
        context,
        budget,
        global_provider_worker_admission(),
        global_provider_worker_lifecycle(),
        cancellation,
    )
}

fn execute_provider_read_with_policy(
    provider: Arc<dyn CodeIntelligenceProvider>,
    request: CodeIntelligenceReadRequest,
    context: CodeIntelligenceContext,
    budget: Duration,
    worker_admission: Arc<ProviderWorkerAdmission>,
    worker_lifecycle: Arc<ProviderWorkerLifecycle>,
    cancellation: &CancellationToken,
) -> Result<ProviderReadOutcome, String> {
    if cancellation.is_cancelled() {
        return Err(cancelled_error(
            "code intelligence read stopped before provider start",
        ));
    }
    let provider_id = provider.id();
    let _permit = worker_admission.try_acquire(provider_id).ok_or_else(|| {
        format!(
            "{} read provider worker capacity exhausted (limit {})",
            provider_id.as_str(),
            worker_admission.per_provider_limit
        )
    })?;
    let worker_cancellation = cancellation.linked_child();
    let worker_token = worker_cancellation.clone();
    let started_at = Instant::now();
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = thread::Builder::new()
        .name(format!("unica-code-read-{}", provider_id.as_str()))
        .spawn(move || {
            let _permit = _permit;
            let result = catch_unwind(AssertUnwindSafe(|| {
                provider.read(
                    &request,
                    &context,
                    ProviderDeadline::from_started_at(started_at, budget),
                    &worker_token,
                )
            }))
            .map_err(|panic| {
                let detail = panic
                    .downcast_ref::<&str>()
                    .map(|value| (*value).to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic payload".to_string());
                format!("{} read provider panicked: {detail}", provider_id.as_str())
            })
            .and_then(|result| result);
            let _ = tx.send(result);
        })
        .map_err(|error| {
            format!(
                "failed to start {} read provider worker: {error}",
                provider_id.as_str()
            )
        })?;
    worker_lifecycle.track(handle);

    loop {
        if cancellation.is_cancelled() {
            worker_cancellation.cancel();
            return Err(cancelled_error(
                "code intelligence read stopped while provider was running",
            ));
        }
        let remaining = budget
            .checked_sub(started_at.elapsed())
            .unwrap_or(Duration::ZERO);
        if remaining.is_zero() {
            worker_cancellation.cancel();
            return Err(format!(
                "{} provider exceeded its {} ms read budget",
                provider_id.as_str(),
                budget.as_millis()
            ));
        }
        match rx.recv_timeout(remaining.min(Duration::from_millis(25))) {
            Ok(result) => {
                let result =
                    arbitrate_provider_read_result(result, cancellation, &worker_cancellation);
                worker_lifecycle.reap();
                return result;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                worker_lifecycle.reap();
                return Err(format!(
                    "{} read provider ended without a result",
                    provider_id.as_str()
                ));
            }
        }
    }
}

fn arbitrate_provider_read_result(
    result: Result<ProviderReadOutcome, String>,
    cancellation: &CancellationToken,
    worker_cancellation: &CancellationToken,
) -> Result<ProviderReadOutcome, String> {
    if cancellation.is_cancelled() {
        worker_cancellation.cancel();
        return Err(cancelled_error(
            "code intelligence read stopped while provider was running",
        ));
    }
    result
}

#[cfg(test)]
pub(crate) fn track_code_search_worker_for_test(handle: thread::JoinHandle<()>) {
    global_provider_worker_lifecycle().track(handle);
}

fn provider_timeout_section(provider: ProviderId, budget: Duration) -> ProviderSearchSection {
    let diagnostic = if provider == ProviderId::GitGrep {
        format!(
            "git-grep search was too slow and exceeded its {} ms budget before completion",
            budget.as_millis()
        )
    } else {
        format!(
            "provider exceeded its {} ms search budget",
            budget.as_millis()
        )
    };
    ProviderSearchSection {
        provider,
        status: ProviderSectionStatus::Failed,
        hits: Vec::new(),
        diagnostics: vec![diagnostic],
        artifacts: Vec::new(),
    }
}

fn provider_admission_exhausted_section(
    provider: ProviderId,
    limit: usize,
) -> ProviderSearchSection {
    ProviderSearchSection {
        provider,
        status: ProviderSectionStatus::Unavailable,
        hits: Vec::new(),
        diagnostics: vec![format!(
            "provider worker capacity exhausted (limit {limit})"
        )],
        artifacts: Vec::new(),
    }
}

fn failed_after_panic(provider: ProviderId, panic: Box<dyn Any + Send>) -> ProviderSearchSection {
    let detail = panic
        .downcast_ref::<&str>()
        .map(|value| (*value).to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic payload".to_string());
    ProviderSearchSection {
        provider,
        status: ProviderSectionStatus::Failed,
        hits: Vec::new(),
        diagnostics: vec![format!("provider panicked: {detail}")],
        artifacts: Vec::new(),
    }
}

fn section_problem(section: &ProviderSearchSection) -> String {
    let detail = if section.diagnostics.is_empty() {
        section.status.as_str().to_string()
    } else {
        section.diagnostics.join("; ")
    };
    format!("{}: {detail}", section.provider.as_str())
}

#[cfg(test)]
mod tests {
    use super::{
        arbitrate_provider_read_result, execute_provider_read_with_policy, CodeSearchCoordinator,
        ProviderWorkerAdmission, ProviderWorkerLifecycle,
    };
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::{
        CodeIntelligenceContext, CodeIntelligenceProvider, CodeIntelligenceReadRequest,
        CodeIntelligenceRegistry, ProviderCapability, ProviderDeadline, ProviderId,
        ProviderReadOutcome, ProviderSearchHit, ProviderSearchSection, ProviderSectionStatus,
        SearchRequest,
    };
    use crate::domain::operational_config::CodeIntelligenceDeadlines;
    use crate::domain::source_roots::ResolvedSourceRoot;
    use crate::domain::workspace::WorkspaceContext;
    use serde_json::Map;
    use std::path::PathBuf;
    use std::sync::{mpsc, Arc, Barrier};
    use std::thread;
    use std::time::{Duration, Instant};

    struct GateProvider {
        id: ProviderId,
        started: mpsc::Sender<ProviderId>,
        release: Arc<Barrier>,
        status: ProviderSectionStatus,
    }

    impl CodeIntelligenceProvider for GateProvider {
        fn id(&self) -> ProviderId {
            self.id
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
            self.started.send(self.id).unwrap();
            self.release.wait();
            ProviderSearchSection {
                provider: self.id,
                status: self.status,
                hits: Vec::new(),
                diagnostics: Vec::new(),
                artifacts: Vec::new(),
            }
        }
    }

    fn context() -> CodeIntelligenceContext {
        CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: PathBuf::from("/workspace"),
                workspace_root: PathBuf::from("/workspace"),
                cache_root: PathBuf::from("/cache"),
                workspace_epoch: 3,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: PathBuf::from("/workspace/src"),
            },
        )
    }

    #[test]
    fn coordinator_starts_all_providers_before_waiting_and_restores_registry_order() {
        let (started_tx, started_rx) = mpsc::channel();
        let release = Arc::new(Barrier::new(4));
        let providers = [
            (ProviderId::Rlm, ProviderSectionStatus::Ok),
            (ProviderId::BslAnalyzer, ProviderSectionStatus::Empty),
            (ProviderId::GitGrep, ProviderSectionStatus::Ok),
        ]
        .into_iter()
        .map(|(id, status)| {
            Arc::new(GateProvider {
                id,
                started: started_tx.clone(),
                release: Arc::clone(&release),
                status,
            }) as Arc<dyn CodeIntelligenceProvider>
        })
        .collect();
        let coordinator =
            CodeSearchCoordinator::new(CodeIntelligenceRegistry::new(providers).unwrap());

        let worker = thread::spawn(move || {
            coordinator
                .search(
                    &SearchRequest {
                        query: "Post".to_string(),
                        limit: 20,
                    },
                    &context(),
                    &CancellationToken::new(),
                )
                .unwrap()
        });

        let mut started = Vec::new();
        for _ in 0..3 {
            started.push(
                started_rx
                    .recv_timeout(Duration::from_secs(2))
                    .expect("all providers must start concurrently"),
            );
        }
        assert_eq!(started.len(), 3);
        release.wait();

        let execution = worker.join().unwrap();
        assert!(execution.ok);
        assert_eq!(
            execution
                .result
                .sections
                .iter()
                .map(|section| section.provider)
                .collect::<Vec<_>>(),
            vec![
                ProviderId::Rlm,
                ProviderId::BslAnalyzer,
                ProviderId::GitGrep
            ]
        );
    }

    struct StaticProvider {
        id: ProviderId,
        section: ProviderSearchSection,
    }

    impl CodeIntelligenceProvider for StaticProvider {
        fn id(&self) -> ProviderId {
            self.id
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
            self.section.clone()
        }
    }

    fn static_provider(
        id: ProviderId,
        status: ProviderSectionStatus,
        diagnostic: &str,
    ) -> Arc<dyn CodeIntelligenceProvider> {
        Arc::new(StaticProvider {
            id,
            section: ProviderSearchSection {
                provider: id,
                status,
                hits: Vec::new(),
                diagnostics: (!diagnostic.is_empty())
                    .then(|| diagnostic.to_string())
                    .into_iter()
                    .collect(),
                artifacts: Vec::new(),
            },
        })
    }

    #[test]
    fn partial_success_is_ok_and_all_failed_is_not() {
        let request = SearchRequest {
            query: "Post".to_string(),
            limit: 20,
        };
        let cancellation = CancellationToken::new();
        let partial = CodeSearchCoordinator::new(
            CodeIntelligenceRegistry::new(vec![
                static_provider(
                    ProviderId::Rlm,
                    ProviderSectionStatus::Unavailable,
                    "index building",
                ),
                static_provider(ProviderId::BslAnalyzer, ProviderSectionStatus::Empty, ""),
                static_provider(
                    ProviderId::GitGrep,
                    ProviderSectionStatus::Failed,
                    "git failed",
                ),
            ])
            .unwrap(),
        )
        .search(&request, &context(), &cancellation)
        .unwrap();

        assert!(partial.ok);
        assert_eq!(partial.warnings.len(), 2);

        let failed = CodeSearchCoordinator::new(
            CodeIntelligenceRegistry::new(vec![
                static_provider(
                    ProviderId::Rlm,
                    ProviderSectionStatus::Unavailable,
                    "index building",
                ),
                static_provider(
                    ProviderId::BslAnalyzer,
                    ProviderSectionStatus::Failed,
                    "analyzer failed",
                ),
                static_provider(
                    ProviderId::GitGrep,
                    ProviderSectionStatus::Failed,
                    "git failed",
                ),
            ])
            .unwrap(),
        )
        .search(&request, &context(), &cancellation)
        .unwrap();

        assert!(!failed.ok);
        assert_eq!(failed.errors.len(), 3);
    }

    #[test]
    fn empty_registry_reports_why_no_provider_served_search() {
        let execution =
            CodeSearchCoordinator::new(CodeIntelligenceRegistry::new(Vec::new()).unwrap())
                .search(
                    &SearchRequest {
                        query: "Post".to_string(),
                        limit: 20,
                    },
                    &context(),
                    &CancellationToken::new(),
                )
                .unwrap();

        assert!(!execution.ok);
        assert_eq!(
            execution.errors,
            vec!["no search-capable code intelligence providers are registered"]
        );
    }

    #[test]
    fn coordinator_enforces_limit_and_normalizes_provider_local_ranks() {
        let hits = (0..4)
            .map(|index| ProviderSearchHit {
                rank: if index == 0 { 0 } else { 99 },
                provider_score: None,
                path: format!("Module{index}.bsl"),
                line: index + 1,
                end_line: None,
                symbol: None,
                kind: None,
                snippet: format!("hit {index}"),
                attributes: Map::new(),
            })
            .collect();
        let provider = Arc::new(StaticProvider {
            id: ProviderId::GitGrep,
            section: ProviderSearchSection {
                provider: ProviderId::GitGrep,
                status: ProviderSectionStatus::Ok,
                hits,
                diagnostics: Vec::new(),
                artifacts: Vec::new(),
            },
        });

        let execution =
            CodeSearchCoordinator::new(CodeIntelligenceRegistry::new(vec![provider]).unwrap())
                .search(
                    &SearchRequest {
                        query: "Post".to_string(),
                        limit: 2,
                    },
                    &context(),
                    &CancellationToken::new(),
                )
                .unwrap();

        assert_eq!(execution.result.sections[0].hits.len(), 2);
        assert_eq!(
            execution.result.sections[0]
                .hits
                .iter()
                .map(|hit| hit.rank)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    struct BudgetProvider {
        id: ProviderId,
        maximum: Duration,
    }

    impl CodeIntelligenceProvider for BudgetProvider {
        fn id(&self) -> ProviderId {
            self.id
        }

        fn capabilities(&self) -> &[ProviderCapability] {
            &[ProviderCapability::Search]
        }

        fn search(
            &self,
            _request: &SearchRequest,
            _context: &CodeIntelligenceContext,
            deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> ProviderSearchSection {
            assert!(deadline.remaining() <= self.maximum);
            ProviderSearchSection {
                provider: self.id,
                status: ProviderSectionStatus::Empty,
                hits: Vec::new(),
                diagnostics: Vec::new(),
                artifacts: Vec::new(),
            }
        }
    }

    #[test]
    fn coordinator_applies_provider_budgets_and_renders_from_canonical_sections() {
        let providers = [
            (ProviderId::Rlm, Duration::from_secs(45)),
            (ProviderId::BslAnalyzer, Duration::from_secs(120)),
            (ProviderId::GitGrep, Duration::from_millis(500)),
        ]
        .into_iter()
        .map(|(id, maximum)| {
            Arc::new(BudgetProvider { id, maximum }) as Arc<dyn CodeIntelligenceProvider>
        })
        .collect();
        let execution =
            CodeSearchCoordinator::new(CodeIntelligenceRegistry::new(providers).unwrap())
                .search(
                    &SearchRequest {
                        query: "Post".to_string(),
                        limit: 20,
                    },
                    &context(),
                    &CancellationToken::new(),
                )
                .unwrap();

        // ADR-0023: the three sections are the result; there is no rendered
        // duplicate of them to compare against.
        assert_eq!(
            execution
                .result
                .sections
                .iter()
                .map(|section| (section.provider.as_str(), section.hits.len()))
                .collect::<Vec<_>>(),
            vec![("rlm", 0), ("bsl-analyzer", 0), ("git-grep", 0)]
        );
        assert_eq!(
            execution
                .result
                .sections
                .iter()
                .map(|section| section.provider)
                .collect::<Vec<_>>(),
            vec![
                ProviderId::Rlm,
                ProviderId::BslAnalyzer,
                ProviderId::GitGrep
            ]
        );
    }

    #[test]
    fn coordinator_projects_each_configured_provider_budget_without_hidden_caps() {
        let deadlines = CodeIntelligenceDeadlines::for_test_values(
            Duration::from_secs(100),
            Duration::from_secs(30),
            Duration::from_secs(70),
            Duration::from_secs(40),
        );
        let coordinator = CodeSearchCoordinator::with_deadlines(
            CodeIntelligenceRegistry::new(Vec::new()).unwrap(),
            deadlines,
        );

        assert_eq!(
            coordinator.provider_budget(ProviderId::BslAnalyzer),
            Duration::from_secs(100)
        );
        assert_eq!(
            coordinator.provider_budget(ProviderId::Rlm),
            Duration::from_secs(30)
        );
        assert_eq!(
            coordinator.provider_budget(ProviderId::GitGrep),
            Duration::from_secs(70)
        );
        assert_eq!(deadlines.provider_read_timeout(), Duration::from_secs(40));
    }

    #[test]
    fn coordinator_accepts_full_positive_i64_config_budget_without_instant_overflow() {
        let maximum = Duration::from_secs(i64::MAX as u64);
        let deadlines = CodeIntelligenceDeadlines::for_test(maximum);
        let coordinator = CodeSearchCoordinator::with_deadlines(
            CodeIntelligenceRegistry::new(vec![static_provider(
                ProviderId::GitGrep,
                ProviderSectionStatus::Empty,
                "",
            )])
            .unwrap(),
            deadlines,
        );

        let execution = coordinator
            .search(
                &SearchRequest {
                    query: "Post".to_string(),
                    limit: 20,
                },
                &context(),
                &CancellationToken::new(),
            )
            .expect("a valid configured budget must not overflow Instant");

        assert!(execution.ok, "{execution:?}");
    }

    struct DeadlineIgnoringProvider(ProviderId);

    impl CodeIntelligenceProvider for DeadlineIgnoringProvider {
        fn id(&self) -> ProviderId {
            self.0
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
            thread::sleep(Duration::from_millis(500));
            ProviderSearchSection {
                provider: ProviderId::BslAnalyzer,
                status: ProviderSectionStatus::Empty,
                hits: Vec::new(),
                diagnostics: Vec::new(),
                artifacts: Vec::new(),
            }
        }
    }

    #[test]
    fn coordinator_enforces_budget_when_provider_ignores_deadline_and_cancellation() {
        let admission = Arc::new(ProviderWorkerAdmission::new(1));
        let lifecycle = Arc::new(ProviderWorkerLifecycle::new());
        let coordinator = CodeSearchCoordinator::with_policy(
            CodeIntelligenceRegistry::new(vec![
                Arc::new(DeadlineIgnoringProvider(ProviderId::BslAnalyzer)),
                static_provider(ProviderId::GitGrep, ProviderSectionStatus::Empty, ""),
            ])
            .unwrap(),
            Duration::from_millis(30),
            Arc::clone(&admission),
            Arc::clone(&lifecycle),
        );
        let started = Instant::now();

        let execution = coordinator
            .search(
                &SearchRequest {
                    query: "Post".to_string(),
                    limit: 20,
                },
                &context(),
                &CancellationToken::new(),
            )
            .unwrap();

        assert!(
            started.elapsed() < Duration::from_millis(250),
            "public search waited for a non-cooperative provider"
        );
        assert!(execution.ok);
        assert_eq!(
            execution.result.sections[0].status,
            ProviderSectionStatus::Failed
        );
        assert!(execution.result.sections[0].diagnostics[0].contains("30 ms search budget"));

        assert_eq!(admission.active_count(ProviderId::BslAnalyzer), 1);
        assert_eq!(lifecycle.pending_count(), 1);
        let second = CodeSearchCoordinator::with_policy(
            CodeIntelligenceRegistry::new(vec![
                Arc::new(DeadlineIgnoringProvider(ProviderId::BslAnalyzer)),
                static_provider(ProviderId::GitGrep, ProviderSectionStatus::Empty, ""),
            ])
            .unwrap(),
            Duration::from_millis(30),
            Arc::clone(&admission),
            Arc::clone(&lifecycle),
        )
        .search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context(),
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(
            second.result.sections[0].status,
            ProviderSectionStatus::Unavailable
        );
        assert!(second.result.sections[0].diagnostics[0].contains("capacity exhausted"));

        assert!(lifecycle.drain(Duration::from_secs(2)));
        assert_eq!(lifecycle.pending_count(), 0);
        assert_eq!(admission.active_count(ProviderId::BslAnalyzer), 0);
    }

    #[test]
    fn indexed_result_survives_git_grep_timeout_with_a_slow_search_warning() {
        let admission = Arc::new(ProviderWorkerAdmission::new(1));
        let lifecycle = Arc::new(ProviderWorkerLifecycle::new());
        let coordinator = CodeSearchCoordinator::with_policy(
            CodeIntelligenceRegistry::new(vec![
                Arc::new(DeadlineIgnoringProvider(ProviderId::GitGrep)),
                static_provider(ProviderId::Rlm, ProviderSectionStatus::Empty, ""),
            ])
            .unwrap(),
            Duration::from_millis(30),
            admission,
            Arc::clone(&lifecycle),
        );

        let execution = coordinator
            .search(
                &SearchRequest {
                    query: "Post".to_string(),
                    limit: 6,
                },
                &context(),
                &CancellationToken::new(),
            )
            .unwrap();

        assert!(execution.ok, "{execution:?}");
        let git_grep = execution
            .result
            .sections
            .iter()
            .find(|section| section.provider == ProviderId::GitGrep)
            .unwrap();
        assert_eq!(git_grep.status, ProviderSectionStatus::Failed);
        assert!(git_grep.hits.is_empty());
        assert!(git_grep.diagnostics[0].contains("too slow"));
        assert!(execution
            .warnings
            .iter()
            .any(|warning| warning.contains("git-grep") && warning.contains("too slow")));
        assert!(lifecycle.drain(Duration::from_secs(2)));
    }

    struct StaticReadProvider;

    impl CodeIntelligenceProvider for StaticReadProvider {
        fn id(&self) -> ProviderId {
            ProviderId::Rlm
        }

        fn capabilities(&self) -> &[ProviderCapability] {
            &[ProviderCapability::Definition, ProviderCapability::Outline]
        }

        fn search(
            &self,
            _request: &SearchRequest,
            _context: &CodeIntelligenceContext,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> ProviderSearchSection {
            unreachable!("read-only fixture")
        }

        fn read(
            &self,
            _request: &CodeIntelligenceReadRequest,
            _context: &CodeIntelligenceContext,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> Result<ProviderReadOutcome, String> {
            Ok(ProviderReadOutcome {
                provider: ProviderId::Rlm,
                ok: true,
                summary: "read".to_string(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: Vec::new(),
                stdout: None,
                stderr: None,
                data: None,
            })
        }
    }

    struct DeadlineIgnoringReadProvider;

    impl CodeIntelligenceProvider for DeadlineIgnoringReadProvider {
        fn id(&self) -> ProviderId {
            ProviderId::Rlm
        }

        fn capabilities(&self) -> &[ProviderCapability] {
            &[ProviderCapability::Definition]
        }

        fn search(
            &self,
            _request: &SearchRequest,
            _context: &CodeIntelligenceContext,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> ProviderSearchSection {
            unreachable!("read-only fixture")
        }

        fn read(
            &self,
            _request: &CodeIntelligenceReadRequest,
            _context: &CodeIntelligenceContext,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> Result<ProviderReadOutcome, String> {
            thread::sleep(Duration::from_millis(500));
            Ok(ProviderReadOutcome {
                provider: ProviderId::Rlm,
                ok: true,
                summary: "late".to_string(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: Vec::new(),
                stdout: None,
                stderr: None,
                data: None,
            })
        }
    }

    #[test]
    fn read_coordinator_enforces_deadline_for_non_cooperative_provider() {
        let admission = Arc::new(ProviderWorkerAdmission::new(1));
        let lifecycle = Arc::new(ProviderWorkerLifecycle::new());
        let started = Instant::now();

        let error = execute_provider_read_with_policy(
            Arc::new(DeadlineIgnoringReadProvider),
            CodeIntelligenceReadRequest::Definition {
                name: "Post".to_string(),
                module_hint: String::new(),
                limit: 50,
            },
            context(),
            Duration::from_millis(30),
            Arc::clone(&admission),
            Arc::clone(&lifecycle),
            &CancellationToken::new(),
        )
        .unwrap_err();

        assert!(error.contains("30 ms read budget"), "{error}");
        assert!(started.elapsed() < Duration::from_millis(250));
        assert_eq!(admission.active_count(ProviderId::Rlm), 1);
        assert!(lifecycle.drain(Duration::from_secs(2)));
        assert_eq!(admission.active_count(ProviderId::Rlm), 0);
    }

    #[test]
    fn read_coordinator_accepts_full_positive_i64_config_budget_without_instant_overflow() {
        let admission = Arc::new(ProviderWorkerAdmission::new(1));
        let lifecycle = Arc::new(ProviderWorkerLifecycle::new());

        let outcome = execute_provider_read_with_policy(
            Arc::new(StaticReadProvider),
            CodeIntelligenceReadRequest::Definition {
                name: "Post".to_string(),
                module_hint: String::new(),
                limit: 50,
            },
            context(),
            Duration::from_secs(i64::MAX as u64),
            admission,
            Arc::clone(&lifecycle),
            &CancellationToken::new(),
        )
        .expect("a valid configured read budget must not overflow Instant");

        assert!(outcome.ok, "{outcome:?}");
        assert!(lifecycle.drain(Duration::from_secs(1)));
    }

    #[test]
    fn post_receive_arbitration_gives_parent_cancellation_priority_over_ok_result() {
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.linked_child();
        cancellation.cancel();
        let result = Ok(ProviderReadOutcome {
            provider: ProviderId::Rlm,
            ok: true,
            summary: "result published after parent cancellation".to_string(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
            data: None,
        });

        let error = arbitrate_provider_read_result(result, &cancellation, &worker_cancellation)
            .expect_err("parent cancellation must win over a received Ok result");

        assert!(error.starts_with("cancelled:"), "{error}");
    }

    struct PanickingProvider;

    impl CodeIntelligenceProvider for PanickingProvider {
        fn id(&self) -> ProviderId {
            ProviderId::Rlm
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
            panic!("provider fixture panic")
        }
    }

    #[test]
    fn provider_panic_isolated_as_failed_section() {
        let coordinator = CodeSearchCoordinator::new(
            CodeIntelligenceRegistry::new(vec![
                Arc::new(PanickingProvider),
                static_provider(ProviderId::GitGrep, ProviderSectionStatus::Empty, ""),
            ])
            .unwrap(),
        );

        let execution = coordinator
            .search(
                &SearchRequest {
                    query: "Post".to_string(),
                    limit: 20,
                },
                &context(),
                &CancellationToken::new(),
            )
            .unwrap();

        assert!(execution.ok);
        assert_eq!(
            execution.result.sections[0].status,
            ProviderSectionStatus::Failed
        );
        assert!(execution.result.sections[0].diagnostics[0].contains("provider fixture panic"));
        assert_eq!(execution.warnings.len(), 1);
    }

    struct CancellingProvider;

    impl CodeIntelligenceProvider for CancellingProvider {
        fn id(&self) -> ProviderId {
            ProviderId::Rlm
        }

        fn capabilities(&self) -> &[ProviderCapability] {
            &[ProviderCapability::Search]
        }

        fn search(
            &self,
            _request: &SearchRequest,
            _context: &CodeIntelligenceContext,
            _deadline: ProviderDeadline,
            cancellation: &CancellationToken,
        ) -> ProviderSearchSection {
            cancellation.cancel();
            ProviderSearchSection {
                provider: ProviderId::Rlm,
                status: ProviderSectionStatus::Empty,
                hits: Vec::new(),
                diagnostics: Vec::new(),
                artifacts: Vec::new(),
            }
        }
    }

    #[test]
    fn provider_local_cancellation_does_not_cancel_parent_search() {
        let coordinator = CodeSearchCoordinator::new(
            CodeIntelligenceRegistry::new(vec![Arc::new(CancellingProvider)]).unwrap(),
        );

        let execution = coordinator
            .search(
                &SearchRequest {
                    query: "Post".to_string(),
                    limit: 20,
                },
                &context(),
                &CancellationToken::new(),
            )
            .unwrap();

        assert!(execution.ok);
        assert_eq!(
            execution.result.sections[0].status,
            ProviderSectionStatus::Empty
        );
    }
}
