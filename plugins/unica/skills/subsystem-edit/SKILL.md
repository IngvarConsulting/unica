---
name: subsystem-edit
description: Точечное редактирование подсистемы 1С. Используй когда нужно добавить или удалить объекты из подсистемы, управлять дочерними подсистемами или изменить свойства
argument-hint: <at> <ops>
allowed-tools:
  - Read
  - Glob
---

# /subsystem-edit — редактирование подсистемы 1С

## MCP routing

- Preferred path: use MCP `unica` tool `unica.apply` с операциями
  `content.add`, `content.remove`, `childSubsystem.add`,
  `childSubsystem.remove` и `props.set`.
- Do not call internal MCP/CLI adapters directly. They are hidden behind
  `unica` and synchronized by the orchestrator.
- Подсистему называет адрес: `args.at` вида `<набор>:Subsystem.<Имя>`.
  Дочерняя — продолжением того же адреса. Путь к XML наружу не выходит; из
  диффа или лога его переводит аварийный `unica.resolve`.
- Всегда сначала `dryRun: true`. Применяй `dryRun: false` только когда
  пользователь явно попросил внести именно эту правку, и только с `ifRev` из
  предпросмотра.
- Проверка поддержки поставщика работает внутри `unica`. Если она блокирует
  заблокированный объект на поддержке, предпочитай расширение или явный план
  смены состояния поддержки, а не правку метаданных поддержки напрямую.

## Операции

| Операция | `args` | Что делает |
|---|---|---|
| `content.add` | `items: [{object}]` | Добавляет объекты в состав |
| `content.remove` | `items: [{object}]` | Удаляет объекты из состава |
| `childSubsystem.add` | `items: [{name}]` | Заводит дочернюю подсистему |
| `childSubsystem.remove` | `items: [{name}]` | Удаляет дочернюю подсистему |
| `props.set` | `values: {…}` | Меняет свойства: `Synonym`, `IncludeInCommandInterface`, `UseOneCommand` и прочие |

Элемент списка можно писать строкой вместо объекта. Операции применяются по
порядку и публикуются одной транзакцией: либо все, либо ни одной. Повтор
эквивалентной операции даёт `changed: false` без записи.

Заведение дочерней подсистемы создаёт и её собственный файл, и ссылку на неё у
родителя — это одна правка, а не две.

## Порядок

1. Найди подсистему: `unica.search {corpus: "names", kind: "Subsystem"}`.
2. Прочти её: `unica.view {at}` — состав и дочерние лежат в ветвях.
3. Предпросмотр: `unica.apply` с `dryRun: true`; ответ несёт план и `ifRev`.
4. Применение: тот же вызов с `dryRun: false` и этим `ifRev`.
5. Проверка: `unica.check {at}`.

## Примеры

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
      ],
      "dryRun": true
    }
  }
}
```

### Дочерняя подсистема и свойство

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
            "items": [{"name": "Заказы"}]
          }
        },
        {
          "op": "props.set",
          "args": {
            "at": "main:Subsystem.Продажи",
            "values": {"IncludeInCommandInterface": false}
          }
        }
      ],
      "dryRun": false,
      "ifRev": "<rev из предпросмотра>"
    }
  }
}
```
