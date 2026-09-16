---
name: form-compile
description: Компиляция управляемой формы 1С из JSON-определения или из метаданных объекта. Используй когда нужно создать форму с нуля по описанию элементов или по стандартному пресету объекта
allowed-tools:
  - Bash
  - Read
  - Write
  - Glob
---

# /form-compile — Генерация Form.xml

## MCP routing

- Preferred path: use MCP `unica` tool `unica.apply`: форма заводится
  операцией `form.create`, наполняется `element.add`, `formAttribute.add`,
  `formCommand.add`, `event.bind`.
- Не зови внутренние адаптеры напрямую: они спрятаны за MCP `unica`.
- Всегда сначала `dryRun: true`; `dryRun: false` — только по явной просьбе
  пользователя и только с `ifRev` из превью.
- Словарь операций узла даёт `unica.view {at}` в секции `can`.

## Шаг 1 — завести форму

Адрес называет будущую форму: `<набор>:<Вид>.<Имя>.Form.<Форма>`; `values.type`
задаёт назначение, `values.name` обязано совпасть с именем в адресе.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Catalog.Валюты.Form.ФормаЭлемента",
      "ops": [
        {"op": "form.create", "args": {"values": {"name": "ФормаЭлемента", "type": "ObjectForm"}}}
      ],
      "dryRun": true
    }
  }
}
```

## Шаг 2 — наполнить

Применение первого шага публикует форму; после него её адрес принимает
элементы, реквизиты и команды. Порядок тот же, что у всякой операции:
превью → применение с `ifRev` → превью следующего шага → применение.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Catalog.Валюты.Form.ФормаЭлемента",
      "ops": [
        {"op": "formAttribute.add", "args": {"items": [{"name": "Объект", "type": "CatalogObject.Валюты", "main": true}]}},
        {"op": "element.add", "args": {"items": [{"input": "Наименование", "path": "Объект.Наименование"}]}}
      ],
      "dryRun": true
    }
  }
}
```

Точечные правки готовой формы держит `form-edit`; DSL ниже описывает, как
выражать элементы в `items`.

## Чего словарь не пишет

Сборки целой формы из готового JSON-определения одним вызовом на поверхности
**нет**: DSL ниже остаётся справочником формата. Виды элементов, которые
словарь создаёт, и виды, которые только читаются, перечислены в `form-edit`.
Форма, которую не собрать этими операциями, — пробел контракта Unica MCP.

## JSON DSL — справка

### Структура верхнего уровня

```json
{
  "title": "Заголовок формы",
  "properties": { "autoTitle": false, ... },
  "events": { "OnCreateAtServer": "ПриСозданииНаСервере" },
  "excludedCommands": ["Reread"],
  "elements": [ ... ],
  "attributes": [ ... ],
  "commands": [ ... ],
  "parameters": [ ... ]
}
```

- `title` — заголовок формы (multilingual). Можно указать и в `properties`, но лучше на верхнем уровне
- `properties` — свойства формы: `autoTitle`, `windowOpeningMode`, `commandBarLocation`, `saveDataInSettings`, `width`, `height` и др.
- `events` — обработчики событий формы (ключ: имя события 1С, значение: непустое строковое имя процедуры). Этот формат не поддерживает `callType`.
- `excludedCommands` — исключённые стандартные команды

### Новые возможности DSL

- Форма отчёта: `reportResult`, `reportFormType`, `reportResultViewMode`.
- Управление выбором: `choiceParameters`, `choiceParameterLinks`, `availableTypes`.
- Служебные элементы и панели: `extendedTooltip`, `commandBar`, `contextMenu`, `mobileCommandBarContent`.
- Ролевое ограничение элементов и команд: `roles`.
- Командный интерфейс и навигация: `CommandInterface`, `NavigationPanel`.
- Специальные визуальные типы: `chart`, `GanttChart`.
- Динамические списки: `dynamicDataRead` в настройках реквизита `DynamicList`.

### Элементы (ключ определяет тип)

