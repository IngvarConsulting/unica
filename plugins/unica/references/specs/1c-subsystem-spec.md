# Спецификация формата XML подсистем и командного интерфейса 1С

> Активный контракт Unica: платформа `8.3.27`, формат выгрузки `2.20`.

Формат: XML-выгрузка конфигурации 1С:Предприятие 8.3 (Конфигуратор → Конфигурация → Выгрузить конфигурацию в файлы).
Текущие инструкции и XML-примеры относятся к формату `2.20`; прежний формат упоминается только в исторической справке.

Источники: выгрузки Бухгалтерия предприятия (платформы 8.3.20, 8.3.24, 8.3.27), ERP 2 (8.3.24).

> **Связанные спецификации:**
> - Корневая структура конфигурации — [1c-configuration-spec.md](1c-configuration-spec.md)
> - Объекты метаданных — [1c-config-objects-spec.md](1c-config-objects-spec.md)
> - Сводный индекс — [1c-specs-index.md](1c-specs-index.md)

---

## 1. Структура каталогов

### 1.1. Подсистемы (Subsystems)

Подсистемы организуют иерархическое дерево в каталоге `Subsystems/`:

```
Subsystems/
├── ПодсистемаА.xml                       # Определение подсистемы
├── ПодсистемаА/                          # Каталог подсистемы (если есть вложенные или CommandInterface)
│   ├── Ext/
│   │   └── CommandInterface.xml          # Командный интерфейс подсистемы (опционально)
│   └── Subsystems/                       # Вложенные подсистемы (опционально)
│       ├── Дочерняя1.xml
│       ├── Дочерняя1/
│       │   ├── Ext/
│       │   │   └── CommandInterface.xml
│       │   └── Subsystems/
│       │       └── Внучатая1.xml         # Вложенность до 3+ уровней
│       └── Дочерняя2.xml
├── ПодсистемаБ.xml                       # Лист — без каталога
└── ...
```

**Правила:**
- Каждая подсистема имеет файл `<Имя>.xml` (обязательно)
- Каталог `<Имя>/` создаётся только если есть вложенные подсистемы или файл `Ext/CommandInterface.xml`
- Вложенные подсистемы хранятся в `<Имя>/Subsystems/` — рекурсивно повторяя ту же структуру
- Глубина вложенности не ограничена (на практике до 3–4 уровней)

### 1.2. Группы команд (CommandGroups)

```
CommandGroups/
├── Документы.xml
├── Отчеты.xml
├── Печать.xml
└── ...
```

Плоская структура — только XML-файлы, без подкаталогов.

### 1.3. Общие команды (CommonCommands)

```
CommonCommands/
├── АнализСчета.xml                     # Метаданные
├── АнализСчета/
│   └── Ext/
│       └── CommandModule.bsl           # Модуль команды
└── ...
```

### 1.4. Командный интерфейс конфигурации (корневой)

```
Ext/
└── CommandInterface.xml              # Глобальный командный интерфейс — порядок разделов
```

Корневой `CommandInterface.xml` определяет порядок подсистем верхнего уровня в панели разделов.

---

## 2. Пространства имён XML

### 2.1. Подсистема (файл метаданных)

Корневой элемент — `<MetaDataObject>`, стандартный набор деклараций:

```xml
<MetaDataObject
    xmlns="http://v8.1c.ru/8.3/MDClasses"
    xmlns:app="http://v8.1c.ru/8.2/managed-application/core"
    xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config"
    xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi"
    xmlns:ent="http://v8.1c.ru/8.1/data/enterprise"
    xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform"
    xmlns:style="http://v8.1c.ru/8.1/data/ui/style"
    xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system"
    xmlns:v8="http://v8.1c.ru/8.1/data/core"
    xmlns:v8ui="http://v8.1c.ru/8.1/data/ui"
    xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web"
    xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows"
    xmlns:xen="http://v8.1c.ru/8.3/xcf/enums"
    xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef"
    xmlns:xr="http://v8.1c.ru/8.3/xcf/readable"
    xmlns:xs="http://www.w3.org/2001/XMLSchema"
    xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    version="2.20">
```

### 2.2. Командный интерфейс (CommandInterface.xml)

Корневой элемент — `<CommandInterface>`:

```xml
<CommandInterface
    xmlns="http://v8.1c.ru/8.3/xcf/extrnprops"
    xmlns:xr="http://v8.1c.ru/8.3/xcf/readable"
    xmlns:xs="http://www.w3.org/2001/XMLSchema"
    xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    version="2.20">
```

---

## 3. Формат подсистемы

### 3.1. Общая структура

