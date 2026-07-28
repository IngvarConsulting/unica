# 10. Требования к качеству

Эта глава — реестр требований к качеству. Инвариант отвечает на вопрос «что
нельзя сломать», требование — на вопрос «насколько хорошо система обязана себя
вести». Поэтому каждая запись здесь описывает сценарий: что происходит
(стимул) и какой измеримый ответ система обязана дать.

## Как читать реестр

- Формат записи, классы проверок и правила ID общие с
  [реестром инвариантов](../invariants.md); там же описано, что означают поля
  `Rule`, `Decision`, `Check` и `Scope`.
- ID требований цитируются в описании PR и в ревью так же, как ID инвариантов:
  «REQ-SAFETY-02 сохранён», «REQ-PERF-03 меняется, см. новый бенчмарк».
- Требование не повторяет текст инварианта. Если структурное правило уже
  нормировано, требование формулирует наблюдаемое качество и ссылается на
  инвариант по ID (INV-DOC-08).
- Если измеримый ответ ничем не проверяется автоматически, класс проверки —
  `manual`, а в цели честно написано, что именно смотрит человек. Отсутствие
  проверки — это долг, а не оформительская мелочь; такие места собраны в
  [главе 11](11-risks-and-technical-debt.md).

## PERF — задержки и бюджеты

### REQ-PERF-01 — No public call waits without a deadline

- **Rule:** Every request from a public tool to an internal workspace service
  carries explicit connect, read, and overall budgets, so an unresponsive or
  hung service turns into a typed error within the recorded deadline instead of
  an MCP call that never returns.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace_services.rs`
- **Check:** `doc-assert` — `tests/ci/test_product_contracts.py`
- **Scope:** runtime

### REQ-PERF-02 — A runtime is handed over only after a bounded handshake

- **Rule:** `unica-bootstrap` reports an installation as successful only after
  the installed runtime answers MCP `initialize` and `tools/list` within a fixed
  time budget; a runtime that stalls or answers incompletely fails verification
  instead of being handed to the host.
- **Decision:** ADR-0008
- **Check:** `ci-test` — `crates/unica-bootstrap/tests/verification_contract.rs`
- **Check:** `release-gate` — `scripts/ci/smoke-unica-bootstrap.py`
- **Check:** `manual` — the numeric budget passed by `install_and_verify_runtime`
  in `crates/unica-bootstrap/src/main.rs` is not asserted by a test; re-read it
  when the runtime start path changes
- **Scope:** packaged, release, runtime

### REQ-PERF-03 — Warm state is reused, not rebuilt per call

- **Rule:** A workspace service started for one workspace root plus source root
  is reused by later calls while it is live, and its idle and maximum-age
  budgets are resolved from fixed defaults or their environment overrides, so
  repeated analyzer calls do not pay a cold start.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace_services.rs`
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
- **Scope:** runtime

## TOKEN — расход контекста модели

### REQ-TOKEN-01 — Cache coordination costs no extra call

- **Rule:** A caller never needs a second tool call to learn what a mutation
  invalidated: the same result reports the impacted cache names, so the model is
  never asked which internal engine owns which cache (structural rule:
  INV-CACHE-01).
- **Decision:** ADR-0001, ADR-0003
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Scope:** runtime

### REQ-TOKEN-02 — Long-running output stays in files, not in the result

- **Rule:** Runtime job output is captured per stream into `stdout.log` and
  `stderr.log` beside the durable job record, and `unica.runtime.job.*` returns
  the retained stream tails and those paths instead of the log files
  themselves, so polling a long build or test run does not carry the whole log
  into the result.
