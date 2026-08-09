# PR #386: `SubsystemAddress` и доказанная топология подсистем — план реализации

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task. Работать в head-ветке существующего PR #386; новый или дочерний PR не создавать.

**Goal:** привести PR #386 к согласованному контракту: `SubsystemAddress` сериализуется в плоском диалекте БСП, `unica.subsystem.info` сохраняет `tree` и возвращает рядом два исчерпывающих списка функциональных и интерфейсных подсистем, а `unica.meta.info` проверяет командный интерфейс по той же доказанной зарегистрированной топологии.

**Architecture:** отдельный доменный `SubsystemAddress` не участвует в `SourceTarget` и не наследует грамматику `MetadataAddress`. Один инфраструктурный построитель под безопасным снимком читает регистрации из `Configuration/ChildObjects` и рекурсивных `Subsystem/ChildObjects`, строго проверяет дескрипторы и вычисляет эффективную роль по всей цепочке. От него строятся обе публичные проекции и доказательство для `meta`-валидации; неполное чтение даёт `unavailable`, а не доказанное отсутствие.

**Tech Stack:** Rust 2021, `serde`, `roxmltree`, существующий secure-tree reader, Cargo tests, Python 3.12 CI-contract tests, GitHub CLI.

## Global Constraints

- Исходная точка плана: PR #386, head `5084122cbc20c5c6cf8a56306a87e933be14c02e`, ветка fork `Oxotka/unica:feat/meta-register-subsystem-command-interface`, база `IngvarConsulting/unica:main` на `249b8a2290d6c70925c8bf58c90422047fc4ab86`. Перед началом и перед публикацией обновить ссылки: эти SHA — контрольный снимок, не вечная истина.
- Все исправления относятся к регрессиям самого PR #386, поэтому коммиты отправляются в его существующую head-ветку. Не открывать новый PR и не строить стек.
- `tree` не заменяется. В ответ каталога `Subsystems/` рядом появляются `functionalSubsystems` и `interfaceSubsystems`.
- `SubsystemAddress` имеет вид `СтандартныеПодсистемы.Обсуждения`: только программные имена от корня, без `Subsystem.` и без повторения вида. Платформенная запись `Subsystem.A.Subsystem.B` остаётся форматом XML и преобразуется только в том XML-reader, которому она реально нужна.
- `MetadataAddress` возвращается к общей грамматике из `main`; подсистемная ветка, лимит и специальный prefix из PR удаляются. Законный адрес одного объекта `Subsystem.A` не переосмысливается, но путь вложенности подсистем больше не кодируется как `MetadataAddress`.
- Порядок `tree` и обоих списков — детерминированный pre-order по регистрации: сначала порядок `Configuration/ChildObjects`, затем порядок каждого `Subsystem/ChildObjects`. Алфавитная сортировка файлов недопустима: спецификация объявляет порядок `ChildObjects` значимым.
- Роли образуют точное разбиение зарегистрированных узлов: `interface`, только если собственный `IncludeInCommandInterface=true` и он `true` у всех зарегистрированных предков; иначе `functional`. `true` под `false` остаётся функциональным. Третьего вида для совпадающих по назначению структур нет.
- Источником дерева являются только регистрации. Лишний XML-файл игнорируется; зарегистрированный, но отсутствующий/связанный ссылкой/нечитаемый/переименованный/повреждённый дескриптор делает топологию недоказанной.
- `Name`, `Content`, `ChildObjects` и `IncludeInCommandInterface` читаются по действующей спецификации формата. `IncludeInCommandInterface` обязан встречаться ровно один раз и содержать ровно `true` или `false`; отсутствие, `True`, `1` и другие значения — ошибка доказательства.
- Предел — восемь **имён подсистем**, а не восемь сегментов вместе с видом. Один доменный `SUBSYSTEM_ADDRESS_MAX_DEPTH` используется и парсером адреса, и безопасным обходом.
- Перед каждым исправлением дефекта добавить тест, запустить его на текущем коде и зафиксировать ожидаемое падение по нужной причине. Только после этого менять реализацию и повторять тот же тест до PASS.
- Публичный контракт меняется, поэтому в том же PR обязательны approved design note, новая ADR, выведенный инвариант, архитектурная ведомость и skill-документация.

