# 1. Введение и цели

## Назначение

Unica — плагин для повседневной разработки на 1C:Enterprise. Один каталог
`plugins/unica` обслуживает два хоста, Codex и Claude Code, поэтому продуктовые
формулировки во всём активном слое спецификаций не привязаны к конкретному
хосту (INV-PRODUCT-01,
[ADR-0012](../../decisions/0012-one-plugin-directory-for-two-hosts.md)).

Плагин закрывает рабочий цикл разработчика: определение workspace и его source
sets, правку XML-исходников конфигурации и расширений, формы, схемы компоновки
данных, макеты, роли, подсистемы, сборку и запуск платформы, поиск по коду,
диагностику и справочные знания.

## Главная архитектурная цель

The model sees exactly one public MCP server, `unica`. Every other engine is an
internal adapter, so cache, index, and workspace-state coordination happens
inside the orchestrator instead of being delegated to the model (INV-MCP-01,
INV-CACHE-01).

## Заинтересованные стороны

- AI agent — the model running inside a supported host. It calls stable
  `unica.*` tools and receives one compact structured result per operation.
- 1C developer — works through operation skills and the tools they route to,
  without choosing an engine, a cache, or a host-specific entry point.
- Maintainer — updates bundled tools, skills, the Rust orchestrator, and the
  specifications without breaking the public MCP contract (INV-MCP-08).

## Цели

1. One public MCP contract that is identical on both supported hosts.
2. Minimal model context spent on infrastructure coordination.
3. Cache and workspace state owned explicitly by the Rust orchestrator.
4. Operation semantics owned by native Rust handlers behind `unica.*` tools.
5. Packaging verified for every supported host and target before publication.

## Качественные требования

Качественные требования, из которых выведены эти цели, живут в главе
[10. Требования к качеству](10-quality-requirements.md) и здесь не
повторяются.
