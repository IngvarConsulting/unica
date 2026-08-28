- Date: `2026-08-28`
- Status: `approved`
- Decision: `DEC.2026-08-28.V0-13-TASK-OBSERVATION-SLICE`

# Завершение миграции v0.13 через foundation-first wavefront

## Назначение

Основная часть записки меняет способ завершения миграции, а не выбранную
композицию продукта. Она заменяет последовательное прочтение Tasks 1–24 из
`docs/plans/2026-08-23-v0-12-3-to-v0-13-migration.md` на короткий критический
путь, три непересекающихся исполнительских потока и отдельные release gates.

Ревью выявило три ранее оставленные открытыми продуктовые границы и одну общую
границу release evidence. Новый semantic blocker — это не только
live receipt-time owner, а отдельный private durable `ReceiptLedger`,
который доказывает at-most-once domain execution при потере direct
response, restart и duplicate submit. Направлением владеет planned
`DEC.2026-08-28.DAEMON-RECEIPT-LEDGER`, но его пустой `establishes`
не может быть дописан после merge. Поэтому W0 implementation commit
создаёт newly dated active successor `DEC.*` с realized evidence и
полным `establishes`, создаёт выведенные active `INV.*`/`CTR.*` и
меняет у planned predecessor только lifecycle-поля `status`/`superseded-by`.
Модель
наблюдения durable Task принадлежит planned
`DEC.2026-08-28.V0-13-TASK-OBSERVATION-SLICE`. Request-level reconciliation
apply effects принадлежит отдельному planned
`DEC.2026-08-28.REQUEST-LEVEL-APPLY-EFFECT-RECONCILIATION` и раскрыта в своей
проектной записке. Schema-derived semantic oracle, content-addressed
provenance scope lock и отзыв S2 при drift принадлежат planned
`DEC.2026-08-28.V0-13-MIGRATION-EVIDENCE-GATE` и раскрыты в отдельной
проектной записке. Каждая реализация получит действующее основание только через
active successor/evidence `DEC.*` с именованной проверкой и производным
правилом. Body принятого product decision после merge не меняется; successor
обновляет только его lifecycle-поля `status` и `superseded-by`. Эта записка не
выдаёт планируемое поведение за текущее.

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
- lifecycle до executor неполон: блокирующие workspace admission/prepare идут
  до live Invocation/Task owner, а canonical MCP cancellation до получения
  TaskId теряется; поэтому S1 ещё не достигнут даже при зелёных POSIX-тестах;
- in-memory receipt map и предварительно зарезервированный TaskId не закрывают
  durable receipt: restart забывает pre-Task owner, а потерянный Direct
  response не имеет долговечного dedupe/recovery witness; поэтому такая
  реализация не проходит S1;
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

После исходного design snapshot текущий committed baseline дошёл до
`f66d5948`. Уже доставлены и не входят повторно в remaining estimate:

- formatting/guardrails, compaction unmerged retained-evidence decision и
  ранний Windows cleanup (`22a3a9cf`, `9156efe7`);
- retained actor root (`41593777`);
- hidden apply-family seams, targeted actor-issued admission и no-publication
  proof (`21170c94`, `764dd1e8`, `3229485d`);
- семь parity shards и strict structural inventory validator
  (`d0893efe`, `686ea194`, `e870d810`), для которого pinned check сейчас
  зелёный 39/39, но semantic oracle/execution остаются W0.5/W4;
- Windows Job Object ownership/root-and-child cleanup (`f66d5948`) с зелёными
  local smoke/probe/actionlint/full-code gates.

Последний пункт implementation-complete, но не заменяет fresh remote `win-x64`
evidence для S1. Structural parity skeleton также не заменяет W0.5 oracle и W4
semantic execution.

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

Один receipt-time Invocation owner должен существовать до потенциально
блокирующих filesystem admission и service preparation, но live owner не
является durability boundary. Private daemon владеет отдельным
`ReceiptLedger`, а TaskStore остаётся хранилищем только materialized Tasks.
Допустимый submit атомарно резервирует exact
`InvocationId` + `reservedTaskId` + daemon-derived secret-free request identity до
предметной validation, workspace admission, `prepare` и `execute`. Wire-строка,
которую strict outer-envelope parser не может безошибочно превратить в
`SubmitInvocation` с каноническими обоими IDs, tool, bounded arguments и
workspaceHint, не
является valid submit: она отклоняется без ledger row и без любой
предметной работы. Эта strict envelope boundary не включает
валидацию tool arguments, workspace или execution class. Она при этом строго
отклоняет `responseBudgetMs > 7000` и oversized workspaceHint; clamp/truncate с
последующим reserve запрещён.

Request identity содержит exact 64-hex `CoreIdentity`, `ToolIdentity`,
`NormalizedArgumentsHash` канонических arguments и application-owned
`RequestScopeHash`
от `workspaceHint`; transient response budget, deadline и retry timing в него
не входят. Frontend и daemon используют одну canonical derivation, чтобы
pre-submit cancel мог назвать полный `ReceiptKey`, но daemon всегда
пересчитывает digest из strict parsed submit и отклоняет mismatch до reserve и
любой предметной работы. Exact duplicate после restart или потери transport читает тот же
receipt/terminal result и не вызывает `prepare`/`execute` повторно.
Checked `CoreIdentityDigest` принадлежит application; infrastructure
`CoreIdentity` оборачивает его и отдаёт typed digest, а application не
импортирует infrastructure identity type.
`RequestScopeHash` distinct от actor-derived `SafeIdentityHash` и вычисляется
одной application helper как SHA-256(`unica.request-scope.v1\0` + u32-BE length
+ exact strict-parsed workspaceHint UTF-8 bytes), без filesystem
canonicalization, separator/case/Unicode folding или trim. Frozen
`workspace-a` vector равен
`9f7a5a77bb6eb469cd20147a9aeee9d9769a8372f587bd89635d15684ee02b39`;
frontend/server/fixture вызывают одну helper, а не копируют algorithm.
Application-owned `ReceiptKeyDigest` вычисляется SHA-256 над domain
`unica.receipt-key.v1\0`, затем над шестью normalized components в fixed order:
InvocationId, reservedTaskId, core digest, tool wire name, normalized arguments
hash, request-scope hash; каждый component framed как u32 big-endian byte length
плюс bytes. Другого hash/framing owner нет. На open derived uniqueness indexes
`InvocationId -> exact ReceiptKey` и `reservedTaskId -> exact ReceiptKey`
rebuild-ятся вместе по retained active/tombstone/link records. Partial collision
reject-ится до mutation; persisted collision срывает store open до listener.
Distinct application-owned checked `TaskLinkDigest` является стабильной link
identity, не Task-record digest. One helper SHA-256-ит domain
`unica.task-link.v1\0`, затем u32-BE length + normalized bytes exact ordered
`ReceiptKeyDigest` (lowercase 64-hex ASCII), canonical TaskId UUID text,
canonical InvocationId UUID text и actor-derived `workspaceIdentityHash`
(lowercase 64-hex ASCII). Status/version/updatedAt/cancelRequested/terminal и
ephemeral actor generation исключены. Ledger, TaskStore, runtime recovery и
terminal codec получают typed digest через эту helper; fixture вызывает только
production export. Frozen vector: ReceiptKeyDigest из 64 `0`, TaskId
`11111111-1111-4111-8111-111111111111`, InvocationId
`22222222-2222-4222-8222-222222222222`, workspaceIdentityHash из 64 `a` →
`4c73d08219973c72e759a9f85e156fa42c9d8e61a56e704b70d1c7c042b73da0`.
Потеря submit session сама по себе не меняет state и не ускоряет Task handoff:
daemon продолжает исходный lifecycle под original cutoff и ещё может durable
опубликовать Direct. Только original cutoff внутри Unbound либо текущий
post-prepare `KnownLong`/actor-bound cutoff выбирают соответствующий Task path.
Durable cutoff descriptor хранит acceptance epoch и исходный bounded budget;
live monotonic cutoff остаётся capability процесса. Restart не восстанавливает
`Instant` из wall time и не выдаёт новый budget: bare `Reserved` закрывается по
durable phase/cancel evidence.
Protocol/CoreIdentity одновременно ограждает private session и форкает state
directory: v3/v4/v5 process, key и state не смешиваются.
Этот request digest никогда не подменяет actor-derived workspace identity,
которая появляется только после успешного admission.
Повторное использование одного ID с другим парным ID или другим
request identity даёт closed mismatch rejection и не меняет исходную
Invocation.

