# Логическая адресация целей 1С

- Date: `2026-07-29`
- Status: `approved`
- Decision: `ADR-0021`

## Результат проектирования

Публичный селектор существующего объекта метаданных или модуля не должен
зависеть от каталога Platform XML. Каноническая идентичность состоит из
`sourceSet` и логического `metadataPath`, а физический путь остаётся
наблюдаемым местоположением и закрытой ручкой поставщика.

Проект намеренно разделён на два решения:

- ADR-0021 владеет адресами, корнями, `resolve/children`, местоположениями,
  селекторами создания и миграцией предметных инструментов;
- ADR-0022 владеет `resources/read/apply`, снимками, ресурсными handles и
  дополнительными гарантиями низкоуровневой записи.

Разделение не означает отказ от resource API. Оно не позволяет временной
аварийной поверхности определять постоянную идентичность сущностей.

## Проверенный baseline

### Источники и корни

`unica.project.map` сейчас публикует для набора только:

- `name`;
- `kind`;
- `path`;
- `sourceFormat`;
- `formatEvidence`.

Автоопределение создаёт только `main/Configuration` для `.`, `src` или
`src/cf`. `Extension`, `ExternalProcessor` и `ExternalReport` требуют
объявления в `v8project.yaml`.

Набор `Configuration` или `Extension` соответствует одному корню
`Configuration.xml`. Набор `ExternalProcessor` или `ExternalReport` является
контейнером: рядом могут находиться `First.xml`/`First/`,
`Second.xml`/`Second/` и следующие артефакты. Поэтому один `sourceSet`
внешнего вида не выбирает конкретный объект.

### Уже опубликованные логические селекторы

`unica.cfe.patch_method` уже публикует `ModulePath`. Его grammar:

```text
CommonModule.Name
Type.Name.ObjectModule
Type.Name.ManagerModule
Type.Name.RecordSetModule
Type.Name.ValueManagerModule
Type.Name.Form.FormName
```

Это логический адрес, а не файловый путь. Проект обязан мигрировать его, а не
утверждать, что `modulePath` не существовал.

`moduleHint` у `unica.code.definition` имеет другую семантику: это
регистронезависимая подстрока, которая сужает выдачу при одинаковом имени
метода. Она не идентифицирует цель и не должна превращаться в обязательный
точный адрес.

`Object` у `unica.cfe.borrow` допускает батч через `;;`, а `object` у runtime
описывает собственную предметную цель. Они требуют отдельной инвентаризации и
не переводятся механической заменой имени поля.

### Публичные файловые селекторы

Текущая поверхность смешивает несколько разных ролей пути:

| Роль | Примеры | Целевая судьба |
| --- | --- | --- |
| выбор набора анализа | `sourceDir` | `sourceSet` после миграции инструмента; до неё действует ADR-0006 |
| точная существующая цель | `path`, `ObjectPath`, `FormPath`, `TemplatePath`, `RightsPath`, `SubsystemPath`, `CIPath`, часть `ConfigPath`/`ExtensionPath` | `SourceTarget` |
| логический модуль | `ModulePath` | общая grammar `metadataPath` с таблицей миграции |
| поисковое сужение | `moduleHint` | остаётся filter/prefix, не становится identity |
| создание внутри набора | `OutputPath`, `OutputDir`, `Parent`, `SrcDir` у compile/add операций | родительская цель + новое имя |
| создание нового набора/артефакта | `cf.init`, `cfe.init`, `epf.init`, `erf.init` | предметный выходной путь остаётся |
| явная payload | `JsonPath`, `DefinitionFile` | файловый путь остаётся |
| публикация/экспорт | output/package destinations | файловый путь остаётся |

До изменения конкретного инструмента его фактический код и схема остаются
источником истины. Эта таблица задаёт классификацию миграции, а не подменяет
полный inventory, который составляется для каждого implementation slice.

### Местоположения результатов

`git-grep` выполняет поиск по всему корню набора и может найти строку в
`Configuration.xml`, `Rights.xml`, `Form.xml` или другом ресурсе без
самостоятельного `MetadataAddress`. Аналогичные результаты возможны у
диагностики формата и валидации регистраций.

Нельзя одновременно требовать адрес у каждого результата и запрещать
фиктивные адреса. Публичное местоположение должно различать:

- адресуемую цель;
- неадресуемый ресурс с ближайшим адресуемым владельцем;
- позицию внутри наблюдаемого ресурса.

Физический путь может присутствовать во втором варианте как местоположение
текущего provider-а, но не как идентичность сущности.

### Действующие writer-гарантии

Containment сейчас начинается с объявленных публичных путевых аргументов.
`tool_context.rs` получает список `write_path_args`; пустой список завершает
проверку успешно. Механическая замена пути на адрес выключила бы
`INV-SOURCE-WRITE-CONTAINMENT`.

