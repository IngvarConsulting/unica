---
id: INV.SOURCE.SUBSYSTEM-DEADLINE-UNAVAILABLE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::public_subsystem_info_deadline_returns_no_data
scope: [source]
---

# Истечение срока не публикует проекцию подсистемы

Истечение срока во время зарегистрированной предпроверки `subsystem.info`
завершает вызов ошибкой `provider deadline exceeded`, не помечает её как
`provider_unavailable` и возвращает `data: null`, не публикуя частичную или
пустую доказанную проекцию.
