# RLM Nuitka Standalone Consumer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pin the published Nuitka standalone RLM archives, safely materialize their complete payload in the Unica bundle, and preserve every dependency in the verified final runtime.

**Architecture:** Both RLM lock records refer to one immutable target archive but select different entrypoints. `build-unica-tools.py` downloads and validates it once, writes a complete root-level `runtimeFiles` closure, and `package-unica-runtime.py` packages exactly that closure; Rust bootstrap production code remains unchanged because final runtime metadata already hashes every file.

**Tech Stack:** Python 3.12, safe `tarfile` streaming, deterministic `tar.gz`, JSON lock/manifests, Rust bootstrap integration tests, GitHub Actions package matrix.

## Global Constraints

- Start only after the toolchain plan publishes and independently verifies exactly three RLM archives, three checksums, three provenance documents, and the MIT license.
- Pin only GitHub-published SHA-256/size values cross-checked against checksum, provenance, asset digest, and attestation; never use local rebuild digests.
- Both RLM records share archive repository/tag/name/SHA/size per target; only `binaryName` and `archiveBinary` differ.
- `tools.lock.json` stays schema `1`; `archive-release-asset` is an additive build-time strategy, not a runtime host contract.
- Download once per `(repository, tag, target, assetName, sha256)` identity.
- Never use `extractall()`. Accept only ordinary files under `payload/` after exact manifest/file-set validation.
- Generated `tools.json` uses schema `2`, distinct `tools[]`, and root `runtimeFiles[]` containing path, SHA-256, size, and executable flag for every target file.
- Package all and only `runtimeFiles`; reject missing, extra, linked, duplicate, or metadata-drifted files before archive publication.
- Command paths remain `bin/<target>/rlm-bsl-index[.exe]` and `bin/<target>/rlm-bsl-mcp[.exe]`.
- Public MCP, `unica.*`, RLM generation/cache/status/lock, source revision, and workspace semantics do not change.
- No hidden build.2 fallback after switching; rollback is a new commit restoring the old immutable lock.
- Existing bootstrap manifest schema and bootstrap production code stay unchanged unless a failing regression proves otherwise.
- PR evidence includes archive/extracted/final sizes, download time, file count, extracted packaged smoke, and three exact target job links.

---

## File Structure

- Create `scripts/ci/unica_runtime_archive.py`: read-only safe validator for the toolchain archive.
- Modify `scripts/ci/build-unica-tools.py`: grouped download, staged payload, `runtimeFiles`.
- Modify `scripts/ci/package-unica-runtime.py`: exact closure packaging.
- Modify `plugins/unica/third-party/tools.lock.json`: immutable archive pin and two selectors.
- Create `docs/provenance/reviews/2026-08-14-rlm-v1-33-nuitka-standalone.json`: published evidence.
- Modify attribution/provenance/product/package tests and bootstrap integration fixture.
- Ship the approved design, ADR-0061, decision index, and `INV-PKG-TOOL-CLOSURE`.
- Modify release workflow only for size/evidence output; do not alter runtime semantics.

---

### Task 1: Validate the Toolchain Archive Fail-Closed

**Files:**
- Create: `scripts/ci/unica_runtime_archive.py`
- Modify: `tests/ci/test_build_unica_tools.py`

**Interfaces:**
- Produces: `RuntimeArchiveFile(path: PurePosixPath, sha256: str, size: int, executable: bool, payload: bytes)`.
- Produces: `load_verified_archive(path, *, release_tag, source_commit, target, entrypoints) -> tuple[RuntimeArchiveFile, ...]`.

- [ ] **Step 1: Write unsafe-archive RED tests**

Test a valid fixture, then separately reject absolute/`..`/backslash/duplicate paths, symlink, hardlink, device/FIFO, missing/extra member, digest/size/mode drift, wrong release/source/target, missing/non-executable entrypoint, and unequal multidist executables.

- [ ] **Step 2: Run RED**

```bash
python3.12 -m unittest tests.ci.test_build_unica_tools.BuildUnicaToolsTests.test_verified_archive_rejects_unsafe_and_drifted_members -v
```

Expected: absent helper import.

- [ ] **Step 3: Implement read-only validation**

Inspect every `TarInfo` before reading content; never extract. Parse exactly one schema-1 `manifest.json`; compare actual `payload/` members with declared files in both directions and return immutable records only after all checks. Error text uses archive-relative names, not absolute host paths.

- [ ] **Step 4: Run GREEN and commit**

