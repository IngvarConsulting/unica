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
- Сначала вызови `unica.apply` с `at` и `ops`: это план без записи.
  Когда пользователь поручил внести эту правку, вызови `unica.apply` только
  с `executionToken` из `data.executionToken` успешного плана.
- Словарь операций узла даёт `unica.view {at}` в словаре `can`: что не названо
  там, того поверхность не пишет.
- Поддержку объекта проверяет сама операция; заблокированный поставщиком
  объект правится через расширение, а не правкой метаданных поддержки.

Добавляет и удаляет элементы, реквизиты и команды существующей управляемой
формы. Идентификаторы, companion-элементы (`ContextMenu`, `ExtendedTooltip` и
прочие) и привязки событий собирает сама операция.

## Адрес и операции

Предмет операции задаёт адрес `at`: форма, её элемент любой вложенности или
событие формы и элемента. Адреса элементов и событий берутся из ответа
`unica.view` на узле формы и в ветвях `Item` и `Event`; имя набора даёт
`unica.view {}`, адрес по имени объекта — `unica.search {corpus: "names"}`.

| Адрес `at` | Операции |
|---|---|
| `<набор>:<Вид>.<Имя>.Form.<Форма>` | `item.add` в корень формы, `attribute.add`, `command.add`, `form.remove` |
| `….Form.<Форма>.Item.<Группа>[.Item.<Вложенная>…]` — контейнер | `item.add` внутрь контейнера, `item.remove` самого контейнера |
| `….Item.<Поле>` — поле, декорация, кнопка | `item.remove` |
| `….Item.<ФормаКоманднаяПанель>`, `….Item.<Таблица>.Item.<ТаблицаКоманднаяПанель>` | `item.add` кнопки в автоматическую командную панель |
| `….Form.<Форма>[.Item.<Элемент>…].Event.<Событие>` | `event.bind`, `event.implement` |
| `<набор>:<Вид>.<Имя>` | `form.add`, `form.set` |

Контейнер — группа, страницы, страница, таблица, командная панель, группа
кнопок, подменю. Автоматическая командная панель формы и таблицы видна
элементом `Item` и принимает кнопки; удаляется она только вместе с
владельцем. Точный набор операций узла — в его словаре `can`. Колонки
внутри группы колонок таблицы (`ColumnGroup`) `view` пока не показывает:
их адреса нет, и правка таких колонок — пробел контракта Unica MCP.

### Закрытый контракт операции

Подробный контракт операции читается адресом `<узел>.Can.<операция>`,
например `main:Catalog.Номенклатура.Form.ФормаЭлемента.Can.item.add`.
Он перечисляет ровно те ключи, которые записывает писатель операции.
Ключ вне контракта, значение не того типа или вне перечисления получают
`bad_value` с точным путём (`ops[0].args.items[0].backColor`) до записи:
операция не теряет его молча.

`item.add` добавляет один элемент без вложенного содержимого. Ключи
элемента зависят от его `type`: например, `backColor` и `font` пишет только
кнопка, а `title` не пишут таблица и поле надписи. Вложенный элемент
добавляется отдельным `item.add` на адресе контейнера
(`….Item.<Группа>`), обработчик — `event.bind` на адресе события
(`….Item.<Элемент>.Event.<Событие>`) с явным именем. Ключи `children`,
`columns`, `elements`, `commandBar`, `autoCmdBar`, `autoCommandBar`,
`contextMenu`, `extendedTooltip`, `html`, `on` и `handlers` получают
`bad_value`, а `next` ведёт к контракту `item.add`. Командная панель
принимает только кнопки: её контракт публикует один вид `Button`. Группа и поле в ней добавляются в одном вызове
двумя операциями: вторая адресует группу через `args.at`.

`attribute.add` пишет `name`, `title` (строка), `type`, `main`,
`savedData`, `fillChecking` и `columns` реквизита-таблицы. Проверка
заполнения `"fillChecking": "ShowError"` записывается тегом `<FillCheck>`;
без ключа проверки нет. Просмотр и редактирование (`view`, `edit`),
функциональные опции, `useAlways`, `save`, `valueType`, настройки
динамического списка и видимость по ролям этой операцией не пишутся
и получают `bad_value`.