```xml
<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="..." version="2.20">
    <Subsystem uuid="<UUID>">
        <Properties>
            <Name>ИмяПодсистемы</Name>
            <Synonym>...</Synonym>
            <Comment/>
            <IncludeHelpInContents>true</IncludeHelpInContents>
            <IncludeInCommandInterface>true</IncludeInCommandInterface>
            <UseOneCommand>false</UseOneCommand>
            <Explanation>...</Explanation>
            <Picture>...</Picture>
            <Content>...</Content>
        </Properties>
        <ChildObjects>
            <Subsystem>Дочерняя1</Subsystem>
            <Subsystem>Дочерняя2</Subsystem>
        </ChildObjects>
    </Subsystem>
</MetaDataObject>
```

### 3.2. Свойства (Properties)

| Свойство | Тип | Обязательно | Описание |
|---|---|---|---|
| `Name` | string | да | Программное имя (CamelCase, без пробелов) |
| `Synonym` | LocalString | да | Отображаемое имя (локализованное) |
| `Comment` | string | да | Комментарий. Обычно пустой тег `<Comment/>` |
| `IncludeHelpInContents` | bool | да | Включать справку в содержание |
| `IncludeInCommandInterface` | bool | да | Отображать в командном интерфейсе (`true` — раздел видим) |
| `UseOneCommand` | bool | да | Использовать единственную команду (подсистема-ярлык). `true` — при открытии сразу выполняется единственная команда из Content |
| `Explanation` | LocalString | да | Подсказка при наведении на раздел. Пустой тег если не задана |
| `Picture` | Picture | да | Иконка раздела. Пустой тег `<Picture/>` если не задана |
| `Content` | Content | да | Список включённых объектов метаданных. Пустой тег `<Content/>` если нет |

#### Synonym, Explanation (LocalString)

Локализованная строка с элементами по языкам:

```xml
<Synonym>
    <v8:item>
        <v8:lang>ru</v8:lang>
        <v8:content>Администрирование</v8:content>
    </v8:item>
    <v8:item>
        <v8:lang>en</v8:lang>
        <v8:content>Administration</v8:content>
    </v8:item>
</Synonym>
```

Может быть пустым тегом: `<Explanation/>`

#### Picture

Ссылка на общую картинку конфигурации:

```xml
<Picture>
    <xr:Ref>CommonPicture.Администрирование</xr:Ref>
    <xr:LoadTransparent>false</xr:LoadTransparent>
</Picture>
```

- `xr:Ref` — ссылка на объект `CommonPicture.<Имя>`
- `xr:LoadTransparent` — загружать с прозрачностью (`true`/`false`)
- Пустой тег `<Picture/>` если иконка не задана

#### Content

Список объектов метаданных, включённых в подсистему:

```xml
<Content>
    <xr:Item xsi:type="xr:MDObjectRef">Catalog.Номенклатура</xr:Item>
    <xr:Item xsi:type="xr:MDObjectRef">Document.РеализацияТоваровУслуг</xr:Item>
    <xr:Item xsi:type="xr:MDObjectRef">Report.Продажи</xr:Item>
    <xr:Item xsi:type="xr:MDObjectRef">CommonCommand.Настройки</xr:Item>
    <xr:Item xsi:type="xr:MDObjectRef">FunctionalOption.ИспользоватьСклады</xr:Item>
    <xr:Item xsi:type="xr:MDObjectRef">Constant.ИспользоватьСклады</xr:Item>
    <xr:Item xsi:type="xr:MDObjectRef">CommandGroup.Настройки</xr:Item>
    <xr:Item xsi:type="xr:MDObjectRef">Role.ПодсистемаПродажи</xr:Item>
</Content>
```

Каждый элемент `<xr:Item>` имеет фиксированный атрибут `xsi:type="xr:MDObjectRef"` и содержит полный путь объекта в формате `<ТипОбъекта>.<Имя>`.

**Встречающиеся типы объектов в Content:**

| Тип | Пример |
|---|---|
| `Catalog` | `Catalog.Номенклатура` |
| `Document` | `Document.РеализацияТоваровУслуг` |
| `Report` | `Report.Продажи` |
| `DataProcessor` | `DataProcessor.ПанельАдминистрирования` |
| `CommonModule` | `CommonModule.МодульОбщий` |
| `CommonCommand` | `CommonCommand.Настройки` |
| `CommonForm` | `CommonForm.ФормаОбщая` |
| `CommonPicture` | `CommonPicture.КартинкаОбщая` |
| `CommandGroup` | `CommandGroup.Настройки` |
| `Constant` | `Constant.ИспользоватьСклады` |
| `FunctionalOption` | `FunctionalOption.ИспользоватьСклады` |
| `Enum` | `Enum.ВидыОпераций` |
| `Role` | `Role.ПодсистемаПродажи` |
| `InformationRegister` | `InformationRegister.КурсыВалют` |
| `AccumulationRegister` | `AccumulationRegister.Продажи` |
| `AccountingRegister` | `AccountingRegister.Хозрасчетный` |
| `ChartOfAccounts` | `ChartOfAccounts.Хозрасчетный` |
| `ChartOfCharacteristicTypes` | `ChartOfCharacteristicTypes.Виды` |
| `ExchangePlan` | `ExchangePlan.ОбменДанными` |
| `DocumentJournal` | `DocumentJournal.Журнал` |
| UUID (ссылка по идентификатору) | `cf4e9dea-4052-4a62-8427-0d37e8c47a23` |

