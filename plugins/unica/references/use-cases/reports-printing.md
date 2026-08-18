# Reports, Printing, DCS, And MXL

## When to use

Use this when the user needs reports, DCS/DCS schemas, tabular document layouts,
print forms, BSP external processing registration, or EPF/ERF build/export.

Do not use `operation=load` for `.epf` or `.erf`. External processors and
reports are handled through external source-sets with `build`, `dump`, and
`make`.

## Primary path

По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.

- `unica.dcs.*` for DCS/DCS schema info, compile, edit, and validation.
- `unica.mxl.*` for MXL info, compile, decompile, and validation.
- `unica.template.*` for adding/removing templates on metadata objects.
- `epf-init` and `erf-init` for make-ready artifact scaffolds inside external
  source-sets, with an optional managed form. These skills call
  `unica.epf.init` or `unica.erf.init`
  and do not synthesize `Configuration.xml` or a platform-generated CDFI sidecar.
- `epf-bsp-init` and `epf-bsp-add-command` for BSP registration code.
- `v8-runner` with `unica.runtime.execute` only previews EPF/ERF external
  source-set `build`/`dump`/`make` arguments with `dryRun: true`; it does not
  build, dump, or publish an artifact.

Declare the generated directory in `v8project.yaml` as
`EXTERNAL_DATA_PROCESSORS` or `EXTERNAL_REPORTS` under `format: DESIGNER` and
place descriptors directly in that source-set root, then preview
`operation=make` with `dryRun: true` and report artifact publication as
unverified.
These scaffolds are platform XML and are rejected for EDT external-project
layouts. Do not use `operation=load` for `.epf` or `.erf`.

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
