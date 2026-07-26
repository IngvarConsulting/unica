# JSON DSL для схемы компоновки данных (СКД)

Компактный JSON-формат для описания `DataCompositionSchema` (Template.xml).
Компилируется через MCP `unica.dcs.compile` в XML, валидируется через MCP `unica.dcs.validate`.

## Граница `dcs.compile` и `dcs.edit`

`unica.dcs.compile` создаёт новый XML из JSON-определения. `unica.dcs.edit`
изменяет существующий XML отдельными операциями и принимает собственные
operation-specific payloads. Совпадение имён полей или внутренних helper-форм
у двух инструментов не означает, что полный JSON из этого документа можно
передать в оба инструмента.

В текущем `dcs.compile` есть известные **семантические пробелы маппинга**:

| Область JSON | Текущий контракт `dcs.compile` |
|--------------|--------------------------------|
| Поле набора данных | `role`, `orderExpression`/`order`, `appearance` и `availableValues` не сериализуются. В shorthand также не сериализуются `@role` и `#restrict`; ограничения поддержаны только в object-форме. `availableValues` поддержан для параметров, но не для полей. |
| Параметр | `inputParameters` и `nilValue` не сериализуются; массив в `value` не превращается в повторяемые `<value>`, поэтому для compile-контракта допустимо только скалярное значение. |
| Настройки варианта | `userFields` и `additionalProperties` не сериализуются. `order` на уровне настроек и группировки — отдельная поддержанная ветка и не относится к пробелу `orderExpression` поля. |
| Макеты | Корневые `templates` и связанные привязки макетов не сериализуются. |

Эти ключи могут быть синтаксически приняты JSON-парсером и затем потеряны без
ошибки. XSD-валидация и платформенный импорт проверяют только уже созданный XML,
поэтому не доказывают сохранение пропущенного намерения JSON. До отдельного
устранения каждого пробела эти ветки не входят в заявленное покрытие
`dcs.compile`. `dcs.edit` имеет отдельное покрытие только для явно заявленных
операций (в частности, роли и ограничения поля), а не для всей полной модели.

---

## 1. Корневая структура

```json
{
  "dataSources": [...],
  "dataSets": [...],
  "dataSetLinks": [...],
  "calculatedFields": [...],
  "totalFields": [...],
  "parameters": [...],
  "templates": [...],
  "groupTemplates": [...],
  "settingsVariants": [...]
}
```

В этой обзорной форме `templates`/`groupTemplates` показаны только как части
целевой модели; текущий `dcs.compile` их не переносит в XML.

**Умолчания:**
- `dataSources` опущен → авто-создаётся `{ "name": "ИсточникДанных1", "type": "Local" }`
- `source` в наборе опущен → первый dataSource
- `name` набора опущен → "НаборДанных1", "НаборДанных2"...
- `settingsVariants` опущен → один вариант "Основной" с детальной группировкой и `selection: ["Auto"]`

---

## 2. Источники данных (dataSources)

```json
"dataSources": [
  { "name": "ИсточникДанных1", "type": "Local" }
]
```

| Поле | Обязат. | Умолчание | XML-маппинг |
|------|---------|-----------|-------------|
| `name` | да | — | `<name>` |
| `type` | нет | `"Local"` | `<dataSourceType>` |

Значения `type`: `"Local"`, `"External"`.

---

## 3. Наборы данных (dataSets)

Тип определяется по ключу-дискриминатору:

| Ключ | Тип | xsi:type |
|------|-----|----------|
| `query` | Запрос | `DataSetQuery` |
| `objectName` | Объект | `DataSetObject` |
| `items` | Объединение | `DataSetUnion` |

### DataSetQuery (самый частый)

```json
{ "name": "Продажи", "query": "ВЫБРАТЬ ...", "fields": [...], "autoFillFields": false }
```

### DataSetObject

```json
{ "name": "ТаблицаПроверки", "objectName": "ТаблицаПроверки", "fields": [...] }
```

### DataSetUnion

```json
{
  "name": "Объединение",
  "items": [
    { "name": "Набор1", "query": "...", "fields": [...] },
    { "name": "Набор2", "query": "...", "fields": [...] }
  ],
  "fields": [...]
}
```

| Поле | Обязат. | Описание |
|------|---------|----------|
| `name` | нет | Авто: "НаборДанных1"... |
| `source` | нет | Имя dataSource (авто: первый) |
| `query` | да* | Текст запроса (DataSetQuery). Поддерживает `@file` — см. ниже |
| `objectName` | да* | Имя объекта (DataSetObject) |
| `items` | да* | Вложенные наборы (DataSetUnion) |
| `fields` | нет | Массив полей |
| `autoFillFields` | нет | `false` — отключить автозаполнение (по умолчанию не выводится = true) |

### Ссылка на внешний файл запроса (@file)

Вместо inline-текста запроса можно указать путь к внешнему файлу с префиксом `@`:

```json
{ "query": "@queries/sales.sql" }
```

Порядок разрешения пути:
1. Абсолютный путь — используется как есть
2. Относительно директории JSON-файла определения
3. Относительно текущей рабочей директории (CWD)
4. Если файл не найден — ошибка компиляции

---

## 4. Поля — shorthand и объектная форма

### Shorthand-строка

Для `dcs.compile` shorthand сейчас надёжно задаёт только `dataPath` и `type`.
Маркеры `@role` и `#restrict` удаляются при разборе, но в XML не попадают;
ограничения для compile следует задавать object-формой, а роль поля пока не
маппится вообще. Полная shorthand-грамматика с ролями и ограничениями
используется operation-specific полями `dcs.edit`.

```
"<dataPath>[: <type>] [@role...] [#restrict...]"
```

Примеры:

```json
"fields": [
  "Наименование",
  "Количество: decimal(15,2)",
  "Организация: CatalogRef.Организации @dimension",
  "Служебное: string #noFilter #noOrder",
  "Счёт: CatalogRef.Хозрасчетный @account",
  "Сумма: decimal(15,2) @balance",
  "СуммаНач: decimal(15,2) @balance"
]
```

### Объектная форма

В `dcs.compile` из этой формы маппятся `dataPath`, `field`, `title`, `type`,
`restrict`, `attrRestrict` и `presentationExpression`. Показанные ниже
`role`, `orderExpression`, `appearance` и `availableValues` описывают полную
модель, но текущий компилятор их молча не переносит в XML.

```json
{
  "dataPath": "Сумма",
  "field": "Сумма",
  "title": "Сумма продаж",
  "type": "decimal(15,2)",
  "role": { "dimension": true },
  "restrict": ["noFilter", "noGroup"],
  "attrRestrict": ["noFilter"],
  "appearance": { "Формат": "ЧДЦ=2" },
  "presentationExpression": "Формат(Сумма, \"ЧДЦ=2\")",
  "orderExpression": { "expression": "ЕстьNULL(Поле.Порядок, 10000)", "orderType": "Asc", "autoOrder": false },
  // или массив (если на поле несколько <orderExpression> для multi-sort fallback):
  // "orderExpression": [{...}, {...}]
  "availableValues": [
    { "value": 1, "presentation": { "ru": "Доход", "en": "Income" } },
    { "value": 2, "presentation": { "ru": "Расход", "en": "Expense" } }
  ]
}
```

`availableValues` — целевая форма списка допустимых значений поля с
(опциональной multilang) подписью. Для полей она пока не маппится
`dcs.compile`; поддержанная одноимённая ветка параметров описана в разделе 6.

### Парсинг shorthand по инструментам

1. `dcs.compile` оставляет `dataPath[: type]`, но пока не переносит `@`/`#`.
2. `dcs.edit` в операциях add/modify-field принимает `@`-флаги роли и
   `#`-ограничения вместе с `dataPath[: type]`.
3. Только отдельная операция `set-field-role` принимает после `dataPath`
   одновременно `@`-флаги и пары `key=value`.

### Типы

