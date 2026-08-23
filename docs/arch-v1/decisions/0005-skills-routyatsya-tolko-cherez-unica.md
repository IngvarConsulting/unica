# ADR-0005: Skills route только через `unica`

- Статус: `accepted`
- Дата: `2026-05-03`

## Контекст

Skills are the prompt-visible operational guidance for Codex. If skills mention
different MCP servers for different tasks, the model again becomes responsible
for infrastructure routing and cache coordination.

## Решение

Skills must route through MCP `unica`.

1. In-scope XML/DSL skills prefer `unica.*` tools through MCP `unica`.
2. Skills do not instruct the LLM to call internal adapter MCP servers.
3. Mutating skill guidance preserves explicit `dryRun: false` only for requested
   mutations.
4. Out-of-scope skills may mention domain tools such as v8-runner conceptually,
   but not as separate public MCP registrations.

## Неграницы

1. This ADR does not ban documenting internal tooling in maintainer docs.
2. This ADR does not require every skill to have a dedicated MCP tool.
3. This ADR does not ban maintainer-only parity fixtures; ADR-0004 bans runtime
   operation-file backends.

## Последствия

1. Skill tests must reject old internal MCP names in prompt-visible routing.
2. Migrated skill tests must reject operation-file workflow guidance.
3. Changing `.mcp.json` must be synced with skills.
4. New operation skills should be written against `unica.*` first.

## План реализации

1. Keep `tests/ci/test_unica_skills.py` aligned with this ADR.
2. Update skill routing blocks when public tools are renamed.
3. Keep maintainer-only internal tool docs outside prompt-routing language.

## Верификация

- [x] ADR fixes one MCP route for skills.
- [x] ADR preserves dry-run mutation safety.
- [x] ADR distinguishes maintainer docs from prompt-visible routing.
