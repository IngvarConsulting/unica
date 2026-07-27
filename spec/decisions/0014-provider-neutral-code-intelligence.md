# ADR-0014: Provider-neutral code intelligence

- Статус: `accepted`
- Дата: `2026-07-26`

## Контекст

Code intelligence combines independent engines with different transport and
runtime requirements: RLM, `bsl-analyzer`, and fixed-string `git grep`.
Every request also selects an effective workspace and source root. A linked
worktree has its own workspace and cache identity even when most files are
shared with another checkout.

Current upstream index APIs operate on one complete source root. They do not
expose a supported base-index plus worktree-delta contract. Binding the provider
contract either to those storage details or to one runtime hosting model would
mix public search semantics with replaceable infrastructure.

## Решение

1. Application code uses provider-neutral `CodeIntelligenceProvider` contracts.
   A provider declares its stable id and executable capabilities, produces
   provider-local search data, and handles typed read requests for any
   definition, outline, or object-profile capabilities it advertises.
2. The bundled registry is built in the composition root and accepts constructor
   injection for tests. Its order is authoritative for public section order.
3. `unica.code.search` runs the bundled providers `rlm`, `bsl-analyzer`, and
   `git-grep`; ranks and optional scores remain local to a provider. Unica does
   not fuse, rerank, or deduplicate hits across sections.
4. Application code validates the public request, resolves the effective source
   root once, starts providers concurrently, applies the public and
   provider-specific budgets, bounds admitted workers per provider, and gives
   cancellation priority through a token linked to the MCP request. Every
   provider receives the same typed workspace and source-root identity.
5. The provider contract is runtime-hosting-neutral. An infrastructure provider
   implementation owns provider-specific invocation, response parsing, and
   state transitions. Its runtime may live in the main MCP process or behind a
   workspace service; this ADR does not require either hosting model or
   cross-chat reuse.
6. Index and session state are isolated by normalized
   `workspaceRoot + sourceRoot`. A linked worktree therefore has an independent
   provider state. Within that identity, a provider may use its supported
   in-place update operation.
7. Unica does not read, copy, or merge RLM SQLite files. Cross-worktree shared
   indexes, base-plus-delta indexes, and content-addressed index fragments
   require a separate measured decision and a supported provider API.
8. A provider failure is represented in its section. A public search succeeds
   when at least one provider returns `ok` or `empty`; cancellation has priority
   and never returns a partial public result.
9. The public MCP boundary remains one server named `unica`. Provider selection
   is not a public tool argument, and `git-grep` is an internal search section.

## Неграницы

1. This ADR does not introduce dynamic loading or user configuration of third
   party providers.
2. This ADR does not change an upstream RLM API or add an RLM provider server.
3. This ADR does not compare relevance scores from different providers.
4. This ADR does not introduce cross-worktree index reuse, RLM SQLite cloning,
   or a main-branch index with worktree overlays.
5. This ADR does not require provider runtimes to use a workspace service or
   promise cross-chat process reuse.

## Последствия

1. New code intelligence engines can be tested with fake providers without
   starting processes or creating indexes.
2. Adapter migrations are independently reviewable: the contract is stable
   before any individual engine is moved behind it.
3. A new worktree may require its own lazy index build. Repeated changes within
   that worktree may use a provider-supported update.
4. While one provider is building or unavailable, successful providers remain
   useful through the partial-result contract.
5. Runtime hosting can change without changing the application contract.
6. A future shared-index proposal must include measured build and update costs,
   typical worktree lifetime and reuse, invalidation semantics, and a supported
   provider composition API.
7. Package and acceptance tests must continue proving the one-server public
   contract and the fixed three-section search response.

## Верификация

- [x] The application request carries one resolved workspace and source-root
      identity, and every provider receives that same identity.
- [x] The registry rejects duplicate ids and preserves injected search-provider
      order.
- [x] Every advertised read capability resolves to an executable provider
      through the same registry; application routing does not downcast or call
      a concrete RLM provider.
- [x] Provider and registry tests use fake providers without starting a process
      or workspace service.
- [x] Linked worktrees receive independent workspace, cache, and service
      identities.
- [x] The composition root registers RLM, bsl-analyzer, and git-grep adapters.
- [x] The public coordinator runs all registered search providers in parallel.
- [x] Production provider code uses supported upstream interfaces and does not
      read, copy, or merge RLM SQLite files.
- [x] Rust acceptance tests and the blocking packaged release assessment prove
      the fixed three-section response; package smoke proves the single public
      MCP server.

Implementation evidence:

- `domain/code_intelligence.rs` owns the provider-neutral request, context,
  registry, section, and result contracts.
- `application/code_intelligence.rs` starts owned provider workers before
  waiting, enforces the public and provider deadlines even for a
  non-cooperative provider, bounds retained workers through per-provider
  admission with RAII cleanup, registers every worker handle for the MCP EOF
  shutdown grace shared with active tool calls under one aggregate deadline,
  restores registry order, isolates failures, and applies the partial-success
  and linked-cancellation rules.
- `infrastructure/code_intelligence.rs` contains the three provider adapters;
  its RLM adapter implements the typed read SPI, while
  `infrastructure/rlm_navigation.rs` keeps definition, outline, and metadata
  profile on the same supported persistent RLM MCP API.
- `workspace_services.rs` owns the reusable RLM logical session
  (`rlm_start`/`rlm_execute`/`rlm_end`) and the reusable analyzer transport.
- `issue_89_workspace_service.rs` proves worktree/source-root selection,
  cancellation recovery, persistent RLM reuse, and the ordered
  `rlm`/`bsl-analyzer`/`git-grep` public response.
- `plugins/unica/third-party/tools.lock.json` pins the verified unmodified RLM
  v1.29.1 source at
  `8bc6e9fc83b522f9a79eab3193eb13fc2cecb8ed` and the immutable
  `rlm-tools-bsl-v1.29.1-build.2` platform assets.
- `check-tool-contracts.py`, `test_product_contracts.py`, and the packaged MCP
  smoke prove executable compatibility without importing the private RLM
  SQLite schema; the blocking release assessment validates the exact
  `rlm`/`bsl-analyzer`/`git-grep` section order and shape.
- The extracted `darwin-arm64` runtime was exercised against a real indexed
  configuration. The cold request returned ordered `rlm` (`ok`),
  `bsl-analyzer` (`unavailable`), and `git-grep` (`ok`) sections while the
  analyzer index warmed; the repeated request observed all three providers as
  `ok`. Definition and outline completed through the persistent RLM MCP API.
- `docs/superpowers/plans/2026-07-27-rlm-integration-api-issue-draft.md`
  records the post-implementation upstream API audit; it is a review draft and
  is not an accepted Unica architecture dependency.
