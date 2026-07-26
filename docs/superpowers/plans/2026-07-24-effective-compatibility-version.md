# Effective Compatibility Version Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the `LineNumberLength` compatibility policy derive an effective platform behavior version instead of treating `DontUse` as a version-independent state.

**Architecture:** Add a pure policy helper that accepts both the metadata compatibility literal and the active platform line. Normalize `DontUse` to the platform line, normalize explicit `VersionX` values to their encoded version, and keep the existing `8.3.26` boundary and fail-closed enum validation. The production path supplies `ACTIVE_FORMAT_PROFILE.platform_line`.

**Tech Stack:** Rust 2021, existing `FormatProfile`, native metadata writer unit tests, Markdown skill documentation.

## Global Constraints

- The active mutation profile remains platform `8.3.27` and export format `2.20`.
- Explicit compatibility modes remain bounded by the active enum contract.
- `LineNumberLength` is fixed at `5` for effective versions through `8.3.26`.
- `LineNumberLength` accepts `5..=9` for effective versions newer than `8.3.26`.
- Unknown modes and invalid active platform lines fail closed.
- No public MCP schema or filesystem transaction behavior changes.

---

### Task 1: Normalize the effective compatibility version

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta.rs`

**Interfaces:**
- Consumes: `ACTIVE_FORMAT_PROFILE.platform_line`, `cf_validate_enum_allowed`, and `MetaEditLineNumberLengthPolicy`.
- Produces: `meta_edit_line_number_length_policy_for_platform(mode, platform_line) -> MetaEditLineNumberLengthPolicy`.

- [x] **Step 1: Write the failing policy matrix**

Add a unit test with these expectations:

```rust
for (mode, platform_line, expected) in [
    ("DontUse", "8.3.26", MetaEditLineNumberLengthPolicy::FixedFive),
    ("DontUse", "8.3.27", MetaEditLineNumberLengthPolicy::Editable),
    ("DontUse", "8.5.4", MetaEditLineNumberLengthPolicy::Editable),
    ("Version8_3_24", "8.5.4", MetaEditLineNumberLengthPolicy::FixedFive),
    ("Version8_3_27", "8.5.4", MetaEditLineNumberLengthPolicy::Editable),
] {
    assert_eq!(
        meta_edit_line_number_length_policy_for_platform(mode, platform_line),
        expected,
    );
}
```

Also assert that `DontUse` with `8.3.27.2074` and `invalid` returns
`UnknownCompatibility`.

- [x] **Step 2: Run the test and verify RED**

Run:

```bash
cargo test --package unica-coder line_number_length_policy_uses_effective_platform_version -- --exact
```

Expected: compilation fails because
`meta_edit_line_number_length_policy_for_platform` does not exist.

- [x] **Step 3: Implement minimal normalization**

Import `ACTIVE_FORMAT_PROFILE`. Add a strict three-component dotted platform
line parser and the policy helper:

```rust
fn meta_edit_line_number_length_policy_for_platform(
    mode: &str,
    platform_line: &str,
) -> MetaEditLineNumberLengthPolicy {
    if !cf_validate_enum_allowed("CompatibilityMode").contains(&mode) {
        return MetaEditLineNumberLengthPolicy::UnknownCompatibility;
    }
    let version = if mode == "DontUse" {
        meta_edit_parse_platform_line(platform_line)
    } else {
        mode.strip_prefix("Version")
            .and_then(meta_edit_parse_compatibility_version)
    };
    match version {
        Some(version) if version > (8, 3, 26) => MetaEditLineNumberLengthPolicy::Editable,
        Some(_) => MetaEditLineNumberLengthPolicy::FixedFive,
        None => MetaEditLineNumberLengthPolicy::UnknownCompatibility,
    }
}
```

Make `meta_edit_line_number_length_policy_from_mode` delegate to this helper
with `ACTIVE_FORMAT_PROFILE.platform_line`.

- [x] **Step 4: Run focused tests and verify GREEN**

Run:

```bash
cargo test --package unica-coder line_number_length_policy -- --test-threads=1
```

Expected: all selected tests pass.

### Task 2: Document and validate the effective-version contract

**Files:**
- Modify: `plugins/unica/skills/meta-edit/SKILL.md`
- Modify: `plugins/unica/skills/meta-edit/child-operations.md`

**Interfaces:**
- Consumes: the effective-version policy from Task 1.
- Produces: identical reader-facing semantics in both public documentation locations.

- [x] **Step 1: Update both documentation locations**

State that `DontUse` resolves to the active platform profile, an explicit
`VersionX` resolves to `X`, and values `5..=9` are editable only when that
effective version is newer than `8.3.26`.

- [x] **Step 2: Run focused and repository verification**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --package unica-coder --all-targets --all-features -- -D warnings
cargo test --package unica-coder
python3.12 -m unittest tests.ci.test_unica_skills tests.ci.test_unica_mcp_script_parity
git diff --check
```

Expected: every command exits `0`; Rust and Python suites report zero failures.

