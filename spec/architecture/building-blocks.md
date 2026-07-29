# Строительные блоки

Документ описывает фактическую структуру кода: какие крейты существуют, из каких
модулей они собраны, кто на кого имеет право ссылаться и где физически лежит
реализация публичных операций. Нормативные правила здесь не дублируются — они
принадлежат [реестру инвариантов](invariants.md) и цитируются по ID. Поведение
тех же блоков во времени описано в [runtime](runtime.md).

## Уровень 1 — Cargo workspace

Корневой `Cargo.toml` объявляет пять членов workspace. Публичные runtime-крейты
наследуют версию из `[workspace.package]`, а внутренние format-крейты следуют
своему совместимому контракту.

| Крейт | Библиотека | Бинарник | Назначение |
| --- | --- | --- | --- |
| `crates/unica-coder` | `unica_coder` | `unica` | Оркестратор: обслуживает единственную публичную MCP-поверхность, диспетчеризует все инструменты `unica.*` и владеет состоянием кеша рабочего пространства. |
| `crates/unica-bootstrap` | `unica_bootstrap` | `unica-bootstrap` | Тонкий пусковой бинарник публичного пакета: разрешает закреплённый runtime, публикует его в проверенный кеш и передаёт ему stdio. |
| `crates/unica-format-core` | `unica_format_core` | — | Закрытая format-neutral семантическая модель, команды, результаты и порты. |
| `crates/unica-application` | `unica_application` | — | Format-neutral use cases и policy orchestration над core-портами. |
| `crates/unica-adapter-platform-xml` | `unica_adapter_platform_xml` | — | Семейный адаптер Platform XML: private probes, layouts, version modules, readers и writers. |


Граф format-крейтов следует ADR-0019: `unica-format-core` не зависит от host,
application или adapter; `unica-application` зависит только от core;
`unica-adapter-platform-xml` зависит от core и держит parser/writer/version
внутри семейства; только composition/infrastructure `unica-coder` связывает
application с factory адаптера (INV-SOURCE-FAMILY-PRIVATE-NATIVE).

`unica` — единственный бинарник, который хост видит как MCP-сервер
(INV-MCP-NO-ENGINE-SERVERS, INV-MCP-SINGLE-ENTRY); в публичном пакете перед ним стоит `unica-bootstrap`,
который сам MCP-сервером не является (INV-PKG-THIN-PACKAGE).

У `unica` нет подкоманд. `main.rs` разбирает аргументы в фиксированном порядке:
`--workspace-service`, затем `--runtime-job-worker`, затем `--help`/`-h`, иначе
процесс становится stdio MCP-сервером. Первые два — скрытые внутренние режимы
(INV-APP-LAZY-HIDDEN-SERVICES), их поведение описано в [runtime](runtime.md).

### Ограничения, заданные извне

- Публичный stdio-сервер построен на официальном Rust SDK `rmcp` 2.2,
  подключённом с `default-features = false` и признаками `server` и
  `transport-io` (ADR-0013). Макросы SDK для объявления инструментов не
  используются: имена, описания и входные схемы задаются данными
  (INV-MCP-DATA-DRIVEN-SCHEMA, INV-MCP-SDK-TRANSPORT). Вместе с SDK в ранее полностью синхронный бинарник
  приходит рантайм `tokio`.
- Сервер отвечает на `initialize`, `ping`, `tools/list` и `tools/call`,
  объявляет только возможность tools, а версию протокола берёт из константы SDK
  `ProtocolVersion::LATEST`, а не из литерала в коде.
- Публикуемый пакет маркетплейса не несёт полного runtime. Его `.mcp.json`
  запускает `unica-bootstrap`, который скачивает закреплённый runtime для
  текущего хоста с утверждённого источника релизов
  `https://github.com/IngvarConsulting/unica/releases/download/<tag>/` и
  проверяет его до запуска (ADR-0008, INV-PKG-THIN-PACKAGE, INV-PKG-VERIFIED-ATOMIC-INSTALL).
- Поставляемые движки запускаются напрямую: путь к бинарнику разрешается через
  сгенерированный `plugins/unica/third-party/manifest.json`, а его SHA-256
  сверяется до старта процесса. Скрипта-обёртки между runtime и поставляемым
  инструментом нет.

