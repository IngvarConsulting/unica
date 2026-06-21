---
name: subsystem-info
description: Анализ структуры подсистемы 1С из XML-выгрузки — состав, дочерние подсистемы, командный интерфейс, дерево иерархии. Используй для изучения структуры подсистем и навигации по конфигурации
argument-hint: <SubsystemPath> [-Mode overview|content|ci|tree|full] [-Name <элемент>]
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /subsystem-info — Структура подсистемы 1С

## MCP routing

- Preferred path: use MCP `unica` tool `unica.subsystem.info`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.subsystem.info`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Читает XML подсистемы из выгрузки конфигурации 1С и выводит компактное описание структуры.

В `overview` и `full` показывает `Поддержка` подсистемы по `Ext/ParentConfigurations.bin`. Используй строку поддержки как guardrail перед `unica.subsystem.edit` или `unica.interface.edit`.

## MCP параметры

| Параметр | Описание |
|----------|----------|
| `SubsystemPath` | Путь к XML-файлу подсистемы, каталогу подсистемы или каталогу `Subsystems/` (для tree) |
| `Mode` | Режим: `overview` (default), `content`, `ci`, `tree`, `full` |
| `Name` | Drill-down: тип объекта в content, секция в ci, имя подсистемы в tree |
| `Limit` / `Offset` | Пагинация (по умолчанию 150 строк) |
| `OutFile` | Записать результат в файл (UTF-8 BOM) |

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.info",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "src/Subsystems/Продажи",
      "Mode": "overview",
      "Limit": 120
    }
  }
}
```

## Пять режимов

| Режим | Что показывает |
|---|---|
| `overview` *(default)* | Компактная сводка: свойства, состав (сгруппирован по типам), дочерние подсистемы, наличие CI |
| `content` | Список Content с группировкой по типу объекта. `-Name Catalog` — только каталоги |
| `ci` | Разбор CommandInterface.xml: видимость, размещение, порядок команд/подсистем/групп |
| `tree` | Рекурсивное дерево иерархии подсистем с маркерами [CI], [OneCmd], [Скрыт] |
| `full` | Полная сводка: overview + content + ci в одном вызове |

## Примеры

### Обзор подсистемы

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.info",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "Subsystems/Продажи.xml"
    }
  }
}
```

### Состав подсистемы

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.info",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "Subsystems/Администрирование.xml",
      "Mode": "content"
    }
  }
}
```

### Только документы в составе

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.info",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "Subsystems/Продажи.xml",
      "Mode": "content",
      "Name": "Document"
    }
  }
}
```

### Командный интерфейс подсистемы

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.info",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "Subsystems/Продажи.xml",
      "Mode": "ci"
    }
  }
}
```

### Дерево подсистем от корня

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.info",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "Subsystems",
      "Mode": "tree"
    }
  }
}
```

### Дерево от конкретной подсистемы

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.info",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "Subsystems/Администрирование.xml",
      "Mode": "tree"
    }
  }
}
```

### Дерево только для одной подсистемы

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.info",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "Subsystems",
      "Mode": "tree",
      "Name": "Администрирование"
    }
  }
}
```
