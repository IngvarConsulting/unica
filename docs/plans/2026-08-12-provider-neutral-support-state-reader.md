# Provider-Neutral Support State Reader Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` (recommended) or
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking. Every production change starts with a
> failing test, records the observed RED reason, and ends with the narrow GREEN
> command before broader verification.

**Goal:** Закрыть
[#289](https://github.com/IngvarConsulting/unica/issues/289): семь предметных
читателей получают состояние поддержки через провайдер-нейтральный
`SupportStateReader` по логической цели; неспособность EDT/неизвестного
поставщика становится явной ошибкой, а не выдуманным `notSupported`.

**Architecture:** Домен владеет типизированными операциями порта для
конфигурации, адресуемого объекта и вложенной подсистемы. Workspace-bound
реализация выбирает адаптер по `SourceFormat`; первый срез реализует только
Platform XML. Общий мост предметных читателей возвращает одновременно
логическую цель и закрытый путь XML: предметный файл читается по пути, состояние
поддержки — только по цели. В композиционном корне reader внедряется в обычные
typed-readers, подготовленный `subsystem.info` и `meta.info`; три старые
path-функции после миграции удаляются.

**Tech Stack:** Rust 2021, `serde`, `serde_json`, `roxmltree`, существующие
`domain/source_target.rs`, `project_sources`, `platform_xml_source_targets` и
`native_operations`, Python 3.12 architecture tests, Markdown ADR/invariant
corpus.

## Global Constraints

- ADR-0054 и `INV-APP-SUPPORT-STATE` владеют новым общим контрактом. ADR-0049
  продолжает владеть совместимостью логического и временного файлового
  селекторов; ADR-0023 — единственным типизированным представлением фактов.
- Публичная поверхность `unica.*`, аргументы и успешные wire-ответы не меняются.
  `state`, `editingEnabled`, `objects`, `directEditSafe` и все строковые значения
  сериализуются побайтно так же, как на исходной ветке.
- `configuration_support` принимает только `TargetKind::SourceRoot`,
  `object_support` — только `TargetKind::MetadataObject`. Неподходящая цель даёт
  `target_unsupported`; порт никогда не принимает `Path`.
- Вложенная подсистема по ADR-0036 не является `MetadataAddress` и передаётся в
  `subsystem_support` как `ResolvedSubsystemTarget { sourceSet,
  SubsystemAddress }`; верхнеуровневая подсистема остаётся `ResolvedTarget`.
- Только доказанный Platform XML-набор без marker-а означает `notSupported` (для
  расширения — `extension`). EDT, `Unknown` и `Invalid` дают
  `provider_unavailable`. Нерегулярный marker даёт `state_unreadable`,
  непустой marker без распознаваемого заголовка — `state_invalid`.
- Файловый селектор должен принадлежать ровно одному самому глубокому
  зарегистрированному набору и пройти существующую обратную локализацию. Нельзя
  выводить адрес подъёмом по каталогам или именем файла.
- Внутренняя политика inverse locator-а для reader bridge доказывает точные
  QName, вид и имя descriptor-а при любой согласованной сырой версии; обычный
  resolver затем доказывает регистрацию и равенство версий owner/target, а
  format guard сохраняет обязанность классифицировать старый или новый формат.
  Публичная политика `source.locate` остаётся ограниченной активным профилем.
- `unica.support.edit` и mutating support guard не входят в задачу и сохраняют
  действующую path/pre-image семантику. Их `find_support_config_dir`,
  `read_support_state`, UUID dependency helpers и тесты не удаляются.
- В `dcs.info` уже есть `data.support`: нового поля не добавлять. Удаляется
  только устаревшая строка старого text builder-а и её path-helper.
- Каждый дефект выполняется red → green. Rust-тесты запускать с
  `-- --test-threads=1`; Python CI — через `python3.12`.
- После каждого task запускать `cargo fmt --all`, `git diff --check` и
  `git status --short`. Коммиты ниже — точки самостоятельного ревью одного
  итогового implementation PR.

## File Structure

| File | Responsibility |
| --- | --- |
| Create: `crates/unica-coder/src/domain/support_state.rs` | Доменные ответы, закрытые enum, стабильная ошибка и `SupportStateReader`. |
| Create: `crates/unica-coder/src/infrastructure/support_state.rs` | Workspace-bound выбор поставщика и Platform XML adapter; физический marker остаётся здесь. |
| Modify: `crates/unica-coder/src/infrastructure/native_operations/logical_selector.rs` | `ResolvedReadTarget` и доказанный мост от временного пути к `ResolvedTarget`. |
| Modify: `crates/unica-coder/src/infrastructure/application_ports.rs` | Фабрика reader-а, внедрение в native/meta/prepared subsystem маршруты и архитектурный guard-test. |
| Modify: `crates/unica-coder/src/infrastructure/native_operations/typed_result.rs` | Передача малого порта пяти обычным typed-readers. |
| Modify: `crates/unica-coder/src/infrastructure/native_operations/{common,cf,role,mxl,dcs,form,subsystem}.rs` | Миграция читателей и удаление трёх path-based read helpers. |
| Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/{edit,info}.rs`, `infrastructure/metadata_operations.rs` | Сохранение разрешённой цели и чтение поддержки `meta.info` через порт. |
| Modify: `spec/architecture/{invariants,building-blocks,concepts}.md`, `spec/decisions/{0054-provider-neutral-support-state-reader,README}.md` | Проверяемое следствие ADR, границы reader/guard и принятие решения. |

---

## Task 0: Зафиксировать изоляцию и baseline реализации

**Files:** none.

**Interfaces:**

- Consumes: утверждённые design и proposed ADR в коммите `0ab5ab8e` плюс этот
  план.
- Produces: ветка `codex/issue-289-provider-neutral-support-state-reader`,
  созданная в текущем app-managed linked worktree без второго worktree.

- [ ] **Step 1: Verify the worktree and clean tree.**

```bash
git rev-parse --git-dir
git rev-parse --git-common-dir
git rev-parse --show-superproject-working-tree
git status --short --branch
```

Expected: git-dir начинается с `.git/worktrees/`, common-dir — основной
репозиторий, superproject пуст, рабочее дерево чистое. Если это перестало быть
так, не переносить чужие изменения: остановиться и разрешить пересечение.

- [ ] **Step 2: Create the implementation branch at the reviewed documents.**

```bash
git switch -c codex/issue-289-provider-neutral-support-state-reader
git merge-base --is-ancestor origin/main HEAD
```

Expected: новая ветка содержит актуальный `origin/main` и только утверждённые
документы/план поверх него.

- [ ] **Step 3: Re-run the recorded baseline.**

```bash
cargo build -p unica-coder
cargo test -p unica-coder role_info_reads_the_support_state_from_the_configuration_root -- --test-threads=1
cargo test -p unica-coder dcs_info -- --test-threads=1
```

Expected: PASS. Это отделяет регрессию реализации от исходной ветки.

---

## Task 1: Ввести доменный порт без файловой ручки

**Files:**

- Create: `crates/unica-coder/src/domain/support_state.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`

**Interfaces:**

- Consumes: `domain::source_target::ResolvedTarget` и
  `domain::subsystem::SubsystemAddress`.
- Produces for Tasks 2–6:

```rust
pub trait SupportStateReader: Send + Sync {
    fn configuration_support(
        &self,
        target: &ResolvedTarget,
    ) -> Result<ConfigurationSupportData, SupportReadError>;

    fn object_support(
        &self,
        target: &ResolvedTarget,
    ) -> Result<ObjectSupportData, SupportReadError>;

    fn subsystem_support(
        &self,
        target: &ResolvedSubsystemTarget,
    ) -> Result<ObjectSupportData, SupportReadError>;
}

pub enum SupportReadErrorCode {
    ProviderUnavailable,
    TargetUnsupported,
    EvidenceUnavailable,
    StateUnreadable,
    StateInvalid,
}
```

`ConfigurationSupportState` сериализуется как `notSupported | extension |
removed | supported`; `ObjectSupportState` — как `notSupported |
removedFromSupport | configurationReadOnly | locked | editableWithSupport`.

- [ ] **Step 1: Write the failing domain contract tests first.**

В новом модуле добавить тесты
`support_state_wire_values_preserve_the_reader_contract` и
`provider_unavailable_is_an_error_not_not_supported`. Первый сериализует все
варианты и точные поля, второй использует fake-reader:

```rust
struct UnavailableReader;

impl SupportStateReader for UnavailableReader {
    fn configuration_support(
        &self,
        _target: &ResolvedTarget,
    ) -> Result<ConfigurationSupportData, SupportReadError> {
        Err(SupportReadError::new(
            SupportReadErrorCode::ProviderUnavailable,
            "support-state provider is unavailable",
        ))
    }

    fn object_support(
        &self,
        _target: &ResolvedTarget,
    ) -> Result<ObjectSupportData, SupportReadError> {
        Err(SupportReadError::new(
            SupportReadErrorCode::ProviderUnavailable,
            "support-state provider is unavailable",
        ))
    }

    fn subsystem_support(
        &self,
        _target: &ResolvedSubsystemTarget,
    ) -> Result<ObjectSupportData, SupportReadError> {
        Err(SupportReadError::new(
            SupportReadErrorCode::ProviderUnavailable,
            "support-state provider is unavailable",
        ))
    }
}
```

Подключить `pub mod support_state;`, но до объявления production-типов запустить
тест.

- [ ] **Step 2: Verify RED.**

```bash
cargo test -p unica-coder support_state_wire_values_preserve -- --test-threads=1
```

Expected: compile FAIL на отсутствующих `SupportStateReader`/типах. Записать
именно эту причину; не исправлять посторонние предупреждения.

- [ ] **Step 3: Implement the minimal closed domain model.**

Добавить `Debug + Clone + PartialEq + Eq + Serialize` у wire-типов,
`#[serde(rename_all = "camelCase")]` у структур и enum-состояний,
`#[serde(rename_all = "snake_case")]` у кода ошибки. `SupportReadError`
хранит `code` и безопасное `message`, реализует `Display` как
`<snake_case_code>: <message>` и `std::error::Error`. Не добавлять `PathBuf` или
provider handle ни в один публичный доменный тип.

- [ ] **Step 4: Run the narrow green.**

```bash
cargo fmt --all
cargo test -p unica-coder support_state_wire_values_preserve -- --test-threads=1
cargo test -p unica-coder provider_unavailable_is_an_error -- --test-threads=1
```

Expected: PASS; JSON совпадает с текущими успешными payloads.

- [ ] **Step 5: Commit.**

```bash
git add crates/unica-coder/src/domain
git diff --cached --check
git commit -m "refactor(domain): define support state reader port"
```

---

## Task 2: Реализовать workspace-bound Platform XML reader

**Files:**

- Create: `crates/unica-coder/src/infrastructure/support_state.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/common.rs`

**Interfaces:**

- Consumes: Task 1 port/types; `resolve_named_source_set`, `SourceFormat`,
  `SourceSetKind`, `resolve_platform_xml_target`,
  `platform_xml_resource_evidence`, `support_root_uuid_from_bytes` and the
  existing support-state parser.
- Produces:

```rust
pub(crate) struct WorkspaceSupportStateReader<'a> {
    context: &'a WorkspaceContext,
}

impl<'a> WorkspaceSupportStateReader<'a> {
    pub(crate) fn new(context: &'a WorkspaceContext) -> Self;
}
```

The reader resolves `target.source_set`, rejects every format except
`SourceFormat::PlatformXml`, re-resolves the exact logical target, and only then
opens `<source-root>/Ext/ParentConfigurations.bin`.

- [ ] **Step 1: Add the failing provider matrix.**

В `infrastructure/support_state.rs` создать fixtures `v8project.yaml` для
Platform XML и EDT и тесты:

- `support_state_reader_rejects_edt_instead_of_claiming_not_supported` — обе
  операции возвращают `ProviderUnavailable`;
- `platform_xml_configuration_support_distinguishes_absent_unreadable_and_invalid_marker`
  — отсутствующий marker даёт `NotSupported`, каталог вместо файла —
  `StateUnreadable`, непустые 64 байта с неверным заголовком `{9,0,1}` —
  `StateInvalid`;
- `platform_xml_object_support_uses_the_resolved_descriptor_uuid` — правило
  `0,0,<uuid>` даёт `Locked`, а не эвристический ответ по leaf path;
- `support_reader_rejects_the_wrong_target_kind` — два симметричных
  `TargetUnsupported`.

Для removed-state использовать пустой marker и корректный заголовок
`{6,0,0}`; произвольный короткий мусор не объявлять removed-state.

- [ ] **Step 2: Verify RED.**

```bash
cargo test -p unica-coder support_state_reader_rejects_edt -- --test-threads=1
```

Expected: compile FAIL, потому что `WorkspaceSupportStateReader` и модуль ещё не
существуют.

- [ ] **Step 3: Add a strict read result beside the legacy mutation parser.**

В `common.rs` оставить `read_support_state(&Path) -> Option<SupportState>` без
изменений для mutating guard/edit. Добавить отдельный crate-private strict
вход, который:

1. `symlink_metadata`: `NotFound => Ok(None)`, другая ошибка =>
   `StateUnreadable`;
2. ссылка/reparse point или не regular file => `StateUnreadable`;
3. ошибка `fs::read` => `StateUnreadable`;
4. пустые байты => removed-state;
5. непустые байты обязаны пройти `parse_support_header`, иначе
   `StateInvalid`; `vendor_count == 0` => removed-state.

Не менять fail-open трактовку старого `read_support_state`; это намеренная
неграница ADR-0054, а не скрытая унификация.

- [ ] **Step 4: Implement provider selection and exact projections.**

В `WorkspaceSupportStateReader`:

- `resolve_named_source_set` отображать на `EvidenceUnavailable`, не раскрывая
  путь;
- `Edt | Unknown | Invalid` отображать на `ProviderUnavailable` до проверки
  marker-а;
- `configuration_support` принимать `SourceRoot`, использовать kind набора для
  `Extension`, а counts проецировать в `u64`;
- `object_support` принимать `MetadataObject`, извлекать UUID только из байтов
  доказанного descriptor-а и применять прежнюю таблицу `0/1/2`;
- неизвестный UUID/rule означает доказанное `NotSupported`, а недоступный или
  неразбираемый descriptor — `EvidenceUnavailable`.

- [ ] **Step 5: Run the provider green and protect mutation semantics.**

```bash
cargo fmt --all
cargo test -p unica-coder support_state_reader -- --test-threads=1
cargo test -p unica-coder platform_xml_configuration_support -- --test-threads=1
cargo test -p unica-coder platform_xml_object_support -- --test-threads=1
cargo test -p unica-coder support_guard -- --test-threads=1
cargo test -p unica-coder support_edit -- --test-threads=1
```

Expected: provider tests PASS; existing mutation tests unchanged.

- [ ] **Step 6: Commit.**

```bash
git add crates/unica-coder/src/infrastructure/mod.rs \
  crates/unica-coder/src/infrastructure/support_state.rs \
  crates/unica-coder/src/infrastructure/native_operations/common.rs
git diff --cached --check
git commit -m "feat(infrastructure): read support state by logical target"
```

---

## Task 3: Возвращать логическую цель вместе с предметным файлом

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/native_operations/logical_selector.rs`
- Modify: focused selector tests in the same module

**Interfaces:**

- Consumes: `ResolvedTarget`, `discover_project_source_map`,
  `normalize_path_identity`, `select_unique_deepest_source_set_match`,
  `locate_platform_xml_source_path` and existing proof functions.
- Replaces `LogicalSelection` with:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedReadTarget {
    pub(crate) target: ResolvedTarget,
    pub(crate) resource_path: PathBuf,
}

pub(crate) fn physical_selection(
    resource_path: &Path,
    context: &WorkspaceContext,
    want: AttachedResource,
) -> Result<ResolvedReadTarget, LogicalSelectorFailure>;
```

`logical_selection` keeps its
`Option<Result<ResolvedReadTarget, LogicalSelectorFailure>>` selector semantics.

- [ ] **Step 1: Write the failing inverse-bridge tests.**

Расширить существующую fixture ролью, формой, макетом, DCS и подсистемой. Для
каждого `AttachedResource` проверить:

```rust
let logical = logical_selection(&logical_args, &context, want, kinds)
    .expect("logical selector was used")
    .unwrap();
let physical = physical_selection(&logical.resource_path, &context, want).unwrap();
assert_eq!(physical.target, logical.target);
assert_eq!(physical.resource_path, logical.resource_path);
```

Отдельные tests:

- `physical_support_target_uses_the_owner_of_an_attached_resource` —
  `Rights.xml`, `Form.xml` и `Template.xml` получают адрес владельца;
- `physical_support_target_rejects_unregistered_and_ambiguous_paths` — файл вне
  карты и два одинаково глубоких набора отказывают;
- `configuration_xml_maps_to_the_source_root` — `metadata_path == None` и
  `TargetKind::SourceRoot`;
- `descriptor_maps_to_its_own_metadata_address` — subsystem descriptor получает
  собственный адрес.

- [ ] **Step 2: Verify RED.**

```bash
cargo test -p unica-coder physical_support_target_uses_the_owner -- --test-threads=1
```

Expected: compile FAIL на отсутствующем `physical_selection` и поле `target`.

- [ ] **Step 3: Preserve the resolver's `ResolvedTarget` in the logical arm.**

В `resolve` после `resolve_platform_xml_target` не разбирать
`resolution.resolved` на `source_set/metadata_path`. Сохранить объект целиком и
вернуть его рядом с уже доказанным `resource_path`.

- [ ] **Step 4: Implement the physical inverse without layout guessing.**

Алгоритм `physical_selection`:

1. нормализовать путь и корни всех source sets;
2. выбрать единственный самый глубокий содержащий set;
3. для `ConfigurationRoot` потребовать точное совпадение с доказанным
   `<source-root>/Configuration.xml` и собрать корневой `SourceTarget` без
   `metadataPath`; для остальных Platform XML-ресурсов вызвать
   `locate_platform_xml_source_path` с именем set и относительным путём;
4. взять `metadata_path` для `Descriptor`, `owner_metadata_path` для
   `Rights/Form/Template`. Для прикреплённого ресурса результат локатора
   `NotAddressable` допустим только вместе с доказанным `owner_metadata_path`:
   leaf-файл не является самостоятельной логической целью. `OwnerUnproven`,
   `OutsideSourceSet` и остальные отказы остаются ошибками;
5. собрать `SourceTarget`, снова вызвать `resolve_platform_xml_target`,
   доказать `AttachedResource` и потребовать совпадение нормализованных путей;
6. любое `rejection`, формат не Platform XML, неоднозначность или несовпадение
   вернуть как стабильный `provider_unavailable | target_not_found |
   containment_denied`, не выводя физический путь в сообщении.

- [ ] **Step 5: Run the bridge green.**

```bash
cargo fmt --all
cargo test -p unica-coder logical_selector -- --test-threads=1
cargo test -p unica-coder physical_support_target -- --test-threads=1
cargo test -p unica-coder configuration_xml_maps_to_the_source_root -- --test-threads=1
```

Expected: PASS; logical and physical calls produce the same `ResolvedTarget`.

- [ ] **Step 6: Commit.**

```bash
git add crates/unica-coder/src/infrastructure/native_operations/logical_selector.rs
git diff --cached --check
git commit -m "refactor(readers): retain logical support targets"
```

---

## Task 4: Внедрить reader в обычные native typed-readers

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/typed_result.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/cf.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/role.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/mxl.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/dcs.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/form.rs`

**Interfaces:**

- Consumes: Tasks 1–3.
- Produces: `NativeOperationAdapter::invoke_with_data` получает новый параметр
  `support_reader: &dyn SupportStateReader` перед `control`; пять
  object/configuration readers мигрированы. `dcs.info` keeps the existing
  support field.

- [ ] **Step 1: Add a recording reader and failing route tests.**

В tests `application_ports.rs` определить `RecordingSupportStateReader` с
`Arc<Mutex<Vec<(Operation, ResolvedTarget)>>>` и фабрику, создающую reader для
текущего `WorkspaceContext`. Тесты обращаются к ещё отсутствующему test-only
конструктору `InfrastructureApplicationPorts::with_support_reader_factory`,
поэтому первый запуск обязан упасть на compile boundary. Интерфейс, который
нужно реализовать только после RED в Step 3:

```rust
pub(crate) trait SupportStateReaderFactory: Send + Sync {
    fn create<'a>(
        &'a self,
        context: &'a WorkspaceContext,
    ) -> Box<dyn SupportStateReader + 'a>;
}
```

Проверить существующими минимальными fixtures, что `cf.info`, `role.info`,
`mxl.info`, `dcs.info` и `form.info` записывают соответственно root или owner
target. Recording reader возвращает разные sentinel states для configuration и
object, чтобы тест доказывал вызов правильного метода, а не только наличие
поля.

- [ ] **Step 2: Verify RED.**

```bash
cargo test -p unica-coder native_typed_readers_receive_logical_support_targets -- --test-threads=1
```

Expected: compile FAIL — `InfrastructureApplicationPorts` не принимает фабрику,
а `invoke_with_data` не принимает reader.

- [ ] **Step 3: Add the production factory at the composition boundary.**

`InfrastructureApplicationPorts::new()` хранит закрытую
`WorkspaceSupportStateReaderFactory`; test-only constructor принимает recording
factory. В `invoke_handler_with_operational_config` создать reader ровно один
раз на вызов и передать ссылку в `NativeOperationAdapter::invoke_with_data`.
Обновить прямые вызовы адаптера в его unit tests явным
`WorkspaceSupportStateReader::new(&context)`; default path-based fake не
добавлять.

- [ ] **Step 4: Migrate the five analyzers.**

Для каждого info-only resolver-а возвращать `ResolvedReadTarget`: logical arm
уже имеет target, legacy arm вызывает `physical_selection`. Validators должны
по-прежнему получать только `resource_path` и не начинать читать поддержку.

Заменить вызовы:

| Reader | Old | New |
| --- | --- | --- |
| `cf.info` | `support_state_data(config_path, extension_purpose.is_some())` | `configuration_support(&selection.target)` |
| `role.info` | `object_support_state(rights_path)` | `object_support(&selection.target)` |
| `mxl.info` | `object_support_state(template_path)` | `object_support(&selection.target)` |
| `dcs.info` | `object_support_state(template_path)` | `object_support(&selection.target)` |
| `form.info` | `object_support_state(form_path)` | `object_support(&selection.target)` |

Ошибка порта возвращает failed `AdapterOutcome` с первым `errors` равным
`SupportReadError::to_string()` и `data: None`; частичный typed payload не
публикуется.

- [ ] **Step 5: Remove only DCS's duplicate prose.**

Удалить строку с префиксом `Поддержка:` из `dcs_info_overview` и импорт
`support_status_for_path`. `DcsInfoData.support` и его contract-test оставить.
Добавить failing-before-change assertion
`dcs_info_overview_does_not_render_a_second_support_grammar`, который требует
отсутствия `Поддержка:` в legacy overview при наличии typed `data.support`.

- [ ] **Step 6: Run narrow green and selector parity.**

```bash
cargo fmt --all
cargo test -p unica-coder native_typed_readers_receive_logical_support_targets -- --test-threads=1
cargo test -p unica-coder answers_identically -- --test-threads=1
cargo test -p unica-coder role_info_reads_the_support_state_from_the_configuration_root -- --test-threads=1
cargo test -p unica-coder dcs_info -- --test-threads=1
```

Expected: PASS; five readers use recording targets; all existing logical/path
parity tests retain equal `support`.

- [ ] **Step 7: Commit.**

```bash
git add crates/unica-coder/src/infrastructure/application_ports.rs \
  crates/unica-coder/src/infrastructure/native_operations/{typed_result,cf,role,mxl,dcs,form}.rs
