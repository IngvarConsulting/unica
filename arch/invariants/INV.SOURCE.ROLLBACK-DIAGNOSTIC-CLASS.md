---
id: INV.SOURCE.ROLLBACK-DIAGNOSTIC-CLASS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check:
  - crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs::registration_rollback_preserves_same_name_recovery_decoy_after_parent_swap
  - crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs::registration_rollback_validation_reports_preserved_quarantine
  - crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs::removal_rollback_preserves_concurrent_file_and_recovery_artifact
  - crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs::removal_rollback_preserves_concurrent_empty_directory_and_recovery_tree
  - crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs::successful_registration_cleanup_warns_and_preserves_decoy_after_parent_swap
scope: [source]
---

# Ошибка отката отделена от остатка очистки

Неудача восстановления или удаления уже опубликованного пути получает класс
`RollbackFailed` и диагностику `rollback encountered:` с путём восстановления.
Диагностика `cleanup encountered:` сохраняет класс исходной ошибки и относится
только к неудалённому временному, карантинному или уже восстановленному остатку.
