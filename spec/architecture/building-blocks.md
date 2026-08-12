# Строительные блоки

Документ описывает фактическую структуру кода: какие крейты существуют, из каких
модулей они собраны, кто на кого имеет право ссылаться и где физически лежит
реализация публичных операций. Нормативные правила здесь не дублируются — они
принадлежат [реестру инвариантов](invariants.md) и цитируются по ID. Поведение
тех же блоков во времени описано в [runtime](runtime.md).

## Уровень 1 — Cargo workspace

Корневой `Cargo.toml` объявляет ровно двух членов workspace; оба наследуют одну
версию из `[workspace.package]`.

| Крейт | Библиотека | Бинарник | Назначение |
| --- | --- | --- | --- |
| `crates/unica-coder` | `unica_coder` | `unica` | Оркестратор: обслуживает единственную публичную MCP-поверхность, диспетчеризует все инструменты `unica.*` и владеет состоянием кеша рабочего пространства. |
| `crates/unica-bootstrap` | `unica_bootstrap` | `unica-bootstrap` | Тонкий пусковой бинарник публичного пакета: разрешает закреплённый runtime, публикует его в проверенный кеш и передаёт ему stdio. |

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
| `unica.cf.*` | контейнер конфигурации: создание, чтение, правка, валидация | `infrastructure/native_operations/cf.rs` |
| `unica.cfe.*` | расширения конфигурации: создание, заимствование, перехват, сравнение | `native_operations/cfe.rs` |
| `unica.meta.*` | создание, чтение, типизированная правка и удаление объектов метаданных | координатор `application/metadata.rs` поверх `infrastructure/metadata_operations.rs`; индексное обогащение `meta.info` остаётся приватной возможностью `infrastructure/rlm_navigation.rs` (ADR-0025, INV-MCP-META-SURFACE) |
| `unica.form.*` | управляемые формы | `native_operations/form.rs` |
| `unica.dcs.*` | схемы компоновки данных | `native_operations/dcs.rs` |
| `unica.mxl.*` | табличные документы | `native_operations/mxl.rs` |
| `unica.role.*` | роли и права доступа | `native_operations/role.rs` |
| `unica.subsystem.*` | подсистемы | `native_operations/subsystem.rs` |
| `unica.interface.*` | командный интерфейс подсистемы | `native_operations/interface.rs` |
| `unica.template.*` | макеты, присоединённые к объекту метаданных | `native_operations/template.rs` |
| `unica.help.*` | встроенная справка объекта | `native_operations/help.rs` |
| `unica.support.*` | состояние поддержки конфигурации и её объектов | `native_operations/support.rs` |
| `unica.epf.*`, `unica.erf.*` | заготовки внешних обработок и внешних отчётов | `native_operations/external.rs` |
| `unica.build.*` | выгрузка, загрузка, обновление, сборка и запуск через платформу | `RuntimeAdapter` в `infrastructure/internal_adapters.rs` поверх поставляемого `v8-runner` |
| `unica.runtime.*` | типизированные сценарии runtime и долгоживущие задания | `RuntimeAdapter` и `RuntimeJobAdapter` в `internal_adapters.rs`, состояние — `infrastructure/runtime_jobs.rs` |
| `unica.code.*` | поиск, навигация, правка и диагностика BSL | поиск и навигация — реестр поставщиков в `infrastructure/code_intelligence.rs`; граф и диагностика — адаптер анализатора в `internal_adapters.rs`; `unica.code.patch` — `native_operations/code.rs` |
| `unica.source.*` | логическое разрешение и обход целей, снимки ресурсов, ограниченное чтение и запасная полная замена BSL | `application/source_navigation.rs` и `application/source_resources.rs` через `infrastructure/platform_xml_source_targets.rs` и `infrastructure/platform_xml_resources.rs` |
| `unica.standards.*` | знания о стандартах разработки 1С | `StandardsAdapter` в `internal_adapters.rs` |

