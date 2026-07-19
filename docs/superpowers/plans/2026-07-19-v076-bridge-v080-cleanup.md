# Unica v0.7.6 Bridge and v0.8.0 Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Historical execution record once committed; live code, tests, package metadata, GitHub state, and published assets remain authoritative.

**Goal:** Publish `v0.7.6` as the immutable legacy migration bridge, merge the bounded marketplace regression policy, prove the bridge manually, close issue #90, and open the issue #135 pull request that removes legacy migration from `v0.8.0`.

**Architecture:** `IngvarConsulting/unica` owns the frozen bridge implementation and release assets. `IngvarConsulting/unica-marketplace` owns bounded automatic promotion checks plus a dispatch-only full historical regression. The subsequent `v0.8.0` source branch keeps normal bootstrap run/verify and canonical marketplace upgrades while deleting executable legacy migration code and referring old users to the immutable `v0.7.6` assets.

**Tech Stack:** Rust/Cargo, Python 3.12 `unittest`, POSIX shell, Windows PowerShell 5.1, GitHub Actions, Codex CLI plugin commands, GitHub CLI.

## Global Constraints

- Direct update to `v0.8.0` is supported only from canonical `v0.7.5`, canonical `v0.7.6`, or canonical technical `0.7.x` installations.
- Any local, duplicated, or otherwise legacy layout must run the immutable `v0.7.6` migration installer first.
- Full historical migration regression and issue #90 are manual only; there is no schedule.
- Automatic marketplace promotion checks cover fresh installation and immediately previous stable canonical upgrade on macOS, Linux, and Windows.
- There is no `0.9.x -> 1.x` legacy receipt or barrier.
- `v0.7.6` release documentation links to its own published `install-unica.sh` and `install-unica.ps1` assets.
- `v0.8.0` contains no executable legacy migration, backup, rollback, or installer-shim implementation.
- Use `apply_patch` for every file edit and deletion; preserve unrelated user changes.
- Do not close issue #90 until the published and promoted `v0.7.6` manual regression succeeds.

---

### Task 1: Correct the Marketplace Policy to the v0.7.6 Bridge

**Repository:** `/Users/ingvarvilkman/Documents/git/unica-marketplace`

**Files:**
- Modify: `tests/test_regression_policy.py`
- Modify: `tests/test_detect_promotion.py`
- Modify: `tests/test_verify_marketplace.py`
- Modify: `scripts/regression_policy.py`
- Modify: `scripts/detect_promotion.py`
- Modify: `.github/workflows/legacy-migration-regression.yml`
- Modify: `.github/workflows/verify.yml`
- Modify: `MIGRATION.md`
- Modify: `docs/superpowers/specs/2026-07-19-marketplace-regression-policy-design.md`
- Modify: `docs/superpowers/plans/2026-07-19-marketplace-regression-policy.md`
- Delete uncommitted: `scripts/legacy_barrier.py`
- Delete uncommitted: `tests/test_legacy_barrier.py`

**Interfaces:**
- `build_manual_cases(inventory, profile_set)` accepts `current` or `bridge`.
- Manual `bridge` mode adds only the representative historical CLI cases needed to prove `v0.7.6`; it does not create a future release receipt.
- `scripts/detect_promotion.py` emits no `barrier_required` output.
- `regression-policy` depends on contract, staged package, fresh install, target resolution, seed, and previous-stable upgrade only.

- [ ] **Step 1: Replace the obsolete barrier test with failing bridge-policy tests**

Use these assertions in the existing test modules:

```python
def test_bridge_profile_adds_only_representative_historical_cli_cases(self):
    cases = policy.build_manual_cases(policy.load_release_inventory(RELEASES), "bridge")
    historical = [case["label"] for case in cases
                  if case["codexProfile"] == "historical-0.144.1"]
    self.assertEqual(historical, ["v0.3.11", "issue-90-duplicate"])

def test_promotion_detection_has_no_future_legacy_barrier(self):
    outputs = self.run_detector(previous="0.9.8", target="1.0.0")
    self.assertNotIn("barrier_required", outputs)

def test_manual_regression_is_the_v076_bridge_and_has_no_receipt(self):
    manual = FULL_WORKFLOW.read_text(encoding="utf-8")
    automatic = VERIFY_WORKFLOW.read_text(encoding="utf-8")
    self.assertIn("profile_set:", manual)
    self.assertIn("bridge", manual)
    self.assertNotIn("barrier-receipt:", manual)
    self.assertNotIn("verify-legacy-barrier:", automatic)
    self.assertNotIn("legacy_barrier.py", manual + automatic)
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
python3 -m unittest tests.test_regression_policy tests.test_detect_promotion tests.test_verify_marketplace -v
```

