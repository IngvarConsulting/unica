---
id: INV.SOURCE.SUBSYSTEM-DEADLINE-UNAVAILABLE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::public_subsystem_info_deadline_covers_registered_preflight
scope: [source]
---

# Истечение срока не публикует проекцию подсистемы

Истечение срока во время зарегистрированной предпроверки `subsystem.info`
завершает вызов типизированной недоступностью провайдера и не публикует
частичную или пустую доказанную проекцию.
