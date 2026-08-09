# Network Documentation Providers (ADR-0032) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Реализовать второй заход справки платформы — сетевые поставщики `v8std` и `kb-1ci` в общем реестре `DocumentationProvider`, аргумент `sourceKinds`, файл политики `unica.toml` — и перевести ADR-0032 в `accepted`.

**Architecture:** Реестр из трёх поставщиков (порядок: `platform-syntax-help`, `kb-1ci`, `v8std`); `v8std` — обёртка над существующим движком `StandardsAdapter` (фасады `unica.standards.*` не меняют контракт и делят с поставщиком движок, endpoint и политику); `kb-1ci` обходит навигационное дерево площадки от корня, находит руководства по именам узлов (адреса не зашиты), сопоставляет запрос с заголовками узлов и дочитывает ограниченное число страниц через серверный рендер `/bin/view/OnecInt/KB/...` (`#xwikicontent`); политика `unica.toml`/`unica.local.toml` разбирается по правилу «неясность — отказ» и управляет только сетевым выходом.

**Tech Stack:** Rust 2021, `serde_json`, `toml` (новая workspace-зависимость), `ureq` (уже в графе), существующие `DocumentationProvider`/`DocumentationRegistry`, `StandardsAdapter`+`HttpClient` из `internal_adapters.rs`.

## Global Constraints

- Утверждённый контракт — ADR-0032 (`spec/decisions/0032-setevye-istochniki-dokumentacii.md`) и раздел «Второй источник» проектной записки `docs/design/2026-08-08-platform-help-documentation-provider-design.md`. Отклонение фиксируется правкой записи тем же коммитом (запись ещё не `accepted` в целевой ветке — правка разрешена).
- Ни один тест не требует сети: транспорты подменяются трейтами; живые проверки — только под переменными окружения (`UNICA_KB1CI_LIVE`).
- Тест пишется до кода и наблюдается падающим по причине дефекта.
- Публичная поверхность: `unica.documentation.search` получает ровно один новый аргумент `sourceKinds`; `unica.documentation.get` НЕ вводится (неграница ADR-0032 п.1).
- Идентификаторы: поставщики `platform-syntax-help`, `kb-1ci`, `v8std`; корпуса `kb-developer-guide`, `kb-administrator-guide`, `public-standards`. Wire-значения `sourceKinds`: `platform-help`, `development-standard`.
- Попадание стандарта не имеет версии платформы: `applicableVersion` = `unversioned` (wire-константа; решение записывается в ADR-0032 при принятии).
- Отказ сети/политики/версии — статус секции, никогда не ответ по памяти; отказ разбора политики — жёсткий отказ всего вызова.
- Каждая правка публичного поведения синхронизирует ADR/инварианты/ведомость тем же коммитом; `generate-tool-surface.py` перегенерируется.

## Разведанные факты (живые зонды 2026-08-09, в план входят как данные)

- Дерево: `GET https://kb.1ci.com/bin/get/OnecInt/Extensions/GuideNavigation/NavigationSource/WebHome?language=en&outputSyntax=plain&data=children&id=<узел>`; корень — `id=#`; ответ `[{id,text,children,a_attr:{title,href}}]`.
- Путь к руководствам: корень → `1C:Enterprise Platform` → `Guides` → `Developer Guides` / `Administrator Guides`.
- Версии руководства разработчика лежат ДВУМЯ уровнями: старые (8.3.22–8.3.25) — прямыми детьми `Developer Guides` с текстом `1C:Enterprise <v> Developer Guide`, новые (8.3.26, 8.3.27) — детьми контейнера `1C:Enterprise Developer Guide`. Обход собирает оба уровня.
- `Administrator Guides` сегодня несёт ОДИН режим — `Administrator Guide.Client/Server Mode` (версии 8.3.22–8.5.1). Файловый режим из дерева исчез (третий переезд площадки): состав режимов берётся из дерева динамически, а не списком; отсутствующий режим — не ошибка. Это отклонение от текста ADR-0032 п.1 («оба руководства администратора») и фиксируется правкой записи в Task 6.
- Руководство разработчика 8.3.27: 52 главы верхнего уровня (совпало с запиской).
- Контент: pretty-href из `a_attr` отдаёт SPA-оболочку БЕЗ текста; серверный рендер — `https://kb.1ci.com/bin/view/OnecInt/KB/<сегменты>/?language=en`, признак — `id="xwikicontent"`, заголовок — первый `<h1>`. Сегменты строятся из `id` узла: отрезать префикс `OnecInt.KB.` и суффикс `.WebHome`, разделить по `.` с учётом экранирования `\.` (точка внутри имени сегмента).
- `document_id` попадания — абсолютный pretty-URL (`https://kb.1ci.com` + `a_attr.href`): устойчивый локатор, который открывает человек.
- `v8std_search` (эндпоинт `https://ai.v8std.ru/mcp`, движок `StandardsAdapter::invoke_with_client`): `result.content[0].text` — JSON-строка с `results: [{id,type,title,description,url,markdown_url,score,...}]`.

