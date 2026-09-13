# P0 RC/package proof implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a dry, machine-readable RC/package proof contour for v0.13 that validates the packaged native/compatibility wire surfaces, rejects the v0.12.3 legacy tool baseline, records install/update/offline/restart/rollback outcomes, and cannot publish or bump versions.

**Architecture:** Keep the existing package, wire-probe, release-assessment, and published-asset verifiers as focused producers. Add one release-proof coordinator that consumes those producer outputs and a checked-in v0.12.3 baseline, validates the complete P0 matrix, and emits a deterministic JSON/Markdown verdict. Wire it into the CI package path as a dry gate; tag publication remains conditional on a real release tag and the marketplace workflow remains the only promotion path.

**Tech Stack:** Python 3.12 standard library, `unittest`, GitHub Actions YAML, existing Unica package/probe/assessment scripts.

**Spec:** GitHub issue #581, section “P0 — RC/package proof без переключения версии”.

## Global Constraints

- Keep the public server identity `unica` and the exact native 8 / compatibility 11 tool surfaces.
- Do not add tools, run intents, `query.execute`, skills, or incompatible package keys.
- Do not change current package versions, create tags, publish release assets, or promote a marketplace catalog.
- Use the checked-in observed v0.12.3 baseline as immutable input; a new baseline requires a new versioned fixture.
- Keep P0 changes inside its exclusive ownership paths; shared production wiring belongs to J0.
- Every new behavior gets a RED test before implementation and a focused GREEN verification.

---

### Task 1: Establish P0 proof contract and immutable baseline inputs

**Files:**
- Create: `scripts/ci/release-proof.py`
- Create: `tests/ci/test_release_proof.py`
- Modify: `scripts/ci/classify-workflow-changes.py`
- Modify: `tests/ci/test_classify_workflow_changes.py`

**Interfaces:**
- `release-proof.py` consumes package/probe/assessment JSON files plus the v0.12.3 baseline JSON and writes a machine-readable proof report and Markdown summary.
- The proof report exposes exact surface counts, legacy-overlap result, named lifecycle scenario outcomes, prerelease promotion result, and mutation guard outcomes.

- [x] **Step 1: Write the failing tests** for exact 8/11 surface validation, legacy overlap rejection, missing lifecycle scenario rejection, prerelease non-promotion, and dry-mode mutation rejection.
- [x] **Step 2:** Run `python3.12 -m unittest tests/ci/test_release_proof.py -v` and confirm the new coordinator is missing or rejects the required contract for the expected reason.
- [x] **Step 3:** Implement the smallest typed parser/validator and deterministic report writer. The validator must reject malformed or missing evidence rather than infer success.
- [x] **Step 4:** Run the focused tests and confirm they pass.
- [x] **Step 5:** Add the new script and tests to the CI classification contour and prove the classifier routes them through package/release/CI checks.

### Task 2: Make package and wire outputs consumable by the coordinator

**Files:**
- Modify: `scripts/ci/package-unica-plugin.py`
- Modify: `scripts/ci/probe-unica-wire.py`
- Modify: `tests/ci/test_package_unica_plugin.py`
- Modify: `tests/ci/test_probe_unica_wire.py`

**Interfaces:**
- Wire evidence carries explicit profile identity (`native` or `compatibility`); package output carries immutable package/source identities needed by the proof coordinator.
- Wire probe can run against the built package entrypoint for both profiles and preserves the existing deterministic output shape when no profile is requested.

- [x] **Step 1: Add RED tests** showing that profile identity and package identity are absent or ambiguous in the current evidence path.
- [x] **Step 2:** Run only those tests and verify the failure is about missing P0 evidence, not a fixture/setup error.
- [x] **Step 3:** Add minimal output metadata and CLI plumbing without changing the public MCP surface.
- [x] **Step 4:** Re-run package and wire focused suites, including malformed/duplicate/pagination protections.

### Task 3: Add machine-readable lifecycle/readiness scenarios

**Files:**
- Modify: `scripts/ci/release-assessment.py`
- Modify: `scripts/ci/verify-release-assets.py`
- Modify: `tests/ci/test_release_assessment.py`
- Modify: `tests/ci/test_verify_release_assets.py`

**Interfaces:**
- Assessment evidence names separate `fresh_install`, `upgrade`, `offline_prefetch`, `restart`, and `rollback` scenarios with explicit `status`, `supported`, and `evidence` fields.
- Asset verification preserves byte-level and immutable identity checks and reports typed outcomes suitable for aggregation.

- [x] **Step 1: Add RED tests** requiring all five lifecycle scenario keys and rejecting a single overloaded “install passed” result.
- [x] **Step 2:** Run the focused tests and verify the current assessment does not satisfy the complete matrix.
- [x] **Step 3:** Extend the existing report schema and validators with explicit scenario outcomes; unsupported local-only execution must be typed and must not become success.
- [x] **Step 4:** Run the full assessment and asset focused suites.

### Task 4: Wire the dry P0 gate into CI without enabling publication

**Files:**
- Modify: `.github/workflows/unica-plugin-release.yml`
- Modify: `tests/ci/test_unica_workflow.py`
- Modify: `tests/ci/test_evaluate_ci_gate.py`

**Interfaces:**
- Pull requests and manual dry runs execute the P0 coordinator against main-version package evidence.
- Tag-only jobs remain the sole path that uploads release assets; P0 itself has no write permissions and no version/tag/publish step.

- [x] **Step 1: Add RED workflow-contract tests** for a dry P0 job, explicit no-publish/no-version-bump guard, and prerelease handling.
- [x] **Step 2:** Run the workflow contract tests and verify the current workflow lacks the required gate.
- [x] **Step 3:** Add the minimal CI job/steps and aggregate it in `unica-ci`; keep tag publication and marketplace promotion dependencies unchanged.
- [x] **Step 4:** Run workflow, classification, aggregate-gate, package, probe, assessment, and asset suites together.

### Task 5: Final P0 verification and handoff evidence

**Files:**
- Modify: `docs/release-runbook.md` only if the new dry proof command needs a documented invocation.
- Modify: `tests/ci/test_release_proof.py` only for final contract coverage.

- [x] **Step 1:** Run Python compilation for all changed CI scripts/tests.
- [x] **Step 2:** Run the full `tests/ci` suite and the affected architecture/source guards.
- [x] **Step 3:** Run the dry proof command on the current `0.12.0` main-version package fixtures and inspect the generated JSON/Markdown verdict.
- [x] **Step 4:** Verify `git diff` contains no version bump, tag, release upload, marketplace write, or shared J0 wiring.
- [x] **Step 5:** Record the focused/full test evidence and P0 status in the issue/PR handoff.
