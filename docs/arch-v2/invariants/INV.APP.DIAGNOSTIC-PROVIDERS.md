---
id: INV.APP.DIAGNOSTIC-PROVIDERS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/diagnostics.rs::diagnostics_concurrency_contains_provider_panic_and_keeps_sibling_items
scope: [app]
---

# Отказ одного диагностического поставщика не скрывает другого

Паника одного параллельного поставщика локализуется в его секции, а исправные
наблюдения соседнего поставщика остаются в общем результате.
