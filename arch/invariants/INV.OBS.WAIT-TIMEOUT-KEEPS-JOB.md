---
id: INV.OBS.WAIT-TIMEOUT-KEEPS-JOB
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/runtime_jobs.rs::caller_wait_timeout_does_not_stop_the_active_job
scope: [app, product]
---

# Срок ожидателя не останавливает наблюдаемое задание

Истечение срока вызывающей стороны прекращает только ожидание: активная запись
задания продолжает выполняться и остаётся доступной для последующего опроса.
