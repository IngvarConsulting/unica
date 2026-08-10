# Reader Invocation Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Исправить #297: убрать `dryRun` из всех read-only инструментов, провести через application предметные режимы `Read | Preview | Apply`, всегда выполнять настоящее чтение и отклонять успешный typed-reader без `data`.

**Architecture:** `ToolSpec` становится единственным владельцем двух ортогональных классификаций: `ToolExecution::{Read, Mutation}` и `ResultContract::{Typed, ExternalStream}`. После проверки формы application выводит `InvocationMode`, reader-specific функции режима не получают, а общий порт принимает enum вместо двусмысленного boolean. Постусловие `Read + Typed + ok => data.is_some()` проверяется централизованно до событий и cache-report. Skill parity выводит категорию из `tools/list`, исполняет мутации в preview, а readers — над локальными фикстурами и stand-ins без сети и установленной платформы.

**Tech Stack:** Rust 2021, `serde_json`, `rmcp`, Python 3.12 `unittest`, локальные JSON-RPC/HTTP stand-ins, существующие XML/DSL-фикстуры Unica.

## Global Constraints

- [ ] Proposal PR #426 не сливать. После явного подтверждения реализации закрыть #426 без merge, обновить `origin/main`, убедиться через `git cat-file -e origin/main:spec/decisions/0042-readers-ne-prinimayut-preview.md`, что ADR-0042 в `main` отсутствует, и создать `codex/issue-297-reader-invocation` непосредственно от `origin/main`. Перенести в implementation branch утверждённые design, ADR-0042 и этот план как обычные файлы того же связного changeset. Если ADR-0042 уже появилась в `main`, остановиться: менять её статус запрещает `INV-DOC-SUPERSEDE-NOT-EDIT`, а новый номер требует отдельного решения пользователя.
- [ ] Не включать #298–#301 и не менять контракт preview/apply мутаторов из #290: отсутствие `dryRun` у мутации по-прежнему означает preview, `true` — preview, `false` — apply.
- [ ] Не сохранять alias `dryRun` у readers: версия 0.12.0 ещё не выпущена, совместимый двусмысленный маршрут не нужен.
- [ ] Не добавлять `outputSchema`, не менять JSON-RPC code/shape ошибок и не подставлять `{}`, `null` либо иной синтетический `data`.
- [ ] Не обращаться из тестов к живому `ai.v8std.ru`, локальной установке 1С, пользовательскому RLM/cache или установленным на машине бинарям.
- [ ] На каждом дефекте соблюдать red → green: сначала запустить новый тест на текущем коде и сохранить в логе точную причину падения, только затем менять production code.
- [ ] После каждого task просматривать `git diff --check` и `git status --short`; коммиты из плана — самостоятельные точки ревью, но один итоговый implementation PR.

---

### Task 0: Закрыть proposal без merge и создать независимую implementation branch

**Files:**

- Import after branch creation: `docs/design/2026-08-10-reader-invocation-mode-design.md`
- Import after branch creation: `docs/plans/2026-08-10-reader-invocation-mode.md`
- Import after branch creation: `spec/decisions/0042-readers-ne-prinimayut-preview.md`
- Import after branch creation: `spec/decisions/README.md`

- [ ] **Step 1: Verify the explicit execution approval and proposal state.**

Не считать утверждение design автоматическим разрешением закрыть PR или начать
runtime-изменения. В новой сессии получить явное «реализуй»/«делай» от
пользователя, затем:

```bash
gh pr view 426 --json state,isDraft,baseRefName,headRefName,url
git status --short --branch
```

Expected: #426 открыт с base `main`, head
`codex/issue-297-reader-invocation-design`; worktree proposal чистый.

- [ ] **Step 2: Close #426 without merge.**

```bash
gh pr close 426 --comment "Проект ADR-0042 утверждён. Закрываю proposal без merge: implementation PR от main перенесёт эти документы вместе с кодом, тестами и accepted ADR, не нарушая INV-DOC-SUPERSEDE-NOT-EDIT."
gh pr view 426 --json state,mergedAt
```

Expected: `state` равен `CLOSED`, `mergedAt` равен `null`.

- [ ] **Step 3: Refresh main and prove ADR-0042 has not become history.**

```bash
git fetch origin main codex/issue-297-reader-invocation-design
git cat-file -e origin/main:spec/decisions/0042-readers-ne-prinimayut-preview.md
```

Expected: `git cat-file` завершается code 1. Code 0 — жёсткая остановка и
запрос решения пользователя: существующую в target branch ADR-0042 менять
нельзя.

- [ ] **Step 4: Create an isolated implementation worktree from main.**

Use `superpowers:using-git-worktrees` and create branch
`codex/issue-297-reader-invocation` from `origin/main`. Не переключать текущий
proposal worktree и не переиспользовать его ветку.

- [ ] **Step 5: Import the reviewed proposal as a squash, not as a PR base.**

В новом implementation worktree:

```bash
git merge --squash origin/codex/issue-297-reader-invocation-design
git diff --cached --check
git commit -m "docs(design): approve reader invocation boundary"
git merge-base --is-ancestor origin/codex/issue-297-reader-invocation-design HEAD
```

Expected: squash commit содержит design, plan и proposed ADR-0042;
`merge-base --is-ancestor` завершается code 1, доказывая отсутствие parent/base
зависимости от proposal head.

