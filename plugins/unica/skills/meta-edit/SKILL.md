---
name: meta-edit
description: Типизированно изменить существующий объект метаданных 1С по логическому адресу одной транзакцией.
argument-hint: <at> <operations>
allowed-tools:
  - Read
  - Glob
---

# /unica:meta-edit — структурное редактирование метаданных

## MCP routing

- Preferred path: use MCP `unica` tool `unica.apply` с операциями изменения
  объекта метаданных.
- Объект и его коллекции называет адрес: `main:Catalog.Валюты` — сам объект,
  `main:Catalog.Валюты.Attribute` — его реквизиты. Физический путь остаётся
  внутренней деталью; наружу его отдаёт только аварийный `unica.resolve`.
- Имя набора даёт `unica.view {}`, адрес по имени — `unica.search {corpus:
  "names"}`. `"main"` ниже только пример.
- Сначала вызови `unica.apply` с `at` и `ops`: это план без записи.
  Когда пользователь поручил внести эту правку, вызови `unica.apply` только
  с `executionToken` из `data.executionToken` успешного плана.
- Операции одного вызова упорядочены: каждая видит результат предыдущих, и
  публикуются они одной транзакцией — либо все, либо ни одной.
- Читай из ответа `changed`, `effects` по порядку операций и диагностики.
  Ответ описывает правку семантическими `effects`, а не возвращает
  полный XML объекта.
- Проверка поддержки поставщика выполняется внутри `unica`. Для закрытого
  объекта используй путь расширения, а не прямую правку служебных файлов.
- Какие операции исполнимы на узле, объявляет сам узел: словарь `can`
  отдаёт `unica.view` с `filter: {"sections": ["can"]}`, а подробный
  контракт одной операции — с `filter: {"sections": ["can"], "can": {"op":
  "<операция>"}}`. Отказ `fixCall` несёт `next` к узлу, где операция
  исполнима: выполни его, а не подбирай аргументы.

## Операции

| Предмет | Операции |
|---|---|
| Свойства объекта | `props.set` |
| Реквизиты | `attribute.add`, `attribute.set`, `attribute.remove` |
| Табличные части | `tabularSection.add`, `tabularSection.set`, `tabularSection.remove` |
| Измерения и ресурсы | `dimension.*`, `resource.*` |
| Значения перечисления | `enumValue.add`, `enumValue.set`, `enumValue.remove` |
| Графы журнала | `column.add`, `column.set`, `column.remove` |
| Макеты и команды | `template.*`, `command.*` |
| Предопределённые элементы | `predefinedItem.add`, `predefinedItem.set`, `predefinedItem.remove` |
| Ссылки наружу | `relation.add`, `relation.replace`, `relation.remove` |
| Встроенная справка | `help.create` |
| Объект целиком | `object.remove` |

Допустимые свойства, коллекции, виды типов и связей берутся из контракта
операции (`argsSchema` подробного `can`). Общие прикладные правила — в
[соглашениях по метаданным](../../references/platform/metadata-conventions.md).

**Член создаётся на владельце, а изменяется и удаляется по своему адресу.**
`*.add` адресует того, кому член принадлежит, и несёт новые элементы в
`items`: `attribute.add` с `at: "main:Catalog.Валюты"` добавляет реквизит
шапки, а с `at: "main:Catalog.Валюты.TabularSection.Представления"` —
реквизит этой табличной части. `*.set` и `*.remove` адресуют сам член:
`main:Catalog.Валюты.TabularSection.Представления`,
`main:Catalog.Валюты.Attribute.Код`,
`main:Catalog.Валюты.TabularSection.Представления.Attribute.КодЯзыка`,
`main:Enum.ВидыОпераций.EnumValue.Приход`, `…Dimension.<Имя>`,
`…Resource.<Имя>`, `…Column.<Имя>`, `…Command.<Имя>`, `…Template.<Имя>`,
`…PredefinedItem.<Имя>`. Имени члена в аргументах нет: `set` несёт в `values`
только изменяемые поля, переименование — `newName`; `remove` аргументов не
принимает. Прежний вызов с `scope.tabularSection` или `values.name` получает
отказ, чей `next` называет правильный адрес.