---

### Task 0: Привязать работу к существующему PR и обновить базу

**Files:**
- Preserve: `docs/plans/2026-08-09-pr-386-subsystem-address-and-topology.md`
- Do not modify yet: production code

- [ ] **Step 1: Confirm the live PR head and working tree**

Run:

```bash
git status --short --branch
gh pr view 386 --repo IngvarConsulting/unica --json state,isDraft,headRefName,headRefOid,baseRefName,url
git fetch origin
```

Expected: PR открыт, не draft, head-ветка совпадает с указанной выше; рабочее дерево содержит только этот план либо чисто. Если remote head изменился, сначала перечитать diff и новые комментарии.

- [ ] **Step 2: Attach a local branch without creating another PR**

Если checkout всё ещё detached, получить существующую head-ветку cross-fork PR через GitHub CLI:

```bash
gh pr checkout 386 --repo IngvarConsulting/unica
```

Expected: локальная ветка называется `feat/meta-register-subsystem-command-interface`, указывает на head PR и имеет `remote`/`pushremote` `https://github.com/Oxotka/unica.git`. Публикация в Task 7 идёт обычным `git push` в существующую head-ветку fork.

- [ ] **Step 3: Bring in the current `main` without rewriting shared history**

Run:

```bash
git rev-list --left-right --count origin/main...HEAD
git merge --no-edit origin/main
```

Expected: ветка содержит актуальный `main`. Если merge уже не нужен, Git сообщает `Already up to date`. Rebase и force-push использовать только после отдельного согласования с владельцем ветки.

- [ ] **Step 4: Re-run the narrow baseline**

Run:

```bash
cargo test -p unica-coder source_target::tests
cargo test -p unica-coder subsystem_info_typed_result_tests
cargo test -p unica-coder meta_info_surface_tests
```

Expected: текущий PR остаётся зелёным до появления новых регрессионных тестов.

---

### Task 1: Зафиксировать согласованное архитектурное решение

**Files:**
- Create: `docs/design/2026-08-09-subsystem-address-and-effective-role-design.md`
- Create: `spec/decisions/0033-subsystem-address-and-effective-role.md`
- Modify: `spec/decisions/README.md`
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/architecture/building-blocks.md`

**Interfaces:**
- Decision owner: provisional `ADR-0033`; перед merge номер сверяется с уже попавшими в `main` решениями и при конфликте меняется во всех ссылках. На исходном `main` ADR-0031 уже accepted, ADR-0032 уже proposed.
- Derived rule: `INV-SOURCE-SUBSYSTEM-TOPOLOGY`.

- [ ] **Step 1: Write the approved design note**

Начать документ строго так:

```markdown
- Date: `2026-08-09`
- Status: `approved`
- Decision: `ADR-0033`
```

В записке воспроизвести принятые границы: отдельный публичный тип; диалект БСП; сохранение `tree`; два списка рядом; registered-only topology; эффективная роль по цепочке; fail-closed; один построитель для `subsystem.info` и `meta.info`; pre-order регистрации; поведение для вложенного каталога `Subsystems` как проекции ветки.

- [ ] **Step 2: Add ADR and the derived invariant**

ADR должна иметь `Статус: accepted`, дату `2026-08-09` и разделы `Контекст`, `Решение`, `Неграницы`, `Последствия`, `Верификация`. В `Решении` явно записать:

1. `SubsystemAddress` не является `MetadataAddress`/`SourceTarget`.
2. Публичный диалект — БСП, платформенный repeated-kind остаётся на XML-границе.
3. Три результата каталога (`tree`, два списка) — проекции одного зарегистрированного графа.
4. Роль эффективна по всей цепочке и даёт исчерпывающее разбиение.
5. Неполное доказательство не превращается в пустой список или предупреждение об отсутствии.

В `INV-SOURCE-SUBSYSTEM-TOPOLOGY` сослаться на ADR и на Rust-тесты построителя/публичной поверхности. В `building-blocks.md` добавить инфраструктурный `subsystem_topology` как единственного владельца чтения зарегистрированного графа.

- [ ] **Step 3: Verify documentation structure**

Run:

```bash
python3.12 -m unittest tests/ci/test_design_documents.py tests/ci/test_architecture_registry.py
```

Expected: PASS. Если к этому моменту `main` уже занял ADR-0033, перенумеровать запись и все ссылки до запуска.

- [ ] **Step 4: Commit**

```bash
git add docs/design/2026-08-09-subsystem-address-and-effective-role-design.md spec/decisions spec/architecture/invariants.md spec/architecture/building-blocks.md docs/plans/2026-08-09-pr-386-subsystem-address-and-topology.md
git commit -m "docs(subsystem): зафиксировать адрес и доказанную топологию"
```

---

### Task 2: Ввести `SubsystemAddress` и убрать подсистему из специальной грамматики `MetadataAddress`

**Files:**
- Create: `crates/unica-coder/src/domain/subsystem.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Modify: `crates/unica-coder/src/domain/source_target.rs`