git diff --cached --check
git commit -m "refactor(readers): inject logical support state reader"
```

---

## Task 5: Мигрировать prepared `subsystem.info`

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/subsystem.rs`

**Interfaces:**

- Consumes: Task 4 factory; `ResolvedReadTarget` from Task 3.
- Produces: prepared subsystem object read invokes `object_support` for a
  top-level descriptor or `subsystem_support` for a nested registered address
  before building `PreparedSubsystemInfo`; tree read remains without object
  support.

- [ ] **Step 1: Write the failing prepared-route tests.**

Добавить:

- `prepared_subsystem_info_records_the_descriptor_target` — через публичный
  `prepare_tool_invocation` и recording factory проверяет
  `Subsystem.Sales`/`MetadataObject`;
- `prepared_nested_subsystem_keeps_the_dedicated_subsystem_address` —
  зарегистрированный `Sales.Online` остаётся `SubsystemAddress` и не
  притворяется `MetadataAddress`;
- `subsystem_tree_does_not_invent_object_support` — каталог `Subsystems/`
  возвращает tree и не вызывает ни один метод reader-а;
- `subsystem_support_failure_publishes_no_partial_data` — reader возвращает
  `ProviderUnavailable`, outcome неуспешен, `handler.data == None`.

- [ ] **Step 2: Verify RED.**