`ReceiptLedger` хранит закрытые состояния `CancelReserved`, `Reserved` с фазами
`Unbound`/`ActorBound`/`Begun`, `DirectTerminalUnacked`,
`AcknowledgedTombstone`, `TaskPromisedUnbound`, `TaskPromisedActorBound`,
`TaskHandoffActorBound`, `TaskReceiptOwnedActorBound`,
`TaskTerminalReceiptBacked`, `TaskBound` и
`TaskTerminalBound`, затем terminal-only `TaskRetirementPending`;
v5 terminal payload един для Direct, receipt-backed Task и v5 TaskStore:
`Completed { DomainResult }`, `Failed { V5SafeFailureReason }` либо
`Cancelled`.
Terminal owner определяется current durable state, не historical promise.
Semantic validation rejection и все `WorkspaceAdmissionError` завершаются до
actor bind: в `Reserved::Unbound` они Direct, в `TaskPromisedUnbound` —
receipt-backed Task. `Invalid` становится `Completed` с
`DomainResult.ok=false`; `Capacity` — `Failed {
V5SafeFailureReason::WorkspaceCapacity }` без prepare/execute/restart;
`RegistryFailed` — `Failed { V5SafeFailureReason::WorkspaceRegistryFailed }`,
затем `RestartRequested` и fail-stop. `service.prepare` вызывается только после
`Begun`: его semantic rejection остаётся Direct только пока owner
`Reserved::Begun`; в pre-bind `TaskHandoffActorBound` оно stage-ится, затем
while handoff+live reservation retained TaskStore пишет/readback-ит terminal и
ledger одним commit переходит прямо в `TaskTerminalBound`, без промежуточного
`TaskBound`; при proven begun
Link Capacity — receipt-owned terminal; после `TaskBound` — всегда TaskStore +
`TaskTerminalBound`. `Failed` несёт только closed `V5SafeFailureReason`, а
`Cancelled` не несёт `DomainResult`. Current `SafeFailureReason` и schema-v2
`StoredInvocationRecord` остаются byte/serde-acceptance unchanged. Side-by-side
v5 вводит отдельные `V5SafeFailureReason` и `V5StoredInvocationRecord`:
v5 enum содержит exact legacy `InvocationFailed`, `ResultTooLarge`,
`Interrupted`, `ResumeUnsupported`, `PersistenceFailed` плюс
`OutcomeUncertain`, `TaskCapacity`, `WorkspaceCapacity`,
`WorkspaceRegistryFailed`, с total infallible conversion legacy->v5.
Он не alias и не расширение legacy enum. `InterruptedBeforeExecution` остаётся
recovery classification, а persisted reason — `V5SafeFailureReason::Interrupted`.
Active v3 persisted path, включая current early protocol mapping admission
Capacity/RegistryFailed, остаётся неизменным до W0c. Receipt не откатывается;
`TaskCapacity` принадлежит только proven `LinkCapacity` до `Begun`,
а два workspace reasons — только typed binder branches. Exact canonical
payload/persisted-record/transient-frame size preflight предшествует каждой
соответствующей mutation/write; после prepared frame запрещён fallback protocol
error. Post-reserve catalog/schema
invariant failure, admission identity/proof drift, serialization invariant и
store/receipt uncertainty требуют fail-stop плюс exact readback/recovery.
Binder раздельно классифицирует semantic Invalid, Capacity, RegistryFailed,
deadline expiry, identity/proof drift и internal: только semantic Invalid
становится `Completed`, а последние три technical cases не маскируются под
admission result.
`outcome_uncertain` является closed terminal outcome,
а не replayable фазой. Перед первым `service.prepare` daemon сначала
durable-фиксирует `Reserved::Begun` с actor-derived identity. Если процесс
падает после этого marker, но до terminal
commit, recovery не может обещать известный business outcome: она фиксирует
`V5SafeFailureReason::OutcomeUncertain`, distinct в store/snapshot/projections, и никогда не
повторяет callback. Исключение допустимо только для конкретной операции,
если её writer в том же slice доказывает atomic coupling или idempotent
recovery. Bare `Reserved::Unbound/ActorBound` без committed promise/handoff при
restart становится `DirectTerminalUnacked(cancelled |
interrupted_before_execution)` по durable flag; bare `Reserved::Begun` без
handoff становится `DirectTerminalUnacked(Failed {
V5SafeFailureReason::OutcomeUncertain })`. Task не
создаётся задним числом: её разрешает только committed
`TaskPromised*`/`TaskHandoffActorBound`. Direct terminal сначала фиксируется как
`DirectTerminalUnacked`, и только после durability proof пишется в submit
session. Private explicit ACK той же exact identity разрешает удалить terminal
payload/nonessential metadata и оставить compact tombstone, содержащий только
exact key, terminal digest и epoch первого committed ACK. Cutoff, original
budget и result bytes в tombstone не переходят. До ACK потеря response
восстанавливает тот же terminal Direct result; после ACK duplicate в
пределах horizon закрыто отклоняется, а не исполняется снова.
Канонический private wire — `unica-daemon-jsonl-5`: recovery идёт через
`RecoverInvocationReceipt { receiptKey }`, ACK — через
`AcknowledgeInvocationReceipt { receiptKey, terminalDigest }`, а cancellation несёт
полный `ReceiptKey`. Client получает `V5PendingDirectReceipt { terminal:
V5TerminalOutcome, receiptKey, terminalDigest, terminalEpochMs }`, не
result-only wrapper, и ACK-ит его только после успешного построения окончательной
immutable native projection: `CallToolResult` для Completed либо exact
`ErrorData` для Failed/Cancelled. Projection/drop/crash до готового final object
ACK не посылает. Premature,
mismatched и Task ACK закрыто отклоняются без mutation; потеря request/response
разрешает exact retry. Если tombstone pool заполнен после expired-only reclaim,
ACK возвращает typed `TombstoneCapacity`, а byte-equivalent
`DirectTerminalUnacked` остаётся до retry либо terminal+1h expiry.
Это подтверждает daemon→frontend handoff, но не обещает exactly-once
доставку frontend→MCP host.

Wire/task freeze является hard gate W0, а не implementation detail. Все вновь
введённые v5 envelopes/records/selected enum variants — `camelCase` +
`deny_unknown_fields`; tags и variants — exact snake-case, defaults/flatten/open
maps в этих новых типах запрещены. Existing `DomainResult` внутри Completed
переиспользуется byte-for-byte с неизменной serde acceptance; v5 не создаёт его
копию и не меняет active v3 contract. Protocol
messages имеют `protocolVersion:5` и не имеют `schemaVersion`. Client `kind`
algebra: `hello(protocolVersion,token,coreIdentity,ownerLease)`, `ping`,
`release`, `submit_invocation(invocation)`, `get_task(taskId)`,
`wait_task(taskId,waitMs)`, `cancel_task(taskId)`,
`recover_invocation_receipt(receiptKey)`,
`acknowledge_invocation_receipt(receiptKey,terminalDigest)`,
`cancel_invocation(receiptKey)`. `V5InvocationRequest` exact fields:
`invocationId,reservedTaskId,tool,arguments,workspaceHint,responseBudgetMs`;
full `V5ReceiptKey`:
`invocationId,reservedTaskId,coreIdentityDigest,tool,normalizedArgumentsHash,requestScopeHash`.

Server `kind` algebra: `ready(protocolVersion,coreIdentity,daemonPid,instanceId)`,
`pong`, `released`, `invocation(outcome)` для submit/exact duplicate/recovery,
`task(snapshot)`,
`invocation_acknowledged(acknowledgement)`, `error(code)`.
`V5InvocationResponse` tag `resultType` имеет exact `receipt_pending`, `direct`,
`task`, `acknowledged`; `V5TerminalOutcome` tag `status` имеет только
`completed{result}`, `failed{reason}`, `cancelled{}`. Code-only protocol errors:
`invalid_request,handshake_required,protocol_mismatch,core_mismatch,unauthorized,duplicate_lease,overloaded,owner_capacity,receipt_not_found,receipt_expired,receipt_capacity,tombstone_capacity,invocation_identity_mismatch,task_not_found,task_expired,store_failed,durability_uncertain`.
Post-reserve semantic/admission/uncertain/capacity outcomes не являются
protocol errors.

Strict `V5DaemonTaskSnapshot` tag `status` имеет five variants
`queued|working|completed|failed|cancelled` и common exact fields
`taskId,invocationId,receiptKeyDigest,createdAtEpochMs,updatedAtEpochMs,ttlMs,pollIntervalMs,version,cancelRequested`;
Completed добавляет `terminalEpochMs,terminalDigest,result`, Failed —
`terminalEpochMs,terminalDigest,reason`, Cancelled —
`terminalEpochMs,terminalDigest`. Distinct `V5StoredInvocationRecord` имеет
`schemaVersion:1` плюс exact
`taskId,invocationId,receiptKeyDigest,tool,normalizedArgumentsHash,workspaceIdentityHash,createdAtEpochMs,updatedAtEpochMs,ttlMs,pollIntervalMs,version,cancelRequested,task`;
nested task использует ту же closed terminal algebra без optional result/open
failure. `V5SafeFailureReason` — exact legacy five плюс
`OutcomeUncertain,TaskCapacity,WorkspaceCapacity,WorkspaceRegistryFailed`.

