---
name: meta-remove
description: Удалить объект метаданных из конфигурации 1С. Используй когда нужно удалить, убрать объект из конфигурации
argument-hint: <ConfigDir> -Object <Type.Name>
allowed-tools:
  - Bash
  - Read
  - Glob
  - AskUserQuestion
---

# /meta-remove — удаление объекта метаданных

## MCP routing

- Preferred path: use MCP `unica` tool `unica.meta.remove`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.meta.remove`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.
- Vendor support guard runs inside `unica`; if it blocks a locked/read-only supported object, prefer CFE/release-support or an explicit support-state change plan instead of editing raw support metadata.

Безопасно удаляет объект из XML-выгрузки конфигурации. Перед удалением проверяет ссылки на объект в реквизитах, коде и других метаданных. Если ссылки найдены — удаление блокируется.

## Использование

```
/meta-remove <ConfigDir> -Object <Type.Name>
```

## Параметры

| Параметр   | Обязательный | Описание                                        |
|------------|:------------:|-------------------------------------------------|
| ConfigDir  | да           | Корневая директория выгрузки (где Configuration.xml) |
| Object     | да           | Тип и имя объекта: `Catalog.Товары`, `Document.Заказ` и т.д. |
| DryRun     | нет          | Только показать что будет удалено, без изменений |
| Force      | нет          | Удалить несмотря на найденные ссылки            |

## MCP вызов

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.remove",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigDir": "src",
      "Object": "Catalog.СтарыйСправочник",
      "dryRun": false
    }
  }
}
```

## Поддерживаемые типы

Catalog, Document, Enum, Constant, InformationRegister, AccumulationRegister, AccountingRegister, CalculationRegister, ChartOfAccounts, ChartOfCharacteristicTypes, ChartOfCalculationTypes, BusinessProcess, Task, ExchangePlan, DocumentJournal, Report, DataProcessor, CommonModule, ScheduledJob, EventSubscription, HTTPService, WebService, DefinedType, Role, Subsystem, CommonForm, CommonTemplate, CommonPicture, CommonAttribute, SessionParameter, FunctionalOption, FunctionalOptionsParameter, Sequence, FilterCriterion, SettingsStorage, XDTOPackage, WSReference, StyleItem, Language

## Примеры

### Проверка ссылок и dry run

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.remove",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigDir": "C:\\WS\\tasks\\cfsrc\\acc_8.3.24",
      "Object": "Catalog.Устаревший",
      "dryRun": true
    }
  }
}
```

### Удалить объект без ссылок

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.remove",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigDir": "C:\\WS\\tasks\\cfsrc\\acc_8.3.24",
      "Object": "Catalog.Устаревший",
      "dryRun": false
    }
  }
}
```

### Принудительно удалить несмотря на ссылки

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.remove",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigDir": "C:\\WS\\tasks\\cfsrc\\acc_8.3.24",
      "Object": "Catalog.Устаревший",
      "Force": true,
      "dryRun": false
    }
  }
}
```

### Удалить общий модуль

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.remove",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigDir": "src",
      "Object": "CommonModule.МойМодуль",
      "dryRun": false
    }
  }
}
```
