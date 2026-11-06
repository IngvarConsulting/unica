# Хранилище вариантов отчёта

Синтетическая конфигурация, принятая и повторно выгруженная 8.3.27.2214.
Основа — empty_configuration и публичная синтетическая fixture external_artifacts/erf;
ExternalReport преобразован в обычный Report с ReportObject/ReportManager.
Сохранены СКД MainSchema, четыре управляемые формы, команды, реквизиты,
табличные части, макеты и BSL. StorageA/StorageB — независимые синтетические
SettingsStorage. Исходный VariantsStorage пуст; отдельное SettingsStorage отчёта
назначено StorageB, чтобы проверки различали два свойства. Пользовательские
исходники и базы не использовались.
