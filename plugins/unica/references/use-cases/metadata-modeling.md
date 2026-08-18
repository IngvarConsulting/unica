# Metadata Modeling

## When to use

Use this when the user needs to create, inspect, edit, validate, or remove
configuration metadata: configuration root files, catalogs, documents,
registers, constants, enums, common modules, subsystems, command interfaces,
templates, external processors/reports as metadata objects, and related XML.

Do not use this for database build/dump/load or artifact build/export. Those are
runtime workflows whose typed arguments can currently only be previewed by
`v8-runner` through `unica.runtime.execute`.

По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.

## Primary path

Before selecting XML metadata tools, inspect the project with
`unica.project.map` and choose the target source-set. Native metadata tools work
with platform XML source-sets (`sourceFormat=platform_xml`). If the selected
source-set is EDT (`sourceFormat=edt`), do not apply platform XML edits directly;
preview the intended runtime conversion/build arguments with `dryRun: true`, or
ask for an explicit platform XML target. Preview does not convert or build the
source-set.

The workspace itself does not have a single source format. A project can contain
an EDT configuration source-set and a platform XML external processor/report
source-set. The format decision belongs to the selected source-set.

Use native MCP tools exposed by the public `unica` server:

- `unica.cf.*` for `Configuration.xml`, languages, roles, and child-object registration.
- `unica.meta.*` for typed metadata object info/add/edit/remove operations.
- `unica.subsystem.*` and `unica.interface.*` for sections and command interface.
- `unica.template.*` for adding or removing metadata templates.

A platform-generated CDFI sidecar `ConfigDumpInfo.xml` whose root is
`ConfigDumpInfo` is per-infobase runtime state, not metadata source. Do not edit
or generate that sidecar with Unica metadata tools, do not use it as source
format evidence, and keep it out of Git. A legitimate metadata descriptor
(including an external EPF/ERF descriptor) for an object actually named
`ConfigDumpInfo` remains source and belongs in Git.

## Related references

- `../specs/1c-configuration-spec.md`
- `../specs/1c-config-objects-spec.md`
- `../specs/1c-subsystem-spec.md`
