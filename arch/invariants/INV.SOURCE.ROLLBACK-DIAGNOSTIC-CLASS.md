---
id: INV.SOURCE.ROLLBACK-DIAGNOSTIC-CLASS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs::rollback_and_cleanup_diagnostics_keep_distinct_failure_classes
scope: [source]
---

# Ошибка отката отделена от остатка очистки

Неудача восстановления или удаления уже опубликованного пути получает класс
`RollbackFailed` и диагностику `rollback encountered:` с путём восстановления.
Диагностика `cleanup encountered:` сохраняет класс исходной ошибки и относится
только к неудалённому временному, карантинному или уже восстановленному остатку.
