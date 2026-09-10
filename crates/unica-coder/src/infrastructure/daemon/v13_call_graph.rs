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
use serde_json::{json, Map, Value};

/// Название секции, которой вызывающий просит сводку графа.
pub(super) const CALL_GRAPH_SECTION: &str = "callGraph";

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

    fn result(&self, direction: CallGraphDirection) -> &CallGraphResult {
        match direction {
            CallGraphDirection::Callers => &self.callers,
            CallGraphDirection::Callees => &self.callees,
        }
    }
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
) -> Result<CallGraphSummary, String> {
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
    Ok(CallGraphSummary {
        callers,
        callees,
        reason,
    })
}

fn unavailable() -> CallGraphResult {
    CallGraphResult {
        state: CallGraphState::Unavailable,
        total: None,
        edges: Vec::new(),
        revision: None,
        stale: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        branch_collection, branch_direction, branch_owner, extend_method_node, CallGraphSummary,
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
        }
    }

    fn edge(id: &str, provenance: CallEdgeProvenance) -> CallGraphEdge {
        CallGraphEdge {
            id: id.to_string(),
            provenance,
        }
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