Canonical terminal bytes — minified strict outcome JSON; одна application
authority строит `V5CanonicalTerminal` и `TerminalDigest` над domain
`unica.terminal-outcome.v1\0`, u32-BE length и payload.
Frozen Cancelled vector использует 22-byte payload `{"status":"cancelled"}`,
framed input hex
`756e6963612e7465726d696e616c2d6f7574636f6d652e763100000000167b22737461747573223a2263616e63656c6c6564227d`
и digest
`f2d0423d2613a0d09397b750542e4542f7653d78ebd5e0448f1326d09145d9ae`;
fixture вызывает production export, не local framing/hash. Application объявляет
opaque owner-specific linear publication types; единственный
`infrastructure/daemon/terminal_codec_v5.rs` строит
`PreparedReceiptRecord`, optional `PreparedTaskRecord`, optional
`PreparedTaskLifecycleLinkRecord` и transient
`PreparedWireFrame`, никогда из arbitrary adapter bytes. Direct/ReceiptTask
pieces bind exact receipt expected version; BoundTask pieces bind independent
task expected version, lifecycle-link expected version и exact link digest.
Staged pre-materialization pieces bind active receipt+reservation versions and
build the new sole lifecycle-link record at commit. Отдельные
HandoffStage pieces до первой staged publication строят
`PreparedStagedReceiptRecord` и typed
`StagedTerminalTransferSizeCertificate`, bound к protocol-v5/CoreIdentity,
key/task/link, terminal digest+epoch, exact receipt version и frozen schema/limit
versions; raw outcome/bounds/certificate ledger не принимает. До первой ledger
mutation certificate exact-preflight-ит staged receipt и conservative maxima для
final terminal Task, sole `TaskTerminalBound` lifecycle-link record bytes, v5
Task wire и receipt-backed staged-winner fallback receipt+wire для proven
LinkCapacity без reservation. Bounds охватывают
`Absent`, каждый `ExactProvisional` Queued/Working, обе cancel booleans, max-width
u64 version/epochs и JSONL newline. Certificate evidence persists inside the
staged record without a wire frame, поэтому ledger возвращает opaque typed
`CommittedStagedHandoffReceiptWithCertificate` exact readback/version, включая
reopen. StagedHandoffTask pieces позже принимают только этот readback, внутри
sole codec rehydrate-ят canonical terminal и bind closed
`StagedTaskPublicationExpectation::{Absent,
ExactProvisional { taskId, invocationId, status: Queued|Working, version,
cancelRequested, taskLinkDigest }}`, live
reservation, receipt expected version и link digest, не требуя существующего
`TaskBound`. Late codec consumes checked certificate and requires every exact
piece size `<=` its owner-specific bound; valid certificate makes late oversize
unreachable, while binding/schema/size mismatch fail-stops without changing the
staged winner or reclassifying it as `ResultTooLarge`. Каждый
store consumes только свою piece/version; TaskStore возвращает exact readback с
оставшимися lifecycle-link/wire pieces, ledger после link verification
consume-ит sole lifecycle-link piece, удаляет active receipt representation и
возвращает committed publication с untouched wire frame.
Bundle algebra закрыта: `Direct|ReceiptBackedTask = PreparedReceiptRecord +
PreparedWireFrame`; `BoundTaskStore = PreparedTaskRecord +
PreparedTaskLifecycleLinkRecord + PreparedWireFrame`; `HandoffStage =
PreparedStagedReceiptRecord + StagedTerminalTransferSizeCertificate`;
`StagedHandoffTask = PreparedTaskRecord + PreparedTaskLifecycleLinkRecord +
PreparedWireFrame`; `StagedCapacityFallback = PreparedReceiptRecord +
PreparedWireFrame`. Cross-owner/extra piece и `PreparedReceiptRecord` в двух
TaskStore-owned variants отвергаются типом; staged TaskStore commit atomically
replaces active receipt+reservation sole lifecycle-link record-ом.

Persisted certificate schema закрыта буквально: strict
`deny_unknown_fields { certificateVersion:1, protocolIdentity:"v5",
coreIdentityDigest, receiptKeyDigest, taskId, invocationId, taskLinkDigest,
terminalDigest, terminalEpochMs, receiptRecordSchemaVersion:1,
taskRecordSchemaVersion:1, lifecycleLinkRecordSchemaVersion:1,
terminalCodecVersion:1,
maxDaemonResponseLineBytes:8454144, stagedReceiptRecordMaxBytes,
maxTaskLifecycleLinkRecordBytes:1024,
taskTerminalBoundLinkRecordMaxBytes,
taskPublicationCases, capacityFallbackCases }`. Здесь нет nullable Options или
неопределённых schema/limits digests. `taskPublicationCases` имеет exact пять
internally tagged `deny_unknown_fields` entries с tag field `kind`: один
`kind:"absent" { finalTaskRecordMaxBytes, taskResponseFrameMaxBytes }` и четыре
`kind:"exact_provisional" { status, version, cancelRequested,
finalTaskRecordMaxBytes, taskResponseFrameMaxBytes }` для exact lowercase
`queued|working` × false/true, с literal version
`18446744073709551615` как max-u64 width witness, в literal order absent,
queued/false, queued/true, working/false, working/true.
`capacityFallbackCases` имеет exact один entry с tag field `source`:
`source:"link_capacity"` только с
`receiptBackedRecordMaxBytes,taskResponseFrameMaxBytes`. Missing/extra/
cross-variant fields, duplicate tags, wrong order/cardinality/literals fail reopen.
Late codec выбирает exact actual case, проверяет его bytes/real-version width
против bound, требует `taskTerminalBoundLinkRecordMaxBytes <= 1024` и не
допускает terminal reclassification.
Closed `StagedLinkCapacityEvidence` возвращает certificate authority untouched и
доказывает отсутствие reservation. Sole codec строит certified
`TaskTerminalReceiptBacked` fallback record+wire с тем же winner и ничего не
освобождает. Этот path не пишет `TaskCapacity` поверх staged winner.
Persisted owner хранит ровно один canonical payload/digest/terminal epoch под
inclusive MAX entitlement, не duplicate full response frame. Immediate submit
использует prepared frame; exact duplicate/recovery через тот же
CoreIdentity-bound codec заново preflight-ят owner-specific frame до каждого
write. После frame preflight reserialization/fallback запрещены, но durable
terminal не притворяется persisted wire cache. Static ownership guard разрешает
piece construction и envelope serialization только codec/golden tests. Native Direct projection:
Completed — exact `CallToolResult`, Failed — `ErrorData(-32603, closed
reason message, {code})`, Cancelled — `ErrorData(-32603,"daemon invocation was
cancelled",{code:"invocation_cancelled"})`; ACK следует только после готового
immutable object. Native Task projection использует Working для internal
Queued/Working, Completed+CallToolResult, Failed+typed ErrorData либо Cancelled;
compatibility сохраняет exact queued/working/completed/failed/cancelled status,
raw IDs/timestamps и nine reason codes. Exact reason code/message table из
`2026-08-28-daemon-receipt-ledger-design.md` является обязательным byte-level
gate; v5 не переиспользует open `InvocationFailure`, а `OutcomeUncertain` не
сворачивается в `Interrupted`. Current v3 serde/projectors остаются literal
unchanged.

Для staged handoff TaskStore terminal write/readback происходит, пока ledger
сохраняет handoff, live reservation, staged canonical payload и persisted checked
transfer-size certificate. Late pieces уже доказаны этим certificate и exact
проверяются против его bounds. Remaining linear lifecycle-link/wire pieces
разрешают только direct
handoff→`TaskTerminalBound` commit с удалением staged bytes. Crash в этом окне
оставляет exact safety copy в обоих stores; recovery делает только Task
readback и ledger commit, без `TaskBound`, callback либо reserialization.

К cutoff 7000 мс внутри ещё `Reserved::Unbound` validation/admission ledger
атомарно переходит в `TaskPromisedUnbound` и сразу проецирует exact reserved TaskId
как durable queued Task. Если validation или actor admission ещё блокируются,
TaskStore record не создаётся: без actor-derived workspace identity он был бы
ложным. Cancel или restart до actor bind terminalizes эту receipt-backed Task
под тем же TaskId без callback и без TaskStore placeholder. Текущий
`KnownLong`, возвращаемый `service.prepare` уже после `ActorBound` и
`Reserved::Begun`, не возвращается в unbound-состояние: он проходит через
actor-bound handoff intent и пытается exact TaskStore create/readback;
proven Link Capacity после `Begun` оставляет его receipt-owned без TaskStore record.

Semantic validation либо любой `WorkspaceAdmissionError` после unbound promise,
но до actor bind durable-публикует `TaskTerminalReceiptBacked` под current ledger
owner: Invalid — `Completed` с exact canonical `DomainResult.ok=false`,
Capacity/RegistryFailed —
receipt-backed `Failed` с соответствующей v5 reason; RegistryFailed затем
останавливает listener/restarts, Capacity оставляет daemon доступным. TaskStore
до actor bind не существует. Эта projection повторно читается
до полного Task TTL; Direct ACK к ней неприменим. Promised pipeline получает
one per-invocation fixed two-second cleanup grace от commit promise: validation
передаёт ту же absolute deadline admission/bind без reset; зависший
validator/admission вызывает
process-owned fail-stop без join, а terminal-winner check перед каждым
следующим callback/bind запрещает позднему worker продолжить после
cancel/restart/terminal.

