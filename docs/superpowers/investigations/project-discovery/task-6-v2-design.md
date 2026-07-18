# Task 6A/6B v2 — BSL Evidence Implementation Design

> **Task 5B v6 back-propagation, 2026-07-18.** This file supersedes immutable
> Task 6 v1 SHA-256
> `0462f2b97a4cb04aa9503af00df8d64c74a197257471e4d4fe0459bbf1995743`.
> The former provider-local individual-record prefix is rejected: every BSL
> query now classifies complete `SemanticAtomicGroupIdV2` groups and applies its
> local `max_records` prefix-stop before returning. Implement only from this v2
> file together with the frozen Task 5B v6 and Task 7 v6 hashes.

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` (recommended) or
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Реализовать receipt-grade определения, точный lexical search и
консервативные BSL call edges исключительно из байтов authoritative
`SourceSnapshot`, оставив workspace service v2 только content-addressed
ускорителем того же парсера.

**Architecture:** Task 6A вводит чистый bounded lexer/parser и три query-specific
provider-а над `SourceSnapshotPort::read_verified`; их результат не зависит от
наличия индекса, SQLite, bsl-analyzer или workspace service. Task 6B добавляет
workspace-service cache schema v2: сервис может вернуть результат того же
парсера только для точного file content digest, а вызывающий процесс сначала
повторно подтверждает байты через snapshot reader. Cache miss, stale response,
old schema или отказ сервиса всегда приводят к локальному Task 6A parse и не
меняют evidence/outcome.

**Tech Stack:** Rust, `serde`, существующий `sha2`, application discovery
ports/model, Task 4 `SourceManifest`/verified reader, Task 5 shared Platform XML
catalog. Новая parser dependency и новый публичный MCP tool не нужны.

## Global Constraints

- Источник истины: code/tests/package metadata > active spec > historical plan.
- Публичный MCP server остаётся один, `unica`; Task 6 не регистрирует
  `unica.project.discover` и не меняет его wire schema.
- Только analysis snapshot участвует в BSL inference; mutation snapshots лишь
  связаны с будущим receipt.
- Только Platform XML анализируется; EDT остаётся diagnostic-only и не вызывает
  BSL providers.
- Любой принятый byte должен быть manifest-listed и прочитан через
  `SourceSnapshotPort::read_verified`.
- SQLite, RLM DB, display text, объединение MCP text sections и live glob scan
  запрещены в authoritative path.
- Bounded/unavailable/failed никогда не означают отсутствие.
- Wall-clock abort не возвращает timing-dependent partial prefix.
- `maxEvidence` — resource bound, а не relevance/top-k threshold.
- `SEMANTIC_ATOMIC_GROUP_REGISTRY=semantic-evidence-groups/v2` и
  `SEMANTIC_ATOMIC_ENCODER=semantic-evidence-group-encoder/v2` из Task 5B v6
  применяются до каждого provider-local record ceiling.
- Service/cache hit/miss, duration и workspace epoch не входят в stable digests.

---

## 1. Решение и исправления исходного Task 6

Исторический plan в текущем виде выполнять нельзя. В нём typed live
workspace/index service назван основным источником, а snapshot scan — fallback.
Это переворачивает freshness boundary: сервис читает live source root и не
доказывает, что facts получены из тех же байтов, которые входят в receipt.

Принятое разделение:

1. **Task 6A — correctness path.** Один parser contract
   `unica.snapshot-bsl.v1`, одни типы syntax analysis, один результат для local
   execution и workspace-service cache.
2. **Task 6B — performance path.** Workspace service v2 кэширует результат
   Task 6A по `(parserContract, contentDigest)`. Он не выдаёт evidence,
   отрицательные факты или coverage самостоятельно.
3. Существующий `call_bsl_mcp()` остаётся display consumer для `unica.code.*`.
   Его `result_text`/`mcp_tool_text()` не используется discovery.
4. `workspace_index.rs` не меняется в Task 6. Ни прямой SQLite read, ни
   `IndexReadiness` не нужны receipt-grade parser-у.
5. Доступность сервиса не является provider outcome. Service failure не даёт
   `bounded`, `unavailable`, warning или другой provider version — выполняется
   тот же parser локально.

## 2. Blocking preconditions после Task 5

Task 6 начинается только после принятия immutable Task 5A, Task 5B v6 and Task
5C v2 SHAs, one-build shared `PlatformConfigurationCatalogSetV1`, и
синхронизации active spec. Branch name, dirty diff, current HEAD или этот design
hash не заменяют три recorded acceptance SHA. Task 6 module mapping borrows the
same catalog set from `EvidenceExecutionContext`; it does not invoke Metadata or
Support adapters and does not parse a second registration catalog.
Дополнительно Task 5 application contract нужно поправить в двух местах.

### 2.1 ProviderGap сохраняет source identity и query-wide gap

Task 5A теперь вводит общий `SourceScopedArtifact` для support-запроса
и scoped provider gaps. Task 6 обязан потреблять именно этот тип, а не
возвращать `ArtifactRef`-only scope. Иначе одинаковый canonical ref в
analysis и destination source sets снова станет неразличимым.

Закрытый контракт также должен выражать:

- file/byte/result bound в explore-запросе без `knownArtifacts`;
- malformed BSL в source file, для которого нет публичного artifact kind;
- root application modules, CommonForm modules и nested object commands,
  которым активная ArtifactRef grammar не даёт canonical ref.

До Task 6 тип должен стать закрытым:

```rust
pub(crate) enum ProviderGapScope {
    Artifacts(Vec<SourceScopedArtifact>), // non-empty, sorted, deduplicated
    QueryWide,
    SourceSetWide(AtomicSourceIdentityV2),// exact linked logical identity
}

