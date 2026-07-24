---
name: form-edit
description: Добавление и удаление элементов, реквизитов и команд в существующей управляемой форме 1С. Используй когда нужно точечно модифицировать готовую форму
argument-hint: <FormPath> (<JsonPath> | <definition>)
allowed-tools:
  - Bash
  - Read
  - Write
  - Glob
---

# /form-edit — Редактирование формы

## MCP routing

- Preferred path: use MCP `unica` tool `unica.form.edit`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.form.edit`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.
- Vendor support guard runs inside `unica`; if it blocks a locked/read-only supported object, prefer CFE/release-support or an explicit support-state change plan instead of editing raw support metadata.

Добавляет или удаляет элементы, реквизиты и/или команды в существующем Form.xml. Автоматически выделяет ID из правильного пула, генерирует companion-элементы (ContextMenu, ExtendedTooltip, и др.) и обработчики событий.

## Использование

Используй MCP `unica` tool `unica.form.edit` с `FormPath` и ровно одним источником изменений: `JsonPath` или inline-объектом `definition`.

## Параметры

| Параметр | Обязательный | Описание |
|----------|:------------:|----------|
| FormPath | да | Путь к существующему Form.xml |
| JsonPath | один из двух | Путь к JSON с описанием изменений |
| definition | один из двух | То же описание как inline JSON object |

## MCP вызов

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.form.edit",
    "arguments": {
      "cwd": "<workspace>",
      "FormPath": "src/Catalogs/Номенклатура/Forms/ФормаЭлемента/Ext/Form.xml",
      "JsonPath": "forms/patch-add-article.json",
      "dryRun": false
    }
  }
}
```

Для небольшого изменения можно передать DSL без временного файла:

```json
{
  "FormPath": "src/Catalogs/Номенклатура/Forms/ФормаЭлемента/Ext/Form.xml",
  "definition": {
    "formEvents": [
      { "name": "OnCreateAtServer", "handler": "ПриСозданииНаСервере" }
    ]
  },
  "dryRun": false
}
```

## JSON формат

```json
{
  "into": "ГруппаШапка",
  "after": "Контрагент",
  "elements": [
    { "input": "Склад", "path": "Объект.Склад", "on": ["OnChange"] }
  ],
  "attributes": [
    { "name": "СуммаИтого", "type": "decimal(15,2)" }
  ],
  "commands": [
    { "name": "Рассчитать", "action": "РассчитатьОбработка" }
  ]
}
```

### Расширения (extension-формы)

Для заимствованных форм (с `<BaseForm>`) автоматически активируется extension-режим: ID начинаются с 1000000+. Доступны дополнительные секции:

```json
{
  "formEvents": [
    { "name": "OnCreateAtServer", "handler": "Расш1_ПриСозданииПосле", "callType": "After" },
    { "name": "OnOpen", "handler": "Расш1_ПриОткрытии", "callType": "Before" }
  ],
  "elementEvents": [
    { "element": "Банк", "name": "OnChange", "handler": "Расш1_БанкПриИзменении", "callType": "Before" }
  ],
  "commands": [
    { "name": "Подбор", "action": "Расш1_ПодборПосле", "callType": "After" },
    { "name": "Запрос", "actions": [
      { "callType": "Before", "handler": "Расш1_ЗапросПеред" },
      { "callType": "After", "handler": "Расш1_ЗапросПосле" }
    ]}
  ],
  "elements": [
    { "input": "Поле", "path": "Объект.Поле", "on": [{ "event": "OnChange", "callType": "After" }] }
  ]
}
```

### Позиционирование элементов

| Ключ | По умолчанию | Описание |
|------|-------------|----------|
| `into` | корневой ChildItems | Имя группы/таблицы/страницы, куда вставлять |
| `after` | в конец | Имя элемента, после которого вставлять |

### Удаление элементов

Строгий контракт удаления содержит только точное имя:

```json
{
  "removeElements": [
    { "name": "Товары" }
  ]
}
```

- Элемент сопоставляется с атрибутом XML `name` точно, с учётом регистра и пробелов. Поиск по префиксу и нормализация имени не выполняются.
- В записи разрешено только строковое непустое поле `name`. Параметров `includeCompanions` и `ifMissing` нет: они отклоняются как неизвестные поля.
- Удаляется всё структурное XML-поддерево элемента. Вложенные элементы и contained companions (`ContextMenu`, `ExtendedTooltip`, `AutoCommandBar` и другие узлы внутри поддерева) удаляются вместе с владельцем и перечисляются в результате с `reason: "contained"`.
- По умолчанию отсутствующая цель завершает весь вызов ошибкой `FORM_ELEMENT_NOT_FOUND`; повторное удаление отсутствующего элемента не является idempotent no-op.
- Удалять можно только публичный элемент рабочего дерева формы, непосредственно принадлежащий контейнеру `ChildItems`. Корневой `AutoCommandBar`, отдельный companion внутри владельца, baseline внутри `BaseForm` и другие именованные узлы вне рабочего дерева защищены; неоднозначные и перекрывающиеся цели также отклоняются.
- До публикации проверяются конфликты с тем же `definition` (`elements`, вложенные `children`/`columns`, `into`, `after`, `elementEvents`) и поддерживаемые ссылки в остающемся рабочем XML: binding paths вида `Items.<name>.CurrentData...` (включая имена с точками), `Form.Item.<name>.StandardCommand.*` и `AdditionSource/Item`.
- Если сохраняемый элемент ссылается на удаляемый contained companion, весь вызов завершается атомарной ошибкой `FORM_EDIT_REMOVE_SURVIVING_REFERENCE`: companion не отделяется от владельца и не удаляется частично. Ссылки из baseline `BaseForm` не блокируют изменение рабочего дерева, а сам baseline не редактируется.
- Проверка ссылок намеренно не анализирует BSL и не переписывает обращения к элементу в `Module.bsl`. Такие ссылки нужно найти и изменить отдельно до удаления.
- Весь batch атомарен: планирование, проверки ссылок и полная валидация спроецированного XML завершаются до фиксации транзакции. Apply дополнительно повторяет `unica.form.validate` после записи внутри транзакции; при любой ошибке Form.xml не меняется.

