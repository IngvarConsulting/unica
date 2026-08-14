# RLM v1.33.0 Generational Cutover Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reproducibly publish RLM `v1.33.0` through `unica-toolchain`, measure the released and source CLIs on the issue #485 workspace, and switch Unica atomically to `rlm-bsl-mcp` / `rlm-bsl-index` with a cold builder-15 generation that never migrates or deletes builder 14.

**Architecture:** Delivery has three independently reviewable boundaries: a producer PR and immutable release in `unica-toolchain`, a prerequisite Unica PR for #487, and one Unica consumer PR for the binary pin plus generation cutover. The consumer derives every builder-15 path from the same safe `workspaceRoot + sourceRoot` state root, scopes the RLM database and orchestrator status/lock to `index-v15`, and keeps all builder-14 bytes untouched for rollback.

**Tech Stack:** Python 3.12, `uv`, PyInstaller, GitHub Actions and `gh`; Rust 2021, serde/serde_json, the existing `WorkspaceIndexService` and persistent MCP transport; Python `unittest` benchmark and package guards.

## Global Constraints

- Upstream repository: `https://github.com/Dach-Coin/rlm-tools-bsl`.
- Upstream source tag: `v1.33.0`; exact commit: `3e6920cd015a61af4ba7aa1a5f1fedd8bc935549`.
- Toolchain manifest/release identity remains `rlm-tools-bsl`; first immutable release identity is `rlm-tools-bsl-v1.33.0-build.1`.
- Upstream `console_scripts` names remain `sourceName = rlm-tools-bsl` and `sourceName = rlm-bsl-index`; only released/installed `assetBase` and `binaryName` become `rlm-bsl-mcp` and `rlm-bsl-index`.
- Toolchain Python package and modules remain `rlm_tools_bsl`, `rlm_tools_bsl.server`, and `rlm_tools_bsl.cli`.
- The consumer may change its pin only after all six native executable assets, three checksum files, three provenance files, and the MIT license asset have been independently downloaded and verified.
- Public MCP remains one server named `unica`; no public `unica.*` name, argument, result, or provider-selection contract changes.
- Issue #487 is a separate prerequisite PR. The RLM consumer PR must not hide the existing unsafe-root defect behind a renamed directory.
- Issue #487 must preserve ADR-0014: only `unica-bootstrap/src/host/**` may read host-specific data roots. It passes the resolved exact provider-state directory to `unica-coder` as host-neutral `UNICA_PROVIDER_STATE_DIR`; the platform-boundary guard is not weakened.
- ADR-0018 governs over legacy path compatibility: every normalized `workspaceRoot + sourceRoot` pair receives a SHA-256-scoped provider-state root even when `cacheRoot` is already outside `sourceRoot`. Existing legacy bytes are neither moved nor deleted; the new pair builds cold.
- In #487, the currently pinned builder-14 status and lock also move below the SHA-256 pair root. The old shared `<cacheRoot>/caches/bsl_index_status.json` and `<cacheRoot>/locks/bsl_index.lock` are not read, rewritten, or deleted and cannot select `building`, `update`, or terminal failure for the new pair.
- Builder 15 uses `<safe-provider-state-root>/rlm-bsl/index-v15`; builder 14 is neither opened, updated, copied, migrated, nor deleted.
- Builder-15 status and lock live under `caches/rlm-bsl/index-v15/` and `locks/rlm-bsl/index-v15/` below the same safe state root. Legacy status/lock files do not gate builder 15.
- `missing` starts one background cold `build`; `incomplete` starts one background `update`; `building` and `incomplete` never permit an RLM-backed read.
- The old binary name `rlm-tools-bsl` is absent from the new runtime. No compatibility executable or launcher is added.
- Existing dated provenance snapshots are not rewritten. This delivery adds one new RLM `v1.33.0` review record containing the actual released hashes.
- Raw benchmark JSON, absolute workspace paths, and client object names stay under ignored `docs-local/`; only sanitized aggregates reach GitHub issues.
- Every defect found while executing the plan follows RED → GREEN. A pre-existing defect not caused by the current PR is split into an independent `main`-based PR.
- PRs are not stacked on another open PR head. The toolchain PR targets `unica-toolchain/main`; #487 and the consumer PR each target `unica/main`.

---

## Delivery Units and File Structure

### `unica-toolchain` producer PR

- Create `tests/test_rlm_manifest.py`
  - Own the exact RLM source, release identity, upstream entrypoints, released names, and expected release file set.
- Modify `manifests/rlm-tools-bsl.json`
  - Pin `v1.33.0`, build revision `1`, the exact source commit, and new asset bases.
- Do not modify `toolchain/builders/python_pyinstaller.py`
  - Its distinction between `sourceName` and `assetBase` already supports this rename.

### Unica prerequisite PR for #487

- Modify `crates/unica-coder/src/infrastructure/workspace_index.rs`
  - Own the safe RLM provider-state parent and the legacy `rlm-tools-bsl` index directory used before the version bump.
- Modify `crates/unica-coder/src/infrastructure/workspace_services.rs`
  - Pass the resolved source root to the persistent RLM transport and use the same directory as the CLI.
- Modify `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
  - Prove builder and MCP reader use one external directory for `source-set.path: "."`.
- Modify `spec/architecture/invariants.md` and `spec/architecture/runtime.md`
  - Record the already-required safe placement as a derived ADR-0018 rule; no new public contract is introduced.

### Unica benchmark and consumer PR

- Create `scripts/dev/benchmark-rlm-index.py`
  - Own deterministic scenario selection, mutation/restoration, timed CLI execution, provenance capture, and local JSON.
- Create `tests/dev/test_benchmark_rlm_index.py`
  - Prove clean-tree refusal, deterministic selection, guaranteed restoration, and raw sample schema.
- Modify `plugins/unica/third-party/tools.lock.json`
  - Pin the released pair and add explicit `releaseName: "rlm-tools-bsl"` to separate release-group identity from executable identity.
- Create `docs/provenance/reviews/2026-08-13-rlm-v1-33-product-update.json`
  - Preserve exact toolchain release assets, hashes, source identity, and the reviewed compatibility conclusion.
- Modify `tests/ci/test_skill_provenance.py`, `tests/ci/test_build_unica_tools.py`, `tests/ci/test_attributions.py`
  - Bind the lock, review record, release group, and attribution markers to the new pair.
- Modify `scripts/ci/check-tool-contracts.py` and `tests/ci/test_product_contracts.py`
  - Rename the MCP executable and exercise the exact MCP/index helper surface Unica consumes.
- Modify `crates/unica-coder/src/infrastructure/workspace_index.rs`
  - Scope database/status/lock to builder-15 generation and classify `incomplete` explicitly.
- Modify `crates/unica-coder/src/infrastructure/workspace_services.rs`, `crates/unica-coder/src/infrastructure/code_intelligence.rs`, and `crates/unica-coder/src/infrastructure/rlm_navigation.rs`
  - Start `rlm-bsl-mcp` with the builder-15 directory and carry the retryable incomplete/read barrier through internal responses.
- Modify `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`, `tests/ci/test_package_unica_runtime.py`, and `tests/ci/test_unica_mcp_script_parity.py`
  - Prove the renamed packaged process and new generation work on the platform boundary.
- Modify `plugins/unica/ATTRIBUTIONS.md`, `plugins/unica/README.md`, `spec/provenance/README.md`, `spec/architecture/glossary.md`, `spec/architecture/runtime.md`, and `spec/architecture/building-blocks.md`
  - Synchronize internal names, version, release provenance, and generation layout.
- Modify `spec/architecture/invariants.md`, `spec/decisions/0059-rlm-generacionnyy-perehod.md`, and `spec/decisions/README.md`
  - Add the executable generation-cutover invariant and accept ADR-0059 with the implementation.

---

### Task 1: Pin the Toolchain Manifest Contract Before Changing the Manifest

**Repository:** `/Users/ingvarvilkman/Documents/git/unica-toolchain`

**Files:**
- Create: `tests/test_rlm_manifest.py`
- Test: `tests/test_rlm_manifest.py`

**Interfaces:**
- Consumes: `load_manifest`, `release_tag`, `expected_asset_names`, and `expected_release_files` from `toolchain.manifest`.
- Produces: an executable contract for release `rlm-tools-bsl-v1.33.0-build.1` and the six renamed assets.

- [ ] **Step 1: Create a clean toolchain worktree from current `origin/main`**

```bash
git fetch origin main
git worktree add \
  /Users/ingvarvilkman/Documents/git/unica-toolchain-rlm-v1-33 \
  -b codex/rlm-v1-33 \
  origin/main
```

Verify:

```bash
git -C /Users/ingvarvilkman/Documents/git/unica-toolchain-rlm-v1-33 status --short --branch
git -C /Users/ingvarvilkman/Documents/git/unica-toolchain-rlm-v1-33 rev-parse HEAD origin/main
```

Expected: clean branch and equal commit IDs before edits.

- [ ] **Step 2: Add the failing repository-manifest test**

Create `tests/test_rlm_manifest.py`:

```python
from pathlib import Path
import unittest

from toolchain.manifest import (
    PythonBuilderSpec,
    expected_asset_names,
    expected_release_files,
    load_manifest,
    release_tag,
)


REPO_ROOT = Path(__file__).resolve().parents[1]


