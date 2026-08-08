# Transactions, Locks, Responsible Reading

This reference owns transaction shape, managed locks, and responsible reading.
Other documents that touch these subjects point here instead of restating them:
`document-posting.md` keeps only the balance-control order specific to posting,
`register-design.md` only the parallelism that follows from the dimension set,
and `db-performance.md` only the evidence side of a deadlock.

The normative rules here are development standards, not platform evidence:
`#std460` managed lock mode, `#std490` object locks, `#std648` responsible
reading, `#std659` excessive locks, `#std661` locking balance reads, `#std783`
transaction rules, and diagnostics `АПК:66`, `АПК:67`, `АПК:325`–`АПК:332`,
`АПК:415`, `АПК:478`, `АПК:521`, `АПК:1319`, `АПК:1320`, `АПК:1327`, `АПК:1328`,
`BSLLS:PairingBrokenTransaction`, `BSLLS:BeginTransactionBeforeTryCatch`,
`BSLLS:CommitTransactionOutsideTryCatch`,
`BSLLS:WrongUseOfRollbackTransactionMethod`, `v8cs:lock-out-of-try`,
`v8cs:ql-using-for-update`. Confirm the current wording with
`unica.standards.explain` before citing one.

## What Makes A Read Responsible

`#std648` defines it by consequence, not by mechanism. A read is responsible when
its result either changes data in the infobase, or drives a business decision
that leads to later changes.

A responsible read runs inside a transaction with managed locks set **before**
the read. The typical cases are reading during posting before writing movements,
reading for a consistent handoff to an external system, and bulk processing or
restructuring during an update.

The failure this prevents is not exotic. Read a counter, add one, write it back,
with nothing holding the range in between: two sessions read the same value and
both write the same increment, and one of them silently disappears. The same
shape hides behind any read-decide-write pair.

- Data that will be written or deleted needs an exclusive managed lock
  (`АПК:1327`); data that is read as part of the decision needs at least a shared
  one (`АПК:1328`).
- For reference objects, take object locks in addition before modifying the data
  (`#std490`). Object locks are a separate mechanism with their own lifetime and
  their own failure mode; `object-locks.md` owns them.

## Transaction Shape

`#std783` starts from a symptom: `В этой транзакции уже происходили ошибки`
comes from breaking these rules, and it is hard to reproduce and hard to debug.

Three facts drive everything else:

- Nested transactions are not supported.
- **An exception does not roll the transaction back.** It raises the internal
  "no successful completion" flag; the actual rollback happens when execution
  reaches `ЗафиксироватьТранзакцию` or `ОтменитьТранзакцию`. Every
  `НачатьТранзакцию` therefore needs both paired calls.
- Transactions can also be started by the platform itself.

The shape follows from those:

```bsl
НачатьТранзакцию();
Попытка
    // блокировка, чтение, запись
    ЗафиксироватьТранзакцию();
Исключение
    ОтменитьТранзакцию();
    // обработка исключения
КонецПопытки;
```

- Begin and finish in the same method. Splitting `НачатьТранзакцию` and its
  commit across functions is what makes the failure unreadable later
  (`BSLLS:PairingBrokenTransaction`, `АПК:325`–`АПК:327`).
- Nothing executable between `НачатьТранзакцию` and `Попытка` (`АПК:331`).
- The commit sits inside the `Попытка` (`АПК:329`), and nothing that can raise
  sits between it and `Исключение` (`АПК:330`).
- `ОтменитьТранзакцию` is the first statement of the `Исключение` block
  (`АПК:332`, `АПК:478`), and no branch leaves the block without finishing the
  transaction (`АПК:521`). This governs the transaction's own exception block. A
  nested `Попытка` around an object lock is a different thing: a failed object
  lock does not require rolling the transaction back — see `object-locks.md`.
- `Заблокировать()` belongs inside the `Попытка` (`АПК:1320`,
  `v8cs:lock-out-of-try`), and a lock object built but never locked is
  `АПК:1319`.
- Conditions guarding the transaction calls must match each other (`АПК:415`).

## Managed Locks

- The configuration runs in the `Управляемый` data lock control mode (`#std460`);
  its absence is `АПК:67`. Under managed mode `ДЛЯ ИЗМЕНЕНИЯ` belongs to the
  automatic mode and is `АПК:66` / `v8cs:ql-using-for-update`.
- A lock wait happens when two sessions try to capture the same resource, where a
  resource is an indivisible set of data that can only be locked whole
  (`#std659`). Design therefore asks two questions: which resources does each
  operation capture, and how finely is the data sliced.
- When looking for excess locking in the metadata structure, check sequences,
  accounting registers, and accumulation registers first (`#std659`).
- Fix one lock order across registers and objects, and keep it identical in every
  handler that touches them. A deadlock between two paths is a violated ordering
  contract, not a reason to add blind retries.

## Keeping The Transaction Short

- No user input and no network call while a transaction is open.
- Do not open explicit nested transaction control inside object write or posting
  handlers; they already run inside a write transaction opened by the platform.
- Move a lock as late and release it as early as the logic allows. The balance
  control case is worked out in `document-posting.md`, which follows `#std661`:
  write the uncontrolled registers first, the controlled ones last, and query for
  negative balances only after that write.

## Stop Rules

- Do not read data that drives a write without locking it first.
- Do not assume an exception rolled anything back.
- Do not split a transaction across methods.
- Do not resolve a deadlock by adding retries before the lock order is stated.
- Do not widen a lock range to make a wait go away; state which resource is
  contended first.
- If public MCP `unica` lacks the operation needed to inspect the code path,
  the lock state, or the runtime evidence, report a Unica MCP contract gap.