- **Decision:** ADR-0001
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/runtime_jobs.rs`
- **Scope:** runtime

## SAFETY — безопасность мутаций и данных

### REQ-SAFETY-01 — Mutation previews before it applies

- **Rule:** A mutating tool called without an explicit `dryRun: false` resolves
  to a dry run: it plans the change, reports its cache impact, and writes
  nothing; only an explicit `dryRun: false` may touch the workspace.
- **Decision:** ADR-0003, ADR-0005
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Scope:** runtime

### REQ-SAFETY-02 — Secrets never reach the model

- **Rule:** Connection strings, passwords, tokens, and other secret-keyed values
  are redacted from tool results, runtime job records, echoed argument vectors,
  and error messages before they leave the process, including when a secret key
  and its value are split across streamed chunks.
- **Decision:** n/a
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/redaction.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/runtime_jobs.rs`
- **Scope:** runtime

### REQ-SAFETY-03 — Supported vendor objects are protected before planning

- **Rule:** A native mutating operation whose target is locked by the
  configuration support state is refused with a typed outcome, and the refusal
  happens before planning, so a dry run and an applied call reach the same
  verdict; only the support state recorded in the configuration may downgrade
  the refusal to a warning.
- **Decision:** n/a
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Scope:** runtime

### REQ-SAFETY-04 — A failing write leaves no half-written tree

- **Rule:** A metadata or file publication either becomes fully visible or does
  not happen: an interrupted, failing, or cancelled write leaves the previous
  source tree intact rather than a partially rewritten one.
- **Decision:** n/a
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs`
- **Scope:** runtime

## OBS — наблюдаемость результата

### REQ-OBS-01 — One stable result envelope

- **Rule:** Every tool returns the same top-level envelope — `ok`, `summary`,
  `changes`, `warnings`, `errors`, `artifacts`, `cache` — and richer payloads
  enter through optional typed fields such as `diagnostics`, `data`, and `job`,
  so a new adapter adds detail without reshaping the contract callers parse.
- **Decision:** ADR-0001, ADR-0002
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Scope:** runtime

### REQ-OBS-02 — Long operations are observable while they run

- **Rule:** An operation started as a runtime job exposes its status, its
  bounded logs, and its completion through durable records, so progress is
  inspectable without holding an MCP call open for the whole run.
- **Decision:** ADR-0001, ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/runtime_jobs.rs`
- **Scope:** runtime

## MAINT — сопровождаемость

### REQ-MAINT-01 — Replacing an adapter does not rewrite MCP handling

- **Rule:** Replacing or re-implementing an internal adapter is confined to
  `infrastructure`: tool names, schemas, dispatch, and protocol mapping stay
  where they are, and the layer guard fails the change if the boundary is
  crossed (structural rule: INV-APP-05).
- **Decision:** ADR-0002, ADR-0009
- **Check:** `guard-script` — `scripts/ci/check-rust-platform-boundary.py`
- **Check:** `ci-test` — `tests/ci/test_rust_platform_boundary.py`
- **Scope:** source

### REQ-MAINT-02 — A new public tool is data, not transport code

- **Rule:** Adding a public tool adds a registry entry and its input schema to
  the application layer and requires no change to the MCP transport module,
  because names, descriptions, and schemas are data (structural rule:
  INV-MCP-05).
- **Decision:** ADR-0001, ADR-0013
- **Check:** `ci-test` — `crates/unica-coder/src/application/tool_contracts.rs`
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_script_parity.py`
- **Scope:** source

### REQ-MAINT-03 — Donor drift fails the suite, not the workspace

- **Rule:** Behaviour that must stay compatible with an adapted donor model is
  covered by a parity scenario executed as a real dry-run MCP call, and the
  reviewed donor snapshot is pinned by digest, so a Rust port that drifts fails
  CI before a user meets the difference.
- **Decision:** ADR-0004
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_script_parity.py`
- **Check:** `ci-test` — `tests/ci/test_donor_parity_contract.py`
- **Scope:** ci

## COMPAT — совместимость

### REQ-COMPAT-01 — Every supported target is built and verified together

- **Rule:** A release builds `unica` and `unica-bootstrap` for every supported
  host target, and each target verifies the archive it produced before that
  archive can be published; a target that fails its own verification blocks
  publication for all of them.
- **Decision:** ADR-0010, ADR-0008
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Check:** `release-gate` — `scripts/ci/verify-release-assets.py`
- **Scope:** ci, release

