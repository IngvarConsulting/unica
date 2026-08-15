# Observable Provider-Neutral Code Search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task by task with the stated verification gates.

**Goal:** Сделать `unica.code.search` одним наблюдаемым MCP-вызовом с логическим входом и выходом, параллельными ролями `semantic`/`symbol`/`lexical`, явной полнотой и удобным для модели progress-контрактом.

**Architecture:** Слой application оркестрирует роли через `CodeIntelligenceProvider` и транспортно-нейтральный `SearchProgressSink`. Инфраструктура разрешает логическую область один раз, поставщики ограничивают ею поиск до ранжирования и подсчёта, а общий локатор проецирует внутренние пути в закрытую алгебру `SourceLocation`. MCP-слой только преобразует progress token запроса в `notifications/progress`; без token используется no-op sink. Решение принадлежит ADR-0056 и не включает источник поколений RLM или bounded streaming `git-grep` из ADR-0057/ADR-0058.

**Tech Stack:** Rust 2021, `serde`, `serde_json`, `rmcp 2.2`, `tokio`, Python 3.12 CI-contract tests, GitHub Actions.

**PR boundary:** Выполнять в самостоятельной ветке от актуального `origin/main`. PR не должен иметь базой head другого PR. После слияния этого PR планы ADR-0057 и ADR-0058 пересобираются от нового `main`; между собой они не зависят.

**Design source:** `docs/design/2026-08-13-observable-code-search-design.md`, `spec/decisions/0056-observable-provider-neutral-code-search.md`.

## Целевой wire-контракт

Канонический вызов:

```json
{
  "sourceSet": "main",
  "metadataPath": "CommonModule.Integration.Module",
  "query": "HTTPСоединение",
  "limit": 20
}
```

На время миграции разрешён ровно один из селекторов:

```text
sourceSet [+ metadataPath]
sourceDir
```

`metadataPath` без `sourceSet`, одновременные `sourceSet` и `sourceDir`, а также резервный переход к `sourceDir` после ошибки логического разрешения отклоняются до запуска ролей.

Финальные данные:

```json
{
  "coverage": "partial",
  "elapsedMs": 2481,
  "sections": [
    {
      "role": "semantic",
      "provider": "rlm",
      "status": "limitReached",
      "searchComplete": false,
      "ranking": "provider",
      "ordering": "provider",
      "matches": {"returned": 20, "total": 20, "relation": "lowerBound"},
      "hits": [
        {
          "rank": 1,
          "providerScore": 0.82,
          "location": {
            "kind": "addressed",
            "sourceSet": "main",
            "metadataPath": "CommonModule.Integration.Module",
            "targetKind": "module"
          },
          "line": 42,
          "snippet": "..."
        }
      ],
      "diagnostics": []
    }
  ]
}
```

`status` принимает `ok`, `empty`, `limitReached`, `timedOut`, `unavailable`, `failed`. `empty` допустим только вместе с `searchComplete: true`, `matches: {returned: 0, total: 0, relation: "exact"}`. `limitReached` и `timedOut` требуют `searchComplete: false` и `relation: lowerBound`; при `relation: unknown` поле `total` отсутствует. `ranking: none` запрещает `rank` и `providerScore` у попаданий. `coverage` равен `complete`, только когда все применимые роли завершили полный поиск; `partial` — когда есть полезная, но неполная секция; `none` — когда ни одна роль не доказала результат.

Верхнеуровневый `OperationResult.ok` истинен, когда хотя бы одна роль завершила доказанный поиск (`ok`/`empty`) либо вернула неполные доказанные hits (`limitReached` или `timedOut` с `returned > 0`). `timedOut` с нулём, `unavailable` и `failed` сами по себе успеха не создают.

Progress notification использует стандартные `progress`/`total` и типизированную мету:

```json
{
  "progressToken": "search-17",
  "progress": 1,
  "total": 3,
  "message": "semantic: building index; symbol: searching; lexical: completed",
  "_meta": {
    "io.unica/searchProgress": {
      "schemaVersion": 1,
      "elapsedMs": 2100,
      "deadlineMs": 300000,
      "nextUpdateWithinMs": 2000,
      "providers": [
        {"role": "semantic", "provider": "rlm", "state": "running", "phase": "preparing", "detailCode": "buildingIndex", "resultsFound": 0},
        {"role": "symbol", "provider": "bsl-analyzer", "state": "running", "phase": "searching", "resultsFound": 4},
        {"role": "lexical", "provider": "git-grep", "state": "completed", "phase": "searching", "resultsFound": 20}
      ]
    }
  }
}
```

