---
id: INV.APP.RUNTIME-RESOURCE-TREE
status: active
governs: product
decision: DEC.2026-08-24.LONG-WORK-OWNERSHIP-SLICE
check: crates/unica-coder/src/infrastructure/runtime_jobs.rs::runtime_resource_tree_lease_contract
scope: [app, platform]
---

# Runtime resource остаётся занятым до смерти всего принадлежащего дерева

Завершение leader при живом descendant сохраняет phase Running и `active.lock`.
Cancellation и drop завершают и пожинают owned process tree в одном bounded
окне; terminal state и release ресурса наблюдаемы только после доказанной смерти
дерева. Runtime SharedWork key выводится из того же resource authority и точного
активного job lease, а не из аргументов вызова.
