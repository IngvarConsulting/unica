# Issue #186 Compact Russian Screening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Добавить компактный русскоязычный итоговый обзор screening всех 19 источников issue #186 без дублирования полного доказательного корпуса.

**Architecture:** Новый документ является производным навигационным обзором. Полный английский screening остаётся evidence ledger и единственным владельцем SHA, точечных ссылок и развёрнутых карточек; русская версия обязана совпадать с ним по составу источников, решениям, `deep-dive`-кандидатам и пяти тематическим направлениям.

**Tech Stack:** Markdown, PowerShell 5.1, Python `unittest`/`pytest`, Git.

## Global Constraints

- Создать только `docs/provenance/reviews/2026-08-03-issue-186-source-screening-ru.md`; не менять runtime, package, MCP, архитектурные реестры или английский evidence ledger.
- Полный источник доказательств: `docs/provenance/reviews/2026-08-03-issue-186-source-screening.md`.
- Русская таблица содержит ровно те же 19 репозиториев и решения `deep-dive`, `defer`, `reject`.
- Отдельно раскрываются все семь `deep-dive`: `Menestre1/reasoning-bank-poc`, `Regsorm/code-index-mcp`, `feenlace/mcp-1c`, `Desko77/1c-formsserver`, `alexiosus/mxl-merge-tool`, `genlab-1c/prism`, `alonehobo/1c-trusted-gateway`.
- Сохраняются пять направлений: workflow/context, code intelligence, live data/safety, artifacts/documentation, benchmark/evaluation.
- Screening-черновик может готовиться без model override или на Luna; финальная проверка синхронности выполняется явно на `gpt-5.6-sol`.

---

### Task 1: Компактный русский screening

**Files:**
- Read: `docs/provenance/reviews/2026-08-03-issue-186-source-screening.md`
- Read: `docs/design/2026-08-04-issue-186-russian-screening-design.md`
- Create: `docs/provenance/reviews/2026-08-03-issue-186-source-screening-ru.md`

**Interfaces:**
- Consumes: английский summary registry, решения 19 карточек, cross-source normalization и thematic shortlist.
- Produces: компактный русский обзор со ссылкой на полный evidence ledger.

- [ ] **Step 1: Создать шапку и границу достоверности**

Указать дату среза `2026-08-03`, область `screening only`, обязательную перекрёстную проверку сильной моделью и прямую относительную ссылку на `2026-08-03-issue-186-source-screening.md`. Явно зафиксировать: при расхождении действует полный английский evidence ledger.

- [ ] **Step 2: Перенести решения всех 19 источников**

Создать русскую таблицу с колонками `Источник`, `Доказанный механизм`, `Значение для Unica`, `Решение`. Не добавлять новые утверждения, которых нет в английском summary registry или карточке источника.

- [ ] **Step 3: Раскрыть семь кандидатов deep-dive**

Для каждого кандидата дать три компактных пункта: `Что доказано`, `Что проверить в Unica`, `Минимальный bounded experiment`. Эксперимент брать из соответствующей карточки и тематического shortlist, не превращая неподтверждённую гипотезу в рекомендацию.

- [ ] **Step 4: Свести пять тематических направлений**

Для каждого направления назвать кандидатов, главный открытый вопрос и границу следующего исследования. Завершить рекомендуемым порядком: общая экспериментальная рамка → code intelligence → live data/safety → artifacts → evaluation; workflow/context исследовать только через измеримый сценарий.

- [ ] **Step 5: Проверить полноту локально**

Run:

```powershell
$source = Get-Content -Raw -Encoding UTF8 'docs/provenance/reviews/2026-08-03-issue-186-source-screening.md'
$russian = Get-Content -Raw -Encoding UTF8 'docs/provenance/reviews/2026-08-03-issue-186-source-screening-ru.md'
$repositories = [regex]::Matches($source, '^### `([^`]+/[^`]+)`$', 'Multiline') | ForEach-Object { $_.Groups[1].Value }
$missing = $repositories | Where-Object { $russian -notmatch [regex]::Escape($_) }
"sources=$($repositories.Count) missing=$($missing.Count)"
```

Expected: `sources=19 missing=0`.

- [ ] **Step 6: Commit**

```powershell
git add docs/provenance/reviews/2026-08-03-issue-186-source-screening-ru.md
git commit -m "docs(research): add compact Russian screening"
```

### Task 2: Sol consistency gate and final verification

**Files:**
- Review: `docs/provenance/reviews/2026-08-03-issue-186-source-screening.md`
- Review: `docs/provenance/reviews/2026-08-03-issue-186-source-screening-ru.md`
- Test: `tests/ci/test_design_documents.py`

**Interfaces:**
- Consumes: русский обзор из Task 1 и английский evidence ledger.
- Produces: подтверждение отсутствия смыслового дрейфа и готовый коммит для текущего PR.

- [ ] **Step 1: Выполнить read-only quality gate на `gpt-5.6-sol`**

Проверить ровно четыре свойства: 19/19 источников; точное совпадение решений; семь `deep-dive`; пять тематических направлений. Отдельно отметить любые русские формулировки, которые усиливают доказанность относительно английской карточки.

- [ ] **Step 2: Исправить Critical и Important замечания**

Править только русский документ. После исправления отправить каждое замечание на scoped re-review той же модели `gpt-5.6-sol`.

- [ ] **Step 3: Запустить проверки**

Run:

```powershell
git diff --check upstream/main...HEAD
python -m pytest tests/ci/test_design_documents.py -q -p no:cacheprovider
git diff --exit-code upstream/main...HEAD -- crates plugins spec tests scripts
```

Expected: `git diff --check` без вывода; `8 passed`; последний `git diff` без вывода.

- [ ] **Step 4: Опубликовать дополнение в существующий PR**

```powershell
git push origin codex/issue-186-screening
```

Expected: обновлён существующий PR #328; новый PR не создаётся.
