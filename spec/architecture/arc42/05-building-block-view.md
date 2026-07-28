# 5. Представление строительных блоков

Глава описывает фактическую структуру кода: какие крейты существуют, из каких
модулей они собраны и кто на кого имеет право ссылаться. Нормативные правила
здесь не дублируются — они принадлежат
[реестру инвариантов](../invariants.md) и цитируются по ID.

## Уровень 1 — Cargo workspace

Корневой `Cargo.toml` объявляет ровно два члена workspace.

| Крейт | Библиотека | Бинарник | Назначение |
| --- | --- | --- | --- |
| `crates/unica-coder` | `unica_coder` | `unica` | The orchestrator: it serves the single public MCP surface, dispatches every `unica.*` tool, and owns workspace cache state. |
| `crates/unica-bootstrap` | `unica_bootstrap` | `unica-bootstrap` | The thin launcher shipped in the public package: it resolves the pinned runtime, publishes it into a verified cache, and hands stdio to `unica`. |

`unica` — единственный бинарник, который видит хост как MCP-сервер
(INV-MCP-01, INV-MCP-02); `unica-bootstrap` в публичном пакете стоит перед ним
и сам MCP-сервером не является (INV-PKG-02).

`unica` не имеет подкоманд. `main.rs` разбирает аргументы в фиксированном
порядке: `--workspace-service`, затем `--runtime-job-worker`, затем
`--help`/`-h`, иначе — stdio MCP-сервер. Первые два — скрытые внутренние
режимы (INV-APP-07), см. [главу 6](06-runtime-view.md).

## Уровень 2 — `unica-coder`

`lib.rs` объявляет четыре слоя и делает `infrastructure` крейт-приватным:
публичны только `application`, `domain`, `interfaces` и реэкспорт
`run_platform_main`. `composition.rs` — единственный композиционный корень:
`UnicaApplication::new()` собирает приложение поверх
`InfrastructureApplicationPorts`.

### Слой `domain`

Чистая модель без ввода-вывода.

- `cache` — `CacheAccess`, `CacheImpact`, `CacheReport`; maps emitted events
  onto invalidated and eagerly refreshed cache names.
- `cancellation` — `CancellationToken` and the shared `cancelled:` error
  prefix used by every cancellable operation.
- `events` — `DomainEvent` and `DomainEventKind`, the typed facts a mutating
  operation emits.
- `form_edit` — the managed-form edit definition schema and its validation
  rules.
- `format_profile` — the active export format profile plus root-version
  classification and compatibility ordering.
- `project_sources` — the source-set model: `ProjectSourceMap`,
  `ProjectSourceSet`, `SourceFormat`, `SourceSetKind`.
- `source_roots` — `ResolvedSourceRoot` and the deterministic default
  source-set selection shared by every source-root consumer (INV-SOURCE-05).
- `workspace` — `WorkspaceContext`: a passive record of `cwd`,
  `workspace_root`, `cache_root`, and `workspace_epoch`.

`WorkspaceContext` ничего не обнаруживает: это структура данных. Обнаружение
рабочего пространства выполняет `infrastructure::workspace::discover_workspace`,
потому что домену запрещён доступ к файловой системе и окружению (ADR-0009,
INV-APP-05).

### Слой `application`

Транспортно-нейтральная оркестрация (ADR-0002).

- `mod` — `UnicaApplication`, `ToolSpec`, `ToolHandler`, `RuntimeJobAction`,
  `OperationResult`, the canonical registry `tools()`, and the
  `call_tool` / `call_tool_cancellable` dispatcher.
- `tool_contracts` — JSON input schemas, path-alias normalization, and
  argument validation for every registered tool (INV-MCP-05).
- `operation_descriptors` — descriptors of native operations, including
  support-guard and format-guard policies and native path alias groups.
- `ports` — the `ApplicationPorts` trait plus `HandlerOutcome`,
  `SupportGuardCheck`, and `FormatGuardCheck`.
- `outcome` — `AdapterOutcome`, the shape every adapter returns before the
  application renders `OperationResult`.

