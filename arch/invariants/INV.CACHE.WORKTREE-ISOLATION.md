---
id: INV.CACHE.WORKTREE-ISOLATION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/workspace.rs::git_worktree_boundary_prevents_parent_workspace_discovery
scope: [cache]
---

# Связанное worktree не наследует корень родительской копии

Файл-указатель `.git` связанного рабочего дерева останавливает поиск маркера
рабочего пространства в родительской основной копии.
