use super::ports::{ApplicationPorts, HandlerOutcome};
use super::AdapterOutcome;
use crate::domain::cancellation::CancellationToken;
use crate::domain::diagnostics::*;
use crate::domain::operational_config::OperationalConfig;
use crate::domain::source_location::SourceLocation;
use crate::domain::source_target::{MetadataAddress, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20};
use crate::domain::workspace::WorkspaceContext;
use serde_json::{Map, Value};
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const MAX_CONCURRENT_DIAGNOSTIC_WORKERS_PER_PROVIDER: usize = 32;

pub(crate) fn invoke(
    ports: &dyn ApplicationPorts,
    args: &Map<String, Value>,
    workspace: &WorkspaceContext,
    operational_config: Option<&OperationalConfig>,
    cancellation: &CancellationToken,
) -> Result<HandlerOutcome, String> {
    let mut request = parse_diagnostic_request(args)
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    if request.action == DiagnosticAction::Analyze {
        request.timeout = Some(
            operational_config
                .ok_or_else(|| {
                    "diagnostics analyze call is missing operational config".to_string()
                })?
                .code_diagnostics()
                .analyze_timeout(),
        );
    }
    let result = DiagnosticCoordinator::new(ports.diagnostic_provider_registry()?, ports)
        .execute(&request, workspace, cancellation)
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    let ok = result.ok;
    let summary = match result.state {
        DiagnosticResultState::Completed => {
            "unica.code.diagnostics completed through provider-neutral diagnostics"
        }
        DiagnosticResultState::Partial => {
            "unica.code.diagnostics returned a partial provider-neutral result"
        }
        DiagnosticResultState::Failed => {
            "unica.code.diagnostics failed because no provider produced a useful result"
        }
    }
    .to_string();
    let warnings = (result.state == DiagnosticResultState::Partial)
        .then(|| "one or more diagnostic providers returned an incomplete result".to_string())
        .into_iter()
        .collect();
    let errors = (!ok)
        .then(|| "diagnostics_failed: no provider produced a useful result".to_string())
        .into_iter()
        .collect();
    let data = serde_json::to_value(result)
        .map_err(|error| format!("failed to serialize diagnostic result: {error}"))?;
    Ok(HandlerOutcome::with_data(
        AdapterOutcome {
            ok,
            summary,
            changes: Vec::new(),
            warnings,
            errors,
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
            command: None,
        },
        data,
    ))
}

pub(crate) trait DiagnosticMapping: Send + Sync {
    fn resolve_context(
        &self,
        request: &DiagnosticRequest,
        workspace: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<DiagnosticContext, DiagnosticRequestError>;

    fn map_observation(
        &self,
        observation: DiagnosticObservation,
        context: &DiagnosticContext,
        cancellation: &CancellationToken,
    ) -> Result<DiagnosticItem, DiagnosticMapError>;

    fn map_observations(
        &self,
        observations: Vec<DiagnosticObservation>,
        context: &DiagnosticContext,
        cancellation: &CancellationToken,
    ) -> Result<Vec<DiagnosticItem>, DiagnosticMapError> {
        observations
            .into_iter()
            .map(|observation| self.map_observation(observation, context, cancellation))
            .collect()
    }
}

impl<T: ApplicationPorts + ?Sized> DiagnosticMapping for T {
    fn resolve_context(
        &self,
        request: &DiagnosticRequest,
        workspace: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<DiagnosticContext, DiagnosticRequestError> {
        ApplicationPorts::resolve_diagnostic_context(self, request, workspace, cancellation)
    }

    fn map_observation(
        &self,
        observation: DiagnosticObservation,
        context: &DiagnosticContext,
        cancellation: &CancellationToken,
    ) -> Result<DiagnosticItem, DiagnosticMapError> {
        ApplicationPorts::map_diagnostic_observation(self, observation, context, cancellation)
    }

    fn map_observations(
        &self,
        observations: Vec<DiagnosticObservation>,
        context: &DiagnosticContext,
        cancellation: &CancellationToken,
    ) -> Result<Vec<DiagnosticItem>, DiagnosticMapError> {
        ApplicationPorts::map_diagnostic_observations(self, observations, context, cancellation)
    }
}

pub(crate) struct DiagnosticCoordinator<'a, M: DiagnosticMapping + ?Sized> {
    registry: DiagnosticProviderRegistry,
    mapping: &'a M,
}

impl<'a, M: DiagnosticMapping + ?Sized> DiagnosticCoordinator<'a, M> {
    pub(crate) fn new(registry: DiagnosticProviderRegistry, mapping: &'a M) -> Self {
        Self { registry, mapping }
    }

