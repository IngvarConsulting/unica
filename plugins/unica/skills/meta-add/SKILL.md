---
name: meta-add
description: Создать и при необходимости сразу настроить один объект метаданных 1С через атомарную типизированную операцию.
argument-hint: <sourceSet> <kind> <name> [operations] [dryRun]
allowed-tools:
  - Read
  - Glob
---

# /meta-add — создание объекта метаданных

## MCP routing

- Preferred path: use MCP `unica` tool `unica.meta.add`.
- Передайте логический набор исходников `sourceSet`, поддерживаемый вид `kind`,
  имя `name`, при необходимости непустой массив `operations` и `dryRun`.
- Вызов по умолчанию строит preview. Передавайте `dryRun: false` только когда
  пользователь явно попросил применить изменение.
- Когда объект должен быть настроен уже при создании, передайте `operations`
  того же закрытого типизированного контракта, что у `unica.meta.edit`. Шаблон,
  операции, дочерние ресурсы и регистрация публикуются одной транзакцией.
- Для изменений уже существующего объекта используйте `unica.meta.edit`.
- Результат и обязательная внутренняя проверка находятся в типизированном
  `data.validation`.

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
          "elements": [{"name": "ВнешнийКод"}]
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
