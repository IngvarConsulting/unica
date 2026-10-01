---
id: INV.APP.RUNTIME-RETAINED-JOB-PUBLICATION
check:
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::quarantine_release_never_removes_active_lock_from_replacement_jobs_root
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::quarantine_record_read_never_follows_replacement_after_retained_validation
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::quarantine_record_publish_never_writes_replacement_root
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::quarantine_publication_never_releases_after_same_root_job_directory_replacement
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::quarantine_post_rename_flush_failure_never_releases_active_lock
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::quarantine_post_publication_confirmation_rejects_same_root_job_directory_swap
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::quarantine_record_transition_completes_only_in_exact_retained_root
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::ownership_retained_transition_never_writes_replacement_job_directory
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::failed_activation_cleanup_never_writes_replacement_job_directory
---

# Завершение runtime записывается в исходный каталог задания

При сохранении процесса после ошибки запуска и при освобождении ресурса
после неопределённого завершения используются удерживаемые файловые
дескрипторы исходных каталогов заданий. Повторная попытка не выбирает заново
`jobs/<id>` по пути. Подменённый каталог с тем же именем и такими же байтами
`record.json` не получает ни новую запись, ни снятие `active.lock`.

Новое состояние записывается атомарной заменой файла и синхронизируется.
Перед снятием блокировки подтверждается, что опубликованный файл находится
под именем `record.json` в исходном каталоге задания. Ошибка синхронизации
или подмена до этого подтверждения сохраняет блокировку; повторная попытка
также не принимает подменённый каталог.

Гарантия относится к передаче владения после ошибки запуска и к освобождению
удерживаемого ресурса. Обычная запись при опросе состояния здесь не охвачена.
Изменение файлов извне после последнего подтверждения не исключается.
