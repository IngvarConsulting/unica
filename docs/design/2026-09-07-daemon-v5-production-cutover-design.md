- Date: `2026-09-07`
- Status: `approved`
- Decision: `DEC.2026-09-07.DAEMON-V5-PRODUCTION-CUTOVER`

# Перевод production daemon с протокола v3 на v5 и снятие v3

## Задача

Production frontend (`unica` в режиме stdio) поднимает пользовательский
daemon протокола v3. Протокол v5 с durable ReceiptLedger реализован
полностью, доказан 65 именованными чёрноящичными тестами на трёх ОС
(D0, #647) и стоит рядом, но в продукт не подключён: его выбирает только
тестовый вход. Задача — перевести production на v5, довести до
подтверждения на конвейере и после этого удалить v3 из системы, не оставив
двух wire identity ни в коде, ни в реестре.

Источники требований: план W0a → W0b → W0c в
[`docs/plans/2026-08-28-v0-13-completion.md`](../plans/2026-08-28-v0-13-completion.md),
семантика фронтенда в
[2026-08-28-daemon-receipt-ledger-design.md](2026-08-28-daemon-receipt-ledger-design.md),
порядок волн в
[2026-08-28-v0-13-completion-wavefront-design.md](2026-08-28-v0-13-completion-wavefront-design.md),
пункты J0-1 и J0-2 зонтичной задачи #581.

## Что есть сейчас

**Выбор протокола — только identity.** `CoreIdentity::production()`
даёт digest v3, `production_v5()` — digest v5. Режим `--daemon` выбирает
рантайм по identity (`interfaces/daemon.rs`): ровно v5-digest поднимает
`runtime_v5::run_daemon`, любой другой — `server::run_daemon`. Каталоги
состояния разведены: `daemon-p3-<digest>` и `daemon-p5-<digest>`, поэтому
v3-процесс и v5-процесс никогда не делят endpoint, receipts и TaskStore.
Производственный вход `connect_default_user_daemon` строит клиент v3 с
`production()`; больше ничего в продукте v3 не выбирает.

**Серверная сторона v5 готова.** `runtime_v5.rs` держит полный цикл:
handshake, reserve до валидации, Direct/Task, recover, ACK, cancel,
startup reconciliation, fail-stop, capacity. Предметный сервис v13 у него
тот же, что у v3 (`DaemonServerConfig::invocation_service`), поэтому
переключение не меняет исполнение инструментов, только durable-контур
вокруг них.

**Клиентской стороны v5 нет.** `client_v5.rs` умеет connect-or-spawn
(имя `_for_protocol_test`), Hello/Ping, submit/cancel/recover/ACK
(без вызывающих, `dead_code`), а `get/wait/cancel_task` закрыты под
`cfg(test | feature)`. Маршрутизатор MCP (`canonical_daemon_router`),
типы обработчиков, проекции native Task (`interfaces/task_projection.rs`)
и compatibility-инструментов (`application/v13/task_tools.rs`) написаны
на типах v3: `DaemonOwner`, `InvocationResponse`, `DaemonTaskSnapshot`,
`DaemonErrorCode`. Модулей `task_projection_v5.rs` и `task_tools_v5.rs`,
которые план называл, в дереве нет.

**Свидетельства v5 не идут в конвейере.** Контракт
`tests/daemon_receipt_ledger.rs` собирается только с признаком
`receipt-ledger-test-support` (`required-features`), а команды nextest в
`run-tests.py` и `allure_results.nextest_list` признаков не передают:
65 тестов не попадают ни в план, ни в отчёт. Пять тестов
`tests/daemon_process.rs` стоят под `#[ignore]` «до маршрутизации яруса»;
из-за `#[ignore]` сломан и их собственный дочерний fixture: `spawn_frontend`
запускает тест по имени без `--include-ignored`, ребёнок ничего не
исполняет, и два v3-теста падают по таймауту готовности.

**Реестр закреплён на v3.** `CTR.WIRE.DAEMON-INVOCATION-PROTOCOL`
(версия 3, producer `protocol.rs`, четыре запроса), восемь
`INV.APP.DAEMON-*` с проверками в `daemon/mod.rs`, `task_store.rs`,
`invocation_store_actor.rs`, `application/invocation.rs`,
`CTR.APP.DAEMON-LONG-WORK-CAPABILITIES` с producer `server.rs`; владельцы —
`DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE` и
`DEC.2026-08-24.NATIVE-TASK-PROJECTION-SLICE`. Wavefront-замысел прямо
требует, чтобы после W0c в реестре осталась ровно одна текущая wire
identity.

**Замер 07.09.2026, локально (Apple Silicon, профиль `default`, все ядра).**
Контракт ledger и пять process-тестов: 70 тестов, 559 с стены, 67 зелёных
(три нестабильных прошли повтором), три красных: нагрузочный
`wall_clock_writer_sustains_32_receipts_per_second_on_posix` (p99 428 мс
при бюджете 250 мс под параллельной нагрузкой) и два v3 process-теста
(сломанный fixture, см. выше). Самые долгие — тесты ёмкости и нагрузки,
200–520 с каждый.

## Развилки и выбор

**Фронтенд строится на wire v5 напрямую, а не через адаптер к типам v3.**
Адаптер «v5 → DaemonTaskSnapshot v3» сохранил бы v3-типы в интерфейсе и
сделал бы удаление v3 второй переделкой того же кода. Вместо этого
интерфейс переводится на `V5DaemonTaskSnapshot` и коды `V5DaemonErrorCode`
теми же функциями и с теми же именами проверок реестра; v3-ветки этих
функций уходят вместе с v3.

**Дисциплина квитанций — как в замысле ledger.** Direct-ответ приходит
как `V5PendingDirectReceipt`; фронтенд сначала строит окончательную
проекцию (`CallToolResult` для Completed, `ErrorData` -32603 с закрытым
кодом для Failed/Cancelled), и только после этого шлёт ACK с exact
`receiptKey` и `terminalDigest`. Ошибка проекции (размер, сериализация)
ACK не шлёт: терминал остаётся unacked и истекает через час без replay.
ACK идёт на той же сессии с собственным коротким бюджетом, а не на
остатке транспортного, который к этому моменту может быть исчерпан.
Ответ на ACK (tombstone, `tombstone_capacity`, `receipt_expired`,
транспортная ошибка) на результат для хоста не влияет: ACK доказывает
передачу daemon → frontend, не доставку хосту.

**Потеря ответа на submit — восстановление, не повтор.** Если запись
frame прошла, а ответ не пришёл, фронтенд открывает новую сессию и шлёт
`RecoverInvocationReceipt` с ключом, который умеет вычислить сам
(`invocationId`, `reservedTaskId`, digest core identity, инструмент,
нормализованный hash аргументов, hash workspace hint —
`receipt_key_is_canonicalized_identically_by_client_and_server`). Direct
terminal → проекция и ACK, Task → снимок, `receipt_not_found` → submit не
дошёл, честная транспортная ошибка; `ReceiptPending` → ждать до
`acceptedEpochMs + originalBudgetMs + 125 мс` в пределах своего cutoff и
спросить снова. UUID никогда не генерируются повторно, за горизонт
cutoff восстановление не продолжается. Двух исполнений одного вызова не
бывает ни в одной ветке.

**Compatibility-инструменты держат действующий контракт v2, а не форму
сценарного бегунка.** `CTR.WIRE.COMPATIBILITY-TASK-TOOLS` (версия 2,
`DEC.2026-09-01.V0-13-REFUSAL-DISCIPLINE`) кладёт код отказа в
`diagnostics[]` и оставляет `data.task` чистым снимком. Проекции в
сценарном бегунке `receipt_scenario_v5.rs` написаны раньше этого решения и
несут `data.code`; они — свидетельство ledger, а не продукта, и остаются
как есть. Native-проекция совпадает с замыслом побайтово: `status`
queued/working → `working`, `completed.result` — тот же `CallToolResult`,
`failed.error` — таблица девяти закрытых причин, включая
`outcome_uncertain`, `task_capacity`, `workspace_capacity`,
`workspace_registry_failed`.

**Отмена со стороны хоста в этом переходе не расширяется.** v3 не
передаёт `notifications/cancelled` в daemon; v5 умеет `CancelInvocation`
по exact ключу. Ленивая отмена — отдельный шаг после переключения: она
меняет наблюдаемое поведение хоста и требует своего свидетельства.

**Свидетельства v5 идут своими ярусами.** Контракт ledger — второй
вызов nextest с признаком (`-p unica-coder --features
receipt-ledger-test-support --test daemon_receipt_ledger`); признак не
включается на весь workspace, потому что под ним рантайм ведёт себя иначе
на пути fail-stop (`cfg(all(feature, not(test)))`), а тестировать надо
поставляемый код. Размер `medium` (очередь, `main`, релиз) для всего
контракта, кроме тестов ёмкости и нагрузки; те — `large` (ночь, все три
ОС, `threads-required = "num-cpus"`, срок 900 с). На Windows ночью
контракт идёт целиком, как весь набор. Ярус process-тестов
маршрутизируется снятием `#[ignore]`: они `kind(test)` и уже `medium`.

**Одна wire identity в реестре — одним решением.** Новое решение
заводится вместе с переключением по умолчанию: оно заменяет
`DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE` и
`DEC.2026-08-24.NATIVE-TASK-PROJECTION-SLICE`, принимает во владение
каждое правило, чья проверка или producer переезжают на v5, переводит
`CTR.WIRE.DAEMON-INVOCATION-PROTOCOL` в версию 5 и переносит проверки
`INV.APP.DAEMON-*` на именованные тесты v5. Решения, чьи свидетельства
достаточно переадресовать (`realized` — записываемое поле), не
заменяются: `DEC.2026-08-23.USER-CORE-DAEMON-SLICE` получает v5-версию
того же process-теста под тем же именем. `DEC.2026-08-28.DAEMON-RECEIPT-LEDGER`
уже `active`, описывает сам ledger и остаётся в силе.

**v3 снимается после подтверждения, а не вместе с переключением.**
Переключение и снятие — разные риски: первое доказывается прогоном
продукта, второе — тем, что после удаления ни одна проверка реестра не
осиротела. Между ними — очередь, push в `main`, ночь с Windows и полный
контур с дымом упакованного MCP.

## Шаги

| Шаг | PR | Содержание | Готово, когда |
| --- | --- | --- | --- |
| A | свидетельства v5 в конвейере | эта записка; второй вызов nextest для контракта ledger; план и результаты из двух вызовов; ярусы `medium`/`large` в `.config/nextest.toml`; снятие `#[ignore]` с process-тестов; стражи состава | `daemon_receipt_ledger` и `daemon_process` видны в отчёте очереди; large-подмножество — в ночном прогоне |
| B | фронтенд v5 | клиент: `connect_or_spawn`, `connect_peer`, типизированные ошибки, операции Task без cfg; ключ квитанции на клиенте; проекции native и compatibility на `V5DaemonTaskSnapshot`; маршрутизатор v5 с ACK и recover; тесты на fake-сервере v5 и на настоящем рантайме в потоке | маршрутизатор v5 доказан тестами, production всё ещё v3 |
| C | переключение и реестр | `connect_default_user_daemon` → v5; тест identity переписан под новый default (J0-2); решение-преемник, контракт протокола версии 5, проверки `INV.APP.DAEMON-*` на v5; process-тесты на v5 identity | реестр зелёный с одной wire identity; PR влит через очередь |
| D | доказательство | push в `main`, ночь (Windows), ручной запуск полного контура с дымом упакованного MCP на трёх ОС; запись в «Ход» | все линии зелёные на вершине с v5 или каждый красный разобран |
| E | снятие v3 | клиент, протокол и роутер v3, v3-ветки проекций, цикл сервера v3 в `server.rs` (общий предметный сервис остаётся), `FileInvocationStore`/`invocation_store_actor`, `production()` = v5, каталог `daemon-p3-*` больше не создаётся; тесты и документы v3 | `grep` не находит `unica-daemon-jsonl-3`, `DaemonOwner`, `protocol::DaemonTaskSnapshot`; реестр и стражи зелёные |

Шаг E может идти несколькими PR по слоям (фронтенд → сервер → хранилища),
каждый через очередь.

## Критерии готовности

| Критерий | Как измеряется |
| --- | --- |
| Production поднимает v5 | дым упакованного MCP на трёх ОС; в каталоге состояния появляется `daemon-p5-*`, `daemon-p3-*` — нет |
| Ни одного двойного исполнения | тесты recover: потеря ответа на submit, крах фронтенда до ACK, повтор ACK |
| Проекции побайтово | `completed.result` == direct `CallToolResult`; `failed.error` — девять причин; compatibility по контракту v2 |
| Контракт ledger в отчёте | сайт показывает `unica-coder::daemon_receipt_ledger` в линии `main`; large-подмножество — в ночи |
| Реестр | `registry.py --check`, `immutability.py --base origin/main`, `tests/arch` зелёные; ровно одна wire identity |
| Стоимость | шаг тестов Rust в очереди растёт не больше чем на длительность второй компиляции с признаком; замер в «Ход» |

## Реестр: что переустанавливается на шаге C

| Запись | Сейчас | После |
| --- | --- | --- |
| `CTR.WIRE.DAEMON-INVOCATION-PROTOCOL` | версия 3, producer `protocol.rs`, check `mod.rs::invocation_protocol_round_trips_all_four_strict_requests_and_closed_responses` | версия 5, producer `protocol_v5.rs`, check `protocol_v5.rs::strict_v5_client_decoder_round_trips_every_closed_request_kind`; текст: identity `unica-daemon-jsonl-5`, десять запросов, receipt/ACK/recover, коды v5 |
| `CTR.APP.DAEMON-LONG-WORK-CAPABILITIES` | producer/check `server.rs::daemon_exact_long_work_ownership_contract` | producer `runtime_v5.rs`, check `tests/daemon_receipt_ledger.rs::production_known_long_task_executes_after_the_initial_working_projection`; версия 1 — форма capability не меняется |
| `INV.APP.DAEMON-INVOCATION-OWNERSHIP` | `mod.rs::daemon_executes_one_canonical_invocation_and_poll_cancel_never_relaunches_it` | `daemon_receipt_ledger.rs::exact_duplicate_preserves_cutoff_without_second_domain_callback` |
| `INV.APP.DAEMON-INVOCATION-HANDOFF` | `mod.rs::daemon_invocation_receipt_deadline_is_single_and_never_replenished` | `runtime_v5.rs::complete_v5_frame_near_cutoff_cannot_receive_a_fresh_response_budget`, `daemon_receipt_ledger.rs::cutoff_during_admission_projects_exact_unbound_task` |
| `INV.APP.DAEMON-TASK-PERSISTENCE` | `mod.rs::durable_handoff_persists_only_closed_hashes_not_arguments_paths_or_failure_text` | `invocation_store_v5.rs::all_five_task_statuses_round_trip_with_the_exact_selected_fields`, `record_rejects_wrong_schema_unknown_duplicate_and_every_missing_root_field` |
| `INV.APP.DAEMON-TASK-RECOVERY` | восемь проверок миграции v1→v2 в `task_store.rs` | `task_store_v5.rs::recovery_terminalizes_queued_without_starting_domain_work`, `runtime_v5.rs::startup_terminalizes_pre_task_receipts_without_replaying_domain_work`, `daemon_receipt_ledger.rs::restart_begun_without_committed_handoff_is_direct_outcome_uncertain` |
| `INV.APP.DAEMON-TERMINAL-RECONCILIATION` | `application/invocation.rs::terminal_publication_faults_reconcile_without_reexecution_or_false_idle` | `task_store_v5.rs::completed_terminal_cas_reconciles_commit_uncertain_by_exact_readback`, `daemon_receipt_ledger.rs::every_cross_store_crash_point_reconciles_without_split_brain` |
| `INV.APP.DAEMON-STORE-FAIL-STOP` | семнадцать проверок в `invocation_store_actor.rs`, `task_store.rs`, `server.rs`, `mod.rs` | `receipt_ledger_actor.rs::*_fail_stops_actor`, `runtime_v5.rs::commit_uncertain_is_returned_before_process_owned_fail_stop_retains_endpoint`, `displaced_receipt_authority_fail_stops_until_process_death`, `task_store_v5.rs::capacity_never_lazily_expires_terminal_records_and_not_found_is_typed`, `daemon_receipt_ledger.rs::noncooperative_prepare_forces_fail_stop_after_two_second_grace` |
| `INV.APP.DAEMON-ACTOR-AUTHORITY` | `mod.rs::canonical_invocation_authority_is_actor_bound_and_revision_fenced` | `daemon_receipt_ledger.rs::bound_task_start_rejects_missing_foreign_stale_actor_proof_without_mutation` |
| `INV.APP.DAEMON-ACTOR-CAPACITY` | `server.rs::daemon_workspace_actor_admission_is_concurrent_bounded_and_fail_closed` | без изменений: реестр actor общий для v3 и v5 и остаётся в `server.rs` |
| `INV.WIRE.NATIVE-TASK-CAPABILITY`, `CTR.WIRE.NATIVE-TASK-PROJECTION`, `INV.WIRE.V13-TASK-PROFILES`, `CTR.WIRE.COMPATIBILITY-TASK-TOOLS` | проверки в `mcp.rs` | те же имена проверок, тесты переписаны на v5; владелец native-проекции — решение-преемник |
| `DEC.2026-08-23.USER-CORE-DAEMON-SLICE` | `realized: tests/daemon_process.rs::two_frontend_processes_race_to_one_daemon_pid_record_and_endpoint` (v3) | тот же тест на identity v5 |

Точные имена сверяются с деревом при заведении решения; таблица — карта,
а не текст записей.

## Чего в плане нет

- Ленивая отмена по `notifications/cancelled` через `CancelInvocation`.
- Публичный idempotency/resume API, `unica.receipt.*`, второй MCP-сервер —
  их запрещает замысел ledger.
- Перенос общего предметного сервиса из `server.rs` в отдельный модуль:
  пути проверок реестра остались бы теми же только при сохранении файла.
- Переключение публичной версии (`0.13.0-rc.1`, G6) — отдельные ворота
  зонтичной задачи.

## Ход

**Шаг A, 07.09.2026.** Контракт ledger в одиночном локальном прогоне
(профиль `default`, все ядра): 65 тестов, все зелёные, 572 с стены, 2547 с
суммарно; четыре прошли повтором (`wall_clock_writer…`, два
`link_capacity_…`, `deterministic_horizon_load…`). Девять тестов по
200–530 с плюс два настенных ворот (`wall_clock_writer…` 60 с,
`thirty_two_lazy_cancel…` 125 мс) объявлены `large`; остальные 56 — 340 с
суммарно, самый долгий 80 с — идут `medium`. Регулярное выражение nextest
читается буквально, перенос строки внутри `test(/…/)` — символ имени,
поэтому ярус объявлен по одному `test(/^имя$/)` на строку. Причина двух
красных v3 process-тестов — `#[ignore]` на fixture: дочерний процесс
запускал тест по имени без `--include-ignored`; снятие атрибута чинит их
без правки кода.

**Шаг A на `main`, PR #773 влит 07.09.2026 через очередь.** Прогон push в
`main`: план Rust 4614 тестов на ubuntu и 4621 на macOS (контракт ledger
и process-тесты в плане), второй вызов nextest с признаком записал по 56
результатов `medium` на каждом раннере; вторая компиляция и прогон — 1 мин
48 с на ubuntu и 2 мин 4 с на macOS. Джоба Rust целиком: 7 мин 21 с и
10 мин 51 с. Сайт пересобрался. Ночь запущена руками для яруса `large`.

**Шаг B.** Клиент v5: `connect_or_spawn`, `connect_peer_before`,
типизированные `V5TransportError` (не отправлено / ответ потерян) и
`V5TaskExchangeError`, операции submit/recover/ACK/get/wait/cancel с
абсолютным сроком. Проекции v5 в `task_projection.rs`: native Task,
`DirectProjection` с таблицей девяти закрытых причин. Маршрутизатор
`interfaces/daemon_router.rs`: ACK после проекции, recover по ключу,
ожидание `ReceiptPending` до `accepted + budget + 125 мс` в пределах
cutoff. Тесты: сценарный fake-daemon по кадрам (ACK с точным digest,
потеря ответа без повторной отправки, pending за cutoff, чужая квитанция,
подделанный digest) и настоящий рантайм v5 в потоке (tombstone после ACK,
закрытая причина без текста, known-long handoff с get/wait/cancel).
Находка: ответ с поддельным digest строгий декодер клиента отвергает, и
клиент идёт в recover — «oversized Direct» до проекции не доходит, его
отсекает сам daemon (`canonical_v5_terminal`).

**Шаг C.** `CoreIdentity::production()` — digest v5, `production_v3()` —
явный seam для тестов рантайма v3 (под `cfg(test | feature)`);
`connect_default_user_daemon` поднимает `V5DaemonProcessOwner`. Роутер MCP
переведён на `CanonicalDaemonRouter`: прямой результат приходит уже
проекцией (`SurfaceToolOutcome::Direct`), снимки — `V5DaemonTaskSnapshot`,
коды — `V5DaemonErrorCode`; проекции v3 и роутер v3 удалены из `interfaces`.
Cutoff `unica.task.result` выводится от момента приёма запроса
(`received_at + waitMs + 125 мс`), а не от входа в роутер: так контракт
compatibility-инструментов держится и при вызове роутера напрямую. Клиент v5
получил checkpoint после разбора ответа: поздний payload не публикуется и
травит сессию. Тесты MCP переписаны на fake-daemon v5 (сессия на поток,
задержка handshake и ответа вместо ручных часов) и на живой рантайм v5 в
потоке (перезапуск с сохранением Task); «враждебные формы снимка» ушли —
закрытый union v5 их не представляет, остались утечки текста причины.
Process-тест гонки двух фронтендов переведён на identity v5: второй daemon
на том же корне получает `timed out waiting for stable receipt authority`.
Реестр: решение `DEC.2026-09-07.DAEMON-V5-PRODUCTION-CUTOVER` заменяет
ROUTING-SLICE и NATIVE-TASK-PROJECTION-SLICE, контракт протокола — версия 5,
проверки `INV.APP.DAEMON-*`, `INV.APP.EXACT-LONG-WORK-OWNERSHIP` и
`CTR.APP.DAEMON-LONG-WORK-CAPABILITIES` — на тестах v5. Находка: страж
неизменности разбирает Rust через tree-sitter, и `&raw` он читает как
raw-borrow — переменную с таким именем брать по ссылке нельзя.
Джоба `guards` не ставила зависимости наборов, и страж на CI отказывал
текстом «tree-sitter Rust parser unavailable» — починено отдельным PR
(#778); ночной `large` на Windows не уложился в час — срок джобы Rust для
`profile: large` поднят до трёх часов (#777).

**Шаг D, 07.09.2026, после вливания #776 (`main` = 30b9d4bf).**
Push-прогон `main`: с первой попытки красный на ubuntu —
`two_frontend_processes_race_to_one_daemon_pid_record_and_endpoint`, второй
ping fixture после ухода конкурента получил EAGAIN через 5 с; перезапуск
джобы зелёный, локально 5 из 5 — нагрузочный флейк: под дисковой нагрузкой
проба authority в акторе ledger тянется дольше 2 с, обработчик ping ждёт
её дважды в пределах своего 10-секундного потолка, клиент сдаётся раньше.
Финальный ping в fixture теперь повторяется на новой сессии до трёх раз.
Полный контур (`workflow_dispatch`, `profile: main`): сборка и дым
упакованного MCP на трёх ОС зелёные с daemon v5, пакет и проба bootstrap
зелёные; красной осталась оценка на BSP 3.2.1.446 — четыре сценария
(`workspace-check`, `configuration-view`, `literal-search`, `identity-diff`)
отпали ровно на 7,2 с с «protocol-v5 deadline expired during connect»:
daemon занят первым захватом большой конфигурации, ответ на submit
теряется на cutoff, а восстановлению по ключу не оставалось бюджета. У v3
здесь был закрытый отказ «daemon deadline expired during invocation submit
response», который оценка переигрывает один раз для read-only инструментов.
Правка: восстановление получает собственное ограниченное окно 750 мс после
cutoff — daemon к этому моменту уже передал работу в Task, и хост получает
квитанцию вместо ошибки, — а если и окно закрылось, роутер отвечает тем же
закрытым отказом, что v3: правило хоста «read-only можно переиграть,
мутацию нельзя» не меняется. Ночь: большой ярус на ubuntu и macOS зелёный,
Windows — под новым трёхчасовым сроком, итог ниже.

**Ночь 07.09.2026 (run 34152595981).** Ubuntu и macOS: девять тестов
яруса `large` зелёные, настенные ворота
`wall_clock_writer_sustains_32_receipts_per_second_on_posix` держат
32 квитанции в секунду на обоих раннерах (60 с) — открытый вопрос записки
закрыт. Модель горизонта: 415 с на ubuntu, 465 с на macOS. Windows уложился
в 85 минут при новом сроке 180: контракт ledger 64 из 65, не влезла в два
срока `large` только детерминированная модель горизонта
(`deterministic_horizon_load_does_not_saturate`) — она платформе
безразлична и снята с Windows-ночи (#781); ещё два красных теста на Windows —
`invocation_protocol_round_trips_all_four_strict_requests_and_closed_responses`
и `truncated_handshake_transport_closes_without_a_protocol_response` из
`daemon/mod.rs` — тесты v3, красные там и до перехода (прогон 34052206875
от 06.09), уходят вместе с v3 на шаге E.

**Выкидка #780 из очереди (run 34155030958) и настоящая причина.**
Приёмочный корпус (`ci-medium`): пятнадцать из двадцати одного сценария
`unica.docs` получили отказ «daemon invocation receipt is still pending at
the frontend cutoff» вместо `ok | provider | task`; на `main` тот же случай
выглядел как «protocol-v5 deadline expired during connect» и проходил корпус
только благодаря подстроке «deadline expired» — класс `provider`. Первая
правка в #780 — бюджет daemon считать после установленного соединения (тест
`daemon_budget_is_what_remains_of_the_frontend_budget_once_the_connection_stands`),
а квитанции, чей срок лежит за окном восстановления, отвечать тем же
закрытым отказом, что потерянной отправке; она верна, но корпус ей не
починить. Опыт на живом рантайме v5 в потоке (`ScriptedService` с задержкой
9 с, класс не known-long): ответ приходит на 7,13 с и это отказ, Task нет.
Причина в самом daemon: рантайм v5 не владеет cutoff. Замысел ledger
(«Cutoff и связь с TaskStore») требует, чтобы deadline owner к седьмой
секунде durable переводил `Reserved` в `TaskPromisedUnbound` либо
`TaskHandoffActorBound` и отвечал Task в прежние 125 мс, но в production
`promise_task_unbound`/`begin_bound_task_handoff` вызывались только
сценарным бегунком контракта: `receipt_scenario_v5.rs` на `AdvanceMonotonic`
сам продвигал квитанцию и сам сочинял ответ на submit, а
`capture_missing_submit_writer_after_reserve` записывал «writer path
unavailable» как улику. Inline-исполнение шло синхронно в потоке сессии,
daemon отвечал только по завершении работы: чтение дольше семи секунд
(docs через медленного провайдера, первый захват большой конфигурации на
BSP) для хоста было отказом, а поздняя Direct-публикация под истёкшим
сроком операции — вероятная причина `store_failed` на S219 в push-прогоне
`main` 34159657259 (просроченный ticket актора — латч fail-stop, следующий
submit получает `store_failed`, daemon перезапускается). Правка
(`runtime_v5.rs`): `drive_inline_invocation` — prepare и execute идут в
рабочем потоке, поток сессии владеет cutoff по `remaining_handoff_budget`
той же `InvocationResponseDeadline`; исход до cutoff — Direct как прежде
(команде ledger оставлен запас сериализации, а не истёкший срок операции);
на самом cutoff — `commit_cutoff_handoff`: тот же write-ahead intent,
exact TaskStore record, `TaskBound` и `start_bound_task`, что у known-long,
регистрация уже идущего cancellation token, ответ — снимок Working;
единственная попытка продолжается и публикует терминал в Task сама.
Гонка «исход пришёл, пока handoff коммитится» решена общим слотом с
условной переменной: исход после коммита публикуется в Task сразу, ответ —
терминальный снимок. Под сценарным бегунком (`scenario_control`) владельцем
cutoff остаётся бегунок — контракт из 65 тестов не меняется; доказательство
production-владельца — тест на живом рантайме
`live_daemon_hands_inline_work_over_the_cutoff_to_a_task_the_same_attempt_completes`
(Task на седьмой секунде, терминал Task — исход той же попытки,
`executions == 1`), корпус на CI и оценка BSP. Владение cutoff закреплено
решением `DEC.2026-09-08.DAEMON-V5-CUTOFF-OWNER`: оно берёт
`INV.APP.DAEMON-INVOCATION-HANDOFF` и добавляет к его проверкам тест на
живом рантайме — до него инвариант седьмой секунды держался только на
сценарном бегунке. Не покрыт cutoff до
`Begun` (валидация или admission дольше семи секунд — `TaskPromisedUnbound`
с двухсекундным grace по замыслу): в production это редкий путь, он
остаётся за сценарным бегунком и записан в открытые вопросы.

**Шаг E, план по инвентаризации после C.** Снаружи ядра v3 ссылки остались
в четырёх файлах: пробы «v5 отвергает v3/v4» в `receipt_scenario_v5.rs`
(кадры `protocol_v3`), ветка V3 тестового входа в `interfaces/daemon.rs`,
`production_v3` в `identity.rs`, один тест `runtime_v5.rs`. Внутри:
`client.rs` целиком; в `protocol.rs` общим остаётся только
`InvocationRequest` (его принимает общий предметный сервис); в `server.rs`
цикл v3 (`run_daemon`, `handle_connection`, `DaemonInvocationRuntime`,
реестр lease v3) и пятнадцать тестов на этом цикле, среди них свидетельства
реестра — `hidden_v13_logical_lease_survives…`, `canonical_refusals_answer…`,
`daemon_shared_delivery…`, `v13_daemon_rejects_unproved_edt…`,
`production_v3_daemon_configuration_executes_useful_modes…` и другие; их
переписывать на рантайм v5 под теми же именами, чтобы правила реестра не
трогать; в `daemon/mod.rs` 52 теста клиента v3 — снимаются, свидетельство
`DEC.2026-08-25.LOGICAL-READ-CORE-SLICE` переадресуется (поле `realized`
записываемое); хранилища v3 — `FileInvocationStore`, `invocation_store_actor`,
исполнитель в `application/invocation.rs` — снимаются, общие константы и
`normalized_arguments_hash` остаются. Снятие идёт своим решением: проверка
`INV.APP.V13-USEFUL-PARTIAL-MODES` меняет имя, а единственной допустимой
identity становится production v5 — process-тест двух несовместимых
identity теряет предмет.

**Шаг E1 — тесты `server.rs` сняты с обвязки v3.** Тридцать один тест на
`DaemonInvocationRuntime` (исполнитель v3) переведён под теми же именами
на две обвязки v5: Direct-путь — общий `V5CanonicalInvocationRuntime`
(`direct_v5`: bind → prepare → execute в потоке теста, ровно тот путь,
которым daemon идёт до cutoff; известно-долгий класс — не Direct-исход),
Task-путь — живой рантайм v5 в потоке (`LiveV5Daemon`: тест держит
канонический рантайм через `with_canonical_runtime_for_test` и наблюдает
реестры actor и capability, submit/get/wait/cancel идут по проводу v5).
У канонического рантайма появились тестовые ручки: реестр actor с
политикой (`with_workspace_actors_for_test`), сервис runtime-джобов
(`with_runtime_service_for_test` — в production он и у v3 не подключался),
захват срока по часам рантайма. Свидетельства реестра сохранили имена:
общая доставка между worktree (`realized`
EXACT-SHARED-DELIVERY-SLICE), обязательства долгой работы
(`daemon_exact_long_work_ownership_contract` — три подпроцессных
обязательства на live-рантайме), реестр actor
(`daemon_workspace_actor_admission…`: ёмкость доказывается удержанием
bound-invocation, а не Task), финальность выбора источников
(`restart_request_does_not_claim_noncooperative_actor…` — у v5 отмена
только просьба к идущей попытке: actor остаётся у неё до возврата, что и
утверждает правило). Ушли только помощники v3: `submit_at_receipt`,
`ownership_contract_runtime`, `DelayedPrepareService`
(`daemon_receipt_deadline_is_not_replenished_after_delayed_prepare` звал
лишь тест v3 из `mod.rs`), `UnavailableCancelStore`. До шагов E2/E3 в
`server.rs` остаются `typed_executor_errors_map_to_closed_protocol_codes…`
(таблица кодов v3) и `working_task_recovery_is_resume_unsupported…`
(восстановление хранилища v3, проверка
`INV.APP.RETAINED-SOURCE-SELECTION-FINALITY` переадресуется решением E).

**Шаг E2 — v3 снят: клиент, цикл сервера, протокол, identity.** Удалены
`client.rs` целиком, цикл v3 и исполнитель `DaemonInvocationRuntime` в
`server.rs` (с реестром lease, писателями ответов и тестовыми паузами),
`DaemonServerConfig` лишился ручек хранилища и бюджета сверки; из
`protocol.rs` остался только `InvocationRequest`; в `identity.rs` нет
`DaemonProtocolIdentity` и `production_v3` — каталог состояния любой
core identity `daemon-p5-…`, `DAEMON_PROTOCOL_VERSION` один; вход
`--daemon` (`interfaces/daemon.rs`) поднимает только рантайм v5, тестовый
вход — только `V5DaemonProcessOwner`; `protocol_v5` проверяет точную
production identity, а не «класс протокола». В `daemon/mod.rs` из 52 тестов
v3 остались и переведены на v5 те, у которых есть предмет: каталог
состояния и identity, аудит фасада `server` (syn), граница предметного
сервиса, `injected_hidden_v13_service_executes_real_view_and_find…`
(`realized` LOGICAL-READ-CORE-SLICE — на `LiveV5Daemon`, теперь общей
обвязке `pub(crate)` из `server.rs`), actor-bound чтение и публикация
после подмены корня/ревизии (staged-байты не утекают в терминал v5),
`daemon_executes_one_canonical_invocation_and_poll_cancel_never_relaunches_it`
(`realized` ROUTING-SLICE: один вызов — одно исполнение, cancel и
перезапуск daemon ничего не переисполняют). Процесс-тест двух
несовместимых identity удалён вместе с предметом; таблица кодов v3 и
восстановление хранилища v3 — тоже, а
`working_task_recovery_is_resume_unsupported…` (проверка
`INV.APP.RETAINED-SOURCE-SELECTION-FINALITY`) доказывается на хранилище v5:
Working после смерти процесса терминализируется как `OutcomeUncertain` без
domain-вызова. Проба контракта «v5 отвергает v3» держит кадры v3 как
рукописные байты (`v3_wire` в бегунке), identity v3 — литерал digest-а.
Реестр: решение `DEC.2026-09-08.DAEMON-V3-RETIREMENT` берёт
`INV.APP.V13-USEFUL-PARTIAL-MODES` (текст и имя проверки без «v3»).
Находка: страж неизменности не даёт переадресовать `realized` решения
кроме случая «агрегатор раскрыт в составляющие» — свидетельства снятых
тестов не переадресуются, а воссоздаются под теми же именами на v5.
Остаток для E3: `application/invocation.rs` (исполнитель v3),
`task_store.rs`, `invocation_store_actor.rs`, v3-часть
`invocation_store.rs`.

**Шаг D закрыт (run 34164918273, `main` = 643678b7 с production-владельцем
cutoff).** Полный контур `profile: main` зелёный: сборка инструментов,
пакет, проба bootstrap и дым упакованного MCP на трёх ОС, оценка на BSP
3.2.1.446 — `passed`, blocking failures 0; четыре сценария, отпадавшие на
7,2 с, теперь проходят за 8,6–9,4 с: daemon отдаёт Task на седьмой секунде,
оценка добирает результат через `unica.task.result`. Push-прогоны `main`
после #782, #783 зелёные, корпус `ci-medium` — через очередь.

**Шаг E3 — исполнитель и хранилища v3 сняты.** Удалены исполнитель v3
(`InvocationExecutor`, `PreparedDaemonInvocation`, `LiveInvocation`,
сверка терминала, состояния `Invocation`) и legacy-адаптер
`DomainResult → OperationResult` из `application/invocation.rs` — в модуле
остались бюджет ответа (`InvocationResponseDeadline`, окно handoff, запас
сериализации, бюджет сверки) и `normalized_arguments_hash`;
`infrastructure/task_store.rs` и `application/invocation_store_actor.rs`
целиком; из `application/invocation_store.rs` — записи, переходы, ошибки и
trait хранилища v3, остались `EpochMillisClock` (и `SystemEpochMillisClock`
рядом с ним), `ToolIdentity`, `SafeFailureReason` и пределы canonical result;
из `domain/invocation.rs` — `TaskSnapshot`, `InvocationOutcome` и
resume-дескрипторы v3. Доказательство «бюджет операции логического чтения
переживает handoff и завершается один раз» (два вызывающих теста в
`v13_service.rs` и `v13_read/tests.rs`) переведено на живой рантайм v5 с
управляемыми часами: седьмая секунда наступает по часам daemon, submit
получает Task, отпущенная попытка завершает его — `executions == 1`.
Критерий шага E выполнен: `grep` не находит `unica-daemon-jsonl-3`,
`DaemonOwner`, `protocol::DaemonTaskSnapshot` в коде (identity v3 живёт
только литералом в пробе контракта и в тесте identity), реестр и стражи
зелёные. Записка переведена в `approved`: замысел исполнен и доказан.

## Открытые вопросы

- Cutoff до `Begun`: production deadline owner покрывает prepare и execute;
  валидацию и admission дольше семи секунд (`TaskPromisedUnbound`,
  двухсекундный grace unbound-пайплайна по замыслу ledger) по-прежнему
  продвигает только сценарный бегунок. Когда бегунок перестанет подменять
  владельца и контракт станет доказывать production-путь — отдельный шаг.
- Сколько стоит вторая компиляция с признаком в очереди: если больше
  трёх минут, контракт ledger переезжает в отдельную джобу матрицы.
