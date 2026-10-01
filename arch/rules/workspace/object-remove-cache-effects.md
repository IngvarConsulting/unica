---
id: INV.CACHE.REPORTED-EFFECTS
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_object_remove_reports_typed_cache_impact_in_preview_and_publication
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::prepared_apply_dry_run_returns_projected_effect_receipt_without_any_write
---

# Удаление объекта сообщает о влиянии на кеш

При удалении объекта через `unica.apply` с операцией `object.remove` результат
того же вызова содержит событие изменения и список затронутых кешей.

В предпросмотре это ожидаемое влияние: исходники, кеш и сведения о ревизии
остаются прежними. После успешного применения результат описывает
опубликованные изменения.