Прежние аргументы, которые называли элемент или событие, сняты:
`items[].into` у `item.add`, `items[].name`, элемент-строка `items[]`
и `values.name` у `item.remove`, `values.element` и `values.event`
у `event.bind`. Вызов с ними получает
`bad_value`, а `next` ведёт к адресу узла, который называло значение.
Прежние имена `element.add`, `element.remove`, `formAttribute.add`
и `formCommand.add` тоже сняты: вызов получает `bad_value` без записи.
Если заменившая (`item.add`, `item.remove`, `attribute.add` или
`command.add` формы) исполнима на адресе вызова или на его предке, `next`
ведёт к её контракту `<узел>.Can.<операция>` на ближайшем таком узле.
Если такого узла нет — например, `element.remove` на самой форме или
на автоматической командной панели, — отказ называет форму адреса,
а `next` ведёт к словарю узла.

Реквизит и команда формы добавляются теми же именами, что у объекта
метаданных: `attribute.add` и `command.add` на адресе формы пишут в форму,
а на адресе объекта — в его описание. Контракт по адресу
`<форма>.Can.attribute.add` — контракт реквизита формы.

`command.add` добавляет команды формы. Элемент `items[]` принимает
`name`, `title`, `tooltip`, `action`, `shortcut`, `picture`,
`representation`; другой ключ отклоняется до записи. Контракт —
`<форма>.Can.command.add`.

## MCP вызов

