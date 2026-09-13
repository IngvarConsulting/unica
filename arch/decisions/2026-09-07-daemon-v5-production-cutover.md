---
id: DEC.2026-09-07.DAEMON-V5-PRODUCTION-CUTOVER
status: active
governs: product
realized: crates/unica-coder/src/interfaces/daemon_router.rs::live_daemon_executes_once_and_compacts_the_acknowledged_receipt_to_a_tombstone
supersedes: [DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE, DEC.2026-08-24.NATIVE-TASK-PROJECTION-SLICE]
superseded-by: null
establishes: [INV.APP.DAEMON-INVOCATION-OWNERSHIP, INV.APP.DAEMON-INVOCATION-HANDOFF, INV.APP.DAEMON-TASK-PERSISTENCE, INV.APP.DAEMON-TASK-RECOVERY, INV.APP.DAEMON-ACTOR-AUTHORITY, INV.APP.DAEMON-TERMINAL-RECONCILIATION, INV.APP.DAEMON-STORE-FAIL-STOP, INV.APP.EXACT-LONG-WORK-OWNERSHIP, INV.WIRE.NATIVE-TASK-CAPABILITY, INV.WIRE.SDK-TRANSPORT, CTR.WIRE.NATIVE-TASK-PROJECTION, CTR.WIRE.DAEMON-INVOCATION-PROTOCOL, CTR.APP.DAEMON-LONG-WORK-CAPABILITIES]
changes: [CTR.WIRE.DAEMON-INVOCATION-PROTOCOL]
design: docs/design/2026-09-07-daemon-v5-production-cutover-design.md
---

# Production daemon говорит только по протоколу v5

**Решение.** Производственный stdio frontend поднимает пользовательский daemon
ровно с identity `unica-daemon-jsonl-5` и ведёт canonical v0.13 через
ReceiptLedger: каждый вызов получает свежие `invocationId` и `reservedTaskId`,
daemon резервирует квитанцию до валидации, Direct-терминал подтверждается
frontend-ом только после построения окончательного `CallToolResult` или
закрытой `ErrorData`, потерянный ответ восстанавливается по тому же ключу без
повторной отправки, а работа дольше cutoff становится durable Task с заранее
известным идентификатором. В реестре остаётся одна wire identity: контракт
протокола переходит в версию 5, native-проекция Task, compatibility-инструменты
и capability долгой работы переезжают на снимки v5 с закрытыми причинами отказа.

Обязательства предшественников переносятся без ослабления: один вызов — одно
исполнение в daemon; седьмая секунда делит direct и Task; durable record несёт
только закрытые identity, digest и причину; после restart ничего не
переисполняется; terminal подтверждается без повторного domain execution;
store bounded, fail-stop завершается только смертью процесса; actor authority
точного WorkspaceActor; native Task только для capability `2026-07-28`.

Identity v3 остаётся явным seam для тестов рантайма v3 и снимается вместе с
его кодом отдельным шагом; удаление ничего в реестре не меняет.

**Почему.** Ledger v5 доказан чёрноящичным контрактом на трёх ОС, а
production всё ещё ходил по v3 без квитанций: потеря ответа означала
неопределённость, которую нельзя было ни восстановить, ни доказать.
**Цена.** Второй вызов nextest с признаком в очереди и ночной ярус нагрузочных
тестов; отмена со стороны хоста через `CancelInvocation` — отдельный шаг.
