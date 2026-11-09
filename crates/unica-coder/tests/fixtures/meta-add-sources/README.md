# meta-add-sources

Пустая конфигурация 8.3.27 (`tests/fixtures/platform_8_3_27/empty_configuration`)
с шестью объектами, записанными `unica.meta.add`: `Catalog.MetaAddSource`,
`Document.MetaAddRegistrar`, `ChartOfAccounts.MetaAddAccounts`,
`ChartOfCalculationTypes.MetaAddCalculationTypes`, `Task.MetaAddTask` и
`CommonModule.MetaAddHandlers` с экспортными процедурами `Run()` и
`Handle(Source, Cancel)`.

Это источники ссылок для тестов `meta_add_surface_tests`: регистратор, план
счетов, обработчики подписки и регламентного задания. Байты — как их записал
писатель: BOM и CRLF.
