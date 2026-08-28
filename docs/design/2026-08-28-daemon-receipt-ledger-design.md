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
| ReceiptLedger | exact receipt key, pre-admission request identity, durable actor binding, original lifecycle и cancel authority до commit `TaskBound`, Direct ACK, deduplication, promised/handoff Task projection, canonical receipt-backed terminal payload, handoff intent, compact terminal evidence | progress, cancellation после `TaskBound` или копией task result после commit `TaskBound` |
| TaskStore (`InvocationStore` в текущем коде) | provisional exact actor-bound record во время handoff; после commit `TaskBound` — sole-owned `Queued`/`Working`/terminal Task, timestamps, TTL, canonical result, closed failure reason и durable `cancelRequested` | unbound promised Task, pre-Task retry, direct ACK, pre-receipt cancellation |
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
  - exact 64-hex `CoreIdentity` текущих ABI и private protocol;
  - закрытый `ToolIdentity` одной из восьми canonical operations;
  - `NormalizedArgumentsHash` canonical JSON arguments;
  - `SafeIdentityHash` request scope, вычисленный из `workspaceHint`.

Response budget, monotonic deadline, cancel timing и transport retry count не
входят в identity: повтор не может получить новый lifecycle изменением budget.
Client и server используют одну canonicalization функцию; wire test сравнивает
их byte-for-byte `ReceiptKey` и отдельно доказывает mismatch каждого поля.

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

Request-scope hash не является workspace execution identity. После успешного
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
| `DirectTerminalUnacked` | exact canonical terminal result либо closed `interrupted_before_execution`/`cancelled`/`outcome_uncertain`, terminal epoch, result digest | `AcknowledgedTombstone` либо physical deletion через один час с освобождением live count/result quota |
| `AcknowledgedTombstone` | key, terminal/result digest, acknowledged epoch; без result bytes | expiry после idempotency horizon |
| `TaskPromisedUnbound` | key, stable Task timestamps/TTL/poll interval, reserved result quota, `cancelRequested`; queued projection без workspace identity/result | `TaskPromisedActorBound`, `TaskTerminalReceiptBacked` |
| `TaskPromisedActorBound` | promised Task, actor-derived workspace identity, exact TaskStore-bind intent, reserved result quota, `cancelRequested`; queued projection, `begun=false` | `TaskBound`, `TaskTerminalReceiptBacked` |
| `TaskHandoffActorBound` | stable Task projection, actor-derived workspace identity, exact write-ahead handoff intent, `begun`, `cancelRequested`, reserved result quota и optional staged terminal payload | `TaskBound`/`TaskTerminalBound` после exact TaskStore readback; при proven Capacity staged terminal выигрывает, без него до `Begun` — `TaskTerminalReceiptBacked(task_capacity)`, после `Begun` — `TaskReceiptOwnedActorBound` |
| `TaskReceiptOwnedActorBound` | stable Working Task projection, actor-derived identity, `begun=true`, `cancelRequested`, reserved result quota, latched proven TaskStore Capacity; TaskStore create больше не повторяется | `TaskTerminalReceiptBacked` с actual outcome либо `outcome_uncertain` после crash |
| `TaskTerminalReceiptBacked` | receipt-owned Task до commit `TaskBound`, exact canonical terminal result либо closed failure/cancelled, terminal epoch/digest, optional bound identity | expiry после полного Task TTL; repeated read без ACK |
| `TaskBound` | key, actor-derived workspace identity, exact Task record identity/digest, bind epoch, `begun`; TaskStore владеет durable cancel flag; при `begun=false` resolver нормализует projection как queued даже если TaskStore уже Working | после exact Working readback durable `begun=false→true`, затем `TaskTerminalBound` |
| `TaskTerminalBound` | key, Task identity, closed terminal status, task outcome digest, terminal epoch; без копии result | expiry совместно с terminal Task retention |

`TaskTerminalReceiptBacked.payload` — закрытый union
`Completed { DomainResult }` / `Failed { SafeFailureReason }` / `Cancelled` с
тем же status/result/failure shape, timestamps и size checks, что TaskStore.
Digest-only terminal до TaskStore запрещён: validation `DomainResult` остаётся
byte-equivalent, а admission/cancel failure сохраняет exact closed reason,
не свободный текст.

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

