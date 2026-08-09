# Лексическое ядро documentation.search — план реализации

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Реализовать issue #415: пословный, морфологический и нечёткий матчинг в `unica.documentation.search` одним лексическим ядром для всех локальных поставщиков; нормализация запроса и TTL-кеш для v8std; ADR и ретривал-гейт.

**Architecture:** Новый модуль `infrastructure/documentation_retrieval.rs` — токенизация (не-алфанумерик + CamelCase, включая кириллицу), стемминг ru/en (Snowball), ограниченная нечёткость (полосный Дамерау–Левенштейн по словарю термов), BM25F по инвертированному индексу в памяти. Поставщики `platform-syntax-help`, `configuration-help`, `kb-1ci` строят индексы этим ядром вместо `contains`; kb дополнительно расширяет русские запросы словарём ru↔en, выведенным из двуязычных заголовков корпуса установки. Секции независимы, оценки локальны — INV-MCP-DOCUMENTATION-SECTIONS не меняется; индексы живут только в памяти процесса — ADR-0029 п.11 соблюдён.

**Tech Stack:** Rust (crates/unica-coder), rust-stemmers (Snowball ru/en), собственный полосный DL, собственный BM25F. Без tantivy, без состояния на диске, без новых сетевых вызовов.

## Global Constraints

- Оценки и ранги локальны для секции; слияния секций нет (INV-MCP-DOCUMENTATION-SECTIONS).
- Индексы только в памяти процесса, ключ — установка/корпус; на диск не пишется ничего (ADR-0029 п.11).
- kb-1ci: обход только от объявленных корней, `PAGE_FETCH_CAP` не растёт (ADR-0032 п.2–3).
- Политика `unica.toml` проверяется до сети и до кеша; `policy-denied` не отвечает из кеша.
- Запись решения (ADR-0035) и инвариант INV-MCP-SEARCH-SEMANTICS — в этом же наборе изменений (AGENTS.md, «общее ядро — политика для >1 потребителя»).
- Сначала падающий тест, потом код (AGENTS.md «Правила разработки»).
- Коммиты без GPG-подписи (`--no-gpg-sign`), русские сообщения в стиле репозитория, ссылка на #415.

---

### Task 1: Зависимость rust-stemmers и токенизация

**Files:**
- Modify: `Cargo.toml` (workspace.dependencies)
- Modify: `crates/unica-coder/Cargo.toml`
- Create: `crates/unica-coder/src/infrastructure/documentation_retrieval.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs` (объявить модуль)

**Interfaces:**
- Produces: `pub fn tokenize(text: &str) -> Vec<String>` — токены в нижнем регистре; разбиение по не-алфанумерик и по границе «строчная→Заглавная» (Unicode, включая кириллицу); цифры остаются в токене.

- [ ] Тест (в `#[cfg(test)]` нового модуля):

```rust
#[test]
fn tokenize_splits_camel_case_including_cyrillic() {
    assert_eq!(tokenize("ТаблицаЗначений.Свернуть"), vec!["таблица", "значений", "свернуть"]);
    assert_eq!(tokenize("ValueTable.GroupBy"), vec!["value", "table", "group", "by"]);
    assert_eq!(tokenize("СтрНайти"), vec!["стр", "найти"]);
    assert_eq!(tokenize("как удалить элемент — массива?"), vec!["как", "удалить", "элемент", "массива"]);
    assert_eq!(tokenize("HTTPСоединение2"), vec!["http", "соединение2"]);
}
```

- [ ] `cargo test -p unica-coder tokenize_splits` — FAIL (модуля нет).
- [ ] Реализация: `rust-stemmers = "1.2"` в workspace (комментарий: лицензии MIT/BSD-3, офлайн-Snowball ru/en для ядра поиска документации, ADR-0035); `rust-stemmers.workspace = true` в крейте. `tokenize`: проход по `char`, граница токена — не-`char::is_alphanumeric`, дополнительный разрез при `prev.is_lowercase() && current.is_uppercase()`; `to_lowercase()` на каждом токене.
- [ ] `cargo test -p unica-coder tokenize_splits` — PASS.
- [ ] Коммит: `feat(documentation): токенизация лексического ядра поиска (#415)`.

### Task 2: Стемминг ru/en по письменности токена

