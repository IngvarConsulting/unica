---
name: object-events
description: "Обработчики событий объекта 1С. Используй когда нужно выбрать обработчик под логику, написать или отревьюить ПередЗаписью, ПриЗаписи, ОбработкаПроверкиЗаполнения, ОбработкаЗаполнения, ПриКопировании, ПередУдалением, разобраться с параметром Отказ, признаком ОбменДанными.Загрузка или подписками на события."
---

# Object Event Handlers

## MCP routing

- Preferred path: use MCP `unica` tools `unica.project.map`, `unica.meta.info`, `unica.code.search`, `unica.code.definition`, `unica.code.outline`, `unica.code.graph`, `unica.code.patch`, `unica.code.diagnostics`, and `unica.runtime.execute`.
- По INV-MCP-RUNTIME-RECEIPT текущий runtime-контракт: `unica.runtime.execute` — preview-only и вызывается только с `dryRun: true`; любой applied-режим возвращает fail-closed до workspace discovery и process spawn. Preview не является runtime verification. Не обходи этот отказ прямым runner-ом, через `unica.build.*` или fallback через `unica.runtime.job.*`.
- Use `unica.standards.search` and `unica.standards.explain` for a development-standard about handlers: 396, 455, 463, 464, 465, 466, 686, 752, 773, and diagnostics АПК:75, АПК:144, АПК:1340, BSLLS:DataExchangeLoading, BSLLS:UsingCancelParameter, BSLLS:MissingEventSubscriptionHandler. These are standards, not evidence of runtime behavior; confirm the wording before citing one.
- Do not call internal analyzer, runtime, standards, or package adapters directly. They are hidden behind MCP `unica`.

## References

- Read `../../references/platform/object-events.md` for the handler map, the conditional fill-check shape, the exchange guard, and the cancel-parameter rule.
- Read `../../references/platform/document-posting.md` when the handler in question is `ОбработкаПроведения` or `ОбработкаУдаленияПроведения`.
- Read `../../references/platform/platform-mechanics.md` for transaction boundaries and logging inside write handlers.

## Core model

Pick the handler by what the logic needs to see, not by what is convenient:

- Needs the fill source → `ОбработкаЗаполнения`. Refusing "create based on" belongs here too, raised as an exception, not moved into a separate command handler (std396).
- Needs to reject bad data before writing → `ОбработкаПроверкиЗаполнения` (std463).
- Needs the old stored values, or must fill or check before the write → `ПередЗаписью` (std464).
- Needs the object to already exist in the database → `ПриЗаписи`. Do not modify the object there; it is already written (std465).
- Needs to run before the object disappears → `ПередУдалением` (std752).
- Needs to strip values that must not survive a copy → `ПриКопировании` (std466).

Two rules cut across all of them: `ОбменДанными.Загрузка` is checked first in `ПередЗаписью`, `ПриЗаписи`, and `ПередУдалением` (std773), and `Отказ` is only ever assigned `Истина` (std686).

## Workflow

1. Name what the logic needs to observe — fill source, old values, written state, or nothing yet — and let that pick the handler.
2. Locate what already runs: read the object module with `unica.code.outline` and `unica.code.definition`, and find subscriptions on the same events with `unica.code.search` and `unica.meta.info`. A subscription is invisible from the object module it affects.
3. Use `unica.code.graph` when the handler calls shared procedures, to see what else the change reaches.
4. Write the guard before the logic: `Если ОбменДанными.Загрузка Тогда Возврат; КонецЕсли;` — in the subscription handler too, not only in the object module.
5. Express conditional requiredness by collecting `НепроверяемыеРеквизиты` and removing them from `ПроверяемыеРеквизиты` at the end, never by adding to `ПроверяемыеРеквизиты`.
6. Apply the change with `unica.code.patch`, one verifiable step at a time.
7. Verify statically with `unica.code.diagnostics`; use `unica.runtime.execute` only to preview typed syntax/test arguments, and require separate runtime evidence for the exchange path when the object participates in one.

## Design rules

- The object must load as it is during exchange: no repeated business logic, no extra checks, no changes that could distort data or block the load (std773). Code that sets `ОбменДанными.Загрузка = Истина` takes responsibility for the object's integrity itself.
- Never assign `Ложь` to `Отказ`, and never assign a boolean function result to it — either can clear a `Истина` set earlier in the same handler or by another subscriber (std686, АПК:144). The rule covers `СтандартнаяОбработка` and `Выполнение` too.
- Setting `Отказ = Истина` without a message leaves the user the platform's own text, which names the object and nothing else. Report the reason, or raise an exception instead.
- Adding names to `ПроверяемыеРеквизиты` hides the conditional check from analysis of the `Проверка заполнения` property (std463).
- Event handlers belong in the standard event-handler region; procedures that are not handlers do not (std455, АПК:1340).

## Review checklist

- `ПередЗаписью`, `ПриЗаписи`, and `ПередУдалением` check `ОбменДанными.Загрузка` before anything else — in subscription handlers as well as object modules (АПК:75).
- Any exception to that guard carries a comment stating the reason.
- `ПриЗаписи` does not modify the object being written.
- Conditional fill checks remove from `ПроверяемыеРеквизиты`, never add to it.
- `Отказ` is only ever assigned `Истина`, and every refusal tells the user why.
- Every declared subscription has its handler procedure.
- Subscriptions on the same events were reviewed together with the object module change.

## Stop rules

- Do not move a "create based on" refusal out of `ОбработкаЗаполнения` into command handlers.
- Do not add logic to a write or delete handler without the exchange guard.
- Do not conclude what runs on write or delete from the object module alone before checking subscriptions.

## Contract gaps

If public MCP `unica` cannot inspect the object's modules, its subscriptions, or the diagnostics needed for the task, report a Unica MCP contract gap with the missing operation.
