---
id: DEC.2026-08-24.EXACT-SHARED-DELIVERY-SLICE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/daemon/server.rs::daemon_shared_delivery_releases_request_admission_before_wait_and_shares_across_worktrees
supersedes: []
superseded-by: null
establishes: [INV.APP.EXACT-SHARED-WORK, CTR.APP.EXACT-SHARED-DELIVERY]
design: docs/design/2026-08-23-v0-13-execution-surface-design.md
---

# Daemon объединяет холодную поставку только по точной identity

**Решение.** Daemon владеет одним `SharedWork` поставки. Его точный ключ —
`artifact + version + target + sha256 + delivery form`; workspace и worktree в
него не входят, поэтому два worktree могут присоединиться к одному неизменяемому
артефакту. Другой SHA-256 или любая другая часть ключа создаёт отдельную работу.
Producer возвращает только `ArtifactReady` с той же identity и абсолютным
install root либо закрыто классифицированный отказ; неподходящий вид ключа,
подмена identity и относительный root отклоняются на границе поставки.

Присоединение не ждёт producer под admission: ожидание начинается после durable
Task handoff. Delivery process-owned: отмена одного follower освобождает его
lease, но не отменяет producer. Generic owner-bound режим отменяет producer при
потере последнего владельца; attach, cancellation и retirement одного точного
ключа линеаризованы без перекрывающихся producer.

V12 compatibility adapter использует ту же exact реализацию в своём
process-local `DeliveryDesk` и сохраняет прежний `working`/progress result до
Task 22; он не получает daemon-owned `Arc`. Canonical V13 capability уже
принадлежит `ActorBoundExecution`, но production subject handler пока dormant;
этот срез не утверждает durable delivery-progress projection. Варианты ключей
Index, Provider и Runtime только зарезервированы и не маршрутизируются до Task
11. Проверку staging, SHA-256 и atomic ready по-прежнему владеют
`INV.PKG.VERIFIED-ATOMIC-INSTALL` и `INV.PKG.CORRUPT-ARCHIVE-NOT-READY`.

**Почему.** Неизменяемая поставка может безопасно разделяться между worktree
только по полному manifest identity, а второй coalescer или ожидание до handoff
создали бы duplicate download либо заняли request admission.

**Цена.** До Task 22 V12 синхронно наблюдает тот же producer через старую форму,
а скрытый V13 имеет capability без production subject consumer.
