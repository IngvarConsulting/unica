# ADR-0011: DCS is the canonical data composition domain

- Status: accepted
- Date: 2026-07-21
- Issue: [#158](https://github.com/IngvarConsulting/unica/issues/158)

## Context

Unica exposes its data composition operations as `unica.skd.*`, prompt-visible
skills as `skd-*`, and Rust identifiers as `skd`/`Skd`. `SKD` is a
transliteration of the Russian abbreviation `СКД`; the official English 1C
term is **Data Composition System (DCS)**. The repository already uses DCS in
the reference specification and native diagnostic prose, so the public package
and the implementation contradict their own terminology.

This rename changes a pre-1.0 public MCP and skill contract. Keeping both names
without a removal boundary would create two equal public domains and make the
incorrect term permanent.

The release workflow generates GitHub release notes from merged pull requests
and deliberately rejects repository-owned release-note files. A migration note
therefore belongs in the packaged README and in the generated notes contributed
by the pull request, not under a new `docs/releases` tree.

## Decision

`dcs`/`Dcs`/`DCS` is the only canonical English name for this domain:

| Removed contract | Canonical contract |
| --- | --- |
| `unica.skd.compile` | `unica.dcs.compile` |
| `unica.skd.edit` | `unica.dcs.edit` |
| `unica.skd.info` | `unica.dcs.info` |
| `unica.skd.validate` | `unica.dcs.validate` |
| `skd-compile/edit/info/validate` | `dcs-compile/edit/info/validate` |

The change is atomic and has no `skd` compatibility alias. Unica is pre-1.0,
the issue explicitly requires a single canonical surface, and no repository
contract demonstrates a consumer that requires a temporary bridge. Consumers
must replace `skd` with `dcs` when moving to the release that contains this
change.

Rust modules, operation names, event variants, cache graph identifiers,
diagnostics, active documentation, package metadata, and active test scenario
names use the same canonical term. Operation behavior and input schemas do not
otherwise change.

The platform XML root/type `DataCompositionSchema` remains unchanged. The
existing `SetMainSKD` and `setMainSKD` input spellings also remain unchanged
because they mirror the established template-registration/platform contract;
renaming them would be a separate schema change rather than part of the domain
rename.

Upstream donor paths, immutable reference scripts, harvested BSP fixture paths,
and their manifests may retain `skd` where it identifies external historical
bytes. Active provenance records must describe the Unica side as `dcs` while
keeping the original upstream path names verbatim.

A package-contract CI guard enforces the canonical tool and skill set, rejects
the removed aliases, rejects `DSC`, and limits remaining English `skd` matches
to the explicit XML/platform and external-fixture exceptions above.

## Consequences

- Existing callers must migrate `unica.skd.*` calls to `unica.dcs.*`.
- Prompt-visible skills are discoverable only under `dcs-*` names.
- Cache invalidation and operation behavior remain equivalent after their
  identifier rename.
- Generated release notes receive the breaking-change summary from the pull
  request; the packaged README carries the durable migration table.
- Future public English identifiers cannot reintroduce `skd` or the incorrect
  `dsc` spelling without failing CI.
