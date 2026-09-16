---
name: mxl-compile
description: Создание макета табличного документа (MXL) и его областей операциями unica.apply. Используй когда нужен макет печатной формы; сборки целого макета из JSON-определения на поверхности нет
allowed-tools:
  - Bash
  - Read
  - Write
  - Glob
---

# /mxl-compile — Компилятор макета из DSL

## MCP routing

- Preferred path: use MCP `unica` tool `unica.apply`; макет табличного
  документа заводится операцией `template.add`, области правятся `mxl.set`.
- Не зови внутренние адаптеры напрямую: они спрятаны за MCP `unica`.
- Всегда сначала `dryRun: true`; `dryRun: false` — только по явной просьбе
  пользователя и только с `ifRev` из превью.

## Шаг 1 — завести макет

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Report.Продажи",
      "ops": [
        {"op": "template.add", "args": {"items": [{"name": "ПФ_MXL_Продажи", "templateType": "SpreadsheetDocument"}]}}
      ],
      "dryRun": true
    }
  }
}
```


**Порядок важен.** Превью ничего не публикует, поэтому дочерний узел появится
только после применения первого шага:

1. превью `template.add` (`dryRun: true`) — план и `rev`;
2. применение `template.add` (`dryRun: false` с `ifRev` из превью) — макет создан;
3. превью операций наполнения по адресу макета;
4. применение их плана со своим `ifRev`.

Обратиться к адресу макета до шага 2 нельзя: план отказывает, потому что цели
ещё нет.

## Шаг 2 — задать область

`mxl.set` адресует сам макет и задаёт одну именованную область: `area` — имя,
`cells` — её ячейки, `columns` — число колонок.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Report.Продажи.Template.ПФ_MXL_Продажи",
      "ops": [
        {"op": "mxl.set", "args": {"values": {"area": "Шапка", "columns": 4, "cells": [{"col": 1, "text": "Номенклатура"}, {"col": 2, "text": "Сумма"}]}}}
      ],
      "dryRun": true
    }
  }
}
```

Области задаются по одной; несколько операций `mxl.set` идут одним `ops` и
применяются атомарно.

## Чего словарь не пишет

Сборки целого макета из готового JSON-определения одним вызовом на поверхности
**нет**: DSL ниже остаётся справочником формата, а не входом инструмента.
Шрифты, стили, палитры, объединения ячеек и параметры страницы `mxl.set` не
принимает — он берёт `area`, `cells` и `columns`. Макет, которого этими
средствами не собрать, — пробел контракта Unica MCP; писать `Template.xml`
руками в обход поверхности нельзя.

Если макет создаётся по изображению, структуру и пропорции определи внешним
средством анализа изображения, а затем выражай области операциями выше.

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
