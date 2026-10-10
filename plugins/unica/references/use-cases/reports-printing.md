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
  Read the target's `can` section to discover its operations. For a DCS operation, `mxl.set` or `template.add`, explicitly request its argument contract:

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

## Composing and changing a DCS

Find the template through `unica.view`, then read its properties, `DataSet`
and `Setting` branches. Request `can` on the actual target and the contract
of the operation you need. The DCS Template is a writable target for schema
components; an existing dataset is the target for its fields and query;
an existing setting is the target for its selection, filters and structure.
Use names returned by the reader. A variant need not be named `Основной`.

To create a schema, use the owner's `template.add` contract with
`templateType:DataCompositionSchema`. Execute creation before addressing
its new Template. The scaffold has a data source and a setting. Add its
components through separate operations: a dataset, its fields, schema
parameters, calculations, totals, and the variant's settings. Read the
scaffold's source and setting names instead of supplying guessed defaults.

Choose the dataset kind for the data actually supplied to the schema:

- Query uses a named data source and query text. Add its fields explicitly;
  field types derive from the query.
- Object uses a named data source and `objectName`, the runtime object's
  name. Its schema dataset name is a separate name. Describe its fields
  and types explicitly.
- Union combines datasets. Create the Union, then add each member at that
  Union's address. Nested members are addressed by repeating
  `DataSet.<name>` in the logical path.

Links between datasets, calculated expressions and totals are separate
schema components. A calculated expression derives a field; a total
aggregates a field, optionally for a specified group. Adding or removing
these components does not rewrite their references or variant settings.
Plan those changes explicitly in the same batch.

Read a query through `DataSet.<name>.Query`: join `data.items[].text` with
newlines in page order. Query replacement takes literal text, not an
`@file` path. A patch with `once:true` requires exactly one match; zero or
multiple matches fail. An empty replacement deletes the matched text.
Keep the fields needed by the schema in the query result.

Adding a field does not select it for output. Configure selection separately
at the required variant or named group. Field roles use a structured object,
for example `{"dimension":true}`, rather than flags embedded in a string.
Titles and literal values can contain brackets and `@` without becoming
commands. Request the operation's contract for accepted type and role values.
A formatted presentation does not change the field's underlying value.

Schema parameters declare types, defaults, expressions and availability.
Variant data parameters choose values and whether to use them. These are
separate changes. Query references use `&Name`; expressions may use
`ПараметрыДанных.Name`. Renaming a schema parameter changes its declaration;
update the affected queries and expressions explicitly. StandardPeriod
values represent a named period or a Custom period with dates; obtain the
accepted value shape from `can`.

Configure selection, filters, order, conditional appearance and output
parameters through their own operations. Adding a filter does not clear
existing filters. Setting a selected filter or data parameter patches the
supplied properties and keeps unspecified settings. A `clear` operation
clears its whole selected collection; use an individual removal operation
when its contract is available and only one item should be removed.

Read `Setting.<name>.Item` to see grouping structure in document order.
Each row gives `index`, `parentIndex`, `axis`, `kind`, `name` and `groupBy`.
Indices identify rows within that variant, not logical nodes. Names may be
absent or repeated, and rows have no `at`. Use an exact, uniquely named
item as a selector; unknown or ambiguous names fail before publication.
`structure.patch` takes the item's `name` separately from its `groupBy`.

Compose nested structure by adding each named group, table or chart
separately. Select its parent explicitly; a table's row/column or a chart's
point/series axis selects where its grouping belongs. Two `groupBy` fields
form one group by both fields, not two nested groups. `structure.set`
replaces the variant's structure; with `details:true` it adds detail records
under that group. Empty `groupBy` means detail records. Choose a targeted
patch or addition when other structure should be preserved.

Preview the complete `ops` batch. Preview writes no sources; execute only
its successful `data.executionToken`. If the response contains a Task,
observe that Task to terminal state without repeating the mutation. Read
the resulting fields, query and settings, then check the Template. A stale
revision requires a new read and preview; a rejected plan must not publish
partial changes.

The [platform XML reference](../specs/1c-dcs-spec.md) describes namespaces,
types, order and schema concepts. It is format documentation. The accepted
arguments and supported operations come from the target's `can` contract.
Do not supply a whole-schema JSON definition or write XML manually to
bypass a missing operation.

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

## Reading and changing a spreadsheet layout

Find the source-set with `view {}` and the template with name search or its
owner's Template branch. Read the template, its `Area` collection, then a
named area's `Body` and `Parameter` branches. A supplied physical path can
be translated by `resolve`; it is not a selector for `view`.

The canonical Template node exposes the Area branch. Height, default width,
column sets, outside-area contents and merge/drawing counts belong to the
format/internal reader; the current canonical projection does not publish
these as Template properties or extra branches. Area properties describe its kind (`Rows`, `Columns`, `Rectangle`, `Drawing`), boundaries,
column/drawing identity and `contentCount`. `Body` lists nonempty text cells
in reading order with `index`, `text` and `template`; the index is a reading
ordinal, not a row/column coordinate. Empty and parameter-only cells are
not text rows. `Parameter` lists parameters, including substitutions in
text templates. The `[tpl]` display marker denotes a substitution from a text
template; it is not part of the parameter's name. Inspect the Parameter
collection rather than constructing a leaf address from that marker.
Read actual names instead of inferring them from visible
labels. A tabular document's `ПолучитьОбласть` can combine a row and a
column area by their names; this does not make column areas editable.

For exact arguments, request `filter.can.op: "mxl.set"` at the Template or
`template.add` at its owner. Creation must be executed before the new
Template is addressed. An addressed parameter or text-template cell becomes
ordinary text when written. Unaddressed neighbors should be verified after
publication. Width can grow but the operation does not shrink it.

Reading does not establish writability. The current writer cannot preserve
all platform constructs: drawings, non-Rows areas, outside rows, overlapping
or differently ordered areas, identified/multiple column sets and
multilingual text/format collections refuse before writing. Such a refusal
requires editing in the Designer; do not bypass it with manual XML writes.
Support-state rules of the metadata owner also apply.

Full JSON-DSL compilation/decompilation, font/style definitions, rowStyle,
merges, palettes and page properties are not public operations. The DSL
reference preserves their format knowledge and historical conversion
models, including generated font/style names and recognition of uniformly
styled empty cells. Historical generated font names include `default`, `bold`,
`header`, `small`, `italic`; style names describe properties, for example
`bordered-center`, `bold-right`, `border-top`. Uniformly formatted cells with
no parameter or text become `rowStyle` in the internal conversion model
and are omitted from its explicit cells. These are format rules, not
new canonical view fields. It is not a payload accepted by `mxl.set`, nor a JSON
round-trip produced by `view`. An image may inform layout structure, but
cannot supply capabilities missing from the public contract.

Execute only the preview's own `executionToken`; reread Area/Body/Parameter
and run `check` on the Template. Check infers `mxl` from the template kind.
Observe a returned Task to terminal state without repeating the mutation.

## Related references

- `../specs/1c-dcs-spec.md`
- `../specs/1c-spreadsheet-spec.md`
- `../specs/mxl-dsl-spec.md`
- `../specs/1c-epf-spec.md`
- `../specs/1c-erf-spec.md`
