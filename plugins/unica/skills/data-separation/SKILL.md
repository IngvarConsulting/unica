---
name: data-separation
description: "Разделение данных 1С. Используй когда нужно проверить tenant-boundaries, разделители, безопасные запросы, RLS/права, фоновые задания, обмены или интеграции в разделенной базе."
---

# Data Separation

## MCP routing

- Preferred path: use MCP `unica` tools `unica.project.map`, `unica.code.search`, `unica.meta.info`, `unica.role.info`, `unica.code.diagnostics`, `unica.standards.search`, `unica.standards.explain`, and `unica.runtime.execute`.
- Use `unica.dcs.info` when reports/DCS queries may bypass tenant filters.
- Do not call internal metadata, analyzer, standards, runtime, or package adapters directly. They are hidden behind MCP `unica`.

## References

- Read `references/platform/platform-mechanics.md` for tenant-boundaries, rights, temporary storage, background jobs, and exchange behavior.
- Read `references/platform/db-performance.md` when separated data changes query plans, indexes, locks, or virtual table filters.
- Read `references/platform/runtime-diagnostics.md` when the issue appears only in ЖР/ТЖ or runtime traces.

## Workflow

1. Identify separation model: separator values, tenant ownership, user/session context, rights/RLS, privileged code, and external ids.
2. Inspect metadata and roles with `unica.meta.info` and `unica.role.info`; find risky code with `unica.code.search`.
3. Trace tenant value through reads, writes, reports, background jobs, exchange messages, file batches, temp storage, and integration calls.
4. Review queries for missing tenant filters, unsafe privileged mode, broad virtual tables, and joins that cross boundaries.
5. Verify with syntax/tests through `unica.runtime.execute`; include at least two tenant contexts when testing behavior.

## Red flags

- Code writes objects without setting separator attributes.
- Background job runs under a broad user context and processes all tenants.
- Exchange or integration payload omits tenant/external owner id.
- Report/DCS query uses privileged access without a documented reason.
- Temporary storage or files are shared across tenant/session boundaries.

## Contract gaps

If public MCP `unica` cannot expose separation metadata, role details, or runtime context required for the task, report a Unica MCP contract gap.
