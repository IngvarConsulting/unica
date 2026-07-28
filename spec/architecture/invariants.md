# Реестр архитектурных инвариантов

Этот документ — машинно-проверяемый реестр правил, которые должны оставаться
верными при развитии Unica. Каждая запись формулирует одно нормативное правило,
называет решение, из которого оно следует, и конкретную проверку, которая его
удерживает.

Реестр не описывает архитектуру и не заменяет arc42: он фиксирует то, что
нельзя сломать молча. Если изменение нарушает инвариант, сначала нужен новый
ADR, который явно заменяет или уточняет текущее решение; после этого правится
запись реестра и её проверка. Правка записи без ADR — дефект процесса, а не
редакторская работа.

## Как читать реестр

- Записи сгруппированы по областям; порядок областей — от границ продукта к
  документационному слою.
- Ссылки на решения даны по ID вида `ADR-NNNN`; действующий каталог решений —
  [spec/decisions/README.md](../decisions/README.md). Нормативный текст решения
  не копируется сюда, копируется только следствие, которое проверяется.
- Если правило нормировано в ADR, а проверки в репозитории нет, класс проверки
  — `manual` с честным описанием того, что именно проверяет человек.

## Как устроен реестр

Этот раздел описывает формат обоих реестров корпуса: реестра инвариантов
(`INV-*`, этот файл) и реестра требований к качеству (`REQ-*`,
[глава 10](arc42/10-quality-requirements.md)). Глава 10 ссылается сюда и не
повторяет формат.

Каждая запись оформлена одинаково. Заголовок записи —
`### <ID> — <короткое имя>`, где тире это U+2014, окружённое пробелами. Сразу
за заголовком идёт пустая строка и затем четыре поля-булета. Порядок ниже —
принятое оформление; тест проверяет наличие полей, а не их последовательность,
поэтому за порядком следит ревью:

- Поле `Rule` — ровно одно нормативное утверждение на английском, проверяемое
  кодом, тестом или ревью.
- Поле `Decision` — один ADR, список ADR через запятую либо литерал `n/a`.
- Поле `Check` — одна или несколько строк; в каждой сначала класс проверки,
  затем тире U+2014, затем цель, обе части в обратных кавычках.
- Поле `Scope` — контуры, в которых правило обязано выполняться.

Классы проверок:

| Класс | Что это | Что стоит в `<target>` |
| --- | --- | --- |
| `ci-test` | автоматический тест, исполняемый в CI (Python unittest или Rust `#[test]`) | путь к файлу с тестом |
| `guard-script` | скрипт-страж, исполняемый набором тестов или workflow | путь к скрипту |
| `doc-assert` | тест, который проверяет содержимое документации | путь к файлу с тестом |
| `release-gate` | шаг релизного конвейера, блокирующий публикацию | путь к скрипту или workflow |
| `manual` | ручная проверка при ревью | свободное описание |

Правила идентификаторов:

- ID соответствует `^(INV|REQ)-[A-Z][A-Z0-9]*-[0-9]{2}$`. Префикс `INV`
  принадлежит инвариантам, префикс `REQ` — требованиям к качеству.
- ID уникален во всём корпусе спецификаций и никогда не переиспользуется после
  удаления записи: удалённый номер остаётся выведенным из обращения.
- Область фиксирует владельца правила, а не файл, в котором оно проверяется.
  У каждого реестра свой набор областей, и наборы не пересекаются: инварианты
  используют `PRODUCT`, `MCP`, `SKILL`, `APP`, `CACHE`, `SOURCE`, `PKG`,
  `PLATFORM`, `CI`, `DOC`; требования к качеству — `PERF`, `TOKEN`, `SAFETY`,
  `OBS`, `MAINT`, `COMPAT`, `REL`. Новая область заводится вместе с первой
  записью, которая ей принадлежит, и добавляется в этот перечень.
