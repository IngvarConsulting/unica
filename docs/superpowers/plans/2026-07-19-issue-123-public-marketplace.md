# Issue 123 Public Marketplace Implementation Plan

> **Historical execution context.** Validate every command and requirement against current code, tests, and package metadata before reuse.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish Unica 0.7.0 through `IngvarConsulting/unica-marketplace` as one thin `unica@unica` plugin that bootstraps a checksum-pinned native runtime for the current platform and transactionally migrates legacy installations.

**Architecture:** `IngvarConsulting/unica` builds full per-target runtime archives and three small native `unica-bootstrap` launchers. The public marketplace stores only plugin metadata, skills, assets, launchers, and a deterministic runtime manifest; `.mcp.json` enters through a command-scoped Git shell alias, which selects the native launcher without Node. A two-phase marketplace staging/tag/promotion flow keeps the stable catalog on the previous immutable tag until every published asset and platform smoke test passes.

**Tech Stack:** Rust 2021 (`ureq`, `sha2`, `fs2`, `flate2`, `tar`, `serde`), Python 3.12 packaging/contract tests, POSIX shell, Windows PowerShell 5.1 shims, GitHub Actions, Codex plugin CLI JSON commands.

## Global Constraints

- Target release is exactly `v0.7.0`; workspace, plugin, tools lock, native `--version`, runtime manifest, and marketplace package versions must agree.
- Consumer marketplace, plugin, selector, and public MCP identities are `unica`, `unica`, `unica@unica`, and `unica`.
- Standard Git and a compatible Codex CLI are the only installation prerequisites; Node.js, Python, curl, wget, jq, and archive tools are not consumer prerequisites.
- Consumer metadata, docs, installers, release names, and cache paths must contain no `unica-local`.
- Prompt-visible skills remain MCP-first through native `unica.*` tools.
- Full runtime downloads are target-specific; thin marketplace packages contain no full tool runtime.
- Stable marketplace metadata uses `git-subdir` pinned to an immutable tag or SHA, never `source: local`, `latest`, or a semver range.
- Stdout from bootstrap and the launched process is reserved for MCP; bootstrap diagnostics use stderr.
- Runtime publication is lock-protected, checksum-verified, same-filesystem, and atomic; incomplete state never receives a ready marker.
- Normal install/update/migration paths use native `codex plugin` commands and do not manually populate Codex plugin cache or edit `config.toml`.
- Runtime release publication must not depend on optional assessment Pages publication.
- Tests are written and observed failing before production behavior is added.

---

## File and Component Map

### Source/build repository

- `crates/unica-bootstrap/src/manifest.rs`: typed manifest and origin/version validation.
- `crates/unica-bootstrap/src/target.rs`: supported host detection and entry selection.
- `crates/unica-bootstrap/src/archive.rs`: safe tar.gz extraction and file verification.
- `crates/unica-bootstrap/src/cache.rs`: cache paths, lock, ready marker, and atomic publication.
- `crates/unica-bootstrap/src/download.rs`: HTTPS download with redirect/origin policy.
- `crates/unica-bootstrap/src/process.rs`: MCP runtime replacement/supervision.
- `crates/unica-bootstrap/src/migration.rs`: Codex JSON discovery and transaction/rollback engine.
- `crates/unica-bootstrap/src/lib.rs`: stable interfaces shared by CLI and tests.
- `crates/unica-bootstrap/src/main.rs`: narrow CLI (`run`, `verify`, `migrate`).
- `plugins/unica/bootstrap/launch.sh`: Git-shell platform selector only.
- `plugins/unica/.mcp.json`: source-development cargo launcher; generated thin package rewrites it to the Git alias.
- `plugins/unica/runtime-manifest.json`: checked-in schema-valid development placeholder, replaced deterministically in packages.
- `scripts/ci/package-unica-runtime.py`: deterministic target runtime tarball and metadata builder.
- `scripts/ci/package-unica-plugin.py`: thin package and marketplace payload builder.
- `scripts/ci/check-version-contract.py`: one version equality guard.
- `scripts/install-unica.sh` and `scripts/install-unica.ps1`: transition shims that fetch the stable package and delegate to native migration.
- `tests/ci/test_package_unica_runtime.py`: runtime archive generation tests.
- `tests/ci/test_package_unica_plugin.py`: thin package, Git alias, and catalog tests.
- `tests/ci/test_install_unica_script.py`: shim contract tests.
- `tests/ci/test_version_contract.py`: version equality and mismatch diagnostics.
- `.github/workflows/unica-plugin-release.yml`: build/runtime/release verification.
- `.github/workflows/publish-unica-marketplace.yml`: cross-repository staging and promotion automation.
- `docs/releases/v0.7.0.md`, `README.md`, `plugins/unica/README.md`: consumer release/install/update/migration/uninstall docs.

