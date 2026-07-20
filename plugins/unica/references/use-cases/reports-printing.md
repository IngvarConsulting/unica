# Reports, Printing, DCS, And MXL

## When to use

Use this when the user needs reports, DCS/DCS schemas, tabular document layouts,
print forms, BSP external processing registration, or EPF/ERF build/export.

Do not use `operation=load` for `.epf` or `.erf`. External processors and
reports are handled through external source-sets with `build`, `dump`, and
`make`.

## Primary path

- `unica.dcs.*` for DCS/DCS schema info, compile, edit, and validation.
- `unica.mxl.*` for MXL info, compile, decompile, and validation.
- `unica.template.*` for adding/removing templates on metadata objects.
- `epf-init` and `erf-init` for make-ready artifact scaffolds inside external
  source-sets, with an optional managed form. These skills call
  `unica.epf.init` or `unica.erf.init`
  and do not synthesize `Configuration.xml` or a platform-generated CDFI sidecar.
- `epf-bsp-init` and `epf-bsp-add-command` for BSP registration code.
- `v8-runner` with `unica.runtime.execute` for EPF/ERF external source-set build/dump/make.

Declare the generated directory in `v8project.yaml` as
`EXTERNAL_DATA_PROCESSORS` or `EXTERNAL_REPORTS` under `format: DESIGNER` and
place descriptors directly in that source-set root, then use `operation=make`.
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

- `references/specs/1c-dcs-spec.md`
- `references/specs/dcs-dsl-spec.md`
- `references/specs/1c-spreadsheet-spec.md`
- `references/specs/mxl-dsl-spec.md`
- `references/specs/1c-epf-spec.md`
- `references/specs/1c-erf-spec.md`
