# Meta Validation Findings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Довести PR #316 и #317 до независимо проверяемого состояния: новые предупреждения `MetadataValidator` должны иметь стабильные коды, точное поле и язык, не меняя предупреждающую семантику.

**Architecture:** Существующий строковый путь `MetaValidationReporter` сохраняется для ещё не перенесённых правил. Для новых правил рядом добавляется структурная находка, которая один раз преобразуется в `MetaDiagnostic` на границе `MetadataValidator`, где известен `metadataPath`. Одинаковый минимальный механизм реализуется в обеих независимых head-ветках; после слияния #316 ветка #317 перебазируется и наследует общую часть из `main`.

**Tech Stack:** Rust 2021, Serde/serde_json, Cargo tests, Python 3.12 `unittest`, Git worktrees, GitHub CLI.

## Global Constraints

- База обоих PR — `main`; дочерний или prerequisite-PR не создаётся.
- PR #316 изменяется только через head `Oxotka/unica:feat/meta-redundant-list-presentation`.
- PR #317 изменяется только через head `Oxotka/unica:feat/meta-command-interface-soft-limit`.
- `maintainer_can_modify` у обоих PR должен оставаться `true` перед первым push.
- Превышение 38 символов остаётся предупреждением и называется верхним порогом, а не жёстким пределом.
- До 30 символов включительно предупреждения нет; 31–38 дают только `command_text_recommended_limit`; от 39 — только `command_text_upper_limit`.
- `redundant_list_presentation` относится к `properties.ListPresentation`; коды длины относятся к фактически выбранному `properties.Synonym` или `properties.ListPresentation`.
- Языковая находка возвращает непустое `language`; неязыковая диагностика не сериализует это поле.
- `message` не используется тестами как идентификатор правила.
- `ListPresentation` не добавляется в `setProperties`; ветка проверяется на реальном Platform XML.
- Открытые поддеревья `data`, `diagnostics` и `job` общей `outputSchema` не закрываются этим изменением.
- На каждый дефект сначала запускается падающий тест на неизменённой реализации, затем минимальная правка и зелёный повтор.
- Issue #283 этими PR не закрывается.

---

### Task 1: Зафиксировать согласованные проектные артефакты и изолировать #316

**Files:**
- Create: `docs/design/2026-08-10-meta-validation-findings-design.md`
- Create: `docs/plans/2026-08-10-meta-validation-findings.md`
- Create: `spec/decisions/0035-tipizirovannye-nahodki-validacii-meta.md`
- Modify: `spec/decisions/README.md`

**Interfaces:**
- Consumes: согласованный дизайн и `origin/main` at `1ab40f985af9a217ae01be038d043a3c2c045169` or newer.
- Produces: один локальный documentation commit, который обе PR-ветки получают через `cherry-pick`; отдельный worktree на `refs/codex/pr316`.

- [ ] **Step 1: Повторно проверить проектные документы**

Run:

```bash
git diff --check
python3.12 tests/ci/test_design_documents.py
python3.12 tests/ci/test_architecture_registry.py
```

Expected: все команды завершаются с кодом `0`.

- [ ] **Step 2: Создать локальную ветку проектных артефактов и commit**

Run:

```bash
git switch -c codex/meta-validation-findings-plan
git add docs/design/2026-08-10-meta-validation-findings-design.md \
  docs/plans/2026-08-10-meta-validation-findings.md \
  spec/decisions/0035-tipizirovannye-nahodki-validacii-meta.md \
  spec/decisions/README.md
git commit -m "docs(meta): спроектировать типизированные находки валидации"
```

Expected: commit содержит только четыре перечисленных пути. Сохранить его OID как `DESIGN_COMMIT`.

- [ ] **Step 3: Создать изолированный worktree PR #316**

Run:

```bash
git fetch origin main --quiet
git fetch origin refs/pull/316/head:refs/codex/pr316 --force --quiet
PR316_DIR=$(mktemp -d /tmp/unica-pr316.XXXXXX)
git worktree add -b codex/pr316-working "$PR316_DIR" refs/codex/pr316
git -C "$PR316_DIR" cherry-pick "$DESIGN_COMMIT"
```

