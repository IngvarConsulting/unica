# ADR-0014: Provider-neutral code intelligence

- Статус: `accepted`
- Дата: `2026-07-26`

## Контекст

Code intelligence combines independent engines with different transport and
runtime requirements: RLM, `bsl-analyzer`, and fixed-string `git grep`.
Keeping their process commands, index storage, and response parsing in the
application orchestration path couples public tools to a particular engine and
makes partial failure handling inconsistent.

## Решение

1. Application code uses provider-neutral `CodeIntelligenceProvider` contracts.
   A provider declares its stable id, capabilities, and produces its own search
   section.
2. The bundled registry is built in the composition root and accepts constructor
   injection for tests. Its order is authoritative for public section order.
3. `unica.code.search` runs the bundled providers `rlm`, `bsl-analyzer`, and
   `git-grep`; ranks and optional scores remain local to a provider. Unica does
   not fuse, rerank, or deduplicate hits across sections.
4. Infrastructure adapters own subprocess commands, MCP sessions, index
   lifecycle, response parsing, and provider-specific deadlines. Application
   code does not read RLM SQLite files or know their schema.
5. A provider failure is represented in its section. A public search succeeds
   when at least one provider returns `ok` or `empty`; cancellation has priority
   and never returns a partial public result.
6. The public MCP boundary remains one server named `unica`. Provider selection
   is not a public tool argument, and `git-grep` is an internal search section.

## Неграницы

1. This ADR does not introduce dynamic loading or user configuration of third
   party providers.
2. This ADR does not change an upstream RLM API or add an RLM provider server.
3. This ADR does not compare relevance scores from different providers.

## Последствия

1. New code intelligence engines can be tested with fake providers without
   starting processes or creating indexes.
2. Adapter migrations are independently reviewable: the contract is stable
   before any individual engine is moved behind it.
3. Package and acceptance tests must continue proving the one-server public
   contract and the fixed three-section search response.

## Верификация

- [x] The application contract has stable provider ids, capabilities, request,
      section status, hits, and diagnostics.
- [x] The registry rejects duplicate ids and preserves injected search-provider
      order.
- [ ] The composition root registers RLM, bsl-analyzer, and git-grep adapters.
- [ ] The public coordinator runs all registered search providers in parallel.
