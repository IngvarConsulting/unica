---
id: INV.TOKEN.CACHE-IMPACT-IN-RESULT
status: superseded
governs: product
decision: DEC.2026-09-03.V0-13-LEGACY-BATCH-2
check: crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_object_remove_reports_typed_cache_impact_in_preview_and_publication
scope: [app, cache, product]
---

# Meta remove сразу сообщает влияние на кеш

Результат реального `unica.meta.remove` в preview и apply содержит режим кеша,
событие и списки инвалидаций и refresh, не требуя второго публичного вызова.
Это доказательство не обобщается на результат каждого публичного мутатора.
