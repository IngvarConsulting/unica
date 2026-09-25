# Ведомость публичной поверхности инструментов

Порождается `scripts/ci/generate-tool-surface.py` из `tools/list` собранного бинаря. Руками правится только [`tool-surface-review.json`](../tests/fixtures/v013/tool-surface-review.json): контракт результата и сценарии. Имена, описания и аргументы принадлежат реестру v0.13 в `crates/unica-coder/src/application/v13/tool_catalog.rs`; здесь они лишь показаны рядом.

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
| `ifRev` | string | по условию | Revision returned by a prior dryRun preview; required when dryRun is false. |
| `ops` | array | да | Ordered operations advertised by the target node's can data. |

**Результат сейчас:** Для `props.set` и `attribute.add/set/remove` доказаны общий ordered staged planner, одинаковый postimage/effect plan hash в dry-run/real и атомарная retained-публикация (отвечают типизированным `data`)

**Целевой контракт:** Спроектировать недостающие object/relation contracts, затем переносить остальные типизированные семейства операций

**Сценарии:**

- Изменить свойство через доказанную retained-публикацию `props.set`
- Добавить, изменить и удалить атрибут с одинаковым доказуемым dry-run/real планом

## check

### `unica.check`

Confirm workspace source-set admission, or validate one logical node: readability plus every validator its kind owns. Node diagnostics are returned in stable pages.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `at` | string | нет | Qualified logical address: <sourceSet>:<Kind>[.<Name>...]. Omit only for workspace bootstrap where allowed. |
| `cursor` | string | нет | Continuation cursor from an earlier check of the same node. |
| `limit` | integer | нет | Maximum diagnostics in one node-check page (default 20, maximum 50). |

**Результат сейчас:** Без `at` доказывает admission source set; с `at` читает узел и запускает все валидаторы его вида (`cf`/`cfe` для корня по виду набора, `form`, `dcs`/`mxl` по `TemplateType`, `role`, `subsystem`, `interface`, `meta`, `bsl`), отдавая `status`, `validators` и диагностики страницами; узел без валидаторов отвечает читаемостью (отвечают типизированным `data`)

**Целевой контракт:** Держать таблицу вид → валидаторы закрытой и доказанной корпусом; на проводе у `check` нет аргумента выбора валидатора

**Сценарии:**

- Проверить, что рабочее пространство и его source set допущены
- Проверить один логический узел всеми валидаторами его вида и дочитать все страницы диагностик
- Проверить читаемость узла без валидаторов

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

Search bundled Unica and safe 1C documentation by topic, or open a document locator. Search hits and long document text use pages; each search source reports whether its retrieved window is complete.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `cursor` | string | нет | Continue the same documentation search or long document. A document cursor checks its complete text and metadata for changes; concatenate document.text fragments in page order. |
| `limit` | integer | нет | Maximum hits per search page or text fragments per long document page, from 1 to 50. A text fragment is one line or at most 16 KiB of a longer line; short documents still arrive whole. |
| `query` | string | да | Documentation question or search phrase. |
| `source` | string | нет | Optional documented source kind, not a provider identity. |

**Результат сейчас:** Отвечает до допуска рабочей области; поиск по platform-help и development-standard возвращает страницы `data.sections` с `searchComplete` для каждой секции и всего ответа. В пределах бюджета страницы совпадения чередуются между корпусами; курсор сверяет полный полученный ответ. Локальные поставщики перечисляют все совпадения; адаптер v8std дочитывает страницы по курсору поставщика, а прежний сервер без курсора оставляет заполненное окно из 50 результатов явно неполным. `page.stoppedBy: complete` означает конец полученного окна, а не обязательно всего поиска. Локатор открывает короткий документ целиком, а длинный текст — точными страницами с byte range и курсором, связанным с полным документом. configuration-documentation отвечает `unsupported_source` до actor-safe reader (отвечают типизированным `data`)

**Целевой контракт:** Добавить actor-owned nofollow/cancellation reader для документации конфигурации, выбор locale и version

**Сценарии:**

- Искать по справке платформы или стандартам
- Получить typed unsupported для документации конфигурации без обхода actor boundary
- Спросить справку из каталога, который ещё не рабочая область

## resolve

### `unica.resolve`

Emergency bridge between a logical address and the source layout, in both directions. Use it only when a path arrived from outside Unica - a diff, a build log, a stack trace - or when a file has to be opened outside Unica. To find an object by name use search; to read it use view.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `at` | string | нет | Qualified logical address whose source location is needed. |
| `path` | string | нет | Path to a source file or object directory, absolute or relative to the workspace root. |

**Результат сейчас:** `data` несёт один предмет: `at`, `kind`, `path` и закрытый признак `lines`; ответ точный либо `not_found` и не несёт `rev` (отвечают типизированным `data`)

**Целевой контракт:** Держать путь в одном редком инструменте: в частом ответе он звал бы читать файл мимо адреса

**Сценарии:**

- Узнать, какому объекту принадлежит путь, пришедший из диффа, лога сборки или трассы
- Получить путь к файлу или каталогу объекта, чтобы прочитать или починить его вне Unica
- Узнать файл и диапазон строк метода, когда его открывают вне Unica

