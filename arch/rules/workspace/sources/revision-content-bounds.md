---
id: INV.SOURCE.REVISION-CONTENT-BOUNDS
check:
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_ambient_targeted_content_uses_retained_bounds_and_checkpoints
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_incremental_targeted_content_uses_retained_bounds_and_checkpoints
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_incremental_targeted_content_honors_mid_read_stop
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_targeted_resources_honor_cancellation_deadline_and_limits
  - crates/unica-coder/src/infrastructure/source_revision.rs::retained_file_hashing_checks_cancellation_between_bounded_chunks
  - crates/unica-coder/src/infrastructure/source_revision.rs::production_revision_scan_does_not_cap_aggregate_content_at_sixteen_gib
  - crates/unica-coder/src/infrastructure/source_revision.rs::production_revision_scan_still_refuses_byte_count_overflow
  - crates/unica-coder/src/infrastructure/source_revision.rs::production_revision_scan_does_not_cap_a_file_at_two_hundred_fifty_six_mib
---

# Ревизия читает всё учитываемое содержимое ограниченными блоками

Полный обход, проверка удерживаемых файлов и обновление только изменившихся
файлов хешируют всё учитываемое содержимое без фиксированного предела размера
одного файла или суммы байтов. Все пути используют одинаковый учёт с проверкой
переполнения; при обновлении прежний размер файла заменяется новым, а не
прибавляется к нему повторно.

Чтение идёт блоками не более 64 КиБ и проверяет срок и отмену операции между
ними. Большой файл или корпус может потребовать больше времени на полный
обход. Истечение срока, отмена или переполнение счётчика дают отказ, а не
успешную ревизию частично прочитанного дерева.
