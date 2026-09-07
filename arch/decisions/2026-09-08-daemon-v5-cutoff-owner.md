---
id: DEC.2026-09-08.DAEMON-V5-CUTOFF-OWNER
status: active
governs: product
realized: crates/unica-coder/src/interfaces/daemon_router.rs::live_daemon_hands_inline_work_over_the_cutoff_to_a_task_the_same_attempt_completes
supersedes: []
superseded-by: null
establishes: [INV.APP.DAEMON-INVOCATION-HANDOFF]
design: docs/design/2026-09-07-daemon-v5-production-cutover-design.md
---

# Production-рантайм v5 сам владеет cutoff inline-исполнения

**Решение.** Daemon протокола v5 исполняет prepare и execute inline-класса в
рабочем потоке, а поток сессии владеет cutoff той же
`InvocationResponseDeadline`, что захвачена при bind: исход до седьмой
секунды публикуется Direct, а на самом cutoff daemon durable переводит
begun-квитанцию в handoff — тот же write-ahead intent, exact TaskStore
record, `TaskBound` и старт, что у known-long, — регистрирует уже идущий
cancellation token и отвечает снимком Working. Единственная попытка не
перезапускается и не отменяется: её исход публикуется в Task тем, кто им
владеет после коммита. Ни одна поздняя Direct-публикация не идёт под
истёкшим сроком операции.

Cutoff до `Begun` — валидация или admission дольше семи секунд,
`TaskPromisedUnbound` с grace unbound-пайплайна — production пока не
покрывает; в контракте ledger владельцем cutoff остаётся сценарный бегунок,
пока отдельный шаг не переведёт его на production-путь.

**Почему.** До этого решения инвариант седьмой секунды был доказан только
сценарным бегунком контракта: он сам продвигал квитанцию на
`AdvanceMonotonic` и сочинял ответ на submit, а production отвечал только по
завершении работы — чтение дольше семи секунд было для хоста отказом, а
поздняя публикация роняла daemon в fail-stop.

**Цена.** Ещё один поток на inline-вызов и слот с условной переменной для
гонки «исход пришёл, пока handoff коммитится»; ответ handoff несёт стоимость
exact TaskStore record внутри запаса сериализации.
