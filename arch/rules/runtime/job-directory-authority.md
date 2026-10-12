---
id: INV.APP.RUNTIME-JOB-DIRECTORY-AUTHORITY
check:
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::ordinary_poll_and_finish_never_follow_a_replaced_job_directory
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::attached_process_poll_and_finish_never_follow_a_replaced_job_directory
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::full_rebuild_fallback_never_follows_a_replaced_job_directory
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::record_publication_and_lock_release_never_follow_a_replaced_job_directory
---

# Запущенное задание остаётся связано со своим исходным каталогом

До запуска процесса или принятия уже запущенного процесса Unica удерживает
исходный каталог задания. Опрос, запись состояния и журналов, завершение
и повторная полная сборка используют этот же каталог.

Другой каталог, появившийся под тем же именем `jobs/<id>`, не получает записи
задания; его блокировка остаётся нетронутой. Если принадлежность каталога
не подтверждена, процесс остаётся под наблюдением по
[правилу освобождения ресурса](resource-release-proof.md).

Правило охватывает только службу, которая владеет процессом задания. Вне его
области остаются:

- операции до запуска процесса: отмена и отказ задания в очереди
  (`cancel_enqueued_before_start_guarded`, `cancel_queued_job_guarded`,
  `fail_queued_job_guarded`);
- чтение из других процессов (`status_at`, `logs_at`, `list_at`, `wait_at`);
- запрос отмены `request_cancel_at`, который пишет маркер в каталог по имени;
- восстановление после устаревшего пульса `recover_stale_active_locked`:
  оно пишет `Lost` в каталог по имени и сохраняет `active.lock`, если
  процесс мог запуститься.

Если каталог `jobs/<id>` подменён и не возвращён, наблюдение не может
подтвердить принадлежность и держит `active.lock` бесконечно. Восстановление
ручное. Можно вернуть исходный каталог под имя `jobs/<id>`: пока наблюдающий
процесс Unica жив, он завершит задание и снимет блокировку сам. Если исходного каталога нет, убедитесь, что процесс
задания и его потомки завершены, и удалите `jobs/active.lock`.
