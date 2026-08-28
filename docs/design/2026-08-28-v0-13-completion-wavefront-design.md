- Date: `2026-08-28`
- Status: `approved`
- Decision: `DEC.2026-08-28.V0-13-TASK-OBSERVATION-SLICE`

# Завершение миграции v0.13 через foundation-first wavefront

## Назначение

Основная часть записки меняет способ завершения миграции, а не выбранную
композицию продукта. Она заменяет последовательное прочтение Tasks 1–24 из
`docs/plans/2026-08-23-v0-12-3-to-v0-13-migration.md` на короткий критический
путь, три непересекающихся исполнительских потока и отдельные release gates.

Ревью выявило одну ранее оставленную открытой продуктовую границу: обязательную
модель наблюдения durable Task в v0.13. Её выбор принадлежит planned
`DEC.2026-08-28.V0-13-TASK-OBSERVATION-SLICE`. Реализация получит действующее
основание только через новый active successor/evidence `DEC.*`, который при G6
сошлётся на именованную проверку, supersede эту planned-запись и изменит
производный контракт. Принятое product decision после merge не переписывается;
эта записка не выдаёт планируемое поведение за текущее.

Остальные продуктовые правила остаются во владении существующих `DEC.*`,
`INV.*` и `CTR.*`. Если в ходе W0 потребуется изменить другое правило, работа
останавливается до появления successor `DEC.*` и выведенной записи
`INV.*`/`CTR.*`.

Пользователь одобрил вариант foundation-first fan-out 28 августа 2026 года.

## Снимок исходного состояния

Исходный архитектурный снимок был снят с PR #631 на
`e143ba02ad0baf7caaaf1f036c96e5dad2dd8edc`. Это baseline проектирования, а не
указатель на текущий head: изменяемые W0 evidence и SHA ведутся в PR #631 и
issue #581. На исходном снимке:

- PR остаётся draft и blocked;
- production-конструктор stdio выбирает `SurfaceRelease::V12`;
- hidden V13 catalog и два профиля `tools/list` существуют;
- daemon, Invocation, durable Task, WorkspaceActor и SharedWork существенно
  реализованы, но production daemon по умолчанию устанавливает dormant service;
- реальный canonical service обслуживает `view` и `find`, а остальные шесть
  входов ещё не образуют production vertical slices;
- `apply` имеет общую модель, code/event planners и развиваемый XDTO slice, но
  не имеет закрытого набора из 96 подключённых операций;
- опубликованный baseline v0.12.3 содержит 74 имени, тогда как текущая ведомость
  `main` содержит 71 имя;
- пакет содержит 73 отслеживаемых `plugins/unica/skills/*/SKILL.md`, которые
  должны перейти на новую поверхность атомарно с публичным cutover;
- bootstrap и core-first delivery уже являются действующим фундаментом, но
  bootstrap verification всё ещё проверяет старые публичные инструменты.

Количество изменённых строк PR не является мерой готовности. До тех пор пока
production stdio создаёт V12 application, публичная миграция не состоялась.

## Применяемые архитектурные решения

### Daemon и Invocation

Wavefront применяет, но не переопределяет:

- `DEC.2026-08-23.USER-CORE-DAEMON-SLICE`;
- `DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE`;
- `INV.APP.DAEMON-INVOCATION-HANDOFF`;
- `INV.APP.DAEMON-INVOCATION-OWNERSHIP`;
- `INV.APP.DAEMON-ACTOR-AUTHORITY`;
- `INV.APP.DAEMON-ACTOR-CAPACITY`;
- `INV.APP.DAEMON-TASK-PERSISTENCE`;
- `INV.APP.DAEMON-TASK-RECOVERY`;
- `INV.APP.DAEMON-TERMINAL-RECONCILIATION`.

Начиная с состояния S3 один публичный локальный stdio MCP `unica` становится
thin frontend. Скрытый versioned daemon владеет Invocation, TaskStore,
WorkspaceActor и долгой работой. V13 frontend не исполняет предметную операцию,
не повторяет её после handoff и не делает fallback на V12. До G6 production
V12 продолжает использовать действующий legacy handler.

### Запуск и поставка зависимостей

Wavefront сохраняет bootstrap-first цепочку и применяет:

- `DEC.2026-08-19.CORE-FIRST-ACQUISITION`;
- `DEC.2026-08-20.ENGINES-COME-FROM-THE-TOOLCHAIN`;
- `DEC.2026-08-20.PREFETCH-FILLS-THE-CLOSED-CONTOUR`;
- `DEC.2026-08-24.EXACT-SHARED-DELIVERY-SLICE`;
- `INV.APP.EXACT-SHARED-WORK`;
- `CTR.APP.EXACT-SHARED-DELIVERY`.

Публичный пакет запускает native bootstrap, bootstrap проверяет и запускает
core, а engines доставляются лениво по точной immutable identity. `prefetch`
наполняет полный offline-контур. Поставка не становится публичной операцией
`run` и не создаёт `tools-download`.

Local stdio остаётся выбранной deployment model: Unica работает с локальными
файлами, процессами, платформой 1С и пользовательским состоянием. Переход на
remote MCP, MCP App или дополнительный MCPB-контур не входит в миграцию v0.13.

### Предметная поверхность

Целью остаётся `DEC.2026-08-23.V0-13-EXECUTION-SURFACE`:

- native Tasks profile: ровно `view`, `apply`, `find`, `search`, `check`,
  `diff`, `run`, `docs`;
- compatibility profile: те же восемь плюс `task.get`, `task.result`,
  `task.cancel`;
- старые публичные имена и `runtime.job.*` не сохраняются как aliases;
- `tests`, `features` и `log` остаются вне v0.13.

Восемь entry points используют один canonical result и один daemon dispatcher.
Словари операций `apply` и `run` являются данными, а не расширением
`tools/list`.

### Tasks

Wavefront применяет:

- `DEC.2026-08-24.NATIVE-TASK-PROJECTION-SLICE`;
- `DEC.2026-08-24.COMPATIBILITY-TASK-TOOLS-SLICE`;
- `DEC.2026-08-24.LONG-WORK-OWNERSHIP-SLICE`;
- `INV.APP.EXACT-LONG-WORK-OWNERSHIP`;
- `INV.APP.RUNTIME-RESOURCE-TREE`;
- `INV.WIRE.NATIVE-TASK-CAPABILITY`;
- `INV.WIRE.V13-TASK-PROFILES`;
- `CTR.APP.DAEMON-LONG-WORK-CAPABILITIES`;
- `CTR.WIRE.NATIVE-TASK-PROJECTION`;
- `CTR.WIRE.COMPATIBILITY-TASK-TOOLS`;
- `CTR.WIRE.DAEMON-INVOCATION-PROTOCOL`.

Каждый предметный вызов имеет ровно одно исполнение. Сервер сам выбирает direct
result или Task: known-long materializes Task сразу, остальные операции обязаны
либо завершиться напрямую, либо иметь durable receipt к абсолютной границе
7000 мс. Native и compatibility projections читают одну Invocation.

Planned `DEC.2026-08-28.V0-13-TASK-OBSERVATION-SLICE` ограничивает целевое
наблюдение прогресса после receipt изменением status и `updatedAt`, доступным
через polling. Действующие envelope, TTL, poll interval и terminal
result/failure сохраняются. Общий resume protocol и новый публичный progress
API не создаются. Invocation без доказанного resume owner после restart
получает закрытый terminal outcome и не переисполняется. До появления
реализации и именованной проверки действующий
`CTR.APP.DAEMON-LONG-WORK-CAPABILITIES` продолжает честно называть progress
открытой границей.

### Retained apply

Транзакционные stop conditions не вводятся этой запиской. Ими владеют:

- `DEC.2026-08-26.RETAINED-APPLY-TRANSACTION-FOUNDATION-SLICE`;
- `INV.APP.RETAINED-APPLY-CLOSED-PARTICIPANTS`;
- `INV.CACHE.RETAINED-APPLY-REVISION-ROLLBACK`;
- `INV.CACHE.RETAINED-APPLY-DETERMINISTIC-ORDER`;
- `INV.SOURCE.RETAINED-APPLY-WRITE-FREE`.

## Обнаруженные противоречия

### Daemon protocol v2 против v3

`DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE` называет protocol v2, тогда как
`DEC.2026-08-24.NATIVE-TASK-PROJECTION-SLICE` и active
`CTR.WIRE.DAEMON-INVOCATION-PROTOCOL` требуют v3 и
`unica-daemon-jsonl-3`.