```bash
cargo test -p unica-coder prepared_subsystem_info_records_the_descriptor_target -- --test-threads=1
```

Expected: FAIL — подготовленный путь по-прежнему вызывает
`object_support_state(&xml_path)` и recording reader пуст.

- [ ] **Step 3: Thread the reader through preparation, not after it.**

Передать `&dyn SupportStateReader` в `prepare_subsystem_info`, cancellable
wrapper и `prepare_subsystem_info_with_checkpoint`. Для верхнеуровневой
подсистемы доказать `ResolvedTarget` через существующий physical bridge; для
вложенной сохранить адрес из зарегистрированной топологии как
`ResolvedSubsystemTarget`, не расширяя `MetadataAddress` вопреки ADR-0036.
Tree branch сохраняет path-only scope. После capture/parse и перед сборкой
`SubsystemInfoResult` вызвать соответствующий метод порта с контрольной точкой
до и после. Ошибку вернуть как preparation failure без typed data.

- [ ] **Step 4: Run narrow green and deadline/cancellation regressions.**

```bash
cargo fmt --all
cargo test -p unica-coder prepared_subsystem_info -- --test-threads=1
cargo test -p unica-coder prepared_nested_subsystem -- --test-threads=1
cargo test -p unica-coder subsystem_tree_does_not_invent_object_support -- --test-threads=1
cargo test -p unica-coder subsystem_info_cancellation -- --test-threads=1
cargo test -p unica-coder subsystem_info_deadline -- --test-threads=1
```