После ActorBound оба Task path используют один recoverable write-ahead
protocol. Ранее созданный unbound promise переходит в
`TaskPromisedActorBound`, а cutoff или
current `KnownLong` из `Reserved::ActorBound`/`Reserved::Begun` — в
`TaskHandoffActorBound`; оба состояния несут exact Task record с actor-derived
identity. Infrastructure-private `runtime_v5::InvocationCoordinator` единолично держит
`BoundStartCancelGate`, provider `CancellationToken`, actual actor proof и
actor/resource lease. Application `invocation_v5` владеет только pure lifecycle
state machine и `ActorBindingClaim { identity, generation }`/opaque token/port
types. `bind_actor`/`bind_promised_actor` принимают claim и после durable bind
возвращают committed receipt + one-shot `V5ActorBindingToken` exact key/identity/
generation. Перед Direct begun coordinator private-проверяет live lease, под
gate предъявляет token ledger, а ledger consumes exact match once. Для Task start
`authorize_bound_task_start` так же consumes actor token и возвращает distinct
`PostWorkingActorAuthorization`; после atomic Working readback coordinator снова
private-проверяет lease, а ledger consumes это authorization в
`mark_bound_task_begun`. Оба Task-bound ledger calls проверяют exact current
sole lifecycle-link expected version/CAS; active receipt version после
materialization не существует. Это не cryptographic/unforgeable claim: static
ownership/import guard ограничивает constructor ledger module, actual
lease/proof/verifier/coordination — `runtime_v5`, и запрещает infrastructure
import из application. Verdict boolean отсутствует, restart token из persisted
hash не строит.
До любой TaskStore mutation ledger durable резервирует exact Task-link count и
full 1 KiB byte entitlement. При отсутствии staged/cancel winner proven link
Capacity выбирает TaskCapacity branch
без TaskStore mutation; только с `TaskLinkReservation` TaskStore делает
idempotent create/readback. В обычной nonterminal ветке ledger materializes link
и фиксирует `TaskBound` под live per-invocation `BoundStartCancelGate`, который
охватывает exact identity/cancel-flag readback и transfer sole cancel authority;
только затем resolver атомарно переключает источник projection. `bind_task`
consumes pre-bind reservation и возвращает distinct opaque
`TaskBoundLinkAuthorization` exact key/task/link/generation. В staged-terminal
ветке TaskStore сразу пишет/readback-ит exact terminal record, пока handoff и
reservation retained. `Absent` создаёт terminal; exact provisional
Queued/Working, успевший committed до staged CAS, атомарно terminalize-ится
только по exact TaskId/InvocationId/status/version/cancel/link-digest readback;
same terminal idempotent, foreign/mismatch fail-stop. После этого одна ledger mutation consumes reservation/materializes
link прямо в `TaskTerminalBound` и удаляет staged payload. `TaskBound` и start
authorization в этой ветке не создаются.
Успешная link reservation структурно резервирует TaskStore count slot. Если
TaskStore create всё же возвращает count `Capacity`, adapter выдаёт только
`TaskStoreCapacityInvariantViolation`; тот же continuously-held gate сохраняет
intent/reservation/staged winner и закрывает listener для fail-stop без receipt
commit, fallback либо release. `CommitUncertain` также удерживает reservation до
exact reconciliation, но остаётся отдельной commit-классификацией. Normal
pre-Begun `task_capacity` и begun `TaskReceiptOwnedActorBound` возникают только
из proven Link Capacity до reservation. Последний сохраняет single live attempt,
actor identity, cancel flag и reserved result quota, не повторяет reservation/
TaskStore create и публикует actual либо crash-derived `outcome_uncertain` как
`TaskTerminalReceiptBacked`.
Pre-bind terminal/cancel release-ит `TaskLinkReservation` только своей durable
terminal mutation; successful bind consumes её навсегда. Post-bind
`TaskBoundLinkAuthorization` invalidates at terminal/cancel winner, generation
change, Task/link expiry or PID death; restart её не reconstruct-ит и callback
не запускает, а materialized link сохраняется до Task retention.
Recovery materializes link для existing Task только из exact retained handoff
intent + matching live `TaskLinkReservation`; absent/expired/mismatched
reservation означает corruption/fail-stop до link mutation и не может обойти
4096 cap.

Для `TaskBound { begun:false }` coordinator удерживает тот же gate до receipt
begun: private-verifies live actor/resource lease и consumes actor binding token
в `authorize_bound_task_start`, получая `PostWorkingActorAuthorization`. Затем
TaskStore sole-writer принимает exact
`start_working_if_not_cancel_requested(TaskBoundLinkAuthorization,
exactCurrentTaskBoundProof, taskIdentity, expectedVersion, deadline)` без separate false-read
либо возвращает cancel/terminal winner, либо commits exact versioned
Queued→Working/readback с remaining exact link evidence. Coordinator повторно
private-verifies lease, а `mark_bound_task_begun` под тем же guard consumes
`PostWorkingActorAuthorization`, проверяет readback/link и фиксирует
`begun:true`; затем gate освобождается
и вызывается `service.prepare`. Missing/foreign/stale proof до authorization
ничего не меняет; stale после Working запрещает begun/prepare, требует
fail-stop, а recovery terminalizes interrupted-before-execution. Executor
удерживает proof/actor/resource lease до terminal cleanup или смерти PID. До
receipt commit resolver маскирует underlying Working как queued. Crash в этом
окне terminalizes cancelled только при durable TaskStore flag, иначе
interrupted-before-execution без callback; crash после receipt begun —
`outcome_uncertain` без replay, а begun+Queued mismatch ведёт к fail-stop. Уже
begun handoff продолжает тот же единственный attempt.

Direct path не обязан создавать TaskStore record: под тем же gate
`mark_reserved_begun` атомарно проверяет live proof и `cancelRequested=false` и
возвращает один begun/cancel winner, затем terminal outcome хранится в
ReceiptLedger.
Cancel до submit создаёт durable pre-submit reservation; последующий exact
submit завершается без предметной работы.

До commit `TaskBound` после exact TaskStore create/readback `ReceiptLedger`
остаётся единственным durable cancel authority:
`TaskPromisedActorBound` и `TaskHandoffActorBound` сохраняют monotonic
`cancelRequested` до TaskStore create и до live-token signal. Bind под
`BoundStartCancelGate` переносит тот же flag в TaskStore, подтверждает
identity/flag exact readback и только затем публикует `TaskBound`, передавая
authority. Cancel берёт тот же gate, re-resolves current authority, durable
фиксирует flag/terminal и только затем сигналит token. Поэтому cancel после
false observation, но до atomic Working, выигрывает без callback; cancel после
Working ждёт receipt begun и становится post-Begun. Begun handoff не может
объявить `cancelled`: crash между receipt flag, Task create и token создаёт
exact Working Task и terminalizes `outcome_uncertain` без replay.
`TaskReceiptOwnedActorBound` также сохраняет cancel flag в ReceiptLedger,
но не может stage-ить `cancelled` после `Begun`.
Native `tasks/cancel` и compatibility `unica.task.cancel` проходят один resolver
и для каждого closed receipt/Task state возвращают одинаковый typed winner;
terminal/read-only state не получает новый DomainResult, TTL или terminal
переход из-за выбора projection API.

Protocol-v5 startup не вызывает legacy eager `TaskStore::open`, который
terminalize-ит `Queued`/`Working` до чтения receipt. Сначала
`FileInvocationStoreV5::open_inspect_only` возвращает immutable
`TaskStoreRecoveryCatalog` без mutations; затем single-threaded
`ReceiptRecoveryCoordinator::reconcile` выбирает exact
`RecoveryTerminalReason` и вызывает только
`terminalize_recovered_exact`. Listener публикуется лишь после
`RecoveryComplete`; orphan `Queued` и orphan `Working` v5 records без exact
receipt link проверяются раздельно и оба fail-stop-ятся без eager mutation.
TaskStore create `CommitUncertain` сохраняет intent для exact readback и не
может быть переименован в proven Capacity либо вызвать blind retry. Inspect-only
open также не lazy-delete-ит expired v5 Task: terminal retirement исполняет
ReceiptLedger-led `TaskRetirementPending` saga до listener; active `TaskBound` с
absent Task без Pending evidence является corruption.

Емкость и retention ledger не заимствуют лимит TaskStore. W0 фиксирует
отдельные production bounds из наблюдаемой нагрузки. Общий live count cap 64
охватывает `CancelReserved`, `Reserved`, promised/handoff/receipt-owned,
`DirectTerminalUnacked` и `TaskTerminalReceiptBacked`; его
actual-plus-reserved byte cap равен
`64 × 8 454 144 = 541 065 216` bytes. `CancelReserved` занимает один slot и
не более 1 024 metadata bytes без result reservation, живёт исходные 7 125 мс
без продления от duplicate, а exact submit атомарно получает полный result
reserve в том же pool. Этот reserve остаётся сквозь promise/handoff до одного из
четырёх durable events: exact TaskStore bind/readback, Direct ACK, physical
expiry/delete `DirectTerminalUnacked` или expiry receipt-backed terminal Task.
Поэтому canonical terminal projection до exact TaskStore bind/readback всегда
имеет место.
Каждый payload-capable entitlement учитывает ровно один persisted canonical
terminal payload/digest/epoch вместе с record metadata; full response frame не
persisted рядом и не начисляется второй раз. Immediate/duplicate/recovery frame
transient и каждый раз проходит отдельный codec size/hash preflight до write.
В staged transfer receipt MAX share остаётся charged до direct
`TaskTerminalBound`; link reservation уже гарантирует TaskStore count slot, а
codec отдельно preflight-ит exact record против независимого per-record byte
limit. Одна canonical payload может кратко существовать в двух
stores как crash-safe transfer copy, но это не второй receipt entitlement и не
persisted wire frame; ledger commit удаляет staged copy и освобождает receipt
share.
`DirectTerminalUnacked` хранится один час именно от terminal epoch либо до ACK;
по истечении часа record физически удаляется без tombstone и освобождает exact live count/result quota;
`TaskTerminalReceiptBacked` — один час от terminal epoch без Direct ACK.

