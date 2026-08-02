# Доказанный контракт `Ext/Package.bin` пакета XDTO

> **Активный записываемый профиль:** платформа `8.3.27`, формат выгрузки
> `2.20` (ADR-0016). Этот документ задаёт поддерживаемое Unica подмножество
> `Package.bin`, а не является полной грамматикой XDTO.

## 1. Граница доказательств

Контракт сверен с XDTO-материалами donor `Nikolay-Shirokov/cc-1c-skills` на
commit `2067778ba3bad527bd1e5850304d1c82acb81fc8`. Следующий блок — закреплённый
inventory доказательств, а не маршруты выполнения и не ссылки внутри пакета:

<!-- xdto-donor-evidence:start -->
```text
docs/xdto-guide.md
docs/xdto-dsl-spec.md
.claude/skills/xdto-compile/SKILL.md
.claude/skills/xdto-decompile/SKILL.md
.claude/skills/xdto-edit/SKILL.md
.claude/skills/xdto-info/SKILL.md
.claude/skills/xdto-validate/SKILL.md
tests/skills/cases/xdto-compile/
tests/skills/cases/xdto-decompile/
tests/skills/cases/xdto-edit/
tests/skills/cases/xdto-info/
tests/skills/cases/xdto-validate/
```
<!-- xdto-donor-evidence:end -->

Локальная fixture
`tests/fixtures/xdto/enterprise-data-minimal/` закрепляет только ветви,
необходимые домену ADR-0024: импорты, глобальное свойство, локальные типы,
вложенный `ObjectType` и самозакрывающийся именованный `objectType`.

Donor и fixture доказывают описанные ниже формы, но не все элементы, атрибуты,
фасеты и комбинации платформенной модели XDTO. Неописанный узел нельзя молча
считать поддерживаемым: reader сообщает его как `unsupported_node`, а writer не
перестраивает и не нормализует его.

## 2. Файлы и владелец профиля

Логический объект `XDTOPackage.<Имя>` состоит из двух ресурсов:

- `XDTOPackages/<Имя>.xml` — дескриптор метаданных с `Name` и `Namespace`;
- `XDTOPackages/<Имя>/Ext/Package.bin` — текстовая XML-модель пакета.

Версией формата владеет корень `MetaDataObject` дескриптора. Перед записью
`Package.bin` writer разрешает дескриптор через выбранный `sourceSet` и требует
точный литерал `version="2.20"` по ADR-0016. `Namespace` дескриптора должен быть
равен `package/@targetNamespace`; расхождение является ошибкой цели, а не
основанием переписать один из URI.

Доказанная fixture `Package.bin` записана как UTF-8 с одним BOM, с наблюдёнными
`CRLF` и табами для отступов. Writer принимает и сохраняет ровно наблюдённое
наличие нуля или одного начального BOM; два и более начальных BOM отклоняются
как `unsupported_node` до построения плана. Расширение `.bin` не означает
двоичный формат.

## 3. Корень и порядок верхнего уровня

Корень имеет QName `{http://v8.1c.ru/8.1/xdto}package` и обязательный
`targetNamespace`. Для поддерживаемого подмножества прямые дети следуют
группами в порядке:

```text
import* → property* → valueType* → objectType*
```

Внутри каждой группы существующий порядок значим и сохраняется. Writer
вставляет новый узел в соответствующую группу, но не сортирует соседей и не
переставляет остальные группы.

## 4. Поддерживаемые контейнеры

Следующая таблица — машинно проверяемая граница контракта writer-а. Статус
`supported` означает, что конструкция закреплена fixture и разрешена контрактом
writer-а; `unsupported` означает, что donor такую форму знает, но этот контракт
writer-а её не обещает.

<!-- xdto-evidence-contract:start -->
| Status | Construct | Rule |
| --- | --- | --- |
| `supported` | `package/import` | `direct-child` |
| `supported` | `package/property` | `direct-child` |
| `supported` | `package/valueType` | `direct-child` |
| `supported` | `package/objectType` | `direct-child` |
| `supported` | `objectType/property` | `direct-child` |
| `supported` | `property/typeDef:ObjectType` | `direct-child` |
| `supported` | `typeDef:ObjectType/property` | `direct-child` |
| `supported` | `property-identity` | `exactly-one(name,ref)` |
| `supported` | `ref-target` | `global-property` |
| `supported` | `owned-type` | `zero-or-one(type,typeDef:ObjectType)` |
| `supported` | `lowerBound` | `0-or-1;default=1` |
| `supported` | `upperBound` | `-1-or-integer>=1;default=1` |
| `supported` | `finite-bounds` | `lower<=upper` |
| `unsupported` | `valueType/enumeration` | `writer-contract` |
| `unsupported` | `valueType/pattern` | `writer-contract` |
| `unsupported` | `valueType/typeDef:ValueType` | `writer-contract` |
| `unsupported` | `property/typeDef:ValueType` | `writer-contract` |
| `unsupported` | `valueType/memberTypes` | `writer-contract` |
<!-- xdto-evidence-contract:end -->

