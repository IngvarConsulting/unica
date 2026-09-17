---
id: INV.SOURCE.RETAINED-APPLY-WRITE-FREE
check:
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_admission_and_dry_run_revision_observation_are_cache_tree_write_free
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::prepared_apply_dry_run_returns_projected_effect_receipt_without_any_write
---

# Подготовка и предпросмотр правок не меняют исходники и кеш

Допуск к `apply`, планирование и предпросмотр сохраняют исходные файлы,
содержимое и состав дерева кеша, включая каталог ревизий. Отсутствующие
каталоги кеша при этом не создаются.

Предпросмотр возвращает ревизию, полученную при допуске, без публикации
изменений и без продвижения состояния ревизии в памяти.
