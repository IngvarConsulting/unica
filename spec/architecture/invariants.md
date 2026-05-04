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
   MCP tools before the corresponding files are removed.
5. For mutating operations, skills should keep dry-run unless the user explicitly
   requested mutation.

## Application Boundary

1. Application use cases own tool dispatch and domain event emission.
2. MCP transport maps protocol requests to application calls.
3. Infrastructure adapters must not bypass application cache/event handling.
4. Skill-local operation files are migration debt and must not become accepted
   architecture.

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

## Packaging

1. Generated binaries are not committed.
2. Packaged execution goes through checksum-verifying launchers.
3. The bundled public binary name is `unica`.
4. Generated package smoke must verify the packaged `.mcp.json`, not only source
   files.
