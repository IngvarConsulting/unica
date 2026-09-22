---
id: INV.SOURCE.BOUND-HANDLER-PREFLIGHT
check:
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::staged_code_preserves_module_owner_support_format_and_preimage_guards
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::staged_code_reuses_actor_revision_and_race_fences
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::code_patch_rolls_back_if_owner_descriptor_changes_before_commit
---

# Правка BSL не публикуется по изменившемуся владельцу

Планировщик правки BSL проверяет дескриптор владельца модуля и связывает
использованные байты с изменением. Если после планирования изменился
дескриптор владельца или сам модуль, подготовленная правка отклоняется.
Чужой результат не перезаписывается.

Проверки охватывают планировщик BSL и публикацию через актора;
они не доказывают это свойство для каждого другого семейства операций.
