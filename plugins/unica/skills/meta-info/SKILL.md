---
name: meta-info
description: Анализ структуры объекта метаданных 1С из XML-выгрузки — реквизиты, табличные части, формы, движения, типы. Используй для изучения структуры объектов (вместо чтения XML-файлов напрямую) и как подготовительный шаг при написании запросов и кода, работающего с объектами
argument-hint: <sourceSet> <metadataPath> [-Mode overview|brief|full] [-Name <элемент>]
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /meta-info — Структура объекта метаданных 1С

## MCP routing

- Preferred path: use MCP `unica` tool `unica.meta.info`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
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
| `Mode` | Режим: `overview` (default), `brief`, `full` |
| `Name` | Drill-down по имени элемента (реквизит, ТЧ, значение перечисления, шаблон URL, операция) |
| `Limit` / `Offset` | Пагинация (по умолчанию 150 строк) |

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
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "Catalog.Номенклатура",
      "Mode": "overview",
      "Limit": 120
    }
  }
}
```

## Три режима

| Режим | Что показывает |
|---|---|
| `overview` *(default)* | Заголовок + ключевые свойства + структура без раскрытия деталей |
| `brief` | Всё одной-двумя строками: имена полей, счётчики |
| `full` | Всё раскрыто: колонки ТЧ, список источников подписки, движения, формы |

Для ссылочных объектов (`Справочник`, `Документ`, `Перечисление`, планы, `ПланОбмена`, `БизнесПроцесс`, `Задача`) вывод содержит `Представление типа`. В `full` дополнительно раскрываются `Представление объекта`, расширенные представления и представления списка, если они заданы в XML.

`-Name` — drill-down: раскрыть конкретный элемент объекта (ТЧ, реквизит, шаблон URL, операцию веб-сервиса).

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
      "cwd": "<workspace>",
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
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "Document.АвансовыйОтчет",
      "Mode": "full"
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
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "InformationRegister.КурсыВалют",
      "Mode": "brief"
    }
  }
}
```

### Drill-down в табличную часть документа

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "Document.АвансовыйОтчет",
      "Name": "Товары"
    }
  }
}
```

### Drill-down в реквизит

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "Catalog.Валюты",
      "Name": "ОсновнаяВалюта"
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
      "cwd": "<workspace>",
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
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "HTTPService.ExternalAPI"
    }
  }
}
```

### HTTP-сервис: drill-down в шаблон URL

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "HTTPService.ExternalAPI",
      "Name": "АктуальныеЗадачи"
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
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "WebService.EnterpriseDataUpload_1_0_1_1"
    }
  }
}
```

### Веб-сервис: drill-down в операцию

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "WebService.EnterpriseDataUpload_1_0_1_1",
      "Name": "TestConnection"
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
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "EventSubscription.ПолныйРегистрацияУдаления",
      "Mode": "full"
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
      "cwd": "<workspace>",
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
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "DefinedType.GLN"
    }
  }
}
```
