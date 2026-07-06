---
name: form-compile
description: Компиляция управляемой формы 1С из JSON-определения или из метаданных объекта. Используй когда нужно создать форму с нуля по описанию элементов или по стандартному пресету объекта
argument-hint: <JsonPath> <OutputPath> | FromObject <OutputPath>
allowed-tools:
  - Bash
  - Read
  - Write
  - Glob
---

# /form-compile — Генерация Form.xml

## MCP routing

- Preferred path: use MCP `unica` tool `unica.form.compile`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.form.compile`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.
- Vendor support guard runs inside `unica`; if it blocks a locked/read-only supported object, prefer CFE/release-support or an explicit support-state change plan instead of editing raw support metadata.

Режимы:
1. **JSON DSL** — из JSON-определения формы
2. **From object** — автоматически из метаданных объекта 1С по стандартному пресету; `OutputPath` должен указывать на `.../TypePlural/ObjectName/Forms/FormName/Ext/Form.xml`

> **При проектировании формы с нуля (5+ элементов или нечёткие требования)** — используй справочник `form-patterns`. Для простых форм (1-3 поля) — не нужно.

## Параметры

| Параметр   | Обязательный | Описание                        |
|------------|:------------:|---------------------------------|
| JsonPath   | режим JSON   | Путь к JSON-определению формы   |
| FromObject | режим object | Флаг генерации по метаданным объекта |
| ObjectPath | нет          | Путь к XML объекта; если не указан, `unica` выводит его из `OutputPath` |
| Purpose    | нет          | Назначение формы; если не указано, `unica` выводит его из имени формы |
| OutputPath | да           | Путь к выходному Form.xml       |

## MCP вызов

### JSON DSL из файла

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.form.compile",
    "arguments": {
      "cwd": "<workspace>",
      "JsonPath": "<json>",
      "OutputPath": "<Form.xml>",
      "dryRun": false
    }
  }
}
```

### JSON DSL в форме объекта конфигурации

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.form.compile",
    "arguments": {
      "cwd": "<workspace>",
      "JsonPath": "<json>",
      "OutputPath": "<.../TypePlural/ObjectName/Forms/FormName/Ext/Form.xml>",
      "dryRun": false
    }
  }
}
```

### From object

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.form.compile",
    "arguments": {
      "cwd": "<workspace>",
      "FromObject": true,
      "OutputPath": "<.../Catalogs/Валюты/Forms/ФормаЭлемента/Ext/Form.xml>",
      "dryRun": false
    }
  }
}
```

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
- `events` — обработчики событий формы (ключ: имя события 1С, значение: имя процедуры)
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
| `"label"`    | LabelDecoration   | имя (текст задаётся через `title`)                |
| `"labelField"` | LabelField      | имя                                               |
| `"table"`    | Table             | имя                                               |
| `"pages"`    | Pages             | имя                                               |
| `"page"`     | Page              | имя                                               |
| `"button"`   | Button            | имя                                               |
| `"picture"`  | PictureDecoration | имя                                               |
| `"picField"` | PictureField      | имя                                               |
| `"calendar"` | CalendarField     | имя                                               |
| `"cmdBar"`   | CommandBar        | имя                                               |
| `"autoCmdBar"` | AutoCommandBar формы | имя — наполняет главную АКП формы (id=-1), не попадает в `<ChildItems>` |
| `"popup"`    | Popup             | имя                                               |

### Общие свойства (все типы элементов)

| Ключ | Описание |
|------|----------|
| `name` | Переопределить имя (по умолчанию = значение ключа типа) |
| `title` | Заголовок элемента |
| `visible: false` | Скрыть (синоним: `hidden: true`) |
| `enabled: false` | Сделать недоступным (синоним: `disabled: true`) |
| `readOnly: true` | Только чтение |
| `on: [...]` | События с автоименованием обработчиков |
| `handlers: {...}` | Явное задание имён обработчиков: `{"OnChange": "МоёИмя"}` |

### Допустимые имена событий (`on`)

Компилятор предупреждает о неизвестных событиях. Имена регистрозависимы — используйте точно как указано.

