# Режим блокировок объектов

Основа — нативная фикстура workspace-register-dimensions. Через MCP добавлены Catalog, Constant, ChartOfCharacteristicTypes, Task, BusinessProcess и ExchangePlan, реквизит Catalog.Key. Подготовлены непустые DataLockFields и модуль Catalog. Весь набор импортирован и выгружен Designer 8.3.27.2214 (XML 2.20) на отдельной копии базы. Режим корня Managed, всех 13 владельцев Automatic. ConfigDumpInfo.xml исключён; данных ERP нет.

Платформа сохранила все 78 сочетаний: три режима корня (Automatic/Managed/AutomaticAndManaged) × два объектных режима × 13 видов. Дополнительного запрета по режиму корня нет; фактические транзакции/блокировки не проверяются.
