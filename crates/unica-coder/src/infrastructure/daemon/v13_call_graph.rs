//! Ветви `Caller` и `Callee` у узла метода.
//!
//! Граф вызовов живёт не в дереве исходников, а в индексе анализатора, поэтому
//! он и не приходит из читателя: служба спрашивает его отдельно и по запросу —
//! так же, как словарь `can`. Сводка по умолчанию не считается: на холодном
//! пространстве к двум запросам добавилось бы построение индекса.

use crate::domain::address::{AddressSegment, NodeKind, QualifiedAddress};
use crate::domain::call_graph_identity::{CallGraphIdentity, CallGraphIdentityError};
use crate::domain::code_intelligence::{
    CallEdgeProvenance, CallGraphDirection, CallGraphResult, CallGraphState,
};
use crate::domain::invocation::DomainResult;
use serde_json::{json, Map, Value};

/// Название секции, которой вызывающий просит сводку графа.
pub(super) const CALL_GRAPH_SECTION: &str = "callGraph";

pub(super) enum CallGraphFetchError {
    Provider(String),
    Changed,
}

impl From<String> for CallGraphFetchError {
    fn from(error: String) -> Self {
        Self::Provider(error)
    }
}

/// Что служба узнала у анализатора по обоим направлениям.
#[derive(Debug, Clone)]
pub(super) struct CallGraphSummary {
    pub(super) callers: CallGraphResult,
    pub(super) callees: CallGraphResult,
    /// Почему граф недоступен, если он недоступен.
    ///
    /// Состояние `unavailable` без причины прячет её: читатель видит, что
    /// графа нет, и не знает, движок не поставлен или запрос не дошёл. Причина
    /// идёт предупреждением ответа, а не подменяет состояние.
    pub(super) reason: Option<String>,
}

impl CallGraphSummary {
    /// Сводное состояние двух направлений.
    ///
    /// Хуже из двух: если одно направление ещё строится, счёт второго верен, но
    /// узел целиком назвать готовым нельзя — читатель принял бы неполную
    /// картину за полную.
    fn state(&self) -> CallGraphState {
        match (self.callers.state, self.callees.state) {
            (CallGraphState::Unavailable, _) | (_, CallGraphState::Unavailable) => {
                CallGraphState::Unavailable
            }
            (CallGraphState::Indexing, _) | (_, CallGraphState::Indexing) => {
                CallGraphState::Indexing
            }
            _ => CallGraphState::Ready,
        }
    }

    fn total(&self, direction: CallGraphDirection) -> Option<u64> {
        match direction {
            CallGraphDirection::Callers => self.callers.total,
            CallGraphDirection::Callees => self.callees.total,
        }
    }

    /// Есть ли в запрошенном направлении сосед, которого анализатор называет
    /// файлом: только ему нужна раскладка, чтобы получить адрес. Направление
    /// спрашивается именно потому, что страница отвечает за своё: сосед другого
    /// направления не повод строить раскладку и не повод отказать этой странице.
    pub(super) fn names_a_peer_by_file(&self, direction: CallGraphDirection) -> bool {
        self.result(direction)
            .edges
            .iter()
            .any(|edge| edge.id.starts_with("method/file/"))
    }

    pub(super) fn result(&self, direction: CallGraphDirection) -> &CallGraphResult {
        match direction {
            CallGraphDirection::Callers => &self.callers,
            CallGraphDirection::Callees => &self.callees,
        }
    }
}

/// A capped analyzer answer is only a sizing probe. Never publish its edges
/// as a complete branch, even when it contains enough for the first page.
pub(super) fn required_full_branch_limit(
    summary: &CallGraphSummary,
    direction: CallGraphDirection,
) -> Result<Option<usize>, String> {
    let result = summary.result(direction);
    if result.state != CallGraphState::Ready || result.complete {
        return Ok(None);
    }
    result
        .total
        .and_then(|total| usize::try_from(total).ok())
        .map(Some)
        .ok_or_else(|| "call graph neighbour count cannot be represented".to_string())
}

