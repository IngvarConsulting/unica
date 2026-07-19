# Stable v0.7.8 Migration Bridge

## Status

Approved by the maintainer on 2026-07-20.

## Problem

Marketplace version tags were created from staging commits. Those commits
contained the new plugin files but not the promoted stable catalog. The native
bootstrap also ignored the installer ref and registered the marketplace from
`main`. A dedicated migration tag would hide the first defect without fixing
the release process and would still leave consumer updates pinned.

## Decision

- `v0.7.6` and `v0.7.7` are technical releases; `v0.7.8` is the supported
  stable bridge.
- A marketplace version tag identifies a promotion commit whose plugin files
  and stable catalog both reference the same version.
- Promotion no longer requires a tag on the staging commit. The promotion PR
  publishes its exact head SHA; the signed version tag is created at that SHA
  before the PR is merged.
- The installer defaults to the immutable `v0.7.8` marketplace snapshot and
  passes that ref to both bootstrap migration commands.
- The bootstrap registers the public marketplace through Codex CLI with
  `ref = main`, then temporarily checks out the immutable `v0.7.8` snapshot in
  the Codex-managed marketplace root before installing the plugin.
- Migration verification first runs against `v0.7.8`. The same transaction then
  runs the ordinary `marketplace upgrade`, returning the checkout to the saved
  `main` ref, updating the plugin to the current stable release, and verifying
  that installed runtime again.
- The migration transaction owns rollback for the Codex registration,
  temporary checkout, installed plugin, settings, and legacy paths.

## Consumer boundary

- Versions `0.3.0` through `0.7.4` use the published `v0.7.8` installer.
- Versions `0.7.5` and later use the ordinary marketplace update commands.
- The README exposes only the version command, the two-row action table,
  platform-specific installer commands, update commands, and one rollback
  sentence.

## Verification

Focused source tests prove ref propagation, command order, rollback, and both
issue #90 path identities. The marketplace repository owns the manual
full-history regression against published and promoted `v0.7.8` bytes.
