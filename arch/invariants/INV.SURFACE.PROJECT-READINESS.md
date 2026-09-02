---
id: INV.SURFACE.PROJECT-READINESS
status: active
governs: product
decision: DEC.2026-09-02.DIRECTIONAL-RUNTIME-OPERATIONS
check: crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_bootstrap_separates_source_and_repository_readiness
scope: [wire]
---

# Готовность проекта отделена от готовности репозитория

Проект с корректным набором исходников и без Git отвечает `ready=true`,
`repositoryReady=false` и отдельной диагностикой отсутствия репозитория.