**Interfaces:**
- Produces: `pub fn stem_token(token: &str) -> String` — кириллический токен стеммится русским Snowball, прочие — английским; результат в нижнем регистре.

- [ ] Тест:

```rust
#[test]
fn stem_token_uses_script_matched_snowball() {
    assert_eq!(stem_token("таблицу"), stem_token("таблица"));
    assert_eq!(stem_token("значений"), stem_token("значения"));
    assert_eq!(stem_token("tables"), stem_token("table"));
    assert_ne!(stem_token("регистр"), stem_token("регламент"));
}
```

- [ ] FAIL → реализация (`rust_stemmers::Stemmer::create(Algorithm::Russian|English)`, выбор по наличию кириллического символа; стеммеры — `std::sync::LazyLock`) → PASS.
- [ ] Коммит: `feat(documentation): стемминг ru/en в лексическом ядре (#415)`.

### Task 3: Полосный Дамерау–Левенштейн с порогом

**Interfaces:**
- Produces: `pub fn bounded_damerau_levenshtein(a: &str, b: &str, cap: usize) -> Option<usize>` (расстояние ≤ cap, иначе None); `pub fn fuzzy_cap(token_chars: usize) -> usize` (0 при len<4, 1 при 4..=8, 2 при >8).

- [ ] Тест:

```rust
#[test]
fn bounded_distance_respects_cap_and_transposition() {
    assert_eq!(bounded_damerau_levenshtein("стрнайтти", "стрнайти", 2), Some(1));
    assert_eq!(bounded_damerau_levenshtein("свренуть", "свернуть", 2), Some(1)); // транспозиция
    assert_eq!(bounded_damerau_levenshtein("массив", "запрос", 2), None);
    assert_eq!(fuzzy_cap(3), 0);
    assert_eq!(fuzzy_cap(8), 1);
    assert_eq!(fuzzy_cap(9), 2);
}
```

- [ ] FAIL → реализация: DP по `Vec<char>` с полосой ширины `2*cap+1` и ранним выходом, транспозиция соседних (restricted DL); предварительный отказ при `|len(a)-len(b)| > cap` → PASS.
- [ ] Коммит: `feat(documentation): ограниченная нечёткость Дамерау–Левенштейна (#415)`.

### Task 4: Инвертированный индекс и BM25F

**Interfaces:**
- Produces:
  - `pub struct RetrievalFields<'a> { pub title: &'a str, pub signature: &'a str, pub body: &'a str }`
  - `pub struct RetrievalIndex` c `pub fn build<'a>(documents: impl IntoIterator<Item = RetrievalFields<'a>>) -> RetrievalIndex` и `pub fn query(&self, query: &str, limit: usize, expansions: &[Vec<String>]) -> Vec<RetrievalHit>`
  - `pub struct RetrievalHit { pub document: usize, pub score: f32 }`
  - Семантика: термы — стемы токенов; взвешенный TF полей (title 4.0, signature 2.0, body 1.0) и взвешенная длина — BM25F (k1=1.2, b=0.75); IDF по формуле Robertson `ln(1 + (N - df + 0.5)/(df + 0.5))`; токен запроса без постингов уходит в нечёткий fallback по словарю термов (префильтр длины, только термы на минимальной найденной дистанции, множители 0.7/0.5 за d=1/2); точный (нестеммированный) токен запроса, найденный среди сырых токенов заголовка, умножает оценку документа на 1.2; `expansions[i]` — дополнительные термы-синонимы i-го токена запроса, участвуют как обычные термы с множителем 0.9 и не штрафуют документы без них; порядок при равных оценках — по возрастанию номера документа.

- [ ] Тесты (минимум):

