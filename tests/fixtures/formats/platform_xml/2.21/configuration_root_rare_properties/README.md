# Семь редких свойств корня конфигурации

Платформенные образцы для [#3159](https://github.com/IngvarConsulting/unica/issues/3159). Конфигуратор 8.5.1.1529 на macOS по очереди загрузил три синтетические конфигурации в расходуемый клон DemoERPUH через `/LoadConfigFromFiles` и выгрузил их через `/DumpConfigToFiles` в пустые каталоги. В `src` сохранены файлы этих выгрузок за исключением служебного `ConfigDumpInfo.xml`. Данные и исходники ERP в образцы не входят. Формат XML — 2.21, режим совместимости — `Version8_3_27`.

| Свойство XML | Режим в образце | Непустое значение в выгрузке | `unica.view` | `Can.props.set` | Задача на поддержку |
| --- | --- | --- | --- | --- | --- |
| `DefaultStyle` | [Обычное приложение](ordinary/src/Configuration.xml), `OrdinaryApplication` | `Style.Probe` | Поля нет | Поля нет | [#3229](https://github.com/IngvarConsulting/unica/issues/3229) |
| `DefaultInterface` | [Обычное приложение](ordinary/src/Configuration.xml), `OrdinaryApplication` | `Interface.ProbeInterface` | Поля нет | Поля нет | [#3229](https://github.com/IngvarConsulting/unica/issues/3229) |
| `DefaultReportAppearanceTemplate` | [Управляемое приложение](desktop/src/Configuration.xml), `ManagedApplication` | `CommonTemplate.ProbeAppearance` типа `DataCompositionAppearanceTemplate` | Поля нет | Поля нет | [#3229](https://github.com/IngvarConsulting/unica/issues/3229) |
| `AdditionalFullTextSearchDictionaries` | [Управляемое приложение](desktop/src/Configuration.xml), `ManagedApplication` | `xr:Item` типа `xr:MDObjectRef` со ссылкой `CommonTemplate.ProbeDictionary` на текстовый макет | Поля нет | Поля нет | [#3230](https://github.com/IngvarConsulting/unica/issues/3230) |
| `StandaloneConfigurationRestrictionRoles` | [Управляемое приложение](desktop/src/Configuration.xml), `ManagedApplication` | `xr:Item` типа `xr:MDObjectRef` со ссылкой `Role.SalesReader` | Поля нет | Поля нет | [#3230](https://github.com/IngvarConsulting/unica/issues/3230) |
| `MobileApplicationURLs` | [Мобильное назначение](mobile/src/Configuration.xml), `MobilePlatformApplication` вместе с `PlatformApplication` | `v8:Value` типа `xs:string` со значением `https://example.org/unica-probe` | Поля нет | Поля нет | [#3231](https://github.com/IngvarConsulting/unica/issues/3231) |
| `AllowedIncomingShareRequestTypes` | [Мобильное назначение](mobile/src/Configuration.xml), `MobilePlatformApplication` вместе с `PlatformApplication` | `v8:Value` типа `app:AllowedIncomingShareRequestType`. MIME `text/plain`, UTI и расширение пусты, `processingVariant=0`, `isCustom=false` | Поля нет | Поля нет | [#3231](https://github.com/IngvarConsulting/unica/issues/3231) |

В таблице указан режим, в котором Конфигуратор сохранил значение. Проверка не запускала клиенты обычного или мобильного приложения, полнотекстовое индексирование и автономное развёртывание.

Для сверки откройте `ordinary`, `desktop` или `mobile` как отдельный проект. Выполните `unica.view {"at":"main:Configuration"}` и `unica.view {"at":"main:Configuration.Can.props.set"}`. На проверенной ревизии `4aa82fa77` оба вызова успешны. В первом результате соответствующих полей нет, во втором их нет в `props.contract.argsSchema.properties.values.properties`. Отдельный предпросмотр `unica.apply` с `props.set` для каждого из семи полей даёт `bad_value` с именем неизвестного поля. Токен исполнения не выдаётся, файлы сохраняются.

Чтобы перепроверить данные платформой, загрузите `src` в свою расходуемую базу и выгрузите конфигурацию в пустой каталог. Сверяйте значение свойства и его XML-тип. Вид и имя объекта, на который указывает ссылка, можно проверить по соседнему дескриптору. Контроль не зависит от UUID новых объектов или идентичности очередной выгрузки побайтно.
