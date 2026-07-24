# Public Donor Skill Parity Matrix Design

## Status

Draft for written user review. The user approved the proposed generated-table
approach on 2026-07-24; implementation remains gated on review of this file.

## Problem

Unica tracks `Nikolay-Shirokov/cc-1c-skills` as a donor, but the information
needed to answer basic maintenance questions is split across several contracts:

- `plugins/unica/provenance/skill-upstreams.json` says which donor skills were
  ported or adapted;
- public Unica skill documents mention the available `unica.*` tools;
- the accepted donor snapshot contains only the scripts and case scopes selected
  for executable parity;
- `donor-relations.json` records reviewed outcomes for those selected cases;
- upstream contains additional scripts and tests which are neither copied nor
  represented in current parity statistics.

This makes it difficult to answer, per donor skill:

1. whether Unica borrowed it;
2. which Unica tools implement or overlap it;
3. which scripts Nikolay uses;
4. whether executable parity exists and what its reviewed result is;
5. whether the upstream test corpus was copied.

The current accepted parity commit is
`e01688e764a3cf1c1b4a0ad5069ea885837cfb2e`. At design time upstream `main`
still resolves to that commit, so there is no newer upstream commit to accept.
The missing work is breadth: the repository has 72 donor skill directories,
65 skills with scripts, and 585 JSON case-runner cases, while the accepted
snapshot contains scripts for 11 skills and 152 cases owned by four parity
skills.

The 585-case count is not the whole upstream test suite. `web-test` has a
separate JavaScript test suite and there are shared/integration test assets.
A public matrix must not report "no tests" merely because a skill does not use
the JSON case runner.

## Goals

1. Maintain one public, reviewable row for every accepted donor skill.
2. Copy and hash all donor skill scripts at the accepted commit.
3. Copy the complete test-only donor corpus needed for future parity work,
   including JSON case-runner cases, referenced fixtures, shared runner assets,
   and separately structured suites such as `web-test`.
4. Keep executable parity distinct from mere corpus availability.
5. Derive relation statistics from reviewed relation records rather than
   manually maintained prose.
6. Produce a public Markdown table which can be regenerated deterministically.
7. Make stale mappings, unknown tools, missing donor files, and stale generated
   output fail offline CI.
8. Produce a snapshot-specific analytical assessment of upstream changes and
   borrowing candidates.

## Non-goals

- Automatically declaring every copied donor case compatible with Unica.
- Calling an unsupported case `donor_ahead` merely because no Unica mapping
  exists.
- Adding 26 currently untracked donor skills to the packaged Unica skill
  surface without individual product decisions.
- Shipping donor scripts or the test corpus in the marketplace plugin.
- Treating a related runtime tool as proof that the donor skill was borrowed.
- Running platform-, browser-, or database-dependent donor suites in normal
  offline CI.
- Replacing `skill-upstreams.json` or `donor-relations.json` with a Markdown
  document.

## Authorities

Conflicts are resolved in this order:

1. current Unica tool contracts, code, and tests;
2. accepted donor snapshot bytes and their baseline manifest;
3. `plugins/unica/provenance/skill-upstreams.json`;
4. reviewed `donor-relations.json`;
5. the explicit donor-to-Unica mapping registry introduced by this design;
6. generated Markdown.

The Markdown table is a projection, never a source of truth.

## Current baseline facts

At design time:

| Measure | Value |
|---|---:|
| Donor skill directories | 72 |
| Donor skills with scripts | 65 |
| Explicit `ported-to-unica` provenance entries | 46 |
| Donor skills with no explicit provenance mapping | 26 |
| Skills with executable donor parity | 4 |
| Accepted executable relation records | 152 |
| Upstream JSON case-runner cases | 585 |
| Accepted JSON cases | 152 |

The 152 reviewed relations contain:

| Relation | Count |
|---|---:|
| `exact` | 4 |
| `compatible` | 8 |
| `intentional_divergence` | 88 |
| `donor_ahead` | 52 |

Per parity owner:

| Donor skill | `exact` | `compatible` | `intentional_divergence` | `donor_ahead` |
|---|---:|---:|---:|---:|
| `cfe-borrow` | 0 | 0 | 6 | 0 |
| `form-compile` | 0 | 2 | 19 | 24 |
| `meta-compile` | 4 | 2 | 41 | 26 |
| `skd-compile` | 0 | 4 | 22 | 2 |

`form-compile` owns both `form-compile` and
`form-compile-from-object` donor case scopes. `skd-*` donor skills map to
`dcs-*` Unica skills.

## Options considered

### Manually maintained Markdown

This is easy to create but duplicates scripts, tools, relation counts, and test
coverage. It will drift after the first upstream or Unica change and provides no
reliable CI invariant.

### Generated Markdown backed by existing contracts and a small mapping registry

