- Date: `2026-09-08`
- Status: `draft`
- Decision: `none` — записка ведёт к трём решениям: крючки тестовой поддержки (шаг F2), владелец срока во всех фазах (F3) и бегунок-наблюдатель (F4); каждое заводится своим шагом, до него реестр не меняется

# Долг рантайма v5: владелец срока, крючки и бегунок-наблюдатель

## Задача

Перевод production daemon на протокол v5 закрыт
([2026-09-07-daemon-v5-production-cutover-design.md](2026-09-07-daemon-v5-production-cutover-design.md),
#773–#786). Оценка v5 против v3 от 08.09.2026 назвала четыре долга, которые
остались после переключения и которые пользователь решил закрыть до любой
работы над скоростью:

1. **Бегунок контракта подменяет production-переходы.** Контракт ledger из
   65 тестов проходит потому, что `receipt_scenario_v5.rs` на
   `AdvanceMonotonic` сам продвигает квитанцию на cutoff, сам сочиняет ответ
   на submit, сам взводит двухсекундный grace и изображает смерть процесса,
   сам терминализирует упавшие квитанции вместо преемника и подтверждает
   Direct-квитанции прямо в ledger в обход daemon.
2. **Production владеет cutoff только после `Begun`.** Решение
   `DEC.2026-09-08.DAEMON-V5-CUTOFF-OWNER` прямо оставило cutoff во время
   валидации и admission (`TaskPromisedUnbound`) и между bind actor и `Begun`
   за бегунком. Поток сессии делает bind синхронно и до `Begun` ни на что не
   реагирует.
3. **Улики недостающего перехода мертвы.** Контур W0a
   (`ProductionMissingTransitionEvidence`, пробы reachability, ветка
   `FacadeEnvelope::ProductionMissingTransition`) существовал, чтобы красный
   тест приносил доказательство, что production дошёл до границы и не смог
   сделать следующий переход. Сегодня у каждого действия сценария есть
   production-владелец, и в контур никто не попадает.
4. **Файлы рантайма и ledger неподъёмны.** `runtime_v5.rs` — 11,2 тыс. строк
   и 261 атрибут признака `receipt-ledger-test-support`, `receipt_ledger.rs` —
   16,3 тыс. строк, из них 6,4 тыс. тестов. Хуже размера то, что production
   ветвится по признаку: `#[cfg(not(feature))] let owns_cutoff = true;` —
   под признаком тестируется не тот код, который отгружается.

## Что есть сейчас

**Подмена в бегунке.** Ветка `AdvanceMonotonic` при достижении бюджета
ответа читает состояние квитанции и делает переход сама:
`Reserved/Unbound` — `promise_task_unbound_for_scenario`, ответ Task из
проекции ledger, `arm_fail_stop_deadline` на +2000 мс;
`Reserved/ActorBound` и `Reserved/Begun` —
`begin_bound_task_handoff_for_scenario`, для `Begun` ещё
`materialize_cutoff_handoff_for_test` на живом рантайме;
`TaskHandoffActorBound` с заранее сконфигурированным терминалом —
`stage_bound_handoff_terminal_for_scenario`. Ответ на submit кладётся в
отчёт с `latencyMs = budget`, а настоящий ответ daemon дочитывается позже в
`PendingSubmit::finish` и не сверяется. Достижение grace —
`fail_stop_deadline_reached` → `record_forced_process_exit`,
`record_process_exit(2000)`, снятие барьеров и остановка daemon; после
fail-stop и в `Crash` бегунок сам пишет `outcome_uncertain` для `Begun` и
`interrupted` для `TaskPromisedUnbound` (`publish_direct_terminal_for_scenario`,
`publish_receipt_backed_task_terminal_for_scenario`), хотя в production это
делает преемник в `reconcile_pre_task_startup`. ACK идёт через
`acknowledge_direct_for_scenario` и `acknowledge_without_startup` мимо
daemon; `arm_skip_next_startup_reconciliation` (восемь мест) выключает
сверку преемника. Тридцать пять `AdvanceMonotonic` в двадцати тестах
контракта зависят от этой ветки.

**Production.** `drive_inline_invocation` владеет cutoff от `Begun`:
рабочий поток делает prepare и execute, поток сессии ждёт исход или
`remaining_handoff_budget`. Но `owns_cutoff = scenario_control.is_none()`:
под бегунком production не владеет даже этим. До `Begun`
`execute_reserved_invocation` делает bind в потоке сессии синхронно; ветки
`TaskPromisedUnbound` и `TaskHandoffActorBound` после bind существуют только
потому, что бегунок мог продвинуть квитанцию, и в ветке promised поток сессии
исполняет весь вызов до конца, прежде чем ответить. Grace fail-stop в
production не реализован ни для валидации и admission после обещания, ни
для отмены некооперативной попытки — а `INV.APP.DAEMON-STORE-FAIL-STOP`
называет проверкой `noncooperative_prepare_forces_fail_stop_after_two_second_grace`,
который держится на бегунке.

**Улики.** `receipt_ledger_test_evidence.rs` — 1397 строк,
`receipt_ledger_reachability.rs` — 248, пробы и типы reachability в
`runtime_v5.rs` — около 300, диспетчер фасада в
`receipt_ledger_test_support.rs` — около 250, в контрактном файле —
`ProductionBoundary`, `EvidenceCode`, `missing_boundary` и ветка
`FUNCTIONAL RED`. Вход `run_supported_receipt_scenario_for_test` возвращает
`None` для «неподдерживаемой» формы, и только тогда фасад идёт в пробы;
`is_supported` перечисляет формы, которые ни один тест не использует.

**Признак по файлам.** `runtime_v5.rs` 261, `receipt_ledger.rs` 17,
`server.rs` 13, `receipt_ledger_actor.rs` 11, `protocol_v5.rs` 10,
`application/receipt_ledger.rs` 10, `task_store_v5.rs` 9, `client_v5.rs` 9.
В `runtime_v5.rs` четырнадцать параметров методов стоят под признаком
(`telemetry: &V5ReceiptRuntimeTelemetry`, `scenario_control: Option<&…>`).

**Константы.** Cutoff 7000 мс (`INVOCATION_HANDOFF_WINDOW`), запас
сериализации 125 мс, `TASK_RECONCILIATION_BUDGET` 2 с (он же
`CUTOFF_HANDOFF_COMMIT_BUDGET`), grace бегунка `SCENARIO_FAIL_STOP_GRACE_MS`
2000, опрос accept-цикла 10 мс, опрос cutoff 5 мс, TTL Task 3 600 000 мс у
обоих.

## Развилки и выбор

**Порядок шагов.** Улики первыми: их снятие убирает вторую ветку фасада, в
которую бегунок проваливается на любой незнакомой форме. Крючки раньше
владельца: перестройка `execute_reserved_invocation` в 1000 строк с 80
блоками `cfg` нечитаема, а тест владельца в обычном `cargo test` (без
признака) нуждается в крючке «задержи admission». Бегунок-наблюдатель после
владельца: убирать подмену можно, только когда production делает переход
сам. Разрез файлов последним: он не меняет поведения и ляжет на уже
переписанный код.

**Крючки — объект, а не `cfg`.** Рантайм получает `Arc<dyn V5RuntimeHooks>`
с пустыми методами по умолчанию; production ставит `NoHooks`, бегунок под
признаком — свою реализацию поверх нынешних `V5ReceiptRuntimeTelemetry` и
`ReceiptScenarioControl`. Виртуальный вызов пустого метода стоит наносекунды
против 30 мс на команду ledger. События (`V5ReceiptRuntimeEventKind`),
точки пауз (`ScenarioBarrierPoint`) и отказы (валидация, admission, prepare,
crash после побочного эффекта, сбой хранилища) становятся production-типами:
закрытый список мест, где рантайм инструментирован. Обобщённый параметр
`V5ReceiptRuntime<H>` отвергнут: он расползается по конфигурации сервера и
тестам ради экономии, которую нечем измерить. `cfg(not(feature))` в `src/`
запрещается совсем — у production не бывает ветки, которая есть только без
признака.

**Владелец срока — поток сессии, от `Reserved`.** Поток сессии владеет
ответом и сроком с момента резервирования; рабочий поток делает bind,
переходы ledger (`bind_reserved_actor`, `mark_reserved_begun`), prepare и
execute. Гонка между продвижением владельца и переходами рабочего
разрешается тем же, чем сегодня разрешается гонка бегунка и рантайма:
проверкой версии записи в ledger и повторным чтением состояния проигравшим.
На cutoff владелец читает состояние: `Unbound` — `promise_task_unbound`,
ответ проекцией ledger, взвод grace; `ActorBound` и `Begun` — полный
handoff (intent, link, TaskStore, `TaskBound`, start), ответ снимком
TaskStore, рабочий поток после своего шага перечитывает квитанцию и
доводит попытку в Task. Если на `Begun` исход рабочего уже лежит в слоте,
владелец после intent делает durable staging терминала до создания
TaskStore — production реализует staged-путь, который контракт уже
описывает, а преемник уже умеет доигрывать. Отвергнуто: оставить владельцем
бегунок и объявить контракт «моделью» — тогда 65 тестов не доказывают
ничего о production.

**Сторож grace.** Две секунды после обещания без bind actor и две секунды
после отмены попытки без терминала — fail-stop: listener закрыт, запрошен
перезапуск, процесс умирает, преемник терминализирует. Реализация — список
взведённых сроков на часах рантайма, который accept-цикл проверяет каждые
10 мс; поддельные часы бегунка двигают его так же, как настоящие. Риск
назван прямо: инструмент, который дольше двух секунд не смотрит на токен
отмены, убьёт daemon. Это не новая политика, а правило
`INV.APP.DAEMON-STORE-FAIL-STOP`, у которого до сих пор не было
production-реализации.

**Бегунок только наблюдает.** Ответ на submit приходит по проводу из потока
клиента, отчёт не сочиняется. Crash и fail-stop заканчиваются остановкой
daemon; терминал упавшей квитанции ставит преемник на `Restart`. ACK идёт по
проводу к живому или поднятому daemon; `skip_next_startup_reconciliation`
снимается. Посев начального состояния (`seed_*_for_test`, `inject_*_for_test`
на хранилищах) остаётся: это конструирование мира до сценария, а не подмена
перехода живого вызова. Страж выводит множество разрешённых методов из
трейта порта ledger и TaskStore: бегунку доступны только методы чтения.

**Разрез.** Тесты — в `tests.rs` дочерних модулей; из `runtime_v5.rs`
выходят `task_projection.rs`, `sessions.rs` (accept-цикл, lease, сроки
запросов), `hooks.rs`, `scenario_probes.rs` (методы `_for_test` под
признаком); `receipt_ledger.rs` становится каталогом с `records.rs`,
`catalog.rs`, `store.rs`. Страж границы кодека получает новые пути; правил
это не меняет.

## Шаги

- **F1 — снять улики.** Удалить `receipt_ledger_test_evidence.rs`,
  `receipt_ledger_reachability.rs`, пробы и типы reachability в
  `runtime_v5.rs` и `protocol_v5.rs`, диспетчер проб в фасаде, `is_supported`;
  в контракте — ветку `ProductionMissingTransition` и её типы. Незнакомая
  форма сценария — ошибка бегунка, а не улика. Реестр не трогается.
- **F2 — крючки.** `runtime_v5/hooks.rs` с трейтом, production-типами
  событий, точек пауз и отказов; замена всех `cfg`-блоков в теле рантайма и
  параметров под признаком на вызовы крючков; страж
  `tests/ci/test_receipt_ledger_test_support_boundary.py`: атрибут признака
  висит только на элементах (mod, use, fn, struct, enum, impl, поле), никогда
  на выражении или операторе; `cfg(not(feature))` запрещён. Решение
  `DEC.2026-09-08.V5-RUNTIME-HOOKS`, правило
  `INV.TEST.LEDGER-SUPPORT-GATES-ITEMS`.
- **F3 — владелец срока.** Срок захватывается при резервировании и
  передаётся в bind; `drive_reserved_invocation` со слотом по фазам; сторож
  grace; staging на `Begun`; бегунок перестаёт продвигать квитанцию на
  `AdvanceMonotonic`. Решение `DEC.2026-09-08.DAEMON-V5-DEADLINE-OWNER`
  замещает `DEC.2026-09-08.DAEMON-V5-CUTOFF-OWNER`, принимает
  `INV.APP.DAEMON-INVOCATION-HANDOFF` (переход владельца обязателен: старое
  решение стало superseded); `INV.APP.DAEMON-STORE-FAIL-STOP` остаётся за
  cutover до F4. Свидетельство — тест на живом рантайме без признака: занятый
  вызов отдаёт Task на седьмой секунде, попытка завершается в него один раз.
- **F4 — бегунок-наблюдатель.** Снять писателей `*_for_scenario`,
  `materialize_cutoff_handoff_for_test`, `staged_handoff`, `bound_task`,
  `configured_precomputed_terminal`, `arm_fail_stop_deadline`,
  `skip_next_startup_reconciliation`, `acknowledge_without_startup`; страж
  `tests/ci/test_receipt_harness_boundary.py`. Решение
  `DEC.2026-09-08.LEDGER-HARNESS-OBSERVES-ONLY`, правило
  `INV.TEST.LEDGER-HARNESS-OBSERVES`.
- **F5 — разрез файлов.** Без решения.

Каждый шаг — свой PR через очередь; правки Rust идут с `cargo fmt --check` и
`clippy --all-features -D warnings`, контракт ledger — локально целиком до
push.

## Критерии готовности

- В `receipt_scenario_v5.rs` нет вызовов `promise_task_unbound`,
  `begin_bound_task_handoff`, `stage_bound_task_handoff_terminal`,
  `publish_direct_terminal`, `publish_receipt_backed_task_terminal`,
  `acknowledge_direct`; страж F4 зелёный.
- В `runtime_v5.rs` не больше двух атрибутов признака (объявление модуля и
  реэкспорт); `rg 'cfg\(not\(feature = "receipt-ledger-test-support"' crates`
  пуст; страж F2 зелёный.
- Контракт ledger — 65 тестов — зелёный на трёх ОС через production-владельца;
  корпус приёмки зелёный; тест владельца до `Begun` зелёный без признака.
- Ни один файл рантайма и ledger не длиннее 8 тыс. строк.
- Direct-вызов с нулевой работой не медленнее 208 мс p50 на этой машине
  после F2 — крючки не должны стоить ничего измеримого.

## Реестр

| Шаг | Решение | Правило | Свидетельство |
| --- | --- | --- | --- |
| F1 | нет | нет | — |
| F2 | `DEC.2026-09-08.V5-RUNTIME-HOOKS` | `INV.TEST.LEDGER-SUPPORT-GATES-ITEMS` (новое) | `tests/ci/test_receipt_ledger_test_support_boundary.py` |
| F3 | `DEC.2026-09-08.DAEMON-V5-DEADLINE-OWNER`, замещает `DEC.2026-09-08.DAEMON-V5-CUTOFF-OWNER` | `INV.APP.DAEMON-INVOCATION-HANDOFF` (переходит к новому решению, получает production-проверку) | тест владельца на живом рантайме без признака |
| F4 | `DEC.2026-09-08.LEDGER-HARNESS-OBSERVES-ONLY` | `INV.TEST.LEDGER-HARNESS-OBSERVES` (новое) | `tests/ci/test_receipt_harness_boundary.py` |
| F5 | нет | нет | — |

Даты решений — по дню их появления в дереве; в записке они названы
заранее ради связности и правятся при заведении.

## Чего в плане нет

- Ускорения v5: по замерам 08.09 (208 мс p50 на Direct-вызов с нулевой
  работой против 154 мс у v3, из них около 175 мс — пять durable-команд
  ledger) оптимизация не нужна; ночной замер как ворота — отдельная тема.
- Новых действий сценария и новых состояний ledger.
- Изменений провода v5 и поверхности `unica.*`.

## Ход

**Шаг F1 — улики сняты.** Удалены `receipt_ledger_test_evidence.rs` и
`receipt_ledger_reachability.rs`, пробы reachability и типы
`V5ExecutorReachability`/`V5TaskProjectionReachability` в `runtime_v5.rs`
вместе с полем `evidence_capture` и захватом улик в обработчике сессии,
проба строгого конверта в `protocol_v5.rs`, диспетчер проб и дублирующие
типы входа в фасаде `receipt_ledger_test_support.rs` (фасад стал одним
вызовом бегунка), `is_supported` и `has_supported_shape` в бегунке, ветка
`ProductionMissingTransition` с типами `ProductionBoundary`, `EvidenceCode`,
`ActionKind` в контрактном файле — около 3,8 тыс. строк. Бегунок отвечает
ошибкой на любую незнакомую форму: пятнадцать выходов `Ok(None)`, которые
раньше молча передавали сценарий в пробы, стали `Err` с именем формы.
Попутная находка: тесты lib под признаком `receipt-ledger-test-support`
не входят ни в одну полосу конвейера (контракт запускается только целью
`daemon_receipt_ledger`), и три из них были красными на `main`: устаревший
список публичных функций фасада (исправлен здесь), страж «бегунок не
ссылается на `ReceiptLedgerStore`» (ссылка в `rotate_receipt_generation`) и
`late_cancel_preserves_the_committed_actor_bound_task_terminal` — тест ждёт,
что отмена после перезапуска увидит терминал `interrupted`, поставленный
сверкой преемника, а бегунок на отмене без живого daemon взводит
`skip_next_startup_reconciliation`, и отмена застаёт квитанцию живой. Тест
прав, ручка — подмена; оба остаются красными до шага F4, где страж на `syn`
заменяется стражем в `tests/ci`, а ручка снимается.

**Шаг F2 — крючки.** `runtime_v5/hooks.rs`: трейт `V5RuntimeHooks` с
пустыми методами по умолчанию, `NoHooks` для production, production-типы
`V5ReceiptRuntimeEventKind` (сорок событий), `V5Stage`, `V5PausePoint`
(шестнадцать точек, включая новую `BeforeRetirementSnapshot` вместо
барьера под `cfg(all(test, feature))`), `V5AdmissionRejection`,
`V5StoreFaultPoint`. Рантайм держит `hooks: Arc<dyn V5RuntimeHooks>` и
берёт его из `DaemonServerConfig` (`runtime_hooks`, поля переопределений
часов и пропуска сверки теперь безусловные). Телеметрия, аренды и
писатели `*_for_scenario` переехали в
`runtime_v5/receipt_scenario_v5/scenario_hooks.rs` вместе с
`ScenarioHooks` — реализацией трейта поверх телеметрии и сценарного
управления; методы `_for_test` рантайма и проекции — в
`receipt_scenario_v5/scenario_probes.rs` (дочерний модуль бегунка, доступ к
внутренностям сохранён). В `runtime_v5.rs` осталось два атрибута признака
(объявление модуля и реэкспорт) против 261; `cfg(not(feature))` в `src/`
нет. Слоты впрыска сбоев хранилищ безусловны: `arm_receipt_row_directory_sync_fault`
для всего процесса плюс поток-локальный слот для unit-тестов,
`inject_next_publication_failure` у TaskStore; команды наблюдателя
`SnapshotCatalog` и `RotateGenerationForTest` актора и порт ledger
безусловны — их шлёт только наблюдатель. Два выравнивания production с
тем, что раньше делал только впрыснутый отказ: `WorkspaceRegistryFailed`
на admission переводит daemon в fail-stop, а ответ идёт вариантом
`JsonFailStop`/`PreparedFailStop`; тест на гонку retirement снят с
признака и идёт в обычной полосе. Страж
`scripts/ci/check-receipt-ledger-test-support-boundary.py` с
`tests/ci/test_receipt_ledger_test_support_boundary.py`: признак только на
элементах, `not(feature)` запрещён; решение
`DEC.2026-09-08.V5-RUNTIME-HOOKS`, правило
`INV.TEST.LEDGER-SUPPORT-GATES-ITEMS`.

**Шаг F3 — владелец срока.** Поток сессии владеет ответом и cutoff; рабочий
поток `unica-v5-invocation-pipeline` гонит конвейер (`execute_reserved_invocation`
запускает воркер и слушает его отчёт через `PipelineSlot`). До `Begun` владелец
на седьмой секунде продвигает квитанцию в `promote_at_cutoff` по фазе: `Unbound`
в обещанный Task с заряженным сторожем `FailStopWatchdogs`, `ActorBound` в
handoff, `Begun` в handoff и — если наблюдатель не держит `BeforeTaskStoreCreate`
— материализует Task тут же и кладёт `(record, bound)` в слот. Воркер,
проигравший гонку, перечитывает квитанцию и продолжает единственную попытку
через `continue_promised`/`continue_handoff`/`continue_into_bound_task`/
`continue_promoted_begun_handoff`; на паузе `PrepareEntered` он забирает
материализованный Task из слота и исполняет его через `drive_prepared_bound_task`
(выделен из хвоста `continue_into_bound_task`; known-long уходит в поток
исполнения, direct продолжается на этом же потоке). Сторожа проверяет
accept-loop на часах канонического рантайма; fail-stop, освобождающий authority,
сперва `join_task_executions()`, чтобы отсоединённое продолжение не удержало
receipt-authority живой. Обвязка больше не продвигает: `PendingSubmit::await_response`
и `await_handoff_intent` ждут ответ рантайма, `quiesce_promoted_continuation`
(в Checkpoint/WaitForEvent/финале) ждёт, пока продолжение осядет (по счётчику
`promoted_continuations`), и после fail-stop освобождает застрявший endpoint —
иначе `connect_or_spawn` спавнит настоящий процесс. Решение
`DEC.2026-09-08.DAEMON-V5-DEADLINE-OWNER` замещает
`DEC.2026-09-08.DAEMON-V5-CUTOFF-OWNER` и принимает
`INV.APP.DAEMON-INVOCATION-HANDOFF`; проверка — `owner_promotes_a_slow_inline_attempt_to_a_task_at_the_cutoff`,
живой рантайм без признака. Все 56 тестов контракта зелёные; два теста lib под
признаком (`scenario_owner_helpers_cannot_bypass_the_actor_store_boundary`,
`late_cancel_preserves_the_committed_actor_bound_task_terminal`) на этом шаге
ещё красные — их закрывает F4.

**Шаг F4 — бегунок-наблюдатель.** Диспетчер действий
`run_supported_receipt_scenario_for_test` больше не делает ни одного
durable-перехода. Сняты: подтверждение мимо демона
(`acknowledge_without_startup` — теперь `exchange_once` по проводу, а при живом
демоне `acknowledge_on_live_daemon`), терминализация за упавший процесс на трёх
путях (`Restart`, крэши `ReservedBegun` и `TaskPromisedUnbound` — крэш объявляет
выход процесса, как соседние точки, и квитанцию сверяет старт преемника; ответа
упавший submit не отдаёт), крючок `bound_task_override` (его работу делает
`PipelineSlot::take_owner_materialized` из F3) и крючок `staged_handoff` (рантайм
перечитывает устаревшую за паузу квитанцию сам —
`reread_handoff_after_pause`). `rotate_receipt_generation` ходит через актора, а
не открывает `ReceiptLedgerStore`.

Оставшиеся записи вынесены в помощников, чьё имя называет владельца:
`seed_receipt_state`, `seed_staged_cross_store_terminal`,
`seed_direct_probe_terminal` (засев живым рантаймом — решение пользователя
08.09.2026 «живой владелец для посева»), `run_direct_load` (нагрузка),
`acknowledge_on_retained_actor` (слушателя нет, актор — единственный владелец),
`corrupt_receipt_identity_index`, `rotate_receipt_generation` и
`stage_terminal_as_second_owner`. Последний — единственная запись, которую
пробовали снять и вернули: терминал ставится поверх попытки, припаркованной на
`BeforeTaskStoreCreate`, то есть это чередование **второго** владельца, а не
переход за наблюдаемую попытку; без него
`oversized_result_and_uncertain_store_commit_fail_closed` не доходит до
`BoundHandoffTerminalStaged`. Отсюда и формулировка правила: бегунок не пишет за
наблюдаемую попытку, но играет других владельцев по описи.

`skip_next_startup_reconciliation` осталась фикстурой ровно там, где обвязка
поднимает своего демона под один запрос, которого в production уже поднят
(воротные операции, отмена, чтение Task, `start_blocked_submit`,
подтверждение); разводит её `lazy_session`, который бегунок раньше
игнорировал: сессия, владеющая попыткой, получает посев, ленивая встречает
терминал преемника. После этого
`late_cancel_preserves_the_committed_actor_bound_task_terminal` зелёный.

Страж `scripts/ci/check-receipt-harness-boundary.py` с
`tests/ci/test_receipt_harness_boundary.py` держит опись «владелец → переход»,
запрет записи в диспетчере и запрет имён `ReceiptLedgerStore`/`ReceiptLedgerPort`.
Страж на `syn` `scenario_owner_helpers_cannot_bypass_the_actor_store_boundary`
снят — он проверял часть того же и жил в полосе, которую не гоняет ни один ярус.
Решение `DEC.2026-09-08.LEDGER-HARNESS-OBSERVES-ONLY`, правило
`INV.TEST.LEDGER-HARNESS-OBSERVES`. Контракт 56/56, полоса lib под признаком
зелёная, `tests/ci` — 866, `tests/arch` — 127.

**Шаг F5 — разрез файлов.** Три файла держали почти сорок тысяч строк; теперь
ни один файл рантайма и ledger не длиннее восьми тысяч, самый большой — 7782.

`runtime_v5.rs` (9168) перерос порог только из-за тестов: production — 6311.
Модуль уехал в `runtime_v5/tests.rs`.

`receipt_scenario_v5.rs` (13 013) разложен по роли: `wire.rs` — типы, в которые
разбирается запрос; `control.rs` — барьеры, фикстуры и наблюдаемое состояние;
`dispatch.rs` — диспетчер действий; `tests.rs` — юнит-тесты. В корне остались
помощники-владельцы (засев, нагрузка, наблюдения) — 7286.

`receipt_ledger.rs` (16 285) разложен на `tests.rs`, `validation.rs` (проверка
активной записи), `catalog.rs` (вставка, удаление, замена), `records.rs`
(сборка durable-записей), `encoding.rs` (имена, байты и единственный
сериализатор активных записей) и `port.rs` (адаптер `ReceiptLedgerPort`). В
корне остался `impl ReceiptLedgerStore` — 7777.

Пути модулей везде прежние, поэтому выражения отбора nextest выбирают те же
тесты; переписан один терм — модуль тестов рантайма стал файловым, и генератор
`scripts/ci/size-filters.py` пишет для него другую форму.

Двух стражей пришлось научить новой раскладке. Страж границы бегунка читает
теперь всю обвязку: корень и водителей (`control`, `dispatch`, `wire`)
проверяет, `scenario_hooks`/`scenario_probes`/`tests` освобождает с названной
причиной, а незнакомый `.rs` рядом с обвязкой роняет — классифицировать его
должен человек. Страж кодека узнал, что хранилище — это модуль-каталог, а не
один файл (правила пути держатся на корне и всех детях), и что
`#[cfg(test)] mod name;` делает файл целиком тестовым: без этого вынос тестов
превратил бы их в production в глазах стража.

**Ревью #804 нашло у стража F4 три дефекта, и один был не гипотетическим.**
Опись писателей не знала `reserve` и пакетных команд актора — и диспетчер
действительно писал мимо неё в двух местах (засев квитанции для фикстуры порчи
индекса и отвергаемое резервирование чужого ключа); оба вынесены в названных
владельцев. Владельца определял отступ, а не лексическая область, поэтому метод
внутри `impl` наследовал владельца от соседа сверху; теперь область держится
стеком «имя → глубина скобок», а объявление `fn имя(` вызовом не считается.
Читался один файл, тогда как `scenario_probes.rs` и `scenario_hooks.rs` были
освобождены целиком — а там девять функций пишут в ledger; сплошное
освобождение заменено описью, право открыть `ReceiptLedgerStore` мимо актора
получили четыре названные фикстуры вместо целого файла. Освобождён остался
только `tests.rs`. На каждую находку — регрессионный тест; тестов стража 15.

Вне разреза остался контрактный файл `crates/unica-coder/tests/daemon_receipt_ledger.rs`
(13 186 строк): критерий говорит про файлы рантайма и ledger, а это цель
тестов. Резать её — отдельная работа.

## Открытые вопросы

- Насколько admission в production бывает медленным: на оценке BSP bind
  занимал миллисекунды. Владелец до `Begun` делается не ради частоты, а
  потому что правило реестра его требует и контракт его доказывает.
- Staging на `Begun` добавляет одну команду ledger (около 30 мс) только когда
  исход уже есть в момент cutoff; на горячий путь это не влияет.