Регистрация макета — `template.add` с `name` и необязательным `templateType`
из закрытого набора `HTMLDocument`, `TextDocument`, `SpreadsheetDocument` (по
умолчанию), `BinaryData`, `DataCompositionSchema`; снятие регистрации —
`template.remove`. Наполнение содержимого остаётся у предметных инструментов
своего вида.

`help.create` создаёт `Ext/Help.xml` и `Ext/Help/<lang>.html` владельца и
включает `IncludeHelpInContents` его формам; повтор — отказ, операция
создаёт только раз.

### Предопределённые элементы

Коллекция доступна только для `Catalog`, `ChartOfAccounts`,
`ChartOfCharacteristicTypes` и `ChartOfCalculationTypes`. Общие поля элемента:
`id`, `name`, `code`, `description`. Дополнительные поля закрыты видом
владельца:

- `Catalog`: `isFolder`;
- `ChartOfCharacteristicTypes`: `isFolder` и структурный `type` из
  опубликованной схемы, не строка и не QName;
- `ChartOfAccounts`: `accountType`, `offBalance`, `order`, `accountingFlags`,
  `extDimensionTypes`;
- `ChartOfCalculationTypes`: `actionPeriodIsBase`.

Для плана счетов `accountType` допускает `Active`, `Passive`,
`ActivePassive`; `accountingFlags` — закрытый объект `имя: boolean`;
`extDimensionTypes` — массив объектов с `name` и необязательными `turnover`,
`accountingFlags`. Явно переданные пустые `{}` и `[]` очищают соответственно
поддержанные `Flag` и `ExtDimensionType`; отсутствие поля сохраняет прежнее
значение.

`predefinedItem.add` на владельце создаёт только корневой элемент; `id` и
`name` задаются здесь. Совпадающий UUID даёт no-op только при эквивалентном
образе, иначе `already_exists`. `predefinedItem.set` адресует элемент
`…PredefinedItem.<Имя>` и принимает в `values` изменяемые поля,
переименование — `newName`; `predefinedItem.remove` — тот же адрес без
аргументов. Имя находится на любой глубине иерархии и уникально в объекте:
добавление или переименование на занятое имя отвергается. Удаление родителя
удаляет всё его поддерево. Неуказанные поля и неизвестные XML-узлы
сохраняются.

Элемент читается `unica.view` по тому же адресу: `props.id` — его UUID,
остальные поля — в `props`. Список элементов отдаёт ветвь
`…PredefinedItem` владельца.

### Ссылки наружу

Владельцы справочника и движения документа меняются через `relation.replace`
со связью `owners` или `registerRecords`. Источник подписки заменяется той же
операцией со связью `source`; `targets: []` очищает список, но пустой
итоговый `Source` не является допустимым состоянием подписки, поэтому такой
запрос годится только как часть изменения, чей итог снова непуст.

Цели — закрытое логическое объединение `object`, `manager`, `recordSet`,
`definedType` и `family`. `targets` — wire-массив набора:
порядок его членов семантически незначим, и перестановка тех же целей
является exact-byte no-op. При изменении Unica выпускает цели в
детерминированном порядке.

В расширении источник из основной конфигурации записан `v8:TypeId`; ветвь
`Relation` показывает его элементом `{"relation":"source","kind":"typeId",
"typeId":…,"readOnly":true}`. Задать его через `targets` нельзя, а
`relation.replace` меняет только логические цели и сохраняет такие элементы
как есть; итог из одних `v8:TypeId` платформа не загружает, поэтому нужна хотя
бы одна цель своего набора.

Конфигурационные варианты передают логический `metadataPath`; Unica проверяет
регистрацию, дескриптор и совпадающий `GeneratedType` под тем же владельцем,
что и подписка. `DefinedType` разворачивается рекурсивно, а примитивы, ссылки
и `valueStorage` логическими источниками событий не являются.

## Границы

Не переноси поля снятого Meta JSON DSL по сходству имён:

- один вызов изменяет один объект; несколько объектов — несколько вызовов;
- вложенные шаблоны URL и методы HTTP-сервиса, операции и параметры
  веб-сервиса, расписания, реквизиты адресации и учётные признаки читаются, но
  типизированного писателя не имеют;
