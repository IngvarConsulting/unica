- Date: `2026-08-28`
- Status: `approved`
- Decision: `DEC.2026-08-28.DAEMON-RECEIPT-LEDGER`

# Durable ReceiptLedger для private daemon invocation

## Контекст и найденное противоречие

Текущий daemon заранее получает frontend-выделенные `InvocationId` и
`reservedTaskId`, но до materialization Task держит receipt lifecycle в памяти.
TaskStore доказывает состояние только после создания Task. Поэтому потеря
direct response либо смерть daemon до Task handoff не имеет restart-stable
источника, по которому можно отличить исходный вызов от positive-budget replay.

Фраза «exactly-once execution/outcome» для произвольного provider некорректна.
Daemon может атомарно записать свой receipt, а provider — отдельно изменить
файл, запустить процесс или выполнить внешний side effect. Без общей транзакции
между provider commit и receipt terminalization существует окно:

1. side effect уже committed;
2. процесс погиб до durable terminal receipt;
3. после restart неизвестно, завершилось ли предметное действие.

Повтор в этом окне может удвоить side effect, а объявление успеха может его
выдумать. Поэтому целевой договор — bounded at-most-once attempt плюс durable
`begun`/terminal evidence. Неразрешимое окно закрывается outcome `uncertain`, а
не replay или ложным success. Exactly-once допустим только для конкретного
provider, который позже свяжет предметный commit и receipt одной доказанной
транзакцией; общий daemon такой связи не предполагает.

## Цели

- зарезервировать exact receipt до validation/admission/prepare/execute;
- пережить restart до появления Task и восстановить lost direct response;
- не допустить второго execution из-за retry, ACK loss или нового budget;
- оставить ReceiptLedger владельцем lifecycle/result через `ActorBound`,
  promised и handoff до exact TaskStore create/readback + commit `TaskBound`;
  только после этого TaskStore становится владельцем Task state/result;
- сделать cutoff handoff восстанавливаемым при любом crash между двумя stores;
- ограничить records, bytes, retention и работу recovery;
- сохранить публичные V12, 8-tool native и 11-tool compatibility поверхности;
- дать тестируемый честный ответ для недоказуемого terminal side effect.

## Не цели

- exactly-once доставка ответа от stdio frontend до MCP host;
- вечная дедупликация без bounded retention horizon;
- публичные idempotency keys, receipt API, generic resume или replay;
- объединение ReceiptLedger и TaskStore;
- progress/log streaming или новый публичный MCP tool;
- автоматическое предметное reconciliation неизвестного side effect.

## Выбранная граница

ReceiptLedger и TaskStore имеют разные обязанности:

| Компонент | Владеет | Не владеет |
| --- | --- | --- |
| ReceiptLedger | exact receipt key, pre-admission request identity, durable actor binding, original lifecycle и cancel authority; в ordinary path — до commit `TaskBound`, в staged-terminal path — до exact terminal TaskStore readback и direct commit `TaskTerminalBound`; Direct ACK, deduplication, promised/handoff Task projection, canonical receipt-backed/staged terminal payload, handoff intent, compact terminal evidence | progress, cancellation после ordinary `TaskBound` или копией task result после completed ownership transfer |
| TaskStore (`InvocationStore` в текущем коде) | provisional exact actor-bound record во время ordinary handoff; crash-safe staged-terminal safety copy до direct `TaskTerminalBound`; после ordinary `TaskBound` либо staged `TaskTerminalBound` — sole-owned terminal/`Queued`/`Working` Task, timestamps, TTL, canonical result, closed failure reason и durable `cancelRequested` | unbound promised Task, pre-Task retry, direct ACK, pre-receipt cancellation |
| Live executor | monotonic cutoff capability, per-invocation start/cancel gate, cancellation token, actor/resource leases, один attempt | restart-stable identity или единственным доказательством terminal state |

Оба store принадлежат private versioned daemon и имеют разных sole-writer
actors. ReceiptLedger открывается из sibling-каталога `receipts`, TaskStore — из
существующего `tasks`. Зависание любого writer переводит daemon в тот же
process-owned fail-stop; второй execution не запускается.

## Exact identity

`ReceiptKey` состоит из:

- canonical UUIDv4 `InvocationId`;
- canonical UUIDv4 `reservedTaskId`;
- `RequestIdentity`:
  - application-owned checked 64-hex `CoreIdentityDigest` текущих ABI и
    private protocol;
  - закрытый `ToolIdentity` одной из восьми canonical operations;
  - `NormalizedArgumentsHash` canonical JSON arguments;
  - application-owned checked `RequestScopeHash`, вычисленный из exact
    strict-parsed `workspaceHint`.

Response budget, monotonic deadline, cancel timing и transport retry count не
входят в identity: повтор не может получить новый lifecycle изменением budget.
Client и server используют одну canonicalization функцию; wire test сравнивает
их byte-for-byte `ReceiptKey` и отдельно доказывает mismatch каждого поля.
Application определяет и валидирует `CoreIdentityDigest`; infrastructure
`CoreIdentity` оборачивает этот checked value и отдаёт его через typed digest
accessor. Application code не импортирует infrastructure identity type.

`ReceiptKeyDigest` имеет одну application-owned authority и ровно одну
каноническую encoding: SHA-256 сначала получает ASCII domain
`unica.receipt-key.v1\0`, затем для каждого normalized component — four-byte
unsigned big-endian byte length и сами bytes. Порядок фиксирован:
canonical `InvocationId`, canonical `reservedTaskId`, `CoreIdentityDigest`,
canonical tool wire name, lowercase `NormalizedArgumentsHash`, lowercase
`RequestScopeHash`. Client, daemon, ReceiptLedger и recovery
вызывают эту authority; ни infrastructure adapter, ни fixture не реализует
второй framing/hash algorithm.

`RequestScopeHash` не является alias actor-derived `SafeIdentityHash` и имеет
отдельную application-owned authority `request_scope_hash(workspace_hint)`.
После strict UTF-8 parse и проверок non-empty/control/size она вычисляет SHA-256
над ASCII domain `unica.request-scope.v1\0`, затем four-byte unsigned big-endian
длиной exact UTF-8 bytes `workspaceHint`, затем самими bytes. Никакой filesystem
canonicalization, separator normalization, symlink resolution, Unicode/case
folding или trim до hash нет. Frozen vector: `workspaceHint = "workspace-a"`,
input hex
`756e6963612e726571756573742d73636f70652e7631000000000b776f726b73706163652d61`,
result lowercase hex
`9f7a5a77bb6eb469cd20147a9aeee9d9769a8372f587bd89635d15684ee02b39`.
Frontend, daemon/server и fixture вызывают одну эту helper; fixture не имеет
второй реализации и проверяет frozen vector через production export.

`TaskLinkDigest` — отдельный application-owned checked 64-hex type и единственная
authority `task_link_digest(...)`. SHA-256 получает ASCII domain
`unica.task-link.v1\0`, затем ровно четыре normalized components, каждый как
u32-BE byte length + bytes, в порядке: lowercase 64-hex `ReceiptKeyDigest`,
canonical TaskId UUID text, canonical InvocationId UUID text, lowercase 64-hex
actor-derived `workspaceIdentityHash`. Mutable Task status/version,
`cancelRequested`, timestamps, terminal outcome и ephemeral actor generation в
digest не входят, поэтому это link identity, не Task-record digest, и она
стабильна на всём lifecycle. Ledger, TaskStore, runtime recovery и terminal
codec получают typed digest только из application helper; fixture вызывает его
production export. Frozen vector для ReceiptKeyDigest из 64
`0`, TaskId `11111111-1111-4111-8111-111111111111`, InvocationId
`22222222-2222-4222-8222-222222222222` и workspaceIdentityHash из 64 `a` даёт
`4c73d08219973c72e759a9f85e156fa42c9d8e61a56e704b70d1c7c042b73da0`.

Durable `OriginalCutoffDescriptor` содержит acceptance epoch и исходный
bounded response budget только как restart-stable evidence. Соответствующий
cutoff внутри живого процесса остаётся monotonic capability actor-а. После
restart daemon не преобразует wall time обратно в `Instant` и не выдаёт новый
budget: bare `Reserved` закрывается по durable phase/cancel evidence, а
committed promise/handoff/terminal следует собственному closed recovery rule.

Raw arguments, path, workspace hint, runtime text, stdout/stderr и свободный
failure text не сохраняются как identity/lifecycle metadata. Единственное
исключение — уже разрешённый публичным контрактом bounded canonical terminal
payload в `DirectTerminalUnacked`, `TaskTerminalReceiptBacked` либо временно
staged в `TaskHandoffActorBound`: он проходит тот же allowlist и 8 MiB + 64 KiB
limit, что direct response/TaskStore, и живёт только до ACK, TaskStore readback
либо Task TTL. Equality проверяет все identity-компоненты, а не
только один caller-controlled UUID. Exact совпадение идемпотентно; совпадение
только части ключа возвращает закрытый `invocation_identity_mismatch` и не
меняет исходный record.

`RequestScopeHash` не является workspace execution identity. После успешного
`WorkspaceActor` admission daemon получает actor-derived `SafeIdentityHash` из
retained actor capability и durable записывает
`ActorBound { boundWorkspaceIdentity }`. TaskStore принимает только этот
actor-derived hash. Подстановка request-scope hash в
`StoredInvocationRecord.workspaceIdentityHash` запрещена.

`SubmitInvocation` сначала проходит bounded JSONL read и строгий typed parse с
`deny_unknown_fields`, canonical UUID/tool проверкой и текущим request limit
16 KiB. После parse daemon сам вычисляет request identity из server
`CoreIdentity` и parsed tool/arguments/workspaceHint; caller-supplied digest не
принимается как authority. Затем выполняется durable reserve. Только после
подтверждённого reserve разрешены hidden-V13 validation, workspace binding,
service preparation и execution.

## Durable state machine

Schema v1 ReceiptLedger имеет следующие состояния:

| Состояние | Payload | Разрешённый следующий переход |
| --- | --- | --- |
| `CancelReserved` | full proposed key, `cancelReservedAt`, fixed `expiresAt`, `cancelRequested=true`; encoded metadata ≤ 1 KiB, без result reservation | exact submit атомарно резервирует result quota и становится `Reserved(cancelRequested=true)` либо expiry через 7125 мс |
| `Reserved` | key, `reservedAt`, original response cutoff descriptor, phase `Unbound`/`ActorBound { boundWorkspaceIdentity }`/`Begun { boundWorkspaceIdentity }`, `cancelRequested`, reserved result quota | из `Unbound`: `DirectTerminalUnacked`/`TaskPromisedUnbound`; из `ActorBound`/`Begun`: `DirectTerminalUnacked`/`TaskHandoffActorBound` |
| `DirectTerminalUnacked` | terminal epoch, terminal digest и ровно один v5 terminal payload: `Completed { DomainResult }`, `Failed { V5SafeFailureReason }` либо `Cancelled`; semantic rejection после valid submit остаётся `Completed` с `DomainResult.ok=false` | `AcknowledgedTombstone` либо physical deletion через один час от terminal epoch с освобождением live count/result quota |
| `AcknowledgedTombstone` | exact key, terminal digest и epoch первого committed ACK; без cutoff, original budget, result payload и nonessential lifecycle metadata | expiry через 15 минут от первого ACK |
| `TaskPromisedUnbound` | key, stable Task timestamps/TTL/poll interval, reserved result quota, `cancelRequested`; queued projection без workspace identity/result | `TaskPromisedActorBound`, `TaskTerminalReceiptBacked` |
| `TaskPromisedActorBound` | promised Task, actor-derived workspace identity, exact TaskStore-bind intent, reserved result quota, `cancelRequested`; queued projection, `begun=false` | `TaskBound`, `TaskTerminalReceiptBacked` |
| `TaskHandoffActorBound` | stable Task projection, actor-derived workspace identity, exact write-ahead handoff intent, `begun`, `cancelRequested`, reserved result quota и optional staged terminal payload | без staged terminal: `TaskBound` после exact nonterminal TaskStore readback; со staged terminal: TaskStore terminal write/readback и одна ledger mutation прямо в `TaskTerminalBound`, без промежуточного `TaskBound`; при proven Link Capacity staged terminal выигрывает, без него до `Begun` — `TaskTerminalReceiptBacked(Failed { V5SafeFailureReason::TaskCapacity })`, после `Begun` — `TaskReceiptOwnedActorBound` |
| `TaskReceiptOwnedActorBound` | stable Working Task projection, actor-derived identity, `begun=true`, `cancelRequested`, reserved result quota, latched proven `LinkCapacity`; link reservation/TaskStore create больше не повторяются | `TaskTerminalReceiptBacked` с actual outcome либо `Failed { V5SafeFailureReason::OutcomeUncertain }` после crash |
| `TaskTerminalReceiptBacked` | receipt-owned Task до commit `TaskBound`, terminal epoch/digest, stable Task identity/timestamps и ровно один v5 terminal payload: `Completed { DomainResult }`, `Failed { V5SafeFailureReason }` либо `Cancelled` | expiry через полный Task TTL от terminal epoch; repeated read без ACK |
| `TaskBound` | key, actor-derived workspace identity, exact Task record identity/digest, bind epoch, `begun`; TaskStore владеет durable cancel flag; при `begun=false` resolver нормализует projection как queued даже если TaskStore уже Working | после exact Working readback durable `begun=false→true`, затем `TaskTerminalBound` |
| `TaskTerminalBound` | key, Task identity/version, materialized link digest, closed terminal status, task outcome digest, terminal epoch, Task TTL и exact `expiresAt`; без копии result | только при `now >= expiresAt`: `TaskRetirementPending` до любого TaskStore delete |
| `TaskRetirementPending` | key/task/link, terminal digest/epoch/TTL/expiresAt, expected terminal Task version и retained materialized-link + dual-ID accounting; без result | после opaque-authorized exact TaskStore `Deleted` либо `AbsentExactWithPending` proof ledger CAS-delete-ит state/link/indexes; uncertainty/mismatch остаётся Pending и fail-stop-ит |

Один v5 terminal union применяется к Direct, receipt-backed Task и v5
TaskStore: `Completed { DomainResult }` / `Failed { V5SafeFailureReason }` /
`Cancelled`. Terminal owner определяется текущим durable состоянием, а не
историческим фактом promise. Semantic validation rejection и
`WorkspaceAdmissionError::Invalid` после strict valid submit и durable reserve,
но до actor bind являются предметным завершением hidden v5: в
`Reserved::Unbound` сохраняется `DirectTerminalUnacked(Completed {
DomainResult { ok:false, ... } })`, а в `TaskPromisedUnbound` —
`TaskTerminalReceiptBacked(Completed { тот же byte-equivalent DomainResult })`.
Все три `WorkspaceAdmissionError` возникают до `ActorBound`, поэтому
`Capacity` и `RegistryFailed` выбирают тот же Direct либо promised receipt owner:
первая durable пишет `Failed { V5SafeFailureReason::WorkspaceCapacity }` без
prepare/execute/restart, вторая — `Failed {
V5SafeFailureReason::WorkspaceRegistryFailed }`, затем `RestartRequested` и
fail-stop до следующего callback.

`service.prepare` вызывается только после committed `Begun`. Его semantic
rejection становится Direct лишь пока durable owner остаётся
`Reserved::Begun`. Если cutoff уже committed `TaskHandoffActorBound`, outcome
сначала stage-ится в handoff. Пока ledger сохраняет этот handoff, live
reservation и staged canonical payload, codec preflight-ит обе store pieces;
TaskStore при `Absent` атомарно создаёт terminal record, при
`ExactProvisional` CAS-terminalize-ит exact TaskId/InvocationId/Queued-or-Working/
version/cancel/link-digest record, а exact same
terminal idempotently readback-ит; любой foreign/mismatch fail-stop-ит. Только
после exact terminal readback одна ledger mutation consumes reservation, materializes link и переходит
прямо в `TaskTerminalBound`, удаляя staged payload. Промежуточный `TaskBound`
в этой ветке запрещён.
Если begun handoff получил proven Link Capacity и стал
`TaskReceiptOwnedActorBound`, тот же outcome terminalizes receipt-backed; если
`TaskBound` уже committed, любой prepare/execute terminal публикуется только в
TaskStore с последующим `TaskTerminalBound`. Promise сам по себе не разрешает
receipt-backed terminal после передачи ownership.
`Failed` содержит только закрытый `V5SafeFailureReason` и не содержит
`DomainResult`; `Cancelled` также не содержит `DomainResult` или failure text.
Digest-only terminal до TaskStore запрещён: canonical result либо typed reason
сохраняется byte-equivalent с теми же status/result/failure shape, stable Task
timestamps и size checks, что TaskStore.
Каждая Task projection несёт raw TaskId/InvocationId,
`createdAt`/`updatedAt`/`ttlMs`/`pollIntervalMs`; `createdAt`, TTL и poll interval
не меняются при promotion/reopen, `updatedAt` не регрессирует, а terminal
projection фиксирует `updatedAt == terminalEpoch`. Поэтому Task expiry
вычисляется от terminal epoch, не от create/read/checkpoint epoch.

