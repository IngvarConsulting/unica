# PR 605 Architecture v2 Review Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make PR 605 an internally consistent, semantically audited and mechanically enforced migration from architecture v1 to v2 without changing runtime delivery behavior.

**Architecture:** Keep the archive, Fate ledger, active records and their guards in one PR. Strengthen the registry and immutability guards first, then make Fate prove why every v1 subject was carried, superseded or retired, and finally align the loading decisions with the already-merged runtime. Runtime delivery resilience remains a separate future PR from `main`.

**Tech Stack:** Python 3.12, `unittest`, Markdown front matter, Git, GitHub CLI,
`tree-sitter==0.25.2`, `tree-sitter-rust==0.24.0`, Rust tests used as named
architectural evidence.

**Spec:** `docs/design/2026-08-21-pr-605-architecture-v2-review-fixes-design.md`

## Global Constraints

- PR 605 changes architecture, checks and documentation only; it does not change loader threads, timeouts, locks, retry or public `unica.*` contracts.
- Every defect starts with a focused test that fails for the reviewed reason before implementation changes.
- `docs/arch-v1/` remains byte-frozen except for `FATE.md`; regenerate `MANIFEST.sha256` after the ledger audit and prove that only the `FATE.md` digest changed.
- A v2 invariant or contract names one exact test such as `path::named_test`; a file containing many tests is not a falsifier.
- A product rule imported unchanged may cite the process decision `DEC.2026-08-18.CARRIED-RULES`; a later edit to that product rule requires a newly introduced active, realized product decision.
- Decisions not yet present in `main` are corrected in place and are not given a false supersession history.
- Preserve one PR based on `main`; do not create a stacked PR.
- Use `python3.12` for repository Python checks.

---

### Task 1: Enforce the published registry schema

**Files:**
- Modify: `scripts/arch/registry.py`
- Modify: `tests/arch/test_registry.py`
- Modify: `arch/README.md`
- Create: `arch/decisions/2026-08-21-platform-xml-profile.md`
- Create: `arch/decisions/2026-08-21-list-cache-fields.md`
- Modify: `arch/contracts/CTR.FORMAT.PLATFORM-XML-8-3-27.md`
- Modify: `arch/contracts/CTR.WIRE.LIST-CACHE-FIELDS.md`
- Modify: `arch/contracts/CTR.WIRE.TOOL-SURFACE.md`

**Interfaces:**
- Consumes: `Record`, `records()`, `REQUIRED_PROPS`, front-matter values parsed by `parse_front_matter()`
- Produces: `validation_errors(found: list[Record]) -> list[str]`, strict props for every record kind, two active product decisions grounding the previously ungrounded contracts

- [ ] **Step 1: Add failing schema tests**

Add fixture-backed tests to `tests/arch/test_registry.py` that construct temporary `Record` values and call `REGISTRY.validation_errors()`:

```python
def contract_record(props: dict) -> REGISTRY.Record:
    return REGISTRY.Record(
        id=props.get("id", ""),
        kind="contract",
        path=REGISTRY.ARCH_ROOT / "contracts" / "CTR.WIRE.EXAMPLE.md",
        props=props,
        body="# Example\n",
    )

def test_contract_requires_scope_consumers_and_a_decision() -> None:
    props = {
        "id": "CTR.WIRE.EXAMPLE",
        "status": "active",
        "governs": "product",
        "version": "1",
        "producer": "src/example.rs",
        "check": "tests/arch/test_registry.py::test_contract_requires_scope_consumers_and_a_decision",
    }
    errors = REGISTRY.validation_errors([contract_record(props)])
    self.assertTrue(any("scope" in error for error in errors), errors)
    self.assertTrue(any("consumers" in error for error in errors), errors)
    self.assertTrue(any("decision" in error for error in errors), errors)
```

Add separate tests for `decision: null`, a reference to an invariant instead of a decision, a superseded decision behind an active rule, an empty list, `version: 0`, and `version: text`.

- [ ] **Step 2: Run the focused tests and confirm the current validator is absent or too weak**

Run:

```bash
python3.12 -m unittest \
  tests.arch.test_registry.RecordShapeTests.test_contract_requires_scope_consumers_and_a_decision \
  tests.arch.test_registry.RecordShapeTests.test_active_rules_reference_active_decisions \
  tests.arch.test_registry.RecordShapeTests.test_contract_version_is_a_positive_integer -v
```

Expected: FAIL because `validation_errors` does not exist and current required props omit `scope` and `consumers`.

- [ ] **Step 3: Implement centralized validation**