class RlmManifestTests(unittest.TestCase):
    def test_v1_33_release_keeps_upstream_entrypoints_and_renames_assets(self) -> None:
        manifest = load_manifest(REPO_ROOT / "manifests" / "rlm-tools-bsl.json")

        self.assertIsInstance(manifest.builder, PythonBuilderSpec)
        self.assertEqual(manifest.name, "rlm-tools-bsl")
        self.assertEqual(manifest.version, "1.33.0")
        self.assertEqual(manifest.build_revision, 1)
        self.assertEqual(manifest.source.kind, "release")
        self.assertEqual(manifest.source.ref, "v1.33.0")
        self.assertEqual(
            manifest.source.commit,
            "3e6920cd015a61af4ba7aa1a5f1fedd8bc935549",
        )
        self.assertEqual(release_tag(manifest), "rlm-tools-bsl-v1.33.0-build.1")

        self.assertEqual(
            [
                (binary.source_name, binary.asset_base, binary.package, binary.module)
                for binary in manifest.builder.binaries
            ],
            [
                ("rlm-tools-bsl", "rlm-bsl-mcp", "rlm_tools_bsl", "rlm_tools_bsl.server"),
                ("rlm-bsl-index", "rlm-bsl-index", "rlm_tools_bsl", "rlm_tools_bsl.cli"),
            ],
        )
        self.assertEqual(
            expected_asset_names(manifest),
            {
                "rlm-bsl-mcp-darwin-arm64",
                "rlm-bsl-mcp-linux-x64",
                "rlm-bsl-mcp-win-x64.exe",
                "rlm-bsl-index-darwin-arm64",
                "rlm-bsl-index-linux-x64",
                "rlm-bsl-index-win-x64.exe",
            },
        )
        self.assertEqual(
            expected_release_files(manifest) - expected_asset_names(manifest),
            {
                "license-rlm-tools-bsl-MIT.txt",
                "checksums-rlm-tools-bsl-darwin-arm64.txt",
                "checksums-rlm-tools-bsl-linux-x64.txt",
                "checksums-rlm-tools-bsl-win-x64.txt",
                "provenance-rlm-tools-bsl-darwin-arm64.json",
                "provenance-rlm-tools-bsl-linux-x64.json",
                "provenance-rlm-tools-bsl-win-x64.json",
            },
        )


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 3: Run the focused test and verify RED**

```bash
python3.12 -m unittest tests.test_rlm_manifest -v
```

Expected: FAIL because the checked-in manifest still names `v1.29.1`, build revision `2`, the old commit, and `assetBase = rlm-tools-bsl`.

- [ ] **Step 4: Commit only the failing contract**

```bash
git add tests/test_rlm_manifest.py
git commit -m "test(rlm): specify v1.33 toolchain release"
```

---

### Task 2: Update, Build, and Publish RLM v1.33.0 Through `unica-toolchain`

**Repository:** `/Users/ingvarvilkman/Documents/git/unica-toolchain-rlm-v1-33`

**Files:**
- Modify: `manifests/rlm-tools-bsl.json`
- Test: `tests/test_rlm_manifest.py`

**Interfaces:**
- Consumes: the exact contract from Task 1.
- Produces: immutable release `rlm-tools-bsl-v1.33.0-build.1` and a local evidence JSON containing the downloaded SHA-256 values.

- [ ] **Step 1: Apply the minimal manifest update**

Change only these values in `manifests/rlm-tools-bsl.json`:

```json
{
  "version": "1.33.0",
  "buildRevision": 1,
  "source": {
    "kind": "release",
    "repository": "https://github.com/Dach-Coin/rlm-tools-bsl",
    "ref": "v1.33.0",
    "commit": "3e6920cd015a61af4ba7aa1a5f1fedd8bc935549"
  }
}
```

Keep the first binary's upstream entrypoint and change only its released base:

```json
{
  "package": "rlm_tools_bsl",
  "sourceName": "rlm-tools-bsl",
  "module": "rlm_tools_bsl.server",
  "assetBase": "rlm-bsl-mcp",
  "smokeArgs": ["--help"]
}
```

Keep the second binary exactly:

```json
{
  "package": "rlm_tools_bsl",
  "sourceName": "rlm-bsl-index",
  "module": "rlm_tools_bsl.cli",
  "assetBase": "rlm-bsl-index",
  "smokeArgs": ["--help"]
}
```

- [ ] **Step 2: Run manifest and repository tests and verify GREEN**

```bash
python3.12 -m unittest tests.test_rlm_manifest -v
python3.12 -m unittest discover -s tests
python3.12 -m py_compile scripts/*.py toolchain/*.py toolchain/builders/*.py tests/*.py
python3.12 scripts/toolchain.py describe \
  --manifest manifests/rlm-tools-bsl.json
```

Expected: all tests pass; `describe` reports `rlm-tools-bsl-v1.33.0-build.1` and exactly 13 release files.

- [ ] **Step 3: Validate the pinned source before compiling**

```bash
python3.12 scripts/toolchain.py validate-source \
  --manifest manifests/rlm-tools-bsl.json \
  --repo-root . \
  --work-dir .build/source-rlm-tools-bsl \
  --out-dir dist/source-rlm-tools-bsl
```

Expected JSON fields:

```json
{
  "commit": "3e6920cd015a61af4ba7aa1a5f1fedd8bc935549",
  "patches": [],
  "licenses": ["license-rlm-tools-bsl-MIT.txt"]
}
```

- [ ] **Step 4: Build the Darwin asset through the real toolchain before opening the PR**

```bash
python3.12 scripts/toolchain.py build \
  --manifest manifests/rlm-tools-bsl.json \
  --repo-root . \
  --target darwin-arm64 \
  --work-dir .build/rlm-tools-bsl-darwin-arm64 \
  --out-dir dist/rlm-tools-bsl-darwin-arm64
```

Verify the produced bytes rather than a source launcher:

```bash
dist/rlm-tools-bsl-darwin-arm64/rlm-bsl-mcp-darwin-arm64 --help
dist/rlm-tools-bsl-darwin-arm64/rlm-bsl-index-darwin-arm64 --help
shasum -a 256 dist/rlm-tools-bsl-darwin-arm64/rlm-bsl-*-darwin-arm64
```

Expected: both `--help` calls exit `0`; the first output exposes stdio transport and the second exposes `index build`, `index update`, and `index info`.

- [ ] **Step 5: Commit and open the independent toolchain PR**

```bash
git add manifests/rlm-tools-bsl.json
git commit -m "build(rlm): publish v1.33 assets"
git push -u origin codex/rlm-v1-33
gh pr create \
  --repo IngvarConsulting/unica-toolchain \
  --base main \
  --head codex/rlm-v1-33 \
  --title "build(rlm): publish v1.33.0 assets" \
  --body "Pins Dach-Coin/rlm-tools-bsl v1.33.0 at 3e6920cd015a61af4ba7aa1a5f1fedd8bc935549 and publishes rlm-bsl-mcp / rlm-bsl-index without changing upstream console entrypoint names. Consumer: IngvarConsulting/unica#488."
```

Wait for checks, review the diff, merge to `main`, then refresh the exact merged commit:

```bash
gh pr checks --repo IngvarConsulting/unica-toolchain --watch
gh pr merge --repo IngvarConsulting/unica-toolchain --merge --delete-branch
git fetch origin main
```

- [ ] **Step 6: Dispatch the release only from merged `main`**

```bash
gh workflow run release-tool.yml \
  --repo IngvarConsulting/unica-toolchain \
  --ref main \
  -f tool=rlm-tools-bsl
gh run list \
  --repo IngvarConsulting/unica-toolchain \
  --workflow release-tool.yml \
  --limit 1
```

Watch the returned run ID to completion:

```bash
gh run watch RUN_ID --repo IngvarConsulting/unica-toolchain --exit-status
```

`RUN_ID` is the numeric ID printed by the immediately preceding `gh run list`; do not select an older run.

- [ ] **Step 7: Independently download and verify the immutable release**

```bash
release_dir="$(mktemp -d)"
gh release download rlm-tools-bsl-v1.33.0-build.1 \
  --repo IngvarConsulting/unica-toolchain \
  --dir "$release_dir"
find "$release_dir" -maxdepth 1 -type f -print | sort
```

Expected: the exact 13-file set from Task 1. Verify each target checksum from its own checksum file:

```bash
(cd "$release_dir" && shasum -a 256 -c checksums-rlm-tools-bsl-darwin-arm64.txt)
(cd "$release_dir" && shasum -a 256 -c checksums-rlm-tools-bsl-linux-x64.txt)
(cd "$release_dir" && shasum -a 256 -c checksums-rlm-tools-bsl-win-x64.txt)
```

Verify every provenance record:

```bash
python3.12 - "$release_dir" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

root = Path(sys.argv[1])
expected_commit = "3e6920cd015a61af4ba7aa1a5f1fedd8bc935549"
expected_tag = "rlm-tools-bsl-v1.33.0-build.1"
evidence = {
    "schemaVersion": 1,
    "releaseTag": expected_tag,
    "sourceCommit": expected_commit,
    "assets": {},
}
for path in sorted(root.glob("provenance-rlm-tools-bsl-*.json")):
    data = json.loads(path.read_text(encoding="utf-8"))
    assert data["releaseTag"] == expected_tag
    assert data["source"]["commit"] == expected_commit
    for asset in data["assets"]:
        binary = root / asset["name"]
        digest = hashlib.sha256(binary.read_bytes()).hexdigest()
        assert digest == asset["sha256"]
        evidence["assets"][asset["name"]] = digest
assert len(evidence["assets"]) == 6
(root / "release-evidence.json").write_text(
    json.dumps(evidence, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
print(root / "release-evidence.json")
PY
```

GitHub release assets preserve bytes but not the POSIX executable bit. After
checksum and provenance verification, make only the two local downloaded
Darwin copies executable, verify that their SHA-256 values are unchanged, and
run the host-native released bytes again:

```bash
chmod +x \
  "$release_dir/rlm-bsl-mcp-darwin-arm64" \
  "$release_dir/rlm-bsl-index-darwin-arm64"
shasum -a 256 \
  "$release_dir/rlm-bsl-mcp-darwin-arm64" \
  "$release_dir/rlm-bsl-index-darwin-arm64"
"$release_dir/rlm-bsl-mcp-darwin-arm64" --help
"$release_dir/rlm-bsl-index-darwin-arm64" --help
```

