---
name: transactions-locks
description: "Транзакции, блокировки и ответственное чтение 1С. Используй когда нужно написать или отревьюить транзакцию, поставить управляемые блокировки, решить надо ли блокировать чтение перед записью, разобрать ошибку «В этой транзакции уже происходили ошибки», ожидание блокировки, deadlock или избыточные блокировки."
---

# Transactions, Locks, Responsible Reading

## MCP routing

- Preferred path: use MCP `unica` tools `unica.project.map`, `unica.code.search`, `unica.code.definition`, `unica.code.outline`, `unica.code.graph`, `unica.code.patch`, `unica.code.diagnostics`, `unica.meta.info`, and `unica.runtime.execute`.
- Use `unica.standards.search` and `unica.standards.explain` for a development-standard about transactions and locks: 460, 490, 648, 659, 661, 783, and diagnostics АПК:66, АПК:67, АПК:329, АПК:1319, АПК:1327, АПК:1328, BSLLS:PairingBrokenTransaction, v8cs:lock-out-of-try. These are standards, not evidence of runtime behavior; confirm the wording before citing one.
- Do not call internal analyzer, runtime, standards, or package adapters directly. They are hidden behind MCP `unica`.

## Scope boundary

This skill owns transaction shape, managed locks, and responsible reading wherever they appear. Skills that touch the subject defer here: `document-posting` keeps only the balance-control order, `register-design` only the parallelism that follows from the dimension set, `background-jobs` only job restartability, `db-performance` only the evidence side of a deadlock.

Concurrency between users editing the same object is a different mechanism and belongs to `object-locks`. A failed object lock, in particular, does not require rolling this transaction back.

## References

- Read `../../references/platform/transactions-locks.md` for the responsible-read definition, the transaction shape, managed locks, and lock order.
- Read `../../references/platform/document-posting.md` when the transaction is a document posting.
- Read `../../references/platform/db-performance.md` and `../../references/platform/runtime-diagnostics.md` when a lock wait or deadlock has to be diagnosed from evidence.

## Core model

- **A read is responsible when its result changes data, or drives a decision that leads to changes** (std648). Then it runs inside a transaction with managed locks set *before* the read — not after, and not "the write will lock it anyway".
- **An exception does not roll the transaction back** (std783). It raises the internal no-successful-completion flag; the rollback happens when execution reaches `ЗафиксироватьТранзакцию` or `ОтменитьТранзакцию`. This is why every `НачатьТранзакцию` needs both paired calls.
- **Nested transactions are not supported**, and the platform starts transactions of its own.
- **A lock wait is two sessions capturing the same resource**, where a resource is an indivisible set of data locked only as a whole (std659). Contention is a question about how finely the data is sliced, not about lock syntax.

The read-decide-write pair is the shape to recognise: read a value, compute from it, write it back. Without a lock held across all three, two sessions read the same value and one of the writes disappears.

## Workflow

1. Classify every read on the path: does its result change data or drive a decision that will? If yes, it is responsible and needs a lock.
2. Name the resources the operation captures and in what order, before writing any lock code.
3. Locate the existing transaction boundaries with `unica.code.outline` and `unica.code.graph` — a transaction begun in one method and finished in another is the defect, not a style issue.
4. Write the shape whole: `НачатьТранзакцию()`, then `Попытка` with the lock, the read, the write and the commit, then `Исключение` with `ОтменитьТранзакцию()` first.
5. Apply with `unica.code.patch`, one verifiable step at a time.
6. Verify with `unica.code.diagnostics` — the transaction-scheme diagnostics are exactly what this skill's rules encode — and `unica.runtime.execute` for syntax and tests.
7. For diagnosis, build the timeline from the runtime evidence and identify the contended resource before proposing any change.

## Design rules

- Data to be written or deleted takes an exclusive managed lock; data read as part of the decision takes at least a shared one (АПК:1327, АПК:1328). For reference objects, add object locks before modifying (std490).
- Begin and finish a transaction in the same method (BSLLS:PairingBrokenTransaction, АПК:325–АПК:327).
- Nothing executable between `НачатьТранзакцию` and `Попытка` (АПК:331); the commit inside the `Попытка` (АПК:329); nothing that can raise between the commit and `Исключение` (АПК:330); `ОтменитьТранзакцию` first in the exception block (АПК:332, АПК:478); no branch leaving without finishing (АПК:521).
- `Заблокировать()` goes inside the `Попытка` (АПК:1320, v8cs:lock-out-of-try); a lock object built and never locked is АПК:1319.
- The configuration runs in the `Управляемый` lock mode (std460, АПК:67); `ДЛЯ ИЗМЕНЕНИЯ` belongs to the automatic mode and is АПК:66.
- Keep one lock order across registers and objects. A deadlock is a violated ordering contract, not a reason for blind retries.
- Keep the transaction short: no user input, no network call while it is open, and no explicit nested transaction control inside object write or posting handlers.
- When looking for excess locking in metadata, check sequences, accounting registers, and accumulation registers first (std659).

## Review checklist

- Every read whose result drives a write is inside a transaction with the lock set before it.
- Each `НачатьТранзакцию` has both a commit and a rollback, in the same method.
- The exception block starts with `ОтменитьТранзакцию`, and no code after the commit can raise.
- `Заблокировать()` is called, and called inside the `Попытка`.
- Lock modes match intent: exclusive on what is written, shared on what is read.
- The lock order is stated and identical everywhere those resources are touched.
- No user prompt, network call, or nested transaction inside the transaction.

## Stop rules

- Do not assume an exception rolled anything back.
- Do not rely on the write to take the lock that the read needed.
- Do not widen a lock range to make a wait disappear before naming the contended resource.
- Do not add retries to a deadlock before the lock order is stated.

## Contract gaps

If public MCP `unica` cannot inspect the code path, the lock state, or the runtime evidence needed for the task, report a Unica MCP contract gap with the missing operation.
