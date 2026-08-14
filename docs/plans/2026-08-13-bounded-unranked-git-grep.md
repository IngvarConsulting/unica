# Bounded Unranked Git-Grep Search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task by task with the stated verification gates.

**Goal:** Сделать lexical-роль `git-grep` быстрым ограниченным резервным поиском: первые `limit` доказанных попаданий либо собственный deadline 2 секунды, с сохранением частичного результата и честной неполнотой.

**Architecture:** Общий role/progress/logical-location контракт берётся из уже слитого ADR-0056. `git-grep` пишет NUL-разделённые записи, process runner передаёт их потребителю потоково и умеет остановить дерево по решению callback. Поставщик проверяет область и логически локализует запись до увеличения accepted count. Он не сортирует и не присваивает rank; порядок объявлен как `providerTraversal`. Решение принадлежит только ADR-0058.

**Tech Stack:** Rust 2021, `std::process`, существующий `ManagedChild`, Git CLI, platform integration tests, Python 3.12 release-contract tests.

**PR boundary:** Начать самостоятельную ветку от актуального `origin/main` после слияния ADR-0056. Не базировать PR на открытой search-contract ветке и не включать RLM source revisions. Из PR #469 переносить только доказанные lifecycle/process исправления, не его 500 ms, скрытый limit 6 и старую архитектурную формулировку.

**Design source:** `docs/design/2026-08-13-bounded-unranked-git-grep-search-design.md`, `spec/decisions/0058-bounded-unranked-git-grep-search.md`.

## Терминальные состояния lexical-секции

| Причина | status | searchComplete | matches | hits |
| --- | --- | --- | --- | --- |
| Git exit 0 до limit | `ok` | `true` | exact N | N |
| Git exit 1 без строк | `empty` | `true` | exact 0 | 0 |
| accepted hits == limit | `limitReached` | `false` | lowerBound N | N |
| own/global timeout | `timedOut` | `false` | lowerBound N | 0..N |
| parent cancellation | вызов отменён | payload не публикуется | — | — |
| spawn/protocol/path failure | `failed` или `unavailable` | `false` | unknown | 0 |

Даже ноль попаданий при timeout не является `empty`. `limitReached` означает нижнюю границу, а не точный total. При `ranking:none` поля `rank` и `providerScore` отсутствуют.

## Task 1: Научить streaming process runner останавливаться по решению consumer

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/platform/process.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- Test: `crates/unica-coder/src/infrastructure/platform/process.rs`
- Test: `crates/unica-coder/src/infrastructure/internal_adapters.rs`

**Step 1: Написать падающие process tests**

Добавить helper mode, который бесконечно печатает строки и держит descendant process. Проверить:

1. callback возвращает stop на третьей строке;
2. `wait_for_line_output` завершается до process timeout;
3. результат содержит `stopped_by_consumer=true`, `timed_out=false`, `cancelled=false`;
4. callback после stop больше не вызывается, даже если pipe уже buffered;
5. leader и descendant reaped в пределах существующего bounded termination;
6. cancellation, произошедшая одновременно со stop, имеет приоритет и даёт `cancelled=true`;
7. diagnostics streaming с callback `Continue` сохраняет прежнее поведение.

Run: `cargo test -p unica-coder infrastructure::platform::process::tests -- --test-threads=1`

Expected: FAIL — callback возвращает `()` и runner не различает consumer stop.

**Step 2: Ввести явный control enum**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamControl {
    Continue,
    Stop,
}
```

Изменить signatures:

```rust
pub fn wait_for_line_output<F>(
    &mut self,
    max_line_bytes: usize,
    on_line: F,
) -> Result<ManagedLineOutput, String>
where
    F: FnMut(usize, &[u8]) -> StreamControl;
