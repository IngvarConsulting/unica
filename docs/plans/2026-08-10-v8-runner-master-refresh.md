# V8-runner Master Refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Собрать неизменяемый трёхплатформенный `v8-runner` из upstream commit `7ce1b062843d86644fe55741dbe0ee79f7ca767d` и переключить Unica на проверенный `v8-runner-nightly-master-build.2`.

**Architecture:** Сначала `IngvarConsulting/unica-toolchain` закрепляет точный upstream commit, собирает три target asset и публикует immutable release. Только после проверки опубликованных байтов Unica обновляет consumer lock, документацию strict platform resolution и историческую запись upstream review; публичная MCP-поверхность и full-only source-sync guard не меняются.

**Tech Stack:** Python 3.12, GitHub Actions/CLI, Cargo/Rust 1.95.0, JSON manifests, Rust/Python contract tests.

## Global Constraints

- Upstream source: `https://github.com/alkoleft/v8-runner-rust`, ref `master`, exact commit `7ce1b062843d86644fe55741dbe0ee79f7ca767d`.
- Toolchain release identity: `v8-runner-nightly-master-build.2`; existing tags/releases are immutable and must not be moved or overwritten.
- Targets: `darwin-arm64`, `linux-x64`, `win-x64`; Unica lock changes only after all three assets are published and verified.
- License remains `AGPL-3.0-only`; packaged license text remains `plugins/unica/third-party/licenses/v8-runner/LICENSE`.
- Unica `version` remains `0.5.1`; nightly identity is carried by `sourceTag`, `sourceCommit`, and `assetTag`.
- Keep applied `mode=incremental|partial` fail-closed and keep `source_sync_dump_guard`; upstream issue `#30` and PR `#39` are not in the pinned commit.
- Do not expose `noBuild`, `dependsOn`, raw runner flags, or any other new MCP argument in this change.
- This is a config/data refresh: update existing general contract tests, but do not add a test whose sole purpose is to duplicate build revision `2` or a release checksum outside `tools.lock.json`.

---

### Task 1: Pin the new snapshot in unica-toolchain

**Files:**
- Modify: `/Users/ingvarvilkman/Documents/git/unica-toolchain/manifests/v8-runner.json`
- Test: `/Users/ingvarvilkman/Documents/git/unica-toolchain/tests/test_manifest.py`
- Test: `/Users/ingvarvilkman/Documents/git/unica-toolchain/tests/test_toolchain_cli.py`

**Interfaces:**
- Consumes: upstream Git commit `7ce1b062843d86644fe55741dbe0ee79f7ca767d` and the schema-v3 nightly manifest contract.
- Produces: a merged `main` manifest whose `describe` result is exactly `v8-runner-nightly-master-build.2`.

- [ ] **Step 1: Create an isolated toolchain worktree from current origin/main**

```bash
cd /Users/ingvarvilkman/Documents/git/unica-toolchain
git fetch origin main
git check-ignore -q .worktrees
git worktree add .worktrees/v8-runner-master-build-2 \
  -b codex/v8-runner-master-build-2 origin/main
cd /Users/ingvarvilkman/Documents/git/unica-toolchain/.worktrees/v8-runner-master-build-2
```

Expected: clean worktree based on `origin/main`; no pre-existing local or remote branch with that name.

- [ ] **Step 2: Verify the target source and release identity are still unambiguous**

```bash
git ls-remote https://github.com/alkoleft/v8-runner-rust.git refs/heads/master
gh release view v8-runner-nightly-master-build.2 \
  --repo IngvarConsulting/unica-toolchain
git ls-remote --exit-code origin refs/tags/v8-runner-nightly-master-build.2
```

Expected: upstream `master` resolves to `7ce1b062843d86644fe55741dbe0ee79f7ca767d`; both release/tag lookups for build 2 report not found. If master differs, stop rather than silently changing the approved source commit.

- [ ] **Step 3: Update only the v8-runner manifest**

Use `apply_patch` to change:

```json
"buildRevision": 2
```

and:

```json
"commit": "7ce1b062843d86644fe55741dbe0ee79f7ca767d"
```

Keep `source.kind=nightly`, `source.ref=master`, Rust `1.95.0`, target matrix, AGPL license, smoke arguments, and empty patches unchanged.

- [ ] **Step 4: Validate manifest shape, exact source, and local checks**

```bash
python3.12 scripts/toolchain.py describe --manifest manifests/v8-runner.json
python3.12 scripts/toolchain.py validate-source \
  --manifest manifests/v8-runner.json \
  --repo-root . \
  --work-dir .build/source-v8-runner \
  --out-dir dist/source-v8-runner
python3.12 -m unittest discover -s tests
python3.12 -m py_compile scripts/*.py toolchain/*.py toolchain/builders/*.py tests/*.py
actionlint
git diff --check
```