Expected: PASS; preparation remains the only execution route and preserves
control semantics.

- [ ] **Step 5: Commit.**

```bash
git add crates/unica-coder/src/infrastructure/application_ports.rs \
  crates/unica-coder/src/infrastructure/native_operations/subsystem.rs
git diff --cached --check
git commit -m "refactor(subsystem): read support through the logical port"
```

---

## Task 6: Мигрировать `meta.info` и удалить path-based read API

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/metadata_operations.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/info.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/common.rs`

**Interfaces:**

- Consumes: resolved target already produced by
  `resolve_typed_metadata_object`; Task 4 factory.
- Produces: `ResolvedMetadataObject.resolved_target`, port-driven
  `MetaSupportStatus`, no `object_support_state`, `support_state_data` or
  `support_status_for_path` production functions.

- [ ] **Step 1: Write the failing meta route/error tests.**

В `application_ports.rs`/`metadata_operations.rs` добавить:

- `meta_info_passes_its_resolved_target_to_support_reader` — recording reader
  видит точный `sourceSet + metadataPath + MetadataObject`;
- `meta_info_maps_support_provider_failure_to_logical_diagnostic` —
  `SupportReadErrorCode::ProviderUnavailable` становится
  `MetaDiagnosticCode::ProviderUnavailable`, содержит `metadataPath`, не
  содержит separator и не публикует local data;
- `meta_info_preserves_the_existing_support_projection` — `Locked` и
  `ConfigurationReadOnly` => `MetaSupportStatus::Locked`,
  `RemovedFromSupport` => `Unsupported`, остальные => `Supported`.

- [ ] **Step 2: Verify RED.**

```bash
cargo test -p unica-coder meta_info_passes_its_resolved_target_to_support_reader -- --test-threads=1
```

Expected: compile/behavior FAIL: `read_metadata_local` не создаёт reader, а
`ResolvedMetadataObject` хранит только descriptor path.

- [ ] **Step 3: Preserve the exact resolution and inject the reader.**

Добавить в `ResolvedMetadataObject` поле `resolved_target: ResolvedTarget` из
`resolution.resolved` до перемещения handle. Из
`InfrastructureApplicationPorts::read_metadata_local` создать reader через ту
же factory и передать в `MetadataOperations::read_local`, затем в
`read_typed_meta_info`. `typed_support_status` принимает
`&dyn SupportStateReader` и `&resolved.resolved_target`; path больше не
принимает.

Ошибка порта отображается в `MetaFailure` с кодом `ProviderUnavailable` и
`with_metadata_path(target.clone())`. Стабильный support error code оставить в
message, но не включать provider path.

- [ ] **Step 4: Delete the three legacy read helpers after all callers move.**

Из `common.rs` удалить только:

```text
support_state_data
object_support_state
support_status_for_path
```

и старые output structs, уже переехавшие в домен. Сохранить
`support_state_lines_for_configuration`, `read_support_state`,
`find_support_config_dir`, UUID helpers и support guard/edit imports.

- [ ] **Step 5: Run narrow green and prove the names are gone.**

```bash
cargo fmt --all
cargo test -p unica-coder meta_info_passes_its_resolved_target -- --test-threads=1
cargo test -p unica-coder meta_info_maps_support_provider_failure -- --test-threads=1
cargo test -p unica-coder meta_info_preserves_the_existing_support_projection -- --test-threads=1
rg -n "object_support_state|support_state_data|support_status_for_path" \
  crates/unica-coder/src/infrastructure/native_operations \
  crates/unica-coder/src/infrastructure/metadata_operations.rs
