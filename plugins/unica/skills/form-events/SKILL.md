---
name: form-events
description: "Модуль управляемой формы 1С. Используй когда нужно написать или отревьюить обработчики формы, расставить директивы компиляции, сократить серверные вызовы и трафик, разобраться с параметрами формы, подключаемыми через УстановитьДействие обработчиками или клиент-серверной границей в модуле формы."
---

# Form Module Events

## MCP routing

- Preferred path: use MCP `unica` tools `unica.view {}`, `unica.form.info`, `unica.form.edit`, `unica.meta.info`, `unica.code.search`, `unica.code.definition`, `unica.code.patch`, `unica.code.diagnostics`, and `unica.runtime.execute`.
- По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.
- Use `unica.standards.search` and `unica.standards.explain` for a development-standard about form modules: 439, 455, 487, 492, 642, 724, 741, and diagnostics АПК:100, АПК:526, АПК:547, АПК:1410, АПК:1412, BSLLS:SeveralCompilerDirectives, BSLLS:ServerSideExportFormMethod. These are standards, not evidence of runtime behavior; confirm the wording before citing one.
- Do not call internal analyzer, runtime, standards, or package adapters directly. They are hidden behind MCP `unica`.

## References

- Read `../../references/platform/form-events.md` for the client/server split, directives, the server-call budget, form parameters, and attached handlers.
- Read `../../references/platform/object-events.md` when the logic belongs to the object being written rather than to the form.
- Read `../../references/specs/form-patterns.md` and use `form-patterns` for layout, archetypes, and UX; this skill owns the module, not the arrangement.

## Core model

A form module holds client and server code in one file, and the directive on each procedure decides which. Everything else follows:

- **Where it runs** — `&НаКлиенте`, `&НаСервере`, `&НаСервереБезКонтекста` belong in form and command modules; elsewhere use preprocessor instructions (std439). One directive per procedure, never zero and never two.
- **What it costs** — one user action must not produce extra server calls from configuration code, and a deviation needs a stated reason (std487).
- **What it receives** — form parameters are declared on the parameter tab, so `ПриСозданииНаСервере` reads them directly instead of probing with `Параметры.Свойство()` (std741).
- **How it is wired** — a handler assigned through `УстановитьДействие` carries the `Подключаемый_` prefix (std492).

## Workflow

1. Decide which side the logic belongs to before writing it: needs the database or the object → server; needs the user or the form's visual state → client.
2. Inspect the form with `unica.form.info` for the declared events, parameters, and items, and `unica.meta.info` for the object behind it.
3. Read the existing module with `unica.view` on the module node (its `Method` branch lists the methods) and `unica.code.definition` before adding to it.
4. Count the server calls the change adds on the path of a single user action. If it adds one, name the reason.
5. Declare any new form parameter through `unica.form.edit` before reading it in the module.
6. Apply module changes with `unica.code.patch`, one verifiable step at a time, giving every new procedure exactly one directive.
7. Verify statically with `unica.code.diagnostics`; use `unica.runtime.execute` to preview typed syntax/test arguments and, with `dryRun: false`, to run them, and require separate evidence for runtime behavior and opening the affected form.

## Command Availability

When a handler must disable, lock, or re-enable a form command in the UI, do not
write `Команды[ИмяКоманды].Доступность`: the form command object is not the UI
element whose availability is shown to the user. Inspect the form first with
`unica.form.info` and find every item whose `CommandName` is
`Form.Command.<ИмяКоманды>` or the matching standard-command reference.

Set availability on every related item instead:

```bsl
Элементы[ИмяЭлемента].Доступность = Ложь;
```

If the same command is rendered by a main command bar button, table command bar
button, context-menu item, group button, or submenu item, update all of them or
state that the form must be inspected further before code is generated. For table
part standard commands, check the table's `ТолькоПросмотр` property first:
read-only tables usually let the platform block add, copy, delete, and move-row
commands without duplicate manual code. Add manual blocking mainly for custom
buttons or menu items whose handlers can still change table rows, prices,
discounts, VAT, sorting, selection, loading, filling, or recalculation.

## Design rules

- Do not branch with `#Если Сервер` or `#Если Клиент` inside a `КлиентСервер` common module — the execution context cannot be determined reliably there (std439, АПК:547). Split into `Клиент` and `Сервер` modules with the same function name and keep the shared part in `КлиентСервер`.
- A directive in a server-only or client-only common module is noise; the context is already fixed.
- A server procedure called from the client marks its parameters `Знач` (АПК:1412), and a form must not expose a server export method (BSLLS:ServerSideExportFormMethod).
- Startup handlers should not reach the server. When unavoidable, pass every startup parameter in one call and cache repeats through a reusable-return module (std487, std724); with БСП use `ОбщегоНазначенияПереопределяемый`.
- Long work belongs in a background job (std642), not in a longer server call. Route it to `background-jobs`.
- A form that needs parameters and opens only from code must not be the object's main form. If it has to be main, check the parameters in `ПриСозданииНаСервере` and raise an exception that tells the user why it cannot open (std741).
- Do not call a form event handler programmatically. Extract the body into a named procedure and call that from both places.

## Review checklist

- Every procedure in the form module has exactly one compilation directive.
- No preprocessor branch on client versus server inside a `КлиентСервер` module.
- The change adds no server call to a per-action path without a stated reason.
- Every form parameter read in the module is declared on the parameter tab — no `Параметры.Свойство()` probing in `ПриСозданииНаСервере` (АПК:1410).
- Handlers attached with `УстановитьДействие` carry the `Подключаемый_` prefix (АПК:100).
- Form event handlers sit in the standard event-handler region, and non-handlers do not.
- Write-related form events are only present when the main attribute is a persistent object or record.

## Stop rules

- Do not put logic that belongs to the object's own write path into the form module; it will not run when the object is written from code.
- Do not read an undeclared form parameter.
- Do not grow a server call to avoid a background job.

## Contract gaps

If public MCP `unica` cannot inspect the form, its parameters, its module, or the diagnostics needed for the task, report a Unica MCP contract gap with the missing operation.