Expected: `describe` reports `v8-runner-nightly-master-build.2`; `validate-source` reports commit `7ce1b062843d86644fe55741dbe0ee79f7ca767d` and copies `license-v8-runner-AGPL-3.0-only.txt`; all tests and static checks pass.

- [ ] **Step 5: Commit, push, open the toolchain PR, and wait for CI**

```bash
git add manifests/v8-runner.json
git -c commit.gpgsign=false commit -m "build: refresh v8-runner master snapshot"
git push -u origin codex/v8-runner-master-build-2
gh pr create \
  --repo IngvarConsulting/unica-toolchain \
  --base main \
  --head codex/v8-runner-master-build-2 \
  --title "build: refresh v8-runner master snapshot" \
  --body "Refreshes v8-runner from 72d346c0a8fcf8373d9388257d11e6bef0ad70b2 to 7ce1b062843d86644fe55741dbe0ee79f7ca767d after review of the 36-commit range. The AGPL-3.0-only license and darwin-arm64/linux-x64/win-x64 target matrix are unchanged. This snapshot does not fix CDFI issue #30; Unica keeps its full-only source-sync guard."
gh pr checks --watch --repo IngvarConsulting/unica-toolchain
```

- [ ] **Step 6: Merge only the green independent toolchain PR**

```bash
gh pr merge --repo IngvarConsulting/unica-toolchain --merge --delete-branch
git fetch origin main
git show origin/main:manifests/v8-runner.json
```

Expected: merged manifest contains build revision `2` and the exact approved commit.

---

### Task 2: Build and verify the immutable toolchain release

**Files:**
- Read: `/Users/ingvarvilkman/Documents/git/unica-toolchain/.github/workflows/release-tool.yml`
- Read: release assets for `IngvarConsulting/unica-toolchain@v8-runner-nightly-master-build.2`

**Interfaces:**
- Consumes: merged manifest from Task 1.
- Produces: three executable assets, three checksum files, three provenance files, and the AGPL license asset under an immutable GitHub release.

- [ ] **Step 1: Dispatch the release workflow from merged main**

```bash
merge_sha=$(gh api repos/IngvarConsulting/unica-toolchain/commits/main --jq .sha)
gh workflow run release-tool.yml \
  --repo IngvarConsulting/unica-toolchain \
  --ref main \
  -f tool=v8-runner
gh run list \
  --repo IngvarConsulting/unica-toolchain \
  --workflow "Build tool release" \
  --limit 3 \
  --json databaseId,headSha,status,conclusion,createdAt,url
```

Verify `merge_sha` is the Task 1 merge commit. Select only a new run whose
`headSha` equals `merge_sha`; do not infer identity from list order.

- [ ] **Step 2: Wait for all matrix jobs and release publication**

```bash
run_id=$(gh run list \
  --repo IngvarConsulting/unica-toolchain \
  --workflow "Build tool release" \
  --limit 10 \
  --json databaseId,headSha \
  --jq --arg sha "$merge_sha" '[.[] | select(.headSha == $sha)][0].databaseId')
test -n "$run_id"
gh run watch "$run_id" --repo IngvarConsulting/unica-toolchain --exit-status
gh release view v8-runner-nightly-master-build.2 \
  --repo IngvarConsulting/unica-toolchain \
  --json tagName,isDraft,isPrerelease,assets,url
```

Expected: metadata, all three target builds, release file-set check, attestation, and release job succeed; release is neither draft nor prerelease.

- [ ] **Step 3: Download and inspect the exact release file set**

```bash
release_dir=$(mktemp -d /tmp/v8-runner-build-2.XXXXXX)
gh release download v8-runner-nightly-master-build.2 \
  --repo IngvarConsulting/unica-toolchain \
  --dir "$release_dir"
find "$release_dir" -maxdepth 1 -type f -print | sort
```

Expected files include exactly one binary, checksum, and provenance record for each target plus `license-v8-runner-AGPL-3.0-only.txt`; no archive wrapper or unexpected file is accepted.

- [ ] **Step 4: Verify checksums and provenance before touching Unica**

```bash
cd "$release_dir"
shasum -a 256 v8-runner-darwin-arm64 v8-runner-linux-x64 v8-runner-win-x64.exe
for checksum in checksums-v8-runner-*.txt; do shasum -a 256 --check "$checksum"; done
jq -e \
  '.releaseTag == "v8-runner-nightly-master-build.2"
   and .source.ref == "master"
   and .source.commit == "7ce1b062843d86644fe55741dbe0ee79f7ca767d"
   and .builder.rust == "1.95.0"' \
  provenance-v8-runner-*.json
```

