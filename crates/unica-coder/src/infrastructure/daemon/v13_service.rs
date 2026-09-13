use super::server::{ActorBoundExecution, ActorBoundInvocation, CanonicalInvocationService};
/// Потолок страницы ветви графа: объявленный максимум поверхности.
const CALL_GRAPH_PAGE_LIMIT: usize = 50;

use super::v13_call_graph::{
    branch_collection, branch_direction, branch_owner, extend_method_node, fetch_summary,
    CallGraphSummary, CALL_GRAPH_SECTION,
};
use super::v13_read_modes::{filter_diff_data, project_view_sections, search_scope_prefix};
use crate::application::invocation_store::ToolIdentity;
use crate::application::operation_descriptors::ExecutionClass;
use crate::application::result_store::ViewCursorStore;
use crate::application::tool_contracts::SurfaceRelease;
use crate::application::v13::apply::parse_request as parse_apply_request;
use crate::application::v13::find::FindRequest;
use crate::application::v13::resolve::{ResolveRequest, ResolvedLines, ResolvedSource};
use crate::application::v13::tool_catalog::catalog_for;
use crate::application::v13::view::{ViewRequest, ViewService};
use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::apply::OperationRegistry;
use crate::domain::call_graph_identity::{CallGraphIdentity, CallGraphIdentityError};
use crate::domain::cancellation::CancellationToken;
use crate::domain::invocation::{DomainResult, InvocationFailure};
use crate::domain::refusal::RefusalCode;
use crate::infrastructure::native_operations::apply::{
    ApplyPlanErrorKind, ApplyStagedState, PlannedApplyEffects, StagedChangeKind, StagedFileState,
};
use crate::infrastructure::native_operations::apply_families::plan_hidden_v13_apply;
use crate::infrastructure::v13_find::{LayoutFindSource, WorkspaceFindDirectoryBuilder};
use crate::infrastructure::workspace_actor::{
    ApplyAdmissionError, ApplyEffectDisposition, ApplyPublicationErrorKind,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Canonical v0.13 service installed by the production v3 daemon composition.
/// Each public name has a useful closed mode; unfinished variants fail with a
/// typed `unsupported_*` result rather than pretending an engine is missing.
pub(crate) struct CanonicalV13ReadService {
    cursors: Arc<ViewCursorStore>,
    find_builder: WorkspaceFindDirectoryBuilder,
    /// Порты приложения живут столько же, сколько служба, а не сколько вызов.
    /// Внутри них стол доставок: он принадлежит серверу и переживает вызов,
    /// который доставку начал, чтобы её получил следующий. Порты на каждый
    /// вызов означали бы стол на каждый вызов — и доставки перестали бы
    /// делиться.
    ports: Arc<crate::infrastructure::application_ports::InfrastructureApplicationPorts>,
}

impl Default for CanonicalV13ReadService {
    fn default() -> Self {
        Self {
            cursors: Arc::new(ViewCursorStore::default()),
            find_builder: WorkspaceFindDirectoryBuilder::default(),
            ports: Arc::new(
                crate::infrastructure::application_ports::InfrastructureApplicationPorts::new(),
            ),
        }
    }
}

impl CanonicalInvocationService for CanonicalV13ReadService {
    fn prepare(
        &self,
        _invocation: &ActorBoundInvocation,
    ) -> Result<ExecutionClass, Box<DomainResult>> {
        Ok(ExecutionClass::InlineCandidate)
    }

    fn execute(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure> {
        match invocation.tool() {
            ToolIdentity::View => Ok(invocation
                .rejected_logical_read_result()
                .unwrap_or_else(|| self.execute_view(invocation, &cancellation))),
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
        }
    }
}

impl CanonicalV13ReadService {
    fn execute_apply(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let source_sets = invocation.admitted_source_set_names();
        let request = match parse_apply_request(invocation.arguments(), &source_sets) {
            Ok(request) => request,
            Err(error) => {
                return error_result(
                    Some(error.location().to_string()),
                    error.code(),
                    error.to_string(),
                )
            }
        };
        let (binding, admission) = match invocation.admit_apply(&request, cancellation) {
            Ok(admitted) => admitted,
            // A stale `ifRev` is a caller conflict with a known recovery —
            // re-read the revision and retry — so it answers with its own
            // code instead of masquerading as an unavailable provider.
            Err(error @ ApplyAdmissionError::StaleRevision { .. }) => {
                return error_result(
                    Some(request.at().to_string()),
                    RefusalCode::StaleRevision,
                    error.to_string(),
                )
            }
            Err(ApplyAdmissionError::Other(error)) => {
                return error_result(
                    Some(request.at().to_string()),
                    RefusalCode::ProviderUnavailable,
                    error,
                )
            }
        };
        for (index, operation) in request.ops().iter().enumerate() {
            let Some(descriptor) = OperationRegistry::closed().lookup(operation.name()) else {
                return with_node_dictionary(
                    error_result(
                        Some(format!("ops[{index}].op")),
                        RefusalCode::UnsupportedOperation,
                        format!(
                            "apply operation `{}` is not in the canonical registry",
                            operation.name()
                        ),
                    ),
                    &operation.at().to_string(),
                );
            };
            let target_kind = operation
                .at()
                .segments()
                .last()
                .expect("a parsed logical address has a terminal segment")
                .kind();
            if !descriptor.applies_to_operation_target(operation.at()) {
                return with_node_dictionary(
                    error_result(
                        Some(format!("ops[{index}].at")),
                        RefusalCode::BadValue,
                        format!(
                            "apply operation `{}` does not apply to {}",
                            operation.name(),
                            target_kind.as_str()
                        ),
                    ),
                    &operation.at().to_string(),
                );
            }
        }
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
                return error_result(
                    error.path().map(str::to_string),
                    apply_plan_error_code(error.kind()),
                    error.to_string(),
                )
            }
            Ok(planned) => planned,
        };
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
                return error_result(
                    Some("ops".to_string()),
                    RefusalCode::ProviderUnavailable,
                    error.to_string(),
                )
            }
        };
        let publication = match invocation.publish_prepared_apply(prepared) {
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
        result
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
        let mut request = match ViewRequest::new(at) {
            Ok(request) => request,
            Err(error) => return view_error_result(Some(at.to_string()), error),
        };
        if let Some(filter) = arguments.get("filter") {
            let Some(filter) = filter.as_object() else {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::BadValue,
                    "view filter must be an object",
                );
            };
            request = request.with_filter(filter.clone());
        }
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
            let owner_result =
                ViewService::with_shared_cursors(authority, Arc::clone(&self.cursors)).view(
                    match ViewRequest::new(&owner.to_string()) {
                        Ok(request) => request,
                        Err(error) => return view_error_result(Some(owner.to_string()), error),
                    },
                );
            if !owner_result.ok {
                return owner_result;
            }
            return match self.call_graph_summary(invocation, &owner, cancellation) {
                Ok(summary) => {
                    let mut answer = owner_result;
                    if let Some(reason) = summary.reason.clone() {
                        answer.warnings.push(json!({"callGraph": reason}));
                    }
                    answer.data = Some(branch_collection(
                        &address,
                        direction,
                        &summary,
                        // Резолвер адреса по пути к ветви не подключён: сосед,
                        // названный файлом, попадёт в `limits`, а не в ответ
                        // внутренним именем анализатора.
                        |_| None,
                    ));
                    answer.cursor = None;
                    answer
                }
                Err(refusal) => *refusal,
            };
        }
        let mut result =
            ViewService::with_shared_cursors(authority, Arc::clone(&self.cursors)).view(request);
        let sections = arguments
            .get("filter")
            .and_then(Value::as_object)
            .and_then(|filter| filter.get("sections"));
        let call_graph_requested = sections.and_then(Value::as_array).is_some_and(|sections| {
            sections
                .iter()
                .any(|section| section.as_str() == Some(CALL_GRAPH_SECTION))
        });
        if call_graph_requested && result.ok {
            let is_method = result
                .data
                .as_ref()
                .and_then(|data| data.get("kind"))
                .and_then(Value::as_str)
                == Some(NodeKind::Method.as_str());
            if !is_method {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::UnsupportedSection,
                    "view section `callGraph` is computed for a method node only",
                );
            }
            match self.call_graph_summary(invocation, &address, cancellation) {
                Ok(summary) => {
                    if let Some(reason) = summary.reason.clone() {
                        result.warnings.push(json!({"callGraph": reason}));
                    }
                    if let Some(data) = result.data.as_mut().and_then(Value::as_object_mut) {
                        extend_method_node(data, &address, &summary);
                    }
                }
                Err(refusal) => return *refusal,
            }
        }
        if result.ok {
            if let (Some(data), Some(sections)) = (result.data.as_ref(), sections) {
                match project_view_sections(data, sections) {
                    Ok(projected) => result.data = Some(projected),
                    Err(error) => {
                        return error_result(Some(at.to_string()), error.code(), error.to_string())
                    }
                }
            }
        }
        result
    }

    /// Сводка графа вызовов для одного метода.
    ///
    /// Личность узла строится из адреса; модуль, который анализатор называет
    /// файлом, требует разрешённого пути, и пока резолвер к этому маршруту не
    /// подключён, такой адрес отказывает названным случаем, а не отвечает
    /// пустым графом.
    fn call_graph_summary(
        &self,
        invocation: &ActorBoundExecution,
        method_at: &QualifiedAddress,
        cancellation: &CancellationToken,
    ) -> Result<CallGraphSummary, Box<DomainResult>> {
        let identity = CallGraphIdentity::from_address(method_at, None).map_err(|error| {
            Box::new(error_result(
                Some(method_at.to_string()),
                match error {
                    CallGraphIdentityError::PathRequired => RefusalCode::UnsupportedSection,
                    _ => RefusalCode::BadValue,
                },
                match error {
                    CallGraphIdentityError::PathRequired => {
                        "the analyzer names this module by file, and the call graph route does not resolve the path yet".to_string()
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
        fetch_summary(
            self.ports.as_ref(),
            workspace,
            method_at.source_set(),
            &identity,
            // Тот же потолок страницы, что у навигации: 50 — объявленный
            // максимум поверхности, и второго числа здесь не заводится.
            CALL_GRAPH_PAGE_LIMIT,
            operational.code_intelligence().provider_read_timeout(),
            cancellation,
        )
        .map_err(|error| {
            Box::new(error_result(
                Some(method_at.to_string()),
                RefusalCode::ProviderUnavailable,
                error,
            ))
        })
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
            Some(value) => match bounded_usize(value).filter(|limit| *limit <= 200) {
                Some(limit) => limit,
                None => {
                    return error_result(
                        None,
                        RefusalCode::BadValue,
                        "search limit must be an integer from 1 through 200",
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
            let viewed = self.execute_view_arguments(
                invocation,
                &Map::from_iter([("at".to_string(), Value::String(scope.to_string()))]),
                cancellation,
            );
            if !viewed.ok {
                return viewed;
            }
        }
        let mut matches = Vec::new();
        let mut revisions = Vec::new();
        for source in selected {
            revisions.push(source.revision_identity());
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
                limit.saturating_sub(matches.len()),
                scope_prefix.as_deref(),
                &scope_at,
                cancellation,
            ) {
                Ok(found) => matches.extend(found),
                Err(error) => return error_result(None, RefusalCode::ProviderUnavailable, error),
            }
            if matches.len() == limit {
                break;
            }
        }
        let mut result = DomainResult::success(format!("{mode} BSL search completed"));
        result.data = Some(serde_json::json!({
            "mode": mode,
            "matches": matches,
        }));
        result.rev = combined_revision(&revisions);
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
        use crate::application::code_intelligence::CodeSearchCoordinator;
        use crate::application::ports::ApplicationPorts;
        use crate::domain::code_intelligence::{ProviderRole, SearchRequest};

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
        // Канонический вход — логический адрес; порт разрешения контекста
        // говорит словарём v0.12. Перевод делается здесь и только здесь.
        let mut selector = Map::new();
        match arguments.get("scope").and_then(Value::as_str) {
            Some(scope) => match QualifiedAddress::parse(scope) {
                Ok(address) => {
                    selector.insert(
                        "sourceSet".to_string(),
                        Value::String(address.source_set().to_string()),
                    );
                    let owner = address
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
                        .join(".");
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
            None => match invocation.admitted_source_set_names().first() {
                Some(name) => {
                    selector.insert("sourceSet".to_string(), Value::String((*name).to_string()));
                }
                None => {
                    return error_result(
                        None,
                        RefusalCode::ProviderUnavailable,
                        "no admitted source set is available for a role search",
                    )
                }
            },
        }

        let (search_context, _scope) =
            match self.ports.resolve_code_search_context(context, &selector) {
                Ok(resolved) => resolved,
                Err(error) => return error_result(None, RefusalCode::ProviderUnavailable, error),
            };
        let registry = match self.ports.code_intelligence_registry() {
            Ok(registry) => registry,
            Err(error) => return error_result(None, RefusalCode::ProviderUnavailable, error),
        };
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
        let limit = arguments
            .get("limit")
            .and_then(bounded_usize)
            .filter(|limit| *limit <= 200)
            .unwrap_or(20);
        let request = SearchRequest {
            query: query.to_string(),
            limit,
        };
        let execution =
            match CodeSearchCoordinator::with_deadlines(registry, operational.code_intelligence())
                .search_observed(
                    &request,
                    &search_context,
                    cancellation,
                    &crate::domain::progress::NoopProgressSink,
                ) {
                Ok(execution) => execution,
                Err(error) => return error_result(None, RefusalCode::ProviderUnavailable, error),
            };
        // Провайдер, который не отработал, ничего не доказывает: пустой ответ
        // при неудачном прогоне выглядел бы как «искали и не нашли».
        if !execution.ok {
            return error_result(
                None,
                RefusalCode::ProviderUnavailable,
                format!("`{}` search did not complete", role.as_str()),
            );
        }
        let mut result = DomainResult::success(format!("{} search completed", role.as_str()));
        result.data = Some(serde_json::json!({
            "mode": role.as_str(),
            "matches": execution.result.sections,
        }));
        result
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
        if let Some(limit) = arguments.get("limit") {
            let Some(limit) = bounded_usize(limit) else {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "search limit must be a positive integer",
                );
            };
            request = match request.with_limit(limit) {
                Ok(request) => request,
                Err(error) => return error_result(None, error.code(), error.to_string()),
            };
        }
        let sources = match invocation.layout_sources() {
            Ok(sources) => sources,
            Err(error) => return error_result(None, RefusalCode::ProviderUnavailable, error),
        };
        let Some(deadline) = sources.first().map(|source| source.deadline()) else {
            return error_result(
                None,
                RefusalCode::ProviderUnavailable,
                "no admitted source set is available for a name search",
            );
        };
        let layout = sources
            .iter()
            .map(|source| LayoutFindSource::new(source.name(), source.kind(), source.root()))
            .collect::<Vec<_>>();
        let directory = match self.find_builder.build(&layout, deadline, cancellation) {
            Ok(directory) => directory,
            Err(error) => return error_result(None, error.code(), error.to_string()),
        };
        let found = directory.find(request);
        let matches: Vec<Value> = found
            .candidates()
            .iter()
            .map(|candidate| {
                serde_json::json!({
                    "at": candidate.at(),
                    "kind": candidate.kind(),
                    "title": candidate.title(),
                    "reason": candidate.reason(),
                })
            })
            .collect();
        let mut result = DomainResult::success("name search completed");
        result.data = Some(serde_json::json!({
            "mode": "names",
            "matches": matches,
            // Совпадение по близости — догадка, и она названа: читатель
            // обязан отличать «нашлось» от «похоже на».
            "approximate": found.is_nearest(),
        }));
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
                "check takes only `at`; the validators of a node follow from its kind",
            );
        }
        if let Some(at) = arguments.get("at") {
            let Some(at) = at.as_str() else {
                return error_result(None, RefusalCode::BadValue, "check at must be a string");
            };
            let view_arguments =
                Map::from_iter([("at".to_string(), Value::String(at.to_string()))]);
            let viewed = self.execute_view_arguments(invocation, &view_arguments, cancellation);
            if !viewed.ok {
                if let Some(refusal) = unreadable_target_format_refusal(invocation, at) {
                    return refusal;
                }
                return viewed;
            }
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
                result.rev = viewed.rev;
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
        let filter = arguments.get("filter");
        if arguments.contains_key("cursor") {
            return error_result(
                None,
                RefusalCode::UnsupportedCursor,
                "diff pagination cursors are not implemented",
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
        if let Some(filter) = filter {
            left_data = match filter_diff_data(&left_data, filter) {
                Ok(data) => data,
                Err(error) => return error_result(None, error.code(), error.to_string()),
            };
            right_data = match filter_diff_data(&right_data, filter) {
                Ok(data) => data,
                Err(error) => return error_result(None, error.code(), error.to_string()),
            };
        }
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
        let revisions = [left_result.rev, right_result.rev]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let mut result = DomainResult::success("logical nodes compared");
        result.data = Some(serde_json::json!({
            "equal": equal,
            "changes": changes,
            "truncated": truncated,
        }));
        result.rev = combined_revision(&revisions);
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
            return error_result(
                None,
                RefusalCode::ProviderUnavailable,
                "resolve has no admitted source sets",
            );
        };
        let layout = sources
            .iter()
            .map(|source| LayoutFindSource::new(source.name(), source.kind(), source.root()))
            .collect::<Vec<_>>();
        let directory = match self.find_builder.build(&layout, deadline, cancellation) {
            Ok(directory) => directory,
            Err(error) => return error_result(None, error.code(), error.to_string()),
        };
        let Some(entry) = directory.locate_path(path) else {
            return error_result(
                None,
                RefusalCode::NotFound,
                format!("no admitted source set places `{path}`"),
            );
        };
        // Путь называет файл, а не узел внутри него: строки не спрашивали.
        let Some(placed) = entry.placed_path() else {
            return error_result(
                None,
                RefusalCode::NotFound,
                format!("no admitted source set places `{path}`"),
            );
        };
        resolve_result(ResolvedSource::new(
            entry.at(),
            entry.kind(),
            placed,
            ResolvedLines::NotLineBased,
        ))
    }

    fn resolve_address(
        &self,
        invocation: &ActorBoundExecution,
        address: &QualifiedAddress,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let at = address.to_string();
        let sources = match invocation.read_sources() {
            Ok(sources) => sources,
            Err(error) => {
                return error_result(Some(at), RefusalCode::ProviderUnavailable, error);
            }
        };
        let Some(source) = sources
            .iter()
            .find(|source| source.source_set_name() == address.source_set())
        else {
            return error_result(
                Some(at),
                RefusalCode::ProviderUnavailable,
                "resolve source set was not admitted by the workspace actor",
            );
        };
        let root = source.retained_root();
        let layout = vec![LayoutFindSource::new(
            source.source_set_name(),
            source.source_kind(),
            root.as_ref(),
        )];
        let directory = match self
            .find_builder
            .build(&layout, source.deadline(), cancellation)
        {
            Ok(directory) => directory,
            Err(error) => return error_result(Some(at), error.code(), error.to_string()),
        };
        let owner = owning_metadata_address(address);
        let Some(entry) = directory.locate_address(&owner) else {
            return error_result(
                Some(at),
                RefusalCode::NotFound,
                format!("the source layout does not place `{owner}`"),
            );
        };
        let Some(placed) = entry.placed_path() else {
            return error_result(
                Some(at),
                RefusalCode::NotFound,
                format!("the source layout does not place `{owner}`"),
            );
        };
        let kind = entry.kind().to_string();
        let placed = placed.to_string();
        let lines = match self.resolve_lines(invocation, address, cancellation) {
            Ok(lines) => lines,
            Err(result) => return *result,
        };
        resolve_result(ResolvedSource::new(at, kind, placed, lines))
    }

    /// Строки обещаются там, где источник строчный. У BSL они настоящие, и
    /// проекция модуля их уже знает; у реквизита и элемента формы источник
    /// древовидный, и обещать строки значило бы обещать то, что развалится.
    fn resolve_lines(
        &self,
        invocation: &ActorBoundExecution,
        address: &QualifiedAddress,
        cancellation: &CancellationToken,
    ) -> Result<ResolvedLines, Box<DomainResult>> {
        if !address
            .segments()
            .iter()
            .any(|segment| segment.kind() == NodeKind::Module)
        {
            return Ok(ResolvedLines::NotLineBased);
        }
        let at = address.to_string();
        let view_arguments = Map::from_iter([("at".to_string(), Value::String(at.clone()))]);
        let viewed = self.execute_view_arguments(invocation, &view_arguments, cancellation);
        if !viewed.ok {
            return Err(Box::new(viewed));
        }
        let props = viewed
            .data
            .as_ref()
            .and_then(|data| data.get("props"))
            .cloned()
            .unwrap_or(Value::Null);
        let from = props.get("line").and_then(Value::as_u64);
        let to = props.get("endLine").and_then(Value::as_u64);
        match (from, to) {
            (Some(from), Some(to)) => Ok(ResolvedLines::Range {
                from: from as usize,
                to: to as usize,
            }),
            // Узел модуля есть, но собственного диапазона у него нет: предмет
            // занимает файл целиком, и называть часть было бы неверно.
            _ => Ok(ResolvedLines::NotLineBased),
        }
    }
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

/// Узел, который раскладка действительно размещает. Метод, область и тело
/// лежат внутри файла своего владельца, поэтому путь берётся у владельца, а
/// внутренняя часть адреса отвечает за строки.
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

fn combined_revision(revisions: &[String]) -> Option<String> {
    if revisions.is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(b"unica-v13-read-set-v1\0");
    for revision in revisions {
        hasher.update(revision.as_bytes());
        hasher.update([0]);
    }
    Some(format!("unica-read-set-sha256-v1:{:x}", hasher.finalize()))
}

fn apply_plan_error_code(kind: ApplyPlanErrorKind) -> RefusalCode {
    match kind {
        ApplyPlanErrorKind::BadValue => RefusalCode::BadValue,
        ApplyPlanErrorKind::NotFound => RefusalCode::NotFound,
        ApplyPlanErrorKind::ProviderUnavailable => RefusalCode::ProviderUnavailable,
        ApplyPlanErrorKind::InvalidState => RefusalCode::InvalidState,
        ApplyPlanErrorKind::InvalidSource => RefusalCode::InvalidSource,
        ApplyPlanErrorKind::Staging(_) => RefusalCode::ProviderUnavailable,
        ApplyPlanErrorKind::Postcondition => RefusalCode::PostconditionFailed,
    }
}

fn apply_publication_error_code(kind: ApplyPublicationErrorKind) -> RefusalCode {
    match kind {
        ApplyPublicationErrorKind::Cancelled => RefusalCode::Cancelled,
        ApplyPublicationErrorKind::Deadline => RefusalCode::DeadlineExceeded,
        ApplyPublicationErrorKind::ConcurrentRevision => RefusalCode::RevisionMismatch,
        ApplyPublicationErrorKind::ContainmentIdentity => RefusalCode::ProviderUnavailable,
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
/// которых его и зовут. Порты здесь строятся свои — путь диагностик стола
/// доставок не трогает, бинарь анализатора берётся из поставляемых
/// инструментов, — и потому общий стол сервера не нужен.
fn run_bsl_diagnostics(
    ports: &crate::infrastructure::application_ports::InfrastructureApplicationPorts,
    address: &QualifiedAddress,
    context: &crate::domain::workspace::WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<(bool, Vec<Value>), Box<DomainResult>> {
    use crate::application::diagnostics::DiagnosticCoordinator;
    use crate::application::ports::ApplicationPorts;
    use crate::domain::diagnostics::{
        DiagnosticAction, DiagnosticFilter, DiagnosticRequest, DiagnosticResultState,
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
    // Сужение обязательно: `metadata_path: None` означает анализ всего набора
    // исходников, и на боевой конфигурации это минуты работы и диагностики
    // чужих файлов, приписанные спрошенному узлу. Неразобранный адрес — отказ,
    // а не молчаливое расширение области.
    let owner = address
        .segments()
        .iter()
        .take_while(|segment| segment.name().is_some())
        .map(|segment| {
            let name = segment.name().unwrap_or_default();
            format!("{}.{name}", segment.kind().as_str())
        })
        .collect::<Vec<_>>()
        .join(".");
    let metadata_path = match crate::domain::source_target::MetadataAddress::parse(
        crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        &owner,
    ) {
        Ok(path) => Some(path),
        Err(error) => {
            return Err(Box::new(error_result(
                Some(address.to_string()),
                RefusalCode::BadValue,
                format!("BSL diagnostics cannot narrow to `{owner}`: {error}"),
            )))
        }
    };
    let request = DiagnosticRequest {
        action: DiagnosticAction::Analyze,
        source_set: address.source_set().to_string(),
        metadata_path,
        filter: DiagnosticFilter::default(),
        range: None,
        limit: 200,
        // Срок берётся из настройки пользователя. Подменять его выдуманным
        // умолчанием нельзя: человек настроил срок и не узнал бы, что
        // настройка не читается.
        timeout: Some(
            match crate::infrastructure::operational_config::load_operational_config(
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
        ),
    };
    match DiagnosticCoordinator::new(registry, ports).execute(&request, context, cancellation) {
        Ok(result) => {
            // Провайдер, который не отработал, не доказывает чистоту кода.
            // Пустой список находок при незавершённом прогоне выглядел бы как
            // «проверено и чисто» — худшее направление ошибки для инструмента
            // проверки.
            if !result.ok || result.state != DiagnosticResultState::Completed {
                return Err(Box::new(error_result(
                    Some(address.to_string()),
                    RefusalCode::ProviderUnavailable,
                    "BSL analysis did not complete, so the module is unproven",
                )));
            }
            let findings: Vec<Value> = result
                .items
                .iter()
                .filter_map(|item| serde_json::to_value(item).ok())
                .collect();
            // Провалом считается ошибка, а не всякая пометка: подсказка по
            // стилю не ломает модуль, и остальные валидаторы поверхности
            // судят так же.
            let passed = !result.items.iter().any(|item| {
                matches!(
                    item,
                    crate::domain::diagnostics::DiagnosticItem::Diagnostic {
                        severity: crate::domain::diagnostics::DiagnosticSeverity::Error,
                        ..
                    } | crate::domain::diagnostics::DiagnosticItem::ResourceFailure { .. }
                )
            });
            Ok((passed, findings))
        }
        Err(error) => Err(Box::new(error_result(
            Some(address.to_string()),
            RefusalCode::ProviderUnavailable,
            format!("{}: {}", error.code, error.message),
        ))),
    }
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
    let mut validators = Vec::new();
    for step in plan {
        let verdict = match step {
            CheckStep::Native(validator) => {
                run_native_validator(&address, kind, validator, context)
            }
            CheckStep::Meta => run_meta_validator(&address, at, context, cancellation),
            CheckStep::Bsl => run_bsl_diagnostics(ports, &address, context, cancellation),
        };
        match verdict {
            Err(refusal) => return *refusal,
            Ok((step_passed, step_diagnostics)) => {
                passed &= step_passed;
                validators.push(step.name());
                diagnostics.extend(step_diagnostics.into_iter().map(|mut diagnostic| {
                    if let Some(object) = diagnostic.as_object_mut() {
                        object.insert("validator".to_string(), Value::String(step.name().into()));
                    }
                    diagnostic
                }));
            }
        }
    }
    let mut result = DomainResult::success(if passed {
        "validation passed"
    } else {
        "validation reported findings"
    });
    result.at = Some(at.to_string());
    result.data = Some(serde_json::json!({
        "status": if passed { "passed" } else { "failed" },
        "at": at,
        "kind": kind,
        "validators": validators,
        "diagnostics": diagnostics,
    }));
    result
}

fn run_native_validator(
    address: &QualifiedAddress,
    kind: &str,
    validator: crate::application::v13::check::CheckValidator,
    context: &crate::domain::workspace::WorkspaceContext,
) -> Result<(bool, Vec<Value>), Box<DomainResult>> {
    use crate::application::v13::check::{normalize_native_outcome, CheckError};
    use crate::infrastructure::native_operations::v13_analysis::{validate, validator_selector};

    let at = address.to_string();
    let selector = validator_selector(validator, address, context)
        .map_err(|error| Box::new(error_result(Some(at.clone()), RefusalCode::BadValue, error)))?;
    let native = validate(validator, &selector, context);
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
        Err(CheckError::DependencyUnavailable) => Err(Box::new(error_result(
            Some(at),
            RefusalCode::ProviderUnavailable,
            "the native validator dependency is unavailable",
        ))),
        Err(error) => Err(Box::new(error_result(
            Some(at),
            error.code(),
            error.to_string(),
        ))),
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
            Err(Box::new(error_result(
                Some(at.to_string()),
                RefusalCode::ProviderUnavailable,
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

/// Отказ по операции называет маршрут к словарю узла, а не перечисляет
/// операции в тексте: перечень уже публикуется секцией `can`, и агенту нужен
/// путь к нему, а не копия внутри сообщения. Без этого пробелы словаря
/// `apply` не всплывают — агент повторяет вслепую.
fn with_node_dictionary(mut result: DomainResult, at: &str) -> DomainResult {
    result.next.push(serde_json::json!({
        "tool": "unica.view",
        "args": {"at": at, "filter": {"sections": ["can"]}},
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

fn error_result(at: Option<String>, code: RefusalCode, message: impl Into<String>) -> DomainResult {
    DomainResult::canonical_rejection(at, code, message)
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
    if let Some(next) = error.next() {
        result.next.push(next.clone());
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn logical_read_operation_budget_outlives_task_handoff_and_completes_once() {
        crate::infrastructure::daemon::server::actor_capacity_tests::assert_operation_budget_survives_handoff_and_completes_once(
            crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
        );
    }
}