| DSL ключ     | XML элемент       | Значение ключа                                    |
|--------------|-------------------|---------------------------------------------------|
| `"group"`    | UsualGroup        | `"horizontal"` / `"vertical"` / `"alwaysHorizontal"` / `"alwaysVertical"` / `"collapsible"` |
| `"input"`    | InputField        | имя элемента                                      |
| `"check"`    | CheckBoxField     | имя                                               |
| `"label"`    | LabelDecoration   | имя — надпись-декорация; текст задаётся `title`, гиперссылка `hyperlink` |
| `"labelField"` | LabelField      | имя                                               |
| `"table"`    | Table             | имя                                               |
| `"pages"`    | Pages             | имя                                               |
| `"page"`     | Page              | имя                                               |
| `"button"`   | Button            | имя                                               |
| `"cmdBar"`   | CommandBar        | имя                                               |
| `"autoCmdBar"` | AutoCommandBar формы | имя — наполняет главную АКП формы (id=-1), не попадает в `<ChildItems>` |

### Общие свойства (все типы элементов)

| Ключ | Описание |
|------|----------|
| `name` | Переопределить имя (по умолчанию = значение ключа типа) |
| `title` | Заголовок элемента |
| `visible: false` | Скрыть (синоним: `hidden: true`) |
| `enabled: false` | Сделать недоступным (синоним: `disabled: true`) |
| `readOnly: true` | Только чтение |
| `on: [...]` | События с автоименованием обработчиков (если они допустимы для типа элемента) |
| `handlers: {...}` | Явное задание имён обработчиков: `{"OnChange": "МоёИмя"}` |

### Допустимые имена событий (`on`)

Компилятор отклоняет незарегистрированные события. Имена регистрозависимы — используйте точно как указано.

**Форма** (`events`): `OnCreateAtServer`, `OnOpen`, `BeforeClose`, `OnClose`, `NotificationProcessing`, `ChoiceProcessing`, `ExternalEvent`, `OnReopen`, `OnMainServerAvailabilityChange`, `OnReadAtServer`, `BeforeWrite`, `NewWriteProcessing`, `FillCheckProcessingAtServer`, `BeforeWriteAtServer`, `OnWriteAtServer`, `AfterWriteAtServer`, `AfterWrite`, `BeforeLoadDataFromSettingsAtServer`, `OnLoadDataFromSettingsAtServer`, `OnSaveDataInSettingsAtServer`, `BeforeLoadUserSettingsAtServer`, `OnLoadUserSettingsAtServer`, `OnSaveUserSettingsAtServer`, `OnUpdateUserSettingSetAtServer`, `BeforeLoadVariantAtServer`, `OnLoadVariantAtServer`, `OnSaveVariantAtServer`, `OnChangeDisplaySettings`, `URLProcessing`, `URLListGetProcessing`, `URLGetProcessing`, `NavigationProcessing`

События `OnReadAtServer`, `BeforeWrite`, `BeforeWriteAtServer`, `OnWriteAtServer`, `AfterWriteAtServer` и `AfterWrite` допустимы только при главном реквизите формы постоянного объекта или записи.

<!-- form-event-registry:start -->
| DSL ключ | Допустимые события `on` |
|----------|--------------------------|
| `input` | `OnChange`, `StartChoice`, `Clearing`, `ChoiceProcessing`, `AutoComplete`, `TextEditEnd`, `Opening`, `Creating`, `EditTextChange`, `Tuning`, `StartListChoice`, `MultipleValuesDelete` |
| `check` | `OnChange` |
| `labelField` | `URLProcessing`, `Click`, `OnChange` |
| `table` | `Selection`, `OnActivateRow`, `BeforeAddRow`, `BeforeDeleteRow`, `OnStartEdit`, `OnChange`, `BeforeRowChange`, `AfterDeleteRow`, `OnEditEnd`, `OnActivateCell`, `OnGetDataAtServer`, `Drag`, `DragCheck`, `ValueChoice`, `ChoiceProcessing`, `DragStart`, `BeforeEditEnd`, `BeforeExpand`, `DragEnd`, `OnUpdateUserSettingSetAtServer`, `BeforeCollapse`, `BeforeLoadUserSettingsAtServer`, `OnActivateField`, `RefreshRequestProcessing`, `NewWriteProcessing`, `OnLoadUserSettingsAtServer`, `OnCurrentParentChange`, `OnSaveUserSettingsAtServer`, `URLGetProcessing` |
| `pages` | `OnCurrentPageChange` |
| `page` | — |
| `button` | — |
| `cmdBar` | — |
| `autoCmdBar` | — |
| `group` | — |
<!-- form-event-registry:end -->