```

и `ProcessRunner::run_streaming` аналогично. `ManagedLineOutput` и `ProcessStreamOutput` получают `stopped_by_consumer: bool`.

**Step 3: Остановить process tree без публикации buffered tail**

`drain_line_messages` возвращает `StreamControl`. После первого `Stop`:

- снова проверить parent cancellation;
- вызвать hard `terminate()` для всего process tree;
- дочитать channel только в discard callback для завершения reader thread;
- не передавать consumer ни одной дополнительной строки;
- вернуть status с `stopped_by_consumer=true` независимо от exit code принудительно остановленного git.

Не кодировать consumer stop как timeout или cancellation: эти причины имеют разные wire-последствия.

**Step 4: Адаптировать единственного существующего streaming consumer**

`DiagnosticsJsonlParser::push_line` оборачивается в `StreamControl::Continue`. Добавить regression test, что все JSONL-строки по-прежнему прочитаны и `stopped_by_consumer=false`.

**Step 5: Запустить тесты и закоммитить**

Run: `cargo test -p unica-coder infrastructure::platform::process::tests infrastructure::internal_adapters::tests -- --test-threads=1`

Expected: PASS.

```bash
git add crates/unica-coder/src/infrastructure/platform/process.rs crates/unica-coder/src/infrastructure/platform/mod.rs crates/unica-coder/src/infrastructure/internal_adapters.rs
git commit -m "feat: останавливать потоковый процесс по пределу consumer"
```

## Task 2: Читать `git grep` как безопасный поток записей

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/code_intelligence.rs`
- Test: `crates/unica-coder/src/infrastructure/code_intelligence.rs`

**Step 1: Написать падающие parser tests**

Покрыть byte records:

```text
path\0line\0snippet\n
```

и случаи:

- path содержит `:` и пробел;
- snippet содержит `:` и не теряется;
- invalid UTF-8 path отклоняется диагностикой;
- invalid line number отклоняется;
- malformed record не увеличивает accepted count;
- строка вне логической области не увеличивает accepted count;
- logical locator failure не публикует raw path;
- accepted record получает `matchKind=literal`, но не rank/score.

Run: `cargo test -p unica-coder git_grep_record -- --test-threads=1`

Expected: FAIL — parser ожидает `path:line:snippet` string.

**Step 2: Изменить команду**

Целевые args:

```text
git -c core.quotepath=false grep --no-color --untracked --null -n -F -e QUERY -- PATHS...
```

Удалить `-m`: у git это предел на файл, а не глобальный предел ответа. `PATHS` выводятся из `CodeSearchScope`: весь source root либо точные/subtree pathspecs. Не передавать абсолютные пути.

**Step 3: Выделить bounded accumulator**

```rust
struct GitGrepAccumulator {
    limit: usize,
    read_rows: usize,
    read_bytes: u64,
    hits: Vec<ProviderSearchHit>,
    diagnostics: Vec<String>,
}

enum AcceptedRecord {
    Accepted,
    Rejected,
    Fatal,
}
```

Callback увеличивает `read_rows/read_bytes`, парсит NUL fields, проверяет `CodeSearchScope`, вызывает общий logical projector и только затем push-ит hit. На `hits.len() == limit` возвращает `StreamControl::Stop`.

**Step 4: Не сортировать результат**

Удалить `sort_by`, `truncate` и перенумерацию rank. Сохранять ровно порядок callback. Секция объявляет:

```text
ranking = none
ordering = providerTraversal
```

`matchKind=literal` — атрибут совпадения, не score.

**Step 5: Запустить parser tests и закоммитить**

Run: `cargo test -p unica-coder git_grep_record -- --test-threads=1`

Expected: PASS.

```bash
git add crates/unica-coder/src/infrastructure/code_intelligence.rs
git commit -m "refactor: читать git-grep как неранжируемый поток"
```