| DSL | XML v8:Type | Квалификатор |
|-----|-------------|--------------|
| `string` | `xs:string` | Length=0, AllowedLength=Variable |
| `string(N)` | `xs:string` | Length=N, AllowedLength=Variable |
| `string(N,fixed)` | `xs:string` | Length=N, AllowedLength=Fixed |
| `decimal(D,F)` | `xs:decimal` | Digits=D, FractionDigits=F, AllowedSign=Any |
| `decimal(D,F,nonneg)` | `xs:decimal` | Digits=D, FractionDigits=F, AllowedSign=Nonnegative |
| `boolean` | `xs:boolean` | — |
| `date` | `xs:dateTime` | DateFractions=Date |
| `dateTime` | `xs:dateTime` | DateFractions=DateTime |
| `CatalogRef.XXX` | `d5p1:CatalogRef.XXX` | inline xmlns:d5p1 |
| `DocumentRef.XXX` | `d5p1:DocumentRef.XXX` | inline xmlns:d5p1 |
| `EnumRef.XXX` | `d5p1:EnumRef.XXX` | inline xmlns:d5p1 |
| `ChartOfAccountsRef.XXX` | `d5p1:ChartOfAccountsRef.XXX` | inline xmlns:d5p1 |
| `StandardPeriod` | `v8:StandardPeriod` | — |
| `DocumentRef` (без `.XXX`) | `<v8:TypeSet xmlns:d5p1=...>d5p1:DocumentRef</v8:TypeSet>` | композитный тип-набор (все ссылки указанного класса) |
| `typeid:<GUID>` | `<v8:TypeId>` | служебный roundtrip-маркер существующего типа |

Для фиксированного профиля 8.3.27 длина строки — целое `0..1024`; `fixed`
требует длину больше нуля. Точность числа — целое `0..38`, дробная часть —
`0..digits`, третий параметр допускает только точный `nonneg`. Составной тип
разделяется `|`; пустые части и повтор одного платформенного типа отклоняются
до записи. XML всегда группируется по XSD: `Type` → `TypeSet` → `TypeId` →
Number/String/Date qualifiers. Порядок значений внутри повторяемой группы
семантический и при платформенном раундтрипе проверяется как мультимножество.

> **Ссылочные типы** (`CatalogRef.XXX`, `DocumentRef.XXX` и др.) эмитируются с inline namespace declaration: `<v8:Type xmlns:d5p1="http://v8.1c.ru/8.1/data/enterprise/current-config">d5p1:CatalogRef.XXX</v8:Type>`. Использование префикса `cfg:` вместо `d5p1:` с объявлением namespace приводит к ошибке XDTO. Сборка EPF со ссылочными типами требует базу с соответствующей конфигурацией (не пустую).

> **TypeSet (тип-набор)** — голое имя без точки (`CatalogRef`, `DocumentRef`, `EnumRef`, `ChartOfAccountsRef`, `ChartOfCharacteristicTypesRef`, `ChartOfCalculationTypesRef`, `BusinessProcessRef`, `TaskRef`, `ExchangePlanRef`, `InformationRegisterRef`, `AnyRef`) — указывает на **все** ссылки этого класса конфигурации (а не на конкретный объект). Эмитится как `<v8:TypeSet>` вместо `<v8:Type>`. Используется в параметрах типа «исключаемые документы» и подобных.

> **Определяемые типы** (`DefinedType.XXX`) в фиксированном DCS-профиле
> 8.3.27 не принимаются. Runtime XSD допускает такой QName в `v8:TypeSet`, но
> платформа 8.3.27.2074 при импорте и повторной выгрузке удаляет весь
> `valueType` с `DefinedType.*` у параметров, полей наборов и вычисляемых полей.
> `dcs.compile` и типизированные операции `dcs.edit` поэтому отклоняют такое
> значение до записи. Укажите раскрытый состав определяемого типа явно,
> например `string(16)` или составной тип через `|`.

### Синонимы типов

Все имена типов регистронезависимые. Поддерживаются русские и альтернативные имена:

| Синоним | Канонический тип |
|---------|-----------------|
| `число`, `Число` | `decimal` |
| `строка`, `Строка` | `string` |
| `булево`, `Булево`, `bool` | `boolean` |
| `дата`, `Дата` | `date` |
| `датаВремя`, `ДатаВремя` | `dateTime` |
| `СтандартныйПериод` | `StandardPeriod` |
| `int`, `integer`, `number`, `num` | `decimal` |
| `СправочникСсылка.XXX` | `CatalogRef.XXX` |
| `ДокументСсылка.XXX` | `DocumentRef.XXX` |
| `ПеречислениеСсылка.XXX` | `EnumRef.XXX` |
| `ПланСчетовСсылка.XXX` | `ChartOfAccountsRef.XXX` |
| `ПланВидовХарактеристикСсылка.XXX` | `ChartOfCharacteristicTypesRef.XXX` |

Параметризованные: `число(15,2)` → `decimal(15,2)`, `строка(100)` → `string(100)`.

### Роли (`dcs.edit`; не `dcs.compile`)

Ключ `role` объектного поля в полном определении `dcs.compile` сейчас
игнорируется. В текущем публичном контракте полная роль задаётся отдельной
операцией `dcs.edit` `set-field-role`; её `value` имеет форму:

```
Сумма @balance balanceGroupName=Сумма balanceType=OpeningBalance
```

Операции add/modify-field принимают внутри shorthand только `@`-флаги. JSON
формы `role` как строка, массив или объект не являются payload-контрактом
`dcs.edit` и не маппятся `dcs.compile`.

**Поддерживаемые ключи**:

| Категория | Ключи |
|-----------|-------|
| `@`-флаги (boolean) | `@dimension`, `@account`, `@balance`, `@required`, `@ignoreNullValues`, `@dimensionAttribute`; `@period` — sugar |
| Строковые KV | `parentDimension`, `accountTypeExpression`, `balanceGroupName`, `accountField` |
| Типизированные KV | `periodNumber` (integer), `periodType` (`Main`/`Specify`/`Additional`), `balanceType` (`None`/`OpeningBalance`/`ClosingBalance`), `accountingBalanceType` (`None`/`Debit`/`Credit`) |

Для фиксированного профиля 8.3.27 действует строгий whitelist
`DataSetFieldRole`; неизвестный ключ отклоняется до записи. `@period` — sugar
для `periodNumber=1` + `periodType=Main` (можно переопределить явно).
`autoOrder`, `expression` и `orderType` относятся к `orderExpression`, а не к
роли поля.

**XML-выход `dcs.edit`**: `<dcscom:KEY>true</dcscom:KEY>` для флагов;
`<dcscom:KEY>VALUE</dcscom:KEY>` для KV. Поддерживается только точное имя
`balanceGroupName`; alias `balanceGroup` не существует.

### Ограничения

| DSL shorthand | Объектная форма | XML useRestriction |
|---------------|----------------|-----|
| `#noField` | `"noField"` | `<field>true</field>` |
| `#noFilter` / `#noCondition` | `"noFilter"` | `<condition>true</condition>` |
| `#noGroup` | `"noGroup"` | `<group>true</group>` |
| `#noOrder` | `"noOrder"` | `<order>true</order>` |

### Оформление поля (appearance; пока не `dcs.compile`)

Ниже показана XML-модель оформления поля. Ключ `appearance` поля набора данных
текущий `dcs.compile` не сериализует. Это не относится к поддержанной ветке
`settings.conditionalAppearance`.

```json
"appearance": {
  "Формат": "ЧДЦ=2",
  "ГоризонтальноеПоложение": "Center"
}
```

Маппинг на XML:
```xml
<appearance>
  <dcscor:item xsi:type="dcsset:SettingsParameterValue">
    <dcscor:parameter>Формат</dcscor:parameter>
    <dcscor:value xsi:type="xs:string">ЧДЦ=2</dcscor:value>
  </dcscor:item>
</appearance>
```

Значения `ГоризонтальноеПоложение` → `xsi:type="v8ui:HorizontalAlign"`.

---

## 5. Итоговые поля (totalFields)

### Shorthand

```
"<dataPath>: <Функция>"
"<dataPath>: <Функция>(<выражение>)"
```

Примеры:

```json
"totalFields": [
  "Количество: Сумма",
  "Цена: Максимум",
  "Стоимость: Сумма(Кол * Цена)"
]
```

**Парсинг:** `"A: Func"` → `dataPath=A`, `expression=Func(A)`. `"A: Func(expr)"` → `dataPath=A`, `expression=Func(expr)`.

Функции (русские): `Сумма`, `Количество`, `Максимум`, `Минимум`, `Среднее`.

### Объектная форма

