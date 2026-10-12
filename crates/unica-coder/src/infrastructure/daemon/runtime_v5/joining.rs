//! Присоединение одинакового чтения к уже идущему заданию (#1376).
//!
//! Повтор чтения после потерянного ответа на submit — новый вызов с новыми
//! идентификаторами. Если такое же чтение уже стало заданием, демон не
//! запускает второй обход: новый вызов получает собственный `taskId`, который
//! в памяти демона указывает на общее задание. Каждый держатель общего
//! задания отменяет только свой интерес; сама работа отменяется, когда её
//! отменили все держатели.
//!
//! Реестр живёт только в памяти. Квитанций и записей хранилища заданий
//! присоединившийся вызов не создаёт, поэтому восстановление после аварии
//! его не видит: после перезапуска демона его `taskId` неизвестен.
//!
//! Модуль не делает ввода-вывода: решения о состоянии общего задания
//! принимает рантайм по его снимку, а здесь только учёт держателей.

use crate::application::invocation::InvocationResponseDeadline;
use crate::application::receipt_ledger::{
    receipt_key_digest, ReceiptKey, ReceiptKeyDigest, RequestIdentity, V5ToolIdentity,
};
use crate::domain::invocation::{InvocationId, TaskId};
use std::collections::{HashMap, HashSet};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// Сколько разных чтений одновременно ждут возможных присоединений. При
/// переполнении забывается самое старое: оно теряет только возможность
/// присоединения, а не работу.
pub(super) const MAX_JOINABLE_LEADERS: usize = 256;
/// Сколько присоединившихся вызовов демон помнит одновременно. Сверх этого
/// новый вызов исполняется сам, как до присоединения.
pub(super) const MAX_JOINED_HOLDERS: usize = 1024;

/// Сколько квитанций без задания демон помнит по их `taskId`. Это вызовы,
/// чей прямой ответ ещё не подтверждён: идущие и потерявшие ответ.
pub(super) const MAX_RECEIPT_HANDLES: usize = 4096;

/// Присоединяются только инструменты, не меняющие исходники, базу и
/// окружение. `apply` и `run` не присоединяются никогда, включая
/// предварительный просмотр: их ответ зависит от момента вызова.
pub(super) const fn tool_is_joinable(tool: V5ToolIdentity) -> bool {
    match tool {
        V5ToolIdentity::View
        | V5ToolIdentity::Resolve
        | V5ToolIdentity::Search
        | V5ToolIdentity::Check
        | V5ToolIdentity::Diff
        | V5ToolIdentity::Docs => true,
        V5ToolIdentity::Apply | V5ToolIdentity::Run => false,
    }
}

/// Точная идентичность одного держателя: под ней он видит общее задание.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct JoinHolder {
    pub(super) task_id: TaskId,
    pub(super) invocation_id: InvocationId,
    pub(super) receipt_key_digest: ReceiptKeyDigest,
}

impl JoinHolder {
    pub(super) fn of(key: &ReceiptKey) -> Self {
        Self {
            task_id: key.reserved_task_id(),
            invocation_id: key.invocation_id(),
            receipt_key_digest: receipt_key_digest(key),
        }
    }

    /// Вызов предъявил ровно тот ключ, под которым присоединился.
    pub(super) fn matches(&self, key: &ReceiptKey) -> bool {
        *self == Self::of(key)
    }
}

/// Чтение, к которому можно присоединиться.
#[derive(Debug, Clone)]
pub(super) struct JoinCandidate {
    pub(super) task_id: TaskId,
    /// Обработчик submit ведущего вызова ещё не ответил: квитанция ещё не
    /// стала заданием или завершилась прямым ответом.
    pub(super) in_flight: bool,
    /// Срок передачи ведущего вызова в задание. Когда он истёк, ведущий
    /// вызов как раз переходит в задание, и короткое ожидание окупается.
    pub(super) leader_deadline: InvocationResponseDeadline,
}

/// Вид присоединившегося вызова.
#[derive(Debug, Clone)]
pub(super) struct JoinAlias {
    pub(super) leader: TaskId,
    pub(super) holder: JoinHolder,
    /// Когда этот держатель отменил свой интерес.
    pub(super) cancelled_at_epoch_ms: Option<u64>,
}

