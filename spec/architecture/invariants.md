# Архитектурные инварианты

Этот документ фиксирует правила, которые должны оставаться верными при развитии
Unica. Если изменение нарушает инвариант, сначала нужен новый ADR, который явно
заменяет или уточняет текущее решение.

## Product Boundary

1. Unica is a Codex plugin for 1C:Enterprise developer workflows.
2. Public skills model developer operations, not infrastructure tools.
3. Low-level bundled tools must not become required LLM-visible knowledge.
4. The plugin must be usable from a generated marketplace package, not only from
   the source checkout.

## Public MCP Surface

1. The only public MCP server is `unica`.
2. `.mcp.json` must declare exactly one `mcpServers` entry.
3. `initialize` must return `serverInfo.name = "unica"`.
4. Public tools must use `unica.*` names.
5. Internal engines must not be exposed as separate MCP registrations.
6. Adding, removing, or renaming a public MCP tool requires tests and ADR sync.

## Skill Routing

1. Skills route through MCP `unica`.
2. Skills must not instruct the LLM to call internal adapter servers directly.
3. Skills must not use skill-local Python/PowerShell operation files as the
   target execution path.
4. Former script command semantics must be implemented inside native `unica.*`
   MCP tools. Reference scripts are allowed only under `tests/fixtures`.
5. For mutating operations, skills should keep dry-run unless the user explicitly
   requested mutation.

## Application Boundary

1. Application use cases own tool dispatch and domain event emission.
2. MCP transport maps protocol requests to application calls.
3. Infrastructure adapters must not bypass application cache/event handling.
4. `unica-coder` must not contain a runtime operation-file fallback for
   Python/PowerShell/Bash scripts.

## Cache And Workspace State

1. The orchestrator owns workspace state and cache invalidation.
2. Mutating operations emit domain events for cache impact.
3. `UNICA_CACHE_DIR` can override the default volatile cache root.
4. Dry-run operations report cache impact without writing cache state.
5. Applied mutations may update `WorkspaceStateRepository`.

## Workspace Source Sets

1. Source format is a property of a source-set, not of the whole workspace.
2. One source-set must not be treated as mixed-format; conflicting format
   markers inside one source-set make it invalid/ambiguous.
3. A workspace may contain several source-sets with different effective
   formats, such as an EDT configuration and platform XML external processors.
4. Native platform XML metadata operations must select a platform XML source-set
   before editing XML files.

## Project Discovery

1. `DiscoverExtensionPointsUseCase` owns discovery orchestration through six
   typed evidence ports; it must not parse display output, read adapter storage,
   or silently replace an unavailable provider with an unbounded scan.
2. Related artifacts, runtime flow edges, and actionable extension-point
   candidates are separate result domains. Lexical evidence alone is never
   actionable.
3. A negative proposal verdict requires complete, fresh evidence. Bounded,
   stale, unavailable, unsupported, or conflicting evidence produces
   `unknown`, never a fabricated contradiction.
4. Every conclusion keeps canonical identity, location, provider provenance,
   coverage, freshness, source fingerprint, and material checks. Stable output
   has no scalar confidence or domain-specific synonym dictionary.
5. `workspaceEpoch` is diagnostic-only. Receipt validity uses content
   fingerprints for every linked analysis and destination source-set and their
   composite.

## Discovery Receipts And Guard

1. A discovery receipt is server-owned investigation evidence, not user
   authorization; `dryRun: false` remains an independent requirement.
2. Receipt scope consists of atomic grants. Each grant binds the tool, exact
   target, mutation class, change kind, normalized output-affecting parameters,
   destination source-set, and exact allowed artifacts without cross-product
   expansion.
3. When an enforceable applied mutation presents a valid receipt, its exclusive
   receipt lease is acquired before the handler and remains held as a lease
   through handler execution, effect verification, and atomic receipt
   advancement or revocation. An observe/warn call allowed without a receipt
   has no receipt transition.
4. Dry-run never acquires or advances a receipt. Failed or out-of-scope writes
   cannot advance a receipt as if the planned mutation succeeded.
5. Discovery requirement and rollout mode are independent. Mode is server or
   workspace configuration; no mutation call can supply a bypass argument.
6. Support policy is evaluated before discovery policy, and dry-run remains
   available in every guard mode.

## Shadow Observations

1. A non-authoritative shadow observation is written only after the operation
   outcome is known. Journal failure cannot change the handler outcome, receipt
   decision, or configured rollout mode.
2. The store is an OS-locked, schema-versioned JSONL journal with bounded
   rollover, aggregate counters, and deterministic replay inputs.
3. Observation and replay records contain digests and policy predicates but
   must never contain task text or source text, raw mutation arguments,
   absolute paths, or unhashed artifact names.
4. Corrupt and unknown-schema records are reported and excluded rather than
   silently rewritten. Audit/replay is maintainer-only and is not a public MCP
   tool.

## Packaging

1. Generated binaries are not committed.
2. Packaged execution goes through checksum-verifying launchers.
3. The bundled public binary name is `unica`.
4. Generated package smoke must verify the packaged `.mcp.json`, not only source
   files.