---

### Task 1: Типизировать реестр и сузить published schemas

**Files:**

- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Modify: `crates/unica-coder/src/application/metadata.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/composition.rs`
- Modify: `crates/unica-coder/src/interfaces/mcp.rs`
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/format_guard.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/registry.rs`
- Modify: `crates/unica-coder/src/application/meta_info_surface_tests.rs`
- Modify: `crates/unica-coder/src/application/meta_add_surface_tests.rs`
- Modify: `crates/unica-coder/src/application/meta_remove_surface_tests.rs`
- Modify: `crates/unica-coder/tests/format_8_3_27_xml_corpus.rs`
- Modify: `crates/unica-coder/tests/platform_meta_surface.rs`
- Modify: `tests/ci/test_meta_surface_contract.py`

- [ ] **Step 1: Write the failing schema/registry tests.**

В `application::tests` добавить таблицу по всем 73 `ToolSpec`. До production-правки тест использует существующий `mutating`, чтобы red был поведенческим, а не только compile-failure:

```rust
#[test]
fn reader_schemas_never_publish_dry_run_and_mutations_keep_it() {
    for tool in tools() {
        let schema = input_schema_for_tool(&tool);
        let properties = schema["properties"].as_object().unwrap();
        assert_eq!(
            properties.contains_key("dryRun"),
            tool.mutating,
            "{} publishes the wrong invocation switch",
            tool.name,
        );
    }
}
```

Отдельно проверить `unica.meta.info` и три metadata mutation, потому что их schema строится ранним special-case в `metadata_input_schema`.

- [ ] **Step 2: Run the red test.**

Run:

```bash
cargo test -p unica-coder reader_schemas_never_publish_dry_run_and_mutations_keep_it -- --nocapture
```

Expected: FAIL на первом non-metadata reader, потому что `COMMON_ARGS` публикует `dryRun` независимо от `mutating`.

- [ ] **Step 3: Add the typed registry categories.**

В `application/mod.rs` рядом с `ToolSpec` ввести:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolExecution {
    Read,
    Mutation,
}

impl ToolExecution {
    pub const fn is_mutating(self) -> bool {
        matches!(self, Self::Mutation)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultContract {
    Typed,
    ExternalStream,
}

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub execution: ToolExecution,
    pub result_contract: ResultContract,
    pub cache_access: CacheAccess,
    pub handler: ToolHandler,
}
```

Заменить все 73 `mutating: true/false` на `execution: ToolExecution::Mutation/Read`. `ResultContract::Typed` назначить ровно 47 инструментам, для которых `tool-surface-review.json` содержит одновременно `scope: "in"` и `result.contract: "typed"`; остальным 26 (`prose`, `partial`, `job`, а также scope `retiring`/`runtime`) назначить `ExternalStream`.

- [ ] **Step 4: Make the shared schema conditional on `ToolExecution`.**

Изменить:

```rust
const COMMON_ARGS: &[&str] = &["cwd", "confirm"];

fn allowed_args(tool: &ToolSpec) -> Vec<&'static str> {
    let mut names = COMMON_ARGS.to_vec();
    if tool.execution.is_mutating() {
        names.push("dryRun");
    }
    // existing handler-specific additions follow
}
```

Не добавлять `dryRun` в `MetadataOperation::Info`; сохранить его у Add/Edit/Remove. Переписать описание аргумента на «preview switch for mutation tools», убрав утверждение «present on every tool».

- [ ] **Step 5: Replace boolean registry reads without changing behavior yet.**

В `composition.rs`, format/support guards, native registry и тестах заменить `tool.mutating` на `tool.execution.is_mutating()`. В адаптерах пока оставить их внутренний параметр `mutating: bool`: на этом task передавать `spec.execution.is_mutating()`. Не вводить `InvocationMode` частями.

Обновить структурные Python assertions в `test_meta_surface_contract.py` с regex `mutating:` на `execution: ToolExecution::...`; assertions должны по-прежнему доказывать одну read и три mutation metadata operations.

- [ ] **Step 6: Run focused green tests.**

Run:

```bash
cargo fmt --all
cargo test -p unica-coder reader_schemas_never_publish_dry_run_and_mutations_keep_it -- --nocapture
cargo test -p unica-coder application::tool_contracts -- --test-threads=1
cargo test -p unica-coder --test platform_meta_surface -- --test-threads=1
python3.12 -m unittest tests.ci.test_meta_surface_contract
```

Expected: PASS; `tools/list`-schema readers не имеют `dryRun`, mutations сохраняют его.

- [ ] **Step 7: Commit.**

```bash
git add crates/unica-coder/src crates/unica-coder/tests tests/ci/test_meta_surface_contract.py
git commit -m "refactor(mcp): classify reader and mutation tools"
```

---

### Task 2: Ввести `InvocationMode` и двухфазную validation boundary

**Files:**

- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Modify: `crates/unica-coder/src/application/metadata.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_state.rs`

- [ ] **Step 1: Write failing mode and pre-discovery tests.**

Добавить `RejectDiscoveryPorts`, у которого `discover_workspace` увеличивает `AtomicUsize` и паникует. Для каждого `ToolExecution::Read` вызвать public application с `dryRun: true` и `dryRun: false`, проверить:

```rust
assert!(error.contains("does not accept argument `dryRun`"), "{tool}: {error}");
assert_eq!(ports.discovery_calls.load(Ordering::SeqCst), 0);
```

Для metadata reader принять его существующий префикс `metadata operation does not accept argument`, но также требовать ноль вызовов порта.

Добавить compile-red test для нового enum:

```rust
assert_eq!(InvocationMode::from_validated_args(reader, &Map::new()).unwrap(), InvocationMode::Read);
assert_eq!(InvocationMode::from_validated_args(mutation, &Map::new()).unwrap(), InvocationMode::Preview);
assert_eq!(InvocationMode::from_validated_args(mutation, &json!({"dryRun": false}).as_object().unwrap()).unwrap(), InvocationMode::Apply);
```

- [ ] **Step 2: Run the red tests.**

Run:

```bash
cargo test -p unica-coder reader_rejects_dry_run_before_workspace_discovery -- --nocapture
cargo test -p unica-coder invocation_mode_is_derived_from_validated_tool_execution -- --nocapture
```

Expected: первый тест PASS после Task 1; второй не компилируется, потому что предметного режима ещё нет. Зафиксировать оба результата: первый защищает порядок, второй создаёт новый API.

- [ ] **Step 3: Split shape validation from semantic validation.**

В `tool_contracts.rs` выделить:

```rust
pub fn validate_tool_argument_shape(tool: ToolSpec, args: &Map<String, Value>) -> Result<(), String>;
pub fn validate_tool_argument_semantics(
    tool: ToolSpec,
    args: &Map<String, Value>,
    mode: InvocationMode,
) -> Result<(), String>;
```

Shape-stage выполняет closed-name и top-level type checks. Для metadata вынести из `parse_metadata_request` package-visible `validate_metadata_argument_shape`: он переиспользует `metadata_top_level_fields`, проверяет top-level JSON-типы и ничего не читает/не пишет. Полный `parse_metadata_request` остаётся semantic-stage и не дублирует unknown-field check.

Все прежние semantic validators получают `InvocationMode` либо вычисленный `mode.is_preview()` только там, где условие действительно относится к мутации. Проверка required args использует `mode`: preview может сохранять существующие послабления мутаций, `Read` и `Apply` требуют полный payload.

- [ ] **Step 4: Add and derive `InvocationMode` only after shape validation.**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationMode {
    Read,
    Preview,
    Apply,
}

impl InvocationMode {
    fn from_validated_args(spec: ToolSpec, args: &Map<String, Value>) -> Result<Self, String> {
        match spec.execution {
            ToolExecution::Read => Ok(Self::Read),
            ToolExecution::Mutation => match args.get("dryRun") {
                None | Some(Value::Bool(true)) => Ok(Self::Preview),
                Some(Value::Bool(false)) => Ok(Self::Apply),
                Some(_) => Err(format!("{} argument `dryRun` must be a boolean", spec.name)),
            },
        }
    }

