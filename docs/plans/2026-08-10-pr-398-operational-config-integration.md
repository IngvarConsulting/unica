# Интеграция PR #398 с общим контейнером конфигурации — план реализации

> **Для agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Довести PR #398 до совместимого с текущим `main` состояния, сохранив
сетевую политику ADR-0032, добавив операционные сроки ADR-0038 и устранив гонку
между cancellation и ошибкой загрузчика.

**Architecture:** Один инфраструктурный модуль разбирает синтаксис TOML,
проверяет общую `version` и закрытый корень `version|operational|network|providers`.
Каждый потребитель строго проверяет только своё поддерево и сливает свои поля.
Application проверяет cancellation после любого результата загрузчика до
проекции ошибки конфигурации.

**Tech Stack:** Rust 2021, `toml = "0.8"`, Cargo workspace tests, Python 3.12
architecture guards.

## Global Constraints

- Публичный MCP остаётся одним сервером `unica`; новые tools, arguments и result
  fields не добавляются.
- `unica.toml` и `unica.local.toml` принадлежат общему контейнеру ADR-0032 и
  ADR-0038; второго config root нет.
- Network-only файл без `version` остаётся совместимым.
- Слой с `[operational]` требует `version = 1`; любая присутствующая `version`
  обязана быть целым числом `1`.
- Неизвестный корневой ключ отказывает всем config-consuming вызовам;
  неизвестный ключ потребительского поддерева отказывает только его вызовам.
- Для каждого дефекта сначала запускается тест, падающий на текущем коде по
  ожидаемой причине, и только затем меняется production code.

---

### Task 1: Синхронизация существующего PR с `main`

**Files:**
- Modify: `Cargo.lock`
- Modify: `Cargo.toml`
- Modify: `crates/unica-coder/Cargo.toml`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `spec/decisions/README.md`

**Interfaces:**
- Consumes: PR head `6ffba393`, `origin/main` `282dd987`.
- Produces: одна история существующего PR с сохранёнными
  `prepare_tool_invocation` current main и operational snapshot PR #398.

- [ ] **Step 1: Зафиксировать одобренные design, ADR-0038 и этот план**

```bash
git add docs/design/2026-08-09-operational-code-config-design.md \
  docs/plans/2026-08-10-pr-398-operational-config-integration.md \
  spec/architecture/invariants.md spec/decisions/README.md \
  spec/decisions/0033-workspace-operational-config-snapshot.md \
  spec/decisions/0038-workspace-operational-config-snapshot.md
git commit -m "docs(config): согласовать операционный снимок с ADR-0032"
```

- [ ] **Step 2: Влить текущий `main` без переписывания авторских коммитов**

```bash
git merge --no-ff origin/main
```

Expected: конфликты в `Cargo.lock`, `application/mod.rs`, `application/ports.rs`
и `spec/decisions/README.md`.

- [ ] **Step 3: Разрешить конфликты по владельцам контрактов**

В `application/mod.rs` последовательность вызова остаётся такой:

```rust
let prepared = ports.prepare_tool_invocation(spec, args, &context, cancellation)?;
let operational_config = operational_config::resolve_for_call(ports, spec, args, &context)?;
// dispatch получает и prepared, и operational_config; ни одна ветка не теряется.
```

В workspace dependencies оставить один `toml = "0.8"`; `Cargo.lock`
перегенерировать `cargo check -p unica-coder`. В индекс ADR включить все
принятые записи `0032`…`0038` в числовом порядке.

- [ ] **Step 4: Проверить интегрированную компиляцию до функциональных правок**

```bash
cargo check -p unica-coder
python3.12 -m unittest tests.ci.test_architecture_registry tests.ci.test_design_documents
```

Expected: PASS; функциональная несовместимость config parser-ов пока остаётся и
будет воспроизведена Task 2.

### Task 2: Общий строгий корень workspace config

**Files:**
- Create: `crates/unica-coder/src/infrastructure/workspace_config.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/operational_config.rs`
- Modify: `crates/unica-coder/src/infrastructure/documentation_policy.rs`
- Test: те же три Rust-модуля под `#[cfg(test)]`.

**Interfaces:**
- Produces:

```rust
pub(crate) enum WorkspaceConfigRootErrorKind {
    InvalidToml,
    MissingVersion,
    InvalidVersionType,
    UnsupportedVersion,
    UnknownField,
}

pub(crate) struct WorkspaceConfigRootError {
    kind: WorkspaceConfigRootErrorKind,
    field_path: String,
}

pub(crate) fn parse_workspace_config_root(
    contents: &str,
) -> Result<toml::Table, WorkspaceConfigRootError>;
```

`parse_workspace_config_root` принимает только корневые поля `version`,
`operational`, `network`, `providers`; требует `version = 1` при наличии
`operational` и проверяет любую присутствующую версию.

- [ ] **Step 1: Написать падающие consumer-level тесты совместимости**

```rust
#[test]
fn network_only_layer_without_version_keeps_operational_defaults() {
    // unica.toml: [network] default = "deny"
    // load_operational_config обязан вернуть compiled defaults.
}

#[test]
fn mixed_versioned_container_serves_network_and_operational_consumers() {
    // version = 1 + [network] + [operational.code_intelligence]
    // оба настоящих loader-а обязаны успешно прочитать свои значения.
}

#[test]
fn documentation_policy_ignores_valid_operational_subtree() {
    // operational-поля не становятся неизвестными network-секциями.
}
```

Production mutations caught: возврат независимых `ROOT_FIELDS` в одном из
loader-ов, безусловное требование `version` для прежнего сетевого слоя и
отклонение известной соседней секции.

- [ ] **Step 2: Запустить RED**

```bash
cargo test -p unica-coder network_only_layer_without_version_keeps_operational_defaults
cargo test -p unica-coder mixed_versioned_container_serves_network_and_operational_consumers
cargo test -p unica-coder documentation_policy_ignores_valid_operational_subtree
```

Expected: FAIL — текущий operational parser отвергает `network`, а network
parser отвергает `version`/`operational`.

- [ ] **Step 3: Реализовать минимальный общий root parser**

```rust
const ROOT_FIELDS: &[&str] = &["version", "operational", "network", "providers"];

pub(crate) fn parse_workspace_config_root(
    contents: &str,
) -> Result<toml::Table, WorkspaceConfigRootError> {
    let root = contents.parse::<toml::Table>().map_err(|_| invalid_toml())?;
    reject_unknown_root_fields(&root, ROOT_FIELDS)?;
    validate_present_version(&root)?;
    if root.contains_key("operational") && !root.contains_key("version") {
        return Err(missing_version());
    }
    Ok(root)
}
```

`documentation_policy::parse_policy` обрабатывает `network|providers`, а
`version|operational` пропускает как известные соседние поля. Operational
loader обрабатывает `operational`, а `network|providers` оставляет сетевому
потребителю. Оба преобразуют общий error в свой существующий безопасный
диагностический контракт.

- [ ] **Step 4: Запустить GREEN и регрессионные suites обоих потребителей**

```bash
cargo test -p unica-coder workspace_config -- --test-threads=1
cargo test -p unica-coder operational_config -- --test-threads=1
cargo test -p unica-coder documentation_policy -- --test-threads=1
cargo test -p unica-coder standards_documentation -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 5: Зафиксировать общий контейнер**

```bash
git add Cargo.toml Cargo.lock crates/unica-coder
git commit -m "fix(config): объединить корневой контракт workspace config"
```

### Task 3: Cancellation выигрывает у результата loader-а

**Files:**
- Modify: `crates/unica-coder/src/application/mod.rs`
- Test: `crates/unica-coder/src/application/mod.rs`.

**Interfaces:**
- Consumes: `CancellationToken`, `operational_config::resolve_for_call`.
- Produces: единая post-load arbitration до проекции `Ok` или `Err` loader-а.

- [ ] **Step 1: Написать падающий тест гонки**

```rust
#[test]
fn cancellation_during_failed_operational_config_load_wins() {
    let token = CancellationToken::new();
    let ports = Arc::new(OperationalConfigRecordingPorts::failing_and_cancelling(token.clone()));
    let app = UnicaApplication::with_ports(ports);
    let error = app
        .call_tool_cancellable("unica.code.search", &json!({"query":"needle"}).as_object().unwrap(), token)
        .expect_err("cancellation must win over invalid config");
    assert!(crate::domain::cancellation::is_cancelled_error(&error));
}
```

Production mutation caught: немедленный `return Ok(invalid-config)` из `Err`
ветки до повторной проверки токена.

- [ ] **Step 2: Запустить RED**

```bash
cargo test -p unica-coder cancellation_during_failed_operational_config_load_wins
```

Expected: FAIL — текущий код возвращает `OperationResult { ok: false }`.

- [ ] **Step 3: Выполнить post-load arbitration до match**

```rust
let requires_snapshot = operational_config::requires_snapshot(spec, args);
if requires_snapshot && cancellation.is_cancelled() { /* cancelled before */ }
let resolved_config = operational_config::resolve_for_call(ports, spec, args, &context);
if requires_snapshot && cancellation.is_cancelled() { /* cancelled after */ }
let operational_config = match resolved_config { /* project Ok or diagnostic */ };
```

- [ ] **Step 4: Запустить GREEN и application regression tests**

```bash
cargo test -p unica-coder cancellation_during_failed_operational_config_load_wins
cargo test -p unica-coder operational_config -- --test-threads=1
cargo test -p unica-coder cancellation -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 5: Зафиксировать арбитрацию cancellation**

