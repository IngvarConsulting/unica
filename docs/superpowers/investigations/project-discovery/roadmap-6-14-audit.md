# Read-only audit: Tasks 6-14 Project Discovery And Discovery Receipts

Дата аудита: 2026-07-17  
Worktree: `/private/tmp/unica-project-discovery-receipts`  
Область: активная спецификация, исторический implementation plan, текущий Rust/Python-код, package metadata и release workflow. Tracked-файлы не изменялись.

## Итоговая оценка

Tasks 6-14 нельзя безопасно выполнять буквально в текущем порядке и с текущими acceptance criteria. Общая архитектура жизнеспособна, но до реализации нужно закрыть пять контрактных блокеров:

1. **BSL facts из live workspace service сейчас не привязаны к immutable snapshot.** Это противоречит receipt-grade freshness contract.
2. **Receipt issuance API и публичная wire shape неполны.** Текущий issuer получает только proposals + snapshot и вызывается до вычисления `analysisId`/canonical evidence; такой API физически не может сохранить требуемые task/evidence/analysis digests.
3. **Нет полного guard descriptor matrix и fail-open/fail-closed semantics для внутренних ошибок.** Нельзя детерминированно решить, какие инструменты принимают `discoveryReceipt`, что делать при corrupt store/post-snapshot failure и как вести `observe/warn/deny`.
4. **Observation journal/replay не имеет точных v1 bounds, replay schema и wire-поля для operator diagnostics.** Реализовать «bounded», cross-process sequence и crash recovery без дополнительных решений нельзя.
5. **Gold corpus поставлен слишком поздно и дублирует Task 7.** Кроме того, packaged state-machine concurrency нельзя доказать одним stdio-процессом, потому что текущий MCP loop обрабатывает запросы последовательно.

До устранения этих блокеров публичный `unica.project.discover` регистрировать нельзя. После устранения Tasks 6-14 можно завершить, но Task 14 не является финальным gate: обязательный Task 15 из того же плана всё равно должен пройти полностью.

## Подтверждённые противоречия и пробелы

### 1. Task 6: live index/graph нельзя считать snapshot-grade evidence

Спецификация требует, чтобы providers использовали общий snapshot-bound reader и не переоткрывали unconstrained live paths (`spec/architecture/extension-point-discovery.md:765-773`). Explicit fallback тоже обязан сохранять typed provenance, coverage и bounds (`...:1146-1154`).

План, напротив, делает typed workspace/index service основным путём и snapshot reader — только fallback при недоступности service (`docs/superpowers/plans/2026-07-17-project-discovery-receipts.md:541-556`). Текущая реализация service подтверждает проблему:

- request содержит live `source_root`, tool name/args и timeout, но не snapshot fingerprint/manifest (`crates/unica-coder/src/infrastructure/workspace_services.rs:161-189`);
- response содержит только display-oriented `result_text`/`stderr` и RLM db path (`.../workspace_services.rs:554-570`, `:657-660`);
- MCP parser склеивает все text sections (`.../workspace_services.rs:1142-1179`), тогда как план уже запрещает это;
- `SERVICE_SCHEMA_VERSION` всё ещё `1` (`.../workspace_services.rs:20`), то есть старый процесс нельзя отличить от будущего typed contract без обязательного bump.

Простое присваивание returned facts текущего `sourceFingerprint` не доказывает, что analyzer/index читал именно эти bytes. Это false freshness и потенциальная false receipt.

**Рекомендуемая граница:**

- authoritative `DefinitionPort`/`CallGraphPort` в validate mode всегда подтверждает facts через manifest-listed bytes, полученные `SourceSnapshotPort::read_verified()`;
- workspace service/RLM может быть acceleration/hint provider, но его facts нельзя повышать до receipt-grade, пока service не докажет точное равенство snapshot manifest;
- практичный v1: parse/tokenize immutable snapshot bytes как источник истины; service results только сужают candidates и затем полностью перепроверяются snapshot parser;
- response service должен стать typed (`structuredContent` либо ровно один JSON text block), `SERVICE_SCHEMA_VERSION` увеличивается, старый text-only process не переиспользуется;
- provider file/byte/deadline limits должны быть точными. Чтобы не вводить новые неподтверждённые числа, можно переиспользовать утверждённые snapshot ceilings (`200000`, `4GiB`, `120s`) плюс request limits `maxEvidence`/`maxGraphDepth`; wall-clock abort не может возвращать timing-dependent complete prefix.

