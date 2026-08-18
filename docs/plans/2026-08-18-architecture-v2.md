# Architecture v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Завести `arch/` — нормативный слой v2 из трёх символических реестров с порождаемым индексом и собственными стражами — и наполнить его тем, что уже решено, не перенося ни одной записи v1.

**Architecture:** Три реестра (`decisions/`, `invariants/`, `contracts/`), по одному файлу на запись, front-matter props несут адресацию. Символ и путь выводятся друг из друга, поэтому навигация не требует индекса; порождаемый `arch/index.md` нужен для чтения и поиска. Стражи `tests/arch/` проверяют форму реестра, а не его содержание.

**Tech Stack:** Markdown с YAML front-matter, Python 3.12 (`/opt/homebrew/bin/python3.12`), unittest.

## Global Constraints

- Проза и документы — по-русски; идентификаторы, символы и сообщения коммитов — по-английски.
- Локальные тесты гоняются `/opt/homebrew/bin/python3.12 -m unittest`.
- Коммиты без GPG-подписи (`git -c commit.gpgsign=false commit …`).
- `docs/arch-v1/**` — архив: руками не правится, ссылки на него допустимы только как на историю.
- **Инвариант без существующей проверки не заводится.** Пока нет кода — есть решение, а не инвариант.
- **Ни одна запись v1 не портируется.** Предмет архивной записи попадает в v2 только когда всплывает в работе, и тогда пишется новое решение.
- Тесты поведения и формата (`cargo test`, `tests/ci`, `tests/dev`) сбросом не затронуты и остаются зелёными на каждом шаге.

## Принятые решения по открытым вопросам

| Вопрос | Решение | Почему |
| --- | --- | --- |
| Разделитель символа | точка: `INV.SURFACE.ONE-ADDRESS` | совпадает с разделителем имён инструментов — глазу не нужно переключаться |
| Перечень областей | открытый | закрытый требует решения заранее; дрейф виден в порождаемом индексе одной колонкой |
| `acceptance/`, `provenance/` | уже в архиве | уехали вместе с `spec/` при переносе; новые свидетельства заводятся как контракты |
| Потолок тела ADR | 40 строк | назначен по опыту чтения v1; правило стартовое и меняется решением |

---

## Task 1: Скелет `arch/` и страж формы

**Files:**
- Create: `arch/README.md`, `arch/decisions/.gitkeep`, `arch/invariants/.gitkeep`, `arch/contracts/.gitkeep`
- Create: `scripts/arch/registry.py` (чтение props, порождение индекса)
- Test: `tests/arch/__init__.py`, `tests/arch/test_registry_shape.py`

**Interfaces:**
- Produces: `registry.records(root) -> list[Record]` с полями `id`, `kind`, `path`, `props`, `body`; `registry.render_index(records) -> str`.
- Consumes: ничего.

- [ ] **Step 1: Написать падающий тест** — `tests/arch/test_registry_shape.py`:
  символ совпадает с именем файла; обязательные props присутствуют по виду записи;
  вид выводится из каталога; пустой реестр валиден.
- [ ] **Step 2: Прогнать, убедиться в падении** — `scripts/arch/registry.py` не существует.
- [ ] **Step 3: Реализовать чтение** — разбор front-matter без внешних зависимостей
  (YAML-подмножество: скаляры, списки, `null`), `Record` как `dataclass`.
- [ ] **Step 4: Зелёный прогон.**
- [ ] **Step 5: Commit** — `feat(arch): registry skeleton and shape guard`.

## Task 2: Ссылки, проверки и атомарность

**Files:**
- Modify: `scripts/arch/registry.py`
- Test: `tests/arch/test_registry_links.py`, `tests/arch/test_registry_atomicity.py`

- [ ] **Step 1: Падающие тесты** — каждая ссылка на символ разрешается;
  `superseded-by` взаимен `supersedes`; путь в `check:` существует и содержит
  названный тест; тело решения ≤ 40 строк и содержит ровно один блок `**Решение.**`.
- [ ] **Step 2: Убедиться в падении.**
- [ ] **Step 3: Реализовать резолвер** символов и проверку `check:` через
  `rg`-подобный поиск имени теста в файле.
