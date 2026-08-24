---
id: DEC.2026-08-23.PRIVATE-COMPILE-RECOVERY
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs::compile_recovery_is_reserved_outside_workspace_source_root
supersedes: []
superseded-by: null
establishes: [INV.SOURCE.PRIVATE-COMPILE-RECOVERY]
---

# Recovery compile-транзакции остаётся вне дерева исходников

**Решение.** Compile-транзакция размещает registration backup, removal backup
и rollback quarantine под `<workspace>/.build/unica/recovery`, когда цель
принадлежит workspace. Для цели без маркера workspace сохраняется локальный
sibling recovery, необходимый автономным транзакциям.

**Почему.** Наблюдатель исходников может удержать служебный каталог открытым и
сделать последующую индексацию source-set невозможной. Приватный корень не
попадает в обход исходников и остаётся на том же томе, что и workspace.

**Цена.** Транзакция обязана найти ближайший `v8project.yaml` и подготовить
приватный каталог до резервирования recovery.
