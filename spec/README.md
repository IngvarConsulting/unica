# Spec Guide

`spec/` stores the active internal truth layer for Unica planning, architecture
rules, ADRs, and acceptance.

## Active Entry Points

- Implementation task list: open implementation gaps only.
- `decisions/README.md`: accepted architecture decisions and their owning ADR files.
- `architecture/invariants.md`: non-negotiable rules that changes must preserve.
- `architecture/change-checklist.md`: required sync checklist for MCP, skill, cache, and package changes.
- `architecture/extension-point-discovery.md`: accepted architecture, public
  contract, rollout, acceptance, and migration design for Project Discovery.
- `architecture/arc42/`: detailed architecture map for maintainers and agentic work.
- `acceptance/unica-mcp-validation.md`: active acceptance and smoke plan for the single MCP contract.

## Project Boundary

Unica is a Codex plugin for 1C:Enterprise development workflows. Its public AI
surface is the single MCP server `unica`. Build/runtime tools, BSL analysis,
standards lookup, and XML/DSL operations are owned by the orchestrator. Current
skill-local Python/PowerShell operation files are migration debt: their command
semantics must move into native MCP tools and the files must be removed from
skills after parity.

## Usage Rule

If a statement here conflicts with current code, tests, `.mcp.json`, or plugin
packaging metadata, trust the current implementation first and then update this
spec layer.
