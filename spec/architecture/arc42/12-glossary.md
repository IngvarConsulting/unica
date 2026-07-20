# 12. Глоссарий

- Adapter: internal boundary that calls a bundled tool or remote endpoint.
- Cache impact: the set of cache names invalidated or refreshed by domain events.
- Capability row: retained proof that one exact host OS, 1C platform
  family/version, and locale passed every required real repository scenario.
- Canonical delta: semantic configuration change keyed by metadata UUID and
  property path, independent of raw XML file counts.
- Coordination locator: owner-only non-overridable record that binds canonical
  project/target identities to one durable state root and unresolved history.
- Distribution: full CF created by `/CreateDistributionFiles -cffile` and
  proven to establish vendor support when deployed.
- Instance ID: internal UUID used for one disposable task cycle and its paths;
  distinct from the external tracker task ID.
- Integration set: verified add/modify/delete repository object set bound to
  merge, validation, acquired-lock, and commit digests; it includes new objects
  that cannot themselves be locked before creation.
- Lock plan: explained closure of repository development objects needed for the
  canonical delta, ownership, additions/deletions, and references.
- Operation ID: caller-stable idempotency key bound to one canonical mutating
  request and durable result.
- Project ID: stable UUID in local Unica configuration used to locate durable
  task state across project relocation; it is not the mutable workspace epoch.
- Repository transport: file or server access topology, included in a capability
  row independently of original-infobase kind.
- Original infobase: developer infobase that remains connected to the main 1C
  configuration repository throughout branched development.
- Task infobase: owned disposable local File IB used for one task cycle.
- Branched task context: original project `cwd` plus opaque task/workspace IDs
  resolved internally to the owned disposable workspace by compatible tools.
- Unknown effect: a journal state where an external platform mutation may have
  occurred but its postcondition is not proven; only reconciliation may proceed.
- Domain event: a typed fact emitted by an operation, for example `FormChanged`.
- Reference operation script: Python/PowerShell/Bash donor model kept under
  `tests/fixtures` for parity tests. It is not a runtime backend.
- MCP: Model Context Protocol.
- Orchestrator: the Rust `unica` server that owns public tool dispatch and
  cache/state coordination.
- Public MCP server: the only MCP server visible to LLM through `.mcp.json`.
- Skill: Codex operation instruction under `plugins/unica/skills`.
- Workspace epoch: lightweight fingerprint used to associate cache state with
  the current workspace.