## Публичная поверхность и место реализации

Источник истины по составу публичной поверхности — реестр в коде:
`UnicaApplication::tools()` в `crates/unica-coder/src/application/mod.rs`.
Таблица ниже перечисляет группы и указывает, в каком модуле лежит их
реализация; конкретные имена инструментов, их схемы и количество читаются из
реестра, а не отсюда. Каждый публичный инструмент называется
`unica.<группа>.<операция>` (INV-MCP-NAMESPACE).

| Группа | Назначение | Где реализована |
| --- | --- | --- |
| `unica.project.*` | состояние рабочего пространства и карта наборов исходников | `infrastructure/application_ports.rs` поверх `infrastructure/project_sources.rs` и `GitTrackingAdapter` |
| `unica.cf.*`, `unica.cfe.*` | конфигурации и расширения | host mapping в `native_operations/registry.rs`, native read/write в `unica-adapter-platform-xml` |
| `unica.meta.*` | объекты метаданных и их структура | neutral navigation/application ports и Platform XML adapter; `unica.meta.profile` — provider registry через `infrastructure/rlm_navigation.rs` |
| `unica.form.*`, `unica.dcs.*`, `unica.mxl.*` | формы, DCS и табличные документы | typed host mapping в `native_operations/registry.rs`, native read/write в Platform XML adapter |
| `unica.role.*`, `unica.subsystem.*`, `unica.interface.*` | права, подсистемы и командный интерфейс | typed host mapping в `native_operations/registry.rs`, native read/write в Platform XML adapter |
| `unica.template.*`, `unica.help.*`, `unica.support.*` | макеты, справка и поддержка | typed host mapping в `native_operations/registry.rs`, native read/write в Platform XML adapter |
| `unica.epf.*`, `unica.erf.*` | внешние обработки и отчёты | typed host mapping в `native_operations/registry.rs`, native read/write в Platform XML adapter |
| `unica.build.*` | выгрузка, загрузка, обновление, сборка и запуск через платформу | `RuntimeAdapter` в `infrastructure/internal_adapters.rs` поверх поставляемого `v8-runner` |
| `unica.runtime.*` | типизированные сценарии runtime и долгоживущие задания | `RuntimeAdapter` и `RuntimeJobAdapter` в `internal_adapters.rs`, состояние — `infrastructure/runtime_jobs.rs` |
| `unica.code.*` | поиск, навигация, правка и диагностика BSL | поиск и навигация — реестр поставщиков в `infrastructure/code_intelligence.rs`; граф и диагностика — адаптер анализатора в `internal_adapters.rs`; `unica.code.patch` — `native_operations/code.rs` |
| `unica.standards.*` | знания о стандартах разработки 1С | `StandardsAdapter` в `internal_adapters.rs` |

Внутренние границы, до которых дотягиваются адаптеры: поставляемый инструмент
runtime для операций платформы, поставляемый анализатор BSL и индекс кода,
типизированный `git grep` как одна секция поиска, зарегистрированные семейные
адаптеры через core-порты и удалённый эндпоинт стандартов по HTTP. Ни одна из них не
является публичной MCP-регистрацией, и видимый модели текст никогда не называет
их целью вызова (INV-MCP-NO-ENGINE-SERVERS, INV-PRODUCT-NO-ENGINE-ROUTING).

## Уровень 2 — `unica-coder`

`lib.rs` объявляет четыре слоя и делает `infrastructure` крейт-приватным:
публичны только `application`, `domain`, `interfaces` и реэкспорт
`run_platform_main`. `composition.rs` — единственный композиционный корень:
`UnicaApplication::new()` собирает приложение поверх
`InfrastructureApplicationPorts`.

### Слой `domain`

Чистая модель без ввода-вывода.

- `cache` — `CacheAccess`, `CacheImpact`, `CacheReport`; отображает
  произошедшие события на имена кешей, которые становятся недействительными или
  требуют упреждающего обновления.
- `cancellation` — `CancellationToken` и общий префикс ошибки `cancelled:`,
  которым отчитывается любая отменяемая операция.
- `code_intelligence` — нейтральные к движку `CodeIntelligenceProvider`,
  `CodeIntelligenceRegistry`, `CodeIntelligenceContext`, возможности, секции
  поиска и типизированные запросы чтения (INV-APP-CODE-PROVIDER-BOUNDARY).
