---
name: platform-help
description: "Справка платформы 1С и объектной модели BSL. Используй когда нужно уточнить метод, свойство, конструктор, поведение API, версию платформы, совместимость или стандартное решение задачи."
---

# Platform Help

## MCP routing

- For project context, use MCP `unica` tools `unica.code.search`, `unica.project.map`, and `unica.runtime.execute`.
- `unica.standards.search` and `unica.standards.explain` return development standards, not platform API/help. Use them only when the question explicitly asks for a standard or code-style rule, and label that source as `development-standard`.
- Platform behavior, API signatures, and version-dependent mechanics require a `platform-help` source. Until it is exposed by public MCP `unica`, report this as a `platform-help` contract gap rather than substituting a standards result.
- Use object-specific `unica.*.info` tools when the API question depends on metadata structure.
- Do not call internal standards, runtime, or package adapters directly. They are hidden behind MCP `unica`.

## Workflow

1. State the exact platform/API question: object, method/property, platform version, infobase mode, client/server context, managed/ordinary mode, and whether code runs in UI, server, background job, or external integration.
2. Classify the requested evidence: use platform help for API/mechanics, and use `unica.standards.*` only for development standards. State the source type in the answer.
3. Validate against local project context with `unica.project.map` and targeted `unica.code.search` if the answer depends on project conventions.
4. If behavior is version-sensitive, ask for or read the configured platform version before giving a hard answer.
5. For code examples, run `unica.runtime.execute` with `operation=syntax` when feasible.

## Platform context

- Read `../../references/platform/compatibility-modes.md` for every question about a
  compatibility mode or version-sensitive behavior. Resolve the runtime
  platform, literal mode, effective compatibility version, and
  feature-specific boundary separately.
- Read `../../references/platform/platform-mechanics.md` when the answer depends on runtime context, auth, temporary storage, data separation, background jobs, or client/server boundaries.
- Read `../../references/platform/runtime-diagnostics.md` when a platform question is really about a startup/runtime failure and needs evidence before an answer.
- Do not give a platform answer from memory when version, mode, or context can change the behavior. Resolve that first, then answer.

## Stop rules

- Do not present `unica.standards.*` output as proof of platform API behavior or exact method signatures.
- If the requested platform-help source is not available through public MCP `unica`, report it as a `platform-help contract gap` instead of bypassing the public boundary.

## MCP examples

When the answer requires platform help that the public MCP server does not yet
expose, report `platform-help contract gap` and identify the required platform
version and runtime context. Do not replace it with a standards-search call.
