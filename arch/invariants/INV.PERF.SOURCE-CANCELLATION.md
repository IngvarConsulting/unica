---
id: INV.PERF.SOURCE-CANCELLATION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/platform_xml_resources.rs::source_resources_cancellation_between_phases_returns_private_error
scope: [product, source]
---

# Отмена проверяется между фазами снимка

Отмена между разрешением цели и публикацией снимка прекращает операцию до
выдачи частичного результата.