Expected: `git -C "$PR316_DIR" status --short` пуст, а `git -C "$PR316_DIR" merge-base HEAD origin/main` совпадает с merge base живого PR #316.

- [ ] **Step 4: Проверить право обновления fork-ветки**

Run:

```bash
gh api repos/IngvarConsulting/unica/pulls/316 --jq '.maintainer_can_modify'
```

Expected: `true`. При `false` не создавать другой PR и не пушить; подготовить commit OID для владельца ветки.

---

### Task 2: Типизировать предупреждение #316 через красный публичный тест

**Files:**
- Modify: `crates/unica-coder/src/application/meta_info_surface_tests.rs`
- Modify: `crates/unica-coder/src/domain/metadata/diagnostics.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs`
- Modify struct literals: `crates/unica-coder/src/infrastructure/native_operations/meta/remove.rs`
- Modify struct literals: `crates/unica-coder/src/infrastructure/native_operations/meta/info.rs`
- Modify struct literals: `crates/unica-coder/src/infrastructure/native_operations/meta/publisher.rs`

**Interfaces:**
- Consumes: `MetaValidationReporter::warn(String)`, `MetaValidationRun`, `MetaDiagnostic` and PR #316 XML fixture helper.
- Produces: `MetaDiagnosticCode::RedundantListPresentation`, `MetaDiagnostic::warning`, `MetaDiagnostic::with_language`, `MetaValidationFinding`, `MetaValidationReporter::warn_finding`.

- [ ] **Step 1: Переписать тест #316 на машинный контракт**

Change `add_enum_with_presentations` to return cloned diagnostic objects rather than messages:

```rust
fn validation_diagnostics(data: &Value) -> Vec<Value> {
    data["validation"]["diagnostics"]
        .as_array()
        .expect("validation diagnostics")
        .clone()
}

fn diagnostic_by_code<'a>(diagnostics: &'a [Value], code: &str) -> Option<&'a Value> {
    diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == code)
}
```

For the equal Russian texts, assert literal boundary values:

```rust
let warning = diagnostic_by_code(&diagnostics, "redundant_list_presentation")
    .expect("typed redundancy warning");
assert_eq!(warning["severity"], "warning");
assert_eq!(warning["metadataPath"], "Enum.RedundantList");
assert_eq!(warning["field"], "properties.ListPresentation");
assert_eq!(warning["language"], "ru");
assert!(warning["message"].as_str().is_some_and(|message| !message.is_empty()));
```

For distinct text and fallback, assert that `diagnostic_by_code(...).is_none()`.

- [ ] **Step 2: Запустить тест и доказать RED**

Run:

```bash
cargo test -p unica-coder info_warns_when_list_presentation_duplicates_the_synonym -- --nocapture --test-threads=1
```

Expected: FAIL because the existing warning has `code = validation_failed` and therefore `typed redundancy warning` is absent. A fixture/XML failure is not an acceptable RED.

- [ ] **Step 3: Расширить доменную диагностику минимальными полями**

Add the enum variant and optional location member:

```rust
pub(crate) enum MetaDiagnosticCode {
    // existing variants
    RedundantListPresentation,
}

pub(crate) struct MetaDiagnostic {
    // existing fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) language: Option<String>,
}
```

Initialize `language: None` in `error`, add a warning constructor and builder:

```rust
pub(crate) fn warning(code: MetaDiagnosticCode, message: impl Into<String>) -> Self {
    let mut diagnostic = Self::error(code, message);
    diagnostic.severity = MetaDiagnosticSeverity::Warning;
    diagnostic
}

pub(crate) fn with_language(mut self, language: impl Into<String>) -> Self {
    self.language = Some(language.into());
    self
}
```

Add `language: None` to every direct `MetaDiagnostic { ... }` literal reported by:

```bash
rg -n 'MetaDiagnostic \{' crates/unica-coder/src
```

Update the exhaustive serialization test with the literal
`"redundant_list_presentation"`, and add a serialization assertion proving a
non-language diagnostic omits `language` while `.with_language("ru")` emits it.

- [ ] **Step 4: Добавить переходный структурный канал reporter**

