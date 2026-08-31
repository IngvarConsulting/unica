# Ведомость публичной поверхности инструментов

Порождается `scripts/ci/generate-tool-surface.py` из `tools/list` собранного бинаря. Руками правится только [`tool-surface-review.json`](tool-surface-review.json): контракт результата и сценарии. Имена, описания и аргументы принадлежат реестру в `crates/unica-coder/src/application/mod.rs` и `tool_contracts.rs`; здесь они лишь показаны рядом (`CTR.WIRE.TOOL-SURFACE`).

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

—

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `at` | string | да | qualified logical address |
| `dryRun` | boolean | нет | — |
| `ifRev` | string | нет | — |
| `ops` | array | да | — |

**Результат сейчас:** Для `props.set` и `attribute.add/set/remove` доказаны общий ordered staged planner, одинаковый postimage/effect plan hash в dry-run/real и атомарная retained-публикация (отвечают типизированным `data`)

**Целевой контракт:** Спроектировать недостающие object/relation contracts, затем переносить остальные типизированные семейства операций

**Сценарии:**

- Изменить свойство через доказанную retained-публикацию `props.set`
- Добавить, изменить и удалить атрибут с одинаковым доказуемым dry-run/real планом

## check

### `unica.check`

—

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `at` | string | нет | qualified logical address |
| `filter` | object | нет | — |

**Результат сейчас:** Без фильтра доказывает admission source set или читаемость указанного `at`; filter validation и маршрутизация meta/cf существуют только provisional, реальное исполнение валидатора не доказано (отвечают типизированным `data`)

**Целевой контракт:** Подключить реальные валидаторы и доказать их канонические diagnostics до объявления профилей поддержанными

**Сценарии:**

- Проверить, что рабочее пространство и его source set допущены
- Проверить читаемость конкретного логического узла

## diff

### `unica.diff`

—

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `cursor` | string | нет | — |
| `filter` | object | нет | — |
| `left` | string | да | qualified logical address |
| `limit` | integer | нет | — |
| `right` | string | да | qualified logical address |

**Результат сейчас:** Сравнивает два узла одного логического вида и возвращает bounded JSON changes с общей revision; закрытые `paths`/`sections` фильтры поддержаны, cursor пока неподдержан (отвечают типизированным `data`)

**Целевой контракт:** Добавить предметные diff-проекции и revision-bound pagination

**Сценарии:**

- Сравнить две логические проекции одного вида
- Доказать равенство узлов без чтения физических файлов

## docs

### `unica.docs`

—

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `query` | string | да | — |
| `source` | string | нет | — |

**Результат сейчас:** Поиск по platform-help и development-standard возвращает `data.sections`; configuration-documentation отвечает `unsupported_source` до actor-safe reader (отвечают типизированным `data`)

**Целевой контракт:** Добавить actor-owned nofollow/cancellation reader для документации конфигурации, адресное получение документа, locale и version

**Сценарии:**

- Искать по справке платформы или стандартам
- Получить typed unsupported для документации конфигурации без обхода actor boundary

## find

### `unica.find`

—

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `kind` | string | нет | — |
| `limit` | integer | нет | — |
| `query` | string | да | — |

**Результат сейчас:** `data.candidates` содержит детерминированные кандидаты квалифицированных логических адресов (отвечают типизированным `data`)

**Целевой контракт:** Добавлять закрытые виды поиска без возврата к физическим selector-ам

**Сценарии:**

- Разрешить имя объекта в квалифицированный логический адрес
- Найти несколько кандидатов перед точным `view` или `apply`

## run

### `unica.run`

—

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `args` | object | нет | — |
| `op` | string | нет | — |

**Результат сейчас:** Вызов без `op` возвращает закрытый словарь; `syntax.check` выполняется как durable cancellable Task с пятиминутным process timeout, bounded capture и закрытым terminal/provider результатом; остальные операции неподдержаны (отвечают типизированным `data`)

**Целевой контракт:** Подключать следующие операции только через такие же bounded Task и закрытые terminal/provider-контракты

**Сценарии:**

- Получить машинно-читаемый словарь допустимых runtime намерений
- Запустить bounded проверку синтаксиса без публикации raw stdout, command или artifact path

## search

### `unica.search`

—

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `limit` | integer | нет | — |
| `query` | string | да | — |
| `regex` | boolean | нет | — |
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

—

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `at` | string | да | qualified logical address |
| `cursor` | string | нет | — |
| `filter` | object | нет | — |
| `limit` | integer | нет | — |

**Результат сейчас:** `data` содержит типизированную проекцию логического узла; поддержаны квалифицированный `at`, базовое чтение, revision, bounded cursor и закрытые секции `props`/`branches`/`can`/`limits`/`items` (отвечают типизированным `data`)

**Целевой контракт:** Расширять проекции через закрытые `filter`, не возвращая физические пути

**Сценарии:**

- Прочитать конфигурацию или объект метаданных по квалифицированному адресу
- Получить наблюдаемую структуру узла и revision для последующей проверки