## Карта файлов

| Файл | Роль |
| --- | --- |
| `crates/unica-coder/src/domain/documentation.rs` | +`cancellation` в `DocumentationContext`; фильтр применимости в терминах домена |
| `crates/unica-coder/src/application/documentation.rs` | фильтрация секций по `source_kinds`, «применимый поставщик» |
| `crates/unica-coder/src/infrastructure/documentation_policy.rs` | НОВЫЙ: разбор `unica.toml`/`unica.local.toml`, fail-closed |
| `crates/unica-coder/src/infrastructure/standards_documentation.rs` | НОВЫЙ: поставщик `v8std` поверх `StandardsAdapter` |
| `crates/unica-coder/src/infrastructure/kb_1ci.rs` | НОВЫЙ: транспорт, обход дерева, поставщик `kb-1ci` |
| `crates/unica-coder/src/infrastructure/internal_adapters.rs` | endpoint/политика `StandardsAdapter` через общую цепочку |
| `crates/unica-coder/src/infrastructure/application_ports.rs` | реестр из трёх; `sourceKinds`; политика в ветке и фасадах |
| `crates/unica-coder/src/application/tool_contracts.rs` | аргумент `sourceKinds` |
| `Cargo.toml`, `crates/unica-coder/Cargo.toml` | `toml` workspace-зависимость |
| `spec/decisions/0032-*.md`, `spec/architecture/invariants.md`, ведомость, скилл, `.gitignore` | Task 6 |

---

### Task 1: Аргумент `sourceKinds` и применимость поставщика

**Files:** Modify: `domain/documentation.rs`, `application/documentation.rs`, `application/tool_contracts.rs`, `application/mod.rs` (описание НЕ трогать — его расширит Task 6), `infrastructure/application_ports.rs`; ведомость.

**Interfaces:**
- Produces: `SourceKind::parse(value: &str) -> Option<SourceKind>` (wire-строки как в `as_str`); `application::documentation::search` фильтрует: при непустом `request.source_kinds` опрашиваются только поставщики, у которых есть корпус подходящего смысла; из их ответов публикуются только секции подходящего смысла; правило успеха считает только применимые секции (текущая формулировка INV уже говорит «применимая»).
- Диспетчер: `sourceKinds` — массив строк; неизвестное значение — `Err`, называющий значение и допустимые.