> **UUID-ссылки**: Некоторые элементы Content содержат UUID вместо именованной ссылки. Это ссылки на объекты из расширений или объекты, которые были удалены/переименованы.

Пустой тег `<Content/>` — подсистема не содержит объектов (организующая подсистема).

### 3.3. Дочерние объекты (ChildObjects)

Список вложенных подсистем:

```xml
<ChildObjects>
    <Subsystem>НастройкиПрограммы</Subsystem>
    <Subsystem>Пользователи</Subsystem>
    <Subsystem>ПроведениеДокументов</Subsystem>
</ChildObjects>
```

- Каждый `<Subsystem>` содержит только **имя** дочерней подсистемы (не UUID, не путь)
- Порядок элементов значим — определяет порядок в дереве конфигурации
- Пустой тег `<ChildObjects/>` — лист дерева, нет вложенных подсистем
- Определения дочерних подсистем находятся в `<ИмяРодителя>/Subsystems/<ИмяДочерней>.xml`

### 3.4. Типовые паттерны подсистем

**Организующая подсистема** — содержит только дочерние, без собственных объектов:
```xml
<Content/>
<ChildObjects>
    <Subsystem>Дочерняя1</Subsystem>
    <Subsystem>Дочерняя2</Subsystem>
</ChildObjects>
```

**Листовая подсистема** — содержит объекты, без вложенных:
```xml
<Content>
    <xr:Item xsi:type="xr:MDObjectRef">Catalog.Товары</xr:Item>
</Content>
<ChildObjects/>
```

**Подсистема-ярлык** (`UseOneCommand=true`) — открывает единственную команду:
```xml
<UseOneCommand>true</UseOneCommand>
<Content>
    <xr:Item xsi:type="xr:MDObjectRef">DataProcessor.Помощь</xr:Item>
</Content>
<ChildObjects/>
```

**Смешанная подсистема** — и объекты, и дочерние:
```xml
<Content>
    <xr:Item xsi:type="xr:MDObjectRef">CommonModule.Модуль</xr:Item>
    <xr:Item xsi:type="xr:MDObjectRef">Report.Отчёт</xr:Item>
</Content>
<ChildObjects>
    <Subsystem>Дочерняя1</Subsystem>
</ChildObjects>
```

---

## 4. Формат командного интерфейса (CommandInterface.xml)

### 4.1. Общая структура

```xml
<?xml version="1.0" encoding="UTF-8"?>
<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops"
    xmlns:xr="http://v8.1c.ru/8.3/xcf/readable"
    xmlns:xs="http://www.w3.org/2001/XMLSchema"
    xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    version="2.20">
    <CommandsVisibility>...</CommandsVisibility>       <!-- опционально -->
    <CommandsPlacement>...</CommandsPlacement>          <!-- опционально -->
    <CommandsOrder>...</CommandsOrder>                  <!-- опционально -->
    <SubsystemsOrder>...</SubsystemsOrder>             <!-- опционально -->
    <GroupsOrder>...</GroupsOrder>                      <!-- опционально -->
</CommandInterface>
```

Все 5 дочерних элементов опциональны. Порядок фиксирован: `CommandsVisibility` → `CommandsPlacement` → `CommandsOrder` → `SubsystemsOrder` → `GroupsOrder`.

### 4.2. Два уровня CommandInterface

| Уровень | Путь | Типичное содержание |
|---|---|---|
| **Корневой** (конфигурация) | `Ext/CommandInterface.xml` | Только `SubsystemsOrder` — порядок разделов верхнего уровня |
| **Подсистема** | `Subsystems/<Имя>/Ext/CommandInterface.xml` | Любая комбинация из 5 секций |

#### Корневой командный интерфейс

Содержит только порядок подсистем верхнего уровня:

```xml
<CommandInterface xmlns="..." version="2.20">
    <SubsystemsOrder>
        <Subsystem>Subsystem.Руководителю</Subsystem>
        <Subsystem>Subsystem.БанкИКасса</Subsystem>
        <Subsystem>Subsystem.Продажи</Subsystem>
        <Subsystem>Subsystem.Покупки</Subsystem>
        <Subsystem>Subsystem.Администрирование</Subsystem>
    </SubsystemsOrder>
</CommandInterface>
```

### 4.3. CommandsVisibility — видимость команд

Управляет видимостью отдельных команд в разделе:

```xml
<CommandsVisibility>
    <Command name="Catalog.Номенклатура.StandardCommand.OpenList">
        <Visibility>
            <xr:Common>false</xr:Common>
        </Visibility>
    </Command>
    <Command name="Report.Продажи.Command.ПродажиПоКонтрагентам">
        <Visibility>
            <xr:Common>true</xr:Common>
        </Visibility>
    </Command>
</CommandsVisibility>
```

