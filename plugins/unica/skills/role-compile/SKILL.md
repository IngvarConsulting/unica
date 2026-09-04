---
name: role-compile
description: Создание роли 1С из описания прав. Используй когда нужно создать новую роль с набором прав на объекты
argument-hint: <JsonPath> <OutputDir>
allowed-tools:
  - Bash
  - Read
  - Write
  - Glob
---

# /role-compile — генерация роли 1С из JSON DSL

## MCP routing

- Preferred path: use MCP `unica` tool `unica.role.compile`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.role.compile`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.
- Vendor support guard runs inside `unica`; if it blocks a locked/read-only supported object, prefer CFE/release-support or an explicit support-state change plan instead of editing raw support metadata.

Принимает JSON-определение роли → генерирует `Roles/Имя.xml` (метаданные) и `Roles/Имя/Ext/Rights.xml` (права). UUID автоматически.

## MCP параметры

| Параметр | Описание |
|----------|----------|
| `JsonPath` | Путь к JSON-определению роли |
| `OutputDir` | Корень выгрузки конфигурации (где `Configuration.xml`, `Roles/` и т.д.) |

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.role.compile",
    "arguments": {
      "cwd": "<workspace>",
      "JsonPath": "roles/read-products.json",
      "OutputDir": "src",
      "dryRun": false
    }
  }
}
```

Создаёт `{OutputDir}/Roles/Имя.xml` и `{OutputDir}/Roles/Имя/Ext/Rights.xml`. Регистрирует `<Role>` в `Configuration.xml`.

## JSON DSL

### Структура

```json
{ "name": "ИмяРоли", "synonym": "Отображаемое имя", "objects": [...], "templates": [...] }
```

Необязательные: `comment` (""), `setForNewObjects` (false), `setForAttributesByDefault` (true), `independentRightsOfChildObjects` (false).

### Shorthand-строки и объектная форма

```json
"objects": [
  "Catalog.Номенклатура: @view",
  "Document.Реализация: @edit",
  "DataProcessor.Загрузка: @view",
  "InformationRegister.Цены: Read, Update",
  { "name": "Document.Продажа", "preset": "view", "rights": {"Delete": false}, "rls": {"Read": "#Шаблон(\"\")"} }
]
```

- Shorthand: `"Тип.Имя: @пресет"` или `"Тип.Имя: Право1, Право2"`
- Объектная форма: `preset` + `rights` (переопределения) + `rls` (ограничения)

### Пресеты

| Пресет | Действие |
|--------|----------|
| `@view` | Просмотр — Read, View (+InputByString для справочников/документов; Use+View для обработок/отчётов) |
| `@edit` | Полное редактирование — CRUD + Interactive* + Posting (документы) |

`@` обязателен в shorthand. В объектной форме — `"preset": "view"` без `@`.

Для сервисов (WebService, HTTPService, IntegrationService) пресеты не определены — используй явные права: `"WebService.Имя: Use"`.

### Русские синонимы

Поддерживаются русские типы (`Справочник`→Catalog, `Документ`→Document) и права (`Чтение`→Read, `Просмотр`→View). Смешивание допустимо: `"Справочник.Контрагенты: Чтение, View"`.

### Шаблоны RLS

```json
"templates": [{"name": "ДляОбъекта(Мод)", "condition": "ГДЕ Организация = &ТекОрг"}]
```

Ссылка в `rls`: `"#ДляОбъекта(\"\")"`. Символ `&` автоматически экранируется в XML.

## Примеры

### Простая роль

```json
{
  "name": "ЧтениеНоменклатуры", "synonym": "Чтение номенклатуры",
  "objects": ["Catalog.Номенклатура: @view", "Catalog.Контрагенты: @view", "DataProcessor.Загрузка: @view"]
}
```

### Роль с RLS

```json
{
  "name": "ЧтениеДокументовПоОрганизации",
  "synonym": "Чтение документов (ограничение по организации)",
  "objects": [
    "Catalog.Организации: @view",
    {"name": "Document.РеализацияТоваровУслуг", "preset": "view", "rls": {"Read": "#ДляОбъекта(\"\")"}}
  ],
  "templates": [{"name": "ДляОбъекта(Модификатор)", "condition": "ГДЕ Организация = &ТекущаяОрганизация"}]
}
```

Подробные таблицы пресетов, русских синонимов и дополнительные примеры — в `dsl-reference.md`.

## Верификация

### Проверка корректности XML, прав и RLS

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.check",
    "arguments": {
      "at": "<sourceSet>:Role.<Name>"
    }
  }
}
```

### Сводка структуры роли

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.view",
    "arguments": {
      "cwd": "<workspace>",
      "at": "<набор>:Role.<ИмяРоли>"
    }
  }
}
```
