# Metadata Modeling

## When to use

Use this when the user needs to create, inspect, edit, validate, or remove
configuration metadata: configuration root files, catalogs, documents,
registers, constants, enums, common modules, subsystems, command interfaces,
templates, external processors/reports as metadata objects, and related XML.

Do not use this for database build/dump/load or artifact build/export. Those are
runtime workflows handled by `v8-runner`.

## Primary path

Before selecting XML metadata tools or planning changes in an existing typical
or supported configuration, run the implicit `extension-point-discovery`
preflight. Its first inspection call is the task-only
`unica.project.discover`; inspect `OperationResult.data.discovery` candidates,
evidence locations, provider outcomes, warnings, missing checks, and analysis
snapshot. Resolve architecture-changing gaps only with the public read-only
tools named by that skill, and stop while a material gap remains unresolved.
The snapshot is analysis evidence, not mutation authorization, a freshness
guarantee, or a mutation receipt.

After the preflight, inspect the project with
`unica.project.map` and choose the target source-set. Native metadata tools work
with platform XML source-sets (`sourceFormat=platform_xml`). If the selected
source-set is EDT (`sourceFormat=edt`), do not apply platform XML edits directly;
use runtime conversion/build workflows or ask for an explicit platform XML
target.

The workspace itself does not have a single source format. A project can contain
an EDT configuration source-set and a platform XML external processor/report
source-set. The format decision belongs to the selected source-set.

Use native MCP tools exposed by the public `unica` server:

- `unica.project.discover` for the mandatory task-only extension-point preflight.
- `unica.cf.*` for `Configuration.xml`, languages, roles, and child-object registration.
- `unica.meta.*` for metadata object info/compile/edit/remove/validate.
- `unica.subsystem.*` and `unica.interface.*` for sections and command interface.
- `unica.template.*` for adding or removing metadata templates.

A platform-generated CDFI sidecar `ConfigDumpInfo.xml` whose root is
`ConfigDumpInfo` is per-infobase runtime state, not metadata source. Do not edit
or generate that sidecar with Unica metadata tools, do not use it as source
format evidence, and keep it out of Git. A legitimate metadata descriptor
(including an external EPF/ERF descriptor) for an object actually named
`ConfigDumpInfo` remains source and belongs in Git.

## Related references

- `references/specs/1c-configuration-spec.md`
- `references/specs/1c-config-objects-spec.md`
- `references/specs/meta-dsl-spec.md`
- `references/specs/1c-subsystem-spec.md`