```

Expected: tests PASS; `rg` exits 1 with no production matches.

- [ ] **Step 6: Commit.**

```bash
git add crates/unica-coder/src/infrastructure/application_ports.rs \
  crates/unica-coder/src/infrastructure/metadata_operations.rs \
  crates/unica-coder/src/infrastructure/native_operations/common.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta
git diff --cached --check
git commit -m "refactor(meta): read support by resolved metadata target"
```

---

## Task 7: Закрепить архитектурный контракт и принять ADR

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/architecture/building-blocks.md`
- Modify: `spec/architecture/concepts.md`
- Modify: `spec/decisions/0054-provider-neutral-support-state-reader.md`
- Modify: `spec/decisions/README.md`

**Interfaces:**

- Consumes: полностью мигрированные семь readers.
- Produces: `INV-APP-SUPPORT-STATE`, executable guard и accepted ADR-0054.

- [ ] **Step 1: Add the structural guard for the already red-proven migration.**

В tests `application_ports.rs` через `include_str!` прочитать ровно семь reader
файлов и `metadata_operations.rs`. Тест
`support_readers_cannot_bypass_the_logical_port` обязан:

1. отвергать строки `object_support_state(`, `support_state_data(` и
   `support_status_for_path(`;
2. видеть семь route-specific маршрутов `configuration_support(` /
   `object_support(` и отдельный `subsystem_support(` для вложенного адреса;
