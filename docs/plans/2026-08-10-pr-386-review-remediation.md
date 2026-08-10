# PR #386: устранение дефектов доказательства топологии — план реализации

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` to implement this plan task-by-task. Работать только в head-ветке существующего PR #386; новый PR не создавать.

**Goal:** сохранить требуемые публичные проекции — дерево в `unica.subsystem.info` и две плоские группы членств текущего объекта в `unica.meta.info` — и устранить все дефекты повторного ревью: нетипизированный `Content`, alias identity, влияние незарегистрированных файлов, раскрытие symlink, устаревший format guard и отсутствие cancellation/deadline.

**Architecture:** единый registration-driven builder читает `Configuration.xml`, затем только транзитивно зарегистрированные subsystem descriptors через удерживаемый no-follow root. Builder возвращает типизированную топологию и точный набор format dependencies. `subsystem.info` выводит выбранный адрес из физического зарегистрированного пути и строит structural projection; `meta.info` сопоставляет типизированные `ContentReference` с адресом и UUID текущего descriptor. Публичный native path передаёт cancellation и bounded deadline до checkpoint builder-а.

**Tech Stack:** Rust 2021, `serde`, `roxmltree`, `uuid`, существующие platform-specific secure-read primitives, Cargo tests, Python 3.12 CI-contract tests, GitHub CLI.

## Global Constraints

- Публикация идёт только в существующую head-ветку PR #386 `feat/meta-register-subsystem-command-interface`; force-push и дочерний PR запрещены.
- Исправляется причина дефекта. Нельзя фильтровать ошибку после небезопасного обхода, предварительно `canonicalize()` путь или подменять неизвестное пустым результатом.
- `SubsystemAddress` остаётся отдельным от `MetadataAddress` и сериализуется в диалекте БСП: `СтандартныеПодсистемы.Обсуждения`.
- `unica.subsystem.info` возвращает только structural `tree`: полное для каталога и сфокусированное для доказанно зарегистрированного XML. Ролевых списков в нём нет.
- `unica.meta.info` возвращает `functionalSubsystems` и `interfaceSubsystems` только для анализируемого объекта. При полном доказательстве поля присутствуют, включая `[]`; при недоступном доказательстве поля отсутствуют и complete-read получает `provider_unavailable`.
- Источник истины — `Configuration/ChildObjects` и рекурсивные `Subsystem/ChildObjects`. Физический XML только подтверждает ожидаемый зарегистрированный узел.
- Незарегистрированный файл, каталог или symlink не перечисляется и не влияет на byte/file/entry budgets или verdict. Зарегистрированный symlink и symlink в компонентах выбранного source root приводят к fail-closed.
- Все файлы доказательства читаются под одним удерживаемым no-follow root; перед `Complete` повторно подтверждаются identity открытых каталогов и файлов. Нельзя реализовать это серией независимых повторных открытий root.
- `Content/xr:Item` принимает только корректный `MetadataAddress` либо UUID. Сопоставление членства выполняется по доказанным адресу и UUID текущего metadata descriptor.
- Публичный `call_tool` без снятого аргумента `Mode` обязан охватывать format guard-ом `Configuration.xml` и все фактически читаемые зарегистрированные descriptors.
- Builder наблюдает cancellation и bounded deadline на каждом шаге и перед terminal `Complete`.
- Каждый дефект проходит RED → GREEN: тест должен падать на исходном коде по ожидаемой причине до production-правки. RED и GREEN команды фиксируются в отчёте задачи.
- После обновления `origin/main` номера ADR-0033–ADR-0035 уже заняты; решение этой ветки в `main` не попадало, поэтому до слияния оно перенумеровано в ADR-0036 без создания истории замещения.

---

### Task 1: Уточнить нормативную границу доказательства

**Files:**
- Modify: `docs/design/2026-08-09-subsystem-address-and-effective-role-design.md`
- Modify: `spec/decisions/0036-subsystem-address-and-effective-role.md`
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/architecture/building-blocks.md`
- Modify: `docs/plans/2026-08-10-pr-386-review-remediation.md`

**Interfaces:**
- Produces: уточнённые ADR-0036 и `INV-SOURCE-SUBSYSTEM-TOPOLOGY`.

- [x] **Step 1: Закрепить типизированную идентичность**

Указать, что item в `Content` — закрытая сумма `MetadataAddress | UUID`, malformed item ломает доказательство, а `meta.info` сопоставляет оба идентификатора текущего descriptor.

- [x] **Step 2: Закрепить registration-driven retained proof**

Явно запретить перечисление всей физической раскладки и серию независимых root-open. В доказательство входят только `Configuration.xml` и транзитивно зарегистрированные descriptors под одним удерживаемым no-follow root; только они расходуют бюджеты и образуют format dependencies.

- [x] **Step 3: Закрепить поведение потребителей**

Для concrete `subsystem.info` адрес выводится из физического пути относительно доказанного source root, а не из XML `Name`. Standalone/unregistered XML сохраняет локальное описание без `tree`; ошибка доказательства зарегистрированной области не подавляется. Public native path наблюдает cancellation и bounded deadline. Format guard не зависит от снятого `Mode`.

