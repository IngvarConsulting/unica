# Реестр приёмочных сценариев поверхности v0.13

<!-- Сгенерировано scripts/ci/render-acceptance-registry.py из
     tests/fixtures/acceptance/scenario-corpus.json. Не правьте вручную:
     измените корпус и выполните `python scripts/ci/render-acceptance-registry.py --write`. -->

Корпус держит **379 сценариев** реальных задач разработчика конфигурации и **812 шагов** канонических вызовов `unica.*`. Каждый шаг заморожен в одном из классов ответа. Исходный профиль `tests/ci/test_acceptance_scenarios.py` исполняется против собранного `target/debug/unica`; основное рабочее пространство — `tests/fixtures/acceptance/workspace/`; сценарии форматных проб (выгрузка 2.21, без версии, без файла поддержки) идут на выведенном из него пространстве `tests/fixtures/acceptance/workspace-format/`, а сценарии ответа до допуска наборов — на пустом `tests/fixtures/acceptance/workspace-bare/`. Профиль delivery использует отдельную `tests/fixtures/acceptance/workspace-symbol/` с контрольным методом. СКД, MXL и вставка BSL проверяются на отдельных `workspace-dcs/`, `workspace-mxl/` и `workspace-code/`; агентные сценарии выбираются большим набором `tests/agent_evaluation/`. Источник истины — JSON корпуса; этот документ — его рендер для людей, и проверка на расхождение входит в тот же тест.

Профили исполнения: `source` — 367 сценария; `delivery/bsl-analyzer` — 7; `agent-evaluation/codex` — 2; `fault-injection/analyzer-jsonl-fault` — 3. Delivery-драйвер скачивает закреплённый движок с проверкой SHA256 и выполняет MCP на отдельной синтетической фикстуре. Ошибка доставки или незавершённая задача проваливает прогон. Агентный профиль исполняется отдельным большим набором в release/all: реальный Codex, закрытый аудит инструментов, проверки итогового XML и независимый смысловой reviewer; отсутствие CLI или авторизации означает отказ прогона. Fault-injection исполняет управляемый нативный producer повреждённого JSONL через настоящий процесс и публичный check; это не проверка опубликованного анализатора. Этот medium-профиль обязателен в очереди и не скачивает движок. Runtime добавляется с исполняемым драйвером; неизвестный профиль не пропускается.

## Классы ответа шага

| Класс | Что означает | Шагов |
| --- | --- | ---: |
| `ok` | результат | 696 |
| `refused` | умышленный типизированный отказ | 70 |
| `ok` / `provider` / `task` | любой из: результат; провайдер недоступен; длинная задача | 20 |
| `provider` | провайдер недоступен | 12 |
| `gap` | задокументированный пробел | 6 |
| `unsupported` | закрытый список | 3 |
| `ok` / `provider` | любой из: результат; провайдер недоступен | 2 |
| `ok` / `provider` / `task` / `unsupported` | любой из: результат; провайдер недоступен; длинная задача; закрытый список | 2 |
| `gap` / `unsupported` | любой из: задокументированный пробел; закрытый список | 1 |

Провод проходной, когда каждый его шаг отвечает результатом или типизированным отказом своего класса. Сырой транспортный сбой и незадокументированный типизированный отказ роняют прогон. Шагов с пробелом (`gap`) сейчас: **7** в **7 сценариях** (S299, S319, S325, S354, S355, S356, S357).

### Известные пробелы

Потолок корпуса — восемь gap-шагов. Известный ошибочный вердикт остаётся дефектом и не означает поддержку сценария.