```json
{ "dataPath": "X", "expression": "Максимум(X)", "group": "Группа1" }
```

### Привязка к группировкам (group)

В объектной форме поле `group` может быть строкой или массивом строк. Каждая строка задаёт имя группировки, для которой вычисляется итог:

```json
"totalFields": [
  { "dataPath": "Кол", "expression": "Сумма(Кол)", "group": ["ГруппаПользователей", "ГруппаПользователей Иерархия", "ОбщийИтог"] }
]
```

XML-маппинг — по `<group>` на каждый элемент:
```xml
<totalField>
  <dataPath>Кол</dataPath>
  <expression>Сумма(Кол)</expression>
  <group>ГруппаПользователей</group>
  <group>ГруппаПользователей Иерархия</group>
  <group>ОбщийИтог</group>
</totalField>
```

---

## 6. Параметры (parameters)

### Shorthand

```
"<name>: <type> [= <default>] [@autoDates] [@valueList] [@hidden]"
```

Примеры:

```json
"parameters": [
  "Период: StandardPeriod = LastMonth @autoDates",
  "Организация: CatalogRef.Организации",
  "ДатаОтчета: date"
]
```

**Парсинг:** `"A: T = V"` → `name=A`, `type=T`, `value=V`. Значение
`LastMonth` и другие именованные варианты периодов → `v8:StandardPeriod` с
`v8:variant`, но без `v8:startDate`/`v8:endDate`. Эти даты сериализуются только
для варианта `Custom`: платформа 8.3.27.2074 меняет именованный вариант с
явными датами на `Custom` при раундтрипе.

`<default>` в shorthand может быть **списком** — несколько значений через
запятую (с `'...'` для запятой внутри значения). В этом случае эмитятся
несколько `<value>`, а `valueListAllowed=true` выводится автоматически (явный
`@valueList` не нужен). Объектная форма `"value": [ ... ]` этому shorthand
**не эквивалентна и не поддержана**: в `dcs.compile` поле `value` объекта
принимает только скаляр.

```json
"parameters": ["Виды: ChartOfCharacteristicTypesRef.ВидыСубконтоХозрасчетные = ПланВидовХарактеристик.ВидыСубконтоХозрасчетные.Контрагенты, ПланВидовХарактеристик.ВидыСубконтоХозрасчетные.Договоры"]
```

### @autoDates

Флаг `@autoDates` в shorthand параметра автоматически генерирует два дополнительных параметра:
- `ДатаНачала` (date, expression=`&<Имя>.ДатаНачала`, availableAsField=false)
- `ДатаОкончания` (date, expression=`&<Имя>.ДатаОкончания`, availableAsField=false)

Заменяет типовой бойлерплейт из 5 строк на 1:

```json
// Было:
"parameters": [
  "Период: StandardPeriod = LastMonth",
  { "name": "ДатаНачала", "type": "date", "expression": "&Период.ДатаНачала", "availableAsField": false },
  { "name": "ДатаОкончания", "type": "date", "expression": "&Период.ДатаОкончания", "availableAsField": false }
]

// Стало:
"parameters": ["Период: StandardPeriod = LastMonth @autoDates"]
```

### @valueList

Флаг `@valueList` генерирует `<valueListAllowed>true</valueListAllowed>` — разрешает передавать список значений в параметр:

```json
"parameters": ["Организации: CatalogRef.Организации @valueList"]
```

### @hidden

Флаг `@hidden` — скрытый параметр. Автоматически ставит `availableAsField=false` и исключает параметр из автогенерируемых `dataParameters` при `"dataParameters": "auto"`:

```json
"parameters": [
  { "name": "Счет43", "type": "ChartOfAccountsRef.Хозрасчетный", "value": "...", "hidden": true },
  "СкрытыйПараметр: string = test @hidden"
]
```

### Объектная форма

```json
{
  "name": "ДатаНач",
  "title": "Дата начала",
  "type": "date",
  "value": "0001-01-01T00:00:00",
  "expression": "&Период.ДатаНачала",
  "availableAsField": false,
  "useRestriction": true,
  "use": "Always"
}
```

| Поле | Описание |
|------|----------|
| `name` | Имя параметра |
| `title` | Заголовок (умолч. = name) |
| `type` | Тип (см. таблицу типов) |
| `value` | Значение по умолчанию. В текущем `dcs.compile` поддержан только скаляр; JSON-массив не разворачивается в повторяемые `<value>` и не входит в compile-контракт. |
| `expression` | Выражение для вычисления |
| `availableAsField` | `false` — скрыть из полей |
| `valueListAllowed` | `true` — разрешить список значений |
| `hidden` | `true` — скрытый параметр (авто `availableAsField=false`, исключение из `dataParameters: auto`) |
| `useRestriction` | `true` — скрыть от пользователя |
| `use` | `"Always"`, `"Auto"` |
| `denyIncompleteValues` | `true` — запретить произвольные значения (только из availableValues) |
| `availableValues` | Поддержанный для параметров массив `[{value, presentation}]`; `value` — строка, число или bool, `presentation` — строка. Одноимённый ключ поля набора данных не маппится. |
| `inputParameters` | Целевая/roundtrip-модель параметров ввода. Текущий `dcs.compile` этот ключ параметра не сериализует. |
| `nilValue` | Целевая форма явного `<value xsi:nil="true"/>`. На верхнеуровневом параметре текущий `dcs.compile` ключ игнорирует; поддержка `nilValue` в `settings.dataParameters` — отдельная ветка. |

### availableValues параметра (`dcs.compile`)

Список допустимых значений параметра. Тип значения определяется автоматически (`Перечисление.*`, `Справочник.*` и др. → `dcscor:DesignTimeValue`):

```json
{
  "name": "ПорядокОкругления",
  "type": "EnumRef.Округления",
  "value": "Перечисление.Округления.Окр1_00",
  "use": "Always",
  "denyIncompleteValues": true,
  "availableValues": [
    {"value": "Перечисление.Округления.Окр1_00", "presentation": "руб. коп"},
    {"value": "Перечисление.Округления.Окр1", "presentation": "руб."},
    {"value": "Перечисление.Округления.Окр1000", "presentation": "тыс. руб"}
  ]
}
```

### Значения параметров по типу

| Тип | value | XML |
|-----|-------|-----|
| `StandardPeriod` | `"LastMonth"`, `"ThisYear"` и др. | `<v8:variant xsi:type="v8:StandardPeriodVariant">LastMonth</v8:variant>` |
| `date` | `"0001-01-01T00:00:00"` | `xsi:type="xs:dateTime"` |
| `string` | `"текст"` | `xsi:type="xs:string"` |
| `boolean` | `true`/`false` | `xsi:type="xs:boolean"` |

Стандартные варианты периодов: `Custom`, `Today`, `ThisWeek`, `ThisMonth`, `ThisQuarter`, `ThisYear`, `LastMonth`, `LastQuarter`, `LastYear`.

---

## 7. Вычисляемые поля (calculatedFields)

### Shorthand

```
"<dataPath> = <expression>"
```

```json
"calculatedFields": [
  "УИД = Строка(Ссылка.УникальныйИдентификатор())",
  "Итого = Количество * Цена"
]
```

### Объектная форма

```json
{
  "dataPath": "Итого",
  "expression": "Количество * Цена",
  "title": "Итого",
  "type": "decimal(15,2)",
  "restrict": ["noGroup"],
  "appearance": { "Формат": "ЧДЦ=2" }
}
```

Ключ `field` — алиас для `dataPath` (используется если `dataPath` не указан).
`dcs.compile` маппит здесь `dataPath`/`field`, `expression`, `title`, `type` и
`restrict`; `appearance` вычисляемого поля пока игнорируется и не входит в
compile-покрытие.

---

## 8. Связи наборов (dataSetLinks)

Только объектная форма:

```json
"dataSetLinks": [
  {
    "source": "Периоды",
    "dest": "Данные",
    "sourceExpr": "Месяц",
    "destExpr": "Месяц",
    "parameter": "НачалоМесяца"
  }
]
```

