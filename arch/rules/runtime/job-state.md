---
id: INV.OBS.DETACHED-JOB-STATE
check:
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::detached_worker_owns_the_queued_record_until_terminal_state
---

# Состояние фонового задания сохраняется после завершения worker

Поставленное в очередь задание имеет сохранённую запись. Исполняющий его
worker обновляет эту запись до итогового состояния, включая время начала
и завершения. Другой экземпляр сервиса заданий читает тот же результат
по идентификатору задания после завершения worker.
