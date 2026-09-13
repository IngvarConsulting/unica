---
name: interface-edit
description: Настройка командного интерфейса подсистемы 1С. Используй когда нужно скрыть или показать команды, разместить в группах, настроить порядок
argument-hint: <at> <ops>
allowed-tools:
  - Read
  - Glob
---

# /interface-edit — правка командного интерфейса

## MCP routing

- Preferred path: use MCP `unica` tool `unica.apply` с операциями
  `commandVisibility.set`, `commandPlacement.set`, `commandOrder.set`,
  `groupOrder.set` и `subsystemOrder.set`.
- Do not call internal MCP/CLI adapters directly. They are hidden behind
  `unica` and synchronized by the orchestrator.
- Интерфейс называет адрес: `args.at` вида
  `<набор>:Subsystem.<Имя>.Interface`. Пути к `CommandInterface.xml` наружу
  нет; путь из диффа или лога переводит в адрес аварийный `unica.resolve`.
- Всегда сначала `dryRun: true`. Применяй `dryRun: false` только когда
  пользователь явно попросил внести именно эту правку, и только с `ifRev` из
  предпросмотра.

**Команда лежит в аргументах, а не в адресе.** Имя команды в интерфейсе — это
ссылка на команду, живущую в другом месте (`CommonCommand.Печать`,
`Catalog.Валюты.Command.Создать`). Адрес несёт интерфейс, потому что предмет
правки — место, которое подсистема команде отводит, а не сама команда.

## Операции

| Операция | `at` | `args` | Секция файла |
|---|---|---|---|
| `commandVisibility.set` | `…Subsystem.<Имя>.Interface` | `items: [{command, visible}]` | `CommandsVisibility` |
| `commandPlacement.set` | `…Subsystem.<Имя>.Interface` | `items: [{command, group?, placement?}]` | `CommandsPlacement` |
| `commandOrder.set` | `…Subsystem.<Имя>.Interface` | `values: {group?, commands: […]}` | `CommandsOrder` |
| `groupOrder.set` | `…Subsystem.<Имя>.Interface` | `values: {groups: […]}` | `GroupsOrder` |
| `subsystemOrder.set` | `<набор>:Configuration` | `values: {subsystems: […]}` | `SubsystemsOrder` |

**Порядок подсистем верхнего уровня метит в корень конфигурации.** Корневой
документ несёт единственную секцию, поэтому узла у него нет — он описан
свойством корня.

**Порядок задаётся целиком.** `commandOrder.set` и `groupOrder.set` принимают
полную последовательность и заменяют секцию: частичный список некуда вставить,
потому что порядок — отношение между всеми элементами, а не свойство одного.

**Каждая операция правит свою секцию и ничью больше.** В частности, правка
видимости касается только общего значения: переопределения по ролям в том же
блоке остаются нетронутыми. Их число видно в чтении полем `roleOverrides` —
если оно не ноль, `visible` не вся правда о видимости команды.

## Порядок

1. Найди подсистему: `unica.search {corpus: "names", kind: "Subsystem"}`.
2. Прочти интерфейс: `unica.view {at: "…Subsystem.<Имя>.Interface"}` — ветви
   `Command`, `Group` и `Subsystem` показывают команды, порядок групп и
   порядок дочерних подсистем.
3. Предпросмотр: `unica.apply` с `dryRun: true`; ответ несёт план и `ifRev`.
4. Применение: тот же вызов с `dryRun: false` и этим `ifRev`.

## Примеры

### Скрыть одну команду и показать другую

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Subsystem.Продажи",
      "ops": [
        {
          "op": "commandVisibility.set",
          "args": {
            "at": "main:Subsystem.Продажи.Interface",
            "items": [
              {"command": "Catalog.Товары.StandardCommand.OpenList", "visible": false},
              {"command": "Report.Продажи.Command.Отчёт", "visible": true}
            ]
          }
        }
      ],
      "dryRun": true
    }
  }
}
```

### Разместить команду и задать порядок в группе

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Subsystem.Продажи",
      "ops": [
        {
          "op": "commandPlacement.set",
          "args": {
            "at": "main:Subsystem.Продажи.Interface",
            "items": [{"command": "Report.Продажи.Command.Отчёт", "group": "NavigationPanelImportant"}]
          }
        },
        {
          "op": "commandOrder.set",
          "args": {
            "at": "main:Subsystem.Продажи.Interface",
            "values": {
              "group": "NavigationPanelImportant",
              "commands": [
                "Report.Продажи.Command.Отчёт",
                "Catalog.Товары.StandardCommand.OpenList"
              ]
            }
          }
        }
      ],
      "dryRun": false,
      "ifRev": "<rev из предпросмотра>"
    }
  }
}
```

### Порядок подсистем верхнего уровня

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Configuration",
      "ops": [
        {
          "op": "subsystemOrder.set",
          "args": {
            "at": "main:Configuration",
            "values": {"subsystems": ["Subsystem.Продажи", "Subsystem.Склад"]}
          }
        }
      ],
      "dryRun": true
    }
  }
}
```
