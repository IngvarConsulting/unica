# Судьба записей архитектуры v1

Архитектура v1 заморожена в этом каталоге. Эта таблица — единственный маршрут
из старого нормативного слоя в действующий `arch/`. Каждая запись имеет ровно
одну судьбу:

- `carried` — обязательство сохранено без смены смысла;
- `superseded` — предмет пересмотрен и теперь принадлежит названному преемнику;
- `retired` — отдельного обязательства в архитектуре v2 больше нет. Код и тесты
  при этом остаются более высоким источником истины и могут сохранять поведение.

У `carried` и `superseded` причина равна `—`. У `retired` преемник запрещён,
а причина обязательна: `tool-surface-bound`, `check-removed`,
`behavior-removed: DEC.*` либо, только для ADR и приёмочных документов,
`historical-only`. Страж проверяет, что причина подтверждается старым блоком
правила, разрешимостью его проверок или активным решением той же стороны
`product`/`process`, что и удаляемое поведение v1.
Ячейка преемника содержит только ID v2 в обратных кавычках; несколько ID
разделяются запятой, `<br>` или точной комбинацией `,<br>`.

Покрыто записей: 233. Ledger проверяется `scripts/arch/fate.py`.

## Решения

| Запись v1 | Судьба | Преемник v2 | Причина |
| --- | --- | --- | --- |
| `ADR-0001` | `carried` | `INV.WIRE.ONE-SERVER` | — |
| `ADR-0002` | `carried` | `INV.APP.THIN-TRANSPORT` | — |
| `ADR-0003` | `superseded` | `INV.CACHE.ORCHESTRATOR-OWNED` | — |
| `ADR-0004` | `retired` | — | `historical-only` |
| `ADR-0005` | `retired` | — | `historical-only` |
| `ADR-0006` | `superseded` | `INV.APP.HIDDEN-SERVICES` | — |
| `ADR-0008` | `retired` | — | `historical-only` |
| `ADR-0009` | `carried` | `INV.PLATFORM.OS-BEHIND-FACADE` | — |
| `ADR-0010` | `retired` | — | `historical-only` |
| `ADR-0011` | `retired` | — | `historical-only` |
| `ADR-0012` | `superseded` | `INV.PKG.TWO-HOSTS-ONE-TREE` | — |
| `ADR-0013` | `retired` | — | `historical-only` |
| `ADR-0014` | `carried` | `INV.HOST.KNOWLEDGE-BEHIND-FACADE` | — |
| `ADR-0015` | `retired` | — | `historical-only` |
| `ADR-0016` | `retired` | — | `historical-only` |
| `ADR-0017` | `superseded` | `INV.APP.PROVIDER-NEUTRAL` | — |
| `ADR-0018` | `retired` | — | `historical-only` |
| `ADR-0019` | `retired` | — | `historical-only` |
| `ADR-0020` | `retired` | — | `historical-only` |
| `ADR-0021` | `retired` | — | `historical-only` |
| `ADR-0022` | `superseded` | `INV.SOURCE.SNAPSHOT-BINDING` | — |
| `ADR-0023` | `superseded` | `CTR.WIRE.TOOL-SURFACE` | — |
| `ADR-0024` | `retired` | — | `historical-only` |
| `ADR-0025` | `superseded` | `CTR.WIRE.TOOL-SURFACE` | — |
| `ADR-0026` | `retired` | — | `historical-only` |
| `ADR-0027` | `superseded` | `INV.SOURCE.EXACT-VERSION` | — |
| `ADR-0028` | `retired` | — | `historical-only` |
| `ADR-0029` | `retired` | — | `historical-only` |
| `ADR-0030` | `retired` | — | `historical-only` |
| `ADR-0031` | `superseded` | `INV.SOURCE.EXACT-VERSION` | — |
| `ADR-0032` | `retired` | — | `historical-only` |
| `ADR-0033` | `retired` | — | `historical-only` |
| `ADR-0034` | `retired` | — | `historical-only` |
| `ADR-0035` | `retired` | — | `historical-only` |
| `ADR-0036` | `retired` | — | `historical-only` |
| `ADR-0037` | `retired` | — | `historical-only` |
| `ADR-0038` | `retired` | — | `historical-only` |
| `ADR-0039` | `retired` | — | `historical-only` |
| `ADR-0040` | `superseded` | `INV.APP.CONFIG-SNAPSHOT` | — |
| `ADR-0041` | `retired` | — | `historical-only` |
| `ADR-0042` | `retired` | — | `historical-only` |
| `ADR-0043` | `retired` | — | `historical-only` |
| `ADR-0044` | `carried` | `INV.WIRE.PREVIEW-IS-MUTATION-ONLY` | — |
| `ADR-0045` | `retired` | — | `historical-only` |
| `ADR-0046` | `retired` | — | `historical-only` |
| `ADR-0047` | `retired` | — | `historical-only` |
| `ADR-0048` | `retired` | — | `historical-only` |
| `ADR-0049` | `carried` | `INV.SURFACE.ACCEPTANCE-UNCHANGED` | — |
| `ADR-0050` | `retired` | — | `historical-only` |
| `ADR-0051` | `retired` | — | `historical-only` |
| `ADR-0052` | `carried` | `INV.SURFACE.PUBLISHED-ARGS-ARE-READ` | — |
| `ADR-0053` | `retired` | — | `historical-only` |
| `ADR-0054` | `retired` | — | `historical-only` |
| `ADR-0055` | `retired` | — | `historical-only` |
| `ADR-0056` | `retired` | — | `historical-only` |
| `ADR-0057` | `retired` | — | `historical-only` |
| `ADR-0058` | `retired` | — | `historical-only` |
| `ADR-0059` | `retired` | — | `historical-only` |
| `ADR-0060` | `retired` | — | `historical-only` |
| `ADR-0061` | `retired` | — | `historical-only` |
| `ADR-0062` | `retired` | — | `historical-only` |
| `ADR-0063` | `retired` | — | `historical-only` |
| `ADR-0064` | `retired` | — | `historical-only` |
| `ADR-0065` | `retired` | — | `historical-only` |
| `ADR-0066` | `superseded` | `CTR.WIRE.TOOL-SURFACE` | — |
| `ADR-0067` | `retired` | — | `historical-only` |
| `ADR-0068` | `retired` | — | `historical-only` |
| `ADR-0069` | `retired` | — | `historical-only` |
| `ADR-0070` | `retired` | — | `historical-only` |
| `ADR-0071` | `retired` | — | `historical-only` |
| `ADR-0072` | `retired` | — | `historical-only` |
| `ADR-0073` | `retired` | — | `historical-only` |
| `ADR-0074` | `superseded` | `CTR.WIRE.TOOL-SURFACE` | — |
| `ADR-0075` | `retired` | — | `historical-only` |
| `ADR-0076` | `carried` | `DEC.2026-08-19.CORE-FIRST-ACQUISITION` | — |

