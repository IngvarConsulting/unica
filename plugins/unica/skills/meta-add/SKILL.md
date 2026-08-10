---
name: meta-add
description: Создать и при необходимости сразу настроить один объект метаданных 1С через атомарную типизированную операцию.
argument-hint: <sourceSet> <kind> <name> [operations] [dryRun]
allowed-tools:
  - Read
  - Glob
---

# /unica:meta-add — создание объекта метаданных

## MCP routing

- Preferred path: use MCP `unica` tool `unica.meta.add`.
- Передайте логический набор исходников `sourceSet`, поддерживаемый вид `kind`,
  имя `name`, при необходимости непустой массив `operations` и `dryRun`.
- Вызов по умолчанию строит preview. Передавайте `dryRun: false` только когда
  пользователь явно попросил применить изменение.
- Когда объект должен быть настроен уже при создании, передайте `operations`
  того же закрытого типизированного контракта, что у `unica.meta.edit`. Шаблон,
  операции, дочерние ресурсы и регистрация публикуются одной транзакцией.
- Источник `EventSubscription` задаётся при создании тем же вариантом
  `editRelations`, что и при последующем редактировании: `relation: "source"`,
  `mode: "replace"` и типизированный массив `targets`. Отдельного шестого
  значения `op` для источника нет. В наборе исходников уже должна быть
  экспортная процедура общего модуля с двумя параметрами: минимальный шаблон
  выбирает её как обработчик подписки до применения `operations`.
- Для изменений уже существующего объекта используйте `unica.meta.edit`.
- Успешный и предметно неуспешный `tools/call` возвращает `structuredContent`;
  `isError == !structuredContent.ok`. Читайте проверку из
  `structuredContent.data.validation`; `content[0].text` не является вторым
  контрактом результата.
- Preview описывает изменение семантическими
  `structuredContent.data.effects`, а не возвращает полный XML объекта.
- `sourceSet` — это имя набора исходников из `v8project.yaml`, а не
  константа. Получите его через `unica.project.map`; `"main"` в примерах
  ниже — иллюстрация, а не значение по умолчанию.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.add",
    "arguments": {
      "sourceSet": "main",
      "kind": "Catalog",
      "name": "НовыйСправочник",
      "operations": [
        {
          "op": "setProperties",
          "values": {"Comment": "Создан и настроен одним вызовом"}
        },
        {
          "op": "add",
          "collection": "attributes",
          "elements": [
            {
              "name": "ВнешнийКод",
              "type": {
                "variants": [
                  {"kind": "string", "length": 36, "allowedLength": "variable"}
                ]
              },
              "required": true
            }
          ]
        }
      ],
      "dryRun": true
    }
  }
}
```

### Создать подписку с типизированным источником

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.add",
    "arguments": {
      "sourceSet": "main",
      "kind": "EventSubscription",
      "name": "ОбработкаБулевыхИзменений",
      "operations": [
        {
          "op": "editRelations",
          "relation": "source",
          "mode": "replace",
          "targets": [
            {"kind": "boolean"}
          ]
        }
      ],
      "dryRun": true
    }
  }
}
```

Имена, синонимы, представления и правила заполнения сверяйте с
[общими соглашениями Unica](../../references/platform/metadata-conventions.md).
Полный список `kind` берите из опубликованной схемы `unica.meta.add`: схема
является контрактом, поэтому перечень не дублируется в скилле.