If any release check fails, do not replace the release. Increase `buildRevision`, open a new toolchain PR, and publish a new immutable release identity.

---

### Task 3: Close #487 in an Independent Unica Prerequisite PR

**Repository:** `/Users/ingvarvilkman/Documents/git/unica`

**Files:**
- Modify: `crates/unica-bootstrap/src/host/runtime_cache.rs`
- Modify: `crates/unica-bootstrap/src/host/mod.rs`
- Modify: `crates/unica-bootstrap/src/lib.rs`
- Modify: `crates/unica-bootstrap/src/main.rs`
- Modify: `crates/unica-bootstrap/src/platform/process.rs`
- Modify: `crates/unica-bootstrap/src/verification.rs`
- Modify: `crates/unica-bootstrap/tests/platform/verification_contract.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Modify: `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/architecture/runtime.md`

**Interfaces:**
- Produces: host-facade `provider_state_root() -> Result<PathBuf>` and host-neutral runtime variable `UNICA_PROVIDER_STATE_DIR`.
- Produces: `pub(crate) fn rlm_provider_state_root(context: &WorkspaceContext, source_root: &Path) -> Result<PathBuf, String>`.
- Produces: `pub(crate) fn rlm_index_dir(context: &WorkspaceContext, source_root: &Path) -> Result<PathBuf, String>` for the currently pinned builder-14 layout.
- Produces: pair-scoped builder-14 `status_path(context, source_root)` and `lock_path(context, source_root)` below the same provider-state root; legacy shared coordination files are nonbinding.
- Consumes: normalized `workspaceRoot + sourceRoot`, `path_starts_with_host_root`, and the existing host data roots.

- [ ] **Step 1: Create a clean #487 worktree from current `origin/main`**

```bash
git fetch origin main
git worktree add \
  /Users/ingvarvilkman/Documents/git/unica/.worktrees/issue-487-rlm-state-root \
  -b codex/issue-487-rlm-state-root \
  origin/main
```

- [ ] **Step 2: Add failing unit tests for safe and unsafe roots**

Add tests beside the `workspace_index.rs` path tests. Use an injected external base so tests do not mutate process-global environment:

```rust
#[test]
fn rlm_provider_state_scopes_the_existing_safe_cache_layout() {
    let context = test_context("safe-provider-root");
    let source_root = context.workspace_root.join("src");
    fs::create_dir_all(&source_root).unwrap();

    let actual = rlm_provider_state_root_with(
        &context,
        &source_root,
        Some(context.workspace_root.parent().unwrap().join("host-data")),
    )
    .unwrap();

    let safe_parent = normalize_path_identity(&context.cache_root.join("provider-state")).unwrap();
    assert!(actual.starts_with(&safe_parent));
    assert_ne!(actual, normalize_path_identity(&context.cache_root).unwrap());
    assert_eq!(actual.file_name().unwrap().to_string_lossy().len(), 68);
    assert!(actual.file_name().unwrap().to_string_lossy().starts_with("rlm-"));
    cleanup(&context);
}

#[test]
fn rlm_provider_state_moves_outside_a_workspace_wide_source_root() {
    let mut context = test_context("unsafe-provider-root");
    context.cache_root = context.workspace_root.join(".build/unica");
    let external = context.workspace_root.parent().unwrap().join("host-data");

    let first = rlm_provider_state_root_with(
        &context,
        &context.workspace_root,
        Some(external.clone()),
    )
    .unwrap();
    let second = rlm_provider_state_root_with(
        &context,
        &context.workspace_root,
        Some(external),
    )
    .unwrap();

    assert_eq!(first, second);
    assert!(!path_starts_with_host_root(&first, &context.workspace_root));
    assert_eq!(rlm_index_dir_with_root(&first), first.join("rlm-tools-bsl"));
    cleanup(&context);
}

#[test]
fn rlm_provider_state_separates_source_roots() {
    let context = test_context("separate-provider-roots");
    let external = context.workspace_root.parent().unwrap().join("host-data");
    let first_source = context.workspace_root.join("src/configuration");
    let second_source = context.workspace_root.join("src/extension");
    fs::create_dir_all(&first_source).unwrap();
    fs::create_dir_all(&second_source).unwrap();

    let first = rlm_provider_state_root_with(&context, &first_source, Some(external.clone())).unwrap();
    let second = rlm_provider_state_root_with(&context, &second_source, Some(external)).unwrap();

    assert_ne!(first, second);
    cleanup(&context);
}
```

- [ ] **Step 3: Run the tests and verify RED**

```bash
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::rlm_provider_state_ -- --nocapture
```

Expected: compilation fails because the shared safe-root functions do not exist.

- [ ] **Step 4: Implement one safe-root resolver and reuse it for builder and reader**

In `workspace_index.rs`, add a normalized identity key. ADR-0018 takes precedence over legacy path compatibility: both safe and unsafe roots receive the pair identity, while every old directory remains untouched:

```rust
const LEGACY_RLM_INDEX_DIR_NAME: &str = "rlm-tools-bsl";

pub(crate) fn rlm_provider_state_root(
    context: &WorkspaceContext,
    source_root: &Path,
) -> Result<PathBuf, String> {
    rlm_provider_state_root_with(context, source_root, neutral_provider_state_root())
}

fn rlm_provider_state_root_with(
    context: &WorkspaceContext,
    source_root: &Path,
    external_base: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let preferred = normalize_path_identity(&context.cache_root)?;
    let workspace = normalize_path_identity(&context.workspace_root)?;
    let source = normalize_path_identity(source_root)?;
    let base = if !path_starts_with_host_root(&preferred, &source) {
        preferred.join("provider-state")
    } else {
        external_base.ok_or_else(|| {
            "UNICA_PROVIDER_STATE_DIR, HOME, or USERPROFILE is required for RLM state outside sourceRoot".to_string()
        })?
    };
    let mut hasher = Sha256::new();
    for component in [&workspace, &source] {
        hasher.update(provider_state_path_identity(component));
        hasher.update([0]);
    }
    let identity = format!("{:x}", hasher.finalize());
    let root = normalize_path_identity(&base.join(format!("rlm-{identity}")))?;
    if path_starts_with_host_root(&root, &source) {
        return Err("failed to place RLM state outside the indexed source tree".to_string());
    }
    Ok(root)
}

fn neutral_provider_state_root() -> Option<PathBuf> {
    std::env::var_os("UNICA_PROVIDER_STATE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from)
                .map(|home| home.join(".unica").join("provider-state"))
        })
}

fn rlm_index_dir_with_root(root: &Path) -> PathBuf {
    root.join(LEGACY_RLM_INDEX_DIR_NAME)
}

pub(crate) fn rlm_index_dir(
    context: &WorkspaceContext,
    source_root: &Path,
) -> Result<PathBuf, String> {
    rlm_provider_state_root(context, source_root)
        .map(|root| rlm_index_dir_with_root(&root))
}
```

Import `provider_state_path_identity`, `path_starts_with_host_root`, and `sha2::{Digest, Sha256}`. Keep `provider_state_path_identity` separate from lock identity: it returns the normalized raw `OsStr` bytes on Unix, preserving both path case and distinct non-UTF-8 byte paths, while retaining the required existing Windows lock normalization as bytes. Do not change containment semantics or the existing lock-coalescing identity. The SHA-256 identity is persisted in the directory name, so it must remain stable across Rust releases; `DefaultHasher` is forbidden here because the standard library does not promise a stable algorithm, and lossy Unicode conversion is forbidden because it can collapse distinct Unix paths. When the normal cache is safe, the pair root is `<cacheRoot>/provider-state/rlm-<sha256>`; otherwise it is `<UNICA_PROVIDER_STATE_DIR>/rlm-<sha256>`. The old `<cacheRoot>/rlm-tools-bsl` and old external directories remain byte-for-byte untouched. `unica-coder` may read only the host-neutral `UNICA_PROVIDER_STATE_DIR`; `HOME`/`USERPROFILE` provide a direct-development fallback under `.unica/provider-state`. Host-specific variables and `.codex`/`.claude` path knowledge remain forbidden in this crate. Both `WorkspaceIndexService::commands` and `PersistentMcpSession::start_rlm_transport` must call `rlm_index_dir`; remove their duplicate `context.cache_root.join("rlm-tools-bsl")` expressions. Pass `source_root` into `start_rlm_transport` from `RlmMcpSession::start`.

The same pair root owns current builder-14 coordination metadata: `status_path(context, source_root)` returns `<pair-root>/caches/bsl_index_status.json` and `lock_path(context, source_root)` returns `<pair-root>/locks/bsl_index.lock`. Thread `source_root` through status/lock reads and writes, recovery, readiness, and test helpers. Do not consult the legacy shared paths under `context.cache_root`; this makes terminal, retryable, and fresh-lock legacy records nonbinding without parsing or migrating their schema. A source-root resolution failure writes no shared marker. Add regressions proving each of those three legacy records leaves the new pair `Missing`, selects cold `build`, and leaves every legacy byte unchanged.

In `crates/unica-bootstrap/src/host/runtime_cache.rs`, add a separately tested `provider_state_root()` resolution chain. A fully expanded host-neutral `UNICA_PROVIDER_STATE_DIR` wins; otherwise the host facade resolves the supported host data/home roots and appends `unica/provider-state`. Export it through `host/mod.rs` and `lib.rs`. `main.rs` resolves it before launching or verifying the installed runtime. Pass the exact path to `launch_runtime` and `verify_mcp_runtime`; those functions set only `UNICA_PROVIDER_STATE_DIR` on the child `Command`. Do not mutate process-global environment. On Unix the `exec` command and on Windows the supervised child must receive the same value.

Add host-facade tests for override, each supported host root, HOME/USERPROFILE fallback, unexpanded-token rejection, empty-value rejection, and missing-root error. Empty provider overrides, published roots, declared roots, `HOME`, and `USERPROFILE` do not become relative bases and do not suppress the next valid fallback. Extend platform launch/verification fixtures to record and assert the neutral variable. Run `python3.12 scripts/ci/check-rust-platform-boundary.py` and `python3.12 -m unittest tests.ci.test_rust_platform_boundary`; zero diagnostics is a Task 3 acceptance requirement.

- [ ] **Step 5: Add the platform regression proving both processes share the path**

Extend the issue-89 fixture to record `RLM_INDEX_DIR` for `rlm-bsl-index` and `rlm-tools-bsl`, then assert:

```rust
assert_eq!(indexer_rlm_index_dir, reader_rlm_index_dir);
assert!(!path_starts_with_host_root(
    Path::new(&indexer_rlm_index_dir),
    &workspace_root,
));
```

The fixture workspace must use:

```yaml
source-set:
  - name: main
    type: CONFIGURATION
    path: .
  - name: extension
    type: EXTENSION
    path: src/extension