## Инварианты

| Запись v1 | Судьба | Преемник v2 | Причина |
| --- | --- | --- | --- |
| `INV-PRODUCT-SINGLE-PLUGIN-TREE` | `superseded` | `INV.PKG.TWO-HOSTS-ONE-TREE` | — |
| `INV-PRODUCT-DEVELOPER-OPERATIONS` | `retired` | — | — |
| `INV-PRODUCT-NO-ENGINE-ROUTING` | `retired` | — | — |
| `INV-PRODUCT-PACKAGE-PARITY` | `retired` | — | — |
| `INV-PRODUCT-TOOL-VERSION-SOURCE` | `retired` | — | — |
| `INV-PRODUCT-DCS-NAMING` | `retired` | — | — |
| `INV-PRODUCT-NO-FORMAT-MIGRATION` | `superseded` | `INV.PRODUCT.FULL-DUMP-PROFILE` | — |
| `INV-MCP-META-SURFACE` | `retired` | — | — |
| `INV-MCP-META-OBSERVATION` | `retired` | — | — |
| `INV-MCP-META-INFO-COVERAGE` | `retired` | — | — |
| `INV-MCP-EVENT-SOURCE` | `retired` | — | — |
| `INV-MCP-EVENT-BINDING` | `retired` | — | — |
| `INV-MCP-META-FINDINGS` | `retired` | — | — |
| `INV-MCP-ROLE-EDIT` | `retired` | — | — |
| `INV-MCP-XDTO-LOGICAL-TARGET` | `retired` | — | — |
| `INV-MCP-NO-ENGINE-SERVERS` | `carried` | `INV.WIRE.ONE-SERVER` | — |
| `INV-MCP-SINGLE-ENTRY` | `carried` | `INV.WIRE.ONE-SERVER` | — |
| `INV-MCP-SERVER-NAME` | `carried` | `INV.WIRE.ONE-SERVER` | — |
| `INV-MCP-NAMESPACE` | `carried` | `INV.SURFACE.NAMESPACE` | — |
| `INV-MCP-DATA-DRIVEN-SCHEMA` | `retired` | — | — |
| `INV-MCP-REACHABLE-ARGS` | `superseded` | `INV.SURFACE.PUBLISHED-ARGS-ARE-READ` | — |
| `INV-MCP-SDK-TRANSPORT` | `retired` | — | — |
| `INV-MCP-VERSION-TIERS` | `retired` | — | — |
| `INV-MCP-DEFERRED-READ` | `retired` | — | — |
| `INV-MCP-BOUNDED-ADMISSION` | `carried` | `INV.WIRE.BOUNDED-ADMISSION` | — |
| `INV-MCP-DELIVERY-STATE` | `carried` | `DEC.2026-08-20.LONG-WORK-ANSWERS-WITH-STATE` | — |
| `INV-MCP-RUNTIME-RECEIPT` | `superseded` | `CTR.WIRE.TOOL-SURFACE` | — |
| `INV-MCP-SURFACE-SYNC` | `superseded` | `CTR.WIRE.TOOL-SURFACE` | — |
| `INV-MCP-TYPED-RESULT` | `superseded` | `CTR.WIRE.TOOL-SURFACE` | — |
| `INV-MCP-DIAGNOSTIC-TARGET` | `retired` | — | — |
| `INV-MCP-PROJECT-READINESS` | `retired` | — | — |
| `INV-MCP-PREVIEW-MUTATION-ONLY` | `carried` | `INV.WIRE.PREVIEW-IS-MUTATION-ONLY` | — |
| `INV-MCP-SOURCE-SURFACE` | `retired` | — | — |
| `INV-MCP-CODE-SEARCH-ROLES` | `retired` | — | — |
| `INV-MCP-DOCUMENTATION-SECTIONS` | `retired` | — | — |
| `INV-MCP-DOCUMENTATION-GET` | `retired` | — | — |
| `INV-APP-DOCUMENTATION-NETWORK-POLICY` | `retired` | — | — |
| `INV-MCP-SEARCH-SEMANTICS` | `retired` | — | — |
| `INV-MCP-OUTLINE-DATA` | `retired` | — | — |
| `INV-SKILL-DECLARED-ROUTING` | `retired` | — | — |
| `INV-SKILL-NO-ADAPTER-TARGETS` | `retired` | — | — |
| `INV-SKILL-NO-SCRIPT-ROUTE` | `retired` | — | — |
| `INV-SKILL-SCRIPTS-AS-FIXTURES` | `retired` | — | — |
| `INV-SKILL-DOCUMENTED-PREVIEW` | `retired` | — | — |
| `INV-SKILL-SOURCE-FALLBACK` | `retired` | — | — |
| `INV-SKILL-EXECUTABLE-EXAMPLES` | `retired` | — | — |
| `INV-SKILL-REACHABLE-REFERENCES` | `retired` | — | — |
| `INV-APP-DISPATCH-OWNERSHIP` | `superseded` | `INV.APP.THIN-TRANSPORT` | — |
| `INV-APP-THIN-TRANSPORT` | `carried` | `INV.APP.THIN-TRANSPORT` | — |
| `INV-APP-NO-ADAPTER-BYPASS` | `carried` | `INV.APP.DEPENDENCY-DIRECTION` | — |
| `INV-APP-NO-SCRIPT-BACKEND` | `retired` | — | — |
| `INV-APP-DEPENDENCY-DIRECTION` | `carried` | `INV.APP.DEPENDENCY-DIRECTION` | — |
| `INV-APP-NO-DIRECT-GIT` | `retired` | — | — |
| `INV-APP-SUPPORT-STATE` | `retired` | — | — |
| `INV-APP-PARTIAL-FALLBACK` | `retired` | — | — |
| `INV-APP-CONFIG-SNAPSHOT` | `carried` | `INV.APP.CONFIG-SNAPSHOT` | — |
| `INV-APP-CODE-PROVIDER-BOUNDARY` | `carried` | `INV.APP.PROVIDER-NEUTRAL` | — |
| `INV-APP-DIAGNOSTIC-PROVIDERS` | `retired` | — | — |
| `INV-APP-DOCUMENTATION-NO-DISK-STATE` | `retired` | — | — |
| `INV-APP-OUTLINE-SOURCE` | `retired` | — | — |
| `INV-APP-LAZY-HIDDEN-SERVICES` | `carried` | `INV.APP.HIDDEN-SERVICES` | — |
| `INV-CACHE-ORCHESTRATOR-OWNED` | `superseded` | `INV.CACHE.ORCHESTRATOR-OWNED` | — |
| `INV-CACHE-REPORTED-EFFECTS` | `superseded` | `INV.CACHE.ORCHESTRATOR-OWNED` | — |
| `INV-CACHE-WORKSPACE-ROOT` | `retired` | — | — |
| `INV-CACHE-PROVIDER-STATE-OUTSIDE-SOURCE` | `carried` | `INV.CACHE.STATE-OUTSIDE-SOURCE` | — |
| `INV-CACHE-GENERATION-CUTOVER` | `retired` | — | — |
| `INV-CACHE-WRITE-FREE-PREVIEW` | `retired` | — | — |
| `INV-CACHE-PERSISTED-STALENESS` | `retired` | — | — |
| `INV-CACHE-WORKTREE-ISOLATION` | `retired` | — | — |
| `INV-CACHE-RLM-REVISION` | `retired` | — | — |
| `INV-CACHE-RUNTIME-ROOT-ORDER` | `retired` | — | — |
| `INV-SOURCE-ROOT-SEPARATION` | `retired` | — | — |
| `INV-SOURCE-PORTABLE-GIT` | `retired` | — | — |
| `INV-SOURCE-PER-SET-FORMAT` | `superseded` | `INV.SOURCE.FORMAT-PER-SET` | — |
| `INV-SOURCE-UNAMBIGUOUS-SET` | `retired` | — | — |
| `INV-SOURCE-MULTI-FORMAT-WORKSPACE` | `retired` | — | — |
| `INV-SOURCE-AUTODETECT-CATALOG` | `retired` | — | — |
| `INV-SOURCE-PLATFORM-XML-ONLY` | `retired` | — | — |
| `INV-SOURCE-SINGLE-RESOLVED-ROOT` | `superseded` | `INV.SOURCE.DEFAULT-SET-SELECTION` | — |
| `INV-SOURCE-LOGICAL-IDENTITY` | `retired` | — | — |
| `INV-SOURCE-SUBSYSTEM-TOPOLOGY` | `retired` | — | — |
| `INV-SOURCE-READER-SELECTOR` | `retired` | — | — |
| `INV-SOURCE-READER-MIGRATION` | `retired` | — | — |
| `INV-SOURCE-WRITE-TARGET-KIND` | `retired` | — | — |
| `INV-SOURCE-SNAPSHOT-BINDING` | `carried` | `INV.SOURCE.SNAPSHOT-BINDING` | — |
| `INV-SOURCE-ROLE-ALLOWLIST` | `superseded` | `INV.SOURCE.SNAPSHOT-BINDING` | — |
| `INV-SOURCE-OBSERVED-EOL` | `superseded` | `INV.SOURCE.OBSERVED-BYTES` | — |
| `INV-SOURCE-TAIL-INSERT` | `retired` | — | — |
| `INV-SOURCE-ATOMIC-PUBLISH` | `superseded` | `INV.SOURCE.ATOMIC-PUBLISH` | — |
| `INV-SOURCE-IDEMPOTENT-REWRITE` | `retired` | — | — |
| `INV-SOURCE-WRITE-CONTAINMENT` | `carried` | `INV.SOURCE.WRITE-CONTAINMENT` | — |
| `INV-SOURCE-WRITABLE-FORMAT` | `superseded` | `INV.SOURCE.EXACT-VERSION` | — |
| `INV-SOURCE-ROOT-POLICIES` | `retired` | — | — |
| `INV-SOURCE-OWNER-VERSION-GATE` | `superseded` | `INV.SOURCE.EXACT-VERSION` | — |
| `INV-SOURCE-EXACT-VERSION-LITERAL` | `retired` | — | — |
| `INV-SOURCE-EXACT-ROOT-QNAME` | `retired` | — | — |
| `INV-SOURCE-BOUND-PREIMAGES` | `retired` | — | — |
| `INV-SOURCE-ROLLBACK-VISIBLE` | `retired` | — | — |
| `INV-PKG-UNTRACKED-BUILD-OUTPUT` | `retired` | — | — |
| `INV-PKG-THIN-PACKAGE` | `superseded` | `INV.PKG.THIN-PACKAGE` | — |
| `INV-PKG-VERIFIED-ATOMIC-INSTALL` | `superseded` | `INV.PKG.CORRUPT-ARCHIVE-NOT-READY` | — |
| `INV-PKG-TOOL-CLOSURE` | `retired` | — | — |
| `INV-PKG-BINARY-NAME` | `retired` | — | — |
| `INV-PKG-VERSION-LOCKSTEP` | `retired` | — | — |
| `INV-PKG-OLDEST-CLIENT-KEYS` | `retired` | — | — |
| `INV-PKG-DEV-ONLY-PACKAGE` | `retired` | — | — |
| `INV-PKG-NO-INTERNAL-MATERIAL` | `carried` | `INV.PKG.THIN-PACKAGE` | — |
| `INV-PKG-ATTRIBUTION-COVERAGE` | `carried` | `INV.PKG.ATTRIBUTION` | — |
| `INV-PLATFORM-OS-BEHIND-FACADE` | `carried` | `INV.PLATFORM.OS-BEHIND-FACADE` | — |
| `INV-PLATFORM-NO-PATH-EXEMPTIONS` | `carried` | `INV.PLATFORM.OS-BEHIND-FACADE` | — |
| `INV-PLATFORM-COLOCATED-TESTS` | `carried` | `INV.PLATFORM.OS-BEHIND-FACADE` | — |
| `INV-PLATFORM-NO-ORPHAN-PROCESSES` | `superseded` | `INV.WIRE.EOF-DRAINS-WORKERS` | — |
| `INV-HOST-NEUTRAL-ORCHESTRATOR` | `carried` | `INV.HOST.KNOWLEDGE-BEHIND-FACADE` | — |
| `INV-HOST-KNOWLEDGE-BEHIND-FACADE` | `carried` | `INV.HOST.KNOWLEDGE-BEHIND-FACADE` | — |
| `INV-HOST-UNIFORM-CALL-SITES` | `carried` | `INV.HOST.KNOWLEDGE-BEHIND-FACADE` | — |
| `INV-CI-MANDATORY-BUILD` | `retired` | — | — |
| `INV-CI-EXACT-CACHE-KEYS` | `retired` | — | — |
| `INV-CI-NARROW-ARTIFACTS` | `retired` | — | — |
| `INV-CI-SELF-VERIFIED-ARCHIVE` | `retired` | — | — |
| `INV-CI-TAG-ONLY-PUBLISH` | `carried` | `INV.CI.TAG-ONLY-PUBLISH` | — |
| `INV-CI-SINGLE-GATE` | `carried` | `INV.CI.ONE-AGGREGATE-GATE` | — |
| `INV-DOC-REGISTRY-ENTRY-FORMAT` | `retired` | — | — |
| `INV-DOC-NO-ID-REUSE` | `retired` | — | — |
| `INV-DOC-REAL-CHECKS` | `retired` | — | — |
| `INV-DOC-INDEX-SYNC` | `retired` | — | — |
| `INV-DOC-ARCHIVE-NOT-NORMATIVE` | `retired` | — | — |
| `INV-DOC-RELATIVE-LINKS` | `retired` | — | — |
| `INV-DOC-RUSSIAN-NORMATIVE` | `retired` | — | — |
| `INV-DOC-SINGLE-RULE-OWNER` | `retired` | — | — |
| `INV-DOC-SUPERSEDE-NOT-EDIT` | `superseded` | `INV.REGISTRY.PRODUCT-DECISION-IS-HISTORY` | — |

