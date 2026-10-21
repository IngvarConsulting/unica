# Metadata Modeling

## When to use

Use this when the user needs to create, inspect, edit, validate, or remove
configuration metadata: configuration root files, catalogs, documents,
registers, constants, enums, common modules, subsystems, command interfaces,
templates, external processors/reports as metadata objects, and related XML.

Do not use this for database build/dump/load or artifact build/export. Those
are `unica.run` operations: `push`, `pull`, `download`, `make`.

Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`dryRunRequired`. Контракт вызова бери оттуда, а не из этого текста;
при `implemented: true` используй опубликованную `argsSchema`; при
`support.state: limited` разрешено только подмножество `support.supportedArgs`.
При `support.state: unavailable` остановись; не выдумывай аргументов при
`argsSchema: null`. Для плановой операции сначала проверь результат `dryRun: true`,
затем исполняй запрос с `dryRun: false`. Preview не фиксирует входы между
вызовами. Не обходи контракт прямым runner-ом.

## Primary path

Before selecting XML metadata tools, inspect the project with
`unica.view {}` and choose the target source-set. Native metadata tools work
with platform XML source-sets (`sourceFormat=platform_xml`). If the selected
source-set is EDT (`sourceFormat=edt`), do not apply platform XML edits directly;
ask for an explicit platform XML target: Unica reads platform XML only, and
Designer/EDT conversion is not on the v0.13 surface.

The workspace itself does not have a single source format. A project can contain
an EDT configuration source-set and a platform XML external processor/report
source-set. The format decision belongs to the selected source-set.

Use the public `unica` server: read a node with `unica.view`, change it with
`unica.apply`, and validate it with `unica.check`. Every node answer lists its
operations in `can`; take the argument contract from there.

- Configuration root properties and child objects: `props.set`,
  `object.create` and `object.remove` (`cf-edit`, `meta-add` skills).
- Object attributes, tabular sections, forms, commands and templates:
  `attribute.*`, `tabularSection.*`, `form.*`, `command.*` and `template.*`
  at the object's address (`meta-edit` skill).
- Subsystems and the command interface: `subsystem.create`, `content.*`,
  `childSubsystem.*` and the `command*.set` operations (`subsystem-edit`,
  `interface-edit` skills).
- A new configuration root has no canonical operation; the `cf-init` skill
  gives the file format and names the contract gap.

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
