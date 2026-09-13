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

- Preferred path: use MCP `unica` tools `unica.view` and `unica.search`.
- Do not call internal MCP/CLI adapters directly. They are hidden behind
  `unica` and synchronized by the orchestrator.
- Макет — узел логического дерева: `unica.view {at}` по адресу
  `<набор>:<Вид>.<Имя>.Template.<Макет>`. Файлового селектора у чтения нет;
  путь, пришедший снаружи, переводит в адрес аварийный `unica.resolve`.

Узел макета отдаёт компактную сводку: именованные области, параметры, наборы
колонок. Читать тысячи строк XML не нужно.

Поддержка объекта-владельца приходит полем `support`; учитывай её состояние
перед правкой макета.

## Адресация

| Что | Как |
|-----|-----|
| Имя набора исходников | `unica.view {}` |
| Адрес макета по имени | `unica.search {corpus: "names", kind: "Template"}` |
| Адрес по пути извне | `unica.resolve {path}` |
| Сам макет | `unica.view {at: "<набор>:<Вид>.<Имя>.Template.<Макет>"}` |
| Область макета | `unica.view {at: "…Template.<Макет>.Area.<Область>"}` |
| Содержимое ячеек области | `unica.view {at: "…Area.<Область>.Body"}` |

## Поля узла

| Поле | Что содержит |
|------|--------------|
| `title` | Имя макета |
| `props.support` | Поддержка по `Ext/ParentConfigurations.bin` |
| `props.rows`, `props.columns` | Логическая высота и ширина по умолчанию |
| `props.columnSets` | Дополнительные наборы колонок: `id` и `size` |
| ветвь `Area` | Именованные области; счёт ветви равен их числу |
| `props.mergeCount`, `props.drawingCount` | Счётчики объединений и рисунков |

У узла области в `props` лежат `kind` (`Rows`, `Columns`, `Rectangle`,
`Drawing`), границы, `columnsId`, `drawingId` и `contentCount` — число непустых
ячеек. Ветвь `Parameter` перечисляет параметры области, ветвь `Body` — её
содержимое.

Пересечения строчных и колоночных областей для `ПолучитьОбласть` строятся из
ветви `Area`: возьми области `kind: "Rows"` и `kind: "Columns"` и перемножь
имена.

## MCP вызов

### Макет целиком

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.view",
    "arguments": { "at": "main:Report.Продажи.Template.Печать" }
  }
}
```

### Одна область

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.view",
    "arguments": { "at": "main:Report.Продажи.Template.Печать.Area.Шапка" }
  }
}
```

### Содержимое ячеек области

Текст ячеек — отдельный адрес, а не признак в аргументах: структурное чтение
области за текст не платит, а ветвь `Body` честно объявляет свою длину полем
`contentCount`.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.view",
    "arguments": { "at": "main:Report.Продажи.Template.Печать.Area.Шапка.Body" }
  }
}
```

Каждая ячейка приходит в порядке чтения и несёт `index`, `text` и признак
`template` — стоят ли в ней подстановки `[Параметр]`.

## Чтение данных

### Области отсортированы сверху вниз

Элементы ветви `Area` идут в порядке `beginRow` для строчных областей и
`beginCol` для колоночных, поэтому порядок совпадает с порядком в макете.

### Параметры и detailParameter

Ветвь `Parameter` перечисляет параметры области. Параметры, пришедшие из
шаблонов ячеек, помечены суффиксом `[tpl]`.

### Параметры вне областей

Всё, что лежит за пределами именованных областей, собрано отдельно, а не
растворено среди областей.