Внутренние границы, до которых дотягиваются адаптеры: поставляемый инструмент
runtime для операций платформы, поставляемый анализатор BSL и индекс кода,
типизированный `git grep` как одна секция поиска, внутрипроцессные нативные
операции над XML и DSL, удалённый эндпоинт стандартов по HTTP. Ни одна из них не
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
- `format_profile` — активный профиль формата выгрузки, классификация версии
  корня и порядок совместимости.
- `operational_config` — закрытая неизменяемая модель `OperationalConfig`,
  типизированные сроки операций с кодом: файловые значения в целых секундах
  не меньше 1 без верхнего ограничения операционной политики, умолчания
  120/45/15/45/120 и безопасная форма диагностики без файлового ввода-вывода
  (INV-APP-CONFIG-SNAPSHOT).
- `project_sources` — модель наборов исходников: `ProjectSourceMap`,
  `ProjectSourceSet`, `SourceFormat`, `SourceSetKind`.
- `source_roots` — `ResolvedSourceRoot` и детерминированный выбор набора
  исходников по умолчанию, общий для всех потребителей корня исходников
  (INV-SOURCE-SINGLE-RESOLVED-ROOT).
- `source_target` — `SourceTarget`, `MetadataAddress`, `ResolvedTarget` и
  профиль канонических адресов Platform XML (INV-SOURCE-LOGICAL-IDENTITY).
- `support_state` — провайдер-нейтральные состояния конфигурации и объекта,
  стабильные ошибки, логическая цель вложенной подсистемы и малый
  `SupportStateReader` без файловых путей (INV-APP-SUPPORT-STATE).
- `subsystem` — отдельный от `MetadataAddress` тип `SubsystemAddress` в плоском
  диалекте БСП, эффективная роль узла и закрытая идентичность
  `ContentReference` как `MetadataAddress | UUID` (INV-SOURCE-SUBSYSTEM-TOPOLOGY).
- `source_resources` — роли, полнота, публичные манифесты, результаты чтения и
  замены и стабильные коды ошибок ресурсного доступа
  (INV-SOURCE-SNAPSHOT-BINDING, INV-SOURCE-ROLE-ALLOWLIST).
- `workspace` — `WorkspaceContext`: пассивная запись о `cwd`, `workspace_root`,
  `cache_root` и `workspace_epoch`.

`WorkspaceContext` ничего не обнаруживает: это структура данных. Обнаружение
рабочего пространства выполняет `infrastructure::workspace::discover_workspace`,
потому что домену запрещён доступ к файловой системе и окружению
(ADR-0009, INV-APP-DEPENDENCY-DIRECTION).

### Слой `application`

Транспортно-нейтральная оркестрация (ADR-0002).

- `mod` — `UnicaApplication`, `ToolSpec`, категории `ToolExecution`, контракты
  `ResultContract`, режимы вызова `InvocationMode`, `ToolHandler`,
  `RuntimeJobAction`, `OperationResult`, канонический реестр `tools()` и
  диспетчер `call_tool` / `call_tool_cancellable`.
- `operational_config` — выбор затронутых вызовов, наложение явного срока
  диагностики и сборка одного эффективного снимка через порт приложения
  (INV-APP-CONFIG-SNAPSHOT).
- `code_intelligence` — `CodeSearchCoordinator` и общий исполнитель операций
  чтения: параллельный запуск поставщиков, сроки, ограничение допущенных
  исполнителей, отмена, сохранение порядка секций и политика частичного успеха
  (INV-MCP-CODE-SEARCH-SECTIONS, INV-MCP-BOUNDED-ADMISSION).
- `source_navigation` — запросы и результаты
  `unica.source.resolve`/`unica.source.children`, ограничение выдачи и
  типизированные адресуемые либо наблюдаемые местоположения.
