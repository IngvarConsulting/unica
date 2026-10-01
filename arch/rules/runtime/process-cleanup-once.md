---
id: INV.APP.PROCESS-CLEANUP-ONCE
check:
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::system_process_drop_uses_no_second_window_after_cleanup_deadline_is_consumed
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::external_cancel_error_binds_system_drop_to_the_same_cleanup_deadline
---

# Освобождение объекта процесса не начинает очистку заново

Если очистка после неудачного запуска или отмены уже предпринята,
последующее освобождение объекта процесса (`Drop`) не повторяет очистку
дерева процессов и обоих потоков вывода и не выделяет новое время ожидания.

Ошибка очистки не доказывает завершение процессов и не разрешает
[освободить занятый ресурс](resource-release-proof.md).
Проверки этих двух путей выполняются на поддерживаемых Unix-системах;
они не подтверждают поведение Windows или общий срок всей очистки.
