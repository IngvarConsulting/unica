# Unica v0.7.6 Migration Bridge and v0.8.0 Legacy Cleanup Design

> Superseded at the release barrier: the `v0.7.6` manual regression exposed an
> overlapping-cache cleanup defect. The corrected bridge contract is
> [`2026-07-19-v077-migration-bridge-v080-cleanup-amendment.md`](2026-07-19-v077-migration-bridge-v080-cleanup-amendment.md).

**Status:** Approved direction from the maintainer request dated 2026-07-19.

## Purpose

Unica `v0.7.6` is the final release that understands historical local and
duplicated installation layouts. It is an immutable migration bridge, not the
start of an indefinitely growing compatibility layer.

After a legacy installation has been normalized by `v0.7.6`, the ordinary
marketplace update path can move it to `v0.8.0` and later releases. Unica
`v0.8.0` removes the legacy migration implementation from the current package.

This policy resolves two competing requirements:

1. users of old Unica releases must be able to migrate without waiting for
   `v1.0.0`;
2. every future Unica release must not carry discovery, backup, rollback, and
   compatibility code for obsolete installation layouts.

## Definitions

### Legacy installation

An installation is legacy if any of these conditions is present:

- a local marketplace named `unica` or `unica-local`;
- the legacy selector `unica@unica-local`;
- duplicated `unica@unica` and `unica@unica-local` registrations;
- historical marketplace or plugin cache paths owned by Unica;
- a package layout that is not the public `IngvarConsulting/unica-marketplace`
  catalog and the canonical `unica@unica` plugin.

The installed version number does not override this classification. A legacy
layout reporting `0.7.x` must still use the `v0.7.6` bridge.

### Canonical 0.7.x installation

A canonical installation has:

- marketplace name `unica`;
- marketplace source `IngvarConsulting/unica-marketplace`;
- exactly one installed and enabled plugin selector, `unica@unica`;
- no remaining legacy registrations or managed legacy paths.

The phrase "technical 0.7.x version" means an unpublished or prerelease
`0.7.x` build already installed in this canonical layout. It does not include
local or duplicated layouts merely because their version starts with `0.7`.

## Supported transition policy

| Starting state | Required transition to `v0.7.6` | Transition from canonical `0.7.x` to `v0.8.0` |
| --- | --- | --- |
| Fresh installation | Install from the public marketplace | Ordinary marketplace update |
| Canonical `v0.7.5` | Ordinary marketplace update to `v0.7.6` | Ordinary marketplace update |
| Canonical `v0.7.6` | Already normalized | Ordinary marketplace update |
| Canonical technical `0.7.x` | Ordinary marketplace update to `v0.7.6` when normalization is not already proven | Ordinary marketplace update |
| Any local, duplicated, or otherwise legacy installation | Run the immutable `v0.7.6` migration installer | Only after the bridge reports a canonical `v0.7.6` installation |

The public documentation replaces the existing prose section "Переход со
старой установки и откат" with a Russian-language version of this table. The
legacy row links directly to the immutable release assets:

- `https://github.com/IngvarConsulting/unica/releases/download/v0.7.6/install-unica.sh`
- `https://github.com/IngvarConsulting/unica/releases/download/v0.7.6/install-unica.ps1`

The documentation must not link legacy users to `main`, a mutable marketplace
branch, or the installer from a later release.

## v0.7.6 bridge behavior

The `v0.7.6` installers own the entire migration transaction:

1. clone the public marketplace and resolve its immutable `v0.7.6` catalog
   entry;
2. run native `migrate-preflight` without mutating the Codex profile;
3. capture exact configuration and all owned paths that may be changed;
4. remove known legacy plugin and marketplace registrations;
5. add or upgrade the canonical marketplace;
6. install exactly one `unica@unica` plugin;
7. restore canonical user settings that Codex rewrites during plugin commands;
8. verify the package, runtime, MCP tools, and fresh prompt-visible skills;
9. remove only the legacy paths identified during preflight;
10. prove that the resulting Codex discovery is canonical.

Any failure after backup creation performs rollback to the preflight state and
reports the retained backup directory. Re-running the bridge against an already
canonical `v0.7.6` installation is idempotent and performs verification without
creating a needless migration backup.

Issue #90 is one historical fixture within this bridge contract. Its two path
variants are tested manually as part of the full migration regression, but are
not permanent automatic release checks.

## v0.8.0 package boundary

