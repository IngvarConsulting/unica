# Жизненный цикл членов метаданных

Source-фикстура для `tabularSection.*`, `dimension.*`, `resource.*`,
`enumValue.*` и `column.*`: добавление, адресное переименование и удаление
через preview и executionToken. Сценарии проверяют постобразы, UUID,
сохранность остальных байтов владельца и соседних файлов.

Основа — `workspace-metadata-commands`: Items, язык, модули команд
и посторонний `Refresh/Ext/unrelated.txt` скопированы оттуда.
Табличная часть Представления со вложенными реквизитами взята из
`workspace/src/Catalogs/Валюты.xml`; имена её порождённых типов заменены
с Валюты на Items. Регистр CorpusRegister —
`platform_8_3_27/incomplete_borrowed/parent-information-register.xml`
с измерением Идентификатор из
`workspace/src/InformationRegisters/АдминистративнаяИерархия.xml`.
Его переводы строк приведены к CRLF; исходный ресурс Value сохранён.
Перечисление — `workspace/src/Enums/LanguageAware.xml` со значением Yes
из `workspace-metadata-values/src/Enums/Flags.xml` и полным значением Ошибка
из `workspace/src/Enums/ВажностьПроблемыУчета.xml`. Журнал скопирован из
`workspace/src/DocumentJournals/ДокументыОрганизации.xml`.

Выдержка не содержит документ из ссылок колонки журнала. Эти ссылки
служат неизменяемым содержимым, сценарии их не разрешают. Импорт всей
конфигурации в 1С этими source-сценариями не заявляется.
