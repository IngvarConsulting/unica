# Unica References

This directory is organized by 1C development use case.
Use this index when a skill needs background guidance beyond its own `SKILL.md`.

## Use Cases

| Intent | Reference |
| --- | --- |
| Create a workspace, configure `v8project.yaml`, build/dump/load, publish CF/CFE/EPF/ERF | `references/use-cases/workspace-runtime.md` |
| Create, inspect, edit, validate, or remove metadata objects and configuration roots | `references/use-cases/metadata-modeling.md` |
| Design or modify managed forms and form modules | `references/use-cases/forms-ui.md` |
| Build reports, DCS/DCS schemas, MXL layouts, print forms, and external report artifacts | `references/use-cases/reports-printing.md` |
| Create or inspect extensions, borrow objects, and generate method interceptors | `references/use-cases/extensions-cfe.md` |
| Create, validate, or audit roles and access rights | `references/use-cases/rights-access.md` |
| Prepare an autonomous debug contour and test through the web client | `references/use-cases/autonomous-server-debug.md` |
| Search, review, diagnose, refactor, test, or optimize BSL code | `references/use-cases/code-quality-review.md` |
| Implement integrations and contract-backed integration changes | `references/use-cases/integrations.md` |

## Stable Specs

XML formats, DSL contracts, and reusable layout patterns live in
`references/specs/README.md`.

## Platform And Tooling

- `references/platform/development-standards.md` — coding, architecture, and form-module standards.
- `references/platform/platform-solutions.md` — common platform pitfalls and fix templates.
- `references/platform/runtime-diagnostics.md` — ЖР/ТЖ, startup, web-client, HTTP, background-job, and process/session diagnostics.
- `references/platform/db-performance.md` — query, DCS, indexes, locks, virtual tables, and DBMS evidence.
- `references/platform/integration-contracts.md` — HTTP/SOAP/OData/JSON/XML/file-exchange contracts and error semantics.
- `references/platform/platform-mechanics.md` — background jobs, temp storage, auth/crypto, data separation, and platform runtime boundaries.
- `references/tooling/v8project.md` — project configuration contract.
- `references/tooling/runtime-build.md` — runtime build/dump/load/make details.
- `references/tooling/internal-package.md` — maintainer-only packaging and tool-wrapper notes.

## Provenance

The previous upstream-shaped folders were intentionally removed. Reference
content is now maintained as Unica guidance; provenance is available from git
history, not from duplicated source trees.