Отдельный bound Task-link pool допускает 4 096 records, максимум 1 024 bytes
каждый и 4 194 304 bytes суммарно. `TaskBound`, `TaskTerminalBound` и
`TaskRetirementPending` являются closed variants одного sole
`TaskLifecycleLinkRecord`, а не дополнительными active-receipt records: ledger
materialization CAS удаляет active receipt representation, переводит key/dual-ID
indexes на lifecycle-link record и не оставляет duplicate metadata charge. Его
retention не короче retention соответствующей Task и заканчивается только
ordered terminal retirement; terminal variant хранит TTL/expiresAt.
Reserved+materialized lifecycle links входят в одни caps. Для isolated v5 state
каждый TaskStore record injectively/exactly связан с materialized lifecycle link
либо retained reservation; Pending после Task delete может только увеличить
число links относительно Tasks. Startup до listener и каждая mutation проверяют
`taskStoreRecordCount <= materializedLifecycleLinkCount +
liveLinkReservationCount <= 4096`. Reservation происходит до TaskStore create и
успешно добавляется только при pre-create `taskStoreRecordCount <= 4095`; exact
readback converts её, а `CommitUncertain` удерживает. Post-reservation count
Capacity является `TaskStoreCapacityInvariantViolation`, не release/terminal
branch. Proven link Capacity на
4 097-й Task не касается TaskStore и не вытесняет существующие records;
pre-Begun handoff публикует
receipt-backed `task_capacity`, а begun handoff остаётся
`TaskReceiptOwnedActorBound` до actual/uncertain terminal. Post-ACK Direct horizon равен 15 минут и
`32 calls/s × 900 s + 64 = 28 864` compact tombstones. Tombstone ограничен
512 bytes, содержит только exact key/digest/first-ACK epoch, поэтому его independent byte cap равен
`28 864 × 512 = 14 778 368` bytes. Цель 32 acknowledged Direct calls/s выведена
из текущего admission limit 32: каждый admitted worker завершает и ACK-ит
один fast Direct в секунду. Deterministic test сначала проводит 28 800 ACKed
lifecycle за fake-clock 900 секунд с 32 retained terminals, не более 32
одновременно cycling Direct и 4 096 independent Task links; отдельная фаза
заполняет ровно 64 live slots и отклоняет 65-й. Tombstone expected count и
high-water вычисляются из raw first-ACK epochs и interval
`[ackEpoch, ackEpoch+900s)`; 28 864 — cap, а не требование держать все 28 800
traffic tombstones одновременно в `t=900s`. Full pool после expired-only
reclaim возвращает typed capacity и сохраняет unacked Direct. Reopen
пересчитывает exact count/bytes и сохраняет terminal/first-ACK epochs. Каждая OS за 60 секунд проводит ≥1 920 полных
reserve→small terminal→ACK lifecycle, с p99 ≤250 мс, нулём capacity/store
errors и drain writer queue ≤2 с.
Live/nonterminal `TaskBound` не expires. Terminal Task сначала имеет
`TaskTerminalBound`; при `now >= expiresAt` ledger CAS-пишет
`TaskRetirementPending` с key/task/link, terminal digest/epoch/TTL/expiresAt,
expected Task version и retained link/dual-ID accounting, после чего resolver
возвращает `task_expired`. Begin transition возвращает opaque nonserialized
one-shot authorization. После reopen infrastructure-private coordinator читает
exact Pending и единственный может вызвать
`authorize_existing_task_retirement(pending_exact_readback,
pending_expected_link_version, deadline)`; ledger mint-ит свежую exact-bound
authorization, а token старого процесса недействителен. Только такая Pending
authorization разрешает v5 TaskStore exact delete-if-expired. Exact `Deleted` либо
`AbsentExactWithPending` proof позволяет final ledger CAS удалить Pending
lifecycle-link record и indexes. Crash до Pending, после Pending/до delete и после
delete/до final CAS возобновляется из exact state; `CommitUncertain`/mismatch
retains Pending and fail-stops. TaskStore не lazy-delete-ит v5 records.
Истекшие records удаляются
до отказа по capacity; live payload/link не вытесняется. Переполнение
отклоняет valid submit до validation/prepare/execute и не занимает TaskStore slot.
Это private daemon protocol, а не публичный generic idempotency/resume API.

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

- native Tasks profile: ровно `unica.view`, `unica.apply`, `unica.find`,
  `unica.search`, `unica.check`, `unica.diff`, `unica.run`, `unica.docs`;
- compatibility profile: те же восемь плюс `unica.task.get`,
  `unica.task.result`, `unica.task.cancel`;
- старые публичные имена и `unica.runtime.job.*` не сохраняются как aliases;
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

В пределах bounded ReceiptLedger retention/deduplication horizon один
exact `ReceiptKey` имеет не более одной попытки `prepare`/`execute`;
после physical expiry вечная idempotency не обещается. Сервер сам
выбирает direct result или Task: current post-prepare `KnownLong` сразу materializes exact
actor-bound Task через `TaskHandoffActorBound`, остальные операции обязаны
либо завершиться напрямую, либо иметь durable receipt к абсолютной границе
7000 мс. Durable receipt начинается в `ReceiptLedger`, а не с первой
TaskStore-записи. Direct outcome до private ACK остаётся recovery result,
а Task projection к cutoff читает receipt-backed `TaskPromisedUnbound` с тем же
TaskId; TaskStore получает его только после ActorBound. Native и compatibility
projections читают одну Invocation.

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

Эти записи регулируют публикацию уже готового `PlannedApplyEffects`, но не его
вывод из cross-family batch. Этой новой границей владеет planned
`DEC.2026-08-28.REQUEST-LEVEL-APPLY-EFFECT-RECONCILIATION`; до W2a она не
является действующим инвариантом.

## Обнаруженные противоречия

### Текущий daemon protocol v3 и целевой v5

`DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE`,
`DEC.2026-08-24.NATIVE-TASK-PROJECTION-SLICE` и active
`CTR.WIRE.DAEMON-INVOCATION-PROTOCOL` согласованы на текущем protocol v3
и identity `unica-daemon-jsonl-3`. Это действующее состояние hidden
foundation, а не противоречие реестра.

Целевой ReceiptLedger меняет wire lifecycle и требует protocol v5.
W0a не расширяет active `ClientRequest`/`ServerResponse`,
`SafeFailureReason` или schema-v2 `StoredInvocationRecord`: distinct strict
`V5ClientRequest`/`V5ServerResponse`, `V5SafeFailureReason` и
`V5StoredInvocationRecord` живут side-by-side в distinct
`crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs`,
`crates/unica-coder/src/application/invocation_store_v5.rs` и
`crates/unica-coder/src/infrastructure/task_store_v5.rs`; v5 lifecycle и
composition дополнительно живут в distinct
`crates/unica-coder/src/application/invocation_v5.rs` и
`crates/unica-coder/src/infrastructure/daemon/runtime_v5.rs`, а projections —
в distinct `interfaces/task_projection_v5.rs`/`application/v13/task_tools_v5.rs`
(либо exact-equivalent versioned modules).
Current `application/invocation.rs`, `protocol.rs`, `client.rs`, v3 store
types/decoders и projectors не меняются. Перед v5 runtime private
`CanonicalInvocationService`, `ActorBoundInvocation`, `ActorBoundExecution` и
capability helpers semantic-neutral извлекаются из `daemon/server.rs` в shared
`daemon/invocation_service.rs`; v3 server получает narrow import, а
`v13_service.rs` меняет только import/impl path. V3 byte-level JSONL/serde и
behavior gates до/после extraction запрещают semantic drift, но literal-file
freeze `server.rs`/`v13_service.rs` больше не заявляется. `CoreIdentity` и state selector
параметризуются protocol identity, при этом default `production()` composition
остаётся v3. W0c переключает composition на v5, не переопределяя и не ослабляя
legacy decoder.
Existing `interfaces/daemon.rs --daemon` entry становится additive versioned
dispatch seam, но не новым CLI/test/env selector: он strict parse-ит уже
переданный `--core-identity`: только exact known
`CoreIdentity::production_v5()` выбирает distinct `runtime_v5::run_daemon`, а
каждый другой уже принимаемый canonical 64-hex CoreIdentity продолжает v3
`server::run_daemon`. Invalid syntax отклоняется прежним parser; arbitrary v3
fixture identities остаются accepted. V5 client spawn-ит тот же CLI с v5 CoreIdentity. Endpoint/path helper
принимает typed protocol identity вместо hardcoded v3. Default connect/
`production()` строит V3 в W0a/W0b; W0c меняет только default constructor на V5.
Executable acceptance проходит real v5 client → spawned `--daemon` → v5
endpoint/handshake и отдельно доказывает v3 default/decoder guard до W0c;
v3 service extraction остаётся byte/behavior neutral.
Поэтому W0c создаёт newly dated active successor/evidence, атомарно
заменяет владельцев v3 и выведенные records, и оставляет в реестре
ровно одну текущую wire identity. До этого v3 не выдаётся за
ReceiptLedger и S1 не достигнут.

### 71 имя против опубликованных 74

Удаление только 71 имени текущего `main` не доказывает миграцию опубликованной
v0.12.3. Release acceptance использует immutable fixture с 74 именами и явно
проверяет судьбу шести `unica.runtime.job.*`.

### Реализованный фундамент против production readiness

Hidden V13 tests доказывают отдельные slices, но production stdio остаётся V12,
а default daemon остаётся dormant. Поэтому Tasks 1–14 старого плана после W0
становятся frozen foundation baseline, но не доказательством публичного
cutover.