`call_tool` выполняет один и тот же порядок для любого инструмента:
normalize path aliases, resolve `dryRun`, validate arguments, discover the
workspace through the port, validate the tool context, evaluate the format
guard, dispatch the handler, evaluate the support guard for mutating tools,
emit domain events, and report cache impact.

### Слой `interfaces`

Три модуля — три способа запустить процесс.

- `mcp` — the public `unica` stdio MCP server on `rmcp` (ADR-0013):
  `run_stdio()` serves `rmcp::transport::stdio()` and implements
  `ServerHandler::list_tools` and `ServerHandler::call_tool` by delegating to
  `UnicaApplication` (INV-APP-02, INV-MCP-06).
- `workspace_service` — entry point of the hidden `--workspace-service` mode;
  it only forwards process arguments to
  `infrastructure::workspace_services::run_workspace_service_from_args`.
- `runtime_job_worker` — entry point of the hidden `--runtime-job-worker`
  mode; it only forwards to
  `infrastructure::runtime_jobs::run_worker_from_args`.

Оба служебных модуля — делегаты в несколько строк: они не содержат логики и не
регистрируются как публичные MCP-серверы (INV-APP-07).

### Слой `infrastructure`

Адаптеры, файловое состояние и всё, что знает об операционной системе.

- `application_ports` — `InfrastructureApplicationPorts`, the single
  implementation of `ApplicationPorts` wired by the composition root
  (INV-APP-03).
- `internal_adapters` — `CliAdapter`, `RuntimeAdapter`, `RuntimeJobAdapter`,
  `GitTrackingAdapter`, `CodeSearchAdapter`, `CodeNavigationAdapter`,
  `BslAnalyzerMcpAdapter`, and `StandardsAdapter`. `GitTrackingAdapter` is
  crate-private: `unica.project.status` and `unica.project.map` read git
  tracking state through it instead of spawning `git` themselves
  (INV-APP-06).
- `native_operations` — facade over the family-owned native XML/DSL
  operations, described below.
- `platform` — the ADR-0009 platform facade, described below.
- `workspace` — `discover_workspace` builds `WorkspaceContext`: it walks up from
  the working directory and stops at the first ancestor that carries
  `v8project.yaml` or a linked-worktree `.git` pointer file; a plain `.git`
  directory is not a marker, so an ordinary checkout without `v8project.yaml`
  keeps the working directory itself as the workspace root. It takes the cache
  root from `UNICA_CACHE_DIR` or `<workspaceRoot>/.build/unica` (INV-CACHE-03),
  and computes the workspace epoch from the root path, the size and
  modification time of `v8project.yaml`, `Configuration.xml`, and
  `src/Configuration.xml`, and the bytes of the git `HEAD` reached through the
  `.git` directory or the worktree pointer file, so a linked worktree stays
  isolated (INV-CACHE-06).
- `workspace_state` — `WorkspaceStateRepository` persists cache state and
  eager-refresh records under the cache root (INV-CACHE-01).
- `workspace_services` — `WorkspaceServiceManager` and
  `run_workspace_service_from_args`: the lifecycle of the hidden
  workspace-scoped service and its internal JSONL protocol (ADR-0006).
- `workspace_index` — lifecycle of the persistent BSL index built by the
  bundled `rlm-bsl-index`: readiness classification, lock ownership,
  heartbeats, and background build or update.
- `runtime_jobs` — durable runtime-job state shared by the runtime-job worker
  and the transport adapter: the job store, phase transitions, cancel markers,
  and bounded output tails.
- `bundled_tools` — `resolve_bundled_tool` locates a bundled executable through
  `third-party/manifest.json` for the current target and verifies its SHA-256
  before it is executed; a dry run may fall back to `tools.lock.json` and
  report a warning instead of failing.
- `plugin_runtime` — `find_plugin_root` resolves the plugin root, preferring
  `UNICA_PLUGIN_ROOT` and otherwise walking up from the executable and the
  working directory.
- `tool_context` — `validate_tool_context` rejects a call whose paths leave the
  workspace or whose source set does not carry the format the operation
  requires (INV-SOURCE-04).
- `path_policy` — `WorkspacePathPolicy`, the shared containment rule for
  resolved paths.
