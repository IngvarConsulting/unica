- Date: `2026-08-10`
- Status: `draft`
- Decision: `ADR-0041`

# Полная типизированная read-model `unica.meta.info`

## Результат

`unica.meta.info` сохраняет действующий логический адрес, общий конверт и
обратную совместимость текущих полей, но перестаёт выводить полноту чтения из
возможностей writer-а. Для чтения вводятся отдельные закрытые профили корневых
свойств и всех 23 видов метаданных, более широкая алгебра наблюдаемых типов и
типизированный `details`, вариант которого определяется верхнеуровневым `kind`.

Исполняемая fixture-матрица классифицирует каждый семантический узел
`Properties` и `ChildObjects` профиля 8.3.27/2.20. Новый или уже существующий
составной узел нельзя пропустить молча: он либо попадает в типизированное поле,
либо даёт диагностику и недоказанное значение. Это устраняет причину #293, а не
добавляет очередной частный parser.

Проект меняет публичный контракт результата существующего инструмента, поэтому
архитектурный выбор принадлежит proposed ADR-0041. Пока решение не принято
вместе с реализацией, действующие ADR, инварианты и ведомость поверхности не
объявляют новую полноту доставленной.

## Исходное состояние

### Что из #274 уже доставлено

Текущий `main` на `b0205624` больше не соответствует первоначальному repro
[#274](https://github.com/IngvarConsulting/unica/issues/274).

| Требование #274 | Текущее наблюдаемое место |
| --- | --- |
| Владельцы справочника | `data.relations.owners` |
| `Hierarchical`, `CodeLength`, `DefaultPresentation` | `data.properties` |
| Формы | `data.collections.forms` |
| Предопределённые элементы | явно запрошенная секция `data.predefinedItems` |
| Неработающий `Detailed` | удалён из публичной схемы в PR #285 |

Следовательно, #274 остаётся полезным исходным пользовательским сценарием, но
больше не описывает самостоятельный дефект runtime. Реализация ADR-0041 должна
закрыть #274 по подтверждённой матрице, а не повторно реализовать перечисленные
поля.

### Что из #293 уже доставлено

Две части [#293](https://github.com/IngvarConsulting/unica/issues/293) также
устарели:

- companion `Ext/Predefined.xml` читается через логическую цель в явно
  выбранную секцию `predefinedItems`;
- `EventSubscription.Source` возвращается в `relations.source` как закрытое
  логическое объединение ADR-0039; `Event` и `Handler` доступны в
  `properties`.

Оставшиеся подтверждённые потери:

- `Constant.Properties.Type`;
- `DefinedType.Properties.Type`;
- `ScheduledJob.Properties.MethodName`;
- `HTTPService.ChildObjects.URLTemplate` и вложенные `Method`;
- `WebService.Properties.XDTOPackages`, операции, параметры, направления и
  возвращаемые XDTO-типы;
- типы, допустимые при чтении, но отсутствующие в публичной мутационной
  алгебре, например `v8:UUID`.

Матрица #293 ошибочно требует составной `Schedule` у `ScheduledJob`. В формате
8.3.27 регламентное задание содержит `MethodName`, `Use`, `Predefined` и
параметры перезапуска. Тройка `Schedule`, `ScheduleValue`, `ScheduleDate`
принадлежит `CalculationRegister`. Проект закрепляет корректного владельца и не
переносит ошибку постановки в контракт.

### Системная причина

`read_typed_meta_info` строит `properties` по `METADATA_PROPERTY_SPECS`. Этот
реестр одновременно ограничивает `setProperties`, поэтому читатель видит в
основном то, что умеет менять writer. Для типов дочерних элементов чтение также
переиспользует мутационную `MetadataType`: валидный read-only тип становится
`null`, а элемент помечается неполным.

Тест `info_observes_every_typed_mutation_field` доказывает паритет с writer-ом,
но логически не может доказать полноту reader-а. Метка `typed` в
`tool-surface-review.json` доказывает форму результата и отсутствие prose, а не
то, что все семантические факты дескриптора представлены. Смешение этих двух
понятий и есть причина повторяющихся потерь.

## Цели

1. Вернуть все семантические факты, обещанные действующими форматными
   спецификациями, подтверждёнными fixtures и прежним reader-поведением для 23
   поддерживаемых видов.
2. Не связывать полноту чтения с тем, что публично изменяет `meta.add/edit`.
3. Различать неприменимость, доказанное отсутствие или пустоту и невозможность
   доказать значение.
4. Не публиковать XML QName, namespace aliases, физические пути или сырые
   фрагменты как обход типизации.
5. Сделать появление непокрытого составного узла падающей проверкой.
6. Сохранить логическую адресацию ADR-0021, типизированный конверт ADR-0023,
   четыре операции ADR-0025 и связку подписки ADR-0039.

## Неграницы

- Новый инструмент, аргумент, режим `Detailed` или универсальный `raw` не
  добавляется.
- `unica.meta.info` не становится lossless XML decompiler и не обещает
  побайтовое восстановление дескриптора из ответа.
- Companion `predefinedItems` остаётся явно выбираемой локальной секцией с
  существующим `limit`; descriptor-local `details` не получает отдельного
  селектора.
- Публичная алгебра мутаций не расширяется только потому, что reader научился
  наблюдать поле.
- Проект не меняет формат XML и ничего не записывает в дерево исходников.
- Design PR не закрывает #293 и #274: закрытие требует executable fixtures и
  runtime-реализацию.

## Рассмотренные подходы

### 1. Добавить недостающие поля в текущие общие структуры

Можно добавить `type`, `urlTemplates`, `operations` и `methodName` рядом с
существующими полями `MetaInfoData`.

Преимущество — маленький первоначальный diff. Недостаток — причина остаётся:
следующее составное поле снова зависит от ручного обнаружения. Общая структура
наполняется полями, неприменимыми к большинству видов, а тесты продолжают
доказывать только текущий список частных исправлений. Подход отвергнут.

### 2. Закрытый read-профиль свойств и kind-specific `details`

Каждый из `MetadataKind::ALL` получает ровно один вариант read-model. Общие
поля сохраняются. Корневые свойства проецируются отдельным read-side реестром,
а составные и вложенные факты, не представимые общей секцией, попадают в
`details`. Профиль вида классифицирует прямые узлы дескриптора и не
переиспользует writer как список разрешённых наблюдений.

Преимущество — полнота становится исполняемым свойством, а формат ответа
остаётся предметным и логическим. Цена — 23 варианта и fixture для каждого
вида, даже если `details` конкретного вида пока пуст. Это выбранный подход.

### 3. Нормализованное XML-дерево в `data`

Можно вернуть рекурсивное дерево элементов, атрибутов и текстов и тем самым
сохранить любой узел без предметного моделирования.

Такой ответ технически полнее, но переносит наружу wire-структуру, QName,
порядок форматных контейнеров и детали поставщика. Потребитель снова вынужден
понимать XML 1С, а `data` превращается в другой синтаксис raw XML. Подход
противоречит ADR-0021/ADR-0023 и отвергнут.

## Публичная read-model

### Совместимый верхний уровень

Сохраняются существующие поля:

- `metadataPath`, `kind`, `name`, `synonym`, `support`;
- `properties`, `relations`, `collections`;
- `functionalSubsystems`, `interfaceSubsystems`;
- `predefinedItems`, `usage`, `validation`.

Добавляется обязательный объект `details`. В Rust пару `kind + details`
сериализует закрытый enum с представлением `#[serde(tag = "kind", content =
"details")]`, расплющенный в `MetaInfoData`. Поэтому невозможно построить
`kind: "HTTPService"` с вариантом деталей `WebService`.

Пример константы:

```json
{
  "metadataPath": "Constant.MainCurrency",
  "kind": "Constant",
  "name": "MainCurrency",
  "details": {
    "type": {
      "variants": [
        {
          "kind": "reference",
          "metadataPath": "Catalog.Currencies"
        }
      ]
    }
  }
}
```

Пример HTTP-сервиса:

```json
{
  "metadataPath": "HTTPService.ExternalAPI",
  "kind": "HTTPService",
  "name": "ExternalAPI",
  "details": {
    "urlTemplates": [
      {
        "name": "Metrics",
        "template": "/v1/kpi/",
        "methods": [
          {
            "name": "Get",
            "httpMethod": "GET",
            "handler": "MetricsGet"
          }
        ]
      }
    ]
  }
}
```

JSON-форма существующих элементов `properties` и написание уже опубликованных
ключей сохраняются. Однако их источником становится отдельный закрытый
`MetaInfoPropertyProfile`: он включает нынешнее writer-подмножество и все
прочие доказанные корневые свойства, публичным владельцем которых является
общая секция `properties`. Корневой XML-узел не обязан становиться общей
property: логически связанный или специфичный виду факт явно маршрутизируется
в identity, relation либо `details`. Новые read-only ключи `properties`
добавочны для текущих потребителей, а добавление ключа в этот профиль не
расширяет `meta.add/edit`.

`details` не дублирует значения, которые естественно представлены общей
секцией. Например, `EventSubscription.Event` и `Handler` остаются в
`properties`, а `Source` — в `relations.source`. Вариант
`EventSubscription` в `details` может быть пустым только после того, как
fixture-классификатор доказал, что остальные семантические узлы этого вида
маршрутизированы в общие секции.

### Варианты `details`

Закрытый enum содержит все 23 варианта. На первой реализации дополнительные
поля нужны следующим владельцам:

| Вид | Поля `details` |
| --- | --- |
| `Constant` | `type` |
| `DefinedType` | `type` |
| `CalculationRegister` | логически связанная тройка `schedule` |
| `ScheduledJob` | логическая ссылка `method` |
| `HTTPService` | `urlTemplates[].methods[]` |
| `WebService` | `xdtoPackages`, `operations[].parameters[]` |
| остальные 17 видов | `{}` допустим только если fixture-профиль доказал отсутствие дополнительных вложенных фактов; обнаруженный факт добавляет поле соответствующему варианту |

Пустой `details` означает отсутствие дополнительных kind-specific полей у
данного варианта, а не пустоту неизвестного XML. Fixture-классификатор всё равно
проверяет прямые узлы этого вида и не позволяет получить `{}` из-за пропуска.

`CalculationRegister.details.schedule` имеет форму:

```json
{
  "register": "InformationRegister.WorkSchedules",
  "valueField": "InformationRegister.WorkSchedules.Resource.DayValue",
  "dateField": "InformationRegister.WorkSchedules.Dimension.Date"
}
```

Тройка либо доказана целиком, либо равна `null` с диагностикой на первом
несогласованном поле. Частичная структура не публикуется как рабочая связь.

`ScheduledJob.details.method` разделяет адрес общего модуля и имя процедуры,
но сериализуется только логическими значениями:

```json
{
  "metadataPath": "CommonModule.MonthClose",
  "method": "RunScheduled"
}
```

XDTO-типы WebService публикуются расширенным именем `{namespace, localName}`.
Префикс XML не входит в результат, потому что два разных префикса могут
обозначать одно и то же имя.

## Разделение read- и write-алгебры типов

Новая `ObservedMetadataType` описывает всё доказанное чтением в профиле
8.3.27/2.20: примитивы с квалификаторами, ссылки по логическим адресам,
`DefinedType`, `ValueStorage`, `UUID` и остальные варианты, закреплённые
форматными fixtures. Она не даёт права записи.

Действующая `MetadataType` остаётся более узким типом мутации. Writer получает
её только явным fallible-преобразованием из наблюдаемого типа или разбором
публичной мутационной нагрузки. Поэтому добавление read-only варианта не
расширяет `meta.add/edit` случайно.

Один wire-parser разбирает `Type` в нейтральные наблюдаемые факты. Политика
чтения сохраняет все варианты; политика записи отдельно решает, какие из них
допустимы. Это устраняет нынешнюю ветку, которая узнаёт валидный `v8:UUID`, но
всё равно возвращает `type: null`.

## Семантика отсутствия и неполноты

Для kind-specific поля действуют три различимых состояния:

1. Поле отсутствует из JSON-структуры варианта — свойство неприменимо к виду.
2. Поле присутствует со значением `null` — применимо, но значение отсутствует
   в источнике либо не доказано; во втором случае `validation.diagnostics`
   обязательно называет поле и причину.
3. Массив равен `[]` — соответствующий контейнер прочитан полностью и доказанно
   пуст.

Неполностью разобранный массив не публикуется как полный. Его значение равно
`null`, уже доказанные общие данные сохраняются, `validation.status` равен
`failed`, а диагностика содержит логический `metadataPath` и публичный путь
поля. Физический путь и XML-фрагмент в неё не попадают.

Ошибка корневой идентичности или невозможность безопасно прочитать точный
descriptor image остаётся жёстким отказом без выдуманной `data`. Неизвестный
семантический узел поддерживаемого профиля даёт `provider_unavailable`, а не
молчаливый пропуск.

## Исполняемая полнота

### Профиль вида

`MetaInfoPropertyProfile` — закрытый read-side реестр корневых свойств. Для
каждой пары `MetadataKind + property` он задаёт публичный ключ, ожидаемую
read-side алгебру значения и применимость, если публичным владельцем узла
является общая секция `properties`. Он не импортирует writer-разрешение из
`METADATA_PROPERTY_SPECS`; совпадающие записи связаны отдельным тестом на
сохранение уже опубликованных ключей и значений.

`MetaInfoKindProfile` для каждого `MetadataKind::ALL` классифицирует:

- direct children `Properties` как identity, запись
  `MetaInfoPropertyProfile`, relation или поле конкретного `details`;
- direct children `ChildObjects` как общую коллекцию или kind-specific
  вложенную структуру;
- относящийся к объекту companion как существующую явно выбираемую секцию;
- только форматные и дублирующие узлы как явное исключение с одной причиной.

Семантический узел нельзя пометить исключением. Если fixture или спецификация
доказывает прикладное значение, профиль обязан дать ему публичное typed-место.
Повторно используемые причины исключения закрыты перечислением, чтобы строка
`ignored` не стала обходом полноты.

### Fixture-матрица

Отслеживаемый manifest содержит ровно одну основную platform fixture на каждый
из 23 видов и дополнительные edge fixtures для составных полей. Guard
утверждает одновременно:

- множество fixture-kind равно `MetadataKind::ALL`;
- каждый прямой семантический узел fixture классифицирован профилем;
- каждый объявленный профильный маршрут наблюдаем хотя бы в одной fixture;
- `kind` и вариант `details` совпадают;
- неизвестный составной узел делает тест красным, а не исчезает из результата.

Минимальные regression fixtures из #293 дополняются исправленной матрицей:

| Fixture | Доказательство |
| --- | --- |
| `Catalog` | owners, hierarchy/code/presentation, forms, opt-in predefined items |
| `Constant` | полный наблюдаемый `type` |
| `DefinedType` | type set и квалификаторы |
| `EventSubscription` | `Source`, `Event`, `Handler` без wire QName |
| `HTTPService` | root URL, URL templates, methods, verbs, handlers |
| `WebService` | namespace, packages, operations, parameters, directions, return types |
| `ScheduledJob` | method, use, predefined и restart properties |
| `CalculationRegister` | `Schedule`/`ScheduleValue`/`ScheduleDate` у правильного владельца |
| `Document` | attributes, tabular sections, forms, templates, commands |
| четыре регистра | dimensions, resources, attributes и применимые properties |
| `Enum` | значения и доказанные свойства значения |

Fixtures берутся из официального дампа платформы или уже подтверждённого
platform corpus. Если fixture расходится с действующей форматной
спецификацией, исправляется спецификация, а не fixture.

## Размещение реализации

Планируемая декомпозиция не возвращает логику в application layer:

- `crates/unica-coder/src/domain/metadata/info.rs` — публичная read-model,
  `MetaInfoDetails` и три состояния значений;
- `crates/unica-coder/src/domain/metadata/info_properties.rs` — read-side
  ключи и значения корневых свойств без разрешений мутации;
- `crates/unica-coder/src/domain/metadata/observed_types.rs` — read-side типы и
  явное сужение к writer-типу;
- `crates/unica-coder/src/infrastructure/native_operations/meta/info_projection.rs`
  — профиль 23 видов и чистая проекция captured XML;
- `crates/unica-coder/src/infrastructure/native_operations/meta/info.rs` —
  логическое разрешение, захват доказательств, validation resources и
  координация;
- `crates/unica-coder/src/application/meta_info_surface_tests.rs` — публичные
  regression и tri-state assertions;
- `tests/fixtures/platform_8_3_27/meta_info/` — manifest и platform fixtures.

`application/metadata.rs` не получает XML-знание и не становится вторым
реестром полей.

## Порядок реализации

1. Добавить failing regressions для оставшихся потерь на текущем `main` и
   убедиться, что каждый падает из-за отсутствующего typed-поля.
2. Ввести `ObservedMetadataType` и перевести существующие типы элементов на
   read-side parser без расширения writer-а.
3. Отделить `MetaInfoPropertyProfile` от `METADATA_PROPERTY_SPECS`, сохранив
   wire-форму существующих ключей и добавив доказанные read-only свойства.
4. Ввести закрытый `MetaInfoDetails` и мигрировать Constant/DefinedType,
   ScheduledJob и CalculationRegister.
5. Добавить HTTPService и WebService с QName expansion и вложенными
   коллекциями.
6. Добавить manifest всех 23 видов и coverage guard; каждый обнаруженный им
   пробел сначала закреплять отдельным красным тестом.
7. Синхронизировать skill, acceptance, tool-surface review, ADR-0041 и новый
   `INV-MCP-META-INFO-COVERAGE` одним implementation PR.
8. Проверить JSON-RPC: typed `structuredContent`, отсутствие `stdout`, raw XML и
   физических путей; после этого перевести ADR-0041 в `accepted` и закрыть
   #293/#274 фактическими ссылками на проверки.

## Проверка design PR

Design PR меняет только проектную записку, proposed ADR и индекс решений. Его
достаточно проверяют:

```sh
python3.12 -m unittest tests.ci.test_design_documents
python3.12 -m unittest tests.ci.test_architecture_registry
python3.12 scripts/ci/check-architecture-sync.py --base origin/main
git diff --check
```

Полный baseline `cargo test --workspace -- --test-threads=1` уже выполнен на
исходном `b0205624`: 2345 unit tests прошли, 2 ignored; все integration и doc
tests завершились без ошибок.
