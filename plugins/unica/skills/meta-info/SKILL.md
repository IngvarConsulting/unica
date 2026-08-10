---
name: meta-info
description: Прочитать типизированную локальную структуру и validation объекта метаданных 1С, при необходимости дополнив её списками использования из дерева исходников.
argument-hint: <sourceSet> <metadataPath> [sections] [limit]
allowed-tools:
  - Read
  - Glob
---

# /meta-info — структура и проверка объекта метаданных

## MCP routing

- Preferred path: use MCP `unica` tool `unica.meta.info`.
- Выбирайте объект логически через `sourceSet + metadataPath`; расположение XML
  внутри выгрузки остаётся внутренней деталью `unica`.
- Читайте локальную структуру и `data.validation` из одного результата. Отдельный
  публичный вызов проверки не нужен.
- Инструмент читает только дерево исходников и не обращается к индексу кода ни
  при каких аргументах. Без `sections` он ограничивается самим объектом; чтобы
  добавить списки использования, перечислите нужные `sections`, а `limit`
  (`1..=50`) ограничивает только `predefinedItems`.
- Успешный и предметно неуспешный `tools/call` возвращает `structuredContent`;
  `isError == !structuredContent.ok`. Читайте локальную структуру, validation и
  доступные частичные данные из `structuredContent.data`; `content[0].text` не
  является вторым контрактом результата.
