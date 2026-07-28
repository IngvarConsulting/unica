# 4. Стратегия решения

## Разделение на слои

Unica uses a pragmatic DDD split:

- `domain` — workspace identity, cache impact, domain events, and pure rules;
- `application` — the public tool registry, dispatch, and orchestration;
- `infrastructure` — internal adapters, platform facades, and workspace state;
- `interfaces` — the MCP transport.

Направление зависимостей между слоями нормировано и проверяется стражем
(INV-APP-05, INV-APP-01, INV-APP-02); эта глава только называет разбиение.

## Стратегические линии

Ниже — пять линий, по которым принимались решения. Это описание стратегии, а не
перечень решений: полный список принятых ADR живёт только в
[индексе решений](../../decisions/README.md), и второй копии у него быть не
должно (INV-DOC-04, INV-DOC-08). Решения цитируются по ID.

**Одна публичная поверхность.** The public MCP surface is exactly one server,
`unica`; every engine hides behind an internal adapter, and skills route through
that one boundary instead of naming an engine (ADR-0001, ADR-0005). Domain names
on that surface are canonical and carry no compatibility aliases, so a tool name
means one thing forever (ADR-0011).

**Транспорт отделён от смысла.** The application layer stays transport-neutral
and the interface layer only maps protocol requests to application calls
(ADR-0002). The stdio transport itself is owned by the official Rust SDK, while
tool contracts stay outside the SDK macro layer and remain data-driven
(ADR-0013). OS-specific code lives behind platform facades, so layer direction
is a property a guard can check rather than a convention (ADR-0009).

**Состояние принадлежит оркестратору.** The orchestrator owns workspace state,
and typed domain events drive cache invalidation instead of adapters
invalidating caches for themselves (ADR-0003). Expensive warm state lives in
hidden services scoped by workspace root plus source root, which keeps analyzer
and index state warm without turning an engine into a public registration
(ADR-0006).

**Операции реализованы нативно.** Every developer operation is a native handler
behind a `unica.*` tool. Donor operation scripts survive only as reference
fixtures for parity tests, never as an execution path (ADR-0004).

**Поставка тонкая и проверяемая.** The published package carries metadata and
bootstrap binaries, not a runtime; the host runtime is fetched and verified
before use, and publication is two-phase so the catalog never points at bytes
that are not final (ADR-0008). One plugin directory serves both hosts, with only
the manifest directories host-specific (ADR-0012). The pipeline builds once per
platform runner with a locked dependency graph, keeps artifacts narrow, and
publishes only from a tag (ADR-0010).
