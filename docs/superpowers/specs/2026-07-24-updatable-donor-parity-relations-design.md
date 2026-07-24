# Updatable Donor Parity Relations Design

## Status

The design was approved in conversation on 2026-07-24. Written-spec review is
required before implementation planning starts.

## Problem

The current parity suite combines two different contracts under one name:

1. native Unica operations are compared with locally adapted Python reference
   models; and
2. native Unica operations are compared with cases and scripts copied from
   Nikolay Shirokov's `cc-1c-skills` repository.

The first contract is useful for migration equivalence, but it is not an
independent donor oracle after Unica changes the copied Python implementation.
The second contract is a real donor comparison, but most known differences are
accepted only by a broad category such as `snapshot_diff`. A different,
unreviewed file delta can therefore replace the reviewed delta without
invalidating the expectation.

The 1C:Enterprise 8.3.27 / export format 2.20 work demonstrates the problem.
Unica can intentionally:

- support an operation that the donor does not support;
- reject donor behavior that the platform proves invalid;
- emit a platform-canonical representation that differs from donor bytes; or
- add diagnostics while preserving the donor-supported operation.

Literal equality must not block those improvements. At the same time, donor
updates must remain a normal supported workflow rather than freezing one
historical snapshot forever.

## Goals

1. Keep donor-supported behavior as a durable regression signal.
2. Allow evidence-backed Unica extensions and platform corrections without
   waiting for the donor to adopt them.
3. Update donor skills, scripts, fixtures, and cases from upstream through a
   deterministic review workflow.
4. Re-evaluate relations when affected donor content changes.
5. Carry unchanged relations forward mechanically.
6. Detect a changed delta even when its broad mismatch category is unchanged.
7. Preserve the single native `unica.*` MCP runtime boundary and reject script
   fallback.

## Non-goals

- Executing a floating donor `main` branch in CI.
- Automatically accepting a new donor baseline.
- Automatically porting every donor capability into Unica.
- Treating donor output as stronger evidence than an exact 1C platform proof.
- Reintroducing donor Python scripts into the packaged plugin or runtime path.
- Converting every XML family to a semantic comparator in the first change.

## Authorities

Conflicts are resolved in this order:

1. verified 1C platform behavior for a claimed platform contract;
2. Unica code, public tool contracts, and focused tests;
3. the accepted donor snapshot for donor compatibility;
4. architecture specifications and skill prose.

Donor behavior remains authoritative for the compatibility surface it
introduced unless a reviewed relation records why Unica differs.

## Terminology

### Tracking ref

The moving upstream ref, currently `cc-1c-skills` `main`. It is used only to
discover drift and select a candidate refresh target.

### Accepted baseline

A concrete donor commit accepted for one or more skills after review. It is
stable inside normal CI, but it advances through donor-refresh changes.

### Donor snapshot

Exact scripts, cases, and fixtures copied from an accepted baseline. Snapshot
files are never edited to make a Unica feature pass. They change only through
the donor-refresh workflow.

### Unica reference model

A locally adapted Python model used to check migration equivalence with native
Rust. It retains attribution to its donor origin but is explicitly Unica-owned
test code and may evolve with Unica.

### Relation

The reviewed meaning of the comparison between one donor case and the current
Unica result.

## Separation of test layers

### Layer 1: Native runtime boundary

Every native operation remains covered by tests that reject `command` fallback.
This layer does not depend on donor equality.

### Layer 2: Unica migration equivalence

The current `reference_skills` tree becomes
`unica_reference_models`. Tests using it are named as Unica model-equivalence
tests, not donor parity tests. Source attribution remains, but comments and
provenance state that these files are adapted test implementations.

This layer may use exact stdout, stderr, command, and workspace comparisons
because both sides belong to Unica.

### Layer 3: Donor compatibility

The `cc-1c-skills` scripts, cases, and fixtures form the exact donor snapshot.
Each discovered donor case has a relation record. This is the only layer called
donor compatibility or donor parity.

### Layer 4: Platform correctness

Focused unit, corpus, XSD, and real-platform tests prove platform overrides.
A donor relation may cite these tests as evidence, but it may not replace them.

