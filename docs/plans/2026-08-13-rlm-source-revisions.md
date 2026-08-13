# RLM Source Revisions and Trusted Fences Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task by task with the stated verification gates.

**Goal:** Убрать три полных обхода дерева из каждого RLM-backed запроса, сохранив доказуемую свежесть через монотонное поколение, версионированный digest и pre/post fence.

**Architecture:** Один `SourceRevisionService` принадлежит скрытому workspace service для нормализованной пары workspace root + source root. Он держит trusted/untrusted state, in-memory manifest и SHA-256 digest корпуса. Доменные события и файловый watcher изменяют поколение; быстрый platform fence гарантирует доставку предыдущих событий. На macOS backend использует FSEvents `FlushSync`; на остальных платформах первая версия честно сообщает `Unsupported`, и RLM делает full reconcile вместо ложного O(1) fence. `WorkspaceIndexService` и RLM session получают только `SourceRevisionSnapshot` через порт и не читают частное состояние сервиса.

**Tech Stack:** Rust 2021, `sha2`, macOS CoreServices FSEvents через `objc2-core-services 0.3.2`, `objc2-core-foundation 0.3.2`, `dispatch2`, существующий workspace service protocol, platform integration tests.

**PR boundary:** Начать самостоятельную ветку от актуального `origin/main` после слияния ADR-0055. Не базировать на открытом git-grep PR; ADR-0056 и ADR-0057 независимы. Общий progress payload не менять — этот PR только наполняет semantic role состояниями/фазами и detail codes `reconcilingSources`, `buildingIndex`, `updatingIndex`, `executingQuery`, `sourceRevisionUntrusted`.

**Design source:** `docs/design/2026-08-13-rlm-source-revisions-design.md`, `spec/decisions/0056-rlm-source-revision-freshness.md`.

## Инвариант свежести

RLM output публикуется только при цепочке:

```text
pre fence -> Trusted(G, D, V)
ready marker == (G, D, V)
execute RLM
post fence -> Trusted(G, D, V)
```

Любое изменение G/D/V, `Reconciling`, `Untrusted`, overflow, watcher gap, root replacement или unsupported fast fence отбрасывает output. Unsupported backend не означает unavailable RLM: он означает консервативный full reconcile на границе.

## Task 1: Определить revision state machine и детерминированный corpus digest

**Files:**

- Create: `crates/unica-coder/src/domain/source_revision.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Create: `crates/unica-coder/src/infrastructure/source_revision.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/source_roots.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_state.rs`
- Test: `crates/unica-coder/src/domain/source_revision.rs`
- Test: `crates/unica-coder/src/infrastructure/source_revision.rs`
- Test: `crates/unica-coder/src/infrastructure/source_roots.rs`

**Step 1: Написать падающие state/digest tests**

Требовать:

- generation начинается с 1 после первого успешного reconcile и только возрастает;
- snapshot имеет `algorithm="unica-source-sha256-v1"`, generation и 64-hex digest;
- digest зависит от relative path, entry kind и file content, но не от mtime/absolute root;
- одинаковое дерево в двух temp roots даёт одинаковый digest;
- изменение содержимого при сохранённых size/mtime меняет digest;
- rename отличается от delete+не связанного create только финальным manifest, но итоговый digest соответствует финальному корпусу;
- `.build` и другие исключения текущего corpus policy не меняют digest;
- unreadable/reparse entry и symlink на индексируемый RLM-файл переводят
  reconcile в `Untrusted`; symlink вне корпуса пропускается, а symlink-каталог
  не обходится, как и в закреплённом RLM runtime;
- cancellation/deadline оставляет последнее поколение untrusted и не публикует частичный digest;
- рестарт с тем же нормализованным root и digest сохраняет generation, с другим digest увеличивает его;
- отсутствующая, повреждённая или неизвестной версии запись не делает старый ready marker доверенным;
- legacy `source_generation()` тесты продолжают доказывать старое поведение до окончательной миграции.

Run: `cargo test -p unica-coder source_revision -- --test-threads=1`

Expected: FAIL — модулей и SHA corpus scanner ещё нет.

**Step 2: Ввести доменные типы**

```rust
pub const SOURCE_REVISION_ALGORITHM: &str = "unica-source-sha256-v1";