```bash
python3.12 -m unittest tests.ci.test_build_unica_tools -v
python3.12 -m py_compile scripts/ci/unica_runtime_archive.py
git add scripts/ci/unica_runtime_archive.py tests/ci/test_build_unica_tools.py
git commit -m "feat(packaging): verify standalone tool archives"
```

---

### Task 2: Download Once and Generate the Runtime Closure

**Files:**
- Modify: `scripts/ci/build-unica-tools.py`
- Modify: `tests/ci/test_build_unica_tools.py`

**Interfaces:**
- Consumes: Task 1 records and synthetic archive lock entries.
- Produces: `tools.json` schema `2` with root `runtimeFiles[]` and unchanged `tools[].binaryPath`.

- [ ] **Step 1: Write grouped-download RED tests**

Use two RLM records sharing one fake archive; patch `download()` and require one call. Require both commands and one library in the staged tree and:

```json
{
  "schemaVersion": 2,
  "runtimeFiles": [
    {"path": "bin/linux-x64/libpython3.12.so.1.0", "sha256": "...", "size": 123, "executable": false},
    {"path": "bin/linux-x64/rlm-bsl-index", "sha256": "...", "size": 456, "executable": true},
    {"path": "bin/linux-x64/rlm-bsl-mcp", "sha256": "...", "size": 456, "executable": true}
  ]
}
```

Reject conflicting archive identities, duplicate destination, collision with another tool, missing selector, and a second download attempt.
Replace the old source-scan test that forbids `archive-release-asset` with behavioral tests for the supported strategy; retaining that sentinel would contradict the accepted ADR.

- [ ] **Step 2: Run RED**

```bash
python3.12 -m unittest tests.ci.test_build_unica_tools -v
```

Expected: unsupported strategy and missing `runtimeFiles`.

- [ ] **Step 3: Implement grouping and staged materialization**

Group by:

```python
ArchiveIdentity = tuple[str, str, str, str, str]
# repository, tag, target, assetName, sha256
```

Download/hash/validate once. Write all verified payload bytes under a fresh work-dir staging tree, verify collisions, then materialize the complete target payload. Always generate sorted `runtimeFiles` for Cargo, direct, and archive tools, including every binary.

- [ ] **Step 4: Run GREEN and commit**

```bash
python3.12 -m unittest tests.ci.test_build_unica_tools tests.ci.test_smoke_unica_bootstrap -v
python3.12 -m py_compile scripts/ci/build-unica-tools.py
git add scripts/ci/build-unica-tools.py tests/ci/test_build_unica_tools.py
git commit -m "feat(packaging): materialize shared RLM payload"
```

---

### Task 3: Package All and Only Declared Files

**Files:**
- Modify: `scripts/ci/package-unica-runtime.py`
- Modify: `tests/ci/test_package_unica_runtime.py`
- Modify: `crates/unica-bootstrap/tests/runtime_install.rs`

**Interfaces:**
- Consumes: `tools.json` schema `2` and `runtimeFiles[]`.
- Produces: existing final runtime archive/metadata schema with the full file closure.

- [ ] **Step 1: Write closure RED tests**

Add a shared library to the bundle and require it once in deterministic output. Reject missing, extra, linked, digest/size/mode-drifted, duplicate, out-of-target files and `binaryPath` outside closure. Change the legacy-stray sentinel: undeclared `bin/<target>/rlm-tools-bsl` must fail, not be silently omitted.

Add a Rust install fixture with `unica` and one non-executable library; require exact bytes and ready marker only after both verify.

- [ ] **Step 2: Run RED**

```bash
python3.12 -m unittest tests.ci.test_package_unica_runtime -v
cargo test -p unica-bootstrap --test runtime_install -- --nocapture
```

Expected: Python derives only tool binaries and fails. Rust should already pass, proving production bootstrap needs no change.

- [ ] **Step 3: Implement exact closure consumption**

Require `tools.json.schemaVersion == 2`. Validate all declarations before opening output. Compare the exact regular-file set below `bundle_root/bin/<target>` with `runtimeFiles`; keep separate `bootstrap/` outside that comparison. Preserve generated `third-party/manifest.json` and final metadata schemas.

- [ ] **Step 4: Run GREEN and commit**

```bash
python3.12 -m unittest tests.ci.test_package_unica_runtime -v
cargo test -p unica-bootstrap --test runtime_install -- --nocapture
python3.12 -m py_compile scripts/ci/package-unica-runtime.py
git add scripts/ci/package-unica-runtime.py tests/ci/test_package_unica_runtime.py crates/unica-bootstrap/tests/runtime_install.rs
git commit -m "feat(packaging): preserve runtime file closure"
```