## Task 1: Зафиксировать доменные типы роли, полноты и логической локации

**Files:**

- Create: `crates/unica-coder/src/domain/source_location.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Modify: `crates/unica-coder/src/domain/code_intelligence.rs`
- Modify: `crates/unica-coder/src/application/source_navigation.rs`
- Test: `crates/unica-coder/src/domain/code_intelligence.rs`
- Test: `crates/unica-coder/src/application/source_navigation.rs`

**Step 1: Написать падающие сериализационные тесты**

Добавить тесты, которые требуют:

- `ProviderRole::{Semantic, Symbol, Lexical}` сериализуются как `semantic`, `symbol`, `lexical`;
- `ProviderIdentity { role, provider }` не ограничивает provider закрытым enum;
- `SourceLocation` одна и та же для `source.locate` и `code.search`;
- `SearchRanking::None` удаляет `rank` и `providerScore`, а `SearchRanking::Provider` допускает их;
- `SearchCountRelation::{Exact, LowerBound, Unknown}` и расширенный `ProviderSectionStatus` сериализуются в camelCase;
- `empty` с неполным `matches` отклоняется конструктором `ProviderSearchSection::new`.

Run: `cargo test -p unica-coder domain::code_intelligence application::source_navigation -- --test-threads=1`

Expected: FAIL — новых типов и конструкторных проверок ещё нет.

**Step 2: Перенести общую алгебру локации в domain**

Определить в `domain/source_location.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SourceLocation {
    Addressed {
        source_set: String,
        metadata_path: Option<MetadataAddress>,
        target_kind: TargetKind,
    },
    Unaddressable {
        source_set: String,
        owner_metadata_path: Option<MetadataAddress>,
        path: String,
    },
}
```

Переместить сюда `LocateRejection`; `application/source_navigation.rs` должен импортировать типы, а не объявлять вторую wire-форму.

**Step 3: Заменить фиксированный provider id ролью и provenance**

В `domain/code_intelligence.rs` ввести:

```rust
pub enum ProviderRole { Semantic, Symbol, Lexical }

pub struct ProviderIdentity {
    pub role: ProviderRole,
    pub provider: String,
}

pub enum SearchRanking { Provider, None }
pub enum SearchOrdering { Provider, ProviderTraversal }
pub enum ProviderSectionStatus { Ok, Empty, LimitReached, TimedOut, Unavailable, Failed }
pub enum SearchCountRelation { Exact, LowerBound, Unknown }

pub struct SearchMatchCount {
    pub returned: usize,
    pub total: Option<usize>,
    pub relation: SearchCountRelation,
}
```

`ProviderSearchHit` получает `location: SourceLocation`; `path` удаляется из сериализуемой формы. `rank` становится `Option<usize>` с `skip_serializing_if`, `provider_score` остаётся `Option<f64>` и также пропускается. Внутренний raw path держать отдельным не-`Serialize` типом `ProviderSearchCandidate`, чтобы случайно не вернуть его наружу.

**Step 4: Реализовать проверяемый конструктор секции**

`ProviderSearchSection::new(...) -> Result<Self, String>` проверяет таблицу:

| Условие | Обязательство |
| --- | --- |
| `status=empty` | complete, exact zero, hits empty |
| `status=limitReached|timedOut` | incomplete, lowerBound, returned equals hits, total is at least returned |
| `ranking=none` | у всех hits нет rank/score |
| `ranking=provider` | rank начинается с 1 и строго возрастает внутри секции |
| `status=unavailable|failed` | hits empty, matches relation unknown |

Поля `ProviderSearchSection` сделать private для модуля и выдавать секцию только через проверяемые constructors (`complete`, `limit_reached`, `timed_out`, `unavailable`, `failed`). Провайдер не должен собирать публичную секцию struct literal-ом в обход этих проверок.

**Step 5: Запустить тесты и закоммитить**

Run: `cargo test -p unica-coder domain::code_intelligence application::source_navigation -- --test-threads=1`

Expected: PASS.

```bash
git add crates/unica-coder/src/domain/source_location.rs crates/unica-coder/src/domain/mod.rs crates/unica-coder/src/domain/code_intelligence.rs crates/unica-coder/src/application/source_navigation.rs
git commit -m "refactor: ввести ролевой контракт поиска кода"
```

## Task 2: Разрешать логическую область один раз и fail closed

**Files:**

- Modify: `crates/unica-coder/src/domain/code_intelligence.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/source_roots.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs`
- Modify: `crates/unica-coder/src/infrastructure/code_intelligence.rs`
- Test: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Test: `crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs`
- Test: `crates/unica-coder/tests/platform/code_intelligence_symlinked_workspace.rs`

**Step 1: Написать падающие тесты области**

Покрыть четыре сценария:

1. `sourceSet=main` разрешается ровно один раз и передаёт всем ролям один канонический root.
2. `sourceSet=main + metadataPath=CommonModule.X.Module` даёт точный файловый фильтр; metadata object даёт фильтры descriptor + subtree.
3. Ошибка логического адреса не вызывает legacy `sourceDir` resolver.
4. Кандидат вне source set отбрасывается до rank/`matches` и не расходует `limit`.

Run: `cargo test -p unica-coder code_search_scope -- --test-threads=1`

Expected: FAIL — `CodeSearchScope` и его resolver отсутствуют.

**Step 2: Добавить закрытый тип области**

```rust
pub struct CodeSearchScope {
    pub source_set: String,
    pub source_root: PathBuf,
    pub filters: Vec<RelativeSearchFilter>,
    pub legacy_selector: bool,
}

