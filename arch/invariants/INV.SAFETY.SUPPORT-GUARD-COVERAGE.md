---
id: INV.SAFETY.SUPPORT-GUARD-COVERAGE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::mutating_native_support_guard_matrix_is_closed
scope: [app, product]
---

# Каждая нативная мутация явно классифицирована по защите поддержки

Дескриптор каждой нативной мутации либо требует редактируемого владельца, либо
входит в закрытый перечень операций, не меняющих объект на поддержке.