/// Что делать с отменой одного держателя.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HolderCancel {
    /// Держатель отпущен; общую работу ещё ждут другие держатели.
    Released,
    /// Отменить работу этого задания: её больше никто не ждёт.
    CancelShared(TaskId),
}

struct Leader {
    task_id: TaskId,
    in_flight: bool,
    deadline: InvocationResponseDeadline,
    order: u64,
}

#[derive(Default)]
struct SharedWork {
    /// Когда держатель исходного вызова отменил свой интерес.
    leader_released_at_epoch_ms: Option<u64>,
    /// Общая работа уже отменена: к ней больше не присоединяются.
    cancel_issued: bool,
    joiners: Vec<TaskId>,
}

#[derive(Default)]
struct JoinState {
    joinable: HashMap<RequestIdentity, Leader>,
    next_order: u64,
    shared: HashMap<TaskId, SharedWork>,
    aliases: HashMap<TaskId, JoinAlias>,
    /// Отправки `apply` и `run`, которые ещё могут менять состояние.
    mutations: HashSet<TaskId>,
}

impl JoinState {
    fn forget_leader_task(&mut self, task_id: TaskId) {
        self.joinable.retain(|_, leader| leader.task_id != task_id);
    }
}

#[derive(Default)]
pub(super) struct JoinRegistry {
    state: Mutex<JoinState>,
    changed: Condvar,
}

impl JoinRegistry {
    fn lock(&self) -> MutexGuard<'_, JoinState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Чтение получило новую квитанцию: к нему могут присоединиться, пока
    /// оно не завершится и пока не пришла отправка `apply` или `run`.
    pub(super) fn register_leader(&self, key: &ReceiptKey, deadline: InvocationResponseDeadline) {
        if !tool_is_joinable(key.tool()) {
            return;
        }
        let mut state = self.lock();
        let order = state.next_order;
        state.next_order = state.next_order.wrapping_add(1);
        state.joinable.insert(
            key.request_identity(),
            Leader {
                task_id: key.reserved_task_id(),
                in_flight: true,
                deadline,
                order,
            },
        );
        if state.joinable.len() > MAX_JOINABLE_LEADERS {
            if let Some(oldest) = state
                .joinable
                .iter()
                .min_by_key(|(_, leader)| leader.order)
                .map(|(identity, _)| identity.clone())
            {
                state.joinable.remove(&oldest);
            }
        }
    }

    /// Обработчик submit ведущего вызова ответил: задание создано или вызов
    /// завершился прямым ответом.
    pub(super) fn leader_settled(&self, task_id: TaskId) {
        let mut state = self.lock();
        for leader in state.joinable.values_mut() {
            if leader.task_id == task_id {
                leader.in_flight = false;
            }
        }
        drop(state);
        self.changed.notify_all();
    }

    /// Отправка, меняющая состояние, закрывает присоединение ко всем ранее
    /// начатым чтениям: следующее чтение должно увидеть её результат.
    pub(super) fn close_joins(&self) {
        self.lock().joinable.clear();
    }

    /// Мутация получила квитанцию: пока она не завершилась, чтения не
    /// присоединяются и не становятся ведущими.
    pub(super) fn track_mutation(&self, task_id: TaskId) {
        let mut state = self.lock();
        state.joinable.clear();
        state.mutations.insert(task_id);
    }

    pub(super) fn tracked_mutations(&self) -> Vec<TaskId> {
        self.lock().mutations.iter().copied().collect()
    }

    pub(super) fn mutation_finished(&self, task_id: TaskId) {
        self.lock().mutations.remove(&task_id);
    }

    pub(super) fn candidate(&self, identity: &RequestIdentity) -> Option<JoinCandidate> {
        self.lock()
            .joinable
            .get(identity)
            .map(|leader| JoinCandidate {
                task_id: leader.task_id,
                in_flight: leader.in_flight,
                leader_deadline: leader.deadline.clone(),
            })
    }

    /// Ведущий вызов закончился или пропал: к нему больше не присоединяются.
    pub(super) fn forget_leader(&self, identity: &RequestIdentity, task_id: TaskId) {
        let mut state = self.lock();
        if state
            .joinable
            .get(identity)
            .is_some_and(|leader| leader.task_id == task_id)
        {
            state.joinable.remove(identity);
        }
    }

