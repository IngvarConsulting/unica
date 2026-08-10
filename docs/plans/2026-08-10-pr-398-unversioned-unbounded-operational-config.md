# Неверсионированная operational config без верхних лимитов — план реализации

> **Для agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Исправить PR #398 так, чтобы `unica.toml` оставался намеренно
неверсионированным, а положительные operational deadlines могли превышать
прежние константы `45/120` и доходили до реального поставщика без скрытого
обрезания.

**Architecture:** Общий root parser принимает только `operational`, `network`
и `providers`; `version` является неизвестным полем. Домен хранит положительные
конечные `Duration` без policy-maximum, сохраняя только compiled defaults и
отношение provider deadline ≤ total deadline. Пути workspace service,
получившие caller budget, используют именно этот абсолютный бюджет; внутренние
control/start defaults остаются 120-секундными.

**Tech Stack:** Rust 2021, `toml = "0.8"`, Cargo workspace tests, Python 3.12
architecture guards, GitHub CLI.

## Global Constraints

- Публичные инструменты, аргументы и result envelope не меняются.
- Публичный `unica.code.diagnostics.timeoutSeconds` сохраняет контракт
  `30..=3600`; ограничение относится к явному MCP-аргументу, а не к значениям
  `[operational]` в workspace config.
- Каждый operational timeout — целое число секунд `>= 1`; `0` не означает
  бесконечность.
- Верхнего policy-limit у operational timeout нет; техническая граница входа —
  положительный диапазон TOML `i64`, безопасно представимый как `u64`.
- Compiled defaults остаются `120/45/15/45/120` секунд.
- `search_rlm_timeout_seconds` и `search_git_grep_timeout_seconds` не могут
  превышать итоговый `search_total_timeout_seconds`, потому что больший
  provider budget никогда не может быть эффективен внутри общего deadline.
- Общие служебные тайм-ауты запуска, ping, shutdown и обычных запросов без
  operational budget остаются неизменными.
- Для каждого дефекта сначала запускается тест, падающий на текущем коде по
  ожидаемой причине, и только затем меняется production code.

---

### Task 1: Удалить версию из общего файлового контракта

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/workspace_config.rs`
- Modify: `crates/unica-coder/src/infrastructure/operational_config.rs`
- Modify: `crates/unica-coder/src/infrastructure/documentation_policy.rs`
- Test: те же Rust-модули под `#[cfg(test)]`.

**Interfaces:**
- Consumes: два независимых слоя `unica.toml` и `unica.local.toml`.
- Produces:

```rust
const ROOT_FIELDS: &[&str] = &["operational", "network", "providers"];

pub(crate) enum WorkspaceConfigRootErrorKind {
    InvalidToml,
    UnknownField,
}
```

- [ ] **Step 1: Изменить root tests до нового контракта**

```rust
#[test]
fn accepts_operational_root_without_version() {
    let root = parse_workspace_config_root(
        "[operational.code_intelligence]\nsearch_total_timeout_seconds = 90\n",
    ).expect("unversioned operational root");
    assert!(root.contains_key("operational"));
}

#[test]
fn rejects_version_as_unknown_root_field() {
    let error = parse_workspace_config_root("version = 1\n")
        .expect_err("version is outside the fixed schema");
    assert_eq!(error.kind(), WorkspaceConfigRootErrorKind::UnknownField);
    assert_eq!(error.field_path(), "version");
}
```

- [ ] **Step 2: Запустить RED**

```bash
cargo test -p unica-coder workspace_config::tests::accepts_operational_root_without_version
cargo test -p unica-coder workspace_config::tests::rejects_version_as_unknown_root_field
```

Expected: первый тест получает `MissingVersion`, второй ошибочно принимает
`version = 1`.

- [ ] **Step 3: Упростить общий parser**

```rust
pub(crate) fn parse_workspace_config_root(
    contents: &str,
) -> Result<Table, WorkspaceConfigRootError> {
    let root = contents.parse::<Table>().map_err(|_| invalid_toml())?;
    reject_unknown_root_fields(&root)?;
    Ok(root)
}
```

