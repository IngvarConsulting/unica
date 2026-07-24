# Updatable Donor Parity Relations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace category-only script parity with an updatable, content-bound donor snapshot and reviewed per-case relations while keeping adapted Unica Python models separate.

**Architecture:** Keep locally adapted Python implementations as Unica-owned migration-equivalence models. Add a pristine selected `cc-1c-skills` snapshot, a baseline hash manifest, a relation registry with exact observations, and a two-phase maintainer refresh command. Normal CI remains offline; only explicit refresh preparation fetches upstream.

**Tech Stack:** Python 3.12 `unittest`, JSON manifests, Git CLI, existing Rust MCP binary and parity harness.

## Global Constraints

- `plugins/unica/provenance/skill-upstreams.json` remains the accepted donor-baseline source of truth.
- A floating donor `main` ref is never executed by normal CI.
- Donor snapshot bytes change only through a reviewed donor-refresh artifact.
- Adapted Unica reference models are not described as unchanged donor scripts.
- Changed donor content invalidates its relation; unchanged content carries the relation forward.
- `donor_ahead` is reviewed and non-blocking unless it exposes a safety regression, loss of shared behavior, or platform contradiction.
- Category-only expected-gap entries are removed.
- Native `unica.*` operations must continue rejecting script fallback.
- Donor scripts remain test-only and never enter the packaged plugin.

---

## File Structure

- Create `scripts/ci/donor_parity_contract.py`: JSON loading, path safety, hashing, case dependency digesting, observation fingerprints, baseline validation, and relation validation.
- Create `scripts/ci/refresh-cc-1c-parity.py`: explicit `prepare` and `apply` refresh phases.
- Create `tests/ci/test_donor_parity_contract.py`: focused contract and integrity tests.
- Create `tests/ci/test_refresh_cc_1c_parity.py`: synthetic-upstream refresh tests.
- Create `tests/fixtures/unica_mcp_script_parity/donor-baseline.json`: exact accepted files, commits, and hashes.
- Create `tests/fixtures/unica_mcp_script_parity/donor-relations.json`: one reviewed relation per donor case.
- Create `plugins/unica/provenance/reviews/2026-07-24-cc-1c-parity-refresh.json`: initial accepted refresh record.
- Move `tests/fixtures/unica_mcp_script_parity/reference_skills/` to `tests/fixtures/unica_mcp_script_parity/unica_reference_models/`.
- Create `tests/fixtures/unica_mcp_script_parity/cc-1c-skills/skills/`: pristine selected donor scripts.
- Modify `tests/ci/test_unica_mcp_script_parity.py`: separate Unica model equivalence from donor relations and consume JSON relations.
- Modify `tests/ci/test_skill_provenance.py`: enforce model attribution, baseline synchronization, and refresh review.
- Modify `plugins/unica/provenance/skill-upstreams.json`: watch selected donor case scopes and record accepted commits.
- Modify `spec/decisions/0004-legacy-skill-scripts-are-migration-debt.md`: distinguish Unica models from donor snapshots.

---

### Task 1: Separate adapted Unica models from the donor snapshot

**Files:**
- Move: `tests/fixtures/unica_mcp_script_parity/reference_skills/` → `tests/fixtures/unica_mcp_script_parity/unica_reference_models/`
- Modify: `tests/ci/test_unica_mcp_script_parity.py`
- Modify: `tests/ci/test_skill_provenance.py`
- Modify: `plugins/unica/provenance/skill-upstreams.json`
- Modify: `spec/decisions/0004-legacy-skill-scripts-are-migration-debt.md`

**Interfaces:**
- Produces: `UNICA_REFERENCE_MODELS_ROOT`, used only by strict Unica-owned equivalence scenarios.
- Preserves: all existing strict scenario behavior and native no-fallback assertions.

- [ ] **Step 1: Write failing provenance tests for the new ownership boundary**

Add to `tests/ci/test_skill_provenance.py`:

