# PR #386: дерево подсистем и ролевые членства объекта — план реализации

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Работать в head-ветке существующего PR #386; новый или дочерний PR не создавать.

**Goal:** `unica.subsystem.info` возвращает структурное дерево с контекстом предков выбранной подсистемы, а `unica.meta.info` возвращает две плоские группы подсистем, в состав которых входит анализируемый объект.

**Architecture:** отдельный `SubsystemAddress` и один secure registered-topology builder сохраняются. `subsystem.info` строит из графа только полное или сфокусированное дерево; `meta.info` строит из того же графа функциональные и интерфейсные членства текущего объекта и передаёт их валидатору без второго обхода.

**Tech Stack:** Rust 2021, `serde`, `roxmltree`, существующий secure-tree reader, Cargo tests, Python 3.12 CI-contract tests, GitHub CLI.

## Global Constraints

- Публикация идёт только в существующую head-ветку PR #386 `feat/meta-register-subsystem-command-interface`; force-push и новый PR запрещены.
- `SubsystemAddress` остаётся отдельным от `MetadataAddress` и сериализуется как `СтандартныеПодсистемы.Обсуждения`.
- Источник истины топологии — регистрации `Configuration/ChildObjects` и рекурсивные `Subsystem/ChildObjects`, не список XML-файлов.
- `IncludeInCommandInterface` обязателен и каноничен; эффективная роль учитывает весь путь предков.
- `subsystem.info` не возвращает `functionalSubsystems` и `interfaceSubsystems`.
- Каталог `Subsystems/` возвращает полное дерево выбранной области.
- Конкретная зарегистрированная подсистема возвращает сфокусированное дерево: единственная цепочка предков от корня и полный набор потомков выбранного узла; соседние ветки предков исключаются.
- Прежние подробные поля конкретной подсистемы сохраняются. Для отдельного незарегистрированного XML сфокусированное дерево отсутствует, но локальное описание продолжает работать.
- `meta.info` возвращает `functionalSubsystems` и `interfaceSubsystems` только для подсистем, чей `Content` содержит текущий объект; это не списки всей конфигурации.
- При полном доказательстве обе коллекции присутствуют, в том числе пустыми. При недоступной топологии они не подменяются пустыми, а чтение получает `provider_unavailable`.
- Каждый дефект проходит RED → GREEN до изменения следующего поведения.

---

### Task 1: Зафиксировать исправленную архитектурную границу

**Files:**
- Modify: `docs/design/2026-08-09-subsystem-address-and-effective-role-design.md`
- Modify: `spec/decisions/0036-subsystem-address-and-effective-role.md`
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/architecture/building-blocks.md`
- Modify: `docs/plans/2026-08-09-pr-386-subsystem-address-and-topology.md`

**Interfaces:**
- Produces: ADR-0036 and `INV-SOURCE-SUBSYSTEM-TOPOLOGY` with separate public projections.

- [ ] **Step 1: Record the corrected projections**

Write explicitly that `subsystem.info` owns structure and `meta.info` owns current-object memberships. Define the focused tree and fail-closed behavior without adding a third scanner.

- [ ] **Step 2: Run documentation structure guards**

Run:

```bash
python3.12 -m unittest tests/ci/test_design_documents.py tests/ci/test_architecture_registry.py
```

Expected: PASS.

- [ ] **Step 3: Commit the corrected decision**

```bash
git add docs/design/2026-08-09-subsystem-address-and-effective-role-design.md docs/plans/2026-08-09-pr-386-subsystem-address-and-topology.md spec/decisions/0036-subsystem-address-and-effective-role.md spec/architecture/invariants.md spec/architecture/building-blocks.md
git commit -m "docs(subsystem): разделить дерево и членства объекта"
```

---

### Task 2: Оставить в `subsystem.info` только структурное дерево

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/native_operations/subsystem.rs`

**Interfaces:**
- Consumes: `SubsystemTopologyNode` from `infrastructure/subsystem_topology.rs`.
- Produces: `SubsystemInfoAnswer::Tree { tree }` for directories and `SubsystemInfoResult.tree: Option<Vec<SubsystemTreeNode>>` for a concrete subsystem.

- [ ] **Step 1: Change the directory serialization tests to the required shape**

Update `pointing_at_the_subsystems_folder_answers_with_tree_and_effective_role_lists` to `pointing_at_the_subsystems_folder_answers_only_with_tree`. Its exact JSON must contain only:

```json
{
  "tree": [
    {"name":"СтандартныеПодсистемы","content":0,"children":[{"name":"Обсуждения","content":1,"children":[]}]},
    {"name":"Служебные","content":0,"children":[{"name":"ФоновыеЗадания","content":0,"children":[]}]}
  ]
}
```

Update the nested-directory test analogously: only its subtree under `tree`, no role lists.

- [ ] **Step 2: Add the focused-tree regression test**