Extend the schema and add the validator in `scripts/arch/registry.py`:

```python
REQUIRED_PROPS = {
    "decision": ("id", "status", "governs", "realized"),
    "invariant": ("id", "status", "governs", "decision", "check", "scope"),
    "contract": (
        "id", "status", "governs", "version", "decision",
        "producer", "consumers", "check", "scope",
    ),
}

def validation_errors(found: list[Record]) -> list[str]:
    by_id = {record.id: record for record in found}
    errors: list[str] = []
    for record in found:
        for key in REQUIRED_PROPS[record.kind]:
            if key not in record.props or record.props[key] in (None, ""):
                errors.append(f"{record.relative}: missing prop `{key}`")
        if record.kind in {"invariant", "contract"}:
            for key in ("scope",) + (("consumers",) if record.kind == "contract" else ()):
                if not isinstance(record.props.get(key), list) or not record.props[key]:
                    errors.append(f"{record.relative}: `{key}` must be a non-empty list")
            owner = by_id.get(record.props.get("decision"))
            if owner is None or owner.kind != "decision":
                errors.append(f"{record.relative}: decision does not resolve to a decision")
            elif record.props.get("status") == "active" and owner.props.get("status") != "active":
                errors.append(f"{record.relative}: active rule cites a non-active decision")
        if record.kind == "contract":
            version = str(record.props.get("version", ""))
            if not version.isdecimal() or int(version) < 1:
                errors.append(f"{record.relative}: version must be a positive integer")
    return errors
```

Make `registry.py --check` print these errors and return 1 before comparing the generated index. Keep symbol/path, known status, known governs and evidence-existence checks in their existing focused tests; do not duplicate their prose in the validator.

- [ ] **Step 4: Ground the two contracts that currently use `decision: null`**

Create `DEC.2026-08-21.PLATFORM-XML-PROFILE` with:

```yaml
status: active
governs: product
realized: crates/unica-coder/tests/format_8_3_27_xml_corpus.rs::source_resource_reads_preserve_every_corpus_byte
establishes: [CTR.FORMAT.PLATFORM-XML-8-3-27]
```

Its single decision is that resource reads preserve every corpus byte for
Platform XML 8.3.27 / format 2.20. It grounds only
`CTR.FORMAT.PLATFORM-XML-8-3-27`; Task 5 later supersedes broad ADR-0016 with
the complete set of exact source invariants. Create
`DEC.2026-08-21.LIST-CACHE-FIELDS` with:

```yaml
status: active
governs: product
realized: crates/unica-coder/src/interfaces/mcp.rs::modern_list_results_carry_required_cache_fields_and_legacy_stays_clean
establishes: [CTR.WIRE.LIST-CACHE-FIELDS]
```

Its single decision is that modern `tools/list` carries `ttlMs` and `cacheScope`, while legacy responses omit them. Point each contract at its product decision and add `scope: [platform]`, `scope: [wire]`, and `scope: [wire]` respectively to the format, list-cache and tool-surface contracts.

- [ ] **Step 5: Run registry tests and regenerate the index**

Run:

```bash
python3.12 -m unittest tests.arch.test_registry -v
python3.12 scripts/arch/registry.py --write-index
python3.12 scripts/arch/registry.py --check
```

Expected: PASS; both new decisions and all three contracts appear in `arch/index.md`.

- [ ] **Step 6: Commit the schema slice**

```bash
git add scripts/arch/registry.py tests/arch/test_registry.py arch/README.md \
  arch/decisions/2026-08-21-platform-xml-profile.md \
  arch/decisions/2026-08-21-list-cache-fields.md arch/contracts arch/index.md
git commit -m "fix(arch): enforce registry record schema"
```

### Task 2: Reject false product-rule grounds

**Files:**
- Modify: `scripts/arch/immutability.py`
- Modify: `tests/arch/test_product_immutability.py`
- Modify: `tests/ci/requirements.txt`
- Modify: `docs/design/2026-08-21-pr-605-architecture-v2-review-fixes-design.md`

**Interfaces:**
- Consumes: base records returned by `_records_at()`, current records below `arch/`
- Produces: `_records_introduced(repo: Path, base: dict[str, str]) -> dict[str, IntroducedRecord]`; a product edit is admitted only by a new active, realized product decision

- [ ] **Step 1: Replace the permissive positive fixture with explicit grounds**

Add an evidence file to each temporary repository, and define five introduced grounds: invariant, planned product decision, active process decision, unrealized active product decision, and realized active product decision. The valid ground must contain:

```yaml
id: DEC.2026-03-03.WHY-IT-CHANGES
status: active
governs: product
realized: tests/evidence.py::test_reason
```

The fixture creates `tests/evidence.py` containing `def test_reason(): pass`.

- [ ] **Step 2: Add the four failing negative tests and one positive test**

Add a helper that writes an introduced record, rewrites the accepted rule to its
ID and returns the verdict:

```python
def point_rule_at(self, filename: str, text: str, identifier: str):
    target = self.fixture.root / "arch" / "decisions" / filename
    target.write_text(text, encoding="utf-8")
    self.fixture.rule.write_text(
        RULE.replace("так, а не иначе", "уже совсем иначе").replace(
            "DEC.2026-01-01.PROMISE", identifier
        ),
        encoding="utf-8",
    )
    return self.fixture.inspect()
```

Use it from the exact methods
`test_a_new_invariant_is_not_a_product_ground`,
`test_a_planned_decision_is_not_a_product_ground`,
`test_a_process_decision_is_not_a_product_ground`,
`test_an_unrealized_active_decision_is_not_a_product_ground`, and
`test_an_active_realized_product_decision_is_a_ground`. Each negative test
asserts one offender and the rejected property (`decision`, `planned`,
`process`, or `realized`) in the diagnostic; the realized product case asserts
an empty offender tuple.

- [ ] **Step 3: Run the negative cases and reproduce the bypass**

Run:

```bash
python3.12 -m unittest \
  tests.arch.test_product_immutability.ProductImmutabilityTests.test_a_new_invariant_is_not_a_product_ground \
  tests.arch.test_product_immutability.ProductImmutabilityTests.test_a_planned_decision_is_not_a_product_ground \
  tests.arch.test_product_immutability.ProductImmutabilityTests.test_a_process_decision_is_not_a_product_ground \
  tests.arch.test_product_immutability.ProductImmutabilityTests.test_an_unrealized_active_decision_is_not_a_product_ground -v
```

Expected: FAIL because all introduced IDs are currently accepted.

- [ ] **Step 4: Implement typed introduced records**

Add:

```python
@dataclass(frozen=True)
class IntroducedRecord:
    kind: str
    path: str
    props: dict

def _records_introduced(repo: Path, base: dict[str, str]) -> dict[str, IntroducedRecord]:
    known = {
        props["id"]
        for text in base.values()
        if (props := _split(text)[0]).get("id")
    }
    introduced: dict[str, IntroducedRecord] = {}
    for directory, kind in (
        ("decisions", "decision"),
        ("invariants", "invariant"),
        ("contracts", "contract"),
    ):
        for path in sorted((repo / "arch" / directory).glob("*.md")):
            props, _ = _split(path.read_text(encoding="utf-8"))
            identifier = props.get("id")
            if identifier and identifier not in known:
                introduced[identifier] = IntroducedRecord(
                    kind=kind,
                    path=path.relative_to(repo).as_posix(),
                    props=props,
                )
    return introduced
```

Derive `kind` from the directory. For a changed product invariant or contract,
reject the new ground unless its record is a decision with `status == "active"`,
`governs == "product"`, and a non-empty `realized`. Resolve `path::name`
against the temporary repository as a function definition: Python uses stdlib
`ast`; Rust uses a pinned `tree-sitter-rust` syntax tree and requires an exact
attributed `function_item` with a body. Do not accept comments, strings, trait
signatures, macro token trees or identifier substrings. The two parser pins live
in `tests/ci/requirements.txt`, which CI installs before the arch checks.

- [ ] **Step 5: Run focused and full immutability tests**

Run:

```bash
python3.12 -m unittest tests.arch.test_product_immutability -v
python3.12 scripts/arch/immutability.py --base origin/main
```

Expected: fixture suite PASS; live check honestly reports zero base product records until v2 reaches `main`.

- [ ] **Step 6: Commit the immutability slice**

```bash
git add scripts/arch/immutability.py tests/arch/test_product_immutability.py
git commit -m "fix(arch): validate new product rule grounds"
```

### Task 3: Make Fate retirement evidence machine-checkable

**Files:**
- Modify: `scripts/arch/fate.py`
- Modify: `tests/arch/test_fate_coverage.py`
- Modify: `docs/arch-v1/FATE.md`

**Interfaces:**
- Consumes: v1 ADR files, v1 invariant/requirement blocks, v1 acceptance files, v2 front matter
- Produces: `FateRow(subject: str, fate: str, successors: tuple[str, ...], reason: str)` and checked retirement reasons