Expected: failures name the old `barrier` profile, `barrier_required`, and receipt expectations.

- [ ] **Step 3: Implement the minimal bridge policy**

Apply these exact semantic changes:

```python
PROFILE_SETS = {"current", "bridge"}

if profile_set == "bridge":
    selected = [entry for entry in inventory["releases"]
                if entry["version"] == "0.3.11"]
    selected.extend(inventory["fixtures"])
    cases.extend(case_for(entry, "historical-0.144.1") for entry in selected)
```

Remove `requires_legacy_barrier`, `barrier_required`, the receipt job, the
receipt verifier job, and all aggregate dependencies on that job. Rename the
manual workflow choice from `barrier` to `bridge`; its target remains the exact
selected marketplace commit and source release. Delete the two uncommitted
barrier files with `apply_patch`.

Rewrite the active marketplace policy and migration guide so they state:

```text
v0.7.6 is the immutable legacy migration bridge.
The full historical regression is workflow_dispatch only.
Issue #90 is included only in that manual regression.
Automatic promotion checks fresh install and previous-stable canonical update.
There is no weekly schedule and no later 0.9-to-1.0 receipt.
```

- [ ] **Step 4: Verify GREEN and syntax**

Run:

```bash
python3 -m unittest discover -s tests -v
python3 -m py_compile scripts/*.py tests/*.py
actionlint .github/workflows/*.yml .github/actions/setup-locked-codex/action.yml
git diff --check
```

Expected: all tests pass, Python compilation succeeds, actionlint reports no
errors, and `git diff --check` is empty.

- [ ] **Step 5: Commit the corrected policy**

```bash
git add .github scripts tests MIGRATION.md docs/superpowers
git commit -m "ci: make v0.7.6 the migration bridge"
```

### Task 2: Publish and Merge Marketplace PR #9

**Repository:** `/Users/ingvarvilkman/Documents/git/unica-marketplace`

**Files:** no new source files; GitHub PR metadata only.

**Interfaces:**
- Remote branch `codex/issue-90-migration-regression` contains the verified local commits.
- PR #9 is ready for review and targets `main`.

- [ ] **Step 1: Verify the exact branch before push**

Run:

```bash
git status --short --branch
git log --oneline origin/codex/issue-90-migration-regression..HEAD
git diff --check origin/main...HEAD
```

Expected: clean branch, only planned commits, no whitespace errors.

- [ ] **Step 2: Push without rewriting history**

```bash
git push origin codex/issue-90-migration-regression
```

- [ ] **Step 3: Update PR #9 and mark it ready**

The PR body must include:

```markdown
Closes no historical issue by itself: issue #90 closes only after the published
v0.7.6 manual bridge regression succeeds.

- automatic: fresh install + previous-stable canonical update on 3 OS
- manual: full historical inventory, issue #90, rollback, historical Codex CLI
- absent: schedule and 0.9-to-1.0 receipt
```

Run `gh pr ready 9 --repo IngvarConsulting/unica-marketplace` after updating the body.

- [ ] **Step 4: Wait for hosted checks and merge**

```bash
gh pr checks 9 --repo IngvarConsulting/unica-marketplace --watch
gh pr merge 9 --repo IngvarConsulting/unica-marketplace --squash --delete-branch=false
gh pr view 9 --repo IngvarConsulting/unica-marketplace --json state,mergedAt,mergeCommit,url
```

Expected: checks succeed and PR state is `MERGED` with a merge commit.

### Task 3: Turn Source PR #136 into Release v0.7.6

**Repository:** `/Users/ingvarvilkman/Documents/git/unica/.worktrees/test-legacy-migrations`

**Files:**
- Modify: `tests/ci/test_version_contract.py`
- Modify: `tests/ci/test_product_contracts.py`
- Modify: `tests/ci/test_install_unica_script.py`
- Modify: `tests/ci/test_package_unica_plugin.py`
- Modify: `tests/ci/test_smoke_unica_bootstrap.py`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `plugins/unica/.codex-plugin/plugin.json`
- Modify: `plugins/unica/runtime-manifest.json`
- Modify: `plugins/unica/third-party/tools.lock.json`
- Modify: `.github/workflows/unica-plugin-release.yml`
- Modify: `README.md`
- Modify: `plugins/unica/README.md`
- Create: `docs/releases/v0.7.6.md`

