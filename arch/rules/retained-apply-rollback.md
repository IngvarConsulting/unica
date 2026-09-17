---
id: INV.CACHE.RETAINED-APPLY-REVISION-ROLLBACK
check:
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_failures_restore_source_cache_and_revision_machine_exactly
---

# Откат retained apply охватывает источники, кеш и ревизию

Ошибка публикации retained apply восстанавливает исходные байты источников
и eager cache, revision record, `state.json` и состояние revision machine.
Проверка вводит отказ при записи источника, eager metadata, revision record,
`state.json` и после всех postimages. Восстановление после аварийного завершения
процесса и отмена операции находятся за границей этой проверки.
