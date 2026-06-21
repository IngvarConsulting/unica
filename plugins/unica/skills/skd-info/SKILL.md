---
name: skd-info
description: Анализ структуры схемы компоновки данных 1С (СКД) — наборы, поля, параметры, варианты. Используй для понимания отчёта — источник данных (запрос), доступные поля, параметры
argument-hint: <TemplatePath> [-Mode overview|query|fields|links|calculated|resources|params|variant|templates|trace|full] [-Name <dataset|variant|field|group>] [-Raw]
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /skd-info — Анализ схемы компоновки данных

## MCP routing

- Preferred path: use MCP `unica` tool `unica.skd.info`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.skd.info`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Читает Template.xml схемы компоновки данных (СКД) и выводит компактную сводку. Заменяет необходимость читать тысячи строк XML.

В `overview` и `full` показывает `Поддержка` для объекта-владельца макета по `Ext/ParentConfigurations.bin`. Режим `query` остаётся пригодным для round-trip текста запроса; support-state используй как риск перед `unica.skd.edit`.

## MCP параметры

| Параметр | Описание |
|----------|----------|
| `TemplatePath` | Путь к Template.xml или каталогу макета (авто-резолв в `Ext/Template.xml`) |
| `Mode` | Режим анализа (по умолчанию `overview`) |
| `Name` | Имя набора (query), поля (fields/calculated/resources/trace), варианта (variant) или группировки/поля (templates) |
| `Batch` | Номер пакета запроса, 0 = все (только query) |
| `Raw` | Только для `Mode=query`: сырой текст запроса целиком, без заголовков/оглавления/разделителей пакетов. Используй для выгрузки в SQL-файл и возврата через `unica.skd.edit` / `set-query @file` |
| `Limit` / `Offset` | Пагинация (по умолчанию 150 строк; `Raw` не усекается) |
| `OutFile` | Записать результат в файл (UTF-8 BOM) |

### Overview: точка входа

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.skd.info",
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
    "name": "unica.skd.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>",
      "Mode": "query",
      "Name": "НоменклатураСЦенами"
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
    "name": "unica.skd.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>",
      "Mode": "query",
      "Name": "ДанныеТ13",
      "Batch": 3
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
    "name": "unica.skd.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>",
      "Mode": "query",
      "Name": "ДанныеТ13",
      "Raw": true,
      "OutFile": "query.sql"
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
    "name": "unica.skd.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>",
      "Mode": "fields",
      "Name": "КадастроваяСтоимость"
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
    "name": "unica.skd.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>",
      "Mode": "calculated",
      "Name": "КоэффициентКи"
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
    "name": "unica.skd.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>",
      "Mode": "resources",
      "Name": "СуммаНалога"
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
    "name": "unica.skd.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>",
      "Mode": "trace",
      "Name": "Коэффициент Ки"
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
    "name": "unica.skd.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>",
      "Mode": "variant",
      "Name": 1
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
    "name": "unica.skd.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>",
      "Mode": "templates"
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
    "name": "unica.skd.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<путь>",
      "Mode": "templates",
      "Name": "ВидНалоговойБазы"
    }
  }
}
```

## Режимы

| Режим | Без `-Name` | С `-Name` |
|-------|-------------|-----------|
| `overview` | Навигационная карта схемы + подсказки Next | — |
| `query` | — | Текст запроса набора (с оглавлением батчей) |
| `fields` | Карта: имена полей по наборам | Деталь поля: набор, тип, роль, формат |
| `links` | Все связи наборов | — |
| `calculated` | Карта: имена вычисляемых полей | Выражение + заголовок + ограничения |
| `resources` | Карта: имена ресурсов (`*` = групповые формулы) | Формулы агрегации по группировкам |
| `params` | Таблица параметров: тип, значение, видимость | — |
| `variant` | Список вариантов | Структура группировок + фильтры + вывод |
| `templates` | Карта привязок шаблонов (field/group) | Содержимое шаблона: строки, ячейки, выражения |
| `trace` | — | Полная цепочка: набор → вычисление → ресурс |
| `full` | Полная сводка: overview + query + fields + resources + params + variant | — |

Паттерн: без `-Name` — карта/индекс, с `-Name` — деталь конкретного элемента. Режим `full` объединяет 6 ключевых режимов в один вызов.

## Типичный workflow

1. `overview` — понять структуру, увидеть подсказки
2. `trace -Name <поле>` — узнать как считается колонка отчёта (от заголовка до запроса за один вызов)
3. `query -Name <набор>` — посмотреть текст SQL-запроса
4. `variant -Name <N>` — посмотреть группировки и фильтры варианта

Переработка запроса (round-trip): `Mode=query`, `Name=<набор>`, `"Raw": true`, `OutFile=q.sql` ->
правка `q.sql` -> `unica.skd.edit` с `Operation=set-query`, `Value="@q.sql"`.
`Raw` отдаёт запрос целиком без декораций, поэтому выгрузка и возврат точны, включая многопакетные запросы с временными таблицами.

Подробные примеры вывода каждого режима — в `modes-reference.md`.

## Верификация

### Overview: точка входа

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.skd.info",
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
    "name": "unica.skd.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "<path>",
      "Mode": "trace",
      "Name": "<field>"
    }
  }
}
```