## Task 3: Спроецировать limit/timeout/exit в честную секцию

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/code_intelligence.rs`
- Modify: `crates/unica-coder/src/domain/operational_config.rs`
- Modify: `crates/unica-coder/src/infrastructure/operational_config.rs`
- Test: `crates/unica-coder/src/infrastructure/code_intelligence.rs`
- Test: `crates/unica-coder/src/domain/operational_config.rs`
- Test: `crates/unica-coder/src/infrastructure/operational_config.rs`

**Step 1: Написать падающую table-driven матрицу**

Собрать fake `ProcessStreamOutput` для каждой строки таблицы в начале плана. Дополнительно проверить:

- timeout с 3 hits возвращает эти 3 hits;
- timeout с 0 hits не возвращает `empty`;
- consumer stop ровно на limit даёт `limitReached`, даже если принудительный exit non-zero;
- natural exit 0 с N < limit даёт exact N;
- exit 1 + no rows/stderr даёт exact empty;
- exit 128/not repository даёт unavailable;
- line/projection fatal error даёт failed без raw artifacts;
- cancellation уничтожает partial data.

Run: `cargo test -p unica-coder git_grep_section -- --test-threads=1`

Expected: FAIL — timeout сейчас очищает hits и status не выражает неполноту.

**Step 2: Разрешать причину по приоритету**

Порядок:

```text
parent cancellation
fatal parse/scope/projection error
consumer stop (limitReached)
timeout
natural exit
```

Это не позволяет forced exit после consumer stop затереть правильную причину и не позволяет partial result пережить cancellation.

**Step 3: Зафиксировать двухсекундное умолчание**

В compiled config `SEARCH_GIT_GREP_DEFAULT_SECONDS = 2`. Файловый `search_git_grep_timeout_seconds` остаётся положительным integer seconds, не превышающим total. Нулевое/дробное значение отклоняется, а не превращается в «сразу пусто».

**Step 4: Публиковать полезный progress**

Lexical provider сообщает:

- `state=running, phase=preparing` при spawn;
- `state=running, phase=searching` со `readRows`, `acceptedHits`, `readBytes`;
- terminal state `completed`/`timedOut`/`failed` с elapsed;
- heartbeat поставляет последние counters без выдуманного процента.

Progress errors не меняют accumulator/process.

**Step 5: Запустить тесты и закоммитить**

Run: `cargo test -p unica-coder git_grep domain::operational_config infrastructure::operational_config -- --test-threads=1`

Expected: PASS.

```bash
git add crates/unica-coder/src/infrastructure/code_intelligence.rs crates/unica-coder/src/domain/operational_config.rs crates/unica-coder/src/infrastructure/operational_config.rs
git commit -m "feat: ограничить lexical-поиск limit или deadline"
```

## Task 4: Доказать поведение на реальном Git repository и process tree

**Files:**

- Create: `crates/unica-coder/tests/platform/code_search_git_grep.rs`
- Modify: `crates/unica-coder/tests/platform_code_intelligence.rs`
- Test: `crates/unica-coder/tests/platform/code_search_git_grep.rs`

**Step 1: Написать интеграционные тесты во временном repo**

Фикстура создаёт `.git`, tracked и untracked BSL/XML файлы в известном порядке. Тесты:

1. `limit=3` возвращает первые три publishable hit и `limitReached`.
2. Непубликуемый path перед тремя допустимыми не расходует limit.
3. Естественный negative search возвращает exact empty.
4. Медленный fake git wrapper успевает выдать две NUL-записи, затем timeout; обе записи сохранены.
5. Search по `metadataPath` не публикует соседний module.
6. Hit round-trip проходит в `unica.source.locate`/логический инструмент без физического path.
7. Parent cancellation reaps wrapper descendant и не отдаёт section payload.

Тест timeout использует injected runner/wrapper и короткий test-only deadline; он не ждёт реальные 2 секунды.

Run: `cargo test -p unica-coder --test platform_code_intelligence -- --test-threads=1`

Expected: FAIL до реализации, затем PASS.

**Step 2: Добавить platform wrapper**

Подключить новый файл из `platform_code_intelligence.rs` по принятому в этом test target шаблону. Не создавать отдельный `[[test]]`, если текущий wrapper уже собирает platform modules.

**Step 3: Запустить process + integration suite и закоммитить**

Run: `cargo test -p unica-coder infrastructure::platform::process::tests -- --test-threads=1`

Run: `cargo test -p unica-coder --test platform_code_intelligence -- --test-threads=1`

Expected: PASS.

```bash
git add crates/unica-coder/tests/platform/code_search_git_grep.rs crates/unica-coder/tests/platform_code_intelligence.rs
git commit -m "test: проверить bounded git-grep на реальном репозитории"
```

## Task 5: Синхронизировать ADR, registry, skill и release assessment

**Files:**

- Modify: `spec/decisions/0058-bounded-unranked-git-grep-search.md`
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/architecture/quality-requirements.md`
- Modify: `spec/architecture/tool-surface.md`
- Modify: `plugins/unica/skills/code-search/SKILL.md`
- Modify: `scripts/ci/release-assessment.py`
- Modify: `tests/ci/test_release_assessment.py`
- Modify: `tests/ci/test_architecture_registry.py`
- Modify: `tests/ci/test_unica_skills.py`

