# PR 182 Form Design Guide Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Correct every actionable issue found in PR #182: make its form UX guidance complete and executable, record the adapted MIT source, and make PR change classification robust to a moving base branch.

**Architecture:** Keep one canonical UX section mirrored between the reference and prompt-visible skill under a byte-for-byte contract test. Extend the existing native Form XML emitter only for the already documented properties used by that section. Register the upstream and packaged license in the existing provenance/attribution inventories. Preserve the full checkout graph in CI instead of re-shallowing it before a triple-dot diff.

**Tech Stack:** Rust and `serde_json`, Python 3.12 `unittest`, Markdown, JSON, GitHub Actions YAML.

## Global Constraints

- Code and tests are the source of truth; prose must not advertise unsupported `radio` elements.
- Keep the public server and tool boundary as `unica` / `unica.*`.
- Keep prompt-visible instructions MCP-first.
- Pin adapted material to `Oxotka/1CDesignGuide@edc05eaf5c191250a184b0e185006bf4b412f7a5`.
- Keep `ATTRIBUTIONS.md` manually authored and package the exact upstream MIT notice.
- Fix tests before implementation for each independent defect.
- Do not expand this PR into the complete Form DSL appearance or element-type parity backlog.

---

### Task 1: Make the documented tooltip and button appearance executable

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/native_operations/form.rs`

**Interfaces:**
- Consumes element JSON keys `tooltip`, `tooltipRepresentation`, button `backColor`, and button `font`.
- Produces Form XML tags `<ToolTip>`, `<ToolTipRepresentation>`, `<BackColor>`, and `<Font .../>`.
- String `font` values are style references; object values accept the documented font attributes and serialize booleans in lowercase.

- [ ] **Step 1: Add focused failing Rust tests**

Add tests beside existing `form_compile_xml` coverage. Compile an input and a button whose definitions contain the four properties and assert the exact XML fragments and their ordering. Include XML-special characters in a tooltip or font value to prove escaping.

- [ ] **Step 2: Prove the regression**

Run:

```bash
cargo test -p unica-coder \
  infrastructure::native_operations::form::tests::form_compile_emits_tooltip_and_button_appearance \
  -- --exact --nocapture
```

Expected: FAIL because the current emitter silently drops the properties.

- [ ] **Step 3: Implement the narrow emitter support**

Add small helpers for tooltip and font emission. Reuse the existing localized-text and XML escaping helpers. Call tooltip emission for currently supported input/check/button/label-field kinds where a title can be emitted, and call appearance emission only for buttons. Reject or ignore nothing new beyond the established permissive optional-property behavior.

- [ ] **Step 4: Run focused and module tests**

```bash
cargo test -p unica-coder \
  infrastructure::native_operations::form::tests::form_compile_emits_tooltip_and_button_appearance \
  -- --exact --nocapture
cargo test -p unica-coder infrastructure::native_operations::form -- --nocapture
cargo fmt --check
```

Expected: PASS.

- [ ] **Step 5: Commit the runtime contract**

```bash
git add crates/unica-coder/src/infrastructure/native_operations/form.rs
git -c commit.gpgsign=false commit -m "fix: emit documented form button properties"
```

---

### Task 2: Record the 1C Design Guide source and license

**Files:**
- Modify: `tests/ci/test_attributions.py`
- Modify: `plugins/unica/provenance/skill-upstreams.json`
- Modify: `plugins/unica/ATTRIBUTIONS.md`
- Create: `plugins/unica/third-party/licenses/1c-design-guide/LICENSE`

**Interfaces:**
- Adds upstream marker `<!-- unica-attribution: upstream 1c-design-guide -->`.
- Adds provenance upstream `1c-design-guide`, role `guidance`, with one adapted `form-patterns` entry.
- Local license link resolves inside the packaged plugin and includes `Copyright (c) 2024 Nikita` plus the complete MIT permission notice.

- [ ] **Step 1: Add the failing inventory expectation**

Extend `test_expected_markers_follow_package_inventories` to require `("upstream", "1c-design-guide")` and add a focused assertion that its attribution section references the local license.

- [ ] **Step 2: Prove the missing attribution**

```bash
python3.12 -m unittest tests.ci.test_attributions -v
```

Expected: FAIL because neither provenance nor the manually written section exists yet.

- [ ] **Step 3: Add provenance, editorial attribution, and exact license**

Register repository, tracking ref, pinned commit, upstream and local paths, test contract, and adapted decision in `skill-upstreams.json`. Add the human-readable section to `ATTRIBUTIONS.md`, naming Nikita Aripov and linking the repository, pinned baseline, and packaged license. Copy the exact upstream MIT license text into the new license file.

- [ ] **Step 4: Verify all offline contracts**

```bash
python3.12 -m unittest tests.ci.test_attributions tests.ci.test_skill_provenance tests.ci.test_package_unica_plugin -v
python3.12 scripts/ci/check-attributions.py
python3.12 scripts/ci/check-skill-upstreams.py --validate-only
```

Expected: PASS with no inventory, marker, or license errors.

- [ ] **Step 5: Commit provenance**

```bash
git add tests/ci/test_attributions.py plugins/unica/provenance/skill-upstreams.json \
  plugins/unica/ATTRIBUTIONS.md plugins/unica/third-party/licenses/1c-design-guide/LICENSE