    pub(crate) fn execute(
        &self,
        request: &DiagnosticRequest,
        workspace: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<DiagnosticResult, DiagnosticRequestError> {
        if cancellation.is_cancelled() {
            return Err(cancelled_request_error(
                "diagnostics stopped before providers started",
            ));
        }
        let context = self
            .mapping
            .resolve_context(request, workspace, cancellation)?;
        if request.range.is_some() && context.target.target_kind != TargetKind::Module {
            return Err(request_error(
                "target_kind_mismatch",
                Some("range"),
                "range is supported only for a module findings target",
            ));
        }
        let selected = select_providers(&self.registry, request, context.target.target_kind)?;
        let selection = selection(request, &context, &selected);
        let provider_order = selected
            .iter()
            .enumerate()
            .map(|(index, selected)| (selected.descriptor.id.as_str(), index))
            .collect::<HashMap<_, _>>();
        let provider_request = DiagnosticProviderRequest {
            action: request.action,
            source_set: request.source_set.clone(),
            metadata_path: request.metadata_path.clone(),
            target_kind: context.target.target_kind,
            filter: request.filter.clone(),
            range: request.range,
        };
        let executions = execute_selected_providers(
            &selected,
            &provider_request,
            &context,
            request.timeout.unwrap_or(Duration::from_secs(120)),
            cancellation,
        )?;
        let mut sections = Vec::with_capacity(selected.len());
        let mut all_items = Vec::new();
        for (selected_provider, execution) in selected.iter().zip(executions) {
            let ProviderExecution::Outcome(mut outcome) = execution else {
                sections.push(unsupported_section(
                    selected_provider.descriptor,
                    request.action,
                ));
                continue;
            };
            sanitize_provider_outcome(&mut outcome, &context);
            let mut provider_items =
                match self
                    .mapping
                    .map_observations(outcome.observations, &context, cancellation)
                {
                    Ok(items) => items,
                    Err(error) => {
                        sections.push(failed_mapping_section(
                            selected_provider.descriptor,
                            outcome.version,
                            error,
                            request.action,
                        ));
                        continue;
                    }
                };
            if request.action == DiagnosticAction::Catalog {
                provider_items.extend(outcome.rules.into_iter().map(|rule| {
                    DiagnosticItem::DiagnosticRule {
                        provider: rule.provider.as_str(),
                        code: rule.code,
                        default_severity: rule.default_severity,
                        title: rule.title,
                        description: rule.description,
                        tags: rule.tags,
                    }
                }));
            }
            provider_items.retain(|item| item_matches_request(item, request, &context));
            let resource_failures = provider_items
                .iter()
                .filter(|item| matches!(item, DiagnosticItem::ResourceFailure { .. }))
                .count();
            let items_total = provider_items.len();
            all_items.extend(provider_items);
            sections.push(DiagnosticProviderSection {
                id: selected_provider.descriptor.id.as_str(),
                status: outcome.status,
                complete: outcome.complete,
                version: outcome.version,
                capabilities: (request.action == DiagnosticAction::Catalog)
                    .then(|| selected_provider.descriptor.into()),
                readiness: outcome.readiness,
                items_total: action_has_items(request.action).then_some(items_total),
                items_returned: action_has_items(request.action).then_some(0),
                resource_failures: action_has_observations(request.action)
                    .then_some(resource_failures),
                truncated: action_has_items(request.action).then_some(false),
                error: outcome.error,
            });
        }

        if cancellation.is_cancelled() {
            return Err(cancelled_request_error(
                "diagnostics stopped before result assembly",
            ));
        }

        all_items.sort_by_cached_key(|item| diagnostic_sort_key(item, &provider_order));
        let items_total = all_items.len();
        all_items.truncate(request.limit);
        let items_returned = all_items.len();
        let truncated = items_returned < items_total;
        for section in &mut sections {
            if action_has_items(request.action) {
                let returned = all_items
                    .iter()
                    .filter(|item| item_provider(item) == section.id)
                    .count();
                section.items_returned = Some(returned);
                section.truncated = Some(returned < section.items_total.unwrap_or(0));
            }
        }
        let any_success = sections.iter().any(|section| {
            matches!(
                section.status,
                DiagnosticProviderStatus::Completed | DiagnosticProviderStatus::Empty
            )
        });
        let all_complete = !sections.is_empty()
            && sections.iter().all(|section| {
                matches!(
                    section.status,
                    DiagnosticProviderStatus::Completed | DiagnosticProviderStatus::Empty
                ) && section.complete
            });
        let (ok, state, complete) = if all_complete {
            (true, DiagnosticResultState::Completed, true)
        } else if any_success {
            (true, DiagnosticResultState::Partial, false)
        } else {
            (false, DiagnosticResultState::Failed, false)
        };
        Ok(DiagnosticResult {
            ok,
            action: request.action,
            selection,
            state,
            complete,
            providers: sections,
            items_total: action_has_items(request.action).then_some(items_total),
            items_returned: action_has_items(request.action).then_some(items_returned),
            truncated: action_has_items(request.action).then_some(truncated),
            items: all_items,
        })
    }
}

struct SelectedProvider {
    descriptor: &'static DiagnosticProviderDescriptor,
    provider: Arc<dyn DiagnosticProvider>,
    applicable: bool,
}

enum ProviderExecution {
    Unsupported,
    Outcome(DiagnosticProviderOutcome),
}

fn execute_selected_providers(
    selected: &[SelectedProvider],
    request: &DiagnosticProviderRequest,
    context: &DiagnosticContext,
    total_budget: Duration,
    cancellation: &CancellationToken,
) -> Result<Vec<ProviderExecution>, DiagnosticRequestError> {
    let started_at = Instant::now();
    let (sender, receiver) = mpsc::channel();
    let mut slots = (0..selected.len())
        .map(|_| None)
        .collect::<Vec<Option<ProviderExecution>>>();
    let child_cancellations = selected
        .iter()
        .map(|provider| provider.applicable.then(|| cancellation.linked_child()))
        .collect::<Vec<_>>();
    let admission = diagnostic_worker_admission();
    let lifecycle = diagnostic_worker_lifecycle();

    for (index, selected_provider) in selected.iter().enumerate() {
        if !selected_provider.applicable {
            slots[index] = Some(ProviderExecution::Unsupported);
            continue;
        }
        let provider_id = selected_provider.descriptor.id;
        let Some(permit) = admission.try_acquire(provider_id) else {
            slots[index] = Some(ProviderExecution::Outcome(provider_failure_outcome(
                "provider_busy",
                "diagnostic provider worker capacity is exhausted",
                true,
            )));
            continue;
        };
        let provider = Arc::clone(&selected_provider.provider);
        let sender = sender.clone();
        let request = request.clone();
        let context = context.clone();
        let worker_cancellation = child_cancellations[index]
            .as_ref()
            .expect("applicable provider has a child cancellation token")
            .clone();
        let spawn = thread::Builder::new()
            .name(format!("unica-diagnostics-{}", provider_id.as_str()))
            .spawn(move || {
                let _permit = permit;
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    provider.execute(
                        &request,
                        &context,
                        ProviderDeadline::from_started_at(started_at, total_budget),
                        &worker_cancellation,
                    )
                }))
                .map(|outcome| normalize_provider_outcome(provider_id, request.action, outcome))
                .unwrap_or_else(|panic| provider_panic_outcome(provider_id, panic));
                let _ = sender.send((index, outcome));
            });
        match spawn {
            Ok(handle) => lifecycle.track(handle),
            Err(_) => {
                slots[index] = Some(ProviderExecution::Outcome(provider_failure_outcome(
                    "provider_start_failed",
                    "diagnostic provider worker could not be started",
                    true,
                )));
            }
        }
    }
    drop(sender);

    while slots.iter().any(Option::is_none) {
        if cancellation.is_cancelled() {
            cancel_diagnostic_children(&child_cancellations);
            return Err(cancelled_request_error(
                "diagnostics stopped while providers were running",
            ));
        }
        let remaining = total_budget
            .checked_sub(started_at.elapsed())
            .unwrap_or(Duration::ZERO);
        if remaining.is_zero() {
            for (index, slot) in slots.iter_mut().enumerate() {
                if slot.is_none() {
                    if let Some(token) = &child_cancellations[index] {
                        token.cancel();
                    }
                    *slot = Some(ProviderExecution::Outcome(provider_failure_outcome(
                        "provider_timeout",
                        "diagnostic provider exceeded the invocation deadline",
                        true,
                    )));
                }
            }
            break;
        }
        match receiver.recv_timeout(remaining.min(Duration::from_millis(20))) {
            Ok((index, outcome)) if slots[index].is_none() => {
                slots[index] = Some(ProviderExecution::Outcome(outcome));
            }
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    if cancellation.is_cancelled() {
        cancel_diagnostic_children(&child_cancellations);
        return Err(cancelled_request_error(
            "diagnostics stopped while providers were completing",
        ));
    }
    cancel_diagnostic_children(&child_cancellations);
    lifecycle.reap();
    Ok(slots
        .into_iter()
        .zip(selected)
        .map(|(execution, provider)| {
            execution.unwrap_or_else(|| {
                ProviderExecution::Outcome(provider_failure_outcome(
                    "provider_ended_without_result",
                    format!(
                        "diagnostic provider {} ended without a result",
                        provider.descriptor.id.as_str()
                    ),
                    true,
                ))
            })
        })
        .collect())
}

fn cancel_diagnostic_children(children: &[Option<CancellationToken>]) {
    for child in children.iter().flatten() {
        child.cancel();
    }
}

fn normalize_provider_outcome(
    provider_id: DiagnosticProviderId,
    action: DiagnosticAction,
    mut outcome: DiagnosticProviderOutcome,
) -> DiagnosticProviderOutcome {
    for observation in &mut outcome.observations {
        match observation {
            DiagnosticObservation::Diagnostic { provider, .. }
            | DiagnosticObservation::ResourceFailure { provider, .. } => *provider = provider_id,
        }
    }
    for rule in &mut outcome.rules {
        rule.provider = provider_id;
    }
    match outcome.status {
        DiagnosticProviderStatus::Completed => {
            if outcome.error.is_some() {
                return provider_contract_failure(
                    outcome.version,
                    "completed provider outcome included an error",
                );
            }
        }
        DiagnosticProviderStatus::Empty => {
            if !outcome.complete
                || !outcome.observations.is_empty()
                || !outcome.rules.is_empty()
                || outcome.readiness.is_some()
                || outcome.error.is_some()
            {
                return provider_contract_failure(
                    outcome.version,
                    "empty provider outcome was not complete and payload-free",
                );
            }
        }
        DiagnosticProviderStatus::Failed
        | DiagnosticProviderStatus::Unavailable
        | DiagnosticProviderStatus::Unsupported => {
            if outcome.complete
                || !outcome.observations.is_empty()
                || !outcome.rules.is_empty()
                || outcome.readiness.is_some()
                || outcome.error.is_none()
            {
                return provider_contract_failure(
                    outcome.version,
                    "non-success provider outcome was not incomplete, payload-free, and typed",
                );
            }
        }
    }
    match action {
        DiagnosticAction::Status => {
            if !outcome.observations.is_empty() || !outcome.rules.is_empty() {
                return provider_contract_failure(
                    outcome.version,
                    "status provider returned findings or rules",
                );
            }
            if matches!(
                outcome.status,
                DiagnosticProviderStatus::Completed | DiagnosticProviderStatus::Empty
            ) && (outcome.status != DiagnosticProviderStatus::Completed
                || !outcome.complete
                || outcome.readiness.is_none())
            {
                return provider_contract_failure(
                    outcome.version,
                    "completed status provider omitted complete readiness evidence",
                );
            }
        }
        DiagnosticAction::Catalog => {
            if !outcome.observations.is_empty() || outcome.readiness.is_some() {
                return provider_contract_failure(
                    outcome.version,
                    "catalog provider returned findings or readiness",
                );
            }
        }
        DiagnosticAction::Analyze => {
            if !outcome.rules.is_empty() || outcome.readiness.is_some() {
                return provider_contract_failure(
                    outcome.version,
                    "analyze provider returned rules or readiness",
                );
            }
        }
        DiagnosticAction::Findings => {
            if !outcome.rules.is_empty() {
                return provider_contract_failure(
                    outcome.version,
                    "findings provider returned catalog rules",
                );
            }
            if let Some(readiness) = outcome.readiness.take() {
                if readiness.state != DiagnosticReadinessState::Ready {
                    return DiagnosticProviderOutcome {
                        status: DiagnosticProviderStatus::Unavailable,
                        complete: false,
                        version: outcome.version,
                        observations: Vec::new(),
                        rules: Vec::new(),
                        readiness: None,
                        error: Some(DiagnosticError {
                            code: "provider_not_ready".to_string(),
                            message: "diagnostic provider findings are not ready".to_string(),
                            retryable: true,
                        }),
                    };
                }
                return provider_contract_failure(
                    outcome.version,
                    "findings provider returned readiness instead of findings",
                );
            }
        }
    }
    outcome
}

fn provider_contract_failure(
    version: Option<String>,
    message: impl Into<String>,
) -> DiagnosticProviderOutcome {
    let mut outcome = provider_failure_outcome("provider_contract_invalid", message, false);
    outcome.version = version;
    outcome
}

fn provider_failure_outcome(
    code: &str,
    message: impl Into<String>,
    retryable: bool,
) -> DiagnosticProviderOutcome {
    DiagnosticProviderOutcome {
        status: DiagnosticProviderStatus::Failed,
        complete: false,
        version: None,
        observations: Vec::new(),
        rules: Vec::new(),
        readiness: None,
        error: Some(DiagnosticError {
            code: code.to_string(),
            message: message.into(),
            retryable,
        }),
    }
}

fn provider_panic_outcome(
    _provider_id: DiagnosticProviderId,
    _panic: Box<dyn Any + Send>,
) -> DiagnosticProviderOutcome {
    provider_failure_outcome(
        "provider_panicked",
        "diagnostic provider terminated unexpectedly",
        false,
    )
}

fn sanitize_provider_outcome(outcome: &mut DiagnosticProviderOutcome, context: &DiagnosticContext) {
    outcome.version = outcome
        .version
        .take()
        .map(|version| redact_public_physical_paths(&version, context));
    if let Some(error) = &mut outcome.error {
        sanitize_public_diagnostic_error(error);
    }
    for observation in &mut outcome.observations {
        match observation {
            DiagnosticObservation::Diagnostic { code, message, .. } => {
                *code = redact_public_physical_paths(code, context);
                *message = redact_public_physical_paths(message, context);
            }
            DiagnosticObservation::ResourceFailure { error, .. } => {
                sanitize_public_diagnostic_error(error);
            }
        }
    }
    for rule in &mut outcome.rules {
        rule.code = redact_public_physical_paths(&rule.code, context);
        rule.title = redact_public_physical_paths(&rule.title, context);
        rule.description = rule
            .description
            .take()
            .map(|description| redact_public_physical_paths(&description, context));
    }
}

fn sanitize_public_diagnostic_error(error: &mut DiagnosticError) {
    error.message = match error.code.as_str() {
        "source_analysis_failed" => "diagnostic provider could not analyze the selected resource",
        "source_decode_failed" => "source is not valid in the detected encoding",
        "provider_not_ready" => "diagnostic provider is not ready",
        "target_not_supported" => "diagnostic provider does not support the selected target",
        "action_not_supported" => "diagnostic provider does not support the selected action",
        "location_outside_source_set" => "provider resource is outside the selected sourceSet",
        "provider_contract_invalid" => "diagnostic provider returned an invalid result",
        "provider_timeout" => "diagnostic provider deadline exceeded",
        "provider_panicked" => "diagnostic provider terminated unexpectedly",
        "provider_start_failed" | "provider_unavailable" => "diagnostic provider is unavailable",
        "diagnostics_invalid" => "diagnostic provider returned an invalid diagnostics stream",
        "diagnostics_incomplete" => "diagnostic provider returned an incomplete diagnostics stream",
        "diagnostics_pending" => "diagnostic provider has not completed diagnostics",
        "cancelled" => "diagnostic provider request was cancelled",
        _ => "diagnostic provider reported an error",
    }
    .to_string();
    if error.code.is_empty()
        || !error
            .code
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        error.code = "provider_error".to_string();
    }
}