```

Keep the second source set so this remains a multi-source lifecycle regression; changing `main` to `path: .` must not silently remove the prior coverage.

- [ ] **Step 6: Run focused and full prerequisite verification**

```bash
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::rlm_provider_state_
cargo test -p unica-coder --lib infrastructure::workspace_services::tests::bsl_analyzer_cache_
cargo test -p unica-coder --test issue_89_workspace_service -- --test-threads=1
cargo test -p unica-bootstrap
python3.12 scripts/ci/check-rust-platform-boundary.py
python3.12 -m unittest tests.ci.test_rust_platform_boundary
python3.12 scripts/ci/check-architecture-sync.py --base origin/main --strict
git diff --check
```

- [ ] **Step 7: Update the derived rule, commit, and open the independent PR**

Add `INV-CACHE-PROVIDER-STATE-OUTSIDE-SOURCE` under `CACHE`:

```markdown
#### INV-CACHE-PROVIDER-STATE-OUTSIDE-SOURCE — Постоянное состояние поставщика не индексирует само себя

- **Rule:** Постоянное состояние RLM выводится из нормализованных `workspaceRoot + sourceRoot`, остаётся вне индексируемого `sourceRoot`, изолирует разные рабочие пространства, worktree и корни исходников и передаётся одинаково индексатору и читающему процессу.
- **Decision:** ADR-0018
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace_index.rs`
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
- **Scope:** runtime
```

Commit and publish:

```bash
git add crates/unica-coder/src/infrastructure/workspace_index.rs \
  crates/unica-coder/src/infrastructure/workspace_services.rs \
  crates/unica-coder/tests/platform/issue_89_workspace_service.rs \
  spec/architecture/invariants.md \
  spec/architecture/runtime.md
git commit -m "fix(cache): keep RLM state outside source root"
git push -u origin codex/issue-487-rlm-state-root
gh pr create \
  --repo IngvarConsulting/unica \
  --base main \
  --head codex/issue-487-rlm-state-root \
  --title "fix(cache): keep RLM state outside source root" \
  --body "Closes #487. Keeps the existing safe layout, but derives an isolated external provider-state root when sourceRoot contains the normal workspace cache."
```

Merge #487 before creating the consumer branch. Do not base the consumer PR on this PR's head.

---

### Task 4: Add a Reproducible RLM Benchmark Harness

**Repository:** a fresh Unica consumer worktree from `origin/main` after #487 merges.

**Files:**
- Create: `scripts/dev/benchmark-rlm-index.py`
- Create: `tests/dev/test_benchmark_rlm_index.py`

**Interfaces:**
- Produces: `Scenario`, `Sample`, `ensure_clean`, `select_inputs`, `run_incremental_scenario`, `result_document`, `run_benchmark`, and `markdown_summary`.
- Produces JSON schema version `1` with executable/source provenance, exact repo-relative selected files, raw samples, index statistics, peak RSS, index size, and final clean-tree proof.
- Consumes: an explicit RLM index executable and an existing clean Git workspace; never the user's normal RLM cache.

- [ ] **Step 1: Create the consumer worktree only after the #487 merge is on `origin/main`**

```bash
git fetch origin main
git worktree add \
  /Users/ingvarvilkman/Documents/git/unica/.worktrees/rlm-v1-33-consumer \
  -b codex/rlm-v1-33-consumer \
  origin/main
```

- [ ] **Step 2: Add failing harness tests**

Create `tests/dev/test_benchmark_rlm_index.py` using `importlib.util` to load the hyphenated script. Cover these exact assertions:

```python
def test_refuses_a_dirty_tracked_tree(self) -> None:
    repo = self.git_fixture()
    (repo / "src" / "CommonModules" / "One" / "Module.bsl").write_text(
        "Процедура One(Changed)\nКонецПроцедуры\n",
        encoding="utf-8",
    )
    with self.assertRaisesRegex(RuntimeError, "tracked Git tree must be clean"):
        MODULE.ensure_clean(repo)

def test_selects_exact_deterministic_scenario_sizes(self) -> None:
    repo = self.git_fixture(bsl_count=120, root_xml_count=12, form_xml_count=2)
    selected = MODULE.select_inputs(repo)
    self.assertEqual(len(selected["bsl-1"]), 1)
    self.assertEqual(len(selected["bsl-10"]), 10)
    self.assertEqual(len(selected["bsl-100"]), 100)
    self.assertEqual(len(selected["xml-form-1"]), 1)
    self.assertEqual(len(selected["xml-root-10"]), 10)
    self.assertEqual(selected, MODULE.select_inputs(repo))
    self.assertTrue(all(not path.is_absolute() for paths in selected.values() for path in paths))

def test_restores_files_and_runs_reverse_update_after_a_failed_measurement(self) -> None:
    repo = self.git_fixture()
    selected = MODULE.select_inputs(repo)["bsl-1"]
    calls = []
    with self.assertRaisesRegex(RuntimeError, "measured update failed"):
        MODULE.run_incremental_scenario(
            repo=repo,
            paths=selected,
            marker="UNICA_RLM_BENCHMARK_MARKER",
            measured_update=lambda: (_ for _ in ()).throw(RuntimeError("measured update failed")),
            reverse_update=lambda: calls.append("reverse"),
        )
    MODULE.ensure_clean(repo)
    self.assertEqual(calls, ["reverse"])

def test_result_keeps_raw_samples_and_provenance(self) -> None:
    result = MODULE.result_document(
        label="packaged-v1.33.0",
        source_commit="3e6920cd015a61af4ba7aa1a5f1fedd8bc935549",
        executable_sha256="a" * 64,
        repo_head="b" * 40,
        selected={"bsl-1": [Path("src/CommonModules/One/Module.bsl")]},
        samples={"bsl-1": [MODULE.Sample(5.2, 120_000_000, True, "fresh")]},
        final_clean=True,
    )
    self.assertEqual(result["schemaVersion"], 1)
    self.assertEqual(result["samples"]["bsl-1"][0]["durationSeconds"], 5.2)
    self.assertTrue(result["finalClean"])

def test_markdown_summary_rejects_absolute_workspace_paths(self) -> None:
    document = MODULE.result_document(
        label="packaged-v1.33.0",
        source_commit="3e6920cd015a61af4ba7aa1a5f1fedd8bc935549",
        executable_sha256="a" * 64,
        repo_head="b" * 40,
        selected={"bsl-1": [Path("/client/workspace/Secret/Module.bsl")]},
        samples={"bsl-1": [MODULE.Sample(5.2, 120_000_000, True, "fresh")]},
        final_clean=True,
    )
    with self.assertRaisesRegex(RuntimeError, "summary contains an absolute path"):
        MODULE.markdown_summary([document])
```

The test class's `git_fixture` creates `src/CommonModules/ModuleNNN/Module.bsl`,
`src/Documents/DocumentNNN.xml`, and
`src/Documents/Document000/Forms/FormNNN/Ext/Form.xml`, then initializes Git,
sets a local test identity, adds all files, and commits with signing disabled.
It returns the repository root only after `git status --porcelain` is empty.

- [ ] **Step 3: Run harness tests and verify RED**

```bash
python3.12 -m unittest tests.dev.test_benchmark_rlm_index -v
```

Expected: FAIL because the script and interfaces do not exist.

- [ ] **Step 4: Implement the deterministic scenario contract**

The script must define:

```python
@dataclass(frozen=True)
class Scenario:
    name: str
    repeats: int
    paths_key: str | None


SCENARIOS = (
    Scenario("cold-build", 1, None),
    Scenario("noop-update", 5, None),
    Scenario("bsl-1", 6, "bsl-1"),
    Scenario("bsl-10", 6, "bsl-10"),
    Scenario("bsl-100", 6, "bsl-100"),
    Scenario("xml-form-1", 6, "xml-form-1"),
    Scenario("xml-root-10", 6, "xml-root-10"),
)


@dataclass(frozen=True)
class Sample:
    duration_seconds: float
    peak_rss_bytes: int | None
    git_fast_path: bool | None
    final_status: str
```

Selection uses only `git ls-files -z`, sorts repo-relative paths, and applies these predicates:

```python
def is_bsl(path: Path) -> bool:
    return path.suffix.lower() == ".bsl"

def is_form_xml(path: Path) -> bool:
    parts = path.parts
    return path.name == "Form.xml" and "Forms" in parts and "Ext" in parts

def is_root_xml(path: Path) -> bool:
    return path.suffix.lower() == ".xml" and "Ext" not in path.parts and path.name != "Configuration.xml"
```

`ensure_clean` executes `git status --porcelain --untracked-files=no` and refuses any output. Mutation appends a BSL comment or XML comment containing the marker. Restoration always runs in `finally`:

```python
try:
    mutate(repo, paths, marker)
    return measured_update()
finally:
    subprocess.run(
        ["git", "restore", "--source=HEAD", "--", *map(str, paths)],
        cwd=repo,
        check=True,
    )
    reverse_update()
    ensure_clean(repo)
```

