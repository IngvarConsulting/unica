# Судьба записей замороженного слоя

Слой заморожен 18.08.2026. Этот файл — единственное, что из него нужно читать:
он говорит, что умерло, а чей предмет пересмотрен заново в `arch/`.

**«Умерло»** значит, что запись не имеет силы и её текст не является основанием
ни для чего. Открывать её стоит только чтобы восстановить историю конкретного
изменения — не чтобы узнать, как система устроена сейчас.

**«Заменено»** значит, что предмет записи пересмотрен на своих основаниях, а
несколько близких записей могли сойтись в одну. Символ дан, чтобы не искать;
обратной ссылки из `arch/` нет намеренно.

Итого: решений 72, инвариантов 128, из них пересмотрено заново 110.

## Решения

| Запись | О чём была | Судьба |
| --- | --- | --- |
| `ADR-0001` | Единый публичный MCP `unica` | заменено на `INV.WIRE.ONE-SERVER` |
| `ADR-0002` | Транспортно-нейтральный application layer | заменено на `INV.APP.THIN-TRANSPORT` |
| `ADR-0003` | Cache и workspace state принадлежат orchestrator | заменено на `INV.CACHE.ORCHESTRATOR-OWNED` |
| `ADR-0004` | Operation scripts are reference-only, not runtime backends | умерло |
| `ADR-0005` | Skills route только через `unica` | заменено на `INV.WIRE.ONE-SERVER` |
| `ADR-0006` | Workspace-scoped internal services | заменено на `INV.APP.HIDDEN-SERVICES` |
| `ADR-0008` | Публичный маркетплейс и тонкий проверяемый runtime | заменено на `INV.PKG.THIN-PACKAGE` |
| `ADR-0009` | Зависящий от ОС код живёт за платформенными фасадами инфраструктуры | заменено на `INV.PLATFORM.OS-BEHIND-FACADE` |
| `ADR-0010` | Кеш сборки и поток артефактов в CI | умерло |
| `ADR-0011` | DCS — каноническое имя домена компоновки данных | умерло |
| `ADR-0012` | Один каталог плагина обслуживает Codex и Claude Code | заменено на `INV.PKG.TWO-HOSTS-ONE-TREE` |
| `ADR-0013` | Транспортом MCP владеет официальный Rust SDK | заменено на `CON.WIRE.PROTOCOL` |
| `ADR-0014` | Знание о хосте живёт за host-фасадом инфраструктуры | заменено на `INV.HOST.KNOWLEDGE-BEHIND-FACADE` |
| `ADR-0015` | Узкие границы `unica.code.patch` v1 | умерло |
| `ADR-0016` | Единственный записываемый профиль выгрузки — платформа `8.3.27`, формат `2.20` | заменено на `CON.FORMAT.PLATFORM-XML-8-3-27` |
| `ADR-0017` | Нейтральная к поставщику модель анализа кода | заменено на `INV.APP.PROVIDER-NEUTRAL` |
| `ADR-0018` | Состояние поставщиков изолировано рабочим деревом | заменено на `INV.CACHE.STATE-OUTSIDE-SOURCE` |
| `ADR-0019` | Публичные MCP-схемы используют канонические имена путей | заменено на `INV.SURFACE.ACCEPTANCE-UNCHANGED` |
| `ADR-0020` | `unica.code.outline` возвращает типизированную структуру текущего BSL-файла | умерло |
| `ADR-0021` | Логические адреса целей не зависят от файловой раскладки | умерло |
| `ADR-0022` | Низкоуровневый доступ к ресурсам ограничен снимком и ролью | заменено на `INV.SOURCE.SNAPSHOT-BINDING` |
| `ADR-0023` | Результат инструмента — типизированные данные, а не проза | заменено на `DEC.2026-08-18.RESULT-FORM` |
| `ADR-0024` | Домен XDTO правит пакет по логическому адресу, а валидация живёт в предпросмотре | умерло |
| `ADR-0025` | Поверхность `unica.meta.*` состоит из одного чтения и трёх мутаций | заменено на `DEC.2026-08-18.NO-FILE-DSL` |
| `ADR-0026` | `insert` без селектора пишет в конец модуля | умерло |
| `ADR-0027` | Корни платформенного XML имеют единый закрытый реестр версионирования | заменено на `INV.SOURCE.EXACT-VERSION` |
| `ADR-0028` | Чтение `unica.meta.info` не теряет данные из-за несвязанной проверки | умерло |
| `ADR-0029` | Справка платформы приходит из установки, а поиск по документации нейтрален к поставщику | умерло |
| `ADR-0030` | Объектную целостность требует вход `meta`, межобъектную — проверка над `cf` | умерло |
| `ADR-0031` | Публикация корня и владение форматом задаются раздельно | заменено на `INV.SOURCE.EXACT-VERSION` |
| `ADR-0032` | Сетевые источники документации — база знаний вендора, фасад стандартов и файл политики сетевого выхода | умерло |
| `ADR-0033` | Полнотекстовое получение документа по локатору попадания | умерло |
| `ADR-0034` | Справка конфигурации рабочего пространства как корпус документации | умерло |
| `ADR-0035` | Находки проверки метаданных несут код правила и языковое местоположение | умерло |
| `ADR-0036` | Адрес и эффективная роль подсистемы выводятся из зарегистрированной топологии | умерло |
| `ADR-0037` | Лексическое ядро поиска по документации | умерло |
| `ADR-0038` | Источник подписки задаётся типизированным отношением метаданных | умерло |
| `ADR-0039` | Подписка валидируется как единая логическая связка | умерло |
| `ADR-0040` | Операционная конфигурация рабочего пространства разрешается снимком на вызов | заменено на `INV.APP.CONFIG-SNAPSHOT` |
| `ADR-0041` | `unica.meta.info` имеет отдельную полную read-model | умерло |
| `ADR-0042` | Чтение метаданных не зависит от плана мутации | умерло |
| `ADR-0043` | Предопределённые элементы и права роли изменяются типизированными логическими операциями | умерло |
| `ADR-0044` | Читатели не принимают режим предпросмотра | заменено на `INV.WIRE.PREVIEW-IS-MUTATION-ONLY` |
| `ADR-0045` | Диагностика и RLM различают завершённый типизированный результат | умерло |
| `ADR-0046` | Отсутствие предопределённых элементов выражается отсутствием файла | умерло |
| `ADR-0047` | `unica.meta.info` имеет отдельную полную read-model | умерло |
| `ADR-0048` | Типизированный читатель публикует только достижимый селектор | умерло |
| `ADR-0049` | Предметный читатель принимает логический адрес раньше, чем теряет путь | заменено на `INV.SURFACE.ACCEPTANCE-UNCHANGED` |
| `ADR-0050` | Идентичность заимствованного описателя выдаётся один раз | умерло |
| `ADR-0051` | Аргументы команды публикует тот инструмент, который её исполняет | умерло |
| `ADR-0052` | Аргумент, которого не читает ни один обработчик, не публикуется | заменено на `INV.SURFACE.PUBLISHED-ARGS-ARE-READ` |
| `ADR-0053` | Постоянная сессия поставщика не удерживает рабочее дерево | умерло |
| `ADR-0054` | Состояние поддержки читается по логической цели через порт | умерло |
| `ADR-0055` | Дымовая проверка исполняет извлечённый проверенный архив runtime | умерло |
| `ADR-0056` | Поиск кода наблюдаем и опирается на роли поставщиков | умерло |
| `ADR-0057` | Свежесть RLM доказывается доверенным поколением исходников | умерло |
| `ADR-0058` | `git-grep` возвращает ограниченный неранжируемый префикс | умерло |
| `ADR-0059` | Несовместимая версия RLM получает новое поколение состояния | заменено на `INV.CACHE.STATE-OUTSIDE-SOURCE` |
| `ADR-0060` | `unica.project.status` публикует типизированную готовность проекта | умерло |
| `ADR-0061` | Многофайловый RLM входит в runtime замкнутой полезной нагрузкой | умерло |
| `ADR-0062` | Готовность RLM является типизированным состоянием | умерло |
| `ADR-0063` | Публичная диагностика адресует логические цели | умерло |
| `ADR-0064` | Поставщики диагностик сохраняют происхождение и отказывают независимо | заменено на `INV.APP.PROVIDER-NEUTRAL` |
| `ADR-0065` | Миграция читателя выбирает мост или прямое переключение явно | умерло |
| `ADR-0066` | Терминальный результат runtime возвращается в том же вызове | заменено на `DEC.2026-08-18.NO-JOB-REGISTRY` |
| `ADR-0067` | Доказанный отказ частичной runtime-сборки один раз повторяется полным путём | умерло |
| `ADR-0068` | Публикация идёт линейным конвейером от единственного человеческого тега | заменено на `INV.CI.TAG-ONLY-PUBLISH` |
| `ADR-0069` | Версии протокола MCP обслуживаются по документации SDK, гарантируются по матрице хостов | заменено на `CON.WIRE.PROTOCOL` |
| `ADR-0070` | Большое чтение продолжается по ссылке без повторного вычисления | умерло |
| `ADR-0071` | `unica.xdto.edit` принимает типизированный массив операций одной транзакцией | умерло |
| `ADR-0072` | Регистрация макетов и встроенная справка принадлежат `meta.edit`; `template.*` и `help.add` снимаются | умерло |