### Live receipt owner против durable receipt

Заранее выданные IDs, in-memory map и cancellation token закрывают
гонку внутри одного процесса, но не доказывают at-most-once после
restart или потери Direct response. TaskStore также не может заменить
receipt ledger: до cutoff Task ещё может не существовать, а успешный
Direct вообще не обязан становиться публичной Task. S1 поэтому
блокируется до отдельной durable state machine, её crash reconciliation и
bounded compaction.

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
  |-- private durable ReceiptLedger / dedupe horizon
  |-- durable TaskStore for materialized Tasks
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
| S1 | Зелёный и слитый hidden foundation: durable ReceiptLedger резервирует exact IDs/request identity до предметной работы; Unbound cutoff создаёт `TaskPromisedUnbound`, pre-actor-bind terminal сохраняется как `TaskTerminalReceiptBacked`, а promised и actor-bound/begun paths используют разные durable intents, receipt-owned Link Capacity fallback и exact TaskStore reconciliation; inspect-only TaskStore startup выбирает terminal reason только по receipt evidence до listener; terminal Direct/Task восстанавливаются без replay, bare Reserved recovery не изобретает Task, а crash после `Reserved::Begun` до terminal commit даёт distinct `V5SafeFailureReason::OutcomeUncertain`; все 61 named ReceiptLedger/protocol/identity/reserve/handoff/ACK/result-bounds/retention/cancel/restart/capacity cases зелены на macOS, Linux и Windows; newly dated active successor/evidence и derived active `INV.*`/`CTR.*` ссылаются на именованные проверки, а state machine, crash windows и capacity math прошли independent semantic review; публичный V12 неизменён |
| S2 | Все 8 handlers и принятый registry `apply` работают через daemon, schema-derived semantic parity и content-addressed provenance scope lock закрыты; состояние отзывается при drift |
| S3 | RC публикует ровно 8/11, legacy surface отсутствует |
| S4 | Stable v0.13 прошёл fresh/upgrade/rollback/offline и host matrix |

Ни S1, ни S2 не являются пользовательским релизом. Пользовательская граница
меняется только S2 → S3.

## Wavefront

### W0: stabilize and bound

PR #631 получает зелёные macOS, Linux и Windows gates. Integrator единолично
меняет receipt/protocol/store hotspots, один worker пишет только black-box RED
matrix, а два оставшихся слота независимо проверяют capacity/recovery/crash и
полную семантику. Windows/process implementation уже доставлен; adversarial
reviewer возвращается к его коду только если fresh `win-x64` CI воспроизведёт
дефект. Эти линии не конкурируют за production receipt files.

Black-box matrix использует non-default feature
`receipt-ledger-test-support`: opaque fixture предоставляет fake clocks, named
barriers/crash points, bounded faults и наблюдаемые snapshots/counters, но не
raw stores, forgeable actor proof или runtime selector. Сначала test file
compile-RED на отсутствующем contract. Третий 5 095-line snapshot
`eec6a2102ec6734b3522f8af3ddedebfababdb48d1ede2ffa9c8985ac2b6bb21`
сохранил 61 exact name и единственный compile `E0432`, но independent
semantic+concurrency review отклонил его: fixture принимала entitlement,
возвращала protocol/identity/crash verdict booleans вместо raw evidence, не
несла raw receipt/task IDs, terminal/first-ACK epochs, stable Task fields и
typed failure reason и не доказывала post-attempt protocol event. RED-authoring
gate остаётся открытым; declaration bridge начинается только после repaired
snapshot и повторного approval.

Repaired scenario input содержит только clock mode и primitive actions, без
expected result и без `totalEntitlementBytesEach`. Fixture возвращает raw safe
TaskId/InvocationId, key components/fingerprints, post-handshake event,
terminal/first-ACK epochs, Task `createdAt`/`updatedAt`/`ttlMs`/`pollIntervalMs`,
store/projection records, counters и digests; test сам вычисляет equality,
round-trip, split-brain, staged-winner, accounting и high-water verdicts.

После фиксации единственного compile `E0432`, но до root facade, integrator
создаёт минимальный side-by-side production v5 reachability shell. Application
получает checked `RequestScopeHash`/`ReceiptKeyDigest`/`TaskLinkDigest`,
`V5CanonicalTerminal` и единственные hash/framing/canonical-terminal authorities.
Production `protocol_v5` реально читает bounded frame и strict-decode-ит/probe-ит
v5 envelope; `runtime_v5` входит через exact production-v5 CoreIdentity,
использует shared `CanonicalInvocationService` и делает реальный
ReceiptLedger open/generation/exact-inspect step. Произвольные accepted 64-hex
identities и default composition остаются v3 до W0c; нового CLI/env/MCP/test
selector нет.

Эти production protocol/runtime/store sites, и только они, mint-ят sealed
feature-gated `ReachedProductionBoundary`/`ProductionMissingTransitionEvidence`
после фактически выполненного step. Root facade имеет только read-only access и
не может конструировать evidence. Static guard запрещает в facade hash/framing,
canonical-terminal codec и любой `ActionKind -> boundary/code/event` switch.
Лишь после этого thin `receipt_ledger_test_support` strict-parse-ит inputs,
вызывает application authorities, routes primitive actions в typed production
operations и сериализует их sealed evidence с action index/kind как correlation.
Default/package build facade не компилирует.

Затем feature-enabled harness даёт ровно 61 functional-RED scenario, а
обязательный five-test smoke set достигает реального daemon/store. Ни один RED
не может происходить из fixture echo, `todo!`, setup timeout или facade-owned
boundary classifier: missing transition возвращает фактически достигнутая
production operation. W0a закрывает
identity/state/actor/store/reserve filters и только
ReceiptLedger-local live/link/tombstone accounting, segments и horizon.
Link Capacity/post-reservation invariant branches, 4 097th Task boundary,
ACK/recovery/handoff
остаются RED до W0b. Поэтому W0a — reviewed TDD checkpoint, а не отдельное
mergeable состояние. Side-by-side v5 доступен через injected hidden path;
default private v3 меняется на v5 только в W0c одновременно с active successor
и derived records.

До S1 private durable `ReceiptLedger` атомарно резервирует exact
InvocationId/TaskId/request identity на valid-submit boundary и затем один
live owner владеет admission/prepare/execute. Direct result durable до response
и private ACK; потеря transport при живом daemon или restart после durable
terminal commit возвращают тот же result без replay. Crash после
`Reserved::Begun`, но до terminal commit возвращает distinct
`V5SafeFailureReason::OutcomeUncertain`,
а не вымышленный result и не replay. Без committed promise/handoff bare
`Reserved::Unbound/ActorBound` recovery остаётся Direct
cancelled/interrupted-before-execution, а bare `Reserved::Begun` — Direct
`outcome_uncertain`; только committed Task intent разрешает Task projection.
Cutoff внутри pre-actor-bind
validation/admission материализует exact receipt-backed
`TaskPromisedUnbound`; зависший pre-Begun pipeline после fixed grace ведёт к
process fail-stop и не может поздно победить terminal. Current
`ExecutionClass::KnownLong` и cutoff во время `prepare`/`execute` уже имеют
actor-derived identity и `Reserved::Begun`, поэтому используют
`TaskHandoffActorBound` и пытаются exact TaskStore create/readback;
proven Link Capacity переводит begun handoff в receipt-owned branch без TaskStore
record. TaskStore никогда не получает
workspace-hint substitute. В `TaskPromisedUnbound` cancel/restart до actor bind
terminalizes receipt-backed Task без callback и TaskStore. После ActorBound,
но до `TaskBound`, durable cancel flag остаётся в ReceiptLedger intent;
только `BoundStartCancelGate` + exact TaskStore create/identity/flag readback +
`TaskBound` переносят authority в TaskStore.
Closed task-publication Capacity (`LinkCapacity`) под тем же continuously-held
gate не затирает staged/cancel winner. Staged Link evidence не имеет reservation
и использует certified receipt-backed winner. Без winner pre-Begun даёт
receipt-backed `task_capacity`, а post-Begun — `TaskReceiptOwnedActorBound`,
который не повторяет create и хранит actual/uncertain terminal в ledger. При restart protocol-v5 сначала
открывает TaskStore inspect-only; ReceiptLedger-led coordinator различает
Queued/Working по `begun` evidence и завершает reconciliation до listener.
Cancel-before-submit, 32 simultaneous submit/cancel pairs, restart каждой
durable фазы, uncertain promised-to-TaskStore promotion, exact duplicate, each
of six identity-component mismatches, ACK
compaction, separate storage/byte caps, dominant link-count admission и retention доказываются
barrier/crash-injection + fake-clock тестами. Active protocol-v5 `Queued`/`Working`
после restart не terminalize-ится и не удаляется наугад: Task
без exact receipt/link evidence является corruption, поэтому startup fail-stop-ится
до mutation и listener publication.
Тот же W0 gate отдельно покрывает strict `responseBudgetMs > 7000`, oversized
workspaceHint и terminal algebra по current durable owner (Direct,
pre-TaskBound ledger, receipt-owned begun Link Capacity или TaskStore),
premature/lost/full-pool ACK после final Completed `CallToolResult` либо
Failed/Cancelled `ErrorData`, native и
compatibility cancel для каждого closed state, orphan Queued и orphan Working,
Task-create `CommitUncertain`, exact accounting/reopen/overflow и tombstone
high-water из first-ACK epochs. Эти integrity/capacity failures не превращают
каждый terminal status в fail-stop: semantic `DomainResult.ok=false` остаётся
обычным Completed, а proven capacity остаётся typed backpressure.

