---
id: INV.PERF.SOURCE-SNAPSHOT-TTL
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/platform_xml_resources.rs::ttl_boundary_expires_pages_and_reads_at_exact_deadline
scope: [product, source]
---

# Страница и чтение истекают на границе срока снимка

После точной границы TTL снимок одинаково недоступен для продолжения страницы и
для чтения ресурса.
