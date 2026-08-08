# Module Placement

Use this reference when deciding which module hosts a procedure, what flags a
common module carries, or how startup and presentation code is arranged.

This is about *hosting*: which module the code lives in and what that module is
allowed to do. Classifying an exported method as public, service, or overridable
interface, and deciding what a change does to compatibility, belongs to
`api-design` instead.

The normative rules here are development standards, not platform evidence:
`#std455` module structure, `#std469` creating common modules, `#std474` naming,
`#std486` object, manager and common modules, `#std556` initial launch,
`#std679` the server call flag, `#std724` reusable-return modules, `#std746`
presentation handlers, and diagnostics `АПК:73`, `АПК:80`, `АПК:83`, `АПК:84`,
`АПК:85`, `АПК:90`, `АПК:125`, `АПК:363`, `АПК:435`–`АПК:439`, `АПК:444`,
`АПК:1245`, `BSLLS:CommonModuleInvalidType`, `BSLLS:CommonModuleNameWords`,
`v8cs:common-module-server-call`. Confirm the current wording with
`unica.standards.explain` before citing one.

## Object, Manager, Or Common

`#std486` gives one question per module kind:

- **Object module** — does the code work with a specific instance
  (`СправочникОбъект`, `ДокументОбъект`), through `ЭтотОбъект` and module
  variables, including before the object is written? Then it belongs here:
  object event handlers, instance fill procedures. Note the cost of calling an
  exported procedure from elsewhere — the caller usually has to get the instance
  through `ПолучитьОбъект()` first, which reads the whole object including its
  tabular sections.
- **Manager module** — is the code static with respect to the metadata object:
  about the set of objects (list printing, information common to all instances,
  data updates tied to the metadata object), or about an already-written object
  passed in as a reference (a print form by reference, movements by reference)?
  Then it belongs here, and it must not require an instance of the data object.
- **Common module** — is the functionality impossible to attribute to exactly one
  metadata object? Then it is shared, and it belongs in a common module grouped
  by one subsystem or one functional purpose.

## The Four Common Module Contexts

`#std469` fixes four combinations. Anything else is `АПК:125` /
`BSLLS:CommonModuleInvalidType`.

| Context | Клиент (упр.) | Сервер | Внешнее соединение | Клиент (обычн.) | Вызов сервера | Name postfix |
| --- | --- | --- | --- | --- | --- | --- |
| Server | – | yes | yes | yes | – | none, or `Сервер` on a name clash |
| Server for client calls | – | yes | – | – | yes | `ВызовСервера` |
| Client | yes | – | – | yes | – | `Клиент`, or `Глобальный` |
| Client-server | yes | yes | yes | yes | – | `КлиентСервер` |

- A server module keeps `Вызов сервера` off so that procedures taking mutable
  types work correctly — subscription handlers that receive an object, and server
  procedures called from object modules and similar server-side paths.
- Exported procedures in a `ВызовСервера` module must not take mutable types.
- Naming postfixes are enforced: `Клиент` (`АПК:80`), `Глобальный` (`АПК:83`),
  `ПолныеПрава` (`АПК:84`), `ПовтИсп` (`АПК:85`), `ВызовСервера` (`АПК:90`),
  `КлиентСервер` (`АПК:1245`). A global module does not additionally carry
  `Клиент` (`АПК:363`).
- Avoid generic words in a common module name — `Процедуры`, `Функции`,
  `Обработчики`, `Модуль`, `Функциональность` (`АПК:73`,
  `BSLLS:CommonModuleNameWords`).

## The Server Call Flag Is A Security Boundary

`#std679` treats `Вызов сервера` as an exposure decision, not a convenience:

- Do not set it on every server module by default. Only the API genuinely called
  from the client belongs in a module that carries it.
- That API must not reveal data the user should not see, and must not perform
  actions the user is not allowed to perform. A server calculation returns the
  result, not the source or intermediate data the current user may have no rights
  to.
- Code that enters privileged mode, and code inside modules marked
  `Привилегированный`, needs the most scrutiny here.
- In a managed application, work with object instances belongs on the server. Do
  not create or fetch them from client common modules — including under the
  `ТолстыйКлиентУправляемоеПриложение` directive — or from ordinary forms running
  in managed mode. That keeps object module and subscription code off the client
  and avoids the extra server calls it would generate.
- If the configuration is not meant to run in the thick client, clear the
  configuration's thick-client managed-application flag; it removes false
  findings from configuration checking.

## Cached Modules Are Not Free

`#std724` on reusable-return modules:

- Cache what comes from the database, from external sources, or from expensive
  computation. Do not cache what is computed faster than it is retrieved —
  returning a string constant from a cached function is `АПК:435`, and the same
  applies to Number, Date, Boolean, and predefined items
  (`АПК:436`–`АПК:439`). An exported procedure in such a module is `АПК:444`.
- Cache only what will actually be read often. A cached value is dropped 20
  minutes after it was computed, or 6 minutes after its last use, whichever comes
  first, and also on memory pressure, worker process restart, or the client
  switching to another worker process. The standard notes these intervals can
  differ between versions, so treat them as the shape of the lifetime, not as a
  guarantee.
- Keep the input parameter range narrow. A function keyed by counterparty, in a
  base with many counterparties and little repeat access, mostly fills memory
  with values nobody reads back.
- Over-use costs memory (`#std725`).

## Presentation Handlers In The Manager Module

`ОбработкаПолученияПолейПредставления` and `ОбработкаПолученияПредставления`
override how an object is shown in fields and lists (`#std746`).

- They run every time any presentation is needed, so excess data or a poor field
  choice slows the whole system down.
- Do not run queries or fetch objects in them. Dotted access to an attribute of a
  reference type is forbidden here because it reads the whole object from the
  database, and getting presentations and attributes of references is likewise
  undesirable.
- They can also fire while an object is written or deleted during data exchange,
  because the presentation is needed for the event log entry. The same
  constraints as registration logic apply, including not reaching for predefined
  items that may not be loaded yet — or may already have been deleted — during
  exchange (`#std697`).

## Startup And First Launch

`#std556` requires a mechanism that detects the configuration's first launch and
performs the minimal initial fill, and detects the first launch of a new release
and applies the required data changes. With БСП, that is the infobase version
update subsystem.

- Separate mandatory initial fill, without which the configuration does not work,
  from optional fill that only eases the start.
- Show the administrator the configuration description or the version change
  description after the first launch or a release change.
- Detect incomplete processing, warn the user, and write the detailed operation
  and error log to the event log.
- In a distributed infobase, subordinate node updates run after the already
  updated data arrives from the main node, and must not reprocess the same data
  or recreate what exists. Unconditional creation in a subordinate node multiplies
  the data across every node through exchange; unconditional modification
  registers it for upload to the main node and loads the channel for nothing.
  Check for existence with a query before creating.

## Stop Rules

- Do not put logic needing an object instance into a manager module, and do not
  reach for `ПолучитьОбъект()` from a manager module to work around that.
- Do not set `Вызов сервера` to make a call compile.
- Do not add a cached function without naming what it reads and how often it is
  read back.
- Do not query or dereference inside a presentation handler.
- Do not create data unconditionally in a first-launch or update handler.
- If public MCP `unica` lacks the operation needed to inspect the module, its
  flags, or its callers, report a Unica MCP contract gap.