    /// Подождать изменения ведущих вызовов не дольше `timeout`.
    pub(super) fn wait_changed(&self, timeout: Duration) {
        let state = self.lock();
        let _ = self
            .changed
            .wait_timeout(state, timeout)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }

    pub(super) fn holders_full(&self) -> bool {
        self.lock().aliases.len() >= MAX_JOINED_HOLDERS
    }

    pub(super) fn shared_leaders(&self) -> Vec<TaskId> {
        self.lock().shared.keys().copied().collect()
    }

    /// Присоединить вызов к общему заданию. Отказ — если ведущий вызов уже
    /// не принимает присоединений (отменён, вытеснен, закрыт мутацией) или
    /// реестр полон; тогда вызов исполняется сам.
    pub(super) fn attach(
        &self,
        identity: &RequestIdentity,
        leader: TaskId,
        joiner: &ReceiptKey,
    ) -> Option<JoinAlias> {
        let mut state = self.lock();
        let holder = JoinHolder::of(joiner);
        if holder.task_id == leader
            || state.aliases.len() >= MAX_JOINED_HOLDERS
            || state.aliases.contains_key(&holder.task_id)
            || state.shared.contains_key(&holder.task_id)
            || !state
                .joinable
                .get(identity)
                .is_some_and(|candidate| candidate.task_id == leader)
            || state
                .shared
                .get(&leader)
                .is_some_and(|shared| shared.cancel_issued)
        {
            return None;
        }
        let alias = JoinAlias {
            leader,
            holder,
            cancelled_at_epoch_ms: None,
        };
        state
            .shared
            .entry(leader)
            .or_default()
            .joiners
            .push(alias.holder.task_id);
        state.aliases.insert(alias.holder.task_id, alias.clone());
        Some(alias)
    }

    pub(super) fn alias(&self, task_id: TaskId) -> Option<JoinAlias> {
        self.lock().aliases.get(&task_id).cloned()
    }

    /// Задание исходного вызова, к которому присоединились другие вызовы.
    pub(super) fn is_shared_leader(&self, task_id: TaskId) -> bool {
        self.lock().shared.contains_key(&task_id)
    }

    /// Когда держатель исходного вызова отпустил общее задание.
    pub(super) fn leader_released_at(&self, task_id: TaskId) -> Option<u64> {
        self.lock()
            .shared
            .get(&task_id)
            .and_then(|shared| shared.leader_released_at_epoch_ms)
    }

    /// Отмена одного держателя: исходного вызова или присоединившегося.
    ///
    /// Общая работа отменяется, только когда её отменили все держатели.
    /// Решение принимается под одним замком с присоединением, поэтому
    /// вызов, присоединившийся раньше решения, не теряет работу из-за чужой
    /// отмены, а присоединившийся позже не получает отменённую работу.
    pub(super) fn cancel_holder(&self, task_id: TaskId, epoch_ms: u64) -> HolderCancel {
        let mut state = self.lock();
        let leader = match state.aliases.get_mut(&task_id) {
            Some(alias) => {
                alias.cancelled_at_epoch_ms.get_or_insert(epoch_ms);
                alias.leader
            }
            None => {
                let Some(shared) = state.shared.get_mut(&task_id) else {
                    // Никто не присоединился: отмена — обычная отмена задания.
                    state.forget_leader_task(task_id);
                    return HolderCancel::CancelShared(task_id);
                };
                shared.leader_released_at_epoch_ms.get_or_insert(epoch_ms);
                task_id
            }
        };
        let JoinState {
            shared, aliases, ..
        } = &mut *state;
        let Some(work) = shared.get_mut(&leader) else {
            return HolderCancel::Released;
        };
        let everyone_released = work.leader_released_at_epoch_ms.is_some()
            && work.joiners.iter().all(|joiner| {
                aliases
                    .get(joiner)
                    .is_none_or(|alias| alias.cancelled_at_epoch_ms.is_some())
            });
        if !everyone_released || work.cancel_issued {
            return HolderCancel::Released;
        }
        work.cancel_issued = true;
        state.forget_leader_task(leader);
        HolderCancel::CancelShared(leader)
    }

