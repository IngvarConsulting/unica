# Stable v0.7.8 Migration Bridge Implementation Plan

> **Status:** Historical execution context after completion; live contracts are code, README, package metadata, and tests.

**Goal:** Ship `v0.7.8` as the stable migration bridge without pinning future
marketplace updates.

## Tasks

- [x] Reproduce the mismatch between staging version tags and promoted catalog
  commits for every published marketplace tag.
- [x] Reproduce Codex CLI source replacement rules and verify that a marketplace
  configured for `main` can temporarily use a detached promotion snapshot.
- [x] Add failing contracts for installer ref propagation, bootstrap parsing,
  migration command order, and promotion-before-tag workflow ordering.
- [x] Register the public marketplace at `main`, temporarily check out
  `v0.7.8`, install and verify the frozen bridge, then run the ordinary upgrade
  back to `main` and verify the current stable plugin.
- [x] Change the promotion workflow so the signed version tag targets the
  promotion PR head rather than the earlier staging merge.
- [x] Bump package contracts to `0.7.8`, simplify the README, and publish Russian
  descriptions for `v0.7.6`, `v0.7.7`, and `v0.7.8`.
- [ ] Run all source guardrails and obtain green PR CI.
- [ ] Publish and verify source and marketplace `v0.7.8`, correct historical
  marketplace version tags, delete `migration-v0.7.7`, and run the manual
  regression barrier.
