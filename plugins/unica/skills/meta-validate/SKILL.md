---
name: meta-validate
description: Валидация объекта метаданных 1С. Используй после создания или модификации объекта конфигурации для проверки корректности
argument-hint: <ObjectPath> [-Detailed] [-MaxErrors 30] — pipe-separated paths for batch
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /meta-validate — валидация объекта метаданных 1С

## MCP routing

- Preferred path: use MCP `unica` tool `unica.meta.validate`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.meta.validate`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Проверяет XML объекта метаданных из выгрузки конфигурации на структурные ошибки.

## Параметры

| Параметр   | Обяз. | Умолч. | Описание                                      |
|------------|:-----:|---------|-------------------------------------------------|
| ObjectPath | да    | —       | Путь к XML-файлу или каталогу. Через `\|` для batch |
| Detailed   | нет   | —       | Подробный вывод (все проверки, включая успешные) |
| MaxErrors  | нет   | 30      | Остановиться после N ошибок (per object)        |

## MCP вызов

### Один объект

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.validate",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "Catalogs/Номенклатура/Номенклатура.xml"
    }
  }
}
```

### Несколько объектов через разделитель

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.validate",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "Catalogs/Банки|Documents/Заказ"
    }
  }
}
```