    /// Общее задание удалено по сроку хранения или неизвестно после
    /// перезапуска: его держатели больше ничего не получат.
    pub(super) fn forget_shared(&self, leader: TaskId) {
        let mut state = self.lock();
        if let Some(work) = state.shared.remove(&leader) {
            for joiner in work.joiners {
                state.aliases.remove(&joiner);
            }
        }
        state.forget_leader_task(leader);
    }
}

/// Квитанции, ещё не ставшие заданием, по их зарезервированному `taskId`.
///
/// Ошибка потерянного ответа на submit называет `taskId`. Пока вызов ещё
/// не передан в задание или уже завершился прямым ответом, которого никто
/// не подтвердил, задания с этим идентификатором нет; по этой записи демон
/// всё равно находит квитанцию и отвечает её состоянием.
/// Ключ квитанции по её `taskId` и порядок записи, плюс следующий порядок.
type HandleTable = (HashMap<TaskId, (ReceiptKey, u64)>, u64);

#[derive(Default)]
pub(super) struct ReceiptHandles {
    state: Mutex<HandleTable>,
}

impl ReceiptHandles {
    fn lock(&self) -> MutexGuard<'_, HandleTable> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn insert(&self, key: &ReceiptKey) {
        let mut guard = self.lock();
        let (handles, next_order) = &mut *guard;
        let order = *next_order;
        *next_order = next_order.wrapping_add(1);
        handles.insert(key.reserved_task_id(), (key.clone(), order));
        if handles.len() > MAX_RECEIPT_HANDLES {
            if let Some(oldest) = handles
                .iter()
                .min_by_key(|(_, (_, order))| *order)
                .map(|(task_id, _)| *task_id)
            {
                handles.remove(&oldest);
            }
        }
    }

    pub(super) fn get(&self, task_id: TaskId) -> Option<ReceiptKey> {
        self.lock().0.get(&task_id).map(|(key, _)| key.clone())
    }

    pub(super) fn remove(&self, task_id: TaskId) {
        self.lock().0.remove(&task_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::invocation::normalized_arguments_hash;
    use crate::application::ports::TokioClock;
    use crate::application::receipt_ledger::CoreIdentityDigest;
    use std::sync::Arc;

    fn key(tool: V5ToolIdentity, query: &str) -> ReceiptKey {
        let arguments = serde_json::json!({"query": query})
            .as_object()
            .unwrap()
            .clone();
        ReceiptKey::new(
            InvocationId::new(),
            TaskId::new(),
            RequestIdentity::new(
                CoreIdentityDigest::from_sha256([7; 32]),
                tool,
                normalized_arguments_hash(&arguments),
                "11".repeat(32).parse().unwrap(),
            ),
        )
    }

    fn deadline() -> InvocationResponseDeadline {
        InvocationResponseDeadline::capture(Arc::new(TokioClock))
    }

    #[test]
    fn only_tools_that_change_nothing_are_joinable() {
        for tool in V5ToolIdentity::ALL {
            let mutating = matches!(tool, V5ToolIdentity::Apply | V5ToolIdentity::Run);
            assert_eq!(tool_is_joinable(tool), !mutating, "{tool:?}");
        }
    }

    #[test]
    fn a_mutating_leader_is_never_registered() {
        let registry = JoinRegistry::default();
        for tool in [V5ToolIdentity::Apply, V5ToolIdentity::Run] {
            let leader = key(tool, "same");
            registry.register_leader(&leader, deadline());
            assert!(registry.candidate(&leader.request_identity()).is_none());
        }
    }

    #[test]
    fn only_the_exact_request_identity_finds_its_leader() {
        let registry = JoinRegistry::default();
        let leader = key(V5ToolIdentity::Search, "same");
        registry.register_leader(&leader, deadline());

        let same = key(V5ToolIdentity::Search, "same");
        let other_arguments = key(V5ToolIdentity::Search, "other");
        let other_tool = key(V5ToolIdentity::View, "same");
        assert_eq!(
            registry
                .candidate(&same.request_identity())
                .map(|candidate| candidate.task_id),
            Some(leader.reserved_task_id())
        );
        assert!(registry
            .candidate(&other_arguments.request_identity())
            .is_none());
        assert!(registry.candidate(&other_tool.request_identity()).is_none());
    }

    #[test]
    fn a_mutation_closes_joins_to_reads_started_before_it() {
        let registry = JoinRegistry::default();
        let leader = key(V5ToolIdentity::Search, "same");
        registry.register_leader(&leader, deadline());
        registry.close_joins();
        let joiner = key(V5ToolIdentity::Search, "same");
        assert!(registry.candidate(&joiner.request_identity()).is_none());
        assert!(registry
            .attach(
                &joiner.request_identity(),
                leader.reserved_task_id(),
                &joiner
            )
            .is_none());
    }

    #[test]
    fn shared_work_is_cancelled_only_when_every_holder_cancelled() {
        let registry = JoinRegistry::default();
        let leader = key(V5ToolIdentity::Search, "same");
        let identity = leader.request_identity();
        let leader_task = leader.reserved_task_id();
        registry.register_leader(&leader, deadline());
        let first = key(V5ToolIdentity::Search, "same");
        let second = key(V5ToolIdentity::Search, "same");
        registry.attach(&identity, leader_task, &first).unwrap();
        registry.attach(&identity, leader_task, &second).unwrap();

        assert_eq!(
            registry.cancel_holder(first.reserved_task_id(), 10),
            HolderCancel::Released
        );
        assert_eq!(
            registry.cancel_holder(leader_task, 11),
            HolderCancel::Released
        );
        assert_eq!(registry.leader_released_at(leader_task), Some(11));
        assert_eq!(
            registry.cancel_holder(second.reserved_task_id(), 12),
            HolderCancel::CancelShared(leader_task)
        );
        // Cancelled shared work takes no new holders, and a repeated cancel
        // does not cancel it twice.
        let late = key(V5ToolIdentity::Search, "same");
        assert!(registry.attach(&identity, leader_task, &late).is_none());
        assert_eq!(
            registry.cancel_holder(second.reserved_task_id(), 13),
            HolderCancel::Released
        );
        assert_eq!(
            registry
                .alias(first.reserved_task_id())
                .unwrap()
                .cancelled_at_epoch_ms,
            Some(10)
        );
    }

    #[test]
    fn a_leader_without_holders_cancels_as_before_and_stops_taking_joins() {
        let registry = JoinRegistry::default();
        let leader = key(V5ToolIdentity::Check, "same");
        let identity = leader.request_identity();
        registry.register_leader(&leader, deadline());
        assert_eq!(
            registry.cancel_holder(leader.reserved_task_id(), 5),
            HolderCancel::CancelShared(leader.reserved_task_id())
        );
        let joiner = key(V5ToolIdentity::Check, "same");
        assert!(registry
            .attach(&identity, leader.reserved_task_id(), &joiner)
            .is_none());
    }

    #[test]
    fn retirement_forgets_every_holder_of_the_shared_task() {
        let registry = JoinRegistry::default();
        let leader = key(V5ToolIdentity::Docs, "same");
        let identity = leader.request_identity();
        registry.register_leader(&leader, deadline());
        let joiner = key(V5ToolIdentity::Docs, "same");
        registry
            .attach(&identity, leader.reserved_task_id(), &joiner)
            .unwrap();
        registry.forget_shared(leader.reserved_task_id());
        assert!(registry.alias(joiner.reserved_task_id()).is_none());
        assert!(!registry.is_shared_leader(leader.reserved_task_id()));
        assert!(registry.candidate(&identity).is_none());
    }

    #[test]
    fn the_joinable_index_and_the_holders_stay_bounded() {
        let registry = JoinRegistry::default();
        let first = key(V5ToolIdentity::View, "first");
        registry.register_leader(&first, deadline());
        for index in 0..MAX_JOINABLE_LEADERS {
            registry.register_leader(&key(V5ToolIdentity::View, &index.to_string()), deadline());
        }
        assert!(
            registry.candidate(&first.request_identity()).is_none(),
            "the oldest leader is forgotten first"
        );

        let leader = key(V5ToolIdentity::View, "shared");
        let identity = leader.request_identity();
        registry.register_leader(&leader, deadline());
        for _ in 0..MAX_JOINED_HOLDERS {
            let joiner = key(V5ToolIdentity::View, "shared");
            registry
                .attach(&identity, leader.reserved_task_id(), &joiner)
                .unwrap();
        }
        assert!(registry.holders_full());
        let overflow = key(V5ToolIdentity::View, "shared");
        assert!(registry
            .attach(&identity, leader.reserved_task_id(), &overflow)
            .is_none());
    }
}
