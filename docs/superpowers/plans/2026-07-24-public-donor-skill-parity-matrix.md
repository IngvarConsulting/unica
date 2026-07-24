# Public Donor Skill Parity Matrix Implementation Plan

> **For Codex:** Execute this plan with `superpowers:executing-plans`, applying
> `superpowers:test-driven-development` to every contract or generator change.

**Goal:** Accept Nikolay Shirokov's complete script and test corpus at one exact
commit, distinguish stored coverage from executable parity, and publish a
deterministically generated six-column skill matrix plus a reviewed gap
analysis.

**Architecture:** The accepted fixture remains test-only. Its manifest hashes
the complete corpus, while `executableCaseScopes` selects the JSON case scopes
that require reviewed Unica relations. Existing provenance remains the adoption
authority. A small semantic registry joins donor skills to Unica skills and
tools, and a checked generator projects all authorities into public Markdown.

**Tech Stack:** Python 3.12 standard library, JSON contracts, Markdown,
`unittest`, Git.

---

### Task 1: Separate complete corpus coverage from executable parity

**Files:**
- Modify: `scripts/ci/donor_parity_contract.py`
- Test: `tests/ci/test_donor_parity_contract.py`

- [ ] Add a failing test whose baseline contains a stored-only case scope and
      prove relation validation does not demand a relation for that case.
- [ ] Add failing validation tests for missing, duplicate, or unknown
      `executableCaseScopes`.
- [ ] Run the focused test and observe the expected failure.
- [ ] Introduce baseline schema version 2 and validate:
      `corpusSkills`, `corpusTests`, and `executableCaseScopes`.
- [ ] Select relation-required case IDs strictly from
      `executableCaseScopes`; continue hashing every copied case and file.
- [ ] Preserve strict rejection of missing or extra relations inside the
      executable selection.
- [ ] Run focused contract tests and the full CI unit-test module.
- [ ] Commit the contract change.

### Task 2: Refresh the complete donor corpus without inventing parity

**Files:**
- Modify: `scripts/ci/refresh-cc-1c-parity.py`
- Test: `tests/ci/test_refresh_cc_1c_parity.py`

- [ ] Add a synthetic upstream skill, unselected JSON case scope, fixture, and
      separate suite to the refresh fixture.
- [ ] Add failing tests proving a full-corpus prepare copies all donor scripts,
      all JSON case-runner assets, and separate suites while only selected
      executable scopes enter relation review.
- [ ] Add a failing test proving focused refresh retains its narrow,
      backward-compatible behavior.
- [ ] Run focused tests and observe the expected failures.
- [ ] Add an explicit `--full-corpus` prepare mode.
- [ ] Build the candidate snapshot from the upstream commit using allowlisted
      source roots and regular files only:
      `.claude/skills/*/scripts/**`, `tests/skills/**`, and other discovered
      `tests/**` suites under `suites/`.
- [ ] Generate schema-v2 corpus inventories and hashes; preserve unchanged
      reviewed relations for executable cases.
- [ ] Make review metadata enumerate exact copied upstream paths and the
      executable selection separately.
- [ ] Run focused refresh and contract tests.
- [ ] Commit the refresh change.

### Task 3: Add the semantic mapping registry and deterministic matrix generator

**Files:**
- Create: `plugins/unica/provenance/donor-skill-map.json`
- Create: `scripts/ci/generate-donor-skill-matrix.py`
- Create: `tests/ci/test_generate_donor_skill_matrix.py`
- Modify: `tests/ci/test_skill_provenance.py`

- [ ] Add fixture-based failing tests for all six requested columns, stable row
      ordering, escaping, relation counts, corpus coverage, and adoption derived
      only from `skill-upstreams.json`.
- [ ] Add failing validation tests for unknown donor skills, missing accepted
      rows, unknown Unica skills/tools, invalid relation enums, and stale output.
- [ ] Run the focused tests and observe the expected failures.
- [ ] Implement `--write` and `--check` modes using repository contracts only.
- [ ] Populate one explicit mapping record for every accepted donor skill;
      classify tools as `direct`, `supporting`, or `related`.
- [ ] Derive scripts, case counts, suite coverage, and parity counts from the
      accepted snapshot, baseline, and relations.
- [ ] Add provenance coverage assertions without duplicating adoption state in
      the registry.
- [ ] Run focused generator and provenance tests.
- [ ] Commit the registry and generator.

### Task 4: Accept the exact upstream corpus as the new baseline

**Files:**
- Modify: `tests/fixtures/unica_mcp_script_parity/cc-1c-skills/**`
- Modify: `tests/fixtures/unica_mcp_script_parity/donor-baseline.json`
- Modify: `tests/fixtures/unica_mcp_script_parity/donor-relations.json`
- Modify: `plugins/unica/provenance/skill-upstreams.json`
- Modify: `plugins/unica/provenance/reviews/2026-07-24-cc-1c-skills.json`

- [ ] Fetch donor `main`, resolve and record one immutable target commit.
- [ ] Compare it with both the accepted executable baseline and the general
      provenance baseline.
- [ ] Run full-corpus `prepare` against that exact commit.
- [ ] Inspect candidate path inventory, counts, hashes, and relation carry-over.
- [ ] Review every provenance-tracked changed path and record the baseline
      advance explicitly; do not infer adoption for unmapped skills.
- [ ] Verify existing unchanged executable relations carry forward and any
      changed executable cases are explicitly resolved.
- [ ] Accept the candidate atomically.
- [ ] Run donor contract, refresh, provenance, and package-boundary tests.
- [ ] Commit the accepted corpus and reviewed baseline.

### Task 5: Publish the matrix and snapshot-specific gap analysis

**Files:**
- Create: `docs/donor-skill-parity.md`
- Create: `docs/research/2026-07-24-cc-1c-skills-gap-analysis.md`
- Modify: `README.md`
- Modify: `.github/workflows/build-unica-plugin.yml`
- Modify: `tests/ci/test_package_unica_plugin.py`

- [ ] Generate the public matrix and inspect all donor rows and headline totals.
- [ ] Write the dated analysis of commits, changed files, per-skill deltas, and
      prioritized borrowing opportunities.
- [ ] Explicitly distinguish executable parity, copied-only corpus, related
      Unica capability, and donor adoption.
- [ ] Link the maintained matrix from the repository documentation.
- [ ] Add offline CI `--check` execution and package tests proving the corpus,
      registry, generated docs, and report are not shipped in the plugin.
- [ ] Run the generator in `--check` mode and focused documentation/package
      tests.
- [ ] Commit public documentation and CI freshness checks.

### Task 6: Verify and prepare integration

**Files:**
- Verify all changed files

- [ ] Run `python3.12 -m unittest discover -s tests/ci --durations 20`.
- [ ] Run `cargo build --workspace`.
- [ ] Build the plugin package and inspect its file list for donor corpus leaks.
- [ ] Run `git diff --check`.
- [ ] Run generator `--check` and donor repository-contract validation directly.
- [ ] Review the final diff for source-of-truth conflicts, generated-output
      drift, accidental package expansion, and unsupported parity claims.
- [ ] Use `superpowers:verification-before-completion`.
- [ ] Use `superpowers:finishing-a-development-branch` for the handoff.
