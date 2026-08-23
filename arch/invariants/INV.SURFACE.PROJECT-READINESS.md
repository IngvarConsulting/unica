---
id: INV.SURFACE.PROJECT-READINESS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::project_status_without_git_separates_source_and_repository_readiness
scope: [wire]
---

# Готовность проекта отделена от готовности репозитория

Проект с корректным набором исходников и без Git отвечает `ready=true`,
`repositoryReady=false` и отдельной диагностикой отсутствия репозитория.