**Interfaces:**
- Every package version surface is exactly `0.7.6`.
- The transition table contains immutable `v0.7.6` installer asset URLs.
- The release workflow publishes those scripts together with all three runtime archives.

- [ ] **Step 1: Write failing version and documentation contracts**

Change the version expectation to:

```python
self.assertEqual(values, {
    "workspace": "0.7.6",
    "plugin": "0.7.6",
    "tools-lock-unica": "0.7.6",
})
```

Add this README contract:

```python
def test_readme_documents_the_frozen_v076_bridge(self):
    readme = (REPO_ROOT / "README.md").read_text(encoding="utf-8")
    self.assertIn("| Исходное состояние |", readme)
    self.assertIn("releases/download/v0.7.6/install-unica.sh", readme)
    self.assertIn("releases/download/v0.7.6/install-unica.ps1", readme)
    self.assertIn("v0.7.5", readme)
    self.assertIn("техническ", readme.lower())
    self.assertIn("v0.8.0", readme)
```

Update package fixture expectations from `0.7.5`/`v0.7.5` to
`0.7.6`/`v0.7.6` where they describe the current source package.

- [ ] **Step 2: Run focused tests and verify RED**

```bash
python3 tests/ci/test_version_contract.py -v
python3 tests/ci/test_product_contracts.py -v
python3 tests/ci/test_install_unica_script.py -v
python3 tests/ci/test_package_unica_plugin.py -v
python3 tests/ci/test_smoke_unica_bootstrap.py -v
```

Expected: failures report current `0.7.5` surfaces, missing table, and missing
`v0.7.6` release note.

- [ ] **Step 3: Bump all source and package surfaces to 0.7.6**

Set:

```toml
[workspace.package]
version = "0.7.6"
```

Set plugin metadata, runtime release tag/URLs, `tools.lock.json` Unica runtime
version, Cargo lock workspace package versions, and the release workflow
fallback to `0.7.6`/`v0.7.6`. Do not change third-party tool versions.

- [ ] **Step 4: Replace the README migration prose with the transition table**

Use this policy in both public README files:

```markdown
| Исходное состояние | Переход |
| --- | --- |
| Локальная, дублированная или иная legacy-установка | Запустить замороженный `install-unica.sh` или `install-unica.ps1` из релиза `v0.7.6`. |
| Каноническая `v0.7.5` | Выполнить обычное обновление marketplace до `v0.7.6`. |
| Каноническая `v0.7.6` | Для `v0.8.0` выполнить обычное обновление marketplace. |
| Каноническая техническая `0.7.x` | Для `v0.8.0` выполнить обычное обновление; legacy-layout сначала привести через `v0.7.6`. |
```

The shell and PowerShell link targets must be exactly:

```text
https://github.com/IngvarConsulting/unica/releases/download/v0.7.6/install-unica.sh
https://github.com/IngvarConsulting/unica/releases/download/v0.7.6/install-unica.ps1
```

Keep the automatic rollback explanation, but remove wording that directs users
to mutable repository scripts.

- [ ] **Step 5: Add `docs/releases/v0.7.6.md`**

The note must describe both issue #90 path fixes, exact settings preservation,
the final migration-bridge boundary, manual marketplace regression ownership,
and the fact that `v0.8.0` removes legacy migration.

- [ ] **Step 6: Verify GREEN**

```bash
python3 -m unittest discover -s tests/ci -v
python3 -m py_compile scripts/ci/*.py tests/ci/*.py
sh -n scripts/install-unica.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --test-threads=1
git diff --check
```

Expected: all commands exit zero.

- [ ] **Step 7: Commit release preparation**

```bash
git add Cargo.toml Cargo.lock plugins .github README.md docs/releases tests
git commit -m "release: prepare Unica v0.7.6 migration bridge"
```

### Task 4: Merge PR #136 and Publish the Signed v0.7.6 Release

**Repository:** `/Users/ingvarvilkman/Documents/git/unica/.worktrees/test-legacy-migrations`

**Files:** GitHub PR/release state only.

**Interfaces:**
- PR #136 contains Tasks 3 and the previously reviewed issue #90 root-cause fixes.
- Signed tag `v0.7.6` points to the merged `main` commit.

- [ ] **Step 1: Push and update PR #136**

