---
name: xdto
description: Просмотреть или точечно изменить схему XDTO-пакета 1С по логическому адресу. Используй для EnterpriseData `valueType`, `objectType` и свойств типов.
argument-hint: <sourceSet> <XDTOPackage.Name> [operation]
allowed-tools:
  - Read
  - Glob
---

# /xdto — XDTO-пакеты 1С

Перед чтением или мутацией сверяй поддерживаемую грамматику и байтовые гарантии
с `../../references/specs/1c-xdto-spec.md`.

## MCP routing

- Используй только MCP `unica`: `unica.xdto.info` читает пакет, а
  `unica.xdto.edit` строит и применяет точечную мутацию.
- Всегда начинай с `unica.xdto.info`, затем перед каждой мутацией вызывай
  `unica.xdto.edit` с `dryRun: true`. Повторяй ровно тот же запрос с
  `dryRun: false` лишь после явного подтверждения пользователя; любое изменение
  аргументов требует нового preview.
- Один вызов `unica.xdto.edit` выполняет одну операцию и публикует не более чем
  одну атомарную мутацию. Полный сценарий из нескольких операций веди как
  упорядоченную неатомарную последовательность отдельных пар preview/apply;
  следующий preview строится уже после применения предыдущего шага.
- Передавай `sourceSet` и `metadataPath: "XDTOPackage.<Имя>"`. Никогда не
  передавай путь к `XDTOPackages/.../Ext/Package.bin`: он остаётся внутренней
  раскладкой платформенной выгрузки.
- Не вызывай donor-команды compile, decompile или validate и не запускай их
  скриптовые обёртки: публичная граница этого скилла состоит ровно из двух
  нативных инструментов выше.

`unica.xdto.edit` v1 поддерживает `add-value-type`, `add-object-type`,
`add-property`, `remove-type` и `remove-property`. Для вложенного анонимного
типа используй `propertyPath`, например `"СсылкаНаОбъект"` для
`ЛюбаяСсылка`. Writer сохраняет BOM и наблюдённые переводы строк, а повтор того
же добавления возвращает no-op. QName в `base` и `property.type` передавай с
существующим префиксом. Если префикс не виден в области вставки, writer повторит
его объявление локально только при единственном доказанном соответствии
префикса URI во всём пакете; отсутствующее или противоречивое соответствие
отклоняется без угадывания URI.

## 1. Прочитать логическую цель

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "unica.xdto.info",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "XDTOPackage.EnterpriseData_1_17_3"
    }
  }
}
```

## 2. Построить preview

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "tools/call",
  "params": {
    "name": "unica.xdto.edit",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
      "operation": "add-property",
      "typeName": "ЛюбаяСсылка",
      "propertyPath": "СсылкаНаОбъект",
      "property": {
        "name": "Документ_НовыйДокумент",
        "type": "tns:Документ_ЗаказКлиента",
        "minOccurs": 0
      },
      "dryRun": true
    }
  }
}
```

## 3. Применить только после подтверждения

Только после явного подтверждения пользователя повтори без изменений все
аргументы preview, кроме `dryRun`:

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "tools/call",
  "params": {
    "name": "unica.xdto.edit",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
      "operation": "add-property",
      "typeName": "ЛюбаяСсылка",
      "propertyPath": "СсылкаНаОбъект",
      "property": {
        "name": "Документ_НовыйДокумент",
        "type": "tns:Документ_ЗаказКлиента",
        "minOccurs": 0
      },
      "dryRun": false
    }
  }
}
```
