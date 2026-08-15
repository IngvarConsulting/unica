# RLM Nuitka Standalone Toolchain Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `rlm-bsl-index` and `rlm-bsl-mcp` once per target as one Nuitka standalone multidist payload and publish it as one deterministic, fully described archive.

**Architecture:** `unica-toolchain` keeps upstream `v1.33.0` but replaces the RLM PyInstaller builder with a dedicated Nuitka builder. The builder emits one target archive containing two ordinary executable copies, shared standalone dependencies, and a manifest binding every payload file; release metadata hashes and attests that archive as the only target asset.

**Tech Stack:** Python `3.12.10`, uv `0.11.29`, Nuitka `4.1.3`, deterministic `tar.gz`, GitHub Actions, Python `unittest`.

## Global Constraints

- Work in an isolated `codex/rlm-nuitka-standalone` worktree based on fresh `unica-toolchain/origin/main`.
- Source stays tag `v1.33.0`, commit `3e6920cd015a61af4ba7aa1a5f1fedd8bc935549`, tree `4b321de0454d4d0998762659891374a3a1326cd0`, patches `[]`.
- Builder versions are exactly Python `3.12.10`, uv `0.11.29`, Nuitka `4.1.3`; record the actual compiler from the Nuitka XML report.
- Candidate identity is `rlm-tools-bsl-v1.33.0-build.3`. Reject it before merge and release if its tag or release already exists; rebase and consistently choose the next free revision instead of overwriting it.
- Publish one `rlm-tools-bsl-<target>.tar.gz`, checksum, and provenance document per target plus one MIT license: exactly ten release files.
- Payload contains ordinary files only. Reject links, devices, absolute paths, backslashes, `..`, duplicates, undeclared files, and unsafe modes.
- `rlm-bsl-index[.exe]` and `rlm-bsl-mcp[.exe]` are byte-identical ordinary copies of the multidist executable. Links and launcher scripts are forbidden.
- PR CI builds, extracts, and smokes the real archive on `darwin-arm64`, `linux-x64`, and `win-x64` before merge.
- Dispatch release only once, from merged `main`; do not change Unica in this plan.

---

## File Structure

- Modify `toolchain/manifest.py`: add the builder schema and builder-specific archive inventory.
- Create `toolchain/builders/python_nuitka_standalone.py`: setup, multidist compile, report parsing, payload materialization.
- Create `toolchain/runtime_archive.py`: deterministic archive writer and fail-closed validator.
- Modify `scripts/toolchain.py`: dispatch, extracted-archive smoke, observed builder identity.
- Modify `toolchain/provenance.py`: bind the archive and observed builder.
- Modify `manifests/rlm-tools-bsl.json`: select Nuitka without changing source identity.
- Modify `.github/workflows/ci.yml` and `.github/workflows/release-tool.yml`: three-target PR proof and archive release.
- Modify existing manifest/CLI/provenance/repository tests; create focused builder/archive tests.

---

### Task 1: Define Builder and Release Inventory

**Files:**
- Modify: `toolchain/manifest.py`
- Modify: `tests/test_manifest.py`
- Modify: `tests/test_rlm_manifest.py`

**Interfaces:**
- Produces: `PythonNuitkaStandaloneSpec` and builder-specific `expected_asset_names()`.
- Consumes later: the builder, CLI, provenance, workflows.

- [ ] **Step 1: Write failing schema tests**

Assert:

```python
manifest = load_manifest(path)
self.assertIsInstance(manifest.builder, PythonNuitkaStandaloneSpec)
self.assertEqual(manifest.builder.nuitka_version, "4.1.3")
self.assertEqual(expected_asset_names(manifest), {
    "rlm-tools-bsl-darwin-arm64.tar.gz",
    "rlm-tools-bsl-linux-x64.tar.gz",
    "rlm-tools-bsl-win-x64.tar.gz",
})
```

Reject missing/extra `nuitkaVersion`, empty `includePackage`, duplicate `sourceName`, duplicate `assetBase`, and PyInstaller-only `collectAll`.