- `events` — `DomainEvent` и `DomainEventKind`: типизированные факты, о которых
  сообщает мутирующая операция.
- `form_edit` — схема определения правки управляемой формы и правила её
  проверки.
- `project_sources` — модель наборов исходников: `ProjectSourceMap`,
  `ProjectSourceSet`, `SourceFormat`, `SourceSetKind`.
- `source_roots` — `ResolvedSourceRoot` и детерминированный выбор набора
  исходников по умолчанию, общий для всех потребителей корня исходников
  (INV-SOURCE-SINGLE-RESOLVED-ROOT).
- `workspace` — `WorkspaceContext`: пассивная запись о `cwd`, `workspace_root`,
  `cache_root` и `workspace_epoch`.

`WorkspaceContext` ничего не обнаруживает: это структура данных. Обнаружение
рабочего пространства выполняет `infrastructure::workspace::discover_workspace`,
потому что домену запрещён доступ к файловой системе и окружению
(ADR-0009, INV-APP-DEPENDENCY-DIRECTION).

### Слой `application`

Транспортно-нейтральная оркестрация (ADR-0002).

- `mod` — `UnicaApplication`, `ToolSpec`, `ToolHandler`, `RuntimeJobAction`,
  `OperationResult`, канонический реестр `tools()` и диспетчер
  `call_tool` / `call_tool_cancellable`.
- `code_intelligence` — `CodeSearchCoordinator` и общий исполнитель операций
  чтения: параллельный запуск поставщиков, сроки, ограничение допущенных
  исполнителей, отмена, сохранение порядка секций и политика частичного успеха
  (INV-MCP-CODE-SEARCH-SECTIONS, INV-MCP-BOUNDED-ADMISSION).
- `tool_contracts` — входные JSON-схемы, нормализация алиасов путей и проверка
  аргументов для каждого зарегистрированного инструмента (INV-MCP-DATA-DRIVEN-SCHEMA).
- `operation_descriptors` — описатели нативных операций, включая политики
  стража поддержки и стража формата и группы алиасов путей.
- `ports` — трейт `ApplicationPorts` и типы `HandlerOutcome`,
  `SupportGuardCheck`, `FormatGuardCheck`.
- `outcome` — `AdapterOutcome`, форма, которую возвращает любой адаптер до того,
  как приложение соберёт `OperationResult`.

`call_tool` выполняет один и тот же порядок для любого инструмента: нормализует
алиасы путей, разрешает `dryRun`, проверяет аргументы, обнаруживает рабочее
пространство через порт, проверяет контекст вызова, оценивает страж формата,
пропускает вызов через стражи маршрута runtime и синхронизации выгрузки,
для мутирующего инструмента оценивает страж поддержки, вызывает обработчик,
собирает доменные события и отчитывается о влиянии на кеш. Страж поддержки
срабатывает до обработчика, а не после: заблокированная мутация не доходит даже
до планирования.

### Слой `interfaces`

Три модуля — три способа запустить процесс.

- `mcp` — публичный stdio MCP-сервер `unica` на `rmcp` (ADR-0013): `run_stdio()`
  обслуживает `rmcp::transport::stdio()` и реализует
  `ServerHandler::list_tools` и `ServerHandler::call_tool`, делегируя работу
  `UnicaApplication` (INV-APP-THIN-TRANSPORT, INV-MCP-SDK-TRANSPORT).
- `workspace_service` — точка входа скрытого режима `--workspace-service`; она
  только передаёт аргументы процесса в
  `infrastructure::workspace_services::run_workspace_service_from_args`.
- `runtime_job_worker` — точка входа скрытого режима `--runtime-job-worker`; она
  только передаёт управление в `infrastructure::runtime_jobs::run_worker_from_args`.

Оба служебных модуля — делегаты в несколько строк: логики в них нет, и
публичными MCP-серверами они не регистрируются (INV-APP-LAZY-HIDDEN-SERVICES).

### Слой `infrastructure`

Адаптеры, файловое состояние и всё, что знает об операционной системе.

- `application_ports` — `InfrastructureApplicationPorts`, единственная
  реализация `ApplicationPorts`, связываемая композиционным корнем (INV-APP-NO-ADAPTER-BYPASS).
