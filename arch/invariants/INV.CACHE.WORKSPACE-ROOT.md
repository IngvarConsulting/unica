---
id: INV.CACHE.WORKSPACE-ROOT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/workspace.rs::v8project_yaml_in_ancestor_defines_workspace_root
scope: [cache]
---

# Кеш рабочего пространства по умолчанию лежит под его корнем

Без отдельного переопределения корень кеша равен `.build/unica` внутри
обнаруженного корня рабочего пространства.
