# Issue #154 CI build and artifact optimization implementation plan

> **For Codex:** Execute this plan task by task with red/green tests and run the
> complete verification section before claiming completion.

**Goal:** Remove duplicate Cargo work and oversized intermediate artifacts while
preserving the existing package, release, deterministic archive, and stable CI
gate contracts defined by ADR-0010.

**Architecture:** The platform `build-tools` matrix owns the complete local
build/package/verify lifecycle. It builds `unica` and `unica-bootstrap` once in a
shared Cargo target directory, verifies the local runtime pair, and exports only
the narrowly scoped runtime metadata, bootstrap, and conditionally required
runtime archive artifacts. Downstream jobs consume those explicit artifact
families; tag publication still verifies the complete published matrix.

**Tech stack:** Python 3.12, `unittest`, Cargo, GitHub Actions YAML, existing
package and release verification scripts.

---

## Task 1: Build workspace binaries once and emit build metrics

**Files:**

- Modify: `tests/ci/test_build_unica_tools.py`
- Modify: `scripts/ci/build-unica-tools.py`

### Step 1: Add failing unit tests

Replace the independent Cargo-tool and bootstrap expectations with one test for
a combined helper. The test must create fake `unica` and `unica-bootstrap`
outputs in the same target directory and prove that:

- exactly one `cargo build` command is issued;
- the command includes `--locked`, both `--package` selections, both `--bin`
  selections, and one `--target-dir`;
- the Unica runtime binary and bootstrap binary are copied into their current
  package layouts;
- elapsed Cargo time is returned for workflow reporting.

Add a separate test for the metrics writer. It must prove a stable JSON schema
containing `schemaVersion`, `target`, and `cargoBuildSeconds` and a trailing
newline.

Run:

```bash
python3.12 -m unittest tests.ci.test_build_unica_tools
```

Expected: failure because the combined helper and metrics writer do not exist.

### Step 2: Implement the combined build

In `build-unica-tools.py`:

- import `time`;
- replace the per-tool Cargo helper and separate bootstrap helper with
  `build_cargo_workspace_binaries(...)`;
- gather all `cargo-workspace` tools and build their packages/binaries together
  with `unica-bootstrap` using one `cargo build --release --locked` invocation;
- validate every requested `(cargoPackage, cargoBin)` pair and reject binary-name
  collisions using locked Cargo workspace metadata before applying Cargo's
  global package/bin filters;
- use `.build/tool-work/<target>/cargo-target` as the only target directory for
  both workspace binaries;
- copy runtime binaries to `bin/<target>/` and bootstrap to
  `bootstrap/bin/<target>/`, retaining executable-mode handling;
- preserve direct release-asset download and checksum behavior;
- add optional CLI argument `--metrics-file` and write the build metrics after a
  successful Cargo invocation.

Keep the existing CLI defaults compatible with local installation scripts.

### Step 3: Run the focused test

```bash
python3.12 -m unittest tests.ci.test_build_unica_tools
```

Expected: pass.

### Step 4: Commit

```bash
git add scripts/ci/build-unica-tools.py tests/ci/test_build_unica_tools.py
git -c commit.gpgsign=false commit -m "ci: build Cargo package binaries once"
```

## Task 2: Verify one freshly packaged runtime pair

**Files:**

- Modify: `tests/ci/test_verify_release_assets.py`
- Modify: `scripts/ci/verify-release-assets.py`

### Step 1: Add a failing single-target contract test

Build only the Linux runtime fixture and prove that the verifier can validate
one `tar.gz` plus metadata pair without requiring the other targets. Retain an
assertion that tampering with the archive fails verification.

Run:

```bash
python3.12 -m unittest tests.ci.test_verify_release_assets
```

Expected: failure because the verifier currently always requires all targets.

### Step 2: Extract target-level verification

Refactor the existing verification body into
`verify_runtime_asset_pair(asset_dir, target) -> str`. Keep every current check:
archive checksum, member set, per-member checksum, executable mode, and zeroed
mtime. Make `verify_release_assets(...)` aggregate the target-level function and
still require one common version across all requested targets.

Add `--target` with choices from the supported target set. With no option, the
CLI must retain full-matrix release verification; with the option, it must
validate just that target pair for the platform job.

### Step 3: Run the focused test

```bash
python3.12 -m unittest tests.ci.test_verify_release_assets
```

Expected: pass.

### Step 4: Commit