3. подтверждать, что объявление trait в `domain/support_state.rs` не содержит
   `Path`/`PathBuf`.

Это guard от повторного появления дефекта, уже воспроизведённого recording/error
tests Tasks 4–6, а не новый найденный дефект; искусственный падающий control в
production-коммит не добавлять. Mutating parser является документированной
неграницей и в forbidden set не входит.

- [ ] **Step 2: Verify the guard is GREEN.**

```bash
cargo test -p unica-coder support_readers_cannot_bypass_the_logical_port -- --test-threads=1
```

Expected: PASS. Если guard находит старое имя, миграция Tasks 4–6 неполна:
вернуться к соответствующему ранее падавшему route test, а не ослаблять guard.

- [ ] **Step 3: Add the invariant and architectural map.**

В APP-области `invariants.md` добавить запись `INV-APP-SUPPORT-STATE`: предметные
читатели получают состояние поддержки только через доменный
`SupportStateReader` по логической цели (`ResolvedTarget` либо
`ResolvedSubsystemTarget`); отсутствие реализации поставщика и недоступность
свидетельства являются ошибками, а не состоянием `notSupported`, и физическая
раскладка marker-а остаётся внутри инфраструктуры конкретного поставщика.
Владельцем записи является ADR-0054, исполняемой проверкой — тест в
`crates/unica-coder/src/infrastructure/application_ports.rs`.

