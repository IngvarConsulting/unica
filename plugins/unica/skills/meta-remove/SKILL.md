---
name: meta-remove
description: Безопасно удалить объект метаданных 1С по логическому адресу с анализом ссылок и атомарной публикацией.
argument-hint: <sourceSet> <metadataPath>
allowed-tools:
  - Read
  - Glob
  - AskUserQuestion
---

# /meta-remove — удаление объекта метаданных

## MCP routing

- Preferred path: use MCP `unica` tool `unica.meta.remove`.
- Выбирайте объект только через `sourceSet + metadataPath`.
- Сначала оставьте `dryRun` по умолчанию и изучите типизированный план,
  зависимости и диагностики.
- Обычное применение требует `dryRun: false`. Принудительное применение при
  найденных ссылках разрешено только тройным подтверждением
  `force: true`, `confirm: true`, `dryRun: false`.
- Vendor support guard выполняется до публикации; закрытый объект не обходится
  прямым редактированием служебных файлов.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.remove",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "Catalog.Устаревший",
      "dryRun": true
    }
  }
}
```