Шаги (каждый: тест → RED → код → GREEN → коммит):
- [ ] 1. RED `domain::documentation::tests::source_kind_parses_exactly_its_wire_identifiers`: `parse("platform-help")==Some(PlatformHelp)`, `parse("development-standard")==Some(DevelopmentStandard)`, `parse("standards")==None`. Падает: функции нет (компиляция — добавить заглушку `None` и наблюдать падение по значению).
- [ ] 2. GREEN: `parse` зеркалом `as_str`.
- [ ] 3. RED `application::documentation::tests::a_source_kind_filter_skips_non_matching_providers_and_sections`: два Stub-поставщика разных смыслов; фильтр `[DevelopmentStandard]` — секция PlatformHelp не публикуется, PlatformHelp-поставщик не опрошен (Stub со счётчиком вызовов); успех считается по применимым.
- [ ] 4. GREEN: фильтр в `search` (пропуск поставщика по `corpora()`, пост-фильтр секций по смыслу).
- [ ] 5. RED (диспетчер, стенд `RecordingProvider`): `the_documentation_branch_parses_source_kinds_and_refuses_unknown_values` — `["development-standard"]` доходит до запроса; `["standards"]` — `Err` с именем значения.
- [ ] 6. GREEN: разбор в ветке + `DOCUMENTATION_ARGS` + `ARG_DESCRIPTIONS` («Optional list of source kinds…: platform-help, development-standard; unknown values are refused»), регенерация ведомости.
- [ ] 7. Коммит `feat(documentation): фильтр sourceKinds по смыслу источника`.

### Task 2: Политика `unica.toml` (fail-closed)

**Files:** Create: `infrastructure/documentation_policy.rs`; Modify: `infrastructure/mod.rs`, оба `Cargo.toml` (+`toml = "0.8"` workspace).

**Interfaces (Produces):**
```rust
pub enum NetworkAccess { Allow, Deny }
pub struct DocumentationPolicy { /* приватно: default + по-поставщикам */ }
impl DocumentationPolicy {
    /// known — идентификаторы поставщиков, которым разрешено фигурировать в файле.
    pub fn load(workspace_root: &Path, known: &[&str]) -> Result<DocumentationPolicy, String>;
    pub fn network(&self, provider: &str) -> NetworkAccess;      // с умолчанием default
    pub fn endpoint(&self, provider: &str) -> Option<String>;    // из файла (local поверх основного)
}
```
Правила (из записки, раздел «Настройка»): файла нет — умолчания (default=allow, endpoints пусты); не разбирается — `Err`; неизвестная секция/ключ/значение (`[network].default` не allow|deny, `network` не allow|deny, неизвестный `providers.<id>`, неизвестный ключ внутри) — `Err` с именем; `unica.local.toml` перекрывает по-ключево.

- [ ] 1. RED-набор в модуле (`tempfile`): `absent_file_gives_permissive_defaults`; `unparseable_file_is_a_hard_refusal`; `an_unknown_provider_id_is_a_refusal_not_a_silent_skip`; `an_unknown_network_value_is_a_refusal`; `an_unknown_key_is_a_refusal`; `the_local_overlay_wins_per_key`; `default_deny_denies_only_providers_without_their_own_allow`.
- [ ] 2. GREEN: разбор через `toml::Value` вручную (не derive — нужен отказ на неизвестных ключах).
- [ ] 3. Коммит `feat(documentation): политика сетевого выхода unica.toml с отказом на неясности`.

### Task 3: Поставщик `v8std` и общая цепочка endpoint/политики фасадов

**Files:** Create: `infrastructure/standards_documentation.rs`; Modify: `internal_adapters.rs` (`StandardsAdapter::invoke` получает `endpoint: &str` параметром; текущее разрешение env выносится наружу), `application_ports.rs` (реестр: + `V8StdDocumentationProvider`; фасадная ветка `StandardsAdapter` строит endpoint по цепочке и отказывает при `policy=deny` или нечитаемой политике).

**Interfaces (Produces):**
```rust
pub struct V8StdDocumentationProvider { pub endpoint: String, pub network: NetworkAccess,
    pub http: Arc<dyn HttpClient> }
impl DocumentationProvider for V8StdDocumentationProvider {
    // id "v8std"; corpora: [public-standards / DevelopmentStandard / Community];
    // needs_network() -> true
}
/// Цепочка endpoint (записка, «Настройка»): unica.local.toml → unica.toml →
/// env UNICA_STANDARDS_MCP_URL → builtin https://ai.v8std.ru/mcp.
pub fn resolve_standards_endpoint(policy: &DocumentationPolicy) -> String;
```
Отображение ответа: `result.content[0].text` → JSON → `results[]` → хиты: `rank=i+1`, `provider_score=score as f32`, `document_id=url`, `title`, `snippet=description`, `signature=None`, `applicable_version="unversioned"`; язык секции `"ru"`. Пустые `results` — `Empty`; ошибка HTTP/формата — `Failed` с текстом; `NetworkAccess::Deny` — `Unavailable{PolicyDenied, "сетевой выход v8std запрещён политикой unica.toml"}` без обращения к сети.