- [ ] **Step 2: Run RED**

```bash
python3.12 -m unittest tests.test_manifest tests.test_rlm_manifest -v
```

Expected: import/schema failures because the builder kind is absent and RLM still exposes six loose executables.

- [ ] **Step 3: Implement the minimal schema**

```python
@dataclass(frozen=True)
class PythonNuitkaStandaloneSpec:
    kind: Literal["python-nuitka-standalone"]
    python_version: str
    uv_version: str
    nuitka_version: str
    lock_file: str
    include_package: str
    binaries: tuple[BinarySpec, ...]
```

Extend `BuilderSpec`; load exactly those fields. For this builder return `f"{manifest.name}-{target}.tar.gz"`; preserve existing Cargo/PyInstaller inventory behavior.

- [ ] **Step 4: Run GREEN and commit**

```bash
python3.12 -m unittest tests.test_manifest tests.test_rlm_manifest -v
python3.12 -m unittest discover -s tests
git add toolchain/manifest.py tests/test_manifest.py tests/test_rlm_manifest.py
git commit -m "feat(rlm): define Nuitka standalone assets"
```

---

### Task 2: Create the Deterministic Runtime Archive

**Files:**
- Create: `toolchain/runtime_archive.py`
- Create: `tests/test_runtime_archive.py`

**Interfaces:**
- Produces: `PayloadFile(path, sha256, size, executable)`.
- Produces: `write_runtime_archive(...) -> Path` and `validate_runtime_archive(...) -> RuntimeArchiveManifest`.
- Consumes later: the builder and archived-byte smoke.

- [ ] **Step 1: Write deterministic and negative tests**

Assert two writes are byte-identical and all members are sorted with uid/gid/mtime zero, blank owner names, and normalized `0644`/`0755` modes. Separately reject `/absolute`, `payload/../escape`, backslash paths, duplicates, symlink, hardlink, FIFO/device, missing/extra payload, digest/size/mode drift, wrong release/source/target, missing entrypoint, and unequal entrypoint bytes.

- [ ] **Step 2: Run RED**

```bash
python3.12 -m unittest tests.test_runtime_archive -v
```

Expected: import failure for the absent module.

- [ ] **Step 3: Implement writer and validator**

Never use `extractall()`. Validate names through `PurePosixPath`; manually read ordinary member bytes. Write deterministically:

```python
with archive_path.open("wb") as raw:
    with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as tar:
            add_bytes(tar, "manifest.json", manifest_bytes, False)
            for item in sorted(payload, key=lambda item: item.path):
                add_bytes(tar, f"payload/{item.path}", item.payload, item.executable)
```

Manifest schema `1` contains release tag; source ref/commit/tree/patches; target key/triple; entrypoint map; builder identity; exact complete file list. Compare archive membership with that list in both directions before returning.

- [ ] **Step 4: Run GREEN and commit**

```bash
python3.12 -m unittest tests.test_runtime_archive -v
python3.12 -m py_compile toolchain/runtime_archive.py tests/test_runtime_archive.py
git add toolchain/runtime_archive.py tests/test_runtime_archive.py
git commit -m "feat(toolchain): add verified runtime archives"
```

---

### Task 3: Compile One Nuitka Multidist Payload

**Files:**
- Create: `toolchain/builders/python_nuitka_standalone.py`
- Create: `tests/test_python_nuitka_standalone.py`
- Modify: `toolchain/builders/__init__.py`

**Interfaces:**
- Produces: `NuitkaBuildResult(assets: tuple[Path, ...], builder_identity: dict[str, object])`.
- Produces: `build_python_nuitka_standalone(...) -> NuitkaBuildResult`.
- Consumes: existing entrypoint resolution/stub helpers and Task 2 archive writer.

- [ ] **Step 1: Write exact-command RED tests**

Use a fake runner that creates `rlm-bsl-index.dist`, shared library, and report:

```xml
<nuitka-compilation-report nuitka_version="4.1.3">
  <scons_environment c_compiler="Clang" the_cc_name="clang" the_compiler="clang" />
</nuitka-compilation-report>
```

