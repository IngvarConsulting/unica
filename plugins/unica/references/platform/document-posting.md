# Document Posting

Use this reference when the task is about what a document records, when it is
allowed to record it, and what it must read under lock while doing so.

The normative rules here are development standards, not platform evidence:
`#std450` write order, `#std603` posting requirements, `#std477` register
self-sufficiency, `#std661` locking balance reads, `#std663` and `#std664`
totals separation, and diagnostics `АПК:105`, `АПК:123`, `АПК:226`, `АПК:227`.
Confirm the current wording with `unica.standards.explain` before citing one.

## Should The Document Post At All

- Posting is how an event enters accounting, so most documents post
  (`Posting = Allow`). An unposted document is a draft: possibly incomplete, not
  subject to business-logic checks, and absent from accounting. A posted document
  is the final copy.
- Lifecycle stages are statuses on a posted document, not a reason to leave it
  unposted. Posting records the primary reflection of the event; statuses refine
  how it is reflected.
- Documents that only fix that an event happened in time — incoming
  correspondence, calls, meetings — do not post. Neither do documents whose
  posting technology diverges far from the platform's, even when they look
  posted to the user.
- When one user action must both register the event and reflect it, write the
  new document straight in posting mode. Do not solve this by turning posting
  off.

## Posting Model

- A document's movements are register record sets whose recorder is the
  document. Writing them is the posting result; cancelling posting removes them.
- Two object-module handlers own the behavior: `ОбработкаПроведения(Отказ,
  РежимПроведения)` and `ОбработкаУдаленияПроведения(Отказ)`. Both already run
  inside a write transaction opened by the platform.
- Declare the target registers in the document's `RegisterRecords` list before
  writing to `Движения.<Регистр>`. A register missing from that list is not a
  code error, it is a metadata error.
- `Posting` (`Allow | Deny`) decides whether the document posts at all.
  `RealTimePosting` (`Allow | Deny`) decides whether the real-time branch can
  ever be reached.
- `RegisterRecordsDeletion` (`AutoDeleteOnUnpost | AutoDeleteOff`) decides who
  deletes movements on unpost. With `AutoDeleteOff` the deletion handler must do
  it, and an empty deletion handler silently leaves stale movements.
- `RegisterRecordsWritingOnPost` (`WriteSelected | WriteAll`) decides whether the
  platform writes every declared register or only the sets marked with
  `Движения.<Регистр>.Записывать = Истина`.

## Real-Time And Regular Posting

- The mode is chosen by the platform, not by the caller: the document date is
  compared with the current session date. Same date means real-time, an earlier
  date means regular. The handler reads it from the `РежимПроведения` parameter
  and compares it with `РежимПроведенияДокумента.Оперативный`.
- Real-time posting decides an operation happening now in a multi-user database.
  Availability checks — negative balances, credit limits, reservation conflicts —
  belong here.
- Regular posting records a fact that is already complete or is certain. Balance
  checks there mostly produce false failures during backdated entry and initial
  data load; a wrong resulting state is analyzed separately, not blocked at post
  time.
- A configuration may forbid real-time posting entirely and order documents by
  its own algorithm. That is a design decision, and it changes what the handler
  is allowed to assume.

## Position On The Time Axis

- The document date carries one-second precision, so several documents can share
  it. A point in time — date plus reference — resolves the order inside a second
  and also orders register records subordinate to a recorder.
- `МоментВремени` exposes `Дата`, `Ссылка`, and `Сравнить()`. The order it
  produces for same-second documents follows references, does not match creation
  order, and is not visible or changeable by users. Never present it as
  "which document the user entered first".
- Each real-time posting produces a real-time timestamp. When the previous
  timestamp is equal to or greater than the current session time, the next one is
  one second later, so a burst of real-time postings advances document dates past
  the wall clock.

## Choosing The Registers To Post To

- Split by accounting subject, not by convenience. Quantity on hand, monetary
  cost, and sales analytics are three subjects, and a single document commonly
  writes all three in one handler.
- A balance accumulation register has receipt and expense directions and yields
  balance, turnover, and combined virtual tables. A turnover register yields
  only the turnover table.
- Dimension rule for a balance register: a value may be a dimension only when
  both receipt and expense are always meaningful for it. A value that exists on
  receipts but not on expenses — a vendor on a goods receipt, for example —
  belongs to a turnover register or to a record attribute, never to a balance
  dimension. Ignoring this produces balances that never net out.
- An information register posted by a document needs `Подчинение регистратору`
  and a write mode that matches how the document rewrites its own records.
- A register must be logically independent of its recorders (`#std477`). No logic
  or report over register data may reach a recorder field through a dot: that
  produces implicit joins, and in a distributed infobase the movements can
  migrate to a node where the recorder does not exist. `АПК:123` flags it.
