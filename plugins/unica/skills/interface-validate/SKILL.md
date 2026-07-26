---
name: interface-validate
description: Валидация командного интерфейса 1С. Используй после настройки командного интерфейса подсистемы для проверки корректности
argument-hint: <CIPath> [-Detailed] [-MaxErrors 30]
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /interface-validate — валидация CommandInterface.xml

## MCP routing

- Preferred path: use MCP `unica` tool `unica.interface.validate`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.interface.validate`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Проверяет XML командного интерфейса на структурные ошибки: корневой элемент, допустимые секции, порядок, формат ссылок на команды, дубликаты.

## Параметры

| Параметр  | Обяз. | Умолч. | Описание                                |
|-----------|:-----:|---------|-----------------------------------------|
| CIPath    | да    | —       | Путь к CommandInterface.xml             |
| Detailed  | нет   | —       | Подробный вывод (все проверки, включая успешные) |
| MaxErrors | нет   | 30      | Остановиться после N ошибок              |

## MCP вызов

### Каталог подсистемы

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.interface.validate",
    "arguments": {
      "cwd": "<workspace>",
      "CIPath": "Subsystems/Продажи"
    }
  }
}
```

### Прямой путь к CommandInterface.xml

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.interface.validate",
    "arguments": {
      "cwd": "<workspace>",
      "CIPath": "Subsystems/Продажи/Ext/CommandInterface.xml"
    }
  }
}
```