- `source_resources` — оркестрация читающих `resources/read` и разбор
  ограниченных аргументов (INV-MCP-SOURCE-SURFACE).
- `tool_contracts` — входные JSON-схемы, нормализация алиасов путей и проверка
  аргументов для каждого зарегистрированного инструмента (INV-MCP-DATA-DRIVEN-SCHEMA).
- `operation_descriptors` — описатели нативных операций, включая политики
  стража поддержки и стража формата и группы алиасов путей.
- `ports` — трейт `ApplicationPorts` и типы `HandlerOutcome`,
  `SupportGuardCheck`, `FormatGuardCheck`; порт загрузки операционной
  конфигурации принимает уже обнаруженный `WorkspaceContext`.
- `outcome` — `AdapterOutcome`, форма, которую возвращает любой адаптер до того,
  как приложение соберёт `OperationResult`.

`call_tool` выполняет один и тот же порядок для любого инструмента: нормализует
алиасы путей, проверяет аргументы, выводит `InvocationMode`, проверяет
семантику режима и только затем обнаруживает рабочее пространство через порт.
Читатель не принимает `dryRun` и получает режим `Read`; мутация без `dryRun`
либо с истинным значением получает `Preview`, а с `dryRun: false` — `Apply`
(INV-MCP-PREVIEW-MUTATION-ONLY). После обнаружения приложение проверяет контекст
вызова, оценивает страж формата и пропускает вызов через стражи маршрута runtime
и синхронизации выгрузки. Для мутирующего инструмента оно оценивает страж
поддержки. Перед обработчиком четыре
затронутые операции с кодом разрешают один `OperationalConfig`; остальные
вызовы не разрешают его и не читают `[operational]` (сетевые потребители
документации и стандартов независимо читают `network` и `providers` по
`INV-APP-DOCUMENTATION-NETWORK-POLICY`). Затем приложение вызывает
обработчик и немедленно проверяет постусловие результата: успешный
`Read + Typed` без `data` отклоняется как `typed_result_missing`. Только после
этой проверки приложение собирает доменные события и отчитывается о влиянии на
кеш. Страж
поддержки и отказ конфигурации срабатывают до обработчика, а не после:
заблокированный вызов не доходит даже до планирования или запуска процесса.

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
- `workspace_config` — общий разбор каждого присутствующего слоя
  `unica.toml` и `unica.local.toml` по единой закрытой схеме: синтаксис TOML,
  только поля `operational`, `network`, `providers`; формат не версионируется,
  а `version` отвергается как неизвестное корневое поле
  (INV-APP-CONFIG-SNAPSHOT, INV-APP-DOCUMENTATION-NETWORK-POLICY).
- `operational_config` — чтение слоёв от `workspaceRoot`, проверка только
  поддерева `[operational]` и отображение его ошибок в диагностику без
  абсолютного пути и сырого значения; ошибка `network` или `providers` не
  меняет операционный снимок (INV-APP-CONFIG-SNAPSHOT,
  REQ-SAFETY-SECRET-REDACTION).
- `documentation_policy` — проверка только поддеревьев `network` и
  `providers` поверх общего корня; ошибка `[operational]` не меняет сетевую
  политику (INV-APP-DOCUMENTATION-NETWORK-POLICY).
- `rlm_navigation` — типизированные definition, outline и object-profile поверх
  поддерживаемой MCP-сессии RLM без чтения частной SQLite-схемы; проверка
  готовности и операция расходуют полный бюджет вызывающей стороны без
  120-секундной верхней границы.
- `native_operations` — фасад над нативными операциями над XML и DSL,
  разложенными по семействам; разобран ниже.
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
  сессия RLM и внутренний протокол JSONL с `schema_version = 4`. Обычные и
  управляющие пути сохраняют свои умолчания, а рабочие запросы анализатора и
  RLM с бюджетом вызывающей стороны передают остаток без потерь полями
  `timeout_seconds` и `timeout_nanos` и не ограничивают его 120 секундами
  (ADR-0018, ADR-0040).
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
- `support_state` — workspace-bound реализация `SupportStateReader`: разрешает
  логическую цель, выбирает поставщика по формату источника и скрывает
  Platform XML marker, descriptor UUID и физическую раскладку
  (INV-APP-SUPPORT-STATE).
