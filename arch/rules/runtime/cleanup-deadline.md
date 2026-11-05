---
id: INV.APP.RUNTIME-CLEANUP-DEADLINE
check:
  - crates/unica-process/src/runtime_job.rs::system_cleanup_passes_one_exact_deadline_to_delayed_tree_and_both_readers
  - crates/unica-process/src/runtime_job.rs::external_cancel_passes_its_bound_deadline_to_both_real_readers
---

# Очистка процесса использует общий срок ожидания

При отмене и освобождении процесса Unica выделяет общий срок на остановку
дерева процессов, ожидание выхода и завершение чтения stdout и stderr.
Каждая стадия использует оставшееся время; новый полный срок ей не выдаётся.
Ошибка остановки дерева не отменяет учёт обоих потоков вывода.