In `validation.rs`, define the transitional finding:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MetaValidationFinding {
    code: MetaDiagnosticCode,
    field: String,
    language: Option<String>,
    message: String,
}
```

Add `findings: Vec<MetaValidationFinding>` to `MetaValidationReporter` and
`MetaValidationRun`. Implement:

```rust
pub(super) fn warn_finding(
    &mut self,
    code: MetaDiagnosticCode,
    field: impl Into<String>,
    language: Option<&str>,
    message: impl Into<String>,
) {
    self.findings.push(MetaValidationFinding {
        code,
        field: field.into(),
        language: language.map(str::to_owned),
        message: message.into(),
    });
}
```

`finalize` must return legacy `errors`, legacy `warnings`, and structured
`findings` with this shape:

```rust
pub(super) fn finalize(
    self,
) -> (bool, Vec<String>, Vec<String>, Vec<MetaValidationFinding>) {
    (
        self.errors.is_empty(),
        self.errors,
        self.warnings,
        self.findings,
    )
}
```

Destructure all four values when constructing `MetaValidationRun`. In
`MetadataValidator::evaluate`, map every finding exactly once:

```rust
let diagnostic = MetaDiagnostic::warning(finding.code, finding.message)
    .with_metadata_path(subject.target.clone())
    .with_field(finding.field);
let diagnostic = match finding.language {
    Some(language) => diagnostic.with_language(language),
    None => diagnostic,
};
diagnostics.push(diagnostic);
```

Do not parse code, field, or language from `message`. Leave all existing
`report.warn(...)` calls on the legacy path.

- [ ] **Step 5: Перевести только правило #316**

Replace the new redundancy `report.warn(format!(...))` with:

```rust
report.warn_finding(
    MetaDiagnosticCode::RedundantListPresentation,
    "properties.ListPresentation",
    Some(language_code),
    format!(
        "3. Properties: ListPresentation '{list_text}' duplicates the Synonym \
         for the command interface, language '{language_code}'"
    ),
);
```

The equality, non-empty checks, and per-language selection remain unchanged.

- [ ] **Step 6: Запустить GREEN и mutation checks**

Run:

```bash
cargo test -p unica-coder info_warns_when_list_presentation_duplicates_the_synonym -- --nocapture --test-threads=1
cargo test -p unica-coder info_allows_list_presentation_that_differs_from_the_synonym -- --nocapture --test-threads=1
cargo test -p unica-coder info_allows_synonym_fallback_without_a_redundancy_warning -- --nocapture --test-threads=1
cargo test -p unica-coder diagnostic_codes_serialize_to_the_stable_exhaustive_vocabulary
```

Expected: PASS. Mentally replacing the code with `ValidationFailed`, the field
with `properties`, the language with `None`, or the equality branch with `!=`
must make at least one public surface test fail.

- [ ] **Step 7: Commit #316 implementation**

Run:

```bash
git add crates/unica-coder/src/application/meta_info_surface_tests.rs \
  crates/unica-coder/src/domain/metadata/diagnostics.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/remove.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/info.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/publisher.rs