Поле в группу шапки — `item.add` на адресе группы. Сосед `after`
называет элемент того же контейнера, после которого встаёт новый:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Catalog.Номенклатура.Form.ФормаЭлемента.Item.ГруппаШапка",
      "ops": [
        {
          "op": "item.add",
          "args": {
            "items": [
              {"name": "Артикул", "type": "InputField", "path": "Объект.Артикул", "after": "Код"}
            ]
          }
        }
      ]
    }
  }
}
```

Обработчик события элемента — `event.bind` на адресе события. `values`
несут `handler` и, для формы расширения, `callType` (`Before`, `After`,
`Override`):

```json
{
  "at": "main:Catalog.Номенклатура.Form.ФормаЭлемента.Item.ГруппаШапка.Item.Банк.Event.OnChange",
  "ops": [{"op": "event.bind", "args": {"values": {"handler": "БанкПриИзменении"}}}]
}
```

Событие формы адресуется так же: `….Form.ФормаЭлемента.Event.OnOpen`.
Имя события в адресе — английское, как в ветви `Event` узла.
`event.implement` на том же адресе пишет и привязку, и процедуру в модуле.

В одном вызове операции исполняются по порядку над промежуточным
результатом: группа, добавленная первой операцией, принимает элементы
следующей, если та адресует её через `args.at`.

Применение — вызов только с `executionToken` из `data.executionToken` успешного плана.

### Позиционирование элементов

Новый элемент встаёт в конец контейнера, названного адресом. `after`
называет соседа в том же контейнере; элемент другого контейнера
отклоняется без записи. Командная панель принимает кнопки: `Button` в ней
записывается как `CommandBarButton`. Если в форме два элемента носят одно
имя, операция отклоняется без записи. Повторное такое же добавление ничего
не меняет.

### Удаление элементов

`item.remove` удаляет элемент, названный адресом
`…Form.<Форма>.Item.<Группа>.Item.<Имя>`; аргументов у операции нет:

```json
{
  "at": "main:Catalog.Номенклатура.Form.ФормаЭлемента.Item.Товары",
  "ops": [{"op": "item.remove"}]
}
```

Каждый элемент удаляется своей операцией со своим адресом.

- Элемент сопоставляется с атрибутом XML `name` точно, с учётом регистра и пробелов. Поиск по префиксу и нормализация имени не выполняются.
- Параметров `includeCompanions` и `ifMissing` нет: аргументы операции отклоняются.
- Удаляется всё структурное XML-поддерево элемента. Вложенные элементы и contained companions (`ContextMenu`, `ExtendedTooltip`, `AutoCommandBar` и другие узлы внутри поддерева) удаляются вместе с владельцем и перечисляются в результате с `reason: "contained"`.
- Отсутствующая цель завершает весь вызов отказом `not_found`; `next` ведёт к родителю. Повторное удаление отсутствующего элемента не является idempotent no-op.
- Удалять можно только публичный элемент рабочего дерева формы, непосредственно принадлежащий контейнеру `ChildItems`. Корневой `AutoCommandBar`, отдельный companion внутри владельца, baseline внутри `BaseForm` и другие именованные узлы вне рабочего дерева защищены; неоднозначные и перекрывающиеся цели также отклоняются.
- До публикации проверяются поддерживаемые ссылки в остающемся рабочем XML: binding paths вида `Items.<name>.CurrentData...` (включая имена с точками), `Form.Item.<name>.StandardCommand.*` и `AdditionSource/Item`.
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

При планировании (`at` и `ops`) файл и cache events не меняются. Preview, apply и no-op проходят одну полную валидацию спроецированного XML до возврата `validation: "passed"`. Apply (`executionToken`) публикует только успешно проверенный результат и инвалидирует кэш событием `FormChanged`, только если `changed: true`. Валидный idempotent no-op без удаления сохраняет исходные байты, возвращает `changed: false`, пустой `removed` и не создаёт cache event; невалидный исходный XML завершается ошибкой.

### Типы элементов

В `item.add` каждый `items[]` задаёт `name` и `type`; без `type`
создаётся поле ввода. Словарь создаёт двенадцать видов. Кроме ключей
таблицы, каждый вид принимает `after` и флаги `visible`, `hidden`,
`userVisible` (только `false`), `enabled`, `disabled`, `readOnly`.

| `type` | XML тег | Ключи вида | Companions |
|------|---------|------------|------------|
| `InputField` | InputField | `path`, `title`, `tooltip`, `tooltipRepresentation`, `titleLocation`, `inputHint`, `width`, `height`, кнопки и флаги поля | ContextMenu, ExtendedTooltip |
| `HTMLDocumentField` | HTMLDocumentField | `path`, `title`, `tooltip`, `tooltipRepresentation`, `titleLocation`, `width`, `height`, `autoMaxWidth`, `autoMaxHeight`, `skipOnInput` | ContextMenu, ExtendedTooltip |
| `TextDocumentField` | TextDocumentField | `path`, `title`, `tooltip`, `tooltipRepresentation`, `titleLocation`, `width`, `height`, `autoMaxWidth`, `autoMaxHeight` | ContextMenu, ExtendedTooltip |
| `CheckBox` | CheckBoxField | `path`, `title`, `tooltip`, `tooltipRepresentation`, `titleLocation`, `checkBoxType`, `footerDataPath` | ContextMenu, ExtendedTooltip |
| `Label` | LabelDecoration | `title` (строка или `{text, formatted}`), `formatted`, `hyperlink`, `width`, `height`, `autoMaxWidth`, `autoMaxHeight`, `tooltipRepresentation` | ContextMenu, ExtendedTooltip |
| `LabelField` | LabelField | `path`, `tooltip`, `tooltipRepresentation`, `footerDataPath` | ContextMenu, ExtendedTooltip |
| `Group` | UsualGroup | `title`, `orientation`, `behavior`, `collapsed`, `representation`, `titleDataPath`, `currentRowUse`, `showTitle`, `showLeftMargin` | ExtendedTooltip |
| `Table` | Table | `path`, `representation`, `autoInsertNewRow`, `commandBarLocation`, `initialTreeView`, `enableDrag`, `rowPictureDataPath`, `tableAutofill`, `_dynList` и ключи динамического списка | ContextMenu, AutoCommandBar, Search*, ViewStatus* |
| `Pages` | Pages | `title`, `pagesRepresentation`, `currentRowUse` | ExtendedTooltip |
| `Page` | Page | `title`, `orientation`, `showTitle`, `titleDataPath` | ExtendedTooltip |
| `Button` | Button | `command`, `commandName` или `stdCommand`, `title`, `tooltip`, `tooltipRepresentation`, `representation`, `defaultButton`, `picture`, `locationInCommandBar`, `backColor`, `font` | ExtendedTooltip |
| `CommandBar` | CommandBar | `commandSource`, `autofill` | — |

Перечисления (`titleLocation`, `orientation`, `representation` и другие)
принимают значения платформы с заглавной буквы: `None`, `Horizontal`,
`GroupBox`. Полный список ключей и значений — в контракте
`<адрес>.Can.item.add`. Ключи динамического списка (`autoRefresh`,
`autoRefreshPeriod`, `choiceFoldersAndItems`, `restoreCurrentRow`,
`showRoot`, `allowRootChoice`, `updateOnDataChange`,
`allowGettingCurrentRowURL`) пишутся только с `_dynList: true`; кнопка
принимает одну команду. Флаги поля ввода `clearButton`, `spinButton`,
`dropListButton`, `markIncomplete`, `skipOnInput`, `horizontalStretch`,
`verticalStretch` и `autofill` командной панели принимают только `true`:
явный `false` платформа отличает от умолчания, а операция его пока
не пишет.

### Поле HTML-документа

HTML-поле привязывается через `path` к строковому реквизиту формы.
Публичный `items[]` задаёт `name` и `type: "HTMLDocumentField"`;
внутренний ключ `html` здесь отклоняется, в том числе вместе с `type`.
Сначала создайте его через `attribute.add`, если реквизита ещё нет.
Затем запросите план добавления поля и примените его отдельным вызовом
`unica.apply` только с `executionToken` из `data.executionToken` ответа:

```json
{
  "at": "main:Catalog.ТехническиеПроекты.Form.ФормаЭлемента",
  "ops": [{"op": "item.add", "args": {"items": [{
    "name": "ПолеОписания", "type": "HTMLDocumentField",
    "path": "СтраницаРедактораОписания", "titleLocation": "None",
    "width": 1, "height": 1, "autoMaxWidth": false, "skipOnInput": true
  }]}}]
}
```

Поддержаны `path`, `title`, `tooltip`, `titleLocation`, `width`/`height`
(целые `0..4294967295`), `autoMaxWidth`/`autoMaxHeight`, `skipOnInput`,
`visible`, `userVisible`, `enabled`, `readOnly` и синонимы `hidden`/`disabled`.
Булевы параметры принимают JSON boolean. События — только `OnClick` и
`DocumentComplete`: привяжите их `event.bind` по адресу события
`…Form.ФормаЭлемента.Item.ПолеОписания.Event.DocumentComplete` с явным
именем обработчика, в том же вызове после `item.add` или позже.
DOM-свойство `Document` относится к выполнению 1С и не записывается в XML.

Чтобы поместить HTML-поле в группу, вызовите `item.add` на адресе группы;
`after` задаёт соседний элемент. Полученную форму
прочитайте через `unica.view` и проверьте через `unica.check`.

### Поле текстового документа

Поле текстового документа задаётся в `item.add` через `name`,
`type: "TextDocumentField"` и `path`. Путь должен вести к реквизиту формы
типа `d5p1:TextDocument`; при необходимости сначала создайте реквизит через
`attribute.add`. Внутренний ключ `textDoc` публичная операция отклоняет.

Поддержаны общие свойства размещения и оформления из контракта
`<адрес формы>.Can.item.add`; `skipOnInput` и `editMode` не поддержаны.
Событие `OnChange` привязывается отдельной операцией `event.bind` по адресу
поля. Для размещения в группе вызывайте `item.add` на адресе группы.
Результат применяйте отдельным вызовом с `executionToken`, затем прочитайте
поле через `unica.view` и проверьте форму через `unica.check`.

### Чего словарь не пишет

Остальные виды платформенных полей и декораций **поверхность не создаёт**:
поля картинки, табличного и форматированного документа,
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

### Доступность команд формы

Если пользователь просит заблокировать, отключить или сделать недоступной
команду формы, управляй UI-элементами, связанными с командой, а не объектом
команды. Не генерируй и не советуй `Команды[ИмяКоманды].Доступность`: у команды
формы не используется свойство `Доступность` для управления кнопками и пунктами
меню.

Перед написанием BSL-кода прочитай узел формы через `unica.view` по
квалифицированному адресу `at` (CTR.SOURCE.LOGICAL-NODE-VIEW-SHAPE). Например,
аргументы вызовов чтения формы и её коллекции элементов:

```json
{"name": "unica.view", "arguments": {"at": "main:Catalog.Номенклатура.Form.ФормаЭлемента"}}
```

```json
{"name": "unica.view", "arguments": {"at": "main:Catalog.Номенклатура.Form.ФормаЭлемента.Item"}}
```

Ответ узла содержит `props` и `branches`, страница коллекции — `items` с
адресами `at`. Обойди все вложенные ветви `Item` по адресам из `branches` и
все страницы по возвращённому `cursor`, сохраняя адрес и параметры чтения.
У элемента нынешний view публикует в `props` только `tag`, `title`, `visible`,
`enabled`, `readOnly` (`title` равен `null`, если отдельный заголовок не задан).

Для установления связи требуется `CommandName`, например
`Form.Command.<ИмяКоманды>`, либо соответствующая ссылка стандартной команды.
Однако нынешний view не публикует ни `CommandName`, ни `binding`: по одному
дереву элементов нельзя доказать связь с командой и полноту связанных элементов.
Не угадывай связь по имени или заголовку. Для недостающих данных используй
аварийный мост к исходникам, разрешённый для чтения файла вне Unica:

```json
{"name": "unica.resolve", "arguments": {"at": "main:Catalog.Номенклатура.Form.ФормаЭлемента"}}
```

Возвращённый `path` относится к корню выбранного набора исходников, а не
обязательно к корню рабочего пространства. Сверь корень и формат набора
с конфигурацией проекта. В выгрузке Designer это может быть дескриптор
`Catalogs/Номенклатура/Forms/ФормаЭлемента.xml`: дерево элементов находится
рядом, в `ФормаЭлемента/Ext/Form.xml`. Проверь существование файла и его
корень `Form` в пространстве имён `http://v8.1c.ru/8.3/xcf/logform`;
дескриптор `MetaDataObject` не содержит полного дерева формы. Не переноси
эту раскладку на EDT и другие форматы без подтверждения.

