---
id: INV.SOURCE.IDEMPOTENT-CONSUMED-SELECTOR
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/native_operations/code.rs::code_patch_replace_that_consumes_its_selector_cannot_apply_twice
scope: [source]
---

# Поглощённый селектор закрывает повторную запись

Замена, поглотившая собственный селектор, удовлетворяет идемпотентности
отказом: повторный идентичный вызов не находит цель и ничего не записывает,
поэтому второго применения не происходит.
