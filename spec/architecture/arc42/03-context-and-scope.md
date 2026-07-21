# 3. Контекст и границы системы

## System Context

Unica sits between Codex/LLM and local 1C development assets. It translates
operation-level requests into typed Rust use cases, cache decisions, and internal
adapter calls.

## Public Boundary

The only public MCP server is `unica`.

Public tool groups:

- `unica.project.*`
- `unica.cf.*`, `unica.cfe.*`, `unica.meta.*`
- `unica.form.*`, `unica.dcs.*`, `unica.mxl.*`, `unica.role.*`
- `unica.interface.*`, `unica.subsystem.*`, `unica.template.*`
- `unica.build.*`
- `unica.runtime.*`
- `unica.code.*`
- `unica.standards.*`
- `unica.branched.*`, `unica.delivery.*`, `unica.merge.*`,
  `unica.repository.*`

## Internal Boundary

Internal adapters may call:

- v8-runner wrappers for build/runtime operations;
- BSL analysis wrappers for code search and diagnostics;
- remote v8std endpoint for standards knowledge.
- a typed native Designer adapter for distribution, supported update, compare,
  merge, validation, and configuration-repository operations;
- a local secret resolver for repository/infobase credential references.

These adapters are not public MCP registrations.

## Out Of Scope

- Making every adapter native Rust before the public single-MCP contract is
  stable.
- Publishing separate MCP servers for specialized engines.
- Replacing donor reference scripts that are used only by parity fixtures.
- Creating/administering production repositories, stealing/admin-unlocking
  foreign locks, extension repositories, production DB restructuring, or
  loading task XML into a repository-bound original infobase.