`ExecutionClass::KnownLong` не является pre-admission классификацией: в текущем
контракте его возвращает `service.prepare`, то есть receipt уже `ActorBound` и
`Begun`. Поэтому known-long и cutoff во время prepare/execute сначала durable
пишут `TaskHandoffActorBound`, затем пытаются выполнить exact TaskStore
create/readback и commit `TaskBound`; доказанная TaskStore Capacity оставляет
begun attempt receipt-owned. Ни одна из ветвей не переходит назад в
`TaskPromisedUnbound`: это состояние разрешено только когда original cutoff
наступил внутри ещё `Unbound` validation/admission.

Validation/admission rejection до promised handoff публикуется как обычный
`DirectTerminalUnacked` failure; после promise та же ошибка terminalizes
`TaskTerminalReceiptBacked` с тем же bounded canonical result. Если процесс
погиб с `begun=true` и без
доказанного terminal:

- receipt без promised/Task становится
  `DirectTerminalUnacked(outcome_uncertain)`;
- `TaskPromisedUnbound` не может иметь `Begun`: restart/cancel terminalizes его
  `TaskTerminalReceiptBacked` как interrupted-before-execution/cancelled без
  callback;
- `TaskBound` получает в TaskStore terminal failed с закрытой причиной
  `OutcomeUncertain`, затем ledger становится `TaskTerminalBound`;
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

К существующим messages добавляются:

- `RecoverInvocationReceipt { receiptKey }`;
- `AcknowledgeInvocationReceipt { receiptKey, terminalDigest }`.

`CancelInvocation` получает полный `receiptKey`, а не только пару UUID.
Responses добавляют `InvocationRecovered` и idempotent
`InvocationAcknowledged`. Closed protocol errors добавляют:
`receipt_not_found`, `receipt_expired`, `receipt_capacity`,
`invocation_identity_mismatch`, `outcome_uncertain`.

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
`Reserved::Begun` — `DirectTerminalUnacked(outcome_uncertain)`. Только уже
committed promised/handoff state разрешает receipt-backed Task или exact
TaskStore reconciliation. Ни один recovery путь не вызывает domain callback и
не изобретает Task задним числом.

## Direct ACK и граница доставки

Daemon сначала durable публикует `DirectTerminalUnacked`, затем отправляет
result. Client возвращает интерфейсному слою не голый result, а
`PendingDirectReceipt { result, receiptKey, terminalDigest }`. ACK отправляется
только после успешной проверки размера, parse и построения окончательного
`CallToolResult`. Drop/error до этой точки ACK не посылает.

ACK commit переводит record в компактный tombstone; повторный ACK после потери
ACK response читает тот же tombstone и успешен. Если ACK не committed,
recovery возвращает исходный direct result. Terminal digest запрещает ACK
другого результата при той же identity.

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

## Cutoff и связь с TaskStore

Семисекундный handoff не ослабляется ожиданием actor identity. Если workspace
admission либо semantic validation всё ещё barrier-blocked, deadline owner к
original cutoff durable переводит только `Reserved::Unbound` в
`TaskPromisedUnbound` и в пределах прежних 125 мс возвращает queued Task с exact
`reservedTaskId`. `task.get` и `task.cancel` читают это состояние прямо из
ReceiptLedger. Это не TaskStore record и не подставляет request-scope hash
вместо actor identity.

Вся pre-`Begun` unbound pipeline — semantic validation вместе с actor
admission/binding — получает один fixed cleanup grace две секунды после promise,
а не новое окно на каждый этап. Worker проверяет durable terminal winner после
validation, перед admission, после admission, перед actor bind и перед
`Begun`/любым следующим callback. Rejection, успевший выиграть race, пишет
`TaskTerminalReceiptBacked`; actor capability, успевшая выиграть race, сначала
пишет `TaskPromisedActorBound`. Проигравший late return только освобождает lease
и не продолжает pipeline.

Если unbound worker не вышел и не достиг durable actor-bound handoff за две
секунды, executor пишет restart intent, закрывает listener и daemon main
завершает PID без join этого worker. Successor пишет
`TaskTerminalReceiptBacked(InterruptedBeforeExecution)` и никогда не вызывает
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
2. Пока PID жив, coordinator удерживает тот же `BoundStartCancelGate` на всём
   переносе cancel-authority. Под ним TaskStore выполняет idempotent
   create-if-absent `Queued` для `begun=false` либо `Working` для `begun=true` и
   переносит monotonic `cancelRequested` из intent. ReceiptLedger публикует
   `TaskBound` только после exact readback, доказавшего ту же identity и cancel
   flag; с этого commit единственным cancel-authority становится TaskStore.
   Recovery выполняет эту последовательность single-threaded до listener.
   Exact уже существующая запись считается тем же commit;
   identity/state/cancel regression является corruption/fail-stop.