### Marketplace repository

- `.agents/plugins/marketplace.json`: stable catalog pinned to the immutable marketplace tag.
- `plugins/unica/**`: generated thin package.
- `README.md`: install/update/uninstall contract.
- `MIGRATION.md`: legacy detection, backup, rollback, and new-task boundary.
- `.github/workflows/verify.yml`: package validation and three-platform bootstrap/fresh-install smoke.

---

### Task 1: Enforce the 0.7.0 Version Contract

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `plugins/unica/.codex-plugin/plugin.json`
- Modify: `plugins/unica/third-party/tools.lock.json`
- Create: `scripts/ci/check-version-contract.py`
- Create: `tests/ci/test_version_contract.py`

**Interfaces:**
- Consumes: JSON and TOML package metadata already tracked in the repository.
- Produces: `read_version_contract(repo_root: Path) -> dict[str, str]` and `validate_version_contract(values: dict[str, str], expected: str | None) -> list[str]`.

- [ ] **Step 1: Write failing equality and mismatch tests**

```python
class VersionContractTests(unittest.TestCase):
    def test_repository_versions_are_exactly_0_7_0(self):
        values = module.read_version_contract(REPO_ROOT)
        self.assertEqual(set(values.values()), {"0.7.0"})

    def test_mismatch_names_both_contract_fields(self):
        errors = module.validate_version_contract(
            {"workspace": "0.7.0", "plugin": "0.6.1"}, expected="0.7.0"
        )
        self.assertEqual(errors, ["plugin version 0.6.1 != expected 0.7.0"])
```

- [ ] **Step 2: Run RED**

Run: `python3.12 -m unittest tests.ci.test_version_contract -v`  
Expected: FAIL because `check-version-contract.py` and `0.7.0` metadata do not exist.

- [ ] **Step 3: Implement the validator and update exact metadata**

The script parses workspace TOML with `tomllib`, plugin and tools-lock JSON with
`json`, reads the Unica tool entry by name, prints one error per mismatch, and
exits 1 on errors. Set all four current version sources to `0.7.0` and regenerate
`Cargo.lock` with `cargo check --workspace`.

- [ ] **Step 4: Run GREEN and repository contract smoke**

Run: `python3.12 -m unittest tests.ci.test_version_contract -v && python3.12 scripts/ci/check-version-contract.py --expected 0.7.0`  
Expected: 2 tests pass; validator exits 0 with `Unica version contract: 0.7.0`.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock plugins/unica/.codex-plugin/plugin.json \
  plugins/unica/third-party/tools.lock.json scripts/ci/check-version-contract.py \
  tests/ci/test_version_contract.py
git commit -m "build: establish Unica 0.7.0 version contract"
```

### Task 2: Add Typed Runtime Manifest and Target Detection

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/unica-bootstrap/Cargo.toml`
- Create: `crates/unica-bootstrap/src/lib.rs`
- Create: `crates/unica-bootstrap/src/manifest.rs`
- Create: `crates/unica-bootstrap/src/target.rs`
- Create: `crates/unica-bootstrap/src/main.rs`
- Create: `plugins/unica/runtime-manifest.json`

**Interfaces:**
- Consumes: `runtime-manifest.json` schema version 1.
- Produces: `RuntimeManifest::load(path: &Path) -> Result<Self>`, `RuntimeManifest::validate(plugin_version: &str) -> Result<()>`, `HostTarget::detect(os: &str, arch: &str) -> Result<HostTarget>`, and `RuntimeManifest::target(HostTarget) -> Result<&TargetRuntime>`.

- [ ] **Step 1: Write failing Rust tests for schema, version, origin, and targets**

