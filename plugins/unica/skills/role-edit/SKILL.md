---
name: role-edit
description: Создать роль 1С с набором прав или типизированно изменить права существующей роли по логическому адресу, сохранив RLS, шаблоны и остальные права.
argument-hint: <at> <operations>
allowed-tools:
  - Read
  - Glob
---

# /unica:role-edit — создание роли и точечное редактирование её прав

## MCP routing

- Preferred path: use MCP `unica` tool `unica.apply` с операциями
  `role.create`, `right.add` и `right.set`.
- Права одного объекта — запись роли со своим адресом
  `<набор>:Role.<Роль>.Right.<Вид>_<Имя>`, например
  `main:Role.Демо.Right.Catalog_Демо`. Записи роли видны в ветви
  `<набор>:Role.<Роль>.Right`. Физический `Rights.xml` — внутренняя деталь;
  наружу путь отдаёт только аварийный `unica.resolve`, и в обычном ходе
  работы он не нужен.
- Имя набора даёт `unica.view {}`, адрес роли по имени —
  `unica.search {corpus: "names", kind: "Role"}`. `"main"` ниже только пример.
- Сначала вызови `unica.apply` с `at` и `ops`: это план без записи.
  Когда пользователь поручил внести эту правку, вызови `unica.apply` только
  с `executionToken` из `data.executionToken` успешного плана.
- Точную форму аргументов даёт контракт операции по её адресу:
  `<набор>:Configuration.Can.role.create`,
  `<набор>:Role.<Роль>.Can.right.add` и
  `<набор>:Role.<Роль>.Right.<Вид>_<Имя>.Can.right.set`.
- Читай из ответа `changed`, `effects` по порядку операций и диагностики.
  Физический путь и diff контрактом результата не являются.

Права роли читаются ветвью `…Role.<Имя>.Right`: запись объекта адресуется
`Right.<Вид>_<Имя>`. У вложенного объекта (команда, реквизит) имя содержит
точки, и в адресе они экранируются `\.`:
`main:Role.Демо.Right.Catalog_Демо\.Command\.Печать` (в JSON — `\\.`).
Готовый адрес берётся из `items[].at` ветви.

## Операция `role.create`

Создание метит в корень конфигурации: `args.at` — это
`<набор>:Configuration`, имя роли лежит в `values.name`. Адреса, которого
ещё нет, назвать нельзя. Роль появляется без прав; записи прав создаёт
`right.add` на адресе новой роли в том же пакете. Роль и все её права
публикуются одной транзакцией: либо роль появляется настроенной, либо
не появляется вовсе. Каталогов выгрузки и путей к `Rights.xml` тут нет.

## Операция `right.set`

Меняет одно право существующей записи. Адрес — сама запись, `args.values`
принимает `right` и хотя бы одно из `value` и `rls`:

| Поле | Что задаёт |
|---|---|
| `right` | Имя права, например `Delete` |
| `value` | Булево значение права |
| `rls` | Условие ограничения записей |

`right.set` не создаёт запись: адрес объекта, которого в роли нет, получает
`not_found` с `next` к роли. Объект в `values.object` снят — такой вызов
получает отказ с адресом записи в `next`.

## Операция `right.add`

Создаёт записи прав для объектов, которых в роли ещё нет. Адрес — роль,
элемент `args.items` называет объект (`object`, например `Catalog.Демо`)
и его начальные права (`rights`, например `{"Read": true}`). Объект, у
которого запись уже есть, отвергается: его права меняет `right.set` по
адресу записи. Ограничение записей на новую запись ставит `right.set` в том
же пакете.

Вложенный объект (`DataProcessor.X.Command.Y`, реквизит) получает одну
запись: остальные точки его имени в адресе экранируются,
`Role.R.Right.DataProcessor_X\.Command\.Y`. Адрес записи — только
каноническое имя `<Вид>_<Имя>` из ветви `Right`.

Операции выполняются по порядку и публикуются одной транзакцией: либо все,
либо ни одной. Повтор эквивалентной `right.set` даёт `changed: false` без
записи.

Платформа хранит объектное право только со значением, отличным от умолчания
роли `setForNewObjects`. Поэтому `right.set` со значением умолчания (обычно
`false`) удаляет элемент права вместе с его RLS, а опустевшую запись —
целиком; если право уже отсутствует, это законный no-op. По той же причине
`right.add` отвергает запись, все права которой равны умолчанию: она
осталась бы пустой.

