---
id: INV.APP.EVENT-SOURCE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/metadata_operations.rs::meta_add_event_subscription_source_replace_needs_no_catalog_and_round_trips
scope: [app]
---

# Логический источник подписки проходит обратное чтение

Добавление подписки с логическим семейством источников публикует типизированный
источник, читает его обратно тем же значением и повторно применяется как noop.