Expected: every checksum verifies and every provenance document names the same source commit, release identity, and Rust toolchain. Preserve the three printed binary SHA-256 values for Task 3.

- [ ] **Step 5: Run the native smoke available on the current host**

```bash
chmod +x "$release_dir/v8-runner-darwin-arm64"
"$release_dir/v8-runner-darwin-arm64" --version
"$release_dir/v8-runner-darwin-arm64" build --help
```

Expected: both commands exit zero. Linux and Windows native smoke are proven by their corresponding successful workflow matrix jobs, not by cross-execution on macOS.

---

### Task 3: Switch Unica to the verified release

**Files:**
- Modify: `plugins/unica/third-party/tools.lock.json`
- Modify: `plugins/unica/references/tooling/v8project.md`
- Modify: `plugins/unica/skills/v8-runner/references/config-and-backends.md`
- Create: `docs/provenance/reviews/2026-08-10-v8-runner-master-refresh.json`
- Modify: `tests/ci/test_skill_provenance.py`
- Modify: `tests/ci/test_unica_skills.py`

**Interfaces:**
- Consumes: the exact commit, release tag, and three verified SHA-256 values from Task 2.
- Produces: a lock entry resolvable to real release bytes, user-facing strict platform guidance, and regression tests that keep the lock/docs synchronized.

- [ ] **Step 1: Put the detached Unica worktree on its implementation branch**

```bash
cd /Users/ingvarvilkman/.codex/worktrees/d7ea/unica
git switch -c codex/v8-runner-master-refresh
git status --short --branch
```

Expected: the branch contains design commit `85fe6055` and this plan commit; worktree is otherwise clean.

- [ ] **Step 2: Write the failing lock and documentation expectations**

In `tests/ci/test_skill_provenance.py`, change the expected v8-runner commit in `test_tool_lock_ref_uses_tools_lock_as_single_binary_baseline` to:

```python
"7ce1b062843d86644fe55741dbe0ee79f7ca767d"
```

In `tests/ci/test_unica_skills.py`, extend `test_v8_runner_docs_track_current_v8project_contract` with:

```python
self.assertIn("tools.platform.strict", v8_runner_docs)
self.assertIn("strict: true", v8_runner_docs)
self.assertIn("fail-closed", v8_runner_docs)
```

- [ ] **Step 3: Run the two tests and verify RED for the intended reasons**

```bash
python3.12 -m unittest \
  tests.ci.test_skill_provenance.SkillProvenanceTests.test_tool_lock_ref_uses_tools_lock_as_single_binary_baseline \
  tests.ci.test_unica_skills.UnicaSkillRoutingTests.test_v8_runner_docs_track_current_v8project_contract \
  -v
```

Expected: the provenance test reports old `72d346c0a8fcf8373d9388257d11e6bef0ad70b2`; the documentation test reports missing strict platform guidance. Syntax/import errors are not an acceptable RED state.

- [ ] **Step 4: Update the v8-runner lock entry from verified release bytes**

Use `apply_patch` on only the `v8-runner` object in `tools.lock.json`:

- `sourceCommit` becomes `7ce1b062843d86644fe55741dbe0ee79f7ca767d`;
- `assetTag` becomes `v8-runner-nightly-master-build.2`;
- each target `sha256` becomes the corresponding 64-character value printed and independently verified in Task 2.

Keep version, repositories, source tag, license, strategy, binary name, and asset names unchanged.

- [ ] **Step 5: Document opt-in strict platform resolution without widening MCP**

Add the following effective config shape to both runtime references:

```yaml
tools:
  platform:
    version: "8.3.27.1859"
    path: "C:\\Program Files\\1cv8\\8.3.27.1859\\bin"
    strict: true
```

The accompanying prose must name the key as `tools.platform.strict`, state that `strict: true` is opt-in and fail-closed, pins `1cv8`/`1cv8c`/`ibcmd` to one canonical installation root, rejects a missing utility or incompatible/unknown version, and permits a machine-local path from `v8project.local.yaml`. It must also say that omitted/false preserves legacy fallback and that this config field is not a new `unica.runtime.execute` argument.

- [ ] **Step 6: Add the append-only upstream review record**

Create `docs/provenance/reviews/2026-08-10-v8-runner-master-refresh.json` with:

- schema version `1`, review id and date `2026-08-10`;
- repository/ref, baseline `72d346c0a8fcf8373d9388257d11e6bef0ad70b2`, target `7ce1b062843d86644fe55741dbe0ee79f7ca767d`, and `reviewedCommits: 36`;
- applied groups: IBCMD data isolation, prepared-infobase `test --no-build` upstream availability, strict platform resolution, Windows directory publication, Windows detached stdio isolation;
- integration decision: binary/package refresh plus strict config documentation;
- deferred groups: public `noBuild` MCP argument, CDFI/private shadow issue `#30`/PR `#39`, dependency graph issue `#32`/PR `#50`;
- explicit statement that source-sync guard and public MCP surface remain unchanged.

