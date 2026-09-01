---
name: object-locks
description: "Объектные блокировки 1С — пессимистическая и оптимистическая. Используй когда нужно защитить объект от конкурентного редактирования, разобрать «Запись была изменена или удалена другим пользователем», «объект заблокирован», выбрать между Заблокировать и ЗаблокироватьДанныеДляРедактирования, решить про идентификатор формы или обработать конфликт версий."
---

# Object Locks: Pessimistic And Optimistic

## MCP routing

- Preferred path: use MCP `unica` tools `unica.view {}`, `unica.code.search`, `unica.code.definition`, `unica.code.outline`, `unica.code.graph`, `unica.code.patch`, `unica.form.info`, `unica.code.diagnostics`, and `unica.runtime.execute`.
- По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.
- Use `unica.standards.search` and `unica.standards.explain` for the one development-standard here, 490, and for related ones 648 and 783. These are standards, not evidence of runtime behavior; confirm the wording before citing one.
- Use `unica.runtime.execute` only to preview typed arguments; reproducing a conflict requires separate runtime evidence and cannot be inferred from preview.
- Do not call internal analyzer, runtime, standards, or package adapters directly. They are hidden behind MCP `unica`.

## Scope boundary

This skill owns concurrency between **users editing the same object**. Concurrency between transactions contending for data — managed locks, responsible reading, transaction shape — belongs to `transactions-locks`. The two mechanisms have different lifetimes and different failure modes, and one is not a substitute for the other.

## References

- Read `../../references/platform/object-locks.md` for both lock kinds, lock lifetime, the form id rule, and how object locks interact with transactions.
- Read `../../references/platform/transactions-locks.md` when the same code path also reads data that drives a write.
- Read `../../references/platform/form-events.md` when the editing path runs through a non-standard form module.

## Core model

Two locks doing two different jobs:

- **Pessimistic** stops the object being *edited* elsewhere while someone holds it. Set around editing, released when the form closes or the session ends.
- **Optimistic** stops a *write* whose in-memory object is stale. A version check performed at write time, always on, not configurable per write.

Two facts decide most reviews:

- **The pessimistic lock is cooperative.** It does not prevent the object in the database from being modified or deleted — it only prevents another lock on the same data. The object is protected only because every modifying path locks first. One path that writes without locking makes the scheme decorative, which is why std490 requires taking the lock before modifying an existing object from code.
- **Optimistic locking guarantees only non-overwriting.** A stale write is refused, including one from another software object in the same session. Nothing is merged, and the user is not told what changed.

## Workflow

1. Name the concurrency you are actually facing: two users on one object → this skill; two transactions on shared data → `transactions-locks`.
2. Find every path that modifies the object with `unica.code.search` and `unica.code.graph`. A cooperative lock is only as good as the path that skips it.
3. Choose the shape before writing code: fail loudly so the user learns who holds the object, or skip and retry on the next run for background work (std490).
4. Decide the form id question. With a form id the lock follows the form's lifetime; without one it follows the session, the server call, or the transaction. Do not mix both for the same object — that combination raises.
5. Inspect a non-standard editing form with `unica.form.info` and reproduce the standard behaviour with the form lock and unlock methods.
6. Decide what the version conflict says to the user before it happens; the platform's own message names nothing useful.
7. Apply with `unica.code.patch`, verify statically with `unica.code.diagnostics`, and preview the intended runtime request with `unica.runtime.execute`; require separate two-session evidence before calling the conflict reproduced.

## Design rules

- Take the object lock before modifying an existing object from code (std490), through the object's `Заблокировать()` or the global `ЗаблокироватьДанныеДляРедактирования()`.
- Background and scheduled work catches the lock failure, logs a warning to the event log, skips the object, and picks it up next run. Blocking a user from a job is not an option.
- Object locking operations affect only other object locking operations — not data operations and not the transaction flow.
- A failed object lock inside a transaction does **not** require rolling that transaction back. It may be caught and the transaction continued; this is the qualifier on the general rollback reflex in `transactions-locks`.
- A lock taken inside a transaction without a form id is released when the transaction ends.
- A version conflict is reported to the user in application terms, not as a raw platform message.

## Review checklist

- Every path that modifies the object takes the lock, including the ones reached from jobs and integrations.
- The chosen shape matches the caller: loud for interactive, skip-and-log for background.
- Locking with and without a form id is not mixed for the same object.
- Locks taken from code are released, or their release is explained by the lifetime chosen.
- No transaction is rolled back merely because an object lock failed.
- The version-conflict path exists and says something actionable.
- A non-standard editing form sets and removes the lock the way the standard form extension would.

## Stop rules

- Do not treat a pessimistic lock as protection against writing.
- Do not add an object lock in place of a managed lock on a read that drives a write; that is `transactions-locks`.
- Do not leave a version conflict surfacing as a raw platform error.
- Do not claim a concurrency fix without reproducing the conflict on two sessions.

## Contract gaps

If public MCP `unica` cannot inspect the editing path, the form, or the runtime evidence needed to reproduce the conflict, report a Unica MCP contract gap with the missing operation.