Удалить `SUPPORTED_VERSION`, `MissingVersion`, `InvalidVersionType`,
`UnsupportedVersion` и их проекции в обоих потребителях. Удалить `version = 1`
из валидных fixtures; заменить прежние version-specific проверки на одну
проверку `UnknownField` с путём `version`.

- [ ] **Step 4: Запустить GREEN и оба consumer suite**

```bash
cargo test -p unica-coder workspace_config -- --test-threads=1
cargo test -p unica-coder operational_config -- --test-threads=1
cargo test -p unica-coder documentation_policy -- --test-threads=1
```

Expected: PASS.

### Task 2: Принимать любой положительный operational deadline

**Files:**
- Modify: `crates/unica-coder/src/domain/operational_config.rs`
- Modify: `crates/unica-coder/src/infrastructure/operational_config.rs`
- Test: те же модули под `#[cfg(test)]`.

**Interfaces:**
- Produces: `OperationalConfigLayer::set_timeout_seconds` принимает любое
  `seconds >= 1`, а `OperationalConfig::with_diagnostics_analyze_timeout`
  продолжает отдельно проверять публичный explicit argument `30..=3600`.

- [ ] **Step 1: Добавить тест больших файловых значений**

```rust
#[test]
fn operational_layers_accept_deadlines_above_compiled_defaults() {
    let mut shared = OperationalConfigLayer::default();
    for (field, seconds) in [
        (OperationalConfigField::SearchTotal, 7_200),
        (OperationalConfigField::SearchRlm, 3_600),
        (OperationalConfigField::SearchGitGrep, 1_800),
        (OperationalConfigField::ProviderRead, 7_200),
        (OperationalConfigField::DiagnosticsAnalyze, 7_200),
    ] {
        shared.set_timeout_seconds(
            field,
            seconds,
            OperationalConfigDiagnosticSource::Shared,
        ).expect("positive operational deadline");
    }
    let config = OperationalConfig::from_layers(Some(&shared), None).unwrap();
    assert_eq!(config.code_intelligence().search_total_timeout(), Duration::from_secs(7_200));
    assert_eq!(config.code_diagnostics().analyze_timeout(), Duration::from_secs(7_200));
}
```

- [ ] **Step 2: Запустить RED**

```bash
cargo test -p unica-coder operational_layers_accept_deadlines_above_compiled_defaults
```

Expected: FAIL с `OutOfRange` на первом значении выше прежнего maximum.

- [ ] **Step 3: Оставить только положительность**

```rust
const fn minimum(self) -> i64 { 1 }

if seconds < field.minimum() {
    return Err(OperationalConfigDiagnostic::new(
        OperationalConfigDiagnosticCode::OutOfRange,
        source,
        field.path(),
    ));
}
```

Удалить пять `*_MAX_SECONDS` и config-minimum 30. Не менять
`DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS` в публичном tool contract и проверки
`with_diagnostics_analyze_timeout`, потому что это отдельный explicit override.

- [ ] **Step 4: Запустить GREEN и loader suites**

```bash
cargo test -p unica-coder operational_layers_accept_deadlines_above_compiled_defaults
cargo test -p unica-coder domain::operational_config -- --test-threads=1
cargo test -p unica-coder infrastructure::operational_config -- --test-threads=1
```

Expected: PASS; ноль и отрицательные значения по-прежнему дают `OutOfRange`.

### Task 3: Убрать скрытые caps из caller-budgeted workspace service paths

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Test: тот же модуль под `#[cfg(test)]`.

**Interfaces:**
- Consumes: `timeout: Duration`, уже выведенный из operational snapshot и
  остатка общего deadline.
- Produces: `WorkspaceServiceCallDeadline::new(timeout)` для RLM execute и
  readiness; payload `rlm_start.execution_timeout_seconds` равен округлённому
  вверх остатку caller deadline.

- [ ] **Step 1: Добавить тесты manager budgets выше 120 секунд**

Расширить `RecordingConnector` записью переданного `budget` и проверить
реальные manager-вызовы:

