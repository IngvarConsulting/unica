# Issue 115 Attribution Page Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a manually authored, source-backed `ATTRIBUTIONS.md` whose component coverage and packaged license references are enforced offline.

**Architecture:** Keep versions, repositories, and component identity in the existing package manifests. Add a focused Python checker that extracts explicit attribution markers from the handwritten Markdown and compares them with tool, adapter, and provenance inventories; prose remains human-owned. Correct the two provenance/license contradictions before relying on those inventories.

**Tech Stack:** Markdown, JSON package metadata, Python 3.12 standard library and `unittest`, existing plugin packager.

## Global Constraints

- Scope is the public Unica plugin artifact, not development, CI, test, or transitive dependencies.
- `ATTRIBUTIONS.md` is manually written and must never be generated or rewritten from metadata.
- CI is offline; it validates recorded identifiers, HTTPS URLs, and packaged paths, not live network availability or free-form prose accuracy.
- Keep the public MCP boundary as one server named `unica` with `unica.*` tools.
- `ai_rules_1c` contributed ideas only; no copied or adapted material and no redistribution license claim.
- The pinned `v8-runner` source license is `AGPL-3.0-only`, not MIT.
- Fix source metadata and tests together; do not preserve false assertions for compatibility.

---

### Task 1: Correct license and provenance contracts

**Files:**
- Modify: `plugins/unica/third-party/tools.lock.json`
- Create: `plugins/unica/third-party/licenses/v8-runner/LICENSE`
- Modify: `plugins/unica/third-party/NOTICE.md`
- Modify: `plugins/unica/provenance/skill-upstreams.json`
- Create: `plugins/unica/provenance/reviews/2026-07-22-ai-rules-idea-provenance-correction.json`
- Modify: `scripts/ci/check-skill-upstreams.py`
- Modify: `tests/ci/test_skill_provenance.py`
- Modify: `tests/ci/test_package_unica_plugin.py`

**Interfaces:**
- Consumes: pinned commits already declared in `tools.lock.json` and `skill-upstreams.json`.
- Produces: `v8-runner.license == "AGPL-3.0-only"`; `ai-rules-1c.usage == "inspiration-only"`; each `ai-rules-1c` entry has `status == "inspiration-only"`, `primarySource == "unica"`, and `decision == "ignored-with-reason"`.

- [ ] **Step 1: Add failing contract tests**

Add tests with these assertions:

```python
def test_v8_runner_license_matches_pinned_source_and_is_packaged(self) -> None:
    lock = json.loads((self.repo_root() / "plugins/unica/third-party/tools.lock.json").read_text())
    runner = next(tool for tool in lock["tools"] if tool["name"] == "v8-runner")
    self.assertEqual(runner["license"], "AGPL-3.0-only")
    license_path = self.repo_root() / "plugins/unica/third-party/licenses/v8-runner/LICENSE"
    self.assertTrue(license_path.is_file())
    self.assertIn("GNU AFFERO GENERAL PUBLIC LICENSE", license_path.read_text())

def test_ai_rules_is_recorded_as_inspiration_not_adaptation(self) -> None:
    upstream = next(item for item in self.load_provenance()["upstreams"] if item["id"] == "ai-rules-1c")
    self.assertEqual(upstream["usage"], "inspiration-only")
    for entry in upstream["entries"]:
        self.assertEqual(entry["status"], "inspiration-only")
        self.assertEqual(entry["primarySource"], "unica")
        self.assertEqual(entry["decision"], "ignored-with-reason")
        self.assertIn("ideas", entry["decisionReason"])
```

Extend the package-source copy test to expect
`third-party/licenses/v8-runner/LICENSE` in the destination.

- [ ] **Step 2: Run the focused tests and verify the intended failures**

Run:

```bash
python3.12 -m unittest \
  tests.ci.test_skill_provenance.SkillProvenanceTests.test_v8_runner_license_matches_pinned_source_and_is_packaged \
  tests.ci.test_skill_provenance.SkillProvenanceTests.test_ai_rules_is_recorded_as_inspiration_not_adaptation -v
```

Expected: FAIL because the lock says MIT, the AGPL license file is absent, and provenance still says `adapted`/`ported`.

- [ ] **Step 3: Correct the source contracts**

Change the `v8-runner` lock license to `AGPL-3.0-only`. Copy the exact license text from the pinned source URL
`https://github.com/alkoleft/v8-runner-rust/blob/ad72f64222ab0a7e6dfd391adb437a956c0a2428/LICENSE`
into `third-party/licenses/v8-runner/LICENSE`, and reference it from `NOTICE.md`.

Add `"inspiration-only"` to `ALLOWED_STATUSES`. Set `usage` on the `ai-rules-1c` upstream and mechanically correct every one of its entries to the interface above. Each note and decision reason must say that Unica used ideas only, that the local skill is Unica-owned, and that no donor expression was copied or adapted. Record the correction and its reason in the new review JSON without rewriting the dated historical review.