---

### Task 4: Pin Published Bytes and Bind Provenance

**Files:**
- Modify: `plugins/unica/third-party/tools.lock.json`
- Create: `docs/provenance/reviews/2026-08-14-rlm-v1-33-nuitka-standalone.json`
- Modify: `plugins/unica/ATTRIBUTIONS.md`
- Modify: `tests/ci/test_build_unica_tools.py`
- Modify: `tests/ci/test_skill_provenance.py`
- Modify: `tests/ci/test_attributions.py`
- Modify: `tests/ci/test_product_contracts.py`

**Interfaces:**
- Produces: tracked source/release/archive/payload/builder evidence and real entrypoint contract proof.

- [ ] **Step 1: Download and verify the immutable release**

```bash
: "${RLM_RELEASE_TAG:=rlm-tools-bsl-v1.33.0-build.3}"
audit_dir="$(mktemp -d -t unica-rlm-standalone-pin.XXXXXX)"
gh release download "$RLM_RELEASE_TAG" --repo IngvarConsulting/unica-toolchain --dir "$audit_dir"
find "$audit_dir" -maxdepth 1 -type f -print | sort
```

Require ten files and cross-check outer hashes, checksums, provenance, attestations, internal source/target/builder identities, entrypoints, and payload hashes before editing tracked values.

- [ ] **Step 2: Write lock and provenance RED tests**

Keep bsl-analyzer/v8-runner on `direct-release-asset`. For both RLM tools require `archive-release-asset`, tag build.3, the same target archive objects, and distinct `archiveBinary` values. Every target archive has a literal 64-hex SHA and positive size copied from the audit.

Require source ref/commit/tree/patches, builder Python/uv/Nuitka/compiler per target, release tag, three archive hashes/sizes, extracted payload identities, entrypoint mapping, and compatibility verdict. Mutation-test every identity so build.2 or a local rebuild digest fails. Require attribution to name Nuitka as build tooling without calling internal MCP a public Unica API.

- [ ] **Step 3: Run RED**

```bash
python3.12 -m unittest tests.ci.test_build_unica_tools tests.ci.test_skill_provenance tests.ci.test_attributions tests.ci.test_product_contracts -v
```

- [ ] **Step 4: Pin published bytes, add evidence, and run real contracts**

Copy literal archive SHA-256/sizes from the audit into both RLM lock records while preserving upstream source/version/license and distinct binary names. Write only audited published values to the review. Run the existing contract checker against a complete materialized bundle; do not teach runtime contract code to read release archives.

- [ ] **Step 5: Run GREEN and commit**

```bash
python3.12 -m unittest tests.ci.test_build_unica_tools tests.ci.test_skill_provenance tests.ci.test_attributions tests.ci.test_product_contracts -v
python3.12 scripts/ci/check-attributions.py
python3.12 scripts/ci/check-tool-contracts.py --target "$TARGET" --tools-dir "$TOOLS_DIR"
git add plugins/unica/third-party/tools.lock.json docs/provenance/reviews/2026-08-14-rlm-v1-33-nuitka-standalone.json plugins/unica/ATTRIBUTIONS.md tests/ci/test_build_unica_tools.py tests/ci/test_skill_provenance.py tests/ci/test_attributions.py tests/ci/test_product_contracts.py
git commit -m "build(rlm): pin standalone release evidence"
```

---

### Task 5: Ship ADR, Invariant, and Size Evidence

**Files:**
- Read: `docs/design/2026-08-14-rlm-nuitka-standalone-multidist-design.md`
- Read: `spec/decisions/0061-rlm-mnogofaylovyy-runtime-iz-proveryaemogo-arhiva.md`
- Read: `spec/architecture/invariants.md`
- Modify: `.github/workflows/unica-plugin-release.yml` only for evidence output.
- Modify: `tests/ci/test_design_documents.py`
- Modify: `tests/ci/test_architecture_registry.py`
- Modify: `tests/ci/test_classify_workflow_changes.py`

**Interfaces:**
- Produces: accepted ADR-0061 and `INV-PKG-TOOL-CLOSURE`; public MCP stays unchanged.

- [ ] **Step 1: Bind documents and write workflow-evidence RED tests**

Add regression assertions that ADR-0061 owns archive/multidist, the invariant owns full closure and fail-closed unsafe members, and active prose does not claim RLM remains a direct loose asset. These may already pass because the approved documents are the plan input. Separately assert each target release job emits target archive size, extracted RLM payload size, final runtime size, and file count; that assertion must fail before workflow changes.

