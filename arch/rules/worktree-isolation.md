---
id: INV.CACHE.WORKTREE-ISOLATION
check:
  - crates/unica-coder/src/infrastructure/workspace.rs::git_worktree_boundary_prevents_parent_workspace_discovery
---

# Worktree не наследует чужое рабочее пространство

Файл-указатель `.git` связанного worktree останавливает поиск корня рабочего
пространства в родительской основной копии.