git -c commit.gpgsign=false commit -m "docs: attribute adapted form design guidance"
```

---

### Task 3: Correct and complete the form UX guidance

**Files:**
- Modify: `plugins/unica/references/specs/form-patterns.md`
- Modify: `plugins/unica/skills/form-patterns/SKILL.md`
- Modify: `tests/ci/test_unica_skills.py`
- Modify: `.gitattributes`
- Modify: `plugins/unica/references/use-cases/forms-ui.md`

**Interfaces:**
- The section headed `## UX-правила для элементов и компоновки форм` is identical in skill and reference through the next horizontal rule.
- Guidance uses `tooltip` / `tooltipRepresentation`, exactly one `defaultButton`, and supported binary `checkBoxType: "switcher"`.
- Multivalue tumbler advice explicitly avoids presenting unsupported native `radio` DSL.

- [ ] **Step 1: Add failing content and synchronization tests**

Add an extractor for the UX section and assert equality. Assert the pinned source URL and coverage for: ordinary/collapsible/popup groups, command panel, commands, header, footer, large link click target, positive checkbox wording, switcher/tumbler distinction, context-sensitive primary-button placement, and valid tooltip/default-button examples. Reject `buttonHint`, `RGB(`, a second default button, and an executable `radio` claim.

- [ ] **Step 2: Prove the current PR prose is incomplete**

```bash
python3.12 -m unittest tests.ci.test_unica_skills.UnicaSkillContractTests -v
```

Expected: FAIL on missing sections, divergent mirrors, and invalid DSL keys.

- [ ] **Step 3: Rewrite the canonical UX section and mirror it**

Adapt the complete applicable source guidance in concise Russian. Explain the deliberate double-negative example. Separate the single default action from visual emphasis of equivalent actions. Replace `buttonHint` with `tooltip` plus `tooltipRepresentation: "Button"`. Correct the old blanket “buttons at bottom” principle to full-screen top-left versus modal bottom-right.

Keep existing detailed advanced DSL examples outside the synchronized section. Remove the whitespace-only `forms-ui.md` change. Preserve the existing CRLF file intentionally by adding a narrow `.gitattributes` `whitespace=cr-at-eol` rule rather than normalizing the whole skill and obscuring review history.

- [ ] **Step 4: Verify the skill contract and whitespace hygiene**

```bash
python3.12 -m unittest tests.ci.test_unica_skills -v
git diff --check origin/pr/182...
```

Expected: PASS and no trailing-whitespace diagnostics.

- [ ] **Step 5: Commit the guidance**

```bash
git add .gitattributes plugins/unica/references/specs/form-patterns.md \
  plugins/unica/skills/form-patterns/SKILL.md tests/ci/test_unica_skills.py \
  plugins/unica/references/use-cases/forms-ui.md
git -c commit.gpgsign=false commit -m "docs: complete form design guidance"
```

---

### Task 4: Preserve the merge base in PR change classification

**Files:**
- Modify: `.github/workflows/unica-plugin-release.yml`
- Modify: `tests/ci/test_unica_workflow.py`

**Interfaces:**
- `actions/checkout` retains `fetch-depth: 0`.
- The explicit base fetch contains no shallow depth.
- Change classification retains `${{ github.base_ref }}...HEAD` and all `FORCE_FULL` behavior.

- [ ] **Step 1: Add a failing workflow regression test**

Assert that the workflow contains `fetch-depth: 0`, fetches `github.base_ref` without `--depth`, does not contain `git fetch --no-tags --depth=1`, and still computes a triple-dot diff.

- [ ] **Step 2: Prove the shallow-fetch defect**

```bash
python3.12 -m unittest tests.ci.test_unica_workflow -v
```

Expected: FAIL because the current command re-shallows the full checkout.

- [ ] **Step 3: Remove only the shallow-depth flag**

Keep the fetch, branch selection, `FORCE_FULL` gates, and triple-dot comparison unchanged.

- [ ] **Step 4: Verify the workflow contract**

```bash
python3.12 -m unittest tests.ci.test_unica_workflow -v
```

Expected: PASS.

- [ ] **Step 5: Commit the CI fix**

```bash
git add .github/workflows/unica-plugin-release.yml tests/ci/test_unica_workflow.py
git -c commit.gpgsign=false commit -m "fix: preserve merge base in plugin CI"
```

---

### Task 5: Integrate, review, and update the existing PR

**Files:**
- Modify only files required by failures introduced or exposed by Tasks 1-4.

- [ ] **Step 1: Review the complete base-to-head diff**

Check scope, source-of-truth contradictions, generated/package contracts, prose accuracy, JSON validity, and accidental whitespace-only changes. Run a fresh independent code review against `origin/main...HEAD` and resolve every actionable finding.

- [ ] **Step 2: Run the full local verification matrix**

```bash
cargo fmt --check
cargo test --workspace
python3.12 -m unittest discover -s tests/ci --durations 20
python3.12 scripts/ci/check-attributions.py
python3.12 scripts/ci/check-skill-upstreams.py --validate-only
python3.12 -m json.tool plugins/unica/provenance/skill-upstreams.json >/dev/null
git diff --check origin/pr/182...
git status --short
```

Expected: all commands PASS; status is clean.

- [ ] **Step 3: Guard against a moved remote PR head**

Read PR #182 again. If the remote head changed since `cb46934539b48f7550214dcf12806f6ab8277db7`, fetch and integrate it without force-pushing or discarding work, then repeat Step 2.

- [ ] **Step 4: Push to the existing contributor branch**

Push the verified commits to `Oxotka/unica:add-ux-rules-to-form-patterns` through maintainer access, without force. Do not open a second PR.

- [ ] **Step 5: Monitor GitHub checks and close the review loop**

Wait for required checks, diagnose and fix any failures through the same test-first cycle, then post a concise Russian PR comment describing corrected runtime behavior, attribution, guidance coverage, CI root cause, and verification results.
