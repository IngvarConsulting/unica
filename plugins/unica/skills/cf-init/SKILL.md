---
name: cf-init
description: Создать пустую конфигурацию 1С (scaffold XML-исходников). Используй когда нужно начать новую конфигурацию с нуля
argument-hint: <Имя> [<каталог набора>]
allowed-tools:
  - Bash
  - Read
  - Write
  - Glob
---

# /cf-init — Создание пустой конфигурации 1С

## MCP routing

- **Канонической операции создания конфигурации на поверхности нет.** Словарь
  `unica.apply` правит существующий узел: `object.create` заводит объект
  метаданных **внутри** конфигурации, но самого корня `Configuration.xml` не
  создаёт.
- Не зови внутренние адаптеры напрямую: они спрятаны за MCP `unica`.
- Готовность проверяет `unica.check`, результат читает `unica.view` по адресу.

Поэтому корень новой конфигурации записывается файловыми средствами по
образцам ниже — тем же порядком, каким заводится `v8project.yaml`
(`DEC.2026-09-09.PROJECT-CONFIG-IS-HANDWRITTEN`). Нужна именно операция
создания корня — это пробел контракта Unica MCP; сообщи о нём, а не подменяй
его правкой чужой конфигурации.

## Порядок работы

1. Выбери пустой каталог набора, например `src`. Существующий
   `Configuration.xml` не перезаписывай: правку существующего корня ведёт
   [cf-edit](../cf-edit/SKILL.md).
2. Запиши три файла по образцам ниже: `Configuration.xml`,
   `Languages/Русский.xml` и `Ext/ClientApplicationInterface.xml`. Каждый
   `<UUID>` замени своим новым UUID (`uuidgen`), одинаковых быть не должно.
   `ConfigDumpInfo.xml` не создавай: это служебный файл конкретной базы, а не
   исходник.
3. Объяви каталог набором в `v8project.yaml`, не затирая остальные поля:

   ```yaml
   source-set:
     - name: main
       type: CONFIGURATION
       path: src
   ```

   Полный минимальный файл и `v8project.local.yaml` описаны в
   [v8project](../../references/tooling/v8project.md).
4. Убедись в допуске набора: `unica.check {}`.
5. Прочитай корень: `unica.view {at: "main:Configuration"}`.
6. Объекты заводит `unica.apply` (`object.create`, `props.set`) по
   [meta-add](../meta-add/SKILL.md) и [cf-edit](../cf-edit/SKILL.md).

## Формат корня

Образцы сняты с рабочей заготовки, выгруженной платформой 8.3.27 в формате 2.20 (`tests/fixtures/platform_8_3_27/template-new-xml/seed/`), без её объектов и с новыми UUID.
Полное описание свойств, их порядка и состава — в
[спецификации конфигурации](../../references/specs/1c-configuration-spec.md):
§2 — `Configuration.xml`, §4.2 — `ClientApplicationInterface.xml`,
§5 — языки. Файлы метаданных пиши в UTF-8; платформа выгружает их с BOM.

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
			<Name>МояКонфигурация</Name>
			<Synonym>
				<v8:item>
					<v8:lang>ru</v8:lang>
					<v8:content>Моя конфигурация</v8:content>
				</v8:item>
			</Synonym>
			<Comment/>
			<NamePrefix/>
			<ConfigurationExtensionCompatibilityMode>Version8_3_27</ConfigurationExtensionCompatibilityMode>
			<DefaultRunMode>ManagedApplication</DefaultRunMode>
			<UsePurposes>
				<v8:Value xsi:type="app:ApplicationUsePurpose">PlatformApplication</v8:Value>
			</UsePurposes>
			<ScriptVariant>Russian</ScriptVariant>
			<DefaultRoles/>
			<Vendor/>
			<Version/>
			<UpdateCatalogAddress/>
			<IncludeHelpInContents>false</IncludeHelpInContents>
			<UseManagedFormInOrdinaryApplication>false</UseManagedFormInOrdinaryApplication>
			<UseOrdinaryFormInManagedApplication>false</UseOrdinaryFormInManagedApplication>
			<AdditionalFullTextSearchDictionaries/>
			<CommonSettingsStorage/>
			<ReportsUserSettingsStorage/>
			<ReportsVariantsStorage/>
			<FormDataSettingsStorage/>
			<DynamicListsUserSettingsStorage/>
			<URLExternalDataStorage/>
			<Content/>
			<DefaultReportForm/>
			<DefaultReportVariantForm/>
			<DefaultReportSettingsForm/>
			<DefaultReportAppearanceTemplate/>
			<DefaultDynamicListSettingsForm/>
			<DefaultSearchForm/>
			<DefaultDataHistoryChangeHistoryForm/>
			<DefaultDataHistoryVersionDataForm/>
			<DefaultDataHistoryVersionDifferencesForm/>
			<DefaultCollaborationSystemUsersChoiceForm/>
			<RequiredMobileApplicationPermissions/>
			<UsedMobileApplicationFunctionalities>
				<app:functionality><app:functionality>Biometrics</app:functionality><app:use>true</app:use></app:functionality>
				<app:functionality><app:functionality>Location</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>BackgroundLocation</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>BluetoothPrinters</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>WiFiPrinters</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>Contacts</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>Calendars</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>PushNotifications</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>LocalNotifications</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>InAppPurchases</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>PersonalComputerFileExchange</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>Ads</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>NumberDialing</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>CallProcessing</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>CallLog</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>AutoSendSMS</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>ReceiveSMS</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>SMSLog</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>Camera</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>Microphone</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>MusicLibrary</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>PictureAndVideoLibraries</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>AudioPlaybackAndVibration</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>BackgroundAudioPlaybackAndVibration</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>InstallPackages</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>OSBackup</app:functionality><app:use>true</app:use></app:functionality>
				<app:functionality><app:functionality>ApplicationUsageStatistics</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>BarcodeScanning</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>BackgroundAudioRecording</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>AllFilesAccess</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>Videoconferences</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>NFC</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>DocumentScanning</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>SpeechToText</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>Geofences</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>IncomingShareRequests</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>AllIncomingShareRequestsTypesProcessing</app:functionality><app:use>false</app:use></app:functionality>
				<app:functionality><app:functionality>TextToSpeech</app:functionality><app:use>false</app:use></app:functionality>
			</UsedMobileApplicationFunctionalities>
			<StandaloneConfigurationRestrictionRoles/>
			<MobileApplicationURLs/>
			<AllowedIncomingShareRequestTypes/>
			<MainClientApplicationWindowMode>Normal</MainClientApplicationWindowMode>
			<DefaultInterface/>
			<DefaultStyle/>
			<DefaultLanguage>Language.Русский</DefaultLanguage>
			<BriefInformation/>
			<DetailedInformation/>
			<Copyright/>
			<VendorInformationAddress/>
			<ConfigurationInformationAddress/>
			<DataLockControlMode>Managed</DataLockControlMode>
			<ObjectAutonumerationMode>NotAutoFree</ObjectAutonumerationMode>
			<ModalityUseMode>DontUse</ModalityUseMode>
			<SynchronousPlatformExtensionAndAddInCallUseMode>DontUse</SynchronousPlatformExtensionAndAddInCallUseMode>
			<InterfaceCompatibilityMode>TaxiEnableVersion8_2</InterfaceCompatibilityMode>
			<DatabaseTablespacesUseMode>DontUse</DatabaseTablespacesUseMode>
			<CompatibilityMode>Version8_3_27</CompatibilityMode>
			<DefaultConstantsForm/>
		</Properties>
		<ChildObjects>
			<Language>Русский</Language>
		</ChildObjects>
	</Configuration>
