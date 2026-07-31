---
name: subsystem-info
description: Анализ структуры подсистемы 1С из XML-выгрузки — состав, дочерние подсистемы, командный интерфейс, дерево иерархии. Используй для изучения структуры подсистем и навигации по конфигурации
argument-hint: <SubsystemPath> [-Mode overview|content|ci|tree|full] [-Name <элемент>]
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /subsystem-info — Структура подсистемы 1С

## MCP routing

- Preferred path: use MCP `unica` tool `unica.subsystem.info`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.subsystem.info`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Читает XML подсистемы из выгрузки конфигурации 1С и выводит компактное описание структуры.

В `overview` и `full` показывает `Поддержка` подсистемы по `Ext/ParentConfigurations.bin`. Используй строку поддержки как guardrail перед `unica.subsystem.edit` или `unica.interface.edit`.

## MCP параметры

| Параметр | Описание |
|----------|----------|
| `SubsystemPath` | XML подсистемы, её каталог — или каталог `Subsystems` целиком |

Инструмент отвечает про то, на что указали: файл или каталог подсистемы дают её
описание, каталог `Subsystems` — дерево иерархии. `Mode`, `Name`, `Limit` и
`Offset` сняты: они выбирали срез печатного отчёта, которого больше нет.

## Поля `data`

Для одной подсистемы:

| Поле | Что содержит |
|------|--------------|
| `name`, `synonym`, `comment`, `explanation`, `picture` | Идентичность и оформление; отсутствующее — `null` |
| `includeInCommandInterface`, `useOneCommand` | Свойства командного интерфейса подсистемы |
| `support` | Поддержка по `Ext/ParentConfigurations.bin` |
| `content` | Состав: полные имена объектов |
| `groups` | Состав, сгруппированный по виду объекта |
| `children` | Имена дочерних подсистем |
| `commandInterface` | `visibility`, `placement` и `order`, либо `null`, если `CommandInterface.xml` нет |

Для каталога `Subsystems`:

| Поле | Что содержит |
|------|--------------|
| `tree` | Корневые подсистемы: `name`, `content` со счётчиком состава и вложенные `children` |

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.info",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "src/Subsystems/Продажи"
    }
  }
}
```

## Примеры

### Состав подсистемы

`data.content` даёт полные имена объектов, `data.groups` — те же объекты по
видам, поэтому отбор «только документы» делается по массиву.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.info",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "Subsystems/Администрирование.xml"
    }
  }
}
```

### Командный интерфейс подсистемы

`data.commandInterface` равен `null`, когда файла нет — это не то же самое, что
интерфейс, который ничего не скрывает.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.info",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "Subsystems/Продажи.xml"
    }
  }
}
```

### Дерево подсистем

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.info",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "Subsystems"
    }
  }
}
```

### Дерево ветки

Каталог `Subsystems` внутри подсистемы даёт поддерево этой ветки.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.info",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "Subsystems/Продажи/Subsystems"
    }
  }
}
```