**Недостающие acceptance tests Task 6:**

- service отвечает fact для старого content, snapshot содержит новый content -> fact rejected/blocking, не relabeled свежим fingerprint;
- live file меняется после capture, service видит изменение, snapshot parser — старое содержимое -> только snapshot fact принимается;
- stale RLM index, old service schema, multi-text MCP content, malformed `structuredContent`, JSON + лишний text -> typed unavailable/contract violation по точному правилу;
- Russian/English `Процедура|Procedure`, `Функция|Function`, `Экспорт|Export`, annotations, multiline signature, default parameters, escaped strings, line/block comments, preprocessor regions;
- dynamic calls, method calls on expressions, ambiguous definitions -> conservative `unknown/bounded`, никогда ложный resolved call;
- exact duplicate fact dedupes, conflicting definition/call remains blocker;
- deterministic file creation/order permutations produce identical facts and digests.

**STOP Task 6:** если хоть один receipt-eligible verdict может зависеть от service/index fact, не подтверждённого immutable snapshot bytes.

### 2. Task 7: механизм 7 противоречит binding matrix; corpus дублируется

Активная binding matrix всё ещё говорит `BindingDetails::ExchangePlan + handles` (`spec/...:128-143`), но gold boundary требует `ExchangePlan -> registered EventSubscription -> exact CommonModule handler` (`spec/...:1232-1240`). Эти два утверждения несовместимы: прямой `handles` edge обходит обязательную subscription.

До Task 7 спецификация должна закрепить один вариант, согласованный с уже подготовленным Task 5 brief: typed `SubscriptionSource`, `ExchangePlan --uses--> EventSubscription --subscribes--> handler`. Старый `ExchangePlan handles handler` должен быть запрещён provider contract test.

Task 7 уже требует 8 families x 6 cases (`plan:573-590`), а Task 13 повторно создаёт те же 48 cases как gold corpus (`plan:999-1004`). Это создаёт две независимые истины и будущий drift.

**Рекомендуемая граница:** до реализации Task 7 создать schema + evaluator для `corpus.json` и использовать его cases в Task 7. Task 13 затем добавляет metamorphic/package/state-machine слои, а не переписывает 48 scenarios второй раз.

**Недостающие acceptance tests Task 7:**

- family 7 direct ExchangePlan callback/BSP orchestration всегда `unknown/unsupported_mechanism_variant`;
- family 8 обязательно сохраняет ownership chain Report/DataProcessor -> Form -> Command -> FormModule handler;
- structural `contains/defines` не попадает в runtime `connection_ports`;
- unrelated provider degradation не блокирует доказанную proposal; material degradation блокирует;
- EDT не вызывает Platform XML/BSL providers вообще;
- every actionable candidate имеет connected runtime flow, exact target existence и known compatible support state.

**STOP Task 7:** пока Task 5 contract не принят, family-7 edge model не исправлен в spec/tests и Task 6 freshness gate не закрыт.

### 3. Task 8: «один resolver» требует invocation plumbing, которого нет в file list

План требует, чтобы issuer, guard и native handler потребляли один `CfeMethodPatchPlan` (`plan:637-645`). Но текущий application port передаёт handler только raw `ToolSpec + args + context + dryRun`, а `NativeOperationAdapter` снова резолвит raw args. Task 8 file list не включает обязательные изменения `application/ports.rs`, `infrastructure/native_operations.rs` и `registry.rs`.

Текущий handler также принимает неизвестный `Context` как произвольную annotation (`crates/unica-coder/src/infrastructure/native_operations/cfe.rs:3753-3778`), а module path parser допускает неизвестное module kind как `<other>.bsl` (`.../cfe.rs:4242-4303`). Это расходится со strict mutationIntent contract.

**Рекомендуемая граница:**

