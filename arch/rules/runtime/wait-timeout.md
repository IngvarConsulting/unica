---
id: INV.OBS.WAIT-TIMEOUT-KEEPS-JOB
check:
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::caller_wait_timeout_does_not_stop_the_active_job
---

# Истечение ожидания не отменяет задание

Если срок ожидания истёк, сервис возвращает текущее состояние задания
с признаком `wait_timed_out`. Само задание продолжает работу: его состояние
можно запросить снова и получить результат после завершения.