**Interfaces:**

```rust
pub const SUBSYSTEM_ADDRESS_MAX_DEPTH: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct SubsystemAddress(String);

impl SubsystemAddress {
    pub fn parse(raw: &str) -> Result<Self, SubsystemAddressError>;
    pub fn as_str(&self) -> &str;
    pub(crate) fn from_names<'a>(names: impl IntoIterator<Item = &'a str>)
        -> Result<Self, SubsystemAddressError>;
}

pub(crate) enum EffectiveSubsystemRole {
    Functional,
    Interface,
}
```

- [ ] **Step 1: Write failing domain tests**

В `domain/subsystem.rs` сначала добавить тесты, которые требуют:

- `СтандартныеПодсистемы.Обсуждения` парсится и сериализуется строкой без префикса;
- регистр и исходное написание имён сохраняются;
- пустой сегмент, невалидный идентификатор 1С и 9-й уровень отклоняются;
- ровно 8 имён принимаются;
- `from_names(["A", "B"])` даёт `A.B`.

В `source_target.rs` заменить тест PR `flat_subsystem_addresses_and_their_prefixes_share_one_depth` регрессией: `Subsystem.A.B` и такой prefix снова отклоняются общей грамматикой, а `Subsystem.A` остаётся обычным двухсегментным metadata address.

- [ ] **Step 2: Run tests and record the expected failure**

```bash
cargo test -p unica-coder domain::subsystem::tests source_target::tests
```

Expected: FAIL — модуля/типа ещё нет, а текущая специальная ветка принимает `Subsystem.A.B`.

- [ ] **Step 3: Implement the minimal domain type and revert the address hack**

Использовать Unicode-aware правила идентификатора: первый символ — `_` или alphabetic, остальные — `_` или alphanumeric. В `source_target.rs` удалить `SUBSYSTEM_ADDRESS_MAX_SEGMENTS`, обе ветки `subsystem_root` и подсистемный тест PR, вернув общую грамматику и общий предел prefix из `main`.

- [ ] **Step 4: Re-run tests**

```bash
cargo test -p unica-coder domain::subsystem::tests source_target::tests
```

Expected: PASS; отдельно проверить, что `MetadataAddress::target_kind()` больше не вызывается для плоского пути подсистемы.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/domain/mod.rs crates/unica-coder/src/domain/subsystem.rs crates/unica-coder/src/domain/source_target.rs
git commit -m "refactor(subsystem): отделить SubsystemAddress от MetadataAddress"
```

---

### Task 3: Построить единый зарегистрированный граф под безопасным снимком

**Files:**
- Create: `crates/unica-coder/src/infrastructure/subsystem_topology.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Reuse: `crates/unica-coder/src/infrastructure/platform/secure_read.rs`
- Test in: `crates/unica-coder/src/infrastructure/subsystem_topology.rs`