3. Если в `TaskHandoffActorBound` уже staged terminal outcome, coordinator
   terminalizes созданный Task тем же bounded outcome и exact-readback-ит его.
   Иначе ReceiptLedger fsync-переходом публикует `TaskBound` с digest Task
   record; terminal readback публикует `TaskTerminalBound`.
4. Task resolver атомарно меняет источник projection с receipt-backed promise
   или handoff intent на exact TaskStore record. Старый queued snapshot и новый
   snapshot имеют те же TaskId, invocationId, createdAt, TTL и poll interval;
   timestamps не регрессируют.
5. Для `begun=false` после шага 4 coordinator захватывает per-invocation
   `BoundStartCancelGate` и удерживает его до receipt `begun`. Под gate он
   reauthorize-ит matching live actor/resource proof, затем TaskStore sole-writer
   атомарно выполняет `start_working_if_not_cancel_requested`: без отдельного
   false-read переводит exact versioned `Queued` в `Working` либо возвращает
   durable cancel/terminal winner. Только для Working readback ReceiptLedger
   повторно проверяет proof/authorization и durable пишет `begun=true`; до этого
   resolver нормализует Task как queued. Затем gate освобождается и вызывается
   `service.prepare`. Cancel использует тот же gate, поэтому он не может durable
   записаться между Working readback и receipt `begun`: до start он запрещает
   callback, после `begun` является post-Begun cancellation. Live executor
   удерживает proof/actor/resource lease до terminal cleanup либо process
   fail-stop. Уже begun short/blocked prepare на cutoff продолжает единственный
   attempt и получает тот же Task. `KnownLong` никогда не создаёт
   `TaskPromisedUnbound`.

Typed TaskStore `Capacity` является доказанным отказом create, а не
`CommitUncertain`. Для `TaskPromisedActorBound` или
`TaskHandoffActorBound { begun:false }` coordinator без callback атомарно пишет
`TaskTerminalReceiptBacked(task_capacity)`. Уже staged terminal или committed
pre-Begun cancel имеет приоритет и становится terminal winner вместо
`task_capacity`. Для begun handoff без staged terminal coordinator durable latch-ит
`TaskReceiptOwnedActorBound`, не повторяет create, не вытесняет чужой Task и не
останавливает listener: единственный live attempt продолжает работу, а его
actual terminal сохраняется в заранее зарезервированный
`TaskTerminalReceiptBacked`. Crash до terminal даёт receipt-backed
`outcome_uncertain`. Неопределённость самого create никогда не маскируется как
Capacity: она сохраняет intent и ведёт к fail-stop/exact reconciliation.

Crash recovery действует по durable state:

- `TaskPromisedUnbound` без actor binding terminalizes receipt-backed Task как
  interrupted-before-execution; callback не вызывается;
- `TaskPromisedActorBound`/TaskStore-bind intent без Task пытается создать exact
  actor-bound queued Task с тем же `cancelRequested`, затем terminalizes
  cancelled при установленном flag либо interrupted-before-execution, потому
  что это состояние всегда `begun=false`; proven Capacity вместо повторного
  create даёт receipt-backed cancel winner либо `task_capacity`;
- `TaskHandoffActorBound` без Task пытается создать exact actor-bound
  Queued/Working по `begun` и переносит `cancelRequested`; при `begun=false`
  terminalizes cancelled/interrupted-before-execution, а при `begun=true` —
  только `outcome_uncertain`, даже если cancel был committed до Task
  create/token. Если recovery получает proven Capacity, он без повторного create
  сразу пишет receipt-backed cancel/`task_capacity` до `begun`, а после `begun`
  — staged terminal winner либо `outcome_uncertain`; listener Capacity не
  блокирует. Callback не вызывается;
- `TaskReceiptOwnedActorBound` после restart terminalizes
  `TaskTerminalReceiptBacked(outcome_uncertain)` без TaskStore create или
  callback;
