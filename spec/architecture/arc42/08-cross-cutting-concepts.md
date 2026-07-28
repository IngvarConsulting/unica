# 8. Сквозные концепции

Здесь собраны механизмы, которые действуют не в одном модуле, а во всех
операциях сразу. Нормативная формулировка каждого живёт в
[реестре инвариантов](../invariants.md); эта глава объясняет замысел и называет
код, в котором механизм реализован.

## Single public MCP

The LLM sees exactly one server and never coordinates several MCP caches or
indexes. Every engine Unica uses internally is reached through that one boundary
(INV-MCP-01, INV-MCP-02). This is also the primary token-saving mechanism: the
model holds one tool namespace instead of a set of engine-specific protocols.

## Dry run by default

Mutating operations default to a preview that reports what would change without
writing anything, including without writing cache state (INV-CACHE-04). A skill
passes `dryRun: false` only for a mutation the user explicitly asked for
(INV-SKILL-05). The preview and the applied run take the same code path, so a
preview that succeeds is evidence about the real mutation, not a separate
estimate.

## Domain events and cache impact

An operation does not invalidate caches directly. It emits a typed domain event
(`domain/events.rs`), and the event is mapped to a cache impact
(`domain/cache.rs`) that names what is invalidated and what is refreshed eagerly.
The orchestrator owns that mapping and the workspace state behind it
(INV-CACHE-01, INV-CACHE-02). An adapter that invalidated a cache on its own
would put the same knowledge in two places, which is exactly what this
indirection prevents.

## Internal adapters

Adapters are typed boundaries around engines: bundled CLI tools, the standards
MCP endpoint, the analyzer and index services. They may speak CLI or MCP
internally, but their names, transports, and cache lifecycles never reach the
LLM (INV-SKILL-02), and they reach workspace state through application ports
rather than around them (INV-APP-03).

Python, PowerShell, and Bash operation files are not an adapter class. They exist
only as donor reference models under `tests/fixtures` for parity tests
(INV-SKILL-04, INV-APP-04).

## Workspace-scoped services

Some adapters need warm state that is expensive to rebuild. They run behind
hidden services scoped by workspace root plus source root, owned by `unica` and
tracked in volatile cache state (INV-APP-07). The lifecycle rule is: start
lazily, reuse while live, invalidate on domain events, exit after idle and
maximum-age limits. Control-plane reads must not start a service — see
[главу 6](06-runtime-view.md) за полным сценарием.

## Cancellation

Cancellation is cooperative and carries a typed marker: a cancelled operation
returns an error prefixed `cancelled:` (`domain/cancellation.rs`) and is never
reported as a successful partial result. A cancelled public request gets no
response, and cleanup at a managed process boundary is bounded rather than
immediate (INV-MCP-07).

## Uniform operation result

Every operation reports through one shape (`application/outcome.rs`): success
flag, summary, changes, warnings, errors, artifacts, and optional captured
output. An adapter failure becomes a warning or an error inside that shape
instead of an opaque transport error, so a caller can distinguish "the operation
ran and refused" from "the operation could not run".

## Workspace containment

Every path an operation reads or writes is resolved through the workspace path
policy (`infrastructure/path_policy.rs`), which rejects escapes from the
workspace and from the selected source root. Source-root selection itself is
deterministic and shared by all operations (INV-SOURCE-05), so two tools in one
session cannot disagree about which source set they are editing.

## Support state guard

Objects that belong to a vendor configuration are protected before a native
mutation runs (`infrastructure/support_guard.rs`). The guard is fail-closed: an
unreadable, damaged, or non-regular support marker yields a diagnostic state and
blocks the mutation instead of being treated as "not on support".

## Secret redaction

Captured output passes through a streaming redactor
(`infrastructure/redaction.rs`) that masks values of connection strings,
passwords, tokens, and secrets before they reach the operation result. Redaction
is applied at the boundary rather than at each call site, so a new adapter
inherits it by default.

## Порядок разрешения противоречий

Порядок источников истины нормирован в [`AGENTS.md`](../../../AGENTS.md) и здесь
намеренно не дублируется: у этого правила один владелец (INV-DOC-08). Для
архитектурного слоя из него следует одно дополнение — материалы каталогов
`docs/design/` и `docs/plans/` являются архивными и не участвуют в разрешении
противоречий вообще, даже когда они новее активного документа (INV-DOC-05).