pub enum RelativeSearchFilter {
    Exact(PathBuf),
    Subtree(PathBuf),
}
```

Пустой `filters` означает весь source set. Все пути в фильтрах — нормальные относительные пути без `..`, symlink и абсолютного префикса.

**Step 3: Выделить отдельный порт search context**

Добавить `ApplicationPorts::resolve_code_search_context`; не расширять семантику существующего `resolve_code_intelligence_context`, которым пользуются definition/outline. Инфраструктурная реализация:

- для `sourceSet` вызывает `resolve_named_source_set`;
- для `metadataPath` использует доказанный `resolve_platform_xml_target` и новый метод `ClosedPlatformXmlTarget::search_filters()`;
- для `sourceDir` использует прежний resolver и восстанавливает имя source set, если оно доказуемо;
- нормализует workspace/cwd/root в один identity class;
- после любой логической ошибки возвращает её немедленно.

**Step 4: Ограничить кандидаты до ранжирования и счёта**

Добавить общий `CodeSearchScope::accepts(relative_path)`. Провайдер, который не умеет передать фильтр своему backend, обязан отфильтровать сырой поток до локального rank/`matches`. Если backend отдаёт уже усечённый top-K и потому фильтрация не может доказать корректный ответ, секция завершается `failed` с диагностикой `scope_unsupported`, а не ищет весь workspace и не выдаёт неполный результат за полный.

**Step 5: Проецировать внутренний путь общей функцией**

В `infrastructure/code_intelligence.rs` добавить `project_search_candidate(context, candidate, cancellation)`. Функция вызывает тот же `locate_platform_xml_source_path`, который обслуживает `unica.source.locate`, и возвращает `SourceLocation::Addressed` либо `Unaddressable`. Абсолютный путь и path вне source set никогда не входят в `ProviderSearchHit`.

**Step 6: Запустить тесты и закоммитить**

Run: `cargo test -p unica-coder code_search_scope -- --test-threads=1`

Run: `cargo test -p unica-coder --test platform_code_intelligence -- --test-threads=1`

Expected: PASS; symlinked cwd и `/var` alias остаются корректными.

```bash
git add crates/unica-coder/src/domain/code_intelligence.rs crates/unica-coder/src/application/ports.rs crates/unica-coder/src/infrastructure/application_ports.rs crates/unica-coder/src/infrastructure/source_roots.rs crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs crates/unica-coder/src/infrastructure/code_intelligence.rs crates/unica-coder/tests/platform/code_intelligence_symlinked_workspace.rs
git commit -m "feat: ограничить поиск логической областью"
```

## Task 3: Перестроить coordinator вокруг ролей, общего deadline и progress sink

**Files:**

- Modify: `crates/unica-coder/src/domain/code_intelligence.rs`
- Modify: `crates/unica-coder/src/application/code_intelligence.rs`
- Modify: `crates/unica-coder/src/infrastructure/code_intelligence.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Modify: `crates/unica-coder/src/domain/operational_config.rs`
- Modify: `crates/unica-coder/src/infrastructure/operational_config.rs`
- Test: `crates/unica-coder/src/application/code_intelligence.rs`
- Test: `crates/unica-coder/src/infrastructure/code_intelligence.rs`
- Test: `crates/unica-coder/src/domain/operational_config.rs`

