---
name: meta-edit
description: Точечное редактирование объекта метаданных 1С. Используй когда нужно добавить, удалить или изменить реквизиты, табличные части, измерения, ресурсы или свойства существующего объекта конфигурации
argument-hint: <ObjectPath> -Operation <op> -Value "<val>" | -DefinitionFile <json> [-NoValidate]
allowed-tools:
  - Bash
  - Read
  - Write
  - Glob
---

# /meta-edit — точечное редактирование метаданных 1С

## MCP routing

- Preferred path: use MCP `unica` tool `unica.meta.edit`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.meta.edit`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.
- Vendor support guard runs inside `unica`; if it blocks a locked/read-only supported object, prefer CFE/release-support or an explicit support-state change plan instead of editing raw support metadata.

Атомарные операции модификации существующих XML объектов метаданных.

## MCP вызов

### Inline mode: простая операция

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.edit",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "<path>",
      "Operation": "modify-property",
      "Value": "DescriptionLength=150",
      "dryRun": false
    }
  }
}
```

### JSON mode: файл операций

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.edit",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "<path>",
      "DefinitionFile": "<json>",
      "dryRun": false
    }
  }
}
```

## Операции — сводная таблица

Batch через `;;` во всех операциях. Подробный синтаксис — в файлах по ссылкам.

### Дочерние элементы — [child-operations.md](child-operations.md)

| Операция | Формат Value | Пример |
|----------|-------------|--------|
| `add-attribute` | `Имя: Тип \| флаги` | `"Сумма: Число(15,2) \| req, index"` |
| `add-ts` | `ТЧ: Рекв1: Тип1, Рекв2: Тип2` | `"Товары: Ном: CatalogRef.Ном, Кол: Число(15,3)"` |
| `add-dimension` | `Имя: Тип \| флаги` | `"Организация: CatalogRef.Организации \| master"` |
| `add-resource` | `Имя: Тип` | `"Сумма: Число(15,2)"` |
| `add-enumValue` | `Имя` | `"Значение1 ;; Значение2"` |
| `add-column` | `Имя: Тип` | `"Тип: EnumRef.ТипыДокументов"` |
| `add-form` / `add-template` / `add-command` | `Имя` | `"ФормаЭлемента"` |
| `add-ts-attribute` | `ТЧ.Имя: Тип` | `"Товары.Скидка: Число(15,2)"` |
| `remove-*` | `Имя` | `"СтарыйРеквизит ;; ЕщёОдин"` |
| `remove-ts-attribute` | `ТЧ.Имя` | `"Товары.УстаревшийРекв"` |
| `modify-attribute` | `Имя: ключ=значение` | `"Статус: fillValue=Enum.Статусы.EnumValue.Новый"` |
| `modify-ts-attribute` | `ТЧ.Имя: ключ=значение` | `"Товары.Статус: fillValue=nil"` |
| `modify-ts` | `ТЧ: ключ=значение` | `"Товары: lineNumberLength=9"` |

Позиционная вставка: `"Склад: CatalogRef.Склады >> after Организация"`.

Ключ `lineNumberLength` (`line_number_length`, `line-number-length`) меняет
существующее свойство `LineNumberLength` табличной части. Допустимы целые значения
от 5 до 9 только для хранимых объектов, если эффективная версия совместимости
новее `Version8_3_26`: для `DontUse` берётся активная версия платформенного
профиля Unica, для явного `VersionX` — версия `X`. При эффективной версии не
новее `Version8_3_26` платформа фиксирует значение 5. Для `Report`,
`DataProcessor`, `ExternalReport` и `ExternalDataProcessor` длина номера строки
не ограничена, поэтому свойство неприменимо.

Ключ `fillValue` изменяет только уже существующее в XML свойство `FillValue`. Для
`modify-attribute` оно допустимо у хранимых объектов, но не у `DataProcessor` и
`Report`; для `modify-ts-attribute`, наоборот, — только у `DataProcessor` и `Report`.
Формат значения:

- `nil` или пустая строка — `<FillValue xsi:nil="true"/>`;
- `Enum.Имя.EnumValue.Значение` и `ВидОбъекта.Имя.EmptyRef`, где вид —
  `Catalog`, `Document`, `ExchangePlan`, `ChartOfAccounts`,
  `ChartOfCharacteristicTypes`, `ChartOfCalculationTypes`, `BusinessProcess` или
  `Task`, — `xsi:type="xr:DesignTimeRef"`;
- `true`/`false`, десятичное число и ISO date-time `YYYY-MM-DDTHH:MM:SS` —
  `xs:boolean`, `xs:decimal` и `xs:dateTime` соответственно;
- остальные значения — `xs:string`.

Если свойство или контекст недопустимы, операция завершается ошибкой без записи файла.
При успешной замене сохраняются исходные BOM, EOL, XML declaration, стиль self-closing
элементов и наличие завершающего перевода строки.

### Свойства объекта — [properties-reference.md](properties-reference.md)

| Операция | Формат Value | Пример |
|----------|-------------|--------|
| `modify-property` | `Ключ=Значение` | `"CodeLength=11 ;; DescriptionLength=150"` |
| `add-owner` | `MetaType.Name` | `"Catalog.Контрагенты ;; Catalog.Организации"` |
| `add-registerRecord` | `MetaType.Name` | `"AccumulationRegister.ОстаткиТоваров"` |
| `add-basedOn` | `MetaType.Name` | `"Document.ЗаказКлиента"` |
| `add-inputByString` | `Путь поля` | `"StandardAttribute.Description"` |
| `set-owners` / `set-registerRecords` / `set-basedOn` / `set-inputByString` | Замена всего списка | `"Catalog.Орг ;; Catalog.Контр"` |
| `remove-owner` / `remove-registerRecord` / ... | Удаление из списка | `"Catalog.Контрагенты"` |

### JSON DSL — [json-dsl.md](json-dsl.md)

Для комбинированных операций (add + remove + modify в одном файле), синонимы ключей/типов, таблица поддерживаемых объектов.

## Быстрые примеры

### Добавить реквизиты

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.edit",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "src/Catalogs/Контрагенты/Контрагенты.xml",
      "Operation": "add-attribute",
      "Value": "Комментарий: Строка(200) ;; Сумма: Число(15,2) | index",
      "dryRun": false
    }
  }
}
```