| Поле | XML |
|------|-----|
| `source` / `sourceDataSet` | `<sourceDataSet>` |
| `dest` / `destinationDataSet` | `<destinationDataSet>` |
| `sourceExpr` / `sourceExpression` | `<sourceExpression>` |
| `destExpr` / `destinationExpression` | `<destinationExpression>` |
| `parameter` | `<parameter>` (опц.) |
| `parameterListAllowed` | `<parameterListAllowed>true</parameterListAllowed>` (опц., bool) |
| `startExpression` | `<startExpression>` (опц.) |
| `linkConditionExpression` | `<linkConditionExpression>` (опц.) |
| `required` | `true` — платформенное умолчание, XML-элемент опускается; `false` → `<required>false</required>` |

decompile эмитит длинные имена (`sourceDataSet` и т.д.); compile принимает обе формы.

---

## 9. Варианты настроек (settingsVariants)

```json
"settingsVariants": [{
  "name": "Основной",
  "presentation": "Основной вариант",
  "settings": {
    "userFields": [...],
    "selection": [...],
    "filter": [...],
    "order": [...],
    "conditionalAppearance": [...],
    "outputParameters": {...},
    "dataParameters": [...],
    "structure": [...],
    "additionalProperties": { "ВариантНаименование": "...", "Адрес": "..." }
  }
}]
```

`additionalProperties` — платформенная модель словаря служебных свойств
варианта (`<dcsset:additionalProperties>`). Текущий `dcs.compile` этот ключ не
сериализует; обещания bit-perfect round-trip для него нет.

### selection

```json
"selection": [
  "Наименование",
  { "field": "Количество", "title": "Кол-во" },
  { "field": "Контрагент", "viewMode": "Inaccessible" },
  { "field": "Скрытое", "use": false },
  { "auto": true, "use": false },
  "Auto"
]
```

- Строка → `SelectedItemField`
- `"Auto"` → `SelectedItemAuto` (только на уровне группировок; на верхнем уровне settings игнорируется)
- Объект с `field` + опц. `title`/`viewMode`/`use` → `SelectedItemField`. `use: false` = поле выборки отключено (видно в UI, но не применяется)
- Объект `{ auto: true, use: false }` → отключённый `SelectedItemAuto`
- Объект с `folder`/`items` → `SelectedItemFolder` — группа полей с заголовком и `placement=Auto`:

```json
"selection": [
  "Auto",
  "Счет",
  {"folder": "Поступление", "items": ["ПолеА", "ПолеБ", "ПолеВ"]},
  {"folder": "Выбытие", "items": ["ВыбытиеРеализовано", "ВыбытиеПрочее"]}
]
```

Опциональное поле `placement` (`Auto` / `Horizontally` / `Vertically` / `Special`) задаёт расположение элементов внутри группы (по умолчанию `Auto`):

```json
{"folder": "Экономия ФОТ", "items": ["ЭкономияФОТ", "ЭкономияФОТПроцент"], "placement": "Horizontally"}
```

### filter

#### Shorthand-строка

```json
"filter": [
  "Организация = _ @off @user",
  "Дата >= 2024-01-01T00:00:00",
  "Статус filled",
  "Количество > 0"
]
```

Формат: `"<Поле> <оператор> [<значение>] [@off] [@user] [@quickAccess] [@normal] [@inaccessible]"`.

