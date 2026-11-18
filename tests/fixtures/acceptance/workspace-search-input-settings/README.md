# Поиск и ввод по строке

Синтетические Catalog.Codes и Document.Приход с непустым InputByString, формами выбора, полями, модулями и соседними объектами. Исходник — стенд блокировок данных, выгруженный Designer 8.3.27.2214 (XML 2.20); формы добавлены через публичный MCP и загружены платформой на отдельной копии базы. ConfigDumpInfo.xml исключён, данных ERP нет.

Платформенные значения SearchStringModeOnInputByString — Begin/AnyPart, ChoiceDataGetModeOnInputByString — Directly/Background. Значения Anywhere и SlowlyObtainedSet не проходят импорт. Root ChoiceHistoryOnInput принимает Auto/DontUse; Use не проходит импорт. Root CreateOnInput принимает Auto/Use/DontUse. Это настройки метаданных: работа полнотекстового индекса и клиентского ввода здесь не проверяется.

Все 16 сочетаний двух режимов поиска, полнотекстового поиска при вводе и получения данных выбора сохранены платформой у обоих владельцев. В частности, включённый FullTextSearchOnInputByString сохраняется при FullTextSearch=DontUse.
