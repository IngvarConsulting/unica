---
name: mxl-info
description: Анализ структуры макета табличного документа (MXL) — области, параметры, наборы колонок. Используй при разработке печати — получить области и заполняемые параметры макета
argument-hint: <TemplatePath>
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /mxl-info — Анализ структуры макета

## MCP routing

- Preferred path: use MCP `unica` tool `unica.mxl.info`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.mxl.info`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Читает Template.xml табличного документа и выводит компактную сводку: именованные области, параметры, наборы колонок. Заменяет необходимость читать тысячи строк XML.

В текстовом выводе показывает `Поддержка` для объекта-владельца макета по `Ext/ParentConfigurations.bin`. JSON-режим сохраняет структурный контракт; состояние поддержки учитывай перед mutating `unica.mxl.*`.

## Использование

```
/mxl-info <TemplatePath>
```

## Параметры

| Параметр | Описание |
|----------|----------|
| `TemplatePath` | Путь к `Template.xml` макета или к каталогу макета |
| `WithText` | Включить текстовое содержимое ячеек в `texts` и `templates` |

`Format`, `MaxParams`, `Limit` и `Offset` сняты: результат приходит
типизированным в `data` (ADR-0023), поэтому режим вывода, обрезка списков
параметров и постраничная печать больше не нужны. `SrcDir`,
`ProcessorName` и `TemplateName` сняты по ADR-0048: составной адрес требовал
всех трёх, а `TemplatePath` был обязателен всегда, поэтому до этой ветки
вызов не доходил. Макет адресуется одним `TemplatePath`.

## Поля `data`

| Поле | Что содержит |
|------|--------------|
| `name` | Имя макета |
| `support` | Поддержка по `Ext/ParentConfigurations.bin` |
| `rows`, `columns` | Логическая высота и ширина по умолчанию |
| `columnSets` | Дополнительные наборы колонок: `id` и `size` |
| `areas` | Именованные области: `name`, `kind` (`Rows`, `Columns`, `Rectangle`, `Drawing`), границы, `columnsId`, `drawingId`, `params`, `details` |
| `areas[].texts`, `areas[].templates` | Содержимое ячеек — `null`, пока не запрошен `WithText` |
| `outside` | Параметры, детали и тексты вне именованных областей |
| `mergeCount`, `drawingCount` | Счётчики объединений и рисунков |

Пересечения строчных и колоночных областей для `ПолучитьОбласть` строятся из
`areas`: возьми `kind: "Rows"` и `kind: "Columns"` и перемножь имена.

## MCP вызов

### Прямой путь к Template.xml

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.mxl.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>/Ext/Template.xml"
    }
  }
}
```

### Каталог макета вместо файла

Каталог макета сам разрешается в `Ext/Template.xml`, поэтому путь из состава
объекта можно передать как есть, не дописывая хвост.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.mxl.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<каталог объекта>/Templates/<макет>"
    }
  }
}
```

### Включить текстовое содержимое ячеек

`WithText` — единственный оставшийся селектор состава: без него `texts` и
`templates` равны `null`, то есть «не запрашивали», а не «пусто».

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.mxl.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>",
      "WithText": true
    }
  }
}
```

## Чтение данных

### Области отсортированы сверху вниз

`areas` идут в порядке `beginRow` для строчных областей и `beginColumn` для
колоночных, поэтому порядок в массиве совпадает с порядком в макете.

### Параметры и detailParameter

`params` — параметры области, `details` — её `detailParameter`. Параметры,
пришедшие из шаблонов ячеек, помечены суффиксом `[tpl]`.

### Параметры вне областей

Всё, что лежит за пределами именованных областей, собрано в `outside`, а не
растворено среди областей.
