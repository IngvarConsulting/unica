---
name: form-add
description: Добавить пустую управляемую форму к объекту 1С. Используй когда нужно создать у объекта новую форму
argument-hint: <ObjectPath> <FormName> [Purpose] [SetDefault]
allowed-tools:
  - Bash
  - Read
  - Write
  - Edit
  - Glob
  - Grep
---

# /form-add — Добавление формы к объекту конфигурации

## MCP routing

- Preferred path: use MCP `unica` tool `unica.form.add`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.form.add`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.
- Vendor support guard runs inside `unica`; if it blocks a locked/read-only supported object, prefer CFE/release-support or an explicit support-state change plan instead of editing raw support metadata.

Создаёт управляемую форму (metadata XML + Form.xml + Module.bsl) и регистрирует её в корневом XML объекта конфигурации (Document, Catalog, InformationRegister и др.).

## Usage

Используй MCP `unica` tool `unica.form.add` с `ObjectPath`, `FormName`, `Purpose`, `Synonym` и `SetDefault`.

| Параметр    | Обязательный | По умолчанию | Описание                                     |
|-------------|:------------:|--------------|----------------------------------------------|
| ObjectPath  | да           | —            | Путь к XML-файлу объекта (Documents/Док.xml)  |
| FormName    | да           | —            | Имя формы (ФормаДокумента)                    |
| Purpose     | нет          | Object       | Назначение: Object, List, Choice, Record      |
| Synonym     | нет          | = FormName   | Синоним формы                                 |
| SetDefault  | нет          | авто         | Boolean: `true` — назначить форму по умолчанию, `false` — не менять default-slot, отсутствие параметра — автоназначение только для пустого слота |

## MCP вызов

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.form.add",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "src/Catalogs/Номенклатура.xml",
      "FormName": "ФормаЭлемента",
      "Purpose": "Object",
      "Synonym": "Форма элемента",
      "SetDefault": true,
      "dryRun": false
    }
  }
}
```

## Purpose — назначение формы

| Purpose | Допустимые типы объектов | Основной реквизит | DefaultForm-свойство |
|---------|-------------------------|-------------------|---------------------|
| Object  | Document, Catalog, DataProcessor, Report, ExternalDataProcessor, ExternalReport, ChartOf*, ExchangePlan, BusinessProcess, Task | Объект (тип: *Object.Имя) | DefaultObjectForm (DefaultForm для DataProcessor/Report/ExternalDataProcessor/ExternalReport) |
| List    | Все кроме DataProcessor | Список (DynamicList) | DefaultListForm |
| Choice  | Document, Catalog, ChartOf*, ExchangePlan, BusinessProcess, Task | Список (DynamicList) | DefaultChoiceForm |
| Record  | InformationRegister | Запись (InformationRegisterRecordManager) | DefaultRecordForm |

## Примеры

### Форма документа

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.form.add",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "Documents/АвансовыйОтчет.xml",
      "FormName": "ФормаДокумента",
      "Purpose": "Object",
      "dryRun": false
    }
  }
}
```

### Форма списка каталога

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.form.add",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "Catalogs/Контрагенты.xml",
      "FormName": "ФормаСписка",
      "Purpose": "List",
      "dryRun": false
    }
  }
}
```

### Форма записи регистра сведений

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.form.add",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "InformationRegisters/КурсыВалют.xml",
      "FormName": "ФормаЗаписи",
      "Purpose": "Record",
      "dryRun": false
    }
  }
}
```

### Форма выбора с синонимом

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.form.add",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "Catalogs/Номенклатура.xml",
      "FormName": "ФормаВыбора",
      "Purpose": "Choice",
      "Synonym": "Выбор номенклатуры",
      "dryRun": false
    }
  }
}
```

### Установить как форму по умолчанию

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.form.add",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectPath": "Documents/Заказ.xml",
      "FormName": "ФормаДокументаНовая",
      "Purpose": "Object",
      "SetDefault": true,
      "dryRun": false
    }
  }
}
```
## Workflow

1. `unica.form.add` — создать каркас формы
2. `unica.form.compile` или `unica.form.edit` — наполнить Form.xml элементами
3. `unica.form.validate` — проверить корректность
4. `unica.form.info` — проанализировать результат