- [ ] 1. RED (фейковый `HttpClient`, канон ответа из зонда): `v8std_provider_maps_results_into_a_development_standard_section`; `v8std_denied_by_policy_answers_policy_denied_without_touching_the_network` (фейк со счётчиком, 0 вызовов); `v8std_transport_failure_is_failed_not_empty`; `resolve_standards_endpoint_prefers_config_then_env_then_builtin`.
- [ ] 2. GREEN: модуль + правка `StandardsAdapter::invoke` (подпись с endpoint; вызывающие обновлены).
- [ ] 3. RED (диспетчер): `the_standards_facades_refuse_when_policy_denies_v8std` — временный workspace с `unica.toml` `[providers.v8std] network="deny"`, вызов `unica.standards.search` → `Err`/`ok:false` с `policy`, сеть не тронута (фейк не нужен: отказ до транспорта); и `the_registry_wires_three_providers_in_declared_order` (platform-syntax-help, kb-1ci — с Task 5, до него: v8std вторым; порядок закрепляется списком id).
- [ ] 4. GREEN: композиция. Примечание: тест описания инструмента из первого захода (`…promises_only_declared_source_kinds`) с появлением корпуса стандартов перестаёт запрещать слово «standards» — проверить, что он остаётся зелёным сам.
- [ ] 5. Коммит `feat(documentation): v8std за общим контрактом, фасады делят движок и политику`.

### Task 4: `kb-1ci` — транспорт и обход дерева

**Files:** Create: `infrastructure/kb_1ci.rs` (транспорт + дерево; поставщик — Task 5); Modify: `infrastructure/mod.rs`.

**Interfaces (Produces):**
```rust
pub trait KbTransport: Send + Sync {
    /// GET с honest UA "unica-coder documentation provider (+https://github.com/IngvarConsulting/unica)",
    /// таймаут 30 с; реализация Ureq разносит обращения не чаще одного в 500 мс
    /// (Mutex<Instant>); отменяемость проверяется вызывающим между запросами.
    fn get(&self, url: &str) -> Result<String, String>;
}
pub struct UreqKbTransport; // + const BASE: &str = "https://kb.1ci.com"
pub struct KbNode { pub id: String, pub title: String, pub has_children: bool, pub href: String }
pub fn children(transport: &dyn KbTransport, base: &str, node_id: &str) -> Result<Vec<KbNode>, String>;
/// Сегменты контентного адреса из id узла: срезать "OnecInt.KB." и ".WebHome",
/// разделить по '.', трактуя "\." как точку ВНУТРИ сегмента.
pub fn content_segments(node_id: &str) -> Option<Vec<String>>;
pub fn content_url(base: &str, node_id: &str) -> Option<String>; // /bin/view/OnecInt/KB/<seg>/?language=en
/// Страница: признак настоящего рендера — id="xwikicontent"; заголовок — первый <h1>;
/// текст — разметка снята (локальный strip, как corpus::strip_markup). Оболочка без
/// xwikicontent (перенаправление на корень) => Err "страницы нет".
pub fn read_page(html: &str) -> Result<KbPage, String>; // KbPage { title: String, text: String }
```
Обход руководств (двухуровневая раскладка версий — из зондов):
```rust
pub struct GuideCatalog { pub developer: Vec<KbGuideVersion>, pub administrator: Vec<KbGuideVersion> }
pub struct KbGuideVersion { pub version: String, pub title: String, pub node_id: String, pub mode: Option<String> }
/// От корня '#': найти "1C:Enterprise Platform" → "Guides" → категории по тексту
/// "Developer Guides"/"Administrator Guides" (contains, без регистра). Разработчик:
/// версии среди прямых детей ("1C:Enterprise <v> Developer Guide") и среди детей
/// контейнеров без версии в имени ("1C:Enterprise Developer Guide"). Администратор:
/// каждый ребёнок категории — режим (mode = его text), его дети — версии.
/// Версия — первое вхождение r"8\.\d+(\.\d+)?" в text узла.
pub fn discover_guides(t: &dyn KbTransport, base: &str) -> Result<GuideCatalog, String>;
```