**Step 1: Написать падающие orchestration tests**

Тестовые провайдеры с barrier должны доказать:

- все три `search()` входят до освобождения barrier;
- порядок секций всегда semantic, symbol, lexical независимо от порядка завершения;
- heartbeat публикуется на старте, при смене phase и после 2 секунд без события;
- `total=3`, progress — число terminal roles, а не процент;
- одна `unavailable` роль не отменяет другие;
- global deadline завершает активные роли как `timedOut` и сохраняет уже завершённые секции;
- общий `ok` принимает complete section или неполные доказанные hits, но не timeout с нулём;
- parent cancellation отменяет workers и возвращает cancellation error без частичного payload;
- провайдер implementation может измениться без изменения role routing.

Run: `cargo test -p unica-coder application::code_intelligence -- --test-threads=1`

Expected: FAIL — role registry и progress sink отсутствуют.

**Step 2: Добавить transport-neutral progress seam**

```rust
pub trait SearchProgressSink: Send + Sync {
    fn publish(&self, snapshot: SearchProgressSnapshot);
}

pub struct SearchProgressSnapshot {
    pub schema_version: u32,
    pub elapsed_ms: u64,
    pub deadline_ms: u64,
    pub next_update_within_ms: u64,
    pub providers: Vec<SearchProviderProgress>,
}

pub enum SearchProviderState {
    Queued,
    Running,
    Completed,
    Unavailable,
    Failed,
    TimedOut,
    Cancelled,
}

pub enum SearchProviderPhase {
    Preparing,
    Searching,
    Ranking,
}
```

`SearchProviderProgress` содержит role/provider, отдельные `state` и `phase`, необязательный стабильный `detailCode`, `resultsFound` и только при измеренном знаменателе `completedUnits/totalUnits`. `reconcilingSources` и `buildingIndex` являются detail codes при phase `preparing`, а не новыми фазами. Добавить `NoopSearchProgressSink`. Provider получает узкий `ProviderProgressSink`, который может менять только snapshot своей роли; он не видит MCP token.

**Step 3: Сделать registry ролевым**

`CodeIntelligenceRegistry::new` принимает по одному provider на каждую роль, отклоняет дубликат и отсутствие роли. `CodeIntelligenceProvider` возвращает `ProviderIdentity`, который flatten-ится в wire-поля `role` и `provider`; coordinator ветвится только по `ProviderRole`.

**Step 4: Исполнить один общий deadline**

Изменить compiled defaults:

```text
search_total_timeout_seconds = 300
search_rlm_timeout_seconds = 300
search_git_grep_timeout_seconds = 2
```

Semantic и symbol получают остаток global deadline; lexical — `min(remaining, configured lexical timeout)`. Существующие положительные файловые override и constraint `role <= total` сохраняются. Coordinator использует `recv_timeout` до ближайшего heartbeat/global deadline, а не отдельное последовательное ожидание роли.

У каждой worker role есть дочерний cancellation token, связанный с parent token. Global/role deadline отменяет только соответствующих незавершённых workers, затем использует существующий bounded worker/process drain; parent cancellation помечается отдельно и запрещает payload. Поэтому общий timeout может сохранить уже завершённые секции, а пользовательская отмена — нет, и возврат не оставляет бесконтрольные threads/process trees.

**Step 5: Дождаться RLM readiness внутри semantic role**

`RlmProvider::search` при `Missing`, `Stale` или `Building` сообщает `state=running`, `phase=preparing` и соответствующий `detailCode`, затем вызывает закрытую операцию workspace service `wait_rlm_readiness` с остатком provider deadline. Она запускает/проверяет индекс один раз, затем ждёт изменения status marker с ограниченным backoff `100ms, 200ms, 400ms, 800ms, 1s...`; пока marker остаётся `building`, полный `source_generation` повторно не вычисляется. Когда marker становится `ready`, `failed` или deadline истекает, generation пересверяется ровно на терминальной границе. `Failed`/`Unavailable` терминальны. Это позволяет одному честному integration call дождаться реально построенного индекса, не превращая private polling в повторный обход дерева и не выводя polling в MCP-поверхность. ADR-0057 позднее заменит обе терминальные сверки trusted revision fence.