git commit -m "fix(meta): типизировать предупреждение о лишнем представлении"
```

Expected: commit не содержит #317 thresholds или публичное свойство
`ListPresentation`.

---

### Task 3: Принять ADR #316, закрепить инвариант и опубликовать ветку

**Files:**
- Modify: `spec/decisions/0035-tipizirovannye-nahodki-validacii-meta.md`
- Modify: `spec/decisions/README.md`
- Modify: `spec/architecture/invariants.md`
- Modify: `plugins/unica/references/platform/metadata-conventions.md`

**Interfaces:**
- Consumes: working `redundant_list_presentation` structuredContent from Task 2.
- Produces: accepted ADR-0035 and `INV-MCP-META-FINDINGS` owned by ADR-0035.

- [ ] **Step 1: Сделать решение действующим внутри PR #316**

Change ADR status from `proposed` to `accepted`. Move its link from
`Предложенные решения` into the chronological accepted list in
`spec/decisions/README.md`; restore the no-proposals sentence in the proposed
section. Before changing the status, remove the `meta_add_surface_tests` item
from ADR-0035 verification: standalone #316 does not yet implement either
command-text code, so that line would make a false claim.

- [ ] **Step 2: Добавить проверяемый инвариант**

Insert after `INV-MCP-META-SURFACE` a registry record with identifier
`INV-MCP-META-FINDINGS` and title «Перенесённые находки метаданных имеют
устойчивую идентичность». Its `Rule` says that every language-specific warning
migrated from the string path returns a distinct stable rule code, exact
`properties.*` field and non-empty `language` in `structuredContent`, while
`message` is not a machine identifier. Set `Decision` to `ADR-0035`, `Check` to
`ci-test` over
`crates/unica-coder/src/application/meta_info_surface_tests.rs`, and `Scope` to
`source, runtime`. Wrap prose to the surrounding Markdown width.

- [ ] **Step 3: Уточнить справку #316**

Keep the platform convention human-readable, name `ListPresentation` and
`Synonym`, and state that equality creates a non-blocking warning. Do not make
the English message or the Rust enum spelling the human prose contract.

- [ ] **Step 4: Проверить полный #316 contour**

Run:

```bash
cargo fmt --all -- --check
cargo clippy -p unica-coder --all-targets --all-features -- -D warnings
cargo test -p unica-coder -- --test-threads=1
python3.12 tests/ci/test_design_documents.py
python3.12 tests/ci/test_architecture_registry.py
python3.12 tests/ci/test_reference_metadata_conventions.py
python3.12 tests/ci/test_meta_surface_contract.py
python3.12 tests/ci/test_tool_surface_ledger.py
python3.12 tests/ci/test_unica_mcp_smoke.py
git diff --check origin/main...HEAD
```

Expected: all PASS. If a named Python file is absent on the refreshed branch,
use `rg --files tests/ci | rg 'meta|surface|mcp_smoke|reference'` and run the
tracked replacement that owns the same contract; record the substitution in
the final report.

- [ ] **Step 5: Commit architecture and docs**

Run:

```bash
git add spec/decisions/0035-tipizirovannye-nahodki-validacii-meta.md \
  spec/decisions/README.md spec/architecture/invariants.md \
  plugins/unica/references/platform/metadata-conventions.md
git commit -m "docs(meta): закрепить идентичность находок валидации"
```

- [ ] **Step 6: Push только в существующую head-ветку #316**

Run:

```bash
git remote get-url oxotka >/dev/null 2>&1 || \
  git remote add oxotka https://github.com/Oxotka/unica.git
git push oxotka HEAD:feat/meta-redundant-list-presentation
gh pr view 316 --json headRefOid,mergeable,mergeStateStatus
gh pr checks 316
```

Expected: GitHub `headRefOid` equals local `HEAD`; no new PR is created. Wait
for required checks to finish and inspect failures before claiming readiness.

---

### Task 4: Изолировать #317 и доказать отсутствующий машинный контракт

**Files:**
- Modify: `crates/unica-coder/src/application/meta_add_surface_tests.rs`
- Reuse design files from Task 1 in the independent branch.

**Interfaces:**
- Consumes: live `refs/pull/317/head`, literal threshold mapping from the approved design.
- Produces: failing public tests for both threshold codes and the non-empty `ListPresentation` path.

- [ ] **Step 1: Создать отдельный worktree #317**

Run from the primary repository:

```bash
git fetch origin refs/pull/317/head:refs/codex/pr317 --force --quiet
PR317_DIR=$(mktemp -d /tmp/unica-pr317.XXXXXX)
git worktree add -b codex/pr317-working "$PR317_DIR" refs/codex/pr317
git -C "$PR317_DIR" cherry-pick "$DESIGN_COMMIT"
gh api repos/IngvarConsulting/unica/pulls/317 --jq '.maintainer_can_modify'
```

Expected: clean worktree and `true` maintenance permission.

- [ ] **Step 2: Заменить проверки текста на таблицу машинных результатов**

Make the helper return full diagnostics. Use hand-derived cases:

```rust
let cases = [
    (30, None),
    (31, Some("command_text_recommended_limit")),
    (38, Some("command_text_recommended_limit")),
    (39, Some("command_text_upper_limit")),
];
```

For every `Some(code)`, assert `severity = warning`, `field =
properties.Synonym`, `language = ru`, `validation.status = passed`, and absence
of the other threshold code. For `None`, assert both codes absent. Do not assert
exact `message`.

- [ ] **Step 3: Добавить реальный `ListPresentation` case**

Add a test-only helper that patches the generated descriptor's existing
`<ListPresentation/>` element into:

```xml
<ListPresentation>
    <v8:item>
        <v8:lang>ru</v8:lang>
        <v8:content>Текст длиной не менее тридцати девяти символов</v8:content>
    </v8:item>