### Поле ввода (input)

| Ключ | Описание | Пример |
|------|----------|--------|
| `path` | DataPath — привязка к данным | `"Объект.Организация"` |
| `titleLocation` | Размещение заголовка | `"none"`, `"left"`, `"top"` |
| `multiLine: true` | Многострочное поле | текстовое поле, комментарий |
| `passwordMode: true` | Режим пароля (звёздочки) | поле ввода пароля |
| `choiceButton: true` | Кнопка выбора ("...") | ссылочное поле |
| `clearButton: true` | Кнопка очистки ("X") | |
| `spinButton: true` | Кнопка прокрутки | числовые поля |
| `dropListButton: true` | Кнопка выпадающего списка | |
| `markIncomplete: true` | Пометка незаполненного | обязательные поля |
| `skipOnInput: true` | Пропускать при обходе Tab | |
| `inputHint` | Подсказка в пустом поле | `"Введите наименование..."` |
| `width` / `height` | Размер | числа |
| `autoMaxWidth: false` | Отключить авто-ширину | для фиксированных полей |
| `horizontalStretch: true` | Растягивать по ширине | |

### Чекбокс (check)

| Ключ | Описание |
|------|----------|
| `path` | DataPath |
| `titleLocation` | Размещение заголовка |

### Группа (group)

Значение ключа задаёт ориентацию: `"horizontal"`, `"vertical"`, `"alwaysHorizontal"`, `"alwaysVertical"`, `"collapsible"`.

| Ключ | Описание |
|------|----------|
| `showTitle: true` | Показывать заголовок группы |
| `united: false` | Не объединять рамку |
| `representation` | `"none"`, `"normal"`, `"weak"`, `"strong"` |
| `children: [...]` | Вложенные элементы |

### Таблица (table)

**Важно**: таблица требует связанный реквизит формы типа `ValueTable` с колонками (см. раздел "Связки").

| Ключ | Описание |
|------|----------|
| `path` | DataPath (привязка к реквизиту-таблице) |
| `columns: [...]` | Колонки — массив элементов (обычно `input`) |
| `representation` | `"List"`, `"Tree"` или `"HierarchicalList"` (регистр нормализуется) |
| `autoInsertNewRow: true` | Автоматически добавлять новую строку; `false` является платформенным default и XML-тег не создаёт |
| `changeRowSet: true` | Разрешить добавление/удаление строк |
| `changeRowOrder: true` | Разрешить перемещение строк |
| `height` | Высота в строках таблицы |
| `header: false` | Скрыть шапку |
| `footer: true` | Показать подвал |
| `commandBarLocation` | `"None"`, `"Top"`, `"Auto"` |
| `searchStringLocation` | `"None"`, `"Top"`, `"Auto"` |
| `choiceMode: true` | Режим выбора (для форм выбора) |
| `initialTreeView` | `"ExpandTopLevel"` и др. (иерархические списки) |
| `enableDrag: true` | Разрешить перетаскивание |
| `enableStartDrag: true` | Разрешить начало перетаскивания |
| `rowPictureDataPath` | Путь к картинке строки (напр. `"Список.DefaultPicture"`) |
| `tableAutofill: false` | Управление Autofill внутреннего AutoCommandBar |

### Страницы (pages + page)

| Ключ (pages) | Описание |
|------|----------|
| `pagesRepresentation` | `"None"`, `"TabsOnTop"`, `"TabsOnBottom"` и др. |
| `children: [...]` | Массив `page` |

| Ключ (page) | Описание |
|------|----------|
| `title` | Заголовок вкладки |
| `group` | Ориентация внутри страницы |
| `children: [...]` | Содержимое страницы |

### Кнопка (button)