В `building-blocks.md` добавить `domain::support_state` и
`infrastructure::support_state`. В `concepts.md` разделить два контракта:
readers fail-closed по ADR-0054; mutating guard пока сохраняет прежнее
fail-open отрицательное пространство как явную неграницу, а не как общее
правило чтения.

- [ ] **Step 4: Accept ADR-0054 only with executable code.**

Сменить статус ADR-0054 `proposed` → `accepted`, перенести строку в README из
«Предлагаемые» в «Принятые». Не менять номер: перед коммитом доказать, что
`origin/main` не принял параллельную ADR-0054; при конфликте перенумеровать эту
ещё не принятую запись и все её ссылки.

- [ ] **Step 5: Run architecture green.**

```bash
cargo fmt --all
cargo test -p unica-coder support_readers_cannot_bypass_the_logical_port -- --test-threads=1
python3.12 -m unittest \
  tests.ci.test_design_documents \
  tests.ci.test_architecture_registry \
  tests.ci.test_rust_platform_boundary
python3.12 scripts/ci/check-rust-platform-boundary.py
```

Expected: PASS; ADR/invariant links resolve and dependency direction remains
valid.

- [ ] **Step 6: Commit.**

```bash
git add crates/unica-coder/src/infrastructure/application_ports.rs \
  spec/architecture spec/decisions
git diff --cached --check
git commit -m "docs(architecture): require logical support state reads"
```