- `format_guard` — `evaluate_format_guard` сравнивает формат выгрузки
  затронутого источника с активным профилем формата.
- `project_sources` — `discover_project_source_map` строит карту наборов
  исходников рабочего пространства из `v8project.yaml` и физической
  инвентаризации.
- `source_roots` — `resolve_source_root` и `normalize_path_identity`,
  детерминированный выбор корня исходников для анализатора и индекса.
- `subsystem_topology` — снимок, выводимый из регистраций под одним удерживаемым
  корнем, открытым без перехода по символическим ссылкам: он читает только `Configuration.xml` и транзитивно
  зарегистрированные дескрипторы, возвращает их точные зависимости формата,
  проверяет отмену и ограниченный срок, строит только дерево для `subsystem.info` и ролевые
  членства текущего дескриптора по адресу и UUID для `meta.info`
  (INV-SOURCE-SUBSYSTEM-TOPOLOGY).
- `platform_xml_source_targets` — отрисовка кандидата по логическому адресу,
  восстановление адреса по пути и закрытые ручки, которые каждый writer обязан
  повторно авторизовать (INV-SOURCE-LOGICAL-IDENTITY).
- `platform_xml_resources` — хранилище ограниченных снимков на срок жизни
  приложения, чтение точного диапазона байтов и безопасная замена одного
  BSL-ресурса (REQ-PERF-SOURCE-BOUNDS).
- `platform_xml_owner` — определяет, какой платформенный XML-файл владеет
  объектом метаданных, и сообщает происхождение этого решения.
- `metadata_kinds` — статическая таблица видов метаданных: XML-тег, каталог
  исходников, отображаемое имя.
- `redaction` — `redactor` и `StreamRedactor` вычищают строки подключения,
  пароли, токены и секреты из захваченного вывода процессов.

#### `infrastructure::native_operations`

`NativeOperationAdapter` — тонкий фасад в сотню строк. Он выбирает между
предпросмотром, типизированной мутацией, обычной мутацией и чтением, а сама
операция принадлежит модулю своего семейства.

- Семейства: `cf`, `cfe`, `code`, `dcs`, `external`, `form`, `help`,
  `interface`, `meta`, `mxl`, `role`, `subsystem`, `support`, `template`.
- `registry` — таблица маршрутизации: `invoke_read`, `invoke_preview` и
  `invoke_mutation` отображают идентификатор операции на обработчик семейства.
  Здесь же живёт `native_mutation_file_input_contract` — исчерпывающая
  классификация того, какие файловые входы выбирает вызывающая сторона у каждого
  публичного мутатора.
- `typed_result` — `NativeOperationResult` и `invoke_with_data`: путь нативных
  операций, которые возвращают структурированное поле `data` рядом с
  `AdapterOutcome`. Он разводит preview/apply у типизированных мутаций и всегда
  выполняет зарегистрированные типизированные чтения; обычный `invoke`
  отклоняет операции, обязанные идти путём с данными. Общее постусловие
  успешного `Read + Typed` проверяет application
  (INV-MCP-TYPED-RESULT, INV-MCP-PREVIEW-MUTATION-ONLY).
- `common` — общее чтение XML, экранирование идентификаторов и обобщённый
  разбор.
- `compile_transaction` — атомарная по отказу публикация для писателей,
  собирающих метаданные.
- `single_file_publisher` — атомарная по отказу публикация ровно одного файла с
  точным содержимым.
- `text_snapshot` — побайтовый снимок исходного файла вместе с его BOM и
  профилем перевода строки.
