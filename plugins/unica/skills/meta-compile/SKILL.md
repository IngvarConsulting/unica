---
name: meta-compile
description: Создать объект метаданных 1С. Используй когда нужно создать или добавить справочник, документ, регистр, перечисление, константу, общий модуль, обработку, отчёт и др.
argument-hint: <JsonPath> <OutputDir>
allowed-tools:
  - Bash
  - Read
  - Write
  - Glob
---

# /meta-compile — генерация объектов метаданных из JSON DSL

## MCP routing

- Preferred path: use MCP `unica` tool `unica.meta.compile`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.meta.compile`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Принимает JSON-определение объекта метаданных → генерирует XML + модули в структуре выгрузки конфигурации + регистрирует в Configuration.xml.

## Порядок работы

1. Составь JSON по синтаксису и примерам ниже → запиши во временный файл
2. Вызови MCP `unica.meta.compile`
3. Если нужно изменить созданный объект — `unica.meta.edit`
4. Если нужно проверить — `unica.meta.validate`

## MCP вызов

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.compile",
    "arguments": {
      "cwd": "<workspace>",
      "JsonPath": "definitions/catalog-products.json",
      "OutputDir": "src",
      "dryRun": false
    }
  }
}
```

| Параметр | Описание |
|----------|----------|
| `JsonPath` | Путь к JSON-файлу (один объект `{...}` или массив `[{...}, ...]`) |
| `OutputDir` | Корень выгрузки конфигурации (где `Configuration.xml`, `Catalogs/`, `Documents/` и т.д.) |

## JSON DSL

### Общая структура

```json
{ "type": "Catalog", "name": "Номенклатура", ...свойства типа... }
```

`type` и `name` — обязательные. `synonym` генерируется из `name` автоматически (CamelCase → слова через пробел). Можно задать явно: `"synonym": "Мой синоним"`.

### Shorthand реквизитов

Используется в `attributes`, `dimensions`, `resources`, `tabularSections`:

```
"ИмяРеквизита"                    → String(10) по умолчанию
"ИмяРеквизита: Тип"               → с типом
"ИмяРеквизита: Тип | req, index"  → с флагами
```

Типы: `String(100)`, `Number(15,2)`, `Boolean`, `Date`, `DateTime`, `CatalogRef.Xxx`, `DocumentRef.Xxx`, `EnumRef.Xxx`, `DefinedType.Xxx` и др. ссылочные.

Составной тип: `"Значение: String + Number(15,2) + CatalogRef.Контрагенты"`.

Флаги: `req`, `index`, `indexAdditional`, `nonneg`, `master`, `mainFilter`, `denyIncomplete`, `useInTotals`.

В объектной форме реквизита можно задать `choiceHistoryOnInput`, чтобы управлять `<ChoiceHistoryOnInput>` (`Auto`, `DontUse` и другие платформенные значения). Не указывай поле без необходимости: по умолчанию используется `Auto`.

### Свойства по типам

Примеров и shorthand-синтаксиса выше достаточно для типовых задач. Если нужны свойства типа, не показанные в примерах, и их допустимые значения — см. reference-файл:

- `reference/types-basic.md` — Catalog, Document, Enum, Constant, DefinedType, Report, DataProcessor
- `reference/types-registers.md` — InformationRegister, AccumulationRegister, AccountingRegister, CalculationRegister, ChartOfAccounts, ChartOfCharacteristicTypes, ChartOfCalculationTypes
- `reference/types-process.md` — BusinessProcess, Task, ExchangePlan, CommonModule, ScheduledJob, EventSubscription, DocumentJournal
- `reference/types-web.md` — HTTPService, WebService

Эта инструкция и reference-файлы — полная документация для генерации. Не ищи примеры XML в выгрузках конфигураций.

## Примеры паттернов DSL

### Минимальный объект

```json
{ "type": "Catalog", "name": "Валюты" }
```

### С реквизитами

```json
{
  "type": "Catalog", "name": "Организации",
  "descriptionLength": 100,
  "attributes": ["ИНН: String(12)", "КПП: String(9)", "Директор: CatalogRef.ФизическиеЛица"]
}
```

### С табличной частью

```json
{
  "type": "Document", "name": "ПриходнаяНакладная",
  "registerRecords": ["AccumulationRegister.ОстаткиТоваров"],
  "attributes": ["Организация: CatalogRef.Организации", "Контрагент: CatalogRef.Контрагенты"],
  "tabularSections": { "Товары": ["Номенклатура: CatalogRef.Номенклатура", "Количество: Number(15,3)", "Цена: Number(15,2)"] }
}
```

### Регистровый паттерн (измерения + ресурсы)

```json
{
  "type": "InformationRegister", "name": "КурсыВалют", "periodicity": "Day",
  "dimensions": ["Валюта: CatalogRef.Валюты | master, mainFilter, denyIncomplete"],
  "resources": ["Курс: Number(15,4)", "Кратность: Number(10,0)"]
}
```

### Batch — несколько объектов в одном файле

```json
[
  { "type": "Enum", "name": "Статусы", "values": ["Новый", "Закрыт"] },
  { "type": "Catalog", "name": "Валюты" },
  { "type": "Constant", "name": "ОсновнаяВалюта", "valueType": "CatalogRef.Валюты" }
]
```
