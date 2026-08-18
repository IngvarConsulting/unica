<!-- ПОРОЖДАЕТСЯ scripts/arch/registry.py --write-index; руками не правится -->

# Индекс реестра

| Символ | Вид | Статус | Суть | Файл |
| --- | --- | --- | --- | --- |
| `CON.FORMAT.PLATFORM-XML-8-3-27` | контракт | active | Профиль Platform XML 8.3.27 и сохранение байтов | [contracts/CON.FORMAT.PLATFORM-XML-8-3-27.md](contracts/CON.FORMAT.PLATFORM-XML-8-3-27.md) |
| `CON.WIRE.PROTOCOL` | контракт | active | Три ревизии протокола и только реализованные возможности | [contracts/CON.WIRE.PROTOCOL.md](contracts/CON.WIRE.PROTOCOL.md) |
| `CON.WIRE.TOOL-SURFACE` | контракт | active | Ведомость поверхности порождается из бинаря | [contracts/CON.WIRE.TOOL-SURFACE.md](contracts/CON.WIRE.TOOL-SURFACE.md) |
| `DEC.2026-08-18.ADDRESS-GRAMMAR` | решение | active | Адрес — чередование `Вид.Имя` до любой глубины | [decisions/2026-08-18-address-grammar.md](decisions/2026-08-18-address-grammar.md) |
| `DEC.2026-08-18.ARCHITECTURE-RESET` | решение | active | Архитектура описывается заново | [decisions/2026-08-18-architecture-reset.md](decisions/2026-08-18-architecture-reset.md) |
| `DEC.2026-08-18.CARRIED-RULES` | решение | active | Правило переносится, если его проверка жива и предмет не про имена инструментов | [decisions/2026-08-18-carried-rules.md](decisions/2026-08-18-carried-rules.md) |
| `DEC.2026-08-18.EIGHT-ENTRIES` | решение | active | Публичная поверхность — восемь входов | [decisions/2026-08-18-eight-entries.md](decisions/2026-08-18-eight-entries.md) |
| `DEC.2026-08-18.NO-FILE-DSL` | решение | active | Файловый DSL не входит в публичный контракт | [decisions/2026-08-18-no-file-dsl.md](decisions/2026-08-18-no-file-dsl.md) |
| `DEC.2026-08-18.NO-JOB-REGISTRY` | решение | active | Долгая работа — один вызов с прогрессом, а не реестр заданий | [decisions/2026-08-18-no-job-registry.md](decisions/2026-08-18-no-job-registry.md) |
| `DEC.2026-08-18.NODE-OR-DATA` | решение | active | Адрес достаёт до именуемых узлов, множества передаются данными | [decisions/2026-08-18-node-or-data.md](decisions/2026-08-18-node-or-data.md) |
| `DEC.2026-08-18.REGISTRY-SHAPE` | решение | active | Три символических реестра, одна запись — один файл | [decisions/2026-08-18-registry-shape.md](decisions/2026-08-18-registry-shape.md) |
| `DEC.2026-08-18.RESULT-FORM` | решение | active | Результат — компактный JSON со слотовой дисциплиной | [decisions/2026-08-18-result-form.md](decisions/2026-08-18-result-form.md) |
| `INV.APP.CONFIG-SNAPSHOT` | инвариант | active | Конфигурация вызова разрешается снимком | [invariants/INV.APP.CONFIG-SNAPSHOT.md](invariants/INV.APP.CONFIG-SNAPSHOT.md) |
| `INV.APP.DEPENDENCY-DIRECTION` | инвариант | active | Направление зависимостей между слоями закреплено проверкой | [invariants/INV.APP.DEPENDENCY-DIRECTION.md](invariants/INV.APP.DEPENDENCY-DIRECTION.md) |
| `INV.APP.HIDDEN-SERVICES` | инвариант | active | Внутренние сервисы скрыты и привязаны к рабочему пространству | [invariants/INV.APP.HIDDEN-SERVICES.md](invariants/INV.APP.HIDDEN-SERVICES.md) |
| `INV.APP.PROVIDER-NEUTRAL` | инвариант | active | Анализ кода и диагностики не зависят от движка | [invariants/INV.APP.PROVIDER-NEUTRAL.md](invariants/INV.APP.PROVIDER-NEUTRAL.md) |
| `INV.APP.THIN-TRANSPORT` | инвариант | active | Транспорт только отображает протокол на вызовы application | [invariants/INV.APP.THIN-TRANSPORT.md](invariants/INV.APP.THIN-TRANSPORT.md) |
| `INV.CACHE.ORCHESTRATOR-OWNED` | инвариант | active | Состоянием рабочего пространства владеет оркестратор | [invariants/INV.CACHE.ORCHESTRATOR-OWNED.md](invariants/INV.CACHE.ORCHESTRATOR-OWNED.md) |
| `INV.CACHE.PREVIEW-WRITES-NOTHING` | инвариант | active | Предпросмотр не оставляет следов | [invariants/INV.CACHE.PREVIEW-WRITES-NOTHING.md](invariants/INV.CACHE.PREVIEW-WRITES-NOTHING.md) |
| `INV.CACHE.STATE-OUTSIDE-SOURCE` | инвариант | active | Состояние поставщика лежит вне индексируемого источника | [invariants/INV.CACHE.STATE-OUTSIDE-SOURCE.md](invariants/INV.CACHE.STATE-OUTSIDE-SOURCE.md) |
| `INV.CI.TAG-ONLY-PUBLISH` | инвариант | active | Публикация происходит только по тегу и через один шлюз | [invariants/INV.CI.TAG-ONLY-PUBLISH.md](invariants/INV.CI.TAG-ONLY-PUBLISH.md) |
| `INV.DOC.ARCHIVE-FROZEN` | инвариант | active | Замороженный слой не читается и не правится | [invariants/INV.DOC.ARCHIVE-FROZEN.md](invariants/INV.DOC.ARCHIVE-FROZEN.md) |
| `INV.DOC.SUPERPOWERS-BOUNDARY` | инвариант | active | Формы superpowers не входят в реестр | [invariants/INV.DOC.SUPERPOWERS-BOUNDARY.md](invariants/INV.DOC.SUPERPOWERS-BOUNDARY.md) |
| `INV.HOST.KNOWLEDGE-BEHIND-FACADE` | инвариант | active | Знание о хосте живёт за host-фасадом | [invariants/INV.HOST.KNOWLEDGE-BEHIND-FACADE.md](invariants/INV.HOST.KNOWLEDGE-BEHIND-FACADE.md) |
| `INV.PKG.ATTRIBUTION` | инвариант | active | Атрибуция остаётся полной | [invariants/INV.PKG.ATTRIBUTION.md](invariants/INV.PKG.ATTRIBUTION.md) |
| `INV.PKG.THIN-PACKAGE` | инвариант | active | Публичный пакет тонкий | [invariants/INV.PKG.THIN-PACKAGE.md](invariants/INV.PKG.THIN-PACKAGE.md) |
| `INV.PKG.TWO-HOSTS-ONE-TREE` | инвариант | active | Один каталог плагина обслуживает двух хостов | [invariants/INV.PKG.TWO-HOSTS-ONE-TREE.md](invariants/INV.PKG.TWO-HOSTS-ONE-TREE.md) |
| `INV.PKG.VERIFIED-ATOMIC-INSTALL` | инвариант | active | Runtime проверяется контрольной суммой и ставится атомарно | [invariants/INV.PKG.VERIFIED-ATOMIC-INSTALL.md](invariants/INV.PKG.VERIFIED-ATOMIC-INSTALL.md) |
| `INV.PLATFORM.NO-ORPHANS` | инвариант | active | Дочерние процессы удерживаются целыми деревьями | [invariants/INV.PLATFORM.NO-ORPHANS.md](invariants/INV.PLATFORM.NO-ORPHANS.md) |
| `INV.PLATFORM.OS-BEHIND-FACADE` | инвариант | active | Зависящий от ОС код живёт за платформенными фасадами | [invariants/INV.PLATFORM.OS-BEHIND-FACADE.md](invariants/INV.PLATFORM.OS-BEHIND-FACADE.md) |
| `INV.PRODUCT.NO-FORMAT-MIGRATION` | инвариант | active | Unica не мигрирует формат выгрузки | [invariants/INV.PRODUCT.NO-FORMAT-MIGRATION.md](invariants/INV.PRODUCT.NO-FORMAT-MIGRATION.md) |
| `INV.REGISTRY.SYMBOL-MATCHES-PATH` | инвариант | active | Символ записи выводится из её пути | [invariants/INV.REGISTRY.SYMBOL-MATCHES-PATH.md](invariants/INV.REGISTRY.SYMBOL-MATCHES-PATH.md) |
| `INV.SOURCE.ATOMIC-PUBLISH` | инвариант | active | Мутация источника публикуется атомарно или не публикуется | [invariants/INV.SOURCE.ATOMIC-PUBLISH.md](invariants/INV.SOURCE.ATOMIC-PUBLISH.md) |
| `INV.SOURCE.EXACT-VERSION` | инвариант | active | Версия формата — точный литерал, а корень — точный QName | [invariants/INV.SOURCE.EXACT-VERSION.md](invariants/INV.SOURCE.EXACT-VERSION.md) |
| `INV.SOURCE.FORMAT-PER-SET` | инвариант | active | Формат — свойство набора исходников | [invariants/INV.SOURCE.FORMAT-PER-SET.md](invariants/INV.SOURCE.FORMAT-PER-SET.md) |
| `INV.SOURCE.OBSERVED-BYTES` | инвариант | active | Байты источника наблюдаются, а не назначаются | [invariants/INV.SOURCE.OBSERVED-BYTES.md](invariants/INV.SOURCE.OBSERVED-BYTES.md) |
| `INV.SOURCE.SINGLE-RESOLVED-ROOT` | инвариант | active | Корень исходников выбирается детерминированно и один раз | [invariants/INV.SOURCE.SINGLE-RESOLVED-ROOT.md](invariants/INV.SOURCE.SINGLE-RESOLVED-ROOT.md) |
| `INV.SOURCE.SNAPSHOT-BINDING` | инвариант | active | Ресурс действует внутри своего снимка и своей роли | [invariants/INV.SOURCE.SNAPSHOT-BINDING.md](invariants/INV.SOURCE.SNAPSHOT-BINDING.md) |
| `INV.SOURCE.WRITE-CONTAINMENT` | инвариант | active | Запись не выходит за корень рабочего пространства | [invariants/INV.SOURCE.WRITE-CONTAINMENT.md](invariants/INV.SOURCE.WRITE-CONTAINMENT.md) |
| `INV.SURFACE.ACCEPTANCE-UNCHANGED` | инвариант | active | Сужение публикации не сужает приём | [invariants/INV.SURFACE.ACCEPTANCE-UNCHANGED.md](invariants/INV.SURFACE.ACCEPTANCE-UNCHANGED.md) |
| `INV.SURFACE.NAMESPACE` | инвариант | active | Публичные инструменты живут в пространстве unica | [invariants/INV.SURFACE.NAMESPACE.md](invariants/INV.SURFACE.NAMESPACE.md) |
| `INV.SURFACE.PUBLISHED-ARGS-ARE-READ` | инвариант | active | Публикуется то, что обработчик читает | [invariants/INV.SURFACE.PUBLISHED-ARGS-ARE-READ.md](invariants/INV.SURFACE.PUBLISHED-ARGS-ARE-READ.md) |
| `INV.WIRE.BOUNDED-ADMISSION` | инвариант | active | Приём вызовов ограничен, отмена кооперативна | [invariants/INV.WIRE.BOUNDED-ADMISSION.md](invariants/INV.WIRE.BOUNDED-ADMISSION.md) |
| `INV.WIRE.DATA-DRIVEN-SCHEMA` | инвариант | active | Контракты инструментов заданы данными | [invariants/INV.WIRE.DATA-DRIVEN-SCHEMA.md](invariants/INV.WIRE.DATA-DRIVEN-SCHEMA.md) |
| `INV.WIRE.ONE-SERVER` | инвариант | active | Модель видит один сервер и ни одного движка | [invariants/INV.WIRE.ONE-SERVER.md](invariants/INV.WIRE.ONE-SERVER.md) |
| `INV.WIRE.PREVIEW-IS-MUTATION-ONLY` | инвариант | active | Предпросмотр принадлежит мутации | [invariants/INV.WIRE.PREVIEW-IS-MUTATION-ONLY.md](invariants/INV.WIRE.PREVIEW-IS-MUTATION-ONLY.md) |