- `Scope` перечисляет контуры, в которых правило обязано выполняться:
  `source` (рабочее дерево), `packaged` (сгенерированный пакет), `ci`
  (конвейер), `release` (публикация), `runtime` (исполнение).

## PRODUCT — границы продукта

### INV-PRODUCT-01 — One plugin directory serves two hosts

- **Rule:** Unica ships as one plugin directory that serves both Codex and
  Claude Code; `.mcp.json`, `skills/`, references, and the MCP boundary stay
  host-neutral, and only the manifest directories `.codex-plugin/` and
  `.claude-plugin/` are host-specific.
- **Decision:** ADR-0012
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Check:** `doc-assert` — `tests/ci/test_product_contracts.py`
- **Scope:** source, packaged

### INV-PRODUCT-02 — Public surface models developer operations

- **Rule:** Public skills and `unica.*` tools model 1C:Enterprise developer
  operations; infrastructure and packaging concerns stay out of the
  prompt-visible surface.
- **Decision:** ADR-0001, ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged, runtime

### INV-PRODUCT-03 — Bundled engines stay out of prompt-visible routing

- **Rule:** Prompt-visible skills and references must not instruct the model to
  invoke bundled low-level engines directly or name them as MCP servers; a
  domain tool may be mentioned conceptually, but never as a call target.
- **Decision:** ADR-0001, ADR-0005, ADR-0006
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged, runtime

### INV-PRODUCT-04 — Generated package is a first-class delivery

- **Rule:** Every public contract that holds in the source checkout also holds
  in the generated marketplace package, and package-level validation is
  required in addition to source-level validation.
- **Decision:** ADR-0001, ADR-0008
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Check:** `release-gate` — `scripts/ci/smoke-unica-bootstrap.py`
- **Scope:** packaged, release

### INV-PRODUCT-05 — One bundled-tool version authority

- **Rule:** `plugins/unica/third-party/tools.lock.json` is the authority for
  bundled tool versions, and a provenance record for a bundled tool points at it
  through `toolLockRef` instead of carrying its own version or baseline commit.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `tests/ci/test_skill_provenance.py`
- **Check:** `guard-script` — `scripts/ci/check-skill-upstreams.py`
- **Scope:** source, packaged, ci

### INV-PRODUCT-06 — DCS is the canonical data composition domain

- **Rule:** Active English identifiers for the data composition domain use
  `dcs`/`Dcs`/`DCS` in tools, skills, Rust modules, package metadata, and
  active documentation; the removed transliterated alias and the
  reversed-letter misspelling of the abbreviation must not reappear outside the
  explicitly allowed donor and platform-schema exceptions.
- **Decision:** ADR-0011
- **Check:** `ci-test` — `tests/ci/test_dcs_naming_contract.py`
- **Check:** `release-gate` — `scripts/ci/smoke-unica-mcp.py`
- **Scope:** source, packaged, runtime, release

## MCP — публичная MCP-поверхность

### INV-MCP-01 — `unica` is the only LLM-visible MCP server

- **Rule:** Internal engines (build/runtime, BSL analysis, code index,
  standards, XML/DSL operations) are reachable only through internal adapters
  and are never registered as separate public MCP servers.
- **Decision:** ADR-0001, ADR-0006
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged, runtime

### INV-MCP-02 — Single public MCP server

- **Rule:** `plugins/unica/.mcp.json` declares exactly one `mcpServers` entry
  named `unica`, in the source tree and in every generated package.
- **Decision:** ADR-0001
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** source, packaged

### INV-MCP-03 — Server identity on the wire

- **Rule:** `initialize` returns `serverInfo.name = "unica"`.
- **Decision:** ADR-0001
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Check:** `ci-test` — `crates/unica-coder/src/interfaces/mcp.rs`
- **Scope:** runtime

### INV-MCP-04 — Public tools live in the `unica.*` namespace

- **Rule:** The public tool set is addressed by `unica.<group>.<operation>`
  names, and the packaged runtime exposes every required `unica.*` tool under
  that name without exposing a removed alias.