**Step 6: Собирать coverage из доказанной полноты**

Coordinator не пересчитывает provider score и не сравнивает секции. Он проверяет конструктор секции, вычисляет `coverage`, сортирует по role и прикладывает общий elapsed. Старое нормализующее `truncate + rerank` удалить: limit обязан соблюдать provider до публикации.

**Step 7: Запустить тесты и закоммитить**

Run: `cargo test -p unica-coder application::code_intelligence infrastructure::code_intelligence domain::operational_config infrastructure::operational_config -- --test-threads=1`

Expected: PASS.

```bash
git add crates/unica-coder/src/domain/code_intelligence.rs crates/unica-coder/src/application/code_intelligence.rs crates/unica-coder/src/infrastructure/code_intelligence.rs crates/unica-coder/src/infrastructure/workspace_index.rs crates/unica-coder/src/infrastructure/workspace_services.rs crates/unica-coder/src/domain/operational_config.rs crates/unica-coder/src/infrastructure/operational_config.rs
git commit -m "feat: оркестрировать наблюдаемые роли поиска"
```

## Task 4: Передать progress token через MCP без утечки SDK в application

**Files:**

- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/interfaces/mcp.rs`
- Test: `crates/unica-coder/src/application/mod.rs`
- Test: `crates/unica-coder/src/interfaces/mcp.rs`

**Step 1: Написать падающие MCP protocol tests**

Через существующий duplex `McpClient` проверить:

1. Call с `params._meta.progressToken` получает первый notification не позднее 250 ms и до финального result.
2. Notification сохраняет token, standard progress/total и `io.unica/searchProgress`.
3. Два heartbeat без phase change разделены не более чем 2 секундами виртуального тестового clock.
4. Call без token не получает progress notification.
5. Ошибка `notify_progress`/закрытый peer не отменяет поиск.
6. Cancellation notification по-прежнему имеет приоритет.

Run: `cargo test -p unica-coder interfaces::mcp::tests -- --test-threads=1`

Expected: FAIL — ToolCallHandler не принимает observer.

**Step 2: Ввести application invocation observer**

Добавить `UnicaApplication::call_tool_observed(name, args, cancellation, Arc<dyn SearchProgressSink>)`. Существующие `call_tool` и `call_tool_cancellable` делегируют ему с no-op sink, поэтому прямые тесты и другие интерфейсы не меняются.

Провести sink только в ветку `CodeIntelligenceOperation::Search`; другие инструменты не знают о search progress.

**Step 3: Реализовать MCP bridge каналом**

В `UnicaServer::call_tool`:

- прочитать progress token из `CallToolRequestParams.meta`;
- если token есть, создать `tokio::sync::watch::channel<Option<SearchProgressSnapshot>>(None)`;
- синхронный sink в `spawn_blocking` заменяет последнее значение; промежуточные снимки coalesce, финальный не теряется и producer не блокируется транспортом;
- отдельная async task вызывает `context.peer.notify_progress(...)`;
- после result закрыть канал, дождаться forwarder, игнорируя transport error;
- без token использовать no-op и не создавать task.

Так SDK types остаются в `interfaces/mcp.rs`, а блокирующий coordinator не вызывает async API.

**Step 4: Запустить тесты и закоммитить**

Run: `cargo test -p unica-coder interfaces::mcp::tests application::tests -- --test-threads=1`

Expected: PASS.

```bash
git add crates/unica-coder/src/application/mod.rs crates/unica-coder/src/interfaces/mcp.rs
git commit -m "feat: публиковать ход поиска через MCP progress"
```

## Task 5: Синхронизировать публичную схему, документацию и архитектурный реестр

**Files:**

- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Modify: `crates/unica-coder/src/application/operation_descriptors.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `plugins/unica/skills/code-search/SKILL.md`
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/architecture/quality-requirements.md`
- Modify: `spec/architecture/tool-surface.md`
- Modify: `spec/architecture/change-checklist.md`
- Modify: `spec/architecture/building-blocks.md`
- Modify: `spec/architecture/concepts.md`
- Modify: `spec/architecture/runtime.md`
- Modify: `tests/ci/test_tool_surface_ledger.py`
- Modify: `tests/ci/test_release_assessment.py`
- Test: `crates/unica-coder/src/application/tool_contracts.rs`

**Step 1: Написать падающие schema/registry tests**

Требовать в схеме `sourceSet`, optional `metadataPath`, migration `sourceDir`, mutual exclusion и неизменный `limit 1..50`. В ledger требовать roles, logical location, completeness/ranking/`matches` и progress meta. В architecture test требовать числовые budgets `300`, `2`, heartbeat `2` и ADR-0056.

Run: `cargo test -p unica-coder application::tool_contracts -- --test-threads=1`

Run: `python3.12 -m unittest tests.ci.test_tool_surface_ledger tests.ci.test_architecture_registry tests.ci.test_release_assessment`

Expected: FAIL на старой схеме и fixed provider sections.

**Step 2: Обновить contract registry**

`CODE_SEARCH_ARGS` становится `limit, metadataPath, query, sourceDir, sourceSet`. Schema содержит `oneOf` для canonical/legacy selector и запрет `metadataPath` без `sourceSet`. Description объясняет role-local ranking и progress, а не перечисляет реализации как API.

**Step 3: Обновить нормативных владельцев**

- `INV-MCP-CODE-SEARCH-SECTIONS`: Decision ADR-0056, роли/происхождение/полнота/отмена.
- `INV-APP-CODE-PROVIDER-BOUNDARY`: Decision ADR-0017, ADR-0056; application зависит от роли, не implementation.
- `INV-APP-CONFIG-SNAPSHOT`: Decision ADR-0040, ADR-0056; новые defaults 300/300/2.
- добавить `REQ-OBS-SEARCH-PROGRESS`: start, phase change, heartbeat <=2s, no fake percentage, MCP token optional.
- `tool-surface.md` и checklist ссылаются на ID владельцев, не копируют норму как отдельное правило.

ADR-0017 и ADR-0040 уже приняты в `main`; их исторический текст не переписывать и не помечать целиком superseded, потому что ADR-0056 заменяет только названные части.

**Step 4: Обновить skill**

Примеры используют `sourceSet`; отдельно показать migration `sourceDir`. Объяснить модели:

- ждать финальный ответ, пока progress показывает живые роли;
- не сравнивать score разных ролей;
- `ranking:none` — префикс обхода, не лучшие результаты;
- `lowerBound/unknown` не трактовать как точный total;
- `unaddressable` нельзя передавать как `metadataPath`.

**Step 5: Запустить contract tests и закоммитить**

Run: `cargo test -p unica-coder application::tool_contracts -- --test-threads=1`

Run: `python3.12 -m unittest tests.ci.test_tool_surface_ledger tests.ci.test_architecture_registry tests.ci.test_unica_skills tests.ci.test_release_assessment`

Expected: PASS.

```bash
git add crates/unica-coder/src/application/tool_contracts.rs crates/unica-coder/src/application/operation_descriptors.rs crates/unica-coder/src/application/mod.rs plugins/unica/skills/code-search/SKILL.md spec/architecture/invariants.md spec/architecture/quality-requirements.md spec/architecture/tool-surface.md spec/architecture/change-checklist.md spec/architecture/building-blocks.md spec/architecture/concepts.md spec/architecture/runtime.md tests/ci/test_tool_surface_ledger.py tests/ci/test_release_assessment.py
git commit -m "docs: синхронизировать контракт наблюдаемого поиска"
```

## Task 6: Сделать долгую BSP-проверку честной и условной

**Files:**

- Modify: `scripts/ci/release-assessment.py`
- Modify: `scripts/ci/classify-workflow-changes.py`
- Modify: `scripts/ci/evaluate-ci-gate.py`
- Modify: `.github/workflows/unica-plugin-release.yml`
- Modify: `tests/ci/test_classify_workflow_changes.py`
- Modify: `tests/ci/test_evaluate_ci_gate.py`
- Modify: `tests/ci/test_unica_workflow.py`
- Modify: `tests/ci/test_release_assessment.py`

**Step 1: Написать падающие CI routing tests**

Добавить output `search_integration_changed`. Он true для:

```text
crates/unica-coder/src/application/code_intelligence.rs
crates/unica-coder/src/application/mod.rs
crates/unica-coder/src/domain/code_intelligence.rs
crates/unica-coder/src/infrastructure/code_intelligence.rs
crates/unica-coder/src/infrastructure/application_ports.rs
crates/unica-coder/src/infrastructure/operational_config.rs
crates/unica-coder/src/infrastructure/platform/process.rs
crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs
crates/unica-coder/src/infrastructure/source_roots.rs
crates/unica-coder/src/infrastructure/workspace_index.rs
crates/unica-coder/src/infrastructure/workspace_services.rs
crates/unica-coder/src/infrastructure/source_revision.rs
crates/unica-coder/src/infrastructure/platform/source_revision_fence.rs
crates/unica-coder/src/interfaces/mcp.rs
scripts/ci/release-assessment.py
tests/ci/test_release_assessment.py
```

И true при `--force-full`/label `ci:full`. Обычная несвязанная Rust-правка должна оставить release-assessment skipped, не пропуская обычные Rust/package checks.

Run: `python3.12 -m unittest tests.ci.test_classify_workflow_changes tests.ci.test_evaluate_ci_gate tests.ci.test_unica_workflow`

Expected: FAIL — output и условие отсутствуют.

**Step 2: Маршрутизировать длинный job**

Провести output через `classify-changes`. Условие `release-assessment`:

```yaml
if: >-
  ${{
    always() &&
    needs.build-tools.result == 'success' &&
    needs.classify-changes.outputs.search_integration_changed == 'true'
  }}
