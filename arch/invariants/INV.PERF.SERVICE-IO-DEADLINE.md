---
id: INV.PERF.SERVICE-IO-DEADLINE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/workspace_services.rs::cancellable_connector_partial_response_cannot_bypass_deadline
scope: [app, product]
---

# Частичный ответ не продлевает срок чтения

Фрагменты ответа внутреннего сервиса разделяют один крайний срок и не могут
поддерживать публичный вызов бесконечной последовательностью неполных данных.
