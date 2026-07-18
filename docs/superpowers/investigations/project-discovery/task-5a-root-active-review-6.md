# Task 5A root active review addendum 6

Immutable handoff checklist for exact source ordering and source-free cluster
digests. This note is review evidence only and is outside the tracked diff.

## P0: role-group rank and source identity

- `source_group_rank` is a two-valued role-group rank: Analysis is 0 and every
  Destination is 1. Destination ordering comes from the following canonical
  source-set identity field; it is not encoded as ranks 1, 2, 3, ... .
- Canonical source-set identity encodes the exact `ResolvedSourceSet` identity
  (name, kind, format, relative root, mapping digest) with one shared versioned
  encoder. Source/manifest fingerprint is snapshot state and must not enter the
  source-set identity ordering bytes.
- Reuse/expose the domain source-identity encoder rather than inventing
  incompatible length/tag widths in the limiter.
- Tests must prove destination input permutation canonicalizes identically and
  a fingerprint-only change does not change source-identity order.

## P0: source-free cluster digest

- `provider_fact_digest` is not automatically source-free: the typed CFE whole
  observations embed `SourceScopedArtifact.source_set`.
- Add a dedicated source-free semantic projection/encoder that excludes source
  set, freshness, location and provider context while preserving the complete
  typed payload, polarity, UUID, flavor, membership, subject artifact and role.
- Equal semantic CFE halves in differently named destination sources must have
  equal source-free cluster digests and remain distinct through their outer
  source-scoped group keys.

## Required report evidence

- Task 5A report must name this note and SHA-256 in addition to the first five
  immutable root review notes.
