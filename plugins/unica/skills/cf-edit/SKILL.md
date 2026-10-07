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
узле, называет `unica.view {at}` в секции `can`; ниже — те операции, которыми
выражается предмет этого скилла:

| Операция | Предмет |
|---|---|
| `props.set` | свойства корня: версия, поставщик, режим совместимости и прочие пары «ключ — значение» |
| `object.create`, `object.remove` | состав конфигурации: объект метаданных заводится по виду и имени, снимается по своему адресу |
| `subsystemOrder.set` | порядок подсистем верхнего уровня: `values: {subsystems: […]}`; правила перестановки — в `/unica:interface-edit` |

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

`commandVisibility.set`, `commandPlacement.set`, `commandOrder.set` и
`groupOrder.set` на `<набор>:Configuration` отклоняются: их адрес —
интерфейс подсистемы, например `main:Subsystem.Продажи.Interface`; вложенная
подсистема адресуется всей цепочкой. Маршрута `main:Interface` у корня нет.
Эти правки описаны в `/unica:interface-edit`.

**Роли по умолчанию, панели и стартовая страница** канонической операции не
имеют: их правка — пробел контракта Unica MCP, сообщай о нём, а не подменяй
соседней операцией.

## Операции

| Операция | Формат Value | Описание |
|----------|-------------|----------|
| `modify-property` | `Ключ=Значение` (batch `;;`) | Изменить свойство |
| `add-childObject` | `Type.Name` (batch `;;`) | Зарегистрировать уже существующий файл объекта в ChildObjects. Для создания нового объекта используй `/unica:meta-add`, `/unica:role-compile`, `/unica:subsystem-compile` — они регистрируют автоматически |
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

### Роли по умолчанию, панели, стартовая страница

Канонической операции нет — это пробел контракта Unica MCP. Прежний DSL
выражал их значениями `add-defaultRole`, `set-panels`, `set-home-page`;
описание формата осталось в `reference.md` как справочник, вызовом оно не
является.
