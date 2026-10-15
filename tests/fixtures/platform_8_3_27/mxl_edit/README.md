# Spreadsheet edits: platform evidence

Raw `Ext/Template.xml` exports from 1C **8.3.27.2074** (`ibcmd`), BOM and CRLF
included. They back the byte tests of the `mxl.set` and `area.add` writer
(issue #1448).

`Template.xml` is the export of a hand-written spreadsheet with header,
parameter, template and merged cells, an outside row with a row format,
compressed empty rows, a row of a second column set and Columns and
Rectangle areas. `Empty.xml` is the empty template the platform writes
(`tests/fixtures/platform_8_3_27/mxl/Template.xml` with BOM and CRLF); its
import and export give the same bytes.

Each `expected-*.xml` is the writer's output for one edit of these inputs.
Every one was imported into a disposable file infobase as a common template
and exported again; the export was byte-identical to the writer's output.

```bash
ibcmd=/opt/1cv8/8.3.27.2074/ibcmd
"$ibcmd" infobase create --db-path=ib --create-database
"$ibcmd" infobase config import --db-path=ib src
"$ibcmd" infobase config export --db-path=ib out
```

`src` is an exported empty configuration with the templates added as
`CommonTemplates/T<n>` and listed in `Configuration.xml`.