```rust
#[test]
fn manifest_rejects_plugin_version_mismatch_before_target_selection() {
    let manifest = fixture_manifest("0.7.0");
    let error = manifest.validate("0.7.1").unwrap_err();
    assert!(error.to_string().contains("plugin version 0.7.0 != 0.7.1"));
}

#[test]
fn target_detection_accepts_git_for_windows_uname() {
    assert_eq!(HostTarget::detect("MINGW64_NT-10.0", "x86_64").unwrap(), HostTarget::WinX64);
}

#[test]
fn manifest_rejects_non_release_https_origin() {
    let mut manifest = fixture_manifest("0.7.0");
    manifest.targets.get_mut("linux-x64").unwrap().asset.url =
        "https://example.invalid/unica.tar.gz".into();
    assert!(manifest.validate("0.7.0").unwrap_err().to_string().contains("release origin"));
}
```

- [ ] **Step 2: Run RED**

Run: `cargo test -p unica-bootstrap -- --nocapture`  
Expected: FAIL because the workspace member and types do not exist.

- [ ] **Step 3: Add the focused crate and strict data types**

Use `serde(deny_unknown_fields)` structs with explicit `schema_version`,
`plugin_version`, source repository/commit, release repository/tag, and a
`BTreeMap<String, TargetRuntime>`. Validate lowercase 64-character SHA-256,
HTTPS GitHub release URLs, relative file paths without `..` or backslashes, one
entrypoint per target, and exactly the three supported targets.

- [ ] **Step 4: Add a checked-in development placeholder**

The placeholder is schema-valid, version `0.7.0`, source/release values set to
`workspace`, and targets omitted only through an explicit `development: true`
mode rejected by packaged bootstrap. This lets source-tree schema checks pass
without pretending unpublished checksums are real.

- [ ] **Step 5: Run GREEN**

Run: `cargo test -p unica-bootstrap -- --nocapture`  
Expected: all manifest and target tests pass.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/unica-bootstrap plugins/unica/runtime-manifest.json
git commit -m "feat(bootstrap): validate pinned runtime manifests"
```

### Task 3: Implement Safe Download, Extraction, and Atomic Cache Publication

**Files:**
- Modify: `crates/unica-bootstrap/Cargo.toml`
- Create: `crates/unica-bootstrap/src/archive.rs`
- Create: `crates/unica-bootstrap/src/cache.rs`
- Create: `crates/unica-bootstrap/src/download.rs`
- Modify: `crates/unica-bootstrap/src/lib.rs`

**Interfaces:**
- Consumes: validated `TargetRuntime`.
- Produces: `RuntimeInstaller::ensure(&self, manifest: &RuntimeManifest, target: HostTarget) -> Result<RuntimeInstallation>`, `extract_verified_tar_gz`, and `RuntimeInstallation { root: PathBuf, entrypoint: PathBuf }`.

- [ ] **Step 1: Write failing archive-security tests**

Create in-memory tar.gz fixtures and assert rejection of `../escape`, absolute
paths, symlinks, hardlinks, devices, missing files, extra files, and mismatched
hashes. Assert a valid archive preserves only declared files.

- [ ] **Step 2: Write failing cache transaction tests**

```rust
#[test]
fn interrupted_extraction_never_publishes_ready_runtime() {
    let result = installer_with_corrupt_archive().ensure(&manifest(), HostTarget::LinuxX64);
    assert!(result.is_err());
    assert!(!final_runtime_dir().exists());
}

#[test]
fn concurrent_installers_publish_one_verified_directory() {
    let results = run_two_installers_against_one_cache();
    assert!(results.iter().all(Result::is_ok));
    assert_eq!(download_count(), 1);
    assert_ready_marker_matches_manifest();
}
```

- [ ] **Step 3: Run RED**

Run: `cargo test -p unica-bootstrap -- --nocapture`  
Expected: FAIL because extraction and cache modules do not exist.

- [ ] **Step 4: Implement downloader policy**

Use `ureq` with bounded connect/read timeouts. Require HTTPS initially and after
redirects, cap redirects, stream to a unique same-filesystem temporary file, and
hash while writing. Never buffer a runtime archive fully in memory.

- [ ] **Step 5: Implement safe extraction and file verification**

Use `flate2::read::GzDecoder` and `tar::Archive`, validate each entry before
unpacking, reject link/device types, use `unpack_in`, compare the exact extracted
file set with the manifest, and hash every file.

- [ ] **Step 6: Implement lock and atomic publication**

Use an `fs2` exclusive file lock keyed by version and target. After locking,
recheck the ready marker. Build under a UUID directory adjacent to the final
directory, write `.ready.json` last, and rename atomically. On an invalid
existing final directory, rename it to a transaction-owned quarantine path
before replacement and remove only paths owned by the current transaction.

- [ ] **Step 7: Run GREEN and repeat concurrency test**

Run: `cargo test -p unica-bootstrap -- --nocapture --test-threads=1` twice.  
Expected: all tests pass twice with one download in the concurrency fixture.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock crates/unica-bootstrap
git commit -m "feat(bootstrap): publish verified runtimes atomically"
```