- Не вызывайте внутренние MCP/CLI-адаптеры и skill-local scripts.
- `sourceSet` — это имя набора исходников из `v8project.yaml`, а не
  константа. Получите его через `unica.project.map`; `"main"` в примерах
  ниже — иллюстрация, а не значение по умолчанию.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "Catalog.Валюты"
    }
  }
}
```

## Ответ

`data` всегда начинается с локально прочитанной структуры объекта: канонического
`metadataPath`, вида, имени, синонима, состояния поддержки, свойств именами
платформы, владельцев, реквизитов, измерений, ресурсов, табличных частей, форм,
макетов и команд. Поле `validation` (`data.validation`) содержит `status` и
типизированные `diagnostics`; та же внутренняя проверка выполняется перед каждой
мутацией.

У структурного типа элемента есть `variants` и `mutationCapability` со
значением `editable` или `readOnly`. `uuid` — доказанно редактируемый вариант;
его входная форма — `{"kind": "uuid"}`. Синтаксически корректный, но ещё не
моделируемый платформенный QName оставляет `type` отсутствующим, отмечает только
этот элемент как `incomplete` и даёт warning, не превращая весь вызов в ошибку.
Это не разрешает передавать такой QName в `meta.add` или `meta.edit`.

Формы и макеты наблюдаются по ссылке владельца и отдельному XML-дескриптору,
команды — по встроенному дескриптору владельца без выдуманного
`Commands/<Name>.xml`. Страница HTML-макета проверяется как зарегистрированный
UTF-8-ресурс и не разбирается как XML; её `DOCTYPE`, HTML-сущности и исходные
байты не нормализуются.

`data.kind` всегда связан с обязательным `data.details`: это закрытый вариант
для каждого из 23 видов метаданных, а не необязательный универсальный словарь.
Полнотой этой пары владеют ADR-0047 и `INV-MCP-META-INFO-COVERAGE`.
Для видов без дополнительных фактов `details` равен `{}`. Constant и
DefinedType возвращают в нём наблюдаемый `type`, включая доказанно
редактируемый `UUID`;
ScheduledJob — логический адрес общего модуля и имя метода;
CalculationRegister — тройку `schedule`; HTTPService — `urlTemplates` с
методами; WebService — `xdtoPackages`, операции, параметры и XDTO QName в форме
`{namespace, localName}`. Возможность прочитать иной тип или свойство не
означает, что `unica.meta.add/edit` разрешит его записать.

Дополнительные владельцы не сваливаются в общий словарь:
`ChartOfCharacteristicTypes.details.type` хранит тип значения,
`ChartOfCalculationTypes.details.baseCalculationTypes` — логические адреса
базовых видов расчёта, `DocumentJournal.details.registeredDocuments` — адреса
зарегистрированных документов. Корневые декларации сериализуются отдельно как
`standardAttributes`, `standardTabularSections` и `characteristics`; их
прикладные свойства имеют закрытые значения `text`, `boolean`, `localizedString`,
`typed`, `nil` или `empty`. `localizedString.values` сохраняет пары
`{language, content}`. Только ранее опубликованный `properties.Synonym` остаётся
плоской строкой ради wire-совместимости; новые локализованные свойства языки не
теряют.

Для применимой вложенной коллекции доказанно пустое значение равно `[]`.
Доказанно отсутствующее применимое значение равно `null` без ошибки.
Недоказанное или повреждённое значение равно `null` и сопровождается диагностикой
с путём публичного поля, например `details.urlTemplates[0].methods[0].handler`;
частичный массив за полный не выдаётся.

Новые коллекции `recalculations`, `accountingFlags`,
`extDimensionAccountingFlags` и `addressingAttributes` находятся в
`data.collections`. Неприменимая к виду коллекция отсутствует; применимая без
контейнера равна `null`; полностью прочитанный пустой контейнер равен `[]`.
Элементы accounting/addressing дополнительно сохраняют известные стандартные
свойства в `properties`; неизвестный или повреждённый вложенный узел обнуляет
всю коллекцию с диагностикой, но неизвестный корректный QName оставляет один
`incomplete`-элемент и warning. `relations.dataLockFields` использует ту же
all-or-none семантику и возвращает типизированные field-цели.

У `EventSubscription` поле `data.relations.source` содержит массив того же
закрытого размеченного объединения, которое принимают `unica.meta.add` и
`unica.meta.edit`: `object`, `manager`, `recordSet` и `definedType` возвращаются с
логическим `metadataPath`, `family` — с `sourceClass`. Для менеджера константы
`sourceClass` различает `constantManager` и `constantValueManager`. Это обратное
чтение логически допустимого `Properties/Source`, а не строка XML-типа. Примитив,
ссылка или другой форматный тип не выдаётся как допустимая цель: чтение содержит
диагностику. Позиция элемента массива не входит в идентичность источника.

`data.functionalSubsystems` и `data.interfaceSubsystems` — только членства
текущего объекта соответственно в функциональных и интерфейсных подсистемах.
Запись `Content` может ссылаться на объект логическим адресом метаданных или
UUID его корневого дескриптора. После полного доказательства топологии отсутствие
членств сериализуется как `[]`. Если зарегистрированная топология повреждена,
недоступна или её обработка отменена, оба поля отсутствуют, а диагностика
содержит `provider_unavailable`; это состояние не подменяется пустыми массивами.

Явно запрошенные секции читаются из дерева исходников, а не из индекса.
`data.usage` содержит `roles`, `subscriptions` и `functionalOptions` обычными
полными массивами: они прочитаны из того же снимка, что и сам объект, и
разойтись с ним не могут, поэтому никаких признаков давности у них нет.
Предопределённые элементы лежат отдельно в `data.predefinedItems` вместе с
`total`, `returned` и `truncated`, потому что это содержимое самого объекта.
`items` возвращается плоско в документном порядке: каждый элемент содержит
UUID в `id`, поддержанные typed-поля своего владельца и `parentId` для
вложенного элемента. `Catalog`, `ChartOfAccounts`,
`ChartOfCharacteristicTypes` и `ChartOfCalculationTypes` имеют разные закрытые
наборы полей; структурные `type`, `accountingFlags` и `extDimensionTypes`
читайте как объекты из ответа, не восстанавливайте из них строковый DSL.
Подписка может достигать объекта через `DefinedType`; такое совпадение входит в
ответ и помечено полем `via`. Без `sections` или с `sections: []` не читается
ничего сверх самого объекта.

Адрес принимает русские и английские псевдонимы вида, а в
`data.metadataPath` возвращает каноническую английскую форму. Если известен
только путь файла, сначала используйте `unica.source.locate`; если известно имя
— `unica.source.resolve`. Адрес модуля (`Catalog.X.ObjectModule`) здесь не
поддерживается: код читают `unica.code.*`.

«Представление типа», «Представление объекта» и представления списка ссылочного
объекта находятся в `properties` под платформенными именами
`ObjectPresentation`, `ExtendedObjectPresentation`, `ListPresentation` и
`ExtendedListPresentation`.

Раздел «Поддержка» читается из `Ext/ParentConfigurations.bin`. Объект на замке
изменяйте через CFE/release-support flow, не через raw support metadata.

Соглашения по именам, синонимам и представлениям находятся в
[общей ссылке](../../references/platform/metadata-conventions.md); перечни видов
и свойств не дублируются здесь, потому что их публикует схема операции.

## Примеры

### Документ: локальная структура и validation

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "Document.АвансовыйОтчет"
    }
  }
}
```

### Документ и явно запрошенные списки использования

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "Document.АвансовыйОтчет",
      "sections": ["roles", "subscriptions", "functionalOptions"],
      "limit": 20
    }
  }
}
```

### Предопределённые элементы в документном порядке

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "Catalog.Валюты",
      "sections": ["predefinedItems"],
      "limit": 20
    }
  }
}
```

Сначала проверяйте `total`, `returned` и `truncated`, затем обходите `items`.
`parentId: null` означает корневой элемент; UUID родителя связывает вложенный
элемент без раскрытия физической структуры `Predefined.xml`.

### HTTP-сервис

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "HTTPService.ExternalAPI"
    }
  }
}
```

### Веб-сервис

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "WebService.EnterpriseDataUpload_1_0_1_1"
    }
  }
}
```

### Определяемый тип

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "DefinedType.GLN"
    }
  }
}
```

### Подписка на событие: типизированные источники

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "EventSubscription.ОбработкаИзменений"
    }
  }
}
```