    pub const fn is_preview(self) -> bool { matches!(self, Self::Preview) }
    pub const fn is_apply(self) -> bool { matches!(self, Self::Apply) }
}
```

В `call_tool` порядок должен быть именно таким:

```rust
let normalized_args = normalize_native_path_aliases(spec, args)?;
validate_tool_argument_shape(spec, &normalized_args)?;
let mode = InvocationMode::from_validated_args(spec, &normalized_args)?;
validate_tool_argument_semantics(spec, &normalized_args, mode)?;
let context = ports.discover_workspace(cwd)?;
```

- [ ] **Step 5: Replace mixed port booleans with the enum.**

`validate_tool_context`, `prepare_tool_invocation`, `invoke_handler` и `cache_report` принимают `InvocationMode`. В `WorkspaceStateRepository::report` пока передать `mode.is_preview()`; `Read` тем самым остаётся `false`, не создаёт событий и получает cache mode `read`.

В mutation-only адаптеры преобразовывать boolean только через exhaustive match, а не `mode.is_preview()` без доказанной категории:

```rust
let dry_run = match (spec.execution, mode) {
    (ToolExecution::Mutation, InvocationMode::Preview) => true,
    (ToolExecution::Mutation, InvocationMode::Apply) => false,
    (ToolExecution::Read, InvocationMode::Read) => false,
    _ => return Err(format!("invalid invocation mode for {}", spec.name)),
};
```

- [ ] **Step 6: Run green and mutation-regression tests.**

Run:

```bash
cargo fmt --all
cargo test -p unica-coder invocation_mode_ -- --nocapture
cargo test -p unica-coder reader_rejects_dry_run_before_workspace_discovery -- --nocapture
cargo test -p unica-coder preview -- --test-threads=1
cargo test -p unica-coder runtime_profile_guard -- --test-threads=1
```

Expected: PASS; default/true/false mutation behavior не изменилось.

- [ ] **Step 7: Commit.**

```bash
git add crates/unica-coder/src
git commit -m "refactor(application): derive explicit invocation modes"
```

---

### Task 3: Удалить preview из reader-specific ветвей

**Files:**

- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/source_resources.rs`
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/typed_result.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations.rs`

- [ ] **Step 1: Rewrite the two tests that currently bless the defect.**

Заменить:

- `public_subsystem_info_dry_run_does_not_read_a_missing_target` на rejection-test `public_subsystem_info_rejects_dry_run_before_reading_target`;
- `public_subsystem_validate_dry_run_does_not_read_a_missing_target` на такое же правило для ExternalStream reader;
- `the_documentation_branch_previews_instead_of_searching_on_dry_run` на `documentation_reader_polls_provider_without_an_invocation_switch`, который вызывает reader branch и требует ровно один вызов `RecordingProvider` и typed `data`.

Добавить прямые infrastructure tests для native typed reader, code search/read, source resources, standards, documentation, project и read-only runtime-job routes: `InvocationMode::Read` должен доходить до предметного обработчика; `Preview`/`Apply` с `ToolExecution::Read` должны отклоняться как внутреннее несоответствие.

- [ ] **Step 2: Run the rewritten tests before removing branches.**

Run:

```bash
cargo test -p unica-coder public_subsystem_info_rejects_dry_run_before_reading_target -- --nocapture
cargo test -p unica-coder documentation_reader_polls_provider_without_an_invocation_switch -- --nocapture
```

Expected: первый PASS благодаря boundary Task 2; второй compile/behavior FAIL, пока infrastructure API всё ещё принимает dry-run и умеет вернуть placeholder.

- [ ] **Step 3: Remove mode parameters from reader-only functions.**

Удалить `dry_run`/`InvocationMode` из сигнатур:

- `invoke_code_intelligence_search`;
- `invoke_code_intelligence_read`;
- `source_resources::invoke`;
- source-navigation reader functions (если enum дошёл до них механически);
- documentation search/get helpers;
- prepared `subsystem.info` read path.

Reader-specific функция должна компилироваться только как чтение; режим остаётся у общего orchestrator и mixed infrastructure port.

- [ ] **Step 4: Delete all reader placeholder branches.**

В `typed_result.rs` убрать `if !dry_run` вокруг `cf-info`, `role-info`, `cfe-diff`, `dcs-info`, `form-info`, `mxl-info`, `subsystem-info`; match читателей выполняется безусловно до generic adapter fallback.

В `application/mod.rs` удалить early returns с `dry run: ...provider-neutral...` из code intelligence.

В `infrastructure/application_ports.rs`:

- `prepare_tool_invocation` для `subsystem-info` всегда готовит read;
- documentation search/get всегда строят registry и вызывают provider;
- project readers всегда читают;
- runtime job status/wait/logs/list вызывают адаптер с доказанным `dry_run = false`;
- `CodeAdapter` graph/analyze и read-only validate/decompile branches получают `false` только из `(Read, Read)` exhaustive branch.

Generic `NativeOperationAdapter::invoke` продолжает получать boolean для mutations/external CLI, но native typed readers больше не достигают его как preview fallback.

- [ ] **Step 5: Run focused green tests.**

Run:

```bash
cargo fmt --all
cargo test -p unica-coder application::tests::public_subsystem_ -- --test-threads=1
cargo test -p unica-coder infrastructure::application_ports::tests::the_documentation_ -- --test-threads=1
cargo test -p unica-coder code_intelligence -- --test-threads=1
cargo test -p unica-coder source_resources -- --test-threads=1
cargo test -p unica-coder runtime_job -- --test-threads=1
```

Expected: PASS; ни один reader-specific API не принимает preview flag.

- [ ] **Step 6: Commit.**

```bash
git add crates/unica-coder/src
git commit -m "fix(readers): always execute subject reads"
```

---

### Task 4: Добавить fail-closed постусловие typed result и синхронизацию с ledger

**Files:**

- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/typed_result.rs`
- Modify: `tests/ci/test_tool_surface_ledger.py`

- [ ] **Step 1: Write the failing result-contract tests.**

Использовать `FixedOutcomePorts` и добавить четыре cases:

```rust
#[test]
fn successful_typed_reader_without_data_fails_closed() {
    let error = app_with(AdapterOutcome::ok("lost payload"), None)
        .call_tool("unica.project.status", &Map::new())
        .unwrap_err();
    assert!(error.starts_with("typed_result_missing:"), "{error}");
}
```

Также закрепить:

- `Read + Typed + ok:false + data:none` возвращает обычный `OperationResult` с `ok:false`;
- `Mutation + Typed + ok:true + data:none` этим ADR не блокируется;
- `Read + ExternalStream + ok:true + data:none` не блокируется.

Добавить registry/ledger test, читающий `../../spec/architecture/tool-surface-review.json` и для каждого tool вычисляющий ожидаемое значение:

```rust
let expected = if entry["scope"] == "in" && entry["result"]["contract"] == "typed" {
    ResultContract::Typed
} else {
    ResultContract::ExternalStream
};
assert_eq!(tool.result_contract, expected, "{}", tool.name);
```

- [ ] **Step 2: Run the red tests.**

Run:

```bash
cargo test -p unica-coder successful_typed_reader_without_data_fails_closed -- --nocapture
cargo test -p unica-coder tool_specs_match_reviewed_result_contracts -- --nocapture
```

Expected: первый FAIL, возвращая успешный result без data; второй показывает любую ошибочную ручную классификацию `ResultContract`.

- [ ] **Step 3: Enforce the postcondition centrally.**

Сразу после получения `HandlerOutcome`, до event projection и `cache_report`, вызвать:

```rust
fn enforce_result_contract(
    spec: ToolSpec,
    mode: InvocationMode,
    outcome: &HandlerOutcome,
) -> Result<(), String> {
    if mode == InvocationMode::Read
        && spec.result_contract == ResultContract::Typed
        && outcome.adapter.ok
        && outcome.data.is_none()
    {
        return Err(format!(
            "typed_result_missing: {} returned ok without OperationResult.data",
            spec.name
        ));
    }
    Ok(())
}
```

Не переносить эту проверку в отдельные native handlers: она обязана покрывать project, source, metadata, code, standards и documentation одной границей. `typed_operation_result` продолжает только сериализовать имеющийся payload; он не создаёт placeholder.

- [ ] **Step 4: Strengthen the ledger guard.**

В `test_tool_surface_ledger.py` сохранить проверку 47 typed entries и добавить assertion, что Rust registry test назван в `INV-MCP-TYPED-RESULT` после Task 7. Не пытаться разбирать Rust literals Python-regex: поведенческий Rust test уже сверяет обе стороны.

- [ ] **Step 5: Run green tests.**

Run:

```bash
cargo fmt --all
cargo test -p unica-coder typed_reader -- --test-threads=1
cargo test -p unica-coder tool_specs_match_reviewed_result_contracts -- --nocapture
python3.12 -m unittest tests.ci.test_tool_surface_ledger
```

Expected: PASS; stable prefix `typed_result_missing:` виден только у внутреннего contract breach.

- [ ] **Step 6: Commit.**

```bash
git add crates/unica-coder/src tests/ci/test_tool_surface_ledger.py
git commit -m "fix(mcp): require data from successful typed readers"
```

---

### Task 5: Закрепить public MCP transport и representative read flows

**Files:**

- Modify: `crates/unica-coder/src/interfaces/mcp.rs`
- Modify: `tests/ci/test_unica_mcp_smoke.py`
- Modify: `tests/ci/test_unica_mcp_script_parity.py`

- [ ] **Step 1: Add failing tools/list and JSON-RPC regression tests.**

В MCP smoke получить `tools/list` и проверить по всем entries:

- 38 readers не публикуют `dryRun`;
- 35 mutations публикуют boolean `dryRun` с default `true`;
- `cwd` и `confirm` не используются как эвристика категории.

Категорию брать из expected registry projection, а не из суффиксов имён.

Через `tools/call` вызвать `unica.project.status` и `unica.subsystem.info` с каждым `dryRun` boolean и несуществующим `cwd`/target. Требовать JSON-RPC execution error с `does not accept argument` — unknown field обязан победить workspace/target error.

Добавить обычные вызовы без `dryRun` для representative project, source, meta и native subsystem readers; проверять `ok:true`, `data` и отсутствие `dry run` в summary.

- [ ] **Step 2: Run the tests against the partially migrated code.**

Run:

```bash
cargo test -p unica-coder interfaces::mcp -- --test-threads=1
python3.12 -m unittest tests.ci.test_unica_mcp_smoke
```

Expected: новые assertions выявляют любой оставшийся schema/transport drift; исправлять только production boundary, не ослаблять тест.

- [ ] **Step 3: Preserve the existing error shape.**

Расширить `tool_execution_failure_keeps_json_rpc_error_shape`: `typed_result_missing:` и reader `dryRun` rejection используют существующий `TOOL_EXECUTION_ERROR`; не вводить новый code и не превращать internal contract breach в `OperationResult { ok:false }`.

- [ ] **Step 4: Run green transport tests and commit.**

```bash
cargo fmt --all
cargo test -p unica-coder interfaces::mcp -- --test-threads=1
python3.12 -m unittest tests.ci.test_unica_mcp_smoke
git add crates/unica-coder/src/interfaces/mcp.rs tests/ci/test_unica_mcp_smoke.py tests/ci/test_unica_mcp_script_parity.py
git commit -m "test(mcp): cover reader invocation contracts"
```

---

### Task 6: Перевести skill parity с fake preview на реальные детерминированные reads

**Files:**

- Modify: `tests/ci/test_unica_mcp_script_parity.py`
- Add: `tests/fixtures/unica_mcp_script_parity/reader-standins/bsl_mcp.py`
- Add: `tests/fixtures/unica_mcp_script_parity/reader-standins/rlm_index.py`
- Add: `tests/fixtures/unica_mcp_script_parity/reader-standins/v8std_response.json`
- Reuse: `tests/fixtures/unica_mcp_script_parity/bsp/**`
- Reuse: `tests/fixtures/unica_mcp_script_parity/{cf-info,cf-validate,cfe-diff,form-validate,interface-validate,role-info,role-validate-predefined-data}/**`
- Reuse: `tests/fixtures/xdto/enterprise-data-minimal/**`

- [ ] **Step 1: Write the new parity assertion before adding fixtures.**

Переименовать тест в `test_every_skill_tools_call_example_executes_by_tool_mode`. Сначала запросить `tools/list`, построить projection:

```python
execution_by_tool = {
    tool["name"]: (
        "mutation"
        if "dryRun" in tool["inputSchema"]["properties"]
        else "read"
    )
    for tool in tools
}
```

Заменить `dry_run_message_for_example` на `execution_message_for_example`:

```python
if execution_by_tool[tool_name] == "mutation":
    arguments["dryRun"] = True
else:
    arguments.pop("dryRun", None)
```

