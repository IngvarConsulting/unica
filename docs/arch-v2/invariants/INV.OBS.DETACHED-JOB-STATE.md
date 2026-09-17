---
id: INV.OBS.DETACHED-JOB-STATE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/runtime_jobs.rs::runtime_job_lifecycle_and_log_bounds_are_complete
scope: [app, product]
---

# Отделённый worker ведёт долговременное состояние задания

Запущенное задание остаётся наблюдаемой записью от очереди до терминального
состояния, пока отделённый worker владеет его выполнением.
