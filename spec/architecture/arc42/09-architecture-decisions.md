# 9. Архитектурные решения

Accepted decisions live in `spec/decisions`.

Current decisions:

- ADR-0001: one public MCP `unica`;
- ADR-0002: transport-neutral application layer;
- ADR-0003: orchestrator-owned cache and workspace state;
- ADR-0004: operation scripts as reference-only, not runtime backends;
- ADR-0005: skills route only through `unica`;
- ADR-0006: workspace-scoped internal services for warm analyzer/index state;
- ADR-0009: OS-specific code behind infrastructure platform facades.

If a future change adds, removes, or renames a public MCP tool, changes cache
ownership, or exposes an internal engine directly, it must update or supersede
the relevant ADR.
