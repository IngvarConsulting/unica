# Metadata Modeling

## When to use

Use this when the user needs to create, inspect, edit, validate, or remove
configuration metadata: configuration root files, catalogs, documents,
registers, constants, enums, common modules, subsystems, command interfaces,
templates, external processors/reports as metadata objects, and related XML.

Do not use this for database build/dump/load or artifact publication. Those are
runtime workflows handled by `v8-runner`.

## Primary path

Before selecting XML metadata tools, inspect the project with
`unica.project.map` and choose the target source-set. Native metadata tools work
with platform XML source-sets (`sourceFormat=platform_xml`). If the selected
source-set is EDT (`sourceFormat=edt`), do not apply platform XML edits directly;
use runtime conversion/build workflows or ask for an explicit platform XML
target.

The workspace itself does not have a single source format. A project can contain
an EDT configuration source-set and a platform XML external processor/report
source-set. The format decision belongs to the selected source-set.

Use native MCP tools exposed by the public `unica` server:

- `unica.cf.*` for `Configuration.xml`, `ConfigDumpInfo.xml`, languages, roles, and child-object registration.
- `unica.meta.*` for metadata object info/compile/edit/remove/validate.
- `unica.subsystem.*` and `unica.interface.*` for sections and command interface.
- `unica.template.*` for adding or removing metadata templates.

## Related references

- `references/specs/1c-configuration-spec.md`
- `references/specs/1c-config-objects-spec.md`
- `references/specs/meta-dsl-spec.md`
- `references/specs/1c-subsystem-spec.md`
