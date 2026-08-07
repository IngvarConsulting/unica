---
name: role-edit
description: Типизированно изменить права существующей роли 1С по логическому адресу, сохранив RLS, шаблоны и остальные права.
argument-hint: <sourceSet> <metadataPath> <operations>
allowed-tools:
  - Read
  - Glob
---

# /unica:role-edit — точечное редактирование прав роли

## MCP routing

- Preferred path: use MCP `unica` tool `unica.role.edit`.
- Выбирайте роль только через `sourceSet + metadataPath` вида `Role.<Имя>`.
  Физический `Rights.xml` — внутренняя деталь разрешителя.
- Передавайте непустой упорядоченный массив `operations`; сейчас его закрытый
  вариант — `setRight` с `objectName`, `right` и булевым `value`.
- Вызов по умолчанию строит preview. Передавайте `dryRun: false` только когда
  пользователь явно попросил применить изменение.
- Читайте `metadataPath`, `changed`, `effects` по `operationIndex`,
  `validation` и `diagnostics` из `structuredContent.data`. `stdout`, diff и
  физический путь не являются контрактом результата.
- Не передавайте снятые top-level поля `RightsPath`, `Path`, `ObjectName`,
  `Name` и `Value`; schema и parser обязаны их отклонить.
- `sourceSet` — имя набора исходников из `v8project.yaml`, а не константа.
  Получите его через `unica.project.map`; `"main"` ниже — только пример.

## Вызов

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.role.edit",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "Role.Демо",
      "operations": [
        {
          "op": "setRight",
          "objectName": "Catalog.Демо",
          "right": "Delete",
          "value": false
        }
      ],
      "dryRun": true
    }
  }
}
```

Операции выполняются последовательно и публикуются одной транзакцией. Writer
проверяет право для вида `objectName` до изменения XML, сохраняет RLS,
templates, глобальные флаги, остальные объектные блоки и права. Повтор
эквивалентной операции даёт `changed: false` без записи.

Для `DataProcessor.*` операция `Use=false` применяет правило платформы:
удаляется весь объектный блок. Это не обобщается на другие виды объектов или
права. Неподдерживаемое сочетание завершается диагностикой без частичной
мутации.
