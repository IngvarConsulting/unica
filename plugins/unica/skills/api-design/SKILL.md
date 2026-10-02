---
name: api-design
description: "Проектирование и ревью API 1С: публичный программный интерфейс, служебный интерфейс, переопределяемые модули, совместимость, версионирование и миграция потребителей. Используй когда нужно спроектировать API, оценить изменение экспортных методов или проверить, можно ли вызывать метод/запрос из другой подсистемы."
---

# API Design

Основано на статье Infostart "База по API": `https://infostart.ru/1c/articles/2683808/`.

## MCP routing

- Preferred path: use MCP `unica` tools `unica.search`, `unica.check`, `unica.view {}`, `unica.view` on the subsystem node, `unica.view` on the object node, `unica.docs`, and `unica.run`.
- Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
при `implemented: true` используй опубликованную `argsSchema`; при
`support.state: limited` разрешено только подмножество `support.supportedArgs`.
При `support.state: unavailable` остановись; не выдумывай аргументов при
`argsSchema: null`. Превью исполнением не является. Не обходи контракт прямым runner-ом.
- Use v8std through `unica.docs` with `source: "development-standard"` for standards 483, 543, 551, 553, and 644 before making compatibility claims.
- Use `test-authoring` for unit tests that model API consumer scenarios; use `integration-implement` only when the task is about HTTP/REST/SOAP/gRPC transport implementation.
- Do not call internal analyzer, standards, runtime, or package adapters directly. They are hidden behind MCP `unica`.

## Core model

Treat 1C as a modular monolith: libraries contain functional subsystems, and subsystems communicate through declared interfaces. Direct access across subsystem/library boundaries is a design decision, not a convenience.

Classify every exported method before changing or calling it:

- `Программный интерфейс`: public contract for external consumers; backward compatibility is required.
- `Для вызова из других подсистем`: stable integration-facing area; do not treat it as arbitrary internal code.
- `Переопределяемый интерфейс`: extension point called by the library; compatibility is required for consumers that implement it.
- `Служебный программный интерфейс`: internal contract inside one library; forward compatibility is expected, backward compatibility is not guaranteed.
- `Служебные процедуры и функции`: private implementation inside one functional subsystem; external calls are a defect unless the project explicitly documents otherwise.

## Workflow

1. Map source-sets with `unica.view {}` and inspect subsystem/object boundaries with `unica.view` on the subsystem node or `unica.view` on the object node.
2. When the API belongs to a concrete metadata object, inspect `unica.view` on the object node for related modules, roles, subscriptions, and functional options before classifying the boundary.
3. Find the candidate API with `unica.search`; inspect the module with `unica.view` on the module node (its `Method` branch lists the methods) before reading broad code.
4. Find callers and impact of exported methods with `unica.search` by the method name (a call graph is not on the v0.13 surface); the same search covers export area comments, module suffixes, deprecated sections, and literal contract mentions.
5. Check standards through `unica.docs` with `source: "development-standard"`: functional subsystems, libraries, overridable modules, version numbering, and backward compatibility.
6. Classify the change: new method, optional parameter, mandatory parameter, removed/renamed method, changed parameter type, behavior change, deprecated method, or direct data access across a boundary.
7. Decide the required version impact and migration path.
8. Verify statically with `unica.check` on the module node (test runs are outside the v0.13 surface), then report runtime behavior as unverified unless separate evidence is supplied. Keep consumer-style tests for APIs with real callers.

## Compatibility rules

- Adding a public method or optional public parameter is new functionality: raise the version number, not only the build number.
- Adding a mandatory parameter, removing a mandatory parameter, deleting a public method, or changing a parameter type is breaking unless old callers still work through an adapter.
- Renaming a parameter usually does not break BSL callers because calls are positional, but changing the meaning of the parameter can still break behavior.
- Renaming or deleting a public method requires keeping the old signature in an `Устаревшие процедуры и функции` area and adding a migration path.
- Compatibility requirements override cosmetic standards such as renaming for style.
- Build number changes are for bug fixes; do not smuggle public API expansion into a build-only release.

## Overridable modules

For переопределяемые modules:

- Do not add new mandatory procedures or mandatory parameters.
- Do not change parameter types.
- Do not delete parameters that existing implementations may still receive.
- New optional procedures and optional parameters are acceptable when old implementations keep working.
- A removed parameter should be retained as unused/deprecated until consumers can migrate.

## API-first checklist

- Confirm an API is really needed; prefer simpler manual or existing mechanisms when the task does not require system-to-system integration.
- Describe use cases, data flow, state model, auth, errors, idempotency, and consumer migration before implementation.
- Choose transport separately from contract: HTTPS/REST, SOAP, gRPC, file exchange, CLI, WebSocket, or in-process module API.
- Treat API release as product release: documentation, version impact, changelog, consumer notification, and tests.
- Write unit or integration tests that model consumer calls; they become upgrade checks for the contract.

## Review red flags

- Calls to service procedures from another functional subsystem.
- Calls to service program interface from another library without an explicit contract.
- Query joins or selects across subsystem data when tables/fields are not documented as part of the API.
- Public API behavior changes hidden under bug-fix version increments.
- Removed or renamed public methods without deprecated wrappers.
- Suppressed AПК or BSL LS warnings around deprecated/interface-boundary diagnostics without a documented reason.

## MCP examples

```jsonc
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.docs",
    "arguments": {
      "query": "стандарт 644",
      "source": "development-standard"
    }
  }
}
```

```jsonc
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.search",
    "arguments": {
      "query": "Устаревшие процедуры и функции",
      "scope": "<source-set-from-unica-view>:Configuration",
      "limit": 20
    }
  }
}
```