Прочитай содержимое формы через `Read` и найди все элементы с точным
`CommandName` нужной команды, учитывая пространство имён XML. Обойди весь XML, включая `AutoCommandBar`,
`ContextMenu`, командные панели таблиц и вложенные группы, а не только
`ChildItems` корня: проекция `Item` не показывает все эти контейнеры.
Зафиксируй имена элементов и их привязки. Это только чтение; изменения
исходников по-прежнему выполняются через `unica.apply` с preview и `executionToken`.

Проверь также автозаполнение командных панелей и создание элементов в модуле
формы: статический XML не доказывает состав динамического интерфейса.
Если файл недоступен, формат не подтверждён или полнота связей остаётся
неизвестной, назови конкретное ограничение и найденных кандидатов; не генерируй
BSL с неподтверждёнными именами. Пробел чтения MCP обозначай как
**Unica MCP contract gap**, а динамический состав формы — как требующий
проверки кода или открытой формы. Отсутствие `binding` в `view` само по себе
не завершает исследование.

Когда связи подтверждены, меняй все связанные элементы через
`Элементы[ИмяЭлемента].Доступность`: кнопку основной командной панели, кнопку
командной панели таблицы, пункт контекстного меню, кнопку группы или подменю.
Одна найденная кнопка не доказывает, что других элементов этой команды нет.

