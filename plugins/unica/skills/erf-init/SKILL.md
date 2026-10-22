---
name: erf-init
description: Создать пустой make-ready scaffold внешнего отчёта 1С (ERF) в корне Designer/platform-XML external source-set, с модулем объекта и опциональной управляемой формой. Используй при запросе создать новый внешний отчёт с нуля; не используй для обычного Report внутри конфигурации.
---

# Создание внешнего отчёта ERF

## MCP routing

- **Канонической операции создания внешнего отчёта на поверхности нет.** `unica.apply`
  правит существующий узел; дескриптор внешнего объекта вне конфигурации ни
  одна операция не заводит.
- Сформируй дескриптор и `ObjectModule.bsl` файловыми средствами по формату
  ниже, объяви набор нужного типа в `v8project.yaml`, затем проверь
  `unica.check {}` и прочитай `unica.view {}`: набор обязан читаться как
  `sourceFormat=platform_xml`.
- Для сборки `.erf` используй `unica.run` с `op: "make"`, относительным
  `output` и именем `sourceSet`. Нужны один объект в этом наборе и объявленный
  набор основной конфигурации для временной базы раннера.
- Чтение и проверка идут через MCP `unica` (`unica.view`, `unica.check`); внутренние adapters и skill-local scripts не вызывать.
- Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`dryRunRequired`. Контракт вызова бери оттуда, а не из этого текста;
используй опубликованную `argsSchema`; при `support.state: limited`
разрешено только подмножество `support.supportedArgs`. Недоступной операции
в словаре нет. Для плановой операции сначала проверь результат
`dryRun: true`, затем исполняй запрос с `dryRun: false`. Preview
не фиксирует входы между вызовами. Не обходи контракт прямым runner-ом.

## Порядок работы

1. Убедиться, что `v8project.yaml` использует Designer mode: явно `format: DESIGNER` либо поле `format` отсутствует и действует Designer default v8-runner. Skill создаёт platform XML, а EDT external-project layout не поддерживается.
2. Сохранить существующие `workPath`, `infobase`, credentials и local overrides. Не заменять connection string и не инициализировать существующую ИБ ради заготовки.
3. Найти source-set с `type: EXTERNAL_REPORTS` и писать файлы прямо в его `path`, без вложенного подкаталога: v8-runner ищет дескрипторы непосредственно в корне source-set.
4. Если source-set ещё не объявлен, записать файлы в выбранный новый каталог, затем явно добавить этот каталог как корень Designer source-set (фрагмент ниже). Проверить регистрацию через `unica.view {}`: `kind=external_report`, `sourceFormat=platform_xml`.
5. Записать дескриптор `<Имя>.xml` и `<Имя>/Ext/ObjectModule.bsl` по образцам раздела «Формат файлов». Управляемую форму добавлять, только если она нужна: три файла под `<Имя>/Forms/` и ссылка на неё в дескрипторе.
6. Записав файлы, проверить набор: `unica.check {}` и `unica.view {}` — превью и применение тут не при чём, операции нет.
7. Макет СКД в составе внешнего отчёта канонической операцией не создаётся: `template.add` применим только к объектам конфигурации, а внешний набор исходников не пишет ни одна операция. Макет и схему тоже кладут файлами по [спецификации отчёта](../../references/specs/1c-erf-spec.md); это тот же пробел контракта. По запросу собрать `.erf` через `make`: сначала preview, затем execute. При отсутствии основной конфигурации сообщить prerequisite; не создавать её автоматически.

Имя объекта и имя формы должны быть идентификаторами 1С. Существующий дескриптор или одноимённый каталог не перезаписывай. При `format: EDT` остановиться и объяснить несовместимость, не создавать Designer XML внутри EDT source-set.

В существующем валидном `v8project.yaml` добавить только этот фрагмент, используя ключ `source-set` в единственном числе и не затирая остальные поля:

```yaml
source-set:
  - name: external-reports
    type: EXTERNAL_REPORTS
    path: src/external-reports
```

Для нового изолированного workspace полный минимальный пример имеет также обязательный runtime-контекст. `v8project.yaml`:

```yaml
workPath: build/runtime
format: DESIGNER
source-set:
  - name: external-reports
    type: EXTERNAL_REPORTS
    path: src/external-reports
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

Образцы записаны в формате выгрузки 8.3.27 / 2.20. Полное описание — в [спецификации внешнего отчёта](../../references/specs/1c-erf-spec.md):
§2 — корневой файл, §6 — формы отчёта, §7 — модуль объекта, §10 — минимальный пример. Метаданные (`<Имя>.xml`, `Forms/<Форма>.xml`) пиши в UTF-8;
платформа выгружает их с BOM. Каждый `<UUID>` замени новым UUID
(`uuidgen`), одинаковых быть не должно.

Дерево заготовки с формой `ФормаОтчета`:

```text
Остатки.xml
Остатки/Ext/ObjectModule.bsl
Остатки/Forms/ФормаОтчета.xml
Остатки/Forms/ФормаОтчета/Ext/Form.xml
Остатки/Forms/ФормаОтчета/Ext/Form/Module.bsl
```

### Дескриптор `Остатки.xml`

```xml
<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xen="http://v8.1c.ru/8.3/xcf/enums" xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<ExternalReport uuid="<UUID>">
		<InternalInfo>
			<xr:ContainedObject>
				<xr:ClassId>e41aff26-25cf-4bb6-b6c1-3f478a75f374</xr:ClassId>
				<xr:ObjectId><UUID></xr:ObjectId>
			</xr:ContainedObject>
			<xr:GeneratedType name="ExternalReportObject.Остатки" category="Object">
				<xr:TypeId><UUID></xr:TypeId>
				<xr:ValueId><UUID></xr:ValueId>
			</xr:GeneratedType>
		</InternalInfo>
		<Properties>
			<Name>Остатки</Name>
			<Synonym>
				<v8:item>
					<v8:lang>ru</v8:lang>
					<v8:content>Остатки товаров</v8:content>
				</v8:item>
			</Synonym>
			<Comment/>
			<DefaultForm>ExternalReport.Остатки.Form.ФормаОтчета</DefaultForm>
			<AuxiliaryForm/>
			<MainDataCompositionSchema/>
			<DefaultSettingsForm/>
			<AuxiliarySettingsForm/>
			<DefaultVariantForm/>
			<VariantsStorage/>
			<SettingsStorage/>
		</Properties>
		<ChildObjects>
			<Form>ФормаОтчета</Form>
		</ChildObjects>
	</ExternalReport>
</MetaDataObject>
```

- `ClassId` — константа класса `ExternalReport`; `ObjectId`, `TypeId`,
  `ValueId` — новые UUID.
- Имя в `GeneratedType`, `DefaultForm` и `Name` совпадает с именем файла.
- Без формы: `<DefaultForm/>` и `<ChildObjects/>`.

### Модуль объекта `Остатки/Ext/ObjectModule.bsl`

```bsl
#Область ПрограммныйИнтерфейс

#КонецОбласти
```

### Форма (по запросу)

Метаданные формы `Остатки/Forms/ФормаОтчета.xml`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xen="http://v8.1c.ru/8.3/xcf/enums" xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<Form uuid="<UUID>">
		<Properties>
			<Name>ФормаОтчета</Name>
			<Synonym>
				<v8:item>
					<v8:lang>ru</v8:lang>
					<v8:content>ФормаОтчета</v8:content>
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

Описание формы `Остатки/Forms/ФормаОтчета/Ext/Form.xml` — пустая форма
с основным реквизитом `Объект`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:dcscor="http://v8.1c.ru/8.1/data-composition-system/core" xmlns:dcssch="http://v8.1c.ru/8.1/data-composition-system/schema" xmlns:dcsset="http://v8.1c.ru/8.1/data-composition-system/settings" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<ReportFormType>Main</ReportFormType>
	<AutoShowState>Auto</AutoShowState>
	<ReportResultViewMode>Auto</ReportResultViewMode>
	<ViewModeApplicationOnSetReportResult>Auto</ViewModeApplicationOnSetReportResult>
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<Attributes>
		<Attribute name="Объект" id="1">
			<Type>
				<v8:Type>cfg:ExternalReportObject.Остатки</v8:Type>
			</Type>
			<MainAttribute>true</MainAttribute>
		</Attribute>
	</Attributes>
</Form>
```

Модуль формы `Остатки/Forms/ФормаОтчета/Ext/Form/Module.bsl` может быть
пустым; элементы и обработчики добавляются позже по
[form-edit](../form-edit/SKILL.md) и [form-events](../form-events/SKILL.md).

## Верификация

Проверь результат до того, как объявлять объект готовым. Проверить, что записаны `<Имя>.xml`, `<Имя>/Ext/ObjectModule.bsl` и, если нужна форма, три файла под `<Имя>/Forms/`. Форму дополнительно проверить `unica.check` по адресу её узла. Отдельный generic Meta validator не использовать: он не принимает root `ExternalReport`. Не создавать `Configuration.xml`, platform-generated CDFI sidecar или СКД без запроса; legitimate external descriptor может называться `ConfigDumpInfo.xml`, если пользователь выбрал такое имя объекта.

После сборки проверь терминальную квитанцию `make`: путь, непустой размер и SHA-256. Соседние output-файлы должны сохраниться. `upload` остаётся недоступным; сборка не означает загрузку в пользовательскую базу.