- [ ] **Step 2: Run RED**

```bash
python3.12 -m unittest tests.ci.test_design_documents tests.ci.test_architecture_registry -v
python3.12 scripts/ci/check-architecture-sync.py --base origin/main
```

Expected: document assertions pass, the workflow evidence assertion fails for missing metrics, and architecture sync still says public MCP unchanged.

- [ ] **Step 3: Reconcile document numbering and add metrics**

Confirm actual field names/failures still match the approved documents. If `origin/main` gained another ADR, renumber this unmerged ADR and all references in a dedicated docs commit before continuing. Have each target job report target archive size, extracted RLM payload size, final runtime size, and file count without changing package bytes.

- [ ] **Step 4: Run GREEN and commit**

```bash
python3.12 -m unittest tests.ci.test_design_documents tests.ci.test_architecture_registry tests.ci.test_classify_workflow_changes -v
python3.12 scripts/ci/check-architecture-sync.py --base origin/main
git diff --check
git add .github/workflows/unica-plugin-release.yml tests/ci/test_design_documents.py tests/ci/test_architecture_registry.py tests/ci/test_classify_workflow_changes.py
git commit -m "ci(rlm): report standalone runtime size"
```

---

### Task 6: Full Verification and Ready PR

**Files:**
- No extra source changes unless RED proves an introduced defect.

**Interfaces:**
- Produces: one ready Unica PR consuming the immutable toolchain release.

- [ ] **Step 1: Focused gates**

```bash
python3.12 -m unittest tests.ci.test_build_unica_tools tests.ci.test_package_unica_runtime tests.ci.test_product_contracts tests.ci.test_skill_provenance tests.ci.test_attributions tests.ci.test_design_documents tests.ci.test_architecture_registry -v
cargo test -p unica-bootstrap --test runtime_install -- --nocapture
python3.12 scripts/ci/check-attributions.py
python3.12 scripts/ci/check-architecture-sync.py --base origin/main
python3.12 scripts/ci/check-rust-platform-boundary.py
```

- [ ] **Step 2: Full local gates**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --test-threads=1
python3.12 -m unittest discover -s tests/ci
python3.12 -m unittest discover -s tests/dev
python3.12 -m py_compile scripts/ci/*.py scripts/dev/*.py tests/ci/*.py tests/dev/*.py
git diff --check
```

Run any issue-89 ignored tests explicitly if still ignored.

- [ ] **Step 3: Real Darwin package proof**

In a fresh ignored root: build from exact lock, run real tool contracts, package twice and assert byte equality, verify metadata, extract to a clean directory, and run packaged Unica MCP smoke. Record free space separately; CI remains authoritative for three targets.

- [ ] **Step 4: Push, open ready PR, and wait**

```bash
git push -u origin codex/rlm-nuitka-standalone-consumer
gh pr create --base main --head codex/rlm-nuitka-standalone-consumer --title "build(rlm): consume standalone runtime archives" --body-file "$PR_BODY_FILE"
gh pr checks --watch
```

Require source guardrails, Search, three Rust OS jobs, CodeQL, three exact Build tools jobs, deterministic package verification, extracted MCP smoke, thin bootstrap probes, and no unresolved actionable review. Rebase/renumber if main advanced; wait for replacement exact-head checks; merge normally without bypass.

---

### Task 7: Repeat Benchmark and Update Both Issues

**Files:**
- No tracked source files; raw JSON remains under ignored `docs-local/`.

**Interfaces:**
- Produces: valid source/build.2/standalone comparison and closes the measurement evidence loop.

- [ ] **Step 1: Enforce preflight**

Before every variant require at least `20 GiB` physically free, exact fixture head/count, tracked clean state, marker absence, empty disjoint state, and exact published identity. Stop instead of marking passed on any failure.

- [ ] **Step 2: Run reviewed workloads**

Run the same 133-command full benchmark and 20-start startup benchmark with identical selected paths and disjoint states. Preserve command ledger, fast-path/fresh status, restoration, final cleanliness, and stderr.

- [ ] **Step 3: Validate and publish sanitized results**

Require identical repo head/selection/command order, all incremental fast paths, all fresh samples, final clean state, and no absolute/client paths. Report cold/noop/mutation medians/ranges, startup first/median/p95, build time, archive/extracted/final sizes, and aggregate-child RSS definition.

Idempotently replace the preliminary section in Unica #505 and append downstream evidence to upstream #29. Distinguish speed from the download/installed-size cost; close only if each issue acceptance criterion is actually met.