| Ключ | Описание |
|------|----------|
| `command` | Имя команды формы → `Form.Command.Имя` |
| `stdCommand` | Стандартная команда: `"Close"` → `Form.StandardCommand.Close`; с точкой: `"Товары.Add"` → `Form.Item.Товары.StandardCommand.Add` |
| `defaultButton: true` | Кнопка по умолчанию |
| `type` | `"usual"`, `"hyperlink"`, `"commandBar"` |
| `picture` | Картинка кнопки |
| `representation` | `"Auto"`, `"Text"`, `"Picture"`, `"PictureAndText"` |
| `locationInCommandBar` | `"Auto"`, `"InCommandBar"`, `"InAdditionalSubmenu"` |

### Командная панель (cmdBar)

Дополнительная пользовательская панель команд, размещается как обычный элемент в layout формы.

| Ключ | Описание |
|------|----------|
| `autofill: true` | Автозаполнение стандартными командами |
| `children: [...]` | Кнопки панели |

### Главная автокомандная панель формы (autoCmdBar)

Наполняет встроенную AutoCommandBar формы (id=-1) кастомными кнопками. Указывать только если нужно добавить свои кнопки на главную панель или явно управлять автозаполнением.

| Ключ | Описание |
|------|----------|
| `autofill: true/false` | Автозаполнение стандартными командами |
| `horizontalAlign` | `"Left"` / `"Center"` / `"Right"` |
| `children: [...]` | Кнопки |

```json
{ "autoCmdBar": "ФормаКоманднаяПанель", "autofill": true, "children": [
   { "button": "ИзменитьВыделенные", "command": "ИзменитьВыделенные",
     "locationInCommandBar": "InAdditionalSubmenu" }
]}
```

### Реквизиты (attributes)

```json
{ "name": "Объект", "type": "DataProcessorObject.Загрузка", "main": true }
{ "name": "Список", "type": "DynamicList", "main": true, "settings": {
    "mainTable": "Catalog.Номенклатура", "dynamicDataRead": true
}}
{ "name": "Итого", "type": "decimal(15,2)" }
{ "name": "Таблица", "type": "ValueTable", "columns": [
    { "name": "Номенклатура", "type": "CatalogRef.Номенклатура" },
    { "name": "Количество", "type": "decimal(10,3)" }
]}
```

- `savedData: true` — сохраняемые данные
- `main: true` — главный реквизит формы (например, основной `*Object.*`, `DynamicList`, `*RecordSet.*`)

### Команды (commands)

```json
{ "name": "Загрузить", "action": "ЗагрузитьОбработка", "shortcut": "Ctrl+Enter" }
```

- `title` — заголовок (если отличается от name)
- `picture` — картинка команды

### Система типов

**Примитивные:**

| DSL                    | XML                                    |
|------------------------|----------------------------------------|
| `"string"` / `"string(100)"` | `xs:string` + StringQualifiers  |
| `"decimal(15,2)"`     | `xs:decimal` + NumberQualifiers        |
| `"decimal(10,0,nonneg)"` | с AllowedSign=Nonnegative           |
| `"boolean"`            | `xs:boolean`                          |
| `"date"` / `"dateTime"` / `"time"` | `xs:dateTime` + DateFractions |

**Ссылочные и объектные (`cfg:Prefix.Name`):**

| DSL | Описание |
|-----|----------|
| `"CatalogRef.XXX"` / `"CatalogObject.XXX"` | Справочник |
| `"DocumentRef.XXX"` / `"DocumentObject.XXX"` | Документ |
| `"EnumRef.XXX"` | Перечисление |
| `"DataProcessorObject.XXX"` / `"ReportObject.XXX"` | Обработка / Отчёт |
| `"InformationRegisterRecordSet.XXX"` | Набор записей регистра сведений |
| `"AccumulationRegisterRecordSet.XXX"` | Набор записей регистра накопления |
| `"DynamicList"` | Динамический список |

Также допустимы: `ChartOfAccountsRef/Object`, `ChartOfCharacteristicTypesRef/Object`, `ChartOfCalculationTypesRef/Object`, `ExchangePlanRef/Object`, `BusinessProcessRef/Object`, `TaskRef/Object`, `AccountingRegisterRecordSet`, `CalculationRegisterRecordSet`, `InformationRegisterRecordManager`, `ConstantsSet`.