Canonical terminal payload — minified UTF-8 JSON выбранного strict
`V5TerminalOutcome` с exact key order выбранного variant: `status`, затем
`result` для Completed либо `reason` для Failed; Cancelled содержит только
`status`. Поэтому bytes равны `{"status":"completed","result":...}`,
`{"status":"failed","reason":"<exact snake_case reason>"}` или
`{"status":"cancelled"}`. `TerminalDigest` вычисляет одна application-owned
authority как SHA-256 над ASCII domain `unica.terminal-outcome.v1\0`, u32-BE
длиной canonical payload и payload bytes. Writer не хеширует повторную
serialization и не принимает caller-supplied digest.
Frozen vector: canonical Cancelled payload `{"status":"cancelled"}` имеет 22
bytes; framed input hex равен
`756e6963612e7465726d696e616c2d6f7574636f6d652e763100000000167b22737461747573223a2263616e63656c6c6564227d`,
а `TerminalDigest` —
`f2d0423d2613a0d09397b750542e4542f7653d78ebd5e0448f1326d09145d9ae`.
Fixture вызывает production application export и не повторяет framing/hash.

Application владеет только typed `V5CanonicalTerminal`: exact outcome,
canonical payload и `TerminalDigest`, построенные одной application authority;
caller-supplied payload/digest не принимается. Application port объявляет
owner-specific linear preflight bundle types, но не сериализует receipt, Task
или wire envelope. Единственный versioned codec/coordinator
`infrastructure/daemon/terminal_codec_v5.rs` получает exact key, canonical
terminal, terminal epoch, owner-specific versions и durable owner и до первой
publication строит закрытые pieces: `PreparedReceiptRecord`, optional
`PreparedTaskRecord`, optional `PreparedTaskLifecycleLinkRecord` и
`PreparedWireFrame`. Каждая piece opaque, exact-bound к
ReceiptKeyDigest/outcome/terminal epoch/owner, содержит byte count и SHA-256;
Direct/ReceiptTask bundle несёт exact `receiptExpectedVersion`, а BoundTask —
independent `taskExpectedVersion`, `lifecycleLinkExpectedVersion` и exact link
digest. До materialization staged bundle вместо link version несёт active
`receiptExpectedVersion` и reservation version; codec строит новый sole link
record как часть commit.
Closed bundle algebra exact: `Direct|ReceiptBackedTask` содержит только
`PreparedReceiptRecord + PreparedWireFrame`; `BoundTaskStore` — только
`PreparedTaskRecord + PreparedTaskLifecycleLinkRecord + PreparedWireFrame`;
`HandoffStage` — только `PreparedStagedReceiptRecord +
StagedTerminalTransferSizeCertificate`; `StagedHandoffTask` — только
`PreparedTaskRecord + PreparedTaskLifecycleLinkRecord + PreparedWireFrame` и
атомарно replaces active receipt+reservation sole link record-ом;
`StagedCapacityFallback` — только `PreparedReceiptRecord + PreparedWireFrame`.
Cross-owner piece presence и PreparedReceiptRecord в двух TaskStore-owned
variants являются type/schema error.
Первую durable публикацию staged outcome разрешает только HandoffStage bundle.
До ledger mutation sole codec строит exact size/hash/CAS-preflighted
`PreparedStagedReceiptRecord` и typed `StagedTerminalTransferSizeCertificate`.
Certificate exact-bound к protocol-v5/CoreIdentity, ReceiptKey/Task identity/link
digest, terminal digest+epoch и frozen schema/limit versions. Он preflight-ит
staged receipt bytes и консервативно доказывает upper bounds для final terminal
Task record, sole `TaskTerminalBound` lifecycle-link record bytes, v5 Task wire
frame и receipt-backed staged-winner fallback
`TaskTerminalReceiptBacked` record+wire для proven LinkCapacity без reservation.
Maxima охватывают `Absent` и каждый
`ExactProvisional` с `Queued|Working`, обеими cancel booleans, максимальной
десятичной шириной `u64` version/epochs и завершающим JSONL `\n`. Certificate
evidence входит в preflighted staged record, сохраняется без wire frame и после
reopen возвращается sole codec как checked opaque type; raw `outcome`, bounds или
certificate ledger writer не принимает.
Persisted certificate является strict `deny_unknown_fields` record с exact
`certificateVersion:1,protocolIdentity:"v5",coreIdentityDigest,
receiptKeyDigest,taskId,invocationId,taskLinkDigest,terminalDigest,
terminalEpochMs,receiptRecordSchemaVersion:1,taskRecordSchemaVersion:1,
lifecycleLinkRecordSchemaVersion:1,
terminalCodecVersion:1,maxDaemonResponseLineBytes:8454144,
maxTaskLifecycleLinkRecordBytes:1024,
stagedReceiptRecordMaxBytes,taskTerminalBoundLinkRecordMaxBytes,
taskPublicationCases,capacityFallbackCases`.
Никакого неописанного schema/limits digest в certificate нет: version и numeric
limit проверяются как literal values при reopen. `taskPublicationCases` — exact
five-entry closed internally tagged algebra с tag field `kind`, не nullable
Options: один `kind:"absent" { finalTaskRecordMaxBytes,
taskResponseFrameMaxBytes }` и четыре `kind:"exact_provisional" { status,
version, cancelRequested,
finalTaskRecordMaxBytes, taskResponseFrameMaxBytes }` для exact wire statuses
`queued|working` × false/true; `version` в этих четырёх certificate witnesses
равен literal `18446744073709551615`, то есть `u64::MAX` decimal-width bound;
array order literal: absent, queued/false, queued/true, working/false,
working/true. `capacityFallbackCases` — exact one-entry closed algebra с tag
field `source`: только `source:"link_capacity" { receiptBackedRecordMaxBytes,
taskResponseFrameMaxBytes }`. Certificate record и
каждый nested variant имеют
`deny_unknown_fields`; missing/extra/cross-variant fields, другая cardinality,
duplicate tag, иной order либо иное literal значение отвергаются при reopen. Late codec
строит из actual readback тот же closed exact publication case с реальным u64
version и доказывает его decimal width/bytes против соответствующего
witness/bound; `taskTerminalBoundLinkRecordMaxBytes` обязан быть ≤ literal 1024,
Link Capacity path обязан выбрать этот единственный source case.
Отдельный StagedHandoffTask bundle consume-ит committed staged readback с этим
certificate, несёт exact terminal-publication expectation,
`receiptExpectedVersion`, live reservation binding и exact link digest; он не
требует уже опубликованного `TaskBound` и rehydrate-ит canonical terminal только
из committed staged receipt record. Late codec строит exact pieces и проверяет
каждый exact byte count `<=` соответствующего certified bound. Valid certificate
делает late oversize недостижимым; binding/size/schema mismatch является
invariant corruption и fail-stop-ит без смены staged winner на `ResultTooLarge`.
`StagedTaskPublicationExpectation` закрыт ровно как `Absent` либо
`ExactProvisional { taskId, invocationId, status: Queued|Working, version,
cancelRequested, taskLinkDigest }`; все IDs/digest canonical и exact, mutable
observation не заменяется boolean/verdict.
Каждая linear record piece проверяет только соответствующую store version;
единого cross-store generation нет. Wire bytes включают завершающий `\n`, а record limits применяются к
полным persisted bytes. Completed тем самым preflight-ит canonical
`DomainResult`; Failed/Cancelled не создают fake result.

Persisted owner хранит ровно один canonical terminal payload, digest и terminal
epoch под inclusive `MAX_DAEMON_RESPONSE_LINE_BYTES` entitlement; full response
frame рядом с payload не сохраняется. «Ровно один» запрещает две payload copies
внутри одного owner record и persisted wire frame; staged transfer ниже
намеренно допускает краткую crash-safe копию той же canonical payload в двух
разных stores, charged существующим ReceiptLedger entitlement и заранее
проверенным TaskStore record capacity, до удаления ledger copy при commit.
Pieces потребляются линейно в порядке owner-а. Direct и receipt-owned Task передают `PreparedReceiptRecord` ledger writer-у; committed publication
возвращает нетронутый `PreparedWireFrame` для send. TaskStore-owned terminal
сначала передаёт `PreparedTaskRecord` TaskStore и получает exact readback вместе
с оставшимися lifecycle-link/wire pieces, затем ledger проверяет current
TaskBound lifecycle-link record, consume-ит `PreparedTaskLifecycleLinkRecord` для
in-place `TaskTerminalBound` CAS и возвращает
committed publication, всё ещё несущую исходный `PreparedWireFrame`. Никакой
writer не consume-ит frame до durable commit и ни один переход не
re-serialize-ит outcome. Constructor технически доступен codec внутри crate,
поэтому это не cryptographic claim: static ownership guard разрешает
construction owner pieces и serialization receipt/Task/wire envelope только
`terminal_codec_v5.rs` и strict golden-vector tests; новый call site ломает CI.
Immediate submit использует этот transient prepared frame. Exact duplicate и
recovery читают persisted canonical payload и через тот же CoreIdentity-bound
codec заново строят, полностью size/hash-проверяют и лишь затем одним write
передают новый owner-specific `PreparedWireFrame`; после frame preflight
запрещены reserialization и fallback в current protocol error. Поэтому
гарантия относится к каждому write, а не обещает хранить wire bytes после
durable terminal. Post-reserve catalog/schema invariant failure, admission
identity/proof drift, serialization invariant violation и store/receipt commit
uncertainty ведут к process fail-stop и exact readback/recovery. Они не
маскируются под semantic failure или admission backpressure. Binder обязан
отдельно классифицировать semantic `Invalid`, typed `Capacity`, typed
`RegistryFailed`, deadline expiry, actor identity/proof drift и internal error:
только `Invalid` становится `Completed`, Capacity/RegistryFailed следуют
закрытым failure branches выше, а deadline/drift/internal не превращаются в
предметный result.

Staged-handoff bundle является отдельным owner type: task piece разрешает
при `Absent` atomic terminal create, при `ExactProvisional` CAS-terminalization
только exact TaskId/InvocationId/Queued-or-Working/version/cancel/link-digest
record и idempotent readback только exact same terminal; foreign/mismatch
fail-stop-ит без ledger mutation. Matching live `TaskLinkReservation`
обязателен. Late exact task/lifecycle-link/wire pieces consume checked
`StagedTerminalTransferSizeCertificate`; все размеры обязаны укладываться в его
owner-specific bounds. Lifecycle-link piece разрешает только direct
`TaskHandoffActorBound(staged)` → `TaskTerminalBound` с independent receipt
expected version и exact link digest. После TaskStore write exact readback
возвращает remaining lifecycle-link/wire pieces; ledger атомарно consumes
reservation, materializes link, удаляет staged payload и возвращает untouched
wire frame. Crash до ledger commit оставляет одновременно exact terminal Task и
исходный staged handoff; recovery повторяет только exact readback и ledger
commit, не callback и не terminal serialization. Proven LinkCapacity выбирает
отдельный late bundle: он consume-ит тот же certificate, committed staged readback
и closed `StagedLinkCapacityEvidence`, доказывающий отсутствие reservation, строит certified
`TaskTerminalReceiptBacked` record+Task frame и оставляет исходный terminal
winner; late oversize/reclassification/`TaskCapacity` overwrite запрещены.

Текущий production `SafeFailureReason` и schema-v2 `StoredInvocationRecord`
остаются буквально и по serde acceptance неизменными: `SafeFailureReason`
содержит `InvocationFailed`, `ResultTooLarge`, `Interrupted`,
`ResumeUnsupported` и `PersistenceFailed`. Side-by-side v5 вводит отдельный
закрытый `V5SafeFailureReason` с этими пятью вариантами плюс ровно
`OutcomeUncertain`, `TaskCapacity`, `WorkspaceCapacity` и
`WorkspaceRegistryFailed`, а также total infallible conversion
`SafeFailureReason -> V5SafeFailureReason`. Это не alias и не расширение legacy
enum. `InterruptedBeforeExecution` остаётся recovery classification; persisted
v5 reason для него — `V5SafeFailureReason::Interrupted`. `TaskCapacity`
применяется только к proven `LinkCapacity` до `Begun`,
`WorkspaceCapacity`/`WorkspaceRegistryFailed` — только к соответствующим typed
admission branches. До атомарного W0c selection активный v3 path,
включая current early protocol mapping Capacity/RegistryFailed, current
`SafeFailureReason` и schema-v2 record decoder, не меняется.

Внутри `Reserved` разрешена только цепочка `Unbound` → `ActorBound` → `Begun`.
Повторный bind exact actor identity идемпотентен; другой actor-derived hash для
уже bound receipt означает workspace identity drift/corruption, переводит
daemon в `RestartRequested` и не меняет TaskStore.

`Begun` записывается и подтверждается перед первым вызовом `service.prepare`;
это консервативно считает preparation частью attempt и не предполагает, что
будущий provider никогда не выполнит там side effect. После `Begun` никакой
recovery, submit, get, cancel или ACK не получает domain callback для повторного
запуска. Actor/resource lease в памяти обязан соответствовать сохранённому
`boundWorkspaceIdentity`; mismatch до prepare закрывает admission/fail-stop.

Live `BoundStartCancelGate` одной invocation линеаризует start и cancel до
`Begun`. Direct transition `mark_reserved_begun` под этим gate одной операцией
ReceiptLedger проверяет `Reserved::ActorBound` и `cancelRequested=false` и пишет
`Begun`; отдельного read-before-write нет. Cancel, выигравший gate первым,
durable terminalizes cancelled и запрещает callback; `Begun`, выигравший первым,
делает последующий cancel post-Begun.

Application-owned `ActorBindingClaim { identity, generation }` — checked pure
value без infrastructure handle. `bind_actor`/`bind_promised_actor` принимают
claim и только после durable bind возвращают committed receipt вместе с opaque
one-shot `V5ActorBindingToken`, exact-bound к key/identity/generation; fields и
constructor token private ledger module. Application `invocation_v5` владеет
только pure lifecycle executor/state machine и этими claim/token/port types.

Infrastructure-private `runtime_v5::InvocationCoordinator` является
единственным live owner actual `WorkspaceActor`/resource lease, lease verifier,
`BoundStartCancelGate` и provider `CancellationToken`. Непосредственно перед
Direct begun под тем же gate он private-verifies matching live lease/generation
и предъявляет ledger token; ledger exact сравнивает key/identity/generation и
consume-ит его один раз. Для Task path coordinator так же consume-ит actor
binding token в `authorize_bound_task_start`, но получает distinct one-shot
`PostWorkingActorAuthorization`; только после atomic TaskStore Working
readback он снова private-verifies live lease и этим authorization вызывает
`mark_bound_task_begun`. Application не импортирует infrastructure и не
принимает verdict boolean; tokens не являются cryptographic/unforgeable claim
внутри crate, не serializable и после restart не восстанавливаются из durable
hash. Static ownership/import guard разрешает token construction только ledger,
actual lease/proof/verifier и coordination sequence — только `runtime_v5`, а
application imports infrastructure запрещает; новые call sites ломают CI.
Missing/foreign/stale lease закрыто отклоняется до ledger mutation. Coordinator
держит gate и lease до terminal cleanup либо process fail-stop; cancel сначала
commits current durable authority и лишь затем сигналит provider token.

`ExecutionClass::KnownLong` не является pre-admission классификацией: в текущем
контракте его возвращает `service.prepare`, то есть receipt уже `ActorBound` и
`Begun`. Поэтому known-long и cutoff во время prepare/execute сначала durable
пишут `TaskHandoffActorBound`, затем пытаются выполнить exact TaskStore
create/readback и commit `TaskBound`; доказанная Link Capacity оставляет
begun attempt receipt-owned. Ни одна из ветвей не переходит назад в
`TaskPromisedUnbound`: это состояние разрешено только когда original cutoff
наступил внутри ещё `Unbound` validation/admission.

Semantic validation и любой `WorkspaceAdmissionError` завершаются до actor bind:
`Reserved::Unbound` публикует Direct, `TaskPromisedUnbound` — receipt-backed
Task. Prepare rejection уже post-`Begun`: в Direct owner оно публикует Direct, в
pre-bind handoff stage-ится, затем terminal TaskStore write/readback и direct
handoff→`TaskTerminalBound` commit сохраняют outcome без промежуточного
`TaskBound`; в `TaskReceiptOwnedActorBound` публикуется receipt-backed,
а после `TaskBound` — только TaskStore + `TaskTerminalBound`. RegistryFailed
после terminal commit дополнительно закрывает listener и запрашивает restart.
Durable reserve ни в одной branch не откатывается. Если процесс
погиб с `begun=true` и без
доказанного terminal:

- receipt без promised/Task становится `DirectTerminalUnacked(Failed {
  V5SafeFailureReason::OutcomeUncertain })`;
- `TaskPromisedUnbound` не может иметь `Begun`: restart/cancel terminalizes его
  `TaskTerminalReceiptBacked` как interrupted-before-execution/cancelled без
  callback;
- `TaskBound` получает в v5 TaskStore `Failed {
  V5SafeFailureReason::OutcomeUncertain }`, затем ledger становится
  `TaskTerminalBound`;
