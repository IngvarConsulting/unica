# Architecture Decision Records

This directory holds the architecture decisions of Unica. A record answers one
question: what did the project commit to, why, and which check keeps that
commitment true.

Rules that follow from a decision live in the invariant registry
[`../architecture/invariants.md`](../architecture/invariants.md). The registry
references a record by ID and does not copy its normative text, so every rule
has exactly one owner and two documents cannot disagree about it.

## Accepted ADRs

- [ADR-0001: Единый публичный MCP `unica`](0001-edinyy-publichnyy-mcp-unica.md)
- [ADR-0002: Транспортно-нейтральный application layer](0002-transportno-neytralnyy-application-layer.md)
- [ADR-0003: Cache и workspace state принадлежат orchestrator](0003-cache-i-workspace-state-prinadlezhat-orchestratoru.md)
- [ADR-0004: Operation scripts are reference-only, not runtime backends](0004-legacy-skill-scripts-are-migration-debt.md)
- [ADR-0005: Skills route только через `unica`](0005-skills-routyatsya-tolko-cherez-unica.md)
- [ADR-0006: Workspace-scoped internal services](0006-workspace-scoped-internal-services.md)
- [ADR-0008: Public marketplace with a thin verified runtime](0008-public-marketplace-thin-runtime.md)
- [ADR-0009: OS-specific code behind infrastructure platform facades](0009-os-specific-code-behind-platform-facade.md)
- [ADR-0010: CI build cache and artifact flow](0010-ci-build-cache-and-artifact-flow.md)
- [ADR-0011: DCS is the canonical data composition domain](0011-canonical-dcs-domain.md)
- [ADR-0012: One plugin directory serves Codex and Claude Code](0012-one-plugin-directory-for-two-hosts.md)
- [ADR-0013: MCP transport is owned by the official Rust SDK](0013-mcp-transport-official-rust-sdk.md)

This index lists every record file in this directory, and every listed record
exists. Both directions are held by `INV-DOC-04`.

## Record Template

Every record accepted from ADR-0014 onward uses this shape. ADR-0001 through
ADR-0013 predate the template, keep their original headings, and are not
retrofitted.

```markdown
# ADR-NNNN: <the decision in one line>

- Status: `accepted`
- Date: `2026-07-27`

## Context

## Decision

## Non-goals

## Consequences

## Verification
```

### Header fields

- `Status` — one of the values in [Status Values](#status-values), in
  backticks.
- `Date` — the date the record was accepted, `YYYY-MM-DD`, in backticks.
- `Updated` — optional, `YYYY-MM-DD`, in backticks. Add it when the normative
  text of an already accepted record changes, and leave `Date` untouched so the
  acceptance date is never rewritten. `Updated` is the only accepted spelling
  for this field.
- `Issue` — optional link to the tracking issue, as in ADR-0011.

No other header field is allowed, and the fields keep the order above.

### Sections

- `Context` — the forces that make a decision necessary: current behavior, the
  cost it imposes, and the constraints that cannot be relaxed. Observed facts
  only, no proposal.
- `Decision` — what the project commits to, written in the present tense as
  numbered normative statements. This is the section other documents cite.
- `Non-goals` — what the decision deliberately leaves open, so that a later
  change in that space is not read as a violation.
- `Consequences` — what the decision costs: work it creates, options it
  closes, and trade-offs accepted knowingly.
- `Verification` — the checks that fail when the decision is broken.

### Verification is a list of checks, not a summary

`Verification` names runnable checks, one per line, each pointing at something
that can fail: a test file or a `cargo test` filter, a guard script under
`scripts/ci/`, a CI job, or a release gate step. Each line says what the check
proves, so a reader can run it and get a verdict.

A line that describes the record instead of a check — "the ADR defines X",
"the ADR covers Y" — is not verification. It restates the document to itself,
can never fail, and is rejected in review. When a decision genuinely has no
automated check yet, write one `manual` item that names what a reviewer
inspects; do not fill the section with self-reference.

## Language

New records are written in English: the decision statements, the non-goals, the
consequences, and the verification list. A single sentence never mixes English
and Russian (`INV-DOC-07`).

ADR-0001 through ADR-0006 carry Russian titles, headings, and header field
names. They are historical records of decisions already taken. They are not
translated, renamed, or reformatted — rewriting them would change what the
project can prove it decided, and gains nothing.

## Status Values

- `accepted`: active decision.
- `superseded`: replaced by a newer record, which the header names.
- `proposed`: not yet active; not a source of truth while it holds this status.

## Numbering And Lifecycle

- Numbers are monotonic: a new record takes the next unused number.
- A number is spent the moment it is assigned and is never reissued, whatever
  became of the record that held it (`INV-DOC-02`).
- A decision that stops being true is not deleted. It takes the status
  `superseded`, names the record that replaces it, and stays in this directory
  so that older commits, tests, and release notes keep resolving.
- A number withdrawn before acceptance leaves no file. It is recorded below
  instead of leaving a dangling index link (`INV-DOC-05`).

### Retired numbers

- `0007` — spent on a record that was withdrawn before it reached `accepted`.
  There is no `0007-*.md` file, the number is never reissued, and the gap
  between ADR-0006 and ADR-0008 is expected rather than a missing document.

## When A Record Is Required

A change to the public surface needs a decision record in the same change set.
The public surface is the set of `unica.*` tools and their contracts, the MCP
server identity, skill routing, the packaging and release contract, the layer
boundaries, and any rule carried by the invariant registry.

The record, the registry entry, and the check move together. Two different
checks hold that rule, and neither covers the other:

- `scripts/ci/check-architecture-sync.py` runs on every pull request and fails a
  change that adds or removes a `unica.*` tool declaration in the Rust registry
  without touching `spec/decisions/`, `spec/acceptance/`, or
  `spec/architecture/`. It looks only at tool declarations, so a change to the
  behaviour of an existing tool is still caught by review, not by the guard.
- `tests/ci/test_architecture_registry.py` fails a registry entry whose decision
  record does not exist, and an index that no longer matches the records on
  disk.

When code changes violate an accepted record, update or supersede that record
in the same change set. Changing the code and leaving the record standing is a
process defect, not a documentation backlog item.