**Платформенные:**

| DSL | XML |
|-----|-----|
| `"ValueTable"` | `v8:ValueTable` |
| `"ValueTree"` | `v8:ValueTree` |
| `"ValueList"` | `v8:ValueListType` |
| `"TypeDescription"` | `v8:TypeDescription` |
| `"UUID"` | `v8:UUID` |
| `"FormattedString"` | `v8ui:FormattedString` |
| `"Picture"` / `"Color"` / `"Font"` | `v8ui:*` |
| `"DataCompositionSettings"` | `dcsset:DataCompositionSettings` |
| `"Type1 \| Type2"` | составной тип (несколько `<v8:Type>`) |

**Недопустимые типы (XDTO-ошибка при загрузке):**

> `FormDataStructure`, `FormDataCollection`, `FormDataTree` — runtime-типы 1С, не существуют в XML-схеме. Вместо них используйте `CatalogObject.XXX`, `DocumentObject.XXX`, `DataProcessorObject.XXX`, `ValueTable`, `ValueTree`.

## Связки: элемент + реквизит

Таблица и некоторые поля требуют связанный реквизит. Элемент ссылается на реквизит через `path`.

**Таблица** — элемент `table` + реквизит `ValueTable`:
```json
{
  "elements": [
    { "table": "Товары", "path": "Объект.Товары", "columns": [
      { "input": "Номенклатура", "path": "Объект.Товары.Номенклатура" }
    ]}
  ],
  "attributes": [
    { "name": "Объект", "type": "DataProcessorObject.Загрузка", "main": true,
      "columns": [
        { "name": "Товары", "type": "ValueTable", "columns": [
          { "name": "Номенклатура", "type": "CatalogRef.Номенклатура" }
        ]}
      ]
    }
  ]
}
```

Или, если таблица привязана к реквизиту формы (не к Объект):
```json
{
  "elements": [
    { "table": "ТаблицаДанных", "path": "ТаблицаДанных", "columns": [
      { "input": "Наименование", "path": "ТаблицаДанных.Наименование" }
    ]}
  ],
  "attributes": [
    { "name": "ТаблицаДанных", "type": "ValueTable", "columns": [
      { "name": "Наименование", "type": "string(150)" }
    ]}
  ]
}
```

## Паттерны

### Диалог загрузки файла

```json
{
  "title": "Загрузка из файла",
  "properties": { "autoTitle": false },
  "events": { "OnCreateAtServer": "ПриСозданииНаСервере" },
  "elements": [
    { "group": "horizontal", "name": "ГруппаФайл", "children": [
      { "input": "ИмяФайла", "path": "ИмяФайла", "title": "Файл", "inputHint": "Выберите файл...", "choiceButton": true, "on": ["StartChoice"] },
      { "check": "ПерваяСтрокаЗаголовок", "path": "ПерваяСтрокаЗаголовок" }
    ]},
    { "input": "Результат", "path": "Результат", "multiLine": true, "height": 8, "readOnly": true, "title": "Лог" },
    { "group": "horizontal", "name": "ГруппаКнопок", "children": [
      { "button": "Загрузить", "command": "Загрузить", "defaultButton": true },
      { "button": "Закрыть", "stdCommand": "Close" }
    ]}
  ],
  "attributes": [
    { "name": "Объект", "type": "ExternalDataProcessorObject.ЗагрузкаИзФайла", "main": true },
    { "name": "ИмяФайла", "type": "string" },
    { "name": "ПерваяСтрокаЗаголовок", "type": "boolean" },
    { "name": "Результат", "type": "string" }
  ],
  "commands": [
    { "name": "Загрузить", "action": "ЗагрузитьОбработка", "shortcut": "Ctrl+Enter" }
  ]
}
```

### Мастер (wizard) с шагами

