# View Workspace Bootstrap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `unica.view {}` the first-call workspace discovery path and make the canonical MCP surface self-explanatory.

**Architecture:** A bounded read-only bootstrap is handled inside the daemon before source-set actor admission, because actor identity cannot exist in an empty workspace. Addressed reads retain the existing actor-owned path. The MCP catalog owns concise descriptions and the initialize hint.

**Tech Stack:** Rust, rmcp, serde_json, existing workspace/source discovery, Cargo tests, Python architecture guards.

**Spec:** `docs/design/2026-09-01-view-workspace-bootstrap-design.md`

## Global Constraints

- Keep exactly eight native and eleven compatibility `unica.*` tools.
- `unica.view {}` is read-only and must work without `v8project.yaml` or source sets.
- Never hide an invalid `v8project.yaml` behind autodetection.
- Do not advertise unimplemented `source.create` or `source.attach` as a next action.
- Keep each tool description below 2 KiB and compatibility `tools/list` below 16 KiB.

---

### Task 1: Wire contract and discovery descriptions

**Files:**
- Modify: `crates/unica-coder/src/application/v13/tool_catalog.rs`
- Modify: `crates/unica-coder/src/interfaces/mcp.rs`

**Interfaces:**
- Produces: `V13ToolContract.description: &'static str`
- Produces: `CANONICAL_INSTRUCTIONS: &str`

- [x] Write catalog tests requiring optional `view.at`, non-empty tool/argument descriptions, and byte budgets.
- [x] Run `cargo test -p unica-coder application::v13::tool_catalog::tests` and observe the expected failures.
- [x] Add concise descriptions to the eight tools and every input property, then project them into MCP `Tool.description`.
- [x] Replace the no-instructions baseline with canonical bootstrap instructions and append startup notice without losing either message.
- [x] Run the focused catalog and MCP initialize tests until green.

### Task 2: Bootstrap result model

**Files:**
- Create: `crates/unica-coder/src/infrastructure/daemon/v13_workspace_bootstrap.rs`
- Modify: `crates/unica-coder/src/infrastructure/daemon/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/daemon/server.rs`

**Interfaces:**
- Produces: `execute_view_bootstrap(request: &InvocationRequest, deadline: &InvocationResponseDeadline) -> Option<DomainResult>`
- Consumes: `discover_workspace`, `discover_project_source_map_controlled`

- [x] Add daemon tests for configured, autodetected, empty, and malformed workspaces; assert no file changes.
- [x] Run each focused test and confirm it fails because empty workspaces are rejected by actor admission.
- [x] Implement the pre-admission read route and stable `data.config`, `data.sourceSets`, readiness, `setup`, diagnostics, and `next` shapes.
- [x] Reject bootstrap-only pagination/filter fields without `at`, while preserving addressed reads.
- [x] Run the focused daemon tests until green.

### Task 3: Architecture records and generated surface

**Files:**
- Create: `arch/decisions/2026-09-01-view-workspace-bootstrap.md`
- Create: `arch/invariants/INV.SURFACE.WORKSPACE-BOOTSTRAP.md`
- Modify: `arch/contracts/CTR.WIRE.TOOL-SURFACE.md`
- Modify: `arch/invariants/INV.SURFACE.ARGUMENTS-DESCRIBED.md`
- Modify: `arch/invariants/INV.SURFACE.PROJECT-READINESS.md`
- Regenerate: `arch/index.md`
- Regenerate: tool-surface artifacts selected by `scripts/ci/generate-tool-surface.py`

**Interfaces:**
- Produces: `DEC.2026-09-01.VIEW-WORKSPACE-BOOTSTRAP`
- Produces: `INV.SURFACE.WORKSPACE-BOOTSTRAP`

- [x] Add the decision and invariant with exact named tests as evidence.
- [x] Move changed contract/rules to the new decision and bump `CTR.WIRE.TOOL-SURFACE` to version 3.
- [x] Regenerate architecture index and tool-surface artifacts with repository scripts.
- [x] Run architecture and generated-artifact guards.

### Task 4: End-to-end verification and PR

**Files:**
- Modify only files required by failing verification.

**Interfaces:**
- Consumes: packaged/compiled stdio server and actual `tools/list` JSON.
- Produces: independent PR based on `main`.

- [x] Run focused Rust tests, `cargo test -p unica-coder`, and relevant Python CI tests.
- [x] Build the package-selected MCP binary and call initialize, tools/list, and `unica.view {}` over stdio in an empty temporary workspace.
- [x] Measure compact tools/list bytes and o200k/cl100k tokens; record the numbers in the PR.
- [x] Review the diff for unrelated changes, commit, push, and open a PR with base `main`.