Unica `v0.8.0` accepts ordinary updates only from canonical `v0.7.5`, canonical
`v0.7.6`, and canonical technical `0.7.x` installations. The current package
does not attempt to repair legacy layouts.

The issue #135 cleanup pull request removes from the `v0.8.0` source package:

- the `migrate` and `migrate-preflight` bootstrap commands;
- the migration engine, legacy discovery classification, backup, rollback, and
  managed-path cleanup implementation;
- dependencies used only by migration;
- legacy migration fixtures, tests, CI jobs, and installer shims;
- documentation that presents legacy migration as a capability of the current
  release.

The cleanup retains:

- bootstrap `run` and `verify` behavior;
- normal fresh installation through the public marketplace;
- ordinary canonical marketplace upgrade coverage;
- the `v0.7.6` documentation table and immutable release-asset links as the
  supported entry point for legacy users.

`v0.8.0` does not add a second legacy detector merely to print a refusal. Such a
detector would preserve the legacy classification code under another name and
contradict the cleanup objective. The documented bridge is the compatibility
boundary.

## Regression ownership in unica-marketplace PR #9

The marketplace repository owns end-to-end installation and promotion policy.
The source repository retains focused unit and package-contract tests.

Automatic promotion checks are intentionally bounded:

- validate the staged manifest and immutable source tag;
- fresh-install the staged plugin on macOS, Linux, and Windows;
- upgrade the immediately previous stable canonical release on all three
  systems;
- publish one stable aggregate required check.

Full migration regression is manual only. The workflow must be dispatchable
against the immutable `v0.7.6` marketplace ref and must cover:

- the supported historical legacy inventory;
- representative historical Codex CLI contracts;
- both issue #90 path variants;
- rollback after an injected Codex failure;
- exact settings preservation and final canonical discovery.

There is no weekly schedule and no `0.9.x`-to-`1.0.0` migration receipt or
barrier. `v0.7.6` is the stable bridge selected by this policy.

## Release and delivery sequence

1. Merge marketplace PR #9 with the bounded automatic policy and manual bridge
   regression.
2. Update source PR #136 to version `0.7.6`, publish the transition table and
   release notes, and preserve its fixes for both issue #90 paths.
3. Merge PR #136 after local and hosted verification.
4. Create and push the signed `v0.7.6` tag.
5. Verify the release workflow and the actual published installers and runtime
   archives.
6. Stage and promote the exact `v0.7.6` source tag through the marketplace
   promotion gate.
7. Dispatch the full manual migration regression against the promoted
   `v0.7.6` bridge and retain the workflow URL as evidence.
8. Close issue #90 only after that regression succeeds.
9. Rewrite issue #135 to this bridge-and-cleanup policy and open a separate
   `v0.8.0` cleanup pull request linked to it.

Issue #135 remains open while its cleanup PR is unmerged. A prepared pull
request is evidence of implementation progress, not evidence that cleanup is
already released.

## Verification criteria

### Source PR #136 and release

- all version surfaces agree on `0.7.6`;
- source tests cover both legacy path identities and settings preservation;
- package and release workflows are green;
- the signed tag resolves to the merged commit;
- release assets contain the two immutable installer scripts and expected
  platform runtime archives;
- the README transition table links to those published scripts.

### Marketplace PR #9

- policy tests reject schedules and obsolete barrier logic;
- automatic jobs cover only fresh installation and previous-stable canonical
  upgrade on the supported systems;
- the full historical matrix and issue #90 fixture are manual inputs;
- source tag commits and downloaded assets are SHA-256 verified;
- the aggregate promotion check is stable and required.

### v0.8.0 cleanup PR

- tests first demonstrate that migration commands and legacy policy surfaces
  are absent;
- current bootstrap build and runtime verification remain green;
- repository search finds no executable legacy migration implementation outside
  historical design and release documents;
- the PR body links issue #135 and states that old installations must first run
  the frozen `v0.7.6` installer.

## Rejected alternatives

### Carry legacy migration until v1.0.0

Rejected because every `0.8.x` and `0.9.x` package would retain a growing,
high-risk compatibility surface even though a stable bridge can be published
now.

### Run the full historical matrix automatically or weekly

Rejected because the matrix is expensive, historical states do not change, and
the scenario is a release/manual confidence check rather than a useful signal
on every commit.

### Treat every reported 0.7.x version as directly upgradable

Rejected because issue #90 is fundamentally about installation identity and
paths, not the semantic version string. Direct upgrade eligibility must be
based on canonical state.