## Relation model

Every donor case uses one of these relations:

| Relation | Meaning | Default gate |
| --- | --- | --- |
| `exact` | Donor and Unica normalized observations are equal. | Blocking |
| `compatible` | Unica preserves reviewed donor outcomes and may add reviewed output. | Blocking |
| `unica_extension` | Unica supports reviewed behavior absent from the donor. | Allowed |
| `platform_override` | Verified platform behavior requires a difference. | Allowed with evidence |
| `donor_ahead` | The donor added relevant behavior not yet present in Unica. | Reported, non-blocking |
| `intentional_divergence` | Product scope intentionally differs for a recorded reason. | Allowed with evidence |

`donor_ahead` does not block accepting a donor refresh. It must record either
`decision: adopt` or `decision: defer`, an owner, a reason, and evidence. A
security regression, loss of an already shared capability, or contradiction of
a verified platform contract is not `donor_ahead`; it is a blocking review
failure.

## Exact observations

A non-`exact` relation stores a normalized observation fingerprint containing:

- donor success;
- Unica success;
- mismatch kind;
- normalized stdout SHA-256;
- normalized stderr SHA-256;
- normalized workspace SHA-256; and
- expected-file presence.

The relation also stores:

- case identifier;
- donor content digest;
- relation;
- reason;
- evidence paths;
- decision and owner when required.

The donor content digest binds the case definition, selected script, setup
steps, fixtures, and relevant skill configuration. Changing any bound input
invalidates the relation even if the accepted baseline commit changes for other
reasons.

The observation fingerprint prevents an arbitrary new `snapshot_diff` from
silently replacing the reviewed `snapshot_diff`.

## Repository structure

### Existing source of provenance

`plugins/unica/provenance/skill-upstreams.json` remains the source of truth for:

- donor repository;
- tracking ref;
- accepted global or per-skill baseline commits;
- watched upstream paths;
- local and contract paths; and
- review decisions.

### New baseline manifest

`tests/fixtures/unica_mcp_script_parity/donor-baseline.json` materializes the
accepted snapshot:

- schema version;
- upstream id and tracking ref;
- one accepted commit per refreshed skill/corpus scope;
- upstream path and local path for every copied file;
- SHA-256 for every copied regular file; and
- aggregate content digest per skill and case.

CI verifies that manifest commits match the corresponding provenance entries.

### New relation registry

`tests/fixtures/unica_mcp_script_parity/donor-relations.json` contains one
record per donor case. It is data, not Python source, so refresh review can
produce deterministic diffs.

### Refresh tooling

`scripts/ci/refresh-cc-1c-parity.py` provides two explicit phases:

1. `prepare` fetches or reuses the existing upstream cache, resolves a concrete
   target commit, stages exact donor bytes, calculates impacted cases, and
   writes a candidate review report without changing accepted files.
2. `apply` requires a reviewed report with no unresolved cases, copies the
   staged snapshot, updates the manifest and accepted provenance commits, and
   refuses a target that no longer matches the reviewed commit.

The script supports selected skills and a complete refresh. Paths must remain
repository-relative, symlinks are rejected, and only files matched by the
selected skill's watched upstream paths may enter the snapshot.

The current provenance index watches donor skill implementation and guidance
paths but does not cover the copied `tests/skills/cases` corpus. Implementation
therefore expands each participating entry's `upstreamPaths` to include its
`.claude/skills/<skill>` implementation and its exact
`tests/skills/cases/<case-scope>` directories. Shared case scopes such as
`form-compile-from-object` are mapped explicitly to their owning Unica tool;
refresh tooling must not infer ownership from a path prefix.

### Review artifact

Each refresh creates
`plugins/unica/provenance/reviews/YYYY-MM-DD-cc-1c-parity-refresh.json` with:

- previous and target commits;
- selected and affected skills;
- changed upstream paths;
- added, removed, changed, and unchanged cases;
- carried and invalidated relations;
- per-case review decisions; and
- the exact target commit accepted by `apply`.

## Refresh lifecycle

1. Run the existing upstream drift checker against `trackingRef`.
2. Select a concrete target commit and affected skills.
3. Prepare a candidate snapshot in `.build`; accepted repository files remain
   unchanged on preparation failure.
