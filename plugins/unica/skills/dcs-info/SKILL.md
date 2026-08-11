---
name: dcs-info
description: Анализ структуры схемы компоновки данных 1С (СКД) — наборы, поля, параметры, варианты. Используй для понимания отчёта — источник данных (запрос), доступные поля, параметры
argument-hint: <TemplatePath>
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /dcs-info — Анализ схемы компоновки данных

## MCP routing

- Preferred path: use MCP `unica` tool `unica.dcs.info`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.dcs.info`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Читает Template.xml схемы компоновки данных (СКД) и выводит компактную сводку. Заменяет необходимость читать тысячи строк XML.

В `overview` и `full` показывает `Поддержка` для объекта-владельца макета по `Ext/ParentConfigurations.bin`. Режим `query` остаётся пригодным для round-trip текста запроса; support-state используй как риск перед `unica.dcs.edit`.

## Ответ

Инструмент отвечает типизированным `data` и отдаёт схему целиком; режимы,
`Raw`, `Name`, `Limit` и `Offset` больше не нужны:

| Секция | Что в ней |
|---|---|
| `dataSets` | наборы данных, их поля и сырой текст запроса целиком, с отступами строк продолжения |
| `links` | связи наборов данных с выражениями источника и приёмника |
| `calculatedFields` | вычисляемые поля с выражением и заголовком |
| `totalFields` | ресурсы (итоги) с выражением |
| `parameters` | параметры с типом, значением, выражением и признаком ограничения |
| `variants` | варианты настроек: отбор, порядок, структура |
| `templates` | макеты схемы |

## MCP параметры

| Параметр | Описание |
|----------|----------|
| `TemplatePath` | Путь к Template.xml или каталогу макета (авто-резолв в `Ext/Template.xml`) |
| `sourceSet`    | Имя набора исходников из `v8project.yaml`                                  |
| `metadataPath` | Логический адрес, например `Report.<Отчёт>.Template.<Макет>`               |

Кроме селектора цели предметных аргументов нет — только общие `cwd` и `confirm`.

Селектор цели ровно один: либо `sourceSet` + `metadataPath`, либо
`TemplatePath`. Оба сразу отклоняются кодом `selector_conflict` (ADR-0049).
 `Mode`,
`Name`, `Batch`, `Raw`, `Limit` и `Offset` сняты
(ADR-0048): схема приходит целиком, сырой текст запроса лежит в
`dataSets[].query`, а отбор набора, поля или варианта выполняется над `data`.

### Overview: точка входа

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.dcs.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>"
    }
  }
}
```

### Текст запроса набора

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.dcs.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>"
    }
  }
}
```

### Текст третьего пакета запроса

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.dcs.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>"
    }
  }
}
```

### Сырой текст запроса для round-trip правки

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.dcs.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>"
    }
  }
}
```

### Деталь поля

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.dcs.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>"
    }
  }
}
```

### Вычисляемое поле

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.dcs.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>"
    }
  }
}
```

### Ресурс

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.dcs.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>"
    }
  }
}
```

### Трассировка поля

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.dcs.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>"
    }
  }
}
```

### Вариант настроек

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.dcs.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>"
    }
  }
}
```

### Карта шаблонов

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.dcs.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>"
    }
  }
}
```

### Шаблон по имени

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.dcs.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>"
    }
  }
}
```

## Что где лежит в `data`

Прежние одиннадцать режимов были одиннадцатью отчётами по одной схеме. Теперь
ответ приходит целиком, и «режим» — это выбор секции в `data` у себя:

| Прежний режим | Где эти факты теперь |
|---------------|----------------------|
| `overview` | `support`, `dataSources`, `dataSets[].name`/`kind`, `links` |
| `query` | `dataSets[].query` — текст целиком, включая многопакетные запросы |
| `fields` | `dataSets[].fields[]`: `dataPath`, `field`, `title` |
| `links` | `links[]` с выражениями источника и приёмника |
| `calculated` | `calculatedFields[]`: выражение, заголовок, `restricted` |
| `resources` | `totalFields[]`: выражение и `group` (`null` — итог в целом) |
| `params` | `parameters[]`: тип, значение, выражение, `restricted`, `availableAsField` |
| `variant` | `variants[]`: `selection`, `order`, `filters`, `structure[].groupBy` |
| `templates` | `templates[]` |
| `trace` | Собирается у потребителя: поле ищется в `dataSets[].fields`, затем в `calculatedFields` и `totalFields` |
| `full` | Весь ответ и есть `full` |

`-Name` больше не аргумент инструмента. Отбор по имени делается над нужной
коллекцией внутри `data`: набор — фильтром `data.dataSets` по `name`, поле —
обходом `data.dataSets[].fields` по `dataPath`, вычисляемое поле и ресурс — по
`dataPath` в `data.calculatedFields` и `data.totalFields`, вариант — по `name` в
`data.variants`. Второй вызов инструмента для этого не нужен.

## Типичный workflow

1. Один вызов — вся схема в `data`.
2. Колонка отчёта: найди `dataPath` в `calculatedFields` или `totalFields`,
   затем сопоставь операнды выражения с `dataSets[].fields[].dataPath`.
3. Текст запроса: `dataSets[].query`.
4. Группировки и фильтры варианта: `variants[].structure[].groupBy` и
   `variants[].filters`.

Переработка запроса (round-trip): возьми `data.dataSets[].query` ->
правка текста из ответа -> `unica.dcs.edit` с `Operation=set-query` и `Value=<исправленный текст>`.
Текст приходит без декораций и без обрезки, поэтому передача точна, включая
многопакетные запросы с временными таблицами.

## Верификация

### Overview: точка входа

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.dcs.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<path>"
    }
  }
}
```

### Трассировка поля

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.dcs.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<path>"
    }
  }
}
```

## Логический адрес вместо пути

`unica.dcs.info` принимает либо логический селектор, либо файловый путь —
ровно один из двух. Оба сразу отклоняются кодом `selector_conflict`.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.dcs.info",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "<имя набора>",
      "metadataPath": "Report.<Отчёт>.Template.<Макет>"
    }
  }
}
```

Имя набора даёт `unica.project.map`, адрес — `unica.source.resolve`, а
`unica.source.locate` переводит в адрес путь, найденный иначе. Файловый
селектор сохраняется до отдельного среза его снятия (ADR-0049).