| Сценарий | Наблюдение | Причина |
| --- | --- | --- |
| S299 | `gap` | Канонический `view` отказывает чужому курсору кодом `invalid_cursor`, который не входит в закрытый набор отказов поверхности (bad_value, not_found, invalid_state, invalid_source, stale_revision). |
| S319 | `gap` | Заимствование регистрирует общий модуль без каталога модуля, а первая запись модуля через `apply` его не создаёт: планировщик требует существующий каталог (`MissingParent`), и превью отвечает `provider_unavailable` «staged source evidence is unavailable» (#955, #1245). |
| S325 | `gap`: известный ошибочный `ok` (#791) | #791: связанный ChoiceProcessing имеет три параметра вместо пяти; check ошибочно отвечает passed. Исправление должно дать failed и требует снять gap. |
| S354 | `gap` | #896: IncludeHelpInContents отчёта отсутствует в типизированном props.set |
| S355 | `gap` | #852: props.set не принимает RegisterType=Turnovers после успешного создания регистра |
| S356 | `gap` / `unsupported` | #919: операция переноса существующего поля отсутствует в каноническом реестре |
| S357 | `gap` | #895: замена метода текстом из четырёх методов пока даёт postcondition_failed; в 13.1 согласована полная поддержка |

## Как предложить сценарий или операцию

Идеи новых задач приветствуются именно здесь: если в таблице покрытия ниже операция помечена как непокрытая, или вы видите задачу разработчика, которую поверхность должна решать одним проводом, а сценария нет, — это и есть предложение.

1. Откройте issue по форме [«Новая операция»](https://github.com/IngvarConsulting/unica/issues/new?template=new_operation.yml)
   (она сама ставит метку `area:surface`): задача словами разработчика 1С и, если можете, провод
   вызовов, которым её должно быть можно решить. Этого достаточно: сценарий допишем вместе.
2. Или добавьте сценарий сами: следующий свободный номер `S###`, область из списка ниже, задача
   одной фразой и провод шагов. Шаг — это `tool`, `args` и `expect`:

   ```json
   {
     "id": "S267",
     "area": "Реквизиты и состав",
     "task": "Сделать реквизит обязательным для заполнения",
     "wire": [
       {
         "tool": "unica.apply",
         "args": {
           "at": "main:Catalog.Валюты.TabularSection.Представления.Attribute.КодЯзыка",
           "ops": [{"op": "attribute.set", "args": {"values": {"required": true}}}],
           "dryRun": true
         },
         "expect": ["ok"]
       }
     ]
   }
   ```

3. Поднимите счётчики сценариев и шагов в тесте формы (`test_corpus_holds_the_run_free_scenario_set_uniquely_numbered`),
   прогоните `python -m unittest tests/ci/test_acceptance_scenarios.py` и заморозьте тот класс,
   которым поверхность действительно ответила. Умышленный отказ фиксируется с полем `refusal` —
   точным текстом `код: сообщение` ответа; шаг `check` с профилем — с полем `status` (`passed` или
   `failed`); пробел — с полем `gap` (причина); пробелов допускается не больше восьми, и каждый —
   заявка на починку поверхности, а не на переутверждение корпуса.
4. Обновите этот документ: `python scripts/ci/render-acceptance-registry.py --write`.

Адреса и аргументы в примере — реальные узлы фикстурного пространства; его состав описан в
`tests/fixtures/acceptance/README.md`.

## Покрытие реестра `apply`

Закрытый реестр держит **120 операций**; сценарии покрывают **112**, помечены реализованными **120**. Операция без сценария — первая кандидатура для новой задачи.

### Метаданные: объекты и коллекции

| Операция | Аргументы | Реализована | Сценарии |
| --- | --- | --- | --- |
| `object.borrow` | `From` | да | S313, S319 |
| `object.create` | `values` | да | S103, S104, S105, S106, S107, S108, S109, S110, S117, S119, S120, S121, S122, S123, S124, S314, S355 |
| `object.remove` | только `at` | да | S111, S112, S285, S293, S296, S314 |
| `relation.add` | `values` | да | S113, S115 |
| `relation.remove` | `values` | да | S115 |
| `relation.replace` | `values` | да | S114 |
| `help.create` | `values` | да | S116 |
| `attribute.add` | `items` | да | S059, S060, S061, S062, S063, S064, S065, S066, S067, S068, S072, S094, S095, S097, S099, S110, S259, S260 |
| `attribute.set` | `values` | да | S050, S051, S052, S053, S069, S102 |
| `attribute.remove` | только `at` | да | S070, S071 |
| `tabularSection.add` | `items` | да | S073, S098 |
| `tabularSection.set` | `values` | да | S056, S074 |
| `tabularSection.remove` | `values` | да | S075 |
| `dimension.add` | `items` | да | S076 |
| `dimension.set` | `values` | да | S077, S080 |
| `dimension.remove` | `values` | да | S078, S081 |
| `resource.add` | `items` | да | S079 |
| `resource.set` | `values` | да | **не покрыто — предложите задачу** |
| `resource.remove` | `values` | да | **не покрыто — предложите задачу** |
| `enumValue.add` | `items` | да | S082, S282 |
| `enumValue.set` | `values` | да | S083 |
| `enumValue.remove` | `values` | да | S084 |
| `column.add` | `items` | да | S085 |
| `column.set` | `values` | да | S086 |
| `column.remove` | `values` | да | S087 |
| `template.add` | `items` | да | S196, S199, S203, S277, S284, S330, S331, S361, S362, S363, S364, S365, S367, S368, S370, S371, S372, S373, S374, S375, S376, S377, S378, S379 |
| `template.set` | `values` | да | S197 |
| `template.remove` | `values` | да | S198 |
| `command.add` | `items` | да | **не покрыто — предложите задачу** |
| `command.set` | `values` | да | **не покрыто — предложите задачу** |
| `command.remove` | `values` | да | **не покрыто — предложите задачу** |
| `predefinedItem.add` | `items` | да | S088, S089, S090 |
| `predefinedItem.set` | `values` | да | S089 |
| `predefinedItem.remove` | `values` | да | S090 |

### Свойства

| Операция | Аргументы | Реализована | Сценарии |
| --- | --- | --- | --- |
| `props.set` | `values` | да | S037, S038, S039, S040, S041, S042, S043, S044, S045, S046, S047, S048, S049, S054, S055, S057, S058, S091, S096, S099, S226, S227, S256, S257, S258, S275, S276, S281, S287, S288, S310, S354, S355 |

### Формы

| Операция | Аргументы | Реализована | Сценарии |
| --- | --- | --- | --- |
| `form.add` | `items` | да | S125, S126, S136, S137, S138, S145, S273, S283, S312 |
| `form.set` | `values` | да | S127 |
| `form.remove` | только `at` | да | S128, S295 |
| `form.create` | `values` | да | S129 |
| `element.add` | `items` | да | S130, S139, S141, S142 |
| `element.remove` | только `at` | да | S131 |
| `formAttribute.add` | `items` | да | S132 |
| `formCommand.add` | `items` | да | S133, S135 |
| `event.bind` | `values` | да | S134, S140, S143, S325 |

### Роли и права

| Операция | Аргументы | Реализована | Сценарии |
| --- | --- | --- | --- |
| `role.create` | `values` | да | S170 |
| `right.set` | `values` | да | S168, S169, S171, S172, S173, S177, S279 |

### Схемы компоновки данных

| Операция | Аргументы | Реализована | Сценарии |
| --- | --- | --- | --- |
| `dataSource.add` | `items` | да | S361 |
| `dataSource.set` | `values` | да | S361 |
| `dataSource.remove` | `values` | да | S361, S366 |
| `dataSet.add` | `items` | да | S361, S367, S368, S370 |
| `dataSet.set` | `values` | да | S361 |
| `dataSet.remove` | только `at` | да | S361 |
| `field.add` | `items` | да | S181, S326, S361, S366, S368 |
| `field.set` | `values` | да | S182, S361 |
| `field.remove` | только `at` | да | S183, S368 |
| `fieldRole.set` | `values` | да | S361 |
| `parameter.add` | `items` | да | S184, S185, S362, S376 |
| `parameter.set` | `values` | да | S185, S362, S376 |
| `parameter.remove` | только `at` | да | S362 |
| `parameter.rename` | `values` | да | S362 |
| `parameter.reorder` | `items` | да | S362 |
| `calculatedField.add` | `items` | да | S190, S362, S371 |
| `calculatedField.remove` | `values` | да | S362 |
| `total.add` | `items` | да | S191, S362, S371, S379 |
| `total.remove` | `values` | да | S362 |
| `variant.add` | `items` | да | S192, S363 |
| `variant.set` | `values` | да | S363 |
| `variant.remove` | `values` | да | S363 |
| `query.set` | `values` | да | S188, S326, S327, S329, S361 |
| `query.patch` | `values` | да | S189, S327, S329, S361, S367 |
| `filter.add` | `items` | да | S186, S363, S364, S365 |
| `filter.set` | `values` | да | S363 |
| `filter.remove` | `values` | да | S363 |
| `filter.clear` | `values` | да | S187, S365, S369 |
| `selection.add` | `items` | да | S363, S364, S365, S368, S372, S374, S378 |
| `selection.clear` | `values` | да | S365, S374, S378 |
| `order.add` | `items` | да | S363, S364, S365, S372, S374 |
| `order.clear` | `values` | да | S365, S374 |
| `dataParameter.add` | `items` | да | S363, S372 |
| `dataParameter.set` | `values` | да | S363 |
| `outputParameter.set` | `values` | да | S363, S365, S372, S373, S377 |
| `conditionalAppearance.add` | `items` | да | S363, S365, S373, S377 |
| `conditionalAppearance.clear` | `values` | да | S365 |
| `structure.add` | `items` | да | S364, S375 |
| `structure.set` | `values` | да | S193, S364 |
| `structure.patch` | `values` | да | S326, S327, S364, S375 |
| `structure.remove` | `values` | да | S364 |
| `dataSetLink.add` | `items` | да | S361, S370 |
| `calculatedField.set` | `values` | да | S371 |
| `total.set` | `values` | да | S371, S379 |
| `selection.set` | `values` | да | S372, S374, S378 |
| `selection.remove` | `values` | да | S372, S374, S378 |
| `order.set` | `values` | да | S372, S374 |
| `order.remove` | `values` | да | S372, S374 |
| `dataParameter.remove` | `values` | да | S372 |
| `outputParameter.remove` | `values` | да | S372 |
| `conditionalAppearance.remove` | `values` | да | S373 |
| `dataSetLink.set` | `values` | да | S370 |
| `dataSetLink.remove` | `values` | да | S370 |

### Табличные документы

| Операция | Аргументы | Реализована | Сценарии |
| --- | --- | --- | --- |
| `mxl.set` | `values` | да | S199, S331, S332, S333, S334 |

### Пакеты XDTO

| Операция | Аргументы | Реализована | Сценарии |
| --- | --- | --- | --- |
| `valueType.add` | `values` | да | S214 |
| `objectType.add` | `values` | да | S215 |
| `property.add` | `values` | да | S216 |
| `type.remove` | только `at` | да | S217 |
| `property.remove` | `values` | да | S218 |

### Подсистемы

| Операция | Аргументы | Реализована | Сценарии |
| --- | --- | --- | --- |
| `subsystem.create` | `values` | да | S205 |
| `content.add` | `items` | да | S206, S207 |
| `content.remove` | только `at` | да | S207 |
| `childSubsystem.add` | `items` | да | S208 |
| `childSubsystem.remove` | только `at` | да | S209 |

### Поддержка поставщика

| Операция | Аргументы | Реализована | Сценарии |
| --- | --- | --- | --- |
| `supportCapability.set` | `values` | да | S246, S280, S288 |
| `supportRule.set` | `values` | да | S247, S274, S286, S287 |

### Interface

| Операция | Аргументы | Реализована | Сценарии |
| --- | --- | --- | --- |
| `commandVisibility.set` | `items` | да | **не покрыто — предложите задачу** |
| `commandPlacement.set` | `items` | да | **не покрыто — предложите задачу** |
| `commandOrder.set` | `values` | да | S320 |
| `groupOrder.set` | `values` | да | **не покрыто — предложите задачу** |
| `subsystemOrder.set` | `values` | да | S321 |

### Код BSL

| Операция | Аргументы | Реализована | Сценарии |
| --- | --- | --- | --- |
| `code.insert` | `text` | да | S156, S159, S278, S314, S319, S325, S336, S337, S338 |
| `code.replace` | `text` | да | S157, S316, S338, S357 |

### События

| Операция | Аргументы | Реализована | Сценарии |
| --- | --- | --- | --- |
| `event.implement` | только `at` | да | S158 |

## Покрытие профилей `check`

| Профиль | Сценарии |
| --- | --- |
| `cf` | S224, S267, S271, S301 |
| `cfe` | S290, S313, S314 |
| `form` | S225, S268, S302, S325 |
| `dcs` | S194, S326, S328, S329, S330, S361, S364 |
| `mxl` | S201, S331, S332, S335, S350 |
| `role` | S175, S179, S269, S291, S303 |
| `subsystem` | S210, S212, S270 |
| `interface` | S266 |
| `meta` | S035, S101, S222, S223, S228, S264, S347, S348, S349 |

## Инструменты в проводах

| Инструмент | Сценариев |
| --- | ---: |
| `unica.apply` | 213 |
| `unica.view` | 101 |
| `unica.check` | 62 |
| `unica.search` | 33 |
| `unica.docs` | 24 |
| `unica.diff` | 15 |
| `unica.resolve` | 3 |

## Каталог сценариев

Области: Навигация и чтение (48) · Свойства объектов (27) · Реквизиты и состав (46) · Создание и удаление объектов (22) · Формы (26) · Код BSL (37) · Роли и права (15) · СКД (40) · Макеты (14) · Подсистемы и интерфейс (12) · XDTO и обмены (9) · Сборка, ИБ и выгрузка (2) · Проверки качества (19) · Тесты и запуск (1) · Документация и стандарты (17) · Поддержка поставщика (16) · Расширения конфигурации (7) · Объекты метаданных (3) · Рабочее пространство (4) · Метаданные (2) · Приёмочный корпус (8) · Известные пробелы (4)

### Навигация и чтение

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S001 | Найти справочник валют по имени и открыть его свойства | `source` | `search`<br>`view` | `ok`<br>`ok` |
| S002 | Найти объект по синониму «Важность проблемы учета» | `source` | `search` | `ok` |
| S003 | Найти все справочники, отфильтровав кандидатов по виду | `source` | `search` | `ok` |
| S004 | Разрешить неточное имя «Валюта» в точный адрес (ближайшие кандидаты) | `source` | `search` | `ok` |
| S005 | Посмотреть состав конфигурации верхним уровнем | `source` | `view` | `ok` |
| S006 | Прочитать свойства документа акта об уничтожении персональных данных | `source` | `view` | `ok` |
| S007 | Перечислить реквизиты справочника Валюты | `source` | `view` | `ok` |
| S008 | Открыть один реквизит Наценка и посмотреть его свойства | `source` | `view` | `ok` |
| S009 | Посмотреть табличную часть Представления справочника Валюты | `source` | `view` | `ok` |
| S010 | Перечислить реквизиты табличной части Представления | `source` | `view` | `ok` |
| S011 | Посмотреть стандартные реквизиты справочника | `source` | `view` | `ok` |
| S012 | Перечислить измерения регистра сведений Административная иерархия | `source` | `view` | `ok` |
| S013 | Перечислить ресурсы регистра сведений | `source` | `view` | `ok` |
| S014 | Перечислить значения перечисления Важность проблемы учета | `source` | `view` | `ok` |
| S015 | Посмотреть модули справочника Валюты | `source` | `view` | `ok` |
| S016 | Посмотреть табличные части документа | `source` | `view` | `ok` |
| S017 | Перечислить макеты отчёта Анализ версий объектов | `source` | `view` | `ok` |
| S018 | Прочитать только свойства справочника без веток | `source` | `view` | `ok` |
| S019 | Прочитать только ветки состава справочника | `source` | `view` | `ok` |
| S020 | Пролистать реквизиты справочника страницей в три элемента | `source` | `view` | `ok` |
| S021 | Найти документ по подстроке имени «Уничтожении» | `source` | `search` | `ok` |
| S022 | Найти регистры сведений по виду | `source` | `search` | `ok` |
| S023 | Найти общий модуль Google-переводчика | `source` | `search` | `ok` |
| S024 | Найти роль продавца-читателя | `source` | `search` | `ok` |
| S025 | Найти подсистему администрирования | `source` | `search` | `ok` |
| S026 | Узнать, какие операции доступны на корне конфигурации | `source` | `view` | `ok` |
| S027 | Узнать словарь операций справочника перед правкой | `source` | `view` | `ok` |
| S028 | Узнать словарь операций перечисления | `source` | `view` | `ok` |
| S029 | Узнать словарь операций регистра сведений | `source` | `view` | `ok` |
| S030 | Убедиться, что объект не изменился после рефакторинга (diff с самим собой) | `source` | `diff` | `ok` |
| S031 | Сравнить два перечисления одного вида между собой | `source` | `diff` | `ok` |
| S032 | Сравнить только свойства двух перечислений | `source` | `diff` | `ok` |
| S033 | Сравнить синонимы двух перечислений точечно по указателю | `source` | `diff` | `ok` |
| S034 | Сравнить два реквизита одного справочника | `source` | `diff` | `ok` |
| S035 | Проверить читаемость узла справочника перед работой | `source` | `check` валидаторы `meta` | `ok` |
| S036 | Узнать вердикт по рабочему пространству: готовность, проверки, диагностики | `source` | `check`<br>`check` (как форма клиента) | `ok`<br>`ok` |
| S250 | Посмотреть языки конфигурации | `source` | `view` | `ok` |
| S251 | Открыть язык Русский и его свойства | `source` | `view` | `ok` |
| S252 | Посмотреть одно значение перечисления | `source` | `view` | `ok` |
| S253 | Найти табличную часть по имени | `source` | `search` | `ok` |
| S254 | Пролистать состав конфигурации небольшой страницей | `source` | `view` | `ok` |
| S255 | Открыть измерение регистра и его свойства | `source` | `view` | `ok` |
| S299 | Продолжить страницу чужим курсором — отказ вместо чужого ответа | `source` | `view` | `gap` |
| S300 | Ограничить страницу подсказок поиска | `source` | `search` | `ok` |
| S301 | Проверить корень старой выгрузки 2.19 — совет перевыгрузить платформой приходит из check | `source` | `check` валидаторы `cf` | `ok` |
| S305 | Прочитать узел старой выгрузки — чтение открыто, совет о формате остаётся за check | `source` | `view` | `ok` |
| S307 | Перевести путь из чужого диффа в логический адрес | `source` | `resolve` | `ok` |
| S308 | Узнать файл и строки метода, чтобы открыть его вне Unica | `source` | `resolve` | `ok` |

### Свойства объектов

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S037 | Изменить комментарий справочника Валюты (превью без записи) | `source` | `apply` `props.set` | `ok` |
| S038 | Изменить комментарий справочника и записать по-настоящему | `source` | `apply` `props.set`<br>`apply` | `ok`<br>`ok` |
| S039 | Изменить синоним перечисления (превью) | `source` | `apply` `props.set` | `ok` |
| S040 | Изменить комментарий регистра сведений (превью) | `source` | `apply` `props.set` | `ok` |
| S041 | Изменить синоним документа (превью) | `source` | `apply` `props.set` | `ok` |
| S042 | Изменить комментарий общего модуля (превью) | `source` | `apply` `props.set` | `ok` |
| S043 | Изменить синоним отчёта (превью) | `source` | `apply` `props.set` | `ok` |
| S044 | Увеличить длину наименования справочника до 200 (превью) | `source` | `apply` `props.set` | `ok` |
| S045 | Изменить длину кода справочника (превью) | `source` | `apply` `props.set` | `ok` |
| S046 | Поменять длину наименования документа (превью) | `source` | `apply` `props.set` | `ok` |
| S047 | Прочитать объект и записать комментарий по сохранённому плану | `source` | `view`<br>`apply` `props.set`<br>`apply`<br>`view` | `ok`<br>`ok`<br>`ok`<br>`ok` |
| S048 | Изменить сразу два свойства одним вызовом (превью) | `source` | `apply` `props.set` | `ok` |
| S049 | Изменить свойства двух объектов последовательными вызовами | `source` | `apply` `props.set`<br>`apply` `props.set` | `ok`<br>`ok` |
| S050 | Изменить синоним реквизита документа (превью) | `source` | `apply` `attribute.set` | `ok` |
| S051 | Изменить тип реквизита на строку подлиннее (превью) | `source` | `apply` `attribute.set` | `ok` |
| S052 | Сделать реквизит обязательным к заполнению (превью) | `source` | `apply` `attribute.set` | `ok` |
| S053 | Включить индексирование реквизита документа (превью) | `source` | `apply` `attribute.set` | `ok` |
| S054 | Сначала посмотреть словарь операций, затем поменять свойство (полный маршрут) | `source` | `view`<br>`apply` `props.set` | `ok`<br>`ok` |
| S055 | Прочитать свойство перед правкой и сравнить после превью (диффом) | `source` | `view`<br>`apply` `props.set`<br>`diff` | `ok`<br>`ok`<br>`ok` |
| S056 | Изменить синоним табличной части (текущий ответ поверхности) | `source` | `apply` `tabularSection.set` | `ok` |
| S057 | Изменить комментарий роли (превью правки свойств роли) | `source` | `apply` `props.set` | `ok` |
| S058 | Изменить синоним подсистемы (превью) | `source` | `apply` `props.set` | `ok` |
| S256 | Записать комментарий отчёта по-настоящему | `source` | `apply` `props.set`<br>`apply` | `ok`<br>`ok` |
| S257 | Записать синоним общего модуля по-настоящему | `source` | `apply` `props.set`<br>`apply` | `ok`<br>`ok` |
| S258 | Записать комментарий документа по-настоящему | `source` | `apply` `props.set`<br>`apply` | `ok`<br>`ok` |
| S275 | Изменить комментарий справочника в выгрузке 2.21 — отказ по формату новее профиля | `source` | `apply` `props.set` | `refused` |
| S276 | Изменить комментарий справочника в выгрузке без версии корня — формат 1.0 старше профиля, отказ до записи | `source` | `apply` `props.set` | `refused` |

### Реквизиты и состав

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S059 | Добавить строковый реквизит в документ (превью) | `source` | `apply` `attribute.add` | `ok` |
| S060 | Добавить строковый реквизит и сразу записать | `source` | `apply` `attribute.add`<br>`apply` | `ok`<br>`ok` |
| S061 | Добавить числовой реквизит с точностью (превью) | `source` | `apply` `attribute.add` | `ok` |
| S062 | Добавить булев реквизит (превью) | `source` | `apply` `attribute.add` | `ok` |
| S063 | Добавить реквизит-дату (превью) | `source` | `apply` `attribute.add` | `ok` |
| S064 | Добавить ссылочный реквизит на справочник валют (превью) | `source` | `apply` `attribute.add` | `ok` |
| S065 | Добавить реквизит с синонимом и позицией после существующего (превью) | `source` | `apply` `attribute.add` | `ok` |
| S066 | Добавить сразу два реквизита одним вызовом (превью) | `source` | `apply` `attribute.add` | `ok` |
| S067 | Добавить реквизит документу (превью) | `source` | `apply` `attribute.add` | `ok` |
| S068 | Добавить обязательный реквизит с комментарием (превью) | `source` | `apply` `attribute.add` | `ok` |
| S069 | Поменять синоним и тип реквизита одним вызовом (превью) | `source` | `apply` `attribute.set` | `ok` |
| S070 | Удалить реквизит документа (превью) | `source` | `apply` `attribute.remove` | `ok` |
| S071 | Удалить реквизит, сверившись со словарём операций | `source` | `view`<br>`apply` `attribute.remove` | `ok`<br>`ok` |
| S072 | Добавить реквизит в существующую табличную часть документа через scope (превью) | `source` | `apply` `attribute.add` | `ok` |
| S073 | Создать пустую табличную часть документа (превью) | `source` | `apply` `tabularSection.add` | `ok` |
| S074 | Изменить свойства табличной части (текущий ответ поверхности) | `source` | `apply` `tabularSection.set` | `ok` |
| S075 | Удалить табличную часть (текущий ответ поверхности) | `source` | `apply` `tabularSection.remove` | `ok` |
| S076 | Добавить измерение в регистр сведений (текущий ответ поверхности) | `source` | `apply` `dimension.add` | `ok` |
| S077 | Изменить измерение регистра (текущий ответ поверхности) | `source` | `apply` `dimension.set` | `ok` |
| S078 | Удалить измерение регистра (текущий ответ поверхности) | `source` | `apply` `dimension.remove` | `ok` |
| S079 | Добавить ресурс в регистр сведений (текущий ответ поверхности) | `source` | `apply` `resource.add` | `ok` |
| S080 | Изменить измерение регистра (в фикстуре нет ресурсов — правим измерение) | `source` | `apply` `dimension.set` | `ok` |
| S081 | Удалить измерение регистра (в фикстуре нет ресурсов — удаляем измерение) | `source` | `apply` `dimension.remove` | `ok` |
| S082 | Добавить значение перечисления (текущий ответ поверхности) | `source` | `apply` `enumValue.add` | `ok` |
| S083 | Изменить синоним значения перечисления (текущий ответ поверхности) | `source` | `apply` `enumValue.set` | `ok` |
| S084 | Удалить значение перечисления (текущий ответ поверхности) | `source` | `apply` `enumValue.remove` | `ok` |
| S085 | Добавить колонку в журнал документов | `source` | `apply` `column.add` | `ok` |
| S086 | Изменить колонку журнала | `source` | `apply` `column.set` | `ok` |
| S087 | Удалить колонку журнала | `source` | `apply` `column.remove` | `ok` |
| S088 | Добавить предопределённый элемент справочника (текущий ответ поверхности) | `source` | `apply` `predefinedItem.add` | `ok` |
| S089 | Добавить предопределённый элемент и уточнить его описание одним пакетом (превью) | `source` | `apply` `predefinedItem.add` `predefinedItem.set` | `ok` |
| S090 | Добавить и удалить предопределённый элемент одним пакетом (обратимость превью) | `source` | `apply` `predefinedItem.add` `predefinedItem.remove` | `ok` |
| S091 | Ошибочный ключ аргументов ведёт к подсказке скелета (само-восстановление агента) | `source` | `apply` `props.set`<br>`apply` `props.set` | `refused`<br>`ok` |
| S092 | Узнать словарь операций табличной части перед правкой | `source` | `view` | `ok` |
| S093 | Узнать словарь операций реквизита | `source` | `view` | `ok` |
| S094 | Добавить реквизит перечислению нельзя — применимость видна в словаре | `source` | `view`<br>`apply` `attribute.add` | `ok`<br>`refused` |
| S095 | Полный маршрут: найти объект, изучить состав, превью добавления, перечитать состав | `source` | `search`<br>`view`<br>`apply` `attribute.add`<br>`view` | `ok`<br>`ok`<br>`ok`<br>`ok` |
| S096 | Изменить свойство и вернуть обратно (обратимость реальной записи) | `source` | `apply` `props.set`<br>`apply`<br>`apply` `props.set`<br>`apply`<br>`diff` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S097 | Превью повторяем: два одинаковых превью дают один план | `source` | `apply` `attribute.add`<br>`apply` `attribute.add` | `ok`<br>`ok` |
| S098 | Создать пустую табличную часть документа (превью) | `source` | `apply` `tabularSection.add` | `ok` |
| S099 | Пакет из добавления и правки свойств за один вызов (превью) | `source` | `apply` `attribute.add` `props.set` | `ok` |
| S100 | Добавить стандартный реквизит нельзя — словарь этого не предлагает | `source` | `view` | `ok` |
| S101 | Проверить состав после серии правок | `source` | `check` валидаторы `meta` | `ok` |
| S102 | Сравнить реквизит до и после превью (превью не меняет байты) | `source` | `apply` `attribute.set`<br>`diff` | `ok`<br>`ok` |
| S259 | Добавить реквизит регистру сведений (превью) | `source` | `apply` `attribute.add` | `ok` |
| S260 | Реальный объект БСП не проходит постусловие правки — граница ридера постобраза | `source` | `apply` `attribute.add` | `ok` |

### Создание и удаление объектов

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S103 | Создать новый справочник Номенклатура | `source` | `apply` `object.create` | `ok` |
| S104 | Создать документ Заказ клиента | `source` | `apply` `object.create` | `ok` |
| S105 | Создать перечисление Статусы заказа | `source` | `apply` `object.create` | `ok` |
| S106 | Создать регистр сведений Цены номенклатуры | `source` | `apply` `object.create` | `ok` |
| S107 | Создать общий модуль РаботаСЗаказами | `source` | `apply` `object.create` | `ok` |
| S108 | Создать отчёт Продажи за период | `source` | `apply` `object.create` | `ok` |
| S109 | Создать обработку Загрузка прайса | `source` | `apply` `object.create` | `ok` |
| S110 | Создать справочник Контрагенты по-настоящему, добавить ему реквизит ИНН и прочитать результат | `source` | `apply` `object.create`<br>`apply`<br>`apply` `attribute.add`<br>`apply`<br>`view` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S111 | Удалить устаревший объект конфигурации | `source` | `apply` `object.remove` | `ok` |
| S112 | Удалить справочник, на который ссылаются права роли — поверхность отказывает и называет ссылающийся файл | `source` | `apply` `object.remove` | `refused` |
| S113 | Добавить документ в основания создания справочника (превью) | `source` | `apply` `relation.add` | `ok` |
| S114 | Заменить состав оснований создания справочника (превью) | `source` | `apply` `relation.replace` | `ok` |
| S115 | Добавить и убрать основание создания одним пакетом (обратимость связи, превью) | `source` | `apply` `relation.add` `relation.remove` | `ok` |
| S116 | Завести встроенную справку объекта на русском языке | `source` | `apply` `help.create` | `ok` |
| S117 | Скопировать объект: прочитать состав и создать аналог (маршрут с текущим ответом) | `source` | `view`<br>`apply` `object.create` | `ok`<br>`ok` |
| S118 | Узнать из словаря корня, доступно ли создание объектов | `source` | `view` | `ok` |
| S119 | Создать вид характеристик | `source` | `apply` `object.create` | `ok` |
| S120 | Создать план обмена | `source` | `apply` `object.create` | `ok` |
| S121 | Создать константу | `source` | `apply` `object.create` | `ok` |
| S122 | Создать регистр накопления | `source` | `apply` `object.create` | `ok` |
| S123 | Создать журнал документов | `source` | `apply` `object.create` | `ok` |
| S124 | Создать определяемый тип | `source` | `apply` `object.create` | `ok` |

### Формы

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S125 | Добавить форму списка справочнику | `source` | `apply` `form.add` | `ok` |
| S126 | Добавить форму элемента | `source` | `apply` `form.add` | `ok` |
| S127 | Назначить основную форму списка | `source` | `apply` `form.set` | `ok` |
| S128 | Удалить устаревшую форму | `source` | `apply` `form.remove` | `ok` |
| S129 | Создать произвольную форму с нуля | `source` | `apply` `form.create` | `ok` |
| S130 | Добавить на форму элемента поле ввода реквизита | `source` | `apply` `element.add` | `ok` |
| S131 | Удалить служебную группу с формы элемента | `source` | `apply` `element.remove` | `ok` |
| S132 | Добавить реквизит формы | `source` | `apply` `formAttribute.add` | `ok` |
| S133 | Добавить команду формы с кнопкой | `source` | `apply` `formCommand.add` | `ok` |
| S134 | Назначить обработчик события ПриОткрытии | `source` | `apply` `event.bind` | `ok` |
| S135 | Полный маршрут доработки формы: найти форму, изучить, добавить команду | `source` | `view`<br>`apply` `formCommand.add` | `ok`<br>`ok` |
| S136 | Добавить форму документу | `source` | `apply` `form.add` | `ok` |
| S137 | Добавить форму отчёту | `source` | `apply` `form.add` | `ok` |
| S138 | Добавить форму перечислению из словаря операций | `source` | `view`<br>`apply` `form.add` | `ok`<br>`ok` |
| S139 | Скрыть элемент формы по функциональной опции | `source` | `apply` `element.add` | `ok` |
| S140 | Настроить условное оформление на форме | `source` | `apply` `event.bind` | `ok` |
| S141 | Добавить табличное поле списка на форму | `source` | `apply` `element.add` | `ok` |
| S142 | Переставить элементы формы | `source` | `apply` `element.add` | `ok` |
| S143 | Привязать обработчик изменения к полю формы | `source` | `apply` `event.bind` | `ok` |
| S144 | Посмотреть формы объекта перед доработкой (в фикстуре их нет — пустой состав) | `source` | `view` | `ok` |
| S145 | Добавить форму настроек регистру | `source` | `apply` `form.add` | `ok` |
| S146 | Словарь операций узла языка | `source` | `view` | `ok` |
| S273 | Добавить форму справочнику в выгрузке 2.21 — запись вне записываемого профиля отказывает до первого байта | `source` | `apply` `form.add` | `refused` |
| S302 | Проверить форму в старой выгрузке — тот же совет на узле формы | `source` | `check` валидаторы `form` | `ok` |
| S312 | Повторно добавить существующую форму элемента — отказ без дубля регистрации | `source` | `apply` `form.add` | `refused` |
| S325 | Показать известный дефект #791: проверка формы принимает трёхпараметровый обработчик выбора вместо пяти параметров | `source` | `apply` `event.bind`<br>`apply`<br>`apply` `code.insert`<br>`apply`<br>`check` валидаторы `form` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`gap`: известный ошибочный `ok` (#791) |

### Код BSL

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S147 | Найти, где определена функция ПеревестиТекст | `source` | `search` | `ok` |
| S148 | Найти все вызовы функции по конфигурации | `source` | `search` | `ok` |
| S149 | Найти использования строкового литерала (адрес сервиса) | `source` | `search` | `ok` |
| S150 | Поискать только в модуле переводчика | `source` | `search` | `ok` |
| S151 | Поискать в объекте без кода — пустой результат, не ошибка | `source` | `search` | `ok` |
| S152 | Ограничить поиск корнем конфигурации | `source` | `search` | `ok` |
| S153 | Поиск с ограничением количества результатов | `source` | `search` | `ok` |
| S154 | Поиск регулярным выражением | `source` | `search` | `ok` |
| S155 | Посмотреть методы общего модуля | `source` | `view` | `ok` |
| S156 | Вставить служебный код после метода модуля | `source` | `apply` `code.insert` | `ok` |
| S157 | Заменить реализацию функции модуля | `source` | `apply` `code.replace` | `ok` |
| S158 | Реализовать обработчик события объекта в модуле объекта | `source` | `apply` `event.implement` | `ok` |
| S159 | Полный маршрут правки кода: найти вызов, изучить модуль, вставить код | `source` | `search`<br>`view`<br>`apply` `code.insert` | `ok`<br>`ok`<br>`ok` |
| S160 | Найти мёртвый код: поиск по имени функции, которую никто не зовёт | `source` | `search` | `ok` |
| S161 | Найти обработчики определения настроек | `source` | `search` | `ok` |
| S162 | Поиск кириллического идентификатора со спецсимволами | `source` | `search` | `ok` |
| S163 | Изучить словарь операций модуля | `source` | `view` | `ok` |
| S164 | Найти все экспортные функции в коде конфигурации | `source` | `search` | `ok` |
| S165 | Открыть конкретный метод модуля | `source` | `view` | `ok` |
| S166 | Найти вызовы функции перевода в коде | `source` | `search` | `ok` |
| S261 | Найти все внешние адреса, зашитые в код | `source` | `search` | `ok` |
| S278 | Вставить код в общий модуль выгрузки 2.21 — отказ по формату владельца | `source` | `apply` `code.insert` | `refused` |
| S316 | После правки функции проверить анализатором только изменённый модуль | `source` | `apply` `code.replace`<br>`apply`<br>`check` валидаторы `bsl` | `ok`<br>`ok`<br>`ok` / `provider` |
| S317 | Найти определение функции по смыслу | `source` | `search` | `provider` |
| S318 | Найти определение функции по символу | `source` | `search` | `provider` |
| S324 | После найденного метода отсутствующий символ дважды даёт завершённую пустую выдачу | `delivery/bsl-analyzer` | `search`<br>`search`<br>`search` | `ok`<br>`ok`<br>`ok` |
| S336 | Module и Body: append в общем, объектном и собственном модуле расширения; точные BOM/CRLF, соседний код, повтор без записи и неизменный descriptor | `source` | `apply` `code.insert`<br>`apply`<br>`apply` `code.insert`<br>`apply`<br>`apply` `code.insert`<br>`apply`<br>`apply` `code.insert`<br>`apply`<br>`apply` `code.insert`<br>`apply`<br>`apply` `code.insert`<br>`apply`<br>`apply` `code.insert`<br>`apply`<br>`apply` `code.insert`<br>`apply`<br>`apply` `code.insert`<br>`apply`<br>`apply` `code.insert`<br>`apply`<br>`apply` `code.insert`<br>`apply`<br>`apply` `code.insert`<br>`apply` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S337 | Anchor before/after учитывает только методы; method before/after сохраняет аннотацию и соседей; точный post-image и повтор без записи | `source` | `apply` `code.insert`<br>`apply`<br>`apply` `code.insert`<br>`apply`<br>`apply` `code.insert`<br>`apply`<br>`apply` `code.insert`<br>`apply`<br>`apply` `code.insert`<br>`apply`<br>`apply` `code.insert`<br>`apply`<br>`apply` `code.insert`<br>`apply`<br>`apply` `code.insert`<br>`apply` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S338 | Method/Method.Body insert, Body+selector и неполные position/selector отказывают без записи; замена Method.Body сохраняет подпись/BOM/CRLF/соседей и повтор | `source` | `apply` `code.insert`<br>`apply` `code.insert`<br>`apply` `code.insert`<br>`apply` `code.insert`<br>`apply` `code.insert`<br>`apply` `code.replace`<br>`apply`<br>`apply` `code.replace`<br>`apply` | `refused`<br>`refused`<br>`refused`<br>`refused`<br>`refused`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S339 | Полный анализ без фильтра сохраняет предупреждение | `delivery/bsl-analyzer-diagnostics` | `check` валидаторы `bsl` | `ok` |
| S340 | Действующий TOML фильтр авторов не объявляет скрытые находки чистым кодом | `delivery/bsl-analyzer-diagnostics` | `check` | `provider` |
| S341 | JSON фильтр авторов также называет неполноту CLI | `delivery/bsl-analyzer-diagnostics` | `check` | `provider` |
| S342 | Модульный diff-filter перекрывает diff_base без ложной неполноты | `delivery/bsl-analyzer-diagnostics` | `check` валидаторы `bsl` | `ok` |
| S343 | Пустой TOML перекрывает JSON author filter согласно pinned precedence | `delivery/bsl-analyzer-diagnostics` | `check` валидаторы `bsl` | `ok` |
| S344 | Пустой поток называет отсутствие start и действие восстановления без номера строки | `fault-injection/analyzer-jsonl-fault` | `check` | `provider` |
| S345 | Повреждённый JSONL называет безопасную строку и причину без исходного payload | `fault-injection/analyzer-jsonl-fault` | `check` | `provider` |
| S346 | Неизвестная severity даёт безопасный отказ без секретных значений | `fault-injection/analyzer-jsonl-fault` | `check` | `provider` |

### Роли и права

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S167 | Посмотреть, что за роль SalesReader | `source` | `search`<br>`view` | `ok`<br>`ok` |
| S168 | Дать роли право чтения справочника Валюты | `source` | `apply` `right.set` | `ok` |
| S169 | Забрать право удаления интерактивно | `source` | `apply` `right.set` | `ok` |
| S170 | Создать новую роль МенеджерПродаж | `source` | `apply` `role.create` | `ok` |
| S171 | Настроить RLS на чтение по подразделению | `source` | `apply` `right.set` | `ok` |
| S172 | Дать право проведения документа | `source` | `apply` `right.set` | `ok` |
| S173 | Право на запуск обработки для роли | `source` | `apply` `right.set` | `ok` |
| S174 | Узнать по документации, как раздаются права на регистры | `source` | `docs` | `ok` / `provider` / `task` |
| S175 | Проверить роль | `source` | `check` валидаторы `role` | `ok` |
| S176 | Словарь операций роли | `source` | `view` | `ok` |
| S177 | Полный маршрут: найти роль, изучить, выдать право | `source` | `search`<br>`apply` `right.set` | `ok`<br>`ok` |
| S178 | Сравнить права двух ролей (диффом одинаковых видов) | `source` | `diff` | `ok` |
| S179 | Проверить читаемость узла роли | `source` | `check` валидаторы `role` | `ok` |
| S279 | Выдать право в роли выгрузки 2.21 — отказ по формату до правки Rights.xml | `source` | `apply` `right.set` | `refused` |
| S303 | Проверить роль в старой выгрузке — совет доходит и до прав | `source` | `check` валидаторы `role` | `ok` |

### СКД

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S180 | Открыть схему компоновки отчёта | `source` | `view` | `ok` |
| S181 | Добавить поле в набор данных СКД | `source` | `apply` `field.add` | `ok` |
| S182 | Изменить заголовок поля СКД | `source` | `apply` `field.set` | `ok` |
| S183 | Удалить поле из схемы | `source` | `apply` `field.remove` | `ok` |
| S184 | Добавить параметр периода | `source` | `apply` `parameter.add` | `ok` |
| S185 | Задать значение параметра по умолчанию | `source` | `apply` `parameter.add` `parameter.set` | `ok` |
| S186 | Добавить отбор по автору | `source` | `apply` `filter.add` | `ok` |
| S187 | Очистить отборы схемы | `source` | `apply` `filter.clear` | `ok` |
| S188 | Поменять текст запроса набора данных | `source` | `apply` `query.set` | `ok` |
| S189 | Точечно поправить запрос | `source` | `apply` `query.patch` | `ok` |
| S190 | Добавить вычисляемое поле | `source` | `apply` `calculatedField.add` | `ok` |
| S191 | Добавить итог по количеству версий | `source` | `apply` `total.add` | `ok` |
| S192 | Добавить вариант отчёта | `source` | `apply` `variant.add` | `ok` |
| S193 | Настроить структуру группировок варианта | `source` | `apply` `structure.set` | `ok` |
| S194 | Проверить схему компоновки данных | `source` | `check` валидаторы `dcs` | `ok` |
| S195 | Изучить словарь операций схемы | `source` | `view` | `ok` |
| S326 | По контракту подсказчика изменить поле, запрос и именованную группу из исходного #1299; preview не пишет, три XML-эффекта и check подтверждены | `source` | `view`<br>`view`<br>`view`<br>`view`<br>`view`<br>`apply` `field.add`<br>`apply`<br>`view`<br>`apply` `query.set`<br>`apply`<br>`view`<br>`apply` `structure.patch`<br>`apply`<br>`view`<br>`view`<br>`view`<br>`check` валидаторы `dcs` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S327 | Ошибочные цели, типы, can-фильтры, имена групп и число замен дают точный отказ без изменения XML, включая частично подходящую структуру | `source` | `view`<br>`view`<br>`view`<br>`view`<br>`view`<br>`view`<br>`view`<br>`diff`<br>`apply` `query.set`<br>`apply` `query.set`<br>`apply` `query.patch`<br>`apply` `query.patch`<br>`apply` `structure.patch`<br>`apply` `structure.patch` `structure.patch`<br>`apply` `structure.patch`<br>`apply` `query.patch`<br>`apply` `query.patch` | `ok`<br>`refused`<br>`refused`<br>`refused`<br>`refused`<br>`ok`<br>`refused`<br>`unsupported`<br>`refused`<br>`refused`<br>`refused`<br>`refused`<br>`refused`<br>`refused`<br>`refused`<br>`refused`<br>`refused` |
| S328 | Реальный Codex без навыков/обходов самостоятельно находит контракт, публикует три изменения, проверяет XML и честно описывает границу создания схем; полный transcript оценивает независимый агент | `agent-evaluation/codex` | `view`<br>`view`<br>`view`<br>`check` валидаторы `dcs` | `ok`<br>`ok`<br>`ok`<br>`ok` |
| S329 | Текст запроса одинаков при raw quotes/entities; once сохраняет пробелы и пустую замену; preview читается как исходная версия, устаревший токен не публикуется; XML comments/PI сохраняются побайтно | `source` | `view`<br>`apply` `query.set`<br>`view`<br>`apply` `query.patch`<br>`apply`<br>`apply`<br>`apply` `query.patch`<br>`apply`<br>`view`<br>`check` валидаторы `dcs` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`refused`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S330 | Новый DCS template — каркас с Основной, без DataSet; коллекция и XML честно показывают отсутствие набора данных | `source` | `view`<br>`apply` `template.add`<br>`apply`<br>`view`<br>`view`<br>`view`<br>`check` валидаторы `dcs` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S361 | Создать СКД примитивами: источник, Query/Object/Union и вложенный набор; проверить чтение, XML, отсутствие скрытой выборки и итоговый check | `source` | `apply` `template.add`<br>`apply`<br>`view`<br>`apply` `dataSource.add` `dataSource.set` `dataSet.add`<br>`apply`<br>`view`<br>`apply` `dataSet.add`<br>`apply`<br>`view`<br>`apply` `field.add` `field.add` `field.add` `field.add` `dataSet.set` `dataSet.set`<br>`apply`<br>`view`<br>`apply` `field.set` `fieldRole.set` `query.set` `query.patch` `dataSetLink.add`<br>`apply`<br>`view`<br>`view`<br>`check` валидаторы `dcs`<br>`view`<br>`apply` `dataSet.remove` `dataSource.remove`<br>`apply` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S362 | Параметры, вычисления и итоги имеют отдельные операции; rename сохраняет ссылки, reorder сохраняет чужие элементы, удаления проверены после положительных эффектов | `source` | `apply` `template.add`<br>`apply`<br>`view`<br>`apply` `parameter.add` `calculatedField.add` `total.add`<br>`apply`<br>`view`<br>`apply` `parameter.set` `parameter.rename` `parameter.reorder`<br>`apply`<br>`view`<br>`apply` `parameter.remove` `calculatedField.remove` `total.remove`<br>`apply` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S363 | Создать отдельный вариант: явная выборка, сортировка, отбор, параметры данных/вывода и оформление; set меняет только выбранное свойство и сохраняет соседние настройки | `source` | `apply` `template.add`<br>`apply`<br>`view`<br>`apply` `variant.add`<br>`apply`<br>`view`<br>`apply` `selection.add` `order.add` `filter.add` `dataParameter.add` `outputParameter.set` `conditionalAppearance.add`<br>`apply`<br>`view`<br>`apply` `variant.set` `filter.set` `dataParameter.set` `outputParameter.set`<br>`apply`<br>`view`<br>`apply` `filter.remove`<br>`apply`<br>`view`<br>`apply` `variant.remove`<br>`apply` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S364 | Составить table/chart с осями и именованными вложенными группами; локальная настройка и patch сохраняют остальные элементы; заменить и удалить структуру явно | `source` | `apply` `template.add`<br>`apply`<br>`view`<br>`apply` `structure.add` `structure.add`<br>`apply`<br>`view`<br>`apply` `structure.patch` `selection.add` `filter.add` `order.add`<br>`apply`<br>`view`<br>`view`<br>`apply` `structure.remove`<br>`apply`<br>`view`<br>`apply` `structure.set`<br>`apply`<br>`check` валидаторы `dcs` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S365 | Положительные эффекты четырёх настроек наблюдаются перед clear; очистка одной коллекции сохраняет остальные и параметры вывода | `source` | `apply` `template.add`<br>`apply`<br>`view`<br>`apply` `selection.add` `filter.add` `order.add` `conditionalAppearance.add` `outputParameter.set`<br>`apply`<br>`view`<br>`apply` `selection.clear`<br>`apply`<br>`view`<br>`apply` `filter.clear` `order.clear` `conditionalAppearance.clear`<br>`apply` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S366 | Поздняя ошибка удаления используемого источника отменяет ранее подготовленное поле; XML остаётся побайтно прежним, token не выдаётся | `source` | `view`<br>`apply` `field.add` `dataSource.remove` | `ok`<br>`refused` |
| S367 | Типизированный query.patch принимает прежние управляющие последовательности как буквальный текст и сохраняет остальные запросы | `source` | `apply` `template.add`<br>`apply`<br>`view`<br>`apply` `dataSet.add`<br>`apply`<br>`view`<br>`apply` `query.patch`<br>`apply`<br>`view` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S368 | Добавление и удаление поля не изменяет выборку; явная selection.add сохраняется после field.remove, соседние поля неизменны | `source` | `apply` `template.add`<br>`apply`<br>`view`<br>`apply` `dataSet.add`<br>`apply`<br>`view`<br>`apply` `field.add`<br>`apply`<br>`view`<br>`apply` `selection.add`<br>`apply`<br>`view`<br>`apply` `field.remove`<br>`apply` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S369 | Повтор пустой очистки не нормализует декларацию, соседние байты или переводы строк; итоговый SHA256 остаётся прежним | `source` | `view`<br>`apply` `filter.clear`<br>`apply` | `ok`<br>`ok`<br>`ok` |
| S370 | Изменить связь по четырём явным селекторам; проверить условие/параметр, затем удалить выбранную связь сохранив соседнюю | `source` | `apply` `template.add`<br>`apply`<br>`view`<br>`apply` `dataSet.add` `dataSetLink.add`<br>`apply`<br>`view`<br>`apply` `dataSetLink.set`<br>`apply`<br>`view`<br>`view`<br>`apply` `dataSetLink.remove`<br>`apply` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S371 | calculatedField.set и total.set сохраняют непереданные заголовок и группировку, меняют только явно переданное выражение | `source` | `apply` `template.add`<br>`apply`<br>`view`<br>`apply` `calculatedField.add` `total.add`<br>`apply`<br>`view`<br>`apply` `calculatedField.set` `total.set`<br>`apply` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S372 | Патч и удаление выбранного элемента не очищают соседние элементы выборки/сортировки, параметры данных и вывода удаляются отдельно | `source` | `apply` `template.add`<br>`apply`<br>`view`<br>`apply` `selection.add` `order.add` `dataParameter.add` `outputParameter.set` `outputParameter.set`<br>`apply`<br>`view`<br>`apply` `selection.set` `order.set`<br>`apply`<br>`view`<br>`view`<br>`apply` `selection.remove` `order.remove` `dataParameter.remove` `outputParameter.remove`<br>`apply` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S373 | conditionalAppearance.remove использует прочитанный индекс и удаляет один элемент; второй элемент и параметры вывода сохраняются | `source` | `apply` `template.add`<br>`apply`<br>`view`<br>`apply` `conditionalAppearance.add` `outputParameter.set`<br>`apply`<br>`view`<br>`view`<br>`apply` `conditionalAppearance.remove`<br>`apply` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S374 | Явно вернуть Auto после clear; set/remove выбирают прочитанный индекс Auto, сохраняют соседние Field элементы | `source` | `apply` `template.add`<br>`apply`<br>`view`<br>`apply` `selection.clear` `order.clear` `selection.add` `order.add`<br>`apply`<br>`view`<br>`view`<br>`apply` `selection.set` `order.set`<br>`apply`<br>`view`<br>`apply` `selection.remove` `order.remove`<br>`apply` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S375 | Отключение и пользовательские свойства Table/Chart/Group; повторные именованные оси и patch groupBy=[] с сохранением соседей | `source` | `apply` `template.add`<br>`apply`<br>`view`<br>`apply` `structure.add` `structure.add`<br>`apply`<br>`view`<br>`apply` `structure.patch` `structure.patch` `structure.patch`<br>`apply`<br>`view` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S376 | Типизированные значения reference/nil, список default/available values и именованный/Custom StandardPeriod; изменение ссылки сохраняет заявленный тип | `source` | `apply` `template.add`<br>`apply`<br>`view`<br>`apply` `parameter.add`<br>`apply`<br>`view`<br>`apply` `parameter.set` `parameter.set` `parameter.set`<br>`apply`<br>`view`<br>`apply` `parameter.set` `parameter.set` `parameter.set` `parameter.set`<br>`apply` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S377 | Условное оформление имеет отдельный typed filter, use=false, Color и многоязычный формат; заголовок и enum параметра вывода сохраняют платформенные типы | `source` | `apply` `template.add`<br>`apply`<br>`view`<br>`apply` `conditionalAppearance.add` `outputParameter.set` `outputParameter.set` `outputParameter.set`<br>`apply`<br>`view`<br>`apply` `outputParameter.set` `outputParameter.set`<br>`apply`<br>`view` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S378 | Folder создаётся отдельно от детей; parentIndexes из чтения точно выбирают вложенную выборку при set/remove и сохраняют соседей, заголовки и placement | `source` | `apply` `template.add`<br>`apply`<br>`view`<br>`apply` `selection.clear` `selection.add` `selection.add` `selection.add`<br>`apply`<br>`view`<br>`view`<br>`apply` `selection.set` `selection.set` `selection.remove`<br>`apply`<br>`view`<br>`apply` `selection.remove`<br>`apply` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S379 | Один итог имеет несколько explicit group bindings; set заменяет весь список, пустой список снимает привязки, соседний итог сохраняется | `source` | `apply` `template.add`<br>`apply`<br>`view`<br>`apply` `total.add`<br>`apply`<br>`view`<br>`apply` `total.set` `total.set`<br>`apply` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |

### Макеты

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S196 | Добавить печатный макет справочнику | `source` | `apply` `template.add` | `ok` |
| S197 | Поменять синоним макета | `source` | `apply` `template.set` | `ok` |
| S198 | Удалить устаревший макет | `source` | `apply` `template.remove` | `ok` |
| S199 | Добавить табличный макет справочнику и заполнить шапку одним пакетом | `source` | `apply` `template.add` `mxl.set` | `ok` |
| S200 | Посмотреть макеты отчёта перед правкой | `source` | `view` | `ok` |
| S201 | Проверить табличный макет | `source` | `check` валидаторы `mxl` | `ok` |
| S202 | Узнать по стандартам, как оформлять печатные формы | `source` | `docs` | `ok` / `provider` / `task` |
| S203 | Добавить печатный макет документу | `source` | `apply` `template.add` | `ok` |
| S277 | Добавить макет справочнику в выгрузке 2.21 — отказ по формату | `source` | `apply` `template.add` | `refused` |
| S331 | Исходный сценарий #1301: создать макет, две адресованные cells читаются по порядку; check=mxl passed | `source` | `view`<br>`apply` `template.add`<br>`apply`<br>`view`<br>`apply` `mxl.set`<br>`apply`<br>`view`<br>`check` валидаторы `mxl` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S332 | Скалярные cells/null, замена параметра/шаблона, соседи, рост без уменьшения ширины, повтор без дубликата, новая область и stale token | `source` | `view`<br>`apply` `mxl.set`<br>`apply` `mxl.set`<br>`apply`<br>`view`<br>`view`<br>`apply`<br>`apply` `mxl.set`<br>`apply`<br>`view`<br>`apply` `mxl.set`<br>`apply`<br>`view`<br>`check` валидаторы `mxl` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`refused`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S333 | Чтение шире редактирования: соседние строки, пересечения и порядок областей, языки и наборы колонок не теряются при отказе | `source` | `view`<br>`apply` `mxl.set`<br>`view`<br>`apply` `mxl.set`<br>`view`<br>`apply` `mxl.set`<br>`view`<br>`apply` `mxl.set`<br>`view`<br>`apply` `mxl.set`<br>`view`<br>`apply` `mxl.set`<br>`view`<br>`apply` `mxl.set`<br>`view`<br>`apply` `mxl.set`<br>`view`<br>`apply` `mxl.set`<br>`view`<br>`apply` `mxl.set`<br>`view`<br>`apply` `mxl.set` | `ok`<br>`refused`<br>`ok`<br>`refused`<br>`ok`<br>`refused`<br>`ok`<br>`refused`<br>`ok`<br>`refused`<br>`ok`<br>`refused`<br>`ok`<br>`refused`<br>`ok`<br>`refused`<br>`ok`<br>`refused`<br>`ok`<br>`refused`<br>`ok`<br>`refused` |
| S334 | Массив/пустые cells, координаты/типы, оформление, дробная ширина и адрес Area отказывают без записи | `source` | `view`<br>`apply` `mxl.set`<br>`apply` `mxl.set`<br>`apply` `mxl.set`<br>`apply` `mxl.set`<br>`apply` `mxl.set`<br>`apply` `mxl.set`<br>`apply` `mxl.set` | `ok`<br>`refused`<br>`refused`<br>`refused`<br>`refused`<br>`refused`<br>`refused`<br>`refused` |
| S335 | Реальный агент по can/справке создаёт MXL и заполняет область, сохраняет исходный макет, проверяет содержимое и честно объясняет предел DSL | `agent-evaluation/codex` | `view`<br>`view`<br>`check` валидаторы `mxl` | `ok`<br>`ok`<br>`ok` |

### Подсистемы и интерфейс

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S204 | Посмотреть подсистему администрирования | `source` | `view` | `ok` |
| S205 | Создать подсистему Продажи | `source` | `apply` `subsystem.create` | `ok` |
| S206 | Включить справочник в состав подсистемы | `source` | `apply` `content.add` | `ok` |
| S207 | Добавить объект в состав подсистемы и тут же исключить его одним пакетом | `source` | `apply` `content.add` `content.remove` | `ok` |
| S208 | Добавить дочернюю подсистему | `source` | `apply` `childSubsystem.add` | `ok` |
| S209 | Исключить дочернюю подсистему из состава родительской | `source` | `apply` `childSubsystem.remove` | `ok` |
| S210 | Проверить подсистему | `source` | `check` валидаторы `subsystem` | `ok` |
| S211 | Словарь операций подсистемы | `source` | `view` | `ok` |
| S212 | Проверить читаемость узла подсистемы | `source` | `check` валидаторы `subsystem` | `ok` |
| S213 | Узнать по стандартам, как строить командный интерфейс раздела | `source` | `docs` | `ok` / `provider` / `task` |
| S320 | Переставить важные команды Администрирования, забыв одну — перестановка проходит, неполный список отклоняется без записи | `source` | `apply` `commandOrder.set`<br>`apply` `commandOrder.set` | `ok`<br>`refused` |
| S321 | Задать порядок подсистем на интерфейсе подсистемы — порядок подсистем живёт в корне, отказ до планирования | `source` | `apply` `subsystemOrder.set` | `refused` |

### XDTO и обмены

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S214 | Добавить тип-значение в XDTO-пакет | `source` | `apply` `valueType.add` | `ok` |
| S215 | Добавить объектный тип в пакет | `source` | `apply` `objectType.add` | `ok` |
| S216 | Добавить свойство в объектный тип | `source` | `apply` `property.add` | `ok` |
| S217 | Удалить неиспользуемый тип из пакета | `source` | `apply` `type.remove` | `ok` |
| S218 | Удалить свойство типа | `source` | `apply` `property.remove` | `ok` |
| S219 | Узнать по документации, как устроены XDTO-пакеты | `source` | `docs` | `ok` / `provider` / `task` |
| S315 | Открыть один тип большого XDTO-пакета без находок по остальным типам | `source` | `view` | `ok` |
| S322 | Открыть шаблоны URL и метод HTTP-сервиса из выгрузки Конфигуратора | `source` | `view`<br>`view`<br>`view` | `ok`<br>`ok`<br>`ok` |
| S323 | Открыть операцию веб-сервиса из выгрузки Конфигуратора | `source` | `view`<br>`view`<br>`view` | `ok`<br>`ok`<br>`ok` |

### Сборка, ИБ и выгрузка

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S220 | Проверить готовность рабочего пространства перед сборкой | `source` | `check` | `ok` |
| S221 | Узнать по документации, как выгружать конфигурацию в файлы | `source` | `docs` | `ok` / `provider` / `task` |

### Проверки качества

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S222 | Проверить читаемость каждого ключевого объекта | `source` | `check` валидаторы `meta`<br>`check` валидаторы `meta`<br>`check` валидаторы `meta` | `ok`<br>`ok`<br>`ok` |
| S223 | Прогнать проверку описания объекта | `source` | `check` валидаторы `meta` | `ok` |
| S224 | Прогнать проверку корня конфигурации | `source` | `check` валидаторы `cf` | `ok` |
| S225 | Проверить форму списка | `source` | `check` валидаторы `form` | `ok` |
| S226 | Убедиться, что превью не тронуло исходники (дифф равенства) | `source` | `apply` `props.set`<br>`diff` | `ok`<br>`ok` |
| S227 | Сравнить перечисления после правки одного из них | `source` | `apply` `props.set`<br>`apply`<br>`diff` | `ok`<br>`ok`<br>`ok` |
| S228 | Проверить регистр после правки измерений (читаемость) | `source` | `check` валидаторы `meta` | `ok` |
| S229 | Точечный дифф стандартных реквизитов | `source` | `diff` | `ok` |
| S230 | Передать check фильтр — аргумента нет, валидаторы выбирает Unica по виду узла | `source` | `check` | `refused` |
| S231 | Проверить модуль на читаемость | `source` | `check` | `ok` / `provider` |
| S264 | Проверить читаемость отчёта | `source` | `check` валидаторы `meta` | `ok` |
| S265 | Сравнить наборы измерений регистра (равенство коллекций) | `source` | `diff` | `ok` |
| S266 | Проверить командный интерфейс подсистемы — валидатор interface выбран по узлу Interface | `source` | `check` валидаторы `interface` | `ok` |
| S267 | Проверить корень конфигурации в выгрузке формата 2.21 — проверка проходит и предупреждает о формате новее профиля | `source` | `check` валидаторы `cf` | `ok` |
| S268 | Проверить форму списка в выгрузке 2.21 — предупреждение о формате рядом с вердиктом | `source` | `check` валидаторы `form` | `ok` |
| S269 | Проверить роль в выгрузке 2.21 — предупреждение о формате рядом с вердиктом | `source` | `check` валидаторы `role` | `ok` |
| S270 | Проверить подсистему в выгрузке 2.21 с командным интерфейсом 2.21 — предупреждение о формате | `source` | `check` валидаторы `subsystem` | `ok` |
| S271 | Проверить корень конфигурации без атрибута версии — формат считается 1.0 и проверка предупреждает о миграции | `source` | `check` валидаторы `cf` | `ok` |
| S272 | Проверить отсутствующую подсистему — типизированный отказ, а не транспортная ошибка | `source` | `check` | `refused` |

### Тесты и запуск

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S232 | Узнать по документации, как подключить YaXUnit | `source` | `docs` | `ok` / `provider` / `task` |

### Документация и стандарты

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S233 | Как работает срез последних регистра сведений | `source` | `docs` | `ok` / `provider` / `task` |
| S234 | Что такое ХранилищеЗначения и как с ним работать | `source` | `docs` | `ok` / `provider` / `task` |
| S235 | Синтаксис директив компиляции модулей | `source` | `docs` | `ok` / `provider` / `task` |
| S236 | Как правильно назвать общий модуль по стандартам | `source` | `docs` | `ok` / `provider` / `task` |
| S237 | Стандарт оформления процедур и функций | `source` | `docs` | `ok` / `provider` / `task` |
| S238 | Правила использования привилегированного режима | `source` | `docs` | `ok` / `provider` / `task` |
| S239 | Поиск по обоим источникам сразу (без фильтра источника) | `source` | `docs` | `ok` / `provider` / `task` |
| S240 | Документация конфигурации пока не подключена (честная граница) | `source` | `docs` | `ok` / `provider` / `task` / `unsupported` |
| S241 | Неизвестный источник документации — закрытый список источников | `source` | `docs` | `ok` / `provider` / `task` / `unsupported` |
| S242 | Как объявить табличную часть в структуре метаданных | `source` | `docs` | `ok` / `provider` / `task` |
| S243 | Стандарт про обработку ошибок и Попытку | `source` | `docs` | `ok` / `provider` / `task` |
| S244 | Как работает механизм функциональных опций | `source` | `docs` | `ok` / `provider` / `task` |
| S262 | Как пользоваться длительными операциями на сервере | `source` | `docs` | `ok` / `provider` / `task` |
| S263 | Стандарт по именованию реквизитов объектов | `source` | `docs` | `ok` / `provider` / `task` |
| S297 | Пустой запрос к документации — отказ до обращения к поставщику | `source` | `docs` | `refused` |
| S298 | Неизвестный источник документации — отказ называет допустимые | `source` | `docs` | `unsupported` |
| S311 | Спросить документацию из каталога, который ещё не рабочая область | `source` | `docs`<br>`docs` | `ok` / `provider` / `task`<br>`unsupported` |

### Поддержка поставщика

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S245 | Посмотреть состояние поддержки объекта | `source` | `view` | `ok` |
| S246 | Выключить возможность изменения конфигурации на поддержке | `source` | `apply` `supportCapability.set` | `ok` |
| S247 | Разрешить редактирование объекта поставщика с сохранением поддержки | `source` | `apply` `supportRule.set` | `ok` |
| S248 | Снять объект с поддержки по стандарту — сначала документация | `source` | `docs` | `ok` / `provider` / `task` |
| S249 | Проверить поддержку всего проекта | `source` | `check` | `ok` |
| S274 | Разрешить редактирование объекта поставщика в выгрузке 2.21 — отказ по формату до чтения ParentConfigurations.bin | `source` | `apply` `supportRule.set` | `refused` |
| S280 | Включить возможность изменения в конфигурации, которая не на поддержке — типизированный отказ вместо тихого no-op | `source` | `apply` `supportCapability.set` | `refused` |
| S281 | Изменить свойство объекта поставщика на замке — отказ стража поддержки до первого байта | `source` | `apply` `props.set` | `refused` |
| S282 | Добавить значение перечислению поставщика на замке — отказ стража поддержки | `source` | `apply` `enumValue.add` | `refused` |
| S283 | Добавить форму объекту поставщика на замке — отказ стража поддержки | `source` | `apply` `form.add` | `refused` |
| S284 | Добавить макет объекту поставщика на замке — отказ стража поддержки | `source` | `apply` `template.add` | `refused` |
| S285 | Удалить объект поставщика на замке — отказ стража поддержки | `source` | `apply` `object.remove` | `refused` |
| S286 | Разрешить редактирование объекта на замке — операция поддержки проходит сквозь страж | `source` | `apply` `supportRule.set` | `ok` |
| S287 | Снять замок с объекта поставщика и после этого изменить его свойство | `source` | `apply` `supportRule.set`<br>`apply`<br>`apply` `props.set` | `ok`<br>`ok`<br>`ok` |
| S288 | Запретить изменения конфигурации на поддержке и попытаться изменить редактируемый объект — отказ по общему флагу | `source` | `apply` `supportCapability.set`<br>`apply`<br>`apply` `props.set` | `ok`<br>`ok`<br>`refused` |
| S295 | Удалить форму у объекта поставщика на замке — отказ стража поддержки до поиска формы | `source` | `apply` `form.remove` | `refused` |

### Расширения конфигурации

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S289 | Открыть корень набора-расширения — проекция корня как у конфигурации | `source` | `view` | `ok` |
| S290 | Проверить корень набора-расширения — валидатор cfe выбран по виду набора | `source` | `check` валидаторы `cfe` | `ok` |
| S291 | Проверить роль расширения — валидатор role работает и в наборе-расширении | `source` | `check` валидаторы `role` | `ok` |
| S292 | Проверить язык расширения — у узла нет валидаторов, ответ о читаемости | `source` | `check` | `ok` |
| S313 | Заимствовать отчёт и общий модуль в расширение и проверить расширение | `source` | `apply` `object.borrow` `object.borrow`<br>`apply`<br>`check` валидаторы `cfe` | `ok`<br>`ok`<br>`ok` |
| S314 | Создать собственный общий модуль расширения, вставить в него код и удалить модуль | `source` | `apply` `object.create`<br>`apply`<br>`apply` `code.insert`<br>`apply`<br>`apply` `object.remove`<br>`apply`<br>`check` валидаторы `cfe` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S319 | Вставить код в заимствованный общий модуль расширения | `source` | `apply` `object.borrow`<br>`apply`<br>`apply` `code.insert` | `ok`<br>`ok`<br>`gap` |

### Объекты метаданных

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S293 | Удалить объект, на который ещё ссылаются права роли, с force — план исполним и несёт предупреждение о сохранённых ссылках | `source` | `apply` `object.remove` | `ok` |
| S296 | Опубликовать удаление объекта без ссылок — результат несёт событие и влияние на кеш | `source` | `apply` `object.remove`<br>`apply` | `ok`<br>`ok` |
| S304 | Проверить объект в старой выгрузке — типизированный отказ поставщика вместо догадки | `source` | `check` | `provider` |

### Рабочее пространство

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S294 | Посмотреть готовность рабочего пространства без адреса — наборы, готовность источников и репозитория, проверки | `source` | `view`<br>`view` (как форма клиента) | `ok`<br>`ok` |
| S306 | Открыть рабочее пространство с несколькими наборами конфигурации — неоднозначность названа, а не скрыта отказом | `source` | `view` | `ok` |
| S309 | Позвать поверхность из каталога, который не является рабочей областью 1С — отказ называет причину и приводит туда, где отвечают без набора исходников | `source` | `search`<br>`view`<br>`check` | `refused`<br>`ok`<br>`ok` |
| S310 | Прочитать, сравнить и изменить узел в незаведённой области — три актор-связанных маршрута называют одну причину, а не безымянный сбой | `source` | `resolve`<br>`diff`<br>`apply` `props.set` | `refused`<br>`refused`<br>`refused` |

### Метаданные

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S347 | Предупреждение имеет свой код и сохраняет вердикт passed | `source` | `view`<br>`check` валидаторы `meta` | `ok`<br>`ok` |
| S348 | Предупреждение имеет свой код и сохраняет вердикт failed | `source` | `view`<br>`check` валидаторы `meta` | `ok`<br>`ok` |

### Приёмочный корпус

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S349 | EmptyRef, EnumValue и Characteristic читаются без потери данных и изменения XML | `source` | `view`<br>`view`<br>`view`<br>`check` валидаторы `meta` | `ok`<br>`ok`<br>`ok`<br>`ok` |
| S350 | Форма и области MXL принадлежат точному владельцу внешнего набора | `source` | `view`<br>`view`<br>`view`<br>`view`<br>`view`<br>`view`<br>`view`<br>`check` валидаторы `mxl` | `ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok`<br>`ok` |
| S351 | Физически существующие незарегистрированные форма/макет недоступны | `source` | `view`<br>`view` | `refused`<br>`refused` |
| S352 | Повреждённый payload не становится успехом; соседний владелец сохраняется | `source` | `view`<br>`check`<br>`view` | `provider`<br>`provider`<br>`ok` |
| S353 | Ссылка на payload соседнего владельца отклоняется; собственная соседняя форма доступна | `source` | `view`<br>`view`<br>`view` | `provider`<br>`provider`<br>`ok` |
| S358 | EPF/ERF без main доступны, отсутствует требование sourceDir | `source` | `view`<br>`view`<br>`view` | `ok`<br>`ok`<br>`ok` |
| S359 | Отдельные Object-модули EPF/ERF и модуль формы достигают терминального BSL результата | `delivery/bsl-analyzer-external` | `check`<br>`check` валидаторы `bsl`<br>`check` валидаторы `bsl`<br>`check` валидаторы `bsl` | `ok`<br>`ok`<br>`ok`<br>`ok` |
| S360 | Недоступный объявленный внешний набор даёт failed с source_set.path_missing | `source` | `view`<br>`check` | `ok`<br>`ok` |

### Известные пробелы

| № | Задача | Профиль | Провод | Классы шагов |
| --- | --- | --- | --- | --- |
| S354 | Включить справку отчёта: props.set IncludeHelpInContents пока отказывает (#896) | `source` | `apply` `props.set` | `gap` |
| S355 | Создать регистр, затем сменить RegisterType на Turnovers: props.set пока отказывает (#852) | `source` | `apply` `object.create`<br>`apply`<br>`apply` `props.set` | `ok`<br>`ok`<br>`gap` |
| S356 | Перенести существующее поле вместе с companions в существующую группу: element.move пока отсутствует (#919) | `source` | `apply` `element.move` | `gap` / `unsupported` |
| S357 | Выделить три helper-метода одной заменой: текущий отказ сохранён до исправления #895 | `source` | `apply` `code.replace` | `gap` |