- [ ] **Step 4: Run provenance and packaging contracts**

Run:

```bash
python3.12 -m unittest tests.ci.test_skill_provenance tests.ci.test_package_unica_plugin -v
python3.12 scripts/ci/check-skill-upstreams.py --validate-only
```

Expected: PASS; validation reports `errors: []`.

- [ ] **Step 5: Commit the corrected contracts**

```bash
git add plugins/unica/third-party plugins/unica/provenance \
  scripts/ci/check-skill-upstreams.py tests/ci/test_skill_provenance.py \
  tests/ci/test_package_unica_plugin.py
git commit -m "fix: correct attribution source contracts"
```

### Task 2: Add the offline attribution coverage checker

**Files:**
- Create: `scripts/ci/check-attributions.py`
- Create: `tests/ci/test_attributions.py`

**Interfaces:**
- Consumes markers in the exact form `<!-- unica-attribution: KIND ID -->`, where `KIND` is `project`, `tool`, `adapter`, or `upstream`.
- Produces: `expected_markers(repo_root: Path) -> set[tuple[str, str]]`, `parse_sections(markdown: str) -> dict[tuple[str, str], str]`, and `validate_attributions(repo_root: Path, attribution_file: Path | None = None) -> list[str]`. Consecutive markers inside one level-two Markdown section map to the same section body so related tools can share prose.
- CLI exits 0 and prints `attributions: ok` when valid; otherwise prints one error per line to stderr and exits 1.

- [ ] **Step 1: Write failing parser and inventory tests**

Create `tests/ci/test_attributions.py` using `importlib.util` like `test_skill_provenance.py`. Cover:

```python
def test_expected_markers_follow_package_inventories(self):
    self.assertEqual(module.expected_markers(self.repo_root()), {
        ("project", "unica"),
        ("tool", "bsl-analyzer"), ("tool", "v8-runner"),
        ("tool", "rlm-tools-bsl"), ("tool", "rlm-bsl-index"),
        ("adapter", "v8std"),
        ("upstream", "cc-1c-skills"), ("upstream", "ai-rules-1c"),
        ("upstream", "v8-runner-rust"),
    })

def test_parse_sections_rejects_duplicate_markers(self):
    with self.assertRaisesRegex(ValueError, "duplicate attribution marker: tool demo"):
        module.parse_sections("<!-- unica-attribution: tool demo -->\nA\n<!-- unica-attribution: tool demo -->\nB")

def test_validation_reports_missing_unknown_and_invalid_links(self):
    errors = module.validate_attributions(fixture_root, fixture_root / "ATTRIBUTIONS.md")
    self.assertIn("missing attribution: tool demo", errors)
    self.assertIn("unknown attribution: tool ghost", errors)
    self.assertIn("tool other: repository link must be absolute HTTPS", errors)
```

Use temporary fixture roots with minimal JSON manifests; do not mutate repository files in tests.

- [ ] **Step 2: Run tests and verify import/function failures**

Run: `python3.12 -m unittest tests.ci.test_attributions -v`

Expected: FAIL because `scripts/ci/check-attributions.py` does not exist.

- [ ] **Step 3: Implement the minimal checker**

Implement with the standard library only. Parse marker lines using:

```python
MARKER_RE = re.compile(r"^<!-- unica-attribution: (project|tool|adapter|upstream) ([a-z0-9][a-z0-9._-]*) -->$")
LINK_RE = re.compile(r"\[[^]]+\]\(([^)]+)\)")
```

`expected_markers` reads the four authoritative files, excludes the `unica` tool from tool markers, and adds it as the project marker. `parse_sections` splits on level-two headings, collects all markers in each heading block, and maps every collected marker to that complete block; duplicate markers remain errors. `validate_attributions` compares exact sets. Project, tool, and upstream sections require labelled `Repository:` and `Author:` HTTPS links. Tool and non-inspiration upstream sections must also contain a labelled `License:` link; local license links must resolve inside `plugins/unica`, while external HTTPS license links are permitted for non-redistributed sources. Adapter sections instead require `Provider:` and `Service:` HTTPS links and the phrase `not redistributed`.

- [ ] **Step 4: Run checker tests and the real CLI**

Run:

```bash
python3.12 -m unittest tests.ci.test_attributions -v
python3.12 scripts/ci/check-attributions.py
```

Expected: unit tests PASS; the real CLI FAILS only with `ATTRIBUTIONS.md not found` until Task 3.

- [ ] **Step 5: Commit the checker**

```bash
git add scripts/ci/check-attributions.py tests/ci/test_attributions.py
git commit -m "test: enforce attribution coverage"
```

### Task 3: Write and package the attribution page

**Files:**
- Create: `plugins/unica/ATTRIBUTIONS.md`
- Modify: `README.md`
- Modify: `plugins/unica/README.md`
- Modify: `tests/ci/test_attributions.py`
- Modify: `tests/ci/test_package_unica_plugin.py`