- domain parser создаёт immutable normalized `CfeMethodPatchPlanCore` из output-affecting args;
- application resolver добавляет resolved destination source identity/root и exact contained artifact;
- guard передаёт тот же `ResolvedMutationPlan` в `HandlerInvocation`; handler не парсит raw args повторно и не выбирает другой path;
- direct non-guard invocation (dry-run/observe without receipt) использует тот же resolver, а не legacy parser;
- proposal target, destination source set/root, exact artifact, interceptor, context и method kind сравниваются атомарно.

**Недостающие acceptance tests Task 8:**

- uppercase/lowercase aliases дают один canonical digest, conflicting aliases rejected;
- unknown Context, unknown module kind, separators/traversal/control chars, ambiguous case-only source aliases rejected before write;
- proposal target and raw `ModulePath/MethodName` mismatch;
- `ExtensionPath` resolves exactly to declared destination source set; sibling/EDT/config root rejected;
- duplicate decorator **или** generated procedure/function rejected before any byte changes;
- resolver/guard/handler see byte-identical plan/digest; handler cannot silently re-resolve another artifact.

**STOP Task 8:** если issuer, guard и handler могут получить разные plans/digests или handler продолжает самостоятельно разбирать output-affecting raw args.

### 4. Task 9: receipt issuance API сейчас не может выполнить спецификацию

Receipt обязан хранить analysis/task/evidence digests, atomic grants и baseline (`spec/...:1433-1444`). Но текущий `ReceiptIssuanceRequest` содержит только `proposals` и `snapshot`, а port называется `assess()` и возвращает только `ReceiptEligibility` (`crates/unica-coder/src/application/discovery/ports.rs:707-730`). Более того, use case вызывает issuer **до** вычисления `analysisId` и canonical evidence (`.../use_case.rs:139-154`).

Wire contract также неполон: exact `DiscoveryReportWire` содержит `receiptEligibility`, но не optional `receipt` (`.../model.rs:1110-1139`), хотя Task 9 требует вернуть public receipt view.

**Обязательный refactor до persistence:**

1. Сначала собрать/провалидировать providers, graph, verdicts, checks и canonical evidence.
2. Вычислить `analysisId`, domain-separated `taskDigest` и `evidenceDigest`.
3. Resolve **все** mutation intents Task 8 resolver-ом.
4. Только после all-or-nothing validation вызвать `ReceiptIssuerPort::issue(ReceiptIssueRequest)`.
5. Atomic store write возвращает public receipt view; report invariant: `eligible=true` iff valid `receipt` present, `eligible=false` iff receipt absent.

Спецификация должна добавить exact optional `receipt` field в Structured Result и три canonical fixtures: explore/no receipt, validate/ineligible, validate/issued.

**Missing store contracts/acceptance tests:**

- exact persisted `ReceiptRecordV1` schema and canonical digest algorithms;
- exact normal-policy reason codes для corrupt record, unknown receipt schema, incompatible analysis contract и store unavailable. Сейчас public list их не определяет;
- malformed public receipt id rejected as argument error before filesystem access; valid missing id -> `receipt_not_found`;
- process-local registry + real cross-process `fs2::try_lock_exclusive`; one caller reaches handler revision;
- persistent lock inode never deleted; crash/drop releases OS lock; revision reread and expected-revision check under lock;
- Windows replace while another process has read/closed record handles; Unix durability; runtime-job tests rerun after extraction of `atomic_file` (сейчас primitive живёт в `runtime_jobs.rs:1476-1555`);
- atomic writer never exposes partial JSON and cleans only its own temp file;
- workspace-key digest has explicit pure Windows rules (drive/UNC/case/trailing separators) and exact Unix bytes; equal Windows roots collide intentionally, different roots do not;
- list/index API for receipts linked to affected source sets. Task 10 reconciliation нельзя строить на unbounded scan без server-owned limit/index;
- deterministic sorted reconciliation IDs, safe handling corrupt/busy records, bounded inactive-record GC without TTL validity;
- no task/source/raw args/absolute path in persisted record.

**Рекомендация по linked receipts:** в Task 9 добавить repository API `list_linked(source_set_identity)`. Либо хранить проверяемый secondary index, либо определить точный `maxReconcileReceipts` и safe lazy-stale fallback. Без этого каждое изменение может превратиться в unbounded scan.