### Task 4: Add Platform-neutral Launcher and Process Supervision

**Files:**
- Create: `plugins/unica/bootstrap/launch.sh`
- Create: `crates/unica-bootstrap/src/process.rs`
- Modify: `crates/unica-bootstrap/src/main.rs`
- Modify: `crates/unica-bootstrap/src/lib.rs`
- Modify: `tests/ci/test_package_unica_plugin.py`

**Interfaces:**
- Consumes: `RuntimeInstaller::ensure`.
- Produces: CLI commands `run --plugin-root PATH`, `verify --plugin-root PATH`, and the generated Git alias MCP server configuration.

- [ ] **Step 1: Write failing launcher contract tests**

Assert the generated MCP command is `git`, `cwd` is `.`, the alias starts with
`!`, no global config write occurs, stdout remains empty in selector tests, and
the launcher maps Darwin/Linux/MINGW/MSYS to the exact bootstrap path while
rejecting unsupported combinations with exit code 78.

- [ ] **Step 2: Run RED**

Run: `python3.12 -m unittest tests.ci.test_package_unica_plugin.PackageUnicaPluginTests.test_packaged_mcp_uses_git_shell_alias -v`  
Expected: FAIL because packages still launch a bundled full runtime directly.

- [ ] **Step 3: Implement the selector**

`launch.sh` reads optional test-only `UNICA_BOOTSTRAP_UNAME_S` and
`UNICA_BOOTSTRAP_UNAME_M`, otherwise calls `uname`. It derives its plugin root
from the first argument passed by the Git alias and `exec`s only a path inside
`bootstrap/bin/<target>/`.

- [ ] **Step 4: Implement runtime launch semantics**

On Unix, use `std::os::unix::process::CommandExt::exec`. On Windows, inherit
stdio, assign the child to a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`,
wait, and return the child's exact exit code. Emit all pre-exec errors to stderr.

- [ ] **Step 5: Run GREEN**

Run: `cargo test -p unica-bootstrap process -- --nocapture && python3.12 -m unittest tests.ci.test_package_unica_plugin -v`  
Expected: Rust process tests and updated package tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/unica-bootstrap plugins/unica/bootstrap/launch.sh \
  tests/ci/test_package_unica_plugin.py
git commit -m "feat(bootstrap): launch through portable Git entrypoint"
```

### Task 5: Build Deterministic Runtime Assets and Thin Plugin Payloads

**Files:**
- Create: `scripts/ci/package-unica-runtime.py`
- Create: `tests/ci/test_package_unica_runtime.py`
- Modify: `scripts/ci/build-unica-tools.py`
- Rewrite focused sections: `scripts/ci/package-unica-plugin.py`
- Modify: `tests/ci/test_build_unica_tools.py`
- Modify: `tests/ci/test_package_unica_plugin.py`

**Interfaces:**
- Consumes: target tool bundle, target bootstrap binary, verified release tag and source commit.
- Produces: `unica-runtime-<target>.tar.gz`, `unica-runtime-<target>.json`, and one `marketplace/` thin payload with `runtime-manifest.json`.

- [ ] **Step 1: Write failing deterministic archive tests**

Assert two builds from identical inputs have identical SHA-256, tar members are
sorted with normalized uid/gid/mtime/modes, the archive contains only one target,
and target metadata hashes match every file and archive.

- [ ] **Step 2: Write failing thin-package tests**

Assert the generated plugin contains all three bootstrap binaries, no
`bin/<target>` full runtime, no target marketplace archives, one Git alias MCP
server, a manifest with three release assets, and no `unica-local` or
`source: local` string.

- [ ] **Step 3: Run RED**

Run: `python3.12 -m unittest tests.ci.test_package_unica_runtime tests.ci.test_package_unica_plugin -v`  
Expected: failures show current target-specific fat marketplace behavior.

