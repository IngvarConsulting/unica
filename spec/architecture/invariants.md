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
5. For mutating operations with an honest preview, skills keep dry-run unless
   the user explicitly requested mutation. Operations without an honest preview
   use a typed prepare/apply boundary and explicit authorization; they must not
   fabricate a dry-run result.

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
4. Operations that support dry-run report cache impact without writing cache
   state.
5. Applied mutations may update `WorkspaceStateRepository`.

## Branched Development State And Safety

1. Configuration-repository workflow state is durable operational state, not
   volatile cache; it never depends on `.build/unica` or `UNICA_CACHE_DIR` for
   recovery.
2. Every mutating branched-development call has a caller-stable `operationId`;
   replay with different canonical input is rejected.
3. The original infobase remains bound to the same repository and never receives
   task XML sources.
4. Baselines are verified full distributions; CFU and ordinary-CF substitution
   are forbidden.
5. Supported update, main merge, repository mutation, rollback, and cleanup fail
   closed when the exact platform capability or postcondition is unproven.
6. Raw/public `-force`, supported-update/merge/commit/unlock force, implicit
   conflict defaults, automatic unresolved-reference clearing, and commit with
   `-keepLocked` are outside the automated workflow. The sole exception is the
   adapter-derived repository-update structural confirmation for an approved
   exact incoming add/delete plan with capability evidence.
7. Compensation releases only locks attributable to the current operation; an
   ambiguous or pre-existing lock is never released automatically.
8. Repository credentials are typed secrets and never enter public results,
   journals, retained logs, archives, or MCP stdout.
9. Cleanup targets only a marker-owned disposable instance root after canonical
   containment, nonce, symlink/reparse, Git/worktree, and terminal-state checks.
10. An implementation milestone cannot close issue #137 until the full
    modify/add/delete/reference/support/recovery acceptance matrix passes.
11. A non-overridable per-user target locator prevents state-root overrides
    from hiding unresolved tasks or start-attempt replay records.
12. Compatible general tools resolve an opaque `branchedTask` workspace ID;
    prompt-visible flows never pass the disposable path as `cwd`.
13. Repository-account identity has its own persistent reservation across
    different originals; exclusive same-user lock ownership is never shared by
    active tasks.
14. Any ordinary task mutation after verification atomically returns to
    `developing` and invalidates every descendant workflow proof; manual merge
    receipts are session-scoped and expire with sandbox recreation.
15. Compatible general-tool results recursively project structured and free-text
    values to workspace-relative names or registered artifact IDs; absolute
    disposable/state/coordination paths and path-bearing secrets never cross
    the MCP boundary.

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
5. Branched-development skills route only through public domain tools and must
   not expose Designer, raw repository flags, or `v8-runner` as workflow steps.
