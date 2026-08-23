---
id: INV.SOURCE.PRIVATE-COMPILE-RECOVERY
status: active
governs: product
decision: DEC.2026-08-23.PRIVATE-COMPILE-RECOVERY
check: crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs::compile_recovery_is_reserved_outside_workspace_source_root
scope: [source]
---

# Recovery compile-транзакции не публикуется в source-set

Registration backup, removal backup и rollback quarantine для цели внутри
workspace резервируются под `<workspace>/.build/unica/recovery`, а не внутри
дерева исходников.