**Interfaces:**

```rust
pub(crate) struct SubsystemTopology {
    pub(crate) roots: Vec<SubsystemTopologyNode>,
}

pub(crate) struct SubsystemTopologyNode {
    pub(crate) address: SubsystemAddress,
    pub(crate) name: String,
    pub(crate) role: EffectiveSubsystemRole,
    pub(crate) content: Vec<String>,
    pub(crate) children: Vec<SubsystemTopologyNode>,
}

pub(crate) fn capture_registered_subsystem_topology(
    source_root: &Path,
    checkpoint: impl FnMut() -> io::Result<()>,
) -> Result<SubsystemTopology, SubsystemTopologyError>;

impl SubsystemTopology {
    pub(crate) fn functional_addresses(&self) -> Vec<SubsystemAddress>;
    pub(crate) fn interface_addresses(&self) -> Vec<SubsystemAddress>;
    pub(crate) fn interface_memberships(&self, object_ref: &str) -> Vec<SubsystemAddress>;
}
```

- [ ] **Step 1: Write the failing topology matrix**

Создать фикстуры Platform XML и тесты на следующие независимые случаи:

1. Корни берутся из `Configuration/ChildObjects`, дети — только из `Subsystem/ChildObjects`; лишние валидные и повреждённые незарегистрированные XML игнорируются.
2. Порядок результата следует регистрации, даже если имена файлов сортируются иначе.
3. Цепочка `true → true` даёт interface/interface; `false → true` даёт functional/functional; каждый зарегистрированный узел встречается ровно в одном списке.
4. Объект в `Content` интерфейсного узла попадает в `interface_memberships`; тот же объект под исключённым предком — нет.
5. Зарегистрированный отсутствующий файл, symlink вместо файла, несовпадение `<Name>` и имени регистрации, дубликат регистрации, отсутствующий/повторный/небулевый `IncludeInCommandInterface`, повреждённый XML и глубина 9 возвращают ошибку.
6. Конфигурация без зарегистрированных подсистем доказывает пустую топологию даже без каталога `Subsystems`.
7. Ошибка checkpoint до чтения, после последнего разбора и непосредственно перед успешным возвратом не может дать `Ok`; пустая топология также проходит финальный checkpoint.

- [ ] **Step 2: Run the new tests and verify failure**

```bash
cargo test -p unica-coder subsystem_topology::tests
```

Expected: FAIL — построителя ещё нет.

- [ ] **Step 3: Capture one stable source-root snapshot**

Вызвать `capture_root_relative_regular_files(source_root, Path::new(""), ...)`, выбирая `Configuration.xml` и descriptor XML под допустимой структурой `Subsystems/<Name>(/Subsystems/<Name>)*.xml`; в `Ext/` не спускаться. Лимиты entries/files/bytes и `maximum_depth = SUBSYSTEM_ADDRESS_MAX_DEPTH * 2` определить здесь один раз.

Сначала построить индекс снимка по логическому пути, затем разбирать **только зарегистрированные** дескрипторы. Это позволяет игнорировать лишние файлы, но не позволяет им доказывать членство.

- [ ] **Step 4: Parse and prove the graph fail-closed**

Для каждого уровня:

- проверить единственный ожидаемый объект XML и обязательные `Properties`, `Name`, `Content`, `ChildObjects`, `IncludeInCommandInterface`;
- проверить точное совпадение регистрации, имени файла и `<Name>`;
- вычислить адрес из цепочки имён через `SubsystemAddress::from_names`;
- вычислить effective role как `ancestor_effective_include && own_include`;
- сохранить порядок `ChildObjects`, не сортировать узлы после разбора;
- вызвать checkpoint перед каждым разбором и ещё раз перед `Ok`, включая ветку пустого дерева.

- [ ] **Step 5: Re-run tests**

```bash
cargo test -p unica-coder subsystem_topology::tests
```

Expected: PASS по всей матрице; ни один malformed/incomplete case не возвращает пустые доказанные списки.

