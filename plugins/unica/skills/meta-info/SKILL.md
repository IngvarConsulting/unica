---
name: meta-info
description: Анализ структуры объекта метаданных 1С из XML-выгрузки — реквизиты, табличные части, формы, движения, типы. Используй для изучения структуры объектов (вместо чтения XML-файлов напрямую) и как подготовительный шаг при написании запросов и кода, работающего с объектами
argument-hint: <sourceSet> <metadataPath>
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /meta-info — Структура объекта метаданных 1С

## MCP routing

- Preferred path: use MCP `unica` tool `unica.meta.info`; `unica` owns typed metadata reads and validation.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.meta.info`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Читает объект метаданных 1С по логическому адресу и выводит компактное описание структуры. Раскладка выгрузки конфигуратора знать не нужно: `sourceSet` называет набор исходников из карты проекта, `metadataPath` — сам объект.

В основном выводе показывает `Поддержка` по `Ext/ParentConfigurations.bin`: не на поддержке, на замке, редактируется с сохранением поддержки, снято с поддержки или read-only. Если объект на замке, планируй доработку через CFE/release-support flow, а не через прямую правку raw support metadata.

## MCP параметры

| Параметр | Описание |
|----------|----------|
| `sourceSet` | Имя набора исходников из карты проекта; список даёт `unica.project.map` |
| `metadataPath` | Логический адрес объекта: `Catalog.Номенклатура`, `Справочник.Номенклатура`, `Catalog.Номенклатура.Form.ФормаЭлемента` |
| `sections` | Связанные индексные секции: `modules`, `roles`, `subscriptions`, `functionalOptions`, `predefinedItems`; без аргумента запрашиваются первые четыре |
| `limit` | Максимум элементов каждой связанной секции; по умолчанию `20` |

Адрес принимает русские и английские псевдонимы вида, а отвечает канонической
английской формой в `data.metadataPath` — её можно передать дальше любому
логическому инструменту. Если известен только путь файла, `unica.source.locate`
переводит его в адрес, а `unica.source.resolve` ищет объект по имени.

Модуль объекта этот инструмент не читает: `Catalog.X.ObjectModule` отклоняется,
для кода есть `unica.code.*`.

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

## Ответ

Инструмент отвечает типизированным `data`: локальную структуру объекта и
`validation` он отдаёт целиком — вид, имя, синоним, поддержку, свойства
**именами платформы** (`NumberType`,
`Hierarchical`, `LevelCount`…), владельцев, реквизиты, измерения, ресурсы,
табличные части с колонками, значения перечисления, формы, макеты и команды.
Запрошенные индексные секции ограничены `limit` и отдельно сообщают статус,
свежесть, `total` и `truncated`; `predefinedItems` включается только явно.
Физические режимы и drill-down больше не нужны — берите нужную секцию из
`data`.

«Представление типа», «Представление объекта» и представления списка ссылочных
объектов лежат в `properties` под платформенными именами `ObjectPresentation`,
`ExtendedObjectPresentation`, `ListPresentation`, `ExtendedListPresentation` —
переводить их обратно в русские подписи инструмент больше не пытается.

## Поддерживаемые типы (23)

**Ссылочные:** Справочник, Документ, Перечисление, Бизнес-процесс, Задача, План обмена, План счетов, ПВХ, ПВР
**Регистры:** Регистр сведений, Регистр накопления, Регистр бухгалтерии, Регистр расчёта
**Сервисные:** Отчёт, Обработка, HTTP-сервис, Веб-сервис, Общий модуль, Регламентное задание, Подписка на событие
**Прочие:** Константа, Журнал документов, Определяемый тип

## Примеры

### Справочник: overview

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

### Документ: полная сводка

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

### Регистр сведений: краткая сводка

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "InformationRegister.КурсыВалют"
    }
  }
}
```

### Табличные части документа

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

### Реквизиты справочника

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

### Общий модуль

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "CommonModule.ОбщегоНазначения"
    }
  }
}
```

### HTTP-сервис: шаблоны URL и методы

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

### HTTP-сервис: шаблоны URL в `data`

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

### Веб-сервис: операции с параметрами

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

### Веб-сервис: операции в `data`

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

### Подписка на событие

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "EventSubscription.ПолныйРегистрацияУдаления"
    }
  }
}
```

### Регламентное задание

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "ScheduledJob.АвтоматическоеЗакрытиеМесяца"
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