- shorthand-флаги `index`, `indexAdditional`, `nonneg`, `master`,
  `mainFilter`, `denyIncomplete`, `useInTotals` нельзя упаковывать в строку;
- не подставляй составное значение в `props.set` строкой и не заводи временный
  файл определения.

Тип значения константы задаётся через `props.set` по адресу константы:
`values: {"Type": {"variants": [{"kind": "boolean"}]}}`. При создании помести
эту операцию сразу после `object.create` в том же вызове. Для ссылки используй
вариант `{"kind": "reference", "metadataPath": "Catalog.Номенклатура"}`;
строка QName не принимается. Состав типов и квалификаторы те же, что у
реквизитов. `view` возвращает тип в `props.type` как компактную JSON-строку.

Если схема не представляет сценарий, остановись и скажи прямо, что оставшийся
шаг выполняется в Конфигураторе.

Скалярные свойства используй только под именем платформы в PascalCase и со
значением, опубликованным текущей схемой. Свойство наблюдения
`mutationCapability` во вход писателя не передаётся, и неизвестный QName
нельзя копировать из XML в аргументы. Вариант `{"kind":"typeId"}` — тип
основной конфигурации в расширении — только читается (`readOnly`); правка
других свойств реквизита его сохраняет.

## MCP examples

### Изменить свойства и реквизит

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Catalog.Валюты",
      "ops": [
        {
          "op": "props.set",
          "args": {
            "at": "main:Catalog.Валюты",
            "values": {"Comment": "Справочник валют"}
          }
        },
        {
          "op": "attribute.set",
          "args": {
            "at": "main:Catalog.Валюты.Attribute.Код",
            "values": {"required": true}
          }
        }
      ]
    }
  }
}
```

### Правка реквизита табличной части и удаление

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Catalog.Валюты",
      "ops": [
        {
          "op": "attribute.set",
          "args": {
            "at": "main:Catalog.Валюты.TabularSection.Курсы.Attribute.Курс",
            "values": {"required": true}
          }
        },
        {
          "op": "attribute.remove",
          "args": {
            "at": "main:Catalog.Валюты.TabularSection.Курсы.Attribute.Устарело"
          }
        }
      ]
    }
  }
}
```

### Реквизит табличной части и её переименование

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Catalog.Валюты.TabularSection.Представления",
      "ops": [
        {
          "op": "attribute.add",
          "args": {
            "at": "main:Catalog.Валюты.TabularSection.Представления",
            "items": [
              {
                "name": "Сокращение",
                "type": {"variants": [{"kind": "string", "length": 10, "allowedLength": "variable"}]}
              }
            ]
          }
        },
        {
          "op": "tabularSection.set",
          "args": {
            "at": "main:Catalog.Валюты.TabularSection.Представления",
            "values": {"newName": "ЯзыковыеПредставления"}
          }
        }
      ]
    }
  }
}
```

### Предопределённые элементы

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Catalog.Валюты",
      "ops": [
        {
          "op": "predefinedItem.add",
          "args": {
            "at": "main:Catalog.Валюты",
            "items": [
              {
                "id": "0f5f4f2a-1a2b-4c3d-8e4f-5a6b7c8d9e0f",
                "name": "Рубль",
                "code": "643",
                "description": "Российский рубль"
              }
            ]
          }
        },
        {
          "op": "predefinedItem.set",
          "args": {
            "at": "main:Catalog.Валюты.PredefinedItem.Рубль",
            "values": {"description": "Рубль РФ"}
          }
        },
        {
          "op": "predefinedItem.remove",
          "args": {
            "at": "main:Catalog.Валюты.PredefinedItem.Евро"
          }
        }
      ]
    }
  }
}
```

### Заменить источник подписки

Сначала получи план этого изменения:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:EventSubscription.ПриЗаписи",
      "ops": [
        {
          "op": "relation.replace",
          "args": {
            "at": "main:EventSubscription.ПриЗаписи",
            "values": {
              "relation": "source",
              "targets": [
                {
                  "kind": "recordSet",
                  "metadataPath": "InformationRegister.ИсторияИзменений"
                }
              ]
            }
          }
        }
      ]
    }
  }
}
```

Для исполнения передай `data.executionToken` из этого успешного плана:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "executionToken": "<data.executionToken из успешного плана>"
    }
  }
}
```
