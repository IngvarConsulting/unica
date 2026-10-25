# Дополнительный индекс 8.3.27

AdditionalIndexes.xml — буквальный экспорт 1С 8.3.27.2214 из независимо
созданной временной конфигурации. Справочник CorpusCatalog взят из локальной
native-catalog-descriptor.xml; у индекса ByReferenceWithName ключевое поле Ref
и включённое поле Description. Вход составлен синтетически по форме XML,
затем прошёл DESIGNER /LoadConfigFromFiles, /UpdateDBCfg и /DumpConfigToFiles.
Сохранены BOM, CRLF, UUID и все namespace-объявления результата платформы.

Проверяется допуск подчинённого документа к полной публикации и сохранность
его байтов, а не смысл или эффективность индекса. Версия корня — точный
литерал 2.20. Справка платформы 8.3.27.2214:
platform-syntax-help:platform-guides:form_AdditionalIndexesView.
Схема входного примера сверялась с публичной XML-фикстурой
https://github.com/crimsongoldteam/nkdk/blob/main/packages/core/metadata/commonObjects/additionalIndex/__fixtures__/full.xml;
поставляемый файл порождён нашей конфигурацией и экспортом платформы.
