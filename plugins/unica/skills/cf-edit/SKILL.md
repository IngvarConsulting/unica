---
name: cf-edit
description: Точечное редактирование конфигурации 1С. Используй когда нужно изменить свойства конфигурации, добавить или удалить объект из состава, настроить роли по умолчанию, поменять раскладку панелей, настроить начальную страницу
argument-hint: -ConfigPath <path> -Operation <op> -Value <value>
allowed-tools:
  - Bash
  - Read
  - Write
  - Glob
---

# /cf-edit — редактирование конфигурации 1С

## MCP routing

- Preferred path: use MCP `unica` tool `unica.apply` по адресу корня набора (`<набор>:Configuration`): свойства правит `props.set`, состав — `object.create`/`object.remove`, порядок подсистем верхнего уровня — `subsystemOrder.set`; остальной командный интерфейс — на интерфейсе подсистемы через `/unica:interface-edit`.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Сначала вызови `unica.apply` с `at` и `ops`: это план без записи.
  Когда пользователь поручил внести эту правку, вызови `unica.apply` только
  с `executionToken` из `data.executionToken` успешного плана.
- Vendor support guard runs inside `unica`; if it blocks a locked/read-only supported object, prefer CFE/release-support or an explicit support-state change plan instead of editing raw support metadata.

Точечное редактирование Configuration.xml: свойства, состав ChildObjects, роли по умолчанию.

## Адрес и операции

Цель — корень набора: `<набор>:Configuration`. Что именно словарь пишет на этом
узле, называет `unica.view {at}` в словаре `can`; ниже — те операции, которыми
выражается предмет этого скилла:

| Операция | Предмет |
|---|---|
| `props.set` | свойства корня: версия, поставщик, режим совместимости и прочие пары «ключ — значение» |
| `object.create`, `object.remove` | состав конфигурации: объект метаданных заводится по виду и имени, снимается по своему адресу; общий макет снимается тем же `object.remove` |
| `subsystemOrder.set` | порядок подсистем верхнего уровня: `values: {subsystems: […]}`; правила перестановки — в `/unica:interface-edit` |
| `panels.set` | раскладка панелей: области `top`, `left`, `right`, `bottom`, в каждой панели и группы панелей |
| `homePage.set` | начальная страница: шаблон и формы левой и правой колонок |

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Configuration",
      "ops": [
        {"op": "props.set", "args": {"values": {"version": "1.0.0.1", "vendor": "Фирма"}}}
      ]
    }
  }
}
```

Применение — вызов только с `executionToken` из `data.executionToken` успешного плана.

Перечень свойств и значения перечислений `props.set` берёт из схемы `values`
в `can` корня; имя и префикс, язык, роли по умолчанию и хранилища им не
правятся. У корня расширения `props.set` правит только описание самого
расширения: синоним, комментарий, поставщика, версию, сведения о нём и режим
совместимости. Синоним и другие многоязычные строки пишутся на основном
языке набора; если у расширения он не задан, план отказывает.

`commandVisibility.set`, `commandPlacement.set`, `commandOrder.set` и
`groupOrder.set` на `<набор>:Configuration` отклоняются: их адрес —
интерфейс подсистемы, например `main:Subsystem.Продажи.Interface`, а у
`commandOrder.set` — группа этого интерфейса (`…Interface.Group.<Группа>`);
вложенная подсистема адресуется всей цепочкой. Маршрута `main:Interface`
у корня нет.
Эти правки описаны в `/unica:interface-edit`.

`panels.set` и `homePage.set` заменяют документ целиком: что не названо,
того не будет. Текущая раскладка лежит в `view` корня, в `panels` и
`homePage`, и принимается операцией обратно как есть. Если в `panels` есть
`unknown` — панели, которых Unica не знает, — `panels.set` отказывает, а не
теряет их. У корня расширения
этих операций нет.

**Роли по умолчанию** канонической операции не имеют: их правка — пробел
контракта Unica MCP, сообщай о нём, а не подменяй соседней операцией.

## Операции

| Операция | Формат Value | Описание |
|----------|-------------|----------|
| `modify-property` | `Ключ=Значение` (batch `;;`) | Изменить свойство |
| `add-childObject` | `Type.Name` (batch `;;`) | Зарегистрировать уже существующий файл объекта в ChildObjects. Для создания нового объекта используй `/unica:meta-add`, `/unica:role-edit` (`role.create`), `/unica:subsystem-edit` (`subsystem.create`) — они регистрируют автоматически |
| `remove-childObject` | `Type.Name` (batch `;;`) | Удалить объект из ChildObjects |
| `add-defaultRole` | `Role.Name` или `Name` | Добавить роль по умолчанию |
| `remove-defaultRole` | `Role.Name` или `Name` | Удалить роль по умолчанию |
| `set-defaultRoles` | Имена через `;;` | Заменить список ролей по умолчанию |
| `set-panels` | JSON-объект (см. [reference.md](reference.md)) | Перезаписать `Ext/ClientApplicationInterface.xml` (раскладка панелей) |
| `set-home-page` | JSON-объект (см. [reference.md](reference.md)) | Перезаписать `Ext/HomePageWorkArea.xml` (начальная страница) |

Допустимые значения свойств, формат DefinitionFile (JSON), каноничный порядок: [reference.md](reference.md)

## Примеры

### Изменить версию и поставщика

```json
{
  "op": "props.set",
  "args": {"values": {"version": "1.0.0.1", "vendor": "Фирма"}}
}
```

### Добавить объект в состав

```json
{
  "op": "object.create",
  "args": {"values": {"kind": "Catalog", "name": "Товары"}}
}
```

### Снять объект из состава

```json
{
  "op": "object.remove",
  "args": {"at": "main:Catalog.Товары"}
}
```

### Раскладка панелей

Панели: `sections`, `open`, `favorites`, `history`, `functions` и `tools`.
Каждая стоит в раскладке один раз, `tools` — только в `top`. Массив имён —
группа панелей; не названная область остаётся пустой.

```json
{
  "op": "panels.set",
  "args": {"values": {"top": [["tools", "open"]], "left": ["sections"]}}
}
```

### Начальная страница

Шаблон — `OneColumn`, `TwoColumnsEqualWidth` или `TwoColumnsVariableWidth`;
у `OneColumn` есть только `left`. Форма — `CommonForm.Ф` или
`Вид.Имя.Form.Ф`, роль — `Role.Р`; и форма, и роль должны существовать в
наборе, иначе план отказывает; форма, которую создаёт тот же `apply`,
идёт раньше `homePage.set`. Ссылку, уже стоящую на странице (платформа
пишет некоторые формы идентификатором), операция принимает как прочитанную.
`height` по умолчанию 10, `common` — видимость для ролей вне списка, по
умолчанию `true`.

```json
{
  "op": "homePage.set",
  "args": {"values": {
    "template": "TwoColumnsEqualWidth",
    "left": [{"form": "CommonForm.РабочийСтол", "height": 15,
              "common": false, "roles": [{"role": "Role.Менеджер", "visible": true}]}],
    "right": [{"form": "Catalog.Товары.Form.ФормаСписка"}]
  }}
}
```

### Роли по умолчанию

Канонической операции нет — это пробел контракта Unica MCP. Прежний DSL
выражал их значениями `add-defaultRole` и `set-defaultRoles`; описание
формата осталось в `reference.md` как справочник, вызовом оно не является.
