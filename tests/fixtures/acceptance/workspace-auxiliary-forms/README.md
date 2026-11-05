# Вспомогательные формы владельцев

Изолированная конфигурация XML 2.20, выгруженная платформой 8.3.27.2074.
Сценарий `S-1671-auxiliary-forms` назначает формы A, заменяет их формами B
и повторяет эквивалентное назначение через публичные `view` и `apply`.
Каждая запись исполняет сохранённый `executionToken`; preview и no-op
проверяются по исходным байтам. Основные назначения заполнены заранее.

| Владелец | Ключи `form.set.values` | Формы |
| --- | --- | --- |
| Catalog.Support | auxiliaryFolderForm, auxiliaryFolderChoiceForm | FolderA/B, FolderChoiceA/B |
| InformationRegister.Facts | auxiliaryRecordForm | RecordA/B |
| Report.CorpusReport | auxiliaryForm, auxiliarySettingsForm | MainA/B, SettingsA/B |
| DataProcessor.Processor | auxiliaryForm | MainA/B |
| DocumentJournal.Entries | auxiliaryForm | ListA/B |

Собственная форма должна быть зарегистрирована у владельца и иметь
существующий дескриптор `Form` с совпадающим именем. План удерживает байты
этого дескриптора: его изменение или удаление после preview требует нового
плана. Назначение не переписывает тело или модуль формы и не ограничивает
собственные формы управляемым вариантом.

Для платформенной проверки сохраняйте `src` после назначения A и замены B.
Каждый снимок импортируйте в отдельную пустую базу командой
`ibcmd infobase create --import=<src> --apply --force` с изолированными
параметрами подключения из `scripts/dev/verify-8-3-27-platform.py`.
Выполните `ibcmd config check --force`, затем `ibcmd config export`.
Сравнивайте все семь непустых ссылок и семантику дескрипторов владельцев
с соответствующим снимком; импорт только исходной фикстуры недостаточен.