- **Decision:** ADR-0001, ADR-0011
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Check:** `release-gate` — `scripts/ci/smoke-unica-mcp.py`
- **Scope:** runtime, packaged, release

### INV-MCP-05 — Tool contracts are data-driven and adapter-free

- **Rule:** Tool names and descriptions come from the `ToolSpec` registry in
  `application/mod.rs`, input schemas come from `application/tool_contracts.rs`
  on top of `application/operation_descriptors.rs`, the transport only assembles
  the three, and no public tool schema exposes raw adapter arguments.
- **Decision:** ADR-0001, ADR-0013
- **Check:** `ci-test` — `crates/unica-coder/src/interfaces/mcp.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/application/tool_contracts.rs`
- **Scope:** runtime

### INV-MCP-06 — Transport is owned by the official Rust SDK

- **Rule:** The public stdio server is an `rmcp::ServerHandler` implementation in
  `interfaces/mcp.rs` that serves `initialize`, `tools/list`, and `tools/call`
  from the application registry, and both the `rmcp` types and the SDK tool
  macros stay inside that module.
- **Decision:** ADR-0013, ADR-0002
- **Check:** `ci-test` — `crates/unica-coder/src/interfaces/mcp.rs`
- **Check:** `manual` — no guard script knows the crate name, so review confirms
  that `rmcp` imports and the SDK tool macros stay inside
  `crates/unica-coder/src/interfaces/mcp.rs`
- **Scope:** source, runtime

### INV-MCP-07 — Admission is bounded and cancellation is cooperative

- **Rule:** At most 32 concurrent `tools/call` workers are admitted, excess
  calls fail with JSON-RPC `-32603` containing `overloaded`, a request
  cancelled through `notifications/cancelled` receives no response, and
  transport shutdown cancels still-running domain operations within a bounded
  grace.
- **Decision:** ADR-0013, ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/interfaces/mcp.rs`
- **Scope:** runtime

### INV-MCP-08 — Public surface changes are synchronized

- **Rule:** Adding, removing, or renaming a public MCP tool changes the Rust
  registry, the parity harness, and the architecture layer — a decision record,
  an acceptance plan, or a registry entry — in one change set.
- **Decision:** ADR-0001, ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_script_parity.py`
- **Check:** `guard-script` — `scripts/ci/check-architecture-sync.py`
- **Check:** `ci-test` — `tests/ci/test_architecture_sync_guard.py`
- **Scope:** source, packaged

## SKILL — маршрутизация скиллов

### INV-SKILL-01 — Skills route through MCP `unica`

- **Rule:** Every in-scope skill documents its routing through MCP `unica` and
  names the `unica.*` tool it calls.
- **Decision:** ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged

### INV-SKILL-02 — Skills never name internal adapter servers

- **Rule:** Prompt-visible skills and references must not name internal adapter
  MCP servers or their tool identifiers as routing targets.
- **Decision:** ADR-0001, ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged

### INV-SKILL-03 — Skill-local operation scripts must not return

- **Rule:** Skills must not ship or reference skill-local Python, PowerShell, or
  shell operation files as an execution path; the migration to native `unica.*`
  handlers is complete and reintroducing such a path requires a superseding
  decision.
- **Decision:** ADR-0004, ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged, runtime

### INV-SKILL-04 — Reference models are test-only fixtures

- **Rule:** Adapted operation scripts exist only as Unica-owned reference models
  under `tests/fixtures/unica_mcp_script_parity/unica_reference_models`, the
  reviewed donor snapshot only under
  `tests/fixtures/unica_mcp_script_parity/cc-1c-skills`, and neither tree is
  packaged or reachable at runtime.
- **Decision:** ADR-0004
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** source, packaged

### INV-SKILL-05 — Mutating guidance defaults to preview