W0 обязан оставить в реестре одну текущую wire identity. План не разрешает
сохранить обе формулировки под видом compatibility.

### 71 имя против опубликованных 74

Удаление только 71 имени текущего `main` не доказывает миграцию опубликованной
v0.12.3. Release acceptance использует immutable fixture с 74 именами и явно
проверяет судьбу шести `unica.runtime.job.*`.

### Реализованный фундамент против production readiness

Hidden V13 tests доказывают отдельные slices, но production stdio остаётся V12,
а default daemon остаётся dormant. Поэтому Tasks 1–14 старого плана после W0
становятся frozen foundation baseline, но не доказательством публичного
cutover.

### Release-переключение внутри mega-PR

Стабильная версия не должна включаться внутри PR #631. Release runbook требует
отдельный version-bump PR, а настоящий prerelease требует опубликованных
immutable RC assets. Поэтому hidden foundation и public version cutover имеют
разные gates.

## Рассмотренные организационные варианты

### Один mega-PR до самого cutover

Этот вариант быстрее создаёт следующий коммит, но оставляет один огромный
review/CI tail, заставляет независимых агентов одновременно менять central
hotspots и связывает исправления foundation с release-переключением.

### Foundation-first fan-out

Выбранный вариант:

1. стабилизировать PR #631 как hidden V13 foundation;
2. слить foundation без изменения публичной V12 поверхности;
3. из нового `main` вести независимые PR по disjoint domain slices;
4. выполнять общую интеграцию одним владельцем central hotspots;
5. сделать RC cutover отдельным последовательным PR.

Worker branches могут быть подготовлены от accepted foundation SHA до merge,
но открывать stacked PR с базой на head #631 запрещено. После merge каждый PR
создаётся от актуального `main`.

### Перестроить V13 заново от main

Вариант отвергнут: он теряет уже проверенные daemon, Task, actor, read и
delivery slices, не уменьшая объём предметных handlers и release acceptance.

## Целевая композиция

```text
Codex / Claude Code
        |
        v
one public local stdio MCP `unica`
        |
        v
native bootstrap
  |-- verified versioned core cache
  |-- lazy exact engine delivery
  `-- offline prefetch closure
        |
        v
thin stdio frontend
        |
        v
private per-user, protocol/core-ABI daemon
  |-- durable Invocation / Task store
  |-- WorkspaceActor per authenticated source profile
  |-- exact SharedWork for delivery/index/provider/runtime
  `-- one canonical dispatcher with 8 domain handlers
        |
        +-- native Tasks profile: 8 tools
        `-- compatibility profile: 11 tools