pub(crate) struct ProviderGap {
    pub(crate) reason_code: String,
    pub(crate) scope: ProviderGapScope,
    pub(crate) location: Option<SourceLocation>,
}
```

`Artifacts` несёт exact source-set и artifact для каждого affected subject.
`SourceSetWide` несёт exact linked logical source identity, когда file/catalog gap
ещё не выразим через `ArtifactRef`; он не должен заражать sibling
destination source sets. Task 6 BSL providers создают artifact/source-wide
scopes с analysis source-set из exact query. `QueryWide` относится к точному
query данного port invocation по всем его source sets; request,
source fingerprint, port identity и provider outcome snapshot уже участвуют в
`analysisId`. Все три варианта получают разные stable tags; уже
принятые tags `Artifacts=1`, `QueryWide=2` не сдвигаются, а
`SourceSetWide=3`. Пустой
`Artifacts` запрещён.

### 2.2 Task 5 BSL context остаётся отдельным от mutation context

`contract::ExecutionContext` — публичная mutation grammar с четырьмя значениями.
BSL дополнительно имеет `&НаКлиентеНаСервере`/`&AtClientAtServer`, а отсутствие
директивы означает module default, не безусловный server.

Task 5A вводит отдельный internal enum; Task 6 его переиспользует без
второго registry и без изменения stable tags:

```rust
pub(crate) enum BslExecutionContext {
    ModuleDefault,
    AtServer,
    AtClient,
    AtServerNoContext,
    AtClientAtServer,
    AtClientAtServerNoContext,
}
```

`DefinitionShape` и `ProviderFact::Call` используют его. Публичный CFE
`ExecutionContext` и его четыре wire spelling не меняются. Task 5 callback
compatibility интерпретирует `ModuleDefault` вместе с exact module kind:
ObjectModule default допустим для canonical server object callback;
CommonCommand callback требует explicit `AtClient`.

### 2.3 Общий catalog обязателен

Task 4 snapshot selection, Task 5 XML providers и Task 6 BSL module mapping
должны потреблять один `PlatformConfigurationCatalogV1`. Task 6 не реализует
второй parser `Configuration.xml`, не угадывает registration по directory name
и не принимает unregistered BSL file.

## 3. Exact file map

### Application contract

- Modify: `crates/unica-coder/src/application/discovery/model.rs`
  - `BslExecutionContext`;
  - `DefinitionShape.is_async` и BSL context;
  - target-less unresolved/dynamic call representation;
- inherited `ProviderGapScope`/`SourceScopedArtifact` validation for new BSL
  gap producers.
- Modify: `crates/unica-coder/src/application/discovery/ports.rs`
  - exact `CodeSearchQuery`, `DefinitionQuery`, `CallGraphQuery`;
  - query-specific signatures трёх BSL ports;
  - validation/canonicalization новых gap scopes.
- Modify: `crates/unica-coder/src/application/discovery/determinism.rs`
  - domain-separated stable encoding новых closed variants;
  - async/context/call target/gap scope входят в record/outcome digest.
- Modify: `crates/unica-coder/src/application/discovery/evidence_graph.rs`
  - flow edge только для `Resolved` call с canonical target;
  - две definitions одного canonical method — конфликт даже при одинаковой
    signature.
- Modify: `crates/unica-coder/src/application/discovery/use_case.rs`
  - временный initial query construction; Task 7 заменит его staged traversal;
  - никаких concrete parser/service imports.
- Modify: `crates/unica-coder/src/application/discovery/mod.rs`
  - exhaustive tags, constructors, collision and graph tests.

### Pure parser and providers

Вместо одного растущего `bsl.rs` создать focused module tree:

- Create: `crates/unica-coder/src/infrastructure/discovery/bsl/mod.rs`
  - provider structs, query execution, bounds/outcome aggregation.
- Create: `crates/unica-coder/src/infrastructure/discovery/bsl/lexer.rs`
  - strict UTF-8/BOM decoding, trivia masking, tokens and exact locations.
- Create: `crates/unica-coder/src/infrastructure/discovery/bsl/parser.rs`
  - definitions, method ranges, call syntax, conditional/dynamic gaps;
  - pure bounded
    `parse_bsl_file(bytes, limits, deadline) -> Result<BslFileAnalysis, BslFileFailure>`.
- Create: `crates/unica-coder/src/infrastructure/discovery/bsl/module_catalog.rs`
  - manifest path <-> canonical module mapping through Task 5 shared catalog.
- Create: `crates/unica-coder/src/infrastructure/discovery/bsl/cache.rs`
  - strict serializable parser-cache DTO v2 and validation, no I/O.
- Create: `crates/unica-coder/src/infrastructure/discovery/mod.rs` (or modify it
  if Task 5B creates it first)
  - private `mod bsl` and construction exports required by later wiring.
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
  - declare the private `discovery` module; do not broaden public exports.
- Modify only if Task 5 has not already exposed the shared catalog:
  `crates/unica-coder/src/infrastructure/platform_xml.rs`.

### Workspace acceleration

- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
  - `SERVICE_SCHEMA_VERSION = 2`;
  - typed `BslDiscoveryCacheV2` request/response;
  - bounded content-addressed cache using Task 6A parser;
  - existing BslMcp display response remains separate.
- Do not modify: `crates/unica-coder/src/infrastructure/workspace_index.rs`.
- Do not modify: public tool list/schema or `interfaces/mcp.rs`.

### Fixtures and contract tests

- Create: `tests/fixtures/project_discovery/bsl/definitions-ru/**`
- Create: `tests/fixtures/project_discovery/bsl/definitions-en/**`
- Create: `tests/fixtures/project_discovery/bsl/lexical-decoys/**`
- Create: `tests/fixtures/project_discovery/bsl/static-calls/**`
- Create: `tests/fixtures/project_discovery/bsl/dynamic-and-conditional/**`
- Create: `tests/fixtures/project_discovery/bsl/duplicate-definitions/**`
- Create: `tests/fixtures/project_discovery/bsl/unsupported-module-paths/**`
- Modify: `tests/ci/test_product_contracts.py`
  - forbid discovery imports of workspace index/SQLite/display parsers;
  - keep public discovery tool unregistered at this stage.
- Modify: `spec/architecture/extension-point-discovery.md`
  - duplicate the accepted Task 6A/6B source, grammar, bounds, outcome and cache
    contract in the active architecture layer in the same Task 6A/6B commits.
- Modify: `docs/superpowers/plans/2026-07-17-project-discovery-receipts.md`
  - mark Task 6 Steps 3-4 as superseded: snapshot parser is authoritative and
    service v2 is acceleration only. Do not leave an executable contradictory
    live-service-primary instruction behind.

## 4. Internal query contract

Общий `DiscoveryQueryPlan` слишком широкий для conclusive empty batch. Каждый
provider должен получить точный scope, для которого `Complete(empty)` имеет
однозначный смысл.

```rust
pub(crate) struct CodeSearchQuery {
    pub(crate) analysis_source: ResolvedSourceSet,
    pub(crate) analysis_configuration_catalog_digest: String,
    pub(crate) terms: Vec<String>,       // exact request.searchTerms only
    pub(crate) max_records: u16,         // physical records, whole v2 groups
}

pub(crate) struct DefinitionQuery {
    pub(crate) analysis_source: ResolvedSourceSet,
    pub(crate) analysis_configuration_catalog_digest: String,
    pub(crate) methods: Vec<ArtifactRef>, // method only, sorted unique
    pub(crate) max_records: u16,
}

pub(crate) struct CallGraphQuery {
    pub(crate) analysis_source: ResolvedSourceSet,
    pub(crate) analysis_configuration_catalog_digest: String,
    pub(crate) callers: Vec<ArtifactRef>, // method only, sorted unique
    pub(crate) max_records: u16,
}
```

```text
BSL_PROVIDER_QUERY_ENCODER = "snapshot-bsl-provider-query/v2"
```

Each query digest is `H("unica.snapshot-bsl-provider-query/v2", payload)` using
the Task 5B v6 primitives. Payload is the u16 port tag, exact Analysis
`AtomicSourceIdentityV2` (role plus all five `ResolvedSourceSet` logical fields),
`fingerprint32(snapshot.analysis.source_fingerprint)`, the canonical typed
`digest32(analysis_configuration_catalog_digest)` from the exact borrowed
`PlatformConfigurationCatalogSetV1`, the canonical typed terms/methods/callers
vector, and u16 `max_records`. Every ArtifactRef is
`ArtifactIdentityBytesV1 = kind stable tag + Unicode-lowercase canonical ref`,
exactly matching equality/hash; source spelling never becomes a second query
identity. Empty vectors encode count zero. Input order, JSON/debug text and
workspace epoch never enter the digest. Fingerprint binds freshness/query cache
but is excluded from `AtomicSourceIdentityV2`, semantic group keys and group
ordering.

Rules:

- `analysis_source` mismatch on any logical identity field returns retryable
  `Unavailable(source_set_mismatch)`
  before any read.
- catalog digest mismatch against the borrowed analysis catalog is a contract
  violation before any read; Task 6 never reconstructs the catalog.
- `terms` preserve caller text. Task 6 does not search semantic `concepts` and
  does not add synonyms. Task 7 may derive exact identifiers from typed
  artifacts, but only in application code and visibly.
- Empty query returns `Complete(empty)` with zero reads for its exact empty
  scope.
- `DefinitionQuery` contains method refs from explicit proposals,
  `knownArtifacts`, and later typed callback/binding targets.
- `CallGraphQuery` is **one-hop outgoing**. Task 7 owns traversal and invokes it
  with the next canonical frontier up to `maxGraphDepth`; infrastructure does
  not select roots or an architecture.
- `CodeSearchQuery.terms` contains `0..=128` exact, nonblank, unique values of
  at most 256 UTF-8 bytes, sorted by exact UTF-8 bytes. Case-only variants stay
  distinct because `search_term` preserves the request value.
- `methods` and `callers` contain `0..=2000` exact method ArtifactRefs, sorted
  and deduplicated by `ArtifactRef` identity. `max_records` is `1..=2000`.
- `max_records` is part of the exact typed query digest and never means “take
  this many records first”. The provider completes semantic parsing,
  classification and physical-record deduplication, then applies the shared v2
  whole-group prefix-stop from Task 5B section 6.1.
- A forged query with a wrong kind, invalid cardinality/order/identity or a
  `ResolvedSourceSet` different from the captured analysis source is a provider
  contract violation.

The normative BSL query golden imports the Task 5B v6 Analysis source fixture
(`mapping_digest="sha256:" + "a"*64`) and fixes:

```text
port = DefinitionPort(3)
snapshot.analysis.source_fingerprint = "sha256:" + ("b" * 64)
analysis_configuration_catalog_digest = "c" * 64
methods = []
max_records = 7
payload length = 220
SHA-256(payload) =
  3d363a007dacb05ffaeabf40ce645e793979b8b5f5391e86224e9fd79582b709
H("unica.snapshot-bsl-provider-query/v2", payload) =
  78cc2f7fa751f7e5c52c669e668c2031abcbc6919ce137fe6fc4f1d41329a0cc
```

CodeSearch/CallGraph use their own port tag and typed vector with the same field
order. The fixed value changes only with `BSL_PROVIDER_QUERY_ENCODER`.

Port signatures become:

```rust
trait CodeSearchPort {
    fn search(&self, query: &CodeSearchQuery,
              context: &EvidenceExecutionContext<'_>)
        -> ProviderOutcome<EvidenceRecord>;
}
trait DefinitionPort {
    fn definitions(&self, query: &DefinitionQuery,
                   context: &EvidenceExecutionContext<'_>)
        -> ProviderOutcome<EvidenceRecord>;
}
trait CallGraphPort {
    fn calls(&self, query: &CallGraphQuery,
             context: &EvidenceExecutionContext<'_>)
        -> ProviderOutcome<EvidenceRecord>;
}
```

## 5. Snapshot-byte source of truth

For every provider invocation:

1. Validate exact query and analysis source-set identity.
2. Enumerate only `ManifestEntry::Present` paths whose final extension is
   ASCII-case-insensitive `.bsl`, canonically sorted by workspace-relative
   UTF-8 bytes. Platform-owned path segments are then mapped by the exact
   shared-catalog rules in section 10; two paths that case-fold to one platform
   module identity are a bounded `ambiguous_bsl_module_path`, never silently
   deduplicated.
3. Intersect with the query scope:
   - CodeSearch: all manifest BSL files;
   - Definition: only exact target module files;
   - CallGraph: exact caller module files, then exact registered common-module
     files referenced by syntactically static qualified calls.
4. Read accepted files only through `read_verified(snapshot.analysis, path)`.
5. Verify returned byte length and SHA-256 against the manifest entry even
   though the concrete reader already does so; forged fake ports remain caught.
6. Obtain `BslFileAnalysis` from a validated v2 cache entry or run the same
   `parse_bsl_file()` locally.
7. Map syntax facts to canonical ArtifactRefs and construct all typed
   `EvidenceRecord`s with the exact analysis `AtomicSourceIdentityV2`, source
   fingerprint and diagnostic workspace epoch. Classify every physical record
   through Task 5B v6's shared v2 atomic encoder, validate complete groups,
   sort/deduplicate records inside groups, then apply the query's local
   whole-group `max_records` prefix-stop. No individual record is dropped before
classification.

The exact Task 5B v6 identity goldens are imported, not copied into a local
encoder. Changing only the captured source fingerprint must change the query
digest, physical-record digest, evidence ID and analysis ID, while preserving
`ResolvedSourceSetIdentityBytesV1`, `AtomicSourceIdentityV2`, semantic group-key
bytes and per-port/global group order. Changing only kind, source format,
relative root or mapping digest must change source identity and group key even
when the display source name is equal. Case-equivalent ArtifactRefs must produce
identical query/group bytes, including the Unicode-expanding lowercase golden.

For a reverse-mapped exact Definition/Call target, absence of its expected
module path from the manifest is conclusive only because Task 4 captured the
complete registered subtree selected by the same catalog. Definition may then
emit `DefinitionAbsent` without a file read. An unregistered owner, incomplete
catalog, unsupported module identity or ambiguous case-folded path is bounded,
not absent.

Any `SourceFingerprintMismatch` or read-time snapshot uncertainty discards the
whole invocation prefix and returns retryable unavailable. A provider must not
combine records read before a mismatch with records read after it.

Provider identities are stable regardless of cache venue:

| Port | name | version |
| --- | --- | --- |
| CodeSearchPort | `unica.snapshot_bsl_search` | `2` |
| DefinitionPort | `unica.snapshot_bsl_definitions` | `2` |
| CallGraphPort | `unica.snapshot_bsl_calls` | `2` |

The pure parser contract remains `unica.snapshot-bsl.v1` because its syntax
output did not change. Provider versions change because result admission,
outcome gaps and digests did. Cache entries contain parser analysis only and do
not cache provider grouping or a prior query ceiling.

## 6. Exact supported BSL lexical grammar

The v1 lexer is deliberately smaller than a full compiler but must be exact for
what it accepts. It follows the primary ANTLR grammar's case-insensitive
Russian/English keywords, Unicode identifiers, procedure/function declarations,
parameter form and call syntax. Anything outside the closed subset is a typed
gap, never guessed evidence.

### 6.1 Encoding and locations

- Strict UTF-8; one optional leading UTF-8 BOM is accepted and removed.
- Invalid UTF-8 or a second/interior BOM: `unsupported_bsl_encoding` scoped to
  the module/query.
- Line and column are 1-based Unicode scalar positions; a tab counts as one
  scalar. CRLF is one newline; bare CR and LF are also newlines.
- Definition location is the method-name token. Call location is the callee
  token. Code occurrence location is the first matched token.

### 6.2 Tokens hidden from search/call detection

- `//` through line end;
- date literal `'...'`;
- string literal delimited by `"`, with doubled `""` escape;
- multiline string continuation using `|` until the closing quote;
- whitespace;
- content inside `#Удаление/#Delete` through
  `#КонецУдаления/#EndDelete`.

Unterminated string/date/delete block makes that file atomic-invalid for the
affected capability: discard its facts and add `malformed_bsl_syntax`.

### 6.3 Case-insensitive closed keywords

Required paired spellings:

| Russian | English |
| --- | --- |
| `Процедура` | `Procedure` |
| `Функция` | `Function` |
| `КонецПроцедуры` | `EndProcedure` |
| `КонецФункции` | `EndFunction` |
| `Экспорт` | `Export` |
| `Знач` | `Val` |
| `Асинх` | `Async` |
| `Выполнить` | `Execute` |
| `Ждать` | `Await` |
| `Перем` | `Var` |
| `Для` | `For` |
| `Каждого` | `Each` |
| `Из` | `In` |
| `По` | `To` |
| `Цикл` | `Do` |
| `КонецЦикла` | `EndDo` |

Identifiers follow Unicode letter or `_`, followed by Unicode letters, digits
or `_`. Keywords are case-insensitive. Mixed Russian/English keyword forms are
accepted because the language grammar treats each spelling as the same token.

### 6.4 Compiler directives

Recognize exact paired spellings:

- `&НаСервере` / `&AtServer`;
- `&НаКлиенте` / `&AtClient`;
- `&НаСервереБезКонтекста` / `&AtServerNoContext`;
- `&НаКлиентеНаСервере` / `&AtClientAtServer`;
- `&НаКлиентеНаСервереБезКонтекста` /
  `&AtClientAtServerNoContext`.

No directive means `ModuleDefault`. Exactly zero or one recognized context may
apply to a declaration; repeated or conflicting contexts make that declaration
malformed. An otherwise lexically valid unknown annotation and balanced
argument list are retained as `unsupported_bsl_annotation` on the affected
definition/caller. They never become execution context and are never silently
treated as ordinary runtime semantics.

### 6.5 Preprocessor

- `#Область/#Region`, `#КонецОбласти/#EndRegion`,
  `#Вставка/#Insert`, `#КонецВставки/#EndInsert` are structural markers;
  active content is parsed.
- `#Удаление/#Delete` content is excluded.
- Any conditional branch (`#Если/#If`, `#ИначеЕсли/#ElsIf`, `#Иначе/#Else`,
  `#КонецЕсли/#EndIf`) is tokenized to find its balanced extent. Definitions and
  calls inside it are not promoted as positive facts, but symbol effects are not
  discarded. Collect canonical `maybe_definition_names`,
  `maybe_module_shadow_names`, and per-method `maybe_local_shadow_names` from
  every branch, including nested branches, and union all alternatives.
- A conditional same-module definition makes every unconditional direct call to
  that name Ambiguous/Unknown. A conditional exported definition in a registered
  CommonModule similarly blocks a qualified positive resolution from another
  module. Conditional parameter/Var/assignment/For/For Each binders make the
  matching receiver Dynamic in their containing method/module even when the call
  token is outside the conditional extent. The conservative effect is
  scope-wide, not source-order dependent.
- An unconditional definition/call outside the extent remains usable only when
  its resolution does not intersect any maybe-defined/maybe-shadowed set. Every
  intersection receives exact `conditional_compilation_unknown`; no runtime edge
  or negative absence follows. CodeSearch may retain the literal occurrence, but
  that scoped gap prevents it from proving runtime reachability.
- Other directives add `unsupported_bsl_preprocessor`; no negative fact may be
  emitted for their affected scope.

### 6.6 Async lexer mode and unsupported tokens

The handwritten lexer has an explicit closed state machine:

```text
Default
  -- Async + Procedure --> AsyncProcedureBody(expected EndProcedure)
  -- Async + Function  --> AsyncFunctionBody(expected EndFunction)
```

`Async` is a declaration modifier only when followed by the matching declaration
header. The body mode exits only at a matching terminator at declaration/body
delimiter depth zero and outside deleted/conditional-preprocessor ambiguity.
Russian/English spellings may mix when the semantic terminator kind matches;
Procedure closed by EndFunction or Function by EndProcedure is malformed.
Nested declaration starts, unbalanced arguments/body delimiters, an ambiguous
terminator across preprocessor branches, or EOF while in async mode yields
`MalformedSyntax` for Definition/Call capability with no partial file facts.
`Await/Ждать` is recognized as async-body syntax only in that mode; outside it
creates an exact unsupported-token capability gap rather than being hidden.

Every nontrivia source byte must belong to one closed significant token,
punctuation token, literal, comment, preprocessor token, or explicit unsupported
token. The reference grammar's catch-all UNKNOWN/HIDDEN behavior is not copied.
An unsupported token produces `unsupported_bsl_token` for the containing
module/method and every capability whose parse could change; it is never skipped
before Definition absence or Call resolution. Raw attacker bytes stay in the
bounded source span, never the reason string.

### 6.7 Pure parser output contract

`parse_bsl_file()` has no path, manifest, catalog, evidence or service access.
It returns syntax only:

```rust
struct BslFileAnalysis {
    significant_tokens: Vec<BslSignificantToken>,
    definitions: Vec<BslSyntaxDefinition>,
    maybe_definitions: Vec<BslMaybeDefinition>,
    calls: Vec<BslSyntaxCall>,
    module_shadow_names: Vec<String>,
    maybe_module_shadow_names: Vec<String>,
    gaps: Vec<BslSyntaxGap>,
}

struct BslSignificantToken {
    class: BslTokenClass,
    comparison_text: String,
    span: BslSpan,
    inside_conditional: bool,
}

struct BslSpan {
    start_byte: u32,
    end_byte_exclusive: u32,
    line: u32,
    column: u32,
}

struct BslSyntaxDefinition {
    name: String,
    name_span: BslSpan,
    body_span: BslSpan,
    shape: DefinitionShape,
    local_shadow_names: Vec<String>,
    maybe_local_shadow_names: Vec<String>,
}

struct BslMaybeDefinition {
    name: String,
    may_be_exported: bool,
    span: BslSpan,
}

struct BslSyntaxCall {
    caller_definition_index: u32,
    receiver: Option<String>,
    callee: Option<String>,
    syntax: BslCallSyntax,
    callee_span: BslSpan,
}

struct BslSyntaxGap {
    reason: BslSyntaxGapReason,
    capability: BslCapability,
    method_name: Option<String>,
    span: BslSpan,
}

enum BslTokenClass {
    Identifier,
    Keyword,
    Number,
    Boolean,
    UndefinedOrNull,
    Punctuation,
    Unsupported,
}

enum BslCallSyntax { Direct, Qualified, Execute, AccessChain, Unsupported }
enum BslCapability { Search, Definition, Call, All }
enum BslSyntaxGapReason {
    UnsupportedAnnotation,
    UnsupportedPreprocessor,
    ConditionalCompilation,
    ConditionalSymbolEffect,
    UnsupportedToken,
    UnsupportedShadowBinder,
    UnsupportedCallSyntax,
    ModuleLevelCall,
}

enum BslFileFailure {
    UnsupportedEncoding,
    MalformedSyntax,
    FileBytesLimit,
    TokenLimit,
    NestingLimit,
    DefinitionLimit,
    CallLimit,
    Deadline,
}
```

`comparison_text` is Unicode-lowercase for identifier, keyword, Boolean and
UndefinedOrNull tokens and exact source spelling for number/punctuation tokens.
Unsupported uses the fixed empty comparison string and its bounded span; it
never embeds raw attacker text. The vector never contains comments, strings,
dates, deleted text or full source bytes.
Parser-level gap/failure enums map exhaustively to section 11 reason codes;
arbitrary parser strings never cross the cache boundary. A `BslFileFailure`
returns no partial analysis. `Deadline` aborts the entire provider invocation;
all other file failures are deterministic bounded gaps for that file/scope.

Definitions/calls with conditional or unsupported semantic annotations are not
placed in the positive vectors; conditional definitions and binders also enter
the explicit maybe sets before the tokens are discarded. Definite/maybe shadow
and definition names are Unicode-lowercase, sorted and unique. Vectors are
canonical: tokens/calls by byte span, definitions/maybe-definitions by identity
then name span, gaps by reason/capability/method/span.
`validate_against(byte_len)` checks every span, index, closed variant,
identifier/list/count bound, set disjointness where required and canonical
order. It does
not claim semantic correctness of an untrusted producer; the same-binary trust
restriction in section 12 still applies.

## 7. Definition grammar and facts

Accepted declaration:

```text
[annotations/directives]* [Async]
(Procedure|Function) Identifier "(" [Parameter ("," Parameter)*] ")" [Export]
... matching body ...
(EndProcedure|EndFunction)

Parameter := [annotations]* [Val] Identifier ["=" ConstLiteral]
```

`ConstLiteral` is limited to the lexer's numeric, string, date, boolean,
undefined/null forms with optional sign where the language grammar permits it.
Nested declarations, mismatched terminators, duplicate parameter names,
unbalanced delimiters or non-constant defaults make the file malformed for
definitions.

`DefinitionShape` is exactly:

```rust
pub(crate) struct DefinitionShape {
    pub(crate) is_function: bool,
    pub(crate) is_async: bool,
    pub(crate) exported: bool,
    pub(crate) parameters: Vec<DefinitionParameter>,
    pub(crate) context: BslExecutionContext,
}
```

Rules:

- method ref is `<canonical module ref>.<source spelling of method>`;
- identity is Unicode-lowercase through existing `ArtifactRef` equality;
- two declarations with the same canonical module+method are retained with
  distinct locations and create `duplicate_definition` conflict even when
  shapes are byte-identical;
- different shapes additionally create `conflicting_definition_shapes`;
- `DefinitionAbsent` is emitted only for an exact queried method when its
  registered module scope is complete, fully parsed, within all bounds, and no
  matching declaration exists;
- no absent fact is emitted for a target touched by any gap.

Async is part of the definition digest. Task 5 canonical callbacks are sync;
an async variation is `unsupported_callback_signature_variant`, not silently
the same definition.

### 7.1 Definition observations and Task 5 runtime joins

Task 6 produces `DefinitionShape`; it never promotes a declarative handler or
copies declarative runtime context into BSL context. The application join imports
the accepted Task 5A/5B v6 decision tables exactly:

| Requirement | Declarative runtime context | Supported BSL Definition |
| --- | --- | --- |
| EventSubscription | `SameAsSourceEvent` | exported synchronous Procedure, exact arity from the one descriptor signature class, `ModuleDefault` |
| predefined ScheduledJob | `Server` | exported synchronous zero-arity Procedure **or Function**, `ModuleDefault` |
| HTTPService route | `Server` | synchronous one-parameter by-reference nondefaulted Function, `ModuleDefault`; Export nonmaterial |
| Form command | `Client` | one-parameter by-reference nondefaulted Procedure, explicit `AtClient`, sync or async; Export nonmaterial |

EventSubscription arity is supplied only by the complete Task 5B v6 descriptor
after all selected event-source families agree on its signature class. Task 6
does not reconstruct that class. A wrong Event kind/export/arity is No; an
otherwise exact async or explicit BSL context is the versioned unsupported
Unknown. ScheduledJob is queried only for exact Use=true, Predefined=true,
supported module profile; Predefined=false has no Definition endpoint. Wrong
Scheduled kind/export/arity is No, while async or explicit BSL context is
Unknown.

`BindingRuntimeContextV1::Server` for ScheduledJob/HTTP is **not**
`BslExecutionContext::AtServer`; their supported primary rows are unannotated
`ModuleDefault`. EventSubscription `SameAsSourceEvent` is likewise not AtServer.
HTTP/Form parameter transfer/default and async rows follow their exact Task 5B
policies above. Missing/gapped Definition remains Unknown. Only EvidenceGraph
owns these joins and creates runtime edges after a Yes; Task 6 returns all exact
shapes, including incompatible/unsupported ones, as observations.

Task 6 v2 design may be conditionally reviewed before implementation, but no
production acceptance/freeze claim is valid until immutable accepted Task 5A
and Task 5B v6 SHAs are recorded and these rows are byte-for-byte synchronized
with code/tests/active spec.

## 8. Search-term grammar

CodeSearch uses exactly `request.searchTerms`; it does not search `task` or
`concepts` and does not invent synonyms.

Each term is lexed in query mode into a non-empty sequence of significant
tokens. Matching is exact token-sequence matching:

- identifier, keyword, boolean and undefined/null comparison uses the one
  Unicode-lowercase `comparison_text` rule from section 6.7;
- punctuation and numeric comparison uses exact source spelling;
- arbitrary source whitespace is allowed between query tokens;
- comments, strings, date literals and deleted blocks cannot satisfy a term;
- one evidence record is emitted per match start, with `search_term` preserving
  the caller's exact string;
- subject is the containing method, otherwise the representable module.

A term that cannot be tokenized into the supported query grammar yields a
query-wide `unsupported_bsl_search_term` gap, not a substring guess.
In particular, query-mode string/date/comment tokens are rejected rather than
being called an exact literal match.

Case metamorphic fixtures require `TRUE/True`, `FALSE/False`,
`UNDEFINED/Undefined` and `NULL/Null` (plus Russian boolean/undefined spellings
where applicable) to match identically. There is no second “exact literal” rule
for these case-insensitive token classes.

## 9. Conservative static-call contract

Task 6 emits call observations only inside a completely parsed method body.
Module-level code never becomes a resolved runtime edge; a relevant occurrence
gets `unsupported_module_level_call`.

The v1 extractor recognizes only these balanced-token shapes after excluding a
declaration header: `Identifier(...)`, `Identifier.Identifier(...)`,
`Execute/Выполнить(...)`, and access/call chains containing one of those call
parentheses. Arguments are balanced but not semantically evaluated. A direct
or qualified call must not be immediately part of a longer access chain;
computed/indexed receivers and a call result used as the next receiver are
Dynamic. Unknown parenthesized language constructs are ignored only when their
leading token is a closed control/operator keyword; otherwise they create an
`unsupported_bsl_call_syntax` caller gap.

### 9.1 Target representation

Current mandatory `object: ArtifactRef` cannot represent a dynamic or missing
callee. Replace it with a closed target:

```rust
pub(crate) enum CallTarget {
    Artifact(ArtifactRef),
    Named(String),  // syntactically static spelling, no canonical definition
    Dynamic,
}
```

`Evidence.object` remains optional and is present only for `Artifact`.
`Named` is either one identifier (`Method`) or two dot-separated identifiers
(`CommonModule.Method`); each segment follows the existing 512-byte/128-scalar
identifier ceiling and the whole value is at most 1024 UTF-8 bytes. It
participates in the evidence digest. A dynamic call at a different source
column remains distinct through location.

### 9.2 Resolution table

| Syntax | Result |
| --- | --- |
| `Method(...)` with exactly one same-module definition | `Resolved`, `Direct`, canonical method target |
| `Method(...)` with duplicate same-module definitions | `Ambiguous`, `Direct`, candidate target; no flow edge |
| `Method(...)` intersecting a conditional maybe-definition | `Ambiguous`, `Direct`; `conditional_compilation_unknown`; no edge |
| `Method(...)` with no same-module definition | `Unresolved`, `Direct`, named target; scoped gap |
| unshadowed `CommonModuleName.Method(...)` with one registered exported definition | `Resolved`, `Method`, canonical CommonModule method |
| same qualified call with duplicate definitions | `Ambiguous`, `Method`; no edge |
| same qualified call with a conditional maybe-definition in the target module | `Ambiguous`, `Method`; `conditional_compilation_unknown`; no edge |
| same qualified call missing or non-exported | `Unresolved`, `Method`; no edge |
| registered common-module receiver shadowed or possibly assigned in caller/module scope | `Dynamic`, `Dynamic`; no edge |
| receiver other than an exact registered common-module identifier | `Dynamic`, `Dynamic`; no edge |
| chained/computed receiver, `Execute/Выполнить`, dynamic handler | `Dynamic`, `Dynamic`; no edge |

Nested calls are independently extracted. `Await/Ждать` does not change the
callee identity. Built-ins and globally exposed common-module functions are
not guessed: an unqualified name without a same-module definition is
`Unresolved` until a typed global-symbol catalog exists.

For qualified resolution, the closed shadow pass recognizes all binders that v2
accepts:

```text
formal parameter Identifier
Var/Перем Identifier ("," Identifier)*
Identifier "=" expression
For/Для Identifier "=" expression To/По expression Do/Цикл
For/Для Each/Каждого Identifier In/Из expression Do/Цикл
```

The same forms at module level populate `module_shadow_names` and affect every
procedure/function, regardless of textual order. Method forms populate
`local_shadow_names` for the complete method. Conditional occurrences populate
the corresponding maybe sets and have the same conservative scope-wide effect.

An assignment/access/index lvalue whose first identifier equals a registered
CommonModule name (`Name.Member=...`, `Name[...]=...`) proves that receiver is a
runtime value in that scope and also downgrades it to Dynamic; it is not treated
as a safe platform module qualifier. Any other accepted-language binder/lvalue
shape that the pass cannot classify creates `unsupported_bsl_shadow_analysis`
for the module/caller, and **all** qualified CommonModule-looking calls in that
scope stay Dynamic. The implementation may deliberately overapproximate
shadows, but it may not omit an accepted binder and still emit a resolved edge.

This deliberately permits false negatives; it forbids treating a local or
conditionally present object named like a CommonModule as that CommonModule.

A queried caller must have exactly one unconditional definition and no
same-name maybe-definition. Zero callers produce `missing_caller_definition`;
duplicate or conditional/maybe callers produce `ambiguous_caller_definition`.
In both cases the CallGraph result is bounded for that caller and emits no calls
from that body.

`ProviderFact::Call` carries caller subject, `CallTarget`, resolution, call
type and the caller definition's `BslExecutionContext`. EvidenceGraph creates
`FlowKind::Calls` only for `(Resolved, Artifact(target))`.

An unresolved/dynamic/ambiguous call makes only its queried caller scope
bounded. It does not poison unrelated methods or Definition/CodeSearch output.

## 10. Module path mapping

Strip the exact captured analysis `relative_root`, then accept only paths
proven by the shared registered catalog:

| Manifest-relative shape | Canonical module |
| --- | --- |
| `CommonModules/<N>/Ext/Module.bsl` | `CommonModule.<N>` |
| `<RegisteredDir>/<N>/Ext/<ModuleKind>.bsl` | `<Kind>.<N>.<ModuleKind>` |
| `<RegisteredDir>/<N>/Forms/<F>/Ext/Form/Module.bsl` | `<Kind>.<N>.Form.<F>.FormModule` |

`ModuleKind` is exactly `Module`, `ObjectModule`, `ManagerModule`,
`RecordSetModule`, `ValueManagerModule` or `CommandModule` from the shared
registry. Reverse mapping for Definition/Call queries uses the same catalog and
requires the owner/form registration.

The active public ArtifactRef grammar cannot represent these valid snapshot
paths:

- root `Ext/*ApplicationModule.bsl`, `SessionModule.bsl`,
  `ExternalConnectionModule.bsl`;
- `CommonForms/<N>/Ext/Form/Module.bsl`;
- nested object `Commands/<C>/Ext/Module.bsl`/`CommandModule.bsl`.

Task 6 must not fabricate refs for them and must not silently skip them.
CodeSearch receives query-wide `unsupported_bsl_module_identity` with the exact
path location. Definition/Call queries can remain complete only when none of
their exact target scopes depends on such a path. Expanding public ArtifactRef
is a later explicit architecture change, not a hidden Task 6 side effect.

## 11. Exact bounds and outcomes

Server-owned v1 constants:

```rust
const MAX_BSL_FILES: usize = 20_000;
const MAX_BSL_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const MAX_BSL_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_BSL_TOKENS_PER_FILE: usize = 1_000_000;
const MAX_BSL_NESTING: usize = 256;
const MAX_BSL_DEFINITIONS_PER_FILE: usize = 4_096;
const MAX_BSL_CALLS_PER_FILE: usize = 65_536;
const MAX_BSL_PROVIDER_GAPS: usize = 256;
const MAX_BSL_GAP_SUBJECTS: usize = 2_000;
const MAX_BSL_ELAPSED: Duration = Duration::from_secs(30);
```

The parser receives injectable limits/clock in tests. Production constructors
use the constants above.

BSL records use `SemanticAtomicGroupIdV2::StandaloneFact`, except when a later
accepted provider contract explicitly assigns a stronger closed group. Its
source is exact Analysis `AtomicSourceIdentityV2`. The fact family, subject,
optional relation/object and source-free typed-payload digest identify one
semantic group; all location/provider witnesses of that semantic value are
physical records inside it. Therefore two locations of the same resolved
caller-to-callee Call, or of one Definition shape, cannot be split by the local
ceiling.

`material_subjects` is exact and query-owned:

- CodeSearch: the source-qualified containing Method/module subject plus every
  conclusion scope associated with the exact search term;
- DefinitionPresent/DefinitionAbsent: the exact queried Method;
- Call: the exact queried caller plus the resolved Artifact target when present;
  Named/Dynamic targets add no fabricated ArtifactRef;
- conflict/companion observations required for one queried Definition/Call stay
  in the same semantic group or make classification a contract violation.

Groups and physical records use the Task 5B v6 section-6.1 golden encoder. The
local query digest includes `max_records`; changing only that ceiling therefore
cannot reuse a larger/smaller cached provider outcome. The workspace cache still
stores parser syntax only and has no admission result to reuse.

Deterministic resource handling:

- paths are canonical-sorted before counting;
- total file-count/total-byte bound stops before the first overflowing path and
  never skips it to inspect later paths;
- an individually oversized file is skipped atomically and later canonical
  paths may still be parsed; its scope remains gapped;
- per-file syntax bound discards that file's partial analysis atomically;
- all records are classified into complete v2 groups and physically
  deduplicated, then the whole-group canonical prefix fitting
  `query.max_records` is retained; the first group that does not fit and every
  later group are dropped, with no skip-and-continue;
- every deterministic truncation returns `Bounded` with exact scoped gaps;
- deadline returns retryable `Unavailable(bsl_provider_deadline)` with zero
  records.

Result-bound gap scope is fixed: CodeSearch uses `QueryWide`; Definition uses
the exact queried methods whose records/absence proofs were omitted; CallGraph
uses the exact queried callers whose observations were omitted. File/byte
bounds use exact affected method/caller artifacts when the query supplies them,
and `QueryWide` for all-source CodeSearch. No omitted scope may receive an
absence fact.

After file/language/call/result gaps are complete, canonicalize and deduplicate
the full vector. At `<=256` gaps and `<=2,000` distinct exact artifact subjects,
retain it. At 257 or 2,001 replace the entire vector with exactly one QueryWide
`bsl_gap_limit`; never append the sentinel to a retained arbitrary prefix.

Closed v1 provider-gap reason codes are:

| Family | Exact reason codes |
| --- | --- |
| path/material | `unsupported_bsl_module_identity`, `ambiguous_bsl_module_path`, `unsupported_bsl_encoding`, `malformed_bsl_syntax` |
| language subset | `unsupported_bsl_annotation`, `unsupported_bsl_preprocessor`, `conditional_compilation_unknown`, `unsupported_bsl_token`, `unsupported_bsl_shadow_analysis`, `unsupported_bsl_search_term`, `unsupported_bsl_call_syntax`, `unsupported_module_level_call` |
| call resolution | `dynamic_bsl_call`, `unresolved_bsl_call`, `ambiguous_bsl_call`, `missing_caller_definition`, `ambiguous_caller_definition` |
| deterministic resource | `bsl_file_limit`, `bsl_total_bytes_limit`, `bsl_file_bytes_limit`, `bsl_token_limit`, `bsl_nesting_limit`, `bsl_definition_limit`, `bsl_call_limit`, `bsl_result_limit`, `bsl_gap_limit` |

An implementation must use these codes rather than embedding paths, counts or
parser text in `reason_code`; exact path/line/column belongs in location and
exact artifacts belong in gap scope.

Reason-code matrix:

| Condition | Outcome | retryable | records |
| --- | --- | ---: | --- |
| exact complete query | Complete | false | all canonical records |
| unsupported module/encoding/preprocessor/malformed syntax | Bounded + scoped gap | false | only facts from complete unaffected files/scopes |
| dynamic/unresolved/ambiguous call in queried caller | Bounded + caller gap | false | safe observations, no runtime edge |
| file/byte/syntax/result deterministic bound | Bounded + scoped gap | false | deterministic safe prefix |
| elapsed safety deadline | Unavailable `bsl_provider_deadline` | true | none |
| source-set mismatch | Unavailable `source_set_mismatch` | true | none, zero reads |
| content/fingerprint changed during verified read | Unavailable `source_fingerprint_mismatch` | true | none |
| snapshot reader unavailable | Unavailable `source_snapshot_unavailable` | true | none |
| provider constructs impossible path/fact/freshness | ContractViolation | n/a | operation fails |
| workspace service/cache failure | no provider outcome change | n/a | local Task 6A result |

All independent gaps survive in canonically sorted `ProviderGap[]`; do not
collapse them to the first reason.

## 12. Workspace service schema v2

### 12.1 Separation from display MCP

Keep two unrelated request/response branches:

```text
BslMcp                  -> WorkspaceServiceBslOutput { result_text, stderr }
BslDiscoveryCacheV2     -> BslDiscoveryCacheResponseV2 { typed parser cache }
```

Discovery never calls `BslMcp`, `mcp_tool_text`, RLM readiness or SQLite.
`structuredContent` from bsl-analyzer is not receipt-grade in Task 6.

The provider depends only on an optional infrastructure-private acceleration
interface, not on workspace globals:

```rust
trait BslFileAnalysisCache: Send + Sync {
    fn analyze(
        &self,
        request: &BslDiscoveryCacheRequestV2,
    ) -> Result<BslDiscoveryCacheResponseV2, BslCacheError>;
}
```

`None`, `BslCacheError` and every rejected response all select the same local
parser. No application port or public tool is added for this interface.

### 12.2 Wire DTO

All structs use `deny_unknown_fields`, camelCase, canonical sorting and exact
schema/parser versions:

```rust
struct BslDiscoveryCacheRequestV2 {
    schema_version: u32,          // exactly 2
    parser_contract: String,      // exactly unica.snapshot-bsl.v1
    source_fingerprint: String,
    files: Vec<BslCacheFileRequestV2>,
}

struct BslCacheFileRequestV2 {
    source_relative_path: String,
    byte_length: u64,
    content_digest: String,
}

struct BslDiscoveryCacheResponseV2 {
    schema_version: u32,
    parser_contract: String,
    source_fingerprint: String,
    entries: Vec<BslCacheEntryV2>,
}

struct BslCacheEntryV2 {
    source_relative_path: String,
    byte_length: u64,
    observed_content_digest: String,
    analysis: BslFileAnalysis,
}
```

Request paths are contained slash paths relative to the service's exact
`source_root`; absolute, backslash, drive, empty, dot and traversal components
are rejected before I/O. Service reads with the existing no-follow contained
filesystem primitive, computes bytes/digest, and returns no entry on mismatch.
The client derives this path by stripping the captured
`analysis.source_set.relative_root` from the workspace-relative manifest key;
failure to strip exactly is a cache miss, never a second path interpretation.

### 12.3 Caller acceptance algorithm

A cache entry is accepted only when all are true:

1. workspace service record schema is 2 and exact package version/root identity
   still matches;
2. response schema/parser/source fingerprint are exact;
3. response entries are a canonical sorted subset of request files, with no
   duplicate, unrequested or extra fields; zero entries for a requested file is
   a legitimate miss;
4. path, length and digest equal the authoritative manifest entry;
5. caller independently obtains the bytes through `read_verified` and their
   digest/length equal the same manifest entry;
6. `BslFileAnalysis::validate_against(byte_len)` proves canonical order,
   bounded identifiers, valid spans and closed enum values.

The client sends canonical batches only after it has the verified bytes from
section 5. A missing entry runs `parse_bsl_file(verified_bytes)` for that file.
Any structural response failure (wrong envelope, order, duplicate or extra
entry) discards the whole batch; an entry-specific length/digest/analysis
failure discards that entry. Both cases fall back locally and are not provider
contract violations because acceleration is non-authoritative.

The v1 trust boundary assumes the token-authenticated loopback workspace
service is the same Unica binary. If that process is ever treated as untrusted,
digest matching alone cannot prove semantic completeness; bytes must be sent
to the caller parser or reparsed locally. Do not claim both an untrusted service
and a parse-speedup without that stronger protocol.

### 12.4 Cache and transport bounds

```rust
const MAX_BSL_CACHE_ENTRIES: usize = 4_096;
const MAX_BSL_CACHE_SOURCE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_BSL_CACHE_BATCH_FILES: usize = 128;
const MAX_BSL_CACHE_BATCH_SOURCE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_WORKSPACE_SERVICE_FRAME_BYTES: usize = 64 * 1024 * 1024;
```

Cache key is `(parser_contract, content_digest)`, not path or mtime. FIFO/LRU
eviction is allowed because it affects performance only. Oversized request,
entry or response produces a cache error and local parse. Internal TCP framing
is bounded before JSON allocation; this does not change Task 12's future public
stdio 4 MiB limit.

The caller partitions the already canonical file list into stable consecutive
batches satisfying both batch bounds. Cache-capacity/source-byte ceilings may
change only hit rate; they never truncate provider files or evidence.

Semantic cache equivalence is asserted for the same snapshot, deterministic
limits and a non-expired injected deadline. A real accelerator and a real
wall-clock abort cannot guarantee identical availability at the exact deadline
boundary: a hit can finish where a local parse times out. In either schedule a
deadline returns zero records and cannot issue negative proof. If product
policy instead requires hit/miss-identical outcomes under every adversarial
clock schedule, Task 6B must stop or reparse locally (which removes the claimed
parse acceleration); do not conceal that trade-off in tests.

## 13. RED -> GREEN implementation sequence

### Task 6A.1: Close application types before parser code

- [ ] Add RED tests:
  - inherited `ProviderGap` accepts `QueryWide` without a fake artifact,
    preserves `SourceSetWide` without contaminating sibling source sets, and
    source-scoped gaps for equal refs in different source sets stay distinct;
  - `bsl_execution_context_tags_are_closed_and_collision_free`;
  - `binding_runtime_context_never_converts_to_bsl_execution_context`;
  - table-driven Task 5B v6 Event/Scheduled/Form/HTTP Definition joins match
    section 7.1, including SameAsSourceEvent/Server versus ModuleDefault and
    Client versus AtClient;
  - `definition_digest_changes_for_async_and_each_context`;
  - `targetless_call_never_creates_flow_edge`;
  - `duplicate_identical_definition_is_a_conflict`;
  - exact query constructors reject wrong kinds/order/duplicates;
  - BSL query golden bytes/digest change for only `max_records` or the borrowed
    analysis-catalog digest and are stable under equivalent input permutations;
  - fingerprint-only recapture changes query/physical/evidence identity but not
    logical source/group order; equal source names with different full logical
    fields remain distinct;
  - mixed-case/Unicode case-equivalent ArtifactRefs import the exact Task 5B v6
    `ArtifactIdentityBytesV1` goldens and cannot change query/group bytes;
  - Task 6 imports the one `SemanticAtomicGroupIdV2` encoder/registry and cannot
    declare a local copy or change its golden bytes.
- [ ] Run:
  `cargo test --locked -p unica-coder application::discovery -- --nocapture`
  and record RED for missing types/invariants.
- [ ] Implement model/ports/determinism/evidence-graph changes only.
- [ ] Re-run the same command; expected GREEN.
- [ ] Review gate: public CFE `ExecutionContext` wire fixtures must be unchanged.

### Task 6A.2: Pure lexer/parser

- [ ] Add RU/EN fixtures and RED tests for:
  - procedure/function, sync/async, export, Val/default, mixed keyword language;
  - all six BSL contexts;
  - comments, single/multiline strings and date literals as decoys;
  - direct/nested/qualified calls and exact line/column;
  - qualified CommonModule call shadowed by parameter/Var/assignment stays
    Dynamic;
  - deleted/insert/conditional preprocessor blocks;
  - conditional definition/call never becomes positive runtime evidence;
  - conditional module Var plus unconditional qualified call remains Dynamic;
  - conditional local Var, bare assignment, For and For Each binder plus an
    unconditional qualified call remains Dynamic;
  - conditional same-module definition plus unconditional direct call is
    Ambiguous, and conditional exported CommonModule definition blocks an
    external qualified resolution;
  - nested conditional branches union every maybe-shadow path;
  - module-level Var/bare/access/index assignment shadows qualified receivers in
    every method; unclassified binder/lvalue gives
    `unsupported_bsl_shadow_analysis`, never a resolved edge;
  - TRUE/True, FALSE/False, UNDEFINED/Undefined and NULL/Null query/source case
    permutations compare identically;
  - async Procedure/Function exits only on matching semantic terminator;
    mixed-language matching terminators pass, mismatched kind, EOF, nested
    declaration, delimiter and preprocessor-boundary ambiguity fail atomically;
  - every unsupported nontrivia token creates `unsupported_bsl_token` for the
    affected capability and is never hidden;
  - malformed terminators, invalid UTF-8, nesting/token/definition/call bounds.
- [ ] Run:
  `cargo test --locked -p unica-coder infrastructure::discovery::bsl:: -- --nocapture`
  and record RED because parser modules do not exist.
- [ ] Implement lexer, then parser. No provider/service I/O in this slice.
- [ ] Re-run; expected GREEN including exact boundary and boundary+1 cases.

### Task 6A.3: Snapshot providers and module catalog

- [ ] Add RED tests:
  - only manifest-listed BSL is read;
  - source mismatch performs zero reads;
  - CommonModule/owner/form paths map in both directions;
  - unsupported root/CommonForm/nested-command paths produce gaps;
  - exact DefinitionAbsent only under complete target module scope;
  - stale read discards the entire invocation prefix;
  - static call resolution follows section 9 and never guesses object methods;
  - conditional exported target in a registered CommonModule makes an otherwise
    exact cross-module call Ambiguous/Unknown;
  - service/index/SQLite are not required;
  - two locations for one semantic resolved Call/Definition form one v2 group;
    `max_records=1` drops both rather than returning one witness, under both
    source/insertion orders;
  - an earlier two-record group that does not fit prefix-stops before a later
    one-record group and gaps both exact query scopes;
  - 256/257 gaps and 2,000/2,001 exact subjects retain the exact vector or the
    one QueryWide `bsl_gap_limit` sentinel.
- [ ] Implement provider orchestration over a fake snapshot reader first, then
  one real `FilesystemSourceSnapshots` integration fixture.
- [ ] Run:
  `cargo test --locked -p unica-coder discovery::bsl -- --nocapture`.
- [ ] Add exact file/byte/result/deadline determinism tests with injected limits
  and clock; re-run expected GREEN.

### Task 6B.1: Service v2 cache

- [ ] Add RED tests:
  - schema-v1 record is not reusable after bump;
  - strict v2 DTO rejects unknown/duplicate/out-of-order/escaped paths;
  - exact digest hit returns Task 6A analysis;
  - stale digest, wrong parser version, extra/missing entry and service failure
    all fall back locally;
  - with a non-expired injected clock, cache hit, miss and unavailable service
    produce byte-identical `ProviderOutcome`, evidence IDs and
    provider-outcome digests;
  - changed live service bytes cannot be accepted for snapshot bytes;
  - cache eviction and frame bounds affect only hit rate.
  - canonical batch partitioning at file/byte N and N+1 and subset cache hits.
- [ ] Run:
  `cargo test --locked -p unica-coder infrastructure::workspace_services::tests::discovery_bsl -- --nocapture`
  and record RED.
- [ ] Bump service schema, add cache request/response branch and reuse the pure
  parser. Do not alter display BslMcp parsing.
- [ ] Re-run service tests and complete Task 6A suite; expected GREEN.
- [ ] Update the active spec with the accepted Task 6A/6B contract before the
  corresponding implementation commit; no code-only contract is complete.

### Task 6A/6B final verification

- [ ] Run:

```text
cargo test --locked -p unica-coder discovery::bsl -- --nocapture
cargo test --locked -p unica-coder application::discovery -- --nocapture
cargo test --locked -p unica-coder infrastructure::workspace_services -- --nocapture
cargo test --locked -p unica-coder
cargo fmt --all -- --check
cargo clippy --locked -p unica-coder --all-targets -- -D warnings
python3 tests/ci/test_product_contracts.py
git diff --check
```

- [ ] Confirm `git grep` finds no discovery import/use of `workspace_index`,
  `rusqlite`, `result_text`, `mcp_tool_text` or public tool registration.
- [ ] Commit Task 6A and Task 6B separately so either correctness or cache can
  be rejected independently:
  - `feat: добавить snapshot-bound bsl evidence`
  - `perf: добавить content-verified bsl cache v2`

## 14. Required acceptance matrix

Minimum named behavioral groups:

1. RU and EN definitions, plus mixed language keywords.
2. Procedure/function, async/sync, exported/private, Val/default and context.
3. Same spelling in comment, string, date, deleted block — zero facts.
4. Same-module direct call and exported CommonModule qualified call — resolved.
5. Non-exported, missing, duplicate, object receiver, chained receiver and
   Execute — unresolved/ambiguous/dynamic, never a flow edge.
6. Duplicate identical and duplicate conflicting definitions — both blocking.
7. Complete exact absence versus bounded unknown.
8. Every deterministic limit at N and N+1.
9. Deadline before first read and after one parsed file — both zero records.
10. Snapshot content race — zero accepted prefix.
11. Unsupported but valid module paths — visible gap, no invented ref.
12. Cache hit/miss/stale/old schema/service down — identical authoritative
    output.
13. Provider/file/service permutation — identical ordering and IDs.
14. Hard decoy registered module with similar names — no accidental target.
15. Provider-local `max_records` classifies before loss and retains/drops whole
    v2 semantic groups, including multiple locations of one Call/Definition.
16. Query/group golden bytes and provider versions are stable; local gap
    normalization passes 256/257 and 2,000/2,001 boundaries. The Task 5B v6
    full-source-identity, fingerprint-stability and Unicode-lowercase artifact
    goldens pass through the shared encoder, never a Task 6 copy.
17. Conditional definitions/binders propagate maybe-defined/maybe-shadowed sets
    outside their lexical branch and block direct/qualified false edges.
18. Parameters, Var, bare assignment, For, For Each, module assignments and
    access/index lvalues follow the closed conservative shadow table; every
    unclassified accepted binder yields a scoped gap.
19. Identifier/keyword/boolean/undefined/null search uses one Unicode-lowercase
    comparison rule with the required case-metamorphic fixtures.
20. Async mode passes matching mixed-language terminators and fails closed for
    mismatched/unterminated/nested/preprocessor-ambiguous bodies; no unsupported
    source token is hidden by a catch-all lexer rule.
21. Event/ScheduledJob/Form/HTTP joins match Task 5A/5B v6 exactly and never
    equate declarative runtime Server/SameAsSourceEvent with BSL AtServer.

## 15. Corpus-seed dependency before Task 7 v6

Task 6 unit fixtures are reusable micro-configurations, but they do not replace
the gold corpus. После GREEN Task 6B и **до wiring Task 7 v6** создать RED corpus
seed from Task 13:

- strict Task 7 v6 `tests/fixtures/project_discovery/corpus.json` schema v2;
- all 48 family case definitions, even though Task 7 end-to-end evaluator is
  still RED;
- direct references to Task 6 fixture case IDs rather than duplicated BSL text;
- explicit expected provider gaps for dynamic call, conditional compilation,
  unsupported module identity and deterministic bound;
- lexical-decoy and registered-hard-decoy cases for each relevant family.

The corpus evaluator may initially fail because concrete orchestration is Task
7 v6, but schema validation and fixture references must pass. Task 7 v6 must not be
allowed to shape the corpus after seeing its own output.

## 16. Hard STOP conditions

Stop implementation and show the owner if any condition is true:

- Task 5 active spec still says `ExchangePlan/handles` or lacks the shared
  registration catalog required by Task 6.
- accepted immutable Task 5A/5B v6/5C v2 SHAs are absent, or Task 6 cannot
  borrow the once-built `PlatformConfigurationCatalogSetV1` without adapter
  chaining/reparsing;
- inherited `ProviderGap` cannot express `QueryWide` without a fake artifact,
  cannot scope a non-artifact material gap to one exact linked source set, or
  its artifact scope loses the exact source-set identity.
- `DefinitionShape`/Call still reuse the four-value mutation context.
- root/CommonForm/nested-command BSL is silently skipped or assigned an
  invented canonical ref.
- under the same non-expired injected deadline, any
  evidence/absence/coverage changes when workspace service is stopped.
- the owner requires cache hit/miss identity even across different real
  wall-clock deadline schedules while also requiring parse acceleration.
- a service result is accepted on source fingerprint alone without exact file
  content digest and caller `read_verified`.
- bsl-analyzer/RLM/SQLite/display text becomes receipt-grade evidence.
- `DefinitionAbsent` is emitted for any target touched by a gap, bound,
  malformed file or stale read.
- dynamic, ambiguous, unresolved or module-level call becomes a flow edge.
- conditional definitions/shadows are confined to directive token spans instead
  of propagating maybe-defined/maybe-shadowed sets to outside calls, or any
  conditional-symbol intersection becomes a runtime edge/negative absence;
- any accepted parameter, Var, bare/module assignment, For, For Each,
  access/index lvalue or unknown binder is omitted from the closed conservative
  shadow decision without `unsupported_bsl_shadow_analysis`;
- identifier/keyword/boolean/undefined/null comparison uses different case
  rules or fails the required case-metamorphic fixtures;
- Event/ScheduledJob/Form/HTTP Definition joins locally conflate declarative
  runtime contexts with BSL execution contexts or differ from Task 5B v6;
- async mode can exit on a mismatched/ambiguous terminator, or any unsupported
  nontrivia token is hidden/skipped rather than producing its exact capability
  gap;
- duplicate identical definitions are deduplicated before conflict detection.
- wall-clock abort returns facts parsed before the timeout.
- file/result prefix depends on filesystem/service order instead of canonical
  order.
- any CodeSearch/Definition/Call implementation drops or slices an individual
  record before shared v2 group classification, or uses a local group encoder;
- provider version remains `1` after adopting v2 admission semantics, or the
  query digest omits `max_records`;
- a BSL query/group uses only source-set display name, includes fingerprint in
  `AtomicSourceIdentityV2`/group order, omits fingerprint from query freshness,
  or encodes exact-spelling ArtifactRef bytes instead of the shared
  Unicode-lowercase identity;
- parser reads mutation snapshots or EDT source in v1.
- public `unica.project.discover` registration/tool schema is changed in Task 6.
- Task 7 begins before the 48-case corpus seed is fixed and schema-valid.

## 17. Source notes

The closed lexical subset above is aligned with the primary BSL Parser ANTLR
sources: the lexer is case-insensitive, pairs Russian/English procedure,
function, end, export and Val keywords, recognizes Unicode identifiers and
Async; the parser defines procedure/function declarations, parameters and
global/qualified call forms. This is a compatibility reference, not a copied
runtime dependency. Unica's implementation remains a smaller conservative
parser and labels every unsupported construct rather than claiming full ANTLR
coverage.

- Lexer source of truth:
  <https://github.com/1c-syntax/bsl-parser/blob/develop/src/main/antlr/BSLLexer.g4>
- Parser source of truth:
  <https://github.com/1c-syntax/bsl-parser/blob/develop/src/main/antlr/BSLParser.g4>