### Составной тип

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.edit",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "src/Catalogs/Контрагенты/Контрагенты.xml",
      "Operation": "add-attribute",
      "Value": "Значение: Строка + Число(15,2) + Дата + CatalogRef.Контрагенты",
      "dryRun": false
    }
  }
}
```

### Добавить табличную часть с реквизитами

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.edit",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "src/Documents/ЗаказПокупателя/ЗаказПокупателя.xml",
      "Operation": "add-ts",
      "Value": "Товары: Ном: CatalogRef.Ном | req, Кол: Число(15,3), Цена: Число(15,2)",
      "dryRun": false
    }
  }
}
```

### Удалить реквизит

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.edit",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "src/Catalogs/Контрагенты/Контрагенты.xml",
      "Operation": "remove-attribute",
      "Value": "УстаревшийРеквизит",
      "dryRun": false
    }
  }
}
```

### Переименовать и сменить тип

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.edit",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "src/Catalogs/Контрагенты/Контрагенты.xml",
      "Operation": "modify-attribute",
      "Value": "СтароеИмя: name=НовоеИмя, type=Строка(500)",
      "dryRun": false
    }
  }
}
```

### Изменить свойства объекта

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.edit",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "src/Catalogs/Контрагенты/Контрагенты.xml",
      "Operation": "modify-property",
      "Value": "CodeLength=11 ;; DescriptionLength=150",
      "dryRun": false
    }
  }
}
```

### Владельцы справочника

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.edit",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "src/Catalogs/ДоговорыКонтрагентов/ДоговорыКонтрагентов.xml",
      "Operation": "set-owners",
      "Value": "Catalog.Контрагенты ;; Catalog.Организации",
      "dryRun": false
    }
  }
}
```

## Верификация

### Валидация после редактирования

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.validate",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "<ObjectPath>"
    }
  }
}
```

### Сводка объекта

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "<ObjectPath>"
    }
  }
}
```
