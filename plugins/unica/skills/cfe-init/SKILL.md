---
name: cfe-init
description: Создать расширение конфигурации 1С (CFE) — scaffold XML-исходников. Используй когда нужно создать новое расширение для исправления, доработки или дополнения конфигурации
argument-hint: <Имя> [<каталог набора>] [Patch|Customization|AddOn]
allowed-tools:
  - Bash
  - Read
  - Write
  - Glob
---

# /cfe-init — Создание расширения конфигурации 1С

## MCP routing

- **Канонической операции создания расширения на поверхности нет.** Словарь
  `unica.apply` правит существующий узел; корень расширения с его
  `NamePrefix`, `ConfigurationExtensionPurpose` и связью с родителем ни одна
  операция не заводит.
- Не зови внутренние адаптеры напрямую: они спрятаны за MCP `unica`.
- Готовность проверяет `unica.check`, результат читает `unica.view` по адресу.

Корень расширения записывается файловыми средствами по образцам ниже. Нужна
именно операция создания расширения — это пробел контракта Unica MCP; сообщи о
нём, а не подменяй его правкой родительской конфигурации.

## Порядок работы

1. Найди родительскую конфигурацию через `unica.view {}`. Из её файлов
   возьми `uuid` элемента `<Language>` в `Languages/Русский.xml`, атрибут
   `version` у `MetaDataObject` и значения `CompatibilityMode` и
   `InterfaceCompatibilityMode` из `Configuration.xml`.
2. Выбери пустой каталог набора, например `src/ext`. Существующий
   `Configuration.xml` не перезаписывай.
3. Запиши `Configuration.xml` и `Languages/Русский.xml` по образцам ниже;
   роль `Roles/<Префикс>ОсновнаяРоль.xml` — если расширению нужна роль
   по умолчанию. Каждый `<UUID>` замени новым UUID (`uuidgen`). Каталога
   `Ext/` у корня расширения нет; `ConfigDumpInfo.xml` не создавай.
4. Объяви каталог набором в `v8project.yaml`, не затирая остальные поля:

   ```yaml
   source-set:
     - name: ext
       type: EXTENSION
       path: src/ext
   ```

5. Убедись в допуске набора: `unica.check {}`.
6. Прочитай корень: `unica.view {at: "ext:Configuration"}`.
7. Объекты родителя заимствуй операцией `object.borrow` по
   [cfe-borrow](../cfe-borrow/SKILL.md); перехват метода пиши по
   [cfe-patch-method](../cfe-patch-method/SKILL.md).

## Формат корня

Состав повторяет заготовку, которую платформа 8.3.27 загрузила в расширение
(`ibcmd config import --extension`) и выгрузила в формате 2.20. Свойства,
их порядок и отличия от конфигурации описаны в
[спецификации расширения](../../references/specs/1c-extension-spec.md):
§2 — `Configuration.xml`, §9 — языки, §10 — роли. Файлы метаданных пиши
в UTF-8; платформа выгружает их с BOM.

### `Configuration.xml`