pub(super) fn full_branch_matches(
    first: &CallGraphSummary,
    full: &CallGraphSummary,
    direction: CallGraphDirection,
) -> bool {
    let first = first.result(direction);
    let full = full.result(direction);
    first.state == CallGraphState::Ready
        && full.state == CallGraphState::Ready
        && full.complete
        && full.total == first.total
        && full.revision == first.revision
}

#[derive(Debug)]
pub(super) enum CompleteBranchError<E> {
    CountTooLarge(String),
    Fetch(E),
    Changed,
    Incomplete,
}

/// Obtain one complete immutable branch before the caller publishes page one.
/// The initial 50-neighbour answer only sizes the request; a capped answer is
/// never returned to the pager. `fetch` runs only when another provider read is
/// necessary and its exact limit is derived from the provider's own total.
pub(super) fn complete_branch<D, E>(
    initial: (CallGraphSummary, Option<D>),
    direction: CallGraphDirection,
    fetch: impl FnOnce(usize) -> Result<(CallGraphSummary, Option<D>), E>,
) -> Result<(CallGraphSummary, Option<D>), CompleteBranchError<E>> {
    let limit = required_full_branch_limit(&initial.0, direction)
        .map_err(CompleteBranchError::CountTooLarge)?;
    let Some(limit) = limit else {
        return Ok(initial);
    };
    let (full, full_directory) = fetch(limit).map_err(CompleteBranchError::Fetch)?;
    let first_result = initial.0.result(direction);
    let full_result = full.result(direction);
    if full_result.state != CallGraphState::Ready {
        // Reindexing or provider loss is a named state, not a malformed
        // complete answer. Publish that state without any truncated edges or cursor.
        return Ok((full, full_directory.or(initial.1)));
    }
    if first_result.revision != full_result.revision || first_result.total != full_result.total {
        return Err(CompleteBranchError::Changed);
    }
    if !full_branch_matches(&initial.0, &full, direction) {
        return Err(CompleteBranchError::Incomplete);
    }
    Ok((full, full_directory.or(initial.1)))
}