- [x] **Step 4: Проверить документацию и закоммитить**

Run:

```bash
python3.12 -m unittest tests/ci/test_design_documents.py tests/ci/test_architecture_registry.py
```

Expected: PASS.

Commit: `docs(subsystem): уточнить границы доказательства топологии`.

---

### Task 2: Сделать builder типизированным и registration-driven

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/platform/secure_read.rs`
- Modify: platform-specific implementation blocks in the same file as required
- Modify: `crates/unica-coder/src/infrastructure/subsystem_topology.rs`

**Interfaces:**
- Produces: retained-root secure read session/capability for a dynamically discovered allowlist.
- Produces: `ContentReference::{MetadataAddress, Uuid}` and target identity accepting both values.
- Produces: `SubsystemTopology` plus exact registered dependency paths.

- [ ] **Step 1: RED — Content grammar and dual identity**

Add focused tests proving that valid metadata addresses and UUIDs parse, matching UUID membership is found, non-matching UUID is not found, and an arbitrary nonempty string such as `broken-reference` rejects the topology. Run `cargo test -p unica-coder subsystem_topology::tests -- --nocapture`; record the expected failures before production edits.

- [ ] **Step 2: GREEN — typed ContentReference**

Parse every `xr:MDObjectRef` as either the existing `MetadataAddress` grammar or `uuid::Uuid`; retain a typed enum in topology nodes. Membership APIs accept a target identity containing both the current `MetadataAddress` and descriptor UUID and compare by variant, never by unrelated raw strings.

- [ ] **Step 3: RED — unrelated physical layout is inert**

Add tests where a valid registered topology coexists with (a) an unregistered XML larger than the topology byte budget, (b) an unregistered file symlink, and (c) an unregistered directory/symlink branch. The expected complete topology must remain identical. Preserve tests that a registered descriptor symlink, missing descriptor and registered over-budget descriptor fail closed.

- [ ] **Step 4: GREEN — retained registration closure**

Extend the generic secure-read boundary with a retained-root session/capability that:

- opens every root component no-follow and holds the root identity;
- reads explicitly requested relative regular files no-follow with per-file and cumulative limits;
- accumulates only opened registered paths in entry/file/byte budgets;
- retains/revalidates opened directory and file identities and performs a terminal checkpoint;
- works through the existing Unix/Windows platform boundary.

Use it in `capture_registered_subsystem_topology`: read `Configuration.xml`, parse registrations, then recursively request only expected descriptor paths. Do not enumerate unrelated directory entries and do not reopen root independently per descriptor. Return exact logical dependency paths in registration order for consumers/format guard.

- [ ] **Step 5: Regression and platform verification**

Run:

```bash
cargo test -p unica-coder subsystem_topology::tests
cargo test -p unica-coder secure_read::tests
cargo test -p unica-coder platform::secure_read
```

Expected: PASS (a filter matching zero tests must be corrected, not counted as evidence).

Commit: `fix(subsystem): читать только зарегистрированную топологию`.

---

### Task 3: Исправить public `subsystem.info`, format guard и cancellation

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/native_operations/subsystem.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/typed_result.rs`
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/format_guard.rs`
- Modify: nearby application/native tests that exercise public `call_tool`

**Interfaces:**
- Consumes: exact registered dependencies and topology from Task 2.
- Produces: lexical, identity-safe focused-tree resolution.
- Produces: cancellation/deadline-aware `subsystem-info` invocation.

- [ ] **Step 1: RED — alias and symlinked scope**

Add a concrete-file test where registered `Sales.xml` coexists with unregistered `Copy.xml` declaring `<Name>Sales</Name>`; `Copy.xml` must keep local data and omit `tree`. Add root and nested `Subsystems` symlink tests; topology must not be returned/followed. Run the focused suite and record RED.

- [ ] **Step 2: GREEN — physical identity without canonicalize**

Remove pre-proof `canonicalize()`. Derive source scope and candidate `SubsystemAddress` lexically from the exact relative shape `Subsystems/<Parent>/Subsystems/<Leaf>.xml`; require file stem, descriptor `Name` and registered address to agree. Return `Ok(None)` only for a real standalone/unregistered XML. Change focused-tree helper to `Result<Option<_>, String>` and propagate proof failures for a registered source area.

- [ ] **Step 3: RED — format guard without `Mode`**

Through the public `call_tool` path, test both a `Subsystems/` directory and a concrete registered XML with no `Mode`. Put `version="2.21"` on a registered descendant while the selected descriptor remains `2.20`; assert the normal format warning/guard result includes the descendant dependency. Add a resolver-error case so errors cannot silently become an empty dependency set.

- [ ] **Step 4: GREEN — path-semantic dependency resolution**

Remove every `Mode` branch from subsystem dependency resolution. Resolve the same source root and registered closure used by the handler and return `Configuration.xml` plus all registered descriptors actually required by the full/focused topology. Do not suppress dependency-resolution errors in format guard.

- [ ] **Step 5: RED/GREEN — mid-read cancellation and bounded deadline**

Add a deterministic test hook that cancels after secure traversal starts, plus an exhausted-deadline test. Extend the native typed invocation boundary to receive `CancellationToken` and a bounded `ProviderDeadline` (or an equivalent existing deadline type), pass them only where needed without fabricating default uncancellable tokens, and make both full and focused captures checkpoint cancellation/deadline including the terminal checkpoint.

- [ ] **Step 6: Verify and commit**

Run:

```bash
cargo test -p unica-coder subsystem_info_typed_result_tests
cargo test -p unica-coder format_guard
cargo test -p unica-coder application_ports
```

Expected: PASS.

Commit: `fix(subsystem): защитить публичное чтение топологии`.

---

### Task 4: Провести UUID identity через `meta.info` и синхронизировать контракт

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/info.rs`
- Modify: `crates/unica-coder/src/application/meta_info_surface_tests.rs`
- Modify: `crates/unica-coder/src/application/ports.rs` and typed evidence fixtures if required
- Modify: `plugins/unica/skills/subsystem-info/SKILL.md`
- Modify: `plugins/unica/skills/meta-info/SKILL.md`
- Modify: `plugins/unica/references/platform/metadata-conventions.md`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Modify: `crates/unica-coder/src/application/operation_descriptors.rs` if the generated/public description owns the wording
- Modify: `spec/architecture/tool-surface-review.json`
- Generate: `spec/architecture/tool-surface.md`
- Modify: `tests/ci/test_subsystem_surface_contract.py`
- Modify: `tests/ci/test_meta_surface_contract.py`
- Modify: `tests/ci/test_reference_metadata_conventions.py`

