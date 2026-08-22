---
id: INV.PLATFORM.PROCESS-TREE-LIFECYCLE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/platform/process.rs::managed_process_tree_lifecycle_is_bounded
scope: [platform, product]
---

# Управляемая работа владеет полным деревом процессов

Тайм-аут, отмена, drop и явная очистка startup-процесса завершают и пожинают
лидера вместе с потомками за ограниченное время. Unix использует отдельную
группу процессов, Windows прикрепляет приостановленного ребёнка к Job Object до
его запуска.
