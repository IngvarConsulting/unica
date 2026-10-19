use super::server::{ActorBoundExecution, ActorBoundInvocation, CanonicalInvocationService};
/// Потолок страницы ветви графа: объявленный максимум поверхности.
const CALL_GRAPH_PAGE_LIMIT: usize = 50;
/// Provider APIs accept a top-N request, but no offset. Keep their existing
/// public bound as the fetch window and page the returned evidence separately.
const PROVIDER_SEARCH_FETCH_LIMIT: usize = 200;

use super::v13_call_graph::{
    branch_collection, branch_direction, branch_owner, call_graph_support, complete_branch,
    declare_method_branches, fetch_summary, module_file_for_placed, placed_for_module_file,
    unready_branch_result, CallGraphFetchError, CallGraphSummary, CompleteBranchError,
};
use super::v13_read_modes::{attach_can, operation_node_with, search_scope_prefix, source_backed};
use crate::application::invocation_store::ToolIdentity;
use crate::application::operation_descriptors::ExecutionClass;
use crate::application::result_store::{
    SearchCursorBinding, SearchCursorStore, StoredSearchCursor, ViewCursorBinding, ViewCursorStore,
};
use crate::application::tool_contracts::SurfaceRelease;
use crate::application::v13::apply::parse_request as parse_apply_request;
use crate::application::v13::find::{FindIndex, FindRequest};
use crate::application::v13::resolve::{ResolveRequest, ResolvedLines, ResolvedSource};
use crate::application::v13::tool_catalog::catalog_for;
use crate::application::v13::view::{ViewError, ViewReadAuthority, ViewRequest, ViewService};
use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::apply::{
    OperationRegistry, OperationSite, SourceSetTraits, TemplateFormat, UNWRITABLE_SOURCE_MESSAGE,
};
use crate::domain::call_graph_identity::{CallGraphIdentity, CallGraphIdentityError};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::{
    CallGraphState, CodeIntelligenceContext, CodeIntelligenceRegistry, ProviderDeadline,
    ProviderRole, SearchRequest,
};
use crate::domain::invocation::{DomainResult, InvocationFailure};
use crate::domain::project_sources::SourceSetKind;
use crate::domain::refusal::{RefusalCode, RefusalDetail};
use crate::infrastructure::capacity_observation::CapacityObserver;
use crate::infrastructure::metadata_kinds::metadata_kind;
use crate::infrastructure::native_operations::apply::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyStagedState, ApplyStagingErrorKind,
    PlannedApplyEffects, StagedChangeKind, StagedFileState,
};
use crate::infrastructure::native_operations::apply_families::plan_hidden_v13_apply;
use crate::infrastructure::v13_find::{
    FindBuildError, LayoutFindSource, WorkspaceFindDirectoryBuilder,
};
use crate::infrastructure::workspace_actor::{
    ApplyAdmissionError, ApplyEffectDisposition, ApplyPublicationErrorKind,
    SavedApplyExecutionError,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;

/// Canonical v0.13 service installed by the production v3 daemon composition.
/// Each public name has a useful closed mode; unfinished variants fail with a
/// typed `unsupported_*` result rather than pretending an engine is missing.
pub(crate) struct CanonicalV13ReadService {
    cursors: Arc<ViewCursorStore>,
    search_cursors: Arc<SearchCursorStore>,
    find_builder: WorkspaceFindDirectoryBuilder,
    /// Порты приложения живут столько же, сколько служба, а не сколько вызов.
    /// Внутри них стол доставок: он принадлежит серверу и переживает вызов,
    /// который доставку начал, чтобы её получил следующий. Порты на каждый
    /// вызов означали бы стол на каждый вызов — и доставки перестали бы
    /// делиться.
    ports: Arc<crate::infrastructure::application_ports::InfrastructureApplicationPorts>,
    #[cfg(test)]
    search_providers:
        Option<Vec<Arc<dyn crate::domain::code_intelligence::CodeIntelligenceProvider>>>,
}

impl Default for CanonicalV13ReadService {
    fn default() -> Self {
        Self {
            cursors: Arc::new(ViewCursorStore::default()),
            search_cursors: Arc::new(SearchCursorStore::default()),
            find_builder: WorkspaceFindDirectoryBuilder::default(),
            ports: Arc::new(
                crate::infrastructure::application_ports::InfrastructureApplicationPorts::new(),
            ),
            #[cfg(test)]
            search_providers: None,
        }
    }
}

impl CanonicalInvocationService for CanonicalV13ReadService {
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
            cursors: Arc::new(ViewCursorStore::new(max_total_bytes)),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(super) fn retained_view_usage_for_test(
        &self,
    ) -> crate::application::result_store::ViewRetainedUsage {
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
        service.cursors =
            Arc::new(ViewCursorStore::default().with_capacity_observer(observer.clone()));
        service.find_builder = service.find_builder.with_capacity_observer(observer);
        service
    }

    #[cfg(test)]
    pub(super) fn with_search_providers_for_test(
        providers: Vec<Arc<dyn crate::domain::code_intelligence::CodeIntelligenceProvider>>,
    ) -> Self {
        Self {
            search_providers: Some(providers),
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
            return invocation.execute_saved_apply(token.as_str().unwrap(), cancellation, |mut result, publication| {
                match publication {
                    Ok(publication) => {
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
        let traits = SourceSetTraits::of_source(
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
        admission.bind_request(&request);
        let operations = request
            .ops()
            .iter()
            .enumerate()
            .map(|(index, operation)| {
                serde_json::json!({
                    "index": index,
                    "op": operation.name(),
                    "at": operation.at(),
                })
            })
            .collect::<Vec<_>>();
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
        let has_changes = !staged.planned_changes().is_empty();
        if !has_changes {
            effects = Default::default();
        }
        let plan_warnings = effects.warnings().to_vec();
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
            "operations": operations,
            "planHash": plan_hash,
            "effects": publication.effects().events().len(),
            "cache": publication.effects().cache(),
        }));
        if !borrowing.is_empty() {
            result.data.as_mut().expect("apply data exists")["borrowing"] =
                serde_json::json!(borrowing);
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
        let read_at = match QualifiedAddress::parse(at) {
            Ok(address) => match address.operation_query() {
                Some((owner, Some(op))) => {
                    if let Some(paging) = ["cursor", "limit"]
                        .into_iter()
                        .find(|name| arguments.contains_key(*name))
                    {
                        return error_result(
                            Some(at.to_string()),
                            RefusalCode::BadValue,
                            format!(
                                "an operation contract is one node and has no pages; omit {paging}"
                            ),
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
        let can_context = SourceSetTraits::read(source.source_kind(), authority.platform_profile());
        let service = ViewService::with_shared_cursors(authority, Arc::clone(&self.cursors));
        let enrich_and_project = |data: &Value| -> Result<Value, ViewError> {
            let mut data = data.clone();
            if source.source_kind() == crate::domain::project_sources::SourceSetKind::Extension
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
                            .map(crate::domain::apply::FormItemRole::from_tag)
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
        let mut result = service.view_projected(request, &enrich_and_project);
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
                                == crate::domain::project_sources::SourceSetKind::Configuration
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
        let (matcher, mode) = match arguments.get("regex") {
            None | Some(Value::Bool(false)) => (
                super::v13_read_modes::SearchMatcher::Literal(query.to_string()),
                "literal",
            ),
            Some(Value::Bool(true)) => {
                // Bounded compile: a pattern that cannot be compiled within
                // the size budget is a caller error, not a provider failure.
                match regex::RegexBuilder::new(query)
                    .size_limit(1 << 20)
                    .dfa_size_limit(1 << 20)
                    .build()
                {
                    Ok(compiled) => (
                        super::v13_read_modes::SearchMatcher::Regex(compiled),
                        "regex",
                    ),
                    Err(error) => {
                        return error_result(
                            None,
                            RefusalCode::BadValue,
                            format!("search regex is not a valid pattern: {error}"),
                        )
                    }
                }
            }
            Some(_) => {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "search regex must be a boolean",
                )
            }
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
            .into_iter()
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
        if let Some(scope) = scope.as_ref() {
            if let Err(error) = search_scope_prefix(scope) {
                return error_result(Some(scope.to_string()), error.code(), error.to_string());
            }
            let source = selected
                .first()
                .expect("a scoped source set is admitted above");
            let authority = match source.logical_view_read_authority(cancellation) {
                Ok(authority) => authority,
                Err(error) => {
                    return error_result_detailed(
                        Some(scope.to_string()),
                        RefusalDetail::ProviderAbsent,
                        error,
                    )
                }
            };
            let snapshot = match authority.snapshot(scope) {
                Ok(snapshot) => snapshot,
                Err(error) => return view_error_result(Some(scope.to_string()), error),
            };
            if let Err(error) = authority.validate_search_scope_owner(scope, &snapshot) {
                return view_error_result(Some(scope.to_string()), error);
            }
            if scope.segments()[0].kind() == NodeKind::Configuration
                || scope.segments()[0].name().is_none()
            {
                // The root and an unnamed kind branch both depend on the
                // configuration descriptor, but neither needs its projected
                // child collection before a text-search page is returned.
                let root_at =
                    QualifiedAddress::parse(&format!("{}:Configuration", scope.source_set()))
                        .expect("Configuration is a canonical root address");
                let descriptor = match authority.identity_export_path(&root_at) {
                    Ok(descriptor) => descriptor,
                    Err(error) => return view_error_result(Some(scope.to_string()), error),
                };
                if let Some(descriptor) = descriptor {
                    if let Err(error) = source
                        .retained_root()
                        .read_relative_regular_prefix(std::path::Path::new(&descriptor), 1)
                    {
                        let message = format!("search scope descriptor is unavailable: {error}");
                        return if error.kind() == std::io::ErrorKind::NotFound {
                            error_result(
                                Some(scope.to_string()),
                                RefusalCode::InvalidState,
                                message,
                            )
                        } else {
                            error_result_detailed(
                                Some(scope.to_string()),
                                RefusalDetail::SourceUnreadable,
                                message,
                            )
                        };
                    }
                }
            }
        }
        let source_sets = selected
            .iter()
            .map(|source| source.source_set_name().to_owned())
            .collect();
        let binding = SearchCursorBinding {
            workspace_identity: invocation.workspace_identity_hash().as_str().to_owned(),
            query: query.to_owned(),
            scope: scope.as_ref().map(ToString::to_string),
            mode: mode.to_owned(),
            kind: None,
            source_sets,
            result_fingerprint: None,
            page_limit: limit,
        };
        let cursor = match arguments.get("cursor") {
            None => None,
            Some(Value::String(token)) => match self.search_cursors.read(token, &binding) {
                Ok(cursor) => Some((token.as_str(), cursor)),
                Err(error) => return error.into_result(None),
            },
            Some(_) => {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "search cursor must be a string",
                )
            }
        };
        let offset = cursor.as_ref().map_or(0, |(_, cursor)| cursor.offset);
        let mut skip = offset;
        let mut matches = Vec::new();
        let mut uncovered = 0_usize;
        let mut uncovered_details = Vec::new();
        let mut scan_complete = true;
        for source in selected {
            let scope_at = scope.clone().unwrap_or_else(|| {
                QualifiedAddress::parse(&format!("{}:Configuration", source.source_set_name()))
                    .expect("actor source-set names and Configuration address are canonical")
            });
            let scope_prefix = match search_scope_prefix(&scope_at) {
                Ok(prefix) => prefix,
                Err(error) => {
                    return error_result(
                        Some(scope_at.to_string()),
                        error.code(),
                        error.to_string(),
                    )
                }
            };
            match source.search_bsl_literal(
                &matcher,
                &mut skip,
                limit.saturating_add(1).saturating_sub(matches.len()),
                scope_prefix.as_deref(),
                &scope_at,
                cancellation,
            ) {
                Ok(mut found) => {
                    let mut groups = std::collections::BTreeMap::<String, Vec<usize>>::new();
                    for (index, hit) in found.matches.iter().enumerate() {
                        if let Some(at) = hit.get("at").and_then(Value::as_str) {
                            groups.entry(at.to_owned()).or_default().push(index);
                        }
                    }
                    // A page retains its hits, not every full module projection.
                    // Each module is parsed once and released before the next one.
                    for (raw, indices) in groups {
                        let Ok(module) = QualifiedAddress::parse(&raw) else {
                            continue;
                        };
                        let Ok(authority) = source.logical_view_read_authority(cancellation) else {
                            continue;
                        };
                        for index in indices {
                            let hit = &mut found.matches[index];
                            let Some(line) = hit
                                .get("line")
                                .and_then(Value::as_u64)
                                .and_then(|line| usize::try_from(line).ok())
                            else {
                                continue;
                            };
                            let body = match authority.narrowest_body_for_lines(&module, line, line)
                            {
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
                    matches.extend(found.matches);
                    uncovered += found.uncovered;
                    uncovered_details.extend(
                        found
                            .details
                            .into_iter()
                            .take(32_usize.saturating_sub(uncovered_details.len())),
                    );
                    scan_complete &= found.scan_complete;
                }
                Err(error) => return error_result(None, RefusalCode::ProviderUnavailable, error),
            }
            if matches.len() > limit {
                scan_complete = false;
                break;
            }
        }
        self.search_page(
            matches,
            cursor,
            binding,
            uncovered,
            uncovered_details,
            scan_complete,
        )
    }

    fn search_page(
        &self,
        matches: Vec<Value>,
        cursor: Option<(&str, StoredSearchCursor)>,
        binding: SearchCursorBinding,
        uncovered: usize,
        uncovered_details: Vec<Value>,
        scan_complete: bool,
    ) -> DomainResult {
        let state = if uncovered > 0 {
            "partial"
        } else if scan_complete {
            "completed"
        } else {
            "in progress"
        };
        let summary = format!("{} BSL search {state}", binding.mode);
        let mut extra_data = Map::new();
        extra_data.insert("dataFreshness".into(), json!("unknown"));
        extra_data.insert("pageConsistency".into(), json!("live"));
        extra_data.insert(
            "fileCoverage".to_owned(),
            json!({
                "complete": scan_complete && uncovered == 0,
                "scanComplete": scan_complete,
                "uncovered": uncovered,
                "uncoveredIsLowerBound": !scan_complete,
                "detailsTruncated": uncovered > uncovered_details.len(),
                "details": uncovered_details,
            }),
        );
        self.search_page_with_data(matches, cursor, binding, summary, extra_data)
    }

    fn search_page_with_data(
        &self,
        matches: Vec<Value>,
        cursor: Option<(&str, StoredSearchCursor)>,
        binding: SearchCursorBinding,
        summary: String,
        extra_data: Map<String, Value>,
    ) -> DomainResult {
        use crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES;
        use crate::application::v13::view::PREFERRED_PAGE_BYTES;

        let mode = binding.mode.as_str();
        let limit = binding.page_limit;
        let offset = cursor.as_ref().map_or(0, |(_, cursor)| cursor.offset);
        let revision: Option<String> = None;
        let mut page_matches = Vec::new();
        let mut byte_stop = false;
        let measured_bytes = |items: &[Value]| {
            search_page_probe_bytes(mode, items, &summary, &extra_data, revision.as_deref())
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
            let issued = match cursor {
                Some((_, stored)) => self.search_cursors.insert_next(&stored, next_offset),
                None => self.search_cursors.insert_first(binding, next_offset),
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
    /// `lexical` — буквальное совпадение, `symbol` — символьный индекс,
    /// `semantic` — смысловая близость. Роли и провайдеры под ними построены
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

        let Some(role) = ProviderRole::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == role)
        else {
            let mut refusal = error_result(
                None,
                RefusalCode::BadValue,
                format!(
                    "search role `{role}` is unknown; use one of {}",
                    ProviderRole::ALL
                        .iter()
                        .map(|role| format!("`{}`", role.as_str()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
            refusal.next.push(serde_json::json!({
                "tool": "unica.search",
                "args": {"query": query},
                "reason": "буквальный поиск без роли",
            }));
            return refusal;
        };

        let context = invocation.workspace_context();
        let unscoped_lexical = role == ProviderRole::Lexical && !arguments.contains_key("scope");
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
                        crate::domain::platform_profile::PlatformProfile::v8_3_27()
                            .module_capability(&address)
                    {
                        match crate::infrastructure::logical_event_source::module_source_address(
                            &address, capability,
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
            None if unscoped_lexical => {}
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

        let mut search_context = if unscoped_lexical {
            None
        } else {
            match self.ports.resolve_code_search_context(context, &selector) {
                Ok((resolved, _)) => Some(resolved),
                Err(error) => return error_result(None, RefusalCode::ProviderUnavailable, error),
            }
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
        // before starting the provider. Lexical and role-free search stay local.
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
        let request = SearchRequest {
            query: query.to_string(),
            limit: PROVIDER_SEARCH_FETCH_LIMIT,
        };
        let selected_sources = if unscoped_lexical {
            invocation
                .admitted_source_set_names()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        } else {
            vec![search_context
                .as_ref()
                .and_then(|context| context.source_root.source_set.clone())
                .unwrap_or_default()]
        };
        let retained_roots = if role == ProviderRole::Lexical {
            match invocation.read_sources() {
                Ok(sources) => sources
                    .iter()
                    .map(|source| {
                        (
                            source.source_set_name().to_owned(),
                            source.retained_root().path().to_path_buf(),
                        )
                    })
                    .collect::<Vec<_>>(),
                Err(error) => return error_result(None, RefusalCode::InvalidState, error),
            }
        } else {
            Vec::new()
        };
        if role == ProviderRole::Lexical {
            if let Some(search_context) = search_context.as_mut() {
                if let Err(error) = bind_lexical_source_scope(search_context, &retained_roots) {
                    return error_result(None, RefusalCode::InvalidState, error);
                }
            }
        }
        let execution = match if unscoped_lexical {
            search_unscoped_lexical(
                self.ports.as_ref(),
                context,
                UnscopedLexicalSources {
                    names: &selected_sources,
                    retained_roots: &retained_roots,
                },
                registry,
                &request,
                cancellation,
                operational.code_intelligence(),
            )
        } else {
            search_selected_role(
                registry,
                role,
                &request,
                search_context
                    .as_ref()
                    .expect("explicit or default role context"),
                cancellation,
                operational.code_intelligence(),
            )
        } {
            Ok(execution) => execution,
            Err(error) => {
                if let Some(refusal) = engine.and_then(|engine| {
                    engine_failure_refusal(&context.cwd, engine, None, cancellation)
                }) {
                    return refusal;
                }
                let code = if error.starts_with(crate::domain::cancellation::CANCELLED_PREFIX) {
                    RefusalCode::Cancelled
                } else {
                    RefusalCode::ProviderUnavailable
                };
                return error_result(None, code, error);
            }
        };
        let binding = SearchCursorBinding {
            workspace_identity: invocation.workspace_identity_hash().as_str().to_owned(),
            query: query.to_owned(),
            scope: arguments
                .get("scope")
                .and_then(Value::as_str)
                .map(str::to_owned),
            mode: role.as_str().to_owned(),
            kind: None,
            source_sets: selected_sources,
            result_fingerprint: None,
            page_limit: limit,
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
        let project_hits = |hits: &[crate::domain::code_intelligence::ProviderSearchHit]| -> Result<Vec<Map<String, Value>>, ViewError> {
            use crate::domain::source_location::SourceLocation;
            use crate::domain::source_target::TargetKind;
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
        let built = match self
            .find_builder
            .build_for_search(&layout, deadline, cancellation)
        {
            Ok(built) => built,
            Err(error) => return find_build_error_result(None, error),
        };
        let scope_binding = scope.as_ref().map(ToString::to_string);
        if let Some(scope) = scope {
            if let Err((code, message)) = validate_name_scope(&built.index, &scope, first.kind()) {
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
            request = request.with_scope(scope);
        }
        let found = match built
            .index
            .find_all_checked(request, deadline, cancellation)
        {
            Ok(found) => found,
            Err(error) => return error_result(None, error.code(), error.to_string()),
        };
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
        extra_data.insert("approximate".to_owned(), Value::Bool(found.is_nearest()));
        let empty_page_bytes = search_page_probe_bytes("names", &[], &summary, &extra_data, None);
        if empty_page_bytes > crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES {
            return error_result(
                None,
                RefusalCode::ResultTooLarge,
                "name search page metadata exceeds the transport limit",
            );
        }
        // Names do not have a source revision lease. Rebuild the complete
        // ranked answer on each page and bind the cursor to the public stream
        // and coverage instead of claiming a source revision.
        let mut hasher = Sha256::new();
        hasher.update(b"unica-v13-names-result-v1\0");
        for (index, candidate) in found.candidates().iter().enumerate() {
            if index % 128 == 0 {
                if let Some(refusal) = name_search_interruption(deadline, cancellation) {
                    return refusal;
                }
            }
            let public = json!({
                "at": candidate.at(),
                "kind": candidate.kind(),
                "title": candidate.title(),
                "reason": candidate.reason(),
            });
            let bytes = serde_json::to_vec(&public).expect("name match is serializable");
            // An item after the first page must not receive a continuation
            // that could never carry it. The empty probe includes every fixed
            // response field and its cursor; adding one JSON array item adds
            // exactly its serialized byte length.
            if empty_page_bytes.saturating_add(bytes.len())
                > crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES
            {
                return error_result(
                    None,
                    RefusalCode::ResultTooLarge,
                    "one name search result exceeds the transport limit",
                );
            }
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(&bytes);
        }
        let evidence =
            json!({"approximate": found.is_nearest(), "sourceCoverage": coverage.clone()});
        let bytes = serde_json::to_vec(&evidence).expect("name coverage is serializable");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
        if let Some(refusal) = name_search_interruption(deadline, cancellation) {
            return refusal;
        }
        let binding = SearchCursorBinding {
            workspace_identity: invocation.workspace_identity_hash().as_str().to_owned(),
            query: query.to_owned(),
            scope: scope_binding,
            mode: "names".to_owned(),
            kind: arguments
                .get("kind")
                .and_then(Value::as_str)
                .map(str::to_owned),
            source_sets: selected
                .iter()
                .map(|source| source.name().to_owned())
                .collect(),
            result_fingerprint: Some(format!("names-sha256-v1:{:x}", hasher.finalize())),
            page_limit: limit,
        };
        let cursor = match arguments.get("cursor") {
            None => None,
            Some(Value::String(token)) => match self.search_cursors.read(token, &binding) {
                Ok(cursor) => Some((token.as_str(), cursor)),
                Err(error) => return error.into_result(None),
            },
            Some(_) => {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "search cursor must be a string",
                )
            }
        };
        let offset = cursor.as_ref().map_or(0, |(_, cursor)| cursor.offset);
        let matches = found
            .candidates()
            .iter()
            .skip(offset)
            .take(limit + 1)
            .map(|candidate| {
                json!({
                    "at": candidate.at(),
                    "kind": candidate.kind(),
                    "title": candidate.title(),
                    "reason": candidate.reason(),
                })
            })
            .collect();
        let result = self.search_page_with_data(matches, cursor, binding, summary, extra_data);
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
                None => 20,
                Some(value) => match bounded_usize(value).filter(|limit| *limit <= 50) {
                    Some(limit) => limit,
                    None => {
                        return error_result(
                            Some(at.to_string()),
                            RefusalCode::BadValue,
                            "check limit must be an integer from 1 through 50",
                        )
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
            let viewed = self.execute_view_arguments(invocation, &view_arguments, cancellation);
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
            Some(value) => match bounded_usize(value).filter(|limit| *limit <= 1_000) {
                Some(limit) => limit,
                None => {
                    return error_result(
                        None,
                        RefusalCode::BadValue,
                        "diff limit must be an integer from 1 through 1000",
                    )
                }
            },
            None => 100,
        };
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
        let mut left_data = left_result
            .data
            .as_ref()
            .expect("successful view has data")
            .clone();
        let mut right_data = right_result
            .data
            .as_ref()
            .expect("successful view has data")
            .clone();
        source_backed(&mut left_data);
        source_backed(&mut right_data);
        if left_data.get("kind") != right_data.get("kind") {
            return error_result(
                None,
                RefusalCode::IncomparableNodes,
                "diff requires nodes of the same logical kind",
            );
        }
        let mut changes = Vec::new();
        collect_json_changes("", &left_data, &right_data, limit + 1, &mut changes);
        let truncated = changes.len() > limit;
        changes.truncate(limit);
        let equal = changes.is_empty() && !truncated;
        // Diff has no continuation yet (#1217): a truncated answer must say
        // that the rest is missing and how to ask a smaller question.
        let mut result = if truncated {
            let narrower = if limit < 1_000 {
                "compare smaller nodes or call again with a larger limit (at most 1000)"
            } else {
                "compare smaller nodes"
            };
            DomainResult::success(format!(
                "logical nodes compared; only the first {limit} differences are returned and the rest are omitted; no continuation exists, so {narrower}"
            ))
        } else {
            DomainResult::success("logical nodes compared")
        };
        result.data = Some(serde_json::json!({
            "equal": equal,
            "changes": changes,
            "truncated": truncated,
        }));

        result
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
            if !["at", "path"].contains(&key.as_str()) {
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
        let request = match ResolveRequest::new(at, path) {
            Ok(request) => request,
            Err(error) => return error_result(None, error.code(), error.to_string()),
        };
        match request {
            ResolveRequest::Path(path) => self.resolve_path(invocation, &path, cancellation),
            ResolveRequest::Address(address) => {
                self.resolve_address(invocation, &address, cancellation)
            }
        }
    }

    fn resolve_path(
        &self,
        invocation: &ActorBoundExecution,
        path: &str,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let sources = match invocation.layout_sources() {
            Ok(sources) => sources,
            Err(error) => return error_result(None, RefusalCode::ProviderUnavailable, error),
        };
        let Some(deadline) = sources.first().map(|source| source.deadline()) else {
            return error_result_detailed(
                None,
                RefusalDetail::ProviderAbsent,
                "resolve has no admitted source sets",
            );
        };
        let layout = sources
            .iter()
            .map(|source| LayoutFindSource::new(source.name(), source.kind(), source.root()))
            .collect::<Vec<_>>();
        let entry = match self
            .find_builder
            .locate_path(&layout, path, deadline, cancellation)
        {
            Ok(entry) => entry,
            Err(error) => return find_build_error_result(None, error),
        };
        let Some(entry) = entry else {
            return error_result(
                None,
                RefusalCode::NotFound,
                format!("no admitted source set places `{path}`"),
            );
        };
        // Кандидаты на присоединённые файлы подтверждаются тем же читателем,
        // что и view: регистрацию владельца и точный ресурс нельзя вывести
        // только из правдоподобной раскладки.
        if matches!(entry.kind(), "Module" | "Metadata")
            || entry
                .placed_path()
                .is_some_and(|path| path.ends_with("/Ext/Form.xml"))
        {
            let address = match QualifiedAddress::parse(entry.at()) {
                Ok(address) => address,
                Err(error) => return error_result(None, RefusalCode::BadValue, error.to_string()),
            };
            let authority = match invocation
                .layout_read_source(address.source_set())
                .and_then(|source| source.logical_view_read_authority(cancellation))
            {
                Ok(authority) => authority,
                Err(error) => {
                    return error_result_detailed(
                        Some(entry.at().to_owned()),
                        RefusalDetail::SourceUnreadable,
                        error,
                    )
                }
            };
            return match Self::resolve_with_authority(&authority, &address, entry.placed_path()) {
                Ok(source) if Some(source.path()) == entry.placed_path() => resolve_result(source),
                Ok(_) => error_result(
                    Some(entry.at().to_owned()),
                    RefusalCode::InvalidSource,
                    "the logical reader does not place the requested resource at this path",
                ),
                Err(error) => view_error_result(Some(entry.at().to_owned()), error),
            };
        }
        resolve_result(ResolvedSource::new(
            entry.at(),
            entry.kind(),
            entry.placed_path().expect("located path has a place"),
            ResolvedLines::NotLineBased,
        ))
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
    let target = crate::domain::source_target::MetadataAddress::parse(
        crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        target,
    )
    .ok()?;
    let relative = crate::infrastructure::logical_event_source::metadata_descriptor_relative(
        &target,
        source_kind,
    )
    .ok()?;
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
    let progress = crate::infrastructure::engine_delivery::CanonicalDeliveryProgress::default();
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
    deadlines: crate::domain::operational_config::CodeIntelligenceDeadlines,
) -> Result<crate::application::code_intelligence::CodeSearchExecution, String> {
    use crate::application::code_intelligence::CodeSearchCoordinator;

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
        &crate::domain::progress::NoopProgressSink,
    )
}

struct UnscopedLexicalSources<'a> {
    names: &'a [String],
    retained_roots: &'a [(String, std::path::PathBuf)],
}

/// Bind every lexical route to the actor's source-root ownership. Nested
/// source sets own their files even when Git ignores their source export.
fn bind_lexical_source_scope(
    context: &mut CodeIntelligenceContext,
    retained_roots: &[(String, std::path::PathBuf)],
) -> Result<(), String> {
    let name = context
        .source_root
        .source_set
        .as_deref()
        .unwrap_or_default();
    let root = &context.source_root.path;
    if !retained_roots
        .iter()
        .any(|(retained_name, retained_root)| retained_name == name && retained_root == root)
    {
        return Err(format!(
            "source set `{name}` no longer matches its actor-admitted root"
        ));
    }
    let scope = context
        .search_scope
        .as_mut()
        .ok_or_else(|| "resolved lexical source omitted its scope".to_string())?;
    for (_, other_root) in retained_roots {
        if let Ok(relative) = other_root.strip_prefix(root) {
            if !relative.as_os_str().is_empty() {
                scope.excluded_subtrees.push(relative.to_path_buf());
            }
        }
    }
    Ok(())
}

fn search_unscoped_lexical(
    ports: &crate::infrastructure::application_ports::InfrastructureApplicationPorts,
    workspace: &crate::domain::workspace::WorkspaceContext,
    sources: UnscopedLexicalSources<'_>,
    registry: CodeIntelligenceRegistry,
    request: &SearchRequest,
    cancellation: &CancellationToken,
    deadlines: crate::domain::operational_config::CodeIntelligenceDeadlines,
) -> Result<crate::application::code_intelligence::CodeSearchExecution, String> {
    use crate::application::code_intelligence::CodeSearchExecution;
    use crate::application::ports::ApplicationPorts;
    use crate::domain::code_intelligence::{
        CodeSearchResult, ProviderSearchSection, ProviderSectionStatus, SearchCoverage,
        SearchOrdering, SearchRanking,
    };

    let source_sets = sources.names;
    if source_sets.is_empty() {
        return Err("no admitted source set is available for a role search".to_string());
    }
    let started = Instant::now();
    let budget = deadlines.search_total_timeout();
    let mut contexts = source_sets
        .iter()
        .map(|name| {
            let selector = Map::from_iter([("sourceSet".to_string(), Value::String(name.clone()))]);
            ports
                .resolve_code_search_context(workspace, &selector)
                .map(|(context, _)| context)
        })
        .collect::<Result<Vec<_>, _>>()?;
    for context in contexts.iter_mut() {
        bind_lexical_source_scope(context, sources.retained_roots)?;
    }

    let provider = registry
        .search_providers()
        .find(|candidate| candidate.identity().role == ProviderRole::Lexical)
        .cloned()
        .ok_or_else(|| "no search provider is registered for `lexical`".to_string())?;
    let identity = provider.identity();
    let mut hits = Vec::new();
    let mut diagnostics = Vec::new();
    let mut warnings = Vec::new();
    let mut incomplete = None;
    for (name, context) in source_sets.iter().zip(contexts.iter()) {
        if cancellation.is_cancelled() {
            return Err(crate::domain::cancellation::cancelled_error(
                "unscoped lexical search stopped between source sets",
            ));
        }
        let remaining = budget.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            diagnostics.push(format!(
                "source set `{name}` was not searched before the deadline"
            ));
            incomplete = Some(ProviderSectionStatus::TimedOut);
            break;
        }
        let selected = CodeIntelligenceRegistry::new(vec![Arc::clone(&provider)])?;
        let source_request = SearchRequest {
            query: request.query.clone(),
            limit: request.limit.saturating_sub(hits.len()),
        };
        let execution = search_selected_role(
            selected,
            ProviderRole::Lexical,
            &source_request,
            context,
            cancellation,
            deadlines.within_search_budget(remaining),
        )?;
        let section = execution
            .result
            .sections
            .into_iter()
            .next()
            .ok_or_else(|| format!("lexical provider omitted source set `{name}`"))?;
        warnings.extend(
            execution
                .warnings
                .into_iter()
                .map(|warning| format!("{name}: {warning}")),
        );
        diagnostics.extend(
            section
                .diagnostics
                .into_iter()
                .map(|diagnostic| format!("{name}: {diagnostic}")),
        );
        hits.extend(section.hits);
        match section.status {
            ProviderSectionStatus::Ok | ProviderSectionStatus::Empty => {}
            status => {
                diagnostics.push(format!(
                    "source set `{name}` search ended as {}",
                    status.as_str()
                ));
                if incomplete.is_none()
                    || status == ProviderSectionStatus::TimedOut
                    || (status == ProviderSectionStatus::Partial
                        && incomplete != Some(ProviderSectionStatus::TimedOut))
                {
                    incomplete = Some(status);
                }
            }
        }
        if hits.len() >= request.limit {
            if incomplete.is_some() && incomplete != Some(ProviderSectionStatus::LimitReached) {
                diagnostics.push("lexical result limit was also reached".to_string());
            }
            incomplete.get_or_insert(ProviderSectionStatus::LimitReached);
            break;
        }
    }
    let section = match incomplete {
        None => ProviderSearchSection::complete(
            identity,
            SearchRanking::None,
            SearchOrdering::ProviderTraversal,
            hits,
            diagnostics,
        )?,
        Some(ProviderSectionStatus::LimitReached) => ProviderSearchSection::limit_reached(
            identity,
            SearchRanking::None,
            SearchOrdering::ProviderTraversal,
            hits,
            diagnostics,
        )?,
        Some(ProviderSectionStatus::TimedOut) => ProviderSearchSection::timed_out(
            identity,
            SearchRanking::None,
            SearchOrdering::ProviderTraversal,
            hits,
            diagnostics,
        )?,
        Some(_) if hits.is_empty() => {
            ProviderSearchSection::failed_with_diagnostics(identity, diagnostics)
        }
        Some(_) => ProviderSearchSection::partial(
            identity,
            SearchRanking::None,
            SearchOrdering::ProviderTraversal,
            hits,
            diagnostics,
        )?,
    };
    let complete = section.search_complete;
    let reportable = complete
        || matches!(
            section.status,
            ProviderSectionStatus::Partial
                | ProviderSectionStatus::LimitReached
                | ProviderSectionStatus::TimedOut
        ) && (!section.hits.is_empty() || section.status == ProviderSectionStatus::TimedOut);
    if reportable && section.status == ProviderSectionStatus::Partial {
        warnings.push("lexical search contains partial source results".to_string());
    }
    Ok(CodeSearchExecution {
        ok: reportable,
        result: CodeSearchResult {
            coverage: if complete {
                SearchCoverage::Complete
            } else if reportable {
                SearchCoverage::Partial
            } else {
                SearchCoverage::None
            },
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            sections: vec![section],
        },
        warnings,
        errors: Vec::new(),
    })
}

/// Ошибка движка может содержать stderr и физические пути. Причину берём
/// только из типизированной остановки, а данные отказа собираем явно.
fn provider_search_failure(
    section: crate::domain::code_intelligence::ProviderSearchSection,
) -> DomainResult {
    use crate::domain::code_intelligence::SearchTerminationCode;

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
    execution: crate::application::code_intelligence::CodeSearchExecution,
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
    execution: crate::application::code_intelligence::CodeSearchExecution,
    mut binding: SearchCursorBinding,
    cursor_token: Option<&str>,
    cancellation: &CancellationToken,
    project_hits: &F,
) -> DomainResult
where
    F: Fn(
        &[crate::domain::code_intelligence::ProviderSearchHit],
    ) -> Result<Vec<Map<String, Value>>, ViewError>,
{
    use crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES;
    use crate::application::v13::view::PREFERRED_PAGE_BYTES;

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
        let scope_hint = if role == ProviderRole::Lexical {
            " or scope"
        } else {
            ""
        };
        warnings.push(format!(
            "Search returned 200 matches. Ask the user to narrow the query{scope_hint} before searching again."
        ));
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

fn collect_json_changes(
    path: &str,
    left: &Value,
    right: &Value,
    limit: usize,
    changes: &mut Vec<Value>,
) {
    if left == right || changes.len() >= limit {
        return;
    }
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => {
            let keys = left
                .keys()
                .chain(right.keys())
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            for key in keys {
                let escaped = key.replace('~', "~0").replace('/', "~1");
                let child_path = format!("{path}/{escaped}");
                match (left.get(key), right.get(key)) {
                    (Some(left), Some(right)) => {
                        collect_json_changes(&child_path, left, right, limit, changes)
                    }
                    (left, right) => changes.push(serde_json::json!({
                        "path": child_path,
                        "left": left,
                        "right": right,
                    })),
                }
                if changes.len() >= limit {
                    break;
                }
            }
        }
        _ => changes.push(serde_json::json!({
            "path": if path.is_empty() { "/" } else { path },
            "left": left,
            "right": right,
        })),
    }
}

fn find_build_error_result(at: Option<String>, error: FindBuildError) -> DomainResult {
    match error.detail() {
        Some(detail) => error_result_detailed(at, detail, error.to_string()),
        None => error_result(at, error.code(), error.to_string()),
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
    let mut probe = DomainResult::success(summary.to_owned());
    probe.data = Some(search_page_data(mode, matches, extra_data));
    // "complete" is the longest terminal reason; a cursor also reserves
    // space even when this turns out to be the final page.
    probe.page = Some(json!({"stoppedBy": "complete"}));
    probe.cursor = Some(SearchCursorStore::token_probe());
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
fn run_bsl_diagnostics(
    ports: &crate::infrastructure::application_ports::InfrastructureApplicationPorts,
    address: &QualifiedAddress,
    invocation: &ActorBoundExecution,
    cancellation: &CancellationToken,
) -> Result<CheckStepVerdict, Box<DomainResult>> {
    use crate::application::diagnostics::DiagnosticCoordinator;
    use crate::application::ports::ApplicationPorts;
    use crate::domain::diagnostics::{DiagnosticAction, DiagnosticFilter, DiagnosticRequest};

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
    let mapping = authority
        .diagnostic_mapping(address, context)
        .map_err(|error| Box::new(view_error_result(Some(address.to_string()), error)))?;
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
    // Анализатор, которого ещё нет на машине, доставляется этим же вызовом,
    // как у поиска с ролью: незавершённая поставка отвечает `working`
    // с повтором, неудачная — своим классом и полями движка (#1338), а
    // движок, которого и после поставки нет, — общим отказом (#549).
    let analyzer_engine = true;
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
        action: DiagnosticAction::Analyze,
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
        Some((mapping.module_scope(), source.deadline())),
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
            Ok(CheckStepVerdict {
                passed,
                diagnostics: findings,
                suppressed: bsl_suppression_facts(&result),
            })
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
    mapping: &crate::infrastructure::diagnostics::CanonicalModuleDiagnosticMapping,
    address: &QualifiedAddress,
    cancellation: &CancellationToken,
) -> Result<Vec<crate::domain::diagnostics::DiagnosticItem>, Box<DomainResult>> {
    use crate::domain::diagnostics::{DiagnosticFocus, DiagnosticItem, DiagnosticSeverity};
    use crate::domain::source_location::SourceLocation;
    use crate::domain::source_target::TargetKind;
    use crate::infrastructure::bsl_outline::{
        misplaced_variable_declarations, MISPLACED_VARIABLE_MESSAGE,
    };

    let text = match std::fs::read_to_string(mapping.module_scope().module_path) {
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

fn bsl_result_proves_full_verdict(result: &crate::domain::diagnostics::DiagnosticResult) -> bool {
    use crate::domain::diagnostics::DiagnosticResultState;
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
fn bsl_incomplete_message(result: &crate::domain::diagnostics::DiagnosticResult) -> String {
    use crate::application::diagnostics::public_diagnostic_error_message;
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
                    crate::domain::diagnostics::stream_error::canonical_stream_error(&error.message)
                }
                crate::domain::engine::BUNDLED_TOOL_MISSING => {
                    crate::application::diagnostics::canonical_missing_engine_reason(&error.message)
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
        if let crate::domain::diagnostics::DiagnosticItem::ResourceFailure { error, .. } = item {
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
    result: &crate::domain::diagnostics::DiagnosticResult,
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
fn bsl_provider_absent(result: &crate::domain::diagnostics::DiagnosticResult) -> bool {
    result.providers.iter().any(|section| {
        section.error.as_ref().is_some_and(|error| {
            matches!(
                error.code.as_str(),
                crate::domain::engine::BUNDLED_TOOL_MISSING
                    | "provider_install_invalid"
                    | "provider_start_failed"
            )
        })
    })
}

/// Подавление по настройке пользователя называется фактом рядом с
/// вердиктом: «прошёл» с базовой линией значит «прошёл после фильтра».
fn bsl_suppression_facts(result: &crate::domain::diagnostics::DiagnosticResult) -> Vec<Value> {
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

fn bsl_findings_passed(items: &[crate::domain::diagnostics::DiagnosticItem]) -> bool {
    use crate::domain::diagnostics::{DiagnosticItem, DiagnosticSeverity};
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

fn run_node_checks(
    ports: &crate::infrastructure::application_ports::InfrastructureApplicationPorts,
    invocation: &ActorBoundExecution,
    at: &str,
    kind: &str,
    viewed: Option<&Value>,
    cancellation: &CancellationToken,
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
            CheckStep::Native(validator) => {
                run_native_validator(&address, kind, validator, invocation, cancellation)
                    .map(CheckStepVerdict::from)
            }
            CheckStep::Meta => {
                run_meta_validator(&address, at, context, cancellation).map(CheckStepVerdict::from)
            }
            CheckStep::Bsl => run_bsl_diagnostics(ports, &address, invocation, cancellation),
        };
        match verdict {
            Err(refusal) => return *refusal,
            Ok(step_verdict) => {
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
) -> Result<(bool, Vec<Value>), Box<DomainResult>> {
    use crate::application::v13::check::normalize_native_outcome;
    use crate::infrastructure::native_operations::v13_analysis::{validate, validator_selector};

    let context = invocation.workspace_context();
    let at = address.to_string();
    let native = if matches!(
        validator,
        crate::application::v13::check::CheckValidator::Subsystem
            | crate::application::v13::check::CheckValidator::Dcs
            | crate::application::v13::check::CheckValidator::Mxl
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
        if validator == crate::application::v13::check::CheckValidator::Subsystem {
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

/// The typed metadata validator of one object descriptor: the same read and
/// validation the metadata family runs, folded into the check envelope.
fn run_meta_validator(
    address: &QualifiedAddress,
    at: &str,
    context: &crate::domain::workspace::WorkspaceContext,
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
    let target = crate::domain::source_target::MetadataAddress::parse(
        crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        &path,
    )
    .map_err(|error| {
        Box::new(error_result(
            Some(at.to_string()),
            RefusalCode::BadValue,
            error.to_string(),
        ))
    })?;
    let request = crate::application::metadata::MetaInfoRequest {
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
            let diagnostics = validated
                .diagnostics
                .iter()
                .map(|diagnostic| serde_json::to_value(diagnostic).unwrap_or(Value::Null))
                .collect::<Vec<_>>();
            Ok((
                validated.status == crate::domain::metadata::MetaValidationStatus::Passed,
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
    source: SourceSetTraits,
    template_format: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
) -> Result<(), Box<DomainResult>> {
    preflight_apply_operations_with(
        request,
        source,
        template_format,
        &|_| None,
        &crate::domain::apply::no_member_lookup,
    )
}

/// То же, что `preflight_apply_operations`, с поиском узла по имени: снятый
/// `dataSet` ведёт к настоящему, в том числе вложенному, набору, а имя
/// элемента формы — к его пути в форме.
pub(super) fn preflight_apply_operations_with(
    request: &crate::domain::apply::ApplyRequest,
    source: SourceSetTraits,
    template_format: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
    form_item: &dyn Fn(&QualifiedAddress) -> Option<crate::domain::apply::FormItemRole>,
    members: crate::domain::apply::MemberPathLookup<'_>,
) -> Result<(), Box<DomainResult>> {
    let registry = OperationRegistry::closed();
    for (index, operation) in request.ops().iter().enumerate() {
        let Some(descriptor) = registry.lookup(operation.name()) else {
            // Снятая операция ведёт к заменившей на узле-родителе (#1354).
            if let Some(retired) = crate::domain::apply::RetiredOperation::lookup(operation.name())
            {
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
        let descriptor = OperationSite::new(operation.at(), source)
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
        let Some(site) = OperationSite::new(operation.at(), source) else {
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
                if let Some(target) = OperationSite::new(&redirect.at, source).and_then(|target| {
                    descriptor.nearest_applicable_ancestor_with(&target, template_format, form_item)
                }) {
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
        if let Some(target) = descriptor.retired_query_alias_target(&site, members) {
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
                let refusal = error_result(
                    Some(format!("ops[{index}].at")),
                    RefusalCode::BadValue,
                    message,
                );
                // Узел, который читается, показывает свой словарь; адрес,
                // которого нет ни в одном дереве, ведёт к корню набора.
                let readable = crate::infrastructure::logical_tree::route_logical_address(
                    operation.at(),
                    source.profile(),
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
    retired: crate::domain::apply::RetiredOperation,
    source: SourceSetTraits,
    template_format: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
    form_item: &dyn Fn(&QualifiedAddress) -> Option<crate::domain::apply::FormItemRole>,
) -> DomainResult {
    let location = Some(format!("ops[{index}].op"));
    let site = OperationSite::new(at, source);
    let target = site
        .as_ref()
        .and_then(|site| retired.replacement_target(site, template_format, form_item));
    let replacement = retired.descriptor();
    match (target, replacement) {
        (Some(target), Some(replacement)) => {
            let refusal = error_result(
                location,
                RefusalCode::BadValue,
                format!(
                    "apply operation `{}` is retired: call `{}` at `{target}`; {}",
                    retired.op, retired.replacement, retired.arguments
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
                    retired.op, retired.replacement, retired.arguments
                ),
            );
            let readable =
                crate::infrastructure::logical_tree::route_logical_address(at, source.profile())
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
    descriptor: crate::domain::apply::OperationDescriptor,
    at: &QualifiedAddress,
    form_item: &dyn Fn(&QualifiedAddress) -> Option<crate::domain::apply::FormItemRole>,
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
    descriptor: crate::domain::apply::OperationDescriptor,
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
    error: &crate::infrastructure::native_operations::apply::ApplyPlanError,
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
    let address_only = OperationRegistry::closed()
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

fn validate_name_scope(
    directory: &FindIndex,
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
        return directory
            .has_address(&scope.to_string())
            .then_some(())
            .ok_or((
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
    ) && !directory.has_address(&format!("{}:Configuration", scope.source_set()))
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
        if !directory.has_address(&owner) {
            return Err((
                RefusalCode::NotFound,
                "name search scope owner does not exist",
            ));
        }
    }
    if segments
        .last()
        .is_some_and(|segment| segment.name().is_some())
        && !directory.has_address(&scope.to_string())
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
    if bytes.len() <= crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES {
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
    error: crate::application::v13::view::ViewError,
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
mod tests {
    use super::UnscopedLexicalSources;
    use crate::application::code_intelligence::CodeSearchExecution;
    use crate::application::result_store::SearchCursorBinding;
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::{
        CodeIntelligenceContext, CodeIntelligenceProvider, CodeIntelligenceRegistry,
        CodeSearchScope, ProviderCapability, ProviderDeadline, ProviderIdentity, ProviderRole,
        ProviderSearchSection, SearchOrdering, SearchRanking, SearchRequest,
    };
    use crate::domain::operational_config::CodeIntelligenceDeadlines;
    use crate::domain::source_roots::ResolvedSourceRoot;
    use crate::domain::workspace::WorkspaceContext;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn saved_apply_execution_refusal_distinguishes_token_from_internal_state() {
        use super::SavedApplyExecutionError;
        use crate::infrastructure::workspace_actor::ApplyPublicationError;
        for (error, code, outcome) in [
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
        use crate::infrastructure::native_operations::apply::{
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
            crate::domain::refusal::RefusalCode::StaleRevision
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
        use crate::domain::code_intelligence::{
            CodeSearchResult, ProviderSearchHit, SearchCoverage,
        };
        use crate::domain::source_location::SourceLocation;
        use crate::domain::source_target::TargetKind;

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
        use crate::domain::code_intelligence::IndexMaintenanceEvidence;
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
        let store = crate::application::result_store::SearchCursorStore::default();
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
                    "x".repeat(crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES);
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
        let store = crate::application::result_store::SearchCursorStore::default();
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
        let store = crate::application::result_store::SearchCursorStore::default();
        let cancellation = CancellationToken::new();
        let mut execution = provider_search_test_execution(25, 8, false);
        execution.result.sections[0].hits[24].snippet =
            "x".repeat(crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES);
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
            &crate::application::result_store::SearchCursorStore::default(),
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
        use crate::domain::diagnostics::{
            DiagnosticAction, DiagnosticResult, DiagnosticResultState, DiagnosticSelection,
        };
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
        use crate::domain::diagnostics::{
            DiagnosticAction, DiagnosticProviderSection, DiagnosticProviderStatus,
            DiagnosticResult, DiagnosticResultState, DiagnosticSelection, DiagnosticSuppression,
            DiagnosticSuppressionReason,
        };
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
        combined.items.push(crate::domain::diagnostics::DiagnosticItem::ResourceFailure {
            provider:"bsl-analyzer",
            location:crate::domain::source_location::SourceLocation::Addressed {
                source_set:"main".to_string(),metadata_path:None,
                target_kind:crate::domain::source_target::TargetKind::Module,
            },location_reason:None,
            error:crate::domain::diagnostics::DiagnosticError {
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
        use crate::domain::diagnostics::stream_error::{StreamErrorKind, MISSING_START_MESSAGE};
        use crate::domain::diagnostics::{
            DiagnosticAction, DiagnosticError, DiagnosticProviderSection, DiagnosticProviderStatus,
            DiagnosticResult, DiagnosticResultState, DiagnosticSelection,
        };
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
                    crate::application::diagnostics::public_diagnostic_error_message(code)
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
        use crate::domain::diagnostics::{
            DiagnosticAction, DiagnosticError, DiagnosticProviderSection, DiagnosticProviderStatus,
            DiagnosticResult, DiagnosticResultState, DiagnosticSelection,
        };
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
            crate::domain::engine::InstallMode::Source,
            crate::domain::engine::InstallMode::Marketplace,
        ] {
            let missing = crate::domain::engine::MissingEngine::new(
                "bsl-analyzer",
                "win-x64",
                "C:/private/engine/bsl-analyzer.exe",
                None,
                mode,
            );
            let mut result = failed("bundled_tool_missing");
            result.providers[0].error.as_mut().unwrap().message =
                crate::application::diagnostics::missing_engine_public_reason(&missing);
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
        use crate::domain::diagnostics::{DiagnosticFocus, DiagnosticItem, DiagnosticSeverity};
        use crate::domain::source_location::SourceLocation;
        use crate::domain::source_target::TargetKind;
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
        tool: crate::application::invocation_store::ToolIdentity,
        arguments: serde_json::Value,
    ) -> crate::domain::invocation::DomainResult {
        invoke_exact_read_with_service(
            workspace,
            tool,
            arguments,
            Arc::new(super::CanonicalV13ReadService::default()),
        )
    }

    fn invoke_exact_read_with_service(
        workspace: &std::path::Path,
        tool: crate::application::invocation_store::ToolIdentity,
        arguments: serde_json::Value,
        service: Arc<super::CanonicalV13ReadService>,
    ) -> crate::domain::invocation::DomainResult {
        use crate::application::ports::TokioClock;
        use crate::infrastructure::daemon::protocol::InvocationRequest;
        use crate::infrastructure::daemon::server::V5CanonicalInvocationRuntime;
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
        use crate::application::invocation_store::ToolIdentity;
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

    #[test]
    fn view_nested_regions_never_substitutes_the_final_name() {
        use crate::application::invocation_store::ToolIdentity;
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

    /// Resolving a metadata object by its own place answers exactly what the
    /// full layout directory answers, for every object shape it handles (#1457).
    #[test]
    fn resolve_metadata_point_matches_the_full_layout_directory() {
        use crate::application::invocation_store::ToolIdentity;
        use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
        use crate::infrastructure::v13_find::{LayoutFindSource, WorkspaceFindDirectoryBuilder};
        let workspace = resolve_exact_workspace();
        let source = std::fs::canonicalize(workspace.path().join("src")).unwrap();
        std::fs::create_dir_all(source.join("Catalogs/Items/Commands/Refresh/Ext")).unwrap();
        std::fs::write(
            source.join("Catalogs/Items/Commands/Refresh/Ext/CommandModule.bsl"),
            "",
        )
        .unwrap();
        let root = RetainedDirectoryCapability::open(&source).unwrap();
        let directory = WorkspaceFindDirectoryBuilder::default()
            .build(
                &[LayoutFindSource::new(
                    "main",
                    crate::domain::project_sources::SourceSetKind::Configuration,
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
        use crate::application::invocation_store::ToolIdentity;
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
        use crate::application::invocation_store::ToolIdentity;
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
        use crate::application::invocation_store::ToolIdentity;
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
        use crate::application::invocation_store::ToolIdentity;
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
        use crate::application::invocation_store::ToolIdentity;
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
        use crate::application::invocation_store::ToolIdentity;
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
            let budgeted =
                crate::application::v13::source_text::apply_response_budget(&whole, 16_384)
                    .into_owned();
            let at = budgeted.next[0]["args"]["at"].as_str().unwrap();
            let address = crate::domain::address::QualifiedAddress::parse(at).unwrap();
            let range = address.line_range().unwrap();
            assert_eq!(range.from, first);
            let part = invoke_exact_read(workspace.path(), ToolIdentity::View, json!({"at":at}));
            assert!(part.ok, "{part:?}");
            assert!(crate::application::v13::source_text::apply_response_budget(&part, 16_384).ok);
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
                !crate::application::v13::source_text::apply_response_budget(&one_more, 16_384).ok,
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
        use crate::application::invocation_store::ToolIdentity;
        use crate::domain::code_intelligence::ProviderSearchHit;
        use crate::domain::source_location::SourceLocation;
        use crate::domain::source_target::{
            MetadataAddress, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
        };
        let workspace = resolve_exact_workspace();
        std::fs::write(workspace.path().join("src/CommonModules/Probe/Ext/Module.bsl"), "// outside Needle\n\n// Needle doc\nПроцедура Ping()\n\tСообщить(\"Needle\");\nКонецПроцедуры\n").unwrap();
        for role in [
            ProviderRole::Lexical,
            ProviderRole::Symbol,
            ProviderRole::Semantic,
        ] {
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
        use crate::application::invocation_store::ToolIdentity;
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

    #[test]
    fn resolve_exact_module_body_numbers_lines_like_view() {
        use crate::application::invocation_store::ToolIdentity;
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
        use crate::application::invocation_store::ToolIdentity;
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
        use crate::application::invocation_store::ToolIdentity;
        use crate::application::operation_descriptors::ExecutionClass;
        use crate::application::ports::TokioClock;
        use crate::domain::invocation::{DomainResult, InvocationFailure};
        use crate::infrastructure::daemon::protocol::InvocationRequest;
        use crate::infrastructure::daemon::server::V5CanonicalInvocationRuntime;
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
    fn canonical_lexical_module_scope_searches_only_the_selected_file() {
        use crate::application::invocation_store::ToolIdentity;
        use crate::application::ports::TokioClock;
        use crate::infrastructure::daemon::protocol::InvocationRequest;
        use crate::infrastructure::daemon::server::V5CanonicalInvocationRuntime;

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
                "Catalog.Items.ObjectModule",
            ),
            (
                "main:Catalog.Items.Module.Manager",
                "Catalogs/Items/Ext/ManagerModule.bsl",
                "Catalog.Items.ManagerModule",
            ),
            (
                "main:Catalog.Items.Form.Main.Module.Form",
                "Catalogs/Items/Forms/Main/Ext/Form/Module.bsl",
                "Catalog.Items.Form.Main.FormModule",
            ),
            (
                "main:Catalog.Items.Command.Act.Module.Command",
                "Catalogs/Items/Commands/Act/Ext/CommandModule.bsl",
                "Catalog.Items.Command.Act.CommandModule",
            ),
            (
                "main:CommonModule.Shared",
                "CommonModules/Shared/Ext/Module.bsl",
                "CommonModule.Shared.Module",
            ),
            (
                "main:Module.Session",
                "Ext/SessionModule.bsl",
                "SessionModule",
            ),
            (
                "main:Catalog.Neighbor.Module.Object",
                "Catalogs/Neighbor/Ext/ObjectModule.bsl",
                "Catalog.Neighbor.ObjectModule",
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
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(super::CanonicalV13ReadService::default()),
            Arc::new(TokioClock),
        );
        let search = |scope: &str| {
            let request = InvocationRequest::new(
                ToolIdentity::Search,
                json!({"query":"Needle", "corpus":"text", "role":"lexical", "scope":scope, "limit":50}),
                std::fs::canonicalize(workspace.path()).unwrap().to_string_lossy(),
                7_000,
            ).unwrap();
            runtime
                .bind(request)
                .unwrap()
                .prepare()
                .unwrap()
                .execute(CancellationToken::new())
                .unwrap()
        };
        for (scope, _, metadata_path) in modules {
            let result = search(scope);
            assert!(result.ok, "{scope}: {result:?}");
            assert!(result.diagnostics.is_empty(), "{scope}: {result:?}");
            let hits = result.data.as_ref().unwrap()["matches"][0]["hits"]
                .as_array()
                .unwrap();
            assert_eq!(
                hits.len(),
                1,
                "{scope} must not include a neighbor: {result:?}"
            );
            assert_eq!(
                hits[0]["location"]["metadataPath"], metadata_path,
                "{scope}: {result:?}"
            );
        }
        for (scope, count) in [("main:Catalog.Items", 4), ("main:Configuration", 7)] {
            let result = search(scope);
            assert!(result.ok, "{scope}: {result:?}");
            assert_eq!(
                result.data.as_ref().unwrap()["matches"][0]["hits"]
                    .as_array()
                    .unwrap()
                    .len(),
                count,
                "{scope}: {result:?}"
            );
        }
        for scope in [
            "main:CommonModule.Empty",
            "main:Catalog.Items.Module.Manager",
        ] {
            if scope.ends_with("Manager") {
                std::fs::remove_file(source.join("Catalogs/Items/Ext/ManagerModule.bsl")).unwrap();
            }
            let result = search(scope);
            assert!(
                result.ok,
                "registered absent module is empty: {scope}: {result:?}"
            );
            assert!(result.diagnostics.is_empty(), "{scope}: {result:?}");
            assert!(
                result.data.as_ref().unwrap()["matches"][0]["hits"]
                    .as_array()
                    .unwrap()
                    .is_empty(),
                "{scope}: {result:?}"
            );
        }
        let result = search("main:Catalog.Unknown.Module.Object");
        assert!(
            !result.ok,
            "an unknown owner must not become a source-wide search: {result:?}"
        );
    }

    #[test]
    fn canonical_role_search_preserves_typed_failure_without_private_provider_text() {
        use crate::application::invocation_store::ToolIdentity;
        use crate::application::ports::TokioClock;
        use crate::infrastructure::daemon::protocol::InvocationRequest;
        use crate::infrastructure::daemon::server::V5CanonicalInvocationRuntime;

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
                Some(crate::domain::code_intelligence::IndexMaintenanceEvidence::StartFailed);
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
        use crate::application::invocation_store::ToolIdentity;
        use crate::application::ports::TokioClock;
        use crate::application::result_store::SearchCursorStore;
        use crate::infrastructure::daemon::protocol::InvocationRequest;
        use crate::infrastructure::daemon::server::V5CanonicalInvocationRuntime;

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
        use crate::application::invocation_store::ToolIdentity;
        use crate::application::ports::TokioClock;
        use crate::infrastructure::daemon::protocol::InvocationRequest;
        use crate::infrastructure::daemon::server::V5CanonicalInvocationRuntime;

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
        )
        .with_search_scope(CodeSearchScope::all(
            "main".into(),
            PathBuf::from("/workspace"),
            false,
        ))
    }

    #[derive(Clone, Copy)]
    enum SplitLexicalCase {
        PartialThenLimit,
        PartialThenFailed,
        PartialThenTimedOut,
        EmptyThenTimedOut,
    }

    struct SplitLexicalProvider(SplitLexicalCase);

    impl CodeIntelligenceProvider for SplitLexicalProvider {
        fn identity(&self) -> ProviderIdentity {
            ProviderIdentity::new(ProviderRole::Lexical, "split-lexical")
        }

        fn capabilities(&self) -> &[ProviderCapability] {
            &[ProviderCapability::Search]
        }

        fn search(
            &self,
            request: &SearchRequest,
            context: &CodeIntelligenceContext,
            _: ProviderDeadline,
            _: &CancellationToken,
        ) -> ProviderSearchSection {
            use crate::domain::code_intelligence::ProviderSearchHit;
            use crate::domain::source_location::SourceLocation;

            let name = context.source_root.source_set.as_deref().unwrap();
            let count = match (self.0, name) {
                (SplitLexicalCase::EmptyThenTimedOut, "first") => 0,
                (_, "first") => 1,
                (SplitLexicalCase::PartialThenLimit, _) => request.limit,
                _ => 0,
            };
            let hits = (0..count)
                .map(|line| ProviderSearchHit {
                    rank: None,
                    provider_score: None,
                    location: SourceLocation::Unaddressable {
                        source_set: name.to_string(),
                        owner_metadata_path: None,
                        path: "Module.bsl".to_string(),
                    },
                    line: line + 1,
                    end_line: None,
                    symbol: None,
                    kind: None,
                    snippet: "Needle".to_string(),
                    attributes: serde_json::Map::new(),
                })
                .collect();
            match (self.0, name) {
                (SplitLexicalCase::EmptyThenTimedOut, "first") => ProviderSearchSection::complete(
                    self.identity(),
                    SearchRanking::None,
                    SearchOrdering::ProviderTraversal,
                    hits,
                    Vec::new(),
                )
                .unwrap(),
                (_, "first") => ProviderSearchSection::partial(
                    self.identity(),
                    SearchRanking::None,
                    SearchOrdering::ProviderTraversal,
                    hits,
                    vec!["one record could not be read".to_string()],
                )
                .unwrap(),
                (SplitLexicalCase::PartialThenLimit, _) => ProviderSearchSection::limit_reached(
                    self.identity(),
                    SearchRanking::None,
                    SearchOrdering::ProviderTraversal,
                    hits,
                    Vec::new(),
                )
                .unwrap(),
                (SplitLexicalCase::PartialThenFailed, _) => ProviderSearchSection::failed(
                    self.identity(),
                    "second source failed".to_string(),
                ),
                _ => ProviderSearchSection::timed_out(
                    self.identity(),
                    SearchRanking::None,
                    SearchOrdering::ProviderTraversal,
                    hits,
                    vec!["second source deadline exceeded".to_string()],
                )
                .unwrap(),
            }
        }
    }

    struct PerSourceDeadlineProvider;

    impl CodeIntelligenceProvider for PerSourceDeadlineProvider {
        fn identity(&self) -> ProviderIdentity {
            ProviderIdentity::new(ProviderRole::Lexical, "per-source-deadline")
        }

        fn capabilities(&self) -> &[ProviderCapability] {
            &[ProviderCapability::Search]
        }

        fn search(
            &self,
            _: &SearchRequest,
            context: &CodeIntelligenceContext,
            deadline: ProviderDeadline,
            _: &CancellationToken,
        ) -> ProviderSearchSection {
            if context.source_root.source_set.as_deref() == Some("first") {
                std::thread::sleep(Duration::from_secs(1));
            } else if deadline.remaining().unwrap() < Duration::from_millis(2500) {
                return ProviderSearchSection::timed_out(
                    self.identity(),
                    SearchRanking::None,
                    SearchOrdering::ProviderTraversal,
                    Vec::new(),
                    vec!["the second source lacked its configured provider budget".to_string()],
                )
                .unwrap();
            }
            ProviderSearchSection::complete(
                self.identity(),
                SearchRanking::None,
                SearchOrdering::ProviderTraversal,
                Vec::new(),
                Vec::new(),
            )
            .unwrap()
        }
    }

    fn two_source_lexical_workspace(
    ) -> (tempfile::TempDir, WorkspaceContext, Vec<(String, PathBuf)>) {
        let root = tempfile::tempdir().unwrap();
        for name in ["first", "second"] {
            std::fs::create_dir(root.path().join(name)).unwrap();
        }
        std::fs::write(
            root.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: first\n    type: CONFIGURATION\n    path: first\n  - name: second\n    type: CONFIGURATION\n    path: second\n",
        )
        .unwrap();
        let workspace = WorkspaceContext {
            cwd: root.path().to_path_buf(),
            workspace_root: root.path().to_path_buf(),
            cache_root: root.path().join(".build"),
            workspace_epoch: 1,
        };
        let roots = ["first", "second"]
            .into_iter()
            .map(|name| {
                (
                    name.to_string(),
                    crate::infrastructure::source_roots::normalize_path_identity(
                        &root.path().join(name),
                    )
                    .unwrap(),
                )
            })
            .collect::<Vec<_>>();
        (root, workspace, roots)
    }

    #[test]
    fn unscoped_lexical_keeps_partial_evidence_at_the_global_limit() {
        let (_root, workspace, roots) = two_source_lexical_workspace();
        let execution = super::search_unscoped_lexical(
            &crate::infrastructure::application_ports::InfrastructureApplicationPorts::new(),
            &workspace,
            UnscopedLexicalSources {
                names: &["first".to_string(), "second".to_string()],
                retained_roots: &roots,
            },
            CodeIntelligenceRegistry::new(vec![Arc::new(SplitLexicalProvider(
                SplitLexicalCase::PartialThenLimit,
            ))])
            .unwrap(),
            &SearchRequest {
                query: "Needle".to_string(),
                limit: 200,
            },
            &CancellationToken::new(),
            CodeIntelligenceDeadlines::for_test(Duration::from_secs(5)),
        )
        .unwrap();
        assert!(execution.ok);
        let section = &execution.result.sections[0];
        assert_eq!(
            section.status,
            crate::domain::code_intelligence::ProviderSectionStatus::Partial
        );
        assert!(!section.search_complete);
        assert_eq!(section.hits.len(), 200);
        assert!(section
            .diagnostics
            .iter()
            .any(|item| item.contains("one record could not be read")));
    }

    #[test]
    fn unscoped_lexical_names_a_later_failed_or_timed_out_source() {
        use crate::domain::code_intelligence::ProviderSectionStatus;

        for (case, expected) in [
            (
                SplitLexicalCase::PartialThenFailed,
                ProviderSectionStatus::Partial,
            ),
            (
                SplitLexicalCase::PartialThenTimedOut,
                ProviderSectionStatus::TimedOut,
            ),
        ] {
            let (_root, workspace, roots) = two_source_lexical_workspace();
            let execution = super::search_unscoped_lexical(
                &crate::infrastructure::application_ports::InfrastructureApplicationPorts::new(),
                &workspace,
                UnscopedLexicalSources {
                    names: &["first".to_string(), "second".to_string()],
                    retained_roots: &roots,
                },
                CodeIntelligenceRegistry::new(vec![Arc::new(SplitLexicalProvider(case))]).unwrap(),
                &SearchRequest {
                    query: "Needle".to_string(),
                    limit: 200,
                },
                &CancellationToken::new(),
                CodeIntelligenceDeadlines::for_test(Duration::from_secs(5)),
            )
            .unwrap();
            assert!(execution.ok);
            let section = &execution.result.sections[0];
            assert_eq!(section.status, expected);
            assert_eq!(section.hits.len(), 1);
            assert!(
                section
                    .diagnostics
                    .iter()
                    .any(|item| item.contains("source set `second` search ended as")),
                "{section:#?}"
            );
        }
    }

    #[test]
    fn unscoped_lexical_reports_zero_hit_timeout_without_a_generic_refusal() {
        let (_root, workspace, roots) = two_source_lexical_workspace();
        let execution = super::search_unscoped_lexical(
            &crate::infrastructure::application_ports::InfrastructureApplicationPorts::new(),
            &workspace,
            UnscopedLexicalSources {
                names: &["first".to_string(), "second".to_string()],
                retained_roots: &roots,
            },
            CodeIntelligenceRegistry::new(vec![Arc::new(SplitLexicalProvider(
                SplitLexicalCase::EmptyThenTimedOut,
            ))])
            .unwrap(),
            &SearchRequest {
                query: "Needle".to_string(),
                limit: 200,
            },
            &CancellationToken::new(),
            CodeIntelligenceDeadlines::for_test(Duration::from_secs(5)),
        )
        .unwrap();
        assert!(execution.ok);
        let result = super::provider_search_page(
            &crate::application::result_store::SearchCursorStore::default(),
            ProviderRole::Lexical,
            execution,
            provider_search_test_binding(ProviderRole::Lexical, 20),
            None,
            &CancellationToken::new(),
        );
        let result = serde_json::to_value(result).unwrap();
        assert_eq!(result["ok"], true, "{result:#}");
        assert_eq!(result["data"]["matches"][0]["status"], "timedOut");
        assert_eq!(
            result["data"]["matches"][0]["termination"]["code"],
            "deadlineExceeded"
        );
        assert!(result["data"]["matches"][0]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("source set `second`")));
    }

    #[test]
    fn unscoped_lexical_keeps_each_provider_timeout_under_the_shared_total() {
        let (_root, workspace, roots) = two_source_lexical_workspace();
        let execution = super::search_unscoped_lexical(
            &crate::infrastructure::application_ports::InfrastructureApplicationPorts::new(),
            &workspace,
            UnscopedLexicalSources {
                names: &["first".to_string(), "second".to_string()],
                retained_roots: &roots,
            },
            CodeIntelligenceRegistry::new(vec![Arc::new(PerSourceDeadlineProvider)]).unwrap(),
            &SearchRequest {
                query: "Needle".to_string(),
                limit: 200,
            },
            &CancellationToken::new(),
            CodeIntelligenceDeadlines::for_test_values(
                Duration::from_secs(10),
                Duration::from_secs(10),
                Duration::from_secs(3),
                Duration::from_secs(10),
            ),
        )
        .unwrap();
        let section = &execution.result.sections[0];
        assert!(section.search_complete, "{section:#?}");
        assert_eq!(
            section.status,
            crate::domain::code_intelligence::ProviderSectionStatus::Empty
        );
    }

    #[test]
    fn explicit_search_roles_name_only_the_engine_they_execute() {
        assert_eq!(super::selected_role_engine(ProviderRole::Lexical), None);
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
        use crate::application::code_intelligence::CodeSearchExecution;
        use crate::domain::code_intelligence::{
            CodeSearchResult, ProviderSearchHit, SearchCoverage,
        };
        use crate::domain::source_location::SourceLocation;

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
            &crate::application::result_store::SearchCursorStore::default(),
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
        assert!(error.starts_with(crate::domain::cancellation::CANCELLED_PREFIX));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn computed_can_section_may_exceed_the_preferred_page_size() {
        let at = "main:Catalog.Товары";
        let mut result =
            crate::domain::invocation::DomainResult::success("logical collection page resolved");
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
        let mut result =
            crate::domain::invocation::DomainResult::success("logical collection page resolved");
        result.at = Some(at.to_string());
        result.page = Some(json!({"stoppedBy": "complete"}));
        result.data = Some(json!({"at": at, "items": ["X".repeat(
            crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES
        )]}));
        let checked = super::enforce_view_result_limit(result, at);
        assert!(!checked.ok);
        assert_eq!(checked.diagnostics[0]["code"], "result_too_large");
    }

    #[test]
    fn unfinished_work_outlives_task_handoff_and_completes_once() {
        crate::infrastructure::daemon::server::actor_capacity_tests::assert_unfinished_work_survives_handoff_and_completes_once();
    }
}