- execution не replay-ится даже при положительном новом budget.

Если в будущем конкретный typed resume owner сможет доказать transaction
coupling, он потребует отдельного решения и инварианта. Общего resume owner этот
проект не создаёт.

## Protocol и CoreIdentity

Целевая identity — `unica-daemon-jsonl-5`. На baseline 2026-08-28
`origin/main` ещё не содержал daemon protocol; committed feature-branch
predecessor — v3. Локальная experimental v4 не считается продуктовым контрактом
и резервируется как несовместимая промежуточная identity. Переход на v5 меняет
`CoreIdentity` и state directory: v3/v4/v5 process и state не смешиваются, а
protocol tests явно отклоняют обе predecessor identity.

W0a не добавляет variants или fields в active `ClientRequest`,
`ServerResponse`, `SafeFailureReason` либо schema-v2
`StoredInvocationRecord`: их Rust definitions, JSON wire/persisted bytes и
strict serde acceptance остаются буквально прежними. V5 использует отдельные
`V5ClientRequest`, `V5ServerResponse`, `V5StoredInvocationRecord` и
`V5SafeFailureReason` с собственными strict decoders. `CoreIdentity` и selector
state directory параметризуются protocol identity, но `production()`
composition по умолчанию продолжает выбирать v3. Только W0c атомарно
переключает composition на v5; он не переопределяет и не ослабляет legacy
decoder.

Реальный `--daemon` entry в `interfaces/daemon.rs` становится additive closed
dispatch seam без нового CLI/env/test selector. Existing strict parse
`--core-identity` не меняет acceptance: только exact known
`CoreIdentity::production_v5()` выбирает `runtime_v5::run_daemon`; каждый другой
уже принимаемый canonical 64-hex CoreIdentity продолжает маршрутизироваться в v3
`server::run_daemon`, поэтому arbitrary fixture identity не ломается. Invalid
syntax отклоняется прежним parser. V5 client запускает тот же executable с v5
CoreIdentity; endpoint helper принимает typed `DaemonProtocolIdentity`, чтобы
v3/v5 path не вычислялся второй раз. Default connect остаётся v3 в W0a/W0b, а
W0c атомарно меняет default constructor. Process acceptance обязана провести
real v5 client через spawned `--daemon` к v5 handshake и одновременно доказать
прежнее v3 default/decoder/arbitrary-identity поведение.

Side-by-side runtime требует сначала semantic-neutral извлечь private
`CanonicalInvocationService`, `ActorBoundInvocation`, `ActorBoundExecution` и
их capability helpers из `server.rs` в shared infrastructure module
`daemon/invocation_service.rs`. V3 server импортирует этот seam, v13 service
меняет только impl/import path, а v5 runtime использует тот же service. Это
узкое изменение `server.rs`/`v13_service.rs`, а не literal-file freeze: gate —
byte-equivalent v3 JSONL/golden serde rejection, daemon lifecycle и service
behavior до и после extraction. Protocol logic и v3 semantics при этом не
переносятся в v5 и не изменяются.

### Frozen v5 serde algebra

Protocol-v5 не наследует open `InvocationFailure` и не расширяет v3 enums.
Каждый перечисленный record имеет `rename_all = "camelCase"` и
`deny_unknown_fields`; каждый internally tagged enum имеет exact lowercase
snake-case tag/value и отклоняет поле, не принадлежащее выбранному variant.
Scalar enums также отклоняют неизвестную строку. Optional/flatten map и
`#[serde(default)]` в этой algebra запрещены. `DomainResult` сохраняет свой
существующий strict record contract. Protocol JSONL не несёт `schemaVersion`:
его discriminator — exact `protocolVersion: 5`. Persisted v5 Task record имеет
отдельный обязательный `schemaVersion: 1`, scoped только к isolated v5 state
directory; legacy schema-v1/v2 decoders его не открывают.

Exact strict records:

- `V5ReceiptKey`: `invocationId`, `reservedTaskId`, `coreIdentityDigest`,
  `tool`, `normalizedArgumentsHash`, `requestScopeHash`;
- `V5InvocationRequest`: `invocationId`, `reservedTaskId`, `tool`, `arguments`,
  `workspaceHint`, `responseBudgetMs`; server выводит key из handshake
  `CoreIdentityDigest` и production hash helpers, caller не передаёт digest;
- `V5PendingDirectReceipt`: `receiptKey`, `terminal`, `terminalDigest`,
  `terminalEpochMs`; поле называется `terminal`, не `result`, потому что owner
  может завершиться любым `V5TerminalOutcome`;
- `V5AcknowledgedReceipt`: `receiptKey`, `terminalDigest`, `ackEpochMs`,
  `expiresEpochMs`; последнее поле вычисляется как `ackEpochMs + 900000`, но в
  tombstone persisted остаются только key, digest и first ACK epoch.

`V5ClientRequest` использует tag `kind` и имеет ровно такие variants/fields:

| `kind` | Fields |
| --- | --- |
| `hello` | `protocolVersion`, `token`, `coreIdentity`, `ownerLease` |
| `ping` | none |
| `release` | none |
| `submit_invocation` | `invocation: V5InvocationRequest` |
| `get_task` | `taskId` |
| `wait_task` | `taskId`, `waitMs` |
| `cancel_task` | `taskId` |
| `recover_invocation_receipt` | `receiptKey: V5ReceiptKey` |
| `acknowledge_invocation_receipt` | `receiptKey: V5ReceiptKey`, `terminalDigest` |
| `cancel_invocation` | `receiptKey: V5ReceiptKey` |

`V5ServerResponse` использует tag `kind` и имеет ровно такие variants/fields:

| `kind` | Fields |
| --- | --- |
| `ready` | `protocolVersion`, `coreIdentity`, `daemonPid`, `instanceId` |
| `pong` | none |
| `released` | none |
| `invocation` | `outcome: V5InvocationResponse`; единственный response kind для submit, exact duplicate и `recover_invocation_receipt` |
| `task` | `snapshot: V5DaemonTaskSnapshot` |
| `invocation_acknowledged` | `acknowledgement: V5AcknowledgedReceipt` |
| `error` | `code: V5DaemonErrorCode` |

`V5InvocationResponse` использует tag `resultType`: `receipt_pending` содержит
`receiptKey`, `phase`, `acceptedEpochMs`, `originalBudgetMs`,
`cancelRequested`; `direct` содержит `receipt: V5PendingDirectReceipt`; `task`
содержит `snapshot: V5DaemonTaskSnapshot`; `acknowledged` содержит
`acknowledgement: V5AcknowledgedReceipt`. `phase` — один из exact
`cancel_reserved`, `reserved_unbound`, `reserved_actor_bound`,
`reserved_begun`. Submit normally возвращает Direct/Task; pending существует
только для read-only exact recovery живого original lifecycle и не даёт новый
budget.

`V5TerminalOutcome` использует tag `status`: `completed { result }`,
`failed { reason }`, `cancelled {}`. `V5SafeFailureReason` сериализуется exact
strings `invocation_failed`, `result_too_large`, `interrupted`,
`resume_unsupported`, `persistence_failed`, `outcome_uncertain`,
`task_capacity`, `workspace_capacity`, `workspace_registry_failed`. Это ровно
девять variants; `OutcomeUncertain` нигде не преобразуется в `Interrupted`.

`V5DaemonTaskSnapshot` — internally tagged `status` union. Все пять variants
имеют exact common fields `taskId`, `invocationId`, `receiptKeyDigest`,
`createdAtEpochMs`, `updatedAtEpochMs`, `ttlMs`, `pollIntervalMs`, `version`,
`cancelRequested`. `queued` и `working` не имеют дополнительных fields;
`completed` добавляет `terminalEpochMs`, `terminalDigest`, `result`; `failed`
добавляет `terminalEpochMs`, `terminalDigest`, `reason`; `cancelled` добавляет
только `terminalEpochMs`, `terminalDigest`. Result/reason/terminal fields в
неподходящем variant не optional, а запрещены. Raw `TaskId`/`InvocationId` и
created/updated timestamps не заменяются verdict booleans.

`V5StoredInvocationRecord` имеет exact top-level fields `schemaVersion` (только
integer `1`), `taskId`, `invocationId`, `receiptKeyDigest`, `tool`,
`normalizedArgumentsHash`, actor-derived `workspaceIdentityHash`,
`createdAtEpochMs`, `updatedAtEpochMs`, `ttlMs`, `pollIntervalMs`, `version`,
`cancelRequested`, `task`. Вложенный `task` — тот же closed status algebra, но
без повторения identity/time fields: `queued {}`, `working {}`,
`completed { terminalEpochMs, terminalDigest, result }`,
`failed { terminalEpochMs, terminalDigest, reason }`,
`cancelled { terminalEpochMs, terminalDigest }`. Legacy `statusMessage`,
open `failure`, optional `result`, `resume` и unknown fields запрещены.
`V5StoredInvocationRecord` и `V5DaemonTaskSnapshot` декодируются разными strict
types, а projection между ними является total function с exact field equality.

`V5DaemonErrorCode` содержит exact code-only strings `invalid_request`,
`handshake_required`, `protocol_mismatch`, `core_mismatch`, `unauthorized`,
`duplicate_lease`, `overloaded`, `owner_capacity`, `receipt_not_found`,
`receipt_expired`, `receipt_capacity`, `tombstone_capacity`,
`invocation_identity_mismatch`, `task_not_found`, `task_expired`,
`store_failed`, `durability_uncertain`. Semantic Invalid и все четыре typed v5
failure reasons `task_capacity`, `workspace_capacity`,
`workspace_registry_failed`, `outcome_uncertain` после reserve являются
terminal outcome, не `error` response. Premature/Task ACK даёт
`invalid_request`; full-key/digest mismatch —
`invocation_identity_mismatch`; neither mutates state. `tombstone_capacity`
означает только невозможность committed ACK compaction и не изменяет исходный
`DirectTerminalUnacked`.

Поведение submit/recovery:

1. Новый exact key durable резервируется и продолжает исходный lifecycle.
2. Exact повтор `SubmitInvocation` не создаёт новый deadline и не вызывает
   prepare/execute: он наблюдает исходный live receipt либо durable state.
3. `DirectTerminalUnacked` возвращает byte-equivalent canonical result.
4. Потеря/закрытие submit session сама не меняет receipt, не отменяет callback и
   не ускоряет handoff. Единственный attempt продолжает исходный lifecycle под
   original cutoff: completion до cutoff публикует recoverable
   `DirectTerminalUnacked`; только сам cutoff выбирает
   `TaskPromisedUnbound`/`TaskHandoffActorBound`, а KnownLong — actor-bound
   handoff. Exact recover лишь читает этот state и не создаёт новый budget.
5. `TaskPromisedUnbound`/`TaskPromisedActorBound` возвращает stable queued
   snapshot, `TaskHandoffActorBound`/`TaskReceiptOwnedActorBound` — stable
   queued/working snapshot, а `TaskTerminalReceiptBacked` — exact terminal
   snapshot из ReceiptLedger; `TaskBound`/`TaskTerminalBound` читает exact
   snapshot из TaskStore.
6. `AcknowledgedTombstone` подтверждает уже принятый receipt без result replay.
7. Partial identity match возвращает mismatch; unknown и expired различимы.

В том же процессе хранится исходная monotonic deadline capability. После
restart deadline не реконструируется из wall clock и не получает новую
duration: recovered `CancelReserved` сохраняет original epoch expiry;
`Reserved::Unbound`/`ActorBound` без committed promise/handoff становится
`DirectTerminalUnacked(cancelled|interrupted_before_execution)`, а
`Reserved::Begun` — `DirectTerminalUnacked(Failed {
V5SafeFailureReason::OutcomeUncertain })`. Только уже
committed promised/handoff state разрешает receipt-backed Task или exact
TaskStore reconciliation. Ни один recovery путь не вызывает domain callback и
не изобретает Task задним числом.

## Direct ACK и граница доставки

Daemon сначала durable публикует `DirectTerminalUnacked`, затем отправляет
terminal. Client возвращает интерфейсному слою не голый result, а
`V5PendingDirectReceipt { terminal: V5TerminalOutcome, receiptKey,
terminalDigest, terminalEpochMs }`. ACK отправляется только после успешной
проверки размера/parse и полного построения соответствующего immutable final
native projection: `CallToolResult` для Completed либо exact `ErrorData` для
Failed/Cancelled. Эта projection уже существует до первого ACK write; никакой
из трёх terminal variants не ACK-ится по одному факту decode. Drop/projection
error, suppressed cancelled JSON-RPC response либо crash frontend до этой точки
ACK не посылает. Успешно построенная Failed/Cancelled projection использует тот
же ACK ordering, что Completed.
ACK, пришедший до committed `DirectTerminalUnacked`, является premature и
закрыто отклоняется без mutation; ACK никогда не используется как способ
дописать или выбрать terminal outcome.

ACK commit переводит record в компактный tombstone; повторный ACK после потери
ACK response читает тот же tombstone и успешен. Если ACK не committed,
recovery возвращает исходный direct terminal. Terminal digest запрещает ACK
другого terminal outcome при той же identity.

Tombstone содержит только exact key, terminal digest и epoch первого
committed ACK. Он не наследует original cutoff, response budget, result bytes
или другой lifecycle payload, а повторный ACK не меняет first-ACK epoch. Если
bounded tombstone count/byte pool заполнен после expired-only reclamation,
`acknowledge_direct` возвращает typed `TombstoneCapacity`; исходный
`DirectTerminalUnacked` остаётся byte-equivalent и занимает прежнюю live/result
quota до успешного retry либо terminal+1h expiry. ACK capacity не закрывает
listener, не теряет уже построенный `CallToolResult` и не превращается в
успешный ACK. Потеря ACK request оставляет unacked terminal; потеря ACK response
оставляет либо тот же unacked terminal, либо committed tombstone, поэтому
retry exact key/digest разрешает оба исхода без replay.

ACK разрешён только для `DirectTerminalUnacked`. Promised Task является
многократно читаемым durable объектом: `GetTask`, `WaitTask` и compatibility
`task.result` могут повторно запросить один canonical result до TTL. Поэтому
`TaskTerminalReceiptBacked` не compact-ится первым чтением или Direct ACK,
остаётся единственным владельцем payload один час от terminal epoch и после
expiry отвечает `task_expired`. `CancelTask` после terminal возвращает того же
winner. ACK с Task terminal digest отклоняется как `invalid_request` и не
удаляет payload.

Этот ACK доказывает передачу результата от daemon во владение stdio frontend,
но не подтверждает, что MCP host уже получил JSON-RPC response: SDK не даёт
daemon транзакционного callback после host delivery. Crash frontend после ACK
может потерять доставку, поэтому exactly-once delivery намеренно не заявляется.
Публичный idempotency/resume API для компенсации этого окна не добавляется.

### Frozen native/compatibility projection

V5 frontend не переиспользует current open `InvocationFailure` или legacy
`DaemonTaskSnapshot`. Direct `Completed(result)` становится immutable native
`CallToolResult` с exact `structuredContent = result`, пустым `content` и
`isError = !result.ok`; `Failed(reason)` становится JSON-RPC `ErrorData` с code
`-32603`, exact message/code из таблицы ниже и data
`{"code":"<reason-code>"}`; `Cancelled` становится `ErrorData` с code `-32603`,
message `daemon invocation was cancelled`, data
`{"code":"invocation_cancelled"}`. Эти ErrorData являются final terminal
projection, а не transport/store failure. Их полные JSON bytes также bounded до
ACK.

| `V5SafeFailureReason` | Exact code | Exact message |
| --- | --- | --- |
| `InvocationFailed` | `invocation_failed` | `daemon invocation failed` |
| `ResultTooLarge` | `result_too_large` | `daemon invocation result exceeded the canonical byte limit` |
| `Interrupted` | `interrupted` | `daemon invocation was interrupted` |
| `ResumeUnsupported` | `resume_unsupported` | `daemon invocation cannot be resumed after restart` |
| `PersistenceFailed` | `persistence_failed` | `daemon invocation terminal state could not be persisted` |
| `OutcomeUncertain` | `outcome_uncertain` | `daemon invocation outcome is uncertain` |
| `TaskCapacity` | `task_capacity` | `daemon Task capacity was exhausted before execution` |
| `WorkspaceCapacity` | `workspace_capacity` | `workspace capacity was exhausted` |
| `WorkspaceRegistryFailed` | `workspace_registry_failed` | `workspace registry is unavailable` |

Один total v5 Task projector читает strict `V5DaemonTaskSnapshot`; native и
compatibility adapters не реконструируют reason по status или prose:

| Internal v5 status/payload | Native MCP Tasks | `unica.task.get` / `unica.task.cancel` | `unica.task.result` |
| --- | --- | --- | --- |
| `queued` | `Task.status="working"`, `TaskPayload::Working`; raw Task id/timestamps/TTL/poll copied | `DomainResult(ok=true, summary="Task is still working")`, `data.task.status="queued"`, next `unica.task.result` | тот же working receipt |
| `working` | `Task.status="working"`, `TaskPayload::Working`; raw fields copied | тот же result, `data.task.status="working"` | тот же working receipt |
| `completed(result)` | `Task.status="completed"`, `TaskPayload::Completed` с exact native `CallToolResult(result)` | `DomainResult(ok=true, summary="Task completed")`, `data.task.status="completed"` | exact original `DomainResult`, включая `ok=false` только если он был semantic Completed |
| `failed(reason)` для каждого из 9 | `Task.status="failed"`, `TaskPayload::Failed` с `ErrorData(-32603, exact message, {code})` из таблицы | `DomainResult(ok=false, summary=<exact message>, data={code, task...status:"failed"})` | тот же typed failure result; reason не сворачивается в `task_failed` |
| `cancelled` | `Task.status="cancelled"`, `TaskPayload::Cancelled`, без result/error payload | `DomainResult(ok=false, summary="Task was cancelled", data={code:"task_cancelled", task...status:"cancelled"})` | тот же cancelled result без invented domain result |

Во всех compatibility `data.task` exact fields — `taskId`, `invocationId`,
`createdAtEpochMs`, `updatedAtEpochMs`, `ttlMs`, `pollIntervalMs`, `version`,
`cancelRequested`, `status`; terminal rows также имеют `terminalEpochMs` и
`terminalDigest`. Native `tasks/cancel` возвращает protocol-defined empty
success только после durable cancel/winner resolution; следующий `tasks/get`
возвращает ту же строку матрицы. Terminal cancel idempotent и не меняет payload,
timestamps или winner. Current v3 native/compat projectors и их legacy
code/message mapping остаются byte/acceptance unchanged; W0c выбирает отдельные
v5 projectors атомарно. В частности `OutcomeUncertain` остаётся distinct reason
в v5 storage, snapshot, native и compatibility projection и никогда не
проецируется как `interrupted`.

## Cutoff и связь с TaskStore

Семисекундный handoff не ослабляется ожиданием actor identity. Если workspace
admission либо semantic validation всё ещё barrier-blocked, deadline owner к
original cutoff durable переводит только `Reserved::Unbound` в
`TaskPromisedUnbound` и в пределах прежних 125 мс возвращает queued Task с exact
`reservedTaskId`. `task.get` и `task.cancel` читают это состояние прямо из
ReceiptLedger. Это не TaskStore record и не подставляет request-scope hash
вместо actor identity.

Вся pre-`Begun` unbound pipeline одной invocation — semantic validation вместе
с actor admission/binding — получает один fixed cleanup grace две секунды,
начинающийся ровно при commit `TaskPromisedUnbound`. Та же absolute grace
deadline передаётся через validation → admission → bind и ни при возврате
validation, ни при входе в admission, ни при смене worker stage не сбрасывается.
Worker проверяет durable terminal winner после validation, перед admission,
после admission, перед actor bind и перед `Begun`/любым следующим callback.
Rejection, успевший выиграть race, пишет
`TaskTerminalReceiptBacked`; actor capability, успевшая выиграть race, сначала
пишет `TaskPromisedActorBound`. Проигравший late return только освобождает lease
и не продолжает pipeline.

Если unbound worker не вышел и не достиг durable actor-bound handoff за две
секунды, executor пишет restart intent, закрывает listener и daemon main
завершает PID без join этого worker. Successor пишет
classification `InterruptedBeforeExecution` публикуется как
`TaskTerminalReceiptBacked(Failed { V5SafeFailureReason::Interrupted })` и
никогда не вызывает
validation/admission/domain callback. Так один terminal promise не может жить
рядом с зависшим owner, который позднее продолжит выполнение.

Если cutoff наступил уже после durable `ActorBound` либо во время
`service.prepare`/execute, deadline owner пишет `TaskHandoffActorBound`, а не
unbound promise, и этот durable intent сам является временным Task projection,
достаточным для ответа в исходные 125 мс до exact TaskStore create/readback и
commit `TaskBound`. Тот же переход используется после возврата
`ExecutionClass::KnownLong`: prepare к этому моменту уже один раз начат под
`Begun`. Optional terminal outcome, выигравший race с TaskStore creation,
durable stage-ится в этом intent и переносится в exact Task без второго
callback.

Между ReceiptLedger и TaskStore нет общей физической транзакции. Promised и
already-bound handoff используют два явных write-ahead входа:

1. После unbound promise ReceiptLedger fsync-переходом пишет
   `TaskPromisedActorBound { begun:false }`; после уже actor-bound cutoff либо
   KnownLong он пишет `TaskHandoffActorBound { begun }`. Оба состояния содержат
   exact `reservedTaskId`, actor-derived Task identity и TaskStore-bind intent.
2. До первой TaskStore mutation coordinator под тем же gate вызывает
   `reserve_task_link`: ReceiptLedger durable резервирует exact key/Task identity,
   один общий lifecycle-link/TaskStore count slot и maximum 1 KiB link byte
   entitlement в handoff intent.
   Недостаток link count/bytes является proven pre-TaskStore Capacity: TaskStore
   остаётся byte-for-byte неизменным, а pre-Begun/begun branches следуют тем же
   `TaskCapacity`/receipt-owned правилам. Reservation idempotent для exact intent;
   partial collision отклоняется до mutation.
3. Пока PID жив, coordinator удерживает тот же `BoundStartCancelGate` на всём
   переносе cancel-authority. Только с live `TaskLinkReservation` TaskStore выполняет idempotent
   create-if-absent `Queued` для `begun=false` либо `Working` для `begun=true` и
   переносит monotonic `cancelRequested` из intent. После exact readback
   ReceiptLedger одной mutation materializes reserved link, публикует `TaskBound`
   и доказывает ту же identity/cancel flag; с этого commit единственным
   cancel-authority становится TaskStore. `bind_task` consumes pre-bind
   reservation и возвращает post-bind `TaskBoundLinkAuthorization` exact
   task/link/key/generation. Recovery выполняет эту
   последовательность single-threaded до listener, но materializes link для уже
   существующей Task только когда exact retained handoff intent содержит matching
   live `TaskLinkReservation`. Отсутствующая, expired или mismatched reservation
   означает corruption/fail-stop до link mutation: иначе restart обошёл бы
   boundary 4096. Exact уже существующая запись считается тем же commit;
   identity/state/cancel regression является corruption/fail-stop.
   `CommitUncertain` удерживает reservation и intent до exact reconciliation;
   blind retry/release запрещены. Успешная reservation структурно гарантирует
   TaskStore count headroom; последующий `TaskStore::Capacity` преобразуется
   только в `TaskStoreCapacityInvariantViolation`, сохраняет intent/reservation и
   fail-stop-ит без terminal branch, fallback либо release.
4. Если в `TaskHandoffActorBound` уже staged terminal outcome, coordinator не
   публикует промежуточный `TaskBound`. Пока handoff, live reservation и staged
   canonical payload остаются ledger-owned, sole codec consume-ит persisted
   checked `StagedTerminalTransferSizeCertificate` и committed staged readback,
   строит exact terminal Task, sole `TaskTerminalBound` lifecycle-link piece и
   transient wire frame и проверяет каждый размер против certified upper bound.
   TaskStore по closed expectation либо создаёт exact
   terminal Task из `Absent`, либо CAS-terminalize-ит exact provisional.
   Если ordinary create успел записать exact provisional
   Queued/Working до того, как staged outcome выиграл handoff CAS, та же
   operation атомарно terminalize-ит только exact expected TaskId/InvocationId/
   status/version/cancel/link digest; exact same terminal принимается как
   idempotent readback, любой foreign/mismatch
   fail-stop-ит без ledger mutation. Затем ledger одной mutation consumes reservation,
   materializes link, удаляет staged payload и переходит прямо в
   `TaskTerminalBound`. Crash между writes восстанавливается из exact terminal
   Task + retained staged handoff/reservation без callback. Без staged terminal
   обычная ветка materializes `TaskBound` и продолжает single attempt; later
   terminal readback публикует `TaskTerminalBound`.
5. Task resolver атомарно меняет источник projection с receipt-backed promise
   или handoff intent на exact TaskStore record. Старый queued snapshot и новый
   snapshot имеют те же TaskId, invocationId, createdAt, TTL и poll interval;
   timestamps не регрессируют.
6. Для `begun=false` после шага 5 coordinator захватывает per-invocation
   `BoundStartCancelGate` и удерживает его до receipt `begun`. Под gate он
   private-verifies matching live actor/resource lease и consume-ит
   `V5ActorBindingToken`, получая one-shot
   `PostWorkingActorAuthorization`; он
   предъявляет current `TaskBoundLinkAuthorization` + exact TaskBound proof,
   затем TaskStore sole-writer
   атомарно выполняет `start_working_if_not_cancel_requested`: без отдельного
   false-read переводит exact versioned `Queued` в `Working` либо возвращает
   durable cancel/terminal winner. Только для Working readback coordinator
   повторно private-verifies lease и ReceiptLedger consume-ит
   `PostWorkingActorAuthorization`, проверяет exact TaskBound/link/generation и
   durable пишет `begun=true`; до этого
   resolver нормализует Task как queued. Затем gate освобождается и вызывается
   `service.prepare`. Cancel использует тот же gate, поэтому он не может durable
   записаться между Working readback и receipt `begun`: до start он запрещает
   callback, после `begun` является post-Begun cancellation. Live executor
   удерживает proof/actor/resource lease до terminal cleanup либо process
   fail-stop. Уже begun short/blocked prepare на cutoff продолжает единственный
   attempt и получает тот же Task. `KnownLong` никогда не создаёт
   `TaskPromisedUnbound`.

Только typed `LinkCapacity`, доказанный до reservation и TaskStore mutation,
является нормальным task-publication backpressure. Для
`TaskPromisedActorBound` или `TaskHandoffActorBound { begun:false }` coordinator
без callback атомарно пишет `TaskTerminalReceiptBacked(Failed {
V5SafeFailureReason::TaskCapacity })`; уже staged terminal или committed
pre-Begun cancel имеет приоритет. Для begun handoff без staged terminal
coordinator durable latch-ит `TaskReceiptOwnedActorBound`, не повторяет
reservation/create и не вытесняет чужой Task; live attempt продолжает работу, а
его actual terminal сохраняется receipt-backed. Crash до terminal даёт
`outcome_uncertain`. После успешной reservation typed TaskStore `Capacity` не
является backpressure: это `TaskStoreCapacityInvariantViolation`, которое
сохраняет intent/reservation и закрывает listener для fail-stop/exact
reconciliation. `CommitUncertain` также сохраняет intent/reservation, но остаётся
отдельной commit-классификацией.

Crash recovery действует по durable state:

- `TaskPromisedUnbound` без actor binding terminalizes receipt-backed Task как
  interrupted-before-execution; callback не вызывается;
- `TaskPromisedActorBound`/TaskStore-bind intent без Task сначала exact-readback-ит
  retained reservation либо пытается её создать. Proven Link Capacity до
  reservation даёт receipt-backed cancel winner либо `task_capacity`; с live
  reservation recovery создаёт exact actor-bound queued Task с тем же
  `cancelRequested`, затем terminalizes cancelled при установленном flag либо
  interrupted-before-execution, потому что это состояние всегда `begun=false`;
- `TaskHandoffActorBound` без Task так же сначала разрешает Link Capacity либо
  использует retained reservation, затем создаёт exact actor-bound
  Queued/Working по `begun` и переносит `cancelRequested`; при `begun=false`
  terminalizes cancelled/interrupted-before-execution, а при `begun=true` —
  только `outcome_uncertain`, даже если cancel был committed до Task
  create/token. Link Capacity до reservation сохраняет staged/cancel winner,
  иначе даёт `task_capacity` до `begun` либо receipt-owned
  `outcome_uncertain` после `begun`; callback не вызывается. TaskStore Capacity
  с уже live reservation остаётся invariant violation и не публикует listener;
- `TaskReceiptOwnedActorBound` после restart terminalizes
  `TaskTerminalReceiptBacked(Failed {
  V5SafeFailureReason::OutcomeUncertain })` без TaskStore create или callback;
- exact Task без `TaskBound`/`TaskTerminalBound` допускает materialization link
  только из exact retained handoff intent с matching live
  `TaskLinkReservation`; затем проверяется actor-derived identity и Task
  terminalizes по `begun` без callback. Отсутствующая/mismatched reservation —
  corruption/fail-stop, а не reconstructed entitlement;
- `TaskBound { begun:false }` с TaskStore `Queued` либо `Working` terminalizes
  cancelled при durable TaskStore `cancelRequested=true`, иначе
  interrupted-before-execution, всегда без callback; комбинация Working
  означает crash после first commit шага 5 и до receipt `begun`;
- `TaskBound { begun:true }` требует exact TaskStore `Working` и terminalizes
  `outcome_uncertain` без callback; это crash после second commit шага 5 и до
  либо во время prepare. `begun=true` + Queued и любая иная status/identity
  комбинация являются corruption/fail-stop;
- mismatch или неподтверждаемая запись: `RestartRequested`, без публикации
  staged result.

Task completion также упорядочен: сначала TaskStore durable terminal и exact
readback, затем `TaskTerminalBound` в ledger. Crash между ними исправляется
чтением TaskStore; terminal domain callback не повторяется. Task result не
копируется в ReceiptLedger после commit `TaskBound`.

## Cancellation

Exact cancellation устанавливает durable `cancelRequested` до сигнала live
token. Повторы той же identity read-only/idempotent; mismatch не отменяет
исходный вызов. Cancel, проигравший уже committed direct/task terminal,
возвращает terminal winner.

Пока PID жив, start и cancel одной invocation используют один
`BoundStartCancelGate`. Cancel под gate заново определяет current durable
authority: до `TaskBound` пишет ReceiptLedger, после — TaskStore; только после
committed flag/terminal winner освобождает gate и сигналит token. Поэтому
наблюдение `cancelRequested=false` не является разрешением на start, а atomic
store transition и receipt `begun` образуют один live critical section.

`request_cancel_or_reserve` является exact monotonic transition для каждого
pre-TaskStore active state. `TaskPromisedUnbound` может атомарно стать
`TaskTerminalReceiptBacked(cancelled)`: actor identity и competing Task create у
него отсутствуют. `TaskPromisedActorBound` и
`TaskHandoffActorBound { begun:false }` сначала сохраняют flag, затем coordinator
exact создаёт Queued Task с тем же flag и readback-ит его, публикует
`TaskBound`, затем terminalizes cancel в TaskStore и публикует
`TaskTerminalBound`.

Для `TaskHandoffActorBound { begun:true }` и
`TaskReceiptOwnedActorBound` cancel только сохраняет flag; он не может stage-ить
`Cancelled`, потому что prepare/execute уже мог совершить side effect. При
TaskStore bind flag сначала idempotent переносится в TaskStore, затем ledger
публикует `TaskBound`; live token сигналится только после durable flag в текущем
authority. Receipt-owned Link Capacity branch продолжает хранить flag в ledger.
После `TaskBound` аналогичный flag устанавливает только TaskStore. Crash begun
handoff между receipt flag, Task create и token terminalizes
`outcome_uncertain`, а не `cancelled`, и callback не replay-ится.

После `Begun` cancellation token получает ровно две секунды
`NONCOOPERATIVE_CANCEL_GRACE`. Если `prepare`/`execute` не освобождает live
lease, `InvocationExecutor` помечает `RestartRequested`; server закрывает
listener и возвращается из daemon main без join заблокированного handler.
Именно смерть PID, а не очистка map, освобождает actor/provider resources.
Successor переводит begun receipt/Task в `Failed {
V5SafeFailureReason::OutcomeUncertain }` внутри Rust; `cancelled`
разрешён только до `Begun` либо когда callback cooperatively завершился и
durable cancel доказан до grace.

Cancel-before-submit использует durable `CancelReserved` в общем live count
pool:

- полный `ReceiptKey`, без raw request;
- максимум 64 live receipt records суммарно, encoded `CancelReserved` не более
  1 KiB и учитывается в общем actual-plus-reserved byte cap без full result
  reservation;
- TTL `7000 + 125 = 7125` мс;
- exact duplicate не создаёт новую запись и не продлевает TTL;
- startup reopen восстанавливает неизменный `expiresAt`, удаляет только уже
  expired reservation и не пересчитывает TTL от нового wall clock;
- при exact submit запись атомарно получает full result reservation, становится
  `Reserved(cancelRequested=true)`, terminalizes cancelled и не начинает
  validation/admission/preparation; если byte reservation невозможна, submit
  получает `receipt_capacity`, а side effect не начат;
- исчерпание возвращает `receipt_capacity`, а не вытесняет живую reservation.

Закрытие submit socket не является ни cancellation, ни handoff trigger. Cancel
storm не создаёт по записи на каждый пакет: одна identity имеет один state,
authenticated session capacity остаётся действующей, expired pre-submit
reservations очищаются перед новым admission.

