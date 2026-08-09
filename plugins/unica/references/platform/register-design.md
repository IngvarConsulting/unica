# Register Design

Use this reference when deciding what a register stores, how it is keyed, and
how it will be read and written at volume.

The normative rules here are development standards, not platform evidence:
`#std447` record manager use, `#std477` register self-sufficiency, `#std657` and
`#std733` virtual table access, `#std664` and `#std663` totals separation,
`#std661` locking balance reads, `#std708` totals for periodic information
registers, `#std792` bulk writes, and diagnostics `АПК:123`, `АПК:229`,
`BSLLS:DenyIncompleteValues`, `BSLLS:VirtualTableCallWithoutParameters`,
`v8cs:ql-virtual-table-filters`, `v8cs:register-resource-precision`. Confirm the
current wording with `unica.standards.explain` before citing one.

## Choosing The Register Class

- **Information register** stores state keyed by dimensions: prices, rates,
  settings, links. Periodic when the value has a history worth querying by date.
- **Accumulation register, `RegisterType = Balances`** stores quantities that
  come in and go out. It yields balance, turnover, and combined virtual tables.
- **Accumulation register, `RegisterType = Turnovers`** accumulates turnovers
  only. Balance is meaningless for it, and it yields one virtual table.
- **Accounting register** records double entry against a chart of accounts, with
  subconto supplied by the account. Amounts come from correspondence rather than
  from resources declared in `ChildObjects`.
- **Calculation register** records accruals with an action period and optionally
  a base period, with displacement and recalculations handled by the platform.

Split by accounting subject, not by convenience. Quantity on hand, monetary
cost, and sales analytics are three subjects and three registers, even when one
document writes all of them.

## Dimensions, Resources, Attributes

- A dimension is a key. A resource is a value that can be aggregated. An
  attribute is accompanying information that is never a key and never summed.
  Putting a value in the wrong slot is the defect that is most expensive to
  correct later, because it changes the stored table.
- For a `Balances` register a dimension is valid only when both receipt and
  expense are always meaningful for it. A value that exists on receipts but not
  on expenses — a vendor on a goods receipt — belongs to a turnover register or
  to an attribute. Ignoring this produces balances that never net out.
- Parallelism follows the dimension set: balances for one dimension combination
  live in one resource (`#std664`). Choose dimensions for the balance
  granularity the application actually needs.
- Set `DenyIncompleteValues` on dimensions that must always carry a value. A
  record with an empty dimension is meaningless however it was written, and the
  platform check removes the need to guard it in interactive and programmatic
  paths alike. `BSLLS:DenyIncompleteValues` covers this; it is off by default and
  can fire on dimensions that are genuinely optional.
- `Master` on a dimension couples record lifetime to that value: deleting the
  master value deletes the records. That is a modelling decision, not a flag to
  set by habit.
- Resource length and precision must fit the values the application will
  actually accumulate (`v8cs:register-resource-precision`).
- A register must be logically independent of its recorders (`#std477`). No logic
  or report over register data may reach a recorder field through a dot: that
  produces implicit joins, and in a distributed infobase the movements can
  migrate to a node where the recorder does not exist. `АПК:123` flags it.

## Information Register Periodicity And Write Mode

- `WriteMode = Independent` means records are written directly.
  `RecorderSubordinate` means a document owns them and posting writes them.
- `InformationRegisterPeriodicity = RecorderPosition` is the non-periodic form
  subordinate to a recorder: the period is the recorder's position.
- `EnableTotalsSliceLast` and `EnableTotalsSliceFirst` are worth enabling only
  when every condition of `#std708` holds at once: a large data volume is
  expected, the configuration issues frequent slice queries for the current
  moment without a period parameter, the remaining slice conditions use only
  dimensions and separators in `Независимо и совместно` mode, and the register's
  access restrictions use only the same. A price register qualifies; a currency
  rate register usually does not.

## Totals Separation Is A Trade-Off, Not A Default

- `#std664` recommends totals separation when the dimension set alone cannot give
  the needed write concurrency: two sessions writing rows that match on all
  dimensions otherwise wait for each other.
- `#std733` says the opposite for reads: the platform builds its cheapest plan —
  reading the stored current-balance table with no grouping by dimensions — only
  when totals separation is **not** used, the balance is requested without a
  date, and the outer query uses every dimension of the virtual table in
  `ВЫБРАТЬ` or in the join condition.
- `#std661` and `#std664` add that separation buys nothing while a balance control
  query still sits early in the transaction: the control query merges the split
  resources back into one.

State which side the application is on and why. A register that is written
concurrently by many sessions and read mostly in reports leans to separation; a
register read on every screen refresh leans away from it.

## Reading Registers

- Filters belong in the virtual table parameters, not in an outer `ГДЕ`
  (`#std657`, `v8cs:ql-virtual-table-filters`). A virtual table called with no
  parameters at all is `BSLLS:VirtualTableCallWithoutParameters`.
- Read information register data with a query when nothing will be modified
  (`#std447`).
- Use `РегистрСведенийМенеджерЗаписи` only when the filter covers every dimension
  of the register at once. The platform implements record-manager writes through
  two record sets with filters, so record set event handlers fire either way, and
  in some scenarios the manager is slower for the extra work. Otherwise use
  `РегистрСведенийНаборЗаписей`.

## Writing At Volume

- Do not write record sets in a loop, one or a few records at a time: it is
  several times slower than writing one set (`#std792`). Batch around 1000
  records per set.
- Do not rewrite a large set when 30% or fewer of its records change. A rewrite
  deletes everything matching the filter and reinserts the whole set including
  unchanged rows, which is slower than applying only the changes and inflates
  the DBMS transaction log.
- Use the replacement modes together with computing the changed records: pass
  `РежимЗамещения = Ложь` when appending to an empty register, and otherwise
  `Слияние`, `Удаление` (from 8.3.26), or `Обновление` (from 8.3.27), which
  process records in batches.

## Rights

- A role must not grant modification rights on a register subordinate to a
  recorder. Those records belong to posting, not to the user. `АПК:229` raises it
  at `Обязательно` severity, tied to `#std689` p. 1.7.

## Stop Rules

- Do not add a dimension to a `Balances` register that is meaningful on only one
  record direction.
- Do not enable totals separation without naming the read path it costs, and do
  not disable it without naming the write contention it restores.
- Do not enable slice totals on a periodic information register unless every
  `#std708` condition holds; the extra tables cost writes.
- Do not change a register's dimensions, resources, periodicity, or write mode
  without a stated migration for the data already stored.
- If public MCP `unica` lacks the operation needed to inspect the register, its
  recorders, or its read paths, report a Unica MCP contract gap.