**STOP Task 9:** любое частично проверенное multi-grant issue; persist до вычисления всех digests/plans; lock acquisition без expected revision; удаление lock file; receipt report без точной wire invariant.

### 5. Task 10: отсутствуют полный descriptor matrix и error semantics

Spec приводит лишь примеры `not_required/advisory_only/enforceable` (`spec/...:875-900`), а Task 10 требует классифицировать every public/native operation (`plan:787-790`). Текущий `ToolSpec` вообще не имеет discovery classification (`application/mod.rs:30-60`). Пока нет исчерпывающей таблицы, разные implementers неизбежно выберут разные policy и schema acceptance для `discoveryReceipt`.

До кода нужна exact spec table для **каждого mutating public tool**. Минимум: только `unica.cfe.patch_method` enforceable; явно перечисленные advisory operations; остальные mutating operations not_required. Read-only tools не принимают receipt.

Также не определено:

- принимают ли advisory operations `discoveryReceipt` (без resolver они не могут безопасно lease/advance; рекомендуемый v1 ответ — нет, schema принимает receipt только для enforceable descriptors);
- observe/warn/deny behavior при receipt store I/O, corrupt schema, pre-snapshot failure, post-snapshot failure и atomic transition failure;
- точное значение «nearest applicable `.v8-project.json`» для nested cwd/source/destination;
- wire outcome, если handler успел изменить bytes, но post snapshot или receipt transition failed;
- exact `MutationEffects.coverage` variants для non-native/broad mutations.

**Рекомендуемая pipeline decomposition:**

1. pure config resolver (env > applicable workspace file > observe), injected в tests; никаких process-global env races;
2. pure descriptor/target decision;
3. pre-snapshot + receipt pre-evaluation -> expected revision;
4. non-blocking lease under which record/revision/fingerprint are reread;
5. handler получает resolved plan;
6. exact typed effects + post snapshot;
7. advance/revoke while current lease held;
8. release current lease;
9. events/cache;
10. linked receipt reconciliation in sorted order;
11. workspace invalidation;
12. observation;
13. result.

Текущий application pipeline вызывает handler сразу после support guard (`application/mod.rs:360-408`) и делает events/cache/invalidation после display `AdapterOutcome` (`:439-462`), поэтому Task 10 должен быть полноценным application refactor, а не добавкой вокруг handler.

**Недостающие acceptance tests Task 10:**

- exhaustive descriptor snapshot: каждый public tool/classification, enforceable iff resolver exists;
- `discoveryReceipt` schema только там, где resolver может его проверить;
- invalid env/file mode is config error, not silent observe; exact precedence and nested applicability;
- support block wins over missing receipt and handler never runs;
- dry-run resolves target/diagnostic, но не lease/transition/write;
- two processes, same receipt/revision: second is busy/stale before handler;
- post-snapshot unavailable after real write -> conservative revoke attempt + stable result/operator diagnostic; stale active record still cannot be reused because fingerprint mismatch;
- atomic advance failure after write cannot yield a reusable active receipt;
- exact/broad/unknown effects; display `changes/artifacts` never authorizes scope;
- no second receipt lock while current lease held;
- mode matrix including valid, missing, invalid, busy receipt and advisory/not_required;
- `off` does no guard evaluation/observation, but future receipt reuse remains protected by fingerprint validation.

**STOP Task 10:** handler invocation до successful enforceable lease in deny; nested lock under current lease; post-write uncertainty that leaves a reusable receipt; policy decision reconstructed from display strings.

### 6. Task 11: observation contract недостаточно точен

Spec обещает OS-locked schema-versioned JSONL, bounded rollover/counters и pure replay (`spec/...:1009-1032`), но ни spec, ни plan не задают:

- maximum segment bytes/count/retention;
- cross-process monotonic sequence allocation;
- crash semantics для partial final JSONL line;
- journal-vs-counter source of truth and recovery after one write succeeds, а другой нет;
- exact `GuardReplayInputV1` fields/canonical digest;
- top-level public/operator field для `discovery_observation_write_failed`.