```python
def test_adapted_python_models_are_named_as_unica_owned_test_models(self) -> None:
    root = (
        self.repo_root()
        / "tests/fixtures/unica_mcp_script_parity/unica_reference_models"
    )
    self.assertTrue(root.is_dir())
    self.assertFalse(
        (
            self.repo_root()
            / "tests/fixtures/unica_mcp_script_parity/reference_skills"
        ).exists()
    )
    python_models = sorted(root.glob("*/scripts/*.py"))
    self.assertGreater(len(python_models), 0)
    for path in python_models:
        text = path.read_text(encoding="utf-8", errors="ignore")
        self.assertIn("Adapted from https://github.com/Nikolay-Shirokov/cc-1c-skills", text)
```

- [ ] **Step 2: Verify RED**

Run:

```bash
python3.12 -m unittest tests.ci.test_skill_provenance.SkillProvenanceTests.test_adapted_python_models_are_named_as_unica_owned_test_models -v
```

Expected: FAIL because `unica_reference_models` does not exist.

- [ ] **Step 3: Move the tree and update strict-model naming**

Run the mechanical move:

```bash
git mv tests/fixtures/unica_mcp_script_parity/reference_skills \
  tests/fixtures/unica_mcp_script_parity/unica_reference_models
```

In `test_unica_mcp_script_parity.py` use:

```python
UNICA_REFERENCE_MODELS_ROOT = FIXTURES_ROOT / "unica_reference_models"
```

Rename `run_python_script` to `run_unica_reference_model`,
`run_reference_skill_raw` to `run_unica_reference_model_raw`, and
`test_mcp_calls_match_reference_python_scripts` to
`test_mcp_calls_match_unica_reference_models`.

Change every Python source marker from:

```python
# Source: https://github.com/Nikolay-Shirokov/cc-1c-skills
```

to:

```python
# Adapted from https://github.com/Nikolay-Shirokov/cc-1c-skills
```

Update provenance `localPaths` and ADR wording to call this tree adapted
Unica-owned test models.

- [ ] **Step 4: Verify GREEN and strict equivalence**

Run:

```bash
python3.12 -m unittest \
  tests.ci.test_skill_provenance.SkillProvenanceTests.test_adapted_python_models_are_named_as_unica_owned_test_models \
  tests.ci.test_unica_mcp_script_parity.UnicaMcpScriptParityTests.test_mcp_calls_match_unica_reference_models -v
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add spec/decisions/0004-legacy-skill-scripts-are-migration-debt.md \
  plugins/unica/provenance/skill-upstreams.json \
  tests/ci/test_skill_provenance.py \
  tests/ci/test_unica_mcp_script_parity.py \
  tests/fixtures/unica_mcp_script_parity/unica_reference_models
git -c commit.gpgsign=false commit -m "test: separate Unica models from donor parity"
```

---

### Task 2: Add baseline, digest, observation, and relation contracts

**Files:**
- Create: `scripts/ci/donor_parity_contract.py`
- Create: `tests/ci/test_donor_parity_contract.py`

**Interfaces:**
- Produces: `sha256_file`, `sha256_json`, `case_content_digest`,
  `observation_fingerprint`, `validate_baseline`, and `validate_relations`.
- Consumes: repository root, donor snapshot root, baseline JSON, relation JSON,
  and normalized donor/Unica observations.

- [ ] **Step 1: Write failing unit tests**

Create `tests/ci/test_donor_parity_contract.py` with tests equivalent to:

```python
def test_baseline_detects_changed_snapshot_bytes(self):
    with self.fixture_repo() as fixture:
        manifest = fixture.write_manifest()
        fixture.snapshot_file.write_text("changed\n", encoding="utf-8")
        errors = self.module.validate_baseline(
            fixture.root, manifest, fixture.provenance
        )
        self.assertIn("sha256 mismatch", "\n".join(errors))

def test_changed_case_or_script_changes_content_digest(self):
    with self.fixture_repo() as fixture:
        before = self.module.case_content_digest(fixture.case_root, "demo/basic")
        fixture.script.write_text("print('changed')\n", encoding="utf-8")
        after = self.module.case_content_digest(fixture.case_root, "demo/basic")
        self.assertNotEqual(before, after)

def test_relation_rejects_same_gap_kind_with_changed_fingerprint(self):
    errors = self.module.validate_relation_observation(
        relation=self.reviewed_relation(),
        content_digest="case-digest",
        observation={**self.reviewed_observation(), "unicaStdoutSha256": "changed"},
    )
    self.assertIn("observation fingerprint changed", "\n".join(errors))
```