- [ ] **Step 1: Extend fixture rows to four columns and add failing reason tests**

Use this fixture header:

```markdown
| Subject | Fate | Successor | Reason |
| --- | --- | --- | --- |
| `ADR-0001` | `retired` | — | `historical-only` |
| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` | — |
```

Add tests rejecting: missing reason for retired, non-empty reason for carried, `historical-only` on INV/REQ, `check-removed` while a named check resolves, `tool-surface-bound` when the old rule contains no literal `unica.*` identity, and `behavior-removed` pointing to a missing or planned decision.

- [ ] **Step 2: Run fixture tests and confirm the three-column parser accepts invalid retirement**

Run:

```bash
python3.12 -m unittest tests.arch.test_fate_coverage -v
```

Expected: FAIL in the new reason tests.

- [ ] **Step 3: Implement typed Fate parsing and v1 block inspection**

Add a frozen dataclass and parse the fourth cell. Parse every v1 INV/REQ block into its body and last inline-code value from each `**Check:**` line; the first inline code is the check class and must not be mistaken for the path. A check resolves when its file exists and, if `::name` is present, that name occurs in the file.

Apply these rules:

```python
RETIRED_REASONS = {"tool-surface-bound", "check-removed", "historical-only"}

# behavior-removed: DEC.YYYY-MM-DD.NAME is parsed separately.
# carried/superseded require reason == "—" and at least one successor.
# retired forbids successors and requires an allowed, evidenced reason.
# historical-only is limited to ADR-* and acceptance/*.
# check-removed requires at least one old check and zero resolving checks.
# tool-surface-bound requires a literal `unica.*` name in the old rule block;
# this prevents using it for generic cache/source rules.
# behavior-removed requires an active decision; its governs side matches the
# product or process behavior being removed.
```

- [ ] **Step 4: Mechanically add the fourth column without falsifying reasons**

Set Reason to `—` for every existing `carried` and `superseded` row. Set retired ADR and acceptance rows to `historical-only` only after confirming their derived INV/REQ/contract rows remain separately represented. Leave retired INV/REQ rows failing until Tasks 4–6 classify them; do not use `tool-surface-bound` merely to make the suite green.

- [ ] **Step 5: Run fixture-only tests**

Run individual fixture tests, excluding `test_every_v1_subject_has_exactly_one_fate`, until the ledger audit is complete:

```bash
python3.12 -m unittest \
  tests.arch.test_fate_coverage.FateCoverageTests.test_a_complete_fate_ledger_passes \
  tests.arch.test_fate_coverage.FateCoverageTests.test_a_missing_retirement_reason_is_rejected \
  tests.arch.test_fate_coverage.FateCoverageTests.test_a_live_check_cannot_be_called_removed \
  tests.arch.test_fate_coverage.FateCoverageTests.test_a_generic_rule_cannot_claim_tool_surface_retirement -v
```

Expected: PASS for fixture behavior; the live ledger remains deliberately red until its domain audit is finished.

- [ ] **Step 6: Commit the guard and explicit red ledger state**

Commit only if the commit message states that the following domain commits complete the ledger before push:

```bash
git add scripts/arch/fate.py tests/arch/test_fate_coverage.py docs/arch-v1/FATE.md
git commit -m "test(arch): require evidence for Fate retirement"
```

### Task 4: Audit public-surface, application and cache fates

**Files:**
- Modify: `docs/arch-v1/FATE.md`
- Create or modify: records under `arch/invariants/INV.APP.*.md`
- Create or modify: records under `arch/invariants/INV.CACHE.*.md`
- Create or modify: records under `arch/invariants/INV.SURFACE.*.md`
- Create or modify: records under `arch/invariants/INV.WIRE.*.md`
- Modify: `arch/decisions/2026-08-18-carried-rules.md`

**Interfaces:**
- Consumes: old rule bodies and resolving checks reported by the Fate guard
- Produces: a truthful fate for every retired `INV-PRODUCT-*`, `INV-MCP-*`, `INV-SKILL-*`, `INV-APP-*` and `INV-CACHE-*` subject

- [ ] **Step 1: Classify literal public-surface rules**

Use `tool-surface-bound` only when the old rule block names the retired `unica.*` tool or argument whose identity no longer survives. Rules about transport, cache, provider neutrality, redaction, cancellation or source behavior remain independent even if their old ID starts `INV-MCP-`.

The following retired MCP rules must be carried or superseded rather than dismissed solely by prefix: `INV-MCP-DATA-DRIVEN-SCHEMA`, `INV-MCP-SDK-TRANSPORT`, `INV-MCP-VERSION-TIERS`, and `INV-MCP-DEFERRED-READ`.

- [ ] **Step 2: Carry every surviving APP and CACHE obligation**

For every retired `INV-APP-*` and `INV-CACHE-*` row whose old check still resolves, either name an existing v2 successor that proves the complete rule or preserve its semantic suffix under the dotted `INV.APP` or `INV.CACHE` namespace. Use `governs: process` for implementation boundaries such as no direct Git/script backend and `governs: product` for externally observable support, fallback, state and cache behavior.

Each created record follows this exact shape:

```markdown
---
id: INV.CACHE.WORKSPACE-ROOT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/workspace.rs::v8project_yaml_in_ancestor_defines_workspace_root
scope: [cache]
---