```bash
git add crates/unica-coder/src/application/mod.rs
git commit -m "fix(config): сохранить приоритет отмены при ошибке loader-а"
```

### Task 4: Нормативная синхронизация интегрированного контракта

**Files:**
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/architecture/runtime.md`
- Modify: `spec/architecture/building-blocks.md`
- Modify: `spec/decisions/README.md`
- Modify: `spec/decisions/0038-workspace-operational-config-snapshot.md`
- Modify: `docs/design/2026-08-09-operational-code-config-design.md`

**Interfaces:**
- Produces: `INV-APP-CONFIG-SNAPSHOT` с владельцем ADR-0038 и
  `INV-APP-DOCUMENTATION-NETWORK-POLICY` со ссылками ADR-0032 + ADR-0038.

- [ ] **Step 1: Синхронизировать Rule обоих потребителей**

В Rule явно закрепить общий закрытый корень, совместимость прежнего сетевого слоя,
обязательную версию operational-слоя и локальность ошибок поддеревьев.

- [ ] **Step 2: Проверить нормативные guards**

```bash
uv run --with PyYAML==6.0.3 python -m unittest \
  tests.ci.test_architecture_registry \
  tests.ci.test_design_documents \
  tests.ci.test_unica_skills
python3.12 scripts/ci/check-architecture-sync.py
```

Expected: PASS.

- [ ] **Step 3: Зафиксировать документацию**

```bash
git add docs/design docs/plans spec/architecture spec/decisions
git commit -m "docs(config): закрепить общий контейнер ADR-0038"
```

### Task 5: Полная проверка и доставка в существующий PR

**Files:** все изменённые файлы PR #398 после merge.

**Interfaces:**
- Produces: проверенный head существующей ветки
  `korolevpavel/feat/issue-338-operational-config`.

- [ ] **Step 1: Выполнить полный verification gate**

```bash
cargo fmt --all -- --check
cargo test --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
uv run --with PyYAML==6.0.3 python -m unittest discover -s tests/ci
git diff --check origin/main...HEAD
git status --short
```

Expected: все команды PASS; статус содержит только ожидаемые коммиты, рабочее
дерево пусто.

- [ ] **Step 2: Провести независимое semantic review**

Reviewer обязан проверить общий root parser, потребительскую локальность
ошибок, сохранение `prepare_tool_invocation`, cancellation arbitration и
соответствие ADR-0032/ADR-0038.

- [ ] **Step 3: Обновить именно существующую head-ветку PR**

```bash
git push https://github.com/korolevpavel/unica.git \
  HEAD:feat/issue-338-operational-config
```

Перед push повторно проверить `gh pr view 398 --json headRefOid,headRefName`;
если head изменился, остановиться и сначала интегрировать новые коммиты автора.

- [ ] **Step 4: Дождаться GitHub checks**

```bash
gh pr checks 398 --repo IngvarConsulting/unica --watch
```

Expected: обязательные checks завершены успешно на новом head.
