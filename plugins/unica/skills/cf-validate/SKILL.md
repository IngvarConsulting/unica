---
name: cf-validate
description: Валидация конфигурации 1С. Используй после создания или модификации конфигурации для проверки корректности
argument-hint: <ConfigPath> [-Detailed] [-MaxErrors 30]
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /cf-validate — валидация конфигурации 1С

## MCP routing

- Preferred path: use MCP `unica` tool `unica.cf.validate`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.cf.validate`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Проверяет Configuration.xml на структурные ошибки: XML well-formedness, InternalInfo, свойства, enum-значения, ChildObjects, DefaultLanguage, файлы языков, каталоги объектов.

## Параметры

| Параметр   | Обяз. | Умолч. | Описание                                      |
|------------|:-----:|---------|-------------------------------------------------|
| ConfigPath | один из двух | —       | Путь к Configuration.xml или каталогу выгрузки  |
| sourceSet  | один из двух | —       | Имя набора исходников из `v8project.yaml`       |
| Detailed   | нет   | —       | Подробный вывод (все проверки, включая успешные) |
| MaxErrors  | нет   | 30      | Остановиться после N ошибок                     |

Селектор цели ровно один: либо `sourceSet`, либо `ConfigPath`. Оба сразу
отклоняются кодом `selector_conflict` (ADR-0049).

## MCP вызов

### Каталог выгрузки

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.validate",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigPath": "upload/cfempty"
    }
  }
}
```

### Прямой путь к Configuration.xml

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.validate",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigPath": "upload/cfempty/Configuration.xml"
    }
  }
}
```

## Логический адрес вместо пути

`unica.cf.validate` принимает либо логический селектор, либо файловый путь —
ровно один из двух. Оба сразу отклоняются кодом `selector_conflict`.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.validate",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "<имя набора>"
    }
  }
}
```

Имя набора даёт `unica.project.map`. Файловый селектор сохраняется до
отдельного среза его снятия (ADR-0049).