```bash
git push origin codex/test-legacy-migrations
gh pr edit 136 --repo IngvarConsulting/unica \
  --title "release: make v0.7.6 the legacy migration bridge" \
  --body-file .build/pr-136-body.md
gh pr checks 136 --repo IngvarConsulting/unica --watch
```

Create `.build/pr-136-body.md` with `apply_patch` before the command. Its body
names both issue #90 installation paths, the transition table, version
surfaces, and exact local verification commands. Delete the ignored temporary
file with `apply_patch` after `gh pr edit` succeeds.

- [ ] **Step 2: Merge and prove the merge**

```bash
gh pr merge 136 --repo IngvarConsulting/unica --squash --delete-branch=false
gh pr view 136 --repo IngvarConsulting/unica --json state,mergedAt,mergeCommit,url
```

Expected: state `MERGED` and a non-null merge commit.

- [ ] **Step 3: Create the signed release tag from remote main**

Use a clean detached release worktree at the exact merged commit. Verify no
existing tag/release, then run:

```bash
git tag -s v0.7.6 -m "Unica v0.7.6"
git tag -v v0.7.6
git push origin v0.7.6
```

Expected: signature verification succeeds before push.

- [ ] **Step 4: Verify the published release and bytes**

```bash
release_run_id="$(gh run list --repo IngvarConsulting/unica \
  --workflow unica-plugin-release.yml --branch v0.7.6 --limit 1 \
  --json databaseId --jq '.[0].databaseId')"
test -n "$release_run_id"
gh run watch --repo IngvarConsulting/unica "$release_run_id"
gh release view v0.7.6 --repo IngvarConsulting/unica --json tagName,isDraft,isPrerelease,publishedAt,assets,url
release_asset_dir="$(mktemp -d)"
gh release download v0.7.6 --repo IngvarConsulting/unica --dir "$release_asset_dir"
```

Require exactly one `install-unica.sh`, one `install-unica.ps1`, and the expected
runtime archive/metadata assets for `darwin-arm64`, `linux-x64`, and `win-x64`.
Inspect the downloaded scripts for the `migrate-preflight` then `migrate` chain
and verify runtime archives with the repository verifier.

### Task 5: Promote v0.7.6, Run Full Manual Regression, and Close Issue #90

**Repositories:** both source and marketplace.

**Files:** GitHub workflow, marketplace promotion PR, and issue state only.

**Interfaces:**
- Marketplace stable catalog points to immutable source ref `v0.7.6`.
- Manual workflow runs from the merged PR #9 policy commit with `profile_set=bridge` and target version `0.7.6`.

- [ ] **Step 1: Verify staging and promotion PRs**

Wait for the source `publish-unica-marketplace.yml` staging workflow. Inspect
the staged payload version/ref/digests, merge its staging PR, dispatch or follow
the explicit promote job, then require the promotion PR `regression-policy`
check to succeed before merging.

- [ ] **Step 2: Prove the stable catalog**

```bash
gh api repos/IngvarConsulting/unica-marketplace/contents/.agents/plugins/marketplace.json?ref=main --jq .content | base64 --decode
gh release view v0.7.6 --repo IngvarConsulting/unica-marketplace
```

Expected: source ref is exactly `v0.7.6`; staged plugin version is `0.7.6`.

- [ ] **Step 3: Dispatch full bridge regression**

```bash
gh workflow run legacy-migration-regression.yml \
  --repo IngvarConsulting/unica-marketplace \
  --ref main \
  -f marketplace_ref=main \
  -f target_version=0.7.6 \
  -f profile_set=bridge
manual_run_id="$(gh run list --repo IngvarConsulting/unica-marketplace \
  --workflow legacy-migration-regression.yml --event workflow_dispatch --limit 1 \
  --json databaseId --jq '.[0].databaseId')"
test -n "$manual_run_id"
gh run watch --repo IngvarConsulting/unica-marketplace "$manual_run_id"
```

Expected: every inventory case, both issue #90 layouts, three-platform rollback,
settings preservation, canonical discovery, MCP/prompt proof, and idempotence
succeed.

- [ ] **Step 4: Close issue #90 with evidence**

Post a comment containing the source release URL, marketplace promotion commit,
manual workflow URL, issue #90 case names, and rollback jobs. Then close:

```bash
gh issue close 90 --repo IngvarConsulting/unica --reason completed
gh issue view 90 --repo IngvarConsulting/unica --json state,closedAt,url
```

Expected: issue state `CLOSED` only after the manual run is successful.