Every RLM process receives only:

```python
env = {
    **os.environ,
    "RLM_INDEX_DIR": str(index_dir),
}
```

The script accepts these required arguments:

```text
--repo PATH
--executable PATH
--label packaged-v1.33.0|source-v1.33.0
--source-commit 40_HEX
--index-dir PATH
--output PATH
```

Reject `--index-dir` when it is equal to, contains, or is contained by `--repo`. Require the index directory to be empty before `cold-build`. Record executable SHA-256, `git rev-parse HEAD`, Python/platform identity, per-command stdout/stderr tails, parsed `Fast path: True`, `Status`, `Modules`, `Methods`, DB size, recursive index bytes, and a `finalClean` boolean. Exit non-zero unless final status is `fresh`, the marker is absent from tracked files, and the tracked tree is clean.

The same script also accepts `--summarize RESULT... --append-to BODY`. That
mode computes count/median/min/max from raw arrays, never emits `selected` file
names, rejects any absolute path in the generated Markdown, and appends one
idempotent section headed `## Замер RLM v1.33.0`.

- [ ] **Step 5: Run tests and developer guards and verify GREEN**

```bash
python3.12 -m unittest tests.dev.test_benchmark_rlm_index -v
python3.12 -m py_compile scripts/dev/benchmark-rlm-index.py tests/dev/test_benchmark_rlm_index.py
python3.12 -m unittest discover -s tests/dev
git diff --check
```

- [ ] **Step 6: Commit the harness separately on the consumer branch**

```bash
git add scripts/dev/benchmark-rlm-index.py tests/dev/test_benchmark_rlm_index.py
git commit -m "test(rlm): add reproducible index benchmark"
```

---

### Task 5: Run the Packaged and Source v1.33.0 Benchmarks and Update #485

**Files:**
- Local only: `docs-local/benchmarks/rlm-v1.33.0/packaged.json`
- Local only: `docs-local/benchmarks/rlm-v1.33.0/source.json`
- GitHub: issue `IngvarConsulting/unica#485`

**Interfaces:**
- Consumes: released Darwin binary and `release-evidence.json` from Task 2, plus the harness from Task 4.
- Produces: two raw comparable runs and one sanitized issue section.

- [ ] **Step 1: Prepare the exact source CLI without changing the measured workspace**

```bash
source_checkout="$(mktemp -d)"
git clone --filter=blob:none --branch v1.33.0 \
  https://github.com/Dach-Coin/rlm-tools-bsl.git \
  "$source_checkout"
test "$(git -C "$source_checkout" rev-parse HEAD)" = \
  "3e6920cd015a61af4ba7aa1a5f1fedd8bc935549"
uv sync --frozen --no-dev --directory "$source_checkout" --python python3.12
"$source_checkout/.venv/bin/rlm-bsl-index" --help
```

- [ ] **Step 2: Prove the benchmark workspace is the unchanged #485 fixture**

```bash
BENCHMARK_REPO=/Users/ingvarvilkman/Documents/git/Sendbox
test -d "$BENCHMARK_REPO/.git"
git -C "$BENCHMARK_REPO" diff --quiet
git -C "$BENCHMARK_REPO" diff --cached --quiet
git -C "$BENCHMARK_REPO" status --porcelain --untracked-files=no
test "$(git -C "$BENCHMARK_REPO" ls-files | wc -l | tr -d ' ')" = "48159"
git -C "$BENCHMARK_REPO" rev-parse HEAD
```

This is the same real Sendbox checkout used for the earlier 48,159-file
measurements. Stop if the tracked-file count or clean-tree proof differs.

- [ ] **Step 3: Run the packaged executable with its own empty index directory**

```bash
mkdir -p docs-local/benchmarks/rlm-v1.33.0
release_dir="$(mktemp -d)"
gh release download rlm-tools-bsl-v1.33.0-build.1 \
  --repo IngvarConsulting/unica-toolchain \
  --dir "$release_dir"
(cd "$release_dir" && \
  shasum -a 256 -c checksums-rlm-tools-bsl-darwin-arm64.txt)
python3.12 - "$release_dir" <<'PY'
import hashlib
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
provenance = json.loads(
    (root / "provenance-rlm-tools-bsl-darwin-arm64.json").read_text()
)
assert provenance["schemaVersion"] == 3
assert provenance["releaseTag"] == "rlm-tools-bsl-v1.33.0-build.1"
assert provenance["source"]["commit"] == \
    "3e6920cd015a61af4ba7aa1a5f1fedd8bc935549"
assets = {asset["name"]: asset["sha256"] for asset in provenance["assets"]}
path = root / "rlm-bsl-index-darwin-arm64"
actual = hashlib.sha256(path.read_bytes()).hexdigest()
assert actual == assets[path.name]
(root / "packaged-index.sha256").write_text(actual + "\n")
PY
chmod +x "$release_dir/rlm-bsl-index-darwin-arm64"
test "$(shasum -a 256 "$release_dir/rlm-bsl-index-darwin-arm64" | cut -d ' ' -f 1)" = \
  "$(tr -d '\n' < "$release_dir/packaged-index.sha256")"
packaged_state="$(mktemp -d)"
python3.12 scripts/dev/benchmark-rlm-index.py \
  --repo "$BENCHMARK_REPO" \
  --executable "$release_dir/rlm-bsl-index-darwin-arm64" \
  --label packaged-v1.33.0 \
  --source-commit 3e6920cd015a61af4ba7aa1a5f1fedd8bc935549 \
  --index-dir "$packaged_state" \
  --output docs-local/benchmarks/rlm-v1.33.0/packaged.json
```

- [ ] **Step 4: Run the exact source commit with a second empty index directory**

```bash
source_state="$(mktemp -d)"
python3.12 scripts/dev/benchmark-rlm-index.py \
  --repo "$BENCHMARK_REPO" \
  --executable "$source_checkout/.venv/bin/rlm-bsl-index" \
  --label source-v1.33.0 \
  --source-commit 3e6920cd015a61af4ba7aa1a5f1fedd8bc935549 \
  --index-dir "$source_state" \
  --output docs-local/benchmarks/rlm-v1.33.0/source.json
```

- [ ] **Step 5: Verify restoration and provenance before using the numbers**

```bash
git -C "$BENCHMARK_REPO" diff --quiet
git -C "$BENCHMARK_REPO" diff --cached --quiet
test -z "$(git -C "$BENCHMARK_REPO" grep -l UNICA_RLM_BENCHMARK_MARKER -- . || true)"
python3.12 -m json.tool docs-local/benchmarks/rlm-v1.33.0/packaged.json >/dev/null
python3.12 -m json.tool docs-local/benchmarks/rlm-v1.33.0/source.json >/dev/null
```

Compare the packaged executable hash in JSON with `release-evidence.json`. Confirm both result documents contain the same selected repo-relative files and repo HEAD. Do not average incomparable selections.

- [ ] **Step 6: Append a sanitized v1.33.0 section to issue #485**

Generate a Markdown table from the raw arrays using Python `statistics.median`, minimum, and maximum. The section must include:

- release tag and source commit;
- packaged Darwin SHA-256;
- separate packaged/source rows for all seven scenarios;
- count, median, and observed range;
- cold build RSS/index size and final module/method counts;
- comparison with the existing v1.29.1 source rows;
- an explicit statement whether the measured data changes `quiet period = 5 s`, `max batch delay = 30 s`, or provider deadline `45 s`.

Before editing the issue, assert the generated Markdown contains neither the benchmark root nor any selected object path. Then update through a body file:

```bash
issue_body="$(mktemp)"
gh issue view 485 --repo IngvarConsulting/unica --json body --jq .body > "$issue_body"
python3.12 scripts/dev/benchmark-rlm-index.py \
  --summarize \
  docs-local/benchmarks/rlm-v1.33.0/packaged.json \
  docs-local/benchmarks/rlm-v1.33.0/source.json \
  --append-to "$issue_body"
gh issue edit 485 --repo IngvarConsulting/unica --body-file "$issue_body"
```

The harness implementation in Task 4 must provide this `--summarize ... --append-to` mode and reject absolute paths in emitted Markdown.

---

### Task 6: Pin the New Consumer Identities, Assets, and Provenance

**Repository:** `/Users/ingvarvilkman/Documents/git/unica/.worktrees/rlm-v1-33-consumer`

**Files:**
- Modify: `plugins/unica/third-party/tools.lock.json`
- Create: `docs/provenance/reviews/2026-08-13-rlm-v1-33-product-update.json`
- Modify: `tests/ci/test_skill_provenance.py`
- Modify: `tests/ci/test_build_unica_tools.py`
- Modify: `plugins/unica/ATTRIBUTIONS.md`
- Modify: `tests/ci/test_attributions.py`
- Modify: `plugins/unica/README.md`

**Interfaces:**
- Consumes: the six exact SHA-256 values in Task 2 `release-evidence.json`.
- Produces: lock identities `rlm-bsl-mcp` and `rlm-bsl-index`, both with `releaseName = rlm-tools-bsl` and one source/release tuple.

- [ ] **Step 1: Add failing consumer identity tests before editing the lock**

Replace the old RLM test in `test_skill_provenance.py` with a test that loads the new review record and asserts:

```python
expected_names = {"rlm-bsl-mcp", "rlm-bsl-index"}
self.assertEqual(set(review["tools"]), expected_names)
for name in expected_names:
    locked = locked_tools[name]
    recorded = review["tools"][name]
    self.assertEqual(locked["version"], "1.33.0")
    self.assertEqual(locked["sourceTag"], "v1.33.0")
    self.assertEqual(locked["sourceCommit"], "3e6920cd015a61af4ba7aa1a5f1fedd8bc935549")
    self.assertEqual(locked["assetTag"], "rlm-tools-bsl-v1.33.0-build.1")
    self.assertEqual(locked["releaseName"], "rlm-tools-bsl")
    self.assertEqual(locked["assets"], recorded["assets"])
```

