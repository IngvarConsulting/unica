- Date: `2026-08-27`
- Status: `approved`
- Decision: `DEC.2026-08-27.RETAINED-APPLY-REVISION-TRANSIENT-AUTHORITY-SLICE`

# Retained-apply revision transient authority

## Problem

Retained apply keeps the exact preimage of every replacement or removal as an
ignored sibling until revision validation and the later transaction gates have
succeeded. Retained capture correctly charges every enumerated child, including
ignored files, but this makes a valid final tree at the entry limit fail while
its rollback preimage is live. A spare entry or `.unica-apply-*` filter would
also exempt an arbitrary foreign ignored child and defeat the same bound.

Projection had a separate mismatch: it started from the semantic manifest, so
it could not charge already captured ignored entries, newly inserted parent
directories, or the live scanner's parent-depth boundary.

## Approved design

Planning captures retain their successful enumeration count in addition to the
semantic manifest. Both stable planning captures must agree on that count.
Projection starts from the second count and applies the final batch topology:
replacement is neutral, removal subtracts its file, creation adds its file and
each missing parent once across the whole batch. Existing ignored entries stay
charged. Checked arithmetic, the entry limit and the exact live parent-depth
boundary are enforced before publication.

The retained transaction journal issues one sealed
`RetainedApplyRevisionTransients<'journal>` after every source publication and
before revision postvalidation. It borrows the prepared root and the journal's
actual displaced capabilities, indexed by exact retained parent identity. Each
entry binds the exact parent, recovery name and regular-file identity. The type
is neither cloneable nor serializable; its private issuer is the only production
construction path.

Production postvalidation requires this authority. For each retained directory,
the scanner may extend only its local enumeration allowance by the exact number
of journal entries indexed to that parent. Both enumeration passes in both full
captures retain the named child without following links and prove parent, name,
regular-file identity, current named identity and hard-link count one. Only a
proof observed exactly once is omitted from accounting, manifest and retained
identities. Lookup uses the retained filesystem's name comparator; duplicate or
equivalent names fail closed.

An unrelated ignored file, including a matching prefix, has no authority and
continues to consume capacity. Missing, replaced, aliased, reparse, foreign-root
or foreign-parent recoveries fail with typed containment or invariant evidence.
Create-only publication issues an empty authority.

## Lifecycle and rollback

The borrow ends before cache publication, revision installation, journal
mutation, cleanup or rollback. Cancellation and deadline preserve their typed
cause and enter the existing rollback path. Validation or later transaction
failure leaves the journal owning each recovery. Rollback rechecks single-link
identity before restoring a displaced file; an added hard-link alias produces
rollback-incomplete evidence and is never restored as an ordinary preimage.

On success the revision installs before cleanup consumes the journal. A cleanup
diagnostic may leave an ordinary ignored artifact, but no authority survives;
the artifact consumes entry capacity on subsequent admission and after actor
reconstruction. No authority is serialized or reconstructed after restart.

## Compatibility boundary

This slice changes no MCP tool, argument, result, route, revision digest or
record schema, transaction participant, publication order, v0.12 behavior,
package or release contract. Enumeration count is private capture accounting
and is not encoded into `unica-source-sha256-v1`.

The design complements the retained-apply transaction foundation and the actor
revision artifact policy. It establishes one new invariant and extends the
existing projection/capture aggregate witness in code without editing that
immutable active record.

## Rejected alternatives

- A global spare allowance cannot distinguish a journal preimage from a foreign
  ignored entry.
- Prefix, path-string or unowned identity exclusions are forgeable and replayable.
- Cleanup before validation destroys rollback authority for later failures.
- Trusting a projected manifest instead of live capture misses external drift,
  identity substitution and entry-bound violations.