- [ ] **Step 4: Add native bootstrap output to tool builds**

`build-unica-tools.py` builds `unica-bootstrap` for the current native target
and copies it under `bootstrap/bin/<target>/`; it remains outside `tools.json`
because it is package infrastructure rather than an internal Unica tool.

- [ ] **Step 5: Implement deterministic runtime packaging**

The new script loads and validates `tools.json`, stages `bin/<target>`, creates
a normalized tar.gz stream, and writes target metadata with archive and file
checksums. It rejects unknown targets, duplicate files, symlinks, and lock
mismatches.

- [ ] **Step 6: Convert plugin packager to thin output**

Replace fat-binary copying and target-specific archive naming with:

```text
--runtime-metadata-root PATH
--bootstrap-root PATH
--release-tag v0.7.0
--source-commit <40 hex>
--out-dir PATH
```

Copy tracked plugin sources, omit the development manifest, copy exactly three
bootstrap binaries, generate the release manifest, rewrite `.mcp.json` to the
Git alias, and emit a directory suitable for the marketplace staging PR.

- [ ] **Step 7: Run GREEN**

Run: `python3.12 -m unittest tests.ci.test_build_unica_tools tests.ci.test_package_unica_runtime tests.ci.test_package_unica_plugin -v`  
Expected: all packaging/build tests pass.

- [ ] **Step 8: Commit**

```bash
git add scripts/ci/build-unica-tools.py scripts/ci/package-unica-runtime.py \
  scripts/ci/package-unica-plugin.py tests/ci/test_build_unica_tools.py \
  tests/ci/test_package_unica_runtime.py tests/ci/test_package_unica_plugin.py
git commit -m "build: generate thin plugin and pinned runtime assets"
```

### Task 6: Implement Shared Transactional Migration Engine

**Files:**
- Create: `crates/unica-bootstrap/src/codex.rs`
- Create: `crates/unica-bootstrap/src/migration.rs`
- Modify: `crates/unica-bootstrap/src/lib.rs`
- Modify: `crates/unica-bootstrap/src/main.rs`

**Interfaces:**
- Consumes: `codex plugin marketplace list --json`, `codex plugin list --available --json`, a filesystem abstraction, and command runner.
- Produces: `MigrationEngine::preflight() -> Result<MigrationPlan>`, `MigrationEngine::apply(plan) -> Result<MigrationReport>`, and automatic `rollback(journal) -> Result<()>`.

- [ ] **Step 1: Capture Codex 0.145.0-alpha.18 JSON schemas in test fixtures**

Use an isolated temporary `CODEX_HOME`; store only synthetic/empty schema-shaped
fixtures under `crates/unica-bootstrap/tests/fixtures/`. Do not commit local user
paths or tokens.

- [ ] **Step 2: Write failing legacy-layout classification tests**

Cover local source named `unica`, source/plugin named `unica-local`, duplicate
selectors, orphaned exact paths, already canonical source, and an unknown source
owning marketplace name `unica`. The unknown-owner case must fail before backup
or mutation.

- [ ] **Step 3: Write failure-injection rollback tests**

For each mutating journal step, make the next command fail and assert the exact
prior marketplace/plugin/config/path snapshot is restored, the backup remains,
the diagnostic log is redacted, and the process exits nonzero.

- [ ] **Step 4: Run RED**

Run: `cargo test -p unica-bootstrap migration -- --nocapture`  
Expected: FAIL because no migration engine exists.

- [ ] **Step 5: Implement discovery and preflight**

Parse JSON with typed, forward-compatible structures that preserve unknown
fields but require identifiers/source types used for decisions. Probe the Git
shell alias. Resolve only paths reported by Codex or matching documented legacy
roots under `CODEX_HOME`; reject path escape and symlink escape before backup.

- [ ] **Step 6: Implement journaled apply and rollback**

Every successful mutation appends a journal entry before the next step. Use
native Codex remove/add/upgrade commands. Preserve old directories until MCP and
prompt proofs pass. Rollback executes inverse actions in reverse order and
restores the exact backed-up config atomically.

- [ ] **Step 7: Add runtime and MCP verification**

`verify` ensures the runtime, starts it with piped stdio, sends JSON-RPC
`initialize` and `tools/list`, and requires `unica.project.status`,
`unica.standards.search`, and `unica.standards.explain`. Bound startup and each
response; kill the process tree on timeout.