Для каждого successful typed reader требовать `data`; для mutation summary требовать `preview`/`dry run`; для reader запрещать обе фразы. Сохранить byte-for-byte snapshot workspace до и после всей пачки.

- [ ] **Step 2: Run red parity.**

Run:

```bash
python3.12 -m unittest tests.ci.test_unica_mcp_script_parity.UnicaMcpParityTests.test_every_skill_tools_call_example_executes_by_tool_mode
```

Expected: FAIL на первом reader, которому прежний placeholder скрывал отсутствующую цель/поставщика. Это обязательный red; не добавлять allowlist и не возвращаться к schema-only проверке.

- [ ] **Step 3: Materialize exact filesystem reader fixtures.**

Добавить `prepare_skill_reader_fixtures(...)`, который проходит все reader examples и либо материализует их concrete path, либо заменяет только markdown-placeholder (`<path>`, `<путь>`, `<TemplatePath>`, `<OutputPath>`, `<CIPath>`, `<RightsPath>`, `<MetadataPath>`, `<каталог>`) на platform-safe путь внутри temp workspace.

Матрица обязательна:

- CF: `src/cf`, `test-tmp/cf`, `upload/cfempty` и file form получают валидный `Configuration.xml` из существующих `cf-info`/`cf-validate` fixtures;
- CFE: `src/cfe`, `src/extensions/MyExtension` и file form получают валидный extension descriptor; base остаётся `src/cf`;
- DCS: все targets получают копию `BSP_DCS_OBJECT_FIXTURE` в `Ext/Template.xml` и owner descriptor, если selector указывает logical directory;
- Form: concrete `Валюты`, `Номенклатура`, `МояОбработка` и placeholder targets получают `BSP_FORM_BUSINESS_PROCESS_FIXTURE`/`form-validate/Form.xml`;
- Interface: `Subsystems/Продажи/Ext/CommandInterface.xml` и placeholder получают `interface-validate` fixture;
- MXL: concrete и placeholder targets получают `BSP_MXL_*` fixture в `Ext/Template.xml`;
- Role: `ЧтениеНоменклатуры` и placeholder получают валидный `Rights.xml`, а metadata owner — согласованный `Configuration.xml`;
- Subsystem: создать registrations и exact hierarchy `Продажи/ОптовыеПродажи`, используя BSP subsystem/command-interface bytes;
- Meta и XDTO: переиспользовать уже существующие `prepare_meta_*` и `enterprise-data-minimal`, не создавать вторую модель;
- Code search: создать BSL-модули с четырьмя query literals и методом `ОбработкаПроведения`.

Каждый helper должен assert-ить, что обработал все reader tool names, встретившиеся в JSON examples. Необработанный новый reader example — fail с `document:line`, а не тихий fallback.

- [ ] **Step 4: Add isolated code-intelligence stand-ins.**

Создать temp plugin root копированием только контрактных manifests/references из `PLUGIN_ROOT`, а не использовать пользовательскую установку. В current-target `bin/<target>/` положить executable Python stand-ins:

- `bsl_mcp.py` отвечает на MCP initialize/tools/list/tools/call для graph/analyze/search с typed fixture payload и читает только temp BSL source;
- тот же protocol fixture обслуживает persistent `rlm-tools-bsl` definition request;
- `rlm_index.py` реализует status/info contract `rlm-bsl-index` над temp DB marker.

Пересчитать SHA-256 и записать только в temp-копию `third-party/tools.lock.json`. Production lockfile не менять. Перед запуском установить `UNICA_PLUGIN_ROOT` на temp plugin root и `UNICA_CACHE_DIR` на temp cache. Assertions должны доказать, что stand-ins видели ожидаемые `definition` и `graph` calls.

- [ ] **Step 5: Add local v8std and platform-help corpora.**

Для v8std поднять `ThreadingHTTPServer` на `127.0.0.1:0`, отвечающий на POST `/mcp` canned JSON-RPC из `v8std_response.json`; передать endpoint через `UNICA_STANDARDS_MCP_URL`. Сервер считает calls; после test требовать отсутствие запросов на любой другой host.

Для platform help не использовать `UNICA_PLATFORM_HELP_DIR`: он доступен только под Rust `#[cfg(test)]`, а parity запускает обычный бинарь. Реализовать в Python малые helpers `v8_block`, `v8_container` и `hbk_bytes` по формату из `platform_help/container.rs::tests_support`; внутри `FileStorage` положить ZIP с HTML-страницами для трёх search queries и exact locator:

```text
objects/catalog238/ValueTable/methods/GroupBy1290.html
```

Создать temp installation `8.3.27.2074/{shcntx_ru.hbk,1cv8_ru.hbk}` и дописать
в temp `v8project.yaml` вычисленный абсолютный путь, а не строковый
placeholder:

```python
with (workspace / "v8project.yaml").open("a", encoding="utf-8") as config:
    config.write(
        "tools:\n"
        "  platform:\n"
        "    version: '8.3.27.2074'\n"
        f"    path: '{installation.as_posix()}'\n"
    )
```

Это штатный production route, но полностью локальные bytes.

- [ ] **Step 6: Pass environment explicitly to every MCP batch.**

Расширить `call_mcp_messages(..., extra_env: dict[str, str] | None = None)`. Один и тот же env используется для `tools/list` и всех batch по 32 сообщения. Stand-ins живут до завершения последнего batch; cleanup выполняется в `finally`.