Поэтому адресный resolver должен возвращать закрытую ручку, а инфраструктурный
writer обязан повторно пропускать фактическую цель через containment и
остальные проверки. Нельзя считать, что скрытый путь автоматически безопасен.

## Рассмотренные варианты

### Сохранить пути и расширить `code.patch`

Самый короткий путь к #260 — разрешить `Extension` в действующем resolver-е.
Он полезен как отдельный первый vertical slice, но не решает переносимость
остальных точных селекторов и оставляет несколько несовместимых logical
grammars.

### Перенести целиком модель PR #210

PR #210 доказывает ценность provider boundaries, версий, completeness и
закрытых handles. Однако его публичный `objectKey`, discovery graph и большой
новый crate не нужны для решения адресации. Кроме того, PR смешивает
идентичность, навигацию, resource access и writer semantics.

### Небольшое адресное ядро плюс отдельный resource ADR

Выбран этот вариант:

- один `SourceTarget`;
- один `MetadataAddress` для объектов и модулей;
- два read-only инструмента `resolve/children`;
- типизированные addressed/unaddressable locations;
- отдельные контракты точного выбора, создания и поискового фильтра;
- поинструментная миграция;
- resource access остаётся обязательным направлением, но живёт в ADR-0022.

## Модель идентичности

```text
SourceTarget =
  sourceSet
  + optional MetadataAddress
```

`sourceSet` — точное имя из карты проекта. Для новой канонической точной
операции оно обязательно. Старые операции продолжают использовать
детерминированный default из ADR-0006 до собственного migration slice.

Отсутствующий `metadataPath` — не `null target`, а корень:

| Вид source set | Значение `sourceSet` без `metadataPath` |
| --- | --- |
| `Configuration` | корневой объект конфигурации |
| `Extension` | корневая изолированная проекция расширения |
| `ExternalProcessor` | виртуальный контейнер внешних обработок |
| `ExternalReport` | виртуальный контейнер внешних отчётов |

Виртуальный контейнер внешних артефактов не является
`metadataObject`. Точная внешняя цель обязана включать вид и имя:

```text
ExternalDataProcessor.Import
ExternalReport.Sales
ExternalDataProcessor.Import.Form.Main
ExternalReport.Sales.Form.ReportForm
```

Это устраняет противоречие прежнего проекта: примеры включали имя внешнего
артефакта, хотя правило утверждало, что `sourceSet` уже выбрал единственный
корень.

## `MetadataAddress`

Адрес состоит из точечных сегментов:

- платформенный вид;
- прикладное имя;
- дочерний платформенный вид и имя;
- при необходимости терминал прикреплённого модуля.

Корневые коллекции Platform API не входят в адрес. Например, выражение
`Метаданные.Справочники.Контрагенты` соответствует объектному адресу:

```text
Catalog.Counterparties
```

Канонический wire-вывод использует английские платформенные токены. Поставщик
принимает точные русские и английские псевдонимы и преобразует их в одно
типизированное представление. Прикладное имя не переводится и возвращается в
написании источника.

Этот выбор предпочтительнее `addressDialect=ru|en`:

- один адрес стабилен между provider-ами;
- Platform XML уже использует английские ссылки;
- локаль процесса не влияет на identity;
- `sourceSet` не обязан хранить недоказанную языковую характеристику.

Профиль адресов версионирован как данные. Первый профиль соответствует
платформе 8.3.27 и Platform XML 2.20. Он содержит только подтверждённые:

- виды верхнего уровня;
- вложенные коллекции;
- addressability;
- псевдонимы;
- допустимые module terminals.

Неизвестный сегмент или профиль fail closed.

## Модули

Модуль использует тот же `MetadataAddress`, но получает
`targetKind=module`. Терминал нужен, чтобы объект формы и модуль формы не имели
одного адреса.

Канонические примеры Unica:

```text
CommonModule.General.Module
Catalog.Counterparties.ObjectModule
Document.Sales.ManagerModule
Document.Sales.Form.List.FormModule
```

`Module`, `ObjectModule`, `ManagerModule` и `FormModule` являются терминалами
профиля Unica. Проект не называет их дословными именами из HBK или отладчика.
Root modules не публикуются, пока их таблица не подтверждена официальной или
runtime-фикстурой 8.3.27.

Миграция существующего `ModulePath`:

| Старое значение | Новый `metadataPath` |
| --- | --- |
| `CommonModule.Name` | `CommonModule.Name.Module` |
| `Type.Name.ObjectModule` | без изменения терминала |
| `Type.Name.ManagerModule` | без изменения терминала |
| `Type.Name.RecordSetModule` | без изменения терминала |
| `Type.Name.ValueManagerModule` | без изменения терминала |
| `Type.Name.Form.FormName` | `Type.Name.Form.FormName.FormModule` |