</ListPresentation>
```

Call `unica.meta.info` or the existing mutation validation path and assert
`command_text_upper_limit`, `field = properties.ListPresentation`, `language =
ru`, and no `properties.Synonym` length finding. The test helper must stay in
the test module and must not add a production XML mutation API.

- [ ] **Step 4: Запустить tests и доказать RED**

Run:

```bash
cargo test -p unica-coder command_text -- --nocapture --test-threads=1
```

Expected: threshold tests fail because findings still have
`code = validation_failed`; the `ListPresentation` branch must reach validation
rather than fail during fixture preparation.

---

### Task 5: Типизировать оба порога #317 без блокировки

**Files:**
- Modify: `crates/unica-coder/src/domain/metadata/diagnostics.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs`
- Modify direct diagnostic literals in the same files listed by Task 2.
- Modify: `crates/unica-coder/src/application/meta_add_surface_tests.rs`

**Interfaces:**
- Consumes: the same `MetaValidationFinding` interface specified in Task 2, recreated independently in #317.
- Produces: `MetaDiagnosticCode::CommandTextRecommendedLimit` and `MetaDiagnosticCode::CommandTextUpperLimit`.

- [ ] **Step 1: Реализовать общий структурный механизм в #317**

Apply the exact `language`, `warning`, `with_language`, `MetaValidationFinding`,
`warn_finding`, `MetaValidationRun.findings`, and single boundary conversion
defined in Task 2. In this branch the exhaustive code vocabulary contains the
two command-text variants, not `RedundantListPresentation` unless #316 has
already merged and the branch was rebased onto it.

- [ ] **Step 2: Перевести threshold selector**

Use mutually exclusive branches:

```rust
let (code, message) = if length > 38 {
    (
        MetaDiagnosticCode::CommandTextUpperLimit,
        format!(
            "3. Properties: {source} '{text}' is longer than the upper \
             command-interface threshold of 38 characters ({length})"
        ),
    )
} else if length > 30 {
    (
        MetaDiagnosticCode::CommandTextRecommendedLimit,
        format!(
            "3. Properties: {source} '{text}' is longer than the recommended \
             command-interface length of 30 characters ({length})"
        ),
    )
} else {
    return;
};
report.warn_finding(
    code,
    format!("properties.{source}"),
    language,
    message,
);
```

Change `meta_validate_warn_long_command_text` to accept `language: Option<&str>`
so the reporter never depends on `Option<&String>`. Pass
`Some(language_code.as_str())` from `meta_validate_check_command_texts` and
preserve `chars().count()`.

- [ ] **Step 3: Сохранить apply semantics**

Keep the existing `dryRun: false` surface test and assert:

```rust
assert!(result.ok, "warning must not block apply: {:?}", result.errors);
assert_eq!(data["validation"]["status"], "passed");
```

The descriptor must exist after the call. No error branch may be introduced for
length 39 or greater.

- [ ] **Step 4: Запустить GREEN и mutation checks**

Run:

```bash
cargo test -p unica-coder command_text -- --nocapture --test-threads=1
cargo test -p unica-coder diagnostic_codes_serialize_to_the_stable_exhaustive_vocabulary
```

Expected: PASS. Mutating `> 38` to `>= 38`, `> 30` to `>= 30`, swapping the two
codes, choosing `Synonym` when `ListPresentation` is non-empty, or emitting both
codes must fail a literal boundary assertion.

- [ ] **Step 5: Commit #317 implementation**

Run:

```bash
git add crates/unica-coder/src/application/meta_add_surface_tests.rs \
  crates/unica-coder/src/domain/metadata/diagnostics.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/remove.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/info.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/publisher.rs