- `internal_adapters` — `CliAdapter`, `RuntimeAdapter`, `RuntimeJobAdapter`,
  `GitTrackingAdapter`, `BslAnalyzerMcpAdapter` и `StandardsAdapter`.
  `GitTrackingAdapter`
  крейт-приватен: `unica.project.status` и `unica.project.map` читают состояние
  отслеживания в git через него, а не запускают `git` сами (INV-APP-NO-DIRECT-GIT).
- `code_intelligence` — инфраструктурные поставщики `rlm`, `bsl-analyzer` и
  `git-grep`, их команды, разбор ответов и отображение состояний.
- `rlm_navigation` — типизированные definition, outline и object-profile поверх
  поддерживаемой MCP-сессии RLM без чтения частной SQLite-схемы.
- `native_operations` — тонкий фасад MCP/semantic mapping и BSL-only логики;
  native Platform XML read/write выполняет семейный adapter.
- `platform` — фасад платформы из ADR-0009; разобран ниже.
- `workspace` — `discover_workspace` строит `WorkspaceContext`: поднимается от
  рабочего каталога вверх и останавливается на первом предке, который несёт
  `v8project.yaml` или файл-указатель `.git` связанного worktree. Обычный
  каталог `.git` маркером не считается, поэтому в рядовом чекауте без
  `v8project.yaml` корнем рабочего пространства остаётся сам рабочий каталог.
  Корень кеша берётся из `UNICA_CACHE_DIR` либо из `<workspaceRoot>/.build/unica`
  (INV-CACHE-WORKSPACE-ROOT), а эпоха рабочего пространства считается из пути корня, размера
  и времени изменения `v8project.yaml`, `Configuration.xml` и
  `src/Configuration.xml`, а также из байтов git-файла `HEAD`, найденного через
  каталог `.git` или через файл-указатель worktree, — так связанный worktree
  остаётся изолированным (INV-CACHE-WORKTREE-ISOLATION).
- `workspace_state` — `WorkspaceStateRepository` хранит состояние кеша и записи
  об упреждающих обновлениях под корнем кеша (INV-CACHE-ORCHESTRATOR-OWNED).
- `workspace_services` — `WorkspaceServiceManager` и
  `run_workspace_service_from_args`: жизненный цикл скрытого сервиса рабочего
  пространства, отдельные тёплые транспорты `bsl-analyzer` и RLM, логическая
  сессия RLM и внутренний протокол JSONL (ADR-0018).
- `workspace_index` — жизненный цикл постоянного индекса BSL, который строит
  поставляемый `rlm-bsl-index`: классификация готовности, владение блокировкой,
  сердцебиение, фоновая сборка и обновление.
- `runtime_jobs` — долговечное состояние заданий runtime, общее для рабочего
  процесса заданий и транспортного адаптера: хранилище заданий, переходы фаз,
  маркеры отмены и ограниченные хвосты вывода.
- `bundled_tools` — `resolve_bundled_tool` находит поставляемый исполняемый файл
  через `third-party/manifest.json` для текущей цели сборки и сверяет его
  SHA-256 до запуска; в предпросмотре допускается откат к `tools.lock.json` с
  предупреждением вместо отказа.
- `plugin_runtime` — `find_plugin_root` разрешает корень плагина, предпочитая
  `UNICA_PLUGIN_ROOT` и иначе поднимаясь вверх от исполняемого файла и от
  рабочего каталога.
- `tool_context` — `validate_tool_context` отклоняет вызов, чьи пути выходят за
  рабочее пространство или чей набор исходников не несёт формат, который нужен
  операции (INV-SOURCE-PLATFORM-XML-ONLY).
- `path_policy` — `WorkspacePathPolicy`, общее правило удержания разрешённых
  путей внутри рабочего пространства.
- `support_guard` — `evaluate_support_guard` блокирует мутирующую операцию или
  предупреждает о ней по состоянию поддержки целевого объекта.
- `format_guard` — `evaluate_format_guard` сравнивает формат выгрузки
  затронутого источника с активным профилем формата.
- `project_sources` — `discover_project_source_map` строит карту наборов
  исходников рабочего пространства из `v8project.yaml` и физической
  инвентаризации.