- exact Task без `TaskBound`/`TaskTerminalBound` проверяется по actor-derived
  identity, link дописывается и terminalizes по `begun`; предметный callback не
  вызывается;
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
authority. Receipt-owned capacity branch продолжает хранить flag в ledger.
После `TaskBound` аналогичный flag устанавливает только TaskStore. Crash begun
handoff между receipt flag, Task create и token terminalizes
`outcome_uncertain`, а не `cancelled`, и callback не replay-ится.

После `Begun` cancellation token получает ровно две секунды
`NONCOOPERATIVE_CANCEL_GRACE`. Если `prepare`/`execute` не освобождает live
lease, `InvocationExecutor` помечает `RestartRequested`; server закрывает
listener и возвращается из daemon main без join заблокированного handler.
Именно смерть PID, а не очистка map, освобождает actor/provider resources.
Successor переводит begun receipt/Task в `outcome_uncertain`:
`SafeFailureReason::OutcomeUncertain` внутри Rust, `cancelled`
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
- `CancelReserved` занимает один общий count slot и не более 1 KiB actual
  metadata, но не result reservation; exact submit атомарно меняет его на
  `Reserved` только если может зарезервировать полный
  `MAX_DAEMON_RESPONSE_LINE_BYTES` в том же byte cap;
- каждый принятый submit держит worst-case result reservation сквозь
  `Reserved`, оба promised/handoff состояния и до одного из четырёх доказанных
  событий: exact TaskStore bind/readback, Direct ACK, expiry с physical deletion
  `DirectTerminalUnacked` либо expiry `TaskTerminalReceiptBacked`; поэтому
  rejection/cancel после promise всегда имеет заранее выделенное место для
  canonical terminal Task payload;
- при terminal receipt-backed failure/cancel reservation может уменьшиться до
  exact encoded bytes, но count slot остаётся до expiry; completed result bytes
  остаются полностью учтены;
- active/nonterminal records не вытесняются; отказ capacity самого
  ReceiptLedger происходит до `begun`, а доказанная TaskStore Capacity после
  `begun` использует уже зарезервированную receipt-owned ветвь;
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

Только `TaskBound` и `TaskTerminalBound` не занимают receipt result quota:
result уже находится в TaskStore. `TaskPromisedActorBound` и
`TaskHandoffActorBound` остаются в live payload pool до exact TaskStore
create/readback и commit `TaskBound`;
`TaskReceiptOwnedActorBound` остаётся там до receipt-backed terminal.
Link pool повторяет current TaskStore count limit — 4096 exact records — и
имеет отдельный byte cap 4 MiB при maximum encoded link 1 KiB. Retention
заканчивается не раньше соответствующего terminal Task, поэтому большое число
long-running Tasks не вытесняет Direct receipts и наоборот.

На boundary 4097 proven Capacity не вытесняет существующий Task и не занимает
link slot: pre-Begun handoff закрывается receipt-backed `task_capacity`, begun
handoff latch-ится receipt-owned до actual/uncertain terminal. Это независимый
backpressure path; только create `CommitUncertain`, а не Capacity, требует
fail-stop/reconciliation до listener.

### Compact acknowledged-tombstone pool

Product load target — 32 acknowledged Direct calls/s. Private local ACK закрывает
транспортное окно быстро, поэтому post-ACK deduplication horizon равен 15 минут,
а не часу. Count cap выводится явно:

`32 * 900 + 64 = 28864` tombstones.

Последние 64 records — две секунды headroom при target rate. Encoded tombstone
ограничен 512 bytes, общий byte cap равен
`28864 * 512 = 14778368` bytes. ACKed tombstones никогда не занимают
active/unacked count, reserved result bytes или Task-link capacity. Повторное
чтение/ACK не продлевает 15-minute horizon; удаляются только expired records.

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
tombstones/14778368 bytes; reopen сохраняет original cancel/terminal expiry.
Только один оборванный tail frame принимается как pre-commit. Corruption
committed frame fail-closed.