```rust
fn doc(title: &str, body: &str) -> ... // хелпер RetrievalFields { title, signature: "", body }

#[test]
fn word_order_and_morphology_do_not_matter() {
    let index = RetrievalIndex::build([
        doc("ТаблицаЗначений.Свернуть (ValueTable.GroupBy)", "Группирует строки таблицы значений."),
        doc("Массив.Удалить (Array.Delete)", "Удаляет элемент массива."),
    ]);
    let hits = index.query("свернуть таблицу значений", 5, &[]);
    assert_eq!(hits[0].document, 0);
    let hits = index.query("как удалить элемент массива", 5, &[]);
    assert_eq!(hits[0].document, 1);
}

#[test]
fn title_match_outranks_body_match() { /* терм в title у doc A, тот же терм только в body у doc B → A выше */ }

#[test]
fn typo_falls_back_to_fuzzy_with_discount() {
    // "СтрНайтти" находит документ с "СтрНайти", но оценка ниже точного запроса
}

#[test]
fn long_enumeration_page_ranks_below_short_titled_page() {
    // "Свернуть" в title короткого документа против одного вхождения в длинном body → короткий выше
}

#[test]
fn expansions_add_terms_without_penalizing_originals() {
    // запрос "свернуть" + expansions [["group","by"]] находит документ с только-английским заголовком
}

#[test]
fn ties_break_by_document_index_deterministically() { /* два одинаковых документа → порядок 0,1 */ }
```

- [ ] FAIL → реализация: словарь `BTreeMap<String, TermId>`; постинги `Vec<Vec<(u32 doc, f32 weighted_tf)>>`; длины `Vec<f32>`; сырые токены заголовков `Vec<Vec<String>>`; скоринг в `HashMap<u32, f32>` → сортировка (score desc, doc asc) → truncate(limit) → PASS.
- [ ] Коммит: `feat(documentation): BM25F и нечёткий fallback лексического ядра (#415)`.

### Task 5: ADR-0035 и инвариант INV-MCP-SEARCH-SEMANTICS