Create a registered chain `Корень → Родитель → Выбранная → Потомок` plus sibling `Соседняя`. Point `SubsystemPath` at `Выбранная.xml` and assert `data.tree` equals a one-root tree whose ancestor nodes contain only the path to `Выбранная`, while `Выбранная.children` contains the complete `Потомок` subtree. Assert serialized data has no `functionalSubsystems` or `interfaceSubsystems` anywhere.

- [ ] **Step 3: Run the new tests and observe RED**

Run:

```bash
cargo test -p unica-coder subsystem_info_typed_result_tests -- --nocapture
```

Expected: FAIL because the directory still returns role lists and a file result has no `tree`.

- [ ] **Step 4: Implement the minimal structural projection**

Change the public types to:

```rust
pub(crate) struct SubsystemInfoResult {
    // existing fields stay unchanged
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tree: Option<Vec<SubsystemTreeNode>>,
}

pub(crate) enum SubsystemInfoAnswer {
    Subsystem(Box<SubsystemInfoResult>),
    Tree { tree: Vec<SubsystemTreeNode> },
}
```

Add `focused_subsystem_tree(roots, address_names)`: every ancestor is converted with one child — the next path node; the selected node uses the existing recursive `subsystem_tree_node` and therefore retains all descendants. Resolve the selected address from the XML parent `Subsystems` scope plus the descriptor name. If no configuration root or registration can be proved, return `None` for this additive field without breaking the existing standalone XML answer.

- [ ] **Step 5: Run the focused suite to GREEN**

```bash
cargo test -p unica-coder subsystem_info_typed_result_tests
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/unica-coder/src/infrastructure/native_operations/subsystem.rs
git commit -m "fix(subsystem): вернуть дереву контекст выбранной подсистемы"
```

---

