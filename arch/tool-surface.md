# Ведомость публичной поверхности инструментов

Порождается `scripts/ci/generate-tool-surface.py` из `tools/list` собранного бинаря. Руками правится только [`tool-surface-review.json`](tool-surface-review.json): контракт результата и сценарии. Имена, описания и аргументы принадлежат реестру v0.13 в `crates/unica-coder/src/application/v13/tool_catalog.rs`; здесь они лишь показаны рядом (`CTR.WIRE.TOOL-SURFACE`).

Колонка «Результат сейчас» — наблюдение ревью, а не машинный факт: страж проверяет полноту охвата и совпадение аргументов с реестром, но не читает поведение обработчика.

## Итог

- Инструментов: **11**
- Отвечают типизированным `data`: **11**
- Типизированы частично: часть результата всё ещё текст: **0**
- Отвечают снимком задания в `job`: **0**
- Отвечают прозой в `stdout`: **0**

- В границах типизации: **11**
- Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`): **0**
- Вне границ: семейство runtime и build изучается отдельно: **0**
- Осталось перевести на типизированный `data` в границах работы: **0**
- Публикуют больше 20 аргументов из общего списка: **0**

## apply

### `unica.apply`

Preview or atomically apply typed edits to one logically addressed 1C node.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `at` | string | да | Qualified logical address: <sourceSet>:<Kind>[.<Name>...]. Omit only for workspace bootstrap where allowed. |
| `dryRun` | boolean | нет | Validate and return the plan without publishing when true. |
| `ifRev` | string | нет | Optional revision fence from an earlier read. |
| `ops` | array | да | Ordered operations advertised by the target node's can data. |

**Результат сейчас:** Для `props.set` и `attribute.add/set/remove` доказаны общий ordered staged planner, одинаковый postimage/effect plan hash в dry-run/real и атомарная retained-публикация (отвечают типизированным `data`)

**Целевой контракт:** Спроектировать недостающие object/relation contracts, затем переносить остальные типизированные семейства операций

**Сценарии:**

- Изменить свойство через доказанную retained-публикацию `props.set`
- Добавить, изменить и удалить атрибут с одинаковым доказуемым dry-run/real планом

## check

### `unica.check`

Confirm workspace source-set admission, or validate one logical node's readability.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `at` | string | нет | Qualified logical address: <sourceSet>:<Kind>[.<Name>...]. Omit only for workspace bootstrap where allowed. |
| `filter` | object | нет | Optional validation profile; requires at. |

**Результат сейчас:** Без фильтра доказывает admission source set или читаемость указанного `at`; filter validation и маршрутизация meta/cf существуют только provisional, реальное исполнение валидатора не доказано (отвечают типизированным `data`)

**Целевой контракт:** Подключить реальные валидаторы и доказать их канонические diagnostics до объявления профилей поддержанными

**Сценарии:**

- Проверить, что рабочее пространство и его source set допущены
- Проверить читаемость конкретного логического узла

## diff

### `unica.diff`

Compare two readable logical nodes of the same kind without changing files.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `cursor` | string | нет | Continuation cursor from an earlier diff. |
| `filter` | object | нет | Optional projection applied before comparison. |
| `left` | string | да | Qualified logical address of the left node. |
| `limit` | integer | нет | Maximum differences to return. |
| `right` | string | да | Qualified logical address of the right node. |

**Результат сейчас:** Сравнивает два узла одного логического вида и возвращает bounded JSON changes с общей revision; закрытые `paths`/`sections` фильтры поддержаны, cursor пока неподдержан (отвечают типизированным `data`)

**Целевой контракт:** Добавить предметные diff-проекции и revision-bound pagination

**Сценарии:**

- Сравнить две логические проекции одного вида
- Доказать равенство узлов без чтения физических файлов

## docs

### `unica.docs`

Search bundled Unica and safe 1C documentation by topic.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `query` | string | да | Documentation question or search phrase. |
| `source` | string | нет | Optional documented source kind, not a provider identity. |

**Результат сейчас:** Поиск по platform-help и development-standard возвращает `data.sections`; configuration-documentation отвечает `unsupported_source` до actor-safe reader (отвечают типизированным `data`)

**Целевой контракт:** Добавить actor-owned nofollow/cancellation reader для документации конфигурации, адресное получение документа, locale и version

**Сценарии:**

- Искать по справке платформы или стандартам
- Получить typed unsupported для документации конфигурации без обхода actor boundary

## find

### `unica.find`

Resolve a human query to canonical logical address candidates.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `kind` | string | нет | Optional logical kind such as Catalog or CommonModule. |
| `limit` | integer | нет | Maximum address candidates to return. |
| `query` | string | да | Object name or address fragment to resolve. |

**Результат сейчас:** `data.candidates` содержит детерминированные кандидаты квалифицированных логических адресов (отвечают типизированным `data`)

**Целевой контракт:** Добавлять закрытые виды поиска без возврата к физическим selector-ам

**Сценарии:**

- Разрешить имя объекта в квалифицированный логический адрес
- Найти несколько кандидатов перед точным `view` или `apply`

## run

### `unica.run`

List canonical runtime operations, or execute one implemented operation.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `args` | object | нет | Typed arguments for the selected operation. |
| `op` | string | нет | Canonical operation name; omit to list operation status. |

**Результат сейчас:** Вызов без `op` возвращает закрытый словарь; `syntax.check` выполняется как durable cancellable Task с пятиминутным process timeout, bounded capture и закрытым terminal/provider результатом; остальные операции неподдержаны (отвечают типизированным `data`)

**Целевой контракт:** Подключать следующие операции только через такие же bounded Task и закрытые terminal/provider-контракты

**Сценарии:**

- Получить машинно-читаемый словарь допустимых runtime намерений
- Запустить bounded проверку синтаксиса без публикации raw stdout, command или artifact path

## search

### `unica.search`

Search BSL content or symbols, optionally under one logical subtree.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `limit` | integer | нет | Maximum matches to return. |
| `query` | string | да | Literal BSL text or symbol to search for. |
| `regex` | boolean | нет | Request regex matching; currently only false is implemented. |
| `scope` | string | нет | logical subtree address |

**Результат сейчас:** Литеральный bounded-поиск по BSL возвращает `data.matches` для `Configuration` и разрешённого поддерева объекта метаданных; regex и symbol остаются неподдержанными (отвечают типизированным `data`)

**Целевой контракт:** Добавить символический и regex-режимы через закрытые варианты контракта

**Сценарии:**

- Найти буквальное вхождение в BSL внутри source set
- Ограничить поиск логическим корнем конфигурации

## task

### `unica.task.cancel`

Idempotently request cancellation and return the current durable Task state without re-running the subject tool.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `taskId` | string | да | Opaque Task identifier returned by Unica |

**Результат сейчас:** Идемпотентно запрашивает отмену и возвращает текущее durable состояние Task (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Отменить Task в клиенте без native Tasks

### `unica.task.get`

Read the current durable Task state immediately without waiting or re-running the subject tool.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `taskId` | string | да | Opaque Task identifier returned by Unica |

**Результат сейчас:** Возвращает текущий durable Task state без повторного исполнения предметного вызова (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Немедленно получить состояние Task в клиенте без native Tasks

### `unica.task.result`

Wait for a Task result for a bounded interval; returns the canonical result or a new working receipt without re-running the subject tool.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `taskId` | string | да | Opaque Task identifier returned by Unica |
| `waitMs` | integer | нет | Bounded wait in milliseconds; defaults to 7000 |

**Результат сейчас:** Ждёт не более 7000 мс и возвращает terminal result либо новый working receipt без повторного исполнения (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Ожидать результат Task в compatibility-профиле

## view

### `unica.view`

Inspect the workspace with no arguments, or read one logical 1C node by address.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `at` | string | нет | Qualified logical address: <sourceSet>:<Kind>[.<Name>...]. Omit only for workspace bootstrap where allowed. |
| `cursor` | string | нет | Continuation cursor from an earlier addressed view. |
| `filter` | object | нет | Optional projection such as sections; valid only with at. |
| `limit` | integer | нет | Maximum child items to return; valid only with at. |

**Результат сейчас:** Без аргументов `data` описывает workspace, `v8project.yaml`, source sets, readiness и setup; с квалифицированным `at` содержит типизированную проекцию логического узла, revision, bounded cursor и закрытые секции `props`/`branches`/`can`/`limits`/`items` (отвечают типизированным `data`)

**Целевой контракт:** Расширять проекции через закрытые `filter`, не возвращая физические пути

**Сценарии:**

- Обнаружить workspace и получить точный рецепт v8project.yaml до source admission
- Прочитать конфигурацию или объект метаданных по квалифицированному адресу
- Получить наблюдаемую структуру узла и revision для последующей проверки
