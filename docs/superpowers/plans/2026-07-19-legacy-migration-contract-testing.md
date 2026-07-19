# Legacy Migration Contract Testing Implementation Plan

> Historical: this plan is preserved as execution context. Current source of truth is code/tests/package metadata, then `spec/`, not this plan.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make compatibility with legacy Codex CLI JSON contracts and upgrades from the previous stable Unica release an executable release requirement.

**Architecture:** Keep deterministic, sanitized outputs captured from real Codex releases as Rust contract fixtures and parse them through one compatibility boundary before migration classification. Add a Windows pre-publication job that runs the candidate preflight against the pinned old CLI and previous published Unica, plus an independent scheduled/manual workflow that exercises the promoted stable marketplace end to end without touching the runner profile. The split avoids a release dependency cycle: the source workflow publishes runtime assets before the marketplace workflow can stage and promote that candidate.

**Tech Stack:** Rust/Serde integration tests, Python CI contract tests, PowerShell 5.1, GitHub Actions, GitHub release assets.

## Global Constraints

- Fix the JSON compatibility boundary rather than bypassing preflight.
- Keep one public MCP server named `unica` and use only public `codex plugin` commands for migration.
- Fixtures must be captured from a named official Codex release and contain no user paths, credentials, or mutable machine data.
- Real upgrade verification uses isolated temporary state and the published previous-stable Unica asset.
- Candidate preflight must fail before publication when the supported legacy contract cannot be classified.
- Full stable migration must fail its scheduled/manual workflow when runtime, MCP, prompt, or idempotency verification fails.

---

### Task 1: Legacy Codex JSON contract fixtures

**Files:**
- Create: `crates/unica-bootstrap/tests/fixtures/codex-0.145.0-alpha.18/marketplaces-local.json`
- Create: `crates/unica-bootstrap/tests/fixtures/codex-0.145.0-alpha.18/plugins-installed.json`
- Create: `crates/unica-bootstrap/tests/fixtures/codex-0.145.0-alpha.18/metadata.json`
- Modify: `crates/unica-bootstrap/tests/migration_contract.rs`
- Modify: `crates/unica-bootstrap/src/codex.rs`

**Interfaces:**
- Consumes: the exact output shapes of `codex plugin marketplace list --json` and `codex plugin list --available --json` from `codex-cli 0.145.0-alpha.18`.
- Produces: `MarketplaceList` and `PluginList` values normalized into the existing migration classifier.

- [x] **Step 1: Capture and sanitize the real legacy outputs**

Run the official CLI against a temporary `CODEX_HOME`, install the published v0.6.1 local marketplace there, replace the temporary root with `${CODEX_HOME}`, and record the exact commands and CLI version in `metadata.json`.

- [x] **Step 2: Write the failing parser and classification tests**

Add tests that deserialize both captured files, assert the legacy marketplace source and installed Unica identity, and pass them to `classify_discovery` expecting removal of the local marketplace/plugin and installation of the canonical pair.

- [x] **Step 3: Verify RED**

Run: `cargo test -p unica-bootstrap parses_codex_0_145 -- --nocapture`

Expected: FAIL because `MarketplaceRecord.marketplace_source` currently requires a field absent from the captured legacy output.

- [x] **Step 4: Normalize both supported schemas at the discovery boundary**

Implement custom deserialization or an untagged wire representation in `codex.rs` that accepts the captured legacy source fields and the current nested `marketplaceSource`, but still rejects records without enough source identity for ownership decisions.

- [x] **Step 5: Verify GREEN and regression coverage**

Run: `cargo test -p unica-bootstrap --test migration_contract -- --test-threads=1`

Expected: all migration contract tests pass, including the captured legacy contract.

### Task 2: Contract capture guardrail

**Files:**
- Create: `scripts/ci/capture-codex-contract.py`
- Create: `tests/ci/test_capture_codex_contract.py`

**Interfaces:**
- Consumes: `--codex PATH`, `--codex-home PATH`, `--expected-version VERSION`, `--output-dir PATH`, and JSON emitted by the two public Codex discovery commands.
- Produces: deterministic sanitized fixtures plus metadata containing the exact CLI version and commands.

- [x] **Step 1: Write failing Python tests**