**Элемент `Command`:**
- Атрибут `name` — полный путь команды (см. [формат ссылок на команды](#46-формат-ссылок-на-команды))
- `Visibility` > `xr:Common` — видимость для всех пользователей: `true` (видима) / `false` (скрыта)

> **Семантика**: Определяет какие команды видимы/скрыты **по умолчанию** для всех пользователей в данном разделе. Перечисляются только команды, чья видимость **отличается** от автоматической.

### 4.4. CommandsPlacement — размещение команд в группах

Определяет в какую группу команд помещается команда:

```xml
<CommandsPlacement>
    <Command name="Report.Продажи.Command.ПродажиПоКонтрагентам">
        <CommandGroup>CommandGroup.Отчеты</CommandGroup>
        <Placement>Auto</Placement>
    </Command>
    <Command name="DataProcessor.Настройки.Command.Открыть">
        <CommandGroup>NavigationPanelOrdinary</CommandGroup>
        <Placement>Auto</Placement>
    </Command>
</CommandsPlacement>
```

**Элемент `Command`:**
- `name` — ссылка на команду
- `CommandGroup` — идентификатор группы (см. [стандартные группы](#47-группы-команд))
- `Placement` — стратегия размещения. Единственное наблюдаемое значение: `Auto`

### 4.5. CommandsOrder — порядок команд

Определяет порядок отображения команд внутри групп:

```xml
<CommandsOrder>
    <Command name="Document.СчётНаОплату.StandardCommand.Create">
        <CommandGroup>ActionsPanelCreate</CommandGroup>
    </Command>
    <Command name="Document.Реализация.StandardCommand.Create">
        <CommandGroup>ActionsPanelCreate</CommandGroup>
    </Command>
    <Command name="Report.Продажи.Command.ПродажиПоКонтрагентам">
        <CommandGroup>CommandGroup.Отчеты</CommandGroup>
    </Command>
</CommandsOrder>
```

**Элемент `Command`:**
- `name` — ссылка на команду
- `CommandGroup` — группа, в которой определяется порядок

Команды отображаются в порядке их перечисления в `CommandsOrder`.

### 4.6. SubsystemsOrder — порядок вложенных подсистем

Определяет порядок дочерних подсистем в навигационной панели:

```xml
<SubsystemsOrder>
    <Subsystem>Subsystem.Продажи.Subsystem.ПродажиГлавное</Subsystem>
    <Subsystem>Subsystem.Продажи.Subsystem.РозничныеПродажи</Subsystem>
    <Subsystem>Subsystem.Продажи.Subsystem.РасчетыСКонтрагентами</Subsystem>
</SubsystemsOrder>
```

**Формат пути подсистемы:**
- Корневой уровень: `Subsystem.<Имя>`
- Вложенный уровень: `Subsystem.<Родитель>.Subsystem.<Дочерняя>`
- Глубже: `Subsystem.<A>.Subsystem.<B>.Subsystem.<C>`

> **В корневом CommandInterface** используются пути `Subsystem.<Имя>` (верхний уровень).
> **В CommandInterface подсистемы** используются полные пути от корня: `Subsystem.<Родитель>.Subsystem.<Дочерняя>`.

### 4.7. GroupsOrder — порядок групп команд

Определяет порядок отображения самих групп:

```xml
<GroupsOrder>
    <Group>NavigationPanelOrdinary</Group>
    <Group>CommandGroup.НачальноеЗаполнение</Group>
    <Group>NavigationPanelSeeAlso</Group>
    <Group>ActionsPanelTools</Group>
    <Group>CommandGroup.Отчеты</Group>
    <Group>CommandGroup.Сервис</Group>
    <Group>CommandGroup.Информация</Group>
</GroupsOrder>
```

---

## 5. Формат группы команд (CommandGroup)

### 5.1. Структура каталогов

```
CommandGroups/
├── Документы.xml
├── Отчеты.xml
├── Печать.xml
├── Настройки.xml
└── ...
```

Плоская структура — только XML-файлы, без подкаталогов и модулей.

### 5.2. Общая структура XML

```xml
<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="..." version="2.20">
    <CommandGroup uuid="<UUID>">
        <Properties>
            <Name>Документы</Name>
            <Synonym>...</Synonym>
            <Comment/>
            <Representation>Auto</Representation>
            <ToolTip>...</ToolTip>
            <Picture>...</Picture>
            <Category>ActionsPanel</Category>
        </Properties>
    </CommandGroup>
</MetaDataObject>
```

### 5.3. Свойства (Properties)

| Свойство | Тип | Обязательно | Описание |
|---|---|---|---|
| `Name` | string | да | Программное имя (CamelCase) |
| `Synonym` | LocalString | да | Отображаемое имя |
| `Comment` | string | да | Комментарий. Обычно пустой тег |
| `Representation` | enum | да | Способ отображения |
| `ToolTip` | LocalString | да | Подсказка. Пустой тег если не задана |
| `Picture` | Picture | да | Иконка. Пустой тег если не задана |
| `Category` | enum | да | Категория размещения |

#### Representation

| Значение | Описание |
|---|---|
| `Auto` | По умолчанию (наиболее частое) |
| `Picture` | Только картинка |
| `PictureAndText` | Картинка с текстом |

#### Category

| Значение | Описание |
|---|---|
| `ActionsPanel` | Панель действий (главное меню раздела) |
| `FormCommandBar` | Командная панель формы |
| `NavigationPanel` | Панель навигации |

#### Picture (расширенный формат)

```xml
<Picture>
    <xr:Ref>StdPicture.Print</xr:Ref>
    <xr:LoadTransparent>true</xr:LoadTransparent>
    <xr:TransparentPixel x="7" y="5"/>          <!-- опционально -->
</Picture>
```

- `xr:Ref` — ссылка на картинку: `StdPicture.<Имя>`, `CommonPicture.<Имя>`, или `0` (числовая ссылка)
- `xr:LoadTransparent` — загружать с прозрачностью
- `xr:TransparentPixel` — координаты прозрачного пикселя (опционально)

### 5.4. Пример: минимальная группа

```xml
<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="..." version="2.20">
    <CommandGroup uuid="774911fc-5b94-4ac7-8923-fd317727c671">
        <Properties>
            <Name>Документы</Name>
            <Synonym>
                <v8:item>
                    <v8:lang>ru</v8:lang>
                    <v8:content>Документы</v8:content>
                </v8:item>
            </Synonym>
            <Comment/>
            <Representation>Auto</Representation>
            <ToolTip/>
            <Picture/>
            <Category>ActionsPanel</Category>
        </Properties>
    </CommandGroup>
</MetaDataObject>
```

### 5.5. Пример: группа с картинкой и подсказкой

```xml
<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="..." version="2.20">
    <CommandGroup uuid="ac39b903-0c60-417e-a50f-49ed375424f5">
        <Properties>
            <Name>Печать</Name>
            <Synonym>
                <v8:item>
                    <v8:lang>ru</v8:lang>
                    <v8:content>Печать</v8:content>
                </v8:item>
            </Synonym>
            <Comment>Печать</Comment>
            <Representation>Auto</Representation>
            <ToolTip>
                <v8:item>
                    <v8:lang>ru</v8:lang>
                    <v8:content>Печать</v8:content>
                </v8:item>
            </ToolTip>
            <Picture>
                <xr:Ref>StdPicture.Print</xr:Ref>
                <xr:LoadTransparent>true</xr:LoadTransparent>
            </Picture>
            <Category>FormCommandBar</Category>
        </Properties>
    </CommandGroup>
</MetaDataObject>
```

---

## 6. Формат общей команды (CommonCommand)

### 6.1. Структура каталогов

```
CommonCommands/
├── АнализСчета.xml                     # Метаданные команды
├── АнализСчета/
│   └── Ext/
│       └── CommandModule.bsl           # Модуль команды
├── НастройкиСинхронизации.xml
├── НастройкиСинхронизации/
│   └── Ext/
│       └── CommandModule.bsl
└── ...
```

Каждая команда состоит из:
- `<Имя>.xml` — определение метаданных (обязательно)
- `<Имя>/Ext/CommandModule.bsl` — модуль реализации (обязательно)

### 6.2. Общая структура XML

```xml
<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="..." version="2.20">
    <CommonCommand uuid="<UUID>">
        <Properties>
            <Name>АнализСчета</Name>
            <Synonym>...</Synonym>
            <Comment/>
            <Group>NavigationPanelOrdinary</Group>
            <Representation>Auto</Representation>
            <ToolTip>...</ToolTip>
            <Picture/>
            <Shortcut/>
            <IncludeHelpInContents>false</IncludeHelpInContents>
            <CommandParameterType/>
            <ParameterUseMode>Single</ParameterUseMode>
            <ModifiesData>false</ModifiesData>
            <OnMainServerUnavalableBehavior>Auto</OnMainServerUnavalableBehavior>
        </Properties>
    </CommonCommand>
</MetaDataObject>
```

### 6.3. Свойства (Properties)

| Свойство | Тип | Обязательно | Описание |
|---|---|---|---|
| `Name` | string | да | Программное имя |
| `Synonym` | LocalString | да | Отображаемое имя |
| `Comment` | string | да | Комментарий |
| `Group` | string | да | Группа размещения команды |
| `Representation` | enum | да | Способ отображения: `Auto`, `PictureAndText` |
| `ToolTip` | LocalString | да | Подсказка |
| `Picture` | Picture | да | Иконка |
| `Shortcut` | string | да | Сочетание клавиш. Пример: `F11`. Пустой тег если нет |
| `IncludeHelpInContents` | bool | да | Включать в содержание справки |
| `CommandParameterType` | TypeList | да | Типы параметра команды |
| `ParameterUseMode` | enum | да | Режим использования параметра: `Single` или `Multiple` |
| `ModifiesData` | bool | да | Модифицирует данные |
| `OnMainServerUnavalableBehavior` | enum | да | Поведение при недоступности сервера: `Auto` |

#### Group — стандартные группы размещения

Команда размещается в одной из групп:

**Панель навигации раздела:**
- `NavigationPanelOrdinary` — обычные команды навигации
- `NavigationPanelImportant` — важные команды навигации

**Панель навигации формы:**
- `FormNavigationPanelGoTo` — «Перейти» в форме
- `FormNavigationPanelImportant` — важные в навигации формы

**Командная панель формы:**
- `FormCommandBarImportant` — важные на панели команд формы
- `FormCommandBarCreateBasedOn` — «Создать на основании»

**Панель действий:**
- `ActionsPanelTools` — сервисные действия
- `ActionsPanelReports` — отчёты в панели действий

**Пользовательская группа:**
- `CommandGroup.<Имя>` — ссылка на группу команд

#### CommandParameterType — типы параметра

Определяет к каким объектам привязана команда. Если пусто — команда глобальная.

**Без параметра (глобальная команда):**
```xml
<CommandParameterType/>
```

**Один тип:**
```xml
<CommandParameterType>
    <v8:Type>cfg:CatalogRef.Сотрудники</v8:Type>
</CommandParameterType>
```

**Несколько типов:**
```xml
<CommandParameterType>
    <v8:Type>cfg:ExchangePlanRef.Полный</v8:Type>
    <v8:Type>cfg:ExchangePlanRef.ПоОрганизации</v8:Type>
    <v8:Type>cfg:ExchangePlanRef.АвтономнаяРабота</v8:Type>
</CommandParameterType>
```

Формат типов: `cfg:<ТипСсылки>.<ИмяОбъекта>`, где типы ссылок:
- `cfg:CatalogRef.<Имя>` — ссылка на справочник
- `cfg:DocumentRef.<Имя>` — ссылка на документ
- `cfg:ExchangePlanRef.<Имя>` — ссылка на план обмена

---

## 7. Команды объектов (встроенные команды)

Помимо общих команд (`CommonCommands/`), команды могут быть определены внутри объектов метаданных (справочников, документов, обработок и т.д.).

### 7.1. Структура

Команды определяются в секции `<ChildObjects>` родительского объекта и хранят модуль в подкаталоге:

```
Catalogs/
├── Номенклатура.xml              # В ChildObjects определены Command-ы
└── Номенклатура/
    └── Commands/
        └── КомандаИсторияДвижений/
            └── Ext/
                └── CommandModule.bsl   # Модуль команды
```

### 7.2. XML-определение (внутри ChildObjects родителя)

```xml
<ChildObjects>
    <Form>ФормаЭлемента</Form>
    <Form>ФормаСписка</Form>
    <Command uuid="d95fb51f-719f-4d67-ab58-b8144e3f2b7e">
        <Properties>
            <Name>КомандаИсторияДвижений</Name>
            <Synonym>
                <v8:item>
                    <v8:lang>ru</v8:lang>
                    <v8:content>История движений</v8:content>
                </v8:item>
            </Synonym>
            <Comment/>
            <Group>FormNavigationPanelImportant</Group>
            <CommandParameterType>
                <v8:Type>cfg:CatalogRef.Номенклатура</v8:Type>
            </CommandParameterType>
            <ParameterUseMode>Single</ParameterUseMode>
            <ModifiesData>false</ModifiesData>
            <Representation>Auto</Representation>
            <ToolTip/>
            <Picture/>
            <Shortcut/>
            <OnMainServerUnavalableBehavior>Auto</OnMainServerUnavalableBehavior>
        </Properties>
    </Command>
</ChildObjects>
```

### 7.3. Свойства команды объекта

Набор свойств идентичен CommonCommand (см. [раздел 6.3](#63-свойства-properties-1)), за исключением отсутствия `IncludeHelpInContents`.

| Свойство | Описание |
|---|---|
| `Name` | Программное имя команды |
| `Synonym` | Отображаемое имя |
| `Comment` | Комментарий |
| `Group` | Группа размещения (те же значения что у CommonCommand) |
| `CommandParameterType` | Тип параметра (обычно ссылка на тип владельца) |
| `ParameterUseMode` | `Single` или `Multiple` |
| `ModifiesData` | Модифицирует данные (`true`/`false`) |
| `Representation` | Способ отображения |
| `ToolTip` | Подсказка |
| `Picture` | Иконка |
| `Shortcut` | Сочетание клавиш |
| `OnMainServerUnavalableBehavior` | Поведение при недоступности сервера |

> **Порядок свойств**: В командах объектов `Representation` идёт **после** `ModifiesData`, а в CommonCommand — после `Comment`. Это единственное структурное различие.

### 7.4. Ссылка на команду объекта в CommandInterface

В `CommandInterface.xml` команда объекта адресуется как:
```
<ТипОбъекта>.<ИмяОбъекта>.Command.<ИмяКоманды>
```

Пример: `Catalog.Номенклатура.Command.КомандаИсторияДвижений`

---

## 8. Формат ссылок на команды в CommandInterface

Атрибут `name` в элементах `Command` использует точечную нотацию:

### 8.1. Стандартные команды (StandardCommand)

```
<ТипОбъекта>.<ИмяОбъекта>.StandardCommand.<Операция>
```

| Операция | Назначение |
|---|---|
| `OpenList` | Открыть список |
| `Open` | Открыть (для отчётов, обработок, общих форм) |
| `Create` | Создать новый |
| `CreateFolder` | Создать группу (для иерархических справочников) |

**Примеры:**
- `Catalog.Номенклатура.StandardCommand.OpenList`
- `Document.Реализация.StandardCommand.Create`
- `Report.Продажи.StandardCommand.Open`
- `DataProcessor.Настройки.StandardCommand.Open`
- `CommonForm.ФормаОбщая.StandardCommand.Open`
- `InformationRegister.КурсыВалют.StandardCommand.OpenList`
- `DocumentJournal.ЖурналДокументов.StandardCommand.OpenList`

### 8.2. Пользовательские команды (Command)

```
<ТипОбъекта>.<ИмяОбъекта>.Command.<ИмяКоманды>
```

**Примеры:**
- `Report.Продажи.Command.ПродажиПоКонтрагентам`
- `DataProcessor.Панель.Command.ОткрытьНастройки`
- `Catalog.Оборудование.Command.ОткрытьОборудование`
- `InformationRegister.Настройки.Command.ОбменДанными`
- `ExchangePlan.Мобильное.Command.ОткрытьНастройки`

### 8.3. Общие команды (CommonCommand)

```
CommonCommand.<ИмяКоманды>
```

**Примеры:**
- `CommonCommand.НастройкиСинхронизацииДанных`
- `CommonCommand.ДополнительныеОбработкиПродажи`
- `CommonCommand.ПерсональныеНастройки`

### 8.4. UUID-ссылки

```
0:<UUID>
```

**Пример:** `0:91941f81-07dd-43a0-9baa-f5969b7472db`

Формат `0:<UUID>` используется для команд из расширений или ссылок на объекты, недоступные по имени.

---

## 9. Группы команд в CommandInterface

### 9.1. Стандартные группы панелей

| Идентификатор | Панель | Описание |
|---|---|---|
| `NavigationPanelImportant` | Навигация | Важные команды (верхняя часть) |
| `NavigationPanelOrdinary` | Навигация | Обычные команды |
| `NavigationPanelSeeAlso` | Навигация | Блок «См. также» (нижняя часть) |
| `ActionsPanelCreate` | Действия | Блок «Создать» |
| `ActionsPanelTools` | Действия | Блок «Сервис» |

### 9.2. Пользовательские группы

```
CommandGroup.<ИмяГруппы>
```

Ссылаются на объекты метаданных `CommandGroup`, определённые в конфигурации (каталог `CommandGroups/`).

**Типичные примеры:**
- `CommandGroup.Настройки`
- `CommandGroup.Отчеты`
- `CommandGroup.Сервис`
- `CommandGroup.Информация`
- `CommandGroup.НачальноеЗаполнение`
- `CommandGroup.НДС`

---

<!-- legacy-format-reference:start -->
## 10. Различия версий формата

| Аспект | `2.17` (8.3.20–8.3.24) | `2.20` (исследовано на 8.3.27) |
|---|---|---|
| Атрибут `version` | `2.17` | `2.20` |
| BOM (UTF-8 BOM) | Нет | Да (EF BB BF) |
| Набор элементов | Идентичный | Идентичный |
| Набор свойств подсистемы | Идентичный | Идентичный |
| Секции CommandInterface | Идентичные | Идентичные |

> Между исследованными версиями `2.17` и `2.20` структурных различий в формате подсистем и командного интерфейса не обнаружено. Это историческое наблюдение по образцам, а не разрешение генерировать прежний формат.

<!-- legacy-format-reference:end -->

---

## 11. Полные примеры

### 11.1. Подсистема верхнего уровня (Продажи.xml)

```xml
<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"
    xmlns:v8="http://v8.1c.ru/8.1/data/core"
    xmlns:xr="http://v8.1c.ru/8.3/xcf/readable"
    xmlns:xs="http://www.w3.org/2001/XMLSchema"
    xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    version="2.20">
    <Subsystem uuid="2885099a-00f4-481b-8469-faee8a208e7c">
        <Properties>
            <Name>Продажи</Name>
            <Synonym>
                <v8:item>
                    <v8:lang>ru</v8:lang>
                    <v8:content>Продажи</v8:content>
                </v8:item>
            </Synonym>
            <Comment/>
            <IncludeHelpInContents>true</IncludeHelpInContents>
            <IncludeInCommandInterface>true</IncludeInCommandInterface>
            <UseOneCommand>false</UseOneCommand>
            <Explanation/>
            <Picture>
                <xr:Ref>CommonPicture.Продажи</xr:Ref>
                <xr:LoadTransparent>false</xr:LoadTransparent>
            </Picture>
            <Content>
                <xr:Item xsi:type="xr:MDObjectRef">Report.Продажи</xr:Item>
                <xr:Item xsi:type="xr:MDObjectRef">Report.ВаловаяПрибыль</xr:Item>
                <xr:Item xsi:type="xr:MDObjectRef">CommonCommand.ДополнительныеОбработкиПродажи</xr:Item>
                <xr:Item xsi:type="xr:MDObjectRef">CommonCommand.ДополнительныеОтчетыПродажи</xr:Item>
                <xr:Item xsi:type="xr:MDObjectRef">Role.ПодсистемаПродажи</xr:Item>
            </Content>
        </Properties>
        <ChildObjects>
            <Subsystem>ПродажиГлавное</Subsystem>
            <Subsystem>РозничныеПродажи</Subsystem>
            <Subsystem>РасчетыСКонтрагентами</Subsystem>
        </ChildObjects>
    </Subsystem>
</MetaDataObject>
```

### 11.2. CommandInterface подсистемы (Продажи/Ext/CommandInterface.xml)

```xml
<?xml version="1.0" encoding="UTF-8"?>
<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops"
    xmlns:xr="http://v8.1c.ru/8.3/xcf/readable"
    xmlns:xs="http://www.w3.org/2001/XMLSchema"
    xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    version="2.20">
    <CommandsVisibility>
        <Command name="Report.Продажи.Command.ПродажиПоКонтрагентам">
            <Visibility>
                <xr:Common>true</xr:Common>
            </Visibility>
        </Command>
        <Command name="Report.Продажи.Command.ПродажиПоНоменклатуре">
            <Visibility>
                <xr:Common>true</xr:Common>
            </Visibility>
        </Command>
        <Command name="CommonCommand.ДополнительныеОбработкиПродажи">
            <Visibility>
                <xr:Common>true</xr:Common>
            </Visibility>
        </Command>
    </CommandsVisibility>
    <CommandsPlacement>
        <Command name="Report.Продажи.Command.ПродажиПоКонтрагентам">
            <CommandGroup>CommandGroup.Отчеты</CommandGroup>
            <Placement>Auto</Placement>
        </Command>
        <Command name="CommonCommand.ДополнительныеОбработкиПродажи">
            <CommandGroup>CommandGroup.Сервис</CommandGroup>
            <Placement>Auto</Placement>
        </Command>
    </CommandsPlacement>
    <CommandsOrder>
        <Command name="Report.Продажи.Command.ПродажиПоКонтрагентам">
            <CommandGroup>CommandGroup.Отчеты</CommandGroup>
        </Command>
        <Command name="Report.Продажи.Command.ПродажиПоНоменклатуре">
            <CommandGroup>CommandGroup.Отчеты</CommandGroup>
        </Command>
        <Command name="CommonCommand.ДополнительныеОбработкиПродажи">
            <CommandGroup>CommandGroup.Сервис</CommandGroup>
        </Command>
    </CommandsOrder>
    <SubsystemsOrder>
        <Subsystem>Subsystem.Продажи.Subsystem.ПродажиГлавное</Subsystem>
        <Subsystem>Subsystem.Продажи.Subsystem.РозничныеПродажи</Subsystem>
        <Subsystem>Subsystem.Продажи.Subsystem.РасчетыСКонтрагентами</Subsystem>
    </SubsystemsOrder>
    <GroupsOrder>
        <Group>ActionsPanelTools</Group>
        <Group>CommandGroup.Отчеты</Group>
        <Group>CommandGroup.Сервис</Group>
    </GroupsOrder>
</CommandInterface>
```

### 11.3. Корневой CommandInterface (Ext/CommandInterface.xml)

```xml
<?xml version="1.0" encoding="UTF-8"?>
<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops"
    xmlns:xr="http://v8.1c.ru/8.3/xcf/readable"
    xmlns:xs="http://www.w3.org/2001/XMLSchema"
    xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    version="2.20">
    <SubsystemsOrder>
        <Subsystem>Subsystem.Руководителю</Subsystem>
        <Subsystem>Subsystem.БанкИКасса</Subsystem>
        <Subsystem>Subsystem.Продажи</Subsystem>
        <Subsystem>Subsystem.Покупки</Subsystem>
        <Subsystem>Subsystem.Администрирование</Subsystem>
    </SubsystemsOrder>
</CommandInterface>
```
