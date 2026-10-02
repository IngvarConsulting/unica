---
id: INV.WIRE.APPLY-REQUIRES-THE-FENCE
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::two_plans_on_one_revision_cannot_both_publish
  - crates/unica-coder/src/application/v13/apply.rs::planning_and_saved_plan_execution_have_disjoint_public_shapes
---

# Применение исполняет сохранённый план на неизменившихся исходниках

`unica.apply` с `at` и `ops` проверяет операции и возвращает план без записи.
Успешный план содержит `data.executionToken`. Для исполнения вызывающий
передаёт только `executionToken`: адрес, операции и ревизии берутся из
сохранённого плана. Публичная схема разделяет эти два вида вызова;
смешанные аргументы, `dryRun` и `ifRev` не допускаются.

Перед записью исполнитель проверяет сохранённые ревизии исходников.
Два плана на одной ревизии не могут оба опубликовать изменения:
после первого применения второй получает `stale_revision`, а правка
первого остаётся целой. Для нового применения нужен свежий план.
