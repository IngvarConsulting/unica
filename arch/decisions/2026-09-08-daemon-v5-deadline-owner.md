---
id: DEC.2026-09-08.DAEMON-V5-DEADLINE-OWNER
status: active
governs: product
realized: crates/unica-coder/src/interfaces/daemon_router.rs::live_daemon_hands_a_failing_inline_attempt_over_the_cutoff_and_the_task_fails_once
supersedes: [DEC.2026-09-08.DAEMON-V5-CUTOFF-OWNER]
superseded-by: null
establishes: [INV.APP.DAEMON-INVOCATION-HANDOFF]
design: docs/design/2026-09-08-daemon-v5-runtime-debt-design.md
---

# Production-рантайм v5 сам владеет cutoff на всех фазах до Begun

**Решение.** Поток сессии daemon протокола v5 владеет и ответом, и cutoff
захваченной при приёме `InvocationResponseDeadline`; рабочий поток
`unica-v5-invocation-pipeline` гонит конвейер одной зарезервированной попытки —
валидацию, admission, durable-переходы, prepare и execute. До `Begun` cutoff
принадлежит владельцу: на седьмой секунде он durable продвигает квитанцию по её
фазе — `Unbound` в promised Task с grace fail-stop, `ActorBound` в handoff-intent,
`Begun` в handoff и, если наблюдатель не держит создание TaskStore,
материализует Task тут же — и отвечает снимком. Рабочий поток продолжает ту же
единственную попытку в этот Task, перечитывая durable-состояние на проигранной
гонке. С `Begun` cutoff держит inline-drive на рабочем потоке, как прежде.
Единственная попытка не перезапускается; её исход публикует тот, кто владеет ею
после коммита.

**Почему.** `DEC.2026-09-08.DAEMON-V5-CUTOFF-OWNER` перевёл в production только
cutoff на `Begun`; седьмую секунду до `Begun` — валидацию или admission дольше
семи секунд — по-прежнему изображал сценарный бегунок контракта: он сам
продвигал квитанцию на `AdvanceMonotonic` и сочинял ответ на submit. Это
решение переносит владение cutoff всех до-`Begun` фаз в рантайм, а бегунок
теперь ждёт ответ рантайма и лишь наблюдает его. Инвариант седьмой секунды
доказывается production-путём (`NoHooks`, без признака сборки).

**Цена.** Поток-владелец и рабочий поток на каждый зарезервированный вызов, слот
с условной переменной для гонки «владелец продвинул / рабочий дошёл до drive», и
перечитывание durable-состояния рабочим потоком после каждой паузы, когда за ним
наблюдают. Fail-stop, освобождающий authority, сперва присоединяет рабочие
потоки, чтобы отсоединённое продолжение не удержало receipt-authority живой.