```

## Переход состояний

| Состояние | Наблюдаемая граница |
| --- | --- |
| S0 | Публичный V12, hidden неполный V13, PR #631 blocked |
| S1 | Зелёный и слитый hidden foundation, публичный V12 неизменён |
| S2 | Все 8 handlers и принятый registry `apply` работают через daemon, parity закрыта |
| S3 | RC публикует ровно 8/11, legacy surface отсутствует |
| S4 | Stable v0.13 прошёл fresh/upgrade/rollback/offline и host matrix |

Ни S1, ни S2 не являются пользовательским релизом. Пользовательская граница
меняется только S2 → S3.

## Wavefront

### W0: stabilize and bound

PR #631 получает зелёные macOS, Linux и Windows gates. В этой волне закрываются
текущие defect failures по TDD, устраняется v2/v3 contradiction, фиксируется
internal SPI отдельных family planners, validation и canonical result.
Request-level apply router не считается замороженным, пока W2a не докажет
глобальные индексы, порядок и effects от финального postimage. Публичный V12 не
меняется.

### W2a: request router и ранние seams

Сразу после S1 integrator закрывает shared seam до первого W1 merge. Router
парсит request один раз, сохраняет исходный `ops[i]`, передаёт XDTO, Code и
Event только через их admission-sealed authorities и выводит domain events из
финального postimage всего request, а не суммирует промежуточные singleton
результаты. Здесь же компилируются стабильные W3 seams. Только после aggregate
тестов на inverse operations, interleaved families, global error index и poison
rollback family SPI считается frozen для fan-out.

Effect finalizer сначала отбрасывает path-bound candidates без изменения в
финальном postimage и только затем выполняет stable first-surviving-occurrence
dedup по `DEC.2026-08-26.RETAINED-APPLY-EFFECT-PUBLICATION-SLICE`. Обратный
порядок ошибочен: transient первый duplicate не должен поглотить surviving
второй.

### W1: закрыть измеренный registry apply

Текущий implementation inventory содержит 96 уникальных операций. Три workers
параллельно реализуют его незакрытые группы: 34 metadata/properties, 23
form/role/subsystem/support/XDTO и 36 DCS/MXL operations. Уже принятые две code
operations и одна event operation не переписываются. Число 96 является
датированным измерением кода, а не новым публичным архитектурным лимитом. Если
parity inventory обнаружит пропуск, сначала исправляются fixture и этот план.
Каждый slice несёт writer parity fixture, RED/GREEN staged transaction proof и
отдельное ревью.

### W2b: dispatcher and daemon apply

Integrator использует уже существующий `OperationRegistry::closed()` как один
implementation source для `view.can[]`, parse и dispatch, подключает family
planners к WorkspaceActor и устанавливает real `apply` handler в canonical
daemon service. W2b выполняется по мере поступления W1 slices и не добавляется к
календарю отдельной последовательной фазой. Он расширяет уже доказанный W2a
router, а не возвращает singleton dispatch.

### W3: remaining entry points

После завершения собственной apply-линии каждый worker сразу переходит к
`search` + `docs`, `check` + `diff` или `run`, не ожидая две другие линии.
Исключение — B8 (`apply(dryRun)` parity): он ждёт финальную интеграцию apply в
W2b. Integrator один регистрирует handlers в daemon и MCP shared files.

### W4: continuous parity and skills

Parity matrix создаётся в W0 и заполняется каждым vertical slice. В конце
остаётся aggregate gate, а не поздняя отдельная wiring task. Параллельно три
workers готовят migration mapping, fixtures и непубликуемые patch series для 73
skills. Владелец каждого skill фиксируется в manifest; исходное распределение
строится детерминированным LPT по размеру отслеживаемого `SKILL.md`, а не по
неравным буквенным диапазонам. Эти patches не сливаются в ветку с
package-selected V12 и применяются только внутри atomic G6.

### W5: hidden V13 integration readiness

Hidden V13 проверяется существующими injected in-process и daemon wire
harnesses; новый CLI, environment switch или package mode для этого не
создаётся. W5 также доказывает неизменность действующего bootstrap/core-first
контура и готовит RED package acceptance для RC. Реальный packaged V13,
cold-list/engine Task, offline prefetch, cache isolation и rollback проверяются
после package-selected cutover внутри G6. Package scripts меняются только в
ответ на падающий acceptance test.

### G6: atomic RC cutover

Отдельный PR переключает package-selected surface на `0.13.0-rc.1`, включает
production daemon routing, применяет подготовленные skill patches, удаляет
legacy registrations/job schemas и вместе обновляет bootstrap verification,
manifests и architecture evidence. В том же PR staged RC package доказывает
cold list без engine, одну Task/одну delivery, offline prefetch, cache isolation
и rollback. RC tag и prerelease остаются owner-approved действиями.

### G7: stable release

После host matrix отдельный version-only PR поднимает `0.13.0`; затем release
pipeline строит immutable assets, выполняет stage, fresh/upgrade probes и
promote в marketplace.

## Multi-agent ownership

Используются четыре active slots: integrator и три workers.

Integrator единолично владеет:

- `crates/unica-coder/src/application/v13/mod.rs` и общими V13 facade files;
- `crates/unica-coder/src/domain/apply.rs` и common validation/result interfaces;
- `crates/unica-coder/src/infrastructure/native_operations/apply.rs` и
  корневым dispatcher registry;
- `crates/unica-coder/src/infrastructure/workspace_actor.rs`;
- `crates/unica-coder/src/infrastructure/daemon/mod.rs`;
- `crates/unica-coder/src/infrastructure/daemon/server.rs`;
- `crates/unica-coder/src/infrastructure/daemon/v13_service.rs`;
- `crates/unica-coder/src/interfaces/mcp.rs`;
- `crates/unica-coder/src/interfaces/task_projection.rs`;
- surface/version manifests, architecture registry и aggregate tests.

W0 ограничивает family-level часть crate-private SPI, уже начатую текущим
кодом; W2a замораживает request-level wrapper и final-effect reconciliation:

```text
ApplyRequest + ApplyOp + OperationRegistry
ApplyStagedState + ProvisionalApplyEffects + PlannedApplyEffects + ApplyPlanError
parse_<family>_plan_operation(...)
IndexedPlanOperation<T> { request_index, operation }
ProvisionalApplyEffect { event, touched_paths }
plan_<family>_batch(
    ApplyStagedState,
    actor-issued <Family>ApplyAuthority,
    &[IndexedPlanOperation<FamilyPlanOperation>],
) -> Result<(ApplyStagedState, ProvisionalApplyEffects), ApplyPlanError>
finalize_request_effects(&ApplyStagedState, ProvisionalApplyEffects)
    -> PlannedApplyEffects