Тип `ResolvedModule` допустим внутри application/infrastructure как
доказательство `targetKind`, но не вводит второй wire-selector.

## Разрешение и обход

### `unica.source.resolve`

Вход:

- `sourceSet`;
- `query`;
- режим `exact|prefix`;
- необязательный `targetKind`;
- cursor и limit.

Выход:

- ограниченный список кандидатов;
- канонический `metadataPath`;
- `targetKind`;
- отображаемое имя;
- признак точного или префиксного совпадения;
- completeness и cursor.

Инструмент не выполняет fuzzy ranking и не выбирает один из нескольких
кандидатов. Платформенный тип и UUID объекта текущий wire-контракт не
возвращает: идентичностью остаётся только `sourceSet + metadataPath`.

### `unica.source.children`

Вход:

- `sourceSet`;
- необязательный точный `metadataPath`;
- cursor и limit.

Поведение:

- для Configuration/Extension без адреса перечисляет корневые коллекции и
  подтверждённые root modules;
- для ExternalProcessor/ExternalReport без адреса перечисляет отдельные
  артефакты;
- для точной цели перечисляет один уровень непосредственных детей;
- коллекции возвращаются как узлы `nodeKind: "collection"` и не принимаются
  отдельным входным аргументом;
- collection-узел и неадресуемый элемент не получают фиктивный address;
- элемент сообщает ближайшего адресуемого владельца и completeness.

## Типизированное местоположение

Каноническая форма:

```text
SourceLocation =
  AddressedLocation
  | UnaddressableResourceLocation
```

`AddressedLocation`:

```json
{
  "kind": "addressed",
  "sourceSet": "main",
  "metadataPath": "CommonModule.General.Module",
  "targetKind": "module",
  "range": {"startLine": 10, "endLine": 12}
}
```

`UnaddressableResourceLocation`:

```json
{
  "kind": "unaddressable",
  "sourceSet": "main",
  "ownerMetadataPath": "Role.Accountant",
  "path": "Roles/Accountant/Ext/Rights.xml",
  "range": {"startLine": 4, "endLine": 4}
}
```

`path` во второй форме является относительным наблюдаемым местоположением
provider-а. Клиент не может использовать его как переносимый identity.

Существующий `INV-MCP-CODE-SEARCH-SECTIONS` сохраняется: секции `rlm`,
`bsl-analyzer` и `git-grep` не смешиваются. Каждый provider нормализует только
то, что способен доказать; отсутствие адреса не скрывает hit.

## Создание

Операция над существующей целью принимает `SourceTarget`.

Создание дочернего объекта использует:

```text
CreateTarget =
  sourceSet
  + parentMetadataPath?  // отсутствие означает addressable root
  + newName
```

Предметная операция по-прежнему определяет создаваемый kind. Пользователь не
передаёт произвольный platform type, если инструмент уже задаёт его своим
именем.

Примеры:

- `form.compile`: владелец + новое имя формы;
- `dcs.compile`, `mxl.compile`, `template.add`: владелец + новое имя макета;
- `meta.compile`: родительский адрес/корень + новый kind/name;
- `subsystem.compile`: родительская подсистема/корень + новое имя;
- `help.add`: существующий owner + создаваемая роль help.

Создание `Configuration`, `Extension`, `ExternalProcessor` или
`ExternalReport` как нового артефакта сохраняет output path: до создания
логического target ещё не существует.

## Граница поставщика и writer-а

Resolver возвращает:

```text
ResolvedTarget {
  sourceSet,
  canonicalAddress?,
  targetKind,
  providerId,
  closedHandle,
  evidence
}
```

`closedHandle`:

- не сериализуется;
- связан с provider/source set/target;
- не принимается от клиента;
- не доказывает writer capability.

Перед мутацией handler:

1. повторно разрешает target или валидирует generation handle;
2. получает фактические ресурсы;
3. проверяет каждый write path через `WorkspacePathPolicy::resolve_write`;
4. проверяет Platform XML owner/profile;
5. проверяет support;
6. связывает точные preimages с mutation plan;
7. строит preview и apply из одного плана;
8. публикует атомарно;
9. возвращает post-hash/diff/ranges/validation;
10. публикует доменное событие только после успеха.

Таким образом removal публичного path не создаёт bypass
`INV-SOURCE-WRITE-CONTAINMENT`.

## Миграция публичной поверхности

Миграция выполняется по инструментам, а не одним PR на 43+ операций.

Для каждого среза обязательны:

1. точный inventory текущих arguments и result locations;
2. классификация поля: identity, scope, search filter, create target, payload
   или destination;