Assert two `--main=` values, `--mode=standalone`, `--include-package=rlm_tools_bsl`, `--include-package-data=rlm_tools_bsl`, `--assume-yes-for-downloads`, and `--report=`. Reject `--onefile` and temp-cache options. Assert archive entrypoints are ordinary, byte-identical, and share the dependency once. Test wrong tool version, module mismatch, missing report/compiler/executable, and unequal copies.

- [ ] **Step 2: Run RED**

```bash
python3.12 -m unittest tests.test_python_nuitka_standalone -v
```

Expected: import failure for the absent builder.

- [ ] **Step 3: Implement exact setup and build**

Generate `rlm-bsl-index.py` and `rlm-bsl-mcp.py`; retain pre-import Windows UTF-8 stdio configuration. Execute:

```python
runner(["uv", "sync", "--frozen", "--no-dev", "--directory", str(source.path), "--python", sys.executable])
runner(["uv", "pip", "install", "--python", str(venv_python), "Nuitka==4.1.3"])
runner([str(venv_python), "-m", "nuitka", "--version"])
```

Compile both mains in one command. Copy the output executable with `shutil.copyfile()` under both command names, copy every remaining `.dist` ordinary file unchanged, normalize modes, parse only expected XML attributes, and call `write_runtime_archive()`.

- [ ] **Step 4: Run GREEN and commit**

```bash
python3.12 -m unittest tests.test_python_nuitka_standalone tests.test_python_builder -v
python3.12 -m py_compile toolchain/builders/*.py
git add toolchain/builders/python_nuitka_standalone.py toolchain/builders/__init__.py tests/test_python_nuitka_standalone.py
git commit -m "feat(rlm): build standalone multidist runtime"
```

---

### Task 4: Dispatch, Smoke, and Record Archived Bytes

**Files:**
- Modify: `scripts/toolchain.py`
- Modify: `toolchain/provenance.py`
- Modify: `tests/test_toolchain_cli.py`
- Modify: `tests/test_provenance.py`

**Interfaces:**
- Consumes: `NuitkaBuildResult` and `validate_runtime_archive()`.
- Produces: provenance whose only target asset is the archive and whose builder object matches its internal manifest.

- [ ] **Step 1: Write RED tests**

Patch the builder to return a valid archive. Assert `build()` validates/extracts into a fresh smoke directory; runs root help for both commands and full index help lifecycle including Cyrillic; passes only the archive to provenance; and passes observed compiler identity. Corrupt the archive after build and assert no metadata is written.

- [ ] **Step 2: Run RED**

```bash
python3.12 -m unittest tests.test_toolchain_cli tests.test_provenance -v
```

Expected: unsupported dispatch and loose-asset smoke assumptions fail.

- [ ] **Step 3: Implement dispatch and archive smoke**

Make `_smoke()` consume a map keyed by `asset_base`. Existing builders derive it from loose assets; Nuitka derives it only from a freshly validated extraction. Keep strict UTF-8 decode and literal output checks. Pass the observed builder identity into `write_target_metadata()`.

- [ ] **Step 4: Run GREEN and commit**

```bash
python3.12 -m unittest tests.test_toolchain_cli tests.test_provenance -v
python3.12 -m unittest discover -s tests
git add scripts/toolchain.py toolchain/provenance.py tests/test_toolchain_cli.py tests/test_provenance.py
git commit -m "feat(toolchain): smoke archived RLM payloads"
```

---

### Task 5: Switch Manifest and Three-Target CI