In `test_build_unica_tools.py`, change the expected external tool set to include `rlm-bsl-mcp`, and resolve release identity with:

```python
declared_release_name = tool.get("releaseName", tool["name"])
self.assertEqual(release_name, declared_release_name)
self.assertTrue(
    any(
        candidate.get("releaseName", candidate["name"]) == declared_release_name
        and (
            candidate["repository"],
            candidate["sourceTag"],
            candidate["sourceCommit"],
        ) == source_identity
        for candidate in external_tools
    )
)
```

Update attribution expectations to require markers `tool rlm-bsl-mcp` and `tool rlm-bsl-index` in one section.

- [ ] **Step 2: Run the focused tests and verify RED**

```bash
python3.12 -m unittest \
  tests.ci.test_skill_provenance \
  tests.ci.test_build_unica_tools \
  tests.ci.test_attributions
```

Expected: failures name the old lock/attribution identity and the absent v1.33 review record.

- [ ] **Step 3: Add the immutable review record from release evidence**

Create `docs/provenance/reviews/2026-08-13-rlm-v1-33-product-update.json` with this schema and the exact six asset entries from `release-evidence.json`:

```json
{
  "schemaVersion": 1,
  "id": "2026-08-13-rlm-v1-33-product-update",
  "generatedAt": "2026-08-13",
  "source": {
    "repository": "https://github.com/Dach-Coin/rlm-tools-bsl",
    "tag": "v1.33.0",
    "commit": "3e6920cd015a61af4ba7aa1a5f1fedd8bc935549"
  },
  "toolchain": {
    "repository": "https://github.com/IngvarConsulting/unica-toolchain",
    "releaseTag": "rlm-tools-bsl-v1.33.0-build.1",
    "buildRevision": 1
  },
  "tools": {
    "rlm-bsl-mcp": {"assets": {}},
    "rlm-bsl-index": {"assets": {}}
  },
  "compatibility": {
    "builder": "15",
    "previousBuilder": "14",
    "strategy": "cold-generation-cutover",
    "legacyStateDeleted": false,
    "publicMcpChanged": false
  }
}
```

Populate each `assets` object as the lock expects: target keys mapping to `{assetName, sha256}`. The file is evidence from the immutable release, not a copy of the old 2026-08-12 snapshot.

- [ ] **Step 4: Update both lock records atomically**

The MCP record becomes:

```json
{
  "name": "rlm-bsl-mcp",
  "version": "1.33.0",
  "repository": "https://github.com/Dach-Coin/rlm-tools-bsl",
  "sourceTag": "v1.33.0",
  "sourceCommit": "3e6920cd015a61af4ba7aa1a5f1fedd8bc935549",
  "assetRepository": "https://github.com/IngvarConsulting/unica-toolchain",
  "assetTag": "rlm-tools-bsl-v1.33.0-build.1",
  "releaseName": "rlm-tools-bsl",
  "license": "MIT",
  "assetStrategy": "direct-release-asset",
  "binaryName": "rlm-bsl-mcp"
}
```

The index record keeps `name` and `binaryName = rlm-bsl-index`, receives the same version/source/release fields, and also receives `releaseName = rlm-tools-bsl`. Copy target asset names and SHA-256 values exactly from the new review record.

- [ ] **Step 5: Synchronize attribution and package prose**

Change the grouped attribution heading/markers to:

```markdown
### rlm-bsl-mcp и rlm-bsl-index

<!-- unica-attribution: tool rlm-bsl-mcp -->
<!-- unica-attribution: tool rlm-bsl-index -->

- Закреплённая версия: `1.33.0`, commit `3e6920cd015a61af4ba7aa1a5f1fedd8bc935549`
```

Keep the upstream repository and MIT license path named `rlm-tools-bsl`; that is source identity, not executable identity. Update the plugin README runtime list to `rlm-bsl-mcp` and `rlm-bsl-index`.

- [ ] **Step 6: Run provenance and lock tests and verify GREEN**

```bash
python3.12 -m unittest \
  tests.ci.test_skill_provenance \
  tests.ci.test_build_unica_tools \
  tests.ci.test_attributions
python3.12 scripts/ci/check-attributions.py
git diff --check
```

- [ ] **Step 7: Commit the consumer supply-chain boundary**

```bash
git add plugins/unica/third-party/tools.lock.json \
  docs/provenance/reviews/2026-08-13-rlm-v1-33-product-update.json \
  tests/ci/test_skill_provenance.py \
  tests/ci/test_build_unica_tools.py \
  plugins/unica/ATTRIBUTIONS.md \
  tests/ci/test_attributions.py \
  plugins/unica/README.md
git commit -m "build(rlm): pin v1.33 toolchain assets"
```

---

### Task 7: Rename the Runtime Contract and Exercise the Actual v1.33 Helpers

**Files:**
- Modify: `scripts/ci/check-tool-contracts.py`
- Modify: `tests/ci/test_product_contracts.py`
- Modify: `tests/ci/test_unica_mcp_script_parity.py`
- Modify: `tests/fixtures/unica_mcp_script_parity/reader-standins/bsl_mcp.py`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Modify: `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`

**Interfaces:**
- Produces: help contract label/tool `rlm-bsl-mcp`.
- Produces: `check_rlm_mcp_contract(mcp_tool, index_tool) -> list[str]`, using the same JSON-RPC protocol and fixed helpers as the Rust adapter.
- Consumes: actual bundled v1.33 bytes in the three-target `build-tools` job.

- [ ] **Step 1: Rename stand-ins and add failing tool-contract assertions**

Replace every internal executable lookup/fixture key `rlm-tools-bsl` with `rlm-bsl-mcp`, except source repository, release identity, license directory, and historical prose. Update `TOOL_HELP_CHECKS`:

```python
(
    "rlm-bsl-mcp server",
    "rlm-bsl-mcp",
    ["--help"],
    ["--transport", "stdio", "streamable-http"],
),
```

Add a test where only `rlm-tools-bsl` exists and assert the contract reports `rlm-bsl-mcp: binary not found`. Add a passing fixture named `rlm-bsl-mcp`.

- [ ] **Step 2: Run the contract tests and verify RED**

```bash
python3.12 -m unittest tests.ci.test_product_contracts -v
cargo test -p unica-coder --lib infrastructure::workspace_services::tests:: -- --nocapture
```

Expected: the old resolver and fixtures still require `rlm-tools-bsl`.

- [ ] **Step 3: Implement a bounded MCP contract probe for consumed helpers**

Build a two-module temporary Git fixture using the same source text as `check_rlm_mtime_recovery_contract`, build it with `rlm-bsl-index`, then launch `rlm-bsl-mcp` with the same temporary `RLM_INDEX_DIR`. Send newline-delimited JSON-RPC:

```python
initialize = {
    "jsonrpc": "2.0",
    "id": 1,
    "method": "initialize",
    "params": {
        "protocolVersion": "2025-03-26",
        "capabilities": {},
        "clientInfo": {"name": "unica-contract", "version": "1"},
    },
}
initialized = {"jsonrpc": "2.0", "method": "notifications/initialized"}
```

Call `rlm_start` with the same argument contract as Rust:

```python
{
    "path": str(workspace),
    "query": "ContractTest1",
    "effort": "low",
    "max_output_chars": 100000,
    "max_execute_calls": 10000,
    "execution_timeout_seconds": 30,
    "include_metadata": False,
}
```

Then call `rlm_execute` three times with JSON-emitting code for:

```python
search("ContractTest", scope="all", limit=20)
find_definition("ContractTest1", module_hint=None, limit=20)
get_object_profile("CommonModule.ContractOne", sections=None, include_flow=False, include_code_usages=False, limit=20)
```

Validate that each tool result is non-error, `stdout` parses as JSON, definition entries preserve list-valued `params`, and `_meta.truncated`/`total_is_lower_bound` values are boolean when present. Close with `rlm_end`. Bound every read and process wait to 120 seconds and always terminate the process tree in `finally`.

Do not add a `parse_form` adapter: repository search proves Unica does not consume `parse_form(...).attributes[].types`. The contract probe covers only `WorkspaceRlmOperation` helpers actually emitted by `fixed_rlm_helper_code`.

- [ ] **Step 4: Route the new binary in Rust and platform fixtures**

Change only the executable lookup:

```rust
let program = resolve_bundled_tool(&plugin_root, "rlm-bsl-mcp", true)?.program;
```

Update platform fixture lists and the parity stand-in key to `rlm-bsl-mcp`. Keep `rlm_tools_bsl.server` out of runtime code; the packaged manifest owns executable resolution.

- [ ] **Step 5: Run renamed contracts and verify GREEN**

```bash
python3.12 -m unittest tests.ci.test_product_contracts
python3.12 -m unittest tests.ci.test_unica_mcp_script_parity
cargo test -p unica-coder --lib infrastructure::workspace_services::tests::
cargo test -p unica-coder --test issue_89_workspace_service -- --test-threads=1
```

- [ ] **Step 6: Commit the runtime identity change**

```bash
git add scripts/ci/check-tool-contracts.py \
  tests/ci/test_product_contracts.py \
  tests/ci/test_unica_mcp_script_parity.py \
  tests/fixtures/unica_mcp_script_parity/reader-standins/bsl_mcp.py \
  crates/unica-coder/src/infrastructure/workspace_services.rs \
  crates/unica-coder/tests/platform/issue_89_workspace_service.rs
git commit -m "refactor(rlm): rename bundled MCP executable"
```

---

### Task 8: Cut Over Database, Status, and Lock to Builder-15 Generation

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Modify: `crates/unica-coder/src/infrastructure/code_intelligence.rs`
- Modify: `crates/unica-coder/src/infrastructure/rlm_navigation.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_state.rs`
- Test: the same Rust modules and `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`

