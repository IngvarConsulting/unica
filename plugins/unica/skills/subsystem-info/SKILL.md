---
name: subsystem-info
description: Анализ структуры подсистемы 1С из XML-выгрузки — состав, дочерние подсистемы, командный интерфейс, дерево и плоские списки эффективных ролей. Используй для изучения структуры подсистем и навигации по конфигурации
argument-hint: <SubsystemPath>
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
описание, каталог `Subsystems` — дерево и два плоских списка. `Mode`, `Name`, `Limit` и
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
| `functionalSubsystems` | Функциональные подсистемы в pre-order регистрации |
| `interfaceSubsystems` | Интерфейсные подсистемы в pre-order регистрации |

Оба списка содержат `SubsystemAddress` в диалекте БСП: программные имена от
корня через точку, например `СтандартныеПодсистемы.Обсуждения`, без префикса
`Subsystem.` и без повторения вида. Узел интерфейсный, только если его
`IncludeInCommandInterface` и флаги всех предков равны `true`; остальные узлы
функциональные. Вместе списки исчерпывают зарегистрированные узлы `tree`
(ADR-0033, `INV-SOURCE-SUBSYSTEM-TOPOLOGY`).

Три проекции строятся только от `Configuration/ChildObjects` и рекурсивных
`Subsystem/ChildObjects`. Незарегистрированный XML игнорируется. Если
зарегистрированный дескриптор отсутствует, повреждён, связан символической
ссылкой или не содержит единственный канонический `IncludeInCommandInterface`,
инструмент возвращает ошибку вместо частичного дерева или пустых списков.

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

Типизированное `data` для каталога имеет вид:

```json
{
  "tree": [
    {
      "name": "СтандартныеПодсистемы",
      "content": 0,
      "children": [
        {"name": "Обсуждения", "content": 1, "children": []}
      ]
    }
  ],
  "functionalSubsystems": ["Служебные"],
  "interfaceSubsystems": [
    "СтандартныеПодсистемы",
    "СтандартныеПодсистемы.Обсуждения"
  ]
}
```

### Дерево ветки

Каталог `Subsystems` внутри подсистемы даёт поддерево этой ветки. Оба плоских
списка ограничиваются узлами поддерева, но адреса остаются абсолютными от корня
конфигурации.

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
