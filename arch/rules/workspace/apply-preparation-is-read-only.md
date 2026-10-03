---
id: INV.SOURCE.RETAINED-APPLY-WRITE-FREE
check:
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_plan_fence_is_targeted_and_binds_read_only_inputs
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_admission_and_dry_run_are_cache_tree_write_free
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::prepared_apply_dry_run_returns_projected_effect_receipt_without_any_write
---

# Подготовка и предпросмотр правок не меняют исходники и кеш

Допуск к `apply`, планирование и предпросмотр сохраняют исходные файлы,
содержимое и состав дерева кеша, включая каталог ревизий. Отсутствующие
каталоги кеша при этом не создаются.

Предпросмотр возвращает идентификатор подготовленного плана: выбранные корни,
прочитанные зависимости, исходное и ожидаемое содержимое затронутых файлов,
операции и их эффекты. Этот идентификатор подтверждает конкретный план через
`ifRev`. Получение идентификатора не требует снимка всего набора исходников
и не продвигает состояние ревизии в памяти.