**Interfaces:**
- Produces: `const RLM_PRODUCT_DIR: &str = "rlm-bsl"` and `const RLM_INDEX_GENERATION: &str = "index-v15"`.
- Produces: `rlm_generation_root`, generation-scoped `status_path`/`lock_path`, and `IndexReadiness::Incomplete`.
- Consumes: `rlm_provider_state_root` merged by #487.

- [ ] **Step 1: Add failing generation and incomplete-state regressions**

Add tests with these exact properties:

```rust
#[test]
fn builder_15_uses_a_new_generation_and_leaves_builder_14_untouched() {
    let context = test_context("generation-cutover");
    let source_root = context.workspace_root.join("src");
    fs::create_dir_all(&source_root).unwrap();
    let state_root = rlm_provider_state_root(&context, &source_root).unwrap();
    let legacy = state_root.join("rlm-tools-bsl");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("builder-14-sentinel"), "keep").unwrap();

    let commands = WorkspaceIndexService::with_runner(&RecordingIndexRunner::default())
        .commands(&context, &source_root, &CancellationToken::new())
        .unwrap();
    let actual = PathBuf::from(&commands.info.env[0].1);

    assert_eq!(actual, state_root.join("rlm-bsl/index-v15"));
    assert_eq!(fs::read_to_string(legacy.join("builder-14-sentinel")).unwrap(), "keep");
    cleanup(&context);
}

#[test]
fn legacy_status_and_lock_do_not_gate_builder_15() {
    let context = test_context("legacy-markers");
    let source_root = context.workspace_root.join("src");
    fs::create_dir_all(&source_root).unwrap();
    fs::create_dir_all(context.cache_root.join("caches")).unwrap();
    fs::create_dir_all(context.cache_root.join("locks")).unwrap();
    fs::write(context.cache_root.join("caches/bsl_index_status.json"), "legacy").unwrap();
    fs::write(context.cache_root.join("locks/bsl_index.lock"), "legacy").unwrap();

    assert!(!status_path(&context, &source_root).unwrap().ends_with("caches/bsl_index_status.json"));
    assert!(!lock_path(&context, &source_root).unwrap().ends_with("locks/bsl_index.lock"));
    assert_eq!(fs::read_to_string(context.cache_root.join("caches/bsl_index_status.json")).unwrap(), "legacy");
    assert_eq!(fs::read_to_string(context.cache_root.join("locks/bsl_index.lock")).unwrap(), "legacy");
    cleanup(&context);
}

#[test]
fn incomplete_info_is_retryable_but_never_readable() {
    let readiness = readiness_from_info(&IndexOutput::success(
        "Index: /tmp/bsl_index.db\n  Status:   incomplete\n",
    ));
    assert_eq!(readiness, IndexReadiness::Incomplete);
}

#[test]
fn incomplete_starts_update_not_build() {
    let context = test_context("incomplete-update");
    fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
    let runner = RecordingIndexRunner {
        outputs: RefCell::new(vec![IndexOutput::success(
            "Index: /tmp/bsl_index.db\n  Status:   incomplete\n",
        )]),
        ..Default::default()
    };

    let report = WorkspaceIndexService::with_runner(&runner)
        .start_for_workspace(&context, &Map::new(), false);

    assert_eq!(report.warnings, vec!["rlm index recovery started"]);
    assert_eq!(runner.backgrounds.borrow()[0].primary.args[0..2], ["index", "update"]);
    cleanup(&context);
}
```

Add adapter tests asserting `Incomplete` becomes retryable `index_pending:` and never an output section.

- [ ] **Step 2: Run focused tests and verify RED**

```bash
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::builder_15_uses_a_new_generation -- --nocapture
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::legacy_status_and_lock_do_not_gate_builder_15 -- --nocapture
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::incomplete_ -- --nocapture
```

Expected: compilation fails on the new generation helpers and `Incomplete` variant.

- [ ] **Step 3: Introduce one generation-root function and use it everywhere**

Add:

```rust
const RLM_PRODUCT_DIR: &str = "rlm-bsl";
const RLM_INDEX_GENERATION: &str = "index-v15";

pub(crate) fn rlm_generation_root(
    context: &WorkspaceContext,
    source_root: &Path,
) -> Result<PathBuf, String> {
    Ok(rlm_provider_state_root(context, source_root)?
        .join(RLM_PRODUCT_DIR)
        .join(RLM_INDEX_GENERATION))
}

pub fn status_path(
    context: &WorkspaceContext,
    source_root: &Path,
) -> Result<PathBuf, String> {
    Ok(rlm_provider_state_root(context, source_root)?
        .join("caches")
        .join(RLM_PRODUCT_DIR)
        .join(RLM_INDEX_GENERATION)
        .join(STATUS_FILE_NAME))
}

fn lock_path(
    context: &WorkspaceContext,
    source_root: &Path,
) -> Result<PathBuf, String> {
    Ok(rlm_provider_state_root(context, source_root)?
        .join("locks")
        .join(RLM_PRODUCT_DIR)
        .join(RLM_INDEX_GENERATION)
        .join(LOCK_FILE_NAME))
}
```

Thread `Result<PathBuf, String>` through command, readiness, status, and lock
paths so a safe-root failure reports unavailable and writes no in-source
marker. Tests use `.unwrap()` only after constructing a valid external base.

Reject every existing symbolic-link or Windows reparse-point component below
the provider root on the database, status, and lock generation routes. Do not
prove containment by canonicalizing both sides after following such an alias:
`rlm-bsl/index-v15 -> rlm-tools-bsl/index` must be unavailable, not a valid
builder-15 database, and status/lock aliases must never redirect writes into
retained builder-14 state.

Refactor `read_bsl_index_status`, `write_status`, `failed_status_for_source`, `active_lock`, `bind_readiness_to_source_generation`, and test helpers to accept `source_root`. `bsl_index_is_ready(context)` resolves the default source root with `resolve_source_root(context, None)` and reads only that generation marker.

- [ ] **Step 4: Add `Incomplete` to every internal exhaustive boundary**

Extend the enum:

```rust
pub enum IndexReadiness {
    Ready { db_path: PathBuf },
    Missing,
    Stale { status: String },
    Building,
    Incomplete,
    Failed(String),
    Unavailable(String),
}
```

Parse exact upstream output:

```rust
Some("incomplete") => IndexReadiness::Incomplete,
```

In `start_for_workspace`, map `Incomplete` to a single-flight background `index update` with warning `rlm index recovery started`. In `ready_index`, return `Incomplete`. In service serialization use `index_status = "incomplete"`; in `unavailable_rlm_execution`, `rlm_navigation`, and `code_intelligence`, map it to retryable/pending wording. Never map it to `Ready`, `Missing`, or a successful empty result.

- [ ] **Step 5: Point both executables at the generation root**

`WorkspaceIndexService::commands` sets:

```rust
let env = vec![(
    "RLM_INDEX_DIR".to_string(),
    rlm_generation_root(context, source_root)?.into_os_string(),
)];
```

`PersistentMcpSession::start_rlm_transport` receives `source_root` and sets the
same native value. Carry it through `IndexCommand` and `ManagedCommand` as
`OsString`/`PathBuf`; `display().to_string()` is allowed only for human
diagnostics, not environment transport. Add both the ordinary cross-platform
comparison and a real invalid-byte Unix regression proving builder and reader
receive byte-identical `RLM_INDEX_DIR`.

- [ ] **Step 6: Preserve recovery and read barriers through the whole lifecycle**

Add worker tests:

- successful `update` from `Incomplete` followed by `fresh info` writes a ready builder-15 marker;
- failed/cancelled recovery keeps a non-ready marker and releases its generation lock;
- concurrent recovery requests create one background job;
- old builder-14 sentinel, DB, status, and lock bytes remain byte-for-byte unchanged;
- a ready builder-15 marker with matching source generation permits reads;
- any active builder-15 lock wins over a ready marker and returns `Building`.

Run:

```bash
cargo test -p unica-coder --lib infrastructure::workspace_index::tests:: -- --test-threads=1
cargo test -p unica-coder --lib infrastructure::workspace_services::tests:: -- --test-threads=1
cargo test -p unica-coder --lib infrastructure::code_intelligence::tests:: -- --test-threads=1
cargo test -p unica-coder --lib infrastructure::rlm_navigation::tests:: -- --test-threads=1
cargo test -p unica-coder --test issue_89_workspace_service -- --test-threads=1
```

- [ ] **Step 7: Commit the generational runtime change**

```bash
git add crates/unica-coder/src/infrastructure/workspace_index.rs \
  crates/unica-coder/src/infrastructure/workspace_services.rs \
  crates/unica-coder/src/infrastructure/code_intelligence.rs \
  crates/unica-coder/src/infrastructure/rlm_navigation.rs \
  crates/unica-coder/src/infrastructure/workspace_state.rs \
  crates/unica-coder/tests/platform/issue_89_workspace_service.rs
git commit -m "feat(rlm): cut over to builder 15 generation"
```

---

### Task 9: Synchronize Architecture, Packaging, and Runtime Absence Contracts

**Files:**
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/decisions/0059-rlm-generacionnyy-perehod.md`
- Modify: `spec/decisions/README.md`
- Modify: `spec/architecture/runtime.md`
- Modify: `spec/architecture/building-blocks.md`
- Modify: `spec/architecture/glossary.md`
- Modify: `spec/provenance/README.md`
- Modify: `tests/ci/test_package_unica_runtime.py`
- Modify: any active non-historical file reported by the final old-name scan.

**Interfaces:**
- Produces: `INV-CACHE-GENERATION-CUTOVER` owned by ADR-0059.
- Produces: package test proving `rlm-bsl-mcp` exists and `rlm-tools-bsl` does not.

- [ ] **Step 1: Add failing package composition assertions**

Extend the runtime fixture with `rlm-bsl-mcp` and `rlm-bsl-index`, then assert archive members contain both and exclude the old name:

```python
self.assertIn("bin/linux-x64/rlm-bsl-mcp", members)
self.assertIn("bin/linux-x64/rlm-bsl-index", members)
self.assertNotIn("bin/linux-x64/rlm-tools-bsl", members)
```

Run:

```bash
python3.12 -m unittest tests.ci.test_package_unica_runtime -v
```

Expected: RED until the fixture and expected archive inventory use the new pair.

- [ ] **Step 2: Add the generation-cutover invariant**

Under `CACHE`, add:

```markdown
#### INV-CACHE-GENERATION-CUTOVER — Несовместимый индекс получает новое поколение