- [ ] **Step 6: Commit**

```bash
git add crates/unica-coder/src/infrastructure/mod.rs crates/unica-coder/src/infrastructure/subsystem_topology.rs
git commit -m "feat(subsystem): доказать зарегистрированную топологию"
```

---

### Task 4: Вернуть из `unica.subsystem.info` дерево и два плоских списка

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/native_operations/subsystem.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/typed_result.rs` only if exhaustive matching requires it
- Modify: `crates/unica-coder/src/application/mod.rs` tests only if public serialization is asserted there

**Result contract:**

```rust
Tree {
    tree: Vec<SubsystemTreeNode>,
    functional_subsystems: Vec<SubsystemAddress>,
    interface_subsystems: Vec<SubsystemAddress>,
}
```

`serde(rename_all = "camelCase")` обязан дать ровно `tree`, `functionalSubsystems`, `interfaceSubsystems`.

- [ ] **Step 1: Rewrite the current tree fixture as a registered topology and make it fail**

В `subsystem_info_typed_result_tests` заменить незарегистрированные файловые фикстуры на `Configuration.xml` плюс `ChildObjects` во всех родителях и явные канонические флаги. Добавить точную JSON-проверку:

```json
{
  "tree": [
    {
      "name": "СтандартныеПодсистемы",
      "content": 0,
      "children": [
        {"name": "Обсуждения", "content": 1, "children": []}
      ]
    }
  ],
  "functionalSubsystems": ["Служебные", "Служебные.ФоновыеЗадания"],
  "interfaceSubsystems": ["СтандартныеПодсистемы", "СтандартныеПодсистемы.Обсуждения"]
}
```

Также добавить тесты: незарегистрированный файл не появляется ни в одной проекции; `true` под `false` остаётся functional; malformed registered descriptor завершает tool error, а не возвращает пустые списки.

- [ ] **Step 2: Run tests and verify the contract fails**

```bash
cargo test -p unica-coder subsystem_info_typed_result_tests
```

Expected: FAIL — текущий вариант содержит только `tree` и файловый scanner не знает регистрации.

- [ ] **Step 3: Replace `subsystem_tree_nodes` with topology projections**

Для верхнего `Subsystems/` определить `source_root` как родителя каталога. Для поддерживаемого сейчас вложенного `.../Subsystems/<Parent>/Subsystems` найти внешний source root, разобрать alternating path и выбрать зарегистрированную ветку из полного графа. `tree` ветки содержит те же узлы, что раньше, а оба списка разделяют именно возвращаемое поддерево; адреса остаются абсолютными от корня конфигурации.

Одиночный subsystem result (`SubsystemInfoAnswer::Subsystem`) не менять: это другой контракт и не должен случайно потребовать чтения всей конфигурации.

- [ ] **Step 4: Re-run focused tests**

```bash
cargo test -p unica-coder subsystem_info_typed_result_tests
```

Expected: PASS; сериализация не содержит `Subsystem.` и не теряет `tree`.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/infrastructure/native_operations/subsystem.rs crates/unica-coder/src/infrastructure/native_operations/typed_result.rs crates/unica-coder/src/application/mod.rs
git commit -m "feat(subsystem): вернуть дерево и списки эффективных ролей"
```

Перед `git add` исключить из команды файлы, которые фактически не менялись.

---

### Task 5: Перевести `meta.info` и валидатор на тот же граф

**Files:**
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/info.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/validation_context.rs`
- Modify: `crates/unica-coder/src/application/meta_info_surface_tests.rs`
- Modify: `crates/unica-coder/src/application/meta_add_surface_tests.rs`
- Modify: `crates/unica-coder/src/infrastructure/metadata_operations.rs`

**Interfaces:**

```rust
pub(crate) enum MetadataSubsystemEvidence {
    Complete {
        interface_subsystems: Vec<SubsystemAddress>,
    },
    Unavailable(Vec<MetaDiagnostic>),
}

