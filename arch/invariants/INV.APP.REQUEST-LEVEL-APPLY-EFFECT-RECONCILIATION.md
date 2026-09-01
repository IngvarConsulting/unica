---
id: INV.APP.REQUEST-LEVEL-APPLY-EFFECT-RECONCILIATION
status: active
governs: product
check: crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::request_level_reconciliation_drops_cancelled_effect_before_deduplication
decision: DEC.2026-09-01.REQUEST-LEVEL-APPLY-EFFECT-RECONCILIATION
scope: [app, source]
---

# Apply effect receipt следует финальному staged postimage

Request-level apply передаёт family planners immutable global operation index и
один staged state. Provisional candidate выживает только если все его
path-bound subjects представлены в финальных staged changes относительно
admitted preimage; stable deduplication выполняется после этой фильтрации.
