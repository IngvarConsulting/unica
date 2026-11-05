# Общий реквизит: автоиспользование и разделение

Буквальный XML-экспорт `ibcmd config export` платформы 8.3.27.2074 из
изолированной временной базы после import/apply/check. Основа —
`platform_8_3_27/empty_configuration`; добавлены общий реквизит
`ОбластьДанных` типа String(10), параметр сеанса `ЗначениеОбласти`
того же типа, булев параметр сеанса `ИспользоватьРазделение` и булева
константа `УсловноеРазделение`. Сохранены байты native export, включая
BOM и CRLF; `ConfigDumpInfo.xml` исключён.

Начальное состояние: `AutoUse=DontUse`, `DataSeparation=DontUse`,
`SeparatedDataUse=Independently`, три ссылки пусты;
`UsersSeparation`, `AuthenticationSeparation`,
`ConfigurationExtensionsSeparation` равны `DontUse`.

Фикстура подтверждает приёмку начального XML платформой; поведение
изменений через Unica проверяет отдельный приёмочный сценарий.
Приёмка XML не доказывает изоляцию данных или безопасность runtime.

`borrowed-autouse.xml` — буквальный экспорт заимствованного общего реквизита
из отдельного расширения `Probe` после import/check/apply/export той же
платформой. Платформа сохранила `AutoUse=Use` и минимальные свойства
заимствования; остальные восемь свойств разделения в этом дескрипторе
отсутствуют. Файл расположен вне `src`, чтобы не менять состав начальной
конфигурации.

`borrowed-Configuration.xml` и `borrowed-Role.xml` — соответствующие
дескрипторы расширения и роли `ОсновнаяРоль` из того же экспорта.

`alias-string-{commonattribute,sessionparameter,definedtype}.xml` —
буквальные дескрипторы отдельного принятого платформой разделителя через
`cfg:DefinedType.ТипРазделителя` со значением String(10). Платформа сохраняет
TypeSet в общем реквизите и параметре сеанса. Сочетание такого общего
реквизита с прямым String(10) в параметре сеанса также принято платформой:
совпадение типов определяется раскрытым значением.
`alias-storage-definedtype.xml` происходит из успешного цикла с
DataSeparation=DontUse; при Separate тот же ValueStorage вызывает отказ
обновления базы с SDBL-ошибкой построения хеша.

`characteristic-string-{commonattribute,sessionparameter,chart}.xml` —
буквальные дескрипторы принятого разделителя через
`cfg:Characteristic.Characteristics`. План основан на
`workspace-search-input-settings`, его Properties/Type заменён на String(10).
Платформа сохранила TypeSet и прямое описание типа в плане. Эти дескрипторы
также расположены вне `src` и не меняют начальную конфигурацию.
