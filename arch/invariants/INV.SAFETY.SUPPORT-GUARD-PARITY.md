---
id: INV.SAFETY.SUPPORT-GUARD-PARITY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/support_guard.rs::public_support_guard_resolver_matrix_runs_real_handlers
scope: [app, product]
---

# Защита поддержки одинакова для preview и apply

Каждая защищённая нативная мутация проверяет запертого владельца до обоих
планировщиков; отказ preview и apply совпадает по публичному результату. Каждое
исключение явно перечислено и достигает обоих планировщиков.