**Interfaces:**
- Consumes: marker/checker contract from Task 2 and corrected source metadata from Task 1.
- Produces: a Russian-language editorial page covering every expected marker, plus discoverable README links and packaged-file proof.

- [ ] **Step 1: Add failing repository and packaging assertions**

Add tests that call `validate_attributions(repo_root)` and expect `[]`, assert both READMEs contain a relative link to `ATTRIBUTIONS.md`, and use `copy_tracked_plugin_source` to prove the staged plugin contains `ATTRIBUTIONS.md` plus all referenced local license files.

- [ ] **Step 2: Run tests and verify missing-page/link failures**

Run:

```bash
python3.12 -m unittest tests.ci.test_attributions tests.ci.test_package_unica_plugin -v
```

Expected: FAIL because the page and README links do not exist.

- [ ] **Step 3: Verify upstream facts before writing**

For every pinned repository, inspect the pinned commit rather than the current default branch. Use `gh api repos/OWNER/REPO/contents/PATH?ref=COMMIT` for LICENSE, package metadata, and author-bearing notices. Record project or contributor names only when a pinned primary file states them; otherwise thank the named project contributors and link its contributor history rather than inventing a personal author.

Explicit facts already established by the approved design:

- `v8-runner` at `ad72f64222ab0a7e6dfd391adb437a956c0a2428` is AGPL-3.0-only;
- `ai_rules_1c` supplied ideas only and published no license at its baseline;
- `rlm-tools-bsl`'s pinned LICENSE credits Stefan O'Shea and Roman Starchenko;
- `cc-1c-skills`'s pinned LICENSE credits Nick Shirokov.

- [ ] **Step 4: Write the page by hand**

Write natural prose, not a manifest dump. Include these sections:

```markdown
# Авторы, источники и лицензии

## Unica
<!-- unica-attribution: project unica -->

## Встроенные инструменты
<!-- unica-attribution: tool bsl-analyzer -->
<!-- unica-attribution: tool v8-runner -->
<!-- unica-attribution: tool rlm-tools-bsl -->
<!-- unica-attribution: tool rlm-bsl-index -->

## Внешние сервисы
<!-- unica-attribution: adapter v8std -->

## Источники поведения и идей
<!-- unica-attribution: upstream cc-1c-skills -->
<!-- unica-attribution: upstream ai-rules-1c -->
<!-- unica-attribution: upstream v8-runner-rust -->

## Как читается цепочка лицензий
## Благодарности
```

Group the two RLM tools in one narrative while retaining both markers. Explain aggregation versus adaptation, identify local license paths, state that v8std is not redistributed, and label `ai_rules_1c` as inspiration without a redistribution claim.

- [ ] **Step 5: Add README links and pass focused tests**

Add `Авторы, источники и лицензии` links near the existing license sections. Run:

```bash
python3.12 scripts/ci/check-attributions.py
python3.12 -m unittest tests.ci.test_attributions tests.ci.test_package_unica_plugin -v
```

Expected: `attributions: ok`; all tests PASS.

- [ ] **Step 6: Commit the manual page**

```bash
git add plugins/unica/ATTRIBUTIONS.md README.md plugins/unica/README.md \
  tests/ci/test_attributions.py tests/ci/test_package_unica_plugin.py
git commit -m "docs: add packaged attribution page"
```

### Task 4: Full validation and issue handoff

**Files:**
- Modify only files required by failures caused by Tasks 1-3.

**Interfaces:**
- Consumes: all preceding task deliverables.
- Produces: a clean branch with repository, package, provenance, and Rust tests passing.

- [ ] **Step 1: Run all Python CI tests**

Run: `python3.12 -m unittest discover -s tests/ci -p 'test_*.py' -v`

Expected: PASS with zero failures and errors.

- [ ] **Step 2: Run provenance and attribution validators**

Run:

```bash
python3.12 scripts/ci/check-skill-upstreams.py --validate-only
python3.12 scripts/ci/check-attributions.py
```

Expected: no provenance errors and `attributions: ok`.

- [ ] **Step 3: Run the Rust workspace suite**

Run: `cargo test`

Expected: PASS. If the known bootstrap JSON-RPC timeout appears once, rerun only the failing test and report both results; do not conceal the baseline flake.

- [ ] **Step 4: Verify the final diff and package content**

Run:

```bash
git diff --check origin/main...HEAD
git status --short
git log --oneline origin/main..HEAD
```

Expected: no whitespace errors, no uncommitted files, and focused commits for contracts, checker, page, and any test-only repair.

- [ ] **Step 5: Request code review and prepare GitHub handoff**

Use `superpowers:requesting-code-review`. Address only verified findings, then summarize the corrected license/provenance contradictions, tests, and the manual-maintenance limitation. Do not close issue #115 until the branch is published and CI/package verification succeeds.