/// Дополнить узел метода сводкой графа: счёт в `props`, направления в ветвях.
///
/// Состояние называется всегда, а счёт — только когда он известен. Опустить
/// состояние и оставить один счёт значило бы сделать «не посчитано»
/// неотличимым от «вызовов нет».
pub(super) fn extend_method_node(
    data: &mut Map<String, Value>,
    at: &QualifiedAddress,
    summary: &CallGraphSummary,
) {
    let state = summary.state();
    let props = data
        .entry("props".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if let Some(props) = props.as_object_mut() {
        props.insert(
            "callGraph".to_string(),
            serde_json::to_value(state).unwrap_or(Value::Null),
        );
        for (direction, key) in [
            (CallGraphDirection::Callers, "callers"),
            (CallGraphDirection::Callees, "callees"),
        ] {
            if let Some(total) = summary.total(direction) {
                props.insert(key.to_string(), json!(total));
            }
        }
    }
    // Ветвь объявляется только со счётом: объявить её без счёта значило бы
    // пообещать страницу, длины которой никто не знает.
    let mut branches = data
        .get("branches")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (direction, kind) in [
        (CallGraphDirection::Callers, NodeKind::Caller),
        (CallGraphDirection::Callees, NodeKind::Callee),
    ] {
        if let Some(total) = summary.total(direction) {
            branches.push(json!({
                "at": format!("{at}.{}", kind.as_str()),
                "count": total,
            }));
        }
    }
    if !branches.is_empty() {
        data.insert("branches".to_string(), Value::Array(branches));
    }
}

/// Страница одной ветви: элементы показывают наружу.
///
/// Элемент несёт адрес чужого метода и происхождение ребра. Личность, которую
/// в адрес перевести нечем, не выбрасывается и не отдаётся внутренним именем
/// анализатора: она названа в `limits`. Приём уже применён к командам
/// интерфейса, заданным идентификатором исчезнувшего объекта.
pub(super) fn branch_collection(
    at: &QualifiedAddress,
    direction: CallGraphDirection,
    summary: &CallGraphSummary,
    address_for_path: impl Fn(&str) -> Option<QualifiedAddress> + Copy,
) -> Value {
    let result = summary.result(direction);
    let kind = match direction {
        CallGraphDirection::Callers => NodeKind::Caller,
        CallGraphDirection::Callees => NodeKind::Callee,
    };
    let mut items = Vec::new();
    let mut unaddressable = 0usize;
    for edge in &result.edges {
        let peer = CallGraphIdentity::parse_analyzer_id(&edge.id)
            .and_then(|identity| identity.to_address(at.source_set(), address_for_path));
        match peer {
            Ok(address) => items.push(json!({
                "at": address.to_string(),
                "kind": address
                    .segments()
                    .last()
                    .map(AddressSegment::kind)
                    .unwrap_or(NodeKind::Method)
                    .as_str(),
                "provenance": match edge.provenance {
                    CallEdgeProvenance::Resolved => "resolved",
                    CallEdgeProvenance::Inferred => "inferred",
                },
            })),
            Err(CallGraphIdentityError::PathRequired) | Err(_) => unaddressable += 1,
        }
    }
    let mut node = Map::new();
    node.insert("at".to_string(), json!(at.to_string()));
    node.insert("kind".to_string(), json!(kind.as_str()));
    node.insert("title".to_string(), json!(kind.as_str()));
    let mut props = Map::new();
    props.insert(
        "callGraph".to_string(),
        serde_json::to_value(result.state).unwrap_or(Value::Null),
    );
    if let Some(stale) = result.stale {
        props.insert("stale".to_string(), json!(stale));
    }
    if let Some(revision) = result.revision {
        props.insert("graphRevision".to_string(), json!(revision));
    }
    node.insert("props".to_string(), Value::Object(props));
    node.insert("items".to_string(), Value::Array(items));
    if unaddressable > 0 {
        node.insert(
            "limits".to_string(),
            json!([{
                "kind": "unaddressablePeer",
                "count": unaddressable,
                "reason": "the analyzer names this method by file, and its logical address was not resolved",
            }]),
        );
    }
    Value::Object(node)
}

/// A graph that is still indexing or unavailable has no complete collection
/// to paginate. Name its state without a misleading completed page.
pub(super) fn unready_branch_result(
    at: &QualifiedAddress,
    direction: CallGraphDirection,
    summary: &CallGraphSummary,
) -> DomainResult {
    debug_assert_ne!(summary.result(direction).state, CallGraphState::Ready);
    let mut data = branch_collection(at, direction, summary, |_| None);
    data.as_object_mut()
        .expect("branch collection is a node")
        .remove("items");
    let mut result = DomainResult::success("call graph branch is not ready");
    result.at = Some(at.to_string());
    result.data = Some(data);
    if let Some(reason) = &summary.reason {
        result.warnings.push(json!({"callGraph": reason}));
    }
    result
}

/// Направление, которое называет адрес, если это адрес ветви графа.
pub(super) fn branch_direction(at: &QualifiedAddress) -> Option<CallGraphDirection> {
    let terminal = at.segments().last()?;
    if terminal.name().is_some() {
        return None;
    }
    match terminal.kind() {
        NodeKind::Caller => Some(CallGraphDirection::Callers),
        NodeKind::Callee => Some(CallGraphDirection::Callees),
        _ => None,
    }
}

/// Адрес метода, которому принадлежит ветвь.
pub(super) fn branch_owner(at: &QualifiedAddress) -> Option<QualifiedAddress> {
    let raw = at.to_string();
    let (owner, _) = raw.rsplit_once('.')?;
    QualifiedAddress::parse(owner).ok()
}

/// Спросить у анализатора оба направления для одного метода.
///
/// Провайдер выбирается по возможности, а не по имени: `CallGraph` объявлен
/// только у анализатора BSL, и если его в поставке нет, состояние называется
/// `unavailable`, а не превращается в нулевой счёт.
pub(super) fn fetch_summary(
    ports: &dyn crate::application::ports::ApplicationPorts,
    workspace: &crate::domain::workspace::WorkspaceContext,
    source_set: &str,
    identity: &CallGraphIdentity,
    limit: usize,
    budget: std::time::Duration,
    cancellation: &crate::domain::cancellation::CancellationToken,
) -> Result<CallGraphSummary, CallGraphFetchError> {
    let mut args = Map::new();
    args.insert("sourceSet".to_string(), json!(source_set));
    let (context, _scope) = ports.resolve_code_search_context(workspace, &args)?;
    let registry = ports.code_intelligence_registry()?;
    let id = identity.to_analyzer_id();
    let mut answers = Vec::with_capacity(2);
    let mut reason = None;
    for direction in [CallGraphDirection::Callers, CallGraphDirection::Callees] {
        let Some(provider) =
            registry.provider_for(crate::domain::code_intelligence::ProviderCapability::CallGraph)
        else {
            return Ok(CallGraphSummary {
                callers: unavailable(),
                callees: unavailable(),
                reason: Some("no code intelligence provider implements the call graph".to_string()),
            });
        };
        let outcome = crate::application::code_intelligence::execute_provider_read(
            provider,
            crate::domain::code_intelligence::CodeIntelligenceReadRequest::CallGraph {
                id: id.clone(),
                direction,
                limit,
            },
            context.clone(),
            budget,
            cancellation,
        );
        answers.push(match outcome {
            Ok(outcome) => match outcome.data {
                Some(crate::domain::code_intelligence::CodeIntelligenceReadData::CallGraph(
                    result,
                )) => {
                    if result.state == CallGraphState::Unavailable && reason.is_none() {
                        reason = outcome.errors.first().cloned();
                    }
                    result
                }
                // Провайдер ответил, но не графом: это дефект провода, и он
                // назван недоступностью, а не нулём.
                _ => {
                    reason = Some(format!(
                        "{} answered without a call graph",
                        outcome.provider.provider
                    ));
                    unavailable()
                }
            },
            Err(error) => {
                if reason.is_none() {
                    reason = Some(error);
                }
                unavailable()
            }
        });
    }
    let callees = answers.pop().expect("два направления");
    let callers = answers.pop().expect("два направления");
    if callers.state == CallGraphState::Ready
        && callees.state == CallGraphState::Ready
        && callers.revision != callees.revision
    {
        return Err(CallGraphFetchError::Changed);
    }
    Ok(CallGraphSummary {
        callers,
        callees,
        reason,
    })
}

/// Файл модуля, которым анализатор называет форму или команду, по месту их
/// дескриптора в раскладке: форма размещена по `…/Forms/<Имя>.xml`, её модуль
/// лежит в `…/Forms/<Имя>/Ext/Form/Module.bsl`; команда размещена каталогом,
/// её модуль — `Ext/CommandModule.bsl` в нём. Остальные виды анализатор
/// называет логически, и файла им не нужно.
pub(super) fn module_file_for_placed(kind: &str, placed: &str) -> Option<String> {
    match kind {
        "Form" => placed
            .strip_suffix(".xml")
            .map(|directory| format!("{directory}/Ext/Form/Module.bsl")),
        "Command" => Some(format!(
            "{}/Ext/CommandModule.bsl",
            placed.trim_end_matches('/')
        )),
        _ => None,
    }
}

/// Обратный перевод: по пути файла модуля из ответа анализатора — место
/// дескриптора в раскладке и роль модуля, которой заканчивается адрес.
pub(super) fn placed_for_module_file(path: &str) -> Option<(String, &'static str)> {
    if let Some(directory) = path.strip_suffix("/Ext/Form/Module.bsl") {
        return Some((format!("{directory}.xml"), "Form"));
    }
    if let Some(directory) = path.strip_suffix("/Ext/CommandModule.bsl") {
        return Some((directory.to_string(), "Command"));
    }
    None
}

fn unavailable() -> CallGraphResult {
    CallGraphResult {
        state: CallGraphState::Unavailable,
        total: None,
        edges: Vec::new(),
        revision: None,
        stale: None,
        complete: false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        branch_collection, branch_direction, branch_owner, complete_branch, extend_method_node,
        full_branch_matches, module_file_for_placed, placed_for_module_file,
        required_full_branch_limit, unready_branch_result, CallGraphSummary, CompleteBranchError,
    };
    use crate::domain::address::QualifiedAddress;
    use crate::domain::code_intelligence::{
        CallEdgeProvenance, CallGraphDirection, CallGraphEdge, CallGraphResult, CallGraphState,
    };
    use serde_json::{json, Map};

    fn address(raw: &str) -> QualifiedAddress {
        QualifiedAddress::parse(raw).expect("адрес разбирается")
    }

    fn ready(total: u64, edges: Vec<CallGraphEdge>) -> CallGraphResult {
        CallGraphResult {
            state: CallGraphState::Ready,
            total: Some(total),
            edges,
            revision: Some(1),
            stale: Some(false),
            complete: true,
        }
    }

    fn edge(id: &str, provenance: CallEdgeProvenance) -> CallGraphEdge {
        CallGraphEdge {
            id: id.to_string(),
            provenance,
        }
    }

    #[test]
    fn a_capped_graph_requires_a_complete_same_revision_snapshot_before_paging() {
        let mut first = CallGraphSummary {
            callers: ready(
                73,
                (0..50)
                    .map(|n| {
                        edge(
                            &format!("method/common/Модуль/Метод{n}"),
                            CallEdgeProvenance::Resolved,
                        )
                    })
                    .collect(),
            ),
            callees: ready(0, Vec::new()),
            reason: None,
        };
        first.callers.complete = false;
        assert_eq!(
            required_full_branch_limit(&first, CallGraphDirection::Callers).unwrap(),
            Some(73)
        );

        let mut full = first.clone();
        full.callers.complete = true;
        full.callers.edges.extend((50..73).map(|n| {
            edge(
                &format!("method/common/Модуль/Метод{n}"),
                CallEdgeProvenance::Resolved,
            )
        }));
        assert!(full_branch_matches(
            &first,
            &full,
            CallGraphDirection::Callers
        ));
        assert_eq!(
            required_full_branch_limit(&full, CallGraphDirection::Callers).unwrap(),
            None
        );
        full.callers.revision = Some(2);
        assert!(!full_branch_matches(
            &first,
            &full,
            CallGraphDirection::Callers
        ));
        full.callers.revision = Some(1);
        full.callers.complete = false;
        assert!(!full_branch_matches(
            &first,
            &full,
            CallGraphDirection::Callers
        ));
    }

    #[test]
    fn capped_graph_refetches_exact_total_before_publishing_a_branch() {
        let mut first = CallGraphSummary {
            callers: ready(
                73,
                (0..50)
                    .map(|n| {
                        edge(
                            &format!("method/common/Модуль/Метод{n}"),
                            CallEdgeProvenance::Resolved,
                        )
                    })
                    .collect(),
            ),
            callees: ready(0, Vec::new()),
            reason: None,
        };
        first.callers.complete = false;
        let mut full = first.clone();
        full.callers.complete = true;
        full.callers.edges.extend((50..73).map(|n| {
            edge(
                &format!("method/common/Модуль/Метод{n}"),
                CallEdgeProvenance::Resolved,
            )
        }));
        let mut requested = Vec::new();
        let (accepted, _): (CallGraphSummary, Option<()>) = complete_branch(
            (first.clone(), None),
            CallGraphDirection::Callers,
            |limit| {
                requested.push(limit);
                Ok::<_, ()>((full.clone(), None))
            },
        )
        .unwrap();
        assert_eq!(requested, [73]);
        let at = address("main:CommonModule.Модуль.Method.Проба.Caller");
        let collection = branch_collection(&at, CallGraphDirection::Callers, &accepted, |_| None);
        assert_eq!(collection["items"].as_array().unwrap().len(), 73);
        assert_eq!(
            collection["items"][72]["at"],
            "main:CommonModule.Модуль.Method.Метод72"
        );

        let mut changed = full.clone();
        changed.callers.revision = Some(2);
        assert!(matches!(
            complete_branch(
                (first.clone(), None::<()>),
                CallGraphDirection::Callers,
                |_| Ok::<_, ()>((changed, None))
            ),
            Err(CompleteBranchError::Changed)
        ));
        full.callers.complete = false;
        assert!(matches!(
            complete_branch(
                (first.clone(), None::<()>),
                CallGraphDirection::Callers,
                |_| { Ok::<_, ()>((full, None)) }
            ),
            Err(CompleteBranchError::Incomplete)
        ));

        let mut unavailable = first.clone();
        unavailable.callers = CallGraphResult {
            state: CallGraphState::Unavailable,
            total: None,
            edges: Vec::new(),
            revision: None,
            stale: None,
            complete: false,
        };
        let (not_ready, _): (CallGraphSummary, Option<()>) =
            complete_branch((first, None), CallGraphDirection::Callers, |_| {
                Ok::<_, ()>((unavailable, None))
            })
            .unwrap();
        assert_eq!(not_ready.callers.state, CallGraphState::Unavailable);
        assert!(not_ready.callers.edges.is_empty());
        let not_ready_answer = unready_branch_result(&at, CallGraphDirection::Callers, &not_ready);
        assert!(not_ready_answer.ok);
        assert_eq!(
            not_ready_answer.data.as_ref().unwrap()["props"]["callGraph"],
            "unavailable"
        );
        assert!(not_ready_answer
            .data
            .as_ref()
            .unwrap()
            .get("items")
            .is_none());
        assert!(not_ready_answer.page.is_none());
        assert!(not_ready_answer.cursor.is_none());
        assert!(not_ready_answer.rev.is_none());

        let mut indexing = not_ready;
        indexing.callers.state = CallGraphState::Indexing;
        let (still_indexing, _): (CallGraphSummary, Option<()>) = complete_branch(
            (
                CallGraphSummary {
                    callers: CallGraphResult {
                        complete: false,
                        ..ready(73, Vec::new())
                    },
                    callees: ready(0, Vec::new()),
                    reason: None,
                },
                None,
            ),
            CallGraphDirection::Callers,
            |_| Ok::<_, ()>((indexing, None)),
        )
        .unwrap();
        assert_eq!(still_indexing.callers.state, CallGraphState::Indexing);
        assert!(still_indexing.callers.edges.is_empty());
        let indexing_answer =
            unready_branch_result(&at, CallGraphDirection::Callers, &still_indexing);
        assert_eq!(
            indexing_answer.data.as_ref().unwrap()["props"]["callGraph"],
            "indexing"
        );
        assert!(indexing_answer.page.is_none());
    }

    /// Сводка кладётся в `props`, направления — в ветви со счётом.
    #[test]
    fn the_method_node_carries_the_count_and_names_the_state() {
        let at = address("main:CommonModule.Общий.Method.Утилита");
        let summary = CallGraphSummary {
            reason: None,
            callers: ready(
                2,
                vec![edge(
                    "method/object/Catalog/Валюты/ПриЗаписи",
                    CallEdgeProvenance::Resolved,
                )],
            ),
            callees: ready(0, Vec::new()),
        };
        let mut data = Map::new();
        data.insert("at".to_string(), json!(at.to_string()));
        extend_method_node(&mut data, &at, &summary);

        assert_eq!(data["props"]["callGraph"], "ready");
        assert_eq!(data["props"]["callers"], 2);
        // Ноль — доказанный ответ и печатается: «никто не зовёт» это факт.
        assert_eq!(data["props"]["callees"], 0);
        assert_eq!(
            data["branches"],
            json!([
                {"at": "main:CommonModule.Общий.Method.Утилита.Caller", "count": 2},
                {"at": "main:CommonModule.Общий.Method.Утилита.Callee", "count": 0}
            ])
        );
    }

    /// Индекс ещё строится: состояние названо, счёт не печатается, и ветвь не
    /// объявляется — иначе она пообещала бы страницу неизвестной длины.
    #[test]
    fn an_indexing_graph_names_itself_instead_of_printing_a_zero() {
        let at = address("main:CommonModule.Общий.Method.Утилита");
        let indexing = CallGraphResult {
            state: CallGraphState::Indexing,
            total: None,
            edges: Vec::new(),
            revision: None,
            stale: None,
            complete: false,
        };
        let summary = CallGraphSummary {
            reason: None,
            callers: indexing.clone(),
            callees: indexing,
        };
        let mut data = Map::new();
        extend_method_node(&mut data, &at, &summary);

        assert_eq!(data["props"]["callGraph"], "indexing");
        assert!(data["props"].get("callers").is_none());
        assert!(data.get("branches").is_none());
    }

    /// Одно готовое направление не делает узел готовым: читатель принял бы
    /// неполную картину за полную.
    #[test]
    fn one_unready_direction_decides_the_whole_state() {
        let at = address("main:CommonModule.Общий.Method.Утилита");
        let summary = CallGraphSummary {
            reason: None,
            callers: ready(1, Vec::new()),
            callees: CallGraphResult {
                state: CallGraphState::Unavailable,
                total: None,
                edges: Vec::new(),
                revision: None,
                stale: None,
                complete: false,
            },
        };
        let mut data = Map::new();
        extend_method_node(&mut data, &at, &summary);
        assert_eq!(data["props"]["callGraph"], "unavailable");
        // Счёт известного направления остаётся: он замерен, а не выдуман.
        assert_eq!(data["props"]["callers"], 1);
    }

    /// Элемент ветви показывает наружу и несёт происхождение ребра.
    ///
    /// Личность, названную файлом, без резолвера в адрес перевести нечем; она
    /// не выбрасывается и не отдаётся внутренним именем анализатора, а названа
    /// в `limits`.
    #[test]
    fn the_branch_points_outward_and_names_what_it_could_not_address() {
        let at = address("main:CommonModule.Общий.Method.Утилита.Caller");
        let owner = branch_owner(&at).expect("владелец ветви");
        assert_eq!(owner.to_string(), "main:CommonModule.Общий.Method.Утилита");
        assert_eq!(
            branch_direction(&at),
            Some(CallGraphDirection::Callers),
            "адрес ветви называет направление"
        );

        let summary = CallGraphSummary {
            reason: None,
            callers: ready(
                3,
                vec![
                    edge(
                        "method/object/Catalog/Валюты/ПриЗаписи",
                        CallEdgeProvenance::Resolved,
                    ),
                    edge(
                        "method/manager/Catalog/Валюты/Одноимённый",
                        CallEdgeProvenance::Inferred,
                    ),
                    edge(
                        "method/file/Catalogs/Валюты/Forms/Форма/Ext/Form/Module.bsl::ПриОткрытии",
                        CallEdgeProvenance::Resolved,
                    ),
                ],
            ),
            callees: ready(0, Vec::new()),
        };
        let page = branch_collection(&at, CallGraphDirection::Callers, &summary, |_| None);

        assert_eq!(page["kind"], "Caller");
        assert_eq!(page["props"]["callGraph"], "ready");
        assert_eq!(page["props"]["graphRevision"], 1);
        assert_eq!(
            page["items"],
            json!([
                {
                    "at": "main:Catalog.Валюты.Module.Object.Method.ПриЗаписи",
                    "kind": "Method",
                    "provenance": "resolved"
                },
                {
                    "at": "main:Catalog.Валюты.Module.Manager.Method.Одноимённый",
                    "kind": "Method",
                    "provenance": "inferred"
                }
            ])
        );
        assert_eq!(page["limits"][0]["kind"], "unaddressablePeer");
        assert_eq!(page["limits"][0]["count"], 1);
        assert!(page["limits"][0]["reason"]
            .as_str()
            .is_some_and(|reason| !reason.trim().is_empty()));

        // С резолвером тот же элемент получает адрес, и `limits` исчезает.
        let resolved = branch_collection(&at, CallGraphDirection::Callers, &summary, |path| {
            assert_eq!(path, "Catalogs/Валюты/Forms/Форма/Ext/Form/Module.bsl");
            QualifiedAddress::parse("main:Catalog.Валюты.Form.Форма.Module.Form").ok()
        });
        assert_eq!(resolved["items"].as_array().expect("страница").len(), 3);
        assert!(resolved.get("limits").is_none());
        assert_eq!(
            resolved["items"][2]["at"],
            "main:Catalog.Валюты.Form.Форма.Module.Form.Method.ПриОткрытии"
        );
    }

    /// Перевод адреса формы и команды в файл модуля и обратно замкнут: то, что
    /// раскладка размещает дескриптором, анализатор называет файлом модуля.
    #[test]
    fn form_and_command_modules_translate_between_placement_and_analyzer_file() {
        assert_eq!(
            module_file_for_placed("Form", "Catalogs/Валюты/Forms/Форма.xml").as_deref(),
            Some("Catalogs/Валюты/Forms/Форма/Ext/Form/Module.bsl")
        );
        assert_eq!(
            module_file_for_placed("Command", "Catalogs/Валюты/Commands/Обновить").as_deref(),
            Some("Catalogs/Валюты/Commands/Обновить/Ext/CommandModule.bsl")
        );
        assert_eq!(
            module_file_for_placed("Catalog", "Catalogs/Валюты.xml"),
            None
        );
        assert_eq!(
            placed_for_module_file("Catalogs/Валюты/Forms/Форма/Ext/Form/Module.bsl"),
            Some(("Catalogs/Валюты/Forms/Форма.xml".to_string(), "Form"))
        );
        assert_eq!(
            placed_for_module_file("Catalogs/Валюты/Commands/Обновить/Ext/CommandModule.bsl"),
            Some(("Catalogs/Валюты/Commands/Обновить".to_string(), "Command"))
        );
        assert_eq!(
            placed_for_module_file("Catalogs/Валюты/Ext/ObjectModule.bsl"),
            None
        );
    }

    /// Страница отвечает за своё направление: файловый сосед у вызываемых не
    /// заставляет страницу вызывающих просить раскладку.
    #[test]
    fn a_file_named_peer_is_seen_only_in_its_own_direction() {
        let summary = CallGraphSummary {
            callers: ready(
                1,
                vec![edge(
                    "method/object/Catalog/Валюты/ПриЗаписи",
                    CallEdgeProvenance::Resolved,
                )],
            ),
            callees: ready(
                1,
                vec![edge(
                    "method/file/Catalogs/Валюты/Forms/Форма/Ext/Form/Module.bsl::ПриОткрытии",
                    CallEdgeProvenance::Resolved,
                )],
            ),
            reason: None,
        };

        assert!(!summary.names_a_peer_by_file(CallGraphDirection::Callers));
        assert!(summary.names_a_peer_by_file(CallGraphDirection::Callees));
    }

    /// Адрес, не называющий ветвь графа, направления не даёт.
    #[test]
    fn only_a_nameless_branch_terminal_names_a_direction() {
        assert_eq!(
            branch_direction(&address("main:CommonModule.Общий.Method.Утилита")),
            None
        );
        assert_eq!(
            branch_direction(&address(
                "main:CommonModule.Общий.Method.Утилита.Caller.Что"
            )),
            None,
            "именованный лист ветвью не является"
        );
        assert!(
            branch_owner(&address("main:CommonModule")).is_none(),
            "у одного сегмента владельца ветви нет"
        );
    }
}
