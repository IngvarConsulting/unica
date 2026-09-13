---
id: INV.CACHE.ORCHESTRATOR-OWNED
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::application_dispatches_workspace_cache_and_handlers_through_ports
scope: [cache]
---

# Application dispatch кеша проходит через объявленные порты

Application dispatch обнаруживает рабочее пространство и передаёт обработчик и
его заявленные области кеша через соответствующие порты.