**Files:**
- Modify: `manifests/rlm-tools-bsl.json`
- Modify: `tests/test_rlm_manifest.py`
- Modify: `tests/test_repository_contract.py`
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/release-tool.yml`

**Interfaces:**
- Produces: build.3 with ten exact release files and a three-target PR gate.

- [ ] **Step 1: Write RED repository/workflow tests**

Require builder kind/versions/source/build revision `3`, three archive assets, three checksums, three provenance files, license, and no loose RLM assets. Require a three-target PR matrix without release permissions; require release CI to install uv for the new builder and attest `dist/*.tar.gz`.

- [ ] **Step 2: Run RED**

```bash
python3.12 -m unittest tests.test_rlm_manifest tests.test_repository_contract -v
```

Expected: old PyInstaller manifest, loose inventory, and Windows-only CI fail.

- [ ] **Step 3: Change manifest and workflows**

Use:

```json
{
  "kind": "python-nuitka-standalone",
  "pythonVersion": "3.12.10",
  "uvVersion": "0.11.29",
  "nuitkaVersion": "4.1.3",
  "lockFile": "uv.lock",
  "includePackage": "rlm_tools_bsl"
}
```

Keep the existing two binary records and smoke checks, order index first for deterministic multidist output, and set `buildRevision` to `3`. Use a 60-minute, `fail-fast: false` PR matrix. Do not add upload/release mutation to PR CI.

- [ ] **Step 4: Run GREEN and commit**

```bash
python3.12 -m unittest discover -s tests
python3.12 -m py_compile scripts/*.py toolchain/*.py toolchain/builders/*.py tests/*.py
docker run --rm -v "$PWD:/repo" -w /repo rhysd/actionlint:1.7.7
python3.12 scripts/toolchain.py validate-source --manifest manifests/rlm-tools-bsl.json --repo-root . --work-dir .build/validate-rlm --out-dir dist/validate-rlm
git diff --check
git add manifests/rlm-tools-bsl.json tests/test_rlm_manifest.py tests/test_repository_contract.py .github/workflows/ci.yml .github/workflows/release-tool.yml
git commit -m "build(rlm): select Nuitka standalone archives"
```

---

### Task 6: Review, Merge, Release, and Audit

**Files:**
- No source changes unless a RED proves a branch-introduced defect.

**Interfaces:**
- Produces: merged producer PR and independently verified immutable release for the consumer plan.

- [ ] **Step 1: Final local gates**

```bash
python3.12 -m unittest discover -s tests
python3.12 -m py_compile scripts/*.py toolchain/*.py toolchain/builders/*.py tests/*.py
git diff --check
git status --short
```

- [ ] **Step 2: Push, open ready PR, and wait for exact-head checks**

```bash
git push -u origin codex/rlm-nuitka-standalone
gh pr create --base main --head codex/rlm-nuitka-standalone --title "build(rlm): publish Nuitka standalone runtime archives" --body-file "$PR_BODY_FILE"
gh pr checks --watch
```

PR evidence includes source, schema, ten-file inventory, archive/extracted sizes, entrypoint hash equality, compiler identities, three target job links, rollback, and preliminary benchmark caveat. Inspect all unresolved review threads; fix introduced findings RED-first in the same PR.

- [ ] **Step 3: Recheck identity and merge normally**

```bash
git fetch origin main
test "$(git merge-base HEAD origin/main)" = "$(git rev-parse origin/main)"
! gh release view rlm-tools-bsl-v1.33.0-build.3 >/dev/null 2>&1
! git ls-remote --exit-code origin refs/tags/rlm-tools-bsl-v1.33.0-build.3 >/dev/null 2>&1
gh pr merge "$PR_NUMBER" --merge --delete-branch
```

- [ ] **Step 4: Dispatch release exactly once and audit it**

```bash
gh workflow run release-tool.yml --ref main -f tool=rlm-tools-bsl
gh run watch "$RELEASE_RUN_ID" --exit-status
audit_dir="$(mktemp -d -t unica-toolchain-rlm-build3-audit.XXXXXX)"
gh release download rlm-tools-bsl-v1.33.0-build.3 --dir "$audit_dir"
find "$audit_dir" -maxdepth 1 -type f -print | sort
```

Require ten files; recompute hashes before chmod; validate checksum/provenance/attestation/internal manifests; extract all targets; smoke both commands and index lifecycle. Preserve the audit path and exact values for the consumer pin. Append producer evidence to issues #505 and upstream #29, but do not claim final performance yet.