This is a new historical record; do not rewrite dated records from June or July.

- [ ] **Step 7: Run targeted tests and verify GREEN**

```bash
python3.12 -m unittest \
  tests.ci.test_skill_provenance \
  tests.ci.test_unica_skills \
  tests.ci.test_build_unica_tools \
  -v
python3.12 scripts/ci/check-skill-upstreams.py --validate-only
git diff --check
```

Expected: all targeted tests pass; offline provenance validation reports no errors.

- [ ] **Step 8: Commit the coherent Unica update**

```bash
git add \
  plugins/unica/third-party/tools.lock.json \
  plugins/unica/references/tooling/v8project.md \
  plugins/unica/skills/v8-runner/references/config-and-backends.md \
  docs/provenance/reviews/2026-08-10-v8-runner-master-refresh.json \
  tests/ci/test_skill_provenance.py \
  tests/ci/test_unica_skills.py
git -c commit.gpgsign=false commit -m "build: refresh v8-runner master snapshot"
```

---

### Task 4: Verify the packaged consumer path and publish the Unica PR

**Files:**
- Verify: `scripts/ci/build-unica-tools.py`
- Verify: `scripts/ci/check-tool-contracts.py`
- Verify: `scripts/ci/package-unica-plugin.py`
- Verify: all files changed in Tasks 1–3

**Interfaces:**
- Consumes: committed lock and documentation from Task 3.
- Produces: local behavioral/package evidence and a reviewable Unica PR based on `main`.

- [ ] **Step 1: Build the current-host tool bundle from the updated lock**

```bash
bundle_dir=$(mktemp -d /tmp/unica-v8-runner-bundle.XXXXXX)
python3.12 scripts/ci/build-unica-tools.py \
  --target darwin-arm64 \
  --repo-root . \
  --out-dir "$bundle_dir" \
  --work-dir .build/v8-runner-master-refresh
python3.12 scripts/ci/check-tool-contracts.py \
  --target darwin-arm64 \
  --tools-dir "$bundle_dir/bin/darwin-arm64"
```

Expected: checksum-verified download of build 2, `--version`/`build --help`, partial-load BOM/CRLF/Cyrillic-path smoke, bounded external EPF stdout/stderr/wait smoke, and all other tool contracts pass.

- [ ] **Step 2: Run repository contract and package verification**

```bash
python3.12 -m unittest discover -s tests/ci
python3.12 scripts/ci/check-skill-upstreams.py --validate-only
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
git diff --check
git status --short
```

Expected: all checks pass with no untracked generated binaries. If a baseline failure appears, rerun the exact failing test and classify it before attributing it to the runner update.

- [ ] **Step 3: Prove the source-sync guard was not weakened**

```bash
cargo test -p unica-coder --lib --locked \
  application::tests::applied_incremental_dump_is_blocked_at_every_unica_runtime_entry_point \
  -- --exact --nocapture
cargo test -p unica-coder --lib --locked \
  application::tests::applied_dump_requires_explicit_full_mode_at_every_runtime_entry_point \
  -- --exact --nocapture
```

Expected: both exact guard tests pass before publication.

- [ ] **Step 4: Push and open a ready Unica PR**

Before running `gh pr create`, use `apply_patch` to create the ignored working
draft `.superpowers/v8-runner-unica-pr.md`. The body must link the immutable
toolchain release and its successful run, list the three verified SHA-256
values from Task 2, summarize the five applied upstream groups, and explicitly
state that CDFI `#30`, dependency graph `#32`, `noBuild` MCP exposure, and
removal of the full-only guard are deferred. Do not stage the draft.

```bash
git push -u origin codex/v8-runner-master-refresh
gh pr create \
  --repo IngvarConsulting/unica \
  --base main \
  --head codex/v8-runner-master-refresh \
  --title "build: refresh v8-runner master snapshot" \
  --body-file .superpowers/v8-runner-unica-pr.md
```

- [ ] **Step 5: Wait for GitHub checks and report readiness without merging Unica**

```bash
gh pr checks --watch --repo IngvarConsulting/unica
gh pr view --repo IngvarConsulting/unica \
  --json number,url,state,mergeable,reviewDecision,statusCheckRollup
```

Expected: the PR is independently reviewable against `main` and all required checks are green. Merging the Unica PR remains a separate user/reviewer approval; no Unica version bump or marketplace release is part of this plan.