Protocol-v5 startup запрещает текущий legacy eager-open, который сам
terminalizes все TaskStore `Queued`/`Working` до чтения receipt evidence.
TaskStore сначала открывается через
`FileInvocationStore::open_inspect_only(...) -> (Self,
TaskStoreRecoveryCatalog)`: constructor проверяет
ownership/schema/checksum/capacity и возвращает immutable catalog, но не меняет
active records. Затем single-threaded `ReceiptRecoveryCoordinator::reconcile`
сопоставляет exact identities и `begun`/`cancelRequested`/handoff evidence и
вызывает только typed `terminalize_recovered_exact`. Только возвращённый
`RecoveryComplete` разрешает публикацию listener. Working/Queued v5 Task без
exact receipt/handoff evidence является corruption/fail-stop; legacy v3/v4
state отделён CoreIdentity и не мигрируется этим open. Обычный legacy
`FileInvocationStore::open`, который eager-terminalize-ит active records, для
protocol-v5 state вызывать запрещено.

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
| Crash после strict parse, до reserve ACK | exact reserve находится либо submit получает закрытый store failure | execution без durable reserve |
| Crash в `Reserved::Unbound/ActorBound`, до committed promise/handoff | `DirectTerminalUnacked(cancelled | interrupted_before_execution)` по durable flag; Task не создаётся | receipt-backed Task задним числом либо domain callback после restart |
| Crash после committed promise/handoff, до `Begun` | receipt-backed Task либо exact TaskStore terminal cancelled/interrupted-before-execution | откат в Direct или domain callback после restart |
| Promise на cutoff, затем validation/admission rejection | exact `TaskTerminalReceiptBacked` переживает restart; TaskStore пуст; repeated `task.result` byte-equivalent до TTL и payload учтён в live quota | digest-only Task terminal, потерянный result или TaskStore до ActorBound |
| Non-cooperative validation/admission после unbound promise | один общий grace 2 с, terminal-winner checks и смерть PID; successor terminalizes interrupted-before-execution | поздний bind/callback после terminal winner или вечный owner lease |
| Crash в `Reserved::Begun` без committed handoff, до provider return | `DirectTerminalUnacked(outcome_uncertain)` | receipt-backed Task задним числом, success/failure на догадке или replay |
| Side effect committed, terminal receipt не записан | `outcome_uncertain`, предметная диагностика | exactly-once claim |
| Direct terminal записан, response потерян | recover возвращает byte-equivalent result до ACK | новый execution |
| Unacked Direct достигает terminal+1h | committed physical deletion освобождает payload/count/result quota; recover после horizon не обещан | вечная reservation либо eviction до horizon |
| Submit session закрыта до cutoff, callback завершается вовремя | lifecycle остаётся Direct; `DirectTerminalUnacked` доступен через recover | немедленный Task handoff, cancel или новый deadline |
| Submit session закрыта, затем наступает original cutoff | обычный phase-aware promise/handoff ровно на исходном cutoff | handoff по disconnect либо replenished budget |
| ACK request/response потерян | unacked result либо idempotent acknowledged tombstone | result mismatch или replay |
| Positive-budget повтор | исходный cutoff/state | новый семисекундный lifecycle |
| `KnownLong` после prepare | `TaskHandoffActorBound { begun:true }` → exact Working Task либо receipt-owned begun branch при proven Capacity | переход назад в `TaskPromisedUnbound`, повтор create после latched Capacity или потеря actor identity |
| Cutoff во время non-cooperative prepare | exact Task либо receipt-owned begun branch через `TaskHandoffActorBound`; после crash `outcome_uncertain` | orphan Task, unbound promise или второй prepare |
| Cutoff во время barrier-blocked actor admission | exact queued `TaskPromisedUnbound` в 7 с + 125 мс | direct timeout или TaskStore с request hash |
| Actor identity mismatch при TaskStore bind | fail-stop без Task mutation | last-writer-wins binding |
| Missing/foreign/stale live actor proof до bound Task start | closed authority rejection; TaskStore/receipt неизменны, callback не вызван | считать durable Task identity live capability |
| Actor proof stale после Working readback, до receipt `begun` | fail-stop; recovery terminalizes interrupted-before-execution без callback | `begun`, prepare или потеря retained lease |
| Crash между Task create и receipt link | exact identity reconciliation | второй Task или execution |
| Inspect-only open видит v5 `Queued`/`Working` | record и version остаются byte-equivalent до ReceiptLedger-led classification; одна и та же Working запись становится interrupted/cancelled при `begun=false` и `outcome_uncertain` при `begun=true` | eager terminalization до чтения receipt evidence |
| Active v5 Task не имеет exact receipt/link evidence | corruption/fail-stop до Task mutation и listener publication | автоматическая orphan terminalization либо доступный listener |
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
| Task terminal commit, receipt terminal не записан | readback Task и дописанный `TaskTerminalBound` | повтор domain execution |
| Repeated get/result receipt-backed Task | тот же canonical snapshot до terminal+1h; Direct ACK отклонён | first-read deletion, TTL renewal или потеря payload после restart |
| Result > 8 MiB | bounded closed `result_too_large` terminal | unbounded serialization |
| 64 live receipt boundary | 65-й reserve/cancel получает `receipt_capacity` до `Begun`; exact expiry/ACK/bind освобождает quota | side effect без worst-case result reservation |
| TaskStore boundary 4097 до `Begun` | existing 4096 Tasks неизменны; receipt-backed `task_capacity`, callback отсутствует, listener доступен | eviction, повтор create/link либо fail-stop на proven Capacity |
| TaskStore boundary 4097 после `Begun` | `TaskReceiptOwnedActorBound`; live attempt даёт actual receipt-backed terminal, crash — `outcome_uncertain`; staged terminal остаётся winner | eviction, повтор create/link, потеря result либо `task_capacity` поверх staged outcome |
| Tombstone pool под target load | 15 минут post-ACK evidence независимо от active quota | starvation active quota |
| Store commit невозможно доказать | listener закрыт, `RestartRequested` | staged response или fallback executor |