- [ ] **Step 7: Run parity green and prove no writes/network dependence.**

Run:

```bash
python3.12 -m unittest tests.ci.test_unica_mcp_script_parity.UnicaMcpParityTests.test_every_skill_tools_call_example_executes_by_tool_mode
python3.12 -m unittest tests.ci.test_unica_mcp_script_parity
```

Expected: PASS; workspace snapshot до/после идентичен, все 205 documented calls исполнены, readers не содержат `dryRun`, mutation calls не применили изменения.

- [ ] **Step 8: Commit.**

```bash
git add tests/ci/test_unica_mcp_script_parity.py tests/fixtures/unica_mcp_script_parity/reader-standins
git commit -m "test(skills): execute reader examples against local fixtures"
```

---

### Task 7: Принять ADR-0042 и синхронизировать нормативную архитектуру

**Files:**

- Modify: `spec/decisions/0042-readers-ne-prinimayut-preview.md`
- Modify: `spec/decisions/README.md`
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/architecture/change-checklist.md`
- Modify: `scripts/ci/generate-tool-surface.py`
- Modify: `spec/architecture/tool-surface.md` (generated)
- Verify: `spec/architecture/tool-surface-review.json`
- Modify: `tests/ci/test_architecture_registry.py`
- Modify: `tests/ci/test_tool_surface_ledger.py`
- Modify: `tests/ci/test_unica_skills.py`

- [ ] **Step 1: Add the failing architecture assertions.**

Проверить, что:

- ADR-0042 имеет `accepted` и находится в accepted index;
- существует уникальный `INV-MCP-PREVIEW-MUTATION-ONLY`;
- `INV-MCP-TYPED-RESULT` называет `typed_result_missing` и ADR-0042;
- `INV-SKILL-EXECUTABLE-EXAMPLES` различает preview mutation и real deterministic reader;
- change checklist ссылается на новый invariant, не копируя его Rule.

Run:

```bash
python3.12 -m unittest tests.ci.test_architecture_registry tests.ci.test_tool_surface_ledger tests.ci.test_unica_skills
```

Expected: FAIL до нормативных правок.

- [ ] **Step 2: Accept ADR-0042 in the implementation changeset.**

Proposal PR #426 к этому моменту закрыт без merge, поэтому ADR-0042 ещё не
попадала в целевую ветку и остаётся редактируемой по правилам жизненного цикла.
Перенести утверждённый файл в implementation branch сразу со статусом
`accepted` и добавить строку в Accepted в `spec/decisions/README.md`; Proposed
entry в implementation diff не создаётся. Содержание раздела `Решение` не
переписывать, кроме исправления доказанного противоречия. Перед изменением
обязательно проверить отсутствие пути в `origin/main`; если он найден,
остановиться, а не переписывать историю.

- [ ] **Step 3: Add and extend invariant owners.**

Добавить:

```markdown
### INV-MCP-PREVIEW-MUTATION-ONLY — Предпросмотр принадлежит мутации

- **Rule:** `ToolExecution::Read` не публикует и не принимает `dryRun` и
  исполняется только как `InvocationMode::Read`; `ToolExecution::Mutation`
  выводит `Preview` при отсутствующем или истинном `dryRun` и `Apply` только
  при `dryRun: false`.
- **Decision:** ADR-0042
- **Check:** `cargo-test` — `crates/unica-coder/src/application/mod.rs`
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_script_parity.py`
- **Scope:** source, runtime, packaged
```

В `INV-MCP-TYPED-RESULT` добавить постусловие successful `Read + Typed` и stable `typed_result_missing`, Decision ADR-0042 и Rust check `tool_specs_match_reviewed_result_contracts`.

В `INV-SKILL-EXECUTABLE-EXAMPLES` заменить «каждый как сухой прогон» на «mutation — preview; reader — настоящий MCP read над deterministic fixture/local stand-in», Decision ADR-0005, ADR-0042.

- [ ] **Step 4: Regenerate the ledger from the built registry.**

В generator убрать жёсткий текст «общих `cwd`/`dryRun`/`confirm`»: выводить фактически опубликованные общие аргументы либо нейтральное «предметных аргументов нет». Затем:

```bash
cargo build --quiet --package unica-coder --bin unica
python3.12 scripts/ci/generate-tool-surface.py --binary target/debug/unica
python3.12 scripts/ci/generate-tool-surface.py --check --binary target/debug/unica
```

`tool-surface-review.json` не менять, если contract/scope/scenarios не изменились; его неизменность допустима, потому что Rust test теперь сверяет classifications, а generated ledger отражает удаление `dryRun` из 38 schemas.

- [ ] **Step 5: Run architecture green tests and guard.**

```bash
python3.12 -m unittest \
  tests.ci.test_design_documents \
  tests.ci.test_architecture_registry \
  tests.ci.test_tool_surface_ledger \
  tests.ci.test_unica_skills
python3.12 scripts/ci/check-architecture-sync.py --base origin/main --strict
```

Expected: PASS; runtime, ledger, ADR и invariant registry описывают одну границу.

- [ ] **Step 6: Commit.**

```bash
git add spec scripts/ci/generate-tool-surface.py tests/ci/test_architecture_registry.py tests/ci/test_tool_surface_ledger.py tests/ci/test_unica_skills.py
git commit -m "docs(architecture): accept reader invocation boundary"
```