```

Добавить `classify-changes` в `needs`. Обновить gate evaluator: package pipeline не обязан запускать release-assessment, но search integration contour обязан получить success.

**Step 3: Ждать индекс в самом search call**

Release assessment передаёт progress token, читает progress notifications до финального response и сохраняет их в scenario artifact. `validate_code_search` требует terminal semantic section `ok|empty`, а не считает `building/unavailable` успешной проверкой. Собственный timeout сценария должен быть больше compiled 300s с небольшим transport запасом, например 330s; индекс не опрашивается отдельным публичным инструментом.

**Step 4: Запустить CI tests и закоммитить**

Run: `python3.12 -m unittest tests.ci.test_classify_workflow_changes tests.ci.test_evaluate_ci_gate tests.ci.test_unica_workflow tests.ci.test_release_assessment`

Expected: PASS.

```bash
git add scripts/ci/release-assessment.py scripts/ci/classify-workflow-changes.py scripts/ci/evaluate-ci-gate.py .github/workflows/unica-plugin-release.yml tests/ci/test_classify_workflow_changes.py tests/ci/test_evaluate_ci_gate.py tests/ci/test_unica_workflow.py tests/ci/test_release_assessment.py
git commit -m "ci: запускать честную проверку поиска по изменению механизма"
```

## Task 7: Полная верификация и подготовка самостоятельного PR

**Files:**

- Verify only; исправления вносятся в соответствующий предыдущий commit или отдельный осмысленный commit.

**Step 1: Формат и статические проверки**

Run: `cargo fmt --all -- --check`

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`

