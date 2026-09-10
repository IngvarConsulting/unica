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
- Всегда сначала `dryRun: true`. Применяй `dryRun: false` только когда
  пользователь явно попросил внести именно эту правку, и только с `ifRev` из
  предпросмотра.
- Операции одного вызова упорядочены: каждая видит результат предыдущих, и
  публикуются они одной транзакцией — либо все, либо ни одной.
- Читай из ответа `changed`, `effects` по порядку операций и диагностики.
  Ответ описывает правку семантическими `effects`, а не возвращает
  полный XML объекта.
- Проверка поддержки поставщика выполняется внутри `unica`. Для закрытого
  объекта используй путь расширения, а не прямую правку служебных файлов.
- Что операция допустима на этом узле, объявляет сам узел: `can` в ответе
  `unica.view {at}`.

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

Допустимые свойства, коллекции, виды типов и связей берутся из опубликованной
схемы операции. Общие прикладные правила — в
[соглашениях по метаданным](../../references/platform/metadata-conventions.md).

**Адрес операции — сам объект**, а элемент называют `items` или `values`:
`attribute.add` с `at: "main:Catalog.Валюты"` и `items`, `tabularSection.set`
с `values`, несущим `name`. Исключение одно и оно полезное: `attribute.set` и
`attribute.remove` адресуют реквизит листом —
`main:Catalog.Валюты.Attribute.Код` — и через табличную часть:
`main:Catalog.Валюты.TabularSection.Курсы.Attribute.Курс`. Отдельного поля
области видимости при этом не нужно: область называет адрес.

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

`predefinedItem.add` создаёт только корневой элемент. Совпадающий UUID даёт
no-op только при эквивалентном образе, иначе `already_exists`.
`predefinedItem.set` принимает `values` с изменяемыми полями,
`predefinedItem.remove` — `values` с одним `id`. Обе находят UUID на любой
глубине; удаление родителя удаляет всё его поддерево. Неуказанные поля и неизвестные
XML-узлы сохраняются.

**Читателя у предопределённых элементов на канонической поверхности пока
нет.** Писать их можно, а прочитать обратно этим же путём — нет; не выдавай за
них соседний факт.

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

Если схема не представляет сценарий, остановись и скажи прямо, что оставшийся
шаг выполняется в Конфигураторе.

Скалярные свойства используй только под именем платформы в PascalCase и со
значением, опубликованным текущей схемой. Свойство наблюдения
`mutationCapability` во вход писателя не передаётся, и неизвестный QName
нельзя копировать из XML в аргументы.

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
      ],
      "dryRun": true
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
      ],
      "dryRun": true
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
          "op": "predefinedItem.remove",
          "args": {
            "at": "main:Catalog.Валюты",
            "values": {"id": "1a2b3c4d-5e6f-4708-8910-1112131415ff"}
          }
        }
      ],
      "dryRun": true
    }
  }
}
```

### Заменить источник подписки

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
      ],
      "dryRun": false,
      "ifRev": "<rev из предпросмотра>"
    }
  }
}
```
