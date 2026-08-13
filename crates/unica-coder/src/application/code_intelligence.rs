use crate::domain::cancellation::{cancelled_error, CancellationToken};
use crate::domain::code_intelligence::{
    CodeIntelligenceContext, CodeIntelligenceProvider, CodeIntelligenceReadRequest,
    CodeIntelligenceRegistry, CodeSearchResult, ProviderDeadline, ProviderIdentity,
    ProviderProgressSink, ProviderProgressUpdate, ProviderReadOutcome, ProviderRole,
    ProviderSearchSection, ProviderSectionStatus, SearchCoverage, SearchProgressSink,
    SearchProgressSnapshot, SearchProviderPhase, SearchProviderProgress, SearchProviderState,
    SearchRequest,
};
use crate::domain::operational_config::CodeIntelligenceDeadlines;
use std::any::Any;
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const MAX_CONCURRENT_WORKERS_PER_PROVIDER: usize = 32;
const SEARCH_PROGRESS_HEARTBEAT: Duration = Duration::from_secs(2);

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

enum ProviderWorkerMessage {
    Progress {
        index: usize,
        update: ProviderProgressUpdate,
    },
    Completed {
        index: usize,
        section: ProviderSearchSection,
    },
}

struct ChannelProviderProgressSink {
    index: usize,
    sender: mpsc::Sender<ProviderWorkerMessage>,
}

impl ProviderProgressSink for ChannelProviderProgressSink {
    fn publish(&self, update: ProviderProgressUpdate) {
        let _ = self.sender.send(ProviderWorkerMessage::Progress {
            index: self.index,
            update,
        });
    }
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

    #[cfg(test)]
    pub(crate) fn search(
        &self,
        request: &SearchRequest,
        context: &CodeIntelligenceContext,
        cancellation: &CancellationToken,
    ) -> Result<CodeSearchExecution, String> {
        self.search_observed(
            request,
            context,
            cancellation,
            &crate::domain::code_intelligence::NoopSearchProgressSink,
        )
    }

    pub(crate) fn search_observed(
        &self,
        request: &SearchRequest,
        context: &CodeIntelligenceContext,
        cancellation: &CancellationToken,
        progress: &dyn SearchProgressSink,
    ) -> Result<CodeSearchExecution, String> {
        self.search_with_progress_interval(
            request,
            context,
            cancellation,
            progress,
            SEARCH_PROGRESS_HEARTBEAT,
        )
    }