- [ ] 1. RED на чистые функции: `content_segments_unescapes_dots_inside_a_segment` (`"OnecInt.KB.A.B\\.3\\.27_X.WebHome"` → `["A","B.3.27_X"]`), `content_url_builds_the_server_rendered_view`, `read_page_takes_h1_and_refuses_a_shell_without_xwikicontent`.
- [ ] 2. GREEN.
- [ ] 3. RED на обход с фейковым транспортом (канонические JSON из зондов, включая двухуровневые версии разработчика и единственный режим администратора): `discover_guides_collects_versions_from_both_developer_layouts`, `discover_guides_takes_administrator_modes_from_the_tree_not_from_a_list`.
- [ ] 4. GREEN: `children`, `discover_guides`.
- [ ] 5. Коммит `feat(kb-1ci): транспорт площадки и обход дерева руководств`.

### Task 5: `kb-1ci` — поставщик, кеш дерева, реестр, отменяемость

**Files:** Modify: `kb_1ci.rs` (+поставщик), `domain/documentation.rs` (+`cancellation: CancellationToken` в `DocumentationContext`; все конструкторы в тестах — `CancellationToken::default()`), `application_ports.rs` (реестр из трёх, порядок `platform-syntax-help`, `kb-1ci`, `v8std`; политика и транспорт в конструктор).

**Interfaces (Produces):**
```rust
pub struct Kb1ciProvider { pub base: String, pub network: NetworkAccess, pub transport: Arc<dyn KbTransport> }
// id "kb-1ci"; corpora: kb-developer-guide и kb-administrator-guide (PlatformHelp/Vendor);
// needs_network() -> true.
```
Поведение `search` (по записке, разделы «Второй источник», «Отказы»):
- политика Deny → одна секция `Unavailable{PolicyDenied}` (без сети);
- версия: `context.platform_version` (семейство — префиксное совпадение цифр), иначе численно старшая доступная; подходящей нет — `Unavailable{VersionMissing, "…есть: <список версий>"}` С ПЕРЕЧНЕМ доступных;
- дерево выбранного руководства (главы + их дети, два уровня) кешируется в памяти процесса в слоте с ключом (base, node_id) — тот же приём `OnceLock<Mutex<Option<…>>>`, что и индекс установки; на диск ничего;
- совпадение запроса — по заголовкам узлов (contains, lower); страницы НЕ читаются для ранжирования; для верхних `min(limit, 5)` совпадений читается контент (`content_url` → `read_page`) — фрагмент первые 400 символов текста; страница-оболочка → попадание с пустым snippet и предупреждением секции (`warnings`), не отказ;
- `context.cancellation.is_cancelled()` проверяется перед каждым сетевым запросом; отменённый вызов возвращает `Unavailable{Timeout? нет}` — решение: секция `Failed{"вызов отменён"}` не нужна: прерываемся `Unavailable{Timeout,"вызов отменён до завершения обхода"}` — зафиксировать в ADR формулировкой «отмена не публикует частичных секций поставщика»;
- сетевые ошибки обхода/страницы → `Failed` с текстом; локальные поставщики не задеты (это уже держит частичный успех application-слоя).