pub struct SourceRevision {
    pub generation: u64,
    pub digest: String,
    pub algorithm: String,
}

pub enum SourceRevisionState {
    Reconciling { generation: u64 },
    Trusted(SourceRevision),
    Untrusted { generation: u64, reason: SourceRevisionTrustLoss },
}

pub enum SourceRevisionTrustLoss {
    Startup,
    WatcherGap,
    Overflow,
    RootChanged,
    UnsupportedFence,
    ReconcileFailed,
}

pub struct SourceRevisionEventBatch {
    pub sequence: u64,
    pub events: Vec<SourcePathEvent>,
}

pub struct SourcePathEvent {
    pub relative_path: PathBuf,
    pub kind: SourcePathEventKind,
}

pub enum SourcePathEventKind {
    Upsert,
    Remove,
}
```

Состояние не сериализуется в публичный search payload целиком: наружу выходят только progress state/phase/detail code и безопасная diagnostics без digest/path.

**Step 3: Выделить corpus policy из старого walker**

Перенести правила depth, directory exclusion и symlink handling из `source_roots::source_generation_until` в общий `SourceCorpusPolicy`. Старую функцию временно реализовать поверх policy, чтобы старые analyzer/RLM тесты не разошлись до миграции.

**Step 4: Реализовать full scanner**

`scan_source_manifest(root, policy, checkpoint)` возвращает отсортированный `BTreeMap<RelativePath, SourceEntryDigest>`. Для regular file SHA-256 считается по content chunks, entry kind и relative path; directory boundaries входят в финальный fold. Абсолютный root, mtime, inode и host-specific metadata не входят.

Финальный digest строится детерминированным fold по manifest. Это O(N) CPU по уже находящимся в памяти 32-byte entry digests при одном incremental событии, но не O(N) filesystem traversal. Если benchmark покажет этот fold значимым, Merkle-map — отдельная оптимизация без изменения wire/ADR.

**Step 5: Реализовать чистую state machine**

`SourceRevisionMachine` принимает `begin_reconcile`, `finish_reconcile`, `apply_entries`, `lose_trust`. Service преобразует `Upsert` в чтение и content digest одного доказанно contained regular file, `Remove` удаляет entry; неизвестный тип, directory batch и ошибка чтения теряют trust. `finish_reconcile` публикует Trusted только если поколение/epoch наблюдателя не менялись во время scan; иначе caller повторяет reconcile или остаётся Untrusted.

**Step 6: Сохранить revision record атомарно**

Приватная запись лежит в `context.cache_root/source-revisions/<identity-sha256>.json`, где identity hash строится из нормализованных workspace root и source root. Schema:

```json
{
  "schemaVersion": 1,
  "workspaceRoot": "/normalized/workspace",
  "sourceRoot": "/normalized/workspace/src",
  "generation": 17,
  "algorithm": "unica-source-sha256-v1",
  "digest": "64-hex"
}
```

Публикация использует тот же атомарный transaction/file-replacement primitive, что workspace state. Startup после стабильного reconcile сравнивает digest с валидной записью: равный сохраняет generation, отличный увеличивает; неизвестная schema/identity/digest не переиспользует ready marker. Incremental trusted update публикует новую запись после обновления manifest. Ошибка публикации переводит state в Untrusted — память процесса не может объявлять поколение устойчивым, если его не сможет восстановить следующий процесс.

**Step 7: Запустить тесты и закоммитить**

Run: `cargo test -p unica-coder source_revision source_roots -- --test-threads=1`

Expected: PASS.

```bash
git add crates/unica-coder/src/domain/source_revision.rs crates/unica-coder/src/domain/mod.rs crates/unica-coder/src/infrastructure/source_revision.rs crates/unica-coder/src/infrastructure/mod.rs crates/unica-coder/src/infrastructure/source_roots.rs crates/unica-coder/src/infrastructure/workspace_state.rs
git commit -m "feat: ввести доверенные ревизии корпуса RLM"
```

## Task 2: Определить платформенную границу fence и консервативный fallback

**Files:**

- Create: `crates/unica-coder/src/infrastructure/platform/source_revision_fence.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform/mod.rs`
- Test: `crates/unica-coder/src/infrastructure/platform/source_revision_fence.rs`
- Test: `tests/ci/test_rust_platform_boundary.py`

**Step 1: Написать падающие capability tests**

Fake backend должен доказать:

- `flush()` возвращает `Proven` только после применения всех callback batches до barrier;
- overflow/root change/gap возвращают typed trust loss;
- unsupported backend никогда не возвращает Proven;
- deadline/cancellation interrupt fence;
- failure не возвращает последнее trusted snapshot как fallback.

Run: `cargo test -p unica-coder source_revision_fence -- --test-threads=1`

Expected: FAIL — platform seam отсутствует.

**Step 2: Ввести узкий trait**

```rust
pub trait SourceRevisionFence: Send + Sync {
    fn capability(&self) -> FenceCapability;
    fn flush(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<FenceOutcome, String>;
}

pub trait SourceRevisionEventSink: Send + Sync {
    fn apply_batch(&self, batch: SourceRevisionEventBatch);
    fn lose_trust(&self, reason: SourceRevisionTrustLoss);
}

pub enum FenceCapability {
    ProvenFast,
    Unsupported,
}

pub enum FenceOutcome {
    Proven,
    TrustLost(SourceRevisionTrustLoss),
}
```

Platform module не знает RLM, index marker и search. Он только владеет watcher lifetime/barrier и отправляет нормализованные batches через `SourceRevisionEventSink`. Backend удерживает `Weak<dyn SourceRevisionEventSink>`, а не `Arc<SourceRevisionService>`: service владеет backend, поэтому сильная обратная ссылка создала бы цикл и удержала workspace root после завершения runtime.

**Step 3: Реализовать non-macOS backend**

На `cfg(not(target_os = "macos"))` вернуть `FenceCapability::Unsupported`; `flush` не притворяется барьером. Добавить test-only fake backend для state/service tests на любой CI OS.

**Step 4: Проверить platform boundary и закоммитить**

Run: `python3.12 -m unittest tests.ci.test_rust_platform_boundary`

Run: `cargo test -p unica-coder source_revision_fence -- --test-threads=1`

Expected: PASS.

```bash
git add crates/unica-coder/src/infrastructure/platform/source_revision_fence.rs crates/unica-coder/src/infrastructure/platform/mod.rs tests/ci/test_rust_platform_boundary.py
git commit -m "refactor: выделить платформенный fence ревизии"
```

## Task 3: Реализовать доказуемый macOS FSEvents fence

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/unica-coder/Cargo.toml`
- Create: `crates/unica-coder/src/infrastructure/platform/source_revision_fence/macos.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform/source_revision_fence.rs`
- Test: `crates/unica-coder/src/infrastructure/platform/source_revision_fence/macos.rs`
- Test: `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`

**Step 1: Добавить macOS-only зависимости**

В workspace dependencies:

```toml
objc2-core-services = { version = "0.3.2", default-features = false, features = ["std", "FSEvents", "dispatch2"] }
objc2-core-foundation = { version = "0.3.2", default-features = false, features = ["std", "CFArray", "CFString"] }
dispatch2 = { version = "0.3", default-features = false, features = ["alloc"] }
```

Подключить их в `crates/unica-coder/Cargo.toml` только под `target.'cfg(target_os = "macos")'.dependencies`. После `cargo check` зафиксировать реально разрешённые версии в `Cargo.lock`; не включать default feature set CoreServices.

**Step 2: Написать падающий macOS integration test**

На APFS temp root:

1. запустить watcher;
2. выполнить стабильный initial reconcile;
3. записать BSL file внешним `std::fs::write`;
4. вызвать `flush`;
5. проверить, что generation вырос и digest соответствует полному rescan;
6. удалить/rename file и повторить;
7. drop backend завершает stream/queue thread и не удерживает temp root.

Отдельная capability-проверка с injected filesystem classifier требует `ProvenFast` только для APFS и `Unsupported` для network/unknown values.

Тест пометить `#[cfg(target_os = "macos")]`; он должен укладываться в секунды и не использовать sleep как доказательство доставки.

Run on macOS: `cargo test -p unica-coder macos_fsevents_fence -- --test-threads=1`

Expected: FAIL — backend отсутствует.

**Step 3: Создать stream до initial reconcile**

До создания stream backend проверяет filesystem type корня через macOS `statfs`. В первой версии `ProvenFast` разрешён только для локального APFS, на котором выполняется integration evidence; неизвестный, сетевой и иной filesystem возвращает `Unsupported` и идёт через full reconcile. Сам факт `target_os=macos` недостаточен для обещания fence.

FSEvents stream создаётся с:

```text
kFSEventStreamCreateFlagFileEvents
kFSEventStreamCreateFlagWatchRoot
kFSEventStreamCreateFlagNoDefer
kFSEventStreamCreateFlagUseCFTypes
```

и `since_when = kFSEventStreamEventIdSinceNow`. Callback context владеет `Arc<Mutex<WatcherState>>`; stream получает dedicated serial dispatch queue. Callback нормализует только пути внутри source root и передаёт batch через слабую ссылку `SourceRevisionEventSink`.

Unsafe wrapper обязан:

- удерживать callback context дольше stream;
- на Drop выполнить `Stop`, `Invalidate`, `SetDispatchQueue(None)`, `Release` в этом порядке;
- не panic-овать через FFI boundary;
- переводить malformed callback data в watcher gap/Untrusted.

**Step 4: Обработать trust-loss flags**

Любой из flags:

```text
MustScanSubDirs
UserDropped
KernelDropped
EventIdsWrapped
RootChanged
```

не применяется incrementally; state получает соответствующий `TrustLost`. Directory/coalesced event без достаточного file-level evidence также вызывает reconcile.

**Step 5: Реализовать barrier**

`flush` вызывает `FSEventStreamFlushSync`. Возврат из него считается Proven только если callback state не отметил gap и service применил все batches. Затем service читает snapshot под тем же synchronization protocol. Не заменять `FlushSync` ожиданием latency/sleep.

**Step 6: Закрыть race initial reconcile**

Порядок startup:

```text
start stream
FlushSync
capture watcher epoch
full scan
FlushSync
publish Trusted only if watcher epoch unchanged
```

Если epoch изменился, применить накопленные precise events к manifest либо повторить full scan до deadline. Частичный manifest trusted не становится.

**Step 7: Запустить macOS и cross-platform checks, затем закоммитить**

Run on macOS: `cargo test -p unica-coder macos_fsevents_fence issue_89_workspace_service -- --test-threads=1`

Run: `cargo check -p unica-coder --all-targets`

Run: `python3.12 -m unittest tests.ci.test_rust_platform_boundary tests.ci.test_classify_workflow_changes`

Expected: PASS; classifier обязан направить новый platform file в трёхплатформенный CI contour и search integration job.

```bash
git add Cargo.toml Cargo.lock crates/unica-coder/Cargo.toml crates/unica-coder/src/infrastructure/platform/source_revision_fence.rs crates/unica-coder/src/infrastructure/platform/source_revision_fence/macos.rs crates/unica-coder/tests/platform/issue_89_workspace_service.rs
git commit -m "feat: доказать ревизию через macOS FSEvents fence"
```

## Task 4: Подключить SourceRevisionService к workspace service и доменным событиям

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/source_revision.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Modify: `crates/unica-coder/src/domain/events.rs`
- Modify: `crates/unica-coder/src/domain/cache.rs`
- Test: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Test: `crates/unica-coder/src/domain/cache.rs`
- Test: `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`

**Step 1: Написать падающие ownership/isolation tests**

Требовать:

- одна нормализованная workspace+source pair создаёт один service/watcher;
- разные source roots и worktrees не делят generation;
- повторный warm call переиспользует service;
- Drop runtime завершает watcher;
- exact `SourceResourcesReplaced` обновляет/инвалидирует нужные entries;
- broad `SourceSetChanged` и неполный artifact переводят state в Untrusted;
- дубликат одного изменения из domain event и watcher может увеличить generation ещё раз, но не способен сохранить старую ready marker свежей;
- analyzer generation state пока не меняется этим PR.

Run: `cargo test -p unica-coder workspace_services source_revision -- --test-threads=1`

Expected: FAIL — runtime хранит два `Mutex<Option<u64>>` и invalidated bool.

**Step 2: Добавить service в runtime**

`WorkspaceServiceRuntime` получает:

```rust
source_revisions: Arc<SourceRevisionService>
```

Он создаётся после доказанной workspace/source identity и до первого RLM readiness. `analyzer_source_generation` остаётся отдельно до собственного решения; `rlm_source_generation` и `rlm_invalidated` удаляются после миграции call sites.

**Step 3: Провести доменные события**

`WorkspaceServiceRuntime::invalidate(events)` передаёт RLM-relevant events в service. Exact resource replacement использует sourceSet/affected targets как evidence; события без достаточного относительного набора путей вызывают `lose_trust`, а не искусственное generation-only fresh state.

**Step 4: Публиковать безопасное состояние semantic provider**

При reconcile semantic role получает `state=running`, `phase=preparing`, `detailCode=reconcilingSources`; обслуживание индекса — `buildingIndex`/`updatingIndex`; execute — `phase=searching`, `detailCode=executingQuery`; при trust loss — `state=failed`, `detailCode=sourceRevisionUntrusted` без digest и физических paths. Progress sink подключён через ADR-0055 seam; workspace service не знает MCP token.

**Step 5: Запустить тесты и закоммитить**

Run: `cargo test -p unica-coder workspace_services source_revision domain::cache -- --test-threads=1`

Run: `cargo test -p unica-coder --test issue_89_workspace_service -- --test-threads=1`

Expected: PASS.

```bash
git add crates/unica-coder/src/infrastructure/source_revision.rs crates/unica-coder/src/infrastructure/workspace_services.rs crates/unica-coder/src/domain/events.rs crates/unica-coder/src/domain/cache.rs crates/unica-coder/tests/platform/issue_89_workspace_service.rs
git commit -m "feat: владеть ревизией в workspace service"
```

## Task 5: Привязать RLM ready marker и index jobs к полной ревизии

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Test: `crates/unica-coder/src/infrastructure/workspace_index.rs`
- Test: `crates/unica-coder/src/infrastructure/workspace_services.rs`

**Step 1: Написать падающие marker/job tests**

Проверить:

- legacy marker только с `source_generation` читается, но считается stale;
- новый ready marker содержит `indexedRevision {generation,digest,algorithm}`;
- build/update захватывает Trusted revision до запуска;
- изменение generation или trust loss до публикации не пишет ready;
- marker с тем же generation, но другим digest/algorithm stale;
- terminal failure относится к exact revision и не благословляет следующую;
- background job не вызывает `source_generation()` самостоятельно.

Run: `cargo test -p unica-coder workspace_index -- --test-threads=1`

Expected: FAIL — marker хранит только `Option<u64>`.

**Step 2: Мигрировать status schema**

Добавить:

```rust
pub struct IndexedSourceRevision {
    pub generation: u64,
    pub digest: String,
    pub algorithm: String,
}
```

`BslIndexStatus` временно читает legacy `source_generation`, но новые записи публикуют только `indexed_revision`. Legacy ready marker всегда запускает update. Удаление legacy read выполняется отдельной будущей миграцией после одного release window.

**Step 3: Ввести revision port**

```rust
pub trait SourceRevisionPort: Send + Sync {
    fn trusted_snapshot(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<SourceRevision, String>;
}
```

`WorkspaceIndexService` получает port конструктором. `IndexBackgroundJob` несёт captured revision. Никакой index code не вызывает tree walker.

**Step 4: Fence publication**

Перед записью ready background job просит snapshot снова. Публикация разрешена только при полном equality captured/current revision. Иначе status остаётся stale/build-requested и progress сообщает изменение во время build.

**Step 5: Запустить тесты и закоммитить**

Run: `cargo test -p unica-coder workspace_index workspace_services -- --test-threads=1`

Expected: PASS.

```bash
git add crates/unica-coder/src/infrastructure/workspace_index.rs crates/unica-coder/src/infrastructure/workspace_services.rs
git commit -m "feat: привязать индекс RLM к доверенной ревизии"
```

## Task 6: Заменить три full walks в RLM execution на pre/post fence

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Modify: `crates/unica-coder/src/infrastructure/code_intelligence.rs`
- Test: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Test: `crates/unica-coder/src/infrastructure/code_intelligence.rs`
- Test: `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`

**Step 1: Написать падающие execution-boundary tests**

С fake revision service/fence проверить:

1. Trusted G + matching marker + post Trusted G публикует output.
2. G меняется во время execute — output discarded, section incomplete/unavailable.
3. Post fence теряет trust — output discarded.
4. Pre unsupported вызывает full reconcile; после него запрос может выполниться.
5. Warm ProvenFast request вызывает 2 flush и 0 full scanner invocations.
6. Readiness/build path не добавляет скрытый третий scan.
7. Cancellation между execute и post fence не публикует output.
8. Session invalidation ключуется той же full revision, не отдельным counter/TTL.

Run: `cargo test -p unica-coder workspace_services rlm_provider -- --test-threads=1`

Expected: FAIL — `handle_rlm_mcp` вызывает `source_generation_with_deadline` pre/post.

**Step 2: Ввести один helper trusted boundary**

```rust
fn trusted_revision_at_boundary(
    revisions: &SourceRevisionService,
    deadline: &ElapsedBudgetDeadline,
    cancellation: &CancellationToken,
) -> Result<SourceRevision, ServiceResponse>
```

Он:

- на ProvenFast делает fence + snapshot;
- на Unsupported/TrustLost делает bounded full reconcile + snapshot;
- никогда не возвращает last-known revision после failure;
- использует остаток caller deadline без 120s cap.

**Step 3: Переписать `handle_rlm_mcp`**

Удалить pre/post `source_generation_with_deadline`, `observe_source_generation` и RLM invalidated bool. Сравнивать полные revision snapshots. Проверять matching ready marker до execute и equality после execute. Любой stale output уничтожается до `ServiceResponse`.

**Step 4: Привязать RLM session**

`RlmMcpSession` хранит revision, на которой создан. При другой current revision transport/session инвалидируется до следующего execute. TTL остаётся lifecycle budget, но не доказательством свежести.

**Step 5: Удалить старый RLM generation path**

После миграции всех RLM call sites удалить `rlm_source_generation`, `rlm_invalidated` и RLM-вызовы `source_generation_with_deadline`. Функцию оставить только если она ещё нужна analyzer; переименовать в analyzer-specific, чтобы новый RLM код не мог случайно вернуться к ней.

**Step 6: Запустить тесты и закоммитить**

Run: `cargo test -p unica-coder workspace_services rlm_provider -- --test-threads=1`

Run: `cargo test -p unica-coder --test issue_89_workspace_service -- --test-threads=1`

Expected: PASS, включая assertion `warm_scan_count == 0`.

```bash
git add crates/unica-coder/src/infrastructure/workspace_services.rs crates/unica-coder/src/infrastructure/code_intelligence.rs crates/unica-coder/tests/platform/issue_89_workspace_service.rs
git commit -m "perf: заменить обходы RLM на trusted fence"
```

## Task 7: Обновить архитектурный контракт, progress и эксплуатационные доказательства

**Files:**

- Modify: `spec/decisions/0056-rlm-source-revision-freshness.md`
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/architecture/quality-requirements.md`
- Modify: `spec/architecture/runtime.md`
- Modify: `spec/architecture/building-blocks.md`
- Modify: `spec/architecture/risks.md`
- Modify: `plugins/unica/skills/code-search/SKILL.md`
- Modify: `tests/ci/test_architecture_registry.py`
- Modify: `tests/ci/test_product_contracts.py`
- Modify: `tests/ci/test_unica_skills.py`

**Step 1: Написать падающие doc-contract tests**

Проверки должны находить:

```text
SourceRevisionService
workspaceRoot + sourceRoot
generation
unica-source-sha256-v1
pre/post fence
FSEventStreamFlushSync
Unsupported -> full reconcile
zero warm full scans
indexedRevision
```

Run: `python3.12 -m unittest tests.ci.test_architecture_registry tests.ci.test_product_contracts tests.ci.test_unica_skills`

Expected: FAIL до синхронизации docs.

**Step 2: Обновить владельцев правил**

Добавить инвариант `INV-CACHE-SOURCE-REVISION`: один владелец revision на identity, provider получает snapshot, trust loss не использует last known value. Добавить `REQ-PERF-RLM-WARM-FENCE`: на ProvenFast backend warm request делает два fence и ноль full corpus scans. `REQ-PERF-WARM-REUSE` ссылается на новый измеримый контракт, но не дублирует его.

В risk registry явно оставить:

- холодный SHA content reconcile O(total bytes);
- non-macOS первая версия консервативна и не обещает ускорения;
- APFS/macOS evidence не переносится автоматически на network/unsupported filesystem.

**Step 3: Обновить skill**

Модель должна понимать `phase=preparing` с detail codes `reconcilingSources`/`buildingIndex`/`updatingIndex` и `phase=searching` с `executingQuery`; отсутствие процента при reconcile нормально. Не советовать отдельный polling tool. Digest/generation наружу не показываются.

**Step 4: Перевести ADR-0056 в accepted и закоммитить**

Только после зелёных state/index/execution/platform tests.

```bash
git add spec/decisions/0056-rlm-source-revision-freshness.md spec/architecture/invariants.md spec/architecture/quality-requirements.md spec/architecture/runtime.md spec/architecture/building-blocks.md spec/architecture/risks.md plugins/unica/skills/code-search/SKILL.md tests/ci/test_architecture_registry.py tests/ci/test_product_contracts.py tests/ci/test_unica_skills.py
git commit -m "docs: принять доверенные поколения RLM"
```

## Task 8: Измерить cold/warm и выполнить полную верификацию

**Files:**

- Create: `crates/unica-coder/tests/platform/source_revision_performance.rs`
- Modify: `crates/unica-coder/tests/platform_code_intelligence.rs`
- Verify: whole workspace and CI contracts.

**Step 1: Добавить детерминированный performance contract**

Тест строит temp corpus с тысячами файлов через fixture generator и instrumented scanner/fence. Он не утверждает wall-clock threshold на shared CI; он утверждает работу:

```text
cold: full_scans = 1, file_reads = corpus_files
warm: full_scans = 0, file_reads = 0, fences = 2
external edit: incremental_reads = changed_files, then fences = 2
overflow: full_scans = 1 before accepted output
```

Отдельно локально вывести cold/warm elapsed как diagnostic, но не делать нестабильным pass/fail.

Run: `cargo test -p unica-coder --test platform_code_intelligence source_revision_performance -- --test-threads=1`

Expected: PASS.

**Step 2: Формат, lint, полные тесты**

Run: `cargo fmt --all -- --check`

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`

Run: `cargo test --workspace -- --test-threads=1`

Run: `python3.12 -m unittest discover -s tests/ci --durations 20`

Expected: PASS.

**Step 3: Архитектурные и platform guards**

Run: `python3.12 scripts/ci/check-rust-platform-boundary.py`

Run: `python3.12 scripts/ci/check-architecture-sync.py --base origin/main --strict`

Run: `git diff --check origin/main...HEAD`

Expected: PASS.

**Step 4: Реальный BSP assessment**

Поскольку classifier относит revision/index/service paths к `search_integration_changed`, PR обязан дождаться GitHub job `Assess Linux runtime on BSP` либо явно запустить workflow с `ci:full`. На Linux ожидается консервативный reconcile path; macOS FSEvents correctness доказывает matrix integration test. Сохранить в PR:

- cold index duration;
- first search duration;
- second warm search duration;
- progress state/phase/detail codes;
- semantic terminal status;
- отсутствие warm full scans из deterministic test.

**Step 5: Перебазировать только на main и открыть PR**

Если common search contract ещё не в `main`, остановиться. Git-grep PR может быть открыт или слит — зависимости на него нет. После rebase повторить Steps 2–3. Открыть самостоятельный PR с ADR-0056, measured evidence, macOS/non-macOS distinction и явным указанием, что ускорение первой версии гарантировано только backend с ProvenFast fence.
