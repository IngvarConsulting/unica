---
name: meta-info
description: Анализ структуры объекта метаданных 1С из XML-выгрузки — реквизиты, табличные части, формы, движения, типы. Используй для изучения структуры объектов (вместо чтения XML-файлов напрямую) и как подготовительный шаг при написании запросов и кода, работающего с объектами
argument-hint: <ObjectPath> [-Mode overview|brief|full] [-Name <элемент>]
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

Читает XML объекта метаданных из выгрузки конфигурации 1С и выводит компактное описание структуры.

## MCP параметры

| Параметр | Описание |
|----------|----------|
| `ObjectPath` | Путь к XML-файлу объекта или каталогу (авто-резолв `<name>/<name>.xml`) |
| `Mode` | Режим: `overview` (default), `brief`, `full` |
| `Name` | Drill-down по имени элемента (реквизит, ТЧ, значение перечисления, шаблон URL, операция) |
| `Limit` / `Offset` | Пагинация (по умолчанию 150 строк) |
| `OutFile` | Записать результат в файл (UTF-8 BOM) |

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "src/Catalogs/Номенклатура.xml",
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
      "ObjectPath": "Catalogs/Валюты/Валюты.xml"
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
      "ObjectPath": "Documents/АвансовыйОтчет/АвансовыйОтчет.xml",
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
      "ObjectPath": "InformationRegisters/КурсыВалют/КурсыВалют.xml",
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
      "ObjectPath": "Documents/АвансовыйОтчет/АвансовыйОтчет.xml",
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
      "ObjectPath": "Catalogs/Валюты/Валюты.xml",
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
      "ObjectPath": "CommonModules/ОбщегоНазначения/ОбщегоНазначения.xml"
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
      "ObjectPath": "HTTPServices/ExternalAPI/ExternalAPI.xml"
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
      "ObjectPath": "HTTPServices/ExternalAPI/ExternalAPI.xml",
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
      "ObjectPath": "WebServices/EnterpriseDataUpload_1_0_1_1/EnterpriseDataUpload_1_0_1_1.xml"
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
      "ObjectPath": "WebServices/EnterpriseDataUpload_1_0_1_1/EnterpriseDataUpload_1_0_1_1.xml",
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
      "ObjectPath": "EventSubscriptions/ПолныйРегистрацияУдаления/ПолныйРегистрацияУдаления.xml",
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
      "ObjectPath": "ScheduledJobs/АвтоматическоеЗакрытиеМесяца/АвтоматическоеЗакрытиеМесяца.xml"
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
      "ObjectPath": "DefinedTypes/GLN/GLN.xml"
    }
  }
}
```
