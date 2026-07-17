# 5. Представление строительных блоков

## Top Level

- `interfaces::mcp`: stdio MCP transport, JSON-RPC methods, tool list and call
  response mapping.
- `application`: `UnicaApplication`, `ToolSpec`, `ToolHandler`,
  `OperationResult`.
- `domain`: `WorkspaceContext`, `DomainEvent`, `CacheImpact`, `CacheReport`.
- `infrastructure`: command adapters, standards adapter, package launchers, and
  `WorkspaceStateRepository`.

## Domain Blocks

- `WorkspaceContext` discovers cwd, workspace root, cache root, and workspace
  epoch.
- `DomainEventKind` describes state-changing facts such as `FormChanged`,
  `BuildCompleted`, and `SourceSetChanged`.
- `CacheImpact` maps events to invalidated and eagerly refreshed cache names.

## Application Blocks

- `tools()` is the canonical public tool registry.
- `call_tool()` resolves dry-run semantics, workspace context, adapter dispatch,
  event emission, and cache reporting.
- Mutating tools default to `dryRun: true`.
- `DiscoverExtensionPointsUseCase` orchestrates typed evidence, graph assembly,
  proposal validation, and receipt eligibility without parsing display output.
- `DiscoveryGuard` resolves exact mutation targets and keeps support policy,
  discovery requirement, and configured rollout mode as separate decisions.

## Discovery Evidence Blocks

- `MetadataCatalogPort` exposes metadata identity and platform binding facts.
- `CodeSearchPort` exposes bounded lexical facts without promoting them to
  reachability.
- `DefinitionPort` exposes canonical symbol definitions and exact no-match
  coverage.
- `CallGraphPort` exposes typed call edges and explicit provider degradation.
- `FormInspectionPort` exposes form-command, action, and handler bindings.
- `SupportStatePort` exposes whether and where the selected mutation is
  supported.
- The evidence graph separates related artifacts, runtime flow edges, and
  actionable candidates. Proposal validation returns `supported`,
  `contradicted`, or `unknown` with material checks.

## Infrastructure Blocks

- Native operation handlers implement XML/DSL backend behavior inside
  `unica-coder`.
- `CliAdapter` invokes checksum-wrapped bundled tools.
- `StandardsAdapter` is the internal standards boundary and must become the real
  HTTP MCP client before closing the standards gap.
- `WorkspaceStateRepository` persists volatile cache state under the configured
  cache root.
- Source snapshot providers build source-format-aware manifests and a
  content-based composite fingerprint for analysis and destination source-sets.
- `DiscoveryReceiptRepository` stores server-owned rolling receipts and holds an
  exclusive receipt lease across the complete applied handler window.
- `ShadowObservationRepository` appends a non-authoritative,
  schema-versioned JSONL journal with bounded rollover and counters. Its
  sanitized comparison predicates support deterministic replay. Journal and
  replay records must never contain task text or source text.

## Target Native MCP Handlers

The target implementation for configuration, form, SKD, MXL, role, subsystem,
interface, and template operations is native Rust logic behind `unica.*` tools.
Python/PowerShell/Bash operation files must not remain as runtime building
blocks. Reference scripts belong in test fixtures only.
