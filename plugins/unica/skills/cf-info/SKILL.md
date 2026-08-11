---
name: cf-info
description: Анализ структуры конфигурации 1С — свойства, состав, счётчики объектов. Используй для обзора конфигурации — какие объекты есть, сколько их, какие настройки
argument-hint: <ConfigPath>
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /cf-info — Структура конфигурации 1С

## MCP routing

- Preferred path: use MCP `unica` tool `unica.cf.info`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.cf.info`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Читает Configuration.xml из выгрузки конфигурации и возвращает её описание
типизированными данными в `data` (ADR-0023). Режимов вывода больше нет: ответ
всегда полный, а выбирать нужные поля — дело вызывающего.

## MCP параметры

| Параметр | Описание |
|----------|----------|
| `ConfigPath` | Путь к Configuration.xml или каталогу выгрузки |
| `sourceSet`  | Имя набора исходников из `v8project.yaml`      |

Селектор цели ровно один: либо `sourceSet`, либо `ConfigPath`. Оба сразу
отклоняются кодом `selector_conflict` (ADR-0048).

## Поля `data`

| Поле | Что содержит |
|------|--------------|
| `format` | Версия формата выгрузки из корневого атрибута |
| `name`, `synonym`, `version`, `vendor` | Идентичность конфигурации; отсутствующее значение — `null` |
| `extensionPurpose` | Назначение расширения или `null` для конфигурации |
| `support` | `state`: `notSupported`, `extension`, `removed` или `supported`; `editingEnabled`; `objects` со счётчиками `locked`/`editable`/`removed` |
| `properties` | Совместимость, режим запуска, язык, блокировки, модальность, префикс и остальные свойства корня |
| `childObjects`, `totalObjects` | Состав по видам: `kind` и `count`, плюс общее число |
| `homePage` | Шаблон начальной страницы и её колонки с формами, высотами и ролями, либо `null` |

`support` читается из `Ext/ParentConfigurations.bin`, а прежняя строка
`Поддержка` в отчёте им заменена. Используй его как сигнал
риска перед мутирующими `unica.*`; сам файл поддержки не редактируй.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.info",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigPath": "src/Configuration.xml"
    }
  }
}
```

## Примеры

### Каталог выгрузки вместо файла

Путь можно указать на каталог: инструмент сам найдёт в нём `Configuration.xml`.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.info",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigPath": "src"
    }
  }
}
```

### Проверка состояния поддержки перед доработкой

Читай `data.support`: `state` отличает свою конфигурацию от расширения, снятой
с поддержки и стоящей на ней, а `objects` показывает, сколько объектов на замке,
редактируется и снято.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.info",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigPath": "src"
    }
  }
}
```

### Состав конфигурации

`data.childObjects` даёт пары `kind`/`count`, `data.totalObjects` — общее число.
Счётчик по виду берётся из массива, а не разбором строки отчёта.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.info",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigPath": "src"
    }
  }
}
```

### Начальная страница

`data.homePage` содержит шаблон и обе колонки с формами, высотами и ролями,
либо `null`, если `Ext/HomePageWorkArea.xml` в выгрузке нет.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.info",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigPath": "src"
    }
  }
}
```

### Один вид объектов

Отбор по `kind` делается по массиву, поэтому имя вида не нужно искать в тексте.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.info",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigPath": "src/Configuration.xml"
    }
  }
}
```

## Логический адрес вместо пути

`unica.cf.info` принимает либо логический селектор, либо файловый путь —
ровно один из двух. Оба сразу отклоняются кодом `selector_conflict`.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.info",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "<имя набора>"
    }
  }
}
```

Имя набора даёт `unica.project.map`. Файловый селектор сохраняется до
отдельного среза его снятия (ADR-0048).
