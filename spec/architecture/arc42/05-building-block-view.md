# 5. Представление строительных блоков

## Top Level

- `interfaces::mcp`: stdio MCP transport, JSON-RPC methods, tool list and call
  response mapping.
- `application`: `UnicaApplication`, `ToolSpec`, `ToolHandler`,
  `OperationResult`.
- `domain`: `WorkspaceContext`, `DomainEvent`, `CacheImpact`, `CacheReport`.
- `infrastructure`: command adapters, standards adapter, package launchers, and
  `WorkspaceStateRepository`.
- `branched-development`: task state, canonical delta, merge decisions, lock
  planning, repository integration, archive, and recovery.

## Domain Blocks

- `WorkspaceContext` discovers cwd, workspace root, cache root, and workspace
  epoch.
- `DomainEventKind` describes state-changing facts such as `FormChanged`,
  `BuildCompleted`, and `SourceSetChanged`.
- `CacheImpact` maps events to invalidated and eagerly refreshed cache names.
- Branched domain models define phase transitions, canonical UUID/property
  deltas, ownership/reference closure, support layers, relevant anchors,
  compensated locks, and terminal proofs without filesystem/process access.

## Application Blocks

- `tools()` is the canonical public tool registry.
- `call_tool()` resolves dry-run semantics, workspace context, adapter dispatch,
  event emission, and cache reporting.
- Mutating tools with an honest preview default to `dryRun: true`; mutations
  without one expose a typed prepare/apply authorization boundary.
- Branched use cases expose an exhaustive execution policy per closed request
  variant (with a tool default only when uniform), require stable operation IDs
  for mutations, and extend results with typed workflow evidence/data.

## Infrastructure Blocks

- Native operation handlers implement XML/DSL backend behavior inside
  `unica-coder`.
- `CliAdapter` invokes checksum-wrapped bundled tools.
- `StandardsAdapter` is the internal standards boundary and must become the real
  HTTP MCP client before closing the standards gap.
- `WorkspaceStateRepository` persists volatile cache state under the configured
  cache root.
- `BranchedTaskRepository` persists schema-versioned operational records under
  the durable state root; it is not an extension of `WorkspaceStateRepository`.
- `BranchedTargetLocator` persists the project/target-to-state-root registration
  and start-attempt replay records under a non-overridable owner-only
  coordination root.
- `DesignerPort` is a typed platform-neutral boundary. Its OS-specific locator,
  process, encoding, service-message, and filesystem implementation lives under
  `infrastructure/platform/**`.
- `SecretResolver` resolves references only in memory. `TaskWorkPolicy` owns
  instance markers, containment, quarantine, and destructive cleanup guards.
- A detached branched-operation worker owns long `operationId`-bound contained
  or authoritative Designer calls across MCP disconnect and records effect
  barriers. Read-only inspections instead use bounded ephemeral processes and
  never create durable operation state.

## Target Native MCP Handlers

The target implementation for configuration, form, DCS, MXL, role, subsystem,
interface, and template operations is native Rust logic behind `unica.*` tools.
Python/PowerShell/Bash operation files must not remain as runtime building
blocks. Reference scripts belong in test fixtures only.

The pinned `v8-runner` remains an internal adapter for its existing runtime
contract; it is not the implementation of supported update or repository tools.
