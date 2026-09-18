---
id: INV.SOURCE.REVISION-CONTENT-BOUNDS
check:
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_ambient_targeted_content_uses_retained_bounds_and_checkpoints
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_incremental_targeted_content_uses_retained_bounds_and_checkpoints
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_incremental_targeted_content_honors_mid_read_stop
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_targeted_resources_honor_cancellation_deadline_and_limits
  - crates/unica-coder/src/infrastructure/source_revision.rs::retained_file_hashing_checks_cancellation_between_bounded_chunks
---

# Все способы чтения ревизии соблюдают одни ограничения содержимого

Полный обход, проверка удерживаемых файлов и обновление только изменившихся
файлов хешируют содержимое с одинаковыми пределами на один файл и на общую
сумму байтов. При обновлении прежний размер файла заменяется новым,
а не прибавляется к нему повторно.

Чтение идёт ограниченными блоками и проверяет срок и отмену операции
между ними. Превышение размера, истечение срока или отмена дают отказ,
а не успешную ревизию частично прочитанного дерева.
