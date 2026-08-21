<!-- ПОРОЖДАЕТСЯ scripts/arch/registry.py --write-index; руками не правится -->

# Индекс реестра

| Символ | Вид | Статус | Построено | Суть | Файл |
| --- | --- | --- | --- | --- | --- |
| `CTR.FORMAT.PLATFORM-XML-8-3-27` | контракт · product | active |  | Чтение ресурсов сохраняет байты корпуса Platform XML 8.3.27 | [contracts/CTR.FORMAT.PLATFORM-XML-8-3-27.md](contracts/CTR.FORMAT.PLATFORM-XML-8-3-27.md) |
| `CTR.WIRE.LIST-CACHE-FIELDS` | контракт · product | active |  | Современный list несёт cache-поля, legacy сохраняет прежнюю форму | [contracts/CTR.WIRE.LIST-CACHE-FIELDS.md](contracts/CTR.WIRE.LIST-CACHE-FIELDS.md) |
| `CTR.WIRE.TOOL-SURFACE` | контракт · product | active |  | Ведомость поверхности порождается из бинаря | [contracts/CTR.WIRE.TOOL-SURFACE.md](contracts/CTR.WIRE.TOOL-SURFACE.md) |
| `DEC.2026-08-18.ARCHITECTURE-RESET` | решение · process | active | да | Архитектура описывается заново | [decisions/2026-08-18-architecture-reset.md](decisions/2026-08-18-architecture-reset.md) |
| `DEC.2026-08-18.CARRIED-RULES` | решение · process | active | да | Правило переносится, если его проверка жива и предмет не про имена инструментов | [decisions/2026-08-18-carried-rules.md](decisions/2026-08-18-carried-rules.md) |
| `DEC.2026-08-18.REGISTRY-SHAPE` | решение · process | active | да | Три символических реестра, одна запись — один файл | [decisions/2026-08-18-registry-shape.md](decisions/2026-08-18-registry-shape.md) |
| `DEC.2026-08-19.ARTIFACT-IS-THE-ARCHIVE` | решение · product | active | да | Единица доставки — архив, а не инструмент | [decisions/2026-08-19-artifact-is-the-archive.md](decisions/2026-08-19-artifact-is-the-archive.md) |
| `DEC.2026-08-19.ARTIFACT-VERSIONED-CACHE` | решение · product | active | да | Артефакт кешируется по своей версии, а не по версии плагина | [decisions/2026-08-19-artifact-versioned-cache.md](decisions/2026-08-19-artifact-versioned-cache.md) |
| `DEC.2026-08-19.CORE-FIRST-ACQUISITION` | решение · product | active | да | В стартовый бюджет едет только ядро | [decisions/2026-08-19-core-first-acquisition.md](decisions/2026-08-19-core-first-acquisition.md) |
| `DEC.2026-08-19.DECISIONS-FORM-INSIDE` | решение · process | active | да | Решение формируется в реестре, а не в трекере | [decisions/2026-08-19-decisions-form-inside.md](decisions/2026-08-19-decisions-form-inside.md) |
| `DEC.2026-08-19.DELIVERY-HAS-NO-BUDGET` | решение · product | active | да | У доставки нет срока, срок принадлежит запуску | [decisions/2026-08-19-delivery-has-no-budget.md](decisions/2026-08-19-delivery-has-no-budget.md) |
| `DEC.2026-08-19.NO-SELF-GLOSS` | решение · process | active | да | Правило говорит о предмете, а не о себе | [decisions/2026-08-19-no-self-gloss.md](decisions/2026-08-19-no-self-gloss.md) |
| `DEC.2026-08-19.PRODUCT-OR-PROCESS` | решение · process | active | да | Запись объявляет, кто заметит её нарушение | [decisions/2026-08-19-product-or-process.md](decisions/2026-08-19-product-or-process.md) |
| `DEC.2026-08-19.PRODUCT-RECORD-IS-HISTORY` | решение · process | active | да | Принятая продуктовая запись правится только заменой | [decisions/2026-08-19-product-record-is-history.md](decisions/2026-08-19-product-record-is-history.md) |
| `DEC.2026-08-19.REALIZATION-AXIS` | решение · process | active | да | Решённое и построенное — разные оси | [decisions/2026-08-19-realization-axis.md](decisions/2026-08-19-realization-axis.md) |
| `DEC.2026-08-19.REGISTRY-GUARDS-RUN` | решение · process | active | да | Правила о реестре проверяются там же, где правила о продукте | [decisions/2026-08-19-registry-guards-run.md](decisions/2026-08-19-registry-guards-run.md) |
| `DEC.2026-08-19.RETENTION-BY-ARTIFACT` | решение · product | active | да | Кеш удерживает свежие версии каждого артефакта, а не считает ссылки | [decisions/2026-08-19-retention-by-artifact.md](decisions/2026-08-19-retention-by-artifact.md) |
| `DEC.2026-08-19.RULE-CLAIMS-ONLY-WHAT-IT-CHECKS` | решение · process | active | да | Правило заявляет ровно то, что проверяет | [decisions/2026-08-19-rule-claims-only-what-it-checks.md](decisions/2026-08-19-rule-claims-only-what-it-checks.md) |
| `DEC.2026-08-19.STARTUP-LEAVES-A-RECORD` | решение · product | active | да | Запуск оставляет след, который переживает убийство | [decisions/2026-08-19-startup-leaves-a-record.md](decisions/2026-08-19-startup-leaves-a-record.md) |
| `DEC.2026-08-20.ENGINES-COME-FROM-THE-TOOLCHAIN` | решение · product | active | да | Движок приезжает из тулчейна, а не из копии в выпуске плагина | [decisions/2026-08-20-engines-come-from-the-toolchain.md](decisions/2026-08-20-engines-come-from-the-toolchain.md) |
| `DEC.2026-08-20.LONG-WORK-ANSWERS-WITH-STATE` | решение · product | active | да | Доставка зависимости отвечает состоянием раньше, чем её обрывает хост | [decisions/2026-08-20-long-work-answers-with-state.md](decisions/2026-08-20-long-work-answers-with-state.md) |
| `DEC.2026-08-20.PREFETCH-FILLS-THE-CLOSED-CONTOUR` | решение · product | active | да | Закрытый контур наполняется заранее, а не по требованию | [decisions/2026-08-20-prefetch-fills-the-closed-contour.md](decisions/2026-08-20-prefetch-fills-the-closed-contour.md) |
| `DEC.2026-08-21.LIST-CACHE-FIELDS` | решение · product | active | да | Cache-fields split modern and legacy tools/list responses | [decisions/2026-08-21-list-cache-fields.md](decisions/2026-08-21-list-cache-fields.md) |
| `DEC.2026-08-21.PLATFORM-XML-PROFILE` | решение · product | active | да | Writable Platform XML profile is 8.3.27 / format 2.20 | [decisions/2026-08-21-platform-xml-profile.md](decisions/2026-08-21-platform-xml-profile.md) |
| `INV.APP.CONFIG-SNAPSHOT` | инвариант · product | active |  | Оверлей конфигурации не меняет исходный снимок | [invariants/INV.APP.CONFIG-SNAPSHOT.md](invariants/INV.APP.CONFIG-SNAPSHOT.md) |
| `INV.APP.DEPENDENCY-DIRECTION` | инвариант · process | active |  | Направление зависимостей между слоями закреплено проверкой | [invariants/INV.APP.DEPENDENCY-DIRECTION.md](invariants/INV.APP.DEPENDENCY-DIRECTION.md) |
| `INV.APP.HIDDEN-SERVICES` | инвариант · product | active |  | Внутренние сервисы привязаны к рабочему пространству | [invariants/INV.APP.HIDDEN-SERVICES.md](invariants/INV.APP.HIDDEN-SERVICES.md) |
| `INV.APP.PROVIDER-NEUTRAL` | инвариант · product | active |  | Читатель кода выбирается по capability поставщика | [invariants/INV.APP.PROVIDER-NEUTRAL.md](invariants/INV.APP.PROVIDER-NEUTRAL.md) |
| `INV.APP.THIN-TRANSPORT` | инвариант · process | active |  | Транспорт только отображает протокол на вызовы application | [invariants/INV.APP.THIN-TRANSPORT.md](invariants/INV.APP.THIN-TRANSPORT.md) |
| `INV.CACHE.INDEX-PREVIEW-WRITE-FREE` | инвариант · product | active |  | Предпросмотр индекса не оставляет состояния | [invariants/INV.CACHE.INDEX-PREVIEW-WRITE-FREE.md](invariants/INV.CACHE.INDEX-PREVIEW-WRITE-FREE.md) |
| `INV.CACHE.ORCHESTRATOR-OWNED` | инвариант · process | active |  | Координация кеша принадлежит application | [invariants/INV.CACHE.ORCHESTRATOR-OWNED.md](invariants/INV.CACHE.ORCHESTRATOR-OWNED.md) |
| `INV.CACHE.STATE-OUTSIDE-SOURCE` | инвариант · product | active |  | Кеш анализатора не попадает в индексируемый источник | [invariants/INV.CACHE.STATE-OUTSIDE-SOURCE.md](invariants/INV.CACHE.STATE-OUTSIDE-SOURCE.md) |
| `INV.CI.ONE-AGGREGATE-GATE` | инвариант · process | active |  | Каждый pull request закрывает один агрегирующий шлюз | [invariants/INV.CI.ONE-AGGREGATE-GATE.md](invariants/INV.CI.ONE-AGGREGATE-GATE.md) |
| `INV.CI.REGISTRY-GUARDS-RUN` | инвариант · process | active |  | Стражи реестра входят в контур источника | [invariants/INV.CI.REGISTRY-GUARDS-RUN.md](invariants/INV.CI.REGISTRY-GUARDS-RUN.md) |
| `INV.CI.TAG-ONLY-PUBLISH` | инвариант · process | active |  | Релиз начинается человеческим тегом | [invariants/INV.CI.TAG-ONLY-PUBLISH.md](invariants/INV.CI.TAG-ONLY-PUBLISH.md) |
| `INV.DOC.ARCHIVE-FROZEN` | инвариант · process | active |  | Архив v1 заморожен по содержимому | [invariants/INV.DOC.ARCHIVE-FROZEN.md](invariants/INV.DOC.ARCHIVE-FROZEN.md) |
| `INV.DOC.SUPERPOWERS-BOUNDARY` | инвариант · process | active |  | Формы superpowers не входят в реестр | [invariants/INV.DOC.SUPERPOWERS-BOUNDARY.md](invariants/INV.DOC.SUPERPOWERS-BOUNDARY.md) |
| `INV.HOST.KNOWLEDGE-BEHIND-FACADE` | инвариант · process | active |  | Знание о хосте живёт за host-фасадом | [invariants/INV.HOST.KNOWLEDGE-BEHIND-FACADE.md](invariants/INV.HOST.KNOWLEDGE-BEHIND-FACADE.md) |
| `INV.PKG.ATTRIBUTION` | инвариант · product | active |  | Атрибуция остаётся полной | [invariants/INV.PKG.ATTRIBUTION.md](invariants/INV.PKG.ATTRIBUTION.md) |
| `INV.PKG.CORRUPT-ARCHIVE-NOT-READY` | инвариант · product | active |  | Повреждённый архив не становится готовой установкой | [invariants/INV.PKG.CORRUPT-ARCHIVE-NOT-READY.md](invariants/INV.PKG.CORRUPT-ARCHIVE-NOT-READY.md) |
| `INV.PKG.RETENTION-BY-ARTIFACT` | инвариант · product | active |  | Сборка мусора считает версии по артефакту | [invariants/INV.PKG.RETENTION-BY-ARTIFACT.md](invariants/INV.PKG.RETENTION-BY-ARTIFACT.md) |
| `INV.PKG.THIN-PACKAGE` | инвариант · product | active |  | Публичный пакет тонкий | [invariants/INV.PKG.THIN-PACKAGE.md](invariants/INV.PKG.THIN-PACKAGE.md) |
| `INV.PKG.TWO-HOSTS-ONE-TREE` | инвариант · product | active |  | Два хоста разрешают один корень плагина | [invariants/INV.PKG.TWO-HOSTS-ONE-TREE.md](invariants/INV.PKG.TWO-HOSTS-ONE-TREE.md) |
| `INV.PLATFORM.OS-BEHIND-FACADE` | инвариант · process | active |  | Зависящий от ОС код живёт за платформенными фасадами | [invariants/INV.PLATFORM.OS-BEHIND-FACADE.md](invariants/INV.PLATFORM.OS-BEHIND-FACADE.md) |
| `INV.PRODUCT.FULL-DUMP-PROFILE` | инвариант · product | active |  | Полная выгрузка использует общий активный профиль | [invariants/INV.PRODUCT.FULL-DUMP-PROFILE.md](invariants/INV.PRODUCT.FULL-DUMP-PROFILE.md) |
| `INV.REGISTRY.BINDING-IS-REALIZED` | инвариант · process | active |  | Действующее решение не бывает непостроенным | [invariants/INV.REGISTRY.BINDING-IS-REALIZED.md](invariants/INV.REGISTRY.BINDING-IS-REALIZED.md) |
| `INV.REGISTRY.CHECK-EXISTS` | инвариант · process | active |  | Запись, несущая проверку, называет существующую цель | [invariants/INV.REGISTRY.CHECK-EXISTS.md](invariants/INV.REGISTRY.CHECK-EXISTS.md) |
| `INV.REGISTRY.GOVERNS-DECLARED` | инвариант · process | active |  | Сторона записи объявлена и известна | [invariants/INV.REGISTRY.GOVERNS-DECLARED.md](invariants/INV.REGISTRY.GOVERNS-DECLARED.md) |
| `INV.REGISTRY.NO-SELF-GLOSS` | инвариант · process | active |  | Инвариант и контракт не толкуют свои поля | [invariants/INV.REGISTRY.NO-SELF-GLOSS.md](invariants/INV.REGISTRY.NO-SELF-GLOSS.md) |
| `INV.REGISTRY.NO-TRACKER-LINKS` | инвариант · process | active |  | Реестр не ссылается на трекер | [invariants/INV.REGISTRY.NO-TRACKER-LINKS.md](invariants/INV.REGISTRY.NO-TRACKER-LINKS.md) |
| `INV.REGISTRY.PRODUCT-DECISION-IS-HISTORY` | инвариант · process | active |  | Продуктовое решение из базы не правится | [invariants/INV.REGISTRY.PRODUCT-DECISION-IS-HISTORY.md](invariants/INV.REGISTRY.PRODUCT-DECISION-IS-HISTORY.md) |
| `INV.REGISTRY.PRODUCT-RULE-NEEDS-GROUND` | инвариант · process | active |  | Продуктовое правило меняется вместе с решением о причине | [invariants/INV.REGISTRY.PRODUCT-RULE-NEEDS-GROUND.md](invariants/INV.REGISTRY.PRODUCT-RULE-NEEDS-GROUND.md) |
| `INV.REGISTRY.REALIZATION-NAMED` | инвариант · process | active |  | Решение называет свидетельство реализации | [invariants/INV.REGISTRY.REALIZATION-NAMED.md](invariants/INV.REGISTRY.REALIZATION-NAMED.md) |
| `INV.REGISTRY.SYMBOL-MATCHES-PATH` | инвариант · process | active |  | Символ записи выводится из её пути | [invariants/INV.REGISTRY.SYMBOL-MATCHES-PATH.md](invariants/INV.REGISTRY.SYMBOL-MATCHES-PATH.md) |
| `INV.SOURCE.ATOMIC-PUBLISH` | инвариант · product | active |  | Ошибка до commit не меняет цель публикации | [invariants/INV.SOURCE.ATOMIC-PUBLISH.md](invariants/INV.SOURCE.ATOMIC-PUBLISH.md) |
| `INV.SOURCE.DEFAULT-SET-SELECTION` | инвариант · product | active |  | Набор `main` детерминированно выбирается по умолчанию | [invariants/INV.SOURCE.DEFAULT-SET-SELECTION.md](invariants/INV.SOURCE.DEFAULT-SET-SELECTION.md) |
| `INV.SOURCE.EXACT-VERSION` | инвариант · product | active |  | Версия самой цели старше версии окружающего набора | [invariants/INV.SOURCE.EXACT-VERSION.md](invariants/INV.SOURCE.EXACT-VERSION.md) |
| `INV.SOURCE.FORMAT-PER-SET` | инвариант · product | active |  | Кодовая мутация соблюдает формат своего набора исходников | [invariants/INV.SOURCE.FORMAT-PER-SET.md](invariants/INV.SOURCE.FORMAT-PER-SET.md) |
| `INV.SOURCE.OBSERVED-BYTES` | инвариант · product | active |  | Снимок сохраняет исходные байты и отделяет BOM | [invariants/INV.SOURCE.OBSERVED-BYTES.md](invariants/INV.SOURCE.OBSERVED-BYTES.md) |
| `INV.SOURCE.SNAPSHOT-BINDING` | инвариант · product | active |  | Ресурс действует только внутри выдавшего его снимка | [invariants/INV.SOURCE.SNAPSHOT-BINDING.md](invariants/INV.SOURCE.SNAPSHOT-BINDING.md) |
| `INV.SOURCE.WRITE-CONTAINMENT` | инвариант · product | active |  | Запись не выходит за корень рабочего пространства | [invariants/INV.SOURCE.WRITE-CONTAINMENT.md](invariants/INV.SOURCE.WRITE-CONTAINMENT.md) |
| `INV.SURFACE.ACCEPTANCE-UNCHANGED` | инвариант · product | active |  | Сужение публикации не сужает приём | [invariants/INV.SURFACE.ACCEPTANCE-UNCHANGED.md](invariants/INV.SURFACE.ACCEPTANCE-UNCHANGED.md) |
| `INV.SURFACE.ARGUMENTS-DESCRIBED` | инвариант · product | active |  | Каждый опубликованный аргумент описан | [invariants/INV.SURFACE.ARGUMENTS-DESCRIBED.md](invariants/INV.SURFACE.ARGUMENTS-DESCRIBED.md) |
| `INV.SURFACE.NAMESPACE` | инвариант · product | active |  | Публичные инструменты живут в пространстве unica | [invariants/INV.SURFACE.NAMESPACE.md](invariants/INV.SURFACE.NAMESPACE.md) |
| `INV.SURFACE.PUBLISHED-ARGS-ARE-READ` | инвариант · product | active |  | Снятые непрочитанные аргументы не возвращаются в схему | [invariants/INV.SURFACE.PUBLISHED-ARGS-ARE-READ.md](invariants/INV.SURFACE.PUBLISHED-ARGS-ARE-READ.md) |
| `INV.SURFACE.RESULT-CONTRACTS-MATCH-REVIEW` | инвариант · product | active |  | Контракт результата совпадает с ревью поверхности | [invariants/INV.SURFACE.RESULT-CONTRACTS-MATCH-REVIEW.md](invariants/INV.SURFACE.RESULT-CONTRACTS-MATCH-REVIEW.md) |
| `INV.WIRE.BOUNDED-ADMISSION` | инвариант · product | active |  | Приём вызовов ограничен | [invariants/INV.WIRE.BOUNDED-ADMISSION.md](invariants/INV.WIRE.BOUNDED-ADMISSION.md) |
| `INV.WIRE.COOPERATIVE-CANCELLATION` | инвариант · product | active |  | Отмена доходит до исполнителя | [invariants/INV.WIRE.COOPERATIVE-CANCELLATION.md](invariants/INV.WIRE.COOPERATIVE-CANCELLATION.md) |
| `INV.WIRE.EOF-DRAINS-WORKERS` | инвариант · product | active |  | EOF-дренирование не оставляет отслеживаемый worker | [invariants/INV.WIRE.EOF-DRAINS-WORKERS.md](invariants/INV.WIRE.EOF-DRAINS-WORKERS.md) |
| `INV.WIRE.ONE-SERVER` | инвариант · product | active |  | Плагин объявляет один публичный MCP-сервер | [invariants/INV.WIRE.ONE-SERVER.md](invariants/INV.WIRE.ONE-SERVER.md) |
| `INV.WIRE.PREVIEW-IS-MUTATION-ONLY` | инвариант · product | active |  | Предпросмотр принадлежит мутации | [invariants/INV.WIRE.PREVIEW-IS-MUTATION-ONLY.md](invariants/INV.WIRE.PREVIEW-IS-MUTATION-ONLY.md) |
