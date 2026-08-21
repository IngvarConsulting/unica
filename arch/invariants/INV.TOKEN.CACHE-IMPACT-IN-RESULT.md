---
id: INV.TOKEN.CACHE-IMPACT-IN-RESULT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::mutating_tool_defaults_to_dry_run_and_reports_cache
scope: [app, cache, product]
---

# Результат мутации сразу сообщает влияние на кеш

Тот же результат предпросмотра называет доменное событие и инвалидированный
кеш, не требуя отдельного запроса о последствиях.