4. Calculate donor content digests.
5. Carry a relation forward only when its donor content digest is unchanged.
6. Execute impacted donor cases against current Unica and calculate candidate
   observations.
7. Review each changed or new case:
   - retain or change its relation;
   - remove a deviation if the donor caught up;
   - record `donor_ahead` when the donor moved ahead;
   - port donor behavior in a separate or same change when selected;
   - retain a platform override only while its cited evidence still passes.
8. Apply the reviewed snapshot and update baseline commits, relations,
   provenance, and the refresh artifact together.
9. Run focused parity, provenance, and refresh-contract tests.

An upstream commit change alone never invalidates an unchanged relation. A
bound content change always requires review.

## Normal feature lifecycle

A normal Unica feature may:

- change native behavior;
- update Unica reference models;
- add focused platform evidence; and
- add or change a donor relation with evidence.

It may not change donor snapshot bytes or accepted donor commits. CI reports
such a mixed change as an unreviewed donor refresh unless the matching refresh
artifact is present.

## CI gates

Normal CI is offline and deterministic. It blocks when:

- a donor snapshot file does not match its manifest hash;
- a baseline manifest commit disagrees with provenance;
- a donor case has no relation;
- a changed donor content digest reuses an unreviewed relation;
- an observation differs from the reviewed fingerprint;
- an evidence path is missing;
- an `exact` or `compatible` shared capability regresses;
- a platform override's cited contract test fails;
- a native tool exposes script fallback; or
- donor bytes change without a complete refresh artifact.

CI reports but does not block a reviewed `donor_ahead` relation. Reports list
the owning skill, decision, reason, and current observation.

Network drift checks remain an explicit maintainer action. They do not make
normal CI depend on GitHub availability.

## Migration

Migration proceeds without one large semantic-review backlog:

1. Rename the adapted reference layer and remove donor-oracle wording.
2. Generate baseline hashes for the current exact donor corpus.
3. Convert current exact donor cases to `exact`.
4. Convert every existing expected gap to a relation with its current exact
   observation fingerprint.
5. Mark each migrated non-exact relation as reviewed with a concrete reason and
   existing evidence path; broad category-only expectations are removed.
6. Add semantic comparators incrementally by XML family. Replacing a fingerprint
   with semantic assertions may tighten a relation but may not weaken the
   default gates.

The migration does not claim that all existing differences are desirable. It
makes their current shape observable and prevents them from changing silently.

## Failure handling

- Fetch or ref-resolution failure leaves accepted files untouched.
- An upstream path outside the watched scope aborts preparation.
- Symlink, traversal, duplicate destination, or non-regular-file input aborts
  preparation.
- A changed target commit between prepare and apply aborts apply.
- Missing review decisions keep the candidate in `needs-review`.
- Removed donor cases remain visible in the review artifact and require an
  explicit removal decision.
- A malformed relation registry fails before any parity subprocess starts.

## Tests

The implementation must prove:

1. baseline hashes detect a modified donor script;
2. a moving tracking ref does not affect normal offline CI;
3. an unchanged donor content digest carries its relation across a baseline
   update;
4. a changed script, case, fixture, or setup invalidates the relation;
5. the same broad mismatch kind with different bytes fails;
6. `platform_override` requires an existing evidence path;
7. `donor_ahead` is reported and non-blocking after review;
8. a shared donor capability regression blocks;
9. a donor refresh cannot update bytes without manifest, provenance, relations,
   and review artifact agreement;
10. adapted Unica reference models can change without pretending that the donor
    changed; and
11. all native tools still reject script fallback.

## Acceptance criteria

- Donor scripts and cases can advance to a reviewed concrete upstream commit.
- Only content-affected relations require manual reconsideration.
- No category-only expected-gap allowlist remains.
- Unica 8.3.27 platform corrections pass as evidence-backed relations.
- A donor capability newly absent from Unica is visible as `donor_ahead`
  without blocking the refresh.
- A regression in already shared behavior remains blocking.
- The packaged plugin contains no donor operation scripts.
- Normal CI is deterministic and requires no network.
