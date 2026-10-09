# Reports, Printing, DCS, And MXL

## When to use

Use this when the user needs reports, DCS/DCS schemas, tabular document layouts,
print forms, BSP external processing registration, or EPF/ERF build/export.

`make` in `unica.run` takes `.cf`/`.cfe` only, and `upload` is unavailable.
External processors and reports live in external source-sets. Target `push`
and `pull` source transfer is unavailable for them, and their publication as `.epf`/`.erf`
is outside the v0.13 surface.

## Primary path

Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`dryRunRequired`. Контракт вызова бери оттуда, а не из этого текста;
при `implemented: true` используй опубликованную `argsSchema`; при
`support.state: limited` разрешено только подмножество `support.supportedArgs`.
При `support.state: unavailable` остановись; не выдумывай аргументов при
`argsSchema: null`. Для плановой операции сначала проверь результат `dryRun: true`,
затем исполняй запрос с `dryRun: false`. Preview не фиксирует входы между
вызовами. Не обходи контракт прямым runner-ом.

- Read schemas and layouts through `unica.view`, change them through
  `unica.apply`, and validate the template through `unica.check`.
  Read the target's `can` section to discover its operations. For a DCS
  operation, explicitly request its argument contract:

  ```json
  {"at":"cf:Report.F05Report.Template.F05Schema.DataSet.F05Data",
   "filter":{"sections":["can"],"can":{"op":"query.set"}}}
  ```

  This is a `unica.view` argument example over the acceptance fixture;
  choose actual source-set, template and dataset names from your workspace.
  `implemented:false` means a registered operation has no implementation.
  An unknown operation is refused. Use the returned `contract.argsSchema`,
  target, effects and example arguments; this reference does not duplicate
  the operation catalogue.
- Add or remove metadata templates using the owning node's `can` dictionary.
- `epf-init` and `erf-init` for make-ready artifact scaffolds inside external
  source-sets, with an optional managed form. These skills call
  `unica.epf.init` or `unica.erf.init`
  and do not synthesize `Configuration.xml` or a platform-generated CDFI sidecar.
- `epf-bsp-init` and `epf-bsp-add-command` for BSP registration code.
- For source transfer, inspect the `unica.run` dictionary of the current
  source-set. External processor/report transfer and `.epf`/`.erf`
  publication remain unavailable.

Declare the generated directory in `v8project.yaml` as
`EXTERNAL_DATA_PROCESSORS` or `EXTERNAL_REPORTS` under `format: DESIGNER` and
place descriptors directly in that source-set root, and report artifact
publication as a Unica MCP contract gap.
These scaffolds are platform XML and are rejected for EDT external-project
layouts.

## Reading and changing an existing DCS

Open the template, then its `DataSet` and `Setting` collections. A variant
need not be named `Основной`; use the name returned by the reader. Query and
field operations address an existing dataset. Variant settings address an
existing setting. The template itself can be read and checked but is not a
DCS edit target.

Read the complete query through `DataSet.<name>.Query`: join
`data.items[].text` with newlines in page order. Query replacement takes text
itself, not an `@file` path. Preserve the query fields required by the schema.
A patch with `once:true` requires exactly one match; zero and multiple
matches fail. An empty replacement deletes matching text. For the argument
shape and literal-token restrictions, request the operation's contract.

Read `Setting.<name>.Item` to see grouping structure in document order.
Each row gives `index`, `parentIndex`, `axis`, `kind`, `name` and `groupBy`.
Indices identify rows within this variant, not logical nodes. Names may be
absent or repeated, and rows have no `at`. Table row/column and chart
point/series axes remain visible. Patch only a real uniquely named group;
an unknown or ambiguous name fails before any publication.

Replacing structure removes the previous structure. Two fields in
`groupBy` form one group by both fields, not two nested groups.
`details:true` appends detail records under that group. An empty `groupBy`
creates detail records even when `details:false`. Patching a named group
preserves other groups; get its exact argument syntax from the contract.

Adding a field or calculated field can also add it to the variant's selected
fields. Duplicate fields may be skipped; inspect the post-image instead of
treating a successful response as proof of a change. Removing a dataset
field also removes its selection entry. A calculated expression describes
a derived field; a total describes aggregation. Current total arguments do
not expose grouping associations.

Schema parameters are distinct from a variant's data parameters. Query
references use `&Name`; expressions referring to data parameters use
`ПараметрыДанных.Name`. A schema parameter's default value, availability,
and a variant's choice to use it are separate facts. Existing StandardPeriod
setups may derive start/end dates, but the typed operations do not expose
the old autoDates, hidden, value-list or available-values flags. Read existing
parameters before changing them; changing a name does not automatically
rename every query reference.

A filter tests a field against a value. Clearing filters, selection,
sorting or conditional appearance clears the whole corresponding
collection. It is not a substitute for removing one item. Group filters,
user-setting presentation, selection folders and per-group selection from
the old DSL are not promised by the typed argument schema.

For a DataSetObject, `name` identifies the dataset in the schema while
`objectName` identifies the data supplied to the processor at runtime.
Do not turn such a dataset into a query dataset to work around a missing
operation. Field types and role values are validated against the platform
format; read the operation contract for supported values. A formatted
presentation does not change the underlying field value used for drilldown.

Preview the complete `ops` batch first. The plan writes no sources; execute
only its successful `data.executionToken`. If the response contains a Task,
observe that same Task to terminal state without repeating the mutation.
After execution, read the changed fields, query and grouping rows, then
check the template. A stale revision requires a new read and preview;
the rejected plan must not publish partial changes.

## Creating a new DCS: current boundary

`template.add` with `templateType:DataCompositionSchema` creates a scaffold
with a data source and a setting, but no dataset. Apply that plan before
reading its children. The existing setting can be configured; query and
field operations cannot populate an absent dataset. Full JSON-DSL
compilation and adding datasets/links are unavailable on the public
surface. State the gap; do not promise a full report or write XML manually
to bypass it.

The old DSL also describes variant data parameters, sorting insertion,
conditional appearance insertion, output parameters, parameter renaming
and reordering, individual filter/total/calculated-field removal,
group templates and drilldown. These are format concepts, not additional
MCP operations. The format references below preserve that knowledge,
including parameter lists, filter groups, selection folders, table/chart
structure, output templates and bindings. Their JSON is not an input to
`unica.apply`.

This is a fragment to merge into an existing valid `v8project.yaml`; it does
not replace required `workPath`, `builder`, or `infobase.connection`. Preserve
the existing connection and local overrides, and never initialize an existing
project database merely to create a scaffold:

```yaml
format: DESIGNER
source-set:
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: src/external-processors
  - name: external-reports
    type: EXTERNAL_REPORTS
    path: src/external-reports
```

## Related references

- `../specs/1c-dcs-spec.md`
- `../specs/dcs-dsl-spec.md`
- `../specs/1c-spreadsheet-spec.md`
- `../specs/mxl-dsl-spec.md`
- `../specs/1c-epf-spec.md`
- `../specs/1c-erf-spec.md`
