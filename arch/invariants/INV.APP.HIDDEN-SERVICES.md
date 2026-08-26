---
id: INV.APP.HIDDEN-SERVICES
status: active
governs: product
decision: DEC.2026-08-19.RULE-CLAIMS-ONLY-WHAT-IT-CHECKS
check: crates/unica-coder/src/infrastructure/workspace_services.rs::service_identity_reuses_same_workspace_source_root_and_separates_other_roots
scope: [app]
---

# Внутренние сервисы привязаны к рабочему пространству

Служебный процесс живёт в границах своего рабочего пространства: один корень
исходников переиспользует ту же службу, чужие корни получают отдельные.
