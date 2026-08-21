---
id: INV.SAFETY.SUPPORT-GUARD-PARITY
status: active
governs: product
decision: DEC.2026-08-22.EVIDENCE-BOUNDED-SAFETY
check: crates/unica-coder/src/infrastructure/support_guard.rs::public_support_guard_resolver_matrix_runs_real_handlers
scope: [app, product]
---

# Ветви защиты поддержки имеют preview/apply представителей

Закрытый инвентарь политик проверяется отдельно. Реальные представители
handler-resolved, path-arg и object-name защиты — `code.patch`,
`subsystem.compile` и `form.remove` — одинаково блокируют preview и apply до
обработчика; представитель исключения `cf.init` достигает обоих планировщиков.
Это не объявляет исполненными остальные строки инвентаря.