pub(crate) struct MetadataValidationSubject {
    // existing fields unchanged
    pub(crate) subsystem_evidence: Option<MetadataSubsystemEvidence>,
}
```

`None` означает «не собирали и ничего не утверждаем»; `Complete { [] }` — доказанное отсутствие; `Unavailable` — проверка не завершена.

- [ ] **Step 1: Add failing validator tests for the evidence states**

Тестами закрепить:

- relevant register + `Complete { interface_subsystems: [] }` даёт semantic warning;
- непустой список подавляет warning;
- `None` молчит для add/edit;
- `Unavailable` не рождает ложный semantic warning, но `validate_complete_read` возвращает `provider_unavailable`;
- обычный `MetadataResourceRole::Dependency` больше не распознаётся как подсистема по префиксу target.

Run:

```bash
cargo test -p unica-coder meta::validation::tests
```

Expected: FAIL — текущая модель знает только availability и ищет подсистемы среди generic dependencies.

- [ ] **Step 2: Replace the evidence model and validator branch**

В `ports.rs` добавить отдельный enum. В `validation.rs` удалить особое сравнение leaf identity для `Dependency { target: Subsystem... }`; language/прочие зависимости снова проходят общий путь. `meta_validate_check_register_command_interface` принимает только typed evidence для этой проверки и не делает прямой filesystem scan.

- [ ] **Step 3: Delete the dead duplicate scanner**

Удалить из `validation_context.rs` введённые PR функции/лимиты `meta_validate_subsystem_command_interface_scan` и их тесты. Не удалять общий `MetaValidationReferenceInputs.config_dir` и существующие ветки других проверок: они предшествуют PR и не относятся к этой регрессии. У вызова subsystem rule убрать только его `config_dir`-аргумент.

- [ ] **Step 4: Write failing `meta.info` surface tests against registrations**

Переписать helpers так, чтобы они регистрировали корневые подсистемы в `Configuration/ChildObjects`, а дочерние — в родительском `ChildObjects`. Добавить до реализации случаи:

- незарегистрированная подсистема с нужным `Content` не подавляет warning;
- незарегистрированный родитель не доказывает цепочку ребёнка;
- missing/renamed registered descriptor и отсутствующий/небулевый flag дают `provider_unavailable`, не «нет раздела»;
- `true` под `false` даёт warning;
- зарегистрированный `true → true` с объектом проходит;
- cancellation/deadline после последнего обработанного descriptor и на пустой топологии не возвращают complete result.

Run:

```bash
cargo test -p unica-coder meta_info_surface_tests
```

Expected: FAIL на current scanner: он доверяет файлам без регистраций и принимает неполные дескрипторы.

- [ ] **Step 5: Gather evidence from `SubsystemTopology`**

В `meta/info.rs` удалить `typed_subsystem_images`, `subsystem_names_from_logical_path`, локальные scan limits и `SubsystemDescriptorFacts`. Для релевантного регистра вызвать общий builder с `registrar_scan_checkpoint(deadline, cancellation)`, затем вернуть `Complete { interface_subsystems: topology.interface_memberships(target.as_str()) }`. Ошибку builder преобразовать в `ProviderUnavailable` с field `subsystemEvidence`.

Не добавлять descriptor bytes в `subject.resources`: граф уже доказал identity, регистрацию и effective role, а повторный разбор в validator создавал вторую политику.

- [ ] **Step 6: Update construction and mutation tests**

Все конструкторы `MetadataValidationSubject` должны задавать `None` либо новый enum осознанно. В `meta_add_surface_tests` оставить доказательство, что mutation без обзора конфигурации не выдаёт межобъектный warning (ADR-0030). В `metadata_operations.rs` заменить ожидание fake dependency `Subsystem.Main` ожиданием typed evidence или убрать его, если этот сценарий mutation не собирает доказательства.

- [ ] **Step 7: Re-run the complete focused slice**

```bash
cargo test -p unica-coder meta::validation::tests
cargo test -p unica-coder meta_info_surface_tests
cargo test -p unica-coder meta_add_surface_tests
cargo test -p unica-coder metadata_operations
```

Expected: PASS; `subsystem.info` и `meta.info` импортируют один и тот же builder, а в production code больше нет второго recursive subsystem scanner.

- [ ] **Step 8: Commit**

```bash
git add crates/unica-coder/src/application/ports.rs crates/unica-coder/src/application/meta_info_surface_tests.rs crates/unica-coder/src/application/meta_add_surface_tests.rs crates/unica-coder/src/infrastructure/metadata_operations.rs crates/unica-coder/src/infrastructure/native_operations/meta
git commit -m "fix(meta): проверять разделы по зарегистрированной топологии"
```

---

### Task 6: Синхронизировать форматную и публичную документацию

**Files:**
- Modify: `plugins/unica/references/specs/1c-subsystem-spec.md`
- Modify: `plugins/unica/references/platform/metadata-conventions.md`
- Modify: `plugins/unica/skills/subsystem-info/SKILL.md`
- Modify: `spec/architecture/tool-surface.md`
- Modify: `spec/architecture/tool-surface-review.json`
- Modify: `tests/ci/test_reference_metadata_conventions.py`
- Modify: `tests/ci/test_meta_surface_contract.py`
- Create: `tests/ci/test_subsystem_surface_contract.py`

- [ ] **Step 1: Make the prose-contract tests expose the current contradiction**

До правки прозы добавить assertions, что:

- `IncludeInCommandInterface` обязателен и каноничен; отсутствующий flag не описывается как default true;
- tool surface и skill перечисляют `tree`, `functionalSubsystems`, `interfaceSubsystems`;
- публичный пример использует `СтандартныеПодсистемы.Обсуждения`, а platform XML example сохраняет `Subsystem.СтандартныеПодсистемы.Subsystem.Обсуждения`.

Новый `test_subsystem_surface_contract.py` должен связывать слои: проверять наличие `SubsystemAddress` в отдельном domain module, отсутствие специального подсистемного лимита в `source_target.rs`, три поля tree-result и ссылки ledger/skill на ADR/invariant. Поведенную семантику этот guard не дублирует — она остаётся в Rust-тестах.

Run:

```bash
python3.12 -m unittest tests/ci/test_reference_metadata_conventions.py tests/ci/test_meta_surface_contract.py tests/ci/test_tool_surface_ledger.py tests/ci/test_subsystem_surface_contract.py
```

Expected: FAIL — текущая проза заявляет default для отсутствующего flag и описывает только `tree`.

- [ ] **Step 2: Align the sources according to their authority**

В format spec не переименовывать платформенные ссылки в БСП-диалект. Дописать только строгий статус обязательного boolean и ошибку неполного доказательства. В `metadata-conventions.md` заменить «функциональные флага не несут» на effective role: выгрузка несёт явный `false`, а исключённый предок делает функциональными всех потомков независимо от их собственного `true`.

В skill и tool ledger показать два списка рядом с сохранённым деревом, объяснить диалект адресов и fail-closed. Для вложенного каталога описать, что списки относятся к возвращаемому поддереву, но адреса остаются от корня конфигурации.

- [ ] **Step 3: Run contract tests**

```bash
python3.12 -m unittest tests/ci/test_reference_metadata_conventions.py tests/ci/test_meta_surface_contract.py tests/ci/test_tool_surface_ledger.py tests/ci/test_subsystem_surface_contract.py tests/ci/test_architecture_sync_guard.py tests/ci/test_architecture_registry.py tests/ci/test_design_documents.py
python3.12 scripts/ci/check-architecture-sync.py
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add plugins/unica/references/specs/1c-subsystem-spec.md plugins/unica/references/platform/metadata-conventions.md plugins/unica/skills/subsystem-info/SKILL.md spec/architecture tests/ci
git commit -m "docs(subsystem): синхронизировать адреса и эффективные роли"
```

---

### Task 7: Полная проверка, повторный аудит и обновление существующего PR

**Files:**
- Modify externally: title, body and comments of PR #386
- No new production files unless a failing verification exposes a root cause

- [ ] **Step 1: Prove there is one implementation path**

Run:

```bash
rg -n "typed_subsystem_images|meta_validate_subsystem_command_interface_scan|SUBSYSTEM_ADDRESS_MAX_SEGMENTS|target\.segments\(\).*Subsystem" crates/unica-coder/src
rg -n "capture_registered_subsystem_topology" crates/unica-coder/src
```

Expected: первая команда ничего не находит; вторая показывает domain-neutral builder и ровно двух production consumers — `subsystem.info` и `meta.info`.

- [ ] **Step 2: Run formatting, lint and all relevant tests**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test -p unica-coder
python3.12 -m unittest discover -s tests/ci -p 'test_*.py'
git diff --check origin/main...HEAD
```