- `form_event_registry` — реестр и правила проверки привязок событий
  управляемой формы.
- `meta_validation_context` — межобъектный контекст, который нужен проверке
  метаданных.

Все операции над конфигурацией, расширениями, формами, схемами компоновки
данных, табличными документами, ролями, подсистемами, командным интерфейсом,
справкой, макетами и внешними обработками реализованы нативно на Rust за
инструментами `unica.*`. Пути исполнения через файл операции у runtime нет: нет
ни обработчика унаследованных скриптов, ни продуктового запуска интерпретатора
(INV-APP-NO-SCRIPT-BACKEND). Эталонные модели на Python и PowerShell существуют только как
эталоны паритета в тестах (INV-SKILL-SCRIPTS-AS-FIXTURES).

#### Где на самом деле лежит работа

Реестр `tools()` — это описание поверхности, а не реализация. Тело операции
живёт в модуле семейства, и именно там сосредоточен объём кода. Числа ниже —
снимок на момент написания; они нужны, чтобы соотнести масштабы, а не как
контракт.

| Модуль | Порядок величины, строк |
| --- | --- |
| `native_operations/meta.rs` | ~21 700 |
| `native_operations/form.rs` | ~18 800 |
| `native_operations/dcs.rs` | ~12 700 |
| `native_operations/cfe.rs` | ~10 800 |
| `native_operations/cf.rs` | ~4 800 |
| `native_operations/mxl.rs` | ~4 200 |
| `application/tool_contracts.rs` | ~4 000 |
| `native_operations/registry.rs` | ~400 |
| `native_operations.rs` (фасад) | ~120 |

Отсюда практическое следствие для того, кому поручено «добавить инструмент».
Запись в `application/mod.rs` — самая маленькая часть работы. Полный набор мест,
которые придётся тронуть у нативного инструмента:

1. `application/mod.rs` — запись `ToolSpec`: имя, описание, категория
   `ToolExecution`, контракт `ResultContract`, `CacheAccess` и вариант
   `ToolHandler`.
2. `application/tool_contracts.rs` — список допустимых и обязательных
   аргументов, входная JSON-схема и проверка значений. Схема данных, а не
   макросов SDK: инструмент без записи здесь останется без схемы и без проверки
   аргументов.
3. `application/operation_descriptors.rs` — `OperationDescriptor`: обязательные
   аргументы, аргументы-пути на запись и на чтение, политика разрешения путей
   формата, политика стража формата и политика стража поддержки. Без описателя
   мутация пройдёт мимо стражей.
4. `infrastructure/native_operations/registry.rs` — маршрут операции в модуль
   семейства и явная классификация файловых входов. Тест
   `every_native_mutator_has_an_explicit_file_input_contract` требует записи для
   каждого публичного мутатора и сверяет итоговый набор целиком.
5. `infrastructure/native_operations/<семейство>.rs` — собственно операция:
   чтение исходника снимком, разбор, изменение, атомарная публикация, сборка
   `AdapterOutcome`, а для типизированных операций — ещё и `data`.
6. `domain/events.rs` и `domain/cache.rs` — доменное событие мутации и правило,
   по которому оно превращается во влияние на кеш.
7. `tests/ci/test_unica_mcp_script_parity.py` — стенд паритета: добавление,
   удаление или переименование публичного инструмента синхронно меняет реестр в
   Rust, стенд и архитектурный слой (INV-MCP-SURFACE-SYNC).
8. Скилл в `plugins/unica/skills/`, если операция должна попасть в
   маршрутизацию: скилл описывает операцию разработчика, а не внутренний
   инвентарь инструментов (INV-PRODUCT-DEVELOPER-OPERATIONS, INV-SKILL-DECLARED-ROUTING).

Инструмент, который обслуживается адаптером, а не нативной операцией, вместо
шагов 4 и 5 требует ветки в `infrastructure/internal_adapters.rs`.

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
