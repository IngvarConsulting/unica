---
id: INV.APP.EVENT-BINDING
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/metadata_operations.rs::event_subscription_requires_explicit_non_global_module_fact
scope: [app]
---

# Обработчик подписки требует явный признак неглобального модуля

Валидация прочитанной подписки отказывает по полю `properties.handler`, если
дескриптор общего модуля не содержит доказательство `Global=false`.