### Task 3: Публиковать ролевые членства текущего объекта в `meta.info`

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/subsystem_topology.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/domain/metadata/results.rs`
- Modify: `crates/unica-coder/src/application/metadata.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/info.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs`
- Test: `crates/unica-coder/src/application/meta_info_surface_tests.rs`

**Interfaces:**
- Produces: `MetadataSubsystemEvidence::Complete { functional_subsystems, interface_subsystems }`.
- Produces: top-level serialized `MetaInfoData.functionalSubsystems` and `MetaInfoData.interfaceSubsystems`.
- Consumes: exact `MetadataAddress::as_str()` as the `Content/xr:Item` object reference.

- [ ] **Step 1: Add surface tests for current-object memberships**

Add registered subsystems so `Catalog.Inspectable` belongs to:

- functional root `Служебные`;
- interface root `Продажи`;
- nested interface `Продажи.ОптовыеПродажи`;
- one unrelated subsystem whose `Content` contains another object.

Assert exact arrays:

```json
"functionalSubsystems": ["Служебные"],
"interfaceSubsystems": ["Продажи", "Продажи.ОптовыеПродажи"]
```

Assert the unrelated subsystem is absent. Add a complete empty-topology case that returns both arrays as `[]`.

- [ ] **Step 2: Add the unavailable-evidence serialization assertion**

Extend the existing missing registered descriptor test: result remains failed with `provider_unavailable`, and serialized `data` contains neither membership field. This prevents unknown evidence from becoming false empty arrays.

- [ ] **Step 3: Run the new surface tests and observe RED**

```bash
cargo test -p unica-coder meta_info_surface_tests -- --nocapture
```

Expected: FAIL because the two fields do not exist and evidence is currently gathered only for command-interface registers.

- [ ] **Step 4: Extend the shared topology projection**

Add `functional_memberships(&self, object_ref)` symmetric to `interface_memberships`. Both traverse registered nodes in pre-order and include only nodes whose `content` contains the exact object reference.

- [ ] **Step 5: Make read evidence complete for every configuration object**

Change `typed_subsystem_evidence` so `meta.info` collects topology membership independently of metadata kind and `UseStandardCommands`. For source roots whose owner is not `Configuration.xml`, return a proved empty pair because they cannot participate in that configuration's subsystem registrations. Preserve the terminal checkpoint before `Complete`.

Construct:

```rust
MetadataSubsystemEvidence::Complete {
    functional_subsystems: topology.functional_memberships(target.as_str()),
    interface_subsystems: topology.interface_memberships(target.as_str()),
}
```

- [ ] **Step 6: Publish the evidence in typed data without duplicating it**

Add optional fields to `MetaInfoData`; `Some(vec![])` serializes as an empty array, `None` is omitted. Before consuming `read.local`, derive the pair from `read.validation_subject.subsystem_evidence`. Pass it into `MetaLocalInfo::into_info`. Update fake coordinator fixtures explicitly.

- [ ] **Step 7: Keep validation on the same evidence**

Update `meta_validate_check_register_command_interface` to destructure `interface_subsystems` with `..`; it still warns only for eligible registers and does not scan files itself. Update direct validator fixtures with both vectors.

- [ ] **Step 8: Run focused suites to GREEN**

```bash
cargo test -p unica-coder subsystem_topology::tests
cargo test -p unica-coder meta_info_surface_tests
cargo test -p unica-coder validation::tests
cargo test -p unica-coder metadata::tests
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/unica-coder/src/infrastructure/subsystem_topology.rs crates/unica-coder/src/application/ports.rs crates/unica-coder/src/domain/metadata/results.rs crates/unica-coder/src/application/metadata.rs crates/unica-coder/src/infrastructure/native_operations/meta/info.rs crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs crates/unica-coder/src/application/meta_info_surface_tests.rs
git commit -m "feat(meta): вернуть ролевые членства объекта"
```

---

### Task 4: Синхронизировать публичный контракт и skill

**Files:**
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Modify: `plugins/unica/skills/subsystem-info/SKILL.md`
- Modify: `plugins/unica/references/platform/metadata-conventions.md`
- Modify: `spec/architecture/tool-surface-review.json`
- Generate: `spec/architecture/tool-surface.md`
- Modify: `tests/ci/test_subsystem_surface_contract.py`
- Modify: `tests/ci/test_meta_surface_contract.py`
- Modify: `tests/ci/test_reference_metadata_conventions.py`

**Interfaces:**
- `subsystem.info`: complete/focused structural tree only.
- `meta.info`: current-object `functionalSubsystems` and `interfaceSubsystems`.

- [ ] **Step 1: Make CI contracts fail on the old allocation**

Change `test_subsystem_surface_contract.py` to reject role-list markers in the `subsystem.info` result contract and require focused-tree wording. Change `test_meta_surface_contract.py` to require both membership fields in `MetaInfoData`, complete typed evidence with two vectors, and one shared builder.

- [ ] **Step 2: Run the contract tests and observe RED**

```bash
python3.12 -m unittest tests/ci/test_subsystem_surface_contract.py tests/ci/test_meta_surface_contract.py tests/ci/test_reference_metadata_conventions.py
```

Expected: FAIL on stale skill, ledger and result descriptions.

- [ ] **Step 3: Update descriptions and examples**

Remove the role lists from every `subsystem-info` directory example. Add a concrete file example with `tree` showing ancestor chain and selected descendants. Document the two membership arrays under `meta.info` conventions and make clear they contain only the current object memberships.

- [ ] **Step 4: Regenerate the tool-surface ledger**

Run the repository generator named in `spec/architecture/tool-surface.md`, then verify the diff contains the corrected result contracts only.

- [ ] **Step 5: Run contracts to GREEN and commit**

```bash
python3.12 -m unittest tests/ci/test_subsystem_surface_contract.py tests/ci/test_meta_surface_contract.py tests/ci/test_reference_metadata_conventions.py tests/ci/test_architecture_registry.py
git add crates/unica-coder/src/application/mod.rs crates/unica-coder/src/application/tool_contracts.rs plugins/unica/skills/subsystem-info/SKILL.md plugins/unica/references/platform/metadata-conventions.md spec/architecture/tool-surface-review.json spec/architecture/tool-surface.md tests/ci/test_subsystem_surface_contract.py tests/ci/test_meta_surface_contract.py tests/ci/test_reference_metadata_conventions.py
git commit -m "docs(meta): развести дерево и членства подсистем"
```

---

### Task 5: Полная проверка и обновление существующего PR

**Files:**
- Verify: entire PR diff against `origin/main`.

- [ ] **Step 1: Run formatting, lint and full tests**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test -p unica-coder --quiet
python3.12 -m unittest discover -s tests/ci -p 'test_*.py'
git diff --check origin/main...HEAD
```

Expected: PASS; Python uses `tests/ci/requirements.txt` in an isolated environment.

- [ ] **Step 2: Refresh remote state without rewriting history**

Fetch `origin/main` and the fork head. If either changed, merge normally, inspect the resulting tree, and rerun the affected checks. Do not force-push.

- [ ] **Step 3: Push and verify exact head**

Push the existing branch and confirm `gh pr view 386 --json headRefOid` equals local `HEAD`.

- [ ] **Step 4: Correct the PR title/body and post the completion comment**

The body must explicitly state:

- `subsystem.info` owns the full/focused tree and no role lists;
- `meta.info` owns the two current-object membership lists;
- all previously posted contrary descriptions are superseded;
- exact local and GitHub Actions results.

- [ ] **Step 5: Verify no unresolved review threads and all required checks**

Use the bundled review-thread reader, resolve only threads whose fixes are present in the pushed head, and wait for required checks to finish.

## Completion Checklist

- [ ] Directory `subsystem.info` returns only `tree`.
- [ ] Concrete registered subsystem returns ancestor chain plus its full descendant tree.
- [ ] `meta.info` returns both membership arrays for the current object only.
- [ ] Unknown topology never serializes as false empty membership arrays.
- [ ] Register validation consumes the same typed evidence.
- [ ] ADR, invariant, skill, ledger and PR description match the code.
- [ ] Full local CI and GitHub Actions pass on the final head.
