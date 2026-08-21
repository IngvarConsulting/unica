---
id: INV.APP.DOCUMENTATION-SECTIONS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/documentation.rs::sections_follow_registry_order_and_carry_provenance
scope: [app]
---

# Секции документации следуют порядку поставщиков

Поиск сохраняет порядок реестра поставщиков, а каждая секция несёт собственные
происхождение, статус и локальные попадания.
