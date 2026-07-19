# Cross-Repository Upgrade Regression Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` or `superpowers:subagent-driven-development` to implement this plan task-by-task.

**Status:** Historical execution context. Live code, tests, package metadata, and current GitHub state remain authoritative.

**Goal:** Close the complete Unica upgrade defect from issue #90 by preserving canonical plugin settings through both legacy paths, while making `IngvarConsulting/unica-marketplace` the single owner of manual full-history regression and automatic release gates.

**Architecture:** PR #136 owns the native migration transaction, its Codex contract fixtures, and source-release preflight. Marketplace PR #9 owns end-to-end tests that need an already published versioned Unica release: a manually dispatchable full supported-history matrix, a scheduled stable regression, and automatic reduced gates for candidate promotion. This separation avoids the circular dependency in which an unpublished source candidate would need to download its own release assets.

**Tech Stack:** Rust, `toml_edit`, PowerShell, Python `unittest`, GitHub Actions reusable workflows, Codex CLI `0.145.0-alpha.18`.

## Global Constraints

- Build the public plugin ID from the manifest identity and leave exactly one enabled `unica@unica`.
- Cover both historical managed roots, `marketplaces/unica-local` and `marketplaces/unica`, plus the exact duplicate registration `unica@unica` + `unica@unica-local` from issue #90.
- Preserve all user-owned keys below `[plugins."unica@unica"]`; `enabled` remains owned by Codex and must not be restored over the freshly installed registration.
- Any preservation or validation failure must enter the existing transactional rollback and restore the original config bytes and permissions.
- PR #136 may preflight unpublished release payloads but must not claim a full candidate upgrade before immutable assets exist.
- Marketplace PR #9 must test only non-draft published semver source releases, capture the installer asset SHA-256 values reported by GitHub, and verify downloaded installers against them. GitHub currently reports `isImmutable: false` for v0.7.5, so the live enforceable contract is publication plus exact content digest rather than an unavailable immutable-release flag. Manual full-history runs and automatic gates must select the marketplace ref under test rather than silently cloning `main`.
- The manual full matrix covers every supported published legacy state already enumerated in PR #9 on macOS, Linux, and Windows. The automatic release policy covers previous stable and the exact issue #90 v0.6.1 duplicate fixture on all three operating systems.

### Task 1: Add failing migration setting-preservation contracts

**Files:**
- Modify: `crates/unica-bootstrap/tests/migration_contract.rs`

**Step 1: Add an exact duplicate success fixture**

Create a runner that reproduces native Codex mutations: removing `unica@unica` deletes its whole config table, removing `unica@unica-local` deletes the alias table, and adding canonical Unica recreates only `enabled = true`.

**Step 2: Assert both paths**

Add engine-level tests for:

- updating an older canonical `unica@unica` while preserving direct and nested user settings;
- migrating the exact issue #90 duplicate state while removing the alias registration/cache and preserving the canonical direct and nested settings.

**Step 3: Run the focused red tests**

Run:

```bash
cargo test -p unica-bootstrap --test migration_contract preserves -- --nocapture
```

Expected: both new tests fail because successful migration currently restores settings only during rollback.

