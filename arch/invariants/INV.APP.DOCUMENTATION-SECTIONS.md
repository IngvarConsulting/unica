---
id: INV.APP.DOCUMENTATION-SECTIONS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/documentation.rs::sections_follow_registry_order_and_carry_provenance
scope: [app]
---

# Секции документации следуют порядку поставщиков

Поиск сохраняет порядок двух поставщиков из фикстуры; первая спроецированная
секция несёт проверенные `sourceKind`, `authority`, `corpus`, фактический язык и
версию первого попадания.