Session arithmetic дополняется transport gate. Один frontend сохраняет 65
owner slots: anchor + 32 submit + 32 lazy cancel. Handshake slots повышаются до
32, а nonblocking accept loop за один cadence drains bounded batch до 32
connections вместо одного accept + 20 мс sleep; иначе 32 cancellations заняли
бы минимум 640 мс и не поместились бы в 125 мс budget. Barrier test обязан
доказать, что все 32 authenticated lazy-cancel sessions admitted и получают
durable/idempotent outcome в исходные 125 мс на Windows, macOS и Linux. Retry
не открывает новый cancellation deadline.

При cooperative terminal handler удаляет live receipt/task и drop-ает
ActorBound/resource lease до idle check. При двухсекундном grace expiry restart
path не ждёт handler или actor lease. Поэтому normal cleanup освобождает idle,
а non-cooperative callback переводит daemon к смерти PID вместо вечного
ожидания lease.

## Capacity, retention и idempotency horizon

Один общий лимит в 4096 records неприемлем: при TTL один час он поддерживает
лишь `4096 / 3600 = 1.14` terminal calls/s и способен остановить обычный Direct
traffic. Поэтому capacity разделена на независимые pools.

### Active/unacked payload pool

- общий live count cap равен 64 для `CancelReserved`, `Reserved`,
  `TaskPromisedUnbound`, `TaskPromisedActorBound`,
  `TaskHandoffActorBound`, `TaskReceiptOwnedActorBound`,
  `DirectTerminalUnacked` и
  `TaskTerminalReceiptBacked`: 32 текущих call admissions плюс 32 retained
  maximum-size terminal payloads в целевой рабочей точке;
- общий actual-plus-reserved byte cap равен
  `64 * MAX_DAEMON_RESPONSE_LINE_BYTES = 64 * 8454144 = 541065216` bytes;
- один canonical result не более 8 MiB, envelope не более 64 KiB;
- payload-capable live record получает один total entitlement ровно
  `MAX_DAEMON_RESPONSE_LINE_BYTES`: его `encodedBytes` уже входит в этот share,
  а `reservedResultBytes` означает только оставшийся headroom
  `MAX_DAEMON_RESPONSE_LINE_BYTES - encodedBytes`. Складывать полный MAX поверх
  actual metadata запрещено: иначе даже 64 корректных admissions не помещаются
  в заявленный literal cap. После каждой durable mutation remaining reservation
  пересчитывается так, чтобы `encodedBytes + reservedResultBytes == MAX`; этот
  entitlement сохраняется до одного из перечисленных release events.
  Terminal record включает ровно один canonical terminal payload/digest/epoch;
  transient response JSONL frame, codec prefixes/suffixes и их fingerprints не
  persisted и не учитываются второй раз. Каждый такой frame всё равно отдельно
  обязан пройти полный response-size preflight перед конкретным socket write;
- staged handoff transfer не резервирует второй ReceiptLedger count/MAX share:
  исходный `encodedBytes + reservedResultBytes == MAX` остаётся занят до direct
  commit `TaskTerminalBound`; link reservation уже гарантирует TaskStore count
  slot, а codec до write отдельно preflight-ит exact encoded terminal record
  против независимого per-record byte limit. Между TaskStore write
  и ledger commit одна и та же canonical payload физически может кратко
  находиться в обоих stores, но это один linear transfer entitlement, не две
  logical terminal admissions; full response frame не сохраняется ни в одном.
  Final ledger commit освобождает receipt MAX share, TaskStore продолжает
  учитывать только свой exact record;
- `CancelReserved` занимает один общий count slot и не более 1 KiB actual
  metadata, но не result reservation; exact submit атомарно меняет его на
  `Reserved` только если может заменить actual-only charge одним полным total
  entitlement в том же byte cap;
- каждый принятый submit держит worst-case result reservation сквозь
  `Reserved`, оба promised/handoff состояния и до одного из четырёх доказанных
  событий: exact TaskStore bind/readback, Direct ACK, expiry с physical deletion
  `DirectTerminalUnacked` либо expiry `TaskTerminalReceiptBacked`; поэтому
  rejection/cancel после promise всегда имеет заранее выделенное место для
  canonical terminal Task payload;
- terminal receipt-backed failure/cancel переводит часть entitlement из
  remaining reservation в exact encoded bytes, но сумма остаётся равна MAX и
  count slot сохраняется до expiry; completed result bytes не учитываются
  второй раз и не освобождают headroom преждевременно;
- active/nonterminal records не вытесняются; отказ capacity самого
  ReceiptLedger происходит до `begun`, а post-reservation TaskStore Capacity
  является invariant violation и не consume-ит зарезервированную receipt-owned
  ветвь как normal terminal;
- `DirectTerminalUnacked` хранится один час с terminal epoch либо до ACK;
- по достижении этого часа unacked Direct payload физически удаляется и в той
  же committed reclamation освобождает live count и exact result reservation;
  после horizon recover не обещает result и exact-ID reuse остаётся
  protocol-invalid;
- `TaskTerminalReceiptBacked` хранится один час с terminal epoch, repeated
  `task.get`/`task.result` не продлевают TTL и не освобождают quota;
- 64 abandoned receipt-backed Tasks могут исчерпать finite pool до TTL; это
  явный bounded overload, а не обещание бесконечного retention. При normal
  Direct traffic немедленный ACK освобождает slot, и 32 calls/s gate проверяет
  отсутствие starvation;
- record, ещё удерживаемый non-cooperative live attempt, TTL не удаляется:
  daemon закрывает admission/fail-stop, successor даёт uncertain terminal.

### Task-link pool

`TaskBound`, `TaskTerminalBound` и `TaskRetirementPending` не являются вторым
active-receipt record поверх materialized link. Они кодируются как три closed
state variants одного sole `TaskLifecycleLinkRecord` внутри link pool; этот
record несёт exact key/task/link metadata и dual-ID accounting, но не result.
При materialization ledger одной CAS удаляет active receipt representation,
конвертирует reservation в этот sole record и переводит key/index lookup на
него. Поэтому bound-family lifecycle не расходует 64-record receipt result
pool и не дублируется между receipt/link byte caps; result уже находится в
TaskStore. `TaskPromisedActorBound` и
`TaskHandoffActorBound` остаются в live payload pool до exact TaskStore
create/readback и commit `TaskBound`;
`TaskReceiptOwnedActorBound` остаётся там до receipt-backed terminal.
Link pool повторяет current TaskStore count limit — 4096 exact records — и
имеет отдельный byte cap 4 MiB при maximum encoded link 1 KiB. В isolated v5
state его admission доминирует TaskStore count admission: для каждого TaskStore
record существует injective exact materialized lifecycle link либо retained live
reservation, а Pending cleanup может только сохранить link после Task delete.
Поэтому startup до listener и каждая mutation обязаны доказывать
`taskStoreRecordCount <= materializedLifecycleLinkCount +
liveLinkReservationCount <= 4096`; persisted нарушение fail-stop-ит до mutation.
Retention
заканчивается только через ordered retirement соответствующего terminal Task;
sole lifecycle-link record хранит terminal TTL/expiresAt, поэтому большое число
long-running Tasks не вытесняет Direct receipts и наоборот.

Каждый handoff до TaskStore create сначала durable резервирует в этом pool один
count slot и полный 1 KiB byte entitlement. Reserved и materialized links входят
в те же 4096/4 MiB caps и публикуются отдельными exact counters. Proven link
Capacity выбирает branch до TaskStore mutation. Exact TaskStore readback
атомарно конвертирует reservation в sole `TaskLifecycleLinkRecord::TaskBound`;
staged terminal readback конвертирует её прямо в
`TaskLifecycleLinkRecord::TaskTerminalBound`, а `CommitUncertain` удерживает
reservation до reconciliation. Успешная новая reservation означает
pre-create `taskStoreRecordCount <= 4095`, поэтому count-only TaskStore create
не может вернуть normal Capacity. Если adapter всё же получает этот ответ, он
возвращает `TaskStoreCapacityInvariantViolation`, удерживает reservation/intent
и fail-stop-ит без release. Поэтому store не
может содержать Task без заранее выделенного link evidence, а crash между create
и bind восстанавливается только из exact retained intent + matching live
reservation. Directory scan без reservation не восстанавливает entitlement и
fail-stop-ит до link mutation.

На boundary 4097 proven link Capacity не вызывает TaskStore create:
pre-Begun handoff закрывается receipt-backed `Failed {
V5SafeFailureReason::TaskCapacity }`, begun handoff latch-ится receipt-owned до
actual/uncertain terminal. Это единственный normal task-publication backpressure
path. И create `CommitUncertain`, и `TaskStoreCapacityInvariantViolation`
удерживают reservation/intent и требуют fail-stop/reconciliation до listener,
но только первый означает неизвестный commit. Если staged terminal уже committed до
reserve attempt, proven LinkCapacity не выбирает `TaskCapacity`: closed Link
evidence + persisted certificate публикуют тот же staged winner как
`TaskTerminalReceiptBacked` без reservation; crash/reopen повторяет exact
readback/commit.

Terminal Task/link retirement является отдельной closed saga, а не независимым
TTL delete. Live/nonterminal `TaskBound` не expires. Если recovery видит
`TaskBound` рядом с exact terminal/expired TaskStore record, он сначала exact
readback/CAS-ом публикует `TaskTerminalBound` (допустим fused typed proof), и
только затем применяет expiry. При `now >= expiresAt` ledger сначала CAS-пишет
тот же sole lifecycle-link record CAS-переходит в `TaskRetirementPending`,
сохраняя key/task/link, terminal digest/epoch/TTL,
expiresAt, expected Task version и link/dual-ID accounting; resolver с этого
commit немедленно возвращает `task_expired`. Begin transition выдаёт opaque
one-shot `TaskRetirementAuthorization`, exact-bound к Pending link
version/identity. После reopen infrastructure-private coordinator перед delete
обязан прочитать exact Pending и вызвать coordinator-only
`authorize_existing_task_retirement`; ledger сверяет readback/current link
version и mint-ит новый nonserialized one-shot token. Старый process token после
restart недействителен. Только с
ней v5 TaskStore выполняет `delete_terminal_if_expired` и возвращает exact
`Deleted`, `AbsentExactWithPending`, `CommitUncertain` либо mismatch. Лишь первые
два proof разрешают ledger CAS удалить Pending lifecycle-link record и оба ID-index
entries. Crash до intent оставляет normal terminal Task; после intent/до delete —
retained Pending+Task; после delete/до final ledger CAS — retained Pending+absent
Task, который exact `AbsentExactWithPending` завершает. `CommitUncertain`/mismatch
сохраняют Pending и требуют fail-stop/exact readback. Active `TaskBound` + absent
Task без Pending является corruption. V5 TaskStore не lazy-delete-ит record сам.

### Compact acknowledged-tombstone pool

Product load target — 32 acknowledged Direct calls/s. Private local ACK закрывает
транспортное окно быстро, поэтому post-ACK deduplication horizon равен 15 минут,
а не часу. Count cap выводится явно:

`32 * 900 + 64 = 28864` tombstones.

Последние 64 records — две секунды headroom при target rate. Каждый tombstone
содержит только exact key, terminal digest и first-ACK epoch, ограничен 512
bytes; общий byte cap равен `28864 * 512 = 14778368` bytes. ACKed tombstones
никогда не занимают active/unacked count, reserved result bytes или Task-link
capacity. Повторное чтение/ACK не продлевает 15-minute horizon; удаляются только
records, для которых `now >= firstAckEpoch + 900s`.

Count `28 864` является capacity bound, а не требованием иметь все `28 800`
traffic tombstones одновременно в snapshot ровно на 900-й секунде. Test
вычисляет expected live high-water из raw first-ACK epochs и exact half-open
interval `[firstAckEpoch, firstAckEpoch + 900s)`. Перед отказом ACK writer
reclaim-ит только expired tombstones; если count или byte cap всё ещё заполнен,
он возвращает typed `TombstoneCapacity` и не меняет
`DirectTerminalUnacked`. Это backpressure ACK pool, а не fail-stop и не
разрешение вытеснить unexpired evidence.

Таким образом normal Direct path при немедленном ACK ограничен измеренным
writer throughput, а не 4096-record retention. Cutover блокируется, если
deterministic model либо wall-clock Windows/macOS/Linux gate не держит 32
calls/s без capacity rejection или растущего writer backlog.

At-most-once evidence сохраняется один час для unacked Direct и
receipt-backed terminal Task, на срок Task retention для Task-bound и 15 минут
после Direct ACK. Exact-ID reuse после соответствующего horizon является
protocol-invalid; conforming frontend никогда не генерирует повторно UUID и
прекращает recover/ACK retry до его окончания.
После физического удаления tombstone server больше не обещает распознать
malicious reuse: вечную дедупликацию finite store не обещает.

## Persistence layout и recovery bounds

`receipts/` не переиспользует `tasks/` и имеет собственный ownership lock.
Не более 64 live records (`CancelReserved` плюс payload-capable lifecycle)
сохраняются identity-bound atomic files; каждый файл bounded
`MAX_DAEMON_RESPONSE_LINE_BYTES`, а `CancelReserved` дополнительно ограничен 1
KiB. Task links и compact tombstones не создают десятки тысяч файлов: они
хранятся в framed append-only segments maximum 4 MiB с length, schema и
checksum. Startup строит три раздельных bounded indexes до 64 live records и
541065216 actual-plus-reserved bytes, 4096 task links/4194304 bytes и 28864 live
tombstones/14778368 bytes; reopen пересчитывает exact actual/reserved bytes и
live count из production records, сохраняет original cancel expiry, terminal
epoch и first-ACK epoch и не продлевает ни один horizon.
Поверх всех retained active records, tombstones и Task links store при open
derived/rebuild-ит два uniqueness index:
`InvocationId -> exact ReceiptKey` и `reservedTaskId -> exact ReceiptKey`.
Новый partial collision любого ID с другим exact key отклоняется до mutation;
exact key остаётся idempotent. Если rebuild находит persisted collision,
store open завершается ошибкой до mutation/listener publication — corruption
не выбирает произвольного winner и не ослабляет проверку после compaction.
Только один оборванный tail frame принимается как pre-commit. Corruption
committed frame fail-closed.

Protocol-v5 startup запрещает текущий legacy eager-open, который сам
terminalizes все TaskStore `Queued`/`Working` до чтения receipt evidence.
TaskStore сначала открывается через
`FileInvocationStoreV5::open_inspect_only(...) -> (Self,
TaskStoreRecoveryCatalog)`: constructor проверяет
ownership/schema/checksum/capacity и возвращает immutable catalog, но не меняет
active records. Затем single-threaded `ReceiptRecoveryCoordinator::reconcile`
сопоставляет exact identities и `begun`/`cancelRequested`/handoff evidence и
вызывает только typed `terminalize_recovered_exact`. Только возвращённый
`RecoveryComplete` разрешает публикацию listener. Working/Queued v5 Task без
exact receipt/handoff evidence является corruption/fail-stop; legacy v3/v4
state отделён CoreIdentity и не мигрируется этим open. Обычный legacy
`FileInvocationStore::open`, который eager-terminalize-ит active records, для
protocol-v5 state вызывать запрещено. Тот же catalog не выполняет lazy TTL
deletion: `TaskRetirementPending` reconcile-ится до listener, а absent Task при
active `TaskBound` без exact Pending evidence является corruption.

Writer вращает segment по достижении 4 MiB. Compaction запускается только при
segment rotation, startup или explicit maintenance threshold, никогда на
каждом ACK, и переписывает только live frames в новую generation. Generation
pointer меняется после fsync файлов и каталога; старую generation удаляют после
подтверждения новой. Переход payload → tombstone сначала durable append-ит
tombstone, затем удаляет payload; crash между шагами оставляет два exact
matching evidence и recovery безопасно завершает удаление. Mismatch —
corruption, не last-writer wins.

Recovery ограничивает число directory entries, segment bytes, frame size,
active payload bytes и elapsed store budget до выделения неограниченной памяти.
Writer actor имеет bounded channel и использует исходный absolute store
deadline. Неустранимая commit uncertainty вызывает process-owned fail-stop.

## Crash/edge acceptance matrix