3. новая data-driven schema;
4. typed legacy error с новым selector;
5. README migration row;
6. release-note summary;
7. skill/examples parity;
8. ADR/invariant/acceptance sync;
9. consumer test старого отказа и нового успеха.

`tools/list` не публикует одновременно старый identity path и
`metadataPath`. Runtime bridge допустим только если отдельный slice докажет
однозначное преобразование и ограничит срок; по умолчанию ломающий переход
возвращает typed replacement error.

ADR-0019 сохраняется для настоящих file arguments. Для удаляемого identity path
ADR-0021 является более новым решением: semantic replacement не называется
alias normalization.

## Реализованный срез

PR #266 реализует адресное ядро, `unica.source.resolve`,
`unica.source.children` и миграцию `unica.code.patch`:

- Configuration + Extension пишутся в Platform XML 8.3.27/2.20;
- ExternalProcessor/ExternalReport с несколькими артефактами доступны для
  read-only навигации;
- существующий BSL-модуль выбирается через `sourceSet + metadataPath`;
- предпросмотр и применение одной вставки сохраняют побайтовые гарантии,
  BSL-парсер и повторную проверку closed handle;
- `sourceDir/path` получают `legacy_target_removed`.

Миграция `cfe.patch_method`, exact-target предметных инструментов и
местоположений code intelligence остаётся отдельными последующими изменениями;
публичный адрес для них уже определён ADR-0021.

## Ошибки

Минимальные стабильные коды:

- `source_set_required`;
- `source_set_not_found`;
- `source_root_not_addressable`;
- `metadata_address_invalid`;
- `metadata_address_ambiguous`;
- `metadata_address_not_found`;
- `target_kind_mismatch`;
- `address_profile_unsupported`;
- `unaddressable_location`;
- `legacy_target_removed`;
- `containment_denied`;
- `writer_capability_missing`.

Человекочитаемый текст не является кодом ошибки. `legacy_target_removed`
называет канонические replacement fields.

## Проверки

При принятии ADR проверено:

- inventory публичных селекторов сверяется с `operation_descriptors.rs` и
  `tool_contracts.rs`;
- внешний fixture содержит два артефакта в одном source set;
- module profile сопоставляется с существующей `ModulePath` grammar;
- root modules не добавляются без platform/runtime evidence;
- каждый create tool классифицирован отдельно;
- `git-grep` fixture содержит адресуемый BSL hit и неадресуемый XML hit;
- closed handle containment проверяется отдельно от public path arguments.

Исполняемые проверки покрывают:

- unit tests parser/canonicalizer для ru/en aliases;
- Configuration/Extension root tests;
- ExternalProcessor/ExternalReport multi-artifact roots;
- exact/prefix resolve and direct children;
- addressed/unaddressable location serialization;
- migration failure and replacement hint;
- existing ADR-0019 path aliases for genuine file fields;
- preview/apply parity, preimages, containment and support;
- no cache event on preview/failure;
- one typed event on successful mutation.

## Парная оценка работы ИИ

Оценка выполняется не в одном меняющемся дереве, а на двух закреплённых
ревизиях:

1. baseline `origin/main` до migration slice;
2. commit-кандидат после slice.

Одинаковый набор задач включает поиск цели, чтение результата, preview и apply.
Для baseline разрешены только реально опубликованные arguments; для кандидата —
только новая schema. Результат сохраняется в
`docs/provenance/reviews/YYYY-MM-DD-logical-addressing-ai-evaluation.md`.

Проход:

- каждая baseline-задача, поддержанная обеими ревизиями, остаётся выполнимой;
- сценарий #260 завершается через MCP без ручного файлового редактирования;
- кандидат не требует от модели знания Platform XML directory layout;
- unaddressable result не получает вымышленный address;
- число ошибочных tool calls не выше baseline;
- все применяющие вызовы начинаются с успешного preview.

Это `manual` acceptance artifact, а не нормативный владелец контракта.

## Риски

- Профиль адресов может неполно описать редкий module kind. Fail-closed и
  fixtures важнее эвристического покрытия.
- English canonical tokens менее привычны русскоязычному пользователю, но дают
  один wire identity; UI/skills могут показывать displayName на нужном языке.
- Поинструментная миграция временно оставляет разные селекторы у разных tools.
  Migration table и typed errors делают границу явной.
- Closed handle добавляет новую точку безопасности. Он не должен подменять
  containment, support или preimage checks.
- Resource API остаётся существенным риском, но его расширение не может
  незаметно изменить ADR-0021: им владеет ADR-0022.

## Что берётся из PR #210

Берётся:

- provider boundary;
- named source sets;
- explicit format/profile evidence;
- closed handles;
- completeness;
- read/write capability separation.

Не берётся:

- public `objectKey`;
- full discovery graph;
- semantic action graph;
- новый crate до доказанного второго provider-а;
- resource writer внутри адресного ADR.
