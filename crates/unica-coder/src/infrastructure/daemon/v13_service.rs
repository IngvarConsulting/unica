use super::server::{ActorBoundExecution, ActorBoundInvocation, CanonicalInvocationService};
use crate::domain::apply::OperationDescriptorExt;
use crate::domain::apply::OperationRegistryExt;
use crate::domain::apply::RetiredOperationExt;
use crate::domain::apply::SourceSetWriteSupport;
use crate::domain::engine::MissingEngineRefusal;
use unica_work_control::provider_deadline::ProviderDeadline;
/// Потолок страницы ветви графа: объявленный максимум поверхности.
const CALL_GRAPH_PAGE_LIMIT: usize = 50;
/// Ranked providers (`symbol`, `semantic`) accept a top-N request, but no
/// offset, and a larger N is another answer rather than a longer one (RLM
/// splits N into per-category quotas). Keep their
/// public bound as the fetch window and page the returned evidence
/// separately.
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
    attach_can, operation_node_with, search_scope_route, source_backed, SearchScopeRoute,
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
use crate::infrastructure::native_operations::apply_families::plan_hidden_v13_apply;
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
    use crate::infrastructure::platform::{TempFileSpill, VIEW_SNAPSHOT_MAX_RETAINED_BYTES};
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

impl CanonicalV13ReadService {
    #[cfg(test)]
    pub(super) fn with_view_cursor_budget_for_test(max_total_bytes: usize) -> Self {
        Self {
            cursors: Arc::new(ViewCursorStore::new(max_total_bytes).with_spill(Arc::new(
                crate::infrastructure::platform::TempFileSpill::default(),
            ))),
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

    fn execute_apply(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        if let Some(token) = invocation.arguments().get("executionToken") {
            if invocation.arguments().len() != 1 || token.as_str().is_none_or(str::is_empty) {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "execution requires only a nonempty executionToken",
                );
            }
            let deadline = unica_work_control::operation_deadline::OperationDeadline::NoDeadline;
            return invocation.execute_saved_apply(token.as_str().unwrap(), cancellation, deadline, |mut result, publication| {
                match publication {
                    Ok(publication) => {
                        if publication.effects().disposition() == ApplyEffectDisposition::Committed
                            && !publication.effects().events().is_empty()
                        {
                            use crate::application::ports::ApplicationPorts;
                            self.ports.notify_committed_changes(
                                invocation.workspace_context(),
                                publication.effects().events(),
                                cancellation,
                                deadline,
                            );
                        }
                        result.summary = "metadata apply published atomically".into();
                        result.rev = Some(publication.rev().to_owned());
                        if let Some(data) = result.data.as_mut() {
                            data["mode"] = json!("published");
                        }
                        result.next.clear();
                        if !publication.cleanup_diagnostics().is_empty() {
                            result.warnings.push(json!({"code": "retained_cleanup_incomplete", "count": publication.cleanup_diagnostics().len(), "message": "published apply left bounded internal recovery cleanup diagnostics"}));
                        }
                        result
                    }
                    Err(error) => error_result(result.at.clone(), apply_publication_error_code(error.kind()), format!("{error}; request a fresh plan with apply(at, ops)")),
                }
            }).unwrap_or_else(saved_apply_execution_error_result);
        }
        if invocation.arguments().contains_key("dryRun")
            || invocation.arguments().contains_key("ifRev")
        {
            return error_result(
                None,
                RefusalCode::BadValue,
                "apply accepts at and ops to prepare a plan, or only executionToken to execute it",
            );
        }
        let source_sets = invocation.admitted_source_set_names();
        let mut planning_arguments = invocation.arguments().clone();
        planning_arguments.insert("dryRun".into(), json!(true));
        let request = match parse_apply_request(&planning_arguments, &source_sets) {
            Ok(request) => request,
            Err(error) => {
                return error_result(
                    Some(error.location().to_string()),
                    error.code(),
                    error.to_string(),
                )
            }
        };
        // A line selector is a read address. Refuse before write admission
        // creates staging, even if an op overrides the top-level target.
        if let Some(ranged) = std::iter::once(request.at())
            .chain(request.ops().iter().map(|op| op.at()))
            .find(|at| at.line_range().is_some())
        {
            let mut refusal = error_result(Some(ranged.to_string()), RefusalCode::BadValue, "Body line ranges are read-only; replace a whole containing Method or use the module Body");
            if let Ok(source) = invocation.apply_range_refusal_source(ranged) {
                if let Ok(authority) = source.logical_view_read_authority(cancellation) {
                    let range = ranged.line_range().expect("selected ranged address");
                    if let Ok(body) =
                        authority.narrowest_body_for_lines(ranged, range.from, range.to)
                    {
                        let segments = body.segments();
                        let target = if segments.len() >= 2
                            && segments[segments.len() - 2].kind() == NodeKind::Method
                        {
                            body.prefix(segments.len() - 1)
                        } else {
                            body
                        };
                        refusal.next.push(json!({"tool":"unica.view","args":{"at":target.to_string()},"reason":"прочитать целый метод или Body модуля перед правкой"}));
                    }
                }
            }
            if refusal.next.is_empty() {
                let segments = ranged.segments();
                let module_body = if segments.len() >= 2
                    && segments[segments.len() - 2].kind() == NodeKind::Method
                {
                    format!("{}.Body", ranged.prefix(segments.len() - 2))
                } else {
                    ranged.without_line_range().to_string()
                };
                refusal.next.push(json!({"tool":"unica.view","args":{"at":module_body},"reason":"прочитать Body модуля и выбрать семантическую цель правки"}));
            }
            return refusal;
        }
        let (binding, mut admission) = match invocation.admit_apply(&request, cancellation) {
            Ok(admitted) => admitted,
            Err(ApplyAdmissionError::Other(error)) => {
                return error_result(
                    Some(request.at().to_string()),
                    RefusalCode::ProviderUnavailable,
                    error,
                )
            }
        };
        let traits = crate::domain::apply::source_set_traits_of_source(
            binding.source_kind(),
            binding.source_format(),
            binding.source_profile(),
        );
        let template_format = |template: &QualifiedAddress| {
            crate::infrastructure::native_operations::apply_families::staged_template_format(
                &admission, &binding, template,
            )
        };
        let member_path = |container: &QualifiedAddress, name: &str| {
            crate::infrastructure::native_operations::apply_families::staged_member_path(
                &admission, &binding, container, name,
            )
        };
        let form_item = |item: &QualifiedAddress| {
            crate::infrastructure::native_operations::apply_families::staged_form_item_role(
                &admission, &binding, item,
            )
        };
        if let Err(refusal) = preflight_apply_operations_with(
            &request,
            traits,
            &template_format,
            &form_item,
            &member_path,
        ) {
            return *refusal;
        }
        admission.bind_request_identity(
            &request.at().to_string(),
            request.ops().iter().map(|operation| {
                (
                    operation.name(),
                    operation.at().to_string(),
                    operation.args(),
                )
            }),
        );
        let (staged, mut effects) = match plan_hidden_v13_apply(&request, &binding, &admission) {
            Err(error)
                if error.kind() == ApplyPlanErrorKind::ProviderUnavailable
                    && error.to_string().contains("not implemented") =>
            {
                return error_result(
                    error.path().map(str::to_string),
                    RefusalCode::UnsupportedOperation,
                    "canonical v0.13 apply operation is not implemented",
                )
            }
            Err(error) => {
                let refusal = apply_plan_error_result(&error);
                let node_exists = |at: &QualifiedAddress| {
                    crate::infrastructure::native_operations::apply_families::staged_node_existence(
                        &admission, &binding, at,
                    )
                };
                return with_missing_target_parent(refusal, &request, &error, &node_exists);
            }
            Ok(planned) => planned,
        };
        let borrowing = effects.borrowing().to_vec();
        let plan_details = effects
            .details()
            .map(|(key, rows)| (key, rows.to_vec()))
            .collect::<Vec<_>>();
        let has_changes = !staged.planned_changes().is_empty();
        if !has_changes {
            effects = Default::default();
        }
        let plan_warnings = effects.warnings().to_vec();
        // The answer does not echo the request: only an operation that
        // attached a warning is named, so the answer does not grow with `ops`.
        let operations = effects
            .warned_operations()
            .iter()
            .filter_map(|index| {
                request
                    .ops()
                    .get(*index)
                    .map(|operation| (index, operation))
            })
            .map(|(index, operation)| {
                serde_json::json!({
                    "index": index,
                    "op": operation.name(),
                    "at": operation.at(),
                })
            })
            .collect::<Vec<_>>();
        let plan_hash = apply_plan_hash(&staged, &effects);
        let changed = effects
            .events()
            .iter()
            .map(|event| {
                let at = if event.artifact.contains(':') {
                    event.artifact.clone()
                } else {
                    format!("{}:{}", request.at().source_set(), event.artifact)
                };
                serde_json::json!({
                    "at": at,
                    "event": event.name(),
                })
            })
            .collect::<Vec<_>>();
        let prepared = match admission.prepare_with_effects(staged, effects) {
            Ok(prepared) => prepared,
            Err(error) => {
                return apply_staging_refusal(
                    Some("ops".to_string()),
                    error.kind(),
                    error.to_string(),
                )
            }
        };
        let publication = match invocation.preview_prepared_apply(&prepared) {
            Ok(publication) => publication,
            Err(error) => {
                return error_result(
                    Some(request.at().to_string()),
                    apply_publication_error_code(error.kind()),
                    error.to_string(),
                )
            }
        };
        let disposition = match publication.effects().disposition() {
            ApplyEffectDisposition::Projected => "preview",
            ApplyEffectDisposition::Committed => "published",
        };
        let mut result = DomainResult::success(match publication.effects().disposition() {
            ApplyEffectDisposition::Projected => "metadata apply plan prepared without publication",
            ApplyEffectDisposition::Committed => "metadata apply published atomically",
        });
        result.at = Some(request.at().to_string());
        result.data = Some(serde_json::json!({
            "validated": true,
            "mode": disposition,
            "executable": true,
            "operationCount": request.ops().len(),
            "planHash": plan_hash,
            "effects": publication.effects().events().len(),
            "cache": publication.effects().cache(),
        }));
        if !operations.is_empty() {
            result.data.as_mut().expect("apply data exists")["operations"] =
                serde_json::Value::Array(operations);
        }
        if !borrowing.is_empty() {
            result.data.as_mut().expect("apply data exists")["borrowing"] =
                serde_json::json!(borrowing);
        }
        for (key, rows) in plan_details {
            debug_assert!(
                PLAN_DETAIL_KEYS.contains(&key),
                "plan detail section `{}` is not registered",
                key.as_str()
            );
            result.data.as_mut().expect("apply data exists")[key.as_str()] =
                serde_json::Value::Array(rows);
        }
        if has_changes {
            result.changed = changed;
        }
        result.warnings.extend(plan_warnings);
        if !publication.cleanup_diagnostics().is_empty() {
            result.warnings.push(serde_json::json!({
                "code": "retained_cleanup_incomplete",
                "count": publication.cleanup_diagnostics().len(),
                "message": "published apply left bounded internal recovery cleanup diagnostics"
            }));
        }
        result.rev = Some(publication.rev().to_string());
        match invocation.save_prepared_apply(prepared, result.clone()) {
            Ok(token) => {
                result.data.as_mut().expect("apply plan data exists")["executionToken"] =
                    json!(token);
                result
                    .next
                    .push(json!({"tool": "unica.diff", "arguments": {"executionToken": token}}));
                result
                    .next
                    .push(json!({"tool": "unica.apply", "arguments": {"executionToken": token}}));
                result
            }
            Err(error) => error_result(
                result.at.clone(),
                RefusalCode::InvalidState,
                error.to_string(),
            ),
        }
    }

    fn execute_view(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        self.execute_view_arguments(invocation, invocation.arguments(), cancellation)
    }

    fn execute_view_arguments(
        &self,
        invocation: &ActorBoundExecution,
        arguments: &Map<String, Value>,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let Some(at) = arguments.get("at").and_then(Value::as_str) else {
            return error_result(
                None,
                RefusalCode::BadValue,
                "view requires string argument `at`",
            );
        };
        // `<узел>.Can.<op>` спрашивает контракт одной операции словаря узла:
        // читается сам узел, а ответ — его операция.
        let mut operation = None;
        // `<узел>.Props.<имя>` читает одно свойство узла целиком (#2088):
        // узел проецируется без предела размера, ответ — это свойство.
        let mut property = None;
        let read_at = match QualifiedAddress::parse(at) {
            Ok(address) if address.property_query().is_some() => {
                let (owner, name) = address.property_query().expect("checked above");
                // A property is one value: like any single node it accepts a
                // page size it does not need, but it has no continuation.
                if arguments.contains_key("cursor") {
                    return error_result(
                        Some(at.to_string()),
                        RefusalCode::BadValue,
                        "a node property is one value and has no pages; omit cursor",
                    );
                }
                property = Some(name.to_string());
                owner.to_string()
            }
            Ok(address) => match address.operation_query() {
                Some((owner, Some(op))) => {
                    // A contract is one node: like a source node it accepts a
                    // page size it does not need, but it has no continuation.
                    if arguments.contains_key("cursor") {
                        return error_result(
                            Some(at.to_string()),
                            RefusalCode::BadValue,
                            "an operation contract is one node and has no pages; omit cursor",
                        );
                    }
                    operation = Some(op.to_string());
                    owner.to_string()
                }
                Some((owner, None)) => {
                    let mut refusal = error_result(
                        Some(at.to_string()),
                        RefusalCode::BadValue,
                        "Can names one operation: `<node>.Can.<op>`; the node answer lists its operations in can",
                    );
                    refusal.next.push(json!({
                        "tool": "unica.view",
                        "args": {"at": owner.to_string()},
                        "reason": "операции узла перечислены в его can",
                    }));
                    return refusal;
                }
                None => at.to_string(),
            },
            Err(_) => at.to_string(),
        };
        let mut request = match ViewRequest::new(&read_at) {
            Ok(request) => request,
            Err(error) => return view_error_result(Some(at.to_string()), error),
        };
        if let Some(limit) = arguments.get("limit") {
            let Some(limit) = bounded_usize(limit) else {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::BadValue,
                    "view limit must be a positive integer",
                );
            };
            request = match request.with_limit(limit) {
                Ok(request) => request,
                Err(error) => return view_error_result(Some(at.to_string()), error),
            };
        }
        if let Some(cursor) = arguments.get("cursor") {
            let Some(cursor) = cursor.as_str() else {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::BadValue,
                    "view cursor must be a string",
                );
            };
            request = request.with_cursor(cursor.to_string());
        }
        let address = match QualifiedAddress::parse(&read_at) {
            Ok(address) => address,
            Err(error) => {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::BadValue,
                    error.to_string(),
                )
            }
        };
        let sources = match invocation.read_sources() {
            Ok(sources) => sources,
            Err(error) => {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::ProviderUnavailable,
                    error,
                )
            }
        };
        let Some(source) = sources
            .into_iter()
            .find(|source| source.source_set_name() == address.source_set())
        else {
            return error_result(
                Some(at.to_string()),
                RefusalCode::ProviderUnavailable,
                "view source set was not admitted by the workspace actor",
            );
        };
        #[cfg(test)]
        if request.cursor().is_none() {
            observe_fresh_view_capability_for_test(&address.to_string(), || {
                source.revision_identity()
            });
        }
        let authority = match source.logical_view_read_authority(cancellation) {
            Ok(authority) => authority,
            Err(error) => {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::ProviderUnavailable,
                    error,
                )
            }
        };
        // Ветвь графа вызовов не живёт в дереве исходников, поэтому читатель о
        // ней не знает: страницу собирает служба, а существование владельца
        // доказывает тот же читатель обычным чтением узла метода.
        if let Some(direction) = branch_direction(&address) {
            let Some(owner) = branch_owner(&address) else {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::NotFound,
                    "call graph branch has no owning method",
                );
            };
            if operation.is_some() {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::UnsupportedSection,
                    "a call graph branch has no operation dictionary",
                );
            }
            // Ветвь метода, у которого графа нет (метод HTTP-сервиса, модуль
            // непокрытой роли), отказывает по адресу, до чтения владельца и заказа поставки: ни скачивание,
            // ни наличие движка не меняют этого ответа.
            if let Err(message) = call_graph_support(&owner) {
                return error_result(Some(at.to_string()), RefusalCode::BadValue, message);
            }
            let service = ViewService::with_shared_cursors(authority, Arc::clone(&self.cursors));
            if request.cursor().is_some() {
                return enforce_view_result_limit(
                    service.view_saved_collection(request, &owner),
                    at,
                );
            }
            let owner_result = service.view(match ViewRequest::new(&owner.to_string()) {
                Ok(request) => request,
                Err(error) => return view_error_result(Some(owner.to_string()), error),
            });
            if !owner_result.ok {
                return owner_result;
            }
            let Some(expected_revision) = owner_result.rev.as_deref() else {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::InvalidResult,
                    "owning method has no source revision",
                );
            };
            // Ветвь — единственный вход в граф, и анализатор, которого нет на
            // машине, она поставляет этим же вызовом, как `check` и `search`:
            // пока поставка идёт, ответ просит повтор, провал поставки — её
            // типизированный отказ, а движок, которого и после поставки нет,
            // — общий отказ «нет поставляемого движка» (#549), а не граф
            // «недоступен».
            // Тестовый поставщик графа заменяет анализатор: заказывать его
            // поставку тогда незачем, и тест не зависит от окружения.
            #[cfg(not(test))]
            let analyzer_engine = true;
            #[cfg(test)]
            let analyzer_engine =
                crate::infrastructure::application_ports::CODE_INTELLIGENCE_PROVIDER_OVERRIDE
                    .with(|slot| slot.borrow().is_none());
            let cwd = &invocation.workspace_context().cwd;
            if analyzer_engine {
                if let Err(refusal) = acquire_engine(
                    invocation,
                    cwd,
                    BSL_ANALYZER_ENGINE,
                    Some(at.to_string()),
                    cancellation,
                ) {
                    return *refusal;
                }
            }
            return match self.call_graph_summary(
                invocation,
                &owner,
                CALL_GRAPH_PAGE_LIMIT,
                cancellation,
            ) {
                Ok(initial) => {
                    let (summary, directory) = match complete_branch(initial, direction, |limit| {
                        self.call_graph_summary(invocation, &owner, limit, cancellation)
                    }) {
                        Ok(answer) => answer,
                        Err(CompleteBranchError::CountTooLarge(message)) => {
                            return error_result(
                                Some(at.to_string()),
                                RefusalCode::ResultTooLarge,
                                message,
                            )
                        }
                        Err(CompleteBranchError::Fetch(refusal)) => return *refusal,
                        Err(CompleteBranchError::Changed) => {
                            return error_result(
                                Some(at.to_string()),
                                RefusalCode::ConcurrentChange,
                                "call graph changed while collecting its page; retry",
                            )
                        }
                        Err(CompleteBranchError::Incomplete) => {
                            return error_result(
                                Some(at.to_string()),
                                RefusalCode::InvalidResult,
                                "call graph provider did not return the complete branch",
                            )
                        }
                    };
                    // Анализатор не ответил на ветвь: движок, не прошедший
                    // проверку поставки или не запустившийся, — тот же отказ
                    // «нет движка», что на всех маршрутах (#549).
                    if analyzer_engine
                        && summary.result(direction).state == CallGraphState::Unavailable
                    {
                        if let Some(refusal) = engine_failure_refusal(
                            cwd,
                            BSL_ANALYZER_ENGINE,
                            Some(at.to_string()),
                            cancellation,
                        ) {
                            return refusal;
                        }
                    }
                    if summary.result(direction).state != CallGraphState::Ready {
                        return enforce_view_result_limit(
                            unready_branch_result(&address, direction, &summary),
                            at,
                        );
                    }
                    // Сосед, которого анализатор называет файлом, получает адрес
                    // через раскладку: файл модуля → дескриптор → узел → роль
                    // модуля. Раскладка строится один раз и только когда такой
                    // сосед есть; кого раскладка не размещает, тот остаётся в
                    // `limits`.
                    let directory = match directory {
                        Some(directory) => Some(directory),
                        None if summary.names_a_peer_by_file(direction) => {
                            match self.layout_directory(
                                invocation,
                                Some(owner.to_string()),
                                owner.source_set(),
                                cancellation,
                            ) {
                                Ok(directory) => Some(directory),
                                Err(refusal) => return *refusal,
                            }
                        }
                        None => None,
                    };
                    let source_set = owner.source_set();
                    let data = branch_collection(&address, direction, &summary, |path| {
                        let directory = directory.as_ref()?;
                        let (placed, role) = placed_for_module_file(path)?;
                        let entry = directory.locate_path(&placed)?;
                        let entry = entry.owner;
                        if entry.at().split(':').next() != Some(source_set) {
                            return None;
                        }
                        QualifiedAddress::parse(&format!("{}.Module.{role}", entry.at())).ok()
                    });
                    let mut answer = service.view_preloaded_collection(
                        request,
                        &owner,
                        expected_revision,
                        Some(data),
                    );
                    if let Some(reason) = summary.reason {
                        answer.warnings.push(json!({"callGraph": reason}));
                    }
                    enforce_view_result_limit(answer, at)
                }
                Err(refusal) => *refusal,
            };
        }
        let can_context = crate::domain::apply::source_set_traits_for_read(
            source.source_kind(),
            authority.platform_profile(),
        );
        let service = ViewService::with_shared_cursors(authority, Arc::clone(&self.cursors));
        let enrich_and_project = |data: &Value| -> Result<Value, ViewError> {
            let mut data = data.clone();
            if source.source_kind() == unica_source_contract::SourceSetKind::Extension
                && data
                    .get("props")
                    .is_some_and(|props| props.get("belonging").is_none())
            {
                let borrowing = source
                    .logical_view_read_authority(cancellation)
                    .map_err(|message| ViewError::detailed(RefusalDetail::ProviderAbsent, message))?
                    .object_borrowing(&address)?;
                if let Some(borrowing) = borrowing {
                    if let Some(props) = data.get_mut("props").and_then(Value::as_object_mut) {
                        props.extend(crate::infrastructure::v13_read_projection::borrowing_props(
                            &borrowing,
                        ));
                    }
                }
            }
            self.resolve_borrowed_parent(invocation, &address, &mut data, cancellation);
            // Узел метода называет ветви графа, не спрашивая анализатор: граф
            // читается только адресом ветви.
            if let Some(node) = data.as_object_mut() {
                declare_method_branches(node);
            }
            match operation.as_deref() {
                Some(op) => {
                    let owner_at = data
                        .get("at")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    // Роль элемента-предка читается его узлом: маршрут снятой
                    // операции не ведёт в поле, куда элемент не добавить.
                    let ancestor_role = |item: &QualifiedAddress| {
                        if item.segments().last()?.kind() != NodeKind::Item {
                            return None;
                        }
                        let read = service.view(ViewRequest::new(&item.to_string()).ok()?);
                        read.data?
                            .get("props")?
                            .get("tag")?
                            .as_str()
                            .map(unica_forms::FormItemRole::from_tag)
                    };
                    operation_node_with(&data, op, &owner_at, can_context, &ancestor_role).map_err(
                        |error| {
                            let mut refused = ViewError::new(error.code(), error.to_string());
                            // Снятая операция ведёт к контракту заменившей (#1465).
                            refused.set_next(match error.next() {
                                Some(contract) => json!({
                                    "tool": "unica.view",
                                    "args": {"at": contract},
                                    "reason": "контракт операции, заменившей снятую",
                                }),
                                None => json!({
                                    "tool": "unica.view",
                                    "args": {"at": owner_at},
                                    "reason": "операции, применимые к этому узлу",
                                }),
                            });
                            refused
                        },
                    )
                }
                None => {
                    attach_can(&mut data, can_context);
                    Ok(data)
                }
            }
        };
        let mut result = match property.as_deref() {
            Some(name) => property_view(
                crate::infrastructure::v13_read_projection::with_whole_prop(name, || {
                    service.view_projected(request, &enrich_and_project)
                }),
                at,
                name,
            ),
            None => service.view_projected(request, &enrich_and_project),
        };
        // Ответ операции адресован ею самой, а не узлом, который читался.
        if let Some(op) = operation.as_deref() {
            let owner = result.at.clone();
            match (result.ok, owner) {
                (true, Some(owner)) => {
                    let can_at = format!("{owner}.Can.{op}");
                    if let Some(data) = result.data.as_mut().and_then(Value::as_object_mut) {
                        data.insert("at".to_string(), Value::String(can_at.clone()));
                    }
                    result.at = Some(can_at);
                }
                _ => result.at = Some(at.to_string()),
            }
        }
        enforce_view_result_limit(result, at)
    }

    /// Сводка графа вызовов для одного метода.
    ///
    /// Личность узла строится из адреса; модуль формы или команды анализатор
    /// называет файлом, и путь к нему даёт раскладка — та же, которой живёт
    /// `resolve`. Раскладка строится только для таких адресов и возвращается
    /// вызывающему, чтобы соседей, названных файлом, не переводить второй
    /// сборкой.
    fn call_graph_summary(
        &self,
        invocation: &ActorBoundExecution,
        method_at: &QualifiedAddress,
        limit: usize,
        cancellation: &CancellationToken,
    ) -> Result<(CallGraphSummary, Option<FindIndex>), Box<DomainResult>> {
        let mut directory = None;
        let identity = match CallGraphIdentity::from_address(method_at, None) {
            Err(CallGraphIdentityError::PathRequired) => {
                let built = self.layout_directory(
                    invocation,
                    Some(method_at.to_string()),
                    method_at.source_set(),
                    cancellation,
                )?;
                let owner = owning_metadata_address(method_at);
                let path = built
                    .locate_address(&owner)
                    .and_then(|entry| module_file_for_placed(entry.kind(), entry.placed_path()?))
                    .ok_or_else(|| {
                        Box::new(error_result(
                            Some(method_at.to_string()),
                            RefusalCode::NotFound,
                            format!(
                                "the source layout does not place the module file of `{owner}`"
                            ),
                        ))
                    })?;
                let identity = CallGraphIdentity::from_address(method_at, Some(&path));
                directory = Some(built);
                identity
            }
            other => other,
        };
        let identity = identity.map_err(|error| {
            Box::new(error_result(
                Some(method_at.to_string()),
                RefusalCode::BadValue,
                match error {
                    CallGraphIdentityError::PathRequired => {
                        "the analyzer names this module by file, and its path could not be resolved"
                            .to_string()
                    }
                    CallGraphIdentityError::UnsupportedRole(role) => {
                        format!("call graph does not cover module role `{role}` yet")
                    }
                    CallGraphIdentityError::NotAMethod => {
                        "call graph answers for a method node only".to_string()
                    }
                    CallGraphIdentityError::Malformed(detail) => detail,
                },
            ))
        })?;
        let workspace = invocation.workspace_context();
        let operational = crate::infrastructure::operational_config::load_operational_config(
            &workspace.workspace_root,
        )
        .map_err(|diagnostic| {
            Box::new(error_result(
                Some(method_at.to_string()),
                RefusalCode::InvalidState,
                format!("operational config is unreadable: {diagnostic}"),
            ))
        })?;
        let summary = fetch_summary(
            self.ports.as_ref(),
            workspace,
            method_at.source_set(),
            &identity,
            // Первая попытка берёт не более 50 соседей. Ветвь с большим
            // счётом повторяет запрос для полного снимка до публикации курсора.
            limit,
            operational.code_intelligence().provider_read_timeout(),
            cancellation,
        )
        .map_err(|error| {
            let (code, message) = match error {
                CallGraphFetchError::Provider(message) => {
                    (RefusalCode::ProviderUnavailable, message)
                }
                CallGraphFetchError::Changed => (
                    RefusalCode::ConcurrentChange,
                    "call graph changed between directions; retry the question".to_string(),
                ),
            };
            Box::new(error_result(Some(method_at.to_string()), code, message))
        })?;
        Ok((summary, directory))
    }

    fn resolve_borrowed_parent(
        &self,
        invocation: &ActorBoundExecution,
        address: &QualifiedAddress,
        data: &mut Value,
        cancellation: &CancellationToken,
    ) {
        let Some(data) = data.as_object_mut() else {
            return;
        };
        let Some(owner) = address
            .segments()
            .first()
            .filter(|_| address.segments().len() == 1)
        else {
            return;
        };
        let kind = owner.kind().as_str();
        let Some(props) = data.get_mut("props").and_then(Value::as_object_mut) else {
            return;
        };
        let Some(parent_id) = props
            .get("parentId")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return;
        };
        let mut matches = Vec::new();
        let sources = match invocation
            .admit_borrowing_parent_sources(cancellation)
            .and_then(|()| {
                invocation.read_sources().map(|sources| {
                    sources
                        .into_iter()
                        .filter(|source| {
                            source.source_kind()
                                == unica_source_contract::SourceSetKind::Configuration
                        })
                        .collect::<Vec<_>>()
                })
            }) {
            Ok(sources) if !sources.is_empty() => sources,
            _ => {
                props.insert("parentStatus".into(), json!("unavailable"));
                return;
            }
        };
        for source in sources {
            let candidate = source
                .logical_view_read_authority(cancellation)
                .map_err(|_| ())
                .and_then(|authority| {
                    authority
                        .borrowed_parent_matches(&parent_id, kind)
                        .map_err(|_| ())
                });
            match candidate {
                Ok(found) => matches.extend(found),
                Err(_) => {
                    props.insert("parentStatus".into(), json!("unavailable"));
                    return;
                }
            }
        }
        let status = match matches.as_slice() {
            [parent] => {
                props.insert("extends".into(), json!(parent));
                "resolved"
            }
            [] => "not_found",
            _ => "ambiguous",
        };
        props.insert("parentStatus".into(), json!(status));
    }

    /// Справочник раскладки одного допущенного набора — тот же, что строит
    /// `resolve`: адрес ↔ место в исходниках.
    fn layout_directory(
        &self,
        invocation: &ActorBoundExecution,
        at: Option<String>,
        source_set: &str,
        cancellation: &CancellationToken,
    ) -> Result<FindIndex, Box<DomainResult>> {
        let source = admitted_read_source(
            invocation.read_sources(),
            |source| source.source_set_name(),
            at.as_deref(),
            source_set,
        )?;
        let root = source.retained_root();
        let layout = vec![LayoutFindSource::new(
            source.source_set_name(),
            source.source_kind(),
            root.as_ref(),
        )];
        self.find_builder
            .build(&layout, source.deadline(), cancellation)
            .map_err(|error| Box::new(find_build_error_result(at, error)))
    }

    fn execute_search(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let arguments = invocation.arguments();
        let Some(query) = arguments.get("query").and_then(Value::as_str) else {
            return error_result(
                None,
                RefusalCode::BadValue,
                "search requires string argument `query`",
            );
        };
        if query.trim().is_empty() {
            return error_result(
                None,
                RefusalCode::BadValue,
                "search query must not be blank",
            );
        }
        // Корпус выбирает, где искать, а не как: имена метаданных и текст
        // BSL — это один вопрос над разными сводами. Умолчание оставлено
        // текстовым, потому что таким `search` был до появления второго
        // корпуса.
        let corpus = match arguments.get("corpus") {
            None | Some(Value::String(_)) => match arguments
                .get("corpus")
                .and_then(Value::as_str)
                .unwrap_or("text")
            {
                "text" => SearchCorpus::Text,
                "names" => SearchCorpus::Names,
                other => {
                    let mut refusal = error_result(
                        None,
                        RefusalCode::BadValue,
                        format!("search corpus `{other}` is unknown; use `text` or `names`"),
                    );
                    refusal.next.push(serde_json::json!({
                        "tool": "unica.search",
                        "args": {"query": query, "corpus": "names"},
                        "reason": "поиск по именам и синонимам метаданных",
                    }));
                    return refusal;
                }
            },
            Some(_) => {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "search corpus must be a string",
                )
            }
        };
        if corpus == SearchCorpus::Names {
            if arguments.contains_key("role") {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "name search does not accept a provider role",
                );
            }
            return self.execute_search_names(invocation, query, arguments, cancellation);
        }
        // Роль выбирает, чем искать по тексту: точным совпадением своими
        // силами или провайдером — символьным индексом, смысловым поиском.
        // Без роли поиск остаётся буквальным, каким был.
        if let Some(role) = arguments.get("role") {
            let Some(role) = role.as_str() else {
                return error_result(None, RefusalCode::BadValue, "search role must be a string");
            };
            return self.execute_search_role(invocation, query, role, arguments, cancellation);
        }
        let flag = |name: &str| match arguments.get(name) {
            None | Some(Value::Bool(false)) => Ok(false),
            Some(Value::Bool(true)) => Ok(true),
            Some(_) => Err(format!("search {name} must be a boolean")),
        };
        let (regex, ignore_case) = match (flag("regex"), flag("ignoreCase")) {
            (Ok(regex), Ok(ignore_case)) => (regex, ignore_case),
            (Err(error), _) | (_, Err(error)) => {
                return error_result(None, RefusalCode::BadValue, error)
            }
        };
        let text_query = match TextQuery::new(query, regex, ignore_case) {
            Ok(text_query) => text_query,
            Err(error) => return error_result(None, RefusalCode::BadValue, error),
        };
        let limit = match arguments.get("limit") {
            Some(value) => match bounded_usize(value).filter(|limit| *limit <= 50) {
                Some(limit) => limit,
                None => {
                    return error_result(
                        None,
                        RefusalCode::BadValue,
                        "search limit must be an integer from 1 through 50",
                    )
                }
            },
            None => 20,
        };
        let scope = match arguments.get("scope") {
            None => None,
            Some(Value::String(scope)) => match QualifiedAddress::parse(scope) {
                Ok(address) => Some(address),
                Err(error) => {
                    return error_result(
                        Some(scope.to_string()),
                        RefusalCode::BadValue,
                        error.to_string(),
                    )
                }
            },
            Some(_) => {
                return error_result(None, RefusalCode::BadValue, "search scope must be a string")
            }
        };
        let sources = match invocation.read_sources() {
            Ok(sources) => sources,
            Err(error) => return error_result(None, RefusalCode::ProviderUnavailable, error),
        };
        let selected = sources
            .iter()
            .filter(|source| {
                scope
                    .as_ref()
                    .is_none_or(|scope| source.source_set_name() == scope.source_set())
            })
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return error_result(
                scope.map(|scope| scope.to_string()),
                RefusalCode::NotFound,
                "search scope does not name an admitted source set",
            );
        }
        let route = match scope.as_ref() {
            None => None,
            Some(scope) => match self.admit_text_scope(scope, selected[0], cancellation) {
                Ok(route) => Some(route),
                Err(refusal) => return *refusal,
            },
        };
        let mode = text_query.mode;
        let binding = SearchCursorBinding {
            workspace_identity: invocation.workspace_identity_hash().as_str().to_owned(),
            query: query.to_owned(),
            scope: scope.as_ref().map(ToString::to_string),
            mode: text_search_binding_mode(mode, ignore_case),
            kind: None,
            source_sets: selected
                .iter()
                .map(|source| source.source_set_name().to_owned())
                .collect(),
            result_fingerprint: None,
            page_limit: limit,
        };
        let cursor_token = match arguments.get("cursor") {
            None => None,
            Some(Value::String(token)) => Some(token.as_str()),
            Some(_) => {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "search cursor must be a string",
                )
            }
        };
        let (offset, continuation) = match cursor_token {
            None => (0, TextContinuation::Start),
            Some(token) => match self.search_cursors.resume_position(token, &binding) {
                Ok((offset, resume)) if !resume.file.is_empty() => {
                    (offset, TextContinuation::Resume(resume))
                }
                // Every continuation names the file it re-enters.
                Ok(_) => return SearchCursorError::Stale.into_result(None),
                Err(error) => return error.into_result(None),
            },
        };
        let text_sources = selected
            .iter()
            .map(|source| {
                let root = source.retained_root().path().to_path_buf();
                TextSource {
                    source,
                    scope: scope.as_ref().map_or_else(
                        || format!("{}:Configuration", source.source_set_name()),
                        ToString::to_string,
                    ),
                    prefix: route.as_ref().and_then(|route| route.prefix.clone()),
                    external_binding: route
                        .as_ref()
                        .and_then(|route| route.external_binding.clone()),
                    // A nested source set owns its files; its parent skips them.
                    excluded: sources
                        .iter()
                        .filter_map(|other| {
                            let other = other.retained_root();
                            let nested = other.path().strip_prefix(&root).ok()?;
                            (!nested.as_os_str().is_empty())
                                .then(|| nested.to_string_lossy().replace('\\', "/"))
                        })
                        .collect(),
                }
            })
            .collect::<Vec<_>>();
        // Git reads a first page fastest; a continuation re-enters its file
        // through the walk, which does not read the delivered part again.
        let engine = match continuation {
            TextContinuation::Start => choose_engine(&text_query, &text_sources, cancellation),
            TextContinuation::Resume(_) => TextEngine::Walk,
        };
        #[cfg(test)]
        let engine = self
            .text_engine
            .filter(|_| text_query.candidates.is_some() && continuation == TextContinuation::Start)
            .unwrap_or(engine);
        // The first page counts the whole corpus and so never ends by time:
        // a long one becomes a Task. A continuation ends at a share of the
        // direct answer's budget.
        let count_totals = continuation == TextContinuation::Start;
        let walk_budget = {
            let remaining =
                invocation.remaining_response_budget() / WALK_PAGE_RESPONSE_BUDGET_DIVISOR;
            (!count_totals && !remaining.is_zero()).then_some(remaining)
        };
        #[cfg(test)]
        let walk_budget = self.walk_budget.filter(|_| !count_totals).or(walk_budget);
        let request = TextPageRequest {
            query: &text_query,
            sources: text_sources,
            module: route
                .as_ref()
                .and_then(|route| route.module.as_ref())
                .map(ToString::to_string),
            wanted: limit + 1,
            engine,
            continuation,
            walk_budget,
            count_totals,
            #[cfg(test)]
            cancel_past_page: self.cancel_past_page,
        };
        let scan = match scan_text_page(&request, cancellation) {
            Ok(scan) => scan,
            Err(TextSearchError::Interrupted(error)) => return view_error_result(None, error),
            Err(TextSearchError::Engine(error)) => {
                return error_result(None, RefusalCode::ProviderUnavailable, error)
            }
        };
        let stored = match cursor_token {
            None => None,
            Some(token) => {
                // The traversal must still reach the file the cursor
                // re-enters and the matches it delivered there.
                let Some(passed) = scan.passed.clone() else {
                    return SearchCursorError::Stale.into_result(None);
                };
                let mut answered = binding.clone();
                answered.result_fingerprint = Some(passed);
                match self.search_cursors.read(token, &answered) {
                    Ok(stored) => Some(stored),
                    Err(error) => return error.into_result(None),
                }
            }
        };
        self.text_search_page(&sources, scan, binding, stored, offset, mode, cancellation)
    }

    /// Checks a text-search scope against the admitted source set before
    /// any source is read, and routes it to a subtree and a module.
    fn admit_text_scope(
        &self,
        scope: &QualifiedAddress,
        source: &super::server::ActorReadSourceCapability,
        cancellation: &CancellationToken,
    ) -> Result<SearchScopeRoute, Box<DomainResult>> {
        let refuse = |result: DomainResult| Box::new(result);
        let mut route = search_scope_route(scope, source.source_kind()).map_err(|error| {
            refuse(error_result(
                Some(scope.to_string()),
                error.code(),
                error.to_string(),
            ))
        })?;
        // A module is admitted through its owner: the registered object, or
        // the configuration for its own application modules.
        let owner = match route.module {
            None => scope.clone(),
            Some(_) => {
                let first = &scope.segments()[0];
                let owner = match first.name() {
                    Some(name) if first.kind() != NodeKind::Module => {
                        format!("{}:{}.{name}", scope.source_set(), first.kind().as_str())
                    }
                    _ => format!("{}:Configuration", scope.source_set()),
                };
                QualifiedAddress::parse(&owner).expect("a module owner is a canonical address")
            }
        };
        let authority = source
            .logical_view_read_authority(cancellation)
            .map_err(|error| {
                refuse(error_result_detailed(
                    Some(scope.to_string()),
                    RefusalDetail::ProviderAbsent,
                    error,
                ))
            })?;
        let snapshot = authority
            .snapshot(&owner)
            .map_err(|error| refuse(view_error_result(Some(scope.to_string()), error)))?;
        authority
            .validate_search_scope_owner(&owner, &snapshot)
            .map_err(|error| refuse(view_error_result(Some(scope.to_string()), error)))?;
        if matches!(
            source.source_kind(),
            unica_source_contract::SourceSetKind::ExternalProcessor
                | unica_source_contract::SourceSetKind::ExternalReport
        ) {
            if let Some(name) = owner.segments()[0].name() {
                let descriptor = authority
                    .existing_export_path(&owner, None)
                    .map_err(|error| refuse(view_error_result(Some(scope.to_string()), error)))?;
                let binding =
                    unica_source_designer::external_artifact::ExternalArtifactBinding::new(
                        source.source_kind(),
                        name,
                        std::path::Path::new(&descriptor),
                    )
                    .map_err(|error| {
                        refuse(error_result(
                            Some(scope.to_string()),
                            RefusalCode::InvalidSource,
                            error,
                        ))
                    })?;
                route.prefix = Some(binding.payload().to_string_lossy().replace('\\', "/"));
                route.external_binding = Some(binding);
            }
        }
        if owner.segments()[0].kind() == NodeKind::Configuration
            || owner.segments()[0].name().is_none()
        {
            // The root and an unnamed kind branch both depend on the
            // configuration descriptor, but neither needs its projected
            // child collection before a text-search page is returned.
            let root_at = QualifiedAddress::parse(&format!("{}:Configuration", scope.source_set()))
                .expect("Configuration is a canonical root address");
            let descriptor = authority
                .identity_export_path(&root_at)
                .map_err(|error| refuse(view_error_result(Some(scope.to_string()), error)))?;
            if let Some(descriptor) = descriptor {
                if let Err(error) = source
                    .retained_root()
                    .read_relative_regular_prefix(std::path::Path::new(&descriptor), 1)
                {
                    let message = format!("search scope descriptor is unavailable: {error}");
                    return Err(refuse(if error.kind() == std::io::ErrorKind::NotFound {
                        error_result(Some(scope.to_string()), RefusalCode::InvalidState, message)
                    } else {
                        error_result_detailed(
                            Some(scope.to_string()),
                            RefusalDetail::SourceUnreadable,
                            message,
                        )
                    }));
                }
            }
        }
        Ok(route)
    }

    /// One page of a text-search scan: method-narrowed addresses, the byte
    /// and count bounds, and the continuation after the last delivered hit.
    #[allow(clippy::too_many_arguments)]
    fn text_search_page(
        &self,
        sources: &[super::server::ActorReadSourceCapability],
        scan: TextScan,
        mut binding: SearchCursorBinding,
        stored: Option<StoredSearchCursor>,
        offset: usize,
        mode: &str,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        use crate::application::v13::view::PREFERRED_PAGE_BYTES;
        use unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES;

        let limit = binding.page_limit;
        let mut values = scan
            .hits
            .iter()
            .take(limit)
            .map(|hit| hit.value.clone())
            .collect::<Vec<_>>();
        // A page retains its hits, not every full module projection: each
        // module is parsed once and released before the next one.
        let mut groups = std::collections::BTreeMap::<(String, String), Vec<usize>>::new();
        for (index, hit) in scan.hits.iter().take(limit).enumerate() {
            if let Some(at) = hit.value.get("at").and_then(Value::as_str) {
                groups
                    .entry((hit.source_set.clone(), at.to_owned()))
                    .or_default()
                    .push(index);
            }
        }
        for ((source_set, raw), indices) in groups {
            let Ok(module) = QualifiedAddress::parse(&raw) else {
                continue;
            };
            let Some(source) = sources
                .iter()
                .find(|source| source.source_set_name() == source_set)
            else {
                continue;
            };
            let Ok(authority) = source.logical_view_read_authority(cancellation) else {
                continue;
            };
            let authority = match scan.hits[indices[0]].external_binding.as_ref() {
                Some(proof) => {
                    if let Err(error) = proof.validate(source) {
                        return view_error_result(Some(module.to_string()), error);
                    }
                    authority.with_external_point_binding(proof.binding.clone())
                }
                None => authority,
            };
            for index in indices {
                let hit = &mut values[index];
                let Some(line) = hit
                    .get("line")
                    .and_then(Value::as_u64)
                    .and_then(|line| usize::try_from(line).ok())
                else {
                    continue;
                };
                let body = match authority.narrowest_body_for_lines(&module, line, line) {
                    Ok(body) => body.to_string(),
                    Err(error) => {
                        if matches!(
                            error.code(),
                            RefusalCode::Cancelled
                                | RefusalCode::DeadlineExceeded
                                | RefusalCode::ConcurrentChange
                        ) {
                            return view_error_result(Some(module.to_string()), error);
                        }
                        hit["limits"] = json!(["Method boundaries could not be established; this address opens the module Body line"]);
                        format!("{module}.Body")
                    }
                };
                hit["at"] = json!(format!("{body}#L{line}"));
            }
        }
        let scan_complete = scan.end == TextScanEnd::Complete;
        let state = if scan.uncovered > 0 {
            "partial"
        } else if scan_complete {
            "completed"
        } else {
            "in progress"
        };
        let summary = format!("{mode} BSL search {state}");
        let mut extra_data = Map::new();
        extra_data.insert("dataFreshness".into(), json!("unknown"));
        extra_data.insert("pageConsistency".into(), json!("live"));
        extra_data.insert(
            "fileCoverage".to_owned(),
            json!({
                "complete": scan_complete && scan.uncovered == 0,
                "scanComplete": scan_complete,
                "uncovered": scan.uncovered,
                "uncoveredIsLowerBound": !scan_complete || scan.uncovered_lower_bound,
                "detailsTruncated": scan.uncovered > scan.details.len(),
                "details": scan.details,
            }),
        );
        // The volume of the whole corpus travels with the first page only.
        if let Some(totals) = &scan.totals {
            extra_data.insert(
                "totals".to_owned(),
                totals.to_value(MAX_TEXT_TOTALS_LEADING_MODULES),
            );
        }
        // A cursor also carries the path it re-enters.
        let resume_bytes = scan
            .hits
            .iter()
            .map(|hit| &hit.next.0)
            .chain(scan.time_next.as_ref().map(|next| &next.0))
            .map(|next| match next {
                TextContinuation::Resume(resume) => 2 * (16 + resume.file.len()),
                TextContinuation::Start => 0,
            })
            .max()
            .unwrap_or(0);
        let measured_bytes = |items: &[Value]| {
            search_page_probe_bytes(mode, items, &summary, &extra_data, None) + resume_bytes
        };
        if measured_bytes(&[]) > MAX_CANONICAL_RESULT_BYTES {
            return error_result(
                None,
                RefusalCode::ResultTooLarge,
                "search page metadata exceeds the transport limit",
            );
        }
        let mut page = Vec::new();
        let mut byte_stop = false;
        for item in &values {
            let mut candidate = page.clone();
            candidate.push(item.clone());
            let bytes = measured_bytes(&candidate);
            if bytes > MAX_CANONICAL_RESULT_BYTES {
                if page.is_empty() {
                    return error_result(
                        None,
                        RefusalCode::ResultTooLarge,
                        "one search result exceeds the transport limit",
                    );
                }
                byte_stop = true;
                break;
            }
            if bytes > PREFERRED_PAGE_BYTES && !page.is_empty() {
                byte_stop = true;
                break;
            }
            page.push(item.clone());
        }
        let consumed = page.len();
        let next = if consumed < scan.hits.len() {
            Some(scan.hits[consumed - 1].next.clone())
        } else if scan.end == TextScanEnd::Time {
            scan.time_next.clone()
        } else {
            None
        };
        let stopped_by = match &next {
            None => "complete",
            Some(_) if byte_stop => "bytes",
            Some(_) if consumed < scan.hits.len() => "limit",
            Some(_) => "deadline",
        };
        let mut result = DomainResult::success(summary);
        result.data = Some(search_page_data(mode, &page, &extra_data));
        result.page = Some(json!({"stoppedBy": stopped_by}));
        if let Some((continuation, fingerprint)) = next {
            let next_offset = match SearchCursorStore::next_offset(offset, consumed) {
                Ok(offset) => offset,
                Err(error) => return error.into_result(None),
            };
            let resume = match continuation {
                TextContinuation::Resume(resume) => resume,
                TextContinuation::Start => SearchResume::START,
            };
            binding.result_fingerprint = Some(fingerprint);
            let issued = match stored {
                Some(stored) => {
                    self.search_cursors
                        .insert_next_answer(&stored, &binding, next_offset, &resume)
                }
                None => self
                    .search_cursors
                    .insert_first_resumable(binding, next_offset, &resume),
            };
            match issued {
                Ok(token) => result.cursor = Some(token),
                Err(error) => return error.into_result(None),
            }
        }
        result
    }

    fn search_page_with_data(
        &self,
        matches: Vec<Value>,
        cursor: Option<(&str, StoredSearchCursor)>,
        binding: SearchCursorBinding,
        summary: String,
        extra_data: Map<String, Value>,
    ) -> DomainResult {
        self.search_page_keyed(
            matches,
            None,
            &SearchCursorStore::token_probe(),
            cursor,
            binding,
            summary,
            extra_data,
        )
    }

    /// One search page. With `keys`, the rank of each match, the next
    /// continuation resumes after the last match shown instead of at an
    /// offset into an answer held whole.
    #[allow(clippy::too_many_arguments)]
    fn search_page_keyed(
        &self,
        matches: Vec<Value>,
        keys: Option<&[SearchAfter]>,
        cursor_probe: &str,
        cursor: Option<(&str, StoredSearchCursor)>,
        binding: SearchCursorBinding,
        summary: String,
        extra_data: Map<String, Value>,
    ) -> DomainResult {
        use crate::application::v13::view::PREFERRED_PAGE_BYTES;
        use unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES;

        let mode = binding.mode.as_str();
        let limit = binding.page_limit;
        let offset = cursor.as_ref().map_or(0, |(_, cursor)| cursor.offset);
        let revision: Option<String> = None;
        let mut page_matches = Vec::new();
        let mut byte_stop = false;
        let measured_bytes = |items: &[Value]| {
            search_page_probe_bytes_with(
                mode,
                items,
                &summary,
                &extra_data,
                revision.as_deref(),
                cursor_probe,
            )
        };
        if measured_bytes(&[]) > MAX_CANONICAL_RESULT_BYTES {
            return error_result(
                None,
                RefusalCode::ResultTooLarge,
                "search page metadata exceeds the transport limit",
            );
        }
        for item in matches.iter().take(limit) {
            let mut candidate = page_matches.clone();
            candidate.push(item.clone());
            let bytes = measured_bytes(&candidate);
            if bytes > MAX_CANONICAL_RESULT_BYTES {
                if page_matches.is_empty() {
                    return error_result(
                        None,
                        RefusalCode::ResultTooLarge,
                        "one search result exceeds the transport limit",
                    );
                }
                byte_stop = true;
                break;
            }
            if bytes > PREFERRED_PAGE_BYTES && !page_matches.is_empty() {
                byte_stop = true;
                break;
            }
            page_matches.push(item.clone());
        }
        let consumed = page_matches.len();
        let more = matches.len() > consumed;
        let stopped_by = if !more {
            "complete"
        } else if byte_stop {
            "bytes"
        } else {
            "limit"
        };
        let mut result = DomainResult::success(summary);
        result.data = Some(search_page_data(mode, &page_matches, &extra_data));
        result.page = Some(json!({"stoppedBy": stopped_by}));
        result.rev = revision;
        if more {
            let next_offset = match SearchCursorStore::next_offset(offset, consumed) {
                Ok(offset) => offset,
                Err(error) => return error.into_result(None),
            };
            let after =
                keys.and_then(|keys| consumed.checked_sub(1).and_then(|last| keys.get(last)));
            let issued = match cursor {
                Some((_, stored)) => {
                    self.search_cursors
                        .insert_next_after(&stored, next_offset, after)
                }
                None => self
                    .search_cursors
                    .insert_first_after(binding, next_offset, after),
            };
            let issued = match issued {
                Ok(token) => token,
                Err(error) => return error.into_result(None),
            };
            result.cursor = Some(issued);
        }
        result
    }

    /// Поиск по именам и синонимам метаданных.
    ///
    /// Свод берётся из того же справочника, что обслуживает разрешение
    /// локатора, — он уже держит имена, синонимы и адреса. Наружу отдаётся
    /// доказательство совпадения по имени: `at`, `kind`, `title` и `reason`.
    /// Пути тут нет намеренно: `search` — частый ответ, а путь в частом
    /// ответе зовёт читать файл мимо адреса.
    /// Поиск по тексту силами провайдера выбранной роли.
    ///
    /// `symbol` — символьный индекс, `semantic` — смысловая близость;
    /// буквальный поиск роли не имеет. Роли и провайдеры под ними построены
    /// давно; недоставало провода от канонической службы, потому что до
    /// провайдеров ходил лишь legacy-диспетчер, которого на проводе нет.
    fn execute_search_role(
        &self,
        invocation: &ActorBoundExecution,
        query: &str,
        role: &str,
        arguments: &Map<String, Value>,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        use crate::application::ports::ApplicationPorts;

        let role = match super::v13_search_admission::search_provider_role(arguments, role) {
            Ok(role) => role,
            Err(refusal) => return *refusal,
        };

        let context = invocation.workspace_context();
        // Канонический вход — логический адрес; порт разрешения контекста
        // говорит словарём v0.12. Перевод делается здесь и только здесь.
        let mut selector = Map::new();
        match arguments.get("scope") {
            Some(Value::String(scope)) => match QualifiedAddress::parse(scope) {
                Ok(address) => {
                    selector.insert(
                        "sourceSet".to_string(),
                        Value::String(address.source_set().to_string()),
                    );
                    let owner = if let Some(capability) =
                        unica_bsl::profile::PlatformProfile::v8_3_27().module_capability(&address)
                    {
                        match unica_source_designer::module_layout::module_source_address(
                            &address,
                            capability.owner_kind(),
                            capability.role(),
                            capability.route_kind(),
                        ) {
                            Ok(module) => module.as_str().to_owned(),
                            Err(error) => {
                                return error_result_detailed(
                                    Some(scope.to_string()),
                                    RefusalDetail::ProviderAbsent,
                                    error,
                                );
                            }
                        }
                    } else {
                        address
                            .segments()
                            .iter()
                            .take_while(|segment| segment.name().is_some())
                            .map(|segment| {
                                format!(
                                    "{}.{}",
                                    segment.kind().as_str(),
                                    segment.name().unwrap_or("")
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(".")
                    };
                    if !owner.is_empty() {
                        selector.insert("metadataPath".to_string(), Value::String(owner));
                    }
                }
                Err(error) => {
                    return error_result(
                        Some(scope.to_string()),
                        RefusalCode::BadValue,
                        error.to_string(),
                    )
                }
            },
            Some(_) => {
                return error_result(None, RefusalCode::BadValue, "search scope must be a string")
            }
            None => match invocation.admitted_source_set_names().first() {
                Some(name) => {
                    selector.insert("sourceSet".to_string(), Value::String((*name).to_string()));
                }
                None => {
                    return error_result_detailed(
                        None,
                        RefusalDetail::ProviderAbsent,
                        "no admitted source set is available for a role search",
                    )
                }
            },
        }

        let search_context = match self.ports.resolve_code_search_context(context, &selector) {
            Ok((resolved, _)) => resolved,
            Err(error) => return error_result(None, RefusalCode::ProviderUnavailable, error),
        };
        let registry = match self.ports.code_intelligence_registry() {
            Ok(registry) => registry,
            Err(error) => return error_result(None, RefusalCode::ProviderUnavailable, error),
        };
        #[cfg(test)]
        let registry = self
            .search_providers
            .as_ref()
            .map_or(registry, |providers| {
                CodeIntelligenceRegistry::new(providers.clone())
                    .expect("valid test search providers")
            });
        // Срок берётся из настройки пользователя, а не подменяется умолчанием:
        // человек настроил срок и должен узнать, что настройка не читается.
        let operational = match crate::infrastructure::operational_config::load_operational_config(
            &context.workspace_root,
        ) {
            Ok(config) => config,
            Err(diagnostic) => {
                return error_result(
                    None,
                    RefusalCode::InvalidState,
                    format!("operational config is unreadable: {diagnostic}"),
                )
            }
        };
        let limit = match arguments.get("limit") {
            None => 20,
            Some(value) => match bounded_usize(value).filter(|limit| *limit <= 50) {
                Some(limit) => limit,
                _ => {
                    return error_result(
                        None,
                        RefusalCode::BadValue,
                        "provider search page limit must be a positive integer at most 50",
                    )
                }
            },
        };
        let cursor = match arguments.get("cursor") {
            None => None,
            Some(Value::String(token)) => Some(token.as_str()),
            Some(_) => {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "search cursor must be a string",
                )
            }
        };
        // An explicit provider role has no neighboring provider to fall back
        // to. Acquire only the selected engine after validating the request,
        // before starting the provider.
        let engine = selected_role_engine(role);
        #[cfg(test)]
        let engine = engine.filter(|_| self.search_providers.is_none());
        if let Some(engine) = engine {
            if let Err(refusal) =
                acquire_engine(invocation, &context.cwd, engine, None, cancellation)
            {
                return *refusal;
            }
        }
        let selected_sources = vec![search_context
            .source_root
            .source_set
            .clone()
            .unwrap_or_default()];
        let binding = SearchCursorBinding {
            workspace_identity: invocation.workspace_identity_hash().as_str().to_owned(),
            query: query.to_owned(),
            scope: arguments
                .get("scope")
                .and_then(Value::as_str)
                .map(str::to_owned),
            mode: role.as_str().to_owned(),
            kind: None,
            source_sets: selected_sources.clone(),
            result_fingerprint: None,
            page_limit: limit,
        };
        // Ranked roles answer one finite window that every page re-reads.
        let request = SearchRequest {
            query: query.to_string(),
            limit: PROVIDER_SEARCH_FETCH_LIMIT,
        };
        let execution = match search_selected_role(
            registry,
            role,
            &request,
            &search_context,
            cancellation,
            operational.code_intelligence(),
        ) {
            Ok(execution) => execution,
            Err(error) => {
                if let Some(refusal) = engine.and_then(|engine| {
                    engine_failure_refusal(&context.cwd, engine, None, cancellation)
                }) {
                    return refusal;
                }
                let code = if error.starts_with(unica_work_control::cancellation::CANCELLED_PREFIX)
                {
                    RefusalCode::Cancelled
                } else {
                    RefusalCode::ProviderUnavailable
                };
                return error_result(None, code, error);
            }
        };
        // Поставщик не ответил: если причина — движок, не прошедший проверку
        // поставки (контрольная сумма), отказ называет движок, а не сбой поиска.
        if !execution.ok {
            if let Some(refusal) = engine
                .and_then(|engine| engine_failure_refusal(&context.cwd, engine, None, cancellation))
            {
                return refusal;
            }
        }
        let sources = match invocation.read_sources() {
            Ok(sources) => sources,
            Err(error) => {
                return error_result_detailed(None, RefusalDetail::SourceUnreadable, error)
            }
        };
        let project_hits = |hits: &[unica_code_intelligence::model::ProviderSearchHit]| -> Result<Vec<Map<String, Value>>, ViewError> {
            use unica_source_contract::location::SourceLocation;
            use unica_address::source_target::TargetKind;
            let limited = || Map::from_iter([("limits".into(), json!(["The current source does not prove a Body range containing this provider snippet"]))]);
            let mut projected = vec![Map::new(); hits.len()];
            let mut groups = std::collections::BTreeMap::<String, (usize,QualifiedAddress,Vec<usize>)>::new();
            for (index,hit) in hits.iter().enumerate() {
                let SourceLocation::Addressed { source_set, metadata_path: Some(target), target_kind: TargetKind::Module } = &hit.location else { continue; };
                projected[index] = limited();
                let Some(source_index) = sources.iter().position(|source| source.source_set_name() == source_set) else { continue; };
                let Ok(authority) = sources[source_index].logical_view_read_authority(cancellation) else { continue; };
                let Some(module) = authority.module_address_for_source_target(target) else { continue; };
                groups.entry(module.to_string()).or_insert_with(|| (source_index,module,Vec::new())).2.push(index);
            }
            // Only the current module's semantic projection is retained.
            // Enrichment never changes provider order, ranks or snippets.
            for (_, (source_index,module,indices)) in groups {
                let Ok(authority) = sources[source_index].logical_view_read_authority(cancellation) else { continue; };
                for index in indices {
                    let hit = &hits[index];
                    if hit.line == 0 || hit.snippet.is_empty() { continue; }
                    let Some(to) = hit.end_line.or_else(|| hit.line.checked_add(hit.snippet.lines().count().saturating_sub(1))) else { continue; };
                    match authority.body_for_search_hit(&module, hit.line, to, &hit.snippet) {
                        Ok(Some(body)) => {
                            let at = if to == hit.line { format!("{body}#L{}", hit.line) } else { format!("{body}#L{}-L{to}", hit.line) };
                            projected[index] = Map::from_iter([("at".into(),json!(at))]);
                        }
                        Ok(None) => {}
                        Err(error) if matches!(error.code(), RefusalCode::Cancelled | RefusalCode::DeadlineExceeded | RefusalCode::ConcurrentChange) => return Err(error),
                        Err(_) => {}
                    }
                }
            }
            Ok(projected)
        };
        provider_search_page_projected(
            &self.search_cursors,
            role,
            execution,
            binding,
            cursor,
            cancellation,
            &project_hits,
        )
    }

    fn execute_search_names(
        &self,
        invocation: &ActorBoundExecution,
        query: &str,
        arguments: &Map<String, Value>,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let mut request = match FindRequest::new(query) {
            Ok(request) => request,
            Err(error) => return error_result(None, error.code(), error.to_string()),
        };
        if let Some(kind) = arguments.get("kind") {
            let Some(kind) = kind.as_str() else {
                return error_result(None, RefusalCode::BadValue, "search kind must be a string");
            };
            request = match request.with_kind(kind) {
                Ok(request) => request,
                Err(error) => return error_result(None, error.code(), error.to_string()),
            };
        }
        let limit = match arguments.get("limit") {
            None => 20,
            Some(value) => match bounded_usize(value).filter(|limit| *limit <= 50) {
                Some(limit) => limit,
                None => {
                    return error_result(
                        None,
                        RefusalCode::BadValue,
                        "search limit must be an integer from 1 through 50",
                    )
                }
            },
        };
        request = match request.with_limit(limit) {
            Ok(request) => request,
            Err(error) => return error_result(None, error.code(), error.to_string()),
        };
        let scope = match arguments.get("scope") {
            None => None,
            Some(Value::String(scope)) => match QualifiedAddress::parse(scope) {
                Ok(scope) => Some(scope),
                Err(error) => {
                    return error_result(
                        Some(scope.to_string()),
                        RefusalCode::BadValue,
                        error.to_string(),
                    )
                }
            },
            Some(_) => {
                return error_result(None, RefusalCode::BadValue, "search scope must be a string")
            }
        };
        let sources = match invocation.layout_sources() {
            Ok(sources) => sources,
            Err(error) => return error_result(None, RefusalCode::ProviderUnavailable, error),
        };
        if sources.is_empty() {
            return error_result_detailed(
                None,
                RefusalDetail::ProviderAbsent,
                "name search has no admitted source sets",
            );
        }
        let selected = sources
            .iter()
            .filter(|source| {
                scope
                    .as_ref()
                    .is_none_or(|scope| source.name() == scope.source_set())
            })
            .collect::<Vec<_>>();
        let Some(first) = selected.first() else {
            return error_result(
                scope.as_ref().map(ToString::to_string),
                RefusalCode::NotFound,
                "name search scope does not name an admitted source set",
            );
        };
        let deadline = first.deadline();
        let layout = selected
            .iter()
            .map(|source| LayoutFindSource::new(source.name(), source.kind(), source.root()))
            .collect::<Vec<_>>();
        let scope_binding = scope.as_ref().map(ToString::to_string);
        let binding_for = |result_fingerprint: Option<String>| SearchCursorBinding {
            workspace_identity: invocation.workspace_identity_hash().as_str().to_owned(),
            query: query.to_owned(),
            scope: scope_binding.clone(),
            mode: "names".to_owned(),
            kind: arguments
                .get("kind")
                .and_then(Value::as_str)
                .map(str::to_owned),
            source_sets: selected
                .iter()
                .map(|source| source.name().to_owned())
                .collect(),
            result_fingerprint,
            page_limit: limit,
        };
        // The question half of a continuation is checked before the walk:
        // the rank it resumes after decides which names the page keeps.
        let after = match arguments.get("cursor") {
            None => None,
            Some(Value::String(token)) => {
                match self
                    .search_cursors
                    .question_position(token, &binding_for(None))
                {
                    Ok((_, Some(after))) => match usize::try_from(after.score) {
                        Ok(score) => Some((score, after.at)),
                        Err(_) => return SearchCursorError::Invalid.into_result(None),
                    },
                    // A name continuation always carries its rank.
                    Ok((_, None)) => return SearchCursorError::Invalid.into_result(None),
                    Err(error) => return error.into_result(None),
                }
            }
            Some(_) => {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "search cursor must be a string",
                )
            }
        };
        // Walking the layout ranks the names and notes whether the scope
        // exists; only this page's candidates and one more are kept.
        let ranked_request = match &scope {
            Some(scope) => request.clone().with_scope(scope.clone()),
            None => request.clone(),
        };
        let watched = scope.as_ref().map(name_scope_witnesses).unwrap_or_default();
        let built = match self.find_builder.search_names(
            &layout,
            NameRanker::new(ranked_request, watched, limit.saturating_add(1), after),
            deadline,
            cancellation,
        ) {
            Ok(built) => built,
            Err(error) => return find_build_error_result(None, error),
        };
        if let Some(scope) = scope {
            if let Err((code, message)) =
                validate_name_scope(&|at| built.present.contains(at), &scope, first.kind())
            {
                if built.omissions.total != 0
                    && matches!(code, RefusalCode::NotFound | RefusalCode::InvalidState)
                {
                    return error_result_detailed(
                        Some(scope.to_string()),
                        RefusalDetail::SourceUnreadable,
                        "name search cannot prove the requested scope in an incomplete source layout",
                    );
                }
                return error_result(Some(scope.to_string()), code, message);
            }
        }
        let found = built.names;
        let summary = if built.omissions.total == 0 {
            "name search completed".to_string()
        } else {
            format!(
                "name search has {} unreadable descriptor candidates",
                built.omissions.total
            )
        };
        let coverage = json!({
            "complete": built.omissions.total == 0,
            "omitted": built.omissions.total,
            "detailsTruncated": built.omissions.total > built.omissions.details.len(),
            "details": built.omissions.details.iter().map(|detail| json!({
                "sourceSet": detail.source_set,
                "reason": detail.reason,
            })).collect::<Vec<_>>(),
        });
        let mut extra_data = Map::new();
        extra_data.insert("sourceCoverage".to_owned(), coverage.clone());
        extra_data.insert("approximate".to_owned(), Value::Bool(found.nearest));
        // A name continuation is keyed by the rank of its last item; the
        // probe reserves a key as long as the longest address.
        let cursor_probe = SearchCursorStore::keyed_token_probe(found.largest_address_bytes.max(1));
        let empty_page_bytes =
            search_page_probe_bytes_with("names", &[], &summary, &extra_data, None, &cursor_probe);
        if empty_page_bytes > unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES {
            return error_result(
                None,
                RefusalCode::ResultTooLarge,
                "name search page metadata exceeds the transport limit",
            );
        }
        // An item after the first page must not receive a continuation that
        // could never carry it. The empty probe includes every fixed response
        // field and its cursor; adding one JSON array item adds exactly its
        // serialized byte length, and the walk kept the largest one.
        if found.matched != 0
            && empty_page_bytes.saturating_add(found.largest_item_bytes)
                > unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES
        {
            return error_result(
                None,
                RefusalCode::ResultTooLarge,
                "one name search result exceeds the transport limit",
            );
        }
        // Names do not have a source revision lease. Every page walks the
        // layout again and binds the cursor to the ranked answer — its count
        // and order-free digest — and coverage instead of a source revision.
        let mut hasher = Sha256::new();
        hasher.update(b"unica-v13-names-result-v2\0");
        hasher.update(found.fingerprint);
        let evidence = json!({"approximate": found.nearest, "sourceCoverage": coverage.clone()});
        let bytes = serde_json::to_vec(&evidence).expect("name coverage is serializable");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
        if let Some(refusal) = name_search_interruption(deadline, cancellation) {
            return refusal;
        }
        let binding = binding_for(Some(format!("names-sha256-v2:{:x}", hasher.finalize())));
        let cursor = match arguments.get("cursor").and_then(Value::as_str) {
            None => None,
            Some(token) => match self.search_cursors.read(token, &binding) {
                Ok(cursor) => Some((token, cursor)),
                Err(error) => return error.into_result(None),
            },
        };
        let keys = found
            .candidates
            .iter()
            .map(|(score, candidate)| SearchAfter {
                score: *score as u64,
                at: candidate.at().to_owned(),
            })
            .collect::<Vec<_>>();
        let matches = found
            .candidates
            .iter()
            .map(|(_, candidate)| public_name_match(candidate))
            .collect();
        let result = self.search_page_keyed(
            matches,
            Some(&keys),
            &cursor_probe,
            cursor,
            binding,
            summary,
            extra_data,
        );
        if let Some(refusal) = name_search_interruption(deadline, cancellation) {
            return refusal;
        }
        result
    }

    fn execute_check(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let arguments = invocation.arguments();
        if arguments.contains_key("filter") {
            return error_result(
                None,
                RefusalCode::BadValue,
                "check does not accept a validator filter; validators follow from the node kind",
            );
        }
        if let Some(at) = arguments.get("at") {
            let Some(at) = at.as_str() else {
                return error_result(None, RefusalCode::BadValue, "check at must be a string");
            };
            let limit = match arguments.get("limit") {
                None => DEFAULT_CHECK_LIMIT,
                Some(value) => match bounded_usize(value).filter(|limit| *limit <= MAX_CHECK_LIMIT)
                {
                    Some(limit) => limit,
                    None => {
                        let message = format!(
                            "check limit must be an integer from 1 through {MAX_CHECK_LIMIT}"
                        );
                        return error_result(Some(at.to_string()), RefusalCode::BadValue, message);
                    }
                },
            };
            let cursor = match arguments.get("cursor") {
                None => None,
                Some(Value::String(cursor)) => Some(cursor.as_str()),
                Some(_) => {
                    return error_result(
                        Some(at.to_string()),
                        RefusalCode::BadValue,
                        "check cursor must be a string",
                    )
                }
            };
            if let Some(cursor) = cursor {
                let binding = ViewCursorBinding {
                    canonical_at: at.to_string(),
                    projection: "check".into(),
                    source_set_identity: format!(
                        "{}:{}",
                        invocation.workspace_identity_hash().as_str(),
                        at.split_once(':').map_or("", |(name, _)| name)
                    ),
                    snapshot_id: String::new(),
                    page_limit: limit,
                };
                return crate::application::v13::check::page_diagnostics(
                    &self.cursors,
                    binding,
                    None,
                    Some(cursor),
                );
            }
            let view_arguments =
                Map::from_iter([("at".to_string(), Value::String(at.to_string()))]);
            let (viewed, mut prepared_xdto) = match QualifiedAddress::parse(at) {
                Ok(address) if matches!(address.segments(), [owner] if owner.kind() == NodeKind::XdtoPackage && owner.name().is_some()) => {
                    prepare_xdto_check(invocation, &address, cancellation)
                }
                _ => (
                    self.execute_view_arguments(invocation, &view_arguments, cancellation),
                    None,
                ),
            };
            if !viewed.ok {
                if let Some(refusal) = unreadable_target_format_refusal(invocation, at) {
                    return refusal;
                }
                return viewed;
            }
            let Some(revision) = viewed.rev.as_deref() else {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::InvalidState,
                    "check could not establish its result snapshot",
                );
            };
            let binding = ViewCursorBinding {
                canonical_at: at.to_string(),
                projection: "check".to_string(),
                source_set_identity: format!(
                    "{}:{}",
                    invocation.workspace_identity_hash().as_str(),
                    at.split_once(':').map_or("", |(source_set, _)| source_set)
                ),
                snapshot_id: revision.to_string(),
                page_limit: limit,
            };
            let kind = viewed
                .data
                .as_ref()
                .and_then(|data| data.get("kind"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let mut result = run_node_checks(
                &self.ports,
                invocation,
                at,
                &kind,
                viewed.data.as_ref(),
                cancellation,
                prepared_xdto.take(),
            );
            if result.ok {
                if cancellation.is_cancelled() {
                    return error_result(
                        Some(at.to_string()),
                        RefusalCode::Cancelled,
                        "check was cancelled before publishing its result",
                    );
                }
                result.rev = Some(revision.to_string());
                return crate::application::v13::check::page_diagnostics(
                    &self.cursors,
                    binding,
                    Some(result),
                    None,
                );
            }
            return result;
        }
        // Вердикт по рабочему пространству отвечается до допуска наборов:
        // он и объясняет, почему допуска нет. Сюда управление доходит только
        // при сломанной маршрутизации, и молчать об этом нельзя.
        error_result(
            None,
            RefusalCode::InvalidState,
            "workspace check was routed past the pre-admission root answer",
        )
    }

    fn execute_diff(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let arguments = invocation.arguments();
        if arguments.contains_key("executionToken") {
            let Some(token) = arguments
                .get("executionToken")
                .and_then(Value::as_str)
                .filter(|token| !token.is_empty())
            else {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "diff executionToken must be a nonempty string",
                );
            };
            if arguments
                .keys()
                .any(|key| !matches!(key.as_str(), "executionToken" | "limit" | "cursor"))
            {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "saved-plan diff accepts only executionToken, limit and cursor",
                );
            }
            let limit = match arguments.get("limit") {
                None => SOURCE_NAVIGATION_LIMIT_DEFAULT,
                Some(value) => match bounded_usize(value)
                    .filter(|limit| *limit <= SOURCE_NAVIGATION_LIMIT_MAX)
                {
                    Some(limit) => limit,
                    None => {
                        return error_result(
                            None,
                            RefusalCode::BadValue,
                            "saved-plan diff limit must be an integer from 1 through 50",
                        )
                    }
                },
            };
            let cursor = match arguments.get("cursor") {
                None => None,
                Some(Value::String(cursor)) => Some(cursor.as_str()),
                Some(_) => {
                    return error_result(
                        None,
                        RefusalCode::BadValue,
                        "diff cursor must be a string",
                    )
                }
            };
            return match invocation.read_saved_apply_diff(token, limit, cursor, cancellation) {
                Ok(page) => {
                    let mut result =
                        DomainResult::success("exact saved source ranges read without execution");
                    if let Some(cursor) = page["cursor"].as_str() {
                        result.cursor = Some(cursor.to_owned());
                        result.next.push(json!({"tool": "unica.diff", "arguments": {"executionToken": token, "limit": limit, "cursor": cursor}}));
                    }
                    let mut page = page;
                    let stopped_by = page
                        .as_object_mut()
                        .unwrap()
                        .remove("stoppedBy")
                        .expect("saved source page has a stop reason");
                    result.page = Some(json!({"stoppedBy": stopped_by}));
                    result.data = Some(page);
                    result
                }
                Err(error) => saved_apply_execution_error_result(error),
            };
        }
        if let Some(unknown) = arguments
            .keys()
            .find(|name| !matches!(name.as_str(), "left" | "right" | "limit" | "cursor"))
        {
            return error_result(
                None,
                RefusalCode::BadValue,
                format!("canonical invocation has unknown argument `{unknown}`"),
            );
        }
        let Some(left) = arguments.get("left").and_then(Value::as_str) else {
            return error_result(
                None,
                RefusalCode::BadValue,
                "diff requires string argument `left`",
            );
        };
        let Some(right) = arguments.get("right").and_then(Value::as_str) else {
            return error_result(
                None,
                RefusalCode::BadValue,
                "diff requires string argument `right`",
            );
        };
        // Контракт операции вычисляется из реестра, а не читается из
        // исходников: сравнение его никогда не увидело бы изменения.
        if let Some(computed) = [left, right].into_iter().find(|side| {
            QualifiedAddress::parse(side)
                .ok()
                .is_some_and(|address| address.operation_query().is_some())
        }) {
            return error_result(
                Some(computed.to_string()),
                RefusalCode::TargetKindUnsupported,
                "diff compares source-backed nodes; an operation contract at `.Can.<op>` is computed from the registry, read it with view",
            );
        }
        let limit = match arguments.get("limit") {
            Some(value) => match bounded_usize(value).filter(|limit| *limit <= MAX_DIFF_LIMIT) {
                Some(limit) => limit,
                None => {
                    return error_result(
                        None,
                        RefusalCode::BadValue,
                        format!("diff limit must be an integer from 1 through {MAX_DIFF_LIMIT}"),
                    )
                }
            },
            None => DEFAULT_DIFF_LIMIT,
        };
        let cursor = match arguments.get("cursor") {
            None | Some(Value::Null) => None,
            Some(Value::String(cursor)) if cursor.is_empty() => None,
            Some(Value::String(cursor)) => Some(cursor.clone()),
            Some(_) => {
                return error_result(None, RefusalCode::BadValue, "diff cursor must be a string")
            }
        };
        if let Some(cursor) = cursor {
            let request = match DiffRequest::new(left, right) {
                Ok(request) => request
                    .with_limit(limit)
                    .in_workspace(invocation.workspace_identity_hash().as_str())
                    .with_cursor(cursor),
                Err(error) => return error_result(None, error.code(), error.to_string()),
            };
            return diff_page_result(&request, self.diff.continue_saved(&request));
        }
        let left_result = self.execute_view_arguments(
            invocation,
            &Map::from_iter([("at".to_string(), Value::String(left.to_string()))]),
            cancellation,
        );
        if !left_result.ok {
            return left_result;
        }
        let right_result = self.execute_view_arguments(
            invocation,
            &Map::from_iter([("at".to_string(), Value::String(right.to_string()))]),
            cancellation,
        );
        if !right_result.ok {
            return right_result;
        }
        let request = match DiffRequest::new(left, right) {
            Ok(request) => request
                .with_limit(limit)
                .in_workspace(invocation.workspace_identity_hash().as_str()),
            Err(error) => return error_result(None, error.code(), error.to_string()),
        };
        let side = |at: &QualifiedAddress, result: &DomainResult| {
            let mut data = result.data.clone().expect("successful view has data");
            source_backed(&mut data);
            let kind = data
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            DiffSource::new(at.clone(), kind, data)
        };
        let left = side(request.left(), &left_result);
        let right = side(request.right(), &right_result);
        diff_page_result(&request, self.diff.compare(&request, &left, &right))
    }

    fn execute_run(
        &self,
        invocation: &ActorBoundExecution,
        _cancellation: &CancellationToken,
    ) -> DomainResult {
        let arguments = invocation.arguments();
        let catalog = catalog_for(SurfaceRelease::V13).expect("canonical catalog exists");
        let Some(op) = arguments.get("op") else {
            return super::v13_run_dictionary::run_dictionary_result();
        };
        let Some(op) = op.as_str() else {
            return error_result(None, RefusalCode::BadValue, "run op must be a string");
        };
        if arguments.get("args").is_some_and(|args| !args.is_object()) {
            return error_result(None, RefusalCode::BadValue, "run args must be an object");
        }
        if !catalog
            .run_dictionary
            .iter()
            .any(|operation| operation.name() == op)
        {
            return error_result(
                Some(op.to_string()),
                RefusalCode::UnsupportedOperation,
                format!("unknown canonical run operation `{op}`"),
            );
        }
        error_result(
            Some(op.to_string()),
            RefusalCode::UnsupportedOperation,
            format!("canonical run operation `{op}` is not implemented yet"),
        )
    }

    /// Аварийный двусторонний мост в файловую раскладку.
    ///
    /// Ответ точный или его нет: догадок здесь не делают. Ранжирование живёт
    /// в `search`, и путать эти два вопроса одним именем было главной бедой
    /// снятого `find`.
    fn execute_resolve(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let arguments = invocation.arguments();
        for key in arguments.keys() {
            if !["at", "path", "paths"].contains(&key.as_str()) {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    format!("resolve does not accept argument `{key}`"),
                );
            }
        }
        let at = match arguments.get("at") {
            None => None,
            Some(Value::String(at)) => Some(at.as_str()),
            Some(_) => {
                return error_result(None, RefusalCode::BadValue, "resolve at must be a string")
            }
        };
        let path = match arguments.get("path") {
            None => None,
            Some(Value::String(path)) => Some(path.as_str()),
            Some(_) => {
                return error_result(None, RefusalCode::BadValue, "resolve path must be a string")
            }
        };
        let paths = match arguments.get("paths") {
            None => None,
            Some(Value::Array(items)) => {
                match items.iter().map(Value::as_str).collect::<Option<Vec<_>>>() {
                    Some(items) => Some(items),
                    None => {
                        return error_result(
                            None,
                            RefusalCode::BadValue,
                            "resolve paths must be an array of strings",
                        )
                    }
                }
            }
            Some(_) => {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "resolve paths must be an array of strings",
                )
            }
        };
        let request = match ResolveRequest::new(at, path, paths.as_deref()) {
            Ok(request) => request,
            Err(error) => return error_result(None, error.code(), error.to_string()),
        };
        match request {
            ResolveRequest::Path(path) => {
                match with_resolve_layout(invocation, |layout, deadline| {
                    self.locate_path(invocation, layout, deadline, &path, cancellation)
                }) {
                    Ok(Ok(source)) => resolve_result(source.source),
                    Ok(Err(refusal)) | Err(refusal) => *refusal,
                }
            }
            ResolveRequest::Paths(items) => {
                match with_resolve_layout(invocation, |layout, deadline| {
                    self.resolve_paths(invocation, layout, deadline, &items, cancellation)
                }) {
                    Ok(result) => result,
                    Err(refusal) => *refusal,
                }
            }
            ResolveRequest::Address(address) => {
                self.resolve_address(invocation, &address, cancellation)
            }
        }
    }

    /// Пакет путей и строк: каждый различный путь ищется один раз, каждый
    /// модуль читается одной проекцией на все его строки. Ответ — по
    /// элементу на позицию входа, в том же порядке; отказ элемента не
    /// роняет пакет. Отмена и срок останавливают весь вызов: публикация
    /// чтения раскладки всё равно заменила бы ими ответ.
    fn resolve_paths(
        &self,
        invocation: &ActorBoundExecution,
        layout: &[LayoutFindSource<'_>],
        deadline: ProviderDeadline,
        items: &[Result<PathLine, ResolveError>],
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let mut located = BTreeMap::<String, Result<LocatedSource, Box<DomainResult>>>::new();
        let mut lines = BTreeMap::<String, BTreeSet<usize>>::new();
        for item in items.iter().flatten() {
            let key = item.key();
            if !located.contains_key(&key) {
                let source =
                    self.locate_path(invocation, layout, deadline, item.path(), cancellation);
                if let Err(refusal) = &source {
                    if call_stopping_refusal(refusal) {
                        return *refusal.clone();
                    }
                }
                located.insert(key.clone(), source);
            }
            if let (Some(line), Ok(source)) = (item.line(), &located[&key]) {
                // Строчный источник — файл BSL; у модуля целиком диапазона
                // нет, но строки у файла есть.
                if source.source.path().to_ascii_lowercase().ends_with(".bsl") {
                    lines.entry(key).or_default().insert(line);
                }
            }
        }
        let mut placed =
            BTreeMap::<String, Result<BTreeMap<usize, ModuleLine>, Box<DomainResult>>>::new();
        for (key, requested) in lines {
            let Ok(located_source) = &located[&key] else {
                continue;
            };
            let source = &located_source.source;
            let requested = requested.into_iter().collect::<Vec<_>>();
            let module = QualifiedAddress::parse(source.address())
                .map_err(|error| ViewError::new(RefusalCode::BadValue, error.to_string()))
                .and_then(|module| {
                    let authority = invocation
                        .layout_read_source(module.source_set())
                        .and_then(|read| read.logical_view_read_authority(cancellation))
                        .map_err(|error| {
                            ViewError::detailed(RefusalDetail::SourceUnreadable, error)
                        })?;
                    let authority = match &located_source.external_binding {
                        Some(binding) => authority.with_external_point_binding(binding.clone()),
                        None => authority,
                    };
                    authority.module_lines(&module, &requested)
                });
            let answer = match module {
                Ok(found) => Ok(requested.into_iter().zip(found).collect()),
                Err(error)
                    if matches!(
                        error.code(),
                        RefusalCode::Cancelled | RefusalCode::DeadlineExceeded
                    ) =>
                {
                    return view_error_result(Some(source.address().to_string()), error)
                }
                Err(error) => Err(Box::new(view_error_result(
                    Some(source.address().to_string()),
                    error,
                ))),
            };
            placed.insert(key, answer);
        }
        let mut resolved = 0_usize;
        let answers = items
            .iter()
            .map(|item| {
                let item = match item {
                    Ok(item) => item,
                    Err(error) => {
                        return batch_refusal(&error_result(None, error.code(), error.to_string()))
                    }
                };
                let source = match &located[&item.key()] {
                    Ok(source) => &source.source,
                    Err(refusal) => return batch_refusal(refusal),
                };
                let answer = match (item.line(), placed.get(&item.key())) {
                    (None, _) => ResolvedPath::new(source.address(), source.kind(), None),
                    (Some(_), None) => ResolvedPath::new(
                        source.address(),
                        source.kind(),
                        Some(LinePlace::NotLineBased),
                    ),
                    (Some(_), Some(Err(refusal))) => return batch_refusal(refusal),
                    (Some(line), Some(Ok(module))) => match &module[&line] {
                        ModuleLine::Method(method) => ResolvedPath::new(
                            method.to_string(),
                            NodeKind::Method.as_str(),
                            Some(LinePlace::InMethod),
                        ),
                        ModuleLine::OutsideMethod => ResolvedPath::new(
                            source.address(),
                            source.kind(),
                            Some(LinePlace::OutsideMethod),
                        ),
                        ModuleLine::PastEnd { lines } => {
                            return batch_refusal(&error_result(
                                Some(source.address().to_string()),
                                RefusalCode::NotFound,
                                format!(
                                    "line {line} is past the end of `{}`, which has {lines} lines",
                                    source.path()
                                ),
                            ))
                        }
                    },
                };
                resolved += 1;
                serde_json::to_value(answer).expect("the closed resolve model always serializes")
            })
            .collect::<Vec<_>>();
        let mut result =
            DomainResult::success(format!("{resolved} of {} paths resolved", items.len()));
        result.data = Some(json!({ "items": answers }));
        result
    }

    /// Путь пришёл снаружи: точный предмет раскладки или отказ, который
    /// одиночный `path` отдаёт целиком, а пакет — элементу.
    fn locate_path(
        &self,
        invocation: &ActorBoundExecution,
        layout: &[LayoutFindSource<'_>],
        deadline: ProviderDeadline,
        path: &str,
        cancellation: &CancellationToken,
    ) -> Result<LocatedSource, Box<DomainResult>> {
        let entry = self
            .find_builder
            .locate_path(layout, path, deadline, cancellation)
            .map_err(|error| Box::new(find_build_error_result(None, error)))?;
        let Some(entry) = entry else {
            return Err(Box::new(error_result(
                None,
                RefusalCode::NotFound,
                format!("no admitted source set places `{path}`"),
            )));
        };
        // Кандидаты на присоединённые файлы подтверждаются тем же читателем,
        // что и view: регистрацию владельца и точный ресурс нельзя вывести
        // только из правдоподобной раскладки.
        if matches!(entry.kind(), "Module" | "Metadata")
            || entry
                .placed_path()
                .is_some_and(|path| path.ends_with("/Ext/Form.xml"))
        {
            let address = QualifiedAddress::parse(entry.at()).map_err(|error| {
                Box::new(error_result(None, RefusalCode::BadValue, error.to_string()))
            })?;
            let authority = invocation
                .layout_read_source(address.source_set())
                .and_then(|source| source.logical_view_read_authority(cancellation))
                .map_err(|error| {
                    Box::new(error_result_detailed(
                        Some(entry.at().to_owned()),
                        RefusalDetail::SourceUnreadable,
                        error,
                    ))
                })?;
            let (authority, external_binding) = if let Some(descriptor) = entry.owner_descriptor() {
                let source = invocation
                    .layout_read_source(address.source_set())
                    .map_err(|message| {
                        Box::new(error_result_detailed(
                            Some(entry.at().to_owned()),
                            RefusalDetail::SourceUnreadable,
                            message,
                        ))
                    })?;
                if matches!(
                    source.source_kind(),
                    unica_source_contract::SourceSetKind::ExternalProcessor
                        | unica_source_contract::SourceSetKind::ExternalReport
                ) {
                    let name = address
                        .segments()
                        .first()
                        .and_then(|owner| owner.name())
                        .ok_or_else(|| {
                            Box::new(error_result(
                                Some(entry.at().to_owned()),
                                RefusalCode::InvalidSource,
                                "external target has no owner name",
                            ))
                        })?;
                    let binding =
                        unica_source_designer::external_artifact::ExternalArtifactBinding::new(
                            source.source_kind(),
                            name,
                            std::path::Path::new(descriptor),
                        )
                        .map_err(|message| {
                            Box::new(error_result(
                                Some(entry.at().to_owned()),
                                RefusalCode::InvalidSource,
                                message,
                            ))
                        })?;
                    (
                        authority.with_external_point_binding(binding.clone()),
                        Some(binding),
                    )
                } else {
                    (authority, None)
                }
            } else {
                (authority, None)
            };
            return match Self::resolve_with_authority(&authority, &address, entry.placed_path()) {
                Ok(source) if Some(source.path()) == entry.placed_path() => Ok(LocatedSource {
                    source,
                    external_binding,
                }),
                Ok(_) => Err(Box::new(error_result(
                    Some(entry.at().to_owned()),
                    RefusalCode::InvalidSource,
                    "the logical reader does not place the requested resource at this path",
                ))),
                Err(error) => Err(Box::new(view_error_result(
                    Some(entry.at().to_owned()),
                    error,
                ))),
            };
        }
        Ok(LocatedSource {
            source: ResolvedSource::new(
                entry.at(),
                entry.kind(),
                entry.placed_path().expect("located path has a place"),
                ResolvedLines::NotLineBased,
            ),
            external_binding: None,
        })
    }

    fn resolve_address(
        &self,
        invocation: &ActorBoundExecution,
        address: &QualifiedAddress,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let form_owner = address.form_metadata_owner().or_else(|| {
            matches!(address.segments().last(), Some(last) if last.name().is_some() && matches!(last.kind(), NodeKind::Form | NodeKind::CommonForm))
                .then(|| address.clone())
        });
        if let Some(owner) = form_owner {
            // Preserve resolve's descriptor/owner identity proof before reading
            // content. A missing required owner is invalid source; a successful
            // descriptor proof does not substitute that file for the form body.
            if let Some(mut proof) = self.resolve_metadata_point(invocation, &owner, cancellation) {
                if !proof.ok {
                    proof.at = Some(address.to_string());
                    return proof;
                }
            }
        }
        if matches!(address.segments().last(), Some(last) if last.name().is_some() && matches!(last.kind(), NodeKind::Form | NodeKind::CommonForm))
            || address
                .segments()
                .iter()
                .any(|segment| !segment.kind().is_metadata_kind())
        {
            return match self.resolve_projected_source(invocation, address, cancellation) {
                Ok(source) => resolve_result(source),
                Err(error) => view_error_result(Some(address.to_string()), error),
            };
        }
        let at = address.to_string();
        if let Some(result) = self.resolve_metadata_point(invocation, address, cancellation) {
            return result;
        }
        let directory = match self.layout_directory(
            invocation,
            Some(at.clone()),
            address.source_set(),
            cancellation,
        ) {
            Ok(directory) => directory,
            Err(refusal) => return *refusal,
        };
        let Some(entry) = directory.locate_address(&at) else {
            return error_result(
                Some(at.clone()),
                RefusalCode::NotFound,
                format!("the source layout does not place `{at}`"),
            );
        };
        let Some(placed) = entry.placed_path() else {
            return error_result(
                Some(at.clone()),
                RefusalCode::NotFound,
                format!("the source layout does not place `{at}`"),
            );
        };
        let kind = entry.kind().to_string();
        let placed = placed.to_string();
        resolve_result(ResolvedSource::new(
            at,
            kind,
            placed,
            ResolvedLines::NotLineBased,
        ))
    }

    /// Resolves one metadata object by its own layout place: the target file
    /// and its necessary owners, the same proof as `resolve(path)`. It never
    /// builds the full name directory, so foreign descriptors are not read and
    /// their damage does not decide this answer (#1457).
    fn resolve_metadata_point(
        &self,
        invocation: &ActorBoundExecution,
        address: &QualifiedAddress,
        cancellation: &CancellationToken,
    ) -> Option<DomainResult> {
        let at = address.to_string();
        let source = match admitted_read_source(
            invocation.read_sources(),
            |source| source.source_set_name(),
            Some(&at),
            address.source_set(),
        ) {
            Ok(source) => source,
            Err(refusal) => return Some(*refusal),
        };
        let relative = metadata_layout_path(address, source.source_kind())?;
        let relative = relative.as_str();
        let root = source.retained_root();
        let layout = [LayoutFindSource::new(
            source.source_set_name(),
            source.source_kind(),
            root.as_ref(),
        )];
        let entry =
            match self
                .find_builder
                .locate_path(&layout, relative, source.deadline(), cancellation)
            {
                Ok(entry) => entry,
                Err(error) => return Some(find_build_error_result(Some(at), error)),
            };
        let external = matches!(
            source.source_kind(),
            unica_source_contract::SourceSetKind::ExternalProcessor
                | unica_source_contract::SourceSetKind::ExternalReport
        );
        if external
            && entry.as_ref().is_none_or(|entry| {
                entry.at() != at
                    && QualifiedAddress::parse(entry.at())
                        .ok()
                        .and_then(|located| located.form_metadata_owner())
                        .as_ref()
                        != Some(address)
            })
        {
            return None;
        }
        let Some((kind, placed)) = entry.as_ref().and_then(|entry| {
            (entry.at() == at
                || QualifiedAddress::parse(entry.at())
                    .ok()
                    .and_then(|located| located.form_metadata_owner())
                    .as_ref()
                    == Some(address))
            .then(|| entry.placed_path().map(|placed| (entry.kind(), placed)))
            .flatten()
        }) else {
            return Some(error_result(
                Some(at.clone()),
                RefusalCode::NotFound,
                format!("the source layout does not place `{at}`"),
            ));
        };
        Some(resolve_result(ResolvedSource::new(
            at.clone(),
            kind.to_string(),
            placed.to_string(),
            ResolvedLines::NotLineBased,
        )))
    }

    fn resolve_projected_source(
        &self,
        invocation: &ActorBoundExecution,
        address: &QualifiedAddress,
        cancellation: &CancellationToken,
    ) -> Result<ResolvedSource, ViewError> {
        let source = invocation
            .read_sources()
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?
            .into_iter()
            .find(|source| source.source_set_name() == address.source_set())
            .ok_or_else(|| {
                ViewError::new(RefusalCode::NotFound, "resolve source set was not admitted")
            })?;
        let authority = source
            .logical_view_read_authority(cancellation)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        Self::resolve_with_authority(&authority, address, None)
    }

    fn resolve_with_authority(
        authority: &crate::infrastructure::v13_read::LogicalViewReadAuthority<'_>,
        address: &QualifiedAddress,
        requested_path: Option<&str>,
    ) -> Result<ResolvedSource, ViewError> {
        let snapshot = authority.snapshot(address)?;
        let canonical = authority.canonical_address(address, &snapshot)?;
        let (kind, lines) = authority.source_lines(&canonical, &snapshot)?;
        let path = authority.existing_export_path(&canonical, requested_path)?;
        if authority.snapshot(&canonical)?.revision != snapshot.revision {
            return Err(ViewError::new(
                RefusalCode::StaleCursor,
                "source revision changed during source location resolution",
            ));
        }
        authority.read_checkpoint()?;
        Ok(ResolvedSource::new(
            canonical.to_string(),
            kind,
            path,
            lines,
        ))
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
    let target = unica_address::source_target::MetadataAddress::parse(
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        target,
    )
    .ok()?;
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
        ProviderRole::Semantic => Some("rlm-bsl-mcp"),
    }
}

fn search_selected_role(
    registry: CodeIntelligenceRegistry,
    role: ProviderRole,
    request: &SearchRequest,
    context: &CodeIntelligenceContext,
    cancellation: &CancellationToken,
    deadlines: unica_code_intelligence::deadlines::CodeIntelligenceDeadlines,
) -> Result<unica_code_intelligence::coordinator::CodeSearchExecution, String> {
    use unica_code_intelligence::coordinator::CodeSearchCoordinator;

    let provider = registry
        .search_providers()
        .find(|provider| provider.identity().role == role)
        .cloned()
        .ok_or_else(|| format!("no search provider is registered for `{}`", role.as_str()))?;
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
    "borrowing",
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

fn apply_plan_error_result(error: &ApplyPlanError) -> DomainResult {
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

impl CanonicalV13ReadService {
    /// Ответ порта символов в модели Unica: адреса и диапазоны, без имён
    /// поставщика. Буфер, отличный от сохранённого модуля, отвечает своим
    /// текстом, а то, что за его пределами знает только поставщик, помечено
    /// `stale`. Маршрут LSP (#2173) зовёт это в акторе пространства.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn symbols(
        &self,
        invocation: &ActorBoundExecution,
        request: SymbolRequest<'_>,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        use unica_code_intelligence::symbols::{
            CallDirection, SymbolDocument, SymbolRequestContext, SymbolService,
        };
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
        // The module's canonical address and path, so the places the
        // provider names in this module compare equal to the document.
        let (at, path, saved) =
            match Self::resolve_with_authority(&authority, &request.module, None).and_then(
                |resolved| {
                    authority
                        .module_text(&request.module)
                        .map(|text| (resolved.at().to_string(), resolved.path().to_string(), text))
                },
            ) {
                Ok(module) => module,
                Err(error) => return view_error_result(Some(at), error),
            };
        let environment = crate::infrastructure::symbol_environment::ReaderSymbolEnvironment {
            authority: &authority,
            source_set: &source_set,
        };
        let document = SymbolDocument {
            at: &at,
            path: &path,
            text: request.buffer.unwrap_or(&saved),
            saved: &saved,
        };
        let provider = self.symbol_provider(invocation, &source_set);
        let service =
            SymbolService::new(provider.as_deref(), &environment).with_names(&environment);
        // The provider gets the invocation's deadline, as MCP calls do: a
        // shorter editor budget would retire the shared analyzer session on
        // every slow answer until #2189 lets a deadline cancel without that.
        let context = SymbolRequestContext {
            deadline: source.deadline(),
            cancellation,
        };
        let position = request.position;
        let data = match request.operation {
            SymbolOperation::Definition => {
                serde_json::to_value(service.definition(&document, position, &context))
            }
            SymbolOperation::Hover => {
                serde_json::to_value(service.hover(&document, position, &context))
            }
            SymbolOperation::References => {
                serde_json::to_value(service.references(&document, position, &context))
            }
            SymbolOperation::IncomingCalls => serde_json::to_value(service.calls(
                &document,
                position,
                CallDirection::Incoming,
                &context,
            )),
            SymbolOperation::OutgoingCalls => serde_json::to_value(service.calls(
                &document,
                position,
                CallDirection::Outgoing,
                &context,
            )),
        }
        .expect("symbol answers serialize");
        let mut result = DomainResult::success("symbols answered");
        result.at = Some(at);
        result.data = Some(data);
        result
    }

    /// Поставщик символов набора исходников, если он есть. Его нет — ответ
    /// собирается из текста модуля и называет причину неполноты.
    fn symbol_provider(
        &self,
        invocation: &ActorBoundExecution,
        source_set: &str,
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
            .resolve_code_search_context(invocation.workspace_context(), &args)
            .ok()?;
        Some(Box::new(
            crate::infrastructure::analysis_composition::bsl_symbols(context),
        ))
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
    use unica_bsl_parser::MISPLACED_VARIABLE_MESSAGE;
    use unica_diagnostics::model::DiagnosticFocus;
    use unica_diagnostics::model::DiagnosticItem;
    use unica_diagnostics::model::DiagnosticSeverity;
    use unica_diagnostics::source_ranges::misplaced_variable_declarations;
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
    if unica_metadata::MetaServiceMemberContext::owns_diagnostic(diagnostic) {
        return true;
    }
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
    let target = unica_address::source_target::MetadataAddress::parse(
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        &path,
    )
    .map_err(|error| {
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

/// Предварительная проверка `apply`: каждая операция есть в реестре, набор
/// исходников её семейство пишет, а сама она исполнима на своём узле.
///
/// Сначала читается таблица снятых аргументов: вызов, который называет
/// подузел аргументом, получает `bad_value` с адресом подузла в `next` — а не
/// вердикт «не тот узел» с тупиком к `can` того же узла. Затем проверяется
/// применимость к месту; `next` неприменимой операции ведёт к `can`
/// ближайшего узла, где она исполнима. Если такого узла нет, `next` ведёт к
/// `can` того же узла, когда тот читается, иначе к корню набора, а текст
/// называет форму нужного адреса. `template_format` читает формат макета-
/// предка: `next` не уводит на макет другого формата.
pub(super) fn preflight_apply_operations(
    request: &crate::domain::apply::ApplyRequest,
    source: unica_source_contract::context::SourceSetTraits,
    template_format: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
) -> Result<(), Box<DomainResult>> {
    preflight_apply_operations_with(
        request,
        source,
        template_format,
        &|_| None,
        &unica_operations::recovery::no_member_lookup,
    )
}

/// То же, что `preflight_apply_operations`, с поиском узла по имени: снятый
/// `dataSet` ведёт к настоящему, в том числе вложенному, набору, а имя
/// элемента формы — к его пути в форме.
pub(super) fn preflight_apply_operations_with(
    request: &crate::domain::apply::ApplyRequest,
    source: unica_source_contract::context::SourceSetTraits,
    template_format: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
    form_item: &dyn Fn(&QualifiedAddress) -> Option<unica_forms::FormItemRole>,
    members: unica_operations::recovery::MemberPathLookup<'_>,
) -> Result<(), Box<DomainResult>> {
    let registry = crate::domain::apply::operation_registry();
    for (index, operation) in request.ops().iter().enumerate() {
        let Some(descriptor) = registry.lookup(operation.name()) else {
            // Снятая операция ведёт к заменившей на узле-родителе (#1354).
            if let Some(retired) = crate::domain::apply::retired_operation(operation.name()) {
                return Err(Box::new(retired_operation_refusal(
                    index,
                    operation.at(),
                    retired,
                    source,
                    template_format,
                    form_item,
                )));
            }
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
        if let Some(mut redirect) = registry.retired_argument_redirect_with(
            operation.name(),
            &site,
            operation.args(),
            members,
        ) {
            // Значение назвало элемент, роль которого операцию не принимает
            // (`into` поля): `next` ведёт к ближайшему узлу, где она исполнима.
            if !redirect.listing && !admits_form_item_at(descriptor, &redirect.at, form_item) {
                if let Some(target) = crate::domain::apply::operation_site(&redirect.at, source)
                    .and_then(|target| {
                        descriptor.nearest_applicable_ancestor_with(
                            &target,
                            registry.context(),
                            template_format,
                            form_item,
                        )
                    })
                {
                    redirect.at = target;
                }
            }
            let location = Some(format!("ops[{index}].args.{}", redirect.location));
            if redirect.ambiguous {
                // Имя повторяется (члены разных объединений, одноимённые
                // группы в разных ветвях структуры): адрес за вызывающего
                // не выбирается. `next` — ветвь, где видны все, и словари
                // совпавших узлов (#1357).
                let mut refusal = error_result(
                    location,
                    RefusalCode::BadValue,
                    format!(
                        "`{}` names its subject by address, not by `{}`; the name does not select one addressable node{}; read `{}` and call it at the one you mean",
                        operation.name(),
                        redirect.location,
                        if redirect.candidates.is_empty() {
                            String::new()
                        } else {
                            format!(
                                ": {}",
                                redirect
                                    .candidates
                                    .iter()
                                    .map(|candidate| format!("`{candidate}`"))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        },
                        redirect.at
                    ),
                );
                refusal.next.push(json!({
                    "tool": "unica.view",
                    "args": {"at": redirect.at.to_string()},
                    "reason": "одноимённые узлы этого вида: выберите адрес",
                }));
                for candidate in &redirect.candidates {
                    refusal =
                        with_operation_dictionary(refusal, &candidate.to_string(), descriptor);
                }
                return Err(Box::new(refusal));
            }
            if redirect.listing {
                // Значение называло подузел не именем: адрес из него не
                // строится, и `next` ведёт к коллекции, где имя видно.
                let mut refusal = error_result(
                    location,
                    RefusalCode::BadValue,
                    format!(
                        "`{}` names its subject by address, not by `{}`; call it at `{}.<Name>`, reading the name in `{}`",
                        operation.name(),
                        redirect.location,
                        redirect.at,
                        redirect.at
                    ),
                );
                refusal.next.push(json!({
                    "tool": "unica.view",
                    "args": {"at": redirect.at.to_string()},
                    "reason": format!(
                        "имена узлов для операции `{}`: элемент коллекции несёт свой `id` в `props`",
                        operation.name()
                    ),
                }));
                return Err(Box::new(refusal));
            }
            // Адрес построен из значения аргумента, и узла с таким именем
            // может не быть: вторым шагом — коллекция, где видны имена.
            // Безымянный узел (`DataSet.D.Query`) — часть своего именованного
            // родителя: ветвь строится по родителю.
            let named = redirect
                .at
                .segments()
                .iter()
                .rposition(|segment| segment.name().is_some())
                .map_or(redirect.at.segments(), |last| {
                    &redirect.at.segments()[..=last]
                });
            let (member, owner) = named
                .split_last()
                .expect("a member address has a terminal segment");
            let branch = QualifiedAddress {
                line_range: None,
                source_set: redirect.at.source_set().to_string(),
                segments: owner.to_vec(),
            };
            let branch = format!("{branch}.{}", member.kind().as_str());
            // Аргумент, повторявший сам адрес вызова, просто лишний.
            let mut message = if redirect.at == *operation.at() {
                format!(
                    "`{}` takes its subject from `at`; remove `{}` and call it at `{}`",
                    operation.name(),
                    redirect.location,
                    redirect.at
                )
            } else {
                format!(
                    "`{}` names its subject by address, not by `{}`; call it at `{}`",
                    operation.name(),
                    redirect.location,
                    redirect.at
                )
            };
            // `*.set` пишет только в существующий узел: отсутствующий
            // создаёт операция родителя (#1354).
            if let Some(creator) = descriptor.creator() {
                message.push_str(&format!(
                    "; a missing one is created by `{creator}` on its parent"
                ));
            }
            let refusal = error_result(location, RefusalCode::BadValue, message);
            let mut refusal =
                with_operation_dictionary(refusal, &redirect.at.to_string(), descriptor);
            refusal.next.push(json!({
                "tool": "unica.view",
                "args": {"at": branch},
                "reason": "существующие узлы этого вида у владельца",
            }));
            // Узла с этим именем может не быть: третьим шагом — контракт
            // операции родителя, которая его создаёт (#1354).
            if let Some(creator) = descriptor.creator() {
                let parent = QualifiedAddress {
                    line_range: None,
                    source_set: redirect.at.source_set().to_string(),
                    segments: owner.to_vec(),
                };
                refusal.next.push(json!({
                    "tool": "unica.view",
                    "args": {"at": format!("{parent}.Can.{creator}")},
                    "reason": format!("отсутствующий узел создаёт `{creator}` на родителе"),
                }));
            }
            return Err(Box::new(refusal));
        }
        // Роль элемента формы — фильтр поверх места: в поле элемент не
        // добавить, автоматическую панель отдельно не удалить.
        if descriptor.applies_at(&site)
            && admits_form_item_at(descriptor, operation.at(), form_item)
        {
            continue;
        }
        // Прежний псевдоним запроса `Template.T.Query.D` снят (#1353): отказ
        // называет адрес набора, где операция исполнима.
        if let Some(target) =
            descriptor.retired_query_alias_target(&site, registry.context(), members)
        {
            return Err(Box::new(with_operation_dictionary(
                error_result(
                    Some(format!("ops[{index}].at")),
                    RefusalCode::BadValue,
                    format!(
                        "a query is addressed through its dataset: `{}` applies at `{target}`",
                        operation.name()
                    ),
                ),
                &target.to_string(),
                descriptor,
            )));
        }
        let refused = format!(
            "apply operation `{}` does not apply to {} `{}`",
            operation.name(),
            target_kind.as_str(),
            operation.at()
        );
        let nearest = registry.nearest_applicable_ancestor_with(
            operation.name(),
            &site,
            template_format,
            form_item,
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

/// Отказ снятой операции без записи (#1354, #1465). Создающая операция
/// ведёт к словарю родителя; переименованная — к контракту заменившей на
/// ближайшем узле, где она исполнима, как отказ «не тот узел». Если такого
/// узла в цепочке адреса нет, отказ называет форму адреса заменившей.
fn retired_operation_refusal(
    index: usize,
    at: &QualifiedAddress,
    retired: unica_operations::recovery::RetiredOperation,
    source: unica_source_contract::context::SourceSetTraits,
    template_format: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
    form_item: &dyn Fn(&QualifiedAddress) -> Option<unica_forms::FormItemRole>,
) -> DomainResult {
    let location = Some(format!("ops[{index}].op"));
    let site = crate::domain::apply::operation_site(at, source);
    let target = site.as_ref().and_then(|site| {
        retired.replacement_target(
            site,
            crate::domain::apply::operation_registry().context(),
            template_format,
            form_item,
        )
    });
    let replacement = retired.descriptor();
    match (target, replacement) {
        (Some(target), Some(replacement)) => {
            let refusal = error_result(
                location,
                RefusalCode::BadValue,
                format!(
                    "apply operation `{}` is retired: call `{}` at `{target}`; {}",
                    retired.op,
                    retired.replacement(),
                    retired.arguments
                ),
            );
            if retired.creates.is_some() {
                with_node_dictionary(refusal, &target.to_string())
            } else {
                with_operation_dictionary(refusal, &target.to_string(), replacement)
            }
        }
        (_, replacement) => {
            let shape = replacement.map_or_else(String::new, |replacement| {
                format!(", which addresses {}", replacement.address_shape())
            });
            let refusal = error_result(
                location,
                RefusalCode::BadValue,
                format!(
                    "apply operation `{}` is retired: call `{}`{shape}; {}",
                    retired.op,
                    retired.replacement(),
                    retired.arguments
                ),
            );
            let readable = crate::infrastructure::logical_tree::route_logical_address(
                at,
                crate::domain::apply::source_classification_profile(source),
            )
            .is_ok();
            if readable {
                with_node_dictionary(refusal, &at.to_string())
            } else {
                with_node_dictionary(refusal, &format!("{}:Configuration", at.source_set()))
            }
        }
    }
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
mod plan_detail_key_tests {
    use super::{plan_detail_key_errors, PLAN_DETAIL_KEYS};
    use unica_source_contract::proposal::PlanDetailKey;

    #[test]
    fn registered_plan_detail_sections_are_unique_and_leave_answer_fields_alone() {
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
mod tests {
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
            super::apply_publication_error_code(
                super::ApplyPublicationErrorKind::ContainmentIdentity
            ),
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
        let identity = ProviderIdentity::new(ProviderRole::Semantic, "rlm");
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
        let result =
            |complete: bool, reason: Option<DiagnosticSuppressionReason>| DiagnosticResult {
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
        std::fs::write(workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n").unwrap();
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
                let result =
                    invoke_exact_read(workspace.path(), ToolIdentity::View, json!({"at":at}));
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
            super::validate_name_scope(&present("none"), &scope, SourceSetKind::Configuration)
                .is_ok()
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
            let result =
                invoke_exact_read(workspace.path(), ToolIdentity::Resolve, json!({"at":at}));
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
                .is_some_and(|message| message.contains("`Catalogs/Items.xml`")
                    && message.contains("missing")),
            "the refusal names the missing owner descriptor: {:?}",
            orphan.diagnostics[0]["message"]
        );
    }

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
            let result =
                invoke_exact_read(workspace.path(), ToolIdentity::Resolve, json!({"at":at}));
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
                let viewed =
                    invoke_exact_read(workspace.path(), ToolIdentity::View, json!({"at":at}));
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
            let one_more =
                invoke_exact_read(workspace.path(), ToolIdentity::View, json!({"at":next}));
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
                let view =
                    invoke_exact_read(workspace.path(), ToolIdentity::View, json!({"at":at}));
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
            let runtime =
                V5CanonicalInvocationRuntime::new(Arc::new(service), Arc::new(TokioClock));
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
                .filter(|detail| {
                    matches!(*detail, "buildingIndex" | "updatingIndex" | "indexQueued")
                });
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
            .join("../..")
            .join("plugins/unica/third-party/tools.lock.json");
        let lock: serde_json::Value =
            serde_json::from_slice(&std::fs::read(lock_path).unwrap()).unwrap();
        for (role, tool, artifact) in [
            (ProviderRole::Symbol, "bsl-analyzer", "bsl-analyzer"),
            (ProviderRole::Semantic, "rlm-bsl-mcp", "rlm-tools-bsl"),
        ] {
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

    #[test]
    fn public_role_search_surfaces_partial_provider_warning_and_hit() {
        use unica_code_intelligence::coordinator::CodeSearchExecution;
        use unica_code_intelligence::model::CodeSearchResult;
        use unica_code_intelligence::model::ProviderSearchHit;
        use unica_code_intelligence::model::SearchCoverage;
        use unica_source_contract::location::SourceLocation;

        let section = ProviderSearchSection::partial(
            ProviderIdentity::new(ProviderRole::Semantic, "rlm"),
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
            vec!["ignored malformed RLM result #1".to_string()],
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
                warnings: vec!["rlm: ignored malformed RLM result #1".to_string()],
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
        super::attach_can(
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
            let mut arguments =
                json!({"query":"Needle","scope":"main:CommonModule.Probe","limit":50});
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
                format!(
                    "[operational.code_intelligence]\nsearch_git_grep_timeout_seconds = {value}\n"
                ),
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
            let pages =
                text_search_pages(workspace.path(), engine, json!({"query":"Mark","limit":2}));
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
}

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
