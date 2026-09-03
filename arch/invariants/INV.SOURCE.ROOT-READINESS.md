---
id: INV.SOURCE.ROOT-READINESS
status: active
governs: product
decision: DEC.2026-09-03.V0-13-LEGACY-BATCH-2
check: crates/unica-coder/tests/platform/project_health.rs::project_health_workspace_root_rejection_suppresses_source_derived_git_facts
scope: [source]
---

# Корень-рабочее-пространство закрывает готовность

Набор исходников, чей корень совпадает с рабочим пространством, публикует
`source_set.root_is_workspace`, закрывает `ready` и не изменяет рабочее дерево.