- Значение `_` — пустое (placeholder, не выводится в XML)
- `@off` → `use=false`
- `@user` → `userSettingID=auto` (генерировать GUID)
- `@quickAccess` → `viewMode=QuickAccess`
- `@normal` → `viewMode=Normal` (явный — для bit-perfect, см. [viewMode](#viewmode-режим-доступности))
- `@inaccessible` → `viewMode=Inaccessible`
- Типы значений автоопределяются: `true`/`false` → `xs:boolean`, дата `2024-01-01T00:00:00` → `xs:dateTime`, числа → `xs:decimal`, `Перечисление.X.Y`/`Справочник.X.Y`/`ПланСчетов.X.Y` и др. → `dcscor:DesignTimeValue`, остальное → `xs:string`
- Типы значений автоопределяются: `true`/`false` → boolean, `2024-01-01T00:00:00` → dateTime, числа → decimal, `Перечисление.*`/`Справочник.*`/`ПланСчетов.*`/`Документ.*` → DesignTimeValue, прочее → string
- OrGroup: `{"group": "Or", "items": ["условие1", "условие2"]}` — объединяет условия через ИЛИ

#### Объектная форма

```json
"filter": [
  { "field": "Организация", "op": "=", "use": false, "userSettingID": "auto" },
  { "field": "Дата", "op": ">=", "value": "0001-01-01T00:00:00", "valueType": "xs:dateTime" },
  { "field": "СуммаДт", "op": "=", "value": "СуммаКт", "valueType": "dcscor:Field" },
  { "field": "Статус", "op": "in", "value": [1, 3, 5] },
  { "field": "Контрагенты", "op": "in", "value": [], "userSettingID": "auto" },
  { "group": "Or", "items": [
    { "field": "Статус", "op": "=", "value": true, "valueType": "xs:boolean" },
    { "field": "Пометка", "op": "filled" }
  ], "userSettingID": "auto" }
]
```

| Поле | Описание |
|------|----------|
| `field` | Имя поля |
| `op` | Оператор (см. таблицу) |
| `value` | Правая часть (опц.). См. формы ниже |
| `valueType` | xsi:type для значения (опц.). `"dcscor:Field"` = field-to-field comparison (значение — имя другого поля). Для массива `value: [...]` применяется ко всем элементам — нужен когда auto-detect ошибается (например `Перечисление.X.Y` должно остаться `xs:string`, а не `dcscor:DesignTimeValue`) |
| `use` | Включён (`true` по умолчанию) |
| `presentation` | Текст подсказки |
| `viewMode` | `"Normal"`, `"QuickAccess"`, `"Inaccessible"` |
| `userSettingID` | `"auto"` → генерировать GUID |
| `userSettingPresentation` | Отображаемое имя настройки (LocalStringType) |

**Формы `value`:**
- Скаляр (`"X"`, `5`, `true`, `"2024-01-01T00:00:00"`) — single `<right>` (стандартный случай). Тип определяется автоматически: bool / число / дата / строка.
- Массив `[a, b, c]` — несколько `<right>` подряд (для `in`/`notIn` с конкретными значениями).
- Пустой массив `[]` — `<right xsi:type="v8:ValueListType">` placeholder (типичный паттерн для `in` с пользовательскими настройками — значения заполнит пользователь через UI).

Операторы:

| DSL | XML comparisonType |
|-----|--------------------|
| `=` | `Equal` |
| `<>` | `NotEqual` |
| `>` | `Greater` |
| `>=` | `GreaterOrEqual` |
| `<` | `Less` |
| `<=` | `LessOrEqual` |
| `in` | `InList` |
| `notIn` | `NotInList` |
| `inHierarchy` | `InHierarchy` |
| `contains` | `Contains` |
| `notContains` | `NotContains` |
| `beginsWith` | `BeginsWith` |
| `filled` | `Filled` |
| `notFilled` | `NotFilled` |

Группа условий: `{ "group": "And"|"Or"|"Not", "items": [...] }` → `FilterItemGroup` с `groupType`. Группа также принимает item-level поля `presentation`, `viewMode`, `userSettingID`, `userSettingPresentation` — для регистрации группы как пункта пользовательских настроек.

### order

```json
"order": [
  "Количество desc",
  "Наименование",
  { "field": "Контрагент", "direction": "desc", "viewMode": "Inaccessible" },
  "Auto"
]
```

- `"Field"` → `OrderItemField`, `orderType=Asc`
- `"Field desc"` → `OrderItemField`, `orderType=Desc`
- `"Field asc"` → `OrderItemField`, `orderType=Asc`
- `"Auto"` → `OrderItemAuto` (только на уровне группировок; на верхнем уровне settings игнорируется)
- Объект `{ field, direction?, viewMode?, use? }` — нужен, когда требуется задать `viewMode`, или отключить сортировку через `use: false` (см. [viewMode](#viewmode-режим-доступности))

### conditionalAppearance

Условное оформление — массив правил, каждое задаёт набор полей (selection), условия (filter), параметры оформления (appearance) и мета-атрибуты.

```json
"conditionalAppearance": [
  {
    "selection": ["Сумма"],
    "filter": ["Сумма > 1000"],
    "appearance": { "ЦветТекста": "style:ПросроченныеДанныеЦвет" },
    "presentation": { "ru": "Выделять крупные суммы", "en": "Highlight large amounts" },
    "viewMode": "Normal",
    "userSettingID": "auto"
  },
  {
    "filter": ["Статус notFilled"],
    "appearance": { "Текст": "Не указано", "ЦветТекста": "web:Gray" },
    "presentation": "Скрывать пустые статусы",
    "use": false,
    "useInDontUse": ["group", "fieldsHeader"]
  }
]
```

| Поле | Описание |
|------|----------|
| `selection` | Массив полей, к которым применяется. Пусто/опущено = все поля |
| `filter` | Условия (shorthand-строки или объекты, как в settings filter) |
| `appearance` | Объект `{ "Параметр": "Значение" }` |
| `presentation` | Описание правила (строка или multilang dict `{ru, en}`) |
| `use` | Включено (`true` по умолчанию). `false` = правило отключено |
| `viewMode` | `"Normal"`, `"QuickAccess"`, `"Inaccessible"` |
| `userSettingID` | `"auto"` → генерировать GUID |
| `userSettingPresentation` | Имя в пользовательских настройках (string или multilang) |
| `useInDontUse` | Массив контекстов где правило **НЕ** применяется. Возможные имена: `group`, `hierarchicalGroup`, `overall`, `fieldsHeader`, `header`, `parameters`, `filter`, `resourceFieldsHeader`, `overallHeader`, `overallResourceFieldsHeader` |

**Типы значений appearance** определяются автоматически:
- `style:XXX` → `v8ui:Color` (палитра темы платформы, namespace `http://v8.1c.ru/8.1/data/ui/style`)
- `web:XXX` → `v8ui:Color` (web-имена цветов, namespace `http://v8.1c.ru/8.1/data/ui/colors/web`)
- `win:XXX` → `v8ui:Color` (системные цвета Windows, namespace `http://v8.1c.ru/8.1/data/ui/colors/windows`)
- Ключи `ЦветТекста`/`ЦветФона`/`ЦветГраницы` со значениями типа `auto` или `#XXXXXX` → `v8ui:Color`
- Ключ `Размещение` → `dcscor:DataCompositionTextPlacementType`
- Ключи `ГоризонтальноеПоложение`/`ВертикальноеПоложение` → `v8ui:HorizontalAlign`/`VerticalAlign`
- Ключ `ТипМакета` → `dcsset:DataCompositionGroupTemplateType`
- Ключи `Текст`/`Заголовок`/`Формат` → `v8:LocalStringType` (если значение строка)
- Числовые строки (`"40"`, `"15"`) → `xs:decimal`
- `true`/`false` → `xs:boolean`
- Multilang dict `{ru, en}` для любого ключа → `v8:LocalStringType`
- Прочее → `xs:string`

Поддержка `use=false` на уровне параметра:
```json
"appearance": {
  "ЦветФона": { "value": "web:LightGray", "use": false }
}
```

### outputParameters

```json
"outputParameters": {
  "Заголовок": "Мой отчёт",
  "ВыводитьЗаголовок": "Output",
  "МакетОформления": "ОформлениеОтчетовЧерноБелый"
}
```

Ключ → `dcscor:parameter`, значение → `dcscor:value`.

Типы значений определяются автоматически:
- `"Заголовок"` → `v8:LocalStringType` (примет строку или multilang dict)
- `"ВыводитьЗаголовок"`, `"ВыводитьПараметрыДанных"`, `"ВыводитьОтбор"` → `dcsset:DataCompositionTextOutputType`
- `"РасположениеПолейГруппировки"` → `dcsset:DataCompositionGroupFieldsPlacement`
- `"РасположениеРеквизитов"` → `dcsset:DataCompositionAttributesPlacement`
- `"ГоризонтальноеРасположениеОбщихИтогов"`, `"ВертикальноеРасположениеОбщихИтогов"`, `"РасположениеОбщихИтогов"`, `"РасположениеИтогов"` → `dcscor:DataCompositionTotalPlacement`
- `"РасположениеГруппировки"` → `dcsset:DataCompositionFieldGroupPlacement`
- `"РасположениеРесурсов"` → `dcsset:DataCompositionResourcesPlacement`
- `"ТипМакета"` → `dcsset:DataCompositionGroupTemplateType`
- Multilang dict `{ru, en}` для любого ключа → `v8:LocalStringType`
- Прочие → `xs:string`

Значение можно обернуть в `{ "value": ..., "use": false }` — отключённый параметр (платформа эмитит `<dcscor:use>false</dcscor:use>`). Такая же форма доступна в `appearance` items (см. раздел conditionalAppearance).

#### Полная wrapper-форма (bit-perfect round-trip)

Decompile сохраняет всю периферию каждого outputParameter в wrapper'е:

```json
{
  "value": "Custom",
  "valueType": "v8:StandardPeriod",        // полный xsi:type если не покрыт type-map'ом
  "use": false,                            // <dcscor:use>false</dcscor:use>
  "items": {                               // nested sub-параметры (ТипДиаграммы.ВидПодписей)
    "ТипДиаграммы.ВидПодписей": { "value": "Value", "valueType": "v8ui:ChartLabelType" }
  },
  "viewMode": "Normal",                    // <dcsset:viewMode>Normal</dcsset:viewMode>
  "userSettingID": "auto",
  "userSettingPresentation": { "ru": "Тип" }
}
```

Wrapper эмитится только при наличии extra-полей; простое `"key": "value"` остаётся как есть.

#### Шрифт (v8ui:Font) в appearance

Шрифт — объект с маркером `@type: "Font"`:
```json
"Шрифт": { "@type": "Font", "ref": "sys:DefaultGUIFont", "height": 10, "bold": "true", "italic": "false", "underline": "false", "strikeout": "false", "kind": "WindowsFont" }
```
Все атрибуты исходного XML сохраняются — для bit-perfect.

#### Граница (v8ui:Line) в appearance

Граница — объект с маркером `@type: "Line"` (атрибуты `width`/`gap` и inner `<v8ui:style>` сериализуются inline):
```json
"СтильГраницы": { "@type": "Line", "width": 0, "gap": false, "style": "None" }
```

Стороны (`СтильГраницы.Сверху/.Снизу/.Слева/.Справа`) — nested SettingsParameterValue, кладутся в `items` (как у outputParameters wrapper):
```json
"СтильГраницы": {
  "@type": "Line", "width": 0, "gap": false, "style": "None",
  "items": {
    "СтильГраницы.Сверху": {
      "value": { "@type": "Line", "width": 1, "gap": false, "style": "Solid" },
      "use": false
    },
    "СтильГраницы.Снизу": {
      "value": { "@type": "Line", "width": 1, "gap": false, "style": "Double" }
    }
  }
}
```

Top-level Line хранится **плоско** (`@type`/`width`/`gap`/`style` + `use?`/`items?` на одном уровне). Nested items используют универсальный wrapper `{ value, use? }` — у `value` тип любой (Line/Font/color/text). Значения `style`: `None`, `Solid`, `Double`, `LargeDashed`, `SmallDashed`, `Dotted` и т.п. (значения `v8ui:SpreadsheetDocumentCellLineType`).

### dataParameters

#### Автогенерация

```json
"dataParameters": "auto"
```

Генерирует записи `dataParameters` для всех не-hidden параметров с `userSettingID`. Скрытые параметры (`hidden: true` / `@hidden`) исключаются.

#### Shorthand-строка

```json
"dataParameters": [
  "Период = LastMonth @user",
  "Организация @off @user"
]
```

Формат: `"<Имя> [= <значение>] [@off] [@user] [@quickAccess] [@normal] [@inaccessible]"`.

- Значения-варианты периодов (`LastMonth`, `ThisYear` и др.) автоматически оборачиваются в `v8:StandardPeriod`
- `@off` → `use=false`, `@user` → `userSettingID=auto`

#### Объектная форма

```json
"dataParameters": [
  { "parameter": "Период", "value": { "variant": "LastMonth" }, "userSettingID": "auto" },
  { "parameter": "Организация", "use": false, "viewMode": "Normal", "userSettingID": "auto", "userSettingPresentation": "Организация отчёта" }
]
```

| Поле | Описание |
|------|----------|
| `parameter` | Имя параметра |
| `value` | Значение (объект `{ "variant": "LastMonth" }` для StandardPeriod, или скаляр) |
| `valueType` | Полный xsi:type если кастомный (например `dcsset:DataCompositionGroupUseVariant`). Для пустого значения с `use: false` — `"xs:string"` эмитит `<value xsi:type="xs:string"/>` (placeholder отключённого параметра типа DateTime, бит-перфектный аналог `xsi:nil`) |
| `use` | Включён (`true` по умолчанию) |
| `viewMode` | `"Normal"`, `"QuickAccess"`, `"Inaccessible"` |
| `userSettingID` | `"auto"` → генерировать GUID |
| `userSettingPresentation` | Отображаемое имя настройки (LocalStringType) |

#### StandardPeriod / StandardBeginningDate — shape inference

Compile различает варианты по форме `value`:

| Форма | xsi:type | Когда |
|---|---|---|
| `{variant, startDate, endDate}` | `v8:StandardPeriod` | Custom с явными датами |
| `{variant: "ThisMonth"}` (etc) | `v8:StandardPeriod` | без дат — non-Custom варианты SP |
| `{variant, date}` | `v8:StandardBeginningDate` | Custom с явной датой |
| `{variant: "BeginningOf*"}` | `v8:StandardBeginningDate` | без даты — variant'ы начинаются с `BeginningOf` |
| `"2024-01-15T10:00:00"` (string) | `xs:dateTime` | raw datetime без обёртки |

Platform-pattern: `startDate`/`endDate`/`date` эмитятся ТОЛЬКО для `variant=Custom`. Для `ThisMonth`/`LastYear`/`BeginningOfThisDay`/... — только `<v8:variant>`.

### structure

#### String shorthand (рекомендуется для типичных случаев)

```json
"structure": "Организация > details"
"structure": "Организация > Номенклатура > details"
"structure": "Период > Организация > Номенклатура > details"
```

`>` разделяет уровни вложенности. Каждый сегмент — группировка по указанному полю. `details` (или `детали`) — детальные записи (пустой `groupBy`). Для каждого уровня `selection` и `order` автоматически `["Auto"]`.

#### Массив объектов

```json
"structure": [
  {
    "type": "group",
    "groupBy": ["Организация"],
    "children": [
      { "type": "group" }
    ]
  }
]
```

**Умолчания:** `selection` и `order` по умолчанию `["Auto"]` на каждом уровне (в группировках, строках/колонках таблиц, точках/сериях диаграмм). Указывать явно нужно только если требуется другой набор полей.

#### Группировка (group)

| Поле | Описание |
|------|----------|
| `type` | `"group"` |
| `name` | Имя группировки (опц.) |
| `groupBy` | Массив полей. Каждый элемент — строка (имя поля) или объект `{ field, groupType?, periodAdditionType?, periodAdditionBegin?, periodAdditionEnd? }`. Пусто/опущено = детальные записи. Object-форма нужна когда `groupType ≠ "Items"`, `periodAdditionType ≠ "None"` или задана `periodAdditionBegin/End` (см. ниже) |
| `groupType` | `"Items"` (умолч.), `"Hierarchy"`, `"HierarchyOnly"` |
| `selection` | Выборка (умолч. `["Auto"]`) |
| `filter` | Отборы (как в settings) |
| `order` | Сортировка (умолч. `["Auto"]`) |
| `outputParameters` | Параметры вывода (как в settings) |
| `conditionalAppearance` | Условное оформление группы (как в settings) |
| `use` | `false` = ветка структуры отключена (на самой группе) |
| `viewMode` | `"Normal"`, `"QuickAccess"`, `"Inaccessible"` — режим доступности группы в пользовательских настройках |
| `itemsViewMode` | `"Normal"`, `"QuickAccess"`, `"Inaccessible"` — режим доступности подэлементов группы |
| `userSettingID` | `"auto"` → генерировать GUID. Регистрирует группу как пункт пользовательских настроек |
| `userSettingPresentation` | Имя в пользовательских настройках (string или multilang dict) |
| `children` | Вложенные элементы структуры |

Пустой `groupBy` (или `[]`) = детальные записи (без `groupItems` в XML).

**`periodAdditionBegin` / `periodAdditionEnd`** на field-объекте — даты добавочного периода (`<dcsset:periodAdditionBegin>`/`<dcsset:periodAdditionEnd>`). Compile auto-определяет xsi:type значения: строка вида `2025-01-01T00:00:00` → `xs:dateTime`, иначе (путь к параметру, например `ПараметрыДанных.ДатаНачала`) → `dcscor:Field`.

```json
{ "field": "ПериодМесяц",
  "periodAdditionBegin": "ПараметрыДанных.ДатаНачала",
  "periodAdditionEnd":   "ПараметрыДанных.ДатаОкончания" }
```

#### Таблица (table)

```json
{
  "type": "table",
  "name": "Таблица",
  "rows": [
    { "groupBy": ["Номенклатура"], "selection": ["Auto"], "order": ["Auto"] }
  ],
  "columns": [
    {
      "name": "Период",
      "groupBy": ["Период"],
      "filter": ["Сумма > 0"],
      "selection": ["Auto"],
      "order": ["Auto"],
      "outputParameters": { "РасположениеИтогов": "None" },
      "userSettingID": "auto",
      "userSettingPresentation": { "ru": "Колонка с периодом" }
    }
  ]
}
```

Каждая `column`/`row` принимает те же поля что и `group`: `name`, `groupBy`/`groupFields`, `filter`, `order`, `selection`, `outputParameters`, `conditionalAppearance`, `children` (вложенные `StructureItemGroup`), плюс user-settings — `viewMode`, `userSettingID`, `userSettingPresentation`, `itemsViewMode` (регистрация column/row как пункта «Изменить вариант»).

На самой `table` (отдельно от column/row) также допустимы `selection`, `conditionalAppearance`, `outputParameters`, плюс user-settings: `viewMode`, `userSettingID`, `userSettingPresentation`, `itemsViewMode`, `columnsViewMode`, `rowsViewMode`, `use` (`false` = таблица отключена).
- `columnsViewMode` / `rowsViewMode` — режим доступности секции колонок / строк в пользовательских настройках (значения: `Normal` / `QuickAccess` / `Inaccessible`).

> **Внутренний паттерн**: `<dcsset:item xsi:type="dcsset:StructureItemGroup">` внутри `<dcsset:row>`/`<dcsset:column>`/`<dcsset:points>`/`<dcsset:series>` платформа всегда сериализует в **короткой форме** `<dcsset:item>` без `xsi:type`. Compile эмитит этот вариант автоматически для `children` table axis.

#### Диаграмма (chart)

```json
{
  "type": "chart",
  "points": { "groupBy": ["Организация"], "order": ["Auto"], "filter": [...] },
  "series": { "groupBy": ["Месяц"], "order": ["Auto"] },
  "selection": ["Сумма"]
}
```

`points` и `series` принимают те же поля что table column/row (включая `name` и user-settings).

На самой chart-item: `viewMode`, `userSettingID`, `userSettingPresentation`, `itemsViewMode`, `pointsViewMode`, `seriesViewMode`, `use: false` (диаграмма отключена). `pointsViewMode`/`seriesViewMode` — аналоги `columnsViewMode`/`rowsViewMode` у таблицы.

**Multi-series / multi-points** — `points` и `series` могут быть массивом объектов, тогда генерируется несколько `<dcsset:point>` или `<dcsset:series>` подряд (каждый со своими `groupBy`, `filter`, user-settings). Используется например для разделения данных диаграммы на несколько серий по разным фильтрам:

```json
{
  "type": "chart",
  "points": { "groupBy": ["Период"] },
  "series": [
    { "groupBy": ["Стадия"], "filter": ["Стадия = ЗНАЧЕНИЕ(Перечисление.X.A)"],
      "viewMode": "Normal", "userSettingID": "auto",
      "userSettingPresentation": { "ru": "Серия A" } },
    { "groupBy": ["Стадия"], "filter": ["Стадия = ЗНАЧЕНИЕ(Перечисление.X.B)"],
      "viewMode": "Normal", "userSettingID": "auto",
      "userSettingPresentation": { "ru": "Серия B" } }
  ]
}
```

### userFields (справочная модель; не `dcs.compile`)

Текущий `dcs.compile` игнорирует `settings.userFields`. Раздел ниже фиксирует
форму платформенного XML/целевой JSON, но не описывает поддержанную ветку
компилятора и не входит в XSD/платформенное compile-покрытие.

Дополнительные поля, которые пользователь может задать в режиме «Изменить вариант» через UI. Хранятся в settings варианта. Два подтипа определяются по содержимому объекта:

**Expression-форма** — поле вычисляется выражением (опционально с разделением для детальных строк и для итогов):

```json
"userFields": [
  {
    "dataPath": "ПользовательскиеПоля.Поле1",
    "title": { "ru": "Отработано дней", "en": "Days worked" },
    "detail": {
      "expression": "Выбор Когда Группа = ... Тогда ОтработаноДней Иначе 0 Конец",
      "presentation": "Выбор Когда Группа = ... Тогда [Отработано дней] Иначе 0 Конец"
    },
    "total": {
      "expression": "Сумма(Выбор Когда Группа = ... Тогда ОтработаноДней Иначе 0 Конец)",
      "presentation": "Сумма(Выбор Когда Группа = ... Тогда [Отработано дней] Иначе 0 Конец)"
    }
  }
]
```

| Поле | Описание |
|------|----------|
| `dataPath` | Путь поля в формате `ПользовательскиеПоля.ПолеN` |
| `title` | Заголовок (строка или multilang dict) |
| `detail.expression` | Выражение для детальных записей |
| `detail.presentation` | Тот же expression с подстановкой `[Имя поля]` (для UI) |
| `total.expression` | Выражение для итоговой строки |
| `total.presentation` | Same для UI |

> **Платформенная форма пустых значений** содержит все четыре элемента
> (`detailExpression`, `detailExpressionPresentation`, `totalExpression`,
> `totalExpressionPresentation`), включая пустой
> `<dcsset:totalExpression/>`. Это наблюдение не является гарантией текущего
> `dcs.compile`, который блок `userFields` не эмитит.

**Case-форма** — поле принимает разные значения в зависимости от условий:

```json
"userFields": [
  {
    "dataPath": "ПользовательскиеПоля.Поле1",
    "title": { "ru": "Вид продаж" },
    "cases": [
      {
        "filter": ["ХозОперация <> Перечисление.ХозяйственныеОперации.РеализацияВРозницу"],
        "value": 2,
        "presentation": { "ru": "Только оптовые продажи", "en": "Wholesale only" }
      },
      {
        "filter": ["ХозОперация = Перечисление.ХозяйственныеОперации.РеализацияВРозницу"],
        "value": 3,
        "presentation": { "ru": "Только розничные продажи", "en": "Retail only" }
      }
    ]
  }
]
```

| Поле | Описание |
|------|----------|
| `cases[].filter` | Условие (как в settings filter) |
| `cases[].value` | Значение поля если условие выполнено (типы автоопределяются: bool/decimal/string) |
| `cases[].presentation` | Текст значения для UI (multilang) |

Тип элемента определяется автоматически: наличие `cases` → `UserFieldCase`, иначе → `UserFieldExpression`.

### viewMode (режим доступности)

`viewMode` управляет доступностью элемента в **пользовательских настройках** отчёта («Изменить вариант…» / «Настройки»). Возможные значения:

| Значение | Семантика |
|----------|-----------|
| `"Normal"` | Пользователь видит и может править (default) |
| `"Inaccessible"` | Скрыто от пользователя, не редактируется |
| `"QuickAccess"` | Вынесено в быстрые настройки (на форму отчёта) |
| `"Auto"` | Автоматический режим (наследование от контейнера) |

Применяется в трёх контекстах:

**1. Item-level** — на отдельном элементе блока (см. описание объектной формы соответствующего раздела):

```json
"filter":    [{ "field": "X", "op": "=", "value": "Y", "viewMode": "Inaccessible" }],
"selection": [{ "field": "X", "viewMode": "Inaccessible" }],
"order":     [{ "field": "X", "viewMode": "Inaccessible" }],
"conditionalAppearance": [{ "filter": [...], "appearance": {...}, "viewMode": "Inaccessible" }],
"dataParameters": [{ "parameter": "X", "viewMode": "QuickAccess" }]
```

Shorthand-флаги `@inaccessible`, `@quickAccess` доступны для `filter` и `dataParameters` строковых форм.

**2. Block-level** — на самом блоке (внутри `settings`). Управляет доступностью всей группы как пункта пользовательских настроек:

```json
"settings": {
  "selectionViewMode":              "Inaccessible",
  "filterViewMode":                 "Inaccessible",
  "orderViewMode":                  "Inaccessible",
  "conditionalAppearanceViewMode":  "Inaccessible",
  "itemsViewMode":                  "Inaccessible",
  "selectionUserSettingID":              "auto",
  "filterUserSettingID":                 "auto",
  "orderUserSettingID":                  "auto",
  "conditionalAppearanceUserSettingID":  "auto",
  "selection": [...],
  "filter":    [...]
}
```

`itemsViewMode` на settings — общий режим для всех подэлементов варианта (`<dcsset:itemsViewMode>` в XML). `XxxUserSettingID` парят с `XxxViewMode` — platform пишет их в block-level пользовательских настроек. Пустые блоки (без items) тоже эмитятся, если есть block-level meta — например `<dcsset:conditionalAppearance><dcsset:viewMode>Normal</dcsset:viewMode></dcsset:conditionalAppearance>`.

Также `orderViewMode`/`orderUserSettingID` поддержаны на StructureItemGroup для случаев когда block-level meta лежит на nested `<dcsset:order>`.

**3. Structure item** — на элементе структуры (`group`):

```json
{ "type": "group", "groupBy": ["Организация"], "viewMode": "Inaccessible", "itemsViewMode": "Inaccessible" }
```

**4. Table axis / chart axis** — на самой `column`/`row`/`points`/`series`. Через те же поля `viewMode`, `userSettingID`, `userSettingPresentation` (см. раздел Таблица).

#### Стратегия сохранения

Платформа эмитит `viewMode` непоследовательно: в одних местах `<viewMode>Normal</viewMode>` присутствует явно (когда элемент — пункт пользовательских настроек), в других — нет. Для bit-perfect round-trip:

- `dcs-decompile` сохраняет `viewMode` в JSON **точно как было в XML**, включая явный `"Normal"` если он физически присутствовал.
- `dcs-compile` эмитит `<viewMode>` только если значение задано в JSON (без `implicit Normal`-подстановки).

При компиляции JSON, написанного с нуля моделью, `viewMode` опускается → платформа применит default `Normal` при загрузке схемы.

---

## 10. Макеты и привязки (справочная модель; не `dcs.compile`)

Корневые `templates`, `fieldTemplates`, `groupTemplates` и связанные варианты
привязок текущий `dcs.compile` не сериализует. Весь раздел ниже описывает
целевую/legacy JSON-модель и платформенный XML, но не поддержанную ветку
публичного компилятора и не доказанное платформенным gate покрытие.

### templates — компактная целевая модель

Табличное описание шаблона вывода. Содержимое задаётся через `rows`, оформление — через именованный пресет `style`.

```json
"templates": [
  {
    "name": "Макет1",
    "style": "header",
    "widths": [36, 33, 16, 17],
    "minHeight": 24.75,
    "rows": [
      ["Виды кассы", "Валюта", "Остаток на начало\nпериода", "Остаток на\nконец\nпериода"],
      ["|", "|", "|", "|"],
      ["К1", "К2", "К3", "К4"]
    ]
  },
  {
    "name": "Макет2",
    "style": "data",
    "widths": [36, 33, 16, 17],
    "rows": [["{ВидКассы}", "{Валюта}", "{ОстатокНачало}", "{ОстатокКонец}"]],
    "parameters": [
      { "name": "ВидКассы", "expression": "Представление(СчетМеждународногоУчета)" },
      { "name": "ОстатокНачало", "expression": "ОстатокНаНачалоПериода" }
    ]
  },
  {
    "name": "Макет3",
    "style": "total",
    "widths": [36, 33, 16, 17],
    "rows": [["Итого", "Х", "{ОстатокНачало}", "{ОстатокКонец}"]],
    "parameters": [
      { "name": "ОстатокНачало", "expression": "ОстатокНаНачалоПериода" }
    ]
  }
]
```

#### Свойства шаблона

| Свойство | Описание |
|----------|----------|
| `name` | Имя макета (ссылаются groupTemplate) |
| `rows` | Массив строк; каждая строка — массив ячеек |
| `style` | Именованный пресет оформления (по умолчанию `"data"`) |
| `widths` | Массив ширин колонок (применяется ко всем строкам) |
| `minHeight` | Минимальная высота первой строки (для шапок) |
| `parameters` | Параметры макета — выражения для подстановки (поддерживают `drilldown`) |

#### Синтаксис ячеек

| Значение | Описание |
|----------|----------|
| `"текст"` | Статический текст (`v8:LocalStringType`) |
| `"{Имя}"` | Параметр шаблона (`dcscor:Parameter`), задаётся через `parameters` |
| `"\|"` | Вертикальное объединение с ячейкой выше (`ОбъединятьПоВертикали`) |
| `">"` | Горизонтальное объединение с ячейкой слева (`ОбъединятьПоГоризонтали`) |
| `null` | Пустая ячейка (без содержимого) |

#### Встроенные пресеты стилей

| Пресет | Фон | Шрифт | Выравнивание | Перенос | Рамки |
|--------|-----|-------|-------------|---------|-------|
| `header` | ReportHeaderBackColor | Arial 10 | Center | да | Solid 1px |
| `data` | ReportGroup1BackColor | Arial 10 | — | нет | Solid 1px |
| `subheader` | — | Arial 10 | Center | да | Solid 1px |
| `total` | — | Arial 10 | — | нет | Solid 1px |

#### Пользовательские пресеты (dcs-styles.json)

Файл `dcs-styles.json` в директории определения или в корне проекта. Переопределяет встроенные пресеты или добавляет новые:

```json
{
  "header": {
    "bgColor": "style:ReportHeaderBackColor",
    "borderColor": "style:ReportLineColor",
    "bold": true
  },
  "myStyle": {
    "font": "Arial", "fontSize": 12,
    "bgColor": "#FFE0E0"
  }
}
```

Допустимые ключи: `font`, `fontSize`, `bold`, `italic`, `hAlign`, `vAlign`, `wrap`, `bgColor`, `textColor`, `borderColor`, `borders`. Недостающие ключи берутся из пресета `data`.

Формат цветов: `"style:ИмяСтиля"` (ссылка на стиль платформы) или `"#RRGGBB"` (прямой цвет).

### templates — raw XML (целевая fallback-модель)

Для нестандартных случаев — raw XML вставляется как есть:

```json
"templates": [
  {
    "name": "Макет1",
    "template": "<raw XML dcsat:AreaTemplate>",
    "parameters": [
      { "name": "ТипЦены", "expression": "Представление(ТипЦен)" }
    ]
  }
]
```

В целевой/legacy модели наличие `rows` выбирает компактную форму, иначе
используется raw XML из `template`. Текущий `dcs.compile` этот детект не
выполняет, потому что весь корневой блок `templates` игнорируется.

#### Расшифровка (drilldown) в параметрах шаблона

Ключ `drilldown` в параметре шаблона — три формы по типу значения:

**Форма A (без drilldown)** — обычный `ExpressionAreaTemplateParameter`:
```json
{ "name": "Дата", "expression": "Документ.Дата" }
```

**Форма B (строка, shortcut)** — `ExpressionAreaTemplateParameter` + автоматический `DetailsAreaTemplateParameter` с именем `Расшифровка_<value>`, `fieldExpression` по полю `ИмяРесурса` (`expression="<value>"`), `mainAction=DrillDown`. Ячейки `{name}` получают appearance `Расшифровка → Расшифровка_<value>` автоматически:
```json
{ "name": "Сырье", "expression": "ПоступлениеСырья", "drilldown": "ПоступлениеСырья" }
```

**Форма C (объект)** — самостоятельный `DetailsAreaTemplateParameter` с именем `name`, без `ExpressionAreaTemplateParameter`. Используется когда расшифровка ссылается на data-параметр (а не на ИмяРесурса) и/или нужен другой `mainAction` (например `OpenValue`):
```json
{ "name": "МаршрутныйЛист",
  "drilldown": { "field": "МаршрутныйЛист",
                 "expression": "МаршрутныйЛист",
                 "action": "OpenValue" } }
```
`action` по умолчанию `DrillDown`.

**Override на уровне ячейки** — object-форма `{ value, drilldown }`. Используется когда несколько ячеек должны указывать на один и тот же параметр-расшифровку (объявленный формой C):

```json
"rows": [
  [ { "value": "{Номер}", "drilldown": "МаршрутныйЛист" },
    { "value": "{Дата}",  "drilldown": "МаршрутныйЛист" } ]
]
```

Значение `drilldown` в ячейке — это полное имя параметра-расшифровки (как объявлено в `parameters`). Для shortcut form B override не нужен — appearance подставляется автоматически.

### fieldTemplates

Привязка именованного area-template к полю — `<fieldTemplate><field>X</field><template>Y</template></fieldTemplate>`. Когда платформа выводит значение поля `X`, используется макет `Y`:

```json
"fieldTemplates": [
  { "field": "МаршрутныйЛист", "template": "Макет1" }
]
```

### groupTemplates

```json
"groupTemplates": [
  { "groupName": "ДанныеОтчета", "templateType": "GroupHeader", "template": "Макет1" },
  { "groupField": "Счет", "templateType": "Header", "template": "Макет2" },
  { "groupField": "Счет", "templateType": "OverallHeader", "template": "Макет3" }
]
```

| Ключ | Описание |
|------|----------|
| `groupField` | Привязка к полю группировки → `<groupField>` |
| `groupName` | Привязка к именованной группировке в структуре варианта → `<groupName>` |
| `templateType` | `Header` / `OverallHeader` → `<groupTemplate>`, `GroupHeader` → `<groupHeaderTemplate>` |
| `template` | Имя макета |

---

## 11. Полный пример — минимальный

```json
{
  "dataSets": [
    {
      "name": "НаборДанных1",
      "query": "ВЫБРАТЬ\n\tНоменклатура.Наименование КАК Наименование,\n\tКОЛИЧЕСТВО(1) КАК Количество\nИЗ\n\tСправочник.Номенклатура КАК Номенклатура\nСГРУППИРОВАТЬ ПО\n\tНоменклатура.Наименование",
      "fields": [
        { "dataPath": "Наименование", "title": "Наименование" },
        "Количество"
      ]
    }
  ],
  "totalFields": ["Количество: Сумма"],
  "settingsVariants": [{
    "name": "Основной",
    "settings": {
      "selection": ["Наименование", "Количество"],
      "structure": [{ "type": "group" }]
    }
  }]
}
```

## 12. Полный пример — средний (с shorthand v2)

```json
{
  "dataSets": [
    {
      "name": "Продажи",
      "query": "ВЫБРАТЬ\n\tПродажи.Организация,\n\tПродажи.Номенклатура,\n\tПродажи.Количество,\n\tПродажи.Сумма\nИЗ\n\tРегистрНакопления.Продажи КАК Продажи\n{ГДЕ\n\tПродажи.Период >= &ДатаНачала\n\tИ Продажи.Период < &ДатаОкончания}",
      "fields": [
        "Организация: СправочникСсылка.Организации @dimension",
        "Номенклатура: СправочникСсылка.Номенклатура @dimension",
        "Количество: число(15,3)",
        "Сумма: число(15,2)"
      ]
    }
  ],
  "totalFields": ["Количество: Сумма", "Сумма: Сумма"],
  "parameters": [
    "Период: СтандартныйПериод = LastMonth @autoDates"
  ],
  "settingsVariants": [{
    "name": "Основной",
    "presentation": "Продажи по организациям",
    "settings": {
      "selection": ["Номенклатура", "Количество", "Сумма", "Auto"],
      "filter": ["Организация = _ @off @user"],
      "order": ["Сумма desc", "Auto"],
      "outputParameters": {
        "Заголовок": "Анализ продаж",
        "ВыводитьЗаголовок": "Output"
      },
      "dataParameters": ["Период = LastMonth @user"],
      "structure": "Организация > details"
    }
  }]
}
```

**Сравнение с v1:** средний пример сократился с 58 до 33 строк (−43%). Основная экономия: `@autoDates` (−4 строки), structure shorthand (−9 строк), filter/dataParam shorthand (−4 строки).