- [ ] **Step 8: Run GREEN**

Run: `cargo test -p unica-bootstrap migration -- --nocapture --test-threads=1`  
Expected: classification, idempotence, and all failure-injection cases pass.

- [ ] **Step 9: Commit**

```bash
git add crates/unica-bootstrap
git commit -m "feat(migration): migrate legacy plugins transactionally"
```

### Task 7: Replace Installers with POSIX and PowerShell Migration Shims

**Files:**
- Rewrite: `scripts/install-unica.sh`
- Rewrite: `scripts/install-unica.ps1`
- Rewrite: `tests/ci/test_install_unica_script.py`

**Interfaces:**
- Consumes: standard Git, Codex CLI, stable marketplace catalog.
- Produces: thin shims that fetch the stable immutable package and invoke `unica-bootstrap migrate`.

- [ ] **Step 1: Write failing shim tests**

Use fake `git` and `codex` executables. Assert both shims run preflight before
mutation, use `IngvarConsulting/unica-marketplace`, pass `--ref main`, never
download old fat marketplace assets, never edit config/cache, propagate native
bootstrap exit codes, and retain `--help` plus diagnostic backup reporting.

- [ ] **Step 2: Run RED**

Run: `python3.12 -m unittest tests.ci.test_install_unica_script -v`  
Expected: current local-marketplace installer assertions fail.

- [ ] **Step 3: Implement POSIX shim**

The shell script checks `git`, `codex`, and one-shot Git shell alias; clones the
marketplace `main` into a temporary directory; reads the one pinned `ref` from
the controlled catalog; fetches/checks out that exact ref; then invokes the
target bootstrap with `migrate`. Trap cleanup affects only its temporary clone.

- [ ] **Step 4: Implement PowerShell 5.1 shim**

Use `ConvertFrom-Json` for catalog parsing, Git for clone/checkout, and the
Windows native bootstrap path. Do not require `pwsh`, Bash in PATH, Node, or
manual JSON/config editing.

- [ ] **Step 5: Run GREEN**

Run: `python3.12 -m unittest tests.ci.test_install_unica_script -v`  
Expected: POSIX textual/behavioral tests pass; Windows execution test remains
platform-gated for Actions.

- [ ] **Step 6: Commit**

```bash
git add scripts/install-unica.sh scripts/install-unica.ps1 tests/ci/test_install_unica_script.py
git commit -m "feat(installer): delegate legacy migration to Git package"
```

### Task 8: Rebuild Release and Marketplace Publication Workflows

**Files:**
- Modify: `.github/workflows/unica-plugin-release.yml`
- Create: `.github/workflows/publish-unica-marketplace.yml`
- Modify: `tests/ci/test_unica_workflow.py`
- Modify: `scripts/ci/classify-workflow-changes.py`
- Modify: `tests/ci/test_classify_workflow_changes.py`

**Interfaces:**
- Consumes: three tool/bootstrap bundles, three runtime artifacts, source tag, and `UNICA_MARKETPLACE_TOKEN` or documented GitHub App credential.
- Produces: published runtime assets, verified thin payload artifact, staging PR, immutable marketplace tag request, and promotion PR.

- [ ] **Step 1: Write failing workflow guardrails**

Assert source verification covers both Rust packages, runtime assets publish
without depending on Pages, published bytes are re-downloaded before marketplace
dispatch, staging and promotion are separate jobs/events, PR workflows have
read-only permissions, and cross-repository writes require an explicit secret.

- [ ] **Step 2: Run RED**

Run: `python3.12 -m unittest tests.ci.test_unica_workflow tests.ci.test_classify_workflow_changes -v`  
Expected: current fat-package and Pages-gated assertions fail.

- [ ] **Step 3: Refactor build and release jobs**

Build native tool/bootstrap bundles on each target, create deterministic runtime
archives, publish `unica-runtime-{target}.tar.gz` and checksum metadata, publish
transition shims, and re-download assets into a verification job. Run assessment
independently with `if: always()` publication behavior and no dependency edge
from runtime publication.

- [ ] **Step 4: Add cross-repository workflow**

On a verified `v*` release, generate/upload the thin payload and use an explicit
credential to open a staging PR in `unica-marketplace`. A manual promotion input
requires the staging merge SHA and existing signed tag, then opens a catalog-only
PR. The workflow never moves an existing tag.