- **Rule:** Skill guidance keeps the preview path visible on the destructive and
  incomplete routes: the `meta-remove` skill documents a `"dryRun": true` call,
  and every documented incremental, partial, or external-source-set dump is
  written as a preview call.
- **Decision:** ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged

### INV-SKILL-06 — Skill examples are executable MCP calls

- **Rule:** Every `tools/call` example in a skill is a real, parameterized call
  that executes successfully as an MCP dry run.
- **Decision:** ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_script_parity.py`
- **Scope:** source, packaged

## APP — границы слоёв приложения

### INV-APP-01 — Application owns tool dispatch and domain events

- **Rule:** `UnicaApplication` owns the public tool registry, tool dispatch, and
  domain event emission; a new tool handler enters through application dispatch
  and nowhere else.
- **Decision:** ADR-0002, ADR-0003
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Scope:** source, runtime

### INV-APP-02 — Transport only maps protocol to application calls

- **Rule:** `interfaces::mcp` serves `tools/list` from
  `UnicaApplication::tools()`, dispatches every `tools/call` through
  `call_tool_cancellable`, and returns the application's result envelope as the
  tool text rather than a shape of its own.
- **Decision:** ADR-0002, ADR-0013
- **Check:** `ci-test` — `crates/unica-coder/src/interfaces/mcp.rs`
- **Scope:** source, runtime

### INV-APP-03 — Adapters reach the workspace through application ports

- **Rule:** Infrastructure adapters reach workspace state through
  `ApplicationPorts` and never import the interface layer, so an adapter cannot
  render an MCP response and bypass application cache reporting on the way out.
- **Decision:** ADR-0002, ADR-0003
- **Check:** `guard-script` — `scripts/ci/check-rust-platform-boundary.py`
- **Check:** `ci-test` — `tests/ci/test_rust_platform_boundary.py`
- **Scope:** source, runtime

### INV-APP-04 — No script backend in the runtime

- **Rule:** `unica-coder` contains no runtime operation-file fallback: no legacy
  script handler, and no production spawn of `python`, `python3`, `bash`,
  `powershell`, or `pwsh`.
- **Decision:** ADR-0004
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** source, runtime

### INV-APP-05 — Layer dependency direction is enforced

- **Rule:** `domain` imports neither `application`, `infrastructure`, nor
  `interfaces` and performs no filesystem or process access, and `application`
  imports neither `infrastructure` nor `interfaces`.
- **Decision:** ADR-0009, ADR-0002
- **Check:** `guard-script` — `scripts/ci/check-rust-platform-boundary.py`
- **Check:** `ci-test` — `tests/ci/test_rust_platform_boundary.py`
- **Scope:** source

### INV-APP-06 — Application does not spawn git directly

- **Rule:** Production code under `crates/unica-coder/src/application` never
  constructs a `git` child process; git state is read through infrastructure.
- **Decision:** ADR-0002, ADR-0009
- **Check:** `ci-test` — `tests/ci/test_product_contracts.py`
- **Scope:** source

### INV-APP-07 — Internal workspace services stay hidden and workspace-scoped

- **Rule:** Warm analyzer and index state lives in hidden services keyed by
  workspace root plus source root, started lazily only when a non-dry-run
  analyzer or index operation needs them; cheap read-only operations such as
  `unica.code.grep` do not start a service, and no service becomes a public MCP
  registration.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
- **Scope:** runtime

## CACHE — состояние workspace и кеш

### INV-CACHE-01 — The orchestrator owns workspace state

- **Rule:** Workspace state and cache invalidation belong to the `unica`
  orchestrator; the model is never asked to coordinate cache freshness between
  engines.
- **Decision:** ADR-0003, ADR-0001
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace_state.rs`
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Scope:** runtime

### INV-CACHE-02 — Mutating operations emit typed domain events

- **Rule:** Every mutating operation emits typed domain events, and those events
  map to the invalidated and refreshed cache names reported back to the caller.
- **Decision:** ADR-0003
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Scope:** runtime

