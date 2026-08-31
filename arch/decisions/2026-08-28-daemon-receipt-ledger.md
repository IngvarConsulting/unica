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

**Решение.** До domain validation, workspace admission, preparation и execution daemon
после строгого разбора `SubmitInvocation` сохраняет в отдельном от TaskStore ReceiptLedger
exact `InvocationId`, `reservedTaskId` и закрытую request identity с `CoreIdentity`.
Durable lifecycle имеет direct ветвь `reserved` → `direct-terminal-unacked` → compact
acknowledged tombstone и Task ветвь при cutoff в Unbound: `reserved` →
`task-promised-unbound` → actor-bound handoff intent → exact TaskStore create/readback →
`task-bound` → terminal. Cutoff/known-long после `ActorBound`/`Begun` идёт через отдельный
durable actor-bound handoff intent. Ordinary nonterminal path передаёт sole ownership
TaskStore только через `TaskBound`; staged terminal после exact TaskStore terminal readback
одной ledger mutation переходит прямо в `TaskTerminalBound`, materializes link и удаляет
staged receipt payload без промежуточного `TaskBound`.

Proven Link Capacity до `TaskBound` не вытесняет Tasks и не вызывает TaskStore create;
staged/cancel winner сохраняется. Isolated v5 доказывает `taskStoreRecordCount <= materializedLinkCount + liveReservationCount <= 4096`, поэтому successful reservation уже резервирует TaskStore count slot.
Без winner до `begun` ReceiptLedger terminalizes `task_capacity`, после — сохраняет actual/`outcome_uncertain`; post-reservation `TaskStore::Capacity` является `TaskStoreCapacityInvariantViolation`, сохраняет intent/reservation и fail-stop-ит без terminal/fallback/release.

Если pre-`TaskBound` terminal остаётся receipt-owned, включая proven Link Capacity, ReceiptLedger
хранит bounded canonical payload на весь Task TTL. В staged path payload удаляется только
после terminal TaskStore readback и direct `TaskTerminalBound`; в ordinary path quota
освобождается после nonterminal readback и `TaskBound`. Другие release points — Direct ACK,
часовой expiry unacked Direct либо expiry receipt-backed terminal по полному Task TTL.

Request-scope identity служит только pre-admission deduplication. До `begun` durable
`ActorBound` закрепляет actor-derived identity, используемую в TaskStore record.

Target private protocol v5 получает recovery/ACK и новую identity; committed feature-branch
v3/experimental v4 state не переиспользуются. Exact repeat только читает lifecycle,
mismatch отклоняется, а disconnect/new budget не меняют cutoff/state.

Protocol-v5 startup открывает TaskStore inspect-only. ReceiptLedger-led
reconciliation классифицирует active records до их terminalization и до listener;
legacy eager mutation запрещена. Terminal-only retirement идёт
`TaskTerminalBound` → `TaskRetirementPending` → coordinator-authorized exact TaskStore
delete → proven ledger/link/index removal; live `TaskBound` expiry и lazy delete запрещены.
`TaskBound`/`TaskTerminalBound`/`TaskRetirementPending` — closed states одного sole bounded lifecycle-link record, не duplicate active-receipt records.

Гарантия ограничена at-most-once началом preparation/execution в заявленном retention
horizon. Commit внешнего side effect и terminalization receipt не имеют общей
транзакции: `begun` без доказанного terminal после crash становится закрытым
`outcome_uncertain` и никогда не replay-ится. Exactly-once outcome/delivery не обещан.

Публичный V12, native поверхность 8, compatibility поверхность 11 и отсутствие
generic public idempotency/resume остаются без изменений.

**Почему.** In-memory receipt и TaskStore, появляющийся только при handoff, не дают
restart-stable ответа для потерянного direct receipt и positive-budget replay.

**Цена.** Появляются отдельный bounded store, protocol/CoreIdentity bump, межхранилищный reconciliation и `outcome_uncertain` вместо автоматического повтора.