| Сценарий | Обязательный результат | Запрещённый результат |
| --- | --- | --- |
| Strict envelope несёт `responseBudgetMs > 7000` либо oversized `workspaceHint` | bounded `invalid_request`, zero receipt/store mutation и zero validation/admission/prepare/execute | truncate/clamp, reserve или domain callback |
| Crash после strict parse, до reserve ACK | exact reserve находится либо submit получает закрытый store failure | execution без durable reserve |
| Same InvocationId либо reservedTaskId уже retained с другим exact key в active/tombstone/link set | derived uniqueness index отклоняет partial collision до mutation | second record, collision scoped только к одному pool либо last-writer wins |
| Reopen находит persisted collision одного ID с разными exact keys | store open/rebuild fails до mutation и listener publication | выбор winner, silently rebuilt ambiguous index или listener |
| Semantic validation либо `WorkspaceAdmissionError::Invalid` после valid reserve до cutoff | `DirectTerminalUnacked(Completed { DomainResult.ok=false })`, byte-equivalent Direct response и retained receipt | `Failed`, early protocol error, reserve rollback/fail-stop или потеря canonical DomainResult |
| `WorkspaceAdmissionError::Capacity` после reserve | admission ещё pre-ActorBound: `Failed { V5SafeFailureReason::WorkspaceCapacity }` в `Reserved::Unbound` Direct либо `TaskPromisedUnbound` receipt-backed owner; no callback/restart, retryable только новой invocation | Completed, protocol Rejected, post-TaskBound receipt terminal, same-key replay или listener close |
| `WorkspaceAdmissionError::RegistryFailed` после reserve | admission ещё pre-ActorBound: `Failed { V5SafeFailureReason::WorkspaceRegistryFailed }` в `Reserved::Unbound` Direct либо `TaskPromisedUnbound` receipt-backed owner, затем `RestartRequested`/listener close | Completed, post-TaskBound receipt terminal, response до terminal commit либо продолжение callback/listener |
| Та же semantic Invalid после committed promised cutoff | `TaskTerminalReceiptBacked(Completed)` под stable TaskId/InvocationId и с теми же DomainResult bytes | `Failed`, Direct fallback либо новый result |
| Crash в `Reserved::Unbound/ActorBound`, до committed promise/handoff | `DirectTerminalUnacked(cancelled | interrupted_before_execution)` по durable flag; Task не создаётся | receipt-backed Task задним числом либо domain callback после restart |
| Crash после committed promise/handoff, до `Begun` | receipt-backed Task либо exact TaskStore terminal cancelled/interrupted-before-execution | откат в Direct или domain callback после restart |
| Promise на cutoff, затем semantic Invalid | exact `TaskTerminalReceiptBacked(Completed { DomainResult.ok=false })` переживает restart; TaskStore пуст; repeated `task.result` byte-equivalent до terminal+TTL и payload учтён в live quota | `Failed`, digest-only Task terminal, потерянный result или TaskStore до ActorBound |
| Non-cooperative validation/admission после unbound promise | один общий grace 2 с, terminal-winner checks и смерть PID; successor terminalizes interrupted-before-execution | поздний bind/callback после terminal winner или вечный owner lease |
| Crash в `Reserved::Begun` без committed handoff, до provider return | `DirectTerminalUnacked(Failed { V5SafeFailureReason::OutcomeUncertain })` | receipt-backed Task задним числом, success/failure на догадке или replay |
| Side effect committed, terminal receipt не записан | `outcome_uncertain`, предметная диагностика | exactly-once claim |
| Direct terminal записан, response потерян | recover возвращает byte-equivalent result до ACK | новый execution |
| Unacked Direct достигает terminal+1h | committed physical deletion освобождает payload/count/result quota; recover после horizon не обещан | вечная reservation либо eviction до horizon |
| Submit session закрыта до cutoff, callback завершается вовремя | lifecycle остаётся Direct; `DirectTerminalUnacked` доступен через recover | немедленный Task handoff, cancel или новый deadline |
| Submit session закрыта, затем наступает original cutoff | обычный phase-aware promise/handoff ровно на исходном cutoff | handoff по disconnect либо replenished budget |
| ACK до terminal либо с Task/mismatched digest | typed rejection, исходный state byte-equivalent | premature compaction или terminal winner change |
| ACK request/response потерян | unacked result либо idempotent acknowledged tombstone с неизменным first-ACK epoch | result mismatch, replay или horizon renewal |
| Tombstone pool заполнен после expired-only reclaim | typed `TombstoneCapacity`; исходный `DirectTerminalUnacked` остаётся byte-equivalent до retry/terminal+1h | payload deletion, fake ACK success, eviction unexpired tombstone или fail-stop |
| Positive-budget повтор | исходный cutoff/state | новый семисекундный lifecycle |
| `KnownLong` после prepare | `TaskHandoffActorBound { begun:true }` → exact Working Task либо receipt-owned begun branch при proven Link Capacity | переход назад в `TaskPromisedUnbound`, повтор create после latched Capacity или потеря actor identity |
| Prepare semantic rejection race с cutoff/Task bind | Direct только из `Reserved::Begun`; pre-bind handoff stage-ит outcome; если provisional Queued/Working уже committed, closed expectation CAS-terminalizes exact TaskId/InvocationId/status/version/cancel/link-digest record, иначе `Absent` создаёт terminal; same terminal idempotent, foreign/mismatch fail-stop; затем ledger при retained handoff+reservation одним commit переходит прямо в `TaskTerminalBound`; proven begun Link Capacity использует receipt-owned terminal; already `TaskBound` всегда terminalizes TaskStore | выбор owner по historical promise, create-only staged adapter, промежуточный `TaskBound` с потерей staged payload, receipt-backed terminal после successful TaskBound или second result |
| Cutoff во время non-cooperative prepare | exact Task либо receipt-owned begun branch через `TaskHandoffActorBound`; после crash `outcome_uncertain` | orphan Task, unbound promise или второй prepare |
| Cutoff во время barrier-blocked actor admission | exact queued `TaskPromisedUnbound` в 7 с + 125 мс | direct timeout или TaskStore с request hash |
| Actor identity mismatch при TaskStore bind | fail-stop без Task mutation | last-writer-wins binding |
| Missing/foreign/stale live actor proof до bound Task start | closed authority rejection; TaskStore/receipt неизменны, callback не вызван | считать durable Task identity live capability |
| Actor proof stale после Working readback, до receipt `begun` | fail-stop; recovery terminalizes interrupted-before-execution без callback | `begun`, prepare или потеря retained lease |
| Crash между pre-create link reservation, Task create и materialized receipt link | exact retained intent + matching live reservation + Task identity reconciliation; reservation не release-ится до proven outcome | materialized link без reservation, reconstructed entitlement, второй Task, blind create/release или execution |
| Inspect-only open видит linked v5 `Queued`/`Working` | record, raw TaskId/InvocationId, stable timestamps и version остаются byte-equivalent до ReceiptLedger-led classification; Working становится interrupted/cancelled при `begun=false` и `outcome_uncertain` при `begun=true` | eager terminalization до чтения receipt evidence |
| Orphan v5 `Queued` без exact receipt/link | corruption/fail-stop до Task mutation и listener publication; raw record остаётся evidence | legacy eager interrupted terminal или доступный listener |
| Orphan v5 `Working` без exact receipt/link | corruption/fail-stop до Task mutation и listener publication; raw record остаётся evidence | guessed uncertain/cancelled terminal или callback replay |
| Active v5 Task не имеет exact receipt/link evidence | corruption/fail-stop до Task mutation и listener publication | автоматическая orphan terminalization либо доступный listener |
| TaskStore create возвращает `CommitUncertain` | handoff intent и original receipt сохраняются для exact readback/reconciliation; listener закрыт до proof | считать Capacity, повторить create, latch receipt-owned либо overwrite staged/cancel winner |
| TaskStore после successful link reservation возвращает count `Capacity` | `TaskStoreCapacityInvariantViolation`; exact handoff/reservation/staged winner остаются durable, listener закрыт; reopen либо exact-reconcile-ит valid retained intent, либо fail-stop-ит persisted count/link mismatch до listener | `task_capacity`, receipt fallback, reservation release, доступный listener или поддельный full store без links |
| Crash после TaskStore Working readback, до receipt `begun`, cancel flag false | resolver до crash показывает queued; recovery terminalizes interrupted-before-execution без callback | Working projection либо `outcome_uncertain` без begun evidence |
| Crash после receipt `begun`, до первого prepare callback | exact Working Task terminalizes `outcome_uncertain` без callback replay | restart preparation или begun+Queued acceptance |
| Cancel commit после stale false-observation, до atomic Working transition | `start_working_if_not_cancel_requested` возвращает cancel winner; no Working/`begun`/callback | read-then-write start поверх durable cancel |
| Cancel request приходит после Working, до receipt `begun` | request ждёт live gate; start durable пишет `begun`, затем cancel становится post-Begun и сигналит token | durable pre-Begun cancel вместе с callback либо false interrupted/uncertain |
| Cancel-before-submit race | exact cancelled receipt в пределах 7125 мс | cancellation чужой identity |
| Restart с live/expired `CancelReserved` | original expiry и общий count/byte accounting; только expired reclamation | renewed 7125 мс либо скрытые records сверх cap |
| Crash после cancel flag в begun handoff, до Task create/token | exact Working Task получает flag и terminal `outcome_uncertain`; callback не вызывается | `cancelled`, потерянный flag или replay |
| Cancel storm одного key | один state, bounded read-only repeats | линейный рост records/TTL |
| 32 simultaneous lazy cancels | bounded accept drain/32 handshakes и durable outcomes в 125 мс | 20 мс sleep на каждую session или renewed budget |
| Non-cooperative callback после cancel | restart через 2 с; successor terminal uncertain | вечный lease/idle wait или false cancelled |
| Cancel/complete race | один durable terminal winner | смена terminal winner |
| Native и compatibility cancel для каждого closed state | обе projections возвращают один и тот же typed state/winner; terminal/read-only states не мутируют payload/TTL | расхождение native/compat, generic success или создание нового terminal result |
| Task terminal commit, receipt terminal не записан | readback Task и дописанный `TaskTerminalBound` | повтор domain execution |
| Terminal Task/link expiry: crash before Pending, after Pending/before delete, after uncertain/deleted/before ledger cleanup | before Pending Task remains readable; after Pending resolver returns `task_expired`; reopen exact-reads Pending and coordinator-only `authorize_existing_task_retirement` mints a fresh nonserialized token bound to its current link version, while the old token is rejected; only that authorization deletes exact expired terminal Task, and exact Deleted/Absent-with-Pending proof resumes final CAS removal of sole link record/dual-ID accounting; uncertainty/mismatch retains Pending+fail-stop | v5 TaskStore lazy-delete, live `TaskBound` expiry, absent Task without Pending, reuse of pre-crash token, orphan fail-stop from normal expiry, link/index leak or blind cleanup |
| Staged handoff: TaskStore terminal committed, ledger ещё handoff | exact retained staged payload/reservation/certificate + terminal Task readback дают direct `TaskTerminalBound`; staged bytes удаляются только этим commit | промежуточный `TaskBound`, потеря known outcome, второй callback, late oversize или реконструкция reservation |
| Repeated get/result receipt-backed Task | тот же canonical snapshot до terminal+1h; Direct ACK отклонён | first-read deletion, TTL renewal или потеря payload после restart |
| Result > 8 MiB | owner-specific bundle preflights canonical payload, persisted record(s) и transient frame до mutation/write; before first staged mutation certificate covers staged receipt, conservative final Task/`TaskTerminalBound`/wire maxima and Link Capacity fallback record/wire, including max-u64/Queued/Working/cancel/newline shapes; oversized candidate becomes bounded closed `result_too_large` before staging | unbounded serialization, staged winner with late oversize/reclassification, persisted duplicate response frame or fallback after prepared frame |
| 64 live receipt boundary | 65-й reserve/cancel получает `receipt_capacity` до `Begun`; exact expiry/ACK/bind освобождает quota | side effect без worst-case result reservation |
| Task/link boundary 4097 до `Begun` | link entitlement резервируется до create; existing 4096 Tasks/links неизменны; unstaged branch даёт receipt-backed `task_capacity`, а staged winner использует certified LinkCapacity fallback без reservation и переживает reopen byte-equivalent; callback отсутствует, listener доступен | TaskStore mutation без link reserve, eviction, повтор create/link, `TaskCapacity` поверх staged winner либо fail-stop на proven Link Capacity |
| Task/link boundary 4097 после `Begun` | link entitlement резервируется до create; `TaskReceiptOwnedActorBound`; live attempt даёт actual receipt-backed terminal, crash — `outcome_uncertain`; staged terminal остаётся winner | TaskStore mutation без link reserve, eviction, повтор create/link, потеря result либо `task_capacity` поверх staged outcome |
| Tombstone pool под target load | 15 минут post-ACK evidence независимо от active quota | starvation active quota |
| Store commit невозможно доказать | listener закрыт, `RestartRequested` | staged response или fallback executor |

## Предполагаемые Rust files и interfaces

### Новые files

- `crates/unica-coder/src/application/receipt_ledger.rs` —
  `ReceiptKey`, `RequestIdentity`, `ReceiptState`, `ReceiptTerminalOutcome`,
  `ActorBindingClaim`, one-shot actor/start token types, linear prepared
  publication types, `ReceiptLedger` port и typed errors;
- `crates/unica-coder/src/application/receipt_ledger_actor.rs` — bounded
  sole-writer actor и absolute-deadline operations;
- `crates/unica-coder/src/application/invocation_store_v5.rs` — distinct
  `V5StoredInvocationRecord`, `V5SafeFailureReason`, total legacy-to-v5
  conversion и v5-only Task transitions; current `invocation_store.rs` и its
  schema-v2 decoder не меняются;
- `crates/unica-coder/src/application/invocation_v5.rs` — distinct
  pure `V5InvocationExecutor`/state machine и application claim/token/port types;
  actual actor/resource leases, verifier и gate coordinator здесь запрещены;
  current `invocation.rs` не получает v5 branches;
- `crates/unica-coder/src/infrastructure/receipt_ledger.rs` — owner-only file
  implementation, active files, tombstone segments, recovery/compaction и
  count+byte catalogs;
- `crates/unica-coder/src/infrastructure/task_store_v5.rs` — distinct strict v5
  record store/decoder and inspect-only recovery; current `task_store.rs`
  сохраняет literal schema-v2 acceptance;
- `crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs` — distinct
  `V5ClientRequest`/`V5ServerResponse`, strict v5 decoder и full-key
  recovery/ACK/cancel; current `protocol.rs` types/decoder не меняются;
- `crates/unica-coder/src/infrastructure/daemon/invocation_service.rs` —
  protocol-neutral shared `CanonicalInvocationService`, `ActorBoundInvocation`,
  `ActorBoundExecution` и capability helpers, извлечённые semantic-neutral из
  v3 `server.rs`; v3 и v5 runtimes используют один service seam;
- `crates/unica-coder/src/infrastructure/daemon/runtime_v5.rs` — distinct v5
  server/runtime composition и infrastructure-private `InvocationCoordinator`,
  который единственный владеет actual actor/resource leases, verifier и gate;
- `crates/unica-coder/src/infrastructure/daemon/terminal_codec_v5.rs` — sole
  typed terminal serializer/preflight constructor; arbitrary encoded bytes не
  являются port input;
- `crates/unica-coder/src/infrastructure/receipt_ledger_test_evidence.rs` —
  только под non-default feature `receipt-ledger-test-support`: sealed
  `ReachedProductionBoundary`/`ProductionMissingTransitionEvidence`, которые
  могут mint-ить только allowlisted production v5 protocol/runtime/store sites
  после фактического входа в boundary; root facade может только прочитать и
  сериализовать opaque evidence;
- `crates/unica-coder/src/interfaces/task_projection_v5.rs` и
  `crates/unica-coder/src/application/v13/task_tools_v5.rs` (либо exact
  equivalent distinct modules) — frozen native/compatibility matrix без open
  `InvocationFailure`; current projectors не меняются;
- `crates/unica-coder/src/receipt_ledger_test_support.rs` — только под
  non-default feature `receipt-ledger-test-support`: thin root facade над
  production v5 shell и sealed evidence, manual epoch/monotonic controls, named
  barriers/crash points, bounded fault injection и observable
  snapshots/counters для integration matrix; он не содержит второй lifecycle,
  hash/canonical codec или action-to-boundary classifier.
- `scripts/ci/check-v5-daemon-ownership.py` — static allowlist guard для actor
  binding-token mint/use, live lease verifier sites, terminal artifact
  construction, application-to-infrastructure imports и v5 terminal
  serialization paths.

Минимальный application port:

