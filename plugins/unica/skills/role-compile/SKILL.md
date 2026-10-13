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

- Preferred path: use MCP `unica` tool `unica.apply` с операциями
  `role.create` и `right.add`.
- Do not call internal MCP/CLI adapters directly. They are hidden behind
  `unica` and synchronized by the orchestrator.
- **Создание метит в корень конфигурации:** `args.at` — это
  `<набор>:Configuration`, имя роли лежит в `values.name`. Адреса, которого
  ещё нет, назвать нельзя. Записи прав создаёт `right.add` на самой роли.
- Сначала вызови `unica.apply` с `at` и `ops`: это план без записи.
  Когда пользователь поручил внести эту правку, вызови `unica.apply` только
  с `executionToken` из `data.executionToken` успешного плана.
- Роль и все её права публикуются одной транзакцией: либо роль появляется
  настроенной, либо не появляется вовсе.
- Каталогов выгрузки и путей к `Rights.xml` тут нет — их место занял адрес.

### Пресеты разворачивает скилл, а не инструмент

`right.add` на роли создаёт записи прав: элемент `items` называет объект
(`object`, например `Catalog.Номенклатура`) и его начальные права (`rights`,
например `{"Read": true, "View": true}`). Пресета `@view` инструмент не знает —
разверни его сам по таблице ниже в явные права. Так видно, какие именно права
выданы, и предпросмотр показывает их поимённо, а не имя пресета.

Ограничение записей задаёт `right.set` на уже созданной записи
`<набор>:Role.<Имя>.Right.<Вид>_<Имя>`: `values` с `right` и `rls`. В одном
пакете запись можно создать `right.add` и сразу ограничить `right.set`.
`right.set` не создаёт запись: по адресу, которого нет, он отвечает
`not_found`.

### Чего на канонической поверхности нет

**Шаблоны RLS.** Операции, создающей `restrictionTemplate`, в реестре нет.
`right.add` и `right.set` их сохраняют — существующие шаблоны при правке прав не страдают, —
но объявить новый нечем. Условие ограничения пиши прямо в `values.rls`; если
нужен именно переиспользуемый шаблон, скажи о пробеле контракта, а не
подставляй ссылку на шаблон, которого не создашь.

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

## Порядок

1. Имя набора: `unica.view {}`.
2. Разверни пресеты в явные права по таблице выше.
3. Предпросмотр: `unica.apply` с `at` и `ops`, `at` в корень конфигурации.
4. Применение: вызов только с `executionToken` из `data.executionToken` успешного плана.
5. Проверка: `unica.check {at}` уже по адресу роли.

## Примеры вызова

### Роль с правами одним пакетом

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Configuration",
      "ops": [
        {
          "op": "role.create",
          "args": {
            "at": "main:Configuration",
            "values": {"name": "ЧтениеНоменклатуры"}
          }
        },
        {
          "op": "right.add",
          "args": {
            "at": "main:Role.ЧтениеНоменклатуры",
            "items": [
              {"object": "Catalog.Номенклатура", "rights": {"Read": true, "View": true}}
            ]
          }
        }
      ]
    }
  }
}
```

### Право с ограничением записей

Условие пишется прямо в `values.rls`: шаблон объявить нечем. Запись прав
создаёт `right.add`, а ограничение на неё ставит `right.set` по её адресу.

Сначала получи план этого изменения:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Role.ЧтениеПоОрганизации",
      "ops": [
        {
          "op": "right.add",
          "args": {
            "at": "main:Role.ЧтениеПоОрганизации",
            "items": [
              {"object": "Document.РеализацияТоваровУслуг", "rights": {"Read": true}}
            ]
          }
        },
        {
          "op": "right.set",
          "args": {
            "at": "main:Role.ЧтениеПоОрганизации.Right.Document_РеализацияТоваровУслуг",
            "values": {
              "right": "Read",
              "rls": "ГДЕ Организация = &ТекущаяОрганизация"
            }
          }
        }
      ]
    }
  }
}
```

Для исполнения передай `data.executionToken` из этого успешного плана:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "executionToken": "<data.executionToken из успешного плана>"
    }
  }
}
```

## Верификация

### Проверка корректности XML, прав и RLS

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.check",
    "arguments": {
      "at": "<набор>:Role.<ИмяРоли>"
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
      "at": "<набор>:Role.<ИмяРоли>"
    }
  }
}
```