**Step 1: Написать падающие doc-contract tests**

Требовать literal markers:

```text
2 seconds
ranking none
providerTraversal
limitReached
timedOut
lowerBound
no git grep -m
```

Release validator должен принять lexical timeout с hits и отвергнуть `empty + timedOut` либо rank в unranked section.

Run: `python3.12 -m unittest tests.ci.test_release_assessment tests.ci.test_architecture_registry tests.ci.test_unica_skills`

Expected: FAIL до обновления docs/validator.

**Step 2: Обновить нормативные записи**

`INV-MCP-CODE-SEARCH-SECTIONS` ссылается на ADR-0056 и ADR-0058 для lexical completeness. Добавить/обновить измеримое требование `REQ-PERF-LEXICAL-BOUND`: default 2s либо limit, process tree reaped, partial hits preserved. Не создавать второй общий ADR поиска.

**Step 3: Обновить skill и validator**

Skill прямо говорит модели: lexical hits — первые в provider traversal, не top-K; timeout/limit result можно использовать как evidence, но нельзя считать полным. Release assessment проверяет contract, но не требует lexical `ok`: semantic/symbol могут быть полезны при lexical timeout.

**Step 4: Перевести ADR-0058 в accepted и закоммитить**

Сделать это только после зелёных unit/integration tests.

```bash
git add spec/decisions/0058-bounded-unranked-git-grep-search.md spec/architecture/invariants.md spec/architecture/quality-requirements.md spec/architecture/tool-surface.md plugins/unica/skills/code-search/SKILL.md scripts/ci/release-assessment.py tests/ci/test_release_assessment.py tests/ci/test_architecture_registry.py tests/ci/test_unica_skills.py
git commit -m "docs: принять bounded unranked git-grep"
```

## Task 6: Полная верификация и самостоятельный PR

**Files:**

- Verify: весь diff относительно `origin/main` и актуальное состояние PR #469.

**Step 1: Проверить выбранные изменения из PR #469**

Run: `gh pr diff 469 --repo IngvarConsulting/unica`

Сопоставить process lifecycle hunks с текущим diff. Не cherry-pick-ить commit целиком. В PR description перечислить, какие исправления перенесены и какие старые ограничения сознательно не перенесены.

**Step 2: Полные проверки**

Run: `cargo fmt --all -- --check`

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`

Run: `cargo test --workspace -- --test-threads=1`

Run: `python3.12 -m unittest discover -s tests/ci --durations 20`

Run: `python3.12 scripts/ci/check-architecture-sync.py --base origin/main --strict`

Run: `git diff --check origin/main...HEAD`

Expected: PASS.

**Step 3: Перебазировать только на main и открыть PR**

Обновить `origin/main`; если common search PR ещё не слит, остановиться — базировать этот PR на его head запрещено. После rebase повторить Step 2. Открыть PR с `Closes #467`, ADR-0058, таблицей terminal states и результатами process/integration/full checks.
