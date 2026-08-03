---
name: meta-add
description: Создать минимальный валидный объект метаданных 1С заданного вида через типизированную операцию, а дальнейшие изменения передать meta-edit.
argument-hint: <sourceSet> <kind> <name> [dryRun]
allowed-tools:
  - Read
  - Glob
---

# /meta-add — создание объекта метаданных

## MCP routing

- Preferred path: use MCP `unica` tool `unica.meta.add`.
- Передайте только логический набор исходников `sourceSet`, поддерживаемый вид
  `kind`, имя `name` и при необходимости `dryRun`.
- Вызов по умолчанию строит preview. Передавайте `dryRun: false` только когда
  пользователь явно попросил применить изменение.
- Сложный объект создаётся двумя отдельными шагами: валидный минимальный объект
  через `unica.meta.add`, затем структурные изменения через `unica.meta.edit`.
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
      "dryRun": true
    }
  }
}
```

Имена, синонимы, представления и правила заполнения сверяйте с
[общими соглашениями Unica](../../references/platform/metadata-conventions.md).
Полный список `kind` берите из опубликованной схемы `unica.meta.add`: схема
является контрактом, поэтому перечень не дублируется в скилле.
