---
id: INV.TOKEN.CACHE-IMPACT-IN-RESULT
status: active
governs: product
decision: DEC.2026-08-22.EVIDENCE-BOUNDED-PRESERVATION
check: crates/unica-coder/src/application/meta_remove_surface_tests.rs::real_public_meta_remove_reports_typed_cache_impact_in_the_same_result
scope: [app, cache, product]
---

# Meta remove сразу сообщает влияние на кеш

Результат реального `unica.meta.remove` в preview и apply содержит режим кеша,
событие и списки инвалидаций и refresh, не требуя второго публичного вызова.
Это доказательство не обобщается на результат каждого публичного мутатора.
