# Shirokov Skill Upstream Audit - 2026-06-22

## Scope

This audit covers `cc-1c-skills` by Nikolay Shirokov as the operation-parity donor
for packaged Unica skills. The live donor head after fetch is
`3d36c2026916d2ae8915f0aca0836d55e1ccaabe`.

## Correction

The upstream checker previously resolved `main` before `origin/main`. Because the
cached donor checkout had a stale local `main` at
`ae8241237753850307d94b10df93e5293e29dc74`, the report undercounted current drift.
The correct current drift from the historical baseline is:

- 564 donor commits since `f3466e19fdc37954c030e48daabcc192f0098fe7`;
- 62 watched changed paths remain open after the support-state info batch;
- 31 affected packaged Unica skills remain open.

This is not just a reporting detail. It changes the next backlog: the fresh donor
work is mostly support-state and support-guard behavior, not another form/SKD DSL
batch.

## Already Adapted

- `web-test`: regression runner and packaging-boundary changes are ported through
  Unica-specific web-test guidance and scripts up to `ae8241237753850307d94b10df93e5293e29dc74`.
- `skd-*`: DSL and script behavior through the previous donor window are ported
  through typed `unica.skd.*`.
- `form-*`: form DSL and script behavior through the previous donor window are
  ported through typed `unica.form.*`.
- `meta-info`: donor reference type/object/list presentation output through
  `ae8241237753850307d94b10df93e5293e29dc74` is ported through `unica.meta.info`.
- `meta-compile`: donor `ChoiceHistoryOnInput` handling through
  `ae8241237753850307d94b10df93e5293e29dc74` is ported through `unica.meta.compile`.
- Support-state reporting from `Ext/ParentConfigurations.bin` is ported for
  read-only info commands: `cf-info`, `meta-info`, `form-info`, `skd-info`,
  `mxl-info`, `role-info`, and `subsystem-info`.

## Not Yet Adapted

- Support-guard before mutating vendor-supported objects:
  `cf-edit`, `meta-compile`, `meta-edit`, `meta-remove`, `form-add`,
  `form-compile`, `form-edit`, `skd-compile`, `skd-edit`, `mxl-compile`,
  `role-compile`, `subsystem-compile`, `subsystem-edit`, `template-add`,
  `template-remove`, `help-add`, `interface-edit`.
- New donor `support-edit` capability. Unica should not expose donor raw scripts;
  if accepted, this needs a typed Unica boundary, likely `unica.support.edit`
  with a small `support-edit` skill or a clearly routed `release-support` flow.
- New donor `db-dump-dt` and `db-load-dt` skills. Unica currently has no packaged
  DB dump/load skill group, so this needs a product decision: extend
  `v8-runner`/runtime workflows or add typed DB tools.
- 1cv8 executable path resolver fixes in donor DB/EPF/web-publish scripts. Most
  of those donor skills are not packaged in Unica, but the behavior should be
  checked against `v8-runner` and package launchers before ignoring it.

## Recommended Next Batches

1. Commit the support-state info batch: shared parser, hidden legacy parity for
   `form-info`/`skd-info`, skill docs, and updated provenance.
2. Add support-guard for mutating tools, but only after deciding the Unica escape
   hatch: `deny`, `warn`, `off`, and whether to implement `unica.support.edit`.
3. Review DT dump/load as a separate runtime/database product batch.
4. Review path resolver fixes against current Unica runtime tooling; do not port
   donor PowerShell/Python launch guidance directly into prompt-visible skills.

## Main Contradiction To Avoid

Do not mark mutating form/SKD/meta/role/subsystem tools as fully current just
because read-only support-state reporting is ported. The next unresolved donor
concern is support-guard before mutation, not another info-output refresh.
