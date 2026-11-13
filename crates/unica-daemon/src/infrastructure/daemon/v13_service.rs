use super::server::{ActorBoundExecution, ActorBoundInvocation, CanonicalInvocationService};
use crate::domain::apply::OperationDescriptorExt;
use crate::domain::apply::SourceSetWriteSupport;
use crate::domain::engine::MissingEngineRefusal;
use unica_work_control::provider_deadline::ProviderDeadline;
/// Потолок страницы ветви графа: объявленный максимум поверхности.
// resource-limit: view.call-graph-initial-page
// class: step
// unit: count
// scope: request
// value: 50
// resource: Число соседей первого запроса ветви callGraph, по которому определяется размер полной ветви.
// status: unproven
// origin: 7adeabd5e (#849)
// problem: Первый ответ только задаёт размер запроса; усечённая ветвь не отдаётся пейджеру (комментарий complete_branch).
// on-reach: Полная ветвь запрашивается повторно с точным лимитом; если число не представимо — отказ result_too_large.
const CALL_GRAPH_PAGE_LIMIT: usize = 50;
/// Ranked providers (`symbol`, `semantic`) accept a top-N request, but no
/// offset, and a larger N is another answer rather than a longer one. Keep their
/// public bound as the fetch window and page the returned evidence
/// separately.
// resource-limit: search.provider-fetch-matches
// class: corpus
// unit: count
// scope: request
// value: 200
// resource: Окно совпадений, которое ранжирующие роли symbol и semantic получают от поставщика за запрос; буквальный поиск без роли окна не имеет.
// status: unproven
// origin: 022636d23 (#1029)
// problem: API провайдера принимает top-N без смещения; окно выборки сохранено от прежней публичной границы (комментарий в v13_service.rs).
// on-reach: Неполнота: секция сообщает limitReached и нижнюю оценку, предупреждение «Search returned 200 matches. Ask the user to narrow the query…»; хвост окна недоступен, курсор его не обещает.
// issue: #1119
// review: #1119: окно ролей symbol и semantic без продолжения — их API принимает top-N без смещения; буквальный поиск снят с окна в PR6 (#2079, #2095)
const PROVIDER_SEARCH_FETCH_LIMIT: usize = 200;

use super::server::text_search::{
    choose_engine, scan_text_page, TextContinuation, TextEngine, TextPageRequest, TextQuery,
    TextScan, TextScanEnd, TextSearchError, TextSource, MAX_TEXT_TOTALS_LEADING_MODULES,
    WALK_PAGE_RESPONSE_BUDGET_DIVISOR,
};
use super::v13_call_graph::{
    branch_collection, branch_direction, branch_owner, call_graph_support, complete_branch,
    declare_method_branches, fetch_summary, module_file_for_placed, placed_for_module_file,
    unready_branch_result, CallGraphFetchError, CallGraphSummary, CompleteBranchError,
};
use super::v13_read_modes::{
    attach_can_with_color, operation_node_with, search_scope_route, source_backed, SearchScopeRoute,
};
use crate::application::tool_contracts::SurfaceRelease;
use crate::application::v13::apply::parse_request as parse_apply_request;
use crate::application::v13::check::{DEFAULT_CHECK_LIMIT, MAX_CHECK_LIMIT};
use crate::application::v13::diff::{
    DiffError, DiffHandler, DiffPage, DiffRequest, DiffSource, DEFAULT_DIFF_LIMIT, MAX_DIFF_LIMIT,
};
use crate::application::v13::find::{public_name_match, FindIndex, FindRequest, NameRanker};
use crate::application::v13::resolve::{
    LinePlace, ModuleLine, PathLine, ResolveError, ResolveRequest, ResolvedLines, ResolvedPath,
    ResolvedSource,
};
use crate::application::v13::tool_catalog::catalog_for;
use crate::domain::apply::UNWRITABLE_SOURCE_MESSAGE;
use unica_execution::receipt_runtime::ports::ExecutionClass;
use {
    crate::application::v13::view::ViewReadAuthority, crate::application::v13::view::ViewRequest,
    crate::application::v13::view::ViewService, unica_execution_contract::view_error::ViewError,
};

use crate::domain::call_graph_identity::{CallGraphIdentity, CallGraphIdentityError};
use crate::infrastructure::capacity_observation::CapacityObserver;
use crate::infrastructure::v13_find::{
    FindBuildError, LayoutFindSource, WorkspaceFindDirectoryBuilder,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_code_intelligence::model::CallGraphState;
use unica_code_intelligence::model::CodeIntelligenceContext;
use unica_code_intelligence::model::CodeIntelligenceRegistry;
use unica_code_intelligence::model::ProviderRole;
use unica_code_intelligence::model::SearchRequest;
use unica_execution::invocation_store::ToolIdentity;
use unica_execution_contract::invocation::{DomainResult, InvocationFailure};
use unica_navigation::result_store::{
    SearchAfter, SearchCursorBinding, SearchCursorError, SearchCursorStore, SearchResume,
    StoredSearchCursor, ViewCursorBinding, ViewCursorStore,
};
use unica_navigation::source_navigation::{
    SOURCE_NAVIGATION_LIMIT_DEFAULT, SOURCE_NAVIGATION_LIMIT_MAX,
};
use unica_refusal::{RefusalCode, RefusalDetail};
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyStagingErrorKind, PlannedApplyEffects,
    StagedChangeKind, StagedFileState,
};
use unica_source_contract::SourceSetKind;
use unica_source_designer::layout::metadata_kind;
use unica_templates::TemplateFormat;
use unica_work_control::cancellation::CancellationToken;
use unica_workspace::actor::{
    ApplyAdmissionError, ApplyEffectDisposition, ApplyPublicationErrorKind,
    SavedApplyExecutionError,
};
use unica_workspace::apply::ApplyStagedState;

/// Keep the point-proven external placement for subsequent batch line reads.
/// This is invocation-local evidence, not a complete external inventory.
struct LocatedSource {
    source: ResolvedSource,
    external_binding: Option<unica_source_designer::external_artifact::ExternalArtifactBinding>,
}

/// Canonical v0.13 service installed by the production v3 daemon composition.
/// Each public name has a useful closed mode; unfinished variants fail with a
/// typed `unsupported_*` result rather than pretending an engine is missing.
pub(crate) struct CanonicalV13ReadService {
    cursors: Arc<ViewCursorStore>,
    search_cursors: Arc<SearchCursorStore>,
    diff: Arc<DiffHandler>,
    find_builder: WorkspaceFindDirectoryBuilder,
    /// Порты приложения живут столько же, сколько служба, а не сколько вызов.
    /// Внутри них стол доставок: он принадлежит серверу и переживает вызов,
    /// который доставку начал, чтобы её получил следующий. Порты на каждый
    /// вызов означали бы стол на каждый вызов — и доставки перестали бы
    /// делиться.
    ports: Arc<crate::infrastructure::application_ports::InfrastructureApplicationPorts>,
    #[cfg(test)]
    search_providers:
        Option<Vec<Arc<dyn unica_code_intelligence::model::CodeIntelligenceProvider>>>,
    #[cfg(test)]
    text_engine: Option<TextEngine>,
    #[cfg(test)]
    walk_budget: Option<std::time::Duration>,
    #[cfg(test)]
    cancel_past_page: bool,
}

/// Снимки продолжений лежат во временном файле: в памяти процесса остаются
/// узлы и границы элементов.
pub(crate) fn product_view_cursors() -> ViewCursorStore {
    use crate::infrastructure::{TempFileSpill, VIEW_SNAPSHOT_MAX_RETAINED_BYTES};
    ViewCursorStore::new(VIEW_SNAPSHOT_MAX_RETAINED_BYTES)
        .with_spill(Arc::new(TempFileSpill::default()))
}

impl Default for CanonicalV13ReadService {
    fn default() -> Self {
        Self {
            cursors: Arc::new(product_view_cursors()),
            search_cursors: Arc::new(SearchCursorStore::default()),
            diff: Arc::new(DiffHandler::default()),
            find_builder: WorkspaceFindDirectoryBuilder::default(),
            ports: Arc::new(
                crate::infrastructure::application_ports::InfrastructureApplicationPorts::new(),
            ),
            #[cfg(test)]
            search_providers: None,
            #[cfg(test)]
            text_engine: None,
            #[cfg(test)]
            walk_budget: None,
            #[cfg(test)]
            cancel_past_page: false,
        }
    }
}

impl CanonicalInvocationService for CanonicalV13ReadService {
    fn check_snapshots(&self) -> Option<Arc<ViewCursorStore>> {
        Some(Arc::clone(&self.cursors))
    }

    fn prepare(
        &self,
        invocation: &ActorBoundInvocation,
    ) -> Result<ExecutionClass, Box<DomainResult>> {
        if invocation.tool() == ToolIdentity::Search {
            super::v13_search_admission::admit_search_call(invocation, &self.search_cursors)?;
        }
        Ok(ExecutionClass::InlineCandidate)
    }

    fn execute(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure> {
        if let Some(rejection) = invocation.rejected_logical_read_result() {
            return Ok(rejection);
        }
        let mut result = match invocation.tool() {
            ToolIdentity::View => Ok(self.execute_view(invocation, &cancellation)),
            ToolIdentity::Apply => Ok(self.execute_apply(invocation, &cancellation)),
            ToolIdentity::Resolve => Ok(self.execute_resolve(invocation, &cancellation)),
            ToolIdentity::Search => Ok(self.execute_search(invocation, &cancellation)),
            ToolIdentity::Check => Ok(self.execute_check(invocation, &cancellation)),
            ToolIdentity::Diff => Ok(self.execute_diff(invocation, &cancellation)),
            ToolIdentity::Run => Ok(self.execute_run(invocation, &cancellation)),
            // Справка отвечает до допуска рабочей области: сюда вызов не
            // приходит, и второго её исполнения тут нет. Ветка закрыта
            // отказом, а не паникой: неверная маршрутизация обязана быть
            // видна вызывающему, а не ронять рабочий поток демона.
            ToolIdentity::Docs => Ok(error_result(
                None,
                RefusalCode::InvalidState,
                "docs is answered before workspace admission and does not reach the actor-bound read service",
            )),
        }?;
        if matches!(
            invocation.tool(),
            ToolIdentity::View
                | ToolIdentity::Resolve
                | ToolIdentity::Search
                | ToolIdentity::Check
                | ToolIdentity::Diff
        ) {
            result.rev = None;
        }
        Ok(result)
    }
}

#[cfg(test)]
std::thread_local! {
    static VIEW_CAPABILITY_PROBE_FOR_TEST: std::cell::RefCell<Option<(String, Vec<String>)>> = const { std::cell::RefCell::new(None) };
}

/// A thread-local observer restores the previous observer even after a panic.
#[cfg(test)]
pub(super) struct ViewCapabilityProbeForTest {
    previous: Option<(String, Vec<String>)>,
}

#[cfg(test)]
impl ViewCapabilityProbeForTest {
    pub(super) fn start(at: &str) -> Self {
        Self {
            previous: VIEW_CAPABILITY_PROBE_FOR_TEST
                .with(|slot| slot.replace(Some((at.to_owned(), Vec::new())))),
        }
    }