### INV-CACHE-03 — The volatile cache root is overridable

- **Rule:** The volatile cache root defaults to `<workspaceRoot>/.build/unica`
  and is overridden by `UNICA_CACHE_DIR`, and workspace service records are
  written under whichever root is in effect.
- **Decision:** ADR-0003
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace.rs`
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
- **Scope:** runtime

### INV-CACHE-04 — Dry run reports impact without writing state

- **Rule:** A dry-run call reports its cache impact and writes no workspace
  state, no index, and no service record.
- **Decision:** ADR-0003
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace_index.rs`
- **Scope:** runtime

### INV-CACHE-05 — An applied mutation persists the cache it invalidated

- **Rule:** An applied mutation records its domain events in
  `WorkspaceStateRepository`, so a cache the mutation invalidated is still
  reported stale on a later read instead of being silently rebuilt.
- **Decision:** ADR-0003, ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace_state.rs`
- **Scope:** runtime

### INV-CACHE-06 — A linked worktree is an isolated workspace

- **Rule:** Workspace identity, workspace epoch, cache roots, and internal
  service keys are derived so that a linked git worktree is isolated from the
  primary checkout and from every other worktree, and code that reads git state
  resolves `.git` as either a directory or a pointer file.
- **Decision:** ADR-0003
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace.rs`
- **Scope:** runtime

### INV-CACHE-07 — Runtime cache resolution is deterministic

- **Rule:** `unica-bootstrap` resolves the runtime cache root in a fixed order —
  `UNICA_RUNTIME_CACHE_DIR` used verbatim unless it still contains an unexpanded
  `${` token, then `<CLAUDE_PLUGIN_DATA>/runtimes`, then
  `<CODEX_HOME>/unica/runtimes`, then `<HOME or USERPROFILE>/.codex/unica/runtimes`,
  and fails when none of them is set — and publishes a verified runtime
  atomically under `<cacheRoot>/<pluginVersion>/<target>`.
- **Decision:** ADR-0008, ADR-0012
- **Check:** `ci-test` — `crates/unica-bootstrap/tests/runtime_install.rs`
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Check:** `manual` — the tests cover only the published layout and the
  packaged `UNICA_RUNTIME_CACHE_DIR` value, so review `runtime_cache_root` and
  `codex_home_root` in `crates/unica-bootstrap/src/main.rs` for the whole
  fallback order, the `${` discard, the `.codex` segment added when `CODEX_HOME`
  is unset, and the hard error when no variable is set
- **Scope:** packaged, runtime

## SOURCE — source sets воркспейса

### INV-SOURCE-01 — Source format belongs to a source-set

- **Rule:** `unica.project.map` reports `sourceSets[]` and each entry carries
  its own `sourceFormat`, because source format is a property of a source-set,
  not of the whole workspace.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/project_sources.rs`
- **Check:** `doc-assert` — `tests/ci/test_unica_skills.py`
- **Scope:** runtime, source

### INV-SOURCE-02 — One source-set is never two formats at once

- **Rule:** Conflicting format markers inside a single source-set make that
  source-set invalid or ambiguous; a source-set never reports a blended format.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/project_sources.rs`
- **Scope:** runtime

### INV-SOURCE-03 — A workspace may hold several effective formats

- **Rule:** One workspace may contain several source-sets with different
  effective formats, such as an EDT configuration next to platform XML external
  processors and reports.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/project_sources.rs`
- **Check:** `doc-assert` — `tests/ci/test_unica_skills.py`
- **Scope:** runtime, source

### INV-SOURCE-04 — Native XML operations require a platform XML source-set

- **Rule:** A native platform XML metadata operation resolves a source-set whose
  `sourceFormat` is `platform_xml` before touching XML files, and is rejected
  with a typed error when the resolved source-set is EDT-formatted, invalid, or
  ambiguous.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/tool_context.rs`
- **Scope:** runtime

### INV-SOURCE-05 — Source-root selection is deterministic and shared