| `ADR-0073` | Предпросмотр мутатора честен: одна форма данных, кеш в режиме предпросмотра, явный отказ вместо молчаливого успеха | заменено на `DEC.2026-08-18.FAILURE-NAMES-THE-FILE` |
## Инварианты

| Запись | О чём была | Судьба |
| --- | --- | --- |
| `INV-PRODUCT-SINGLE-PLUGIN-TREE` | Один каталог плагина обслуживает двух хостов | заменено на `INV.PKG.TWO-HOSTS-ONE-TREE` |
| `INV-PRODUCT-DEVELOPER-OPERATIONS` | Публичная поверхность моделирует операции разработчика | умерло |
| `INV-PRODUCT-NO-ENGINE-ROUTING` | Встроенные движки не попадают в маршрутизацию, видимую модели | заменено на `INV.WIRE.ONE-SERVER` |
| `INV-PRODUCT-PACKAGE-PARITY` | Сгенерированный пакет — полноценная поставка | заменено на `INV.PKG.TWO-HOSTS-ONE-TREE` |
| `INV-PRODUCT-TOOL-VERSION-SOURCE` | У версий встроенных инструментов один источник | заменено на `INV.PKG.TWO-HOSTS-ONE-TREE` |
| `INV-PRODUCT-DCS-NAMING` | DCS — каноническое имя домена компоновки данных | умерло |
| `INV-PRODUCT-NO-FORMAT-MIGRATION` | Unica не мигрирует формат выгрузки | заменено на `INV.PRODUCT.NO-FORMAT-MIGRATION` |
| `INV-MCP-META-SURFACE` | Метаданные доступны четырьмя предметными операциями | умерло |
| `INV-MCP-META-OBSERVATION` | Наблюдение метаданных шире возможности мутации | умерло |
| `INV-MCP-META-INFO-COVERAGE` | `meta.info` имеет отдельный полный профиль чтения | умерло |
| `INV-MCP-EVENT-SOURCE` | Источник подписки является типизированным отношением | умерло |
| `INV-MCP-EVENT-BINDING` | Подписка публикуется только как совместимая связка | умерло |
| `INV-MCP-META-FINDINGS` | Перенесённые находки метаданных имеют устойчивую идентичность | умерло |
| `INV-MCP-ROLE-EDIT` | Право роли изменяется через логическую typed-операцию | умерло |
| `INV-MCP-XDTO-LOGICAL-TARGET` | XDTO-пакет выбирается логическим адресом | умерло |
| `INV-MCP-NO-ENGINE-SERVERS` | `unica` — единственный MCP-сервер, видимый модели | заменено на `INV.WIRE.ONE-SERVER` |
| `INV-MCP-SINGLE-ENTRY` | Единственный публичный MCP-сервер | заменено на `INV.WIRE.ONE-SERVER` |
| `INV-MCP-SERVER-NAME` | Имя сервера в протоколе | заменено на `INV.WIRE.ONE-SERVER` |
| `INV-MCP-NAMESPACE` | Публичные инструменты живут в пространстве имён `unica.*` | заменено на `INV.SURFACE.NAMESPACE` |
| `INV-MCP-DATA-DRIVEN-SCHEMA` | Контракты инструментов заданы данными и свободны от адаптеров | заменено на `INV.WIRE.DATA-DRIVEN-SCHEMA` |
| `INV-MCP-REACHABLE-ARGS` | Инструмент публикует только достижимый аргумент | заменено на `INV.WIRE.PREVIEW-IS-MUTATION-ONLY` |
| `INV-MCP-SDK-TRANSPORT` | Транспортом владеет официальный Rust SDK | умерло |
| `INV-MCP-VERSION-TIERS` | Версии протокола обслуживаются по SDK, гарантируются по матрице | умерло |
| `INV-MCP-DEFERRED-READ` | Большое типизированное чтение продолжается по ссылке | умерло |
| `INV-MCP-BOUNDED-ADMISSION` | Приём вызовов ограничен, отмена кооперативна | заменено на `INV.WIRE.BOUNDED-ADMISSION` |
| `INV-MCP-RUNTIME-RECEIPT` | Жизненный цикл runtime принадлежит одному вызову | заменено на `DEC.2026-08-18.NO-JOB-REGISTRY` |
| `INV-MCP-SURFACE-SYNC` | Изменения публичной поверхности синхронны | заменено на `CON.WIRE.TOOL-SURFACE` |
| `INV-MCP-TYPED-RESULT` | Результат инструмента типизирован, а не отрисован текстом | заменено на `DEC.2026-08-18.RESULT-FORM` |
| `INV-MCP-DIAGNOSTIC-TARGET` | Диагностика адресует логическую цель и фокус | умерло |
| `INV-MCP-PROJECT-READINESS` | Готовность проекта публикуется двумя независимыми контурами | умерло |
| `INV-MCP-PREVIEW-MUTATION-ONLY` | Предпросмотр принадлежит мутации | заменено на `INV.WIRE.PREVIEW-IS-MUTATION-ONLY` |
| `INV-MCP-SOURCE-SURFACE` | Ресурсная поверхность логична и ограничена | умерло |
| `INV-MCP-CODE-SEARCH-ROLES` | Поиск наблюдаем и адресуется логически | умерло |
| `INV-MCP-DOCUMENTATION-SECTIONS` | Поиск по документации сохраняет независимые секции поставщиков | умерло |
| `INV-MCP-DOCUMENTATION-GET` | Полный текст документа отдаёт владелец его локатора | умерло |
| `INV-APP-DOCUMENTATION-NETWORK-POLICY` | Сетевой выход поставщиков документации управляется политикой проекта | умерло |
| `INV-MCP-SEARCH-SEMANTICS` | Локальные корпуса документации сопоставляются одним лексическим контрактом | умерло |
| `INV-MCP-OUTLINE-DATA` | Outline возвращает типизированные данные | умерло |
| `INV-SKILL-DECLARED-ROUTING` | Скиллы маршрутизируются через MCP `unica` | умерло |
| `INV-SKILL-NO-ADAPTER-TARGETS` | Скиллы не называют внутренние серверы-адаптеры | заменено на `INV.WIRE.ONE-SERVER` |
| `INV-SKILL-NO-SCRIPT-ROUTE` | Локальные для скилла скрипты операций не возвращаются | умерло |
| `INV-SKILL-SCRIPTS-AS-FIXTURES` | Эталонные модели существуют только как тестовые фикстуры | умерло |
| `INV-SKILL-DOCUMENTED-PREVIEW` | Изменяющие инструкции по умолчанию ведут через предпросмотр | умерло |
| `INV-SKILL-SOURCE-FALLBACK` | Ресурсная запись остаётся запасным маршрутом | умерло |
| `INV-SKILL-EXECUTABLE-EXAMPLES` | Примеры в скиллах — исполнимые вызовы MCP | умерло |
| `INV-SKILL-REACHABLE-REFERENCES` | Справочный документ поставки назван скиллом | умерло |
| `INV-APP-DISPATCH-OWNERSHIP` | Слой application владеет диспетчеризацией и доменными событиями | заменено на `INV.APP.THIN-TRANSPORT` |
| `INV-APP-THIN-TRANSPORT` | Транспорт только отображает протокол на вызовы application | заменено на `INV.APP.THIN-TRANSPORT` |
| `INV-APP-NO-ADAPTER-BYPASS` | Адаптеры идут к рабочему пространству через порты application | заменено на `INV.APP.DEPENDENCY-DIRECTION` |
| `INV-APP-NO-SCRIPT-BACKEND` | В runtime нет скриптового бэкенда | заменено на `INV.APP.DEPENDENCY-DIRECTION` |
| `INV-APP-DEPENDENCY-DIRECTION` | Направление зависимостей между слоями закреплено проверкой | заменено на `INV.APP.DEPENDENCY-DIRECTION` |
| `INV-APP-NO-DIRECT-GIT` | Application не запускает git напрямую | заменено на `INV.APP.DEPENDENCY-DIRECTION` |
| `INV-APP-SUPPORT-STATE` | Состояние поддержки читается по логической цели | умерло |
| `INV-APP-PARTIAL-FALLBACK` | Runtime-build повторяет только доказанный частичный отказ | умерло |
| `INV-APP-CONFIG-SNAPSHOT` | Конфигурация вызова изолирована рабочим пространством | заменено на `INV.APP.CONFIG-SNAPSHOT` |
| `INV-APP-CODE-PROVIDER-BOUNDARY` | Анализ кода не зависит от движка | заменено на `INV.APP.PROVIDER-NEUTRAL` |
| `INV-APP-DIAGNOSTIC-PROVIDERS` | Наблюдения поставщиков компонуются независимо | заменено на `INV.APP.PROVIDER-NEUTRAL` |
| `INV-APP-DOCUMENTATION-NO-DISK-STATE` | Разбор корпуса справки не создаёт состояния на диске | заменено на `INV.APP.HIDDEN-SERVICES` |
| `INV-APP-OUTLINE-SOURCE` | Структура модуля берётся из текущего файла | умерло |
| `INV-APP-LAZY-HIDDEN-SERVICES` | Внутренние сервисы скрыты и привязаны к рабочему пространству | заменено на `INV.APP.HIDDEN-SERVICES` |
| `INV-CACHE-ORCHESTRATOR-OWNED` | Состоянием рабочего пространства владеет оркестратор | заменено на `INV.CACHE.ORCHESTRATOR-OWNED` |
| `INV-CACHE-REPORTED-EFFECTS` | Изменяющие операции порождают типизированные доменные события | заменено на `INV.CACHE.ORCHESTRATOR-OWNED` |
| `INV-CACHE-WORKSPACE-ROOT` | Корень изменчивого кеша можно переопределить | заменено на `INV.CACHE.STATE-OUTSIDE-SOURCE` |
| `INV-CACHE-PROVIDER-STATE-OUTSIDE-SOURCE` | Постоянное состояние поставщика не индексирует само себя | заменено на `INV.CACHE.STATE-OUTSIDE-SOURCE` |
| `INV-CACHE-GENERATION-CUTOVER` | Несовместимый индекс получает новое поколение | заменено на `INV.CACHE.STATE-OUTSIDE-SOURCE` |
| `INV-CACHE-WRITE-FREE-PREVIEW` | Сухой прогон сообщает о последствиях, не записывая состояние | заменено на `INV.CACHE.PREVIEW-WRITES-NOTHING` |
| `INV-CACHE-PERSISTED-STALENESS` | Применённое изменение запоминает инвалидированный им кеш | заменено на `INV.CACHE.ORCHESTRATOR-OWNED` |
| `INV-CACHE-WORKTREE-ISOLATION` | Связанное рабочее дерево git изолировано | заменено на `INV.CACHE.STATE-OUTSIDE-SOURCE` |
| `INV-CACHE-RLM-REVISION` | Готовность RLM привязана к доверенной ревизии | заменено на `INV.CACHE.STATE-OUTSIDE-SOURCE` |
| `INV-CACHE-RUNTIME-ROOT-ORDER` | Разрешение корня кеша runtime детерминировано | заменено на `INV.CACHE.STATE-OUTSIDE-SOURCE` |
| `INV-SOURCE-ROOT-SEPARATION` | Корень исходников отделён от рабочего пространства | заменено на `INV.SOURCE.SINGLE-RESOLVED-ROOT` |
| `INV-SOURCE-PORTABLE-GIT` | Переносимость Git доказывается содержимым репозитория | умерло |
| `INV-SOURCE-PER-SET-FORMAT` | Формат — свойство набора исходников | заменено на `INV.SOURCE.FORMAT-PER-SET` |
| `INV-SOURCE-UNAMBIGUOUS-SET` | Один набор исходников не бывает двух форматов сразу | заменено на `INV.SOURCE.FORMAT-PER-SET` |
| `INV-SOURCE-MULTI-FORMAT-WORKSPACE` | В рабочем пространстве может действовать несколько форматов | заменено на `INV.SOURCE.FORMAT-PER-SET` |
| `INV-SOURCE-PLATFORM-XML-ONLY` | Нативные операции с XML требуют формата platform XML | заменено на `INV.SOURCE.FORMAT-PER-SET` |
| `INV-SOURCE-SINGLE-RESOLVED-ROOT` | Выбор корня исходников детерминирован и общий | заменено на `INV.SOURCE.SINGLE-RESOLVED-ROOT` |
| `INV-SOURCE-LOGICAL-IDENTITY` | Точная цель не зависит от файловой раскладки | заменено на `DEC.2026-08-18.ADDRESS-GRAMMAR` |
| `INV-SOURCE-SUBSYSTEM-TOPOLOGY` | Публичные проекции подсистем выводятся из регистрации | умерло |
| `INV-SOURCE-READER-SELECTOR` | Предметный читатель принимает ровно один селектор цели | умерло |
| `INV-SOURCE-READER-MIGRATION` | Режим миграции читателя объявлен явно | умерло |
| `INV-SOURCE-WRITE-TARGET-KIND` | Писатель принимает только терминал модуля | умерло |
| `INV-SOURCE-SNAPSHOT-BINDING` | Ресурс действует только внутри своего снимка | заменено на `INV.SOURCE.SNAPSHOT-BINDING` |
| `INV-SOURCE-ROLE-ALLOWLIST` | Право записи выдаётся по доказанной роли | заменено на `INV.SOURCE.SNAPSHOT-BINDING` |
| `INV-SOURCE-OBSERVED-EOL` | Перевод строки наблюдается в источнике, а не назначается | заменено на `INV.SOURCE.OBSERVED-BYTES` |
| `INV-SOURCE-TAIL-INSERT` | Вставка без селектора идёт в конец и доказывает повтор | умерло |
| `INV-SOURCE-ATOMIC-PUBLISH` | Мутация источника публикуется атомарно после проверки | заменено на `INV.SOURCE.ATOMIC-PUBLISH` |
| `INV-SOURCE-IDEMPOTENT-REWRITE` | Повторная идентичная мутация ничего не пишет | заменено на `INV.SOURCE.ATOMIC-PUBLISH` |
| `INV-SOURCE-WRITE-CONTAINMENT` | Запись не выходит за корень рабочего пространства | заменено на `INV.SOURCE.WRITE-CONTAINMENT` |
| `INV-SOURCE-WRITABLE-FORMAT` | Записывается только действующий профиль выгрузки | заменено на `INV.SOURCE.EXACT-VERSION` |
| `INV-SOURCE-ROOT-POLICIES` | Публикация и владение форматом задаются независимо | заменено на `INV.SOURCE.EXACT-VERSION` |
| `INV-SOURCE-OWNER-VERSION-GATE` | Версию решает корень-владелец, отказ наступает до первой записи | заменено на `INV.SOURCE.EXACT-VERSION` |
| `INV-SOURCE-EXACT-VERSION-LITERAL` | Поддерживаемая версия — точный литерал, а не численное равенство | заменено на `INV.SOURCE.EXACT-VERSION` |
| `INV-SOURCE-EXACT-ROOT-QNAME` | Цель записи опознаётся по точному QName корня | заменено на `INV.SOURCE.EXACT-VERSION` |
| `INV-SOURCE-BOUND-PREIMAGES` | Мутация привязана к байтам, из которых выведена | заменено на `INV.SOURCE.OBSERVED-BYTES` |
| `INV-SOURCE-ROLLBACK-VISIBLE` | Неудавшийся откат виден как ошибка целостности | заменено на `INV.SOURCE.ATOMIC-PUBLISH` |
| `INV-PKG-UNTRACKED-BUILD-OUTPUT` | Собранные бинарники не попадают под контроль версий | заменено на `INV.PKG.THIN-PACKAGE` |
| `INV-PKG-THIN-PACKAGE` | Публичный пакет маркетплейса тонкий | заменено на `INV.PKG.THIN-PACKAGE` |
| `INV-PKG-VERIFIED-ATOMIC-INSTALL` | Получение runtime проверяется контрольной суммой и атомарно | заменено на `INV.PKG.VERIFIED-ATOMIC-INSTALL` |
| `INV-PKG-TOOL-CLOSURE` | Многофайловый инструмент входит в runtime полностью | заменено на `INV.PKG.VERIFIED-ATOMIC-INSTALL` |
| `INV-PKG-BINARY-NAME` | Публичный бинарник runtime называется `unica` | заменено на `INV.PKG.TWO-HOSTS-ONE-TREE` |
| `INV-PKG-VERSION-LOCKSTEP` | Оба манифеста хостов несут одну версию | заменено на `INV.PKG.TWO-HOSTS-ONE-TREE` |
| `INV-PKG-OLDEST-CLIENT-KEYS` | Манифесты и каталоги не выходят за нижнюю границу клиента | заменено на `INV.PKG.TWO-HOSTS-ONE-TREE` |
| `INV-PKG-DEV-ONLY-PACKAGE` | Локальная отладочная упаковка существует только для разработки | заменено на `INV.PKG.THIN-PACKAGE` |
| `INV-PKG-NO-INTERNAL-MATERIAL` | Внутренние материалы сопровождения не уезжают в поставку | заменено на `INV.PKG.THIN-PACKAGE` |
| `INV-PKG-ATTRIBUTION-COVERAGE` | Атрибуция остаётся полной и доступной | заменено на `INV.PKG.ATTRIBUTION` |
| `INV-PLATFORM-OS-BEHIND-FACADE` | Зависящий от ОС код живёт за платформенными фасадами | заменено на `INV.PLATFORM.OS-BEHIND-FACADE` |
| `INV-PLATFORM-NO-PATH-EXEMPTIONS` | У платформенного стража нет исключений по путям | заменено на `INV.PLATFORM.OS-BEHIND-FACADE` |
| `INV-PLATFORM-COLOCATED-TESTS` | Платформенные тесты лежат рядом со своими адаптерами | заменено на `INV.PLATFORM.OS-BEHIND-FACADE` |
| `INV-PLATFORM-NO-ORPHAN-PROCESSES` | Дочерние процессы удерживаются целыми деревьями | заменено на `INV.PLATFORM.NO-ORPHANS` |
| `INV-HOST-NEUTRAL-ORCHESTRATOR` | Оркестратор нейтрален к хосту | заменено на `INV.HOST.KNOWLEDGE-BEHIND-FACADE` |
| `INV-HOST-KNOWLEDGE-BEHIND-FACADE` | Знание о хосте живёт за host-фасадом | заменено на `INV.HOST.KNOWLEDGE-BEHIND-FACADE` |
| `INV-HOST-UNIFORM-CALL-SITES` | Добавление хоста не меняет мест вызова | заменено на `INV.HOST.KNOWLEDGE-BEHIND-FACADE` |
| `INV-CI-MANDATORY-BUILD` | Одна закреплённая сборка Cargo на платформенный раннер | заменено на `INV.CI.TAG-ONLY-PUBLISH` |
| `INV-CI-EXACT-CACHE-KEYS` | Попадания в кеш Cargo точны и наблюдаемы | заменено на `INV.CI.TAG-ONLY-PUBLISH` |
| `INV-CI-NARROW-ARTIFACTS` | Артефакты узкие, типизированные и недолговечные | заменено на `INV.CI.TAG-ONLY-PUBLISH` |
| `INV-CI-SELF-VERIFIED-ARCHIVE` | Каждая платформа проверяет то, что собрала | заменено на `INV.CI.TAG-ONLY-PUBLISH` |
| `INV-CI-TAG-ONLY-PUBLISH` | Публикация происходит только по тегу | заменено на `INV.CI.TAG-ONLY-PUBLISH` |
| `INV-CI-SINGLE-GATE` | Каждый pull request закрывает один агрегирующий шлюз | заменено на `INV.CI.TAG-ONLY-PUBLISH` |
| `INV-DOC-REGISTRY-ENTRY-FORMAT` | Записи реестра оформлены каноническим форматом | умерло |
| `INV-DOC-NO-ID-REUSE` | ID реестра уникальны и не переиспользуются | умерло |
| `INV-DOC-REAL-CHECKS` | Каждый инвариант называет настоящую проверку | умерло |
| `INV-DOC-INDEX-SYNC` | Индексы синхронны со своими документами | умерло |
| `INV-DOC-ARCHIVE-NOT-NORMATIVE` | Исторические документы помечены как исторические | умерло |
| `INV-DOC-RELATIVE-LINKS` | Относительные ссылки разрешаются от своего документа | умерло |
| `INV-DOC-RUSSIAN-NORMATIVE` | Нормативный текст пишется по-русски | умерло |
| `INV-DOC-SINGLE-RULE-OWNER` | У нормативного текста один владелец | умерло |
| `INV-DOC-SUPERSEDE-NOT-EDIT` | Принятое решение не переписывают | умерло |
