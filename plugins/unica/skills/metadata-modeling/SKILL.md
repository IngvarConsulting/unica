---
name: metadata-modeling
description: "Моделирование метаданных 1С. Используй когда нужно выбрать класс объекта под бизнес-данные — справочник, документ, перечисление, план видов характеристик, константа, — типизировать реквизиты, решить про составные и определяемые типы, длину строк, предопределённые элементы, общие реквизиты, имена и представления."
---

# Metadata Modeling

## MCP routing

- Preferred path: use MCP `unica` tools `unica.view {}`, `unica.cf.info`, `unica.meta.info`, `unica.meta.add`, `unica.meta.edit`, `unica.subsystem.info`, `unica.code.search`, `unica.code.diagnostics`, and `unica.runtime.execute`.
- По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.
- Use `unica.standards.search` and `unica.standards.explain` for a development-standard about modelling: 432, 468, 474, 531, 587, 603, 649, 677, 697, 704, 728, and diagnostics АПК:93, АПК:304, АПК:305, АПК:1205, АПК:1207, АПК:1210-1217, АПК:1329, АПК:1330. These are standards, not evidence of runtime behavior; confirm the wording before citing one.
- Use `unica.role.info` when predefined items need their interactive-deletion rights checked.
- Do not call internal analyzer, runtime, standards, or package adapters directly. They are hidden behind MCP `unica`.

## Scope boundary

This skill owns **which class holds the data and how its attributes are typed**. Register internals belong to `register-design`, what a document records and when to `document-posting`, event handlers to `object-events`, code hosting to `module-placement`, and form layout to `form-patterns`.

## References

- Read `../../references/platform/metadata-modeling.md` for the class choice, attribute typing, predefined items, and names and presentations.
- Read `../../references/platform/register-design.md` when the answer turns out to be a register.
- Read `../../references/platform/metadata-conventions.md` for naming, synonym, and fill-check conventions.

## Core model

Ask what changes and who changes it:

- **A fact at a point in time** → document. Most documents post, and an unposted one is a draft (std603).
- **A stable list the user maintains** → catalog.
- **A fixed set known when the configuration is written** → enumeration. Its values are metadata child objects, so adding one is a configuration change and a deployment, never a user action.
- **A set of kinds the user must extend at run time** → chart of characteristic types. This is the answer whenever "the customer will add their own types later" appears and an enumeration was the first instinct.
- **One value for the whole infobase** → constant.
- **An accumulated resource or a state keyed by dimensions** → register; go to `register-design`.

A value several objects carry identically may be a common attribute (std677) rather than a repeated one.

## Workflow

1. State the business question the data must answer, and who is allowed to change the set of values. That answer picks the class.
2. Check what already exists with `unica.cf.info` and `unica.meta.info` — an existing object with the same meaning is a reason to extend rather than add — and locate the owning subsystem with `unica.subsystem.info`.
3. Type every attribute deliberately: string length, composite type set, and whether a defined type already covers it (std432, std728, std704).
4. Decide predefined items and their update mode before creating the object, and check the interactive-deletion rights with `unica.role.info` (std697).
5. Create with `unica.meta.add` and refine with `unica.meta.edit`, one verifiable step at a time.
6. Fill names, synonyms, and presentations; leaving both object and list presentation empty is АПК:93.
7. Verify statically with `unica.code.diagnostics`; use `unica.runtime.execute` to preview typed syntax/test arguments and, with `dryRun: false`, to run them, report runtime behavior as unverified, and search existing callers with `unica.code.search` when a type changed.

## Design rules

- A composite attribute used in joins, filters, or sorting contains only reference types. Mixing in `Строка`, `Число`, `Дата`, `УникальныйИдентификатор`, `Булево`, `ХранилищеЗначения` slows those queries markedly (std728, АПК:1329).
- To combine a reference with free text, add a separate catalog for the free values instead of a primitive type, fill it automatically on write, and clean it with a scheduled job.
- Do not use `ЛюбаяСсылка`, `СправочникСсылка`, `ДокументСсылка` on stored objects; list the types explicitly (АПК:1330). An over-wide composite joins every table in the type on dotted access without `ВЫРАЗИТЬ`, forces restructuring when a referenced object is deleted, and makes marked-object deletion pay more for reference search and locks.
- String attributes use variable allowed length with a maximum set. Fixed length is only for a genuine equal-length guarantee (std432, АПК:1205). Unlimited length is for large user text and for machine-generated technical strings.
- A type used widely and likely to change on implementation becomes a defined type once, not a copy per use site (std704).
- Predefined items are created automatically: keep `ОбновлениеПредопределенныхДанных = Авто` and do not flip it from code (std697, АПК:304, АПК:305). In a subordinate distributed-infobase node they arrive from the main node, so update handlers that fill them run only in the main node.
- A presentation equal to the synonym is not stored, so setting it to the same text achieves nothing (АПК:1210 and its siblings).

## Review checklist

- The class matches who is allowed to change the set of values, not what was quickest to add.
- No extensible set of kinds is modelled as an enumeration.
- Every composite attribute used in joins, filters, or sorting is reference-only.
- No stored attribute uses a universal composite type.
- Every string attribute has a maximum length, and fixed length is justified.
- A widely repeated type is a defined type, and a widely repeated value is a common attribute.
- Predefined items use automatic update, and interactive deletion rights are off.
- Object or list presentation is filled, and an object does not share a name with its own child (АПК:1207).
- Documents carry the comment field (std531).

## Stop rules

- Do not change an attribute's type, length, or composite type set without a stated migration for the data already stored.
- Do not add a class of object without checking whether an existing one already means the same thing.
- Do not switch predefined-data update mode from code.

## Contract gaps

If public MCP `unica` cannot inspect the object, its attribute types, its predefined items, or the diagnostics needed for the task, report a Unica MCP contract gap with the missing operation.