**Interfaces:**
- Consumes: typed target identity and topology from Task 2.
- Produces: correct UUID/address membership lists and synchronized public documentation.

- [ ] **Step 1: RED — UUID membership at public surface**

Add `meta_info_surface_tests` for a subsystem `Content` that references the current metadata object by its root descriptor UUID. Assert the correct functional/interface list. Add malformed Content and non-matching UUID cases; malformed registered evidence must be `provider_unavailable`, while non-matching valid UUID proves empty arrays.

- [ ] **Step 2: GREEN — descriptor identity propagation**

Parse/retain the UUID of the analyzed metadata object’s root descriptor and pass `{address, uuid}` to both topology membership projections. Do not rescan or reparse subsystem files in the consumer or validator. Ensure missing/invalid target UUID follows the existing metadata descriptor error contract rather than silently degrading to address-only matching.

- [ ] **Step 3: RED/GREEN — contract and skills**

Strengthen Python contract tests first. Remove stale `overview/full` or `Mode` wording. Document:

- `subsystem.info`: directory full tree, registered XML focused tree, standalone/unregistered XML local-only; proof failures and cancellation are not silently omitted;
- `meta.info`: `functionalSubsystems` and `interfaceSubsystems` contain only memberships of the current object, support address and UUID platform references, serialize `[]` only after complete proof, and are absent on `provider_unavailable`.

Regenerate the tool-surface ledger using the repository’s existing generator/check command; do not hand-edit generated output.

- [ ] **Step 4: Focused verification and commit**

Run:

```bash
cargo test -p unica-coder subsystem_topology::tests
cargo test -p unica-coder subsystem_info_typed_result_tests
cargo test -p unica-coder meta_info_surface_tests
python3.12 -m unittest tests/ci/test_subsystem_surface_contract.py tests/ci/test_meta_surface_contract.py tests/ci/test_reference_metadata_conventions.py
```

Expected: PASS.

Commit: `fix(meta): сопоставлять подсистемы по адресу и uuid`.

---

### Task 5: Полная проверка PR перед публикацией

**Files:**
- Modify only if a fresh verification exposes a regression; any fix repeats RED → GREEN and receives scoped review.

- [ ] **Step 1: Formatting, unit/integration and CI contracts**

Run fresh from final HEAD:

```bash
cargo fmt --check
cargo clippy -p unica-coder --all-targets -- -D warnings
cargo test -p unica-coder
python3.12 -m unittest discover -s tests/ci -p 'test_*.py'
```

- [ ] **Step 2: Diff and architecture checks**

Inspect `git diff origin/main...HEAD`, confirm there are no public role lists in `subsystem.info`, no raw-string Content comparison, no `Mode` dependency and no `canonicalize()` before secure proof. Confirm the worktree has no unrelated changes.

- [ ] **Step 3: Final independent review**

Run the `superpowers:requesting-code-review` whole-branch review against the merge base. Any Critical/Important finding gets one fix wave and one scoped re-review before publication.

- [ ] **Step 4: Publish to the existing PR**

Push the existing branch without force. Verify the live PR head equals local HEAD. Reply in every original inline thread with the concrete fix and tests, resolve only those whose fix is present in the live diff, then post one concise summary comment.
