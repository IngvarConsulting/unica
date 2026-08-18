---
name: meta-edit
description: Типизированное атомарное редактирование существующего объекта метаданных 1С по логическому адресу.
argument-hint: <sourceSet> <metadataPath> <operations>
allowed-tools:
  - Read
  - Glob
---

# /unica:meta-edit — структурное редактирование метаданных

## MCP routing

- Preferred path: use MCP `unica` tool `unica.meta.edit`.
- Выбирайте объект только через `sourceSet + metadataPath`.
- Передавайте непустой упорядоченный массив `operations`; все элементы одного
  вызова видят результат предыдущих и публикуются одной транзакцией.
- Вызов по умолчанию строит preview. Передавайте `dryRun: false` только когда
  пользователь явно попросил применить изменение.
- Успешный и предметно неуспешный `tools/call` возвращает `structuredContent`;
  `isError == !structuredContent.ok`. Проверяйте
  `structuredContent.data.validation` и вложенные диагностики;
  `content[0].text` не является вторым контрактом результата.
- Preview возвращает нормализованные семантические
  `structuredContent.data.effects` по `operationIndex`, а не полный XML.
- Vendor support guard выполняется внутри `unica`. Для закрытого объекта
  используйте CFE/release-support flow, а не прямую правку служебных файлов.
- `sourceSet` — это имя набора исходников из `v8project.yaml`, а не
  константа. Получите его через `unica.project.map`; `"main"` в примерах
  ниже — иллюстрация, а не значение по умолчанию.

Поддерживаются шесть значений `op`: `setProperties`, `add`, `update`, `remove`,
`editRelations` и `addHelp`. Для коллекционных операций задаются `collection` и
структурные `elements` либо `names`; связи задаются через `relation`, `mode` и
`targets`. Допустимые свойства, коллекции, виды типов и связей берите из
опубликованной схемы операции. Общие прикладные правила находятся в
[соглашениях по метаданным](../../references/platform/metadata-conventions.md).

Регистрация макета — операция `add` по коллекции `templates`: элемент несёт
`name` и необязательный `templateType` из закрытого набора `HTMLDocument`,
`TextDocument`, `SpreadsheetDocument` (по умолчанию), `BinaryData`,
`DataCompositionSchema`; снятие регистрации — `remove` по той же коллекции.
Наполнение содержимого остаётся у предметных инструментов своего вида.
Встроенная справка — операция `{"op": "addHelp", "lang"?}` (ADR-0072):
create-only создаёт `Ext/Help.xml` и `Ext/Help/<lang>.html` владельца и
включает `IncludeHelpInContents` его формам; повтор — отказ. Прежние
`unica.template.add`, `unica.template.remove` и `unica.help.add` сняты и
отвечают `unknown unica tool`.

Коллекция `predefinedItems` доступна только для `Catalog`,
`ChartOfAccounts`, `ChartOfCharacteristicTypes` и
`ChartOfCalculationTypes`. Для неё `add` и `update` принимают typed
`elements`, а `remove` — массив `ids`; отдельная операция не нужна.
Общие поля элемента: `id`, `name`, `code`, `description`. Дополнительные поля
закрыты видом владельца:

- `Catalog`: `isFolder`;
- `ChartOfCharacteristicTypes`: `isFolder` и структурный `type` из
  опубликованной схемы, не строка и не QName;
- `ChartOfAccounts`: `accountType`, `offBalance`, `order`,
  `accountingFlags`, `extDimensionTypes`;
- `ChartOfCalculationTypes`: `actionPeriodIsBase`.

`type` использует общий структурный `metadataType`. Для плана счетов
`accountType` допускает `Active`, `Passive`, `ActivePassive`;
`accountingFlags` — закрытый объект `имя: boolean`; `extDimensionTypes` —
массив объектов с `name` и необязательными `turnover`, `accountingFlags`.
Явно переданные пустые `{}` и `[]` очищают соответственно поддержанные
`Flag` и `ExtDimensionType`; отсутствие поля сохраняет прежнее значение.

`add` создаёт только корневой элемент. Совпадающий UUID даёт no-op только при
эквивалентном образе, иначе `already_exists`. `update` и `remove` находят UUID
на любой глубине; удаление родителя удаляет всё его поддерево. Неуказанные поля
и неизвестные XML-узлы сохраняются.

Не переносите поля снятого Meta JSON DSL по сходству имён. В частности:

- один вызов изменяет один объект; batch нескольких объектов разбивается на
  отдельные `unica.meta.add`, а `meta.add.operations` атомарен только вместе с
  создаваемым объектом;
- вложенные URL-шаблоны и методы HTTP-сервиса, операции и параметры
  Web-сервиса, расписания, реквизиты адресации, учётные признаки и ссылки без
  опубликованного relation/property-варианта не имеют типизированного writer;
- shorthand-флаги `index`, `indexAdditional`, `nonneg`, `master`,
  `mainFilter`, `denyIncomplete`, `useInTotals` нельзя упаковывать в строку;
- не подставляйте compound-значение в `setProperties` как строку и не создавайте
  временный `DefinitionFile`. Если схема не представляет сценарий, остановитесь
  и явно сообщите, что оставшийся шаг выполняется в Designer.

