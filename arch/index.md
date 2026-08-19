<!-- ПОРОЖДАЕТСЯ scripts/arch/registry.py --write-index; руками не правится -->

# Индекс реестра

| Символ | Вид | Статус | Построено | Суть | Файл |
| --- | --- | --- | --- | --- | --- |
| `CTR.FORMAT.PLATFORM-XML-8-3-27` | контракт | active |  | Профиль Platform XML 8.3.27 и сохранение байтов | [contracts/CTR.FORMAT.PLATFORM-XML-8-3-27.md](contracts/CTR.FORMAT.PLATFORM-XML-8-3-27.md) |
| `CTR.WIRE.PROTOCOL` | контракт | active |  | Три ревизии протокола и только реализованные возможности | [contracts/CTR.WIRE.PROTOCOL.md](contracts/CTR.WIRE.PROTOCOL.md) |
| `CTR.WIRE.TOOL-SURFACE` | контракт | active |  | Ведомость поверхности порождается из бинаря | [contracts/CTR.WIRE.TOOL-SURFACE.md](contracts/CTR.WIRE.TOOL-SURFACE.md) |
| `DEC.2026-08-18.ADDRESS-GRAMMAR` | решение | active | нет | Адрес — чередование `Вид.Имя` до любой глубины | [decisions/2026-08-18-address-grammar.md](decisions/2026-08-18-address-grammar.md) |
| `DEC.2026-08-18.ARCHITECTURE-RESET` | решение | active | да | Архитектура описывается заново | [decisions/2026-08-18-architecture-reset.md](decisions/2026-08-18-architecture-reset.md) |
| `DEC.2026-08-18.CARRIED-RULES` | решение | active | да | Правило переносится, если его проверка жива и предмет не про имена инструментов | [decisions/2026-08-18-carried-rules.md](decisions/2026-08-18-carried-rules.md) |
| `DEC.2026-08-18.CURSOR-CARRIES-REVISION` | решение | active | нет | Продолжение чтения несёт ревизию и отказывает при расхождении | [decisions/2026-08-18-cursor-carries-revision.md](decisions/2026-08-18-cursor-carries-revision.md) |
| `DEC.2026-08-18.DATA-LEAVES-DOCS` | решение | active | нет | Порождаемые данные не лежат в каталогах документации | [decisions/2026-08-18-data-leaves-docs.md](decisions/2026-08-18-data-leaves-docs.md) |
| `DEC.2026-08-18.DOCS-NOT-READ` | решение | active | нет | Тест не читает документацию | [decisions/2026-08-18-docs-not-read.md](decisions/2026-08-18-docs-not-read.md) |
| `DEC.2026-08-18.EIGHT-ENTRIES` | решение | superseded | нет | Публичная поверхность — восемь входов | [decisions/2026-08-18-eight-entries.md](decisions/2026-08-18-eight-entries.md) |
| `DEC.2026-08-18.FAILURE-NAMES-THE-FILE` | решение | active | нет | Из Unica всегда есть выход к файлам | [decisions/2026-08-18-failure-names-the-file.md](decisions/2026-08-18-failure-names-the-file.md) |
| `DEC.2026-08-18.FILE-NODE-CONTENT` | решение | active | нет | Узел файловой природы отдаёт содержимое, а не свойства | [decisions/2026-08-18-file-node-content.md](decisions/2026-08-18-file-node-content.md) |
| `DEC.2026-08-18.MODULE-PROJECTIONS` | решение | active | нет | Модуль читается проекцией по глубине и отбору, а не окном по байтам | [decisions/2026-08-18-module-projections.md](decisions/2026-08-18-module-projections.md) |
| `DEC.2026-08-18.NO-FILE-DSL` | решение | active | нет | Файловый DSL не входит в публичный контракт | [decisions/2026-08-18-no-file-dsl.md](decisions/2026-08-18-no-file-dsl.md) |
| `DEC.2026-08-18.NO-JOB-REGISTRY` | решение | active | нет | Долгая работа — один вызов с прогрессом, а не реестр заданий | [decisions/2026-08-18-no-job-registry.md](decisions/2026-08-18-no-job-registry.md) |
| `DEC.2026-08-18.NODE-OR-DATA` | решение | active | нет | Адрес достаёт до именуемых узлов, множества передаются данными | [decisions/2026-08-18-node-or-data.md](decisions/2026-08-18-node-or-data.md) |
| `DEC.2026-08-18.OPAQUE-IS-VISIBLE` | решение | active | нет | Непонятое содержимое видно и названо, а не отсутствует | [decisions/2026-08-18-opaque-is-visible.md](decisions/2026-08-18-opaque-is-visible.md) |
| `DEC.2026-08-18.REGISTRY-SHAPE` | решение | active | да | Три символических реестра, одна запись — один файл | [decisions/2026-08-18-registry-shape.md](decisions/2026-08-18-registry-shape.md) |
| `DEC.2026-08-18.RESOURCE-FOR-DEFERRED-BYTES` | решение | active | нет | Ресурс предлагается для байтов, которых, скорее всего, не попросят | [decisions/2026-08-18-resource-for-deferred-bytes.md](decisions/2026-08-18-resource-for-deferred-bytes.md) |
| `DEC.2026-08-18.RESULT-FORM` | решение | active | нет | Результат — компактный JSON со слотовой дисциплиной | [decisions/2026-08-18-result-form.md](decisions/2026-08-18-result-form.md) |
| `DEC.2026-08-19.ARCHIVE-DRIFT-IS-RECORDED` | решение | active | да | Заморозка требует не неподвижности, а объяснённого расхождения | [decisions/2026-08-19-archive-drift-is-recorded.md](decisions/2026-08-19-archive-drift-is-recorded.md) |
| `DEC.2026-08-19.DECISIONS-FORM-INSIDE` | решение | active | да | Решение формируется в реестре, а не в трекере | [decisions/2026-08-19-decisions-form-inside.md](decisions/2026-08-19-decisions-form-inside.md) |
| `DEC.2026-08-19.ENTRY-APPLY` | решение | active | нет | `apply` — один писатель на всё дерево | [decisions/2026-08-19-entry-apply.md](decisions/2026-08-19-entry-apply.md) |
| `DEC.2026-08-19.ENTRY-CHECK` | решение | active | нет | `check` — здоровье пространства и годность узла | [decisions/2026-08-19-entry-check.md](decisions/2026-08-19-entry-check.md) |
| `DEC.2026-08-19.ENTRY-DIFF` | решение | active | нет | `diff` — сравнение как отдельный вопрос | [decisions/2026-08-19-entry-diff.md](decisions/2026-08-19-entry-diff.md) |
| `DEC.2026-08-19.ENTRY-DOCS` | решение | active | нет | `docs` — справка платформы и стандарты | [decisions/2026-08-19-entry-docs.md](decisions/2026-08-19-entry-docs.md) |
| `DEC.2026-08-19.ENTRY-FIND` | решение | active | нет | `find` — от чего угодно к адресу | [decisions/2026-08-19-entry-find.md](decisions/2026-08-19-entry-find.md) |
| `DEC.2026-08-19.ENTRY-RUN` | решение | active | нет | `run` — работа платформы | [decisions/2026-08-19-entry-run.md](decisions/2026-08-19-entry-run.md) |
| `DEC.2026-08-19.ENTRY-SEARCH` | решение | active | нет | `search` — по содержимому, а не по имени | [decisions/2026-08-19-entry-search.md](decisions/2026-08-19-entry-search.md) |
| `DEC.2026-08-19.ENTRY-VIEW` | решение | active | нет | `view` — один читатель на всё дерево | [decisions/2026-08-19-entry-view.md](decisions/2026-08-19-entry-view.md) |
| `DEC.2026-08-19.NO-SELF-GLOSS` | решение | active | да | Правило говорит о предмете, а не о себе | [decisions/2026-08-19-no-self-gloss.md](decisions/2026-08-19-no-self-gloss.md) |
| `DEC.2026-08-19.REALIZATION-AXIS` | решение | active | да | Решённое и построенное — разные оси | [decisions/2026-08-19-realization-axis.md](decisions/2026-08-19-realization-axis.md) |
| `DEC.2026-08-19.SURFACE-BY-QUESTION` | решение | active | нет | Вход отвечает на вопрос, а не обслуживает предмет | [decisions/2026-08-19-surface-by-question.md](decisions/2026-08-19-surface-by-question.md) |
| `INV.APP.CONFIG-SNAPSHOT` | инвариант | active |  | Конфигурация вызова разрешается снимком | [invariants/INV.APP.CONFIG-SNAPSHOT.md](invariants/INV.APP.CONFIG-SNAPSHOT.md) |
| `INV.APP.DEPENDENCY-DIRECTION` | инвариант | active |  | Направление зависимостей между слоями закреплено проверкой | [invariants/INV.APP.DEPENDENCY-DIRECTION.md](invariants/INV.APP.DEPENDENCY-DIRECTION.md) |
| `INV.APP.HIDDEN-SERVICES` | инвариант | active |  | Внутренние сервисы скрыты и привязаны к рабочему пространству | [invariants/INV.APP.HIDDEN-SERVICES.md](invariants/INV.APP.HIDDEN-SERVICES.md) |
| `INV.APP.PROVIDER-NEUTRAL` | инвариант | active |  | Анализ кода и диагностики не зависят от движка | [invariants/INV.APP.PROVIDER-NEUTRAL.md](invariants/INV.APP.PROVIDER-NEUTRAL.md) |
| `INV.APP.THIN-TRANSPORT` | инвариант | active |  | Транспорт только отображает протокол на вызовы application | [invariants/INV.APP.THIN-TRANSPORT.md](invariants/INV.APP.THIN-TRANSPORT.md) |
| `INV.CACHE.ORCHESTRATOR-OWNED` | инвариант | active |  | Состоянием рабочего пространства владеет оркестратор | [invariants/INV.CACHE.ORCHESTRATOR-OWNED.md](invariants/INV.CACHE.ORCHESTRATOR-OWNED.md) |
| `INV.CACHE.PREVIEW-WRITES-NOTHING` | инвариант | active |  | Предпросмотр не оставляет следов | [invariants/INV.CACHE.PREVIEW-WRITES-NOTHING.md](invariants/INV.CACHE.PREVIEW-WRITES-NOTHING.md) |
| `INV.CACHE.STATE-OUTSIDE-SOURCE` | инвариант | active |  | Состояние поставщика лежит вне индексируемого источника | [invariants/INV.CACHE.STATE-OUTSIDE-SOURCE.md](invariants/INV.CACHE.STATE-OUTSIDE-SOURCE.md) |
| `INV.CI.TAG-ONLY-PUBLISH` | инвариант | active |  | Публикация происходит только по тегу и через один шлюз | [invariants/INV.CI.TAG-ONLY-PUBLISH.md](invariants/INV.CI.TAG-ONLY-PUBLISH.md) |
| `INV.DOC.ARCHIVE-FROZEN` | инвариант | active |  | Расхождение замороженного слоя объяснено | [invariants/INV.DOC.ARCHIVE-FROZEN.md](invariants/INV.DOC.ARCHIVE-FROZEN.md) |
| `INV.DOC.SUPERPOWERS-BOUNDARY` | инвариант | active |  | Формы superpowers не входят в реестр | [invariants/INV.DOC.SUPERPOWERS-BOUNDARY.md](invariants/INV.DOC.SUPERPOWERS-BOUNDARY.md) |
| `INV.HOST.KNOWLEDGE-BEHIND-FACADE` | инвариант | active |  | Знание о хосте живёт за host-фасадом | [invariants/INV.HOST.KNOWLEDGE-BEHIND-FACADE.md](invariants/INV.HOST.KNOWLEDGE-BEHIND-FACADE.md) |
| `INV.PKG.ATTRIBUTION` | инвариант | active |  | Атрибуция остаётся полной | [invariants/INV.PKG.ATTRIBUTION.md](invariants/INV.PKG.ATTRIBUTION.md) |
| `INV.PKG.THIN-PACKAGE` | инвариант | active |  | Публичный пакет тонкий | [invariants/INV.PKG.THIN-PACKAGE.md](invariants/INV.PKG.THIN-PACKAGE.md) |
| `INV.PKG.TWO-HOSTS-ONE-TREE` | инвариант | active |  | Один каталог плагина обслуживает двух хостов | [invariants/INV.PKG.TWO-HOSTS-ONE-TREE.md](invariants/INV.PKG.TWO-HOSTS-ONE-TREE.md) |
| `INV.PKG.VERIFIED-ATOMIC-INSTALL` | инвариант | active |  | Runtime проверяется контрольной суммой и ставится атомарно | [invariants/INV.PKG.VERIFIED-ATOMIC-INSTALL.md](invariants/INV.PKG.VERIFIED-ATOMIC-INSTALL.md) |
| `INV.PLATFORM.NO-ORPHANS` | инвариант | active |  | Дочерние процессы удерживаются целыми деревьями | [invariants/INV.PLATFORM.NO-ORPHANS.md](invariants/INV.PLATFORM.NO-ORPHANS.md) |
| `INV.PLATFORM.OS-BEHIND-FACADE` | инвариант | active |  | Зависящий от ОС код живёт за платформенными фасадами | [invariants/INV.PLATFORM.OS-BEHIND-FACADE.md](invariants/INV.PLATFORM.OS-BEHIND-FACADE.md) |
| `INV.PRODUCT.NO-FORMAT-MIGRATION` | инвариант | active |  | Unica не мигрирует формат выгрузки | [invariants/INV.PRODUCT.NO-FORMAT-MIGRATION.md](invariants/INV.PRODUCT.NO-FORMAT-MIGRATION.md) |
| `INV.REGISTRY.CHECK-EXISTS` | инвариант | active |  | Запись, несущая проверку, называет существующую цель | [invariants/INV.REGISTRY.CHECK-EXISTS.md](invariants/INV.REGISTRY.CHECK-EXISTS.md) |
| `INV.REGISTRY.NO-SELF-GLOSS` | инвариант | active |  | Инвариант и контракт не толкуют свои поля | [invariants/INV.REGISTRY.NO-SELF-GLOSS.md](invariants/INV.REGISTRY.NO-SELF-GLOSS.md) |
| `INV.REGISTRY.NO-TRACKER-LINKS` | инвариант | active |  | Реестр не ссылается на трекер | [invariants/INV.REGISTRY.NO-TRACKER-LINKS.md](invariants/INV.REGISTRY.NO-TRACKER-LINKS.md) |
| `INV.REGISTRY.REALIZATION-NAMED` | инвариант | active |  | Решение объявляет, построено ли оно | [invariants/INV.REGISTRY.REALIZATION-NAMED.md](invariants/INV.REGISTRY.REALIZATION-NAMED.md) |
| `INV.REGISTRY.SYMBOL-MATCHES-PATH` | инвариант | active |  | Символ записи выводится из её пути | [invariants/INV.REGISTRY.SYMBOL-MATCHES-PATH.md](invariants/INV.REGISTRY.SYMBOL-MATCHES-PATH.md) |
| `INV.SOURCE.ATOMIC-PUBLISH` | инвариант | active |  | Мутация источника публикуется атомарно или не публикуется | [invariants/INV.SOURCE.ATOMIC-PUBLISH.md](invariants/INV.SOURCE.ATOMIC-PUBLISH.md) |
| `INV.SOURCE.EXACT-VERSION` | инвариант | active |  | Версия формата — точный литерал, а корень — точный QName | [invariants/INV.SOURCE.EXACT-VERSION.md](invariants/INV.SOURCE.EXACT-VERSION.md) |
| `INV.SOURCE.FORMAT-PER-SET` | инвариант | active |  | Формат — свойство набора исходников | [invariants/INV.SOURCE.FORMAT-PER-SET.md](invariants/INV.SOURCE.FORMAT-PER-SET.md) |
| `INV.SOURCE.OBSERVED-BYTES` | инвариант | active |  | Байты источника наблюдаются, а не назначаются | [invariants/INV.SOURCE.OBSERVED-BYTES.md](invariants/INV.SOURCE.OBSERVED-BYTES.md) |
| `INV.SOURCE.SINGLE-RESOLVED-ROOT` | инвариант | active |  | Корень исходников выбирается детерминированно и один раз | [invariants/INV.SOURCE.SINGLE-RESOLVED-ROOT.md](invariants/INV.SOURCE.SINGLE-RESOLVED-ROOT.md) |
| `INV.SOURCE.SNAPSHOT-BINDING` | инвариант | active |  | Ресурс действует внутри своего снимка и своей роли | [invariants/INV.SOURCE.SNAPSHOT-BINDING.md](invariants/INV.SOURCE.SNAPSHOT-BINDING.md) |
| `INV.SOURCE.WRITE-CONTAINMENT` | инвариант | active |  | Запись не выходит за корень рабочего пространства | [invariants/INV.SOURCE.WRITE-CONTAINMENT.md](invariants/INV.SOURCE.WRITE-CONTAINMENT.md) |
| `INV.SURFACE.ACCEPTANCE-UNCHANGED` | инвариант | active |  | Сужение публикации не сужает приём | [invariants/INV.SURFACE.ACCEPTANCE-UNCHANGED.md](invariants/INV.SURFACE.ACCEPTANCE-UNCHANGED.md) |
| `INV.SURFACE.NAMESPACE` | инвариант | active |  | Публичные инструменты живут в пространстве unica | [invariants/INV.SURFACE.NAMESPACE.md](invariants/INV.SURFACE.NAMESPACE.md) |
| `INV.SURFACE.PUBLISHED-ARGS-ARE-READ` | инвариант | active |  | Публикуется то, что обработчик читает | [invariants/INV.SURFACE.PUBLISHED-ARGS-ARE-READ.md](invariants/INV.SURFACE.PUBLISHED-ARGS-ARE-READ.md) |
| `INV.WIRE.BOUNDED-ADMISSION` | инвариант | active |  | Приём вызовов ограничен, отмена кооперативна | [invariants/INV.WIRE.BOUNDED-ADMISSION.md](invariants/INV.WIRE.BOUNDED-ADMISSION.md) |
| `INV.WIRE.DATA-DRIVEN-SCHEMA` | инвариант | active |  | Контракты инструментов заданы данными | [invariants/INV.WIRE.DATA-DRIVEN-SCHEMA.md](invariants/INV.WIRE.DATA-DRIVEN-SCHEMA.md) |
| `INV.WIRE.ONE-SERVER` | инвариант | active |  | Модель видит один сервер и ни одного движка | [invariants/INV.WIRE.ONE-SERVER.md](invariants/INV.WIRE.ONE-SERVER.md) |
| `INV.WIRE.PREVIEW-IS-MUTATION-ONLY` | инвариант | active |  | Предпросмотр принадлежит мутации | [invariants/INV.WIRE.PREVIEW-IS-MUTATION-ONLY.md](invariants/INV.WIRE.PREVIEW-IS-MUTATION-ONLY.md) |
