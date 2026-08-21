---
id: INV.OBS.DETACHED-JOB-STATE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/runtime_jobs.rs::detached_worker_owns_the_queued_record_until_terminal_state
scope: [app, product]
---

# Отделённый worker ведёт долговременное состояние задания

Запущенное задание остаётся наблюдаемой записью от очереди до терминального
состояния, пока отделённый worker владеет его выполнением.
