# Reports, Printing, DCS, And MXL

## When to use

Use this when the user needs reports, DCS/DCS schemas, tabular document layouts,
print forms, BSP external processing registration, or EPF/ERF build/export.

`cf.import` and `artifact.build` in `unica.run` take `.cf`/`.cfe` only.
External processors and reports live in external source-sets: `source.import`
and `source.export` move their sources, and their publication as `.epf`/`.erf`
is outside the v0.13 surface.

## Primary path

Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
выбирай только операцию с `implemented: true` и не выдумывай аргументов
записи с `argsSchema: null`; превью исполнением не является. Не обходи
контракт прямым runner-ом.

- `unica.dcs.*` for DCS/DCS schema info, compile, edit, and validation.
- `unica.mxl.*` for MXL info, compile, decompile, and validation.
- `unica.template.*` for adding/removing templates on metadata objects.
- `epf-init` and `erf-init` for make-ready artifact scaffolds inside external
  source-sets, with an optional managed form. These skills call
  `unica.epf.init` or `unica.erf.init`
  and do not synthesize `Configuration.xml` or a platform-generated CDFI sidecar.
- `epf-bsp-init` and `epf-bsp-add-command` for BSP registration code.
- `unica.run` moves external source-set sources with `source.import` and
  `source.export`; it does not publish an `.epf`/`.erf` artifact.

Declare the generated directory in `v8project.yaml` as
`EXTERNAL_DATA_PROCESSORS` or `EXTERNAL_REPORTS` under `format: DESIGNER` and
place descriptors directly in that source-set root, and report artifact
publication as a Unica MCP contract gap.
These scaffolds are platform XML and are rejected for EDT external-project
layouts.

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