</MetaDataObject>
```

Что подставить:

- `Name` — идентификатор 1С; `Synonym` — представление.
- `Vendor` и `Version` заполняются при необходимости:
  `<Vendor>Фирма 1С</Vendor>`, `<Version>1.0.0.1</Version>`.
- `CompatibilityMode` в образце — `Version8_3_27`. Другой режим совместимости
  задаётся там же, например `<CompatibilityMode>Version8_3_24</CompatibilityMode>`;
  `ConfigurationExtensionCompatibilityMode` меняй согласованно с ним (не выше),
  прочие режимы описаны в §2.3.
- `ClassId` в `InternalInfo` — константы платформы (§2.2), их не меняй;
  `ObjectId` — новые UUID.
- Порядок свойств фиксирован (§2.3). `UsedMobileApplicationFunctionalities`
  записан по строке на возможность; так и оставь: состав и значения взяты из
  выгрузки.
- `DefaultLanguage` ссылается на язык из `ChildObjects` и файла
  `Languages/<Имя>.xml`.

### `Languages/Русский.xml`

```xml
<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xen="http://v8.1c.ru/8.3/xcf/enums" xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<Language uuid="<UUID>">
		<Properties>
			<Name>Русский</Name>
			<Synonym>
				<v8:item>
					<v8:lang>ru</v8:lang>
					<v8:content>Русский</v8:content>
				</v8:item>
			</Synonym>
			<Comment/>
			<LanguageCode>ru</LanguageCode>
		</Properties>
	</Language>
</MetaDataObject>
```

### `Ext/ClientApplicationInterface.xml`

Раскладка панелей по умолчанию: панель открытых сверху, панель разделов
слева. Значения `<uuid>` и `panelDef` — фиксированные идентификаторы панелей
платформы (§4.2), `id` у `<panel>` — новые UUID.

```xml
<?xml version="1.0" encoding="UTF-8"?>
<ClientApplicationInterface xmlns="http://v8.1c.ru/8.2/managed-application/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="InterfaceLayouter">
	<top>
		<panel id="<UUID>">
			<uuid>cbab57f2-a0f3-4f0a-89ea-4cb19570ab75</uuid>
		</panel>
	</top>
	<left>
		<panel id="<UUID>">
			<uuid>b553047f-c9aa-4157-978d-448ecad24248</uuid>
		</panel>
	</left>
	<panelDef id="b553047f-c9aa-4157-978d-448ecad24248"/>
	<panelDef id="13322b22-3960-4d68-93a6-fe2dd7f28ca3"/>
	<panelDef id="c933ac92-92cd-459d-81cc-e0c8a83ced99"/>
	<panelDef id="cbab57f2-a0f3-4f0a-89ea-4cb19570ab75"/>
	<panelDef id="b2735bd3-d822-4430-ba59-c9e869693b24"/>
</ClientApplicationInterface>
```

## Верификация

После записи файлов проверь набор через MCP `unica` read-only инструменты.

### Проверить созданную структуру

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.view",
    "arguments": {
      "at": "main:Configuration"
    }
  }
}
```

### Валидировать XML конфигурации

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.check",
    "arguments": {
      "at": "main:Configuration"
    }
  }
}
```