# Короткое проверяемое утверждение

Только свойство, которое полностью фальсифицирует названный тест.
```

If the old file contains no single complete falsifier, add a focused aggregate test to that existing test module first; do not cite the file alone and do not broaden the rule beyond that test.

- [ ] **Step 3: Reconcile preview rules**

Map `INV-CACHE-WRITE-FREE-PREVIEW` to existing `INV.CACHE.INDEX-PREVIEW-WRITE-FREE` plus the new preview-default invariant created in Task 6. Do not claim write-free behavior for workspace state or hidden services unless a focused test proves each part.

- [ ] **Step 4: Run APP/CACHE focused Fate and registry checks**

Run:

```bash
python3.12 scripts/arch/fate.py
python3.12 -m unittest tests.arch.test_registry tests.arch.test_fate_coverage -v
```

Expected at this intermediate point: no APP/CACHE retirement errors; remaining failures name only later SOURCE/PKG/CI/DOC/REQ subjects.

- [ ] **Step 5: Commit the domain audit**

```bash
git add docs/arch-v1/FATE.md arch/invariants arch/decisions/2026-08-18-carried-rules.md
git commit -m "docs(arch): audit surface application and cache fates"
```

### Task 5: Audit source fates

**Files:**
- Modify: `docs/arch-v1/FATE.md`
- Create or modify: records under `arch/invariants/INV.SOURCE.*.md`
- Test when a named falsifier is missing: existing Rust modules named by each v1 SOURCE check

**Interfaces:**
- Consumes: all retired `INV-SOURCE-*` rows
- Produces: exact v2 source invariants or justified product decisions removing behavior

- [ ] **Step 1: Compare each retired SOURCE rule with existing v2 successors**

Use the existing `INV.SOURCE.ATOMIC-PUBLISH`, `DEFAULT-SET-SELECTION`, `EXACT-VERSION`, `FORMAT-PER-SET`, `OBSERVED-BYTES`, `SNAPSHOT-BINDING`, and `WRITE-CONTAINMENT` only when their named check proves the entire old rule. A merely similar heading is not a successor.

- [ ] **Step 2: Create exact records for uncovered live rules**

Preserve every word of the old stable semantic suffix and replace the `INV-SOURCE-` namespace separator with the dotted `INV.SOURCE.` form. Set `governs: product`, `decision: DEC.2026-08-18.CARRIED-RULES`, and `scope: [source]`. Name one exact test from the old check modules; when the old rule spans several modules, add one aggregate test at the narrowest existing boundary and formulate the rule to that test. Set ADR-0016 in Fate to `superseded` only after the complete exact source-invariant set exists; the narrow read-preservation contract from Task 1 is not its successor on its own.

- [ ] **Step 3: Refuse silent behavior removal**

If a source behavior truly no longer exists, create an active product decision describing its removal, point Fate to `behavior-removed: DEC.*`, and prove absence with a named rejection/compatibility test. Do not use `check-removed` while any old check path still resolves.

- [ ] **Step 4: Run source and architecture tests**

Run:

```bash
cargo test -p unica-coder source -- --test-threads=1
python3.12 scripts/arch/fate.py
python3.12 -m unittest tests.arch.test_registry tests.arch.test_fate_coverage -v
```

Expected: no SOURCE entries remain as unjustified retired rows.

- [ ] **Step 5: Commit the source audit**

```bash
git add docs/arch-v1/FATE.md arch/invariants/INV.SOURCE.*.md \
  arch/decisions crates/unica-coder