```text
canonical_v5_terminal(outcome) -> V5CanonicalTerminal
V5TerminalCodecPort::prepare_direct_or_receipt_task(exact_key,
    canonical_terminal, terminal_epoch, receipt_expected_version, terminal_owner)
    -> PreparedReceiptTerminalPublication
V5TerminalCodecPort::prepare_bound_task(exact_key, canonical_terminal,
    terminal_epoch, task_expected_version, lifecycle_link_expected_version,
    exact_link_digest) -> PreparedBoundTaskTerminalPublication
V5TerminalCodecPort::prepare_handoff_stage(exact_key, task_identity,
    exact_link_digest, canonical_terminal, terminal_epoch,
    receipt_expected_version)
    -> PreparedStagedReceiptPublication {
        receipt_record, staged_terminal_transfer_size_certificate
    }
V5TerminalCodecPort::prepare_staged_handoff_terminal(exact_key,
    committed_staged_handoff_receipt, staged_task_publication_expectation,
    exact_link_digest)
    -> PreparedStagedHandoffTerminalPublication
V5TerminalCodecPort::prepare_staged_capacity_fallback(exact_key,
    committed_staged_handoff_receipt,
    staged_capacity_evidence_with_certificate, exact_link_digest)
    -> PreparedStagedCapacityFallbackPublication
reserve(key, original_cutoff, deadline) -> ReservedReceipt
request_cancel_or_reserve(key, fixed_expires_at, deadline) -> CancelResolution
promise_unbound_task(key, deadline) -> PromisedTaskReceipt
bind_actor(key, ActorBindingClaim { identity, generation }, deadline)
    -> (ActorBoundReceipt, V5ActorBindingToken)
bind_promised_actor(key, ActorBindingClaim { identity, generation }, deadline)
    -> (PromisedActorBoundReceipt, V5ActorBindingToken)
mark_reserved_begun(key, actor_binding_token, receipt_expected_version, deadline)
    -> BegunOrCancelWinner
authorize_bound_task_start(key, actor_binding_token,
    exact_current_task_bound_proof, lifecycle_link_expected_version, deadline)
    -> PostWorkingActorAuthorization
mark_bound_task_begun(key, post_working_authorization,
    versioned_working_readback, lifecycle_link_expected_version, deadline)
    -> BegunTaskReceipt
begin_bound_task_handoff(key, task_identity, deadline) -> BoundHandoffIntent
reserve_task_link(key, task_identity, maximum_link_bytes, deadline) -> TaskLinkReservation
resolve_task_link_capacity(key, proven_link_capacity,
    receipt_expected_version, deadline) -> ReceiptOwnedTaskOrTerminal
stage_bound_handoff_terminal(key, PreparedStagedReceiptPublication,
    receipt_expected_version, deadline)
    -> CommittedStagedHandoffReceiptWithCertificate
bind_task(key, task_link_reservation, exact_task_record,
    receipt_expected_version, deadline)
    -> (TaskBoundLifecycleLinkReadback, TaskBoundLinkAuthorization)
commit_staged_handoff_terminal(key, task_link_reservation,
    exact_task_terminal_readback, PreparedTaskLifecycleLinkRecord, PreparedWireFrame,
    receipt_expected_version, deadline)
    -> CommittedTaskPublication { lifecycle_link, prepared_wire_frame }
commit_staged_capacity_terminal(key,
    StagedLinkCapacityEvidence { proven_link_capacity },
    PreparedStagedCapacityFallbackPublication,
    receipt_expected_version, deadline)
    -> CommittedReceiptTaskPublication { terminal, prepared_wire_frame }
publish_direct_terminal(key, PreparedReceiptRecord, PreparedWireFrame, deadline)
    -> CommittedDirectPublication { receipt, prepared_wire_frame }
publish_receipt_backed_task_terminal(key, PreparedReceiptRecord,
    PreparedWireFrame, deadline)
    -> CommittedReceiptTaskPublication { terminal, prepared_wire_frame }
publish_bound_task_terminal(key, exact_task_terminal_readback,
    PreparedTaskLifecycleLinkRecord, lifecycle_link_expected_version,
    PreparedWireFrame, deadline)
    -> CommittedTaskPublication { lifecycle_link, prepared_wire_frame }
begin_task_retirement(key, task_terminal_bound_link_expected_version, now, deadline)
    -> (TaskRetirementPending, TaskRetirementAuthorization)
authorize_existing_task_retirement(pending_exact_readback,
    pending_expected_link_version, deadline) -> TaskRetirementAuthorization
complete_task_retirement(key, exact_task_store_retirement_proof,
    pending_expected_link_version, deadline) -> RetiredTaskLinkAndIdentity
acknowledge_direct(key, terminal_digest, deadline)
    -> AcknowledgedReceipt | TombstoneCapacity
recover(key, deadline) -> ReceiptState
resolve_task(task_id, deadline) -> ReceiptBackedOrStoredTaskSnapshot
```

Каждый mutating method обязан возвращать exact committed record либо typed
`CommitUncertain`; caller reconcile-ит только store transition и не получает
domain callback. `bind_actor`/`bind_promised_actor` принимают application
`ActorBindingClaim { identity, generation }` и возвращают committed receipt +
one-shot ledger token. `mark_reserved_begun` принимает только
`Reserved::ActorBound`, exact token после infrastructure-private live lease
verification, exact expected receipt version и CAS, consume-ит token и атомарно
проверяет `cancelRequested=false`. Для Task path
`authorize_bound_task_start` после такой же private verification consume-ит
actor token при exact current sole lifecycle-link version/CAS и возвращает distinct one-shot
`PostWorkingActorAuthorization`. Coordinator удерживает private
`BoundStartCancelGate` через exact TaskStore start; `mark_bound_task_begun`
принимает только `TaskBound { begun:false }`, эту post-Working authorization,
exact versioned readback и expected lifecycle-link version/CAS. Application port не
импортирует и не принимает infrastructure guard type. Missing/foreign/stale evidence закрыто отклоняется без
`begun`, mutation или callback; lease, устаревший после Working readback, не
допускает `begun`, вызывает fail-stop, а recovery закрывает Task как
interrupted-before-execution.

TaskStore port получает отдельную sole-writer операцию
`start_working_if_not_cancel_requested(task_bound_link_authorization,
exact_current_task_bound_proof, task_identity, expected_version, deadline)
-> StartedVersionedReadbackWithLinkEvidence | CancelOrTerminalWinner`. Она одной
транзакцией перечитывает current version/flag/status и либо пишет Working, либо
возвращает winner; successful readback несёт remaining exact link evidence,
которое вместе с удерживаемым coordinator
`PostWorkingActorAuthorization` проверяет ledger. Stale observation/CAS не может
начать Task.
Terminal retirement использует отдельную sole-writer операцию
`delete_terminal_if_expired(task_retirement_authorization, exact_task_identity,
expected_terminal_version, now, deadline) -> DeletedProof |
AbsentExactWithPendingProof | CommitUncertain | Mismatch`. Authorization
создаётся только committed `TaskRetirementPending`, TaskStore проверяет exact
terminal identity/version/expiresAt и не выполняет ambient/lazy TTL deletion.
Для staged branch отдельная TaskStore operation
`publish_or_readback_staged_terminal(task_link_reservation,
prepared_task_record, StagedTaskPublicationExpectation, deadline)` принимает только
matching live pre-bind reservation и staged-handoff task piece. Она возвращает
exact terminal readback плюс untouched lifecycle-link/wire pieces,
`TaskStoreCapacityInvariantViolation` либо `CommitUncertain`. Оба error paths
сохраняют durable staged receipt/reservation для recovery; Capacity не возвращает
fallback authority и не разрешает receipt terminal, а uncertainty удерживает
весь prepared bundle для exact reconciliation.
`Absent` создаёт terminal record; `ExactProvisional` одним
CAS terminalize-ит только exact TaskId/InvocationId/Queued-or-Working/version/
cancel/link-digest readback; exact same terminal idempotent, а foreign либо
status/identity/version/cancel/link-digest/terminal
mismatch закрыто отклоняется. `TaskBound` и `TaskBoundLinkAuthorization` эта
ветка не создаёт.
TaskStore `create` не принимается без live pre-bind `TaskLinkReservation`.
`bind_task` consumes её только после exact create/readback и возвращает distinct
opaque `TaskBoundLinkAuthorization`, bound exact ReceiptKey/TaskId/link digest и
TaskBound generation. Только эта post-bind capability плюс exact current
TaskBound proof принимаются `start_working_if_not_cancel_requested`; уже
consumed reservation повторно использовать запрещено. Start readback и
`mark_bound_task_begun` сохраняют exact authorization binding; coordinator
непрерывно удерживает свой private gate поверх обеих store-операций.

Pre-bind staged/cancel terminal release-ит reservation только в том же committed
terminal transition; `CommitUncertain` и `TaskStoreCapacityInvariantViolation`
сохраняют durable reservation+intent для recovery и не оставляют reusable live
token. Successful
bind consumes reservation навсегда. `TaskBoundLinkAuthorization` invalidates on
terminal/cancel winner, generation change, Task/link expiry or PID death;
materialized link при этом живёт до Task retention, а restart recovery не
reconstructs start capability и никогда не запускает callback. Static ownership
guard ограничивает constructors/conversions обоих opaque tokens exact
ledger/coordinator call sites.

Startup/recovery seam закрыт следующими typed interfaces:

```text
FileInvocationStoreV5::open_inspect_only(root, clock, deadline)
    -> (FileInvocationStoreV5, TaskStoreRecoveryCatalog)
terminalize_recovered_exact(task_identity, expected_version,
    RecoveryTerminalReason::{Cancelled, InterruptedBeforeExecution, OutcomeUncertain},
    deadline) -> V5StoredInvocationRecord
ReceiptRecoveryCoordinator::reconcile(receipt_catalog, task_catalog, deadline)
    -> RecoveryComplete
```

`TaskStoreRecoveryCatalog` содержит exact identity, version, status и durable
cancel flag; получение catalog не terminalize-ит `Queued`/`Working`.
`begin_bound_task_handoff` — только `Reserved::ActorBound/Begun`.
`stage_bound_handoff_terminal` consume-ит только codec-built
`PreparedStagedReceiptPublication`, проверяет exact expected receipt version/CAS,
persisted-byte/MAX accounting и embedded
`StagedTerminalTransferSizeCertificate` и одной mutation публикует staged
canonical payload с bounded certificate evidence. Он возвращает typed exact
readback/version `CommittedStagedHandoffReceiptWithCertificate`; raw outcome,
digest, bounds, certificate или adapter bytes не принимаются. Later transfer
codec принимает только этот opaque committed readback, проверяет exact
protocol/CoreIdentity/key/task/link/terminal/schema/limit binding, сам rehydrate-ит
canonical terminal/digest/epoch и строит exact pieces только если их размеры не
превышают certified upper bounds. Proven LinkCapacity возвращает closed
`StagedLinkCapacityEvidence` без создания reservation. Только
`prepare_staged_capacity_fallback` + `commit_staged_capacity_terminal` consume
эту evidence и могут сохранить прежний staged winner как certified
receipt-backed Task, доказав отсутствие reservation. После first staged
commit terminal нельзя reclassify-ить как `ResultTooLarge`; late mismatch или
oversize означает invariant corruption/fail-stop без mutation winner.
`publish_receipt_backed_task_terminal` принимает только состояния, где ledger
ещё current owner: оба promised states, pre-Begun
`TaskHandoffActorBound` при proven Link Capacity и
`TaskReceiptOwnedActorBound`; already staged/terminal outcome всегда остаётся
winner. Semantic prepare rejection в ordinary handoff не публикуется этим
методом: оно stage-ится, затем dedicated TaskStore terminal create/readback и
`commit_staged_handoff_terminal` переводят retained handoff прямо в
`TaskTerminalBound`, не через `TaskBound`. А
`publish_bound_task_terminal` требует exact TaskStore readback. Эти typed
preconditions не дают KnownLong потерять actor identity или result ownership.
`bind_task` требует exact expected receipt version/CAS, live reservation и
TaskStore readback с monotonic `cancelRequested`, не меньшим receipt flag; bind
atomically прекращает cancel-authority ReceiptLedger. Coordinator непрерывно
удерживает private gate от TaskStore create/readback через этот ledger commit.
Cancel, выигравший gate до TaskStore bind и commit `TaskBound`, переносится в
TaskStore; cancel после commit `TaskBound` пишет уже только TaskStore.
После successful reservation coordinator удерживает тот же private
`BoundStartCancelGate` от TaskStore create attempt до bind/fail-stop. Adapter
преобразует count `Capacity` только в `TaskStoreCapacityInvariantViolation`;
никакой ReceiptLedger terminal method этот error не принимает. Staged terminal,
committed cancel, intent и reservation остаются byte-equivalent для reopen;
unstaged state тоже не становится `TaskReceiptOwnedActorBound`/`task_capacity`.
`CommitUncertain` остаётся отдельным exact-readback path.
Static import/ownership/call-site guard доказывает, что application не видит
`BoundStartCancelGate`, а только `runtime_v5::InvocationCoordinator` вызывает
в правильном порядке эту gated последовательность ledger/TaskStore операций.

### Изменяемые files реализации

- `crates/unica-coder/Cargo.toml` — объявить non-default
  `receipt-ledger-test-support` и required-feature integration target; default и
  package builds не включают этот код;
- `crates/unica-coder/src/lib.rs` — conditionally export только root
  `#[doc(hidden)]` facade; существующий crate-private `test_support` и production
  interfaces не расширяются;
- `crates/unica-coder/src/composition.rs` — открыть sibling stores и собрать
  их sole-writer actors только в injected hidden-v5 composition; default
  `production()` остаётся v3 до W0c;
- `crates/unica-coder/src/application/mod.rs` — зарегистрировать два новых
  application modules;
- `crates/unica-coder/src/application/invocation_v5.rs` — реализовать отдельный
  pure `V5InvocationExecutor`/state machine: заменить in-memory
  receipt/pending-cancel authority на ReceiptLedger; durable `ActorBound` и `Begun` предшествуют
  `prepare`, task resolver объединяет promised/receipt-backed terminal и
  TaskStore без replay; current `ExecutionClass::KnownLong` после prepare
  маршрутизируется только через `begin_bound_task_handoff`; current
  `application/invocation.rs` не меняется. Actual leases/verifier/gate здесь не
  хранятся и infrastructure не импортируется;
- `crates/unica-coder/src/application/invocation_store_v5.rs` — добавить
  side-by-side `V5StoredInvocationRecord` и закрытый `V5SafeFailureReason` с
  total conversion пяти legacy reasons плюс `OutcomeUncertain`/`TaskCapacity`/
  `WorkspaceCapacity`/`WorkspaceRegistryFailed`,
  exact task-link/start-working readback и startup-only
  terminalization `Queued`/`Working` без callback, а также durable monotonic
  `request_cancel_exact` и inspect-only recovery catalog, не превращая TaskStore
  в ReceiptLedger;
- `crates/unica-coder/src/infrastructure/mod.rs` — зарегистрировать file ledger;
- `crates/unica-coder/src/infrastructure/task_store_v5.rs` — idempotent exact
  create/readback с `cancelRequested`, atomic
  `start_working_if_not_cancel_requested`, post-`TaskBound` cancel authority и
  `open_inspect_only` без eager terminalization, coordinated terminal retention,
  без receipt/ACK state;
- `crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs` — protocol v5,
  `ReceiptKey`, recover/ACK messages/responses и closed codes;
- `crates/unica-coder/src/infrastructure/daemon/identity.rs` — CoreIdentity/state
  selector parameterization и fork tests для protocol v5 без смены default;
- `crates/unica-coder/src/infrastructure/daemon/invocation_service.rs` — сначала
  semantic-neutral извлечь private `CanonicalInvocationService`,
  `ActorBoundInvocation`, `ActorBoundExecution` и capability helpers из
  `server.rs`; `server.rs` получает только narrow import, а `v13_service.rs`
  меняет только import/impl path. До подключения v5 byte-for-byte v3 JSONL,
  serde rejection и daemon behavior tests обязаны пройти без изменения;
- `crates/unica-coder/src/infrastructure/daemon/runtime_v5.rs` — открыть sibling
  stores, reserve сразу после strict parse, выполнить infrastructure-private
  `InvocationCoordinator` с actual leases/verifier/gate, handoff coordinator и
  ReceiptLedger-led startup recovery над inspect-only Task catalog до listener
  publication; записывать actor safe identity до TaskStore/prepare, drain accept
  batch и поддержать 32 concurrent handshakes; v3 `server.rs` остаётся v3 по
  wire/behavior, но narrow extraction/import edit разрешён;
- `crates/unica-coder/src/infrastructure/daemon/client_v5.rs` — exact recovery,
  explicit Direct ACK handle, ACK-loss retry, receipt-backed Task lookup и
  запрет budget renewal; current `client.rs` остаётся v3;
- `crates/unica-coder/src/infrastructure/daemon/mod.rs` — зарегистрировать
  shared service seam, side-by-side modules и process/crash fixtures;
- `crates/unica-coder/src/interfaces/daemon.rs` — additive closed dispatch без
  нового CLI/env/test selector: existing strict `--core-identity` parse остаётся
  прежним; только exact known `CoreIdentity::production_v5()` выбирает
  `runtime_v5::run_daemon`, а любой другой уже принимаемый canonical 64-hex
  CoreIdentity продолжает v3 `server::run_daemon`. Unknown/invalid syntax
  отклоняется прежним parser. Endpoint helper принимает typed protocol identity;
  default connect остаётся v3 до W0c, когда меняется default constructor;
- `crates/unica-coder/src/interfaces/task_projection_v5.rs` и
  `crates/unica-coder/src/application/v13/task_tools_v5.rs` — реализовать exact
  frozen matrix и ACK после успешной final Direct projection; current
  `interfaces/mcp.rs`, `task_projection.rs` и `v13/task_tools.rs` получают только
  W0c composition switch, без изменения legacy types/decoder/mapping или
  `tools/list`/public schemas.

### Test-only integration boundary and reachability order

Integration test собирает library как внешнюю dependency и потому не видит
`pub(crate)` stores, daemon injections и `#[cfg(test)]` support. Wide always-on
Rust API либо 61 тест, падающий на setup, не являются допустимым доказательством.
Матрица использует non-default feature `receipt-ledger-test-support`; её API
scenario-oriented и не отдаёт вызывающему коду raw stores, forgeable actor
proof, arbitrary callback или production selection switch. Трёхплатформенный CI
вызывает отдельный feature-enabled target; обычная поставка feature не включает.
Scenario передаёт fixture только clock mode и primitive actions: готовый
`expected_missing` либо другой expected-shaped response в input запрещён.