- `source_roots` — `resolve_source_root` и `normalize_path_identity`,
  детерминированный выбор корня исходников для анализатора и индекса.
- `platform_xml_owner` — определяет, какой платформенный XML-файл владеет
  объектом метаданных, и сообщает происхождение этого решения.
- `metadata_kinds` — статическая таблица видов метаданных: XML-тег, каталог
  исходников, отображаемое имя.
- `redaction` — `redactor` и `StreamRedactor` вычищают строки подключения,
  пароли, токены и секреты из захваченного вывода процессов.

#### `infrastructure::native_operations`

Этот модуль больше не владеет Platform XML. Он остаётся тонким host-фасадом:

- `registry` разбирает публичные MCP-аргументы в закрытые semantic commands,
  вызывает neutral reader/writer/validator ports и отображает typed result в
  `AdapterOutcome`;
- `meta` отображает neutral validation/navigation DTO в публичное представление,
  не разбирая XML;
- `code` интерпретирует и генерирует только BSL, а module artifact получает и
  публикует через opaque locator/write lease Platform XML adapter;
- `compile_transaction` — host-фасад для BSL/application artifacts, который не
  владеет lock, staging или rollback и делегирует общей adapter publication;
- `common` содержит только разбор MCP-аргументов и нейтральную работу с путями;
- `text_snapshot` сохраняет BOM/EOL семантику BSL; `typed_result` соединяет
  типизированные результаты с application.

CF/CFE, metadata, forms, templates, help, interfaces, roles, subsystems,
support, DCS, MXL и external artifacts реализованы в
`crates/unica-adapter-platform-xml/src/operations/` и приватных
`versions/v2_20` writer modules. Layout, XML tags, namespaces, version literals,
source/destination topology и parser/serializer не входят в compiled host
boundary (INV-SOURCE-FAMILY-PRIVATE-NATIVE,
INV-SOURCE-TYPED-WRITER-BOUNDARY). Provider-neutral code intelligence mainline
остаётся отдельным сервисом application и не проходит через format adapter.

Добавление format-aware операции меняет core semantic command/result, family
adapter implementation, host mapping, capability/authorability policy и
семантические preservation tests. Оно не добавляет XML implementation в
`native_operations` и не создаёт второй publisher в host.

#### `infrastructure::platform`

Единственное место, где допустима специфика операционной системы
(ADR-0009, INV-PLATFORM-OS-BEHIND-FACADE).

- `entrypoint` — `run_platform_main`, который даёт главному потоку в Windows
  стек 8 МиБ.
- `filesystem` — атомарная замена, синхронизация родительского каталога,
  идентичность файла, распознавание ссылок и точек повторного разбора, работа с
  префиксом расширенной длины пути.
- `process` — `ManagedChild`, `ManagedCommand` и
  `cancel_runtime_job_process_tree`: дочерний процесс принадлежит нам целиком,
  вместе со своим деревом (INV-PLATFORM-NO-ORPHAN-PROCESSES).
- `target` — `current_target_id`, отображение операционной системы и
  архитектуры на поддерживаемые идентификаторы целей.
- `full_dump_publication` — публикация синхронной полной выгрузки конфигурации
  под стражем.
- `testing` — помощники под `cfg(test)` для фикстур ссылок и прав.

Тесты платформенного поведения живут рядом с адаптерами, в
`crates/<crate>/tests/platform/` (INV-PLATFORM-COLOCATED-TESTS).

### Правила зависимостей между слоями

Направление зависимостей нормировано в INV-APP-DEPENDENCY-DIRECTION (ADR-0009, ADR-0002),
запрет `infrastructure -> interfaces` — в INV-APP-NO-ADAPTER-BYPASS (ADR-0002,
ADR-0003). Оба проверяет один страж
[`scripts/ci/check-rust-platform-boundary.py`](../../scripts/ci/check-rust-platform-boundary.py),
который исполняется тестами
[`tests/ci/test_rust_platform_boundary.py`](../../tests/ci/test_rust_platform_boundary.py).
Фактические разрешения:

