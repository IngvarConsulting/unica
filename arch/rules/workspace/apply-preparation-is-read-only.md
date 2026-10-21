---
id: INV.SOURCE.RETAINED-APPLY-WRITE-FREE
check:
  - crates/unica-coder/src/infrastructure/workspace_actor_contract_tests.rs::apply_plan_fence_is_targeted_and_binds_read_only_inputs
  - crates/unica-coder/src/infrastructure/workspace_actor_contract_tests.rs::apply_admission_and_dry_run_are_cache_tree_write_free
  - crates/unica-coder/src/infrastructure/workspace_actor_contract_tests.rs::prepared_apply_dry_run_returns_projected_effect_receipt_without_any_write
---

# Подготовка и предпросмотр правок не меняют исходники и кеш

Допуск к `apply`, планирование и предпросмотр сохраняют исходные файлы,
содержимое и состав дерева кеша, включая каталог ревизий. Отсутствующие
каталоги кеша при этом не создаются.

Токен предпросмотра связан с конкретным планом: выбранными корнями,
прочитанными зависимостями, исходным и ожидаемым содержимым затронутых файлов,
операциями и их эффектами. Исполнение этого плана задаёт
[контракт сохранённого плана](apply-revision-fence.md).
Получение токена не требует снимка всего набора исходников и не продвигает
состояние ревизии в памяти.