- `support_guard` — `evaluate_support_guard` blocks or warns a mutating
  operation according to the support state of the target object.
- `format_guard` — `evaluate_format_guard` compares the export format of the
  affected source against the active format profile.
- `project_sources` — `discover_project_source_map` builds the source-set map
  of the workspace from `v8project.yaml` and the physical inventory.
- `source_roots` — `resolve_source_root` and `normalize_path_identity`, the
  deterministic source-root selection used by analyzer and index consumers.
- `platform_xml_owner` — resolves which platform XML file owns a metadata
  object and reports the provenance of that decision.
- `metadata_kinds` — the static table of metadata kinds: XML tag, source
  directory, and display name.
- `redaction` — `redactor` and `StreamRedactor` remove connection strings,
  passwords, tokens, and secrets from captured process output.

#### `infrastructure::native_operations`

`NativeOperationAdapter` — тонкий фасад: он выбирает между dry-run-предпросмотром,
типизированной мутацией и чтением, а сама операция принадлежит модулю своего
семейства.

- Families: `cf`, `cfe`, `code`, `dcs`, `external`, `form`, `help`,
  `interface`, `meta`, `mxl`, `role`, `subsystem`, `support`, `template`.
- `registry` — the dispatch table that maps an operation id onto its preview,
  mutation, or read handler.
- `common` — shared XML reading, identifier escaping, and generic analysis.
- `compile_transaction` — failure-atomic publication for the metadata compile
  writers.
- `single_file_publisher` — failure-atomic publication of one exact file
  payload.
- `text_snapshot` — exact byte snapshot of a source file with its BOM and
  end-of-line profile.
- `form_event_registry` — registry and validation rules for managed-form event
  bindings.
- `meta_validation_context` — the cross-object context a metadata validation
  needs.
- `typed_result` — `NativeOperationResult`, the typed adapter result that
  carries structured `data` next to `AdapterOutcome`.

Все операции конфигурации, расширений, форм, DCS, MXL, ролей, подсистем,
командного интерфейса, справки, макетов и внешних обработок реализованы нативно
на Rust за инструментами `unica.*`.
The runtime carries no operation-file fallback: no legacy script handler and no
production spawn of an interpreter (INV-APP-04). Reference models written in
Python or PowerShell exist only as test fixtures (INV-SKILL-04).

#### `infrastructure::platform`

Единственное место, где допустима ОС-специфика (ADR-0009, INV-PLATFORM-01).

- `entrypoint` — `run_platform_main`, which gives the Windows main thread an
  8 MiB stack.
- `filesystem` — atomic replacement, parent-directory sync, file identity,
  link and reparse-point detection, extended-length prefix handling.
- `process` — `ManagedChild`, `ManagedCommand`, and
  `cancel_runtime_job_process_tree`: a child is owned as a whole process tree
  (INV-PLATFORM-04).
- `target` — `current_target_id`, the mapping from OS and architecture onto the
  supported target ids.
- `full_dump_publication` — guarded publication for synchronous full
  configuration dumps.
- `testing` — `cfg(test)` helpers for link and permission fixtures.

Тесты платформенного поведения живут рядом с адаптерами, в
`crates/<crate>/tests/platform/` (INV-PLATFORM-03).

### Правила зависимостей между слоями

Направление зависимостей нормировано в INV-APP-05 (ADR-0009, ADR-0002) и
проверяется скриптом
[`scripts/ci/check-rust-platform-boundary.py`](../../../scripts/ci/check-rust-platform-boundary.py),
который исполняется тестами
[`tests/ci/test_rust_platform_boundary.py`](../../../tests/ci/test_rust_platform_boundary.py).
Фактические разрешения:

| Слой | Может ссылаться на | Не может ссылаться на |
| --- | --- | --- |
| `domain` | только на себя и внешние крейты | `application`, `infrastructure`, `interfaces`; `std::fs`, `std::env`, `std::process`; `Path`-ввод-вывод |
| `application` | `domain` | `infrastructure`, `interfaces` |
| `infrastructure` | `domain`, `application` | стражем не ограничен; ссылок на `interfaces` в коде нет |
| `interfaces` | `application`, `domain`, `infrastructure` | стражем не ограничен |

