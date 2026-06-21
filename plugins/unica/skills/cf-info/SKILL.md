---
name: cf-info
description: Анализ структуры конфигурации 1С — свойства, состав, счётчики объектов. Используй для обзора конфигурации — какие объекты есть, сколько их, какие настройки
argument-hint: <ConfigPath> [-Mode overview|brief|full] [-Section home-page]
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /cf-info — Структура конфигурации 1С

## MCP routing

- Preferred path: use MCP `unica` tool `unica.cf.info`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.cf.info`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Читает Configuration.xml из выгрузки конфигурации и выводит компактное описание структуры.

В `overview` и `full` выводит `Поддержка` по `Ext/ParentConfigurations.bin`: своя конфигурация, CFE, снята с поддержки, на поддержке, read-only, счётчики объектов на замке/редактируется/снято. Используй это как сигнал риска перед mutating `unica.*`; не редактируй файл поддержки напрямую.

## MCP параметры

| Параметр | Описание |
|----------|----------|
| `ConfigPath` | Путь к Configuration.xml или каталогу выгрузки |
| `Mode` | Режим: `overview` (default), `brief`, `full` |
| `Section` | Drill-down по разделу (alias: `Name`). Сейчас: `home-page` |
| `Limit` / `Offset` | Пагинация (по умолчанию 150 строк) |
| `OutFile` | Записать результат в файл (UTF-8 BOM) |

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.info",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigPath": "src/Configuration.xml",
      "Format": "text",
      "Limit": 150
    }
  }
}
```

## Три режима

| Режим | Что показывает |
|---|---|
| `overview` *(default)* | Заголовок + ключевые свойства + таблица счётчиков объектов по типам |
| `brief` | Одна строка: Имя — "Синоним" vВерсия \| N объектов \| совместимость |
| `full` | Все свойства по категориям + полный список ChildObjects + DefaultRoles + мобильные функциональности |

## Примеры

### Обзор пустой конфигурации

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.info",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigPath": "src"
    }
  }
}
```

### Краткая сводка реальной конфигурации

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.info",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigPath": "src",
      "Mode": "brief"
    }
  }
}
```

### Полная информация

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.info",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigPath": "src",
      "Mode": "full"
    }
  }
}
```

### Полная информация с пагинацией

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.info",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigPath": "src",
      "Mode": "full",
      "Limit": 50,
      "Offset": 100
    }
  }
}
```

### Drill-down: начальная страница

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.info",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigPath": "src",
      "Section": "home-page"
    }
  }
}
```
