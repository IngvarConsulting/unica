---
id: INV.APP.DOCUMENTATION-CONTAINER-FINGERPRINT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/platform_help/provider.rs::a_replaced_container_is_reindexed_instead_of_answering_stale_help
scope: [app]
---

# Замена контейнера меняет отпечаток справочного индекса

После замены контейнера следующий запрос перестраивает индекс и возвращает
новое содержимое вместо страницы из прежнего отпечатка.
