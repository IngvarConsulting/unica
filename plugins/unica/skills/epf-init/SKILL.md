---
name: epf-init
description: Создать пустой make-ready scaffold внешней обработки 1С (EPF) в корне Designer/platform-XML external source-set, с модулем объекта и опциональной управляемой формой. Используй при запросе создать новую внешнюю обработку с нуля; не используй для обычного DataProcessor внутри конфигурации.
---

# Создание внешней обработки EPF

## MCP routing

- **Канонической операции создания внешней обработки на поверхности нет.** `unica.apply`
  правит существующий узел; дескриптор внешнего объекта вне конфигурации ни
  одна операция не заводит.
- Сформируй дескриптор и `ObjectModule.bsl` файловыми средствами по формату
  ниже, объяви набор нужного типа в `v8project.yaml`, затем проверь
  `unica.check {}` и прочитай `unica.view {}`: набор обязан читаться как
  `sourceFormat=platform_xml`.
- Сборку `.epf` словарь тоже не публикует: `make` собирает только
  `.cf` и `.cfe`. Сообщай это как пробел контракта, а не обходи runner-ом.
- Чтение и проверка идут через MCP `unica` (`unica.view`, `unica.check`); внутренние adapters и skill-local scripts не вызывать.
- Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`dryRunRequired`. Контракт вызова бери оттуда, а не из этого текста;
при `implemented: true` используй опубликованную `argsSchema`; при
`support.state: limited` разрешено только подмножество `support.supportedArgs`.
При `support.state: unavailable` остановись; не выдумывай аргументов при
`argsSchema: null`. Для плановой операции сначала проверь результат `dryRun: true`,
затем исполняй запрос с `dryRun: false`. Preview не фиксирует входы между
вызовами. Не обходи контракт прямым runner-ом.
- Сборку `.epf`/`.erf` из исходников словарь `unica.run` не публикует: `make` собирает только `.cf` и `.cfe`. Сообщай это как пробел контракта, а не обходи runner-ом.

## Порядок работы

1. Убедиться, что `v8project.yaml` использует Designer mode: явно `format: DESIGNER` либо поле `format` отсутствует и действует Designer default v8-runner. Skill создаёт platform XML, а EDT external-project layout не поддерживается.
2. Сохранить существующие `workPath`, `infobase`, credentials и local overrides. Не заменять connection string и не инициализировать существующую ИБ ради заготовки.
3. Найти source-set с `type: EXTERNAL_DATA_PROCESSORS` и писать файлы прямо в его `path`, без вложенного подкаталога: v8-runner ищет дескрипторы непосредственно в корне source-set.
4. Если source-set ещё не объявлен, записать файлы в выбранный новый каталог, затем явно добавить этот каталог как корень Designer source-set (фрагмент ниже). Проверить регистрацию через `unica.view {}`: `kind=external_processor`, `sourceFormat=platform_xml`.
5. Записать дескриптор `<Имя>.xml` и `<Имя>/Ext/ObjectModule.bsl` по образцам раздела «Формат файлов». Управляемую форму добавлять, только если она нужна: три файла под `<Имя>/Forms/` и ссылка на неё в дескрипторе.
6. Записав файлы, проверить набор: `unica.check {}` и `unica.view {}` — превью и применение тут не при чём, операции нет.
7. Публикацию артефакта не обещать: `make` в `unica.run` `.epf` не собирает и отвечает `unsupported_operation`.

Имя объекта и имя формы должны быть идентификаторами 1С. Существующий дескриптор или одноимённый каталог не перезаписывай. При `format: EDT` остановиться и объяснить несовместимость, не создавать Designer XML внутри EDT source-set.

В существующем валидном `v8project.yaml` добавить только этот фрагмент, используя ключ `source-set` в единственном числе и не затирая остальные поля:

```yaml
source-set:
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: src/external-processors
```

Для нового изолированного workspace полный минимальный пример имеет также обязательный runtime-контекст. `v8project.yaml`:

```yaml
workPath: build/runtime
format: DESIGNER
source-set:
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: src/external-processors
```

База этой машины описывается в `v8project.local.yaml`:

```yaml
infobases:
  origin:
    connection: 'File=build/ib'
```

Ключ `execution_timeout` не пиши: раннер его не поддерживает, и Unica отказывает в запуске с ним.

Не вызывай `infobase.create` ради заготовки или существующей проектной базы: он заводит базу, а для заготовки это не нужно. Для существующей connection сохранить настройки без переинициализации; `db-auth-check` может классифицировать только уже предоставленное runtime evidence и не запускает auth probe.

## Формат файлов

Образцы записаны в формате выгрузки 8.3.27 / 2.20. Полное описание — в [спецификации внешней обработки](../../references/specs/1c-epf-spec.md):
§3 — корневой файл, §4 — метаданные формы, §6 — `Form.xml`, §7 — модули. Метаданные (`<Имя>.xml`, `Forms/<Форма>.xml`) пиши в UTF-8;
платформа выгружает их с BOM. Каждый `<UUID>` замени новым UUID
(`uuidgen`), одинаковых быть не должно.

