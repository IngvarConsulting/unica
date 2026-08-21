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
| `ADR-0016` | `superseded` | `DEC.2026-08-21.SINGLE-WRITABLE-PLATFORM-XML-PROFILE` | — |
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
| `INV-PRODUCT-DEVELOPER-OPERATIONS` | `superseded` | `INV.SURFACE.SKILL-ROUTING` | — |
| `INV-PRODUCT-NO-ENGINE-ROUTING` | `superseded` | `INV.SURFACE.NO-ADAPTER-TARGETS` | — |
| `INV-PRODUCT-PACKAGE-PARITY` | `superseded` | `INV.PKG.PACKAGED-PUBLIC-SURFACE` | — |
| `INV-PRODUCT-TOOL-VERSION-SOURCE` | `superseded` | `INV.SURFACE.TOOL-VERSION-SOURCE` | — |
| `INV-PRODUCT-DCS-NAMING` | `superseded` | `INV.SURFACE.DCS-NAMING` | — |
| `INV-PRODUCT-NO-FORMAT-MIGRATION` | `superseded` | `INV.SOURCE.NO-FORMAT-MIGRATION`, `INV.SOURCE.OWNER-VERSION-GATE` | — |
| `INV-MCP-META-SURFACE` | `superseded` | `INV.SURFACE.META-TOOLSET` | — |
| `INV-MCP-META-OBSERVATION` | `superseded` | `INV.APP.META-OBSERVATION` | — |
| `INV-MCP-META-INFO-COVERAGE` | `superseded` | `INV.APP.META-INFO-COVERAGE` | — |
| `INV-MCP-EVENT-SOURCE` | `superseded` | `INV.APP.EVENT-SOURCE` | — |
| `INV-MCP-EVENT-BINDING` | `superseded` | `INV.APP.EVENT-BINDING` | — |
| `INV-MCP-META-FINDINGS` | `superseded` | `INV.APP.META-FINDINGS` | — |
| `INV-MCP-ROLE-EDIT` | `superseded` | `INV.SURFACE.ROLE-EDIT-LOGICAL` | — |
| `INV-MCP-XDTO-LOGICAL-TARGET` | `superseded` | `INV.SURFACE.XDTO-LOGICAL-TARGET` | — |
| `INV-MCP-NO-ENGINE-SERVERS` | `carried` | `INV.WIRE.ONE-SERVER` | — |
| `INV-MCP-SINGLE-ENTRY` | `carried` | `INV.WIRE.ONE-SERVER` | — |
| `INV-MCP-SERVER-NAME` | `carried` | `INV.WIRE.ONE-SERVER` | — |
| `INV-MCP-NAMESPACE` | `carried` | `INV.SURFACE.NAMESPACE` | — |
| `INV-MCP-DATA-DRIVEN-SCHEMA` | `superseded` | `INV.WIRE.DATA-DRIVEN-TOOL-LIST`, `INV.SURFACE.NO-RAW-ADAPTER-ARGS` | — |
| `INV-MCP-REACHABLE-ARGS` | `superseded` | `INV.SURFACE.PUBLISHED-ARGS-ARE-READ` | — |
| `INV-MCP-SDK-TRANSPORT` | `superseded` | `INV.WIRE.SDK-DEPENDENCY`, `INV.WIRE.SDK-MODULE-EXPORTS`, `INV.WIRE.SDK-SERVER-HANDLER`, `INV.WIRE.SDK-TRANSPORT`, `INV.WIRE.SDK-INITIALIZE`, `INV.WIRE.DIRECT-FIRST-LIFECYCLE` | — |
| `INV-MCP-VERSION-TIERS` | `superseded` | `INV.WIRE.GUARANTEED-VERSIONS`, `INV.WIRE.PINNED-FALLBACK-VERSION` | — |
| `INV-MCP-DEFERRED-READ` | `superseded` | `INV.APP.DEFERRED-MANIFEST`, `INV.APP.DEFERRED-READ` | — |
| `INV-MCP-BOUNDED-ADMISSION` | `carried` | `INV.WIRE.BOUNDED-ADMISSION` | — |
| `INV-MCP-DELIVERY-STATE` | `carried` | `DEC.2026-08-20.LONG-WORK-ANSWERS-WITH-STATE` | — |
| `INV-MCP-RUNTIME-RECEIPT` | `superseded` | `INV.RUNTIME.EXECUTE-RECEIPT`, `INV.RUNTIME.RISK-CLASSIFICATION`, `INV.RUNTIME.PREVIEW-NONEXECUTING`, `INV.RUNTIME.NO-REFUSAL-FALLBACK` | — |
| `INV-MCP-SURFACE-SYNC` | `superseded` | `CTR.WIRE.TOOL-SURFACE` | — |
| `INV-MCP-TYPED-RESULT` | `superseded` | `CTR.WIRE.TOOL-SURFACE` | — |
| `INV-MCP-DIAGNOSTIC-TARGET` | `superseded` | `INV.SURFACE.DIAGNOSTIC-TARGET` | — |
| `INV-MCP-PROJECT-READINESS` | `superseded` | `INV.SURFACE.PROJECT-READINESS` | — |
| `INV-MCP-PREVIEW-MUTATION-ONLY` | `carried` | `INV.WIRE.PREVIEW-IS-MUTATION-ONLY` | — |
| `INV-MCP-SOURCE-SURFACE` | `superseded` | `INV.SURFACE.SOURCE-TOOL-SPECS` | — |
| `INV-MCP-CODE-SEARCH-ROLES` | `superseded` | `INV.SURFACE.CODE-SEARCH-ROLES` | — |
| `INV-MCP-DOCUMENTATION-SECTIONS` | `superseded` | `INV.APP.DOCUMENTATION-SECTIONS` | — |
| `INV-MCP-DOCUMENTATION-GET` | `superseded` | `INV.APP.DOCUMENTATION-GET` | — |
| `INV-APP-DOCUMENTATION-NETWORK-POLICY` | `superseded` | `INV.APP.DOCUMENTATION-NETWORK-POLICY` | — |
| `INV-MCP-SEARCH-SEMANTICS` | `superseded` | `INV.APP.SEARCH-EXPANSIONS`, `INV.APP.SEARCH-TIE-ORDER` | — |
| `INV-MCP-OUTLINE-DATA` | `superseded` | `INV.APP.OUTLINE-SOURCE` | — |
| `INV-SKILL-DECLARED-ROUTING` | `carried` | `INV.SURFACE.SKILL-ROUTING` | — |
| `INV-SKILL-NO-ADAPTER-TARGETS` | `superseded` | `INV.SURFACE.NO-ADAPTER-TARGETS` | — |
| `INV-SKILL-NO-SCRIPT-ROUTE` | `superseded` | `INV.SURFACE.SKILL-NO-SCRIPT-ROUTE` | — |
| `INV-SKILL-SCRIPTS-AS-FIXTURES` | `superseded` | `INV.APP.SKILL-SCRIPT-FIXTURES` | — |
| `INV-SKILL-DOCUMENTED-PREVIEW` | `superseded` | `INV.SURFACE.SKILL-PREVIEW-GUIDANCE` | — |
| `INV-SKILL-SOURCE-FALLBACK` | `superseded` | `INV.SURFACE.SOURCE-SKILL-ROUTING` | — |
| `INV-SKILL-EXECUTABLE-EXAMPLES` | `superseded` | `INV.SURFACE.EXECUTABLE-SKILL-EXAMPLES` | — |
| `INV-SKILL-REACHABLE-REFERENCES` | `superseded` | `INV.APP.SKILL-REFERENCE-REACHABILITY` | — |
| `INV-APP-DISPATCH-OWNERSHIP` | `superseded` | `INV.APP.THIN-TRANSPORT` | — |
| `INV-APP-THIN-TRANSPORT` | `carried` | `INV.APP.THIN-TRANSPORT` | — |
| `INV-APP-NO-ADAPTER-BYPASS` | `carried` | `INV.APP.DEPENDENCY-DIRECTION` | — |
| `INV-APP-NO-SCRIPT-BACKEND` | `superseded` | `INV.APP.NO-SCRIPT-BACKEND` | — |
| `INV-APP-DEPENDENCY-DIRECTION` | `carried` | `INV.APP.DEPENDENCY-DIRECTION` | — |
| `INV-APP-NO-DIRECT-GIT` | `superseded` | `INV.APP.NO-DIRECT-GIT` | — |
| `INV-APP-SUPPORT-STATE` | `superseded` | `INV.APP.SUPPORT-STATE` | — |
| `INV-APP-PARTIAL-FALLBACK` | `superseded` | `INV.APP.PARTIAL-FALLBACK` | — |
| `INV-APP-CONFIG-SNAPSHOT` | `carried` | `INV.APP.CONFIG-SNAPSHOT` | — |
| `INV-APP-CODE-PROVIDER-BOUNDARY` | `carried` | `INV.APP.PROVIDER-NEUTRAL` | — |
| `INV-APP-DIAGNOSTIC-PROVIDERS` | `superseded` | `INV.APP.DIAGNOSTIC-PROVIDERS` | — |
| `INV-APP-DOCUMENTATION-NO-DISK-STATE` | `superseded` | `INV.APP.DOCUMENTATION-CONTAINER-FINGERPRINT` | — |
| `INV-APP-OUTLINE-SOURCE` | `superseded` | `INV.APP.OUTLINE-SOURCE` | — |
| `INV-APP-LAZY-HIDDEN-SERVICES` | `carried` | `INV.APP.HIDDEN-SERVICES` | — |
| `INV-CACHE-ORCHESTRATOR-OWNED` | `superseded` | `INV.CACHE.ORCHESTRATOR-OWNED` | — |
| `INV-CACHE-REPORTED-EFFECTS` | `superseded` | `INV.CACHE.MUTATION-EVENT-COVERAGE`, `INV.CACHE.EVENT-IMPACT-CLOSED`, `INV.CACHE.REPORTED-EFFECTS` | — |
| `INV-CACHE-WORKSPACE-ROOT` | `superseded` | `INV.CACHE.WORKSPACE-ROOT` | — |
| `INV-CACHE-PROVIDER-STATE-OUTSIDE-SOURCE` | `carried` | `INV.CACHE.STATE-OUTSIDE-SOURCE` | — |
| `INV-CACHE-GENERATION-CUTOVER` | `superseded` | `INV.CACHE.GENERATION-CUTOVER` | — |
| `INV-CACHE-WRITE-FREE-PREVIEW` | `superseded` | `INV.CACHE.INDEX-PREVIEW-WRITE-FREE`, `INV.SAFETY.PREVIEW-BY-DEFAULT` | — |
| `INV-CACHE-PERSISTED-STALENESS` | `superseded` | `INV.CACHE.PERSISTED-STALENESS` | — |
| `INV-CACHE-WORKTREE-ISOLATION` | `superseded` | `INV.CACHE.WORKTREE-ISOLATION` | — |
| `INV-CACHE-RLM-REVISION` | `superseded` | `INV.CACHE.RLM-REVISION` | — |
| `INV-CACHE-RUNTIME-ROOT-ORDER` | `superseded` | `INV.CACHE.OVERRIDE-PRIORITY` | — |
| `INV-SOURCE-ROOT-SEPARATION` | `superseded` | `INV.SOURCE.ROOT-SEPARATION`, `INV.SOURCE.ROOT-ALIAS-SEPARATION`, `INV.SOURCE.ROOT-READINESS` | — |
| `INV-SOURCE-PORTABLE-GIT` | `superseded` | `INV.SOURCE.PORTABLE-GIT`, `INV.SOURCE.PORTABLE-LFS-ADVISORY` | — |
| `INV-SOURCE-PER-SET-FORMAT` | `superseded` | `INV.SOURCE.FORMAT-PER-SET` | — |
| `INV-SOURCE-UNAMBIGUOUS-SET` | `carried` | `INV.SOURCE.UNAMBIGUOUS-SET` | — |
| `INV-SOURCE-MULTI-FORMAT-WORKSPACE` | `carried` | `INV.SOURCE.MULTI-FORMAT-WORKSPACE` | — |
| `INV-SOURCE-AUTODETECT-CATALOG` | `carried` | `INV.SOURCE.AUTODETECT-CATALOG` | — |
| `INV-SOURCE-PLATFORM-XML-ONLY` | `superseded` | `DEC.2026-08-21.LEGACY-UNKNOWN-NATIVE-SOURCE-FORMAT`, `INV.SOURCE.PLATFORM-XML-ONLY` | — |
| `INV-SOURCE-SINGLE-RESOLVED-ROOT` | `superseded` | `INV.SOURCE.DEFAULT-SET-SELECTION` | — |
| `INV-SOURCE-LOGICAL-IDENTITY` | `superseded` | `INV.SOURCE.LOGICAL-IDENTITY`, `INV.SOURCE.LOGICAL-INPUT` | — |
| `INV-SOURCE-SUBSYSTEM-TOPOLOGY` | `superseded` | `INV.SOURCE.SUBSYSTEM-REGISTRATION`, `INV.SOURCE.SUBSYSTEM-TOPOLOGY`, `INV.SOURCE.SUBSYSTEM-MEMBERSHIP`, `INV.SOURCE.SUBSYSTEM-INCOMPLETE-UNAVAILABLE`, `INV.SOURCE.SUBSYSTEM-DEADLINE-UNAVAILABLE`, `INV.SOURCE.SUBSYSTEM-ADDRESS` | — |
| `INV-SOURCE-READER-SELECTOR` | `superseded` | `INV.SOURCE.READER-SELECTOR`, `INV.SOURCE.READER-OUTPUT-PARITY` | — |
| `INV-SOURCE-READER-MIGRATION` | `superseded` | `INV.SOURCE.READER-MIGRATION`, `INV.SURFACE.DIAGNOSTIC-TARGET` | — |
| `INV-SOURCE-WRITE-TARGET-KIND` | `carried` | `INV.SOURCE.WRITE-TARGET-KIND` | — |
| `INV-SOURCE-SNAPSHOT-BINDING` | `carried` | `INV.SOURCE.SNAPSHOT-BINDING` | — |
| `INV-SOURCE-ROLE-ALLOWLIST` | `retired` | — | `behavior-removed: DEC.2026-08-21.SOURCE-READ-ONLY-SURFACE` |
| `INV-SOURCE-OBSERVED-EOL` | `superseded` | `INV.SOURCE.OBSERVED-EOL-PROFILE`, `INV.SOURCE.CODE-PATCH-EOL` | — |
| `INV-SOURCE-TAIL-INSERT` | `carried` | `INV.SOURCE.TAIL-INSERT` | — |
| `INV-SOURCE-ATOMIC-PUBLISH` | `superseded` | `INV.SOURCE.ATOMIC-PUBLISH` | — |
| `INV-SOURCE-IDEMPOTENT-REWRITE` | `retired` | — | `behavior-removed: DEC.2026-08-21.MUTATION-IDEMPOTENCE-SCOPE` |
| `INV-SOURCE-WRITE-CONTAINMENT` | `carried` | `INV.SOURCE.WRITE-CONTAINMENT` | — |
| `INV-SOURCE-WRITABLE-FORMAT` | `superseded` | `INV.SOURCE.WRITABLE-PROFILE`, `INV.SOURCE.EXACT-VERSION` | — |
| `INV-SOURCE-ROOT-POLICIES` | `superseded` | `INV.SOURCE.ROOT-POLICIES`, `INV.SOURCE.ROOT-POLICIES-CLOSED`, `INV.SOURCE.ROOT-POLICY-OWNERSHIP`, `INV.SOURCE.ROOT-POLICY-PUBLICATION` | — |
| `INV-SOURCE-OWNER-VERSION-GATE` | `superseded` | `INV.SOURCE.OWNER-VERSION-GATE`, `INV.SOURCE.EXACT-VERSION` | — |
| `INV-SOURCE-EXACT-VERSION-LITERAL` | `superseded` | `INV.SOURCE.EXACT-VERSION-LITERAL`, `INV.SOURCE.ENTITY-SPELLED-VERSION` | — |
| `INV-SOURCE-EXACT-ROOT-QNAME` | `superseded` | `INV.SOURCE.EXACT-ROOT-QNAME`, `INV.SOURCE.EXACT-ROOT-VERSIONLESS`, `INV.SOURCE.EXACT-ROOT-FORM` | — |
| `INV-SOURCE-BOUND-PREIMAGES` | `superseded` | `INV.SOURCE.BOUND-PREIMAGES`, `INV.SOURCE.BOUND-HANDLER-PREFLIGHT` | — |
| `INV-SOURCE-ROLLBACK-VISIBLE` | `superseded` | `INV.SOURCE.ROLLBACK-VISIBLE`, `INV.SOURCE.ROLLBACK-DIAGNOSTIC-CLASS` | — |
| `INV-PKG-UNTRACKED-BUILD-OUTPUT` | `superseded` | `INV.PKG.TRACKED-BIN-REJECTED`, `INV.PKG.TRACKED-IGNORED-REJECTED`, `INV.PKG.SOURCE-SYMLINK-REJECTED` | — |
| `INV-PKG-THIN-PACKAGE` | `superseded` | `INV.PKG.THIN-PACKAGE` | — |
| `INV-PKG-VERIFIED-ATOMIC-INSTALL` | `superseded` | `INV.PKG.VERIFIED-ATOMIC-INSTALL` | — |
| `INV-PKG-TOOL-CLOSURE` | `superseded` | `INV.PKG.BUILD-TOOL-CLOSURE`, `INV.PKG.BUILD-ARCHIVE-SAFETY`, `INV.PKG.RUNTIME-TOOL-CLOSURE`, `INV.PKG.RUNTIME-TOOL-MODES`, `INV.PKG.RUNTIME-TOOL-PATHS`, `INV.PKG.INSTALL-TOOL-CLOSURE`, `INV.PKG.CORRUPT-ARCHIVE-NOT-READY` | — |
| `INV-PKG-BINARY-NAME` | `superseded` | `INV.PKG.PUBLIC-BINARY-NAME` | — |
| `INV-PKG-VERSION-LOCKSTEP` | `superseded` | `INV.PKG.VERSION-LOCKSTEP`, `INV.PKG.CLAUDE-DEFAULT-DISCOVERY` | — |
| `INV-PKG-OLDEST-CLIENT-KEYS` | `superseded` | `INV.PKG.OLDEST-CLIENT-KEYS`, `INV.PKG.CODEX-CATALOG-RELEASE-PIN`, `INV.PKG.CLAUDE-CATALOG-RELEASE-PIN` | — |
| `INV-PKG-DEV-ONLY-PACKAGE` | `superseded` | `INV.PKG.DEV-PACKAGE-ISOLATED` | — |
| `INV-PKG-NO-INTERNAL-MATERIAL` | `carried` | `INV.PKG.THIN-PACKAGE` | — |
| `INV-PKG-ATTRIBUTION-COVERAGE` | `carried` | `INV.PKG.ATTRIBUTION` | — |
| `INV-PLATFORM-OS-BEHIND-FACADE` | `carried` | `INV.PLATFORM.OS-BEHIND-FACADE` | — |
| `INV-PLATFORM-NO-PATH-EXEMPTIONS` | `carried` | `INV.PLATFORM.OS-BEHIND-FACADE` | — |
| `INV-PLATFORM-COLOCATED-TESTS` | `carried` | `INV.PLATFORM.OS-BEHIND-FACADE` | — |
| `INV-PLATFORM-NO-ORPHAN-PROCESSES` | `superseded` | `INV.PLATFORM.PROCESS-TREE-LIFECYCLE` | — |
| `INV-HOST-NEUTRAL-ORCHESTRATOR` | `carried` | `INV.HOST.KNOWLEDGE-BEHIND-FACADE` | — |
| `INV-HOST-KNOWLEDGE-BEHIND-FACADE` | `carried` | `INV.HOST.KNOWLEDGE-BEHIND-FACADE` | — |
| `INV-HOST-UNIFORM-CALL-SITES` | `carried` | `INV.HOST.KNOWLEDGE-BEHIND-FACADE` | — |
| `INV-CI-MANDATORY-BUILD` | `superseded` | `INV.CI.LOCKED-WORKSPACE-BUILD` | — |
| `INV-CI-EXACT-CACHE-KEYS` | `superseded` | `INV.CI.EXACT-CARGO-CACHE` | — |
| `INV-CI-NARROW-ARTIFACTS` | `superseded` | `INV.CI.NARROW-TARGET-ARTIFACTS`, `INV.CI.THIN-PAYLOAD-RETENTION`, `INV.CI.INTERMEDIATE-RETENTION` | — |
| `INV-CI-SELF-VERIFIED-ARCHIVE` | `superseded` | `INV.CI.RUNTIME-ARCHIVE-SELF-VERIFIED`, `INV.CI.RUNTIME-ARCHIVE-DETERMINISTIC`, `INV.CI.RUNTIME-METADATA-HASHES`, `INV.CI.EXTRACTED-RUNTIME-SMOKE`, `INV.CI.PUBLISHED-ASSETS-REVERIFIED` | — |
| `INV-CI-TAG-ONLY-PUBLISH` | `carried` | `INV.CI.TAG-ONLY-PUBLISH` | — |
| `INV-CI-SINGLE-GATE` | `carried` | `INV.CI.ONE-AGGREGATE-GATE` | — |
| `INV-DOC-REGISTRY-ENTRY-FORMAT` | `superseded` | `INV.DOC.RECORD-SHAPE`, `INV.REGISTRY.SYMBOL-MATCHES-PATH`, `INV.REGISTRY.GOVERNS-DECLARED`, `INV.REGISTRY.CHECK-EXISTS`, `INV.REGISTRY.RECIPROCAL-OWNERSHIP` | — |
| `INV-DOC-NO-ID-REUSE` | `superseded` | `INV.DOC.GLOBAL-ID-NONREUSE`, `INV.REGISTRY.PRODUCT-DECISION-IS-HISTORY`, `INV.REGISTRY.PRODUCT-RULE-NEEDS-GROUND` | — |
| `INV-DOC-REAL-CHECKS` | `superseded` | `INV.REGISTRY.CHECK-EXISTS` | — |
| `INV-DOC-INDEX-SYNC` | `superseded` | `INV.DOC.GENERATED-INDEX-SYNC` | — |
| `INV-DOC-ARCHIVE-NOT-NORMATIVE` | `superseded` | `INV.DOC.PROJECT-NOTES-NON-NORMATIVE` | — |
| `INV-DOC-RELATIVE-LINKS` | `superseded` | `INV.DOC.PACKAGED-RELATIVE-LINKS` | — |
| `INV-DOC-RUSSIAN-NORMATIVE` | `superseded` | `DEC.2026-08-21.V2-PROCESS-POLICY` | — |
| `INV-DOC-SINGLE-RULE-OWNER` | `superseded` | `INV.REGISTRY.RECIPROCAL-OWNERSHIP`, `INV.REGISTRY.NO-SELF-GLOSS` | — |
| `INV-DOC-SUPERSEDE-NOT-EDIT` | `superseded` | `INV.REGISTRY.PRODUCT-DECISION-IS-HISTORY` | — |