Expected: все команды PASS. Если найден дефект, сначала добавить отдельный падающий regression test и исправить причину; не ослаблять проверку.

- [ ] **Step 3: Refresh `main`, ADR number and PR diff**

```bash
git fetch origin
git merge --no-edit origin/main
git diff --stat origin/main...HEAD
git diff --check origin/main...HEAD
gh pr checks 386 --repo IngvarConsulting/unica
```

Повторно проверить, не занял ли `main` provisional ADR number. После merge повторить Step 2. Просмотреть итоговый diff на отсутствие несвязанных изменений и убедиться, что `tree` сохранён.

- [ ] **Step 4: Update the PR title and description**

Заменить узкий title про одно предупреждение на `feat(subsystem): публиковать эффективную топологию для meta` либо эквивалентный заголовок, который отражает обе публичные проекции и их использование валидатором.

Убрать из body утверждения про default true, внутренний `MetadataAddress` и файловый scanner. Новое описание должно назвать:

- `SubsystemAddress` в диалекте БСП;
- registered-only shared topology;
- `tree` плюс два списка;
- effective role по цепочке;
- unavailable на неполном доказательстве;
- сохранение тишины add/edit без собранного межобъектного evidence;
- фактические команды и количество прошедших тестов.

- [ ] **Step 5: Push to the existing PR head**