## Предполагаемые Rust files и interfaces

### Новые files

- `crates/unica-coder/src/application/receipt_ledger.rs` —
  `ReceiptKey`, `RequestIdentity`, `ReceiptState`, `ReceiptTerminalOutcome`,
  `BoundTaskStartAuthorization`, `ReceiptLedger` port и typed errors;
- `crates/unica-coder/src/application/receipt_ledger_actor.rs` — bounded
  sole-writer actor и absolute-deadline operations;
- `crates/unica-coder/src/infrastructure/receipt_ledger.rs` — owner-only file
  implementation, active files, tombstone segments, recovery/compaction и
  count+byte catalogs.

Минимальный application port:

```text
reserve(key, original_cutoff, deadline) -> ReservedReceipt
request_cancel_or_reserve(key, fixed_expires_at, deadline) -> CancelResolution
promise_unbound_task(key, deadline) -> PromisedTaskReceipt
bind_actor(key, bound_workspace_identity, deadline) -> ActorBoundReceipt
bind_promised_actor(key, bound_workspace_identity, deadline) -> PromisedActorBoundReceipt
mark_reserved_begun(key, start_cancel_guard, bound_actor_proof, deadline) -> BegunOrCancelWinner
authorize_bound_task_start(key, start_cancel_guard, bound_actor_proof, deadline) -> BoundTaskStartAuthorization
mark_bound_task_begun(key, start_cancel_guard, authorization, versioned_working_readback, deadline) -> BegunTaskReceipt
begin_bound_task_handoff(key, task_identity, deadline) -> BoundHandoffIntent
stage_bound_handoff_terminal(key, outcome, deadline) -> BoundHandoffIntent
latch_task_store_capacity(key, start_cancel_guard, proven_capacity, deadline) -> ReceiptOwnedTaskOrTerminal
bind_task(key, start_cancel_guard, exact_task_record, deadline) -> TaskBoundReceipt
publish_direct_terminal(key, outcome, deadline) -> PendingDirectReceipt
publish_receipt_backed_task_terminal(key, outcome, deadline) -> ReceiptBackedTaskTerminal
publish_bound_task_terminal(key, exact_task_record, deadline) -> TaskTerminalBoundReceipt
acknowledge_direct(key, terminal_digest, deadline) -> AcknowledgedReceipt
recover(key, deadline) -> ReceiptState
resolve_task(task_id, deadline) -> ReceiptBackedOrStoredTaskSnapshot
```

Каждый mutating method обязан возвращать exact committed record либо typed
`CommitUncertain`; caller reconcile-ит только store transition и не получает
domain callback. `mark_reserved_begun` принимает только
`Reserved::ActorBound`, matching live proof, held start/cancel guard и атомарно
проверяет `cancelRequested=false`. Для Task path executor до любой TaskStore
mutation получает unforgeable `BoundTaskStartAuthorization` только из matching
non-stale proof под тем же guard; `mark_bound_task_begun` принимает только
`TaskBound { begun:false }`, guard, authorization и exact versioned Working
readback. Missing/foreign/stale proof закрыто отклоняется без `begun`, TaskStore
mutation или callback; proof, устаревший после Working readback, не допускает
`begun`, вызывает fail-stop, а recovery закрывает Task как
interrupted-before-execution.