## Требования к качеству

| Запись v1 | Судьба | Преемник v2 | Причина |
| --- | --- | --- | --- |
| `REQ-PERF-DEADLINE` | `superseded` | `INV.PERF.SERVICE-CONNECT-BUDGET`, `INV.PERF.SERVICE-IO-DEADLINE`, `INV.PERF.SERVICE-OPERATION-DEADLINE` | — |
| `REQ-PERF-VERIFIED-HANDOFF` | `superseded` | `INV.PERF.BOOTSTRAP-VERIFY-BUDGET`, `INV.PERF.BOOTSTRAP-VERIFY-LIFECYCLES`, `INV.PERF.RELEASE-ASSET-VERIFICATION`, `INV.PERF.RELEASE-HANDOFF-GATE` | — |
| `REQ-PERF-DELIVERY-WINDOW` | `superseded` | `DEC.2026-08-20.LONG-WORK-ANSWERS-WITH-STATE` | — |
| `REQ-PERF-WARM-REUSE` | `superseded` | `INV.APP.HIDDEN-SERVICES` | — |
| `REQ-PERF-SOURCE-BOUNDS` | `superseded` | `INV.PERF.SOURCE-RESOURCE-LIMITS`, `INV.PERF.SOURCE-SNAPSHOT-TTL`, `INV.PERF.SOURCE-SNAPSHOT-CAPACITY`, `INV.PERF.SOURCE-SNAPSHOT-BYTE-BUDGET`, `INV.PERF.SOURCE-CANCELLATION`, `INV.SURFACE.SOURCE-TOOL-SPECS` | — |
| `REQ-TOKEN-NO-EXTRA-ROUNDTRIP` | `superseded` | `INV.TOKEN.CACHE-IMPACT-IN-RESULT` | — |
| `REQ-TOKEN-BOUNDED-LOG-TAILS` | `superseded` | `INV.TOKEN.RUNTIME-LOG-ARTIFACTS`, `INV.TOKEN.RUNTIME-LOG-TAIL`, `INV.TOKEN.RUNTIME-LOG-REQUEST-BOUND` | — |
| `REQ-SAFETY-PREVIEW-BY-DEFAULT` | `superseded` | `INV.SAFETY.PREVIEW-BY-DEFAULT` | — |
| `REQ-SAFETY-SECRET-REDACTION` | `superseded` | `INV.SAFETY.STREAM-SECRET-REDACTION`, `INV.SAFETY.RUNTIME-SECRET-REDACTION`, `INV.SAFETY.CONFIG-ERROR-REDACTION` | — |
| `REQ-SAFETY-SUPPORT-LOCK` | `superseded` | `INV.SAFETY.SUPPORT-GUARD-COVERAGE`, `INV.SAFETY.SUPPORT-GUARD-PARITY`, `INV.SAFETY.SUPPORT-POLICY-DOWNGRADE` | — |
| `REQ-SAFETY-NO-PARTIAL-WRITE` | `superseded` | `INV.SOURCE.ATOMIC-PUBLISH` | — |
| `REQ-OBS-STABLE-ENVELOPE` | `superseded` | `INV.SURFACE.RESULT-CONTRACTS-MATCH-REVIEW` | — |
| `REQ-OBS-DETACHED-PROGRESS` | `superseded` | `INV.OBS.RUNTIME-JOB-SURFACE`, `INV.OBS.DETACHED-JOB-STATE`, `INV.OBS.WAIT-TIMEOUT-KEEPS-JOB` | — |
| `REQ-MAINT-CONTAINED-ADAPTER-SWAP` | `carried` | `INV.APP.DEPENDENCY-DIRECTION` | — |
| `REQ-MAINT-NO-TRANSPORT-EDIT` | `superseded` | `INV.WIRE.DATA-DRIVEN-TOOL-LIST` | — |
| `REQ-MAINT-DONOR-PARITY` | `superseded` | `INV.MAINT.DONOR-PARITY`, `INV.MAINT.DONOR-SNAPSHOT-INTEGRITY` | — |
| `REQ-COMPAT-ALL-TARGETS-GREEN` | `superseded` | `INV.CI.ALL-TARGETS-GREEN` | — |
| `REQ-COMPAT-OLDEST-CLIENT-LOAD` | `superseded` | `INV.PKG.OLDEST-CLIENT-LOAD` | — |
| `REQ-COMPAT-IDENTICAL-HOST-SURFACE` | `superseded` | `INV.PKG.TWO-HOSTS-ONE-TREE`, `INV.PKG.HOST-SHARED-MCP`, `INV.PKG.HOST-MANIFEST-LOCKSTEP`, `INV.PKG.VERSION-LOCKSTEP`, `INV.PKG.HOST-CATALOG-PROMOTION` | — |
| `REQ-COMPAT-FORMAT-PROFILE` | `superseded` | `INV.PRODUCT.FULL-DUMP-PROFILE`, `INV.SOURCE.WRITABLE-PROFILE` | — |
| `REQ-REL-BUNDLED-ENGINES` | `superseded` | `DEC.2026-08-20.ENGINES-COME-FROM-THE-TOOLCHAIN` | — |
| `REQ-REL-INSTALL-ONCE` | `carried` | `DEC.2026-08-19.ARTIFACT-VERSIONED-CACHE` | — |
| `REQ-REL-COLD-INSTALL-BUDGET` | `superseded` | `INV.PKG.COLD-INSTALL-STARTUP-BUDGET` | — |
| `REQ-REL-NO-SILENT-STALL` | `superseded` | `INV.CI.LINEAR-IDEMPOTENT-PUBLICATION` | — |
| `REQ-REL-REAL-CONFIG-GATE` | `superseded` | `INV.REL.ASSESSMENT-WORKFLOW-GATE`, `INV.REL.ASSESSMENT-PIN`, `INV.REL.ASSESSMENT-REPORT`, `INV.REL.BLOCKING-ASSESSMENT` | — |

## Приёмочные контракты

| Запись v1 | Судьба | Преемник v2 | Причина |
| --- | --- | --- | --- |
| `acceptance/format-profile-8-3-27.md` | `retired` | — | `historical-only` |
| `acceptance/logical-source-addressing-and-resource-access.md` | `retired` | — | `historical-only` |
| `acceptance/unica-mcp-validation.md` | `retired` | — | `historical-only` |