**Files:**
- Create: `spec/decisions/0035-leksicheskoe-yadro-poiska-dokumentacii.md` (Статус accepted, Дата 2026-08-10, Задача #415; Решение: пункты 1–7 из issue; Неграницы: эмбеддинги, слияние секций, полнотекст kb, подмена серверной релевантности v8std; Верификация: имена cargo-тестов задач 1–10)
- Modify: `spec/architecture/invariants.md` — новая запись в области MCP рядом с INV-MCP-DOCUMENTATION-SECTIONS: Rule — локальные корпуса `unica.documentation.search` сопоставляют запрос одним лексическим контрактом (пословность, CamelCase-токенизация, морфология ru/en, ограниченная нечёткость, детерминированный порядок; оценки локальны для секции); Decision: ADR-0035; Check: `ci-test` — `crates/unica-coder/src/infrastructure/documentation_retrieval.rs`; Scope: source, runtime.

- [ ] Написать обе записи; `python3.12 -m pytest tests/ci/test_architecture_registry.py tests/ci/test_design_documents.py` (через /opt/homebrew/bin/python3.12) — PASS; проверить требования индексов (INV-DOC-INDEX-SYNC) — если реестр ведёт индекс, обновить.
- [ ] Коммит: `docs(spec): ADR-0035 — лексическое ядро поиска документации (#415)`.

### Task 6: Двуязычный лексикон из заголовков корпуса

**Interfaces:**
- Produces: `pub struct BilingualLexicon` c `pub fn from_titles<'a>(titles: impl IntoIterator<Item = &'a str>) -> BilingualLexicon` и `pub fn expansions(&self, query: &str) -> Vec<Vec<String>>` (для каждого токена запроса — английские термы его сегмента; пусто для нерусских/ненайденных).
- Разбор заголовка вида `Рус[.Рус2] (Eng[.Eng2])`: скобочная пара в конце; русская и английская часть режутся по `.`; сегменты сопоставляются по позиции; каждый ru-токен сегмента (после `tokenize`) отображается в токены parного en-сегмента; стемы с обеих сторон.

- [ ] Тест:

```rust
#[test]
fn lexicon_maps_ru_tokens_to_en_segment_tokens() {
    let lexicon = BilingualLexicon::from_titles(["ТаблицаЗначений.Свернуть (ValueTable.GroupBy)"]);
    let expansions = lexicon.expansions("свернуть таблицу");
    assert_eq!(expansions[0], vec![stem_token("group"), stem_token("by")]);
    assert_eq!(expansions[1], vec![stem_token("value"), stem_token("table")]);
}
```

- [ ] FAIL → реализация в `documentation_retrieval.rs` → PASS.
- [ ] Коммит: `feat(documentation): двуязычный лексикон ru↔en из заголовков корпуса (#415)`.

### Task 7: platform-syntax-help на ядре

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/platform_help/provider.rs`

**Interfaces:**
- Consumes: `RetrievalIndex`, `RetrievalFields`. `IndexedPage` теряет `title_lower`/`text_lower`; `IndexedCorpus` получает `index: RetrievalIndex` (строится в `index_corpus` из title/signature-текста/текста страницы). `rank_pages` заменяется на запрос индекса; `provider_score` — BM25-оценка; ключ и кеш индекса (`indexed`, `IndexKey`) не меняются. Также `pub(crate) fn bilingual_lexicon_for(root, language) -> Option<Arc<BilingualLexicon>>` — лексикон из заголовков syntax-context, кешируется рядом с индексом (тот же ключ процесса).

- [ ] Новые тесты на фикстурах (там уже есть конструктор корпусов в тестах): естественный порядок слов находит страницу; опечатка находит; английский запрос находит двуязычный заголовок; страница с термом в заголовке выше страницы с термом только в тексте (замена `a_text_match_ranks_below_a_title_match` на относительное сравнение оценок).
- [ ] FAIL → перевод провайдера на ядро; правка существующих тестов, где закреплены константы 1.0/0.5 или подстрочная семантика (сравнения делать относительными, не числовыми) → `cargo test -p unica-coder platform_help` PASS целиком.
- [ ] Коммит: `feat(platform-help): пословный морфологический матчинг на лексическом ядре (#415)`.

### Task 8: configuration-help на ядре

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/configuration_help.rs`

- [ ] Тест: запрос «карточка номенклатуры» (морфология+порядок) находит страницу справки `Catalogs.Номенклатура`; терм в заголовке выше терма в тексте.
- [ ] FAIL → построение `RetrievalIndex` по страницам набора исходников в `search` (корпус мал, индекс на вызов), скоринг ядром → PASS, существующие тесты обновлены.
- [ ] Коммит: `feat(documentation): справка конфигурации на лексическом ядре (#415)`.

### Task 9: kb-1ci — токены, en-стемминг, лексикон

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/kb_1ci.rs`
- Modify: композиционный корень реестра поставщиков (`internal_adapters.rs` либо место сборки `DocumentationRegistry`) — прокинуть источник лексикона.

**Interfaces:**
- Consumes: `RetrievalIndex` (документы — только заголовки узлов), `BilingualLexicon` через `pub trait KbLexiconSource: Send + Sync { fn lexicon(&self, context: &DocumentationContext) -> Option<std::sync::Arc<BilingualLexicon>> }`; боевая реализация зовёт `platform_help::bilingual_lexicon_for` при `context.installation_root == Some(..)`, тестовая — подменная.
- Матчинг: вместо `title.to_lowercase().contains(needle)` — индекс по заголовкам выбранных руководств, `query(..., expansions)`; порядок попаданий — по оценке; `PAGE_FETCH_CAP`, дочитывание и статусы не меняются.

- [ ] Тесты: английский многословный запрос `"value table group"` находит узел `...ValueTable...` (сейчас — нет); русский запрос при наличии лексикона находит английский заголовок; без лексикона — Empty (честная деградация); существующие тесты обхода/отказов не трогаются.
- [ ] FAIL → реализация → `cargo test -p unica-coder kb_1ci` PASS.
- [ ] Коммит: `feat(kb-1ci): пословный матчинг заголовков и ru→en расширение запроса (#415)`.

### Task 10: v8std — нормализация запроса и TTL-кеш в памяти

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/standards_documentation.rs`

**Interfaces:**
- Нормализация: trim + схлопывание пробельных серий в один пробел; уходит на сервер нормализованный запрос.
- Кеш: `static V8STD_SEARCH_CACHE: Mutex<BTreeMap<(String, String, usize), (Instant, String)>>` — ключ (endpoint, нормализованный запрос, limit), значение — сырое успешное тело ответа; TTL `V8STD_CACHE_TTL = 6h` (симметрично kb); политика и отмена проверяются ДО кеша; policy-denied не читает и не пишет кеш; неуспех не кешируется; `#[cfg(test)]`-сброс кеша по образцу kb.

- [ ] Тесты: второй одинаковый запрос не зовёт транспорт (счётчик вызовов фейкового `HttpClient`); истёкший TTL зовёт снова; `deny` отвечает policy-denied и кеша не касается; `"  запрос   с   пробелами  "` уходит на сервер как `"запрос с пробелами"`.
- [ ] FAIL → реализация → `cargo test -p unica-coder standards_documentation` PASS.
- [ ] Коммит: `feat(v8std): нормализация запроса и кеш ответов поиска в памяти (#415)`.

### Task 11: Ретривал-гейт против реальной установки

**Files:**
- Create: `crates/unica-coder/src/infrastructure/platform_help/retrieval_gate.rs` (`#[cfg(test)]`; модуль объявляется рядом с provider)

**Interfaces:**
- Запуск по образцу живого kb-прогона: без `UNICA_RETRIEVAL_GATE=1` тест сразу выходит с пометкой пропуска; корень установки — `UNICA_RETRIEVAL_ROOT` (по умолчанию `/opt/1cv8/8.3.27.2074`).
- Golden-набор — таблица из #415 плюс ожидания: каждый запрос несёт ожидаемую подстроку пути/заголовка и корпус; assert: ожидание в топ-5 секции. Латентность: после первого (холодного) вызова каждый тёплый запрос обязан укладываться в 1 с; холодное построение печатается в вывод.

- [ ] Написать гейт с запросами: `СтрНайти`→`StrFind` в топ-5 syntax-context; `свернуть таблицу значений`→`GroupBy` (ValueTable); `как удалить элемент массива`→`Array.Delete`; `регистр сведений срез последних`→`SliceLast`; `ValueTable GroupBy`→страница метода; `СтрНайтти`→`StrFind`; `Свернуть`→`ValueTable.GroupBy` в топ-5.
- [ ] `UNICA_RETRIEVAL_GATE=1 cargo test -p unica-coder retrieval_gate -- --nocapture` — PASS на этой машине (это и есть «сначала красный»: до задач 7–9 гейт падает, прогнать в обе стороны).
- [ ] Коммит: `test(platform-help): ретривал-гейт golden-запросов против установки 8.3.27 (#415)`.

### Task 12: Скилл, ведомость, полные прогоны

**Files:**
- Modify: `plugins/unica/skills/platform-help/SKILL.md` — шаг 2 Workflow: имя объекта/члена ИЛИ естественная формулировка; добавить MCP-пример с запросом `"свернуть таблицу значений"`.
- Проверки: `python3.12 scripts/ci/generate-tool-surface.py --check` (дескрипторы не менялись — обязан пройти без регенерации).

- [ ] Обновить скилл; `cargo test -p unica-coder` целиком; `/opt/homebrew/bin/python3.12 -m pytest tests/ci -q`; `/opt/homebrew/bin/python3.12 -m pytest tests/dev -q`; `cargo clippy -p unica-coder -- -D warnings`; повторить живой замер из #415 probe-скриптом и сохранить «после»-таблицу для PR.
- [ ] Коммит: `docs(skills): platform-help — естественные формулировки запросов (#415)`.

### Task 13: Pull request

- [ ] `git push -u origin claude/kb-search-semantics-a06528`; `gh pr create --repo IngvarConsulting/unica --base main` — заголовок `feat(documentation): лексическое ядро поиска — пословный, морфологический и нечёткий матчинг (#415)`; тело: Closes #415, до/после-таблица замера, перечень решений (ADR-0035, INV-MCP-SEARCH-SEMANTICS), команды верификации.

## Self-Review

- Покрытие issue #415: п.1 токенизация — Task 1; п.2 пословность — Task 4; п.3 морфология — Task 2; п.4 нечёткость — Tasks 3–4; п.5 BM25F — Task 4; п.6 kb — Tasks 6, 9; п.7 v8std — Task 10; рамки (ADR, гейт, скилл) — Tasks 5, 11, 12. Пробелов нет.
- Отступления от текста issue, называемые в PR: лексикон строится из двуязычных заголовков (`.st`-сигнатуры избыточны — те же имена); переранжирование дочитанных kb-страниц по тексту не реализуется (issue: «можно», не «обязательно»).
- Типы согласованы: `RetrievalFields`/`RetrievalIndex`/`RetrievalHit`/`BilingualLexicon` определены в Task 4/6 и потребляются в Tasks 7–9 под теми же именами.