Task 11 test sketch использует `operator_diagnostics`, а Task 12 OperationResult его не проектирует. Existing `diagnostics` уже используется runtime output (`application/mod.rs:77-80`), и план прямо запрещает перегружать его.

**Обязательные spec additions до Task 11:**

- exact `ObservationRecordV1`, `GuardReplayInputV1`, `ObservationCountersV1` JSON fixtures;
- exact rollover constants and lock path; lock inode persistent;
- `journal` authoritative, counters derived/rebuildable (рекомендуется), либо другая явно атомарная модель;
- top-level optional `operatorDiagnostics: [stable_code...]`, excluded from authoritative result comparison and all stable digests;
- domain-separated hash rules for workspace/targets/resolver/snapshots/receipt/effects/outcome;
- no raw values even in error paths/log messages.

**Недостающие acceptance tests Task 11:**

- multi-thread **и multi-process** append/rollover; unique monotonic sequence;
- crash/partial final line: corrupt fragment reported separately, next valid append remains parseable;
- unknown schema/corrupt line excluded, never rewritten as valid;
- counter-write failure and journal-write failure do not alter handler/receipt/guard authoritative bytes;
- audit can rebuild/check counters from journal;
- live decision and replay invoke the same pure evaluator and produce 100% parity;
- timings/sequence excluded from replay input digest;
- recursive redaction/property test with sentinel task/source/raw args/absolute roots/artifact names;
- CLI help, missing workspace, corrupt journal, replay mismatch exit codes; audit CLI absent from MCP tools/list.

**STOP Task 11:** пока bounds/replay schema/operatorDiagnostics wire не записаны в active spec; если telemetry failure меняет authoritative `ok`, handler outcome, receipt transition или guard decision.

### 7. Task 12: public wire contract нужно зафиксировать полными fixtures

Task 12 правильно отложен до receipts/guard, но exact outer shape остаётся неоднозначной. Spec говорит `data.sourceReadiness` и `data.snapshotCapture`, а успешный discovery example показывает только голый report. Рекомендуемый единый wire contract:

```json
"data": { "discovery": { "schemaVersion": 1 } }
"data": { "sourceReadiness": { "reasonCode": "...", "retryable": false, "sourceSet": "...", "role": "analysis" } }
"data": { "snapshotCapture": { "reasonCode": "...", "retryable": true } }
```

`OperationData` — disjoint one-key object; `discoveryGuard` и `operatorDiagnostics` — отдельные top-level optional fields, absent rather than `null`. Это нужно продублировать в active spec до реализации, как ранее потребовал пользователь.

Текущий common schema добавляет `cwd`, `dryRun`, `confirm` всем tools (`tool_contracts.rs:1490-1508`), поэтому discovery требует отдельной schema branch: только `cwd` из common fields. Runtime validation должна strip `cwd` и strict-deserialize остальное в `DiscoverRequest`.

Текущий stdio использует `BufRead::lines()` и аллоцирует всю строку до проверки (`interfaces/mcp.rs:7-28`), поэтому simple `line.len()` после read не выполняет 4 MiB safety requirement.

**Недостающие acceptance tests Task 12:**

- full JSON fixtures: success/no receipt, success/issued receipt, source readiness, snapshot capture, guard warn, guard deny, observation failure;
- schema/runtime parity for every nested unknown field, explore forbids proposals, validate requires non-empty proposals;
- only enforceable mutation schema accepts strict receipt id; read-only/advisory/not_required reject it;
- `ProjectDiscover` never reaches `ApplicationPorts::invoke_handler`/AdapterOutcome;
- existing tools serialize byte-compatible fields; optional additions absent where irrelevant;
- source/snapshot typed failures are `tools/call` success transport + `ok=false`, provider contract violation remains operation/transport error;
- bounded reader checks bytes excluding newline **before** JSON allocation, rejects `4MiB+1`, accepts exactly `4MiB`, drains the oversized frame, then successfully handles a following valid request;
- no second public MCP server/analyzer tool.

**STOP Task 12:** до green Tasks 5-11, real receipt issuer/store/lease/guard, exact wire fixtures и bounded framing. Нельзя публиковать eligibility-only/no-op receipt half-contract.

