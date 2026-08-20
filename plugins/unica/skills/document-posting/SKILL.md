---
name: document-posting
description: "Проведение документов 1С. Используй когда нужно спроектировать или починить движения документа, выбрать состав регистров, разделить оперативное и неоперативное проведение, поставить контроль остатков, управляемые блокировки, отмену проведения и перепроведение."
---

# Document Posting

## MCP routing

- Preferred path: use MCP `unica` tools `unica.project.map`, `unica.meta.info`, `unica.meta.edit`, `unica.code.search`, `unica.code.definition`, `unica.code.outline`, `unica.code.graph`, `unica.code.patch`, `unica.code.diagnostics`, and `unica.runtime.execute`.
- По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.
- Use `unica.standards.search` and `unica.standards.explain` for a development-standard about posting: 450, 477, 603, 661, 663, 664, and diagnostics АПК:105, АПК:123, АПК:226, АПК:227. These are standards, not evidence of runtime behavior; confirm the wording before citing one.
- Use `unica.role.info` when posting runs in privileged mode or depends on rights or RLS on the target registers.
- Do not call internal analyzer, runtime, standards, or package adapters directly. They are hidden behind MCP `unica`.

## References

- Read `../../references/platform/method-naming.md` before naming any new or renamed BSL method; preserve platform handlers and existing public APIs.
- Read `../../references/platform/document-posting.md` for the posting model, real-time versus regular branches, register choice, lock shape, and stop rules.
- Read `../../references/platform/transactions-locks.md` for the transaction shape, managed lock modes, and what makes the balance read responsible; this skill adds only the ordering specific to posting.
- Read `../../references/platform/db-performance.md` when posting is slow, contends on locks, or produces deadlocks.

## Core model

Posting answers four separate questions, and mixing them is the usual defect:

- **Whether the document posts at all**: most documents do (`Posting = Allow`). An unposted document is a draft. Lifecycle stages are statuses on a posted document, never a reason to disable posting (std603).
- **What is recorded**: the register set declared in the document's `RegisterRecords` and written through `Движения.<Регистр>`.
- **When it may be recorded**: the platform picks the branch by comparing the document date with the session date, and `ОбработкаПроведения` reads it as `РежимПроведения = РежимПроведенияДокумента.Оперативный`. The mode is one input to deciding which checks run, not the whole rule.
- **What must be locked**: only the balances that can actually go negative. The write itself takes the lock — `Движения.<Регистр>.БлокироватьДляИзменения = Истина` — and the control query runs after that write, not before (std661).

By default the handler writes nothing itself: the platform writes the record sets implicitly when it returns, and an explicit `Записать()` inside the handler is `АПК:105` (std450). Balance control is the sanctioned exception, and it has a prescribed order.

## Workflow

1. Name the accounting subjects the document affects before touching code. Each subject is a register, not a column on an existing one.
2. Inspect the document with `unica.meta.info`: declared registers, `Posting`, `RealTimePosting`, `RegisterRecordsDeletion`, `RegisterRecordsWritingOnPost`, and privileged-mode flags. Inspect every target register the same way for dimensions, resources, and periodicity.
3. Read the current handlers with `unica.code.definition` and `unica.code.outline` before reading full modules; use `unica.code.graph` when posting calls shared procedures.
4. Decide the branch split: what the real-time branch checks, what the regular branch skips, and what both always write.
5. Decide which balances need control at all. Skip control where it cannot fail: a receipt document only increases balances, and re-posting cannot write off more than the first posting already did.
6. Fix the write order for the std661 shape: uncontrolled registers early with `БлокироватьДляИзменения = Ложь` in a stable register order, controlled registers last with `БлокироватьДляИзменения = Истина`, negative-balance queries after that write. Keep the same order in every handler that shares those registers.
7. Change metadata with `unica.meta.edit` and module code with `unica.code.patch`, one verifiable step at a time.
8. Verify statically with `unica.code.diagnostics`; use `unica.runtime.execute` to preview typed syntax/test arguments and, with `dryRun: false`, to run them, and require separate evidence for posting, unposting, and runtime behavior with a realistic tabular section.

## Design rules

- A balance register dimension must be meaningful for both receipt and expense. A value present only on receipts belongs to a turnover register or a record attribute.
- Parallelism follows the dimension set, because balances for one dimension combination live in one resource. Reach for totals separation only when the dimension set cannot give the needed granularity, and remember it buys nothing for control queries that still sit early in the transaction (std664).
- No logic or report over register data may reach a recorder field through a dot: implicit joins, and in a distributed infobase the recorder may not exist on the node (std477, АПК:123).
- Availability checks are a business decision, not a branch rule. The real-time mode is the usual place for them; a check in the regular branch must be justified against backdated entry and initial data load.
- Read row-level referential data with one query; take document-level data from the object. Dotted access to a reference attribute inside a row loop is one database read per row.
- Control balances after writing the controlled registers, at the end of the transaction — not in `ПередЗаписью` of the record set module, where the developer does not control the platform's write order and the lock is held while every other register is written.
- An explicit `ДЛЯ ИЗМЕНЕНИЯ` lock is normally unnecessary in the control step: the write already locked what the control query reads.
- Lock order, lock modes, and the transaction shape around all of this are owned by `transactions-locks`; apply them here rather than restating them.

## Review checklist

- Every register written in code is declared in the document metadata.
- The handler does not call `Записать()` except for the balance-control shape (std450, АПК:105).
- Controlled registers are written last and their negative-balance query runs after that write.
- Uncontrolled registers are written in a stable order with `БлокироватьДляИзменения = Ложь`.
- Totals separation is on for the accumulation and accounting registers being written.
- `PostInPrivilegedMode` and `UnpostInPrivilegedMode` are set on a posting document (АПК:226, АПК:227).
- Unposting removes exactly the movements posting created, whether the platform or the deletion handler does it.
- The real-time branch is reachable and the regular branch is correct on its own.
- Refusal sets `Отказ` and reports the failing rows, not only a generic message.
- Re-posting an already posted document is idempotent for the resulting movements.
- The `transactions-locks` checklist has been applied to the handler as a whole.

## Stop rules

- Do not turn posting off to model a draft or a lifecycle stage. Use statuses on a posted document (std603).
- Do not write record sets explicitly inside the handler unless the handler needs that data afterwards.
- Do not change `Posting`, `RealTimePosting`, `RegisterRecordsDeletion`, or the declared register list without checking movements already recorded by existing documents.
- Do not claim a posting performance win without a before/after measurement on comparable data.

## Contract gaps

If public MCP `unica` cannot inspect the document, its registers, movements, or the runtime evidence needed for the task, report a Unica MCP contract gap with the missing operation.
