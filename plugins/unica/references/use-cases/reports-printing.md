# Reports, Printing, DCS, And MXL

## When to use

Use this when the user needs reports, DCS/DCS schemas, tabular document layouts,
print forms, BSP external processing registration, or EPF/ERF build/export.

External processors and reports live in external source-sets. `make` in
`unica.run` builds one `.epf`/`.erf` from a singleton external source-set when
the main configuration source-set is declared; `upload` is unavailable, and
target `push`/`pull` source transfer is unavailable for external sets.

## Primary path

Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`dryRunRequired`. Контракт вызова бери оттуда, а не из этого текста;
используй опубликованную `argsSchema`; при `support.state: limited`
разрешено только подмножество `support.supportedArgs`. Недоступной операции
в словаре нет. Для плановой операции сначала проверь результат
`dryRun: true`, затем исполняй запрос с `dryRun: false`. Preview
не фиксирует входы между вызовами. Не обходи контракт прямым runner-ом.

- Read schemas and layouts through `unica.view`, change them through
  `unica.apply`, and validate the template through `unica.check`.
  Every node answer lists its operations in `can`. For a DCS operation, `area.add`, `mxl.set` or `template.add`, read its argument contract at the operation's own address `<node>.Can.<op>`:

  ```json
  {"at":"cf:Report.F05Report.Template.F05Schema.DataSet.F05Data.Query.Can.query.set"}
  ```

  This is a `unica.view` argument example over the acceptance fixture;
  choose actual source-set, template and dataset names from your workspace.
  The dictionary lists only operations that work at the node; an unlisted
  or unknown operation is refused. Use the returned `props.contract.argsSchema`,
  target, effects and example arguments; this reference does not duplicate
  the operation catalogue.
- Add or remove metadata templates using the owning node's `can` dictionary.
- `epf-init` and `erf-init` for make-ready artifact scaffolds inside external
  source-sets, with an optional managed form. No canonical operation creates
  an external descriptor; this is a contract gap. The skills give the file
  formats to write, then admit the source-set with `unica.check {}`. They do
  not synthesize `Configuration.xml` or a platform-generated CDFI sidecar.
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

Find the template through `unica.view`, then read its properties and its
`DataSource`, `DataSet`, `CalculatedField`, `Total`, `Parameter` and `Setting`
branches. Request `can` on the actual target and the contract
of the operation you need. The DCS Template is a writable target for schema
components; an existing dataset is the target for adding fields;
an existing field `DataSet.<name>.Field.<dataPath>` is the target for its
mapping, title and role; an existing parameter `Parameter.<name>` is the
target for its properties and rename; an existing source
`DataSource.<name>`, calculated field `CalculatedField.<dataPath>` and total
`Total.<dataPath>` are the targets for their changes and removal; an
existing setting is the target for its presentation, selection, filters
and structure, and for its removal.
Creation stays at the parent: `dataSource.add`, `calculatedField.add`,
`total.add`, `field.add`, `parameter.add` and `variant.add` name the new
node in `items`. A name argument that duplicated the address
(`values.name`, `values.field`) is refused, and `next` names the node's
address. A field with a composite `dataPath` escapes the dot in its address:
`DataSet.<name>.Field.Owner\.Code`.
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

A field is addressed by its `dataPath`: `DataSet.<name>.Field.<dataPath>`.
A dot inside a composite `dataPath` is escaped in the address as `\.`, for
example `Field.Контрагент\.ИНН` (`"Контрагент\\.ИНН"` in JSON). Take the
address from the `items[].at` of the dataset's `Field` branch.

Links between datasets, calculated fields and totals are separate
schema components. Links stay rows of the Template's `Item` branch and are
selected by their endpoints and expressions. A calculated expression derives a field; a total
aggregates a field, optionally for a specified group. Adding or removing
these components does not rewrite their references or variant settings.
Plan those changes explicitly in the same batch.

Read a query through `DataSet.<name>.Query`: join `data.items[].text` with
newlines in page order. `query.set` and `query.patch` are called at the
same `DataSet.<name>.Query` address; a call at the dataset itself, the old
`Template.<name>.Query.<dataset>` address and a `dataSet` argument are
refused, and `next` names the query's address. Query
replacement takes literal text, not an `@file` path. A patch with
`once:true` requires exactly one match; zero or multiple matches fail. An empty replacement deletes the matched text.
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
parameters through their own operations. Call them at the settings
container they change: the variant `Setting.<name>` or a named structure
item `Setting.<name>.Item.<item>[.Item.<child>…]`. A `group` argument is
refused, and `next` names the item's address. Data parameters belong to the
variant only. Rows inside a collection keep their keys: a filter is selected
by its field, selection, order and appearance items by the index read from
`Setting.<name>.Property`. Adding a filter does not clear
existing filters. Setting a selected filter or data parameter patches the
supplied properties and keeps unspecified settings. A `clear` operation
clears its whole selected collection; use an individual removal operation
when its contract is available and only one item should be removed.