Also cover missing evidence, invalid traversal, symlinks, missing relations,
`donor_ahead` owner/decision requirements, and exact relation success.

- [ ] **Step 2: Verify RED**

Run:

```bash
python3.12 -m unittest tests.ci.test_donor_parity_contract -v
```

Expected: import failure because `scripts.ci.donor_parity_contract` does not
exist.

- [ ] **Step 3: Implement the pure contract module**

Implement path validation:

```python
def safe_relative_path(raw: str) -> Path:
    path = Path(raw)
    if path.is_absolute() or ".." in path.parts or path == Path("."):
        raise ValueError(f"unsafe repository-relative path: {raw}")
    return path
```

Implement deterministic hashes:

```python
def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()

def sha256_json(value: object) -> str:
    payload = json.dumps(
        value, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()
```

`case_content_digest` must bind `_skill.json`, the case JSON, selected fixture
files, the main script, setup script, pre-run scripts, and post-validator
scripts. `validate_baseline` must reject missing, extra, duplicate, symlink,
non-regular, traversal, hash, upstream-id, tracking-ref, and provenance-commit
errors.

`validate_relations` must require one record per discovered case and no extra
records. Non-exact records require exact observation fingerprints, reasons, and
existing evidence paths. `donor_ahead` additionally requires owner and
`decision` in `{"adopt", "defer"}`.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
python3.12 -m unittest tests.ci.test_donor_parity_contract -v
python3.12 -m py_compile scripts/ci/donor_parity_contract.py
```

Expected: all contract tests PASS and compilation exits 0.

- [ ] **Step 5: Commit**

```bash
git add scripts/ci/donor_parity_contract.py tests/ci/test_donor_parity_contract.py
git -c commit.gpgsign=false commit -m "test: define donor parity contracts"
```

---

### Task 3: Add the two-phase donor refresh tool

**Files:**
- Create: `scripts/ci/refresh-cc-1c-parity.py`
- Create: `tests/ci/test_refresh_cc_1c_parity.py`

**Interfaces:**
- Produces CLI:
  - `prepare --repo-root PATH --upstream-cache PATH --target COMMIT --review-id ID [--skill NAME ...]`
  - `apply --repo-root PATH --review PATH`
- Uses `donor_parity_contract` for safe paths and hashes.
- Writes candidates under `.build/donor-parity-refresh/<review-id>/`.

- [ ] **Step 1: Write failing refresh tests using a synthetic Git donor**

Cover:

```python
def test_prepare_carries_only_unchanged_content_relations(self):
    review = self.prepare_refresh(change_unrelated_file=True)
    self.assertEqual(review["carriedRelations"], ["meta-compile/basic"])
    self.assertEqual(review["needsReview"], [])

def test_prepare_invalidates_relation_when_script_changes(self):
    review = self.prepare_refresh(change_script=True)
    self.assertEqual(review["carriedRelations"], [])
    self.assertEqual(review["needsReview"], ["meta-compile/basic"])

def test_apply_rejects_unresolved_review(self):
    result = self.run_apply(review_status="needs-review")
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("unresolved donor relations", result.stderr)

def test_apply_updates_snapshot_manifest_provenance_and_relations_together(self):
    result = self.run_apply(review_status="reviewed")
    self.assertEqual(result.returncode, 0, result.stderr)
    self.assert_snapshot_matches_manifest()
    self.assert_provenance_matches_manifest()