- **Rule:** Для постоянного корня нормализованной пары `workspaceRoot + sourceRoot` несовместимая версия построителя `RLM` получает каталог данных `rlm-bsl/index-v15`, маркер состояния `caches/rlm-bsl/index-v15/bsl_index_status.json` и маркер блокировки `locks/rlm-bsl/index-v15/bsl_index.lock`; новая версия строит это поколение с нуля, не открывает, не обновляет и не удаляет предыдущее поколение, а состояния `building` и `incomplete` запрещают чтение, поддерживаемое `RLM`.
- **Decision:** ADR-0059
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace_index.rs`
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
- **Scope:** runtime, packaged
```

Change ADR-0059 status from `proposed` to `accepted`; move it from the proposed list to the accepted chronology in `spec/decisions/README.md`. Because ADR-0059 is not yet on `main`, edit it directly rather than creating a replacement ADR. The earlier draft number ADR-0056 was renumbered before publication because ADR-0056 through ADR-0058 had already landed on `main`.

- [ ] **Step 3: Synchronize active prose without rewriting history**

Update active runtime/glossary/building-block/provenance text to:

- executable pair `rlm-bsl-mcp` / `rlm-bsl-index`;
- upstream/release group `rlm-tools-bsl`;
- version `1.33.0` at the exact commit;
- state directory `rlm-bsl/index-v15`;
- generation-scoped status/lock;
- old builder 14 retained and ignored.

Do not edit older dated design, plan, or provenance-review files. Exclude them from the stale-name scan:

```bash
rg -n 'rlm-tools-bsl|rlm-bsl-index|rlm-bsl-mcp' \
  crates plugins tests scripts spec \
  --glob '!docs/design/**' \
  --glob '!docs/plans/**' \
  --glob '!docs/provenance/reviews/**'
```

Every remaining `rlm-tools-bsl` occurrence must be classified as upstream
repository, toolchain release identity, license path, an immutable historical
ADR, or a defect. Runtime executable lookups and package inventories may not
use it.

- [ ] **Step 4: Run docs/package guards and verify GREEN**

```bash
python3.12 -m unittest \
  tests.ci.test_design_documents \
  tests.ci.test_architecture_registry \
  tests.ci.test_product_contracts \
  tests.ci.test_package_unica_runtime \
  tests.ci.test_skill_provenance \
  tests.ci.test_attributions \
  tests.ci.test_build_unica_tools
python3.12 scripts/ci/check-architecture-sync.py --base origin/main --strict
python3.12 scripts/ci/check-attributions.py
git diff --check
```

- [ ] **Step 5: Commit architecture and package synchronization**

```bash
git add spec/architecture/invariants.md \
  spec/decisions/0059-rlm-generacionnyy-perehod.md \
  spec/decisions/README.md \
  spec/architecture/runtime.md \
  spec/architecture/building-blocks.md \
  spec/architecture/glossary.md \
  spec/provenance/README.md \
  tests/ci/test_package_unica_runtime.py
git commit -m "docs(rlm): accept generation cutover contract"
```

---

### Task 10: Verify the Extracted Runtime on All Targets and Open the Consumer PR

**Files:**
- Verification only; fix defects in the task that introduced them.

**Interfaces:**
- Consumes: published immutable release, merged #487 prerequisite, consumer commits, and three-target GitHub Actions.
- Produces: a main-based consumer PR with reproducible evidence and `Closes #488`.

- [ ] **Step 1: Run the complete local source suite**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --package unica-coder --lib -- --test-threads=1
cargo test --package unica-coder --test issue_89_workspace_service -- --test-threads=1
cargo test --workspace -- --test-threads=1
python3.12 -m unittest discover -s tests/ci --durations 20
python3.12 -m unittest discover -s tests/dev --durations 20
python3.12 -m py_compile scripts/ci/*.py tests/ci/*.py
python3.12 -m py_compile scripts/dev/*.py tests/dev/*.py
python3.12 scripts/ci/check-version-contract.py
python3.12 scripts/ci/check-architecture-sync.py --base origin/main --strict
git diff --check
```

- [ ] **Step 2: Build and inspect the local Darwin bundle from the published assets**

```bash
python3.12 scripts/ci/build-unica-tools.py \
  --repo-root . \
  --target darwin-arm64 \
  --lock-file plugins/unica/third-party/tools.lock.json \
  --out-dir .build/tool-bundles/darwin-arm64 \
  --work-dir .build/tool-work
python3.12 scripts/ci/check-tool-contracts.py \
  --target darwin-arm64 \
  --tools-dir .build/tool-bundles/darwin-arm64/bin/darwin-arm64
python3.12 scripts/ci/package-unica-runtime.py \
  --bundle-root .build/tool-bundles/darwin-arm64 \
  --out-dir .build/runtime-assets/darwin-arm64
```

Extract and prove the inventory:

```bash
runtime_root="$(mktemp -d)"
tar -xzf .build/runtime-assets/darwin-arm64/unica-runtime-darwin-arm64.tar.gz \
  -C "$runtime_root"
test -x "$runtime_root/bin/darwin-arm64/rlm-bsl-mcp"
test -x "$runtime_root/bin/darwin-arm64/rlm-bsl-index"
test ! -e "$runtime_root/bin/darwin-arm64/rlm-tools-bsl"
"$runtime_root/bin/darwin-arm64/rlm-bsl-mcp" --help
"$runtime_root/bin/darwin-arm64/rlm-bsl-index" --help
```

- [ ] **Step 3: Push and open one main-based consumer PR**

```bash
git status --short --branch
git push -u origin codex/rlm-v1-33-consumer
gh pr create \
  --repo IngvarConsulting/unica \
  --base main \
  --head codex/rlm-v1-33-consumer \
  --title "feat(rlm): cut over to v1.33 builder generation" \
  --body "Closes #488. Consumes immutable rlm-tools-bsl-v1.33.0-build.1 assets as rlm-bsl-mcp / rlm-bsl-index, uses a fresh index-v15 generation, retains builder 14 for rollback, and records the issue #485 benchmark evidence. Prerequisite: #487."
```

- [ ] **Step 4: Require the three-target package jobs and inspect their logs**

```bash
gh pr checks --repo IngvarConsulting/unica --watch
```

For `linux-x64`, `win-x64`, and `darwin-arm64`, require:

- asset download and SHA verification;
- `rlm-bsl-mcp --help` and `rlm-bsl-index --help`;
- index lifecycle and real MCP helper probe;
- deterministic runtime packaging;
- extracted runtime Unica MCP smoke;
- Rust platform tests where classified;
- absence of `rlm-tools-bsl` from the runtime archive.

Green CI is necessary but not sufficient: inspect the package job logs for all three names/targets and compare the downloaded SHA values with the review record.

- [ ] **Step 5: Append delivery evidence to the already corrected issue #488**

Before implementation, issue #488 was already corrected to remove the obsolete
in-place migration criterion. Verify its acceptance section still requires:

- builder 15 receives `rlm-bsl/index-v15` and generation-scoped status/lock;
- existing builder-14 directory and markers remain unchanged;
- first use starts a cold build;
- `incomplete` recovery stays inside builder 15;
- `building`/`incomplete` block reads;
- rollback to the prior runtime still sees builder 14.

Append links to:

- toolchain PR;
- immutable toolchain release;
- #487 PR;
- consumer PR;
- #485 v1.33 measurement section;
- three-target checks.

Verify milestone remains `v0.12`:

```bash
gh issue view 488 --repo IngvarConsulting/unica --json milestone,state,url
```

- [ ] **Step 6: Merge only after semantic review and final refresh**

Immediately before merge:

```bash
git fetch origin main
git merge-base --is-ancestor origin/main HEAD
gh pr view --repo IngvarConsulting/unica --json reviewDecision,mergeStateStatus,statusCheckRollup
gh pr checks --repo IngvarConsulting/unica
git status --short --branch
```

Resolve review threads in the existing head branch. Merge only when the release evidence, benchmark evidence, package bytes, generation tests, and architecture record all agree.

---

## Final Acceptance Matrix

| Gate | Required evidence | Stops delivery when |
| --- | --- | --- |
| Toolchain source | tag resolves to `3e6920cd...`, frozen lock and MIT license validate | ref, commit, lock, or license differs |
| Toolchain release | 13 exact files, six native hashes, three provenance files, native Darwin smoke | release is partial, mutable, or mismatched |
| #487 prerequisite | builder and reader share one stable path outside `sourceRoot` | unsafe root remains or consumer PR is stacked |
| Benchmark | packaged/source runs use identical files and restored clean tree | paths differ, marker remains, or provenance differs |
| Consumer pin | both tools share version/source/release and verified hashes | mixed reader/indexer identity exists |
| Generation | DB/status/lock all use `index-v15`; v14 bytes unchanged | any migration, deletion, or legacy marker reuse occurs |
| Read barrier | `building` and `incomplete` return retryable non-success | stale/partial data can be published |
| Package | new pair present, old executable absent on all targets | archive contains alias/old name or helper smoke fails |
| Architecture | ADR-0059 accepted and invariant executable | docs still describe in-place migration or old runtime name |
| Issues | #485 contains sanitized measurements; #488 links all evidence in `v0.12` | issue criteria contradict shipped behavior |
