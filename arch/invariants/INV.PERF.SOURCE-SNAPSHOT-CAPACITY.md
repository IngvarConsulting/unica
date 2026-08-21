---
id: INV.PERF.SOURCE-SNAPSHOT-CAPACITY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/platform_xml_resources.rs::source_resource_limits_and_cancellation_matrix_is_exact
scope: [product, source]
---

# Живые снимки ограничены без скрытого вытеснения

Хранилище отклоняет новый снимок при исчерпании вместимости и не вытесняет
неистёкший снимок, на который ещё опирается вызывающая сторона.