```bash
git config --get branch.feat/meta-register-subsystem-command-interface.pushremote
git push
```

Expected: pushremote равен `https://github.com/Oxotka/unica.git`, PR #386 обновлён; новый PR не создан. Не отправлять head в `origin`: PR создан из fork.

- [ ] **Step 6: Verify persisted GitHub state and post the resolution comment**

```bash
gh pr view 386 --repo IngvarConsulting/unica --json headRefOid,body,url
gh pr checks 386 --repo IngvarConsulting/unica --watch
```

После зелёных checks оставить один итоговый комментарий: перечислить закрытые проблемы из аудита, показать пример трёх полей результата и сослаться на тесты. Не объявлять вопрос закрытым, пока live head не совпадает с pushed SHA и обязательные checks не завершились успешно.

---

## Acceptance Checklist

- [ ] `MetadataAddress` не содержит специальной плоской грамматики подсистем.
- [ ] `SubsystemAddress` сериализует `A.B`, а не `Subsystem.A.Subsystem.B` и не `Subsystem.A.B`.
- [ ] `tree` сохранён; два списка находятся рядом и вместе содержат каждый возвращённый зарегистрированный узел ровно один раз.
- [ ] Порядок всех проекций следует `ChildObjects`.
- [ ] Незарегистрированный файл не влияет ни на lists, ни на `meta` warning.
- [ ] Любой разрыв зарегистрированной цепочки и любой неполный read дают unavailable/error, не доказанный пустой список.
- [ ] Собственный `true` под эффективным `false` классифицируется как functional.
- [ ] `subsystem.info` и `meta.info` используют один production builder.
- [ ] Add/edit без обзора конфигурации не выдают ложное межобъектное заключение.
- [ ] Форматная спецификация, convention prose, ADR/invariant, tool ledger и skill не противоречат коду.
- [ ] Изменения находятся в head существующего PR #386, PR body обновлён, live checks зелёные.
