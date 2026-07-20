# 8. Сквозные концепции

## Single Public MCP

The LLM sees one server and does not coordinate multiple MCP caches or indexes.
This is the primary token and context saving mechanism.

## Dry Run Safety

Mutating tools default to dry-run when they can produce an honest preview.
Skills apply them only for explicit user-requested mutations. A platform
mutation without an honest preview uses a typed immutable-sandbox prepare/apply
boundary and explicit authorization; it never returns a fabricated dry-run.

## Cache Ownership

The orchestrator owns cache state. Adapter calls must report through application
use cases so domain events and cache invalidation cannot be bypassed.

Durable repository-operation state is a separate class. It uses stable task and
operation identities, write-ahead remote-effect stages, advisory leases, atomic
synced records, explicit recovery, and schema migration. It is never inferred
from cache freshness.

A non-overridable per-user coordination locator binds canonical targets to the
registered durable root and unresolved tasks. `UNICA_STATE_DIR` changes cannot
create a second operational history for the same original/repository identity.

## Internal Adapter Pattern

Adapters are typed boundaries around existing engines. They may use CLI or MCP
protocol internally, but their names and cache lifecycle are not exposed to LLM.

Python/PowerShell/Bash operation files are not a runtime adapter class for
developer operations. Donor scripts can be kept only as fixture reference models
for native `unica.*` MCP handlers.

## Workspace-scoped Services

Some internal adapters may run behind hidden workspace services. These services
are owned by `unica`, scoped by workspace and source root, and coordinated
through volatile cache state. They are not public MCP registrations and must not
appear in skills as routing targets.

The lifecycle rule is lazy start, reuse while live, invalidate on domain events,
and natural exit after idle or max-age limits. Cheap read-only tools that do not
need warm analyzer/index state must not start the service.

## Source Of Truth Order

When documents disagree, use this order:

1. current code and tests;
2. package manifests and `.mcp.json`;
3. active `spec/`;
4. README and skill prose;
5. archived or research docs.

## Repository Effect Safety

Designer requests are typed and separate public, path, and secret arguments.
Known secrets are scrubbed before persistence. Repository operations succeed
only after observed postconditions; process exit and localized prose alone are
insufficient. Unknown effects fail closed and require reconciliation.

Lock compensation acts only on acquisitions attributable to the current
operation. A dedicated integration account and one original-infobase lease are
mandatory until stronger ownership discovery is proven. The account has its
own repository-plus-username persistent reservation across original infobases.
Raw force, lock stealing, and implicit merge/reference decisions are outside
automation. The typed repository-update adapter may derive only the documented structural
confirmation for exact approved incoming add/delete changes, with capability
evidence.

## Owned Destructive Paths

External task IDs never become path components. A UUID instance, exclusive
marker/nonce, canonical containment, recursive symlink/reparse rejection,
root/home/Git/worktree exclusions, and same-filesystem quarantine guard every
cleanup. Checks repeat immediately before each destructive boundary.