```json
{
  "title": "Мастер настройки",
  "properties": { "autoTitle": false },
  "elements": [
    { "pages": "СтраницыМастера", "pagesRepresentation": "None", "children": [
      { "page": "Шаг1", "title": "Параметры", "children": [
        { "input": "Параметр1", "path": "Параметр1" }
      ]},
      { "page": "Шаг2", "title": "Результат", "children": [
        { "input": "Итог", "path": "Итог", "readOnly": true }
      ]}
    ]},
    { "group": "horizontal", "name": "Навигация", "children": [
      { "button": "Назад", "command": "Назад", "title": "< Назад" },
      { "button": "Далее", "command": "Далее", "title": "Далее >" }
    ]}
  ],
  "attributes": [
    { "name": "Объект", "type": "ExternalDataProcessorObject.Мастер", "main": true },
    { "name": "Параметр1", "type": "string" },
    { "name": "Итог", "type": "string" }
  ],
  "commands": [
    { "name": "Назад", "action": "НазадОбработка" },
    { "name": "Далее", "action": "ДалееОбработка" }
  ]
}
```

### Список с фильтром и таблицей

```json
{
  "title": "Просмотр данных",
  "elements": [
    { "group": "horizontal", "name": "Фильтр", "children": [
      { "input": "Период", "path": "Период", "on": ["OnChange"] },
      { "input": "Организация", "path": "Организация", "on": ["OnChange"] }
    ]},
    { "table": "Данные", "path": "Данные", "changeRowSet": true, "columns": [
      { "input": "Дата", "path": "Данные.Дата" },
      { "input": "Сумма", "path": "Данные.Сумма" },
      { "input": "Комментарий", "path": "Данные.Комментарий" }
    ]}
  ],
  "attributes": [
    { "name": "Объект", "type": "ExternalDataProcessorObject.Просмотр", "main": true },
    { "name": "Период", "type": "date" },
    { "name": "Организация", "type": "string" },
    { "name": "Данные", "type": "ValueTable", "columns": [
      { "name": "Дата", "type": "date" },
      { "name": "Сумма", "type": "decimal(15,2)" },
      { "name": "Комментарий", "type": "string(200)" }
    ]}
  ]
}
```

## Автогенерация

- **Companion-элементы**: ContextMenu, ExtendedTooltip и др. создаются автоматически
- **Обработчики событий**: `"on": ["OnChange"]` → `ОрганизацияПриИзменении`
- **Namespace**: все 17 namespace-деклараций
- **ID**: последовательная нумерация, AutoCommandBar = id="-1"
- **Unknown keys**: выводится предупреждение о нераспознанных ключах

## Workflow

1. **Сборка**: `unica.apply` (`form.create`, затем наполнение) генерирует `Form.xml` и регистрирует `<Form>` в составе объекта-владельца, названного адресом.
2. **Метаданные формы** (`ФормаСписка.xml`) и `Module.bsl` создаёт та же `form.create`; `form.add` регистрирует форму в составе объекта, когда она заводится списком. Существующий `Form.xml` ни одна из них не перезаписывает.
3. **Проверка**: `unica.check` на узле формы, затем `unica.view` on the form node.

## Верификация

### Проверка корректности XML

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.check",
    "arguments": {
      "at": "<sourceSet>:Catalog.Валюты.Form.ФормаЭлемента"
    }
  }
}
```

### Сводка структуры формы

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.view",
    "arguments": {
      "cwd": "<workspace>",
      "at": "main:Catalog.Валюты.Form.ФормаЭлемента"
    }
  }
}
```

## Особенности для внешних обработок (EPF)

- Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
выбирай только операцию с `implemented: true` и не выдумывай аргументов
записи с `argsSchema: null`; превью исполнением не является. Не обходи
контракт прямым runner-ом.

- **Тип главного реквизита**: `ExternalDataProcessorObject.ИмяОбработки` (не `DataProcessorObject`)
- **DataPath**: используйте реквизиты формы (`ИмяРеквизита`), а не `Объект.ИмяРеквизита` — у внешних обработок нет реквизитов объекта в метаданных
- **Ссылочные типы**: `CatalogRef.XXX`, `DocumentRef.XXX` допустимы в XML, но для будущей публикации EPF потребуется база с целевой конфигурацией; runtime-публикацию сначала обнаруживать через `unica.run {}` и использовать `artifact.build` только при `implemented: true`, не угадывая аргументы при `argsSchema: null`
