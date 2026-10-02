---
id: INV.WIRE.APPLY-REQUIRES-THE-FENCE
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::conflicting_preview_plans_cannot_both_publish
  - crates/unica-coder/src/application/v13/apply.rs::the_fence_is_required_by_the_mode_and_the_schema_says_so
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_plan_fence_is_targeted_and_binds_read_only_inputs
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::disjoint_apply_plans_publish_without_a_global_revision_conflict
---

# Применение повторяет подтверждённый план

`unica.apply` с `dryRun: false` или без `dryRun` требует `ifRev`, полученный
из dry-run того же запроса. Предпросмотр допускается без `ifRev` или с ним.
Маркер связывает операции и аргументы, выбранные корни, прочитанные входы,
планируемые байты и эффекты. Он не является ревизией конфигурации.

Сравнение маркера выполняется после построения конкретного плана, до записи.
Изменение его входов или намерения даёт `stale_revision` и требует нового
предпросмотра. Перед публикацией и на её границах проверяются удержанные
корни и точечные исходные байты; отмена, срок и откат сохраняют свои гарантии.

Изменение непрочитанного файла не инвалидирует план. Независимые планы могут
выполняться последовательно; конфликтующие планы не перезаписывают чужие
изменения. Допуск, предпросмотр и применение не обходят исходники ради
снимка всего дерева. `view` не выдаёт маркер для применения.