### 8. Task 13: corpus нужно разнести по фазам и добавить process/package proof

Task 13 Step 3 разрешает «close every gap in production code», но commit add-list включает не все возможные infrastructure/guard/receipt файлы (`plan:1019-1041`). Это ломает reviewable task boundary. Python package-corpus test создаётся, но в Task 13 нет команды, которая его запускает.

**Рекомендуемое разбиение:**

- перед Task 7: strict corpus schema, 48 IDs/cases, shared evaluator contract;
- Tasks 7-10: соответствующие subsets обязаны проходить по мере реализации;
- Task 13: 20+ metamorphic variants, deterministic quality metrics, 12 state-machine cases, packaged runner and corpus parity;
- production gaps исправляются отдельным RED test + fix commit в owning task/module, не скрываются внутри corpus commit.

**Недостающие acceptance tests Task 13:**

- Rust/Python evaluators читают тот же corpus schema, одинаковые case IDs/counts и не имеют duplicated expectations;
- epoch-only mutation сохраняет evidence/analysis/verdict; content fingerprint mutation меняет freshness и stale receipt;
- directory creation order + provider order permutations;
- exact duplicates vs same-ID/different-payload collision;
- material vs unrelated provider faults;
- rename всех XML/BSL/query/resolver/expected identities вместе, включая Unicode/case variants;
- runner требует explicit packaged `--binary`, проверяет resolved path outside source `target/`, никогда не вызывает `cargo run`;
- каждый mutable case получает isolated workspace/cache copy;
- concurrency case запускает **два packaged `unica` processes** с общим workspace/cache. Один stdio process не подходит: текущий loop sequential;
- two-step rolling grant, failed partial write, broad out-of-band mutation, corrupt receipt, busy lock, stale revision;
- обязательная команда в Task 13: `python3.12 -m unittest tests.ci.test_packaged_discovery_corpus -v`.

**STOP Task 13/promotion:** любая zero-tolerance failure из spec; runner использует source-tree binary; state concurrency доказана только threads/одним stdio process; ожидания ослаблены вместо production fix.

### 9. Task 14: package/skill/release proof требует уточнений

Package sources of truth подтверждают одну public registration (`plugins/unica/.mcp.json`) и target-specific bundled `unica` (`plugins/unica/third-party/tools.lock.json`). Их архитектуру менять не нужно; generated package должен лишь переписать launcher на packaged binary.

Но остаются пробелы:

- `release-assessment.py::EXPECTED_PUBLIC_TOOLS` не содержит discovery и fake MCP tests тоже должны быть обновлены;
- provenance test требует entry для **каждого** packaged skill. `project-discovery` — Unica-owned, а текущая schema моделирует только upstream entries. Нужно явно выбрать provenance semantics: рекомендуемый минимальный вариант — `primarySource: "unica"` + `ignored-with-reason` под реальным secondary guidance upstream, как `api-design`; нельзя притворяться, что skill портирован из donor;
- capsule schema/redaction не определены; existing assessment содержит paths/errors/artifacts, поэтому нельзя просто назвать весь assessment «sanitized capsule»;
- `discovery-shadow-replay.py` и native audit CLI не должны дублировать policy evaluator: script только orchestration вокруг explicit packaged binary/CLI;
- release workflow запускает full Rust tests только на Linux (`.github/workflows/unica-plugin-release.yml:95-102`). Windows/mac jobs собирают tools, но не запускают receipt locking/atomic/path tests (`:123-169`); assessment запускает только packaged Linux (`:234-266`). Для новой cross-platform persistence этого недостаточно.

**Недостающие acceptance tests Task 14:**

- skill exact workflow explore -> validate -> immediate exact mutation; unknown/contradicted/no receipt semantics; no scripts/internal analyzers; receipt only for exact enforceable call; partial failure stops/re-investigates;
- `agents/openai.yaml` syntax/content and package inclusion;
- provenance entry says Unica-owned truthfully and offline validator remains green;
- generated archive has one `unica` MCP server, project-discovery skill, agent metadata, provenance, native binary; no source/corpus/research/direct scripts;
- packaged `tools/list` contains exactly one discovery tool with strict schema;
- exact `DiscoveryShadowCapsuleV1` fixture contains hashes/counters/typed outcomes only; recursive test rejects task/source/raw args/absolute BSP/cache paths/unhashed artifact names;
- release assessment invokes packaged discovery and packaged observation audit/replay, not source binary;
- native runner jobs (Windows, macOS, Linux) execute focused receipt atomic/lock/path tests and a packaged tools/list/discovery smoke. Если workflow не расширяется, Windows/mac proof нельзя заявлять.

