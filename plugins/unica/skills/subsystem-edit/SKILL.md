---
name: subsystem-edit
description: Создание и точечное редактирование подсистемы 1С. Используй когда нужно добавить подсистему (раздел) в конфигурацию, добавить или удалить объекты из подсистемы, управлять дочерними подсистемами или изменить свойства
argument-hint: <at> <ops>
allowed-tools:
  - Read
  - Glob
---

# /subsystem-edit — создание и редактирование подсистемы 1С

## MCP routing

- Preferred path: use MCP `unica` tool `unica.apply` с операциями
  `subsystem.create`, `content.add`, `content.remove`, `childSubsystem.add`,
  `childSubsystem.remove` и `props.set`.
- Do not call internal MCP/CLI adapters directly. They are hidden behind
  `unica` and synchronized by the orchestrator.
- Подсистему называет адрес: `args.at` вида `<набор>:Subsystem.<Имя>`.
  Дочерняя — продолжением того же адреса. Путь к XML наружу не выходит; из
  диффа или лога его переводит аварийный `unica.resolve`.
- Сначала вызови `unica.apply` с `at` и `ops`: это план без записи.
  Когда пользователь поручил внести эту правку, вызови `unica.apply` только
  с `executionToken` из `data.executionToken` успешного плана.
- Проверка поддержки поставщика работает внутри `unica`. Если она блокирует
  заблокированный объект на поддержке, предпочитай расширение или явный план
  смены состояния поддержки, а не правку метаданных поддержки напрямую.

## Создание подсистемы

Подсистему верхнего уровня создаёт `subsystem.create` с `args.at` вида
`<набор>:Configuration`; имя новой подсистемы лежит в `values.name`: адреса,
которого ещё нет, назвать нельзя. Вложенную подсистему заводит
`childSubsystem.add` на адресе родителя `<набор>:Subsystem.<Родитель>`.
Каталогов выгрузки и путей к XML родителя тут нет — их место занял адрес.

Создание заводит и файл подсистемы, и запись о ней у родителя. Это одна
правка, а не две. Минимально нужно только имя, всё остальное — умолчания
платформы. Синоним, свойства и состав новой подсистемы задают `props.set`
и `content.add` на её адресе в том же пакете: подсистема со своим составом
и свойствами появляется целиком либо не появляется вовсе.

**Картинка подсистемы.** `props.set` картинку не задаёт: свойство `Picture`
получает отказ `bad_value`, и весь пакет не публикуется. Это пробел
контракта; не подставляй `Picture` в `values`.

## Операции

| Операция | `args` | Что делает |
|---|---|---|
| `subsystem.create` | `values: {name}` на `<набор>:Configuration` | Создаёт подсистему верхнего уровня |
| `content.add` | `items: [{object}]` | Добавляет объекты в состав |
| `content.remove` | `items: [{object}]` | Удаляет объекты из состава |
| `childSubsystem.add` | `items: [{name}]` | Заводит дочернюю подсистему |
| `childSubsystem.remove` | только `at` дочерней | Исключает дочернюю подсистему из состава родителя |
| `props.set` | `values: {…}` | Меняет свойства: `Synonym`, `Comment`, `Explanation`, `IncludeHelpInContents`, `IncludeInCommandInterface`, `UseOneCommand` |

`childSubsystem.remove` вызывается на адресе самой дочерней подсистемы:
`<набор>:Subsystem.<Родитель>.Subsystem.<Имя>`. Имя дочерней в `items` или
`values` получает отказ `bad_value`, а `next` ведёт к её адресу.

Элемент списка можно писать строкой вместо объекта. Операции применяются по
порядку и публикуются одной транзакцией: либо все, либо ни одной. Повтор
эквивалентной операции даёт `changed: false` без записи.

Заведение дочерней подсистемы создаёт и её собственный файл, и ссылку на неё у
родителя — это одна правка, а не две.

Изменение состава или дочерних подсистем сохраняет ссылку картинки и
`Picture.LoadTransparent`, включая отсутствие этого свойства и картинку без
ссылки. Новая дочерняя подсистема получает пустую картинку. Генератор не
добавляет служебные `&#13;` в XML.

`props.set` не поддерживает `LoadTransparent` и `Picture.LoadTransparent`:
предпросмотр и применение отклоняют эти свойства до записи.

## Порядок

1. Найди подсистему: `unica.search {corpus: "names", kind: "Subsystem"}`.
   Для новой подсистемы имя набора даёт `unica.view {}`.
2. Прочти её: `unica.view {at}`. Состав лежит в ветви `Relation`
   (`relation: "content"`), дочерние — в ветви `Subsystem`; вложенная
   подсистема читается по адресу `<набор>:Subsystem.<Родитель>.Subsystem.<Имя>`.
3. Предпросмотр: `unica.apply` с `at` и `ops`; ответ несёт план и `data.executionToken`.
4. Применение: вызов только с `executionToken` из `data.executionToken` успешного плана.
5. Проверка: `unica.check {at}`.

## Примеры

### Новая подсистема с составом и свойствами

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
          "op": "subsystem.create",
          "args": {
            "at": "main:Configuration",
            "values": {"name": "Продажи"}
          }
        },
        {
          "op": "props.set",
          "args": {
            "at": "main:Subsystem.Продажи",
            "values": {
              "Synonym": "Продажи",
              "Comment": "Раздел продаж",
              "IncludeInCommandInterface": true
            }
          }
        },
        {
          "op": "content.add",
          "args": {
            "at": "main:Subsystem.Продажи",
            "items": [{"object": "Catalog.Товары"}, {"object": "Report.Продажи"}]
          }
        }
      ]
    }
  }
}
```

### Состав: добавить и убрать за один раз

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
          "op": "content.add",
          "args": {
            "at": "main:Subsystem.Продажи",
            "items": [{"object": "Catalog.Товары"}, {"object": "Report.Продажи"}]
          }
        },
        {
          "op": "content.remove",
          "args": {
            "at": "main:Subsystem.Продажи",
            "items": [{"object": "Report.Старый"}]
          }
        }
      ]
    }
  }
}
```

### Дочерняя подсистема и свойство

Сначала получи план этого изменения:

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
          "op": "childSubsystem.add",
          "args": {
            "at": "main:Subsystem.Продажи",
            "items": [
              {
                "name": "Заказы"
              }
            ]
          }
        },
        {
          "op": "props.set",
          "args": {
            "at": "main:Subsystem.Продажи",
            "values": {
              "IncludeInCommandInterface": false
            }
          }
        }
      ]
    }
  }
}
```

Для исполнения передай `data.executionToken` из этого успешного плана:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "executionToken": "<data.executionToken из успешного плана>"
    }
  }
}
```