git commit -m "docs(arch): carry verified source invariants"
```

### Task 6: Restore safety, compatibility, package, CI and documentation guarantees

**Files:**
- Modify: `docs/arch-v1/FATE.md`
- Create: `arch/invariants/INV.SAFETY.PREVIEW-BY-DEFAULT.md`
- Create: `arch/invariants/INV.SAFETY.STREAM-SECRET-REDACTION.md`
- Create: `arch/invariants/INV.SAFETY.RUNTIME-SECRET-REDACTION.md`
- Create: `arch/invariants/INV.SAFETY.CONFIG-ERROR-REDACTION.md`
- Create: `arch/invariants/INV.CI.ALL-TARGETS-GREEN.md`
- Create: `arch/invariants/INV.PKG.OLDEST-CLIENT-LOAD.md`
- Create or modify: remaining `INV.PKG.*`, `INV.CI.*`, `INV.DOC.*`, `INV.PERF.*`, `INV.TOKEN.*`, `INV.OBS.*`, `INV.MAINT.*`, `INV.COMPAT.*`, `INV.REL.*`
- Modify: `tests/ci/test_unica_workflow.py`
- Modify: `tests/ci/test_product_contracts.py`

**Interfaces:**
- Consumes: remaining retired PKG/CI/DOC and every retired REQ row
- Produces: named product/process guarantees and a Fate ledger with zero unjustified retirements

- [ ] **Step 1: Add a failing Fate regression and exact compatibility evidence tests**

Add `test_live_safety_and_compatibility_guarantees_are_not_retired` to `tests/arch/test_fate_coverage.py`; it asserts that the six mandatory successors listed in Step 3 resolve and that the five reviewed v1 subjects are `carried` or `superseded`, never `retired`.

Also add `test_every_supported_target_must_pass_before_publication` to `tests/ci/test_unica_workflow.py`; it asserts the Linux, Windows and macOS matrices and that publication depends on the complete build/probe contour. Add `test_release_gate_pins_the_oldest_supported_client` to `tests/ci/test_product_contracts.py`; it asserts `CLAUDE_CLI_VERSION: 2.1.69`, installation of that exact version and the version equality check in `.github/workflows/unica-plugin-release.yml`.

- [ ] **Step 2: Run the new tests before changing architecture records**

Run the architecture regression first:

```bash
python3.12 -m unittest \
  tests.arch.test_fate_coverage.FateCoverageTests.test_live_safety_and_compatibility_guarantees_are_not_retired -v
```

Expected: FAIL because the reviewed subjects are currently retired and the successor records do not exist.

Then run the evidence tests:

```bash
python3.12 -m unittest \
  tests.ci.test_unica_workflow.ArtifactSplitPublicationTests.test_every_supported_target_must_pass_before_publication \
  tests.ci.test_product_contracts.ProductContractTests.test_release_gate_pins_the_oldest_supported_client -v
```

Expected: PASS on current behavior. These two tests name the existing evidence; the preceding Fate test is the required failing reproduction of the architecture defect.

- [ ] **Step 3: Create the mandatory safety and compatibility records**

Use these exact checks:

```text
INV.SAFETY.PREVIEW-BY-DEFAULT
  crates/unica-coder/src/application/mod.rs::mutating_tool_defaults_to_dry_run_and_reports_cache
INV.SAFETY.STREAM-SECRET-REDACTION
  crates/unica-coder/src/infrastructure/redaction.rs::stream_redactor_redacts_secret_key_split_across_chunks
INV.SAFETY.RUNTIME-SECRET-REDACTION
  crates/unica-coder/src/infrastructure/runtime_jobs.rs::terminal_snapshot_and_persistence_are_redacted_and_keep_log_artifacts
INV.SAFETY.CONFIG-ERROR-REDACTION
  crates/unica-coder/src/infrastructure/operational_config.rs::read_errors_are_redacted_to_the_fixed_basename
INV.CI.ALL-TARGETS-GREEN
  tests/ci/test_unica_workflow.py::test_every_supported_target_must_pass_before_publication
INV.PKG.OLDEST-CLIENT-LOAD
  tests/ci/test_product_contracts.py::test_release_gate_pins_the_oldest_supported_client