**Форма** (`events`): `OnCreateAtServer`, `OnOpen`, `BeforeClose`, `OnClose`, `NotificationProcessing`, `ChoiceProcessing`, `OnReadAtServer`, `BeforeWriteAtServer`, `OnWriteAtServer`, `AfterWriteAtServer`, `BeforeWrite`, `AfterWrite`, `FillCheckProcessingAtServer`, `BeforeLoadDataFromSettingsAtServer`, `OnLoadDataFromSettingsAtServer`, `ExternalEvent`, `Opening`

**input / picField**: `OnChange`, `StartChoice`, `ChoiceProcessing`, `AutoComplete`, `TextEditEnd`, `Clearing`, `Creating`, `EditTextChange`

**check**: `OnChange`

**table**: `OnStartEdit`, `OnEditEnd`, `OnChange`, `Selection`, `ValueChoice`, `BeforeAddRow`, `BeforeDeleteRow`, `AfterDeleteRow`, `BeforeRowChange`, `BeforeEditEnd`, `OnActivateRow`, `OnActivateCell`, `Drag`, `DragStart`, `DragCheck`, `DragEnd`

**label / picture**: `Click`, `URLProcessing`

**labelField**: `OnChange`, `StartChoice`, `ChoiceProcessing`, `Click`, `URLProcessing`, `Clearing`

**button**: `Click`

**pages**: `OnCurrentPageChange`

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

### Надпись-декорация (label)

| Ключ | Описание |
|------|----------|
| `title` | Текст надписи (обязательно) |
| `hyperlink: true` | Сделать ссылкой |
| `width` / `height` | Размер |

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
| `children: [...]` | Кнопки/popup |

```json
{ "autoCmdBar": "ФормаКоманднаяПанель", "autofill": true, "children": [
   { "button": "ИзменитьВыделенные", "command": "ИзменитьВыделенные",
     "locationInCommandBar": "InAdditionalSubmenu" }
]}
```

### Выпадающее меню (popup)

| Ключ | Описание |
|------|----------|
| `title` | Заголовок подменю |
| `children: [...]` | Кнопки подменю |

Используется внутри `cmdBar` для группировки кнопок в подменю:
```json
{ "cmdBar": "Панель", "children": [
  { "popup": "Добавить", "title": "Добавить", "children": [
    { "button": "ДобавитьСтроку", "stdCommand": "Товары.Add" },
    { "button": "ДобавитьИзДокумента", "command": "ДобавитьИзДокумента", "title": "Из документа" }
  ]}
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

Также допустимы: `ChartOfAccountsRef/Object`, `ChartOfCharacteristicTypesRef/Object`, `ChartOfCalculationTypesRef/Object`, `ExchangePlanRef/Object`, `BusinessProcessRef/Object`, `TaskRef/Object`, `AccountingRegisterRecordSet`, `InformationRegisterRecordManager`, `ConstantsSet`.

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

1. **Компиляция**: `unica.form.compile` генерирует `Form.xml` и автоматически регистрирует `<Form>` в `ChildObjects` родительского объекта (если OutputPath следует конвенции `.../TypePlural/ObjectName/Forms/FormName/Ext/Form.xml`).
2. **Метаданные формы** (`ФормаСписка.xml`) и `Module.bsl` создаёт `unica.form.add`. Если `unica.form.add` ещё не вызывался — вызови его после `unica.form.compile`. Он не перезаписывает существующий Form.xml.
3. **Проверка**: `unica.form.validate`, затем `unica.form.info`.

## Верификация

### Проверка корректности XML

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.form.validate",
    "arguments": {
      "cwd": "<workspace>",
      "FormPath": "<OutputPath>"
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
    "name": "unica.form.info",
    "arguments": {
      "cwd": "<workspace>",
      "FormPath": "<OutputPath>"
    }
  }
}
```

## Особенности для внешних обработок (EPF)

- **Тип главного реквизита**: `ExternalDataProcessorObject.ИмяОбработки` (не `DataProcessorObject`)
- **DataPath**: используйте реквизиты формы (`ИмяРеквизита`), а не `Объект.ИмяРеквизита` — у внешних обработок нет реквизитов объекта в метаданных
- **Ссылочные типы**: `CatalogRef.XXX`, `DocumentRef.XXX` допустимы в XML, но для публикации EPF потребуется база с целевой конфигурацией; используй `v8-runner` skill и `unica.runtime.execute` с `operation=make` по external source-set
