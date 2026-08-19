---
id: INV.APP.HIDDEN-SERVICES
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/workspace_services.rs
scope: [app]
---

# Внутренние сервисы скрыты и привязаны к рабочему пространству

Служебные процессы не видны модели, поднимаются лениво и живут в границах своего рабочего
пространства.