```

`crates/unica-coder/src/domain/validation.rs` хранит только типы target,
finding и failure. Общая read seam создаётся в
`crates/unica-coder/src/application/validation.rs` как порт `ValidationView`,
читающий типизированные логические узлы. Infrastructure реализует два adapter:
persisted actor snapshot и staged `ApplyStagedState`. Поэтому domain не зависит
от infrastructure, а конкретный validator получает view и не открывает
filesystem самостоятельно.

Worker меняет только своё семейство, новый isolated handler и собственные
fixtures/tests. Если нужен shared seam, worker отдаёт integrator падающий тест,
минимальный fixture и точную форму требуемого интерфейса. Самостоятельная
правка central hotspot означает ошибку декомпозиции.

## Интеграционный ритм

- каждый slice живёт не больше двух рабочих дней до integration;
- каждый дефект сначала воспроизводится падающим тестом;
- перед integration проходят focused tests, spec review и quality review;
- после integration запускаются cross-family tests;
- в конце каждой волны запускаются Rust, Python, architecture и package gates;
- Windows, Linux и macOS являются обязательными release targets;
- long-lived divergent worker branches и stacked PR запрещены.

## Stop conditions

Работа возвращается к architecture/integrator review, если:

- writer не может подготовить весь batch до публикации;
- возникает третий transaction participant помимо Source и WorkspaceCache;
- после actor admission требуется ambient filesystem read;
- operation не сопоставляется ровно одному имени принятого registry;
- parity требует сохранить старую публичную schema или alias;
- recovery требует raw args, secrets, commands или blind replay mutation;
- provider sharing смешивает actor, root или revision identity;
- проверка 7000 мс требует real sleep вместо fake clock;
- worker вынужден править shared integration file;
- до G6 изменяется public V12 `tools/list`;
- после G6 поверхность отличается от 8/11;
- требуется второй public MCP server или separately versioned executable;
- RC cache нельзя отличить от v0.12.3 cache;
- меняется архитектурный контракт без successor `DEC.*` и derived record.

## Вне критического пути

В v0.13 не входят:

- public `tools-download`;
- aliases старых инструментов;
- второй MCP-сервер или новый independently shipped binary;
- raw CLI passthrough в `run`;
- generic resumable mutation framework;
- новый public progress API;
- idempotency-token protocol;
- `tests`, `features`, `log` как отдельные tools;
- gRPC/platform 8.5 profile;
- дополнительная оптимизация удаляемого V12 surface.

## Оценка

Остаток на исходном снимке оценивается в 73–117 person-days. При одном
integrator и трёх workers
реалистичный срок до stable составляет 7–10 недель. Оптимистичная граница в
6 недель достижима только без semantic rework, host delays и release failures.

Ранний W2a и переход каждого worker к собственной W3-линии убирают общий
барьер примерно на 2–4 календарных дня, но не уменьшают person-days.
Параллелизм не отменяет последовательные gates W0/W2a, aggregate parity и
G6/G7.

## Артефакты исполнения

Эту записку реализует
`docs/plans/2026-08-28-v0-13-completion.md`. После его принятия umbrella issue
#581 хранит только живую wave/gate/owner картину и ссылку на новый план; старый
phase ledger остаётся датированной историей и не используется как текущий
backlog.