---

### Task 8: Полная верификация, review и отдельный implementation PR

**Files:**

- Verify: all changed files
- Verify: `plugins/unica/.mcp.json`
- Verify: `plugins/unica/.codex-plugin/plugin.json`
- Verify: `plugins/unica/.claude-plugin/plugin.json`
- Verify: `plugins/unica/third-party/tools.lock.json`

- [ ] **Step 1: Audit scope and stale assumptions.**

Run:

```bash
rg -n "present on every tool|every.*dry.run|previewing a read|if !dry_run|dryRun" \
  crates/unica-coder/src tests/ci spec/architecture plugins/unica/skills
rg -n "\.mutating|mutating:" crates tests scripts spec \
  --glob '!docs/design/**' --glob '!docs/plans/**' \
  --glob '!target/**' --glob '!.build/**' --glob '!dist/**' --glob '!docs-local/**'
git diff --check
git diff --stat origin/main...
```

Expected: `dryRun` остаётся только в mutation contracts/tests/docs; reader-specific preview prose отсутствует; старого поля `ToolSpec.mutating` нет.

- [ ] **Step 2: Run Rust verification.**

```bash
cargo fmt --all -- --check
cargo clippy -p unica-coder --all-targets -- -D warnings
cargo test -p unica-coder -- --test-threads=1
```

Expected: полный crate green; baseline ignored tests остаются ignored, новых ignored нет.

- [ ] **Step 3: Run CI contract verification.**

```bash
python3.12 -m unittest \
  tests.ci.test_unica_mcp_smoke \
  tests.ci.test_unica_mcp_script_parity \
  tests.ci.test_tool_surface_ledger \
  tests.ci.test_meta_surface_contract \
  tests.ci.test_unica_skills \
  tests.ci.test_architecture_registry \
  tests.ci.test_architecture_sync_guard \
  tests.ci.test_package_unica_plugin
python3.12 -m unittest discover -s tests/ci -p 'test_*.py'
python3.12 scripts/ci/check-architecture-sync.py --base origin/main --strict
```

Expected: все CI unit/contract tests green; parity не использует live network.

- [ ] **Step 4: Inspect package-contract files for unintended drift.**

Проверить, что manifests и `tools.lock.json` не изменились: stand-ins существуют только в test fixtures/temp plugin root. Если diff затронул package-contract metadata, остановиться и объяснить причину — ADR-0042 не требует version/package change.

- [ ] **Step 5: Request code review.**

Use `superpowers:requesting-code-review`. Проверить отдельно:

- все 38 readers не принимают `dryRun`;
- все 35 mutations сохранили три прежних режима default/true/false;
- `Read + Typed + ok` без data невозможно;
- reader fixture parity не маскирует failure allowlist-ом;
- implementation PR независим от #426 и основан на `main`.

Исправления review выполнять в этой же head-ветке с собственным red test, если выявлен дефект, привнесённый PR.

- [ ] **Step 6: Create the final commit if review caused changes.**

```bash
git status --short
git add \
  crates/unica-coder/src \
  crates/unica-coder/tests \
  tests/ci \
  tests/fixtures/unica_mcp_script_parity/reader-standins \
  spec \
  docs/design/2026-08-10-reader-invocation-mode-design.md \
  docs/plans/2026-08-10-reader-invocation-mode.md \
  scripts/ci/generate-tool-surface.py
git commit -m "fix(mcp): enforce reader invocation contracts"
```

Не создавать пустой commit; если дерево clean, перейти к push.

- [ ] **Step 7: Push and open one ready implementation PR.**

```bash
git push -u origin codex/issue-297-reader-invocation
gh pr create \
  --base main \
  --head codex/issue-297-reader-invocation \
  --title "fix(mcp): readers do not accept preview mode" \
  --body-file .superpowers/issue-297-pr-body.md
```

Перед командой создать игнорируемый `.superpowers/issue-297-pr-body.md` через
`apply_patch`; не добавлять его в git. PR body должен ссылаться на #297,
ADR-0042 и proposal PR #426, явно разделять root
cause/schema/dispatch/result/parity, перечислять red tests и финальные команды.
Issue #297 закрывать только этим implementation PR (`Closes #297`), не
proposal PR.

- [ ] **Step 8: Verify remote state.**

```bash
gh pr view --json number,url,isDraft,baseRefName,headRefName,mergeStateStatus
gh pr checks --watch
git status --short --branch
```

Expected: ready PR, base `main`, head `codex/issue-297-reader-invocation`, clean tracking worktree, checks green.

## Plan self-review checklist

- [ ] Каждая цель ADR-0042 имеет task: независимая топология (0), schema/category (1), validation/mode (2), reader dispatch (3), typed postcondition (4), transport (5), skill parity (6), normative sync (7), full proof (8).
- [ ] Каждый production defect получает тест, который запускается red до исправления.
- [ ] В плане нет незаполненных шагов, сетевой зависимости или машинно-зависимой установки.
- [ ] Все новые типы согласованы: `ToolExecution` классифицирует tool, `InvocationMode` — конкретный call, `ResultContract` — result postcondition.
- [ ] Proposal и implementation PR не образуют запрещённый стек: #426 закрывается без merge, а реализация начинается от обновлённого `main` и несёт утверждённые документы в одном changeset с кодом.