```

All six derive from `DEC.2026-08-18.CARRIED-RULES`; safety/package records govern product, while the all-target aggregate gate governs process. Update the corresponding old Fate rows to `superseded` and list every narrower successor for the broad secret-redaction requirement.

- [ ] **Step 4: Audit all remaining package, CI, doc and quality rows**

For every remaining retired row:

- PKG and externally observable PERF/TOKEN/SAFETY/OBS/COMPAT/REL rules govern product;
- CI, DOC and MAINT rules govern process;
- use an existing v2 successor only for complete semantic coverage;
- create a focused v2 invariant with a named check for surviving behavior;
- use `check-removed` only when all old checks fail resolution;
- use `behavior-removed: DEC.*` only with an active product removal decision and a named test;
- never use `tool-surface-bound` for package, CI, documentation or generic quality rules.

- [ ] **Step 5: Make the complete Fate ledger pass**

Run:

```bash
python3.12 scripts/arch/fate.py
python3.12 -m unittest tests.arch.test_fate_coverage -v
```

Expected: `architecture-v1 fate coverage: 233 subjects`, with zero errors and no retired live rule hidden behind an invalid reason.

- [ ] **Step 6: Commit the final Fate audit**

```bash
git add docs/arch-v1/FATE.md arch/invariants tests/ci/test_unica_workflow.py \
  tests/ci/test_product_contracts.py
git commit -m "docs(arch): restore verified safety and compatibility rules"
```

### Task 7: Align cache and loading decisions with the implementation

**Files:**
- Modify: `arch/decisions/2026-08-19-artifact-versioned-cache.md`
- Modify: `arch/decisions/2026-08-19-delivery-has-no-budget.md`
- Modify: `arch/decisions/2026-08-20-engines-come-from-the-toolchain.md`

**Interfaces:**
- Consumes: already-merged retention and delivery code from PR 604
- Produces: three active decisions that do not contradict sibling decisions or claim unavailable recovery

- [ ] **Step 1: Record why this prose defect has no synthetic regression test**

The defect is a contradiction between natural-language active decisions, not a machine-observable branch. A test matching Russian phrases would duplicate the decision and pin wording rather than the protected property. Preserve the existing runtime tests as evidence and verify the correction by reading all three decisions together; this is the documented exception to the normal failing-test rule.

- [ ] **Step 2: Correct the cache decision**

State that collection is owned by `DEC.2026-08-19.RETENTION-BY-ARTIFACT`, retains two newest deliveries per artifact and deliberately does not count core-version references. Do not modify retention code or create a superseding decision.

- [ ] **Step 3: Correct cancellation and failure-domain prose**

State that call cancellation stops waiting but not server-owned delivery; only MCP process termination ends an infinite slow transfer, and the partial survives for Range resume. State that `unica-toolchain` adds an independent repository/release availability domain even on the same GitHub provider; checksum protects integrity, not availability, and prefetch is preparation rather than fallback.

- [ ] **Step 4: Run focused delivery evidence and registry shape tests**

Run:

```bash
python3.12 -m unittest tests.arch.test_registry -v
cargo test -p unica-bootstrap a_slow_channel_is_not_cut_off_for_being_slow -- --exact
cargo test -p unica-coder a_cancelled_call_does_not_cancel_the_delivery -- --exact
```

Expected: PASS without runtime source changes.

- [ ] **Step 5: Commit the decision corrections**

```bash
git add arch/decisions
git commit -m "docs(arch): align delivery decisions with runtime"
```

### Task 8: Regenerate architecture artifacts and run the complete local gate

**Files:**
- Modify: `arch/index.md`
- Modify when required by the frozen-archive test: `docs/arch-v1/MANIFEST.sha256`
- Verify only: all other repository files

**Interfaces:**
- Consumes: completed records, Fate ledger and guards
- Produces: reproducible index, frozen archive proof and clean local verification evidence

- [ ] **Step 1: Regenerate the index and archive manifest through repository tools**

Run:

```bash
python3.12 scripts/arch/registry.py --write-index
```

Regenerate the frozen manifest mechanically from the archive root:

```bash
(
  cd docs/arch-v1
  find . -type f ! -name MANIFEST.sha256 -print | LC_ALL=C sort |
    while IFS= read -r path; do shasum -a 256 "$path"; done |
    sed 's#  \./#  #' > MANIFEST.sha256
)
```

Confirm its diff changes the digest of `FATE.md` only; every moved v1 source file remains byte-identical.

- [ ] **Step 2: Run all architecture and Python gates**

```bash
python3.12 -m unittest discover -s tests/arch
python3.12 -m unittest discover -s tests/ci --durations 20
python3.12 -m unittest discover -s tests/dev --durations 20
python3.12 scripts/arch/registry.py --check
python3.12 scripts/arch/fate.py
python3.12 scripts/arch/immutability.py --base origin/main
```

Expected: PASS; the live immutability report may compare zero base v2 records until merge, while fixture tests prove the non-zero behavior.

- [ ] **Step 3: Run Rust formatting, lint and workspace tests**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --test-threads=1
git diff --check origin/main...HEAD
```

Expected: PASS and no whitespace errors.

- [ ] **Step 4: Verify archive moves remain byte-identical**