Use a fake executable that emits paths under an isolated home and assert the capture script replaces only that root with `${CODEX_HOME}`, sorts JSON keys, records both commands, rejects malformed JSON, and never writes partial output.

- [x] **Step 2: Verify RED**

Run: `python3.12 -m unittest tests.ci.test_capture_codex_contract`

Expected: FAIL because the capture script does not exist.

- [x] **Step 3: Implement the capture script**

Run the supplied CLI without a shell, validate the version and JSON outputs, recursively sanitize the exact isolated home, and publish files by renaming a completed temporary directory.

- [x] **Step 4: Verify GREEN**

Run: `python3.12 -m unittest tests.ci.test_capture_codex_contract`

Expected: all capture tests pass.

### Task 3: Real previous-stable Windows upgrade release gate

**Files:**
- Create: `scripts/ci/test-unica-upgrade.ps1`
- Create: `tests/ci/test_unica_upgrade_script.py`
- Create: `.github/workflows/unica-legacy-migration.yml`
- Modify: `.github/workflows/unica-plugin-release.yml`
- Modify: `tests/ci/test_unica_workflow.py`

**Interfaces:**
- Consumes: official Codex release `rust-v0.145.0-alpha.18`, published Unica v0.6.1 Windows ZIP, and the candidate thin marketplace artifact.
- Produces: a nonzero result on preflight/migration/runtime/MCP/idempotency failure and a JSON proof record on success; `Preflight` mode runs before source asset publication and `Full` mode runs against the promoted stable marketplace in a separate workflow.

- [x] **Step 1: Write failing static workflow/script contract tests**

Assert the script requires explicit paths, uses a temporary `CODEX_HOME`, verifies `codex --version`, seeds the published v0.6.1 marketplace, and always runs candidate `migrate-preflight`. In `Full` mode it also runs `migrate`, checks the resulting version/registration, and reruns preflight for idempotency. Assert the source release workflow downloads the pinned official Codex binary and previous release asset and makes preflight a dependency of publication. Assert the separate stable workflow resolves the promoted catalog ref and runs `Full` on a schedule or explicit dispatch.

- [x] **Step 2: Verify RED**

Run: `python3.12 -m unittest tests.ci.test_unica_upgrade_script tests.ci.test_unica_workflow`

Expected: FAIL because the script and workflow job do not exist.

- [x] **Step 3: Implement the isolated PowerShell harness**

Use only caller-provided artifact paths, verify their versions and manifests, invoke the public legacy installer state plus candidate native migrator, and emit a bounded JSON report without host paths or credentials.

- [x] **Step 4: Add the Windows release job**

In the source workflow, download the exact official CLI archive and previous-stable Unica asset plus the candidate thin package, invoke `Preflight`, and add that job to `publish-release-assets.needs`. In the separate stable workflow, clone the canonical marketplace, resolve and checkout the immutable ref named by its stable catalog, invoke `Full` in a fresh isolated home, and upload its report.

- [x] **Step 5: Verify GREEN**

Run: `python3.12 -m unittest tests.ci.test_unica_upgrade_script tests.ci.test_unica_workflow`

Expected: all workflow and harness contract tests pass.

### Task 4: Full validation and PR publication

**Files:**
- Modify: only files listed above plus this plan.

**Interfaces:**
- Consumes: completed Tasks 1-3.
- Produces: a draft GitHub PR against `main` with the validation evidence.

- [x] **Step 1: Run focused verification**

Run: `cargo test -p unica-bootstrap -- --test-threads=1` and `python3.12 -m unittest tests.ci.test_capture_codex_contract tests.ci.test_unica_upgrade_script tests.ci.test_install_unica_script tests.ci.test_unica_workflow`.

- [x] **Step 2: Run repository guardrails**

Run: `cargo fmt --all -- --check`, `cargo clippy -p unica-bootstrap --all-targets --all-features -- -D warnings`, `python3.12 -m unittest discover -s tests/ci`, and `git diff --check`.

- [ ] **Step 3: Review scope and commit**

Inspect `git status --short` and `git diff`; stage only the planned files and commit with `test: verify legacy Unica migrations`.

- [ ] **Step 4: Push and open a draft PR**

Push `codex/test-legacy-migrations` to `origin` and create a draft PR against `main` describing the root cause, compatibility matrix, and exact validation performed.
