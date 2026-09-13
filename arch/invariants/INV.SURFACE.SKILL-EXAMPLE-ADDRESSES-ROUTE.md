---
id: INV.SURFACE.SKILL-EXAMPLE-ADDRESSES-ROUTE
status: active
governs: product
decision: DEC.2026-09-11.SKILL-ADDRESSES-ARE-CHECKED-BY-THE-GRAMMAR
check: crates/unica-coder/src/infrastructure/logical_tree.rs::every_skill_example_address_is_routable
scope: [wire, product]
---

# Адрес из примера скилла разбирается и маршрутизируется

Значение адресного ключа в примере поставляемого скилла разбирается грамматикой
логического адреса и маршрутизируется профилем платформы. Пример, учащий адресу,
на который читатель отвечает отказом, — сломанная инструкция.
