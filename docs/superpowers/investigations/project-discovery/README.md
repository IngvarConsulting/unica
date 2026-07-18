# Project Discovery and Discovery Receipts Investigation Archive

> Status: WIP handoff archive; non-normative.
>
> Authority: historical evidence only. These files do not override code,
> tests, package metadata, the active spec, or accepted ADRs.
>
> Implementation status: the presence of a design, report, review, or passing
> command transcript is not evidence that the functionality is shipped.

This archive preserves the complete recoverable Project Discovery research set
from the implementation session behind this alternative to
[PR #83](https://github.com/IngvarConsulting/unica/pull/83). It is intended to
make a later continuation possible while this branch remains a Draft/WIP.

## What was recovered

- 69 research, design, review, report, generator, and evidence-package files
  were recovered byte-for-byte from the session journal or regenerated from
  their exact recorded commit ranges.
- `RECOVERY-MANIFEST.sha256` records the exact archived bytes and is the
  integrity gate for future continuation.
- The old `.superpowers/sdd/.gitignore` contained only `*` and was deliberately
  excluded: it is control metadata, not research, and would hide this archive
  from Git.
- References to a recovered `.superpowers/sdd/<name>` resolve to `<name>` in
  this directory. References to artifacts that were planned but never created
  are inventoried below; they do not resolve locally. Absolute `/private/tmp/...`
  paths are historical evidence from the deleted temporary worktree and must
  not be reused as live paths.

The deleted temporary worktree also contained a large uncommitted production
prototype. That code is not included here and must not be reconstructed or
presented as delivered without a fresh implementation and review. The durable
branch contains only the committed foundation described below.

## Current authority and delivery state

The current active design is
[extension-point-discovery.md](../../../../spec/architecture/extension-point-discovery.md),
owned by
[ADR 0008](../../../../spec/decisions/0008-project-discovery-and-discovery-receipts.md).
The staged task sequence remains in the
[historical implementation plan](../../plans/2026-07-17-project-discovery-receipts.md).

The branch currently commits the Task 1-4 foundation:

- accepted architecture and ADR;
- strict typed discovery contract and deterministic identities;
- six evidence-port boundary, evidence graph, validation, materiality, and
  no-op receipt eligibility seam;
- contained project source resolution and content-based source snapshots.

It does not yet deliver Tasks 5-15: production Platform XML/BSL evidence,
concrete mechanism orchestration, the shared mutation resolver, durable receipt
store and leases, discovery guard, observation/replay, public MCP registration,
gold corpus, package/skill rollout, or final delivery proof. In particular, no
public `unica.project.discover` tool is registered by this archive.

## Continuation map

| Area | Start here | Archive status |
| --- | --- | --- |
| Committed evidence graph foundation | `task-3-brief.md`, `task-3-report.md` | Promoted into current code/spec; reports remain historical evidence |
| Committed source capture foundation | `task-4-brief.md`, `task-4-report.md`, `task-4-review-package.md` | Promoted into current code/spec; review package is historical evidence |
| Dynamic registered-Form capture | `task-4-v7-dynamic-material-addendum.md` | Open owner contract; not accepted |
| Platform XML evidence | `task-5b-v7-contract.md` | Open owner contract; not accepted |
| ParentConfigurations evidence | `task-5c-evidence-v2-design.md` | Explicit conditional draft; not implementation-ready |
| Bounded BSL evidence | `task-6-v2-v7-addendum.md`, `task-6-v3-golden-generator-evidence.md` | Open owner contract and reproducibility evidence; not accepted |
| Associations and mechanisms | `task-7-v7-addendum.md` | Open owner contract; not accepted |
| Shared mutation resolver and writer seam | `task-8-v6-design.md` | Open downstream design; prerequisites unresolved |
| Receipts, guard, MCP, corpus, packaging | `roadmap-6-14-audit.md`, then Tasks 9-14 in the historical plan | Research backlog only |
| Full verification and delivery | Task 15 in the historical plan | Not started; impossible until Tasks 5-14 are delivered |

Earlier `task-5a-*`, `task-5b-*`, `task-5c-*`, `task-6-*`, `task-7-*`, and
`task-8-*` versions are preserved as lineage, rejected alternatives, reviews,
and contradiction evidence. They are not independent implementation authority.
The four `review-*.diff` files and `task-4-review-package.md` preserve the exact
captured review inputs for the committed foundation. Their original Git diff
options were not recovered, so the manifest verifies their bytes but the four
diffs cannot be independently regenerated from their filename ranges alone.

## Referenced but not recovered

No absent reference is claimed as recovered. The old `.gitignore` was excluded
deliberately, and the following 16 research paths were referenced but did not
exist as completed artifacts at the handoff point:

- missing acceptance authority: `task-4-7-v7-design-package-acceptance.md`;
- future implementation reports: `task-5-report.md`, `task-5b-report.md`, and
  `task-5c-report.md`;
- future audits and reviews: `task-5c-v2-self-audit.md`,
  `task-5b-v7-independent-review.md`,
  `task-5c-evidence-v2-independent-review.md`, and
  `task-6-v2-v7-independent-review.md`;
- pending Task 5C mutation successors: `task-5c-mutation-v2-addendum.md`,
  `task-5c-mutation-v2-self-audit.md`, and
  `task-5c-mutation-v2-independent-review.md`;
- pending Task 7 successors: `task-7-v6-v7-addendum.md`,
  `task-7-v6-v7-self-audit.md`, and
  `task-7-v6-v7-independent-review.md`;
- future receipt/guard addenda: `task-9-v6-addendum.md` and
  `task-10-v6-addendum.md`.

`task-8-v6-design.md` explicitly calls the latter successor documents pending.
Its `task-7-v6-v7-*` names also drift from the recovered `task-7-v7-*` names;
do not silently equate them without a new owner review.

## Known stop condition

The coordinated Task 4/5B/6/7 owner documents name
`task-4-7-v7-design-package-acceptance.md` as their sole external acceptance
ledger. That ledger was checked at the end of the session and did not exist.
Therefore none of those documents may be called frozen or accepted, even where
an owner-local self-audit reports zero findings. This is the central unresolved
handoff condition, not a documentation omission to paper over.

Some older reports also use words such as “accepted” or “complete” for slices
that lived only in the deleted uncommitted prototype. Those claims are
superseded by the live branch state and must not be used as delivery evidence.

## Integrity and whitespace gates

Some recovered review packages contain historical trailing whitespace. Those
bytes are covered by the recovery manifest and must not be normalized merely to
make an unscoped `git diff --check` pass. Future final verification has two
separate gates:

1. verify all recovered payloads with
   `shasum -a 256 -c RECOVERY-MANIFEST.sha256` from this directory;
2. run `git diff --check <base>...HEAD -- .
   ':(exclude,glob)docs/superpowers/investigations/project-discovery/**'` from
   the repository root for live code and specs, then run `git diff --check
   <base>...HEAD -- docs/superpowers/investigations/project-discovery/README.md
   docs/superpowers/investigations/project-discovery/RECOVERY-MANIFEST.sha256`
   for the two archive control files.

The scoped whitespace command passes for the current handoff. The intentionally
preserved payload is governed by its hashes instead.

## Safe continuation procedure

1. Verify the archive from this directory with
   `shasum -a 256 -c RECOVERY-MANIFEST.sha256`.
2. Re-read current code/tests/package metadata, the active spec, and ADR 0008.
3. Reconcile the four current owner documents and create an explicit reviewed
   acceptance ledger only if their APIs and dependency graph are still valid.
4. Promote accepted conclusions into the active spec/ADR before implementation.
5. Resume from Task 5 with fresh RED tests and commit each independently
   reviewable slice; do not revive the deleted prototype wholesale.
6. Apply the two-part integrity and whitespace gate above; never normalize the
   recovered payload as a cleanup step.
7. Keep the PR in Draft until the public MCP, receipts, guard, corpus, package,
   rollout, and Task 15 delivery gates are all current and green.
