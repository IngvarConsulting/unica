---
name: subsystem-compile
description: Создать подсистему 1С — XML-исходники из типизированного определения. Используй когда нужно добавить подсистему (раздел) в конфигурацию
argument-hint: <at> <name> [content]
allowed-tools:
  - Read
  - Glob
---

# /subsystem-compile — создание подсистемы

## MCP routing

- Preferred path: use MCP `unica` tool `unica.apply` с операциями
  `subsystem.create`, `props.set` и `content.add`.
- Do not call internal MCP/CLI adapters directly. They are hidden behind
  `unica` and synchronized by the orchestrator.
- Всегда сначала `dryRun: true`. Применяй `dryRun: false` только когда
  пользователь явно попросил внести именно эту правку, и только с `ifRev` из
  предпросмотра.
- Проверка поддержки поставщика работает внутри `unica`.

**Куда метит создание.** Подсистемы верхнего уровня — `args.at` вида
`<набор>:Configuration`. Вложенной подсистемы — адрес родителя
`<набор>:Subsystem.<Родитель>`. Имя новой подсистемы лежит в `values.name`:
адреса, которого ещё нет, назвать нельзя. Каталогов выгрузки и путей к XML
родителя тут нет — их место занял адрес.

Создание заводит и файл подсистемы, и запись о ней у родителя. Это одна
правка, а не две.

## Что задаётся чем

| Что | Операция | `args` |
|---|---|---|
| Сама подсистема | `subsystem.create` | `values: {name}` |
| Синоним, комментарий, пояснение, картинка, `IncludeInCommandInterface`, `UseOneCommand` | `props.set` | `values: {…}` |
| Состав | `content.add` | `items: [{object}]` |

Минимально нужно только имя. Всё остальное — умолчания платформы.

Операции одного вызова публикуются одной транзакцией: подсистема со своим
составом и свойствами появляется целиком либо не появляется вовсе.

## Примеры

### Минимальная подсистема

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
            "values": {"name": "Тест"}
          }
        }
      ],
      "dryRun": true
    }
  }
}
```

### С составом и свойствами

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
              "Picture": "CommonPicture.Продажи",
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
      ],
      "dryRun": false,
      "ifRev": "<rev из предпросмотра>"
    }
  }
}
```

### Вложенная подсистема

Родителя называет адрес, а не путь к его XML.

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
            "items": [{"name": "Дочерняя"}]
          }
        }
      ],
      "dryRun": true
    }
  }
}
```
