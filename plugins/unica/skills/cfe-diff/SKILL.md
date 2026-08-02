---
name: cfe-diff
description: Анализ расширения конфигурации 1С (CFE) — состав, заимствованные объекты, перехватчики, проверка переноса. Используй когда нужно понять что содержит расширение или проверить перенесены ли вставки в конфигурацию
argument-hint: -ExtensionPath <path> -ConfigPath <path> [-Mode A|B]
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /cfe-diff — Анализ расширения конфигурации

## MCP routing

- Preferred path: use MCP `unica` tool `unica.cfe.diff`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.cfe.diff`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Анализирует расширение в двух режимах: обзор изменений (Mode A) или проверка переноса (Mode B).

## Параметры

| Параметр | Обязательный | Описание |
|----------|:------------:|----------|
| `ExtensionPath` | да | Каталог выгрузки расширения |
| `ConfigPath` | да | Каталог выгрузки конфигурации, с которой сверяется перенос |

`Mode` снят: прежние режимы A и B были двумя взглядами на одно расширение, и
типизированный ответ несёт оба сразу (ADR-0023).

## Поля `data`

| Поле | Что содержит |
|------|--------------|
| `name`, `purpose`, `namePrefix` | Идентичность расширения и его назначение |
| `objects[]` | Состав: `kind`, `name` и `status` — `borrowed`, `own`, `missing` или `unknownKind` |
| `objects[].modules[]` | Модули объекта с перехватчиками: `method` и `kind` перехвата |
| `objects[]` счётчики | `attributes`, `forms`, `tabularSections`, `borrowedItems`, `formNames` |
| `totals` | Сколько объектов заимствовано и сколько собственных |
| `transfer[]` | Проверка переноса вставок: `status` — `transferred`, `notTransferred` или `needsReview`, плюс `blocks` и `reason` |
| `transferTotals` | Итоги проверки переноса |

## MCP вызов

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cfe.diff",
    "arguments": {
      "cwd": "<workspace>",
      "ExtensionPath": "src/cfe",
      "ConfigPath": "src/cf"
    }
  }
}
```

## Примеры

### Что содержит расширение

`objects[].status` отделяет заимствованные объекты от собственных, а
`objects[].modules[].interceptors[]` показывает перехватчики каждого модуля;
каждый перехватчик содержит `method` и `kind`.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cfe.diff",
    "arguments": {
      "cwd": "<workspace>",
      "ExtensionPath": "src/cfe",
      "ConfigPath": "src/cf"
    }
  }
}
```

### Перенесены ли вставки в конфигурацию

`transfer[]` перечисляет перехватчики `&ИзменениеИКонтроль` со статусом
переноса; `needsReview` всегда несёт `reason`, поэтому причина не теряется.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cfe.diff",
    "arguments": {
      "cwd": "<workspace>",
      "ExtensionPath": "src/cfe",
      "ConfigPath": "src/cf"
    }
  }
}
```