```

Also test symlink rejection, removed case review, target mismatch, selected
scope enforcement, dry prepare leaving accepted files unchanged, and explicit
`form-compile-from-object` ownership mapping.

- [ ] **Step 2: Verify RED**

Run:

```bash
python3.12 -m unittest tests.ci.test_refresh_cc_1c_parity -v
```

Expected: FAIL because the refresh CLI does not exist.

- [ ] **Step 3: Implement `prepare`**

Use explicit source mappings:

```python
CASE_SCOPE_OWNERS = {
    "cfe-borrow": "cfe-borrow",
    "dcs-compile": "dcs-compile",
    "form-compile": "form-compile",
    "form-compile-from-object": "form-compile",
    "meta-compile": "meta-compile",
}
```

Copy only:

- top-level case JSON and `_skill.json`;
- `fixtures/**` regular files;
- required `.claude/skills/<skill>/scripts/**` regular files.

Exclude donor `snapshots/**`. Stage via a temporary directory under the review
root, validate the complete candidate, then rename it to `candidate/`.

Generate review fields:

```json
{
  "schemaVersion": 1,
  "upstreamId": "cc-1c-skills",
  "previousCommits": {},
  "targetCommit": "<40 hex>",
  "selectedSkills": [],
  "changedPaths": [],
  "addedCases": [],
  "removedCases": [],
  "changedCases": [],
  "carriedRelations": [],
  "needsReview": [],
  "caseDecisions": {},
  "reviewStatus": "needs-review"
}
```

- [ ] **Step 4: Implement `apply`**

Require:

- target commit still resolves to the reviewed commit;
- `reviewStatus == "reviewed"`;
- every `needsReview` case has a complete relation under `caseDecisions`;
- removed cases have `decision: remove`;
- the candidate manifest validates before accepted files change.

Update snapshot, baseline manifest, relations, provenance
`parityBaselineCommit` values, and
the tracked review artifact. Use a backup directory and restore accepted files
if any publication step fails.

- [ ] **Step 5: Verify GREEN**

Run:

```bash
python3.12 -m unittest tests.ci.test_refresh_cc_1c_parity -v
python3.12 -m py_compile \
  scripts/ci/donor_parity_contract.py \
  scripts/ci/refresh-cc-1c-parity.py
```

Expected: all refresh tests PASS.

- [ ] **Step 6: Commit**

```bash
git add scripts/ci/refresh-cc-1c-parity.py tests/ci/test_refresh_cc_1c_parity.py
git -c commit.gpgsign=false commit -m "feat: add reviewed donor parity refresh"
```

---

### Task 4: Integrate pristine donor execution and reviewed observations

**Files:**
- Create: `tests/fixtures/unica_mcp_script_parity/cc-1c-skills/skills/**`
- Create: `tests/fixtures/unica_mcp_script_parity/donor-baseline.json`
- Create: `tests/fixtures/unica_mcp_script_parity/donor-relations.json`
- Modify: `tests/ci/test_unica_mcp_script_parity.py`
- Modify: `tests/ci/test_donor_parity_contract.py`

**Interfaces:**
- Consumes: Task 2 contract helpers and Task 3 accepted snapshot.
- Produces: exact per-case observations and relation enforcement for every
  discovered donor case.

- [ ] **Step 1: Write failing parity tests for the data-driven registry**

Add tests:

```python
def test_every_donor_case_has_one_reviewed_relation(self):
    cases = {case.case_id for case in iter_cc_1c_skill_cases()}
    relations = load_donor_relations()
    self.assertEqual(set(relations), cases)

def test_donor_snapshot_integrity_and_provenance(self):
    errors = donor_contract.validate_repository_contract(REPO_ROOT)
    self.assertEqual(errors, [])

def test_category_only_expected_gap_allowlist_is_removed(self):
    self.assertNotIn(
        "CC_1C_EXPECTED_GAPS",
        Path(__file__).read_text(encoding="utf-8"),
    )
```

- [ ] **Step 2: Verify RED**

Run:

```bash
python3.12 -m unittest \
  tests.ci.test_unica_mcp_script_parity.UnicaMcpScriptParityTests.test_every_donor_case_has_one_reviewed_relation \
  tests.ci.test_unica_mcp_script_parity.UnicaMcpScriptParityTests.test_donor_snapshot_integrity_and_provenance -v
```

Expected: FAIL because manifests and relation loader do not exist.

- [ ] **Step 3: Prepare and review the current upstream donor snapshot**

Refresh the cache and resolve a concrete commit:

```bash
python3.12 scripts/ci/check-skill-upstreams.py --check --format json
python3.12 scripts/ci/refresh-cc-1c-parity.py prepare \
  --repo-root . \
  --upstream-cache .build/skill-upstreams/cc-1c-skills \
  --target main \
  --review-id 2026-07-24-cc-1c-parity-refresh
```

Inspect every affected case. Record `exact`, `compatible`,
`platform_override`, `donor_ahead`, or `intentional_divergence`; cite existing
platform/spec/test evidence. Mark the review `reviewed` and apply:

```bash
python3.12 scripts/ci/refresh-cc-1c-parity.py apply \
  --repo-root . \
  --review plugins/unica/provenance/reviews/2026-07-24-cc-1c-parity-refresh.json
```

Expected: pristine donor scripts appear under `cc-1c-skills/skills`, and
manifest/provenance hashes agree.

- [ ] **Step 4: Execute donor scripts from the pristine snapshot**

In the parity harness use:

```python
DONOR_SNAPSHOT_ROOT = FIXTURES_ROOT / "cc-1c-skills"
DONOR_SKILLS_ROOT = DONOR_SNAPSHOT_ROOT / "skills"
```

`run_cc_python_script` and donor pre-run steps must use `DONOR_SKILLS_ROOT`.
Unica strict scenarios continue using `UNICA_REFERENCE_MODELS_ROOT`.

Replace `cc_case_parity_gap` with `cc_case_observation`, returning both the gap
message and the complete normalized fingerprint. Compare it through
`validate_relation_observation`.

- [ ] **Step 5: Generate the initial exact observation fingerprints**

Add a maintainer-only command to the parity test module:

```bash
python3.12 tests/ci/test_unica_mcp_script_parity.py \
  --write-donor-observations .build/donor-observations.json
```

Merge reviewed observations into `donor-relations.json`. The writer may produce
candidates under `.build`; it must not overwrite accepted relations.

- [ ] **Step 6: Verify GREEN**

Run:

```bash
python3.12 -m unittest \
  tests.ci.test_donor_parity_contract \
  tests.ci.test_unica_mcp_script_parity -v
```

Expected: all donor cases match their exact reviewed relations; native strict
model parity still passes.

- [ ] **Step 7: Commit**

```bash
git add tests/ci/test_unica_mcp_script_parity.py \
  tests/ci/test_donor_parity_contract.py \
  tests/fixtures/unica_mcp_script_parity/cc-1c-skills \
  tests/fixtures/unica_mcp_script_parity/donor-baseline.json \
  tests/fixtures/unica_mcp_script_parity/donor-relations.json
git -c commit.gpgsign=false commit -m "test: enforce reviewed donor relations"
```

---

### Task 5: Close provenance and offline CI guardrails

**Files:**
- Create: `plugins/unica/provenance/reviews/2026-07-24-cc-1c-parity-refresh.json`
- Modify: `plugins/unica/provenance/skill-upstreams.json`
- Modify: `scripts/ci/check-skill-upstreams.py`
- Modify: `tests/ci/test_skill_provenance.py`
- Modify: `tests/ci/test_package_unica_plugin.py`
- Modify: `spec/decisions/0004-legacy-skill-scripts-are-migration-debt.md`

**Interfaces:**
- Consumes: accepted baseline manifest and refresh review.
- Produces: offline source guardrail failures for mixed/unreviewed donor changes.

- [ ] **Step 1: Write failing provenance and package tests**

Add assertions that:

```python
def test_donor_case_scopes_are_watched_by_provenance(self):
    cc = self.cc_upstream()
    entries = {entry["skill"]: entry for entry in cc["entries"]}
    self.assertIn("tests/skills/cases/meta-compile/**", entries["meta-compile"]["upstreamPaths"])
    self.assertIn("tests/skills/cases/form-compile-from-object/**", entries["form-compile"]["upstreamPaths"])

def test_accepted_donor_baseline_matches_refresh_review(self):
    manifest = self.load_donor_baseline()
    review = self.load_parity_refresh()
    self.assertEqual(review["targetCommit"], manifest["targetCommit"])
    self.assertEqual(review["reviewStatus"], "reviewed")

def test_donor_scripts_are_not_packaged(self):
    package = self.build_package()
    self.assertFalse(any("cc-1c-skills" in name for name in package.names))
```

Replace the historical test that asserts one literal donor SHA with invariants:
40-hex concrete commits, manifest synchronization, and non-floating accepted
baselines.

- [ ] **Step 2: Verify RED**

Run:

```bash
python3.12 -m unittest \
  tests.ci.test_skill_provenance \
  tests.ci.test_package_unica_plugin -v
```

Expected: FAIL until watched case paths and refresh-review assertions exist.

- [ ] **Step 3: Update provenance and offline validation**

Add exact `tests/skills/cases/<scope>/**` patterns. Make
`check-skill-upstreams.py --validate-only` verify that an operation-parity
upstream with a donor baseline manifest has matching concrete
`parityBaselineCommit` values.
Do not perform network operations in validation.

Update ADR-0004 so donor snapshots and adapted Unica models are distinct.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
python3.12 -m unittest \
  tests.ci.test_skill_provenance \
  tests.ci.test_package_unica_plugin -v
python3.12 scripts/ci/check-skill-upstreams.py --validate-only --format json
```

Expected: all tests PASS and validation returns no errors.

- [ ] **Step 5: Commit**

```bash
git add plugins/unica/provenance \
  plugins/unica/provenance/skill-upstreams.json \
  scripts/ci/check-skill-upstreams.py \
  spec/decisions/0004-legacy-skill-scripts-are-migration-debt.md \
  tests/ci/test_skill_provenance.py \
  tests/ci/test_package_unica_plugin.py
git -c commit.gpgsign=false commit -m "test: guard reviewed donor baseline updates"
```

---

### Task 6: Full verification and branch handoff

**Files:**
- Modify only files required by failures attributable to Tasks 1–5.

**Interfaces:**
- Produces: verified branch with no unrelated workspace changes.

- [ ] **Step 1: Run focused Python verification**

```bash
python3.12 -m unittest \
  tests.ci.test_donor_parity_contract \
  tests.ci.test_refresh_cc_1c_parity \
  tests.ci.test_skill_provenance \
  tests.ci.test_unica_mcp_script_parity \
  tests.ci.test_package_unica_plugin -v
```

Expected: PASS with only existing environment-conditioned skips.

- [ ] **Step 2: Run complete CI Python verification**

```bash
python3.12 -m unittest discover -s tests/ci --durations 20
python3.12 -m unittest discover -s tests/dev --durations 20
python3.12 -m py_compile scripts/ci/*.py tests/ci/*.py
```

Expected: PASS with no failures.

- [ ] **Step 3: Run Rust and repository checks**

```bash
cargo test --package unica-coder
cargo clippy --package unica-coder --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
git status --short
```

Expected: all commands exit 0; status contains only intended committed work.

- [ ] **Step 4: Inspect final contract**

Confirm:

- every donor case has one relation;
- no `CC_1C_EXPECTED_GAPS` remains;
- every accepted donor file matches its manifest;
- donor scripts are pristine snapshot bytes;
- adapted models contain `Adapted from`;
- normal validation does not access the network;
- the refresh script's synthetic prepare/apply tests pass; and
- no donor test script is packaged.

- [ ] **Step 5: Commit verification-only corrections if needed**

If verification required corrections, stage only the already declared
Task 1–5 paths shown by `git status --short`, inspect the staged diff, and
commit it:

```bash
git diff --cached --check
git diff --cached --stat
git -c commit.gpgsign=false commit -m "fix: close donor parity verification gaps"
```

Do not create an empty commit when no corrections are required, and do not
stage unrelated paths.
