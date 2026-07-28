# 3. Контекст и границы системы

## Системный контекст

Unica sits between a host that runs the model and the local 1C development
assets. It turns operation-level requests into typed Rust use cases, cache
decisions, and internal adapter calls.

## Внешние акторы

- Codex and Claude Code — the two supported hosts. Both discover `skills/` and
  the root `.mcp.json` in the same plugin directory, and each reads only its
  own manifest directory (INV-PRODUCT-01).
- The AI agent inside a host — the only caller of the public tool surface.
- The 1C developer — states the intent and approves a mutation.
- The local workspace — source sets on disk, `v8project.yaml`, and git state.
- A local 1C:Enterprise installation — reached only through the bundled
  runtime tooling for dump, load, build, and launch operations.
- The remote standards service at `https://ai.v8std.ru/mcp`, overridable with
  `UNICA_STANDARDS_MCP_URL`.
- The release origin
  `https://github.com/IngvarConsulting/unica/releases/download/<tag>/` — the
  only approved source of runtime archives for `unica-bootstrap`
  (INV-PKG-03).

## Публичная граница

The only public MCP server is `unica` (INV-MCP-01, INV-MCP-02), and every
public tool is named `unica.<group>.<operation>` (INV-MCP-04).

Источник истины по составу публичной поверхности — реестр в коде:
`UnicaApplication::tools()` в `crates/unica-coder/src/application/mod.rs`. Эта
глава перечисляет только группы и их назначение; конкретные имена, схемы и
количество инструментов читаются из реестра, а не отсюда.

| Группа | Назначение |
| --- | --- |
| `unica.project.*` | workspace status and the source-set map |
| `unica.cf.*` | configuration container: scaffold, inspect, edit, validate |
| `unica.cfe.*` | configuration extensions: scaffold, borrow, patch, diff |
| `unica.meta.*` | metadata objects and their structure |
| `unica.form.*` | managed forms |
| `unica.dcs.*` | data composition schemas |
| `unica.mxl.*` | spreadsheet documents |
| `unica.role.*` | roles and access rights |
| `unica.subsystem.*` | subsystems |
| `unica.interface.*` | subsystem command interface |
| `unica.template.*` | templates attached to a metadata object |
| `unica.help.*` | built-in object help |
| `unica.support.*` | vendor support state of a configuration and its objects |
| `unica.epf.*` | external data processor scaffolding |
| `unica.erf.*` | external report scaffolding |
| `unica.build.*` | dump, load, update, make, and launch through the platform |
| `unica.runtime.*` | typed runtime workflows and durable runtime jobs |
| `unica.code.*` | BSL search, navigation, patching, and diagnostics |
| `unica.standards.*` | 1C development standards knowledge |

## Внутренняя граница

Internal adapters may reach:

- the bundled runtime tool for platform build and launch operations;
- the bundled BSL analyzer and the BSL index for code intelligence;
- typed `git grep` as one code-search section;
- in-process native XML and DSL operations for metadata artifacts;
- the remote standards endpoint over HTTP.

None of them is a public MCP registration, and prompt-visible text never names
them as a call target (INV-MCP-01, INV-PRODUCT-03).

## Вне области

- Publishing a separate public MCP server for a specialized engine.
- Exposing internal service, cache, or index coordination as a public tool
  argument.
- Reintroducing an operation-file execution path in the runtime (INV-APP-04,
  INV-SKILL-03).
- Replacing the donor snapshot and the reference models that exist only as
  parity fixtures (INV-SKILL-04).
