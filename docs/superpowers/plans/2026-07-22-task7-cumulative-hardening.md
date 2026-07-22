# Task 7 Cumulative Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the four final bounded-work and cancellation findings in Task 7 without adding Task 8 behavior.

**Architecture:** Discovery-only project manifests, directory traversal, contained indexed-file reads, and lexical BSL work become explicitly bounded and cancellable. Resource exhaustion remains typed and produces deterministic partial provider outcomes; verified no-follow containment and public error identities remain unchanged.

**Tech Stack:** Rust 1.96, serde_yaml, SHA-256, verified platform handles, Cargo tests, Python MCP acceptance.

## Global Constraints

- Code/tests and package metadata outrank prose.
- Keep one public `unica` MCP server and the existing `unica.project.discover` surface.
- Do not modify Task 8 skill/package files.
- Write and witness focused RED tests before every production change.
- Cancellation wins over concurrent facility/resource errors.
- Run focused, full, formatting, strict Clippy, diff, and independent review gates.

---

### Task 1: Verified bounded discovery manifest

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/project_sources.rs`
- Modify: `crates/unica-coder/src/infrastructure/source_roots.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform/contained_file.rs` only if a reusable observer is required

**Interfaces:**
- Consumes: implicit discovery workspace root and `CancellationToken`.
- Produces: a discovery-only manifest declaration reader with a fixed byte limit, verified no-follow containment, chunk cancellation, and typed `DiscoveryError` mapping.

- [x] Add oversized, symlink, mid-read cancellation, normal implicit-selection, and stable-error tests.
- [x] Run focused tests and record the unsafe-reader failure boundary.
- [x] Add a discovery-only verified bounded manifest read and parse only its returned bytes.
- [x] Thread cancellation through implicit selection and preserve non-discovery map behavior.
- [x] Run focused project-source/source-root/application tests and commit.

### Task 2: Cancellable indexed BSL validation

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/discovery/bsl.rs`

**Interfaces:**
- Consumes: `DiscoveryQuery` cancellation during indexed source revalidation.
- Produces: contained reread/hash cancellation classified as provider `Failed(discovery_cancelled)`.

- [x] Add a deterministic mid-chunk cancellation observer test around indexed-file validation.
- [x] Run it against the non-cancellable validation boundary.
- [x] Pass the query to validation and use the cancellable contained-file API.
- [x] Run focused BSL/contained-file tests and commit.

### Task 3: Bounded inventory traversal work

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/discovery/inventory.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform/verified_directory.rs` only if the current bounded facade is insufficient
- Modify: `spec/architecture/extension-point-discovery.md`

**Interfaces:**
- Consumes: request `maxFiles`.
- Produces: a documented cumulative traversal-entry limit derived from `maxFiles`; N+1 exhaustion returns stable `Bounded` coverage while cancellation and no-follow checks remain intact.

- [x] Add deterministic irrelevant-fanout/nesting and cancellation tests with a low request limit.
- [x] Run focused inventory tests against the formerly unbounded pending/enumeration path.
- [x] Apply the cumulative bounded verified-directory facade to every traversed directory and cap pending work before insertion.
- [x] Map limit exhaustion to a stable traversal-bound diagnostic with prior verified coverage.
- [x] Document exact derivation/semantics, run platform/inventory tests, and commit.

### Task 4: Bounded lexical BSL work

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/discovery/bsl.rs`
- Modify: `spec/architecture/extension-point-discovery.md`

**Interfaces:**
- Consumes: normalized query terms and verified BSL bytes.
- Produces: deterministic bounded multi-pattern work, cancellation polls, deferred artifact cloning, and `Bounded` prior evidence on work exhaustion.

- [x] Add adversarial many-term/newline tests, mid-work cancellation, retained-prefix determinism, and exact-column regressions.
- [x] Run focused BSL tests against the formerly unbounded line-by-term work path.
- [x] Enforce one explicit deterministic comparison-work budget across the query.
- [x] Return typed lexical work exhaustion; preserve prior facts/files and clone artifacts only after confirmed matches.
- [x] Run BSL/use-case/MCP acceptance tests, document semantics, and commit.

### Task 5: Integration verification and review

**Files:**
- Modify: `.superpowers/sdd/task-7-report.md` (ignored evidence report)

**Interfaces:**
- Consumes: Tasks 1-4 commits.
- Produces: clean cumulative review and exact verification evidence.

- [x] Run source-root, contained-file, inventory, BSL, application, and MCP focused tests.
- [x] Run full `unica-coder` library tests, formatting, strict relevant Clippy, platform/product gates, and diff/status checks.
- [x] Append RED/GREEN evidence, SHAs, bounds, and counts to the Task 7 report.
- [x] Dispatch a fresh independent review; fix all Critical/Important findings with another TDD cycle.
- [x] Confirm tracked-clean status and no Task 8 changes.

### Task 6: Final Rust review follow-up

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/discovery/bsl.rs`
- Modify: `.superpowers/sdd/task-7-report.md` (ignored evidence report)

**Interfaces:**
- Consumes: parsed BSL method metadata and contained-file errors raised while revalidating an inventory-captured regular file.
- Produces: method identities allocated only for confirmed lexical matches, plus stable `Unavailable(bsl_index_stale)` results when the captured file is replaced by a link/reparse point or non-regular file.

- [x] Add an observable many-method regression proving absent terms construct zero method artifacts and matching lines construct only their needed method identity.
- [x] Run the artifact-allocation regressions RED against eager preallocation, then defer construction until a matched line has an owning parsed method.
- [x] Add classification and real directory/link replacement regressions for post-inventory file-kind changes.
- [x] Run the replacement regressions RED against `ContractViolation`, then map only symlink/reparse and non-regular replacements to typed staleness.
- [x] Run focused/full acceptance, strict Clippy, formatting, diff/status checks, update evidence, and self-review both commits.
