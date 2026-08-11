---
name: subsystem-info
description: Анализ структуры подсистемы 1С из XML-выгрузки — состав, дочерние подсистемы, командный интерфейс и полное или сфокусированное дерево. Используй для изучения структуры подсистем и навигации по конфигурации
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

Поле `support` показывает поддержку подсистемы по
`Ext/ParentConfigurations.bin`. Используй его как guardrail перед
`unica.subsystem.edit` или `unica.interface.edit`.

## MCP параметры

| Параметр | Описание |
|----------|----------|
| `SubsystemPath` | Каталог `Subsystems`, зарегистрированный XML подсистемы или самостоятельный незарегистрированный XML |
| `sourceSet`     | Имя набора исходников из `v8project.yaml`                                                            |
| `metadataPath`  | Логический адрес; без него читается всё зарегистрированное дерево                                    |

Селектор цели ровно один: либо `sourceSet` + `metadataPath`, либо
`SubsystemPath`. Оба сразу отклоняются кодом `selector_conflict` (ADR-0049).

Типизированный результат определяется формой цели в `SubsystemPath`:

- каталог `Subsystems` возвращает полное зарегистрированное `tree` в пределах
  этого каталога;
- зарегистрированный XML возвращает сфокусированное `tree`: цепочку от корня до
  выбранной подсистемы и всех её потомков;
- самостоятельный незарегистрированный XML возвращает только локальные поля без
  `tree`.

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
| `tree` | Для зарегистрированной подсистемы — цепочка от корня до неё, а потом полное дерево её потомков |

Для каталога `Subsystems`:

| Поле | Что содержит |
|------|--------------|
| `tree` | Корневые подсистемы: `name`, `content` со счётчиком состава и вложенные `children` |

Дерево строится только от `Configuration/ChildObjects` и рекурсивных
`Subsystem/ChildObjects` (ADR-0036, `INV-SOURCE-SUBSYSTEM-TOPOLOGY`). Для
конкретной зарегистрированной подсистемы оно сохраняет единственную цепочку
предков от корня, выбранный узел и всех его потомков. Так в одном результате
видно и положение подсистемы в конфигурации, и вся вложенная в неё структура.
Незарегистрированный самостоятельный XML сохраняет локальное описание, но не
выдаёт недоказанное дерево. Если
зарегистрированный дескриптор отсутствует, повреждён, связан символической
ссылкой или не содержит единственный канонический `IncludeInCommandInterface`,
инструмент возвращает `provider_unavailable` вместо частичного дерева; отмена
после захвата снимка тоже не скрывается, и результат не выдаётся как доказанно
полный. Отмена и истечение срока сохраняют собственную типизированную семантику
сбоя и не маркируются как `provider_unavailable`.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.info",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "src/Subsystems/Продажи.xml"
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
  ]
}
```

### Контекст конкретной подсистемы

Для зарегистрированного файла `Subsystems/Продажи/Subsystems/ОптовыеПродажи.xml`
поле `tree` показывает цепочку от корня до выбранной подсистемы, а потом всех её
потомков. Соседние ветки в такой сфокусированный результат не входят:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.info",
    "arguments": {
      "cwd": "<workspace>",
      "SubsystemPath": "Subsystems/Продажи/Subsystems/ОптовыеПродажи.xml"
    }
  }
}
```

Типизированное `data` для зарегистрированной подсистемы имеет вид:

```json
{
  "name": "ОптовыеПродажи",
  "children": ["Возвраты"],
  "tree": [
    {
      "name": "Продажи",
      "content": 1,
      "children": [
        {
          "name": "ОптовыеПродажи",
          "content": 2,
          "children": [
            {"name": "Возвраты", "content": 1, "children": []}
          ]
        }
      ]
    }
  ]
}
```

## Логический адрес вместо пути

`unica.subsystem.info` принимает либо логический селектор, либо файловый путь —
ровно один из двух. Оба сразу отклоняются кодом `selector_conflict`.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.subsystem.info",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "<имя набора>",
      "metadataPath": "Subsystem.<Подсистема>"
    }
  }
}
```

Имя набора даёт `unica.project.map`, адрес — `unica.source.resolve`, а `unica.source.locate` переводит
в адрес путь, найденный иначе. Файловый селектор сохраняется до
отдельного среза его снятия (ADR-0049).