    pub(super) fn revisions(&self) -> Vec<String> {
        VIEW_CAPABILITY_PROBE_FOR_TEST.with(|slot| {
            slot.borrow()
                .as_ref()
                .expect("capability observer is active")
                .1
                .clone()
        })
    }
}

#[cfg(test)]
impl Drop for ViewCapabilityProbeForTest {
    fn drop(&mut self) {
        VIEW_CAPABILITY_PROBE_FOR_TEST.with(|slot| {
            slot.replace(self.previous.take());
        });
    }
}

#[cfg(test)]
fn observe_fresh_view_capability_for_test(at: &str, revision: impl FnOnce() -> String) {
    VIEW_CAPABILITY_PROBE_FOR_TEST.with(|slot| {
        if let Some((target, revisions)) = slot.borrow_mut().as_mut() {
            if target == at {
                revisions.push(revision());
            }
        }
    });
}

mod apply_tool;
mod support_choice;
#[cfg(test)]
pub(super) use apply_tool::with_argument_dictionary;
#[cfg(test)]
pub(super) use apply_tool::with_unknown_argument_contract;
mod check_tool;
mod resolve_tool;
mod search_tool;
mod view_tool;

impl CanonicalV13ReadService {
    #[cfg(test)]
    pub(super) fn with_view_cursor_budget_for_test(max_total_bytes: usize) -> Self {
        Self {
            cursors: Arc::new(
                ViewCursorStore::new(max_total_bytes)
                    .with_spill(Arc::new(crate::infrastructure::TempFileSpill::default())),
            ),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(super) fn retained_view_usage_for_test(
        &self,
    ) -> unica_navigation::result_store::ViewRetainedUsage {
        self.cursors.retained_usage_for_test()
    }

    #[cfg(test)]
    pub(super) fn retained_view_binding_ids_for_test(&self) -> Vec<String> {
        self.cursors.retained_binding_ids_for_test()
    }

    #[cfg(test)]
    pub(super) fn retained_view_payload_identities_for_test(&self) -> Vec<usize> {
        self.cursors.retained_payload_identities_for_test()
    }

    pub(crate) fn with_capacity_observer(observer: Arc<CapacityObserver>) -> Self {
        let mut service = Self::default();
        service.cursors = Arc::new(product_view_cursors().with_capacity_observer(observer.clone()));
        service.find_builder = service.find_builder.with_capacity_observer(observer);
        service
    }

    #[cfg(test)]
    pub(super) fn with_search_providers_for_test(
        providers: Vec<Arc<dyn unica_code_intelligence::model::CodeIntelligenceProvider>>,
    ) -> Self {
        Self {
            search_providers: Some(providers),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(super) fn with_text_engine_for_test(engine: TextEngine) -> Self {
        Self {
            text_engine: Some(engine),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(super) fn with_directory_limit_for_test(max_documents: usize) -> Self {
        Self {
            find_builder: WorkspaceFindDirectoryBuilder::with_document_limit(max_documents),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(super) fn with_directory_fact_limit_for_test(max_fact_bytes: usize) -> Self {
        Self {
            find_builder: WorkspaceFindDirectoryBuilder::with_fact_byte_limit_for_test(
                max_fact_bytes,
            ),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(super) fn with_name_read_fault_for_test(
        relative: &'static str,
        kind: std::io::ErrorKind,
    ) -> Self {
        let mut service = Self::default();
        service.find_builder = service
            .find_builder
            .with_local_read_fault_for_test(relative, kind);
        service
    }
}

/// The admitted read capability of one source set, or the refusal that the
/// layout readers answer when the actor did not admit it.
fn admitted_read_source<T>(
    sources: Result<Vec<T>, String>,
    source_set_name: impl Fn(&T) -> &str,
    at: Option<&str>,
    source_set: &str,
) -> Result<T, Box<DomainResult>> {
    let sources = sources.map_err(|error| {
        Box::new(error_result(
            at.map(str::to_string),
            RefusalCode::ProviderUnavailable,
            error,
        ))
    })?;
    sources
        .into_iter()
        .find(|source| source_set_name(source) == source_set)
        .ok_or_else(|| {
            Box::new(error_result(
                at.map(str::to_string),
                RefusalCode::ProviderUnavailable,
                format!("source set `{source_set}` was not admitted by the workspace actor"),
            ))
        })
}

/// The layout place of a metadata object address in its own source set:
/// `Configuration.xml`, an owner descriptor, or a form, template or command
/// of a top-level owner. Other shapes keep the full directory lookup.
fn metadata_layout_path(address: &QualifiedAddress, source_kind: SourceSetKind) -> Option<String> {
    let segments = address.segments();
    if let [root] = segments {
        if root.kind() == NodeKind::Configuration && root.name().is_none() {
            return matches!(
                source_kind,
                SourceSetKind::Configuration | SourceSetKind::Extension
            )
            .then(|| "Configuration.xml".to_string());
        }
    }
    let target = address.to_string();
    let target = target.split_once(':')?.1;
    let target = unica_address::source_target::MetadataAddress::parse_logical(target).ok()?;
    let relative =
        unica_source_designer::layout::metadata_descriptor_relative(&target, source_kind).ok()?;
    let relative = relative.to_str()?.replace('\\', "/");
    // A command has no descriptor: the layout places its directory.
    if matches!(segments.last(), Some(last) if last.kind() == NodeKind::Command) {
        return relative.strip_suffix(".xml").map(str::to_string);
    }
    Some(relative)
}

/// Поставить движок маршрута, если его нет, и убедиться, что он на месте.
///
/// Незавершённая поставка отвечает `working` с повтором, неудачная — своим
/// классом с полями движка; движок, которого после поставки нет, — общим
/// отказом «нет поставляемого движка» (#549). Поставку начинает сам вызов,
/// поэтому повтор того же вызова, который советует отказ, её и завершает.
/// Контрольная сумма здесь не считается: её сверяет
/// [`engine_absence_refusal`] на пути отказа.
fn acquire_engine(
    invocation: &ActorBoundExecution,
    cwd: &std::path::Path,
    engine: &str,
    at: Option<String>,
    cancellation: &CancellationToken,
) -> Result<(), Box<DomainResult>> {
    let Some(plugin_root) = crate::infrastructure::plugin_runtime::find_plugin_root(cwd) else {
        return Err(Box::new(
            crate::infrastructure::bundled_tools::plugin_root_missing(engine).refusal(at),
        ));
    };
    if let Some(mut result) = deliver_missing_engine(invocation, &plugin_root, engine, cancellation)
    {
        result.at = at;
        return Err(Box::new(result));
    }
    if let Some(missing) =
        crate::infrastructure::bundled_tools::missing_engine(&plugin_root, engine)
    {
        return Err(Box::new(missing.refusal(at)));
    }
    Ok(())
}

/// Delivers an engine the call needs before it starts its provider. `None`
/// means the engine is in place or this installation has nothing to deliver;
/// otherwise the answer is the delivery's typed state — still working,
/// failed with its class and the engine's fields — or the caller's
/// cancellation.
fn deliver_missing_engine(
    invocation: &ActorBoundExecution,
    plugin_root: &std::path::Path,
    engine: &str,
    cancellation: &CancellationToken,
) -> Option<DomainResult> {
    let progress = unica_artifact_delivery::desk::CanonicalDeliveryProgress::default();
    let state = crate::infrastructure::engine_delivery::deliver_if_missing(
        invocation.delivery_work(),
        plugin_root,
        engine,
        cancellation,
        &progress,
    );
    if cancellation.is_cancelled() {
        return Some(error_result(
            None,
            RefusalCode::Cancelled,
            format!("cancelled while waiting for {engine} delivery"),
        ));
    }
    crate::infrastructure::engine_delivery::canonical_engine_delivery_result(
        state,
        &progress,
        plugin_root,
        engine,
    )
}

/// Отказ «нет поставляемого движка» после неудачного прогона поставщика:
/// движок, который есть на диске, но не прошёл полную проверку поставки
/// (контрольную сумму), называется тем же отказом, что и отсутствующий.
/// Решает тип отказа резолвера, а не текст ошибки поставщика.
fn engine_absence_refusal(
    cwd: &std::path::Path,
    engine: &str,
    at: Option<String>,
    cancellation: &CancellationToken,
) -> Option<DomainResult> {
    if cancellation.is_cancelled() {
        return None;
    }
    let missing = match crate::infrastructure::plugin_runtime::find_plugin_root(cwd) {
        None => crate::infrastructure::bundled_tools::plugin_root_missing(engine),
        Some(plugin_root) => {
            crate::infrastructure::bundled_tools::engine_absence(&plugin_root, engine)?
        }
    };
    Some(missing.refusal(at))
}

/// Отказ «нет поставляемого движка» после неудачного прогона поставщика,
/// который запускает движок в службе рабочего пространства: шаг поставки
/// называет резолвер, а проверенный движок, процесс которого ОС не создаёт,
/// — шаг `start`. Иначе это не отсутствие движка (#549).
fn engine_failure_refusal(
    cwd: &std::path::Path,
    engine: &str,
    at: Option<String>,
    cancellation: &CancellationToken,
) -> Option<DomainResult> {
    if let Some(refusal) = engine_absence_refusal(cwd, engine, at.clone(), cancellation) {
        return Some(refusal);
    }
    if cancellation.is_cancelled() {
        return None;
    }
    let plugin_root = crate::infrastructure::plugin_runtime::find_plugin_root(cwd)?;
    let (missing, cause) =
        crate::infrastructure::bundled_tools::launch_probe_failure(&plugin_root, engine)?;
    Some(missing.refusal_with_cause(at, &cause))
}

/// Движок диагностик BSL, поиска символов и графа вызовов.
const BSL_ANALYZER_ENGINE: &str = "bsl-analyzer";

fn selected_role_engine(role: ProviderRole) -> Option<&'static str> {
    match role {
        ProviderRole::Lexical => None,
        ProviderRole::Symbol => Some(BSL_ANALYZER_ENGINE),
        // Поставщика смысловой роли в поставке нет: отказ называет реестр.
        ProviderRole::Semantic => None,
    }
}

/// Почему роль некому обслужить. Роль остаётся в контракте `unica.search`, и её
/// получит поставщик, вошедший в реестр.
fn no_search_provider_message(role: ProviderRole) -> String {
    match role {
        ProviderRole::Semantic => "semantic search has no provider: it needs an embedding service \
             (EMBEDDING_URL) and an adapter for it; use role `symbol` or a literal search"
            .to_string(),
        _ => format!("no search provider is registered for `{}`", role.as_str()),
    }
}

fn search_selected_role(
    registry: CodeIntelligenceRegistry,
    role: ProviderRole,
    request: &SearchRequest,
    context: &CodeIntelligenceContext,
    cancellation: &CancellationToken,
    deadlines: unica_code_intelligence::coordinator::CodeIntelligenceDeadlines,
) -> Result<unica_code_intelligence::coordinator::CodeSearchExecution, String> {
    use unica_code_intelligence::coordinator::CodeSearchCoordinator;

    let provider = registry
        .search_providers()
        .find(|provider| provider.identity().role == role)
        .cloned()
        .ok_or_else(|| no_search_provider_message(role))?;
    let selected = CodeIntelligenceRegistry::new(vec![provider])?;
    CodeSearchCoordinator::with_deadlines(selected, deadlines).search_observed(
        request,
        context,
        cancellation,
        &unica_work_control::progress::NoopProgressSink,
    )
}

/// Ошибка движка может содержать stderr и физические пути. Причину берём
/// только из типизированной остановки, а данные отказа собираем явно.
fn provider_search_failure(
    section: unica_code_intelligence::model::ProviderSearchSection,
) -> DomainResult {
    use unica_code_intelligence::model::SearchTerminationCode;

    let mut result = match section.termination.as_ref().map(|reason| reason.code) {
        Some(SearchTerminationCode::DeadlineExceeded) => error_result(
            None,
            RefusalCode::DeadlineExceeded,
            "search provider deadline exceeded",
        ),
        Some(SearchTerminationCode::DependencyPending) => error_result_detailed(
            None,
            RefusalDetail::DependencyPending,
            "search provider dependency is not ready",
        ),
        Some(SearchTerminationCode::UnsupportedScope) => error_result(
            None,
            RefusalCode::UnsupportedScope,
            "search provider does not support this scope",
        ),
        Some(SearchTerminationCode::CapacityExhausted) => error_result_detailed(
            None,
            RefusalDetail::BackendBusy,
            "search provider execution capacity is busy",
        ),
        Some(SearchTerminationCode::ProviderUnavailable) => error_result(
            None,
            RefusalCode::ProviderUnavailable,
            "search provider is unavailable",
        ),
        Some(SearchTerminationCode::ProviderFailed) => {
            error_result(None, RefusalCode::ProviderFailed, "search provider failed")
        }
        Some(SearchTerminationCode::LimitReached) => error_result(
            None,
            RefusalCode::ProviderLimitExceeded,
            "search provider reached its result limit",
        ),
        None => error_result(
            None,
            RefusalCode::ProviderFailed,
            "search provider returned no terminal reason",
        ),
    };
    let mut termination = section.termination;
    if let Some(reason) = &mut termination {
        reason.detail_code = reason.detail_code.take().filter(|detail| {
            reason.code == SearchTerminationCode::DependencyPending
                && matches!(
                    detail.as_str(),
                    "buildingIndex" | "updatingIndex" | "indexQueued"
                )
        });
    }
    let mut evidence = json!({
        "role": section.identity.role,
        "provider": section.identity.provider,
        "status": section.status,
        "searchComplete": section.search_complete,
        "matches": section.matches,
        "indexFreshness": "unknown",
    });
    if let Some(termination) = termination {
        evidence["termination"] = json!(termination);
    }
    if let Some(build_id) = section.index_build_id {
        evidence["indexBuildId"] = json!(build_id);
    }
    if let Some(maintenance) = section.index_maintenance {
        evidence["indexMaintenance"] = json!(maintenance);
    }
    result.data = Some(json!({"mode": section.identity.role, "matches": [evidence]}));
    result
}

#[cfg(test)]
pub(crate) fn provider_search_page(
    cursors: &SearchCursorStore,
    role: ProviderRole,
    execution: unica_code_intelligence::coordinator::CodeSearchExecution,
    binding: SearchCursorBinding,
    cursor_token: Option<&str>,
    cancellation: &CancellationToken,
) -> DomainResult {
    provider_search_page_projected(
        cursors,
        role,
        execution,
        binding,
        cursor_token,
        cancellation,
        &|hits| Ok(vec![Map::new(); hits.len()]),
    )
}

fn provider_search_page_projected<F>(
    cursors: &SearchCursorStore,
    role: ProviderRole,
    execution: unica_code_intelligence::coordinator::CodeSearchExecution,
    mut binding: SearchCursorBinding,
    cursor_token: Option<&str>,
    cancellation: &CancellationToken,
    project_hits: &F,
) -> DomainResult
where
    F: Fn(
        &[unica_code_intelligence::model::ProviderSearchHit],
    ) -> Result<Vec<Map<String, Value>>, ViewError>,
{
    use crate::application::v13::view::PREFERRED_PAGE_BYTES;
    use unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES;

    if cancellation.is_cancelled() {
        return error_result(
            None,
            RefusalCode::Cancelled,
            "provider search page cancelled",
        );
    }
    let mut sections = execution.result.sections;
    if sections.len() != 1 {
        return error_result(
            None,
            RefusalCode::ProviderFailed,
            "selected search provider did not return exactly one section",
        );
    }
    let section = sections.remove(0);
    if section.identity.role != role {
        return error_result(
            None,
            RefusalCode::ProviderFailed,
            "selected search provider returned a section for another role",
        );
    }
    if !execution.ok {
        // Пустой неуспешный ответ не доказывает отсутствие совпадений.
        return provider_search_failure(section);
    }
    let mut warnings = execution.warnings;
    if section.hits.len() >= PROVIDER_SEARCH_FETCH_LIMIT {
        warnings.push(
            "Search returned 200 matches. Ask the user to narrow the query before searching again."
                .to_string(),
        );
    }
    let summary = if section.search_complete {
        format!("{} search completed", role.as_str())
    } else {
        format!("{} search returned incomplete results", role.as_str())
    };
    // Providers expose top-N only. Re-query the same finite window on a later
    // page and bind the cursor to every fact returned by that provider; an
    // index refresh must make the old cursor stale instead of moving hits.
    let mut section_value =
        serde_json::to_value(&section).expect("provider search section is serializable");
    let projections = match project_hits(&section.hits) {
        Ok(projections) if projections.len() == section.hits.len() => projections,
        Ok(_) => {
            return error_result(
                None,
                RefusalCode::InvalidResult,
                "search hit projection count changed",
            )
        }
        Err(error) => return view_error_result(None, error),
    };
    for (extra, value) in projections.into_iter().zip(
        section_value["hits"]
            .as_array_mut()
            .expect("provider hits array"),
    ) {
        value
            .as_object_mut()
            .expect("provider hit object")
            .extend(extra);
    }
    let fingerprint_bytes = serde_json::to_vec(&(&section_value, &warnings))
        .expect("provider search evidence is serializable");
    let mut hasher = Sha256::new();
    hasher.update(b"unica-v13-provider-search-v1\0");
    hasher.update(&fingerprint_bytes);
    binding.result_fingerprint = Some(format!("provider-sha256-v1:{:x}", hasher.finalize()));
    let hits = std::mem::take(
        section_value["hits"]
            .as_array_mut()
            .expect("provider search hits serialize as an array"),
    );
    // The cursor names the previous complete provider answer. Verify that
    // identity before inspecting sizes in a different answer; changed late
    // results must report stale_cursor after pages were already published.
    let cursor = match cursor_token {
        None => None,
        Some(token) => match cursors.read(token, &binding) {
            Ok(cursor) => Some((token, cursor)),
            Err(error) => return error.into_result(None),
        },
    };
    let cursor_placeholder = SearchCursorStore::token_probe();
    let probe = |page_hits: &[Value]| {
        serde_json::to_vec(&provider_search_page_result(
            role,
            &section_value,
            page_hits,
            &warnings,
            &summary,
            "limit",
            Some(cursor_placeholder.as_str()),
        ))
        .expect("provider search page is serializable")
        .len()
    };
    if probe(&[]) > MAX_CANONICAL_RESULT_BYTES {
        return error_result(
            None,
            RefusalCode::ResultTooLarge,
            "provider search page metadata exceeds the transport limit",
        );
    }
    for hit in &hits {
        if cancellation.is_cancelled() {
            return error_result(
                None,
                RefusalCode::Cancelled,
                "provider search page cancelled",
            );
        }
        if probe(std::slice::from_ref(hit)) > MAX_CANONICAL_RESULT_BYTES {
            return error_result(
                None,
                RefusalCode::ResultTooLarge,
                "one provider search result exceeds the transport limit",
            );
        }
    }
    let offset = cursor.as_ref().map_or(0, |(_, stored)| stored.offset);
    if offset >= hits.len() && cursor.is_some() {
        return error_result(
            None,
            RefusalCode::InvalidCursor,
            "provider search cursor is invalid",
        );
    }
    let mut page_hits = Vec::new();
    let mut byte_stop = false;
    for hit in hits.iter().skip(offset).take(binding.page_limit) {
        let mut candidate = page_hits.clone();
        candidate.push(hit.clone());
        let bytes = probe(&candidate);
        if bytes > MAX_CANONICAL_RESULT_BYTES {
            byte_stop = true;
            break;
        }
        if bytes > PREFERRED_PAGE_BYTES && !page_hits.is_empty() {
            byte_stop = true;
            break;
        }
        page_hits.push(hit.clone());
    }
    let next_offset = match SearchCursorStore::next_offset(offset, page_hits.len()) {
        Ok(offset) => offset,
        Err(error) => return error.into_result(None),
    };
    let more = next_offset < hits.len();
    let stopped_by = if !more {
        "complete"
    } else if byte_stop {
        "bytes"
    } else {
        "limit"
    };
    if cancellation.is_cancelled() {
        return error_result(
            None,
            RefusalCode::Cancelled,
            "provider search page cancelled",
        );
    }
    let mut result = provider_search_page_result(
        role,
        &section_value,
        &page_hits,
        &warnings,
        &summary,
        stopped_by,
        None,
    );
    if more {
        let issued = match cursor {
            Some((_, stored)) => cursors.insert_next(&stored, next_offset),
            None => cursors.insert_first(binding, next_offset),
        };
        let issued = match issued {
            Ok(token) => token,
            Err(error) => return error.into_result(None),
        };
        result.cursor = Some(issued);
    }
    result
}

fn provider_search_page_result(
    role: ProviderRole,
    section: &Value,
    hits: &[Value],
    warnings: &[String],
    summary: &str,
    stopped_by: &str,
    cursor: Option<&str>,
) -> DomainResult {
    let mut projected = section.clone();
    projected["hits"] = Value::Array(hits.to_vec());
    // `returned` describes this public page. `total` and `relation` retain
    // the provider's exact or lower-bound claim about its whole search.
    projected["matches"]["returned"] = json!(hits.len());
    let mut result = DomainResult::success(summary);
    result.data = Some(json!({"mode": role.as_str(), "matches": [projected]}));
    result.page = Some(json!({"stoppedBy": stopped_by}));
    result
        .warnings
        .extend(warnings.iter().cloned().map(Value::String));
    result.cursor = cursor.map(str::to_owned);
    result
}

/// Раскладка допущенных наборов для пути, пришедшего снаружи. Её
/// недоступность — отказ всего вызова, а не каждого элемента пакета.
fn with_resolve_layout<T>(
    invocation: &ActorBoundExecution,
    answer: impl FnOnce(&[LayoutFindSource<'_>], ProviderDeadline) -> T,
) -> Result<T, Box<DomainResult>> {
    let sources = invocation
        .layout_sources()
        .map_err(|error| Box::new(error_result(None, RefusalCode::ProviderUnavailable, error)))?;
    let Some(deadline) = sources.first().map(|source| source.deadline()) else {
        return Err(Box::new(error_result_detailed(
            None,
            RefusalDetail::ProviderAbsent,
            "resolve has no admitted source sets",
        )));
    };
    let layout = sources
        .iter()
        .map(|source| LayoutFindSource::new(source.name(), source.kind(), source.root()))
        .collect::<Vec<_>>();
    Ok(answer(&layout, deadline))
}

/// Отказ одного элемента пакета: тот же код, уточнение и адрес, что дал бы
/// одиночный вызов, без исхода, который код и так определяет.
fn batch_refusal(refusal: &DomainResult) -> Value {
    let diagnostic = refusal.diagnostics.first().cloned().unwrap_or_default();
    let mut item = Map::new();
    for key in ["code", "detailCode", "message"] {
        if let Some(value) = diagnostic.get(key) {
            item.insert(key.to_string(), value.clone());
        }
    }
    if let Some(at) = &refusal.at {
        item.insert("at".to_string(), json!(at));
    }
    Value::Object(item)
}

/// Отмена и срок относятся ко всему вызову, а не к элементу пакета.
fn call_stopping_refusal(refusal: &DomainResult) -> bool {
    refusal
        .diagnostics
        .first()
        .and_then(|diagnostic| diagnostic.get("code"))
        .and_then(Value::as_str)
        .is_some_and(|code| {
            code == RefusalCode::Cancelled.as_str()
                || code == RefusalCode::DeadlineExceeded.as_str()
        })
}

/// Ответ моста: один точный предмет и ничего больше.
fn resolve_result(resolved: ResolvedSource) -> DomainResult {
    let mut result = DomainResult::success("source location resolved");
    result.at = Some(resolved.address().to_string());
    result.data = Some(
        serde_json::to_value(resolved).expect("the closed resolve model always serializes to JSON"),
    );
    // Справочник раскладки снимком ревизии не является, поэтому `rev` нет.
    // И `next` отсюда не ведёт никуда: аварийный выход не следующий вопрос
    // ни в одном маршруте.
    result
}

/// Владелец модуля, который размещает раскладка. Граф вызовов по нему
/// находит файл BSL метода: метод, область и тело лежат внутри этого файла.
fn owning_metadata_address(address: &QualifiedAddress) -> String {
    let segments = address.segments();
    let owned = segments
        .iter()
        .position(|segment| {
            matches!(
                segment.kind(),
                NodeKind::Module | NodeKind::Method | NodeKind::Region | NodeKind::Body
            )
        })
        .unwrap_or(segments.len());
    let mut owner = String::from(address.source_set());
    owner.push(':');
    for (index, segment) in segments.iter().take(owned.max(1)).enumerate() {
        if index > 0 {
            owner.push('.');
        }
        owner.push_str(segment.kind().as_str());
        if let Some(name) = segment.name() {
            owner.push('.');
            owner.push_str(name);
        }
    }
    owner
}

/// Every family section of plan details that an apply answer may carry.
const PLAN_DETAIL_KEYS: &[unica_source_contract::proposal::PlanDetailKey] = &[
    unica_xdto::results::XDTO_OBJECTS,
    unica_metadata::EXCHANGE_CONTENT,
    unica_metadata::BORROWING,
];

/// Fields the apply answer writes itself; a family section cannot take them.
const APPLY_DATA_FIELDS: &[&str] = &[
    "validated",
    "mode",
    "executable",
    "operationCount",
    "operations",
    "planHash",
    "effects",
    "cache",
    "executionToken",
];

/// A section key two families share, or one that shadows the answer's own
/// field, would overwrite data in the published answer.
fn plan_detail_key_errors(keys: &[unica_source_contract::proposal::PlanDetailKey]) -> Vec<String> {
    let mut errors = Vec::new();
    for (index, key) in keys.iter().enumerate() {
        if keys[..index].contains(key) {
            errors.push(format!(
                "plan detail section `{}` is registered twice",
                key.as_str()
            ));
        }
        if APPLY_DATA_FIELDS.contains(&key.as_str()) {
            errors.push(format!(
                "plan detail section `{}` shadows an apply answer field",
                key.as_str()
            ));
        }
    }
    errors
}

fn apply_plan_hash(staged: &ApplyStagedState, effects: &PlannedApplyEffects) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"unica-v13-apply-plan-v1\0");
    for change in staged.planned_changes() {
        hasher.update(change.relative_path.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update([match change.kind {
            StagedChangeKind::Create => 1,
            StagedChangeKind::Replace => 2,
            StagedChangeKind::Remove => 3,
        }]);
        match change.current {
            StagedFileState::Bytes(bytes) => {
                hasher.update([1]);
                hasher.update((bytes.len() as u64).to_be_bytes());
                hasher.update(bytes);
            }
            StagedFileState::Absent => hasher.update([0]),
        }
    }
    for event in effects.events() {
        hasher.update(event.name().as_bytes());
        hasher.update([0]);
        hasher.update(event.artifact.as_bytes());
        hasher.update([0]);
    }
    format!("sha256:{:x}", hasher.finalize())
}

/// Projects one page of a left/right comparison. A cut page names its
/// continuation; the cursor answers only the same pair, page size and
/// workspace.
fn diff_page_result(request: &DiffRequest, page: Result<DiffPage, DiffError>) -> DomainResult {
    let page = match page {
        Ok(page) => page,
        Err(error @ DiffError::ChangeTooLarge) => {
            return error_result(
                Some(request.left().to_string()),
                error.code(),
                error.to_string(),
            )
        }
        Err(error) => return error_result(None, error.code(), error.to_string()),
    };
    let limit = request.limit();
    let mut result = if let Some(cursor) = page.cursor() {
        let mut result = DomainResult::success(
            "logical nodes compared; more differences remain; continue with cursor",
        );
        result.cursor = Some(cursor.to_owned());
        result.next.push(json!({"tool": "unica.diff", "arguments": {
            "left": request.left().to_string(),
            "right": request.right().to_string(),
            "limit": limit,
            "cursor": cursor,
        }}));
        result
    } else {
        DomainResult::success("logical nodes compared")
    };
    result.page = Some(json!({"stoppedBy": page.stopped_by()}));
    result.data = Some(serde_json::to_value(&page).expect("diff page serializes"));
    result
}

fn find_build_error_result(at: Option<String>, error: FindBuildError) -> DomainResult {
    match error.detail() {
        Some(detail) => error_result_detailed(at, detail, error.to_string()),
        None => error_result(at, error.code(), error.to_string()),
    }
}

/// The question binding of a text search: its matching mode and whether
/// case is ignored. Admission must build the same string.
pub(super) fn text_search_binding_mode(mode: &str, ignore_case: bool) -> String {
    if ignore_case {
        format!("{mode}:ignoreCase")
    } else {
        mode.to_owned()
    }
}

fn search_page_data(mode: &str, matches: &[Value], extra: &Map<String, Value>) -> Value {
    let mut data = extra.clone();
    data.insert("mode".to_owned(), Value::String(mode.to_owned()));
    data.insert("matches".to_owned(), Value::Array(matches.to_vec()));
    Value::Object(data)
}

fn search_page_probe_bytes(
    mode: &str,
    matches: &[Value],
    summary: &str,
    extra_data: &Map<String, Value>,
    revision: Option<&str>,
) -> usize {
    search_page_probe_bytes_with(
        mode,
        matches,
        summary,
        extra_data,
        revision,
        &SearchCursorStore::token_probe(),
    )
}

fn search_page_probe_bytes_with(
    mode: &str,
    matches: &[Value],
    summary: &str,
    extra_data: &Map<String, Value>,
    revision: Option<&str>,
    cursor_probe: &str,
) -> usize {
    let mut probe = DomainResult::success(summary.to_owned());
    probe.data = Some(search_page_data(mode, matches, extra_data));
    // "complete" is the longest terminal reason; a cursor also reserves
    // space even when this turns out to be the final page.
    probe.page = Some(json!({"stoppedBy": "complete"}));
    probe.cursor = Some(cursor_probe.to_owned());
    probe.rev = revision.map(str::to_owned);
    serde_json::to_vec(&probe).map_or(usize::MAX, |value| value.len())
}

fn name_search_interruption(
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Option<DomainResult> {
    if cancellation.is_cancelled() {
        return Some(error_result(
            None,
            RefusalCode::Cancelled,
            "name search was cancelled",
        ));
    }
    if deadline.is_elapsed() {
        return Some(error_result(
            None,
            RefusalCode::DeadlineExceeded,
            "name search deadline elapsed",
        ));
    }
    None
}

pub(super) fn apply_plan_error_result(error: &ApplyPlanError) -> DomainResult {
    let at = error.path().map(str::to_string);
    let code = match error.kind() {
        ApplyPlanErrorKind::BadValue => RefusalCode::BadValue,
        ApplyPlanErrorKind::NotFound => RefusalCode::NotFound,
        ApplyPlanErrorKind::ProviderUnavailable => RefusalCode::ProviderUnavailable,
        ApplyPlanErrorKind::InvalidState => RefusalCode::InvalidState,
        ApplyPlanErrorKind::InvalidSource => RefusalCode::InvalidSource,
        ApplyPlanErrorKind::Staging(kind) => {
            return apply_staging_refusal(at, kind, error.to_string())
        }
        ApplyPlanErrorKind::Postcondition => RefusalCode::PostconditionFailed,
    };
    let mut result = error_result(at, code, error.to_string());
    result.next.extend(error.next().cloned());
    result
        .diagnostics
        .extend(error.diagnostics().iter().cloned());
    // The stored answer fits the largest client; each client cuts it to its
    // own budget when it is delivered.
    unica_execution_contract::source_text_budget::fit_refusal_diagnostics(
        &result,
        unica_bootstrap::MAX_RESPONSE_BUDGET,
    )
    .into_owned()
}

/// Отказ подготовки staged-исходников называет нарушенное предусловие.
///
/// Прежде любой вид сводился к `provider_unavailable` с исходом `needsHuman`,
/// и агент ждал поставщика там, где надо было создать файл, убрать ссылку
/// из исходников или подготовить новый предпросмотр (#955, #1332). Поставщик
/// здесь — файловая система набора: ошибка её чтения остаётся
/// `provider_unavailable`, но с уточнением `source_unreadable`.
fn apply_staging_refusal(
    at: Option<String>,
    kind: ApplyStagingErrorKind,
    message: String,
) -> DomainResult {
    let code = match kind {
        ApplyStagingErrorKind::Cancelled => RefusalCode::Cancelled,
        ApplyStagingErrorKind::Deadline => RefusalCode::DeadlineExceeded,
        ApplyStagingErrorKind::ConcurrentRevision => RefusalCode::StaleRevision,
        ApplyStagingErrorKind::ContainmentIdentity => RefusalCode::ContainmentDenied,
        ApplyStagingErrorKind::AbsentChainOccupied => RefusalCode::InvalidSource,
        ApplyStagingErrorKind::Invariant => RefusalCode::InvalidState,
        ApplyStagingErrorKind::UnsupportedProvider => {
            return error_result_detailed(at, RefusalDetail::SourceUnreadable, message)
        }
    };
    error_result(at, code, message)
}

fn saved_apply_execution_error_result(error: SavedApplyExecutionError) -> DomainResult {
    match error {
        SavedApplyExecutionError::Busy => error_result_detailed(
            None,
            RefusalDetail::BackendBusy,
            "saved plan is busy; retry after the current operation",
        ),
        SavedApplyExecutionError::Unavailable(message) => {
            error_result(None, RefusalCode::BadValue, message)
        }
        SavedApplyExecutionError::RegistryUnavailable => error_result(
            None,
            RefusalCode::InvalidState,
            "saved apply plans are unavailable",
        ),
        SavedApplyExecutionError::StateUnavailable(message) => {
            error_result(None, RefusalCode::InvalidState, message)
        }
        SavedApplyExecutionError::Publication(error) => error_result(
            None,
            apply_publication_error_code(error.kind()),
            error.to_string(),
        ),
    }
}

fn apply_publication_error_code(kind: ApplyPublicationErrorKind) -> RefusalCode {
    match kind {
        ApplyPublicationErrorKind::Cancelled => RefusalCode::Cancelled,
        ApplyPublicationErrorKind::Deadline => RefusalCode::DeadlineExceeded,
        ApplyPublicationErrorKind::ConcurrentRevision => RefusalCode::StaleRevision,
        // Publication re-proves the containment the plan established. A
        // failure means the sources changed physically after the plan (a
        // swapped directory, a link or alias that appeared, an occupied
        // chain); a new preview names the precise cause. The static causes
        // themselves are refused at planning by `apply_staging_refusal`.
        ApplyPublicationErrorKind::ContainmentIdentity => RefusalCode::StaleRevision,
        ApplyPublicationErrorKind::ProviderPostvalidation => RefusalCode::PostconditionFailed,
        ApplyPublicationErrorKind::SourceSelectionChanged => RefusalCode::SourceSelectionChanged,
        ApplyPublicationErrorKind::RollbackIncomplete => RefusalCode::RollbackIncomplete,
        ApplyPublicationErrorKind::Invariant => RefusalCode::ProviderUnavailable,
    }
}

fn bounded_usize(value: &Value) -> Option<usize> {
    usize::try_from(value.as_u64()?)
        .ok()
        .filter(|value| *value > 0)
}

/// Runs every validator a readable node owns and folds the verdicts into one
/// check envelope. The bridged validators keep the source of validation
/// truth; the plan follows from the node kind and the facts the projection
/// states, never from a caller choice. A node without validators reports
/// readability only.
/// Диагностика BSL через провайдер анализа.
///
/// Провайдеры собраны в продуктовом реестре давно, но канонический провод до
/// них не доходил: `check` отвечал «читается» и молчал о находках, ради
/// которых его и зовут. Порты здесь строятся свои, а недоставленный
/// анализатор доставляется через стол доставок демона до запуска анализа,
/// как у поиска с ролью.
///
/// На объекте метаданных и корне конфигурации шаг спрашивает находки только
/// у поставщиков, которые объявили такую цель. Анализатор BSL судит модули,
/// поэтому без такого поставщика шага нет (`None`): весь набор исходников
/// ради объекта не анализируется.
fn run_bsl_diagnostics(
    ports: &crate::infrastructure::application_ports::InfrastructureApplicationPorts,
    address: &QualifiedAddress,
    kind: &str,
    invocation: &ActorBoundExecution,
    cancellation: &CancellationToken,
) -> Result<Option<CheckStepVerdict>, Box<DomainResult>> {
    use crate::application::ports::ApplicationPorts;
    use unica_address::source_target::TargetKind;
    use unica_diagnostics::coordinator::DiagnosticCoordinator;
    use unica_diagnostics::model::DiagnosticAction;
    use unica_diagnostics::model::DiagnosticFilter;
    use unica_diagnostics::model::DiagnosticRequest;

    let target_kind = match kind {
        "Module" | "Body" => TargetKind::Module,
        "Configuration" => TargetKind::SourceRoot,
        _ => TargetKind::MetadataObject,
    };
    let registry = match ports.diagnostic_provider_registry() {
        Ok(registry) => registry,
        Err(error) => {
            return Err(Box::new(error_result(
                Some(address.to_string()),
                RefusalCode::ProviderUnavailable,
                error,
            )))
        }
    };
    let action = if target_kind == TargetKind::Module {
        DiagnosticAction::Analyze
    } else {
        DiagnosticAction::Findings
    };
    if target_kind != TargetKind::Module
        && !registry.descriptors().any(|descriptor| {
            descriptor.supports_action(action) && descriptor.supports_findings_target(target_kind)
        })
    {
        return Ok(None);
    }
    let context = invocation.workspace_context();
    let sources = invocation.read_sources().map_err(|error| {
        Box::new(error_result_detailed(
            Some(address.to_string()),
            RefusalDetail::SourceUnreadable,
            error,
        ))
    })?;
    let source = sources
        .into_iter()
        .find(|source| source.source_set_name() == address.source_set())
        .ok_or_else(|| {
            Box::new(error_result_detailed(
                Some(address.to_string()),
                RefusalDetail::SourceUnreadable,
                "diagnostics source was not admitted",
            ))
        })?;
    let authority = source
        .logical_view_read_authority(cancellation)
        .map_err(|error| {
            Box::new(error_result_detailed(
                Some(address.to_string()),
                RefusalDetail::SourceUnreadable,
                error,
            ))
        })?;
    let mapping = if target_kind == TargetKind::Module {
        authority.diagnostic_mapping(address, context)
    } else {
        authority.diagnostic_node_mapping(address, context)
    }
    .map_err(|error| Box::new(view_error_result(Some(address.to_string()), error)))?;
    // Анализатор, которого ещё нет на машине, доставляется этим же вызовом,
    // как у поиска с ролью: незавершённая поставка отвечает `working`
    // с повтором, неудачная — своим классом и полями движка (#1338), а
    // движок, которого и после поставки нет, — общим отказом (#549).
    // Анализатор BSL судит только модули: ради чужой цели его не доставляют.
    let analyzer_engine = target_kind == TargetKind::Module;
    #[cfg(test)]
    let analyzer_engine = analyzer_engine
        && crate::infrastructure::application_ports::DIAGNOSTIC_PROVIDER_OVERRIDE
            .with(|slot| slot.borrow().is_none());
    if analyzer_engine {
        acquire_engine(
            invocation,
            &context.cwd,
            BSL_ANALYZER_ENGINE,
            Some(address.to_string()),
            cancellation,
        )?;
    }
    let metadata_path = mapping.target().metadata_path.clone();
    let request = DiagnosticRequest {
        action,
        source_set: address.source_set().to_string(),
        metadata_path,
        filter: DiagnosticFilter::default(),
        range: None,
        // The coordinator truncates only after collecting and sorting all
        // findings. `check` must decide its verdict from the complete set;
        // its public page limit is applied afterwards.
        limit: usize::MAX,
        // Срок берётся только из настройки пользователя. Без настройки
        // срока нет и анализ идёт до конца (#1251); заданный срок
        // соблюдается, а нечитаемая настройка — отказ, а не умолчание.
        timeout: match crate::infrastructure::operational_config::load_operational_config(
            &context.workspace_root,
        ) {
            Ok(config) => config.code_diagnostics().analyze_timeout(),
            Err(diagnostic) => {
                return Err(Box::new(error_result(
                    Some(address.to_string()),
                    RefusalCode::InvalidState,
                    format!("operational config is unreadable: {diagnostic}"),
                )))
            }
        },
    };
    match DiagnosticCoordinator::new(registry, &mapping).execute_scoped(
        &request,
        context,
        cancellation,
        mapping
            .module_scope()
            .map(|scope| (scope, source.deadline())),
    ) {
        Ok(result) => {
            // Провайдер, который не отработал, не доказывает чистоту кода.
            // Пустой список находок при незавершённом прогоне выглядел бы как
            // «проверено и чисто» — худшее направление ошибки для инструмента
            // проверки.
            if !bsl_result_proves_full_verdict(&result) {
                // Движок не доставлен, не прошёл проверку поставки или не
                // запустился — поставщика нет, и отказ тот же, что на всех
                // маршрутах: поля движка в `data` (#549). Решает тип
                // резолвера, а не текст поставщика.
                if analyzer_engine && bsl_provider_absent(&result) {
                    if let Some(refusal) = bsl_engine_refusal(
                        &context.cwd,
                        &result,
                        Some(address.to_string()),
                        cancellation,
                    ) {
                        return Err(Box::new(refusal));
                    }
                }
                // Прогон, который начался и не завершился, отвечает
                // умолчанием кода. В обоих случаях сообщение называет
                // причину и действие восстановления (#1338).
                let message = bsl_incomplete_message(&result);
                if bsl_provider_absent(&result) {
                    return Err(Box::new(error_result_detailed(
                        Some(address.to_string()),
                        RefusalDetail::ProviderAbsent,
                        message,
                    )));
                }
                return Err(Box::new(error_result(
                    Some(address.to_string()),
                    RefusalCode::ProviderUnavailable,
                    message,
                )));
            }
            let structure = bsl_structure_findings(&mapping, address, cancellation)?;
            let findings: Vec<Value> = result
                .items
                .iter()
                .chain(&structure)
                .map(|item| serde_json::to_value(item).expect("diagnostic items serialize"))
                .collect();
            // Провалом считается ошибка, а не всякая пометка: подсказка по
            // стилю не ломает модуль, и остальные валидаторы поверхности
            // судят так же.
            let passed = bsl_findings_passed(&result.items) && bsl_findings_passed(&structure);
            Ok(Some(CheckStepVerdict {
                passed,
                diagnostics: findings,
                suppressed: bsl_suppression_facts(&result),
            }))
        }
        // Координатор отказывает по разным причинам, и уточнение получает
        // только та, которую словарь уточнений называет: «подходящего
        // поставщика нет». Отмена — свой код, а не отказ провайдера; всё
        // остальное отвечает умолчанием, а не выдуманным уточнением.
        Err(error) => Err(Box::new(match error.code {
            "cancelled" => error_result(
                Some(address.to_string()),
                RefusalCode::Cancelled,
                format!("{}: {}", error.code, error.message),
            ),
            "no_applicable_provider" => error_result_detailed(
                Some(address.to_string()),
                RefusalDetail::ProviderAbsent,
                format!("{}: {}", error.code, error.message),
            ),
            _ => error_result(
                Some(address.to_string()),
                RefusalCode::ProviderUnavailable,
                format!("{}: {}", error.code, error.message),
            ),
        })),
    }
}

/// Что спрашивают у порта символов (#2177).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SymbolOperation {
    Definition,
    Hover,
    References,
    IncomingCalls,
    OutgoingCalls,
}

/// Вопрос порта символов: модуль, его буфер в редакторе, если он открыт,
/// и позиция в нём.
pub(crate) struct SymbolRequest<'a> {
    pub(crate) module: QualifiedAddress,
    pub(crate) buffer: Option<&'a str>,
    pub(crate) position: unica_code_intelligence::symbols::SymbolPosition,
    pub(crate) operation: SymbolOperation,
}

/// Ответ службы символов в модели Unica: места или сведения об имени.
pub(crate) enum SymbolOutcome {
    Places(
        unica_code_intelligence::symbols::SymbolAnswer<
            Vec<unica_code_intelligence::symbols::SymbolLocation>,
        >,
    ),
    Info(
        unica_code_intelligence::symbols::SymbolAnswer<
            Option<unica_code_intelligence::symbols::SymbolInfo>,
        >,
    ),
}

impl CanonicalV13ReadService {
    /// Ответ порта символов в модели Unica: адреса и диапазоны, без имён
    /// поставщика. Буфер, отличный от сохранённого модуля, отвечает своим
    /// текстом, а то, что за его пределами знает только поставщик, помечено
    /// `stale`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn symbols(
        &self,
        invocation: &ActorBoundExecution,
        request: SymbolRequest<'_>,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let at = request.module.to_string();
        let source_set = request.module.source_set().to_string();
        let sources = match invocation.read_sources() {
            Ok(sources) => sources,
            Err(error) => {
                return error_result_detailed(Some(at), RefusalDetail::SourceUnreadable, error)
            }
        };
        let Some(source) = sources
            .into_iter()
            .find(|source| source.source_set_name() == source_set)
        else {
            return error_result(
                Some(at),
                RefusalCode::NotFound,
                "the module belongs to no admitted source set",
            );
        };
        let authority = match source.logical_view_read_authority(cancellation) {
            Ok(authority) => authority,
            Err(error) => {
                return error_result_detailed(Some(at), RefusalDetail::SourceUnreadable, error)
            }
        };
        // The provider gets the invocation's deadline, as MCP calls do.
        let context = unica_code_intelligence::symbols::SymbolRequestContext {
            deadline: source.deadline(),
            cancellation,
        };
        let (at, outcome) = match self.symbols_with(
            &authority,
            invocation.workspace_context(),
            request,
            &context,
            false,
        ) {
            Ok(answered) => answered,
            Err(error) => return view_error_result(Some(at), error),
        };
        let data = match outcome {
            SymbolOutcome::Places(answer) => serde_json::to_value(answer),
            SymbolOutcome::Info(answer) => serde_json::to_value(answer),
        }
        .expect("symbol answers serialize");
        let mut result = DomainResult::success("symbols answered");
        result.at = Some(at);
        result.data = Some(data);
        result
    }

    /// The symbol service over one source set's saved sources: the answer
    /// and the canonical address of the module. `yielding` lets the provider
    /// give way to an MCP call, as the editor's door asks (`mcp-priority`).
    pub(crate) fn symbols_with(
        &self,
        authority: &crate::infrastructure::v13_read::LogicalViewReadAuthority<'_>,
        workspace: &unica_source_contract::workspace::WorkspaceContext,
        request: SymbolRequest<'_>,
        context: &unica_code_intelligence::symbols::SymbolRequestContext<'_>,
        yielding: bool,
    ) -> Result<(String, SymbolOutcome), ViewError> {
        use unica_code_intelligence::symbols::{CallDirection, SymbolDocument, SymbolService};
        let source_set = request.module.source_set().to_string();
        // The module's canonical address and path, so the places the
        // provider names in this module compare equal to the document.
        let resolved = Self::resolve_with_authority(authority, &request.module, None)?;
        let saved = authority.module_text(&request.module)?;
        let (at, path) = (resolved.at().to_string(), resolved.path().to_string());
        let environment = crate::infrastructure::symbol_environment::ReaderSymbolEnvironment {
            authority,
            source_set: &source_set,
        };
        let document = SymbolDocument {
            at: &at,
            path: &path,
            text: request.buffer.unwrap_or(&saved),
            saved: &saved,
        };
        let provider = self.symbol_provider(workspace, &source_set, yielding);
        let service =
            SymbolService::new(provider.as_deref(), &environment).with_names(&environment);
        let position = request.position;
        let outcome = match request.operation {
            SymbolOperation::Definition => {
                SymbolOutcome::Places(service.definition(&document, position, context))
            }
            SymbolOperation::Hover => {
                SymbolOutcome::Info(service.hover(&document, position, context))
            }
            SymbolOperation::References => {
                SymbolOutcome::Places(service.references(&document, position, context))
            }
            SymbolOperation::IncomingCalls => SymbolOutcome::Places(service.calls(
                &document,
                position,
                CallDirection::Incoming,
                context,
            )),
            SymbolOperation::OutgoingCalls => SymbolOutcome::Places(service.calls(
                &document,
                position,
                CallDirection::Outgoing,
                context,
            )),
        };
        Ok((at, outcome))
    }

    /// Поставщики диагностик, те же, что у `check`.
    pub(crate) fn diagnostic_registry(
        &self,
    ) -> Result<unica_diagnostics::model::DiagnosticProviderRegistry, String> {
        use crate::application::ports::ApplicationPorts;
        self.ports.diagnostic_provider_registry()
    }

    /// Поставщик символов набора исходников, если он есть. Его нет — ответ
    /// собирается из текста модуля и называет причину неполноты.
    fn symbol_provider(
        &self,
        workspace: &unica_source_contract::workspace::WorkspaceContext,
        source_set: &str,
        yielding: bool,
    ) -> Option<Box<dyn unica_code_intelligence::symbols::SymbolProvider>> {
        use crate::application::ports::ApplicationPorts;
        #[cfg(test)]
        if let Some(provider) = crate::infrastructure::application_ports::SYMBOL_PROVIDER_OVERRIDE
            .with(|slot| slot.borrow().clone())
        {
            return Some(Box::new(SharedSymbolProvider(provider)));
        }
        let mut args = serde_json::Map::new();
        args.insert("sourceSet".to_string(), json!(source_set));
        let (context, _) = self
            .ports
            .resolve_code_search_context(workspace, &args)
            .ok()?;
        let provider = crate::infrastructure::analysis_composition::bsl_symbols(context);
        Some(Box::new(if yielding {
            provider.yielding()
        } else {
            provider
        }))
    }
}

/// A test's shared provider behind the boxed port.
#[cfg(test)]
struct SharedSymbolProvider(Arc<dyn unica_code_intelligence::symbols::SymbolProvider>);

#[cfg(test)]
impl unica_code_intelligence::symbols::SymbolProvider for SharedSymbolProvider {
    fn symbol(
        &self,
        query: &unica_code_intelligence::symbols::SymbolQuery,
        context: &unica_code_intelligence::symbols::SymbolRequestContext<'_>,
    ) -> Result<
        Option<unica_code_intelligence::symbols::ProviderSymbol>,
        unica_code_intelligence::symbols::ProviderFailure,
    > {
        self.0.symbol(query, context)
    }

    fn references(
        &self,
        query: &unica_code_intelligence::symbols::SymbolQuery,
        context: &unica_code_intelligence::symbols::SymbolRequestContext<'_>,
    ) -> Result<
        unica_code_intelligence::symbols::ProviderPlaces,
        unica_code_intelligence::symbols::ProviderFailure,
    > {
        self.0.references(query, context)
    }

    fn calls(
        &self,
        query: &unica_code_intelligence::symbols::SymbolQuery,
        direction: unica_code_intelligence::symbols::CallDirection,
        context: &unica_code_intelligence::symbols::SymbolRequestContext<'_>,
    ) -> Result<
        unica_code_intelligence::symbols::ProviderPlaces,
        unica_code_intelligence::symbols::ProviderFailure,
    > {
        self.0.calls(query, direction, context)
    }
}

/// Поставщик находок, которые проверяет сам Unica, а не анализатор.
const UNICA_BSL_STRUCTURE_PROVIDER: &str = "unica";

/// Код находки: объявление переменной не в начале модуля или метода.
const MISPLACED_VARIABLE_CODE: &str = "VariableDeclarationPlacement";

/// Ошибки компиляции платформы, которых анализатор не видит (#1460).
///
/// Анализатор пропускает `Перем` после методов, а платформа такой модуль
/// не компилирует: `check` отвечал `passed`. Находка принадлежит
/// проверенному модулю — тому же месту, куда отображение относит находки
/// анализатора в этом модуле, — и проваливает проверку.
fn bsl_structure_findings(
    mapping: &crate::infrastructure::diagnostics::CanonicalDiagnosticMapping,
    address: &QualifiedAddress,
    cancellation: &CancellationToken,
) -> Result<Vec<unica_diagnostics::model::DiagnosticItem>, Box<DomainResult>> {
    use unica_address::source_target::TargetKind;
    use unica_diagnostics::model::DiagnosticFocus;
    use unica_diagnostics::model::DiagnosticItem;
    use unica_diagnostics::model::DiagnosticSeverity;
    use unica_diagnostics::source_ranges::misplaced_variable_declarations;
    use unica_diagnostics::source_ranges::MISPLACED_VARIABLE_MESSAGE;
    use unica_source_contract::location::SourceLocation;

    // Объект и корень не модуль: структуру модуля у них не проверить.
    let Some(scope) = mapping.module_scope() else {
        return Ok(Vec::new());
    };
    let text = match std::fs::read_to_string(scope.module_path) {
        Ok(text) => text,
        // Модуля без файла анализатор тоже не судил: судить нечего.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(Box::new(error_result_detailed(
                Some(address.to_string()),
                RefusalDetail::SourceUnreadable,
                format!("BSL module is unreadable: {}", error.kind()),
            )))
        }
    };
    if cancellation.is_cancelled() {
        return Err(Box::new(error_result(
            Some(address.to_string()),
            RefusalCode::Cancelled,
            "check was cancelled before the module structure was checked",
        )));
    }
    let target = mapping.target();
    Ok(misplaced_variable_declarations(&text)
        .into_iter()
        .map(|range| DiagnosticItem::Diagnostic {
            provider: UNICA_BSL_STRUCTURE_PROVIDER,
            location: SourceLocation::Addressed {
                source_set: target.source_set.clone(),
                metadata_path: target.metadata_path.clone(),
                target_kind: TargetKind::Module,
            },
            location_reason: None,
            focus: DiagnosticFocus::SourceRange { range },
            code: MISPLACED_VARIABLE_CODE.to_string(),
            severity: DiagnosticSeverity::Error,
            message: MISPLACED_VARIABLE_MESSAGE.to_string(),
            tags: Vec::new(),
        })
        .collect())
}

fn bsl_result_proves_full_verdict(result: &unica_diagnostics::model::DiagnosticResult) -> bool {
    use unica_diagnostics::model::DiagnosticResultState;
    result.ok
        && result.state == DiagnosticResultState::Completed
        && result.complete
        && result.truncated == Some(false)
        && result.items_total == Some(result.items.len())
        && result.items_returned == Some(result.items.len())
}

/// Итог одного валидатора `check`: вердикт, находки и фильтры, которые
/// пользователь настроил и поставщик применил до ответа.
struct CheckStepVerdict {
    passed: bool,
    diagnostics: Vec<Value>,
    suppressed: Vec<Value>,
}

impl From<(bool, Vec<Value>)> for CheckStepVerdict {
    fn from((passed, diagnostics): (bool, Vec<Value>)) -> Self {
        Self {
            passed,
            diagnostics,
            suppressed: Vec::new(),
        }
    }
}

const BSL_INCOMPLETE_MESSAGE: &str = "BSL analysis did not complete, so the module is unproven";

/// Names why the analysis is incomplete and what restores it (#1338).
///
/// Named baseline causes and canonical JSONL failures keep their own
/// sanitization boundary. Every other provider or resource failure is named
/// by its closed code through the coordinator's fixed public wording, never
/// by the provider's text; an incompleteness nobody named still says so and
/// what to do.
fn bsl_incomplete_message(result: &unica_diagnostics::model::DiagnosticResult) -> String {
    use unica_diagnostics::coordinator::public_diagnostic_error_message;
    let mut reasons: Vec<String> = Vec::new();
    let mut push = |reason: String| {
        if !reasons.contains(&reason) {
            reasons.push(reason);
        }
    };
    for suppression in result
        .providers
        .iter()
        .flat_map(|section| section.suppressions.iter())
    {
        if let Some(reason) = suppression.reason() {
            push(match &reason.detail {
                Some(detail) => format!("diagnostics baseline {}: {detail}", reason.code),
                None => format!("diagnostics baseline {}", reason.code),
            });
        }
    }
    for section in &result.providers {
        if let Some(error) = &section.error {
            // Two closed formatters may name the cause in their own words:
            // a JSONL failure, and an absent engine with the next step its
            // install mode chooses. Anything else is named by its code.
            let canonical = match error.code.as_str() {
                "diagnostics_invalid" => {
                    unica_diagnostics::stream_error::canonical_stream_error(&error.message)
                }
                unica_artifact_delivery::engine::BUNDLED_TOOL_MISSING => {
                    unica_diagnostics::coordinator::canonical_missing_engine_reason(&error.message)
                }
                _ => None,
            };
            push(canonical.unwrap_or_else(|| {
                format!(
                    "{}: {}",
                    section.id,
                    public_diagnostic_error_message(&error.code)
                )
            }));
        }
    }
    for item in &result.items {
        if let unica_diagnostics::model::DiagnosticItem::ResourceFailure { error, .. } = item {
            push(public_diagnostic_error_message(&error.code).to_string());
        }
    }
    if reasons.is_empty() {
        format!("{BSL_INCOMPLETE_MESSAGE}: {BSL_UNNAMED_INCOMPLETENESS}")
    } else {
        format!("{BSL_INCOMPLETE_MESSAGE}: {}", reasons.join("; "))
    }
}

const BSL_UNNAMED_INCOMPLETENESS: &str = "the analyzer did not name a cause. Repeat the call; if the problem persists, report this reason.";

/// The typed missing-engine refusal of an analyzer that gave no result:
/// the resolver names the stage where the engine is absent (manifest, target,
/// file or checksum); an engine that passed every install check but whose
/// process the provider could not start is absent at the `start` stage. Any
/// other incompleteness is not an absent engine (#549).
fn bsl_engine_refusal(
    cwd: &std::path::Path,
    result: &unica_diagnostics::model::DiagnosticResult,
    at: Option<String>,
    cancellation: &CancellationToken,
) -> Option<DomainResult> {
    if let Some(refusal) =
        engine_absence_refusal(cwd, BSL_ANALYZER_ENGINE, at.clone(), cancellation)
    {
        return Some(refusal);
    }
    let start_failed = result.providers.iter().any(|section| {
        section
            .error
            .as_ref()
            .is_some_and(|error| error.code == "provider_start_failed")
    });
    if !start_failed || cancellation.is_cancelled() {
        return None;
    }
    let plugin_root = crate::infrastructure::plugin_runtime::find_plugin_root(cwd)?;
    let program = crate::infrastructure::bundled_tools::installed_engine_path(
        &plugin_root,
        BSL_ANALYZER_ENGINE,
    )?;
    Some(
        crate::infrastructure::bundled_tools::start_failure(
            Some(&plugin_root),
            BSL_ANALYZER_ENGINE,
            &program,
        )
        .refusal(at),
    )
}

/// The engine is not on this machine, failed its install checks or could not
/// be started: the provider is absent rather than broken mid-run.
fn bsl_provider_absent(result: &unica_diagnostics::model::DiagnosticResult) -> bool {
    result.providers.iter().any(|section| {
        section.error.as_ref().is_some_and(|error| {
            matches!(
                error.code.as_str(),
                unica_artifact_delivery::engine::BUNDLED_TOOL_MISSING
                    | "provider_install_invalid"
                    | "provider_start_failed"
            )
        })
    })
}

/// Подавление по настройке пользователя называется фактом рядом с
/// вердиктом: «прошёл» с базовой линией значит «прошёл после фильтра».
fn bsl_suppression_facts(result: &unica_diagnostics::model::DiagnosticResult) -> Vec<Value> {
    result
        .providers
        .iter()
        .flat_map(|section| {
            section.suppressions.iter().map(|suppression| {
                let mut fact = serde_json::to_value(suppression).expect("suppression serializes");
                fact.as_object_mut()
                    .expect("suppression is an object")
                    .insert(
                        "provider".to_string(),
                        Value::String(section.id.to_string()),
                    );
                fact
            })
        })
        .collect()
}

fn bsl_findings_passed(items: &[unica_diagnostics::model::DiagnosticItem]) -> bool {
    use unica_diagnostics::model::DiagnosticItem;
    use unica_diagnostics::model::DiagnosticSeverity;
    !items.iter().any(|item| {
        matches!(
            item,
            DiagnosticItem::Diagnostic {
                severity: DiagnosticSeverity::Error,
                ..
            } | DiagnosticItem::ResourceFailure { .. }
        )
    })
}

/// Internal check evidence only: it never becomes a public `view` response.
/// The complete native verdict is consumed by the common plan and pager.
fn prepare_xdto_check(
    invocation: &ActorBoundExecution,
    address: &QualifiedAddress,
    cancellation: &CancellationToken,
) -> (
    DomainResult,
    Option<crate::application::v13::check::NativeCheckOutcome>,
) {
    let at = address.to_string();
    let prepared = (|| {
        let sources = invocation
            .read_sources()
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        let source = sources
            .into_iter()
            .find(|source| source.source_set_name() == address.source_set())
            .ok_or_else(|| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "XDTO source was not admitted",
                )
            })?;
        let authority = source
            .logical_view_read_authority(cancellation)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        authority.check_xdto(address)
    })();
    match prepared {
        Err(refusal) => (view_error_result(Some(at.clone()), refusal), None),
        Ok((snapshot, outcome)) => {
            let mut evidence = DomainResult::success("XDTO check source admitted");
            evidence.at = Some(at);
            evidence.rev = Some(snapshot.revision);
            evidence.data = Some(
                json!({"kind":"XDTOPackage", "props": if outcome.is_none() { json!({"content":"inherited"}) } else { json!({}) }}),
            );
            (evidence, outcome)
        }
    }
}

fn run_node_checks(
    ports: &crate::infrastructure::application_ports::InfrastructureApplicationPorts,
    invocation: &ActorBoundExecution,
    at: &str,
    kind: &str,
    viewed: Option<&Value>,
    cancellation: &CancellationToken,
    mut prepared_xdto: Option<crate::application::v13::check::NativeCheckOutcome>,
) -> DomainResult {
    use crate::application::v13::check::{plan_for_node, CheckStep};
    use crate::infrastructure::native_operations::v13_analysis::node_facts;

    let address = match QualifiedAddress::parse(at) {
        Ok(address) => address,
        Err(error) => {
            return error_result(
                Some(at.to_string()),
                RefusalCode::BadValue,
                error.to_string(),
            )
        }
    };
    let context = invocation.workspace_context();
    let plan = plan_for_node(kind, node_facts(&address, viewed, context));
    if plan.is_empty() {
        let mut result = DomainResult::success("logical node is readable");
        result.at = Some(at.to_string());
        result.data = Some(serde_json::json!({
            "status": "readable",
            "at": at,
            "kind": kind,
            "validators": [],
            "diagnostics": [],
        }));
        return result;
    }
    let mut passed = true;
    let mut diagnostics = Vec::new();
    let mut suppressed = Vec::new();
    let mut validators = Vec::new();
    for step in plan {
        let verdict = match step {
            CheckStep::Native(validator) => run_native_validator(
                &address,
                kind,
                validator,
                invocation,
                cancellation,
                prepared_xdto.take(),
            )
            .map(|verdict| Some(verdict.into())),
            CheckStep::Meta => run_meta_validator(&address, at, context, cancellation)
                .map(|verdict| Some(verdict.into())),
            CheckStep::Bsl => run_bsl_diagnostics(ports, &address, kind, invocation, cancellation),
        };
        match verdict {
            Err(refusal) => return *refusal,
            // Шаг без применимого поставщика не проверял ничего.
            Ok(None) => {}
            Ok(Some(step_verdict)) => {
                passed &= step_verdict.passed;
                validators.push(step.name());
                let tag = |mut value: Value| {
                    if let Some(object) = value.as_object_mut() {
                        object.insert("validator".to_string(), Value::String(step.name().into()));
                    }
                    value
                };
                diagnostics.extend(step_verdict.diagnostics.into_iter().map(tag));
                suppressed.extend(step_verdict.suppressed.into_iter().map(tag));
            }
        }
    }
    let mut result = DomainResult::success(if passed {
        "validation passed"
    } else {
        "validation reported findings"
    });
    result.at = Some(at.to_string());
    let mut data = serde_json::json!({
        "status": if passed { "passed" } else { "failed" },
        "at": at,
        "kind": kind,
        "validators": validators,
        "diagnostics": diagnostics,
    });
    if !suppressed.is_empty() {
        data["suppressed"] = Value::Array(suppressed);
    }
    result.data = Some(data);
    result
}

fn run_native_validator(
    address: &QualifiedAddress,
    kind: &str,
    validator: crate::application::v13::check::CheckValidator,
    invocation: &ActorBoundExecution,
    cancellation: &CancellationToken,
    prepared_xdto: Option<crate::application::v13::check::NativeCheckOutcome>,
) -> Result<(bool, Vec<Value>), Box<DomainResult>> {
    use crate::application::v13::check::normalize_native_outcome;
    use crate::infrastructure::native_operations::v13_analysis::{validate, validator_selector};

    let context = invocation.workspace_context();
    let at = address.to_string();
    let native = if validator == crate::application::v13::check::CheckValidator::Xdto {
        prepared_xdto.ok_or_else(|| {
            Box::new(error_result(
                Some(at.clone()),
                RefusalCode::InvalidState,
                "XDTO check has no retained source evidence",
            ))
        })?
    } else if matches!(
        validator,
        crate::application::v13::check::CheckValidator::Subsystem
            | crate::application::v13::check::CheckValidator::Dcs
            | crate::application::v13::check::CheckValidator::Mxl
            | crate::application::v13::check::CheckValidator::Form
    ) {
        let sources = invocation.read_sources().map_err(|error| {
            Box::new(error_result_detailed(
                Some(at.clone()),
                RefusalDetail::SourceUnreadable,
                error,
            ))
        })?;
        let source = sources
            .into_iter()
            .find(|source| source.source_set_name() == address.source_set())
            .ok_or_else(|| {
                Box::new(error_result_detailed(
                    Some(at.clone()),
                    RefusalDetail::SourceUnreadable,
                    "template source was not admitted",
                ))
            })?;
        let authority = source
            .logical_view_read_authority(cancellation)
            .map_err(|error| {
                Box::new(error_result_detailed(
                    Some(at.clone()),
                    RefusalDetail::SourceUnreadable,
                    error,
                ))
            })?;
        if validator == crate::application::v13::check::CheckValidator::Form {
            let input = authority
                .form_validation_input(address)
                .map_err(|error| Box::new(view_error_result(Some(at.clone()), error)))?;
            crate::infrastructure::native_operations::v13_analysis::validate_form_input(input)
        } else if validator == crate::application::v13::check::CheckValidator::Subsystem {
            authority
                .validate_subsystem(address)
                .map_err(|error| Box::new(view_error_result(Some(at.clone()), error)))?
        } else {
            let dcs = validator == crate::application::v13::check::CheckValidator::Dcs;
            let input = if dcs {
                authority.dcs_validation_input(address)
            } else {
                authority.mxl_validation_input(address)
            }
            .map_err(|error| Box::new(view_error_result(Some(at.clone()), error)))?;
            if dcs {
                crate::infrastructure::native_operations::v13_analysis::validate_dcs_input(input)
            } else {
                crate::infrastructure::native_operations::v13_analysis::validate_mxl_input(input)
            }
        }
    } else {
        let selector = validator_selector(validator, address, context).map_err(|error| {
            Box::new(error_result(Some(at.clone()), RefusalCode::BadValue, error))
        })?;
        validate(validator, &selector, context)
    };
    match normalize_native_outcome(address, kind, validator, native) {
        Ok(checked) => Ok((
            checked.ok(),
            checked
                .diagnostics()
                .iter()
                .map(|diagnostic| {
                    serde_json::json!({
                        "severity": diagnostic.severity(),
                        "code": diagnostic.code(),
                        "message": diagnostic.message(),
                    })
                })
                .collect(),
        )),
        // Уточнение принадлежит самой ошибке: служба не выбирает его заново и
        // не может разойтись с кодом, который оно сужает.
        Err(error) => Err(Box::new(match error.detail() {
            Some(detail) => error_result_detailed(Some(at), detail, error.to_string()),
            None => error_result(Some(at), error.code(), error.to_string()),
        })),
    }
}

/// Находка читателя, которую валидатор описания не повторяет. Корневые
/// свойства объекта и его дочерние ресурсы валидатор проверяет сам, и вторая
/// формулировка той же находки ответ не дополняет; тип, значения и свойства
/// элементов коллекций видит только читатель.
fn reader_only_finding(diagnostic: &unica_metadata::MetaDiagnostic) -> bool {
    let validator_owned = diagnostic.code
        == unica_metadata::MetaDiagnosticCode::ProviderUnavailable
        || diagnostic.field.as_deref().is_some_and(|field| {
            field == "properties"
                || field.starts_with("properties.")
                || field == "resources"
                || field.starts_with("resources.")
        });
    !validator_owned
}

/// The typed metadata validator of one object descriptor: the same read and
/// validation the metadata family runs, folded into the check envelope.
fn run_meta_validator(
    address: &QualifiedAddress,
    at: &str,
    context: &unica_source_contract::workspace::WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<(bool, Vec<Value>), Box<DomainResult>> {
    use crate::infrastructure::native_operations::v13_analysis::validator_metadata_path;

    let Some(path) = validator_metadata_path(address) else {
        return Err(Box::new(error_result(
            Some(at.to_string()),
            RefusalCode::BadValue,
            "the metadata validator needs one object descriptor; this address names none",
        )));
    };
    let target =
        unica_address::source_target::MetadataAddress::parse_logical(&path).map_err(|error| {
            Box::new(error_result(
                Some(at.to_string()),
                RefusalCode::BadValue,
                error.to_string(),
            ))
        })?;
    let request = unica_metadata::request::MetaInfoRequest {
        source_set: address.source_set().to_string(),
        metadata_path: target,
        sections: Vec::new(),
        limit: 0,
    };
    let readers = crate::infrastructure::support_state::WorkspaceSupportStateReaderFactory;
    let reader =
        crate::infrastructure::support_state::SupportStateReaderFactory::create(&readers, context);
    match crate::infrastructure::metadata_operations::MetadataOperations::read_local(
        &request,
        context,
        cancellation,
        reader.as_ref(),
    ) {
        Ok(read) => {
            let validated =
                crate::infrastructure::metadata_operations::MetadataOperations::validate(
                    &read.validation_subject,
                    context,
                    cancellation,
                );
            // Читатель описания находит то, чего валидатор не видит:
            // немоделируемый тип, битый квалификатор, голый `FillValue`,
            // повреждённые свойства элементов (#1933). Его ошибка решает
            // вердикт так же, как ошибка валидатора.
            let reader_findings = read
                .local
                .diagnostics
                .iter()
                .filter(|diagnostic| reader_only_finding(diagnostic))
                .collect::<Vec<_>>();
            let reader_failed = reader_findings.iter().any(|diagnostic| {
                diagnostic.severity == unica_metadata::MetaDiagnosticSeverity::Error
            });
            let mut diagnostics = Vec::<Value>::new();
            for diagnostic in validated.diagnostics.iter().chain(reader_findings) {
                let value = serde_json::to_value(diagnostic).unwrap_or(Value::Null);
                if !diagnostics.contains(&value) {
                    diagnostics.push(value);
                }
            }
            Ok((
                validated.status == unica_metadata::MetaValidationStatus::Passed && !reader_failed,
                diagnostics,
            ))
        }
        Err(failure) => {
            let message = failure
                .diagnostics
                .iter()
                .filter_map(|diagnostic| {
                    serde_json::to_value(diagnostic).ok().and_then(|value| {
                        value
                            .get("message")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                })
                .collect::<Vec<_>>()
                .join("; ");
            Err(Box::new(error_result_detailed(
                Some(at.to_string()),
                RefusalDetail::SourceUnreadable,
                if message.is_empty() {
                    "the metadata descriptor could not be read for validation".to_string()
                } else {
                    message
                },
            )))
        }
    }
}

/// A target the read port cannot open is named by its export format when
/// the format guard of its validator knows it: the closed refusal replaces a
/// provider failure that would otherwise hide a root outside the active
/// profile (`DEC.2026-08-21.SINGLE-WRITABLE-PLATFORM-XML-PROFILE`).
fn unreadable_target_format_refusal(
    invocation: &ActorBoundExecution,
    at: &str,
) -> Option<DomainResult> {
    use crate::application::ports::FormatGuardCheck;
    use crate::application::v13::check::CheckValidator;
    use crate::infrastructure::native_operations::v13_analysis::{
        source_set_is_extension, validator_selector,
    };

    let address = QualifiedAddress::parse(at).ok()?;
    let context = invocation.workspace_context();
    let validator =
        CheckValidator::for_unread_address(&address, source_set_is_extension(&address, context))?;
    let selector = validator_selector(validator, &address, context).ok()?;
    let check = crate::infrastructure::format_guard::evaluate_read_format_guard(
        validator.native_operation(),
        &selector,
        context,
    )
    .ok()?;
    let (warning, diagnostic) = match check {
        FormatGuardCheck::Allow => return None,
        FormatGuardCheck::Warn {
            warning,
            diagnostic,
        } => (warning, diagnostic),
        FormatGuardCheck::Block {
            outcome,
            diagnostic,
            ..
        } => (outcome.warnings.join(" "), diagnostic),
    };
    let actual = diagnostic
        .get("actualFormat")
        .and_then(Value::as_str)
        .map(|actual| format!(" (export format {actual})"))
        .unwrap_or_default();
    Some(error_result(
        Some(at.to_string()),
        RefusalCode::InvalidSource,
        format!("{warning}{actual}"),
    ))
}

// Syntax is checked for the whole requested StyleItem batch before reading
// descriptor state. The family decoder and bound descriptor own both shapes.
fn validate_style_item_apply_capabilities(
    request: &crate::domain::apply::ApplyRequest,
    source: unica_source_contract::context::SourceSetTraits,
    admission: &unica_workspace::actor::ApplyAdmission,
    binding: &unica_workspace::actor::ProviderRootBinding,
) -> Result<(), Box<DomainResult>> {
    use unica_metadata::descriptor_operations::{PropertiesOperation, PropertiesSite};
    let registry = crate::domain::apply::operation_registry();
    let mut checked = Vec::new();
    for (index, operation) in request.ops().iter().enumerate() {
        let Some(site) = crate::domain::apply::operation_site(operation.at(), source) else {
            continue;
        };
        let Some(descriptor) = registry.lookup_at(operation.name(), &site) else {
            continue;
        };
        if descriptor.operation::<PropertiesOperation>() != Some(PropertiesOperation::Set)
            || PropertiesSite::from_site(&site) != Some(PropertiesSite::StyleItem)
        {
            continue;
        }
        let mut payload = operation.args().clone();
        payload.remove("at");
        let payload = Value::Object(payload);
        let intent = unica_metadata::property_input::decode_properties(
            PropertiesOperation::Set,
            PropertiesSite::StyleItem,
            &payload,
            &format!("ops[{index}].args"),
        )
        .map_err(|error| {
            Box::new(error_result(
                Some(error.path),
                RefusalCode::BadValue,
                error.message,
            ))
        })?;
        checked.push((index, operation, descriptor, intent, payload));
    }
    for (index, operation, descriptor, intent, payload) in checked {
        let observed =
            crate::infrastructure::native_operations::apply_families::staged_style_item_properties(
                admission,
                binding,
                operation.at(),
                Some(&intent),
            )
            .map_err(|error| Box::new(apply_plan_error_result(&error)))?;
        let site = crate::domain::apply::operation_site(operation.at(), source)
            .expect("checked operation site");
        let lookup = |at: &QualifiedAddress| if at == operation.at() { observed } else { None };
        let contract = descriptor
            .argument_contract_at_with_observations(
                &site,
                &crate::domain::apply::observation_lookup_with_style_properties(
                    &|_| None,
                    &|_| None,
                    &lookup,
                ),
            )
            .ok_or_else(|| {
                Box::new(error_result(
                    Some(format!("ops[{index}].at")),
                    RefusalCode::InvalidState,
                    "no writable property capability is confirmed for this descriptor",
                ))
            })?;
        contract
            .normalize(&payload, &format!("ops[{index}].args"))
            .map_err(|error| {
                Box::new(error_result(
                    Some(error.path),
                    RefusalCode::BadValue,
                    error.message,
                ))
            })?;
    }
    Ok(())
}

/// Предварительная проверка `apply`: каждая операция есть в реестре, набор
/// исходников её семейство пишет, а сама она исполнима на своём узле.
///
/// Проверяется применимость к месту; `next` неприменимой операции ведёт к `can`
/// ближайшего узла, где она исполнима. Если такого узла нет, `next` ведёт к
/// `can` того же узла, когда тот читается, иначе к корню набора, а текст
/// называет форму нужного адреса. `template_format` читает формат макета-
/// предка: `next` не уводит на макет другого формата.
pub(super) fn preflight_apply_operations(
    request: &crate::domain::apply::ApplyRequest,
    source: unica_source_contract::context::SourceSetTraits,
    template_format: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
) -> Result<(), Box<DomainResult>> {
    preflight_apply_operations_with(request, source, template_format, &|_| None, &|_| None)
}

/// То же, что `preflight_apply_operations`, с ролью элемента формы.
pub(super) fn preflight_apply_operations_with(
    request: &crate::domain::apply::ApplyRequest,
    source: unica_source_contract::context::SourceSetTraits,
    template_format: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
    form_item: &dyn Fn(&QualifiedAddress) -> Option<unica_forms::FormItemRole>,
    color: &dyn Fn(
        &QualifiedAddress,
    ) -> Option<unica_metadata::style_color::StyleItemPropertiesObservation>,
) -> Result<(), Box<DomainResult>> {
    preflight_apply_operations_observed(
        request,
        source,
        template_format,
        &|at| form_item(at).map(unica_forms::FormObservation::Item),
        color,
    )
}

pub(super) fn preflight_apply_operations_observed(
    request: &crate::domain::apply::ApplyRequest,
    source: unica_source_contract::context::SourceSetTraits,
    template_format: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
    form_observation: &dyn Fn(&QualifiedAddress) -> Option<unica_forms::FormObservation>,
    color: &dyn Fn(
        &QualifiedAddress,
    ) -> Option<unica_metadata::style_color::StyleItemPropertiesObservation>,
) -> Result<(), Box<DomainResult>> {
    let registry = crate::domain::apply::operation_registry();
    for (index, operation) in request.ops().iter().enumerate() {
        let Some(descriptor) = registry.lookup(operation.name()) else {
            return Err(Box::new(with_node_dictionary(
                error_result(
                    Some(format!("ops[{index}].op")),
                    RefusalCode::UnsupportedOperation,
                    format!(
                        "apply operation `{}` is not in the canonical registry",
                        operation.name()
                    ),
                ),
                &operation.at().to_string(),
            )));
        };
        // Общее имя (`attribute.add` объекта и формы, #1465) выбирает
        // запись по адресу вызова.
        let descriptor = crate::domain::apply::operation_site(operation.at(), source)
            .and_then(|site| registry.lookup_at(operation.name(), &site))
            .unwrap_or(descriptor);
        // Набор, который семейство не пишет, — свойство набора, а не «не тот
        // узел»: тот же отказ даёт разбор планировщиком.
        if !source.writes(descriptor.family()) {
            return Err(Box::new(DomainResult::canonical_rejection_detailed(
                Some(format!("ops[{index}].op")),
                RefusalDetail::WrongSourceKind,
                UNWRITABLE_SOURCE_MESSAGE,
            )));
        }
        let target_kind = operation
            .at()
            .segments()
            .last()
            .expect("a parsed logical address has a terminal segment")
            .kind();
        let Some(site) = crate::domain::apply::operation_site(operation.at(), source) else {
            return Err(Box::new(error_result(
                Some(format!("ops[{index}].at")),
                RefusalCode::NotFound,
                format!(
                    "logical address `{}` is not available in the platform profile",
                    operation.at()
                ),
            )));
        };
        // Наблюдаемые факты формы сужают структурную применимость:
        // роль элемента и тип реквизита проверяет само семейство.
        if descriptor.applies_at(&site)
            && (!descriptor
                .observation_type()
                .is::<unica_forms::FormObservation>()
                || descriptor.applies_with_observations(
                    &site,
                    &crate::domain::apply::observation_lookup(template_format, form_observation),
                ))
        {
            continue;
        }
        let refused = format!(
            "apply operation `{}` does not apply to {} `{}`",
            operation.name(),
            target_kind.as_str(),
            operation.at()
        );
        let nearest = registry.nearest_ancestor_with_observations(
            operation.name(),
            &site,
            &crate::domain::apply::observation_lookup_with_style_properties(
                template_format,
                form_observation,
                color,
            ),
        );
        let refusal = match nearest {
            Some((descriptor, target)) => with_operation_dictionary(
                error_result(
                    Some(format!("ops[{index}].at")),
                    RefusalCode::BadValue,
                    format!("{refused}; it applies at `{target}`"),
                ),
                &target.to_string(),
                descriptor,
            ),
            None => {
                // Общее имя (#1465) исполнимо по формам адреса обеих записей.
                let shapes = registry
                    .named(operation.name())
                    .map(|descriptor| descriptor.address_shape())
                    .collect::<Vec<_>>()
                    .join(" or ");
                let message = format!("{refused}; it addresses {shapes}");
                let mut refusal = error_result(
                    Some(format!("ops[{index}].at")),
                    RefusalCode::BadValue,
                    message,
                );
                // Код пишется в модуль, а не в его владельца: узел с модулями
                // ведёт к их адресам (#1964).
                if descriptor.family() == unica_operations::OperationFamily::Code {
                    if let Some(branch) = crate::infrastructure::v13_read::module_branch_for_parent(
                        operation.at(),
                        crate::domain::apply::source_classification_profile(source),
                        source.kind(),
                    ) {
                        refusal.next.push(json!({
                            "tool": "unica.view",
                            "args": {"at": branch.at()},
                            "reason": "адреса модулей этого узла: код пишется по адресу модуля",
                        }));
                    }
                }
                // Узел, который читается, показывает свой словарь; адрес,
                // которого нет ни в одном дереве, ведёт к корню набора.
                let readable = crate::infrastructure::logical_tree::route_logical_address(
                    operation.at(),
                    crate::domain::apply::source_classification_profile(source),
                )
                .is_ok();
                if readable {
                    with_node_dictionary(refusal, &operation.at().to_string())
                } else {
                    with_node_dictionary(
                        refusal,
                        &format!("{}:Configuration", operation.at().source_set()),
                    )
                }
            }
        };
        return Err(Box::new(refusal));
    }
    Ok(())
}

/// Принимает ли роль элемента формы по адресу `at` операцию; адрес не
/// элемента формы фильтр не проходит.
fn admits_form_item_at(
    descriptor: unica_operations::OperationDescriptor,
    at: &QualifiedAddress,
    form_item: &dyn Fn(&QualifiedAddress) -> Option<unica_forms::FormItemRole>,
) -> bool {
    at.segments()
        .last()
        .is_none_or(|segment| segment.kind() != NodeKind::Item)
        || descriptor.admits_form_item(form_item(at))
}

/// `next` к словарю узла, где операция исполнима. Подробный контракт
/// операции (`<узел>.Can.<op>`) запрашивается только там, где он
/// вычисляется: иначе `view` ответил бы на сам переход отказом.
fn with_operation_dictionary(
    mut result: DomainResult,
    at: &str,
    descriptor: unica_operations::OperationDescriptor,
) -> DomainResult {
    let at = if descriptor.argument_contract().is_some() {
        format!("{at}.Can.{}", descriptor.name())
    } else {
        at.to_string()
    };
    result.next.push(json!({
        "tool": "unica.view",
        "args": {"at": at},
        "reason": format!("где исполнима операция `{}`", descriptor.name()),
    }));
    result
}

/// Отказ планировщика «узел не найден» по адресу операции: узел создаёт
/// операция родителя, поэтому `next` ведёт к словарю родителя, а не к адресу,
/// который `view` тоже не найдёт. `node_exists` отличает отсутствующий узел
/// от существующего, у которого нет присоединённого ресурса (`Rights.xml`
/// роли, `Form.xml` формы): во втором случае создавать нечего, и `next` не
/// добавляется.
pub(super) fn with_missing_target_parent(
    mut result: DomainResult,
    request: &crate::domain::apply::ApplyRequest,
    error: &unica_source_contract::proposal::ApplyPlanError,
    node_exists: &dyn Fn(&QualifiedAddress) -> Option<bool>,
) -> DomainResult {
    if error.kind() != ApplyPlanErrorKind::NotFound {
        return result;
    }
    let Some((index, tail)) = error.path().and_then(|path| {
        let rest = path.strip_prefix("ops[")?;
        let (index, tail) = rest.split_once(']')?;
        Some((index.parse::<usize>().ok()?, tail))
    }) else {
        return result;
    };
    let Some(operation) = request.ops().get(index) else {
        return result;
    };
    // Отказ касается самого узла операции, когда он назван путём `at` или
    // когда операция не несёт ничего, кроме адреса (`attribute.remove`):
    // тогда «не найден» — это её узел, а не что-то из аргументов.
    let address_only = crate::domain::apply::operation_registry()
        .lookup(operation.name())
        .is_some_and(|descriptor| descriptor.skeleton_key() == "at");
    if !(matches!(tail, ".at" | ".args.at") || address_only) {
        return result;
    }
    if node_exists(operation.at()) != Some(false) {
        return result;
    }
    let segments = operation.at().segments();
    let parent = if segments.len() > 1 {
        QualifiedAddress {
            line_range: None,
            source_set: operation.at().source_set().to_string(),
            segments: segments[..segments.len() - 1].to_vec(),
        }
    } else {
        match QualifiedAddress::parse(&format!("{}:Configuration", operation.at().source_set())) {
            Ok(root)
                if segments.first().map(|first| first.kind()) != Some(NodeKind::Configuration) =>
            {
                root
            }
            _ => return result,
        }
    };
    result.next.push(json!({
        "tool": "unica.view",
        "args": {"at": parent.to_string()},
        "reason": "узел создаёт операция родителя",
    }));
    result
}

/// Отказ по операции называет маршрут к словарю узла, а не перечисляет
/// операции в тексте: перечень уже публикуется в `can` ответа узла, и агенту нужен
/// путь к нему, а не копия внутри сообщения. Без этого пробелы словаря
/// `apply` не всплывают — агент повторяет вслепую.
fn with_node_dictionary(mut result: DomainResult, at: &str) -> DomainResult {
    result.next.push(serde_json::json!({
        "tool": "unica.view",
        "args": {"at": at},
        "reason": "какие операции применимы к этому узлу",
    }));
    result
}

/// Свод, по которому идёт поиск.
///
/// Имя метаданного объекта и текст модуля — один вопрос «что нашлось и где»,
/// различается доказательство совпадения: у имени это `at`, `kind`, `title`,
/// у текста — `scope`, `line`, `column`, `snippet`. Физического пути нет ни у
/// того, ни у другого: путь в частом ответе приглашает обойти адресное
/// пространство, ради которого логический слой и существует.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchCorpus {
    Text,
    Names,
}

/// The addresses whose existence [`validate_name_scope`] asks about.
fn name_scope_witnesses(scope: &QualifiedAddress) -> Vec<String> {
    let mut witnesses = vec![
        format!("{}:Configuration", scope.source_set()),
        scope.to_string(),
    ];
    let first = &scope.segments()[0];
    if let Some(name) = first.name() {
        witnesses.push(format!(
            "{}:{}.{name}",
            scope.source_set(),
            first.kind().as_str()
        ));
    }
    witnesses
}

fn validate_name_scope(
    has_address: &dyn Fn(&str) -> bool,
    scope: &QualifiedAddress,
    source_kind: SourceSetKind,
) -> Result<(), (RefusalCode, &'static str)> {
    let segments = scope.segments();
    let first = &segments[0];
    let supported_owner = match source_kind {
        SourceSetKind::Configuration | SourceSetKind::Extension => {
            metadata_kind(first.kind().as_str()).is_some()
        }
        SourceSetKind::ExternalProcessor => first.kind() == NodeKind::ExternalDataProcessor,
        SourceSetKind::ExternalReport => first.kind() == NodeKind::ExternalReport,
    };
    if segments.len() == 1 && first.kind() == NodeKind::Configuration {
        if matches!(
            source_kind,
            SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
        ) {
            return Err((
                RefusalCode::UnsupportedScope,
                "external source sets have no Configuration root",
            ));
        }
        return has_address(&scope.to_string()).then_some(()).ok_or((
            RefusalCode::InvalidState,
            "name search Configuration descriptor is unavailable",
        ));
    }
    if !supported_owner || segments.len() > 2 {
        return Err((
            RefusalCode::UnsupportedScope,
            "name search cannot search this logical subtree",
        ));
    }
    if matches!(
        source_kind,
        SourceSetKind::Configuration | SourceSetKind::Extension
    ) && !has_address(&format!("{}:Configuration", scope.source_set()))
    {
        return Err((
            RefusalCode::InvalidState,
            "name search Configuration descriptor is unavailable",
        ));
    }
    if segments.len() == 2 {
        let child = &segments[1];
        if first.name().is_none()
            || !matches!(
                child.kind(),
                NodeKind::Form | NodeKind::Template | NodeKind::Command
            )
        {
            return Err((
                RefusalCode::UnsupportedScope,
                "name search cannot search this logical subtree",
            ));
        }
        let owner = format!(
            "{}:{}.{}",
            scope.source_set(),
            first.kind().as_str(),
            first.name().expect("checked owner name")
        );
        if !has_address(&owner) {
            return Err((
                RefusalCode::NotFound,
                "name search scope owner does not exist",
            ));
        }
    }
    if segments
        .last()
        .is_some_and(|segment| segment.name().is_some())
        && !has_address(&scope.to_string())
    {
        return Err((
            RefusalCode::NotFound,
            "name search scope owner does not exist",
        ));
    }
    Ok(())
}

fn error_result(at: Option<String>, code: RefusalCode, message: impl Into<String>) -> DomainResult {
    DomainResult::canonical_rejection(at, code, message)
}

fn enforce_view_result_limit(result: DomainResult, at: &str) -> DomainResult {
    // The application measures the projected collection before cutting it.
    // Keep the actual transport ceiling after later response enrichment.
    if !result.ok {
        return result;
    }
    let bytes =
        serde_json::to_vec(&result).expect("a domain result containing JSON values serializes");
    if bytes.len() <= unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES {
        result
    } else {
        error_result(
            Some(at.to_string()),
            RefusalCode::ResultTooLarge,
            "projected view result exceeds the transport result limit",
        )
    }
}

/// Отказ с уточнением там, где один код покрывает несколько исходов.
fn error_result_detailed(
    at: Option<String>,
    detail: RefusalDetail,
    message: impl Into<String>,
) -> DomainResult {
    DomainResult::canonical_rejection_detailed(at, detail, message)
}

/// Отказ чтения передаётся целиком, а не разбирается на код и текст: иначе
/// уточнение теряется по дороге и один код снова обслуживает несколько
/// исходов, не различая их.
fn view_error_result(
    at: Option<String>,
    error: unica_execution_contract::view_error::ViewError,
) -> DomainResult {
    let mut result = match error.detail() {
        Some(detail) => DomainResult::canonical_rejection_detailed(at, detail, error.to_string()),
        None => DomainResult::canonical_rejection(at, error.code(), error.to_string()),
    };
    // Маршрут переносится вместе с кодом и уточнением: иначе два пути наружу
    // расходятся, и отказ теряет альтернативу в зависимости от того, каким
    // из них он вышел.
    result.next.extend(error.next().iter().cloned());
    result
}

#[cfg(test)]
mod medium_plan_detail_key_tests {
    use super::{plan_detail_key_errors, PLAN_DETAIL_KEYS};
    use unica_source_contract::proposal::PlanDetailKey;

    #[test]
    fn registered_plan_detail_sections_are_unique_and_leave_answer_fields_alone() {
        assert!(PLAN_DETAIL_KEYS.contains(&PlanDetailKey::new("borrowing")));
        assert_eq!(
            plan_detail_key_errors(PLAN_DETAIL_KEYS),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_shared_or_shadowing_plan_detail_section_is_refused() {
        const FIRST: PlanDetailKey = PlanDetailKey::new("familyRows");
        const SECOND: PlanDetailKey = PlanDetailKey::new("familyRows");
        const SHADOW: PlanDetailKey = PlanDetailKey::new("planHash");
        assert_eq!(
            plan_detail_key_errors(&[FIRST, SECOND, SHADOW]),
            vec![
                "plan detail section `familyRows` is registered twice".to_string(),
                "plan detail section `planHash` shadows an apply answer field".to_string(),
            ]
        );
    }
}

#[cfg(test)]
mod medium_tests;

/// Ответ на `<узел>.Props.<имя>`: одно свойство узла в той же форме, что и
/// в его `props`, по каноническому адресу владельца. Свойства у узла нет —
/// отказ ведёт к самому узлу. Ветви, продолжения и переходы владельца к
/// ответу о свойстве не относятся.
fn property_view(result: DomainResult, at: &str, name: &str) -> DomainResult {
    if !result.ok {
        let mut result = result;
        result.at = Some(at.to_string());
        return result;
    }
    let owner = result.at.clone().unwrap_or_default();
    let property_at = format!("{owner}.Props.{name}");
    let value = result
        .data
        .as_ref()
        .and_then(|data| data.get("props"))
        .and_then(|props| props.get(name))
        .cloned();
    let Some(value) = value else {
        let mut refusal = error_result(
            Some(property_at),
            RefusalCode::NotFound,
            format!("node `{owner}` has no property `{name}`"),
        );
        refusal.next.push(json!({
            "tool": "unica.view",
            "args": {"at": owner},
            "reason": "свойства узла перечислены в его props",
        }));
        return refusal;
    };
    let mut props = Map::new();
    props.insert(name.to_string(), value);
    let mut answer = DomainResult::success("node property resolved");
    answer.at = Some(property_at.clone());
    answer.data = Some(json!({
        "at": property_at,
        "kind": NodeKind::Props.as_str(),
        "title": name,
        "props": props,
    }));
    answer.rev = result.rev;
    answer
}
