---
id: INV.APP.EVENT-BINDING
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/metadata_operations.rs::event_subscription_requires_explicit_non_global_server_module_facts
scope: [app]
---

# Обработчик подписки требует серверный неглобальный модуль

Валидация подписки отклоняет обработчик, если дескриптор общего модуля не
доказывает `Global=false` и требуемый серверный контекст.
