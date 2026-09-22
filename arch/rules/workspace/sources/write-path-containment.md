---
id: INV.SOURCE.WRITE-CONTAINMENT
check:
  - crates/unica-coder/src/infrastructure/path_policy.rs::rejects_write_escape_outside_workspace_root
  - crates/unica-coder/src/infrastructure/platform/filesystem.rs::workspace_policy_rejects_lexically_external_symlink_into_workspace
  - crates/unica-coder/src/infrastructure/platform/filesystem.rs::workspace_policy_rejects_lexically_internal_symlink_outside_workspace
---

# Путь записи проверяется относительно границ проекта

При разрешении пути записи он должен оставаться внутри проекта и после
нормализации `..`, и после разрешения ссылок. Выход за корень отклоняется.
Ссылка снаружи проекта на файл внутри него также не делает внешний путь
допустимым.

Правило относится к проверке пути; оно не гарантирует защиту от его подмены
между проверкой и последующей записью.