В той же волне закрываются остальные defect failures по TDD, устраняется
daemon protocol contradiction, фиксируется internal SPI family planners,
validation и canonical result. Только W0c implementation commit создаёт newly dated
active successor к `DEC.2026-08-28.DAEMON-RECEIPT-LEDGER` с realized evidence,
`establishes` и derived active `INV.*`/`CTR.*`; planned predecessor получает
только lifecycle stamp.
Request-level apply router не считается замороженным, пока W2a не докажет
глобальные индексы, порядок и effects от финального postimage. Публичный V12 не
меняется.

### W0.5: semantic baseline and provenance scope lock

Сразу после S1 три workers параллельно характеризуют все 74 публичных имени и
все capability variants внутри их argument schemas в immutable package/tag
v0.12.3, а также 41 затронутый live-upstream skill entry, пока integrator
выполняет W2a-core. Raw published `tools/list` сохраняется отдельно от sidecar с
payload/package digest, а детерминированный extractor порождает catalog
behavior discriminators. Он не строит слепой cross-product всех value enums:
каждый selector и reachable combination связывается со schema pointer,
immutable V12 handler branch и executable probe. Если schema не перечисляет
behavior-bearing selector, W0.5 блокируется до такой reviewed rule. Единица
legacy oracle — `(legacyTool, legacyVariant)`: один
семейный V12 tool (`runtime.execute`, `meta.edit`, `dcs.edit` и другие) может
иметь несколько разных successors или removal. Единственный integrator-owned
capability oracle фиксирует для каждого variant допустимый typed
successor/projection либо исполняемое rejection evidence, exact legacy request,
immutable fixture, normalized legacy observation и reviewer. Действительно
новая V13 capability записывается отдельно и не приписывается произвольному
legacy имени ради coverage. Catalog и oracle обязаны иметь точно одинаковое
множество legacy identities. V13 shard не может сам объявить свою семантику
правильной: Python shape validation остаётся необходимой, но S2 получается
только сравнением V13 execution с этим независимо снятым oracle. Каждая из 13
`run` operations имеет хотя бы один executable case — legacy-derived или явно
new.

В той же волне immutable upstream review классифицирует каждый затронутый skill
как routing/prose, bundled-tool или product behavior. Последние два класса
сразу добавляются в toolchain либо W1/W3 scope и меняют оценку до fan-out;
возврат W4 → W1/W3 запрещён. Integrator один владеет capability oracle,
provenance review и expected index dispositions, workers передают ему disjoint
evidence. W1 открывается только после join W0.5 и W2a-core. Task 11 превращает
этот scope в tracked patch/index artifacts; до G6 они остаются staged, поэтому
live `upstreamDrift=true` допустим; clean
`affectedEntries: []` доказывается только после atomic application в G6.

### W2a-core и W2a-seams

Параллельно W0.5 integrator закрывает W2a-core до первого W1 merge. Router
парсит request один раз, сохраняет исходный `ops[i]`, передаёт XDTO, Code и
Event только через их admission-sealed authorities и выводит domain events из
финального postimage всего request, а не суммирует промежуточные singleton
результаты. После aggregate тестов на inverse operations, interleaved families,
global error index и poison rollback family SPI считается frozen и W1 fan-out
начинается. Независимый integrator-owned W2a-seams стартует из обновлённого
`main` параллельно W1, компилирует стабильные W3 adapters/validation view и
обязан слиться до первого W3 slice. W1 не блокируется на не относящихся к нему
W3 facade-файлах.

Validation seam заранее делится по файлам, иначе W2b и W3B имеют один
непараллелимый hotspot. W2a-seams создаёт
`infrastructure/v13_validation/mod.rs`, `apply.rs` и `check.rs`. После merge
`mod.rs` остаётся frozen integrator-owned registry/facade, `apply.rs` остаётся
integrator-only staged adapter для W2b, а exclusive write ownership `check.rs`
переходит Worker B для persisted adapter. Любая поздняя смена общего интерфейса
останавливает обе линии и проходит отдельным integrator seam PR из свежего
`main`, а не параллельными правками.

Effect finalizer сначала отбрасывает path-bound candidates без изменения в
финальном postimage и только затем выполняет stable first-surviving-occurrence
dedup по planned
`DEC.2026-08-28.REQUEST-LEVEL-APPLY-EFFECT-RECONCILIATION`. Обратный порядок
ошибочен: transient первый duplicate не должен поглотить surviving второй.
Active retained-publication decision применяется уже к полученному
`PlannedApplyEffects` и не подменяет эту будущую норму.

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
router, а не возвращает singleton dispatch. Для validation W2b меняет только
`v13_validation/apply.rs`; frozen `mod.rs` и Worker-B-owned `check.rs` остаются
read-only.

### W3: remaining entry points

После завершения собственной apply-линии каждый worker сразу переходит к
`search` + `docs`, `check` + `diff` или `run`, не ожидая две другие линии.
Исключение — B8 (`apply(dryRun)` parity): он ждёт финальную интеграцию apply в
W2b, читает финальный `v13_validation/apply.rs` и владеет только отдельным
equality test/fixture. B7 пишет persisted adapter только в `check.rs`, B9/B10 —
diff files; ни один из них не со-редактирует integrator validation files.
Integrator один регистрирует handlers в daemon и MCP shared files.

### W4: continuous parity and skills

Parity matrix создаётся в W0 и заполняется каждым vertical slice. В конце
остаётся aggregate gate, а не поздняя отдельная wiring task. Каждый
`mapped`/`absorbed` `(legacyTool, legacyVariant)` и каждая явно новая V13
capability владеют непустым набором уникальных `caseId` с точной
`(entry, operation)` identity; fixture-driven Rust runner связывает legacy cases
с reviewed W0.5 request/observation, исполняет approved V13 successor через
hidden canonical handler и сравнивает typed result или side effect.
Transport/rejection evidence также исполняется, а new capability доказывается
без вымышленного legacy predecessor. `complete: true` в Python доказывает только
structural closure и не является S2 без semantic runner. Параллельно три workers
готовят migration mapping, fixtures и
четыре tracked patch artifacts для 73 skills и provenance index по уже
замороженной W0.5
классификации. Владелец каждого skill фиксируется в manifest; распределение
строится детерминированным LPT по размеру отслеживаемого `SKILL.md`, а не по
неравным буквенным диапазонам. Эти patches не сливаются в ветку с
package-selected V12 и вместе с единственным integrator-owned provenance index
delta применяются только внутри atomic G6. Content-addressed manifest связывает
review path/digest, upstream target/diff, множество per-entry решений, точно
равное reviewed `affectedEntries` (41 — только исходный снимок), patch/index
bytes, base blobs и expected result blobs. Validator применяет их в чистом
временном дереве; applied-tree mode требует свежие `upstreamDrift=false` и
`affectedEntries=[]`, поэтому local commit SHA, exit code текущего checker и
`--validate-only` index не являются evidence. Изменение upstream target
отзывает S2 и возвращает работу в
новый immutable review; bundled-tool/product gap дополнительно возвращается в
toolchain/W1/W3, после чего W4/W5 повторяются.

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
- `crates/unica-coder/src/infrastructure/v13_validation/mod.rs` и
  `crates/unica-coder/src/infrastructure/v13_validation/apply.rs`;
- `crates/unica-coder/src/infrastructure/workspace_actor.rs`;
- `crates/unica-coder/src/infrastructure/daemon/mod.rs`;
- `crates/unica-coder/src/infrastructure/daemon/invocation_service.rs`;
- `crates/unica-coder/src/infrastructure/daemon/runtime_v5.rs`;
- `crates/unica-coder/src/infrastructure/daemon/terminal_codec_v5.rs`;
- `crates/unica-coder/src/infrastructure/daemon/server.rs`;
- `crates/unica-coder/src/infrastructure/daemon/v13_service.rs`;
- `crates/unica-coder/src/interfaces/daemon.rs` как closed version dispatch seam;
- `crates/unica-coder/src/interfaces/mcp.rs`;
- `crates/unica-coder/src/interfaces/task_projection.rs`;
- surface/version manifests, architecture registry, capability oracle,
  provenance review/index delta и aggregate tests.

W2a-seams единолично создаёт весь `v13_validation/` directory и компилирует
его facade. После merge ownership становится file-disjoint: Worker B получает
только `check.rs` и dedicated check/diff/equality tests; `mod.rs` и `apply.rs`
навсегда остаются integrator-owned. Это time-phased handoff, а не разрешение
двум веткам одновременно менять directory registry.

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
читающий типизированные логические узлы. Infrastructure реализует два adapter в
разных owned files: integrator — staged `ApplyStagedState` в
`v13_validation/apply.rs`, Worker B после W2a handoff — persisted actor snapshot
в `v13_validation/check.rs`. B8 сравнивает их отдельным equality test, читая
apply adapter без правки. Поэтому domain не зависит от infrastructure, а
конкретный validator получает view и не открывает filesystem самостоятельно.

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
- legacy disposition/expected result не подтверждается immutable reviewed v0.12.3 oracle;
- parity требует сохранить старую публичную schema или alias;
- upstream target меняется после W0.5 либо review открывает неоценённый tool/behavior gap; достигнутый S2 отзывается, owning slice и W4/W5 повторяются до G6;
- recovery требует raw args, secrets, commands или blind replay mutation;
- valid submit достигает validation, admission, `prepare` или `execute` до
  durable reservation exact InvocationId/TaskId/request identity;