## run

### `unica.run`

List canonical runtime operations and their invocation contract, or preview/execute one implemented operation.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `args` | object | нет | Typed arguments for the selected operation. |
| `dryRun` | boolean | нет | Required by previewApply operations: true returns a non-mutating plan and revision; false requires ifRev and applies that plan. |
| `ifRev` | string | нет | Revision returned by a prior preview of the same previewApply operation; required when dryRun is false. |
| `infobase` | string | нет | Named infobase; defaults to origin. The runner 0.11 adapter supports only origin. |
| `op` | string | нет | Runner 1.0 operation name; omit to list the target dictionary and adapter support. |

**Результат сейчас:** Все 13 операций целевого словаря исполнимы через закреплённый адаптер 0.11.2. push исходников и pull требуют force; upload загружает без применения БД; apply/reset разделены, reset требует force. infobase.create создаёт пустую базу. Шесть операций ограничены и публикуют поддержанную схему и отсутствующие гарантии. (отвечают типизированным `data`)

**Целевой контракт:** Ограничены шесть операций разработки. Адаптер 1.0 расширяет те же имена контролем поколений и синхронизацией; ifRev сейчас защищает план и локальные входы, а не поколение базы.

**Сценарии:**

- Получить машинно-читаемый словарь допустимых runtime намерений
- Предпросмотреть и выгрузить main CF, extension CFE или полный DT из существующей ИБ
- Различить сборку артефакта, экспорт конфигурации и полный снимок ИБ без выбора platform provider моделью

## search

### `unica.search`

Search one corpus for a query: BSL module text, or the names and synonyms of metadata objects. Optionally under one logical subtree. Results use pages; provider roles report whether their finite search window is complete. Names report descriptor-read coverage separately from approximate name matching.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `corpus` | string | нет | Where to search: `text` matches BSL module content and answers scope, line, column and snippet; `names` matches metadata names and synonyms and answers at, kind and title. Defaults to `text`. |
| `cursor` | string | нет | Continue a previous search page. Bound to the question, source sets, page limit and the relevant revision or complete retrieved result. |
| `kind` | string | нет | `names` corpus only: narrow the search to one logical node kind. |
| `limit` | integer | нет | Maximum matches per page, from 1 to 50. Provider roles may stop after their first 200 retrieved matches and mark the search incomplete. |
| `query` | string | да | Literal BSL text, symbol, or metadata name to search for. |
| `regex` | boolean | нет | Use a regular expression for local text search. |
| `role` | string | нет | `text` corpus only: which provider answers. `lexical` matches literally, `symbol` uses the symbol index, `semantic` matches by meaning. Omit for the literal search Unica performs itself. |
| `scope` | string | нет | logical subtree address |

**Результат сейчас:** Локальный поиск по BSL и именам и поиск через поставщика возвращают `data.matches`, `page.stoppedBy` и, пока есть следующие полученные совпадения, `cursor`. Текстовый курсор повторно читает исходники и проверяет их ревизии; курсоры имён и поставщика повторно собирают ответ и проверяют его отпечаток. Если проверяемые сведения изменились, приходит `stale_cursor`; повтор того же курсора возвращает ту же страницу. Локальный текстовый режим поддерживает литерал и regex. Кодовая роль запрашивает до 200 совпадений, но внутренние пределы bsl-analyzer и RLM могут остановить поиск раньше. Последняя страница полученного окна имеет `page.stoppedBy: complete`; при достижении квоты секция сохраняет `searchComplete: false`, `status: limitReached` и нижнюю оценку числа совпадений; при потере результата статус становится `partial`. При 200 полученных совпадениях ответ рекомендует уточнить запрос. (отвечают типизированным `data`)

**Целевой контракт:** достигнут

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
| `limit` | integer | нет | Maximum child items per addressed view page; a preferred 64 KiB page size may stop earlier, but an indivisible item remains whole. |

**Результат сейчас:** Без аргументов `data` описывает workspace, `v8project.yaml`, source sets, infobase target, readiness и только релевантный setup; infobase-only workspace получает точные preview-продолжения CF и DT; обычная адресная коллекция `view` возвращает до 20 элементов по умолчанию (максимум 50) с целевым размером страницы 64 КиБ; неделимый элемент возвращается целиком до технического предела результата, `page.stoppedBy` называет `limit`, `bytes` или `complete`, курсор продолжает ту же ревизию; ветвь графа вызовов пока не проходит через эту пагинацию; с квалифицированным `at` узел содержит закрытые секции `props`/`branches`/`can`/`limits`/`items` (отвечают типизированным `data`)

**Целевой контракт:** Расширять проекции через закрытые `filter`, не возвращая физические пути

**Сценарии:**

- Обнаружить workspace и получить точный рецепт v8project.yaml до source admission
- Распознать существующую ИБ без исходников и предложить preview выгрузки CF или DT
- Прочитать конфигурацию или объект метаданных по квалифицированному адресу
- Получить наблюдаемую структуру узла и revision для последующей проверки