Автоматическое включение зависимых прав в согласуемый preview ещё не
реализовано ([задача #941](https://github.com/IngvarConsulting/unica/issues/941)).
До реализации проверь доказанные зависимости и согласуй полный набор прав
с человеком. Не считай одиночный `right.set` проверкой полноты этого набора.

Для `DataProcessor.*` значение `Use=false` применяет правило платформы:
удаляется вся запись вместе с зависимым правом `View`. На другие виды
объектов и права это не обобщается.

## Типовые наборы прав

Инструмент не знает пресетов: каждое право в `rights` называется явно,
и предпросмотр показывает выданные права поимённо. Типовые наборы для
`right.add` — разверни их сам по таблицам ниже.

Просмотр:

| Вид объекта | Права |
|---|---|
| Catalog, ExchangePlan, Document, ChartOfAccounts, ChartOfCharacteristicTypes, ChartOfCalculationTypes, BusinessProcess, Task | Read, View, InputByString |
| InformationRegister, AccumulationRegister, AccountingRegister, CalculationRegister, Constant, DocumentJournal | Read, View |
| Sequence | Read |
| CommonForm, CommonCommand, Subsystem, FilterCriterion, CommonAttribute | View |
| DataProcessor, Report | Use, View |
| SessionParameter | Get |
| Configuration (объект `Configuration.<Имя конфигурации>`) | ThinClient, WebClient, Output, SaveUserData, MainWindowModeNormal |

Полное редактирование:

| Вид объекта | Права |
|---|---|
| Catalog, ExchangePlan, ChartOfAccounts, ChartOfCharacteristicTypes, ChartOfCalculationTypes | Read, Insert, Update, Delete, View, Edit, InputByString, InteractiveInsert, InteractiveSetDeletionMark, InteractiveClearDeletionMark |
| Document | Read, Insert, Update, Delete, View, Edit, InputByString, Posting, UndoPosting, InteractiveInsert, InteractiveSetDeletionMark, InteractiveClearDeletionMark, InteractivePosting, InteractivePostingRegular, InteractiveUndoPosting, InteractiveChangeOfPosted |
| BusinessProcess | Read, Insert, Update, Delete, View, Edit, InputByString, Start, InteractiveInsert, InteractiveSetDeletionMark, InteractiveClearDeletionMark, InteractiveActivate, InteractiveStart |
| Task | Read, Insert, Update, Delete, View, Edit, InputByString, Execute, InteractiveInsert, InteractiveSetDeletionMark, InteractiveClearDeletionMark, InteractiveActivate, InteractiveExecute |
| InformationRegister, AccumulationRegister, AccountingRegister, Constant | Read, Update, View, Edit |
| DocumentJournal | Read, View |
| Sequence | Read, Update |
| SessionParameter | Get, Set |
| CommonAttribute | View, Edit |

Для сервисов (WebService, HTTPService, IntegrationService) типового набора
нет: право `Use` называется явно. Полный каталог прав по видам объектов
и список видов без прав в ролях — в
[спецификации ролей](../../references/specs/1c-role-spec.md).

## Чего на канонической поверхности нет

**Шаблоны RLS.** Операции, создающей `restrictionTemplate`, в реестре нет.
`right.add` и `right.set` сохраняют существующие шаблоны, но объявить новый
нечем. Условие ограничения пиши прямо в `values.rls`; если нужен именно
переиспользуемый шаблон, скажи о пробеле контракта, а не подставляй ссылку
на шаблон, которого не создашь.

## Порядок

1. Имя набора: `unica.view {}`; адрес существующей роли —
   `unica.search {corpus: "names", kind: "Role"}`.
2. Разверни типовые наборы в явные права по таблицам выше.
3. Предпросмотр: `unica.apply` с `at` и `ops`. Новая роль — `at` в корень
   конфигурации, правка — `at` на роль или её запись.
4. Применение: вызов только с `executionToken` из `data.executionToken`
   успешного плана.
5. Проверка: `unica.check {at}` по адресу роли.

## MCP examples

### Предпросмотр

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Role.Демо.Right.Catalog_Демо",
      "ops": [
        {
          "op": "right.set",
          "args": {
            "at": "main:Role.Демо.Right.Catalog_Демо",
            "values": {
              "right": "Delete",
              "value": false
            }
          }
        }
      ]
    }
  }
}
```

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
              {
                "object": "Catalog.Номенклатура",
                "rights": {"Read": true, "View": true, "InputByString": true}
              }
            ]
          }
        }
      ]
    }
  }
}
```

### Новая запись с ограничением записей

Сначала получи план этого изменения:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Role.Демо",
      "ops": [
        {
          "op": "right.add",
          "args": {
            "at": "main:Role.Демо",
            "items": [
              {"object": "Document.Заказ", "rights": {"Read": true}}
            ]
          }
        },
        {
          "op": "right.set",
          "args": {
            "at": "main:Role.Демо.Right.Document_Заказ",
            "values": {
              "right": "Read",
              "rls": "ГДЕ Автор = &Пользователь"
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
      "at": "main:Role.ЧтениеНоменклатуры"
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
      "at": "main:Role.ЧтениеНоменклатуры"
    }
  }
}
```