Асимметрия таблицы намеренна. Страж запрещает только те направления, которые
ломают инверсию зависимостей: `domain` не знает никого, `application` не знает
своих адаптеров. Обратные направления разрешены, потому что именно они и есть
связывание: `infrastructure` реализует порты приложения, а `interfaces` для
двух скрытых режимов (`--workspace-service`, `--runtime-job-worker`) вызывает
инфраструктуру напрямую — эти режимы не проходят через диспетчер инструментов.

Дополнительно тот же страж требует, чтобы OS-специфика (`cfg(windows)`,
`cfg(unix)`, `cfg(target_*)`, `windows_sys`) встречалась только внутри
платформенных фасадов `crates/unica-coder/src/infrastructure/platform/` и
`crates/unica-bootstrap/src/platform/` либо в платформенных тестах; исключений
по путям у стража нет (INV-PLATFORM-02).

`infrastructure` не рендерит MCP-ответы и не обходит отчётность кеша: он
доступен приложению только через трейт `ApplicationPorts` (INV-APP-03), а
production-связывание происходит только в `composition.rs`.

### Внешняя граница стандартов

`StandardsAdapter` — работающий HTTP-клиент MCP, а не заглушка. Он строит
конверт JSON-RPC 2.0 `tools/call`, отправляет его POST-запросом (таймаут 30 с,
`Accept: application/json, text/event-stream`) и нормализует ответ, включая
SSE-тело. Эндпоинт берётся из `UNICA_STANDARDS_MCP_URL`, по умолчанию
`https://ai.v8std.ru/mcp`. Маппинг операций: `search` и `explain` с `query` —
на `v8std_search`; `explain` с `codes` — на `v8std_explain_diagnostics`; с
`snippet` — на `v8std_explain_snippet`; с `id` или `idOrAliasOrUrl` — на
`v8std_get_page`. Наружу этот сервер не выставляется: он остаётся внутренним
адаптером за `unica.standards.*` (INV-MCP-01, INV-SKILL-02).

## Уровень 2 — `unica-bootstrap`

CLI принимает ровно две команды: `run --plugin-root <path>` и
`verify --plugin-root <path>` (плюс `--version`).

- `manifest` — `RuntimeManifest`, `TargetRuntime`, `RuntimeAsset`,
  `RuntimeFile`, `ReleaseIdentity`, `SourceIdentity`: the pinned release
  metadata loaded from `runtime-manifest.json`.
- `cache` — `RuntimeInstaller::ensure`: an exclusive per-version and per-target
  lock, a UUID transaction directory, a `.ready.json` readiness record, and an
  atomic rename into `<cacheRoot>/<pluginVersion>/<target>` (INV-PKG-03,
  INV-CACHE-07).
- `download` — `HttpDownloader`, the only network client of the crate.
- `archive` — `sha256_file`, `extract_verified_tar_gz`, `verify_runtime_files`:
  archive digest, traversal-safe extraction, and per-file digest verification.
- `verification` — `verify_mcp_runtime` performs MCP `initialize` and
  `tools/list` over stdio against the installed runtime and requires the three
  stable tools before reporting success.
- `error` — `BootstrapError`, the crate-wide error type.
- `platform` — the ADR-0009 facade of this crate: `entrypoint`
  (`run_platform_main`), `filesystem` (`set_executable`), `process`
  (`launch_runtime`), `target` (`HostTarget::current`).

`verify` дополнительно проверяет установленный пакет до запуска рантайма:
каждый каталог `skills/<dir>` обязан содержать `SKILL.md`, набор
prompt-visible-скиллов обязан включать `code-search`, `platform-help`,
`release-support` и `v8-runner`, а пакет обязан нести хотя бы один манифест
хоста — Codex, Claude Code или оба. Каждый присутствующий манифест обязан
объявлять имя `unica` и версию крейта (INV-PKG-05); сверх этого
Codex-манифест обязан объявлять указатель `skills: "./skills/"`, а
Claude-манифест — не объявлять ключ `skills` вовсе, потому что Claude Code
сканирует `skills/` сам и иначе загрузил бы каталог дважды.