Тип уникального идентификатора задаётся закрытым вариантом
`{"kind": "uuid"}`. Поле `mutationCapability: "readOnly"` из результата
`unica.meta.info` является свойством наблюдения и никогда не передаётся во вход
writer; неизвестный QName также нельзя копировать из XML в аргументы.

Доказанные переходы для прежних коллекций: значения перечисления добавляются
как `collection: "enumValues"`, владельцы справочника и движения документа
меняются через `editRelations` с `relation: "owners"` или
`relation: "registerRecords"`. Источник подписки заменяется той же операцией с
`relation: "source"` и только `mode: "replace"`; `targets: []` очищает список.
Но пустой `Source` не является допустимым итоговым состоянием подписки, поэтому
такой запрос можно использовать только как часть изменения, чей итог снова
непуст; обычная мутация с `targets: []` отклоняется. Цели — закрытое логическое
объединение `object`, `manager`, `recordSet`, `definedType` и `family`.
`targets` — wire-массив набора: порядок его членов семантически незначим, и
перестановка тех же целей является exact-byte no-op. При изменении Unica
выпускает цели в детерминированном порядке.
Конфигурационные варианты передают логический `metadataPath`; Unica проверяет
регистрацию, дескриптор и совпадающий `GeneratedType` под тем же владельцем,
что и подписка. `DefinedType` разворачивается рекурсивно, а примитивы, ссылки и
`valueStorage` не являются логическими источниками событий.
Скалярные свойства используйте только под PascalCase-именем и с enum-значением,
опубликованным текущей схемой.

### Изменить свойства

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.edit",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "Catalog.Контрагенты",
      "operations": [
        {"op": "setProperties", "values": {"Comment": "Проверено"}}
      ],
      "dryRun": true
    }
  }
}
```

### Добавить типизированный реквизит

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.edit",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "Catalog.Контрагенты",
      "operations": [
        {
          "op": "add",
          "collection": "attributes",
          "elements": [
            {
              "name": "Комментарий",
              "type": {
                "variants": [
                  {"kind": "string", "length": 200, "allowedLength": "variable"}
                ]
              }
            }
          ]
        }
      ],
      "dryRun": true
    }
  }
}
```

### Изменить и удалить реквизиты табличной части

`scope.tabularSection` ограничивает обе операции реквизитами существующей
табличной части, а не корневыми реквизитами документа.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.edit",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "Document.ЗаказПокупателя",
      "operations": [
        {
          "op": "update",
          "collection": "attributes",
          "scope": {"tabularSection": "Товары"},
          "elements": [
            {
              "name": "Количество",
              "synonym": "Количество товара",
              "required": true
            }
          ]
        },
        {
          "op": "remove",
          "collection": "attributes",
          "scope": {"tabularSection": "Товары"},
          "names": ["УстаревшийРеквизит"]
        }
      ],
      "dryRun": true
    }
  }
}
```

### Изменить типизированную связь

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.edit",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "Document.ЗаказПокупателя",
      "operations": [
        {
          "op": "editRelations",
          "relation": "basedOn",
          "mode": "replace",
          "targets": [
            {"metadataPath": "Document.СчетПокупателю"}
          ]
        }
      ],
      "dryRun": true
    }
  }
}
```

### Атомарно заменить источник, событие и обработчик подписки

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.edit",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "EventSubscription.ОбработкаИзменений",
      "operations": [
        {
          "op": "setProperties",
          "values": {
            "Event": "BeforeWrite",
            "Handler": "CommonModule.ОбработчикиПодписок.ПередЗаписьюИстории"
          }
        },
        {
          "op": "editRelations",
          "relation": "source",
          "mode": "replace",
          "targets": [
            {
              "kind": "recordSet",
              "metadataPath": "InformationRegister.ИсторияИзменений"
            }
          ]
        }
      ],
      "dryRun": true
    }
  }
}
```

Для набора записей событие `BeforeWrite` передаёт `Cancel` и `Replacing`, поэтому
обработчик выше должен быть экспортной процедурой с тремя параметрами
`(Source, Cancel, Replacing)` в общем модуле с `Global=false`, `Server=true`.
Unica проверяет итоговый post-image, поэтому порядок этих двух операций не
меняет результат. Конкретный менеджер константы требует дополнительный
`"sourceClass": "constantManager"` либо `"constantValueManager"`; у остальных
`manager` это поле запрещено.

### Добавить, изменить и удалить предопределённые элементы

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.edit",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "Catalog.Валюты",
      "operations": [
        {
          "op": "add",
          "collection": "predefinedItems",
          "elements": [
            {
              "id": "c7d2e6fc-3824-4b56-b4be-ae6be4944c0e",
              "name": "ОсновнаяВалюта",
              "code": "643",
              "description": "Рубль",
              "isFolder": false
            }
          ]
        },
        {
          "op": "update",
          "collection": "predefinedItems",
          "elements": [
            {
              "id": "c7d2e6fc-3824-4b56-b4be-ae6be4944c0e",
              "description": "Российский рубль"
            }
          ]
        },
        {
          "op": "remove",
          "collection": "predefinedItems",
          "ids": ["8ed9f480-f17d-4dc8-95c4-b7887e2f918a"]
        }
      ],
      "dryRun": true
    }
  }
}
```