До root facade обязательно появляется минимальный production-reachability
shell. В `application/receipt_ledger.rs` живут checked digest types и единственные
authorities `request_scope_hash(&str) -> Result<RequestScopeHash, _>`,
`receipt_key_digest(&ReceiptKey) -> ReceiptKeyDigest`,
`task_link_digest(&TaskLinkIdentity) -> TaskLinkDigest` и
`canonical_v5_terminal(&ReceiptTerminalOutcome) -> Result<V5CanonicalTerminal,
_>`. `protocol_v5.rs` выполняет реальный bounded frame read и strict v5 envelope
decode/probe; `runtime_v5.rs` имеет настоящий v5 entry, использует shared
`CanonicalInvocationService` и открывает `infrastructure/receipt_ledger.rs`, а
store выполняет реальный generation/open/exact-inspect step. Это минимальный
side-by-side production shell, а не test implementation: exact known
`CoreIdentity::production_v5()` достигает его через закрытый existing
`--core-identity` dispatch, произвольные уже допустимые 64-hex identities и
default `production()` остаются v3 до W0c. Нового CLI/env/MCP/test selector нет.

Фактические `protocol_v5`, `runtime_v5` и ReceiptLedger sites после своего
production step mint-ят feature-gated sealed `ReachedProductionBoundary` и, если
следующий переход ещё не реализован, `ProductionMissingTransitionEvidence`.
Constructors имеют видимость не шире `crate::infrastructure`; поля доступны root
facade только через read-only accessors. Static ownership guard ограничивает
mint allowlist этими production sites и запрещает constructor/mapping в
`receipt_ledger_test_support.rs`. Evidence содержит реально достигнутые boundary,
protocol identity, optional post-attempt event, exact store generations и
bounded fingerprint. Она не выводится из `ActionKind`, имени scenario или
expected code. Поэтому появление GREEN раннего перехода естественно передаёт
execution следующей production operation, и typed missing evidence приходит уже
от неё; никакого facade switch `action -> boundary/code/event` нет.

Только после shell и sealed evidence добавляется thin feature facade с exact ABI:

```text
execute_scenario_json(request: &str) -> Result<String, String>
request_scope_hash_for_test(workspace_hint: &str) -> String
receipt_key_digest_for_test(invocation_id: &str, reserved_task_id: &str,
    core_identity_digest: &str, tool_wire_name: &str,
    normalized_arguments_hash: &str, request_scope_hash: &str) -> String
task_link_digest_for_test(receipt_key_digest: &str, task_id: &str,
    invocation_id: &str, workspace_identity_hash: &str) -> String
canonical_v5_terminal_for_test(terminal_json: &str) -> (Vec<u8>, String)
```

Четыре wrappers strict-parse test input в production application types и
вызывают только указанные authorities. `execute_scenario_json` маршрутизирует
primitive action в соответствующую typed production operation, добавляет только
action index/kind для correlation и сериализует полученное sealed evidence либо
raw production observation. Он не может сам mint-ить reached/missing boundary и
не содержит second ReceiptLedger, terminal codec или ожидаемый verdict.

Primitive setup не содержит `totalEntitlementBytesEach`: entitlement выводится
из production state и exact inspector accounting. Observation возвращает raw
safe evidence, а verdict вычисляет test: canonical TaskId/InvocationId,
client/server key components, post-handshake protocol event и safe frame
fingerprints, receipt terminal/first-ACK epochs, Task
`createdAt`/`updatedAt`/`ttlMs`/`pollIntervalMs`, exact store records,
projection records, callback counters и staged/recovered terminal digests.
Fixture не возвращает `roundTripExact`, `daemonRecomputed`,
`callerDigestUsedAsAuthority`, `splitBrain`, `receiptKeyMatches`,
`taskIdentityMatches`, `stagedTerminalPreserved` или другой boolean verdict.
Raw arguments/workspaceHint/path/secret при этом не экспонируются.

Третий независимый semantic+concurrency review отклонил 5 095-line snapshot
`eec6a2102ec6734b3522f8af3ddedebfababdb48d1ede2ffa9c8985ac2b6bb21`:
он всё ещё принимал entitlement из fixture, не имел перечисленных raw
identity/timestamp/failure observations, self-certified protocol/identity/crash
через booleans и не доказывал post-attempt production event для protocol probe.
Это rejection сохраняет RED-authoring gate открытым; declaration bridge нельзя
замораживать по этому snapshot.

`responseBudgetMs` в test wire остаётся в `0..=7000`. Дополнительные 125 мс —
только transport serialization margin после operation cutoff; они не входят в
durable `OriginalCutoffDescriptor` и не могут быть куплены повтором. Значение
7 125 мс отдельно сохраняется только как absolute TTL `CancelReserved`.

TDD проходит пять различимых состояний в фиксированном порядке:

1. approved test file с 61 exact names сначала сохраняет единственный
   compile-RED `E0432` на отсутствующем root support contract;
2. до facade создаются application hash/canonical authorities, минимальный
   production v5 protocol/runtime/ReceiptLedger reachability shell и sealed
   production-minted evidence;
3. затем добавляется thin feature facade с exact ABI выше; default/package build
   его не компилирует;
4. feature-enabled target обнаруживает ровно 61 tests; обязательный
   reservation/identity/protocol smoke set доходит до реального daemon/store, а
   каждый оставшийся failure является typed functional RED от фактически
   достигнутой production operation, не fixture panic, `todo!`, setup timeout
   или facade-owned action-to-boundary mapping;
5. последовательные W0a/W0b пакеты переводят свои named filters в GREEN.

W0a не закрывает ACK/recovery/handoff tests, владельцем которых остаётся W0b.
Protocol-v5 types и state fork в W0a доступны только injected hidden path;
активный default v3 переключается на v5 атомарно в W0c вместе с active
successor/derived records.

## Test-first implementation slices

1. RED protocol/identity tests:
   `v5_rejects_v3_v4_and_strictly_round_trips_receipt_messages`,
   `receipt_key_is_canonicalized_identically_by_client_and_server` и
   `response_budget_is_not_receipt_identity`; отдельно mismatch каждого exact
   поля, frozen receipt/request-scope/task-link domain+u32-BE-length-prefix digest vectors, partial ID collision
   в active/tombstone/link retention, persisted collision на reopen, unknown
   fields, `responseBudgetMs > 7000`, oversized workspaceHint и
   CoreIdentity/state-dir fork. Protocol probe обязан вернуть raw post-attempt
   production event/evidence, а не fixture verdict. Отдельно semantic-neutral
   extraction `daemon/invocation_service.rs` сравнивает v3 JSONL/strict serde/
   behavior до и после, а process case проводит real v5 client через spawned
   `--daemon` и сохраняет произвольный valid 64-hex v3 fixture identity.
2. RED in-memory port contract tests:
   `known_long_requires_begun_bound_handoff_intent`,
   `unbound_promise_terminal_keeps_canonical_payload_until_task_ttl`, все
   остальные state transitions, terminal winner, positive-budget replay,
   semantic Invalid `Completed(DomainResult.ok=false)` в Direct и unbound
   promised owners, prepare rejection во всех четырёх durable-owner branches,
   `V5SafeFailureReason::{OutcomeUncertain, TaskCapacity, WorkspaceCapacity,
   WorkspaceRegistryFailed}`, premature/Task ACK,
   ACK request/response loss и full tombstone pool с неизменным unacked Direct.
3. RED file-store tests:
   `receipt_backed_task_terminal_survives_reopen_byte_equivalent`,
   `cancel_reserved_reopens_with_original_7125ms_expiry`,
   `task_store_inspect_only_open_preserves_queued_and_working_until_receipt_reconciliation`,
   `receipt_led_startup_distinguishes_working_begun_false_from_begun_true`,
   `v5_active_task_without_exact_receipt_link_fail_stops_before_listener`, crash checkpoints
   каждой atomic replace/append/fsync, tail frame, corruption, dual evidence и
   compaction generation, orphan Queued и orphan Working отдельно, а также
   Task-create `CommitUncertain` без Capacity masquerade/repeated create.
4. RED capacity tests:
   `cancel_reserved_shares_live_64_count_without_result_reservation`,
   `promised_and_handoff_states_hold_worst_case_result_quota`,
   `task_bind_direct_ack_and_receipt_terminal_expiry_release_exact_quota`,
   `direct_unacked_expiry_deletes_payload_and_releases_exact_quota`,
   `link_capacity_before_begun_terminalizes_receipt_backed_without_callback`,
   `task_store_capacity_after_reservation_is_invariant_violation_and_fail_stops`,
   `link_capacity_preserves_staged_terminal_winner`,
   `receipt_owned_begun_crash_terminalizes_outcome_uncertain_without_task_store`,
   `task_store_4097_boundary_preserves_existing_tasks_and_listener_availability`, exact
   64/4096/28864 count и 541065216/4194304/14778368 byte boundaries, exact
   reopen accounting, tombstone overflow, segmented rotation, expired-only
   eviction и high-water, вычисленный test из raw first-ACK epochs.
5. RED executor tests:
   `reserve_precedes_validation_admission_prepare`,
   `restart_reserved_without_committed_handoff_never_invents_task`,
   `restart_begun_without_committed_handoff_is_direct_outcome_uncertain`,
   `validation_rejection_after_promise_recovers_receipt_backed_terminal`,
   `known_long_after_prepare_never_becomes_unbound_promise`, seven-second
   promise under admission barrier,
   `submit_disconnect_before_cutoff_preserves_direct_lifecycle`, durable
   `ActorBound`/`Begun`,
   `bound_task_start_rejects_missing_foreign_stale_actor_proof_without_mutation`,
   `bound_task_start_rechecks_proof_after_working_readback`,
   `direct_actor_bound_cancel_vs_mark_begun_has_one_linearized_winner`, запрет
   request hash в TaskStore, direct recovery и no callback on replay.
6. RED cross-store process tests:
   `begun_cutoff_intent_survives_crash_before_task_create`,
   `cancel_or_restart_before_actor_bind_terminalizes_without_callback`,
   `working_readback_before_receipt_begun_recovers_interrupted_without_callback`,
   `receipt_begun_before_prepare_recovers_outcome_uncertain_without_callback` и
   `begun_receipt_with_queued_task_is_fail_stop`,
   `begun_handoff_cancel_crash_before_task_create_or_token_is_uncertain`,
   `cancel_flag_transfers_monotonically_at_task_bind`,
   `bound_false_cancel_flag_recovers_cancelled_without_callback`, каждый crash
   window intent/create/link и task-terminal/receipt-terminal, включая actual
   daemon restart.
7. RED side-effect fixture: child fsync-ит marker и погибает до terminal receipt;
   successor выдаёт `outcome_uncertain`, marker один, execution count один.
8. RED cancellation/lease fixtures:
   `unbound_validation_and_admission_share_one_two_second_fail_stop_grace`, 32
   barrier-synchronized lazy sessions в 125 мс,
   `cancel_after_false_observation_before_atomic_working_wins_without_callback`,
   `cancel_after_working_before_receipt_begun_waits_and_is_post_begun`, process
   exit без handler join,
   `bound_actor_lease_is_retained_until_terminal_or_process_fail_stop` и normal
   terminal idle cleanup.
9. RED interface tests: `V5PendingDirectReceipt` ACK boundary для
   Completed/Failed/Cancelled, drop/projection-error-without-ACK,
   `receipt_backed_task_result_is_repeatable_and_direct_ack_is_rejected`,
   `task_bound_false_masks_working_as_queued_until_receipt_begun`, byte-equivalent
   restart recovery, native+compatibility cancel во всех closed states и
   неизменные V12/8/11 `tools/list`/schemas.
10. GREEN implementation по тем же slices; затем `cargo fmt`, clippy,
   `cargo test -p unica-coder`, arch/design/registry tests и platform CI.

Детерминированный acceptance с manual clock сначала сохраняет 4096 independent
Task links на всём протяжении 28800 ACK за 900 секунд target traffic с 32
retained terminals и не более 32 одновременно cycling Direct; отдельная фаза
заполняет ровно 64 live records (включая `CancelReserved`, promised, handoff и
receipt-backed terminal) и отклоняет 65-й, затем проверяет точные
ACK/bind/7125ms/15min/1h expiry, rotation и reopen boundaries. Boundary 4097
отдельно проверяет pre-/post-Begun Link Capacity branches, отсутствие
eviction/repeated create и сохранение listener; отдельная fault phase проверяет
post-reservation `TaskStoreCapacityInvariantViolation`. Inspect-only reopen доказывает, что classification ledger
evidence предшествует любой Task terminalization.
Tombstone high-water и expected count в каждом checkpoint вычисляются из raw
first-ACK epochs; test не требует, чтобы все 28 800 traffic tombstones
существовали одновременно ровно в `t=900s`. Отдельный overflow setup заполняет
bounded tombstone pool реальными unexpired records и проверяет typed capacity с
byte-equivalent retained `DirectTerminalUnacked`.
Отдельный case удерживает 32 receipt-backed terminal payloads и
проводит 32 calls/s Direct с немедленным ACK без capacity rejection; 65-й
одновременно live receipt обязан закрыто отклониться до `Begun`.
Отдельный wall-clock gate на Windows, macOS и Linux в течение 60 секунд проводит
не менее 1920 полных reserve → small direct terminal → ACK lifecycle, допускает
ноль capacity/store errors, требует p99 не более 250 мс и полного опустошения
writer queue не позже двух секунд после нагрузки. Результаты публикуются как
cutover evidence; platform gate не заменяется modelled-clock тестом.

## Архитектурная активация

Эта planned запись не делает контракт действующим и поэтому имеет
`establishes: []`. Реализация одним атомарным change set создаёт новую датированную
active successor decision, которая supersedes
`DEC.2026-08-28.DAEMON-RECEIPT-LEDGER`, переводит
`CTR.WIRE.DAEMON-INVOCATION-PROTOCOL` на v5 и создаёт новые derived receipt
reservation/recovery/capacity invariants с точными именами тестов. Мerged
planned запись не дополняется задним числом derived symbols.

В том же change set пересматриваются существующие:

- `INV.APP.DAEMON-INVOCATION-OWNERSHIP` — at-most-once attempt и bounded horizon,
  не exactly-once outcome;
- `INV.APP.DAEMON-INVOCATION-HANDOFF` — durable reserve и write-ahead link;
- `INV.APP.DAEMON-ACTOR-AUTHORITY` — actor-derived binding до `Begun` и
  TaskStore;
- `INV.APP.DAEMON-TASK-PERSISTENCE` — Task payload отдельно от receipt evidence;
- `INV.APP.DAEMON-TASK-RECOVERY` — begun без terminal становится uncertain;
- `INV.APP.DAEMON-TERMINAL-RECONCILIATION` — оба направления cross-store
  readback без domain replay;
- `INV.APP.DAEMON-STORE-FAIL-STOP` — два bounded sole-writer actors и отдельные
  capacity pools.

Поле `changes` decision перечисляет только
`CTR.WIRE.DAEMON-INVOCATION-PROTOCOL`, потому что схема `arch/README.md`
разрешает в нём только существующие contracts. Затрагиваемые invariants названы
выше и будут перевязаны на active successor вместе с тестами; добавлять их в
`changes` означало бы сделать реестр невалидным.

## Сохранение публичной поверхности

- `SurfaceRelease::V12` и текущий публичный package-selected stdio остаются без
  изменения;
- hidden/native profile публикует ровно 8 operations;
- compatibility profile публикует те же 8 плюс `task.get/result/cancel`, итого
  11;
- protocol recovery/ACK доступны только authenticated local daemon frontend;
- `unica.receipt.*`, generic idempotency key, `task.resume`, `task.logs` и второй
  MCP server не создаются;
- G6 cutover и удаление 74 legacy имен остаются отдельным gate.

## Отвергнутые варианты

- **Расширить TaskStore unbound состояниями.** Требует ложной workspace identity.
  Выбранный receipt-backed promised Task появляется только в handoff cutoff и
  после `ActorBound` пытается exact-bind в TaskStore либо остаётся receipt-owned
  при proven Link Capacity; Direct до cutoff ложным Task не становится.
- **Хранить только in-memory map.** Не закрывает restart и ACK-loss window.
- **На retry всегда materialize новый Task.** Не восстанавливает потерянный
  direct result и допускает новый attempt до materialization.
- **Назвать два rename одной атомарной транзакцией.** Crash между stores остаётся;
  его закрывает только intent + exact reconciliation.
- **Повторить begun вызов после crash.** Может удвоить внешний side effect.
- **Считать begun вызов successful после crash.** Может выдумать side effect.
- **Один pool из 4096 records на час.** Ограничивает систему 1.14 calls/s.
- **Вечные tombstones.** Невозможно совместить с finite local storage; договор
  обязан иметь явный horizon.
- **ACK после чтения bytes в client.** Теряет result при projection failure;
  explicit pending handle сдвигает ACK к последней доступной безопасной границе.
