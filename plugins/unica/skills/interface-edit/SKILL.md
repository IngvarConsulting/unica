---
name: interface-edit
description: Настройка командного интерфейса подсистемы 1С. Используй когда нужно скрыть или показать команды, разместить в группах, настроить порядок
argument-hint: <CIPath> <Operation> <Value>
allowed-tools:
  - Bash
  - Read
  - Write
  - Glob
---

# /interface-edit — редактирование CommandInterface.xml

## MCP routing

- Preferred path: use MCP `unica` tool `unica.interface.edit`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.interface.edit`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.
- Vendor support guard runs inside `unica`; if it blocks a locked/read-only supported object, prefer CFE/release-support or an explicit support-state change plan instead of editing raw support metadata.

Точечное редактирование файла командного интерфейса подсистемы 1С.

## Параметры

| Параметр | Обяз. | Описание |
|----------|:-----:|----------|
| CIPath | да | Путь к CommandInterface.xml |
| Operation | нет | Операция: hide, show, place, order, subsystem-order, group-order |
| Value | нет | Значение для операции |
| DefinitionFile | нет | JSON-файл с массивом операций (альтернатива Operation) |
| CreateIfMissing | нет | Создать файл если не существует |
| NoValidate | нет | Скрыть подробный отчёт авто-валидации; обязательная проверка корректности 8.3.27 перед фиксацией остаётся включённой |

## MCP вызов

### Inline mode

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.interface.edit",
    "arguments": {
      "cwd": "<workspace>",
      "CIPath": "<path>",
      "Operation": "hide",
      "Value": "<cmd>",
      "dryRun": false
    }
  }
}
```

### JSON mode

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.interface.edit",
    "arguments": {
      "cwd": "<workspace>",
      "CIPath": "<path>",
      "DefinitionFile": "<json>",
      "dryRun": false
    }
  }
}
```

## Операции

| Операция | Значение | Описание |
|----------|----------|----------|
| hide | Cmd.Name или массив | Скрыть команду (CommandsVisibility, false) |
| show | Cmd.Name или массив | Показать команду (visibility, true) |
| place | {"command":"...","group":"CommandGroup.X"} | Разместить команду в группе |
| order | {"group":"...","commands":[...]} | Задать порядок команд в группе |
| subsystem-order | ["Subsystem.X.Subsystem.A",...] | Порядок дочерних подсистем |
| group-order | ["NavigationPanelOrdinary",...] | Порядок групп |

## Примеры

### Скрыть команду

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.interface.edit",
    "arguments": {
      "cwd": "<workspace>",
      "CIPath": "Subsystems/Продажи/Ext/CommandInterface.xml",
      "Operation": "hide",
      "Value": "Catalog.Товары.StandardCommand.OpenList",
      "dryRun": false
    }
  }
}
```

### Показать команду

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.interface.edit",
    "arguments": {
      "cwd": "<workspace>",
      "CIPath": "Subsystems/Продажи/Ext/CommandInterface.xml",
      "Operation": "show",
      "Value": "Report.Продажи.Command.Отчёт",
      "dryRun": false
    }
  }
}
```

### Разместить в группе

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.interface.edit",
    "arguments": {
      "cwd": "<workspace>",
      "CIPath": "Subsystems/Продажи/Ext/CommandInterface.xml",
      "Operation": "place",
      "Value": "{\"command\":\"Report.X.Command.Y\",\"group\":\"CommandGroup.Отчеты\"}",
      "dryRun": false
    }
  }
}
```

### Задать порядок подсистем

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.interface.edit",
    "arguments": {
      "cwd": "<workspace>",
      "CIPath": "Subsystems/Продажи/Ext/CommandInterface.xml",
      "Operation": "subsystem-order",
      "Value": "[\"Subsystem.X.Subsystem.A\",\"Subsystem.X.Subsystem.B\"]",
      "dryRun": false
    }
  }
}
```

### Создать новый командный интерфейс

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.interface.edit",
    "arguments": {
      "cwd": "<workspace>",
      "CIPath": "<new-path>",
      "Operation": "subsystem-order",
      "Value": "[...]",
      "CreateIfMissing": true,
      "dryRun": false
    }
  }
}
```

## Верификация

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.interface.validate",
    "arguments": {
      "cwd": "<workspace>",
      "CIPath": "<CIPath>"
    }
  }
}
```
