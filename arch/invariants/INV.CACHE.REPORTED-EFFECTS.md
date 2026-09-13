---
id: INV.CACHE.REPORTED-EFFECTS
status: superseded
governs: product
decision: DEC.2026-09-03.V0-13-LEGACY-BATCH-2
check: crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_object_remove_reports_typed_cache_impact_in_preview_and_publication
scope: [app, cache, product, wire]
---

# Публичная мутация возвращает событие и влияние в своём результате

Реальный вызов `unica.meta.remove` в preview и apply возвращает типизированное
событие и списки затронутых кешей в том же сериализуемом результате. Preview
показывает будущую инвалидацию без публикации обновлений и исходных байтов.
