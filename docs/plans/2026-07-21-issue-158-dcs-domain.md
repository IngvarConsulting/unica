# Canonical DCS Domain Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the incorrect public `skd` domain with canonical `dcs` across Unica while preserving DCS operation behavior and platform XML compatibility.

**Architecture:** Perform one atomic pre-1.0 contract migration with no compatibility alias. A package-contract test defines the public boundary first; the implementation then renames the MCP registry, native operation identifiers, skills, active specifications, fixtures, and provenance while preserving `DataCompositionSchema` and `SetMainSKD`/`setMainSKD` input spellings.

**Tech Stack:** Rust, Python 3.12 `unittest`, JSON/YAML/Markdown package metadata, GitHub Actions.

## Global Constraints

- Keep one public MCP server named `unica`.
- Expose only `unica.dcs.compile`, `unica.dcs.edit`, `unica.dcs.info`, and `unica.dcs.validate`; do not register `unica.skd.*` aliases.
- Publish only `dcs-compile`, `dcs-edit`, `dcs-info`, and `dcs-validate` prompt-visible skills.
- Preserve operation argument schemas and DCS XML bytes/semantics.
- Preserve `DataCompositionSchema`, `SetMainSKD`, and `setMainSKD` compatibility spellings.
- Keep skills MCP-first; do not add script-backed runtime paths.
- Keep generated GitHub release notes; do not add `docs/releases` or `body_path`.
- Preserve Russian `СКД` in Russian prose; use DCS in English identifiers and prose.

---

### Task 1: Lock the atomic public package contract

**Files:**
- Create: `tests/ci/test_dcs_naming_contract.py`
- Modify: `tests/ci/test_unica_skills.py`

**Interfaces:**
- Consumes: the tracked plugin tree and Rust tool registry.
- Produces: a CI contract that requires the four DCS tools/skills, forbids the SKD aliases, rejects `DSC`, and documents narrow compatibility exceptions.

- [ ] **Step 1: Write the failing contract test**

Create `DcsNamingContractTests` with these assertions:

```python
EXPECTED_TOOLS = {
    "unica.dcs.compile",
    "unica.dcs.edit",
    "unica.dcs.info",
    "unica.dcs.validate",
}
REMOVED_TOOLS = {name.replace(".dcs.", ".skd.") for name in EXPECTED_TOOLS}

def test_public_dcs_migration_is_atomic_without_skd_aliases(self):
    registry = (REPO_ROOT / "crates/unica-coder/src/application/mod.rs").read_text(encoding="utf-8")
    dcs_surface = set(re.findall(r'name: "(unica\.(?:dcs|skd)\.[^"]+)"', registry))
    self.assertEqual(dcs_surface, EXPECTED_TOOLS)
    self.assertTrue(REMOVED_TOOLS.isdisjoint(dcs_surface))

def test_prompt_visible_dcs_skills_replace_skd_skills(self):
    skill_names = {path.name for path in (REPO_ROOT / "plugins/unica/skills").iterdir() if path.is_dir()}
    self.assertTrue({"dcs-compile", "dcs-edit", "dcs-info", "dcs-validate"} <= skill_names)
    self.assertTrue({"skd-compile", "skd-edit", "skd-info", "skd-validate"}.isdisjoint(skill_names))
```

Add a source/path scan over `README.md`, `crates/unica-coder/src`, `plugins/unica`, `scripts`, and active `spec` files. Exclude append-only provenance reviews and ADR/plan migration prose; parse `skill-upstreams.json` separately so only donor `upstreamPaths` may retain `skd`. The identifier regex must not match embedded compatibility fields `SetMainSKD` and `setMainSKD`.

- [ ] **Step 2: Run the new test and verify RED**

Run: `python3.12 -m unittest tests.ci.test_dcs_naming_contract -v`

Expected: FAIL because the registry exposes `unica.skd.*` and the skill tree contains `skd-*`.

- [ ] **Step 3: Rename existing skill-contract expectations to DCS**

Update `SKILL_TO_TOOL`, scenario requirements, task argument keys, additional allowed MCP tools, and DCS-specific test method/local variable names in `tests/ci/test_unica_skills.py`. Keep Russian `СКД` tokens and `SetMainSKD` unchanged.

- [ ] **Step 4: Commit the RED contract**

```bash
git add tests/ci/test_dcs_naming_contract.py tests/ci/test_unica_skills.py
git commit --no-gpg-sign -m "test: define canonical DCS package contract"
```

### Task 2: Rename the native DCS operation surface

**Files:**
- Rename: `crates/unica-coder/src/infrastructure/native_operations/skd.rs` to `crates/unica-coder/src/infrastructure/native_operations/dcs.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/operation_descriptors.rs`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Modify: `crates/unica-coder/src/domain/cache.rs`
- Modify: `crates/unica-coder/src/domain/events.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/{cf,cfe,common,form,interface,meta,mxl,registry,role,subsystem,template}.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`

**Interfaces:**
- Consumes: unchanged tool argument schemas and XML `DataCompositionSchema` format.
- Produces: `unica.dcs.*`, `dcs-*`, `DcsChanged`, `dcs_graph`, and `dcs.rs` with behavior equivalent to the removed names.

- [ ] **Step 1: Rename the Rust module and identifiers**

Use `git mv` for the module and replace domain identifiers consistently:

```text
unica.skd.* -> unica.dcs.*
skd-*       -> dcs-*
skd_*       -> dcs_*
Skd*        -> Dcs*
SKD prose   -> DCS prose
```

