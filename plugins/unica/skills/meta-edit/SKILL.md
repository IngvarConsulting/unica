---
name: meta-edit
description: Типизированное атомарное редактирование существующего объекта метаданных 1С по логическому адресу.
argument-hint: <sourceSet> <metadataPath> <operations>
allowed-tools:
  - Read
  - Glob
---

# /meta-edit — структурное редактирование метаданных

## MCP routing

- Preferred path: use MCP `unica` tool `unica.meta.edit`.
- Выбирайте объект только через `sourceSet + metadataPath`.
- Передавайте непустой упорядоченный массив `operations`; все элементы одного
  вызова видят результат предыдущих и публикуются одной транзакцией.
- Вызов по умолчанию строит preview. Передавайте `dryRun: false` только когда
  пользователь явно попросил применить изменение.
- Проверяйте `data.validation` и вложенные `data.validation.diagnostics`;
  успешный результат не дублируется текстовым выводом.
- Vendor support guard выполняется внутри `unica`. Для закрытого объекта
  используйте CFE/release-support flow, а не прямую правку служебных файлов.

Поддерживаются пять значений `op`: `setProperties`, `add`, `update`, `remove`
и `editRelations`. Для коллекционных операций задаются `collection` и
структурные `elements` либо `names`; связи задаются через `relation`, `mode` и
`targets`. Строковая грамматика операций и физические файлы определений в
публичный контракт не входят.

### Изменить свойства

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.edit",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "Catalog.Контрагенты",
      "operations": [
        {"op": "setProperties", "values": {"Comment": "Проверено"}}
      ],
      "dryRun": true
    }
  }
}
```

### Добавить типизированный реквизит

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.edit",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "Catalog.Контрагенты",
      "operations": [
        {
          "op": "add",
          "collection": "attributes",
          "elements": [
            {
              "name": "Комментарий",
              "type": {
                "variants": [
                  {"kind": "string", "length": 200, "allowedLength": "variable"}
                ]
              }
            }
          ]
        }
      ],
      "dryRun": true
    }
  }
}
```

Старые companion-файлы в этом каталоге временно сохраняются для Task 11 fact
audit, но не описывают публичный MCP и не должны использоваться как маршрут.
