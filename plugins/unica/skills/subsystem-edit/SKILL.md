---
name: subsystem-edit
description: Точечное редактирование подсистемы 1С. Используй когда нужно добавить или удалить объекты из подсистемы, управлять дочерними подсистемами или изменить свойства
argument-hint: -SubsystemPath <path> -Operation <op> -Value <value>
allowed-tools:
  - Bash
  - Read
  - Write
  - Glob
---

# /subsystem-edit — редактирование подсистемы 1С

## MCP routing

- Preferred path: use MCP `unica` tool `unica.subsystem.edit`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.subsystem.edit`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.
- Vendor support guard runs inside `unica`; if it blocks a locked/read-only supported object, prefer CFE/release-support or an explicit support-state change plan instead of editing raw support metadata.

Точечное редактирование XML подсистемы: состав, дочерние подсистемы, свойства.

## MCP параметры

| Параметр | Описание |
|----------|----------|
| `SubsystemPath` | Путь к XML-файлу подсистемы |
| `DefinitionFile` | JSON-файл с массивом операций |
| `Operation` | Одна операция (альтернатива DefinitionFile) |
| `Value` | Значение для операции |
| `NoValidate` | Скрыть подробный отчёт авто-валидации; обязательная проверка корректности 8.3.27 перед фиксацией остаётся включённой |

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.edit",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "src/Subsystems/Продажи",
      "Operation": "add-content",
      "Value": "Catalog.Номенклатура",
      "dryRun": false
    }
  }
}
```

## Операции

| Операция | Значение | Описание |
|----------|----------|----------|
| `add-content` | `"Catalog.X"` или `["Catalog.X","Document.Y"]` | Добавить объекты в Content |
| `remove-content` | `"Catalog.X"` или `["Catalog.X"]` | Удалить объекты из Content |
| `add-child` | `"ИмяПодсистемы"` | Добавить дочернюю подсистему в ChildObjects |
| `remove-child` | `"ИмяПодсистемы"` | Удалить дочернюю подсистему |
| `set-property` | `{"name":"prop","value":"val"}` | Изменить свойство (Synonym, IncludeInCommandInterface, UseOneCommand, etc.) |

## Примеры

### Добавить объект в состав

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.edit",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "Subsystems/Продажи.xml",
      "Operation": "add-content",
      "Value": "Document.Заказ",
      "dryRun": false
    }
  }
}
```

### Добавить несколько объектов

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.edit",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "Subsystems/Продажи.xml",
      "Operation": "add-content",
      "Value": "[\"Catalog.Товары\",\"Report.Продажи\"]",
      "dryRun": false
    }
  }
}
```

### Удалить объект из состава

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.edit",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "Subsystems/Продажи.xml",
      "Operation": "remove-content",
      "Value": "Report.Старый",
      "dryRun": false
    }
  }
}
```

### Добавить дочернюю подсистему

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.edit",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "Subsystems/Продажи.xml",
      "Operation": "add-child",
      "Value": "НоваяДочерняя",
      "dryRun": false
    }
  }
}
```

### Изменить свойство

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.edit",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "Subsystems/Продажи.xml",
      "Operation": "set-property",
      "Value": "{\"name\":\"IncludeInCommandInterface\",\"value\":\"false\"}",
      "dryRun": false
    }
  }
}
```