Именованные `valueType` и `objectType` являются локальными типами пакета.
`typeDef` в границе ADR-0024 поддерживается как анонимный
`xsi:type="ObjectType"` внутри `property`; его свойства образуют тот же
упорядоченный список, что свойства именованного `objectType`. В частности,
путь `ЛюбаяСсылка/СсылкаНаОбъект` разрешается до вложенного `typeDef`, тогда как
`СоставнойЛюбойОбъект` разрешается прямо до именованного `objectType`.

Иные отношения вложенности и атрибуты вне таблицы требуют нового evidence и
намеренного расширения таблицы; присутствие формы в donor само по себе не
расширяет контракт writer-а ADR-0024.

## 5. Имена, ссылки и уникальность

- `import/@namespace` объявляет зависимость по URI; `schemaLocation` у этой
  модели нет.
- Значения `type`, `base` и `ref` — QName. Каждый префикс должен быть объявлен
  в области видимости. `xs:` и `xsi:` обозначают стандартные пространства;
  ссылка на тип из `targetNamespace`, включая self-reference, также использует
  объявленный префикс, а не голое имя. `ref` отличается от `type`: он разрешает
  идентичность глобального `property`, а не тип значения.
- Публичные аргументы `base` и `property.type` принимают только лексическую
  форму `prefix:local` без окружающих пробелов. Writer сохраняет введённый
  лексический QName точно: он не подставляет и не заменяет префикс. Сначала
  используется объявление в области вставки. Если его нет, локальное
  `xmlns:prefix="URI"` на новом элементе допустимо только когда все объявления
  этого префикса во всём пакете доказывают один и тот же URI. При отсутствии
  объявления или конфликтующих URI writer ничего не угадывает, а последующая
  проверка отклоняет неразрешимый QName.
- QName во внешнем namespace разрешим только при наличии соответствующего
  `import`. Ссылки на локальный именованный тип разрешаются в текущем пакете.
- `valueType/@name` и `objectType/@name` уникальны совместно в пределах
  `targetNamespace`. Идентичности непосредственных свойств одного `objectType`
  или одного `typeDef` уникальны: для собственного объявления это `name`, для
  ссылки — разрешённый QName глобального свойства.

### `propertyPath`

Сегмент `propertyPath` после снятия экранирования является полным XML NCName;
точка внутри NCName поэтому не запрещается. В лексической строке пути
неэкранированная `.` разделяет сегменты, а `\.` означает буквальную точку в
идентичности свойства. Например, JSON-значение `"A\\.B"` адресует свойство
`A.B`, а `"A\\.B.Child"` — последовательность сегментов `A.B` и `Child`.

Других escape-последовательностей нет: обратная косая черта не перед точкой и
завершающая обратная косая черта недопустимы. Пустой сегмент, включая ведущую,
завершающую или двойную неэкранированную точку, также недопустим.

## 6. Свойства и effective bounds

Каждый `property` имеет ровно одну идентичность: собственное `name` либо QName
в `ref`, но не оба. `ref` обязан разрешаться в глобальное свойство пакета или
объявленного `import` и не сочетается с `type` или вложенным `typeDef`.

Собственное объявление с `name` имеет не более одного определения типа: QName
в `type` либо вложенный `typeDef xsi:type="ObjectType"`. Они взаимоисключающие;
отсутствие обоих означает произвольный тип. Остальные формы `typeDef` таблица
явно оставляет unsupported.

Для проверки и типизированного ответа используются effective bounds:

- отсутствующий `lowerBound` означает `1`; допустимые явные литералы — только
  `0` и `1`;
- отсутствующий `upperBound` означает `1`; допустимый конечный литерал — целое
  не меньше `1`;
- `upperBound="-1"` означает неограниченную верхнюю границу;
- для конечной верхней границы выполняется `lowerBound <= upperBound` после
  подстановки effective значений;
- явно записанные литералы сохраняются без канонизации.

## 7. Byte-local writer

Мутация выполняется byte-local: writer декодирует исходный BOM, распознаёт
перевод строки и отступ целевой области, затем вставляет или удаляет только
строки плана. Он сохраняет BOM, наблюдённый EOL, отступы, существующие
namespace-объявления, литералы умолчаний, самозакрывающуюся форму соседних
узлов и порядок нетронутых байтов. Полная сериализация XML-дерева для точечной
правки запрещена, поскольку она меняет доказанные байтовые свойства без
семантической причины.

Один запрос `unica.xdto.edit` содержит одну операцию и создаёт не более одной
атомарно публикуемой мутации. Сценарий, которому нужны несколько операций,
выполняется как упорядоченная неатомарная последовательность запросов; каждый
из них имеет собственные preview и неизменённый apply.

## 8. Fixture

`tests/fixtures/xdto/enterprise-data-minimal/Configuration.xml` и дескриптор
`XDTOPackages/EnterpriseData_1_17_3.xml` связывают логическую цель
`XDTOPackage.EnterpriseData_1_17_3` с профилем `2.20` и namespace пакета.
`XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin` доказывает XML QName
корня, порядок четырёх групп, импорты и QName-ссылки, локальные типы,
`ЛюбаяСсылка/СсылкаНаОбъект/typeDef` и самозакрывающийся
`СоставнойЛюбойОбъект`.
