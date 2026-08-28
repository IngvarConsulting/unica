---
id: DEC.2026-08-28.DAEMON-RECEIPT-LEDGER
status: planned
governs: product
realized: null
supersedes: []
superseded-by: null
establishes: []
changes: [CTR.WIRE.DAEMON-INVOCATION-PROTOCOL]
design: docs/design/2026-08-28-daemon-receipt-ledger-design.md
---

# Private daemon получает отдельный durable ReceiptLedger

**Решение.** До domain validation, workspace admission, preparation и execution
daemon после строгого разбора `SubmitInvocation` сохраняет в отдельном от TaskStore
ReceiptLedger exact `InvocationId`, `reservedTaskId` и закрытую request identity,
включая `CoreIdentity`. Durable lifecycle имеет direct ветвь `reserved` →
`direct-terminal-unacked` → compact acknowledged tombstone и Task ветвь при cutoff
в Unbound: `reserved` → `task-promised-unbound` → actor-bound handoff intent → exact
TaskStore create/readback → `task-bound` → terminal; cutoff/known-long после
`ActorBound`/`Begun` идёт через отдельный durable actor-bound handoff intent, не
возвращаясь в unbound. ReceiptLedger сохраняет lifecycle/result через actor binding
и handoff; только commit `TaskBound` передаёт sole ownership TaskStore.

Proven TaskStore Capacity до `TaskBound` не вытесняет Tasks и не повторяет create:
до `begun` ReceiptLedger terminalizes Task как `task_capacity`, после `begun`
сохраняет actual terminal либо `outcome_uncertain` в receipt-owned ветви.

Если Task terminalizes до commit `TaskBound`, ReceiptLedger хранит bounded canonical
terminal payload на весь Task TTL: repeated get/result читают его без ACK-compaction.
Result quota освобождается только после exact TaskStore readback и commit `TaskBound`, Direct ACK,
часового expiry unacked Direct либо expiry receipt-backed terminal по полному Task TTL.

Request-scope identity служит только pre-admission deduplication. До `begun` durable
`ActorBound` закрепляет actor-derived identity, используемую в TaskStore record.

Target private protocol v5 получает recovery и ACK messages и новую identity. На
baseline 2026-08-28 `origin/main` ещё не содержал daemon protocol; committed
feature-branch predecessor v3 и experimental v4 state не переиспользуются. Повторный
submit с тем же exact key только читает исходный lifecycle, mismatch отклоняется, а
disconnect или новый budget не меняют исходный cutoff/state. Handoff имеет durable
intent, exact idempotent Task create и reconciliation, но не называется физически
атомарной транзакцией.

Protocol-v5 startup открывает TaskStore inspect-only. ReceiptLedger-led
reconciliation классифицирует active records до их terminalization и до listener;
legacy eager terminalization до чтения receipt evidence запрещена.

Гарантия ограничена at-most-once началом preparation/execution в заявленном retention
horizon. Commit внешнего side effect и terminalization receipt не имеют общей
транзакции: `begun` без доказанного terminal после crash становится закрытым
`outcome_uncertain` и никогда не replay-ится. Exactly-once outcome/delivery не обещан.

Публичный V12, native поверхность 8, compatibility поверхность 11 и отсутствие
generic public idempotency/resume остаются без изменений.

**Почему.** In-memory receipt и TaskStore, появляющийся только при handoff, не дают
restart-stable ответа для потерянного direct receipt и positive-budget replay.

**Цена.** Появляются отдельный bounded store, protocol/CoreIdentity bump,
межхранилищный reconciliation и `outcome_uncertain` вместо автоматического повтора.
