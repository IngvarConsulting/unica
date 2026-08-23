---
id: INV.SOURCE.ROOT-READINESS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::project_status_workspace_root_rejection_preserves_the_entire_tree
scope: [source]
---

# Корень-рабочее-пространство закрывает готовность

Набор исходников, чей корень совпадает с рабочим пространством, публикует
`source_set.root_is_workspace`, закрывает `ready` и не изменяет рабочее дерево.