This is the selected approach. Facts already present in provenance, the accepted
corpus, tool contracts, and relation records are derived. Only semantic joins
which cannot be inferred safely are maintained explicitly.

### Hosted dashboard as the primary source

A dashboard would improve filtering but adds hosting and a second publication
surface. It can be added later from the same machine-readable data, but the
version-controlled public Markdown remains the first durable output.

## Repository structure

### Full accepted donor corpus

`tests/fixtures/unica_mcp_script_parity/cc-1c-skills/` becomes the full
test-only accepted corpus. It contains:

- `.claude/skills/<skill>/scripts/**` projected under `skills/<skill>/`;
- `tests/skills/cases/**` projected under `cases/`;
- fixtures and shared case-runner assets required by copied cases;
- separately structured test suites, including `tests/web-test/**`, under a
  stable `suites/` subtree;
- no prompt-visible donor skill prose and no marketplace payload.

The refresh review records the exact upstream paths copied. Unsupported suites
may be stored and hashed without being executed.

### Baseline manifest

`tests/fixtures/unica_mcp_script_parity/donor-baseline.json` continues to bind
accepted bytes to an exact commit, but distinguishes:

- `corpusSkills`: all accepted donor skills and script paths;
- `corpusTests`: per-skill JSON cases plus separately structured suite files;
- `executableCaseScopes`: only scopes intentionally selected for donor parity;
- `cases`: content digests for all JSON cases;
- `files`: hashes for every accepted corpus file.

Relation validation uses `executableCaseScopes`; it does not require a relation
for copied but unselected cases. Existing selected relations remain valid when
their content digest is unchanged.

### Semantic mapping registry

Create `plugins/unica/provenance/donor-skill-map.json`.

It stores only joins which cannot be derived safely:

```json
{
  "schemaVersion": 1,
  "upstreamId": "cc-1c-skills",
  "skills": {
    "skd-compile": {
      "unicaSkills": ["dcs-compile"],
      "tools": [
        {"name": "unica.dcs.compile", "relation": "direct"}
      ],
      "caseScopes": ["skd-compile"]
    },
    "db-dump-xml": {
      "unicaSkills": ["v8-runner"],
      "tools": [
        {"name": "unica.runtime.execute", "relation": "related"}
      ],
      "caseScopes": ["db-dump-xml"]
    }
  }
}
```

Allowed tool relations are:

- `direct`: the public Unica tool owns the adopted operation;
- `supporting`: the tool is part of the documented workflow but not the primary
  implementation;
- `related`: similar capability exists, but provenance does not claim the donor
  skill was borrowed.

The registry does not duplicate adoption state. Adoption is derived from
`skill-upstreams.json`.

### Public generated table

Create `docs/donor-skill-parity.md`.

It starts with:

- accepted donor repository and commit;
- generation command;
- definitions of adoption, parity, and corpus coverage;
- headline counts;
- the requested six-column table.

The table columns are:

| Column | Rule |
|---|---|
| Donor skill | One row per accepted donor skill directory |
| Borrowed in Unica | Derived from explicit `ported-to-unica` provenance; includes the local alias |
| Unica tools | Names from the mapping registry, annotated as direct, supporting, or related |
| Nikolay scripts | Exact accepted script paths, not a hand-written summary |
| Parity state | Aggregated reviewed relation counts, `dependency_only`, `not_selected`, or `unmapped` |
| Test corpus state | Copied/total JSON cases plus separately structured suite coverage |

The generated document is public repository documentation. It is not packaged
into `plugins/unica/`.

### Analytical change assessment

Create
`docs/research/2026-07-24-cc-1c-skills-gap-analysis.md`.

It records:

- accepted general provenance baseline and target donor commit;
- commit and changed-file counts;
- per-skill script, case, and documentation changes;
- newly added and removed skills, scripts, and test scopes;
- a prioritized borrowing assessment;
- explicit decisions to borrow, study, defer, or reject.

This report is snapshot-specific and manually reviewed. Future refreshes create
new dated assessments instead of silently rewriting historical conclusions.

## Column semantics

### Borrowed in Unica

This is not a Boolean inferred from similar names.

- `yes: ported` means `skill-upstreams.json` has
  `status=ported-to-unica` and the current decision records a functional port;
- `yes: adapted/tracked` means the donor skill is explicitly tracked and
  adapted, while a particular later donor delta may be
  `ignored-with-reason`;
- `no explicit provenance` means no donor-to-Unica adoption claim exists.

`ignored-with-reason` does not undo an earlier adoption claim.

### Unica tools

Only names present in the live Unica tool registry are allowed. A related tool
is labelled `related`; it is not presented as proof of parity or adoption.

### Parity state

- relation counts are aggregated from `donor-relations.json`;
- `dependency_only` means donor script bytes are needed by selected cases but
  the skill has no independently executed scope;
