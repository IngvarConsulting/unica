---
id: INV.PERF.SOURCE-SNAPSHOT-CAPACITY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/platform_xml_resources.rs::live_snapshot_capacity_is_bounded_without_evicting_unexpired_snapshots
scope: [product, source]
---

# Живые снимки ограничены без скрытого вытеснения

Хранилище отклоняет новый снимок при исчерпании вместимости и не вытесняет
неистёкший снимок, на который ещё опирается вызывающая сторона.
