---
id: INV.SAFETY.SUPPORT-GUARD-PARITY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::subsystem_compile_guards_locked_parent_before_both_planners
scope: [app, product]
---

# Защищённая подсистема одинаково блокирует preview и apply

`unica.subsystem.compile` проверяет запертого владельца до обоих планировщиков;
предпросмотр и применение возвращают одинаковый отказ без изменений дерева.
