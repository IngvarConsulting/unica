# ADR-0004: Operation scripts are reference-only, not runtime backends

- Статус: `accepted`
- Дата: `2026-05-03`

## Контекст

Unica used Python/PowerShell operation implementations to bootstrap XML/JSON
DSL workflows. Keeping those scripts on the runtime path splits execution
behavior between prompt-visible skill prose and the Rust MCP orchestrator.

The runtime architecture requires one execution surface: MCP `unica`. If
`unica-coder` can fall back to local operation files, cache invalidation,
support-guard checks, dry-run behavior, and command semantics can drift away
from the orchestrator.

## Решение

Python/PowerShell/Bash operation scripts are not accepted runtime backends for
`unica-coder`.

1. All developer operations must be implemented as `unica.*` MCP tools.
2. Existing operation-file command semantics must be ported into native Rust MCP
   handlers with fixture parity.
3. Migrated skills must reference MCP `unica` tools only.
4. Runtime handlers must return native operation results; they must not expose a
   script `command` fallback for XML/DSL operation backends.
5. Locally adapted Python scripts may remain only as Unica-owned reference
   models under `tests/fixtures/unica_mcp_script_parity/unica_reference_models`
   for native migration-equivalence tests. They are not pristine donor code.
6. A separately versioned, byte-for-byte donor snapshot may remain under
   `tests/fixtures/unica_mcp_script_parity/cc-1c-skills` for reviewed
   compatibility tests.
7. Package metadata, generated native binaries, installers, and CI scripts remain
   infrastructure and are not covered by this skill-local removal rule.

## Неграницы

1. This ADR does not require replacing source checkout `cargo run` or packaged
   native binary entrypoints.
2. This ADR does not require replacing bundled external engines that remain
   behind internal adapters.
3. This ADR does not ban tests that execute either Unica-owned reference models
   or scripts from a concrete, reviewed donor snapshot.

## Последствия

1. The active task list must track Rust ports and parity coverage, not runtime
   script fallbacks.
2. Skill tests should reject operation-file workflow guidance.
3. Native MCP handlers become the target home for XML/JSON DSL behavior.
4. Documentation must distinguish adapted Unica reference models from pristine
   donor snapshots; neither is a runtime backend.

## План реализации

1. Add parity fixtures around current operation behavior.
2. Port read-only `info` and `validate` operations into native Rust MCP handlers.
3. Port generators/removers, then editors and complex CFE/UI operations.
4. Rewrite each migrated skill to route only through MCP `unica`.
5. Remove packaged runtime operation scripts after tests pass.
6. Keep adapted models and pristine donor snapshots in separate
   `tests/fixtures` trees with different parity contracts.

## Верификация

- [x] ADR states that operation scripts are not runtime architecture.
- [x] ADR distinguishes package/runtime entrypoints from skill-local operation files.
- [x] ADR requires MCP implementation and parity tests before deletion.
- [x] ADR distinguishes adapted Unica models from pristine donor snapshots.