- frontend/server/fixture расходятся в application-owned `RequestScopeHash`
  vector либо применяют filesystem/case normalization к `workspaceHint`;
- ledger/TaskStore/runtime/codec/fixture расходятся в application-owned
  `TaskLinkDigest` vector, включают mutable Task version/status/cancel/timestamp
  либо ephemeral actor generation;
- v5 wire/task/store принимает unknown/defaulted field, меняет exact
  tag/variant/field spelling или `schemaVersion:1`, переиспользует open
  `InvocationFailure` либо сворачивает `OutcomeUncertain` в `Interrupted`;
- direct outcome пишется в transport до durable terminal commit или ledger
  теряет его до private ACK;
- terminal path не использует owner-specific linear prepared receipt/task/wire
  pieces с independent receipt/task expected versions и exact link digest,
  persists full response frame рядом с canonical payload, reserialize-ит после
  конкретного frame preflight либо ACK-ит Failed/Cancelled до готовой immutable
  ErrorData projection;
- first staged terminal mutation не имеет codec-built persisted
  `StagedTerminalTransferSizeCertificate`, bound к protocol/CoreIdentity/key/task/
  link/terminal/schema/limits, либо certificate не покрывает conservative final
  Task/`TaskTerminalBound`/wire и proven-Link-Capacity fallback maxima; late path
  превышает bound, меняет winner на `ResultTooLarge` или принимает mismatch;
- cutoff внутри Unbound validation/admission не проецирует receipt-backed
  `TaskPromisedUnbound`, current post-prepare `KnownLong` ошибочно возвращается
  в unbound state либо TaskStore получает request-scope identity до ActorBound;
- rejection/cancel/restart обещанной unbound Task не сохраняет exact canonical
  `TaskTerminalReceiptBacked` на один час, Direct ACK удаляет этот payload,
  либо зависший validation/admission переживает общий two-second fail-stop
  grace и поздно достигает bind/callback;
- promised promotion или actor-bound/begun `TaskHandoffActorBound` оставляет
  split-brain между ReceiptLedger и TaskStore, теряет staged terminal либо
  разрешает новый domain callback при reconciliation; staged winner после
  provisional create не CAS-terminalize-ит exact Queued/Working
  TaskId/InvocationId/version/cancel/link-digest record, принимает foreign/
  mismatch, требует create-only path либо публикует
  промежуточный `TaskBound`;
- TaskStore create происходит до durable Task-link count/byte reservation,
  post-reservation Capacity превращается в terminal/fallback или освобождает
  reservation, либо `CommitUncertain` слепо retry-ит create;
- recovery materializes link для existing Task без exact retained handoff intent
  и matching live `TaskLinkReservation`, тем самым reconstruct-ит entitlement
  или обходит boundary 4096;
- v5 TaskStore lazy-delete-ит terminal/live Task по TTL без committed
  `TaskRetirementPending`, resolver не становится expired на Pending commit,
  reopen не mint-ит fresh coordinator-only authorization из exact Pending/current
  link version либо принимает old-process token, absent Task принимается без
  exact Pending proof либо final cleanup протекает мимо sole lifecycle-link/
  dual-ID accounting;
- protocol-v5 TaskStore open eager-terminalize-ит `Queued`/`Working` до
  receipt-led reconciliation, legacy orphan pass касается v5-linked Task либо
  listener публикуется до `RecoveryComplete`;
- `TaskStoreCapacityInvariantViolation` затирает staged/cancel winner,
  release-ит reservation, публикует terminal/fallback или оставляет listener
  доступным вместо fail-stop; либо proven Link Capacity повторяет create после
  `Begun`/не terminalize-ит `TaskReceiptOwnedActorBound` как receipt-backed
  actual/`outcome_uncertain`;
- Direct start/cancel, cancel-authority transfer при `bind_task` и bound Task
  start/cancel не линеаризуются одним live per-invocation
  `BoundStartCancelGate`, либо cancel может durable выиграть в окне между
  Working readback и receipt `begun`;
- `TaskBound { begun:false }` использует separate cancel false-read/write-
  Working вместо sole-writer atomic
  `start_working_if_not_cancel_requested`, вызывает prepare до exact
  versioned Working readback + receipt begun, resolver показывает Working до
  receipt begun commit или recovery не различает Working+begun=false,
  Working+begun=true и invalid Queued+begun=true;
- `bind_actor`/`bind_promised_actor` не принимают application
  `ActorBindingClaim` и не возвращают committed receipt + one-shot actor token;
  `mark_reserved_begun`/`authorize_bound_task_start` не consume-ят exact token,
  либо `mark_bound_task_begun` не consume-ит единообразный
  `PostWorkingActorAuthorization` после Working readback;
- actor authorization передаётся fixture boolean, token можно построить из
  persisted hash, application импортирует infrastructure, либо private runtime
  coordinator не владеет actual verifier/gate/token/lease до terminal;
- cancel flag в promised/handoff intent не durable до TaskStore/token,
  `bind_task` не удерживает тот же gate до exact identity/flag readback либо begun
  handoff после crash объявляется `cancelled` вместо `outcome_uncertain`;
- crash после `Reserved::Begun` приводит к replay или выдуманному known outcome
  вместо closed `outcome_uncertain`;
- recovery из bare `Reserved::Unbound/ActorBound/Begun` без committed
  promise/handoff задним числом создаёт Task вместо exact Direct terminal;
- capacity/retention ReceiptLedger зависит от TaskStore, вытесняет
  live payload/link, считает `CancelReserved` отдельным скрытым +64 pool,
  освобождает promised/handoff result reserve до одного из четырёх
  durable release events, не делает physical delete unacked Direct ровно
  через час либо не восстанавливает лимиты после restart;
- provider sharing смешивает actor, root или revision identity;
- проверка 7000 мс требует real sleep вместо fake clock;
- worker вынужден править shared integration file;
- hidden v5 lifecycle реализован branches внутри current `invocation.rs` или
  v3 server вместо distinct runtime/executor, semantic-neutral shared
  `daemon/invocation_service.rs` extraction меняет v3 byte/behavior, либо v5
  дублирует private service/capability logic;
- `interfaces/daemon.rs` hardcode-ит v3 для v5 client spawn, направляет exact
  production-v5 identity в v3, перестаёт принимать другой ранее valid canonical
  64-hex v3 fixture identity, использует CLI/env/test selector помимо existing
  `--core-identity`, либо endpoint helper не параметризован closed typed
  `DaemonProtocolIdentity`;
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
- публичный generic idempotency/resume protocol; private ReceiptLedger,
  exact duplicate reconciliation, ACK/tombstone и pre-submit cancellation входят в W0,
  но не расширяют `tools/list` или public Tasks API;
- `tests`, `features`, `log` как отдельные tools;
- gRPC/platform 8.5 profile;
- дополнительная оптимизация удаляемого V12 surface.

## Оценка

После подтверждённого durable-receipt blocker и исправления
variant-level/provenance cardinality current scoped base estimate остатка
после delivered `e870d810`/`f66d5948` составляет 102–162 person-days.
Это не upper cap: upstream move либо новый bundled-tool/product gap
требуют explicit re-estimate до fan-out. Завершённые
parity-inventory и Windows/process person-days повторно в эту сумму не входят.
W0 увеличен с 6–10 до 14–24 person-days: добавлены separate durable store,
receipt-backed unbound/terminal Task projection, actor-bound recoverable
handoff reconciliation, inspect-only receipt-led startup, receipt-owned
Link-Capacity fallback, post-reservation capacity invariant, private
ACK/compaction и crash/capacity matrix. Эти
две seam заменяют unsafe recovery/backpressure внутри того же W0, а не
добавляют новую public slice. Сумма
task-level packages равна ровно 102–162 person-days; скрытого contingency сверх
таблицы нет. При
одном integrator и трёх workers реалистичный срок до stable составляет
9–13 недель, из них W0 занимает 8–12 рабочих дней на critical path.
Прежние 2–3 дня на W0 и
шестинедельная оптимистичная граница больше не используются как обязательство.

Критический путь начинается как W0a planned ReceiptLedger contract/RED/store →
W0b unbound Task projection + promised/actor-bound recoverable handoff + Direct ACK/recovery +
cancel/restart/capacity integration →
W0c active successor/evidence + aggregate и independent semantic review →
max(W0.5a worker evidence, W2a-core) →
W0.5b integrator consolidation и два независимых review. Только после этого W1
освобождает join; W2a-seams выполняется параллельно и
блокирует только первый W3 slice. Это сокращает ненужный общий барьер, но не
отменяет последовательные gates W0, aggregate parity, provenance, G6/G7 и RC
soak. Точность оценки medium-low до классификации 41
затронутого skill entry: если live review требует не prose routing, а
behavior/tool port, он включается в scope и оценивается до fan-out, а не
возвращается из W4 задним числом.

## Артефакты исполнения

Эту записку реализует
`docs/plans/2026-08-28-v0-13-completion.md`. После его принятия umbrella issue
#581 хранит только живую wave/gate/owner картину и ссылку на новый план; старый
phase ledger остаётся датированной историей и не используется как текущий
backlog.
