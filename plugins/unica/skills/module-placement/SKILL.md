---
name: module-placement
description: "Размещение кода 1С по модулям. Используй когда нужно выбрать между модулем объекта, модулем менеджера и общим модулем, задать контекст и флаги общего модуля, решить про Вызов сервера и ПовтИсп, назвать модуль по постфиксу, написать обработчики представления в модуле менеджера или механизм первого запуска и обновления версии."
---

# Module Placement

## MCP routing

- Preferred path: use MCP `unica` tools `unica.view {}`, `unica.meta.info`, `unica.meta.add`, `unica.meta.edit`, `unica.subsystem.info`, `unica.code.search`, `unica.code.definition`, `unica.code.graph`, `unica.code.patch`, `unica.code.diagnostics`, and `unica.runtime.execute`.
- По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.
- Use `unica.standards.search` and `unica.standards.explain` for a development-standard about module hosting: 455, 469, 474, 486, 556, 679, 697, 724, 746, and diagnostics АПК:73, АПК:80, АПК:85, АПК:90, АПК:125, АПК:363, АПК:435-439, АПК:444, АПК:1245. These are standards, not evidence of runtime behavior; confirm the wording before citing one.
- Use `unica.role.info` when the module enters privileged mode or carries the privileged flag.
- Do not call internal analyzer, runtime, standards, or package adapters directly. They are hidden behind MCP `unica`.

## Scope boundary

This skill owns **where code lives**: which module hosts it and what that module may do. Classifying an exported method as public, service, or overridable interface, and judging what a change does to compatibility, belongs to `api-design`. Form module directives and the server-call budget inside a form belong to `form-events`. Object write handlers belong to `object-events`.

## References

- Read `../../references/platform/module-placement.md` for the hosting decision, the four common module contexts, the server call flag, cached modules, presentation handlers, and first launch.
- Read `../../references/platform/form-events.md` when the code is going into a form module.
- Read `../../references/platform/object-events.md` when it is an object event handler.

## Core model

One question per module kind (std486):

- **Instance state** — works with `ЭтотОбъект` and module variables, including before the object is written → object module. Calling its exported code from elsewhere usually costs a `ПолучитьОбъект()`, which reads the whole object with its tabular sections.
- **Static for the metadata object** — about the set of objects, or about an already-written one passed as a reference → manager module. It must not require an instance.
- **Cannot be attributed to one metadata object** → common module, grouped by one subsystem or one functional purpose.

Then pick exactly one of the four common module contexts (std469) and name it by its postfix. Anything outside those four is АПК:125.

| Context | Клиент (упр.) | Сервер | Внеш. соед. | Клиент (обычн.) | Вызов сервера | Postfix |
| --- | --- | --- | --- | --- | --- | --- |
| Server | – | yes | yes | yes | – | none / `Сервер` |
| Server for client calls | – | yes | – | – | yes | `ВызовСервера` |
| Client | yes | – | – | yes | – | `Клиент` / `Глобальный` |
| Client-server | yes | yes | yes | yes | – | `КлиентСервер` |

## Workflow

1. Answer the three std486 questions before opening any module. The answer, not convenience, picks the host.
2. Map the neighbourhood with `unica.view {}` and `unica.subsystem.info`: an existing module for the same subsystem or purpose is a reason to extend rather than add.
3. Check the callers with `unica.code.graph` before moving anything — a move that changes the module context changes what the callers may pass.
4. When adding a common module, choose the context row first, then `unica.meta.add` with the matching flags and postfix.
5. Set `Вызов сервера` only for API genuinely called from the client, and state what it exposes.
6. Apply code with `unica.code.patch`, one verifiable step at a time.
7. Verify statically with `unica.code.diagnostics`; use `unica.runtime.execute` to preview typed syntax/test arguments and, with `dryRun: false`, to run them and report runtime behavior as unverified.

## Design rules

- A server module keeps `Вызов сервера` off so procedures taking mutable types work correctly; exported procedures in a `ВызовСервера` module must not take mutable types (std469).
- `Вызов сервера` is an exposure decision, not a convenience (std679). The exposed API must not reveal data the user cannot see or perform actions they are not allowed. A server calculation returns the result, not the source data.
- In a managed application, object instances are worked with on the server. Do not create or fetch them from client common modules, including under `ТолстыйКлиентУправляемоеПриложение` (std679).
- Cache what comes from the database, an external source, or expensive computation — never what is computed faster than it is retrieved (std724, АПК:435 and its siblings). An exported procedure in a cached module is АПК:444.
- A cached value has a bounded lifetime and is also dropped on memory pressure, worker process restart, or a client switching worker process. Keep the parameter range narrow so the cache is actually read back.
- Presentation handlers in the manager module run on every presentation request: no queries, no fetching objects, no dotted access to reference attributes, and no predefined items that exchange may not have loaded yet (std746, std697).
- First launch and release update must be idempotent, and in a distributed infobase must not recreate or unconditionally rewrite data in a subordinate node (std556).

## Review checklist

- The module kind matches the std486 question the code actually answers.
- The common module carries exactly one of the four valid flag combinations.
- The name matches the context postfix: `Клиент`, `КлиентСервер`, `ВызовСервера`, `ПовтИсп`, `ПолныеПрава`, `Глобальный` — and a global module does not also carry `Клиент` (АПК:363).
- The name avoids generic words like `Процедуры`, `Обработчики`, `Функциональность` (АПК:73).
- `Вызов сервера` is set only where a client caller exists, and what it returns is safe to show that caller.
- No cached function returns a constant, and none is keyed by a value with an unbounded range.
- Presentation handlers contain no query, no object fetch, and no dereferencing.
- First-launch and update handlers check for existence before creating.

## Stop rules

- Do not set `Вызов сервера` to make a call compile.
- Do not move code between module kinds without checking callers first.
- Do not add a cached function without naming what it reads and how often it is read back.
- Do not create data unconditionally in a first-launch or update handler.

## Contract gaps

If public MCP `unica` cannot inspect the module, its flags, its callers, or the diagnostics needed for the task, report a Unica MCP contract gap with the missing operation.
