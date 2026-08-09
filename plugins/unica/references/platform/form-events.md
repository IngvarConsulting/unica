# Form Module Events

Use this reference when writing or reviewing a managed form module: which
handler runs where, what a server call costs, how parameters arrive, and how
handlers are attached from code.

The normative rules here are development standards, not platform evidence:
`#std439` compilation directives and preprocessor instructions, `#std455` module
structure, `#std487` server calls and traffic, `#std492` attached form handlers,
`#std642` long server operations, `#std724` reusable-return modules, `#std741`
parameterized forms, and diagnostics `АПК:100`, `АПК:104`, `АПК:526`, `АПК:547`,
`АПК:1410`, `АПК:1412`, `BSLLS:CompilationDirectiveLost`,
`BSLLS:CompilationDirectiveNeedLess`, `BSLLS:SeveralCompilerDirectives`,
`BSLLS:ServerSideExportFormMethod`, `v8cs:form-module-missing-pragma`,
`v8cs:invocation-form-event-handler`, `v8cs:unknown-form-parameter-access`.
Confirm the current wording with `unica.standards.explain` before citing one.

## The Two Sides Of A Form Module

A form module holds client code and server code in one file, and the directive
on each procedure decides which. That is the fact every other rule here follows
from.

The server-side lifecycle handlers are `ПриСозданииНаСервере`,
`ПриЧтенииНаСервере`, `ПередЗаписьюНаСервере`, `ПриЗаписиНаСервере`, and
`ПослеЗаписиНаСервере`; the client-side ones include `ПриОткрытии`,
`ПередЗакрытием`, `ПриЗакрытии`, `ПослеЗаписи`, and `ОбработкаОповещения`, along
with every form item handler. The write-related events are only valid when the
form's main attribute is a persistent object or record — `form-compile.md`
carries that constraint and the full event list.

The object's own `ПередЗаписью` and `ПриЗаписи` are a different context with
different rules; see `object-events.md`.

## Compilation Directives

- `&НаКлиенте`, `&НаСервере`, and `&НаСервереБезКонтекста` belong in managed form
  modules and command modules. Elsewhere prefer preprocessor instructions
  (`#std439`).
- Every procedure in a form module carries exactly one directive. A missing one
  is `АПК:526` and `v8cs:form-module-missing-pragma`; more than one is
  `BSLLS:SeveralCompilerDirectives`.
- In a server-only or client-only common module the execution context is already
  obvious, so a directive there is noise
  (`BSLLS:CompilationDirectiveNeedLess`). In a module marked both client and
  server, directives make it harder to tell which procedures are actually
  available.
- Do not branch with `#Если Сервер` or `#Если Клиент` inside a `КлиентСервер`
  common module: the execution context cannot be determined reliably there
  (`АПК:547`). Split the logic into `Клиент` and `Сервер` modules carrying the
  same function name and keep the shared part in `КлиентСервер`.
- Branching on client mode inside an ordinary client module — `#Если ВебКлиент`,
  for instance — is acceptable.
- Do not let a preprocessor instruction or a region cut across a grammatical
  construct, an expression, a procedure declaration, or a call site.

## Server Calls Are The Budget

`#std487` sets the default target: one user action must not produce additional
server calls from configuration code, and a deviation needs a stated reason. The
total includes calls the platform itself makes, not only the ones written in the
configuration.

- The first call into a client common module can cause an implicit server call
  while the module is not yet compiled. That one is expected.
- For the mobile client and slow links, control the volume of transferred data
  as well as the number of calls, and debug the interaction with server-call
  delay simulation switched on.
- A server procedure called from the client should mark its parameters `Знач`
  (`АПК:1412`).
- A server export method on a form is `BSLLS:ServerSideExportFormMethod`.
- Work that takes long belongs in a background job (`#std642`); see the
  `background-jobs` skill rather than growing the server call.

### Application Startup

- `ПередНачаломРаботыСистемы` and `ПриНачалеРаботыСистемы` should not reach the
  server in the simple case.
- When the server is unavoidable, do not call server procedures directly from the
  application module, the managed application module, or the external connection
  module, and pass every startup parameter across in a single server call.
- When the same startup parameters are needed in several places on the client,
  serve them from a reusable-return module (`#std724`) so the server is called
  once and the result is cached on the client.
- With БСП, place startup server code in
  `ПриДобавленииПараметровРаботыКлиентаПриЗапуске` or
  `ПриДобавленииПараметровРаботыКлиента` of `ОбщегоНазначенияПереопределяемый`.

## Form Parameters

- Declare parameters on the form's parameter tab. Then `ПриСозданииНаСервере`
  reads them directly, and the parameter set is visible without reading the
  module (`#std741`).
- `Параметры.Свойство("Имя", Значение)` inside `ПриСозданииНаСервере` is the
  symptom of an undeclared parameter: `АПК:1410`,
  `v8cs:optional-form-parameter-access`, and
  `v8cs:unknown-form-parameter-access` all cover this area.
- A form that requires parameters and is opened only from code must not be the
  object's main form, and must not be reachable from the "Все функции" menu.
- When the object has no other form, the parameterized one becomes main. Then
  `ПриСозданииНаСервере` checks the parameters and raises an exception whose text
  explains to the user why the form cannot open.

## Handlers Attached From Code

- A handler assigned through `УстановитьДействие` is named with the
  `Подключаемый_` prefix (`#std492`, `АПК:100`,
  `v8cs:module-attachable-event-handler-name`). The prefix is what lets the
  "Поиск неиспользуемых процедур и функций" check tell a dynamically attached
  handler from genuinely dead code.
- Attaching inside the form module itself usually does not raise the unused
  warning; the prefix earns its keep when the attach happens from another module.

## Where Handlers Live

- Form event handlers belong in the standard form event-handler region, and
  procedures that are not handlers do not (`#std455`,
  `v8cs:module-structure-form-event-regions`).
- Do not call a form event handler programmatically
  (`v8cs:invocation-form-event-handler`). Extract the body into a named procedure
  and call that from both places.

## Stop Rules

- Do not add a procedure to a form module without a directive, and do not add a
  second directive to one that has it.
- Do not solve a client/server difference with a preprocessor branch inside a
  `КлиентСервер` module.
- Do not read a form parameter that is not declared on the parameter tab.
- Do not add a server call to a handler that runs on every user action without
  stating why it is needed.
- If public MCP `unica` lacks the operation needed to inspect the form, its
  parameters, or its module, report a Unica MCP contract gap.