Expected: PASS.

**Step 2: Полные тесты**

Run: `cargo test --workspace -- --test-threads=1`

Run: `python3.12 -m unittest discover -s tests/ci --durations 20`

Expected: PASS.

**Step 3: Архитектурный guard**

Run: `python3.12 scripts/ci/check-architecture-sync.py --base origin/main --strict`

Run: `git diff --check origin/main...HEAD`

Expected: PASS; public surface, ADR и registry меняются одним PR.

**Step 4: Проверить packaged MCP**

Run: `python3.12 scripts/ci/smoke-unica-mcp.py --help`

Run точной package/smoke-команды из текущего workflow для локальной платформы, если bundled runtime доступен; отсутствие runtime не заменяет CI release-assessment и должно быть явно записано в PR.

**Step 5: Обновить решение и открыть PR**

Перед PR перевести ADR-0056 из `proposed` в `accepted` в этой ещё не слитой ветке, если все acceptance checks выполнены. Перебазировать только на `origin/main`, повторить Steps 1–3, push и открыть один PR с:

- ссылкой на #275;
- `Closes #275`, если issue теперь целиком покрыта;
- явными неграницами ADR-0057/ADR-0058;
- результатами unit/full/architecture checks;
- отметкой, запускался ли реальный BSP release-assessment.