Preview и apply возвращают одинаковую типизированную форму `data`:

```json
{
  "changed": true,
  "removed": [
    { "name": "Товары", "kind": "Table", "reason": "requested" },
    { "name": "ТоварыКонтекстноеМеню", "kind": "ContextMenu", "reason": "contained" }
  ],
  "validation": "passed"
}
```

При preview (`dryRun: true`, значение по умолчанию) файл и cache events не меняются. Preview, apply и no-op проходят одну полную валидацию спроецированного XML до возврата `validation: "passed"`. Apply (`dryRun: false`) публикует только успешно проверенный результат и инвалидирует кэш событием `FormChanged`, только если `changed: true`. Валидный idempotent no-op без удаления сохраняет исходные байты, возвращает `changed: false`, пустой `removed` и не создаёт cache event; невалидный исходный XML завершается ошибкой.

### Типы элементов

Те же DSL-ключи, что в `unica.form.compile`:

| Ключ | XML тег | Companions |
|------|---------|------------|
| `input` | InputField | ContextMenu, ExtendedTooltip |
| `check` | CheckBoxField | ContextMenu, ExtendedTooltip |
| `label` | LabelDecoration | ContextMenu, ExtendedTooltip |
| `labelField` | LabelField | ContextMenu, ExtendedTooltip |
| `group` | UsualGroup | ExtendedTooltip |
| `table` | Table | ContextMenu, AutoCommandBar, Search*, ViewStatus* |
| `pages` | Pages | ExtendedTooltip |
| `page` | Page | ExtendedTooltip |
| `button` | Button | ExtendedTooltip |

Группы и таблицы поддерживают `children`/`columns` для вложенных элементов.

### Свойства таблицы в `form.edit`

- `representation`: `List` \| `Tree` \| `HierarchicalList`
- `autoInsertNewRow`: `true` \| `false`

### Кнопки: command и stdCommand

- `"command": "ИмяКоманды"` → `Form.Command.ИмяКоманды`
- `"stdCommand": "Close"` → `Form.StandardCommand.Close`
- `"stdCommand": "Товары.Add"` → `Form.Item.Товары.StandardCommand.Add` (стандартная команда элемента)

### Допустимые события (`on`)

Editor до записи проверяет событие по единой платформенной матрице. Недопустимое сочетание возвращает `ok=false` и код `FORM_EVENT_*`, не меняя файл. Основные сочетания:

- **input**: `OnChange`, `StartChoice`, `ChoiceProcessing`, `Clearing`, `AutoComplete`, `TextEditEnd`, `Opening`, `Creating`, `EditTextChange`
- **check**: `OnChange`
- **table**: `OnStartEdit`, `OnEditEnd`, `OnChange`, `Selection`, `BeforeAddRow`, `BeforeDeleteRow`, `OnActivateRow`
- **label**: `Click`, `URLProcessing`
- **picture**: `Click`, `Drag`, `DragCheck`
- **pages**: `OnCurrentPageChange`
- **page/button/group/command bar**: события не поддерживаются

События `table` требуют непустой привязки: `path` для нового элемента или прямого `DataPath` у существующего `Table`. Платформа удаляет обработчики событий у несвязанной таблицы при загрузке/выгрузке конфигурации.

`OnReadAtServer`, `BeforeWrite`, `BeforeWriteAtServer`, `OnWriteAtServer`, `AfterWriteAtServer` и `AfterWrite` разрешены только при подтверждённом persistent object/record типе главного реквизита. Для `DataProcessorObject`, `ReportObject`, `DynamicList` и неизвестного контекста они отклоняются. `NewWriteProcessing` и `FillCheckProcessingAtServer` являются общими событиями формы и этим ограничением не связаны.

### Система типов (для attributes)

`string`, `string(100)`, `decimal(15,2)`, `boolean`, `date`, `dateTime`, `CatalogRef.XXX`, `DocumentObject.XXX`, `ValueTable`, `DynamicList`, `Type1 | Type2` (составной).

### Секции расширений

| Секция | Назначение |
|--------|-----------|
| `formEvents` | События уровня формы; `callType` только для расширения |
| `elementEvents` | События существующих элементов; `callType` только для расширения |
| `callType` на `commands` | callType на Action команды |
| `callType` на `on` | callType на событиях новых элементов (объектный формат) |

Все extension-секции опциональны — без них навык работает как с обычными формами.

Повтор идентичного binding является явным idempotent no-op. Конфликт обработчика/`callType`, duplicate, отсутствующий элемент и любой недопустимый event отклоняют весь batch до мутации; `dryRun: true` использует тот же planner.

## Workflow

1. `unica.form.info` — посмотреть текущую структуру формы
2. Создать JSON с описанием изменений
3. `unica.form.edit` — применить изменение
4. `unica.form.validate` — проверить корректность
5. `unica.form.info` — убедиться, что структура изменилась правильно