- [ ] **Step 5: Run GREEN and YAML parse**

Run: `python3.12 -m unittest tests.ci.test_unica_workflow tests.ci.test_classify_workflow_changes -v && python3.12 -c 'import yaml; [yaml.safe_load(open(p)) for p in [".github/workflows/unica-plugin-release.yml", ".github/workflows/publish-unica-marketplace.yml"]]'`  
Expected: tests pass and both workflows parse.

- [ ] **Step 6: Commit**

```bash
git add .github/workflows scripts/ci/classify-workflow-changes.py \
  tests/ci/test_unica_workflow.py tests/ci/test_classify_workflow_changes.py
git commit -m "ci: publish verified runtimes before marketplace promotion"
```

### Task 9: Update Consumer Contracts and Documentation

**Files:**
- Modify: `README.md`
- Modify: `plugins/unica/README.md`
- Modify: `plugins/unica/references/tooling/internal-package.md`
- Modify: `spec/acceptance/unica-mcp-validation.md`
- Modify: `spec/architecture/arc42/06-runtime-view.md`
- Modify: `spec/architecture/arc42/07-deployment-view.md`
- Create: `spec/decisions/0008-public-marketplace-thin-runtime.md`
- Modify: `spec/decisions/README.md`
- Create: `docs/releases/v0.7.0.md`
- Modify: `tests/ci/test_product_contracts.py`

**Interfaces:**
- Consumes: implemented install/update/migration/bootstrap behavior.
- Produces: one current user-facing contract and an ADR recording the Git prerequisite and two-phase release.

- [ ] **Step 1: Write failing product-contract tests**

Assert docs contain the two Git marketplace commands, explicit update, rollback,
uninstall, Git prerequisite, new-task boundary, runtime checksum/cache behavior,
and no active consumer `unica-local` or old fat marketplace asset names.

- [ ] **Step 2: Run RED**

Run: `python3.12 -m unittest tests.ci.test_product_contracts -v`  
Expected: current installer/local-marketplace documentation fails.

- [ ] **Step 3: Rewrite active docs and architecture**

Describe only actual commands and implemented failure behavior. Keep
`scripts/dev/install-local-unica.sh` development-only and rename its default
marketplace to `unica-dev`. Record why Git alias/native bootstrap was chosen and
why stable promotion is a second commit.

- [ ] **Step 4: Add release notes**

Document breaking delivery change, prerequisites, migration/rollback, artifact
names, cache location, and proof requirements. Do not claim publication before
the published-system verification task succeeds.

- [ ] **Step 5: Run GREEN and forbidden-term scan**

Run: `python3.12 -m unittest tests.ci.test_product_contracts -v && git grep -n 'unica-local' -- README.md plugins scripts/install-unica.sh scripts/install-unica.ps1 spec docs/releases/v0.7.0.md`  
Expected: tests pass; grep reports only explicitly historical migration detection
text approved by the test allowlist, not a consumer selector/path/release name.

- [ ] **Step 6: Commit**

```bash
git add README.md plugins/unica scripts/dev/install-local-unica.sh spec \
  docs/releases/v0.7.0.md tests/ci/test_product_contracts.py
git commit -m "docs: document public marketplace delivery"
```

### Task 10: Create and Validate `IngvarConsulting/unica-marketplace`

**Files in external repository:**
- Create: `.agents/plugins/marketplace.json`
- Create: `plugins/unica/**` from verified thin payload
- Create: `README.md`
- Create: `MIGRATION.md`
- Create: `.github/workflows/verify.yml`
- Create: `LICENSE`

**Interfaces:**
- Consumes: verified thin payload from Task 5 and published runtime URLs from Task 8.
- Produces: public marketplace repository with staging branch, immutable `v0.7.0` tag, and promotion-ready stable catalog.

- [ ] **Step 1: Create the public repository without publishing stable**

From `/Users/ingvarvilkman/Documents/git`, run:
`gh repo create IngvarConsulting/unica-marketplace --public --description "Public Codex marketplace for Unica" --clone`.  
Expected: repository exists and local clone remote is the canonical GitHub URL.

- [ ] **Step 2: Generate staging content**

Use `package-unica-plugin.py` output; validate `.codex-plugin/plugin.json` with
`python3.12 /Users/ingvarvilkman/.codex/skills/.system/plugin-creator/scripts/validate_plugin.py plugins/unica`;
create README/MIGRATION from the source docs; keep
the stable catalog absent or pointing to the previous stable until the package
tag exists.