fn redact_public_physical_paths(text: &str, context: &DiagnosticContext) -> String {
    let mut redacted = text.to_string();
    for root in [
        &context.workspace.cwd,
        &context.workspace.workspace_root,
        &context.workspace.cache_root,
        &context.source_root.path,
    ] {
        let displayed = root.to_string_lossy();
        if !displayed.is_empty() {
            redacted = redacted.replace(displayed.as_ref(), "<physical-path>");
            redacted = redacted.replace(displayed.replace('\\', "/").as_str(), "<physical-path>");
        }
    }
    redact_absolute_path_tokens(&redacted)
}

fn redact_absolute_path_tokens(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while cursor < text.len() {
        let Some((offset, _)) = text[cursor..]
            .char_indices()
            .find(|(offset, _)| absolute_path_starts_at(text, cursor + offset))
        else {
            output.push_str(&text[cursor..]);
            break;
        };
        let start = cursor + offset;
        output.push_str(&text[cursor..start]);
        let end = text[start..]
            .char_indices()
            .skip(1)
            .find_map(|(offset, ch)| {
                (ch.is_whitespace()
                    || matches!(
                        ch,
                        '"' | '\'' | '<' | '>' | '|' | ',' | ';' | ')' | ']' | '}'
                    ))
                .then_some(start + offset)
            })
            .unwrap_or(text.len());
        output.push_str("<physical-path>");
        cursor = end;
    }
    output
}

fn absolute_path_starts_at(text: &str, index: usize) -> bool {
    if !text.is_char_boundary(index) {
        return false;
    }
    let previous = text[..index].chars().next_back();
    let boundary = index == 0
        || previous.is_some_and(|ch| !ch.is_alphanumeric() && !matches!(ch, '_' | '/' | '\\'));
    if !boundary {
        return false;
    }
    let suffix = &text[index..];
    if suffix
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file://"))
        || suffix.starts_with("\\\\")
    {
        return true;
    }
    let mut chars = suffix.chars();
    match (chars.next(), chars.next(), chars.next()) {
        (Some(drive), Some(':'), Some(separator))
            if drive.is_ascii_alphabetic() && matches!(separator, '/' | '\\') =>
        {
            true
        }
        (Some('/'), Some(next), _) if next != '/' && !next.is_whitespace() => true,
        _ => false,
    }
}

struct DiagnosticWorkerAdmission {
    counts: Mutex<HashMap<DiagnosticProviderId, usize>>,
    per_provider_limit: usize,
}

impl DiagnosticWorkerAdmission {
    fn try_acquire(
        self: &Arc<Self>,
        provider_id: DiagnosticProviderId,
    ) -> Option<DiagnosticWorkerPermit> {
        let mut counts = self
            .counts
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let count = counts.entry(provider_id).or_default();
        if *count >= self.per_provider_limit {
            return None;
        }
        *count += 1;
        Some(DiagnosticWorkerPermit {
            admission: Arc::clone(self),
            provider_id,
        })
    }
}

struct DiagnosticWorkerPermit {
    admission: Arc<DiagnosticWorkerAdmission>,
    provider_id: DiagnosticProviderId,
}

impl Drop for DiagnosticWorkerPermit {
    fn drop(&mut self) {
        let mut counts = self
            .admission
            .counts
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(count) = counts.get_mut(&self.provider_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                counts.remove(&self.provider_id);
            }
        }
    }
}

fn diagnostic_worker_admission() -> Arc<DiagnosticWorkerAdmission> {
    static ADMISSION: OnceLock<Arc<DiagnosticWorkerAdmission>> = OnceLock::new();
    Arc::clone(ADMISSION.get_or_init(|| {
        Arc::new(DiagnosticWorkerAdmission {
            counts: Mutex::new(HashMap::new()),
            per_provider_limit: MAX_CONCURRENT_DIAGNOSTIC_WORKERS_PER_PROVIDER,
        })
    }))
}

struct DiagnosticWorkerLifecycle {
    handles: Mutex<Vec<thread::JoinHandle<()>>>,
}

impl DiagnosticWorkerLifecycle {
    fn track(&self, handle: thread::JoinHandle<()>) {
        let mut handles = self
            .handles
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
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
                    .unwrap_or_else(|poison| poison.into_inner());
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
            .unwrap_or_else(|poison| poison.into_inner());
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
}

fn diagnostic_worker_lifecycle() -> Arc<DiagnosticWorkerLifecycle> {
    static LIFECYCLE: OnceLock<Arc<DiagnosticWorkerLifecycle>> = OnceLock::new();
    Arc::clone(LIFECYCLE.get_or_init(|| {
        Arc::new(DiagnosticWorkerLifecycle {
            handles: Mutex::new(Vec::new()),
        })
    }))
}

pub(crate) fn drain_diagnostic_workers(timeout: Duration) -> bool {
    diagnostic_worker_lifecycle().drain(timeout)
}

#[cfg(test)]
pub(crate) fn track_diagnostic_worker_for_test(handle: thread::JoinHandle<()>) {
    diagnostic_worker_lifecycle().track(handle);
}

fn select_providers(
    registry: &DiagnosticProviderRegistry,
    request: &DiagnosticRequest,
    target_kind: TargetKind,
) -> Result<Vec<SelectedProvider>, DiagnosticRequestError> {
    let registered = registry
        .descriptors()
        .map(|descriptor| descriptor.id.as_str())
        .collect::<HashSet<_>>();
    if request
        .filter
        .codes
        .iter()
        .any(|code| !registered.contains(code.provider.as_str()))
    {
        return Err(request_error(
            "provider_unknown",
            Some("filter.codes"),
            "filter.codes contains an unregistered diagnostics provider namespace",
        ));
    }
    let mut selected = Vec::new();
    for provider in registry.providers() {
        let descriptor = provider.descriptor();
        let applicable = descriptor.supports_action(request.action)
            && (request.action != DiagnosticAction::Findings
                || descriptor.supports_findings_target(target_kind));
        if applicable {
            selected.push(SelectedProvider {
                descriptor,
                provider: Arc::clone(provider),
                applicable,
            });
        }
    }
    if selected.is_empty() {
        return Err(request_error(
            "no_applicable_provider",
            None,
            "no registered diagnostics provider supports the selected action and target",
        ));
    }
    Ok(selected)
}

fn selection(
    request: &DiagnosticRequest,
    context: &DiagnosticContext,
    providers: &[SelectedProvider],
) -> DiagnosticSelection {
    let exposes_target = matches!(
        request.action,
        DiagnosticAction::Analyze | DiagnosticAction::Findings
    );
    let exposes_filter = request.action != DiagnosticAction::Status;
    let exposes_limit = action_has_items(request.action);
    DiagnosticSelection {
        source_set: request.source_set.clone(),
        metadata_path: request.metadata_path.clone(),
        target_kind: exposes_target.then_some(context.target.target_kind),
        providers: providers
            .iter()
            .map(|provider| provider.descriptor.id.as_str())
            .collect(),
        filter: exposes_filter.then_some(request.filter.clone()),
        limit: exposes_limit.then_some(request.limit),
    }
}

fn unsupported_section(
    descriptor: &DiagnosticProviderDescriptor,
    action: DiagnosticAction,
) -> DiagnosticProviderSection {
    DiagnosticProviderSection {
        id: descriptor.id.as_str(),
        status: DiagnosticProviderStatus::Unsupported,
        complete: false,
        version: None,
        capabilities: (action == DiagnosticAction::Catalog).then(|| descriptor.into()),
        readiness: None,
        items_total: action_has_items(action).then_some(0),
        items_returned: action_has_items(action).then_some(0),
        resource_failures: action_has_observations(action).then_some(0),
        truncated: action_has_items(action).then_some(false),
        error: Some(DiagnosticError {
            code: "target_not_supported".to_string(),
            message: format!(
                "provider {} does not support {} for the selected target",
                descriptor.id.as_str(),
                action.as_str()
            ),
            retryable: false,
        }),
    }
}

fn failed_mapping_section(
    descriptor: &DiagnosticProviderDescriptor,
    version: Option<String>,
    error: DiagnosticMapError,
    action: DiagnosticAction,
) -> DiagnosticProviderSection {
    DiagnosticProviderSection {
        id: descriptor.id.as_str(),
        status: DiagnosticProviderStatus::Failed,
        complete: false,
        version,
        capabilities: (action == DiagnosticAction::Catalog).then(|| descriptor.into()),
        readiness: None,
        items_total: action_has_items(action).then_some(0),
        items_returned: action_has_items(action).then_some(0),
        resource_failures: action_has_observations(action).then_some(0),
        truncated: action_has_items(action).then_some(false),
        error: Some(DiagnosticError {
            code: error.code.to_string(),
            message: error.message,
            retryable: false,
        }),
    }
}

fn item_matches_request(
    item: &DiagnosticItem,
    request: &DiagnosticRequest,
    context: &DiagnosticContext,
) -> bool {
    let provider_code = match item {
        DiagnosticItem::Diagnostic {
            provider,
            code,
            severity,
            ..
        } => {
            if request
                .filter
                .min_severity
                .is_some_and(|minimum| severity_rank(*severity) < severity_rank(minimum))
            {
                return false;
            }
            Some((*provider, code))
        }
        DiagnosticItem::DiagnosticRule { provider, code, .. } => Some((*provider, code)),
        DiagnosticItem::ResourceFailure { .. } => None,
    };
    if let Some((provider, code)) = provider_code {
        if !request.filter.codes.is_empty()
            && !request
                .filter
                .codes
                .iter()
                .any(|filter| filter.provider == provider && filter.code == *code)
        {
            return false;
        }
    }
    if request.action == DiagnosticAction::Findings
        && !item_location(item)
            .is_some_and(|location| location_within_findings_target(location, &context.target))
    {
        return false;
    }
    let Some(requested_range) = request.range else {
        return true;
    };
    let location = item_location(item);
    if !location.is_some_and(|location| location_matches_target(location, &context.target)) {
        return false;
    }
    match item {
        DiagnosticItem::Diagnostic {
            focus: DiagnosticFocus::SourceRange { range },
            ..
        } => range.intersects(requested_range),
        DiagnosticItem::Diagnostic {
            focus: DiagnosticFocus::Target,
            ..
        }
        | DiagnosticItem::ResourceFailure { .. } => true,
        DiagnosticItem::Diagnostic {
            focus: DiagnosticFocus::Metadata { .. },
            ..
        }
        | DiagnosticItem::DiagnosticRule { .. } => false,
    }
}

fn location_within_findings_target(
    location: &SourceLocation,
    target: &crate::domain::source_target::ResolvedTarget,
) -> bool {
    if location_matches_target(location, target) {
        return true;
    }
    matches!(
        location,
        SourceLocation::Unaddressable {
            source_set,
            owner_metadata_path,
            ..
        } if target.target_kind == TargetKind::MetadataObject
            && source_set == &target.source_set
            && owner_metadata_path == &target.metadata_path
    )
}

fn severity_rank(severity: DiagnosticSeverity) -> u8 {
    match severity {
        DiagnosticSeverity::Hint => 0,
        DiagnosticSeverity::Info => 1,
        DiagnosticSeverity::Warning => 2,
        DiagnosticSeverity::Error => 3,
    }
}

fn location_matches_target(
    location: &SourceLocation,
    target: &crate::domain::source_target::ResolvedTarget,
) -> bool {
    matches!(
        location,
        SourceLocation::Addressed {
            source_set,
            metadata_path,
            target_kind,
        } if source_set == &target.source_set
            && metadata_path == &target.metadata_path
            && target_kind == &target.target_kind
    )
}

fn item_location(item: &DiagnosticItem) -> Option<&SourceLocation> {
    match item {
        DiagnosticItem::Diagnostic { location, .. }
        | DiagnosticItem::ResourceFailure { location, .. } => Some(location),
        DiagnosticItem::DiagnosticRule { .. } => None,
    }
}

fn diagnostic_sort_key(
    item: &DiagnosticItem,
    provider_order: &HashMap<&str, usize>,
) -> (String, String, usize, u8, String, String) {
    let location = match item_location(item) {
        Some(SourceLocation::Addressed {
            source_set,
            metadata_path,
            target_kind,
        }) => format!(
            "0|{source_set}|{}|{:?}",
            metadata_path.as_ref().map_or("", MetadataAddress::as_str),
            target_kind
        ),
        Some(SourceLocation::Unaddressable {
            source_set,
            owner_metadata_path,
            path,
            ..
        }) => format!(
            "1|{source_set}|{}|{path}",
            owner_metadata_path
                .as_ref()
                .map_or("", MetadataAddress::as_str)
        ),
        None => "2".to_string(),
    };
    let focus = match item {
        DiagnosticItem::Diagnostic {
            focus: DiagnosticFocus::Target,
            ..
        }
        | DiagnosticItem::ResourceFailure { .. } => "0".to_string(),
        DiagnosticItem::Diagnostic {
            focus: DiagnosticFocus::SourceRange { range },
            ..
        } => format!(
            "1|{:020}|{:020}|{:020}|{:020}",
            range.start_line, range.start_column, range.end_line, range.end_column
        ),
        DiagnosticItem::Diagnostic {
            focus:
                DiagnosticFocus::Metadata {
                    element_path,
                    property,
                    language,
                },
            ..
        } => format!(
            "2|{}|{}|{}",
            element_path
                .iter()
                .map(|element| format!("{}:{}", element.collection, element.name))
                .collect::<Vec<_>>()
                .join("/"),
            property.as_deref().unwrap_or(""),
            language.as_deref().unwrap_or("")
        ),
        DiagnosticItem::DiagnosticRule { .. } => "3".to_string(),
    };
    let provider = item_provider(item);
    let provider_rank = provider_order.get(provider).copied().unwrap_or(usize::MAX);
    let (kind, code, message) = match item {
        DiagnosticItem::Diagnostic { code, message, .. } => (0, code.clone(), message.clone()),
        DiagnosticItem::ResourceFailure { error, .. } => {
            (1, error.code.clone(), error.message.clone())
        }
        DiagnosticItem::DiagnosticRule { code, title, .. } => (2, code.clone(), title.clone()),
    };
    (location, focus, provider_rank, kind, code, message)
}

fn item_provider(item: &DiagnosticItem) -> &'static str {
    match item {
        DiagnosticItem::Diagnostic { provider, .. }
        | DiagnosticItem::ResourceFailure { provider, .. }
        | DiagnosticItem::DiagnosticRule { provider, .. } => provider,
    }
}

