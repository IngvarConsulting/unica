---
name: xdto
description: Просмотреть или точечно изменить схему XDTO-пакета 1С по логическому адресу. Используй для EnterpriseData `valueType`, `objectType` и свойств типов.
argument-hint: <sourceSet> <XDTOPackage.Name> <operations>
allowed-tools:
  - Read
  - Glob
---

# /unica:xdto — XDTO-пакеты 1С

Перед чтением или мутацией сверяй поддерживаемую грамматику и байтовые гарантии
с `../../references/specs/1c-xdto-spec.md`.

## MCP routing

- Используй только MCP `unica`: `unica.xdto.info` читает пакет, а
  `unica.xdto.edit` строит и применяет точечную мутацию.
- Всегда начинай с `unica.xdto.info`, затем перед каждой мутацией вызывай
  `unica.xdto.edit` с `dryRun: true`. Повторяй ровно тот же запрос с
  `dryRun: false` лишь после явного подтверждения пользователя; любое изменение
  аргументов требует нового preview.
- `unica.xdto.edit` принимает непустой упорядоченный массив `operations`
  (ADR-0071). Связное изменение — тип и его свойства — веди одним вызовом:
  операции видят результаты предыдущих, публикация одна, отказ любой операции
  не оставляет частичной записи. Результат несёт по эффекту на операцию с
  `operationIndex`; ошибка элемента называет `operations[<индекс>]`.
- Передавай `sourceSet` и `metadataPath: "XDTOPackage.<Имя>"`. Никогда не
  передавай путь к `XDTOPackages/.../Ext/Package.bin`: он остаётся внутренней
  раскладкой платформенной выгрузки.
- Не вызывай donor-команды compile, decompile или validate и не запускай их
  скриптовые обёртки: публичная граница этого скилла состоит ровно из двух
  нативных инструментов выше.

Виды операций `unica.xdto.edit` — закрытое объединение с тегом `op`:
`addValueType` (`name`, `base`), `addObjectType` (`name`), `addProperty`
(`typeName`, `property`, необязательный `propertyPath`), `removeType` (`name`),
`removeProperty` (`typeName`, `name`, необязательный `propertyPath`). Для
вложенного анонимного типа используй `propertyPath`, например
`"СсылкаНаОбъект"` для `ЛюбаяСсылка`. Writer сохраняет BOM и наблюдённые
переводы строк, а повтор того же добавления возвращает no-op. QName в `base` и
`property.type` передавай с существующим префиксом. Если префикс не виден в
области вставки, writer повторит его объявление локально только при
единственном доказанном соответствии префикса URI во всём пакете;
отсутствующее или противоречивое соответствие отклоняется без угадывания URI.

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
      "operations": [
        {
          "op": "addProperty",
          "typeName": "ЛюбаяСсылка",
          "propertyPath": "СсылкаНаОбъект",
          "property": {
            "name": "Документ_НовыйДокумент",
            "type": "tns:Документ_ЗаказКлиента",
            "minOccurs": 0
          }
        }
      ],
      "dryRun": true
    }
  }
}
```

Связная последовательность — например `addObjectType` и следом `addProperty` к
созданному типу — передаётся тем же массивом `operations` и проверяется одним
preview.

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
      "operations": [
        {
          "op": "addProperty",
          "typeName": "ЛюбаяСсылка",
          "propertyPath": "СсылкаНаОбъект",
          "property": {
            "name": "Документ_НовыйДокумент",
            "type": "tns:Документ_ЗаказКлиента",
            "minOccurs": 0
          }
        }
      ],
      "dryRun": false
    }
  }
}
```
