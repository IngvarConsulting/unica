---
name: register-design
description: "Проектирование регистров 1С. Используй когда нужно выбрать класс регистра, распределить измерения, ресурсы и реквизиты, задать периодичность и режим записи регистра сведений, решить про разделение итогов и итоги среза, проверить чтение виртуальных таблиц и массовую запись наборов."
---

# Register Design

## MCP routing

- Preferred path: use MCP `unica` tools `unica.view {}`, `unica.meta.info`, `unica.meta.add`, `unica.meta.edit`, `unica.code.search`, `unica.view` on the schema node, `unica.code.diagnostics`, and `unica.runtime.execute`.
- По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.
- Use `unica.docs` with `source: "development-standard"` for the standards about registers: 447, 477, 657, 661, 663, 664, 708, 733, 792, and diagnostics АПК:123, АПК:229, BSLLS:DenyIncompleteValues, BSLLS:VirtualTableCallWithoutParameters. These are standards, not evidence of runtime behavior; confirm the wording before citing one.
- Use `unica.view` on the role node when the register is subordinate to a recorder or carries access restrictions.
- Do not call internal analyzer, runtime, standards, or package adapters directly. They are hidden behind MCP `unica`.

## References

- Read `../../references/platform/register-design.md` for register class choice, the dimension/resource/attribute split, periodicity, totals, read paths, and bulk writes.
- Read `../../references/platform/document-posting.md` when the register is written by document posting.
- Read `../../references/platform/db-performance.md` when the register is already slow or contended.

## Core model

Four decisions, in this order, and each one narrows the next:

- **Which class**: information register for state keyed by dimensions, accumulation register with `RegisterType = Balances` for what comes in and goes out, `RegisterType = Turnovers` when balance is meaningless, accounting register for double entry, calculation register for accruals with an action period.
- **What is a key**: a dimension is a key, a resource is an aggregatable value, an attribute is neither. For a `Balances` register a dimension must be meaningful for both receipt and expense.
- **How it is read**: `Остатки` gets the platform's cheapest plan only without totals separation, without a date parameter, and with every virtual table dimension used in the outer query (std733).
- **How it is written**: one set per batch, not one write per row (std792).

Totals separation is the decision that pulls both ways: std664 wants it for write concurrency, std733 forbids it for the fastest balance read. Name the side and the reason.

## Workflow

1. State the accounting subject and the questions the register must answer. One subject, one register.
2. Pick the class, then split every field into dimension, resource, or attribute before creating anything. Check the receipt/expense rule for every candidate dimension of a `Balances` register.
3. Inspect neighbours with `unica.meta.info` — an existing register with the same subject is a reason to extend rather than add — and locate the read paths with `unica.code.search`, `unica.view` on the module node (its `Method` branch lists the methods), and `unica.view` on the schema node.
4. Decide periodicity and `WriteMode` for an information register, and whether every std708 condition holds before enabling `EnableTotalsSliceLast` or `EnableTotalsSliceFirst`.
5. Decide `EnableTotalsSplitting` against the read paths found in step 3, not in the abstract.
6. Set `DenyIncompleteValues` on dimensions that must always carry a value, and decide `Master` deliberately: it makes record lifetime follow the master value.
7. Create with `unica.meta.add` and refine with `unica.meta.edit`, one verifiable step at a time.
8. Verify statically with `unica.code.diagnostics`; use `unica.runtime.execute` to preview typed syntax/test arguments and, with `dryRun: false`, to run them, report runtime behavior as unverified, and re-check the read paths that step 5 traded against.

## Design rules

- A register must be logically independent of its recorders: no logic or report may reach a recorder field through a dot (std477, АПК:123). In a distributed infobase the movements can migrate to a node where the recorder does not exist.
- Parallelism follows the dimension set, because balances for one dimension combination live in one resource (std664).
- Filters go in the virtual table parameters, not in an outer `ГДЕ` (std657). A virtual table called with no parameters is `BSLLS:VirtualTableCallWithoutParameters`.
- Read information register data with a query when nothing will be modified. Use `РегистрСведенийМенеджерЗаписи` only when the filter covers every dimension at once; otherwise use a record set (std447).
- Batch writes around 1000 records per set, and do not rewrite a large set when 30% or fewer of its records change — a rewrite reinserts unchanged rows and inflates the DBMS transaction log (std792).
- A role must not grant modification rights on a register subordinate to a recorder (АПК:229).

## Review checklist

- Every dimension of a `Balances` register is meaningful on both receipt and expense.
- Resources hold only aggregatable values; accompanying data sits in attributes.
- `DenyIncompleteValues` is set on dimensions that cannot be empty.
- `EnableTotalsSplitting` matches a named decision about write concurrency versus balance read speed.
- Slice totals on a periodic information register satisfy every std708 condition.
- No query or report reaches a recorder field through a dot.
- Virtual tables are called with their filters as parameters.
- Register writes go through record sets in batches, not per-record in a loop.
- No role grants modification rights on a recorder-subordinate register.

## Stop rules

- Do not add a dimension to a `Balances` register that is meaningful on only one record direction.
- Do not enable totals separation without naming the read path it costs, or disable it without naming the write contention it restores.
- Do not change dimensions, resources, periodicity, or `WriteMode` without a stated migration for the data already stored.
- Do not claim a register read got faster without a before/after measurement on comparable volume.

## Contract gaps

If public MCP `unica` cannot inspect the register, its recorders, its read paths, or the runtime evidence needed for the task, report a Unica MCP contract gap with the missing operation.