```rust
#[test]
fn rlm_request_preserves_caller_budget_above_service_default() {
    // matching live record + RecordingConnector
    manager.call_rlm_cancellable(
        &context,
        &source_root,
        WorkspaceRlmOperation::Search { query: "needle".into(), limit: 20 },
        Duration::from_secs(300),
        &CancellationToken::new(),
    ).unwrap();
    assert!(*connector.budgets.borrow().last().unwrap() > Duration::from_secs(240));
}

#[test]
fn rlm_readiness_preserves_caller_budget_above_service_default() {
    manager.rlm_readiness_cancellable_with_timeout(
        &context,
        &source_root,
        &Map::new(),
        Duration::from_secs(300),
        &CancellationToken::new(),
    ).unwrap();
    assert!(*connector.budgets.borrow().last().unwrap() > Duration::from_secs(240));
}
```

- [ ] **Step 2: Добавить payload test для persistent RLM и запустить RED**

```rust
#[test]
fn rlm_start_uses_remaining_caller_deadline_for_execution_timeout() {
    let args = rlm_start_arguments(
        Path::new("/workspace/src"),
        "needle",
        Duration::from_millis(299_001),
    );
    assert_eq!(args["execution_timeout_seconds"], 300);
}
```

```bash
cargo test -p unica-coder rlm_request_preserves_caller_budget_above_service_default
cargo test -p unica-coder rlm_readiness_preserves_caller_budget_above_service_default
cargo test -p unica-coder rlm_start_uses_remaining_caller_deadline_for_execution_timeout
```

Expected: manager tests видят не более 120 секунд; payload test не компилируется
до появления единственного production builder вместо hardcoded `45`.

- [ ] **Step 3: Передать caller budget без clamp**

```rust
let deadline = WorkspaceServiceCallDeadline::new(timeout);
```

Применить только в `call_rlm_cancellable` и
`rlm_readiness_cancellable_with_timeout`. Методы без явного operational timeout
по-прежнему вызывают их с `SERVICE_REQUEST_TIMEOUT`.

- [ ] **Step 4: Построить RLM payload из остатка deadline**

```rust
fn duration_seconds_ceil(duration: Duration) -> u64 {
    duration.as_secs() + u64::from(duration.subsec_nanos() != 0)
}

fn rlm_start_arguments(source_root: &Path, query: &str, timeout: Duration) -> Value {
    json!({
        "path": source_root.display().to_string(),
        "query": query,
        "effort": "low",
        "max_output_chars": 100_000,
        "max_execute_calls": 10_000,
        "execution_timeout_seconds": duration_seconds_ceil(timeout),
        "include_metadata": false
    })
}
```

`ensure_logical_session` передаёт builder тот же `timeout`, который передаётся
в `transport.call`; transport deadline остаётся окончательной границей.

- [ ] **Step 5: Запустить GREEN и workspace-service regression suite**

```bash
cargo test -p unica-coder rlm_request_preserves_caller_budget_above_service_default
cargo test -p unica-coder rlm_readiness_preserves_caller_budget_above_service_default
cargo test -p unica-coder rlm_start_uses_remaining_caller_deadline_for_execution_timeout
cargo test -p unica-coder workspace_services -- --test-threads=1
```

Expected: PASS.

### Task 4: Синхронизировать архитектурный контракт

**Files:**
- Modify: `docs/design/2026-08-09-operational-code-config-design.md`
- Modify: `spec/decisions/0039-workspace-operational-config-snapshot.md`
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/architecture/runtime.md`
- Modify: `spec/architecture/building-blocks.md`
- Modify: `spec/architecture/tool-surface.md`
- Modify: `plugins/unica/skills/code-diagnostics/SKILL.md`
- Modify: `docs/plans/2026-08-10-pr-398-operational-config-integration.md`

**Interfaces:**
- Produces: один согласованный текст: root без `version`, operational values
  `>= 1` без верхнего policy-limit, defaults не являются ceilings, explicit
  diagnostics argument по-прежнему `30..=3600`.

- [ ] **Step 1: Исправить design, ADR и registry Rule**

Удалить формулировки про `version`, hard ceilings и сохранение caps 45/120.
Указать, что caller budget ограничивает только затронутый work request, а
control timeouts сервиса остаются внутренними defaults.

- [ ] **Step 2: Исправить runtime, building blocks, skill и tool-surface**

Не смешивать файловый fallback без верхнего предела с публичным explicit
argument `30..=3600`.

- [ ] **Step 3: Выполнить documentation gates**

```bash
uv run --with-requirements tests/ci/requirements.txt python -m unittest \
  tests.ci.test_architecture_registry \
  tests.ci.test_design_documents \
  tests.ci.test_unica_skills