- [ ] **Step 3: Add failing CI expectations before workflow implementation**

Create a repository-local verifier test/script that rejects local sources,
mutable refs, missing bootstrap binaries, full runtime files, manifest/plugin
version mismatch, and `unica-local` consumer strings. Run it and observe failure
against an intentionally mutated temporary fixture.

- [ ] **Step 4: Implement three-platform workflow**

Validate JSON and package contract on Ubuntu; on `ubuntu-latest`, `macos-14`, and
`windows-latest`, install Codex CLI `0.145.0-alpha.18`, create isolated `CODEX_HOME`, exercise
the exact Git-alias MCP command without Node in PATH, bootstrap the published
runtime, and run MCP `initialize`/`tools/list`. Add an upgrade job from source
release `v0.6.1` and the corresponding synthetic legacy marketplace fixture.

- [ ] **Step 5: Push staging branch and open PR**

Run the external repository verifier locally, commit, push a `codex/stage-v0.7.0`
branch, open a PR, and wait for all checks. Do not update `main` catalog to the
new tag yet.

- [ ] **Step 6: Merge staging and create immutable signed tag**

After checks pass, merge the staging PR, pull `main`, create signed annotated tag
`v0.7.0`, push it, and verify the tag target and signature through GitHub/local
Git.

- [ ] **Step 7: Open promotion PR**

Change only `.agents/plugins/marketplace.json` to point `git-subdir` at `v0.7.0`,
run CI, and merge. Verify `main` resolves the plugin from the immutable tag.

### Task 11: Full Verification, Source Integration, and Published Proof

**Files:**
- Modify only defects found by verification, with a new RED test for each defect.

**Interfaces:**
- Consumes: source branch, GitHub source release, public marketplace tag/main.
- Produces: acceptance evidence for issue 123.

- [ ] **Step 1: Run the complete local source gate**

```bash
python3.12 -m unittest discover -s tests/ci
python3.12 -m py_compile scripts/ci/*.py tests/ci/*.py
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --test-threads=1
git diff --check
```

Expected: every command exits 0; test output reports zero failures.

- [ ] **Step 2: Push source branch and open PR**

Push `codex/issue-123-public-marketplace`, open a source PR referencing issue 123,
wait for all checks, review the patch and check output, then merge only if every
required check passes.

- [ ] **Step 3: Create and publish source tag**

From updated `main`, verify the version contract, create signed `v0.7.0`, push,
and wait for the release workflow. Do not reuse or move an existing tag.

- [ ] **Step 4: Verify published runtime assets**

Use `gh release view v0.7.0`, download every runtime archive and checksum
metadata into a fresh temporary directory, compare hashes with the thin manifest,
inspect members, and execute each available native smoke in its matching CI OS.

- [ ] **Step 5: Verify public fresh install and update**

In clean isolated homes on all three targets, run exactly:

```text
codex plugin marketplace add IngvarConsulting/unica-marketplace --ref main
codex plugin add unica@unica
```

Then start through the installed `.mcp.json`, run `initialize` and `tools/list`,
and verify the three required tool names. Repeat from source release `v0.6.1`
using explicit upgrade/remove/add commands and assert only `unica@unica` is
active.

- [ ] **Step 6: Verify migration and rollback evidence**

Run POSIX and Windows migration fixtures for every supported legacy layout,
repeat successful migration for idempotence, inject each mutation failure, and
retain redacted logs proving restoration.

- [ ] **Step 7: Close the acceptance loop**

Post exact source commit, signed tags, workflow run URLs, release asset hashes,
marketplace tag/main commit, and platform smoke evidence to issue 123. Close the
issue only after the published new-task proof passes.

---

## Plan Self-review Result

- Spec coverage: every design section maps to Tasks 1-11; external repository,
  release assets, migration rollback, docs, and published proof are included.
- Placeholder scan: every implementation step names concrete behavior, files,
  commands, and expected results.
- Type consistency: manifest, installer, migration, packager, and workflow tasks
  use the same `0.7.0`, target names, runtime asset names, and bootstrap command
  interfaces.
- Scope: the issue spans multiple subsystems, so the plan uses independently
  testable tasks and commits while preserving one atomic consumer-publishing
  outcome.
