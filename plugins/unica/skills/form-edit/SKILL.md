---
name: form-edit
description: Добавление и удаление элементов, реквизитов и команд в существующей управляемой форме 1С. Используй когда нужно точечно модифицировать готовую форму
allowed-tools:
  - Bash
  - Read
  - Write
  - Glob
---

# /form-edit — Редактирование формы

## MCP routing

- Preferred path: use MCP `unica` tool `unica.apply` с операциями формы; адрес
  цели — логический, файлового селектора у поверхности нет.
- Не зови внутренние адаптеры напрямую: они спрятаны за MCP `unica`.
- Всегда сначала `dryRun: true`; `dryRun: false` — только по явной просьбе
  пользователя и только с `ifRev` из превью.
- Словарь операций узла даёт `unica.view {at}` в секции `can`: что не названо
  там, того поверхность не пишет.
- Поддержку объекта проверяет сама операция; заблокированный поставщиком
  объект правится через расширение, а не правкой метаданных поддержки.

Добавляет и удаляет элементы, реквизиты и команды существующей управляемой
формы. Идентификаторы, companion-элементы (`ContextMenu`, `ExtendedTooltip` и
прочие) и привязки событий собирает сама операция.

## Адрес и операции

Цель — узел формы: `<набор>:<Вид>.<Имя>.Form.<Форма>`. Имя набора даёт
`unica.view {}`, адрес по имени объекта — `unica.search {corpus: "names"}`.

| Операция | Что делает |
|---|---|
| `element.add` | добавляет элементы; `items[]` несёт то же описание, что раздел «JSON формат» ниже |
| `element.remove` | удаляет элемент, названный адресом (`…Form.Ф.Item.X`) или списком `items` |
| `formAttribute.add` | добавляет реквизит формы |
| `formCommand.add` | добавляет команду формы |
| `event.bind` | привязывает обработчик к событию формы или элемента |
| `form.add`, `form.set`, `form.remove` | состав форм объекта и их свойства |

## MCP вызов

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Catalog.Номенклатура.Form.ФормаЭлемента",
      "ops": [
        {
          "op": "element.add",
          "args": {
            "items": [
              {"input": "Артикул", "path": "Объект.Артикул", "into": "ГруппаШапка"}
            ]
          }
        }
      ],
      "dryRun": true
    }
  }
}
```

Применение — тот же вызов с `dryRun: false` и `ifRev` из превью.

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

`element.remove` называет цель адресом — `…Form.<Форма>.Item.<Имя>` — либо
списком имён в `items`:

```json
{
  "op": "element.remove",
  "args": {"items": [{"name": "Товары"}]}
}
```

Ключ `removeElements` — внутренняя форма описания, в которую операция
разворачивает эти имена; аргументом `element.remove` он не является и в вызове
отклоняется как неизвестное поле. Гарантии ниже описаны для обеих форм.

- Элемент сопоставляется с атрибутом XML `name` точно, с учётом регистра и пробелов. Поиск по префиксу и нормализация имени не выполняются.
- В записи разрешено только строковое непустое поле `name`. Параметров `includeCompanions` и `ifMissing` нет: они отклоняются как неизвестные поля.
- Удаляется всё структурное XML-поддерево элемента. Вложенные элементы и contained companions (`ContextMenu`, `ExtendedTooltip`, `AutoCommandBar` и другие узлы внутри поддерева) удаляются вместе с владельцем и перечисляются в результате с `reason: "contained"`.
- По умолчанию отсутствующая цель завершает весь вызов ошибкой `FORM_ELEMENT_NOT_FOUND`; повторное удаление отсутствующего элемента не является idempotent no-op.
- Удалять можно только публичный элемент рабочего дерева формы, непосредственно принадлежащий контейнеру `ChildItems`. Корневой `AutoCommandBar`, отдельный companion внутри владельца, baseline внутри `BaseForm` и другие именованные узлы вне рабочего дерева защищены; неоднозначные и перекрывающиеся цели также отклоняются.
- До публикации проверяются конфликты с тем же `definition` (`elements`, вложенные `children`/`columns`, `into`, `after`, `elementEvents`) и поддерживаемые ссылки в остающемся рабочем XML: binding paths вида `Items.<name>.CurrentData...` (включая имена с точками), `Form.Item.<name>.StandardCommand.*` и `AdditionSource/Item`.
- Если сохраняемый элемент ссылается на удаляемый contained companion, весь вызов завершается атомарной ошибкой `FORM_EDIT_REMOVE_SURVIVING_REFERENCE`: companion не отделяется от владельца и не удаляется частично. Ссылки из baseline `BaseForm` не блокируют изменение рабочего дерева, а сам baseline не редактируется.
- Проверка ссылок намеренно не анализирует BSL и не переписывает обращения к элементу в `Module.bsl`. Такие ссылки нужно найти и изменить отдельно до удаления.
- Весь batch атомарен: планирование, проверки ссылок и полная валидация спроецированного XML завершаются до фиксации транзакции. Apply дополнительно повторяет проверку формы (`unica.check` на узле формы) после записи внутри транзакции; при любой ошибке Form.xml не меняется.

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

Словарь пишет десять видов:

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
| `commandBar` | CommandBar | — |

Группы и таблицы поддерживают `children`/`columns` для вложенных элементов.

### Чего словарь не пишет

Остальные виды платформенных полей и декораций **поверхность не создаёт**:
поля картинки, текстового, табличного, HTML и форматированного документа,
диаграммы и сводной диаграммы, диаграммы Ганта, календаря, периода,
индикатора, ползунка, географической схемы, дендрограммы, планировщика,
радиокнопки, а также декоративную картинку и поле поиска.

Существующий элемент такого вида **читается** `unica.view` на узле формы и
переживает правку соседей; создать или переписать его этой операцией нельзя.
Если задача требует именно такого элемента — сообщи об этом как о пробеле
контракта Unica MCP и не подменяй его другим видом.

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

1. `unica.view` на узле формы — текущая структура и секция `can`
2. Собрать `ops` по описанию ниже
3. `unica.apply` с `dryRun: true` — план и `rev`
4. `unica.apply` с `dryRun: false` и `ifRev` — применение
5. `unica.check` на узле формы — проверка, затем `unica.view` — убедиться, что структура изменилась правильно