    fn search_with_progress_interval(
        &self,
        request: &SearchRequest,
        context: &CodeIntelligenceContext,
        cancellation: &CancellationToken,
        progress: &dyn SearchProgressSink,
        heartbeat_interval: Duration,
    ) -> Result<CodeSearchExecution, String> {
        if cancellation.is_cancelled() {
            return Err(cancelled_error(
                "unica.code.search stopped before providers started",
            ));
        }

        let mut providers = self.registry.search_provider_arcs();
        providers.sort_by_key(|provider| provider_role_order(provider.identity().role));
        let provider_identities = providers
            .iter()
            .map(|provider| provider.identity())
            .collect::<Vec<_>>();
        let started_at = Instant::now();
        let public_search_budget = self.deadlines.search_total_timeout();
        let mut provider_progress = provider_identities
            .iter()
            .map(|provider| SearchProviderProgress {
                identity: provider.clone(),
                state: SearchProviderState::Queued,
                phase: SearchProviderPhase::Preparing,
                detail_code: None,
                results_found: 0,
            })
            .collect::<Vec<_>>();
        publish_search_progress(
            progress,
            started_at,
            public_search_budget,
            heartbeat_interval,
            &provider_progress,
        );
        let (tx, rx) = mpsc::channel();
        let mut slots = vec![None; providers.len()];
        let provider_budgets = provider_identities
            .iter()
            .map(|provider| {
                self.provider_budget(provider.role)
                    .min(public_search_budget)
            })
            .collect::<Vec<_>>();
        let provider_cancellations = provider_identities
            .iter()
            .map(|_| cancellation.linked_child())
            .collect::<Vec<_>>();
        for (index, provider) in providers.into_iter().enumerate() {
            let provider_identity = provider.identity();
            let Some(worker_permit) = self.worker_admission.try_acquire(provider_identity.clone())
            else {
                slots[index] = Some(provider_admission_exhausted_section(
                    provider_identity,
                    self.worker_admission.per_provider_limit,
                ));
                provider_progress[index].state = SearchProviderState::Unavailable;
                continue;
            };
            provider_progress[index].state = SearchProviderState::Running;
            provider_progress[index].phase = SearchProviderPhase::Searching;
            let tx = tx.clone();
            let request = request.clone();
            let context =
                context
                    .clone()
                    .with_provider_progress(Arc::new(ChannelProviderProgressSink {
                        index,
                        sender: tx.clone(),
                    }));
            let budget = provider_budgets[index];
            let worker_cancellation = provider_cancellations[index].clone();
            let worker_identity = provider_identity.clone();
            let worker_provider = Arc::clone(&provider);
            let spawn_result = thread::Builder::new()
                .name(format!(
                    "unica-code-search-{}",
                    provider_identity.role.as_str()
                ))
                .spawn(move || {
                    let _worker_permit = worker_permit;
                    let mut section = catch_unwind(AssertUnwindSafe(|| {
                        worker_provider.search(
                            &request,
                            &context,
                            ProviderDeadline::from_started_at(started_at, budget),
                            &worker_cancellation,
                        )
                    }))
                    .unwrap_or_else(|panic| failed_after_panic(worker_identity.clone(), panic));
                    if section.identity != worker_identity {
                        section.diagnostics.push(format!(
                            "provider returned mismatched identity {}/{}; normalized to {}/{}",
                            section.identity.role.as_str(),
                            section.identity.provider,
                            worker_identity.role.as_str(),
                            worker_identity.provider
                        ));
                        section.identity = worker_identity;
                    }
                    let _ = tx.send(ProviderWorkerMessage::Completed { index, section });
                });
            match spawn_result {
                Ok(handle) => self.worker_lifecycle.track(handle),
                Err(error) => {
                    slots[index] = Some(ProviderSearchSection::failed(
                        provider_identity,
                        format!("failed to start provider worker: {error}"),
                    ));
                    provider_progress[index].state = SearchProviderState::Failed;
                }
            }
        }
        drop(tx);
        publish_search_progress(
            progress,
            started_at,
            public_search_budget,
            heartbeat_interval,
            &provider_progress,
        );
        let mut last_progress_at = Instant::now();

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
            let mut changed = false;
            for (index, slot) in slots.iter_mut().enumerate() {
                if slot.is_none() && elapsed >= provider_budgets[index] {
                    provider_cancellations[index].cancel();
                    *slot = Some(provider_timeout_section(
                        provider_identities[index].clone(),
                        provider_budgets[index],
                    ));
                    provider_progress[index].state = SearchProviderState::TimedOut;
                    changed = true;
                }
            }
            if changed {
                publish_search_progress(
                    progress,
                    started_at,
                    public_search_budget,
                    heartbeat_interval,
                    &provider_progress,
                );
                last_progress_at = Instant::now();
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
            let heartbeat_wait = heartbeat_interval
                .checked_sub(last_progress_at.elapsed())
                .unwrap_or(Duration::ZERO);
            let wait = next_deadline
                .min(heartbeat_wait)
                .min(Duration::from_millis(50));
            match rx.recv_timeout(wait) {
                Ok(ProviderWorkerMessage::Progress { index, update }) if slots[index].is_none() => {
                    provider_progress[index].phase = update.phase;
                    provider_progress[index].detail_code = update.detail_code;
                    provider_progress[index].results_found = update.results_found;
                    publish_search_progress(
                        progress,
                        started_at,
                        public_search_budget,
                        heartbeat_interval,
                        &provider_progress,
                    );
                    last_progress_at = Instant::now();
                }
                Ok(ProviderWorkerMessage::Completed { index, section })
                    if slots[index].is_none() =>
                {
                    provider_progress[index].state = progress_state_for_section(&section);
                    provider_progress[index].results_found = section.hits.len();
                    slots[index] = Some(section);
                    publish_search_progress(
                        progress,
                        started_at,
                        public_search_budget,
                        heartbeat_interval,
                        &provider_progress,
                    );
                    last_progress_at = Instant::now();
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if last_progress_at.elapsed() >= heartbeat_interval {
                publish_search_progress(
                    progress,
                    started_at,
                    public_search_budget,
                    heartbeat_interval,
                    &provider_progress,
                );
                last_progress_at = Instant::now();
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
            .zip(provider_identities)
            .map(|(section, provider)| {
                section.unwrap_or_else(|| {
                    ProviderSearchSection::failed(
                        provider,
                        "provider worker ended without a result".to_string(),
                    )
                })
            })
            .collect::<Vec<_>>();
        for (index, section) in sections.iter().enumerate() {
            provider_progress[index].state = progress_state_for_section(section);
            provider_progress[index].results_found = section.hits.len();
        }
        publish_search_progress(
            progress,
            started_at,
            public_search_budget,
            heartbeat_interval,
            &provider_progress,
        );
        self.worker_lifecycle.reap();
        let ok = sections.iter().any(|section| {
            matches!(
                section.status,
                ProviderSectionStatus::Ok | ProviderSectionStatus::Empty
            ) || matches!(
                section.status,
                ProviderSectionStatus::LimitReached | ProviderSectionStatus::TimedOut
            ) && !section.hits.is_empty()
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
        let coverage =
            if !sections.is_empty() && sections.iter().all(|section| section.search_complete) {
                SearchCoverage::Complete
            } else if sections.iter().any(|section| {
                section.search_complete
                    || matches!(
                        section.status,
                        ProviderSectionStatus::LimitReached | ProviderSectionStatus::TimedOut
                    ) && !section.hits.is_empty()
            }) {
                SearchCoverage::Partial
            } else {
                SearchCoverage::None
            };
        let result = CodeSearchResult {
            coverage,
            elapsed_ms: u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
            sections,
        };

        Ok(CodeSearchExecution {
            ok,
            result,
            warnings,
            errors,
        })
    }

    fn provider_budget(&self, role: ProviderRole) -> Duration {
        self.deadlines.search_timeout_for(role)
    }
}

fn provider_role_order(role: ProviderRole) -> usize {
    ProviderRole::ALL
        .iter()
        .position(|candidate| *candidate == role)
        .unwrap_or(ProviderRole::ALL.len())
}

fn progress_state_for_section(section: &ProviderSearchSection) -> SearchProviderState {
    match section.status {
        ProviderSectionStatus::Ok
        | ProviderSectionStatus::Empty
        | ProviderSectionStatus::LimitReached => SearchProviderState::Completed,
        ProviderSectionStatus::TimedOut => SearchProviderState::TimedOut,
        ProviderSectionStatus::Unavailable => SearchProviderState::Unavailable,
        ProviderSectionStatus::Failed => SearchProviderState::Failed,
    }
}

fn publish_search_progress(
    sink: &dyn SearchProgressSink,
    started_at: Instant,
    deadline: Duration,
    heartbeat_interval: Duration,
    providers: &[SearchProviderProgress],
) {
    let snapshot = SearchProgressSnapshot {
        schema_version: 1,
        elapsed_ms: duration_millis(started_at.elapsed()),
        deadline_ms: duration_millis(deadline),
        next_update_within_ms: duration_millis(heartbeat_interval),
        providers: providers.to_vec(),
    };
    let _ = catch_unwind(AssertUnwindSafe(|| sink.publish(snapshot)));
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

struct ProviderWorkerAdmission {
    per_provider_limit: usize,
    active: Mutex<HashMap<ProviderIdentity, usize>>,
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

impl ProviderWorkerAdmission {
    fn new(per_provider_limit: usize) -> Self {
        Self {
            per_provider_limit: per_provider_limit.max(1),
            active: Mutex::new(HashMap::new()),
        }
    }

    fn try_acquire(self: &Arc<Self>, provider: ProviderIdentity) -> Option<ProviderWorkerPermit> {
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let count = active.entry(provider.clone()).or_default();
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
    fn active_count(&self, provider: &ProviderIdentity) -> usize {
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(provider)
            .copied()
            .unwrap_or_default()
    }
}

struct ProviderWorkerPermit {
    admission: Arc<ProviderWorkerAdmission>,
    provider: ProviderIdentity,
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
    let provider_identity = provider.identity();
    let _permit = worker_admission
        .try_acquire(provider_identity.clone())
        .ok_or_else(|| {
            format!(
                "{} read provider worker capacity exhausted (limit {})",
                provider_identity.provider, worker_admission.per_provider_limit
            )
        })?;
    let worker_cancellation = cancellation.linked_child();
    let worker_token = worker_cancellation.clone();
    let started_at = Instant::now();
    let (tx, rx) = mpsc::sync_channel(1);
    let worker_identity = provider_identity.clone();
    let handle = thread::Builder::new()
        .name(format!(
            "unica-code-read-{}",
            provider_identity.role.as_str()
        ))
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
                format!(
                    "{} read provider panicked: {detail}",
                    worker_identity.provider
                )
            })
            .and_then(|result| result);
            let _ = tx.send(result);
        })
        .map_err(|error| {
            format!(
                "failed to start {} read provider worker: {error}",
                provider_identity.provider
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
                provider_identity.provider,
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
                    provider_identity.provider
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

fn provider_timeout_section(provider: ProviderIdentity, budget: Duration) -> ProviderSearchSection {
    ProviderSearchSection::timed_out(
        provider,
        crate::domain::code_intelligence::SearchRanking::None,
        crate::domain::code_intelligence::SearchOrdering::Provider,
        Vec::new(),
        vec![format!(
            "provider exceeded its {} ms search budget",
            budget.as_millis()
        )],
    )
    .expect("timeout section is valid")
}

fn provider_admission_exhausted_section(
    provider: ProviderIdentity,
    limit: usize,
) -> ProviderSearchSection {
    ProviderSearchSection::unavailable(
        provider,
        format!("provider worker capacity exhausted (limit {limit})"),
    )
}

fn failed_after_panic(
    provider: ProviderIdentity,
    panic: Box<dyn Any + Send>,
) -> ProviderSearchSection {
    let detail = panic
        .downcast_ref::<&str>()
        .map(|value| (*value).to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic payload".to_string());
    ProviderSearchSection::failed(provider, format!("provider panicked: {detail}"))
}

fn section_problem(section: &ProviderSearchSection) -> String {
    let detail = if section.diagnostics.is_empty() {
        section.status.as_str().to_string()
    } else {
        section.diagnostics.join("; ")
    };
    format!("{}: {detail}", section.identity.provider)
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
        SearchOrdering, SearchProgressSink, SearchProgressSnapshot, SearchProviderState,
        SearchRanking, SearchRequest,
    };
    use crate::domain::operational_config::CodeIntelligenceDeadlines;
    use crate::domain::source_location::SourceLocation;
    use crate::domain::source_roots::ResolvedSourceRoot;
    use crate::domain::workspace::WorkspaceContext;
    use serde_json::Map;
    use std::path::PathBuf;
    use std::sync::{mpsc, Arc, Barrier, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    struct GateProvider {
        id: ProviderId,
        started: mpsc::Sender<ProviderId>,
        release: Arc<Barrier>,
        status: ProviderSectionStatus,
    }

    impl CodeIntelligenceProvider for GateProvider {
        fn identity(&self) -> crate::domain::code_intelligence::ProviderIdentity {
            self.id.identity()
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
            test_section(self.id, self.status, Vec::new(), "")
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

    fn test_section(
        id: ProviderId,
        status: ProviderSectionStatus,
        hits: Vec<ProviderSearchHit>,
        diagnostic: &str,
    ) -> ProviderSearchSection {
        let diagnostics = (!diagnostic.is_empty())
            .then(|| diagnostic.to_string())
            .into_iter()
            .collect::<Vec<_>>();
        match status {
            ProviderSectionStatus::Ok | ProviderSectionStatus::Empty => {
                ProviderSearchSection::complete(
                    id.identity(),
                    SearchRanking::Provider,
                    SearchOrdering::Provider,
                    hits,
                    diagnostics,
                )
                .unwrap()
            }
            ProviderSectionStatus::LimitReached => ProviderSearchSection::limit_reached(
                id.identity(),
                SearchRanking::Provider,
                SearchOrdering::Provider,
                hits,
                diagnostics,
            )
            .unwrap(),
            ProviderSectionStatus::TimedOut => ProviderSearchSection::timed_out(
                id.identity(),
                SearchRanking::Provider,
                SearchOrdering::Provider,
                hits,
                diagnostics,
            )
            .unwrap(),
            ProviderSectionStatus::Unavailable => {
                ProviderSearchSection::unavailable(id.identity(), diagnostic.to_string())
            }
            ProviderSectionStatus::Failed => {
                ProviderSearchSection::failed(id.identity(), diagnostic.to_string())
            }
        }
    }

    fn test_hit(index: usize) -> ProviderSearchHit {
        ProviderSearchHit {
            rank: Some(index + 1),
            provider_score: None,
            location: SourceLocation::Unaddressable {
                source_set: "main".to_string(),
                owner_metadata_path: None,
                path: format!("Module{index}.bsl"),
            },
            line: index + 1,
            end_line: None,
            symbol: None,
            kind: None,
            snippet: format!("hit {index}"),
            attributes: Map::new(),
        }
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
                .map(|section| section.identity.role)
                .collect::<Vec<_>>(),
            vec![
                ProviderId::Rlm.role(),
                ProviderId::BslAnalyzer.role(),
                ProviderId::GitGrep.role()
            ]
        );
    }

    struct StaticProvider {
        section: ProviderSearchSection,
    }

    #[derive(Default)]
    struct RecordingProgressSink(Mutex<Vec<SearchProgressSnapshot>>);

    impl SearchProgressSink for RecordingProgressSink {
        fn publish(&self, snapshot: SearchProgressSnapshot) {
            self.0.lock().unwrap().push(snapshot);
        }
    }

    struct SlowProvider {
        id: ProviderId,
        delay: Duration,
    }

    impl CodeIntelligenceProvider for SlowProvider {
        fn identity(&self) -> crate::domain::code_intelligence::ProviderIdentity {
            self.id.identity()
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
            thread::sleep(self.delay);
            test_section(self.id, ProviderSectionStatus::Empty, Vec::new(), "")
        }
    }

    #[test]
    fn coordinator_publishes_start_heartbeat_and_terminal_role_state() {
        let sink = RecordingProgressSink::default();
        let coordinator = CodeSearchCoordinator::new(
            CodeIntelligenceRegistry::new(vec![Arc::new(SlowProvider {
                id: ProviderId::Rlm,
                delay: Duration::from_millis(80),
            })])
            .unwrap(),
        );

        coordinator
            .search_with_progress_interval(
                &SearchRequest {
                    query: "Post".to_string(),
                    limit: 20,
                },
                &context(),
                &CancellationToken::new(),
                &sink,
                Duration::from_millis(20),
            )
            .unwrap();

        let snapshots = sink.0.lock().unwrap();
        assert!(snapshots.len() >= 4, "snapshots: {snapshots:?}");
        assert!(snapshots
            .iter()
            .any(|snapshot| snapshot.providers[0].state == SearchProviderState::Running));
        assert_eq!(
            snapshots.last().unwrap().providers[0].state,
            SearchProviderState::Completed
        );
        assert_eq!(snapshots.last().unwrap().terminal_roles(), 1);
    }

    impl CodeIntelligenceProvider for StaticProvider {
        fn identity(&self) -> crate::domain::code_intelligence::ProviderIdentity {
            self.section.identity.clone()
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
            section: test_section(id, status, Vec::new(), diagnostic),
        })
    }

    #[test]
    fn coordinator_preserves_the_replaceable_provider_identity_for_a_role() {
        let provider = Arc::new(StaticProvider {
            section: ProviderSearchSection::complete(
                crate::domain::code_intelligence::ProviderIdentity::new(
                    crate::domain::code_intelligence::ProviderRole::Semantic,
                    "replacement-semantic",
                ),
                SearchRanking::Provider,
                SearchOrdering::Provider,
                Vec::new(),
                Vec::new(),
            )
            .unwrap(),
        });

        let execution =
            CodeSearchCoordinator::new(CodeIntelligenceRegistry::new(vec![provider]).unwrap())
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
            execution.result.sections[0].identity.provider,
            "replacement-semantic"
        );
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
    fn coordinator_preserves_provider_local_ranks_and_limit_claim() {
        let hits = (0..2).map(test_hit).collect();
        let provider = Arc::new(StaticProvider {
            section: ProviderSearchSection::limit_reached(
                ProviderId::GitGrep.identity(),
                SearchRanking::Provider,
                SearchOrdering::Provider,
                hits,
                Vec::new(),
            )
            .unwrap(),
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
            vec![Some(1), Some(2)]
        );
        assert_eq!(
            execution.result.sections[0].status,
            ProviderSectionStatus::LimitReached
        );
    }

    struct BudgetProvider {
        id: ProviderId,
        maximum: Duration,
    }

    impl CodeIntelligenceProvider for BudgetProvider {
        fn identity(&self) -> crate::domain::code_intelligence::ProviderIdentity {
            self.id.identity()
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
            test_section(self.id, ProviderSectionStatus::Empty, Vec::new(), "")
        }
    }

    #[test]
    fn coordinator_applies_provider_budgets_and_renders_from_canonical_sections() {
        let providers = [
            (ProviderId::Rlm, Duration::from_secs(300)),
            (ProviderId::BslAnalyzer, Duration::from_secs(300)),
            (ProviderId::GitGrep, Duration::from_secs(2)),
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
                .map(|section| (section.identity.provider.as_str(), section.hits.len()))
                .collect::<Vec<_>>(),
            vec![("rlm", 0), ("bsl-analyzer", 0), ("git-grep", 0)]
        );
        assert_eq!(
            execution
                .result
                .sections
                .iter()
                .map(|section| section.identity.role)
                .collect::<Vec<_>>(),
            vec![
                ProviderId::Rlm.role(),
                ProviderId::BslAnalyzer.role(),
                ProviderId::GitGrep.role()
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
            coordinator.provider_budget(ProviderId::BslAnalyzer.role()),
            Duration::from_secs(100)
        );
        assert_eq!(
            coordinator.provider_budget(ProviderId::Rlm.role()),
            Duration::from_secs(30)
        );
        assert_eq!(
            coordinator.provider_budget(ProviderId::GitGrep.role()),
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

    struct DeadlineIgnoringProvider;

    impl CodeIntelligenceProvider for DeadlineIgnoringProvider {
        fn identity(&self) -> crate::domain::code_intelligence::ProviderIdentity {
            ProviderId::BslAnalyzer.identity()
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
            test_section(
                ProviderId::BslAnalyzer,
                ProviderSectionStatus::Empty,
                Vec::new(),
                "",
            )
        }
    }

    #[test]
    fn coordinator_enforces_budget_when_provider_ignores_deadline_and_cancellation() {
        let admission = Arc::new(ProviderWorkerAdmission::new(1));
        let lifecycle = Arc::new(ProviderWorkerLifecycle::new());
        let coordinator = CodeSearchCoordinator::with_policy(
            CodeIntelligenceRegistry::new(vec![
                Arc::new(DeadlineIgnoringProvider),
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
            ProviderSectionStatus::TimedOut
        );
        assert!(execution.result.sections[0].diagnostics[0].contains("30 ms search budget"));

        assert_eq!(
            admission.active_count(&ProviderId::BslAnalyzer.identity()),
            1
        );
        assert_eq!(lifecycle.pending_count(), 1);
        let second = CodeSearchCoordinator::with_policy(
            CodeIntelligenceRegistry::new(vec![
                Arc::new(DeadlineIgnoringProvider),
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
        assert_eq!(
            admission.active_count(&ProviderId::BslAnalyzer.identity()),
            0
        );
    }

    struct StaticReadProvider;

    impl CodeIntelligenceProvider for StaticReadProvider {
        fn identity(&self) -> crate::domain::code_intelligence::ProviderIdentity {
            ProviderId::Rlm.identity()
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
                provider: ProviderId::Rlm.identity(),
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
        fn identity(&self) -> crate::domain::code_intelligence::ProviderIdentity {
            ProviderId::Rlm.identity()
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
                provider: ProviderId::Rlm.identity(),
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
        assert_eq!(admission.active_count(&ProviderId::Rlm.identity()), 1);
        assert!(lifecycle.drain(Duration::from_secs(2)));
        assert_eq!(admission.active_count(&ProviderId::Rlm.identity()), 0);
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
            provider: ProviderId::Rlm.identity(),
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
        fn identity(&self) -> crate::domain::code_intelligence::ProviderIdentity {
            ProviderId::Rlm.identity()
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
        fn identity(&self) -> crate::domain::code_intelligence::ProviderIdentity {
            ProviderId::Rlm.identity()
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
            test_section(
                ProviderId::Rlm,
                ProviderSectionStatus::Empty,
                Vec::new(),
                "",
            )
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
