# ADR-0001: Единый публичный MCP `unica`

- Статус: `accepted`
- Дата: `2026-05-03`
- Обновлено: `2026-07-28`

> Пункт 2 «Плана реализации» описывал упаковку, действовавшую до ADR-0008.
> Публичный пакет больше не запускает `./bin/<target>/unica` напрямую; прямой
> запуск остался только у локального debug-пакета. Действующая норма —
> `INV-PKG-THIN-PACKAGE` и `INV-PKG-DEV-ONLY-PACKAGE`. Остальная часть решения в силе: правило единого
> публичного сервера ADR-0008 не затрагивает.

## Контекст

Несколько LLM-visible MCP servers заставляют модель выбирать между engines и
координировать их кеши через текстовый контекст. Это тратит токены и создает
риск рассинхронизации после мутаций.

Unica должен помогать экономить контекст, поэтому инфраструктурная координация
должна жить внутри одного orchestrator.

## Решение

Публичная MCP surface Unica состоит из одного server: `unica`.

1. `plugins/unica/.mcp.json` объявляет только `unica`.
2. `initialize` возвращает `serverInfo.name = "unica"`.
3. Public tool names используют namespace `unica.*`.
4. Build/runtime, code analysis, standards, and XML/JSON DSL engines скрыты за
   internal adapters.
5. LLM не должна знать имена internal adapter servers.

## Неграницы

1. Это не запрещает использовать MCP protocol внутри adapter implementation.
2. Это не требует немедленно переписать все adapters на Rust.
3. Это не требует менять формат bundled external engines, которые остаются
   скрытыми за internal adapters.

## Последствия

1. Tool routing становится проще для skills и LLM.
2. Cache invalidation can be coordinated inside one process.
3. Package tests must assert that `.mcp.json` has exactly one public server.
4. Any future public server split requires a superseding ADR.

## План реализации

1. Keep `.mcp.json` single-server.
2. Source checkout starts `unica` with `cargo run --manifest-path ../../Cargo.toml --bin unica`; packages start
   the single public server with `cwd` set to the plugin root. The launcher for
   published packages was replaced by ADR-0008; see `INV-PKG-THIN-PACKAGE`.
3. Keep tests for `initialize`, `tools/list`, and dry-run mutating calls.
4. Validate generated marketplace packages, not only source files.

## Верификация

- [x] ADR states the single public MCP rule.
- [x] ADR distinguishes public server from internal adapters.
- [x] ADR requires package-level validation.