- **Rule:** A non-empty `sourceDir` is resolved relative to the request working
  directory, otherwise a source set named `main` wins, followed by the sole
  configuration source set; the resolved root is normalized, stays inside the
  workspace, and is the same root used for the analyzer, the index, service
  identity, `unica.project.status`, and `unica.project.map`.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/tool_context.rs`
- **Scope:** runtime

## PKG — упаковка и поставка

### INV-PKG-01 — Generated binaries are not committed

- **Rule:** Generated binaries and other generated package paths are never
  tracked in the source tree, and packaging fails when a tracked file falls
  inside a generated path or is a symlink.
- **Decision:** ADR-0008
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** source, packaged

### INV-PKG-02 — The public marketplace package is thin

- **Rule:** The published package carries only plugin files and the three small
  bootstrap binaries; its `.mcp.json` starts the runtime through a
  command-scoped Git shell alias that resolves the plugin root for both hosts
  and hands it to `bootstrap/launch.sh`, and it never depends on a full runtime
  binary or a per-target command matrix.
- **Decision:** ADR-0008, ADR-0012
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** packaged, release

### INV-PKG-03 — Runtime acquisition is checksum-verified and atomic

- **Rule:** Bootstrap downloads the pinned host runtime, verifies the archive
  SHA-256 against the release metadata and every extracted file against its
  recorded digest, and only then publishes the runtime atomically; a corrupt or
  traversal-bearing archive never becomes a ready runtime.
- **Decision:** ADR-0008
- **Check:** `ci-test` — `crates/unica-bootstrap/tests/runtime_install.rs`
- **Check:** `ci-test` — `tests/ci/test_package_unica_runtime.py`
- **Check:** `release-gate` — `scripts/ci/verify-release-assets.py`
- **Scope:** packaged, release, runtime

### INV-PKG-04 — The public runtime binary is named `unica`

- **Rule:** The bundled public binary produced from the Cargo workspace is named
  `unica` and is recorded under that name in
  `plugins/unica/third-party/tools.lock.json`.
- **Decision:** ADR-0001, ADR-0008
- **Check:** `guard-script` — `scripts/ci/check-version-contract.py`
- **Check:** `ci-test` — `tests/ci/test_build_unica_tools.py`
- **Scope:** source, packaged

### INV-PKG-05 — Both host manifests carry one version

- **Rule:** `plugins/unica/.codex-plugin/plugin.json` and
  `plugins/unica/.claude-plugin/plugin.json` both exist and declare the same
  version as the Cargo workspace and the `unica` entry in `tools.lock.json`; the
  Claude manifest declares neither `skills` nor `mcpServers`, because both are
  discovered by default.
- **Decision:** ADR-0012
- **Check:** `guard-script` — `scripts/ci/check-version-contract.py`
- **Check:** `ci-test` — `tests/ci/test_version_contract.py`
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Check:** `ci-test` — `crates/unica-bootstrap/src/main.rs`
- **Scope:** source, packaged

### INV-PKG-06 — Manifests and catalogs stay inside the client floor

- **Rule:** Host manifests and catalog entries use only keys the oldest
  supported client accepts, and both host catalogs pin the same immutable
  release tag with a subdirectory-addressing source type.
- **Decision:** ADR-0012, ADR-0008
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Check:** `doc-assert` — `tests/ci/test_product_contracts.py`
- **Scope:** packaged, release

### INV-PKG-07 — Local debug packaging is development-only

- **Rule:** The local debug package launches the host-target binary
  `bin/<target>/unica` (`unica.exe` on `win-x64`) directly instead of a bootstrap
  payload — by relative path with `cwd` on Codex, through
  `${CLAUDE_PLUGIN_ROOT}` without `cwd` on Claude Code — is built for the
  current host target only, and registers its Codex catalog under the
  `unica-dev` name so that catalog can never be mistaken for the published one.
- **Decision:** ADR-0008
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** source

### INV-PKG-08 — Attribution stays complete and reachable

- **Rule:** Every bundled tool, adapted skill source, and packaged third-party
  asset has an attribution record, and the attribution page is linked from both
  the repository and the packaged README.
- **Decision:** n/a
- **Check:** `guard-script` — `scripts/ci/check-attributions.py`
- **Check:** `ci-test` — `tests/ci/test_attributions.py`
- **Scope:** source, packaged

## PLATFORM — платформенный фасад

### INV-PLATFORM-01 — OS-specific code lives behind platform facades

- **Rule:** OS-specific production code exists only under
  `crates/unica-coder/src/infrastructure/platform/**` and
  `crates/unica-bootstrap/src/platform/**`; filesystem, path, process, and
  entrypoint behavior enters the rest of the code through those facades as
  platform-neutral types.
- **Decision:** ADR-0009
- **Check:** `guard-script` — `scripts/ci/check-rust-platform-boundary.py`
- **Check:** `ci-test` — `tests/ci/test_rust_platform_boundary.py`
- **Scope:** source

### INV-PLATFORM-02 — The platform guard has no path exemptions

- **Rule:** The platform guard admits OS-specific code only by structural
  location — the two platform facade prefixes and nested `tests/platform/**`
  directories — and carries no path-by-path legacy exemption.
- **Decision:** ADR-0009
- **Check:** `ci-test` — `tests/ci/test_rust_platform_boundary.py`
- **Check:** `manual` — the tests exercise the structural rules on sample paths
  but cannot see a new exemption, so review every change to `_is_platform_facade`
  and `_is_platform_test` in `scripts/ci/check-rust-platform-boundary.py` for a
  literal legacy path before it is merged
- **Scope:** source

### INV-PLATFORM-03 — Platform tests live beside their adapters

- **Rule:** Platform-specific tests live next to their adapters or under
  `crates/<crate>/tests/platform/**`, never as a top-level platform test file.
- **Decision:** ADR-0009
- **Check:** `ci-test` — `tests/ci/test_rust_platform_boundary.py`
- **Scope:** source, ci

### INV-PLATFORM-04 — Child processes are owned as process trees

- **Rule:** Analyzer, index, and runtime child processes are owned as process
  trees — a kill-on-close Job Object on Windows, a dedicated process group on
  Unix — so cancellation, timeout, shutdown, or session failure terminates the
  whole tree within bounded waits.
- **Decision:** ADR-0006, ADR-0009
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/platform/process.rs`
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
- **Scope:** runtime

## CI — сборка, артефакты и релизный конвейер

### INV-CI-01 — One locked Cargo build per platform runner

- **Rule:** Each platform runner builds `unica` and `unica-bootstrap` in one
  mandatory `cargo build --locked` invocation against one target-specific Cargo
  target directory; a restored cache accelerates that command but never
  replaces it.
- **Decision:** ADR-0010
- **Check:** `ci-test` — `tests/ci/test_build_unica_tools.py`
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Scope:** ci

### INV-CI-02 — Cargo cache hits are exact and observable

- **Rule:** The Cargo cache key contains runner OS, Unica target, resolved
  toolchain key, and the `Cargo.lock` hash, no prefix restore keys are used, and
  every platform build reports its target, cache outcome, and build duration.
- **Decision:** ADR-0010
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Scope:** ci

### INV-CI-03 — Artifacts are narrow, typed, and short-lived

- **Rule:** Cargo target directories are never uploaded; data crosses job
  boundaries only as runtime metadata, bootstrap payload, and runtime archive
  artifacts with one-day retention, while the thin marketplace payload keeps its
  longer retention for manual staging and promotion.
- **Decision:** ADR-0010
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Scope:** ci, release

### INV-CI-04 — Each platform verifies what it produced

- **Rule:** A platform runner packages its runtime archive and verifies the
  archive checksum, file set, member checksums, executable modes, and zeroed
  timestamps against its metadata before the archive is uploaded or discarded;
  tag publication repeats the verification on the downloaded published bytes.
- **Decision:** ADR-0010, ADR-0008
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Check:** `release-gate` — `scripts/ci/verify-release-assets.py`
- **Scope:** ci, release

### INV-CI-05 — Publication happens only on a tag

- **Rule:** Release assets are published only by tag pushes; pull-request and
  manual runs package and smoke without publishing, and staging and catalog
  promotion remain separate explicit jobs.
- **Decision:** ADR-0008, ADR-0010
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Scope:** ci, release

### INV-CI-06 — One aggregate gate closes every pull request

- **Rule:** Every pull request is decided by the single stable aggregate gate
  that evaluates the source, Rust, packaging, bootstrap, assessment, and
  published-asset jobs together.
- **Decision:** ADR-0010
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Check:** `ci-test` — `tests/ci/test_evaluate_ci_gate.py`
- **Scope:** ci

## DOC — документационный слой

### INV-DOC-01 — Registry entries use the canonical record format

- **Rule:** Every registry entry carries a heading `### <ID> — <short name>`,
  exactly one `Rule`, a `Decision`, at least one `Check`, and a `Scope`, with
  `Check` classes drawn from `ci-test`, `guard-script`, `doc-assert`,
  `release-gate`, or `manual` and `Scope` values drawn from `source`,
  `packaged`, `ci`, `release`, or `runtime`.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Scope:** source

### INV-DOC-02 — Registry IDs are unique and never reused

- **Rule:** Every registry ID matches `^(INV|REQ)-[A-Z][A-Z0-9]*-[0-9]{2}$`, is
  unique across the whole specification corpus, and is never reassigned to a
  different rule after its original entry is removed.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Scope:** source

### INV-DOC-03 — Every invariant names a real check

- **Rule:** Every registry entry names at least one check, and every check that
  is not `manual` points at a path that exists in the repository.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Scope:** source

### INV-DOC-04 — Indexes stay synchronized with their documents

- **Rule:** Every accepted decision record is listed in `spec/decisions/README.md`
  and every listed record exists, and every arc42 chapter file is listed in
  `spec/architecture/arc42/architecture.md` and every listed chapter exists.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Scope:** source

### INV-DOC-05 — Historical documents are marked as historical

- **Rule:** The index of every archived tree — `docs/design` and `docs/plans` —
  carries the archive marker that names it archived planning material rather
  than a source of truth, and no normative document lives outside `spec/`.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Scope:** source

### INV-DOC-06 — Relative links resolve from their own document

- **Rule:** Every relative markdown link in the active documentation layer
  resolves from the directory of the document that carries it, so no reader
  needs the repository root to follow it.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged

### INV-DOC-07 — Normative text is written in English

- **Rule:** The normative fields of a registry record — `Rule`, `Decision`, and
  `Scope` — are written in English, so one grep finds every statement of a rule,
  while chapter headings and explanatory prose may stay in Russian.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Scope:** source

### INV-DOC-08 — Normative text has one owner

- **Rule:** A rule owned by a registry entry or a decision record is referenced
  by ID from other documents instead of being restated, and no arc42 chapter
  restates the decision catalogue as a second index.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Check:** `manual` — only the second-index half is automated, so architecture
  review rejects a copied normative sentence in favour of a reference by ID
- **Scope:** source

## Retired identifiers

Идентификатор, у которого удалена запись, попадает сюда и больше никогда не
выдаётся другому правилу. Иначе ссылка из старого PR или из чужого конспекта
однажды укажет на правило, которого автор ссылки не имел в виду.

Единственное допустимое содержимое раздела — строки вида «идентификатор, дата
вывода, причина». Любой идентификатор, названный здесь, считается выведенным,
поэтому примеры в прозе неуместны: их подхватит
`tests/ci/test_architecture_registry.py`.

Выведенных из обращения идентификаторов пока нет.