**STOP Task 14/release:** package default отличается от `observe`; skill вызывает scripts/internal tools; provenance приписывает ложного donor; capsule содержит forbidden raw data; Windows runner существует, но новая locking implementation на нём не тестируется; real shadow thresholds выдаются за выполненные.

## Исправленный dependency order

1. **Gate 5:** принять Task 5 и синхронизировать active spec (особенно SubscriptionSource/family 7 и ownership family 8).
2. **Task 6A:** authoritative snapshot-byte BSL tokenizer/definitions/calls + exact bounds.
3. **Task 6B:** typed workspace service schema v2 как optional acceleration; stale/live facts перепроверяются 6A.
4. **Corpus seed (часть Task 13):** strict `corpus.json` schema/evaluator и 48 case definitions до mechanism wiring.
5. **Task 7:** mechanisms используют corpus seed и green snapshot-grade providers.
6. **Task 8:** shared CFE parser + resolved plan + application-to-handler invocation plumbing.
7. **Task 9A:** domain receipt/public-view/record schemas, digest constructors, reason-code matrix.
8. **Task 9B:** shared atomic file, receipt repository, linked lookup/index, process + OS lease.
9. **Task 9C:** reorder use case and issue persisted receipt only after all digests/plans are ready.
10. **Task 10A:** exhaustive descriptor matrix, config resolver, pure guard decision.
11. **Task 10B:** typed effects and fixed application pipeline with lease-through-handler.
12. **Task 10C:** post-snapshot transition and other-receipt reconciliation after lease release.
13. **Task 11:** exact journal/counters/replay schema, bounds, CLI, operator diagnostics.
14. **Task 12:** common envelope + strict public MCP + bounded stdio framing.
15. **Task 13 remainder:** metamorphic/metrics/state-machine/package runner, including cross-process cases.
16. **Task 14:** MCP-first skill, truthful provenance, package artifact, sanitized BSP shadow/replay, native CI matrix.
17. **Task 15:** full fmt/clippy/Rust/Python/package/cross-platform smoke + two independent reviews. Task 14 не считается финалом без этого gate.

## Global hard stop conditions

Работу нужно остановить и показать владельцу, если обнаружено любое из следующего:

- receipt-grade verdict зависит от live/unverified provider data;
- family 7 всё ещё допускает direct ExchangePlan -> handler;
- public report может сказать `eligible=true` без persisted receipt;
- одна proposal/grant может авторизовать cross-product соседних параметров/targets/destinations;
- handler для enforceable deny достигается без valid lease или два handler-а используют одну revision;
- post-write uncertainty оставляет receipt повторно применимым;
- current lease удерживается при lock другого receipt;
- telemetry меняет authoritative outcome либо сохраняет forbidden raw data;
- `unica.project.discover` появляется в tools/list до полного receipt/guard state machine;
- packaged corpus использует source binary/fallback либо concurrency проверяется только одним sequential stdio loop;
- любая zero-tolerance safety failure из spec;
- release/skill обещает `warn/deny`, пока реальные observation thresholds не выполнены;
- Windows/mac locking/atomic behavior заявляется проверенным без native runner evidence.

## Что уже согласовано и не нужно перепроектировать

- один public MCP server `unica`;
- один public tool `unica.project.discover` с modes `explore|validate`;
- Platform XML receipt-grade, EDT diagnostic-only/never receipt in v1;
- content fingerprints authoritative, `workspaceEpoch` diagnostic-only;
- atomic grants без cross-product;
- no receipt TTL in v1;
- lease spans handler + effects + post snapshot + transition;
- support guard раньше discovery guard;
- package default `observe`; promotion only by live evidence;
- skill MCP-first, no return to packaged direct scripts.