Run the existing frozen-manifest test and inspect rename detection:

```bash
python3.12 -m unittest tests.arch.test_registry.LayerBoundaryTests.test_archive_matches_frozen_manifest -v
git diff --find-renames=100% --summary origin/main...HEAD
```

Expected: moved v1 files remain 100% renames; only Fate and its generated manifest metadata differ inside the archive.

- [ ] **Step 5: Commit generated artifacts**

```bash
git add arch/index.md docs/arch-v1/MANIFEST.sha256
git commit -m "docs(arch): regenerate reviewed architecture artifacts"
```

If neither file changed, skip the empty commit and record that fact in the PR verification matrix.

### Task 9: Push PR 605 and request semantic re-review

**Files:**
- External mutation: branch `codex/pr-601-arch-v2`
- External mutation: GitHub PR 605 body

**Interfaces:**
- Consumes: clean verified branch and local commit sequence
- Produces: updated remote PR with reviewable scope and current checks

- [ ] **Step 1: Refresh the base and prove no unexpected divergence**

```bash
git fetch --no-tags origin main
git merge-base origin/main HEAD
git rev-parse origin/main
git status --short
```

Expected: clean worktree. If `origin/main` advanced, merge it with `git merge --no-edit origin/main`, resolve only this PR's files, then rerun Task 8; do not rewrite the remote PR history for base refresh.

- [ ] **Step 2: Push without rewriting reviewed history**

```bash
git push origin codex/pr-601-arch-v2
```

Expected: fast-forward update. Do not force-push merely to split the original large commit; the design makes commit restructuring optional, not a merge gate.

- [ ] **Step 3: Replace the empty PR body**

Create ignored `.superpowers/pr-605-body.md` with `apply_patch` and this complete body, replacing only the final check results with the exact observed values from Task 8:

```markdown
## Что меняется

Architecture v1 побайтно замораживается в `docs/arch-v1/`, а действующим слоем становится проверяемый реестр `arch/` из `DEC.*`, `INV.*` и `CTR.*`. PR исправляет выявленные ревью противоречия Fate, схемы props, неизменяемости продуктовых правил и решений о поставке.

## Граница

- База — `main` после слитого PR #604.
- 95 архивных путей перенесены без изменения байтов.
- Runtime-поведение доставки, публичная поверхность `unica.*`, таймауты, потоки и блокировки не меняются.
- Устойчивость бесконечно медленной доставки остаётся отдельным будущим PR от `main`.

## Архитектурные изменения

- Fate содержит обязательное доказуемое основание для каждого `retired` и полный аудит 233 субъектов v1.
- Живые safety, compatibility, cache, source, package и CI-гарантии получили точные v2 successors и именованные проверки.
- Registry требует `scope`, `consumers`, положительную `version` и активное decision-основание.
- Immutability принимает изменение product-rule только с новым active, realized product-decision.
- GC принадлежит `DEC.2026-08-19.RETENTION-BY-ARTIFACT`; reference counting не используется.
- Отмена MCP-вызова не отменяет серверную доставку; второй repository/release pipeline признан отдельной областью доступности.

## Проверка

- `python3.12 -m unittest discover -s tests/arch` — PASS
- `python3.12 -m unittest discover -s tests/ci --durations 20` — PASS
- `python3.12 -m unittest discover -s tests/dev --durations 20` — PASS
- `cargo fmt --all -- --check` — PASS
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — PASS
- `cargo test --workspace -- --test-threads=1` — PASS
- `git diff --check origin/main...HEAD` — PASS

## Merge gate

Нужны зелёные required checks и человеческое семантическое ревью. Статус CodeRabbit не считается ревью, если бот снова пропустил PR из-за числа файлов.
```

Then run:

```bash
gh pr edit 605 --repo IngvarConsulting/unica --body-file .superpowers/pr-605-body.md
```

- [ ] **Step 4: Wait for GitHub checks and inspect review state**

```bash
gh pr checks 605 --repo IngvarConsulting/unica --watch
gh pr view 605 --repo IngvarConsulting/unica \
  --json mergeStateStatus,mergeable,reviewDecision,reviews,statusCheckRollup
```

Expected: required checks green. A CodeRabbit success paired with a skipped-review comment remains non-evidence; request or perform human semantic review of the active v2 records.

- [ ] **Step 5: Report merge readiness without merging automatically**

Report exact head SHA, checks, unresolved review threads and whether every original finding is closed. Merge only after the user separately authorizes it or the existing instruction explicitly asks to carry PR 605 through merge.
