---
id: INV.SOURCE.IDEMPOTENT-PRECHECK
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/native_operations/code.rs::patch_rejects_content_that_would_break_anchor_idempotence
scope: [source]
---

# Недоказуемая повторяемость отклоняется до записи

Первый `unica.code.patch` отклоняется без записи, если его образ после записи не
позволяет доказать семантическую пустоту следующего идентичного вызова.
