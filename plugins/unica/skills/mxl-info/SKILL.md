---
name: mxl-info
description: Анализ структуры макета табличного документа (MXL) — области, параметры, наборы колонок. Используй при разработке печати — получить области и заполняемые параметры макета
argument-hint: <TemplatePath> или <ProcessorName> <TemplateName>
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
/mxl-info <ProcessorName> <TemplateName>
```

## Параметры

| Параметр      | Обязательный | По умолчанию | Описание                                 |
|---------------|:------------:|--------------|------------------------------------------|
| TemplatePath  | нет          | —            | Прямой путь к Template.xml               |
| ProcessorName | нет          | —            | Имя обработки (альтернатива пути)        |
| TemplateName  | нет          | —            | Имя макета (альтернатива пути)           |
| SrcDir        | нет          | `src`        | Каталог исходников                       |
| Format        | нет          | `text`       | Формат вывода: `text` или `json`         |
| WithText      | нет          | false        | Включить статический текст и шаблоны     |
| MaxParams     | нет          | 10           | Макс. параметров в списке на область     |
| Limit         | нет          | 150          | Макс. строк вывода (защита от переполнения) |
| Offset        | нет          | 0            | Пропустить N строк (для пагинации)       |

Укажите либо `-TemplatePath`, либо оба `-ProcessorName` и `-TemplateName`.

## MCP вызов

### Прямой путь к Template.xml или каталогу макета

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.mxl.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>"
    }
  }
}
```

### По имени обработки и макета

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.mxl.info",
    "arguments": {
      "cwd": "<workspace>",
      "ProcessorName": "<Имя>",
      "TemplateName": "<Макет>",
      "SrcDir": "<каталог>"
    }
  }
}
```

### Включить текстовое содержимое ячеек

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

### JSON-вывод для программной обработки

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.mxl.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>",
      "Format": "json"
    }
  }
}
```

### Показать больше параметров на область

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.mxl.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>",
      "MaxParams": 20
    }
  }
}
```

### Пагинация

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.mxl.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>",
      "Offset": 150
    }
  }
}
```

## Чтение вывода

### Области — сортировка сверху вниз

Области перечислены в порядке документа (по позиции строки), а не по алфавиту. Это соответствует порядку вывода областей в коде заполнения — сверху вниз.

```
--- Named areas ---
  Заголовок          Rows     rows 1-4     (1 params)
  Поставщик          Rows     rows 5-6     (1 params)
  Строка             Rows     rows 14-14   (8 params)
  Итого              Rows     rows 16-17   (1 params)
```

Типы областей:
- **Rows** — горизонтальная область (диапазон строк). Получение: `Макет.ПолучитьОбласть("Имя")`
- **Columns** — вертикальная область (диапазон колонок). Получение: `Макет.ПолучитьОбласть("Имя")`
- **Rectangle** — фиксированная область (строки + колонки). Обычно использует отдельный набор колонок.
- **Drawing** — именованный рисунок/штрихкод.

### Пересечения

Когда есть области и Rows, и Columns (этикетки, ценники), скрипт выводит пары пересечений:

```
--- Intersections (use with GetArea) ---
  ВысотаЭтикетки|ШиринаЭтикетки
```

В BSL: `Макет.ПолучитьОбласть("ВысотаЭтикетки|ШиринаЭтикетки")`

### Параметры и detailParameter

Параметры перечислены по областям. Если у параметра есть `detailParameter` (расшифровка), он показан ниже:

```
--- Parameters by area ---
  Поставщик: ПредставлениеПоставщика
    detail: ПредставлениеПоставщика->Поставщик
  Строка: НомерСтроки, Товар, Количество, Цена, Сумма, ... (+3)
    detail: Товар->Номенклатура
```

Это означает: параметр `Товар` отображает значение, а при клике открывает `Номенклатура` (объект расшифровки).

### Параметры из шаблонов (суффикс `[tpl]`)

Некоторые параметры встроены в шаблонный текст: `"Инв № [ИнвентарныйНомер]"`. Они заполняются через fillType=Template, а не fillType=Parameter. Скрипт всегда извлекает их и помечает суффиксом `[tpl]`:

```
  НумерацияЛистов: Номер [tpl], Дата [tpl], НомерЛиста [tpl]
```

В BSL шаблонные параметры заполняются так же, как обычные:
```bsl
Область.Параметры.Номер = НомерДокумента;
Область.Параметры.Дата = ДатаДокумента;
```

Числовые подстановки вроде `[5]`, `[6]` (ссылки на сноски в официальных формах) игнорируются.

### Текстовое содержимое (`-WithText`)

Показывает статический текст (надписи, заголовки) и шаблонные строки с подстановками `[Параметр]`:

```
--- Text content ---
  ШапкаТаблицы:
    Text: "№", "Товар", "Ед. изм.", "Кол-во", "Цена", "Сумма"
  Строка:
    Templates: "Инв № [ИнвентарныйНомер]"
```

- **Text** — статические надписи (fillType=Text). Полезно для понимания назначения колонок.
- **Templates** — текст с подстановками `[ИмяПараметра]` (fillType=Template). Параметр внутри `[]` заполняется программно.
