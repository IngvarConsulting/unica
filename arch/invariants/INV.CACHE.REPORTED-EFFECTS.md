---
id: INV.CACHE.REPORTED-EFFECTS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/meta_remove_surface_tests.rs::real_public_meta_remove_reports_typed_cache_impact_in_the_same_result
scope: [app, cache, product, wire]
---

# Публичная мутация возвращает событие и влияние в своём результате

Реальный вызов `unica.meta.remove` в preview и apply возвращает типизированное
событие и списки затронутых кешей в том же сериализуемом результате. Preview
показывает будущую инвалидацию без публикации обновлений и исходных байтов.