### REQ-COMPAT-02 — The packaged plugin loads at the client floor

- **Rule:** The generated marketplace package and its catalog are validated by a
  real host client pinned to the oldest supported version, because an
  unrecognised manifest or catalog key is a load error there rather than a
  warning (structural rule: INV-PKG-06).
- **Decision:** ADR-0012
- **Check:** `release-gate` — `.github/workflows/unica-plugin-release.yml`
- **Check:** `doc-assert` — `tests/ci/test_product_contracts.py`
- **Check:** `manual` — only one of the two hosts runs a real client validation
  in CI; review the other host's manifest and catalog by hand when either
  contract changes
- **Scope:** packaged, release

### REQ-COMPAT-03 — One package serves both hosts

- **Rule:** The published package contains both host manifest directories and
  both host catalogs at one version, and a single host-neutral launcher, so
  installing the same bytes on either host yields the same public tool surface
  (structural rule: INV-PRODUCT-01, INV-PKG-05).
- **Decision:** ADR-0012
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Check:** `ci-test` — `tests/ci/test_version_contract.py`
- **Scope:** packaged, release

### REQ-COMPAT-04 — The emitted 1C format profile is explicit and audited

- **Rule:** Native XML operations emit the active platform line and export
  format from one shared profile constant, and every deliberate deviation from
  the official platform mapping is recorded in the deviation matrix rather than
  discovered in a user's dump.
- **Decision:** n/a
- **Check:** `ci-test` — `tests/ci/test_format_profile_contract.py`
- **Check:** `ci-test` — `crates/unica-coder/tests/format_8_3_27_xml_corpus.rs`
- **Scope:** runtime, source

## REL — надёжность поставки

### REQ-REL-01 — Bundled engines are used, never the host's PATH

- **Rule:** An internal engine is resolved from the packaged
  `bin/<target>/<tool>` recorded in the third-party manifest and verified by
  SHA-256 before execution, and the packaged bootstrap is smoked with a consumer
  PATH reduced to the system directories of the target — plus Git on Windows —
  in which the smoke asserts that Node.js is unreachable.
- **Decision:** ADR-0006, ADR-0008
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/bundled_tools.rs`
- **Check:** `ci-test` — `tests/ci/test_smoke_unica_bootstrap.py`
- **Check:** `release-gate` — `scripts/ci/smoke-unica-bootstrap.py`
- **Scope:** packaged, runtime, release

### REQ-REL-02 — A repeated launch reuses the published runtime

- **Rule:** A second launch for the same plugin version and host target reuses
  the runtime already published under that cache entry instead of downloading it
  again, and concurrent launches publish it exactly once, so the download and
  publication cost is paid one time per version and target.
- **Decision:** ADR-0008
- **Check:** `ci-test` — `crates/unica-bootstrap/tests/runtime_install.rs`
- **Scope:** packaged, runtime

### REQ-REL-03 — A stalled release reports instead of guessing

- **Rule:** A two-phase release advances only when the required checks are green
  and the human tag exists; when it cannot advance, the scheduled warden reports
  what it is waiting on and fails visibly rather than publishing early or going
  silent.
- **Decision:** ADR-0008
- **Check:** `guard-script` — `scripts/ci/release-warden.py`
- **Check:** `ci-test` — `tests/ci/test_release_warden.py`
- **Check:** `doc-assert` — `tests/ci/test_product_contracts.py`
- **Scope:** release

### REQ-REL-04 — A release candidate is assessed on a real configuration

- **Rule:** Every release candidate runs the public tool scenarios against a
  pinned real 1C configuration and produces a machine-readable report naming
  each scenario, its status, and its duration; a failed blocking scenario fails
  the assessment.
- **Decision:** ADR-0008, ADR-0010
- **Check:** `guard-script` — `scripts/ci/release-assessment.py`
- **Check:** `ci-test` — `tests/ci/test_release_assessment.py`
- **Scope:** ci, release