---

## Task 8: Полная верификация и готовность PR

**Files:** no new production files; only corrections proven by failing tests are
allowed here.

**Interfaces:**

- Consumes: Tasks 0–7.
- Produces: reviewable implementation branch with no legacy reader bypass and a
  reproducible verification record.

- [ ] **Step 1: Run formatting and focused acceptance filters.**

```bash
cargo fmt --all -- --check
cargo test -p unica-coder support_state -- --test-threads=1
cargo test -p unica-coder answers_identically -- --test-threads=1
cargo test -p unica-coder role_info_reads_the_support_state_from_the_configuration_root -- --test-threads=1
cargo test -p unica-coder infrastructure::application_ports::tests -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 2: Run the complete crate suite and lint.**

```bash
cargo test -p unica-coder -- --test-threads=1
cargo clippy -p unica-coder --all-targets -- -D warnings
```

Expected: PASS with zero failures/warnings. Если новый тест падает, применить
`superpowers:systematic-debugging`, сначала установить причину и добавить/сузить
RED, затем править production code.

- [ ] **Step 3: Run repository contract checks.**

```bash
python3.12 -m unittest \
  tests.ci.test_design_documents \
  tests.ci.test_architecture_registry \
  tests.ci.test_rust_platform_boundary \
  tests.ci.test_product_contracts
python3.12 scripts/ci/check-rust-platform-boundary.py
git diff --check origin/main...HEAD
git status --short --branch
```

Expected: PASS; tree clean except intentional uncommitted corrections, which
must receive their own tested commit before handoff.

- [ ] **Step 4: Prove the issue-specific negative space.**

```bash
rg -n "object_support_state|support_state_data|support_status_for_path" \
  crates/unica-coder/src/infrastructure/native_operations \
  crates/unica-coder/src/infrastructure/metadata_operations.rs
rg -n "support_reader\.(configuration_support|object_support)" \
  crates/unica-coder/src/infrastructure
```

Expected: first command exits 1 with no matches; second names the seven intended
readers. `unica.support.edit` and support guard still have their separate legacy
parser path, as ADR-0054 requires.

- [ ] **Step 5: Review the final branch before publishing.**

Use `superpowers:requesting-code-review`, inspect:

```bash
git log --oneline --decorate origin/main..HEAD
git diff --stat origin/main...HEAD
git diff origin/main...HEAD -- \
  crates/unica-coder/src/domain/support_state.rs \
  crates/unica-coder/src/infrastructure/support_state.rs \
  crates/unica-coder/src/infrastructure/native_operations/logical_selector.rs \
  spec/architecture/invariants.md \
  spec/decisions/0054-provider-neutral-support-state-reader.md
```

Expected: один связный changeset issue #289, без новой MCP surface, EDT-reader-а,
изменений support-edit или повторного поля DCS. Только после этого переходить к
`superpowers:finishing-a-development-branch` и предложить пользователю commit /
push / draft PR действия, на которые он дал разрешение.