- `not_selected` means a mapping may exist but the donor cases are not executed;
- `unmapped` means no executable Unica comparison is defined.

`donor_ahead` remains a reviewed relation for an executed case. It is never the
default for an unsupported skill.

### Test corpus state

The cell reports both storage and execution:

- JSON cases: `copied/total`;
- separate suite files: `copied/total` when applicable;
- execution: `reviewed parity`, `stored only`, or `not copied`.

Shared runner files are counted separately and are not attributed to an
individual skill without an explicit mapping.

## Generator

Create `scripts/ci/generate-donor-skill-matrix.py` with two modes:

```text
generate-donor-skill-matrix.py --write
generate-donor-skill-matrix.py --check
```

Inputs:

- accepted donor corpus and baseline;
- `skill-upstreams.json`;
- `donor-skill-map.json`;
- `donor-relations.json`;
- live Unica tool names from the application tool registry.

`--write` deterministically generates the Markdown document.

`--check` fails when:

- an accepted donor skill has no table row;
- a row refers to a missing donor script;
- a mapped Unica skill does not exist;
- a tool name is absent from the live registry;
- relation totals disagree with selected case totals;
- copied/total case or suite counts are inconsistent;
- generated Markdown differs from the checked-in document.

The generator sorts rows by donor skill and sorts script/tool names
lexicographically.

## Refresh lifecycle

1. Resolve upstream `main` to a concrete target commit.
2. Prepare a candidate full corpus without changing accepted files.
3. Report added, removed, and changed skills, scripts, cases, suites, and shared
   assets.
4. Carry existing relations only when selected case content digests are
   unchanged.
5. Require review decisions for changed selected cases.
6. Copy unsupported cases and suites as `stored only`; do not fabricate
   relations.
7. Update `parityBaselineCommit` only for scopes whose executable parity bytes
   changed.
8. Update the general donor baseline only through the existing provenance
   review, because accepting all scripts must not silently accept unrelated
   donor prose.
9. Apply corpus, manifest, reviewed relations, provenance review, and generated
   public table atomically.
10. Run offline generator, provenance, parity, package-boundary, and full CI
    checks.

If upstream moves after preparation, apply aborts and preparation must be
repeated.

## Initial borrowing assessment rules

The analytical report prioritizes:

1. adopted skills with large unimported test suites, because they can improve
   confidence without adding product surface;
2. missing inverse operations such as `form-decompile`, `meta-decompile`, and
   `skd-decompile`, because they close round-trip workflows;
3. donor DB/EPF/ERF scripts whose scenarios may strengthen
   `unica.runtime.execute` without duplicating public commands;
4. high-change mutation skills such as `cfe-patch-method`, `meta-edit`, and
   `skd-edit`;
5. `web-test` as a separate product decision, not an automatic Unica skill
   import.

Each recommendation distinguishes borrowing behavior/tests from copying the
donor implementation.

## Failure handling

- Missing or malformed donor paths abort preparation.
- Symlinks, path traversal, duplicate normalized paths, and untracked generated
  files are rejected.
- Unknown test layouts are reported as unclassified and block a supposedly
  complete corpus refresh.
- Unknown tool names or missing Unica skills block table generation.
- Unsupported runtime dependencies do not block storing a reviewed corpus, but
  they block marking it executable.
- A stale generated table fails CI with the exact regeneration command.
- A changed selected case cannot reuse an old observation fingerprint.

## Tests

Add focused tests for:

1. complete inventory generation without a hard-coded skill count; the current
   accepted corpus is separately expected to contain 72 skills;
2. exact script path listing, including nested script directories;
3. direct/supporting/related tool validation;
4. `skd-*` to `dcs-*` aliases;
5. `form-compile-from-object` ownership;
6. JSON and separately structured suite counts;
7. stored-only cases not requiring relation records;
8. selected cases requiring exactly one reviewed relation;
9. stale Markdown detection;
10. unknown donor skill, tool, script, or test layout failures;
11. full refresh carry-forward for unchanged relations;
12. package tests proving corpus and provenance reports remain test-only.

Run the repository's complete Python CI suite after focused tests. Run Rust
tests when tool-registry extraction or Rust contracts change.

## Acceptance criteria

- Every accepted donor skill has exactly one public table row.
- All donor scripts at the accepted target commit are copied byte-for-byte and
  hashed.
- All recognized donor test suites are copied or explicitly classified by a
  reviewed exclusion.
- The table distinguishes adoption, related tooling, corpus storage, and
  executable parity.
- Existing 152 relations remain unchanged when their content digests are
  unchanged.
- No unsupported skill is mislabeled `donor_ahead`.
- The public Markdown is reproducible offline and guarded by CI.
- The dated analytical report explains upstream changes and recommends
  borrowing priorities with evidence.
- The marketplace package does not contain donor corpus or maintainer
  provenance files.