- [ ] 1. RED (фейковый транспорт): `kb_provider_matches_titles_and_reads_only_the_top_pages` (счётчик GET ≤ 2 обхода + N страниц); `kb_provider_names_available_versions_when_the_requested_one_is_absent`; `kb_denied_by_policy_answers_policy_denied_without_network`; `a_shell_page_becomes_a_warning_not_a_failure`; `a_second_search_answers_the_tree_from_memory` (счётчик: обход один раз); `cancellation_stops_before_the_next_network_request`.
- [ ] 2. GREEN: поставщик + кеш + отменяемость (поле контекста).
- [ ] 3. RED (диспетчер): обновить `the_registry_wires_three_providers_in_declared_order` на тройку и порядок.
- [ ] 4. GREEN: композиция (политика загружается в ветке; `Err` политики — отказ вызова).
- [ ] 5. Живой smoke под `UNICA_KB1CI_LIVE=1` (файл рядом с `real_installation.rs`): вопрос `"URL formats"`, ожидание секции `kb-developer-guide` со статусом Ok и `document_id`, начинающимся с `https://kb.1ci.com/`; прогнать вручную один раз, в CI не требуется.
- [ ] 6. Коммит `feat(kb-1ci): поставщик руководств площадки в реестре документации`.

### Task 6: Записи, скилл, ведомость

**Files:** Modify: `spec/decisions/0032-…md` (статус `accepted`; вписать: порядок реестра; `unversioned`; динамический состав режимов администратора с констатацией исчезновения файлового из дерева; правило отмены; «Верификация» — именованные проверки вместо `manual`), `spec/decisions/README.md` (0032 в принятые), `spec/architecture/invariants.md` (+`INV-APP-DOCUMENTATION-NETWORK-POLICY`: сетевой выход поставщиков документации управляется `unica.toml` c отказом на неясности; запрет — секция `policy-denied`; checks: `documentation_policy.rs`, `standards_documentation.rs`, `kb_1ci.rs`; расширить `INV-MCP-DOCUMENTATION-SECTIONS` словом о фильтре применимости), `application/mod.rs` (описание инструмента: «Search platform help and development standards across documentation providers.» — тест Task 1 первого захода теперь это разрешает), `generate-tool-surface.py` (группа снова «справка платформы и стандарты разработки»), `tool-surface-review.json` (вернуть сценарий «Отличить справку платформы от стандарта разработки в одном ответе»), `plugins/unica/skills/platform-help/SKILL.md` (звать с `"sourceKinds": ["platform-help"]` в примере API-вопроса; правило чтения секции стандартов уже есть; правило про расхождение версий руководства и установки; `policy-denied` — как сообщать), `.gitignore` (+`unica.local.toml`), тесты `tests/ci/test_unica_skills.py` (подстроки нового правила).
- [ ] 1. RED: подстрочный тест скилла на `sourceKinds` в примере; RED: `check-architecture-sync`/ведомость до регенерации.
- [ ] 2. GREEN: все правки, регенерация ведомости.
- [ ] 3. Коммит `docs(spec): принять ADR-0032 и вывести проверяемые правила сетевых поставщиков`.

### Task 7: Верификация и PR

- [ ] 1. `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace -- --test-threads=1`, `tests/ci`, `tests/dev`, оба стража, env-gated `real_installation` на 8.3.27 и 8.5.4, один живой `UNICA_KB1CI_LIVE=1`-прогон, ручной живой вызов v8std через фасад (сеть уже используется фасадами сегодня).
- [ ] 2. PR на `main`: закрывает #242 (этапы 2 и частично 5 — без БСП/конфигурации), #296 (хвост — URL formats через kb), #206 (секция стандартов рядом со справкой различима меткой); `unica.documentation.get` остаётся этапом 3 #242.

## Self-Review

- Покрытие ADR-0032: п.1 — Task 4/5 (отбор руководств; дрейф файлового режима — Task 6); п.2 — Task 4 (обход от корня, адреса не зашиты); п.3 — Task 4/5 (границы обхода, лимиты, spacing, честный UA, отмена); п.4 — Task 3; п.5 — Task 1; п.6–7 — Task 2; верификация — Task 6/7. Порядок секций и `unversioned` — Task 3/5/6.
- Типы сходятся: `NetworkAccess` (Task 2) используется Task 3/5; `KbTransport`/`discover_guides` (Task 4) — Task 5; `SourceKind::parse` (Task 1) — диспетчером.
- Плейсхолдеров нет: каждая проверка названа по имени с ожидаемым падением; канонические JSON фикстур берутся из раздела «Разведанные факты» дословно.
