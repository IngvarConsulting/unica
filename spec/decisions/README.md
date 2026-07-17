# Architecture Decision Records

This directory contains accepted architecture decisions for Unica.

## Accepted ADRs

- [ADR-0001: Единый публичный MCP `unica`](0001-edinyy-publichnyy-mcp-unica.md)
- [ADR-0002: Транспортно-нейтральный application layer](0002-transportno-neytralnyy-application-layer.md)
- [ADR-0003: Cache и workspace state принадлежат orchestrator](0003-cache-i-workspace-state-prinadlezhat-orchestratoru.md)
- [ADR-0004: Operation scripts are reference-only, not runtime backends](0004-legacy-skill-scripts-are-migration-debt.md)
- [ADR-0005: Skills route только через `unica`](0005-skills-routyatsya-tolko-cherez-unica.md)
- [ADR-0006: Workspace-scoped internal services](0006-workspace-scoped-internal-services.md)
- [ADR-0007: Script-backed utility skill exceptions](0007-script-backed-utility-skill-exceptions.md)
- [ADR-0008: Project Discovery and Discovery Receipts](0008-project-discovery-and-discovery-receipts.md)

## ADR Status Values

- `accepted`: active decision.
- `superseded`: replaced by a newer ADR.
- `proposed`: not yet active.

When code changes violate an accepted ADR, update or supersede the ADR in the
same change set.