Дерево заготовки с формой `ОсновнаяФорма`:

```text
ИмпортТоваров.xml
ИмпортТоваров/Ext/ObjectModule.bsl
ИмпортТоваров/Forms/ОсновнаяФорма.xml
ИмпортТоваров/Forms/ОсновнаяФорма/Ext/Form.xml
ИмпортТоваров/Forms/ОсновнаяФорма/Ext/Form/Module.bsl
```

### Дескриптор `ИмпортТоваров.xml`

```xml
<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xen="http://v8.1c.ru/8.3/xcf/enums" xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<ExternalDataProcessor uuid="<UUID>">
		<InternalInfo>
			<xr:ContainedObject>
				<xr:ClassId>c3831ec8-d8d5-4f93-8a22-f9bfae07327f</xr:ClassId>
				<xr:ObjectId><UUID></xr:ObjectId>
			</xr:ContainedObject>
			<xr:GeneratedType name="ExternalDataProcessorObject.ИмпортТоваров" category="Object">
				<xr:TypeId><UUID></xr:TypeId>
				<xr:ValueId><UUID></xr:ValueId>
			</xr:GeneratedType>
		</InternalInfo>
		<Properties>
			<Name>ИмпортТоваров</Name>
			<Synonym>
				<v8:item>
					<v8:lang>ru</v8:lang>
					<v8:content>Импорт товаров</v8:content>
				</v8:item>
			</Synonym>
			<Comment/>
			<DefaultForm>ExternalDataProcessor.ИмпортТоваров.Form.ОсновнаяФорма</DefaultForm>
			<AuxiliaryForm/>
		</Properties>
		<ChildObjects>
			<Form>ОсновнаяФорма</Form>
		</ChildObjects>
	</ExternalDataProcessor>
</MetaDataObject>
```

- `ClassId` — константа класса `ExternalDataProcessor`; `ObjectId`, `TypeId`,
  `ValueId` — новые UUID.
- Имя в `GeneratedType`, `DefaultForm` и `Name` совпадает с именем файла.
- Без формы: `<DefaultForm/>` и `<ChildObjects/>`.

### Модуль объекта `ИмпортТоваров/Ext/ObjectModule.bsl`

```bsl
#Область ПрограммныйИнтерфейс

#КонецОбласти
```

### Форма (по запросу)

Метаданные формы `ИмпортТоваров/Forms/ОсновнаяФорма.xml`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xen="http://v8.1c.ru/8.3/xcf/enums" xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<Form uuid="<UUID>">
		<Properties>
			<Name>ОсновнаяФорма</Name>
			<Synonym>
				<v8:item>
					<v8:lang>ru</v8:lang>
					<v8:content>ОсновнаяФорма</v8:content>
				</v8:item>
			</Synonym>
			<Comment/>
			<FormType>Managed</FormType>
			<IncludeHelpInContents>false</IncludeHelpInContents>
			<UsePurposes>
				<v8:Value xsi:type="app:ApplicationUsePurpose">PlatformApplication</v8:Value>
				<v8:Value xsi:type="app:ApplicationUsePurpose">MobilePlatformApplication</v8:Value>
			</UsePurposes>
			<ExtendedPresentation/>
		</Properties>
	</Form>
</MetaDataObject>
```

Описание формы `ИмпортТоваров/Forms/ОсновнаяФорма/Ext/Form.xml` — пустая форма
с основным реквизитом `Объект`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:dcscor="http://v8.1c.ru/8.1/data-composition-system/core" xmlns:dcssch="http://v8.1c.ru/8.1/data-composition-system/schema" xmlns:dcsset="http://v8.1c.ru/8.1/data-composition-system/settings" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<Attributes>
		<Attribute name="Объект" id="1">
			<Type>
				<v8:Type>cfg:ExternalDataProcessorObject.ИмпортТоваров</v8:Type>
			</Type>
			<MainAttribute>true</MainAttribute>
		</Attribute>
	</Attributes>
</Form>
```

Модуль формы `ИмпортТоваров/Forms/ОсновнаяФорма/Ext/Form/Module.bsl` может быть
пустым; элементы и обработчики добавляются позже по
[form-edit](../form-edit/SKILL.md) и [form-events](../form-events/SKILL.md).

## Верификация

Проверь результат до того, как объявлять объект готовым. Проверить, что записаны `<Имя>.xml`, `<Имя>/Ext/ObjectModule.bsl` и, если нужна форма, три файла под `<Имя>/Forms/`. Форму дополнительно проверить `unica.check` по адресу её узла. Отдельный generic Meta validator не использовать: он не принимает root `ExternalDataProcessor`. Не создавать `Configuration.xml` или platform-generated CDFI sidecar; legitimate external descriptor может называться `ConfigDumpInfo.xml`, если пользователь выбрал такое имя объекта.

Сборку и загрузку артефакта словарь `unica.run` не публикует: `make` отвечает `unsupported_operation` на `.epf`/`.erf`, а `upload` сейчас недоступен вовсе. Сообщай публикацию как пробел контракта Unica MCP.