git commit -m "fix(meta): типизировать предупреждения о длине команды"
```

---

### Task 6: Принять архитектуру #317, проверить и обновить существующую ветку

**Files:**
- Modify: `spec/decisions/0035-tipizirovannye-nahodki-validacii-meta.md`
- Modify: `spec/decisions/README.md`
- Modify: `spec/architecture/invariants.md`
- Modify: `plugins/unica/references/platform/metadata-conventions.md`

**Interfaces:**
- Consumes: working threshold findings from Task 5.
- Produces: independently green PR #317 with the same general ADR policy and a branch-specific invariant check.

- [ ] **Step 1: Принять ADR и инвариант в самостоятельной ветке**

Before #316 merges, perform the same `proposed` → `accepted` and index movement
as Task 3. Add `INV-MCP-META-FINDINGS` with the same Rule and Decision, but use:

```markdown
- **Check:** `ci-test` — `crates/unica-coder/src/application/meta_add_surface_tests.rs`
```

For this standalone path, remove the `meta_info_surface_tests` verification item
from ADR-0035 before accepting it: the #317 branch does not implement
`redundant_list_presentation`.

After #316 merges, rebase first, keep the accepted ADR/invariant from `main`,
and add the `meta_add_surface_tests.rs` check to the existing invariant instead
of creating another decision.

- [ ] **Step 2: Исправить терминологию справки**

State the exclusive ranges 31–38 and 39+, call 38 the upper threshold, and say
both outcomes are warnings. Remove every new use of «жёсткий предел» / `hard
limit` introduced by #317.

- [ ] **Step 3: Выполнить полный #317 contour**

Run the same full Rust and Python commands from Task 3, substituting the #317
worktree. Also run:

```bash
git diff --check origin/main...HEAD
```

Expected: all PASS and no uncommitted files.

- [ ] **Step 4: Commit docs and push existing #317 head**

Run:

```bash
git add spec/decisions/0035-tipizirovannye-nahodki-validacii-meta.md \
  spec/decisions/README.md spec/architecture/invariants.md \
  plugins/unica/references/platform/metadata-conventions.md
git commit -m "docs(meta): уточнить предупреждающие пороги командного текста"
git push oxotka HEAD:feat/meta-command-interface-soft-limit
gh pr view 317 --json headRefOid,mergeable,mergeStateStatus
gh pr checks 317
```

Expected: remote OID equals local `HEAD`, no new PR exists, required checks
eventually pass.

---

### Task 7: Финальная GitHub-проверка и последовательность слияния

**Files:**
- No repository changes before #316 merges.
- After #316 merge, potentially modify only the #317 branch during rebase conflict resolution.

**Interfaces:**
- Consumes: both pushed heads and GitHub CI results.
- Produces: review-ready #316 first and a #317 branch with an explicit post-#316 rebase procedure.

- [ ] **Step 1: Проверить живые threads и checks**

Run thread-aware GraphQL for both PRs and:

```bash
gh pr checks 316
gh pr checks 317
gh pr view 316 --json mergeable,mergeStateStatus,reviewDecision,headRefOid
gh pr view 317 --json mergeable,mergeStateStatus,reviewDecision,headRefOid
```

Expected: no unresolved actionable thread, no failed required check. Do not
reply to or resolve GitHub threads without a separate explicit request.

- [ ] **Step 2: Handoff merge order**

Report #316 as first. Do not merge either PR unless explicitly requested.

- [ ] **Step 3: После фактического слияния #316 перебазировать #317**

Only after GitHub shows #316 merged:

```bash
git fetch origin main --quiet
git rebase origin/main
```

Resolve duplicated design, ADR, invariant, `language` field and reporter code by
keeping the already merged `main` implementation and retaining only #317 code
variants, threshold rule, tests, convention text, and the additional invariant
check. Re-run the complete Task 6 contour, then:

```bash
git push --force-with-lease oxotka HEAD:feat/meta-command-interface-soft-limit
```

Expected: #317 returns to `MERGEABLE`/`CLEAN` with green checks. Never use plain
`--force`.

- [ ] **Step 4: Удалить только чистые временные worktrees**

Verify both temporary worktrees have empty `git status --short`, then remove
their explicit `/tmp/unica-pr316.*` and `/tmp/unica-pr317.*` paths with
`git worktree remove <exact-path>`. Do not remove a dirty worktree.