TaskStore port получает отдельную sole-writer операцию
`start_working_if_not_cancel_requested(task_identity, expected_version,
deadline) -> StartedVersionedReadback | CancelOrTerminalWinner`. Она одной
транзакцией перечитывает current version/flag/status и либо пишет Working, либо
возвращает winner; stale observation/CAS не может начать Task.

Startup/recovery seam закрыт следующими typed interfaces:

```text
FileInvocationStore::open_inspect_only(root, clock, deadline)
    -> (FileInvocationStore, TaskStoreRecoveryCatalog)
terminalize_recovered_exact(task_identity, expected_version,
    RecoveryTerminalReason::{Cancelled, InterruptedBeforeExecution, OutcomeUncertain},
    deadline) -> StoredInvocationRecord
ReceiptRecoveryCoordinator::reconcile(receipt_catalog, task_catalog, deadline)
    -> RecoveryComplete
```

`TaskStoreRecoveryCatalog` содержит exact identity, version, status и durable
cancel flag; получение catalog не terminalize-ит `Queued`/`Working`.
`begin_bound_task_handoff` — только `Reserved::ActorBound/Begun`.
`publish_receipt_backed_task_terminal` принимает все receipt-owned источники до
`TaskBound`: оба promised states, pre-Begun `TaskHandoffActorBound` при proven
Capacity и `TaskReceiptOwnedActorBound`; already staged/terminal outcome всегда
остаётся winner. А
`publish_bound_task_terminal` требует exact TaskStore readback. Эти typed
preconditions не дают KnownLong потерять actor identity или result ownership.
`bind_task` дополнительно требует held guard и TaskStore readback с monotonic
`cancelRequested`, не меньшим receipt flag; bind atomically прекращает
cancel-authority ReceiptLedger. Cancel, выигравший guard до TaskStore bind и
commit `TaskBound`, переносится в TaskStore; cancel после commit `TaskBound`
пишет уже только TaskStore.
`latch_task_store_capacity` требует тот же continuously held
`BoundStartCancelGate` от TaskStore create attempt до receipt commit и принимает
только typed proven Capacity. Он атомарно перечитывает receipt: staged terminal
либо committed pre-Begun cancel остаётся winner; иначе pre-Begun state
terminalizes `task_capacity`, а begun state становится
`TaskReceiptOwnedActorBound` с сохранённым cancel flag. `CommitUncertain` этим
методом принять нельзя.

### Изменяемые files реализации

- `crates/unica-coder/src/application/mod.rs` — зарегистрировать два новых
  application modules;
- `crates/unica-coder/src/application/invocation.rs` — заменить in-memory
  receipt/pending-cancel authority на ReceiptLedger, оставив live map только
  для tokens/leases/wakeup; durable `ActorBound` и `Begun` предшествуют
  `prepare`, task resolver объединяет promised/receipt-backed terminal и
  TaskStore без replay; current `ExecutionClass::KnownLong` после prepare
  маршрутизируется только через `begin_bound_task_handoff`;
- `crates/unica-coder/src/application/invocation_store.rs` — добавить закрытую
  `OutcomeUncertain`, exact task-link/start-working readback и startup-only
  terminalization `Queued`/`Working` без callback, а также durable monotonic
  `request_cancel_exact` и inspect-only recovery catalog, не превращая TaskStore
  в ReceiptLedger;
- `crates/unica-coder/src/infrastructure/mod.rs` — зарегистрировать file ledger;
- `crates/unica-coder/src/infrastructure/task_store.rs` — idempotent exact
  create/readback с `cancelRequested`, atomic
  `start_working_if_not_cancel_requested`, post-`TaskBound` cancel authority и
  `open_inspect_only` без eager terminalization, coordinated terminal retention,
  без receipt/ACK state;
- `crates/unica-coder/src/infrastructure/daemon/protocol.rs` — protocol v5,
  `ReceiptKey`, recover/ACK messages/responses и closed codes;