## Требования к качеству

| Запись v1 | Судьба | Преемник v2 | Причина |
| --- | --- | --- | --- |
| `REQ-PERF-DEADLINE` | `retired` | — | — |
| `REQ-PERF-VERIFIED-HANDOFF` | `retired` | — | — |
| `REQ-PERF-DELIVERY-WINDOW` | `superseded` | `DEC.2026-08-20.LONG-WORK-ANSWERS-WITH-STATE` | — |
| `REQ-PERF-WARM-REUSE` | `superseded` | `INV.APP.HIDDEN-SERVICES` | — |
| `REQ-PERF-SOURCE-BOUNDS` | `retired` | — | — |
| `REQ-TOKEN-NO-EXTRA-ROUNDTRIP` | `retired` | — | — |
| `REQ-TOKEN-BOUNDED-LOG-TAILS` | `retired` | — | — |
| `REQ-SAFETY-PREVIEW-BY-DEFAULT` | `retired` | — | — |
| `REQ-SAFETY-SECRET-REDACTION` | `retired` | — | — |
| `REQ-SAFETY-SUPPORT-LOCK` | `retired` | — | — |
| `REQ-SAFETY-NO-PARTIAL-WRITE` | `superseded` | `INV.SOURCE.ATOMIC-PUBLISH` | — |
| `REQ-OBS-STABLE-ENVELOPE` | `superseded` | `INV.SURFACE.RESULT-CONTRACTS-MATCH-REVIEW` | — |
| `REQ-OBS-DETACHED-PROGRESS` | `retired` | — | — |
| `REQ-MAINT-CONTAINED-ADAPTER-SWAP` | `carried` | `INV.APP.DEPENDENCY-DIRECTION` | — |
| `REQ-MAINT-NO-TRANSPORT-EDIT` | `retired` | — | — |
| `REQ-MAINT-DONOR-PARITY` | `retired` | — | — |
| `REQ-COMPAT-ALL-TARGETS-GREEN` | `retired` | — | — |
| `REQ-COMPAT-OLDEST-CLIENT-LOAD` | `retired` | — | — |
| `REQ-COMPAT-IDENTICAL-HOST-SURFACE` | `retired` | — | — |
| `REQ-COMPAT-FORMAT-PROFILE` | `superseded` | `CTR.FORMAT.PLATFORM-XML-8-3-27` | — |
| `REQ-REL-BUNDLED-ENGINES` | `superseded` | `DEC.2026-08-20.ENGINES-COME-FROM-THE-TOOLCHAIN` | — |
| `REQ-REL-INSTALL-ONCE` | `carried` | `DEC.2026-08-19.ARTIFACT-VERSIONED-CACHE` | — |
| `REQ-REL-COLD-INSTALL-BUDGET` | `superseded` | `DEC.2026-08-19.DELIVERY-HAS-NO-BUDGET` | — |
| `REQ-REL-NO-SILENT-STALL` | `superseded` | `DEC.2026-08-19.DELIVERY-HAS-NO-BUDGET` | — |
| `REQ-REL-REAL-CONFIG-GATE` | `retired` | — | — |

## Приёмочные контракты

| Запись v1 | Судьба | Преемник v2 | Причина |
| --- | --- | --- | --- |
| `acceptance/format-profile-8-3-27.md` | `superseded` | `CTR.FORMAT.PLATFORM-XML-8-3-27` | — |
| `acceptance/logical-source-addressing-and-resource-access.md` | `superseded` | `INV.SOURCE.SNAPSHOT-BINDING` | — |
| `acceptance/unica-mcp-validation.md` | `retired` | — | `historical-only` |
