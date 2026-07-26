---
name: mxl-compile
description: Компиляция табличного документа (MXL) из JSON-определения. Используй когда нужно создать макет печатной формы
argument-hint: <JsonPath> <OutputPath>
allowed-tools:
  - Bash
  - Read
  - Write
  - Glob
---

# /mxl-compile — Компилятор макета из DSL

## MCP routing

- Preferred path: use MCP `unica` tool `unica.mxl.compile`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.mxl.compile`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.
- Vendor support guard runs inside `unica`; if it blocks a locked/read-only supported object, prefer CFE/release-support or an explicit support-state change plan instead of editing raw support metadata.

Принимает компактное JSON-определение макета и генерирует корректный Template.xml для табличного документа 1С. Ассистент описывает *что* нужно (области, параметры, стили), MCP-инструмент обеспечивает *корректность* XML (палитры, индексы, объединения, namespace).

## Использование

```
/mxl-compile <JsonPath> <OutputPath>
```

## Параметры

| Параметр   | Обязательный | Описание                           |
|------------|:------------:|------------------------------------|
| JsonPath   | да           | Путь к JSON-определению макета     |
| OutputPath | да           | Путь для генерации Template.xml    |

## MCP вызов

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.mxl.compile",
    "arguments": {
      "cwd": "<workspace>",
      "JsonPath": "mxl/print-form.json",
      "OutputPath": "src/Reports/ОтчетПродажи/Templates/ПФ_MXL_Продажи/Ext/Template.xml",
      "dryRun": false
    }
  }
}
```

## Рабочий процесс

1. Ассистент пишет JSON-определение через Write tool в файл `.json`
2. Ассистент вызывает MCP `unica.mxl.compile` для генерации Template.xml
3. Ассистент вызывает MCP `unica.mxl.validate` для проверки корректности
4. Ассистент вызывает MCP `unica.mxl.info` для верификации структуры

**Если макет создаётся по изображению** (скриншот, скан печатной формы) — определить структуру, границы колонок и пропорции внешним средством анализа изображения, затем использовать `"Nx"` ширины + `"page"` для автоматического расчёта размеров. Workflow Unica начинается с JSON-структуры и вызовов `unica.mxl.*`.

## JSON-схема DSL

Полная спецификация формата: **`../../references/specs/mxl-dsl-spec.md`** (прочитать через Read tool перед написанием JSON).

Краткая структура:

```
{ columns, page, defaultWidth, columnWidths,
  fonts: { name: { face, size, bold, italic, underline, strikeout } },
  styles: { name: { font, align, valign, border, borderWidth, wrap, format } },
  areas: [{ name, rows: [{ height, rowStyle, cells: [
    { col, span, rowspan, style, param, detail, text, template }
  ]}]}]
}
```

Ключевые правила:
- `page` — формат страницы (`"A4-landscape"`, `"A4-portrait"` или число). Автоматически вычисляет `defaultWidth` из суммы пропорций `"Nx"`
- `col` — 1-based позиция колонки
- `rowStyle` — автозаполнение пустот стилем (рамки по всей ширине)
- Тип заполнения определяется автоматически: `param` → Parameter, `text` → Text, `template` → Template
- `rowspan` — объединение строк вниз (rowStyle учитывает занятые ячейки)
