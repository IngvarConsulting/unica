---
id: INV.SOURCE.IDEMPOTENT-EFFECTS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/native_operations.rs::typed_platform_resource_noop_emits_no_effects
scope: [source]
---

# Типизированный noop ресурса не публикует эффект

Точные noop-ветви типизированных метаданных, роли и XDTO не публикуют мутацию,
доменное событие или изменение кеша и сохраняют исходные байты.
