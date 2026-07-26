---
name: mxl-decompile
description: Декомпиляция табличного документа (MXL) в JSON-определение. Используй когда нужно получить редактируемое описание существующего макета
argument-hint: <TemplatePath>
allowed-tools:
  - Bash
  - Read
  - Write
  - Glob
---

# /mxl-decompile — Декомпилятор макета в DSL

## MCP routing

- Preferred path: use MCP `unica` tool `unica.mxl.decompile`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.mxl.decompile`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Принимает Template.xml табличного документа 1С и возвращает компактное JSON-определение (DSL) в ответе MCP. Не создаёт файлов. Обратная операция к MCP `unica.mxl.compile`.

## Использование

```text
/mxl-decompile <TemplatePath>
```

## Параметры

| Параметр     | Обязательный | Описание            |
|--------------|:------------:|---------------------|
| TemplatePath | да           | Путь к Template.xml |

## MCP вызов

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.mxl.decompile",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "src/Reports/ОтчетПродажи/Templates/ПФ_MXL_Продажи"
    }
  }
}
```

## Рабочий процесс

Декомпиляция существующего макета для анализа или доработки:

1. Ассистент вызывает MCP `unica.mxl.decompile` для получения JSON из Template.xml
2. Ассистент при необходимости сохраняет JSON сам и анализирует или модифицирует его (добавляет области, меняет стили)
3. Ассистент вызывает MCP `unica.mxl.compile` для генерации нового Template.xml
4. Ассистент вызывает MCP `unica.mxl.validate` для проверки

## JSON-схема DSL

Полная спецификация формата: **`../../references/specs/mxl-dsl-spec.md`** (прочитать через Read tool).

## Генерация имён

Скрипт автоматически генерирует осмысленные имена:

- **Шрифты**: `default`, `bold`, `header`, `small`, `italic` — или описательные имена по свойствам
- **Стили**: `bordered`, `bordered-center`, `bold-right`, `border-top` и т.д. — по комбинации свойств

## Детектирование `rowStyle`

Если в строке есть пустые ячейки (без параметров/текста) и все они имеют одинаковый формат — этот формат распознаётся как `rowStyle`, а пустые ячейки исключаются из вывода.