- [ ] **Step 4: Зелёный прогон.**
- [ ] **Step 5: Commit** — `feat(arch): symbol resolution, check existence, atomicity cap`.

## Task 3: Порождаемый индекс

**Files:**
- Create: `arch/index.md` (порождается)
- Modify: `scripts/arch/registry.py` (CLI `--write-index`)
- Test: `tests/arch/test_registry_index.py`

- [ ] **Step 1: Падающий тест** — `index.md` совпадает с `render_index(records)`
  байт в байт; индекс отсортирован по символу; каждая строка несёт символ, вид,
  статус, суть и путь.
- [ ] **Step 2: Убедиться в падении.**
- [ ] **Step 3: Реализовать** `python3 scripts/arch/registry.py --write-index`.
- [ ] **Step 4: Зелёный прогон.**
- [ ] **Step 5: Commit** — `feat(arch): generated one-line-per-symbol index`.

## Task 4: Границы слоя

**Files:**
- Test: `tests/arch/test_layer_boundary.py`

- [ ] **Step 1: Падающие тесты** — ни один файл `arch/**` не содержит маркеров
  superpowers (`For agentic workers`, `**Goal:**`, `**Tech Stack:**`); ни один
  документ `docs/**` не объявляет символ реестра; `docs/arch-v1/**` не изменялся
  после коммита заморозки (сверка `git log` по пути).
- [ ] **Step 2: Убедиться в падении** на заведомо нарушающем временном файле.
- [ ] **Step 3: Реализовать.**
- [ ] **Step 4: Зелёный прогон.**
- [ ] **Step 5: Commit** — `feat(arch): layer boundary guard`.

## Task 5: Первые записи

**Files:**
- Create: `arch/decisions/2026-08-18-*.md`, `arch/invariants/INV.*.md`, `arch/contracts/CON.*.md`
- Modify: `arch/index.md` (перегенерация)

Содержание — только то, что уже решено и, для инвариантов, уже проверяемо:

- [ ] **Step 1: Решения** — сброс архитектуры, форма реестра, восемь входов,
  грамматика адреса, граница «узел или данные», снятие файлового DSL, форма
  результата.
- [ ] **Step 2: Инварианты** — только с существующей проверкой: пространство
  имён, один сервер, версии протокола, «публикуется то, что читается»,
  «сужение публикации не сужает приём», заморозка архива, граница superpowers.
- [ ] **Step 3: Контракты** — ведомость поверхности, профиль формата 8.3.27,
  проводной контракт протокола.
- [ ] **Step 4: Перегенерировать индекс, прогнать `tests/arch`.**
- [ ] **Step 5: Commit** — `feat(arch): first records`.

## Task 6: Переключение входа и снятие старых стражей

**Files:**
- Modify: `AGENTS.md`, `CLAUDE.md`
- Delete: `tests/ci/test_architecture_registry.py`, `tests/ci/test_architecture_sync_guard.py`
- Modify: `tests/ci/test_design_documents.py` (снять проверки формата v1)

- [ ] **Step 1:** `AGENTS.md` получает минимальный текст: где нормативный слой
  (`arch/`), где архив, где планирование, как гонять тесты.
- [ ] **Step 2:** Удалить стражей архитектурного слоя v1 — они стерегут форму,
  которой больше нет. Поведенческие и продуктовые тесты не трогаются.
- [ ] **Step 3:** Полный прогон `tests/ci`, `tests/arch`, `cargo test`.
- [ ] **Step 4: Commit** — `refactor!: arch/ becomes the normative layer`.

## Self-review

- Покрытие предложения: форма реестра → задачи 1–3; граница superpowers →
  задача 4; наполнение → задача 5; переключение → задача 6. Не покрыто
  намеренно: перенос 71 архивной записи (запрещён Global Constraints) и
  решение конфликта с ADR-0025 §4 (не решено владельцем, поэтому решения о
  словаре `op` в задаче 5 нет).
- Типы согласованы: `Record` из задачи 1 потребляется задачами 2–4.
- Плейсхолдеров нет: у каждой задачи названы файлы, проверки и команда коммита.
