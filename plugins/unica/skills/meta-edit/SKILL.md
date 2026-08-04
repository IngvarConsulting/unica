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
- Успешный и предметно неуспешный `tools/call` возвращает `structuredContent`;
  `isError == !structuredContent.ok`. Проверяйте
  `structuredContent.data.validation` и вложенные диагностики;
  `content[0].text` не является вторым контрактом результата.
- Preview возвращает нормализованные семантические
  `structuredContent.data.effects` по `operationIndex`, а не полный XML.
- Vendor support guard выполняется внутри `unica`. Для закрытого объекта
  используйте CFE/release-support flow, а не прямую правку служебных файлов.

Поддерживаются пять значений `op`: `setProperties`, `add`, `update`, `remove`
и `editRelations`. Для коллекционных операций задаются `collection` и
структурные `elements` либо `names`; связи задаются через `relation`, `mode` и
`targets`. Допустимые свойства, коллекции, виды типов и связей берите из
опубликованной схемы операции. Общие прикладные правила находятся в
[соглашениях по метаданным](../../references/platform/metadata-conventions.md).

Не переносите поля снятого Meta JSON DSL по сходству имён. В частности:

- один вызов изменяет один объект; batch нескольких объектов разбивается на
  отдельные `unica.meta.add`, а `meta.add.operations` атомарен только вместе с
  создаваемым объектом;
- вложенные URL-шаблоны и методы HTTP-сервиса, операции и параметры
  Web-сервиса, расписания, реквизиты адресации, учётные признаки и ссылки без
  опубликованного relation/property-варианта не имеют типизированного writer;
- shorthand-флаги `index`, `indexAdditional`, `nonneg`, `master`,
  `mainFilter`, `denyIncomplete`, `useInTotals` нельзя упаковывать в строку;
- не подставляйте compound-значение в `setProperties` как строку и не создавайте
  временный `DefinitionFile`. Если схема не представляет сценарий, остановитесь
  и явно сообщите, что оставшийся шаг выполняется в Designer.

Доказанные переходы для прежних коллекций: значения перечисления добавляются
как `collection: "enumValues"`, владельцы справочника и движения документа
меняются через `editRelations` с `relation: "owners"` или
`relation: "registerRecords"`. Скалярные свойства используйте только под
PascalCase-именем и с enum-значением, опубликованным текущей схемой.

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

### Изменить и удалить реквизиты табличной части

`scope.tabularSection` ограничивает обе операции реквизитами существующей
табличной части, а не корневыми реквизитами документа.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.edit",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "Document.ЗаказПокупателя",
      "operations": [
        {
          "op": "update",
          "collection": "attributes",
          "scope": {"tabularSection": "Товары"},
          "elements": [
            {
              "name": "Количество",
              "synonym": "Количество товара",
              "required": true
            }
          ]
        },
        {
          "op": "remove",
          "collection": "attributes",
          "scope": {"tabularSection": "Товары"},
          "names": ["УстаревшийРеквизит"]
        }
      ],
      "dryRun": true
    }
  }
}
```

### Изменить типизированную связь

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.edit",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "Document.ЗаказПокупателя",
      "operations": [
        {
          "op": "editRelations",
          "relation": "basedOn",
          "mode": "replace",
          "targets": [
            {"metadataPath": "Document.СчетПокупателю"}
          ]
        }
      ],
      "dryRun": true
    }
  }
}
```