- Parallelism follows the dimension set, because balances for one dimension
  combination live in one resource (`#std664`). Pick dimensions for the required
  balance granularity, and reach for totals separation only when the dimension
  set cannot give it.

## Posting Performance

- Read the tabular section with one query instead of walking rows and reaching
  reference attributes through a dot. Dotted access to a reference attribute
  inside a row loop is one database read per row.
- Split the sources deliberately: row-level referential data comes from the
  query, document-level data (date, warehouse, counterparty, responsible) comes
  from the document object already in memory.
- Share one `МенеджерВременныхТаблиц` across the posting queries so the tabular
  section is materialized once and reused by the balance query and the check
  query.
- Measure before and after on a document with a realistic tabular section. A
  posting handler that is fast on three rows tells nothing about five hundred.

## Writing Movements And Controlling Balances

The default is that you write nothing yourself. `#std450`: do not call
`Записать()` on register record sets inside the posting handler. The platform
writes them implicitly when the handler returns; writing them yourself invites
mutual locks between concurrently posting users. `АПК:105` flags the explicit
call. The one exception is register data needed by later algorithms that run
before the handler returns — and balance control is exactly that exception,
which is why the shape below is allowed to write explicitly.

Balance control needs a *locking* read: two users must not read the same balance
for the same period, account, and dimension values and then each decide
independently to write off against it. Ten on hand, one writes off eight, the
other six, and the balance is minus four.

`#std661` gives the order, and it is not "read then write":

1. Decide in advance which balances need control and when. Skip control where it
   cannot fail: a receipt document only increases balances, and re-posting
   cannot write off more than the first posting already did.
2. Early in the transaction, explicitly write the movements for the registers
   that need no control, with `БлокироватьДляИзменения = Ложь`, always in the
   same register order — alphabetical is a fine convention.
3. Confirm totals separation is on for every accumulation and accounting
   register being written.
4. Run the rest of the transaction logic.
5. At the very end, explicitly write the movements for the controlled registers
   with `БлокироватьДляИзменения = Истина`, which lowers deadlock risk.
6. Only then query those registers for negative balances over the relevant
   dimensions. Empty result commits; any negative balance aborts the
   transaction.

Two consequences worth stating out loud:

- An explicit managed lock (`ДЛЯ ИЗМЕНЕНИЯ` under the automatic mode) is
  normally unnecessary here — the write in step 5 already locked what the
  control query reads.
- Controlling balances in `ПередЗаписью` of the record set module is the common
  shape and the wrong one. The developer does not control the order in which the
  platform writes different registers, so a controlled register written early
  holds its lock while every other register is still being written. Move the
  check as close to the end of the transaction as possible.
- Totals separation buys parallelism for writes, not for control. A control
  query over a separated register effectively merges the split resources back
  into one, so parallelism drops to the unseparated level unless the check sits
  at the end of the transaction (`#std664`).

Everything else about this transaction — what makes the balance read responsible
in the first place, lock order, lock modes, and the transaction shape itself —
is owned by `transactions-locks.md`. This document adds only the ordering that is
specific to posting.

## Rights During Posting

- `PostInPrivilegedMode` and `UnpostInPrivilegedMode` are expected on documents
  that post. `АПК:226` and `АПК:227` raise their absence at `Обязательно`
  severity, tied to `#std689` p. 1.7.
- Posting reads and writes registers the user may have no direct rights to;
  privileged posting is what keeps rights configured on the document rather than
  spread across every register it touches.

## Editing Register Records Outside Posting

- When a form edits the movements of a document directly, the handler location
  follows the configuration: use the object-module handler when anything writes
  the object programmatically, and the form-module handler only when nothing
  does.
- Neither location prevents an independent write through a record set obtained
  from the register manager, so a rule enforced only in one handler is not
  enforced.

## Stop Rules

- Do not add a balance check to the regular branch without a named business
  reason and a stated effect on backdated entry.
- Do not write record sets explicitly inside the posting handler unless the
  handler itself needs that data afterwards. Explicit writes outside the
  `#std661` control shape are `АПК:105`.
- Do not turn posting off to make a document behave like a draft or to model a
  lifecycle stage. Use statuses on a posted document instead.
- Do not change `Posting`, `RealTimePosting`, `RegisterRecordsDeletion`, or the
  `RegisterRecords` list without checking the movements already recorded by
  existing documents.
- Do not claim a posting performance win without a before/after measurement on
  comparable data.
- If public MCP `unica` lacks the operation needed to inspect movements, lock
  state, or posting evidence, report a Unica MCP contract gap.