python3.12 scripts/ci/check-architecture-sync.py
```

Expected: PASS.

### Task 5: Удалить недостижимый version-specific diagnostic vocabulary

**Files:**
- Modify: `crates/unica-coder/src/domain/operational_config.rs`
- Test: существующие root/operational tests из Task 1 и Task 2.

**Interfaces:**
- Produces: `OperationalConfigDiagnosticCode` не содержит `MissingField` и
  `UnsupportedVersion`; ни один version-specific code/message нельзя создать
  внутри крейта после того, как `version` стал обычным неизвестным root field.

- [ ] **Step 1: Подтвердить, что варианты недостижимы**

```bash
rg -n 'MissingField|UnsupportedVersion' crates/unica-coder/src
cargo test -p unica-coder rejects_version_as_unknown_root_field
```

Expected: оба имени встречаются только в объявлении enum и его `message` match;
поведенческий root test уже GREEN после зафиксированного RED Task 1. Это
REFACTOR-фаза того же TDD-цикла, а не новое поведение.

- [ ] **Step 2: Удалить два варианта и их сообщения**

```rust
pub enum OperationalConfigDiagnosticCode {
    ReadFailed,
    InvalidToml,
    UnknownField,
    InvalidType,
    OutOfRange,
    InconsistentValues,
}
```

- [ ] **Step 3: Проверить закрытый vocabulary и регрессии**

```bash
cargo test -p unica-coder rejects_version_as_unknown_root_field
cargo test -p unica-coder operational_config -- --test-threads=1
cargo clippy -p unica-coder --all-targets --all-features -- -D warnings
rg -n 'MissingField|UnsupportedVersion' crates/unica-coder/src
```

Expected: tests/clippy PASS; последний `rg` не возвращает совпадений.

- [ ] **Step 4: Зафиксировать cleanup**

```bash
git add crates/unica-coder/src/domain/operational_config.rs
git commit -m "refactor(config): удалить version-specific diagnostics"
```

### Task 6: Интегрировать актуальный `main` и выполнить полный gate

**Files:** все изменённые файлы PR #398.

**Interfaces:**
- Produces: локальный проверенный head, готовый после независимого final review
  заменить `korolevpavel/feat/issue-338-operational-config`.

- [ ] **Step 1: Зафиксировать этот фактически исполненный план**

```bash
git add docs/plans/2026-08-10-pr-398-unversioned-unbounded-operational-config.md
git commit -m "docs(config): добавить план снятия скрытых пределов"
```

- [ ] **Step 2: Обновить внешнее состояние и интегрировать `main`**

```bash
git fetch origin main
gh pr view 398 --repo IngvarConsulting/unica --json headRefOid,headRefName
git merge --no-ff origin/main
```

Expected: remote PR head всё ещё `e686f598`; актуальный `main` на старте Task 6
равен `d7818b5e`. Если head автора изменился, сначала интегрировать его новые
коммиты. Merge с `main` должен сохранять обе независимые ветки изменений без
перенумерации ADR-0039, пока в актуальном `main` последний номер — ADR-0038.

- [ ] **Step 3: Выполнить полный verification gate**

```bash
cargo fmt --all -- --check
cargo test --workspace --all-targets --all-features -- --test-threads=1
cargo clippy --workspace --all-targets --all-features -- -D warnings
uv run --with-requirements tests/ci/requirements.txt python -m unittest discover -s tests/ci
git diff --check origin/main...HEAD
```

- [ ] **Step 4: Проверить историю, diff и подготовить delivery evidence**

```bash
git status --short
git diff --stat e686f598..HEAD
gh pr view 398 --repo IngvarConsulting/unica --json headRefOid,headRefName
```

В `task-6-report.md` приложить новый PR body без `hard ceilings`, файлового
`version` и ложного утверждения о старом 45/120 cap. Push, обновление PR body и
ожидание GitHub checks выполняются только после task review и общего final
review ветки.
