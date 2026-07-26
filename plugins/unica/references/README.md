# Unica References

This directory is organized by 1C development use case.
Use this index when a skill needs background guidance beyond its own `SKILL.md`.

## Use Cases

| Intent | Reference |
| --- | --- |
| Create a workspace, configure `v8project.yaml`, build/dump/load, publish CF/CFE/EPF/ERF | `use-cases/workspace-runtime.md` |
| Create, inspect, edit, validate, or remove metadata objects and configuration roots | `use-cases/metadata-modeling.md` |
| Design or modify managed forms and form modules | `use-cases/forms-ui.md` |
| Build reports, DCS/DCS schemas, MXL layouts, print forms, and external report artifacts | `use-cases/reports-printing.md` |
| Create or inspect extensions, borrow objects, and generate method interceptors | `use-cases/extensions-cfe.md` |
| Create, validate, or audit roles and access rights | `use-cases/rights-access.md` |
| Prepare an autonomous debug contour and test through the web client | `use-cases/autonomous-server-debug.md` |
| Search, review, diagnose, refactor, test, or optimize BSL code | `use-cases/code-quality-review.md` |
| Implement integrations and contract-backed integration changes | `use-cases/integrations.md` |

## Stable Specs

XML formats, DSL contracts, and reusable layout patterns live in
`specs/README.md`.

## Platform And Tooling

- `platform/development-standards.md` — coding, architecture, and form-module standards.
- `platform/metadata-conventions.md` — object naming, synonym, representation, and fill-check conventions.
- `platform/platform-solutions.md` — common platform pitfalls and fix templates.
- `platform/runtime-diagnostics.md` — ЖР/ТЖ, startup, web-client, HTTP, background-job, and process/session diagnostics.
- `platform/db-performance.md` — query, DCS, indexes, locks, virtual tables, and DBMS evidence.
- `platform/integration-contracts.md` — HTTP/SOAP/OData/JSON/XML/file-exchange contracts and error semantics.
- `platform/platform-mechanics.md` — background jobs, temp storage, auth/crypto, data separation, and platform runtime boundaries.
- `tooling/v8project.md` — project configuration contract.
- `tooling/runtime-build.md` — runtime build/dump/load/make details.
- `tooling/internal-package.md` — maintainer-only packaging and tool-wrapper notes.

## Provenance

The previous upstream-shaped folders were intentionally removed. Reference
content is now maintained as Unica guidance; provenance is available from git
history, not from duplicated source trees.
