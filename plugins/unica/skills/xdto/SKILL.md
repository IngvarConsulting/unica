---
name: xdto
description: Просмотреть или точечно изменить схему XDTO-пакета 1С по логическому адресу. Используй для EnterpriseData `valueType`, `objectType` и свойств типов.
argument-hint: <sourceSet> <XDTOPackage.Name> [operation]
allowed-tools:
  - Read
  - Glob
---

# /xdto — XDTO-пакеты 1С

## MCP routing

- Используй только MCP `unica`: `unica.xdto.info` читает пакет, а
  `unica.xdto.edit` строит и применяет точечную мутацию.
- Перед каждой мутацией вызывай `unica.xdto.edit` с `dryRun: true`. Передавай
  `dryRun: false` лишь после явного подтверждения пользователя.
- Передавай `sourceSet` и `metadataPath: "XDTOPackage.<Имя>"`. Никогда не
  передавай путь к `XDTOPackages/.../Ext/Package.bin`: он остаётся внутренней
  раскладкой платформенной выгрузки.

`unica.xdto.edit` v1 поддерживает `add-value-type`, `add-object-type`,
`add-property`, `remove-type` и `remove-property`. Для вложенного анонимного
типа используй `propertyPath`, например `"СсылкаНаОбъект"` для
`ЛюбаяСсылка`. Writer сохраняет BOM и наблюдённые переводы строк, а повтор того
же добавления возвращает no-op.

## Пример preview

```json
{
  "sourceSet": "configuration",
  "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
  "operation": "add-property",
  "typeName": "ЛюбаяСсылка",
  "propertyPath": "СсылкаНаОбъект",
  "property": {
    "name": "Документ_НовыйДокумент",
    "type": "Документ_НовыйДокумент",
    "minOccurs": 0
  },
  "dryRun": true
}
```
