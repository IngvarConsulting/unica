---
name: cfe-borrow
description: Заимствование объектов из конфигурации 1С в расширение (CFE). Используй когда нужно перехватить метод, изменить форму или добавить реквизит к существующему объекту конфигурации
argument-hint: -ExtensionPath <path> -ConfigPath <path> -Object "Catalog.Контрагенты.Form.ФормаЭлемента" -BorrowMainAttribute
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /cfe-borrow — Заимствование объектов из конфигурации

## MCP routing

- Preferred path: use MCP `unica` tool `unica.cfe.borrow`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.cfe.borrow`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Заимствует объекты из основной конфигурации в расширение. Создаёт XML-файлы с `ObjectBelonging=Adopted` и `ExtendedConfigurationObject`, добавляет запись в ChildObjects расширения.

## Предусловие

Расширение должно быть создано (`/cfe-init`) и содержать валидный `Configuration.xml`.

### Авто-определение ConfigPath

Если пользователь не указал `-ConfigPath` — попробуй определить автоматически:
1. Используй `./v8project.yaml`.
2. Найди `source-set` с `type: CONFIGURATION`.
3. Используй его `path` как `-ConfigPath`.
4. Если source-set не найден — спроси путь у пользователя.

## Параметры

| Параметр | Описание |
|----------|----------|
| `ExtensionPath` | Путь к каталогу расширения (обязат.) |
| `ConfigPath` | Путь к конфигурации-источнику (обязат.) |
| `Object` | Что заимствовать (обязат.), batch через `;;` |
| `BorrowMainAttribute` | Заимствовать основной реквизит формы. Без параметра — не заимствует. `Form` — реквизиты, используемые на форме. `All` — все реквизиты объекта. Требует форму в -Object |

## Формат -Object

- `Catalog.Контрагенты` — справочник
- `CommonModule.РаботаСФайлами` — общий модуль
- `Document.РеализацияТоваров` — документ
- `Enum.ВидыОплат` — перечисление
- `Catalog.Контрагенты.Form.ФормаЭлемента` — форма объекта (заимствование формы)
- `Catalog.X ;; CommonModule.Y ;; Enum.Z` — несколько объектов
Поддерживаются все 44 типа объектов конфигурации.

### Заимствование форм

Формат `Тип.Имя.Form.ИмяФормы` заимствует форму конкретного объекта. Если родительский объект ещё не заимствован — он будет заимствован автоматически.

Создаётся:
1. **Метаданные формы** — `Forms/ИмяФормы.xml` с `ObjectBelonging=Adopted`, `FormType=Managed`
2. **Form.xml** — `Forms/ИмяФормы/Ext/Form.xml` с копией исходной формы + `<BaseForm>` (начальное состояние)
3. **Module.bsl** — пустой файл `Forms/ИмяФормы/Ext/Form/Module.bsl`
4. **Регистрация** — `<Form>` в ChildObjects родительского объекта

### Заимствование основного реквизита формы (-BorrowMainAttribute)

**Когда нужно**: пользователь хочет добавить новый реквизит в существующий объект конфигурации и вывести его на заимствованную форму. Без `-BorrowMainAttribute` форма заимствуется "пустой" — только визуальные элементы, без привязки к данным объекта. С `-BorrowMainAttribute` форма сохраняет привязки к реквизитам объекта (DataPath), что позволяет затем добавить на неё новые элементы через `/form-edit`.

**Два режима**:
- `Form` (по умолчанию) — заимствует только те реквизиты объекта, которые уже выведены на форму. Оптимальный выбор для большинства случаев
- `All` — заимствует все реквизиты и табличные части объекта. Используй если планируешь выводить на форму реквизиты, которых на ней ещё нет

**Типовой сценарий** (добавление реквизита + вывод на форму):
1. `/cfe-borrow` с `-BorrowMainAttribute` — заимствовать форму с реквизитами
2. `/meta-edit` — добавить новый реквизит в объект расширения
3. `/form-edit` — вывести реквизит на заимствованную форму

**Защита существующих данных**: если зависимый объект уже заимствован с содержимым (реквизитами, формами) — скрипт не перезаписывает его, а добавляет только недостающее.

## MCP вызов

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cfe.borrow",
    "arguments": {
      "cwd": "<workspace>",
      "ExtensionPath": "src/extensions/MyExtension",
      "ConfigPath": "src",
      "Object": "Catalog.Контрагенты.Form.ФормаЭлемента",
      "BorrowMainAttribute": "Form",
      "dryRun": false
    }
  }
}
```

## Примеры

### Заимствовать один объект

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cfe.borrow",
    "arguments": {
      "cwd": "<workspace>",
      "ExtensionPath": "src",
      "ConfigPath": "C:\\cfsrc\\erp",
      "Object": "Catalog.Контрагенты",
      "dryRun": false
    }
  }
}
```

### Заимствовать форму

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cfe.borrow",
    "arguments": {
      "cwd": "<workspace>",
      "ExtensionPath": "src",
      "ConfigPath": "C:\\cfsrc\\erp",
      "Object": "Catalog.Контрагенты.Form.ФормаЭлемента",
      "dryRun": false
    }
  }
}
```

### Несколько объектов за раз

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cfe.borrow",
    "arguments": {
      "cwd": "<workspace>",
      "ExtensionPath": "src",
      "ConfigPath": "C:\\cfsrc\\erp",
      "Object": "Catalog.Контрагенты ;; CommonModule.ОбщийМодуль ;; Enum.ВидыОплат",
      "dryRun": false
    }
  }
}
```

### Заимствовать форму с основным реквизитом

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cfe.borrow",
    "arguments": {
      "cwd": "<workspace>",
      "ExtensionPath": "src",
      "ConfigPath": "C:\\cfsrc\\erp",
      "Object": "Catalog.Номенклатура.Form.ФормаЭлемента",
      "BorrowMainAttribute": true,
      "dryRun": false
    }
  }
}
```

### Заимствовать форму со всеми реквизитами объекта

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cfe.borrow",
    "arguments": {
      "cwd": "<workspace>",
      "ExtensionPath": "src",
      "ConfigPath": "C:\\cfsrc\\erp",
      "Object": "Catalog.Номенклатура.Form.ФормаЭлемента",
      "BorrowMainAttribute": "All",
      "dryRun": false
    }
  }
}
```

## Верификация

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cfe.validate",
    "arguments": {
      "cwd": "<workspace>",
      "ExtensionPath": "src/extensions/MyExtension"
    }
  }
}
```