Do not replace `SetMainSKD`, `setMainSKD`, or `DataCompositionSchema`.

- [ ] **Step 2: Run focused Rust tests**

Run: `cargo test --package unica-coder application::tests:: -- --test-threads=1`

Expected: all application registry/routing tests pass using DCS names.

Run: `cargo test --package unica-coder infrastructure::native_operations::dcs:: -- --test-threads=1`

Expected: all renamed DCS native-operation tests pass.

- [ ] **Step 3: Commit the native migration**

```bash
git add crates/unica-coder
git commit --no-gpg-sign -m "refactor!: rename native SKD domain to DCS"
```

### Task 3: Migrate package skills, active references, fixtures, and provenance

**Files:**
- Rename: `plugins/unica/skills/skd-{compile,edit,info,validate}` to `plugins/unica/skills/dcs-{compile,edit,info,validate}`
- Rename: `plugins/unica/references/specs/skd-dsl-spec.md` to `plugins/unica/references/specs/dcs-dsl-spec.md`
- Modify: `plugins/unica/.codex-plugin/plugin.json`
- Modify: `plugins/unica/README.md`
- Modify: active files under `plugins/unica/references`, `plugins/unica/skills`, `README.md`, and `spec/architecture`
- Modify: `plugins/unica/provenance/skill-upstreams.json`
- Rename/modify: DCS paths under `tests/fixtures/unica_mcp_script_parity`
- Modify: `tests/ci/test_unica_mcp_script_parity.py`, `tests/ci/test_skill_provenance.py`, `scripts/ci/release-assessment.py`, and `tests/ci/test_release_assessment.py`
- Modify: `scripts/ci/smoke-unica-mcp.py`

**Interfaces:**
- Consumes: the native `unica.dcs.*` tools from Task 2.
- Produces: a packaged DCS-only prompt surface, migrated parity tests/fixtures, explicit donor provenance, and durable migration guidance.

- [ ] **Step 1: Rename tracked package and fixture paths**

Use `git mv` for all active path segments and filenames containing `skd`, including skill directories, `dcs-dsl-spec.md`, Rust-parity reference skills/scripts, cc-1c cases, BSP fixture grouping, and top-level DCS fixture files.

- [ ] **Step 2: Update active content and provenance**

Replace active Unica names with DCS. In `skill-upstreams.json`, rename the four local `skill` values, local plugin/fixture paths, native module paths, and active notes while keeping donor `.claude/skills/skd-*` and `docs/skd-*` upstream paths verbatim.

Add this migration table to `plugins/unica/README.md`:

```markdown
## DCS naming migration

The release containing issue #158 atomically replaces the transliterated
`skd` domain with the official Data Composition System (`dcs`) term. There is
no deprecated alias: replace `unica.skd.compile/edit/info/validate` with
`unica.dcs.compile/edit/info/validate` and `skd-*` skills with `dcs-*`.
```

Extend `scripts/ci/smoke-unica-mcp.py` so `REQUIRED_TOOLS` includes the four
`unica.dcs.*` tools and the smoke fails when any `unica.skd.*` name is present.

- [ ] **Step 3: Run package and parity checks**

Run: `python3.12 -m unittest tests.ci.test_dcs_naming_contract tests.ci.test_unica_skills tests.ci.test_skill_provenance -v`

Expected: PASS.

Run: `python3.12 -m unittest tests.ci.test_unica_mcp_script_parity -v`

Expected: PASS with DCS MCP calls matching the renamed reference fixtures.

- [ ] **Step 4: Commit the package migration**

```bash
git add plugins README.md spec scripts tests
git commit --no-gpg-sign -m "refactor!: publish canonical DCS skills"
```

### Task 4: Verify the source and packaged MCP boundary

**Files:**
- No file changes planned; verification failures return to the owning task.

**Interfaces:**
- Consumes: the completed DCS migration.
- Produces: evidence that source, package, and MCP smoke contracts agree.

- [ ] **Step 1: Run formatting and static checks**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
python3.12 -m py_compile scripts/ci/*.py tests/ci/*.py
git diff --check origin/main...HEAD
```

Expected: every command exits 0.

- [ ] **Step 2: Run all source tests**

```bash
GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=commit.gpgsign GIT_CONFIG_VALUE_0=false python3.12 -m unittest discover -s tests/ci
cargo test --workspace -- --test-threads=1
```

Expected: 0 failures.

- [ ] **Step 3: Build and smoke the current-host package**

Run:

```bash
scripts/dev/install-local-unica.sh \
  --build-dir "$PWD/.build/issue-158-local-package" \
  --skip-install \
  --skip-verify
python3.12 scripts/ci/smoke-unica-mcp.py \
  --binary "$PWD/.build/issue-158-local-package/package/marketplace/plugins/unica/bin/darwin-arm64/unica" \
  --plugin-root "$PWD/.build/issue-158-local-package/package/marketplace/plugins/unica"
```

Expected: the package build exits 0 and smoke prints `verified Unica MCP initialize and tools/list`; its required/forbidden tool contract proves the four DCS tools are present and SKD aliases are absent.

- [ ] **Step 4: Review the final diff and commit any verification-only correction**

Run: `git status -sb && git diff --stat origin/main...HEAD && git diff --check origin/main...HEAD`

Expected: only issue #158 files are changed and the worktree is clean after commits.
