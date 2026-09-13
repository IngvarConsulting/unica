---
id: INV.SOURCE.BOUND-HANDLER-PREFLIGHT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/native_operations/code.rs::code_patch_rolls_back_if_owner_descriptor_changes_before_commit
scope: [source]
---

# Обработчик повторно связывает фактическую XML-зависимость

Изменяющий обработчик повторяет предпроверку по фактической XML-зависимости и
связывает её точный преобраз с транзакцией, поэтому смена дескриптора владельца
между планированием и публикацией отклоняет изменение и восстанавливает цель.