| Слой | Может ссылаться на | Не может ссылаться на |
| --- | --- | --- |
| `domain` | только на себя и внешние крейты | `application`, `infrastructure`, `interfaces`; `std::fs`, `std::env`, `std::process`; ввод-вывод через `Path` |
| `application` | `domain` | `infrastructure`, `interfaces` |
| `infrastructure` | `domain`, `application` | `interfaces` |
| `interfaces` | `application`, `domain`, `infrastructure` | стражем не ограничен |

Асимметрия таблицы намеренна. Страж запрещает те направления, которые ломают
инверсию зависимостей: `domain` не знает никого, `application` не знает своих
адаптеров. Связывающие направления разрешены: `infrastructure` реализует порты
приложения и потому ссылается на `application` и `domain`, а `interfaces` для
двух скрытых режимов (`--workspace-service`, `--runtime-job-worker`) вызывает
инфраструктуру напрямую — эти режимы не проходят через диспетчер инструментов.

`interfaces` при этом остаётся неизвестен всем трём нижним слоям, и для
`infrastructure` у этого запрета своё основание: адаптер, дотянувшийся до слоя
представления, отрисовал бы ответ MCP сам и по дороге наружу обошёл бы отчёт о
кеше, который ведёт application (INV-APP-NO-ADAPTER-BYPASS). Это не «в коде
таких ссылок нет», а проверяемый запрет: страж возвращает
`infrastructure must not reference crate::interfaces`.

Дополнительно тот же страж требует, чтобы специфика операционной системы
(`cfg(windows)`, `cfg(unix)`, `cfg(target_*)`, `windows_sys`) встречалась только
внутри платформенных фасадов
`crates/unica-coder/src/infrastructure/platform/` и
`crates/unica-bootstrap/src/platform/` либо в платформенных тестах; исключений
по путям у стража нет (INV-PLATFORM-NO-PATH-EXEMPTIONS).

Тот же страж удерживает и границу хоста. Host-маркеры — имена хостов, каталоги
манифестов `.codex-plugin` и `.claude-plugin`, переменные окружения
`CODEX_HOME`, `CLAUDE_PLUGIN_DATA` и `CLAUDE_PLUGIN_ROOT` — допускаются только
внутри host-фасада `crates/unica-bootstrap/src/host/` и в host-тестах
`crates/<crate>/tests/host/`, поэтому в `unica-coder` их нет ни в одном слое
(INV-HOST-NEUTRAL-ORCHESTRATOR, INV-HOST-KNOWLEDGE-BEHIND-FACADE).

`infrastructure` не рендерит MCP-ответы и не обходит отчётность кеша: он
доступен приложению только через трейт `ApplicationPorts` (INV-APP-NO-ADAPTER-BYPASS), а
продуктовое связывание происходит только в `composition.rs`.

### Внешняя граница стандартов

`StandardsAdapter` — работающий HTTP-клиент MCP, а не заглушка. Он строит
конверт JSON-RPC 2.0 `tools/call`, отправляет его POST-запросом (таймаут 30 с,
`Accept: application/json, text/event-stream`) и нормализует ответ, включая
тело в формате SSE. Эндпоинт берётся из `UNICA_STANDARDS_MCP_URL`, по умолчанию
`https://ai.v8std.ru/mcp`.

Отображение операций на методы удалённого сервера задано по порядку: `search`
идёт в `v8std_search`; `explain` с аргументом `codes` — в
`v8std_explain_diagnostics`, со `snippet` — в `v8std_explain_snippet`, с `id`
или `idOrAliasOrUrl` — в `v8std_get_page`, а с одним лишь `query` — снова в
`v8std_search`. `explain` без единого из этих аргументов отклоняется. Наружу
этот сервер не выставляется: он остаётся внутренним адаптером за
`unica.standards.*` (INV-MCP-NO-ENGINE-SERVERS, INV-SKILL-NO-ADAPTER-TARGETS).

## Уровень 2 — `unica-bootstrap`

CLI принимает ровно две команды: `run --plugin-root <path>` и
`verify --plugin-root <path>`, плюс отдельный `--version`.

- `manifest` — `RuntimeManifest`, `TargetRuntime`, `RuntimeAsset`,
  `RuntimeFile`, `ReleaseIdentity`, `SourceIdentity`: метаданные закреплённого
  релиза, загружаемые из `runtime-manifest.json`.
