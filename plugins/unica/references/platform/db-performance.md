# DB And Query Performance

Use this reference when a slow 1C scenario may depend on query text, DCS/DCS
settings, metadata shape, DBMS behavior, locks, or data volume.

## Cause-First Workflow

- Identify the user/API scenario first, then the exact query or DCS dataset
  behind it.
- Capture context: caller module, loop boundaries, transaction scope, virtual
  table parameters, temporary table chain, managed locks, lock order, and
  expected row count.
- Inspect metadata before rewriting: registers, dimensions, resources,
  реквизиты, tabular sections, periodicity, totals, and object type.
- Compare platform query intent with DBMS evidence: generated SQL, plan, table
  size, index usage, lock waits, deadlocks, temp storage, WAL or transaction log
  pressure, and DB-specific wait events.

## Optimization Rules

- Prefer narrowing virtual table parameters over broad post-filtering.
- Do not add an index blindly. Tie it to a concrete predicate, join, sort, or
  grouping and check write impact.
- Remove query-in-loop only after confirming the loop cardinality and required
  isolation semantics.
- Keep temporary tables minimal: only fields needed by later stages, only rows
  that survive later filters.
- Treat rights filters as part of behavior. Do not remove `РАЗРЕШЕННЫЕ` or
  privileged access boundaries to win a benchmark.
- Keep DBMS-specific findings explicit: PostgreSQL, MS SQL Server, and file
  mode have different bottlenecks and different proof.

## Red Flags

- A report filters by date, organization, or tenant after joining large tables.
- A virtual table is queried without natural period/account/register filters.
- A handler repeats the same query for every row in a selection.
- Long locks coincide with large writes, posting, exchange, or background jobs.
- Temp storage or DB transaction log grows during a scenario that should be
  read-only.

## Lock Diagnostics

- Connect every wait or deadlock to holder, waiter, transaction, module path,
  user/API scenario, and the exact lock order before proposing code changes.
- Treat managed locks as part of behavior. Removing a lock to win a benchmark is
  a data-consistency change and needs an explicit product decision.

## Verification

Verify the smallest behavioral change first: syntax, scenario test, row-count
comparison, timing on representative data, and DBMS evidence after the change.
If public MCP `unica` cannot expose the needed plan/log artifact, record a
Unica MCP contract gap instead of inventing a measurement.