fn action_has_observations(action: DiagnosticAction) -> bool {
    matches!(
        action,
        DiagnosticAction::Analyze | DiagnosticAction::Findings
    )
}

fn action_has_items(action: DiagnosticAction) -> bool {
    action != DiagnosticAction::Status
}

pub(crate) fn parse_diagnostic_request(
    args: &Map<String, Value>,
) -> Result<DiagnosticRequest, DiagnosticRequestError> {
    let action_raw = required_string(args, "action")?;
    let action = DiagnosticAction::parse(action_raw).ok_or_else(|| {
        request_error(
            "action_invalid",
            Some("action"),
            "action must be analyze, findings, status, or catalog",
        )
    })?;
    let source_set = required_string(args, "sourceSet")?.to_string();
    let metadata_path = args
        .get("metadataPath")
        .and_then(Value::as_str)
        .map(|raw| MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw))
        .transpose()
        .map_err(|_| {
            request_error(
                "metadata_address_invalid",
                Some("metadataPath"),
                "metadataPath is not a valid logical address",
            )
        })?;
    let default_minimum = matches!(
        action,
        DiagnosticAction::Analyze | DiagnosticAction::Findings
    )
    .then_some(DiagnosticSeverity::Warning);
    let filter_value = args.get("filter").and_then(Value::as_object);
    let min_severity = filter_value
        .and_then(|filter| filter.get("minSeverity"))
        .and_then(Value::as_str)
        .map(parse_severity)
        .transpose()?
        .or(default_minimum);
    let codes = filter_value
        .and_then(|filter| filter.get("codes"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .map(|code| DiagnosticCodeFilter {
            provider: code
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            code: code
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
        .collect();
    let range = args
        .get("range")
        .and_then(Value::as_object)
        .map(|range| DiagnosticRange {
            start_line: json_usize(range, "startLine"),
            start_column: json_usize(range, "startColumn"),
            end_line: json_usize(range, "endLine"),
            end_column: json_usize(range, "endColumn"),
        });
    if range.is_some_and(|range| !range.is_non_empty()) {
        return Err(request_error(
            "range_invalid",
            Some("range"),
            "range must be ordered, non-empty, zero-based, and end-exclusive",
        ));
    }
    Ok(DiagnosticRequest {
        action,
        source_set,
        metadata_path,
        filter: DiagnosticFilter {
            min_severity,
            codes,
        },
        range,
        limit: args
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(DIAGNOSTIC_LIMIT_DEFAULT),
        // The operational snapshot is the single owner of the analyze budget.
        // `invoke` installs it after schema validation and config resolution.
        timeout: None,
    })
}

fn required_string<'a>(
    args: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, DiagnosticRequestError> {
    args.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty() && value.trim() == *value)
        .ok_or_else(|| {
            request_error(
                "required_argument_missing",
                Some(field),
                format!("{field} must be a non-empty unpadded string"),
            )
        })
}

fn parse_severity(value: &str) -> Result<DiagnosticSeverity, DiagnosticRequestError> {
    match value {
        "error" => Ok(DiagnosticSeverity::Error),
        "warning" => Ok(DiagnosticSeverity::Warning),
        "info" => Ok(DiagnosticSeverity::Info),
        "hint" => Ok(DiagnosticSeverity::Hint),
        _ => Err(request_error(
            "severity_invalid",
            Some("filter.minSeverity"),
            "minSeverity must be error, warning, info, or hint",
        )),
    }
}

fn json_usize(object: &Map<String, Value>, field: &str) -> usize {
    object
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
}

fn request_error(
    code: &'static str,
    field: Option<&'static str>,
    message: impl Into<String>,
) -> DiagnosticRequestError {
    DiagnosticRequestError {
        code,
        field,
        message: message.into(),
        retryable: false,
    }
}

fn cancelled_request_error(message: impl Into<String>) -> DiagnosticRequestError {
    DiagnosticRequestError {
        code: "cancelled",
        field: None,
        message: message.into(),
        retryable: true,
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_diagnostic_request, DiagnosticCoordinator, DiagnosticMapping};
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::diagnostics::*;
    use crate::domain::project_sources::{ProjectSourceSet, SourceFormat, SourceSetKind};
    use crate::domain::source_location::SourceLocation;
    use crate::domain::source_roots::ResolvedSourceRoot;
    use crate::domain::source_target::{
        MetadataAddress, ResolvedTarget, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
    };
    use crate::domain::workspace::WorkspaceContext;
    use serde_json::{json, Map, Value};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Condvar, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    const ANALYZER: DiagnosticProviderId = DiagnosticProviderId::new_const("bsl-analyzer");
    const LANGUAGE_SERVER: DiagnosticProviderId =
        DiagnosticProviderId::new_const("bsl-language-server");
    const METADATA_VALIDATOR: DiagnosticProviderId =
        DiagnosticProviderId::new_const("metadata-validator");

    static ANALYZER_DESCRIPTOR: DiagnosticProviderDescriptor = DiagnosticProviderDescriptor {
        id: ANALYZER,
        actions: &[
            DiagnosticAction::Analyze,
            DiagnosticAction::Findings,
            DiagnosticAction::Status,
            DiagnosticAction::Catalog,
        ],
        findings_target_kinds: &[TargetKind::Module],
        emits_focus_kinds: &[DiagnosticFocusKind::SourceRange],
    };
    static LANGUAGE_SERVER_DESCRIPTOR: DiagnosticProviderDescriptor =
        DiagnosticProviderDescriptor {
            id: LANGUAGE_SERVER,
            actions: &[DiagnosticAction::Findings, DiagnosticAction::Status],
            findings_target_kinds: &[TargetKind::Module],
            emits_focus_kinds: &[DiagnosticFocusKind::SourceRange],
        };
    static METADATA_DESCRIPTOR: DiagnosticProviderDescriptor = DiagnosticProviderDescriptor {
        id: METADATA_VALIDATOR,
        actions: &[DiagnosticAction::Analyze, DiagnosticAction::Findings],
        findings_target_kinds: &[TargetKind::MetadataObject],
        emits_focus_kinds: &[DiagnosticFocusKind::Metadata],
    };

    struct FakeProvider {
        descriptor: &'static DiagnosticProviderDescriptor,
        outcome: DiagnosticProviderOutcome,
        calls: Arc<AtomicUsize>,
    }

    impl DiagnosticProvider for FakeProvider {
        fn descriptor(&self) -> &'static DiagnosticProviderDescriptor {
            self.descriptor
        }

        fn execute(
            &self,
            _request: &DiagnosticProviderRequest,
            _context: &DiagnosticContext,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> DiagnosticProviderOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.outcome.clone()
        }
    }

    #[derive(Default)]
    struct FakeMapping;

    static FAKE_MAPPING: FakeMapping = FakeMapping;

    impl DiagnosticMapping for FakeMapping {
        fn resolve_context(
            &self,
            request: &DiagnosticRequest,
            workspace: &WorkspaceContext,
            _cancellation: &CancellationToken,
        ) -> Result<DiagnosticContext, DiagnosticRequestError> {
            let target_kind = request
                .metadata_path
                .as_ref()
                .map_or(TargetKind::SourceRoot, MetadataAddress::target_kind);
            Ok(DiagnosticContext::new(
                workspace.clone(),
                ProjectSourceSet {
                    name: request.source_set.clone(),
                    kind: SourceSetKind::Configuration,
                    path: "src".to_string(),
                    source_format: SourceFormat::PlatformXml,
                    format_evidence: Vec::new(),
                    format_probe_error: None,
                },
                ResolvedSourceRoot {
                    source_set: Some(request.source_set.clone()),
                    path: workspace.workspace_root.join("src"),
                },
                ResolvedTarget {
                    source_set: request.source_set.clone(),
                    metadata_path: request.metadata_path.clone(),
                    target_kind,
                },
            ))
        }

        fn map_observation(
            &self,
            observation: DiagnosticObservation,
            context: &DiagnosticContext,
            _cancellation: &CancellationToken,
        ) -> Result<DiagnosticItem, DiagnosticMapError> {
            let (provider, observation_location, focus, payload) = match observation {
                DiagnosticObservation::Diagnostic {
                    provider,
                    location,
                    focus,
                    code,
                    severity,
                    message,
                    tags,
                } => (
                    provider,
                    location,
                    focus,
                    Some((code, severity, message, tags)),
                ),
                DiagnosticObservation::ResourceFailure {
                    provider,
                    location,
                    error,
                } => {
                    let (location, location_reason) = fake_location(location, context)?;
                    return Ok(DiagnosticItem::ResourceFailure {
                        provider: provider.as_str(),
                        location,
                        location_reason,
                        error,
                    });
                }
            };
            let (location, location_reason) = fake_location(observation_location, context)?;
            let focus = match focus {
                DiagnosticObservationFocus::Target => DiagnosticFocus::Target,
                DiagnosticObservationFocus::SourceRange(range) => {
                    DiagnosticFocus::SourceRange { range }
                }
                DiagnosticObservationFocus::Metadata(focus) => focus.into(),
            };
            let (code, severity, message, tags) = payload.unwrap();
            Ok(DiagnosticItem::Diagnostic {
                provider: provider.as_str(),
                location,
                location_reason,
                focus,
                code,
                severity,
                message,
                tags,
            })
        }
    }

    fn fake_location(
        location: DiagnosticObservationLocation,
        context: &DiagnosticContext,
    ) -> Result<(SourceLocation, Option<UnaddressableReason>), DiagnosticMapError> {
        let metadata_path = match location {
            DiagnosticObservationLocation::Logical { metadata_path } => metadata_path,
            DiagnosticObservationLocation::Resource { handle } if handle == "outside" => {
                return Err(DiagnosticMapError {
                    code: "location_outside_source_set",
                    message: "provider resource is outside the selected sourceSet".to_string(),
                })
            }
            DiagnosticObservationLocation::Resource { handle } if handle == "selected" => {
                context.target.metadata_path.clone()
            }
            DiagnosticObservationLocation::Resource { handle } if handle == "inner" => {
                return Ok((
                    SourceLocation::Unaddressable {
                        source_set: context.target.source_set.clone(),
                        owner_metadata_path: context.target.metadata_path.clone(),
                        path: "Catalogs/Selected/Ext/Unknown.xml".to_string(),
                    },
                    Some(UnaddressableReason::ResourceNotAddressable),
                ));
            }
            DiagnosticObservationLocation::Resource { handle } => {
                Some(address(&format!("CommonModule.{handle}.Module")))
            }
        };
        let target_kind = metadata_path
            .as_ref()
            .map_or(TargetKind::SourceRoot, MetadataAddress::target_kind);
        Ok((
            SourceLocation::Addressed {
                source_set: context.target.source_set.clone(),
                metadata_path,
                target_kind,
            },
            None,
        ))
    }

    fn provider(
        descriptor: &'static DiagnosticProviderDescriptor,
        outcome: DiagnosticProviderOutcome,
    ) -> (Arc<dyn DiagnosticProvider>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Arc::new(FakeProvider {
                descriptor,
                outcome,
                calls: Arc::clone(&calls),
            }),
            calls,
        )
    }

    fn successful(observations: Vec<DiagnosticObservation>) -> DiagnosticProviderOutcome {
        DiagnosticProviderOutcome {
            status: if observations.is_empty() {
                DiagnosticProviderStatus::Empty
            } else {
                DiagnosticProviderStatus::Completed
            },
            complete: true,
            version: Some("test".to_string()),
            observations,
            rules: Vec::new(),
            readiness: None,
            error: None,
        }
    }

    fn fake_registry(
        outcomes: [DiagnosticProviderOutcome; 3],
    ) -> (DiagnosticProviderRegistry, [Arc<AtomicUsize>; 3]) {
        let (analyzer, analyzer_calls) = provider(&ANALYZER_DESCRIPTOR, outcomes[0].clone());
        let (language_server, language_server_calls) =
            provider(&LANGUAGE_SERVER_DESCRIPTOR, outcomes[1].clone());
        let (metadata, metadata_calls) = provider(&METADATA_DESCRIPTOR, outcomes[2].clone());
        (
            DiagnosticProviderRegistry::new(vec![analyzer, language_server, metadata]).unwrap(),
            [analyzer_calls, language_server_calls, metadata_calls],
        )
    }

    fn address(raw: &str) -> MetadataAddress {
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw).unwrap()
    }

    fn workspace() -> WorkspaceContext {
        WorkspaceContext {
            cwd: PathBuf::from("workspace"),
            workspace_root: PathBuf::from("workspace"),
            cache_root: PathBuf::from("workspace/.build/unica"),
            workspace_epoch: 1,
        }
    }

    fn run_with_logical_mapping(
        descriptor: &'static DiagnosticProviderDescriptor,
        outcome: DiagnosticProviderOutcome,
        request: &DiagnosticRequest,
        workspace: &WorkspaceContext,
    ) -> DiagnosticResult {
        let (provider, _) = provider(descriptor, outcome);
        let registry = DiagnosticProviderRegistry::new(vec![provider]).unwrap();
        DiagnosticCoordinator::new(registry, &FAKE_MAPPING)
            .execute(request, workspace, &CancellationToken::new())
            .unwrap()
    }

    fn assert_no_physical_transport(value: &Value, workspace_root: &str) {
        match value {
            Value::Object(object) => {
                for (key, value) in object {
                    assert!(
                        !matches!(
                            key.as_str(),
                            "path" | "uri" | "sourceDir" | "stdout" | "stderr" | "command"
                        ),
                        "physical transport key leaked: {key}"
                    );
                    assert_no_physical_transport(value, workspace_root);
                }
            }
            Value::Array(items) => {
                for item in items {
                    assert_no_physical_transport(item, workspace_root);
                }
            }
            Value::String(text) => assert!(
                !text.contains(workspace_root),
                "workspace root leaked into diagnostics data: {text}"
            ),
            _ => {}
        }
    }

    fn findings_request() -> DiagnosticRequest {
        DiagnosticRequest {
            action: DiagnosticAction::Findings,
            source_set: "main".to_string(),
            metadata_path: Some(address("CommonModule.Selected.Module")),
            filter: DiagnosticFilter::default(),
            range: None,
            limit: 200,
            timeout: None,
        }
    }

    fn diagnostic(
        provider: DiagnosticProviderId,
        handle: &str,
        code: &str,
        severity: DiagnosticSeverity,
        focus: DiagnosticObservationFocus,
    ) -> DiagnosticObservation {
        DiagnosticObservation::Diagnostic {
            provider,
            location: DiagnosticObservationLocation::Resource {
                handle: handle.to_string(),
            },
            focus,
            code: code.to_string(),
            severity,
            message: format!("message {code}"),
            tags: Vec::new(),
        }
    }

    fn run(
        registry: DiagnosticProviderRegistry,
        request: &DiagnosticRequest,
    ) -> Result<DiagnosticResult, DiagnosticRequestError> {
        DiagnosticCoordinator::new(registry, &FAKE_MAPPING).execute(
            request,
            &workspace(),
            &CancellationToken::new(),
        )
    }

    #[test]
    fn diagnostics_provider_selection_uses_registry_order_and_skips_inapplicable_providers() {
        let empty = successful(Vec::new());
        let (registry, calls) = fake_registry([empty.clone(), empty.clone(), empty]);
        let automatic = run(registry, &findings_request()).unwrap();
        assert_eq!(
            automatic.selection.providers,
            vec!["bsl-analyzer", "bsl-language-server"]
        );
        assert_eq!(automatic.state, DiagnosticResultState::Completed);
        assert!(automatic.complete);
        assert_eq!(calls[2].load(Ordering::SeqCst), 0);
    }

    #[test]
    fn diagnostics_code_filters_do_not_select_execution_providers() {
        let empty = successful(Vec::new());
        let (registry, calls) = fake_registry([empty.clone(), empty.clone(), empty]);
        let mut narrowed = findings_request();
        narrowed.filter.codes = vec![DiagnosticCodeFilter {
            provider: "bsl-language-server".to_string(),
            code: "UNKNOWN".to_string(),
        }];
        let result = run(registry, &narrowed).unwrap();
        assert_eq!(
            result.selection.providers,
            vec!["bsl-analyzer", "bsl-language-server"]
        );
        assert_eq!(result.items_total, Some(0));
        assert_eq!(calls[0].load(Ordering::SeqCst), 1);
        assert_eq!(calls[1].load(Ordering::SeqCst), 1);
    }

    #[test]
    fn diagnostics_catalog_filters_exact_provider_code_pairs_before_global_limit() {
        let analyzer = DiagnosticProviderOutcome {
            status: DiagnosticProviderStatus::Completed,
            complete: true,
            version: Some("test".to_string()),
            observations: Vec::new(),
            rules: vec![
                DiagnosticRuleObservation {
                    provider: ANALYZER,
                    code: "KEEP".to_string(),
                    default_severity: DiagnosticSeverity::Warning,
                    title: "Keep".to_string(),
                    description: None,
                    tags: Vec::new(),
                },
                DiagnosticRuleObservation {
                    provider: ANALYZER,
                    code: "DROP".to_string(),
                    default_severity: DiagnosticSeverity::Warning,
                    title: "Drop".to_string(),
                    description: None,
                    tags: Vec::new(),
                },
            ],
            readiness: None,
            error: None,
        };
        let (registry, calls) =
            fake_registry([analyzer, successful(Vec::new()), successful(Vec::new())]);
        let mut request = findings_request();
        request.action = DiagnosticAction::Catalog;
        request.metadata_path = None;
        request.filter.min_severity = None;
        request.filter.codes = vec![DiagnosticCodeFilter {
            provider: "bsl-analyzer".to_string(),
            code: "KEEP".to_string(),
        }];
        request.limit = 1;

        let result = run(registry, &request).unwrap();

        assert_eq!(result.items_total, Some(1));
        assert_eq!(result.items_returned, Some(1));
        assert_eq!(result.truncated, Some(false));
        assert!(matches!(
            result.items.as_slice(),
            [DiagnosticItem::DiagnosticRule { provider: "bsl-analyzer", code, .. }]
                if code == "KEEP"
        ));
        assert_eq!(calls[0].load(Ordering::SeqCst), 1);
        assert_eq!(calls[1].load(Ordering::SeqCst), 0);
    }

    #[test]
    fn diagnostics_provider_selection_rejects_no_automatic_applicable_provider() {
        let empty = successful(Vec::new());
        let (analyzer, calls) = provider(&ANALYZER_DESCRIPTOR, empty);
        let registry = DiagnosticProviderRegistry::new(vec![analyzer]).unwrap();
        let mut request = findings_request();
        request.metadata_path = Some(address("Catalog.Items"));
        let error = run(registry, &request).unwrap_err();
        assert_eq!(error.code, "no_applicable_provider");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn diagnostics_result_assembly_filters_sorts_and_applies_one_global_limit() {
        let requested_range = DiagnosticRange {
            start_line: 2,
            start_column: 0,
            end_line: 4,
            end_column: 0,
        };
        let analyzer = successful(vec![
            diagnostic(
                ANALYZER,
                "selected",
                "Z-WARNING",
                DiagnosticSeverity::Warning,
                DiagnosticObservationFocus::SourceRange(DiagnosticRange {
                    start_line: 2,
                    start_column: 0,
                    end_line: 2,
                    end_column: 2,
                }),
            ),
            diagnostic(
                ANALYZER,
                "Alpha",
                "A-INFO",
                DiagnosticSeverity::Info,
                DiagnosticObservationFocus::Target,
            ),
            diagnostic(
                ANALYZER,
                "selected",
                "SELECTED",
                DiagnosticSeverity::Error,
                DiagnosticObservationFocus::Target,
            ),
            DiagnosticObservation::ResourceFailure {
                provider: ANALYZER,
                location: DiagnosticObservationLocation::Resource {
                    handle: "selected".to_string(),
                },
                error: DiagnosticError {
                    code: "source_decode_failed".to_string(),
                    message: "decode failed".to_string(),
                    retryable: false,
                },
            },
        ]);
        let language_server = successful(vec![diagnostic(
            LANGUAGE_SERVER,
            "selected",
            "SELECTED",
            DiagnosticSeverity::Error,
            DiagnosticObservationFocus::Target,
        )]);
        let (registry, _) = fake_registry([analyzer, language_server, successful(Vec::new())]);
        let mut request = findings_request();
        request.range = Some(requested_range);
        request.limit = 3;
        request.filter.min_severity = Some(DiagnosticSeverity::Warning);

        let result = run(registry, &request).unwrap();
        assert_eq!(result.items_total, Some(4));
        assert_eq!(result.items_returned, Some(3));
        assert_eq!(result.items.len(), 3);
        assert_eq!(result.truncated, Some(true));
        assert!(
            result.complete,
            "global truncation does not reduce provider coverage"
        );
        assert_eq!(result.providers[0].resource_failures, Some(1));
        assert_eq!(result.providers[0].items_total, Some(3));
        assert_eq!(result.providers[0].items_returned, Some(2));
        assert_eq!(result.providers[1].items_total, Some(1));
        assert_eq!(result.providers[1].items_returned, Some(1));

        let retained = result
            .items
            .iter()
            .map(|item| match item {
                DiagnosticItem::Diagnostic { provider, code, .. } => {
                    format!("{provider}:{code}")
                }
                DiagnosticItem::ResourceFailure { provider, .. } => {
                    format!("{provider}:resourceFailure")
                }
                DiagnosticItem::DiagnosticRule { .. } => "rule".to_string(),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            retained,
            vec![
                "bsl-analyzer:SELECTED",
                "bsl-analyzer:resourceFailure",
                "bsl-language-server:SELECTED"
            ]
        );
    }

    #[test]
    fn diagnostics_result_assembly_keeps_cross_provider_duplicates_and_metadata_focus_order() {
        let metadata_outcome = successful(vec![
            DiagnosticObservation::Diagnostic {
                provider: METADATA_VALIDATOR,
                location: DiagnosticObservationLocation::Logical {
                    metadata_path: Some(address("Catalog.Items")),
                },
                focus: DiagnosticObservationFocus::Metadata(MetadataFocus {
                    element_path: vec![MetadataElement {
                        collection: "attributes".to_string(),
                        name: "Zeta".to_string(),
                    }],
                    property: Some("Type".to_string()),
                    language: None,
                }),
                code: "META".to_string(),
                severity: DiagnosticSeverity::Warning,
                message: "same".to_string(),
                tags: Vec::new(),
            },
            DiagnosticObservation::Diagnostic {
                provider: METADATA_VALIDATOR,
                location: DiagnosticObservationLocation::Logical {
                    metadata_path: Some(address("Catalog.Items")),
                },
                focus: DiagnosticObservationFocus::Metadata(MetadataFocus {
                    element_path: vec![MetadataElement {
                        collection: "attributes".to_string(),
                        name: "Alpha".to_string(),
                    }],
                    property: Some("Type".to_string()),
                    language: Some("ru".to_string()),
                }),
                code: "META".to_string(),
                severity: DiagnosticSeverity::Warning,
                message: "same".to_string(),
                tags: Vec::new(),
            },
        ]);
        let empty = successful(Vec::new());
        let (registry, _) = fake_registry([empty.clone(), empty, metadata_outcome]);
        let mut request = findings_request();
        request.metadata_path = Some(address("Catalog.Items"));
        let result = run(registry, &request).unwrap();
        let paths = result
            .items
            .iter()
            .map(|item| match item {
                DiagnosticItem::Diagnostic {
                    focus: DiagnosticFocus::Metadata { element_path, .. },
                    ..
                } => element_path[0].name.as_str(),
                item => panic!("unexpected item: {item:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(paths, vec!["Alpha", "Zeta"]);
    }

    #[test]
    fn diagnostics_request_parser_materializes_defaults_and_stable_errors() {
        let args = Map::from_iter([
            ("action".to_string(), json!("findings")),
            ("sourceSet".to_string(), json!("main")),
            (
                "metadataPath".to_string(),
                json!("CommonModule.Shared.Module"),
            ),
        ]);
        let request = parse_diagnostic_request(&args).unwrap();
        assert_eq!(request.action, DiagnosticAction::Findings);
        assert_eq!(request.limit, 200);
        assert_eq!(
            request.filter.min_severity,
            Some(DiagnosticSeverity::Warning)
        );
        assert!(request.filter.codes.is_empty());

        let invalid = Map::<String, Value>::from_iter([
            ("action".to_string(), json!("findings")),
            ("sourceSet".to_string(), json!("main")),
            ("metadataPath".to_string(), json!("not-an-address")),
        ]);
        let error = parse_diagnostic_request(&invalid).unwrap_err();
        assert_eq!(error.code, "metadata_address_invalid");
        assert_eq!(error.field, Some("metadataPath"));
        assert!(!error.retryable);
    }

    fn failed(code: &str) -> DiagnosticProviderOutcome {
        DiagnosticProviderOutcome {
            status: DiagnosticProviderStatus::Failed,
            complete: false,
            version: None,
            observations: Vec::new(),
            rules: Vec::new(),
            readiness: None,
            error: Some(DiagnosticError {
                code: code.to_string(),
                message: "provider failed".to_string(),
                retryable: true,
            }),
        }
    }

    fn building() -> DiagnosticProviderOutcome {
        DiagnosticProviderOutcome {
            status: DiagnosticProviderStatus::Completed,
            complete: true,
            version: Some("test".to_string()),
            observations: Vec::new(),
            rules: Vec::new(),
            readiness: Some(DiagnosticReadiness {
                state: DiagnosticReadinessState::Building,
                retryable: true,
            }),
            error: None,
        }
    }

    #[test]
    fn diagnostics_outcome_matrix_distinguishes_complete_partial_and_failed() {
        let useful = successful(vec![diagnostic(
            ANALYZER,
            "selected",
            "A001",
            DiagnosticSeverity::Warning,
            DiagnosticObservationFocus::Target,
        )]);
        let (registry, _) = fake_registry([
            useful,
            failed("language_server_failed"),
            successful(Vec::new()),
        ]);
        let partial = run(registry, &findings_request()).unwrap();
        assert!(partial.ok);
        assert_eq!(partial.state, DiagnosticResultState::Partial);
        assert!(!partial.complete);

        let (registry, _) = fake_registry([
            failed("analyzer_failed"),
            failed("language_server_failed"),
            successful(Vec::new()),
        ]);
        let failed_result = run(registry, &findings_request()).unwrap();
        assert!(!failed_result.ok);
        assert_eq!(failed_result.state, DiagnosticResultState::Failed);
        assert!(!failed_result.complete);

        let empty = successful(Vec::new());
        let (registry, _) = fake_registry([empty.clone(), empty.clone(), empty]);
        let complete = run(registry, &findings_request()).unwrap();
        assert!(complete.ok);
        assert_eq!(complete.state, DiagnosticResultState::Completed);
        assert!(complete.complete);
    }

    #[test]
    fn diagnostics_outcome_matrix_treats_building_as_status_success_but_not_findings() {
        let (registry, _) = fake_registry([building(), building(), successful(Vec::new())]);
        let mut status_request = findings_request();
        status_request.action = DiagnosticAction::Status;
        status_request.metadata_path = None;
        status_request.filter = DiagnosticFilter {
            min_severity: None,
            codes: Vec::new(),
        };
        let status = run(registry, &status_request).unwrap();
        assert_eq!(status.state, DiagnosticResultState::Completed);
        assert!(status.complete);
        assert!(status.providers.iter().all(|section| {
            section.status == DiagnosticProviderStatus::Completed
                && section
                    .readiness
                    .as_ref()
                    .is_some_and(|readiness| readiness.state == DiagnosticReadinessState::Building)
        }));

        let (registry, _) = fake_registry([building(), building(), successful(Vec::new())]);
        let findings = run(registry, &findings_request()).unwrap();
        assert_eq!(findings.state, DiagnosticResultState::Failed);
        assert!(findings.providers.iter().all(|section| {
            section.status == DiagnosticProviderStatus::Unavailable
                && section
                    .error
                    .as_ref()
                    .is_some_and(|error| error.code == "provider_not_ready" && error.retryable)
        }));
    }

    enum ProviderBehavior {
        Outcome(DiagnosticProviderOutcome),
        WaitForRelease {
            started: mpsc::Sender<DiagnosticProviderId>,
            gate: Arc<(Mutex<bool>, Condvar)>,
        },
        Panic,
        SleepUntilCancelledOrElapsed {
            duration: Duration,
            saw_cancellation: Arc<std::sync::atomic::AtomicBool>,
        },
        WaitForCancellation {
            started: mpsc::Sender<DiagnosticProviderId>,
        },
    }

    struct BehaviorProvider {
        descriptor: &'static DiagnosticProviderDescriptor,
        behavior: ProviderBehavior,
    }

    impl DiagnosticProvider for BehaviorProvider {
        fn descriptor(&self) -> &'static DiagnosticProviderDescriptor {
            self.descriptor
        }

        fn execute(
            &self,
            _request: &DiagnosticProviderRequest,
            _context: &DiagnosticContext,
            _deadline: ProviderDeadline,
            cancellation: &CancellationToken,
        ) -> DiagnosticProviderOutcome {
            match &self.behavior {
                ProviderBehavior::Outcome(outcome) => outcome.clone(),
                ProviderBehavior::WaitForRelease { started, gate } => {
                    started.send(self.descriptor.id).unwrap();
                    let (lock, condition) = &**gate;
                    let mut released = lock.lock().unwrap();
                    while !*released {
                        let (next, _) = condition
                            .wait_timeout(released, Duration::from_millis(250))
                            .unwrap();
                        released = next;
                    }
                    successful(Vec::new())
                }
                ProviderBehavior::Panic => panic!("fake diagnostic provider panic"),
                ProviderBehavior::SleepUntilCancelledOrElapsed {
                    duration,
                    saw_cancellation,
                } => {
                    let started = Instant::now();
                    while started.elapsed() < *duration && !cancellation.is_cancelled() {
                        thread::sleep(Duration::from_millis(2));
                    }
                    saw_cancellation.store(cancellation.is_cancelled(), Ordering::SeqCst);
                    successful(Vec::new())
                }
                ProviderBehavior::WaitForCancellation { started } => {
                    started.send(self.descriptor.id).unwrap();
                    while !cancellation.is_cancelled() {
                        thread::sleep(Duration::from_millis(2));
                    }
                    successful(Vec::new())
                }
            }
        }
    }

    fn behavior_registry(
        providers: Vec<(&'static DiagnosticProviderDescriptor, ProviderBehavior)>,
    ) -> DiagnosticProviderRegistry {
        DiagnosticProviderRegistry::new(
            providers
                .into_iter()
                .map(|(descriptor, behavior)| {
                    Arc::new(BehaviorProvider {
                        descriptor,
                        behavior,
                    }) as Arc<dyn DiagnosticProvider>
                })
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn diagnostics_concurrency_starts_providers_independently() {
        let (started_tx, started_rx) = mpsc::channel();
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let registry = behavior_registry(vec![
            (
                &ANALYZER_DESCRIPTOR,
                ProviderBehavior::WaitForRelease {
                    started: started_tx.clone(),
                    gate: Arc::clone(&gate),
                },
            ),
            (
                &LANGUAGE_SERVER_DESCRIPTOR,
                ProviderBehavior::WaitForRelease {
                    started: started_tx,
                    gate: Arc::clone(&gate),
                },
            ),
        ]);
        let execution = thread::spawn(move || {
            DiagnosticCoordinator::new(registry, &FAKE_MAPPING).execute(
                &findings_request(),
                &workspace(),
                &CancellationToken::new(),
            )
        });
        let first = started_rx.recv_timeout(Duration::from_millis(300));
        let second = started_rx.recv_timeout(Duration::from_millis(300));
        {
            let (lock, condition) = &*gate;
            *lock.lock().unwrap() = true;
            condition.notify_all();
        }
        execution.join().unwrap().unwrap();
        assert!(first.is_ok(), "the first provider did not start");
        assert!(
            second.is_ok(),
            "the second provider waited for the first one"
        );
    }

    #[test]
    fn diagnostics_concurrency_contains_provider_panic_and_keeps_sibling_items() {
        let registry = behavior_registry(vec![
            (&ANALYZER_DESCRIPTOR, ProviderBehavior::Panic),
            (
                &LANGUAGE_SERVER_DESCRIPTOR,
                ProviderBehavior::Outcome(successful(vec![diagnostic(
                    LANGUAGE_SERVER,
                    "selected",
                    "LS001",
                    DiagnosticSeverity::Warning,
                    DiagnosticObservationFocus::Target,
                )])),
            ),
        ]);
        let result = run(registry, &findings_request()).unwrap();
        assert_eq!(result.state, DiagnosticResultState::Partial);
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.providers[0].status, DiagnosticProviderStatus::Failed);
        assert_eq!(
            result.providers[0].error.as_ref().unwrap().code,
            "provider_panicked"
        );
    }

    #[test]
    fn diagnostics_concurrency_timeout_cancels_provider_without_waiting_for_it() {
        let saw_cancellation = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let registry = behavior_registry(vec![(
            &ANALYZER_DESCRIPTOR,
            ProviderBehavior::SleepUntilCancelledOrElapsed {
                duration: Duration::from_millis(150),
                saw_cancellation: Arc::clone(&saw_cancellation),
            },
        )]);
        let mut request = findings_request();
        request.timeout = Some(Duration::from_millis(30));
        let started = Instant::now();
        let result = run(registry, &request).unwrap();
        assert!(started.elapsed() < Duration::from_millis(120));
        assert_eq!(result.providers[0].status, DiagnosticProviderStatus::Failed);
        assert_eq!(
            result.providers[0].error.as_ref().unwrap().code,
            "provider_timeout"
        );
        for _ in 0..50 {
            if saw_cancellation.load(Ordering::SeqCst) {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        assert!(saw_cancellation.load(Ordering::SeqCst));
    }

    #[test]
    fn diagnostics_cancellation_discards_partial_items() {
        let (started_tx, started_rx) = mpsc::channel();
        let registry = behavior_registry(vec![
            (
                &ANALYZER_DESCRIPTOR,
                ProviderBehavior::Outcome(successful(vec![diagnostic(
                    ANALYZER,
                    "selected",
                    "READY",
                    DiagnosticSeverity::Warning,
                    DiagnosticObservationFocus::Target,
                )])),
            ),
            (
                &LANGUAGE_SERVER_DESCRIPTOR,
                ProviderBehavior::WaitForCancellation {
                    started: started_tx,
                },
            ),
        ]);
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let execution = thread::spawn(move || {
            DiagnosticCoordinator::new(registry, &FAKE_MAPPING).execute(
                &findings_request(),
                &workspace(),
                &worker_cancellation,
            )
        });
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        cancellation.cancel();
        let error = execution.join().unwrap().unwrap_err();
        assert_eq!(error.code, "cancelled");
    }

    #[test]
    fn diagnostics_provider_contract_normalizes_identity_and_rejects_malformed_payloads() {
        let mismatched = successful(vec![diagnostic(
            LANGUAGE_SERVER,
            "selected",
            "MISMATCHED",
            DiagnosticSeverity::Warning,
            DiagnosticObservationFocus::Target,
        )]);
        let (registry, _) =
            fake_registry([mismatched, successful(Vec::new()), successful(Vec::new())]);
        let request = findings_request();
        let normalized = run(registry, &request).unwrap();
        assert!(matches!(
            &normalized.items[0],
            DiagnosticItem::Diagnostic {
                provider: "bsl-analyzer",
                ..
            }
        ));

        let malformed_failed = DiagnosticProviderOutcome {
            status: DiagnosticProviderStatus::Failed,
            complete: false,
            version: None,
            observations: Vec::new(),
            rules: Vec::new(),
            readiness: None,
            error: None,
        };
        let (registry, _) = fake_registry([
            malformed_failed,
            successful(Vec::new()),
            successful(Vec::new()),
        ]);
        let failed_result = run(registry, &request).unwrap();
        assert_eq!(
            failed_result.providers[0].error.as_ref().unwrap().code,
            "provider_contract_invalid"
        );

        let status_with_finding = successful(vec![diagnostic(
            ANALYZER,
            "selected",
            "WRONG-ACTION",
            DiagnosticSeverity::Warning,
            DiagnosticObservationFocus::Target,
        )]);
        let (registry, _) = fake_registry([
            status_with_finding,
            successful(Vec::new()),
            successful(Vec::new()),
        ]);
        let mut status_request = findings_request();
        status_request.action = DiagnosticAction::Status;
        status_request.metadata_path = None;
        status_request.filter.min_severity = None;
        let status = run(registry, &status_request).unwrap();
        assert_eq!(
            status.providers[0].error.as_ref().unwrap().code,
            "provider_contract_invalid"
        );

        let analyze_with_rule = DiagnosticProviderOutcome {
            status: DiagnosticProviderStatus::Completed,
            complete: true,
            version: None,
            observations: Vec::new(),
            rules: vec![DiagnosticRuleObservation {
                provider: ANALYZER,
                code: "RULE".to_string(),
                default_severity: DiagnosticSeverity::Warning,
                title: "Rule".to_string(),
                description: None,
                tags: Vec::new(),
            }],
            readiness: None,
            error: None,
        };
        let (registry, _) = fake_registry([
            analyze_with_rule,
            successful(Vec::new()),
            successful(Vec::new()),
        ]);
        let mut analyze_request = findings_request();
        analyze_request.action = DiagnosticAction::Analyze;
        analyze_request.metadata_path = None;
        let analyze = run(registry, &analyze_request).unwrap();
        assert_eq!(
            analyze.providers[0].error.as_ref().unwrap().code,
            "provider_contract_invalid"
        );

        for status in [
            DiagnosticProviderStatus::Failed,
            DiagnosticProviderStatus::Unavailable,
            DiagnosticProviderStatus::Unsupported,
            DiagnosticProviderStatus::Empty,
        ] {
            let malformed = DiagnosticProviderOutcome {
                status,
                complete: status == DiagnosticProviderStatus::Empty,
                version: None,
                observations: vec![diagnostic(
                    ANALYZER,
                    "selected",
                    "MUST-NOT-PUBLISH",
                    DiagnosticSeverity::Warning,
                    DiagnosticObservationFocus::Target,
                )],
                rules: Vec::new(),
                readiness: None,
                error: (status != DiagnosticProviderStatus::Empty).then(|| DiagnosticError {
                    code: "provider_failed".to_string(),
                    message: "provider failed".to_string(),
                    retryable: false,
                }),
            };
            let (registry, _) =
                fake_registry([malformed, successful(Vec::new()), successful(Vec::new())]);
            let rejected = run(registry, &request).unwrap();
            assert!(rejected.items.is_empty(), "status {status:?}");
            assert_eq!(
                rejected.providers[0].error.as_ref().unwrap().code,
                "provider_contract_invalid",
                "status {status:?}"
            );
        }

        let status_without_readiness = successful(Vec::new());
        let (registry, _) = fake_registry([
            status_without_readiness,
            successful(Vec::new()),
            successful(Vec::new()),
        ]);
        let missing_readiness = run(registry, &status_request).unwrap();
        assert_eq!(
            missing_readiness.providers[0].error.as_ref().unwrap().code,
            "provider_contract_invalid"
        );
    }

    #[test]
    fn diagnostics_public_result_redacts_provider_controlled_physical_paths() {
        let private_root = std::env::temp_dir().join("unica-diagnostics-private-message-workspace");
        let private_path = private_root.join("src/CommonModules/Secret/Ext/Module.bsl");
        let completed = DiagnosticProviderOutcome {
            status: DiagnosticProviderStatus::Completed,
            complete: false,
            version: Some("test".to_string()),
            observations: vec![
                DiagnosticObservation::Diagnostic {
                    provider: ANALYZER,
                    location: DiagnosticObservationLocation::Resource {
                        handle: "selected".to_string(),
                    },
                    focus: DiagnosticObservationFocus::Target,
                    code: "PATH-IN-MESSAGE".to_string(),
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "analysis failed at {}; see https://example.invalid/rule",
                        private_path.display()
                    ),
                    tags: Vec::new(),
                },
                DiagnosticObservation::ResourceFailure {
                    provider: ANALYZER,
                    location: DiagnosticObservationLocation::Resource {
                        handle: "selected".to_string(),
                    },
                    error: DiagnosticError {
                        code: "source_analysis_failed".to_string(),
                        message: format!(
                            "could not read {}; fallback C:\\private\\Secret.bsl",
                            private_path.display()
                        ),
                        retryable: false,
                    },
                },
            ],
            rules: Vec::new(),
            readiness: None,
            error: None,
        };
        let failed = DiagnosticProviderOutcome {
            status: DiagnosticProviderStatus::Failed,
            complete: false,
            version: Some("test".to_string()),
            observations: Vec::new(),
            rules: Vec::new(),
            readiness: None,
            error: Some(DiagnosticError {
                code: "provider_failed".to_string(),
                message: format!("provider failed below {}", private_root.display()),
                retryable: false,
            }),
        };
        let (registry, _) = fake_registry([completed, failed, successful(Vec::new())]);
        let workspace = WorkspaceContext {
            cwd: private_root.clone(),
            workspace_root: private_root.clone(),
            cache_root: private_root.join(".build/unica"),
            workspace_epoch: 1,
        };

        let result = DiagnosticCoordinator::new(registry, &FAKE_MAPPING)
            .execute(&findings_request(), &workspace, &CancellationToken::new())
            .unwrap();
        let serialized = serde_json::to_value(result).unwrap();

        assert_no_physical_transport(&serialized, &private_root.to_string_lossy());
        let text = serialized.to_string();
        assert!(!text.contains("C:\\\\private"), "{text}");
        assert!(text.contains("https://example.invalid/rule"), "{text}");
    }

    #[test]
    fn diagnostics_metadata_object_scope_excludes_separately_addressable_children() {
        let outcome = successful(vec![
            diagnostic(
                METADATA_VALIDATOR,
                "selected",
                "OBJECT",
                DiagnosticSeverity::Warning,
                DiagnosticObservationFocus::Target,
            ),
            diagnostic(
                METADATA_VALIDATOR,
                "inner",
                "INNER",
                DiagnosticSeverity::Warning,
                DiagnosticObservationFocus::Target,
            ),
            diagnostic(
                METADATA_VALIDATOR,
                "Child",
                "CHILD",
                DiagnosticSeverity::Warning,
                DiagnosticObservationFocus::Target,
            ),
        ]);
        let (registry, _) =
            fake_registry([successful(Vec::new()), successful(Vec::new()), outcome]);
        let mut request = findings_request();
        request.metadata_path = Some(address("Catalog.Selected"));

        let result = run(registry, &request).unwrap();
        let codes = result
            .items
            .iter()
            .filter_map(|item| match item {
                DiagnosticItem::Diagnostic { code, .. } => Some(code.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(codes, vec!["OBJECT", "INNER"]);
    }

    #[test]
    fn diagnostics_logical_assembly_preserves_cyrillic_module_without_physical_transport() {
        let workspace = workspace();
        let request = DiagnosticRequest {
            action: DiagnosticAction::Findings,
            source_set: "main".to_string(),
            metadata_path: Some(address("CommonModule.Документы Обмена.Module")),
            filter: DiagnosticFilter::default(),
            range: None,
            limit: 200,
            timeout: None,
        };
        let result = run_with_logical_mapping(
            &ANALYZER_DESCRIPTOR,
            successful(vec![diagnostic(
                ANALYZER,
                "Документы Обмена",
                "LineLength",
                DiagnosticSeverity::Warning,
                DiagnosticObservationFocus::SourceRange(DiagnosticRange {
                    start_line: 2,
                    start_column: 1,
                    end_line: 2,
                    end_column: 8,
                }),
            )]),
            &request,
            &workspace,
        );
        let data = serde_json::to_value(result).unwrap();

        assert_eq!(data["items"][0]["location"]["kind"], "addressed");
        assert_eq!(data["items"][0]["location"]["sourceSet"], "main");
        assert_eq!(
            data["items"][0]["location"]["metadataPath"],
            "CommonModule.Документы Обмена.Module"
        );
        assert_eq!(data["items"][0]["location"]["targetKind"], "module");
        assert_eq!(data["items"][0]["focus"]["kind"], "sourceRange");
        assert_no_physical_transport(&data, &workspace.workspace_root.to_string_lossy());
    }

    #[test]
    fn diagnostics_future_provider_contract_accepts_bsl_ls_and_metadata_focus() {
        let workspace = workspace();
        let module_request = DiagnosticRequest {
            action: DiagnosticAction::Findings,
            source_set: "main".to_string(),
            metadata_path: Some(address("CommonModule.Документы Обмена.Module")),
            filter: DiagnosticFilter::default(),
            range: None,
            limit: 200,
            timeout: None,
        };
        let bsl_ls = run_with_logical_mapping(
            &LANGUAGE_SERVER_DESCRIPTOR,
            successful(vec![diagnostic(
                LANGUAGE_SERVER,
                "Документы Обмена",
                "UnusedVariable",
                DiagnosticSeverity::Warning,
                DiagnosticObservationFocus::SourceRange(DiagnosticRange {
                    start_line: 0,
                    start_column: 0,
                    end_line: 0,
                    end_column: 1,
                }),
            )]),
            &module_request,
            &workspace,
        );
        let bsl_ls = serde_json::to_value(bsl_ls).unwrap();
        assert_eq!(bsl_ls["selection"]["providers"][0], "bsl-language-server");
        assert_eq!(bsl_ls["items"][0]["location"]["targetKind"], "module");
        assert_eq!(bsl_ls["items"][0]["focus"]["kind"], "sourceRange");

        let metadata_request = DiagnosticRequest {
            action: DiagnosticAction::Findings,
            source_set: "main".to_string(),
            metadata_path: Some(address("Catalog.Номенклатура")),
            filter: DiagnosticFilter::default(),
            range: None,
            limit: 200,
            timeout: None,
        };
        let metadata = run_with_logical_mapping(
            &METADATA_DESCRIPTOR,
            successful(vec![DiagnosticObservation::Diagnostic {
                provider: METADATA_VALIDATOR,
                location: DiagnosticObservationLocation::Logical {
                    metadata_path: Some(address("Catalog.Номенклатура")),
                },
                focus: DiagnosticObservationFocus::Metadata(MetadataFocus {
                    element_path: vec![
                        MetadataElement {
                            collection: "tabularSections".to_string(),
                            name: "Товары".to_string(),
                        },
                        MetadataElement {
                            collection: "attributes".to_string(),
                            name: "Цена".to_string(),
                        },
                    ],
                    property: Some("Type".to_string()),
                    language: None,
                }),
                code: "MetadataType".to_string(),
                severity: DiagnosticSeverity::Warning,
                message: "invalid metadata type".to_string(),
                tags: Vec::new(),
            }]),
            &metadata_request,
            &workspace,
        );
        let metadata = serde_json::to_value(metadata).unwrap();
        assert_eq!(
            metadata["items"][0]["location"]["targetKind"],
            "metadataObject"
        );
        assert_eq!(metadata["items"][0]["focus"]["kind"], "metadata");
        assert_eq!(metadata["items"][0]["focus"]["property"], "Type");

        let diagnostics_spec = crate::application::tools()
            .into_iter()
            .find(|tool| tool.name == "unica.code.diagnostics")
            .unwrap();
        let schema = crate::application::input_schema_for_tool(&diagnostics_spec);
        let schema_text = serde_json::to_string(&schema).unwrap();
        assert!(schema_text.contains("bsl-analyzer"));
        assert!(!schema_text.contains("bsl-language-server"));
        assert!(!schema_text.contains("metadata-validator"));
    }
}