- [x] **Step 3: Commit and publish**

Stage only the Rust change, the two skill documents, this plan, and its design
document. Commit as `fix: derive effective compatibility version`, push
`agent/fix-effective-compatibility-version`, and open a draft PR to `main`.

### Task 3: Share the compatibility-mode reasoning contract

**Files:**
- Create: `plugins/unica/references/platform/compatibility-modes.md`
- Modify: `plugins/unica/skills/platform-help/SKILL.md`
- Modify: `plugins/unica/skills/release-support/SKILL.md`
- Modify: `plugins/unica/skills/bsp-patterns/SKILL.md`
- Test: `tests/ci/test_unica_skills.py`

**Interfaces:**
- Consumes: public MCP tools already routed by `platform-help`,
  `release-support`, and `bsp-patterns`.
- Produces: one shared `references/platform/compatibility-modes.md` reasoning
  contract, linked from all three scenario skills.

- [x] **Step 1: Add the failing skill-contract test**

Add this test to `UnicaSkillRoutingTests`:

```python
def test_compatibility_guidance_preserves_effective_version_contract(self) -> None:
    reference_path = (
        self.reference_root() / "platform" / "compatibility-modes.md"
    )
    self.assertTrue(reference_path.is_file())
    reference = reference_path.read_text(encoding="utf-8")

    for token in [
        "runtime platform line",
        "configured compatibility mode",
        "effective compatibility version",
        "`DontUse` -> runtime platform line",
        "`VersionX` -> `X`",
        "`CompatibilityMode`",
        "`ConfigurationExtensionCompatibilityMode`",
        "`InterfaceCompatibilityMode`",
        "code location does not select the mode family",
        "corroborating implementation evidence",
        "not complete old-platform equivalence",
    ]:
        with self.subTest(token=token):
            self.assertIn(token, reference)

    for skill in ["platform-help", "release-support", "bsp-patterns"]:
        skill_text = (
            self.skill_root() / skill / "SKILL.md"
        ).read_text(encoding="utf-8")
        with self.subTest(skill=skill):
            self.assertIn(
                "references/platform/compatibility-modes.md",
                skill_text,
            )
```

- [x] **Step 2: Run the contract test and verify RED**

Run:

```bash
python3.12 -m unittest \
  tests.ci.test_unica_skills.UnicaSkillRoutingTests.test_compatibility_guidance_preserves_effective_version_contract
```

Expected: FAIL because
`plugins/unica/references/platform/compatibility-modes.md` does not exist.

- [x] **Step 3: Add the shared reference**

Create `plugins/unica/references/platform/compatibility-modes.md` with these
sections and exact rules:

```markdown
# Platform Compatibility Modes

## Terms

- **runtime platform line** — the platform line that actually runs or will run
  the infobase;
- **configured compatibility mode** — the literal stored in configuration or
  extension metadata;
- **effective compatibility version** — the behavior version used for a
  feature-specific compatibility decision.

## Normalization

`DontUse` -> runtime platform line

`VersionX` -> `X`
```

The remaining sections must:

- distinguish `CompatibilityMode`,
  `ConfigurationExtensionCompatibilityMode`, and
  `InterfaceCompatibilityMode`;
- require the feature contract, not code location, to select the applicable
  mode family;
- require exact-platform documentation before claiming that a literal such as
  `Version8_5_4` exists;
- state that compatibility mode affects more than old method behavior;
- state that BSP is corroborating implementation evidence, not the platform
  specification;
- state that matching effective versions prove compatibility-controlled
  behavior, not complete old-platform equivalence;
- prescribe an answer matrix containing runtime platform, literal mode,
  effective version, feature result, and evidence.

- [x] **Step 4: Route existing skills to the reference**

Add the exact path `references/platform/compatibility-modes.md`:

- to `platform-help` under `## Platform context`, required for any question
  about a compatibility mode or version-sensitive behavior;
- to `release-support` under `## References`, required for upgrades,
  migrations, and configuration/extension compatibility decisions;
- to `bsp-patterns` in a new `## References` section, required when BSP code
  gates behavior by a platform version or compatibility mode.

In `bsp-patterns`, explicitly require platform documentation to remain the
contract source and BSP code to remain implementation evidence.

- [x] **Step 5: Run focused and full skill verification**

Run:

```bash
python3.12 -m unittest \
  tests.ci.test_unica_skills.UnicaSkillRoutingTests.test_compatibility_guidance_preserves_effective_version_contract
python3.12 -m unittest -v \
  tests.ci.test_unica_skills \
  tests.ci.test_unica_mcp_script_parity
git diff --check
```

Expected: the focused contract passes; the full run reports zero failures
(Windows-only skips remain allowed); `git diff --check` exits `0`.

- [x] **Step 6: Commit and update PR #196**

Stage only the shared reference, three skill files, contract test, and updated
plan. Commit as `docs: add compatibility mode guidance` and push
`agent/fix-effective-compatibility-version`. Verify PR #196 remains open,
mergeable, and all CI checks return to green.
