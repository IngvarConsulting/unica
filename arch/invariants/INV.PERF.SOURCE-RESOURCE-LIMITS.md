---
id: INV.PERF.SOURCE-RESOURCE-LIMITS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/domain/source_resources.rs::resource_limits_and_completeness_are_explicit_contract_values
scope: [product, source]
---

# Лимиты ресурсов источника являются явным контрактом

Снимок содержит не более 100 ресурсов, страница — не более 50, чтение — не
более 64 КиБ, срок жизни равен пяти минутам, а полнота имеет закрытые значения.