```xml
<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xen="http://v8.1c.ru/8.3/xcf/enums" xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<Configuration uuid="<UUID>">
		<InternalInfo>
			<xr:ContainedObject>
				<xr:ClassId>9cd510cd-abfc-11d4-9434-004095e12fc7</xr:ClassId>
				<xr:ObjectId><UUID></xr:ObjectId>
			</xr:ContainedObject>
			<xr:ContainedObject>
				<xr:ClassId>9fcd25a0-4822-11d4-9414-008048da11f9</xr:ClassId>
				<xr:ObjectId><UUID></xr:ObjectId>
			</xr:ContainedObject>
			<xr:ContainedObject>
				<xr:ClassId>e3687481-0a87-462c-a166-9f34594f9bba</xr:ClassId>
				<xr:ObjectId><UUID></xr:ObjectId>
			</xr:ContainedObject>
			<xr:ContainedObject>
				<xr:ClassId>9de14907-ec23-4a07-96f0-85521cb6b53b</xr:ClassId>
				<xr:ObjectId><UUID></xr:ObjectId>
			</xr:ContainedObject>
			<xr:ContainedObject>
				<xr:ClassId>51f2d5d8-ea4d-4064-8892-82951750031e</xr:ClassId>
				<xr:ObjectId><UUID></xr:ObjectId>
			</xr:ContainedObject>
			<xr:ContainedObject>
				<xr:ClassId>e68182ea-4237-4383-967f-90c1e3370bc7</xr:ClassId>
				<xr:ObjectId><UUID></xr:ObjectId>
			</xr:ContainedObject>
			<xr:ContainedObject>
				<xr:ClassId>fb282519-d103-4dd3-bc12-cb271d631dfc</xr:ClassId>
				<xr:ObjectId><UUID></xr:ObjectId>
			</xr:ContainedObject>
		</InternalInfo>
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>Расш1</Name>
			<Synonym>
				<v8:item>
					<v8:lang>ru</v8:lang>
					<v8:content>Расширение 1</v8:content>
				</v8:item>
			</Synonym>
			<Comment/>
			<ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>
			<KeepMappingToExtendedConfigurationObjectsByIDs>true</KeepMappingToExtendedConfigurationObjectsByIDs>
			<NamePrefix>Расш1_</NamePrefix>
			<ConfigurationExtensionCompatibilityMode>Version8_3_27</ConfigurationExtensionCompatibilityMode>
			<DefaultRunMode>ManagedApplication</DefaultRunMode>
			<UsePurposes>
				<v8:Value xsi:type="app:ApplicationUsePurpose">PlatformApplication</v8:Value>
			</UsePurposes>
			<ScriptVariant>Russian</ScriptVariant>
			<DefaultRoles>
				<xr:Item xsi:type="xr:MDObjectRef">Role.Расш1_ОсновнаяРоль</xr:Item>
			</DefaultRoles>
			<Vendor/>
			<Version/>
			<DefaultLanguage>Language.Русский</DefaultLanguage>
			<BriefInformation/>
			<DetailedInformation/>
			<Copyright/>
			<VendorInformationAddress/>
			<ConfigurationInformationAddress/>
			<InterfaceCompatibilityMode>TaxiEnableVersion8_2</InterfaceCompatibilityMode>
		</Properties>
		<ChildObjects>
			<Language>Русский</Language>
			<Role>Расш1_ОсновнаяРоль</Role>
		</ChildObjects>
	</Configuration>
</MetaDataObject>
```

Что подставить:

- `Name` — идентификатор 1С; `Synonym` — представление.
- `ConfigurationExtensionPurpose` — `Patch` (исправление), `Customization`
  (доработка) или `AddOn` (дополнение).
- `NamePrefix` — префикс собственных объектов расширения, например `ИБ_`.
  Имя роли начинается с этого же префикса.
- `ConfigurationExtensionCompatibilityMode` — значение `CompatibilityMode`
  родителя, например `Version8_3_17`. `InterfaceCompatibilityMode` и атрибут
  `version` у `MetaDataObject` тоже бери у родителя.
- `Vendor` и `Version` заполняются при необходимости:
  `<Vendor>Компания</Vendor>`, `<Version>1.0.0.1</Version>`.
- `ObjectBelonging>Adopted` без `ExtendedConfigurationObject` отмечает корень
  самого расширения. `ClassId` в `InternalInfo` — константы платформы,
  `ObjectId` — новые UUID.
- Без роли оставь `<DefaultRoles/>` пустым и убери `<Role>` из `ChildObjects`.

### `Languages/Русский.xml`

Язык в расширении всегда заимствован: `ExtendedConfigurationObject` — это
`uuid` языка в `Languages/Русский.xml` родителя. Собственный `uuid` языка
расширения — новый.

```xml
<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xen="http://v8.1c.ru/8.3/xcf/enums" xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<Language uuid="<UUID>">
		<InternalInfo/>
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>Русский</Name>
			<Comment/>
			<ExtendedConfigurationObject><UUID языка родителя></ExtendedConfigurationObject>
			<LanguageCode>ru</LanguageCode>
		</Properties>
	</Language>
</MetaDataObject>
```

### `Roles/Расш1_ОсновнаяРоль.xml`

Собственная роль без `Ext/Rights.xml`; права добавляются позже по
[role-edit](../role-edit/SKILL.md).

```xml
<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xen="http://v8.1c.ru/8.3/xcf/enums" xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<Role uuid="<UUID>">
		<Properties>
			<Name>Расш1_ОсновнаяРоль</Name>
			<Synonym/>
			<Comment/>
		</Properties>
	</Role>
</MetaDataObject>
```

## Верификация

Проверка расширения — `unica.check` на корне набора-расширения (`ext` — имя набора типа `EXTENSION` в `v8project.yaml`); валидатор `cfe` выбирается по виду набора, вердикт в `data.status`.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.check",
    "arguments": {
      "at": "ext:Configuration"
    }
  }
}
```
