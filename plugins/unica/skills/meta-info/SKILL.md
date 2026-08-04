---
name: meta-info
description: Прочитать типизированную локальную структуру и validation объекта метаданных 1С, при необходимости дополнив её независимо доступными связанными секциями.
argument-hint: <sourceSet> <metadataPath> [sections] [limit]
allowed-tools:
  - Read
  - Glob
---

# /meta-info — структура и проверка объекта метаданных

## MCP routing

- Preferred path: use MCP `unica` tool `unica.meta.info`.
- Выбирайте объект логически через `sourceSet + metadataPath`; расположение XML
  внутри выгрузки остаётся внутренней деталью `unica`.
- Читайте локальную структуру и `data.validation` из одного результата. Отдельный
  публичный вызов проверки не нужен.
- Без `sections` вызов остаётся локальным: он читает структуру и validation, но
  не обращается к связанному RLM-провайдеру. Для связанных данных явно укажите
  только нужные `sections` и ограничьте каждую секцию `limit` (`1..=50`).
- Успешный и предметно неуспешный `tools/call` возвращает `structuredContent`;
  `isError == !structuredContent.ok`. Читайте локальную структуру, validation и
  доступные частичные данные из `structuredContent.data`; `content[0].text` не
  является вторым контрактом результата.
- Не вызывайте внутренние MCP/CLI-адаптеры и skill-local scripts.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "Catalog.Валюты"
    }
  }
}
```

## Ответ

`data` всегда начинается с локально прочитанной структуры объекта: канонического
`metadataPath`, вида, имени, синонима, состояния поддержки, свойств именами
платформы, владельцев, реквизитов, измерений, ресурсов, табличных частей, форм,
макетов и команд. Поле `validation` (`data.validation`) содержит `status` и
типизированные `diagnostics`; та же внутренняя проверка выполняется перед каждой
мутацией.

Каждая явно запрошенная `data.related.<section>` имеет собственные `status`,
`freshness`, `total`, `limit` и `truncated`. Недоступность одной индексной секции
— это `soft-fail`: она получает самостоятельный статус и диагностику, но не
обесценивает локальную структуру и остальные секции. Без `sections` или с
`sections: []` `data.related` пуст; все связанные секции, включая
`predefinedItems`, включаются только явно.

Адрес принимает русские и английские псевдонимы вида, а в
`data.metadataPath` возвращает каноническую английскую форму. Если известен
только путь файла, сначала используйте `unica.source.locate`; если известно имя
— `unica.source.resolve`. Адрес модуля (`Catalog.X.ObjectModule`) здесь не
поддерживается: код читают `unica.code.*`.

«Представление типа», «Представление объекта» и представления списка ссылочного
объекта находятся в `properties` под платформенными именами
`ObjectPresentation`, `ExtendedObjectPresentation`, `ListPresentation` и
`ExtendedListPresentation`.

Раздел «Поддержка» читается из `Ext/ParentConfigurations.bin`. Объект на замке
изменяйте через CFE/release-support flow, не через raw support metadata.

Соглашения по именам, синонимам и представлениям находятся в
[общей ссылке](../../references/platform/metadata-conventions.md); перечни видов
и свойств не дублируются здесь, потому что их публикует схема операции.

## Примеры

### Документ: локальная структура и validation

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "Document.АвансовыйОтчет"
    }
  }
}
```

### Документ и явно запрошенные связанные секции

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "Document.АвансовыйОтчет",
      "sections": ["modules", "roles", "subscriptions", "functionalOptions"],
      "limit": 20
    }
  }
}
```

### HTTP-сервис

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "HTTPService.ExternalAPI"
    }
  }
}
```

### Веб-сервис

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "WebService.EnterpriseDataUpload_1_0_1_1"
    }
  }
}
```

### Определяемый тип

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "DefinedType.GLN"
    }
  }
}
```
