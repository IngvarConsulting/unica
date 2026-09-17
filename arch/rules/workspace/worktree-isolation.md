---
id: INV.CACHE.WORKTREE-ISOLATION
check:
  - crates/unica-coder/src/infrastructure/workspace.rs::git_worktree_boundary_prevents_parent_workspace_discovery
---

# Поиск проекта не выходит за границу рабочей копии Git

Когда Unica ищет корень проекта, файл `.git` связанной рабочей копии Git
(`worktree`) останавливает поиск. Подниматься дальше, в родительскую
основную копию, нельзя.

Например, рабочая копия лежит в `main/worktrees/feature`, а файл проекта —
в `main/v8project.yaml`. При поиске из `feature/src` корнем должна стать
`feature`, а не `main`.