- `cache` — `RuntimeInstaller::ensure`: исключительная блокировка на пару
  «версия и цель», транзакционный каталог с именем UUID, запись готовности
  `.ready.json` и атомарное переименование в
  `<cacheRoot>/<pluginVersion>/<target>` (INV-PKG-VERIFIED-ATOMIC-INSTALL, INV-CACHE-RUNTIME-ROOT-ORDER).
- `download` — `HttpDownloader`, единственный сетевой клиент крейта.
- `archive` — `sha256_file`, `extract_verified_tar_gz`, `verify_runtime_files`:
  отпечаток архива, безопасное к обходу каталогов извлечение и пофайловая
  сверка отпечатков.
- `verification` — `verify_mcp_runtime` выполняет по stdio MCP-вызовы
  `initialize` и `tools/list` против установленного runtime и требует наличия
  трёх стабильных инструментов — `unica.project.status`,
  `unica.standards.search` и `unica.standards.explain` — прежде чем сообщить об
  успехе.
- `error` — `BootstrapError`, единый тип ошибки крейта.
- `platform` — фасад ADR-0009 этого крейта: `entrypoint`
  (`run_platform_main`), `filesystem` (`set_executable`), `process`
  (`launch_runtime`), `target` (`HostTarget::current`).
- `host` — host-фасад ADR-0014: единственное место крейта, которое знает имена
  хостов, их каталоги манифестов и их переменные окружения (INV-HOST-KNOWLEDGE-BEHIND-FACADE).
  Разобран ниже.

`verify` дополнительно проверяет установленный пакет до запуска runtime: каждый
каталог `skills/<dir>` обязан содержать `SKILL.md`, набор видимых модели скиллов
обязан включать `code-search`, `platform-help`, `release-support` и `v8-runner`,
а пакет обязан нести манифест каждого известного хоста: один каталог плагина
обслуживает оба, и пакет без одного из манифестов просто не загрузится у этого
хоста. Каждый манифест обязан объявлять имя `unica` и версию крейта
(INV-PKG-VERSION-LOCKSTEP). Сверх этого оба поля обнаружения проверяются в обе
стороны: манифест Codex обязан объявлять указатели `skills: "./skills/"` и
`mcpServers: "./.mcp.json"`, потому что Codex не сканирует пакет сам, а манифест
Claude Code — не объявлять ни одного из них, потому что Claude Code сканирует
`skills/` и корневой `.mcp.json` сам и иначе загрузил бы каждый дважды. Различия между хостами приходят сюда из дескрипторов host-фасада
— сам цикл проверки о конкретном хосте не знает.

### Host-фасад `unica-bootstrap`

Хост описан дескриптором-данными, а не ветвлением в месте вызова: фасад несёт
реестр дескрипторов, и вызывающий код перебирает его целиком. Поэтому поддержка
третьего хоста — добавление дескриптора и его тестов, а не правка `main.rs`
(INV-HOST-UNIFORM-CALL-SITES). Приём тот же, что у `target` в платформенном фасаде, где выбор
сведён к данным и `match` по варианту.

Дескриптор отвечает за две вещи. Первая — каталог манифеста хоста и контракт
этого манифеста: `.codex-plugin` требует указателя `skills: "./skills/"`,
`.claude-plugin` требует его отсутствия. Вторая — источник корня кеша runtime:
переменная окружения хоста и сегменты пути, которые к ней дописываются.

Общий override корня кеша дескрипторам не принадлежит и стоит перед ними:
`UNICA_RUNTIME_CACHE_DIR` выставляет пакет, а не хост, и значение с
неразвёрнутой подстрокой `${` отбрасывается, после чего цепочка продолжается
(INV-CACHE-RUNTIME-ROOT-ORDER). Полный порядок разрешения описан в
[развёртывании](deployment.md).

Наружу фасад отдаёт только host-нейтральные типы, поэтому оркестратор
`unica-coder` о хостах не знает вовсе и доходит до корня плагина через
host-нейтральную `UNICA_PLUGIN_ROOT` (INV-HOST-NEUTRAL-ORCHESTRATOR). Тесты фасада живут рядом с
ним, а host-специфичный тест верхнего уровня — под
`crates/<crate>/tests/host/` (INV-HOST-KNOWLEDGE-BEHIND-FACADE).