Query fields derive their types from the query: `field.add` and `field.set`
refuse an explicit `type` on them. An Auto order item refuses `direction`;
Field order defaults to Asc. A `valueType` modifier needs its value or value
list; an explicitly empty parameter list is valid. Removing a dataset link
takes its selector and refuses properties used to patch that link.

Read `Setting.<name>.Item` to see the grouping structure in document order.
A named item that is unique among its siblings is a node with `at`
(`Setting.<name>.Item.<item>`), `kind`, `axis` and `groupBy`, and its own
`Item` branch lists its children. An unnamed item has no address: it is a
row with `index`, `parentIndex`, `axis`, `kind` and `groupBy`, and the named
items under it are addressed from the nearest named ancestor. Items with a
repeated name on one level stay rows and cannot be targeted until renamed.
An item name is written as stored, spaces and brackets included
(`Item.По периодам`); a dot inside it is escaped as `\.`.
`structure.patch` and `structure.remove` are called at the item's address;
`structure.set` replaces the whole structure at `Setting.<name>`. A `name`,
`parent` or `variant` argument is refused, and `next` names the item's
address; a name repeated in several branches leads to the `Item` branch and
to each matching item.

Compose nested structure by adding each named group, table or chart
separately with `structure.add` at its parent's address: the variant or an
existing item. Execute or batch the parent first; a table's row/column or a
chart's point/series axis selects where its grouping belongs. A new name must
be unique in the variant. Two `groupBy` fields
form one group by both fields, not two nested groups. `structure.set`
replaces the variant's structure; with `details:true` it adds detail records
under that group. Empty `groupBy` means detail records. Choose a targeted
patch or addition when other structure should be preserved.

`groupBy` belongs to a group; table and chart additions refuse it. Add their
grouping on the appropriate axis. Collection `index` and `parentIndexes`
accept JSON integers such as `0`, `0.0` and `0e0`; fractions are refused.

Source removal checks references in the batch's final schema. One batch can
remove a source before removing its referring dataset. If a dataset still
refers to the removed source at the end, the whole batch is refused.

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

For exact arguments, read `<Template>.Area.<name>.Can.mxl.set` at an
existing area, `<Template>.Can.area.add` at the Template or
`<owner>.Can.template.add` at its owner. `mxl.set` writes only an existing
area: a missing area is refused with `not_found`, and `next` leads to the
Template, where `area.add` creates new areas with their initial cells. An area
argument at the Template is refused with the area's address in `next`.
Creation must be executed before the new Template is addressed. An addressed parameter or text-template cell becomes
ordinary text when written. Unaddressed neighbors should be verified after
publication. Width can grow but the operation does not shrink it.

The writer changes only the addressed cells, rows and areas of
`Template.xml`; drawings, merges, other areas, rows outside areas, other
column sets, translations and the bytes around the edit stay as they were.
New cells have no own format, so they take the row and column formatting.
`area.add` appends the area after every used row. A written row below an
area grows it only when nothing follows the area; otherwise create another
area with `area.add`. Some addressed nodes still refuse before writing: a
non-Rows area, an area without rows, a cell inside a merge that does not
start there, a cell holding a value or control, and a cell that takes a
parameter or template fill from its row, column, column-set or document
format. Edit these in the Designer; do not
bypass a refusal with manual XML writes. Support-state rules of the metadata
owner also apply.

Fonts, styles, merges, palettes and page properties are not public inputs:
`mxl.set` writes text only. A `null` value is empty text; it does not create
an absent cell. `view` returns a node projection, not a document to send
back. An image may inform layout structure, but cannot
supply capabilities missing from the public contract.

Execute only the preview's own `executionToken`; reread Area/Body/Parameter
and run `check` on the Template. Check infers `mxl` from the template kind.
Observe a returned Task to terminal state without repeating the mutation.

## Related references

- `../specs/1c-dcs-spec.md`
- `../specs/1c-spreadsheet-spec.md`
- `../specs/1c-epf-spec.md`
- `../specs/1c-erf-spec.md`
