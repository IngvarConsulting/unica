---
id: INV.CACHE.ORCHESTRATOR-OWNED
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::application_dispatches_workspace_cache_and_handlers_through_ports
scope: [cache]
---

# Координация кеша принадлежит application

Application обнаруживает рабочее пространство, вызывает обработчик и отдельно
передаёт заявленные области кеша в порт отчёта. Обработчик не публикует кеш
напрямую.
