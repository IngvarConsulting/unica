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
$englishRows = [regex]::Matches(
  $source,
  '(?m)^\| \[([^\]]+/[^\]]+)\]\([^)]+\) \|.*\| `(deep-dive|defer|reject)` \|\r?$'
)
$russianRows = [regex]::Matches(
  $russian,
  '(?m)^\| `([^`]+/[^`]+)` \|.*\| `(deep-dive|defer|reject)` \|\r?$'
)
$englishDecisions = @{}
foreach ($row in $englishRows) {
  $englishDecisions[$row.Groups[1].Value] = $row.Groups[2].Value
}
$russianDecisions = @{}
foreach ($row in $russianRows) {
  $russianDecisions[$row.Groups[1].Value] = $row.Groups[2].Value
}
if ($englishRows.Count -ne 19 -or $englishDecisions.Count -ne 19) {
  throw "English inventory must contain 19 unique summary rows"
}
if ($russianRows.Count -ne 19 -or $russianDecisions.Count -ne 19) {
  throw "Russian inventory must contain 19 unique summary rows"
}
$missing = @($englishDecisions.Keys | Where-Object { -not $russianDecisions.ContainsKey($_) })
$extra = @($russianDecisions.Keys | Where-Object { -not $englishDecisions.ContainsKey($_) })
if ($missing.Count -or $extra.Count) {
  throw "Repository set drift: missing=$($missing -join ', '); extra=$($extra -join ', ')"
}
$decisionMismatches = @(
  $englishDecisions.Keys | Where-Object {
    $englishDecisions[$_] -ne $russianDecisions[$_]
  }
)
if ($decisionMismatches.Count) {
  throw "Decision drift: $($decisionMismatches -join ', ')"
}
$englishDeepDives = @(
  $englishDecisions.GetEnumerator() |
    Where-Object { $_.Value -eq 'deep-dive' } |
    ForEach-Object { $_.Key } |
    Sort-Object
)
$russianDeepDives = @(
  $russianDecisions.GetEnumerator() |
    Where-Object { $_.Value -eq 'deep-dive' } |
    ForEach-Object { $_.Key } |
    Sort-Object
)
$deepDiveDrift = @(Compare-Object $englishDeepDives $russianDeepDives)
if ($englishDeepDives.Count -ne 7 -or $deepDiveDrift.Count) {
  throw "Deep-dive set drift: English count=$($englishDeepDives.Count); differences=$($deepDiveDrift -join ', ')"
}
$themePairs = @(
  [pscustomobject]@{ English = '### Workflow, skills, and context management'; Russian = '### 1. Общая экспериментальная рамка: workflow/context' },
  [pscustomobject]@{ English = '### Code intelligence'; Russian = '### 2. Code intelligence' },
  [pscustomobject]@{ English = '### Live environments, data, and safety'; Russian = '### 3. Live data/safety' },
  [pscustomobject]@{ English = '### Artifacts and documentation'; Russian = '### 4. Artifacts/documentation' },
  [pscustomobject]@{ English = '### Benchmark and evaluation'; Russian = '### 5. Benchmark/evaluation' }
)
$russianThemeHeadings = [regex]::Matches($russian, '(?m)^### \d+\. .+\r?$')
if ($russianThemeHeadings.Count -ne $themePairs.Count) {
  throw "Russian theme count drift: expected $($themePairs.Count), found $($russianThemeHeadings.Count)"
}
foreach ($themePair in $themePairs) {
  if (-not $source.Contains($themePair.English) -or -not $russian.Contains($themePair.Russian)) {
    throw "Theme mapping drift: $($themePair.English) -> $($themePair.Russian)"
  }
}
"sources=$($englishDecisions.Count) russian=$($russianDecisions.Count) decisions=$($englishDecisions.Count) deep-dives=$($englishDeepDives.Count) themes=$($themePairs.Count)"
```

Expected: `sources=19 russian=19 decisions=19 deep-dives=7 themes=5`.

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
$allowedPaths = @(
  'docs/design/2026-08-03-issue-186-research-slicing-design.md',
  'docs/design/2026-08-04-issue-186-russian-screening-design.md',
  'docs/plans/2026-08-03-issue-186-source-screening.md',
  'docs/plans/2026-08-04-issue-186-russian-screening.md',
  'docs/provenance/reviews/2026-08-03-issue-186-source-screening.md',
  'docs/provenance/reviews/2026-08-03-issue-186-source-screening-ru.md'
)
$changedPaths = @(git diff --name-only upstream/main...HEAD)
if ($LASTEXITCODE -ne 0) {
  throw 'Unable to enumerate changed paths'
}
$unexpectedPaths = @($changedPaths | Where-Object { $_ -notin $allowedPaths })
if ($unexpectedPaths.Count) {
  throw "Unexpected changed paths: $($unexpectedPaths -join ', ')"
}
"changed=$($changedPaths.Count) unexpected=$($unexpectedPaths.Count)"
```

Expected: `git diff --check` без вывода; `8 passed`; `changed=6 unexpected=0`.

- [ ] **Step 4: Опубликовать дополнение в существующий PR**

```powershell
git push origin codex/issue-186-screening
```

Expected: обновлён существующий PR #328; новый PR не создаётся.