```bash
git add scripts/ci/verify-release-assets.py tests/ci/test_verify_release_assets.py
git -c commit.gpgsign=false commit -m "ci: verify platform runtime assets locally"
```

## Task 3: Replace tool bundles and the packaging relay job

**Files:**

- Modify: `tests/ci/test_unica_workflow.py`
- Modify: `.github/workflows/unica-plugin-release.yml`
- Modify: `tests/ci/test_evaluate_ci_gate.py`
- Modify: `scripts/ci/evaluate-ci-gate.py`

### Step 1: Add failing workflow contract tests

Update the workflow tests to require the ADR-0010 contract:

- `build-tools` uses `actions/cache@v5` after the Rust toolchain step;
- the cache key includes runner OS, matrix target, toolchain cache key, and
  `hashFiles('Cargo.lock')`, with no `restore-keys`;
- the build helper always runs and receives `--metrics-file`;
- the job summary exposes `exact-hit`, `miss`, and `error` outcomes plus Cargo
  duration;
- platform jobs package and single-target verify their local runtime pair;
- no `unica-tools-*` artifact or `package-runtime` job remains;
- metadata and bootstrap are separate artifacts for every platform, with exact
  documented paths and one-day retention;
- pull requests and manual runs upload only the Linux runtime archive while tags
  upload all three, also with one-day retention;
- `package-thin` downloads only `unica-runtime-metadata-*` and
  `unica-bootstrap-*`, while `unica-thin-marketplace` has 90-day retention;
- release assessment and tag publishing depend directly on `build-tools`;
- tag publishing still downloads all runtime artifacts and performs published
  full-matrix verification;
- the stable gate no longer expects `package-runtime`.

Update the evaluator unit fixtures and failure case to match the reduced job
graph.

Run:

```bash
python3.12 -m unittest tests.ci.test_unica_workflow tests.ci.test_evaluate_ci_gate
```

Expected: failure against the old workflow/job graph.

### Step 2: Implement build cache, local packaging, and narrow uploads

In the `build-tools` matrix:

- give `dtolnay/rust-toolchain@stable` an id;
- restore/save `.build/tool-work/<target>/cargo-target` via
  `actions/cache@v5`, best-effort, with the exact four-part key and no prefix
  restore keys;
- invoke the build helper with `.build/tool-work` and the target metrics file;
- always write target, normalized cache outcome, and Cargo duration to the job
  summary;
- retain tool contract checks and packaged MCP smoke;
- package the local runtime, then call `verify-release-assets.py --target`;
- stage and upload one-file metadata and exact-layout bootstrap artifacts with
  `retention-days: 1`;
- upload the runtime archive only for Linux or a tag, with
  `retention-days: 1` and no recompression.

Delete `package-runtime`. Rewire downstream jobs as described in the failing
tests and ADR. Add `actions/cache@v5` to the checked Node-action list.

### Step 3: Update the stable gate evaluator

Remove `package-runtime` from `PACKAGE_JOBS` and from all expected result
fixtures. Preserve fail-closed behavior for missing, skipped, failed, and
cancelled jobs that remain in the graph.

### Step 4: Run focused tests

```bash
python3.12 -m unittest tests.ci.test_unica_workflow tests.ci.test_evaluate_ci_gate
```

Expected: pass.

### Step 5: Commit

```bash
git add .github/workflows/unica-plugin-release.yml scripts/ci/evaluate-ci-gate.py tests/ci/test_unica_workflow.py tests/ci/test_evaluate_ci_gate.py
git -c commit.gpgsign=false commit -m "ci: narrow runtime pipeline artifacts"
```

## Task 4: Verify the complete repository contract

### Step 1: Run all CI contract tests

```bash
python3.12 -m unittest discover -s tests/ci
```

Expected: pass.

### Step 2: Run Rust formatting, lint, and tests

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --test-threads=1
```

Expected: all pass.

### Step 3: Run repository hygiene checks

```bash
git diff --check
git status --short
```

Expected: no whitespace errors and only intentional changes before the final
verification commit.

### Step 4: Review against ADR-0010 and issue #154

Inspect the final diff and explicitly check each ADR verification bullet. Do not
claim the performance acceptance evidence locally: cold/warm cache outcomes,
runner time, network volume, and real artifact sizes require GitHub Actions
runs of the implementation commit. Record that evidence in the implementation
PR after the branch is published and rerun unchanged for the warm sample.
