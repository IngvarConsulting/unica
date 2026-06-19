---
name: api-design
description: "Проектирование и ревью API 1С: публичный программный интерфейс, служебный интерфейс, переопределяемые модули, совместимость, версионирование и миграция потребителей. Используй когда нужно спроектировать API, оценить изменение экспортных методов или проверить, можно ли вызывать метод/запрос из другой подсистемы."
---

# API Design

Основано на статье Infostart "База по API": `https://infostart.ru/1c/articles/2683808/`.

## MCP routing

- Preferred path: use MCP `unica` tools `unica.code.search`, `unica.code.definition`, `unica.code.grep`, `unica.code.diagnostics`, `unica.project.map`, `unica.subsystem.info`, `unica.meta.info`, `unica.standards.search`, `unica.standards.explain`, and `unica.runtime.execute`.
- Use v8std through public `unica.standards.*` tools for standards 483, 543, 551, 553, and 644 before making compatibility claims.
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

1. Map source-sets with `unica.project.map` and inspect subsystem/object boundaries with `unica.subsystem.info` or `unica.meta.info`.
2. Find the candidate API with `unica.code.definition`; use `unica.code.search` and `unica.code.grep` for callers, export area comments, module suffixes, deprecated sections, and literal contract mentions.
3. Check standards through `unica.standards.explain` / `unica.standards.search`: functional subsystems, libraries, overridable modules, version numbering, and backward compatibility.
4. Classify the change: new method, optional parameter, mandatory parameter, removed/renamed method, changed parameter type, behavior change, deprecated method, or direct data access across a boundary.
5. Decide the required version impact and migration path.
6. Verify with `unica.code.diagnostics`, `unica.runtime.execute` syntax/tests, and consumer-style tests when the API has real callers.

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
    "name": "unica.standards.explain",
    "arguments": {
      "idOrAliasOrUrl": "644",
      "bodyLimit": 4000
    }
  }
}
```

```jsonc
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.code.search",
    "arguments": {
      "cwd": "<workspace>",
      "query": "Устаревшие процедуры и функции",
      "limit": 20
    }
  }
}
```
