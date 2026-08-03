---
name: meta-validate
description: Проверить объект метаданных через типизированное чтение. Временное имя маршрута сохраняется до редакционной миграции скиллов.
argument-hint: <sourceSet> <metadataPath>
allowed-tools:
  - Read
  - Glob
---

# /meta-validate — переходный маршрут проверки метаданных

## MCP routing

- Preferred path: use MCP `unica` tool `unica.meta.info`.
- Укажите логические `sourceSet + metadataPath`; файловые пути, batch-строки и
  флаги подробности не входят в контракт.
- Читайте `data.validation` для статуса и диагностик. Та же внутренняя проверка
  обязательна после `unica.meta.add`, `unica.meta.edit` и
  `unica.meta.remove`, поэтому отдельного служебного вызова нет.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "Catalog.Номенклатура"
    }
  }
}
```