### Task 6: Rewrite Issue #135 to the Bridge-and-Cleanup Policy

**Repository:** `IngvarConsulting/unica` GitHub issue state.

**Files:** prepared issue body Markdown only; delete the temporary file after use.

- [ ] **Step 1: Replace the issue body**

The new body must include this acceptance matrix:

```markdown
| From | v0.8.0 policy |
| --- | --- |
| canonical v0.7.5 | ordinary marketplace update |
| canonical v0.7.6 | ordinary marketplace update |
| canonical technical 0.7.x | ordinary marketplace update |
| any legacy/local/duplicate state | first run frozen v0.7.6 installer |
```

It must require deletion of migration commands, engine, fixtures, source CI,
installer shims, and migration-only dependencies in `v0.8.0`, while preserving
run/verify and canonical promotion coverage.

- [ ] **Step 2: Verify the live issue body**

```bash
gh issue view 135 --repo IngvarConsulting/unica --json title,body,state,url
```

Expected: title names `v0.8.0`, body names `v0.7.6`, and the issue remains open.

### Task 7: Implement the v0.8.0 Legacy Migration Cleanup

**Repository:** a new ignored worktree based on updated `origin/main`, branch
`codex/issue-135-remove-legacy-migration`.

**Files:**
- Create: `crates/unica-bootstrap/tests/cli_contract.rs`
- Create: `tests/ci/test_legacy_migration_boundary.py`
- Modify: `crates/unica-bootstrap/src/main.rs`
- Modify: `crates/unica-bootstrap/src/lib.rs`
- Modify: `crates/unica-bootstrap/src/codex.rs`
- Modify: `crates/unica-bootstrap/Cargo.toml`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `.github/workflows/unica-plugin-release.yml`
- Modify: `scripts/ci/classify-workflow-changes.py`
- Modify: `tests/ci/test_classify_workflow_changes.py`
- Modify: `tests/ci/test_unica_workflow.py`
- Modify: `tests/ci/test_package_unica_plugin.py`
- Modify: `tests/ci/test_product_contracts.py`
- Modify: `README.md`
- Modify: `plugins/unica/README.md`
- Delete: `crates/unica-bootstrap/src/migration.rs`
- Delete: `crates/unica-bootstrap/tests/migration_contract.rs`
- Delete: `crates/unica-bootstrap/tests/fixtures/codex-0.145.0-alpha.18/README.md`
- Delete: `crates/unica-bootstrap/tests/fixtures/codex-0.145.0-alpha.18/marketplaces-local.json`
- Delete: `crates/unica-bootstrap/tests/fixtures/codex-0.145.0-alpha.18/metadata.json`
- Delete: `crates/unica-bootstrap/tests/fixtures/codex-0.145.0-alpha.18/plugins-installed.json`
- Delete: `crates/unica-bootstrap/tests/fixtures/marketplaces-empty.json`
- Delete: `crates/unica-bootstrap/tests/fixtures/plugins-empty.json`
- Delete: `scripts/install-unica.sh`
- Delete: `scripts/install-unica.ps1`
- Delete: `scripts/ci/test-unica-upgrade.ps1`
- Delete: `tests/ci/test_install_unica_script.py`
- Delete: `tests/ci/test_unica_upgrade_script.py`

**Interfaces:**
- Bootstrap accepts only `run` and `verify`.
- `codex.rs` retains only `CommandSpec`, `CommandRunner`, and
  `SystemCommandRunner` needed by fresh prompt verification.
- Version surfaces advance to `0.8.0`.
- README keeps immutable `v0.7.6` links even though current installer shims are absent.

- [ ] **Step 1: Create the isolated worktree and establish a green baseline**

```bash
git fetch origin main
git check-ignore -q .worktrees
git worktree add .worktrees/issue-135-remove-legacy-migration \
  -b codex/issue-135-remove-legacy-migration origin/main
python3 -m unittest discover -s tests/ci -v
cargo test --workspace -- --test-threads=1
```

Expected: ignored worktree and passing baseline.

- [ ] **Step 2: Write failing absence and CLI tests**

Use:

```rust
#[test]
fn migrate_commands_are_not_part_of_the_v080_cli() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_unica-bootstrap"))
        .arg("migrate")
        .arg("--plugin-root")
        .arg(".")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported command: migrate"));
}
```

Use Python boundary assertions:

```python
def test_v080_has_only_frozen_v076_migration_entrypoint(self):
    self.assertFalse((ROOT / "scripts/install-unica.sh").exists())
    self.assertFalse((ROOT / "scripts/install-unica.ps1").exists())
    self.assertFalse((ROOT / "crates/unica-bootstrap/src/migration.rs").exists())
    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    self.assertIn("releases/download/v0.7.6/install-unica.sh", readme)
    self.assertIn("releases/download/v0.7.6/install-unica.ps1", readme)

def test_release_workflow_has_no_legacy_jobs_or_installer_assets(self):
    workflow = (ROOT / ".github/workflows/unica-plugin-release.yml").read_text()
    for forbidden in ("legacy-migration-preflight", "test-unica-upgrade.ps1",
                      "dist/installer/install-unica.sh", "dist/installer/install-unica.ps1"):
        self.assertNotIn(forbidden, workflow)
```

- [ ] **Step 3: Run tests and verify RED**

```bash
cargo test -p unica-bootstrap --test cli_contract
python3 tests/ci/test_legacy_migration_boundary.py -v
```

Expected: existing migrate command succeeds far enough to produce a different
error, and all legacy files/jobs still exist.

- [ ] **Step 4: Remove migration implementation and shrink bootstrap APIs**

Set the command parser to:

```rust
enum Command { Run, Verify }

match name.as_str() {
    "run" => Command::Run,
    "verify" => Command::Verify,
    other => return Err(BootstrapError::new(format!("unsupported command: {other}"))),
}
```

Delete the listed migration files and fixtures. Remove migration exports from
`lib.rs`, discovery models/functions from `codex.rs`, and `toml_edit` from the
bootstrap crate when `cargo tree -i toml_edit` proves no remaining consumer.
Keep `uuid` because runtime cache publication still uses it.

- [ ] **Step 5: Remove source release legacy orchestration and current shims**

Delete the installer/preflight jobs and assets from
`.github/workflows/unica-plugin-release.yml`, their change-classifier entries,
scripts, and tests. Update workflow tests to require absence. Keep the README
transition table pointing to the published `v0.7.6` URLs.

- [ ] **Step 6: Advance current package versions to 0.8.0 and verify GREEN**

Update the same version surfaces proven in Task 3 to `0.8.0`; then run:

```bash
python3 -m unittest discover -s tests/ci -v
python3 -m py_compile scripts/ci/*.py tests/ci/*.py
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --test-threads=1
rg -n "migrate-preflight|MigrationEngine|legacy-migration-preflight|test-unica-upgrade|scripts/install-unica" \
  crates scripts tests .github README.md plugins/unica/README.md \
  -g '!docs/releases/**' -g '!docs/superpowers/**'
git diff --check
```

Expected: test/build commands exit zero; the search returns only immutable
`v0.7.6` documentation links or no executable references.

- [ ] **Step 7: Commit, push, and open the issue #135 PR**

```bash
git add -A
git commit -m "refactor: remove legacy migration from v0.8.0"
git push -u origin codex/issue-135-remove-legacy-migration
gh pr create --repo IngvarConsulting/unica --base main \
  --head codex/issue-135-remove-legacy-migration \
  --title "refactor: remove legacy migration from v0.8.0" \
  --body-file .build/pr-135-body.md
```

Create `.build/pr-135-body.md` with `apply_patch` before `gh pr create`. The PR
body uses `Refs #135`, not `Closes #135`, because the issue should remain open
until the cleanup is reviewed and merged. It states the supported input
versions and the frozen `v0.7.6` bridge links. Delete the ignored temporary file
with `apply_patch` after PR creation succeeds.

### Task 8: Final Requirement Audit

**Repositories:** source and marketplace plus GitHub state.

- [ ] **Step 1: Build an evidence checklist**

Verify each objective against authoritative evidence:

```text
v0.7.6 migration bridge -> signed source tag + published installer bytes
transition table -> merged README at main + exact release URLs
PR #136 accepted -> MERGED state and merge commit
PR #9 accepted -> MERGED state and merge commit
issue #90 closed -> successful manual bridge run URL precedes closedAt
issue #135 rewritten -> live issue body contains v0.7.6/v0.8.0 matrix
v0.8.0 cleanup PR -> open PR head, diff, and green hosted checks
```

- [ ] **Step 2: Inspect final states and report gaps instead of inferring success**

Run fresh `gh pr view`, `gh issue view`, `gh release view`, `gh run view`,
marketplace catalog API, and local `git status` commands. Only mark the goal
complete if every item is directly proven and no required work remains.