```bsl
Элементы[ИмяЭлемента].Доступность = Ложь;
```

Например, если в проверенной форме кнопки `ЗаполнитьВПанели` и
`ЗаполнитьВМеню` обе связаны с `Form.Command.Заполнить`, отключи обе:

```bsl
Для Каждого ИмяЭлемента Из СтрРазделить("ЗаполнитьВПанели,ЗаполнитьВМеню", ",") Цикл
    Элементы[ИмяЭлемента].Доступность = Ложь;
КонецЦикла;
```

Если связанный элемент нельзя определить однозначно, сначала проанализируй форму
и назови найденные кандидаты. Для стандартных команд табличной части отдельно
проверь свойство таблицы `ТолькоПросмотр` (`readOnly` в `props` узла таблицы):
когда таблица уже только для просмотра, платформа обычно сама блокирует стандартные команды добавления,
копирования, удаления и перемещения строк. Ручная блокировка нужна прежде всего
для пользовательских элементов, запускающих изменение табличной части: подбор,
заполнение, загрузку, пересчёт, удаление строк, сортировку, изменение цен,
скидок, НДС и аналогичные действия.

### Допустимые события элемента

`event.bind` до записи проверяет событие по единой платформенной матрице. Недопустимое сочетание возвращает `ok=false` и код `FORM_EVENT_*`, не меняя файл. Основные сочетания:

- **input**: `OnChange`, `StartChoice`, `ChoiceProcessing`, `Clearing`, `AutoComplete`, `TextEditEnd`, `Opening`, `Creating`, `EditTextChange`
- **check**: `OnChange`
- **table**: `OnStartEdit`, `OnEditEnd`, `OnChange`, `Selection`, `BeforeAddRow`, `BeforeDeleteRow`, `OnActivateRow`
- **label**: `Click`, `URLProcessing`
- **picture**: `Click`, `Drag`, `DragCheck`
- **pages**: `OnCurrentPageChange`
- **page/button/group/command bar**: события не поддерживаются

События `table` требуют непустой привязки: прямого `DataPath` у `Table`. Платформа удаляет обработчики событий у несвязанной таблицы при загрузке/выгрузке конфигурации.

`OnReadAtServer`, `BeforeWrite`, `BeforeWriteAtServer`, `OnWriteAtServer`, `AfterWriteAtServer` и `AfterWrite` разрешены только при подтверждённом persistent object/record типе главного реквизита. Для `DataProcessorObject`, `ReportObject`, `DynamicList` и неизвестного контекста они отклоняются. `NewWriteProcessing` и `FillCheckProcessingAtServer` являются общими событиями формы и этим ограничением не связаны.

### Система типов (для attributes)

`string`, `string(100)`, `decimal(15,2)`, `boolean`, `date`, `dateTime`, `CatalogRef.XXX`, `DocumentObject.XXX`, `ValueTable`, `DynamicList`, `Type1 | Type2` (составной).

### События форм расширения

У формы расширения `event.bind` принимает `callType` (`Before`, `After`,
`Override`) на адресе события формы или элемента. У обычной формы
`callType` не задаётся. Событие нового элемента привязывает `event.bind`
на его адресе: `{"values": {"handler": "Расш1_ПолеПриИзмененииПосле",
"callType": "After"}}`. `command.add` `callType` и `actions`
не принимает: перехват действия команды формы расширения —
`event.implement` с `callType` на адресе
`….Form.<Форма>.Command.<Команда>.Event.Execute`.

Повтор идентичного binding является явным idempotent no-op. Конфликт обработчика/`callType`, duplicate, отсутствующий элемент и любой недопустимый event отклоняют весь batch до мутации; планирование использует тот же planner.

## Workflow

1. `unica.view` на узле формы и в ветвях `Item` и `Event` — адреса узлов и их словари `can`
2. Собрать `ops` на адресах узлов по описанию выше
3. `unica.apply` с `at` и `ops` — план и `data.executionToken`
4. `unica.apply` только с `executionToken` — применение
5. `unica.check` на узле формы — проверка, затем `unica.view` — убедиться, что структура изменилась правильно
