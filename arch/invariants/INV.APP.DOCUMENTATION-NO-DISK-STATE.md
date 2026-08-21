---
id: INV.APP.DOCUMENTATION-NO-DISK-STATE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/platform_help/provider.rs::a_replaced_container_is_reindexed_instead_of_answering_stale_help
scope: [app]
---

# Замена контейнера вытесняет старый справочный индекс

Если отпечаток контейнера установки изменился, следующий запрос перестраивает
индекс и не отвечает страницей из прежнего содержимого.
