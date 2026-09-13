---
id: INV.SOURCE.ROLLBACK-VISIBLE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs::registration_rollback_preserves_same_name_recovery_decoy_after_parent_swap
scope: [source]
---

# Неудавшийся откат виден как ошибка целостности

Неудача безопасного восстановления уже опубликованного пути исходников —
жёсткая ошибка транзакции с диагностикой `rollback encountered:`. Она называет
затронутые и сохранённые пути, не перезаписывает конкурентную замену и сохраняет
восстановительные байты для ручной проверки непроверенной целостности дерева.