### Task 2: Preserve canonical settings inside the native transaction

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/unica-bootstrap/Cargo.toml`
- Modify: `crates/unica-bootstrap/src/migration.rs`
- Modify: `crates/unica-bootstrap/tests/migration_contract.rs`

**Step 1: Add a TOML-aware dependency**

Add `toml_edit` through workspace dependency management. Do not implement table preservation by line scanning.

**Step 2: Capture user-owned canonical settings before mutation**

Parse the preflight `config.toml`, capture every key below `[plugins."unica@unica"]` except `enabled`, and record whether a subtree was captured in the migration snapshot. Reject invalid or unsupported config before the first Codex mutation.

**Step 3: Restore settings after canonical installation**

After `plugin add unica@unica` succeeds and before discovery/runtime verification, merge captured keys into the freshly generated canonical table and atomically replace `config.toml` while retaining its permissions. A failure must be returned through the existing rollback path.

**Step 4: Verify focused and rollback contracts**

Run:

```bash
cargo test -p unica-bootstrap --test migration_contract preserves -- --nocapture
cargo test -p unica-bootstrap --test migration_contract rollback -- --nocapture
```

Expected: all focused tests pass; existing exact-byte rollback assertions remain green.

### Task 3: Keep source CI on its non-circular responsibility

**Files:**
- Delete: `.github/workflows/unica-legacy-migration.yml`
- Modify: `.github/workflows/unica-plugin-release.yml`
- Modify: `scripts/ci/test-unica-upgrade.ps1`
- Modify: `tests/ci/test_unica_upgrade_script.py`
- Modify: `tests/ci/test_unica_workflows.py`

**Step 1: Remove the duplicate stable workflow from the source repo**

Delete the scheduled/manual full stable workflow and its source-workflow path references. Marketplace PR #9 becomes the only stable/full policy owner.

**Step 2: Strengthen unpublished-candidate preflight evidence**

Retain the two managed-root matrix entries (`unica-local`, `unica`) and make the report identify the tested path contract. Keep mode `Preflight`; do not execute a candidate runtime that is not yet published.

**Step 3: Update source workflow tests**

Require the release preflight matrix for both managed roots and forbid the deleted stable workflow. Run:

```bash
python3.12 -m unittest tests.ci.test_unica_upgrade_script tests.ci.test_unica_workflows -v
```

### Task 4: Make marketplace full regression manually reproducible

**Files in `IngvarConsulting/unica-marketplace` PR #9:**
- Modify: `.github/workflows/legacy-migration-case.yml`
- Modify: `.github/workflows/legacy-migration-regression.yml`
- Modify: `tests/test_verify_marketplace.py`
- Modify: `MIGRATION.md`

**Step 1: Add target marketplace ref to the reusable case**

Require the selected marketplace ref and its resolved commit as reusable-workflow inputs. Pass the ref to `install-unica.ps1 -Ref` / `install-unica.sh --ref`, and prove before and after each migration job that the remote ref still resolves to the verified commit; never hardcode `main` for a candidate branch.

**Step 2: Resolve and validate the manual target**

For `workflow_dispatch`, use the GitHub-selected workflow ref by default (and `github.head_ref` for pull requests), allow an optional safe marketplace ref override and target version assertion, read the selected commit's catalog, and fail if its version tag or published non-draft source release does not match the requested target. Capture both installer asset digests for the reusable jobs.

**Step 3: Keep the full supported-history matrix**

Run all existing supported source states and the exact issue #90 duplicate fixture on macOS, Linux, and Windows. Add a weekly schedule against `main`; a manual run can target a candidate branch after its source release has been published.

**Step 4: Test the workflow contract**

Run:

```bash
python3 -m unittest discover -s tests -v
actionlint .github/workflows/*.yml
```

### Task 5: Enforce the automatic marketplace release policy

**Files in `IngvarConsulting/unica-marketplace` PR #9:**
- Modify: `.github/workflows/verify.yml`
- Modify: `.github/workflows/legacy-migration-case.yml`
- Modify: `tests/test_verify_marketplace.py`

**Step 1: Run issue #90 gate before promotion**

When a pull request contains a staged plugin backed by a published immutable source release, invoke the reusable issue #90 duplicate case against the pull request's marketplace branch and staged plugin version on all three operating systems.

**Step 2: Run issue #90 gate after promotion**

Keep the same gate on `main` so the canonical consumer path is independently proved after catalog promotion.

**Step 3: Keep previous-stable coverage**

Retain the previous-stable seed/upgrade policy and ensure it uses the selected candidate/main marketplace ref rather than an implicit unrelated state.

**Step 4: Verify policy assertions**

Require tests to assert pull-request and main triggers, exact issue #90 layout, v0.6.1 source, three operating systems, selected marketplace ref, and target version derivation.

### Task 6: Cross-repository verification and GitHub synchronization

**Files:**
- Modify PR #136 description
- Modify marketplace PR #9 description
- Add an evidence comment to issue #90 only after successful real full regression

**Step 1: Verify PR #136**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --test-threads=1
python3.12 -m unittest discover -s tests/ci
git diff --check
```

**Step 2: Verify marketplace PR #9**

Run:

```bash
python3 -m unittest discover -s tests -v
python3 scripts/verify_marketplace.py
actionlint .github/workflows/*.yml
git diff --check
```

**Step 3: Commit and push both PR branches**

Commit source migration changes to `codex/test-legacy-migrations` and marketplace policy changes to `codex/issue-90-migration-regression`, then push both heads.

**Step 4: Execute real GitHub evidence**

Wait for PR #136 CI. After an immutable patch release containing its migration fix is published and staged in marketplace PR #9, dispatch the full regression on the PR #9 ref and require every supported-history case, including all three issue #90 jobs, to pass.

**Step 5: Synchronize PR descriptions and merge order**

Cross-link both PRs and document this order:

1. merge PR #136;
2. publish the patch release and verify immutable assets;
3. stage that tag in marketplace PR #9;
4. run and pass the manual full regression;
5. merge marketplace PR #9;
6. close issue #90 only after the post-promotion automatic gate passes.