- `crates/unica-coder/src/infrastructure/daemon/identity.rs` — CoreIdentity/state
  fork tests для protocol v5;
- `crates/unica-coder/src/infrastructure/daemon/server.rs` — открыть sibling
  stores, reserve сразу после strict parse, выполнить handoff coordinator и
  ReceiptLedger-led startup recovery над inspect-only Task catalog до listener
  publication; записывать actor safe identity до TaskStore/prepare, drain accept
  batch и поддержать 32 concurrent handshakes;
- `crates/unica-coder/src/infrastructure/daemon/client.rs` — exact recovery,
  explicit Direct ACK handle, ACK-loss retry, receipt-backed Task lookup и
  запрет budget renewal;
- `crates/unica-coder/src/infrastructure/daemon/mod.rs` — process/crash protocol
  fixtures и aggregate invariant tests;
- `crates/unica-coder/src/interfaces/mcp.rs` — ACK после успешной конечной
  Direct projection, route `tasks/get/cancel` и compatibility
  `task.get/result/cancel` через `resolve_task`, без изменения `tools/list` или
  public schemas.

## Test-first implementation slices

1. RED protocol/identity tests:
   `v5_rejects_v3_v4_and_strictly_round_trips_receipt_messages`,
   `receipt_key_is_canonicalized_identically_by_client_and_server` и
   `response_budget_is_not_receipt_identity`; отдельно mismatch каждого exact
   поля, unknown fields и CoreIdentity/state-dir fork.
2. RED in-memory port contract tests:
   `known_long_requires_begun_bound_handoff_intent`,
   `unbound_promise_terminal_keeps_canonical_payload_until_task_ttl`, все
   остальные state transitions, terminal winner, positive-budget replay,
   Task-ACK rejection и Direct ACK loss.
3. RED file-store tests:
   `receipt_backed_task_terminal_survives_reopen_byte_equivalent`,
   `cancel_reserved_reopens_with_original_7125ms_expiry`,
   `task_store_inspect_only_open_preserves_queued_and_working_until_receipt_reconciliation`,
   `receipt_led_startup_distinguishes_working_begun_false_from_begun_true`,
   `v5_active_task_without_exact_receipt_link_fail_stops_before_listener`, crash checkpoints
   каждой atomic replace/append/fsync, tail frame, corruption, dual evidence и
   compaction generation.
4. RED capacity tests:
   `cancel_reserved_shares_live_64_count_without_result_reservation`,
   `promised_and_handoff_states_hold_worst_case_result_quota`,
   `task_bind_direct_ack_and_receipt_terminal_expiry_release_exact_quota`,
   `direct_unacked_expiry_deletes_payload_and_releases_exact_quota`,
   `task_store_capacity_before_begun_terminalizes_receipt_backed_without_callback`,
   `task_store_capacity_after_begun_latches_receipt_owner_and_never_retries_create`,
   `task_store_capacity_preserves_staged_terminal_winner`,
   `receipt_owned_begun_crash_terminalizes_outcome_uncertain_without_task_store`,
   `task_store_4097_boundary_preserves_existing_tasks_and_listener_availability`, exact
   64/4096/28864 count и 541065216/4194304/14778368 byte boundaries, segmented
   rotation и expired-only eviction.
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
9. RED interface tests: `PendingDirectReceipt` ACK boundary, drop-without-ACK,
   `receipt_backed_task_result_is_repeatable_and_direct_ack_is_rejected`,
   `task_bound_false_masks_working_as_queued_until_receipt_begun`, byte-equivalent
   restart recovery и неизменные V12/8/11 `tools/list`/schemas.
10. GREEN implementation по тем же slices; затем `cargo fmt`, clippy,
   `cargo test -p unica-coder`, arch/design/registry tests и platform CI.

Детерминированный acceptance с manual clock проводит 28800 ACK за 900 секунд
target traffic, одновременно проверяет boundary 64 live records (включая
`CancelReserved`, promised, handoff и receipt-backed terminal) и 4096 Task
links, затем точные ACK/bind/7125ms/15min/1h expiry, rotation и reopen
boundaries. Boundary 4097 отдельно проверяет обе Capacity ветви, отсутствие
eviction/repeated create и сохранение listener; inspect-only reopen доказывает,
что classification ledger evidence предшествует любой Task terminalization.
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
  при proven Capacity; Direct до cutoff ложным Task не становится.
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
